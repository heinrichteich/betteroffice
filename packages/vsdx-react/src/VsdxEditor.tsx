import { createT, deepMerge, diagnosticMessage, en } from '@betteroffice/vsdx-i18n';
import type { Translations } from '@betteroffice/vsdx-i18n';
import { canvasPointToModel, initWasm, openDiagram, paintPage, sizeCanvasForPage } from '@betteroffice/vsdx';
import type { Affine, PagePrimitive, CollaborationReplica, DiagramHandle, DiagramSnapshot, HitTestResult, ModelPoint, PageDisplayList, ShapeSnapshot, TextDiagnostic, VsdxFontFace, VsdxPresence } from '@betteroffice/vsdx';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties, PointerEvent, ReactNode } from 'react';
import { AUTO_CONNECT_FADE_MS, HOVER_FREE_DRAG_INCHES, HOVER_PROXIMITY_PX, QUICK_SHAPE_IDS, autoConnectArrowAt, autoConnectArrowCss, autoConnectArrowsForShape, autoConnectHaloHit, connectionPointsForShape, connectorDraft, connectorEndpointGlue, connectorGlue, connectorRouteFromFrame, dropTargetForPoint, hoverPointAt, isConnectorShape, nearestConnectionPointAnywhere, paintAutoConnectOverlay, paintConnectorOverlay, quickShapePlacement, reroutePreviewForMove, routeConnector } from './connector';
import type { AutoConnectSide, ConnectionPoint, ConnectorOverlayRoute, ConnectorOverlayScene } from './connector';
import { Ribbon } from './components/ribbon/Ribbon';
import { RibbonCommandsProvider, findShapePlacement, numericCellValue } from './components/ribbon/commands';
import { ShapesPanel } from './components/shapes/ShapesPanel';
import { standardShapeById, standardShapes } from './components/shapes/shapeLibrary';
import type { StandardShape } from './components/shapes/shapeLibrary';
import { StatusBar, clampZoom } from './components/statusbar';

export interface VsdxShapeSelection { pageId: string; shapeId: string; hit: HitTestResult; }
export interface VsdxEditorApi { handle: DiagramHandle; refresh: () => void; }
/** Save edits before changing a session identity or seed, or remount for a new session. */
export interface VsdxEditorCollaborationOptions {
  clientId: number;
  initialUpdate?: Uint8Array;
  onReplica?: (replica: CollaborationReplica | null) => void;
  presence?: VsdxPresence;
}
export interface VsdxEditorProps {
  file?: Uint8Array;
  fonts: ReadonlyArray<VsdxFontFace>;
  clientId?: number;
  collaboration?: VsdxEditorCollaborationOptions;
  i18n?: Translations;
  className?: string;
  onReady?: (api: VsdxEditorApi) => void;
  onChange?: () => void;
  onError?: (error: Error) => void;
  leftPanel?: ReactNode;
  statusBar?: ReactNode;
}

interface EditorModel { snapshot: DiagramSnapshot | null; pageIndex: number; frame: PageDisplayList | null; }

export function VsdxEditor({ file, fonts, clientId, collaboration, i18n, className, onReady, onChange, onError, leftPanel, statusBar }: VsdxEditorProps) {
  const strings = useMemo(() => deepMerge(en, i18n) as typeof en, [i18n]);
  const t = useMemo(() => createT(strings), [strings]);
  const handleRef = useRef<DiagramHandle | null>(null);
  const onReadyRef = useRef(onReady);
  const onChangeRef = useRef(onChange);
  const onErrorRef = useRef(onError);
  const collaborationRef = useRef(collaboration);
  const attachedCollaborationRef = useRef<VsdxEditorCollaborationOptions | undefined>(undefined);
  const mainCanvasRef = useRef<HTMLCanvasElement>(null);
  const overlayCanvasRef = useRef<HTMLCanvasElement>(null);
  const workspaceRef = useRef<HTMLElement>(null);
  const imageCache = useRef(new Map<string, Promise<CanvasImageSource | null>>());
  const stableFonts = useStableFontFaces(fonts);
  const fontsRef = useRef(stableFonts);
  const registeredFontsRef = useRef<ReadonlyArray<VsdxFontFace>>([]);
  const browserFontsRef = useRef(new Map<string, FontFace>());
  fontsRef.current = stableFonts;
  const requestedClientId = collaboration?.clientId ?? clientId;
  const requestedInitialUpdate = useStableInitialUpdate(collaboration?.initialUpdate);
  const sessionRef = useRef({ file, clientId: requestedClientId, initialUpdate: requestedInitialUpdate });
  const [model, setModel] = useState<EditorModel>({ snapshot: null, pageIndex: 0, frame: null });
  const modelRef = useRef(model);
  const [selection, setSelection] = useState<VsdxShapeSelection | null>(null);
  const [dirty, setDirty] = useState(false);
  const sessionSwitchBlocked = dirty && sessionRef.current.file === file &&
    (sessionRef.current.clientId !== requestedClientId || sessionRef.current.initialUpdate !== requestedInitialUpdate);
  const sessionClientId = sessionSwitchBlocked ? sessionRef.current.clientId : requestedClientId;
  const initialUpdate = sessionSwitchBlocked ? sessionRef.current.initialUpdate : requestedInitialUpdate;
  const [zoom, setZoom] = useState(1);
  const [shapesCollapsed, setShapesCollapsed] = useState(false);
  const [diagnostics, setDiagnostics] = useState<TextDiagnostic[]>([]);
  const [error, setError] = useState<string | null>(null);
  const pointerRef = useRef<DragStart | null>(null);
  const [connectorMode, setConnectorMode] = useState(false);
  const connectorModeRef = useRef(connectorMode);
  connectorModeRef.current = connectorMode;
  const selectionRef = useRef(selection);
  selectionRef.current = selection;
  const zoomRef = useRef(zoom);
  zoomRef.current = zoom;
  const hoverShapeRef = useRef<string | null>(null);
  const connectorDragRef = useRef<{ pageId: string; from: { shapeId: string; point: ConnectionPoint }; current: ModelPoint; snap: { shapeId: string; point: ConnectionPoint } | null } | null>(null);
  const autoHoverRef = useRef<string | null>(null);
  const pointHoverRef = useRef<string | null>(null);
  const pointCursorRef = useRef(false);
  const autoArrowRef = useRef<{ shapeId: string; side: AutoConnectSide } | null>(null);
  const autoAlphaRef = useRef(0);
  const autoFadeStartRef = useRef(0);
  const autoFrameRef = useRef<number | null>(null);
  const [quickMenu, setQuickMenu] = useState<{ shapeId: string; side: AutoConnectSide; x: number; y: number } | null>(null);
  const quickMenuRef = useRef(quickMenu);
  quickMenuRef.current = quickMenu;
  const quickMenuNodeRef = useRef<HTMLDivElement | null>(null);
  const reroutePreviewRef = useRef<ReadonlyArray<readonly ModelPoint[]>>([]);
  const [loading, setLoading] = useState(Boolean(file));
  onReadyRef.current = onReady;
  onChangeRef.current = onChange;
  onErrorRef.current = onError;
  collaborationRef.current = collaboration;

  const reportError = useCallback((value: unknown) => { const next = value instanceof Error ? value : new Error(String(value)); setError(next.message); onErrorRef.current?.(next); }, []);
  const refresh = useCallback((requestedPage?: number, notify = false) => {
    const handle = handleRef.current;
    if (!handle) return;
    if (notify) setDirty(true);
    try {
      if (notify) onChangeRef.current?.();
      const current = handle.snapshot();
      const previous = modelRef.current;
      const activeId = previous.snapshot?.pages[previous.pageIndex]?.id;
      const retainedIndex = current.pages.findIndex((page) => page.id === activeId);
      const pageIndex = Math.max(0, Math.min(requestedPage ?? (retainedIndex >= 0 ? retainedIndex : previous.pageIndex), Math.max(0, current.pages.length - 1)));
      const frame = current.pages.length ? handle.layoutPage(pageIndex) : null;
      modelRef.current = { snapshot: current, pageIndex, frame };
      setModel(modelRef.current);
      setDiagnostics(frame ? collectDiagnostics(frame) : []);
      setSelection((existing) => existing && stillSelectable(current, pageIndex, existing) ? existing : null);
    } catch (value) { reportError(value); }
  }, [reportError]);

  const paintOverlayNow = useCallback(() => {
    const canvas = overlayCanvasRef.current;
    const current = modelRef.current;
    const frame = current.frame;
    const snapshot = current.snapshot;
    if (!canvas || !frame || !snapshot) return;
    const context = canvas.getContext('2d');
    if (!context) return;
    const page = snapshot.pages[current.pageIndex];
    if (!page) return;
    context.setTransform(1, 0, 0, 1, 0, 0);
    context.clearRect(0, 0, canvas.width, canvas.height);
    const selection = selectionRef.current;
    const connectors: ConnectorOverlayRoute[] = [];
    for (const shape of flattenEditorShapes(page.shapes)) {
      if (!isConnectorShape(shape)) continue;
      const route = connectorRouteFromFrame(frame, page.sourcePartPath, shape.sourceId);
      if (!route) continue;
      const [beginGlue, endGlue] = connectorEndpointGlue(route, page.shapes);
      connectors.push({ route, selected: selection?.shapeId === shape.id, beginGlue, endGlue });
    }
    let hoverPoints: ConnectionPoint[] = [];
    if (connectorModeRef.current && hoverShapeRef.current) {
      const placement = findShapePlacement(page.shapes, hoverShapeRef.current);
      if (placement && placement.siblings === page.shapes && !isConnectorShape(placement.shape)) hoverPoints = connectionPointsForShape(placement.shape);
    } else if (!connectorModeRef.current && !connectorDragRef.current && pointHoverRef.current) {
      const placement = findShapePlacement(page.shapes, pointHoverRef.current);
      if (placement && placement.siblings === page.shapes && !isConnectorShape(placement.shape)) hoverPoints = connectionPointsForShape(placement.shape);
    }
    const drag = connectorDragRef.current;
    const end = drag?.snap?.point ?? drag?.current ?? null;
    const scene: ConnectorOverlayScene = {
      hoverPoints,
      snapPoint: drag?.snap?.point ?? null,
      previewRoute: drag && end ? routeConnector(drag.from.point, end) : null,
      reroutePreview: reroutePreviewRef.current,
      connectors,
    };
    const dpr = window.devicePixelRatio || 1;
    paintConnectorOverlay(context, frame, dpr, zoomRef.current, scene);
    if (!connectorModeRef.current && !connectorDragRef.current && autoHoverRef.current) {
      const placement = findShapePlacement(page.shapes, autoHoverRef.current);
      if (placement && placement.siblings === page.shapes && !isConnectorShape(placement.shape)) {
        paintAutoConnectOverlay(context, frame, dpr, zoomRef.current, {
          arrows: autoConnectArrowsForShape(placement.shape),
          hovered: autoArrowRef.current?.shapeId === placement.shape.id ? autoArrowRef.current.side : null,
          alpha: autoAlphaRef.current,
        });
      }
    }
  }, []);

  const cancelAutoFade = useCallback(() => {
    if (autoFrameRef.current !== null && typeof cancelAnimationFrame === 'function') cancelAnimationFrame(autoFrameRef.current);
    autoFrameRef.current = null;
  }, []);

  const startAutoFade = useCallback(() => {
    cancelAutoFade();
    autoFadeStartRef.current = performance.now();
    autoAlphaRef.current = 0;
    if (typeof requestAnimationFrame !== 'function') { autoAlphaRef.current = 1; paintOverlayNow(); return; }
    const tick = () => {
      autoAlphaRef.current = Math.min(1, (performance.now() - autoFadeStartRef.current) / AUTO_CONNECT_FADE_MS);
      paintOverlayNow();
      autoFrameRef.current = autoAlphaRef.current < 1 ? requestAnimationFrame(tick) : null;
    };
    autoFrameRef.current = requestAnimationFrame(tick);
  }, [cancelAutoFade, paintOverlayNow]);

  const hideAutoConnect = useCallback(() => {
    cancelAutoFade();
    autoHoverRef.current = null;
    autoArrowRef.current = null;
    autoAlphaRef.current = 0;
    pointHoverRef.current = null;
    pointCursorRef.current = false;
    if (mainCanvasRef.current) mainCanvasRef.current.style.cursor = '';
    if (quickMenuRef.current) setQuickMenu(null);
    paintOverlayNow();
  }, [cancelAutoFade, paintOverlayNow]);

  useEffect(() => () => { if (autoFrameRef.current !== null && typeof cancelAnimationFrame === 'function') cancelAnimationFrame(autoFrameRef.current); }, []);

  const setPointCursor = useCallback((canvas: HTMLCanvasElement | null, active: boolean) => {
    if (!canvas || pointCursorRef.current === active) return;
    pointCursorRef.current = active;
    canvas.style.cursor = active ? 'crosshair' : '';
  }, []);

  const pointHoverShapeAt = useCallback((shapes: readonly ShapeSnapshot[], canvas: ModelPoint): string | null => {
    const handle = handleRef.current;
    if (!handle) return null;
    const probe = (x: number, y: number): string | null => {
      let hit: HitTestResult | null = null;
      try { hit = handle.hitTest(x, y); } catch { hit = null; }
      const placement = hit ? findShapePlacement(shapes, hit.shapeId) : null;
      return placement && placement.siblings === shapes && !isConnectorShape(placement.shape) ? placement.shape.id : null;
    };
    const direct = probe(canvas.x, canvas.y);
    if (direct) return direct;
    const zoom = Number.isFinite(zoomRef.current) && zoomRef.current > 0 ? zoomRef.current : 1;
    const radius = HOVER_PROXIMITY_PX / zoom;
    for (const [dx, dy] of HOVER_PROBE_DIRS) {
      const found = probe(canvas.x + dx * radius, canvas.y + dy * radius);
      if (found) return found;
    }
    return null;
  }, []);

  const setConnectorActive = useCallback((active: boolean) => {
    connectorDragRef.current = null;
    hoverShapeRef.current = null;
    pointerRef.current = null;
    reroutePreviewRef.current = [];
    autoHoverRef.current = null;
    autoArrowRef.current = null;
    autoAlphaRef.current = 0;
    pointHoverRef.current = null;
    pointCursorRef.current = false;
    if (mainCanvasRef.current) mainCanvasRef.current.style.cursor = '';
    if (quickMenuRef.current) setQuickMenu(null);
    setConnectorMode(active);
    paintOverlayNow();
  }, [paintOverlayNow]);

  const toggleConnector = useCallback(() => {
    const current = modelRef.current;
    if (!handleRef.current || !current.snapshot?.pages[current.pageIndex] || !current.frame) return;
    if (!connectorModeRef.current) {
      try { handleRef.current.layoutPage(current.pageIndex); } catch (value) { reportError(value); return; }
    }
    setConnectorActive(!connectorModeRef.current);
  }, [reportError, setConnectorActive]);

  useEffect(() => {
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key === 'Escape') {
        if (connectorModeRef.current || connectorDragRef.current) setConnectorActive(false);
        else hideAutoConnect();
        return;
      }
      if (event.altKey && (event.key === '3' || event.key === '³')) {
        if (event.repeat) return;
        event.preventDefault();
        toggleConnector();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [hideAutoConnect, setConnectorActive, toggleConnector]);

  useEffect(() => {
    sessionRef.current = { file, clientId: sessionClientId, initialUpdate };
    let disposed = false;
    let handle: DiagramHandle | null = null;
    let stopUpdates = () => {};
    let stopResync = () => {};
    handleRef.current?.dispose(); handleRef.current = null; imageCache.current.clear(); setSelection(null); modelRef.current = { snapshot: null, pageIndex: 0, frame: null }; setModel(modelRef.current); setError(null); setDirty(false);
    if (!file) { setLoading(false); return; }
    setLoading(true);
    const openingFonts = fontsRef.current;
    void Promise.all([initWasm(), loadFonts(openingFonts)]).then(([, loadedFonts]) => {
      if (disposed) return;
      try {
        const activeCollaboration = collaborationRef.current;
        handle = openDiagram(file, { clientId: sessionClientId, fonts: openingFonts, initialUpdate }); registeredFontsRef.current = openingFonts; installFonts(loadedFonts, browserFontsRef.current); handleRef.current = handle; activeCollaboration?.onReplica?.(handle); attachedCollaborationRef.current = activeCollaboration;
        stopUpdates = handle.onUpdate(() => refresh(undefined, true));
        stopResync = handle.onResync(() => refresh(undefined, true));
        refresh(0); setLoading(false); onReadyRef.current?.({ handle, refresh: () => refresh(undefined, false) });
      } catch (value) { setLoading(false); reportError(value); }
    }, (value: unknown) => { if (!disposed) { setLoading(false); reportError(value); } });
    return () => {
      disposed = true;
      try { attachedCollaborationRef.current?.onReplica?.(null); }
      finally {
        attachedCollaborationRef.current = undefined;
        stopUpdates(); stopResync(); handle?.dispose();
        if (handleRef.current === handle) handleRef.current = null;
        for (const face of browserFontsRef.current.values()) document.fonts.delete?.(face);
        browserFontsRef.current.clear();
      }
    };
  }, [sessionClientId, initialUpdate, file, refresh, reportError]);

  useEffect(() => {
    const handle = handleRef.current;
    const attached = attachedCollaborationRef.current;
    if (!handle || sessionSwitchBlocked || attached?.onReplica === collaboration?.onReplica) return;
    attached?.onReplica?.(null);
    collaboration?.onReplica?.(handle);
    attachedCollaborationRef.current = collaboration;
  }, [collaboration, sessionSwitchBlocked]);

  useEffect(() => {
    if (sessionSwitchBlocked) reportError(new Error('Save your changes before switching collaboration sessions.'));
  }, [sessionSwitchBlocked, reportError]);

  const hasDocument = model.snapshot !== null;
  useEffect(() => {
    const handle = handleRef.current;
    if (!handle) return;
    const additions = stableFonts.filter((face) => !registeredFontsRef.current.some((registered) => fontFaceEqual(face, registered)));
    if (!additions.length) return;
    let disposed = false;
    void loadFonts(additions).then((loadedFonts) => {
      if (disposed || handleRef.current !== handle) return;
      try {
        for (const [index, face] of additions.entries()) {
          handle.registerFont(face);
          if (loadedFonts[index]) installFonts([loadedFonts[index]], browserFontsRef.current);
          registeredFontsRef.current = [...registeredFontsRef.current.filter((registered) => !fontFaceKeyEqual(face, registered)), face];
        }
        refresh();
      } catch (value) { reportError(value); }
    }, (value: unknown) => { if (!disposed) reportError(value); });
    return () => { disposed = true; };
  }, [stableFonts, hasDocument, refresh, reportError]);

  useEffect(() => {
    const canvas = mainCanvasRef.current; const frame = model.frame;
    if (!canvas || !frame) return;
    const context = canvas.getContext('2d'); if (!context) return;
    const controller = new AbortController();
    const originHandle = handleRef.current;
    const dpr = window.devicePixelRatio || 1; sizeCanvasForPage(canvas, frame, dpr, zoom);
    void paintPage(context, frame, dpr, zoom, {
      signal: controller.signal,
      resolveImage: async (assetId) => {
        if (controller.signal.aborted) return null;
        try { return await resolveImage(assetId, originHandle, imageCache, t('errors.decodePageImage')); }
        catch (value) { if (!controller.signal.aborted) reportError(value); return null; }
      },
    }).catch((value) => { if (!controller.signal.aborted) reportError(value); });
    return () => controller.abort();
  }, [model.frame, reportError, t, zoom]);

  useEffect(() => {
    const canvas = overlayCanvasRef.current; const frame = model.frame;
    if (!canvas || !frame) return;
    const context = canvas.getContext('2d'); if (!context) return;
    const dpr = window.devicePixelRatio || 1; sizeCanvasForPage(canvas, frame, dpr, zoom);
    paintOverlayNow();
  }, [model.frame, model.snapshot, selection, zoom, connectorMode, paintOverlayNow]);

  const onPointerMove = (event: PointerEvent<HTMLCanvasElement>) => {
    const handle = handleRef.current; const current = modelRef.current; const frame = current.frame;
    const page = current.snapshot?.pages[current.pageIndex];
    if (!handle || !frame || !page) return;
    const pointer = pointerRef.current;
    if (!connectorModeRef.current && pointer) {
      if (autoHoverRef.current || autoArrowRef.current || quickMenuRef.current || pointHoverRef.current) {
        autoHoverRef.current = null;
        autoArrowRef.current = null;
        autoAlphaRef.current = 0;
        pointHoverRef.current = null;
        if (quickMenuRef.current) setQuickMenu(null);
      }
      try {
        const selected = selectionRef.current;
        const placement = selected && selected.pageId === page.id ? findShapePlacement(page.shapes, selected.shapeId) : null;
        if (!placement || placement.siblings !== page.shapes || isConnectorShape(placement.shape)) {
          if (reroutePreviewRef.current.length) { reroutePreviewRef.current = []; paintOverlayNow(); }
          return;
        }
        const point = canvasPointerPosition(event, frame);
        const geometry = resolveDragGeometry(pointer, point.model);
        reroutePreviewRef.current = reroutePreviewForMove(page.shapes, frame, page.sourcePartPath, placement.shape, geometry);
        paintOverlayNow();
      } catch (value) { reportError(value); }
      return;
    }
    if (!connectorModeRef.current) {
      try {
        const point = canvasPointerPosition(event, frame);
        const drag = connectorDragRef.current;
        if (drag) {
          drag.current = point.model;
          drag.snap = connectorTargetForPoint(page.shapes, handle, point.canvas, point.model);
          paintOverlayNow();
          return;
        }
        if (autoHoverRef.current) {
          const hoveredPlacement = findShapePlacement(page.shapes, autoHoverRef.current);
          const arrows = hoveredPlacement && hoveredPlacement.siblings === page.shapes && !isConnectorShape(hoveredPlacement.shape)
            ? autoConnectArrowsForShape(hoveredPlacement.shape)
            : [];
          const arrow = autoConnectArrowAt(arrows, frame, zoomRef.current, point.canvas);
          const next = arrow ? { shapeId: autoHoverRef.current, side: arrow.side } : null;
          const previous = autoArrowRef.current;
          if ((next === null) !== (previous === null) || next?.side !== previous?.side || next?.shapeId !== previous?.shapeId) {
            autoArrowRef.current = next;
            if (next && arrow) {
              const anchor = autoConnectArrowCss(arrow, frame, zoomRef.current);
              setQuickMenu({ shapeId: next.shapeId, side: next.side, x: anchor.x, y: anchor.y });
            } else if (quickMenuRef.current) setQuickMenu(null);
            paintOverlayNow();
          }
          if (arrow) { setPointCursor(event.currentTarget, false); return; }
        }
        let placement = pointHoverRef.current ? findShapePlacement(page.shapes, pointHoverRef.current) : null;
        if (placement && (placement.siblings !== page.shapes || isConnectorShape(placement.shape))) placement = null;
        if (placement && !autoConnectHaloHit(placement.shape, frame, zoomRef.current, point.canvas)) placement = null;
        if (!placement) {
          const probed = pointHoverShapeAt(page.shapes, point.canvas);
          placement = probed ? findShapePlacement(page.shapes, probed) : null;
          if (placement && (placement.siblings !== page.shapes || isConnectorShape(placement.shape))) placement = null;
        }
        const hovered = placement ? placement.shape.id : null;
        if (hovered !== autoHoverRef.current || hovered !== pointHoverRef.current) {
          autoHoverRef.current = hovered;
          pointHoverRef.current = hovered;
          autoArrowRef.current = null;
          if (quickMenuRef.current) setQuickMenu(null);
          if (hovered) startAutoFade();
          else { autoAlphaRef.current = 0; paintOverlayNow(); }
        }
        setPointCursor(event.currentTarget, placement ? hoverPointAt(connectionPointsForShape(placement.shape), frame, zoomRef.current, point.model) !== null : false);
      } catch (value) { reportError(value); }
      return;
    }
    try {
      const point = canvasPointerPosition(event, frame);
      const drag = connectorDragRef.current;
      if (drag) {
        drag.current = point.model;
        drag.snap = connectorTargetForPoint(page.shapes, handle, point.canvas, point.model);
        paintOverlayNow();
        return;
      }
      const hit = handle.hitTest(point.canvas.x, point.canvas.y);
      const placement = hit ? findShapePlacement(page.shapes, hit.shapeId) : null;
      const hovered = placement && placement.siblings === page.shapes && !isConnectorShape(placement.shape) ? placement.shape.id : null;
      if (hovered !== hoverShapeRef.current) { hoverShapeRef.current = hovered; paintOverlayNow(); }
    } catch (value) { reportError(value); }
  };
  const onPointerLeave = (event: PointerEvent<HTMLCanvasElement>) => {
    if (!connectorModeRef.current && !connectorDragRef.current) {
      const next = event.relatedTarget as Node | null;
      if (next && quickMenuNodeRef.current?.contains(next)) return;
      setPointCursor(event.currentTarget, false);
      if (autoHoverRef.current || autoArrowRef.current || quickMenuRef.current || pointHoverRef.current) hideAutoConnect();
      return;
    }
    if (!connectorModeRef.current || connectorDragRef.current) return;
    if (hoverShapeRef.current) { hoverShapeRef.current = null; paintOverlayNow(); }
  };
  const onPointerDown = (event: PointerEvent<HTMLCanvasElement>) => {
    const handle = handleRef.current; const current = modelRef.current; const frame = current.frame; const page = current.snapshot?.pages[current.pageIndex];
    if (!handle || !frame || !page) return;
    if (connectorModeRef.current) {
      pointerRef.current = null;
      try {
        const point = canvasPointerPosition(event, frame);
        const target = connectorTargetForPoint(page.shapes, handle, point.canvas, point.model);
        if (target) {
          connectorDragRef.current = { pageId: page.id, from: target, current: point.model, snap: target };
          setSelection({ pageId: page.id, shapeId: target.shapeId, hit: { kind: 'shape', shapeId: target.shapeId } });
          capturePointer(event);
        } else {
          const hit = handle.hitTest(point.canvas.x, point.canvas.y);
          setSelection(hit ? { pageId: page.id, shapeId: hit.shapeId, hit } : null);
        }
        paintOverlayNow();
      } catch (value) { reportError(value); }
      return;
    }
    pointerRef.current = null;
    reroutePreviewRef.current = [];
    try {
      const point = canvasPointerPosition(event, frame);
      const grabbedId = pointHoverRef.current ?? pointHoverShapeAt(page.shapes, point.canvas);
      const grabbedPlacement = grabbedId ? findShapePlacement(page.shapes, grabbedId) : null;
      const grabbedShape = grabbedPlacement && grabbedPlacement.siblings === page.shapes && !isConnectorShape(grabbedPlacement.shape) ? grabbedPlacement.shape : null;
      const grabbed = grabbedShape ? hoverPointAt(connectionPointsForShape(grabbedShape), frame, zoomRef.current, point.model) : null;
      if (grabbed && grabbedShape) {
        pointHoverRef.current = grabbedShape.id;
        autoHoverRef.current = null;
        autoArrowRef.current = null;
        autoAlphaRef.current = 0;
        if (quickMenuRef.current) setQuickMenu(null);
        connectorDragRef.current = { pageId: page.id, from: { shapeId: grabbedShape.id, point: grabbed }, current: point.model, snap: null };
        setSelection({ pageId: page.id, shapeId: grabbedShape.id, hit: { kind: 'shape', shapeId: grabbedShape.id } });
        setPointCursor(event.currentTarget, true);
        capturePointer(event);
        paintOverlayNow();
        return;
      }
      if (autoHoverRef.current) {
        const hoveredPlacement = findShapePlacement(page.shapes, autoHoverRef.current);
        const arrows = hoveredPlacement && hoveredPlacement.siblings === page.shapes && !isConnectorShape(hoveredPlacement.shape)
          ? autoConnectArrowsForShape(hoveredPlacement.shape)
          : [];
        const arrow = autoConnectArrowAt(arrows, frame, zoomRef.current, point.canvas);
        if (arrow) {
          const sourceId = autoHoverRef.current;
          connectorDragRef.current = { pageId: page.id, from: { shapeId: sourceId, point: arrow.point }, current: point.model, snap: null };
          autoHoverRef.current = null;
          autoArrowRef.current = null;
          autoAlphaRef.current = 0;
          if (quickMenuRef.current) setQuickMenu(null);
          setSelection({ pageId: page.id, shapeId: sourceId, hit: { kind: 'shape', shapeId: sourceId } });
          capturePointer(event);
          paintOverlayNow();
          return;
        }
      }
      if (quickMenuRef.current) setQuickMenu(null);
      autoHoverRef.current = null;
      autoArrowRef.current = null;
      autoAlphaRef.current = 0;
      pointHoverRef.current = null;
      setPointCursor(event.currentTarget, false);
      handle.layoutPage(modelRef.current.pageIndex);
      const hit = handle.hitTest(point.canvas.x, point.canvas.y);
      setSelection(hit ? { pageId: page.id, shapeId: hit.shapeId, hit } : null);
      const placement = hit ? findShapePlacement(page.shapes, hit.shapeId) : null;
      pointerRef.current = hit && placement ? {
        ...point,
        parentTransforms: shapeParentTransforms(frame.primitives, `${page.sourcePartPath}:${placement.shape.sourceId}`) ?? [],
        angle: numericCellValue(placement.shape, 'Angle', 0),
        flipX: numericCellValue(placement.shape, 'FlipX', 0) === 1,
        flipY: numericCellValue(placement.shape, 'FlipY', 0) === 1,
        resize: event.shiftKey,
        pin: { x: numericCellValue(placement.shape, 'PinX'), y: numericCellValue(placement.shape, 'PinY') },
        size: { width: numericCellValue(placement.shape, 'Width'), height: numericCellValue(placement.shape, 'Height') },
      } : null;
      capturePointer(event);
    } catch (value) { reportError(value); }
  };
  const onPointerUp = (event: PointerEvent<HTMLCanvasElement>) => {
    const drag = connectorDragRef.current;
    if (connectorModeRef.current || drag) {
      connectorDragRef.current = null;
      const handle = handleRef.current;
      try {
        const current = modelRef.current;
        const frame = current.frame;
        const page = current.snapshot?.pages[current.pageIndex];
        let end = drag?.snap ?? null;
        if (drag && handle && frame && page) {
          try {
            const point = canvasPointerPosition(event, frame);
            end = connectorTargetForPoint(page.shapes, handle, point.canvas, point.model) ?? drag.snap;
          } catch { end = drag.snap; }
        }
        if (drag && handle && end && (end.shapeId !== drag.from.shapeId || end.point.side !== drag.from.point.side)) {
          const live = handle.snapshot();
          const livePage = live.pages.find((item) => item.id === drag.pageId);
          const fromPlacement = livePage ? findShapePlacement(livePage.shapes, drag.from.shapeId) : null;
          const endPlacement = livePage ? findShapePlacement(livePage.shapes, end.shapeId) : null;
          if (!livePage || !fromPlacement || !endPlacement) refresh(undefined, true);
          else {
            const receipt = handle.addConnector(drag.pageId, connectorDraft(drag.from.point, end.point), connectorGlue(drag.from.shapeId, drag.from.point), connectorGlue(end.shapeId, end.point));
            refresh(undefined, true);
            setSelection({ pageId: drag.pageId, shapeId: receipt.shapeId, hit: { kind: 'shape', shapeId: receipt.shapeId } });
            pointHoverRef.current = end.shapeId;
          }
        } else if (drag && handle && !end && !connectorModeRef.current) {
          const current = modelRef.current;
          const frame = current.frame;
          const page = current.snapshot?.pages[current.pageIndex];
          if (frame && page && page.id === drag.pageId) {
            let drop: ModelPoint | null = null;
            try { drop = canvasPointerPosition(event, frame).model; } catch { drop = null; }
            if (drop && Math.hypot(drop.x - drag.from.point.x, drop.y - drag.from.point.y) >= HOVER_FREE_DRAG_INCHES) {
              try {
                const receipt = handle.addShape(drag.pageId, connectorDraft(drag.from.point, drop));
                refresh(undefined, true);
                setSelection({ pageId: drag.pageId, shapeId: receipt.shapeId, hit: { kind: 'shape', shapeId: receipt.shapeId } });
                pointHoverRef.current = drag.from.shapeId;
              } catch (value) { reportError(value); }
            }
          }
        }
      } catch (value) { reportError(value); }
      setPointCursor(event.currentTarget, false);
      paintOverlayNow();
      return;
    }
    const pointer = pointerRef.current; pointerRef.current = null;
    reroutePreviewRef.current = [];
    const handle = handleRef.current; const selected = selection; const frame = model.frame;
    if (!pointer || !handle || !selected || !frame) { paintOverlayNow(); return; }
    try {
      const point = canvasPointerPosition(event, frame);
      if (Math.abs(point.canvas.x - pointer.canvas.x) < 0.01 && Math.abs(point.canvas.y - pointer.canvas.y) < 0.01) { paintOverlayNow(); return; }
      const geometry = resolveDragGeometry(pointer, point.model);
      if (pointer.resize) handle.resizeShape(selected.pageId, selected.shapeId, inchFormula(geometry.width), inchFormula(geometry.height));
      else handle.moveShape(selected.pageId, selected.shapeId, inchFormula(geometry.x), inchFormula(geometry.y));
      refresh(undefined, true);
    } catch (value) { reportError(value); }
    paintOverlayNow();
  };
  const insertShape = useCallback((shape: StandardShape) => {
    const handle = handleRef.current; const current = modelRef.current; const frame = current.frame;
    const page = current.snapshot?.pages[current.pageIndex];
    if (!handle || !page || !frame) return;
    try {
      const centre = canvasPointToModel(frame.paintTransform, frame.width / 2, frame.height / 2);
      handle.addShape(page.id, shape.draft(centre.x, centre.y, 1, 1));
      refresh(undefined, true);
    } catch (value) { reportError(value); }
  }, [refresh, reportError]);
  const insertQuickShape = useCallback((shape: StandardShape, sourceId: string, side: AutoConnectSide) => {
    const handle = handleRef.current; const current = modelRef.current;
    const page = current.snapshot?.pages[current.pageIndex];
    if (!handle || !page) return;
    try {
      const live = handle.snapshot();
      const livePage = live.pages.find((item) => item.id === page.id);
      const placement = livePage ? findShapePlacement(livePage.shapes, sourceId) : null;
      if (!livePage || !placement || placement.siblings !== livePage.shapes || isConnectorShape(placement.shape)) return;
      const source = placement.shape;
      const layout = quickShapePlacement(source, side, Math.max(0.25, numericCellValue(source, 'Width', 1)), Math.max(0.25, numericCellValue(source, 'Height', 1)));
      if (!layout) return;
      const receipt = handle.addConnectedShape(page.id, shape.draft(layout.x, layout.y, layout.width, layout.height), connectorDraft(layout.from, layout.to), connectorGlue(source.id, layout.from), layout.to.toCell);
      refresh(undefined, true);
      setSelection({ pageId: page.id, shapeId: receipt.shape.shapeId, hit: { kind: 'shape', shapeId: receipt.shape.shapeId } });
    } catch (value) { reportError(value); }
    autoHoverRef.current = null;
    autoArrowRef.current = null;
    autoAlphaRef.current = 0;
    setQuickMenu(null);
    paintOverlayNow();
  }, [refresh, reportError, paintOverlayNow]);
  const quickMenuShapes = useMemo(() => QUICK_SHAPE_IDS.map((id) => standardShapeById(id)).filter((shape): shape is StandardShape => Boolean(shape)), []);
  const reorderPage = useCallback((pageId: string, toIndex: number) => {
    const handle = handleRef.current;
    if (!handle) return;
    try { handle.reorderPage(pageId, toIndex); refresh(toIndex, true); } catch (value) { reportError(value); }
  }, [refresh, reportError]);
  const fitToWindow = useCallback(() => {
    const frame = modelRef.current.frame; const workspace = workspaceRef.current;
    if (!frame || !workspace || frame.width <= 0 || frame.height <= 0) return;
    const rect = workspace.getBoundingClientRect();
    setZoom(clampZoom(Math.min((rect.width - WORKSPACE_MARGIN) / frame.width, (rect.height - WORKSPACE_MARGIN) / frame.height)));
  }, []);
  const download = useCallback((bytes: Uint8Array) => { const buffer = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer; const url = URL.createObjectURL(new Blob([buffer], { type: 'application/vnd.visio' })); const anchor = document.createElement('a'); anchor.href = url; anchor.download = 'diagram.vsdx'; anchor.click(); URL.revokeObjectURL(url); setDirty(false); }, []);
  const integrity = diagnostics.filter((diagnostic) => diagnostic.category === 'integrity');
  const fidelity = diagnostics.filter((diagnostic) => diagnostic.category === 'fidelity');
  return <div className={className} style={styles.root} aria-label={t('editor.appLabel')}>
    <header style={styles.titleBar}><strong>{t('ribbon.documentName')}</strong><span style={{ color: dirty ? '#a16207' : '#526273' }}>{dirty ? t('ribbon.dirty') : t('ribbon.saved')}</span></header>
    <RibbonCommandsProvider handle={handleRef.current} snapshot={model.snapshot} pageId={model.snapshot?.pages[model.pageIndex]?.id} selection={selection} onMutation={() => refresh(undefined, true)} onError={reportError} onDownload={download}>
    <Ribbon t={t} connector={{ active: connectorMode, disabled: !model.frame, onToggle: toggleConnector }} />
    <div style={styles.contentRow}>
    {leftPanel === undefined ? <ShapesPanel shapes={standardShapes} collapsed={shapesCollapsed} onToggleCollapsed={() => setShapesCollapsed((value) => !value)} onInsert={insertShape} t={t} /> : leftPanel}
    <main ref={workspaceRef} style={styles.workspace}>
      {loading && <span>{t('editor.opening')}</span>}
      {!loading && !model.frame && <span>{file ? t('editor.noPages') : t('editor.openPrompt')}</span>}
      <div style={styles.canvasFrame}>
        <canvas ref={mainCanvasRef} onPointerDown={onPointerDown} onPointerMove={onPointerMove} onPointerLeave={onPointerLeave} onPointerUp={onPointerUp} onPointerCancel={() => { pointerRef.current = null; connectorDragRef.current = null; reroutePreviewRef.current = []; autoHoverRef.current = null; autoArrowRef.current = null; autoAlphaRef.current = 0; pointHoverRef.current = null; pointCursorRef.current = false; if (mainCanvasRef.current) mainCanvasRef.current.style.cursor = ''; if (quickMenuRef.current) setQuickMenu(null); paintOverlayNow(); }} aria-label={selection ? t('pages.canvasLabelWithSelection', { current: model.pageIndex + 1, total: model.snapshot?.pages.length ?? 0, name: selection.shapeId }) : t('pages.canvasLabel', { current: model.pageIndex + 1, total: model.snapshot?.pages.length ?? 0 })} style={connectorMode ? { ...styles.canvas, cursor: 'crosshair' } : styles.canvas} />
        <canvas ref={overlayCanvasRef} aria-hidden="true" style={styles.overlay} />
        {quickMenu && model.frame && (
          <div ref={quickMenuNodeRef} role="menu" aria-label={t('shapesPanel.quickShapes')} style={{ ...styles.quickMenu, left: Math.max(4, Math.min(quickMenu.x + 16, model.frame.width * zoom - 44)), top: Math.max(100, Math.min(quickMenu.y, model.frame.height * zoom - 100)) }} onMouseLeave={() => { autoArrowRef.current = null; setQuickMenu(null); paintOverlayNow(); }}>
            {quickMenuShapes.map((shape) => (
              <button key={shape.id} type="button" role="menuitem" aria-label={t(shape.nameKey)} title={t(shape.nameKey)} onClick={() => insertQuickShape(shape, quickMenu.shapeId, quickMenu.side)} style={styles.quickShape}>
                <svg aria-hidden="true" viewBox="0 0 1 1" preserveAspectRatio="xMidYMid meet" style={styles.quickPreview}><path d={shape.preview} /></svg>
              </button>
            ))}
          </div>
        )}
        {selection && <output style={styles.selection}>{t('shapes.selected', { name: selection.shapeId })}</output>}
      </div>
      {integrity.length > 0 && <section role="alert" style={styles.integrity}><strong>{t('diagnostics.integrityHeading')}</strong>{integrity.map((item, index) => <div key={`${item.code}-${index}`}>{diagnosticMessage(t, item.category, item.code)}</div>)}</section>}
      {fidelity.length > 0 && <details style={styles.fidelity}><summary>{t('diagnostics.fidelityHeading')}</summary>{fidelity.map((item, index) => <div key={`${item.code}-${index}`}>{diagnosticMessage(t, item.category, item.code)}</div>)}</details>}
      {error && <div role="alert" style={styles.error}>{error}</div>}
    </main>
    </div>
    {statusBar === undefined ? <StatusBar pages={model.snapshot?.pages ?? []} activeIndex={model.pageIndex} onSelectPage={(index) => refresh(index)} onReorderPage={reorderPage} zoom={zoom} onZoomChange={setZoom} onFitToWindow={fitToWindow} t={t} /> : statusBar}
    </RibbonCommandsProvider>
  </div>;
}

const MIN_SHAPE_INCHES = 0.01;
const WORKSPACE_MARGIN = 32;
const HOVER_PROBE_DIRS: ReadonlyArray<readonly [number, number]> = [[1, 0], [-1, 0], [0, 1], [0, -1], [1, 1], [1, -1], [-1, 1], [-1, -1]];

export function canvasPointerPosition(event: PointerEvent<HTMLCanvasElement>, frame: PageDisplayList): { canvas: ModelPoint; model: ModelPoint } {
  const rect = event.currentTarget.getBoundingClientRect();
  const canvas = {
    x: (event.clientX - rect.left) * frame.width / Math.max(rect.width, 1),
    y: (event.clientY - rect.top) * frame.height / Math.max(rect.height, 1),
  };
  return { canvas, model: canvasPointToModel(frame.paintTransform, canvas.x, canvas.y) };
}

export function inchFormula(value: number): string {
  if (!Number.isFinite(value)) throw new Error('Shape geometry must be finite.');
  const rounded = Number(value.toFixed(6));
  return String(Object.is(rounded, -0) ? 0 : rounded);
}

export interface DragStart { canvas: ModelPoint; model: ModelPoint; resize: boolean; pin: ModelPoint; size: { width: number; height: number }; parentTransforms?: readonly Affine[]; angle?: number; flipX?: boolean; flipY?: boolean; }

export function resolveDragGeometry(start: DragStart, release: ModelPoint): { x: number; y: number; width: number; height: number } {
  const toParent = (point: ModelPoint) => (start.parentTransforms ?? []).reduce((local, transform) => canvasPointToModel(transform, local.x, local.y), point);
  const origin = toParent(start.model);
  const end = toParent(release);
  const deltaX = end.x - origin.x;
  const deltaY = end.y - origin.y;
  if (start.resize) {
    const cos = Math.cos(start.angle ?? 0), sin = Math.sin(start.angle ?? 0);
    const widthDelta = (cos * deltaX + sin * deltaY) * (start.flipX ? -1 : 1);
    const heightDelta = (-sin * deltaX + cos * deltaY) * (start.flipY ? -1 : 1);
    return { x: start.pin.x, y: start.pin.y, width: Math.max(MIN_SHAPE_INCHES, start.size.width + widthDelta), height: Math.max(MIN_SHAPE_INCHES, start.size.height + heightDelta) };
  }
  return { x: start.pin.x + deltaX, y: start.pin.y + deltaY, width: start.size.width, height: start.size.height };
}

export function shapeParentTransforms(primitives: readonly PagePrimitive[], id: string, depth = 0): Affine[] | null {
  if (depth >= 256) return null;
  for (const primitive of primitives) {
    if (primitive.id === id) return [];
    if (primitive.kind === 'group') {
      const nested = shapeParentTransforms(primitive.primitives, id, depth + 1);
      if (nested) return primitive.transform ? [primitive.transform, ...nested] : nested;
    }
  }
  return null;
}

export function stillSelectable(snapshot: DiagramSnapshot, pageIndex: number, selection: VsdxShapeSelection): boolean {
  const page = snapshot.pages[pageIndex];
  return Boolean(page && page.id === selection.pageId && findShapePlacement(page.shapes, selection.shapeId));
}

/** Flattens nested group children. */
function flattenEditorShapes(shapes: readonly ShapeSnapshot[]): ShapeSnapshot[] {
  return shapes.flatMap((shape) => [shape, ...flattenEditorShapes(shape.children)]);
}

/** Nearest point on a known shape, in model inches. */
export function nearestPointOnShape(shapes: readonly ShapeSnapshot[], shapeId: string, at: ModelPoint): ConnectionPoint | null {
  const placement = findShapePlacement(shapes, shapeId);
  if (!placement || isConnectorShape(placement.shape)) return null;
  return nearestConnectionPointAnywhere(connectionPointsForShape(placement.shape), at);
}

/** Drop target forgiving of interior drops; falls back to the hit-tested shape. */
export function connectorTargetForPoint(shapes: readonly ShapeSnapshot[], handle: DiagramHandle, canvas: ModelPoint, at: ModelPoint): { shapeId: string; point: ConnectionPoint } | null {
  const direct = dropTargetForPoint(shapes, at);
  if (direct) return direct;
  let shapeId: string | null = null;
  try { shapeId = handle.hitTest(canvas.x, canvas.y)?.shapeId ?? null; } catch { shapeId = null; }
  if (!shapeId) return null;
  const point = nearestPointOnShape(shapes, shapeId, at);
  return point ? { shapeId, point } : null;
}

/** Pointer capture is best-effort; synthetic pointers must not fail the gesture. */
function capturePointer(event: PointerEvent<HTMLCanvasElement>): void {
  try { event.currentTarget.setPointerCapture(event.pointerId); } catch { void 0; }
}



export function collectDiagnostics(frame: PageDisplayList): TextDiagnostic[] { const result: TextDiagnostic[] = []; const work = frame.primitives.map((primitive) => ({ primitive, depth: 0 })); while (work.length) { const current = work.pop(); if (!current || current.depth >= 256) continue; if (current.primitive.kind === 'textBox') for (const paragraph of current.primitive.paragraphs) for (const run of paragraph.runs) result.push(...run.diagnostics); if (current.primitive.kind === 'group') for (const primitive of current.primitive.primitives) work.push({ primitive, depth: current.depth + 1 }); } return result; }
interface LoadedFont { key: string; face: FontFace; }
async function loadFonts(fonts: ReadonlyArray<VsdxFontFace>): Promise<LoadedFont[]> {
  if (typeof FontFace === 'undefined' || typeof document === 'undefined') return [];
  return Promise.all(fonts.map(async (font) => {
    const source = font.bytes.slice().buffer as ArrayBuffer;
    const face = await new FontFace(font.family, source, { style: font.italic ? 'italic' : 'normal', weight: font.bold ? '700' : '400' }).load();
    return { key: JSON.stringify([font.family, font.bold ?? false, font.italic ?? false]), face };
  }));
}
function installFonts(fonts: readonly LoadedFont[], installed: Map<string, FontFace>): void {
  for (const { key, face } of fonts) {
    const previous = installed.get(key);
    if (previous) document.fonts.delete?.(previous);
    document.fonts.add(face);
    installed.set(key, face);
  }
}
function useStableInitialUpdate(update: Uint8Array | undefined): Uint8Array | undefined { const stable = useRef(update); if (stable.current !== update && (!stable.current || !update || !bytesEqual(stable.current, update))) stable.current = update; return stable.current; }
function useStableFontFaces(fonts: ReadonlyArray<VsdxFontFace>): ReadonlyArray<VsdxFontFace> { const stable = useRef(fonts); if (!fontFacesEqual(stable.current, fonts)) stable.current = fonts; return stable.current; }
function fontFacesEqual(left: ReadonlyArray<VsdxFontFace>, right: ReadonlyArray<VsdxFontFace>): boolean { return left === right || (left.length === right.length && left.every((face, index) => fontFaceEqual(face, right[index]))); }
function fontFaceKeyEqual(left: VsdxFontFace, right: VsdxFontFace): boolean { return left.family === right.family && (left.bold ?? false) === (right.bold ?? false) && (left.italic ?? false) === (right.italic ?? false); }
function fontFaceEqual(left: VsdxFontFace, right: VsdxFontFace): boolean { return fontFaceKeyEqual(left, right) && bytesEqual(left.bytes, right.bytes); }
function bytesEqual(left: Uint8Array, right: Uint8Array): boolean { return left === right || (left.byteLength === right.byteLength && left.every((byte, index) => byte === right[index])); }
function resolveImage(assetId: string, handle: DiagramHandle | null, cache: { current: Map<string, Promise<CanvasImageSource | null>> }, message: string): Promise<CanvasImageSource | null> { const existing = cache.current.get(assetId); if (existing) return existing; const pending = decodeImage(handle?.mediaBytes(assetId), message); cache.current.set(assetId, pending); return pending; }
async function decodeImage(bytes: Uint8Array | undefined, message: string): Promise<CanvasImageSource | null> { if (!bytes) return null; const blob = new Blob([bytes.slice()]); if (typeof createImageBitmap === 'function') return createImageBitmap(blob); const url = URL.createObjectURL(blob); try { return await new Promise<HTMLImageElement>((resolve, reject) => { const image = new Image(); image.onload = () => resolve(image); image.onerror = () => reject(new Error(message)); image.src = url; }); } finally { URL.revokeObjectURL(url); } }
const styles: Record<string, CSSProperties> = { root: { display: 'flex', flexDirection: 'column', width: '100%', height: '100%', minHeight: 480, color: '#172033', background: '#f3f5f8', fontFamily: 'ui-sans-serif, system-ui, sans-serif' }, titleBar: { display: 'flex', alignItems: 'center', gap: 12, minHeight: 32, padding: '0 14px', background: '#f8fafc', borderBottom: '1px solid #d8dee9', fontSize: 13 }, contentRow: { display: 'flex', flex: 1, minHeight: 0 }, workspace: { position: 'relative', display: 'flex', flex: 1, alignItems: 'center', justifyContent: 'center', overflow: 'auto' }, canvasFrame: { position: 'relative', flex: '0 0 auto' }, canvas: { display: 'block', background: '#fff', boxShadow: '0 8px 32px rgba(27, 39, 61, 0.2)', touchAction: 'none' }, overlay: { position: 'absolute', inset: 0, pointerEvents: 'none' }, quickMenu: { position: 'absolute', zIndex: 3, display: 'flex', flexDirection: 'column', gap: 4, padding: 4, background: '#fff', border: '1px solid #d8dee9', borderRadius: 6, boxShadow: '0 8px 24px rgba(27, 39, 61, 0.18)', transform: 'translateY(-50%)' }, quickShape: { appearance: 'none', display: 'grid', placeItems: 'center', width: 32, height: 32, padding: 3, border: '1px solid transparent', borderRadius: 4, background: 'transparent', cursor: 'pointer' }, quickPreview: { width: 24, height: 24, overflow: 'visible', fill: '#fff', stroke: '#172033', strokeWidth: 0.05 }, selection: { position: 'absolute', top: 8, left: 8, padding: '4px 6px', color: '#fff', background: '#2563eb', fontSize: 12 }, integrity: { position: 'absolute', right: 14, bottom: 14, maxWidth: 340, padding: 12, color: '#7f1d1d', background: '#fef2f2', border: '1px solid #fca5a5' }, fidelity: { position: 'absolute', right: 14, bottom: 14, maxWidth: 340, padding: 8, color: '#475569', background: '#fff', fontSize: 12 }, error: { position: 'absolute', left: 14, right: 14, bottom: 14, padding: 10, color: '#8b1e2d', background: '#fff0f2', border: '1px solid #efb8c0' } };

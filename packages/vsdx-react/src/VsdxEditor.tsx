import { createT, deepMerge, diagnosticMessage, en } from '@betteroffice/vsdx-i18n';
import type { Translations } from '@betteroffice/vsdx-i18n';
import { canvasPointToModel, initWasm, openDiagram, paintPage, sizeCanvasForPage } from '@betteroffice/vsdx';
import type { Affine, PagePrimitive, CollaborationReplica, DiagramHandle, DiagramSnapshot, HitTestResult, ModelPoint, PageDisplayList, TextBoxPrimitive, TextDiagnostic, VsdxFontFace, VsdxPresence } from '@betteroffice/vsdx';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties, KeyboardEvent, MouseEvent, PointerEvent, ReactNode } from 'react';
import { Ribbon } from './components/ribbon/Ribbon';
import { RibbonCommandsProvider, copySelection, duplicateEntry, findShapePlacement, numericCellValue, pasteEntry } from './components/ribbon/commands';
import type { VsdxClipboardEntry } from './components/ribbon/clipboard';
import { DUPLICATE_OFFSET, PASTE_OFFSET } from './components/ribbon/clipboard';
import { ShapesPanel } from './components/shapes/ShapesPanel';
import { standardShapes } from './components/shapes/shapeLibrary';
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
  const editWrapRef = useRef<HTMLDivElement>(null);
  const editBoxRef = useRef<HTMLTextAreaElement>(null);
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
  const [clipboard, setClipboard] = useState<VsdxClipboardEntry | null>(null);
  const clipboardRef = useRef(clipboard);
  clipboardRef.current = clipboard;
  const [editing, setEditing] = useState<{ pageId: string; shapeId: string; initial: string; selectAll: boolean } | null>(null);
  const [draft, setDraft] = useState('');
  const editingRef = useRef(editing);
  const draftRef = useRef(draft);
  editingRef.current = editing;
  draftRef.current = draft;
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
  const [loading, setLoading] = useState(Boolean(file));
  onReadyRef.current = onReady;
  onChangeRef.current = onChange;
  onErrorRef.current = onError;
  collaborationRef.current = collaboration;

  const reportError = useCallback((value: unknown) => { const next = value instanceof Error ? value : new Error(String(value)); setError(next.message); onErrorRef.current?.(next); }, []);
  const enterTextEdit = useCallback((pageId: string, shapeId: string, override?: string) => {
    const handle = handleRef.current;
    if (!handle) return;
    let initial = '';
    try { initial = handle.shapeText(pageId, shapeId); }
    catch { initial = ''; }
    const value = override ?? initial;
    setDraft(value);
    draftRef.current = value;
    const next = { pageId, shapeId, initial, selectAll: override === undefined };
    setEditing(next);
    editingRef.current = next;
  }, []);
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
  const commitTextEdit = useCallback(() => {
    const current = editingRef.current;
    const handle = handleRef.current;
    if (!current || !handle) return;
    const text = draftRef.current;
    if (text === current.initial) {
      setEditing(null);
      editingRef.current = null;
      return;
    }
    try {
      handle.setShapeText(current.pageId, current.shapeId, text);
    } catch (value) { reportError(value); return; }
    setEditing(null);
    editingRef.current = null;
    refresh(undefined, true);
  }, [refresh, reportError]);

  useEffect(() => {
    sessionRef.current = { file, clientId: sessionClientId, initialUpdate };
    let disposed = false;
    let handle: DiagramHandle | null = null;
    let stopUpdates = () => {};
    let stopResync = () => {};
    handleRef.current?.dispose(); handleRef.current = null; imageCache.current.clear(); setSelection(null); setClipboard(null); setEditing(null); setDraft(''); modelRef.current = { snapshot: null, pageIndex: 0, frame: null }; setModel(modelRef.current); setError(null); setDirty(false);
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
    const dpr = window.devicePixelRatio || 1; sizeCanvasForPage(canvas, frame, dpr, zoom); context.clearRect(0, 0, canvas.width, canvas.height);
  }, [model.frame, selection, zoom]);

  useEffect(() => {
    if (!editing) return;
    if (!selection || selection.pageId !== editing.pageId || selection.shapeId !== editing.shapeId) setEditing(null);
  }, [editing, selection]);

  useEffect(() => {
    if (!editing) return;
    const box = editBoxRef.current;
    if (!box) return;
    box.focus();
    if (editing.selectAll) box.select();
    else box.setSelectionRange(box.value.length, box.value.length);
  }, [editing]);

  useEffect(() => {
    if (!editing) return;
    const box = editBoxRef.current;
    if (!box) return;
    box.style.height = 'auto';
    box.style.height = `${box.scrollHeight}px`;
  }, [editing, draft, zoom, model.frame]);

  useEffect(() => {
    if (!editing) return;
    const onDown = (event: globalThis.PointerEvent) => {
      if (editWrapRef.current?.contains(event.target as Node)) return;
      commitTextEdit();
    };
    document.addEventListener('pointerdown', onDown, true);
    return () => document.removeEventListener('pointerdown', onDown, true);
  }, [editing, commitTextEdit]);

  const onPointerDown = (event: PointerEvent<HTMLCanvasElement>) => {
    if (editingRef.current) commitTextEdit();
    const handle = handleRef.current; const frame = model.frame; const page = model.snapshot?.pages[model.pageIndex];
    if (!handle || !frame || !page) return;
    pointerRef.current = null;
    try {
      const point = canvasPointerPosition(event, frame);
      handle.layoutPage(model.pageIndex);
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
      event.currentTarget.setPointerCapture(event.pointerId);
    } catch (value) { reportError(value); }
  };
  const onDoubleClick = (event: MouseEvent<HTMLCanvasElement>) => {
    if (editingRef.current) return;
    const handle = handleRef.current; const frame = model.frame; const page = model.snapshot?.pages[model.pageIndex];
    if (!handle || !frame || !page) return;
    try {
      const point = canvasPointerPosition(event, frame);
      handle.layoutPage(model.pageIndex);
      const hit = handle.hitTest(point.canvas.x, point.canvas.y);
      if (!hit) return;
      setSelection({ pageId: page.id, shapeId: hit.shapeId, hit });
      enterTextEdit(page.id, hit.shapeId);
    } catch (value) { reportError(value); }
  };
  const copySelected = useCallback(() => {
    const handle = handleRef.current;
    if (!handle || !selection) return;
    try { setClipboard(copySelection(handle, selection)); } catch (value) { reportError(value); }
  }, [selection, reportError]);
  const cutSelected = useCallback(() => {
    const handle = handleRef.current;
    if (!handle || !selection) return;
    try {
      setClipboard(copySelection(handle, selection));
      handle.deleteShape(selection.pageId, selection.shapeId);
      refresh(undefined, true);
    } catch (value) { reportError(value); }
  }, [selection, refresh, reportError]);
  const pasteClipboardEntry = useCallback(() => {
    const handle = handleRef.current;
    const entry = clipboardRef.current;
    if (!handle || !entry) return;
    try {
      const target = modelRef.current.snapshot?.pages[modelRef.current.pageIndex]?.id ?? selection?.pageId ?? entry.pageId;
      const step = entry.pasteCount + 1;
      const { receipt, entry: next } = pasteEntry(handle, target, entry, PASTE_OFFSET.x * step, PASTE_OFFSET.y * step);
      setClipboard(next);
      setSelection({ pageId: target, shapeId: receipt.shapeId, hit: { kind: 'shape', shapeId: receipt.shapeId } });
      refresh(undefined, true);
    } catch (value) { reportError(value); }
  }, [selection, refresh, reportError]);
  const duplicateSelected = useCallback(() => {
    const handle = handleRef.current;
    if (!handle || !selection) return;
    try {
      const entry = copySelection(handle, selection);
      const receipt = duplicateEntry(handle, selection.pageId, entry, DUPLICATE_OFFSET.x, DUPLICATE_OFFSET.y);
      setSelection({ pageId: selection.pageId, shapeId: receipt.shapeId, hit: { kind: 'shape', shapeId: receipt.shapeId } });
      refresh(undefined, true);
    } catch (value) { reportError(value); }
  }, [selection, refresh, reportError]);
  const onCanvasKeyDown = (event: KeyboardEvent<HTMLCanvasElement>) => {
    const selected = selection;
    if (editingRef.current) return;
    if ((event.ctrlKey || event.metaKey) && !event.altKey) {
      const key = event.key.toLowerCase();
      if (key === 'c' && selected) { event.preventDefault(); copySelected(); return; }
      if (key === 'x' && selected) { event.preventDefault(); cutSelected(); return; }
      if (key === 'v' && clipboardRef.current) { event.preventDefault(); pasteClipboardEntry(); return; }
      if (key === 'd' && selected) { event.preventDefault(); duplicateSelected(); return; }
      return;
    }
    if (!selected) return;
    if (event.key === 'F2') return;
    if (event.key === 'Enter') { event.preventDefault(); enterTextEdit(selected.pageId, selected.shapeId); return; }
    if (isPrintableEntryKey(event)) { event.preventDefault(); enterTextEdit(selected.pageId, selected.shapeId, event.key); }
  };
  const onPointerUp = (event: PointerEvent<HTMLCanvasElement>) => {
    const pointer = pointerRef.current; pointerRef.current = null;
    const handle = handleRef.current; const selected = selection; const frame = model.frame;
    if (!pointer || !handle || !selected || !frame) return;
    try {
      const point = canvasPointerPosition(event, frame);
      if (Math.abs(point.canvas.x - pointer.canvas.x) < 0.01 && Math.abs(point.canvas.y - pointer.canvas.y) < 0.01) return;
      const geometry = resolveDragGeometry(pointer, point.model);
      if (pointer.resize) handle.resizeShape(selected.pageId, selected.shapeId, inchFormula(geometry.width), inchFormula(geometry.height));
      else handle.moveShape(selected.pageId, selected.shapeId, inchFormula(geometry.x), inchFormula(geometry.y));
      refresh(undefined, true);
    } catch (value) { reportError(value); }
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
  const editOverlay = editing && model.frame && model.snapshot ? textEditOverlay(model.frame, model.snapshot, model.pageIndex, editing, zoom) : null;
  return <div className={className} style={styles.root} aria-label={t('editor.appLabel')}>
    <header style={styles.titleBar}><strong>{t('ribbon.documentName')}</strong><span style={{ color: dirty ? '#a16207' : '#526273' }}>{dirty ? t('ribbon.dirty') : t('ribbon.saved')}</span></header>
    <RibbonCommandsProvider handle={handleRef.current} snapshot={model.snapshot} pageId={model.snapshot?.pages[model.pageIndex]?.id} selection={selection} clipboard={clipboard} onClipboardChange={setClipboard} onSelectShape={setSelection} onMutation={() => refresh(undefined, true)} onError={reportError} onDownload={download}>
    <Ribbon t={t} />
    <div style={styles.contentRow}>
    {leftPanel === undefined ? <ShapesPanel shapes={standardShapes} collapsed={shapesCollapsed} onToggleCollapsed={() => setShapesCollapsed((value) => !value)} onInsert={insertShape} t={t} /> : leftPanel}
    <main ref={workspaceRef} style={styles.workspace}>
      {loading && <span>{t('editor.opening')}</span>}
      {!loading && !model.frame && <span>{file ? t('editor.noPages') : t('editor.openPrompt')}</span>}
      <div style={styles.canvasFrame}>
        <canvas ref={mainCanvasRef} tabIndex={0} onPointerDown={onPointerDown} onPointerUp={onPointerUp} onPointerCancel={() => { pointerRef.current = null; }} onDoubleClick={onDoubleClick} onKeyDown={onCanvasKeyDown} aria-label={selection ? t('pages.canvasLabelWithSelection', { current: model.pageIndex + 1, total: model.snapshot?.pages.length ?? 0, name: selection.shapeId }) : t('pages.canvasLabel', { current: model.pageIndex + 1, total: model.snapshot?.pages.length ?? 0 })} style={styles.canvas} />
        <canvas ref={overlayCanvasRef} aria-hidden="true" style={styles.overlay} />
        {selection && !editing && <output style={styles.selection}>{t('shapes.selected', { name: selection.shapeId })}</output>}
        {editing && editOverlay && (
          <div ref={editWrapRef} style={{ ...styles.textEditWrap, left: editOverlay.rect.left, top: editOverlay.rect.top, width: editOverlay.rect.width, height: editOverlay.rect.height }}>
            <textarea
              ref={editBoxRef}
              value={draft}
              onChange={(event) => setDraft(event.target.value)}
              onKeyDown={(event) => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); commitTextEdit(); mainCanvasRef.current?.focus(); } else event.stopPropagation(); }}
              aria-label={editing.shapeId}
              rows={1}
              style={{
                ...styles.textEditBox,
                fontFamily: `"${editOverlay.font.family}", sans-serif`,
                fontSize: editOverlay.font.sizePx,
                fontWeight: editOverlay.font.bold ? 700 : 400,
                fontStyle: editOverlay.font.italic ? 'italic' : 'normal',
                color: editOverlay.font.color,
              }}
            />
          </div>
        )}
      </div>
      {integrity.length > 0 && <section role="alert" style={styles.integrity}><strong>{t('diagnostics.integrityHeading')}</strong>{integrity.map((item, index) => <div key={`${item.code}-${index}`}>{diagnosticMessage(t, item.category, item.code)}</div>)}</section>}
      {fidelity.length > 0 && <details style={styles.fidelity}><summary>{t('diagnostics.fidelityHeading')}</summary>{fidelity.map((item, index) => <div key={`${item.code}-${index}`}>{diagnosticMessage(t, item.category, item.code)}</div>)}</details>}
      {error && <div role="alert" style={styles.error}>{error}</div>}
    </main>
    </div>
    {statusBar === undefined ? <StatusBar pages={model.snapshot?.pages ?? []} activeIndex={model.pageIndex} onSelectPage={(index) => { if (editingRef.current) commitTextEdit(); refresh(index); }} onReorderPage={reorderPage} zoom={zoom} onZoomChange={setZoom} onFitToWindow={fitToWindow} t={t} /> : statusBar}
    </RibbonCommandsProvider>
  </div>;
}

const MIN_SHAPE_INCHES = 0.01;
const WORKSPACE_MARGIN = 32;

export function canvasPointerPosition(event: { clientX: number; clientY: number; currentTarget: HTMLCanvasElement }, frame: PageDisplayList): { canvas: ModelPoint; model: ModelPoint } {
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

export interface TextEditFont { family: string; sizePx: number; bold: boolean; italic: boolean; color: string; }
export interface TextEditRect { left: number; top: number; width: number; height: number; }
export interface TextEditOverlay { rect: TextEditRect; font: TextEditFont; }

export function isPrintableEntryKey(event: { key: string; ctrlKey?: boolean; metaKey?: boolean; altKey?: boolean }): boolean {
  return event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey;
}

export function findPrimitiveById(primitives: readonly PagePrimitive[], id: string, depth = 0): PagePrimitive | null {
  if (depth >= 256) return null;
  for (const primitive of primitives) {
    if (primitive.id === id) return primitive;
    if (primitive.kind === 'group') {
      const nested = findPrimitiveById(primitive.primitives, id, depth + 1);
      if (nested) return nested;
    }
  }
  return null;
}

export function findTextBoxById(primitives: readonly PagePrimitive[], id: string, depth = 0): TextBoxPrimitive | null {
  if (depth >= 256) return null;
  for (const primitive of primitives) {
    if (primitive.kind === 'textBox' && primitive.id === id) return primitive;
    if (primitive.kind === 'group') {
      const nested = findTextBoxById(primitive.primitives, id, depth + 1);
      if (nested) return nested;
    }
  }
  return null;
}

export function applyTextAffine(transform: Affine | undefined, x: number, y: number): { x: number; y: number } {
  const t = transform ?? { a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 };
  return { x: t.a * x + t.c * y + t.e, y: t.b * x + t.d * y + t.f };
}

export function collectGroupCorners(primitives: readonly PagePrimitive[], toCss: (sceneX: number, sceneY: number) => { x: number; y: number }, corners: Array<{ x: number; y: number }>, depth = 0): void {
  if (depth >= 256) return;
  for (const primitive of primitives) {
    if (primitive.kind === 'textBox' || primitive.kind === 'image') {
      for (const [lx, ly] of [[primitive.x, primitive.y], [primitive.x + primitive.width, primitive.y], [primitive.x, primitive.y + primitive.height], [primitive.x + primitive.width, primitive.y + primitive.height]] as const) {
        const scene = applyTextAffine(primitive.transform, lx, ly);
        corners.push(toCss(scene.x, scene.y));
      }
    } else if (primitive.kind === 'placeholder') {
      for (const [lx, ly] of [[primitive.x, primitive.y], [primitive.x + primitive.width, primitive.y], [primitive.x, primitive.y + primitive.height], [primitive.x + primitive.width, primitive.y + primitive.height]] as const) {
        corners.push(toCss(lx, ly));
      }
    } else if (primitive.kind === 'group') {
      collectGroupCorners(primitive.primitives, toCss, corners, depth + 1);
    }
  }
}

export function textEditOverlay(frame: PageDisplayList, snapshot: DiagramSnapshot, pageIndex: number, editing: { pageId: string; shapeId: string }, zoom: number): TextEditOverlay | null {
  const page = snapshot.pages[pageIndex];
  if (!page || page.id !== editing.pageId) return null;
  const placement = findShapePlacement(page.shapes, editing.shapeId);
  if (!placement) return null;
  const sourceId = `${page.sourcePartPath}:${placement.shape.sourceId}`;
  const textBox = findTextBoxById(frame.primitives, sourceId);
  const primitive = textBox ?? findPrimitiveById(frame.primitives, sourceId);
  const safeZoom = Number.isFinite(zoom) && zoom > 0 ? zoom : 1;
  const toCss = (sceneX: number, sceneY: number) => {
    const canvas = applyTextAffine(frame.paintTransform, sceneX, sceneY);
    return { x: canvas.x * safeZoom, y: canvas.y * safeZoom };
  };
  const corners: Array<{ x: number; y: number }> = [];
  let font: TextEditFont = { family: 'Calibri', sizePx: (10 / 72) * 96 * safeZoom, bold: false, italic: false, color: '#000000' };
  if (primitive?.kind === 'textBox') {
    const box = primitive as TextBoxPrimitive;
    const transform = box.transform;
    for (const [lx, ly] of [[box.x, box.y], [box.x + box.width, box.y], [box.x, box.y + box.height], [box.x + box.width, box.y + box.height]] as const) {
      const scene = applyTextAffine(transform, lx, ly);
      corners.push(toCss(scene.x, scene.y));
    }
    const run = box.paragraphs.flatMap((paragraph) => paragraph.runs).find((run) => run.text.length > 0) ?? box.paragraphs.flatMap((paragraph) => paragraph.runs)[0];
    if (run) font = { family: run.family || 'Calibri', sizePx: Math.max(1, run.sizeIn * 96 * safeZoom), bold: run.bold, italic: run.italic, color: run.color || '#000000' };
  } else if (primitive?.kind === 'shape') {
    let minX = Number.POSITIVE_INFINITY, minY = Number.POSITIVE_INFINITY, maxX = Number.NEGATIVE_INFINITY, maxY = Number.NEGATIVE_INFINITY;
    for (const command of primitive.path) {
      const record = command as Record<string, number | string>;
      for (const key of ['x', 'cpx', 'cp1x', 'cp2x']) {
        const x = record[key];
        const yKey = key === 'x' ? 'y' : key === 'cpx' ? 'cpy' : key === 'cp1x' ? 'cp1y' : 'cp2y';
        const y = record[yKey];
        if (typeof x === 'number' && typeof y === 'number' && Number.isFinite(x) && Number.isFinite(y)) {
          const css = toCss(x, y);
          minX = Math.min(minX, css.x); minY = Math.min(minY, css.y); maxX = Math.max(maxX, css.x); maxY = Math.max(maxY, css.y);
        }
      }
    }
    if (!Number.isFinite(minX) || !Number.isFinite(minY) || !Number.isFinite(maxX) || !Number.isFinite(maxY)) return null;
    return { rect: { left: minX, top: minY, width: Math.max(8, maxX - minX), height: Math.max(8, maxY - minY) }, font };
  } else if (primitive?.kind === 'image' || primitive?.kind === 'placeholder') {
    for (const [lx, ly] of [[primitive.x, primitive.y], [primitive.x + primitive.width, primitive.y], [primitive.x, primitive.y + primitive.height], [primitive.x + primitive.width, primitive.y + primitive.height]] as const) {
      const scene = primitive.kind === 'image' ? applyTextAffine(primitive.transform, lx, ly) : { x: lx, y: ly };
      corners.push(toCss(scene.x, scene.y));
    }
  } else if (primitive?.kind === 'group') {
    collectGroupCorners(primitive.primitives, toCss, corners);
    if (!corners.length) return null;
  } else return null;
  if (!corners.length) return null;
  const left = Math.min(...corners.map((corner) => corner.x));
  const top = Math.min(...corners.map((corner) => corner.y));
  const right = Math.max(...corners.map((corner) => corner.x));
  const bottom = Math.max(...corners.map((corner) => corner.y));
  return { rect: { left, top, width: Math.max(8, right - left), height: Math.max(8, bottom - top) }, font };
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
const styles: Record<string, CSSProperties> = { root: { display: 'flex', flexDirection: 'column', width: '100%', height: '100%', minHeight: 480, color: '#172033', background: '#f3f5f8', fontFamily: 'ui-sans-serif, system-ui, sans-serif' }, titleBar: { display: 'flex', alignItems: 'center', gap: 12, minHeight: 32, padding: '0 14px', background: '#f8fafc', borderBottom: '1px solid #d8dee9', fontSize: 13 }, contentRow: { display: 'flex', flex: 1, minHeight: 0 }, workspace: { position: 'relative', display: 'flex', flex: 1, alignItems: 'center', justifyContent: 'center', overflow: 'auto' }, canvasFrame: { position: 'relative', flex: '0 0 auto' }, canvas: { display: 'block', background: '#fff', boxShadow: '0 8px 32px rgba(27, 39, 61, 0.2)', touchAction: 'none' }, overlay: { position: 'absolute', inset: 0, pointerEvents: 'none' }, selection: { position: 'absolute', top: 8, left: 8, padding: '4px 6px', color: '#fff', background: '#2563eb', fontSize: 12 }, textEditWrap: { position: 'absolute', display: 'flex', alignItems: 'center', justifyContent: 'center', overflow: 'visible', background: 'transparent', border: '1px dashed #1d4ed8', zIndex: 2 }, textEditBox: { width: '100%', background: 'transparent', border: 'none', outline: 'none', resize: 'none', overflow: 'visible', textAlign: 'center', whiteSpace: 'pre-wrap', overflowWrap: 'break-word', wordBreak: 'break-word', lineHeight: 1.2, padding: 0, margin: 0 }, integrity: { position: 'absolute', right: 14, bottom: 14, maxWidth: 340, padding: 12, color: '#7f1d1d', background: '#fef2f2', border: '1px solid #fca5a5' }, fidelity: { position: 'absolute', right: 14, bottom: 14, maxWidth: 340, padding: 8, color: '#475569', background: '#fff', fontSize: 12 }, error: { position: 'absolute', left: 14, right: 14, bottom: 14, padding: 10, color: '#8b1e2d', background: '#fff0f2', border: '1px solid #efb8c0' } };

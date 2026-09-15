import { createContext, createElement, useContext, useMemo } from 'react';
import type { ReactNode } from 'react';
import type { DiagramHandle, DiagramSnapshot, FormulaShapeDraft, PageSnapshot, ShapeSnapshot } from '@betteroffice/vsdx';
import type { VsdxShapeSelection } from '../../VsdxEditor';
import { standardShapeById } from '../shapes/shapeLibrary';
import { DUPLICATE_OFFSET, PASTE_OFFSET, buildClipboardEntry, canCopyShape, draftForPaste, draftTreeForPaste, isTreeEntry } from './clipboard';
import type { VsdxClipboardEntry } from './clipboard';

export type RibbonCommandId =
  | 'undo' | 'redo' | 'delete' | 'cut' | 'copy' | 'paste' | 'duplicate'
  | 'fillColor' | 'lineColor' | 'lineWeight' | 'linePattern'
  | 'bringToFront' | 'bringForward' | 'sendBackward' | 'sendToBack'
  | 'rotateLeft' | 'rotateRight' | 'flipHorizontal' | 'flipVertical' | 'addShape' | 'download';

export interface RibbonCommand { id: RibbonCommandId; run: (value?: string) => void; enabled: boolean; active?: boolean; value?: string; }
export type RibbonCommands = Record<RibbonCommandId, RibbonCommand>;

export const RibbonCommandsContext = createContext<RibbonCommands | null>(null);

export interface RibbonCommandsProviderProps {
  handle: DiagramHandle | null;
  snapshot: DiagramSnapshot | null;
  pageId?: string;
  selection: VsdxShapeSelection | null;
  clipboard?: VsdxClipboardEntry | null;
  onClipboardChange?: (next: VsdxClipboardEntry | null) => void;
  onSelectShape?: (selection: VsdxShapeSelection) => void;
  onMutation: () => void;
  onError: (error: unknown) => void;
  onDownload: (bytes: Uint8Array) => void;
  children: ReactNode;
}

export interface ShapePlacement { shape: ShapeSnapshot; index: number; siblings: readonly ShapeSnapshot[]; }

export function findShapePlacement(shapes: readonly ShapeSnapshot[], shapeId: string, depth = 0): ShapePlacement | null {
  if (depth >= 256) return null;
  const index = shapes.findIndex((shape) => shape.id === shapeId);
  if (index >= 0) return { shape: shapes[index], index, siblings: shapes };
  for (const shape of shapes) {
    const nested = findShapePlacement(shape.children, shapeId, depth + 1);
    if (nested) return nested;
  }
  return null;
}

export function pageById(pages: readonly PageSnapshot[], pageId: string | undefined): PageSnapshot | null {
  return (pageId === undefined ? pages[0] : pages.find((page) => page.id === pageId)) ?? null;
}

function placementIn(pages: readonly PageSnapshot[], selection: VsdxShapeSelection | null): ShapePlacement | null {
  if (!selection) return null;
  const page = pages.find((item) => item.id === selection.pageId);
  return page ? findShapePlacement(page.shapes, selection.shapeId) : null;
}

function findCell(shape: ShapeSnapshot | null, name: string) {
  return shape?.cells.find((item) => item.locator.section === null && item.locator.row === null && item.locator.cellName === name);
}

export function cellValue(shape: ShapeSnapshot | null, name: string): string | undefined {
  const current = findCell(shape, name);
  return current?.value ?? current?.formula ?? undefined;
}

function cellFormula(shape: ShapeSnapshot | null, name: string): string | undefined {
  const current = findCell(shape, name);
  return current?.formula ?? current?.value ?? undefined;
}

function color(value: string | undefined, fallback: string): string {
  const hex = value?.match(/#[0-9a-f]{6}/i)?.[0];
  if (hex) return hex;
  const rgb = value?.match(/^RGB\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*\)$/i);
  return rgb ? `#${rgb.slice(1).map((channel) => Math.min(255, Number(channel)).toString(16).padStart(2, '0')).join('')}` : fallback;
}

function colorFormula(value = '#000000'): string {
  const hex = /^#[0-9a-f]{6}$/i.test(value) ? value.slice(1) : '000000';
  return `RGB(${[0, 2, 4].map((offset) => Number.parseInt(hex.slice(offset, offset + 2), 16)).join(',')})`;
}

export function numberValue(value: string | undefined): number {
  const result = Number(value ?? '0');
  return Number.isFinite(result) ? result : 0;
}

export function numericCellValue(shape: ShapeSnapshot, name: string, fallback?: number): number {
  const value = cellValue(shape, name);
  if (value === undefined && fallback !== undefined) return fallback;
  const parsed = value?.trim() ? Number(value) : Number.NaN;
  if (!Number.isFinite(parsed)) throw new Error(`Shape cell ${name} has no resolved numeric value.`);
  return parsed;
}

/** Snapshot the selection into an in-app clipboard entry, carrying the whole subtree. */
export function copySelection(handle: DiagramHandle, selection: VsdxShapeSelection): VsdxClipboardEntry {
  const placement = placementIn(handle.snapshot().pages, selection);
  if (!placement) throw new Error(`vsdx shape ${selection.shapeId} is no longer part of the diagram`);
  let text = '';
  try { text = handle.shapeText(selection.pageId, selection.shapeId); }
  catch { text = ''; }
  const textFor = (shape: ShapeSnapshot): string => {
    try { return handle.shapeText(selection.pageId, shape.id); }
    catch { return ''; }
  };
  const glue = subtreeGlueOf(handle, selection.pageId, selection.shapeId);
  return buildClipboardEntry(selection.pageId, placement.shape, text, { textFor, glue });
}

function subtreeGlueOf(handle: DiagramHandle, pageId: string, shapeId: string): VsdxClipboardEntry['glue'] {
  const subtree = (handle as unknown as { subtreeGlue?: (pageId: string, shapeId: string) => Array<{ connectorSource: string; endpoint: string; targetSource: string; toCell: string }> }).subtreeGlue;
  if (typeof subtree !== 'function') return [];
  return subtree.call(handle, pageId, shapeId).map((glue) => ({ connectorSource: glue.connectorSource, endpoint: glue.endpoint, targetSource: glue.targetSource, toCell: glue.toCell }));
}

/** Paste a clipboard entry with a model-space offset as one atomic shape addition. */
export function pasteEntry(handle: DiagramHandle, targetPageId: string, entry: VsdxClipboardEntry, dx: number, dy: number): { receipt: { shapeId: string }; entry: VsdxClipboardEntry } {
  const page = pageById(handle.snapshot().pages, targetPageId);
  if (!page) throw new Error(`vsdx page ${targetPageId} is no longer part of the diagram`);
  if (isTreeEntry(entry)) {
    const addTree = (handle as unknown as { addShapeTree?: (pageId: string, draft: unknown) => { shapeId: string } }).addShapeTree;
    if (typeof addTree !== 'function') throw new Error('vsdx group paste needs a diagram handle with addShapeTree');
    const receipt = addTree.call(handle, page.id, draftTreeForPaste(entry, dx, dy));
    return { receipt, entry: { ...entry, pasteCount: entry.pasteCount + 1 } };
  }
  const draft = draftForPaste(entry, dx, dy);
  const receipt = addShapeWithText(handle, page.id, draft, entry.text);
  return { receipt, entry: { ...entry, pasteCount: entry.pasteCount + 1 } };
}

/** Duplicate a clipboard entry without touching the clipboard, as one atomic addition. */
export function duplicateEntry(handle: DiagramHandle, targetPageId: string, entry: VsdxClipboardEntry, dx: number, dy: number): { shapeId: string } {
  const page = pageById(handle.snapshot().pages, targetPageId);
  if (!page) throw new Error(`vsdx page ${targetPageId} is no longer part of the diagram`);
  if (isTreeEntry(entry)) {
    const addTree = (handle as unknown as { addShapeTree?: (pageId: string, draft: unknown) => { shapeId: string } }).addShapeTree;
    if (typeof addTree !== 'function') throw new Error('vsdx group duplicate needs a diagram handle with addShapeTree');
    return addTree.call(handle, page.id, draftTreeForPaste(entry, dx, dy));
  }
  return addShapeWithText(handle, page.id, draftForPaste(entry, dx, dy), entry.text);
}

export function addShapeWithText(handle: DiagramHandle, pageId: string, draft: FormulaShapeDraft, text: string): { shapeId: string } {
  if (typeof (handle as { addShapeWithText?: unknown }).addShapeWithText === 'function') {
    return (handle as unknown as { addShapeWithText: (pageId: string, draft: FormulaShapeDraft, text: string) => { shapeId: string } }).addShapeWithText(pageId, draft, text);
  }
  const receipt = handle.addShape(pageId, draft);
  if (text) handle.setShapeText(pageId, receipt.shapeId, text);
  return receipt;
}

export function createRibbonCommands(
  handle: DiagramHandle | null,
  selection: VsdxShapeSelection | null,
  pageId: string | undefined,
  onMutation: () => void,
  onError: (error: unknown) => void,
  onDownload: (bytes: Uint8Array) => void,
  clipboard: VsdxClipboardEntry | null = null,
  onClipboardChange: (next: VsdxClipboardEntry | null) => void = () => {},
  onSelectShape: (selection: VsdxShapeSelection) => void = () => {}
): RibbonCommands {
  const execute = (operation: (current: DiagramHandle, selected: VsdxShapeSelection | null) => void, needsSelection = false) => () => {
    if (!handle || (needsSelection && !selection)) return;
    try { operation(handle, selection); onMutation(); } catch (error) { onError(error); }
  };
  const pages = handle ? handle.snapshot().pages : [];
  const current = placementIn(pages, selection);
  const shape = current?.shape ?? null;
  const selected = Boolean(current && selection);
  const copyable = Boolean(current && selection && canCopyShape(current.shape));
  const topIndex = current ? current.siblings.length - 1 : 0;
  const livePlacement = (currentHandle: DiagramHandle, currentSelection: VsdxShapeSelection | null) => placementIn(currentHandle.snapshot().pages, currentSelection);
  const formula = (cellName: string, value: string) => execute((currentHandle, currentSelection) => {
    currentHandle.setCellFormula(currentSelection!.pageId, currentSelection!.shapeId, { cellName }, value);
  }, true);
  const reorderTo = (target: (placement: ShapePlacement) => number, allowed: (placement: ShapePlacement) => boolean) => execute((currentHandle, currentSelection) => {
    const placement = livePlacement(currentHandle, currentSelection);
    if (placement && allowed(placement)) currentHandle.reorderShape(currentSelection!.pageId, currentSelection!.shapeId, target(placement));
  }, true);
  const setNumeric = (cellName: string, next: (value: number) => string) => execute((currentHandle, currentSelection) => {
    const placement = livePlacement(currentHandle, currentSelection);
    if (!placement) return;
    currentHandle.setCellFormula(currentSelection!.pageId, currentSelection!.shapeId, { cellName }, next(numericCellValue(placement.shape, cellName, 0)));
  }, true);
  const commands = {
    undo: { id: 'undo', enabled: Boolean(handle?.canUndo()), run: execute((currentHandle) => { currentHandle.undo(); }) },
    redo: { id: 'redo', enabled: Boolean(handle?.canRedo()), run: execute((currentHandle) => { currentHandle.redo(); }) },
    delete: { id: 'delete', enabled: selected, run: execute((currentHandle, currentSelection) => { currentHandle.deleteShape(currentSelection!.pageId, currentSelection!.shapeId); }, true) },
    fillColor: { id: 'fillColor', enabled: selected, value: color(cellValue(shape, 'FillForegnd'), '#000000'), run: (value?: string) => formula('FillForegnd', colorFormula(value))() },
    lineColor: { id: 'lineColor', enabled: selected, value: color(cellValue(shape, 'LineColor'), '#000000'), run: (value?: string) => formula('LineColor', colorFormula(value))() },
    lineWeight: { id: 'lineWeight', enabled: selected, value: cellFormula(shape, 'LineWeight'), run: (value?: string) => { if (value) formula('LineWeight', value)(); } },
    linePattern: { id: 'linePattern', enabled: selected, value: cellFormula(shape, 'LinePattern'), run: (value?: string) => { if (value) formula('LinePattern', value)(); } },
    bringToFront: { id: 'bringToFront', enabled: selected && current!.index < topIndex, run: reorderTo((placement) => placement.siblings.length - 1, (placement) => placement.index < placement.siblings.length - 1) },
    bringForward: { id: 'bringForward', enabled: selected && current!.index < topIndex, run: reorderTo((placement) => placement.index + 1, (placement) => placement.index < placement.siblings.length - 1) },
    sendBackward: { id: 'sendBackward', enabled: selected && current!.index > 0, run: reorderTo((placement) => placement.index - 1, (placement) => placement.index > 0) },
    sendToBack: { id: 'sendToBack', enabled: selected && current!.index > 0, run: reorderTo(() => 0, (placement) => placement.index > 0) },
    rotateLeft: { id: 'rotateLeft', enabled: selected, run: setNumeric('Angle', (value) => String(value - Math.PI / 2)) },
    rotateRight: { id: 'rotateRight', enabled: selected, run: setNumeric('Angle', (value) => String(value + Math.PI / 2)) },
    flipHorizontal: { id: 'flipHorizontal', enabled: selected, active: numberValue(cellValue(shape, 'FlipX')) !== 0, run: setNumeric('FlipX', (value) => value === 0 ? '1' : '0') },
    flipVertical: { id: 'flipVertical', enabled: selected, active: numberValue(cellValue(shape, 'FlipY')) !== 0, run: setNumeric('FlipY', (value) => value === 0 ? '1' : '0') },
    addShape: {
      id: 'addShape',
      enabled: Boolean(pageById(pages, pageId)),
      run: execute((currentHandle) => {
        const page = pageById(currentHandle.snapshot().pages, pageId);
        if (!page) throw new Error(`vsdx page ${pageId ?? ''} is no longer part of the diagram`);
        const rectangle = standardShapeById('rectangle');
        if (!rectangle) throw new Error('vsdx standard rectangle shape is unavailable');
        currentHandle.addShape(page.id, rectangle.draft(1, 1, 1, 1));
      }),
    },
    cut: {
      id: 'cut',
      enabled: copyable,
      run: () => {
        if (!handle || !selection) return;
        try {
          onClipboardChange(copySelection(handle, selection));
          handle.deleteShape(selection.pageId, selection.shapeId);
          onMutation();
        } catch (error) { onError(error); }
      },
    },
    copy: {
      id: 'copy',
      enabled: copyable,
      run: () => {
        if (!handle || !selection) return;
        try { onClipboardChange(copySelection(handle, selection)); } catch (error) { onError(error); }
      },
    },
    paste: {
      id: 'paste',
      enabled: Boolean(handle && clipboard && pageById(pages, pageId ?? selection?.pageId ?? clipboard.pageId)),
      run: () => {
        if (!handle || !clipboard) return;
        try {
          const target = pageId ?? selection?.pageId ?? clipboard.pageId;
          const step = clipboard.pasteCount + 1;
          const { receipt, entry } = pasteEntry(handle, target, clipboard, PASTE_OFFSET.x * step, PASTE_OFFSET.y * step);
          onClipboardChange(entry);
          onSelectShape({ pageId: target, shapeId: receipt.shapeId, hit: { kind: 'shape', shapeId: receipt.shapeId } });
          onMutation();
        } catch (error) { onError(error); }
      },
    },
    duplicate: {
      id: 'duplicate',
      enabled: copyable,
      run: () => {
        if (!handle || !selection) return;
        try {
          const entry = copySelection(handle, selection);
          const receipt = duplicateEntry(handle, selection.pageId, entry, DUPLICATE_OFFSET.x, DUPLICATE_OFFSET.y);
          onSelectShape({ pageId: selection.pageId, shapeId: receipt.shapeId, hit: { kind: 'shape', shapeId: receipt.shapeId } });
          onMutation();
        } catch (error) { onError(error); }
      },
    },
    download: { id: 'download', enabled: Boolean(handle), run: () => { if (!handle) return; try { onDownload(handle.save()); } catch (error) { onError(error); } } },
  } as RibbonCommands;
  return commands;
}

export function RibbonCommandsProvider({ handle, snapshot, pageId, selection, clipboard = null, onClipboardChange = () => {}, onSelectShape = () => {}, onMutation, onError, onDownload, children }: RibbonCommandsProviderProps) {
  const commands = useMemo(() => createRibbonCommands(handle, selection, pageId, onMutation, onError, onDownload, clipboard, onClipboardChange, onSelectShape), [handle, snapshot, pageId, selection, clipboard, onClipboardChange, onSelectShape, onMutation, onError, onDownload]);
  return createElement(RibbonCommandsContext.Provider, { value: commands }, children);
}

export function useRibbonCommands(): RibbonCommands {
  const context = useContext(RibbonCommandsContext);
  if (!context) throw new Error('useRibbonCommands must be used within a <RibbonCommandsProvider>');
  return context;
}

import { createContext, createElement, useContext, useMemo } from 'react';
import type { ReactNode } from 'react';
import type { DiagramHandle, DiagramSnapshot, PageDisplayList, PagePrimitive, PageSnapshot, PaletteEntry, ShapeSnapshot } from '@betteroffice/vsdx';
import type { VsdxShapeSelection } from '../../VsdxEditor';
import { standardShapeById } from '../shapes/shapeLibrary';

export type RibbonCommandId =
  | 'undo' | 'redo' | 'delete' | 'fillColor' | 'lineColor' | 'lineWeight' | 'linePattern'
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
  frame?: PageDisplayList | null;
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

function color(value: string | undefined, formula: string | undefined, palette: readonly PaletteEntry[] | undefined, fallback: string): string {
  for (const candidate of [value, formula]) {
    const hex = candidate?.match(/#[0-9a-f]{6}/i)?.[0];
    if (hex) return hex;
  }
  for (const candidate of [value, formula]) {
    const rgb = candidate?.match(/^RGB\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*\)$/i);
    if (rgb) return `#${rgb.slice(1).map((channel) => Math.min(255, Number(channel)).toString(16).padStart(2, '0')).join('')}`;
  }
  for (const candidate of [value, formula]) {
    const index = Number((candidate ?? '').trim());
    if (Number.isInteger(index)) {
      const entry = palette?.find((item) => item.index === index)?.color;
      if (entry) return entry;
    }
  }
  return fallback;
}

function swatchColor(shape: ShapeSnapshot | null, name: string, palette: readonly PaletteEntry[] | undefined, fallback: string): string {
  const cell = findCell(shape, name);
  return color(cell?.value ?? undefined, cell?.formula ?? undefined, palette, fallback);
}

/** Resolved fill and stroke hex for a shape primitive already evaluated by the renderer. */
export function swatchFromPrimitive(primitive: PagePrimitive | null): { fill?: string; stroke?: string } {
  if (!primitive || primitive.kind !== 'shape') return {};
  const fill = primitive.fill?.kind === 'solid'
    ? primitive.fill.color
    : primitive.fill?.kind === 'gradient' ? primitive.fill.stops[0]?.color : undefined;
  const stroke = primitive.kind === 'shape' ? primitive.stroke?.color : undefined;
  const hex = (value: string | undefined) => value?.match(/^#[0-9a-f]{6}$/i)?.[0];
  return { ...(hex(fill) ? { fill: hex(fill) } : {}), ...(hex(stroke) ? { stroke: hex(stroke) } : {}) };
}

export function findPrimitive(primitives: readonly PagePrimitive[], id: string, depth = 0): PagePrimitive | null {
  if (depth >= 256) return null;
  for (const primitive of primitives) {
    if (primitive.id === id) return primitive;
    if (primitive.kind === 'group') {
      const nested = findPrimitive(primitive.primitives, id, depth + 1);
      if (nested) return nested;
    }
  }
  return null;
}

/** Renderer-resolved swatch colours for the selected shape, if its primitive is present. */
export function resolvedSwatch(frame: PageDisplayList | null | undefined, page: PageSnapshot | null, shape: ShapeSnapshot | null): { fill?: string; stroke?: string } {
  if (!frame || !page || !shape) return {};
  return swatchFromPrimitive(findPrimitive(frame.primitives, `${page.sourcePartPath}:${shape.sourceId}`));
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

/** True when a ShapeSheet lock cell evaluates to the enabled value 1. */
export function lockCellEnabled(shape: ShapeSnapshot | null, name: string): boolean {
  return Number(cellValue(shape, name)) === 1;
}

/** True when the stored formula for a cell carries a GUARD interception. */
export function cellIsGuarded(shape: ShapeSnapshot | null, name: string): boolean {
  return hasGuardCall(cellFormula(shape, name) ?? '');
}

function hasGuardCall(formula: string): boolean {
  let index = 0;
  while (index < formula.length && formula[index] === '=') index += 1;
  const isIdentChar = (char: string) => /[A-Za-z0-9_.!]/.test(char);
  while (index < formula.length) {
    const char = formula[index];
    if (char === '"') {
      index += 1;
      while (index < formula.length) {
        if (formula[index] === '"') {
          if (formula[index + 1] === '"') { index += 2; continue; }
          index += 1;
          break;
        }
        index += 1;
      }
      continue;
    }
    if (/[A-Za-z_]/.test(char)) {
      let end = index + 1;
      while (end < formula.length && isIdentChar(formula[end])) end += 1;
      const candidate = formula.slice(index, end);
      let cursor = end;
      while (cursor < formula.length && /\s/.test(formula[cursor])) cursor += 1;
      if (cursor < formula.length && formula[cursor] === '(' && candidate.toUpperCase() === 'GUARD') return true;
      index = end;
      continue;
    }
    index += 1;
  }
  return false;
}

/** True when a delete would be refused by LockDelete or a GUARD on it. */
export function isDeleteBlocked(shape: ShapeSnapshot | null): boolean {
  if (!shape) return false;
  return lockCellEnabled(shape, 'LockDelete') || cellIsGuarded(shape, 'LockDelete');
}

export const HANDLE_RESIZE_LOCKS = ['LockMoveX', 'LockMoveY', 'LockWidth', 'LockHeight', 'LockAspect'] as const;

/** True when a handle resize would be refused by a lock or a GUARD on its pin or size. */
export function isHandleResizeBlocked(shape: ShapeSnapshot | null): boolean {
  if (!shape) return false;
  if (HANDLE_RESIZE_LOCKS.some((lock) => lockCellEnabled(shape, lock))) return true;
  return (['PinX', 'PinY', 'Width', 'Height'] as const).some((cell) => cellIsGuarded(shape, cell));
}

/** True when a single-cell write would be refused by a GUARD on that cell. */
export function isCellWriteBlocked(shape: ShapeSnapshot | null, cellName: string): boolean {
  if (!shape) return false;
  return cellIsGuarded(shape, cellName);
}

export function createRibbonCommands(
  handle: DiagramHandle | null,
  selection: VsdxShapeSelection | null,
  pageId: string | undefined,
  onMutation: () => void,
  onError: (error: unknown) => void,
  onDownload: (bytes: Uint8Array) => void,
  frame?: PageDisplayList | null
): RibbonCommands {
  const execute = (operation: (current: DiagramHandle, selected: VsdxShapeSelection | null) => void, needsSelection = false) => () => {
    if (!handle || (needsSelection && !selection)) return;
    try { operation(handle, selection); onMutation(); } catch (error) { onError(error); }
  };
  const diagram = handle ? handle.snapshot() : null;
  const pages = diagram?.pages ?? [];
  const palette = diagram?.palette;
  const current = placementIn(pages, selection);
  const shape = current?.shape ?? null;
  const selected = Boolean(current && selection);
  const activePage = pages.find((item) => item.id === selection?.pageId) ?? pageById(pages, pageId);
  const swatch = resolvedSwatch(frame ?? null, activePage, shape);
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
    delete: { id: 'delete', enabled: selected && !isDeleteBlocked(shape), run: execute((currentHandle, currentSelection) => { currentHandle.deleteShape(currentSelection!.pageId, currentSelection!.shapeId); }, true) },
    fillColor: { id: 'fillColor', enabled: selected && !isCellWriteBlocked(shape, 'FillForegnd'), value: swatch.fill ?? swatchColor(shape, 'FillForegnd', palette, '#000000'), run: (value?: string) => formula('FillForegnd', colorFormula(value))() },
    lineColor: { id: 'lineColor', enabled: selected && !isCellWriteBlocked(shape, 'LineColor'), value: swatch.stroke ?? swatchColor(shape, 'LineColor', palette, '#000000'), run: (value?: string) => formula('LineColor', colorFormula(value))() },
    lineWeight: { id: 'lineWeight', enabled: selected, value: cellFormula(shape, 'LineWeight'), run: (value?: string) => { if (value) formula('LineWeight', value)(); } },
    linePattern: { id: 'linePattern', enabled: selected, value: cellFormula(shape, 'LinePattern'), run: (value?: string) => { if (value) formula('LinePattern', value)(); } },
    bringToFront: { id: 'bringToFront', enabled: selected && current!.index < topIndex, run: reorderTo((placement) => placement.siblings.length - 1, (placement) => placement.index < placement.siblings.length - 1) },
    bringForward: { id: 'bringForward', enabled: selected && current!.index < topIndex, run: reorderTo((placement) => placement.index + 1, (placement) => placement.index < placement.siblings.length - 1) },
    sendBackward: { id: 'sendBackward', enabled: selected && current!.index > 0, run: reorderTo((placement) => placement.index - 1, (placement) => placement.index > 0) },
    sendToBack: { id: 'sendToBack', enabled: selected && current!.index > 0, run: reorderTo(() => 0, (placement) => placement.index > 0) },
    rotateLeft: { id: 'rotateLeft', enabled: selected && !isCellWriteBlocked(shape, 'Angle'), run: setNumeric('Angle', (value) => String(value - Math.PI / 2)) },
    rotateRight: { id: 'rotateRight', enabled: selected && !isCellWriteBlocked(shape, 'Angle'), run: setNumeric('Angle', (value) => String(value + Math.PI / 2)) },
    flipHorizontal: { id: 'flipHorizontal', enabled: selected && !isCellWriteBlocked(shape, 'FlipX'), active: numberValue(cellValue(shape, 'FlipX')) !== 0, run: setNumeric('FlipX', (value) => value === 0 ? '1' : '0') },
    flipVertical: { id: 'flipVertical', enabled: selected && !isCellWriteBlocked(shape, 'FlipY'), active: numberValue(cellValue(shape, 'FlipY')) !== 0, run: setNumeric('FlipY', (value) => value === 0 ? '1' : '0') },
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
    download: { id: 'download', enabled: Boolean(handle), run: () => { if (!handle) return; try { onDownload(handle.save()); } catch (error) { onError(error); } } },
  } as RibbonCommands;
  return commands;
}

export function RibbonCommandsProvider({ handle, snapshot, pageId, selection, frame, onMutation, onError, onDownload, children }: RibbonCommandsProviderProps) {
  const commands = useMemo(() => createRibbonCommands(handle, selection, pageId, onMutation, onError, onDownload, frame ?? null), [handle, snapshot, pageId, selection, frame, onMutation, onError, onDownload]);
  return createElement(RibbonCommandsContext.Provider, { value: commands }, children);
}

export function useRibbonCommands(): RibbonCommands {
  const context = useContext(RibbonCommandsContext);
  if (!context) throw new Error('useRibbonCommands must be used within a <RibbonCommandsProvider>');
  return context;
}

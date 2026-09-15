import { createContext, createElement, useContext, useMemo } from 'react';
import type { ReactNode } from 'react';
import type { DiagramHandle, DiagramSnapshot, PageSnapshot, ShapeSnapshot } from '@betteroffice/vsdx';
import type { VsdxShapeSelection } from '../../VsdxEditor';
import type { DragStart } from '../../interactions';
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

export function lockCellEnabled(shape: ShapeSnapshot | null, name: string): boolean {
  return Number(cellValue(shape, name)) === 1;
}

export function cellIsGuarded(shape: ShapeSnapshot | null, name: string): boolean {
  if (!shape) return false;
  const seen = new Set<string>();
  let current: string | undefined = name;
  for (let hop = 0; hop <= 10 && current !== undefined; hop += 1) {
    if (seen.has(current)) return true;
    seen.add(current);
    const formula = cellFormula(shape, current);
    if (formula === undefined) return false;
    if (hasGuardCall(formula)) return true;
    const target = setatrefTarget(formula);
    if (target === null) return mentionsSetatref(formula);
    if (!findCell(shape, target)) return true;
    current = target;
  }
  return true;
}

function stripQuoted(text: string): string {
  return text.replace(/"(?:""|[^"])*"/g, ' ');
}

function hasGuardCall(formula: string): boolean {
  return /(^|[^A-Za-z0-9_])GUARD\s*\(/i.test(stripQuoted(formula));
}

function mentionsSetatref(formula: string): boolean {
  return /(^|[^A-Za-z0-9_])SETATREF\s*\(/i.test(stripQuoted(formula));
}

function setatrefTarget(formula: string): string | null {
  const call = splitRootCall(formula);
  if (!call || call.name !== 'SETATREF' || call.args.length !== 1) return null;
  return /^[A-Za-z_][A-Za-z0-9_]*$/.test(call.args[0]) ? call.args[0] : null;
}

function splitRootCall(formula: string): { name: string; args: string[] } | null {
  const text = formula.replace(/^=+/, '').trim();
  const head = /^([A-Za-z_][A-Za-z0-9_.]*)\s*\(/.exec(text);
  if (!head || head.index !== 0) return null;
  let depth = 1;
  let quoted = false;
  let current = '';
  const args: string[] = [];
  for (let index = head[0].length; index < text.length; index += 1) {
    const ch = text[index];
    if (quoted) {
      if (ch === '"') {
        if (text[index + 1] === '"') { current += '""'; index += 1; }
        else quoted = false;
      } else current += ch;
      continue;
    }
    if (ch === '"') { quoted = true; continue; }
    if (ch === '(') { depth += 1; current += ch; continue; }
    if (ch === ')') {
      depth -= 1;
      if (depth === 0) break;
      current += ch;
      continue;
    }
    if (ch === ',' && depth === 1) { args.push(current); current = ''; continue; }
    current += ch;
  }
  if (depth !== 0 || quoted) return null;
  args.push(current);
  return { name: head[1].toUpperCase(), args: args.map((arg) => arg.trim()) };
}

export function isDeleteBlocked(shape: ShapeSnapshot | null): boolean {
  if (!shape) return false;
  return lockCellEnabled(shape, 'LockDelete') || cellIsGuarded(shape, 'LockDelete');
}

export const HANDLE_RESIZE_LOCKS = ['LockMoveX', 'LockMoveY', 'LockWidth', 'LockHeight', 'LockAspect'] as const;

export function isHandleResizeBlocked(shape: ShapeSnapshot | null): boolean {
  if (!shape) return false;
  if (HANDLE_RESIZE_LOCKS.some((lock) => lockCellEnabled(shape, lock))) return true;
  return (['PinX', 'PinY', 'Width', 'Height'] as const).some((cell) => cellIsGuarded(shape, cell));
}

export function isCellWriteBlocked(shape: ShapeSnapshot | null, cellName: string): boolean {
  if (!shape) return false;
  return cellIsGuarded(shape, cellName);
}

const DRAG_STALE_EPSILON = 1e-9;

export function dragStartMatchesShape(shape: ShapeSnapshot | null, start: DragStart): boolean {
  if (!shape) return false;
  try {
    const width = numericCellValue(shape, 'Width');
    const height = numericCellValue(shape, 'Height');
    if (Math.abs(width - start.size.width) > DRAG_STALE_EPSILON) return false;
    if (Math.abs(height - start.size.height) > DRAG_STALE_EPSILON) return false;
    if (Math.abs(numericCellValue(shape, 'PinX') - start.pin.x) > DRAG_STALE_EPSILON) return false;
    if (Math.abs(numericCellValue(shape, 'PinY') - start.pin.y) > DRAG_STALE_EPSILON) return false;
    if (Math.abs(numericCellValue(shape, 'LocPinX', width / 2) - (start.locPin?.x ?? start.size.width / 2)) > DRAG_STALE_EPSILON) return false;
    if (Math.abs(numericCellValue(shape, 'LocPinY', height / 2) - (start.locPin?.y ?? start.size.height / 2)) > DRAG_STALE_EPSILON) return false;
    if (Math.abs(numericCellValue(shape, 'Angle', 0) - (start.angle ?? 0)) > DRAG_STALE_EPSILON) return false;
    if ((numericCellValue(shape, 'FlipX', 0) === 1) !== Boolean(start.flipX)) return false;
    if ((numericCellValue(shape, 'FlipY', 0) === 1) !== Boolean(start.flipY)) return false;
    return true;
  } catch {
    return false;
  }
}

export function locPinAxisFractional(shape: ShapeSnapshot | null, name: string): boolean {
  const cell = findCell(shape, name);
  if (!cell) return true;
  const formula = (cell.formula ?? '').replace(/^=+/, '').trim();
  if (formula !== '') return !Number.isFinite(Number(formula));
  const value = (cell.value ?? '').trim();
  return value === '' || !Number.isFinite(Number(value));
}

export function createRibbonCommands(
  handle: DiagramHandle | null,
  selection: VsdxShapeSelection | null,
  pageId: string | undefined,
  onMutation: () => void,
  onError: (error: unknown) => void,
  onDownload: (bytes: Uint8Array) => void
): RibbonCommands {
  const execute = (operation: (current: DiagramHandle, selected: VsdxShapeSelection | null) => void, needsSelection = false) => () => {
    if (!handle || (needsSelection && !selection)) return;
    try { operation(handle, selection); onMutation(); } catch (error) { onError(error); }
  };
  const pages = handle ? handle.snapshot().pages : [];
  const current = placementIn(pages, selection);
  const shape = current?.shape ?? null;
  const selected = Boolean(current && selection);
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
    fillColor: { id: 'fillColor', enabled: selected, value: color(cellValue(shape, 'FillForegnd'), '#000000'), run: (value?: string) => formula('FillForegnd', colorFormula(value))() },
    lineColor: { id: 'lineColor', enabled: selected, value: color(cellValue(shape, 'LineColor'), '#000000'), run: (value?: string) => formula('LineColor', colorFormula(value))() },
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

export function RibbonCommandsProvider({ handle, snapshot, pageId, selection, onMutation, onError, onDownload, children }: RibbonCommandsProviderProps) {
  const commands = useMemo(() => createRibbonCommands(handle, selection, pageId, onMutation, onError, onDownload), [handle, snapshot, pageId, selection, onMutation, onError, onDownload]);
  return createElement(RibbonCommandsContext.Provider, { value: commands }, children);
}

export function useRibbonCommands(): RibbonCommands {
  const context = useContext(RibbonCommandsContext);
  if (!context) throw new Error('useRibbonCommands must be used within a <RibbonCommandsProvider>');
  return context;
}

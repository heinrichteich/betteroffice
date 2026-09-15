import { createContext, createElement, useContext, useMemo } from 'react';
import type { ReactNode } from 'react';
import type { DiagramHandle, DiagramSnapshot, PageSnapshot, ShapeSnapshot } from '@betteroffice/vsdx';
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
  selection: readonly VsdxShapeSelection[];
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

function placementsIn(pages: readonly PageSnapshot[], selection: readonly VsdxShapeSelection[]): Array<{ selection: VsdxShapeSelection; placement: ShapePlacement }> {
  const result: Array<{ selection: VsdxShapeSelection; placement: ShapePlacement }> = [];
  for (const item of selection) {
    const page = pages.find((entry) => entry.id === item.pageId);
    const placement = page ? findShapePlacement(page.shapes, item.shapeId) : null;
    if (placement) result.push({ selection: item, placement });
  }
  return result;
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
  return (cellFormula(shape, name) ?? '').toUpperCase().includes('GUARD');
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

export function isRotateBlocked(shape: ShapeSnapshot | null): boolean {
  if (!shape) return false;
  return lockCellEnabled(shape, 'LockRotate') || cellIsGuarded(shape, 'Angle');
}

export function isMoveBlocked(shape: ShapeSnapshot | null): boolean {
  if (!shape) return false;
  if (lockCellEnabled(shape, 'LockMoveX') || lockCellEnabled(shape, 'LockMoveY')) return true;
  return cellIsGuarded(shape, 'PinX') || cellIsGuarded(shape, 'PinY');
}

export function createRibbonCommands(
  handle: DiagramHandle | null,
  selection: readonly VsdxShapeSelection[],
  pageId: string | undefined,
  onMutation: () => void,
  onError: (error: unknown) => void,
  onDownload: (bytes: Uint8Array) => void
): RibbonCommands {
  const execute = (operation: (current: DiagramHandle, selected: readonly VsdxShapeSelection[]) => void, needsSelection = false) => () => {
    if (!handle || (needsSelection && selection.length === 0)) return;
    try { operation(handle, selection); onMutation(); } catch (error) { onError(error); }
  };
  const pages = handle ? handle.snapshot().pages : [];
  const placements = placementsIn(pages, selection);
  const first = placements[0];
  const single = placements.length === 1 ? placements[0] : null;
  const shape = first?.placement.shape ?? null;
  const selected = placements.length > 0;
  const topIndex = single ? single.placement.siblings.length - 1 : 0;
  const livePlacements = (currentHandle: DiagramHandle) => placementsIn(currentHandle.snapshot().pages, selection);
  const rollback = (currentHandle: DiagramHandle, applied: number) => {
    for (let index = 0; index < applied; index += 1) {
      try { currentHandle.undo(); } catch { break; }
    }
  };
  const preflightCellWrite = (live: ReturnType<typeof livePlacements>, cellName: string) => {
    for (const { placement } of live) {
      if (isCellWriteBlocked(placement.shape, cellName)) throw new Error('GUARD protects the requested cell');
    }
  };
  const formula = (cellName: string, value: string) => execute((currentHandle) => {
    const live = livePlacements(currentHandle);
    preflightCellWrite(live, cellName);
    let applied = 0;
    try {
      for (const { selection: item } of live) {
        currentHandle.setCellFormula(item.pageId, item.shapeId, { cellName }, value);
        applied += 1;
      }
    } catch (error) {
      rollback(currentHandle, applied);
      throw error;
    }
  }, true);
  const reorderTo = (target: (placement: ShapePlacement) => number, allowed: (placement: ShapePlacement) => boolean) => execute((currentHandle) => {
    const live = livePlacements(currentHandle);
    if (live.length !== 1) return;
    const { selection: item, placement } = live[0];
    if (allowed(placement)) currentHandle.reorderShape(item.pageId, item.shapeId, target(placement));
  }, true);
  const setNumeric = (cellName: string, next: (value: number) => string, lock?: { name: string; reason: string }) => execute((currentHandle) => {
    const live = livePlacements(currentHandle);
    const writes = live.map(({ selection: item, placement }) => {
      if (isCellWriteBlocked(placement.shape, cellName)) throw new Error('GUARD protects the requested cell');
      if (lock && lockCellEnabled(placement.shape, lock.name)) throw new Error(lock.reason);
      return { item, formula: next(numericCellValue(placement.shape, cellName, 0)) };
    });
    let applied = 0;
    try {
      for (const { item, formula: formulaValue } of writes) {
        currentHandle.setCellFormula(item.pageId, item.shapeId, { cellName }, formulaValue);
        applied += 1;
      }
    } catch (error) {
      rollback(currentHandle, applied);
      throw error;
    }
  }, true);
  const commands = {
    undo: { id: 'undo', enabled: Boolean(handle?.canUndo()), run: execute((currentHandle) => { currentHandle.undo(); }) },
    redo: { id: 'redo', enabled: Boolean(handle?.canRedo()), run: execute((currentHandle) => { currentHandle.redo(); }) },
    delete: { id: 'delete', enabled: selected && placements.every((entry) => !isDeleteBlocked(entry.placement.shape)), run: execute((currentHandle) => {
      const live = livePlacements(currentHandle);
      for (const { placement } of live) {
        if (lockCellEnabled(placement.shape, 'LockDelete')) throw new Error('LockDelete protects this delete gesture');
        if (cellIsGuarded(placement.shape, 'LockDelete')) throw new Error('GUARD protects the requested cell');
      }
      let applied = 0;
      try {
        for (const { selection: item } of live) {
          currentHandle.deleteShape(item.pageId, item.shapeId);
          applied += 1;
        }
      } catch (error) {
        rollback(currentHandle, applied);
        throw error;
      }
    }, true) },
    fillColor: { id: 'fillColor', enabled: selected && placements.every((entry) => !isCellWriteBlocked(entry.placement.shape, 'FillForegnd')), value: color(cellValue(shape, 'FillForegnd'), '#000000'), run: (value?: string) => formula('FillForegnd', colorFormula(value))() },
    lineColor: { id: 'lineColor', enabled: selected && placements.every((entry) => !isCellWriteBlocked(entry.placement.shape, 'LineColor')), value: color(cellValue(shape, 'LineColor'), '#000000'), run: (value?: string) => formula('LineColor', colorFormula(value))() },
    lineWeight: { id: 'lineWeight', enabled: selected && placements.every((entry) => !isCellWriteBlocked(entry.placement.shape, 'LineWeight')), value: cellFormula(shape, 'LineWeight'), run: (value?: string) => { if (value) formula('LineWeight', value)(); } },
    linePattern: { id: 'linePattern', enabled: selected && placements.every((entry) => !isCellWriteBlocked(entry.placement.shape, 'LinePattern')), value: cellFormula(shape, 'LinePattern'), run: (value?: string) => { if (value) formula('LinePattern', value)(); } },
    bringToFront: { id: 'bringToFront', enabled: single !== null && single.placement.index < topIndex, run: reorderTo((placement) => placement.siblings.length - 1, (placement) => placement.index < placement.siblings.length - 1) },
    bringForward: { id: 'bringForward', enabled: single !== null && single.placement.index < topIndex, run: reorderTo((placement) => placement.index + 1, (placement) => placement.index < placement.siblings.length - 1) },
    sendBackward: { id: 'sendBackward', enabled: single !== null && single.placement.index > 0, run: reorderTo((placement) => placement.index - 1, (placement) => placement.index > 0) },
    sendToBack: { id: 'sendToBack', enabled: single !== null && single.placement.index > 0, run: reorderTo(() => 0, (placement) => placement.index > 0) },
    rotateLeft: { id: 'rotateLeft', enabled: selected && placements.every((entry) => !isRotateBlocked(entry.placement.shape)), run: setNumeric('Angle', (value) => String(value - Math.PI / 2), { name: 'LockRotate', reason: 'LockRotate protects this rotate gesture' }) },
    rotateRight: { id: 'rotateRight', enabled: selected && placements.every((entry) => !isRotateBlocked(entry.placement.shape)), run: setNumeric('Angle', (value) => String(value + Math.PI / 2), { name: 'LockRotate', reason: 'LockRotate protects this rotate gesture' }) },
    flipHorizontal: { id: 'flipHorizontal', enabled: selected && placements.every((entry) => !isCellWriteBlocked(entry.placement.shape, 'FlipX')), active: numberValue(cellValue(shape, 'FlipX')) !== 0, run: setNumeric('FlipX', (value) => value === 0 ? '1' : '0') },
    flipVertical: { id: 'flipVertical', enabled: selected && placements.every((entry) => !isCellWriteBlocked(entry.placement.shape, 'FlipY')), active: numberValue(cellValue(shape, 'FlipY')) !== 0, run: setNumeric('FlipY', (value) => value === 0 ? '1' : '0') },
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

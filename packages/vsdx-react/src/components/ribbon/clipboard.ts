import type { FormulaShapeDraft, ShapeSnapshot } from '@betteroffice/vsdx';

export interface ClipboardCellLocator { cellName: string; section?: string; sectionIndex?: number; rowIndex?: number; rowName?: string; rowType?: string; }
export interface ClipboardCell { locator: ClipboardCellLocator; name: string; formula?: string; value?: string; rowType?: string; }
export interface VsdxClipboardEntry { pageId: string; name?: string; cells: ClipboardCell[]; text: string; pinX: number | null; pinY: number | null; pasteCount: number; }

export const PASTE_OFFSET = { x: 0.25, y: -0.25 } as const;
export const DUPLICATE_OFFSET = { x: -0.25, y: 0.25 } as const;

/** Leaf shapes copy losslessly; groups carry children a flat draft cannot preserve. */
export function canCopyShape(shape: ShapeSnapshot): boolean {
  return shape.children.length === 0;
}

/** Snapshot a shape into an in-app clipboard entry, preserving every cell formula. */
export function buildClipboardEntry(pageId: string, shape: ShapeSnapshot, text: string): VsdxClipboardEntry {
  if (!canCopyShape(shape)) throw new Error(`vsdx group copy is not supported for shape ${shape.id}`);
  const cells: ClipboardCell[] = shape.cells
    .filter((cell) => cell.locator.cellName.length > 0)
    .map((cell) => {
      const locator: ClipboardCellLocator = { cellName: cell.locator.cellName };
      if (cell.locator.section != null) locator.section = cell.locator.section;
      if (cell.locator.sectionIndex != null) locator.sectionIndex = cell.locator.sectionIndex;
      const row = cell.locator.row;
      if (row && 'index' in row) locator.rowIndex = (row as { index: number }).index;
      else if (row && 'name' in row) locator.rowName = (row as { name: string }).name;
      if (cell.rowType != null) locator.rowType = cell.rowType;
      const entry: ClipboardCell = { locator, name: cell.locator.cellName };
      if (cell.formula != null) entry.formula = cell.formula;
      if (cell.value != null) entry.value = cell.value;
      if (cell.rowType != null) entry.rowType = cell.rowType;
      return entry;
    });
  return { pageId, name: shape.name ?? undefined, cells, text, pinX: resolvedNumeric(shape, 'PinX'), pinY: resolvedNumeric(shape, 'PinY'), pasteCount: 0 };
}

/** Draft a pasted shape, offsetting PinX/PinY so the copy lands visibly apart. */
export function draftForPaste(entry: VsdxClipboardEntry, dx: number, dy: number): FormulaShapeDraft {
  const cells = entry.cells.map((cell) => ({
    locator: toDraftLocator(cell),
    name: cell.name,
    ...(cell.formula !== undefined ? { formula: cell.formula } : {}),
    ...(cell.value !== undefined ? { value: cell.value } : {}),
  }));
  applyPinOffset(cells, 'PinX', entry.pinX, dx);
  applyPinOffset(cells, 'PinY', entry.pinY, dy);
  return { ...(entry.name !== undefined ? { name: entry.name } : {}), cells };
}

/** Resolved numeric value of a root cell, or null when absent or non-numeric. */
export function resolvedNumeric(shape: ShapeSnapshot, name: string): number | null {
  const cell = shape.cells.find((item) => item.locator.section == null && item.locator.row == null && item.locator.cellName === name);
  const raw = cell?.value ?? cell?.formula;
  if (raw == null || !raw.trim()) return null;
  const parsed = Number(raw);
  return Number.isFinite(parsed) ? parsed : null;
}

function toDraftLocator(cell: ClipboardCell): ClipboardCellLocator {
  const locator: ClipboardCellLocator = { cellName: cell.locator.cellName };
  if (cell.locator.section !== undefined) locator.section = cell.locator.section;
  if (cell.locator.sectionIndex !== undefined) locator.sectionIndex = cell.locator.sectionIndex;
  if (cell.locator.rowIndex !== undefined) locator.rowIndex = cell.locator.rowIndex;
  if (cell.locator.rowName !== undefined) locator.rowName = cell.locator.rowName;
  if (cell.locator.rowType !== undefined) locator.rowType = cell.locator.rowType;
  return locator;
}

function applyPinOffset(cells: Array<{ locator: ClipboardCellLocator; name: string; formula?: string; value?: string }>, name: 'PinX' | 'PinY', base: number | null, delta: number): void {
  const next = base == null ? delta : base + delta;
  const formula = toFormula(next);
  const existing = cells.find((cell) => cell.locator.section === undefined && cell.locator.rowIndex === undefined && cell.locator.rowName === undefined && cell.name === name);
  if (existing) { existing.formula = formula; delete existing.value; }
  else cells.push({ locator: { cellName: name }, name, formula });
}

/** Canonical ShapeSheet number literal. */
export function toFormula(value: number): string {
  const rounded = Number(value.toFixed(6));
  return String(Object.is(rounded, -0) ? 0 : rounded);
}

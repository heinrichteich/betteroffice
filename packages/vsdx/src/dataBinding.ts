import { quoteShapeDataValue, shapeDataRows, shapeDataValueFormula, unquoteFormula } from './shapeData';
import type { ShapeDataType } from './shapeData';
import type { DiagramSnapshot, FormulaShapeDraft, ShapeSnapshot } from './types';
import type { DiagramHandle } from './wasm/loader';

/** One imported table: every grid row is kept in `records`; `rows` is the view under the key column. */
export interface DataTable {
  name: string;
  columns: string[];
  keyColumn: string;
  records: Array<Record<string, string>>;
  rows: DataTableRow[];
  skippedRows: number;
}

export interface DataTableRow {
  key: string;
  values: Record<string, string>;
}

/** Where a table came from: an xlsx sheet or a csv file the user already had. */
export interface DataBindingSource {
  kind: 'xlsx' | 'csv';
  name: string;
  sheet: string | null;
  keyColumn: string;
}

/** One row-to-shape link. Values stay on the shape when its row vanishes. */
export interface ShapeBinding {
  shapeId: string;
  pagePart: string;
  sourceId: number;
  shapeName: string | null;
  key: string;
  status: 'ok' | 'stale';
  appliedRows: string[];
}

/** The persisted link document: one entry per bound shape. */
export interface DataBindingDoc {
  source: DataBindingSource;
  bindings: ShapeBinding[];
}

export interface ColumnMapping {
  mapped: Array<{ column: string; rowName: string }>;
  unmappedColumns: string[];
}

export interface BindingRefusal {
  column: string;
  reason: string;
}

export interface BindOutcome {
  binding: ShapeBinding;
  applied: string[];
  refused: BindingRefusal[];
  unmappedColumns: string[];
}

export interface RefreshReport {
  updated: string[];
  stale: Array<{ shapeId: string; key: string }>;
  removed: Array<{ shapeId: string; key: string; reason: string }>;
  refused: Array<{ shapeId: string } & BindingRefusal>;
  unmappedColumns: string[];
}

const LINK_SHAPE_PREFIX = 'BO_DataLink';
const SOURCE_ROW = 'BO_Source';
const BIND_ROW_PREFIX = 'BO_Bind_';
const MAX_BIND_CELLS = 2000;

/** Build a table from display strings; first row holds the column names. */
export function tableFromGrid(grid: string[][], name: string, keyColumn?: string): DataTable {
  if (grid.length === 0 || grid[0].length === 0) throw new Error(`data table "${name}" is empty`);
  const columns = dedupeColumns(grid[0].map((cell, index) => {
    const label = cell.trim();
    return label === '' ? `Column ${columnLetters(index)}` : label;
  }));
  const resolvedKey = keyColumn ?? columns[0];
  if (!columns.includes(resolvedKey)) throw new Error(`key column "${keyColumn}" is not in data table "${name}"`);
  const records: Array<Record<string, string>> = [];
  for (const cells of grid.slice(1)) {
    if (cells.every((cell) => cell.trim() === '')) continue;
    const values: Record<string, string> = {};
    columns.forEach((column, index) => { values[column] = (cells[index] ?? '').trim(); });
    records.push(values);
  }
  return keyedTable(name, columns, records, resolvedKey);
}

function keyedTable(name: string, columns: string[], records: Array<Record<string, string>>, keyColumn: string): DataTable {
  const rows: DataTableRow[] = [];
  const seen = new Set<string>();
  for (const values of records) {
    const key = values[keyColumn] ?? '';
    if (key === '' || seen.has(key)) continue;
    seen.add(key);
    rows.push({ key, values });
  }
  return { name, columns, keyColumn, records, rows, skippedRows: records.length - rows.length };
}

/** Parse csv text into a table; delimiter is the most common of comma, semicolon, tab. */
export function tableFromCsv(text: string, name: string, keyColumn?: string): DataTable {
  const normalized = text.replace(/\r\n/g, '\n').replace(/\r/g, '\n');
  const lines = splitCsvLines(normalized);
  if (lines.length === 0) throw new Error(`data table "${name}" is empty`);
  const delimiter = pickDelimiter(lines[0]);
  return tableFromGrid(lines.map((line) => parseCsvLine(line, delimiter)), name, keyColumn);
}

function splitCsvLines(text: string): string[] {
  const lines: string[] = [];
  let current = '';
  let quoted = false;
  for (let index = 0; index < text.length; index++) {
    const char = text[index];
    if (char === '"') {
      if (quoted && text[index + 1] === '"') { current += '""'; index++; continue; }
      quoted = !quoted;
      current += char;
      continue;
    }
    if (char === '\n' && !quoted) { lines.push(current); current = ''; continue; }
    current += char;
  }
  if (current !== '' || lines.length === 0) lines.push(current);
  return lines.filter((line, index) => index === 0 || line.trim() !== '');
}

function pickDelimiter(header: string): string {
  const counts = [',', ';', '\t'].map((delimiter) => ({ delimiter, count: header.split(delimiter).length }));
  counts.sort((left, right) => right.count - left.count);
  return counts[0].delimiter;
}

function parseCsvLine(line: string, delimiter: string): string[] {
  const cells: string[] = [];
  let current = '';
  let quoted = false;
  for (let index = 0; index < line.length; index++) {
    const char = line[index];
    if (quoted) {
      if (char === '"') {
        if (line[index + 1] === '"') { current += '"'; index++; } else quoted = false;
      } else current += char;
      continue;
    }
    if (char === '"') { quoted = true; continue; }
    if (char === delimiter) { cells.push(current); current = ''; continue; }
    current += char;
  }
  cells.push(current);
  return cells;
}

function dedupeColumns(columns: string[]): string[] {
  const seen = new Map<string, number>();
  return columns.map((column) => {
    const count = seen.get(column) ?? 0;
    seen.set(column, count + 1);
    return count === 0 ? column : `${column} (${count + 1})`;
  });
}

/** Re-key an imported table on another column; every imported row stays available. */
export function rekeyTable(table: DataTable, keyColumn: string): DataTable {
  if (!table.columns.includes(keyColumn)) throw new Error(`key column "${keyColumn}" is not in data table "${table.name}"`);
  if (keyColumn === table.keyColumn) return table;
  return keyedTable(table.name, table.columns, table.records, keyColumn);
}

/** Zero-based column index to A1 letters. */
export function columnLetters(index: number): string {
  let letters = '';
  let value = index;
  do {
    letters = String.fromCharCode(65 + (value % 26)) + letters;
    value = Math.floor(value / 26) - 1;
  } while (value >= 0);
  return letters;
}

/** Minimal xlsx surface used for table import; the suite engine satisfies it. */
export interface WorkbookTableReader {
  rangeCells(sheet: number, range: string): Array<Array<{ input: string }>>;
}

/** Read a trimmed string grid from a workbook sheet through the xlsx engine. */
export function readGridFromWorkbook(reader: WorkbookTableReader, sheet: number, maxRows = 1000, maxCols = 52): string[][] {
  const range = `A1:${columnLetters(Math.max(0, maxCols - 1))}${Math.max(1, maxRows)}`;
  const grid = reader.rangeCells(sheet, range).map((row) => row.map((cell) => cell.input ?? ''));
  while (grid.length > 0 && grid[grid.length - 1].every((cell) => cell.trim() === '')) grid.pop();
  const width = grid.reduce((max, row) => {
    let end = row.length;
    while (end > 0 && row[end - 1].trim() === '') end--;
    return Math.max(max, end);
  }, 0);
  return grid.map((row) => row.slice(0, width));
}

/** Read the active table of a workbook sheet: first row names the columns. */
export function tableFromWorkbook(reader: WorkbookTableReader, sheet: number, name: string, keyColumn?: string): DataTable {
  return tableFromGrid(readGridFromWorkbook(reader, sheet), name, keyColumn);
}

/** Value formula for imported text: numeric text stays numeric, the rest stays visible. */
export function bindingValueFormula(type: ShapeDataType, text: string): string {
  if ((type === 'number' || type === 'currency') && text.trim() !== '' && !/^[+-]?(\d+(\.\d*)?|\.\d+)([eE][+-]?\d+)?$/.test(text.trim())) {
    return quoteShapeDataValue(text);
  }
  return shapeDataValueFormula(type, text);
}

/** Match table columns onto a shape's Property rows by row name, then label. */
export function planColumnMapping(shape: ShapeSnapshot | null, table: DataTable): ColumnMapping {
  const rows = shapeDataRows(shape);
  const mapped: ColumnMapping['mapped'] = [];
  const unmappedColumns: string[] = [];
  for (const column of table.columns) {
    const lowered = column.toLowerCase();
    const match = rows.find((row) => (row.rowName ?? '').toLowerCase() === lowered)
      ?? rows.find((row) => row.label.toLowerCase() === lowered);
    if (match?.rowName) mapped.push({ column, rowName: match.rowName });
    else unmappedColumns.push(column);
  }
  return { mapped, unmappedColumns };
}

/** Identity of an imported table: its kind, file and sheet, independent of the key column. */
export function bindingSourceId(source: DataBindingSource): string {
  return JSON.stringify([source.kind, source.name, source.sheet ?? null]);
}

/** Bookkeeping shape name for one imported table; identity lives in the document, not in this label. */
export function linkShapeName(source: DataBindingSource): string {
  const label = source.sheet === null ? source.name : `${source.name} [${source.sheet}]`;
  const sanitized = label.replace(/\s+/g, ' ').trim().slice(0, 60) || 'Table';
  return `${LINK_SHAPE_PREFIX} ${sanitized}`;
}

/** Draft cells for the invisible link shape carrying the binding document. */
export function bindingDocCells(doc: DataBindingDoc): FormulaShapeDraft['cells'] {
  if (doc.bindings.length > MAX_BIND_CELLS) throw new Error(`data binding for "${doc.source.name}" exceeds ${MAX_BIND_CELLS} links`);
  const property = (row: string, cell: string, formula: string) => ({
    locator: { section: 'Property', rowName: row, cellName: cell }, name: cell, formula,
  });
  const cells: FormulaShapeDraft['cells'] = [
    { locator: { cellName: 'PinX' }, name: 'PinX', formula: '1' },
    { locator: { cellName: 'PinY' }, name: 'PinY', formula: '1' },
    { locator: { cellName: 'Width' }, name: 'Width', formula: '1' },
    { locator: { cellName: 'Height' }, name: 'Height', formula: '1' },
    { locator: { cellName: 'LocPinX' }, name: 'LocPinX', formula: 'Width*0.5' },
    { locator: { cellName: 'LocPinY' }, name: 'LocPinY', formula: 'Height*0.5' },
    { locator: { section: 'Geometry', rowIndex: 0, rowType: 'MoveTo', cellName: 'X' }, name: 'X', formula: '0' },
    { locator: { section: 'Geometry', rowIndex: 0, rowType: 'MoveTo', cellName: 'Y' }, name: 'Y', formula: '0' },
    { locator: { section: 'Geometry', rowIndex: 0, rowType: 'MoveTo', cellName: 'NoShow' }, name: 'NoShow', formula: '1' },
    property(SOURCE_ROW, 'Label', quoteShapeDataValue('Data source')),
    property(SOURCE_ROW, 'Value', quoteShapeDataValue(JSON.stringify(doc.source))),
    property(SOURCE_ROW, 'Invisible', '1'),
  ];
  doc.bindings.forEach((binding, index) => {
    const row = `${BIND_ROW_PREFIX}${index}`;
    cells.push(property(row, 'Label', quoteShapeDataValue(`Binding ${index + 1}`)));
    cells.push(property(row, 'Value', quoteShapeDataValue(JSON.stringify(binding))));
    cells.push(property(row, 'Invisible', '1'));
  });
  return cells;
}

/** Read the binding document off a link shape; null when the shape carries none. */
export function parseBindingDoc(shape: ShapeSnapshot | null | undefined): DataBindingDoc | null {
  if (!shape) return null;
  const rows = shapeDataRows(shape);
  const sourceRow = rows.find((row) => row.rowName === SOURCE_ROW);
  if (!sourceRow) return null;
  let source: DataBindingSource;
  try {
    source = JSON.parse(unquoteFormula(sourceRow.formula) ?? sourceRow.displayValue) as DataBindingSource;
  } catch { return null; }
  if (!source || typeof source !== 'object' || typeof source.keyColumn !== 'string') return null;
  const bindings: ShapeBinding[] = [];
  const bindRows = rows
    .filter((row) => row.rowName?.startsWith(BIND_ROW_PREFIX))
    .sort((left, right) => bindRowOrder(left.rowName) - bindRowOrder(right.rowName));
  for (const row of bindRows) {
    let binding: ShapeBinding;
    try {
      binding = JSON.parse(unquoteFormula(row.formula) ?? row.displayValue) as ShapeBinding;
    } catch { return null; }
    if (!binding || typeof binding.shapeId !== 'string' || typeof binding.key !== 'string') return null;
    bindings.push({
      shapeId: binding.shapeId,
      pagePart: typeof binding.pagePart === 'string' ? binding.pagePart : '',
      sourceId: typeof binding.sourceId === 'number' ? binding.sourceId : 0,
      shapeName: typeof binding.shapeName === 'string' ? binding.shapeName : null,
      key: binding.key,
      status: binding.status === 'stale' ? 'stale' : 'ok',
      appliedRows: Array.isArray(binding.appliedRows) ? binding.appliedRows.filter((entry): entry is string => typeof entry === 'string') : [],
    });
  }
  return { source, bindings };
}

function bindRowOrder(rowName: string | null): number {
  const order = Number(rowName?.slice(BIND_ROW_PREFIX.length));
  return Number.isFinite(order) ? order : Number.MAX_SAFE_INTEGER;
}

/** Locate the link shape whose stored source matches, anywhere in the snapshot. */
export function findBindingShape(snapshot: DiagramSnapshot, source: DataBindingSource): ShapeSnapshot | null {
  const wanted = bindingSourceId(source);
  for (const page of snapshot.pages) {
    for (const shape of collectShapes(page.shapes)) {
      if (!shape.name?.startsWith(`${LINK_SHAPE_PREFIX} `)) continue;
      const doc = parseBindingDoc(shape);
      if (doc && bindingSourceId(doc.source) === wanted) return shape;
    }
  }
  return null;
}

/** Locate a bound target: snapshot id first, then a unique source id whose name still agrees. */
export function findBoundShape(snapshot: DiagramSnapshot, binding: ShapeBinding): { pageId: string; shape: ShapeSnapshot } | null {
  for (const page of snapshot.pages) {
    const direct = findShapeById(page.shapes, binding.shapeId);
    if (direct) return { pageId: page.id, shape: direct };
  }
  const pages = snapshot.pages.filter((page) => binding.pagePart === '' || page.sourcePartPath === binding.pagePart);
  const pool = pages.flatMap((page) => collectShapes(page.shapes).map((shape) => ({ pageId: page.id, shape })));
  if (binding.sourceId !== 0) {
    const bySource = pool.filter((entry) => entry.shape.sourceId === binding.sourceId
      && (binding.shapeName === null || entry.shape.name === binding.shapeName));
    if (bySource.length === 1) return bySource[0];
  }
  return null;
}

function findShapeById(shapes: ShapeSnapshot[], id: string): ShapeSnapshot | null {
  for (const shape of shapes) {
    if (shape.id === id) return shape;
    const nested = findShapeById(shape.children, id);
    if (nested) return nested;
  }
  return null;
}

function collectShapes(shapes: ShapeSnapshot[]): ShapeSnapshot[] {
  return shapes.flatMap((shape) => [shape, ...collectShapes(shape.children)]);
}

/** Persist the binding document: the replacement is built and added before the previous one is dropped. */
export function writeBindingDoc(handle: DiagramHandle, snapshot: DiagramSnapshot, doc: DataBindingDoc): string {
  if (snapshot.pages.length === 0) throw new Error(`cannot store data binding for "${doc.source.name}" in a diagram without pages`);
  const cells = bindingDocCells(doc);
  const wanted = bindingSourceId(doc.source);
  const receipt = handle.addShape(snapshot.pages[0].id, { name: linkShapeName(doc.source), cells });
  for (const page of snapshot.pages) {
    for (const shape of collectShapes(page.shapes)) {
      if (shape.id === receipt.shapeId || !shape.name?.startsWith(`${LINK_SHAPE_PREFIX} `)) continue;
      const existing = parseBindingDoc(shape);
      if (existing && bindingSourceId(existing.source) === wanted) handle.deleteShape(page.id, shape.id);
    }
  }
  return receipt.shapeId;
}

/** Read the stored binding document for a source; null when never bound. */
export function readBindingDoc(snapshot: DiagramSnapshot, source: DataBindingSource): DataBindingDoc | null {
  return parseBindingDoc(findBindingShape(snapshot, source));
}

function refusalReason(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function applyRowValues(handle: DiagramHandle, pageId: string, shapeId: string, shape: ShapeSnapshot, table: DataTable, record: DataTableRow): { applied: string[]; refused: BindingRefusal[]; unmappedColumns: string[]; appliedRows: string[] } {
  const mapping = planColumnMapping(shape, table);
  const rows = shapeDataRows(shape);
  const applied: string[] = [];
  const refused: BindingRefusal[] = [];
  const appliedRows: string[] = [];
  for (const { column, rowName } of mapping.mapped) {
    const row = rows.find((candidate) => candidate.rowName === rowName)!;
    try {
      handle.setCellFormula(pageId, shapeId, { cellName: 'Value', section: 'Property', ...(row.sectionIndex !== undefined ? { sectionIndex: row.sectionIndex } : {}), rowName }, bindingValueFormula(row.type, record.values[column] ?? ''));
      applied.push(column);
      appliedRows.push(rowName);
    } catch (error) {
      refused.push({ column, reason: refusalReason(error) });
    }
  }
  return { applied, refused, unmappedColumns: mapping.unmappedColumns, appliedRows };
}

/** Bind one table row to a shape: columns land in matching Property rows via the mutation policy. */
export function bindRow(handle: DiagramHandle, snapshot: DiagramSnapshot, table: DataTable, source: DataBindingSource, pageId: string, shapeId: string, key: string): BindOutcome {
  const record = table.rows.find((row) => row.key === key);
  if (!record) throw new Error(`row "${key}" is not in data table "${table.name}"`);
  const page = snapshot.pages.find((candidate) => candidate.id === pageId);
  const shape = page ? findShapeById(page.shapes, shapeId) : null;
  if (!page || !shape) throw new Error(`shape "${shapeId}" was not found`);
  const { applied, refused, unmappedColumns, appliedRows } = applyRowValues(handle, pageId, shapeId, shape, table, record);
  const current = readBindingDoc(handle.snapshot(), source) ?? { source, bindings: [] };
  const binding: ShapeBinding = { shapeId, pagePart: page.sourcePartPath, sourceId: shape.sourceId, shapeName: shape.name, key, status: 'ok', appliedRows };
  const kept = current.bindings.filter((entry) => entry.shapeId !== shapeId);
  kept.push(binding);
  writeBindingDoc(handle, handle.snapshot(), { source, bindings: kept });
  return { binding, applied, refused, unmappedColumns };
}

/** Re-apply every binding from an updated table; vanished rows keep their values and turn stale. */
export function refreshBindings(handle: DiagramHandle, snapshot: DiagramSnapshot, table: DataTable, source: DataBindingSource): RefreshReport {
  const stored = readBindingDoc(snapshot, source);
  if (!stored) throw new Error(`data table "${table.name}" has no bindings to refresh`);
  const report: RefreshReport = { updated: [], stale: [], removed: [], refused: [], unmappedColumns: [] };
  const kept: ShapeBinding[] = [];
  for (const binding of stored.bindings) {
    const target = findBoundShape(handle.snapshot(), binding);
    if (!target) {
      report.removed.push({ shapeId: binding.shapeId, key: binding.key, reason: 'shape is gone' });
      continue;
    }
    const record = table.rows.find((row) => row.key === binding.key);
    if (!record) {
      kept.push({ ...binding, status: 'stale' });
      report.stale.push({ shapeId: binding.shapeId, key: binding.key });
      continue;
    }
    const { applied, refused, unmappedColumns, appliedRows } = applyRowValues(handle, target.pageId, target.shape.id, target.shape, table, record);
    kept.push({ ...binding, shapeId: target.shape.id, status: 'ok', appliedRows });
    report.updated.push(...applied.map((column) => `${target.shape.id}:${column}`));
    report.refused.push(...refused.map((entry) => ({ shapeId: target.shape.id, ...entry })));
    for (const column of unmappedColumns) {
      if (!report.unmappedColumns.includes(column)) report.unmappedColumns.push(column);
    }
  }
  writeBindingDoc(handle, handle.snapshot(), { source, bindings: kept });
  return report;
}

/** Drop a shape's binding; its values stay untouched. */
export function unbindRow(handle: DiagramHandle, snapshot: DiagramSnapshot, source: DataBindingSource, shapeId: string): boolean {
  const stored = readBindingDoc(snapshot, source);
  if (!stored) return false;
  const kept = stored.bindings.filter((binding) => binding.shapeId !== shapeId);
  if (kept.length === stored.bindings.length) return false;
  writeBindingDoc(handle, handle.snapshot(), { source: stored.source, bindings: kept });
  return true;
}

/** Property rows of a shape that currently carry linked values, plus whether its link is stale. */
export function bindingMarksForShape(doc: DataBindingDoc | null, shapeId: string): { linked: string[]; stale: boolean } {
  const binding = doc?.bindings.find((entry) => entry.shapeId === shapeId);
  if (!binding) return { linked: [], stale: false };
  return { linked: binding.appliedRows, stale: binding.status === 'stale' };
}

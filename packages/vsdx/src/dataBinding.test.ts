import { beforeAll, describe, expect, test } from 'bun:test';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import {
  bindingMarksForShape, bindRow, bindingValueFormula, columnLetters, findBindingShape, initWasm, linkShapeName,
  openDiagram, parseBindingDoc, planColumnMapping, readBindingDoc, readGridFromWorkbook,
  refreshBindings, rekeyTable, shapeDataRows, tableFromCsv, tableFromGrid, unbindRow, writeBindingDoc,} from './index';
import type { DataBindingSource, DataTable, DiagramHandle, FormulaShapeDraft, ShapeBinding } from './index';

const root = resolve(import.meta.dir, '../../..');
let foundation: Uint8Array;

beforeAll(async () => {
  const [wasm, fixture] = await Promise.all([
    readFile(resolve(root, 'packages/vsdx/src/wasm/generated/vsdx_wasm_bg.wasm')),
    readFile(resolve(root, 'crates/vsdx-parse/tests/fixtures/foundation.vsdx')),
  ]);
  await initWasm(wasm);
  foundation = new Uint8Array(fixture);
});

const SOURCE: DataBindingSource = { kind: 'csv', name: 'devices.csv', sheet: null, keyColumn: 'Device' };

function deviceTable(): DataTable {
  return tableFromGrid([
    ['Device', 'Owner', 'Rack', 'Count'],
    ['SRV-01', 'Team Atlas', 'R7', '12'],
    ['SRV-02', 'Team Beacon', 'R8', '8'],
  ], 'Devices');
}

function propertyShapeCells(rows: Array<{ row: string; type?: string; value?: string; guarded?: boolean }>): FormulaShapeDraft['cells'] {
  const cells: FormulaShapeDraft['cells'] = [
    { locator: { cellName: 'PinX' }, name: 'PinX', formula: '1' },
    { locator: { cellName: 'PinY' }, name: 'PinY', formula: '1' },
    { locator: { cellName: 'Width' }, name: 'Width', formula: '1' },
    { locator: { cellName: 'Height' }, name: 'Height', formula: '1' },
  ];
  for (const { row, type, value, guarded } of rows) {
    cells.push({ locator: { section: 'Property', rowName: row, cellName: 'Value' }, name: 'Value', formula: guarded ? 'GUARD("locked")' : `"${value ?? ''}"` });
    if (type !== undefined) cells.push({ locator: { section: 'Property', rowName: row, cellName: 'Type' }, name: 'Type', formula: type });
  }
  return cells;
}

function openWithDeviceShape(clientId: number): { diagram: DiagramHandle; pageId: string; shapeId: string } {
  const diagram = openDiagram(foundation, { clientId });
  const pageId = diagram.snapshot().pages[0].id;
  const receipt = diagram.addShape(pageId, {
    name: 'Test Server',
    cells: propertyShapeCells([
      { row: 'Device', value: '' },
      { row: 'Owner', value: '' },
      { row: 'Rack', value: '' },
      { row: 'Count', type: '2', value: '0' },
    ]),
  });
  return { diagram, pageId, shapeId: receipt.shapeId };
}

describe('table import', () => {
  test('reads the first row as column names and keys the rest', () => {
    const table = deviceTable();
    expect(table.columns).toEqual(['Device', 'Owner', 'Rack', 'Count']);
    expect(table.keyColumn).toBe('Device');
    expect(table.rows.map((row) => row.key)).toEqual(['SRV-01', 'SRV-02']);
    expect(table.rows[0].values).toEqual({ Device: 'SRV-01', Owner: 'Team Atlas', Rack: 'R7', Count: '12' });
    expect(table.skippedRows).toBe(0);
  });

  test('trims cells, drops blank rows and keeps unkeyable rows out of the view', () => {
    const table = tableFromGrid([
      ['Device', 'Owner'],
      ['  SRV-01  ', ' Team Atlas '],
      ['', ''],
      ['SRV-01', 'duplicate'],
      ['', 'keyless'],
    ], 'Devices');
    expect(table.rows.length).toBe(1);
    expect(table.rows[0].values.Owner).toBe('Team Atlas');
    expect(table.skippedRows).toBe(2);
    expect(table.records.length).toBe(3);
  });

  test('dedupes columns and names blank headers', () => {
    const table = tableFromGrid([['A', 'A', ''], ['1', '2', '3']], 'T');
    expect(table.columns).toEqual(['A', 'A (2)', 'Column C']);
  });

  test('rejects empty grids and unknown key columns', () => {
    expect(() => tableFromGrid([], 'T')).toThrow('is empty');
    expect(() => tableFromGrid([['A']], 'T', 'Missing')).toThrow('key column');
  });

  test('parses quoted csv fields and semicolons', () => {
    const table = tableFromCsv('Device;Owner\nSRV-01;"Team, Atlas"\n"SRV-02";"Quoted ""name"""\n', 'Devices');
    expect(table.columns).toEqual(['Device', 'Owner']);
    expect(table.rows[0].values.Owner).toBe('Team, Atlas');
    expect(table.rows[1].values.Owner).toBe('Quoted "name"');
  });

  test('keeps newlines inside quoted csv fields', () => {
    const table = tableFromCsv('Device,Note\r\nSRV-01,"line one\nline two"\r\nSRV-02,plain\r\n', 'Devices');
    expect(table.rows[0].values.Note).toBe('line one\nline two');
    expect(table.rows[1].values.Note).toBe('plain');
  });

  test('re-keys a table on another column', () => {
    const rekeyed = rekeyTable(deviceTable(), 'Owner');
    expect(rekeyed.keyColumn).toBe('Owner');
    expect(rekeyed.rows.map((row) => row.key)).toEqual(['Team Atlas', 'Team Beacon']);
    expect(() => rekeyTable(deviceTable(), 'Missing')).toThrow('key column');
  });

  test('re-keying recovers rows the first column could not key', () => {
    const table = tableFromGrid([
      ['Device', 'Owner'],
      ['SRV-01', 'Team Atlas'],
      ['', 'Team Beacon'],
      ['SRV-01', 'Team Cartwheel'],
    ], 'Devices');
    expect(table.rows.map((row) => row.key)).toEqual(['SRV-01']);
    expect(table.skippedRows).toBe(2);
    const rekeyed = rekeyTable(table, 'Owner');
    expect(rekeyed.rows.map((row) => row.key)).toEqual(['Team Atlas', 'Team Beacon', 'Team Cartwheel']);
    expect(rekeyed.skippedRows).toBe(0);
    expect(rekeyTable(rekeyed, 'Device').rows.map((row) => row.key)).toEqual(['SRV-01']);
  });

  test('maps typed value formulas and keeps free text visible', () => {
    expect(bindingValueFormula('number', '12')).toBe('12');
    expect(bindingValueFormula('number', 'R7')).toBe('"R7"');
    expect(bindingValueFormula('string', 'R7')).toBe('"R7"');
    expect(bindingValueFormula('boolean', 'yes')).toBe('1');
  });

  test('maps column letters', () => {
    expect([columnLetters(0), columnLetters(25), columnLetters(26), columnLetters(51)]).toEqual(['A', 'Z', 'AA', 'AZ']);
  });

  test('trims the workbook grid to its used range', () => {
    const reader = { rangeCells: (_sheet: number, _range: string) => [[{ input: 'A' }, { input: '' }], [{ input: '1' }, { input: '' }], [{ input: '' }, { input: '' }]] };
    expect(readGridFromWorkbook(reader, 0)).toEqual([['A'], ['1']]);
  });

  test('sanitizes link shape names', () => {
    expect(linkShapeName({ ...SOURCE, name: '  Devices 2026  ' })).toBe('BO_DataLink Devices 2026');
    expect(linkShapeName({ ...SOURCE, name: '' })).toBe('BO_DataLink Table');
    expect(linkShapeName({ ...SOURCE, kind: 'xlsx', name: 'Devices', sheet: 'Q1' })).toBe('BO_DataLink Devices [Q1]');
  });
});

describe('column mapping', () => {
  test('matches row names before labels, case-insensitively', () => {
    const { diagram, shapeId } = openWithDeviceShape(31001);
    try {
      const shape = diagram.snapshot().pages[0].shapes.find((candidate) => candidate.id === shapeId)!;
      const mapping = planColumnMapping(shape, deviceTable());
      expect(mapping.mapped).toEqual([
        { column: 'Device', rowName: 'Device' },
        { column: 'Owner', rowName: 'Owner' },
        { column: 'Rack', rowName: 'Rack' },
        { column: 'Count', rowName: 'Count' },
      ]);
      expect(mapping.unmappedColumns).toEqual([]);
      const partial = planColumnMapping(shape, tableFromGrid([['Device', 'Missing']], 'T'));
      expect(partial.unmappedColumns).toEqual(['Device', 'Missing'].filter((column) => column === 'Missing'));
    } finally { diagram.dispose(); }
  });
});

describe('row binding', () => {
  test('lands columns in shape data through the mutation policy', () => {
    const { diagram, pageId, shapeId } = openWithDeviceShape(31002);
    try {
      const outcome = bindRow(diagram, diagram.snapshot(), deviceTable(), SOURCE, pageId, shapeId, 'SRV-01');
      expect(outcome.applied.sort()).toEqual(['Count', 'Device', 'Owner', 'Rack']);
      expect(outcome.refused).toEqual([]);
      expect(outcome.unmappedColumns).toEqual([]);
      const shape = diagram.snapshot().pages[0].shapes.find((candidate) => candidate.id === shapeId)!;
      const values = Object.fromEntries(shapeDataRows(shape).map((row) => [row.rowName, row.displayValue]));
      expect(values).toMatchObject({ Device: 'SRV-01', Owner: 'Team Atlas', Rack: 'R7', Count: '12' });
      const doc = readBindingDoc(diagram.snapshot(), SOURCE);
      expect(doc?.bindings.map((binding) => binding.key)).toEqual(['SRV-01']);
      expect(doc?.bindings[0].appliedRows.sort()).toEqual(['Count', 'Device', 'Owner', 'Rack']);
    } finally { diagram.dispose(); }
  });

  test('rejects unknown rows and missing shapes without writing', () => {
    const { diagram, pageId, shapeId } = openWithDeviceShape(31003);
    try {
      expect(() => bindRow(diagram, diagram.snapshot(), deviceTable(), SOURCE, pageId, shapeId, 'SRV-99')).toThrow('not in data table');
      expect(() => bindRow(diagram, diagram.snapshot(), deviceTable(), SOURCE, pageId, 'page:1:shape:999', 'SRV-01')).toThrow('was not found');
      expect(readBindingDoc(diagram.snapshot(), SOURCE)).toBeNull();
    } finally { diagram.dispose(); }
  });

  test('reports guarded cells as refusals and keeps the other columns', () => {
    const diagram = openDiagram(foundation, { clientId: 31004 });
    try {
      const pageId = diagram.snapshot().pages[0].id;
      const receipt = diagram.addShape(pageId, {
        name: 'Guarded Box',
        cells: propertyShapeCells([{ row: 'Device', value: '' }, { row: 'Owner', value: '', guarded: true }]),
      });
      const outcome = bindRow(diagram, diagram.snapshot(), deviceTable(), SOURCE, pageId, receipt.shapeId, 'SRV-01');
      expect(outcome.applied).toEqual(['Device']);
      expect(outcome.refused.length).toBe(1);
      expect(outcome.refused[0].column).toBe('Owner');
      expect(outcome.refused[0].reason).toContain('GUARD');
      expect(outcome.unmappedColumns).toEqual(['Rack', 'Count']);
      const shape = diagram.snapshot().pages[0].shapes.find((candidate) => candidate.id === receipt.shapeId)!;
      const values = Object.fromEntries(shapeDataRows(shape).map((row) => [row.rowName, row.displayValue]));
      expect(values.Device).toBe('SRV-01');
      const guarded = shape.cells.find((cell) => cell.locator.section === 'Property' && cell.name === 'Value' && cell.locator.row !== null && 'name' in cell.locator.row && cell.locator.row.name === 'Owner');
      expect(guarded?.formula).toBe('GUARD("locked")');
    } finally { diagram.dispose(); }
  });

  test('keeps non-numeric text visible in number rows', () => {
    const { diagram, pageId, shapeId } = openWithDeviceShape(31011);
    try {
      const table = tableFromGrid([['Device', 'Count'], ['SRV-01', 'many']], 'Devices');
      const outcome = bindRow(diagram, diagram.snapshot(), table, SOURCE, pageId, shapeId, 'SRV-01');
      expect(outcome.applied.sort()).toEqual(['Count', 'Device']);
      const shape = diagram.snapshot().pages[0].shapes.find((candidate) => candidate.id === shapeId)!;
      const count = shapeDataRows(shape).find((row) => row.rowName === 'Count')!;
      expect(count.formula).toBe('"many"');
      expect(count.displayValue).toBe('many');
    } finally { diagram.dispose(); }
  });

  test('unbind keeps the values on the shape', () => {
    const { diagram, pageId, shapeId } = openWithDeviceShape(31005);
    try {
      bindRow(diagram, diagram.snapshot(), deviceTable(), SOURCE, pageId, shapeId, 'SRV-01');
      expect(unbindRow(diagram, diagram.snapshot(), SOURCE, shapeId)).toBe(true);
      expect(unbindRow(diagram, diagram.snapshot(), SOURCE, shapeId)).toBe(false);
      expect(readBindingDoc(diagram.snapshot(), SOURCE)?.bindings).toEqual([]);
      const shape = diagram.snapshot().pages[0].shapes.find((candidate) => candidate.id === shapeId)!;
      expect(shapeDataRows(shape).find((row) => row.rowName === 'Owner')?.displayValue).toBe('Team Atlas');
    } finally { diagram.dispose(); }
  });
});

describe('refresh', () => {
  test('re-importing an updated table updates bound shapes', () => {
    const { diagram, pageId, shapeId } = openWithDeviceShape(31006);
    try {
      bindRow(diagram, diagram.snapshot(), deviceTable(), SOURCE, pageId, shapeId, 'SRV-01');
      const updated = tableFromGrid([
        ['Device', 'Owner', 'Rack', 'Count'],
        ['SRV-01', 'Team Cartwheel', 'R9', '14'],
        ['SRV-02', 'Team Beacon', 'R8', '8'],
      ], 'Devices');
      const report = refreshBindings(diagram, diagram.snapshot(), updated, SOURCE);
      expect(report.stale).toEqual([]);
      expect(report.removed).toEqual([]);
      expect(report.refused).toEqual([]);
      const shape = diagram.snapshot().pages[0].shapes.find((candidate) => candidate.id === shapeId)!;
      const values = Object.fromEntries(shapeDataRows(shape).map((row) => [row.rowName, row.displayValue]));
      expect(values).toMatchObject({ Owner: 'Team Cartwheel', Rack: 'R9', Count: '14' });
    } finally { diagram.dispose(); }
  });

  test('a vanished row keeps its values and turns the binding stale', () => {
    const { diagram, pageId, shapeId } = openWithDeviceShape(31007);
    try {
      bindRow(diagram, diagram.snapshot(), deviceTable(), SOURCE, pageId, shapeId, 'SRV-01');
      const pruned = tableFromGrid([['Device', 'Owner', 'Rack', 'Count'], ['SRV-02', 'Team Beacon', 'R8', '8']], 'Devices');
      const report = refreshBindings(diagram, diagram.snapshot(), pruned, SOURCE);
      expect(report.stale).toEqual([{ shapeId, key: 'SRV-01' }]);
      expect(report.updated).toEqual([]);
      const shape = diagram.snapshot().pages[0].shapes.find((candidate) => candidate.id === shapeId)!;
      const values = Object.fromEntries(shapeDataRows(shape).map((row) => [row.rowName, row.displayValue]));
      expect(values).toMatchObject({ Device: 'SRV-01', Owner: 'Team Atlas', Rack: 'R7', Count: '12' });
      expect(readBindingDoc(diagram.snapshot(), SOURCE)?.bindings[0].status).toBe('stale');
      expect(bindingMarksForShape(readBindingDoc(diagram.snapshot(), SOURCE), shapeId)).toEqual({ linked: ['Device', 'Owner', 'Rack', 'Count'], stale: true });
    } finally { diagram.dispose(); }
  });

  test('a deleted shape is reported and dropped without touching survivors', () => {
    const diagram = openDiagram(foundation, { clientId: 31008 });
    try {
      const pageId = diagram.snapshot().pages[0].id;
      const first = diagram.addShape(pageId, { name: 'First', cells: propertyShapeCells([{ row: 'Device', value: '' }]) });
      const second = diagram.addShape(pageId, { name: 'Second', cells: propertyShapeCells([{ row: 'Device', value: '' }]) });
      const table = tableFromGrid([['Device'], ['SRV-01'], ['SRV-02']], 'Devices');
      bindRow(diagram, diagram.snapshot(), table, SOURCE, pageId, first.shapeId, 'SRV-01');
      bindRow(diagram, diagram.snapshot(), table, SOURCE, pageId, second.shapeId, 'SRV-02');
      diagram.deleteShape(pageId, first.shapeId);
      const report = refreshBindings(diagram, diagram.snapshot(), table, SOURCE);
      expect(report.removed).toEqual([{ shapeId: first.shapeId, key: 'SRV-01', reason: 'shape is gone' }]);
      expect(readBindingDoc(diagram.snapshot(), SOURCE)?.bindings.map((binding) => binding.key)).toEqual(['SRV-02']);
    } finally { diagram.dispose(); }
  });
});

describe('link document identity', () => {
  test('tables whose names share the truncated label keep separate documents', () => {
    const diagram = openDiagram(foundation, { clientId: 31012 });
    try {
      const pageId = diagram.snapshot().pages[0].id;
      const first = diagram.addShape(pageId, { name: 'First', cells: propertyShapeCells([{ row: 'Device', value: '' }]) });
      const second = diagram.addShape(pageId, { name: 'Second', cells: propertyShapeCells([{ row: 'Device', value: '' }]) });
      const table = tableFromGrid([['Device'], ['SRV-01'], ['SRV-02']], 'Devices');
      const shared = 'a'.repeat(60);
      const one: DataBindingSource = { kind: 'csv', name: `${shared}-one.csv`, sheet: null, keyColumn: 'Device' };
      const two: DataBindingSource = { ...one, name: `${shared}-two.csv` };
      expect(linkShapeName(one)).toBe(linkShapeName(two));
      bindRow(diagram, diagram.snapshot(), table, one, pageId, first.shapeId, 'SRV-01');
      bindRow(diagram, diagram.snapshot(), table, two, pageId, second.shapeId, 'SRV-02');
      expect(readBindingDoc(diagram.snapshot(), one)?.bindings.map((binding) => binding.key)).toEqual(['SRV-01']);
      expect(readBindingDoc(diagram.snapshot(), two)?.bindings.map((binding) => binding.key)).toEqual(['SRV-02']);
    } finally { diagram.dispose(); }
  });

  test('two sheets of one workbook keep separate documents', () => {
    const diagram = openDiagram(foundation, { clientId: 31013 });
    try {
      const pageId = diagram.snapshot().pages[0].id;
      const first = diagram.addShape(pageId, { name: 'First', cells: propertyShapeCells([{ row: 'Device', value: '' }]) });
      const second = diagram.addShape(pageId, { name: 'Second', cells: propertyShapeCells([{ row: 'Device', value: '' }]) });
      const table = tableFromGrid([['Device'], ['SRV-01'], ['SRV-02']], 'devices.xlsx');
      const q1: DataBindingSource = { kind: 'xlsx', name: 'devices.xlsx', sheet: 'Q1', keyColumn: 'Device' };
      const q2: DataBindingSource = { ...q1, sheet: 'Q2' };
      bindRow(diagram, diagram.snapshot(), table, q1, pageId, first.shapeId, 'SRV-01');
      bindRow(diagram, diagram.snapshot(), table, q2, pageId, second.shapeId, 'SRV-02');
      expect(readBindingDoc(diagram.snapshot(), q1)?.bindings.map((binding) => binding.shapeId)).toEqual([first.shapeId]);
      expect(readBindingDoc(diagram.snapshot(), q2)?.bindings.map((binding) => binding.shapeId)).toEqual([second.shapeId]);
      expect(refreshBindings(diagram, diagram.snapshot(), table, q1).updated.length).toBe(1);
      expect(readBindingDoc(diagram.snapshot(), q2)?.bindings.map((binding) => binding.shapeId)).toEqual([second.shapeId]);
    } finally { diagram.dispose(); }
  });

  test('a replacement over the link limit leaves the stored document intact', () => {
    const { diagram, pageId, shapeId } = openWithDeviceShape(31014);
    try {
      bindRow(diagram, diagram.snapshot(), deviceTable(), SOURCE, pageId, shapeId, 'SRV-01');
      const oversized: ShapeBinding[] = Array.from({ length: 2001 }, (_, index) => ({
        shapeId: `absent:${index}`, pagePart: '', sourceId: 0, shapeName: null, key: `K${index}`, status: 'ok', appliedRows: [],
      }));
      expect(() => writeBindingDoc(diagram, diagram.snapshot(), { source: SOURCE, bindings: oversized })).toThrow('exceeds 2000 links');
      expect(readBindingDoc(diagram.snapshot(), SOURCE)?.bindings.map((binding) => binding.key)).toEqual(['SRV-01']);
    } finally { diagram.dispose(); }
  });
});

describe('round-trip', () => {
  test('bindings and values survive save and reopen', () => {
    const { diagram, pageId, shapeId } = openWithDeviceShape(31009);
    let saved: Uint8Array;
    try {
      bindRow(diagram, diagram.snapshot(), deviceTable(), SOURCE, pageId, shapeId, 'SRV-02');
      saved = diagram.save();
    } finally { diagram.dispose(); }
    const reopened = openDiagram(saved, { clientId: 31010 });
    try {
      const snapshot = reopened.snapshot();
      const doc = readBindingDoc(snapshot, SOURCE);
      expect(doc?.source).toEqual(SOURCE);
      expect(doc?.bindings.length).toBe(1);
      expect(doc?.bindings[0].key).toBe('SRV-02');
      const shape = snapshot.pages[0].shapes.find((candidate) => candidate.name === 'Test Server');
      expect(shape).toBeDefined();
      const values = Object.fromEntries(shapeDataRows(shape!).map((row) => [row.rowName, row.displayValue]));
      expect(values).toMatchObject({ Device: 'SRV-02', Owner: 'Team Beacon', Rack: 'R8', Count: '8' });
      expect(findBindingShape(snapshot, SOURCE)).not.toBeNull();
      const reparsed = parseBindingDoc(findBindingShape(snapshot, SOURCE));
      expect(reparsed?.bindings[0].appliedRows.sort()).toEqual(['Count', 'Device', 'Owner', 'Rack']);
      const refreshed = refreshBindings(reopened, reopened.snapshot(), deviceTable(), SOURCE);
      expect(refreshed.stale).toEqual([]);
      expect(refreshed.updated.length).toBe(4);
      expect(readBindingDoc(reopened.snapshot(), SOURCE)?.bindings[0].shapeId).toBe(shape!.id);
    } finally { reopened.dispose(); }
  });
});

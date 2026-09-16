import { expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { initWasm as initXlsxWasm, openWorkbook } from '@betteroffice/xlsx';
import { loadXlsxTable } from './vsdxDataTable';

const root = resolve(import.meta.dir, '../../..');

async function syntheticWorkbook(): Promise<File> {
  await initXlsxWasm();
  const sample = new Uint8Array(readFileSync(resolve(root, 'packages/xlsx/test-fixtures/sample.xlsx')));
  const workbook = openWorkbook(sample);
  try {
    const cells: Array<[number, number, string]> = [
      [0, 0, 'Device'], [0, 1, 'Owner'], [0, 2, 'Rack'],
      [1, 0, 'SRV-01'], [1, 1, 'Team Atlas'], [1, 2, 'R7'],
      [2, 0, 'SRV-02'], [2, 1, 'Team Beacon'], [2, 2, 'R8'],
    ];
    for (const [row, col, input] of cells) workbook.editCell(0, row, col, input);
    const bytes = workbook.save();
    return new File([bytes as unknown as BlobPart], 'devices.xlsx', { type: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet' });
  } finally {
    workbook.dispose();
  }
}

test('loads the active sheet of an xlsx file through the suite engine', async () => {
  const file = await syntheticWorkbook();
  const { table, source } = await loadXlsxTable(file);
  expect(table.name).toBe('devices.xlsx');
  expect(table.columns.slice(0, 3)).toEqual(['Device', 'Owner', 'Rack']);
  expect(table.keyColumn).toBe('Device');
  expect(table.rows.slice(0, 2).map((row) => row.key)).toEqual(['SRV-01', 'SRV-02']);
  expect(table.rows[0].values.Owner).toBe('Team Atlas');
  expect(source).toMatchObject({ kind: 'xlsx', name: 'devices.xlsx', sheet: 'Budget', keyColumn: 'Device' });
});

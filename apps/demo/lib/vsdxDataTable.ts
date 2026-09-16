import { tableFromWorkbook } from "@betteroffice/vsdx";
import type { DataBindingSource, DataTable } from "@betteroffice/vsdx";
import { initWasm as initXlsxWasm, openWorkbook as openXlsxWorkbook } from "@betteroffice/xlsx";

let xlsxReady: Promise<void> | null = null;

function ensureXlsxWasm(): Promise<void> {
  if (!xlsxReady) xlsxReady = initXlsxWasm();
  return xlsxReady;
}

/** Read the active sheet of an .xlsx file into a data table via the suite xlsx engine. */
export async function loadXlsxTable(file: File): Promise<{ table: DataTable; source: DataBindingSource }> {
  await ensureXlsxWasm();
  const bytes = new Uint8Array(await file.arrayBuffer());
  const workbook = openXlsxWorkbook(bytes);
  try {
    const info = workbook.sheetInfo();
    const sheetName = info.sheetNames[info.activeSheet] ?? info.sheetNames[0] ?? "Sheet1";
    const table = tableFromWorkbook(
      { rangeCells: (sheet, range) => workbook.rangeCells(sheet, range) },
      info.activeSheet,
      file.name || "table.xlsx",
    );
    return { table, source: { kind: "xlsx", name: file.name || "table.xlsx", sheet: sheetName, keyColumn: table.keyColumn } };
  } finally {
    workbook.dispose();
  }
}

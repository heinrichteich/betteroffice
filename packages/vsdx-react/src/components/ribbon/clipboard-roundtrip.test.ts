import { beforeAll, expect, test } from 'bun:test';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { initWasm, openDiagram } from '@betteroffice/vsdx';
import { buildClipboardEntry, draftForPaste } from './clipboard';
import { PASTE_OFFSET } from './clipboard';

const root = resolve(import.meta.dir, '../../../../..');

function cellKey(cell: { locator: { cellName: string; section: unknown; sectionIndex?: unknown; row: unknown } }): string {
  return JSON.stringify([cell.locator.cellName, cell.locator.section, cell.locator.sectionIndex ?? null, cell.locator.row]);
}

beforeAll(async () => {
  const wasm = await readFile(resolve(root, 'packages/vsdx/src/wasm/generated/vsdx_wasm_bg.wasm'));
  await initWasm(wasm);
});

test('paste round-trips cells and text through save and reopen as one undo step', async () => {
  const bytes = await readFile(resolve(root, 'apps/demo/public/betteroffice-demo.vsdx'));
  const handle = openDiagram(bytes, { clientId: 7101 });
  try {
    const pageId = handle.snapshot().pages[0].id;
    const shapeId = handle.snapshot().pages[0].shapes[0].id;
    handle.setCellFormula(pageId, shapeId, { cellName: 'FillForegnd' }, 'RGB(11,22,33)');
    handle.setShapeText(pageId, shapeId, 'clipboard hello');
    const placement = handle.snapshot().pages[0].shapes.find((shape) => shape.id === shapeId)!;
    const entry = buildClipboardEntry(pageId, placement, handle.shapeText(pageId, shapeId));
    const before = handle.snapshot().pages[0].shapes.length;
    const receipt = handle.addShapeWithText(pageId, draftForPaste(entry, PASTE_OFFSET.x, PASTE_OFFSET.y), entry.text);
    expect(handle.snapshot().pages[0].shapes.length).toBe(before + 1);
    const pasted = handle.snapshot().pages[0].shapes.find((shape) => shape.id === receipt.shapeId)!;
    const copies = new Map(pasted.cells.map((cell) => [cellKey(cell), cell]));
    for (const cell of placement.cells) {
      if (cell.locator.cellName === 'PinX' || cell.locator.cellName === 'PinY') continue;
      const copy = copies.get(cellKey(cell));
      expect(copy?.formula).toBe(cell.formula);
      expect(copy?.value ?? copy?.formula).toBe(cell.value ?? cell.formula);
    }
    expect(handle.shapeText(pageId, receipt.shapeId)).toBe('clipboard hello');
    expect(handle.canUndo()).toBe(true);
    handle.undo();
    expect(handle.snapshot().pages[0].shapes.length).toBe(before);
    expect(handle.canRedo()).toBe(true);
    handle.redo();
    expect(handle.snapshot().pages[0].shapes.length).toBe(before + 1);
    const saved = handle.save();
    const reopened = openDiagram(saved, { clientId: 7102 });
    try {
      const page = reopened.snapshot().pages[0];
      const survived = page.shapes.find((shape) => {
        try { return reopened.shapeText(page.id, shape.id) === 'clipboard hello'; }
        catch { return false; }
      });
      expect(survived).toBeDefined();
      const fill = survived!.cells.find((cell) => cell.locator.cellName === 'FillForegnd');
      expect(fill?.formula).toBe('RGB(11,22,33)');
    } finally {
      reopened.dispose();
    }
  } finally {
    handle.dispose();
  }
});

test('paste preserves formula-less cells parsed from a real file', async () => {
  const bytes = await readFile(resolve(root, 'apps/demo/public/betteroffice-demo.vsdx'));
  const handle = openDiagram(bytes, { clientId: 7104 });
  try {
    const pageId = handle.snapshot().pages[0].id;
    const placement = handle.snapshot().pages[0].shapes[0];
    expect(placement.cells.some((cell) => cell.formula == null && cell.value != null)).toBe(true);
    const entry = buildClipboardEntry(pageId, placement, handle.shapeText(pageId, placement.id));
    const receipt = handle.addShapeWithText(pageId, draftForPaste(entry, PASTE_OFFSET.x, PASTE_OFFSET.y), entry.text);
    const pasted = handle.snapshot().pages[0].shapes.find((shape) => shape.id === receipt.shapeId)!;
    const copies = new Map(pasted.cells.map((cell) => [cellKey(cell), cell]));
    for (const cell of placement.cells) {
      if (cell.locator.cellName === 'PinX' || cell.locator.cellName === 'PinY') continue;
      const copy = copies.get(cellKey(cell));
      expect(copy?.formula).toBe(cell.formula);
      expect(copy?.value ?? copy?.formula).toBe(cell.value ?? cell.formula);
    }
    const width = pasted.cells.find((cell) => cell.locator.cellName === 'Width' && cell.locator.section == null);
    expect(width?.value ?? width?.formula).toBe(
      placement.cells.find((cell) => cell.locator.cellName === 'Width' && cell.locator.section == null)?.value,
    );
    handle.undo();
    expect(handle.snapshot().pages[0].shapes.some((shape) => shape.id === receipt.shapeId)).toBe(false);
    const saved = handle.save();
    const reopened = openDiagram(saved, { clientId: 7105 });
    try {
      expect(reopened.snapshot().pages[0].shapes.some((shape) => shape.id === placement.id)).toBe(true);
    } finally {
      reopened.dispose();
    }
  } finally {
    handle.dispose();
  }
});

test('cut removes the shape but restores it with a single undo', async () => {
  const bytes = await readFile(resolve(root, 'crates/vsdx-parse/tests/fixtures/foundation.vsdx'));
  const handle = openDiagram(bytes, { clientId: 7103 });
  try {
    const pageId = handle.snapshot().pages[0].id;
    const shapeId = handle.snapshot().pages[0].shapes[0].id;
    const before = handle.snapshot().pages[0].shapes.length;
    handle.deleteShape(pageId, shapeId);
    expect(handle.snapshot().pages[0].shapes.length).toBe(before - 1);
    handle.undo();
    expect(handle.snapshot().pages[0].shapes.length).toBe(before);
    expect(handle.snapshot().pages[0].shapes.some((shape) => shape.id === shapeId)).toBe(true);
  } finally {
    handle.dispose();
  }
});

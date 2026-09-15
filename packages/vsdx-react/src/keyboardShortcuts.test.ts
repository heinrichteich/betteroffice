import { expect, mock, test } from 'bun:test';
import type { DiagramHandle, DiagramSnapshot } from '@betteroffice/vsdx';
import { canvasKeyboardIntent, isOwnedBrowserShortcut } from './interactions';
import { createRibbonCommands } from './components/ribbon/commands';

test('Ctrl+R maps to rotate right and is owned so the browser never reloads', () => {
  expect(canvasKeyboardIntent({ key: 'r', ctrlKey: true }, 1)).toEqual({ kind: 'rotateRight' });
  expect(canvasKeyboardIntent({ key: 'R', ctrlKey: true }, 1)).toEqual({ kind: 'rotateRight' });
  expect(canvasKeyboardIntent({ key: 'r', metaKey: true }, 1)).toEqual({ kind: 'rotateRight' });
  expect(canvasKeyboardIntent({ key: 'R', ctrlKey: true, shiftKey: true }, 1)).toEqual({ kind: 'rotateRight' });
  expect(isOwnedBrowserShortcut({ key: 'r', ctrlKey: true })).toBe(true);
  expect(isOwnedBrowserShortcut({ key: 'R', metaKey: true })).toBe(true);
});

test('Ctrl+L and Ctrl+S map to rotate left and save', () => {
  expect(canvasKeyboardIntent({ key: 'l', ctrlKey: true }, 1)).toEqual({ kind: 'rotateLeft' });
  expect(canvasKeyboardIntent({ key: 'L', metaKey: true }, 1)).toEqual({ kind: 'rotateLeft' });
  expect(canvasKeyboardIntent({ key: 's', ctrlKey: true }, 1)).toEqual({ kind: 'save' });
  expect(canvasKeyboardIntent({ key: 'S', metaKey: true }, 1)).toEqual({ kind: 'save' });
  expect(isOwnedBrowserShortcut({ key: 'l', ctrlKey: true })).toBe(true);
  expect(isOwnedBrowserShortcut({ key: 's', ctrlKey: true })).toBe(true);
});

test('unowned browser shortcuts stay with the browser', () => {
  for (const key of ['f', 'p', 'd', 'o']) {
    expect(canvasKeyboardIntent({ key, ctrlKey: true }, 1)).toBeNull();
    expect(canvasKeyboardIntent({ key, metaKey: true }, 1)).toBeNull();
    expect(isOwnedBrowserShortcut({ key, ctrlKey: true })).toBe(false);
  }
  expect(canvasKeyboardIntent({ key: 'r', ctrlKey: true, altKey: true }, 1)).toBeNull();
  expect(isOwnedBrowserShortcut({ key: 'r', ctrlKey: true, altKey: true })).toBe(false);
});

test('forbidden tab and window shortcuts are never owned', () => {
  for (const key of ['w', 'n', 't']) {
    expect(canvasKeyboardIntent({ key, ctrlKey: true }, 1)).toBeNull();
    expect(canvasKeyboardIntent({ key, ctrlKey: true, shiftKey: true }, 1)).toBeNull();
    expect(isOwnedBrowserShortcut({ key, ctrlKey: true })).toBe(false);
    expect(isOwnedBrowserShortcut({ key, ctrlKey: true, shiftKey: true })).toBe(false);
  }
  expect(canvasKeyboardIntent({ key: 'F11' }, 1)).toBeNull();
});

test('owned shortcuts produce no intent from editable targets but still prevent loss', () => {
  const input = { tagName: 'INPUT' };
  expect(canvasKeyboardIntent({ key: 'r', ctrlKey: true, target: input }, 1)).toBeNull();
  expect(canvasKeyboardIntent({ key: 's', ctrlKey: true, target: input }, 1)).toBeNull();
  expect(isOwnedBrowserShortcut({ key: 'r', ctrlKey: true, target: input })).toBe(true);
  expect(isOwnedBrowserShortcut({ key: 's', ctrlKey: true, target: input })).toBe(true);
});

function rotateState(cells: Record<string, string>): DiagramSnapshot {
  return {
    pages: [{
      id: 'page', sourcePartPath: 'page', name: 'Page',
      shapes: [{ id: 'two', sourceId: 1, name: 'two', children: [], cells: Object.entries(cells).map(([name, value]) => ({ locator: { sheet: { page: 1 }, shapeId: 1, section: null, row: null, cellName: name }, name, formula: value, value })) }],
    }],
  };
}

function rotateHandle(state: DiagramSnapshot) {
  const setCellFormula = mock((pageId: string, shapeId: string, locator: { cellName: string }, formula: string) => {
    const target = state.pages.find((page) => page.id === pageId)?.shapes.find((shape) => shape.id === shapeId);
    const cell = target?.cells.find((item) => item.locator.cellName === locator.cellName);
    if (cell) { cell.formula = formula; cell.value = formula; }
    return {};
  });
  const value = { snapshot: () => state, canUndo: () => false, canRedo: () => false, undo: mock(() => ({})), redo: mock(() => ({})), deleteShape: mock(() => ({})), setCellFormula, reorderShape: mock(() => ({})), addShape: mock(() => ({})), save: mock(() => new Uint8Array()) };
  return value as unknown as DiagramHandle & typeof value;
}

test('rotate intent drives the guarded rotate command', () => {
  const intent = canvasKeyboardIntent({ key: 'r', ctrlKey: true }, 1);
  expect(intent).toEqual({ kind: 'rotateRight' });
  const selection = { pageId: 'page', shapeId: 'two', hit: { kind: 'shape' as const, shapeId: 'two' } };
  const state = rotateState({ Angle: '0' });
  const diagram = rotateHandle(state);
  const commands = createRibbonCommands(diagram, selection, 'page', () => {}, () => {}, () => {});
  expect(commands.rotateRight.enabled).toBe(true);
  if (intent?.kind === 'rotateRight') commands.rotateRight.run();
  expect(diagram.setCellFormula).toHaveBeenCalledWith('page', 'two', { cellName: 'Angle' }, String(Math.PI / 2));
  const guarded = rotateState({ Angle: 'GUARD(0)' });
  const guardedDiagram = rotateHandle(guarded);
  const guardedCommands = createRibbonCommands(guardedDiagram, selection, 'page', () => {}, () => {}, () => {});
  expect(guardedCommands.rotateRight.enabled).toBe(false);
  expect(guardedCommands.rotateLeft.enabled).toBe(false);
});

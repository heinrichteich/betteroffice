import { expect, test } from 'bun:test';
import type { PageDisplayList } from '@betteroffice/vsdx';
import { collectDiagnostics } from './VsdxEditor';

const frame: PageDisplayList = {
  contractVersion: 4,
  width: 1,
  height: 1,
  paintTransform: { a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 },
  primitives: [{ kind: 'textBox', id: 'text', zOrder: 0, x: 0, y: 0, width: 1, height: 1, paragraphs: [{ runs: [{ text: 'x', family: 'Arial', sizeIn: 12, bold: false, italic: false, underline: false, smallCaps: false, superscript: false, subscript: false, letterSpacing: 0, color: '#000', diagnostics: [{ category: 'integrity', code: 'missing-media', detail: '' }, { category: 'fidelity', code: 'font-substituted', detail: '' }] }]}], lines: [] }],
};

test('collects structured diagnostics without matching their text', () => {
  expect(collectDiagnostics(frame)).toEqual([
    { category: 'integrity', code: 'missing-media', detail: '' },
    { category: 'fidelity', code: 'font-substituted', detail: '' },
  ]);
});

test('collects defaulted paint diagnostics and tolerates shapes without them', () => {
  const shapes: PageDisplayList = {
    ...frame,
    primitives: [
      { kind: 'shape', id: 'resolved', zOrder: 0, path: [] },
      { kind: 'group', id: 'group', zOrder: 1, primitives: [{ kind: 'shape', id: 'defaulted', zOrder: 2, path: [], diagnostics: [{ category: 'fidelity', code: 'unresolvable-fill-colour', detail: 'unresolvable fill colour: missing colour cell FillForegnd' }] }] },
    ],
  };
  expect(collectDiagnostics(shapes)).toEqual([
    { category: 'fidelity', code: 'unresolvable-fill-colour', detail: 'unresolvable fill colour: missing colour cell FillForegnd' },
  ]);
});

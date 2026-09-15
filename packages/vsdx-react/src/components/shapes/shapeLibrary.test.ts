import { expect, test } from 'bun:test';
import { arrowShapes, arrowVertices, calloutShapes, calloutVertices, polygonVertices, previewPathForVertices, shapeStencils, standardShapes, stencilCatalogue } from './shapeLibrary';

function geometry(shapeId: string) {
  return [...standardShapes, ...arrowShapes, ...calloutShapes].find((shape) => shape.id === shapeId)!.draft(2, 3, 4, 5).cells.filter((cell) => !['Angle', 'FlipX', 'FlipY', 'FillPattern', 'FillForegnd', 'LinePattern', 'LineColor', 'LineWeight'].includes(cell.locator.cellName));
}

test('produces finite, complete formula-only drafts', () => {
  for (const shape of [...standardShapes, ...arrowShapes, ...calloutShapes]) {
    for (const cell of shape.draft(Number.NaN, Number.POSITIVE_INFINITY, Number.NaN, Number.NEGATIVE_INFINITY).cells) {
      expect(cell.formula).toBeTruthy();
      expect(cell.formula).not.toMatch(/(?:nan|infinity)/i);
    }
  }
});

test('encodes the rectangle geometry cell by cell', () => {
  expect(geometry('rectangle')).toEqual([
    { locator: { cellName: 'PinX' }, name: 'PinX', formula: '2' },
    { locator: { cellName: 'PinY' }, name: 'PinY', formula: '3' },
    { locator: { cellName: 'Width' }, name: 'Width', formula: '4' },
    { locator: { cellName: 'Height' }, name: 'Height', formula: '5' },
    { locator: { cellName: 'LocPinX' }, name: 'LocPinX', formula: 'Width*0.5' },
    { locator: { cellName: 'LocPinY' }, name: 'LocPinY', formula: 'Height*0.5' },
    { locator: { section: 'Geometry', rowIndex: 0, rowType: 'MoveTo', cellName: 'X' }, name: 'X', formula: 'Width*0' },
    { locator: { section: 'Geometry', rowIndex: 0, rowType: 'MoveTo', cellName: 'Y' }, name: 'Y', formula: 'Height*0' },
    { locator: { section: 'Geometry', rowIndex: 1, rowType: 'LineTo', cellName: 'X' }, name: 'X', formula: 'Width*1' },
    { locator: { section: 'Geometry', rowIndex: 1, rowType: 'LineTo', cellName: 'Y' }, name: 'Y', formula: 'Height*0' },
    { locator: { section: 'Geometry', rowIndex: 2, rowType: 'LineTo', cellName: 'X' }, name: 'X', formula: 'Width*1' },
    { locator: { section: 'Geometry', rowIndex: 2, rowType: 'LineTo', cellName: 'Y' }, name: 'Y', formula: 'Height*1' },
    { locator: { section: 'Geometry', rowIndex: 3, rowType: 'LineTo', cellName: 'X' }, name: 'X', formula: 'Width*0' },
    { locator: { section: 'Geometry', rowIndex: 3, rowType: 'LineTo', cellName: 'Y' }, name: 'Y', formula: 'Height*1' },
    { locator: { section: 'Geometry', rowIndex: 4, rowType: 'Close', cellName: 'NoShow' }, name: 'NoShow', formula: '0' },
  ]);
});

test('encodes the computed hexagon cell by cell', () => {
  const expected = polygonVertices.hexagon.flatMap(([x, y], index) => [
    { locator: { section: 'Geometry', rowIndex: index, rowType: index === 0 ? 'MoveTo' : 'LineTo', cellName: 'X' }, name: 'X', formula: `Width*${x}` },
    { locator: { section: 'Geometry', rowIndex: index, rowType: index === 0 ? 'MoveTo' : 'LineTo', cellName: 'Y' }, name: 'Y', formula: `Height*${y}` },
  ]);
  expect(geometry('hexagon').slice(6)).toEqual([
    ...expected,
    { locator: { section: 'Geometry', rowIndex: 6, rowType: 'Close', cellName: 'NoShow' }, name: 'NoShow', formula: '0' },
  ]);
});

test('constrains a square to equal dimensions', () => {
  const cells = standardShapes.find((shape) => shape.id === 'square')!.draft(2, 3, 4, 9).cells;
  expect(cells.find((cell) => cell.name === 'Width')?.formula).toBe('4');
  expect(cells.find((cell) => cell.name === 'Height')?.formula).toBe('Width');
});

test('derives every polygon preview and geometry from shared vertices', () => {
  for (const [id, vertices] of Object.entries(polygonVertices)) {
    const shape = standardShapes.find((candidate) => candidate.id === id)!;
    const cells = shape.draft(0, 0, 1, 1).cells.filter((cell) => cell.name === 'X' || cell.name === 'Y').slice(0, vertices.length * 2);
    expect(shape.preview.startsWith(previewPathForVertices(vertices))).toBe(true);
    expect(cells.map((cell) => cell.formula)).toEqual(vertices.flatMap(([x, y]) => [`Width*${x}`, `Height*${y}`]));
  }
});

test('derives every arrow polygon preview and geometry from shared vertices', () => {
  for (const [id, vertices] of Object.entries(arrowVertices)) {
    const shape = arrowShapes.find((candidate) => candidate.id === id)!;
    const cells = shape.draft(0, 0, 1, 1).cells.filter((cell) => cell.name === 'X' || cell.name === 'Y').slice(0, vertices.length * 2);
    expect(shape.preview.startsWith(previewPathForVertices(vertices))).toBe(true);
    expect(cells.map((cell) => cell.formula)).toEqual(vertices.flatMap(([x, y]) => [`Width*${x}`, `Height*${y}`]));
  }
});

test('exposes three stencils covering every shape', () => {
  expect(shapeStencils.map((stencil) => stencil.id)).toEqual(['standard', 'arrows', 'callouts']);
  expect(shapeStencils[0].shapes).toEqual(standardShapes);
  expect(shapeStencils[1].shapes).toEqual(arrowShapes);
  expect(shapeStencils[2].shapes).toEqual(calloutShapes);
  expect(arrowShapes).toHaveLength(37);
  expect(calloutShapes).toHaveLength(6);
});

test('lists only stencils that ship shapes in the browser catalogue', () => {
  expect(stencilCatalogue.length).toBeGreaterThan(0);
  for (const category of stencilCatalogue) {
    expect(category.stencilIds.length).toBeGreaterThan(0);
    for (const id of category.stencilIds) {
      const stencil = shapeStencils.find((candidate) => candidate.id === id);
      expect(stencil).toBeDefined();
      expect(stencil!.shapes.length).toBeGreaterThan(0);
    }
  }
});

test('encodes the rectangular callout cell by cell', () => {
  expect(geometry('calloutBottom')).toEqual([
    { locator: { cellName: 'PinX' }, name: 'PinX', formula: '2' },
    { locator: { cellName: 'PinY' }, name: 'PinY', formula: '3' },
    { locator: { cellName: 'Width' }, name: 'Width', formula: '4' },
    { locator: { cellName: 'Height' }, name: 'Height', formula: '5' },
    { locator: { cellName: 'LocPinX' }, name: 'LocPinX', formula: 'Width*0.5' },
    { locator: { cellName: 'LocPinY' }, name: 'LocPinY', formula: 'Height*0.5' },
    { locator: { section: 'Geometry', rowIndex: 0, rowType: 'MoveTo', cellName: 'X' }, name: 'X', formula: 'Width*0' },
    { locator: { section: 'Geometry', rowIndex: 0, rowType: 'MoveTo', cellName: 'Y' }, name: 'Y', formula: 'Height*1' },
    { locator: { section: 'Geometry', rowIndex: 1, rowType: 'LineTo', cellName: 'X' }, name: 'X', formula: 'Width*1' },
    { locator: { section: 'Geometry', rowIndex: 1, rowType: 'LineTo', cellName: 'Y' }, name: 'Y', formula: 'Height*1' },
    { locator: { section: 'Geometry', rowIndex: 2, rowType: 'LineTo', cellName: 'X' }, name: 'X', formula: 'Width*1' },
    { locator: { section: 'Geometry', rowIndex: 2, rowType: 'LineTo', cellName: 'Y' }, name: 'Y', formula: 'Height*0.22' },
    { locator: { section: 'Geometry', rowIndex: 3, rowType: 'LineTo', cellName: 'X' }, name: 'X', formula: 'Width*0.62' },
    { locator: { section: 'Geometry', rowIndex: 3, rowType: 'LineTo', cellName: 'Y' }, name: 'Y', formula: 'Height*0.22' },
    { locator: { section: 'Geometry', rowIndex: 4, rowType: 'LineTo', cellName: 'X' }, name: 'X', formula: 'Width*0.5' },
    { locator: { section: 'Geometry', rowIndex: 4, rowType: 'LineTo', cellName: 'Y' }, name: 'Y', formula: 'Height*0' },
    { locator: { section: 'Geometry', rowIndex: 5, rowType: 'LineTo', cellName: 'X' }, name: 'X', formula: 'Width*0.38' },
    { locator: { section: 'Geometry', rowIndex: 5, rowType: 'LineTo', cellName: 'Y' }, name: 'Y', formula: 'Height*0.22' },
    { locator: { section: 'Geometry', rowIndex: 6, rowType: 'LineTo', cellName: 'X' }, name: 'X', formula: 'Width*0' },
    { locator: { section: 'Geometry', rowIndex: 6, rowType: 'LineTo', cellName: 'Y' }, name: 'Y', formula: 'Height*0.22' },
    { locator: { section: 'Geometry', rowIndex: 7, rowType: 'Close', cellName: 'NoShow' }, name: 'NoShow', formula: '0' },
  ]);
});

test('derives every rectangular callout preview and geometry from shared vertices', () => {
  for (const [id, vertices] of Object.entries(calloutVertices)) {
    const shape = calloutShapes.find((candidate) => candidate.id === id)!;
    const cells = shape.draft(0, 0, 1, 1).cells.filter((cell) => cell.name === 'X' || cell.name === 'Y').slice(0, vertices.length * 2);
    expect(shape.preview.startsWith(previewPathForVertices(vertices))).toBe(true);
    expect(cells.map((cell) => cell.formula)).toEqual(vertices.flatMap(([x, y]) => [`Width*${x}`, `Height*${y}`]));
  }
});

test('draws every arrow and callout preview from its draft geometry', () => {
  for (const shape of [...arrowShapes, ...calloutShapes]) {
    const rows = new Map<number, { type: string; x?: number; y?: number }>();
    for (const cell of shape.draft(0, 0, 1, 1).cells) {
      const locator = cell.locator as { section?: string; rowIndex?: number; rowType?: string };
      if (locator.section !== 'Geometry' || locator.rowIndex === undefined) continue;
      const entry = rows.get(locator.rowIndex) ?? { type: locator.rowType ?? '' };
      if (cell.name !== 'X' && cell.name !== 'Y') { rows.set(locator.rowIndex, entry); continue; }
      const value = Number((cell.formula ?? '').replace(/^Width\*/, '').replace(/^Height\*/, ''));
      if (cell.name === 'X') entry.x = value; else entry.y = value;
      rows.set(locator.rowIndex, entry);
    }
    const ordered = [...rows.entries()].sort(([left], [right]) => left - right).map(([, row]) => row);
    const tokens = shape.preview.split(' ').filter(Boolean);
    const points: Array<[number, number]> = [];
    let closes = 0;
    for (let index = 0; index < tokens.length;) {
      const command = tokens[index++];
      if (command === 'Z') { closes += 1; continue; }
      if (command === 'M' || command === 'L') { points.push([Number(tokens[index++]), Number(tokens[index++])]); continue; }
      if (command === 'A') { index += 5; points.push([Number(tokens[index++]), Number(tokens[index++])]); continue; }
      throw new Error(`unexpected preview command ${command} in ${shape.id}`);
    }
    const ends = ordered.filter((row) => row.x !== undefined && row.y !== undefined).map((row) => [row.x as number, Number((1 - (row.y as number)).toFixed(12))] as [number, number]);
    expect({ shape: shape.id, points }).toEqual({ shape: shape.id, points: ends });
    expect(closes).toBe(ordered.filter((row) => row.type === 'Close').length);
  }
});

test('keeps every arrow arc off its chord so the engine never flattens it', () => {
  for (const shape of arrowShapes) {
    const cells = shape.draft(0, 0, 1, 1).cells;
    const number = (prefix: string, name: string, rowIndex: number): number | undefined => {
      const cell = cells.find((candidate) => {
        const locator = candidate.locator as { section?: string; rowIndex?: number };
        return locator.section === 'Geometry' && locator.rowIndex === rowIndex && candidate.name === name;
      });
      const raw = cell?.formula?.replace(new RegExp(`^${prefix}\\*`), '');
      const value = raw === undefined ? Number.NaN : Number(raw);
      return Number.isFinite(value) ? value : undefined;
    };
    let previous: [number, number] | undefined;
    let index = 0;
    for (;;) {
      const row = cells.find((candidate) => {
        const locator = candidate.locator as { section?: string; rowIndex?: number; rowType?: string };
        return locator.section === 'Geometry' && locator.rowIndex === index;
      });
      const rowType = (row?.locator as { rowType?: string } | undefined)?.rowType;
      if (rowType === undefined) break;
      const end: [number, number] | undefined = (() => {
        const x = number('Width', 'X', index);
        const y = number('Height', 'Y', index);
        return x === undefined || y === undefined ? undefined : [x, y];
      })();
      if (rowType === 'EllipticalArcTo') {
        const ax = number('Width', 'A', index);
        const ay = number('Height', 'B', index);
        expect(ax).toBeDefined();
        expect(ay).toBeDefined();
        const [sx, sy] = previous!;
        const [ex, ey] = end!;
        const area = Math.abs((ex - sx) * (ay! - sy) - (ey - sy) * (ax! - sx));
        expect(area).toBeGreaterThan(1e-6);
      }
      if (end) previous = end;
      index += 1;
    }
    const arcs = (shape.preview.match(/ A /g) ?? []).length;
    const flags = [...shape.preview.matchAll(/A [0-9.]+ [0-9.]+ 0 (\d) (\d)/g)].map((match) => [match[1], match[2]]);
    expect(flags).toHaveLength(arcs);
    for (const [large] of flags) expect(large).toBe('0');
  }
});

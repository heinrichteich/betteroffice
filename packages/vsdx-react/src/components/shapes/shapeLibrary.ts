import type { FormulaShapeDraft } from '@betteroffice/vsdx';
import type { TranslationKey } from '@betteroffice/vsdx-i18n';

type Point = readonly [number, number];
type GeometryRowType = 'MoveTo' | 'LineTo' | 'EllipticalArcTo' | 'Close';

interface GeometryLocator {
  section: 'Geometry';
  rowIndex: number;
  rowType: GeometryRowType;
  cellName: string;
}

interface GeometryRow {
  type: GeometryRowType;
  end?: Point;
  through?: Point;
  axisRatio?: string;
}

interface GeometryPath {
  rows: readonly GeometryRow[];
  preview: string;
}

export interface StandardShape {
  id: string;
  nameKey: TranslationKey;
  preview: string;
  draft: (x: number, y: number, width: number, height: number) => FormulaShapeDraft;
}

export interface ShapeStencil {
  id: string;
  nameKey: TranslationKey;
  shapes: readonly StandardShape[];
}

export const polygonVertices: Readonly<Record<string, readonly Point[]>> = {
  rectangle: [[0, 0], [1, 0], [1, 1], [0, 1]],
  square: [[0, 0], [1, 0], [1, 1], [0, 1]],
  rightTriangle: [[0, 0], [1, 0], [0, 1]],
  triangle: [[0, 0], [1, 0], [0.5, 1]],
  pentagon: regularPolygon(5),
  hexagon: regularPolygon(6),
  heptagon: regularPolygon(7),
  octagon: regularPolygon(8),
  decagon: regularPolygon(10),
  diamond: [[0.5, 1], [1, 0.5], [0.5, 0], [0, 0.5]],
  cross: [[0.3, 1], [0.7, 1], [0.7, 0.7], [1, 0.7], [1, 0.3], [0.7, 0.3], [0.7, 0], [0.3, 0], [0.3, 0.3], [0, 0.3], [0, 0.7], [0.3, 0.7]],
  chevron: [[0, 0.2], [0.38, 0.2], [0.62, 0], [1, 0.5], [0.62, 1], [0.38, 0.8], [0, 0.8], [0.35, 0.5]],
  parallelogram: [[0.2, 0], [1, 0], [0.8, 1], [0, 1]],
  trapezoid: [[0, 0], [1, 0], [0.8, 1], [0.2, 1]],
  cube: [[0.5, 1], [1, 0.75], [1, 0.25], [0.5, 0], [0, 0.25], [0, 0.75]],
};

function regularPolygon(sides: number): readonly Point[] {
  return Array.from({ length: sides }, (_, index) => {
    const angle = Math.PI / 2 + (Math.PI * 2 * index) / sides;
    return [cleanNumber(0.5 + 0.5 * Math.cos(angle)), cleanNumber(0.5 + 0.5 * Math.sin(angle))] as const;
  });
}

function cleanNumber(value: number): number {
  return Number.isFinite(value) ? Number(value.toFixed(12)) : 0;
}

function numberFormula(value: number): string {
  const safe = cleanNumber(value);
  return Object.is(safe, -0) ? '0' : String(safe);
}

function dimension(value: number, fallback: number): number {
  return Math.max(0.01, Math.abs(Number.isFinite(value) ? value : fallback));
}

function coordinate(value: number): number {
  return Number.isFinite(value) ? value : 0;
}

function pointFormulas([x, y]: Point): [string, string] {
  return [`Width*${numberFormula(x)}`, `Height*${numberFormula(y)}`];
}

function polygonPath(vertices: readonly Point[], extraRows: readonly GeometryRow[] = []): GeometryPath {
  const rows: GeometryRow[] = [
    { type: 'MoveTo', end: vertices[0] },
    ...vertices.slice(1).map((end) => ({ type: 'LineTo' as const, end })),
    { type: 'Close' },
    ...extraRows,
  ];
  return { rows, preview: `${previewPathForVertices(vertices)}${svgRows(extraRows)}` };
}

export function previewPathForVertices(vertices: readonly Point[]): string {
  return `${vertices.map(([x, y], index) => `${index === 0 ? 'M' : 'L'} ${numberFormula(x)} ${numberFormula(1 - y)}`).join(' ')} Z`;
}

function svgRows(rows: readonly GeometryRow[]): string {
  let current: Point = [0, 0];
  return rows.map((row) => {
    if (row.type === 'MoveTo' && row.end) {
      current = row.end;
      return ` M ${numberFormula(current[0])} ${numberFormula(1 - current[1])}`;
    }
    if (row.type === 'LineTo' && row.end) {
      current = row.end;
      return ` L ${numberFormula(current[0])} ${numberFormula(1 - current[1])}`;
    }
    if (row.type === 'Close') return ' Z';
    return '';
  }).join('');
}

function ellipticalPath(rows: readonly GeometryRow[], preview: string): GeometryPath {
  return { rows, preview };
}

function geometryCells(path: GeometryPath): FormulaShapeDraft['cells'] {
  return path.rows.flatMap((row, rowIndex) => {
    const locator = (cellName: string): GeometryLocator => ({ section: 'Geometry', rowIndex, rowType: row.type, cellName });
    if (row.type === 'Close') return [{ locator: locator('NoShow'), name: 'NoShow', formula: '0' }];
    if (!row.end) return [];
    const [x, y] = pointFormulas(row.end);
    const cells = [
      { locator: locator('X'), name: 'X', formula: x },
      { locator: locator('Y'), name: 'Y', formula: y },
    ];
    if (row.type !== 'EllipticalArcTo' || !row.through) return cells;
    const [a, b] = pointFormulas(row.through);
    return [
      ...cells,
      { locator: locator('A'), name: 'A', formula: a },
      { locator: locator('B'), name: 'B', formula: b },
      { locator: locator('C'), name: 'C', formula: '0' },
      { locator: locator('D'), name: 'D', formula: row.axisRatio ?? 'Width/Height' },
    ];
  });
}

function draftFor(id: string, path: GeometryPath, square: boolean, linePattern = '1') {
  return (x: number, y: number, width: number, height: number): FormulaShapeDraft => {
    const safeWidth = dimension(width, 1);
    const safeHeight = square ? safeWidth : dimension(height, 1);
    return {
      name: id,
      cells: [
        { locator: { cellName: 'PinX' }, name: 'PinX', formula: numberFormula(coordinate(x)) },
        { locator: { cellName: 'PinY' }, name: 'PinY', formula: numberFormula(coordinate(y)) },
        { locator: { cellName: 'Width' }, name: 'Width', formula: numberFormula(safeWidth) },
        { locator: { cellName: 'Height' }, name: 'Height', formula: square ? 'Width' : numberFormula(safeHeight) },
        { locator: { cellName: 'LocPinX' }, name: 'LocPinX', formula: 'Width*0.5' },
        { locator: { cellName: 'LocPinY' }, name: 'LocPinY', formula: 'Height*0.5' },
        ...geometryCells(path),
        ...Object.entries({ Angle: '0', FlipX: '0', FlipY: '0', FillPattern: '1', FillForegnd: 'RGB(255,255,255)', LinePattern: linePattern, LineColor: 'RGB(23,32,51)', LineWeight: '0.01' }).map(([name, formula]) => ({ locator: { cellName: name }, name, formula })),
      ],
    };
  };
}

function polygonShape(id: keyof typeof polygonVertices, extraRows: readonly GeometryRow[] = [], square = false): StandardShape {
  const path = polygonPath(polygonVertices[id], extraRows);
  return {
    id,
    nameKey: `shapesPanel.shape.${id}` as TranslationKey,
    preview: path.preview,
    draft: draftFor(id, path, square),
  };
}

const circlePath = ellipticalPath([
  { type: 'MoveTo', end: [1, 0.5] },
  { type: 'EllipticalArcTo', end: [0.5, 1], through: [0.853553390593, 0.853553390593], axisRatio: '1' },
  { type: 'EllipticalArcTo', end: [0, 0.5], through: [0.146446609407, 0.853553390593], axisRatio: '1' },
  { type: 'EllipticalArcTo', end: [0.5, 0], through: [0.146446609407, 0.146446609407], axisRatio: '1' },
  { type: 'EllipticalArcTo', end: [1, 0.5], through: [0.853553390593, 0.146446609407], axisRatio: '1' },
  { type: 'Close' },
], 'M 1 0.5 A 0.5 0.5 0 1 1 0 0.5 A 0.5 0.5 0 1 1 1 0.5 Z');

const ellipsePath = ellipticalPath([
  { type: 'MoveTo', end: [1, 0.5] },
  { type: 'EllipticalArcTo', end: [0.5, 1], through: [0.853553390593, 0.853553390593] },
  { type: 'EllipticalArcTo', end: [0, 0.5], through: [0.146446609407, 0.853553390593] },
  { type: 'EllipticalArcTo', end: [0.5, 0], through: [0.146446609407, 0.146446609407] },
  { type: 'EllipticalArcTo', end: [1, 0.5], through: [0.853553390593, 0.146446609407] },
  { type: 'Close' },
], 'M 1 0.5 A 0.5 0.5 0 1 1 0 0.5 A 0.5 0.5 0 1 1 1 0.5 Z');

const cylinderPath = ellipticalPath([
  { type: 'MoveTo', end: [0, 0.72] },
  { type: 'EllipticalArcTo', end: [1, 0.72], through: [0.5, 1], axisRatio: 'Width/(Height*0.56)' },
  { type: 'LineTo', end: [1, 0.28] },
  { type: 'EllipticalArcTo', end: [0, 0.28], through: [0.5, 0], axisRatio: 'Width/(Height*0.56)' },
  { type: 'LineTo', end: [0, 0.72] },
  { type: 'Close' },
  { type: 'MoveTo', end: [0, 0.72] },
  { type: 'EllipticalArcTo', end: [1, 0.72], through: [0.5, 0.44], axisRatio: 'Width/(Height*0.56)' },
], 'M 0 0.28 A 0.5 0.28 0 0 1 1 0.28 L 1 0.72 A 0.5 0.28 0 0 1 0 0.72 Z M 0 0.28 A 0.5 0.28 0 0 0 1 0.28');

const arcPath = ellipticalPath([
  { type: 'MoveTo', end: [0, 0.15] },
  { type: 'EllipticalArcTo', end: [1, 0.15], through: [0.5, 0.85] },
], 'M 0 0.85 A 0.5 0.7 0 0 1 1 0.85');

const teardropPath = ellipticalPath([
  { type: 'MoveTo', end: [0.5, 1] },
  { type: 'EllipticalArcTo', end: [0.5, 0], through: [1, 0.42], axisRatio: '1' },
  { type: 'EllipticalArcTo', end: [0.5, 1], through: [0, 0.42], axisRatio: '1' },
  { type: 'Close' },
], 'M 0.5 0 C 0.9 0.3 1 0.8 0.5 1 C 0 0.8 0.1 0.3 0.5 0 Z');

const cubeEdges: readonly GeometryRow[] = [
  { type: 'MoveTo', end: [0.5, 1] },
  { type: 'LineTo', end: [0.5, 0.5] },
  { type: 'LineTo', end: [1, 0.25] },
  { type: 'MoveTo', end: [0.5, 0.5] },
  { type: 'LineTo', end: [0, 0.25] },
];

export const standardShapes: readonly StandardShape[] = [
  polygonShape('rectangle'),
  polygonShape('square', [], true),
  { id: 'circle', nameKey: 'shapesPanel.shape.circle', preview: circlePath.preview, draft: draftFor('circle', circlePath, true) },
  { id: 'ellipse', nameKey: 'shapesPanel.shape.ellipse', preview: ellipsePath.preview, draft: draftFor('ellipse', ellipsePath, false) },
  polygonShape('rightTriangle'),
  polygonShape('triangle'),
  polygonShape('pentagon'),
  polygonShape('hexagon'),
  polygonShape('heptagon'),
  polygonShape('octagon'),
  polygonShape('decagon'),
  polygonShape('diamond'),
  polygonShape('cross'),
  polygonShape('chevron'),
  polygonShape('parallelogram'),
  polygonShape('trapezoid'),
  { id: 'cylinder', nameKey: 'shapesPanel.shape.cylinder', preview: cylinderPath.preview, draft: draftFor('cylinder', cylinderPath, false) },
  polygonShape('cube', cubeEdges),
  { id: 'arc', nameKey: 'shapesPanel.shape.arc', preview: arcPath.preview, draft: draftFor('arc', arcPath, false) },
  { id: 'teardrop', nameKey: 'shapesPanel.shape.teardrop', preview: teardropPath.preview, draft: draftFor('teardrop', teardropPath, false) },
];

export const arrowVertices: Readonly<Record<string, readonly Point[]>> = {
  arrowRight: [[0, 0.35], [0.6, 0.35], [0.6, 0.15], [1, 0.5], [0.6, 0.85], [0.6, 0.65], [0, 0.65]],
  arrowLeft: [[1, 0.35], [0.4, 0.35], [0.4, 0.15], [0, 0.5], [0.4, 0.85], [0.4, 0.65], [1, 0.65]],
  arrowUp: [[0.35, 0], [0.35, 0.6], [0.15, 0.6], [0.5, 1], [0.85, 0.6], [0.65, 0.6], [0.65, 0]],
  arrowDown: [[0.35, 1], [0.35, 0.4], [0.15, 0.4], [0.5, 0], [0.85, 0.4], [0.65, 0.4], [0.65, 1]],
  arrowDoubleHorizontal: [[0, 0.5], [0.2, 0.15], [0.2, 0.35], [0.8, 0.35], [0.8, 0.15], [1, 0.5], [0.8, 0.85], [0.8, 0.65], [0.2, 0.65], [0.2, 0.85]],
  arrowDoubleVertical: [[0.5, 0], [0.15, 0.2], [0.35, 0.2], [0.35, 0.8], [0.15, 0.8], [0.5, 1], [0.85, 0.8], [0.65, 0.8], [0.65, 0.2], [0.85, 0.2]],
  stripedArrow: [[0.45, 0.15], [1, 0.5], [0.45, 0.85]],
  notched: [[0.25, 0.35], [0.6, 0.35], [0.6, 0.15], [1, 0.5], [0.6, 0.85], [0.6, 0.65], [0.25, 0.65], [0.4, 0.5]],
  blockArrow: [[0, 0.3], [0.55, 0.3], [0.55, 0.1], [1, 0.5], [0.55, 0.9], [0.55, 0.7], [0, 0.7]],
  quadArrow: [[0.5, 1], [0.7, 0.76], [0.6, 0.76], [0.6, 0.6], [0.76, 0.6], [0.76, 0.7], [1, 0.5], [0.76, 0.3], [0.76, 0.4], [0.6, 0.4], [0.6, 0.24], [0.7, 0.24], [0.5, 0], [0.3, 0.24], [0.4, 0.24], [0.4, 0.4], [0.24, 0.4], [0.24, 0.3], [0, 0.5], [0.24, 0.7], [0.24, 0.6], [0.4, 0.6], [0.4, 0.76], [0.3, 0.76]],
  leftRightUp: [[0.38, 0], [0.38, 0.38], [0.12, 0.38], [0, 0.5], [0.12, 0.62], [0.38, 0.62], [0.38, 0.7], [0.35, 0.7], [0.5, 1], [0.65, 0.7], [0.62, 0.7], [0.62, 0.62], [0.88, 0.62], [1, 0.5], [0.88, 0.38], [0.62, 0.38], [0.62, 0]],
  sharpBent: [[0, 0.25], [0.7, 0.25], [0.7, 0.62], [0.85, 0.62], [0.6, 1], [0.35, 0.62], [0.5, 0.62], [0.5, 0.45], [0, 0.45]],
};

function arrowPolygonShape(id: string, vertices: readonly Point[], extraRows: readonly GeometryRow[] = []): StandardShape {
  const path = polygonPath(vertices, extraRows);
  return {
    id,
    nameKey: `shapesPanel.shape.${id}` as TranslationKey,
    preview: path.preview,
    draft: draftFor(id, path, false),
  };
}

function openShape(id: string, rows: readonly GeometryRow[], preview: string, linePattern = '1'): StandardShape {
  const path = ellipticalPath(rows, preview);
  return {
    id,
    nameKey: `shapesPanel.shape.${id}` as TranslationKey,
    preview: path.preview,
    draft: draftFor(id, path, false, linePattern),
  };
}

const curvedArrowRightPath = ellipticalPath([
  { type: 'MoveTo', end: [0.05, 0.3] },
  { type: 'EllipticalArcTo', end: [0.66, 0.3], through: [0.355, 0.72] },
  { type: 'MoveTo', end: [0.9, 0.3] },
  { type: 'LineTo', end: [0.66, 0.42] },
  { type: 'LineTo', end: [0.66, 0.18] },
  { type: 'Close' },
], 'M 0.05 0.7 A 0.31 0.21 0 0 1 0.66 0.7 M 0.9 0.7 L 0.66 0.58 L 0.66 0.82 Z');

const curvedArrowLeftPath = ellipticalPath([
  { type: 'MoveTo', end: [0.95, 0.3] },
  { type: 'EllipticalArcTo', end: [0.34, 0.3], through: [0.645, 0.72] },
  { type: 'MoveTo', end: [0.1, 0.3] },
  { type: 'LineTo', end: [0.34, 0.42] },
  { type: 'LineTo', end: [0.34, 0.18] },
  { type: 'Close' },
], 'M 0.95 0.7 A 0.31 0.21 0 0 0 0.34 0.7 M 0.1 0.7 L 0.34 0.58 L 0.34 0.82 Z');

const curvedArrowUpPath = ellipticalPath([
  { type: 'MoveTo', end: [0.3, 0.05] },
  { type: 'EllipticalArcTo', end: [0.3, 0.66], through: [0.72, 0.355] },
  { type: 'MoveTo', end: [0.3, 0.9] },
  { type: 'LineTo', end: [0.18, 0.66] },
  { type: 'LineTo', end: [0.42, 0.66] },
  { type: 'Close' },
], 'M 0.3 0.95 A 0.21 0.31 0 0 1 0.3 0.34 M 0.3 0.1 L 0.18 0.34 L 0.42 0.34 Z');

const curvedArrowDownPath = ellipticalPath([
  { type: 'MoveTo', end: [0.3, 0.95] },
  { type: 'EllipticalArcTo', end: [0.3, 0.34], through: [0.72, 0.645] },
  { type: 'MoveTo', end: [0.3, 0.1] },
  { type: 'LineTo', end: [0.18, 0.34] },
  { type: 'LineTo', end: [0.42, 0.34] },
  { type: 'Close' },
], 'M 0.3 0.05 A 0.21 0.31 0 0 0 0.3 0.66 M 0.3 0.9 L 0.18 0.66 L 0.42 0.66 Z');

const bentArrowPath = ellipticalPath([
  { type: 'MoveTo', end: [0, 0.25] },
  { type: 'LineTo', end: [0.5, 0.25] },
  { type: 'EllipticalArcTo', end: [0.7, 0.45], through: [0.641421356237, 0.308578643763] },
  { type: 'LineTo', end: [0.7, 0.62] },
  { type: 'LineTo', end: [0.85, 0.62] },
  { type: 'LineTo', end: [0.6, 1] },
  { type: 'LineTo', end: [0.35, 0.62] },
  { type: 'LineTo', end: [0.5, 0.62] },
  { type: 'LineTo', end: [0.5, 0.45] },
  { type: 'LineTo', end: [0, 0.45] },
  { type: 'Close' },
], 'M 0 0.75 L 0.5 0.75 A 0.2 0.2 0 0 0 0.7 0.55 L 0.7 0.38 L 0.85 0.38 L 0.6 0 L 0.35 0.38 L 0.5 0.38 L 0.5 0.55 L 0 0.55 Z');

const uTurnArrowPath = ellipticalPath([
  { type: 'MoveTo', end: [0, 0.1] },
  { type: 'LineTo', end: [0.6, 0.1] },
  { type: 'EllipticalArcTo', end: [1, 0.5], through: [0.882842712475, 0.217157287525] },
  { type: 'EllipticalArcTo', end: [0.6, 0.9], through: [0.882842712475, 0.782842712475] },
  { type: 'LineTo', end: [0.3, 0.9] },
  { type: 'LineTo', end: [0.3, 0.98] },
  { type: 'LineTo', end: [0, 0.8] },
  { type: 'LineTo', end: [0.3, 0.62] },
  { type: 'LineTo', end: [0.3, 0.7] },
  { type: 'LineTo', end: [0.6, 0.7] },
  { type: 'EllipticalArcTo', end: [0.8, 0.5], through: [0.741421356237, 0.641421356237] },
  { type: 'EllipticalArcTo', end: [0.6, 0.3], through: [0.741421356237, 0.358578643763] },
  { type: 'LineTo', end: [0, 0.3] },
  { type: 'Close' },
], 'M 0 0.9 L 0.6 0.9 A 0.4 0.4 0 0 0 1 0.5 A 0.4 0.4 0 0 0 0.6 0.1 L 0.3 0.1 L 0.3 0.02 L 0 0.2 L 0.3 0.38 L 0.3 0.3 L 0.6 0.3 A 0.2 0.2 0 0 1 0.8 0.5 A 0.2 0.2 0 0 1 0.6 0.7 L 0 0.7 Z');

const circularArrowPath = ellipticalPath([
  { type: 'MoveTo', end: [0.72, 0.138948822335] },
  { type: 'EllipticalArcTo', end: [0.837059554972, 0.802826548262], through: [0.933315411325, 0.443594801827] },
  { type: 'EllipticalArcTo', end: [0.162940445028, 0.802826548262], through: [0.5, 0.96] },
  { type: 'EllipticalArcTo', end: [0.28, 0.138948822335], through: [0.066684588675, 0.443594801827] },
  { type: 'LineTo', end: [0.409903810568, 0.063948822335] },
  { type: 'LineTo', end: [0.38, 0.312153903092] },
  { type: 'EllipticalArcTo', end: [0.316149333651, 0.674269026325], through: [0.263646139277, 0.47832443736] },
  { type: 'EllipticalArcTo', end: [0.683850666349, 0.674269026325], through: [0.5, 0.76] },
  { type: 'EllipticalArcTo', end: [0.62, 0.312153903092], through: [0.736353860723, 0.47832443736] },
  { type: 'Close' },
], 'M 0.72 0.861051177665 A 0.44 0.44 0 0 0 0.837059554972 0.197173451738 A 0.44 0.44 0 0 0 0.162940445028 0.197173451738 A 0.44 0.44 0 0 0 0.28 0.861051177665 L 0.409903810568 0.936051177665 L 0.38 0.687846096908 A 0.24 0.24 0 0 1 0.316149333651 0.325730973675 A 0.24 0.24 0 0 1 0.683850666349 0.325730973675 A 0.24 0.24 0 0 1 0.62 0.687846096908 Z');

const stripedArrowStripes: readonly GeometryRow[] = [
  { type: 'MoveTo', end: [0, 0.32] },
  { type: 'LineTo', end: [0.42, 0.32] },
  { type: 'MoveTo', end: [0, 0.5] },
  { type: 'LineTo', end: [0.42, 0.5] },
  { type: 'MoveTo', end: [0, 0.68] },
  { type: 'LineTo', end: [0.42, 0.68] },
];

export const arrowShapes: readonly StandardShape[] = [
  arrowPolygonShape('arrowRight', arrowVertices.arrowRight),
  arrowPolygonShape('arrowLeft', arrowVertices.arrowLeft),
  arrowPolygonShape('arrowUp', arrowVertices.arrowUp),
  arrowPolygonShape('arrowDown', arrowVertices.arrowDown),
  arrowPolygonShape('arrowDoubleHorizontal', arrowVertices.arrowDoubleHorizontal),
  arrowPolygonShape('arrowDoubleVertical', arrowVertices.arrowDoubleVertical),
  { id: 'curvedArrowRight', nameKey: 'shapesPanel.shape.curvedArrowRight', preview: curvedArrowRightPath.preview, draft: draftFor('curvedArrowRight', curvedArrowRightPath, false) },
  { id: 'curvedArrowLeft', nameKey: 'shapesPanel.shape.curvedArrowLeft', preview: curvedArrowLeftPath.preview, draft: draftFor('curvedArrowLeft', curvedArrowLeftPath, false) },
  { id: 'curvedArrowUp', nameKey: 'shapesPanel.shape.curvedArrowUp', preview: curvedArrowUpPath.preview, draft: draftFor('curvedArrowUp', curvedArrowUpPath, false) },
  { id: 'curvedArrowDown', nameKey: 'shapesPanel.shape.curvedArrowDown', preview: curvedArrowDownPath.preview, draft: draftFor('curvedArrowDown', curvedArrowDownPath, false) },
  openShape('lineArrowRight', [
    { type: 'MoveTo', end: [0, 0.5] },
    { type: 'LineTo', end: [0.76, 0.5] },
    { type: 'MoveTo', end: [1, 0.5] },
    { type: 'LineTo', end: [0.76, 0.62] },
    { type: 'LineTo', end: [0.76, 0.38] },
    { type: 'Close' },
  ], 'M 0 0.5 L 0.76 0.5 M 1 0.5 L 0.76 0.38 L 0.76 0.62 Z'),
  openShape('lineArrowLeft', [
    { type: 'MoveTo', end: [1, 0.5] },
    { type: 'LineTo', end: [0.24, 0.5] },
    { type: 'MoveTo', end: [0, 0.5] },
    { type: 'LineTo', end: [0.24, 0.62] },
    { type: 'LineTo', end: [0.24, 0.38] },
    { type: 'Close' },
  ], 'M 1 0.5 L 0.24 0.5 M 0 0.5 L 0.24 0.38 L 0.24 0.62 Z'),
  openShape('lineArrowUp', [
    { type: 'MoveTo', end: [0.5, 0] },
    { type: 'LineTo', end: [0.5, 0.76] },
    { type: 'MoveTo', end: [0.5, 1] },
    { type: 'LineTo', end: [0.38, 0.76] },
    { type: 'LineTo', end: [0.62, 0.76] },
    { type: 'Close' },
  ], 'M 0.5 1 L 0.5 0.24 M 0.5 0 L 0.38 0.24 L 0.62 0.24 Z'),
  openShape('lineArrowDown', [
    { type: 'MoveTo', end: [0.5, 1] },
    { type: 'LineTo', end: [0.5, 0.24] },
    { type: 'MoveTo', end: [0.5, 0] },
    { type: 'LineTo', end: [0.38, 0.24] },
    { type: 'LineTo', end: [0.62, 0.24] },
    { type: 'Close' },
  ], 'M 0.5 0 L 0.5 0.76 M 0.5 1 L 0.38 0.76 L 0.62 0.76 Z'),
  openShape('lineHorizontal', [
    { type: 'MoveTo', end: [0, 0.5] },
    { type: 'LineTo', end: [1, 0.5] },
  ], 'M 0 0.5 L 1 0.5'),
  openShape('lineVertical', [
    { type: 'MoveTo', end: [0.5, 0] },
    { type: 'LineTo', end: [0.5, 1] },
  ], 'M 0.5 1 L 0.5 0'),
  openShape('lineDiagonal', [
    { type: 'MoveTo', end: [0.05, 0.05] },
    { type: 'LineTo', end: [0.95, 0.95] },
  ], 'M 0.05 0.95 L 0.95 0.05'),
  openShape('lineElbow', [
    { type: 'MoveTo', end: [0.05, 0.7] },
    { type: 'LineTo', end: [0.55, 0.7] },
    { type: 'LineTo', end: [0.55, 0.3] },
  ], 'M 0.05 0.3 L 0.55 0.3 L 0.55 0.7'),
  { id: 'bentArrow', nameKey: 'shapesPanel.shape.bentArrow', preview: bentArrowPath.preview, draft: draftFor('bentArrow', bentArrowPath, false) },
  { id: 'uTurnArrow', nameKey: 'shapesPanel.shape.uTurnArrow', preview: uTurnArrowPath.preview, draft: draftFor('uTurnArrow', uTurnArrowPath, false) },
  arrowPolygonShape('sharpBent', arrowVertices.sharpBent),
  arrowPolygonShape('stripedArrow', arrowVertices.stripedArrow, stripedArrowStripes),
  arrowPolygonShape('notched', arrowVertices.notched),
  arrowPolygonShape('blockArrow', arrowVertices.blockArrow),
  { id: 'circularArrow', nameKey: 'shapesPanel.shape.circularArrow', preview: circularArrowPath.preview, draft: draftFor('circularArrow', circularArrowPath, false) },
  arrowPolygonShape('quadArrow', arrowVertices.quadArrow),
  arrowPolygonShape('leftRightUp', arrowVertices.leftRightUp),
  openShape('arcedLine', [
    { type: 'MoveTo', end: [0.2, 0.9] },
    { type: 'EllipticalArcTo', end: [0.8, 0.3], through: [0.624264068712, 0.724264068712] },
    { type: 'MoveTo', end: [0.8, 0.16] },
    { type: 'LineTo', end: [0.72, 0.3] },
    { type: 'LineTo', end: [0.88, 0.3] },
    { type: 'Close' },
  ], 'M 0.2 0.1 A 0.6 0.6 0 0 1 0.8 0.7 M 0.8 0.84 L 0.72 0.7 L 0.88 0.7 Z'),
];

export function arrowShapeById(id: string): StandardShape | undefined {
  return arrowShapes.find((shape) => shape.id === id);
}

export const shapeStencils: readonly ShapeStencil[] = [
  { id: 'standard', nameKey: 'shapesPanel.standardShapes', shapes: standardShapes },
  { id: 'arrows', nameKey: 'shapesPanel.arrowShapes', shapes: arrowShapes },
];

export function standardShapeById(id: string): StandardShape | undefined {
  return standardShapes.find((shape) => shape.id === id);
}

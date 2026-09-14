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

export const polygonVertices: Readonly<Record<string, readonly Point[]>> = {
  rectangle: [[0, 0], [1, 0], [1, 1], [0, 1]],
  square: [[0, 0], [1, 0], [1, 1], [0, 1]],
  rightTriangle: [[0, 0], [1, 0], [0, 1]],
  triangle: [[0, 0], [1, 0], [0.5, 1]],
  rotatedTriangle: [[0, 1], [1, 1], [0.5, 0]],
  funnel: [[0, 1], [1, 1], [0.62, 0.45], [0.62, 0], [0.38, 0], [0.38, 0.45]],
  pyramid: [[0.5, 0.95], [0.9, 0.45], [0.75, 0.15], [0.15, 0.15]],
  star4: starVertices(4, 0.35),
  star5: starVertices(5, 0.382),
  star6: starVertices(6, 0.4),
  star7: starVertices(7, 0.45),
  star16: starVertices(16, 0.75),
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
  cube: [[0, 0], [0.75, 0], [1, 0.25], [1, 1], [0.25, 1], [0, 0.75]],
};

function regularPolygon(sides: number): readonly Point[] {
  return Array.from({ length: sides }, (_, index) => {
    const angle = Math.PI / 2 + (Math.PI * 2 * index) / sides;
    return [cleanNumber(0.5 + 0.5 * Math.cos(angle)), cleanNumber(0.5 + 0.5 * Math.sin(angle))] as const;
  });
}

function starVertices(points: number, innerRatio: number): readonly Point[] {
  return Array.from({ length: points * 2 }, (_, index) => {
    const angle = Math.PI / 2 + (Math.PI * index) / points;
    const radius = index % 2 === 0 ? 0.5 : 0.5 * innerRatio;
    return [cleanNumber(0.5 + radius * Math.cos(angle)), cleanNumber(0.5 + radius * Math.sin(angle))] as const;
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

function draftFor(id: string, path: GeometryPath, square: boolean, aspectRatio = 0) {
  return (x: number, y: number, width: number, height: number): FormulaShapeDraft => {
    const safeWidth = dimension(width, 1);
    const safeHeight = square ? safeWidth : aspectRatio > 0 ? safeWidth / aspectRatio : dimension(height, 1);
    return {
      name: id,
      cells: [
        { locator: { cellName: 'PinX' }, name: 'PinX', formula: numberFormula(coordinate(x)) },
        { locator: { cellName: 'PinY' }, name: 'PinY', formula: numberFormula(coordinate(y)) },
        { locator: { cellName: 'Width' }, name: 'Width', formula: numberFormula(safeWidth) },
        { locator: { cellName: 'Height' }, name: 'Height', formula: square ? 'Width' : aspectRatio > 0 ? `Width/${numberFormula(aspectRatio)}` : numberFormula(safeHeight) },
        { locator: { cellName: 'LocPinX' }, name: 'LocPinX', formula: 'Width*0.5' },
        { locator: { cellName: 'LocPinY' }, name: 'LocPinY', formula: 'Height*0.5' },
        ...geometryCells(path),
        ...Object.entries({ Angle: '0', FlipX: '0', FlipY: '0', FillPattern: '1', FillForegnd: 'RGB(255,255,255)', LinePattern: '1', LineColor: 'RGB(23,32,51)', LineWeight: '0.01' }).map(([name, formula]) => ({ locator: { cellName: name }, name, formula })),
      ],
    };
  };
}

function polygonShape(id: keyof typeof polygonVertices, extraRows: readonly GeometryRow[] = [], square = false, preview?: string, aspectRatio = 0): StandardShape {
  const path = polygonPath(polygonVertices[id], extraRows);
  return {
    id,
    nameKey: `shapesPanel.shape.${id}` as TranslationKey,
    preview: preview ?? path.preview,
    draft: draftFor(id, path, square, aspectRatio),
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
], 'M 1 0.5 A 0.5 0.333333333333 0 1 1 0 0.5 A 0.5 0.333333333333 0 1 1 1 0.5 Z');

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

const semicirclePath = ellipticalPath([
  { type: 'MoveTo', end: [0, 0] },
  { type: 'LineTo', end: [1, 0] },
  { type: 'EllipticalArcTo', end: [0, 0], through: [0.5, 0.5], axisRatio: '1' },
  { type: 'Close' },
], 'M 0 1 L 1 1 A 0.5 0.5 0 0 0 0 1 Z');

const halfEllipsePath = ellipticalPath([
  { type: 'MoveTo', end: [0, 0] },
  { type: 'LineTo', end: [1, 0] },
  { type: 'EllipticalArcTo', end: [0, 0], through: [0.5, 0.35], axisRatio: 'Width/(Height*0.7)' },
  { type: 'Close' },
], 'M 0 1 L 1 1 A 0.5 0.35 0 0 0 0 1 Z');

const teardropPath = ellipticalPath([
  { type: 'MoveTo', end: [0.5, 1] },
  { type: 'LineTo', end: [0.782352941176, 0.470588235294] },
  { type: 'EllipticalArcTo', end: [0.217647058824, 0.470588235294], through: [0.5, 0], axisRatio: '1' },
  { type: 'LineTo', end: [0.5, 1] },
  { type: 'Close' },
], 'M 0.5 0 L 0.782353 0.529412 A 0.32 0.32 0 1 1 0.217647 0.529412 L 0.5 0 Z');

const pointedOvalPath = ellipticalPath([
  { type: 'MoveTo', end: [0.5, 1] },
  { type: 'EllipticalArcTo', end: [0.5, 0], through: [0.9, 0.5], axisRatio: 'Width/(Height*1.25)' },
  { type: 'EllipticalArcTo', end: [0.5, 1], through: [0.1, 0.5], axisRatio: 'Width/(Height*1.25)' },
  { type: 'Close' },
], 'M 0.5 0 A 0.4 0.5 0 0 1 0.5 1 A 0.4 0.5 0 0 1 0.5 0 Z');

const conePath = ellipticalPath([
  { type: 'MoveTo', end: [0.5, 1] },
  { type: 'LineTo', end: [0, 0.22] },
  { type: 'EllipticalArcTo', end: [1, 0.22], through: [0.5, 0.08], axisRatio: 'Width/(Height*0.28)' },
  { type: 'LineTo', end: [0.5, 1] },
  { type: 'Close' },
  { type: 'MoveTo', end: [0, 0.22] },
  { type: 'EllipticalArcTo', end: [1, 0.22], through: [0.5, 0.36], axisRatio: 'Width/(Height*0.28)' },
], 'M 0.5 0 L 0 0.78 A 0.5 0.14 0 0 0 1 0.78 L 0.5 0 Z M 0 0.78 A 0.5 0.14 0 0 1 1 0.78');

const invertedConePath = ellipticalPath([
  { type: 'MoveTo', end: [0.5, 0] },
  { type: 'LineTo', end: [0, 0.78] },
  { type: 'EllipticalArcTo', end: [1, 0.78], through: [0.5, 0.64], axisRatio: 'Width/(Height*0.28)' },
  { type: 'LineTo', end: [0.5, 0] },
  { type: 'Close' },
  { type: 'MoveTo', end: [0, 0.78] },
  { type: 'EllipticalArcTo', end: [1, 0.78], through: [0.5, 0.92], axisRatio: 'Width/(Height*0.28)' },
], 'M 0.5 1 L 0 0.22 A 0.5 0.14 0 0 0 1 0.22 L 0.5 1 Z M 0 0.22 A 0.5 0.14 0 0 1 1 0.22');

const cubeEdges: readonly GeometryRow[] = [
  { type: 'MoveTo', end: [0, 0.75] },
  { type: 'LineTo', end: [0.75, 0.75] },
  { type: 'LineTo', end: [0.75, 0] },
  { type: 'MoveTo', end: [0.75, 0.75] },
  { type: 'LineTo', end: [1, 1] },
];

const pyramidEdges: readonly GeometryRow[] = [
  { type: 'MoveTo', end: [0.15, 0.15] },
  { type: 'LineTo', end: [0.3, 0.45] },
  { type: 'LineTo', end: [0.9, 0.45] },
  { type: 'MoveTo', end: [0.5, 0.95] },
  { type: 'LineTo', end: [0.75, 0.15] },
  { type: 'MoveTo', end: [0.5, 0.95] },
  { type: 'LineTo', end: [0.3, 0.45] },
];

export const standardShapes: readonly StandardShape[] = [
  polygonShape('rectangle', [], false, 'M 0 0.3 L 1 0.3 L 1 0.7 L 0 0.7 Z'),
  polygonShape('square', [], true),
  { id: 'circle', nameKey: 'shapesPanel.shape.circle', preview: circlePath.preview, draft: draftFor('circle', circlePath, true) },
  { id: 'ellipse', nameKey: 'shapesPanel.shape.ellipse', preview: ellipsePath.preview, draft: draftFor('ellipse', ellipsePath, false, 1.5) },
  polygonShape('rightTriangle'),
  polygonShape('triangle'),
  polygonShape('rotatedTriangle'),
  polygonShape('pentagon'),
  polygonShape('hexagon'),
  polygonShape('heptagon'),
  polygonShape('octagon'),
  polygonShape('decagon'),
  { id: 'cylinder', nameKey: 'shapesPanel.shape.cylinder', preview: cylinderPath.preview, draft: draftFor('cylinder', cylinderPath, false) },
  polygonShape('parallelogram'),
  polygonShape('trapezoid'),
  polygonShape('diamond'),
  polygonShape('cross'),
  polygonShape('chevron'),
  polygonShape('cube', cubeEdges, false, undefined, 4 / 3),
  { id: 'teardrop', nameKey: 'shapesPanel.shape.teardrop', preview: teardropPath.preview, draft: draftFor('teardrop', teardropPath, false) },
  { id: 'semicircle', nameKey: 'shapesPanel.shape.semicircle', preview: semicirclePath.preview, draft: draftFor('semicircle', semicirclePath, false) },
  { id: 'halfEllipse', nameKey: 'shapesPanel.shape.halfEllipse', preview: halfEllipsePath.preview, draft: draftFor('halfEllipse', halfEllipsePath, false) },
  { id: 'cone', nameKey: 'shapesPanel.shape.cone', preview: conePath.preview, draft: draftFor('cone', conePath, false) },
  { id: 'invertedCone', nameKey: 'shapesPanel.shape.invertedCone', preview: invertedConePath.preview, draft: draftFor('invertedCone', invertedConePath, false) },
  polygonShape('pyramid', pyramidEdges),
  { id: 'pointedOval', nameKey: 'shapesPanel.shape.pointedOval', preview: pointedOvalPath.preview, draft: draftFor('pointedOval', pointedOvalPath, false) },
  polygonShape('funnel'),
  polygonShape('star4'),
  polygonShape('star5'),
  polygonShape('star6'),
  polygonShape('star7'),
  polygonShape('star16'),
];

export function standardShapeById(id: string): StandardShape | undefined {
  return standardShapes.find((shape) => shape.id === id);
}

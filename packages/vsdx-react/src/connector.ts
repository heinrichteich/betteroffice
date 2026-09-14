import type { ConnectorGlue, FormulaShapeDraft, PageDisplayList, PagePrimitive, ShapeSnapshot } from '@betteroffice/vsdx';
import type { ModelPoint } from '@betteroffice/vsdx';
import { cellValue } from './components/ribbon/commands';

export type ConnectorSide = 'north' | 'east' | 'south' | 'west' | 'centre';

/** A snap target in model inches. Centre glue is dynamic; outline glue pins to a Connection row. */
export interface ConnectionPoint extends ModelPoint { side: ConnectorSide; toCell?: string; }

export interface ConnectorDragEndpoint { shapeId: string; point: ConnectionPoint; }

export type ConnectorEndpointGlue = 'point' | 'dynamic' | 'unglued';

export interface ConnectorOverlayRoute {
  route: readonly ModelPoint[];
  selected: boolean;
  beginGlue: ConnectorEndpointGlue;
  endGlue: ConnectorEndpointGlue;
}

export interface ConnectorOverlayScene {
  hoverPoints: readonly ConnectionPoint[];
  snapPoint: ConnectionPoint | null;
  previewRoute: readonly ModelPoint[] | null;
  reroutePreview: ReadonlyArray<readonly ModelPoint[]>;
  connectors: ReadonlyArray<ConnectorOverlayRoute>;
}

export const CONNECTOR_SNAP_INCHES = 0.12;
export const CONNECTOR_GLUE_MATCH_INCHES = 1e-6;
const MIN_SPAN_INCHES = 0.01;

function cellNumber(shape: ShapeSnapshot, name: string): number | undefined {
  const raw = cellValue(shape, name);
  if (raw === undefined) return undefined;
  const parsed = Number(raw);
  return Number.isFinite(parsed) ? parsed : undefined;
}

/** Mirrors the engine rule: nonzero OneD, else a full Begin/End endpoint set. */
export function isConnectorShape(shape: ShapeSnapshot): boolean {
  const oneD = cellValue(shape, 'OneD');
  if (oneD !== undefined) {
    const parsed = Number(oneD.trim().replace(/^=/, ''));
    return Number.isNaN(parsed) ? true : parsed !== 0;
  }
  return ['BeginX', 'BeginY', 'EndX', 'EndY'].every((name) => cellValue(shape, name) !== undefined);
}

function shapeBounds(shape: ShapeSnapshot): { left: number; bottom: number; right: number; top: number; centre: ModelPoint } | null {
  const pinX = cellNumber(shape, 'PinX');
  const pinY = cellNumber(shape, 'PinY');
  const width = cellNumber(shape, 'Width');
  const height = cellNumber(shape, 'Height');
  if (pinX === undefined || pinY === undefined || width === undefined || height === undefined) return null;
  if (!(width > 0) || !(height > 0)) return null;
  const locPinX = cellNumber(shape, 'LocPinX') ?? width / 2;
  const locPinY = cellNumber(shape, 'LocPinY') ?? height / 2;
  const left = pinX - locPinX;
  const bottom = pinY - locPinY;
  const centre = { x: pinX, y: pinY };
  return { left, bottom, right: left + width, top: bottom + height, centre };
}

/** Five snap targets in model inches: outline midpoints plus the dynamic centre. */
export function connectionPointsForShape(shape: ShapeSnapshot): ConnectionPoint[] {
  const bounds = shapeBounds(shape);
  if (!bounds) return [];
  const centreX = (bounds.left + bounds.right) / 2;
  const centreY = (bounds.bottom + bounds.top) / 2;
  return [
    { side: 'north', x: centreX, y: bounds.top, toCell: 'Connections.X1' },
    { side: 'east', x: bounds.right, y: centreY, toCell: 'Connections.X2' },
    { side: 'south', x: centreX, y: bounds.bottom, toCell: 'Connections.X3' },
    { side: 'west', x: bounds.left, y: centreY, toCell: 'Connections.X4' },
    { side: 'centre', x: bounds.centre.x, y: bounds.centre.y },
  ];
}

/** Matches the engine RoutStyle rule: nonzero style bends horizontal-first. */
export function routeConnector(from: ModelPoint, to: ModelPoint): ModelPoint[] {
  const start = { x: from.x, y: from.y };
  const end = { x: to.x, y: to.y };
  if (start.x === end.x || start.y === end.y) return [start, end];
  return [start, { x: end.x, y: start.y }, end];
}

/** Centre endpoints glue dynamically; outline endpoints pin to a Connection row. */
export function connectorGlue(shapeId: string, point: ConnectionPoint): ConnectorGlue {
  return point.toCell === undefined ? { shapeId } : { shapeId, toCell: point.toCell };
}

/** Inch formula without exponent noise or negative zero. */
export function formatInches(value: number): string {
  if (!Number.isFinite(value)) throw new Error('Connector geometry must be finite.');
  const rounded = Number(value.toFixed(6));
  return String(Object.is(rounded, -0) ? 0 : rounded);
}

/** 1D draft whose route the engine resolves from glue at layout time. */
export function connectorDraft(from: ModelPoint, to: ModelPoint): FormulaShapeDraft {
  const midX = (from.x + to.x) / 2;
  const midY = (from.y + to.y) / 2;
  const cell = (cellName: string, formula: string) => ({ locator: { cellName }, name: cellName, formula });
  return {
    name: 'Dynamic connector',
    cells: [
      cell('OneD', '1'),
      cell('BeginX', formatInches(from.x)),
      cell('BeginY', formatInches(from.y)),
      cell('EndX', formatInches(to.x)),
      cell('EndY', formatInches(to.y)),
      cell('PinX', formatInches(midX)),
      cell('PinY', formatInches(midY)),
      cell('Width', formatInches(Math.max(Math.abs(to.x - from.x), MIN_SPAN_INCHES))),
      cell('Height', formatInches(Math.max(Math.abs(to.y - from.y), MIN_SPAN_INCHES))),
      cell('RoutStyle', '1'),
      cell('EndArrow', '4'),
      cell('LineColor', 'RGB(23,32,51)'),
      cell('LineWeight', '0.02'),
      cell('LinePattern', '1'),
    ],
  };
}

/** Nearest snap target within the model-space threshold. */
export function nearestConnectionPoint(points: readonly ConnectionPoint[], at: ModelPoint, threshold = CONNECTOR_SNAP_INCHES): ConnectionPoint | null {
  let best: ConnectionPoint | null = null;
  let bestDistance = threshold;
  for (const point of points) {
    const distance = Math.hypot(point.x - at.x, point.y - at.y);
    if (distance <= bestDistance) { best = point; bestDistance = distance; }
  }
  return best;
}

/** Closest point with no threshold; used once a containing shape is known. */
export function nearestConnectionPointAnywhere(points: readonly ConnectionPoint[], at: ModelPoint): ConnectionPoint | null {
  let best: ConnectionPoint | null = null;
  let bestDistance = Number.POSITIVE_INFINITY;
  for (const point of points) {
    const distance = Math.hypot(point.x - at.x, point.y - at.y);
    if (distance < bestDistance) { best = point; bestDistance = distance; }
  }
  return best;
}

/** Snap within threshold, else the nearest point of a shape containing the drop. */
export function dropTargetForPoint(shapes: readonly ShapeSnapshot[], at: ModelPoint): { shapeId: string; point: ConnectionPoint } | null {
  let best: { shapeId: string; point: ConnectionPoint } | null = null;
  let bestDistance = CONNECTOR_SNAP_INCHES;
  for (const shape of shapes) {
    if (isConnectorShape(shape)) continue;
    const point = nearestConnectionPoint(connectionPointsForShape(shape), at, bestDistance);
    if (!point) continue;
    bestDistance = Math.hypot(point.x - at.x, point.y - at.y);
    best = { shapeId: shape.id, point };
  }
  if (best) return best;
  let fallback: { shapeId: string; point: ConnectionPoint } | null = null;
  let fallbackDistance = Number.POSITIVE_INFINITY;
  for (const shape of shapes) {
    if (isConnectorShape(shape)) continue;
    const bounds = shapeBounds(shape);
    if (!bounds) continue;
    if (at.x < bounds.left - CONNECTOR_SNAP_INCHES || at.x > bounds.right + CONNECTOR_SNAP_INCHES) continue;
    if (at.y < bounds.bottom - CONNECTOR_SNAP_INCHES || at.y > bounds.top + CONNECTOR_SNAP_INCHES) continue;
    const point = nearestConnectionPointAnywhere(connectionPointsForShape(shape), at);
    if (!point) continue;
    const distance = Math.hypot(point.x - at.x, point.y - at.y);
    if (distance < fallbackDistance) { fallbackDistance = distance; fallback = { shapeId: shape.id, point }; }
  }
  return fallback;
}

function findPrimitive(primitives: readonly PagePrimitive[], id: string, depth = 0): PagePrimitive | null {
  if (depth >= 256) return null;
  for (const primitive of primitives) {
    if (primitive.id === id) return primitive;
    if (primitive.kind === 'group') {
      const nested = findPrimitive(primitive.primitives, id, depth + 1);
      if (nested) return nested;
    }
  }
  return null;
}

/** Engine-resolved route in model inches, or null when the connector did not paint as a path. */
export function connectorRouteFromFrame(frame: PageDisplayList, sourcePartPath: string, sourceId: number): ModelPoint[] | null {
  const primitive = findPrimitive(frame.primitives, `${sourcePartPath}:${sourceId}`);
  if (!primitive || primitive.kind !== 'shape') return null;
  const route: ModelPoint[] = [];
  for (const command of primitive.path) {
    if ((command.type === 'move' || command.type === 'line') && Number.isFinite(Number(command.x)) && Number.isFinite(Number(command.y))) {
      route.push({ x: Number(command.x), y: Number(command.y) });
    }
  }
  return route.length >= 2 ? route : null;
}

/** Point-glued outline match wins over a coincident centre; otherwise unglued. */
export function classifyConnectorEndpoint(at: ModelPoint, shapes: readonly ShapeSnapshot[], tolerance = CONNECTOR_GLUE_MATCH_INCHES): ConnectorEndpointGlue {
  let dynamic = false;
  for (const shape of shapes) {
    if (isConnectorShape(shape)) continue;
    for (const point of connectionPointsForShape(shape)) {
      if (Math.hypot(point.x - at.x, point.y - at.y) > tolerance) continue;
      if (point.side !== 'centre') return 'point';
      dynamic = true;
    }
  }
  return dynamic ? 'dynamic' : 'unglued';
}

/** Glue states for both route ends, read from geometry. */
export function connectorEndpointGlue(route: readonly ModelPoint[], shapes: readonly ShapeSnapshot[]): [ConnectorEndpointGlue, ConnectorEndpointGlue] {
  if (route.length < 2) return ['unglued', 'unglued'];
  return [classifyConnectorEndpoint(route[0], shapes), classifyConnectorEndpoint(route[route.length - 1], shapes)];
}

export interface MovedShapeGeometry { x: number; y: number; width: number; height: number; }

/** Connection points of a dragged shape at its preview geometry, in model inches. */
export function movedShapePoints(shape: ShapeSnapshot, geometry: MovedShapeGeometry, resize: boolean): ConnectionPoint[] {
  if (!resize) {
    const pinX = cellNumber(shape, 'PinX');
    const pinY = cellNumber(shape, 'PinY');
    if (pinX === undefined || pinY === undefined) return [];
    const dx = geometry.x - pinX;
    const dy = geometry.y - pinY;
    return connectionPointsForShape(shape).map((point) => ({ ...point, x: point.x + dx, y: point.y + dy }));
  }
  const moved: ShapeSnapshot = {
    ...shape,
    cells: shape.cells.map((cell) => {
      if (cell.name === 'PinX') return { ...cell, formula: String(geometry.x), value: String(geometry.x) };
      if (cell.name === 'PinY') return { ...cell, formula: String(geometry.y), value: String(geometry.y) };
      if (cell.name === 'Width') return { ...cell, formula: String(geometry.width), value: String(geometry.width) };
      if (cell.name === 'Height') return { ...cell, formula: String(geometry.height), value: String(geometry.height) };
      return cell;
    }),
  };
  return connectionPointsForShape(moved);
}

function gluedSide(points: readonly ConnectionPoint[], at: ModelPoint): number {
  let best = -1;
  let bestDistance = CONNECTOR_GLUE_MATCH_INCHES;
  for (let index = 0; index < points.length; index++) {
    const distance = Math.hypot(points[index].x - at.x, points[index].y - at.y);
    if (distance <= bestDistance) { best = index; bestDistance = distance; }
  }
  return best;
}

function flattenConnectorShapes(shapes: readonly ShapeSnapshot[]): ShapeSnapshot[] {
  return shapes.flatMap((shape) => [shape, ...flattenConnectorShapes(shape.children)]);
}

/** Whole-route recompute for connectors glued to a dragged shape, in model inches. */
export function reroutePreviewForMove(shapes: readonly ShapeSnapshot[], frame: PageDisplayList, sourcePartPath: string, dragged: ShapeSnapshot, geometry: MovedShapeGeometry): ModelPoint[][] {
  if (isConnectorShape(dragged) || !shapes.some((shape) => shape.id === dragged.id)) return [];
  const before = connectionPointsForShape(dragged);
  if (!before.length) return [];
  const width = cellNumber(dragged, 'Width');
  const height = cellNumber(dragged, 'Height');
  const resize = width !== undefined && height !== undefined && (geometry.width !== width || geometry.height !== height);
  const after = movedShapePoints(dragged, geometry, resize);
  if (after.length !== before.length) return [];
  const previews: ModelPoint[][] = [];
  for (const shape of flattenConnectorShapes(shapes)) {
    if (!isConnectorShape(shape)) continue;
    const route = connectorRouteFromFrame(frame, sourcePartPath, shape.sourceId);
    if (!route || route.length < 2) continue;
    const beginSide = gluedSide(before, route[0]);
    const endSide = gluedSide(before, route[route.length - 1]);
    if (beginSide < 0 && endSide < 0) continue;
    const nextBegin = beginSide >= 0 ? after[beginSide] : route[0];
    const nextEnd = endSide >= 0 ? after[endSide] : route[route.length - 1];
    previews.push(routeConnector(nextBegin, nextEnd));
  }
  return previews;
}

/** Model inches to page pixels at zoom 1; callers scale linearly by zoom. */
export function modelToPage(frame: PageDisplayList, point: ModelPoint): ModelPoint {
  const t = frame.paintTransform;
  return { x: t.a * point.x + t.c * point.y + t.e, y: t.b * point.x + t.d * point.y + t.f };
}

function applyModelTransform(ctx: CanvasRenderingContext2D, frame: PageDisplayList, dpr: number, zoom: number): void {
  const t = frame.paintTransform;
  ctx.setTransform(dpr * zoom, 0, 0, dpr * zoom, 0, 0);
  ctx.transform(t.a, t.b, t.c, t.d, t.e, t.f);
}

function dot(ctx: CanvasRenderingContext2D, at: ModelPoint, radius: number, fill: string): void {
  ctx.beginPath();
  ctx.arc(at.x, at.y, radius, 0, Math.PI * 2);
  ctx.fillStyle = fill;
  ctx.fill();
}

function ring(ctx: CanvasRenderingContext2D, at: ModelPoint, radius: number, color: string, width: number): void {
  ctx.beginPath();
  ctx.arc(at.x, at.y, radius, 0, Math.PI * 2);
  ctx.strokeStyle = color;
  ctx.lineWidth = width;
  ctx.stroke();
}

function strokeRoute(ctx: CanvasRenderingContext2D, route: readonly ModelPoint[], color: string, width: number): void {
  ctx.beginPath();
  route.forEach((point, index) => { if (index === 0) ctx.moveTo(point.x, point.y); else ctx.lineTo(point.x, point.y); });
  ctx.strokeStyle = color;
  ctx.lineWidth = width;
  ctx.lineJoin = 'round';
  ctx.lineCap = 'round';
  ctx.stroke();
}

/** Filled arrowhead in model inches, pointing along the final segment. */
export function arrowheadPolygon(tip: ModelPoint, tail: ModelPoint, length = 0.15, halfWidth = 0.055): ModelPoint[] {
  const dx = tip.x - tail.x;
  const dy = tip.y - tail.y;
  const size = Math.hypot(dx, dy);
  if (!(size > 0)) return [];
  const ux = dx / size;
  const uy = dy / size;
  return [
    { ...tip },
    { x: tip.x - ux * length - uy * halfWidth, y: tip.y - uy * length + ux * halfWidth },
    { x: tip.x - ux * length + uy * halfWidth, y: tip.y - uy * length - ux * halfWidth },
  ];
}

function paintArrowhead(ctx: CanvasRenderingContext2D, route: readonly ModelPoint[], fill: string): void {
  if (route.length < 2) return;
  const polygon = arrowheadPolygon(route[route.length - 1], route[route.length - 2]);
  if (!polygon.length) return;
  ctx.beginPath();
  polygon.forEach((point, index) => { if (index === 0) ctx.moveTo(point.x, point.y); else ctx.lineTo(point.x, point.y); });
  ctx.closePath();
  ctx.fillStyle = fill;
  ctx.fill();
}

function segmentMidpoints(route: readonly ModelPoint[]): ModelPoint[] {
  const result: ModelPoint[] = [];
  for (let index = 1; index < route.length; index++) {
    result.push({ x: (route[index - 1].x + route[index].x) / 2, y: (route[index - 1].y + route[index].y) / 2 });
  }
  return result;
}

/** Paints one selected endpoint from its glue state. */
export function paintConnectorEndpoint(ctx: CanvasRenderingContext2D, at: ModelPoint, glue: ConnectorEndpointGlue): void {
  if (glue === 'point') dot(ctx, at, 0.06, '#16a34a');
  else if (glue === 'dynamic') ring(ctx, at, 0.09, '#16a34a', 0.025);
  else ring(ctx, at, 0.09, '#9aa5b4', 0.025);
}

/** Overlay chrome in model space; zoom applies once through the canvas transform. */
export function paintConnectorOverlay(ctx: CanvasRenderingContext2D, frame: PageDisplayList, dpr: number, zoom: number, scene: ConnectorOverlayScene): void {
  applyModelTransform(ctx, frame, dpr, zoom);
  for (const connector of scene.connectors) {
    paintArrowhead(ctx, connector.route, '#172033');
    if (!connector.selected) continue;
    strokeRoute(ctx, connector.route, '#16a34a', 0.015);
    if (connector.route.length >= 1) paintConnectorEndpoint(ctx, connector.route[0], connector.beginGlue);
    if (connector.route.length >= 2) paintConnectorEndpoint(ctx, connector.route[connector.route.length - 1], connector.endGlue);
    ctx.fillStyle = '#2563eb';
    for (const midpoint of segmentMidpoints(connector.route)) ctx.fillRect(midpoint.x - 0.045, midpoint.y - 0.045, 0.09, 0.09);
  }
  for (const point of scene.hoverPoints) dot(ctx, point, point.side === 'centre' ? 0.07 : 0.055, '#16a34a');
  if (scene.snapPoint) ring(ctx, scene.snapPoint, 0.1, '#ffffff', 0.025);
  if (scene.previewRoute && scene.previewRoute.length >= 2) {
    strokeRoute(ctx, scene.previewRoute, '#172033', 0.02);
    paintArrowhead(ctx, scene.previewRoute, '#172033');
  }
  for (const route of scene.reroutePreview) {
    if (route.length < 2) continue;
    ctx.save();
    ctx.setLineDash([0.08, 0.05]);
    strokeRoute(ctx, route, '#172033', 0.02);
    ctx.restore();
    paintArrowhead(ctx, route, '#172033');
  }
}

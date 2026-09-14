import { expect, test } from 'bun:test';
import type { PageDisplayList, ShapeSnapshot } from '@betteroffice/vsdx';
import {
  arrowheadPolygon,
  autoConnectArrowAt,
  autoConnectArrowCenter,
  autoConnectArrowCss,
  autoConnectArrowsForShape,
  autoConnectHaloHit,
  autoConnectMetrics,
  classifyConnectorEndpoint,
  connectionPointsForShape,
  connectorDraft,
  connectorEndpointGlue,
  connectorGlue,
  connectorRouteFromFrame,
  dropTargetForPoint,
  formatInches,
  isConnectorShape,
  modelToPage,
  movedShapePoints,
  nearestConnectionPoint,
  nearestConnectionPointAnywhere,
  paintAutoConnectOverlay,
  paintConnectorEndpoint,
  paintConnectorOverlay,
  quickShapePlacement,
  reroutePreviewForMove,
  routeConnector,
} from './connector';

function shape(cells: Record<string, string>): ShapeSnapshot {
  return {
    id: 'shape',
    sourceId: 1,
    name: null,
    children: [],
    cells: Object.entries(cells).map(([name, formula]) => ({
      locator: { sheet: { page: 1 }, shapeId: 1, section: null, row: null, cellName: name },
      name,
      formula,
      value: formula,
    })),
  };
}

const frame: PageDisplayList = {
  contractVersion: 4,
  width: 816,
  height: 1056,
  paintTransform: { a: 96, b: 0, c: 0, d: -96, e: 0, f: 1056 },
  primitives: [],
};

test('paints four outline points plus the dynamic centre in model inches', () => {
  const points = connectionPointsForShape(shape({ PinX: '2', PinY: '3', Width: '4', Height: '2', LocPinX: '2', LocPinY: '1' }));
  expect(points).toEqual([
    { side: 'north', x: 2, y: 4, toCell: 'Connections.X1' },
    { side: 'east', x: 4, y: 3, toCell: 'Connections.X2' },
    { side: 'south', x: 2, y: 2, toCell: 'Connections.X3' },
    { side: 'west', x: 0, y: 3, toCell: 'Connections.X4' },
    { side: 'centre', x: 2, y: 3 },
  ]);
});

test('centres the pin when LocPin cells are absent', () => {
  const points = connectionPointsForShape(shape({ PinX: '5', PinY: '6', Width: '2', Height: '2' }));
  expect(points.find((point) => point.side === 'centre')).toEqual({ side: 'centre', x: 5, y: 6 });
  expect(points.find((point) => point.side === 'north')).toEqual({ side: 'north', x: 5, y: 7, toCell: 'Connections.X1' });
});

test('offers no points without resolvable bounds', () => {
  expect(connectionPointsForShape(shape({ PinX: '1' }))).toEqual([]);
  expect(connectionPointsForShape(shape({ PinX: '1', PinY: '1', Width: '0', Height: '1' }))).toEqual([]);
});

test('detects connectors by OneD or by a full endpoint set', () => {
  expect(isConnectorShape(shape({ OneD: '1' }))).toBe(true);
  expect(isConnectorShape(shape({ OneD: '0' }))).toBe(false);
  expect(isConnectorShape(shape({ BeginX: '1', BeginY: '1', EndX: '2', EndY: '2' }))).toBe(true);
  expect(isConnectorShape(shape({ PinX: '1', Width: '1' }))).toBe(false);
});

test('routes horizontal-first like the engine RoutStyle rule', () => {
  expect(routeConnector({ x: 1, y: 1 }, { x: 4, y: 3 })).toEqual([{ x: 1, y: 1 }, { x: 4, y: 1 }, { x: 4, y: 3 }]);
  expect(routeConnector({ x: 1, y: 2 }, { x: 4, y: 2 })).toEqual([{ x: 1, y: 2 }, { x: 4, y: 2 }]);
  expect(routeConnector({ x: 1, y: 1 }, { x: 1, y: 5 })).toEqual([{ x: 1, y: 1 }, { x: 1, y: 5 }]);
});

test('glues the centre dynamically and pins outline points to Connection rows', () => {
  const points = connectionPointsForShape(shape({ PinX: '2', PinY: '3', Width: '4', Height: '2', LocPinX: '2', LocPinY: '1' }));
  expect(connectorGlue('a', points[4])).toEqual({ shapeId: 'a' });
  expect(connectorGlue('a', points[0])).toEqual({ shapeId: 'a', toCell: 'Connections.X1' });
  expect(connectorGlue('a', points[3])).toEqual({ shapeId: 'a', toCell: 'Connections.X4' });
});

test('drafts an orthogonal connector with a target-end arrow', () => {
  const draft = connectorDraft({ x: 1, y: 1 }, { x: 4, y: 3 });
  const formulas = new Map(draft.cells.map((cell) => [cell.name, cell.formula]));
  expect(draft.name).toBe('Dynamic connector');
  expect(formulas.get('OneD')).toBe('1');
  expect(formulas.get('RoutStyle')).toBe('1');
  expect(formulas.get('EndArrow')).toBe('4');
  expect([formulas.get('BeginX'), formulas.get('BeginY'), formulas.get('EndX'), formulas.get('EndY')]).toEqual(['1', '1', '4', '3']);
  expect(Number(formulas.get('Width'))).toBeCloseTo(3);
  expect(Number(formulas.get('Height'))).toBeCloseTo(2);
});

test('formats inch formulas without exponent noise or negative zero', () => {
  expect(formatInches(2)).toBe('2');
  expect(formatInches(-0)).toBe('0');
  expect(formatInches(1 / 3)).toBe('0.333333');
  expect(() => formatInches(Number.NaN)).toThrow('Connector geometry must be finite');
});

test('snaps to the nearest point inside the model-space threshold', () => {
  const points = connectionPointsForShape(shape({ PinX: '2', PinY: '3', Width: '4', Height: '2', LocPinX: '2', LocPinY: '1' }));
  expect(nearestConnectionPoint(points, { x: 2.05, y: 4.02 })?.side).toBe('north');
  expect(nearestConnectionPoint(points, { x: 2, y: 3 })?.side).toBe('centre');
  expect(nearestConnectionPoint(points, { x: 9, y: 9 })).toBeNull();
});

test('projects model inches to page pixels linearly in zoom', () => {
  const page = modelToPage(frame, { x: 2, y: 2 });
  expect(page).toEqual({ x: 192, y: 864 });
  for (const zoom of [0.5, 1, 1.5]) {
    const scaled = { x: page.x * zoom, y: page.y * zoom };
    expect(scaled.x).toBeCloseTo(192 * zoom, 10);
    expect(scaled.y).toBeCloseTo(864 * zoom, 10);
  }
});

test('reads the engine-resolved route back from the display list', () => {
  const routed: PageDisplayList = {
    ...frame,
    primitives: [{ kind: 'shape', id: 'visio/pages/page1.xml:4', zOrder: 3, path: [{ type: 'move', x: 1, y: 1 }, { type: 'line', x: 4, y: 1 }, { type: 'line', x: 4, y: 3 }] }],
  };
  expect(connectorRouteFromFrame(routed, 'visio/pages/page1.xml', 4)).toEqual([{ x: 1, y: 1 }, { x: 4, y: 1 }, { x: 4, y: 3 }]);
  expect(connectorRouteFromFrame(routed, 'visio/pages/page1.xml', 9)).toBeNull();
  const placeholder: PageDisplayList = {
    ...frame,
    primitives: [{ kind: 'placeholder', id: 'visio/pages/page1.xml:4', zOrder: 3, x: 0, y: 0, width: 1, height: 1, reason: 'unresolved' }],
  };
  expect(connectorRouteFromFrame(placeholder, 'visio/pages/page1.xml', 4)).toBeNull();
});

test('points the arrowhead along the final segment', () => {
  const polygon = arrowheadPolygon({ x: 4, y: 2 }, { x: 3, y: 2 });
  expect(polygon[0]).toEqual({ x: 4, y: 2 });
  expect(polygon).toHaveLength(3);
  expect(polygon[1].x).toBeLessThan(4);
  expect(arrowheadPolygon({ x: 1, y: 1 }, { x: 1, y: 1 })).toEqual([]);
});

function recordingContext(): { ctx: CanvasRenderingContext2D; calls: Array<{ name: string; args: unknown[] }> } {
  const calls: Array<{ name: string; args: unknown[] }> = [];
  const transform = { a: 1, b: 0, c: 0, d: 1, e: 0, f: 1 };
  const apply = (t: { a: number; b: number; c: number; d: number; e: number; f: number }) => {
    const next = { a: transform.a * t.a + transform.c * t.b, b: transform.b * t.a + transform.d * t.b, c: transform.a * t.c + transform.c * t.d, d: transform.b * t.c + transform.d * t.d, e: transform.a * t.e + transform.c * t.f + transform.e, f: transform.b * t.e + transform.d * t.f + transform.f };
    Object.assign(transform, next);
  };
  const project = (x: number, y: number) => [transform.a * x + transform.c * y + transform.e, transform.b * x + transform.d * y + transform.f];
  const ctx = new Proxy({}, {
    get: (_target, name: string) => {
      if (name === 'canvas') return undefined;
      return (...args: unknown[]) => {
        if (name === 'setTransform') Object.assign(transform, { a: args[0], b: args[1], c: args[2], d: args[3], e: args[4], f: args[5] });
        if (name === 'transform') apply({ a: args[0] as number, b: args[1] as number, c: args[2] as number, d: args[3] as number, e: args[4] as number, f: args[5] as number });
        if (name === 'arc' || name === 'moveTo' || name === 'lineTo') {
          const [px, py] = project(args[0] as number, args[1] as number);
          calls.push({ name, args: [px, py, ...args.slice(2)] });
        } else calls.push({ name, args });
      };
    },
    set: () => true,
  }) as unknown as CanvasRenderingContext2D;
  return { ctx, calls };
}

test('reads point, dynamic and unglued ends from geometry', () => {
  const target = shape({ PinX: '2', PinY: '3', Width: '4', Height: '2', LocPinX: '2', LocPinY: '1' });
  const other = shape({ PinX: '20', PinY: '20', Width: '2', Height: '2' });
  const shapes = [target, other];
  expect(classifyConnectorEndpoint({ x: 2, y: 4 }, shapes)).toBe('point');
  expect(classifyConnectorEndpoint({ x: 4, y: 3 }, shapes)).toBe('point');
  expect(classifyConnectorEndpoint({ x: 2, y: 3 }, shapes)).toBe('dynamic');
  expect(classifyConnectorEndpoint({ x: 9, y: 9 }, shapes)).toBe('unglued');
  expect(connectorEndpointGlue([{ x: 2, y: 4 }, { x: 2, y: 3 }], shapes)).toEqual(['point', 'dynamic']);
  expect(connectorEndpointGlue([{ x: 2, y: 4 }, { x: 9, y: 9 }], shapes)).toEqual(['point', 'unglued']);
  expect(connectorEndpointGlue([{ x: 1, y: 1 }], shapes)).toEqual(['unglued', 'unglued']);
});

test('prefers an outline match over a coincident centre', () => {
  const target = shape({ PinX: '2', PinY: '2', Width: '2', Height: '2' });
  expect(classifyConnectorEndpoint({ x: 2, y: 2 }, [target])).toBe('dynamic');
  const overlapping = shape({ PinX: '2', PinY: '2', Width: '2', Height: '2' });
  const neighbour = shape({ PinX: '3', PinY: '2', Width: '2', Height: '2' });
  expect(classifyConnectorEndpoint({ x: 2, y: 2 }, [overlapping, neighbour])).toBe('point');
  expect(classifyConnectorEndpoint({ x: 4, y: 2 }, [overlapping, neighbour])).toBe('point');
});

test('ignores connector shapes when classifying an endpoint', () => {
  const target = shape({ PinX: '2', PinY: '3', Width: '4', Height: '2', LocPinX: '2', LocPinY: '1' });
  const connector = shape({ OneD: '1', PinX: '9', PinY: '9', Width: '1', Height: '1' });
  expect(classifyConnectorEndpoint({ x: 9, y: 9 }, [target, connector])).toBe('unglued');
  expect(classifyConnectorEndpoint({ x: 2, y: 3 }, [target, connector])).toBe('dynamic');
});

test('paints a filled dot, a ring, and a grey ring for the three glue states', () => {
  const calls: string[] = [];
  const colours: string[] = [];
  const ctx = new Proxy({}, {
    get: (_target, name: string) => (...args: unknown[]) => {
      calls.push(name);
      if (name === 'arc') colours.push(args.slice(0, 2).join(','));
    },
    set: (_target, name: string, value: unknown) => {
      if ((name === 'fillStyle' || name === 'strokeStyle') && typeof value === 'string') colours.push(value);
      return true;
    },
  }) as unknown as CanvasRenderingContext2D;
  const at = { x: 1, y: 1 };
  paintConnectorEndpoint(ctx, at, 'point');
  expect(calls).toEqual(['beginPath', 'arc', 'fill']);
  expect(colours).toEqual(['1,1', '#16a34a']);
  calls.length = 0;
  colours.length = 0;
  paintConnectorEndpoint(ctx, at, 'dynamic');
  expect(calls).toEqual(['beginPath', 'arc', 'stroke']);
  expect(colours).toEqual(['1,1', '#16a34a']);
  calls.length = 0;
  colours.length = 0;
  paintConnectorEndpoint(ctx, at, 'unglued');
  expect(calls).toEqual(['beginPath', 'arc', 'stroke']);
  expect(colours).toEqual(['1,1', '#9aa5b4']);
});

test('shifts every connection point rigidly on a move', () => {
  const target = shape({ PinX: '2', PinY: '3', Width: '4', Height: '2', LocPinX: '2', LocPinY: '1' });
  const before = connectionPointsForShape(target);
  const after = movedShapePoints(target, { x: 5, y: 6, width: 4, height: 2 }, false);
  expect(after).toHaveLength(before.length);
  for (let index = 0; index < before.length; index++) {
    expect(after[index].x).toBeCloseTo(before[index].x + 3, 10);
    expect(after[index].y).toBeCloseTo(before[index].y + 3, 10);
    expect(after[index].side).toBe(before[index].side);
  }
});

test('recomputes the whole route from the moved shape instead of translating it', () => {
  const dragged = { ...shape({ PinX: '2', PinY: '2', Width: '2', Height: '2' }), id: 'dragged', sourceId: 2 };
  const other = { ...shape({ PinX: '8', PinY: '8', Width: '2', Height: '2' }), id: 'other', sourceId: 3 };
  const connector = { ...shape({ OneD: '1', PinX: '5', PinY: '5', Width: '1', Height: '1' }), id: 'connector', sourceId: 4 };
  const shapes = [dragged, other, connector];
  const routed: PageDisplayList = {
    ...frame,
    primitives: [{ kind: 'shape', id: 'part:4', zOrder: 3, path: [{ type: 'move', x: 2, y: 2 }, { type: 'line', x: 8, y: 2 }, { type: 'line', x: 8, y: 8 }] }],
  };
  const previews = reroutePreviewForMove(shapes, routed, 'part', dragged, { x: 4, y: 5, width: 2, height: 2 });
  expect(previews).toEqual([[{ x: 4, y: 5 }, { x: 8, y: 5 }, { x: 8, y: 8 }]]);
  expect(reroutePreviewForMove(shapes, routed, 'part', other, { x: 9, y: 9, width: 2, height: 2 })).toEqual([
    [{ x: 2, y: 2 }, { x: 9, y: 2 }, { x: 9, y: 9 }],
  ]);
  expect(reroutePreviewForMove(shapes, routed, 'part', connector, { x: 9, y: 9, width: 1, height: 1 })).toEqual([]);
  const untouched: PageDisplayList = {
    ...frame,
    primitives: [{ kind: 'shape', id: 'part:4', zOrder: 3, path: [{ type: 'move', x: 0, y: 0 }, { type: 'line', x: 1, y: 1 }] }],
  };
  expect(reroutePreviewForMove(shapes, untouched, 'part', dragged, { x: 4, y: 5, width: 2, height: 2 })).toEqual([]);
});

test('keeps a point-glued end pinned to its outline side while the shape moves', () => {
  const dragged = { ...shape({ PinX: '2', PinY: '2', Width: '2', Height: '2' }), id: 'dragged', sourceId: 2 };
  const other = { ...shape({ PinX: '8', PinY: '2', Width: '2', Height: '2' }), id: 'other', sourceId: 3 };
  const connector = { ...shape({ OneD: '1', PinX: '5', PinY: '5', Width: '1', Height: '1' }), id: 'connector', sourceId: 4 };
  const shapes = [dragged, other, connector];
  const routed: PageDisplayList = {
    ...frame,
    primitives: [{ kind: 'shape', id: 'part:4', zOrder: 3, path: [{ type: 'move', x: 2, y: 3 }, { type: 'line', x: 8, y: 3 }, { type: 'line', x: 8, y: 2 }] }],
  };
  const previews = reroutePreviewForMove(shapes, routed, 'part', dragged, { x: 4, y: 4, width: 2, height: 2 });
  expect(previews).toEqual([[{ x: 4, y: 5 }, { x: 8, y: 5 }, { x: 8, y: 2 }]]);
});

test('paints overlay chrome once per zoom instead of zoom squared', () => {
  const scene = {
    hoverPoints: [{ side: 'centre' as const, x: 2, y: 2 }],
    snapPoint: null,
    previewRoute: [{ x: 1, y: 1 }, { x: 4, y: 1 }, { x: 4, y: 3 }],
    reroutePreview: [],
    connectors: [{ route: [{ x: 1, y: 1 }, { x: 4, y: 1 }, { x: 4, y: 3 }], selected: true, beginGlue: 'point' as const, endGlue: 'dynamic' as const }],
  };
  const atZoom = (zoom: number) => {
    const { ctx, calls } = recordingContext();
    paintConnectorOverlay(ctx, frame, 1, zoom, scene);
    return calls.filter((call) => call.name === 'arc').map((call) => call.args.slice(0, 2));
  };
  const half = atZoom(0.5);
  const full = atZoom(1);
  const oneHalf = atZoom(1.5);
  expect(half.length).toBeGreaterThan(0);
  expect(half.length).toBe(full.length);
  for (let index = 0; index < full.length; index++) {
    expect(full[index][0]).toBeCloseTo((half[index][0] as number) * 2, 8);
    expect(oneHalf[index][0]).toBeCloseTo((full[index][0] as number) * 1.5, 8);
    expect(oneHalf[index][1]).toBeCloseTo((full[index][1] as number) * 1.5, 8);
  }
});

function placedShape(id: string, pinX: number, pinY: number): ShapeSnapshot {
  return { ...shape({ PinX: String(pinX), PinY: String(pinY), Width: '1', Height: '1' }), id };
}

/** Interior drops must glue instead of silently refusing the gesture. */
test('glues an interior drop to its nearest connection point', () => {
  const shapes = [placedShape('a', 2, 2), placedShape('b', 5, 2)];
  expect(dropTargetForPoint(shapes, { x: 2, y: 2 })?.shapeId).toBe('a');
  const interior = dropTargetForPoint(shapes, { x: 5.2, y: 2.1 });
  expect(interior?.shapeId).toBe('b');
  expect(interior?.point).toEqual(expect.objectContaining({ side: 'centre' }));
  expect(dropTargetForPoint(shapes, { x: 20, y: 20 })).toBeNull();
});

test('prefers the threshold snap over an interior fallback', () => {
  const shapes = [placedShape('a', 2, 2), placedShape('b', 5, 2)];
  expect(dropTargetForPoint(shapes, { x: 2.5, y: 2 })?.point.side).toBe('east');
  expect(nearestConnectionPointAnywhere(connectionPointsForShape(shapes[0]), { x: 9, y: 9 })?.side).toBe('north');
});

test('offers one autoconnect arrow per edge and none for connectors', () => {
  const target = shape({ PinX: '2', PinY: '3', Width: '4', Height: '2', LocPinX: '2', LocPinY: '1' });
  const arrows = autoConnectArrowsForShape(target);
  expect(arrows.map((arrow) => arrow.side)).toEqual(['north', 'east', 'south', 'west']);
  expect(arrows.map((arrow) => arrow.point.toCell)).toEqual(['Connections.X1', 'Connections.X2', 'Connections.X3', 'Connections.X4']);
  expect(autoConnectArrowsForShape(shape({ OneD: '1', PinX: '1', PinY: '1', Width: '1', Height: '1' }))).toEqual([]);
  expect(autoConnectArrowsForShape(shape({ PinX: '1' }))).toEqual([]);
});

test('holds chevron centres a fixed screen distance outside the edge at every zoom', () => {
  const target = shape({ PinX: '2', PinY: '2', Width: '2', Height: '2' });
  const arrows = autoConnectArrowsForShape(target);
  for (const zoom of [0.5, 1, 1.5]) {
    const metrics = autoConnectMetrics(frame, zoom);
    expect(metrics.pixelsPerInch).toBeCloseTo(96 * zoom, 10);
    for (const arrow of arrows) {
      const css = autoConnectArrowCss(arrow, frame, zoom);
      const edge = modelToPage(frame, arrow.point);
      const gap = Math.hypot(css.x - edge.x * zoom, css.y - edge.y * zoom);
      expect(gap).toBeCloseTo(18, 8);
    }
  }
  const east = arrows.find((arrow) => arrow.side === 'east')!;
  const centre = autoConnectArrowCenter(east, frame, 1);
  expect(centre.x).toBeGreaterThan(east.point.x);
  expect(centre.y).toBeCloseTo(east.point.y, 10);
});

test('hit-tests chevrons in screen pixels, not model inches', () => {
  const target = shape({ PinX: '2', PinY: '2', Width: '2', Height: '2' });
  const arrows = autoConnectArrowsForShape(target);
  const east = arrows.find((arrow) => arrow.side === 'east')!;
  const page = modelToPage(frame, autoConnectArrowCenter(east, frame, 1));
  expect(autoConnectArrowAt(arrows, frame, 1, page)?.side).toBe('east');
  expect(autoConnectArrowAt(arrows, frame, 1, { x: 0, y: 0 })).toBeNull();
  expect(autoConnectArrowAt([], frame, 1, page)).toBeNull();
  const zoomed = modelToPage(frame, autoConnectArrowCenter(east, frame, 1.5));
  expect(autoConnectArrowAt(arrows, frame, 1.5, zoomed)?.side).toBe('east');
});

test('holds the hover halo across the edge-to-chevron gap at every zoom', () => {
  const target = shape({ PinX: '2', PinY: '2', Width: '2', Height: '2' });
  for (const zoom of [0.5, 1, 1.5]) {
    const east = modelToPage(frame, { x: 3, y: 2 });
    expect(autoConnectHaloHit(target, frame, zoom, east)).toBe(true);
    expect(autoConnectHaloHit(target, frame, zoom, { x: east.x + 39 / zoom, y: east.y })).toBe(true);
    expect(autoConnectHaloHit(target, frame, zoom, { x: east.x + 41 / zoom, y: east.y })).toBe(false);
    expect(autoConnectHaloHit(target, frame, zoom, { x: 0, y: 0 })).toBe(false);
  }
  expect(autoConnectHaloHit(shape({ OneD: '1', PinX: '1', PinY: '1', Width: '1', Height: '1' }), frame, 1, { x: 96, y: 960 })).toBe(false);
  expect(autoConnectHaloHit(shape({ PinX: '1' }), frame, 1, { x: 96, y: 960 })).toBe(false);
});

test('offsets a quick-shape insert past the source edge with opposing glue', () => {
  const target = shape({ PinX: '2', PinY: '2', Width: '2', Height: '2' });
  const east = quickShapePlacement(target, 'east', 2, 2)!;
  expect(east.x).toBeCloseTo(4.5, 10);
  expect(east.y).toBeCloseTo(2, 10);
  expect(east.from.side).toBe('east');
  expect(east.to).toEqual({ side: 'west', x: 3.5, y: 2, toCell: 'Connections.X4' });
  const north = quickShapePlacement(target, 'north', 2, 2)!;
  expect(north.y).toBeCloseTo(4.5, 10);
  expect(north.to).toEqual(expect.objectContaining({ side: 'south', toCell: 'Connections.X3' }));
  const south = quickShapePlacement(target, 'south', 2, 2)!;
  expect(south.to).toEqual(expect.objectContaining({ side: 'north', toCell: 'Connections.X1' }));
  const west = quickShapePlacement(target, 'west', 2, 2)!;
  expect(west.to).toEqual(expect.objectContaining({ side: 'east', toCell: 'Connections.X2' }));
  expect(quickShapePlacement(target, 'east', 0, 1)).toBeNull();
  expect(quickShapePlacement(shape({ PinX: '1' }), 'east', 1, 1)).toBeNull();
});

test('paints nothing without arrows or alpha', () => {
  const calls: string[] = [];
  const ctx = new Proxy({}, {
    get: (_target, name: string) => (..._args: unknown[]) => { calls.push(name); return undefined; },
    set: () => true,
  }) as unknown as CanvasRenderingContext2D;
  paintAutoConnectOverlay(ctx, frame, 1, 1, null);
  paintAutoConnectOverlay(ctx, frame, 1, 1, { arrows: [], hovered: null, alpha: 1 });
  paintAutoConnectOverlay(ctx, frame, 1, 1, { arrows: autoConnectArrowsForShape(shape({ PinX: '2', PinY: '2', Width: '2', Height: '2' })), hovered: null, alpha: 0 });
  expect(calls).toEqual([]);
  paintAutoConnectOverlay(ctx, frame, 1, 1, { arrows: autoConnectArrowsForShape(shape({ PinX: '2', PinY: '2', Width: '2', Height: '2' })), hovered: 'east', alpha: 1 });
  expect(calls.filter((name) => name === 'fill').length).toBe(4);
});

import { expect, test } from 'bun:test';
import type { PageDisplayList, PageSnapshot } from '@betteroffice/vsdx';
import { connectorRuns, dragSegmentRoute, hitSegmentDot, paintConnectorChrome, previewChrome, selectedConnectorChrome } from './connectorChrome';
import type { SelectedConnectorChrome } from './connectorChrome';

const page: PageSnapshot = {
  id: 'page:1',
  sourcePartPath: 'visio/pages/page1.xml',
  name: 'Page',
  shapes: [{ id: 'page:1:shape:7', sourceId: 7, name: 'Connector', cells: [], children: [] }],
};

function frameWith(connectorId: string): PageDisplayList {
  return {
    contractVersion: 4,
    width: 816,
    height: 1056,
    paintTransform: { a: 96, b: 0, c: 0, d: -96, e: 0, f: 1056 },
    primitives: [
      {
        kind: 'shape',
        id: connectorId,
        zOrder: 0,
        path: [
          { type: 'move', x: 1, y: 1 },
          { type: 'line', x: 4, y: 1 },
          { type: 'line', x: 4, y: 3 },
        ],
        transform: { a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 },
      },
    ],
    connectors: [{ id: connectorId, begin: 'point', end: 'free' }],
  } as unknown as PageDisplayList;
}

const frame = frameWith('visio/pages/page1.xml:7');

test('selects the connector route with one segment per straight run', () => {
  const selected = selectedConnectorChrome(frame, page, { pageId: 'page:1', shapeId: 'page:1:shape:7' });
  expect(selected?.start).toEqual({ x: 1, y: 1 });
  expect(selected?.end).toEqual({ x: 4, y: 3 });
  expect(selected?.segments).toEqual([
    { a: { x: 1, y: 1 }, b: { x: 4, y: 1 }, mid: { x: 2.5, y: 1 } },
    { a: { x: 4, y: 1 }, b: { x: 4, y: 3 }, mid: { x: 4, y: 2 } },
  ]);
});

test('ignores selections that are not a painted connector', () => {
  expect(selectedConnectorChrome(frame, page, { pageId: 'page:1', shapeId: 'missing' })).toBeNull();
  expect(selectedConnectorChrome(frame, page, { pageId: 'page:2', shapeId: 'page:1:shape:7' })).toBeNull();
  expect(selectedConnectorChrome({ ...frame, connectors: [] }, page, { pageId: 'page:1', shapeId: 'page:1:shape:7' })).toBeNull();
  expect(selectedConnectorChrome({ ...frame, primitives: [] }, page, { pageId: 'page:1', shapeId: 'page:1:shape:7' })).toBeNull();
});

test('breaks runs at curves and skips zero-length segments', () => {
  const runs = connectorRuns([
    { type: 'move', x: 0, y: 0 },
    { type: 'line', x: 2, y: 0 },
    { type: 'line', x: 2, y: 0 },
    { type: 'cubic', cp1x: 3, cp1y: 0, cp2x: 3, cp2y: 2, x: 4, y: 2 },
    { type: 'line', x: 6, y: 2 },
  ]);
  expect(runs).toEqual([
    [{ x: 0, y: 0 }, { x: 2, y: 0 }, { x: 2, y: 0 }],
    [{ x: 4, y: 2 }, { x: 6, y: 2 }],
  ]);
});

test('paints glue-coded endpoints and one blue dot per segment', () => {  const selected = selectedConnectorChrome(frame, page, { pageId: 'page:1', shapeId: 'page:1:shape:7' });
  const calls: string[] = [];
  const context = {
    save: () => calls.push('save'),
    restore: () => calls.push('restore'),
    setTransform: () => {},
    transform: () => {},
    beginPath: () => calls.push('beginPath'),
    arc: (x: number, y: number) => calls.push(`arc:${x},${y}`),
    fill: () => calls.push(`fill:${(context as unknown as Record<string, string>).fillStyle}`),
    stroke: () => calls.push(`stroke:${(context as unknown as Record<string, string>).strokeStyle}`),
    fillStyle: '',
    strokeStyle: '',
    lineWidth: 0,
  };
  paintConnectorChrome(context as unknown as CanvasRenderingContext2D, frame, selected as SelectedConnectorChrome, 1, 1);
  expect(calls).toContain('arc:2.5,1');
  expect(calls).toContain('arc:4,2');
  expect(calls).toContain('fill:#2563eb');
  expect(calls).toContain('stroke:#16a34a');
  expect(calls).toContain('stroke:#64748b');
});

test('exposes a draggable run only for all-straight routes', () => {
  const selected = selectedConnectorChrome(frame, page, { pageId: 'page:1', shapeId: 'page:1:shape:7' });
  expect(selected?.draggable).toEqual([
    { x: 1, y: 1 },
    { x: 4, y: 1 },
    { x: 4, y: 3 },
  ]);
  const curved: PageDisplayList = {
    ...frame,
    primitives: [
      {
        kind: 'shape',
        id: 'visio/pages/page1.xml:7',
        zOrder: 0,
        path: [
          { type: 'move', x: 0, y: 0 },
          { type: 'line', x: 2, y: 0 },
          { type: 'cubic', cp1x: 3, cp1y: 0, cp2x: 3, cp2y: 2, x: 4, y: 2 },
          { type: 'line', x: 6, y: 2 },
        ],
        transform: { a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 },
      },
    ],
  } as unknown as PageDisplayList;
  const bent = selectedConnectorChrome(curved, page, { pageId: 'page:1', shapeId: 'page:1:shape:7' });
  expect(bent?.segments.length).toBe(2);
  expect(bent?.draggable).toBeNull();
});

test('hits the nearest segment dot within tolerance', () => {
  const selected = selectedConnectorChrome(frame, page, { pageId: 'page:1', shapeId: 'page:1:shape:7' });
  expect(hitSegmentDot(selected as SelectedConnectorChrome, { x: 2.5, y: 1.05 }, 0.2)).toBe(0);
  expect(hitSegmentDot(selected as SelectedConnectorChrome, { x: 4, y: 2 }, 0.2)).toBe(1);
  expect(hitSegmentDot(selected as SelectedConnectorChrome, { x: 0, y: 0 }, 0.2)).toBeNull();
});

test('drags a segment perpendicular into a four-segment route', () => {
  const vertices = [{ x: 1, y: 1 }, { x: 4, y: 1 }, { x: 4, y: 3 }];
  const route = dragSegmentRoute(vertices, 0, { x: 2.5, y: 1 }, { x: 2.5, y: 2 });
  expect(route).toEqual([
    { x: 1, y: 1 },
    { x: 1, y: 2 },
    { x: 4, y: 2 },
    { x: 4, y: 1 },
    { x: 4, y: 3 },
  ]);
});

test('discards parallel travel and collapses a drag back', () => {
  const vertices = [{ x: 1, y: 1 }, { x: 4, y: 1 }, { x: 4, y: 3 }];
  expect(dragSegmentRoute(vertices, 0, { x: 2.5, y: 1 }, { x: 9, y: 1 })).toEqual(vertices);
  const moved = dragSegmentRoute(vertices, 0, { x: 2.5, y: 1 }, { x: 2.5, y: 2 });
  const back = dragSegmentRoute(moved, 1, { x: 2.5, y: 2 }, { x: 2.5, y: 1 });
  expect(back).toEqual(vertices);
});

test('drags diagonal segments along their normal', () => {
  const vertices = [{ x: 0, y: 0 }, { x: 2, y: 2 }];
  const route = dragSegmentRoute(vertices, 0, { x: 1, y: 1 }, { x: 2, y: 0 });
  expect(route.length).toBe(4);
  expect(route[0]).toEqual({ x: 0, y: 0 });
  expect(route[3]).toEqual({ x: 2, y: 2 });
  for (const [actual, wanted] of [[route[1], { x: 1, y: -1 }], [route[2], { x: 3, y: 1 }]] as const) {
    expect(actual.x).toBeCloseTo(wanted.x, 9);
    expect(actual.y).toBeCloseTo(wanted.y, 9);
  }
});

test('repaints previews with moved dots and fixed endpoints', () => {
  const selected = selectedConnectorChrome(frame, page, { pageId: 'page:1', shapeId: 'page:1:shape:7' });
  const preview = previewChrome(selected as SelectedConnectorChrome, [
    { x: 1, y: 1 },
    { x: 1, y: 2 },
    { x: 4, y: 2 },
    { x: 4, y: 1 },
    { x: 4, y: 3 },
  ]);
  expect(preview.start).toEqual({ x: 1, y: 1 });
  expect(preview.end).toEqual({ x: 4, y: 3 });
  expect(preview.segments.map((segment) => segment.mid)).toEqual([
    { x: 1, y: 1.5 },
    { x: 2.5, y: 2 },
    { x: 4, y: 1.5 },
    { x: 4, y: 2 },
  ]);
});

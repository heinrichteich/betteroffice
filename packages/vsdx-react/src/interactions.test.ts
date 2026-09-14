import { expect, test } from 'bun:test';
import { canvasPointToModel, modelPointToCanvas } from '@betteroffice/vsdx';
import type { ModelPoint } from '@betteroffice/vsdx';
import { paintDragPreview, passedDragThreshold, previewOutline, resolveDragGeometry } from './interactions';
const pagePaintTransform = { a: 96, b: 0, c: 0, d: -96, e: 0, f: 1056 };
const identity = { a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 };
test('passedDragThreshold needs four css pixels by default', () => {
  expect(passedDragThreshold(10, 10, 12, 12)).toBe(false);
  expect(passedDragThreshold(10, 10, 14, 10)).toBe(true);
  expect(passedDragThreshold(10, 10, 13, 10)).toBe(false);
  expect(passedDragThreshold(10, 10, 16, 10, 8)).toBe(false);
  expect(passedDragThreshold(10, 10, 18, 10, 8)).toBe(true);
});
test('previewOutline draws the moved box in canvas coordinates', () => {
  const start = { canvas: { x: 0, y: 0 }, model: { x: 3, y: 3 }, resize: false, pin: { x: 5, y: 2 }, size: { width: 2, height: 1 } };
  expect(previewOutline(start, { x: 4, y: 4 }, pagePaintTransform)).toEqual([{ x: 480, y: 816 }, { x: 672, y: 816 }, { x: 672, y: 720 }, { x: 480, y: 720 }]);
});
test('previewOutline rotates the box with the shape angle', () => {
  const start = { canvas: { x: 0, y: 0 }, model: { x: 0, y: 0 }, resize: false, pin: { x: 2, y: 3 }, size: { width: 4, height: 5 }, angle: Math.PI / 2 };
  const corners = previewOutline(start, { x: 0, y: 0 }, identity);
  expect(corners[0].x).toBeCloseTo(4.5, 10); expect(corners[0].y).toBeCloseTo(1, 10);
  expect(corners[1].x).toBeCloseTo(4.5, 10); expect(corners[1].y).toBeCloseTo(5, 10);
  expect(corners[2].x).toBeCloseTo(-0.5, 10); expect(corners[2].y).toBeCloseTo(5, 10);
  expect(corners[3].x).toBeCloseTo(-0.5, 10); expect(corners[3].y).toBeCloseTo(1, 10);
});
test('previewOutline honours a horizontal flip in corner order', () => {
  const base = { canvas: { x: 0, y: 0 }, model: { x: 3, y: 3 }, resize: false, pin: { x: 5, y: 2 }, size: { width: 2, height: 1 } };
  const plain = previewOutline(base, { x: 3, y: 3 }, identity);
  expect(plain).toEqual([{ x: 4, y: 1.5 }, { x: 6, y: 1.5 }, { x: 6, y: 2.5 }, { x: 4, y: 2.5 }]);
  const flipped = previewOutline({ ...base, flipX: true }, { x: 3, y: 3 }, identity);
  expect(flipped).toEqual([{ x: 6, y: 1.5 }, { x: 4, y: 1.5 }, { x: 4, y: 2.5 }, { x: 6, y: 2.5 }]);
});
test('previewOutline maps the box through the group transform forward', () => {
  const start = { canvas: { x: 0, y: 0 }, model: { x: 10, y: 20 }, resize: false, pin: { x: 2, y: 3 }, size: { width: 4, height: 5 }, parentTransforms: [{ a: 0, b: 2, c: -2, d: 0, e: 10, f: 20 }] };
  expect(previewOutline(start, { x: 8, y: 24 }, identity)).toEqual([{ x: 7, y: 24 }, { x: 7, y: 32 }, { x: -3, y: 32 }, { x: -3, y: 24 }]);
});
test('preview geometry and commit geometry describe the same rectangle', () => {
  const starts = [
    { canvas: { x: 0, y: 0 }, model: { x: 3, y: 3 }, resize: false, pin: { x: 5, y: 2 }, size: { width: 2, height: 1 } },
    { canvas: { x: 0, y: 0 }, model: { x: 0, y: 0 }, resize: false, pin: { x: 2, y: 3 }, size: { width: 4, height: 5 }, angle: Math.PI / 2 },
    { canvas: { x: 0, y: 0 }, model: { x: 10, y: 20 }, resize: false, pin: { x: 2, y: 3 }, size: { width: 4, height: 5 }, parentTransforms: [{ a: 0, b: 2, c: -2, d: 0, e: 10, f: 20 }] },
    { canvas: { x: 0, y: 0 }, model: { x: 3, y: 3 }, resize: true, pin: { x: 5, y: 2 }, size: { width: 2, height: 1 } },
  ];
  const releases = [{ x: 4, y: 4 }, { x: 1, y: -1 }, { x: 8, y: 24 }, { x: 4, y: 5 }];
  starts.forEach((start, index) => {
    const release = releases[index];
    const geometry = resolveDragGeometry(start, release);
    const corners = previewOutline(start, release, identity);
    const centre = { x: (corners[0].x + corners[2].x) / 2, y: (corners[0].y + corners[2].y) / 2 };
    const back = (start.parentTransforms ?? []).reduce((local, transform) => canvasPointToModel(transform, local.x, local.y), centre);
    void back;
    const parentCentre = (start.parentTransforms ?? []).length ? (start.parentTransforms ?? []).reduceRight((point, transform) => ({ x: transform.a * point.x + transform.c * point.y + transform.e, y: transform.b * point.x + transform.d * point.y + transform.f }), { x: geometry.x, y: geometry.y }) : { x: geometry.x, y: geometry.y };
    expect(centre.x).toBeCloseTo(parentCentre.x, 10); expect(centre.y).toBeCloseTo(parentCentre.y, 10);
    const edgeA = Math.hypot(corners[1].x - corners[0].x, corners[1].y - corners[0].y);
    const edgeB = Math.hypot(corners[3].x - corners[0].x, corners[3].y - corners[0].y);
    const sorted = [edgeA, edgeB].sort((left, right) => left - right);
    const expected = [Math.min(geometry.width, geometry.height), Math.max(geometry.width, geometry.height)];
    const parentScale = (start.parentTransforms ?? []).length ? 2 : 1;
    expect(sorted[0]).toBeCloseTo(expected[0] * parentScale, 8); expect(sorted[1]).toBeCloseTo(expected[1] * parentScale, 8);
  });
});
test('preview outline matches page content at zoom 2 through the device transform', () => {
  const paintTransform = { a: 96, b: 0, c: 0, d: -96, e: 0, f: 720 };
  const start = { canvas: { x: 0, y: 0 }, model: { x: 4, y: 3 }, resize: false, pin: { x: 5, y: 2 }, size: { width: 2, height: 1 } };
  const release = { x: 4, y: 3 };
  const geometry = resolveDragGeometry(start, release);
  const corners = previewOutline(start, release, paintTransform);
  const rescaled = (previewOutline as unknown as (start: unknown, release: ModelPoint, paintTransform: unknown, scale: number) => ModelPoint[])(start, release, paintTransform, 2);
  expect(rescaled).toEqual(corners);
  const centre = { x: (corners[0].x + corners[2].x) / 2, y: (corners[0].y + corners[2].y) / 2 };
  const page = modelPointToCanvas(paintTransform, geometry.x, geometry.y);
  expect(centre.x).toBeCloseTo(page.x, 10); expect(centre.y).toBeCloseTo(page.y, 10);
  expect({ x: centre.x * 2, y: centre.y * 2 }).toEqual({ x: 960, y: 1056 });
});
test('paintDragPreview strokes a dashed brand outline and restores state', () => {
  const calls: string[] = [];
  const context = new Proxy({ canvas: {} }, {
    get(target, key) {
      if (key in target) return Reflect.get(target, key);
      return (...args: unknown[]) => { calls.push(`${String(key)}:${args.join(',')}`); };
    },
    set(target, key, value) { calls.push(`${String(key)}=${String(value)}`); Reflect.set(target, key, value); return true; },
  }) as unknown as CanvasRenderingContext2D;
  paintDragPreview(context, [{ x: 1, y: 2 }, { x: 3, y: 2 }, { x: 3, y: 4 }, { x: 1, y: 4 }], 2, 1);
  expect(calls).toContain('setTransform:2,0,0,2,0,0');
  expect(calls).toContain('strokeStyle=#0f6cbd');
  expect(calls).toContain('lineWidth=1');
  expect(calls.some((entry) => entry.startsWith('setLineDash:'))).toBe(true);
  expect(calls.some((entry) => entry.startsWith('stroke:'))).toBe(true);
  expect(calls[calls.length - 1].startsWith('restore:')).toBe(true);
});

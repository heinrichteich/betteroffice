import { afterEach, beforeAll, expect, test } from 'bun:test';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { GlobalRegistrator } from '@happy-dom/global-registrator';
import type { DiagramHandle, ModelPoint } from '@betteroffice/vsdx';

if (!GlobalRegistrator.isRegistered) GlobalRegistrator.register();

const root = resolve(import.meta.dir, '../../..');
let nestedGroups: Uint8Array;

beforeAll(async () => {
  const [wasm, fixture] = await Promise.all([
    readFile(resolve(import.meta.dir, '../../vsdx/src/wasm/generated/vsdx_wasm_bg.wasm')),
    readFile(resolve(root, 'crates/vsdx-parse/tests/fixtures/nested-groups.vsdx')),
  ]);
  const vsdx = await import('@betteroffice/vsdx');
  await vsdx.initWasm(wasm);
  nestedGroups = fixture;
});

const { act, cleanup, fireEvent, render, waitFor } = await import('@testing-library/react');
const { VsdxEditor } = await import('./VsdxEditor');
const { findShapePlacement } = await import('./components/ribbon/commands');
const { placementConnectionPoints } = await import('./VsdxEditor');
const { standardShapeById } = await import('./components/shapes/shapeLibrary');
const { connectorRouteFromFrame, modelToPage } = await import('./connector');

afterEach(() => { cleanup(); });

interface Ready { handle: DiagramHandle; refresh: () => void; }

const NESTED_ID = 'page:1:shape:1:shape:2:shape:3';
const GROUP_ID = 'page:1:shape:1';

function stubCanvasRect(canvas: HTMLCanvasElement, width: number, height: number): void {
  canvas.getBoundingClientRect = () => ({ x: 0, y: 0, left: 0, top: 0, right: width, bottom: height, width, height, toJSON: () => {} }) as DOMRect;
}

function stubCanvas(): () => void {
  const canvasPrototype = Object.getPrototypeOf(document.createElement('canvas')) as HTMLCanvasElement;
  const getContext = canvasPrototype.getContext;
  canvasPrototype.getContext = () => new Proxy({}, { get: () => () => {}, set: () => true }) as never;
  return () => { canvasPrototype.getContext = getContext; };
}

function cssForModel(frame: { width: number; height: number; paintTransform: { a: number; b: number; c: number; d: number; e: number; f: number } }, model: ModelPoint, scale: number): { x: number; y: number } {
  const page = modelToPage(frame as never, model);
  return { x: page.x * scale, y: page.y * scale };
}

function nestedEast(handle: DiagramHandle): ModelPoint {
  const page = handle.snapshot().pages[0];
  const placement = findShapePlacement(page.shapes, NESTED_ID)!;
  return placementConnectionPoints(page.shapes, placement)!.find((point) => point.side === 'east')!;
}

function nestedCentre(handle: DiagramHandle): ModelPoint {
  const page = handle.snapshot().pages[0];
  const placement = findShapePlacement(page.shapes, NESTED_ID)!;
  return placementConnectionPoints(page.shapes, placement)!.find((point) => point.side === 'centre')!;
}

async function dragPath(canvas: HTMLCanvasElement, from: { x: number; y: number }, to: { x: number; y: number }): Promise<void> {
  fireEvent.pointerDown(canvas, { clientX: from.x, clientY: from.y, pointerId: 1, buttons: 1, bubbles: true });
  const steps = 14;
  for (let index = 1; index <= steps; index++) {
    fireEvent.pointerMove(canvas, {
      clientX: from.x + ((to.x - from.x) * index) / steps,
      clientY: from.y + ((to.y - from.y) * index) / steps,
      pointerId: 1,
      buttons: 1,
      bubbles: true,
    });
  }
  fireEvent.pointerUp(canvas, { clientX: to.x, clientY: to.y, pointerId: 1, bubbles: true });
}

test('a nested shape offers its connection points on hover', async () => {
  const restore = stubCanvas();
  try {
    let ready: Ready | undefined;
    render(<VsdxEditor file={nestedGroups} fonts={[]} onReady={(api) => { ready = api; }} />);
    await waitFor(() => expect(ready).toBeDefined());
    const frame = ready!.handle.layoutPage(0);
    const canvas = document.querySelectorAll('canvas')[0] as HTMLCanvasElement;
    stubCanvasRect(canvas, frame.width, frame.height);
    const east = nestedEast(ready!.handle);
    await act(async () => {
      fireEvent.pointerMove(canvas, { clientX: cssForModel(frame, east, 1).x, clientY: cssForModel(frame, east, 1).y, pointerId: 1, bubbles: true });
    });
    expect(canvas.style.cursor).toBe('crosshair');
  } finally {
    cleanup();
    restore();
  }
});

test('dragging out of a nested hover point glues a connector that follows its group', async () => {
  const restore = stubCanvas();
  try {
    let ready: Ready | undefined;
    render(<VsdxEditor file={nestedGroups} fonts={[]} onReady={(api) => { ready = api; }} />);
    await waitFor(() => expect(ready).toBeDefined());
    const rectangle = standardShapeById('rectangle')!;
    let rectId = '';
    await act(async () => {
      rectId = ready!.handle.addShape('page:1', rectangle.draft(20, 10, 1, 1)).shapeId;
      ready!.refresh();
    });
    const frame = ready!.handle.layoutPage(0);
    const canvas = document.querySelectorAll('canvas')[0] as HTMLCanvasElement;
    stubCanvasRect(canvas, frame.width, frame.height);
    const east = nestedEast(ready!.handle);
    const page = ready!.handle.snapshot().pages[0];
    const rect = page.shapes.find((shape) => shape.id === rectId)!;
    const { connectionPointsForShape } = await import('./connector');
    const west = connectionPointsForShape(rect).find((point) => point.side === 'west')!;
    const before = page.shapes.length;
    await act(async () => { await dragPath(canvas, cssForModel(frame, east, 1), cssForModel(frame, west, 1)); });
    await waitFor(() => expect(ready!.handle.snapshot().pages[0].shapes.length).toBe(before + 1));
    const live = ready!.handle.snapshot().pages[0];
    const connector = live.shapes[live.shapes.length - 1];
    expect(connector.name).toBe('Dynamic connector');
    const route = connectorRouteFromFrame(ready!.handle.layoutPage(0), live.sourcePartPath, connector.sourceId)!;
    const centre = nestedCentre(ready!.handle);
    expect(route[0].x).toBeCloseTo(centre.x, 6);
    expect(route[0].y).toBeCloseTo(centre.y, 6);
    expect(route[route.length - 1]).toEqual({ x: west.x, y: west.y });
    await act(async () => { ready!.handle.moveShape('page:1', GROUP_ID, '12', '12'); ready!.refresh(); });
    const followed = connectorRouteFromFrame(ready!.handle.layoutPage(0), live.sourcePartPath, connector.sourceId)!;
    expect(followed[0].x).toBeCloseTo(route[0].x + 2, 6);
    expect(followed[0].y).toBeCloseTo(route[0].y + 2, 6);
    expect(followed[followed.length - 1]).toEqual({ x: west.x, y: west.y });
  } finally {
    cleanup();
    restore();
  }
});

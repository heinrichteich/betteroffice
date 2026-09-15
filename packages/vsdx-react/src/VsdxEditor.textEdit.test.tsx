import { afterEach, beforeAll, expect, test } from 'bun:test';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { GlobalRegistrator } from '@happy-dom/global-registrator';
import * as vsdx from '@betteroffice/vsdx';
import type { DiagramHandle } from '@betteroffice/vsdx';

if (!GlobalRegistrator.isRegistered) GlobalRegistrator.register();

const { act, cleanup, fireEvent, render, waitFor } = await import('@testing-library/react');
const { VsdxEditor } = await import('./VsdxEditor');
const root = resolve(import.meta.dir, '../../..');

let fixture: Uint8Array;

beforeAll(async () => {
  await vsdx.initWasm(await readFile(resolve(root, 'packages/vsdx/src/wasm/generated/vsdx_wasm_bg.wasm')));
  fixture = await readFile(resolve(root, 'apps/demo/public/betteroffice-demo.vsdx'));
});

afterEach(() => { cleanup(); });

test('a refused text edit keeps the draft open for correction', async () => {
  const canvasPrototype = Object.getPrototypeOf(document.createElement('canvas')) as HTMLCanvasElement;
  const getContext = canvasPrototype.getContext;
  canvasPrototype.getContext = () => new Proxy({}, { get: () => () => {}, set: () => true }) as never;
  const errors: Error[] = [];
  let ready: { handle: DiagramHandle; refresh: () => void } | undefined;
  const view = render(<VsdxEditor file={fixture} fonts={[]} onReady={(api) => { ready = api; }} onError={(error) => { errors.push(error); }} />);
  try {
    await waitFor(() => expect(ready).toBeDefined());
    const handle = ready!.handle;
    handle.setShapeText = (() => { throw new Error('text locked'); }) as DiagramHandle['setShapeText'];
    await act(async () => { ready!.refresh(); });
    const main = view.container.querySelectorAll('canvas')[0] as HTMLCanvasElement;
    main.getBoundingClientRect = (() => ({ left: 0, top: 0, width: 960, height: 720, right: 960, bottom: 720, x: 0, y: 0, toJSON: () => ({}) })) as unknown as typeof main.getBoundingClientRect;
    (main as unknown as { setPointerCapture: (id: number) => void }).setPointerCapture = () => {};
    fireEvent.doubleClick(main, { clientX: 480, clientY: 561.6 });
    await waitFor(() => expect(view.container.querySelector('textarea')).not.toBeNull());
    const box = view.container.querySelector('textarea') as HTMLTextAreaElement;
    expect(box.value).toBe('RUST ENGINE');
    fireEvent.change(box, { target: { value: 'rejected draft' } });
    fireEvent.pointerDown(document.body, { clientX: 10, clientY: 10 });
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 20)); });
    const retained = view.container.querySelector('textarea') as HTMLTextAreaElement | null;
    expect(retained).not.toBeNull();
    expect(retained!.value).toBe('rejected draft');
    expect(errors.map((error) => error.message)).toContain('text locked');
    expect(handle.shapeText('page:1', 'page:1:shape:20')).toBe('RUST ENGINE');
  } finally {
    cleanup();
    canvasPrototype.getContext = getContext;
  }
});

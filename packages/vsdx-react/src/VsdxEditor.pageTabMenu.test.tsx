import { afterEach, beforeAll, expect, test } from 'bun:test';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { GlobalRegistrator } from '@happy-dom/global-registrator';
import * as vsdx from '@betteroffice/vsdx';
import type { DiagramHandle, DiagramSnapshot } from '@betteroffice/vsdx';
import { createT, en } from '@betteroffice/vsdx-i18n';
import { useState } from 'react';
import { PAGE_TAB_CONTEXT_ENTRIES, PageTabContextMenu } from './components/ribbon/PageTabContextMenu';
import { RibbonCommandsProvider } from './components/ribbon/commands';
import type { RibbonCommandId } from './components/ribbon/commands';

if (!GlobalRegistrator.isRegistered) GlobalRegistrator.register();

const root = resolve(import.meta.dir, '../../..');
let demo: Uint8Array;

beforeAll(async () => {
  const [wasm, fixture] = await Promise.all([
    readFile(resolve(import.meta.dir, '../../vsdx/src/wasm/generated/vsdx_wasm_bg.wasm')),
    readFile(resolve(root, 'apps/demo/public/betteroffice-demo.vsdx')),
  ]);
  await vsdx.initWasm(wasm);
  demo = fixture;
});

const { act, cleanup, fireEvent, render, waitFor } = await import('@testing-library/react');
const { VsdxEditor } = await import('./VsdxEditor');

interface Calls {
  undos: number;
  redos: number;
  added: unknown[][];
}

function stubHandle(calls: Calls, options: { canUndo: boolean; canRedo: boolean } = { canUndo: true, canRedo: true }): DiagramHandle {
  const state: DiagramSnapshot = { pages: [{ id: 'page', sourcePartPath: 'page', name: 'Page', shapes: [] }] };
  return {
    snapshot: () => state,
    canUndo: () => options.canUndo,
    canRedo: () => options.canRedo,
    undo: () => { calls.undos += 1; },
    redo: () => { calls.redos += 1; },
    addShape: (...args: [string, unknown]) => { calls.added.push([...args]); return { id: 'new', pageId: 'page' }; },
  } as unknown as DiagramHandle;
}

function Host({ diagram, position, closed, focusTarget }: { diagram: DiagramHandle; position: { top: number; left: number }; closed: string[]; focusTarget?: HTMLElement }) {
  const [open, setOpen] = useState(true);
  if (!open) return null;
  return (
    <RibbonCommandsProvider handle={diagram} snapshot={diagram.snapshot()} pageId="page" selection={null} onMutation={() => {}} onError={() => {}} onDownload={() => {}}>
      <PageTabContextMenu t={createT(en)} position={position} onClose={() => { closed.push('close'); setOpen(false); }} onCloseAndFocus={() => { closed.push('focus'); setOpen(false); focusTarget?.focus(); }} />
    </RibbonCommandsProvider>
  );
}

function renderMenu(options: { position?: { top: number; left: number }; withFocusTarget?: boolean; canUndo?: boolean; canRedo?: boolean } = {}) {
  cleanup();
  const calls: Calls = { undos: 0, redos: 0, added: [] };
  const diagram = stubHandle(calls, { canUndo: options.canUndo ?? true, canRedo: options.canRedo ?? true });
  const closed: string[] = [];
  let focusTarget: HTMLElement | undefined;
  if (options.withFocusTarget) {
    focusTarget = document.createElement('button');
    document.body.appendChild(focusTarget);
  }
  const view = render(<Host diagram={diagram} position={options.position ?? { top: 100, left: 100 }} closed={closed} focusTarget={focusTarget} />);
  return { view, calls, closed, focusTarget };
}

function pageTabMenu(): HTMLElement {
  const menus = Array.from(document.querySelectorAll('[role="menu"]')) as HTMLElement[];
  const found = menus.find((menu) => menu.getAttribute('aria-label') === en.contextMenu.pageTabLabel);
  expect(found).not.toBeUndefined();
  return found as HTMLElement;
}

afterEach(() => {
  cleanup();
  document.body.innerHTML = '';
});

test('the page-tab menu offers only existing ribbon commands with a single divider', () => {
  const { view } = renderMenu();
  try {
    expect(PAGE_TAB_CONTEXT_ENTRIES.map((entry) => entry.id)).toEqual(['undo', 'redo', 'addShape']);
    for (const entry of PAGE_TAB_CONTEXT_ENTRIES) expect(entry.children ?? []).toEqual([]);
    const menu = pageTabMenu();
    expect(menu.getAttribute('aria-label')).toBe(en.contextMenu.pageTabLabel);
    expect(en.contextMenu.pageTabLabel).not.toBe(en.contextMenu.label);
    expect(en.contextMenu.pageTabLabel).not.toBe(en.contextMenu.canvasLabel);
    expect(Array.from(menu.querySelectorAll('[data-command-id]')).map((button) => button.getAttribute('data-command-id'))).toEqual(['undo', 'redo', 'addShape']);
    expect(menu.querySelectorAll('[role="separator"]')).toHaveLength(1);
    const labels: Record<RibbonCommandId, string> = en.ribbon.commands as Record<RibbonCommandId, string>;
    for (const entry of PAGE_TAB_CONTEXT_ENTRIES) expect(labels[entry.id]).toBeDefined();
  } finally {
    view.unmount();
  }
});

test('each entry runs its command and closes the menu', () => {
  for (const id of ['undo', 'redo', 'addShape'] as const) {
    const { view, calls, closed } = renderMenu();
    try {
      const item = document.querySelector(`[role="menu"] [data-command-id="${id}"]`) as HTMLElement;
      expect(item).not.toBeNull();
      fireEvent.click(item);
      if (id === 'undo') expect(calls.undos).toBe(1);
      if (id === 'redo') expect(calls.redos).toBe(1);
      if (id === 'addShape') expect(calls.added[0]?.[0]).toBe('page');
      expect(document.querySelector('[role="menu"]')).toBeNull();
      expect(closed[0]).toBe('focus');
    } finally {
      view.unmount();
    }
  }
});

test('a tab right-click selects that page and offers the page-tab menu', async () => {
  const canvasPrototype = Object.getPrototypeOf(document.createElement('canvas')) as HTMLCanvasElement;
  const getContext = canvasPrototype.getContext;
  canvasPrototype.getContext = () => new Proxy({}, { get: () => () => {}, set: () => true }) as never;
  let ready: { handle: DiagramHandle; refresh: () => void } | undefined;
  const view = render(<VsdxEditor file={demo} fonts={[]} onReady={(api) => { ready = api; }} />);
  try {
    await waitFor(() => expect(ready).toBeDefined());
    const pageTabs = view.getByRole('tablist', { name: en.statusBar.pageTabs });
    const tabs = pageTabs.querySelectorAll('[role="tab"]');
    expect(tabs.length).toBeGreaterThan(1);
    expect(tabs[0].getAttribute('aria-selected')).toBe('true');
    const target = tabs[1] as HTMLElement;
    expect(fireEvent.contextMenu(target, { clientX: 200, clientY: 700, button: 2 }) === false).toBe(true);
    await act(async () => {});
    expect(target.getAttribute('aria-selected')).toBe('true');
    const menu = pageTabMenu();
    expect(Array.from(menu.querySelectorAll('[data-command-id]')).map((button) => button.getAttribute('data-command-id'))).toEqual(['undo', 'redo', 'addShape']);
    const pageId = ready!.handle.snapshot().pages[1].id;
    const before = ready!.handle.snapshot().pages[1].shapes.length;
    fireEvent.click(menu.querySelector('[data-command-id="addShape"]') as HTMLElement);
    await act(async () => {});
    expect(document.querySelector('[role="menu"]')).toBeNull();
    const after = ready!.handle.snapshot();
    expect(after.pages[1].id).toBe(pageId);
    expect(after.pages[1].shapes.length).toBe(before + 1);
  } finally {
    cleanup();
    canvasPrototype.getContext = getContext;
  }
});

test('escape closes the page-tab menu and returns focus to the canvas', async () => {
  const canvasPrototype = Object.getPrototypeOf(document.createElement('canvas')) as HTMLCanvasElement;
  const getContext = canvasPrototype.getContext;
  canvasPrototype.getContext = () => new Proxy({}, { get: () => () => {}, set: () => true }) as never;
  let ready: { handle: DiagramHandle; refresh: () => void } | undefined;
  const view = render(<VsdxEditor file={demo} fonts={[]} onReady={(api) => { ready = api; }} />);
  try {
    await waitFor(() => expect(ready).toBeDefined());
    const main = view.container.querySelectorAll('canvas')[0] as HTMLCanvasElement;
    const firstTab = view.getByRole('tablist', { name: en.statusBar.pageTabs }).querySelector('[role="tab"]') as HTMLElement;
    expect(fireEvent.contextMenu(firstTab, { clientX: 120, clientY: 700, button: 2 }) === false).toBe(true);
    await act(async () => {});
    expect(pageTabMenu()).not.toBeNull();
    fireEvent.keyDown(document.activeElement as HTMLElement, { key: 'Escape' });
    expect(document.querySelector('[role="menu"]')).toBeNull();
    expect(document.activeElement).toBe(main);
  } finally {
    cleanup();
    canvasPrototype.getContext = getContext;
  }
});

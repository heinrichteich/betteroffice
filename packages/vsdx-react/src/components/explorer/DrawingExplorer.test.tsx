import { expect, test } from 'bun:test';
import { GlobalRegistrator } from '@happy-dom/global-registrator';
import { createT, en } from '@betteroffice/vsdx-i18n';
import type { CellSnapshot, DiagramSnapshot, ShapeSnapshot } from '@betteroffice/vsdx';

if (!GlobalRegistrator.isRegistered) GlobalRegistrator.register();

const { cleanup, fireEvent, render } = await import('@testing-library/react');
const { DrawingExplorer, MAX_EXPLORER_DEPTH, groupSectionRows, groupShapeCells, isOneDShape, shapeLabel } = await import('./DrawingExplorer');
const t = createT(en);

function cell(name: string, formula: string | null, value: string | null, section: string | null = null, row: { index: number } | { name: string } | null = null, rowType?: string): CellSnapshot {
  return {
    locator: { sheet: { page: 1 }, shapeId: 5, section, sectionIndex: null, row, cellName: name },
    name,
    formula,
    value,
    ...(rowType ? { rowType } : {}),
  };
}

function shape(id: string, sourceId: number, name: string | null, cells: CellSnapshot[] = [], children: ShapeSnapshot[] = []): ShapeSnapshot {
  return { id, sourceId, name, cells, children };
}

function snapshot(): DiagramSnapshot {
  const client = shape('page:1:shape:6', 6, 'client', [cell('PinX', '1', '1'), cell('Width', '2', '2')]);
  const category = shape('page:1:shape:7', 7, 'category', [cell('PinX', '3', '3')]);
  const group = shape('page:1:shape:5', 5, 'Purchase', [cell('Width', '4', '4')], [client, category]);
  const connector = shape('page:1:shape:12', 12, 'Connector', [
    cell('OneD', '1', '1'),
    cell('BeginX', '1', '1'),
    cell('BeginY', '1', '1'),
    cell('EndX', '4', '4'),
    cell('EndY', '4', '4'),
    cell('X', 'WIDTH*0.5', '2', 'Geometry', { index: 0 }, 'MoveTo'),
    cell('Y', 'HEIGHT*0.5', '1', 'Geometry', { index: 0 }, 'MoveTo'),
  ]);
  return {
    pages: [
      { id: 'page:1', sourcePartPath: 'visio/pages/page1.xml', name: 'Page-1', shapes: [group, connector] },
      { id: 'page:2', sourcePartPath: 'visio/pages/page2.xml', name: null, shapes: [] },
    ],
  };
}

function renderExplorer(overrides: Record<string, unknown> = {}) {
  const selected: Array<{ pageId: string; shapeId: string }> = [];
  const pages: number[] = [];
  const view = render(
    <DrawingExplorer
      snapshot={snapshot()}
      activePageId="page:1"
      selection={null}
      onSelectPage={(index: number) => pages.push(index)}
      onSelectShape={(pageId: string, shapeId: string) => selected.push({ pageId, shapeId })}
      t={t}
      {...overrides}
    />,
  );
  return { ...view, selected, pages };
}

function toggle(view: { getByRole: (role: string, options?: object) => HTMLElement }, name: string) {
  fireEvent.click(view.getByRole('button', { name } as never) as HTMLElement);
}

function selectShape(view: { getAllByRole: (role: string) => HTMLElement[] }, label: string) {
  const match = view.getAllByRole('button').find((element) => element.textContent?.startsWith(label));
  if (!match) throw new Error(`missing shape button ${label}`);
  fireEvent.click(match);
}

test('labels shapes with their real ids and names', () => {
  expect(shapeLabel(shape('page:1:shape:5', 5, 'Purchase'))).toBe('Shape 5 "Purchase"');
  expect(shapeLabel(shape('page:1:shape:9', 9, null))).toBe('Shape 9');
});

test('marks groups by containment and connectors by their resolved OneD cell', () => {
  const data = snapshot();
  expect(data.pages[0].shapes[0].children).toHaveLength(2);
  expect(isOneDShape(data.pages[0].shapes[0])).toBe(false);
  expect(isOneDShape(data.pages[0].shapes[1])).toBe(true);
});

test('groups cells by section and rows by row identity', () => {
  const sections = groupShapeCells(snapshot().pages[0].shapes[1].cells);
  expect(sections.map((section) => section.name)).toEqual(['Shape', 'Geometry']);
  const geometry = sections.find((section) => section.name === 'Geometry')!;
  const rows = groupSectionRows(geometry.cells);
  expect(rows).toHaveLength(1);
  expect(rows[0].label).toBe('Row 1 (MoveTo)');
  expect(rows[0].cells.map((entry) => entry.name)).toEqual(['X', 'Y']);
});

test('nests group children by containment with kind badges', () => {
  const view = renderExplorer();
  for (const label of ['Shape 5 "Purchase"', 'Shape 6 "client"', 'Shape 7 "category"', 'Shape 12 "Connector"']) {
    expect(view.container.textContent).not.toContain(label);
  }
  toggle(view, 'Expand Page-1');
  toggle(view, 'Expand Shapes');
  toggle(view, 'Expand Shape 5 "Purchase"');
  for (const label of ['Shape 5 "Purchase"', 'Shape 6 "client"', 'Shape 7 "category"', 'Shape 12 "Connector"']) {
    expect(view.container.textContent).toContain(label);
  }
  expect(view.container.textContent).toContain('Group');
  expect(view.container.textContent).toContain('1-D');
  cleanup();
});

test('selecting a node reports the shape and the selection highlights it', () => {
  const view = renderExplorer();
  toggle(view, 'Expand Page-1');
  toggle(view, 'Expand Shapes');
  selectShape(view, 'Shape 12 "Connector"');
  expect(view.selected).toEqual([{ pageId: 'page:1', shapeId: 'page:1:shape:12' }]);
  cleanup();

  const selected = renderExplorer({ selection: { pageId: 'page:1', shapeId: 'page:1:shape:12', hit: { kind: 'shape', shapeId: 'page:1:shape:12' } } });
  const node = selected.getAllByRole('treeitem').find((element) => element.getAttribute('aria-label') === 'Shape 12 "Connector"')!;
  expect(node).toBeDefined();
  expect(node.getAttribute('aria-selected')).toBe('true');
  cleanup();
});

test('selecting a page node reports its index', () => {
  const view = renderExplorer();
  fireEvent.click(view.getByRole('button', { name: 'Page 2' } as never) as HTMLElement);
  expect(view.pages).toEqual([1]);
  cleanup();
});

test('renders cells lazily with formula and value side by side', () => {
  const view = renderExplorer();
  toggle(view, 'Expand Page-1');
  toggle(view, 'Expand Shapes');
  expect(view.queryByText('WIDTH*0.5')).toBeNull();
  toggle(view, 'Expand Shape 12 "Connector"');
  expect(view.queryByText('WIDTH*0.5')).toBeNull();
  toggle(view, 'Expand ShapeSheet');
  toggle(view, 'Expand Geometry');
  const formula = view.getByText('WIDTH*0.5');
  const row = formula.closest('tr')!;
  expect(row.textContent).toContain('X');
  expect(row.textContent).toContain('2');
  expect(view.getByText('Formula').tagName).toBe('TH');
  expect(view.getByText('Value').tagName).toBe('TH');
  cleanup();
});

test('bounds deeply nested documents with a truncation marker', () => {
  let nested: ShapeSnapshot = shape('page:1:shape:deep', 99, 'leaf');
  for (let depth = MAX_EXPLORER_DEPTH + 2; depth >= 0; depth -= 1) {
    nested = shape(`page:1:shape:${depth}`, depth, null, [], [nested]);
  }
  const deep: DiagramSnapshot = {
    pages: [{ id: 'page:1', sourcePartPath: 'visio/pages/page1.xml', name: 'Page-1', shapes: [nested] }],
  };
  const view = render(
    <DrawingExplorer snapshot={deep} activePageId="page:1" selection={null} onSelectPage={() => {}} onSelectShape={() => {}} t={t} />,
  );
  toggle(view, 'Expand Page-1');
  toggle(view, 'Expand Shapes');
  for (let depth = 0; depth < MAX_EXPLORER_DEPTH; depth += 1) {
    toggle(view, `Expand Shape ${depth}`);
  }
  expect(view.container.textContent).toContain('Deeper levels omitted');
  expect(view.container.querySelectorAll('[role="treeitem"]').length).toBeLessThan(40);
  cleanup();
});

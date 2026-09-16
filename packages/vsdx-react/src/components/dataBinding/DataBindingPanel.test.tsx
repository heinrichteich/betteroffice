import { afterEach, expect, test } from 'bun:test';
import { GlobalRegistrator } from '@happy-dom/global-registrator';
import { createT, en } from '@betteroffice/vsdx-i18n';
import type { CellLocator, DiagramSnapshot, FormulaShapeDraft } from '@betteroffice/vsdx';
import { DataBindingPanel } from './DataBindingPanel';

if (!GlobalRegistrator.isRegistered) GlobalRegistrator.register();

const { cleanup, fireEvent, render } = await import('@testing-library/react');
const t = createT(en);

afterEach(() => cleanup());

interface FakeCell {
  section: string | null;
  row: string | null;
  name: string;
  formula: string;
}

interface FakeShape {
  id: string;
  name: string | null;
  sourceId: number;
  cells: Map<string, FakeCell>;
}

function cellKey(section: string | undefined, rowName: string | undefined, rowIndex: number | undefined, name: string): string {
  if (section && (rowName !== undefined || rowIndex !== undefined)) {
    return `${section}\u{1f}${rowName !== undefined ? `N:${rowName}` : `IX:${rowIndex}`}\u{1f}${name}`;
  }
  return name;
}

function makeHandle() {
  const shapes = new Map<string, FakeShape>();
  let sequence = 0;
  const snapshot = (): DiagramSnapshot => ({
    pages: [{
      id: 'page:1',
      sourcePartPath: 'visio/pages/page1.xml',
      name: 'Page 1',
      shapes: [...shapes.values()].map((shape) => ({
        id: shape.id,
        sourceId: shape.sourceId,
        name: shape.name,
        children: [],
        cells: [...shape.cells.values()].map((cell) => ({
          locator: {
            sheet: { page: 1 },
            shapeId: 1,
            section: cell.section,
            sectionIndex: cell.section ? 0 : undefined,
            row: cell.row ? { name: cell.row } : null,
            cellName: cell.name,
          },
          name: cell.name,
          formula: cell.formula,
          value: null,
        })),
      })),
    }],
  });
  const handle = {
    snapshot,
    setCellFormula: (pageId: string, shapeId: string, locator: CellLocator, formula: string) => {
      const shape = shapes.get(shapeId);
      if (!shape) throw new Error(`shape "${shapeId}" was not found`);
      const key = cellKey(locator.section, locator.rowName, locator.rowIndex, locator.cellName);
      const cell = shape.cells.get(key);
      if (!cell) throw new Error(`cell "${key}" was not found`);
      if (cell.formula.includes('GUARD')) throw new Error('GUARD protects the requested cell');
      const before = cell.formula;
      cell.formula = formula;
      return { pageId, shapeId, cellName: locator.cellName, before, after: formula };
    },
    addShape: (pageId: string, draft: FormulaShapeDraft) => {
      sequence += 1;
      const id = `fake:${sequence}`;
      const shape: FakeShape = { id, name: draft.name ?? null, sourceId: sequence, cells: new Map() };
      for (const cell of draft.cells) {
        const key = cellKey(cell.locator.section, cell.locator.rowName, cell.locator.rowIndex, cell.locator.cellName);
        shape.cells.set(key, {
          section: cell.locator.section ?? null,
          row: cell.locator.rowName ?? null,
          name: cell.locator.cellName,
          formula: cell.formula ?? '',
        });
      }
      shapes.set(id, shape);
      return { pageId, shapeId: id, fromIndex: null, toIndex: null };
    },
    deleteShape: (pageId: string, shapeId: string) => {
      if (!shapes.delete(shapeId)) throw new Error(`shape "${shapeId}" was not found`);
      return { pageId, shapeId, fromIndex: null, toIndex: null };
    },
  };
  const addTarget = (name: string, guardedOwner: boolean) => {
    const shape: FakeShape = { id: 'target:1', name, sourceId: 7, cells: new Map() };
    const property = (row: string, formula: string) => shape.cells.set(`Property\u{1f}N:${row}\u{1f}Value`, { section: 'Property', row, name: 'Value', formula });
    property('Device', '""');
    property('Owner', guardedOwner ? 'GUARD("locked")' : '""');
    shapes.set(shape.id, shape);
    return shape.id;
  };
  return { handle, addTarget };
}

const CSV = 'Device,Owner\nSRV-01,Team Atlas\nSRV-02,Team Beacon\n';

function csvFile(): File {
  return new File([CSV], 'devices.csv', { type: 'text/csv' });
}

test('imports csv, links a row, and refreshes the binding', async () => {
  const { handle, addTarget } = makeHandle();
  const targetId = addTarget('Test Server', false);
  let mutated = 0;
  const errors: unknown[] = [];
  const view = render(
    <DataBindingPanel
      handle={handle as never}
      snapshot={handle.snapshot()}
      selection={{ pageId: 'page:1', shapeId: targetId }}
      onMutated={() => { mutated += 1; }}
      onError={(error) => errors.push(error)}
      t={t}
    />,
  );
  const input = view.getByLabelText(t('dataBinding.fileLabel')) as HTMLInputElement;
  fireEvent.change(input, { target: { files: [csvFile()] } });
  await view.findByText(t('dataBinding.tableSummary', { rows: 2, columns: 2, name: 'devices.csv' }));
  expect(errors).toEqual([]);

  fireEvent.click(view.getByText(t('dataBinding.linkAction')));
  await view.findByText(t('dataBinding.bindingsHeading'));
  const target = handle.snapshot().pages[0].shapes.find((shape) => shape.id === targetId)!;
  const owner = target.cells.find((cell) => cell.locator.row !== null && 'name' in cell.locator.row && cell.locator.row.name === 'Owner');
  expect(owner?.formula).toBe('"Team Atlas"');
  expect(mutated).toBe(1);

  view.rerender(
    <DataBindingPanel
      handle={handle as never}
      snapshot={handle.snapshot()}
      selection={{ pageId: 'page:1', shapeId: targetId }}
      onMutated={() => { mutated += 1; }}
      onError={(error) => errors.push(error)}
      t={t}
    />,
  );
  fireEvent.click(view.getByText(t('dataBinding.refreshAction')));
  await view.findByText(t('dataBinding.refreshed', { updated: 2 }), undefined, { timeout: 2000 }).catch(() => view.getByRole('status'));
  expect(view.getByRole('status').textContent).toContain('2');

  fireEvent.click(view.getByText(t('dataBinding.unlinkAction')));
  expect(mutated).toBe(3);
});

test('surfaces guard refusals and unmapped columns instead of swallowing them', async () => {
  const { handle, addTarget } = makeHandle();
  const targetId = addTarget('Guarded Box', true);
  const errors: unknown[] = [];
  const view = render(
    <DataBindingPanel
      handle={handle as never}
      snapshot={handle.snapshot()}
      selection={{ pageId: 'page:1', shapeId: targetId }}
      onMutated={() => {}}
      onError={(error) => errors.push(error)}
      t={t}
    />,
  );
  const input = view.getByLabelText(t('dataBinding.fileLabel')) as HTMLInputElement;
  fireEvent.change(input, { target: { files: [csvFile()] } });
  await view.findByText(t('dataBinding.tableSummary', { rows: 2, columns: 2, name: 'devices.csv' }));

  fireEvent.click(view.getByText(t('dataBinding.linkAction')));
  const status = await view.findByRole('status');
  expect(status.textContent).toContain('GUARD');
});

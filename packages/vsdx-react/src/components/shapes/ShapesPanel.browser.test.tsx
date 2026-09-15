import { afterEach, expect, test } from 'bun:test';
import { GlobalRegistrator } from '@happy-dom/global-registrator';
import { createT, en } from '@betteroffice/vsdx-i18n';
import { shapeStencils, standardShapes } from './shapeLibrary';
import type { ShapeStencil, StencilCategory } from './shapeLibrary';
import { ShapesPanel } from './ShapesPanel';

if (!GlobalRegistrator.isRegistered) GlobalRegistrator.register();

const { cleanup, fireEvent, render } = await import('@testing-library/react');
const t = createT(en);

afterEach(() => cleanup());

const railStencils = shapeStencils.filter((stencil) => stencil.id !== 'callouts');

function browser(overrides: { stencils?: readonly ShapeStencil[]; catalogue?: readonly StencilCategory[] } = {}) {
  const added: string[] = [];
  const selected: string[] = [];
  const view = render(<ShapesPanel stencils={overrides.stencils ?? railStencils} catalogue={overrides.catalogue} activeStencilId="standard" onSelectStencil={(id) => selected.push(id)} onAddStencil={(id) => added.push(id)} collapsed={false} onToggleCollapsed={() => {}} onInsert={() => {}} t={t} />);
  fireEvent.click(view.getByRole('button', { name: t('shapesPanel.addShapes') }));
  return { ...view, added, selected };
}

function rows(view: ReturnType<typeof browser>) {
  return view.container.querySelectorAll('section[aria-label="Add Shapes"] li');
}

test('opens the browser grouped under category headers', () => {
  const view = browser();
  expect(view.getByRole('heading', { name: t('shapesPanel.categoryBasicDiagram') })).toBeDefined();
  expect(view.getByText(t('shapesPanel.standardShapes'))).toBeDefined();
  expect(view.getByText(t('shapesPanel.arrowShapes'))).toBeDefined();
  expect(view.getByText(t('shapesPanel.calloutShapes'))).toBeDefined();
  expect(view.queryByRole('grid')).toBeNull();
});

test('skips catalogue entries without content', () => {
  const empty: ShapeStencil = { id: 'empty', nameKey: 'shapesPanel.arrowShapes', shapes: [] };
  const catalogue: readonly StencilCategory[] = [{ id: 'basicDiagram', nameKey: 'shapesPanel.categoryBasicDiagram', stencilIds: ['standard', 'missing', 'empty'] }];
  const view = browser({ stencils: [...railStencils, empty], catalogue });
  expect(rows(view)).toHaveLength(1);
  expect(view.getByText(t('shapesPanel.standardShapes'))).toBeDefined();
});

test('filters across stencil and shape names', () => {
  const view = browser();
  const search = view.getByRole('searchbox', { name: t('shapesPanel.searchLabel') });
  fireEvent.change(search, { target: { value: 'oval' } });
  expect(rows(view)).toHaveLength(1);
  expect(view.getByText(t('shapesPanel.calloutShapes'))).toBeDefined();
  fireEvent.change(search, { target: { value: 'hexagon' } });
  expect(rows(view)).toHaveLength(1);
  expect(view.getByText(t('shapesPanel.standardShapes'))).toBeDefined();
  fireEvent.change(search, { target: { value: 'not a stencil' } });
  expect(rows(view)).toHaveLength(0);
  expect(view.getByText(t('shapesPanel.browserEmpty'))).toBeDefined();
});

test('Add reports the stencil and returns to its gallery', () => {
  const view = browser();
  fireEvent.click(view.getByRole('button', { name: `${t('shapesPanel.addStencil')}: ${t('shapesPanel.calloutShapes')}` }));
  expect(view.added).toEqual(['callouts']);
  expect(view.selected).toEqual(['callouts']);
  expect(view.queryByText(t('shapesPanel.browserEmpty'))).toBeNull();
  expect(view.getByRole('grid', { name: t('shapesPanel.standardShapes') })).toBeDefined();
});

test('marks stencils already in the rail as added', () => {
  const view = browser({ stencils: shapeStencils });
  expect(view.queryByRole('button', { name: /Add:/ })).toBeNull();
  expect(view.getAllByText(`✓ ${t('shapesPanel.addedStencil')}`)).toHaveLength(3);
});

test('selecting a rail stencil closes the browser', () => {
  const view = browser();
  fireEvent.click(view.getByRole('button', { name: t('shapesPanel.arrowShapes') }));
  expect(view.selected).toEqual(['arrows']);
  expect(view.getAllByRole('gridcell')).toHaveLength(standardShapes.length);
});

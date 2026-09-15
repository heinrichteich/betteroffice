import { useEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties, KeyboardEvent, ReactElement } from 'react';
import type { TFunction } from '@betteroffice/vsdx-i18n';
import type { ShapeStencil, StandardShape, StencilCategory } from './shapeLibrary';
import { shapeStencils, stencilCatalogue } from './shapeLibrary';

export interface ShapesPanelProps {
  shapes?: readonly StandardShape[];
  stencils?: readonly ShapeStencil[];
  catalogue?: readonly StencilCategory[];
  activeStencilId?: string;
  onSelectStencil?: (id: string) => void;
  onAddStencil?: (id: string) => void;
  collapsed: boolean;
  onToggleCollapsed: () => void;
  onInsert: (shape: StandardShape) => void;
  t: TFunction;
  className?: string;
}

const styles: Record<string, CSSProperties> = {
  root: { display: 'flex', minWidth: 0, height: '100%', background: '#fff', color: '#242424', font: '400 13px ui-sans-serif, system-ui, sans-serif', borderRight: '1px solid #e0e0e0', boxSizing: 'border-box' },
  rail: { display: 'flex', flexDirection: 'column', alignItems: 'center', width: 44, padding: '8px 6px', background: '#f7f7f7', borderRight: '1px solid #e5e5e5', boxSizing: 'border-box' },
  railButton: { appearance: 'none', display: 'grid', placeItems: 'center', width: 30, height: 30, padding: 0, border: 0, borderRadius: 4, background: '#dbeafe', color: '#0f6cbd', cursor: 'pointer' },
  railButtonInactive: { appearance: 'none', display: 'grid', placeItems: 'center', width: 30, height: 30, padding: 0, border: 0, borderRadius: 4, background: 'transparent', color: '#424242', cursor: 'pointer' },
  content: { display: 'flex', flexDirection: 'column', minWidth: 220, width: 280, height: '100%' },
  header: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', minHeight: 44, padding: '0 10px 0 14px', borderBottom: '1px solid #e5e5e5', fontWeight: 600, fontSize: 14 },
  toggle: { appearance: 'none', display: 'grid', placeItems: 'center', width: 28, height: 28, padding: 0, border: 0, borderRadius: 4, background: 'transparent', color: '#424242', cursor: 'pointer' },
  searchWrap: { position: 'relative', margin: 12, width: 'calc(100% - 24px)', boxSizing: 'border-box' },
  search: { width: '100%', height: 32, padding: '0 30px 0 9px', border: '1px solid #bdbdbd', borderRadius: 3, outline: 0, color: '#242424', font: '400 13px ui-sans-serif, system-ui, sans-serif', boxSizing: 'border-box' },
  searchButton: { appearance: 'none', position: 'absolute', top: 0, right: 0, display: 'grid', placeItems: 'center', width: 30, height: 32, padding: 0, border: 0, borderRadius: '0 3px 3px 0', background: 'transparent', color: '#616161', cursor: 'pointer' },
  heading: { margin: '3px 12px 10px', color: '#424242', fontWeight: 600, fontSize: 12 },
  browserHeading: { margin: '3px 2px 4px', color: '#424242', fontWeight: 600, fontSize: 12 },
  grid: { display: 'flex', flexDirection: 'column', gap: 5, padding: '0 10px 12px', overflowY: 'auto' },
  row: { display: 'grid', gridTemplateColumns: 'repeat(3, minmax(0, 1fr))', gap: 5 },
  cell: { display: 'flex', minWidth: 0 },
  tile: { appearance: 'none', flex: 1, display: 'flex', flexDirection: 'column', alignItems: 'center', minWidth: 0, minHeight: 92, padding: '7px 3px 5px', border: '1px solid transparent', borderRadius: 3, background: 'transparent', color: '#242424', cursor: 'pointer', font: '400 11px ui-sans-serif, system-ui, sans-serif', textAlign: 'center' },
  preview: { width: 54, height: 46, marginBottom: 5, overflow: 'visible', fill: '#fff', stroke: '#424242', strokeWidth: 0.03 },
  label: { display: 'block', width: '100%', maxWidth: '100%', overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis' },
  empty: { margin: '20px 12px', color: '#616161', textAlign: 'center' },
  addRailButton: { appearance: 'none', display: 'grid', placeItems: 'center', width: 30, height: 30, padding: 0, border: 0, borderRadius: '50%', background: '#0f6cbd', color: '#fff', cursor: 'pointer' },
  browserList: { display: 'flex', flexDirection: 'column', gap: 12, padding: '0 10px 12px', overflowY: 'auto' },
  browserRow: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8, minWidth: 0, padding: '8px 10px', border: '1px solid #e5e5e5', borderRadius: 4, background: '#fafafa' },
  browserName: { display: 'flex', flexDirection: 'column', minWidth: 0 },
  browserTitle: { overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis', fontWeight: 600, fontSize: 12, color: '#242424' },
  browserCount: { fontSize: 11, color: '#616161' },
  addButton: { appearance: 'none', flexShrink: 0, minWidth: 52, padding: '5px 12px', border: '1px solid #0f6cbd', borderRadius: 12, background: '#fff', color: '#0f6cbd', cursor: 'pointer', font: '600 12px ui-sans-serif, system-ui, sans-serif' },
  addedBadge: { appearance: 'none', flexShrink: 0, minWidth: 52, padding: '5px 12px', border: '1px solid transparent', borderRadius: 12, background: 'transparent', color: '#616161', font: '600 12px ui-sans-serif, system-ui, sans-serif' },
};

const COLUMNS = 3;

function nextFocusIndex(key: string, count: number, index: number): number {
  if (key === 'ArrowRight') return (index + 1) % count;
  if (key === 'ArrowLeft') return (index - 1 + count) % count;
  if (key === 'ArrowDown') return (index + COLUMNS) % count;
  if (key === 'ArrowUp') return (index - COLUMNS + count) % count;
  if (key === 'Home') return 0;
  if (key === 'End') return count - 1;
  return index;
}

function stencilIcon(id: string): ReactElement {
  if (id === 'arrows') return <svg aria-hidden="true" width="18" height="18" viewBox="0 0 18 18"><path d="M 2 9 L 12 9" fill="none" stroke="currentColor" strokeWidth="1.5" /><path d="M 9 4.5 L 14 9 L 9 13.5" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /></svg>;
  if (id === 'callouts') return <svg aria-hidden="true" width="18" height="18" viewBox="0 0 18 18"><path d="M 3 3 L 13 3 L 13 10 L 8 10 L 5.5 13 L 5.5 10 L 3 10 Z" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round" /></svg>;
  return <svg aria-hidden="true" width="18" height="18" viewBox="0 0 18 18"><rect x="3" y="3" width="5" height="5" fill="none" stroke="currentColor" /><circle cx="13" cy="5.5" r="2.5" fill="none" stroke="currentColor" /><path d="M 3 14 L 6 10 L 9 14 Z" fill="none" stroke="currentColor" /></svg>;
}

export function ShapesPanel({ shapes, stencils, catalogue, activeStencilId, onSelectStencil, onAddStencil, collapsed, onToggleCollapsed, onInsert, t, className }: ShapesPanelProps) {
  const [query, setQuery] = useState('');
  const [browserQuery, setBrowserQuery] = useState('');
  const [browserOpen, setBrowserOpen] = useState(false);
  const [focusIndex, setFocusIndex] = useState(0);
  const tileRefs = useRef(new Map<number, HTMLButtonElement>());
  const searchRef = useRef<HTMLInputElement>(null);
  const resolvedStencils = useMemo<readonly ShapeStencil[]>(() => {
    if (stencils && stencils.length > 0) return stencils;
    return [{ id: 'standard', nameKey: 'shapesPanel.standardShapes', shapes: shapes ?? [] }];
  }, [stencils, shapes]);
  const activeStencil = resolvedStencils.find((stencil) => stencil.id === activeStencilId) ?? resolvedStencils[0];
  const previousStencilId = useRef(activeStencil.id);
  useEffect(() => {
    if (previousStencilId.current === activeStencil.id) return;
    previousStencilId.current = activeStencil.id;
    setQuery('');
    setFocusIndex(0);
  }, [activeStencil.id]);
  const selectStencil = (id: string) => {
    if (id !== activeStencil.id) onSelectStencil?.(id);
    setBrowserOpen(false);
    if (collapsed) onToggleCollapsed();
  };
  const openBrowser = () => {
    setBrowserQuery('');
    setBrowserOpen(true);
    if (collapsed) onToggleCollapsed();
  };
  const addStencil = (id: string) => {
    onAddStencil?.(id);
    onSelectStencil?.(id);
    setBrowserOpen(false);
    if (collapsed) onToggleCollapsed();
  };
  const filteredShapes = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase();
    return normalized ? activeStencil.shapes.filter((shape) => t(shape.nameKey).toLocaleLowerCase().includes(normalized)) : activeStencil.shapes;
  }, [query, activeStencil, t]);
  const browserSections = useMemo(() => {
    const normalized = browserQuery.trim().toLocaleLowerCase();
    const known = new Map(shapeStencils.map((stencil) => [stencil.id, stencil]));
    for (const stencil of resolvedStencils) if (!known.has(stencil.id)) known.set(stencil.id, stencil);
    return (catalogue ?? stencilCatalogue).map((category) => ({
      category,
      stencils: category.stencilIds
        .map((id) => known.get(id))
        .filter((stencil): stencil is ShapeStencil => stencil !== undefined && stencil.shapes.length > 0)
        .filter((stencil) => !normalized || t(stencil.nameKey).toLocaleLowerCase().includes(normalized) || stencil.shapes.some((shape) => t(shape.nameKey).toLocaleLowerCase().includes(normalized))),
    })).filter((section) => section.stencils.length > 0);
  }, [browserQuery, catalogue, resolvedStencils, t]);
  const activeIndex = filteredShapes.length ? Math.min(focusIndex, filteredShapes.length - 1) : 0;
  const moveFocus = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const next = nextFocusIndex(event.key, filteredShapes.length, index);
    if (next === index) return;
    event.preventDefault();
    setFocusIndex(next);
    tileRefs.current.get(next)?.focus();
  };
  const rows = Array.from({ length: Math.ceil(filteredShapes.length / COLUMNS) }, (_, row) => filteredShapes.slice(row * COLUMNS, row * COLUMNS + COLUMNS));
  return (
    <aside className={className} style={styles.root} aria-label={t('shapesPanel.title')}>
      <nav style={styles.rail} aria-label={t('shapesPanel.categoriesLabel')}>
        <ul style={{ display: 'contents', margin: 0, padding: 0, listStyle: 'none' }}>
          {resolvedStencils.map((stencil) => {
            const selected = stencil.id === activeStencil.id && !browserOpen;
            return (
              <li key={stencil.id} style={stencil.id === resolvedStencils[0].id ? undefined : { marginTop: 4 }}>
                <button type="button" aria-label={t(stencil.nameKey)} aria-current={selected} title={t(stencil.nameKey)} onClick={() => selectStencil(stencil.id)} style={selected ? styles.railButton : styles.railButtonInactive}>
                  {stencilIcon(stencil.id)}
                </button>
              </li>
            );
          })}
          <li style={{ marginTop: 8 }}>
            <button type="button" aria-label={t('shapesPanel.addShapes')} aria-expanded={browserOpen} title={t('shapesPanel.addShapes')} onClick={() => (browserOpen ? setBrowserOpen(false) : openBrowser())} style={styles.addRailButton}>
              <svg aria-hidden="true" width="16" height="16" viewBox="0 0 16 16"><path d="M 8 3 L 8 13 M 3 8 L 13 8" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" /></svg>
            </button>
          </li>
          {collapsed && <li><button type="button" aria-label={t('shapesPanel.expand')} aria-expanded="false" title={t('shapesPanel.expand')} onClick={onToggleCollapsed} style={{ ...styles.toggle, marginTop: 8 }}><svg aria-hidden="true" width="16" height="16" viewBox="0 0 16 16"><path d="M 6 3 L 11 8 L 6 13" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /></svg></button></li>}
        </ul>
      </nav>
      {!collapsed && !browserOpen && (
        <section style={styles.content} aria-label={t('shapesPanel.title')}>
          <header style={styles.header}>
            <span>{t('shapesPanel.title')}</span>
            <button type="button" aria-label={t('shapesPanel.collapse')} aria-expanded="true" title={t('shapesPanel.collapse')} onClick={onToggleCollapsed} style={styles.toggle}><svg aria-hidden="true" width="16" height="16" viewBox="0 0 16 16"><path d="M 10 3 L 5 8 L 10 13" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /></svg></button>
          </header>
          <div style={styles.searchWrap}>
            <input ref={searchRef} type="search" value={query} onChange={(event) => { setQuery(event.target.value); setFocusIndex(0); }} placeholder={t('shapesPanel.searchPlaceholder')} aria-label={t('shapesPanel.searchLabel')} style={styles.search} />
            <button type="button" aria-hidden="true" tabIndex={-1} onClick={() => searchRef.current?.focus()} style={styles.searchButton}>
              <svg aria-hidden="true" width="16" height="16" viewBox="0 0 16 16"><circle cx="7" cy="7" r="4.5" fill="none" stroke="currentColor" strokeWidth="1.5" /><path d="M 10.5 10.5 L 14 14" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" /></svg>
            </button>
          </div>
          <h2 style={styles.heading}>{t(activeStencil.nameKey)}</h2>
          {filteredShapes.length === 0 ? <p style={styles.empty}>{t('shapesPanel.empty')}</p> : (
            <div role="grid" aria-label={t(activeStencil.nameKey)} style={styles.grid}>
              {rows.map((row, rowIndex) => (
                <div key={rowIndex} role="row" style={styles.row}>
                  {row.map((shape, columnIndex) => {
                    const index = rowIndex * COLUMNS + columnIndex;
                    return (
                      <div key={shape.id} role="gridcell" style={styles.cell}>
                        <button
                          ref={(element) => { if (element) tileRefs.current.set(index, element); else tileRefs.current.delete(index); }}
                          type="button"
                          tabIndex={index === activeIndex ? 0 : -1}
                          aria-label={t(shape.nameKey)}
                          onFocus={() => setFocusIndex(index)}
                          onClick={() => onInsert(shape)}
                          onKeyDown={(event) => moveFocus(event, index)}
                          style={styles.tile}
                        >
                          <svg aria-hidden="true" viewBox="0 0 1 1" preserveAspectRatio="xMidYMid meet" style={styles.preview}><path d={shape.preview} /></svg>
                          <span title={t(shape.nameKey)} style={styles.label}>{t(shape.nameKey)}</span>
                        </button>
                      </div>
                    );
                  })}
                </div>
              ))}
            </div>
          )}
        </section>
      )}
      {!collapsed && browserOpen && (
        <section style={styles.content} aria-label={t('shapesPanel.addShapes')}>
          <header style={styles.header}>
            <span style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
              <button type="button" aria-label={t('shapesPanel.browseBack')} title={t('shapesPanel.browseBack')} onClick={() => setBrowserOpen(false)} style={styles.toggle}><svg aria-hidden="true" width="16" height="16" viewBox="0 0 16 16"><path d="M 10 3 L 5 8 L 10 13" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /></svg></button>
              <span>{t('shapesPanel.addShapes')}</span>
            </span>
            <button type="button" aria-label={t('shapesPanel.collapse')} aria-expanded="true" title={t('shapesPanel.collapse')} onClick={onToggleCollapsed} style={styles.toggle}><svg aria-hidden="true" width="16" height="16" viewBox="0 0 16 16"><path d="M 10 3 L 5 8 L 10 13" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /></svg></button>
          </header>
          <div style={styles.searchWrap}>
            <input type="search" value={browserQuery} onChange={(event) => setBrowserQuery(event.target.value)} placeholder={t('shapesPanel.searchPlaceholder')} aria-label={t('shapesPanel.searchLabel')} style={styles.search} />
            <span aria-hidden="true" style={{ ...styles.searchButton, cursor: 'default' }}>
              <svg aria-hidden="true" width="16" height="16" viewBox="0 0 16 16"><circle cx="7" cy="7" r="4.5" fill="none" stroke="currentColor" strokeWidth="1.5" /><path d="M 10.5 10.5 L 14 14" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" /></svg>
            </span>
          </div>
          {browserSections.length === 0 ? <p style={styles.empty}>{t('shapesPanel.browserEmpty')}</p> : (
            <div style={styles.browserList}>
              {browserSections.map((section) => (
                <div key={section.category.id}>
                  <h2 style={styles.browserHeading}>{t(section.category.nameKey)}</h2>
                  <ul style={{ display: 'flex', flexDirection: 'column', gap: 6, margin: 0, padding: '0 2px', listStyle: 'none' }}>
                    {section.stencils.map((stencil) => {
                      const added = resolvedStencils.some((rail) => rail.id === stencil.id);
                      return (
                        <li key={stencil.id} style={styles.browserRow}>
                          <span style={styles.browserName}>
                            <span title={t(stencil.nameKey)} style={styles.browserTitle}>{t(stencil.nameKey)}</span>
                            <span style={styles.browserCount}>{t('shapesPanel.shapeCount', { count: stencil.shapes.length })}</span>
                          </span>
                          {added
                            ? <span style={styles.addedBadge}>✓ {t('shapesPanel.addedStencil')}</span>
                            : <button type="button" aria-label={`${t('shapesPanel.addStencil')}: ${t(stencil.nameKey)}`} onClick={() => addStencil(stencil.id)} style={styles.addButton}>{t('shapesPanel.addStencil')}</button>}
                        </li>
                      );
                    })}
                  </ul>
                </div>
              ))}
            </div>
          )}
        </section>
      )}
    </aside>
  );
}

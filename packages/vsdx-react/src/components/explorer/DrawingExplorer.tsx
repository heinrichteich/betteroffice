import { useEffect, useRef, useState } from 'react';
import type { CSSProperties } from 'react';
import type { TFunction } from '@betteroffice/vsdx-i18n';
import type { CellSnapshot, DiagramSnapshot, ShapeSnapshot } from '@betteroffice/vsdx';
import type { VsdxShapeSelection } from '../../VsdxEditor';

export interface DrawingExplorerProps {
  snapshot: DiagramSnapshot | null;
  activePageId?: string;
  selection: VsdxShapeSelection | null;
  onSelectPage: (index: number) => void;
  onSelectShape: (pageId: string, shapeId: string) => void;
  t: TFunction;
  className?: string;
  collapsed?: boolean;
  onToggleCollapsed?: () => void;
}

/** Shape nesting rendered before the tree truncates with a marker. */
export const MAX_EXPLORER_DEPTH = 16;

export interface ExplorerSection {
  key: string;
  name: string;
  cells: CellSnapshot[];
}

export interface ExplorerRow {
  key: string;
  label: string | null;
  cells: CellSnapshot[];
}

/** Visible tree label for a shape: its real id plus its name where one exists. */
export function shapeLabel(shape: ShapeSnapshot): string {
  const base = `Shape ${shape.sourceId}`;
  return shape.name ? `${base} "${shape.name}"` : base;
}

function cellField(cell: CellSnapshot): string {
  return cell.locator.cellName ?? cell.name;
}

function numericText(value: string | null | undefined): number | null {
  if (value === null || value === undefined) return null;
  const parsed = Number(value.trim());
  return Number.isFinite(parsed) ? parsed : null;
}

/** True when the resolved ShapeSheet marks the shape as a 1-D connector. */
export function isOneDShape(shape: ShapeSnapshot): boolean {
  const oneD = shape.cells.find((cell) => cellField(cell) === 'OneD');
  if (oneD) {
    const parsed = numericText(oneD.value ?? oneD.formula);
    if (parsed !== null) return parsed !== 0;
    const raw = (oneD.value ?? oneD.formula ?? '').trim();
    return raw !== '' && raw !== '0';
  }
  return ['BeginX', 'BeginY', 'EndX', 'EndY'].every((name) =>
    shape.cells.some((cell) => cellField(cell) === name),
  );
}

/** Group a shape's resolved cells by ShapeSheet section, keeping document order. */
export function groupShapeCells(cells: readonly CellSnapshot[]): ExplorerSection[] {
  const groups = new Map<string, ExplorerSection>();
  for (const cell of cells) {
    const section = cell.locator.section ?? null;
    const sectionIndex = cell.locator.sectionIndex ?? null;
    const name = section ?? 'Shape';
    const key = section === null ? 'Shape' : `${section} ${sectionIndex ?? 0}`;
    const display = section === null || sectionIndex === null || sectionIndex === 0
      ? name
      : `${name} ${sectionIndex + 1}`;
    const existing = groups.get(key);
    if (existing) existing.cells.push(cell);
    else groups.set(key, { key, name: display, cells: [cell] });
  }
  return [...groups.values()];
}

/** Group a section's cells by row, keeping document order. */
export function groupSectionRows(cells: readonly CellSnapshot[]): ExplorerRow[] {
  const groups = new Map<string, ExplorerRow>();
  for (const cell of cells) {
    const row = cell.locator.row ?? null;
    if (row === null) {
      const key = `cell:${cellField(cell)}`;
      const existing = groups.get(key);
      if (existing) existing.cells.push(cell);
      else groups.set(key, { key, label: null, cells: [cell] });
      continue;
    }
    const rowType = cell.rowType ? ` (${cell.rowType})` : '';
    if ('index' in row) {
      const key = `row:${row.index}`;
      const existing = groups.get(key);
      if (existing) {
        existing.cells.push(cell);
        const label = existing.label;
        if (label && !label.includes('(') && cell.rowType) existing.label = `${label} (${cell.rowType})`;
      } else {
        groups.set(key, { key, label: `Row ${row.index + 1}${rowType}`, cells: [cell] });
      }
      continue;
    }
    const key = `name:${row.name}`;
    const existing = groups.get(key);
    if (existing) {
      existing.cells.push(cell);
      const label = existing.label;
      if (label && !label.includes('(') && cell.rowType) existing.label = `${label} (${cell.rowType})`;
    } else {
      groups.set(key, { key, label: `"${row.name}"${rowType}`, cells: [cell] });
    }
  }
  return [...groups.values()];
}

function ancestorShapeIds(shapes: readonly ShapeSnapshot[], shapeId: string, depth = 0): string[] | null {
  if (depth > MAX_EXPLORER_DEPTH + 1) return null;
  for (const shape of shapes) {
    if (shape.id === shapeId) return [shape.id];
    const nested = ancestorShapeIds(shape.children, shapeId, depth + 1);
    if (nested) return [shape.id, ...nested];
  }
  return null;
}

export function DrawingExplorer({ snapshot, activePageId, selection, onSelectPage, onSelectShape, t, className, collapsed, onToggleCollapsed }: DrawingExplorerProps) {
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set(['document', 'pages']));
  const selectedRef = useRef<HTMLElement>(null);
  const toggle = (key: string) => {
    setExpanded((previous) => {
      const next = new Set(previous);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  useEffect(() => {
    if (!snapshot || !selection) return;
    const page = snapshot.pages.find((entry) => entry.id === selection.pageId);
    if (!page) return;
    const chain = ancestorShapeIds(page.shapes, selection.shapeId);
    if (!chain) return;
    setExpanded((previous) => {
      const next = new Set(previous);
      next.add(`page:${page.id}`);
      next.add(`shapes:${page.id}`);
      for (const id of chain) next.add(`shape:${id}`);
      return next;
    });
  }, [snapshot, selection]);

  useEffect(() => {
    selectedRef.current?.scrollIntoView?.({ block: 'nearest' });
  }, [selection]);

  if (collapsed) {
    return (
      <aside className={className} style={styles.rail} aria-label={t('explorer.title')}>
        <button
          type="button"
          aria-label={t('explorer.expand')}
          aria-expanded="false"
          title={t('explorer.expand')}
          onClick={onToggleCollapsed}
          style={styles.railButton}
        >
          <svg aria-hidden="true" width="16" height="16" viewBox="0 0 16 16"><path d="M 10 3 L 5 8 L 10 13" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /></svg>
        </button>
      </aside>
    );
  }

  const pages = snapshot?.pages ?? [];
  const activeIndex = Math.max(0, pages.findIndex((page) => page.id === activePageId));

  return (
    <aside className={className} style={styles.root} aria-label={t('explorer.title')}>
      <header style={styles.header}>
        <span>{t('explorer.title')}</span>
        {onToggleCollapsed && (
          <button type="button" aria-label={t('explorer.collapse')} aria-expanded="true" title={t('explorer.collapse')} onClick={onToggleCollapsed} style={styles.toggle}>
            <svg aria-hidden="true" width="16" height="16" viewBox="0 0 16 16"><path d="M 6 3 L 11 8 L 6 13" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /></svg>
          </button>
        )}
      </header>
      <div style={styles.treeWrap}>
        {pages.length === 0 ? (
          <p style={styles.empty}>{t('explorer.empty')}</p>
        ) : (
          <ul role="tree" aria-label={t('explorer.title')} style={styles.tree}>
            <li role="treeitem" aria-expanded={expanded.has('document')} aria-selected="false">
              <span style={styles.row}>
                <ToggleButton expanded={expanded.has('document')} label={t('explorer.document')} onToggle={() => toggle('document')} t={t} />
                <span style={styles.nodeLabel}>{t('explorer.document')}</span>
              </span>
              {expanded.has('document') && (
                <ul role="group" style={styles.group}>
                  <li role="treeitem" aria-expanded={expanded.has('pages')} aria-selected="false">
                    <span style={styles.row}>
                      <ToggleButton expanded={expanded.has('pages')} label={t('explorer.pages')} onToggle={() => toggle('pages')} t={t} />
                      <span style={styles.nodeLabel}>{t('explorer.pages')}</span>
                    </span>
                    {expanded.has('pages') && (
                      <ul role="group" style={styles.group}>
                        {pages.map((page, index) => (
                          <PageNode
                            key={page.id}
                            pageId={page.id}
                            pageName={page.name ?? t('pages.fallbackTitle', { number: index + 1 })}
                            pageKey={`page:${page.id}`}
                            shapesKey={`shapes:${page.id}`}
                            shapes={page.shapes}
                            active={index === activeIndex}
                            selection={selection}
                            expanded={expanded}
                            onToggle={toggle}
                            onSelectPage={() => onSelectPage(index)}
                            onSelectShape={(shapeId) => onSelectShape(page.id, shapeId)}
                            selectedRef={selectedRef}
                            depth={0}
                            t={t}
                          />
                        ))}
                      </ul>
                    )}
                  </li>
                </ul>
              )}
            </li>
          </ul>
        )}
      </div>
    </aside>
  );
}

function ToggleButton({ expanded, label, onToggle, t }: { expanded: boolean; label: string; onToggle: () => void; t: TFunction }) {
  return (
    <button
      type="button"
      aria-label={expanded ? t('explorer.collapseNode', { name: label }) : t('explorer.expandNode', { name: label })}
      aria-expanded={expanded}
      onClick={onToggle}
      style={styles.toggle}
    >
      <svg aria-hidden="true" width="12" height="12" viewBox="0 0 12 12" style={{ transform: expanded ? 'rotate(90deg)' : undefined }}>
        <path d="M 4 2 L 8 6 L 4 10" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
      </svg>
    </button>
  );
}

function PageNode({ pageId, pageName, pageKey, shapesKey, shapes, active, selection, expanded, onToggle, onSelectPage, onSelectShape, selectedRef, depth, t }: {
  pageId: string;
  pageName: string;
  pageKey: string;
  shapesKey: string;
  shapes: readonly ShapeSnapshot[];
  active: boolean;
  selection: VsdxShapeSelection | null;
  expanded: Set<string>;
  onToggle: (key: string) => void;
  onSelectPage: () => void;
  onSelectShape: (shapeId: string) => void;
  selectedRef: { current: HTMLElement | null };
  depth: number;
  t: TFunction;
}) {
  const pageOpen = expanded.has(pageKey);
  const shapesOpen = expanded.has(shapesKey);
  return (
    <li role="treeitem" aria-expanded={pageOpen} aria-selected={active}>
      <span style={styles.row}>
        <ToggleButton expanded={pageOpen} label={pageName} onToggle={() => onToggle(pageKey)} t={t} />
        <button type="button" aria-current={active} onClick={onSelectPage} style={active ? { ...styles.select, ...styles.selected } : styles.select}>
          {pageName}
        </button>
      </span>
      {pageOpen && (
        <ul role="group" style={styles.group}>
          <li role="treeitem" aria-expanded={shapesOpen} aria-selected="false">
            <span style={styles.row}>
              <ToggleButton expanded={shapesOpen} label={t('explorer.shapes')} onToggle={() => onToggle(shapesKey)} t={t} />
              <span style={styles.nodeLabel}>{t('explorer.shapes')}</span>
            </span>
            {shapesOpen && (
              <ul role="group" style={styles.group}>
                {shapes.length === 0 && <li style={styles.emptyItem}>{t('explorer.noShapes')}</li>}
                {shapes.map((shape) => (
                  <ShapeNode
                    key={shape.id}
                    pageId={pageId}
                    shape={shape}
                    selection={selection}
                    expanded={expanded}
                    onToggle={onToggle}
                    onSelectShape={onSelectShape}
                    selectedRef={selectedRef}
                    depth={depth}
                    t={t}
                  />
                ))}
              </ul>
            )}
          </li>
        </ul>
      )}
    </li>
  );
}

function ShapeNode({ pageId, shape, selection, expanded, onToggle, onSelectShape, selectedRef, depth, t }: {
  pageId: string;
  shape: ShapeSnapshot;
  selection: VsdxShapeSelection | null;
  expanded: Set<string>;
  onToggle: (key: string) => void;
  onSelectShape: (shapeId: string) => void;
  selectedRef: { current: HTMLElement | null };
  depth: number;
  t: TFunction;
}) {
  const label = shapeLabel(shape);
  const selected = selection?.pageId === pageId && selection?.shapeId === shape.id;
  const group = shape.children.length > 0;
  const oneD = isOneDShape(shape);
  const expandable = group || shape.cells.length > 0 || depth >= MAX_EXPLORER_DEPTH;
  const open = expanded.has(`shape:${shape.id}`);
  if (depth >= MAX_EXPLORER_DEPTH && group) {
    return (
      <li role="treeitem" aria-selected={selected} aria-label={label}>
        <span style={styles.row}>
          <button type="button" onClick={() => onSelectShape(shape.id)} style={selected ? { ...styles.select, ...styles.selected } : styles.select}>
            {label}
            {oneD && <Badge text={t('explorer.connector')} />}
          </button>
          <span style={styles.truncated}>{t('explorer.truncated')}</span>
        </span>
      </li>
    );
  }
  return (
    <li
      role="treeitem"
      aria-expanded={expandable ? open : undefined}
      aria-selected={selected}
      aria-label={label}
      ref={selected ? (element) => { selectedRef.current = element; } : undefined}
    >
      <span style={styles.row}>
        {expandable && <ToggleButton expanded={open} label={label} onToggle={() => onToggle(`shape:${shape.id}`)} t={t} />}
        <button type="button" onClick={() => onSelectShape(shape.id)} style={selected ? { ...styles.select, ...styles.selected } : styles.select}>
          {label}
          {group && <Badge text={t('explorer.group')} />}
          {oneD && <Badge text={t('explorer.connector')} />}
        </button>
      </span>
      {open && expandable && (
        <ul role="group" style={styles.group}>
          {shape.children.map((child) => (
            <ShapeNode
              key={child.id}
              pageId={pageId}
              shape={child}
              selection={selection}
              expanded={expanded}
              onToggle={onToggle}
              onSelectShape={onSelectShape}
              selectedRef={selectedRef}
              depth={depth + 1}
              t={t}
            />
          ))}
          {shape.cells.length > 0 && (
            <SheetNode shape={shape} expanded={expanded} onToggle={onToggle} t={t} />
          )}
        </ul>
      )}
    </li>
  );
}

function SheetNode({ shape, expanded, onToggle, t }: { shape: ShapeSnapshot; expanded: Set<string>; onToggle: (key: string) => void; t: TFunction }) {
  const sheetKey = `sheet:${shape.id}`;
  const open = expanded.has(sheetKey);
  const sections = open ? groupShapeCells(shape.cells) : [];
  return (
    <li role="treeitem" aria-expanded={open} aria-selected="false" aria-label={t('explorer.shapeSheet')}>
      <span style={styles.row}>
        <ToggleButton expanded={open} label={t('explorer.shapeSheet')} onToggle={() => onToggle(sheetKey)} t={t} />
        <span style={styles.sheetLabel}>{t('explorer.shapeSheet')}</span>
      </span>
      {open && (
        <ul role="group" style={styles.group}>
          {sections.length === 0 && <li style={styles.emptyItem}>{t('explorer.noCells')}</li>}
          {sections.map((section) => (
            <SectionNode key={section.key} shapeId={shape.id} section={section} expanded={expanded} onToggle={onToggle} t={t} />
          ))}
        </ul>
      )}
    </li>
  );
}

function SectionNode({ shapeId, section, expanded, onToggle, t }: { shapeId: string; section: ExplorerSection; expanded: Set<string>; onToggle: (key: string) => void; t: TFunction }) {
  const sectionKey = `section:${shapeId}:${section.key}`;
  const open = expanded.has(sectionKey);
  const rows = open ? groupSectionRows(section.cells) : [];
  return (
    <li role="treeitem" aria-expanded={open} aria-selected="false" aria-label={section.name}>
      <span style={styles.row}>
        <ToggleButton expanded={open} label={section.name} onToggle={() => onToggle(sectionKey)} t={t} />
        <span style={styles.nodeLabel}>{section.name}</span>
      </span>
      {open && (
        <ul role="group" style={styles.group}>
          {rows.map((row) => (
            <li key={row.key} role="treeitem" aria-selected="false" aria-label={row.label ?? t('explorer.cells')}>
              {row.label && <div style={styles.rowLabel}>{row.label}</div>}
              <table style={styles.table}>
                <thead>
                  <tr>
                    <th style={styles.th}>{t('explorer.cell')}</th>
                    <th style={styles.th}>{t('explorer.formula')}</th>
                    <th style={styles.th}>{t('explorer.value')}</th>
                  </tr>
                </thead>
                <tbody>
                  {row.cells.map((cell, index) => (
                    <tr key={`${cellField(cell)}:${index}`}>
                      <td style={styles.td}>{cellField(cell)}</td>
                      <td style={styles.tdFormula}>{cell.formula ?? '—'}</td>
                      <td style={styles.td}>{cell.value ?? '—'}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </li>
          ))}
        </ul>
      )}
    </li>
  );
}

function Badge({ text }: { text: string }) {
  return <span style={styles.badge}>{text}</span>;
}

const styles: Record<string, CSSProperties> = {
  root: { display: 'flex', flexDirection: 'column', width: 300, minWidth: 240, maxWidth: 360, height: '100%', background: '#fff', color: '#242424', font: '400 13px ui-sans-serif, system-ui, sans-serif', borderLeft: '1px solid #e0e0e0', boxSizing: 'border-box' },
  rail: { display: 'flex', flexDirection: 'column', alignItems: 'center', padding: '8px 6px', background: '#f7f7f7', borderLeft: '1px solid #e5e5e5', boxSizing: 'border-box' },
  railButton: { appearance: 'none', display: 'grid', placeItems: 'center', width: 30, height: 30, padding: 0, border: 0, borderRadius: 4, background: 'transparent', color: '#424242', cursor: 'pointer' },
  header: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', minHeight: 44, padding: '0 10px 0 14px', borderBottom: '1px solid #e5e5e5', fontWeight: 600, fontSize: 14 },
  toggle: { appearance: 'none', display: 'inline-grid', placeItems: 'center', flex: '0 0 auto', width: 22, height: 22, padding: 0, border: 0, borderRadius: 4, background: 'transparent', color: '#424242', cursor: 'pointer' },
  treeWrap: { flex: 1, minHeight: 0, overflowY: 'auto', padding: '6px 6px 12px' },
  tree: { margin: 0, padding: 0, listStyle: 'none' },
  group: { margin: 0, padding: '0 0 0 18px', listStyle: 'none' },
  row: { display: 'flex', alignItems: 'center', gap: 2, minHeight: 26 },
  nodeLabel: { padding: '2px 6px', color: '#242424' },
  sheetLabel: { padding: '2px 6px', color: '#616161', fontStyle: 'italic' },
  select: { appearance: 'none', maxWidth: '100%', padding: '2px 6px', overflow: 'hidden', border: 0, borderRadius: 4, background: 'transparent', color: '#242424', cursor: 'pointer', font: 'inherit', textAlign: 'left', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  selected: { background: '#dbeafe', color: '#0f6cbd', fontWeight: 600 },
  badge: { display: 'inline-block', marginLeft: 6, padding: '0 6px', borderRadius: 8, background: '#e8eaf0', color: '#424242', fontSize: 11, fontWeight: 400, verticalAlign: '1px' },
  rowLabel: { margin: '4px 0 2px 28px', color: '#616161', fontSize: 12, fontWeight: 600 },
  table: { margin: '0 0 6px 28px', borderCollapse: 'collapse', fontSize: 12 },
  th: { padding: '2px 8px 2px 0', borderBottom: '1px solid #e0e0e0', color: '#616161', fontWeight: 600, textAlign: 'left' },
  td: { maxWidth: 220, padding: '2px 8px 2px 0', overflow: 'hidden', borderBottom: '1px solid #f0f0f0', color: '#242424', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  tdFormula: { maxWidth: 220, padding: '2px 8px 2px 0', overflow: 'hidden', borderBottom: '1px solid #f0f0f0', color: '#242424', fontFamily: 'ui-monospace, monospace', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  empty: { margin: '20px 12px', color: '#616161', textAlign: 'center' },
  emptyItem: { color: '#616161', fontSize: 12, listStyle: 'none' },
  truncated: { marginLeft: 6, color: '#616161', fontSize: 12, fontStyle: 'italic' },
};

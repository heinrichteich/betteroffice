import { formatOptions, shapeDataValueFormula, visibleShapeDataRows } from '@betteroffice/vsdx';
import type { ShapeDataRow, ShapeSnapshot } from '@betteroffice/vsdx';
import type { TFunction } from '@betteroffice/vsdx-i18n';
import { useState } from 'react';
import type { CSSProperties, KeyboardEvent } from 'react';

export interface ShapeDataPanelProps {
  shape: ShapeSnapshot | null;
  onCommit: (row: ShapeDataRow, formula: string) => void;
  onError: (error: unknown) => void;
  t: TFunction;
  className?: string;
  linkedRowNames?: ReadonlyArray<string> | ReadonlySet<string>;
  staleRowNames?: ReadonlyArray<string> | ReadonlySet<string>;
}

const styles: Record<string, CSSProperties> = {
  root: { display: 'flex', flexDirection: 'column', width: 264, minWidth: 264, height: '100%', background: '#fff', color: '#242424', font: '400 13px ui-sans-serif, system-ui, sans-serif', borderLeft: '1px solid #e0e0e0', boxSizing: 'border-box' },
  header: { display: 'flex', alignItems: 'center', minHeight: 44, padding: '0 14px', borderBottom: '1px solid #e5e5e5', fontWeight: 600, fontSize: 14 },
  list: { display: 'flex', flexDirection: 'column', gap: 10, margin: 0, padding: '12px 14px', overflowY: 'auto', listStyle: 'none' },
  field: { display: 'flex', flexDirection: 'column', gap: 4, minWidth: 0 },
  label: { overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis', color: '#424242', fontWeight: 600, fontSize: 12 },
  input: { width: '100%', height: 30, padding: '0 8px', border: '1px solid #bdbdbd', borderRadius: 3, outline: 0, color: '#242424', font: '400 13px ui-sans-serif, system-ui, sans-serif', boxSizing: 'border-box' },
  checkRow: { display: 'flex', alignItems: 'center', gap: 8 },
  labelRow: { display: 'flex', alignItems: 'center', gap: 6, minWidth: 0 },
  badge: { flex: 'none', padding: '1px 6px', borderRadius: 8, fontSize: 10, fontWeight: 600, lineHeight: '16px' },
  linkedBadge: { background: '#e8f1fd', color: '#0f6cbd' },
  staleBadge: { background: '#fdf3e3', color: '#8a5100' },
  empty: { margin: '20px 14px', color: '#616161', textAlign: 'center' },
};

export function ShapeDataPanel({ shape, onCommit, onError, t, className, linkedRowNames, staleRowNames }: ShapeDataPanelProps) {
  const rows = visibleShapeDataRows(shape);
  const linked = new Set(linkedRowNames ?? []);
  const stale = new Set(staleRowNames ?? []);
  return (
    <aside className={className} style={styles.root} aria-label={t('shapeData.title')}>
      <header style={styles.header}><span>{t('shapeData.title')}</span></header>
      {!shape ? <p style={styles.empty}>{t('shapeData.noSelection')}</p>
      : rows.length === 0 ? <p style={styles.empty}>{t('shapeData.empty')}</p>
      : (
        <ul style={styles.list}>
          {rows.map((row) => {
            const key = row.rowName ?? (row.rowIndex !== null ? `IX:${row.rowIndex}` : '');
            const isStale = (row.rowName !== null && stale.has(row.rowName)) || stale.has(key);
            const isLinked = isStale || (row.rowName !== null && linked.has(row.rowName)) || linked.has(key);
            return (
              <li key={row.rowName ?? `IX:${row.rowIndex}`} style={styles.field}>
                <span style={styles.labelRow}>
                  <label
                    style={styles.label}
                    title={row.prompt ?? row.label}
                    htmlFor={`shape-data-${row.rowName ?? `ix-${row.rowIndex}`}`}
                  >
                    {row.label}
                  </label>
                  {isLinked && (
                    <span
                      style={{ ...styles.badge, ...(isStale ? styles.staleBadge : styles.linkedBadge) }}
                      title={isStale ? t('shapeData.staleTitle') : t('shapeData.linkedTitle')}
                    >
                      {isStale ? t('shapeData.stale') : t('shapeData.linked')}
                    </span>
                  )}
                </span>
                <ShapeDataValueInput row={row} onCommit={onCommit} onError={onError} t={t} />
              </li>
            );
          })}
        </ul>
      )}
    </aside>
  );
}

function ShapeDataValueInput({ row, onCommit, onError, t }: { row: ShapeDataRow; onCommit: (row: ShapeDataRow, formula: string) => void; onError: (error: unknown) => void; t: TFunction }) {
  const id = `shape-data-${row.rowName ?? `ix-${row.rowIndex}`}`;
  const commit = (formula: string) => {
    try { onCommit(row, formula); }
    catch (error) { onError(error); }
  };
  if (row.type === 'boolean') {
    const checked = row.displayValue !== '' && row.displayValue !== '0' && !/^false$/i.test(row.displayValue);
    return (
      <span style={styles.checkRow}>
        <input id={id} type="checkbox" aria-label={t('shapeData.valueLabel')} checked={checked} onChange={(event) => commit(event.target.checked ? '1' : '0')} />
      </span>
    );
  }
  const options = row.type === 'fixed-list' || row.type === 'variable-list' ? formatOptions(row.format) : [];
  if (options.length > 0) {
    const known = options.includes(row.displayValue);
    return (
      <select
        id={id}
        aria-label={t('shapeData.valueLabel')}
        value={known ? row.displayValue : ''}
        onChange={(event) => commit(shapeDataValueFormula(row.type, event.target.value))}
        style={styles.input}
      >
        {!known && <option value="">{row.displayValue}</option>}
        {options.map((option) => <option key={option} value={option}>{option}</option>)}
      </select>
    );
  }
  return <ShapeDataTextInput key={`${row.rowName ?? row.rowIndex}:${row.displayValue}`} id={id} row={row} onCommit={commit} t={t} />;
}

function ShapeDataTextInput({ id, row, onCommit, t }: { id: string; row: ShapeDataRow; onCommit: (formula: string) => void; t: TFunction }) {
  const [draft, setDraft] = useState(row.displayValue);
  const commitIfChanged = () => {
    if (draft !== row.displayValue) onCommit(shapeDataValueFormula(row.type, draft));
  };
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') (event.target as HTMLInputElement).blur();
    if (event.key === 'Escape') setDraft(row.displayValue);
  };
  return (
    <input
      id={id}
      type="text"
      aria-label={t('shapeData.valueLabel')}
      value={draft}
      onChange={(event) => setDraft(event.target.value)}
      onBlur={commitIfChanged}
      onKeyDown={onKeyDown}
      style={styles.input}
    />
  );
}

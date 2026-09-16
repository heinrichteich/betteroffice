import { bindRow, readBindingDoc, refreshBindings, rekeyTable, tableFromCsv, unbindRow } from '@betteroffice/vsdx';
import type { DataBindingSource, DataTable, DiagramHandle, DiagramSnapshot, RefreshReport } from '@betteroffice/vsdx';
import { createT, en } from '@betteroffice/vsdx-i18n';
import type { TFunction } from '@betteroffice/vsdx-i18n';
import { useMemo, useState } from 'react';
import type { CSSProperties } from 'react';

export interface DataBindingSelection {
  pageId: string;
  shapeId: string;
}

export interface LoadedTable {
  table: DataTable;
  source: DataBindingSource;
}

export interface DataBindingPanelProps {
  handle: DiagramHandle | null;
  snapshot: DiagramSnapshot | null;
  selection: DataBindingSelection | null;
  loadXlsxTable?: (file: File) => Promise<LoadedTable>;
  onMutated: () => void;
  onError: (error: unknown) => void;
  onTableChange?: (loaded: LoadedTable | null) => void;
  t?: TFunction;
  className?: string;
}

const styles: Record<string, CSSProperties> = {
  root: { display: 'flex', flexDirection: 'column', width: 264, minWidth: 264, height: '100%', background: '#fff', color: '#242424', font: '400 13px ui-sans-serif, system-ui, sans-serif', borderLeft: '1px solid #e0e0e0', boxSizing: 'border-box' },
  header: { display: 'flex', alignItems: 'center', minHeight: 44, padding: '0 14px', borderBottom: '1px solid #e5e5e5', fontWeight: 600, fontSize: 14 },
  body: { display: 'flex', flexDirection: 'column', gap: 12, margin: 0, padding: '12px 14px', overflowY: 'auto' },
  section: { display: 'flex', flexDirection: 'column', gap: 6, minWidth: 0 },
  heading: { margin: 0, fontSize: 12, fontWeight: 600, color: '#424242' },
  hint: { margin: 0, fontSize: 12, color: '#616161' },
  label: { fontSize: 12, fontWeight: 600, color: '#424242' },
  input: { width: '100%', height: 30, padding: '0 8px', border: '1px solid #bdbdbd', borderRadius: 3, outline: 0, color: '#242424', font: '400 13px ui-sans-serif, system-ui, sans-serif', boxSizing: 'border-box' },
  button: { height: 30, padding: '0 12px', border: '1px solid #bdbdbd', borderRadius: 3, background: '#fff', color: '#242424', font: '600 13px ui-sans-serif, system-ui, sans-serif', cursor: 'pointer' },
  primary: { background: '#0f6cbd', borderColor: '#0f6cbd', color: '#fff' },
  row: { display: 'flex', alignItems: 'center', gap: 8, minWidth: 0 },
  grow: { flex: 1, minWidth: 0, overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis' },
  notice: { margin: 0, fontSize: 12, color: '#424242', overflowWrap: 'anywhere' },
  badge: { flex: 'none', padding: '1px 6px', borderRadius: 8, fontSize: 10, fontWeight: 600, lineHeight: '16px', background: '#fdf3e3', color: '#8a5100' },
};

export function DataBindingPanel({ handle, snapshot, selection, loadXlsxTable, onMutated, onError, onTableChange, t, className }: DataBindingPanelProps) {
  const translate = useMemo(() => t ?? createT(en), [t]);
  const [table, setTable] = useState<DataTable | null>(null);
  const [source, setSource] = useState<DataBindingSource | null>(null);
  const [rowKey, setRowKey] = useState('');
  const [notice, setNotice] = useState<string | null>(null);
  const ready = handle !== null && snapshot !== null;
  const doc = table && source && snapshot ? readBindingDoc(snapshot, source) : null;

  const fail = (error: unknown) => { onError(error); };
  const describeRefresh = (report: RefreshReport): string => {
    const parts = [translate('dataBinding.refreshed', { updated: report.updated.length })];
    if (report.stale.length > 0) parts.push(translate('dataBinding.staleBindings', { details: report.stale.map((entry) => entry.key).join(', ') }));
    if (report.removed.length > 0) parts.push(translate('dataBinding.removedBindings', { details: report.removed.map((entry) => entry.key).join(', ') }));
    if (report.refused.length > 0) parts.push(translate('dataBinding.refusedColumns', { details: report.refused.map((entry) => `${entry.column}: ${entry.reason}`).join('; ') }));
    if (report.unmappedColumns.length > 0) parts.push(translate('dataBinding.unmappedColumns', { columns: report.unmappedColumns.join(', ') }));
    return parts.join(' ');
  };

  const adopt = (loaded: LoadedTable) => {
    setTable(loaded.table);
    setSource(loaded.source);
    setRowKey(loaded.table.rows[0]?.key ?? '');
    onTableChange?.(loaded);
    if (!handle || !snapshot) return;
    const stored = readBindingDoc(snapshot, loaded.source);
    if (!stored || stored.bindings.length === 0) return;
    setNotice(describeRefresh(refreshBindings(handle, snapshot, loaded.table, loaded.source)));
    onMutated();
  };

  const importFile = async (file: File) => {
    setNotice(null);
    try {
      if (/\.xlsx$/i.test(file.name)) {
        if (!loadXlsxTable) throw new Error(translate('dataBinding.xlsxUnavailable'));
        adopt(await loadXlsxTable(file));
      } else {
        const name = file.name || 'table.csv';
        const parsed = tableFromCsv(await file.text(), name);
        adopt({ table: parsed, source: { kind: 'csv', name, sheet: null, keyColumn: parsed.keyColumn } });
      }
    } catch (error) { fail(error); }
  };

  const changeKeyColumn = (keyColumn: string) => {
    if (!table || !source) return;
    try {
      const rekeyed = rekeyTable(table, keyColumn);
      const next = { ...source, keyColumn };
      setTable(rekeyed);
      setSource(next);
      setRowKey(rekeyed.rows[0]?.key ?? '');
      onTableChange?.({ table: rekeyed, source: next });
    } catch (error) { fail(error); }
  };

  const linkRow = () => {
    if (!handle || !snapshot || !table || !source || !selection || !rowKey) return;
    try {
      const outcome = bindRow(handle, snapshot, table, source, selection.pageId, selection.shapeId, rowKey);
      const parts = [translate('dataBinding.linkedTo', { key: rowKey })];
      if (outcome.applied.length > 0) parts.push(translate('dataBinding.appliedColumns', { columns: outcome.applied.join(', ') }));
      if (outcome.refused.length > 0) parts.push(translate('dataBinding.refusedColumns', { details: outcome.refused.map((entry) => `${entry.column}: ${entry.reason}`).join('; ') }));
      if (outcome.unmappedColumns.length > 0) parts.push(translate('dataBinding.unmappedColumns', { columns: outcome.unmappedColumns.join(', ') }));
      setNotice(parts.join(' '));
      onMutated();
    } catch (error) { fail(error); }
  };

  const refresh = () => {
    if (!handle || !snapshot || !table || !source) return;
    try {
      setNotice(describeRefresh(refreshBindings(handle, snapshot, table, source)));
      onMutated();
    } catch (error) { fail(error); }
  };

  const unlink = (shapeId: string) => {
    if (!handle || !snapshot || !source) return;
    try {
      if (unbindRow(handle, snapshot, source, shapeId)) onMutated();
    } catch (error) { fail(error); }
  };

  return (
    <aside className={className} style={styles.root} aria-label={translate('dataBinding.title')}>
      <header style={styles.header}><span>{translate('dataBinding.title')}</span></header>
      <div style={styles.body}>
        <section style={styles.section} aria-label={translate('dataBinding.importHeading')}>
          <h3 style={styles.heading}>{translate('dataBinding.importHeading')}</h3>
          <label style={styles.label} htmlFor="data-binding-file">{translate('dataBinding.fileLabel')}</label>
          <input
            id="data-binding-file"
            type="file"
            accept=".xlsx,.csv"
            disabled={!ready}
            onChange={(event) => { const file = event.target.files?.[0]; event.target.value = ''; if (file) void importFile(file); }}
          />
          {!table && <p style={styles.hint}>{translate('dataBinding.noTable')}</p>}
          {table && source && (
            <>
              <p style={styles.hint}>{translate('dataBinding.tableSummary', { rows: table.rows.length, columns: table.columns.length, name: table.name })}</p>
              {table.skippedRows > 0 && <p style={styles.hint}>{translate('dataBinding.skippedRows', { count: table.skippedRows, column: table.keyColumn })}</p>}
              <label style={styles.label} htmlFor="data-binding-key">{translate('dataBinding.keyColumnLabel')}</label>
              <select id="data-binding-key" style={styles.input} value={table.keyColumn} onChange={(event) => changeKeyColumn(event.target.value)}>
                {table.columns.map((column) => <option key={column} value={column}>{column}</option>)}
              </select>
            </>
          )}
        </section>
        {table && source && (
          <section style={styles.section} aria-label={translate('dataBinding.linkHeading')}>
            <h3 style={styles.heading}>{translate('dataBinding.linkHeading')}</h3>
            {!selection && <p style={styles.hint}>{translate('dataBinding.noSelection')}</p>}
            <label style={styles.label} htmlFor="data-binding-row">{translate('dataBinding.rowLabel')}</label>
            <select id="data-binding-row" style={styles.input} value={rowKey} onChange={(event) => setRowKey(event.target.value)}>
              {table.rows.map((row) => <option key={row.key} value={row.key}>{row.key}</option>)}
            </select>
            <button type="button" style={{ ...styles.button, ...styles.primary }} disabled={!selection || !rowKey} onClick={linkRow}>
              {translate('dataBinding.linkAction')}
            </button>
          </section>
        )}
        {table && (
          <section style={styles.section} aria-label={translate('dataBinding.bindingsHeading')}>
            <h3 style={styles.heading}>{translate('dataBinding.bindingsHeading')}</h3>
            {(!doc || doc.bindings.length === 0) && <p style={styles.hint}>{translate('dataBinding.noBindings')}</p>}
            {(doc?.bindings ?? []).map((binding) => (
              <div key={binding.shapeId} style={styles.row}>
                <span style={styles.grow} title={binding.shapeId}>{binding.shapeName ?? binding.shapeId} · {binding.key}</span>
                {binding.status === 'stale' && <span style={styles.badge}>{translate('shapeData.stale')}</span>}
                <button type="button" style={styles.button} onClick={() => unlink(binding.shapeId)}>{translate('dataBinding.unlinkAction')}</button>
              </div>
            ))}
            {(doc?.bindings.length ?? 0) > 0 && (
              <button type="button" style={styles.button} onClick={refresh}>{translate('dataBinding.refreshAction')}</button>
            )}
          </section>
        )}
        {notice && <p style={styles.notice} role="status">{notice}</p>}
      </div>
    </aside>
  );
}

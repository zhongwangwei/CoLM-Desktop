import { useEffect, useRef, useState } from 'react';
import { useApp } from '../../lib/app-state';
import { invoke, invokeJson } from '../../lib/ipc';
import { usePageContract, type UiField, type UiAction } from '../../lib/guide';
import type { HistoryOperation } from '../../lib/types';
import { Advanced, Button, Card, Field, Input, Notice, Select } from '../../components/ui';
import {
  historyKindLabel,
  historyStatusLabel,
  historyObject,
  scopedHistoryFilter,
  download,
  emptyFilter,
  historyFilter,
  canReplay,
  canCompare,
  historyPath,
  recordedRunPath,
  comparisonArgs,
  comparisonWinner,
  historySections,
  historyReferences,
  historyEvents,
  historyRecordExport,
  type HistoryRow,
  type ComparisonInput,
  type HistoryDetail,
  type Filter,
} from './history';
import './history-page.css';
type Detail = HistoryDetail;
function DetailRows({ rows, en }: { rows: HistoryRow[]; en: boolean }) {
  return (
    <dl>
      {rows.map((row, index) => (
        <div key={index} className="list-row">
          <dt>{en ? row.en : row.zh}</dt>
          <dd style={{ overflowWrap: 'anywhere', whiteSpace: 'pre-wrap' }}>
            {row.value ?? (en ? 'Not recorded' : '未记录')}
          </dd>
        </div>
      ))}
    </dl>
  );
}
type EvaluationVariable = {
  name: string;
  available: boolean;
  label_zh?: string;
  label_en?: string;
  units?: string;
};
export default function HistoryPage({ casePath }: { casePath?: string }) {
  const { state, openCase, update, navigate } = useApp();
  const en = state.language === 'en';
  const t = (zh: string, enText: string) => (en ? enText : zh);
  const [filter, setFilter] = useState<Filter>({ ...emptyFilter, casePath: casePath ?? '' }),
    [applied, setApplied] = useState<Filter>(filter),
    [offset, setOffset] = useState(0),
    [page, setPage] = useState<{ items: HistoryOperation[]; total: number }>({
      items: [],
      total: 0,
    }),
    [detail, setDetail] = useState<Detail | null>(null),
    [compare, setCompare] = useState(''),
    [result, setResult] = useState<unknown>(null),
    [comparisonInput, setComparisonInput] = useState<ComparisonInput>({
      obs: '',
      variable: '',
      from: '',
      to: '',
      spinup: '0',
      corrected: false,
    }),
    [variables, setVariables] = useState<EvaluationVariable[]>([]),
    [comparisonResult, setComparisonResult] = useState<unknown>(null),
    [busy, setBusy] = useState(false),
    [error, setError] = useState('');
  const epoch = useRef({ active: false }),
    lock = useRef(false);
  const [filtersOpen, setFiltersOpen] = useState(false),
    [detailsOpen, setDetailsOpen] = useState(false);
  const effectiveFilter = scopedHistoryFilter(filter, casePath);
  const kindOptions = [
    ...new Set([
      '',
      'run',
      'remote-run',
      'study',
      'version',
      'case',
      'check',
      'build',
      'assistant',
      ...page.items.map((item) => item.kind),
      filter.kind,
    ]),
  ];
  const statusOptions = [
    ...new Set([
      '',
      'running',
      'succeeded',
      'failed',
      'cancelled',
      'interrupted',
      'unknown',
      ...page.items.map((item) => item.status),
      filter.status,
    ]),
  ];
  useEffect(() => {
    setFilter((f) => ({ ...f, casePath: casePath ?? '' }));
    setApplied((f) => ({ ...f, casePath: casePath ?? '' }));
    setOffset(0);
  }, [casePath]);
  useEffect(() => {
    const id = { active: true };
    epoch.current = id;
    setDetail(null);
    setPage({ items: [], total: 0 });
    setCompare('');
    setVariables([]);
    setComparisonResult(null);
    setBusy(true);
    setError('');
    invokeJson<{ items: HistoryOperation[]; total: number }>('history_list', {
      filter: historyFilter(scopedHistoryFilter(applied, casePath), offset),
    })
      .then((p) => {
        if (id.active && id === epoch.current) setPage(p);
      })
      .catch((e) => {
        if (id.active && id === epoch.current) setError(String(e));
      })
      .finally(() => {
        if (id.active && id === epoch.current) setBusy(false);
      });
    return () => {
      id.active = false;
    };
  }, [applied, offset, casePath]);
  async function action(fn: () => Promise<void>) {
    if (lock.current) return;
    lock.current = true;
    setBusy(true);
    setError('');
    const id = epoch.current;
    try {
      await fn();
    } catch (e) {
      if (id.active && id === epoch.current) setError(String(e));
      throw e;
    } finally {
      lock.current = false;
      if (id.active && id === epoch.current) setBusy(false);
    }
  }
  const run = (fn: () => Promise<void>) => void action(fn).catch(() => {});
  function search() {
    historyFilter(effectiveFilter);
    setOffset(0);
    setDetail(null);
    setApplied(effectiveFilter);
  }
  async function show(id: string) {
    const token = epoch.current;
    const value = await invokeJson<Detail>('history_detail', { id });
    if (token.active && token === epoch.current) {
      setDetail(value);
      setDetailsOpen(false);
      setCompare('');
      setVariables([]);
      setComparisonResult(null);
      setResult(null);
    }
  }
  async function exportAll(format: 'csv' | 'json') {
    const content = await invoke<string>('history_export', {
      filter: historyFilter(scopedHistoryFilter(applied, casePath), 0, Number.MAX_SAFE_INTEGER),
      format,
    });
    download(content, `colm-history.${format}`, format === 'csv' ? 'text/csv' : 'application/json');
  }
  const openPath = detail ? historyPath(detail) : null,
    references = detail ? historyReferences(detail) : [];
  function exportRecord() {
    if (!detail) return;
    const record = historyRecordExport(detail);
    download(record.content, record.name, record.type);
  }
  async function openVersion() {
    if (!detail?.operation.version_id || !detail.operation.revision) return;
    const version = await invokeJson<import('../../lib/types').FrozenRevision>('model_get', {
      id: detail.operation.version_id,
      revision: Number(detail.operation.revision),
    });
    if (!navigate('versions'))
      throw new Error(t('请先保存或放弃当前修改', 'Save or discard current edits first'));
    update({ version, versionRef: version.version });
  }
  function changeComparison<K extends keyof ComparisonInput>(key: K, value: ComparisonInput[K]) {
    setComparisonInput((input) => ({
      ...input,
      [key]: value,
      ...(key === 'obs' ? { variable: '' } : {}),
    }));
    if (key === 'obs') setVariables([]);
    setComparisonResult(null);
  }
  function changeComparedRun(id: string) {
    setCompare(id);
    setVariables([]);
    setComparisonInput((input) => ({ ...input, variable: '' }));
    setComparisonResult(null);
  }
  async function pickObservation() {
    const file = await invoke<string | null>('pick_file', {
      key: 'history-observations',
      filter: 'nc',
    });
    if (file) changeComparison('obs', file);
  }
  async function loadVariables() {
    if (!detail || !compare || !comparisonInput.obs) return;
    const token = epoch.current;
    const other = await invokeJson<Detail>('history_detail', { id: compare });
    const a = recordedRunPath(detail),
      b = recordedRunPath(other);
    if (!a || !b)
      throw new Error(
        t('依据不足：缺少冻结运行副本', 'Insufficient evidence: frozen run copy is missing'),
      );
    const [left, right] = await Promise.all([
      invokeJson<EvaluationVariable[]>('evaluation_catalog', { case: a, obs: comparisonInput.obs }),
      invokeJson<EvaluationVariable[]>('evaluation_catalog', { case: b, obs: comparisonInput.obs }),
    ]);
    if (token.active && token === epoch.current) {
      const common = left.filter(
        (v) => v.available && right.some((w) => w.name === v.name && w.available),
      );
      setVariables(common);
      setComparisonInput((input) => ({ ...input, variable: common[0]?.name ?? '' }));
      setComparisonResult(null);
      if (!common.length)
        throw new Error(
          t('依据不足：无共同可配对变量', 'Insufficient evidence: no common paired variables'),
        );
    }
  }
  async function compareRuns() {
    if (!detail) return;
    const token = epoch.current;
    const response = await invokeJson(
      'history_compare',
      comparisonArgs(detail.operation.operation_id, compare, comparisonInput),
    );
    if (token.active && token === epoch.current) {
      setComparisonResult(response);
      setResult(null);
    }
  }
  const comparisonLabels: Record<keyof ComparisonInput, string> = {
    obs: t('共同观测文件', 'Shared observations'),
    variable: t('共同评估变量', 'Common evaluation variable'),
    from: t('评估开始 (UTC，留空全时段)', 'Evaluation start (UTC, blank for full period)'),
    to: t('评估结束 (UTC，留空全时段)', 'Evaluation end (UTC, blank for full period)'),
    spinup: t('跳过模型记录数', 'Model records to skip'),
    corrected: t('能量闭合校正', 'Energy closure correction'),
  };
  const compareEnabled =
    !busy &&
    !!compare &&
    !!comparisonInput.obs &&
    !!comparisonInput.variable &&
    !!detail &&
    canCompare(detail.operation);
  const labels: Record<keyof Filter, string> = {
    query: t('搜索', 'Search'),
    kind: t('类型', 'Kind'),
    casePath: t('算例路径', 'Case path'),
    versionId: t('版本', 'Version'),
    host: t('主机', 'Host'),
    status: t('状态', 'Status'),
    from: t('开始时间', 'From'),
    to: t('结束时间', 'To'),
  };
  const guideFields: UiField[] = Object.entries(effectiveFilter)
    .filter(([key]) => ['query', 'kind', 'status'].includes(key) || filtersOpen)
    .map(([key, value]) => ({
      id: `history-${key}`,
      label: labels[key as keyof Filter],
      value,
      type: key === 'kind' || key === 'status' ? 'choice' : 'text',
      options:
        key === 'kind'
          ? kindOptions.map((value) => ({
              value,
              label: value ? historyKindLabel(value, en) : t('全部类型', 'All types'),
            }))
          : key === 'status'
            ? statusOptions.map((value) => ({
                value,
                label: value ? historyStatusLabel(value, en) : t('全部状态', 'All statuses'),
              }))
            : undefined,
      disabled: busy,
      readOnly: key === 'casePath' && !!casePath,
      set: (v) => setFilter((f) => scopedHistoryFilter({ ...f, [key]: v }, casePath)),
    }));
  const guideActions: UiAction[] = [
    {
      id: 'history-search',
      label: t('查询', 'Search'),
      kind: 'click',
      enabled: !busy,
      run: search,
    },
    {
      id: 'history-export-json',
      label: t('完整导出 JSON', 'Export all JSON'),
      kind: 'commit',
      enabled: !busy,
      run: () => action(() => exportAll('json')),
    },
    {
      id: 'history-export-csv',
      label: t('完整导出 CSV', 'Export all CSV'),
      kind: 'commit',
      enabled: !busy,
      run: () => action(() => exportAll('csv')),
    },
    {
      id: 'history-previous',
      label: t('上一页', 'Previous'),
      kind: 'click',
      enabled: !busy && offset > 0,
      run: () => setOffset(Math.max(0, offset - 30)),
    },
    {
      id: 'history-next',
      label: t('下一页', 'Next'),
      kind: 'click',
      enabled: !busy && offset + 30 < page.total,
      run: () => setOffset(offset + 30),
    },
    ...page.items.map((item) => ({
      id: `history-detail-${item.operation_id}`,
      label: `${t('详情', 'Details')} ${item.operation_id}`,
      kind: 'click' as const,
      enabled: !busy,
      run: () => action(() => show(item.operation_id)),
    })),
  ];
  if (detail) {
    guideActions.push(
      {
        id: 'history-export-record',
        label: t('导出本条记录 JSON', 'Export this record JSON'),
        kind: 'commit',
        enabled: !busy,
        run: exportRecord,
      },
      ...references.map((reference, index) => ({
        id: `history-reference-${index}`,
        label: `${t(reference.zh, reference.en)} · ${reference.path}`,
        kind: 'click' as const,
        enabled: !busy,
        run: () =>
          action(async () => {
            await invoke('open_path', { path: reference.path });
          }),
      })),
    );
    guideFields.push({
      id: 'history-compare-id',
      label: t('与实际历史比较', 'Compare real history'),
      value: compare,
      type: 'choice',
      options: [
        { value: '', label: '—' },
        ...page.items
          .filter((i) => i.operation_id !== detail.operation.operation_id && canCompare(i))
          .map((i) => ({ value: i.operation_id, label: `${i.kind} · ${i.operation_id}` })),
      ],
      disabled: busy,
      set: changeComparedRun,
    });
    guideActions.push(
      {
        id: 'history-replay',
        label: t('按记录设置重跑', 'Replay recorded settings'),
        kind: 'commit',
        visible: canReplay(detail.operation),
        enabled: !busy && canReplay(detail.operation),
        run: () =>
          action(async () =>
            setResult(await invokeJson('history_replay', { id: detail.operation.operation_id })),
          ),
      },
      {
        id: 'history-compare',
        label: t('核对条件并比较', 'Check conditions and compare'),
        kind: 'click',
        enabled: compareEnabled,
        run: () => action(compareRuns),
      },
      ...detail.related.map((item) => ({
        id: `history-related-${item.operation_id}`,
        label: `${item.kind} · ${item.operation_id}`,
        kind: 'click' as const,
        enabled: !busy,
        run: () => action(() => show(item.operation_id)),
      })),
    );
    if (openPath)
      guideActions.push({
        id: 'history-open-path',
        label: t('打开路径', 'Open path'),
        kind: 'click',
        enabled: !busy,
        run: () =>
          action(async () => {
            await invoke('open_path', { path: openPath });
          }),
      });
    if (detail.operation.case_path)
      guideActions.push({
        id: 'history-open-case',
        label: t('打开算例', 'Open case'),
        kind: 'click',
        enabled: !busy,
        run: () =>
          action(async () => {
            await openCase(detail.operation.case_path!);
            navigate('case');
          }),
      });
    if (detail.operation.version_id && detail.operation.revision)
      guideActions.push({
        id: 'history-open-version',
        label: t('打开版本', 'Open version'),
        kind: 'click',
        enabled: !busy,
        run: () => action(openVersion),
      });
  }
  if (detail) {
    for (const key of ['obs', 'from', 'to', 'spinup'] as const)
      guideFields.push({
        id: `history-evaluation-${key}`,
        label: comparisonLabels[key],
        value: comparisonInput[key],
        type: key === 'spinup' ? 'number' : 'text',
        disabled: busy,
        set: (v) => changeComparison(key, v),
      });
    guideFields.push(
      {
        id: 'history-evaluation-variable',
        label: comparisonLabels.variable,
        value: comparisonInput.variable,
        type: 'choice',
        options: [
          { value: '', label: '—' },
          ...variables.map((v) => ({
            value: v.name,
            label: (en ? v.label_en : v.label_zh) ?? v.name,
          })),
        ],
        disabled: busy,
        set: (v) => changeComparison('variable', v),
      },
      {
        id: 'history-evaluation-corrected',
        label: comparisonLabels.corrected,
        value: String(comparisonInput.corrected),
        type: 'switch',
        disabled: busy,
        set: (v) => changeComparison('corrected', v === 'true'),
      },
    );
    guideActions.push(
      {
        id: 'history-pick-observation',
        label: t('选择观测文件', 'Choose observations'),
        kind: 'click',
        enabled: !busy,
        run: () => action(pickObservation),
      },
      {
        id: 'history-evaluation-catalog',
        label: t('读取共同可用变量', 'Read common variables'),
        kind: 'click',
        enabled: !busy && !!compare && !!comparisonInput.obs,
        run: () => action(loadVariables),
      },
    );
  }
  guideActions.push({
    id: 'history-filters',
    label: t(filtersOpen ? '收起筛选' : '更多筛选', filtersOpen ? 'Hide filters' : 'More filters'),
    kind: 'click',
    enabled: !busy,
    run: () => setFiltersOpen(!filtersOpen),
  });
  if (detail)
    guideActions.push({
      id: 'history-advanced',
      label: t('依据、比较与完整记录', 'Evidence, comparison and full record'),
      kind: 'click',
      enabled: !busy,
      run: () => setDetailsOpen(!detailsOpen),
    });
  const summaryActions = new Set([
    'history-export-record',
    'history-open-path',
    'history-open-case',
    'history-open-version',
    'history-replay',
  ]);
  usePageContract({
    title: t('历史记录', 'History'),
    step: 'history',
    status: error || String(page.total),
    fields: guideFields.filter(
      (field) =>
        detailsOpen ||
        (!field.id.startsWith('history-evaluation-') && field.id !== 'history-compare-id'),
    ),
    actions: guideActions.filter(
      (item) =>
        detailsOpen ||
        summaryActions.has(item.id) ||
        (!item.id.startsWith('history-reference-') &&
          !item.id.startsWith('history-related-') &&
          !['history-compare', 'history-pick-observation', 'history-evaluation-catalog'].includes(
            item.id,
          )),
    ),
  });

  return (
    <div className="page-stack history-page">
      <header className="history-heading">
        <div>
          <h1>{t('历史记录', 'History')}</h1>
          <p className="muted">
            {t(
              '追溯每次运行、研究和修改。选择记录查看依据与后续操作。',
              'Trace runs, research and changes. Select a record to inspect its evidence and available actions.',
            )}
          </p>
        </div>
      </header>
      <Card className="history-filters">
        <form
          onSubmit={(event) => {
            event.preventDefault();
            try {
              search();
            } catch (e) {
              setError(String(e));
            }
          }}
        >
          <div className="history-filter-main">
            <Field label={labels.query}>
              <Input
                disabled={busy}
                value={filter.query}
                placeholder={t('搜索算例、任务或版本…', 'Search cases, tasks or versions…')}
                onChange={(e) => setFilter((f) => ({ ...f, query: e.target.value }))}
              />
            </Field>
            <Field label={labels.kind}>
              <Select
                disabled={busy}
                value={filter.kind}
                onChange={(e) => setFilter((f) => ({ ...f, kind: e.target.value }))}
              >
                {kindOptions.map((kind) => (
                  <option key={kind} value={kind}>
                    {kind ? historyKindLabel(kind, en) : t('全部类型', 'All types')}
                  </option>
                ))}
              </Select>
            </Field>
            <Field label={labels.status}>
              <Select
                disabled={busy}
                value={filter.status}
                onChange={(e) => setFilter((f) => ({ ...f, status: e.target.value }))}
              >
                {statusOptions.map((status) => (
                  <option key={status} value={status}>
                    {status ? historyStatusLabel(status, en) : t('全部状态', 'All statuses')}
                  </option>
                ))}
              </Select>
            </Field>
            <Button type="submit" variant="primary" disabled={busy}>
              {t('查询', 'Search')}
            </Button>
          </div>
          <Advanced
            title={t('更多筛选', 'More filters')}
            open={filtersOpen}
            onOpenChange={setFiltersOpen}
          >
            <div className="form-grid">
              {(['casePath', 'versionId', 'host', 'from', 'to'] as const).map((key) => (
                <Field key={key} label={labels[key]}>
                  <Input
                    disabled={busy}
                    readOnly={key === 'casePath' && !!casePath}
                    type={key === 'from' || key === 'to' ? 'datetime-local' : 'text'}
                    value={effectiveFilter[key]}
                    onChange={(e) =>
                      setFilter((f) =>
                        scopedHistoryFilter({ ...f, [key]: e.target.value }, casePath),
                      )
                    }
                  />
                </Field>
              ))}
            </div>
          </Advanced>
        </form>
        {casePath && (
          <p className="muted history-scoped">
            {t('当前算例', 'Current case')}: {casePath}
          </p>
        )}
      </Card>
      {error && <Notice tone="error">{error}</Notice>}
      <div className="history-workbench">
        <section className="history-list" aria-label={t('历史记录列表', 'History records')}>
          <div className="history-list-toolbar">
            <span className="muted">
              {busy ? t('正在读取…', 'Loading…') : `${page.total} ${t('条记录', 'records')}`}
            </span>
            <div className="button-row">
              {(['csv', 'json'] as const).map((format) => (
                <Button key={format} disabled={busy} onClick={() => run(() => exportAll(format))}>
                  {t('导出', 'Export')} {format.toUpperCase()}
                </Button>
              ))}
            </div>
          </div>
          <div
            className="history-table-scroll"
            tabIndex={0}
            role="region"
            aria-label={t('可滚动的记录表', 'Scrollable record table')}
          >
            <table className="history-table">
              <thead>
                <tr>
                  {[
                    t('时间', 'Time'),
                    t('类型', 'Type'),
                    t('对象 / 操作', 'Object / action'),
                    t('状态', 'Status'),
                    t('详情', 'Details'),
                  ].map((label) => (
                    <th key={label} scope="col">
                      {label}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {page.items.map((item) => (
                  <tr
                    key={item.operation_id}
                    className={
                      detail?.operation.operation_id === item.operation_id ? 'is-selected' : ''
                    }
                  >
                    <td>
                      {item.time_ms == null ? (
                        '—'
                      ) : (
                        <time dateTime={new Date(item.time_ms).toISOString()}>
                          {new Date(item.time_ms).toLocaleDateString()}
                          <br />
                          {new Date(item.time_ms).toLocaleTimeString([], {
                            hour: '2-digit',
                            minute: '2-digit',
                          })}
                        </time>
                      )}
                    </td>
                    <td>{historyKindLabel(item.kind, en)}</td>
                    <td className="history-object" title={item.case_path ?? item.operation_id}>
                      <strong>{historyObject(item)}</strong>
                      <small>
                        {item.phase} ·{' '}
                        {item.host ??
                          (item.location === 'remote'
                            ? t('远程', 'Remote')
                            : item.location === 'local'
                              ? t('本机', 'Local')
                              : t('位置未记录', 'Location not recorded'))}
                      </small>
                    </td>
                    <td>
                      <span
                        className={`history-status history-status-${item.needs_reconciliation ? 'pending' : item.status}`}
                      >
                        {item.needs_reconciliation
                          ? t('待核对', 'Needs reconciliation')
                          : historyStatusLabel(item.status, en)}
                      </span>
                    </td>
                    <td>
                      <Button
                        variant="ghost"
                        aria-pressed={detail?.operation.operation_id === item.operation_id}
                        aria-label={`${t('查看记录', 'View record')} ${item.operation_id}`}
                        disabled={busy}
                        onClick={() => run(() => show(item.operation_id))}
                      >
                        {t('查看', 'View')}
                      </Button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {!page.items.length && !busy && (
            <Notice>
              {error
                ? t('记录读取失败，请重试查询。', 'Unable to load records. Retry the search.')
                : t(
                    '无匹配记录。可调整筛选条件；尚未记录的操作不会显示。',
                    'No matching records. Adjust the filters; unrecorded operations are not listed.',
                  )}
            </Notice>
          )}
          <div className="history-pagination">
            <Button
              disabled={busy || offset === 0}
              onClick={() => setOffset(Math.max(0, offset - 30))}
            >
              {t('上一页', 'Previous')}
            </Button>
            <span>
              {page.total ? offset + 1 : 0}–{offset + page.items.length} / {page.total}
            </span>
            <Button
              disabled={busy || offset + 30 >= page.total}
              onClick={() => setOffset(offset + 30)}
            >
              {t('下一页', 'Next')}
            </Button>
          </div>
        </section>
        <aside className="history-inspector" aria-label={t('记录详情', 'Record details')}>
          {detail ? (
            <Card title={historyKindLabel(detail.operation.kind, en)}>
              <h3>{historyObject(detail.operation)}</h3>
              <span
                className={`history-status history-status-${detail.operation.needs_reconciliation ? 'pending' : detail.operation.status}`}
              >
                {detail.operation.needs_reconciliation
                  ? t('待核对', 'Needs reconciliation')
                  : historyStatusLabel(detail.operation.status, en)}
              </span>
              <DetailRows
                en={en}
                rows={[
                  { zh: '算例', en: 'Case', value: detail.operation.case_path },
                  {
                    zh: '版本',
                    en: 'Version',
                    value: detail.operation.version_id
                      ? `${detail.operation.version_id} · r${detail.operation.revision ?? '—'}`
                      : null,
                  },
                  {
                    zh: '位置',
                    en: 'Location',
                    value: detail.operation.host ?? detail.operation.location,
                  },
                  {
                    zh: '开始',
                    en: 'Started',
                    value:
                      detail.operation.started_ms == null
                        ? null
                        : new Date(detail.operation.started_ms).toLocaleString(),
                  },
                  {
                    zh: '结束',
                    en: 'Finished',
                    value:
                      detail.operation.finished_ms == null
                        ? null
                        : new Date(detail.operation.finished_ms).toLocaleString(),
                  },
                  { zh: '操作标识', en: 'Operation ID', value: detail.operation.operation_id },
                ]}
              />
              {detail.operation.needs_reconciliation && (
                <Notice tone="warning">
                  {t(
                    '需核对真实进程状态，不能视为完成或自动重跑。',
                    'Reconcile the real process before treating this record as complete or replaying it.',
                  )}
                </Notice>
              )}
              <div className="button-row">
                {detail.operation.case_path && (
                  <Button
                    variant="primary"
                    disabled={busy}
                    onClick={() =>
                      run(async () => {
                        await openCase(detail.operation.case_path!);
                        navigate('case');
                      })
                    }
                  >
                    {t('打开算例', 'Open case')}
                  </Button>
                )}
                {openPath && (
                  <Button
                    disabled={busy}
                    onClick={() =>
                      run(async () => {
                        await invoke('open_path', { path: openPath });
                      })
                    }
                  >
                    {t('打开路径', 'Open path')}
                  </Button>
                )}
                {detail.operation.version_id && detail.operation.revision && (
                  <Button disabled={busy} onClick={() => run(openVersion)}>
                    {t('打开版本', 'Open version')}
                  </Button>
                )}
                {canReplay(detail.operation) && (
                  <Button
                    disabled={busy}
                    onClick={() =>
                      run(async () =>
                        setResult(
                          await invokeJson('history_replay', { id: detail.operation.operation_id }),
                        ),
                      )
                    }
                  >
                    {t('按记录设置重跑', 'Replay recorded settings')}
                  </Button>
                )}
                <Button disabled={busy} onClick={exportRecord}>
                  {t('导出本条记录 JSON', 'Export this record JSON')}
                </Button>
              </div>
              {result != null && (
                <Notice>
                  {t(
                    '已返回重跑结果，可在下方完整记录中查看。',
                    'Replay returned a result. Inspect it in the full record below.',
                  )}
                </Notice>
              )}
              <Advanced
                title={t('依据、比较与完整记录', 'Evidence, comparison and full record')}
                open={detailsOpen}
                onOpenChange={setDetailsOpen}
              >
                <div className="card-grid">
                  {historySections(detail).map((section) => (
                    <Card key={section.en} title={t(section.zh, section.en)}>
                      <DetailRows rows={section.rows} en={en} />
                      {section.en === 'Source provenance' &&
                        detail.operation.version_id &&
                        detail.operation.revision && (
                          <Button disabled={busy} onClick={() => run(openVersion)}>
                            {t('查看源码版本', 'View source revision')}
                          </Button>
                        )}
                    </Card>
                  ))}
                </div>
                {detail.operation.needs_reconciliation && (
                  <Notice tone="warning">
                    {t(
                      '记录尚待核对真实进程状态，不视为完成。',
                      'The real process still needs reconciliation; this record is not treated as complete.',
                    )}
                  </Notice>
                )}
                <h3>{t('依据文件', 'Evidence files')}</h3>
                <p className="muted">
                  {t(
                    '大型清单保留在所列文件中，本页仅显示记录中的路径和标识。',
                    'Large manifests remain in the referenced files; this page displays their recorded paths and identities.',
                  )}
                </p>
                <div className="button-row">
                  {references.map((reference) => (
                    <Button
                      key={reference.path}
                      disabled={busy}
                      onClick={() =>
                        run(async () => {
                          await invoke('open_path', { path: reference.path });
                        })
                      }
                    >
                      {t(reference.zh, reference.en)} · {reference.path}
                    </Button>
                  ))}
                </div>
                <Field label={t('与实际历史比较', 'Compare real history')}>
                  <Select
                    disabled={busy}
                    value={compare}
                    onChange={(e) => changeComparedRun(e.target.value)}
                  >
                    <option value="">—</option>
                    {page.items
                      .filter(
                        (i) => i.operation_id !== detail.operation.operation_id && canCompare(i),
                      )
                      .map((i) => (
                        <option key={i.operation_id} value={i.operation_id}>
                          {i.kind} · {i.operation_id}
                        </option>
                      ))}
                  </Select>
                </Field>
                <div className="form-grid">
                  {(['obs', 'from', 'to', 'spinup'] as const).map((key) => (
                    <Field key={key} label={comparisonLabels[key]}>
                      <Input
                        disabled={busy}
                        type={
                          key === 'spinup'
                            ? 'number'
                            : key === 'from' || key === 'to'
                              ? 'datetime-local'
                              : 'text'
                        }
                        min={key === 'spinup' ? 0 : undefined}
                        value={comparisonInput[key]}
                        onChange={(e) => changeComparison(key, e.target.value)}
                      />
                    </Field>
                  ))}
                  <Field label={comparisonLabels.variable}>
                    <Select
                      disabled={busy}
                      value={comparisonInput.variable}
                      onChange={(e) => changeComparison('variable', e.target.value)}
                    >
                      <option value="">—</option>
                      {variables.map((v) => (
                        <option key={v.name} value={v.name}>
                          {(en ? v.label_en : v.label_zh) ?? v.name} {v.units ?? ''}
                        </option>
                      ))}
                    </Select>
                  </Field>
                  <Field label={comparisonLabels.corrected}>
                    <input
                      type="checkbox"
                      disabled={busy}
                      checked={comparisonInput.corrected}
                      onChange={(e) => changeComparison('corrected', e.target.checked)}
                    />
                  </Field>
                </div>
                <div className="button-row">
                  <Button disabled={busy} onClick={() => run(pickObservation)}>
                    {t('选择观测文件', 'Choose observations')}
                  </Button>
                  <Button
                    disabled={busy || !compare || !comparisonInput.obs}
                    onClick={() => run(loadVariables)}
                  >
                    {t('读取共同可用变量', 'Read common variables')}
                  </Button>
                  <Button disabled={!compareEnabled} onClick={() => run(compareRuns)}>
                    {t('核对条件并比较', 'Check conditions and compare')}
                  </Button>
                </div>
                {comparisonResult != null && (
                  <>
                    <Notice tone={comparisonWinner(comparisonResult) ? 'success' : 'warning'}>
                      {comparisonWinner(comparisonResult) === 'tie'
                        ? t(
                            '相同评估条件下 RMSE 相同',
                            'Equal RMSE under verified matching conditions',
                          )
                        : comparisonWinner(comparisonResult)
                          ? `${t('相同评估条件下 RMSE 较低', 'Lower RMSE under verified matching conditions')}: ${comparisonWinner(comparisonResult) === 'a' ? detail.operation.operation_id : compare}`
                          : t(
                              '依据不足，不能判断哪个版本更好',
                              'Insufficient evidence to rank these versions',
                            )}
                    </Notice>
                    <pre>{JSON.stringify(comparisonResult, null, 2)}</pre>
                  </>
                )}
                {result != null && <pre>{JSON.stringify(result, null, 2)}</pre>}
                <h3>{t('关联记录', 'Related operations')}</h3>
                {detail.related.map((item) => (
                  <Button
                    key={item.operation_id}
                    disabled={busy}
                    onClick={() => run(() => show(item.operation_id))}
                  >
                    {item.kind} · {item.operation_id}
                  </Button>
                ))}
                <h3>{t('事件', 'Events')}</h3>
                <div>
                  {historyEvents(detail).map((rows, index) => (
                    <Card key={index} title={`${t('事件', 'Event')} ${index + 1}`}>
                      <DetailRows rows={rows} en={en} />
                    </Card>
                  ))}
                </div>
              </Advanced>
            </Card>
          ) : (
            <Card title={t('选择一条记录', 'Select a record')}>
              <p className="muted">
                {t(
                  '查看运行设置、源码版本、日志及关联操作。',
                  'Inspect recorded settings, source revisions, logs and related operations.',
                )}
              </p>
            </Card>
          )}
        </aside>
      </div>
    </div>
  );
}

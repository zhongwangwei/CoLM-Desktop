import { useEffect, useRef, useState } from 'react';
import { useApp } from '../../lib/app-state';
import { invoke, invokeJson } from '../../lib/ipc';
import { usePageContract, type UiField, type UiAction } from '../../lib/guide';
import type { CaseEntry, HistoryOperation, Server, WizardSeed } from '../../lib/types';
import {
  Advanced,
  Button,
  Card,
  ContextMenu,
  Dialog,
  Field,
  Input,
  Notice,
  Select,
} from '../../components/ui';
import {
  historyKindLabel,
  historyActionLabel,
  historyDurationLabel,
  historyInternal,
  historySubject,
  historyVersionLabel,
  projectSummary,
  projectTimeline,
  statusCountsLabel,
  historyStatusLabel,
  appLocale,
  historyObject,
  scopedHistoryFilter,
  download,
  emptyFilter,
  historyFilter,
  canReplay,
  canDeleteHistory,
  simulationDraft,
  historyStateAfterDirectoryDeletion,
  type HistoryDeletePlan,
  canCompare,
  historyPath,
  recordedRunPath,
  comparisonArgs,
  comparisonWinner,
  historySections,
  historyReferences,
  historyEvents,
  historyRecordExport,
  replayVerificationText,
  type HistoryRow,
  type ComparisonInput,
  type HistoryDetail,
  type Filter,
} from './history';
import './history-page.css';
type Detail = HistoryDetail;
type ClearPlan = {
  records: number;
  unconfirmed: number;
  unconfirmedRemote: number;
  cases: { path: string; bytes: number }[];
  remoteCases: number;
  skipped: string[];
};
type ClearResult = { removed: number; kept: number; deleted: string[]; failed: string[] };
type ProjectDeletePlan = {
  records: number;
  path: string;
  bytes: number;
  host: string | null;
  remotePath: string | null;
  reason: string | null;
};
type ProjectDeleteResult = {
  removed: number;
  path: string;
  host: string | null;
  remotePath: string | null;
};
const sizeText = (bytes: number) =>
  bytes >= 1024 ** 3
    ? `${(bytes / 1024 ** 3).toFixed(1)} GB`
    : bytes >= 1024 ** 2
      ? `${(bytes / 1024 ** 2).toFixed(1)} MB`
      : `${Math.max(1, Math.round(bytes / 1024))} KB`;
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
  const versionNames = Object.fromEntries(
    (state.versions ?? []).map((revision) => [revision.version.id, revision.name]),
  );
  const en = state.language === 'en';
  const t = (zh: string, enText: string) => (en ? enText : zh);
  const [filter, setFilter] = useState<Filter>({ ...emptyFilter, casePath: casePath ?? '' }),
    [applied, setApplied] = useState<Filter>(filter),
    [offset, setOffset] = useState(0),
    [page, setPage] = useState<{ items: HistoryOperation[]; total: number }>({
      items: [],
      total: 0,
    }),
    [group, setGroup] = useState<HistoryOperation | null>(null),
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
  const [deletePlan, setDeletePlan] = useState<HistoryDeletePlan | null>(null);
  const [deletion, setDeletion] = useState<{
    item: HistoryOperation;
    files: boolean;
    wholeCase?: boolean;
    plan: HistoryDeletePlan | null;
  } | null>(null);
  const [deleteName, setDeleteName] = useState('');
  const [clearPlan, setClearPlan] = useState<ClearPlan | null>(null),
    [clearCases, setClearCases] = useState(false),
    [clearUnconfirmed, setClearUnconfirmed] = useState(false),
    [clearOutcome, setClearOutcome] = useState<ClearResult | null>(null),
    // 从项目卡片右键进来时只清这一个项目；为空时是整页的“清空历史记录”。
    [clearProject, setClearProject] = useState<{ path: string; name: string } | null>(null);
  // 项目卡片右键的"删除记录和项目文件夹"：右键时先核对（服务器项目还要到服务器上预检），确认框里再输入文件夹名。
  const [projectPlan, setProjectPlan] = useState<ProjectDeletePlan | null>(null),
    [projectDeletion, setProjectDeletion] = useState<{
      project: { path: string; name: string };
      plan: ProjectDeletePlan;
    } | null>(null),
    [projectConfirm, setProjectConfirm] = useState(''),
    [projectOutcome, setProjectOutcome] = useState<ProjectDeleteResult | null>(null);
  const deletionPath = deletion?.wholeCase ? deletion.plan?.casePath : deletion?.plan?.path;
  const directoryName =
    deletionPath
      ?.replace(/[\\/]+$/, '')
      .split(/[\\/]/)
      .pop() ?? '';
  const deleteConfirmed = !deletion?.wholeCase || (!!directoryName && deleteName === directoryName);
  const menuRequest = useRef<object | null>(null);
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
    menuRequest.current = null;
    setDeletePlan(null);
    setDeletion(null);
    setDetail(null);
    setGroup(null);
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
        if (id.active && id === epoch.current) {
          if (!p.items.length && offset > 0) {
            setOffset(Math.max(0, Math.ceil(p.total / 30) - 1) * 30);
          } else setPage(p);
        }
      })
      .catch((e) => {
        if (id.active && id === epoch.current) setError(String(e));
      })
      .finally(() => {
        if (id.active && id === epoch.current) setBusy(false);
      });
    return () => {
      id.active = false;
      menuRequest.current = null;
    };
  }, [applied, offset, casePath]);
  async function action<T>(fn: () => Promise<T>) {
    if (lock.current) return;
    lock.current = true;
    setBusy(true);
    setError('');
    const id = epoch.current;
    try {
      return await fn();
    } catch (e) {
      if (id.active && id === epoch.current) setError(String(e));
      throw e;
    } finally {
      lock.current = false;
      if (id.active && id === epoch.current) setBusy(false);
    }
  }
  const run = (fn: () => Promise<unknown>) => void action(fn).catch(() => {});
  function search() {
    try {
      historyFilter(effectiveFilter);
    } catch {
      throw new Error(t('开始时间晚于结束时间', 'The start is after the end'));
    }
    setOffset(0);
    setDetail(null);
    setGroup(null);
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
  // From a project's history to where it is simulated: a multi-site project's batch run page,
  // a single case's simulation settings. Not the case list, which is one more step away.
  async function openProject(path: string) {
    const cases = await invokeJson<CaseEntry[]>('list_cases', { root: path });
    const switching = path !== state.root ? { selectedCase: null } : {};
    if (cases.length === 1) {
      update({ root: path, cases, projectCasesOpen: false, ...switching });
      await openCase(cases[0].dir, 'settings');
      return;
    }
    // A multi-site project reopens on the page that creating its stations led to: the batch run
    // page with all of them, showing how they ended. Back from there is the wizard with every
    // step already filled in from what the project was created with.
    const seed = await invokeJson<WizardSeed>('project_wizard_seed', { root: path });
    const server = (
      await invokeJson<{ servers: Server[] }>('remote_config', {}).catch(() => ({
        servers: [] as Server[],
      }))
    ).servers.find((entry) => entry.host === seed.form.host);
    if (!navigate('project')) return;
    update({
      root: path,
      cases,
      projectCasesOpen: true,
      projectRunSettings: {
        root: path,
        cases: cases.map((entry) => entry.dir),
        batch: true,
        host: seed.form.host ?? '',
        threads: '1',
        ranks: '1',
        workers: '1',
        scheduler: server?.scheduler || 'auto',
        cpus: server && server.cpus > 0 ? String(server.cpus) : '',
      },
      newCaseSeed: {
        form: seed.form,
        sites: seed.sites,
        caseValues: seed.case_values,
        step: 4,
        ownData: seed.form.kind === 'site',
        multiSiteEnabled: true,
        multiSiteMode: seed.multi_site_mode,
        multiSiteRepeat: seed.multi_site_repeat,
        siteDrafts: seed.drafts,
        recordDraft: false,
      },
      ...switching,
    });
  }
  // 未完成的新建模拟：回到向导里停下的那一步，接着记在同一条记录上。
  function resumeSimulation(operation: HistoryOperation) {
    const seed = simulationDraft(operation);
    if (!seed || !navigate('new-case')) return;
    update({
      newCaseSeed: {
        ...seed,
        draftId: operation.operation_id,
      },
      simulationReturn: null,
    });
  }
  function closeDetails() {
    setGroup(null);
    setDetail(null);
    setResult(null);
    setCompare('');
    setVariables([]);
    setComparisonResult(null);
  }
  async function showItem(item: HistoryOperation) {
    setGroup(item.run_group || item.project_group ? item : null);
    setDetail(null);
    setResult(null);
    if (!item.run_group && !item.project_group) await show(item.operation_id);
  }
  function deleteAllowed(item: HistoryOperation) {
    return (
      canDeleteHistory(item) &&
      !(
        item.run_group &&
        Object.values(state.running ?? {}).some(
          (run) => run.running && run.run_id === item.run_group!.run_id,
        )
      )
    );
  }
  function menuChanged(item: HistoryOperation, open: boolean) {
    const request = {};
    menuRequest.current = open ? request : null;
    setDeletePlan(null);
    setProjectPlan(null);
    if (open && item.project_group && !busy) {
      const token = epoch.current,
        path = item.project_group.path;
      void invokeJson<ProjectDeletePlan>('history_project_delete_plan', { project: path })
        .catch((e) => ({
          records: 0,
          path,
          bytes: 0,
          host: null,
          remotePath: null,
          reason: String(e),
        }))
        .then((plan) => {
          if (token.active && token === epoch.current && menuRequest.current === request)
            setProjectPlan(plan);
        });
      return;
    }
    if (!open || item.run_group || item.project_group || !deleteAllowed(item) || busy) return;
    const token = epoch.current;
    void invokeJson<HistoryDeletePlan>('history_delete_plan', { id: item.operation_id })
      .then((plan) => {
        if (token.active && token === epoch.current && menuRequest.current === request)
          setDeletePlan(plan);
      })
      .catch((e) => {
        if (token.active && token === epoch.current && menuRequest.current === request)
          setDeletePlan({
            id: item.operation_id,
            path: null,
            host: null,
            reason: String(e),
            casePath: null,
            caseHost: null,
            caseReason: String(e),
          });
      });
  }
  function deleteItems(item: HistoryOperation) {
    const plan = deletePlan?.id === item.operation_id ? deletePlan : null;
    const allowed = deleteAllowed(item);
    if (item.project_group) {
      // 一个多站点项目可有上千条记录，逐条删不现实。只删记录沿用清空历史的确认框；连文件夹一起删的，
      // 右键时已核对过（项目文件夹里只有它自己的、空闲的算例）。
      const { path, name } = item.project_group;
      const folder = projectPlan?.path === path ? projectPlan : null;
      return [
        {
          label: t('仅删除记录…', 'Delete records only…'),
          danger: true,
          disabled: busy,
          onSelect: () => {
            setError('');
            run(() => openClear({ path, name }));
          },
        },
        {
          label: !folder
            ? t('正在核对项目文件夹…', 'Checking the project folder…')
            : folder.reason
              ? folder.reason
              : t('删除记录和项目文件夹…', 'Delete records and project folder…'),
          danger: true,
          disabled: busy || !folder || !!folder.reason,
          onSelect: () => {
            if (!folder || folder.reason) return;
            setError('');
            setProjectConfirm('');
            setProjectOutcome(null);
            setProjectDeletion({ project: { path, name }, plan: folder });
          },
        },
      ];
    }
    if (item.run_group)
      return [
        {
          label: t('仅删除记录', 'Delete record only'),
          danger: true,
          disabled: busy || !allowed,
          onSelect: () => {
            if (deleteAllowed(item)) setDeletion({ item, files: false, plan: null });
          },
        },
      ];
    return [
      {
        label: t('仅删除记录', 'Delete record only'),
        danger: true,
        disabled: busy || !allowed,
        onSelect: () => {
          setError('');
          setDeletion({ item, files: false, plan: null });
        },
      },
      {
        label: !allowed
          ? t('运行中或待核对的记录不能删除', 'Active or unreconciled records cannot be deleted')
          : !plan
            ? t('正在核对目录…', 'Checking directory…')
            : plan.path && !plan.reason
              ? t('删除记录及本次运行目录…', 'Delete record and this run directory…')
              : plan.reason || t('没有可删除的目录', 'No deletable directory'),
        danger: true,
        disabled: busy || !allowed || !plan?.path || !!plan.reason,
        onSelect: () => {
          if (plan?.path && !plan.reason) {
            setError('');
            setDeletion({ item, files: true, plan });
          }
        },
      },
      {
        label: !allowed
          ? t('运行中或待核对的算例不能删除', 'Active or unreconciled cases cannot be deleted')
          : !plan
            ? t('正在核对算例目录…', 'Checking case directory…')
            : plan.casePath && !plan.caseReason
              ? t('删除整个算例文件夹…', 'Delete entire case folder…')
              : plan.caseReason || t('没有可删除的算例目录', 'No deletable case directory'),
        danger: true,
        disabled: busy || !allowed || !plan?.casePath || !!plan.caseReason,
        onSelect: () => {
          if (plan?.casePath && !plan.caseReason) {
            setError('');
            setDeleteName('');
            setDeletion({ item, files: true, wholeCase: true, plan });
          }
        },
      },
    ];
  }
  async function deleteRecord() {
    if (
      !deletion ||
      !deleteAllowed(deletion.item) ||
      !deleteConfirmed ||
      (deletion.files && !deletionPath)
    )
      return;
    const target = deletion;
    const token = epoch.current;
    const response = await invokeJson<{ removed: boolean; path?: string; host?: string }>(
      'history_delete',
      {
        id: target.item.operation_id,
        files: target.files,
        ...(target.item.run_group ? { group: true } : {}),
        ...(target.files ? { expectedPath: deletionPath } : {}),
        ...(target.wholeCase ? { wholeCase: true } : {}),
      },
    );
    if (!response.removed)
      throw new Error(
        t('记录未删除，请刷新后重试。', 'Record was not deleted. Refresh and retry.'),
      );
    if (target.files && response.path) {
      const path = response.path;
      update((current) => historyStateAfterDirectoryDeletion(current, path, response.host));
    }
    if (!token.active || token !== epoch.current) return;
    setDeletion(null);
    setDetail(null);
    setGroup(null);
    setCompare('');
    setVariables([]);
    setComparisonResult(null);
    setResult(null);
    setApplied((current) => ({ ...current }));
  }
  async function deleteProject() {
    if (!projectDeletion || projectConfirm !== projectDeletion.project.name) return;
    const { project, plan } = projectDeletion;
    const result = await invokeJson<ProjectDeleteResult>('history_project_delete', {
      project: project.path,
      expectedPath: plan.path,
      expectedRemote: plan.remotePath,
    });
    update((current) => historyStateAfterDirectoryDeletion(current, result.path, undefined));
    closeDetails();
    setProjectDeletion(null);
    setProjectOutcome(result);
    setOffset(0);
    setApplied((current) => ({ ...current }));
  }
  async function openClear(project?: { path: string; name: string }) {
    const plan = await invokeJson<ClearPlan>(
      'history_clear_plan',
      project ? { project: project.path } : {},
    );
    setClearProject(project ?? null);
    setClearCases(false);
    setClearUnconfirmed(false);
    setClearOutcome(null);
    setClearPlan(plan);
  }
  async function clearHistory() {
    if (!clearPlan) return;
    const result = await invokeJson<ClearResult>('history_clear', {
      cases: clearCases && !clearProject ? clearPlan.cases.map((item) => item.path) : [],
      unconfirmed: clearUnconfirmed,
      ...(clearProject ? { project: clearProject.path } : {}),
    });
    for (const path of result.deleted)
      update((current) => historyStateAfterDirectoryDeletion(current, path, undefined));
    closeDetails();
    setClearPlan(null);
    setClearOutcome(result);
    setOffset(0);
    setApplied((current) => ({ ...current }));
  }
  async function exportAll(format: 'csv' | 'json') {
    const content = await invoke<string>('history_export', {
      filter: historyFilter(scopedHistoryFilter(applied, casePath), 0, Number.MAX_SAFE_INTEGER),
      format,
    });
    return await download(
      content,
      `colm-history.${format}`,
      format === 'csv' ? 'text/csv' : 'application/json',
    );
  }
  const openPath = detail ? historyPath(detail) : null,
    references = detail ? historyReferences(detail) : [];
  async function exportGroup() {
    if (!group?.run_group) return;
    const members = await Promise.all(
      group.run_group.members.map((member) =>
        invokeJson<Detail>('history_detail', { id: member.operation_id }),
      ),
    );
    return download(
      JSON.stringify({ operation: group, members }, null, 2),
      `colm-history-${group.run_group.run_id.replace(/[^a-zA-Z0-9_-]/g, '_')}.json`,
      'application/json',
    );
  }
  async function exportRecord() {
    if (!detail) return;
    const record = historyRecordExport(detail);
    return await download(record.content, record.name, record.type);
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
    includeRoutine: t('显示自动例行记录', 'Show routine records'),
  };
  const guideFields: UiField[] = Object.entries(effectiveFilter)
    .filter(([key]) => ['query', 'kind', 'status'].includes(key) || filtersOpen)
    .map(([key, value]) => ({
      id: `history-${key}`,
      label: labels[key as keyof Filter],
      value: String(value ?? ''),
      type:
        key === 'kind' || key === 'status'
          ? 'choice'
          : key === 'includeRoutine'
            ? 'switch'
            : 'text',
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
      set: (v) =>
        setFilter((f) =>
          scopedHistoryFilter(
            { ...f, [key]: key === 'includeRoutine' ? v === 'true' : v },
            casePath,
          ),
        ),
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
      id: 'history-clear',
      label: t('清空历史记录', 'Clear history'),
      kind: 'click',
      enabled: !busy,
      run: () => action(() => openClear()),
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
      run: () => action(() => showItem(item)),
    })),
  ];
  if (group?.project_group) {
    const path = group.project_group.path;
    guideActions.push({
      id: 'history-open-project',
      label: t('打开这个项目', 'Open this project'),
      kind: 'click',
      enabled: !busy,
      run: () => action(() => openProject(path)),
    });
  }
  if (group?.run_group) {
    guideActions.push(
      ...group.run_group.members.map((member) => ({
        id: `history-member-${member.operation_id}`,
        label: `${t('站点详情', 'Station details')} ${historyObject(member, en)}`,
        kind: 'click' as const,
        enabled: !busy,
        run: () => action(() => show(member.operation_id)),
      })),
      {
        id: 'history-export-run',
        label: t('导出本次运行 JSON', 'Export this run JSON'),
        kind: 'click',
        enabled: !busy,
        run: () => action(exportGroup),
      },
    );
  }
  if (detail) {
    guideActions.push(
      {
        id: 'history-export-record',
        label: t('导出本条记录 JSON', 'Export this record JSON'),
        kind: 'commit',
        enabled: !busy,
        run: () => action(exportRecord),
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
          .flatMap((item) => item.run_group?.members ?? [item])
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
      {deletion && (
        <Dialog
          open
          title={
            deletion.wholeCase
              ? t('删除整个算例文件夹', 'Delete entire case folder')
              : deletion.files
                ? t('删除记录及本次运行目录', 'Delete record and this run directory')
                : t('仅删除记录', 'Delete record only')
          }
          showClose={false}
          onOpenChange={(open) => {
            if (!open && !busy) {
              setDeletion(null);
              setError('');
            }
          }}
        >
          <p>
            {historyObject(deletion.item, en)} · {deletion.item.operation_id}
          </p>
          {deletion.files ? (
            <>
              <Notice tone="warning">
                {deletion.wholeCase
                  ? t(
                      '将永久删除整个算例文件夹，包括其中各次运行和全部文件，同时删除本条记录。目录外引用的原始数据不会删除；其他历史记录仍会保留，但其中的目录可能已不存在。',
                      'Permanently delete the entire case folder, including all runs and files inside it, along with this record. Referenced input data outside this folder will be preserved. Other history records remain, but their directories may no longer exist.',
                    )
                  : t(
                      '将永久删除以下本次运行目录及其中全部文件，同时删除本条记录。目录外引用的原始数据不会删除。',
                      'Permanently delete this run directory and all its files, along with this record. Referenced input data outside this directory will be preserved.',
                    )}
              </Notice>
              <p>
                {(deletion.wholeCase ? deletion.plan?.caseHost : deletion.plan?.host) ??
                  t('本机', 'Local')}
              </p>
              <p style={{ overflowWrap: 'anywhere' }}>{deletionPath}</p>
              {deletion.wholeCase && (
                <Field
                  label={t(
                    `输入算例目录名称“${directoryName}”确认`,
                    `Type the case folder name “${directoryName}” to confirm`,
                  )}
                >
                  <Input
                    autoFocus
                    disabled={busy}
                    value={deleteName}
                    aria-label={t('算例目录名称', 'Case folder name')}
                    onChange={(event) => setDeleteName(event.target.value)}
                  />
                </Field>
              )}
            </>
          ) : (
            <p>
              {deletion.item.run_group
                ? t(
                    `移除本次运行的全部 ${deletion.item.run_group.members.length} 个站点记录，不删除任何算例、运行结果或输入数据。`,
                    `Remove all ${deletion.item.run_group.members.length} station records from this run. Case files, outputs and input data will be preserved.`,
                  )
                : t(
                    '只移除本条历史记录，不删除任何算例、运行结果或输入数据。',
                    'Remove only this history record. Case files, outputs and input data will be preserved.',
                  )}
            </p>
          )}
          {error && <Notice tone="error">{error}</Notice>}
          <div className="button-row">
            <Button
              disabled={busy}
              onClick={() => {
                setDeletion(null);
                setError('');
              }}
            >
              {t('取消', 'Cancel')}
            </Button>
            <Button
              variant="danger"
              disabled={busy || !deleteConfirmed || !deleteAllowed(deletion.item)}
              onClick={() => run(deleteRecord)}
            >
              {busy ? t('正在删除…', 'Deleting…') : t('确认删除', 'Confirm deletion')}
            </Button>
          </div>
        </Dialog>
      )}
      {clearPlan && (
        <Dialog
          open
          title={
            clearProject
              ? t(`删除项目“${clearProject.name}”`, `Delete project “${clearProject.name}”`)
              : t('清空历史记录', 'Clear history')
          }
          showClose={false}
          onOpenChange={(open) => {
            if (!open && !busy) setClearPlan(null);
          }}
        >
          <p>
            {t(
              `将移除 ${clearPlan.records} 条已结束的历史记录。`,
              `Remove ${clearPlan.records} finished history records.`,
            )}
            {clearProject &&
              t(
                '只删记录，项目文件夹和其中的算例、结果都不动。',
                'Only records are removed; the project folder, its cases and results are kept.',
              )}
          </p>
          {clearPlan.unconfirmed > 0 && (
            <>
              <label className="history-clear-cases">
                <input
                  type="checkbox"
                  disabled={busy}
                  checked={clearUnconfirmed}
                  onChange={(event) => setClearUnconfirmed(event.target.checked)}
                />
                {t(
                  `同时移除 ${clearPlan.unconfirmed} 条未确认结束的记录`,
                  `Also remove ${clearPlan.unconfirmed} records whose completion was never confirmed`,
                )}
              </label>
              <p className="muted">
                {t(
                  '多是软件关闭或崩溃时中断的任务。移除只是不再显示，不会停止或取消任何任务；',
                  'Mostly work interrupted when the app closed or crashed. Removing only hides them and never stops or cancels anything; ',
                )}
                {clearPlan.unconfirmedRemote > 0 &&
                  t(
                    `其中 ${clearPlan.unconfirmedRemote} 条在服务器上，作业仍可在算例的运行面板里查询和取回。`,
                    `${clearPlan.unconfirmedRemote} ran on servers; their jobs can still be checked and fetched from the case run panel.`,
                  )}
              </p>
            </>
          )}
          {!clearProject && (
            <>
              {clearPlan.cases.length > 0 ? (
                <>
                  <label className="history-clear-cases">
                    <input
                      type="checkbox"
                      disabled={busy}
                      checked={clearCases}
                      onChange={(event) => setClearCases(event.target.checked)}
                    />
                    {t(
                      `同时永久删除这些记录对应的 ${clearPlan.cases.length} 个本机算例文件夹（共 ${sizeText(clearPlan.cases.reduce((sum, item) => sum + item.bytes, 0))}）`,
                      `Also permanently delete the ${clearPlan.cases.length} local case folders of these records (${sizeText(clearPlan.cases.reduce((sum, item) => sum + item.bytes, 0))} in total)`,
                    )}
                  </label>
                  {clearCases && (
                    <Notice tone="warning">
                      {t(
                        '算例文件夹内的设置、运行结果和全部文件都会删除，无法恢复；文件夹外引用的原始数据不受影响。',
                        'Settings, outputs and every file inside these folders are deleted and cannot be recovered; input data referenced outside them is preserved.',
                      )}
                    </Notice>
                  )}
                  <ul className="history-clear-list">
                    {clearPlan.cases.map((item) => (
                      <li key={item.path}>
                        <span>{item.path}</span>
                        <span className="muted">{sizeText(item.bytes)}</span>
                      </li>
                    ))}
                  </ul>
                </>
              ) : (
                <p className="muted">
                  {t(
                    '这些记录对应的本机算例文件夹已不存在，或不是可验证的算例目录；只移除记录。',
                    'No verifiable local case folders remain for these records; only records are removed.',
                  )}
                </p>
              )}
              {clearPlan.remoteCases > 0 && (
                <p className="muted">
                  {t(
                    `${clearPlan.remoteCases} 个服务器算例不会被删除，只移除记录。`,
                    `${clearPlan.remoteCases} server cases are not deleted; only their records are removed.`,
                  )}
                </p>
              )}
              {clearPlan.skipped.length > 0 && (
                <Advanced
                  title={t(
                    `保留的文件夹（${clearPlan.skipped.length}）`,
                    `Kept folders (${clearPlan.skipped.length})`,
                  )}
                >
                  <ul className="history-clear-list">
                    {clearPlan.skipped.map((reason) => (
                      <li key={reason}>{reason}</li>
                    ))}
                  </ul>
                </Advanced>
              )}
            </>
          )}
          {error && <Notice tone="error">{error}</Notice>}
          <div className="button-row">
            <Button disabled={busy} onClick={() => setClearPlan(null)}>
              {t('取消', 'Cancel')}
            </Button>
            <Button
              variant="danger"
              disabled={
                busy ||
                (!clearPlan.records &&
                  !(clearCases && clearPlan.cases.length) &&
                  !(clearUnconfirmed && clearPlan.unconfirmed))
              }
              onClick={() => run(clearHistory)}
            >
              {busy
                ? clearProject
                  ? t('正在删除…', 'Deleting…')
                  : t('正在清空…', 'Clearing…')
                : clearProject
                  ? t('删除项目记录', 'Delete project records')
                  : clearCases
                    ? t('清空记录并删除文件夹', 'Clear records and delete folders')
                    : t('确认清空记录', 'Confirm clearing records')}
            </Button>
          </div>
        </Dialog>
      )}
      {projectDeletion && (
        <Dialog
          open
          title={t(
            `删除项目“${projectDeletion.project.name}”及其文件夹`,
            `Delete project “${projectDeletion.project.name}” and its folder`,
          )}
          showClose={false}
          onOpenChange={(open) => {
            if (!open && !busy) setProjectDeletion(null);
          }}
        >
          <p>
            {t(
              `将删除这个项目的 ${projectDeletion.plan.records} 条记录，并永久删除下面的文件夹及其中全部算例、运行结果和文件，无法恢复。`,
              `Removes the project's ${projectDeletion.plan.records} records and permanently deletes the folders below with every case, result and file in them. This cannot be undone.`,
            )}
          </p>
          <ul className="history-clear-list">
            {projectDeletion.plan.host && projectDeletion.plan.remotePath && (
              <li>
                <span>
                  {t('服务器', 'Server')} {projectDeletion.plan.host}：
                  {projectDeletion.plan.remotePath}
                </span>
              </li>
            )}
            <li>
              <span>
                {projectDeletion.plan.host
                  ? t('本机的配置缓存', 'Local configuration cache')
                  : t('本机', 'This computer')}
                ：{projectDeletion.plan.path}
              </span>
              <span className="muted">{sizeText(projectDeletion.plan.bytes)}</span>
            </li>
          </ul>
          <Notice tone="warning">
            {t(
              '文件夹外引用的原始数据（站点、强迫场、地表数据）不会删除。',
              'Input data referenced outside these folders (sites, forcing, surface data) is preserved.',
            )}
          </Notice>
          <label>
            {t(
              `输入项目文件夹名“${projectDeletion.project.name}”确认`,
              `Type the project folder name “${projectDeletion.project.name}” to confirm`,
            )}
            <input
              value={projectConfirm}
              disabled={busy}
              onChange={(event) => setProjectConfirm(event.target.value)}
            />
          </label>
          {error && <Notice tone="error">{error}</Notice>}
          <div className="button-row">
            <Button disabled={busy} onClick={() => setProjectDeletion(null)}>
              {t('取消', 'Cancel')}
            </Button>
            <Button
              variant="danger"
              disabled={busy || projectConfirm !== projectDeletion.project.name}
              onClick={() => run(deleteProject)}
            >
              {busy ? t('正在删除…', 'Deleting…') : t('确认删除', 'Confirm deletion')}
            </Button>
          </div>
        </Dialog>
      )}
      {projectOutcome && (
        <Notice tone="success">
          {t(
            `已删除 ${projectOutcome.removed} 条记录和项目文件夹${projectOutcome.host ? `（${projectOutcome.host}：${projectOutcome.remotePath}）` : ''}。`,
            `Removed ${projectOutcome.removed} records and the project folder${projectOutcome.host ? ` (${projectOutcome.host}: ${projectOutcome.remotePath})` : ''}.`,
          )}
        </Notice>
      )}
      {clearOutcome && (
        <Notice tone={clearOutcome.failed.length ? 'warning' : 'success'}>
          {t(
            `已移除 ${clearOutcome.removed} 条记录，删除 ${clearOutcome.deleted.length} 个算例文件夹；保留 ${clearOutcome.kept} 条记录。`,
            `Removed ${clearOutcome.removed} records and deleted ${clearOutcome.deleted.length} case folders; kept ${clearOutcome.kept} records.`,
          )}
          {clearOutcome.failed.map((failure) => (
            <span key={failure} className="history-clear-failure">
              {failure}
            </span>
          ))}
        </Notice>
      )}
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
            <label className="history-routine">
              <input
                type="checkbox"
                disabled={busy}
                checked={Boolean(effectiveFilter.includeRoutine)}
                onChange={(e) =>
                  setFilter((f) =>
                    scopedHistoryFilter({ ...f, includeRoutine: e.target.checked }, casePath),
                  )
                }
              />
              {t(
                '也显示例行记录（自动保存的设置、版本草稿、打开算例时的版本绑定）',
                'Also show routine records (autosaved settings, version drafts, case binding on open)',
              )}
            </label>
          </Advanced>
        </form>
        {casePath && (
          <p className="muted history-scoped">
            {t('当前算例', 'Current case')}: {casePath}
          </p>
        )}
      </Card>
      {error && <Notice tone="error">{error}</Notice>}
      <section className="history-list" aria-label={t('历史记录列表', 'History records')}>
        <div className="history-list-toolbar">
          <span className="muted">
            {busy ? t('正在读取…', 'Loading…') : `${page.total} ${t('条记录', 'records')}`}
          </span>
          <div className="button-row">
            <Button variant="danger" disabled={busy} onClick={() => run(() => openClear())}>
              {t('清空历史记录', 'Clear history')}
            </Button>
            {(['csv', 'json'] as const).map((format) => (
              <Button key={format} disabled={busy} onClick={() => run(() => exportAll(format))}>
                {t('导出', 'Export')} {format.toUpperCase()}
              </Button>
            ))}
          </div>
        </div>
        <div
          className="history-cards"
          role="list"
          aria-label={t('历史记录卡片', 'History record cards')}
        >
          {page.items.map((item) => {
            const selected = (group ?? detail?.operation)?.operation_id === item.operation_id;
            const status = item.needs_reconciliation ? 'pending' : item.status;
            const subject = historySubject(item);
            const version = historyVersionLabel(
              item.run_group?.members[0] ?? item,
              versionNames,
              en,
            );
            const duration = historyDurationLabel(item.duration_ms, en);
            return (
              <ContextMenu
                key={item.operation_id}
                items={deleteItems(item)}
                onOpenChange={(open) => menuChanged(item, open)}
              >
                <Button
                  variant="ghost"
                  role="listitem"
                  className={`history-card history-card-${status} ${selected ? 'is-selected' : ''} ${historyInternal(item) ? 'is-routine' : ''}`}
                  aria-pressed={selected}
                  aria-label={`${t('查看记录', 'View record')} ${historyObject(item, en)}`}
                  title={item.case_path ?? item.operation_id}
                  disabled={busy}
                  onClick={() => run(() => showItem(item))}
                >
                  <span className="history-card-head">
                    <span className="history-card-kind">{historyActionLabel(item, en)}</span>
                    <span className={`history-status history-status-${status}`}>
                      {item.needs_reconciliation
                        ? t('待核对', 'Needs reconciliation')
                        : historyStatusLabel(item.status, en)}
                    </span>
                  </span>
                  <strong className="history-card-title">{historyObject(item, en)}</strong>
                  {(item.run_group || item.project_group || subject.project) && (
                    <span className="history-card-sub">
                      {item.project_group
                        ? `${projectSummary(item.project_group).cases} ${t('个算例', 'cases')} · ${item.project_group.members.length} ${t('条操作', 'operations')}`
                        : item.run_group
                          ? `${item.run_group.members.length} ${t('个站点', 'stations')}`
                          : `${t('项目', 'Project')} ${subject.project}`}
                    </span>
                  )}
                  <span className="history-card-chips">
                    {version && <span className="history-chip">{version}</span>}
                    <span className="history-chip">
                      {item.host ??
                        (item.location === 'remote'
                          ? t('服务器', 'Server')
                          : item.location === 'local'
                            ? t('本机', 'Local')
                            : t('位置未记录', 'Location not recorded'))}
                    </span>
                    {duration && <span className="history-chip">{duration}</span>}
                    {item.project_group &&
                      statusCountsLabel(projectSummary(item.project_group).counts, en) && (
                        <span className="history-chip">
                          {statusCountsLabel(projectSummary(item.project_group).counts, en)}
                        </span>
                      )}
                  </span>
                  <span className="history-card-meta">
                    {item.time_ms == null ? (
                      t('时间未记录', 'Time not recorded')
                    ) : (
                      <time dateTime={new Date(item.time_ms).toISOString()}>
                        {new Date(item.time_ms).toLocaleString(appLocale(en), {
                          year: 'numeric',
                          month: '2-digit',
                          day: '2-digit',
                          hour: '2-digit',
                          minute: '2-digit',
                        })}
                      </time>
                    )}
                  </span>
                </Button>
              </ContextMenu>
            );
          })}
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
      {(group || detail) && (
        <Dialog
          open
          className="history-detail-dialog"
          title={
            detail
              ? `${historyKindLabel(detail.operation.kind, en)} · ${historyObject(detail.operation, en)}`
              : group
                ? historyObject(group, en)
                : ''
          }
          onOpenChange={(open) => {
            if (!open && !busy && !deletion) closeDetails();
          }}
        >
          <div className="history-inspector" aria-label={t('记录详情', 'Record details')}>
            {group?.project_group && detail && (
              <div className="history-timeline-back">
                <Button variant="ghost" disabled={busy} onClick={() => setDetail(null)}>
                  {t('← 返回项目时间线', '← Back to project timeline')}
                </Button>
                <span className="muted">{group.project_group.name}</span>
                <Button
                  disabled={busy}
                  onClick={() => run(() => openProject(group.project_group!.path))}
                >
                  {t('打开这个项目', 'Open this project')}
                </Button>
              </div>
            )}
            {group?.project_group && !detail && (
              <Card title={t('项目时间线', 'Project timeline')}>
                <div className="button-row">
                  <Button
                    variant="primary"
                    disabled={busy}
                    onClick={() => run(() => openProject(group.project_group!.path))}
                  >
                    {t('打开这个项目', 'Open this project')}
                  </Button>
                </div>
                <p className="muted">
                  {group.project_group.path} ·{' '}
                  {t(
                    '同一批的操作合成一行；展开后点算例查看详情，详情里可以删除或对比。',
                    'A batch shares one row; expand it and choose a case for details, where it can be deleted or compared.',
                  )}
                </p>
                <ol className="history-timeline">
                  {projectTimeline(group.project_group.members).map((row) => (
                    <li key={row.key}>
                      <details>
                        <summary>
                          <span className="history-timeline-time">
                            {row.time_ms == null
                              ? t('时间未记录', 'Time not recorded')
                              : new Date(row.time_ms).toLocaleString(appLocale(en), {
                                  month: '2-digit',
                                  day: '2-digit',
                                  hour: '2-digit',
                                  minute: '2-digit',
                                })}
                          </span>
                          <strong>{historyActionLabel(row.members[0], en)}</strong>
                          <span>
                            {row.members.length > 1
                              ? `${row.members.length} ${t('个算例', 'cases')}`
                              : historyObject(row.members[0], en)}
                          </span>
                          <span className="muted">{statusCountsLabel(row.counts, en)}</span>
                        </summary>
                        <div className="button-row">
                          {row.members.map((member) => (
                            <ContextMenu
                              key={member.operation_id}
                              items={deleteItems(member)}
                              onOpenChange={(open) => menuChanged(member, open)}
                            >
                              <Button
                                disabled={busy}
                                onClick={() => run(() => show(member.operation_id))}
                              >
                                {historyObject(member, en)} ·{' '}
                                {member.needs_reconciliation
                                  ? t('待核对', 'Needs reconciliation')
                                  : historyStatusLabel(member.status, en)}
                              </Button>
                            </ContextMenu>
                          ))}
                        </div>
                      </details>
                    </li>
                  ))}
                </ol>
              </Card>
            )}
            {group?.run_group && (
              <Card title={historyObject(group, en)}>
                <p>
                  {group.run_group.members.length} {t('个站点', 'stations')} ·{' '}
                  {group.run_group.run_id}
                </p>
                <p className="muted">
                  {t(
                    '选择站点查看详情；右键站点可删除该站点的记录或文件夹。',
                    'Select a station for details; right-click a station to delete its record or folder.',
                  )}
                </p>
                <div className="button-row">
                  {group.run_group.members.map((member) => (
                    <ContextMenu
                      key={member.operation_id}
                      items={deleteItems(member)}
                      onOpenChange={(open) => menuChanged(member, open)}
                    >
                      <Button
                        disabled={busy}
                        aria-pressed={detail?.operation.operation_id === member.operation_id}
                        onClick={() => run(() => show(member.operation_id))}
                      >
                        {historyObject(member, en)} ·{' '}
                        {member.needs_reconciliation
                          ? t('待核对', 'Needs reconciliation')
                          : historyStatusLabel(member.status, en)}
                      </Button>
                    </ContextMenu>
                  ))}
                </div>
                <Button disabled={busy} onClick={() => run(exportGroup)}>
                  {t('导出本次运行 JSON', 'Export this run JSON')}
                </Button>
              </Card>
            )}
            {detail ? (
              <Card title={historyKindLabel(detail.operation.kind, en)}>
                <h3>{historyObject(detail.operation, en)}</h3>
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
                          : new Date(detail.operation.started_ms).toLocaleString(appLocale(en)),
                    },
                    {
                      zh: '结束',
                      en: 'Finished',
                      value:
                        detail.operation.finished_ms == null
                          ? null
                          : new Date(detail.operation.finished_ms).toLocaleString(appLocale(en)),
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
                  {simulationDraft(detail.operation) && (
                    <Button
                      variant="primary"
                      disabled={busy}
                      onClick={() => resumeSimulation(detail.operation)}
                    >
                      {t(
                        `继续新建（第 ${simulationDraft(detail.operation)!.step} 步）`,
                        `Continue (step ${simulationDraft(detail.operation)!.step})`,
                      )}
                    </Button>
                  )}
                  {detail.operation.case_path && (
                    <Button
                      variant="primary"
                      disabled={busy}
                      onClick={() =>
                        run(async () => {
                          await openCase(detail.operation.case_path!);
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
                            await invokeJson('history_replay', {
                              id: detail.operation.operation_id,
                            }),
                          ),
                        )
                      }
                    >
                      {t('按记录设置重跑', 'Replay recorded settings')}
                    </Button>
                  )}
                  <Button disabled={busy} onClick={() => run(exportRecord)}>
                    {t('导出本条记录 JSON', 'Export this record JSON')}
                  </Button>
                </div>
                {result != null && (
                  <Notice tone="warning">
                    {replayVerificationText(result, en)}{' '}
                    {t('完整结果见下方记录。', 'See the full record below.')}
                  </Notice>
                )}
                <Advanced
                  title={t('依据、比较与完整记录', 'Evidence, comparison and full record')}
                  open={detailsOpen}
                  onOpenChange={setDetailsOpen}
                >
                  <div className="card-grid">
                    {historySections(detail, en).map((section) => (
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
                        .flatMap((item) => item.run_group?.members ?? [item])
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
                    {historyEvents(detail, en).map((rows, index) => (
                      <Card key={index} title={`${t('事件', 'Event')} ${index + 1}`}>
                        <DetailRows rows={rows} en={en} />
                      </Card>
                    ))}
                  </div>
                </Advanced>
              </Card>
            ) : null}
          </div>
        </Dialog>
      )}
    </div>
  );
}

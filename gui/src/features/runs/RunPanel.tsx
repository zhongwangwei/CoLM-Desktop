import { useEffect, useRef, useState } from 'react';
import { useApp } from '../../lib/app-state';
import { invoke, invokeJson } from '../../lib/ipc';
import { effectiveModelFields } from '../cases/case-model';
import type { RunState } from '../../lib/types';
import { usePageContract, type UiField } from '../../lib/guide';
import { Button, Card, Field, Input, Select, Advanced, Notice } from '../../components/ui';
import {
  compatibleKernel,
  remoteSubmissionBlocked,
  remoteRunPermissions,
  reconcileRemoteRun,
  restoreRunView,
  settleRuns,
} from './run-model';
export default function RunPanel({
  casePath,
  effectiveProfile,
  disabled = false,
  active = true,
  summary,
  onBack,
}: {
  casePath: string;
  effectiveProfile: {
    grid?: string;
    classification?: 'IGBP' | 'USGS';
    subgrid?: string;
    crop?: boolean;
  } | null;
  disabled?: boolean;
  active?: boolean;
  summary?: { name: string; period: string };
  onBack?: () => void;
}) {
  const { state, notify, update } = useApp();
  const en = state.language === 'en',
    t = (zh: string, english: string) => (en ? english : zh);
  const [host, setHost] = useState(''),
    [servers, setServers] = useState<{ host: string }[]>([]),
    [kernel, setKernel] = useState(''),
    [preprocessors, setPreprocessors] = useState('rust'),
    [engine, setEngine] = useState('rust'),
    [stage, setStage] = useState(''),
    [ranks, setRanks] = useState('1'),
    [threads, setThreads] = useState('1'),
    [workers, setWorkers] = useState('1'),
    [force, setForce] = useState(false),
    [advanced, setAdvanced] = useState(state.expert),
    [batch, setBatch] = useState<string[]>([]),
    [remote, setRemote] = useState<unknown>(null),
    [recovering, setRecovering] = useState(true),
    [busy, setBusy] = useState(false),
    [monitor, setMonitor] = useState(false);
  const lock = useRef(false),
    alive = useRef(true),
    latest = useRef(state.running);
  latest.current = state.running;
  const persisted = state.running[casePath];
  const kernelMatches =
    !!effectiveProfile &&
    !!compatibleKernel(
      state.kernels.filter((k) => k.dir === kernel),
      effectiveProfile,
      effectiveModelFields(state.version?.overlay),
    );
  const pending = busy || recovering || !!persisted?.running;
  const view = restoreRunView(persisted, casePath),
    remotePermissions = remoteRunPermissions(remote);
  useEffect(() => {
    let current = true;
    alive.current = true;
    invoke<{ servers: { host: string }[] }>('remote_config', {})
      .then((c) => {
        if (current) setServers(c.servers);
      })
      .catch((e) => {
        if (current) notify(String(e), true);
      });
    return () => {
      current = false;
      alive.current = false;
    };
  }, [notify]);
  useEffect(() => {
    let current = true;
    setRecovering(true);
    setRemote(null);
    setBatch([]);
    const known = latest.current[casePath]?.progress?.remote;
    if (known) setRemote(known);
    invokeJson<Record<string, unknown> | null>('remote_job_record', { case: casePath })
      .then((record) => {
        if (!current || !alive.current) return;
        setRemote(record);
        if (record)
          update((old) =>
            old.running[casePath]
              ? {}
              : {
                  running: {
                    ...old.running,
                    [casePath]: {
                      case: casePath,
                      location: 'remote',
                      status: 'unknown',
                      running: true,
                      logs: [],
                      stages: {},
                      progress: {
                        remote: record,
                        ...(typeof record.host === 'string' ? { host: record.host } : {}),
                      },
                    },
                  },
                },
          );
      })
      .catch((e) => {
        if (current) notify(String(e), true);
      })
      .finally(() => {
        if (current && alive.current) setRecovering(false);
      });
    return () => {
      current = false;
    };
  }, [casePath, notify, update]);
  useEffect(() => {
    setKernel(
      effectiveProfile
        ? (compatibleKernel(
            state.kernels,
            effectiveProfile,
            effectiveModelFields(state.version?.overlay),
          )?.dir ?? '')
        : '',
    );
  }, [state.kernels, effectiveProfile, state.version]);
  useEffect(() => {
    const progress = persisted?.progress;
    if (typeof progress?.host === 'string') setHost(progress.host);
    if (progress?.remote) setRemote(progress.remote);
  }, [persisted?.progress]);
  async function action(fn: () => Promise<unknown>) {
    if (lock.current) return;
    lock.current = true;
    setBusy(true);
    try {
      return await fn();
    } catch (e) {
      notify(String(e), true);
      throw e;
    } finally {
      lock.current = false;
      if (alive.current) setBusy(false);
    }
  }
  function patchRun(targets: string[], runId: string, patch: Partial<RunState>) {
    update((old) => ({
      running: {
        ...old.running,
        ...Object.fromEntries(
          targets
            .filter((dir) => old.running[dir]?.run_id === runId)
            .map((dir) => [dir, { ...old.running[dir], ...patch }]),
        ),
      },
    }));
  }
  async function run() {
    await action(async () => {
      if (recovering) throw new Error('正在恢复运行记录 / Restoring run records');
      if (disabled) throw new Error('请先保存算例设置 / Save case settings first');
      const targets = host ? [casePath] : [...new Set([casePath, ...batch])];
      if (targets.some((dir) => latest.current[dir]?.running))
        throw new Error('算例已经在运行或等待核对 / A selected case is already running');
      if (host && remoteSubmissionBlocked(remote))
        throw new Error('已有服务器作业：先核对终态，不重复提交 / Check existing job first');
      if (!kernelMatches)
        throw new Error('请选择与当前模型匹配的内核 / Select a compatible kernel');
      const nums = [ranks, threads, workers].map(Number);
      if (nums.some((n) => !Number.isInteger(n) || n < 1))
        throw new Error('进程、线程与并发数须为正整数');
      const runId = crypto.randomUUID();
      const initial = Object.fromEntries(
        targets.map((dir) => [
          dir,
          {
            case: dir,
            location: host ? 'remote' : 'local',
            run_id: runId,
            status: 'preparing',
            running: true,
            logs: [],
            stages: {},
            ...(host ? { progress: { host } } : {}),
          } satisfies RunState,
        ]),
      );
      latest.current = { ...latest.current, ...initial };
      update((old) => ({ running: { ...old.running, ...initial } }));
      let submitted = false;
      try {
        const revision = state.versionRef;
        if (host) {
          submitted = true;
          patchRun(targets, runId, { status: 'submitting' });
          const result = await invokeJson<Record<string, unknown>>('remote_run', {
            case: casePath,
            host,
            kernel,
            stage: stage || null,
            force,
            engine,
            preprocessors,
            ranks: nums[0],
            modelVersion: null,
            modelRevision: revision,
          });
          patchRun(targets, runId, { status: 'submitted', progress: { host, remote: result } });
          if (alive.current) setRemote(result);
          return result;
        }
        patchRun(targets, runId, { status: 'starting' });
        submitted = true;
        const args = {
          runId,
          kernel,
          force,
          stage: stage || null,
          mpiRanks: nums[0],
          engine,
          preprocessors,
          threads: nums[1],
          modelVersion: null,
          modelRevision: revision,
        };
        const result =
          targets.length > 1
            ? await invoke<{ failed: number; succeeded: number }>('run_batch', {
                ...args,
                cases: targets,
                maxConcurrent: nums[2],
              })
            : await invoke<number>('run_case', { ...args, case: casePath });
        const ok =
          typeof result === 'number'
            ? result === 0
            : result.failed === 0 && result.succeeded === targets.length;
        update((old) => ({ running: settleRuns(old.running, targets, runId, ok) }));
        return result;
      } catch (error) {
        patchRun(targets, runId, {
          status: host && submitted ? 'unknown' : 'failed',
          running: !!host && submitted,
          error: String(error),
        });
        if (host && submitted && alive.current)
          setRemote({ status: { state: 'unknown' }, error: String(error) });
        throw error;
      }
    });
  }
  async function remoteAction(command: 'remote_status' | 'remote_cancel' | 'remote_fetch') {
    if (lock.current || monitor) return;
    if (command === 'remote_cancel' && !remotePermissions.cancel)
      throw new Error('先核对服务器任务身份与运行状态 / Reconcile job identity and state first');
    if (command === 'remote_fetch' && !remotePermissions.fetch)
      throw new Error('先核对服务器任务的真实终态 / Confirm terminal state first');
    lock.current = true;
    setMonitor(true);
    const expectedRunId = latest.current[casePath]?.run_id;
    try {
      let result = await invokeJson<Record<string, unknown>>(
        command,
        command === 'remote_fetch'
          ? { case: casePath, host: null, all: false, overwriteNewer: false }
          : { case: casePath },
      );
      if (command === 'remote_cancel' || command === 'remote_fetch')
        result = await invokeJson<Record<string, unknown>>('remote_status', { case: casePath });
      const merged = reconcileRemoteRun(remote, result);
      if (alive.current) setRemote(merged);
      update((old) => {
        const previous = old.running[casePath];
        if (previous?.run_id !== expectedRunId) return {};
        const status = result.status as
          { state?: string; exit_code?: number; log_tail?: string } | undefined;
        const entry: RunState = {
          ...(previous ?? {
            case: casePath,
            status: 'unknown',
            running: true,
            logs: [],
            stages: {},
          }),
          location: 'remote',
          progress: { ...previous?.progress, remote: merged },
        };
        if (status) {
          entry.running = !remoteRunPermissions(merged).terminal;
          entry.status = remoteRunPermissions(merged).terminal
            ? status.exit_code === 0
              ? 'succeeded'
              : 'failed'
            : (status.state ?? 'unknown');
          if (status.log_tail) entry.logs = status.log_tail.split('\n');
        }
        return { running: { ...old.running, [casePath]: entry } };
      });
      return merged;
    } catch (e) {
      notify(String(e), true);
      throw e;
    } finally {
      lock.current = false;
      if (alive.current) setMonitor(false);
    }
  }
  async function cancel() {
    try {
      const id = latest.current[casePath]?.run_id;
      const targets = id
        ? Object.entries(latest.current)
            .filter(([, run]) => run.run_id === id && run.running)
            .map(([dir]) => dir)
        : [casePath];
      const count = await invoke<number>('cancel_runs', { cases: targets });
      notify(`已向 ${count} 个进程发送停止请求；等待真实退出 / Stop requested`);
      return { stop_requested: count, terminal: false };
    } catch (e) {
      notify(String(e), true);
      throw e;
    }
  }

  const fields: UiField[] = [
    {
      id: 'run-host',
      label: '运行位置 / Machine',
      type: 'choice' as const,
      value: host,
      disabled: pending,
      options: [
        { value: '', label: '本机 / Local' },
        ...servers.map((s) => ({ value: s.host, label: s.host })),
      ],
      set: setHost,
    },
    ...(advanced
      ? [
          {
            id: 'run-kernel',
            label: '内核 / Kernel',
            type: 'choice' as const,
            value: kernel,
            options: state.kernels.map((k) => ({ value: k.dir, label: k.preset })),
            set: setKernel,
          },
          {
            id: 'run-engine',
            label: '引擎 / Engine',
            type: 'choice' as const,
            value: engine,
            options: ['rust', 'fortran'].map((value) => ({ value, label: value })),
            set: setEngine,
          },
          {
            id: 'run-preprocessors',
            label: '前处理引擎 / Preprocessors',
            type: 'choice' as const,
            value: preprocessors,
            options: ['rust', 'fortran'].map((value) => ({ value, label: value })),
            set: setPreprocessors,
          },
          {
            id: 'run-stage',
            label: '阶段 / Stage',
            type: 'choice' as const,
            value: stage,
            options: [
              { value: '', label: '全部 / All' },
              ...['mksrfdata', 'mkinidata', 'colm'].map((value) => ({ value, label: value })),
            ],
            set: setStage,
          },
          ...(
            [
              ['ranks', 'MPI 进程', ranks, setRanks],
              ['threads', '线程 / Threads', threads, setThreads],
              ['workers', '并发算例 / Workers', workers, setWorkers],
            ] as const
          ).map(([id, label, value, set]) => ({
            id: `run-${id}`,
            label,
            value,
            type: 'number' as const,
            set,
          })),
          {
            id: 'run-force',
            label: '重新运行全部阶段 / Force',
            value: String(force),
            type: 'switch' as const,
            set: (v: string) => setForce(v === 'true'),
          },
        ]
      : []),
  ].map((f) => ({ ...f, disabled: pending }));
  fields.push({
    id: 'run-advanced',
    label: t('高级配置', 'Advanced settings'),
    value: String(advanced),
    type: 'switch',
    disabled: pending,
    set: (v) => setAdvanced(v === 'true'),
  });
  if (advanced && !host)
    fields.push(
      ...state.cases
        .filter((c) => c.dir !== casePath)
        .map((c) => ({
          id: `run-batch-${c.name}`,
          label: c.name,
          value: String(batch.includes(c.dir)),
          type: 'switch' as const,
          disabled: pending,
          set: (v: string) =>
            setBatch((b) =>
              v === 'true' ? [...new Set([...b, c.dir])] : b.filter((p) => p !== c.dir),
            ),
        })),
    );
  usePageContract(
    {
      title: '运行 / Run',
      step: 'run',
      kernelDir: effectiveProfile ? kernel || null : null,
      preprocessors,
      status: pending ? 'busy' : (persisted?.status ?? 'ready'),
      details: { run: view, remote, error: persisted?.error },
      fields,
      actions: [
        {
          id: 'run-submit',
          label: t('开始运行', 'Start run'),
          kind: 'commit',
          enabled:
            !disabled && !pending && kernelMatches && (!host || !remoteSubmissionBlocked(remote)),
          run,
        },
        {
          id: 'run-cancel',
          label: '停止本机运行 / Stop',
          kind: 'commit',
          enabled: !!persisted?.running && persisted.location === 'local',
          run: cancel,
        },
        ...(remote
          ? [
              {
                id: 'remote-status',
                label: '核对服务器状态 / Check',
                kind: 'click' as const,
                enabled: !monitor,
                run: () => remoteAction('remote_status'),
              },
              {
                id: 'remote-cancel',
                label: '取消服务器任务 / Cancel',
                kind: 'commit' as const,
                enabled: !monitor && remotePermissions.cancel,
                run: () => remoteAction('remote_cancel'),
              },
              {
                id: 'remote-fetch',
                label: '取回结果 / Fetch',
                kind: 'commit' as const,
                enabled: !monitor && remotePermissions.fetch,
                run: () => remoteAction('remote_fetch'),
              },
            ]
          : []),
      ],
    },
    active,
  );
  const render = (f: UiField) => (
    <Field key={f.id} label={f.label}>
      {f.type === 'choice' ? (
        <Select value={String(f.value)} disabled={pending} onChange={(e) => f.set(e.target.value)}>
          {f.options?.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </Select>
      ) : f.type === 'switch' ? (
        <Input
          type="checkbox"
          checked={force}
          disabled={pending}
          onChange={(e) => setForce(e.target.checked)}
        />
      ) : (
        <Input
          type="number"
          min="1"
          value={String(f.value)}
          disabled={pending}
          onChange={(e) => f.set(e.target.value)}
        />
      )}
    </Field>
  );
  const statusLabels: Record<string, string> = {
    preparing: t('准备中', 'Preparing'),
    starting: t('正在启动', 'Starting'),
    running: t('运行中', 'Running'),
    succeeded: t('运行完成', 'Completed'),
    failed: t('运行失败', 'Failed'),
    cancelled: t('已停止', 'Stopped'),
    unknown: t('等待核对', 'Needs verification'),
    submitted: t('已提交', 'Submitted'),
    submitting: t('正在提交', 'Submitting'),
  };
  const stageLabels: Record<string, string> = {
    mksrfdata: t('准备地表数据', 'Surface data'),
    mkinidata: t('准备初始场', 'Initial conditions'),
    colm: t('模型计算', 'Simulation'),
  };
  return (
    <Card title={t('检查并运行', 'Review and run')} className="simulation-run-card">
      <p className="muted">
        {t('确认以下设置，然后开始计算。', 'Review these settings, then start the calculation.')}
      </p>
      <div className="simulation-review">
        <dl>
          <dt>{t('算例', 'Case')}</dt>
          <dd>{summary?.name ?? state.selectedCase?.entry.name}</dd>
          <dt>{t('时段', 'Period')}</dt>
          <dd>{summary?.period ?? '—'}</dd>
          <dt>{t('模型', 'Model')}</dt>
          <dd>
            {state.version?.name ?? '—'} · r{state.versionRef.revision}
          </dd>
          <dt>{t('运行位置', 'Run location')}</dt>
          <dd>
            {host || t('本机', 'Local')} · {engine === 'rust' ? 'Rust' : 'Fortran'}
          </dd>
        </dl>
        <div className="simulation-checks">
          <p className={disabled ? 'check-pending' : 'check-fact'}>
            <span>{disabled ? '○' : '✓'}</span>
            {disabled
              ? t('设置待保存或读取', 'Settings pending')
              : t('算例设置已保存', 'Case settings saved')}
          </p>
          <p className={kernelMatches ? 'check-fact' : 'check-pending'}>
            <span>{kernelMatches ? '✓' : '○'}</span>
            {kernelMatches
              ? t('内核已匹配', 'Kernel matched')
              : t('尚无匹配内核', 'No matching kernel')}
          </p>
          <p className="check-pending">
            <span>○</span>
            {t('启动时检查实际输入', 'Inputs checked at startup')}
          </p>
        </div>
      </div>
      <div className="run-location">
        {render({
          ...fields[0],
          label: t('运行位置', 'Run location'),
          options: [
            { value: '', label: t('本机', 'Local') },
            ...servers.map((s) => ({ value: s.host, label: s.host })),
          ],
        })}
      </div>
      {disabled && (
        <Notice tone="warning">
          {t(
            '算例设置尚未保存、读取失败或正在加载。请先处理后再运行。',
            'Save or load the case settings successfully before running.',
          )}
        </Notice>
      )}
      {host && remoteSubmissionBlocked(remote) && (
        <Notice tone="warning">
          {t(
            '存在服务器作业。先核对真实终态，避免重复提交。',
            'Reconcile the existing server job before submitting again.',
          )}
        </Notice>
      )}
      {!kernelMatches && (
        <Notice tone="warning">
          {t(
            '没有与当前模型匹配的内核。可在高级配置中选择，或在模型版本页检查。',
            'Select a compatible kernel in Advanced settings, or check Model versions.',
          )}
        </Notice>
      )}
      <Advanced
        open={advanced}
        onOpenChange={setAdvanced}
        title={t(
          '高级配置（引擎、线程与计算阶段）',
          'Advanced settings (engine, threads and stages)',
        )}
      >
        <div className="form-grid">
          {fields
            .slice(1)
            .filter((f) => f.id !== 'run-advanced' && !f.id.startsWith('run-batch-'))
            .map(render)}
        </div>
        {!host && (
          <fieldset>
            <legend>{t('同时运行的其他算例', 'Other cases to run together')}</legend>
            {state.cases
              .filter((c) => c.dir !== casePath)
              .map((c) => (
                <label key={c.dir}>
                  <input
                    type="checkbox"
                    disabled={pending}
                    checked={batch.includes(c.dir)}
                    onChange={(e) =>
                      setBatch((b) =>
                        e.target.checked ? [...b, c.dir] : b.filter((p) => p !== c.dir),
                      )
                    }
                  />
                  {c.name}
                </label>
              ))}
          </fieldset>
        )}
      </Advanced>
      <footer className="simulation-footer">
        {onBack ? (
          <Button disabled={pending} onClick={onBack}>
            {t('上一步', 'Back')}
          </Button>
        ) : (
          <span />
        )}
        <div className="actions">
          {!!persisted?.running && persisted.location === 'local' && (
            <Button variant="danger" onClick={() => void cancel().catch(() => {})}>
              {t('停止运行', 'Stop run')}
            </Button>
          )}
          <Button
            variant="primary"
            disabled={
              disabled || pending || !kernelMatches || (!!host && remoteSubmissionBlocked(remote))
            }
            onClick={() => void run().catch(() => {})}
          >
            {recovering
              ? t('核对运行记录…', 'Checking run records…')
              : pending
                ? t('运行中…', 'Running…')
                : t('开始运行', 'Start run')}
          </Button>
        </div>
      </footer>
      {view && (
        <section className="simulation-monitor" aria-live="polite">
          <h3>{t('运行进度', 'Run progress')}</h3>
          <p role="status">
            {statusLabels[view.status] ?? view.status}
            {view.stage ? ` · ${stageLabels[view.stage] ?? view.stage}` : ''}
          </p>
          {!!view.total && <progress value={view.step ?? 0} max={view.total} />}
          <p className="path">
            {t('运行编号', 'Run ID')}：{view.runId || t('等待核对', 'Needs verification')}
          </p>
          {persisted?.error && <Notice tone="error">{persisted.error}</Notice>}
          <Advanced title={t('查看运行日志', 'View run log')} open={!!persisted?.running}>
            <pre className="run-log">{view.lines.join('\n') || t('暂无日志', 'No log yet')}</pre>
          </Advanced>
        </section>
      )}
      {remote != null && (
        <>
          {remotePermissions.pending && (
            <Notice tone="warning">
              {t(
                '提交身份尚未确认，先核对原作业。',
                'Reconcile the original job; submission identity is unconfirmed.',
              )}
            </Notice>
          )}
          <div className="actions">
            <Button
              disabled={monitor}
              onClick={() => void remoteAction('remote_status').catch(() => {})}
            >
              {t('核对服务器状态', 'Check server status')}
            </Button>
            <Button
              disabled={monitor || !remotePermissions.cancel}
              onClick={() => void remoteAction('remote_cancel').catch(() => {})}
            >
              {t('取消任务', 'Cancel job')}
            </Button>
            <Button
              disabled={monitor || !remotePermissions.fetch}
              onClick={() => void remoteAction('remote_fetch').catch(() => {})}
            >
              {t('取回结果', 'Fetch results')}
            </Button>
          </div>
          <Advanced title={t('服务器作业详情', 'Server job details')}>
            <pre>{JSON.stringify(remote, null, 2)}</pre>
          </Advanced>
        </>
      )}
    </Card>
  );
}

import { bilingual } from '../../lib/bilingual';
import { Fragment, useEffect, useRef, useState } from 'react';
import { useApp } from '../../lib/app-state';
import { invoke, invokeJson } from '../../lib/ipc';
import { effectiveModelFields } from '../cases/case-model';
import { eachLimited, runRemoteBatch } from './remote-batch';
import NamelistPreview from './NamelistPreview';
import EngineProgress from '../cases/EngineProgress';
import ServerLoadPanel from './ServerLoadPanel';
import { serverLoadSummary, type ServerLoad } from './server-load';
import type { BuildProgress } from '../cases/engine-progress';
import { elapsedLabel } from '../cases/simulation-model';
import type { CaseProfile, LastRun, RunState, Server, VersionRef } from '../../lib/types';
import { usePageContract, type UiField } from '../../lib/guide';
import { Button, Card, Field, Input, Select, Advanced, Notice } from '../../components/ui';
import {
  compatibleKernel,
  endedRun,
  openRuns,
  remoteJobFacts,
  runCountsText,
  remoteStatusSummary,
  restoredRuns,
  remoteSubmissionBlocked,
  REMOTE_CHECK_MS,
  remoteRunPermissions,
  reconcileRemoteRun,
  matchesRemoteRun,
  restoreRunView,
  settleRuns,
  submittingHost,
  remoteEndStatus,
  remoteActivity,
} from './run-model';
// A batch is submitted by a loop that outlives the page showing it: leaving the page (or coming
// back to it) must not stop the stations still waiting to be submitted. Only the Stop button does,
// from whichever panel is showing, so the stop marks live here rather than in a component ref.
const stoppedBatches = new Set<string>();
// Runs whose submission loop is alive in this window; the others are only watched.
const drivenBatches = new Set<string>();

export default function RunPanel({
  casePath,
  effectiveProfile,
  disabled = false,
  active = true,
  summary,
  onBack,
  batchCases,
  onBusyChange,
  initialSettings,
  initialHost,
  batchProfiles,
}: {
  casePath: string;
  effectiveProfile: {
    grid?: string;
    classification?: 'IGBP' | 'USGS';
    subgrid?: string;
    crop?: boolean;
    engine?: string;
  } | null;
  disabled?: boolean;
  active?: boolean;
  summary?: { name: string; period: string };
  onBack?: () => void;
  batchCases?: string[];
  onBusyChange?: (busy: boolean) => void;
  initialSettings?: {
    threads: string;
    ranks: string;
    workers: string;
    scheduler: string;
    cpus: string;
  };
  initialHost?: string;
  batchProfiles?: Record<string, CaseProfile>;
}) {
  const { state, notify, update, navigate, openCase } = useApp();
  const en = state.language === 'en',
    t = (zh: string, english: string) => (en ? english : zh);
  const settings =
    initialSettings ??
    (state.projectRunSettings?.cases.includes(casePath) ? state.projectRunSettings : undefined);
  const hadSavedSettings = useRef(!!settings);
  const savedHost =
    state.selectedCase?.dir === casePath ? state.selectedCase.profile?.run_host || '' : '';
  const serverOwned =
    state.selectedCase?.dir === casePath && !!state.selectedCase.profile?.remote_directory;
  const recoveringRun = state.running[casePath];
  const restoredHost =
    recoveringRun?.running || recoveringRun?.status === 'unknown'
      ? recoveringRun.location === 'local'
        ? ''
        : recoveringRun.progress?.host
      : undefined;
  const [host, setHost] = useState(
      typeof restoredHost === 'string' ? restoredHost : (initialHost ?? savedHost),
    ),
    [servers, setServers] = useState<Server[]>([]),
    [kernel, setKernel] = useState(''),
    [preprocessors, setPreprocessors] = useState('rust'),
    [engine, setEngine] = useState<string>(
      (batchCases ? effectiveProfile?.engine : state.selectedCase?.profile?.engine) || 'rust',
    ),
    [stage, setStage] = useState(''),
    [ranks, setRanks] = useState(settings?.ranks || '1'),
    [threads, setThreads] = useState(settings?.threads || '1'),
    [workers, setWorkers] = useState(settings?.workers || '1'),
    [scheduler, setScheduler] = useState(settings?.scheduler || 'auto'),
    [cpus, setCpus] = useState(settings?.cpus || ''),
    [force, setForce] = useState(false),
    [recordInputFingerprints, setRecordInputFingerprints] = useState(false),
    [remote, setRemote] = useState<unknown>(
      recoveringRun?.location === 'remote' && recoveringRun.status === 'unknown'
        ? (recoveringRun.progress?.remote ?? {
            status: { state: 'unknown' },
            error: recoveringRun.error,
          })
        : null,
    ),
    [recovering, setRecovering] = useState(true),
    [busy, setBusy] = useState(false),
    [serverBuild, setServerBuild] = useState<{
      progress: BuildProgress;
      since: number;
      now: number;
    } | null>(null),
    [monitor, setMonitor] = useState(false),
    [lastCheck, setLastCheck] = useState<{ at: number; text: string } | null>(null),
    // 服务器忙不忙（`remote_load`）：选中服务器时读一次，"核对服务器状态"时再读。初值不用 null：
    // 只记哪台服务器的、什么时候读到的。
    [serverLoad, setServerLoad] = useState<{
      host?: string;
      at?: number;
      load?: ServerLoad;
      error?: string;
    }>({}),
    [loadBusy, setLoadBusy] = useState(false);
  const lock = useRef(false),
    alive = useRef(true),
    latest = useRef(state.running),
    currentCase = useRef(casePath),
    cancelledRun = useRef<string | null>(null);
  currentCase.current = casePath;
  latest.current = state.running;
  const persisted = state.running[casePath];
  const canCancel =
    !!persisted?.run_id &&
    (persisted?.location === 'local' || !!batchCases) &&
    Object.values(state.running).some((run) => run.run_id === persisted.run_id && run.running);
  const kernelMatches =
    !!effectiveProfile &&
    [effectiveProfile, ...Object.values(batchProfiles ?? {})].every(
      (profile) =>
        !!compatibleKernel(
          state.kernels.filter((k) => k.dir === kernel),
          { ...profile, grid: profile.grid ?? undefined },
          effectiveModelFields(state.version?.overlay),
        ),
    );
  const pending =
    busy ||
    recovering ||
    !!persisted?.running ||
    !!batchCases?.some(
      (dir) => state.running[dir]?.running && state.running[dir]?.location !== 'remote',
    );
  const resultCasePath = (batchCases ?? [casePath]).find((dir) => {
    const record = state.running[dir];
    return (
      record?.status === 'succeeded' &&
      !record.running &&
      record.requested_stage !== 'mksrfdata' &&
      record.requested_stage !== 'mkinidata'
    );
  });
  const resultRun = resultCasePath ? state.running[resultCasePath] : undefined;
  const completed = !!resultRun;
  const view = restoreRunView(persisted, casePath),
    remotePermissions = remoteRunPermissions(remote);
  // 单个算例在服务器上跑时，进度区也说清服务器在做什么、算到哪一天、作业在哪：和批量页的站点行一样。
  const remoteRun = persisted?.location === 'remote' && !!remote;
  const serverActivity = remoteRun ? remoteActivity(remote, en).text : '';
  const progressData = (
    remoteRun ? (remote as { progress?: unknown }).progress : persisted?.progress
  ) as { date?: unknown } | null | undefined;
  const simulatedDate =
    typeof progressData?.date === 'string' ? progressData.date.slice(0, 10) : '';
  const jobFacts = remoteRun
    ? remoteJobFacts(remote, en).filter((fact) =>
        [
          t('服务器', 'Server'),
          t('调度系统', 'Scheduler'),
          t('申请资源', 'Requested'),
          t('提交时间', 'Submitted'),
        ].includes(fact.label),
      )
    : [];
  const progressTone = !view
    ? 'unknown'
    : ['running', 'starting', 'submitted'].includes(view.status)
      ? 'running'
      : ['succeeded', 'failed', 'cancelled'].includes(view.status)
        ? view.status
        : ['queued', 'preparing', 'submitting'].includes(view.status)
          ? 'waiting'
          : 'unknown';
  const resultsBlocked =
    ((!batchCases || !!host) && pending) ||
    disabled ||
    monitor ||
    (resultRun?.location === 'remote' &&
      !resultRun.run_case &&
      !remoteRunPermissions(resultCasePath === casePath ? remote : resultRun.progress?.remote)
        .fetch);
  useEffect(() => {
    let current = true;
    alive.current = true;
    invoke<{ servers: Server[] }>('remote_config', {})
      .then((c) => {
        if (current) {
          setServers(c.servers);
          const server = c.servers.find((entry) => entry.host === host);
          if (server && !hadSavedSettings.current) {
            setThreads(String(server.threads || 1));
            setScheduler(server.scheduler || 'auto');
            setCpus(server.cpus > 0 ? String(server.cpus) : '');
          }
        }
      })
      .catch((e) => {
        if (current) notify(String(e), true);
      });
    return () => {
      current = false;
      alive.current = false;
    };
  }, [notify, host]);
  useEffect(() => {
    let current = true;
    setRecovering(true);
    setRemote(null);
    const known = latest.current[casePath]?.progress?.remote;
    if (known) setRemote(known);
    // What each case's last run ended as is known from the ledger. A batch that finished is shown
    // finished, rather than as stations waiting to be re-checked on their server.
    let endedHere: LastRun | undefined;
    const restoreEnded = async () => {
      const dirs = batchCases ?? [casePath];
      const last = await invokeJson<Record<string, LastRun>>('project_last_runs', { dirs }).catch(
        () => ({}) as Record<string, LastRun>,
      );
      if (!current || !alive.current) return;
      endedHere = endedRun(last[casePath]) ? last[casePath] : undefined;
      update((old) => {
        const added = { ...restoredRuns(last, old.running), ...openRuns(last, old.running) };
        return Object.keys(added).length ? { running: { ...old.running, ...added } } : {};
      });
    };
    restoreEnded()
      .then(() =>
        invokeJson<Record<string, unknown> | null>('remote_job_record', { case: casePath }),
      )
      .then(async (rawRecord) => {
        // The job file outlives the run. When the ledger says the run ended, the file only gives
        // the server and job name, so fetching stays possible and nothing waits for a re-check.
        const record =
          rawRecord && endedHere
            ? {
                record: rawRecord,
                status: {
                  state: 'finished',
                  exit_code: ['succeeded', 'completed'].includes(endedHere.status) ? 0 : 1,
                },
                pending: false,
                needs_reconciliation: false,
              }
            : rawRecord;
        if (!current || !alive.current) return;
        const previous = latest.current[casePath];
        setRemote(
          record ??
            (previous?.location === 'remote' && previous.status === 'unknown'
              ? (previous.progress?.remote ?? {
                  status: { state: 'unknown' },
                  error: previous.error,
                })
              : null),
        );
        if (record && !endedHere)
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
        else {
          const runs = await invokeJson<
            {
              status: string;
              needs_reconciliation?: boolean;
              operation_id?: string;
              run_case?: string;
              provenance?: { ok?: boolean; stage?: string | null };
            }[]
          >('model_case_runs', { dir: casePath });
          if (!current || !alive.current) return;
          const last = runs[0];
          if (
            last?.run_case &&
            last.provenance?.ok === true &&
            !last.needs_reconciliation &&
            ['succeeded', 'completed'].includes(last.status)
          )
            update((old) =>
              old.running[casePath]
                ? {}
                : {
                    running: {
                      ...old.running,
                      [casePath]: {
                        case: casePath,
                        location: 'local',
                        run_id: last.operation_id,
                        operation_id: last.operation_id,
                        run_case: last.run_case,
                        requested_stage: last.provenance?.stage,
                        status: 'succeeded',
                        running: false,
                        logs: [],
                        stages: {},
                      },
                    },
                  },
            );
        }
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
    // The batch list is a prop rebuilt on every render; the case path identifies the batch here.
    // eslint-disable-next-line react-hooks/exhaustive-deps
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
    if (typeof restoredHost !== 'string') setHost(initialHost ?? savedHost);
  }, [casePath, savedHost, initialHost, restoredHost]);
  useEffect(() => {
    const progress = persisted?.progress;
    if (!batchCases && typeof restoredHost === 'string') setHost(restoredHost);
    if (progress?.remote) setRemote(progress.remote);
  }, [persisted?.progress, batchCases, restoredHost]);
  useEffect(() => {
    onBusyChange?.(batchCases ? busy || recovering : pending);
    return () => onBusyChange?.(false);
  }, [pending, onBusyChange, batchCases, busy, recovering]);
  // 提交服务器运行前，登录节点要先编好引擎（软件更新后几分钟到十几分钟）。站点处在"提交中"时每 5 秒
  // 问一次服务器在不在编、编到哪了；在编就显示和新建模拟里一样的进度卡片。
  const submittingOn = submittingHost(host, batchCases ?? [casePath], state.running);
  useEffect(() => {
    setServerBuild(null);
    if (!submittingOn) return;
    let current = true;
    const poll = () =>
      invoke<BuildProgress>('remote_engine_progress', { host: submittingOn })
        .then((progress) => {
          if (!current) return;
          const now = Date.now();
          setServerBuild((old) =>
            progress?.engine ? { progress, since: old?.since ?? now, now } : null,
          );
        })
        .catch(() => {});
    void poll();
    const timer = setInterval(() => void poll(), 5000);
    return () => {
      current = false;
      clearInterval(timer);
    };
  }, [submittingOn]);
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
      if (recovering) throw new Error(bilingual('正在恢复运行记录', 'Restoring run records'));
      if (disabled) throw new Error(bilingual('请先保存算例设置', 'Save case settings first'));
      const targets = [...new Set(batchCases ?? [casePath])];
      if (!targets.length) throw new Error(t('请选择算例', 'Select a case'));
      if (targets.some((dir) => latest.current[dir]?.running))
        throw new Error(
          bilingual('算例已经在运行或等待核对', 'A selected case is already running'),
        );
      if (host && remoteSubmissionBlocked(remote))
        throw new Error(
          bilingual('已有服务器作业：先核对终态，不重复提交', 'Check existing job first'),
        );
      if (!kernelMatches)
        throw new Error(
          bilingual('请选择与当前模型匹配的编译配置', 'Select a compatible build configuration'),
        );
      const nums = [
        engine === 'rust' ? '1' : ranks,
        siteBatch ? '1' : threads,
        siteBatch ? String(stationConcurrency) : workers,
      ].map(Number);
      if (nums.some((n) => !Number.isSafeInteger(n) || n < 1) || nums[1] > 256)
        throw new Error(
          t(
            '进程与并发数须为正整数，线程数须为 1–256 的整数',
            'Ranks and workers must be positive integers; threads must be between 1 and 256',
          ),
        );
      if (
        host &&
        cpus &&
        (!Number.isInteger(Number(cpus)) || Number(cpus) < 1 || Number(cpus) > 1024)
      )
        throw new Error(
          t('申请 CPU 数须为 1–1024 的整数', 'Requested CPUs must be an integer from 1 to 1024'),
        );
      const runId = crypto.randomUUID();
      cancelledRun.current = null;
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
            requested_stage: stage || null,
            ...(host ? { progress: { host } } : {}),
          } satisfies RunState,
        ]),
      );
      latest.current = { ...latest.current, ...initial };
      update((old) => ({ running: { ...old.running, ...initial } }));
      let submitted = false;
      const prepared: string[] = [];
      const preparationCancelled = () => {
        if (cancelledRun.current !== runId) return false;
        patchRun(
          targets.filter((dir) => latest.current[dir]?.running),
          runId,
          {
            status: 'cancelled',
            running: false,
          },
        );
        return true;
      };
      try {
        const revision = state.versionRef;
        // 用哪个版本运行就把算例绑到哪个版本，重启后仍用它（`model-binding.json`）。
        for (const dir of targets) {
          if (preparationCancelled()) return { cancelled: true };
          try {
            const binding = await invokeJson<{ version: VersionRef }>('model_migrate_case', {
              dir,
              target: revision,
            });
            if (dir === state.selectedCase?.dir)
              update((old) =>
                old.selectedCase?.dir === dir
                  ? { selectedCase: { ...old.selectedCase, binding } }
                  : {},
              );
            prepared.push(dir);
          } catch (error) {
            patchRun([dir], runId, { status: 'failed', running: false, error: String(error) });
            notify(String(error), true);
          }
          if (preparationCancelled()) return { cancelled: true };
        }
        if (!prepared.length) return { succeeded: 0, failed: targets.length };
        if (host) {
          const args = {
            runId,
            host,
            kernel,
            stage: stage || null,
            force,
            recordInputFingerprints,
            engine,
            preprocessors,
            ranks: nums[0],
            threads: nums[1],
            // Server site batches: the requested CPUs are the stations running at once, one each.
            // Rust 引擎只有一个进程，核数就按线程数申请：不让服务器设置里的 CPU 数盖掉它，
            // 免得多开的线程挤在少数几个核上、或多占的核空着。
            cpus: siteBatch ? 1 : engine === 'rust' ? nums[1] : cpus ? Number(cpus) : null,
            scheduler,
            modelVersion: null,
            modelRevision: revision,
          };
          if (batchCases) {
            drivenBatches.add(runId);
            try {
              await runRemoteBatch({
                cases: prepared,
                // 站点批量按节点打包（`colm_remote::pack`）：同时跑多少由载体按节点核数决定，所以站点
                // 一次全部交进服务器的队列。按"同时运行的站点数"一个个等着交时，land02 上 48 核的节点
                // 每次只在跑几个站点，载体还因 60 s 没有新站点而退出、重新排队（2026-10-19）。
                workers: siteBatch ? prepared.length : nums[2],
                // Several submissions and status checks at once: each station costs seconds of SSH.
                // The two phases alternate, so the larger stays under the ten sessions sshd allows.
                submitConcurrency: 5,
                pollConcurrency: 6,
                stopped: () => stoppedBatches.has(runId) || cancelledRun.current === runId,
                record: (dir) => invokeJson('remote_job_record', { case: dir }),
                status: (dir) => invokeJson('remote_status', { case: dir }),
                // 站点批量：分区独占的服务器上，同一批的站点放进同一个节点队列，不再一个站点占一个节点。
                submit: (dir) =>
                  invokeJson('remote_run', {
                    ...args,
                    case: dir,
                    ...(siteBatch ? { pack: true } : {}),
                  }),
                change: (dir, status, result, error) => {
                  const record =
                    result ??
                    (status === 'unknown' ? { status: { state: 'unknown' }, error } : undefined);
                  patchRun([dir], runId, {
                    status,
                    running: !['succeeded', 'failed', 'cancelled'].includes(status),
                    ...(record ? { progress: { host, remote: record } } : {}),
                    ...(error ? { error } : {}),
                  });
                  if (dir === casePath && record && alive.current) setRemote(record);
                },
              });
            } finally {
              drivenBatches.delete(runId);
            }
            return;
          }
          submitted = true;
          patchRun(prepared, runId, { status: 'submitting' });
          const result = await invokeJson<Record<string, unknown>>('remote_run', {
            ...args,
            case: casePath,
          });
          patchRun(prepared, runId, { status: 'submitted', progress: { host, remote: result } });
          if (alive.current) setRemote(result);
          return result;
        }
        patchRun(prepared, runId, { status: batchCases ? 'queued' : 'starting' });
        submitted = true;
        const args = {
          runId,
          kernel,
          force,
          recordInputFingerprints,
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
                cases: prepared,
                maxConcurrent: nums[2],
              })
            : await invoke<number>('run_case', { ...args, case: prepared[0] });
        const ok =
          typeof result === 'number'
            ? result === 0
            : result.failed === 0 && result.succeeded === prepared.length;
        update((old) => ({ running: settleRuns(old.running, prepared, runId, ok) }));
        return result;
      } catch (error) {
        if (host && batchCases) throw error;
        patchRun(
          targets.filter((dir) => latest.current[dir]?.running),
          runId,
          {
            status: host && submitted ? 'unknown' : 'failed',
            running: !!host && submitted,
            error: String(error),
          },
        );
        if (host && submitted && alive.current)
          setRemote({ status: { state: 'unknown' }, error: String(error) });
        throw error;
      }
    });
  }
  async function remoteAction(
    command: 'remote_status' | 'remote_cancel' | 'remote_fetch',
    target = casePath,
    quiet = false,
    // The caller already holds the panel lock (the Check button checks several stations at once).
    held = false,
  ) {
    const targetRemote = target === casePath ? remote : latest.current[target]?.progress?.remote;
    const permissions = remoteRunPermissions(targetRemote);
    if (!held && (lock.current || monitor)) return;
    if (command === 'remote_cancel' && !permissions.cancel)
      throw new Error(
        bilingual('先核对服务器任务身份与运行状态', 'Reconcile job identity and state first'),
      );
    if (command === 'remote_fetch' && !permissions.fetch)
      throw new Error(bilingual('先核对服务器任务的真实终态', 'Confirm terminal state first'));
    if (!held) lock.current = true;
    // Automatic checks run in the background without disabling the panel.
    if (!quiet && !held) setMonitor(true);
    const expectedRunId = latest.current[target]?.run_id;
    try {
      let result = await invokeJson<Record<string, unknown>>(
        command,
        command === 'remote_fetch'
          ? { case: target, host: null, all: false, overwriteNewer: false }
          : { case: target },
      );
      const fetchedCase =
        command === 'remote_fetch' && typeof result.run_case === 'string'
          ? result.run_case
          : undefined;
      if (command === 'remote_cancel' || command === 'remote_fetch')
        result = await invokeJson<Record<string, unknown>>('remote_status', { case: target });
      const merged = reconcileRemoteRun(targetRemote, result);
      if (alive.current && currentCase.current === target) setRemote(merged);
      if (quiet && remoteRunPermissions(merged).terminal && !permissions.terminal)
        notify(
          t('服务器作业已结束，可以取回结果。', 'The server job finished; results can be fetched.'),
        );
      update((old) => {
        const previous = old.running[target];
        const known = previous?.progress?.remote as Record<string, unknown> | undefined;
        const knownRecord = (known?.record ?? known) as Record<string, unknown> | undefined;
        const observed = result.record as Record<string, unknown> | undefined;
        const knownOperation =
          previous?.operation_id ??
          knownRecord?.operation_id ??
          (knownRecord?.provenance as Record<string, unknown> | undefined)?.operation_id;
        const discovered =
          (currentCase.current === target || !!batchCases?.includes(target)) &&
          command === 'remote_status' &&
          previous?.location === 'remote' &&
          previous.status === 'unknown' &&
          typeof knownRecord?.job !== 'string' &&
          typeof knownOperation !== 'string' &&
          typeof observed?.job === 'string' &&
          typeof observed.host === 'string' &&
          observed.host === previous.progress?.host &&
          !result.pending &&
          !result.needs_reconciliation;
        if (
          previous?.run_id !== expectedRunId ||
          (!matchesRemoteRun(previous, result) && !discovered)
        )
          return {};
        const status = result.status as
          { state?: string; exit_code?: number; log_tail?: string } | undefined;
        const provenance = (result.record as { provenance?: { stage?: string | null } } | undefined)
          ?.provenance;
        const entry: RunState = {
          ...(previous ?? {
            case: target,
            status: 'unknown',
            running: true,
            logs: [],
            stages: {},
          }),
          location: 'remote',
          ...(fetchedCase ? { run_case: fetchedCase } : {}),
          ...(provenance && 'stage' in provenance ? { requested_stage: provenance.stage } : {}),
          progress: { ...previous?.progress, remote: merged },
        };
        if (status) {
          entry.running = !remoteRunPermissions(merged).terminal;
          entry.status = remoteRunPermissions(merged).terminal
            ? remoteEndStatus(merged)
            : (status.state ?? 'unknown');
          if (status.log_tail) entry.logs = status.log_tail.split('\n');
        }
        return { running: { ...old.running, [target]: entry } };
      });
      return merged;
    } catch (e) {
      if (!quiet) notify(String(e), true);
      throw e;
    } finally {
      if (!held) lock.current = false;
      if (alive.current && !quiet && !held) setMonitor(false);
    }
  }
  // While this case's server job is unfinished, check its state every 10 seconds.
  const autoCheck = useRef(() => {});
  autoCheck.current = () => {
    if (!document.hidden) void remoteAction('remote_status', casePath, true).catch(() => {});
  };
  const watchRemote =
    !!host &&
    !batchCases &&
    !!remote &&
    !recovering &&
    !busy &&
    !monitor &&
    !remoteRunPermissions(remote).terminal;
  useEffect(() => {
    if (!watchRemote) return;
    const timer = window.setInterval(() => autoCheck.current(), REMOTE_CHECK_MS);
    return () => window.clearInterval(timer);
  }, [watchRemote, casePath]);
  // Stations of a batch whose server job is still open but that no live submission loop follows
  // (the loop ended, or this is a reopened project) are checked every 10 seconds, a few at a time,
  // so jobs that finished on the server turn into "finished" without anyone pressing Check.
  const openBatchRuns = () =>
    (batchCases ?? [])
      .filter((dir) => {
        const run = latest.current[dir];
        return (
          !!run &&
          run.location === 'remote' &&
          run.running &&
          !(run.run_id && drivenBatches.has(run.run_id))
        );
      })
      .slice(0, 8);
  const watchBatch = useRef(async () => {});
  watchBatch.current = async () => {
    if (document.hidden) return;
    for (const dir of openBatchRuns()) {
      if (!alive.current) return;
      await remoteAction('remote_status', dir, true).catch(() => {});
    }
  };
  const watchingBatch = !!host && !!batchCases?.length && !recovering;
  useEffect(() => {
    if (!watchingBatch) return;
    const timer = window.setInterval(() => void watchBatch.current(), REMOTE_CHECK_MS);
    void watchBatch.current();
    return () => window.clearInterval(timer);
    // The watcher reads the current runs through refs.
  }, [watchingBatch, casePath]);
  // The Check button: every open server job of the batch (or the case), with a summary shown.
  async function checkServerStatus() {
    // Whatever happens, the answer is written under the button: a toast alone is easy to miss, and a
    // check that was skipped because the panel is busy used to say nothing at all.
    const say = (text: string, error = false) => {
      setLastCheck({ at: Date.now(), text });
      notify(text, error);
    };
    // 作业状态之外再看服务器忙不忙：和逐个核对作业同时读，不占提交锁。
    const loading = refreshLoad();
    const withLoad = async (text: string) => {
      const load = await loading;
      const partition = servers.find((server) => server.host === host)?.partition ?? '';
      return load
        ? `${text.replace(/[。.]$/, '')}${t('。服务器：', '. Server: ')}${serverLoadSummary(load, en, partition)}`
        : text;
    };
    if (lock.current || monitor) {
      const standing = runCountsText(
        (batchCases ?? [casePath]).map((dir) => latest.current[dir]),
        en,
      );
      say(
        t(
          `正在提交或核对，请稍候；状态会自动更新。${standing ? `现在：${standing}。` : ''}`,
          `Submitting or checking right now; the status updates by itself.${standing ? ` Now: ${standing}.` : ''}`,
        ),
      );
      return;
    }
    const open = batchCases ? openBatchRuns() : [];
    const targets = open.length ? open : [casePath];
    const results: unknown[] = [];
    const errors: string[] = [];
    // Five stations at a time under one lock: one by one, 40 stations took minutes.
    lock.current = true;
    setMonitor(true);
    try {
      await eachLimited(targets, 5, async (dir) => {
        try {
          const merged = await remoteAction('remote_status', dir, true, true);
          if (merged) results.push(merged);
        } catch (error) {
          errors.push(String(error));
          notify(String(error), true);
        }
      });
    } finally {
      lock.current = false;
      if (alive.current) setMonitor(false);
    }
    if (results.length) say(await withLoad(remoteStatusSummary(results, en)));
    else if (errors.length) say(errors[0], true);
    else
      say(
        await withLoad(
          t(
            '没有需要核对的服务器作业：这些站点都已结束，或还没有提交。',
            'No server job needs checking: the stations have finished or were not submitted.',
          ),
        ),
      );
  }
  async function cancel() {
    try {
      const id = latest.current[casePath]?.run_id;
      if (id) {
        cancelledRun.current = id;
        stoppedBatches.add(id);
      }
      const targets = id
        ? Object.entries(latest.current)
            .filter(([, run]) => run.run_id === id && run.running)
            .map(([dir]) => dir)
        : [casePath];
      if (host && batchCases) {
        let count = 0;
        await eachLimited(targets, 5, async (dir) => {
          const record = latest.current[dir]?.progress?.remote;
          if (!record) return;
          try {
            let result = await invokeJson<Record<string, unknown>>('remote_status', { case: dir });
            if (!matchesRemoteRun(latest.current[dir], result)) return;
            if (remoteRunPermissions(result).cancel) {
              await invokeJson('remote_cancel', { case: dir });
              count++;
              result = await invokeJson('remote_status', { case: dir });
            }
            if (!matchesRemoteRun(latest.current[dir], result)) return;
            const terminal = remoteRunPermissions(result).terminal;
            if (id)
              patchRun([dir], id, {
                running: !terminal,
                status: terminal ? remoteEndStatus(result) : 'unknown',
                progress: { host, remote: result },
              });
          } catch (error) {
            notify(String(error), true);
          }
        });
        notify(
          t(
            `已停止继续提交，并向 ${count} 个服务器作业请求取消；已提交作业须核对终态。`,
            `Submission stopped; cancellation requested for ${count} jobs. Reconcile submitted jobs before restarting.`,
          ),
        );
        return { stop_requested: count, terminal: false };
      }
      const count = await invoke<number>('cancel_runs', { cases: targets });
      notify(
        t(
          `已向 ${count} 个进程发送停止请求；等待真实退出`,
          `Stop requested for ${count} processes; waiting for exit`,
        ),
      );
      return { stop_requested: count, terminal: false };
    } catch (e) {
      notify(String(e), true);
      throw e;
    }
  }
  async function viewResults() {
    if (!resultCasePath || resultsBlocked || ((!batchCases || !!host) && lock.current)) return;
    if (resultRun?.location === 'remote' && !resultRun.run_case) {
      if (lock.current) return;
      const fetched = await remoteAction('remote_fetch', resultCasePath);
      const stage = (fetched as { record?: { provenance?: { stage?: string } } } | undefined)
        ?.record?.provenance?.stage;
      if (!fetched || stage === 'mksrfdata' || stage === 'mkinidata') return;
    }
    if (
      alive.current &&
      currentCase.current === casePath &&
      latest.current[resultCasePath]?.run_id === resultRun?.run_id
    ) {
      if (batchCases) await openCase(resultCasePath, 'results');
      else navigate('case', 'results');
    }
  }

  async function changeHost(value: string) {
    if (pending || batchCases || serverOwned || value === host) return;
    const dir = casePath;
    try {
      await action(async () => {
        await invoke('set_case_run_host', { dir, host: value || null });
        update((old) =>
          old.selectedCase?.dir === dir && old.selectedCase.profile
            ? {
                selectedCase: {
                  ...old.selectedCase,
                  profile: { ...old.selectedCase.profile, run_host: value || null },
                },
              }
            : {},
        );
        if (alive.current && currentCase.current === dir) setHost(value);
      });
    } catch {
      // action reports the failure; keep the previously saved host.
    }
  }

  const resultsLabel = batchCases
    ? t('查看站点结果', 'View station results')
    : t('下一步：查看结果', 'Next: view results');

  // A batch of station cases on a server runs as many stations at once as CPUs are requested,
  // each station with one CPU and one thread. Batches with spatial cases keep their settings.
  const siteBatch =
    !!batchCases &&
    !!host &&
    batchCases.every((dir) => state.cases?.some((entry) => entry.dir === dir && !entry.spatial));
  const runServer = servers.find((server) => server.host === host);
  const stationConcurrency =
    Number(cpus) > 0 ? Number(cpus) : runServer?.cpus || runServer?.threads || 1;
  function back() {
    if (!onBack || pending) return;
    if (batchCases)
      update({
        projectRunSettings: {
          root: state.root,
          cases: batchCases,
          batch: true,
          host,
          ranks,
          threads,
          workers,
          scheduler,
          cpus,
        },
      });
    onBack();
  }
  // 快速换服务器时，慢的那次回答不能盖掉当前服务器的，也不能提前收起"读取中"。
  const loadTarget = useRef(host);
  loadTarget.current = host;
  /** Reads how busy the server is; returns the load so a status check can summarise it. */
  async function refreshLoad(): Promise<ServerLoad | undefined> {
    const target = host;
    if (!target) return undefined;
    const mine = () => alive.current && loadTarget.current === target;
    setLoadBusy(true);
    try {
      const load = await invoke<ServerLoad>('remote_load', { host: target });
      if (mine()) setServerLoad({ host: target, at: Date.now(), load });
      return load;
    } catch (error) {
      if (mine()) setServerLoad({ host: target, at: Date.now(), error: String(error) });
      return undefined;
    } finally {
      if (mine()) setLoadBusy(false);
    }
  }

  useEffect(() => {
    if (!batchCases) return;
    const preferences = {
      root: state.root,
      cases: batchCases,
      batch: true,
      host,
      ranks,
      threads,
      workers,
      scheduler,
      cpus,
    };
    if (JSON.stringify(state.projectRunSettings) === JSON.stringify(preferences)) return;
    update((old) =>
      old.root !== preferences.root ||
      JSON.stringify(old.projectRunSettings) === JSON.stringify(preferences)
        ? {}
        : { projectRunSettings: preferences },
    );
  }, [
    batchCases,
    state.root,
    state.projectRunSettings,
    host,
    ranks,
    threads,
    workers,
    scheduler,
    cpus,
    update,
  ]);

  // 换了服务器就读一次它的负载；放在所有 hook 的最后，测试按顺序取前面的 effect。
  useEffect(() => {
    if (host && serverLoad.host !== host) void refreshLoad();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [host]);
  const fields: UiField[] = [
    {
      id: 'run-host',
      label: bilingual('运行位置', 'Machine'),
      type: 'choice' as const,
      value: host,
      disabled: pending || serverOwned || !!batchCases,
      options: [
        { value: '', label: bilingual('本机', 'Local') },
        ...(host && !servers.some((server) => server.host === host)
          ? [{ value: host, label: `${host} · ${t('未配置', 'Not configured')}` }]
          : []),
        ...servers.map((s) => ({ value: s.host, label: s.host })),
      ],
      set: changeHost,
    },
    ...[
      ...(engine === 'fortran'
        ? [['ranks', t('每个算例的 MPI 进程数', 'MPI ranks per case'), ranks, setRanks] as const]
        : []),
      ...((engine === 'rust' || preprocessors === 'rust') && !siteBatch
        ? [['threads', t('每个算例的线程数', 'Threads per case'), threads, setThreads] as const]
        : []),
      ...(batchCases && !siteBatch
        ? [['workers', t('同时运行站点数', 'Concurrent stations'), workers, setWorkers] as const]
        : []),
      // 只有 Fortran 引擎（MPI）才要另填每个进程的 CPU 数；Rust 引擎按线程数申请。
      ...(host && !siteBatch && engine === 'fortran'
        ? [
            [
              'cpus',
              t(
                '每个 MPI 进程申请 CPU 数（空白沿用服务器设置）',
                'Requested CPUs per MPI rank (blank uses server setting)',
              ),
              cpus,
              setCpus,
            ] as const,
          ]
        : []),
    ].map(([id, label, value, set]) => ({
      id: `run-${id}`,
      label,
      value,
      type: 'number' as const,
      set,
    })),
    ...(host
      ? [
          {
            id: 'run-scheduler',
            label: t('提交方式', 'Submission mode'),
            type: 'choice' as const,
            value: scheduler,
            options: ['auto', 'bare', 'slurm', 'pbs', 'lsf'].map((value) => ({
              value,
              label:
                value === 'bare'
                  ? t('直接在服务器运行', 'Run directly on server')
                  : value === 'auto'
                    ? t('自动识别调度器', 'Detect scheduler')
                    : value.toUpperCase(),
            })),
            set: setScheduler,
          },
        ]
      : []),
    {
      id: 'run-input-fingerprints',
      label: t('记录其他文件指纹（默认关闭）', 'Record other file fingerprints (off by default)'),
      value: String(recordInputFingerprints),
      type: 'switch' as const,
      set: (v: string) => setRecordInputFingerprints(v === 'true'),
    },
    ...(!batchCases
      ? [
          {
            id: 'run-kernel',
            label: bilingual('编译配置', 'Build configuration'),
            type: 'choice' as const,
            value: kernel,
            options: state.kernels.map((k) => ({ value: k.dir, label: k.preset })),
            set: setKernel,
          },
          {
            id: 'run-engine',
            label: bilingual('引擎', 'Engine'),
            type: 'choice' as const,
            value: engine,
            options: ['rust', 'fortran'].map((value) => ({ value, label: value })),
            set: setEngine,
          },
          {
            id: 'run-preprocessors',
            label: bilingual('前处理引擎', 'Preprocessors'),
            type: 'choice' as const,
            value: preprocessors,
            options: ['rust', 'fortran'].map((value) => ({ value, label: value })),
            set: setPreprocessors,
          },
          {
            id: 'run-stage',
            label: bilingual('阶段', 'Stage'),
            type: 'choice' as const,
            value: stage,
            options: [
              { value: '', label: bilingual('全部', 'All') },
              ...['mksrfdata', 'mkinidata', 'colm'].map((value) => ({ value, label: value })),
            ],
            set: setStage,
          },
          {
            id: 'run-force',
            label: bilingual('重新运行全部阶段', 'Force'),
            value: String(force),
            type: 'switch' as const,
            set: (v: string) => setForce(v === 'true'),
          },
        ]
      : []),
  ].map((f) => ({ ...f, disabled: pending || ('disabled' in f && f.disabled === true) }));
  usePageContract(
    {
      title: batchCases ? t('批量运行', 'Batch run') : bilingual('运行', 'Run'),
      step: 'run',
      kernelDir: effectiveProfile ? kernel || null : null,
      preprocessors,
      status: pending ? 'busy' : (persisted?.status ?? 'ready'),
      details: batchCases
        ? {
            runs: batchCases.map((dir) => ({
              run: restoreRunView(state.running[dir], dir),
              remote: state.running[dir]?.progress?.remote,
              error: state.running[dir]?.error,
            })),
          }
        : { run: view, remote, error: persisted?.error },
      fields,
      actions: [
        ...(onBack
          ? [
              {
                id: 'run-back',
                label: t('上一步', 'Back'),
                kind: 'click' as const,
                enabled: !pending,
                run: back,
              },
            ]
          : []),
        {
          id: 'run-view-results',
          label: resultsLabel,
          kind: 'click',
          enabled: !!completed && !resultsBlocked,
          run: viewResults,
        },
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
          label: bilingual('停止运行', 'Stop'),
          kind: 'commit',
          enabled: canCancel,
          run: cancel,
        },
        ...(remote
          ? [
              {
                id: 'remote-status',
                label: bilingual('核对服务器状态', 'Check'),
                kind: 'click' as const,
                enabled: !monitor,
                run: () => checkServerStatus(),
              },
              {
                id: 'remote-cancel',
                label: bilingual('取消服务器任务', 'Cancel'),
                kind: 'commit' as const,
                enabled: !monitor && remotePermissions.cancel,
                run: () => remoteAction('remote_cancel'),
              },
              {
                id: 'remote-fetch',
                label: bilingual('取回结果', 'Fetch'),
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
      {f.id === 'run-stage' ? (
        <div className="actions" role="group" aria-label={f.label}>
          {f.options?.map((option) => (
            <Button
              key={option.value}
              variant={f.value === option.value ? 'primary' : 'secondary'}
              aria-pressed={f.value === option.value}
              disabled={!!f.disabled}
              onClick={() => f.set(option.value)}
            >
              {option.label}
            </Button>
          ))}
        </div>
      ) : f.type === 'choice' ? (
        <Select
          value={String(f.value)}
          disabled={!!f.disabled}
          onChange={(e) => f.set(e.target.value)}
        >
          {f.options?.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </Select>
      ) : f.type === 'switch' ? (
        <Input
          type="checkbox"
          checked={f.value === 'true'}
          disabled={pending || f.disabled}
          onChange={(e) => f.set(String(e.target.checked))}
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
    'input-check': t('检查实际输入', 'Checking actual inputs'),
    'input-verify': t('核验运行输入', 'Verifying run inputs'),
    'preparing the engine': t('准备运行环境', 'Preparing the environment'),
    'installing Rust/Cargo': t('安装 Rust/Cargo', 'Installing Rust/Cargo'),
    'installing CMake': t('安装 CMake', 'Installing CMake'),
    'building the Rust engine': t('编译模型引擎', 'Building the model engine'),
    mksrfdata: t('准备地表数据', 'Surface data'),
    mkinidata: t('准备初始场', 'Initial conditions'),
    colm: t('模型计算', 'Simulation'),
  };
  return (
    <Card title={t('检查并运行', 'Review and run')} className="simulation-run-card">
      <div className="run-review">
        <dl className="run-review-facts">
          <div>
            <dt>{t('算例', 'Case')}</dt>
            <dd>{summary?.name ?? state.selectedCase?.entry.name}</dd>
          </div>
          <div>
            <dt>{t('时段', 'Period')}</dt>
            <dd>{summary?.period ?? '—'}</dd>
          </div>
          <div>
            <dt>{t('模型', 'Model')}</dt>
            <dd>
              {state.version?.name ?? '—'} · r{state.versionRef.revision}
            </dd>
          </div>
          <div>
            <dt>{t('引擎', 'Engine')}</dt>
            <dd>{engine === 'rust' ? 'Rust' : 'Fortran'}</dd>
          </div>
        </dl>
        <ul className="run-review-checks">
          <li className={disabled ? 'check-pending' : 'check-fact'}>
            <span aria-hidden="true">{disabled ? '○' : '✓'}</span>
            {disabled
              ? t('设置待保存或读取', 'Settings pending')
              : t('算例设置已保存', 'Case settings saved')}
          </li>
          <li className={kernelMatches ? 'check-fact' : 'check-pending'}>
            <span aria-hidden="true">{kernelMatches ? '✓' : '○'}</span>
            {kernelMatches
              ? t('编译配置已匹配', 'Build configuration matched')
              : t('尚无匹配的编译配置', 'No matching build configuration')}
          </li>
          <li className="check-note">
            <span aria-hidden="true">i</span>
            {t('开始运行时检查输入文件', 'Input files are checked at start')}
          </li>
        </ul>
      </div>
      {serverBuild && submittingOn && (
        <EngineProgress
          host={submittingOn}
          phase="building"
          waited={elapsedLabel(serverBuild.now - serverBuild.since, en)}
          progress={serverBuild.progress}
          en={en}
          hint={t(
            '编好后自动开始提交；同一批的其余站点会等它编完，不会重复编。',
            'Submission starts once it is built; the other stations wait for this build instead of repeating it.',
          )}
        />
      )}
      {/* 运行位置和提交方式放在同一行：两者一起决定作业交到哪、怎么交。 */}
      <div className="run-settings">
        {render({
          ...fields[0],
          label: t('运行位置', 'Run location'),
          options: [
            { value: '', label: t('本机', 'Local') },
            ...(host && !servers.some((server) => server.host === host)
              ? [{ value: host, label: `${host} · ${t('未配置', 'Not configured')}` }]
              : []),
            ...servers.map((s) => ({ value: s.host, label: s.host })),
          ],
        })}
        {['run-scheduler', 'run-ranks', 'run-threads', 'run-workers', 'run-cpus'].flatMap((id) => {
          const field = fields.find((entry) => entry.id === id);
          return field ? [render(field)] : [];
        })}
      </div>
      {host &&
        !siteBatch &&
        (scheduler === 'slurm' ||
          (scheduler === 'auto' &&
            servers.find((server) => server.host === host)?.scheduler === 'slurm')) &&
        !servers.find((server) => server.host === host)?.memory_gb && (
          <p className="muted">
            {t(
              '没填内存：在按整节点给内存的 Slurm 分区（如 land02）上，会按申请的核数分摊节点内存，例如 8 个线程约拿到节点内存的 8/96。大区域或高分辨率的模拟请在计算资源里填"内存 GB"。',
              'No memory set: on Slurm partitions that hand out whole-node memory (such as land02) the node memory is shared out by the cores requested, e.g. 8 threads get about 8/96 of it. For large or high-resolution regions, set "Memory GB" in Compute resources.',
            )}
          </p>
        )}
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
            batchCases
              ? '没有与当前模型匹配的编译配置，请在模型版本页检查。'
              : '没有与当前模型匹配的编译配置。可在下方选择，或在模型版本页检查。',
            batchCases
              ? 'No compatible build configuration. Check Model versions.'
              : 'Select a compatible build configuration below, or check Model versions.',
          )}
        </Notice>
      )}
      {host && batchCases && (
        <Notice>
          {siteBatch
            ? t(
                '站点按节点打包运行：所有站点一次排进服务器上的队列，每个节点作业同时跑满这个节点的核数（站点数多就自动多开节点），跑完一个接着跑下一个。请保持此窗口打开直到站点都交上去；关闭窗口后已交上去的站点照常运行，没交上去的停止。',
                'Stations are packed per node: all of them join a queue on the server, each node job runs as many as the node has cores (more stations open more node jobs) and takes the next as one finishes. Keep this window open until every station is sent; closing it leaves sent stations running and stops the rest.',
              )
            : t(
                '同时运行数包含排队中的作业。请保持此窗口打开以继续提交；关闭窗口后已提交作业继续运行，尚未提交的站点停止。',
                'Concurrency includes queued jobs. Keep this window open to submit remaining stations. Closing it leaves submitted jobs running and stops unsent stations.',
              )}
        </Notice>
      )}
      {host && (
        <ServerLoadPanel
          host={host}
          load={serverLoad.host === host ? serverLoad.load : undefined}
          error={serverLoad.host === host ? serverLoad.error : undefined}
          at={serverLoad.host === host ? serverLoad.at : undefined}
          busy={loadBusy}
          partition={servers.find((server) => server.host === host)?.partition}
          en={en}
          onRefresh={() => void refreshLoad()}
        />
      )}
      {!batchCases && (
        <section className="run-engine" aria-label={t('引擎与计算阶段', 'Engine and stages')}>
          <h3>{t('引擎与计算阶段', 'Engine and stages')}</h3>
          <p className="muted">
            {t(
              '按算例自动匹配编译配置；不同配置均属于同一模型内核。',
              'The case automatically selects a build configuration of the same model kernel.',
            )}
          </p>
          <div className="form-grid">
            {fields
              .slice(1)
              .filter(
                (f) =>
                  ![
                    'run-ranks',
                    'run-threads',
                    'run-workers',
                    'run-cpus',
                    'run-scheduler',
                    'run-input-fingerprints',
                  ].includes(f.id),
              )
              .map(render)}
          </div>
        </section>
      )}
      <Advanced title={t('更多选项与说明', 'More options and notes')}>
        {render(fields.find((field) => field.id === 'run-input-fingerprints')!)}
        <p className="muted">
          {t(
            '驱动数据始终跳过内容指纹，只记录数据集、路径、前缀和时段。此开关控制其他输入、程序及输出文件；关闭时仅记录来源和配置，开启后校验这些文件。模拟阶段会重新运行，驱动内容不作一致性认证。',
            'Forcing data never receive content fingerprints; only their dataset, paths, prefixes and period are recorded. This switch controls other inputs, programs and outputs. When off, only sources and settings are recorded; when on, those files are verified. Simulation runs again, and forcing contents are not certified.',
          )}
        </p>
        <p className="muted">
          {t(
            '点击“开始运行”时检查输入文件；若有缺失，将显示具体原因。',
            'Input files are checked when you start the run; missing inputs are reported.',
          )}
        </p>
        {host && (
          <p className="muted">
            {t(
              '首次源码构建若缺少 Rust/Cargo 或 CMake，会从官方下载安装到服务器用户目录，不需要 sudo。需要服务器联网并已有 C 编译器；已有工具直接复用。',
              'The first source build installs missing Rust/Cargo or CMake from official downloads into your server account, without sudo. Internet access and a C compiler are required; existing tools are reused.',
            )}
          </p>
        )}
      </Advanced>
      <NamelistPreview
        key={casePath}
        casePaths={batchCases ?? [casePath]}
        disabled={disabled}
        en={en}
      />
      <footer className="simulation-footer">
        {onBack ? (
          <Button disabled={pending} onClick={back}>
            {t('上一步', 'Back')}
          </Button>
        ) : (
          <span />
        )}
        <div className="actions">
          {canCancel && (
            <Button variant="danger" onClick={() => void cancel().catch(() => {})}>
              {t('停止运行', 'Stop run')}
            </Button>
          )}
          <Button
            variant={completed ? 'secondary' : 'primary'}
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
          {completed && (
            <Button
              variant="primary"
              disabled={resultsBlocked}
              onClick={() => void viewResults().catch((e) => notify(String(e), true))}
            >
              {resultsLabel}
            </Button>
          )}
        </div>
      </footer>
      {view && !batchCases && (
        <section className="simulation-monitor" aria-live="polite">
          <div className="run-progress-head">
            <h3>{t('运行进度', 'Run progress')}</h3>
            <span className={`run-progress-chip is-${progressTone}`}>
              {statusLabels[view.status] ?? view.status}
            </span>
          </div>
          <p role="status" className="run-progress-status">
            {statusLabels[view.status] ?? view.status}
            {view.stage ? ` · ${stageLabels[view.stage] ?? view.stage}` : ''}
          </p>
          {serverActivity && <p className="run-progress-activity">{serverActivity}</p>}
          {!!view.total && (
            <div className="run-progress-bar">
              <progress value={view.step ?? 0} max={view.total} />
              <span>
                {`${view.step ?? 0} / ${view.total} · ${Math.floor(((view.step ?? 0) / view.total) * 100)}%`}
                {simulatedDate
                  ? t(` · 模拟到 ${simulatedDate}`, ` · simulated to ${simulatedDate}`)
                  : ''}
              </span>
            </div>
          )}
          {!!jobFacts.length && (
            <dl className="run-progress-facts">
              {jobFacts.map((fact) => (
                <div key={fact.label}>
                  <dt>{fact.label}</dt>
                  <dd>{fact.value}</dd>
                </div>
              ))}
            </dl>
          )}
          <p className="path">
            {t('运行编号', 'Run ID')}：{view.runId || t('等待核对', 'Needs verification')}
          </p>
          {persisted?.error && <Notice tone="error">{persisted.error}</Notice>}
          <div className="run-progress-log">
            <strong>{t('最新日志', 'Latest log')}</strong>
            <pre className="run-log">
              {view.lines.slice(-12).join('\n') ||
                (persisted?.location === 'remote'
                  ? t(
                      '服务器还没写出日志：作业可能还在排队或刚开始。软件每 10 秒自动核对一次。',
                      'The server has written no log yet: the job may still be queued or just starting. The app checks every 10 seconds.',
                    )
                  : t('暂无日志', 'No log yet'))}
            </pre>
          </div>
          {view.lines.length > 12 && (
            <Advanced title={t('查看完整日志', 'View the full log')}>
              <pre className="run-log">{view.lines.join('\n')}</pre>
            </Advanced>
          )}
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
            <Button disabled={monitor} onClick={() => void checkServerStatus().catch(() => {})}>
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
          {lastCheck && (
            <p className="muted" role="status">
              {t('上次核对', 'Last check')}{' '}
              {new Date(lastCheck.at).toLocaleTimeString(en ? 'en-US' : 'zh-CN')}
              {t('：', ': ')}
              {lastCheck.text}
            </p>
          )}
          <Advanced title={t('服务器作业详情', 'Server job details')}>
            <dl className="simulation-review">
              {remoteJobFacts(remote, en).map((fact) => (
                <Fragment key={fact.label}>
                  <dt>{fact.label}</dt>
                  <dd>{fact.value}</dd>
                </Fragment>
              ))}
            </dl>
            <Advanced title={t('原始记录（排错用）', 'Raw record (for troubleshooting)')}>
              <pre>{JSON.stringify(remote, null, 2)}</pre>
            </Advanced>
          </Advanced>
        </>
      )}
    </Card>
  );
}

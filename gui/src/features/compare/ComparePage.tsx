import { useEffect, useRef, useState } from 'react';
import { useApp } from '../../lib/app-state';
import { usePageContract } from '../../lib/guide';
import { invoke, invokeJson, listen } from '../../lib/ipc';
import { askConfirm } from '../../lib/confirm';
import { Button, Card, Field, Input, Notice, Select } from '../../components/ui';
import { ActionButton } from '../../components/ActionButton';
import RemotePathBrowser from '../../components/RemotePathBrowser';
import {
  benchErrorText,
  benchLogText,
  buildOpenbenchConfig,
  comparableVariables,
  commonYears,
  comparisonDir,
  comparisonName,
  parseReferences,
  type BenchReference,
  type BenchSimulation,
  type HistoryInfo,
} from './compare';

type Project = { name: string; dir?: string; config?: string | null; report: string | null };
/** 本机（`bench_status`）与服务器（`bench_remote_status`）的状态；服务器那份没有 uv 的路径，多了平台与下载工具。 */
type Status = {
  root: string;
  uv?: string | null;
  uv_own?: boolean;
  platform?: string | null;
  downloader?: string | null;
  checksum?: string | null;
  uv_version: string | null;
  uv_pinned: string;
  openbench_pinned: string;
  python: string | null;
  openbench: string | null;
  projects: Project[];
};
type Pick = { dir: string; label: string; info: HistoryInfo | null; error?: string };
type RemoteJob = {
  job: string;
  status: { state: string; exit_code?: number; log_tail: string; detail?: string | null };
  ended: string | null;
};
/** 正在服务器上跑的那个作业：记在本机，关掉页面、关掉应用再回来都能接着看。 */
type ActiveJob = { job: string; kind: 'install' | 'run'; name?: string };

/** 常用的 PyPI 镜像。国内服务器直连 PyPI 常常很慢（7920 实测十分钟装不完，清华镜像 85 秒）。 */
const INDEXES = [
  { value: '', zh: '官方 PyPI', en: 'Official PyPI' },
  { value: 'https://pypi.tuna.tsinghua.edu.cn/simple', zh: '清华镜像', en: 'Tsinghua mirror' },
  { value: 'https://mirrors.aliyun.com/pypi/simple', zh: '阿里云镜像', en: 'Aliyun mirror' },
];

const stored = (key: string) => {
  try {
    return globalThis.localStorage?.getItem(key) ?? '';
  } catch {
    return '';
  }
};
const store = (key: string, value: string) => {
  try {
    if (value) globalThis.localStorage?.setItem(key, value);
    else globalThis.localStorage?.removeItem(key);
  } catch {
    /* 存不下就只在这次会话里用 */
  }
};
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/**
 * "比较"：用 OpenBench 对几个模拟（不同项目、不同模型版本）做评估，看报告。
 * 环境按需逐级装在应用数据目录里（uv → 专用 Python → OpenBench 与全部依赖），不碰系统。
 * 也可以选一台服务器：环境、登记表、history 与评估都在那边，只把报告和指标取回来（格点算例的 history
 * 往往就在服务器上，拉回本机评估不现实）。
 */
export default function ComparePage() {
  const { state } = useApp();
  const en = state.language === 'en';
  const t = (zh: string, english: string) => (en ? english : zh);
  const shown = (e: unknown) => {
    const [zh, english] = benchErrorText(String(e));
    return t(zh, english);
  };
  // 在哪里评估：空是本机，否则是"计算资源"里配置的一台服务器。按机器记住。
  const [host, setHostState] = useState(() => stored('colm.benchHost'));
  const [servers, setServers] = useState<string[]>([]);
  // 服务器上挑目录：给哪一项挑（模拟的 history，或参考数据根目录）。
  const [browsing, setBrowsing] = useState<'sim' | 'ref' | null>(null);
  const [index, setIndexState] = useState(() => stored('colm.benchIndex'));
  const setIndex = (value: string) => {
    setIndexState(value);
    store('colm.benchIndex', value);
  };
  const [status, setStatus] = useState<Status | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [log, setLog] = useState<string[]>([]);
  const [source, setSource] = useState<'pypi' | 'local'>('pypi');
  const [localSource, setLocalSource] = useState('');
  const [references, setReferences] = useState<BenchReference[]>([]);
  const [picks, setPicks] = useState<Pick[]>([]);
  const [name, setName] = useState('comparison');
  const [from, setFrom] = useState('');
  const [to, setTo] = useState('');
  const [timRes, setTimRes] = useState('Day');
  const [chosen, setChosen] = useState<Record<string, string>>({});
  const [variableQuery, setVariableQuery] = useState('');
  const [yaml, setYaml] = useState('');
  const [sites, setSites] = useState<
    { id: string; station: string; history_dir: string; prefix: string }[]
  >([]);
  const [report, setReport] = useState<{ path: string; html: string } | null>(null);
  // 参考数据根目录：扫描用它，运行时作为 OPENBENCH_REF_ROOT 交给 OpenBench（内置登记表的路径都相对它）。
  // 按机器记住。
  // 服务器上的参考数据在服务器的路径下：每台机器各记一份。
  const refKey = (machine: string) =>
    machine ? `colm.openbenchRefRoot@${machine}` : 'colm.openbenchRefRoot';
  const [scanDir, setScanDirState] = useState(() => stored(refKey(host)));
  const setScanDir = (value: string) => {
    setScanDirState(value);
    store(refKey(host), value);
  };
  const [scanText, setScanText] = useState('');
  const [reg, setReg] = useState({
    name: '',
    rootDir: '',
    dataType: 'stn',
    timRes: 'Day',
    from: '',
    to: '',
    fulllist: '',
    variables: '',
  });
  const lock = useRef(false);
  const job = useRef<ActiveJob | null>(null);
  // 点了"停止"：服务器上先结束进程、后写下"已取消"，中间那一下查到的是"找不到结果"，不能报成失败。
  const stopping = useRef(false);
  // 页面关掉时停止轮询：作业记录留着，回到页面时由新的页面接着等（否则两个轮询同时跑、结果取两遍）。
  const alive = useRef(true);
  const LEFT = 'compare page left';
  const remote = !!host;
  const jobKey = (machine: string) => `colm.benchJob@${machine}`;

  const refresh = async (machine = host) => {
    const next = await invokeJson<Status>(
      machine ? 'bench_remote_status' : 'bench_status',
      machine ? { host: machine } : undefined,
    );
    setStatus(next);
    // 读不到登记表要说出来：原来吞掉错误、显示"0 个"，看不出是哪里坏了（2026-10-23 窗口实测）。
    if (next.openbench)
      setReferences(
        parseReferences(
          await invokeJson(
            machine ? 'bench_remote_references' : 'bench_references',
            machine ? { host: machine } : undefined,
          ).catch((e) => {
            setError(shown(e));
            return [];
          }),
        ),
      );
    else setReferences([]);
    return next;
  };
  useEffect(() => {
    void invokeJson<{ servers: { host: string }[] }>('remote_config')
      .then((config) => setServers(config.servers.map((s) => s.host)))
      .catch(() => setServers([]));
    let stop: (() => void) | undefined;
    void listen<{ stage?: string; line?: string }>('bench://log', (payload) =>
      setLog((old) => [...old.slice(-299), `[${payload.stage ?? ''}] ${payload.line ?? ''}`]),
    ).then((unlisten) => (stop = unlisten));
    return () => stop?.();
    // 只在打开页面时订阅一次日志、读一次服务器列表；状态随所选机器读（下面的 effect）。
  }, []);

  // 一次只做一件事。后端的固定句子换成当前语言；点"停止"结束的不是错误，作为提示显示（复验 C2-06）。
  const task = async (fn: () => Promise<unknown>, done?: string) => {
    if (lock.current) return;
    lock.current = true;
    setBusy(true);
    setError('');
    setNotice('');
    try {
      await fn();
      if (done) setNotice(done);
    } catch (e) {
      if (String(e).includes(LEFT)) return;
      if (benchErrorText(String(e))[0] === '已停止') setNotice(t('已停止', 'Stopped'));
      else setError(shown(e));
    } finally {
      lock.current = false;
      setBusy(false);
      void refresh().catch(() => {});
    }
  };
  // 等服务器上的作业结束：每 3 秒取一次日志末尾显示出来。作业号记在本机，页面关掉、应用关掉都不影响
  // 服务器上的作业，回来接着等。停止按钮结束的是服务器上的作业。
  const watch = async (machine: string, active: ActiveJob) => {
    job.current = active;
    stopping.current = false;
    store(jobKey(machine), JSON.stringify(active));
    let unsure = 0;
    try {
      for (;;) {
        if (!alive.current) throw new Error(LEFT);
        const reply = await invokeJson<RemoteJob>('bench_remote_job', {
          host: machine,
          job: active.job,
          lines: 80,
        });
        setLog(reply.status.log_tail.split('\n').filter(Boolean));
        const state = reply.status.state;
        if (state === 'finished') {
          if (reply.status.exit_code === 0) {
            store(jobKey(machine), '');
            return;
          }
          throw new Error(
            t(
              `服务器上的作业失败（退出码 ${reply.status.exit_code}），原因见下面的日志。`,
              `The server job failed (exit code ${reply.status.exit_code}); see the log below.`,
            ),
          );
        }
        if (reply.ended === 'cancelled' || (stopping.current && state !== 'running'))
          throw new Error('已停止');
        // 进程没了、退出码还没写下：可能正在收尾，多看两次再下结论（2026-10-23 窗口实测停止时的竞态）。
        if ((state === 'lost' || state === 'unknown') && ++unsure > 2)
          throw new Error(
            t(
              `服务器上找不到这个作业的结果（${reply.status.detail ?? state}）。`,
              `The server job ended without a result (${reply.status.detail ?? state}).`,
            ),
          );
        await sleep(3000);
      }
    } catch (e) {
      if (String(e).includes(LEFT)) throw e;
      store(jobKey(machine), '');
      throw e;
    } finally {
      job.current = null;
      stopping.current = false;
    }
  };
  const install = (steps: ('uv' | 'python' | 'openbench')[]) =>
    task(
      async () => {
        for (const step of steps) {
          const from = step === 'openbench' && source === 'local' ? localSource : null;
          const mirror = step === 'openbench' ? index || null : null;
          if (!remote) {
            await invoke('bench_install', { step, source: from, index: mirror });
            continue;
          }
          const submitted = await invokeJson<{ job: string }>('bench_remote_install', {
            host,
            step,
            source: from,
            index: mirror,
          });
          await watch(host, { job: submitted.job, kind: 'install' });
        }
      },
      t('装好了', 'Installed'),
    );
  const stop = () => {
    if (remote && job.current) {
      stopping.current = true;
      void invoke('bench_remote_cancel', { host, job: job.current.job });
    } else void invoke('bench_cancel');
  };
  const uninstall = async (what: 'env' | 'all') => {
    const ok = await askConfirm(
      what === 'all'
        ? t(
            '删掉 OpenBench、专用 Python、uv 和下载缓存？比较项目和报告不删。',
            'Remove OpenBench, the private Python, uv and the download cache? Comparisons and reports stay.',
          )
        : t(
            '删掉 OpenBench 和它的依赖（虚拟环境）？比较项目和报告不删。',
            'Remove OpenBench and its dependencies (the virtual environment)? Comparisons and reports stay.',
          ),
      { confirmLabel: t('删除', 'Remove') },
    );
    if (ok) await task(() => invoke('bench_uninstall', { what }), t('已删除', 'Removed'));
  };

  const ready = !!status?.openbench;
  const steps: ('uv' | 'python' | 'openbench')[] = [
    ...(status?.uv || status?.uv_version ? [] : (['uv'] as const)),
    ...(status?.python ? [] : (['python'] as const)),
    ...(status?.openbench ? [] : (['openbench'] as const)),
  ];

  // 报告里的默认名字：目录名；选的是 `out/<名字>/history` 时用它上一层的名字。
  const pickLabel = (dir: string) => {
    const parts = dir.split(/[/\\]/).filter(Boolean);
    const last = parts.pop() ?? 'sim';
    return last === 'history' ? (parts.pop() ?? last) : last;
  };
  // 选模拟：勾一个算例（或加一个别的目录）就读它的 history 概况，填年份与种类。
  const addPick = async (dir: string, label: string) => {
    if (picks.some((p) => p.dir === dir)) return;
    setPicks((old) => [...old, { dir, label, info: null }]);
    try {
      const info = await invokeJson<HistoryInfo>(
        remote ? 'bench_remote_inspect' : 'bench_inspect',
        remote ? { host, dir } : { dir },
      );
      setPicks((old) => old.map((p) => (p.dir === dir ? { ...p, info } : p)));
    } catch (e) {
      setPicks((old) => old.map((p) => (p.dir === dir ? { ...p, error: shown(e) } : p)));
    }
  };
  const ok = picks.filter((p): p is Pick & { info: HistoryInfo } => !!p.info);
  useEffect(() => {
    const years = commonYears(ok.map((p) => ({ label: p.label, info: p.info })));
    if (years && !from && !to) {
      setFrom(String(years[0]));
      setTo(String(years[1]));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ok.length]);
  // 换了评估的机器：状态、登记表、参考数据根目录都换成那台的；所选模拟的路径属于原来那台，清掉。
  // 那台机器上还有没等完的作业（上次关了页面或应用），接着等。
  useEffect(() => {
    store('colm.benchHost', host);
    setStatus(null);
    setReferences([]);
    setPicks([]);
    setChosen({});
    setYaml('');
    setSites([]);
    setLog([]);
    setScanDirState(stored(refKey(host)));
    void refresh(host).catch((e) => setError(shown(e)));
    const pending = host ? stored(jobKey(host)) : '';
    if (pending)
      try {
        const active = JSON.parse(pending) as ActiveJob;
        void task(
          async () => {
            await watch(host, active);
            if (active.kind === 'run' && active.name) await fetchReport(host, active.name);
          },
          t('服务器上的作业已结束', 'The server job has finished'),
        );
      } catch {
        store(jobKey(host), '');
      }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [host]);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  const allVariables = comparableVariables(
    references,
    ok.map((p) => p.info),
  );
  const variables = comparableVariables(
    references,
    ok.map((p) => p.info),
    variableQuery,
  );
  const generate = () => {
    try {
      const sims: BenchSimulation[] = ok.map((p) => ({ label: p.label, info: p.info }));
      const built = buildOpenbenchConfig({
        name,
        years: [Number(from), Number(to)],
        timRes,
        variables: chosen,
        simulations: sims,
        projectDir: comparisonDir(status?.root ?? '', name),
      });
      setYaml(built.config);
      setSites(built.sites);
      setError('');
    } catch (e) {
      setError(shown(e));
    }
  };
  const run = () =>
    task(
      async () => {
        const project = comparisonName(name);
        if (!remote) {
          const result = await invokeJson<{ report: string | null }>('bench_run', {
            name: project,
            config: yaml,
            sites,
            refRoot: scanDir || null,
          });
          if (result.report) await view(result.report);
          return;
        }
        const submitted = await invokeJson<{ job: string }>('bench_remote_run', {
          host,
          name: project,
          config: yaml,
          sites,
          refRoot: scanDir || null,
        });
        await watch(host, { job: submitted.job, kind: 'run', name: project });
        await fetchReport(host, project);
      },
      t('评估完成', 'Evaluation finished'),
    );
  // 只取回报告、指标与配置；合并出来的站点文件和中间数据留在服务器上。
  const fetchReport = async (machine: string, project: string) => {
    const fetched = await invokeJson<{ report: string | null }>('bench_remote_fetch', {
      host: machine,
      name: project,
    });
    if (fetched.report) await view(fetched.report);
  };
  const view = async (path: string) => {
    const html = await invoke<string>('bench_report', { path });
    setReport({ path, html });
  };

  usePageContract({
    title: t('比较', 'Compare'),
    step: 'compare',
    status: busy ? 'working' : error,
    fields: [
      {
        id: 'compare-name',
        label: t('比较名', 'Comparison name'),
        value: name,
        type: 'text',
        set: setName,
      },
      {
        id: 'compare-from',
        label: t('起始年', 'From year'),
        value: from,
        type: 'number',
        set: setFrom,
      },
      { id: 'compare-to', label: t('结束年', 'To year'), value: to, type: 'number', set: setTo },
    ],
    actions: [],
    details: { status, picks: picks.map((p) => ({ dir: p.dir, label: p.label, info: p.info })) },
  });

  const caseDirs = state.cases ?? [];
  return (
    <div className="page-stack">
      <header className="research-heading">
        <div>
          <h1>{t('比较与评估', 'Compare and evaluate')}</h1>
          <p>
            {t(
              '用 OpenBench 把几个模拟（不同项目、不同模型版本）放在同一批参考数据上打分，生成评估报告。"研究"负责调参数，这里负责比结果。',
              'Score several simulations (from different projects or model versions) against the same reference data with OpenBench and get an evaluation report. Research tunes parameters; this page compares results.',
            )}
          </p>
        </div>
      </header>
      {error && <Notice tone="error">{error}</Notice>}
      {notice && <Notice tone="success">{notice}</Notice>}
      <Card
        title={t('在哪里评估', 'Where to evaluate')}
        description={t(
          '选服务器时，环境、参考数据登记表、模拟的 history 和评估都在那台服务器上，日志实时显示，只把报告和指标取回本机。服务器在"计算资源"里配置；本机和服务器的登记表各管各的。',
          'With a server, the environment, the reference registry, the simulations’ history and the evaluation all stay on that server; the log is shown live and only the report and metrics come back. Servers are set up under Compute resources; this computer and each server keep their own registry.',
        )}
      >
        <Field label={t('运行位置', 'Machine')}>
          <Select value={host} disabled={busy} onChange={(e) => setHostState(e.target.value)}>
            <option value="">{t('本机', 'This computer')}</option>
            {[...new Set([...servers, ...(host ? [host] : [])])].map((value) => (
              <option key={value} value={value}>
                {t(`服务器 ${value}`, `Server ${value}`)}
              </option>
            ))}
          </Select>
        </Field>
      </Card>
      <Card
        title={t('评估环境', 'Evaluation environment')}
        description={t(
          '用 OpenBench 评估。uv、Python、OpenBench 和它的登记表、缓存全部装在软件自己的数据目录里，不碰系统里的 Python，也不和你自己装的 OpenBench 混用：先装 uv（约 20 MB，校验后才用），再装专用 Python 3.12，最后装 OpenBench 和它的全部依赖（约 400 MB，需要联网）。',
          'Evaluation uses OpenBench. uv, Python, OpenBench, its reference registry and caches all live in the app’s own data folder, apart from the system Python and any OpenBench you installed yourself: uv first (about 20 MB, verified before use), then a private Python 3.12, then OpenBench with all its dependencies (about 400 MB, needs the internet).',
        )}
      >
        <ul className="compare-status">
          {status?.root && (
            <li>
              {remote
                ? t(`安装位置（服务器 ${host}）：`, `Installed in (server ${host}): `)
                : t('安装位置：', 'Installed in: ')}
              {status.root}{' '}
              {!remote && (
                <Button onClick={() => void invoke('open_path', { path: status.root })}>
                  {t('打开', 'Open')}
                </Button>
              )}
            </li>
          )}
          {/* 状态回来之前不说"未安装"：要起两次 Python，已装好的机器上首屏会误报（复验 C2-07）。 */}
          {!status ? (
            <li>{t('正在检查已经装了什么…', 'Checking what is installed…')}</li>
          ) : (
            <>
              <li>
                {t('uv：', 'uv: ')}
                {status.uv
                  ? `${status.uv_version ?? '?'} · ${status.uv}`
                  : status.uv_version
                    ? status.uv_version
                    : t('未安装', 'not installed')}
              </li>
              {/* 服务器上下载 uv 要 curl（或 wget）和 sha256sum（或 shasum）；没有就提前说。 */}
              {remote && !status.uv_version && (!status.downloader || !status.checksum) && (
                <li>
                  {t(
                    '这台服务器缺 curl/wget 或 sha256sum/shasum，装不了 uv：请先让管理员装上。',
                    'This server lacks curl/wget or sha256sum/shasum, so uv cannot be installed; ask the administrator to add them.',
                  )}
                </li>
              )}
              {remote && !status.platform && (
                <li>
                  {t(
                    '没有给这台服务器的系统准备 uv（只支持 Linux 与 macOS 的 x86_64、aarch64）。',
                    'No uv is prepared for this server’s system (only Linux and macOS on x86_64 and aarch64).',
                  )}
                </li>
              )}
              <li>
                {t('专用 Python：', 'Private Python: ')}
                {status.python ?? t('未安装', 'not installed')}
              </li>
              <li>
                {t('OpenBench：', 'OpenBench: ')}
                {status.openbench ?? t('未安装', 'not installed')}
                {status.openbench &&
                  status.openbench !== status.openbench_pinned &&
                  t(
                    `（软件验证过的是 ${status.openbench_pinned}；装的是别的版本或本地源码，评估结果以它为准）`,
                    ` (the app was verified with ${status.openbench_pinned}; this is another version or a local source, so results follow it)`,
                  )}
              </li>
            </>
          )}
        </ul>
        <div className="form-grid">
          <Field label={t('OpenBench 从哪装', 'Install OpenBench from')}>
            <Select value={source} onChange={(e) => setSource(e.target.value as 'pypi' | 'local')}>
              <option value="pypi">
                {t(
                  `PyPI 发布版（colm-openbench ${status?.openbench_pinned ?? ''}）`,
                  `PyPI release (colm-openbench ${status?.openbench_pinned ?? ''})`,
                )}
              </option>
              <option value="local">
                {remote
                  ? t(
                      '服务器上的源码目录（可编辑安装）',
                      'Source folder on the server (editable install)',
                    )
                  : t('本地源码目录（可编辑安装）', 'Local source folder (editable install)')}
              </option>
            </Select>
          </Field>
          {source === 'local' && (
            <Field label={t('OpenBench 源码目录', 'OpenBench source folder')}>
              <div className="study-path">
                <Input value={localSource} onChange={(e) => setLocalSource(e.target.value)} />
                {!remote && (
                  <ActionButton
                    action="compare-pick-source"
                    label={t('选择…', 'Choose…')}
                    run={async () => {
                      const dir = await invoke<string | null>('pick_folder', {
                        key: 'openbench-source',
                      });
                      if (dir) setLocalSource(dir);
                    }}
                  />
                )}
              </div>
            </Field>
          )}
          {source === 'pypi' && (
            <Field label={t('从哪个 PyPI 下载', 'Package index')}>
              <Select value={index} onChange={(e) => setIndex(e.target.value)}>
                {INDEXES.map((option) => (
                  <option key={option.value} value={option.value}>
                    {t(option.zh, option.en)}
                  </option>
                ))}
              </Select>
            </Field>
          )}
        </div>
        <div className="button-row">
          <ActionButton
            action="compare-install-all"
            label={
              !status
                ? t('一键安装', 'Install everything')
                : steps.length
                  ? t(
                      `一键安装（${steps.length} 步）`,
                      `Install everything (${steps.length} steps)`,
                    )
                  : t('已全部装好', 'Everything is installed')
            }
            kind="commit"
            variant="primary"
            disabled={busy || !status || !steps.length || (source === 'local' && !localSource)}
            run={() => install(steps)}
          />
          {ready && (
            <ActionButton
              action="compare-reinstall-openbench"
              label={t('重装 OpenBench', 'Reinstall OpenBench')}
              kind="commit"
              disabled={busy || (source === 'local' && !localSource)}
              run={() => install(['openbench'])}
            />
          )}
          {busy && <Button onClick={stop}>{t('停止', 'Stop')}</Button>}
          {!remote && (status?.python || ready) && (
            <Button disabled={busy} onClick={() => void uninstall('env')}>
              {t('删掉 OpenBench', 'Remove OpenBench')}
            </Button>
          )}
          {!remote && status?.uv_own && (
            <Button variant="danger" disabled={busy} onClick={() => void uninstall('all')}>
              {t('全部删掉', 'Remove everything')}
            </Button>
          )}
        </div>
        {log.length > 0 && (
          <pre className="compare-log">
            {log
              .slice(-80)
              .map((line) => {
                // 本机日志带"[阶段] "前缀；服务器作业的日志是原样的行。
                const m = /^(\[[^\]]*\] )([\s\S]*)$/.exec(line);
                if (!en) return line;
                return m ? m[1] + benchLogText(m[2]!) : benchLogText(line);
              })
              .join('\n')}
          </pre>
        )}
      </Card>

      {ready && (
        <Card
          title={t('参考数据', 'Reference data')}
          description={t(
            `OpenBench 登记表里有 ${references.length} 个参考数据集。自己的数据先登记：扫描能认出目录结构标准的数据集；一个文件里有多个候选变量时扫描会停下来问，这时用下面的表单写清楚变量。`,
            `The OpenBench registry lists ${references.length} reference datasets. Register your own data first: a scan recognises datasets laid out in the standard structure; when a file has several candidate variables, register it with the form below and name the variable.`,
          )}
        >
          <div className="form-grid">
            <Field label={t('参考数据根目录', 'Reference data root')}>
              <div className="study-path">
                <Input value={scanDir} onChange={(e) => setScanDir(e.target.value)} />
                <ActionButton
                  action="compare-pick-reference-root"
                  label={t('选择…', 'Choose…')}
                  run={async () => {
                    if (remote) return setBrowsing('ref');
                    const dir = await invoke<string | null>('pick_folder', {
                      key: 'openbench-ref',
                    });
                    if (dir) setScanDir(dir);
                  }}
                />
              </div>
            </Field>
          </div>
          <div className="button-row">
            <ActionButton
              action="compare-scan-preview"
              label={t('预览能认出哪些（不登记）', 'Preview what is recognised (no changes)')}
              disabled={busy || !scanDir}
              run={() =>
                task(async () =>
                  setScanText(
                    remote
                      ? (
                          await invokeJson<{ output: string }>('bench_remote_scan', {
                            host,
                            dir: scanDir,
                          })
                        ).output
                      : await invoke<string>('bench_scan_preview', { dir: scanDir }),
                  ),
                )
              }
            />
          </div>
          {scanText && <pre className="compare-log">{scanText}</pre>}
          <h3 className="study-subhead">
            {t('登记一个参考数据集', 'Register a reference dataset')}
          </h3>
          <div className="form-grid">
            {(
              [
                ['name', t('名字', 'Name')],
                ['rootDir', t('数据目录', 'Data folder')],
                ['fulllist', t('站点清单 CSV（站点数据要）', 'Station list CSV (station data)')],
                ['from', t('起始年', 'From year')],
                ['to', t('结束年', 'To year')],
              ] as const
            ).map(([key, label]) => (
              <Field key={key} label={label}>
                <Input
                  value={reg[key]}
                  onChange={(e) => setReg((old) => ({ ...old, [key]: e.target.value }))}
                />
              </Field>
            ))}
            <Field label={t('类型', 'Type')}>
              <Select
                value={reg.dataType}
                onChange={(e) => setReg((old) => ({ ...old, dataType: e.target.value }))}
              >
                <option value="stn">{t('站点', 'Station')}</option>
                <option value="grid">{t('格点', 'Grid')}</option>
              </Select>
            </Field>
            <Field label={t('时间分辨率', 'Time resolution')}>
              <Select
                value={reg.timRes}
                onChange={(e) => setReg((old) => ({ ...old, timRes: e.target.value }))}
              >
                {['Hour', 'Day', 'Month', 'Year'].map((v) => (
                  <option key={v} value={v}>
                    {v}
                  </option>
                ))}
              </Select>
            </Field>
            <Field
              label={t(
                '变量（每行 变量:文件里的名字:单位[:文件名前缀[:文件名后缀]]，如 Latent_Heat:LE:W m-2；按年分文件的格点数据写上前后缀，如 Latent_Heat:slhf:W m-2:ERA5LAND_:_daymean）',
                'Variables (one per line, variable:name in file:unit[:file prefix[:file suffix]], e.g. Latent_Heat:LE:W m-2; for gridded data split by year give the prefix and suffix, e.g. Latent_Heat:slhf:W m-2:ERA5LAND_:_daymean)',
              )}
            >
              <textarea
                className="code-files-text compare-text"
                rows={3}
                value={reg.variables}
                onChange={(e) => setReg((old) => ({ ...old, variables: e.target.value }))}
              />
            </Field>
          </div>
          <div className="button-row">
            <ActionButton
              action="compare-register"
              label={t('登记', 'Register')}
              kind="commit"
              disabled={busy || !reg.name || !reg.rootDir || !reg.variables.trim()}
              run={() =>
                task(
                  () =>
                    invoke(remote ? 'bench_remote_register' : 'bench_register', {
                      ...(remote ? { host } : {}),
                      name: reg.name.trim(),
                      rootDir: reg.rootDir.trim(),
                      dataType: reg.dataType,
                      timRes: reg.timRes,
                      years: [Number(reg.from), Number(reg.to)],
                      category: null,
                      fulllist: reg.fulllist.trim() || null,
                      variables: reg.variables
                        .split('\n')
                        .map((v) => v.trim())
                        .filter(Boolean),
                    }),
                  t('已登记', 'Registered'),
                )
              }
            />
          </div>
        </Card>
      )}

      {ready && (
        <Card
          title={t('新建比较', 'New comparison')}
          description={t(
            '选几个模拟（可以是不同项目、不同模型版本的算例，也可以加别的 history 目录），定变量和参考数据，生成 OpenBench 配置；配置可以先改再运行。单点算例会先把逐月的 history 并成一个站点文件。',
            'Pick simulations (cases from different projects or model versions, or any history folder), choose variables and references, and generate the OpenBench configuration; edit it before running if needed. Single-point cases are first merged from monthly history into one station file.',
          )}
        >
          <h3 className="study-subhead">{t('模拟', 'Simulations')}</h3>
          {remote && (
            <p className="muted study-hint">
              {t(
                `在服务器 ${host} 上挑算例目录或 history 目录：本机项目里的算例在服务器上读不到。`,
                `Choose case or history folders on server ${host}; cases in local projects cannot be read there.`,
              )}
            </p>
          )}
          <div className="study-cases">
            {(remote ? [] : caseDirs).map((c) => {
              const on = picks.some((p) => p.dir === c.dir);
              return (
                <label key={c.dir} className="param-check">
                  <input
                    type="checkbox"
                    checked={on}
                    disabled={busy}
                    onChange={(e) =>
                      e.target.checked
                        ? void addPick(c.dir, c.name)
                        : setPicks((old) => old.filter((p) => p.dir !== c.dir))
                    }
                  />
                  {c.name}
                </label>
              );
            })}
          </div>
          <div className="button-row">
            <ActionButton
              action="compare-add-folder"
              label={
                remote
                  ? t(
                      '加服务器上的算例或 history 目录…',
                      'Add a case or history folder on the server…',
                    )
                  : t(
                      '加一个别的项目的算例或 history 目录…',
                      'Add a case or history folder from another project…',
                    )
              }
              disabled={busy}
              run={async () => {
                if (remote) return setBrowsing('sim');
                const dir = await invoke<string | null>('pick_folder', { key: 'openbench-sim' });
                if (dir) await addPick(dir, pickLabel(dir));
              }}
            />
          </div>
          {picks.length > 0 && (
            <div className="table-scroll">
              <table className="versions-table">
                <thead>
                  <tr>
                    <th>{t('报告里的名字', 'Name in report')}</th>
                    <th>{t('种类', 'Kind')}</th>
                    <th>{t('年份', 'Years')}</th>
                    <th>{t('时间分辨率', 'Time step')}</th>
                    <th />
                  </tr>
                </thead>
                <tbody>
                  {picks.map((p) => (
                    <tr key={p.dir}>
                      <td>
                        <Input
                          value={p.label}
                          onChange={(e) =>
                            setPicks((old) =>
                              old.map((q) =>
                                q.dir === p.dir ? { ...q, label: e.target.value } : q,
                              ),
                            )
                          }
                        />
                      </td>
                      <td>
                        {p.error
                          ? p.error
                          : !p.info
                            ? t('读取中…', 'Reading…')
                            : p.info.spatial
                              ? t('格点', 'Gridded')
                              : t('单点（站点）', 'Single point (station)')}
                      </td>
                      <td>{p.info ? `${p.info.syear}–${p.info.eyear}` : '—'}</td>
                      <td>{p.info?.tim_res ?? '—'}</td>
                      <td>
                        <Button
                          onClick={() => setPicks((old) => old.filter((q) => q.dir !== p.dir))}
                        >
                          {t('去掉', 'Remove')}
                        </Button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
          <div className="form-grid">
            <Field label={t('比较名', 'Comparison name')}>
              <Input value={name} onChange={(e) => setName(e.target.value)} />
            </Field>
            <Field label={t('起始年', 'From year')}>
              <Input type="number" value={from} onChange={(e) => setFrom(e.target.value)} />
            </Field>
            <Field label={t('结束年', 'To year')}>
              <Input type="number" value={to} onChange={(e) => setTo(e.target.value)} />
            </Field>
            <Field label={t('比较的时间尺度', 'Time scale of the comparison')}>
              <Select value={timRes} onChange={(e) => setTimRes(e.target.value)}>
                <option value="Day">{t('逐日', 'Daily')}</option>
                <option value="Month">{t('逐月', 'Monthly')}</option>
                <option value="Hour">{t('逐时', 'Hourly')}</option>
              </Select>
            </Field>
          </div>
          <h3 className="study-subhead">{t('变量和参考数据', 'Variables and references')}</h3>
          {!ok.length ? (
            <p className="muted study-hint">
              {t(
                '先在上面选模拟，再一个个加要评估的变量，给每个变量挑参考数据。',
                'Pick simulations above first, then add the variables to evaluate one by one and choose a reference for each.',
              )}
            </p>
          ) : (
            <div className="form-grid">
              <Field label={t('筛选变量', 'Filter variables')}>
                <Input
                  value={variableQuery}
                  placeholder={t('如 Heat、Runoff', 'e.g. Heat, Runoff')}
                  onChange={(e) => setVariableQuery(e.target.value)}
                />
              </Field>
              <Field label={t('加一个要评估的变量', 'Add a variable to evaluate')}>
                <Select
                  value=""
                  onChange={(e) => {
                    const picked = allVariables.find((v) => v.variable === e.target.value);
                    // 默认选第一份参考数据：同种（站点或格点）的排在前面。
                    if (picked)
                      setChosen((old) => ({
                        ...old,
                        [picked.variable]: picked.references[0]?.name ?? '',
                      }));
                  }}
                >
                  <option value="">
                    {t(
                      `选择（${variables.length} 个可选）`,
                      `Choose (${variables.length} available)`,
                    )}
                  </option>
                  {variables
                    .filter((v) => !(v.variable in chosen))
                    .map((v) => (
                      <option key={v.variable} value={v.variable}>
                        {v.variable}
                      </option>
                    ))}
                </Select>
              </Field>
            </div>
          )}
          <div className="form-grid">
            {Object.keys(chosen)
              .sort()
              .map((variable) => {
                const options = allVariables.find((v) => v.variable === variable)?.references ?? [];
                return (
                  <Field key={variable} label={variable}>
                    <div className="study-path">
                      <Select
                        value={chosen[variable] ?? ''}
                        onChange={(e) =>
                          setChosen((old) => ({ ...old, [variable]: e.target.value }))
                        }
                      >
                        {options.map((r) => (
                          <option key={r.name} value={r.name}>
                            {r.name}
                            {r.data_type ? ` · ${r.data_type}` : ''}
                            {r.tim_res ? ` · ${r.tim_res}` : ''}
                          </option>
                        ))}
                      </Select>
                      <Button
                        onClick={() =>
                          setChosen((old) => {
                            const next = { ...old };
                            delete next[variable];
                            return next;
                          })
                        }
                      >
                        {t('去掉', 'Remove')}
                      </Button>
                    </div>
                  </Field>
                );
              })}
          </div>
          <div className="button-row">
            <ActionButton
              action="compare-generate"
              label={t('生成配置', 'Generate configuration')}
              disabled={busy || !ok.length}
              run={generate}
            />
          </div>
          {yaml && (
            <>
              <Field
                label={t(
                  'OpenBench 配置（可以先改再运行）',
                  'OpenBench configuration (edit before running if needed)',
                )}
              >
                <textarea
                  className="code-files-text"
                  rows={16}
                  spellCheck={false}
                  value={yaml}
                  onChange={(e) => setYaml(e.target.value)}
                />
              </Field>
              <div className="button-row">
                {!scanDir && (
                  <p className="muted study-hint">
                    {t(
                      '还没填"参考数据根目录"：OpenBench 自带的参考数据都按这个目录找，不填只能用自己登记了完整路径的数据。',
                      'No reference data root yet: OpenBench’s built-in references are looked up under it; without it only references registered with full paths work.',
                    )}
                  </p>
                )}
                <ActionButton
                  action="compare-run"
                  label={t('检查并运行', 'Check and run')}
                  kind="commit"
                  variant="primary"
                  disabled={busy}
                  run={run}
                />
              </div>
            </>
          )}
        </Card>
      )}

      <Card
        title={t('报告', 'Reports')}
        description={t(
          '这里跑过的比较和它们的报告；也能载入别处生成的 OpenBench 报告。',
          'Comparisons run here and their reports; reports produced elsewhere can be loaded too.',
        )}
      >
        {status?.projects.length ? (
          <div className="table-scroll">
            <table className="versions-table">
              <tbody>
                {status.projects.map((p) => (
                  <tr key={p.dir ?? p.name}>
                    <td>{p.name}</td>
                    <td>
                      {p.report ? t('有报告', 'has a report') : t('还没有报告', 'no report yet')}
                    </td>
                    <td>
                      {/* 服务器上的报告先取回本机（报告、指标与配置），再在软件里看。 */}
                      {p.report && remote && (
                        <Button onClick={() => void task(() => fetchReport(host, p.name))}>
                          {t('取回并查看', 'Fetch and view')}
                        </Button>
                      )}
                      {p.report && !remote && (
                        <>
                          <Button onClick={() => void task(() => view(p.report!))}>
                            {t('在软件里看', 'View here')}
                          </Button>
                          <Button onClick={() => void invoke('open_path', { path: p.report })}>
                            {t('用浏览器打开', 'Open in browser')}
                          </Button>
                        </>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <p className="muted">{t('还没有比较。', 'No comparisons yet.')}</p>
        )}
        <div className="button-row">
          <ActionButton
            action="compare-load-report"
            label={t('载入报告…', 'Load a report…')}
            run={() =>
              task(async () => {
                // 选文件和读文件都在后端：前端不能拿一个任意路径去读（复验 D2-05）。
                const picked = await invokeJson<{ path: string; html: string } | null>(
                  'bench_pick_report',
                );
                if (picked) setReport(picked);
              })
            }
          />
        </div>
        {report && (
          <>
            <p className="muted">{report.path}</p>
            {/* 报告放进完全的沙箱：不跑脚本、不给同源权限。OpenBench 3.0.8 的报告里没有一个 <script>
                （实测两份都是 0 个），用不着脚本；Windows 上 wry 会往子框架注入 IPC 初始化脚本，
                允许脚本就只剩 origin 校验一层兜底（复验 D2-04）。 */}
            <iframe
              className="compare-report"
              title={t('OpenBench 报告', 'OpenBench report')}
              sandbox=""
              srcDoc={report.html}
            />
          </>
        )}
      </Card>
      {remote && browsing && (
        <RemotePathBrowser
          host={host}
          directory
          en={en}
          initialPath={browsing === 'ref' ? scanDir || undefined : undefined}
          onSelect={(dir) => {
            const target = browsing;
            setBrowsing(null);
            if (target === 'ref') setScanDir(dir);
            else void addPick(dir, pickLabel(dir));
          }}
          onClose={() => setBrowsing(null)}
        />
      )}
    </div>
  );
}

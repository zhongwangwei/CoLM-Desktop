import { useEffect, useRef, useState } from 'react';
import { useApp } from '../../lib/app-state';
import { usePageContract } from '../../lib/guide';
import { invoke, invokeJson, listen } from '../../lib/ipc';
import { askConfirm } from '../../lib/confirm';
import { Button, Card, Field, Input, Notice, Select } from '../../components/ui';
import { ActionButton } from '../../components/ActionButton';
import {
  buildOpenbenchConfig,
  commonYears,
  comparisonDir,
  comparisonName,
  parseReferences,
  type BenchReference,
  type BenchSimulation,
  type HistoryInfo,
} from './compare';

type Project = { name: string; dir: string; config: string | null; report: string | null };
type Status = {
  root: string;
  uv: string | null;
  uv_own: boolean;
  uv_version: string;
  python: string | null;
  openbench: string | null;
  projects: Project[];
};
type Pick = { dir: string; label: string; info: HistoryInfo | null; error?: string };

/**
 * "比较"：用 OpenBench 对几个模拟（不同项目、不同模型版本）做评估，看报告。
 * 环境按需逐级装在应用数据目录里（uv → 专用 Python → OpenBench 与全部依赖），不碰系统。
 */
export default function ComparePage() {
  const { state } = useApp();
  const en = state.language === 'en';
  const t = (zh: string, english: string) => (en ? english : zh);
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
  const [yaml, setYaml] = useState('');
  const [sites, setSites] = useState<
    { id: string; station: string; history_dir: string; prefix: string }[]
  >([]);
  const [report, setReport] = useState<{ path: string; html: string } | null>(null);
  // 参考数据根目录：扫描用它，运行时作为 OPENBENCH_REF_ROOT 交给 OpenBench（内置登记表的路径都相对它）。
  // 按机器记住。
  const [scanDir, setScanDirState] = useState(() => {
    try {
      return globalThis.localStorage?.getItem('colm.openbenchRefRoot') ?? '';
    } catch {
      return '';
    }
  });
  const setScanDir = (value: string) => {
    setScanDirState(value);
    try {
      globalThis.localStorage?.setItem('colm.openbenchRefRoot', value);
    } catch {
      /* 存不下就只在这次会话里用 */
    }
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

  const refresh = async () => {
    const next = await invokeJson<Status>('bench_status');
    setStatus(next);
    // 读不到登记表要说出来：原来吞掉错误、显示"0 个"，看不出是哪里坏了（2026-10-23 窗口实测）。
    if (next.openbench)
      setReferences(
        parseReferences(
          await invokeJson('bench_references').catch((e) => {
            setError(String(e).replace(/^Error: /, ''));
            return [];
          }),
        ),
      );
    return next;
  };
  useEffect(() => {
    void refresh().catch((e) => setError(String(e)));
    let stop: (() => void) | undefined;
    void listen<{ stage?: string; line?: string }>('bench://log', (payload) =>
      setLog((old) => [...old.slice(-299), `[${payload.stage ?? ''}] ${payload.line ?? ''}`]),
    ).then((unlisten) => (stop = unlisten));
    return () => stop?.();
  }, []);

  // 一次只做一件事；报错原样显示（后端已是中文）。
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
      setError(String(e).replace(/^Error: /, ''));
    } finally {
      lock.current = false;
      setBusy(false);
      void refresh().catch(() => {});
    }
  };
  const install = (steps: ('uv' | 'python' | 'openbench')[]) =>
    task(
      async () => {
        for (const step of steps)
          await invoke('bench_install', {
            step,
            source: step === 'openbench' && source === 'local' ? localSource : null,
          });
      },
      t('装好了', 'Installed'),
    );
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
    ...(status?.uv ? [] : (['uv'] as const)),
    ...(status?.python ? [] : (['python'] as const)),
    ...(status?.openbench ? [] : (['openbench'] as const)),
  ];

  // 选模拟：勾一个算例（或加一个别的目录）就读它的 history 概况，填年份与种类。
  const addPick = async (dir: string, label: string) => {
    if (picks.some((p) => p.dir === dir)) return;
    setPicks((old) => [...old, { dir, label, info: null }]);
    try {
      const info = await invokeJson<HistoryInfo>('bench_inspect', { dir });
      setPicks((old) => old.map((p) => (p.dir === dir ? { ...p, info } : p)));
    } catch (e) {
      setPicks((old) => old.map((p) => (p.dir === dir ? { ...p, error: String(e) } : p)));
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
  const variables = [...new Set(references.flatMap((r) => r.variables))].sort();
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
      setError(String(e).replace(/^Error: /, ''));
    }
  };
  const run = () =>
    task(
      async () => {
        const result = await invokeJson<{ report: string | null }>('bench_run', {
          name: comparisonName(name),
          config: yaml,
          sites,
          refRoot: scanDir || null,
        });
        if (result.report) await view(result.report);
      },
      t('评估完成', 'Evaluation finished'),
    );
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
      {error && <Notice tone="error">{error}</Notice>}
      {notice && <Notice tone="success">{notice}</Notice>}
      <Card
        title={t('评估环境', 'Evaluation environment')}
        description={t(
          '用 OpenBench 评估。环境装在软件自己的数据目录里，不碰系统里的 Python：先装 uv（约 20 MB，校验后才用），再装专用 Python 3.12，最后装 OpenBench 和它的全部依赖（约 400 MB，需要联网）。',
          'Evaluation uses OpenBench. Everything installs into the app’s own data folder without touching the system Python: uv first (about 20 MB, verified before use), then a private Python 3.12, then OpenBench with all its dependencies (about 400 MB, needs the internet).',
        )}
      >
        <ul className="compare-status">
          <li>
            uv：
            {status?.uv
              ? `${status.uv_own ? t('软件自带', 'bundled') : t('系统已有', 'from the system')} · ${status.uv}`
              : t('未安装', 'not installed')}
          </li>
          <li>
            {t('专用 Python：', 'Private Python: ')}
            {status?.python ?? t('未安装', 'not installed')}
          </li>
          <li>OpenBench：{status?.openbench ?? t('未安装', 'not installed')}</li>
        </ul>
        <div className="form-grid">
          <Field label={t('OpenBench 从哪装', 'Install OpenBench from')}>
            <Select value={source} onChange={(e) => setSource(e.target.value as 'pypi' | 'local')}>
              <option value="pypi">
                {t('PyPI 发布版（colm-openbench）', 'PyPI release (colm-openbench)')}
              </option>
              <option value="local">
                {t('本地源码目录（可编辑安装）', 'Local source folder (editable install)')}
              </option>
            </Select>
          </Field>
          {source === 'local' && (
            <Field label={t('OpenBench 源码目录', 'OpenBench source folder')}>
              <div className="study-path">
                <Input value={localSource} onChange={(e) => setLocalSource(e.target.value)} />
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
              </div>
            </Field>
          )}
        </div>
        <div className="button-row">
          <ActionButton
            action="compare-install-all"
            label={
              steps.length
                ? t(`一键安装（${steps.length} 步）`, `Install everything (${steps.length} steps)`)
                : t('已全部装好', 'Everything is installed')
            }
            kind="commit"
            variant="primary"
            disabled={busy || !steps.length || (source === 'local' && !localSource)}
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
          {busy && <Button onClick={() => void invoke('bench_cancel')}>{t('停止', 'Stop')}</Button>}
          {(status?.python || ready) && (
            <Button disabled={busy} onClick={() => void uninstall('env')}>
              {t('删掉 OpenBench', 'Remove OpenBench')}
            </Button>
          )}
          {status?.uv_own && (
            <Button variant="danger" disabled={busy} onClick={() => void uninstall('all')}>
              {t('全部删掉', 'Remove everything')}
            </Button>
          )}
        </div>
        {log.length > 0 && <pre className="compare-log">{log.slice(-80).join('\n')}</pre>}
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
                  setScanText(await invoke<string>('bench_scan_preview', { dir: scanDir })),
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
                '变量（每行 变量:文件里的名字:单位，如 Latent_Heat:LE:W m-2）',
                'Variables (one per line, variable:name in file:unit, e.g. Latent_Heat:LE:W m-2)',
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
                    invoke('bench_register', {
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
          <div className="study-cases">
            {caseDirs.map((c) => {
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
              label={t(
                '加一个别的项目的算例或 history 目录…',
                'Add a case or history folder from another project…',
              )}
              disabled={busy}
              run={async () => {
                const dir = await invoke<string | null>('pick_folder', { key: 'openbench-sim' });
                if (dir) await addPick(dir, dir.split(/[/\\]/).filter(Boolean).pop() ?? 'sim');
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
          <div className="form-grid">
            {variables.map((variable) => {
              const options = references.filter((r) => r.variables.includes(variable));
              return (
                <Field key={variable} label={variable}>
                  <Select
                    value={chosen[variable] ?? ''}
                    onChange={(e) => setChosen((old) => ({ ...old, [variable]: e.target.value }))}
                  >
                    <option value="">{t('不评估', 'Skip')}</option>
                    {options.map((r) => (
                      <option key={r.name} value={r.name}>
                        {r.name}
                        {r.data_type ? ` · ${r.data_type}` : ''}
                        {r.tim_res ? ` · ${r.tim_res}` : ''}
                      </option>
                    ))}
                  </Select>
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
                  <tr key={p.dir}>
                    <td>{p.name}</td>
                    <td>
                      {p.report ? t('有报告', 'has a report') : t('还没有报告', 'no report yet')}
                    </td>
                    <td>
                      {p.report && (
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
                const path = await invoke<string | null>('pick_file', {
                  key: 'openbench-report',
                  filter: 'html',
                });
                if (path) await view(path);
              })
            }
          />
        </div>
        {report && (
          <>
            <p className="muted">{report.path}</p>
            {/* 报告是 OpenBench 生成的独立 HTML：放进沙箱，允许它自己的脚本，不给同源权限。 */}
            <iframe
              className="compare-report"
              title={t('OpenBench 报告', 'OpenBench report')}
              sandbox="allow-scripts"
              srcDoc={report.html}
            />
          </>
        )}
      </Card>
    </div>
  );
}

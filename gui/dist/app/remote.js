//! 服务器运行（R1，docs/design-ai-assistant.md 第 5.3 节）：服务器设置对话框、运行页的“运行位置”、
//! 远程运行的状态与取回结果。实际工作都由 `colm-cli remote-*` 经用户自己的 ssh 完成。
//!
//! 只依赖 ipc/state/ui/i18n/sites/results/shell；runner.js 导入本模块，反过来导入会成环。

import { invoke, hasBackend } from './ipc.js';
import { state } from './state.js';
import { $, status, baseName } from './ui.js';
import { language, translateZh } from './i18n.js';
import { renderCases } from './sites.js';
import { invalidateResultCase } from './results.js';
import { setRunning } from './shell.js';
import { modelEngine } from './engine.js';

const t = text => (language() === 'en' ? translateZh(text) : text);
const POLL_MS = 10_000;

let config = { servers: [] };
/** 远程作业：算例目录 → { host, job, state, exit_code, phase, stages, progress, log, fetched, error } */
const jobs = new Map();
let pollTimer = null;

// ---- 纯函数（tests/remote.mjs）--------------------------------------------------------------

/** “本机前缀 = 服务器前缀”一行一条 → [{local, remote}]；空行忽略，格式不对的行报出来。 */
export function parseMaps(text) {
  const maps = [];
  for (const [index, raw] of String(text ?? '').split('\n').entries()) {
    const line = raw.trim();
    if (!line) continue;
    const at = line.indexOf('=');
    if (at < 0) throw new Error(`${t('路径对应第')} ${index + 1} ${t('行缺少“=”')}`);
    const local = line.slice(0, at).trim();
    const remote = line.slice(at + 1).trim();
    if (!local || !remote) throw new Error(`${t('路径对应第')} ${index + 1} ${t('行两边都要填')}`);
    maps.push({ local, remote });
  }
  return maps;
}

export function formatMaps(maps) {
  return (maps ?? []).map(m => `${m.local} = ${m.remote}`).join('\n');
}

/** 探测结果的几行摘要。 */
export function probeLines(probe) {
  const lines = [
    `${probe.hostname} · ${probe.os} · ${probe.arch}`,
    `${probe.cpus ?? '?'} ${t('核')} · ${probe.memory_gb ?? '?'} GB ${t('内存')} · ${t('工作目录剩余')} ${probe.root_free_gb ?? '?'} GB`,
    `cargo: ${probe.cargo || t('没有')} · gfortran: ${probe.gfortran ? t('有') : t('没有')} · MPI: ${probe.mpi ? t('有') : t('没有')}`,
    `${t('调度系统')}: ${probe.schedulers?.length ? probe.schedulers.join(', ') : t('无（直接在后台运行）')}`,
  ];
  if (probe.engine) {
    lines.push(`${t('引擎')}: ${probe.engine === 'prebuilt' ? t('传预编的程序（这台服务器不用编译）') : t('在服务器上从源码编译')}`);
  }
  if (probe.queues?.length) lines.push(`${t('分区或队列')}: ${probe.queues.join(', ')}`);
  return lines;
}

/** 远程作业一行状态文字。 */
export function jobSummary(job) {
  if (job.error) return `${t('出错')}：${job.error}`;
  if (job.state === 'queued') return job.detail ? `${t('在调度系统里排队')}（${job.detail}）` : t('在调度系统里排队');
  if (job.state === 'running') {
    const p = job.progress;
    if (job.phase === 'building the Rust engine') return t('正在服务器上编译 Rust 引擎（首次约 4 分钟）');
    if (p?.total_steps) return `${t('运行中')} ${Math.min(100, Math.round((p.step / p.total_steps) * 100))}% · ${p.date}`;
    const stage = job.stages?.at(-1);
    return stage ? `${t('运行中')} · ${stage[0]}` : t('运行中');
  }
  if (job.state === 'finished') {
    if (job.exit_code !== 0) return `${t('失败')}（${t('退出码')} ${job.exit_code}）`;
    if (job.autoFetch === false && !job.fetched) return t('完成，等待手动取回结果');
    if (job.fetched && job.partial) return t('完成，已取回所选变量（可以再取回全部）');
    return job.fetched ? t('完成，结果已取回') : t('完成，正在取回结果…');
  }
  if (job.state === 'lost') {
    return job.detail
      ? `${t('作业没有留下退出码，调度系统说')}：${job.detail}`
      : t('服务器上的进程不在了（可能被终止或机器重启）');
  }
  if (job.state === 'submitting') return t('正在上传并提交…');
  return t('状态未知');
}

// ---- 配置 ----------------------------------------------------------------------------------

async function loadConfig() {
  config = await invoke('remote_config').catch(() => ({ servers: [] }));
  config.servers ??= [];
  renderTargets();
  return config;
}

/** 运行位置：'local' 或服务器名。 */
export function runTarget() {
  return $('run-target')?.value || 'local';
}

function renderTargets() {
  const select = $('run-target');
  if (!select) return;
  const keep = state.runTarget ?? select.value ?? 'local';
  select.replaceChildren(new Option(t('本机'), 'local'));
  for (const server of config.servers) select.appendChild(new Option(`${t('服务器')}：${server.host}`, server.host));
  select.value = [...select.options].some(o => o.value === keep) ? keep : 'local';
  syncTarget();
}

/** 选了服务器：Rust 引擎（服务器上编或传预编程序）与 Fortran 内核（服务器上要有编好的内核，可多进程 MPI）都能选。 */
function syncTarget() {
  const remote = runTarget() !== 'local';
  state.runTarget = runTarget();
  $('manage-servers').textContent = t(remote ? '修改这台服务器…' : '管理服务器…');
  const preview = $('preview-job');
  if (preview) preview.hidden = !remote;
}

/** 运行页选的引擎与 MPI 进程数；只有 Fortran 内核用进程数。 */
export function engineChoice(engine = modelEngine(), ranksText = $('mpi-ranks')?.value) {
  if (engine !== 'fortran') return { engine: 'rust', ranks: null };
  const ranks = Math.max(1, Math.trunc(Number(ranksText)) || 1);
  return { engine: 'fortran', ranks };
}

/** 把将要提交的作业脚本全文给用户看（不上传、不提交）。 */
async function previewJob() {
  const dir = (state.batch?.length ? state.batch : state.selected ? [state.selected.dir] : [])[0];
  if (!dir) {
    status(t('先选一个算例'));
    return;
  }
  const text = $('remote-preview-text');
  $('remote-preview-note').textContent = t('正在生成…');
  text.textContent = '';
  $('remote-preview-dialog').showModal();
  try {
    const answer = await invoke('remote_preview', { case: dir, host: runTarget(), kernel: $('kernel').value, stage: null, force: false, ...engineChoice() });
    $('remote-preview-note').textContent = `${baseName(dir)} @ ${runTarget()} · ${t('调度系统')}: ${answer.scheduler}`;
    text.textContent = answer.job_script;
  } catch (error) {
    $('remote-preview-note').textContent = String(error?.message || error);
  }
}

// ---- 服务器对话框 ----------------------------------------------------------------------------

let dialogDone = null;

function fillDialog(host) {
  const server = config.servers.find(s => s.host === host);
  $('remote-host').value = server?.host ?? '';
  $('remote-root').value = server?.root ?? '';
  $('remote-maps').value = formatMaps(server?.maps);
  $('remote-threads').value = server?.threads ?? 8;
  $('remote-scheduler').value = server?.scheduler ?? 'auto';
  $('remote-partition').value = server?.partition ?? '';
  $('remote-account').value = server?.account ?? '';
  $('remote-walltime').value = server?.walltime ?? '';
  $('remote-cpus').value = server?.cpus ?? 0;
  $('remote-memory').value = server?.memory_gb ?? 0;
  $('remote-nodes').value = server?.nodes ?? 0;
  $('remote-fetch-vars').value = server?.fetch_vars ?? '';
  $('remote-env').value = server?.env_script ?? '';
  $('remote-directives').value = (server?.directives ?? []).join('\n');
  $('remote-delete').hidden = !server;
  $('remote-probe-result').replaceChildren();
}

function renderWhich(selected) {
  const which = $('remote-which');
  which.replaceChildren(...config.servers.map(s => new Option(s.host, s.host)), new Option(t('＋ 新服务器'), ''));
  which.value = selected ?? '';
}

function formServer() {
  const host = $('remote-host').value.trim();
  const root = $('remote-root').value.trim();
  if (!host) throw new Error(t('请填写主机（ssh 配置里的别名）'));
  if (!root.startsWith('/')) throw new Error(t('工作目录要是服务器上的绝对路径'));
  const threads = Number($('remote-threads').value);
  const count = id => {
    const n = Number($(id).value);
    return Number.isFinite(n) && n > 0 ? Math.round(n) : 0;
  };
  return {
    host,
    root,
    maps: parseMaps($('remote-maps').value),
    threads: Number.isFinite(threads) && threads > 0 ? Math.round(threads) : 8,
    scheduler: $('remote-scheduler').value || 'auto',
    partition: $('remote-partition').value.trim(),
    account: $('remote-account').value.trim(),
    walltime: $('remote-walltime').value.trim(),
    cpus: count('remote-cpus'),
    memory_gb: count('remote-memory'),
    nodes: count('remote-nodes'),
    fetch_vars: $('remote-fetch-vars').value.trim(),
    env_script: $('remote-env').value.trim(),
    directives: parseDirectives($('remote-directives').value),
  };
}

/** “其他指令”一行一条；空行忽略。 */
export function parseDirectives(text) {
  return String(text ?? '').split('\n').map(l => l.trim()).filter(Boolean);
}

/** 打开服务器设置；保存后返回服务器名，取消返回 null。 */
export async function openServerDialog(host = null) {
  await loadConfig();
  const aliases = await invoke('remote_ssh_hosts').catch(() => []);
  $('remote-host-list').replaceChildren(...aliases.map(a => new Option(a, a)));
  const start = host ?? config.servers[0]?.host ?? null;
  renderWhich(start);
  fillDialog(start);
  $('remote-dialog').showModal();
  return new Promise(resolve => { dialogDone = resolve; });
}

function closeDialog(result) {
  $('remote-dialog').close();
  const done = dialogDone;
  dialogDone = null;
  done?.(result);
}

async function testConnection() {
  const box = $('remote-probe-result');
  box.className = 'remote-probe mini muted';
  box.textContent = t('正在连接…');
  try {
    const server = formServer();
    const probe = await invoke('remote_probe', { host: server.host, root: server.root });
    $('remote-queue-list').replaceChildren(...(probe.queues ?? []).map(q => new Option(q, q)));
    box.replaceChildren(...probeLines(probe).map(line => Object.assign(document.createElement('div'), { textContent: line })));
    for (const problem of probe.problems ?? []) {
      box.appendChild(Object.assign(document.createElement('div'), { className: 'assistant-fail', textContent: `⚠ ${problem}` }));
    }
    box.className = `remote-probe mini ${probe.problems?.length ? '' : 'assistant-key-ok'}`;
    box.prepend(Object.assign(document.createElement('div'), { textContent: probe.problems?.length ? t('连上了，但还缺：') : `✓ ${t('连接正常，可以运行')}` }));
  } catch (error) {
    box.className = 'remote-probe mini assistant-fail';
    box.textContent = String(error?.message || error);
  }
}

async function saveServer() {
  try {
    const server = formServer();
    const servers = config.servers.filter(s => s.host !== server.host && s.host !== $('remote-which').value);
    servers.push(server);
    await invoke('remote_save_config', { config: { servers } });
    state.runTarget = server.host;
    await loadConfig();
    status(`${t('已保存服务器')} ${server.host}`);
    closeDialog(server.host);
  } catch (error) {
    const box = $('remote-probe-result');
    box.className = 'remote-probe mini assistant-fail';
    box.textContent = String(error?.message || error);
  }
}

async function deleteServer() {
  const host = $('remote-which').value;
  if (!host) return;
  await invoke('remote_save_config', { config: { servers: config.servers.filter(s => s.host !== host) } });
  if (state.runTarget === host) state.runTarget = 'local';
  await loadConfig();
  renderWhich(config.servers[0]?.host ?? null);
  fillDialog(config.servers[0]?.host ?? null);
}

// ---- 远程运行 ----------------------------------------------------------------------------------

function renderJobs() {
  const box = $('remote-runs');
  const list = $('remote-run-list');
  if (!box || !list) return;
  box.hidden = jobs.size === 0;
  list.replaceChildren();
  for (const [dir, job] of jobs) {
    const row = document.createElement('div');
    row.className = `remote-run ${job.state}${job.error || (job.state === 'finished' && job.exit_code !== 0) ? ' failed' : ''}`;
    const head = document.createElement('div');
    head.className = 'remote-run-head';
    head.append(
      Object.assign(document.createElement('b'), { textContent: baseName(dir) }),
      Object.assign(document.createElement('span'), { className: 'muted mini', textContent: `@ ${job.host}` }),
      Object.assign(document.createElement('span'), { className: 'mini remote-run-state', textContent: jobSummary(job) }),
    );
    if (job.state === 'running' || job.state === 'queued' || job.state === 'submitting') {
      const cancel = Object.assign(document.createElement('button'), { className: 'btn-ghost btn-stop', type: 'button', textContent: t('终止') });
      cancel.onclick = () => cancelJob(dir);
      head.appendChild(cancel);
    }
    if (job.state === 'finished' && job.exit_code === 0 && (job.partial || !job.fetched)) {
      const all = Object.assign(document.createElement('button'), { className: 'btn-ghost', type: 'button', textContent: t('取回全部变量') });
      all.onclick = () => { all.disabled = true; fetchJob(dir, true); };
      head.appendChild(all);
    }
    row.appendChild(head);
    const failed = job.state === 'finished' && job.exit_code !== 0;
    if (job.log && (failed || job.state === 'lost')) {
      const details = document.createElement('details');
      details.open = failed;
      details.append(Object.assign(document.createElement('summary'), { className: 'mini', textContent: t('服务器上的日志末尾') }),
        Object.assign(document.createElement('pre'), { className: 'assistant-code', textContent: job.log.split('\n').slice(-30).join('\n') }));
      row.appendChild(details);
    }
    list.appendChild(row);
  }
}

async function refreshJob(dir) {
  const job = jobs.get(dir);
  try {
    const answer = await invoke('remote_status', { case: dir });
    if (job.job && (answer.record?.job !== job.job || answer.record?.host !== job.host)) {
      Object.assign(job, { autoFetch: false, fetched: false, partial: false });
    }
    Object.assign(job, {
      host: answer.record?.host ?? job.host,
      job: answer.record?.job ?? job.job,
      state: answer.status?.state ?? 'unknown',
      exit_code: answer.status?.exit_code,
      phase: answer.status?.phase,
      detail: answer.status?.detail,
      log: answer.status?.log_tail ?? '',
      stages: answer.stages ?? [],
      progress: answer.progress,
      error: null,
    });
    if (job.state === 'finished' && job.exit_code === 0 && !job.fetched && job.autoFetch !== false) await fetchJob(dir);
  } catch (error) {
    job.error = String(error?.message || error);
  }
}

async function fetchJob(dir, all = false) {
  const job = jobs.get(dir);
  try {
    const answer = await invoke('remote_fetch', { case: dir, host: job.host, all });
    job.fetched = true;
    job.partial = answer?.partial === true;
    const c = state.cases.find(c => c.dir === dir);
    if (c) c.has_history = true;
    invalidateResultCase(dir);
    renderCases();
    status(`${baseName(dir)}：${t(job.partial ? '服务器上的运行完成，已取回所选变量' : '服务器上的运行完成，结果已取回')}`);
  } catch (error) {
    job.error = `${t('取回结果失败')}：${String(error?.message || error)}`;
  }
  renderJobs();
}

async function poll() {
  pollTimer = null;
  const active = [...jobs].filter(([, j]) => j.state === 'running' || j.state === 'queued' || (j.state === 'finished' && j.exit_code === 0 && !j.fetched && j.autoFetch !== false));
  await Promise.all(active.map(([dir]) => refreshJob(dir)));
  renderJobs();
  const running = [...jobs.values()].filter(j => j.state === 'running' || j.state === 'queued');
  if (running.length) {
    setRunning('busy', `${t('服务器运行中')}（${running.length}）`);
    pollTimer = setTimeout(poll, POLL_MS);
  } else if (active.length) {
    const failed = [...jobs.values()].some(j => j.error || (j.state === 'finished' && j.exit_code !== 0));
    setRunning(failed ? 'fail' : 'ok', t(failed ? '服务器运行有失败' : '服务器运行完成'));
  }
}

function schedulePoll(delay = 3000) {
  if (pollTimer) clearTimeout(pollTimer);
  pollTimer = setTimeout(poll, delay);
}

async function cancelJob(dir) {
  try {
    await invoke('remote_cancel', { case: dir });
    status(`${baseName(dir)}：${t('已请求终止服务器上的运行')}`);
  } catch (error) {
    status(error);
  }
  schedulePoll(1000);
}

/** 运行页的运行按钮在“运行位置”是服务器时走这里：逐个算例提交，之后定时查状态，跑完自动取回结果。 */
export async function remoteRun(stage, dirs, force) {
  const host = runTarget();
  for (const dir of dirs) {
    jobs.set(dir, { host, state: 'submitting', fetched: false });
    renderJobs();
    try {
      const answer = await invoke('remote_run', { case: dir, host, kernel: $('kernel').value, stage, force, ...engineChoice() });
      Object.assign(jobs.get(dir), { job: answer.job, state: 'running', phase: 'submitted' });
      if (answer.engine_uploaded) status(`${t('已把引擎源码传到')} ${host}${t('，首次会在服务器上编译')}`);
    } catch (error) {
      Object.assign(jobs.get(dir), { state: 'unknown', error: String(error?.message || error) });
    }
    renderJobs();
  }
  setRunning('busy', `${t('服务器运行中')}（${dirs.length}）`);
  schedulePoll();
}

/** 打开运行页时接上已提交的远程作业（关掉应用后再打开也能接着跟踪）。 */
async function resumeJobs() {
  // 不管现在选的运行位置是什么都要接：算例可能是上次在服务器上跑的。
  const dirs = (state.batch?.length ? state.batch : state.selected ? [state.selected.dir] : []).filter(d => !jobs.has(d));
  for (const dir of dirs) {
    try {
      const answer = await invoke('remote_status', { case: dir });
      jobs.set(dir, { host: answer.record.host, job: answer.record.job, fetched: false, autoFetch: !state.cases.find(c => c.dir === dir)?.has_history });
      await refreshJob(dir);
    } catch {
      // 没在服务器上跑过（没有提交记录），不显示。
    }
  }
  renderJobs();
  if ([...jobs.values()].some(j => j.state === 'running' || j.state === 'queued')) schedulePoll();
}

/** 服务器上内核的几行摘要。 */
export function kernelLines(kernels) {
  if (!kernels?.length) return [t('服务器上还没有内核')];
  return kernels.map(k => `${k.name} · ${k.preset || '?'} · ${k.full ? t('完整') : t('只有清单（Rust 引擎用）')}`);
}

async function listKernels() {
  const box = $('remote-kernels-result');
  box.className = 'remote-probe mini muted';
  box.textContent = t('正在查看…');
  try {
    const server = formServer();
    // 内核列表按已保存的服务器配置查；没保存过就先提示保存。
    if (!config.servers.some(s => s.host === server.host)) throw new Error(t('先保存这台服务器'));
    const kernels = await invoke('remote_kernels', { host: server.host });
    box.className = 'remote-probe mini';
    box.replaceChildren(...kernelLines(kernels).map(line => Object.assign(document.createElement('div'), { textContent: line })));
  } catch (error) {
    box.className = 'remote-probe mini assistant-fail';
    box.textContent = String(error?.message || error);
  }
}

async function buildKernel() {
  const box = $('remote-kernels-result');
  const button = $('remote-build-kernel');
  const preset = $('remote-kernel-preset').value;
  try {
    const server = formServer();
    if (!config.servers.some(s => s.host === server.host)) throw new Error(t('先保存这台服务器'));
    button.disabled = true;
    box.className = 'remote-probe mini muted';
    box.textContent = `${t('正在服务器上编译')} ${preset}（${t('十几分钟，请不要关闭本窗口')}）…`;
    const answer = await invoke('remote_build_kernel', { host: server.host, preset });
    box.className = 'remote-probe mini assistant-key-ok';
    box.textContent = `✓ ${answer.built ? t('已编好') : t('已经有了，没有重编')}：${answer.name}`;
  } catch (error) {
    box.className = 'remote-probe mini assistant-fail';
    box.textContent = String(error?.message || error);
  } finally {
    button.disabled = false;
  }
}

function wire() {
  if (!hasBackend || !$('run-target')) return;
  $('serverRunCard').onclick = async () => {
    const host = await openServerDialog();
    if (!host) return;
    $('launchgate').hidden = true;
    const { showDomainGate } = await import('./domain.js');
    showDomainGate();
  };
  $('run-target').addEventListener('change', syncTarget);
  $('preview-job').onclick = () => previewJob();
  $('remote-preview-close').onclick = () => $('remote-preview-dialog').close();
  $('manage-servers').onclick = () => openServerDialog(runTarget() === 'local' ? null : runTarget());
  $('remote-which').addEventListener('change', () => fillDialog($('remote-which').value || null));
  $('remote-test').onclick = () => testConnection();
  $('remote-list-kernels').onclick = () => listKernels();
  $('remote-build-kernel').onclick = () => buildKernel();
  $('remote-save').onclick = () => saveServer();
  $('remote-delete').onclick = () => deleteServer().catch(e => status(e));
  $('remote-close').onclick = () => closeDialog(null);
  $('remote-dialog').addEventListener('cancel', () => closeDialog(null));
  addEventListener('colm:step', () => { if (state.step === 'run') resumeJobs().catch(() => {}); });
  addEventListener('colm:language', renderTargets);
  loadConfig().catch(() => {});
}

wire();

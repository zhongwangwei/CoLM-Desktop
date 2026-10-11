//! 计算资源页：在哪里跑——本机，或一台经你自己的 ssh 连接的服务器。运行方式里的“运行位置”就从这些里选。
//!
//! 一台服务器的详细设置（地址、目录对应、调度、环境准备）仍在 remote.js 的服务器对话框里编辑；这里列出所有资源，
//! 给出检查连接、查看与编译内核（官方版或当前选中的修改版）、设为运行位置这几样操作。

import { invoke, hasBackend } from './ipc.js';
import { state } from './state.js';
import { $, status, appConfirm } from './ui.js';
import { language, translateZh } from './i18n.js';
import { go } from './shell.js';
import { openServerDialog, probeLines, kernelLines, runTarget, sessionJobs } from './remote.js';
import { OFFICIAL, modelVersion, versionLabel } from './run-mode.js';
import { cachedProbe, recordProbe } from './remote-probe-cache.js';

import { kernelForSubgrid } from './kernel.js';

const t = text => (language() === 'en' ? translateZh(text) : text);
let refreshRequest = 0;
let resourceRecords = [];
let localVersions = [];
let localVersionError = '';


// ---- 纯函数（tests/resources.mjs）--------------------------------------------------------------

/** 一台服务器卡片上的摘要：工作目录、调度、资源。 */
export function serverFacts(server) {
  const scheduler = !server.scheduler || server.scheduler === 'auto' ? '自动识别调度' : server.scheduler === 'bare' ? '不用调度系统' : server.scheduler;
  return [
    `${t('工作目录')} ${server.root || '?'}`,
    t(scheduler),
    server.threads ? `${server.threads} ${t('线程')}` : null,
    server.maps?.length ? `${server.maps.length} ${t('条目录对应')}` : t('还没有目录对应'),
  ].filter(Boolean).join(' · ');
}

/** 本机卡片：CPU 与随应用的引擎、内核。 */
export function localFacts(cpus, kernels) {
  const presets = (kernels ?? []).map(k => k.preset).filter(Boolean);
  return [
    `${cpus} ${t('个逻辑 CPU')}`,
    t('Rust 引擎随应用安装'),
    presets.length ? `${t('Fortran 内核')}：${presets.join(', ')}` : t('没有找到 Fortran 内核'),
  ].join(' · ');
}

/** 编内核时用哪一版的源码：当前选的修改版，或官方版。 */
export function kernelBuildRequest(host, preset, version) {
  return { host, preset, modelVersion: !version || version === OFFICIAL ? null : version };
}

// ---- 页面 ------------------------------------------------------------------------------------

function el(tag, props = {}, children = []) {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}

function button(text, onclick, className = 'btn-ghost') {
  return el('button', { type: 'button', className, textContent: t(text), onclick });
}

/** 设为运行位置并回到运行方式。 */
function useTarget(host) {
  const select = $('run-target');
  if (select) {
    select.value = host;
    select.dispatchEvent(new Event('change'));
  }
  go('run-mode');
}

export function probeStatus(cached) {
  if (!cached) return t('尚未测试；当前连接状态未知');
  const missing = (cached.probe?.maps ?? []).filter(map => map.local_exists === false || map.remote_exists === false).length;
  const state = cached.error ? t('连接失败（上次测试）')
    : missing ? `${t('上次已连接；缺失目录对应')}：${missing}`
    : cached.probe?.problems?.length ? t('上次已连接，但准备条件未齐全') : t('在线（上次测试）');
  return `${state} · ${t('上次测试')} ${new Date(cached.at).toLocaleString(language() === 'en' ? 'en' : 'zh-CN')}`;
}

function loginFacts(server, probe) {
  return [
    t(server.auth === 'password' ? '密码登录' : server.auth === 'key' ? '密钥登录' : 'SSH 配置登录'),
    server.username, probe?.os, probe?.cpus ? `${probe.cpus} ${t('个逻辑 CPU')}` : '',
    probe ? (probe.schedulers?.join(' / ') || t('无调度系统')) : '',
    server.partition, server.walltime,
  ].filter(Boolean).join(' · ');
}

function recordList(host) {
  const rows = resourceRecords.filter(record => record.host === host);
  return el('section', { className: 'resource-section' }, [
    el('h3', { textContent: t('使用记录（本地已知）') }),
    el('p', { className: 'mini muted', textContent: t('仅显示当前项目和会话的已知记录；未实时查询服务器，不能据此判断空闲。') }),
    ...rows.map(record => el('p', { className: 'mini', textContent: `${record.label} · ${t('上次状态')}：${record.state || t('未知')}` })),
    ...(!rows.length ? [el('p', { className: 'mini muted', textContent: t('没有可显示的记录；当前使用情况未知。') })] : []),
  ]);
}

function renderLocal() {
  const box = $('resource-local');
  if (!box) return;
  const cpus = Math.max(1, Number(navigator.hardwareConcurrency) || 1);
  box.replaceChildren(
    el('div', { className: 'ws-head' }, [
      el('b', { textContent: t('本机') }),
      el('span', { className: 'muted mini', textContent: localFacts(cpus, state.kernels) }),
      runTarget() === 'local' ? el('span', { className: 'ws-in-use', textContent: t('当前运行位置') }) : '',
    ]),
    el('p', { className: 'mini', textContent: `${t('已就绪的版本')}：${[t('官方版'), ...localVersions].join(' · ')}` }),
    ...(localVersionError ? [el('p', { className: 'mini warn', textContent: localVersionError })] : []),
    el('p', { className: 'mini muted', textContent: t('修改版就绪情况按当前算例的内核配置检查。') }),
    el('div', { className: 'pill-row' }, [button('设为运行位置', () => useTarget('local'))]),
  );
}

/** Read local records only. Remote status/probe commands belong to explicit user actions. */
async function readResourceRecords(workspaces) {
  const records = sessionJobs().map(job => ({ host: job.host, label: `${t('算例')} ${job.dir}`, state: job.error ? t('未知') : job.state }));
  const cases = [...new Set([...(state.cases ?? []), state.selected].filter(Boolean).map(item => item.dir).filter(Boolean))];
  const studies = new Set();
  for (const dir of cases) {
    try {
      const record = await invoke('remote_job_record', { case: dir });
      if (record && !sessionJobs().some(job => job.dir === dir && job.job === record.job && job.host === record.host)) {
        records.push({ host: record.host, label: `${t('算例')} ${dir} · ${record.job}`, state: t('已提交；当前状态未知') });
      }
    } catch (error) {
      records.push({ host: '*', label: `${t('记录读取失败')}：${dir}`, state: String(error?.message || error) });
    }
    try {
      const history = await invoke('case_runs', { case: dir });
      for (const study of history.studies ?? []) if (study.dir) studies.add(study.dir);
    } catch (error) {
      records.push({ host: '*', label: `${t('记录读取失败')}：${dir}`, state: String(error?.message || error) });
    }
  }
  for (const dir of studies) {
    try {
      const raw = await invoke('study_status', { studyDir: dir });
      const study = typeof raw === 'string' ? JSON.parse(raw) : raw;
      if (study.remote?.host) records.push({ host: study.remote.host, label: `${t('研究')} ${dir}`, state: study.remote.fetched_at ? t('结果已取回') : `${study.state?.status || t('未知')}（${t('本地记录，可能已过期')}）` });
    } catch (error) { records.push({ host: '*', label: `${t('记录读取失败')}：${dir}`, state: String(error?.message || error) }); }
  }
  for (const ws of workspaces) {
    try {
      const result = await invoke('workspace_remote', { name: ws.name, operation: 'list', host: null, job: null, request: null });
      for (const job of result.jobs ?? []) records.push({ host: job.host, label: `${t('版本检查')} ${ws.name} · ${job.id}`, state: `${job.state || t('未知')}（${t('本地记录，可能已过期')}）` });
    } catch (error) { records.push({ host: '*', label: `${t('记录读取失败')}：${ws.name}`, state: String(error?.message || error) }); }
  }
  return records;
}

function serverCard(server) {
  const out = el('div', { className: 'remote-probe mini', role: 'status' });
  const preset = el('select', { className: 'select' });
  for (const k of state.kernels ?? []) preset.append(new Option(k.preset, k.preset));
  if (!preset.options.length) preset.append(new Option('default', 'default'));
  const show = (lines, kind = '') => {
    out.className = `remote-probe mini ${kind}`.trim();
    out.replaceChildren(...lines.map(line => el('div', { textContent: line })));
  };
  const fail = error => show([String(error?.message || error)], 'assistant-fail');
  const summary = el('p', { className: 'mini resource-status', role: 'status' });
  const login = el('p', { className: 'mini muted' });
  const maps = el('section', { className: 'resource-section' });
  const engines = el('section', { className: 'resource-section' });
  let busy = false;
  const edit = () => openServerDialog(server.host).then(() => refresh()).catch(status);
  function renderProbe() {
    const cached = cachedProbe(server);
    summary.textContent = probeStatus(cached);
    login.textContent = loginFacts(server, cached?.probe);
    maps.replaceChildren(el('h3', { textContent: t('数据目录对应') }));
    for (const map of server.maps ?? []) {
      const found = cached?.probe?.maps?.find(item => item.local === map.local && item.remote === map.remote);
      const mark = value => value === true ? '✓' : value === false ? '✕' : '?';
      maps.append(el('div', { className: 'resource-map mini' }, [
        el('span', { textContent: `${mark(found?.local_exists)} ${t('本机')}：${map.local}` }),
        el('span', { textContent: `${mark(found?.remote_exists)} ${t('服务器')}：${map.remote}` }),
        ...((found?.local_exists === false || found?.remote_exists === false) ? [button('修改目录对应…', edit)] : []),
      ]));
    }
    maps.append(el('p', { className: 'mini muted', textContent: t(server.maps?.length ? '路径状态来自上次连接测试；问号表示尚未检查。' : '还没有目录对应') }));
  }
  function renderEngines(inventory = null, error = null) {
    engines.replaceChildren(el('h3', { textContent: t('已就绪的版本') }));
    if (!inventory) {
      engines.append(el('p', { className: 'mini muted', textContent: error ? `${t('引擎清单读取失败')}：${error}` : t('检查连接后显示服务器引擎清单。') }));
      return;
    }
    for (const reason of inventory.cleanup_blockers ?? []) engines.append(el('p', { className: 'mini warn', textContent: `${t('清理保护')}：${reason}` }));
    // 中断的提交（或清理）留下的记录：只挡清理，不挡提交。超过一小时的可以由人确认后清掉。
    const stale = (inventory.leases ?? []).filter(lease => lease.age_s >= 3600);
    if (stale.length || (inventory.cleanup_blockers ?? []).some(reason => /cleanup in progress or interrupted/.test(reason))) {
      const recover = button('解除中断的提交记录…', async () => {
        if (busy) return;
        if (!(await appConfirm(`${t('只删除一小时以前中断的提交记录与清理锁；正在进行的提交与服务器上的作业、数据都不受影响。')}\n${server.host}: ${server.root}\n${t('确认解除？')}`))) return;
        busy = true;
        recover.disabled = true;
        try {
          const result = await invoke('remote_engine_recover', { server });
          show([`${t('已解除中断的提交记录')}：${(result.removed_leases ?? []).length}${result.removed_lock ? ` · ${t('清理锁')}` : ''}`]);
          renderEngines(await invoke('remote_engines', { server }));
        } catch (error) { fail(error); }
        finally { busy = false; recover.disabled = false; }
      });
      engines.append(el('div', { className: 'resource-engine mini' }, [
        el('span', { textContent: `${t('中断的提交记录')}：${stale.length}` }), recover,
      ]));
    }
    for (const engine of inventory.engines ?? []) {
      const reasons = [...(inventory.cleanup_blockers ?? []), ...(engine.protected_reasons ?? [])];
      const clean = button('清理这个快照…', async () => {
        if (busy || !engine.can_clean || reasons.length) return;
        busy = true;
        clean.disabled = true;
        try {
          const path = `${server.root.replace(/\/+$/, '')}/engine/${engine.id}`;
          if (!(await appConfirm(`${t('仅删除以下服务器引擎快照；下次使用可能需要重新编译。')}\n${server.host}: ${path}\n${t('请勿同时用旧版软件向这个服务器目录提交任务。')}\n${t('确认清理？')}`))) return;
          const result = await invoke('remote_engine_clean', { server, snapshot: engine.id });
          show([`${t('引擎快照已清理')}：${result.removed}`, ...(result.warning ? [String(result.warning)] : [])]);
          try { renderEngines(await invoke('remote_engines', { server })); }
          catch (error) { renderEngines(null, String(error?.message || error)); }
        } catch (error) { fail(error); }
        finally { busy = false; clean.disabled = !engine.can_clean || reasons.length > 0; }
      });
      clean.disabled = !engine.can_clean || reasons.length > 0;
      clean.title = reasons.join(' · ') || t('仅清理未被使用的引擎快照');
      const names = engine.versions?.length ? engine.versions.join(' · ') : t('未匹配版本名称');
      engines.append(el('div', { className: 'resource-engine mini' }, [
        el('strong', { textContent: `${engine.ready ? '✓' : '!'} ${names}` }),
        el('span', { textContent: `${engine.id} · ${t(engine.ready ? '已就绪' : '尚未就绪')}` }),
        ...reasons.map(reason => el('span', { className: 'muted', textContent: `${t('清理保护')}：${reason}` })), clean,
      ]));
    }
    if (!inventory.engines?.length) engines.append(el('p', { className: 'mini muted', textContent: t('服务器尚无引擎快照；首次使用时准备。') }));
  }
  renderProbe();
  renderEngines();
  const version = modelVersion();
  const buildLabel = version === OFFICIAL ? '编译官方版内核' : `编译 ${versionLabel(version)} 的内核`;
  return el('div', { className: 'ws-card resource-card' }, [
    el('div', { className: 'ws-head' }, [
      el('b', { textContent: server.host }),
      el('span', { className: 'muted mini', textContent: serverFacts(server) }),
      runTarget() === server.host ? el('span', { className: 'ws-in-use', textContent: t('当前运行位置') }) : '',
    ]),
    summary, login, maps, recordList(server.host), engines,
    el('div', { className: 'pill-row' }, [
      button('设为运行位置', () => useTarget(server.host), 'run-btn'),
      button('检查连接', async event => {
        if (busy) return;
        busy = true;
        if (event?.currentTarget) event.currentTarget.disabled = true;
        show([t('正在连接…')], 'muted');
        try {
          let probe;
          try { probe = await invoke('remote_probe', { server }); }
          catch (error) { recordProbe(server, null, error); renderProbe(); renderEngines(); throw error; }
          recordProbe(server, probe);
          renderProbe();
          show([...probeLines(probe), ...(probe.problems ?? [])]);
          try { renderEngines(await invoke('remote_engines', { server })); }
          catch (error) { renderEngines(null, String(error?.message || error)); }
        } catch (error) { fail(error); }
        finally { busy = false; if (event?.currentTarget) event.currentTarget.disabled = false; }
      }),
      button('查看内核', async () => {
        show([t('正在读取…')], 'muted');
        try { show(kernelLines(await invoke('remote_kernels', { host: server.host }))); } catch (error) { fail(error); }
      }),
      button('修改…', edit),
    ]),
    el('div', { className: 'resource-kernel mini' }, [
      el('span', { className: 'muted', textContent: t('Fortran 内核预设') }), preset,
      button(buildLabel, async event => {
        const target = event.currentTarget;
        target.disabled = true;
        show([`${t('正在服务器上编译')} ${preset.value}（${t('十几分钟，请不要关闭本窗口')}）…`], 'muted');
        try {
          const answer = await invoke('remote_build_kernel', kernelBuildRequest(server.host, preset.value, version));
          show([`✓ ${answer.built ? t('已编好') : t('已经有了，没有重编')}：${answer.name}`, ...[answer.lock_warning].filter(Boolean)], 'assistant-key-ok');
        } catch (error) { fail(error); } finally { target.disabled = false; }
      }),
    ]),
    out,
  ]);
}

export async function refresh() {
  const request = ++refreshRequest;
  renderLocal();
  const list = $('resource-servers');
  if (!list) return;
  if (!hasBackend) { list.replaceChildren(); return; }
  try {
    const config = await invoke('remote_config');
    const servers = config?.servers ?? [];
    let workspaces = [];
    const readyVersions = [];
    let versionError = '';
    try {
      workspaces = (await invoke('workspace_list'))?.workspaces ?? [];
      const kernel = kernelForSubgrid()?.dir;
      if (kernel) {
        for (const ws of workspaces) {
          const ready = [];
          for (const engine of ['rust', 'fortran']) {
            try {
              const result = await invoke('version_check', { name: ws.name, kernel, engine });
              if (result.ready) ready.push(engine === 'rust' ? 'Rust' : 'Fortran');
            } catch { versionError = t('部分修改版准备情况无法读取。'); }
          }
          if (ready.length) readyVersions.push(`${ws.name}（${ready.join(' / ')}）`);
        }
      } else if (workspaces.length) versionError = t('尚无匹配内核，修改版就绪情况未知。');
    } catch (error) { versionError = `${t('读不到修改版列表')}：${error?.message || error}`; }
    const records = await readResourceRecords(workspaces);
    if (request !== refreshRequest) return;
    resourceRecords = records;
    localVersions = readyVersions;
    localVersionError = versionError;
    renderLocal();
    list.replaceChildren(...(servers.length
      ? servers.map(serverCard)
      : [el('p', { className: 'muted', textContent: t('还没有服务器。点“添加服务器…”，填 ssh 能连上的主机名和服务器上的工作目录。') })]),
    ...records.filter(record => record.host === '*').map(record => el('p', { className: 'mini warn', textContent: `${record.label} · ${record.state}` })));
  } catch (error) {
    if (request === refreshRequest) list.replaceChildren(el('p', { className: 'assistant-fail', textContent: String(error?.message || error) }));
  }
}

function wire() {
  if (!$('resource-servers')) return;
  $('resource-add-server')?.addEventListener('click', async () => { await openServerDialog(null); refresh().catch(status); });
  $('resource-refresh')?.addEventListener('click', () => refresh().catch(status));
  $('open-resources')?.addEventListener('click', () => go('resources'));
  addEventListener('colm:step', () => { if (state.step === 'resources') refresh().catch(status); });
  addEventListener('colm:language', () => { if (state.step === 'resources') refresh().catch(status); });
}

wire();

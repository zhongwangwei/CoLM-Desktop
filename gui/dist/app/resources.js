//! 计算资源页：在哪里跑——本机，或一台经你自己的 ssh 连接的服务器。运行方式里的“运行位置”就从这些里选。
//!
//! 一台服务器的详细设置（地址、目录对应、调度、环境准备）仍在 remote.js 的服务器对话框里编辑；这里列出所有资源，
//! 给出检查连接、查看与编译内核（官方版或当前选中的修改版）、设为运行位置这几样操作。

import { invoke, hasBackend } from './ipc.js';
import { state } from './state.js';
import { $, status } from './ui.js';
import { language, translateZh } from './i18n.js';
import { go } from './shell.js';
import { openServerDialog, probeLines, kernelLines, runTarget } from './remote.js';
import { OFFICIAL, modelVersion, versionLabel } from './run-mode.js';
import { recordProbe } from './remote-probe-cache.js';

const t = text => (language() === 'en' ? translateZh(text) : text);

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
    el('p', { className: 'mini muted', textContent: t('官方版直接可用；修改版要先在“改代码”里编好它的引擎。') }),
    el('div', { className: 'pill-row' }, [button('设为运行位置', () => useTarget('local'))]),
  );
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
  const version = modelVersion();
  const buildLabel = version === OFFICIAL ? '编译官方版内核' : `编译 ${versionLabel(version)} 的内核`;
  return el('div', { className: 'ws-card resource-card' }, [
    el('div', { className: 'ws-head' }, [
      el('b', { textContent: server.host }),
      el('span', { className: 'muted mini', textContent: serverFacts(server) }),
      runTarget() === server.host ? el('span', { className: 'ws-in-use', textContent: t('当前运行位置') }) : '',
    ]),
    el('div', { className: 'pill-row' }, [
      button('设为运行位置', () => useTarget(server.host), 'run-btn'),
      button('检查连接', async () => {
        show([t('正在连接…')], 'muted');
        try {
          const probe = await invoke('remote_probe', { server });
          recordProbe(server, probe);
          show(probeLines(probe));
        } catch (error) {
          recordProbe(server, null, error);
          fail(error);
        }
      }),
      button('查看内核', async () => {
        show([t('正在读取…')], 'muted');
        try { show(kernelLines(await invoke('remote_kernels', { host: server.host }))); } catch (error) { fail(error); }
      }),
      button('修改…', async () => { await openServerDialog(server.host); refresh(); }),
    ]),
    el('div', { className: 'resource-kernel mini' }, [
      el('span', { className: 'muted', textContent: t('Fortran 内核预设') }), preset,
      button(buildLabel, async event => {
        const target = event.currentTarget;
        target.disabled = true;
        show([`${t('正在服务器上编译')} ${preset.value}（${t('十几分钟，请不要关闭本窗口')}）…`], 'muted');
        try {
          const answer = await invoke('remote_build_kernel', kernelBuildRequest(server.host, preset.value, version));
          show([`✓ ${answer.built ? t('已编好') : t('已经有了，没有重编')}：${answer.name}`], 'assistant-key-ok');
        } catch (error) { fail(error); } finally { target.disabled = false; }
      }),
    ]),
    out,
  ]);
}

export async function refresh() {
  renderLocal();
  const list = $('resource-servers');
  if (!list) return;
  if (!hasBackend) { list.replaceChildren(); return; }
  try {
    const config = await invoke('remote_config');
    const servers = config?.servers ?? [];
    list.replaceChildren(...(servers.length
      ? servers.map(serverCard)
      : [el('p', { className: 'muted', textContent: t('还没有服务器。点“添加服务器…”，填 ssh 能连上的主机名和服务器上的工作目录。') })]));
  } catch (error) {
    list.replaceChildren(el('p', { className: 'assistant-fail', textContent: String(error?.message || error) }));
  }
}

function wire() {
  if (!$('resource-servers')) return;
  $('resource-add-server')?.addEventListener('click', async () => { await openServerDialog(null); refresh().catch(status); });
  $('resource-refresh')?.addEventListener('click', () => refresh().catch(status));
  $('open-resources')?.addEventListener('click', () => go('resources'));
  addEventListener('colm:step', () => { if (state.step === 'resources') refresh().catch(status); });
}

wire();

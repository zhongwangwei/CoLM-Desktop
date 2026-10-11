//! 运行方式：每次运行 = 算例 × 模型版本 × 运行方式（在哪里跑、用哪个引擎）。
//!
//! 这里只放纯函数和对 `state` 的读取（runner.js、assistant.js、results.js 都要用，测试见 tests/run-mode.mjs）；
//! 页面与上下文条在 run-mode-view.js。
//!
//! 模型版本只有两种：官方版，和一个修改版（开发工作区）。修改版跑的是它自己编出的全部程序，由
//! `colm-cli run --model-version` 解析与检查（`colm_workspace::version`）；界面只显示检查结果、按结果拦住运行。

import { state } from './state.js';

export const OFFICIAL = 'official';

/** 当前选中的模型版本：`official` 或工作区名。 */
export function modelVersion() {
  return state.modelVersion || OFFICIAL;
}

/** 版本在界面上的名字。 */
export function versionLabel(version) {
  return !version || version === OFFICIAL ? '官方版' : `修改版：${version}`;
}

/** 运行位置在界面上的名字。 */
export function targetLabel(target) {
  return !target || target === 'local' ? '本机' : `服务器：${target}`;
}

export function engineLabel(engine) {
  return engine === 'fortran' ? 'Fortran 内核' : 'Rust 引擎';
}

/**
 * 在服务器上跑时的就绪清单：服务器会用这个版本的源码自己编引擎，所以本机有没有编好不要紧，只要源码干净；
 * 改过 Fortran 又用 Fortran 引擎时，要先在那台服务器上编它的内核（本机看不到，给提示，由提交时检查）。
 */
export function readinessFor(readiness, target, engine) {
  if (!readiness || !target || target === 'local') return readiness;
  const items = (readiness.items ?? []).map(item => {
    if (item.code === 'engine_built') {
      return { ...item, ok: true, blocking: false, message: '在服务器上会用这个版本的源码编 Rust 引擎与前处理（第一次要几分钟）' };
    }
    if (item.code === 'kernel_built') {
      return engine === 'fortran'
        ? { ...item, ok: false, blocking: false, message: '这个版本改了 Fortran：用 Fortran 引擎在服务器上跑之前，要先在那台服务器上编它的内核（计算资源 → 编译内核）' }
        : { ...item, ok: true, blocking: false, message: '这个版本改了 Fortran；Rust 引擎只用内核的宏，服务器上用正式内核的清单' };
    }
    return item;
  });
  return { ...readiness, items, ready: !items.some(item => item.blocking) };
}

/**
 * 能不能按这个运行方式开跑；不能就返回原因（直接给用户看的一句话）。
 * `readiness` 是 `version_check` 的结果（只对修改版有意义）。
 */
export function runModeBlocker({ version, target, readiness, engine }) {
  if (!version || version === OFFICIAL) return null;
  if (!readiness || readiness.name !== version) return `还在检查修改版 ${version} 能不能运行，请稍候再试`;
  readiness = readinessFor(readiness, target, engine);
  if (!readiness.ready) {
    const why = (readiness.items ?? []).filter(item => item.blocking).map(item => item.message);
    return `修改版 ${version} 还不能运行：${why.join('；') || '检查没有通过'}`;
  }
  return null;
}

/** 就绪清单里的一行：状态（ok / warn / block）与文字。`target` 不是本机时按服务器的要求看（`readinessFor`）。 */
export function readinessRows(readiness, target = 'local', engine = 'rust') {
  return (readinessFor(readiness, target, engine)?.items ?? []).map(item => ({
    code: item.code,
    kind: item.blocking ? 'block' : (item.ok ? 'ok' : 'warn'),
    text: item.message,
  }));
}

/** 改动落在哪里，版本卡上的一句话。 */
export function touchesText(touches) {
  if (!touches) return '';
  if (touches.engine && touches.kernel) return '改了 Rust 引擎和 Fortran 内核';
  if (touches.engine) return '改了 Rust 引擎';
  if (touches.kernel) return '改了 Fortran 内核';
  return '没改会影响计算结果的代码';
}

/**
 * 上下文条的四段：算例 · 版本 · 位置 · 引擎。每页都显示，助手也读它。
 * `cases` 是本次作用的算例个数，`caseName` 是当前算例名。
 */
export function contextChips({ caseName, cases = 0, version, target, engine }) {
  const caseText = cases > 1 ? `${caseName ?? '—'} 等 ${cases} 个` : (caseName ?? '还没有算例');
  return [
    { key: 'case', label: '算例', value: caseText },
    { key: 'version', label: '版本', value: versionLabel(version) },
    { key: 'target', label: '位置', value: targetLabel(target) },
    { key: 'engine', label: '引擎', value: engineLabel(engine) },
  ];
}

/** 给助手的运行方式快照（与上下文条同源）。 */
export function runModeSnapshot({ version, target, engine, readiness }) {
  return {
    version: version || OFFICIAL,
    version_label: versionLabel(version),
    target: target || 'local',
    engine: engine || 'rust',
    ready: !runModeBlocker({ version, target, readiness, engine }),
    readiness: version && version !== OFFICIAL && readiness?.name === version
      ? readinessRows(readiness, target, engine)
      : [],
  };
}

const short = sha => String(sha ?? '').slice(0, 8);

/**
 * 结果的出处标签（`produced-by.json`）：这批结果是哪一版、哪个引擎、在哪里跑出来的。
 * 没有记录（旧版应用跑的）返回空表。
 */
export function provenanceChips(record) {
  if (!record || typeof record !== 'object') return [];
  const version = record.version ?? {};
  const chips = [];
  if (version.kind === 'workspace') {
    chips.push({ key: 'version', text: `修改版：${version.name}`, title: `提交 ${short(version.head)}，基于 ${short(version.base_commit)}`, accent: true });
  } else {
    chips.push({ key: 'version', text: '官方版', title: version.colm_cli ? `colm-cli ${version.colm_cli}` : '' });
  }
  chips.push({ key: 'engine', text: engineLabel(record.engine), title: `前处理：${record.preprocessors === 'fortran' ? 'Fortran' : 'Rust'}` });
  chips.push({ key: 'target', text: record.location?.kind === 'server' ? '服务器' : '本机', title: '' });
  if (record.kernel?.identity) chips.push({ key: 'kernel', text: `内核 ${record.kernel.identity}`, title: String(record.kernel.dir ?? '') });
  if (record.ok === false) chips.push({ key: 'failed', text: '这次运行失败', title: String(record.error ?? ''), warn: true });
  if (Number.isFinite(record.finished)) {
    chips.push({ key: 'time', text: new Date(record.finished * 1000).toLocaleString(), title: '运行结束时间' });
  }
  return chips;
}

/** 结果是不是用当前选中的运行方式跑出来的；不是就提示（例如刚换了版本还没重跑）。 */
export function provenanceMismatch(record, { version, engine }) {
  if (!record?.version) return null;
  const recorded = record.version.kind === 'workspace' ? record.version.name : OFFICIAL;
  const wanted = version || OFFICIAL;
  if (recorded !== wanted) return `这批结果是用${versionLabel(recorded)}跑的，当前选的是${versionLabel(wanted)}`;
  if (engine && record.engine && record.engine !== engine) return `这批结果是用${engineLabel(record.engine)}跑的，当前选的是${engineLabel(engine)}`;
  return null;
}

/** 研究固定的模型版本（`study-status` 的 `model_version`），研究状态里的一行。 */
export function studyVersionText(pinned) {
  if (!pinned?.name) return '模型版本：官方版';
  return `模型版本：修改版 ${pinned.name}（快照：提交 ${short(pinned.head)}；之后改代码不影响这个研究）`;
}


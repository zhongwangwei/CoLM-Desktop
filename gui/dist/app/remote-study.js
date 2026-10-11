//! 在服务器上跑研究：运行方式选了服务器时，研究交给那台服务器（`remote-study-run`：同一份设定在那里重建并运行），
//! 这里定时查进度，跑完自动取回（`remote-study-fetch`：核对成员一致后把状态与结果移植进本机研究）。
//! 不依赖 results.js（它调用这里），取回后由调用方给的回调刷新页面。

import { invoke } from './ipc.js';
import { $, status } from './ui.js';
import { language, translateZh } from './i18n.js';

const t = text => (language() === 'en' ? translateZh(text) : text);
const POLL_MS = 15_000;

const watching = new Map();

/** 研究在哪里跑：运行方式里的运行位置（本机或服务器名）。 */
export function studyTarget() {
  return $('run-target')?.value || 'local';
}

/** 研究状态里关于服务器的一行（`study-status` 的 `remote`）。 */
export function serverStudyText(remote) {
  if (!remote?.host) return '';
  if (remote.fetched_at) return `在服务器 ${remote.host} 上跑完，结果已取回（成员的模拟输出留在服务器上）`;
  return `在服务器 ${remote.host} 上运行（作业 ${remote.job}）；跑完会自动取回结果`;
}

/** 服务器上作业的一句进度（`remote-study-status`）。 */
export function serverProgressText(answer) {
  const state = answer?.status?.state;
  if (state === 'finished') return answer.status.exit_code === 0 ? '服务器上的研究跑完了，正在取回…' : `服务器上的研究失败（退出码 ${answer.status.exit_code}）`;
  if (state === 'lost') return '服务器上的作业不在了（可能被终止或机器重启）';
  const study = answer?.study;
  if (study?.completed_candidates != null) return `服务器上运行中：已完成 ${study.completed_candidates} 个候选`;
  return state === 'queued' ? '在服务器的调度系统里排队' : '服务器上正在准备（编译引擎、建研究）';
}

/** 把这些研究交给服务器 `host`。`versionFor(dir)` 给每个研究这次带的模型版本（已固定的为 null）。 */
export async function submitStudies(dirs, { host, kernel, engine, jobs, versionFor }) {
  const warnings = [];
  for (const dir of dirs) {
    const submitted = await invoke('remote_study_run', {
      studyDir: dir, host, kernel, engine, jobs, modelVersion: await versionFor(dir),
    });
    if (submitted?.ledger_warning) warnings.push(`${t('研究已提交，但版本记录未写入')}：${dir}: ${submitted.ledger_warning}`);
  }
  return warnings;
}

/** 定时查这个研究，跑完取回并调用 `onFetched`；同一个研究只查一份。 */
export function watchServerStudy(dir, onFetched) {
  if (watching.has(dir)) return;
  const tick = async () => {
    try {
      const answer = await invoke('remote_study_status', { studyDir: dir });
      status(t(serverProgressText(answer)));
      if (answer?.status?.state === 'finished' || answer?.status?.state === 'lost') {
        watching.delete(dir);
        if (answer.status.state === 'finished' && answer.status.exit_code === 0) {
          await invoke('remote_study_fetch', { studyDir: dir });
          status(t('已取回服务器上的研究结果'));
        }
        await onFetched?.(dir);
        return;
      }
    } catch (error) {
      status(error);
    }
    watching.set(dir, setTimeout(tick, POLL_MS));
  };
  watching.set(dir, setTimeout(tick, 1000));
}

/** 等这个研究在服务器上跑完并取回（“比较两个版本”在服务器上跑两边时用）。失败时抛出。 */
export async function awaitServerStudy(dir, { pollMs = POLL_MS, onProgress } = {}) {
  for (;;) {
    const answer = await invoke('remote_study_status', { studyDir: dir });
    onProgress?.(serverProgressText(answer));
    const state = answer?.status?.state;
    if (state === 'finished') {
      if (answer.status.exit_code !== 0) throw new Error(serverProgressText(answer));
      await invoke('remote_study_fetch', { studyDir: dir });
      return;
    }
    if (state === 'lost') throw new Error(serverProgressText(answer));
    await new Promise(resolve => setTimeout(resolve, pollMs));
  }
}

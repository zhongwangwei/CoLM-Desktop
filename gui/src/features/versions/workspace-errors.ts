/**
 * 用本地软件改源码这条路上的报错：打开程序的错来自窗口后端（中文），快照/写入的错来自 colm-cli
 * （英文）。按固定文字识别后给中英两种说法（复验 C2-06）；不认得的原样返回。
 *
 * 写入类有两个**稳定标记**，前端要靠它们走不同恢复路径，不能按整句匹配（句子会随参数变）：
 * - `content-conflict:`：内容基线过期——文件在软件外被改/删了，本次保存被整次拒绝。
 * - `already-exists:`：新建/加入文件时目标已存在——原文件没被动过，要用明确替换或换名字。
 */
export const CONTENT_CONFLICT = 'content-conflict:';
export const ALREADY_EXISTS = 'already-exists:';

/** colm-cli 的错误经过 anyhow 会带 `Error: ` 前缀（可能多层），Tauri 取的是 stderr 末尾几行。 */
const stripError = (error: string) => error.replace(/^(?:Error: )+/, '');

/** 保存被内容基线拒绝：需要进冲突界面（保留用户文本），不能只当普通错误弹一行。 */
export function isContentConflict(error: string): boolean {
  return stripError(error).includes(CONTENT_CONFLICT);
}

/** 新建/加入文件撞名：要让用户改用"打开已有文件"或明确的"替换"。 */
export function isAlreadyExists(error: string): boolean {
  return stripError(error).includes(ALREADY_EXISTS);
}

export function workspaceErrorText(error: string): [string, string] {
  const text = stripError(error);
  const patterns: [RegExp, (m: RegExpExecArray) => [string, string]][] = [
    [/^找不到程序 ([\s\S]+)$/, (m) => [text, `Cannot find the app ${m[1]}`]],
    [/^打不开 ([\s\S]+?)：([\s\S]*)$/, (m) => [text, `Cannot open ${m[1]}: ${m[2]}`]],
    [/^([\s\S]+) 没能打开 ([\s\S]+)$/, (m) => [text, `${m[1]} could not open ${m[2]}`]],
    [/^([\s\S]+) 不存在$/, (m) => [text, `${m[1]} does not exist`]],
    [/^工作区状态里没有目录$/, () => [text, 'The workspace status has no folder']],
    [
      /^(\S+) was changed outside the app(?:(?::|\s*Caused by:)\s*([\s\S]*?))?\s*$/,
      (m) => [
        `${m[1]} 在软件外被改了，但这个位置不能改${m[2] ? `：${m[2]}` : ''}。把它改回去再试。`,
        `${m[1]} was changed outside the app, but it may not be changed${m[2] ? `: ${m[2]}` : ''}. Undo that change and try again.`,
      ],
    ],
    // 内容基线过期：保存一个字节都没写。界面另有冲突面板保留用户文本，这里给的是可读原因。
    [
      new RegExp(
        `^${CONTENT_CONFLICT} (\\S+) changed on disk since it was opened; reload it or overwrite explicitly$`,
      ),
      (m) => [
        `${m[1]} 在软件外被改了，保存已取消；原文件没有被覆盖。可以重新加载它，或明确用你的文本覆盖。`,
        `${m[1]} changed on disk since it was opened; nothing was saved. Reload it, or explicitly overwrite it with your text.`,
      ],
    ],
    [
      new RegExp(
        `^${CONTENT_CONFLICT} (\\S+) was deleted or replaced outside the app since it was opened; reload it or save it again explicitly$`,
      ),
      (m) => [
        `${m[1]} 在软件外被删除或替换了，保存已取消。可以重新加载它，或明确用你的文本重建。`,
        `${m[1]} was deleted or replaced outside the app since it was opened; nothing was saved. Reload it, or explicitly recreate it with your text.`,
      ],
    ],
    [
      new RegExp(`^${ALREADY_EXISTS} (\\S+) already exists in the workspace; (.*)$`),
      (m) => [
        `${m[1]} 已经存在，原文件没有被改动。打开已有文件，或使用明确的"替换"。`,
        `${m[1]} already exists; the existing file was not changed. Open it, or use an explicit replace.`,
      ],
    ],
  ];
  if (text.includes('rollback incomplete:')) {
    const backup = /originals retained at ([^\n]+)/.exec(text)?.[1];
    return [
      `写入失败，检测到额外修改或恢复失败，已保留当前文件${backup ? `及原件备份 ${backup}` : ''}。请核对文件后重试。`,
      text,
    ];
  }
  if (text.includes('timed out waiting for its writer'))
    return ['等待工作区写入锁超时，尚未保存。请等待其他写入操作结束后重试。', text];
  if (text.includes('workspace is busy') || text.includes('legacy write lock'))
    return ['工作区仍有旧版写入操作，或其状态无法确认。请结束该操作并核对锁文件后重试。', text];
  if (text.includes('non-UTF-8'))
    return ['Git 返回了无法识别的文本或文件名。请将文件名改为 UTF-8 后重试。', text];
  if (text.includes('cannot read local Git ignore rules'))
    return ['无法读取工作区的 Git 忽略规则，已保留原文件。请修复该文件的编码或读取权限。', text];
  if (
    text.includes('changed outside this operation') ||
    text.includes('changed again during the operation')
  )
    return ['工作区文件或暂存区在操作期间发生了额外修改。请核对这些修改后重试。', text];
  if (text.startsWith('workspace_in_use:'))
    return [
      '这个工作区仍被草稿引用，请先删除或迁移草稿，再删除工作区。',
      'This workspace is still referenced by drafts. Delete or migrate them before removing it.',
    ];
  for (const [pattern, both] of patterns) {
    const m = pattern.exec(text);
    if (m) return both(m);
  }
  return [text, text];
}

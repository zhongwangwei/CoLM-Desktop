/**
 * 用本地软件改源码这条路上的报错：打开程序的错来自窗口后端（中文），快照的错来自 colm-cli（英文）。
 * 按固定文字识别后给中英两种说法（复验 C2-06）；不认得的原样返回。
 */
export function workspaceErrorText(error: string): [string, string] {
  const text = error.replace(/^(?:Error: )+/, '');
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
  ];
  for (const [pattern, both] of patterns) {
    const m = pattern.exec(text);
    if (m) return both(m);
  }
  return [text, text];
}

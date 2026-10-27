/**
 * 助手回复用的安全 Markdown 子集（U-07）。为什么不引入第三方渲染库：助手输出会带工具参数、
 * 文件内容甚至网页片段，任何"先转 HTML 再插入"的实现都需要一段可信的消毒器；这里只解析成
 * 我们自己的纯数据 AST，再由 React 逐节点创建元素，原始 HTML 永远不会被当成标记（React 文本节点
 * 会转义），也不需要 `dangerouslySetInnerHTML`。
 *
 * 支持的语法：围栏代码块、行内代码、粗体/斜体、无序/有序列表、标题、引用、管道表格、
 * `[文字](http/https 地址)` 链接。其余标记（HTML、图片、相对地址、javascript:/data: 等）一律当普通
 * 文字显示。链接只允许 http/https，渲染层会加 target=_blank rel=noopener noreferrer。
 */
export type MarkdownInline =
  | { kind: 'text'; text: string }
  | { kind: 'code'; text: string }
  | { kind: 'strong'; children: MarkdownInline[] }
  | { kind: 'em'; children: MarkdownInline[] }
  | { kind: 'link'; text: string; url: string; children: MarkdownInline[] };

export type MarkdownBlock =
  | { kind: 'heading'; level: number; children: MarkdownInline[] }
  | { kind: 'paragraph'; children: MarkdownInline[] }
  | { kind: 'code'; lang: string; code: string }
  | { kind: 'list'; ordered: boolean; start: number; items: MarkdownInline[][] }
  | { kind: 'quote'; lines: MarkdownInline[][] }
  | { kind: 'table'; header: MarkdownInline[][]; rows: MarkdownInline[][][] };

/**
 * 只接受绝对 http/https 地址。解析失败（相对路径、空串）和其它协议（javascript:、data:、
 * file: 等）都返回 null，调用方按普通文字渲染 —— 这些都不是模型回答里该出现的可点击目标。
 */
export function safeHttpUrl(raw: string): string | null {
  const text = raw.trim();
  if (!text) return null;
  try {
    const url = new URL(text);
    if (url.protocol !== 'http:' && url.protocol !== 'https:') return null;
    return url.toString();
  } catch {
    return null;
  }
}

export function parseInline(text: string): MarkdownInline[] {
  // 行内标记：代码 > 链接 > 粗体 > 斜体。代码最先是因为代码里的 * 和 [ ] 不能当标记；
  // 粗体在斜体前，避免 **x** 先被单星号拆开。
  // 正则必须每次调用新建：粗体/斜体内部会递归调用本函数，共用一个带 lastIndex 的全局正则会让
  // 外层循环被内层重置，解析 URL 里的括号时直接死循环（实测会堆栈溢出）。
  // URL 允许一层配对括号（维基百科式的 .../Foo_(bar)），否则链接会在第一个右括号被截断。
  const token =
    /`([^`\n]+)`|\[([^\]\n]+)\]\(((?:[^()\s]|\([^()\s]*\))+)\)|\*\*([^*\n]+)\*\*|\*([^*\n]+)\*/g;
  const nodes: MarkdownInline[] = [];
  let consumed = 0;
  let match: RegExpExecArray | null;
  while ((match = token.exec(text))) {
    if (match.index > consumed)
      nodes.push({ kind: 'text', text: text.slice(consumed, match.index) });
    const [raw, code, linkText, linkUrl, strong, em] = match;
    if (code !== undefined) nodes.push({ kind: 'code', text: code });
    else if (linkText !== undefined && linkUrl !== undefined) {
      const url = safeHttpUrl(linkUrl);
      // 危险/非法链接整段按原文显示：既不给可点击入口，也不吞掉用户能看懂的内容。
      if (url) nodes.push({ kind: 'link', text: linkText, url, children: parseInline(linkText) });
      else nodes.push({ kind: 'text', text: raw });
    } else if (strong !== undefined) nodes.push({ kind: 'strong', children: parseInline(strong) });
    else if (em !== undefined) nodes.push({ kind: 'em', children: parseInline(em) });
    consumed = match.index + raw.length;
  }
  if (consumed < text.length) nodes.push({ kind: 'text', text: text.slice(consumed) });
  return nodes;
}

const FENCE = /^\s{0,3}```\s*([\w.+#-]*)\s*$/;
const FENCE_CLOSE = /^\s{0,3}```\s*$/;
const HEADING = /^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/;
const QUOTE = /^\s{0,3}>\s?/;
const LIST_ITEM = /^(\s*)([-*+]|(\d+)[.)])\s+(.*)$/;
const TABLE_DIVIDER = /^\s*\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)*\|?\s*$/;

function isTableStart(lines: string[], index: number) {
  return (
    index + 1 < lines.length &&
    lines[index].includes('|') &&
    TABLE_DIVIDER.test(lines[index + 1]) &&
    splitRow(lines[index]).length > 0
  );
}

function splitRow(line: string): string[] {
  let text = line.trim();
  if (text.startsWith('|')) text = text.slice(1);
  if (text.endsWith('|')) text = text.slice(0, -1);
  return text.split('|').map((cell) => cell.trim());
}

function startsBlock(lines: string[], index: number) {
  const line = lines[index];
  return (
    !line.trim() ||
    FENCE.test(line) ||
    HEADING.test(line) ||
    QUOTE.test(line) ||
    LIST_ITEM.test(line) ||
    isTableStart(lines, index)
  );
}

/**
 * 把助手回复切成块。输入永远先按纯文本处理：没有转义/解码步骤，所以 `<script>` 只是几行字。
 */
export function parseMarkdown(text: string): MarkdownBlock[] {
  const lines = text.replace(/\r\n?/g, '\n').split('\n');
  const blocks: MarkdownBlock[] = [];
  let at = 0;
  while (at < lines.length) {
    const line = lines[at];
    if (!line.trim()) {
      at++;
      continue;
    }
    const fence = line.match(FENCE);
    if (fence) {
      const code: string[] = [];
      at++;
      // 流式输出时收尾的 ``` 可能还没到：先把已有的当代码块显示，等收尾到了再补全。
      while (at < lines.length && !FENCE_CLOSE.test(lines[at])) {
        code.push(lines[at]);
        at++;
      }
      if (at < lines.length) at++;
      blocks.push({ kind: 'code', lang: fence[1] || '', code: code.join('\n') });
      continue;
    }
    const heading = line.match(HEADING);
    if (heading) {
      blocks.push({
        kind: 'heading',
        level: heading[1].length,
        children: parseInline(heading[2]),
      });
      at++;
      continue;
    }
    if (QUOTE.test(line)) {
      const quoted: MarkdownInline[][] = [];
      while (at < lines.length && QUOTE.test(lines[at])) {
        quoted.push(parseInline(lines[at].replace(QUOTE, '')));
        at++;
      }
      blocks.push({ kind: 'quote', lines: quoted });
      continue;
    }
    if (isTableStart(lines, at)) {
      const header = splitRow(lines[at]).map(parseInline);
      at += 2; // 表头和 |---| 分隔行
      const rows: MarkdownInline[][][] = [];
      while (at < lines.length && lines[at].trim() && lines[at].includes('|')) {
        const cells = splitRow(lines[at]).slice(0, header.length);
        while (cells.length < header.length) cells.push('');
        rows.push(cells.map(parseInline));
        at++;
      }
      blocks.push({ kind: 'table', header, rows });
      continue;
    }
    const item = line.match(LIST_ITEM);
    if (item) {
      const ordered = !!item[3];
      const start = ordered ? Number(item[3]) : 1;
      const items: MarkdownInline[][] = [];
      while (at < lines.length) {
        const next = lines[at].match(LIST_ITEM);
        if (!next) break;
        // 列表项里的缩进续行并进同一项，避免把换行当新块。
        let text = next[4];
        at++;
        while (at < lines.length && /^\s{2,}\S/.test(lines[at]) && !LIST_ITEM.test(lines[at])) {
          text += `\n${lines[at].trim()}`;
          at++;
        }
        items.push(parseInline(text));
      }
      blocks.push({ kind: 'list', ordered, start, items });
      continue;
    }
    const paragraph: string[] = [line.trim()];
    at++;
    while (at < lines.length && !startsBlock(lines, at)) {
      paragraph.push(lines[at].trim());
      at++;
    }
    blocks.push({ kind: 'paragraph', children: parseInline(paragraph.join('\n')) });
  }
  return blocks;
}

/** 代码块"复制"按钮用：把写剪贴板这一步独立出来，便于在没有 DOM 的测试里核对调用内容。 */
export interface ClipboardWriter {
  writeText(text: string): Promise<void>;
}

export async function copyText(
  text: string,
  clipboard: ClipboardWriter | null | undefined = globalThis.navigator?.clipboard,
): Promise<void> {
  if (!clipboard?.writeText) throw new Error('clipboard is not available');
  await clipboard.writeText(text);
}

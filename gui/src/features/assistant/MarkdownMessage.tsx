import { useState, type ReactNode } from 'react';
import { copyText, parseMarkdown, type MarkdownBlock, type MarkdownInline } from './markdown';

/**
 * 安全 Markdown 渲染（U-07）：把 `markdown.ts` 解析出的纯数据块映射成 React 元素。
 * 这里没有任何 HTML 注入入口 —— 文本一律作为 React 子节点，源码里的标签只会显示成字面文字。
 */

function Inline({ nodes }: { nodes: MarkdownInline[] }) {
  return (
    <>
      {nodes.map((node, index) => {
        switch (node.kind) {
          case 'text':
            // 纯文本直接作为字符串子节点：少一层 span，也让"原始 HTML 只是文字"在输出上更直白。
            return node.text;
          case 'code':
            return (
              <code key={index} className="rounded bg-muted px-1 py-0.5 text-[0.92em]">
                {node.text}
              </code>
            );
          case 'strong':
            return (
              <strong key={index}>
                <Inline nodes={node.children} />
              </strong>
            );
          case 'em':
            return (
              <em key={index}>
                <Inline nodes={node.children} />
              </em>
            );
          case 'link':
            return (
              // 来源链接只按 http/https 出现（解析层已过滤），新窗口打开且断开 opener。
              <a
                key={index}
                href={node.url}
                target="_blank"
                rel="noopener noreferrer"
                className="underline"
              >
                <Inline nodes={node.children} />
              </a>
            );
        }
      })}
    </>
  );
}

function CodeBlock({ block, t }: { block: Extract<MarkdownBlock, { kind: 'code' }>; t: Copy }) {
  const [status, setStatus] = useState<'idle' | 'copied' | 'failed'>('idle');
  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
        <span className="truncate">{block.lang || t('代码', 'Code')}</span>
        <button
          type="button"
          className="button button-ghost text-xs"
          aria-label={t('复制代码', 'Copy code')}
          onClick={() => {
            void copyText(block.code).then(
              () => setStatus('copied'),
              () => setStatus('failed'),
            );
          }}
        >
          {status === 'copied'
            ? t('已复制', 'Copied')
            : status === 'failed'
              ? t('复制失败', 'Copy failed')
              : t('复制', 'Copy')}
        </button>
      </div>
      <pre className="overflow-auto rounded border bg-muted p-2 text-xs">
        <code>{block.code}</code>
      </pre>
    </div>
  );
}

type Copy = (zh: string, en: string) => string;

function Heading({ block }: { block: Extract<MarkdownBlock, { kind: 'heading' }> }) {
  const content = <Inline nodes={block.children} />;
  const className = block.level <= 2 ? 'text-base font-bold' : 'text-sm font-semibold';
  switch (block.level) {
    case 1:
      return <h1 className={className}>{content}</h1>;
    case 2:
      return <h2 className={className}>{content}</h2>;
    case 3:
      return <h3 className={className}>{content}</h3>;
    case 4:
      return <h4 className={className}>{content}</h4>;
    case 5:
      return <h5 className={className}>{content}</h5>;
    default:
      return <h6 className={className}>{content}</h6>;
  }
}

function Block({ block, t }: { block: MarkdownBlock; t: Copy }): ReactNode {
  switch (block.kind) {
    case 'heading':
      return <Heading block={block} />;
    case 'paragraph':
      return (
        <p className="whitespace-pre-wrap break-words">
          <Inline nodes={block.children} />
        </p>
      );
    case 'code':
      return <CodeBlock block={block} t={t} />;
    case 'list':
      return block.ordered ? (
        <ol
          className="list-decimal space-y-1 pl-5"
          start={block.start > 1 ? block.start : undefined}
        >
          {block.items.map((item, index) => (
            <li key={index}>
              <Inline nodes={item} />
            </li>
          ))}
        </ol>
      ) : (
        <ul className="list-disc space-y-1 pl-5">
          {block.items.map((item, index) => (
            <li key={index}>
              <Inline nodes={item} />
            </li>
          ))}
        </ul>
      );
    case 'quote':
      return (
        <blockquote className="space-y-1 border-l-2 pl-3 text-muted-foreground">
          {block.lines.map((line, index) => (
            <p key={index} className="whitespace-pre-wrap break-words">
              <Inline nodes={line} />
            </p>
          ))}
        </blockquote>
      );
    case 'table':
      return (
        <div className="overflow-x-auto">
          <table className="w-full border-collapse text-xs">
            <thead>
              <tr>
                {block.header.map((cell, index) => (
                  <th key={index} className="border px-2 py-1 text-left font-semibold">
                    <Inline nodes={cell} />
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {block.rows.map((row, rowIndex) => (
                <tr key={rowIndex}>
                  {row.map((cell, index) => (
                    <td key={index} className="border px-2 py-1 align-top">
                      <Inline nodes={cell} />
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
  }
}

export function Markdown({ text, t }: { text: string; t: Copy }) {
  const blocks = parseMarkdown(text);
  return (
    <div className="space-y-2">
      {blocks.map((block, index) => (
        <Block key={index} block={block} t={t} />
      ))}
    </div>
  );
}

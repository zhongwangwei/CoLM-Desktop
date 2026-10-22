import type { UiAction } from './guide';

/**
 * 测试用：页面测试把页面源码拿出来直接执行，这里给它一个会保留元素树的 `React.createElement`，
 * 再从树里收集 `ActionButton`，得到助手此刻能按的动作（第 737 轮起动作由按钮自己登记）。
 */
type Node = { type: unknown; props: Record<string, unknown> } | Node[] | string | null | undefined;
export function ActionButtonStub() {
  return null;
}
export const treeReact = {
  createElement: (
    type: unknown,
    props: Record<string, unknown> | null,
    ...children: unknown[]
  ) => ({
    type,
    props: { ...(props ?? {}), children },
  }),
  Fragment: 'Fragment',
};
export function treeActions(node: unknown): UiAction[] {
  if (!node || typeof node !== 'object') return [];
  if (Array.isArray(node)) return node.flatMap(treeActions);
  const element = node as Exclude<Node, Node[] | string | null | undefined>;
  const own =
    element.type === ActionButtonStub
      ? [
          {
            id: String(element.props.action),
            label: String(element.props.label),
            kind: (element.props.kind as UiAction['kind']) ?? 'click',
            enabled: !element.props.disabled,
            run: element.props.run as UiAction['run'],
          },
        ]
      : [];
  return [...own, ...treeActions(element.props?.children)];
}

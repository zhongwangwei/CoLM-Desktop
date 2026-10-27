import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import {
  BOTTOM_SLACK_PX,
  atBottom,
  followAfterContent,
  followAfterJump,
  followAfterScroll,
} from './scroll';

const panelSource = readFileSync(new URL('./AssistantPanel.tsx', import.meta.url), 'utf8');
const source = ts.createSourceFile(
  'AssistantPanel.tsx',
  panelSource,
  ts.ScriptTarget.Latest,
  true,
  ts.ScriptKind.TSX,
);
const effects: string[] = [];
const functions = new Map<string, string>();
function visit(node: ts.Node) {
  if (ts.isCallExpression(node) && node.expression.getText(source) === 'useEffect')
    effects.push(node.arguments[0].getText(source));
  if (ts.isFunctionDeclaration(node) && node.name)
    functions.set(node.name.text, node.getText(source));
  ts.forEachChild(node, visit);
}
visit(source);

function evaluate<T>(code: string, name: string, context: Record<string, unknown>): T {
  const js = ts.transpileModule(code, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  return new Function(...Object.keys(context), `${js}; return ${name};`)(
    ...Object.values(context),
  ) as T;
}
function compileFunction<T>(name: string, context: Record<string, unknown>): T {
  const body = functions.get(name);
  assert.ok(body, `AssistantPanel.tsx defines ${name}`);
  return evaluate<T>(body, name, context);
}
function compileExpression<T>(expression: string, context: Record<string, unknown>): T {
  return evaluate<T>(`const value = (${expression});`, 'value', context);
}

test('scroll follow state pauses above the bottom and resumes there', () => {
  assert.equal(atBottom({ scrollTop: 900, scrollHeight: 1000, clientHeight: 100 }), true);
  assert.equal(
    atBottom({ scrollTop: 1000 - 100 - BOTTOM_SLACK_PX, scrollHeight: 1000, clientHeight: 100 }),
    true,
    '整数取整误差范围内仍算在底部',
  );
  assert.equal(
    atBottom({
      scrollTop: 1000 - 100 - BOTTOM_SLACK_PX - 1,
      scrollHeight: 1000,
      clientHeight: 100,
    }),
    false,
  );
  // 上滚暂停，保留已有未读；回到底部恢复跟随并清未读。
  const paused = followAfterScroll(
    { stick: true, unread: false },
    {
      scrollTop: 100,
      scrollHeight: 1000,
      clientHeight: 100,
    },
  );
  assert.deepEqual(paused, { stick: false, unread: false });
  assert.deepEqual(followAfterContent(paused), { stick: false, unread: true });
  const back = followAfterScroll(
    { stick: false, unread: true },
    {
      scrollTop: 900,
      scrollHeight: 1000,
      clientHeight: 100,
    },
  );
  assert.deepEqual(back, { stick: true, unread: false });
  assert.deepEqual(followAfterContent({ stick: true, unread: true }), {
    stick: true,
    unread: false,
  });
  assert.deepEqual(followAfterJump(), { stick: true, unread: false });
});

interface FakeLog {
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
  scrollTo(options: { top: number }): void;
  scrolled: number[];
}

function fakeLog(scrollTop: number, scrollHeight = 1000, clientHeight = 200): FakeLog {
  const log: FakeLog = {
    scrollTop,
    scrollHeight,
    clientHeight,
    scrolled: [],
    scrollTo(options) {
      this.scrolled.push(options.top);
      this.scrollTop = options.top;
    },
  };
  return log;
}

function followingEffect(stick: boolean, log: FakeLog) {
  const calls: string[] = [];
  const latestMessages = { current: [] as unknown[] };
  const effect = effects.find((body) => body.includes('scrollTo'));
  assert.ok(effect, 'the conversation scroll effect is present');
  const run = (messages: unknown[], approval: unknown = null) => {
    const context = {
      messages,
      approval,
      stick: { current: stick },
      unread: false,
      latestMessages,
      log: { current: log },
      followAfterContent,
      setUnread: (value: boolean) => calls.push(`unread:${value}`),
    };
    compileExpression<() => void>(effect, context)();
  };
  return { calls, latestMessages, run };
}

test('new output scrolls to the bottom only while the user is following it', () => {
  const follower = followingEffect(true, fakeLog(0));
  follower.run([{ kind: 'assistant', text: 'a' }]);
  assert.deepEqual(follower.calls, ['unread:false'], '跟随中：清掉未读');
  assert.equal(follower.latestMessages.current.length, 1);

  const scrollLog = fakeLog(0);
  const scrolling = followingEffect(true, scrollLog);
  scrolling.run([{ kind: 'assistant', text: 'a' }]);
  assert.deepEqual(scrollLog.scrolled, [1000], '跟随中：滚到底部');

  const pausedLog = fakeLog(100);
  const paused = followingEffect(false, pausedLog);
  paused.run([{ kind: 'assistant', text: 'a' }]);
  assert.deepEqual(pausedLog.scrolled, []);
  assert.deepEqual(paused.calls, ['unread:true'], '暂停中：标记有新内容，不抢滚动位置');

  // approval 变化不是"新内容"（审批卡在对话区外面），不能给暂停用户亮新内容提示。
  const approvalOnly = followingEffect(false, fakeLog(100));
  const same = [{ kind: 'assistant', text: 'a' }];
  approvalOnly.latestMessages.current = same; // 上一轮已经显示这批消息
  approvalOnly.run(same, { type: 'approval_request' });
  assert.deepEqual(approvalOnly.calls, []);

  // 空对话（欢迎页）不滚动。
  const welcome = followingEffect(true, fakeLog(0));
  welcome.run([]);
  assert.deepEqual(welcome.calls, []);
});

test('user scrolling up pauses following and scrolling back resumes it', () => {
  const upLog = fakeLog(200);
  const upStick = { current: true };
  let upUnread = false;
  const up = compileFunction<() => void>('trackScroll', {
    log: { current: upLog },
    stick: upStick,
    unread: upUnread,
    followAfterScroll,
    setUnread: (value: boolean) => (upUnread = value),
  });
  up();
  assert.equal(upStick.current, false, '上滚后暂停自动滚底');
  assert.equal(upUnread, false);

  const downLog = fakeLog(800);
  const downStick = { current: false };
  let downUnread = true;
  const down = compileFunction<() => void>('trackScroll', {
    log: { current: downLog },
    stick: downStick,
    unread: downUnread,
    followAfterScroll,
    setUnread: (value: boolean) => (downUnread = value),
  });
  down();
  assert.equal(downStick.current, true, '回到底部后恢复自动滚动');
  assert.equal(downUnread, false);
});

test('the new-content button jumps to the latest message and resumes following', () => {
  const log = fakeLog(100);
  const stick = { current: false };
  let unread = true;
  const jump = compileFunction<() => void>('scrollToLatest', {
    log: { current: log },
    stick,
    unread,
    followAfterJump,
    setUnread: (value: boolean) => (unread = value),
  });
  jump();
  assert.deepEqual(log.scrolled, [1000]);
  assert.equal(stick.current, true);
  assert.equal(unread, false);

  const resetStick = { current: false };
  let resetUnread = true;
  const reset = compileFunction<() => void>('resetFollow', {
    stick: resetStick,
    unread: resetUnread,
    followAfterJump,
    setUnread: (value: boolean) => (resetUnread = value),
  });
  reset();
  assert.equal(resetUnread, false);
  assert.equal(resetStick.current, true);
});

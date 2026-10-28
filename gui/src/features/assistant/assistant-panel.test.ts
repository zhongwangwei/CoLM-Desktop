import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { withTimeout } from '../../lib/ipc';
import {
  defaultSettings,
  egressTarget,
  rejectedBeforeTurn,
  withoutLastUser,
  type AgentEvent,
  type Message,
  type Settings,
} from './model';
import {
  clearsDraftAfterAck,
  newTurn,
  turnDelivered,
  turnFailed,
  turnOutput,
  turnPreTurnRejected,
  type PendingTurn,
} from './turn';

/** 与页面里的 DocumentAttachment 对齐；抽出的函数只读 path/name/text 等字段。 */
interface Attachment {
  path: string;
  name: string;
  text: string;
  pages?: number | null;
  truncated?: boolean;
  warnings?: string[];
}

const panelSource = readFileSync(new URL('./AssistantPanel.tsx', import.meta.url), 'utf8');
const source = ts.createSourceFile(
  'AssistantPanel.tsx',
  panelSource,
  ts.ScriptTarget.Latest,
  true,
  ts.ScriptKind.TSX,
);
const effects: { body: string; deps: string }[] = [];
const functions = new Map<string, string>();
const variables = new Map<string, string>();
function visit(node: ts.Node) {
  if (ts.isCallExpression(node) && node.expression.getText(source) === 'useEffect')
    effects.push({
      body: node.arguments[0].getText(source),
      deps: node.arguments[1]?.getText(source) ?? '',
    });
  if (ts.isFunctionDeclaration(node) && node.name)
    functions.set(node.name.text, node.getText(source));
  if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.initializer)
    variables.set(node.name.text, node.initializer.getText(source));
  ts.forEachChild(node, visit);
}
visit(source);

const settle = () => new Promise<void>((resolve) => setImmediate(resolve));

function evaluate<T>(code: string, name: string, context: Record<string, unknown>): T {
  const js = ts.transpileModule(code, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  return new Function(...Object.keys(context), `${js}; return ${name};`)(
    ...Object.values(context),
  ) as T;
}
function compileExpression<T>(expression: string, context: Record<string, unknown>): T {
  return evaluate<T>(`const value = (${expression});`, 'value', context);
}
function compileFunction<T>(name: string, context: Record<string, unknown>): T {
  const body = functions.get(name);
  assert.ok(body, `AssistantPanel.tsx defines ${name}`);
  return evaluate<T>(body, name, context);
}
function effectWith(marker: string) {
  const effect = effects.find((item) => item.body.includes(marker));
  assert.ok(effect, `AssistantPanel.tsx has an effect containing ${marker}`);
  return effect;
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}
function attachment(name: string, text = `content of ${name}`): Attachment {
  return { path: `/${name}`, name, text };
}

// ---------------------------------------------------------------------------
// GUI-16：核心设置可重试、辅助历史可降级、重试不重复订阅
// ---------------------------------------------------------------------------

function initHarness(loadSettings: () => Promise<Settings>, loadHistory: () => Promise<string[]>) {
  const core = effectWith("'assistant_settings'");
  const historyEffect = effectWith("'assistant_input_history'");
  const ui = {
    phase: 'idle' as string,
    initError: '',
    ready: false,
    settings: undefined as Settings | undefined,
    historyError: '',
  };
  const history = { current: [] as string[] };
  const commands: string[] = [];
  const base = {
    hasBackend: () => true,
    withTimeout,
    SETTINGS_TIMEOUT_MS: 25,
    HISTORY_TIMEOUT_MS: 25,
    t: (_zh: string, en: string) => en,
    errorText: (message: string) => String(message).replace(/^Error: /, ''),
  };
  const runCore = compileExpression<() => () => void>(core.body, {
    ...base,
    invoke: (command: string) => {
      commands.push(command);
      return loadSettings();
    },
    setSettings: (value: Settings) => {
      ui.settings = value;
    },
    setReady: (value: boolean) => {
      ui.ready = value;
    },
    setInitPhase: (value: string) => {
      ui.phase = value;
    },
    setInitError: (value: string) => {
      ui.initError = value;
    },
  });
  const runHistory = compileExpression<() => () => void>(historyEffect.body, {
    ...base,
    invoke: (command: string) => {
      commands.push(command);
      return loadHistory();
    },
    history,
    setHistoryError: (value: string) => {
      ui.historyError = value;
    },
  });
  return {
    ui,
    history,
    commands,
    core,
    historyEffect,
    startCore: () => runCore(),
    startHistory: () => runHistory(),
  };
}

test('GUI-16: a failed settings load is visible and retry recovers without restarting', async () => {
  let fail = true;
  const harness = initHarness(
    async () => {
      if (fail) throw new Error('settings.json is unreadable');
      return defaultSettings;
    },
    async () => [],
  );
  // 核心设置与辅助历史必须是两条独立的初始化路径，历史失败不能拖垮设置。
  assert.ok(!harness.core.body.includes('assistant_input_history'));
  assert.ok(!harness.historyEffect.body.includes('assistant_settings'));
  assert.match(harness.core.deps, /initAttempt/, '重试通过重跑核心 effect 实现');
  assert.match(panelSource, /initPhase === 'failed'/, '页面渲染明确的失败态');
  assert.match(panelSource, /重试读取设置/, '失败态有可见的重试入口');

  harness.startCore();
  await settle();
  assert.equal(harness.ui.phase, 'failed');
  assert.match(harness.ui.initError, /unreadable/);
  assert.equal(harness.ui.ready, false, '设置没读到之前助手不能假装可用');

  fail = false;
  harness.startCore();
  await settle();
  assert.equal(harness.ui.phase, 'ready');
  assert.equal(harness.ui.ready, true);
  assert.equal(harness.ui.initError, '');
  assert.equal(harness.ui.settings?.model, defaultSettings.model);
});

test('GUI-16: a settings step that never answers times out into a retryable failure', async () => {
  const harness = initHarness(
    () => new Promise<Settings>(() => {}),
    async () => [],
  );
  harness.startCore();
  await new Promise((resolve) => setTimeout(resolve, 80));
  assert.equal(harness.ui.phase, 'failed', '不能永远停在 loading');
  assert.match(harness.ui.initError, /timed out .*loading assistant settings/i);
  assert.equal(harness.ui.ready, false);
});

test('GUI-16: failed input history uses an empty history and never blocks the assistant', async () => {
  let historyFails = true;
  const harness = initHarness(
    async () => defaultSettings,
    async () => {
      if (historyFails) throw new Error('colm-cli not found');
      return ['earlier question'];
    },
  );
  harness.startCore();
  harness.startHistory();
  await settle();
  // 设置照常 ready：历史只是 ↑/↓ 的记忆，失败不能把整个助手禁用到重启应用。
  assert.equal(harness.ui.ready, true);
  assert.equal(harness.ui.phase, 'ready');
  assert.match(harness.ui.historyError, /colm-cli not found/);
  assert.deepEqual(harness.history.current, []);

  historyFails = false;
  harness.startHistory();
  await settle();
  assert.deepEqual(harness.history.current, ['earlier question']);
  assert.equal(harness.ui.historyError, '');
});

test('GUI-16: retrying initialization never subscribes to the event stream twice', async () => {
  const subscription = effectWith("'assistant://event'");
  assert.equal(subscription.deps, '[]', '事件订阅只跟挂载有关，重试不能重跑它');
  const listens: string[] = [];
  let unlistened = 0;
  const runSubscription = compileExpression<() => () => void>(subscription.body, {
    hasBackend: () => true,
    listen: async (event: string) => {
      listens.push(event);
      return () => {
        unlistened++;
      };
    },
    setError: () => {},
    grant: { current: null },
    runningRef: { current: false },
    invoke: async () => undefined,
    outgoing: { current: null },
  });
  const cleanup = runSubscription();
  await settle();
  assert.deepEqual(listens, ['assistant://event']);

  // 用户点"重试"= 重跑初始化 effect；监听仍然只有一份，也不会启动/重发任何一轮对话。
  let settingsFail = true;
  const harness = initHarness(
    async () => {
      if (settingsFail) throw new Error('unreachable settings');
      return defaultSettings;
    },
    async () => [],
  );
  harness.startCore();
  harness.startHistory();
  await settle();
  settingsFail = false;
  harness.startCore();
  harness.startHistory();
  await settle();
  assert.deepEqual(listens, ['assistant://event']);
  assert.equal(
    harness.commands.filter((command) => command === 'assistant_settings').length,
    2,
    '重试只重读设置',
  );
  assert.deepEqual(
    harness.commands.filter(
      (command) => command === 'assistant_start' || command === 'assistant_send',
    ),
    [],
    '重试不会启动或重发任何一轮回答',
  );

  cleanup();
  assert.equal(unlistened, 1);
});

// ---------------------------------------------------------------------------
// GUI-17：附件读取绑定对话/草稿代次
// ---------------------------------------------------------------------------

function attachHarness(pick: () => Promise<Attachment | null>, charsLimit = 200_000) {
  const conversationRequest = { current: 0 };
  const ui = {
    attaching: false,
    error: '',
    notice: '',
    attachments: [] as Attachment[],
  };
  const attachmentsRef = { current: [] as Attachment[] };
  const setAttachments = (update: Attachment[] | ((old: Attachment[]) => Attachment[])) => {
    ui.attachments = typeof update === 'function' ? update(ui.attachments) : update;
    attachmentsRef.current = ui.attachments;
  };
  const attach = compileFunction<(hint?: unknown) => Promise<void>>('attachDocument', {
    runningRef: { current: false },
    ready: true,
    conversationRequest,
    invokeJson: pick,
    ATTACHMENT_CHARS_LIMIT: charsLimit,
    attachmentsRef,
    t: (_zh: string, en: string) => en,
    attachmentErrorText: (message: string) => String(message),
    setAttaching: (value: boolean) => {
      ui.attaching = value;
    },
    setAttachNotice: (value: string) => {
      ui.notice = value;
    },
    setAttachments,
    setError: (value: string) => {
      ui.error = value;
    },
  });
  return { ui, conversationRequest, attachmentsRef, attach };
}

test('GUI-17: a slow document read does not leak into a new conversation', async () => {
  const pick = deferred<Attachment | null>();
  const harness = attachHarness(() => pick.promise);
  const reading = harness.attach();
  assert.equal(harness.ui.attaching, true, '读取期间状态可见');
  // "新对话"按钮的动作就是 conversationRequest++（与页面里的实现相同）。
  harness.conversationRequest.current++;
  pick.resolve(attachment('slow-paper.pdf'));
  await reading;
  assert.deepEqual(harness.ui.attachments, []);
  assert.deepEqual(harness.attachmentsRef.current, []);
  assert.match(harness.ui.notice, /conversation changed/i);
  assert.equal(harness.ui.error, '', '切换对话不是读取失败');
  assert.equal(harness.ui.attaching, false);
});

test('GUI-17: opening a saved conversation also invalidates the in-flight read', async () => {
  const pick = deferred<Attachment | null>();
  const harness = attachHarness(() => pick.promise);
  const reading = harness.attach();
  // 打开历史会话在页面里同样先 ++conversationRequest，再拉 transcript。
  harness.conversationRequest.current++;
  pick.resolve(attachment('paper.pdf'));
  await reading;
  assert.deepEqual(harness.ui.attachments, []);
  assert.match(harness.ui.notice, /conversation changed/i);
});

test('GUI-17: a document read in the same conversation still attaches and truncates', async () => {
  const pick = deferred<Attachment | null>();
  const harness = attachHarness(() => pick.promise, 10);
  const reading = harness.attach();
  pick.resolve(attachment('big.pdf', '0123456789ABCDEF'));
  await reading;
  assert.equal(harness.ui.attachments.length, 1);
  assert.equal(harness.ui.attachments[0].text, '0123456789');
  assert.equal(harness.ui.attachments[0].truncated, true);
  assert.equal(harness.ui.error, '');
  assert.equal(harness.ui.attaching, false);
});

test('GUI-17: cancelling the picker is quiet; read failures are reported in place', async () => {
  for (const outcome of ['cancel', 'failure'] as const) {
    const pick = deferred<Attachment | null>();
    const harness = attachHarness(() => pick.promise);
    const reading = harness.attach();
    assert.equal(harness.ui.attaching, true);
    if (outcome === 'cancel') pick.resolve(null);
    else pick.reject(new Error('unsupported document type'));
    await reading;
    assert.deepEqual(harness.ui.attachments, []);
    assert.equal(harness.ui.attaching, false, '取消/失败后不会卡在"正在读取"');
    if (outcome === 'cancel') {
      assert.equal(harness.ui.error, '');
      assert.equal(harness.ui.notice, '');
    } else assert.match(harness.ui.error, /unsupported document type/);
  }
});

test('GUI-17: a failure after switching conversations is not reported into the new one', async () => {
  const pick = deferred<Attachment | null>();
  const harness = attachHarness(() => pick.promise);
  const reading = harness.attach();
  harness.conversationRequest.current++;
  pick.reject(new Error('unsupported document type'));
  await reading;
  assert.equal(harness.ui.error, '');
  assert.deepEqual(harness.ui.attachments, []);
});

test('GUI-17: consecutive reads append in order and use the latest attachment list', async () => {
  let pick = deferred<Attachment | null>();
  const harness = attachHarness(() => pick.promise, 25);
  const first = harness.attach();
  // 第一个文件占 20 个字符，25 的上限只剩 5 个位置给第二个。
  pick.resolve(attachment('one.pdf', 'a'.repeat(20)));
  await first;
  pick = deferred<Attachment | null>();
  const second = harness.attach();
  pick.resolve(attachment('two.pdf', 'b'.repeat(10)));
  await second;
  assert.deepEqual(
    harness.ui.attachments.map((item) => item.name),
    ['one.pdf', 'two.pdf'],
  );
  // 只剩 5 个位置：截断而不是整份丢弃。
  assert.equal(harness.ui.attachments[1].text, 'bbbbb');
  assert.equal(harness.ui.attachments[1].truncated, true);

  pick = deferred<Attachment | null>();
  const third = harness.attach();
  pick.resolve(attachment('three.pdf', 'cc'));
  await third;
  assert.match(harness.ui.error, /at their limit/i);
  assert.equal(harness.ui.attachments.length, 2);
});

test('GUI-17: sending stays blocked while a document is still being read', async () => {
  // 只提供 guard 之前用到的上下文：如果 guard 失效、继续走发送路径，就会 ReferenceError。
  const send = compileFunction<(next?: string) => Promise<void>>('send', {
    input: 'question',
    attachments: [],
    t: (_zh: string, en: string) => en,
    runningRef: { current: false },
    ready: true,
    attaching: true,
  });
  await send();
});

// ---------------------------------------------------------------------------
// GUI-18：发送确认与拒绝事件按轮次处理
// ---------------------------------------------------------------------------

function sendHarness(options: { typed?: string; attachments?: Attachment[] } = {}) {
  const typed = options.typed ?? 'hello';
  const initialAttachments = options.attachments ?? [];
  const ui = {
    draft: typed,
    attachments: initialAttachments as Attachment[],
    messages: [] as Message[],
    error: '',
    usage: '',
    usagePending: false,
    contextUsage: null as Record<string, unknown> | null,
    docsIndex: null as AgentEvent | null,
  };
  const refs = {
    running: { current: false },
    outgoing: { current: null as PendingTurn<Attachment> | null },
    turnSeq: { current: 0 },
    history: { current: [] as string[] },
    historyIndex: { current: null as number | null },
    historyDraft: { current: '' },
    streamed: { current: false },
    fresh: { current: false },
    session: { current: 'session' },
    grant: { current: null },
    attachmentsRef: { current: initialAttachments },
  };
  const updates = {
    setInput: (update: string | ((old: string) => string)) => {
      ui.draft = typeof update === 'function' ? update(ui.draft) : update;
    },
    setAttachments: (update: Attachment[] | ((old: Attachment[]) => Attachment[])) => {
      ui.attachments = typeof update === 'function' ? update(ui.attachments) : update;
      refs.attachmentsRef.current = ui.attachments;
    },
    setMessages: (update: Message[] | ((old: Message[]) => Message[])) => {
      ui.messages = typeof update === 'function' ? update(ui.messages) : update;
    },
    setError: (value: string) => {
      ui.error = value;
    },
    setUsage: (value: string) => {
      ui.usage = value;
    },
    setSessionUsage: () => {},
    setUsagePending: (value: boolean) => {
      ui.usagePending = value;
    },
    setContextUsage: (value: Record<string, unknown> | null) => {
      ui.contextUsage = value;
    },
    setLimitReached: () => {},
    setConsent: () => {},
    setApproval: () => {},
    setDocsIndex: (value: AgentEvent) => {
      ui.docsIndex = value;
    },
  };
  const errorText = (message: string) => String(message).replace(/^Error: /, '');
  const t = (_zh: string, en: string) => en;
  const commands: string[] = [];
  const ack = deferred<void>();
  const invoke = (command: string) => {
    commands.push(command);
    if (command === 'assistant_send') return ack.promise;
    return Promise.resolve(undefined);
  };
  const common = {
    outgoing: refs.outgoing,
    turnSeq: refs.turnSeq,
    attachmentsRef: refs.attachmentsRef,
    newTurn,
    turnDelivered,
    turnFailed,
    turnOutput,
    turnPreTurnRejected,
    clearsDraftAfterAck,
    rejectedBeforeTurn,
    withoutLastUser,
    history: refs.history,
    historyIndex: refs.historyIndex,
    historyDraft: refs.historyDraft,
    errorText,
    t,
    setInput: updates.setInput,
    setAttachments: updates.setAttachments,
    setMessages: updates.setMessages,
    setError: updates.setError,
  };
  const beginSendTurn = compileFunction<(typed: string, prompt: string) => PendingTurn<Attachment>>(
    'beginSendTurn',
    common,
  );
  const settleSendTurn = compileFunction<
    (id: number, delivered: boolean, failure?: unknown) => void
  >('settleSendTurn', common);
  const handlerSource = variables.get('handler');
  assert.ok(handlerSource, 'AssistantPanel.tsx defines the assistant event handler');
  const handleEvent = compileExpression<(event: Record<string, unknown>) => void>(handlerSource, {
    active: true,
    outgoing: refs.outgoing,
    turnOutput,
    rejectedBeforeTurn,
    turnPreTurnRejected,
    withoutLastUser,
    setMessages: updates.setMessages,
    setInput: updates.setInput,
    setAttachments: updates.setAttachments,
    setError: updates.setError,
    errorText,
    t,
    app: { current: { assistantSettingsOpen: false } },
    busy: (value: boolean) => {
      refs.running.current = value;
    },
    grant: refs.grant,
    setApproval: updates.setApproval,
    setUsage: updates.setUsage,
    setSessionUsage: updates.setSessionUsage,
    setUsagePending: updates.setUsagePending,
    setContextUsage: updates.setContextUsage,
    setDocsIndex: updates.setDocsIndex,
  });
  const send = compileFunction<(next?: string) => Promise<void>>('send', {
    input: typed,
    attachments: initialAttachments,
    t,
    runningRef: refs.running,
    ready: true,
    attaching: false,
    settingsRef: {
      current: {
        ...defaultSettings,
        web_search: false,
        egress_acknowledged: 'https://api.deepseek.com',
      },
    },
    egressTarget,
    setConsent: updates.setConsent,
    busy: (value: boolean) => {
      refs.running.current = value;
    },
    setError: updates.setError,
    setLimitReached: updates.setLimitReached,
    app: {
      current: {
        root: '/project',
        selectedCase: { entry: { dir: '/case' } },
        versionRef: { id: 'official', revision: 1 },
      },
    },
    currentApprovalSnapshot: () => 'snapshot',
    invoke,
    guideState: () => ({ kernelDir: '/kernel' }),
    fresh: refs.fresh,
    session: refs.session,
    streamed: refs.streamed,
    setMessages: updates.setMessages,
    beginSendTurn,
    settleSendTurn,
    errorText,
  });
  return {
    ui,
    refs,
    commands,
    send,
    handleEvent,
    beginSendTurn,
    settleSendTurn,
    resolveAck: () => ack.resolve(undefined),
    rejectAck: (error: unknown) => ack.reject(error),
  };
}

const rejection = { type: 'error', message: 'no API key is stored for this service; set it' };

test('GUI-18: rejection first, IPC ack later — the restored draft is not cleared again', async () => {
  const paper = attachment('paper.pdf');
  const harness = sendHarness({ attachments: [paper] });
  const sending = harness.send();
  await settle();
  assert.deepEqual(harness.commands, ['assistant_start', 'assistant_send']);
  assert.equal(harness.ui.messages.length, 1, 'user bubble is shown before the turn starts');

  harness.handleEvent(rejection);
  assert.equal(harness.ui.messages.length, 0, 'rejected turn withdraws its bubble');
  assert.equal(harness.ui.draft, 'hello');
  assert.deepEqual(harness.ui.attachments, [paper]);
  assert.equal(harness.refs.running.current, false);

  harness.resolveAck();
  await sending;
  assert.equal(harness.ui.draft, 'hello', 'the late IPC ack must not clear the restored draft');
  assert.deepEqual(harness.ui.attachments, [paper]);
  assert.deepEqual(
    harness.refs.history.current,
    [],
    'a rejected message is not recorded in history',
  );
  assert.equal(harness.refs.outgoing.current?.phase, 'rejected');
});

test('GUI-18: IPC ack first, rejection later — draft and attachments come back', async () => {
  const paper = attachment('paper.pdf');
  const harness = sendHarness({ attachments: [paper] });
  const sending = harness.send();
  await settle();
  harness.resolveAck();
  await sending;
  assert.equal(harness.ui.draft, '');
  assert.deepEqual(harness.ui.attachments, []);
  assert.deepEqual(harness.refs.history.current, ['hello']);

  harness.handleEvent({
    type: 'error',
    message: 'session limit reached: this conversation is full',
  });
  assert.equal(harness.ui.messages.length, 0, 'the bubble is withdrawn once the rejection arrives');
  assert.equal(harness.ui.draft, 'hello');
  assert.deepEqual(harness.ui.attachments, [paper]);
});

test('GUI-18: text typed while waiting is never overwritten by an older turn', async () => {
  const harness = sendHarness();
  const sending = harness.send();
  await settle();
  harness.ui.draft = 'a follow-up question';
  harness.handleEvent({ type: 'error', message: 'still answering the previous message' });
  assert.equal(harness.ui.draft, 'a follow-up question');
  harness.resolveAck();
  await sending;
  assert.equal(harness.ui.draft, 'a follow-up question');
});

test('GUI-18: a late ack from an old turn cannot clear the next turn draft', async () => {
  const harness = sendHarness();
  const sending = harness.send();
  await settle();
  harness.handleEvent(rejection);
  assert.equal(harness.ui.draft, 'hello');
  const next = harness.beginSendTurn('next question', 'next question');
  harness.ui.draft = 'next question';
  harness.resolveAck();
  await sending;
  assert.equal(harness.ui.draft, 'next question');
  assert.deepEqual(harness.refs.history.current, []);
  assert.equal(harness.refs.outgoing.current?.id, next.id);
});

test('GUI-18: a turn the backend already started is not restored by a late refusal text', async () => {
  const harness = sendHarness();
  const sending = harness.send();
  await settle();
  // 后端产出了这一轮自己的事件：消息确实被记下了，IPC 成功与否不再是接受依据。
  harness.handleEvent({ type: 'usage', prompt_tokens: 3, completion_tokens: 5 });
  assert.equal(harness.refs.outgoing.current?.phase, 'accepted');
  harness.ui.draft = 'next question';
  harness.resolveAck();
  await sending;
  assert.equal(harness.ui.draft, 'next question');

  harness.handleEvent({ type: 'error', message: 'no API key is stored for this service; set it' });
  assert.equal(harness.ui.draft, 'next question', 'accepted turns never overwrite the next draft');
  assert.equal(
    harness.ui.messages.filter((item) => item.kind === 'user').length,
    1,
    'the accepted bubble is not withdrawn or duplicated',
  );
});

test('GUI-18: a generic error is not treated as a pre-turn rejection', async () => {
  const harness = sendHarness();
  const sending = harness.send();
  await settle();
  harness.handleEvent({ type: 'error', message: 'HTTP 500 from the model' });
  assert.equal(harness.refs.outgoing.current?.phase, 'preparing');
  assert.equal(harness.ui.messages.length, 1);
  harness.resolveAck();
  await sending;
  assert.equal(harness.ui.draft, '');
});

test('GUI-18: an IPC failure withdraws its own bubble, never an earlier one', async () => {
  const harness = sendHarness();
  harness.rejectAck(new Error('assistant process died'));
  const sending = harness.send();
  await settle();
  await sending;
  assert.equal(harness.ui.messages.length, 0);
  assert.equal(harness.ui.draft, 'hello');
  assert.match(harness.ui.error, /assistant process died/);
  assert.equal(harness.refs.running.current, false);
});

function sessionHarness(loadTranscript: () => Promise<Message[]>) {
  const state = {
    messages: [] as Message[],
    contextUsage: null as unknown,
    limitReached: true,
    stepLimit: 3,
    sessions: [{ id: 'other' }] as unknown,
    attachNotice: 'stale notice',
    usage: '5 + 5 tokens',
    sessionUsage: { turns: 5, tokens: 10 },
    usagePending: false,
    followed: 0,
  };
  const refs = {
    session: { current: null as string | null },
    fresh: { current: true },
    running: { current: false },
    conversationRequest: { current: 0 },
  };
  const open = compileFunction<(item: { id: string }) => Promise<void>>('openSession', {
    conversationRequest: refs.conversationRequest,
    runningRef: refs.running,
    invokeJson: loadTranscript,
    session: refs.session,
    fresh: refs.fresh,
    setMessages: (value: Message[]) => {
      state.messages = value;
    },
    setContextUsage: (value: unknown) => {
      state.contextUsage = value;
    },
    setLimitReached: (value: boolean) => {
      state.limitReached = value;
    },
    setStepLimit: (value: number) => {
      state.stepLimit = value;
    },
    setSessions: (value: unknown) => {
      state.sessions = value;
    },
    setAttachNotice: (value: string) => {
      state.attachNotice = value;
    },
    setUsage: (value: string) => {
      state.usage = value;
    },
    setSessionUsage: (value: { turns: number; tokens: number }) => {
      state.sessionUsage = value;
    },
    setUsagePending: (value: boolean) => {
      state.usagePending = value;
    },
    resetFollow: () => {
      state.followed++;
    },
  });
  return { state, refs, open };
}

test('U-07: switching to a stored conversation clears its usage until that conversation reports it', async () => {
  const transcript = deferred<Message[]>();
  const harness = sessionHarness(() => transcript.promise);
  const opening = harness.open({ id: 'saved-session' });
  transcript.resolve([{ kind: 'user', text: 'old question' }]);
  await opening;
  assert.equal(harness.state.messages.length, 1, 'transcript is shown');
  assert.equal(harness.refs.session.current, 'saved-session');
  assert.equal(harness.refs.fresh.current, false);
  assert.equal(harness.state.usage, '', 'the previous conversation usage must not be reused');
  assert.deepEqual(harness.state.sessionUsage, { turns: 0, tokens: 0 });
  assert.equal(harness.state.usagePending, true, 'usage waits for this conversation');
  assert.equal(harness.state.attachNotice, '');
  assert.equal(harness.state.followed, 1, 'the new conversation starts following the bottom');

  // 这一会话自己的 usage 事件到达后，"待加载"结束，显示的是它的数字。
  const panel = sendHarness();
  panel.ui.usagePending = true;
  panel.handleEvent({ type: 'usage', prompt_tokens: 7, completion_tokens: 3, session_turns: 2 });
  assert.equal(panel.ui.usagePending, false);
  assert.equal(panel.ui.usage, '7 + 3 tokens');
});

test('U-07: a late transcript for a conversation the user already left is ignored', async () => {
  const transcript = deferred<Message[]>();
  const harness = sessionHarness(() => transcript.promise);
  const opening = harness.open({ id: 'old-session' });
  // 用户紧接着新建对话/打开别的会话：代次前进，旧 transcript 不能写入。
  harness.refs.conversationRequest.current++;
  transcript.resolve([{ kind: 'user', text: 'late' }]);
  await opening;
  assert.deepEqual(harness.state.messages, []);
  assert.equal(harness.state.usagePending, false);
  assert.equal(harness.state.usage, '5 + 5 tokens');
});

test('U-07: the panel renders the pending-usage state and keeps markdown for assistant replies only', () => {
  assert.match(panelSource, /用量待加载/);
  assert.match(panelSource, /item\.kind === 'assistant' \? \(\s*<Markdown/);
  assert.match(panelSource, /<Markdown text=\{item\.text \|\| ''\} t=\{t\} \/>/);
});

test('the panel really sends through the extracted turn functions', () => {
  const sendBody = functions.get('send');
  assert.ok(sendBody, 'AssistantPanel.tsx defines send');
  assert.match(sendBody, /beginSendTurn\(/);
  assert.match(sendBody, /settleSendTurn\(/);
  assert.ok(!sendBody.includes('pendingSend'), 'the old single pendingSend ref is gone');
});

test('the context event carries the dynamic window, available input and the estimate label', () => {
  const harness = sendHarness();
  harness.handleEvent({
    type: 'context',
    estimated_tokens: 12_000,
    window_tokens: 128_000,
    compressed_tools: 2,
    available_input_tokens: 100_000,
    output_reserve_tokens: 8_192,
    safety_margin_tokens: 2_560,
    window_source: 'user',
  });
  assert.deepEqual(harness.ui.contextUsage, {
    tokens: 12_000,
    window: 128_000,
    compressed: 2,
    available: 100_000,
    outputReserve: 8_192,
    safety: 2_560,
    source: 'user',
  });
  // 旧 agent 事件没有新字段：可用预算按“未知”处理，不能伪装成 0 或估计成 1M。
  harness.handleEvent({ type: 'context', estimated_tokens: 1, window_tokens: 1_000 });
  assert.deepEqual(harness.ui.contextUsage, {
    tokens: 1,
    window: 1_000,
    compressed: 0,
    available: null,
    outputReserve: 0,
    safety: 0,
    source: null,
  });
});

test('the docs_index_status event keeps the application-level Xerj index state', async () => {
  const harness = sendHarness();
  const event = {
    type: 'docs_index_status',
    backend: 'xerj',
    enabled: true,
    state: 'ready',
    scope: { kind: 'app', name: 'docs' },
    endpoint: 'http://127.0.0.1:9200',
    index: 'colm-docs-v1',
    documents: 12,
    chunks: 340,
    generation: 1_730_000_000_000,
    indexed_at_unix_ms: 1_730_000_000_123,
    detail: 'published generation 1730000000000',
  };
  harness.handleEvent(event);
  assert.deepEqual(harness.ui.docsIndex, event);
  // 应用级设置：索引状态不属于某个对话，设置对话框直接从面板拿最近一次状态。
  assert.match(panelSource, /case 'docs_index_status':/);
  assert.match(panelSource, /docsIndex=\{docsIndex\}/);
  // 未知 state 不能被页面丢掉（后端升级后前端仍要原样显示）。
  harness.handleEvent({ type: 'docs_index_status', state: 'future_state' });
  assert.equal(harness.ui.docsIndex?.state, 'future_state');

  // 后台索引事件不是"模型已接下这一轮"的证据：它不能阻止拒绝时撤回气泡、恢复草稿。
  const pending = sendHarness();
  const sending = pending.send();
  await settle();
  pending.handleEvent({ type: 'docs_index_status', state: 'building' });
  assert.equal(pending.refs.outgoing.current?.phase, 'preparing');
  pending.resolveAck();
  await sending;
});

test('a leftover built-in context estimate is never shown for an external backend', () => {
  // 外部后端自己管上下文：切过去后不显示上一次 API 请求的占用；窗口和可用输入都来自事件而不是常量。
  assert.match(panelSource, /const contextWarning =\s*builtin &&/);
  assert.match(panelSource, /\{builtin &&\s*contextUsage &&/);
  assert.match(panelSource, /next\.backend !== 'builtin' \|\|/);
  assert.match(panelSource, /可用输入估算/);
  assert.match(panelSource, /documented preset/);
});

test('budget errors tell the user how to set a window or shrink the request without claiming anything was sent', () => {
  const errorText = compileFunction<(message: string) => string>('errorText', {
    t: (_zh: string, en: string) => en,
  });
  assert.match(
    errorText(
      'the context window for this model endpoint is unknown; set the context window (in tokens)',
    ),
    /set the context window/i,
  );
  assert.match(
    errorText(
      'the request needs about 120000 estimated input tokens plus 8192 reserved output tokens and 2560 safety tokens, which exceeds the 128000-token context window',
    ),
    /nothing was sent/i,
  );
  assert.equal(errorText('HTTP 500 from the model'), 'HTTP 500 from the model');
});

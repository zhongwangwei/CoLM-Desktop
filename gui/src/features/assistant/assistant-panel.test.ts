import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { withTimeout } from '../../lib/ipc';
import {
  appendDelta,
  defaultSettings,
  egressTarget,
  historySettingsOf,
  pendingReconciliation,
  rejectedBeforeTurn,
  taskSnapshotOf,
  withoutLastUser,
  type AgentEvent,
  type AgentTaskSnapshot,
  type Message,
  type Settings,
} from './model';
import {
  clearsDraftAfterAck,
  eventBelongsToTurn,
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
    sendEpoch: { current: 0 },
    startupId: { current: null as string | null },
    sessionStart: { current: null },
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
    deletingSessionRef: { current: false },
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
    deletingSessionRef: { current: false },
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

function sendHarness(
  options: {
    typed?: string;
    attachments?: Attachment[];
    start?: Promise<void>;
    manualReady?: boolean;
    newSession?: Promise<void>;
    cancelError?: string;
    versionRef?: { id: string; revision: number };
  } = {},
) {
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
    taskSnapshot: null as AgentTaskSnapshot | null,
    reconciling: [] as boolean[],
  };
  const refs = {
    running: { current: false },
    outgoing: { current: null as PendingTurn<Attachment> | null },
    sendEpoch: { current: 0 },
    startupId: { current: null as string | null },
    reviewEpoch: { current: null as number | null },
    sessionStart: {
      current: null as {
        id: string;
        onReady?: () => void;
        target: string | null;
        skipConfigure: boolean;
        ready?: boolean;
        onSnapshot?: (task: AgentTaskSnapshot | null) => void;
        onError?: (error: Error) => void;
      } | null,
    },
    turnSeq: { current: 0 },
    history: { current: [] as string[] },
    historyIndex: { current: null as number | null },
    historyDraft: { current: '' },
    streamed: { current: false },
    fresh: { current: false },
    session: { current: 'session' as string | null },
    agentSession: { current: 'session' },
    grant: { current: null },
    attachmentsRef: { current: initialAttachments },
    settingsRef: {
      current: {
        ...defaultSettings,
        web_search: false,
        egress_acknowledged: 'https://api.deepseek.com',
      } as Settings,
    },
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
    setModel: () => {},
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
  const invocations: { command: string; args: Record<string, unknown> }[] = [];
  const ack = deferred<void>();
  const invoke = (command: string, args: Record<string, unknown> = {}) => {
    commands.push(command);
    invocations.push({ command, args });
    if (command === 'assistant_start' && options.start) return options.start;
    if (command === 'assistant_new_session' && options.newSession) return options.newSession;
    if (command === 'assistant_cancel' && options.cancelError)
      return Promise.reject(options.cancelError);
    if (
      command === 'assistant_start' &&
      !options.manualReady &&
      !options.start &&
      !options.newSession
    )
      queueMicrotask(() => {
        handleEvent({
          type: 'ready',
          session: refs.session.current || 'initial',
          startup_id: args.startupId,
        });
        if (args.resume)
          handleEvent({ type: 'ready', session: args.resume, startup_id: args.startupId });
      });
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
  const beginSendTurn = compileFunction<
    (typed: string, prompt: string, requestId: string) => PendingTurn<Attachment>
  >('beginSendTurn', common);
  const settleSendTurn = compileFunction<
    (id: number, delivered: boolean, failure?: unknown) => void
  >('settleSendTurn', common);
  const handlerSource = variables.get('handler');
  assert.ok(handlerSource, 'AssistantPanel.tsx defines the assistant event handler');
  const handleEvent = compileExpression<(event: Record<string, unknown>) => void>(handlerSource, {
    active: true,
    startupId: refs.startupId,
    sendEpoch: refs.sendEpoch,
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
    app: { current: { assistantSettingsOpen: false, versionRef: options.versionRef } },
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
    setModel: updates.setModel,
    streamed: refs.streamed,
    appendDelta,
    setTaskSnapshot: (value: AgentTaskSnapshot | null) => {
      ui.taskSnapshot = value;
    },
    taskSnapshotOf,
    fresh: refs.fresh,
    session: refs.session,
    agentSession: refs.agentSession,
    runningRef: refs.running,
    eventBelongsToTurn,
    sessionStart: refs.sessionStart,
    clearTaskReview: () => {
      ui.taskSnapshot = null;
    },
  });
  let requestSequence = 0;
  const send = compileFunction<(next?: string) => Promise<void>>('send', {
    deletingSessionRef: { current: false },
    withTimeout,
    startupId: refs.startupId,
    input: typed,
    attachments: initialAttachments,
    t,
    runningRef: refs.running,
    sendEpoch: refs.sendEpoch,
    sessionStart: refs.sessionStart,
    ready: true,
    attaching: false,
    settingsRef: refs.settingsRef,
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
    historySettingsOf,
    newRequestId: () => `turn-test-${++requestSequence}`,
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
    invocations,
    stop: compileFunction<() => Promise<void>>('stop', {
      startupId: refs.startupId,
      reviewEpoch: refs.reviewEpoch,
      setReconciling: (value: boolean) => {
        ui.reconciling.push(value);
      },
      sendEpoch: refs.sendEpoch,
      sessionStart: refs.sessionStart,
      invoke,
      t,
      busy: (value: boolean) => {
        refs.running.current = value;
      },
    }),
    send,
    handleEvent: (event: Record<string, unknown>) =>
      handleEvent({
        ...(event.type === 'ready' ? { startup_id: refs.startupId.current } : {}),
        ...event,
      }),
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
  const next = harness.beginSendTurn('next question', 'next question', 'turn-next-1');
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
  // A3：后端发来匹配本轮 requestId 的 turn_started，消息确实被记下了；IPC 成功与否不再是接受依据。
  const requestId = String(
    harness.invocations.find((item) => item.command === 'assistant_send')?.args.requestId,
  );
  harness.handleEvent({ type: 'turn_started', request_id: requestId });
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

function sessionHarness(
  loadTranscript: () => Promise<Message[]>,
  draft = { input: '', attachments: [] as Attachment[], discard: true },
) {
  const state = {
    input: draft.input,
    attachments: draft.attachments,
    messages: [] as Message[],
    contextUsage: null as unknown,
    limitReached: true,
    stepLimit: 3,
    sessions: [{ id: 'other' }] as unknown,
    attachNotice: 'stale notice',
    taskSnapshot: { requires_reconciliation: true } as AgentTaskSnapshot | null,
    reconcileNote: 'old review',
    reconcileNotice: 'old notice',
    reconcileError: 'old error',
    usage: '5 + 5 tokens',
    sessionUsage: { turns: 5, tokens: 10 },
    usagePending: false,
    followed: 0,
  };
  const refs = {
    deletingSession: { current: false },
    input: { current: draft.input },
    attachments: { current: draft.attachments },
    historyIndex: { current: 3 as number | null },
    session: { current: null as string | null },
    fresh: { current: true },
    running: { current: false },
    conversationRequest: { current: 0 },
  };
  const open = compileFunction<(item: { id: string }) => Promise<void>>('openSession', {
    deletingSessionRef: refs.deletingSession,
    attaching: false,
    confirmConversationChange: async () => draft.discard,
    inputRef: refs.input,
    attachmentsRef: refs.attachments,
    setInput: (value: string) => {
      state.input = value;
    },
    setAttachments: (value: Attachment[]) => {
      state.attachments = value;
    },
    historyIndex: refs.historyIndex,
    conversationRequest: refs.conversationRequest,
    runningRef: refs.running,
    sessionStart: { current: null },
    t: (_zh: string, en: string) => en,
    taskSnapshotOf,
    setTaskSnapshot: (value: AgentTaskSnapshot | null) => {
      state.taskSnapshot = value;
    },
    invokeJson: loadTranscript,
    session: refs.session,
    fresh: refs.fresh,
    clearTaskReview: compileFunction<() => void>('clearTaskReview', {
      setReconciling: () => {},
      setTaskSnapshot: (value: AgentTaskSnapshot | null) => {
        state.taskSnapshot = value;
      },
      setReconcileNote: (value: string) => {
        state.reconcileNote = value;
      },
      setReconcileNotice: (value: string) => {
        state.reconcileNotice = value;
      },
      setReconcileError: (value: string) => {
        state.reconcileError = value;
      },
    }),
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
  assert.equal(harness.state.taskSnapshot, null);
  assert.equal(harness.state.reconcileNote, '');
  assert.equal(harness.state.reconcileNotice, '');
  assert.equal(harness.state.reconcileError, '');
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
  assert.match(
    panelSource,
    /<Markdown[\s\S]*?onOpenUrl=\{\(url\) => void openEvidenceUrl\(url\)\}/,
  );
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
    /blocked model request was not sent/i,
  );
  // 被拦住的是这一次模型请求，不能暗示本轮更早的请求也没发出去。
  assert.match(
    errorText(
      'the request needs about 120000 estimated input tokens plus 8192 reserved output tokens and 2560 safety tokens, which exceeds the 128000-token context window',
    ),
    /earlier requests in this turn may already have been sent/i,
  );
  // 中文文案同样限定"被拦住的这次"，不能暗示本轮更早请求没发。
  assert.match(panelSource, /被拦住的这次模型请求没有发往模型服务（本轮较早的请求可能已经发送）/);
  assert.equal(errorText('HTTP 500 from the model'), 'HTTP 500 from the model');
});

// ---------------------------------------------------------------------------
// U02：聊天范围提示/选择器（持久化 + 每条消息的 history_scope）
// ---------------------------------------------------------------------------

function scopeHarness(options: { saved?: Settings; reject?: Error } = {}) {
  const saved = options.saved ?? defaultSettings;
  const state = {
    settings: null as Settings | null,
    busy: [] as boolean[],
    error: '',
  };
  const requests: { command: string; args: Record<string, unknown> }[] = [];
  const invoke = async (command: string, args: Record<string, unknown> = {}) => {
    requests.push({ command, args });
    if (options.reject) throw options.reject;
    return undefined;
  };
  const refs = { running: { current: false }, settingsRef: { current: saved } };
  const app = { current: { assistantSettingsOpen: false } };
  const changeHistoryScope = compileFunction<
    (scope: 'auto' | 'selected' | 'desktop' | 'upstream' | 'both') => Promise<void>
  >('changeHistoryScope', {
    scopeBusy: false,
    runningRef: refs.running,
    app,
    settingsRef: refs.settingsRef,
    setScopeBusy: (value: boolean) => {
      state.busy.push(value);
    },
    setError: (value: string) => {
      state.error = value;
    },
    errorText: (message: string) => String(message).replace(/^Error: /, ''),
    invoke,
    withHistoryScope: (settings: Settings, scope: string) => ({
      ...settings,
      history: { ...settings.history, scope },
    }),
    historySettingsOf: (settings: Partial<Settings>) => ({
      scope: settings.history?.scope ?? 'auto',
      desktop_path: settings.history?.desktop_path ?? null,
      upstream_path: settings.history?.upstream_path ?? null,
    }),
    setSettings: (value: Settings) => {
      state.settings = value;
    },
  });
  return {
    state,
    refs,
    app,
    requests,
    changeHistoryScope,
    savedSettings: () => refs.settingsRef.current,
  };
}

test('U02: changing the history scope persists it on top of the saved settings', async () => {
  const saved = {
    ...defaultSettings,
    history: { scope: 'auto' as const, desktop_path: '/repos/desktop', upstream_path: null },
  };
  const harness = scopeHarness({ saved });
  await harness.changeHistoryScope('both');
  assert.deepEqual(
    harness.requests.map((request) => request.command),
    ['assistant_save_settings'],
  );
  const sent = harness.requests[0].args.settings as Settings;
  assert.deepEqual(sent.history, {
    scope: 'both',
    desktop_path: '/repos/desktop',
    upstream_path: null,
  });
  // 其它设置（例如模型/后端）不能被范围切换改写。
  assert.equal(sent.model, saved.model);
  assert.equal(sent.backend, saved.backend);
  assert.deepEqual(harness.state.settings?.history.scope, 'both');
  assert.deepEqual(harness.state.busy, [true, false]);
});

test('U02: a failed scope save keeps the old scope and shows the error', async () => {
  const harness = scopeHarness({ reject: new Error('settings file is read-only') });
  await harness.changeHistoryScope('desktop');
  assert.equal(harness.state.settings, null, '失败时不更新界面选择');
  assert.match(harness.state.error, /read-only/);
  assert.deepEqual(harness.state.busy, [true, false]);
});

test('U02: scope changes are ignored while answering or while settings are open', async () => {
  const harness = scopeHarness();
  harness.refs.running.current = true;
  await harness.changeHistoryScope('desktop');
  assert.deepEqual(harness.requests, []);
  harness.refs.running.current = false;
  harness.app.current.assistantSettingsOpen = true;
  await harness.changeHistoryScope('desktop');
  assert.deepEqual(harness.requests, [], '设置对话框打开时不落盘，避免带走未保存草稿');
  harness.app.current.assistantSettingsOpen = false;
  await harness.changeHistoryScope('auto');
  assert.deepEqual(harness.requests, [], '没变化就不写设置');
});

test('U02: every assistant_send carries history_scope and assistant_start keeps modelRevision', async () => {
  const harness = sendHarness();
  const sending = harness.send();
  await settle();
  const start = harness.invocations.find((item) => item.command === 'assistant_start');
  assert.deepEqual(start?.args.modelRevision, { id: 'official', revision: 1 });
  const sent = harness.invocations.find((item) => item.command === 'assistant_send');
  const context = JSON.parse(String(sent?.args.context));
  assert.equal(context.history_scope, 'auto');
  assert.equal(context.kernelDir, '/kernel', '原有 guideState 内容保留');
  harness.resolveAck();
  await sending;

  // 用户选了 Desktop 范围：下一条消息立刻带上它（不必先重启助手）。
  harness.refs.settingsRef.current = {
    ...harness.refs.settingsRef.current,
    history: { scope: 'desktop', desktop_path: null, upstream_path: null },
  };
  // 上一轮靠 turn_done 才复位 running；这里直接复位等价状态。
  harness.refs.running.current = false;
  const second = harness.send('second question');
  await settle();
  const next = harness.invocations.filter((item) => item.command === 'assistant_send').at(-1);
  assert.equal(JSON.parse(String(next?.args.context)).history_scope, 'desktop');
  harness.resolveAck();
  await second;
});

test('U02: the panel shows the backend, model, frozen revision and the routing hint', () => {
  assert.match(panelSource, /historyScopeLabel\(scope, state\.language\)/);
  assert.match(panelSource, /historyContext\.backend/);
  assert.match(panelSource, /historyContext\.model/);
  assert.match(panelSource, /historyContext\.revision/);
  assert.match(panelSource, /historyScopeHint\(state\.language\)/);
  assert.match(panelSource, /historyContextLabel\(/);
  assert.match(panelSource, /assistant-history-scope/);
  assert.match(panelSource, /history_scope: historySettingsOf\(current\)\.scope/);
  assert.match(
    panelSource,
    /disabled=\{!ready \|\| running \|\| scopeBusy \|\| state\.assistantSettingsOpen\}/,
  );
  assert.match(panelSource, /正在保存范围/);
});

// ---------------------------------------------------------------------------
// U03/U04/U05：证据卡、未知副作用核对横幅、迟到事件与预算文案
// ---------------------------------------------------------------------------

test('U03: history tool results render an evidence card and keep the raw JSON fallback', () => {
  assert.match(panelSource, /historyEvidence\(item\.name, item\.result, state\.language\)/);
  assert.match(panelSource, /<EvidenceCardView/);
  assert.match(panelSource, /原始 JSON/);
  // 失败/非 JSON 仍走原来的原始 JSON 分支（historyEvidence 返回 null）。
  assert.match(panelSource, /item\.ok && item\.result != null/);
});

function openHarness() {
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  const state = { error: '' };
  const context = {
    invoke: async (command: string, args: Record<string, unknown> = {}) => {
      calls.push({ command, args });
    },
    setError: (value: string) => {
      state.error = value;
    },
    errorText: (message: string) => String(message).replace(/^Error: /, ''),
    t: (_zh: string, en: string) => en,
  };
  return {
    calls,
    state,
    openEvidencePath: compileFunction<(path: string) => Promise<void>>('openEvidencePath', context),
    openEvidenceUrl: compileFunction<(url: string) => Promise<void>>('openEvidenceUrl', context),
  };
}

test('U03: only open_path for local files and assistant_open_url for http(s) links', async () => {
  const harness = openHarness();
  await harness.openEvidencePath('/repos/desktop/src/a.F90');
  assert.deepEqual(harness.calls.at(-1), {
    command: 'open_path',
    args: { path: '/repos/desktop/src/a.F90' },
  });
  await harness.openEvidenceUrl('https://github.com/o/r/pull/7');
  assert.deepEqual(harness.calls.at(-1), {
    command: 'assistant_open_url',
    args: { url: 'https://github.com/o/r/pull/7' },
  });
  // A2：规范仓库的完整 commit 链接走同一个命令，参数原样。
  const commitUrl = `https://github.com/zhongwangwei/CoLM-Desktop/commit/${'b'.repeat(40)}`;
  await harness.openEvidenceUrl(commitUrl);
  assert.deepEqual(harness.calls.at(-1), {
    command: 'assistant_open_url',
    args: { url: commitUrl },
  });
  // 非 http(s) 链接不发给后端，也不拼成文件路径。
  await harness.openEvidenceUrl('file:///etc/passwd');
  assert.match(harness.state.error, /http\/https/i);
  assert.equal(harness.calls.length, 3);
});

function reconcileHarness(
  options: { reject?: Error; note?: string; response?: Promise<void>; start?: Promise<void> } = {},
) {
  const panel = sendHarness();
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  const state = {
    reconciling: panel.ui.reconciling,
    error: '',
    notice: '',
  };
  const invoke = async (command: string, args: Record<string, unknown> = {}) => {
    calls.push({ command, args });
    if (command === 'assistant_start' && options.start) await options.start;
    if (command === 'assistant_reconcile_effects') {
      if (options.response) await options.response;
      if (options.reject) throw options.reject;
    }
  };
  const conversationRequest = { current: 0 };
  const resumeReviewSession = compileFunction<
    (target: string) => Promise<AgentTaskSnapshot | null>
  >('resumeReviewSession', {
    startupId: panel.refs.startupId,
    newRequestId: () => 'review-start',
    sessionStart: panel.refs.sessionStart,
    app: { current: { root: '/project', selectedCase: null, versionRef: null } },
    guideState: () => ({ kernelDir: '/kernel' }),
    invoke,
    withTimeout,
    t: (_zh: string, en: string) => en,
  });
  const reconcileEffects = compileFunction<(ids: string[]) => Promise<void>>('reconcileEffects', {
    conversationRequest,
    reviewEpoch: panel.refs.reviewEpoch,
    runningRef: panel.refs.running,
    sessionStart: panel.refs.sessionStart,
    session: panel.refs.session,
    agentSession: panel.refs.agentSession,
    sendEpoch: panel.refs.sendEpoch,
    busy: (value: boolean) => {
      panel.refs.running.current = value;
    },
    resumeReviewSession,
    taskSnapshot: taskSnapshotOf({
      requires_reconciliation: true,
      effect_unknown_actions: [{ id: 'old-1' }],
    }),
    pendingReconciliation,
    reconciling: false,
    reconcileNote: options.note ?? '',
    setReconciling: (value: boolean) => {
      state.reconciling.push(value);
    },
    setReconcileError: (value: string) => {
      state.error = value;
    },
    setReconcileNotice: (value: string) => {
      state.notice = value;
    },
    invoke,
    t: (_zh: string, en: string) => en,
    errorText: (message: string) => String(message).replace(/^Error: /, ''),
  });
  return { calls, state, reconcileEffects, conversationRequest, panel };
}

test('U04: reconcile submits only the listed old action ids and keeps the banner on failure', async () => {
  const ok = reconcileHarness({ note: 'checked run status' });
  await ok.reconcileEffects(['old-1', 'old-2']);
  assert.deepEqual(ok.calls.at(-1), {
    command: 'assistant_reconcile_effects',
    args: { ids: ['old-1', 'old-2'], note: 'checked run status' },
  });
  assert.match(ok.state.notice, /submitted/i);
  assert.deepEqual(ok.state.reconciling, [true, false]);

  const failed = reconcileHarness({ reject: new Error('reconcile transport failed') });
  await failed.reconcileEffects(['old-1']);
  assert.match(failed.state.error, /transport failed/);
  assert.equal(failed.state.notice, '', '失败不能显示成功提示');
  assert.deepEqual(failed.state.reconciling, [true, false]);
});

test('U04: banner comes from the task snapshot and normal turn events cannot clear it', async () => {
  const harness = sendHarness();
  const sending = harness.send();
  await settle();
  harness.handleEvent({ type: 'ready', session: 'session' });
  harness.handleEvent({ type: 'ready', session: 'session' });
  harness.handleEvent({ type: 'task_state', task: { requires_reconciliation: true } });
  assert.equal(harness.ui.taskSnapshot?.requires_reconciliation, true);
  // 普通 tool_result / assistant_delta / 新消息都不改任务快照。
  harness.handleEvent({ type: 'tool_result', id: 'call-1', ok: true, result: '{}' });
  harness.handleEvent({ type: 'assistant_delta', text: 'still working' });
  assert.equal(harness.ui.taskSnapshot?.requires_reconciliation, true);
  // 只有新的 task_state 明确不再要求核对才隐藏。
  harness.handleEvent({ type: 'task_state', task: { requires_reconciliation: false } });
  assert.equal(harness.ui.taskSnapshot?.requires_reconciliation, false);
  harness.resolveAck();
  await sending;
  assert.match(panelSource, /review\.required &&/);
  assert.match(panelSource, /<TaskReviewBanner/);
  assert.match(panelSource, /pendingReconciliation\(taskSnapshot\)/);
  assert.match(panelSource, /setTaskSnapshot\(taskSnapshotOf\(event\.task\)\)/);
  assert.match(panelSource, /reconcileEffects\(ids\)/);
});

test('A3: only a matching turn_started accepts the message; late events keep it pending', async () => {
  const harness = sendHarness();
  const sending = harness.send();
  await settle();
  const sentTurn = harness.refs.outgoing.current!;
  const requestId = String(
    harness.invocations.find((item) => item.command === 'assistant_send')?.args.requestId,
  );
  assert.match(requestId, /^turn-/, 'assistant_send 携带本轮 requestId');
  assert.equal(sentTurn.requestId, requestId, 'pending 状态记录同一个 requestId');

  // 迟到的 task_state / usage / context / tool_result 都不再标记“已被接受”。
  harness.handleEvent({ type: 'task_state', task: { requires_reconciliation: false } });
  harness.handleEvent({ type: 'usage', prompt_tokens: 1, completion_tokens: 2 });
  harness.handleEvent({ type: 'context', estimated_tokens: 1, window_tokens: 100 });
  harness.handleEvent({ type: 'tool_result', id: 'call-1', ok: true, result: '{}' });
  assert.equal(harness.refs.outgoing.current?.phase, 'preparing');
  // 旧轮/伪造的 turn_started 不匹配，也不接受。
  harness.handleEvent({ type: 'turn_started', request_id: 'turn-0-old' });
  assert.equal(harness.refs.outgoing.current?.phase, 'preparing');
  // 匹配本轮 requestId 的 turn_started 才算接受。
  harness.handleEvent({ type: 'turn_started', request_id: requestId });
  assert.equal(harness.refs.outgoing.current?.phase, 'accepted');
  harness.resolveAck();
  await sending;
});

for (const phase of ['start', 'newSession'] as const) {
  test(`stop during assistant ${phase} prevents the pending message from being sent`, async () => {
    const pending = deferred<void>();
    const harness = sendHarness({ [phase]: pending.promise });
    harness.refs.fresh.current = phase === 'newSession';
    const sending = harness.send();
    await settle();
    await harness.stop();
    pending.resolve();
    await sending;
    assert.equal(harness.refs.sessionStart.current, null);
    assert.equal(harness.commands.includes('assistant_send'), false);
    assert.equal(harness.ui.messages.length, 0);
    assert.equal(harness.ui.draft, 'hello');
    assert.equal(harness.refs.running.current, false);
  });
}

test('a late task snapshot cannot enter a different or new conversation', () => {
  const harness = sendHarness();
  harness.refs.session.current = 'other-conversation';
  harness.handleEvent({ type: 'task_state', task: { requires_reconciliation: true } });
  assert.equal(harness.ui.taskSnapshot, null);
  harness.refs.session.current = 'session';
  harness.refs.fresh.current = true;
  harness.handleEvent({ type: 'task_state', task: { requires_reconciliation: true } });
  assert.equal(harness.ui.taskSnapshot, null);
});

for (const reject of [undefined, new Error('old failure')]) {
  test(`a reconciliation reply after switching conversations is ignored (${reject ? 'failure' : 'success'})`, async () => {
    const pending = deferred<void>();
    const harness = reconcileHarness({ response: pending.promise, reject });
    const reviewing = harness.reconcileEffects(['old-action']);
    harness.conversationRequest.current++;
    pending.resolve();
    await reviewing;
    assert.equal(harness.state.error, '');
    assert.equal(harness.state.notice, '');
    assert.deepEqual(harness.state.reconciling, [true]);
  });
}

for (const restoredTask of [false, true]) {
  for (const eventsFirst of [false, true]) {
    test(`configure snapshots cannot replace the requested resume session (task=${restoredTask}, eventsFirst=${eventsFirst})`, async () => {
      const start = deferred<void>();
      const harness = sendHarness({ start: start.promise });
      harness.refs.session.current = 'target-B';
      const sending = harness.send();
      if (!eventsFirst) {
        start.resolve();
        await settle();
      }
      harness.handleEvent({ type: 'ready', session: 'old-A' });
      harness.handleEvent({
        type: 'task_state',
        task: { goal: 'old-A', requires_reconciliation: true },
      });
      assert.equal(harness.refs.session.current, 'target-B');
      assert.equal(harness.ui.taskSnapshot, null);
      harness.handleEvent({ type: 'ready', session: 'target-B' });
      if (restoredTask) {
        harness.handleEvent({
          type: 'task_state',
          task: { goal: 'target-B', requires_reconciliation: true },
        });
        assert.equal((harness.ui.taskSnapshot as AgentTaskSnapshot | null)?.goal, 'target-B');
      } else assert.equal(harness.ui.taskSnapshot, null);
      start.resolve();
      await settle();
      harness.resolveAck();
      await sending;
    });
  }
}
for (const eventsFirst of [false, true]) {
  test(`new session ignores configure snapshots regardless of IPC ordering (eventsFirst=${eventsFirst})`, async () => {
    const start = deferred<void>();
    const newSession = deferred<void>();
    const harness = sendHarness({ start: start.promise, newSession: newSession.promise });
    harness.refs.fresh.current = true;
    const sending = harness.send();
    if (!eventsFirst) {
      start.resolve();
      await settle();
      newSession.resolve();
      await settle();
    }
    harness.handleEvent({ type: 'ready', session: 'old-A' });
    harness.handleEvent({
      type: 'task_state',
      task: { goal: 'old-A', requires_reconciliation: true },
    });
    assert.equal(harness.ui.taskSnapshot, null);
    assert.equal(harness.refs.fresh.current, true);
    if (eventsFirst) {
      start.resolve();
      await settle();
    }
    harness.handleEvent({ type: 'ready', session: 'new-B' });
    harness.handleEvent({
      type: 'task_state',
      task: { goal: 'new-B', requires_reconciliation: true },
    });
    assert.equal(harness.refs.session.current, 'new-B');
    assert.equal(harness.refs.fresh.current, false);
    assert.equal((harness.ui.taskSnapshot as AgentTaskSnapshot | null)?.goal, 'new-B');
    newSession.resolve();
    await settle();
    harness.resolveAck();
    await sending;
    assert.equal(
      harness.invocations.find((item) => item.command === 'assistant_start')?.args.resume,
      null,
    );
    assert.equal(
      harness.commands.filter((command) => command === 'assistant_new_session').length,
      1,
    );
  });
}

test('initial configure accepts the initial session and clears the previous review', async () => {
  const harness = sendHarness();
  harness.refs.session.current = null;
  harness.ui.taskSnapshot = { goal: 'stale', requires_reconciliation: true } as AgentTaskSnapshot;
  const sending = harness.send();
  harness.handleEvent({ type: 'ready', session: 'initial' });
  assert.equal(harness.refs.session.current, 'initial');
  assert.equal(harness.ui.taskSnapshot, null);
  harness.handleEvent({
    type: 'task_state',
    task: { goal: 'initial', requires_reconciliation: true },
  });
  assert.equal((harness.ui.taskSnapshot as AgentTaskSnapshot | null)?.goal, 'initial');
  await settle();
  harness.resolveAck();
  await sending;
});

test('failed startup clears its pending session phase and preserves the draft', async () => {
  const start = deferred<void>();
  const harness = sendHarness({ start: start.promise });
  const sending = harness.send();
  start.reject(new Error('startup failed'));
  await sending;
  assert.equal(harness.refs.sessionStart.current, null);
  assert.equal(harness.refs.running.current, false);
  assert.equal(harness.ui.draft, 'hello');
});

test('historical task review appears before any message is sent', async () => {
  const task = {
    requires_reconciliation: true,
    effect_unknown_actions: [{ id: 'history-action' }],
  };
  const harness = sessionHarness(async () => [{ kind: 'task', task }]);
  await harness.open({ id: 'history' });
  assert.deepEqual(pendingReconciliation(harness.state.taskSnapshot).ids, ['history-action']);
  assert.equal(harness.state.taskSnapshot?.requires_reconciliation, true);
});

const reviewTask = { requires_reconciliation: true, effect_unknown_actions: [{ id: 'old-1' }] };

function reviewHandshake(harness: ReturnType<typeof reconcileHarness>) {
  harness.panel.handleEvent({ type: 'ready', session: 'session' });
  harness.panel.handleEvent({ type: 'task_state', task: { requires_reconciliation: false } });
  assert.equal(
    harness.calls.filter((call) => call.command === 'assistant_reconcile_effects').length,
    0,
  );
  harness.panel.handleEvent({ type: 'ready', session: 'history' });
}

test('historical review resumes its own session and waits for its snapshot, without sending a message', async () => {
  const harness = reconcileHarness();
  harness.panel.refs.session.current = 'history';
  const reviewing = harness.reconcileEffects(['old-1']);
  await settle();
  assert.equal(harness.calls[0].args.resume, 'history');
  reviewHandshake(harness);
  assert.equal(harness.panel.refs.running.current, true);
  assert.deepEqual(harness.state.reconciling, [true], 'target ready must not unlock review');
  harness.panel.handleEvent({ type: 'task_state', task: reviewTask });
  await reviewing;
  assert.deepEqual(
    harness.calls.map((call) => call.command),
    ['assistant_start', 'assistant_reconcile_effects'],
  );
  assert.deepEqual(harness.calls[1].args.ids, ['old-1']);
  assert.equal(harness.panel.refs.running.current, false);
});

test('historical review refreshes a changed action list without submitting stale ids', async () => {
  const harness = reconcileHarness();
  harness.panel.refs.session.current = 'history';
  const reviewing = harness.reconcileEffects(['old-1']);
  reviewHandshake(harness);
  harness.panel.handleEvent({
    type: 'task_state',
    task: { ...reviewTask, effect_unknown_actions: [{ id: 'new-1' }] },
  });
  await reviewing;
  assert.match(harness.state.error, /changed/);
  assert.deepEqual(pendingReconciliation(harness.panel.ui.taskSnapshot).ids, ['new-1']);
  assert.equal(harness.calls.length, 1);
});

for (const interruption of ['stop', 'error', 'exited', 'switch'] as const) {
  test(`historical review ${interruption} cannot submit a late reconciliation`, async () => {
    const harness = reconcileHarness();
    harness.panel.refs.session.current = 'history';
    const reviewing = harness.reconcileEffects(['old-1']);
    if (interruption === 'stop') await harness.panel.stop();
    else if (interruption === 'switch') {
      harness.conversationRequest.current++;
      harness.panel.refs.session.current = 'other';
      harness.panel.refs.sessionStart.current?.onError?.(new Error('The conversation changed.'));
    } else harness.panel.handleEvent({ type: interruption, message: 'restore failed' });
    await reviewing;
    harness.panel.handleEvent({ type: 'ready', session: 'history' });
    harness.panel.handleEvent({ type: 'task_state', task: reviewTask });
    assert.equal(harness.calls.length, 1);
    assert.equal(harness.panel.refs.sessionStart.current, null);
    assert.equal(harness.panel.refs.running.current, false);
    assert.equal(harness.state.notice, '');
  });
}

function keyboardEvent(key: string, options: Record<string, unknown> = {}) {
  return {
    key,
    shiftKey: false,
    defaultPrevented: false,
    nativeEvent: { isComposing: false, keyCode: 0 },
    currentTarget: { selectionStart: 1, selectionEnd: 1 },
    prevented: false,
    stopped: false,
    preventDefault() {
      this.prevented = true;
    },
    stopPropagation() {
      this.stopped = true;
    },
    ...options,
  };
}

test('message keyboard preserves IME confirmation, Shift+Enter, and normal Enter sending', () => {
  let sent = 0;
  const composing = { current: false };
  const keydown = compileFunction<(event: ReturnType<typeof keyboardEvent>) => void>(
    'handleInputKeyDown',
    {
      composing,
      input: '你好',
      send: () => {
        sent++;
      },
      history: { current: [] },
    },
  );
  for (const nativeEvent of [
    { isComposing: true, keyCode: 13 },
    { isComposing: false, keyCode: 229 },
  ]) {
    const event = keyboardEvent('Enter', { nativeEvent });
    keydown(event);
    assert.equal(event.prevented, false);
  }
  composing.current = true;
  keydown(keyboardEvent('Enter'));
  composing.current = false;
  keydown(keyboardEvent('Enter', { shiftKey: true }));
  keydown(keyboardEvent('Enter', { defaultPrevented: true }));
  assert.equal(sent, 0);
  const event = keyboardEvent('Enter');
  keydown(event);
  assert.equal(sent, 1);
  assert.equal(event.prevented, true);
  assert.match(panelSource, /onCompositionStart/);
  assert.match(panelSource, /onCompositionEnd/);
  assert.match(panelSource, /onKeyDown=\{handleInputKeyDown\}/);
});

for (const phase of ['start', 'sent'] as const) {
  test(`Escape stops the same assistant operation during ${phase}`, async () => {
    const pending = deferred<void>();
    const panel = sendHarness(phase === 'start' ? { start: pending.promise } : {});
    const sending = panel.send();
    await settle();
    let stopping: Promise<void> | undefined;
    const app = { current: { assistantSettingsOpen: false } };
    const composing = { current: false };
    const keydown = compileFunction<(event: ReturnType<typeof keyboardEvent>) => void>(
      'handlePanelKeyDown',
      {
        runningRef: panel.refs.running,
        app,
        composing,
        stop: panel.stop,
        perform: (action: () => Promise<void>) => {
          stopping = action();
        },
      },
    );
    keydown(keyboardEvent('Escape', { nativeEvent: { isComposing: true } }));
    keydown(keyboardEvent('Escape', { nativeEvent: { keyCode: 229 } }));
    keydown(keyboardEvent('Escape', { defaultPrevented: true }));
    app.current.assistantSettingsOpen = true;
    keydown(keyboardEvent('Escape'));
    app.current.assistantSettingsOpen = false;
    assert.equal(stopping, undefined);
    const event = keyboardEvent('Escape');
    keydown(event);
    await stopping;
    assert.equal(event.prevented, true);
    assert.equal(event.stopped, true);
    assert.equal(panel.commands.at(-1), 'assistant_cancel');
    pending.resolve();
    panel.resolveAck();
    await sending;
    if (phase === 'start') assert.ok(!panel.commands.includes('assistant_send'));
  });
}

test('stopping review after its snapshot but before start IPC settles releases busy immediately', async () => {
  const start = deferred<void>();
  const harness = reconcileHarness({ start: start.promise });
  harness.panel.refs.session.current = 'history';
  const reviewing = harness.reconcileEffects(['old-1']);
  reviewHandshake(harness);
  harness.panel.handleEvent({ type: 'task_state', task: reviewTask });
  await harness.panel.stop();
  await reviewing;
  assert.equal(harness.panel.refs.running.current, false);
  assert.equal(harness.panel.refs.sessionStart.current, null);
  assert.equal(harness.calls.length, 1);
  start.resolve();
  await settle();
  assert.equal(harness.calls.length, 1, 'late start acknowledgement cannot submit the old review');
});

test('review notices changed details even when resumed action ids are reused', async () => {
  const harness = reconcileHarness();
  harness.panel.refs.session.current = 'history';
  const reviewing = harness.reconcileEffects(['old-1']);
  reviewHandshake(harness);
  harness.panel.handleEvent({
    type: 'task_state',
    task: { ...reviewTask, effect_unknown_actions: [{ id: 'old-1', arguments: 'changed' }] },
  });
  await reviewing;
  assert.match(harness.state.error, /changed/);
  assert.equal(harness.calls.length, 1);
});

test('IME arrow keys cannot replace candidate text with input history', () => {
  let changed = false;
  const composing = { current: false };
  const keydown = compileFunction<(event: ReturnType<typeof keyboardEvent>) => void>(
    'handleInputKeyDown',
    {
      composing,
      input: '你好',
      history: { current: ['old message'] },
      historyIndex: { current: null },
      historyDraft: { current: '' },
      historyInput: () => {
        changed = true;
        return { index: 0, value: 'old message' };
      },
      setInput: () => {
        changed = true;
      },
      send: () => {},
    },
  );
  for (const key of ['ArrowUp', 'ArrowDown']) {
    for (const nativeEvent of [
      { isComposing: true, keyCode: 0 },
      { isComposing: false, keyCode: 229 },
    ]) {
      keydown(
        keyboardEvent(key, { nativeEvent, currentTarget: { selectionStart: 0, selectionEnd: 2 } }),
      );
    }
    composing.current = true;
    keydown(keyboardEvent(key, { currentTarget: { selectionStart: 0, selectionEnd: 2 } }));
    composing.current = false;
  }
  assert.equal(changed, false);
});

test('failed historical resume releases review without sending reconciliation', async () => {
  const start = deferred<void>();
  const harness = reconcileHarness({ start: start.promise });
  harness.panel.refs.session.current = 'history';
  const reviewing = harness.reconcileEffects(['old-1']);
  start.reject(new Error('restore IPC failed'));
  await reviewing;
  assert.match(harness.state.error, /restore IPC failed/);
  assert.equal(harness.panel.refs.sessionStart.current, null);
  assert.equal(harness.panel.refs.running.current, false);
  assert.equal(harness.calls.length, 1);
});

for (const resumed of [false, true]) {
  for (const reject of [false, true]) {
    test(`stop during ${resumed ? 'resumed' : 'direct'} review IPC releases busy and ignores late ${reject ? 'failure' : 'success'}`, async () => {
      const response = deferred<void>();
      const harness = reconcileHarness({ response: response.promise });
      if (resumed) harness.panel.refs.session.current = 'history';
      const reviewing = harness.reconcileEffects(['old-1']);
      if (resumed) {
        reviewHandshake(harness);
        harness.panel.handleEvent({ type: 'task_state', task: reviewTask });
      }
      await settle();
      assert.equal(harness.calls.at(-1)?.command, 'assistant_reconcile_effects');
      assert.equal(harness.panel.refs.sessionStart.current, null);
      await harness.panel.stop();
      assert.equal(
        harness.panel.refs.running.current,
        false,
        'stop releases the idle review immediately',
      );
      assert.equal(harness.state.reconciling.at(-1), false);
      const sending = harness.panel.send();
      await settle();
      assert.equal(harness.panel.refs.running.current, true);
      if (reject) response.reject(new Error('late review failure'));
      else response.resolve();
      await reviewing;
      assert.equal(
        harness.panel.refs.running.current,
        true,
        'old review cannot clear the new send',
      );
      assert.equal(harness.state.notice, '');
      assert.equal(harness.state.error, '');
      harness.panel.resolveAck();
      await sending;
    });
  }
}

for (const cancelError of ['助手还没有启动', 'cancel pipe failed']) {
  test(`stop during startup handles cancellation error: ${cancelError}`, async () => {
    const start = deferred<void>();
    const panel = sendHarness({ start: start.promise, cancelError });
    const sending = panel.send();
    await settle();
    if (cancelError === '助手还没有启动') await panel.stop();
    else await assert.rejects(panel.stop(), (error) => error === cancelError);
    start.resolve();
    await sending;
    assert.equal(panel.commands.includes('assistant_send'), false);
    assert.equal(panel.ui.draft, 'hello');
    assert.equal(panel.ui.error, '');
  });
}

test('stop outside startup preserves a missing-process error', async () => {
  const panel = sendHarness({ cancelError: '助手还没有启动' });
  await assert.rejects(panel.stop(), (error) => error === '助手还没有启动');
});

for (const mode of ['external', 'managed'] as const) {
  for (const enabled of [false, true]) {
    test(`document retrieval startup respects ${mode} enabled=${enabled} without a model request`, async () => {
      const effect = effects.find((effect) => effect.body.includes("'assistant_xerj_node'"));
      assert.ok(effect);
      const commands: string[] = [];
      const statuses: unknown[] = [];
      const run = compileExpression<() => (() => void) | undefined>(effect.body, {
        hasBackend: () => true,
        initPhase: 'ready',
        settings: { retrieval: { enabled, mode } },
        setDocsIndex: (value: unknown) => statuses.push(value),
        invoke: (command: string) => {
          commands.push(command);
          return Promise.reject('node unavailable');
        },
      });
      const cleanup = run();
      await settle();
      assert.deepEqual(commands, enabled && mode === 'managed' ? ['assistant_xerj_node'] : []);
      assert.deepEqual(
        statuses.at(-1),
        enabled && mode === 'managed'
          ? { type: 'docs_index_status', state: 'not_connected', detail: 'node unavailable' }
          : null,
      );
      cleanup?.();
    });
  }
}

test('a late node startup failure cannot replace a newer document retrieval status', async () => {
  const effect = effects.find((effect) => effect.body.includes("'assistant_xerj_node'"));
  assert.ok(effect);
  const pending = deferred<void>();
  const statuses: unknown[] = [];
  const run = compileExpression<() => () => void>(effect.body, {
    hasBackend: () => true,
    initPhase: 'ready',
    settings: { retrieval: { enabled: true, mode: 'managed' } },
    setDocsIndex: (value: unknown) => statuses.push(value),
    invoke: () => pending.promise,
  });
  const cleanup = run();
  cleanup();
  pending.reject('old node error');
  await settle();
  assert.deepEqual(statuses, [null]);
});

test('index status from an old model cannot appear as ready for the current selection', () => {
  const panel = sendHarness({ versionRef: { id: 'official', revision: 131 } });
  panel.handleEvent({
    type: 'docs_index_status',
    state: 'ready',
    scope: { kind: 'model', name: 'official@130' },
  });
  assert.equal(panel.ui.docsIndex, null);
  panel.handleEvent({
    type: 'docs_index_status',
    state: 'ready',
    scope: { kind: 'app', name: 'app' },
  });
  assert.equal(panel.ui.docsIndex, null);
  panel.handleEvent({
    type: 'docs_index_status',
    state: 'ready',
    scope: { kind: 'model', name: 'official@131' },
  });
  assert.deepEqual(panel.ui.docsIndex, {
    type: 'docs_index_status',
    state: 'ready',
    scope: { kind: 'model', name: 'official@131' },
  });
});

test('application document index status rejects workspace search status', () => {
  const panel = sendHarness();
  panel.handleEvent({
    type: 'docs_index_status',
    state: 'ready',
    scope: { kind: 'workspace', name: 'work' },
  });
  assert.equal(panel.ui.docsIndex, null);
  const current = { type: 'docs_index_status', state: 'ready', scope: { kind: 'app', name: null } };
  panel.handleEvent(current);
  assert.deepEqual(panel.ui.docsIndex, current);
});

for (const failure of ['error', 'exited']) {
  test(`startup ${failure} after IPC success settles the send and preserves its draft`, async () => {
    const harness = sendHarness({ manualReady: true });
    const sending = harness.send();
    await settle();
    const id = harness.refs.startupId.current;
    harness.handleEvent({ type: 'ready', session: 'session', startup_id: id });
    assert.equal(harness.commands.includes('assistant_send'), false);
    harness.handleEvent({ type: failure, message: 'startup failed', startup_id: id });
    await sending;
    assert.equal(harness.commands.includes('assistant_send'), false);
    assert.equal(harness.refs.sessionStart.current, null);
    assert.equal(harness.refs.running.current, false);
    assert.equal(harness.ui.draft, 'hello');
  });
}

test('stopped startup events cannot bind or reject a new conversation handshake', async () => {
  const harness = sendHarness({ manualReady: true });
  const first = harness.send();
  const oldId = harness.refs.startupId.current;
  await harness.stop();
  await first;
  harness.refs.fresh.current = true;
  harness.refs.session.current = null;
  const second = harness.send();
  const newId = harness.refs.startupId.current;
  assert.notEqual(oldId, newId);
  for (const event of [
    { type: 'ready', session: 'old', startup_id: oldId },
    { type: 'ready', session: 'legacy', startup_id: undefined },
    { type: 'error', message: 'old error', startup_id: oldId },
    { type: 'task_state', task: reviewTask, startup_id: oldId },
  ])
    harness.handleEvent(event);
  assert.equal(harness.refs.session.current, null);
  assert.equal(harness.refs.running.current, true);
  assert.equal(harness.ui.taskSnapshot, null);
  assert.equal(harness.refs.sessionStart.current?.skipConfigure, true);
  harness.handleEvent({ type: 'ready', session: 'previous', startup_id: newId });
  await settle();
  assert.equal(harness.commands.includes('assistant_send'), false);
  harness.handleEvent({ type: 'ready', session: 'new', startup_id: newId });
  await settle();
  assert.equal(harness.refs.session.current, 'new');
  assert.equal(harness.commands.filter((command) => command === 'assistant_send').length, 1);
  harness.resolveAck();
  await second;
});

test('unsent text and attachments require an explicit discard before changing conversations', async () => {
  for (const draft of [
    { input: 'unfinished', attachments: [] },
    { input: '', attachments: [attachment('A.txt')] },
  ]) {
    let answer = false;
    let asked = 0;
    const confirm = compileFunction<() => Promise<boolean>>('confirmConversationChange', {
      inputRef: { current: draft.input },
      attachmentsRef: { current: draft.attachments },
      attaching: false,
      t: (_zh: string, en: string) => en,
      askConfirm: async () => {
        asked++;
        return answer;
      },
    });
    assert.equal(await confirm(), false);
    answer = true;
    assert.equal(await confirm(), true);
    assert.equal(asked, 2);
  }
});

test('opening another session clears only an explicitly discarded draft and attachments', async () => {
  const draft = { input: 'for session A', attachments: [attachment('A.pdf')], discard: false };
  let loads = 0;
  const harness = sessionHarness(async () => {
    loads++;
    return [];
  }, draft);
  await harness.open({ id: 'B' });
  assert.equal(loads, 0);
  assert.equal(harness.state.input, draft.input);
  assert.deepEqual(harness.state.attachments, draft.attachments);
  draft.discard = true;
  await harness.open({ id: 'B' });
  assert.equal(harness.refs.session.current, 'B');
  assert.equal(harness.state.input, '');
  assert.deepEqual(harness.state.attachments, []);
  assert.equal(harness.refs.historyIndex.current, null);
});

test('typing while a history transcript loads keeps the draft and the original conversation', async () => {
  const transcript = deferred<Message[]>();
  const harness = sessionHarness(() => transcript.promise);
  const opening = harness.open({ id: 'B' });
  await settle();
  harness.refs.input.current = 'new unsent text';
  transcript.resolve([]);
  await opening;
  assert.equal(harness.refs.session.current, null);
  assert.equal(harness.refs.input.current, 'new unsent text');
});

function deletionHarness(answers = [true, true], remove: () => Promise<void> = async () => {}) {
  const state = {
    input: 'draft',
    attachments: [attachment('draft.txt')],
    messages: ['old'],
    sessions: [{ id: 'A' }, { id: 'B' }],
    review: true,
    notice: 'notice',
    usage: 'usage',
    usagePending: true,
    contextUsage: { tokens: 90 } as { tokens: number } | null,
    limitReached: true,
    stepLimit: 40,
    sessionUsage: { turns: 1, tokens: 2 },
    followed: 0,
  };
  const refs = {
    input: { current: state.input },
    attachments: { current: state.attachments },
    session: { current: 'A' as string | null },
    fresh: { current: false },
    startupId: { current: 'startup' as string | null },
    request: { current: 0 },
    historyIndex: { current: 2 as number | null },
    history: { current: ['old'] },
    running: { current: false },
  };
  const calls: string[] = [];
  const shared = {
    attaching: false,
    inputRef: refs.input,
    attachmentsRef: refs.attachments,
    t: (_zh: string, en: string) => en,
    askConfirm: async () => {
      calls.push('confirm');
      return answers.shift() ?? false;
    },
  };
  const context = {
    deletingSessionRef: { current: false },
    setDeletingSession: () => {},
    ...shared,
    confirmConversationChange: compileFunction('confirmConversationChange', shared),
    runningRef: refs.running,
    conversationRequest: refs.request,
    session: refs.session,
    fresh: refs.fresh,
    startupId: refs.startupId,
    historyIndex: refs.historyIndex,
    history: refs.history,
    invoke: async (command: string) => {
      calls.push(command);
      if (command === 'assistant_delete_session') return remove();
      return ['updated'];
    },
    setSessions: (update: (old: typeof state.sessions) => typeof state.sessions) => {
      state.sessions = update(state.sessions);
    },
    setInput: (value: string) => {
      state.input = value;
    },
    setAttachments: (value: Attachment[]) => {
      state.attachments = value;
    },
    clearTaskReview: () => {
      state.review = false;
    },
    setMessages: (value: string[]) => {
      state.messages = value;
    },
    setAttachNotice: (value: string) => {
      state.notice = value;
    },
    setUsage: (value: string) => {
      state.usage = value;
    },
    setContextUsage: (value: typeof state.contextUsage) => {
      state.contextUsage = value;
    },
    setLimitReached: (value: boolean) => {
      state.limitReached = value;
    },
    setStepLimit: (value: number) => {
      state.stepLimit = value;
    },
    setSessionUsage: (value: typeof state.sessionUsage) => {
      state.sessionUsage = value;
    },
    setUsagePending: (value: boolean) => {
      state.usagePending = value;
    },
    resetFollow: () => {
      state.followed++;
    },
  };
  const run = (id = 'A') =>
    compileFunction<(item: { id: string; title: string }) => Promise<void>>(
      'deleteSession',
      context,
    )({ id, title: id });
  return { state, refs, calls, context, run, snapshot: () => structuredClone({ state, refs }) };
}

for (const answers of [[false], [true, false]]) {
  test(`cancel deletion at confirmation ${answers.length} preserves all conversation state`, async () => {
    const h = deletionHarness([...answers]);
    const before = h.snapshot();
    await h.run();
    assert.deepEqual(h.snapshot(), before);
    assert.deepEqual(
      h.calls,
      answers.map(() => 'confirm'),
    );
  });
}

test('current session deletion waits for both confirmations and successful IPC before discarding', async () => {
  const pending = deferred<void>();
  const h = deletionHarness([true, true], () => pending.promise);
  const before = h.snapshot();
  const deleting = h.run();
  await settle();
  assert.deepEqual(h.snapshot(), before);
  assert.deepEqual(h.calls, ['confirm', 'confirm', 'assistant_delete_session']);
  pending.resolve();
  await deleting;
  assert.equal(h.refs.session.current, null);
  assert.equal(h.refs.fresh.current, true);
  assert.equal(h.refs.startupId.current, null);
  assert.equal(h.refs.request.current, 1);
  assert.equal(h.refs.historyIndex.current, null);
  assert.equal(h.state.input, '');
  assert.deepEqual(h.state.attachments, []);
  assert.deepEqual(h.state.messages, []);
  assert.equal(h.state.review, false);
  assert.equal(h.state.contextUsage, null);
  assert.equal(h.state.limitReached, false);
  assert.equal(h.state.stepLimit, 0);
});

test('delete IPC failure preserves session, draft, messages and review', async () => {
  const h = deletionHarness([true, true], async () => {
    throw new Error('disk error');
  });
  const before = h.snapshot();
  await assert.rejects(h.run(), /disk error/);
  assert.deepEqual(h.snapshot(), before);
});

test('deleting noncurrent session does not discard the current draft', async () => {
  const h = deletionHarness([true]);
  const before = h.snapshot();
  await h.run('B');
  before.state.sessions = [{ id: 'A' }];
  before.refs.request.current++;
  before.refs.history.current = ['updated'];
  assert.deepEqual(h.snapshot(), before);
  assert.deepEqual(h.calls, ['confirm', 'assistant_delete_session', 'assistant_input_history']);
});

test('deleting session cannot reset another conversation opened while IPC is pending', async () => {
  const pending = deferred<void>();
  const h = deletionHarness([true, true], () => pending.promise);
  const deleting = h.run();
  await settle();
  h.refs.session.current = 'B';
  h.refs.request.current++;
  const before = h.snapshot();
  pending.resolve();
  await deleting;
  before.state.sessions = [{ id: 'B' }];
  before.refs.history.current = ['updated'];
  assert.deepEqual(h.snapshot(), before);
});

test('new text and attachments entered during delete IPC survive successful deletion', async () => {
  const pending = deferred<void>();
  const h = deletionHarness([true, true], () => pending.promise);
  const deleting = h.run();
  await settle();
  h.state.input = h.refs.input.current = 'new text';
  h.state.attachments = h.refs.attachments.current = [attachment('new.txt')];
  pending.resolve();
  await deleting;
  assert.equal(h.refs.session.current, null);
  assert.equal(h.state.input, 'new text');
  assert.deepEqual(h.state.attachments, [attachment('new.txt')]);
  assert.equal(h.refs.historyIndex.current, 2);
});

test('attachment loading prevents deletion without invalidating attachment ownership', async () => {
  const h = deletionHarness();
  h.context.attaching = true;
  const before = h.snapshot();
  await h.run();
  assert.deepEqual(h.snapshot(), before);
  assert.deepEqual(h.calls, []);
});

test('switching identity during confirmation aborts deletion before IPC', async () => {
  const h = deletionHarness();
  const confirmation = deferred<boolean>();
  h.context.askConfirm = () => confirmation.promise;
  const deleting = h.run();
  h.refs.session.current = 'B';
  h.refs.request.current++;
  const before = h.snapshot();
  confirmation.resolve(true);
  await deleting;
  assert.deepEqual(h.snapshot(), before);
  assert.equal(h.calls.includes('assistant_delete_session'), false);
});

test('pending deletion synchronously blocks attachment, send and duplicate deletion', async () => {
  const pending = deferred<void>();
  const h = deletionHarness([true, true], () => pending.promise);
  const deleting = h.run();
  await settle();
  assert.equal(h.context.deletingSessionRef.current, true);
  const attach = compileFunction<() => Promise<void>>('attachDocument', {
    deletingSessionRef: h.context.deletingSessionRef,
  });
  const send = compileFunction<() => Promise<void>>('send', {
    deletingSessionRef: h.context.deletingSessionRef,
    input: 'new input',
    attachments: [],
  });
  await attach();
  await send();
  await h.run();
  assert.deepEqual(h.calls, ['confirm', 'confirm', 'assistant_delete_session']);
  pending.resolve();
  await deleting;
  assert.equal(h.context.deletingSessionRef.current, false);
});

for (const outcome of ['cancel', 'failure']) {
  test(`deletion lock is released on ${outcome}`, async () => {
    const h = deletionHarness(outcome === 'cancel' ? [false] : [true, true], async () => {
      throw new Error('failed');
    });
    if (outcome === 'cancel') await h.run();
    else await assert.rejects(h.run(), /failed/);
    assert.equal(h.context.deletingSessionRef.current, false);
  });
}

for (const transcriptAfterDelete of [false, true]) {
  test(`deletion prevents an earlier transcript from opening the deleted session (${transcriptAfterDelete ? 'after' : 'during'} deletion)`, async () => {
    const transcript = deferred<Message[]>();
    const pending = deferred<void>();
    const opening = sessionHarness(() => transcript.promise);
    opening.refs.session.current = 'A';
    const loading = opening.open({ id: 'B' });
    await settle();
    const h = deletionHarness([true], () => pending.promise);
    h.context.deletingSessionRef = opening.refs.deletingSession;
    h.context.conversationRequest = opening.refs.conversationRequest;
    h.context.session = opening.refs.session;
    const deleting = h.run('B');
    await settle();
    if (transcriptAfterDelete) {
      pending.resolve();
      await deleting;
    }
    transcript.resolve([]);
    await loading;
    assert.equal(opening.refs.session.current, 'A');
    if (!transcriptAfterDelete) {
      pending.resolve();
      await deleting;
    }
  });
}

test('opening history during deletion is blocked and becomes available after cancellation', async () => {
  let loads = 0;
  const opening = sessionHarness(async () => {
    loads++;
    return [];
  });
  opening.refs.session.current = 'A';
  const h = deletionHarness();
  const confirmation = deferred<boolean>();
  h.context.deletingSessionRef = opening.refs.deletingSession;
  h.context.askConfirm = () => confirmation.promise;
  const deleting = h.run('B');
  await opening.open({ id: 'B' });
  assert.equal(loads, 0);
  assert.equal(opening.refs.session.current, 'A');
  confirmation.resolve(false);
  await deleting;
  await opening.open({ id: 'B' });
  assert.equal(loads, 1);
  assert.equal(opening.refs.session.current, 'B');
});

test('settings cannot be saved before the original configuration has loaded', async () => {
  const calls: string[] = [];
  const save = compileFunction<(next: Settings) => Promise<void>>('save', {
    ready: false,
    t: (_zh: string, en: string) => en,
    invoke: async (command: string) => {
      calls.push(command);
    },
  });
  await assert.rejects(save(defaultSettings), /Load the assistant settings/);
  assert.deepEqual(calls, []);
  assert.match(panelSource, /state.assistantSettingsOpen && ready/);
});

test('failed settings offer confirmed backup/reset and reopen only after successful recovery', async () => {
  for (const consent of [false, true]) {
    const calls: string[] = [],
      notices: string[] = [];
    let attempt = 0;
    const updates: unknown[] = [];
    const recover = compileFunction<() => Promise<void>>('recoverSettings', {
      running: false,
      resettingSettings: false,
      initPhase: 'failed',
      t: (_zh: string, en: string) => en,
      askConfirm: async () => consent,
      withTimeout: (work: Promise<unknown>) => work,
      SETTINGS_TIMEOUT_MS: 1000,
      invoke: async (command: string) => {
        calls.push(command);
        return '/backup.json';
      },
      setResettingSettings: () => {},
      setAttachNotice: (text: string) => notices.push(text),
      setSettingsDirty: () => {},
      update: (value: unknown) => updates.push(value),
      setInitAttempt: (fn: (n: number) => number) => {
        attempt = fn(attempt);
      },
      setInitError: (error: string) => {
        throw Error(error);
      },
      errorText: String,
    });
    await recover();
    assert.deepEqual(calls, consent ? ['assistant_reset_settings'] : []);
    assert.equal(attempt, consent ? 1 : 0);
    assert.deepEqual(updates, consent ? [{ assistantSettingsOpen: true }] : []);
    if (consent) assert.match(notices[0], /backup.json/);
  }
});

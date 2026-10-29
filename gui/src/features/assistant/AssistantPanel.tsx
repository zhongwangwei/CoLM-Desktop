import { useEffect, useRef, useState } from 'react';
import { invoke, invokeJson, listen, hasBackend, withTimeout } from '../../lib/ipc';
import { useApp } from '../../lib/app-state';
import { askConfirm } from '../../lib/confirm';
import { guideState, drive } from '../../lib/guide';
import { Button, Select, Notice } from '../../components/ui';
import AssistantSettings from './Settings';
import { Markdown } from './MarkdownMessage';
import { followAfterContent, followAfterJump, followAfterScroll } from './scroll';
import {
  appendDelta,
  apiEfforts,
  defaultSettings,
  egressTarget,
  historyContextLabel,
  historyInput,
  historyScopeHint,
  historyScopeLabel,
  historySettingsOf,
  providerId,
  assistantModels,
  selectAssistantModel,
  thinkSettings,
  thinkValue,
  withChoice,
  withHistoryScope,
  EXAMPLE_STARTERS,
  examplePrompt,
  rejectedBeforeTurn,
  withoutLastUser,
  type AgentEvent,
  type HistoryScope,
  type Message,
  type Settings,
} from './model';
import {
  approvalSnapshot,
  approvedUiRequest,
  needsUiGrant,
  respondToUiRequest,
  uiGrant,
  type UiGrant,
} from './ui-request';
import {
  clearsDraftAfterAck,
  newTurn,
  turnDelivered,
  turnFailed,
  turnOutput,
  turnPreTurnRejected,
  type PendingTurn,
} from './turn';

interface Session {
  id: string;
  title: string;
  turns: number;
  updated_ms: number;
}

/** 本地附件：只保留抽出来的文字与元信息，原文件不写进会话。 */
interface DocumentAttachment {
  path: string;
  name: string;
  text: string;
  title?: string | null;
  pages?: number | null;
  truncated?: boolean;
  warnings?: string[];
}

/**
 * 一条消息里附件文字的合计上限。附件直接放进用户消息，之后每次请求都会连同历史再发一遍：原来的 400 万字符
 * （英文约 100 万 token、中文约 400 万）远超模型上下文，那一次请求必失败，而且留在会话里，此后每次请求都超长、
 * 每次都计入会话预算（DeepSeek 改动复查）。20 万字符约 5–20 万 token；超出的部分截断并标明，不整份拒绝。
 */
const ATTACHMENT_CHARS_LIMIT = 200_000;
const CONTEXT_WARN_RATIO = 0.8;
/**
 * 初始化两步的等待上限（GUI-16）。设置是本地小文件、历史要另起一个 colm-cli 子进程（冷启动可能很慢），
 * 两者任何一个卡住都不能无限禁用整个助手：超时后进入明确的失败态并给出重试入口。
 */
const SETTINGS_TIMEOUT_MS = 10_000;
const HISTORY_TIMEOUT_MS = 15_000;
function tokenLabel(value: number) {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(2)}M`;
  if (value >= 1_000) return `${Math.round(value / 1_000)}k`;
  return String(Math.round(value));
}
export default function AssistantPanel() {
  const { state, navigate, update } = useApp();
  const app = useRef(state);
  app.current = state;
  function t(zh: string, en: string) {
    return app.current.language === 'en' ? en : zh;
  }
  function errorText(message: string) {
    const text = String(message).replace(/^Error: /, '');
    if (text.includes('no API key is stored') || text.includes('no API key is configured'))
      return t(
        '这个服务还没有登记 API Key；点上面的"设置"登记后再发送。',
        'No API key is stored for this service; open Settings, register one, then send again.',
      );
    if (text.startsWith('session limit reached:'))
      return t(
        '本会话已达到设置里的会话上限；开始新对话可以继续。',
        'This conversation reached its configured session limit; start a new conversation to continue.',
      );
    if (text.includes('still answering the previous message'))
      return t(
        '上一条还在回答，请先点"停止"或等它结束。',
        'The previous answer is still running; stop it or wait for it to finish.',
      );
    if (text.includes('the context window for this model endpoint is unknown'))
      return t(
        '这个端点/模型没有已知的上下文窗口。请在助手设置 → 高级设置里填写“上下文窗口”（tokens）后再发送；应用不会猜一个窗口。',
        'This endpoint/model has no known context window. Open assistant settings → Advanced, set the context window (tokens), then send again; the app does not guess one.',
      );
    if (text.includes('estimated input tokens') && text.includes('context window'))
      return t(
        '这条请求即使折叠较早的工具结果仍超过当前上下文窗口。本次没有发往模型服务；请缩短附件或对话、在助手设置里调大上下文窗口，或调低输出 token 上限后再发送。',
        'This request exceeds the configured context window even after folding older tool results. Nothing was sent to the model service; shorten the attachment or conversation, raise the context window in assistant settings, or lower the maximum output tokens, then send again.',
      );
    return text;
  }
  function attachmentErrorText(message: string) {
    const text = String(message);
    if (text.includes('unsupported document type'))
      return t(
        '不支持这种文件；可附加 PDF、DOCX、DOC、TXT、MD、CSV 或 JSON。',
        'Unsupported file type; attach PDF, DOCX, DOC, TXT, MD, CSV or JSON.',
      );
    if (text.includes('no embedded text') || text.includes('scanned image'))
      return t(
        '这个 PDF 没有可提取的文字（可能是扫描件）；请找它的 HTML 版或 OCR 版。',
        'This PDF has no embedded text (it may be scanned); find its HTML or OCR version.',
      );
    if (text.includes('no .doc converter'))
      return t(
        '这台电脑没有 .doc 转换器；请把文件另存为 .docx 或 PDF 再附加。',
        'This computer has no .doc converter; save the file as .docx or PDF and attach it again.',
      );
    if (text.includes('took longer than'))
      return t(
        '这个文件读了两分钟还没读完，已经停下；可能是文件损坏、太大，或转换器卡住了。',
        'Reading this file took over two minutes and was stopped; it may be damaged or too large, or the converter hung.',
      );
    if (text.includes('larger than'))
      return t('文件超过附件的体积上限。', 'The file is larger than the attachment size limit.');
    return errorText(message);
  }
  const [settings, setSettings] = useState<Settings>(defaultSettings);
  const settingsRef = useRef(settings);
  settingsRef.current = settings;
  const [messages, setMessages] = useState<Message[]>([]);
  const [running, setRunning] = useState(false);
  const runningRef = useRef(false);
  const [input, setInput] = useState('');
  const [attachments, setAttachments] = useState<DocumentAttachment[]>([]);
  // 附件读取要按最新的附件列表算剩余字符数：异步读取期间旧渲染的闭包会漏掉刚加进来的文件。
  const attachmentsRef = useRef(attachments);
  attachmentsRef.current = attachments;
  // 刚发出、助手还没开始回应的那一轮：拒绝时用它恢复输入框与附件（D-06 / GUI-18）。
  const outgoing = useRef<PendingTurn<DocumentAttachment> | null>(null);
  const turnSeq = useRef(0);
  const [attaching, setAttaching] = useState(false);
  const [attachNotice, setAttachNotice] = useState('');
  const [error, setError] = useState('');
  // 初始化状态分开存：核心设置失败要显式重试；输入历史只是辅助记忆，失败不挡发送（GUI-16）。
  const [initPhase, setInitPhase] = useState<'idle' | 'loading' | 'ready' | 'failed'>('idle');
  const [initError, setInitError] = useState('');
  const [initAttempt, setInitAttempt] = useState(0);
  const [historyError, setHistoryError] = useState('');
  const [historyAttempt, setHistoryAttempt] = useState(0);
  const [sessions, setSessions] = useState<Session[] | null>(null);
  // 设置对话框里的草稿是否改过：关闭/切走前要确认放弃（U-07）。草稿本身留在 Settings 内部。
  const [settingsDirty, setSettingsDirty] = useState(false);
  const [approval, setApproval] = useState<AgentEvent | null>(null);
  // Xerj 文档索引状态是应用级的，不随对话切换；设置对话框打开时直接显示最近一次状态。
  const [docsIndex, setDocsIndex] = useState<AgentEvent | null>(null);
  const [deciding, setDeciding] = useState(false);
  // U02 历史范围选择正在落盘：按钮禁用并显示忙碌，失败不清掉当前选择。
  const [scopeBusy, setScopeBusy] = useState(false);
  const [consent, setConsent] = useState<string | null>(null);
  const [usage, setUsage] = useState('');
  const [sessionUsage, setSessionUsage] = useState({ turns: 0, tokens: 0 });
  // 历史会话切换后，用量属于那个会话；在拿到它自己的 usage 事件之前不能沿用上一个会话的数字（U-07）。
  const [usagePending, setUsagePending] = useState(false);
  const [contextUsage, setContextUsage] = useState<{
    tokens: number;
    window: number;
    compressed: number;
    /** 窗口减去估算输入、输出预留与安全余量后的可用输入估算；null = 后端未启用预算。 */
    available: number | null;
    outputReserve: number;
    safety: number;
    /** 窗口来源："user" 用户填写 / "preset" 文档确认的预设。 */
    source: string | null;
  } | null>(null);
  const [limitReached, setLimitReached] = useState(false);
  const [stepLimit, setStepLimit] = useState(0);
  const [model, setModel] = useState('');
  const [ready, setReady] = useState(false);
  // 当前内置服务商是否登记过 Key；欢迎页据此给"去设置登记"的入口，而不是等发送后报英文错误。
  const [hasKey, setHasKey] = useState<boolean | null>(null);
  const session = useRef<string | null>(null);
  const conversationRequest = useRef(0);
  const streamed = useRef(false);
  const fresh = useRef(true);
  const grant = useRef<UiGrant | null>(null);
  const grantPage = useRef('');
  const history = useRef<string[]>([]);
  const historyIndex = useRef<number | null>(null);
  const historyDraft = useRef('');
  const log = useRef<HTMLDivElement>(null);
  // 流式自动滚动：默认跟随底部；用户上滚后暂停并提示"有新内容"，回到底部再恢复（U-07）。
  const stick = useRef(true);
  const latestMessages = useRef(messages);
  const [unread, setUnread] = useState(false);
  function currentApprovalSnapshot() {
    const page = app.current,
      view = guideState();
    return approvalSnapshot(
      {
        route: page.route,
        caseTab: page.caseTab,
        root: page.root,
        caseDir: page.selectedCase?.entry.dir,
        versionRef: page.versionRef,
        kernelDir: view.kernelDir,
      },
      view,
    );
  }
  function busy(value: boolean) {
    runningRef.current = value;
    setRunning(value);
  }
  async function perform(action: () => Promise<void>) {
    setError('');
    try {
      await action();
    } catch (e) {
      setError(errorText(String(e)));
    }
  }
  useEffect(() => {
    if (!messages.length) {
      latestMessages.current = messages;
      return;
    }
    // approval 变化沿用原来"滚到底"的行为；只有消息内容变了才给暂停中的用户加"有新内容"。
    const changed = latestMessages.current !== messages;
    latestMessages.current = messages;
    if (stick.current) {
      log.current?.scrollTo({ top: log.current.scrollHeight });
      setUnread(false);
    } else if (changed) setUnread(followAfterContent({ stick: false, unread }).unread);
  }, [messages, approval, unread]);
  useEffect(() => {
    if (!hasBackend()) return;
    let active = true;
    let unlisten: (() => void) | undefined;
    const handler = (event: AgentEvent) => {
      // 后端产出这一轮自己的事件 = 消息确实被模型接下了；IPC 确认只代表送达，不能代替（GUI-18）。
      // docs_index_status 是应用级后台事件（索引构建可能还在跑），不能当成"这一轮被接受"的证据。
      if (
        outgoing.current &&
        event.type !== 'error' &&
        event.type !== 'ready' &&
        event.type !== 'exited' &&
        event.type !== 'docs_index_status'
      )
        outgoing.current = turnOutput(outgoing.current);
      if (!active) return;
      switch (event.type) {
        case 'ready':
          session.current = event.session || session.current;
          setModel(event.model || '');
          break;
        case 'assistant_delta':
          streamed.current = true;
          setMessages((old) => appendDelta(old, 'assistant', event.text || ''));
          break;
        case 'reasoning_delta':
          setMessages((old) => appendDelta(old, 'reasoning', event.text || ''));
          break;
        case 'tool_call':
          grant.current = uiGrant(event);
          grantPage.current = currentApprovalSnapshot();
          setMessages((old) => [
            ...old,
            {
              kind: 'tool',
              id: event.id,
              name: event.name,
              arguments: event.arguments,
              text: event.preapproved
                ? t('后端策略已批准', 'Approved by backend policy')
                : t('运行中', 'Running'),
            },
          ]);
          break;
        case 'tool_result':
          if (grant.current?.toolId === event.id) grant.current = null;
          setMessages((old) =>
            old.map((item) =>
              item.kind === 'tool' && item.id === event.id
                ? { ...item, result: event.result, ok: event.ok }
                : item,
            ),
          );
          break;
        case 'approval_request':
          setApproval(event);
          break;
        case 'ui_request': {
          void respondToUiRequest(
            event,
            async (action, args) => {
              if (needsUiGrant(event)) {
                if (grantPage.current !== currentApprovalSnapshot())
                  throw new Error(
                    t(
                      '审批后页面内容已变化，请重新读取当前页面并请求操作',
                      'The page changed after approval. Read it again and request the operation.',
                    ),
                  );
                if (
                  !approvedUiRequest(
                    grant.current,
                    event,
                    grantPage.current,
                    currentApprovalSnapshot(),
                  )
                )
                  throw new Error(
                    t(
                      '当前界面写入没有匹配的已批准工具请求',
                      'No matching approved tool request for this UI change.',
                    ),
                  );
                grant.current = null;
              }
              if (!active) throw new Error(t('助手面板已卸载', 'The assistant panel was closed.'));
              return await drive(action, args);
            },
            (id, ok, result) => invoke('assistant_ui_result', { id, ok, result }),
          ).catch((e) => {
            if (active) setError(String(e));
          });
          break;
        }
        case 'task_state':
          setMessages((old) => [
            ...old.filter((item) => item.kind !== 'task'),
            { kind: 'task', task: event.task },
          ]);
          break;
        case 'usage':
          setUsage(`${event.prompt_tokens || 0} + ${event.completion_tokens || 0} tokens`);
          setSessionUsage({
            turns: event.session_turns || 0,
            tokens: (event.session_prompt_tokens || 0) + (event.session_completion_tokens || 0),
          });
          setUsagePending(false);
          break;
        case 'context':
          setContextUsage({
            tokens: event.estimated_tokens || 0,
            window: event.window_tokens || 0,
            compressed: event.compressed_tools || 0,
            available:
              typeof event.available_input_tokens === 'number'
                ? event.available_input_tokens
                : null,
            outputReserve: event.output_reserve_tokens || 0,
            safety: event.safety_margin_tokens || 0,
            source: event.window_source || null,
          });
          break;
        case 'docs_index_status':
          // 应用级索引状态；state 里的未知取值留给设置对话框原样显示。
          setDocsIndex(event as AgentEvent);
          break;
        case 'turn_done':
          if (!streamed.current && event.content)
            setMessages((old) => [...old, { kind: 'assistant', text: event.content }]);
          setLimitReached(!!event.limit_reached);
          setStepLimit(event.step_limit || 0);
          busy(false);
          grant.current = null;
          setApproval(null);
          break;
        case 'error': {
          busy(false);
          grant.current = null;
          setApproval(null);
          // 助手在开始这一轮之前就拒绝了（会话到上限、上一条还在答、没有 Key）：这条消息没被记下，
          // 撤回刚显示的气泡，把输入和附件放回去，免得用户打的字和附件白白没了（GUI 审查 D-06）。
          // 只有还没被模型接受的轮次能撤；已经被接受的轮次收到同样的文字只是运行错误（GUI-18）。
          const sent = outgoing.current;
          if (sent && rejectedBeforeTurn(event.message || '')) {
            const outcome = turnPreTurnRejected(sent);
            outgoing.current = outcome.turn;
            if (outcome.restore) {
              setMessages(withoutLastUser);
              // 只补空位：等待期间用户新打的字、新附上的文件不能被旧轮次覆盖（GUI-18）。
              setInput((value) => value || sent.typed);
              setAttachments((items) => (items.length ? items : sent.attachments));
            }
          }
          setError(
            event.message === 'cancelled'
              ? t('已停止。', 'Stopped.')
              : errorText(event.message || t('助手错误', 'Assistant error')),
          );
          break;
        }
        case 'exited':
          busy(false);
          grant.current = null;
          setApproval(null);
          // 登记/删除 Key 会主动停掉进程；设置开着时不把它当异常报红。
          setError(
            app.current.assistantSettingsOpen
              ? ''
              : t(
                  '助手进程已退出，下次发送时重新启动。',
                  'The assistant process exited. It will restart on the next message.',
                ),
          );
          break;
      }
    };
    void (async () => {
      try {
        unlisten = await listen<AgentEvent>('assistant://event', handler);
        if (!active) {
          unlisten();
          return;
        }
      } catch (e) {
        if (active) setError(String(e));
      }
    })();
    return () => {
      active = false;
      unlisten?.();
      grant.current = null;
      if (runningRef.current) void invoke('assistant_cancel', {}).catch(() => {});
    };
    // 只在挂载时订阅一次；处理函数里的 t/errorText 读的是最新页面语言。
    // 初始化（含重试）是后面单独的 effect：重试只重读数据，绝不会再订阅一次，也就不会重复回答（GUI-16）。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  // 设置对话框关闭后重新问一次 Key 状态（可能刚登记/删除过）。
  useEffect(() => {
    if (!hasBackend() || settings.backend !== 'builtin') {
      setHasKey(null);
      return;
    }
    let active = true;
    void invoke<boolean>('assistant_has_key', { baseUrl: settings.base_url })
      .then((value) => {
        if (active) setHasKey(value);
      })
      .catch(() => {
        if (active) setHasKey(null);
      });
    return () => {
      active = false;
    };
  }, [settings.backend, settings.base_url, state.assistantSettingsOpen]);
  // 已生成的错误/提示是当时语言的字符串，切换语言后清掉，避免中英混排。
  useEffect(() => {
    setError('');
    setAttachNotice('');
  }, [state.language]);
  /**
   * 核心设置单独加载/重试（GUI-16）。原来它和输入历史串行 await，历史一失败就整个助手永久不可用；
   * 现在设置失败只把 ready 留在 false 并给重试，历史失败（下一个 effect）不影响发送。
   * 超时把"永远转圈"变成可见的失败态，重试是从零再来一次，不会碰事件订阅。
   */
  useEffect(() => {
    if (!hasBackend()) return;
    let active = true;
    setInitPhase('loading');
    setInitError('');
    void (async () => {
      try {
        const loaded = await withTimeout(
          invoke<Settings>('assistant_settings', {}),
          SETTINGS_TIMEOUT_MS,
          t('读取助手设置', 'loading assistant settings'),
        );
        if (!active) return;
        setSettings(loaded);
        setReady(true);
        setInitPhase('ready');
      } catch (e) {
        if (!active) return;
        setInitPhase('failed');
        setInitError(errorText(String(e)));
      }
    })();
    return () => {
      active = false;
    };
    // 只在挂载和用户点"重试"时执行；t/errorText 读的是最新页面语言。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initAttempt]);
  /**
   * 输入历史是辅助数据（↑/↓ 记忆）：失败只给一条可重试的提示，不挡 ready、不占错误通道，
   * 更不能让助手对话框因此禁用到重启应用（GUI-16）。单独超时，避免 colm-cli 子进程卡住初始化。
   */
  useEffect(() => {
    if (!hasBackend()) return;
    let active = true;
    setHistoryError('');
    void (async () => {
      try {
        const inputs = await withTimeout(
          invoke<string[]>('assistant_input_history', {}),
          HISTORY_TIMEOUT_MS,
          t('读取输入历史', 'loading input history'),
        );
        if (!active) return;
        history.current = inputs;
      } catch (e) {
        if (active) setHistoryError(errorText(String(e)));
      }
    })();
    return () => {
      active = false;
    };
    // 与核心设置分开重试；新增 attempt 只重读历史，不重新订阅事件。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [historyAttempt]);
  async function save(next: Settings) {
    if (runningRef.current)
      throw new Error(
        t('请先停止当前回答再修改设置。', 'Stop the current response before changing settings.'),
      );
    await invoke('assistant_save_settings', { settings: next });
    setSettings(next);
    setModel('');
    // 换后端或换端点/模型/协议后，旧的 API 上下文估算不再属于当前发送目标：清掉，别当成新配置的占用。
    if (
      next.backend !== 'builtin' ||
      next.backend !== settings.backend ||
      next.base_url !== settings.base_url ||
      next.model !== settings.model ||
      next.api_format !== settings.api_format ||
      next.context_window !== settings.context_window
    )
      setContextUsage(null);
  }
  /**
   * U02：把历史范围选择写回已保存设置（只改 history.scope）。
   * 用 settingsRef 的已保存设置做基底，绝不带上设置对话框里的未保存草稿；设置对话框开着时选择器禁用。
   */
  async function changeHistoryScope(scope: HistoryScope) {
    if (scopeBusy || runningRef.current || app.current.assistantSettingsOpen) return;
    const base = settingsRef.current;
    const next = withHistoryScope(base, scope);
    if (historySettingsOf(next).scope === historySettingsOf(base).scope) return;
    setScopeBusy(true);
    setError('');
    try {
      await invoke('assistant_save_settings', { settings: next });
      setSettings(next);
    } catch (e) {
      // 保存失败保持原来的范围：界面不能显示一个并没有落盘的选择。
      setError(errorText(String(e)));
    } finally {
      setScopeBusy(false);
    }
  }
  async function attachDocument() {
    if (runningRef.current || !ready) return;
    // 记下这次读取属于哪个对话/草稿：读取期间可以切换对话，完成后必须核对身份（GUI-17）。
    const request = conversationRequest.current;
    try {
      setAttaching(true);
      setAttachNotice('');
      // 选文件和读文件都在后端：前端不能拿一个任意路径去读（例如存 Key 的 JSON）再发给模型。
      const picked = await invokeJson<DocumentAttachment | null>('assistant_pick_document');
      // 用户取消选文件：什么都不改，读取中状态由 finally 复位。
      if (!picked) return;
      if (request !== conversationRequest.current) {
        // 结果不属于当前对话：丢掉，并明确告诉用户为什么文件没出现，而不是悄悄挂到新对话上。
        setAttachNotice(
          t(
            '读取完成前对话已切换，这个文件没有加入当前对话；需要时请重新附加。',
            'The conversation changed before the file finished reading, so it was not attached to the current conversation; attach it again if you need it.',
          ),
        );
        return;
      }
      if (!picked.text.trim())
        throw new Error(t('这个文件没有可读文字。', 'This file has no readable text.'));
      // 用 ref 读最新附件：连续读取时旧渲染闭包里的 attachments 可能还没有上一个文件。
      const used = attachmentsRef.current.reduce((total, item) => total + item.text.length, 0);
      const room = ATTACHMENT_CHARS_LIMIT - used;
      if (room <= 0)
        throw new Error(
          t(
            '这条消息的附件文字已经到上限；先发送，或删掉一个附件。',
            'The attachments for this message are at their limit; send first or remove one.',
          ),
        );
      const cut = picked.text.length > room;
      setAttachments((old) => [
        ...old,
        {
          ...picked,
          text: cut ? picked.text.slice(0, room) : picked.text,
          truncated: picked.truncated || cut,
        },
      ]);
    } catch (e) {
      // 对话已经切换时，这次读取的失败也不属于当前对话，不占用当前对话的错误提示。
      if (request === conversationRequest.current) setError(attachmentErrorText(String(e)));
    } finally {
      setAttaching(false);
    }
  }
  /** 开始一轮发送：快照这一轮要发出的草稿，并分配轮次号（GUI-18）。 */
  function beginSendTurn(typed: string, prompt: string) {
    const record = newTurn(++turnSeq.current, typed, prompt, attachmentsRef.current);
    outgoing.current = record;
    return record;
  }
  /**
   * 收尾一轮发送：IPC 确认送达、或发送本身失败。
   * 用轮次号核对身份 —— 拒绝事件先到时用户可能已经开始了新一轮，旧轮次的确认不能清掉新草稿（GUI-18）。
   */
  function settleSendTurn(id: number, delivered: boolean, failure?: unknown) {
    const sent = outgoing.current;
    if (!sent || sent.id !== id) return;
    if (delivered) {
      const settled = turnDelivered(sent);
      outgoing.current = settled;
      // 轮次前拒绝的事件已经先到：气泡撤了、草稿恢复了；这里再清一次就会把用户的内容又抹掉。
      if (!clearsDraftAfterAck(settled)) return;
      // 只清还等于这次发出的内容：等待期间用户新打的字保留（GUI-18）。
      setInput((value) => (value.trim() === settled.typed ? '' : value));
      setAttachments([]);
      const entry = settled.typed || settled.prompt;
      if (entry && history.current.at(-1) !== entry)
        history.current = [...history.current, entry].slice(-100);
      historyIndex.current = null;
      historyDraft.current = '';
      return;
    }
    // 拒绝事件已经撤过气泡、恢复过草稿：不再重复处理，也别用更笼统的 IPC 错误覆盖它。
    if (sent.phase === 'rejected') return;
    outgoing.current = turnFailed(sent);
    setMessages(withoutLastUser);
    setError(errorText(String(failure)));
  }
  async function send(next?: string) {
    const typed = (next ?? input).trim();
    const fallback = attachments.length
      ? t(
          '请阅读这些附件并总结要点。',
          'Please read the attached documents and summarize the key points.',
        )
      : '';
    const prompt = typed || fallback;
    // 附件还在读：先等它读完，否则这条消息不带附件就发出去了，读完的附件又挂到下一条上（GUI 审查 D-07）。
    if (!prompt || runningRef.current || !ready || attaching) return;
    const documentText = attachments
      .map((attachment) => {
        const name = attachment.name.replaceAll('"', '');
        const pages = attachment.pages ? ` pages="${attachment.pages}"` : '';
        const truncated = attachment.truncated ? ' truncated="true"' : '';
        // 正文里的 `</document>` 会提前闭合标签，让文档内容冒充成用户的话；转义掉。
        const body = attachment.text.replace(/<\/document/gi, '<\\/document');
        return `<document name="${name}"${pages}${truncated}>\n${body}\n</document>`;
      })
      .join('\n\n');
    const text = documentText ? `${documentText}\n\n${prompt}` : prompt;
    const current = settingsRef.current;
    const target = egressTarget(current);
    if (current.egress_acknowledged !== target) {
      setConsent(target);
      return;
    }
    busy(true);
    setError('');
    setLimitReached(false);
    let sent: PendingTurn<DocumentAttachment> | null = null;
    try {
      const page = app.current;
      const identity = currentApprovalSnapshot();
      await invoke('assistant_start', {
        projectRoot: page.root,
        kernelDir: guideState().kernelDir,
        resume: fresh.current ? null : session.current,
        caseDir: page.selectedCase?.entry.dir || null,
        modelRevision: page.versionRef,
      });
      if (fresh.current) {
        await invoke('assistant_new_session', {});
        fresh.current = false;
      }
      if (identity !== currentApprovalSnapshot())
        throw new Error(
          t(
            '启动期间页面或算例发生变化，请在当前页面重新发送。',
            'The page or case changed while starting. Send again from the current page.',
          ),
        );
      streamed.current = false;
      const shown = attachments.length
        ? `${prompt}\n\n${t('附件：', 'Attachments: ')}${attachments.map((item) => item.name).join('、')}`
        : prompt;
      setMessages((old) => [...old, { kind: 'user', text: shown }]);
      sent = beginSendTurn(typed, prompt);
      // U02：每条消息都带上当前历史范围（保留 guideState 的页面上下文）；范围改动会先落盘到设置。
      await invoke('assistant_send', {
        text,
        context: JSON.stringify({
          ...guideState(),
          history_scope: historySettingsOf(current).scope,
        }),
      });
      settleSendTurn(sent.id, true);
    } catch (e) {
      busy(false);
      // 发送本身失败（例如后端拒绝）：刚显示的气泡撤回；草稿的去留由轮次状态决定（GUI-18）。
      if (sent) settleSendTurn(sent.id, false, e);
      else setError(errorText(String(e)));
    }
  }
  async function decide(approve: boolean, remember = false) {
    if (!approval?.id || deciding) return;
    setDeciding(true);
    try {
      // The backend may emit ui_request before the invoke promise resolves.
      if (grant.current && grant.current.toolId === approval.id) grant.current.approved = approve;
      await invoke('assistant_approve', {
        id: approval.id,
        approve,
        note: approve ? null : t('用户拒绝', 'User declined'),
        remember,
      });
      setApproval(null);
    } catch (e) {
      if (grant.current && grant.current.toolId === approval.id) grant.current.approved = false;
      setError(String(e));
    } finally {
      setDeciding(false);
    }
  }
  /**
   * 用户滚动后更新"是否跟随底部"：滚动位置只能在有 DOM 时读取，判定交给 scroll.ts 的纯函数。
   */
  function trackScroll() {
    const element = log.current;
    if (!element) return;
    const next = followAfterScroll(
      { stick: stick.current, unread },
      {
        scrollTop: element.scrollTop,
        scrollHeight: element.scrollHeight,
        clientHeight: element.clientHeight,
      },
    );
    stick.current = next.stick;
    setUnread(next.unread);
  }
  /** "有新内容"按钮：回到底部并恢复跟随。 */
  function scrollToLatest() {
    const next = followAfterJump();
    stick.current = next.stick;
    setUnread(next.unread);
    log.current?.scrollTo({ top: log.current.scrollHeight });
  }
  /** 切换会话后重新从底部开始跟随；上一个会话的未读提示不能带过来。 */
  function resetFollow() {
    const next = followAfterJump();
    stick.current = next.stick;
    setUnread(next.unread);
  }
  /**
   * 关闭设置对话框（也用于"新对话/会话历史"先收起设置）：有未保存草稿时先确认放弃（U-07）。
   * 返回是否真的关闭了 —— 用户选择继续编辑时，调用方应中止后续切换。
   */
  async function closeSettings() {
    if (!state.assistantSettingsOpen) return true;
    if (settingsDirty) {
      const discard = await askConfirm(
        t(
          '助手设置还有未保存的修改，放弃这些修改并关闭？',
          'The assistant settings have unsaved changes. Discard them and close?',
        ),
        {
          confirmLabel: t('放弃修改', 'Discard'),
          cancelLabel: t('继续编辑', 'Keep editing'),
          danger: true,
        },
      );
      if (!discard) return false;
    }
    setSettingsDirty(false);
    update({ assistantSettingsOpen: false });
    return true;
  }
  /** 打开一个历史会话；请求带对话代次，迟到的 transcript 不写进当前会话（沿用 GUI-17 的代次约定）。 */
  async function openSession(item: Session) {
    const request = ++conversationRequest.current;
    const transcript = await invokeJson<Message[]>('assistant_transcript', { id: item.id });
    if (runningRef.current || request !== conversationRequest.current) return;
    session.current = item.id;
    fresh.current = false;
    setMessages(transcript);
    setContextUsage(null);
    setLimitReached(false);
    setStepLimit(0);
    setSessions(null);
    setAttachNotice('');
    // 用量是会话自己的：先清空并标"待加载"，下一次 usage 事件到达后再显示（U-07）。
    setUsage('');
    setSessionUsage({ turns: 0, tokens: 0 });
    setUsagePending(true);
    resetFollow();
  }
  const view = guideState();
  const builtin = settings.backend === 'builtin';
  const selectedModel = builtin ? settings.model : settings.external[settings.backend]?.model || '';
  const effort = builtin ? thinkValue(settings) : settings.external[settings.backend]?.effort || '';
  // U02：输入框上方显示当前后端/模型/冻结 revision 与历史范围（旧设置缺 history 时按 auto）。
  const historySettings = historySettingsOf(settings);
  const historyContext = historyContextLabel(
    settings,
    state.version,
    state.versionRef,
    state.language,
  );
  const efforts = builtin
    ? apiEfforts(providerId(settings), settings.model)
    : ['', 'low', 'medium', 'high', 'xhigh', 'max'];
  const contextWarning =
    builtin &&
    !!contextUsage &&
    contextUsage.window > 0 &&
    contextUsage.tokens >= contextUsage.window * CONTEXT_WARN_RATIO;
  const sessionWarning =
    settings.session_token_limit > 0 &&
    sessionUsage.tokens >= settings.session_token_limit * CONTEXT_WARN_RATIO;
  const continuePrompt = t('继续未完成的工作。', 'Continue the unfinished work.');
  return (
    <div className="flex h-full min-h-0 flex-col gap-3 border-l p-4">
      <header className="flex items-center justify-between">
        <strong>{t('AI 助手', 'AI assistant')}</strong>
        <Button
          variant="ghost"
          onClick={() => {
            if (state.assistantSettingsOpen) void closeSettings();
            else {
              setSettingsDirty(false);
              update({ assistantSettingsOpen: true });
            }
          }}
        >
          {t('设置', 'Settings')}
        </Button>
      </header>
      <div className="flex flex-wrap gap-2">
        <Button
          disabled={running}
          onClick={() => {
            void (async () => {
              // 设置面板开着时它占住了对话区，点了看不出变化（2026-10-22 用户反馈"无反应"）：先收起设置；
              // 有未保存草稿时先确认，用户选择继续编辑就不切换（U-07）。
              if (!(await closeSettings())) return;
              conversationRequest.current++;
              session.current = null;
              fresh.current = true;
              grant.current = null;
              setMessages([]);
              setApproval(null);
              setInput('');
              setAttachments([]);
              setUsage('');
              setSessionUsage({ turns: 0, tokens: 0 });
              setUsagePending(false);
              setContextUsage(null);
              setLimitReached(false);
              setStepLimit(0);
              setError('');
              setAttachNotice('');
              historyIndex.current = null;
              resetFollow();
            })();
          }}
        >
          {t('新对话', 'New conversation')}
        </Button>
        <Button
          disabled={!ready || running}
          onClick={() => {
            void (async () => {
              if (!(await closeSettings())) return;
              await perform(async () =>
                setSessions(
                  sessions && !state.assistantSettingsOpen
                    ? null
                    : await invokeJson<Session[]>('assistant_sessions', {}),
                ),
              );
            })();
          }}
        >
          {t('会话历史', 'Conversation history')}
        </Button>
      </div>
      {!hasBackend() && (
        <Notice>
          {t(
            '普通浏览器没有原生助手后端，无法发送、读取会话或登记 Key。请在桌面应用中使用。',
            'This browser has no native assistant backend. Use the desktop app to send messages, read conversations or register a key.',
          )}
        </Notice>
      )}
      {hasBackend() && initPhase === 'loading' && !ready && (
        <p className="text-xs text-muted-foreground" role="status">
          {t('正在读取助手设置…', 'Reading assistant settings…')}
        </p>
      )}
      {/* 设置读不出来时给出明确的重试入口，而不是让发送/附件按钮一直灰着（GUI-16）。 */}
      {hasBackend() && initPhase === 'failed' && (
        <Notice tone="error">
          <p>
            {t('助手设置没有读出来：', 'Could not read the assistant settings: ')}
            {initError}
          </p>
          <Button disabled={running} onClick={() => setInitAttempt((value) => value + 1)}>
            {t('重试读取设置', 'Retry loading settings')}
          </Button>
        </Notice>
      )}
      {/* 输入历史只是 ↑/↓ 记忆：读不到也照样能发送，只给一个可重试的警告（GUI-16）。 */}
      {historyError && (
        <Notice tone="warning">
          <p>
            {t(
              '读取输入历史失败，↑/↓ 暂时没有历史记录。',
              'Could not read the input history; ↑/↓ have no earlier inputs for now.',
            )}{' '}
            {historyError}
          </p>
          <Button disabled={running} onClick={() => setHistoryAttempt((value) => value + 1)}>
            {t('重试读取输入历史', 'Retry reading input history')}
          </Button>
        </Notice>
      )}
      {sessions && (
        <section
          aria-label={t('会话历史', 'Conversation history')}
          className="max-h-60 overflow-auto"
        >
          {sessions.length === 0 && <p>{t('没有历史会话。', 'No saved conversations.')}</p>}
          {sessions.map((item) => (
            <div key={item.id} className="flex gap-2">
              <Button disabled={running} onClick={() => void perform(() => openSession(item))}>
                {item.title} · {item.turns} {t('问', 'turns')}
              </Button>
              <Button
                disabled={running}
                variant="ghost"
                onClick={() =>
                  void perform(async () => {
                    // 删除会话连同对话、审计记录一起删掉，不能恢复：先确认（GUI 审查 D-05）。
                    const ok = await askConfirm(
                      t(
                        `删除会话"${item.title}"？对话记录会一起删掉，不能恢复。`,
                        `Delete the conversation "${item.title}"? Its messages are deleted too and cannot be restored.`,
                      ),
                      { confirmLabel: t('删除', 'Delete'), danger: true },
                    );
                    if (!ok) return;
                    await invoke('assistant_delete_session', { id: item.id });
                    setSessions((old) => old?.filter((value) => value.id !== item.id) || null);
                    if (session.current === item.id) {
                      session.current = null;
                      fresh.current = true;
                      conversationRequest.current++;
                      setMessages([]);
                      setAttachNotice('');
                      setUsage('');
                      setSessionUsage({ turns: 0, tokens: 0 });
                      setUsagePending(false);
                      resetFollow();
                    }
                    const inputs = await invoke<string[]>('assistant_input_history').catch(
                      () => null,
                    );
                    if (inputs) history.current = inputs;
                  })
                }
              >
                {t('删除', 'Delete')}
              </Button>
            </div>
          ))}
        </section>
      )}
      {state.assistantSettingsOpen && (
        <div className="max-h-[55vh] overflow-auto">
          <AssistantSettings
            settings={settings}
            onSave={save}
            disabled={running}
            onDirtyChange={setSettingsDirty}
            docsIndex={docsIndex}
          />
        </div>
      )}
      <p className="text-xs text-muted-foreground">
        {t('上下文：', 'Context: ')}
        {view.title} · {state.versionRef?.id}@{state.versionRef?.revision} ·{' '}
        {model || selectedModel || t('默认模型', 'Default model')}
      </p>
      <div className="relative flex min-h-32 flex-1 flex-col">
        <div
          ref={log}
          role="log"
          aria-label={t('助手对话', 'Assistant conversation')}
          className="min-h-0 flex-1 space-y-3 overflow-auto"
          onScroll={trackScroll}
        >
          {messages.length === 0 && (
            <section className="space-y-2">
              {hasBackend() && settings.backend === 'builtin' && hasKey === false && (
                <Notice>
                  <p>
                    {t(
                      '这个服务还没有登记 API Key；登记后才能发送，联网搜索也需要 DeepSeek Key。',
                      'No API key is registered for this service yet; register one before sending. Web search also needs a DeepSeek key.',
                    )}
                  </p>
                  <Button
                    onClick={() => {
                      setSettingsDirty(false);
                      update({ assistantSettingsOpen: true });
                    }}
                  >
                    {t('去设置登记 Key', 'Register a key in Settings')}
                  </Button>
                </Notice>
              )}
              <p>
                {t(
                  '有四个内置站点示例，要不要试试？',
                  'Try one of our four bundled site examples.',
                )}
              </p>
              <div className="grid grid-cols-2 gap-2">
                {EXAMPLE_STARTERS.map((example) => (
                  <Button
                    key={example.id}
                    title={state.language === 'en' ? example.en : example.zh}
                    className="min-w-0 text-xs"
                    disabled={running}
                    onClick={() => {
                      const prompt = examplePrompt(example.id, state.language);
                      setInput((value) => (value.trim() ? `${value}\n${prompt}` : prompt));
                      historyIndex.current = null;
                    }}
                  >
                    {example.id}
                  </Button>
                ))}
              </div>
              <Button disabled={running} onClick={() => navigate('new-case')}>
                {t('查看四个示例', 'Browse the four examples')}
              </Button>
              <p className="text-sm text-muted-foreground">
                {t('点示例只填写问题，发送前可修改。', 'Examples draft an editable question.')}
              </p>
            </section>
          )}
          {messages.map((item, index) =>
            item.kind === 'tool' ? (
              <details key={index} className="rounded border p-2">
                <summary>
                  {item.name} ·{' '}
                  {item.result == null
                    ? item.text || t('未完成', 'Incomplete')
                    : item.ok
                      ? t('完成', 'Completed')
                      : t('失败', 'Failed')}
                </summary>
                <pre className="whitespace-pre-wrap break-all text-xs">{item.arguments}</pre>
                {item.result != null && (
                  <pre className="whitespace-pre-wrap break-all text-xs">{item.result}</pre>
                )}
              </details>
            ) : item.kind === 'reasoning' ? (
              <details key={index}>
                <summary>{t('思考过程', 'Reasoning')}</summary>
                <p className="whitespace-pre-wrap text-sm">{item.text}</p>
              </details>
            ) : item.kind === 'task' ? (
              <p key={index} className="text-xs">
                {String(item.task?.goal || '')} ·{' '}
                {String(item.task?.phase || item.task?.state || '')}
              </p>
            ) : (
              <article
                key={index}
                className={`break-words rounded p-2 ${item.kind === 'user' ? 'whitespace-pre-wrap bg-muted' : ''}`}
              >
                <span className="text-xs text-muted-foreground">
                  {item.kind === 'user' ? t('你', 'You') : t('助手', 'Assistant')}
                </span>
                {/* 用户消息里可能有附件的 <document> 原文，永远按纯文本显示；Markdown 只用于助手回复。 */}
                <div>
                  {item.kind === 'assistant' ? (
                    <Markdown text={item.text || ''} t={t} />
                  ) : (
                    item.text
                  )}
                </div>
              </article>
            ),
          )}
        </div>
        {/* 暂停跟随时把"有新内容"放在对话区底部，不抢走键盘焦点、也不重新排对话。 */}
        {unread && (
          <Button
            variant="primary"
            style={{
              position: 'absolute',
              bottom: 8,
              left: '50%',
              transform: 'translateX(-50%)',
            }}
            onClick={scrollToLatest}
          >
            {t('有新内容，回到底部', 'New content — jump to the latest')}
          </Button>
        )}
      </div>
      {approval && (
        <section
          aria-label={t('工具审批', 'Tool approval')}
          role="alert"
          aria-live="assertive"
          className="space-y-2 rounded border p-3"
        >
          <strong>{t('需要你的批准', 'Your approval is required')}</strong>
          {/* 摘要写的是这次具体要做什么（后端用中文写）；英文界面也照样给出，只看工具名等于盲批。 */}
          {state.language === 'en' && <p>{`${approval.name || 'tool'} requires your approval`}</p>}
          {approval.summary && <p>{approval.summary}</p>}
          <pre className="max-h-40 overflow-auto whitespace-pre-wrap text-xs">
            {approval.arguments}
          </pre>
          <div className="flex flex-wrap gap-2">
            <Button disabled={deciding} onClick={() => void decide(false)}>
              {t('拒绝', 'Decline')}
            </Button>
            {/* 不自动聚焦"批准"：审批卡在用户打字时弹出，一个回车就会批准（DeepSeek 改动复查）。
                卡片本身是 role="alert"，读屏软件照样会播报。 */}
            <Button disabled={deciding} onClick={() => void decide(true)}>
              {t('批准', 'Approve')}
            </Button>
            {!approval.explicit_only && (
              <Button disabled={deciding} onClick={() => void decide(true, true)}>
                {t('本会话允许同类操作', 'Allow similar operations this session')}
              </Button>
            )}
          </div>
        </section>
      )}
      {consent && (
        <section className="rounded border p-3">
          <p>
            {t(
              '本次输入与当前页面上下文将发送至',
              'Your message and current page context will be sent to',
            )}{' '}
            {consent}. {t('确认使用该服务？', 'Use this service?')}
          </p>
          {settings.backend === 'builtin' && settings.web_search && (
            <p className="text-xs text-muted-foreground">
              {t(
                // 读网页先在本机试，读不了（JS 页面、被站点拦截等）会交给 DeepSeek Fetch：网址和网页内容都经过 DeepSeek。
                // 原来写"网页内容在本机读取"，和回退行为矛盾（GUI 审查 D-28）。
                '联网搜索的查询会发往 DeepSeek Search API（api.deepseek.com）；读网页先在本机读取，读不了时会把网址交给 DeepSeek Fetch API，网页内容也会经过 DeepSeek。',
                'Web search queries go to the DeepSeek Search API (api.deepseek.com); pages are read on this computer first, and when that fails the URL goes to the DeepSeek Fetch API, so the page content also passes through DeepSeek.',
              )}
            </p>
          )}
          <Button onClick={() => setConsent(null)}>{t('取消', 'Cancel')}</Button>
          <Button
            onClick={() =>
              void perform(async () => {
                await save({ ...settings, egress_acknowledged: consent });
                setConsent(null);
              })
            }
          >
            {t('同意，随后可发送', 'Agree, then send')}
          </Button>
        </section>
      )}
      {error && <Notice tone="error">{error}</Notice>}
      {attachNotice && <Notice tone="warning">{attachNotice}</Notice>}
      {attachments.length > 0 && (
        <div className="flex flex-col gap-1" aria-label={t('已附加的文档', 'Attached documents')}>
          {attachments.map((attachment, index) => (
            <div
              key={`${attachment.name}-${index}`}
              className="flex items-center gap-2 rounded border px-2 py-1 text-xs"
            >
              <span className="min-w-0 flex-1 truncate" title={attachment.name}>
                {attachment.name}
              </span>
              <span className="shrink-0 text-muted-foreground">
                {attachment.pages
                  ? t(`${attachment.pages} 页`, `${attachment.pages} pages`)
                  : t('文字', 'text')}
                {attachment.truncated ? t('（已截断）', ' (truncated)') : ''}
              </span>
              <Button
                variant="ghost"
                aria-label={t(`移除 ${attachment.name}`, `Remove ${attachment.name}`)}
                disabled={running}
                onClick={() => setAttachments((old) => old.filter((_item, at) => at !== index))}
              >
                ×
              </Button>
            </div>
          ))}
        </div>
      )}
      {limitReached && (
        <Notice tone="warning">
          <p>
            {t(
              `本轮达到 ${stepLimit || 40} 次生成上限，任务未完成。已有操作和结果都保留；继续会在同一会话里接着做，不会清空前面的内容。`,
              `This turn reached its ${stepLimit || 40}-generation limit before completion. Existing actions and results are kept; Continue resumes in the same conversation without clearing earlier content.`,
            )}
          </p>
          <Button disabled={running || !ready} onClick={() => void send(continuePrompt)}>
            {t('继续', 'Continue')}
          </Button>
        </Notice>
      )}
      {contextWarning && (
        <Notice tone="warning">
          {t(
            `当前上下文估算约 ${tokenLabel(contextUsage!.tokens)} / ${tokenLabel(contextUsage!.window)} tokens${
              contextUsage!.available !== null
                ? `（可用输入估算 ${tokenLabel(contextUsage!.available)}）`
                : ''
            }，已接近模型窗口；下一步会自动折叠较早的工具结果。`,
            `Estimated context is about ${tokenLabel(contextUsage!.tokens)} / ${tokenLabel(contextUsage!.window)} tokens${
              contextUsage!.available !== null
                ? ` (available input estimate ${tokenLabel(contextUsage!.available)})`
                : ''
            }, near the model window; the next step will automatically fold older tool results.`,
          )}
        </Notice>
      )}
      {!!contextUsage?.compressed && (
        <p className="text-xs text-muted-foreground">
          {t(
            `已自动折叠 ${contextUsage.compressed} 条较早的工具结果；原始记录仍在本机会话文件里，界面历史不清空。`,
            `${contextUsage.compressed} older tool results were folded automatically; the original records stay in the local session file and the visible history is not cleared.`,
          )}
        </p>
      )}
      {sessionWarning && (
        <Notice tone="warning">
          {t(
            `本会话已用约 ${tokenLabel(sessionUsage.tokens)} / ${tokenLabel(settings.session_token_limit)} tokens，接近会话预算；压缩只影响上下文，不会退还已用 token。`,
            `This conversation has used about ${tokenLabel(sessionUsage.tokens)} / ${tokenLabel(settings.session_token_limit)} tokens and is near its budget; compression only reduces context and does not refund spent tokens.`,
          )}
        </Notice>
      )}
      {/* U02：历史范围提示/选择器。选择落在 AssistantSettings.history.scope，随每条消息的 context 下发。 */}
      <div className="assistant-history-scope flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
        <label className="flex items-center gap-1">
          <span>{t('历史范围：', 'History scope: ')}</span>
          <Select
            aria-label={t('只读历史范围', 'Read-only history scope')}
            value={historySettings.scope}
            disabled={!ready || running || scopeBusy || state.assistantSettingsOpen}
            onChange={(e) => void changeHistoryScope(e.target.value as HistoryScope)}
          >
            {(['auto', 'selected', 'desktop', 'upstream', 'both'] as HistoryScope[]).map(
              (scope) => (
                <option key={scope} value={scope}>
                  {historyScopeLabel(scope, state.language)}
                </option>
              ),
            )}
          </Select>
        </label>
        <span>
          {t('当前：', 'Current: ')}
          {historyContext.backend} · {model || historyContext.model} · {historyContext.revision}
        </span>
        {scopeBusy && <span>{t('正在保存范围…', 'Saving scope…')}</span>}
      </div>
      <p className="text-xs text-muted-foreground">{historyScopeHint(state.language)}</p>
      <textarea
        aria-label={t('给助手的消息', 'Message to assistant')}
        placeholder={t(
          '输入问题；↑ / ↓ 选择历史输入',
          'Ask a question; ↑ / ↓ browse input history',
        )}
        className="min-h-24 w-full rounded border bg-background p-2"
        value={input}
        onChange={(e) => {
          setInput(e.target.value);
          historyIndex.current = null;
        }}
        onKeyDown={(event) => {
          if (
            !event.nativeEvent.isComposing &&
            ((event.key === 'ArrowUp' && event.currentTarget.selectionStart === 0) ||
              (event.key === 'ArrowDown' && event.currentTarget.selectionEnd === input.length))
          ) {
            if (!history.current.length) return;
            event.preventDefault();
            if (historyIndex.current === null) historyDraft.current = input;
            const next = historyInput(
              history.current,
              historyIndex.current,
              historyDraft.current,
              event.key === 'ArrowUp' ? -1 : 1,
            );
            historyIndex.current = next.index;
            setInput(next.value);
          }
          if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) {
            event.preventDefault();
            void send();
          }
        }}
      />
      {/* 附件用图标，发送/停止靠右：输入框下面一行只留三个动作，模型快捷选择在下一行。 */}
      <div className="flex flex-wrap items-center gap-2">
        <Button
          variant="ghost"
          className="assistant-attach-button"
          aria-label={
            attaching ? t('正在读取文档', 'Reading document') : t('添加文档', 'Attach document')
          }
          aria-busy={attaching}
          title={
            attaching ? t('正在读取文档…', 'Reading document…') : t('添加文档', 'Attach document')
          }
          disabled={!ready || running || attaching || state.assistantSettingsOpen}
          onClick={() => void perform(attachDocument)}
        >
          <svg
            aria-hidden="true"
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.8"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.82-2.82l8.49-8.48" />
          </svg>
        </Button>
        <div className="ml-auto flex gap-2">
          <Button
            variant="primary"
            disabled={!ready || running || attaching || (!input.trim() && !attachments.length)}
            onClick={() => void send()}
          >
            {t('发送', 'Send')}
          </Button>
          <Button
            disabled={!running}
            onClick={() =>
              void perform(async () => {
                await invoke('assistant_cancel', {});
              })
            }
          >
            {t('停止', 'Stop')}
          </Button>
        </div>
      </div>
      <div className="assistant-choices">
        <label>
          <span>{t('模型', 'Model')}</span>
          <Select
            aria-label={t('助手模型', 'Assistant model')}
            title={
              state.assistantSettingsOpen
                ? t(
                    '先保存或关闭设置，再切换模型',
                    'Save or close Settings before changing the model',
                  )
                : undefined
            }
            disabled={running || !ready || state.assistantSettingsOpen}
            value={selectedModel}
            onChange={(e) =>
              void perform(() => save(selectAssistantModel(settings, e.target.value)))
            }
          >
            {assistantModels(settings, model).map((value) => (
              <option key={value} value={value}>
                {value || t('默认模型', 'Default model')}
              </option>
            ))}
          </Select>
        </label>
        <label>
          <span>{t('思考程度', 'Reasoning')}</span>
          <Select
            aria-label={t('即时思考强度', 'Reasoning effort')}
            title={
              state.assistantSettingsOpen
                ? t(
                    '先保存或关闭设置，再切换思考强度',
                    'Save or close Settings before changing reasoning effort',
                  )
                : undefined
            }
            disabled={running || !ready || state.assistantSettingsOpen}
            value={effort}
            onChange={(e) =>
              void perform(() =>
                save(
                  builtin
                    ? { ...settings, ...thinkSettings(e.target.value) }
                    : withChoice(settings, { effort: e.target.value }),
                ),
              )
            }
          >
            {Array.from(new Set([...efforts, effort])).map((value) => (
              <option key={value} value={value}>
                {value || t('默认思考', 'Default reasoning')}
              </option>
            ))}
          </Select>
        </label>
      </div>
      <p className="text-xs text-muted-foreground" role="status">
        {running
          ? t('正在处理…', 'Working…')
          : usagePending
            ? t(
                '用量待加载；发送下一条后会显示本会话的用量。',
                "Usage is loading; it will show this conversation's numbers after the next reply.",
              )
            : sessionUsage.turns || sessionUsage.tokens
              ? `${t('本会话', 'This conversation')}: ${[
                  sessionUsage.turns && `${sessionUsage.turns} ${t('轮', 'turns')}`,
                  `${sessionUsage.tokens} tokens`,
                  settings.session_turn_limit || settings.session_token_limit
                    ? [
                        settings.session_turn_limit &&
                          `${sessionUsage.turns}/${settings.session_turn_limit}`,
                        settings.session_token_limit &&
                          `${sessionUsage.tokens}/${settings.session_token_limit}`,
                      ]
                        .filter(Boolean)
                        .join(' · ')
                    : '',
                ]
                  .filter(Boolean)
                  .join(' · ')}`
              : usage}
        {builtin &&
          contextUsage &&
          ` · ${t('上下文估算', 'Context estimate')}: ${tokenLabel(contextUsage.tokens)} / ${tokenLabel(contextUsage.window)} tokens${
            contextUsage.available !== null
              ? ` · ${t('可用输入估算', 'available input estimate')} ${tokenLabel(contextUsage.available)}`
              : ''
          }${
            contextUsage.source === 'user'
              ? t('（窗口：已填写）', ' (window: set)')
              : contextUsage.source === 'preset'
                ? t('（窗口：已确认预设）', ' (window: documented preset)')
                : ''
          }`}
      </p>
    </div>
  );
}

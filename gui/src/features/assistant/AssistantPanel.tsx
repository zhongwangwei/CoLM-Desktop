import { useEffect, useRef, useState } from 'react';
import { invoke, invokeJson, listen, hasBackend } from '../../lib/ipc';
import { useApp } from '../../lib/app-state';
import { askConfirm } from '../../lib/confirm';
import { guideState, drive } from '../../lib/guide';
import { Button, Select, Notice } from '../../components/ui';
import AssistantSettings from './Settings';
import {
  appendDelta,
  apiEfforts,
  defaultSettings,
  egressTarget,
  historyInput,
  providerId,
  assistantModels,
  selectAssistantModel,
  thinkSettings,
  thinkValue,
  withChoice,
  EXAMPLE_STARTERS,
  examplePrompt,
  rejectedBeforeTurn,
  type AgentEvent,
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
  // 刚发出、助手还没开始回应的那条消息：被拒绝时用来恢复输入框与附件（D-06）。
  const pendingSend = useRef<{ typed: string; attachments: DocumentAttachment[] } | null>(null);
  const [attaching, setAttaching] = useState(false);
  const [error, setError] = useState('');
  const [sessions, setSessions] = useState<Session[] | null>(null);
  const [approval, setApproval] = useState<AgentEvent | null>(null);
  const [deciding, setDeciding] = useState(false);
  const [consent, setConsent] = useState<string | null>(null);
  const [usage, setUsage] = useState('');
  const [sessionUsage, setSessionUsage] = useState({ turns: 0, tokens: 0 });
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
    if (messages.length) log.current?.scrollTo({ top: log.current.scrollHeight });
  }, [messages, approval]);
  useEffect(() => {
    if (!hasBackend()) return;
    let active = true;
    let unlisten: (() => void) | undefined;
    const handler = (event: AgentEvent) => {
      // 助手开始回应（任何输出）后，这条消息就算被接下了，不再恢复输入框。
      if (event.type !== 'error' && event.type !== 'ready' && event.type !== 'exited')
        pendingSend.current = null;
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
          break;
        case 'turn_done':
          if (!streamed.current && event.content)
            setMessages((old) => [...old, { kind: 'assistant', text: event.content }]);
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
          const sent = pendingSend.current;
          pendingSend.current = null;
          if (sent && rejectedBeforeTurn(event.message || '')) {
            setMessages((old) => {
              const at = old.map((m) => m.kind).lastIndexOf('user');
              return at >= 0 ? [...old.slice(0, at), ...old.slice(at + 1)] : old;
            });
            setInput((value) => value || sent.typed);
            setAttachments((items) => (items.length ? items : sent.attachments));
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
        const loaded = await invoke<Settings>('assistant_settings', {});
        const inputs = await invoke<string[]>('assistant_input_history', {});
        if (active) {
          setSettings(loaded);
          history.current = inputs;
          setReady(true);
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
  }, [state.language]);
  async function save(next: Settings) {
    if (runningRef.current)
      throw new Error(
        t('请先停止当前回答再修改设置。', 'Stop the current response before changing settings.'),
      );
    await invoke('assistant_save_settings', { settings: next });
    setSettings(next);
    setModel('');
  }
  async function attachDocument() {
    if (runningRef.current || !ready) return;
    try {
      setAttaching(true);
      // 选文件和读文件都在后端：前端不能拿一个任意路径去读（例如存 Key 的 JSON）再发给模型。
      const picked = await invokeJson<DocumentAttachment | null>('assistant_pick_document');
      if (!picked) return;
      if (!picked.text.trim())
        throw new Error(t('这个文件没有可读文字。', 'This file has no readable text.'));
      const used = attachments.reduce((total, item) => total + item.text.length, 0);
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
      setError(attachmentErrorText(String(e)));
    } finally {
      setAttaching(false);
    }
  }
  async function send() {
    const typed = input.trim();
    const fallback = attachments.length
      ? t(
          '请阅读这些附件并总结要点。',
          'Please read the attached documents and summarize the key points.',
        )
      : '';
    const prompt = typed || fallback;
    if (!prompt || runningRef.current || !ready) return;
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
      pendingSend.current = { typed, attachments };
      await invoke('assistant_send', { text, context: JSON.stringify(guideState()) });
      setInput((value) => (value.trim() === typed ? '' : value));
      setAttachments([]);
      if (typed && history.current.at(-1) !== typed)
        history.current = [...history.current, typed].slice(-100);
      else if (!typed && history.current.at(-1) !== prompt)
        history.current = [...history.current, prompt].slice(-100);
      historyIndex.current = null;
      historyDraft.current = '';
    } catch (e) {
      busy(false);
      // 发送本身失败（例如后端拒绝）：刚显示的气泡撤回，输入与附件本来就还在。
      if (pendingSend.current) {
        pendingSend.current = null;
        setMessages((old) => {
          const at = old.map((m) => m.kind).lastIndexOf('user');
          return at >= 0 ? [...old.slice(0, at), ...old.slice(at + 1)] : old;
        });
      }
      setError(errorText(String(e)));
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
  const view = guideState();
  const builtin = settings.backend === 'builtin';
  const selectedModel = builtin ? settings.model : settings.external[settings.backend]?.model || '';
  const effort = builtin ? thinkValue(settings) : settings.external[settings.backend]?.effort || '';
  const efforts = builtin
    ? apiEfforts(providerId(settings), settings.model)
    : ['', 'low', 'medium', 'high', 'xhigh', 'max'];
  return (
    <div className="flex h-full min-h-0 flex-col gap-3 border-l p-4">
      <header className="flex items-center justify-between">
        <strong>{t('AI 助手', 'AI assistant')}</strong>
        <Button
          variant="ghost"
          onClick={() => update({ assistantSettingsOpen: !state.assistantSettingsOpen })}
        >
          {t('设置', 'Settings')}
        </Button>
      </header>
      <div className="flex flex-wrap gap-2">
        <Button
          disabled={running}
          onClick={() => {
            // 设置面板开着时它占住了对话区，点了看不出变化（2026-10-22 用户反馈"无反应"）：先收起设置。
            if (state.assistantSettingsOpen) update({ assistantSettingsOpen: false });
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
            setError('');
            historyIndex.current = null;
          }}
        >
          {t('新对话', 'New conversation')}
        </Button>
        <Button
          disabled={!ready || running}
          onClick={() => {
            if (state.assistantSettingsOpen) update({ assistantSettingsOpen: false });
            void perform(async () =>
              setSessions(
                sessions && !state.assistantSettingsOpen
                  ? null
                  : await invokeJson<Session[]>('assistant_sessions', {}),
              ),
            );
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
      {sessions && (
        <section
          aria-label={t('会话历史', 'Conversation history')}
          className="max-h-60 overflow-auto"
        >
          {sessions.length === 0 && <p>{t('没有历史会话。', 'No saved conversations.')}</p>}
          {sessions.map((item) => (
            <div key={item.id} className="flex gap-2">
              <Button
                disabled={running}
                onClick={() =>
                  void perform(async () => {
                    const request = ++conversationRequest.current;
                    const transcript = await invokeJson<Message[]>('assistant_transcript', {
                      id: item.id,
                    });
                    if (runningRef.current || request !== conversationRequest.current) return;
                    session.current = item.id;
                    fresh.current = false;
                    setMessages(transcript);
                    setSessions(null);
                  })
                }
              >
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
                      setMessages([]);
                      setUsage('');
                      setSessionUsage({ turns: 0, tokens: 0 });
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
          <AssistantSettings settings={settings} onSave={save} disabled={running} />
        </div>
      )}
      <p className="text-xs text-muted-foreground">
        {t('上下文：', 'Context: ')}
        {view.title} · {state.versionRef?.id}@{state.versionRef?.revision} ·{' '}
        {model || selectedModel || t('默认模型', 'Default model')}
      </p>
      <div
        ref={log}
        role="log"
        aria-label={t('助手对话', 'Assistant conversation')}
        className="min-h-32 flex-1 space-y-3 overflow-auto"
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
                <Button onClick={() => update({ assistantSettingsOpen: true })}>
                  {t('去设置登记 Key', 'Register a key in Settings')}
                </Button>
              </Notice>
            )}
            <p>
              {t('有四个内置站点示例，要不要试试？', 'Try one of our four bundled site examples.')}
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
              {String(item.task?.goal || '')} · {String(item.task?.phase || item.task?.state || '')}
            </p>
          ) : (
            <article
              key={index}
              className={`whitespace-pre-wrap break-words rounded p-2 ${item.kind === 'user' ? 'bg-muted' : ''}`}
            >
              <span className="text-xs text-muted-foreground">
                {item.kind === 'user' ? t('你', 'You') : t('助手', 'Assistant')}
              </span>
              <div>{item.text}</div>
            </article>
          ),
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
                '其中联网搜索的查询会发往 DeepSeek Search API（api.deepseek.com）；网页内容在本机读取。',
                'Web search queries are also sent to the DeepSeek Search API (api.deepseek.com); pages are read on this computer.',
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
      {/* 添加文档、发送、停止紧跟在输入框下面；模型与思考程度的快捷选择放到它们后面（用户指出按钮离输入框太远）。 */}
      <div className="flex flex-wrap gap-2">
        <Button
          variant="ghost"
          disabled={!ready || running || attaching || state.assistantSettingsOpen}
          onClick={() => void perform(attachDocument)}
        >
          {attaching ? t('读取中…', 'Reading…') : t('添加文档', 'Attach document')}
        </Button>
        <Button
          variant="primary"
          disabled={!ready || running || (!input.trim() && !attachments.length)}
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
      </p>
    </div>
  );
}

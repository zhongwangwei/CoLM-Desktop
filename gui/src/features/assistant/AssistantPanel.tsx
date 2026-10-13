import { useEffect, useRef, useState } from 'react';
import { invoke, invokeJson, listen, hasBackend } from '../../lib/ipc';
import { useApp } from '../../lib/app-state';
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
export default function AssistantPanel() {
  const { state, navigate, update } = useApp();
  const app = useRef(state);
  app.current = state;
  function t(zh: string, en: string) {
    return app.current.language === 'en' ? en : zh;
  }
  const [settings, setSettings] = useState<Settings>(defaultSettings);
  const settingsRef = useRef(settings);
  settingsRef.current = settings;
  const [messages, setMessages] = useState<Message[]>([]);
  const [running, setRunning] = useState(false);
  const runningRef = useRef(false);
  const [input, setInput] = useState('');
  const [error, setError] = useState('');
  const [sessions, setSessions] = useState<Session[] | null>(null);
  const [approval, setApproval] = useState<AgentEvent | null>(null);
  const [deciding, setDeciding] = useState(false);
  const [consent, setConsent] = useState<string | null>(null);
  const [usage, setUsage] = useState('');
  const [model, setModel] = useState('');
  const [ready, setReady] = useState(false);
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
      setError(String(e));
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
          break;
        case 'turn_done':
          if (!streamed.current && event.content)
            setMessages((old) => [...old, { kind: 'assistant', text: event.content }]);
          busy(false);
          grant.current = null;
          setApproval(null);
          break;
        case 'error':
          busy(false);
          grant.current = null;
          setApproval(null);
          setError(
            event.message === 'cancelled'
              ? t('已停止。', 'Stopped.')
              : event.message || t('助手错误', 'Assistant error'),
          );
          break;
        case 'exited':
          busy(false);
          grant.current = null;
          setApproval(null);
          setError(
            t(
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
  }, []);
  async function save(next: Settings) {
    if (runningRef.current)
      throw new Error(
        t('请先停止当前回答再修改设置。', 'Stop the current response before changing settings.'),
      );
    await invoke('assistant_save_settings', { settings: next });
    setSettings(next);
    setModel('');
  }
  async function send() {
    const text = input.trim();
    if (!text || runningRef.current || !ready) return;
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
      setMessages((old) => [...old, { kind: 'user', text }]);
      await invoke('assistant_send', { text, context: JSON.stringify(guideState()) });
      setInput((value) => (value.trim() === text ? '' : value));
      if (history.current.at(-1) !== text) history.current = [...history.current, text].slice(-100);
      historyIndex.current = null;
      historyDraft.current = '';
    } catch (e) {
      busy(false);
      setError(String(e));
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
  const builtin = settings.backend === 'builtin';
  const selectedModel = builtin ? settings.model : settings.external[settings.backend]?.model || '';
  const effort = builtin ? thinkValue(settings) : settings.external[settings.backend]?.effort || '';
  const efforts = builtin
    ? apiEfforts(providerId(settings), settings.model)
    : ['', 'low', 'medium', 'high', 'xhigh', 'max'];
  return (
    <aside
      aria-label={t('AI 助手', 'AI assistant')}
      className="flex h-full min-h-0 flex-col gap-3 border-l p-4"
    >
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
            conversationRequest.current++;
            session.current = null;
            fresh.current = true;
            grant.current = null;
            setMessages([]);
            setApproval(null);
            setInput('');
            setUsage('');
            setError('');
            historyIndex.current = null;
          }}
        >
          {t('新对话', 'New conversation')}
        </Button>
        <Button
          disabled={!ready || running}
          onClick={() =>
            void perform(async () =>
              setSessions(sessions ? null : await invokeJson<Session[]>('assistant_sessions', {})),
            )
          }
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
                    await invoke('assistant_delete_session', { id: item.id });
                    setSessions((old) => old?.filter((value) => value.id !== item.id) || null);
                    if (session.current === item.id) {
                      session.current = null;
                      fresh.current = true;
                      setMessages([]);
                    }
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
        {t('上下文：当前可见页面', 'Context: currently visible page')} · {state.versionRef?.id}@
        {state.versionRef?.revision} · {model || selectedModel || t('默认模型', 'Default model')}
      </p>
      <div
        ref={log}
        role="log"
        aria-label={t('助手对话', 'Assistant conversation')}
        className="min-h-32 flex-1 space-y-3 overflow-auto"
      >
        {messages.length === 0 && (
          <section className="space-y-2">
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
          className="space-y-2 rounded border p-3"
        >
          <strong>{t('需要你的批准', 'Your approval is required')}</strong>
          <p>{approval.summary}</p>
          <pre className="max-h-40 overflow-auto whitespace-pre-wrap text-xs">
            {approval.arguments}
          </pre>
          <div className="flex flex-wrap gap-2">
            <Button disabled={deciding} onClick={() => void decide(false)}>
              {t('拒绝', 'Decline')}
            </Button>
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
            (event.key === 'ArrowUp' && event.currentTarget.selectionStart === 0) ||
            (event.key === 'ArrowDown' && event.currentTarget.selectionEnd === input.length)
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
      <div className="assistant-choices">
        <label>
          <span>{t('模型', 'Model')}</span>
          <Select
            aria-label={t('助手模型', 'Assistant model')}
            disabled={running || !ready}
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
            disabled={running || !ready}
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
      <div className="flex flex-wrap gap-2">
        <Button
          variant="primary"
          disabled={!ready || running || !input.trim()}
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
      <p className="text-xs text-muted-foreground" role="status">
        {running ? t('正在处理…', 'Working…') : usage}
      </p>
    </aside>
  );
}

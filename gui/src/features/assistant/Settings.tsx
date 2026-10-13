import { useEffect, useRef, useState } from 'react';
import { invoke, invokeJson, hasBackend } from '../../lib/ipc';
import { useApp } from '../../lib/app-state';
import { Button, Input, Select, Field, Advanced, Notice } from '../../components/ui';
import {
  API_PROVIDERS,
  providerLabel,
  apiEfforts,
  parseApiOptions,
  providerId,
  selectProvider,
  rememberProfile,
  settingsForSave,
  thinkValue,
  thinkSettings,
  withChoice,
  type Settings,
  type ModelChoice,
} from './model';

export default function AssistantSettings({
  settings,
  onSave,
  disabled,
}: {
  settings: Settings;
  onSave: (next: Settings) => Promise<void>;
  disabled: boolean;
}) {
  const { state } = useApp();
  const t = (zh: string, en: string) => (state.language === 'en' ? en : zh);
  const lock = useRef(false);
  const [draft, setDraft] = useState(settings);
  const [options, setOptions] = useState(JSON.stringify(settings.api_options, null, 2));
  const [key, setKey] = useState('');
  const [hasKey, setHasKey] = useState<boolean | null>(null);
  const [models, setModels] = useState<ModelChoice[]>([]);
  const [status, setStatus] = useState('');
  const [busy, setBusy] = useState(false);
  const request = useRef(0);
  useEffect(() => {
    setDraft(settings);
    setOptions(JSON.stringify(settings.api_options, null, 2));
  }, [settings]);
  useEffect(() => {
    let active = true;
    const sourceRequest = request;
    setHasKey(null);
    setModels([]);
    request.current++;
    if (hasBackend() && draft.backend === 'builtin')
      invoke<boolean>('assistant_has_key', { baseUrl: draft.base_url })
        .then((value) => {
          if (active) setHasKey(value);
        })
        .catch((error) => {
          if (active) setStatus(String(error));
        });
    return () => {
      active = false;
      sourceRequest.current++;
    };
  }, [draft.backend, draft.base_url]);
  function patch(value: Partial<Settings>) {
    setDraft((old) => ({ ...old, ...value }));
  }
  async function run(action: () => Promise<void>) {
    if (lock.current) return;
    lock.current = true;
    setBusy(true);
    setStatus('');
    try {
      await action();
    } catch (error) {
      setStatus(String(error));
    } finally {
      lock.current = false;
      setBusy(false);
    }
  }
  async function discover() {
    const sequence = ++request.current;
    const found =
      draft.backend === 'builtin'
        ? (
            await invoke<string[]>('assistant_api_models', {
              baseUrl: draft.base_url,
              apiFormat: draft.api_format,
            })
          ).map((id) => ({ id }))
        : draft.backend === 'codex'
          ? await invokeJson<ModelChoice[]>('assistant_codex_models', {})
          : draft.backend === 'opencode'
            ? await invokeJson<ModelChoice[]>('assistant_opencode_models', {})
            : [{ id: 'fable' }, { id: 'opus' }, { id: 'sonnet' }, { id: 'haiku' }];
    if (sequence === request.current) {
      setModels(found);
      setStatus(
        `${t('可用模型', 'Available models')}: ${found.length} (${t('保留手动输入', 'manual entry retained')})`,
      );
    }
  }
  const builtin = draft.backend === 'builtin';
  const model = builtin ? draft.model : draft.external[draft.backend]?.model || '';
  const effort = builtin ? thinkValue(draft) : draft.external[draft.backend]?.effort || '';
  const efforts = builtin
    ? apiEfforts(providerId(draft), model)
    : [
        '',
        ...(models.find((item) => item.id === model)?.efforts ||
          (draft.backend === 'opencode' ? [] : ['low', 'medium', 'high', 'xhigh', 'max'])),
      ];
  return (
    <section aria-label={t('助手设置', 'Assistant settings')} className="space-y-3">
      <fieldset disabled={disabled || busy || !hasBackend()} className="space-y-3">
        <Field label={t('运行后端', 'Backend')}>
          <Select
            value={draft.backend}
            onChange={(e) => {
              if (key) {
                setStatus(
                  t(
                    '请先登记或清空 Key 再切换服务。',
                    'Register or clear the key before switching services.',
                  ),
                );
                return;
              }
              patch({ backend: e.target.value });
            }}
          >
            <option value="builtin">{t('API 服务', 'API service')}</option>
            <option value="codex">{t('本机 Codex', 'Local Codex')}</option>
            <option value="claude_code">{t('本机 Claude Code', 'Local Claude Code')}</option>
            <option value="opencode">{t('本机 OpenCode', 'Local OpenCode')}</option>
          </Select>
        </Field>
        {builtin && (
          <>
            <Field label={t('服务商', 'Provider')}>
              <Select
                value={providerId(draft)}
                onChange={(e) => {
                  try {
                    if (key)
                      throw new Error(
                        t(
                          '请先登记或清空 Key 再切换服务。',
                          'Register or clear the key before switching services.',
                        ),
                      );
                    const next = selectProvider(
                      { ...draft, api_options: parseApiOptions(options) },
                      e.target.value,
                    );
                    setDraft(next);
                    setOptions(JSON.stringify(next.api_options, null, 2));
                  } catch (error) {
                    setStatus(String(error));
                  }
                }}
              >
                {API_PROVIDERS.map((provider) => (
                  <option key={provider.id} value={provider.id}>
                    {providerLabel(provider, state.language)}
                  </option>
                ))}
              </Select>
            </Field>
            <Field
              label="API Key"
              hint={
                hasKey === null
                  ? t('Key 状态尚未确认', 'Key status is not yet confirmed')
                  : hasKey
                    ? t(
                        '已在本机登记，前端不读取已保存 Key',
                        'Registered locally; the saved key is never read by this form',
                      )
                    : t('尚未登记 Key', 'No key registered')
              }
            >
              <Input
                type="password"
                autoComplete="off"
                value={key}
                onChange={(e) => setKey(e.target.value)}
              />
            </Field>
            <div className="flex gap-2">
              <Button
                disabled={!key.trim()}
                onClick={() =>
                  void run(async () => {
                    await invoke('assistant_set_key', { baseUrl: draft.base_url, key });
                    setKey('');
                    setHasKey(true);
                    setStatus(t('Key 已登记', 'Key registered'));
                  })
                }
              >
                {t('登记 Key', 'Register key')}
              </Button>
              <Button
                disabled={!hasKey}
                onClick={() =>
                  void run(async () => {
                    await invoke('assistant_delete_key', { baseUrl: draft.base_url });
                    setHasKey(false);
                  })
                }
              >
                {t('删除 Key', 'Delete key')}
              </Button>
            </div>
          </>
        )}
        <Field label={t('模型', 'Model')}>
          <Input
            list="assistant-model-choices"
            value={model}
            onChange={(e) =>
              builtin
                ? patch({ model: e.target.value })
                : setDraft(withChoice(draft, { model: e.target.value }))
            }
          />
          <datalist id="assistant-model-choices">
            {models.map((item) => (
              <option key={item.id} value={item.id}>
                {item.name || item.id}
              </option>
            ))}
          </datalist>
        </Field>
        <Button onClick={() => void run(discover)}>{t('发现模型', 'Discover models')}</Button>
        <Field label={t('思考强度', 'Reasoning effort')}>
          <Select
            value={effort}
            onChange={(e) =>
              builtin
                ? patch(thinkSettings(e.target.value))
                : setDraft(withChoice(draft, { effort: e.target.value }))
            }
          >
            {Array.from(new Set([...efforts, effort])).map((value) => (
              <option key={value} value={value}>
                {value || t('服务默认', 'Provider default')}
              </option>
            ))}
          </Select>
        </Field>
        <Advanced title={t('高级设置', 'Advanced')}>
          {builtin && (
            <>
              <Field label={t('服务地址', 'Service URL')}>
                <Input
                  value={draft.base_url}
                  onChange={(e) => {
                    if (key) {
                      setStatus(
                        t(
                          '请先登记或清空 Key 再修改地址。',
                          'Register or clear the key before changing the address.',
                        ),
                      );
                      return;
                    }
                    patch({ base_url: e.target.value });
                  }}
                />
              </Field>
              <Field label={t('接口格式', 'API format')}>
                <Select
                  value={draft.api_format}
                  onChange={(e) => patch({ api_format: e.target.value })}
                >
                  <option value="chat_completions">Chat Completions</option>
                  <option value="responses">Responses</option>
                  <option value="anthropic">Anthropic</option>
                </Select>
              </Field>
              <Field label={t('输出 token 上限', 'Maximum output tokens')}>
                <Input
                  type="number"
                  min={1}
                  value={draft.max_output_tokens}
                  onChange={(e) => patch({ max_output_tokens: Number(e.target.value) })}
                />
              </Field>
              <Field label={t('超时秒数', 'Timeout (seconds)')}>
                <Input
                  type="number"
                  min={1}
                  value={draft.timeout_seconds}
                  onChange={(e) => patch({ timeout_seconds: Number(e.target.value) })}
                />
              </Field>
              <Field label={t('额外 API 选项（JSON）', 'Additional API options (JSON)')}>
                <textarea
                  aria-label={t('额外 API 选项', 'Additional API options')}
                  value={options}
                  onChange={(e) => setOptions(e.target.value)}
                />
              </Field>
              <label>
                <input
                  type="checkbox"
                  checked={draft.strict}
                  onChange={(e) => patch({ strict: e.target.checked })}
                />{' '}
                {t('严格工具格式', 'Strict tool format')}
              </label>
            </>
          )}
          <Field label={t('操作审批', 'Operation approval')}>
            <Select value={draft.approval} onChange={(e) => patch({ approval: e.target.value })}>
              <option value="ask">{t('每次询问', 'Ask each time')}</option>
              <option value="auto">
                {t('自动允许后端常规操作', 'Automatically allow routine backend operations')}
              </option>
            </Select>
          </Field>
          <label>
            <input
              type="checkbox"
              checked={draft.web_search}
              onChange={(e) => patch({ web_search: e.target.checked })}
            />{' '}
            {t('联网搜索', 'Web search')}
          </label>
          <Button
            onClick={() =>
              void run(async () =>
                setStatus(
                  JSON.stringify(await invokeJson('assistant_backend_status', {}), null, 2),
                ),
              )
            }
          >
            {t('检查外部后端状态', 'Check external backend status')}
          </Button>
        </Advanced>
        <Button
          onClick={() =>
            void run(async () => {
              if (key)
                throw new Error(t('请先登记或清空 Key。', 'Register or clear the key first.'));
              if (
                ![draft.max_output_tokens, draft.timeout_seconds].every(
                  (n) => Number.isSafeInteger(n) && n > 0,
                )
              )
                throw new Error(
                  t(
                    '输出上限和超时必须是正整数。',
                    'Output limit and timeout must be positive integers.',
                  ),
                );
              const next = settingsForSave(
                rememberProfile({ ...draft, api_options: parseApiOptions(options) }),
                settings,
              );
              await onSave(next);
              setStatus(
                t(
                  '设置已保存，下次发送时生效。',
                  'Settings saved; they apply to the next message.',
                ),
              );
            })
          }
        >
          {t('保存设置', 'Save settings')}
        </Button>
      </fieldset>
      {status && (
        <Notice>
          <pre className="whitespace-pre-wrap text-xs">{status}</pre>
        </Notice>
      )}
    </section>
  );
}

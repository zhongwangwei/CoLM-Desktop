import { useEffect, useRef, useState } from 'react';
import { invoke, invokeJson, hasBackend } from '../../lib/ipc';
import { useApp } from '../../lib/app-state';
import { Button, Input, Select, Field, Advanced, Notice } from '../../components/ui';
import { SaveBadge } from '../../components/SaveBadge';
import {
  API_PROVIDERS,
  providerLabel,
  apiEfforts,
  historyRepoLines,
  historyRepositoryLabel,
  historySettingsOf,
  historySyncSummary,
  managedNodeText,
  parseApiOptions,
  providerId,
  retrievalSettingsProblem,
  retrievalStateText,
  selectProvider,
  rememberProfile,
  settingsForSave,
  thinkValue,
  thinkSettings,
  withChoice,
  withContextWindowCleared,
  withHistoryPath,
  type AgentEvent,
  type HistoryReposStatus,
  type HistoryRepository,
  type HistorySyncResult,
  type Settings,
  type ModelChoice,
  type RetrievalSettings,
  type XerjNodeStatus,
} from './model';
import { draftSaveState, sendingSummary, settingsDraftDirty } from './settings-draft';

export default function AssistantSettings({
  settings,
  onSave,
  disabled,
  onDirtyChange,
  docsIndex,
}: {
  settings: Settings;
  onSave: (next: Settings) => Promise<void>;
  disabled: boolean;
  /** 把草稿是否有未保存修改报给面板：关闭/切走前据此决定要不要确认放弃（U-07）。 */
  onDirtyChange: (dirty: boolean) => void;
  /** 当前 Xerj 索引状态（应用级，不随对话切换）；null/缺省 = 还没收到状态事件。 */
  docsIndex?: AgentEvent | null;
}) {
  const { state } = useApp();
  const t = (zh: string, en: string) => (state.language === 'en' ? en : zh);
  function settingsError(error: unknown) {
    const text = String(error).replace(/^Error: /, '');
    if (text.includes('no API key is stored') || text.includes('no API key is configured'))
      return t(
        '这个服务还没有登记 API Key；先登记 Key 再刷新模型或保存。',
        'No API key is registered for this service; register one before refreshing models or saving.',
      );
    // 索引请求要经过正在运行的助手进程；还没启动时给一句能照做的提示，而不是中文原文。
    if (text.includes('助手还没有启动'))
      return t(
        '助手还没有启动；先发送一条消息启动助手，再更新或查询文档索引。',
        'The assistant is not running yet; send a message first, then update or check the document index.',
      );
    return text;
  }
  const lock = useRef(false);
  const [draft, setDraft] = useState(settings);
  const [options, setOptions] = useState(JSON.stringify(settings.api_options, null, 2));
  const [key, setKey] = useState('');
  const [hasKey, setHasKey] = useState<boolean | null>(null);
  // Xerj 的 Key 按检索 endpoint 另存一条，和模型服务不是同一个条目，不能复用上面的 key/hasKey。
  const [retrievalKey, setRetrievalKey] = useState('');
  const [retrievalHasKey, setRetrievalHasKey] = useState<boolean | null>(null);
  // 托管节点（managed）的实时状态来自 assistant_xerj_node；external 模式不探测、不启动。
  const [xerjNode, setXerjNode] = useState<XerjNodeStatus | null>(null);
  // U01 只读历史仓库：status 来自 assistant_history_status，同步结果来自 assistant_history_sync。
  // 读取失败时保留上次显示（historyStatus 不清空），只把错误单独展示。
  const [historyStatus, setHistoryStatus] = useState<HistoryReposStatus | null>(null);
  const [historyResult, setHistoryResult] = useState<HistorySyncResult | null>(null);
  const [historyError, setHistoryError] = useState('');
  const [historyBusy, setHistoryBusy] = useState(false);
  // U01 收口：逐仓库顺序同步的进度 / 结束结果 / 每仓失败原因；取消标记在当前调用返回后生效。
  const [syncProgress, setSyncProgress] = useState<{
    done: number;
    total: number;
    current: HistoryRepository;
    offline: boolean;
  } | null>(null);
  const [syncOutcome, setSyncOutcome] = useState<
    'success' | 'partial' | 'failed' | 'cancelled' | null
  >(null);
  const [syncFailures, setSyncFailures] = useState<
    { repository: HistoryRepository; message: string }[]
  >([]);
  const syncCancelled = useRef(false);
  // 双击/重复点击时 state 还没刷新，用 ref 挡住第二次逐仓库同步。
  const syncRunning = useRef(false);
  const [models, setModels] = useState<ModelChoice[]>([]);
  const [status, setStatus] = useState('');
  const [busy, setBusy] = useState(false);
  // 只用于保存请求在飞时显示"正在保存"；发现模型、登记 Key 等操作不算保存设置。
  const [saving, setSaving] = useState(false);
  const request = useRef(0);
  // 外部保存（例如选完历史仓库路径立即落盘，父级 settings 随之更新）只同步没有未保存修改的草稿：
  // 有未保存修改时保留用户正在编辑的内容，别让一次路径保存冲掉整个表单（U-07）。
  useEffect(() => {
    if (settingsDraftDirty(settings, draft, options)) return;
    setDraft(settings);
    setOptions(JSON.stringify(settings.api_options, null, 2));
  }, [settings, draft, options]);
  // 状态字符串是生成时语言的文本，切换语言后清掉，避免英文界面残留中文。
  useEffect(() => {
    setStatus('');
  }, [state.language]);
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
  // 必须放在最后：草稿比较依赖 draft/options，改完才报给面板（U-07）。onDirtyChange 是 setState，稳定。
  const dirty = settingsDraftDirty(settings, draft, options);
  useEffect(() => {
    onDirtyChange(dirty);
  }, [dirty, onDirtyChange]);
  // Xerj 的 Key 按 endpoint 单独登记（与模型服务的 Key 不是同一条），改动地址后重新问一次。
  // managed 模式的 Key 由应用自己管，不探测外部 Key。
  // 新 effect 放在已有 effect 之后，避免打乱按出现顺序抽取的页面状态。
  useEffect(() => {
    let active = true;
    setRetrievalHasKey(null);
    if (hasBackend() && draft.retrieval.mode !== 'managed')
      invoke<boolean>('assistant_has_key', { baseUrl: draft.retrieval.endpoint })
        .then((value) => {
          if (active) setRetrievalHasKey(value);
        })
        .catch(() => {
          // 地址可能正改到一半：Key 状态按"尚未确认"显示，报错留给保存校验。
          if (active) setRetrievalHasKey(null);
        });
    return () => {
      active = false;
    };
  }, [draft.retrieval.endpoint, draft.retrieval.mode]);
  // 打开设置时若已保存配置是 managed 模式，自动查一次托管节点状态（只查一次，不自动启动）。
  // 新 effect 放最后，避免打乱按出现顺序抽取的页面状态。
  useEffect(() => {
    if (!hasBackend() || settings.retrieval.mode !== 'managed') return;
    let active = true;
    void invoke<XerjNodeStatus>('assistant_xerj_node', { action: 'status' })
      .then((value) => {
        if (active) setXerjNode(value);
      })
      .catch((error) => {
        if (active) setStatus(settingsError(error));
      });
    return () => {
      active = false;
    };
    // 只在已保存的接入方式变化（首次打开/保存后）时执行；草稿里的未保存切换不触发探测。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings.retrieval.mode]);
  // U01：打开设置对话框时读一次仓库数据状态（只读；绝不自动同步）。新 effect 放最后。
  useEffect(() => {
    void loadHistoryStatus();
    // loadHistoryStatus 每次渲染都是新函数，不能进依赖；只在挂载（打开设置）时读一次，之后靠按钮/同步刷新。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  function patch(value: Partial<Settings>) {
    // 字段一改就清掉上一次"设置已保存"的成功提示，避免过时成功状态（U-05 / U-07）。
    setStatus('');
    setDraft((old) => ({ ...old, ...value }));
  }
  function patchRetrieval(value: Partial<RetrievalSettings>) {
    setStatus('');
    setDraft((old) => ({ ...old, retrieval: { ...old.retrieval, ...value } }));
  }
  /** 放弃草稿 = 把表单恢复成已保存设置；面板会据此清掉"有未保存修改"（U-07）。 */
  function discardDraft() {
    setDraft(settings);
    setOptions(JSON.stringify(settings.api_options, null, 2));
    setStatus(t('已放弃未保存的修改。', 'Unsaved changes discarded.'));
  }
  async function run(action: () => Promise<void>) {
    if (lock.current) return;
    lock.current = true;
    setBusy(true);
    setStatus('');
    try {
      await action();
    } catch (error) {
      setStatus(settingsError(error));
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
            : null;
    // Claude Code 没有列模型的接口；不要编造 fable/opus/sonnet/haiku 之类的假清单。
    if (found === null) {
      setModels([]);
      setStatus(
        t(
          'Claude Code 不能列出模型；请直接填写模型名（例如 sonnet、opus 或完整模型名）。',
          'Claude Code cannot list models; type the model name directly (for example sonnet, opus or the full name).',
        ),
      );
      return;
    }
    if (sequence === request.current) {
      setModels(found);
      setStatus(
        `${t('可用模型', 'Available models')}: ${found.length} (${t('保留手动输入', 'manual entry retained')})`,
      );
    }
  }
  /** 调 assistant_xerj_node，并把返回的托管节点状态存下来。 */
  async function xerjNodeAction(action: 'start' | 'stop' | 'status') {
    const value = await invoke<XerjNodeStatus>('assistant_xerj_node', { action });
    setXerjNode(value);
    return value;
  }
  /** 启动后立刻再查一次：启动是异步的，界面要显示新的 running/port/endpoint。 */
  async function startXerjNode() {
    await xerjNodeAction('start');
    await xerjNodeAction('status');
  }
  /**
   * U01：读取仓库数据状态（只读）。读取失败保留上一次的显示并单独报错；
   * 这里永远不发同步请求——同步必须由用户显式点按钮。
   */
  async function loadHistoryStatus(options: { quiet?: boolean } = {}) {
    if (!hasBackend()) return null;
    // 逐仓库同步的中间刷新用 quiet：外层负责 busy，别让每次刷新闪一下按钮状态。
    if (!options.quiet) setHistoryBusy(true);
    setHistoryError('');
    try {
      const status = await invoke<HistoryReposStatus>('assistant_history_status', {});
      setHistoryStatus(status);
      return status;
    } catch (error) {
      setHistoryError(settingsError(error));
      return null;
    } finally {
      if (!options.quiet) setHistoryBusy(false);
    }
  }
  /** 逐仓库进度里的仓库短名（桌面/官方），与历史仓库行的标签区分开。 */
  function historySyncRepositoryName(repository: HistoryRepository) {
    return repository === 'desktop' ? 'Desktop' : t('CoLM 官方', 'CoLM upstream');
  }
  /** U01：逐仓库进度的中英双语文案（含离线/在线与序号）。 */
  function historySyncProgressText(
    progress: { done: number; total: number; current: HistoryRepository; offline: boolean } | null,
  ) {
    if (!progress) return '';
    const name = historySyncRepositoryName(progress.current);
    const position = `${progress.done + 1}/${progress.total}`;
    return progress.offline
      ? t(`离线更新 ${name}（${position}）…`, `Offline update ${name} (${position})…`)
      : t(`正在同步 ${name}（${position}）…`, `Syncing ${name} (${position})…`);
  }
  /** U01：同步结束提示区分全部成功/部分失败/全部失败/已取消。 */
  function historySyncOutcomeText(
    outcome: 'success' | 'partial' | 'failed' | 'cancelled' | null,
    failures: { repository: HistoryRepository }[],
  ) {
    if (!outcome) return '';
    if (outcome === 'cancelled')
      return t(
        '已停止同步；剩余的仓库步骤没有发起。',
        'Sync stopped; the remaining repository steps were not started.',
      );
    if (outcome === 'success')
      return t('同步完成：两个仓库都已更新。', 'Sync completed: both repositories were updated.');
    if (outcome === 'partial')
      return t(
        `部分失败：${2 - failures.length}/2 个仓库成功，失败的仓库保留上次状态。`,
        `Partially failed: ${2 - failures.length}/2 repositories succeeded; failed repositories keep their previous status.`,
      );
    return t(
      '全部失败：两个仓库都没有更新成功。',
      'All failed: neither repository was updated successfully.',
    );
  }
  /** U01：停止逐仓库同步——只置取消标记；当前 invoke 返回后循环不会再发起下一个仓库。 */
  async function cancelHistorySync() {
    syncCancelled.current = true;
    try {
      await invoke('assistant_history_cancel', {});
    } catch (error) {
      setHistoryError(settingsError(error));
    }
  }
  /**
   * U01 收口：逐个仓库顺序同步（desktop → colm_upstream，固定两个都跑；清掉路径时仍更新
   * 应用管理副本）。每步后都刷新 status（失败也刷，展示 last_attempt）；某步失败继续下一步，
   * 结束时按成功数汇总全部成功/部分失败/全部失败；取消后不再发起下一步。
   */
  async function syncHistory(offline: boolean) {
    if (!hasBackend() || syncRunning.current || syncProgress) return null;
    syncRunning.current = true;
    const repositories: HistoryRepository[] = ['desktop', 'colm_upstream'];
    syncCancelled.current = false;
    setHistoryBusy(true);
    setHistoryError('');
    setHistoryResult(null);
    setSyncOutcome(null);
    setSyncFailures([]);
    const actions: Record<string, string> = {};
    const failures: { repository: HistoryRepository; message: string }[] = [];
    let latest: HistoryReposStatus | null = null;
    try {
      for (const [index, repository] of repositories.entries()) {
        if (syncCancelled.current) break;
        setSyncProgress({ done: index, total: repositories.length, current: repository, offline });
        try {
          const result = await invoke<HistorySyncResult>('assistant_history_sync', {
            repository,
            offline,
          });
          actions[repository] = result.actions?.[repository] ?? (offline ? 'offline' : 'fetched');
          latest = result;
          setHistoryResult(result);
          setHistoryStatus(result);
        } catch (error) {
          // 取消导致的中断不是“仓库失败”：不汇总为错误，由取消提示说明。
          if (!syncCancelled.current) {
            const message = settingsError(error);
            failures.push({ repository, message });
            actions[repository] = 'error';
            setSyncFailures([...failures]);
          }
        }
        // 每步结束（成功/失败/取消）都刷新：失败时 last_attempt 与旧状态都要能看见。
        const refreshed = await loadHistoryStatus({ quiet: true });
        if (refreshed) latest = refreshed;
        if (syncCancelled.current) break;
      }
    } finally {
      setSyncProgress(null);
      setHistoryBusy(false);
    }
    try {
      setHistoryResult({ ...(latest ?? {}), actions });
      if (syncCancelled.current) setSyncOutcome('cancelled');
      else if (failures.length === 0) setSyncOutcome('success');
      else if (failures.length === repositories.length) setSyncOutcome('failed');
      else setSyncOutcome('partial');
      // 结束再刷一次，保证取消/最后一步之后的两行状态尽量新。
      await loadHistoryStatus({ quiet: true });
      return { actions, failures: failures.map((failure) => failure.repository) };
    } finally {
      // 任何意外异常都不能让下一次同步被 syncRunning 永久挡住。
      syncRunning.current = false;
    }
  }
  /**
   * U01：把某个仓库的本地路径写回设置（null = 清除，回到应用管理副本）。
   * 只用**已保存**的 settings 作基底，绝不把设置对话框里的未保存草稿一起保存。
   */
  async function saveHistoryPath(repository: HistoryRepository, path: string | null) {
    const next = withHistoryPath(settings, repository, path);
    setDraft((old) => ({ ...old, history: next.history }));
    setHistoryBusy(true);
    setHistoryError('');
    try {
      await onSave(next);
      // 路径变化会改变身份/path 的读取结果；落盘成功后重新读一次状态。
      await loadHistoryStatus();
    } catch (error) {
      setHistoryError(settingsError(error));
    } finally {
      setHistoryBusy(false);
    }
  }
  /** U01：用后端文件夹选择器挑一个已有本地仓库，选中后立即保存路径。 */
  async function pickHistoryRepository(repository: HistoryRepository) {
    if (!hasBackend() || historyBusy || syncProgress) return;
    const path = await invoke<string | null>('pick_folder', {
      key: repository === 'desktop' ? 'assistant-history-desktop' : 'assistant-history-upstream',
    });
    if (path) await saveHistoryPath(repository, path);
  }
  const builtin = draft.backend === 'builtin';
  const model = builtin ? draft.model : draft.external[draft.backend]?.model || '';
  const effort = builtin ? thinkValue(draft) : draft.external[draft.backend]?.effort || '';
  const efforts = builtin
    ? apiEfforts(providerId(draft), model)
    : [
        '',
        ...(models.find((item) => item.id === model)?.efforts ||
          (draft.backend === 'opencode'
            ? []
            : draft.backend === 'claude_code'
              ? ['low', 'medium', 'high']
              : ['low', 'medium', 'high', 'xhigh', 'max'])),
      ];
  // "实际发送"按已保存设置算，不能用 draft：草稿换服务后、保存前发送仍走旧值（U-07）。
  const sending = sendingSummary(settings, state.language);
  const customProvider = builtin && providerId(draft) === 'custom';
  const presetBase = API_PROVIDERS.find((provider) => provider.id === providerId(draft))?.base_url;
  const editedPreset = builtin && !customProvider && presetBase !== draft.base_url;
  // 旧后端保存的 retrieval 可能还没有 mode：按 external 呈现（与后端"缺字段按 external"一致）。
  const retrievalMode = draft.retrieval.mode === 'managed' ? 'managed' : 'external';
  // 旧后端保存的设置可能还没有 history：统一补默认值，界面不因缺字段崩掉。
  const currentHistory = historySettingsOf(settings);
  const historySummary = historySyncSummary(historyResult, state.language);
  // 自定义服务的地址是必填项，不能藏在"高级设置"里（GUI 审查 A-02/R14）；预设服务改过地址时
  // 把实际目标地址显示出来，避免用户以为还在用预设站点。
  const serviceUrlField = (
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
          patch(withContextWindowCleared(draft, { base_url: e.target.value }));
        }}
      />
    </Field>
  );
  return (
    <section aria-label={t('助手设置', 'Assistant settings')} className="space-y-3">
      {/* 草稿与已保存设置分开说清楚：徽标给状态，这一行给"现在真正发往哪里"（U-07）。 */}
      <div className="flex flex-wrap items-center gap-2">
        <SaveBadge state={draftSaveState(dirty, saving)} t={t} />
        <span className="text-xs text-muted-foreground">
          {t('实际发送：', 'Sending as: ')}
          {sending.service} / {sending.model}
        </span>
      </div>
      <p className="text-xs text-muted-foreground">
        {dirty
          ? t(
              '下面的修改尚未保存；在点"保存设置"之前，发送仍使用上面这个服务与模型。',
              'Changes below are not saved yet; messages keep going to the service and model above until you save.',
            )
          : t(
              '发送使用上面这个服务与模型；下面的修改保存后生效。',
              'Messages go to the service and model above; changes below take effect after saving.',
            )}
      </p>
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
              setStatus('');
              setModels([]);
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
            {customProvider && serviceUrlField}
            {editedPreset && (
              <p className="text-xs text-muted-foreground">
                {t('实际目标地址：', 'Actual target: ')}
                {draft.base_url}
              </p>
            )}
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
                ? patch(withContextWindowCleared(draft, { model: e.target.value }))
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
        <Field label={t('操作审批', 'Operation approval')}>
          <Select value={draft.approval} onChange={(e) => patch({ approval: e.target.value })}>
            <option value="ask">{t('每次询问', 'Ask each time')}</option>
            <option value="auto">
              {t('自动允许后端常规操作', 'Automatically allow routine backend operations')}
            </option>
          </Select>
        </Field>
        {draft.backend !== 'opencode' && (
          <>
            <label>
              <input
                type="checkbox"
                checked={draft.web_search}
                onChange={(e) => patch({ web_search: e.target.checked })}
              />{' '}
              {t('联网搜索与读网页（DeepSeek API）', 'Web search and page fetch (DeepSeek APIs)')}
            </label>
            <p className="text-xs text-muted-foreground">
              {t(
                '每次搜索约 $0.00075；读网页先在本机免费尝试（HTML/文本，以及有文字层的 PDF，≤64 MB、前 300 页），失败时回退 DeepSeek Fetch API（每成功一页约 $0.00075，URL 与页面内容会发给 DeepSeek）。失败/空结果不计费。需要 DeepSeek Key。',
                'Each search costs about $0.00075. Pages are tried locally for free first (HTML/text and PDFs with an embedded text layer, up to 64 MB and the first 300 pages); if that fails, the DeepSeek Fetch API is used (about $0.00075 per successful page, and the URL and content are sent to DeepSeek). Failed or empty requests are free. Requires a DeepSeek key.',
              )}
            </p>
          </>
        )}
        <Advanced title={t('高级设置', 'Advanced')}>
          {builtin && !customProvider && serviceUrlField}
          {builtin && (
            <>
              <Field label={t('接口格式', 'API format')}>
                <Select
                  value={draft.api_format}
                  onChange={(e) =>
                    patch(withContextWindowCleared(draft, { api_format: e.target.value }))
                  }
                >
                  <option value="chat_completions">Chat Completions</option>
                  <option value="responses">Responses</option>
                  <option value="anthropic">Anthropic</option>
                </Select>
              </Field>
              <Field
                label={t('输出 token 上限', 'Maximum output tokens')}
                unit={t('tokens', 'tokens')}
                range={t('≥ 1 的整数', 'integer ≥ 1')}
              >
                <Input
                  type="number"
                  min={1}
                  value={draft.max_output_tokens}
                  onChange={(e) => patch({ max_output_tokens: Number(e.target.value) })}
                />
              </Field>
              <Field
                label={t('上下文窗口', 'Context window')}
                hint={t(
                  '内置 API 按这个窗口预留输入、输出和安全余量。留空时只对文档确认的 DeepSeek V4 模型自动取 1M；未知模型留空会在发送前要求填写，不会猜一个数。',
                  'Built-in API requests reserve input, output and a safety margin against this window. Blank auto-resolves only for the documented DeepSeek V4 models (1M); for any unknown model a blank window asks you to fill it in before sending instead of guessing.',
                )}
                unit={t('tokens', 'tokens')}
                range={t(
                  '1024–10000000 的整数；留空 = 自动',
                  'integer 1024–10000000; blank = auto',
                )}
              >
                <Input
                  type="number"
                  min={1024}
                  max={10_000_000}
                  value={draft.context_window ?? ''}
                  onChange={(e) =>
                    patch({
                      context_window: e.target.value === '' ? null : Number(e.target.value),
                    })
                  }
                />
              </Field>
              <Field
                label={t('超时秒数', 'Timeout (seconds)')}
                unit={t('秒', 'seconds')}
                range={t('≥ 1 的整数', 'integer ≥ 1')}
              >
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
                  onChange={(e) => {
                    setStatus('');
                    setOptions(e.target.value);
                  }}
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
          <Field
            label={t('每会话最多轮数', 'Turns per conversation')}
            hint={t(
              '达到上限后开始新对话可继续。',
              'After the limit, start a new conversation to continue.',
            )}
            range={t('≥ 0；0 = 不限', '≥ 0; 0 = unlimited')}
          >
            <Input
              type="number"
              min={0}
              value={draft.session_turn_limit}
              onChange={(e) => patch({ session_turn_limit: Number(e.target.value) })}
            />
          </Field>
          <Field
            label={t('每会话最多 token', 'Tokens per conversation')}
            hint={t(
              '累计 prompt + completion。每次调用模型都会把整段对话和工具说明重新计入一遍（一次约 1–2 万），一轮带工具调用常有好几次。',
              'Cumulative prompt + completion. Every model call counts the whole conversation and tool descriptions again (about 10–20k each), and a turn with tool calls often makes several.',
            )}
            unit={t('tokens', 'tokens')}
            range={t('≥ 0；0 = 不限', '≥ 0; 0 = unlimited')}
          >
            <Input
              type="number"
              min={0}
              value={draft.session_token_limit}
              onChange={(e) => patch({ session_token_limit: Number(e.target.value) })}
            />
          </Field>
          <div className="space-y-2 border-t pt-3">
            <strong>{t('文档检索（Xerj，可选）', 'Document search (Xerj, optional)')}</strong>
            <p className="text-xs text-muted-foreground">
              {t(
                '应用级设置，与上面选的后端无关：回答可以引用本机 Xerj 里索引的文档。关闭时完全不访问 Xerj。',
                'An application-level setting independent of the backend above: answers may cite documents indexed in a local Xerj. Nothing is sent to Xerj while this is off.',
              )}
            </p>
            <label>
              <input
                type="checkbox"
                checked={draft.retrieval.enabled}
                onChange={(e) => patchRetrieval({ enabled: e.target.checked })}
              />{' '}
              {t('启用文档检索', 'Enable document search')}
            </label>
            <Field
              label={t('接入方式', 'Connection mode')}
              hint={t(
                'external 使用你自行运行的本机节点；managed 由应用启动并停止自己的 Xerj 进程。切换接入方式不会清掉已填的服务地址、索引名或已登记的 Key。',
                'external uses a local node you run yourself; managed starts and stops the app’s own Xerj process. Switching modes does not clear the saved service URL, index name or registered key.',
              )}
            >
              <Select
                value={retrievalMode}
                onChange={(e) =>
                  patchRetrieval({ mode: e.target.value === 'managed' ? 'managed' : 'external' })
                }
              >
                <option value="external">
                  {t('使用已运行的本机节点（external）', 'Use a local node you run (external)')}
                </option>
                <option value="managed">
                  {t(
                    '由应用启动并管理节点（managed）',
                    'Let the app start and manage the node (managed)',
                  )}
                </option>
              </Select>
            </Field>
            {retrievalMode === 'managed' ? (
              <>
                <Field
                  label={t('Xerj 二进制路径（可选）', 'Xerj binary path (optional)')}
                  hint={t(
                    '留空则依次查找 COLM_XERJ_BINARY、应用资源目录、PATH。只接受固定 release v1.0.0-rc.100。',
                    'Leave blank to look for COLM_XERJ_BINARY, the app resources directory, then PATH. Only the pinned release v1.0.0-rc.100 is accepted.',
                  )}
                >
                  <Input
                    placeholder={t('留空自动探测', 'Blank = detect automatically')}
                    value={draft.retrieval.binary_path ?? ''}
                    onChange={(e) => patchRetrieval({ binary_path: e.target.value })}
                  />
                </Field>
                <Field
                  label={t('节点端口（可选）', 'Node port (optional)')}
                  hint={t(
                    '留空自动选连续 3 个空闲 loopback 端口。',
                    'Blank picks three consecutive free loopback ports.',
                  )}
                  range={t('1024–65000 的整数；留空 = 自动', 'integer 1024–65000; blank = auto')}
                >
                  <Input
                    type="number"
                    min={1024}
                    max={65_000}
                    placeholder={t('自动', 'auto')}
                    value={draft.retrieval.port ?? ''}
                    onChange={(e) =>
                      patchRetrieval({
                        port: e.target.value === '' ? null : Number(e.target.value),
                      })
                    }
                  />
                </Field>
                <p className="text-xs text-muted-foreground">
                  {t(
                    '应用不会自动下载或升级 Xerj：版本不等于固定 release v1.0.0-rc.100 时只提示，需要人工替换二进制并重跑验收。',
                    'The app never downloads or upgrades Xerj: if the version differs from the pinned release v1.0.0-rc.100 it only warns; replace the binary manually and re-run acceptance.',
                  )}
                </p>
                <p className="text-xs text-muted-foreground">
                  {t('托管节点：', 'Managed node: ')}
                  {managedNodeText(xerjNode, state.language)}
                </p>
                <p className="text-xs text-muted-foreground">
                  {t('服务地址（由应用决定）：', 'Service URL (set by the app): ')}
                  {xerjNode?.endpoint || t('启动节点后显示', 'shown after the node starts')}
                </p>
                <p className="text-xs text-muted-foreground">
                  {t(
                    '节点操作使用已保存的设置；改了接入方式或二进制路径后请先保存。',
                    'Node actions use the saved settings; save first after changing the mode or binary path.',
                  )}
                </p>
                <div className="flex flex-wrap gap-2">
                  <Button
                    onClick={() =>
                      void run(async () => {
                        await startXerjNode();
                        setStatus(
                          t(
                            '已请求启动 Xerj 节点；状态已刷新。',
                            'Xerj node start requested; status refreshed.',
                          ),
                        );
                      })
                    }
                  >
                    {t('启动节点', 'Start node')}
                  </Button>
                  <Button
                    onClick={() =>
                      void run(async () => {
                        await xerjNodeAction('stop');
                        setStatus(t('已停止 Xerj 节点。', 'Xerj node stopped.'));
                      })
                    }
                  >
                    {t('停止节点', 'Stop node')}
                  </Button>
                  <Button
                    onClick={() =>
                      void run(async () => {
                        await xerjNodeAction('status');
                        setStatus(t('已刷新节点状态。', 'Node status refreshed.'));
                      })
                    }
                  >
                    {t('刷新状态', 'Refresh status')}
                  </Button>
                </div>
                <p className="text-xs text-muted-foreground">
                  {t(
                    'Key 由应用自动生成并保管在本机应用数据目录，不需要手动填写。',
                    'The key is generated and kept by the app in its local application data directory; no manual key entry is needed.',
                  )}
                </p>
              </>
            ) : (
              <>
                <Field
                  label={t('Xerj 服务地址', 'Xerj service URL')}
                  hint={t(
                    '只接受本机回环 http，且必须写明端口；例如 http://127.0.0.1:9200。',
                    'Only loopback http with an explicit port, for example http://127.0.0.1:9200.',
                  )}
                >
                  <Input
                    value={draft.retrieval.endpoint}
                    onChange={(e) => {
                      if (retrievalKey) {
                        setStatus(
                          t(
                            '请先登记或清空 Xerj Key 再修改服务地址。',
                            'Register or clear the Xerj key before changing the service URL.',
                          ),
                        );
                        return;
                      }
                      patchRetrieval({ endpoint: e.target.value });
                    }}
                  />
                </Field>
                <Field
                  label={t('Xerj API Key', 'Xerj API key')}
                  hint={
                    retrievalHasKey === null
                      ? t('Key 状态尚未确认', 'Key status is not yet confirmed')
                      : retrievalHasKey
                        ? t(
                            '已在本机登记，前端不读取已保存 Key',
                            'Registered locally; the saved key is never read by this form',
                          )
                        : t(
                            '尚未登记 Key（无认证的 Xerj 节点可留空）',
                            'No key registered (leave empty for an unauthenticated Xerj node)',
                          )
                  }
                >
                  <Input
                    type="password"
                    autoComplete="off"
                    value={retrievalKey}
                    onChange={(e) => setRetrievalKey(e.target.value)}
                  />
                </Field>
                <div className="flex gap-2">
                  <Button
                    disabled={!retrievalKey.trim()}
                    onClick={() =>
                      void run(async () => {
                        await invoke('assistant_set_key', {
                          baseUrl: draft.retrieval.endpoint,
                          key: retrievalKey,
                        });
                        setRetrievalKey('');
                        setRetrievalHasKey(true);
                        setStatus(t('Xerj Key 已登记', 'Xerj key registered'));
                      })
                    }
                  >
                    {t('登记 Key', 'Register key')}
                  </Button>
                  <Button
                    disabled={!retrievalHasKey}
                    onClick={() =>
                      void run(async () => {
                        await invoke('assistant_delete_key', { baseUrl: draft.retrieval.endpoint });
                        setRetrievalHasKey(false);
                      })
                    }
                  >
                    {t('删除 Key', 'Delete key')}
                  </Button>
                </div>
              </>
            )}
            {/* 索引名与超时两种接入方式共用：external 靠自己运行的节点，managed 的索引也建在这个名字下。 */}
            <Field
              label={t('索引名', 'Index name')}
              hint={t(
                '小写字母、数字、下划线或连字符，1–64 字节，以字母或数字开头。',
                'Lowercase letters, digits, underscores or hyphens, 1–64 bytes, starting with a letter or digit.',
              )}
            >
              <Input
                value={draft.retrieval.index}
                onChange={(e) => patchRetrieval({ index: e.target.value })}
              />
            </Field>
            <Field
              label={t('检索超时（毫秒）', 'Retrieval timeout (ms)')}
              unit={t('毫秒', 'ms')}
              range={t('100–10000 的整数', 'integer 100–10000')}
            >
              <Input
                type="number"
                min={100}
                max={10_000}
                value={draft.retrieval.timeout_ms}
                onChange={(e) => patchRetrieval({ timeout_ms: Number(e.target.value) })}
              />
            </Field>
            <p className="text-xs text-muted-foreground">
              {t('索引状态：', 'Index status: ')}
              {docsIndex ? (
                <>
                  {retrievalStateText(docsIndex.state || '', state.language)}
                  {typeof docsIndex.documents === 'number' &&
                    ` · ${t('文档', 'documents')} ${docsIndex.documents}`}
                  {typeof docsIndex.chunks === 'number' &&
                    ` · ${t('分块', 'chunks')} ${docsIndex.chunks}`}
                  {typeof docsIndex.generation === 'number' &&
                    ` · ${t('索引代次', 'generation')} ${docsIndex.generation}`}
                  {docsIndex.detail ? ` · ${docsIndex.detail}` : ''}
                </>
              ) : (
                t('还没收到状态；点“查询状态”获取。', 'No status yet; use "Check status".')
              )}
            </p>
            <div className="flex flex-wrap gap-2">
              <Button
                onClick={() =>
                  void run(async () => {
                    await invoke('assistant_docs_index', { action: 'refresh' });
                    setStatus(
                      t(
                        '已请求更新文档索引；状态见上方。',
                        'Document index update requested; see the status above.',
                      ),
                    );
                  })
                }
              >
                {t('更新文档索引', 'Update document index')}
              </Button>
              <Button
                onClick={() =>
                  void run(async () => {
                    await invoke('assistant_docs_index', { action: 'status' });
                    setStatus(t('已请求索引状态。', 'Index status requested.'));
                  })
                }
              >
                {t('查询状态', 'Check status')}
              </Button>
            </div>
          </div>
          {/* U01：只读历史仓库数据（Desktop + CoLM 官方）。同步是显式动作，聊天永远不会自动触发。 */}
          <div className="space-y-2 border-t pt-3">
            <strong>{t('仓库数据（只读历史）', 'Repository data (read-only history)')}</strong>
            <p className="text-xs text-muted-foreground">
              {t(
                '只读历史查询用的 Desktop 与 CoLM 官方仓库。同步只在点按钮时发生；普通聊天绝不自动下载或更新仓库。',
                'The Desktop and CoLM upstream repositories used for read-only history queries. Syncing happens only when you press a button; normal chat never downloads or updates them.',
              )}
            </p>
            <p className="text-xs text-muted-foreground">
              {t('数据目录：', 'Data directory: ')}
              {historyStatus?.history_root || t('尚未读取', 'not loaded yet')}
            </p>
            {(historyBusy || syncProgress) && (
              <div className="flex flex-wrap items-center gap-2">
                <p className="text-xs text-muted-foreground">
                  {syncProgress
                    ? historySyncProgressText(syncProgress)
                    : historyStatus
                      ? t('正在处理仓库数据…', 'Working on repository data…')
                      : t('正在读取仓库状态…', 'Reading repository status…')}
                </p>
                <Button variant="ghost" onClick={() => void cancelHistorySync()}>
                  {t('停止同步', 'Stop syncing')}
                </Button>
              </div>
            )}
            {(['desktop', 'colm_upstream'] as HistoryRepository[]).map((repository) => {
              const repo = historyStatus?.repos?.[repository];
              const lines = historyRepoLines(repo, state.language);
              const configuredPath =
                repository === 'desktop'
                  ? currentHistory.desktop_path
                  : currentHistory.upstream_path;
              const localPath = configuredPath || repo?.path || repo?.identity?.root || '';
              return (
                <div key={repository} className="space-y-1 rounded border p-2">
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <strong>{historyRepositoryLabel(repository, state.language)}</strong>
                    <span className="text-xs text-muted-foreground">
                      {lines.state}
                      {repo?.default_branch
                        ? ` · ${t('默认分支', 'default branch')} ${repo.default_branch}`
                        : ''}
                    </span>
                  </div>
                  <p className="text-xs">{lines.lastSync}</p>
                  <p className="text-xs text-muted-foreground">{lines.checkedAt}</p>
                  <p className="text-xs text-muted-foreground">
                    {configuredPath
                      ? t('本地仓库（已指定）：', 'Local repository (configured): ')
                      : t('应用管理副本：', 'App-managed copy: ')}
                    {localPath || t('未设置', 'not set')}
                  </p>
                  {lines.errors.length > 0 && (
                    <Notice tone="error">
                      {lines.errors.map((error) => (
                        <p key={error} className="text-xs">
                          {error}
                        </p>
                      ))}
                    </Notice>
                  )}
                  {lines.preparation && (
                    <p className="text-xs text-muted-foreground">{lines.preparation}</p>
                  )}
                  <div className="flex flex-wrap gap-2">
                    <Button
                      disabled={disabled || historyBusy || syncProgress !== null}
                      onClick={() => void pickHistoryRepository(repository)}
                    >
                      {t('选择已有本地仓库', 'Choose an existing local repository')}
                    </Button>
                    <Button
                      variant="ghost"
                      disabled={disabled || historyBusy || syncProgress !== null || !configuredPath}
                      onClick={() => void saveHistoryPath(repository, null)}
                    >
                      {t('清除本地路径', 'Clear local path')}
                    </Button>
                  </div>
                </div>
              );
            })}
            <div className="flex flex-wrap gap-2">
              <Button
                disabled={disabled || historyBusy || syncProgress !== null}
                onClick={() => void loadHistoryStatus()}
              >
                {t('刷新状态', 'Refresh status')}
              </Button>
              <Button
                disabled={disabled || historyBusy || syncProgress !== null}
                onClick={() => void syncHistory(false)}
              >
                {syncProgress
                  ? historySyncProgressText(syncProgress)
                  : t('准备 / 更新数据', 'Prepare / update data')}
              </Button>
              <Button
                variant="ghost"
                disabled={disabled || historyBusy || syncProgress !== null}
                onClick={() => void syncHistory(true)}
              >
                {t('离线更新（不联网）', 'Offline update (no network)')}
              </Button>
            </div>
            {syncOutcome && (
              <p className="text-xs text-muted-foreground">
                {historySyncOutcomeText(syncOutcome, syncFailures)}
              </p>
            )}
            {syncFailures.length > 0 && (
              <Notice tone="error">
                {syncFailures.map((failure) => (
                  <p key={failure.repository} className="text-xs">
                    {historySyncRepositoryName(failure.repository)}
                    {t('：', ': ')}
                    {failure.message}
                  </p>
                ))}
              </Notice>
            )}
            {historySummary &&
              (historySummary.failed ? (
                <Notice tone="error">{historySummary.text}</Notice>
              ) : (
                <p className="text-xs text-muted-foreground">{historySummary.text}</p>
              ))}
            {historyError && <Notice tone="error">{historyError}</Notice>}
          </div>
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
        <div className="flex flex-wrap gap-2">
          <Button
            onClick={() =>
              void run(async () => {
                if (key)
                  throw new Error(t('请先登记或清空 Key。', 'Register or clear the key first.'));
                // Xerj 的 Key 也是"输入框里有内容"= 还没登记；保存会重启助手，先让用户登记或清空。
                // managed 模式的 Key 由应用管理，外部模式残留的输入不影响保存。
                if (retrievalMode === 'external' && retrievalKey)
                  throw new Error(
                    t('请先登记或清空 Xerj Key。', 'Register or clear the Xerj key first.'),
                  );
                if (
                  ![draft.max_output_tokens, draft.timeout_seconds].every(
                    (n) => Number.isSafeInteger(n) && n > 0,
                  ) ||
                  !(
                    draft.context_window === null ||
                    (Number.isSafeInteger(draft.context_window) &&
                      draft.context_window >= 1024 &&
                      draft.context_window <= 10_000_000)
                  ) ||
                  ![draft.session_turn_limit, draft.session_token_limit].every(
                    (n) => Number.isSafeInteger(n) && n >= 0,
                  )
                )
                  throw new Error(
                    t(
                      '输出上限和超时必须是正整数；上下文窗口须为 1024–10000000 的整数或留空；会话上限必须是非负整数。',
                      'Output limit and timeout must be positive integers; the context window must be an integer between 1024 and 10000000 or blank; session limits must be non-negative integers.',
                    ),
                  );
                // 检索关闭也校验格式：开/关之间切换不应因为存了非法值才发现。
                const retrievalProblem = retrievalSettingsProblem(draft.retrieval, state.language);
                if (retrievalProblem) throw new Error(retrievalProblem);
                // 二进制路径的首尾空白不代表路径本身；空串按"自动探测"归一成 null，别把空白存进设置。
                const retrieval: RetrievalSettings = {
                  ...draft.retrieval,
                  binary_path: draft.retrieval.binary_path?.trim() || null,
                  endpoint: draft.retrieval.endpoint.trim(),
                };
                const next = settingsForSave(
                  rememberProfile({ ...draft, retrieval, api_options: parseApiOptions(options) }),
                  settings,
                );
                setSaving(true);
                try {
                  await onSave(next);
                } finally {
                  setSaving(false);
                }
                // 保存后的归一化值（trim/空串转 null）直接成为草稿，避免刚保存完又显示"未保存修改"。
                setDraft(next);
                setOptions(JSON.stringify(next.api_options, null, 2));
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
          <Button variant="ghost" disabled={!dirty} onClick={discardDraft}>
            {t('放弃修改', 'Discard changes')}
          </Button>
        </div>
      </fieldset>
      {status && (
        <Notice>
          <pre className="whitespace-pre-wrap text-xs">{status}</pre>
        </Notice>
      )}
    </section>
  );
}

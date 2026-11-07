import { useEffect, useState } from 'react';
import { useApp } from '../../lib/app-state';
import { usePageContract } from '../../lib/guide';
import { invokeJson } from '../../lib/ipc';
import {
  confirmForceDelete,
  formatBytes,
  selectionAfterPrune,
  withoutRevisions,
  type PruneReport,
} from '../../lib/model-tidy';
import type { VersionRef } from '../../lib/types';
import { Button, Card, Field, Select } from '../../components/ui';
import { ActionButton } from '../../components/ActionButton';

/** Check how much the old official revisions take, and tidy them on request. */
function ModelCacheCard({ t }: { t: (zh: string, en: string) => string }) {
  const { state, update, refreshVersions } = useApp();
  const [report, setReport] = useState<PruneReport | null>(null);
  const [busy, setBusy] = useState(false);
  // 结果文案按当前语言在渲染时生成；错误原文切换语言后清掉，避免中英混排（F-A-06）。
  const [message, setMessage] = useState<
    | { kind: 'tidied'; removed: number; bytes: number }
    | { kind: 'error' | 'forced'; text: string }
    | null
  >(null);
  useEffect(() => {
    setMessage((current) => (current?.kind === 'error' ? null : current));
  }, [state.language]);
  const run = async (dryRun: boolean) => {
    setBusy(true);
    setMessage(null);
    try {
      const result = await invokeJson<PruneReport>('model_prune', { dryRun });
      setReport(result);
      if (!dryRun) {
        // 和启动期整理一样：当前选中的修订若被删了，回到最新的保留版本，不留悬空的 versionRef（GUI 审查 C-12）。
        update((old) => {
          const version = selectionAfterPrune(old.versions, result.removed, old.versionRef);
          return {
            versions: withoutRevisions(old.versions, result.removed),
            version,
            ...(version ? { versionRef: version.version } : {}),
            chosenVersion: old.chosenVersion
              ? (selectionAfterPrune(old.versions, result.removed, old.chosenVersion)?.version ??
                null)
              : null,
          };
        });
        setMessage({ kind: 'tidied', removed: result.removed.length, bytes: result.bytes });
      }
    } catch (error) {
      setMessage({ kind: 'error', text: String(error) });
    } finally {
      setBusy(false);
    }
  };
  const officials = state.versions.filter((v) => v.version.id === 'official');
  const latest = Math.max(...officials.map((v) => v.version.revision));
  const forceTargets = officials.filter((v) => v.version.revision !== latest).map((v) => v.version);
  const forceCleanup = async () => {
    if (busy) return;
    setBusy(true);
    setMessage(null);
    try {
      const result = await confirmForceDelete(forceTargets, t, (args) =>
        invokeJson<{ deleted: VersionRef[]; failed: { version: VersionRef; reason: string }[] }>(
          'model_delete',
          args,
        ),
      );
      if (!result) return;
      await refreshVersions();
      setReport(null);
      setMessage({
        kind: 'forced',
        text:
          t(
            `已强制清理 ${result.deleted.length} 个版本；保留 ${result.failed.length} 个。`,
            `Force deleted ${result.deleted.length} revisions; kept ${result.failed.length}.`,
          ) +
          result.failed.map((f) => `\n${f.version.id}#${f.version.revision}: ${f.reason}`).join(''),
      });
    } catch (error) {
      setMessage({ kind: 'error', text: String(error) });
    } finally {
      setBusy(false);
    }
  };
  const removable = report?.dry_run ? report.removed.length : 0;
  return (
    <Card
      title={t('模型缓存', 'Model cache')}
      description={t(
        '每次软件更新都会登记一个新的官方修订和约 126 MB 的源码快照。启动时会自动整理没有算例、研究或历史记录使用的旧官方修订；自动整理会保留最新修订和被派生或草稿依赖的修订。强制清理仍保留最新官方修订。',
        'Each software update registers a new official revision with a ~126 MB source snapshot. Old official revisions no case, study or history record uses are tidied at start; automatic tidying keeps the latest one and any a version or draft builds on. Force tidying still keeps the latest official revision.',
      )}
    >
      {report?.dry_run && (
        <p>
          {removable
            ? t(
                `可整理 ${removable} 个旧官方修订，约 ${formatBytes(report.bytes)}；保留 ${report.kept.length} 个。`,
                `${removable} old official revisions can go (about ${formatBytes(report.bytes)}); ${report.kept.length} stay.`,
              )
            : t(
                `没有可整理的旧官方修订；保留 ${report.kept.length} 个。`,
                `Nothing to tidy; ${report.kept.length} official revisions stay.`,
              )}
        </p>
      )}
      {message && (
        <p className="muted">
          {message.kind === 'tidied'
            ? t(
                `已整理 ${message.removed} 个旧官方修订，释放 ${formatBytes(message.bytes)}`,
                `Tidied ${message.removed} old official revisions, freeing ${formatBytes(message.bytes)}`,
              )
            : message.text}
        </p>
      )}
      <div className="actions">
        <ActionButton
          action="settings-cache-check"
          label={t('检查可释放的空间', 'Check reclaimable space')}
          kind="click"
          disabled={busy}
          run={() => run(true)}
        >
          {t('检查可释放的空间', 'Check reclaimable space')}
        </ActionButton>
        {/* 整理模型缓存会删文件，按 D 级规则只在界面上做，不登记给助手。 */}
        <Button disabled={busy || !removable} onClick={() => void run(false)}>
          {t('整理模型缓存', 'Tidy model cache')}
        </Button>
        <Button
          variant="danger"
          disabled={busy || !forceTargets.length}
          onClick={() => void forceCleanup()}
        >
          {t('强制清理旧官方版本', 'Force tidy old official revisions')}
        </Button>
      </div>
    </Card>
  );
}

export default function SettingsPage() {
  const { state, update, navigate } = useApp();
  const t = (zh: string, en: string) => (state.language === 'en' ? en : zh);
  const assistantSettings = () => update({ assistantOpen: true, assistantSettingsOpen: true });
  usePageContract({
    title: t('设置', 'Settings'),
    step: 'settings',
    fields: [
      {
        id: 'app-language',
        label: t('语言', 'Language'),
        value: state.language,
        type: 'choice',
        options: [
          { value: 'zh', label: '中文' },
          { value: 'en', label: 'English' },
        ],
        saves: true,
        set: (value) => update({ language: value as 'zh' | 'en' }),
      },
      {
        id: 'app-theme',
        label: t('外观', 'Appearance'),
        value: state.theme,
        type: 'choice',
        options: [
          { value: 'light', label: t('浅色', 'Light') },
          { value: 'dark', label: t('深色', 'Dark') },
        ],
        saves: true,
        set: (value) => update({ theme: value as 'light' | 'dark' }),
      },
      {
        id: 'app-expert',
        label: t('默认展开专家设置', 'Expand expert settings by default'),
        value: String(state.expert),
        type: 'switch',
        saves: true,
        set: (value) => update({ expert: value === 'true' }),
      },
    ],
    // 按钮自己登记（第 737 轮）。整理模型缓存会删文件，按规矩只在界面上做，不给助手。
    actions: [],
  });
  return (
    <div className="page-stack">
      <h1>{t('设置', 'Settings')}</h1>
      <Card title={t('界面', 'Interface')}>
        <div className="form-grid">
          <Field label={t('语言', 'Language')}>
            <Select
              value={state.language}
              onChange={(e) => update({ language: e.target.value as 'zh' | 'en' })}
            >
              <option value="zh">中文</option>
              <option value="en">English</option>
            </Select>
          </Field>
          <Field label={t('外观', 'Appearance')}>
            <Select
              value={state.theme}
              onChange={(e) => update({ theme: e.target.value as 'light' | 'dark' })}
            >
              <option value="light">{t('浅色', 'Light')}</option>
              <option value="dark">{t('深色', 'Dark')}</option>
            </Select>
          </Field>
        </div>
        <label className="mode-toggle">
          <input
            type="checkbox"
            checked={state.expert}
            onChange={(e) => update({ expert: e.target.checked })}
          />
          {t('默认展开专家设置', 'Expand expert settings by default')}
        </label>
      </Card>
      <Card
        title={t('AI 助手', 'AI assistant')}
        description={t(
          '服务、模型和思考程度在助手设置中配置。',
          'Configure the service, model and reasoning in assistant settings.',
        )}
      >
        <ActionButton
          action="app-assistant-settings"
          label={t('配置 AI 助手', 'Configure AI assistant')}
          run={assistantSettings}
        />
      </Card>
      <ModelCacheCard t={t} />
      <ActionButton action="app-help" label={t('帮助', 'Help')} run={() => navigate('help')} />
    </div>
  );
}

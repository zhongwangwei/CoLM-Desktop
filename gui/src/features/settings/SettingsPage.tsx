import { useState } from 'react';
import { useApp } from '../../lib/app-state';
import { usePageContract } from '../../lib/guide';
import { invokeJson } from '../../lib/ipc';
import { formatBytes, withoutRevisions, type PruneReport } from '../../lib/model-tidy';
import { Button, Card, Field, Select } from '../../components/ui';

/** Check how much the old official revisions take, and tidy them on request. */
function ModelCacheCard({ t }: { t: (zh: string, en: string) => string }) {
  const { update } = useApp();
  const [report, setReport] = useState<PruneReport | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState('');
  const run = async (dryRun: boolean) => {
    setBusy(true);
    setMessage('');
    try {
      const result = await invokeJson<PruneReport>('model_prune', { dryRun });
      setReport(result);
      if (!dryRun) {
        update((old) => ({ versions: withoutRevisions(old.versions, result.removed) }));
        setMessage(
          t(
            `已整理 ${result.removed.length} 个旧官方修订，释放 ${formatBytes(result.bytes)}`,
            `Tidied ${result.removed.length} old official revisions, freeing ${formatBytes(result.bytes)}`,
          ),
        );
      }
    } catch (error) {
      setMessage(String(error));
    } finally {
      setBusy(false);
    }
  };
  const removable = report?.dry_run ? report.removed.length : 0;
  return (
    <Card
      title={t('模型缓存', 'Model cache')}
      description={t(
        '每次软件更新都会登记一个新的官方修订和约 126 MB 的源码快照。启动时会自动整理没有算例、研究或历史记录使用的旧官方修订；最新修订、被派生或草稿依赖的修订始终保留。',
        'Each software update registers a new official revision with a ~126 MB source snapshot. Old official revisions no case, study or history record uses are tidied at start; the latest one and any a version or draft builds on always stay.',
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
      {message && <p className="muted">{message}</p>}
      <div className="actions">
        <Button disabled={busy} onClick={() => void run(true)}>
          {t('检查可释放的空间', 'Check reclaimable space')}
        </Button>
        <Button disabled={busy || !removable} onClick={() => void run(false)}>
          {t('整理模型缓存', 'Tidy model cache')}
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
    actions: [
      {
        id: 'app-assistant-settings',
        label: t('配置 AI 助手', 'Configure AI assistant'),
        kind: 'click',
        run: assistantSettings,
      },
      { id: 'app-help', label: t('帮助', 'Help'), kind: 'click', run: () => navigate('help') },
    ],
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
        <Button onClick={assistantSettings}>{t('配置 AI 助手', 'Configure AI assistant')}</Button>
      </Card>
      <ModelCacheCard t={t} />
      <Button onClick={() => navigate('help')}>{t('帮助', 'Help')}</Button>
    </div>
  );
}

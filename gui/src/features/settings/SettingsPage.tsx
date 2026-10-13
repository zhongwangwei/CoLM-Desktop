import { useApp } from '../../lib/app-state';
import { usePageContract } from '../../lib/guide';
import { Button, Card, Field, Select } from '../../components/ui';

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
      <Button onClick={() => navigate('help')}>{t('帮助', 'Help')}</Button>
    </div>
  );
}

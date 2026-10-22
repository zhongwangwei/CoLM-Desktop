import { Field, Input, Notice, Select } from '../../components/ui';
import { ActionButton } from '../../components/ActionButton';

export type ForcingLayoutProbe = {
  status: 'loading' | 'done' | 'error';
  candidates: string[];
  truncated?: boolean;
  error?: string;
  coverage?: { start: string; end: string } | null;
  coverage_error?: string | null;
};

export default function SpatialForcingFields({
  directory,
  dataset,
  datasets,
  probe,
  catalogError,
  currentDataset,
  disabled,
  en,
  onDirectoryChange,
  onDirectoryBlur,
  onPick,
  pickAction,
  detectAction,
  onDatasetChange,
  onDetect,
}: {
  directory: string;
  dataset: string;
  datasets: string[];
  probe: ForcingLayoutProbe | null;
  catalogError: string;
  currentDataset?: string;
  disabled: boolean;
  en: boolean;
  onDirectoryChange: (value: string) => void;
  onDirectoryBlur: () => void;
  onPick: () => void;
  /** 助手动作名：选择强迫场路径、识别数据集。 */
  pickAction: string;
  detectAction: string;
  onDatasetChange: (value: string) => void;
  onDetect: () => void;
}) {
  const t = (zh: string, english: string) => (en ? english : zh);
  return (
    <>
      <div className="form-grid">
        <Field label={t('强迫场数据路径', 'Forcing data path')}>
          <div className="browse-input">
            <Input
              value={directory}
              disabled={disabled}
              onChange={(event) => onDirectoryChange(event.target.value)}
              onBlur={onDirectoryBlur}
            />
            <ActionButton
              action={pickAction}
              label={t('选择强迫场数据路径', 'Choose forcing data path')}
              disabled={disabled}
              aria-label={t('强迫场数据路径 · 选择', 'Forcing data path · Browse')}
              run={onPick}
            >
              …
            </ActionButton>
          </div>
        </Field>
        <Field label={t('强迫场数据集', 'Forcing dataset')}>
          <div className="browse-input">
            <Select
              disabled={disabled}
              value={dataset}
              onChange={(event) => onDatasetChange(event.target.value)}
            >
              <option value="">
                {currentDataset
                  ? `${t('保留当前配置', 'Keep current configuration')}（${currentDataset}）`
                  : t('等待识别或手动选择', 'Identify or select manually')}
              </option>
              {datasets.map((name) => (
                <option key={name} value={name}>
                  {name}
                </option>
              ))}
            </Select>
            <ActionButton
              action={detectAction}
              label={t('识别数据集', 'Identify dataset')}
              disabled={disabled || !directory.trim() || probe?.status === 'loading'}
              run={onDetect}
            />
          </div>
        </Field>
      </div>
      <div aria-live="polite">
        <p className="field-hint">
          {currentDataset
            ? t(
                '只修改路径会保留当前强迫场设置；选择数据集后，保存时才改用源码内置模板。',
                'Changing only the path preserves current forcing settings. Selecting a dataset applies its source template when saving.',
              )
            : t(
                '按源码内置模板识别文件布局并生成算例配置；无法唯一识别时，请确认数据集。原始数据保留引用。',
                'Source templates identify the layout and generate case configuration. Confirm ambiguous datasets; source data stays in place.',
              )}
        </p>
        {catalogError && <Notice tone="error">{catalogError}</Notice>}
        {probe?.status === 'loading' && (
          <p role="status">{t('正在识别强迫场数据…', 'Identifying forcing data…')}</p>
        )}
        {probe?.error && <Notice tone="error">{probe.error}</Notice>}
        {probe?.status === 'done' && (
          <Notice tone={probe.candidates.length === 1 && !probe.truncated ? 'info' : 'warning'}>
            {probe.truncated
              ? t(
                  '目录内容较多，识别不完整，请手动确认数据集。',
                  'The directory listing is incomplete. Confirm the dataset manually.',
                )
              : probe.candidates.length === 1
                ? `${t('已识别文件布局', 'Identified file layout')}：${probe.candidates[0]}`
                : probe.candidates.length > 1
                  ? `${t('匹配多个模板，请确认数据集', 'Multiple templates match; confirm the dataset')}：${probe.candidates.join('、')}`
                  : t(
                      '未匹配内置模板，请确认配置或选择其他数据集。',
                      'No built-in layout matched. Check the configuration or select another dataset.',
                    )}
          </Notice>
        )}
        {currentDataset === undefined && probe?.status === 'done' && (
          <p className="field-hint">
            {probe.coverage
              ? `${t('已读取强迫场时间覆盖', 'Read forcing time coverage')}：${probe.coverage.start} — ${probe.coverage.end}`
              : `${t('未读取到时间覆盖，请手动设置模拟日期', 'Time coverage unavailable; enter simulation dates manually')}${probe.coverage_error ? `：${probe.coverage_error}` : ''}`}
          </p>
        )}
        <p className="field-hint">
          {t(
            '识别文件布局不代表变量、单位和完整时段已通过校验，实际输入仍会在运行时检查。',
            'Layout detection does not validate variables, units or full coverage. Actual inputs are checked when running.',
          )}
        </p>
      </div>
    </>
  );
}

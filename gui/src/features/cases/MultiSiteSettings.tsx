import { Button, Field, Input, Notice, Select } from '../../components/ui';
import { usePageContract, type UiField } from '../../lib/guide';
import { caseDirectory, type Site } from './case-model';
import {
  firstYearCutoff,
  siteDraft,
  type MultiSiteDraft,
  type MultiSiteMode,
} from './multisite-model';

export default function MultiSiteSettings({
  sites,
  drafts,
  mode,
  repeat,
  projectDirectory,
  disabled,
  en,
  onDraftsChange,
  onModeChange,
  onRepeatChange,
}: {
  sites: Site[];
  drafts: MultiSiteDraft[];
  mode: MultiSiteMode;
  repeat: string;
  projectDirectory: string;
  disabled: boolean;
  en: boolean;
  onDraftsChange: (drafts: MultiSiteDraft[]) => void;
  onModeChange: (mode: MultiSiteMode) => void;
  onRepeatChange: (repeat: string) => void;
}) {
  const t = (zh: string, english: string) => (en ? english : zh);
  const available = (site: Site) => !site.problem && !!site.met_file && !!site.start && !!site.end;
  const options = [
    {
      value: 'first-year',
      label: t(
        '统一预热：各站第一年重复 N 次',
        'Shared spinup: repeat each site’s first year N times',
      ),
    },
    { value: 'per-site', label: t('分站点配置', 'Configure each site') },
  ];
  function select(site: Site, checked: boolean) {
    if (disabled || (checked && !available(site))) return;
    onDraftsChange(
      checked
        ? drafts.some((draft) => draft.site === site.site_file)
          ? drafts
          : [...drafts, siteDraft(site)]
        : drafts.filter((draft) => draft.site !== site.site_file),
    );
  }
  function update(site: string, field: keyof Omit<MultiSiteDraft, 'site'>, value: string) {
    if (!disabled)
      onDraftsChange(
        drafts.map((draft) => (draft.site === site ? { ...draft, [field]: value } : draft)),
      );
  }
  function selectAll() {
    if (!disabled)
      onDraftsChange(
        sites
          .filter(available)
          .map((site) => drafts.find((draft) => draft.site === site.site_file) ?? siteDraft(site)),
      );
  }
  function clear() {
    if (!disabled) onDraftsChange([]);
  }
  const labels = {
    name: t('算例名称', 'Case name'),
    start: t('起始日期（包含预热）', 'Start date (including spinup)'),
    end: t('结束日期', 'End date'),
    spinup_end: t('预热结束 / 正式模拟开始', 'Spinup end / formal simulation start'),
    spinup_repeat: t('预热重复次数', 'Spinup repetitions'),
  };
  const rows = drafts.map((draft) => {
    let error = '';
    if (mode === 'first-year') {
      try {
        firstYearCutoff(draft.start);
      } catch {
        error = t(
          '起始日期没有有效的周年日期，请调整日期或改用分站点配置。',
          'No valid anniversary for this start date. Adjust it or use per-site settings.',
        );
      }
    }
    let destination = '';
    try {
      destination = caseDirectory(projectDirectory, draft.name);
    } catch {
      /* Invalid names are checked before creation. */
    }
    return { draft, error, destination };
  });
  const fields: UiField[] = [
    {
      id: 'multisite-mode',
      label: t('预热配置方式', 'Spinup configuration'),
      type: 'choice',
      value: mode,
      options,
      disabled,
      set: (value) => {
        if (!disabled && (value === 'first-year' || value === 'per-site')) onModeChange(value);
      },
    },
    ...(mode === 'first-year'
      ? [
          {
            id: 'multisite-repeat',
            label: labels.spinup_repeat,
            type: 'number' as const,
            value: repeat,
            disabled,
            set: (value: string) => {
              if (!disabled) onRepeatChange(value);
            },
          },
        ]
      : []),
    ...sites.map((site): UiField => ({
      id: `multisite-select:${site.site_file}`,
      label: site.name,
      type: 'switch',
      value: String(drafts.some((draft) => draft.site === site.site_file)),
      disabled: disabled || !available(site),
      set: (value) => select(site, value === 'true'),
    })),
    ...(mode === 'per-site' ? rows : []).flatMap(({ draft }): UiField[] =>
      (Object.keys(labels) as (keyof typeof labels)[]).map((field) => ({
        id: `multisite-${field}:${draft.site}`,
        label: `${draft.name}: ${labels[field]}`,
        type: field === 'spinup_repeat' ? 'number' : 'text',
        value: draft[field],
        disabled: disabled || (field === 'spinup_repeat' && !draft.spinup_end),
        set: (value) => {
          if (!(field === 'spinup_repeat' && !draft.spinup_end)) update(draft.site, field, value);
        },
      })),
    ),
  ];
  usePageContract({
    title: t('多站点设置', 'Multi-site settings'),
    step: 'new-case',
    fields,
    actions: [
      {
        id: 'multisite-select-all',
        label: t('全选可用站点', 'Select all available'),
        kind: 'click',
        enabled: !disabled,
        run: selectAll,
      },
      {
        id: 'multisite-clear',
        label: t('清空选择', 'Clear selection'),
        kind: 'click',
        enabled: !disabled,
        run: clear,
      },
    ],
  });
  return (
    <div className="page-stack">
      <div className="button-row">
        <Button disabled={disabled} onClick={selectAll}>
          {t('全选可用站点', 'Select all available')}
        </Button>
        <Button disabled={disabled} onClick={clear}>
          {t('清空选择', 'Clear selection')}
        </Button>
        <span>
          {t('已选站点：', 'Selected sites: ')}
          {drafts.length}
        </span>
      </div>
      {sites.map((site) => (
        <div key={site.site_file}>
          <label>
            <input
              type="checkbox"
              checked={drafts.some((draft) => draft.site === site.site_file)}
              disabled={disabled || !available(site)}
              onChange={(event) => select(site, event.target.checked)}
            />{' '}
            {site.name}
          </label>
          {!available(site) && (
            <Notice tone="warning">
              {site.problem ||
                t(
                  '缺少强迫场或有效的覆盖时段，无法选择。',
                  'Forcing or coverage dates are missing; this site cannot be selected.',
                )}
            </Notice>
          )}
        </div>
      ))}
      <Field label={t('预热配置方式', 'Spinup configuration')}>
        <Select
          value={mode}
          disabled={disabled}
          onChange={(event) => onModeChange(event.target.value as MultiSiteMode)}
        >
          {options.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </Select>
      </Field>
      {mode === 'first-year' ? (
        <>
          <Field label={labels.spinup_repeat}>
            <Input
              type="number"
              min="1"
              max="2147483647"
              step="1"
              value={repeat}
              disabled={disabled}
              onChange={(event) => onRepeatChange(event.target.value)}
            />
          </Field>
          <Notice>
            {t(
              '各站将第一年强迫场预热 N 次，之后从原始起始日期重新开始完整的正式模拟，输出包含第一年。此模式使用 Rust 计算核心。',
              'Each site warms up using its first year of forcing N times, then restarts the full formal simulation at the original start date. Output includes the first year. This mode uses the Rust engine.',
            )}
          </Notice>
        </>
      ) : (
        <Notice>
          {t(
            '各站可分别设置日期与预热次数。预热结束日期留空表示不预热；填写后，正式输出从该日期开始。',
            'Set dates and repetitions for each site. Leave spinup end blank to disable spinup; otherwise formal output begins on that date.',
          )}
        </Notice>
      )}
      {mode === 'first-year' &&
        rows
          .filter(({ error }) => error)
          .map(({ draft, error }) => (
            <Notice key={draft.site} tone="error">
              {sites.find((site) => site.site_file === draft.site)?.name ?? draft.name}: {error}
            </Notice>
          ))}
      {(mode === 'per-site' ? rows : []).map(({ draft, destination }) => (
        <section className="card" key={draft.site}>
          <h3>{sites.find((site) => site.site_file === draft.site)?.name ?? draft.name}</h3>
          <div className="form-grid">
            {(['name', 'start', 'end'] as const).map((field) => (
              <Field key={field} label={labels[field]}>
                <Input
                  type={field === 'name' ? 'text' : 'date'}
                  value={draft[field]}
                  disabled={disabled}
                  onChange={(event) => update(draft.site, field, event.target.value)}
                />
              </Field>
            ))}
            <Field label={labels.spinup_end}>
              <Input
                type="date"
                value={draft.spinup_end}
                disabled={disabled}
                onChange={(event) => {
                  update(draft.site, 'spinup_end', event.target.value);
                }}
              />
            </Field>
            {mode === 'per-site' && (
              <Field label={labels.spinup_repeat}>
                <Input
                  type="number"
                  min="1"
                  max="2147483647"
                  step="1"
                  value={draft.spinup_repeat}
                  disabled={disabled || !draft.spinup_end}
                  onChange={(event) => update(draft.site, 'spinup_repeat', event.target.value)}
                />
              </Field>
            )}
          </div>
          <p className="path">
            {t('保存位置：', 'Save to: ')}
            {destination ||
              t('请填写有效的项目目录与算例名称', 'Enter a valid project directory and case name')}
          </p>
        </section>
      ))}
    </div>
  );
}

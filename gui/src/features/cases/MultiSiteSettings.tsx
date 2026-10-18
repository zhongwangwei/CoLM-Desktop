import { useState } from 'react';
import { Button, Field, Input, DateInput, Notice, Select } from '../../components/ui';
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
  const unavailableReason = (site: Site) =>
    site.problem ||
    t(
      '缺少强迫场或有效的覆盖时段，无法选择。',
      'Forcing or coverage dates are missing; this site cannot be selected.',
    );
  const [filter, setFilter] = useState('');
  const query = filter.trim().toLowerCase();
  const shown = query ? sites.filter((site) => site.name.toLowerCase().includes(query)) : sites;
  const availableCount = sites.filter(available).length;
  const selectAllLabel = query
    ? t('全选筛选出的站点', 'Select all filtered')
    : t('全选可用站点', 'Select all available');
  const coordinate = (value: number, positive: string, negative: string) =>
    Number.isFinite(value)
      ? `${Math.abs(value).toFixed(2)}°${value < 0 ? negative : positive}`
      : '';
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
  // Selects every available site in view (all of them without a filter), keeping earlier picks.
  function selectAll() {
    if (disabled) return;
    const picked = new Set([
      ...drafts.map((draft) => draft.site),
      ...shown.filter(available).map((site) => site.site_file),
    ]);
    onDraftsChange(
      sites
        .filter((site) => available(site) && picked.has(site.site_file))
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
    ...(sites.length > 8
      ? [
          {
            id: 'multisite-filter',
            label: t('筛选站点', 'Filter sites'),
            type: 'text' as const,
            value: filter,
            set: setFilter,
          },
        ]
      : []),
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
        label: selectAllLabel,
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
      <div className="multisite-toolbar">
        {sites.length > 8 && (
          <Input
            type="search"
            className="multisite-filter"
            aria-label={t('筛选站点', 'Filter sites')}
            placeholder={t('按名称筛选站点', 'Filter sites by name')}
            value={filter}
            onChange={(event) => setFilter(event.target.value)}
          />
        )}
        <Button disabled={disabled} onClick={selectAll}>
          {selectAllLabel}
        </Button>
        <Button disabled={disabled} onClick={clear}>
          {t('清空选择', 'Clear selection')}
        </Button>
        <span className="multisite-count">
          {t(
            `已选 ${drafts.length} · 可用 ${availableCount} · 共 ${sites.length}`,
            `${drafts.length} selected · ${availableCount} available · ${sites.length} total`,
          )}
        </span>
      </div>
      <div className="multisite-grid" role="group" aria-label={t('站点', 'Sites')}>
        {shown.map((site) => {
          const checked = drafts.some((draft) => draft.site === site.site_file);
          const ok = available(site);
          return (
            <label
              key={site.site_file}
              className={`multisite-site${checked ? ' selected' : ''}${ok ? '' : ' unavailable'}`}
              title={ok ? site.name : unavailableReason(site)}
            >
              <input
                type="checkbox"
                checked={checked}
                disabled={disabled || !ok}
                onChange={(event) => select(site, event.target.checked)}
              />
              <span className="multisite-site-body">
                <span className="multisite-site-name">
                  {site.name}
                  {site.crop && <span className="multisite-tag">{t('作物', 'Crop')}</span>}
                  {site.urban && <span className="multisite-tag">{t('城市', 'Urban')}</span>}
                </span>
                {ok ? (
                  <>
                    <span className="multisite-site-meta">{`${site.start} → ${site.end}`}</span>
                    <span className="multisite-site-meta">
                      {[coordinate(site.lat, 'N', 'S'), coordinate(site.lon, 'E', 'W')]
                        .filter(Boolean)
                        .join(' ')}
                    </span>
                  </>
                ) : (
                  <span className="multisite-site-problem">{unavailableReason(site)}</span>
                )}
              </span>
            </label>
          );
        })}
      </div>
      {shown.length === 0 && (
        <p className="muted">{t('没有名称匹配的站点。', 'No site name matches.')}</p>
      )}
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
              '各站将第一年强迫场预热 N 次，之后从原始起始日期重新开始完整的正式模拟，输出包含第一年。此模式使用 Rust 计算引擎。',
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
      <div className="multisite-cards">
        {(mode === 'per-site' ? rows : []).map(({ draft, destination }) => (
          <section className="card multisite-card" key={draft.site}>
            <h3>{sites.find((site) => site.site_file === draft.site)?.name ?? draft.name}</h3>
            <div className="form-grid">
              {(['name', 'start', 'end'] as const).map((field) => (
                <Field key={field} label={labels[field]}>
                  {field === 'name' ? (
                    <Input
                      type="text"
                      value={draft[field]}
                      disabled={disabled}
                      onChange={(event) => update(draft.site, field, event.target.value)}
                    />
                  ) : (
                    <DateInput
                      calendarLabel={`${t('选择日期', 'Choose date')} · ${labels[field]}`}
                      value={draft[field]}
                      disabled={disabled}
                      onChange={(event) => update(draft.site, field, event.target.value)}
                    />
                  )}
                </Field>
              ))}
              <Field label={labels.spinup_end}>
                <DateInput
                  calendarLabel={`${t('选择日期', 'Choose date')} · ${labels.spinup_end}`}
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
                t(
                  '请填写有效的项目目录与算例名称',
                  'Enter a valid project directory and case name',
                )}
            </p>
          </section>
        ))}
      </div>
    </div>
  );
}

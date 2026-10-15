import { useMemo, useState } from 'react';
import { Button, Card, Input } from '../../components/ui';
import {
  RECOMMENDED_VARIABLES,
  historyKey,
  historyPreset,
  historySelection,
  type HistoryVariable,
  type HistoryVariableItem,
} from './history-variables';

const CATEGORY_LABELS: Record<string, [string, string]> = {
  all: ['全部变量', 'All variables'],
  recommended: ['推荐核心量', 'Recommended'],
  enabled: ['已开启', 'Enabled'],
  energy: ['能量', 'Energy'],
  radiation: ['辐射', 'Radiation'],
  water: ['水分', 'Water'],
  temperature: ['温度', 'Temperature'],
  forcing: ['气象', 'Meteorology'],
  carbon: ['碳与植被', 'Carbon & Veg'],
  other: ['其他', 'Other'],
};

type Props = {
  // 当前内核与配置下 CoLM 历史场全集；settable=false 的写不出来，只读展示原因。
  variables: HistoryVariable[];
  // 已有算例给出每个开关的实际值；新建算例未写的开关沿用 CoLM 默认（开启）。
  values: Record<string, string>;
  defaultOn?: boolean;
  disabled?: boolean;
  en?: boolean;
  onChange: (patch: Record<string, string>) => void;
};

// 输出变量卡片：推荐核心量是科研常用的主要通量/状态量，全部变量是可写出的历史场全集。
// 每个开关都显式写入，不依赖“默认输出历史变量”这一同步规则。
export default function HistoryVariablesCard({
  variables,
  values,
  defaultOn = false,
  disabled = false,
  en = false,
  onChange,
}: Props) {
  const t = (zh: string, eng: string) => (en ? eng : zh);
  const [search, setSearch] = useState('');
  const [filterTab, setFilterTab] = useState<'all' | 'recommended' | 'enabled'>('recommended');
  const effective = useMemo(() => {
    if (!defaultOn) return values;
    const filled = { ...values };
    for (const v of variables) filled[historyKey(v.name)] ??= '.true.';
    return filled;
  }, [values, variables, defaultOn]);
  const selection = historySelection(variables, effective);
  const recommended = useMemo(
    () => new Map(RECOMMENDED_VARIABLES.map((item) => [item.name.toLowerCase(), item])),
    [],
  );
  const catalog = useMemo(
    () =>
      variables.map((v): HistoryVariableItem & { settable: boolean; reason?: string | null } => {
        const known = recommended.get(v.name.toLowerCase());
        return {
          ...(known ?? {
            name: v.name,
            label_zh: v.longName || v.name,
            label_en: v.longName || v.name,
            category: 'other' as const,
            long_name: v.longName ?? undefined,
          }),
          name: v.name,
          recommended: !!known,
          settable: v.settable,
          reason: v.reason,
        };
      }),
    [variables, recommended],
  );
  const isVarOn = (name: string) => effective[historyKey(name)] === '.true.';
  const filtered = useMemo(() => {
    const q = search.trim().toLowerCase();
    return catalog
      .filter((item) => {
        if (filterTab === 'recommended' && !item.recommended) return false;
        if (filterTab === 'enabled' && !(item.settable && isVarOn(item.name))) return false;
        if (filterTab === 'all' && !item.settable) return false;
        if (!q) return true;
        return [item.name, item.label_zh, item.label_en, item.long_name ?? '']
          .join(' ')
          .toLowerCase()
          .includes(q);
      })
      .sort(
        (a, b) => Number(b.recommended) - Number(a.recommended) || a.name.localeCompare(b.name),
      );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [catalog, search, filterTab, effective]);
  const unwritable = catalog.filter((item) => !item.settable);

  return (
    <Card
      title={t('输出变量', 'Output variables')}
      description={t(
        '选择写入历史文件（NetCDF）的变量：推荐核心量为科研常用的主要通量与状态量，全部变量为当前内核与配置下可写出的 CoLM 历史场全集。',
        'Choose variables written to history files: the recommended set covers the main fluxes and states; all variables is every CoLM history field writable with the current kernel and settings.',
      )}
      className="history-variables-card"
    >
      <div className="button-row history-variable-actions">
        <Button
          disabled={disabled || !selection.core.length}
          onClick={() => onChange(historyPreset(variables, 'core'))}
        >
          ★ {t('仅推荐核心量', 'Recommended only')} ({selection.core.length})
        </Button>
        <Button
          disabled={disabled || !selection.available.length}
          onClick={() => onChange(historyPreset(variables, 'all'))}
        >
          {t('一键全开启', 'Turn all on')} ({selection.available.length})
        </Button>
        <Button
          disabled={disabled || !selection.selected}
          onClick={() => onChange(historyPreset(variables, 'none'))}
        >
          {t('一键全清除', 'Clear all')}
        </Button>
        <span className="muted history-variable-count">
          {t(
            `已开启 ${selection.selected} / ${selection.available.length}`,
            `${selection.selected} of ${selection.available.length} enabled`,
          )}
        </span>
      </div>
      <div className="history-variable-filters">
        {(
          [
            ['recommended', selection.core.length],
            ['enabled', selection.selected],
            ['all', selection.available.length],
          ] as const
        ).map(([tab, count]) => (
          <Button
            key={tab}
            variant={filterTab === tab ? 'primary' : 'secondary'}
            onClick={() => setFilterTab(tab)}
          >
            {t(CATEGORY_LABELS[tab][0], CATEGORY_LABELS[tab][1])} ({count})
          </Button>
        ))}
        <Input
          value={search}
          placeholder={t(
            '搜索变量名或描述（例如 fsena、潜热、soil）',
            'Search name or description',
          )}
          onChange={(e) => setSearch(e.target.value)}
          disabled={disabled}
        />
      </div>
      <div className="history-variable-table">
        {filtered.length === 0 ? (
          <p className="muted">{t('没有找到匹配的变量', 'No matching variables found')}</p>
        ) : (
          <table>
            <thead>
              <tr>
                <th>{t('输出', 'Output')}</th>
                <th>{t('变量代码', 'Variable')}</th>
                <th>{t('中文描述与物理意义', 'Description')}</th>
                <th>{t('分类', 'Category')}</th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((item) => (
                <tr key={item.name} className={item.settable ? '' : 'history-variable-blocked'}>
                  <td>
                    {/* 写不出的变量不给勾选框：配置里开着也不会有输出。 */}
                    {item.settable ? (
                      <input
                        type="checkbox"
                        checked={isVarOn(item.name)}
                        disabled={disabled}
                        aria-label={item.name}
                        onChange={(e) =>
                          onChange({
                            [historyKey(item.name)]: e.target.checked ? '.true.' : '.false.',
                          })
                        }
                      />
                    ) : (
                      <span aria-hidden="true">—</span>
                    )}
                  </td>
                  <td className="history-variable-name">
                    {item.name}
                    {item.recommended && <span title={t('推荐核心量', 'Recommended')}> ★</span>}
                  </td>
                  <td>
                    <strong>{en ? item.label_en : item.label_zh}</strong>
                    {!item.settable && item.reason ? (
                      <div className="muted">{item.reason}</div>
                    ) : (
                      item.long_name &&
                      item.long_name !== item.label_zh &&
                      item.long_name !== item.label_en && (
                        <div className="muted">{item.long_name}</div>
                      )
                    )}
                  </td>
                  <td>
                    <span className="badge">
                      {t(
                        CATEGORY_LABELS[item.category]?.[0] || '其他',
                        CATEGORY_LABELS[item.category]?.[1] || 'Other',
                      )}
                    </span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
      {defaultOn && (
        <p className="field-hint">
          {t(
            '新建时尚未按内核核对；创建后在算例设置中会标出当前内核与配置下写不出的变量。',
            'Not yet checked against a kernel; after creation the case settings mark variables the kernel cannot write.',
          )}
        </p>
      )}
      {unwritable.length > 0 && (
        <p className="field-hint">
          {t(
            `另有 ${unwritable.length} 个变量在当前内核或配置下写不出，已从“全部变量”中排除（推荐核心量里如有，会注明原因）。`,
            `${unwritable.length} more variables cannot be written with the current kernel or settings and are excluded from “All variables”.`,
          )}
        </p>
      )}
    </Card>
  );
}

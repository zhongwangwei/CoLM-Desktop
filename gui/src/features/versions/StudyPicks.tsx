import { useEffect, useState } from 'react';
import { invokeJson } from '../../lib/ipc';
import type { StudyPick } from '../../lib/types';
import { Button, Input, Select } from '../../components/ui';
import { parameterType, type Descriptor } from './model';
import { literalNumber } from './ParameterDialog';
import { IGBP, PFT, USGS } from './scope-labels';

type TypeDefault = { index: number; value: number | null };

const storageKey = (owner: string) => `colm.versions.studyPicks.${owner}`;

/** 这个版本（或草稿）上次选的要研究的参数；只是本机的方便，读不到就当没有。 */
export function loadPicks(owner: string): StudyPick[] {
  try {
    const text = localStorage.getItem(storageKey(owner));
    const value = text ? JSON.parse(text) : [];
    return Array.isArray(value) ? value : [];
  } catch {
    return [];
  }
}

function savePicks(owner: string, picks: StudyPick[]) {
  try {
    localStorage.setItem(storageKey(owner), JSON.stringify(picks));
  } catch {
    // 存不下只是下次要重选。
  }
}

const trim = (value: number) => String(Number(value.toPrecision(6)));

/** 一个参数的默认研究范围：目录给了有限的上下界就用它，否则取默认值的 0.5–1.5 倍。 */
export function defaultRange(d: Descriptor, base: number | null): [string, string] {
  const min = d.validation.min == null ? null : literalNumber(d.validation.min);
  const max = d.validation.max == null ? null : literalNumber(d.validation.max);
  if (base != null && base !== 0) {
    const low = base * 0.5,
      high = base * 1.5;
    return [
      trim(min != null ? Math.max(min, Math.min(low, high)) : Math.min(low, high)),
      trim(max != null ? Math.min(max, Math.max(low, high)) : Math.max(low, high)),
    ];
  }
  return [min != null ? trim(min) : '', max != null ? trim(max) : ''];
}

/**
 * 要研究的参数：在版本工作台里先定好研究哪些参数（地类、PFT 参数可以逐个类型选）和范围，
 * 再带去研究页选算例和目标。研究页按算例加载参数行后自动勾上这些（2026-10-20：把研究整合进版本）。
 */
export default function StudyPicks({
  owner,
  catalog,
  en,
  disabled,
  onChange,
}: {
  /** 记住选择用：草稿编号或版本的 id#revision。 */
  owner: string;
  catalog: Descriptor[];
  en: boolean;
  disabled: boolean;
  onChange: (picks: StudyPick[]) => void;
}) {
  const t = (zh: string, english: string) => (en ? english : zh);
  const [picks, setPicks] = useState<StudyPick[]>(() => loadPicks(owner));
  const [chosen, setChosen] = useState('');
  const [typeIndex, setTypeIndex] = useState(1);
  const [problem, setProblem] = useState('');
  useEffect(() => {
    const loaded = loadPicks(owner);
    setPicks(loaded);
    onChange(loaded);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [owner]);
  const eligible = catalog.filter((d) => d.calibration_eligible && parameterType(d) !== 'process');
  const descriptor = eligible.find((d) => d.id === chosen);
  const kind = descriptor ? parameterType(descriptor) : 'global';
  const types =
    kind === 'igbp' ? IGBP : kind === 'usgs' ? USGS : PFT.filter(([id]) => id >= 1 && id <= 16);
  function commit(next: StudyPick[]) {
    setPicks(next);
    savePicks(owner, next);
    onChange(next);
  }
  async function add() {
    if (!descriptor) return;
    const perType = kind !== 'global';
    const index = perType ? typeIndex : null;
    if (picks.some((p) => p.parameter_id === descriptor.id && p.index === index))
      return setProblem(t('这个参数已经在列表里了。', 'This parameter is already listed.'));
    let base = descriptor.default == null ? null : literalNumber(descriptor.default);
    if (perType) {
      const defaults = await invokeJson<TypeDefault[]>('parameter_type_defaults', {
        parameterId: descriptor.id,
      });
      base = defaults.find((d) => d.index === index)?.value ?? null;
    }
    const [min, max] = defaultRange(descriptor, base);
    setProblem('');
    commit([
      ...picks,
      {
        parameter_id: descriptor.id,
        raw_key: descriptor.raw_key,
        scheme: kind === 'igbp' ? 'IGBP' : kind === 'usgs' ? 'USGS' : null,
        index,
        min,
        max,
        scale: descriptor.recommended_scale === 'log' ? 'log' : 'linear',
      },
    ]);
  }
  const typeLabel = (p: StudyPick) => {
    if (p.index == null) return '—';
    const table = p.scheme === 'USGS' ? USGS : p.scheme === 'IGBP' ? IGBP : PFT;
    const hit = table.find(([id]) => id === p.index);
    return hit ? `${p.index} ${en ? hit[2] : hit[1]}` : String(p.index);
  };
  const label = (p: StudyPick) => {
    const d = catalog.find((x) => x.id === p.parameter_id);
    return d ? (en ? d.label_en : d.label_zh) : p.raw_key;
  };
  return (
    <div className="study-picks">
      <div className="study-picks-add">
        <Select
          value={chosen}
          disabled={disabled}
          onChange={(e) => {
            setChosen(e.target.value);
            setTypeIndex(1);
          }}
        >
          <option value="">{t('选一个可率定的参数…', 'Choose a calibratable parameter…')}</option>
          {eligible.map((d) => (
            <option key={d.id} value={d.id}>
              {en ? d.label_en : d.label_zh} · {d.raw_key}
              {parameterType(d) === 'igbp'
                ? ' · IGBP'
                : parameterType(d) === 'usgs'
                  ? ' · USGS'
                  : parameterType(d) === 'pc'
                    ? ' · PC'
                    : ''}
            </option>
          ))}
        </Select>
        {descriptor && kind !== 'global' && (
          <Select
            value={typeIndex}
            disabled={disabled}
            onChange={(e) => setTypeIndex(Number(e.target.value))}
          >
            {types.map(([id, zh, english]) => (
              <option key={id} value={id}>
                {id} · {en ? english : zh}
              </option>
            ))}
          </Select>
        )}
        <Button
          disabled={disabled || !descriptor}
          onClick={() => void add().catch((e) => setProblem(String(e)))}
        >
          {t('加入', 'Add')}
        </Button>
      </div>
      {problem && <p className="param-problem">{problem}</p>}
      {picks.length === 0 ? (
        <p className="versions-empty">
          {t(
            '还没有选。研究页里也可以再选；在这里选好的会带过去自动勾上。',
            'Nothing chosen yet. You can also choose on the research page; whatever is chosen here is ticked there automatically.',
          )}
        </p>
      ) : (
        <table className="versions-table">
          <thead>
            <tr>
              <th>{t('参数', 'Parameter')}</th>
              <th>{t('地类 / 植物类型', 'Class / plant type')}</th>
              <th>{t('下限', 'Min')}</th>
              <th>{t('上限', 'Max')}</th>
              <th>{t('尺度', 'Scale')}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {picks.map((p, i) => (
              <tr key={`${p.parameter_id}|${p.index ?? ''}`}>
                <td>
                  <strong>{label(p)}</strong>
                  <code className="param-key">{p.raw_key}</code>
                </td>
                <td className="muted">{typeLabel(p)}</td>
                {(['min', 'max'] as const).map((field) => (
                  <td key={field}>
                    <Input
                      className="param-input"
                      value={p[field]}
                      disabled={disabled}
                      onChange={(e) =>
                        commit(
                          picks.map((x, j) => (j === i ? { ...x, [field]: e.target.value } : x)),
                        )
                      }
                    />
                  </td>
                ))}
                <td>
                  <Select
                    value={p.scale}
                    disabled={disabled}
                    onChange={(e) =>
                      commit(picks.map((x, j) => (j === i ? { ...x, scale: e.target.value } : x)))
                    }
                  >
                    <option value="linear">{t('线性', 'Linear')}</option>
                    <option value="log">{t('对数', 'Log')}</option>
                  </Select>
                </td>
                <td className="versions-table-action">
                  <Button
                    variant="ghost"
                    disabled={disabled}
                    onClick={() => commit(picks.filter((_, j) => j !== i))}
                  >
                    {t('移除', 'Remove')}
                  </Button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

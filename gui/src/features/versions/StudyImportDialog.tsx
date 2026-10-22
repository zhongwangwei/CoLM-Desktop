import { useEffect, useState } from 'react';
import type { Overlay, ParameterOverride } from '../../lib/types';
import { Dialog, Notice } from '../../components/ui';
import { ActionButton } from '../../components/ActionButton';
import { IGBP, PFT, USGS } from './scope-labels';
import { fieldLabel } from '../../lib/param-presentation';

export type StudyPreview = {
  study: string;
  member: string;
  objective: number | null;
  records: ParameterOverride[];
};

/** 研究结果里一个参数的身份：和后端 `--only` 一样的 `[parameter_id, scheme, index]`。 */
export const chosenKey = (r: ParameterOverride): [string, string | null, number | null] => [
  r.parameter_id,
  r.scope_instance.scheme ?? null,
  r.scope_instance.index ?? null,
];

/**
 * 研究结果作为建议改动：列出研究成员定下的每个参数、草稿里现在的值和研究的值，勾选后才写进草稿。
 * 原来"从研究导入"一次把全部参数写进去，没法只要其中几个（2026-10-20：把研究整合进版本）。
 */
export default function StudyImportDialog({
  preview,
  overlay,
  en,
  busy,
  error,
  onClose,
  onImport,
}: {
  preview: StudyPreview | null;
  overlay: Overlay;
  en: boolean;
  busy: boolean;
  /** 写入失败的原因。对话框是模态的，页面上的错误提示被遮在后面看不见，写不进去时按钮像没反应。 */
  error?: string;
  onClose: () => void;
  onImport: (chosen: [string, string | null, number | null][]) => void;
}) {
  const t = (zh: string, english: string) => (en ? english : zh);
  const [checked, setChecked] = useState<boolean[]>([]);
  useEffect(() => {
    setChecked(preview ? preview.records.map(() => true) : []);
  }, [preview]);
  if (!preview) return null;
  const current = (r: ParameterOverride) =>
    overlay.parameters.find(
      (x) =>
        x.parameter_id === r.parameter_id &&
        (x.scope_instance.index ?? null) === (r.scope_instance.index ?? null) &&
        (x.scope_instance.scheme ?? null) === (r.scope_instance.scheme ?? null),
    )?.value ?? null;
  const scope = (r: ParameterOverride) => {
    const s = r.scope_instance;
    if (s.index == null) return '—';
    const table = s.kind === 'land-cover-class' ? (s.scheme === 'USGS' ? USGS : IGBP) : PFT;
    const hit = table.find(([id]) => id === s.index);
    return hit ? `${s.index} ${en ? hit[2] : hit[1]}` : String(s.index);
  };
  // 和研究页的参数表一样：有界面名就名字在前、键名在后；名称表里没有的（如 PFT 参数）只显示键名。
  const name = (key: string) => {
    const label = fieldLabel(key, en ? 'en' : 'zh');
    return key.toUpperCase().endsWith(label.toUpperCase()) ? (
      <code>{key}</code>
    ) : (
      <>
        <strong>{label}</strong> <code className="param-key">{key}</code>
      </>
    );
  };
  const chosen = preview.records.filter((_, i) => checked[i]);
  return (
    <Dialog
      open
      // 取消和写进去并排放在一行；对话框自带的"关闭"会单独落到下一行。
      showClose={false}
      onOpenChange={(open) => !open && !busy && onClose()}
      className="study-import"
      title={t('从研究导入参数', 'Import parameters from a study')}
      description={t(
        `研究成员 ${preview.member}${preview.objective != null ? `（目标函数 ${preview.objective.toPrecision(4)}）` : ''}。勾选要写进草稿的参数；没勾的保持草稿里现在的值。`,
        `Study member ${preview.member}${preview.objective != null ? ` (objective ${preview.objective.toPrecision(4)})` : ''}. Tick the parameters to write into the draft; the rest keep the draft’s values.`,
      )}
    >
      <p className="muted versions-hint">{preview.study}</p>
      <table className="versions-table">
        <thead>
          <tr>
            <th>
              <input
                type="checkbox"
                aria-label={t('全选', 'Select all')}
                checked={checked.length > 0 && checked.every(Boolean)}
                onChange={(e) => setChecked(preview.records.map(() => e.target.checked))}
              />
            </th>
            <th>{t('参数', 'Parameter')}</th>
            <th>{t('地类 / 植物类型', 'Class / plant type')}</th>
            <th>{t('草稿现在', 'Draft now')}</th>
            <th>{t('研究的值', 'Study value')}</th>
          </tr>
        </thead>
        <tbody>
          {preview.records.map((r, i) => (
            <tr key={chosenKey(r).join('|')}>
              <td>
                <input
                  type="checkbox"
                  aria-label={r.raw_key}
                  checked={!!checked[i]}
                  onChange={(e) =>
                    setChecked(checked.map((value, j) => (j === i ? e.target.checked : value)))
                  }
                />
              </td>
              <td>{name(r.raw_key)}</td>
              <td className="muted">{scope(r)}</td>
              <td className="muted">
                <code>{current(r) ?? t('默认', 'default')}</code>
              </td>
              <td>
                <code>{r.value}</code>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {error && <Notice tone="error">{error}</Notice>}
      <div className="button-row">
        <ActionButton
          action="version-import-study-cancel"
          label={t('取消', 'Cancel')}
          disabled={busy}
          run={onClose}
        >
          {t('取消', 'Cancel')}
        </ActionButton>
        <ActionButton
          action="version-import-study-apply"
          label={t('写进草稿', 'Write into the draft')}
          kind="commit"
          variant="primary"
          disabled={busy || chosen.length === 0}
          run={() => onImport(chosen.map(chosenKey))}
        >
          {t(`写进草稿（${chosen.length} 个）`, `Write into the draft (${chosen.length})`)}
        </ActionButton>
      </div>
    </Dialog>
  );
}

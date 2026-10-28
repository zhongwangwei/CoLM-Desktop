import { useEffect, useState } from 'react';
import type { Overlay, ParameterOverride } from '../../lib/types';
import { Dialog, Notice } from '../../components/ui';
import { ActionButton } from '../../components/ActionButton';
import { IGBP, PFT, USGS } from './scope-labels';
import { fieldLabel } from '../../lib/param-presentation';

/**
 * 一个研究成员训练出的网络摘要（后端 `study_network_summary`）：确认对话框用它说明
 * 带进来的是哪个插槽、几个特征/输出/权重，而不是只写"导入参数"。
 */
export type StudyNetworkSummary = {
  slot: string;
  features: string[];
  outputs: string[];
  hidden: number[];
  normalize: boolean;
  weights: number;
};

/** `model_import_study` 预览：研究成员会带进来的产物。 */
export type StudyPreview = {
  study: string;
  member: string;
  objective: number | null;
  records: ParameterOverride[];
  /** 产物类型；参数导入的旧后端没有这一项，按纯参数处理。 */
  kind?: 'parameters' | 'network' | 'joint';
  network?: StudyNetworkSummary | null;
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
  // 审计 P1-5：DE 直接训练出的网络以前只能走参数导入，后端拒绝网络权重，用户在保存这一步卡住。
  // 现在按产物类型分流：纯参数（旧行为）、网络、参数+网络；网络没有可勾选的参数，整块导入。
  const kind = preview.kind ?? 'parameters';
  const network = kind === 'parameters' ? null : (preview.network ?? null);
  const title =
    kind === 'network'
      ? t('从研究导入 AI 网络', 'Import an AI network from a study')
      : kind === 'joint'
        ? t('从研究导入参数与 AI 网络', 'Import parameters and an AI network from a study')
        : t('从研究导入参数', 'Import parameters from a study');
  const description =
    kind === 'parameters'
      ? t(
          `研究成员 ${preview.member}${preview.objective != null ? `（目标函数 ${preview.objective.toPrecision(4)}）` : ''}。勾选要写进草稿的参数；没勾的保持草稿里现在的值。`,
          `Study member ${preview.member}${preview.objective != null ? ` (objective ${preview.objective.toPrecision(4)})` : ''}. Tick the parameters to write into the draft; the rest keep the draft’s values.`,
        )
      : t(
          `研究成员 ${preview.member}${preview.objective != null ? `（目标函数 ${preview.objective.toPrecision(4)}）` : ''} 训练了网络；网络会整体写进草稿${kind === 'joint' ? '，另外勾选要一起写进去的参数' : ''}。写完后在版本页审阅再冻结。`,
          `Study member ${preview.member}${preview.objective != null ? ` (objective ${preview.objective.toPrecision(4)})` : ''} trained a network; the network is written into the draft as a whole${kind === 'joint' ? ', and you can tick parameters to write in with it' : ''}. Review and freeze it on the versions page.`,
        );
  return (
    <Dialog
      open
      // 取消和写进去并排放在一行；对话框自带的"关闭"会单独落到下一行。
      showClose={false}
      onOpenChange={(open) => !open && !busy && onClose()}
      className="study-import"
      title={title}
      description={description}
    >
      <p className="muted versions-hint">{preview.study}</p>
      {network && (
        <Notice>
          {t(
            `网络插槽 ${network.slot}：${network.features.length} 个特征 → ${network.outputs.join('、') || '—'}（${network.hidden.length ? `隐藏层 ${network.hidden.join('×')}` : '线性'}，${network.weights} 个权重${network.normalize ? '，含冻结的特征标准化' : '，没有特征标准化'}）。`,
            `Network slot ${network.slot}: ${network.features.length} features → ${network.outputs.join(', ') || '—'} (${network.hidden.length ? `hidden ${network.hidden.join('×')}` : 'linear'}, ${network.weights} weights${network.normalize ? ', with frozen feature normalisation' : ', without feature normalisation'}).`,
          )}
        </Notice>
      )}
      {kind !== 'network' && (
        <>
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
        </>
      )}
      {kind !== 'network' && chosen.length > 0 && chosen.length < preview.records.length && (
        <Notice tone="warning">
          {t(
            '只导入一部分参数：草稿里的参数组合和这项研究的成绩不对应，冻结后需要复跑一次对照再下结论。',
            'Only some parameters are imported: the draft’s parameter combination does not match the study’s score, so run a control comparison after freezing before drawing conclusions.',
          )}
        </Notice>
      )}
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
          disabled={busy || (kind !== 'network' && chosen.length === 0)}
          run={() => onImport(chosen.map(chosenKey))}
        >
          {kind === 'network'
            ? t('写进草稿（含网络）', 'Write into the draft (with the network)')
            : kind === 'joint'
              ? t(
                  `写进草稿（${chosen.length} 个参数 + 网络）`,
                  `Write into the draft (${chosen.length} parameters + network)`,
                )
              : t(`写进草稿（${chosen.length} 个）`, `Write into the draft (${chosen.length})`)}
        </ActionButton>
      </div>
    </Dialog>
  );
}

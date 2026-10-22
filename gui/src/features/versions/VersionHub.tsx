import type { FrozenRevision, ModelDraft, VersionRef } from '../../lib/types';
import { Card, Field, Select } from '../../components/ui';
import { ActionButton } from '../../components/ActionButton';
import { hubMethods, type HubMethod } from './hub-methods';
import { changeSummary } from './VersionLibrary';
// 和原研究首页同一套方式卡片的样式（研究页是按需加载的，这里不能指望它已经载入）。
import '../studies/research-home.css';

const keyOf = (v: VersionRef) => `${v.id}#${v.revision}`;

export default function VersionHub({
  versions,
  drafts,
  base,
  inUse,
  en,
  disabled,
  onBase,
  onMethod,
  onOpenDraft,
  onLibrary,
}: {
  versions: FrozenRevision[];
  drafts: ModelDraft[];
  /** 新版本基于哪个版本。 */
  base: FrozenRevision | null;
  inUse: FrozenRevision | null;
  en: boolean;
  disabled: boolean;
  onBase: (key: string) => void;
  onMethod: (method: HubMethod) => void;
  onOpenDraft: (id: string) => void;
  onLibrary: () => void;
}) {
  const t = (zh: string, english: string) => (en ? english : zh);
  const research = (id: string) => !['manual', 'source'].includes(id);
  return (
    <div className="version-hub">
      <div className="page-heading versions-heading">
        <div>
          <h1>{t('新建修改版', 'New modified version')}</h1>
          <p className="muted">
            {t(
              '选一种改模型的方式，点了就开始。手动改参数和修改源码直接建好草稿；率定、不确定性分析和 AI 先跑研究，跑完把结果写进一个新版本的草稿，审阅后冻结。',
              'Choose how to change the model; it starts right away. Editing parameters or code creates a draft directly; calibration, uncertainty analysis and AI first run a study, whose results go into a new version draft that you review and freeze.',
            )}
          </p>
        </div>
        <ActionButton
          action="version-library"
          label={t('所有版本', 'All versions')}
          variant="ghost"
          run={onLibrary}
        >
          {t('所有版本 →', 'All versions →')}
        </ActionButton>
      </div>
      <div className="version-hub-base">
        <Field label={t('基于哪个版本', 'Based on')}>
          <Select
            value={base ? keyOf(base.version) : ''}
            disabled={disabled}
            onChange={(e) => onBase(e.target.value)}
          >
            {versions.map((v) => (
              <option key={keyOf(v.version)} value={keyOf(v.version)}>
                {v.name} · r{v.version.revision}
                {v.version.id === 'official' ? ` (${t('官方', 'official')})` : ''}
              </option>
            ))}
          </Select>
        </Field>
        {inUse && (
          <p className="muted">
            {t(
              `模拟正在使用 ${inUse.name} · r${inUse.version.revision}`,
              `Simulations use ${inUse.name} · r${inUse.version.revision}`,
            )}
          </p>
        )}
      </div>
      <div className="research-methods version-hub-methods" role="group">
        {hubMethods.map((item) => (
          // 手动改参数和修改源码会建草稿（写盘），按 commit 登记；四种研究只是切到研究页。
          <ActionButton
            key={item.id}
            id={`version-method-${item.id}`}
            action={`version-method-${item.id}`}
            label={en ? item.en : item.zh}
            kind={research(item.id) ? 'click' : 'commit'}
            className="research-method"
            disabled={disabled || !base}
            run={() => onMethod(item.id)}
          >
            <span className="research-method-icon">
              <svg
                viewBox="0 0 24 24"
                aria-hidden="true"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.8"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d={item.icon} />
              </svg>
            </span>
            <span className="research-method-copy">
              <strong>{en ? item.en : item.zh}</strong>
              <span>{en ? item.descriptionEn : item.descriptionZh}</span>
              {research(item.id) && (
                <span className="version-hub-tag">
                  {t('先研究，再写入新版本', 'Study first, then a new version')}
                </span>
              )}
            </span>
            <span className="research-method-arrow" aria-hidden="true">
              ›
            </span>
          </ActionButton>
        ))}
      </div>
      {drafts.length > 0 && (
        <Card title={t('继续之前的修改', 'Continue earlier work')}>
          <ul className="version-hub-drafts">
            {drafts.map((d) => (
              <li key={d.id}>
                <ActionButton
                  bare
                  action={`version-draft-${d.id}`}
                  label={d.name}
                  disabled={disabled}
                  run={() => onOpenDraft(d.id)}
                >
                  <strong>{d.name}</strong>
                  <span className="muted">{changeSummary(d.overlay, !!d.code.workspace, en)}</span>
                </ActionButton>
              </li>
            ))}
          </ul>
        </Card>
      )}
    </div>
  );
}

import { Button, Input, Notice } from '../../components/ui';
import {
  knowledgeSaveReview,
  localPathsInAction,
  taskActionOutput,
  taskActionTitle,
  type EvidenceField,
  type EvidenceItem,
  type EvidenceSection,
  type HistoryEvidence,
  type PendingReconciliation,
} from './model';

type Translate = (zh: string, en: string) => string;

/**
 * 证据卡渲染：只显示 model.ts 里 `historyEvidence` 从结构化字段整理出的内容。
 * 缺字段显示“未提供/不可用”由构建器和这里共同保证；这里不做任何文本推断。
 */
function EvidenceFieldView({
  field,
  t,
  onOpenPath,
  onOpenUrl,
}: {
  field: EvidenceField;
  t: Translate;
  onOpenPath: (path: string) => void;
  onOpenUrl: (url: string) => void;
}) {
  return (
    <div className="flex flex-wrap items-baseline gap-1 text-xs">
      <span className="text-muted-foreground">
        {field.label}
        {t('：', ': ')}
      </span>
      {field.mono ? (
        <code className="select-all break-all">{field.value}</code>
      ) : (
        <span className="break-words">{field.value}</span>
      )}
      {field.path && (
        <Button variant="ghost" onClick={() => onOpenPath(field.path!)}>
          {t('打开文件', 'Open file')}
        </Button>
      )}
      {field.url && (
        <Button variant="ghost" onClick={() => onOpenUrl(field.url!)}>
          {field.linkLabel || t('打开链接', 'Open link')}
        </Button>
      )}
    </div>
  );
}

function EvidenceItemView({
  item,
  t,
  onOpenPath,
  onOpenUrl,
}: {
  item: EvidenceItem;
  t: Translate;
  onOpenPath: (path: string) => void;
  onOpenUrl: (url: string) => void;
}) {
  return (
    <details className="rounded border p-1">
      <summary className="cursor-pointer text-xs">
        {item.title || t('未提供标题', 'untitled')}
      </summary>
      <div className="space-y-1 pt-1">
        {item.fields.map((field, index) => (
          <EvidenceFieldView
            key={`${field.label}-${index}`}
            field={field}
            t={t}
            onOpenPath={onOpenPath}
            onOpenUrl={onOpenUrl}
          />
        ))}
        {item.detail && (
          <details>
            <summary className="cursor-pointer text-xs text-muted-foreground">
              {item.detailLabel || t('展开原文', 'Show details')}
            </summary>
            <pre className="max-h-56 overflow-auto whitespace-pre-wrap break-all text-xs">
              {item.detail}
            </pre>
          </details>
        )}
        {(item.path || item.url) && (
          <div className="flex flex-wrap gap-2">
            {item.path && (
              <Button variant="ghost" onClick={() => onOpenPath(item.path!)}>
                {t('打开文件', 'Open file')} · {item.path}
              </Button>
            )}
            {item.url && (
              <Button variant="ghost" onClick={() => onOpenUrl(item.url!)}>
                {item.urlLabel || t('打开链接', 'Open link')}
              </Button>
            )}
          </div>
        )}
      </div>
    </details>
  );
}

function EvidenceSectionView({
  section,
  t,
  onOpenPath,
  onOpenUrl,
}: {
  section: EvidenceSection;
  t: Translate;
  onOpenPath: (path: string) => void;
  onOpenUrl: (url: string) => void;
}) {
  return (
    <section className="space-y-1">
      <strong className="text-xs">{section.title}</strong>
      {section.fields.map((field, index) => (
        <EvidenceFieldView
          key={`${field.label}-${index}`}
          field={field}
          t={t}
          onOpenPath={onOpenPath}
          onOpenUrl={onOpenUrl}
        />
      ))}
      {section.items?.map((item, index) => (
        <EvidenceItemView
          key={`${item.title}-${index}`}
          item={item}
          t={t}
          onOpenPath={onOpenPath}
          onOpenUrl={onOpenUrl}
        />
      ))}
      {section.note && <p className="text-xs text-muted-foreground">{section.note}</p>}
    </section>
  );
}

/** 历史和文档检索结果的证据卡。折叠时只显示一行短答案，展开后按 section/item 展示结构化字段。 */
export function EvidenceCardView({
  evidence,
  t,
  onOpenPath,
  onOpenUrl,
}: {
  evidence: HistoryEvidence;
  t: Translate;
  onOpenPath: (path: string) => void;
  onOpenUrl: (url: string) => void;
}) {
  return (
    <div className="space-y-2">
      {evidence.sections.map((section, index) => (
        <EvidenceSectionView
          key={`${section.title}-${index}`}
          section={section}
          t={t}
          onOpenPath={onOpenPath}
          onOpenUrl={onOpenUrl}
        />
      ))}
    </div>
  );
}

/**
 * U04 未知副作用核对横幅：只在任务快照的 requires_reconciliation 为真时渲染。
 * 成功提交核对后不本地清横幅——等后端发来 requires_reconciliation=false 的新 task_state。
 */
export function TaskReviewBanner({
  review,
  note,
  onNoteChange,
  reconciling,
  notice,
  error,
  onReconcile,
  onOpenPath,
  language,
  t,
}: {
  review: PendingReconciliation;
  note: string;
  onNoteChange: (value: string) => void;
  reconciling: boolean;
  notice: string;
  error: string;
  onReconcile: (ids: string[]) => void;
  onOpenPath: (path: string) => void;
  language: string;
  t: Translate;
}) {
  const actions = review.actions;
  const knowledgeOnly =
    actions.length > 0 && actions.every((action) => knowledgeSaveReview(action, language));
  const title = knowledgeOnly
    ? t('知识卡保存结果待核实', 'Knowledge card save needs verification')
    : t('执行结果待核对', 'Action result needs review');
  const reviewControls = (
    <>
      <label className="flex flex-col gap-1 text-xs">
        <span>{t('核对备注（可选）', 'Review note (optional)')}</span>
        <Input
          aria-label={t('核对备注', 'Review note')}
          value={note}
          onChange={(event) => onNoteChange(event.target.value)}
        />
      </label>
      <div className="flex flex-wrap items-center gap-2">
        <Button disabled={reconciling} onClick={() => onReconcile(review.ids)}>
          {reconciling
            ? t('正在提交核对…', 'Submitting review…')
            : t('标记已核对', 'Mark reviewed')}
        </Button>
        {reconciling && (
          <span className="text-xs text-muted-foreground">
            {t('核对只针对上面列出的旧动作。', 'Review targets only the old actions listed above.')}
          </span>
        )}
      </div>
    </>
  );
  return (
    <section role="alert" aria-label={title} className="notice notice-warning space-y-2">
      <strong>{title}</strong>
      <p className="text-xs">
        {knowledgeOnly
          ? t(
              '程序会自动检查本地知识卡的保存记录。这里只核实是否保存，不代表科学内容已经审核；无法确认的结果会继续保留。',
              'The app automatically checks local knowledge card save records. This verifies saving only, not scientific content; unconfirmed results remain visible.',
            )
          : t(
              '有会修改状态的操作没有确定的结局；结果未知，不能用新回答自动清除。先查看实际运行/输出，再标记已核对。',
              'Some state-changing actions have no confirmed outcome. The result is unknown and cannot be cleared by a new answer; inspect the actual run/output, then mark it reviewed.',
            )}
      </p>
      {actions.length === 0 && (
        <p className="text-xs text-muted-foreground">
          {t(
            '任务快照要求核对，但没有列出待核对动作；仍可用备注说明已查看的状态。',
            'The task snapshot requires review but lists no actions; use the note to record what you checked.',
          )}
        </p>
      )}
      {actions.map((action, index) => {
        const key = action.id ?? `action-${index}`;
        const knowledge = knowledgeSaveReview(action, language);
        const paths = knowledge ? [] : localPathsInAction(action);
        const output = taskActionOutput(action);
        return (
          <div key={key} className="space-y-1 rounded border p-2">
            <strong className="text-xs">
              {knowledge?.target ?? taskActionTitle(action, language)}
            </strong>
            {knowledge ? (
              <p className="text-xs">{knowledge.explanation}</p>
            ) : (
              action.summary && <p className="text-xs">{action.summary}</p>
            )}
            <div className="flex flex-wrap gap-2">
              {paths.map((path, pathIndex) => (
                <Button key={path} variant="ghost" onClick={() => onOpenPath(path)}>
                  {pathIndex === 0 ? t('查看运行/输出', 'View run/output') : t('打开', 'Open')}{' '}
                  {path}
                </Button>
              ))}
            </div>
            {(output || knowledge) && (
              <details>
                <summary className="cursor-pointer text-xs text-muted-foreground">
                  {knowledge
                    ? t('技术详情', 'Technical details')
                    : t('参数 / 结果原文', 'arguments / result')}
                </summary>
                <pre className="max-h-40 overflow-auto whitespace-pre-wrap break-all text-xs">
                  {knowledge ? `${taskActionTitle(action, language)}\n\n${output}` : output}
                </pre>
              </details>
            )}
          </div>
        );
      })}
      {knowledgeOnly ? (
        <details>
          <summary className="cursor-pointer text-xs text-muted-foreground">
            {t(
              '自动核实仍无法确定时，手动记录核查结果',
              'If automatic verification is inconclusive, record a manual check',
            )}
          </summary>
          <p className="my-2 text-xs">
            {t(
              '仅在已检查知识卡保存记录后使用。此操作记录你的核查，不会重试保存，也不代表科学内容通过审核。',
              'Use only after checking the card save records. This records your check; it does not retry saving or approve the scientific content.',
            )}
          </p>
          {reviewControls}
        </details>
      ) : (
        reviewControls
      )}
      {notice && <p className="text-xs">{notice}</p>}
      {error && <Notice tone="error">{error}</Notice>}
    </section>
  );
}

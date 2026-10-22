import { bilingual } from '../../lib/bilingual';
import type { FrozenRevision, ModelDraft } from '../../lib/types';
type Operations = {
  save: (draft: ModelDraft) => Promise<ModelDraft>;
  freeze: (id: string, assetRoot: string | null) => Promise<FrozenRevision>;
};
export async function researchVersion(
  draft: ModelDraft | null,
  dirty: boolean,
  selected: FrozenRevision | null,
  assetRoot: string,
  operations: Operations,
): Promise<FrozenRevision> {
  if (!draft) {
    if (!selected) throw new Error(bilingual('请选择冻结版本', 'Select a frozen revision.'));
    return selected;
  }
  const saved = dirty ? await operations.save(draft) : draft;
  if (saved.id !== draft.id)
    throw new Error(bilingual('保存的草稿不匹配', 'Saved draft identity does not match.'));
  // 配置与已有版本等价时，后端直接返回那个版本（不另起修订号），id 自然和草稿不同。原来把它当成
  // "冻结版本不匹配"报错，而后端已经把草稿删了，重试只剩"找不到草稿"（GUI 审查 2026-10-22）。
  // 调用方按 id 是否等于草稿判断是新冻结的还是复用的。
  return await operations.freeze(saved.id, assetRoot || null);
}
export async function refreshDraftCode(
  draft: ModelDraft,
  hasUnsaved: () => boolean,
  refresh: (id: string) => Promise<ModelDraft>,
): Promise<ModelDraft> {
  const check = () => {
    if (hasUnsaved())
      throw new Error(
        bilingual(
          '草稿有未保存修改，未覆盖当前草稿',
          'The draft has unsaved edits; it was not overwritten.',
        ),
      );
  };
  check();
  const saved = await refresh(draft.id);
  check();
  if (saved.id !== draft.id)
    throw new Error(bilingual('源码刷新草稿不匹配', 'Refreshed draft identity does not match.'));
  return saved;
}

import { useEffect, useRef, useState } from 'react';
import { useApp, useNavigationGuard } from '../../lib/app-state';
import { invoke, invokeJson } from '../../lib/ipc';
import { usePageContract, type UiField, type UiAction } from '../../lib/guide';
import type { FrozenRevision, ModelDraft, HistoryOperation, VersionRef } from '../../lib/types';
import { Advanced, Button, Card, Field, Input, Notice, Select } from '../../components/ui';
import { download } from '../history/history';
import {
  importParameters,
  makeRecord,
  parameterKey,
  upsertParameter,
  type Descriptor,
  type ModelField,
} from './model';
import CodeWorkspace from './CodeWorkspace';
import VersionChanges from './VersionChanges';
import RevisionTree from './RevisionTree';
import ConfigurationOptions from './ConfigurationOptions';
import { setupHandledKeys, type SetupEnvironment } from './scientific-setup';
import { compatibleKernel } from '../runs/run-model';
import { effectiveModelFields } from '../cases/case-model';
import { IGBP, USGS, PFT } from './scope-labels';
import { researchVersion, refreshDraftCode } from './research-version';
import { appLocale } from '../history/history';
export default function VersionsPage() {
  const { state, update, navigate, refreshVersions } = useApp(),
    t = (a: string, b: string) => (state.language === 'en' ? b : a);
  const [draft, setDraft] = useState<ModelDraft | null>(null),
    [drafts, setDrafts] = useState<ModelDraft[]>([]),
    [catalog, setCatalog] = useState<Descriptor[]>([]),
    [fields, setFields] = useState<ModelField[]>([]),
    [name, setName] = useState(''),
    [note, setNote] = useState(''),
    [workspace, setWorkspace] = useState(''),
    // 新版本的代码从哪来：沿用父版本、复制父版本的代码来改，或用一个已有的代码工作区。
    [codeMode, setCodeMode] = useState<'inherit' | 'branch' | 'existing'>('inherit'),
    // 冻结前的检查清单（只有用自己代码的草稿才有）；checkTick 用来手动重查。
    [freezeCheck, setFreezeCheck] = useState<{
      ready: boolean;
      items: { code: string; ok: boolean; blocking: boolean; message: string }[];
    } | null>(null),
    [checkTick, setCheckTick] = useState(0),
    [studyNotice, setStudyNotice] = useState(''),
    [parameter, setParameter] = useState(''),
    [value, setValue] = useState(''),
    [index, setIndex] = useState(1),
    [process, setProcess] = useState(''),
    [query, setQuery] = useState(''),
    [physics, setPhysics] = useState(''),
    [physicsValue, setPhysicsValue] = useState(''),
    [error, setError] = useState(''),
    [busy, setBusy] = useState(false),
    [dirty, setDirty] = useState(false),
    [advanced, setAdvanced] = useState(state.expert),
    [references, setReferences] = useState<HistoryOperation[]>([]),
    [assetRoot, setAssetRoot] = useState(''),
    [processPath, setProcessPath] = useState(''),
    [tab, setTab] = useState<'overview' | 'parameters' | 'configuration' | 'code' | 'usage'>(
      'overview',
    ),
    // 新建修改版的表单：页头按钮打开，建好草稿后收起。
    [creating, setCreating] = useState(false);
  const epoch = useRef({ active: false }),
    lock = useRef(false);
  const selected = state.version,
    shown = draft ?? selected,
    editable = !!draft && draft.id !== 'official';
  useEffect(() => {
    if (state.simulationReturn) document.querySelector('.main-pane')?.scrollTo({ top: 0 });
  }, [state.simulationReturn]);
  const [codeBusy, setCodeBusy] = useState(false),
    codeBusyNow = useRef(false);
  const dirtyNow = useRef(dirty);
  dirtyNow.current = dirty;
  function codePending(pending: boolean) {
    codeBusyNow.current = pending;
    setCodeBusy(pending);
  }
  useNavigationGuard(() =>
    dirtyNow.current
      ? t(
          '请先保存草稿或放弃本地修改后再离开',
          'Save the draft or discard local edits before leaving',
        )
      : lock.current || codeBusyNow.current
        ? t('版本操作进行中，请等待结果', 'A version operation is in progress; wait for its result')
        : null,
  );
  useEffect(() => {
    const token = { active: true };
    epoch.current = token;
    setDraft(null);
    setAssetRoot('');
    setDirty(false);
    setBusy(false);
    setError('');
    Promise.all([
      invokeJson<Descriptor[]>('parameter_catalog'),
      invokeJson<ModelField[]>('model_fields'),
      invokeJson<ModelDraft[]>('model_drafts'),
    ])
      .then(([c, f, d]) => {
        if (token.active && token === epoch.current) {
          setCatalog(c);
          setFields(f);
          setDrafts(d);
        }
      })
      .catch((e) => {
        if (token.active && token === epoch.current) setError(String(e));
      });
    return () => {
      token.active = false;
    };
  }, [state.versionRef.id, state.versionRef.revision]);
  useEffect(() => {
    let live = true;
    setReferences([]);
    invokeJson<{ items: HistoryOperation[] }>('history_list', {
      filter: { versionId: draft?.id ?? state.versionRef.id, limit: 100 },
    })
      .then((h) => {
        if (live) setReferences(h.items);
      })
      .catch((e) => {
        if (live) setError(String(e));
      });
    return () => {
      live = false;
    };
  }, [draft?.id, state.versionRef.id]);
  // 用自己代码的草稿：冻结前要编过引擎、改了 Fortran 要编过内核、改了生成表来源要过 drift、继承的参数
  // 在新代码里还在。代码一变（提交号变了）或草稿保存后重查。
  useEffect(() => {
    let live = true;
    setFreezeCheck(null);
    if (!draft?.code.workspace) return;
    invokeJson<NonNullable<typeof freezeCheck>>('model_freeze_check', { id: draft.id })
      .then((check) => {
        if (live) setFreezeCheck(check);
      })
      .catch((e) => {
        if (live) setError(String(e));
      });
    return () => {
      live = false;
    };
  }, [draft?.id, draft?.code.workspace, draft?.code.commit, draft?.generation, checkTick]);
  async function execute<T>(fn: () => Promise<T>) {
    if (codeBusyNow.current)
      throw new Error(t('代码工作区操作进行中', 'A code workspace operation is in progress'));
    if (lock.current) return;
    lock.current = true;
    setBusy(true);
    setError('');
    const token = epoch.current;
    try {
      return await fn();
    } catch (e) {
      if (token.active && token === epoch.current) setError(String(e));
      throw e;
    } finally {
      lock.current = false;
      if (token.active && token === epoch.current) setBusy(false);
    }
  }
  const run = (fn: () => Promise<unknown>) => void execute(fn).catch(() => {});
  function edit(next: ModelDraft) {
    dirtyNow.current = true;
    setDraft(next);
    setDirty(true);
  }
  async function save(target = draft) {
    if (!target) throw new Error(t('没有当前草稿', 'No current draft'));
    const saved = await invokeJson<ModelDraft>('model_save_draft', {
      draft: target,
      expectedGeneration: target.generation,
    });
    setDraft(saved);
    dirtyNow.current = false;
    setDirty(false);
    setDrafts(await invokeJson('model_drafts'));
    return saved;
  }
  async function freeze(target = draft, unsaved = dirtyNow.current) {
    const frozen = await researchVersion(target, unsaved, selected, assetRoot, {
      save,
      freeze: (id, root) => invokeJson<FrozenRevision>('model_freeze', { id, assetRoot: root }),
    });
    update({ version: frozen, versionRef: frozen.version, chosenVersion: frozen.version });
    setDraft(null);
    dirtyNow.current = false;
    setDirty(false);
    await refreshVersions();
    return frozen;
  }
  function returnToSimulation() {
    const target = state.simulationReturn;
    if (!target) return;
    return navigate(target.route, target.route === 'case' ? 'settings' : undefined);
  }
  async function openResearch() {
    if (draft) await freeze();
    else if (!selected) throw new Error(t('请选择冻结版本', 'Select a frozen revision'));
    lock.current = false;
    if (!navigate('research')) throw new Error(t('当前修改尚未保存', 'Current edits are unsaved'));
  }
  async function discardLocal() {
    if (!draft) return;
    const saved = await invokeJson<ModelDraft>('model_draft', { id: draft.id });
    setDraft(saved);
    dirtyNow.current = false;
    setDirty(false);
  }
  async function select(v: FrozenRevision) {
    if (dirty)
      throw new Error(
        t('请先保存草稿，或放弃本地修改', 'Save the draft or discard local changes first'),
      );
    update({ version: v, versionRef: v.version, chosenVersion: v.version });
    setDraft(null);
  }
  async function loadDraft(id: string) {
    if (dirty) throw new Error(t('请先保存草稿', 'Save the current draft first'));
    const token = epoch.current;
    const d = await invokeJson<ModelDraft>('model_draft', { id });
    if (token.active && token === epoch.current) {
      setDraft(d);
      setAssetRoot('');
      setDirty(false);
    }
  }
  async function importConfigurationProcess(relativePath: string) {
    await execute(async () => {
      if (!draft) throw new Error('No current draft');
      const file = await invoke<string | null>('pick_file', {
        key: 'model-process',
        filter: 'nml',
      });
      if (!file) return;
      const current = dirtyNow.current ? await save() : draft;
      const result = await invokeJson<{ draft: ModelDraft; assetRoot: string }>(
        'model_import_process',
        { id: current.id, file, relativePath, expectedGeneration: current.generation },
      );
      setDraft(result.draft);
      setAssetRoot(result.assetRoot);
      dirtyNow.current = false;
      setDirty(false);
      setDrafts(await invokeJson('model_drafts'));
    });
  }
  // 研究的结果写进这个版本：选一个已完成的率定或敏感性研究，把它最优成员的参数值写成参数改值。
  async function importStudyParameters() {
    await execute(async () => {
      if (!draft) throw new Error('No current draft');
      const studyDir = await invoke<string | null>('pick_folder', { key: 'model-study' });
      if (!studyDir) return;
      const current = dirtyNow.current ? await save() : draft;
      const result = await invokeJson<{ draft: ModelDraft; member: string; parameters: number }>(
        'model_import_study',
        { id: current.id, studyDir, member: null, expectedGeneration: current.generation },
      );
      setDraft(result.draft);
      dirtyNow.current = false;
      setDirty(false);
      setDrafts(await invokeJson('model_drafts'));
      setStudyNotice(
        t(
          `已把研究最优成员 ${result.member} 的 ${result.parameters} 个参数写进这个版本，并记下了研究的站点与时段。`,
          `Wrote ${result.parameters} parameters of the study's best member ${result.member} into this version and recorded the study's sites and periods.`,
        ),
      );
    });
  }
  const setup = state.simulationReturn?.route === 'new-case' ? state.newCaseSeed?.form : null;
  const configurationCase = setup ? null : (state.selectedCase?.entry.dir ?? null);
  const configurationProfile = setup
    ? {
        grid: setup.kind === 'site' ? null : setup.grid,
        classification: setup.landcover as 'IGBP' | 'USGS',
      }
    : (state.selectedCase?.profile ?? {});
  const configurationKernel = compatibleKernel(
    state.kernels,
    configurationProfile,
    effectiveModelFields(shown?.overlay, fields),
  );
  const setupEnvironment: SetupEnvironment = {
    engine: setup?.engine ?? state.selectedCase?.profile.engine,
    subgrid: setup?.subgrid ?? setup?.landcover ?? state.selectedCase?.profile.subgrid,
    caseValues: setup ? state.newCaseSeed?.caseValues : undefined,
    domain:
      setup?.kind ??
      (state.selectedCase?.profile.spatial === false
        ? 'site'
        : state.selectedCase?.profile.spatial
          ? 'region'
          : null),
    grid: configurationProfile.grid,
    classification: configurationProfile.classification,
    kernel: configurationKernel,
    cropKernel: compatibleKernel(
      state.kernels,
      { ...configurationProfile, crop: true },
      effectiveModelFields(shown?.overlay, fields),
    ),
  };
  const allowed = catalog.filter(
    (d) =>
      d.raw_key.toUpperCase() !== 'DEF_SUBGRID_SCHEME' &&
      d.storage !== 'read-only' &&
      !['excluded-internal', 'blocked-pending-hook', 'read-only-context'].includes(d.visibility) &&
      (d.storage !== 'case-nml' ||
        fields.some((f) => f.name.toLowerCase() === d.raw_key.toLowerCase())),
  );
  const descriptor = allowed.find((d) => d.id === parameter),
    modelField = fields.find((f) => f.name === physics);
  function selectParameter(id: string) {
    setParameter(id);
    const d = allowed.find((x) => x.id === id);
    setValue(d?.default ?? '');
    setIndex(d?.scope === 'land-cover-class' ? 1 : 0);
    setProcess('');
  }
  function selectPhysics(name: string) {
    setPhysics(name);
    setPhysicsValue(fields.find((f) => f.name === name)?.default ?? '');
  }
  function addParameter() {
    if (!draft || !descriptor) return;
    const record = makeRecord(descriptor, value, index, process);
    if (
      record.scope_instance.process_file &&
      !draft.overlay.process_files.some(
        (a) => a.relative_path === record.scope_instance.process_file,
      )
    )
      throw new Error('Process asset missing');
    if (
      Object.keys(draft.overlay.physics).some(
        (k) => k.toLowerCase() === record.raw_key.toLowerCase(),
      )
    )
      throw new Error(
        t('该字段已在物理设置中覆盖，请先重置', 'Reset the physics override for this field first'),
      );
    edit({
      ...draft,
      overlay: { ...draft.overlay, parameters: upsertParameter(draft.overlay.parameters, record) },
    });
  }
  async function importFile() {
    if (!draft) return;
    const path = await invoke<string | null>('pick_file', {
      key: 'model-parameters',
      filter: 'json',
    });
    if (!path) return;
    const text = await invoke<string>('read_text', { path });
    const rows = importParameters(
      text,
      catalog,
      fields,
      draft.overlay.process_files.map((a) => a.relative_path),
    );
    edit({
      ...draft,
      overlay: {
        ...draft.overlay,
        parameters: rows.reduce(upsertParameter, draft.overlay.parameters),
      },
    });
  }
  async function refreshCode() {
    if (!draft) return;
    const token = epoch.current;
    const saved = await refreshDraftCode(
      draft,
      () => dirtyNow.current,
      (id) => invokeJson<ModelDraft>('model_refresh_code', { id }),
    );
    if (token.active && token === epoch.current) {
      setDraft(saved);
      setDirty(false);
    }
  }
  async function deriveVersion() {
    if (!selected) return null;
    const d = await invokeJson<ModelDraft>('model_create', {
      name,
      note,
      parent: selected.version,
      workspace: codeMode === 'existing' && workspace ? workspace : null,
      branchCode: codeMode === 'branch',
    });
    setAssetRoot('');
    setDraft(d);
    setDirty(false);
    setCreating(false);
    setTab('overview');
    setDrafts(await invokeJson('model_drafts'));
    return d;
  }
  const guideFields: UiField[] = [
    {
      id: 'version-name',
      label: t('新版本名称', 'New version name'),
      value: name,
      type: 'text',
      set: setName,
    },
    {
      id: 'version-note',
      label: t('目的与说明', 'Purpose and notes'),
      value: note,
      type: 'text',
      set: setNote,
    },
    {
      id: 'version-code',
      label: t('新版本的代码', 'Code of the new version'),
      value: codeMode,
      type: 'choice',
      options: [
        { value: 'inherit', label: t('沿用父版本的代码', 'Keep the parent code') },
        {
          value: 'branch',
          label: t('复制父版本的代码来修改', 'Copy the parent code to change it'),
        },
        { value: 'existing', label: t('使用已有的代码工作区', 'Use an existing code workspace') },
      ],
      set: (value) => setCodeMode(value as typeof codeMode),
    },
    ...(codeMode === 'existing'
      ? [
          {
            id: 'version-workspace',
            label: t('代码工作区', 'Code workspace'),
            value: workspace,
            type: 'text' as const,
            set: setWorkspace,
          },
        ]
      : []),
  ];
  const guideActions: UiAction[] = [
    {
      id: 'version-create',
      label: t('派生新版本', 'Derive version'),
      kind: 'commit',
      enabled: !busy && !codeBusy && !dirty && !!name && !!note && !!selected,
      run: () => execute(deriveVersion),
    },
    ...state.versions.map((v) => ({
      id: `version-select-${v.version.id}-${v.version.revision}`,
      label: `${v.name} r${v.version.revision}`,
      kind: 'click' as const,
      enabled: !busy && !codeBusy && !dirty,
      run: () => execute(() => select(v)),
    })),
    ...drafts.map((d) => ({
      id: `version-draft-${d.id}`,
      label: d.name,
      kind: 'click' as const,
      enabled: !busy && !codeBusy && !dirty,
      run: () => execute(() => loadDraft(d.id)),
    })),
  ];
  if (state.simulationReturn)
    guideActions.push({
      id: 'version-return-simulation',
      label: t('返回模拟配置', 'Return to simulation setup'),
      kind: 'click',
      enabled: !busy && !codeBusy && !dirty,
      run: returnToSimulation,
    });
  if (shown) {
    guideActions.push(
      {
        id: 'version-advanced-toggle',
        label: t('适用范围与率定/验证时段', 'Applicability and calibration / validation periods'),
        kind: 'click',
        run: () => setAdvanced(!advanced),
      },
      {
        id: 'version-export-parameters',
        enabled: !busy && !codeBusy,
        label: t('导出参数 JSON', 'Export parameters JSON'),
        kind: 'click',
        run: () =>
          execute(() =>
            download(
              JSON.stringify({ records: shown.overlay.parameters }, null, 2),
              'model-parameters.json',
              'application/json',
            ),
          ),
      },
      {
        id: 'version-research',
        label: draft
          ? t('保存、冻结并进入研究', 'Save, freeze and open research')
          : t('打开研究与网络', 'Open research and networks'),
        kind: draft ? 'commit' : 'click',
        enabled: !busy && !codeBusy,
        run: () => execute(openResearch),
      },
      {
        id: 'version-history',
        label: t('完整历史', 'Full history'),
        kind: 'click',
        run: () => navigate('history'),
      },
    );
  }
  if (editable && draft) {
    guideFields.push(
      {
        id: 'draft-name',
        label: t('草稿名称', 'Draft name'),
        value: draft.name,
        type: 'text',
        set: (v) => edit({ ...draft, name: v }),
      },
      {
        id: 'draft-note',
        label: t('草稿说明', 'Draft notes'),
        value: draft.note,
        type: 'text',
        set: (v) => edit({ ...draft, note: v }),
      },
      {
        id: 'version-parameter-query',
        label: t('搜索参数', 'Search parameters'),
        value: query,
        type: 'text',
        set: setQuery,
      },
      {
        id: 'version-parameter',
        label: t('参数目录', 'Parameter catalog'),
        value: parameter,
        type: 'choice',
        options: [
          { value: '', label: '—' },
          ...allowed
            .filter(
              (d) =>
                !query ||
                `${d.id} ${d.label_zh} ${d.label_en}`.toLowerCase().includes(query.toLowerCase()),
            )
            .map((d) => ({ value: d.id, label: d.raw_key })),
        ],
        set: selectParameter,
      },
      {
        id: 'version-parameter-value',
        label: t('参数值', 'Parameter value'),
        value,
        type: 'text',
        set: setValue,
      },
      {
        id: 'version-process-path',
        label: t('导入过程文件的相对路径', 'Process file relative path'),
        value: processPath,
        type: 'text',
        set: setProcessPath,
      },
      {
        id: 'version-physics',
        label: t('模型字段', 'Model field'),
        value: physics,
        type: 'choice',
        options: [
          { value: '', label: '—' },
          ...fields
            .filter(
              (f) =>
                !f.derived &&
                f.name !== 'DEF_SUBGRID_SCHEME' &&
                !setupHandledKeys.includes(f.name) &&
                !allowed.some(
                  (d) =>
                    d.raw_key.toLowerCase() === f.name.toLowerCase() &&
                    ['land-cover-class', 'pft-type', 'pc-pft-component'].includes(d.scope),
                ),
            )
            .map((f) => ({ value: f.name, label: f.name })),
        ],
        set: selectPhysics,
      },
      {
        id: 'version-physics-value',
        label: t('物理值', 'Physics value'),
        value: physicsValue,
        type: 'text',
        set: setPhysicsValue,
      },
    );
    if (
      descriptor &&
      ['land-cover-class', 'pft-type', 'pc-pft-component'].includes(descriptor.scope)
    )
      guideFields.push({
        id: 'version-scope-index',
        label: t('地类 / PFT', 'Land class / PFT'),
        value: String(index),
        type: 'choice',
        options: (descriptor.scope === 'land-cover-class'
          ? descriptor.id.includes(':USGS:')
            ? USGS
            : IGBP
          : PFT
        ).map(([id, zh, en]) => ({ value: String(id), label: state.language === 'en' ? en : zh })),
        set: (v) => setIndex(Number(v)),
      });
    if (descriptor?.storage === 'process-parameter-file')
      guideFields.push({
        id: 'version-process',
        label: t('冻结过程文件', 'Frozen process file'),
        value: process,
        type: 'choice',
        options: [
          { value: '', label: '—' },
          ...draft.overlay.process_files.map((a) => ({
            value: a.relative_path,
            label: a.relative_path,
          })),
        ],
        set: setProcess,
      });
    if (advanced) {
      for (const key of ['sites', 'land_classes'] as const)
        guideFields.push({
          id: `version-${key}`,
          label: key,
          value: draft.applicability[key].join(','),
          type: 'text',
          set: (v) =>
            edit({
              ...draft,
              applicability: {
                ...draft.applicability,
                [key]: v
                  .split(',')
                  .map((s) => s.trim())
                  .filter(Boolean),
              },
            }),
        });
      for (const key of ['calibration', 'validation'] as const)
        for (const part of ['start', 'end'] as const)
          guideFields.push({
            id: `version-${key}-${part}`,
            label: `${key} ${part}`,
            value: draft.applicability[key]?.[part] ?? '',
            type: 'text',
            set: (v) =>
              edit({
                ...draft,
                applicability: {
                  ...draft.applicability,
                  [key]: {
                    start: draft.applicability[key]?.start ?? '',
                    end: draft.applicability[key]?.end ?? '',
                    [part]: v,
                  },
                },
              }),
          });
      guideFields.push({
        id: 'version-asset-root',
        label: t('资产根目录', 'Asset root'),
        value: assetRoot,
        type: 'text',
        set: setAssetRoot,
      });
    }
    guideActions.push(
      {
        id: 'version-discard',
        label: t('放弃本地修改', 'Discard local edits'),
        kind: 'click',
        enabled: !busy && !codeBusy && dirty,
        run: () => execute(discardLocal),
      },
      {
        id: 'version-save',
        label: t('保存草稿', 'Save draft'),
        kind: 'commit',
        enabled: !busy && !codeBusy && dirty,
        run: () => execute(save),
      },
      {
        id: 'version-freeze',
        label: t('冻结修订', 'Freeze revision'),
        kind: 'commit',
        enabled: !busy && !codeBusy && freezeCheck?.ready !== false,
        run: () => execute(freeze),
      },
      {
        id: 'version-parameter-add',
        label: t('应用参数到草稿', 'Apply parameter to draft'),
        kind: 'click',
        enabled: !busy && !codeBusy && !!descriptor && !!value,
        run: addParameter,
      },
      {
        id: 'version-import-study',
        label: t('从研究导入最优参数', 'Import the best parameters from a study'),
        kind: 'commit',
        enabled: !busy && !codeBusy,
        run: () => importStudyParameters(),
      },
      {
        id: 'version-import-parameters',
        label: t('导入参数 JSON', 'Import parameters JSON'),
        kind: 'click',
        enabled: !busy && !codeBusy,
        run: () => execute(importFile),
      },
      {
        id: 'version-physics-apply',
        label: t('应用物理设置', 'Apply physics setting'),
        kind: 'click',
        enabled: !busy && !codeBusy && !!modelField && !!physicsValue,
        run: () => {
          if (
            draft.overlay.parameters.some((r) => r.raw_key.toLowerCase() === physics.toLowerCase())
          )
            throw new Error('Reset the parameter override first');
          edit({
            ...draft,
            overlay: {
              ...draft.overlay,
              physics: { ...draft.overlay.physics, [physics]: physicsValue },
            },
          });
        },
      },
    );
  }
  usePageContract({
    title: t('模型版本', 'Model versions'),
    step: 'versions',
    status: error || (dirty ? t('未保存', 'Unsaved') : ''),
    fields: guideFields.map((f) => ({ ...f, disabled: !!f.disabled || busy || codeBusy })),
    actions: guideActions,
  });

  const parentOf = (ref: VersionRef | null | undefined) => {
    if (!ref) return null;
    const found = state.versions.find(
      (v) => v.version.id === ref.id && v.version.revision === ref.revision,
    );
    return found ? `${found.name} · r${found.version.revision}` : `${ref.id} r${ref.revision}`;
  };
  const scopeText = (r: ModelDraft['overlay']['parameters'][number]) => {
    const s = r.scope_instance;
    const label = (table: [number, string, string][], id: number | null) => {
      const hit = table.find(([n]) => n === id);
      return hit ? `${id} ${state.language === 'en' ? hit[2] : hit[1]}` : String(id ?? '');
    };
    switch (s.kind) {
      case 'case-scalar':
        return t('全局', 'Global');
      case 'land-cover-class':
        return `${s.scheme ?? ''} ${label(s.scheme === 'USGS' ? USGS : IGBP, s.index ?? null)}`;
      case 'pft-type':
      case 'pc-pft-component':
        return `PFT ${label(PFT, s.index ?? null)}`;
      default:
        return [s.kind, s.process_file, s.type_name].filter(Boolean).join(' · ');
    }
  };
  const isOfficial = !draft && selected?.version.id === 'official';
  const kindBadge = draft
    ? t('草稿', 'Draft')
    : isOfficial
      ? t('官方', 'Official')
      : t('修改版', 'Modified');
  const counts = shown
    ? {
        parameters: shown.overlay.parameters.length,
        physics: Object.keys(shown.overlay.physics).length,
        files: shown.overlay.process_files.length,
        network: !!shown.overlay.hybrid,
      }
    : null;
  const tabs: [typeof tab, string][] = [
    ['overview', t('概览', 'Overview')],
    [
      'parameters',
      `${t('参数与物理方案', 'Parameters')}${counts && counts.parameters + counts.physics > 0 ? ` (${counts.parameters + counts.physics})` : ''}`,
    ],
    ['configuration', t('过程配置', 'Processes')],
    ['code', t('代码', 'Code')],
    ['usage', `${t('使用记录', 'Usage')}${references.length > 0 ? ` (${references.length})` : ''}`],
  ];
  const changeTable = (
    rows: { key: string; scope?: string; value: string; onRemove?: () => void }[],
    empty: string,
    removeLabel: string,
  ) =>
    rows.length === 0 ? (
      <p className="versions-empty">{empty}</p>
    ) : (
      <div className="table-scroll">
        <table className="versions-table">
          <tbody>
            {rows.map((row) => (
              <tr key={`${row.key}|${row.scope ?? ''}`}>
                <td>
                  <code>{row.key}</code>
                </td>
                {row.scope !== undefined && <td className="muted">{row.scope}</td>}
                <td>
                  <code>{row.value}</code>
                </td>
                {editable && (
                  <td className="versions-table-action">
                    {row.onRemove && (
                      <Button variant="ghost" disabled={busy || codeBusy} onClick={row.onRemove}>
                        {removeLabel}
                      </Button>
                    )}
                  </td>
                )}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    );

  return (
    <div className="page-stack versions-page">
      {state.simulationReturn && (
        <div className="button-row">
          <Button disabled={busy || codeBusy || dirty} onClick={returnToSimulation}>
            ← {t('返回模拟配置', 'Back to simulation setup')}
          </Button>
          {dirty ? (
            <span className="muted">
              {t(
                '有未保存的修改：先保存草稿或放弃修改。',
                'Unsaved edits: save the draft or discard them first.',
              )}
            </span>
          ) : draft ? (
            <span className="muted">
              {t(
                '草稿已保存在版本库里，以后可以接着改；返回后模拟仍用原来的版本。',
                'The draft stays saved in the version library; the simulation keeps its current version.',
              )}
            </span>
          ) : null}
        </div>
      )}
      <div className="page-heading versions-heading">
        <div>
          <h1>{t('模型版本', 'Model versions')}</h1>
          <p className="muted">
            {t(
              '版本 = 官方代码 + 你的改动（参数、物理开关、过程文件、AI 网络，或代码本身）。模拟用哪个版本，在左边选。',
              'A version is the official code plus your changes (parameters, physics switches, process files, an AI network, or the code itself). Choose on the left which one simulations use.',
            )}
          </p>
        </div>
        <Button
          variant="primary"
          disabled={busy || codeBusy || dirty || !selected}
          onClick={() => setCreating(!creating)}
        >
          {creating ? t('收起', 'Close') : `＋ ${t('新建修改版', 'New modified version')}`}
        </Button>
      </div>
      {error && <Notice tone="error">{error}</Notice>}
      <div className="versions-layout">
        <RevisionTree
          versions={state.versions}
          drafts={drafts}
          selected={selected}
          openDraft={draft?.id ?? null}
          dirty={dirty}
          disabled={busy || codeBusy || dirty}
          en={state.language === 'en'}
          onSelect={(v) => run(() => select(v))}
          onOpenDraft={(id) => run(() => loadDraft(id))}
          onDeleted={refreshVersions}
        />
        <div className="versions-main">
          {creating && (
            <Card
              className="versions-create"
              title={t('新建修改版', 'New modified version')}
              description={t(
                `基于 ${parentOf(selected?.version) ?? '—'}。先建成草稿，改完再冻结成新版本。`,
                `Based on ${parentOf(selected?.version) ?? '—'}. It starts as a draft; freeze it once your changes are done.`,
              )}
            >
              <div className="form-grid">
                <Field label={t('名称', 'Name')}>
                  <Input
                    value={name}
                    placeholder={t('如：根系吸水改进', 'e.g. Root water uptake')}
                    disabled={busy || codeBusy}
                    onChange={(e) => setName(e.target.value)}
                  />
                </Field>
                <Field label={t('目的与说明', 'Purpose and notes')}>
                  <Input
                    value={note}
                    placeholder={t('想验证什么、改了哪里', 'What it tests, what it changes')}
                    disabled={busy || codeBusy}
                    onChange={(e) => setNote(e.target.value)}
                  />
                </Field>
              </div>
              <p className="versions-label">{t('新版本的代码', 'Code of the new version')}</p>
              <div className="versions-choices" role="radiogroup">
                {(
                  [
                    [
                      'inherit',
                      t('沿用父版本的代码', 'Keep the parent code'),
                      t(
                        '只改参数、物理开关、过程文件或 AI 网络；代码和父版本完全一样。',
                        'Parameters, physics switches, process files or an AI network only; the code stays the parent’s.',
                      ),
                    ],
                    [
                      'branch',
                      t('复制父版本的代码来修改', 'Copy the parent code to change it'),
                      t(
                        '复制一份只属于这个版本的代码，可以改任意 Rust 或 Fortran 文件、替换或新增文件。',
                        'A copy of the code for this version alone: change any Rust or Fortran file, replace or add files.',
                      ),
                    ],
                    [
                      'existing',
                      t('使用已有的代码工作区', 'Use an existing code workspace'),
                      t(
                        '用一个已经建好的代码工作区的当前代码。',
                        'The current code of a workspace you already have.',
                      ),
                    ],
                  ] as const
                ).map(([mode, title, detail]) => (
                  <button
                    key={mode}
                    type="button"
                    role="radio"
                    aria-checked={codeMode === mode}
                    className={`versions-choice${codeMode === mode ? ' is-active' : ''}`}
                    disabled={busy || codeBusy}
                    onClick={() => setCodeMode(mode)}
                  >
                    <strong>{title}</strong>
                    <span>{detail}</span>
                  </button>
                ))}
              </div>
              {codeMode === 'existing' && (
                <Field label={t('代码工作区', 'Code workspace')}>
                  <Input
                    value={workspace}
                    disabled={busy || codeBusy}
                    onChange={(e) => setWorkspace(e.target.value)}
                  />
                </Field>
              )}
              <div className="button-row">
                <Button
                  disabled={!guideActions[0].enabled}
                  variant="primary"
                  onClick={() => run(deriveVersion)}
                >
                  {t('创建草稿', 'Create draft')}
                </Button>
                <Button variant="ghost" onClick={() => setCreating(false)}>
                  {t('取消', 'Cancel')}
                </Button>
                {(!name || !note) && (
                  <span className="muted">
                    {t('名称和目的都要填。', 'Both a name and a purpose are needed.')}
                  </span>
                )}
              </div>
            </Card>
          )}
          {!shown ? (
            <Card>
              <p className="versions-empty">
                {t('在左边选一个版本。', 'Choose a version on the left.')}
              </p>
            </Card>
          ) : (
            <>
              <Card className="versions-head">
                <div className="versions-head-top">
                  <div className="versions-head-title">
                    <h2>{shown.name}</h2>
                    <span
                      className={`badge versions-badge is-${draft ? 'draft' : isOfficial ? 'official' : 'modified'}`}
                    >
                      {kindBadge}
                    </span>
                    {!draft && selected && (
                      <span className="badge versions-badge is-used">
                        {t('模拟正在使用', 'Used for simulations')}
                      </span>
                    )}
                    {draft && dirty && (
                      <span className="badge versions-badge is-unsaved">
                        {t('未保存', 'Unsaved')}
                      </span>
                    )}
                  </div>
                  <div className="button-row versions-head-actions">
                    {editable ? (
                      <>
                        <Button disabled={busy || codeBusy || !dirty} onClick={() => run(save)}>
                          {t('保存草稿', 'Save draft')}
                        </Button>
                        <Button
                          variant="ghost"
                          disabled={busy || codeBusy || !dirty}
                          onClick={() => {
                            if (window.confirm(t('放弃尚未保存的修改？', 'Discard unsaved edits?')))
                              run(discardLocal);
                          }}
                        >
                          {t('放弃修改', 'Discard edits')}
                        </Button>
                        <Button
                          disabled={busy || codeBusy || freezeCheck?.ready === false}
                          variant="primary"
                          onClick={() => run(freeze)}
                        >
                          {t('冻结为新版本', 'Freeze as a version')}
                        </Button>
                      </>
                    ) : (
                      <>
                        <Button
                          disabled={busy || codeBusy || dirty}
                          onClick={() => setCreating(true)}
                        >
                          {t('基于它新建修改版', 'Derive from it')}
                        </Button>
                        <Button disabled={busy || codeBusy} onClick={() => run(openResearch)}>
                          {t('用它做研究', 'Use it for research')}
                        </Button>
                      </>
                    )}
                  </div>
                </div>
                <dl className="versions-facts">
                  <div>
                    <dt>{draft ? t('状态', 'State') : t('修订', 'Revision')}</dt>
                    <dd>
                      {draft
                        ? dirty
                          ? t('有未保存的修改', 'Unsaved edits')
                          : t(
                              `已保存 ${draft.generation} 次`,
                              draft.generation === 1
                                ? 'Saved once'
                                : `Saved ${draft.generation} times`,
                            )
                        : `r${selected?.version.revision}`}
                    </dd>
                  </div>
                  <div>
                    <dt>{t('基于', 'Based on')}</dt>
                    <dd>{parentOf(shown.parent) ?? t('—（官方基线）', '— (official baseline)')}</dd>
                  </div>
                  <div>
                    <dt>{t('代码', 'Code')}</dt>
                    <dd>
                      {shown.code.workspace
                        ? t(
                            `自己的代码 · ${shown.code.commit?.slice(0, 8) ?? ''}`,
                            `Own code · ${shown.code.commit?.slice(0, 8) ?? ''}`,
                          )
                        : isOfficial
                          ? t('官方源码', 'Official source')
                          : t('与父版本相同', 'Same as the parent')}
                    </dd>
                  </div>
                  {!draft && shown.created_unix > 0 && (
                    <div>
                      <dt>{t('冻结于', 'Frozen')}</dt>
                      <dd>
                        {new Date(shown.created_unix * 1000).toLocaleDateString(
                          appLocale(state.language === 'en'),
                        )}
                      </dd>
                    </div>
                  )}
                </dl>
                <p className="versions-note">
                  {isOfficial
                    ? t(
                        '随软件提供的官方基线，只读。要改它，基于它新建修改版。',
                        'The official baseline shipped with the app, read-only. Derive a modified version to change it.',
                      )
                    : shown.note || <span className="muted">{t('没有写说明。', 'No notes.')}</span>}
                </p>
                {editable && freezeCheck && (
                  <div
                    className="freeze-check"
                    aria-label={t('冻结前检查', 'Checks before freezing')}
                  >
                    <div className="freeze-check-head">
                      <strong>
                        {freezeCheck.ready
                          ? t('冻结前检查：可以冻结', 'Checks before freezing: ready')
                          : t('冻结前检查：还不能冻结', 'Checks before freezing: not yet')}
                      </strong>
                      <Button
                        variant="ghost"
                        disabled={busy || codeBusy}
                        onClick={() => setCheckTick((value) => value + 1)}
                      >
                        {t('重新检查', 'Check again')}
                      </Button>
                    </div>
                    <ul>
                      {freezeCheck.items.map((item) => (
                        <li
                          key={item.code}
                          className={item.blocking ? 'is-blocking' : item.ok ? 'is-ok' : 'is-note'}
                        >
                          <span aria-hidden="true">
                            {item.blocking ? '✕' : item.ok ? '✓' : 'i'}
                          </span>
                          {item.message}
                        </li>
                      ))}
                    </ul>
                  </div>
                )}
                <div className="versions-tabs" role="tablist">
                  {tabs.map(([id, label]) => (
                    <button
                      key={id}
                      type="button"
                      role="tab"
                      aria-selected={tab === id}
                      className={`versions-tab${tab === id ? ' is-active' : ''}`}
                      onClick={() => setTab(id)}
                    >
                      {label}
                    </button>
                  ))}
                </div>
              </Card>
              {/* 页签只隐藏不卸载：代码文件里没保存的编辑、助手能操作的字段都还在。 */}
              <div hidden={tab !== 'overview'} className="versions-panel">
                {counts && (
                  <div className="versions-summary">
                    {(
                      [
                        [t('参数改值', 'Parameter overrides'), counts.parameters],
                        [t('物理开关', 'Physics switches'), counts.physics],
                        [t('过程文件', 'Process files'), counts.files],
                        [
                          t('AI 网络', 'AI network'),
                          counts.network ? t('有', 'Yes') : t('无', 'No'),
                        ],
                      ] as const
                    ).map(([label, value]) => (
                      <div key={label} className="versions-stat">
                        <span>{label}</span>
                        <strong>{value}</strong>
                      </div>
                    ))}
                  </div>
                )}
                {editable && (
                  <Card title={t('名称与说明', 'Name and notes')}>
                    <div className="form-grid">
                      <Field label={t('名称', 'Name')}>
                        <Input
                          disabled={busy || codeBusy}
                          value={draft.name}
                          onChange={(e) => edit({ ...draft, name: e.target.value })}
                        />
                      </Field>
                      <Field label={t('说明', 'Notes')}>
                        <Input
                          disabled={busy || codeBusy}
                          value={draft.note}
                          onChange={(e) => edit({ ...draft, note: e.target.value })}
                        />
                      </Field>
                    </div>
                  </Card>
                )}
                {!draft && selected && (
                  <VersionChanges version={selected} versions={state.versions} />
                )}
                <div className="versions-applicability">
                  <Advanced
                    open={advanced}
                    onOpenChange={setAdvanced}
                    title={t(
                      '适用范围与率定/验证时段',
                      'Applicability and calibration / validation periods',
                    )}
                  >
                    <div className="form-grid">
                      {(['sites', 'land_classes'] as const).map((key) => (
                        <Field
                          key={key}
                          label={
                            key === 'sites'
                              ? t('站点（逗号分隔）', 'Sites (comma separated)')
                              : t('地类（逗号分隔）', 'Land classes (comma separated)')
                          }
                        >
                          <Input
                            disabled={!editable || busy || codeBusy}
                            value={shown.applicability[key].join(',')}
                            onChange={(e) =>
                              draft &&
                              edit({
                                ...draft,
                                applicability: {
                                  ...draft.applicability,
                                  [key]: e.target.value
                                    .split(',')
                                    .map((s) => s.trim())
                                    .filter(Boolean),
                                },
                              })
                            }
                          />
                        </Field>
                      ))}
                      {(['calibration', 'validation'] as const).map((key) => (
                        <div key={key}>
                          <p>
                            {key === 'calibration'
                              ? t('率定', 'Calibration')
                              : t('验证', 'Validation')}
                          </p>
                          {(['start', 'end'] as const).map((part) => (
                            <Field
                              key={part}
                              label={part === 'start' ? t('开始', 'Start') : t('结束', 'End')}
                            >
                              <Input
                                type="text"
                                placeholder="YYYY-MM-DD"
                                maxLength={10}
                                disabled={!editable || busy || codeBusy}
                                value={shown.applicability[key]?.[part] ?? ''}
                                onChange={(e) =>
                                  draft &&
                                  edit({
                                    ...draft,
                                    applicability: {
                                      ...draft.applicability,
                                      [key]: {
                                        start: draft.applicability[key]?.start ?? '',
                                        end: draft.applicability[key]?.end ?? '',
                                        [part]: e.target.value,
                                      },
                                    },
                                  })
                                }
                              />
                            </Field>
                          ))}
                          {editable && (
                            <Button
                              disabled={busy || codeBusy}
                              onClick={() =>
                                edit({
                                  ...draft!,
                                  applicability: { ...draft!.applicability, [key]: null },
                                })
                              }
                            >
                              {t('清除时段', 'Clear period')}
                            </Button>
                          )}
                        </div>
                      ))}
                    </div>
                    <Field
                      label={t(
                        '待冻结资产根目录（留空使用草稿目录）',
                        'Asset root (blank uses draft staging)',
                      )}
                    >
                      <Input
                        disabled={!editable || busy || codeBusy}
                        value={assetRoot}
                        onChange={(e) => setAssetRoot(e.target.value)}
                      />
                    </Field>
                  </Advanced>
                </div>
              </div>
              <div hidden={tab !== 'parameters'} className="versions-panel">
                <Card
                  title={t('参数改值', 'Parameter overrides')}
                  description={t(
                    '相对父版本改了哪些参数；没列出的沿用父版本的值。',
                    'Parameters changed from the parent; anything not listed keeps the parent’s value.',
                  )}
                >
                  {changeTable(
                    shown.overlay.parameters.map((r) => ({
                      key: r.raw_key,
                      scope: scopeText(r),
                      value: `${r.value}${r.unit ? ` ${r.unit}` : ''}`,
                      onRemove: editable
                        ? () =>
                            edit({
                              ...draft!,
                              overlay: {
                                ...draft!.overlay,
                                parameters: draft!.overlay.parameters.filter(
                                  (x) => parameterKey(x) !== parameterKey(r),
                                ),
                              },
                            })
                        : undefined,
                    })),
                    t('没有参数改值。', 'No parameter overrides.'),
                    t('移除', 'Remove'),
                  )}
                  <div className="button-row">
                    {editable && (
                      <>
                        <Button
                          disabled={busy || codeBusy}
                          onClick={() => void importStudyParameters()}
                        >
                          {t('从研究导入最优参数…', 'Import the best parameters from a study…')}
                        </Button>
                        <Button disabled={busy || codeBusy} onClick={() => run(importFile)}>
                          {t('导入参数 JSON', 'Import parameters JSON')}
                        </Button>
                      </>
                    )}
                    <Button
                      variant="ghost"
                      disabled={busy || codeBusy}
                      onClick={() =>
                        run(() =>
                          download(
                            JSON.stringify({ records: shown.overlay.parameters }, null, 2),
                            'model-parameters.json',
                            'application/json',
                          ),
                        )
                      }
                    >
                      {t('导出参数 JSON', 'Export parameters JSON')}
                    </Button>
                  </div>
                  {editable && (
                    <p className="muted versions-hint">
                      {t(
                        '从研究导入：选一个已完成的率定或敏感性研究目录，它最优成员的参数值会写成这里的参数改值。AI 网络请在研究页安装。',
                        'From a study: choose a finished calibration or sensitivity study folder; its best member’s values become overrides here. Install AI networks from the research page.',
                      )}
                    </p>
                  )}
                  {studyNotice && <Notice tone="success">{studyNotice}</Notice>}
                  {editable && (
                    <div className="versions-editor">
                      <h4>{t('添加或修改一个参数', 'Add or change a parameter')}</h4>
                      <div className="form-grid">
                        <Field label={t('搜索参数', 'Search parameters')}>
                          <Input value={query} onChange={(e) => setQuery(e.target.value)} />
                        </Field>
                        <Field label={t('参数目录', 'Parameter catalog')}>
                          <Select
                            disabled={busy || codeBusy}
                            value={parameter}
                            onChange={(e) => selectParameter(e.target.value)}
                          >
                            <option value="">—</option>
                            {allowed
                              .filter(
                                (d) =>
                                  !query ||
                                  `${d.id} ${d.label_zh} ${d.label_en}`
                                    .toLowerCase()
                                    .includes(query.toLowerCase()),
                              )
                              .map((d) => (
                                <option key={d.id} value={d.id}>
                                  {state.language === 'en' ? d.label_en : d.label_zh} · {d.id}
                                </option>
                              ))}
                          </Select>
                        </Field>
                        {descriptor &&
                          ['land-cover-class', 'pft-type', 'pc-pft-component'].includes(
                            descriptor.scope,
                          ) && (
                            <Field label={t('地类 / PFT 索引', 'Land class / PFT index')}>
                              <Select
                                value={index}
                                onChange={(e) => setIndex(Number(e.target.value))}
                              >
                                {(descriptor.scope === 'land-cover-class'
                                  ? descriptor.id.includes(':USGS:')
                                    ? USGS
                                    : IGBP
                                  : PFT
                                ).map(([id, zh, en]) => (
                                  <option key={id} value={id}>
                                    {id} · {state.language === 'en' ? en : zh}
                                  </option>
                                ))}
                              </Select>
                            </Field>
                          )}
                        {descriptor?.storage === 'process-parameter-file' && (
                          <Field label={t('冻结过程文件', 'Frozen process file')}>
                            <Select value={process} onChange={(e) => setProcess(e.target.value)}>
                              <option value="">—</option>
                              {draft.overlay.process_files.map((a) => (
                                <option key={a.relative_path}>{a.relative_path}</option>
                              ))}
                            </Select>
                          </Field>
                        )}
                        <Field
                          label={t(
                            '参数值（Namelist 字面值）',
                            'Parameter value (namelist literal)',
                          )}
                        >
                          <Input
                            value={value}
                            disabled={busy || codeBusy}
                            onChange={(e) => setValue(e.target.value)}
                          />
                        </Field>
                      </div>
                      {descriptor && (
                        <p className="muted versions-hint">
                          {state.language === 'en' ? descriptor.doc_en : descriptor.doc_zh}
                          {' · '}
                          {descriptor.unit ?? '—'} · {descriptor.validation.min ?? '−∞'} …{' '}
                          {descriptor.validation.max ?? '∞'}
                        </p>
                      )}
                      <Button
                        disabled={busy || codeBusy || !descriptor || !value}
                        onClick={() => {
                          try {
                            addParameter();
                          } catch (e) {
                            setError(String(e));
                          }
                        }}
                      >
                        {t('应用到草稿', 'Apply to draft')}
                      </Button>
                    </div>
                  )}
                </Card>
                <Card
                  title={t('物理开关', 'Physics switches')}
                  description={t(
                    '相对父版本改了的 namelist 开关。',
                    'Namelist switches changed from the parent.',
                  )}
                >
                  {changeTable(
                    Object.entries(shown.overlay.physics).map(([key, v]) => ({
                      key,
                      value: v,
                      onRemove: editable
                        ? () => {
                            const next = { ...draft!.overlay.physics };
                            delete next[key];
                            edit({ ...draft!, overlay: { ...draft!.overlay, physics: next } });
                          }
                        : undefined,
                    })),
                    t('没有改物理开关。', 'No physics switches changed.'),
                    t('重置', 'Reset'),
                  )}
                  {editable && (
                    <div className="versions-editor">
                      <h4>{t('改一个开关', 'Change a switch')}</h4>
                      <div className="form-grid">
                        <Field label={t('模型字段', 'Model field')}>
                          <Select value={physics} onChange={(e) => selectPhysics(e.target.value)}>
                            <option value="">—</option>
                            {fields
                              .filter(
                                (f) =>
                                  !f.derived &&
                                  f.name !== 'DEF_SUBGRID_SCHEME' &&
                                  !setupHandledKeys.includes(f.name) &&
                                  !allowed.some(
                                    (d) =>
                                      d.raw_key.toLowerCase() === f.name.toLowerCase() &&
                                      ['land-cover-class', 'pft-type', 'pc-pft-component'].includes(
                                        d.scope,
                                      ),
                                  ),
                              )
                              .map((f) => (
                                <option key={f.name}>{f.name}</option>
                              ))}
                          </Select>
                        </Field>
                        <Field label={t('值', 'Value')}>
                          <Input
                            value={physicsValue}
                            onChange={(e) => setPhysicsValue(e.target.value)}
                          />
                        </Field>
                      </div>
                      {modelField && (
                        <p className="muted versions-hint">
                          {modelField.doc} · {modelField.kind}
                          {modelField.values.length > 0 && ` · ${modelField.values.join(', ')}`}
                        </p>
                      )}
                      <Button
                        disabled={busy || codeBusy || !modelField || !physicsValue}
                        onClick={() => {
                          if (
                            draft.overlay.parameters.some(
                              (r) => r.raw_key.toLowerCase() === physics.toLowerCase(),
                            )
                          ) {
                            setError(
                              t('请先重置同名参数覆盖', 'Reset the parameter override first'),
                            );
                            return;
                          }
                          edit({
                            ...draft,
                            overlay: {
                              ...draft.overlay,
                              physics: { ...draft.overlay.physics, [physics]: physicsValue },
                            },
                          });
                        }}
                      >
                        {t('应用到草稿', 'Apply to draft')}
                      </Button>
                    </div>
                  )}
                </Card>
                <Card
                  title={t('AI 网络', 'AI network')}
                  description={t(
                    '网络在研究页训练和安装，只支持 Rust 引擎。',
                    'Networks are trained and installed on the research page and need the Rust engine.',
                  )}
                >
                  {shown.overlay.hybrid ? (
                    <>
                      <p>
                        {t(
                          `已挂上 AI 网络（${(shown.overlay.hybrid.assets ?? []).length} 个模型文件）。`,
                          `An AI network is attached (${(shown.overlay.hybrid.assets ?? []).length} model files).`,
                        )}
                      </p>
                      <Advanced title={t('网络配置', 'Network configuration')}>
                        <pre>{shown.overlay.hybrid.config}</pre>
                      </Advanced>
                    </>
                  ) : (
                    <p className="versions-empty">{t('没有 AI 网络。', 'No AI network.')}</p>
                  )}
                  <div className="button-row">
                    <Button disabled={busy || codeBusy} onClick={() => run(openResearch)}>
                      {draft
                        ? t('保存、冻结并进入研究', 'Save, freeze and open research')
                        : t('打开研究与网络', 'Open research and networks')}
                    </Button>
                    {draft && (
                      <span className="muted">
                        {t(
                          '会先保存并冻结这个草稿，再用新版本做研究。',
                          'Saves and freezes this draft first, then researches with the new version.',
                        )}
                      </span>
                    )}
                  </div>
                </Card>
                {editable && (
                  <Card>
                    <Advanced title={t('按路径导入过程文件', 'Import a process file by path')}>
                      <Field label={t('导入过程文件的相对路径', 'Process file relative path')}>
                        <Input
                          disabled={busy || codeBusy}
                          value={processPath}
                          onChange={(e) => setProcessPath(e.target.value)}
                        />
                      </Field>
                      <Button
                        disabled={busy || codeBusy || dirty || !processPath}
                        onClick={() =>
                          run(async () => {
                            const file = await invoke<string | null>('pick_file', {
                              key: 'model-process',
                            });
                            if (!file) return;
                            const result = await invokeJson<{
                              draft: ModelDraft;
                              assetRoot: string;
                            }>('model_import_process', {
                              id: draft.id,
                              file,
                              relativePath: processPath,
                              expectedGeneration: draft.generation,
                            });
                            setDraft(result.draft);
                            setAssetRoot(result.assetRoot);
                            setDirty(false);
                          })
                        }
                      >
                        {t('导入过程文件', 'Import process file')}
                      </Button>
                    </Advanced>
                  </Card>
                )}
              </div>
              <div hidden={tab !== 'configuration'} className="versions-panel">
                <Card
                  title={t('过程配置', 'Process options')}
                  description={t(
                    '开启父过程后展开子项；过程文件与 PFT 参数分别配置。',
                    'Parent processes reveal their options; process files and PFT parameters keep their own scopes.',
                  )}
                >
                  <ConfigurationOptions
                    engine={setupEnvironment.engine}
                    subgrid={setupEnvironment.subgrid}
                    caseValues={setupEnvironment.caseValues}
                    excludeKeys={setupHandledKeys}
                    source={
                      draft
                        ? {
                            kind: 'draft',
                            id: draft.id,
                            generation: draft.generation,
                            overlay: draft.overlay,
                          }
                        : { kind: 'frozen', version: selected!.version }
                    }
                    overlay={shown.overlay}
                    fields={fields}
                    catalog={catalog}
                    caseDir={configurationCase}
                    kernelDir={configurationKernel?.dir ?? null}
                    kernelMacros={configurationKernel?.macros}
                    caseScope={
                      setupEnvironment.domain === 'site'
                        ? 'site'
                        : setupEnvironment.domain
                          ? 'spatial'
                          : 'unknown'
                    }
                    disabled={busy || codeBusy}
                    onChange={editable ? (next) => edit({ ...draft!, overlay: next }) : undefined}
                    onImportProcess={editable ? importConfigurationProcess : undefined}
                  />
                </Card>
              </div>
              <div hidden={tab !== 'code'} className="versions-panel">
                {!shown.code.workspace && (
                  <Card>
                    <p className="versions-empty">
                      {draft
                        ? t(
                            '这个草稿沿用父版本的代码。要改代码，新建修改版时选「复制父版本的代码来修改」。',
                            'This draft keeps the parent’s code. To change code, create a modified version with “Copy the parent code to change it”.',
                          )
                        : t(
                            '这个版本用的是官方或父版本的代码，冻结后只读。要改代码，基于它新建修改版，并选「复制父版本的代码来修改」。',
                            'This version uses the official or parent code and is read-only. To change code, derive a modified version and choose “Copy the parent code to change it”.',
                          )}
                    </p>
                  </Card>
                )}
                {shown.code.workspace ? (
                  <CodeWorkspace
                    disabled={busy || codeBusy || dirty}
                    key={shown.code.workspace}
                    name={shown.code.workspace}
                    onChanged={refreshCode}
                    onPendingChange={codePending}
                  />
                ) : (
                  <Card>
                    <Advanced
                      title={t(
                        '从其他代码仓库建一个代码工作区',
                        'Create a code workspace from another repository',
                      )}
                    >
                      <CodeWorkspace
                        disabled={busy || codeBusy || dirty}
                        key="official"
                        name={null}
                        onChanged={refreshCode}
                        onPendingChange={codePending}
                      />
                    </Advanced>
                  </Card>
                )}
              </div>
              <div hidden={tab !== 'usage'} className="versions-panel">
                <Card
                  title={t('使用记录', 'Usage')}
                  description={t(
                    '最近用这个版本做过的运行、研究和其他操作。',
                    'Recent runs, studies and other operations with this version.',
                  )}
                >
                  {references.length === 0 ? (
                    <p className="versions-empty">
                      {t('还没有用这个版本做过什么。', 'Nothing has used this version yet.')}
                    </p>
                  ) : (
                    <div className="table-scroll">
                      <table className="versions-table">
                        <thead>
                          <tr>
                            <th>{t('时间', 'Time')}</th>
                            <th>{t('操作', 'Operation')}</th>
                            <th>{t('状态', 'Status')}</th>
                            <th>{t('算例或服务器', 'Case or server')}</th>
                            <th>{t('修订', 'Revision')}</th>
                          </tr>
                        </thead>
                        <tbody>
                          {references.map((r) => (
                            <tr key={r.operation_id}>
                              <td>
                                {r.time_ms == null
                                  ? '—'
                                  : new Date(r.time_ms).toLocaleString(
                                      appLocale(state.language === 'en'),
                                    )}
                              </td>
                              <td>{r.kind}</td>
                              <td>{r.status}</td>
                              <td className="versions-table-path">
                                {r.case_path ?? r.host ?? '—'}
                              </td>
                              <td>{r.revision ? `r${r.revision}` : '—'}</td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                  )}
                  <Button variant="ghost" onClick={() => navigate('history')}>
                    {t('完整历史', 'Full history')}
                  </Button>
                </Card>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

import { useEffect, useRef, useState } from 'react';
import { useApp, useNavigationGuard } from '../../lib/app-state';
import { invoke, invokeJson } from '../../lib/ipc';
import { withCatalogLabels } from '../../lib/param-presentation';
import { usePageContract, type UiField, type UiAction } from '../../lib/guide';
import type {
  AppState,
  FrozenRevision,
  ModelDraft,
  HistoryOperation,
  StudyPick,
  VersionRef,
} from '../../lib/types';
import { Advanced, Button, Card, Field, Input, Notice } from '../../components/ui';
import { download } from '../history/history';
import {
  importParameters,
  makeRecord,
  parameterKey,
  recordType,
  type ParameterType,
  upsertParameter,
  type Descriptor,
  type ModelField,
} from './model';
import CodeWorkspace from './CodeWorkspace';
import CodeFiles from './CodeFiles';
import StudyPicks from './StudyPicks';
import StudyImportDialog, { type StudyPreview } from './StudyImportDialog';
import VersionChanges from './VersionChanges';
import VersionLibrary from './VersionLibrary';
import VersionHub from './VersionHub';
import { hubMethods, type HubMethod } from './hub-methods';
import ParameterDialog from './ParameterDialog';
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
    // 新建时要不要马上复制一份父版本的代码。不选也行：草稿里点"开始修改代码"时再复制。
    [codeMode, setCodeMode] = useState<'inherit' | 'branch'>('inherit'),
    // 新版本基于哪个版本（空 = 正在看的版本，没有就是模拟正在用的）。
    [parentKey, setParentKey] = useState(''),
    // 正在看的冻结版本（版本库里点"打开"）；和模拟正在用的版本分开。
    [viewKey, setViewKey] = useState<string | null>(null),
    [paramsOpen, setParamsOpen] = useState(false),
    [paramsType, setParamsType] = useState<ParameterType>('global'),
    [paramsParameter, setParamsParameter] = useState(''),
    // 冻结前检查里正在处理的项（编引擎要两三分钟，给个说明）。
    [fixing, setFixing] = useState(''),
    // 研究结果先列出来勾选（不直接全写进草稿）；以及这个版本要研究的参数。
    [studyPreview, setStudyPreview] = useState<StudyPreview | null>(null),
    [picks, setPicks] = useState<StudyPick[]>([]),
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
    [tab, setTab] = useState<'changes' | 'code' | 'research' | 'usage'>('changes');
  const epoch = useRef({ active: false }),
    lock = useRef(false),
    // 从"修改参数（手动）"自动建的草稿：一点没改就离开时删掉，免得点一下就留个空草稿。
    autoCreated = useRef(new Set<string>()),
    draftNow = useRef<ModelDraft | null>(null);
  const keyOf = (v: VersionRef) => `${v.id}#${v.revision}`;
  const selected = state.version,
    viewing = viewKey ? (state.versions.find((v) => keyOf(v.version) === viewKey) ?? null) : null,
    shown = draft ?? viewing,
    editable = !!draft && draft.id !== 'official',
    parentVersion =
      state.versions.find((v) => keyOf(v.version) === parentKey) ?? viewing ?? selected;
  useEffect(() => {
    if (state.simulationReturn) document.querySelector('.main-pane')?.scrollTo({ top: 0 });
  }, [state.simulationReturn]);
  const [codeBusy, setCodeBusy] = useState(false),
    codeBusyNow = useRef(false);
  const dirtyNow = useRef(dirty);
  dirtyNow.current = dirty;
  draftNow.current = draft;
  function dropIfUntouched() {
    const d = draftNow.current;
    if (
      !d ||
      !autoCreated.current.has(d.id) ||
      d.generation !== 1 ||
      dirtyNow.current ||
      d.code.workspace
    )
      return;
    autoCreated.current.delete(d.id);
    void invokeJson('model_delete_draft', { id: d.id, deleteWorkspace: false })
      .then(() => invokeJson<ModelDraft[]>('model_drafts'))
      .then(setDrafts)
      .catch(() => {});
  }
  useEffect(() => () => dropIfUntouched(), []);
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
          setCatalog(withCatalogLabels(c));
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
  // 研究结果刚写进草稿：直接打开那个草稿审阅、冻结（放在上面那个重置之后，不会被它清掉）。
  // 不走 execute：开发模式下副作用会挂两次，第一次的结果被重置作废、第二次又被执行锁挡住，草稿打不开。
  useEffect(() => {
    const id = state.openDraft;
    if (!id) return;
    let live = true;
    invokeJson<ModelDraft>('model_draft', { id })
      .then((d) => {
        if (!live) return;
        update({ openDraft: null });
        setDraft(d);
        setAssetRoot('');
        setDirty(false);
        setViewKey(null);
        setTab('changes');
      })
      .catch((e) => {
        if (!live) return;
        update({ openDraft: null });
        setError(String(e));
      });
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.openDraft]);
  // 研究首页点了"修改参数（手动）"或"修改源码"：进来就建草稿。开发模式下副作用会挂两次，靠 startedMethod
  // 只认一次请求，否则会多建一个空草稿；也不按 live 丢弃结果（第一次挂载的清理会把它作废，草稿就打不开了）。
  // 不走 execute，理由同上。
  const startedMethod = useRef<AppState['versionMethod']>(null);
  // 从研究首页建的草稿：返回按钮回研究首页（返回按来路走，和研究页一样）。
  const [researchDraft, setResearchDraft] = useState<string | null>(null);
  useEffect(() => {
    const request = state.versionMethod;
    if (!request || startedMethod.current === request) return;
    startedMethod.current = request;
    update({ versionMethod: null });
    lock.current = true;
    setBusy(true);
    startMethod(request.method)
      .then((d) => setResearchDraft(d?.id ?? null))
      .catch((e) => setError(String(e)))
      .finally(() => {
        lock.current = false;
        setBusy(false);
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.versionMethod]);
  useEffect(() => {
    let live = true;
    setReferences([]);
    invokeJson<{ items: HistoryOperation[] }>('history_list', {
      filter: { versionId: draft?.id ?? viewing?.version.id ?? state.versionRef.id, limit: 100 },
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
  }, [draft?.id, viewing?.version.id, state.versionRef.id]);
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
  // finish：这是"冻结为新版本"，冻结后草稿删掉；为做研究而冻结时草稿留着，研究结果还要写回它。
  async function freeze(target = draft, unsaved = dirtyNow.current, finish = true) {
    const frozen = await researchVersion(target, unsaved, viewing ?? selected, assetRoot, {
      save,
      freeze: (id, root) =>
        invokeJson<FrozenRevision>('model_freeze', { id, assetRoot: root, finish }),
    });
    update({ version: frozen, versionRef: frozen.version, chosenVersion: frozen.version });
    setDraft(null);
    dirtyNow.current = false;
    setDirty(false);
    setViewKey(keyOf(frozen.version));
    setTab('changes');
    await refreshVersions();
    setDrafts(await invokeJson('model_drafts'));
    return frozen;
  }
  function returnToSimulation() {
    const target = state.simulationReturn;
    if (!target) return;
    return navigate(target.route, target.route === 'case' ? 'settings' : undefined);
  }
  async function openResearch() {
    if (draft) autoCreated.current.delete(draft.id);
    if (draft) await freeze(draft, dirtyNow.current, false);
    else if (viewing) chooseForSimulation(viewing);
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
  // 打开一个冻结版本看；不再顺手把它设为模拟使用的版本（2026-10-20 用户同意拆开）。
  async function select(v: FrozenRevision) {
    if (dirty)
      throw new Error(
        t('请先保存草稿，或放弃本地修改', 'Save the draft or discard local changes first'),
      );
    setDraft(null);
    setViewKey(keyOf(v.version));
    setTab('changes');
  }
  function chooseForSimulation(v: FrozenRevision) {
    update({ version: v, versionRef: v.version, chosenVersion: v.version });
  }
  function backToLibrary() {
    if (dirtyNow.current)
      throw new Error(
        t('请先保存草稿，或放弃本地修改', 'Save the draft or discard local changes first'),
      );
    dropIfUntouched();
    setDraft(null);
    setViewKey(null);
  }
  // "新建修改版"的 6 种方式。手动改参数、修改源码直接建草稿（自动起名，之后在草稿里能改）；
  // 其余 4 种是研究：以所选版本为基础，直接打开研究页对应的方法。
  async function startMethod(method: HubMethod, base = parentVersion) {
    if (!base) throw new Error(t('请选择基于哪个版本', 'Choose the version to start from'));
    if (method === 'manual' || method === 'source') {
      const stamp = new Date().toLocaleString(appLocale(state.language === 'en'), {
        month: '2-digit',
        day: '2-digit',
        hour: '2-digit',
        minute: '2-digit',
      });
      const what =
        method === 'manual'
          ? t('手动修改参数', 'Manual parameter changes')
          : t('修改源码', 'Source code changes');
      const d = await invokeJson<ModelDraft>('model_create', {
        name: `${what} ${stamp}`,
        note: `${what}（${t('基于', 'based on')} ${base.name} r${base.version.revision}）`,
        parent: base.version,
        workspace: null,
        branchCode: method === 'source',
      });
      if (method === 'manual') autoCreated.current.add(d.id);
      setAssetRoot('');
      setDraft(d);
      setDirty(false);
      setViewKey(null);
      setTab(method === 'source' ? 'code' : 'changes');
      setDrafts(await invokeJson('model_drafts'));
      if (method === 'manual') openParameters('global');
      return d;
    }
    if (!selected || keyOf(selected.version) !== keyOf(base.version)) chooseForSimulation(base);
    // 从"新建修改版"开始的研究不绑定草稿：结果出来时再建一个以研究所用版本为父版本的草稿。
    update({ researchMethod: method, researchTarget: null, researchSeed: null });
    lock.current = false;
    if (!navigate('research')) throw new Error(t('当前修改尚未保存', 'Current edits are unsaved'));
    return null;
  }
  // 草稿要改代码时才复制一份父版本的代码（约 3 秒），不改代码的草稿不用复制、也不用编译。
  async function branchCode() {
    if (!draft) return;
    const current = dirtyNow.current ? await save() : draft;
    const next = await invokeJson<ModelDraft>('model_branch_code', {
      id: current.id,
      expectedGeneration: current.generation,
    });
    setDraft(next);
    dirtyNow.current = false;
    setDirty(false);
    setDrafts(await invokeJson('model_drafts'));
  }
  // 冻结前检查里能直接处理的项：编引擎、编内核（用当前算例对应的内核预设，没有就 default）、跑 drift。
  const fixes: Record<string, string> = {
    engine_built: 'build-engine',
    kernel_built: 'build-kernel',
    drift: 'test-drift',
  };
  async function fixFreezeItems(codes: string[]) {
    const name = draft?.code.workspace;
    if (!name) return;
    for (const code of codes) {
      const action = fixes[code];
      if (!action) continue;
      setFixing(code);
      await invokeJson('workspace_check', {
        name,
        action,
        preset: configurationKernel?.preset ?? 'default',
        package: null,
        case: state.selectedCase?.entry.dir ?? null,
      });
    }
    setFixing('');
    setCheckTick((value) => value + 1);
  }
  async function adoptWorkspace(name: string) {
    if (!draft || !name.trim()) return;
    await save({ ...draft, code: { ...draft.code, workspace: name.trim() } });
  }
  async function loadDraft(id: string) {
    if (dirty) throw new Error(t('请先保存草稿', 'Save the current draft first'));
    const token = epoch.current;
    const d = await invokeJson<ModelDraft>('model_draft', { id });
    if (token.active && token === epoch.current) {
      setDraft(d);
      setAssetRoot('');
      setDirty(false);
      setViewKey(null);
      setTab('changes');
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
  // 先列出研究最优成员会带进来的参数，勾选后再写（previewStudy → importChosen）。
  async function previewStudy() {
    await execute(async () => {
      if (!draft) throw new Error('No current draft');
      const studyDir = await invoke<string | null>('pick_folder', { key: 'model-study' });
      if (!studyDir) return;
      const result = await invokeJson<Omit<StudyPreview, 'study'>>('model_import_study', {
        id: draft.id,
        studyDir,
        member: null,
        expectedGeneration: draft.generation,
        preview: true,
        only: null,
      });
      setStudyPreview({ ...result, study: studyDir });
    });
  }
  async function importChosen(chosen: [string, string | null, number | null][]) {
    await execute(async () => {
      if (!draft || !studyPreview) return;
      const current = dirtyNow.current ? await save() : draft;
      const result = await invokeJson<{ draft: ModelDraft; member: string; parameters: number }>(
        'model_import_study',
        {
          id: current.id,
          studyDir: studyPreview.study,
          member: studyPreview.member,
          expectedGeneration: current.generation,
          preview: null,
          only: chosen,
        },
      );
      setDraft(result.draft);
      dirtyNow.current = false;
      setDirty(false);
      setDrafts(await invokeJson('model_drafts'));
      setStudyPreview(null);
      setStudyNotice(
        t(
          `已把研究成员 ${result.member} 的 ${result.parameters} 个参数写进这个版本，并记下了研究的站点与时段。`,
          `Wrote ${result.parameters} parameters of study member ${result.member} into this version and recorded the study's sites and periods.`,
        ),
      );
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
        {
          id: current.id,
          studyDir,
          member: null,
          expectedGeneration: current.generation,
          preview: null,
          only: null,
        },
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
    if (!parentVersion) return null;
    const d = await invokeJson<ModelDraft>('model_create', {
      name,
      note,
      parent: parentVersion.version,
      workspace: null,
      branchCode: codeMode === 'branch',
    });
    setAssetRoot('');
    setDraft(d);
    setDirty(false);
    setViewKey(null);
    setTab(codeMode === 'branch' ? 'code' : 'changes');
    setName('');
    setNote('');
    setCodeMode('inherit');
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
        {
          value: 'inherit',
          label: t(
            '先只改参数（代码要改时再复制）',
            'Parameters first (copy the code when needed)',
          ),
        },
        {
          value: 'branch',
          label: t('现在就复制父版本的代码来修改', 'Copy the parent code now to change it'),
        },
      ],
      set: (value) => setCodeMode(value as typeof codeMode),
    },
    {
      id: 'version-parent',
      label: t('基于哪个版本', 'Based on'),
      value: parentVersion ? keyOf(parentVersion.version) : '',
      type: 'choice',
      options: state.versions.map((v) => ({
        value: keyOf(v.version),
        label: `${v.name} r${v.version.revision}`,
      })),
      set: setParentKey,
    },
  ];
  const guideActions: UiAction[] = [
    {
      id: 'version-create',
      label: t('派生新版本', 'Derive version'),
      kind: 'commit',
      enabled: !busy && !codeBusy && !dirty && !!name && !!note && !!parentVersion,
      run: () => execute(deriveVersion),
    },
    ...state.versions.map((v) => ({
      id: `version-select-${v.version.id}-${v.version.revision}`,
      label: `${v.name} r${v.version.revision}`,
      kind: 'click' as const,
      enabled: !busy && !codeBusy && !dirty,
      run: () => execute(() => select(v)),
    })),
    ...hubMethods.map((m) => ({
      id: `version-method-${m.id}`,
      label: state.language === 'en' ? m.en : m.zh,
      kind: (m.id === 'manual' || m.id === 'source' ? 'commit' : 'click') as 'commit' | 'click',
      enabled: !busy && !codeBusy && !dirty && !!parentVersion,
      run: () => execute(() => startMethod(m.id)),
    })),
    ...state.versions.map((v) => ({
      id: `version-use-${v.version.id}-${v.version.revision}`,
      label: t(
        `设为模拟使用：${v.name} r${v.version.revision}`,
        `Use for simulations: ${v.name} r${v.version.revision}`,
      ),
      kind: 'click' as const,
      enabled: !busy && !codeBusy && !dirty,
      run: () => chooseForSimulation(v),
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
  if (editable && draft && !draft.code.workspace)
    guideActions.push({
      id: 'version-branch-code',
      label: t('开始修改代码', 'Start editing code'),
      kind: 'commit',
      enabled: !busy && !codeBusy,
      run: () => execute(branchCode),
    });
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
    const found = state.versions.find((v) => keyOf(v.version) === keyOf(ref));
    return found ? `${found.name} · r${found.version.revision}` : `${ref.id} r${ref.revision}`;
  };
  // 派生链：官方 r101 › 根系方案 r1 › 根系方案调参 r2。
  const backToResearch = !!draft && draft.id === researchDraft;
  const lineage = (() => {
    const chain: string[] = [];
    let ref = shown?.parent ?? null;
    for (let guard = 0; ref && guard < 32; guard++) {
      chain.unshift(parentOf(ref)!);
      ref = state.versions.find((v) => keyOf(v.version) === keyOf(ref!))?.parent ?? null;
    }
    return chain;
  })();
  const scopeText = (r: ModelDraft['overlay']['parameters'][number]) => {
    const s = r.scope_instance;
    const label = (table: [number, string, string][], id: number | null) => {
      const hit = table.find(([n]) => n === id);
      return hit ? `${id} ${state.language === 'en' ? hit[2] : hit[1]}` : String(id ?? '');
    };
    switch (s.kind) {
      case 'case-scalar':
        return '—';
      case 'land-cover-class':
        return label(s.scheme === 'USGS' ? USGS : IGBP, s.index ?? null);
      case 'pft':
      case 'pc-pft':
        return label(PFT, s.index ?? null);
      default:
        return [s.process_file, s.type_name].filter(Boolean).join(' · ');
    }
  };
  const typeName: Record<ParameterType, string> = {
    global: t('全局', 'Global'),
    igbp: t('地类 IGBP', 'IGBP class'),
    usgs: t('地类 USGS', 'USGS class'),
    pft: 'PFT',
    pc: 'PC',
    process: t('过程文件', 'Process file'),
  };
  const isOfficial = !draft && viewing?.version.id === 'official';
  const inUse =
    !draft && !!viewing && !!selected && keyOf(viewing.version) === keyOf(selected.version);
  const tabs: [typeof tab, string][] = [
    ['changes', t('改动', 'Changes')],
    ['code', t('代码', 'Code')],
    ['research', t('研究', 'Research')],
    ['usage', `${t('使用记录', 'Usage')}${references.length ? ` (${references.length})` : ''}`],
  ];
  const openParameters = (type: ParameterType = 'global', parameterId = '') => {
    setParamsType(type);
    setParamsParameter(parameterId);
    setParamsOpen(true);
  };
  const sortedParameters = shown
    ? [...shown.overlay.parameters].sort(
        (a, b) =>
          Object.keys(typeName).indexOf(recordType(a)) -
            Object.keys(typeName).indexOf(recordType(b)) ||
          a.raw_key.localeCompare(b.raw_key) ||
          (a.scope_instance.index ?? 0) - (b.scope_instance.index ?? 0),
      )
    : [];
  // 改动清单：同一个参数改了好几个地类或植物类型时并成一行（原来一个参数整体缩放就是 17 行）。
  const parameterRows = (() => {
    const groups: { key: string; records: typeof sortedParameters }[] = [];
    for (const r of sortedParameters) {
      const key = `${recordType(r)}|${r.parameter_id}`;
      const last = groups[groups.length - 1];
      if (last?.key === key) last.records.push(r);
      else groups.push({ key, records: [r] });
    }
    return groups;
  })();

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
      {!shown && state.versionsView !== 'library' ? (
        <>
          {error && <Notice tone="error">{error}</Notice>}
          <VersionHub
            versions={state.versions}
            drafts={drafts}
            base={parentVersion}
            inUse={selected}
            en={state.language === 'en'}
            disabled={busy || codeBusy}
            onBase={setParentKey}
            onMethod={(method) => run(() => startMethod(method))}
            onOpenDraft={(id) => run(() => loadDraft(id))}
            onLibrary={() => update({ versionsView: 'library' })}
          />
        </>
      ) : !shown ? (
        <>
          <div className="page-heading versions-heading">
            <div>
              <h1>{t('所有版本', 'All versions')}</h1>
              <p className="muted">
                {t(
                  '版本 = 官方代码 + 你的改动（参数、开关、AI 网络或代码本身）。打开一个版本查看或修改；模拟用哪个版本，用「设为模拟使用」选。',
                  'A version is the official code plus your changes (parameters, switches, an AI network or the code itself). Open one to inspect or change it; choose which one simulations use with “Use for simulations”.',
                )}
              </p>
            </div>
            <Button
              variant="primary"
              disabled={busy || codeBusy}
              onClick={() => update({ versionsView: 'create' })}
            >
              {`＋ ${t('新建修改版', 'New modified version')}`}
            </Button>
          </div>
          {error && <Notice tone="error">{error}</Notice>}
          <VersionLibrary
            versions={state.versions}
            drafts={drafts}
            inUse={selected}
            disabled={busy || codeBusy}
            en={state.language === 'en'}
            onOpen={(v) => run(() => select(v))}
            onUse={chooseForSimulation}
            onOpenDraft={(id) => run(() => loadDraft(id))}
            onDeleted={refreshVersions}
            onDraftDeleted={async () => {
              setDrafts(await invokeJson('model_drafts'));
            }}
          />
        </>
      ) : (
        <>
          <div className="versions-crumbs">
            <button
              type="button"
              disabled={busy || codeBusy}
              onClick={() =>
                run(async () => {
                  backToLibrary();
                  if (!backToResearch) return;
                  setResearchDraft(null);
                  // run 正拿着执行锁，离开保护会把跳转拦下（预览里实测停在了汇总页）；草稿已处理完，放开再走。
                  lock.current = false;
                  navigate('research');
                })
              }
            >
              ←{' '}
              {backToResearch
                ? t('研究首页', 'Research home')
                : state.versionsView === 'library'
                  ? t('所有版本', 'All versions')
                  : t('新建修改版', 'New modified version')}
            </button>
            {lineage.length > 0 && (
              <span className="muted">
                {lineage.join(' › ')} › <strong>{shown.name}</strong>
              </span>
            )}
          </div>
          {error && <Notice tone="error">{error}</Notice>}
          <Card className="versions-head">
            <div className="versions-head-top">
              <div className="versions-head-title">
                <h2>{shown.name}</h2>
                <span
                  className={`badge versions-badge is-${draft ? 'draft' : isOfficial ? 'official' : 'modified'}`}
                >
                  {draft
                    ? t('草稿', 'Draft')
                    : isOfficial
                      ? t('官方', 'Official')
                      : t('修改版', 'Modified')}
                </span>
                {!draft && <span className="versions-rev">r{viewing?.version.revision}</span>}
                {inUse && (
                  <span className="badge versions-badge is-used">
                    {t('模拟正在使用', 'Used for simulations')}
                  </span>
                )}
                {draft && dirty && (
                  <span className="badge versions-badge is-unsaved">{t('未保存', 'Unsaved')}</span>
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
                    {!inUse && viewing && (
                      <Button
                        variant="primary"
                        disabled={busy || codeBusy}
                        onClick={() => chooseForSimulation(viewing)}
                      >
                        {t('设为模拟使用', 'Use for simulations')}
                      </Button>
                    )}
                    <Button
                      disabled={busy || codeBusy}
                      onClick={() =>
                        run(async () => {
                          setParentKey(viewing ? keyOf(viewing.version) : '');
                          backToLibrary();
                          update({ versionsView: 'create' });
                        })
                      }
                    >
                      {t('基于它新建修改版', 'Derive from it')}
                    </Button>
                  </>
                )}
              </div>
            </div>
            <p className="versions-note">
              {isOfficial
                ? t(
                    '随软件提供的官方基线，只读。要改参数或代码，基于它新建修改版。',
                    'The official baseline shipped with the app, read-only. Derive a modified version to change parameters or code.',
                  )
                : shown.note || <span className="muted">{t('没有写说明。', 'No notes.')}</span>}
            </p>
            {editable && freezeCheck && (
              <div className="freeze-check" aria-label={t('冻结前检查', 'Checks before freezing')}>
                <div className="freeze-check-head">
                  <strong>
                    {freezeCheck.ready
                      ? t('冻结前检查：可以冻结', 'Checks before freezing: ready')
                      : t('冻结前检查：还不能冻结', 'Checks before freezing: not yet')}
                  </strong>
                  <span className="button-row freeze-check-actions">
                    {freezeCheck.items.some((item) => item.blocking && fixes[item.code]) && (
                      <Button
                        disabled={busy || codeBusy || dirty}
                        onClick={() =>
                          run(() =>
                            fixFreezeItems(
                              freezeCheck.items
                                .filter((item) => item.blocking && fixes[item.code])
                                .map((item) => item.code),
                            ),
                          )
                        }
                      >
                        {busy ? t('处理中…', 'Working…') : t('一键处理', 'Fix all')}
                      </Button>
                    )}
                    <Button
                      variant="ghost"
                      disabled={busy || codeBusy}
                      onClick={() => setCheckTick((value) => value + 1)}
                    >
                      {t('重新检查', 'Check again')}
                    </Button>
                  </span>
                </div>
                {fixing && busy && (
                  <Notice>
                    {fixing === 'engine_built'
                      ? t(
                          '正在编译这个版本的 Rust 引擎，通常要两三分钟…',
                          'Building this version’s Rust engine; this usually takes two or three minutes…',
                        )
                      : fixing === 'kernel_built'
                        ? t(
                            '正在编译内核，可能要几分钟…',
                            'Building the kernel; this can take a few minutes…',
                          )
                        : t('正在检查生成表…', 'Checking the generated tables…')}
                  </Notice>
                )}
                <ul>
                  {freezeCheck.items.map((item) => (
                    <li
                      key={item.code}
                      className={item.blocking ? 'is-blocking' : item.ok ? 'is-ok' : 'is-note'}
                    >
                      <span aria-hidden="true">{item.blocking ? '✕' : item.ok ? '✓' : 'i'}</span>
                      <span className="freeze-check-message">{item.message}</span>
                      {item.blocking && fixes[item.code] && (
                        <button
                          type="button"
                          className="library-action"
                          disabled={busy || codeBusy || dirty}
                          onClick={() => run(() => fixFreezeItems([item.code]))}
                        >
                          {item.code === 'drift'
                            ? t('现在检查', 'Check now')
                            : item.code === 'kernel_built'
                              ? t(
                                  `编译内核（${configurationKernel?.preset ?? 'default'}）`,
                                  `Build kernel (${configurationKernel?.preset ?? 'default'})`,
                                )
                              : t('编译引擎', 'Build engine')}
                        </button>
                      )}
                    </li>
                  ))}
                </ul>
              </div>
            )}
            <div className="versions-summary">
              {(
                [
                  [t('参数改值', 'Parameter overrides'), String(shown.overlay.parameters.length)],
                  [
                    t('开关改值', 'Switch values'),
                    String(Object.keys(shown.overlay.physics).length),
                  ],
                  [t('过程文件', 'Process files'), String(shown.overlay.process_files.length)],
                  [
                    t('AI 网络', 'AI network'),
                    shown.overlay.hybrid ? t('有', 'Yes') : t('无', 'No'),
                  ],
                  [
                    t('代码', 'Code'),
                    shown.code.workspace
                      ? t('自己的', 'Own')
                      : isOfficial
                        ? t('官方', 'Official')
                        : t('同父版本', 'Parent’s'),
                  ],
                ] as const
              ).map(([label, value]) => (
                <div key={label} className="versions-stat">
                  <span>{label}</span>
                  <strong>{value}</strong>
                </div>
              ))}
            </div>
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
          <div hidden={tab !== 'changes'} className="versions-panel">
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
            <Card
              title={t('参数改值', 'Parameter overrides')}
              description={t(
                '相对父版本改了的参数；没列出的沿用父版本的值。全局、地类、PFT、PC、过程文件参数都在「修改参数」里按类型改。',
                'Parameters changed from the parent; anything not listed keeps the parent’s value. Edit global, land-class, PFT, PC and process-file parameters by type under “Edit parameters”.',
              )}
            >
              <div className="button-row versions-card-actions">
                <Button variant={editable ? 'primary' : undefined} onClick={() => openParameters()}>
                  {editable
                    ? t('修改参数…', 'Edit parameters…')
                    : t('查看全部参数…', 'All parameters…')}
                </Button>
                {editable && (
                  <>
                    <Button
                      disabled={busy || codeBusy}
                      onClick={() => void previewStudy().catch(() => {})}
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
              {studyNotice && <Notice tone="success">{studyNotice}</Notice>}
              {sortedParameters.length === 0 ? (
                <p className="versions-empty">
                  {editable
                    ? t(
                        '还没有参数改值。点「修改参数」按类型改。',
                        'No parameter overrides yet. Use “Edit parameters”.',
                      )
                    : t('没有参数改值。', 'No parameter overrides.')}
                </p>
              ) : (
                <div className="table-scroll">
                  <table className="versions-table">
                    <thead>
                      <tr>
                        <th>{t('类型', 'Type')}</th>
                        <th>{t('参数', 'Parameter')}</th>
                        <th>{t('地类 / 植物类型', 'Class / plant type')}</th>
                        <th>{t('值', 'Value')}</th>
                        <th />
                      </tr>
                    </thead>
                    <tbody>
                      {parameterRows.map(({ key, records: group }) => {
                        const r = group[0]!;
                        const many = group.length > 2;
                        const values = group.map((x) => x.value);
                        return (many ? [r] : group).map((row) => (
                          <tr key={many ? key : parameterKey(row)}>
                            <td>
                              <button
                                type="button"
                                className="library-action"
                                onClick={() => openParameters(recordType(row), row.parameter_id)}
                              >
                                {typeName[recordType(row)]}
                              </button>
                            </td>
                            <td>
                              <code>{row.raw_key}</code>
                            </td>
                            <td className="muted">
                              {many
                                ? recordType(row) === 'pft' || recordType(row) === 'pc'
                                  ? t(`${group.length} 个植物类型`, `${group.length} plant types`)
                                  : t(`${group.length} 个地类`, `${group.length} classes`)
                                : scopeText(row)}
                            </td>
                            <td>
                              <code>
                                {many
                                  ? `${values.reduce((a, b) => (Number(a) <= Number(b) ? a : b))} … ${values.reduce((a, b) => (Number(a) >= Number(b) ? a : b))}`
                                  : row.value}
                                {row.unit ? ` ${row.unit}` : ''}
                              </code>
                            </td>
                            <td className="versions-table-action">
                              {many && (
                                <Button
                                  variant="ghost"
                                  onClick={() => openParameters(recordType(row), row.parameter_id)}
                                >
                                  {t('在矩阵里看', 'Open matrix')}
                                </Button>
                              )}
                              {editable && (
                                <Button
                                  variant="ghost"
                                  disabled={busy || codeBusy}
                                  onClick={() =>
                                    edit({
                                      ...draft!,
                                      overlay: {
                                        ...draft!.overlay,
                                        parameters: draft!.overlay.parameters.filter((x) =>
                                          many
                                            ? x.parameter_id !== row.parameter_id
                                            : parameterKey(x) !== parameterKey(row),
                                        ),
                                      },
                                    })
                                  }
                                >
                                  {many ? t('全部移除', 'Remove all') : t('移除', 'Remove')}
                                </Button>
                              )}
                            </td>
                          </tr>
                        ));
                      })}
                    </tbody>
                  </table>
                </div>
              )}
            </Card>
            {Object.keys(shown.overlay.physics).length > 0 && (
              <Card
                title={t('开关改值', 'Switch values')}
                description={t(
                  '在「其他模型设置」里按过程写下的 namelist 值。',
                  'Namelist values written under “Other model settings”.',
                )}
              >
                <table className="versions-table">
                  <tbody>
                    {Object.entries(shown.overlay.physics).map(([key, v]) => (
                      <tr key={key}>
                        <td>
                          <code>{key}</code>
                        </td>
                        <td>
                          <code>{v}</code>
                        </td>
                        {editable && (
                          <td className="versions-table-action">
                            <Button
                              variant="ghost"
                              disabled={busy || codeBusy}
                              onClick={() => {
                                const next = { ...draft!.overlay.physics };
                                delete next[key];
                                edit({ ...draft!, overlay: { ...draft!.overlay, physics: next } });
                              }}
                            >
                              {t('重置', 'Reset')}
                            </Button>
                          </td>
                        )}
                      </tr>
                    ))}
                  </tbody>
                </table>
              </Card>
            )}
            {!draft && viewing && <VersionChanges version={viewing} versions={state.versions} />}
            <Card>
              <Advanced
                title={t(
                  '其他模型设置：示踪物、过程文件、按过程看参数',
                  'Other model settings: tracers, process files, parameters by process',
                )}
              >
                <p className="muted versions-hint">
                  {t(
                    '过程开关（是否启用某个过程、用哪个方案）属于算例，在模拟设置里选；这里只有版本能带的设置。',
                    'Process switches (whether a process runs, which scheme) belong to a case and are chosen in the simulation setup; only settings a version can carry are here.',
                  )}
                </p>
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
                      : { kind: 'frozen', version: viewing!.version }
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
              </Advanced>
            </Card>
            <Card>
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
                        {key === 'calibration' ? t('率定', 'Calibration') : t('验证', 'Validation')}
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
            </Card>
          </div>
          <div hidden={tab !== 'code'} className="versions-panel">
            {shown.code.workspace && draft ? (
              <CodeWorkspace
                disabled={busy || codeBusy || dirty}
                key={shown.code.workspace}
                name={shown.code.workspace}
                onChanged={refreshCode}
                onPendingChange={codePending}
              />
            ) : (
              <Card title={t('代码', 'Code')}>
                {draft ? (
                  <>
                    <p>
                      {t(
                        '这个草稿还在用父版本的代码。要改代码，点下面的按钮：会复制一份父版本冻结时的代码（约 3 秒），之后可以直接改任意 Rust 或 Fortran 文件、替换或新增文件；冻结时这份代码随版本一起冻结。',
                        'This draft still uses the parent’s code. To change code, start below: it copies the parent’s frozen code (about 3 seconds); then edit any Rust or Fortran file, replace or add files; freezing freezes this code with the version.',
                      )}
                    </p>
                    <div className="button-row">
                      <Button
                        variant="primary"
                        disabled={busy || codeBusy}
                        onClick={() => run(branchCode)}
                      >
                        {t('开始修改代码', 'Start editing code')}
                      </Button>
                    </div>
                    <h3 className="versions-subhead">
                      {t('父版本的代码（只读）', 'The parent’s code (read-only)')}
                    </h3>
                    <CodeFiles
                      name=""
                      message=""
                      en={state.language === 'en'}
                      disabled={false}
                      onChanged={async () => {}}
                      source={draft.parent}
                    />
                    <Advanced
                      title={t(
                        '改用一个已有的代码工作区',
                        'Use an existing code workspace instead',
                      )}
                    >
                      <Field label={t('代码工作区名称', 'Code workspace name')}>
                        <Input value={workspace} onChange={(e) => setWorkspace(e.target.value)} />
                      </Field>
                      <Button
                        disabled={busy || codeBusy || !workspace.trim()}
                        onClick={() => run(() => adoptWorkspace(workspace))}
                      >
                        {t('使用这个工作区', 'Use this workspace')}
                      </Button>
                    </Advanced>
                  </>
                ) : (
                  <>
                    <p>
                      {shown.code.workspace
                        ? t(
                            `这个版本有自己的代码（提交 ${shown.code.commit?.slice(0, 8) ?? ''}），冻结后只读。它改了哪些文件见「改动」里的「这个版本改了什么」。`,
                            `This version has its own code (commit ${shown.code.commit?.slice(0, 8) ?? ''}), read-only once frozen. See “What this version changes” under Changes for the files.`,
                          )
                        : isOfficial
                          ? t('官方源码，只读。', 'The official source, read-only.')
                          : t(
                              '这个版本用的是父版本的代码，只读。',
                              'This version uses its parent’s code, read-only.',
                            )}
                    </p>
                    <div className="button-row">
                      <Button
                        variant="primary"
                        disabled={busy || codeBusy}
                        onClick={() => run(() => startMethod('source', viewing))}
                      >
                        {t('修改代码…', 'Change the code…')}
                      </Button>
                      <span className="muted">
                        {t(
                          '基于这个版本新建一个草稿，并直接复制它的代码来改。',
                          'Creates a draft from this version with a copy of its code to edit.',
                        )}
                      </span>
                    </div>
                    {viewing && (
                      <CodeFiles
                        name=""
                        message=""
                        en={state.language === 'en'}
                        disabled={false}
                        onChanged={async () => {}}
                        source={viewing.version}
                      />
                    )}
                  </>
                )}
                <Advanced
                  title={t(
                    '从其他代码仓库建一个代码工作区',
                    'Create a code workspace from another repository',
                  )}
                >
                  <CodeWorkspace
                    disabled={busy || codeBusy || dirty}
                    key="other"
                    name={null}
                    onChanged={refreshCode}
                    onPendingChange={codePending}
                  />
                </Advanced>
              </Card>
            )}
          </div>
          <div hidden={tab !== 'research'} className="versions-panel">
            <Card
              title={t('要研究的参数', 'Parameters to study')}
              description={t(
                '先在这里定好研究哪些参数和范围（地类、PFT 参数可以逐个类型选），再去研究页选算例、目标和方法；这里选的会自动勾上。',
                'Choose the parameters and ranges here (land-class and PFT parameters type by type), then pick cases, targets and a method on the research page; these are ticked there automatically.',
              )}
            >
              <StudyPicks
                owner={draft ? draft.id : viewing ? keyOf(viewing.version) : ''}
                catalog={allowed}
                en={state.language === 'en'}
                disabled={busy || codeBusy}
                onChange={setPicks}
              />
              <p className="versions-label">
                {draft
                  ? t(
                      `用这个草稿做（会先保存并冻结一个修订给研究用${picks.length ? `，带上 ${picks.length} 个参数` : ''}）：`,
                      `Study this draft (a revision is saved and frozen for the study first${picks.length ? `, with ${picks.length} parameters` : ''}):`,
                    )
                  : t(
                      `用这个版本做${picks.length ? `（带上 ${picks.length} 个参数）` : ''}：`,
                      `Study this version${picks.length ? ` (with ${picks.length} parameters)` : ''}:`,
                    )}
              </p>
              <div className="button-row">
                {hubMethods
                  .filter((m) => m.id !== 'manual' && m.id !== 'source')
                  .map((m, i) => (
                    <Button
                      key={m.id}
                      variant={i === 0 ? 'primary' : undefined}
                      disabled={busy || codeBusy}
                      onClick={() => {
                        update({
                          researchSeed: picks.length ? { parameters: picks } : null,
                          researchMethod: m.id,
                          // 从草稿进去的研究，结果直接写回这个草稿，不用再回来选研究目录。
                          researchTarget: draft ? { draftId: draft.id, name: draft.name } : null,
                        });
                        // 没去成研究页就把种子和方法清掉，免得以后在研究页莫名其妙地生效。
                        run(() =>
                          openResearch().catch((e) => {
                            update({
                              researchSeed: null,
                              researchMethod: null,
                              researchTarget: null,
                            });
                            throw e;
                          }),
                        );
                      }}
                    >
                      {state.language === 'en' ? m.en : m.zh}
                    </Button>
                  ))}
              </div>
            </Card>
            {editable && (
              <Card
                title={t('研究结果', 'Study results')}
                description={t(
                  '跑完的率定或敏感性研究，可以把最优成员的参数勾选后写进这个草稿。',
                  'Tick parameters from the best member of a finished calibration or sensitivity study to write them into this draft.',
                )}
              >
                <div className="button-row">
                  {editable && (
                    <Button
                      disabled={busy || codeBusy}
                      onClick={() => void previewStudy().catch(() => {})}
                    >
                      {t('从研究导入最优参数…', 'Import the best parameters from a study…')}
                    </Button>
                  )}
                </div>
                {draft && (
                  <p className="muted versions-hint">
                    {t(
                      '研究要用冻结的版本：会先把这个草稿冻结成一个修订给研究用，草稿留着；研究跑完在研究页点"写回草稿"就回到这里。别处做的研究用上面的按钮导入。',
                      'Research needs a frozen version: the draft is frozen into a revision for the study and kept; when the study finishes, "Write back to draft" on the research page returns here. Use the button above for studies made elsewhere.',
                    )}
                  </p>
                )}
              </Card>
            )}
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
            </Card>
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
                          <td className="versions-table-path">{r.case_path ?? r.host ?? '—'}</td>
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
          {draft && (
            <StudyImportDialog
              preview={studyPreview}
              overlay={draft.overlay}
              en={state.language === 'en'}
              busy={busy}
              onClose={() => setStudyPreview(null)}
              onImport={(chosen) => void importChosen(chosen).catch(() => {})}
            />
          )}
          <ParameterDialog
            open={paramsOpen}
            onOpenChange={setParamsOpen}
            catalog={allowed}
            overlay={shown.overlay}
            editable={editable}
            en={state.language === 'en'}
            onChange={(next) => draft && edit({ ...draft, overlay: next })}
            initialType={paramsType}
            initialParameter={paramsParameter}
          />
        </>
      )}
    </div>
  );
}

import { useEffect, useRef, useState } from 'react';
import { useApp, useNavigationGuard } from '../../lib/app-state';
import { invoke, invokeJson } from '../../lib/ipc';
import { usePageContract, type UiField, type UiAction } from '../../lib/guide';
import type { FrozenRevision, ModelDraft, HistoryOperation } from '../../lib/types';
import { Advanced, Button, Card, Field, Input, Notice, Select } from '../../components/ui';
import { download } from '../history/history';
import {
  importParameters,
  makeRecord,
  parameterKey,
  upsertParameter,
  versionDepth,
  type Descriptor,
  type ModelField,
} from './model';
import CodeWorkspace from './CodeWorkspace';
import ConfigurationOptions from './ConfigurationOptions';
import { setupHandledKeys, type SetupEnvironment } from './scientific-setup';
import { compatibleKernel } from '../runs/run-model';
import { effectiveModelFields } from '../cases/case-model';
import { IGBP, USGS, PFT } from './scope-labels';
import { researchVersion, refreshDraftCode } from './research-version';
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
    [processPath, setProcessPath] = useState('');
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
      workspace: workspace || null,
    });
    setAssetRoot('');
    setDraft(d);
    setDirty(false);
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
      id: 'version-workspace',
      label: t('代码工作区', 'Code workspace'),
      value: workspace,
      type: 'text',
      set: setWorkspace,
    },
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
        enabled: !busy && !codeBusy,
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

  return (
    <div className="page-stack">
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
      <h1>{t('模型版本', 'Model versions')}</h1>
      {error && <Notice tone="error">{error}</Notice>}
      <div className="form-grid">
        <Card title={t('版本树与修订', 'Version tree and revisions')}>
          <nav aria-label={t('版本树', 'Version tree')}>
            {state.versions.map((v) => (
              <div
                key={`${v.version.id}@${v.version.revision}`}
                style={{ paddingInlineStart: versionDepth(v, state.versions) * 14 }}
              >
                <Button
                  disabled={busy || codeBusy || dirty}
                  variant={selected?.fingerprint === v.fingerprint ? 'primary' : 'ghost'}
                  onClick={() => run(() => select(v))}
                >
                  {v.name} · r{v.version.revision}{' '}
                  {v.version.id === 'official' ? t('官方', 'Official') : ''}
                </Button>
              </div>
            ))}
          </nav>
          <h3>{t('草稿', 'Drafts')}</h3>
          {drafts.map((d) => (
            <Button
              key={d.id}
              disabled={busy || codeBusy || dirty}
              onClick={() => run(() => loadDraft(d.id))}
            >
              {d.name} · {d.id} · g{d.generation}
            </Button>
          ))}
        </Card>
        <Card title={t('派生新版本', 'Derive a new version')}>
          <Field label={t('名称', 'Name')}>
            <Input
              value={name}
              disabled={busy || codeBusy}
              onChange={(e) => setName(e.target.value)}
            />
          </Field>
          <Field label={t('目的与说明', 'Purpose and notes')}>
            <Input
              value={note}
              disabled={busy || codeBusy}
              onChange={(e) => setNote(e.target.value)}
            />
          </Field>
          <Field label={t('代码工作区（可选）', 'Code workspace (optional)')}>
            <Input
              value={workspace}
              disabled={busy || codeBusy}
              onChange={(e) => setWorkspace(e.target.value)}
            />
          </Field>
          <p>
            {t('来源', 'Parent')}: {selected?.name} r{selected?.version.revision}
          </p>
          <Button
            disabled={!guideActions[0].enabled}
            variant="primary"
            onClick={() => run(deriveVersion)}
          >
            {t('派生新版本', 'Derive version')}
          </Button>
        </Card>
      </div>
      {shown && (
        <>
          <Card
            title={`${shown.name} · ${draft ? t('草稿', 'Draft') : `r${selected?.version.revision}`}`}
          >
            <p>{shown.note}</p>
            <p>
              {shown.code.kind} · {shown.code.commit ?? shown.code.source_sha256}
            </p>
            {!editable && (
              <Notice>
                {t(
                  '冻结版本只读。派生新版本后编辑。',
                  'Frozen revisions are read-only. Derive a version to edit.',
                )}
              </Notice>
            )}
            {editable && (
              <>
                <div className="button-row">
                  <Button disabled={busy || codeBusy || !dirty} onClick={() => run(save)}>
                    {t('保存草稿', 'Save draft')}
                  </Button>
                  <Button disabled={busy || codeBusy} variant="primary" onClick={() => run(freeze)}>
                    {t('冻结修订', 'Freeze revision')}
                  </Button>
                  <Button
                    disabled={busy || codeBusy || !dirty}
                    onClick={() => {
                      if (window.confirm(t('放弃尚未保存的修改？', 'Discard unsaved edits?')))
                        run(discardLocal);
                    }}
                  >
                    {t('放弃本地修改', 'Discard local edits')}
                  </Button>
                  <span>{dirty ? t('未保存', 'Unsaved') : `generation ${draft.generation}`}</span>
                </div>
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
              </>
            )}
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
                      <Field key={part} label={part}>
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
          <Card
            title={t('按过程配置后续选项', 'Configure process options')}
            description={t(
              '开启父过程后展开子项；过程文件与 PFT 参数分别配置。',
              'Parent processes reveal their options; process files and PFT parameters keep their own scopes.',
            )}
          >
            <ConfigurationOptions
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
          <Card title={t('参数', 'Parameters')}>
            {editable && (
              <>
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
                {descriptor && (
                  <>
                    <p>{state.language === 'en' ? descriptor.doc_en : descriptor.doc_zh}</p>
                    <p>
                      {descriptor.unit ?? '—'} · {descriptor.activation.join(', ')} ·{' '}
                      {descriptor.validation.min ?? '−∞'} … {descriptor.validation.max ?? '∞'}
                    </p>
                    {['land-cover-class', 'pft-type', 'pc-pft-component'].includes(
                      descriptor.scope,
                    ) && (
                      <Field label={t('地类 / PFT 索引', 'Land class / PFT index')}>
                        <Select value={index} onChange={(e) => setIndex(Number(e.target.value))}>
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
                    {descriptor.storage === 'process-parameter-file' && (
                      <Field label={t('冻结过程文件', 'Frozen process file')}>
                        <Select value={process} onChange={(e) => setProcess(e.target.value)}>
                          <option value="">—</option>
                          {draft.overlay.process_files.map((a) => (
                            <option key={a.relative_path}>{a.relative_path}</option>
                          ))}
                        </Select>
                      </Field>
                    )}
                  </>
                )}
                <Field label={t('参数值（Namelist 字面值）', 'Parameter value (namelist literal)')}>
                  <Input
                    value={value}
                    disabled={busy || codeBusy}
                    onChange={(e) => setValue(e.target.value)}
                  />
                </Field>
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
                  {t('应用参数到草稿', 'Apply parameter to draft')}
                </Button>
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
                      const result = await invokeJson<{ draft: ModelDraft; assetRoot: string }>(
                        'model_import_process',
                        {
                          id: draft.id,
                          file,
                          relativePath: processPath,
                          expectedGeneration: draft.generation,
                        },
                      );
                      setDraft(result.draft);
                      setAssetRoot(result.assetRoot);
                      setDirty(false);
                    })
                  }
                >
                  {t('导入过程文件', 'Import process file')}
                </Button>
                <Button disabled={busy || codeBusy} onClick={() => run(importFile)}>
                  {t('导入参数 JSON', 'Import parameters JSON')}
                </Button>
              </>
            )}
            <Button
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
            {shown.overlay.parameters.map((r) => (
              <div key={parameterKey(r)}>
                <code>
                  {r.raw_key} = {r.value} {r.unit ?? ''}
                </code>
                <p>{JSON.stringify(r.scope_instance)}</p>
                {editable && (
                  <Button
                    disabled={busy || codeBusy}
                    onClick={() =>
                      edit({
                        ...draft!,
                        overlay: {
                          ...draft!.overlay,
                          parameters: draft!.overlay.parameters.filter(
                            (x) => parameterKey(x) !== parameterKey(r),
                          ),
                        },
                      })
                    }
                  >
                    {t('移除覆盖', 'Remove override')}
                  </Button>
                )}
              </div>
            ))}
          </Card>
          <Card title={t('物理方案', 'Physics')}>
            {editable && (
              <>
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
                {modelField && (
                  <p>
                    {modelField.doc} · {modelField.kind} · {modelField.values.join(', ')}
                  </p>
                )}
                <Field label={t('值', 'Value')}>
                  <Input value={physicsValue} onChange={(e) => setPhysicsValue(e.target.value)} />
                </Field>
                <Button
                  disabled={busy || codeBusy || !modelField || !physicsValue}
                  onClick={() => {
                    if (
                      draft.overlay.parameters.some(
                        (r) => r.raw_key.toLowerCase() === physics.toLowerCase(),
                      )
                    ) {
                      setError(t('请先重置同名参数覆盖', 'Reset the parameter override first'));
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
                  {t('应用物理设置', 'Apply physics setting')}
                </Button>
              </>
            )}
            {Object.entries(shown.overlay.physics).map(([key, v]) => (
              <div key={key}>
                <code>
                  {key} = {v}
                </code>
                {editable && (
                  <Button
                    disabled={busy || codeBusy}
                    onClick={() => {
                      const next = { ...draft!.overlay.physics };
                      delete next[key];
                      edit({ ...draft!, overlay: { ...draft!.overlay, physics: next } });
                    }}
                  >
                    {t('重置', 'Reset')}
                  </Button>
                )}
              </div>
            ))}
          </Card>
          <Card title={t('AI 网络', 'AI network')}>
            <Notice>
              {t(
                '网络训练与安装使用研究页真实流程，网络仅支持 Rust 引擎。',
                'Training and installation use the real research workflow. Networks require the Rust engine.',
              )}
            </Notice>
            <pre>{JSON.stringify(shown.overlay.hybrid, null, 2)}</pre>
            {draft && (
              <p>
                {t(
                  '将保存当前草稿并冻结新修订，随后使用该修订开展研究。',
                  'The current draft will be saved and frozen, then used for research.',
                )}
              </p>
            )}
            <Button disabled={busy || codeBusy} onClick={() => run(openResearch)}>
              {draft
                ? t('保存、冻结并进入研究', 'Save, freeze and open research')
                : t('打开研究与网络', 'Open research and networks')}
            </Button>
          </Card>
          <CodeWorkspace
            disabled={busy || codeBusy || dirty}
            key={shown.code.workspace ?? 'official'}
            name={shown.code.workspace}
            onChanged={refreshCode}
            onPendingChange={codePending}
          />
          <Card title={t('实际引用历史', 'Actual usage history')}>
            {references.map((r) => (
              <p key={r.operation_id}>
                {r.kind} · {r.status} · {r.case_path ?? r.host} · {r.revision ?? '—'} ·{' '}
                {r.time_ms == null ? '—' : new Date(r.time_ms).toLocaleString()}
              </p>
            ))}
            <Button onClick={() => navigate('history')}>{t('完整历史', 'Full history')}</Button>
          </Card>
        </>
      )}
    </div>
  );
}

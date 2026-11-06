import { useRef, useState } from 'react';
import { invoke } from '../../lib/ipc';
import { askConfirm } from '../../lib/confirm';
import { useApp, useNavigationGuard } from '../../lib/app-state';
import { usePageContract, type UiField } from '../../lib/guide';
import { Button, Card, Input, Select, Field, Advanced, Notice } from '../../components/ui';
import { ActionButton } from '../../components/ActionButton';
import {
  EMPTY_PREP_FORM,
  type PrepDraft,
  clearPrepDraft,
  prepDraftHasInput,
  readPrepDraft,
  writePrepDraft,
} from './preparation-draft';
import {
  requiredNumber,
  preparedDestination,
  casePreparationOptions,
  preparationSiteModes,
  prepDateValid,
  prepDateKey,
  preparedSiteDestination,
  preparedSiteNameValid,
  preparedDatasetDestinations,
} from './case-model';
interface Slot {
  index: number;
  meaning: string;
  optional: boolean;
  guessed?: string;
  column?: string;
  units?: string;
  wants: string;
  name?: string;
  also_add: string[];
}
interface Imported {
  site: string;
  safe_site: string;
  staged_path: string;
  final_path: string;
  latitude: number;
  longitude: number;
  landtype: number | null;
  timezone_offset_hours: number | null;
}
interface Probe {
  slots?: Slot[];
  suggest_dst?: string;
  time_column?: string;
  site_column?: string;
  latitude_column?: string;
  longitude_column?: string;
  landtype_column?: string;
  utc_offset_column?: string;
  columns?: { name: string }[];
  variables?: (
    | string
    | {
        name: string;
        label: string;
        units?: string;
        column?: string;
        qc_column?: string;
        requires_qc: boolean;
      }
  )[];
}
let currentLang = 'zh';
const t = (zh: string, enText: string) => (currentLang === 'en' ? enText : zh);

/**
 * 数据准备结果的结构化摘要（U-05）。
 *
 * 这一页的后端返回值不止一种形状：转换强迫场/下载 ERA5 返回输出路径字符串；制作站点返回带
 * readiness 的报告；修复缺测返回带 missing/unresolved 的报告；表格拆分与观测转换返回数组；
 * 「检查输入」还会把探测结果当结果。原来一律 JSON.stringify，用户看不出成功还是缺数据。
 *
 * 这里只读后端**真实存在**的字段：字段没有就返回 null，界面显示"后端没有提供"而不是编一个 0。
 * 做成纯函数是为了能直接单测（本文件被 page-results.test.ts 按源码抽取函数，不能随便加自由标识符）。
 */
type PreparationResultSummary = {
  /** complete：产物已写出；partial：写出了但还需处理；checked：只是检查、没有写产物；unknown：结构不认识的完成结果。 */
  state: 'complete' | 'partial' | 'checked' | 'unknown';
  kind:
    | 'forcing'
    | 'era5'
    | 'site'
    | 'observations'
    | 'table-split'
    | 'gap-report'
    | 'probe'
    | 'unknown';
  /** 后端明确给出的输出路径（没有就是 null；页面可以另用自己记录的成功写入路径兜底）。 */
  output: string | null;
  /** GapReport.missing：后端报的缺测总数。 */
  missing: number | null;
  /** GapReport.unresolved：处理后仍未解决的缺测数。 */
  unresolved: number | null;
  /** SiteReport.needs_external.length：运行时仍需 rawdata 提供的字段数。 */
  needsExternal: number | null;
  /** 表格拆分/观测转换里的条目数（后端数组长度）。 */
  sites: number | null;
  /** 观测转换的站数（ConvertedObservationSite 条目数）。 */
  converted: number | null;
  /** 后端原样给出的观测警告。 */
  warnings: string[];
  /** SiteReport.readiness 的三档。 */
  readiness: 'self-contained' | 'needs-rawdata' | 'blocked' | null;
  /** 站点字段来源计数（SiteReport.from_site/from_raster/from_default）；旧后端没有就不显示。 */
  fromSite?: number;
  fromRaster?: number;
  fromDefault?: number;
  /** 走 CoLM 查表的字段数（from_lookup）。 */
  fromLookup?: number;
  /** SiteReport.site_kind / mode（站点结果页显示，D-04 残留）。 */
  siteKind?: string;
  siteMode?: string;
  /** 输入检查里后端给出的槽位数（Probe.slots）。 */
  slots: number | null;
  /** 输入检查里后端给出的变量/列字段数。 */
  fields: number | null;
};

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

const finiteNumber = (value: unknown): number | null =>
  typeof value === 'number' && Number.isFinite(value) ? value : null;

const stringField = (record: Record<string, unknown>, key: string): string | null => {
  const value = record[key];
  return typeof value === 'string' && value.length > 0 ? value : null;
};

function emptySummary(
  kind: PreparationResultSummary['kind'],
  state: PreparationResultSummary['state'],
): PreparationResultSummary {
  return {
    state,
    kind,
    output: null,
    missing: null,
    unresolved: null,
    needsExternal: null,
    sites: null,
    converted: null,
    warnings: [],
    readiness: null,
    slots: null,
    fields: null,
  };
}

/**
 * `gapCheckOnly`：结果是不是只来自「检查缺测」（还没写产物）。缺测报告与修复报告的 JSON 结构相同，
 * 只有页面知道刚点的是哪个按钮；不能从字段形状反推，否则会把一次检查说成"已修复"。
 */
function summarizePreparationResult(
  result: unknown,
  mode: string,
  gapCheckOnly = false,
): PreparationResultSummary | null {
  if (result == null) return null;
  if (typeof result === 'string') {
    if (!result) return null;
    // 生成强迫场与下载 ERA5 都只返回输出路径；两者靠页面当前任务区分。
    return { ...emptySummary(mode === 'era5' ? 'era5' : 'forcing', 'complete'), output: result };
  }
  if (Array.isArray(result)) {
    const items = result.filter(isRecord);
    if (items.some((item) => stringField(item, 'staged_path'))) {
      // 表格拆分只写暂存文件，最终强迫场还没落盘，所以不把 staged_path 当"输出路径"。
      const staged = items.filter((item) => stringField(item, 'staged_path'));
      return { ...emptySummary('table-split', 'partial'), sites: staged.length };
    }
    const converted = items.filter((item) => stringField(item, 'path'));
    const warnings = converted.flatMap((item) =>
      Array.isArray(item.warnings)
        ? item.warnings.filter((warning): warning is string => typeof warning === 'string')
        : [],
    );
    return {
      ...emptySummary('observations', warnings.length ? 'partial' : 'complete'),
      converted: items.length,
      output: converted.length === 1 ? stringField(converted[0], 'path') : null,
      warnings,
    };
  }
  if (!isRecord(result)) return emptySummary('unknown', 'unknown');
  // 制作站点：path + readiness；比缺测报告先判断（站点报告没有 missing/unresolved）。
  if (stringField(result, 'path') && typeof result.readiness === 'string') {
    const readiness =
      result.readiness === 'self_contained'
        ? 'self-contained'
        : result.readiness === 'ready_with_rawdata'
          ? 'needs-rawdata'
          : result.readiness === 'blocked'
            ? 'blocked'
            : null;
    return {
      ...emptySummary('site', readiness === 'self-contained' ? 'complete' : 'partial'),
      output: stringField(result, 'path'),
      needsExternal: Array.isArray(result.needs_external) ? result.needs_external.length : null,
      readiness,
      ...(Array.isArray(result.from_site) ? { fromSite: result.from_site.length } : {}),
      ...(Array.isArray(result.from_raster) ? { fromRaster: result.from_raster.length } : {}),
      ...(Array.isArray(result.from_default) ? { fromDefault: result.from_default.length } : {}),
      ...(Array.isArray(result.from_lookup) ? { fromLookup: result.from_lookup.length } : {}),
      ...(stringField(result, 'site_kind') ? { siteKind: stringField(result, 'site_kind')! } : {}),
      ...(stringField(result, 'mode') ? { siteMode: stringField(result, 'mode')! } : {}),
      ...(Array.isArray(result.warnings)
        ? {
            warnings: result.warnings.filter(
              (warning): warning is string => typeof warning === 'string',
            ),
          }
        : {}),
    };
  }
  // 缺测报告（检查缺测与修复缺测同构）。
  if (
    typeof result.missing === 'number' ||
    typeof result.unresolved === 'number' ||
    typeof result.needs_era5 === 'boolean'
  ) {
    const missing = finiteNumber(result.missing);
    const unresolved = finiteNumber(result.unresolved);
    const state = gapCheckOnly
      ? 'checked'
      : unresolved === 0 || (unresolved === null && missing === 0)
        ? 'complete'
        : 'partial';
    return {
      ...emptySummary('gap-report', state),
      output: stringField(result, 'path'),
      missing,
      unresolved,
    };
  }
  // 输入检查（Probe）：slots/variables/columns 是探测结果，没有写任何产物。
  if (
    Array.isArray(result.slots) ||
    Array.isArray(result.variables) ||
    Array.isArray(result.columns)
  ) {
    const slots = Array.isArray(result.slots) ? result.slots.length : null;
    const variables = Array.isArray(result.variables) ? result.variables.length : 0;
    const columns = Array.isArray(result.columns) ? result.columns.length : 0;
    return {
      ...emptySummary('probe', 'checked'),
      slots,
      fields:
        Array.isArray(result.variables) || Array.isArray(result.columns)
          ? variables + columns
          : null,
    };
  }
  // 结构不认识的完成结果：只显示后端真的给了一个 path 的情况，其余一律不猜。
  return { ...emptySummary('unknown', 'unknown'), output: stringField(result, 'path') };
}

const kindText = (
  kind: PreparationResultSummary['kind'],
  t: (zh: string, en: string) => string,
) => {
  switch (kind) {
    case 'forcing':
      return t('强迫场文件', 'Forcing file');
    case 'era5':
      return t('ERA5-Land 文件', 'ERA5-Land file');
    case 'site':
      return t('站点文件', 'Site file');
    case 'observations':
      return t('观测转换', 'Converted observations');
    case 'table-split':
      return t('表格拆分', 'Split table');
    case 'gap-report':
      return t('缺测报告', 'Gap report');
    case 'probe':
      return t('输入检查', 'Input check');
    case 'unknown':
      return t('处理结果', 'Processing result');
  }
};

const stateText = (
  state: PreparationResultSummary['state'],
  t: (zh: string, en: string) => string,
) => {
  switch (state) {
    case 'complete':
      return t('成功', 'Completed');
    case 'partial':
      return t('部分完成', 'Partially completed');
    case 'checked':
      return t('已检查，未写产物', 'Checked, nothing written');
    case 'unknown':
      return t('已完成', 'Finished');
  }
};

const stateTone = (state: PreparationResultSummary['state']): 'success' | 'warning' | 'info' =>
  state === 'complete' ? 'success' : state === 'partial' ? 'warning' : 'info';

/**
 * 结果区的展示组件（从页面里拆出来，静态渲染就能验证）。只渲染摘要与后端原始字段；不认识的形状
 * 只给"已收到结果"和原始 JSON，不虚构状态、缺测数或下一步。
 */
function PreparationResultView({
  result,
  mode,
  gapCheckOnly = false,
  en,
  hasSelectedCase = false,
  /** 页面自己记录的成功写入路径（缺测修复的 GapReport 不带路径，但后端成功写到了用户给的 dst）。 */
  fallbackOutput = null,
  onOpenRuns,
}: {
  result: unknown;
  mode: string;
  gapCheckOnly?: boolean;
  en: boolean;
  hasSelectedCase?: boolean;
  fallbackOutput?: string | null;
  onOpenRuns?: () => void;
}) {
  const t = (zh: string, enText: string) => (en ? enText : zh);
  const summary = summarizePreparationResult(result, mode, gapCheckOnly);
  if (!summary) return null;
  // 兜底路径只对"后端成功写了文件却没回路径"的缺测报告有意义；别的形状不能拿上一次生成的结果贴到
  // 这一次的结果上（例如先转 netcdf、再拆表格，表格只是暂存，没有最终输出）。
  const output = summary.output ?? (summary.kind === 'gap-report' ? fallbackOutput : null);
  const tone = stateTone(summary.state);
  const readiness =
    summary.readiness === 'self-contained'
      ? t('自带完整字段，可直接运行', 'Self-contained; ready to run')
      : summary.readiness === 'needs-rawdata'
        ? t('运行时仍需要 rawdata 提供部分字段', 'Some fields still come from rawdata at run time')
        : summary.readiness === 'blocked'
          ? t('缺 rawdata，暂时不能运行', 'Blocked until rawdata is available')
          : null;
  const steps: string[] = [];
  if (summary.kind === 'table-split')
    steps.push(
      t(
        '下方逐站点点「修复并完成」；需要站点文件时用「制作站点」。',
        'Finalize each site below; choose “Create site” if site files are needed.',
      ),
    );
  if (summary.kind === 'gap-report') {
    if (summary.state === 'checked')
      steps.push(
        t(
          '确认缺测处理方式后点「生成数据」写入产物；需要补齐时先在高级设置里选 ERA5 辅助文件。',
          'Confirm how gaps should be handled, then click “Generate data”. Choose an ERA5 auxiliary file under advanced settings if gaps need filling.',
        ),
      );
    else if (summary.state === 'partial')
      steps.push(
        t(
          '仍有缺测未解决：提供 ERA5 辅助文件或调整设置后重新生成；当前产物不完整。',
          'Some gaps remain: provide the ERA5 auxiliary file or adjust settings and generate again; the current output is incomplete.',
        ),
      );
    else if (summary.output)
      // 表格逐站"修复并完成"把最终路径和报告合在一起返回；这是这一站落盘的确认。
      steps.push(
        t(
          '这一站的最终强迫场已写入；其余站点按下方列表继续。',
          'This site’s final forcing was written; continue with the remaining sites listed below.',
        ),
      );
    else
      steps.push(
        t(
          '缺测已处理完，可在「项目」里打开算例运行，或点下方「用于新算例」。',
          'Gaps are handled; open a case under Project to run it, or use “Use for new case” below.',
        ),
      );
  }
  if (
    summary.kind === 'site' &&
    summary.fromSite !== undefined &&
    summary.fromDefault !== undefined
  )
    steps.push(
      t(
        `字段来源：站点文件 ${summary.fromSite} 项、rawdata 栅格 ${summary.fromRaster ?? 0} 项、标称默认 ${summary.fromDefault} 项${summary.fromLookup ? `、CoLM 查表 ${summary.fromLookup} 项` : ''}。`,
        `Field provenance: ${summary.fromSite} from the site file, ${summary.fromRaster ?? 0} from rawdata rasters, ${summary.fromDefault} nominal defaults${summary.fromLookup ? `, ${summary.fromLookup} from CoLM lookup` : ''}.`,
      ),
    );
  if (summary.kind === 'site' && summary.state === 'partial')
    steps.push(
      t(
        '运行前还需要 rawdata 目录提供缺的字段。',
        'Running still needs the missing fields from a rawdata directory.',
      ),
    );
  if (summary.kind === 'site' && summary.state === 'complete')
    steps.push(
      t(
        '站点文件已生成，可点下方「用于新算例」带着它继续。新建模拟扫描项目时按 PLUMBER2 的 `<站名>_site.nc` 或 Urban-PLUMBER 的 `<站名>_site_v1.nc` 识别站点文件；强迫场请用同一词干的 `<站名>_Met.nc` / `<站名>_metforcing_v1.nc` 放进 Forcing/，扫描时才能配对。',
        'The site file is ready; use “Use for new case” below to continue with it. New-case scanning recognizes site files named `<site>_site.nc` (PLUMBER2) or `<site>_site_v1.nc` (Urban-PLUMBER); put forcing with the same stem (`<site>_Met.nc` / `<site>_metforcing_v1.nc`) under Forcing/ so the scan can pair them.',
      ),
    );
  if (summary.kind === 'forcing')
    steps.push(
      t(
        '可在「项目」里打开算例运行，或点下方「用于新算例」带着这些文件继续。',
        'Open a case under Project to run it, or use “Use for new case” below to continue with these files.',
      ),
    );
  if (summary.kind === 'era5')
    steps.push(
      t(
        '已将下载目录设为 ERA5 辅助数据。切换到「修复缺测」或表格逐站修复后，再将完成的强迫场用于新算例；此目录不能直接作为站点强迫场。',
        'The download directory is now selected as ERA5 auxiliary data. Repair missing forcing data or finalize table stations before using the completed forcing for a new case; this directory is not a site forcing file.',
      ),
    );
  if (summary.kind === 'observations')
    steps.push(
      t(
        '这些观测文件可用于比较与评估页的参考数据。',
        'These observation files can be used as reference data on the Compare page.',
      ),
    );
  if (summary.kind === 'probe')
    steps.push(
      t(
        '确认变量与单位后点「生成数据」。',
        'Confirm the variables and units, then click “Generate data”.',
      ),
    );
  if (summary.kind === 'unknown')
    steps.push(
      t(
        '后端没有提供可结构化的状态，完整数据见下方原始 JSON。',
        'The backend returned no structured status; see the raw JSON below for the complete data.',
      ),
    );
  if (
    !hasSelectedCase &&
    summary.kind !== 'era5' &&
    (summary.kind === 'forcing' || summary.state === 'complete')
  )
    steps.push(
      t(
        '还没有打开算例：先在「项目」里打开一个算例，再点「检查并运行」。',
        'No case is open yet: open one under Project, then use “Review and run”.',
      ),
    );
  return (
    <Card title={t('处理结果', 'Processing result')}>
      <Notice tone={tone}>
        {stateText(summary.state, t)} · {kindText(summary.kind, t)}
      </Notice>
      {output && (
        <p>
          <strong>{t('输出路径', 'Output path')}</strong>
          <br />
          <code>{output}</code>
        </p>
      )}
      {summary.missing !== null && (
        <p>
          {t(`缺测 ${summary.missing} 个`, `${summary.missing} missing value(s)`)}
          {summary.unresolved !== null &&
            summary.unresolved > 0 &&
            t(
              `；处理后仍有 ${summary.unresolved} 个未解决`,
              `; ${summary.unresolved} still unresolved after processing`,
            )}
        </p>
      )}
      {summary.needsExternal !== null && summary.needsExternal > 0 && (
        <p>
          {t(
            `运行时仍需外部 rawdata 提供 ${summary.needsExternal} 个字段`,
            `${summary.needsExternal} field(s) still need external rawdata at run time`,
          )}
        </p>
      )}
      {readiness && <p>{readiness}</p>}
      {(summary.siteKind || summary.siteMode) && (
        <p>
          {t('站点类型 / 生成模式：', 'Site kind / mode: ')}
          {summary.siteKind ?? '—'} · {summary.siteMode ?? '—'}
        </p>
      )}
      {summary.sites !== null && (
        <p>{t(`拆分站点 ${summary.sites} 个`, `${summary.sites} site(s) split`)}</p>
      )}
      {summary.converted !== null && (
        <p>{t(`已转换站点 ${summary.converted} 个`, `${summary.converted} site(s) converted`)}</p>
      )}
      {summary.slots !== null && (
        <p>
          {t(`识别到 ${summary.slots} 个强迫场槽位`, `${summary.slots} forcing slot(s) detected`)}
          {summary.fields !== null &&
            t(`、${summary.fields} 个字段`, `, ${summary.fields} column(s)`)}
        </p>
      )}
      {summary.warnings.length > 0 && (
        <Notice tone="warning">{summary.warnings.join(en ? '; ' : '；')}</Notice>
      )}
      {steps.map((step) => (
        <p className="muted" key={step}>
          {t('下一步：', 'Next: ')}
          {step}
        </p>
      ))}
      {hasSelectedCase && onOpenRuns && summary.kind !== 'era5' && (
        <div className="actions">
          <Button onClick={onOpenRuns}>{t('去「检查并运行」', 'Go to “Review and run”')}</Button>
        </div>
      )}
      {/* 原始 JSON 保留完整数据，默认折叠；结构化摘要丢掉的字段随时能查。 */}
      <details>
        <summary>{t('原始结果（JSON）', 'Raw result (JSON)')}</summary>
        <pre>{JSON.stringify(result, null, 2)}</pre>
      </details>
    </Card>
  );
}
export default function PreparationPage() {
  const { state, notify, navigate, update, registerCloseGuard } = useApp();
  currentLang = state.language;
  const initialDraft = useRef<PrepDraft | null | undefined>(undefined);
  if (initialDraft.current === undefined) initialDraft.current = readPrepDraft();
  const restoredDraft = initialDraft.current;
  const [imported, setImported] = useState<Imported[]>([]),
    [mode, setMode] = useState(initialDraft.current?.mode ?? 'netcdf'),
    [form, setForm] = useState<Record<string, string>>({
      ...EMPTY_PREP_FORM,
      ...(initialDraft.current?.form ?? {}),
    }),
    [probe, setProbe] = useState<Probe | null>(null),
    [slots, setSlots] = useState<Slot[]>([]),
    [busy, setBusy] = useState(false),
    [advanced, setAdvanced] = useState(state.expert),
    [result, setResult] = useState<unknown>(null),
    // 缺测检查与修复的报告 JSON 同构，只有这里知道刚才点的是哪个按钮：检查不该被说成"已修复"。
    [gapCheckOnly, setGapCheckOnly] = useState(false),
    [error, setError] = useState(''),
    [declined, setDeclined] = useState(''),
    // 这一页做出来、能直接给新算例用的文件：站点文件、转换或修补好的强迫场。
    [produced, setProduced] = useState<{ site?: string; met?: string }>({}),
    // 安装到哪个数据集根（Sitedata/Forcing 的父目录）；记住上次选择（D-06 残留）。
    [datasetRoot, setDatasetRoot] = useState(() => {
      try {
        return globalThis.localStorage?.getItem('colm-prep-dataset-root') ?? '';
      } catch {
        return '';
      }
    }),
    [datasetScan, setDatasetScan] = useState<{
      root: string;
      count: number;
      names: string[];
    } | null>(null),
    // 离页保护的两个来源：用户是否改过本页输入，以及这些输入是否已经被一次成功的
    // "生成数据"消费掉。检查输入只把 result 写回、不算消费——否则改完 src 点一下检查，
    // 还没生成产物离页保护就解除了（GUI 复核 D-05）。
    // 恢复出来的输入仍算"未生成"：刷新后不能悄悄绕过离页保护（D-05）。
    [edited, setEdited] = useState(!!restoredDraft && prepDraftHasInput(restoredDraft)),
    [inputsConsumed, setInputsConsumed] = useState(false);
  const lock = useRef(false);
  // 导航守卫的同步镜像：确认"丢弃并继续"后要立刻允许 navigate，不能等 React 重新渲染
  // （setEdited/setInputsConsumed 是异步的，守卫闭包会读到上一轮的 dirty 状态，本次验收 P2）。
  const dirtyNow = useRef(false);
  function rememberForm(next: Record<string, string>) {
    writePrepDraft({ mode, form: next });
  }
  // 上次"检查"自动推出的输出路径。换了源文件就作废：原来选 A 检查后再换成 B，输出路径还是
  // A 的推荐名，B 的数据被写进 A 的文件（GUI 审查 2026-10-22，P1-1）。用户自己改过的路径不动。
  const autoDst = useRef('');
  const observationVariables = (probe?.variables ?? []).filter((v) => typeof v !== 'string');
  const sourceNames = [
    ...(probe?.variables ?? []).filter((v): v is string => typeof v === 'string'),
    ...(probe?.columns ?? []).map((v) => v.name),
  ];
  const preparation = casePreparationOptions(state.selectedCase?.profile, state.newCaseSeed);
  // 准备页的源文件/映射/探测结果都只活在组件 state 里；一次成功的生成之前切页会全部丢失
  // （GUI 审查 D-05）。生成出产物后允许离开：用户的目的已经达到，继续拦反而挡住“用于新算例”。
  // **不能用 result 是否为空判断完成**：检查输入也会写 result（见 inputsConsumed 注释）。
  const preparationDirty = edited && !inputsConsumed;
  dirtyNow.current = preparationDirty;
  useNavigationGuard(() =>
    dirtyNow.current
      ? t(
          '数据准备页还有未生成的输入或映射。请先点「放弃未生成的输入」或完成一次生成，再切换页面。',
          'This page still has input or mappings that have not been generated. Click “Discard unfinished inputs” or generate once before leaving.',
        )
      : null,
  );
  const set = (k: string, v: string) => {
    const changed = form[k] !== v;
    if (changed) {
      // 字段一改，上一次的成功结果就过时了：先清掉，别让旧的"成功"挂在页面上（U-05）。
      setResult(null);
      // 普通字段（纬度/输出路径/rawdata…）也会改变产物内容：旧产物不能再拿去建例（GUI 复核 D-12）。
      setProduced({});
      // 表格任务暂存列表只由"拆分/转换"输入决定；ERA5 辅助文件和缺测修复参数只影响逐站
      // "修复并完成"那一步，输它们不能把已拆分列表清掉（否则逐站修复入口直接消失，
      // GUI 审查 F-D-01 的回归就是这么来的）。
      if (!['era5', 'short_gap', 'min_overlap'].includes(k)) setImported([]);
      setDeclined('');
      setEdited(true);
      setInputsConsumed(false);
    }
    if (k === 'src' && changed) {
      setProbe(null);
      setSlots([]);
    }
    setForm((f) => {
      const next =
        k !== 'src' || v === f.src
          ? { ...f, [k]: v }
          : { ...f, src: v, dst: f.dst === autoDst.current ? '' : f.dst };
      rememberForm(next);
      return next;
    });
  };
  // 校验错误给用户看的字段名，不把内部 key（src/dst/lon…）泄漏到界面；经纬度和偏移在这里定范围（GUI 审查 D-09）。
  const numericLabels: Record<string, string> = {
    lon: t('经度', 'Longitude'),
    lat: t('纬度', 'Latitude'),
    landtype: t('地类编号', 'Land class'),
    utc_offset: t('UTC 偏移（小时）', 'UTC offset (hours)'),
    step_seconds: t('时间步长（秒）', 'Time step (seconds)'),
    short_gap: t('短缺测最大步数', 'Max short-gap steps'),
    min_overlap: t('最少重叠步数', 'Min overlap steps'),
    height_v: t('风速观测高度（m）', 'Wind measurement height (m)'),
    height_t: t('温度观测高度（m）', 'Temperature measurement height (m)'),
    height_q: t('湿度观测高度（m）', 'Humidity measurement height (m)'),
  };
  const num = (k: string) => {
    const label = numericLabels[k] ?? k;
    const value = requiredNumber(form[k], label, t);
    const range = (min: number, max: number) => {
      if (value < min || value > max)
        throw new Error(
          t(
            `${label}须在 ${min} 到 ${max} 之间（当前 ${value}）`,
            `${label} must be between ${min} and ${max} (currently ${value})`,
          ),
        );
    };
    if (k === 'lon') range(-180, 180);
    if (k === 'lat') range(-90, 90);
    // CLI gapfill.rs 只接受 -12..14；前端不能放行 -14（D-09）。
    if (k === 'utc_offset') range(-12, 14);
    if (k === 'landtype' || k === 'step_seconds') {
      if (!Number.isInteger(value))
        throw new Error(
          t(
            `${label}必须是整数（当前 ${value}）`,
            `${label} must be a whole number (currently ${value})`,
          ),
        );
    }
    if (k === 'step_seconds') range(1, 86_400);
    if (k === 'height_v' || k === 'height_t' || k === 'height_q') range(0.01, 300);
    if (k === 'short_gap' || k === 'min_overlap') {
      if (!Number.isInteger(value))
        throw new Error(
          t(
            `${label}必须是整数（当前 ${value}）`,
            `${label} must be a whole number (currently ${value})`,
          ),
        );
      range(0, 1_000_000);
    }
    return value;
  };
  const optional = (k: string) => (form[k].trim() ? num(k) : null);
  async function work(fn: () => Promise<unknown>) {
    if (lock.current) return;
    lock.current = true;
    const releaseCloseGuard = registerCloseGuard(() =>
      t(
        '数据准备正在执行，关闭会中断任务。',
        'Data preparation is running; closing interrupts it.',
      ),
    );
    setBusy(true);
    // 新操作一开始就清掉旧结果：失败时页面上不能还挂着上一次的成功卡片。
    setResult(null);
    setError('');
    setDeclined('');
    try {
      const value = await fn();
      setResult(value);
      return value;
    } catch (e) {
      setError(String(e));
      // 用户选择"不覆盖"是一个决定，不是失败：页面内说明即可，不推全局错误横幅。否则取消一次覆盖
      // 会在其它页面上挂着一条红色 "Error: 没有覆盖已有的输出文件。"（2026-10-22 窗口实测）。
      // 用属性标记而不是自定义错误类：本文件被测试按源码抽取 function work 执行，catch 里引入新的
      // 自由标识符会让抽取出来的函数在测试上下文里 ReferenceError。
      // 用户按了"停止"也是决定：后端在进程被停止按钮或关窗终止时给出这句固定文字。
      const stopped = String(e).includes('已停止：数据准备被中断');
      const flagged =
        !!e &&
        typeof e === 'object' &&
        ((e as { userDeclined?: boolean }).userDeclined === true ||
          // 观测转换的覆盖取消抛的是 CancelledError（按 name 标记，不引入新的自由标识符/类：
          // 本函数会被测试按源码抽取执行）；取消是决定，不该在其它页面挂红色错误。
          (e as { name?: string }).name === 'CancelledError');
      const declined = stopped || flagged;
      if (declined) {
        // 页面内也按提示显示，不是红色的 "Error: …"。
        setError('');
        setDeclined(
          stopped
            ? t(
                '已停止。输出文件可能不完整；重新生成前请删掉它或换一个输出路径。',
                'Stopped. The output file may be incomplete; delete it or choose another path before generating again.',
              )
            : (e as Error).message,
        );
      } else notify(String(e), true);
      throw e;
    } finally {
      releaseCloseGuard();
      setBusy(false);
      lock.current = false;
    }
  }
  function changeMode(v: string) {
    if (v === mode) return;
    setMode(v);
    // 每个任务有自己的输入字段/输出语义：切换任务必须回到空表单，不能把上一任务的
    // src/经纬度/列名等残留值当成"干净的新任务"（D-05 残留）。
    setForm({ ...EMPTY_PREP_FORM });
    clearPrepDraft();
    autoDst.current = '';
    dirtyNow.current = false;
    setProduced({});
    setImported([]);
    setProbe(null);
    setSlots([]);
    setResult(null);
    setEdited(false);
    setInputsConsumed(true);
    setError('');
    setDeclined('');
  }
  async function pick(k: string) {
    await work(async () => {
      if (k === 'dst' && mode === 'site') {
        // 站点输出是文件，但文件还不存在；让用户选目录，再按扫描器约定的
        // `_site.nc` 后缀自动补全文件名（D-06 残留）。
        const directory = await invoke<string | null>('pick_folder', { key: 'prep-dst' });
        if (directory) set('dst', preparedSiteDestination(directory, form.dst));
        return null;
      }
      const path =
        k === 'rawdata' || (k === 'dst' && ['table', 'observation'].includes(mode))
          ? await invoke<string | null>('pick_folder', { key: `prep-${k}` })
          : await invoke<string | null>('pick_file', { key: `prep-${k}`, filter: null });
      if (path) set(k, path);
      return null;
    });
  }
  async function inspect() {
    return work(async () => {
      const p = await invoke<Probe>(
        mode === 'table'
          ? 'probe_forcing_table'
          : mode === 'observation'
            ? 'probe_observation_table'
            : 'probe_forcing',
        { path: form.src },
      );
      setProbe(p);
      const hasSnow =
        (p.variables ?? []).includes('Snowf') ||
        (p.columns ?? []).some((column) => column.name === 'Snowf');
      setSlots(
        (p.slots ?? []).map((s) => ({
          ...s,
          name: s.guessed ?? s.column ?? '',
          units: s.units ?? '',
          also_add:
            s.index === 4 && (s.guessed ?? s.column) === 'Rainf' && hasSnow ? ['Snowf'] : [],
        })),
      );
      const suggested = p.suggest_dst
        ? ['netcdf', 'gaps'].includes(mode)
          ? preparedDestination(p.suggest_dst, form.src)
          : p.suggest_dst
        : '';
      setForm((f) => {
        const keep = !!f.dst && f.dst !== autoDst.current;
        if (!keep) autoDst.current = suggested;
        const next = {
          ...f,
          dst: keep ? f.dst : suggested,
          ...Object.fromEntries(
            [
              'time_column',
              'site_column',
              'latitude_column',
              'longitude_column',
              'landtype_column',
              'utc_offset_column',
            ].map((k) => [k, (p as unknown as Record<string, string>)[k] ?? f[k]]),
          ),
        };
        rememberForm(next);
        return next;
      });
      return p;
    });
  }
  function choices() {
    const missing = slots.filter((s) => !s.optional && (!s.name || !s.units));
    if (missing.length)
      throw new Error(
        t(
          `请确认变量与单位：${missing.map((s) => s.meaning).join('、')}`,
          `Confirm variables and units: ${missing.map((s) => s.meaning).join(', ')}`,
        ),
      );
    return slots
      .filter((s) => s.name)
      .map((s) => ({
        index: s.index,
        name: s.name,
        units: s.units,
        also_add: [
          ...new Set(
            s.also_add.map((name) => name.trim()).filter((name) => name && name !== s.name),
          ),
        ],
      }));
  }
  function heights() {
    const v = ['height_v', 'height_t', 'height_q'].map((k) => optional(k));
    if (v.every((x) => x === null)) return null;
    if (v.some((x) => x === null || x <= 0))
      throw new Error(
        t(
          '三项观测高度须一同填写正数',
          'All three observation heights must be positive numbers together',
        ),
      );
    return v;
  }
  function gapOptions() {
    return {
      short_gap: num('short_gap'),
      min_overlap: num('min_overlap'),
      utc_offset: optional('utc_offset'),
      latitude: optional('lat'),
      longitude: optional('lon'),
      era5: form.era5 || null,
    };
  }
  // 产物已存在时后端不覆盖（它可能正被某个算例引用）：问过再带 overwrite 重来。
  async function confirmOverwrite<T>(
    write: (overwrite: boolean) => Promise<T>,
    output = form.dst,
  ): Promise<T> {
    try {
      return await write(false);
    } catch (error) {
      if (!String(error).includes('产物已存在，未覆盖')) throw error;
      const ok = await askConfirm(
        t(
          `输出文件已经存在：${output}。覆盖会改掉引用它的算例所用的数据。要覆盖吗？`,
          `The output file already exists: ${output}. Overwriting changes the data of any case that uses it. Overwrite?`,
        ),
        { confirmLabel: t('覆盖', 'Overwrite'), danger: true },
      );
      if (!ok)
        throw Object.assign(
          new Error(
            t('已取消：没有覆盖已有的输出文件。', 'Cancelled: the existing output file was kept.'),
            { cause: error },
          ),
          { userDeclined: true },
        );
      return await write(true);
    }
  }
  async function execute() {
    const output = form.dst;
    const done = await work(async () => {
      if (!form.dst) throw new Error(t('请选择独立的输出路径', 'Destination path required'));
      const flagLike = Object.entries({
        src: form.src,
        dst: form.dst,
        rawdata: form.rawdata,
        time_column: form.time_column,
        site_column: form.site_column,
        site_name: form.site_name,
        latitude_column: form.latitude_column,
        longitude_column: form.longitude_column,
        landtype_column: form.landtype_column,
        utc_offset_column: form.utc_offset_column,
      }).find(([, value]) =>
        String(value ?? '')
          .trim()
          .startsWith('--'),
      );
      if (flagLike) {
        const name = numericLabels[flagLike[0]] ?? labels[flagLike[0]] ?? flagLike[0];
        throw new Error(
          t(
            `${name}不能以 -- 开头（命令行会把它当成参数）。`,
            `${name} must not start with -- (the command line would treat it as a flag).`,
          ),
        );
      }
      if (mode === 'site' && !preparedSiteNameValid(form.dst))
        throw new Error(
          t(
            '站点输出文件名须以 `_site.nc` 或 `_site_v1.nc` 结尾，新建模拟扫描才能识别这个站点；可点「选择…」自动补全。',
            'The site output file must end with `_site.nc` or `_site_v1.nc` so the new-case scan can find it; use “Choose…” to fill the name automatically.',
          ),
        );
      if (mode === 'site')
        return confirmOverwrite((overwrite) =>
          invoke('make_site', {
            out: form.dst,
            lon: num('lon'),
            lat: num('lat'),
            landtype: optional('landtype'),
            rawdata: form.rawdata || null,
            mode: form.site_mode || preparation.mode,
            crop: preparation.crop,
            overwrite,
          }),
        );
      if (mode === 'era5') {
        for (const [key, label] of [
          ['start', t('开始日期', 'Start date')],
          ['end', t('结束日期', 'End date')],
        ] as const) {
          const value = (form[key] ?? '').trim();
          if (!prepDateValid(value))
            throw new Error(
              t(
                `${label}须为有效日期（YYYY-MM-DD 或 YYYYMMDD，当前 ${value || '空'}）`,
                `${label} must be a valid date (YYYY-MM-DD or YYYYMMDD, currently ${value || 'empty'})`,
              ),
            );
        }
        if (prepDateKey(form.start) > prepDateKey(form.end))
          throw new Error(
            t('开始日期不能晚于结束日期。', 'The start date must not be after the end date.'),
          );
        return invoke('download_era5land', {
          dst: form.dst,
          latitude: num('lat'),
          longitude: num('lon'),
          start: form.start,
          end: form.end,
        });
      }
      if (!probe)
        throw new Error(t('先检查输入并确认字段', 'Inspect input and confirm fields first'));
      if (mode === 'observation') {
        const convert = (overwrite: boolean) =>
          invoke('convert_observation_table', {
            src: form.src,
            dstDir: form.dst,
            overwrite,
            options: {
              time_column: form.time_column,
              site_column: form.site_column || null,
              site_name: form.site_name || null,
              variables: observationVariables
                .filter((v) => v.column)
                .map((v) => ({ name: v.name, column: v.column, qc_column: v.qc_column || null })),
            },
          });
        try {
          return await convert(false);
        } catch (error) {
          // 观测产物被覆盖会悄悄换掉引用它的算例的参考数据；先让用户明确确认再重试（D-03）。
          if (!/输出文件已存在|Output already exists/.test(String(error))) throw error;
          const ok = await askConfirm(
            t(
              '目标目录已有同名观测文件。覆盖会改掉引用它的算例所用的观测数据。要覆盖吗？',
              'The destination already contains observation files with the same names. Overwriting changes the observations used by any case that references them. Overwrite?',
            ),
            { confirmLabel: t('覆盖', 'Overwrite'), danger: true },
          );
          if (!ok)
            throw Object.assign(
              new Error(
                t(
                  '已取消：保留已有观测文件。',
                  'Cancelled: the existing observation files were kept.',
                ),
              ),
              { name: 'CancelledError' },
            );
          return await convert(true);
        }
      }
      const mapped = choices();
      if (mode === 'table') {
        const batch = await invoke<Imported[]>('convert_forcing_table', {
          src: form.src,
          dst: form.dst,
          slots: mapped,
          options: {
            time_column: form.time_column,
            site_column: form.site_column || null,
            latitude_column: form.latitude_column || null,
            longitude_column: form.longitude_column || null,
            landtype_column: form.landtype_column || null,
            utc_offset_column: form.utc_offset_column || null,
            utc_offset: optional('utc_offset'),
            latitude: optional('lat'),
            longitude: optional('lon'),
            step_seconds: optional('step_seconds'),
            land_cover_scheme: preparation.classification.toLowerCase(),
            heights: heights(),
          },
        });
        setImported(batch);
        setProduced({});
        return batch;
      }
      if (mode === 'gaps')
        return confirmOverwrite((overwrite) =>
          invoke('repair_forcing', {
            src: form.src,
            dst: form.dst,
            slots: mapped,
            options: gapOptions(),
            overwrite,
          }),
        );
      return confirmOverwrite((overwrite) =>
        invoke('convert_forcing', {
          src: form.src,
          dst: form.dst,
          slots: mapped,
          heights: heights(),
          overwrite,
        }),
      );
    });
    if (done !== undefined) {
      if (mode === 'site') setProduced((p) => ({ ...p, site: output }));
      if (mode === 'era5') {
        setProduced({});
        setForm((old) => {
          const next = { ...old, era5: output };
          rememberForm(next);
          return next;
        });
      }
      if (mode === 'netcdf' || mode === 'gaps') setProduced((p) => ({ ...p, met: output }));
      // 输入已被这次成功消费：允许离页。表格拆分只写了暂存文件，还要逐站点「修复并完成」，
      // 因此不算消费完（否则离页会丢掉待办列表）。
      setInputsConsumed(mode !== 'table');
    }
    return done;
  }
  // 数据准备原来停不了，退出应用后子进程还在写文件（第 732 轮）。后端按进程组终止，
  // 正在跑的 work 随即以"已停止"结束。
  async function stopPreparation() {
    const stopped = await invoke<number>('cancel_preparation');
    if (!stopped)
      notify(
        t(
          '没有正在进行的数据准备可停；检查输入这类短任务会自己结束。',
          'No data preparation is running; short tasks such as inspecting input finish on their own.',
        ),
      );
    return stopped;
  }
  // 带着这一页做好的文件去新建模拟；原来什么都没做也能点，进去是空白向导（2026-10-22 用户实测）。
  const canUse = !['era5', 'observation'].includes(mode) && (!!produced.site || !!produced.met);
  // 把产物安装到数据集根下的 Sitedata/Forcing，并立即重新扫描一次（D-06 残留：
  // 以前产物要用户手工搬进这两个目录，转换完也不会刷新扫描）。
  async function installToDataset() {
    if (!produced.site && !produced.met) return;
    let root = datasetRoot.trim();
    if (!root) {
      const picked = await invoke<string | null>('pick_folder', { key: 'prep-dataset' });
      if (!picked) return;
      root = picked;
      setDatasetRoot(root);
      try {
        globalThis.localStorage?.setItem('colm-prep-dataset-root', root);
      } catch {
        /* 存储不可用：本次会话内继续用这个目录。 */
      }
    }
    const final = preparedDatasetDestinations(produced, root);
    const install = async (source: string, destination: string) => {
      try {
        await invoke('install_prepared_file', { source, destination });
        return true;
      } catch (error) {
        if (!/目标文件已存在|destination already exists/.test(String(error))) throw error;
        const ok = await askConfirm(
          t(
            `数据集目录里已有同名文件：${destination}。覆盖会改掉它；要覆盖吗？`,
            `The dataset already has ${destination}. Overwriting replaces it; continue?`,
          ),
          { confirmLabel: t('覆盖', 'Overwrite'), danger: true },
        );
        if (!ok) return false;
        await invoke('install_prepared_file', { source, destination, overwrite: true });
        return true;
      }
    };
    let installed = 0;
    if (produced.site && final.site && (await install(produced.site, final.site))) installed++;
    if (produced.met && final.met && (await install(produced.met, final.met))) installed++;
    if (!installed) return;
    notify(
      t(
        `已安装 ${installed} 个前处理产物到 ${root} 的 Sitedata/Forcing。`,
        `Installed ${installed} prepared file(s) into Sitedata/Forcing under ${root}.`,
      ),
    );
    if (!final.stemsMatch)
      notify(
        t(
          '站点与强迫场文件名词干不同，扫描时无法配对成同一站点；请改成相同的 <X>_site.nc / <X>_Met.nc 词干。',
          'The site and forcing files use different stems, so the scan cannot pair them; use the same <X>_site.nc / <X>_Met.nc stem.',
        ),
        true,
      );
    try {
      const scan = await invoke<{ sites: { name: string }[] }>('scan_sites', {
        dir: `${root.replace(/[\\/]+$/, '')}/Sitedata`,
        forcingDir: `${root.replace(/[\\/]+$/, '')}/Forcing`,
        quick: true,
      });
      setDatasetScan({
        root,
        count: scan.sites.length,
        names: scan.sites.map((site) => site.name).slice(0, 8),
      });
      notify(
        t(
          `已重新扫描：识别到 ${scan.sites.length} 个站点。`,
          `Rescanned: ${scan.sites.length} site(s) recognized.`,
        ),
      );
    } catch (error) {
      notify(
        t(
          `产物已安装，但重新扫描失败：${String(error)}`,
          `Files were installed, but the rescan failed: ${String(error)}`,
        ),
        true,
      );
    }
  }
  async function useForNewCase() {
    if (!canUse) return false;
    // 表格批量还有站点没"修复并完成"：直接走会被导航守卫拦下，且 newCaseSeed 已经写进状态
    // （GUI 审查 F-D-02）。先明确问，确认后丢弃未完成待办再放行；暂存文件仍留在磁盘上。
    if (imported.length > 0) {
      const ok = await askConfirm(
        t(
          `还有 ${imported.length} 个站点没有完成修复；现在离开会丢弃这些待办（已经拆分的暂存文件仍保留在磁盘）。要去新建模拟吗？`,
          `${imported.length} site(s) are not finalized yet; leaving now discards these pending items (the split staging files stay on disk). Continue to the new simulation?`,
        ),
        { confirmLabel: t('丢弃并继续', 'Discard and continue') },
      );
      if (!ok) return false;
      setImported([]);
      setEdited(false);
      setInputsConsumed(true);
      // 同步放行：navigate 可能在同一 tick 内被调用，守卫必须看到"已确认丢弃"。
      dirtyNow.current = false;
    }
    update({
      newCaseSeed: {
        form: {
          kind: 'site',
          ...(produced.site ? { site: produced.site } : {}),
          ...(produced.met ? { met: produced.met } : {}),
        },
        sites: [],
        ownData: true,
      },
    });
    return navigate('new-case');
  }
  async function finish(item: Imported) {
    return work(async () => {
      const canonical = slots
        .filter((s) => s.name)
        .map((s) => ({
          index: s.index,
          name: ['Tair', 'Qair', 'Psurf', 'Precip', 'Wind_E', 'Wind_N', 'SWdown', 'LWdown'][
            s.index - 1
          ],
          units: s.wants,
          also_add: [],
        }));
      const report = await confirmOverwrite(
        (overwrite) =>
          invoke<{ unresolved: number }>('repair_forcing', {
            src: item.staged_path,
            dst: item.final_path,
            slots: canonical,
            options: {
              ...gapOptions(),
              latitude: item.latitude,
              longitude: item.longitude,
              // Table import already normalized this staged file to UTC.
              utc_offset: null,
            },
            overwrite,
          }),
        item.final_path,
      );
      if (report.unresolved)
        throw new Error(
          t(
            `仍有 ${report.unresolved} 个缺测；请提供 ERA5 后重试`,
            `${report.unresolved} gaps remain; provide ERA5 and retry`,
          ),
        );
      setImported((items) => items.filter((x) => x.staged_path !== item.staged_path));
      setProduced({ met: item.final_path });
      // 最后一站的最终强迫场也写好了，表格任务才算消费完；还有待办站时离页保护继续保留。
      setInputsConsumed(imported.every((x) => x.staged_path === item.staged_path));
      return { path: item.final_path, ...report };
    });
  }
  const labels: Record<string, string> = {
    src: t('原始输入文件', 'Source file'),
    dst: t('独立输出路径', 'Destination path'),
    lon: t('经度', 'Longitude'),
    lat: t('纬度', 'Latitude'),
    landtype: t('地类编号', 'Land class'),
    rawdata: t('Rawdata 目录', 'Rawdata directory'),
    time_column: t('时间列', 'Time column'),
    site_column: t('站点列', 'Site column'),
    site_name: t('单站名称', 'Site name'),
    latitude_column: t('纬度列', 'Latitude column'),
    longitude_column: t('经度列', 'Longitude column'),
    landtype_column: t('地类列', 'Land class column'),
    utc_offset_column: t('时区列', 'UTC offset column'),
    utc_offset: t('UTC 偏移（小时）', 'UTC offset (hours)'),
    step_seconds: t('时间步长（秒）', 'Time step (seconds)'),
    era5: t('ERA5 辅助文件', 'ERA5 auxiliary file'),
    short_gap: t('短缺测最大步数', 'Max short-gap steps'),
    min_overlap: t('最少重叠步数', 'Min overlap steps'),
    start: t('开始日期', 'Start date'),
    end: t('结束日期', 'End date'),
    height_v: t('风速观测高度（m）', 'Wind measurement height (m)'),
    height_t: t('温度观测高度（m）', 'Temperature measurement height (m)'),
    height_q: t('湿度观测高度（m）', 'Humidity measurement height (m)'),
  };
  const basic =
    mode === 'site'
      ? ['dst', 'lon', 'lat', 'landtype', 'rawdata']
      : mode === 'era5'
        ? ['dst', 'lon', 'lat', 'start', 'end']
        : [
            'src',
            'dst',
            ...(mode === 'table' || mode === 'observation' ? ['time_column', 'site_column'] : []),
            ...(mode === 'gaps' ? ['era5'] : []),
          ];
  const extra =
    mode === 'table'
      ? [
          'latitude_column',
          'longitude_column',
          'landtype_column',
          'utc_offset_column',
          'utc_offset',
          'lat',
          'lon',
          'step_seconds',
          'height_v',
          'height_t',
          'height_q',
        ]
      : mode === 'observation'
        ? ['site_name']
        : mode === 'netcdf'
          ? ['height_v', 'height_t', 'height_q']
          : mode === 'gaps'
            ? ['short_gap', 'min_overlap', 'utc_offset', 'lat', 'lon']
            : [];
  const fields: UiField[] = [
    {
      id: 'prep-mode',
      label: t('准备任务', 'Preparation task'),
      value: mode,
      type: 'choice',
      options: [
        ['netcdf', t('NetCDF 强迫场', 'NetCDF forcing')],
        ['table', t('表格强迫场（CSV）', 'Tabular forcing (CSV)')],
        ['site', t('制作站点', 'Create site')],
        ['gaps', t('修复缺测', 'Gap-filling')],
        ['era5', t('下载 ERA5-Land', 'Download ERA5-Land')],
        ['observation', t('转换观测', 'Convert observations')],
      ].map(([value, label]) => ({ value, label })),
      disabled: busy,
      set: changeMode,
    },
    ...(mode === 'site'
      ? [
          {
            id: 'prep-site-mode',
            label: t('地表覆盖 / 植被方案', 'Land cover / vegetation scheme'),
            value: (() => {
              const allowed = preparationSiteModes(preparation.crop === true);
              const current = form.site_mode || preparation.mode;
              return allowed.includes(current) ? current : allowed[0];
            })(),
            type: 'choice' as const,
            hint:
              preparation.crop === true
                ? t(
                    '当前算例启用了 CROP：站点必须用 PFT/PC 结构生成；IGBP/USGS 会在建站审计时被拒绝。',
                    'This case enables CROP: sites must use PFT/PC structures; IGBP/USGS are rejected by the site audit.',
                  )
                : undefined,
            options: [
              ['igbp', t('IGBP 自然植被（17 类）', 'IGBP natural vegetation (17 classes)')],
              ['usgs', t('USGS 自然植被（24 类）', 'USGS natural vegetation (24 classes)')],
              ['pft', t('PFT 植被功能型', 'PFT plant functional types')],
              ['pc', t('PC 植物群落', 'PC plant communities')],
              ['urban-igbp', t('城市 + IGBP', 'Urban + IGBP')],
              ['urban-usgs', t('城市 + USGS', 'Urban + USGS')],
              ['urban-pft', t('城市 + PFT', 'Urban + PFT')],
              ['urban-pc', t('城市 + PC', 'Urban + PC')],
            ]
              .filter(([value]) =>
                (preparationSiteModes(preparation.crop === true) as readonly string[]).includes(
                  value!,
                ),
              )
              .map(([value, label]) => ({ value: value!, label: label! })),
            disabled: busy,
            // set() 会一并清掉旧的 result/produced、标脏并解除"已消费"状态。
            set: (value: string) => set('site_mode', value),
          },
        ]
      : []),
    ...[...basic, ...(advanced ? extra : [])].map((k) => ({
      id: `prep-${k}`,
      label: labels[k],
      value: form[k],
      type: 'text' as const,
      disabled: busy,
      set: (v: string) => {
        set(k, v);
        if (k === 'src') {
          setProbe(null);
          setSlots([]);
          setProduced({});
          setDeclined('');
        }
      },
    })),
    ...slots.flatMap((s, i) =>
      (s.index === 4 ? ['name', 'units', 'also_add'] : ['name', 'units']).map((key) => ({
        id: `prep-slot-${s.index}-${key}`,
        label: `${s.meaning} ${key === 'name' ? t('变量名', 'Variable') : key === 'units' ? t('单位', 'Units') : t('附加降水分量（逗号分隔）', 'Additional precipitation components (comma-separated)')}`,
        value: key === 'also_add' ? s.also_add.join(',') : String(s[key as 'name' | 'units'] ?? ''),
        type: 'text' as const,
        disabled: busy,
        set: (v: string) => {
          // 映射改了，上一次的"成功"与已产物都不再对应当前输入（GUI 审查 D-10/D-12）。
          setResult(null);
          setProduced({});
          setDeclined('');
          setEdited(true);
          setInputsConsumed(false);
          setSlots((all) =>
            all.map((x, j) =>
              i !== j
                ? x
                : key === 'also_add'
                  ? {
                      ...x,
                      also_add: v.split(','),
                    }
                  : { ...x, [key]: v, ...(key === 'name' ? { also_add: [] } : {}) },
            ),
          );
        },
      })),
    ),
    ...(mode === 'observation'
      ? observationVariables.flatMap((v, i) =>
          ['column', 'qc_column'].map((key) => ({
            id: `prep-obs-${i}-${key}`,
            label: `${v.label}${v.units ? ` (${v.units})` : ''}${v.requires_qc ? t('，需要质控列', ' (QC required)') : ''} ${key === 'column' ? t('数据列', 'Data column') : t('质控列', 'QC column')}`,
            value: v[key as 'column' | 'qc_column'] ?? '',
            type: 'text' as const,
            disabled: busy,
            set: (value: string) => {
              setResult(null);
              setProduced({});
              setDeclined('');
              setEdited(true);
              setInputsConsumed(false);
              setProbe((p) =>
                p
                  ? {
                      ...p,
                      variables: observationVariables.map((x, j) =>
                        i === j ? { ...x, [key]: value } : x,
                      ),
                    }
                  : p,
              );
            },
          })),
        )
      : []),
  ];
  fields.push({
    id: 'prep-advanced',
    label: t('高级设置', 'Advanced settings'),
    value: String(advanced),
    type: 'switch',
    set: (v) => setAdvanced(v === 'true'),
  });
  if (imported.length)
    fields.push({
      id: 'prep-batch-era5',
      label: t('ERA5 辅助文件', 'ERA5 auxiliary file'),
      value: form.era5,
      type: 'text',
      disabled: busy,
      set: (v) => set('era5', v),
    });
  const canInspect = !['site', 'era5'].includes(mode);
  usePageContract({
    title: t('准备输入数据', 'Prepare inputs'),
    step: 'preparation',
    status: busy ? 'busy' : error || 'ready',
    details: { probe, result, error, imported },
    fields,
    // 动作由页面上的 ActionButton 自己登记（第 737 轮）。
    actions: [],
  });
  const render = (f: UiField) => (
    <Field key={f.id} label={f.label} hint={f.hint}>
      {f.type === 'choice' ? (
        <Select disabled={busy} value={String(f.value)} onChange={(e) => f.set(e.target.value)}>
          {f.options?.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </Select>
      ) : (
        <Input value={String(f.value)} disabled={busy} onChange={(e) => f.set(e.target.value)} />
      )}
      {(['prep-src', 'prep-rawdata'].includes(f.id) ||
        (f.id === 'prep-dst' && ['table', 'observation', 'site'].includes(mode))) && (
        <ActionButton
          action={`prep-pick-${f.id.slice(5)}`}
          label={t(`选择 ${labels[f.id.slice(5)]}`, `Choose ${labels[f.id.slice(5)]}`)}
          disabled={busy}
          run={() => pick(f.id.slice(5))}
        >
          {t('选择…', 'Browse…')}
        </ActionButton>
      )}
    </Field>
  );
  return (
    <div className="page-stack">
      <h1>{t('准备输入数据', 'Prepare inputs')}</h1>
      <Notice>
        {t(
          '原始文件保留。先检查，再确认变量与单位，最后写入独立输出路径。ERA5 下载需要本机配置的 CDS 凭据。',
          'Source files are preserved. Inspect first, verify variables and units, then write to an isolated destination. ERA5 download requires CDS credentials configured locally.',
        )}
      </Notice>
      {/* 这一页原来只把失败放进 PageContract/全局通知；页面内没有落点，用户取消覆盖后看不到任何说明。 */}
      {error && (
        <>
          <Notice tone="error">{error}</Notice>
          <p className="muted">
            {t(
              '可以直接修改输入后再点「生成数据」重试；已经写出的文件不会被自动删除。',
              'Adjust the inputs and click “Generate data” again to retry; files already written are not deleted automatically.',
            )}
          </p>
        </>
      )}
      {declined && <Notice>{declined}</Notice>}
      {preparationDirty && (
        <Notice tone="warning">
          {t(
            '本页还有未生成/未处理的输入；切页保护会拦住导航。',
            'This page has inputs or mappings that have not been generated yet; leaving is blocked.',
          )}{' '}
          <ActionButton
            action="prep-discard-unfinished"
            label={t('放弃未生成的输入', 'Discard unfinished inputs')}
            disabled={busy}
            run={() => {
              // "放弃未生成的输入"必须真的清空输入，不能只解除守卫（D-05 残留）。
              setForm({ ...EMPTY_PREP_FORM });
              clearPrepDraft();
              autoDst.current = '';
              setProbe(null);
              setSlots([]);
              setEdited(false);
              setInputsConsumed(true);
              dirtyNow.current = false;
              setResult(null);
              setProduced({});
              setImported([]);
              setDeclined('');
              setError('');
            }}
          >
            {t('放弃未生成的输入', 'Discard unfinished inputs')}
          </ActionButton>
        </Notice>
      )}
      <Card>
        <div className="form-grid">
          {fields
            .filter(
              (f) =>
                f.id === 'prep-mode' ||
                // 制作站点的地表覆盖/植被方案选择在基础区直接可见，不能只存在于 page contract
                // 里（GUI 复核 D-04）。
                f.id === 'prep-site-mode' ||
                basic.some((k) => f.id === `prep-${k}`),
            )
            .map(render)}
        </div>
        {canInspect && (
          <ActionButton
            action="prep-inspect"
            label={t('检查输入', 'Inspect input')}
            disabled={busy || !form.src}
            title={
              busy || form.src
                ? undefined
                : t('先选原始输入文件', 'Choose the raw input file first')
            }
            run={inspect}
          />
        )}
        {sourceNames.includes('Rainf') && sourceNames.includes('Snowf') && (
          <Notice>
            {t(
              '检测到 Rainf 与 Snowf：主变量为 Rainf 时已预选 Snowf 合成总降水，请核对附加分量后生成。已有总降水变量时请留空，避免重复累加。附加分量须在源文件中声明单位，转换时统一单位后累加。',
              'Rainf and Snowf detected: Snowf is preselected when the primary variable is Rainf. Verify the additional components before generating total precipitation. Leave additions empty for an existing total. Additional components must declare their units in the source; conversion aligns units before summing.',
            )}
          </Notice>
        )}
        {slots.length > 0 && (
          <div className="form-grid">
            {fields.filter((f) => f.id.startsWith('prep-slot-')).map(render)}
          </div>
        )}
        {mode === 'observation' && (
          <div className="form-grid">
            {fields.filter((f) => f.id.startsWith('prep-obs-')).map(render)}
          </div>
        )}
        <Advanced
          open={advanced}
          onOpenChange={setAdvanced}
          title={t('高级设置', 'Advanced settings')}
        >
          <div className="form-grid">
            {fields.filter((f) => extra.some((k) => f.id === `prep-${k}`)).map(render)}
          </div>
        </Advanced>
        <div className="actions">
          {mode === 'gaps' && (
            <ActionButton
              action="prep-gaps"
              label={t('检查缺测', 'Inspect gaps')}
              disabled={busy || !probe}
              run={() => {
                setGapCheckOnly(true);
                return work(() =>
                  invoke('probe_forcing_gaps', {
                    src: form.src,
                    slots: choices(),
                    options: gapOptions(),
                  }),
                );
              }}
            >
              {t('检查缺测', 'Check gaps')}
            </ActionButton>
          )}
          <ActionButton
            action="prep-execute"
            kind="commit"
            label={t('生成数据', 'Generate data')}
            variant="primary"
            disabled={busy || !form.dst || (canInspect && !probe)}
            title={
              busy
                ? undefined
                : !form.dst
                  ? t('先填输出位置', 'Fill in the output location first')
                  : canInspect && !probe
                    ? t('先点"检查输入"核对字段', 'Click “Inspect input” to check the fields first')
                    : undefined
            }
            run={() => {
              setGapCheckOnly(false);
              return execute();
            }}
          />
          {busy && (
            <ActionButton
              action="prep-stop"
              label={t('停止', 'Stop')}
              run={() => stopPreparation().catch((e) => notify(String(e), true))}
            />
          )}
          <ActionButton
            action="prep-new-case"
            label={t('用于新算例', 'Use for new case')}
            disabled={busy || !canUse}
            title={
              canUse
                ? undefined
                : t('先生成站点文件或强迫场', 'Generate a site file or forcing first')
            }
            run={useForNewCase}
          >
            {t('用于新算例', 'Use for new case')}
          </ActionButton>
          <ActionButton
            action="prep-install-dataset"
            kind="commit"
            label={t('安装到数据集目录并扫描…', 'Install into dataset and rescan…')}
            disabled={busy || (!produced.site && !produced.met)}
            title={
              produced.site || produced.met
                ? undefined
                : t('先生成站点文件或强迫场', 'Generate a site file or forcing first')
            }
            run={() => installToDataset().catch((error) => notify(String(error), true))}
          >
            {t('安装到数据集目录并扫描…', 'Install into dataset and rescan…')}
          </ActionButton>
        </div>
        {datasetScan && (
          <Notice tone={datasetScan.count > 0 ? 'success' : 'warning'}>
            {datasetScan.count > 0
              ? t(
                  `已重新扫描 ${datasetScan.root}：识别到 ${datasetScan.count} 个站点（${datasetScan.names.join('、')}${datasetScan.count > datasetScan.names.length ? ' …' : ''}）。`,
                  `Rescanned ${datasetScan.root}: ${datasetScan.count} site(s) recognized (${datasetScan.names.join(', ')}${datasetScan.count > datasetScan.names.length ? ' …' : ''}).`,
                )
              : t(
                  `已重新扫描 ${datasetScan.root}，但没有识别到站点；请检查 Sitedata/Forcing 的布局与命名（PLUMBER2 或 Urban-PLUMBER）。`,
                  `Rescanned ${datasetScan.root} but found no sites; check the Sitedata/Forcing layout and names (PLUMBER2 or Urban-PLUMBER).`,
                )}
          </Notice>
        )}
      </Card>
      {imported.length > 0 && (
        <Card
          title={t(
            '表格已拆分：检查并完成各站强迫场',
            'Table split: inspect and finalize forcing for each site',
          )}
        >
          <Notice>
            {t(
              '这些是暂存文件。修复后才写入最终强迫场路径；需要站点文件时选择“制作站点”。',
              'These are staged files. Fix gaps before writing to the final forcing path; choose "Create site" if site files are needed.',
            )}
          </Notice>
          <Field label={t('ERA5 辅助文件', 'ERA5 auxiliary file')}>
            <Input value={form.era5} onChange={(e) => set('era5', e.target.value)} />
          </Field>
          {imported.map((item, i) => (
            <div key={item.staged_path}>
              <span>
                {item.site} · {item.staged_path}
              </span>
              <ActionButton
                action={`prep-finish-${i}`}
                kind="commit"
                label={t(`修复并完成 ${item.site}`, `Fix and finalize ${item.site}`)}
                disabled={busy}
                run={() => finish(item)}
              >
                {t('修复并完成', 'Finalize')}
              </ActionButton>
            </div>
          ))}
        </Card>
      )}
      {result != null && (
        <PreparationResultView
          result={result}
          mode={mode}
          gapCheckOnly={gapCheckOnly}
          en={state.language === 'en'}
          hasSelectedCase={!!state.selectedCase}
          // 缺测修复的 GapReport 不带路径，但页面知道后端成功写到了用户给的输出位置。
          fallbackOutput={produced.met ?? produced.site ?? null}
          onOpenRuns={() => {
            navigate('case', 'runs');
          }}
        />
      )}
    </div>
  );
}

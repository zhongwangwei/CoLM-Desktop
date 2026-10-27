import { useRef, useState } from 'react';
import { invoke } from '../../lib/ipc';
import { askConfirm } from '../../lib/confirm';
import { useApp } from '../../lib/app-state';
import { usePageContract, type UiField } from '../../lib/guide';
import { Button, Card, Input, Select, Field, Advanced, Notice } from '../../components/ui';
import { ActionButton } from '../../components/ActionButton';
import { requiredNumber, preparedDestination, casePreparationOptions } from './case-model';
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
        '站点文件已生成，可点下方「用于新算例」带着它继续。',
        'The site file is ready; use “Use for new case” below to continue with it.',
      ),
    );
  if (summary.kind === 'forcing' || summary.kind === 'era5')
    steps.push(
      t(
        '可在「项目」里打开算例运行，或点下方「用于新算例」带着这些文件继续。',
        'Open a case under Project to run it, or use “Use for new case” below to continue with these files.',
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
    (summary.kind === 'forcing' || summary.kind === 'era5' || summary.state === 'complete')
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
      {hasSelectedCase && onOpenRuns && (
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
  const { state, notify, navigate, update } = useApp();
  currentLang = state.language;
  const [imported, setImported] = useState<Imported[]>([]),
    [mode, setMode] = useState('netcdf'),
    [form, setForm] = useState<Record<string, string>>({
      src: '',
      dst: '',
      lon: '',
      lat: '',
      landtype: '',
      rawdata: '',
      time_column: '',
      site_column: '',
      site_name: '',
      latitude_column: '',
      longitude_column: '',
      landtype_column: '',
      utc_offset_column: '',
      utc_offset: '',
      step_seconds: '',
      era5: '',
      short_gap: '3',
      min_overlap: '48',
      start: '',
      end: '',
      height_v: '',
      height_t: '',
      height_q: '',
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
    [produced, setProduced] = useState<{ site?: string; met?: string }>({});
  const lock = useRef(false);
  // 上次"检查"自动推出的输出路径。换了源文件就作废：原来选 A 检查后再换成 B，输出路径还是
  // A 的推荐名，B 的数据被写进 A 的文件（GUI 审查 2026-10-22，P1-1）。用户自己改过的路径不动。
  const autoDst = useRef('');
  const observationVariables = (probe?.variables ?? []).filter((v) => typeof v !== 'string');
  const sourceNames = [
    ...(probe?.variables ?? []).filter((v): v is string => typeof v === 'string'),
    ...(probe?.columns ?? []).map((v) => v.name),
  ];
  const preparation = casePreparationOptions(state.selectedCase?.profile, state.newCaseSeed);
  const set = (k: string, v: string) => {
    // 字段一改，上一次的成功结果就过时了：先清掉，别让旧的"成功"挂在页面上（U-05）。
    if (form[k] !== v) setResult(null);
    if (k === 'src' && v !== form.src) {
      setProbe(null);
      setSlots([]);
    }
    setForm((f) =>
      k !== 'src' || v === f.src
        ? { ...f, [k]: v }
        : { ...f, src: v, dst: f.dst === autoDst.current ? '' : f.dst },
    );
  };
  const num = (k: string) => requiredNumber(form[k], k);
  const optional = (k: string) => (form[k].trim() ? num(k) : null);
  async function work(fn: () => Promise<unknown>) {
    if (lock.current) return;
    lock.current = true;
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
      const declined =
        stopped ||
        (!!e && typeof e === 'object' && (e as { userDeclined?: boolean }).userDeclined === true);
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
      setBusy(false);
      lock.current = false;
    }
  }
  function changeMode(v: string) {
    setMode(v);
    setImported([]);
    setProbe(null);
    setSlots([]);
    setResult(null);
    setError('');
  }
  async function pick(k: string) {
    await work(async () => {
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
        return {
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
  async function confirmOverwrite<T>(write: (overwrite: boolean) => Promise<T>): Promise<T> {
    try {
      return await write(false);
    } catch (error) {
      if (!String(error).includes('产物已存在，未覆盖')) throw error;
      const ok = await askConfirm(
        t(
          `输出文件已经存在：${form.dst}。覆盖会改掉引用它的算例所用的强迫场。要覆盖吗？`,
          `The output file already exists: ${form.dst}. Overwriting changes the forcing of any case that uses it. Overwrite?`,
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
      if (mode === 'site')
        return invoke('make_site', {
          out: form.dst,
          lon: num('lon'),
          lat: num('lat'),
          landtype: optional('landtype'),
          rawdata: form.rawdata || null,
          mode: preparation.mode,
          crop: preparation.crop,
        });
      if (mode === 'era5')
        return invoke('download_era5land', {
          dst: form.dst,
          latitude: num('lat'),
          longitude: num('lon'),
          start: form.start,
          end: form.end,
        });
      if (!probe)
        throw new Error(t('先检查输入并确认字段', 'Inspect input and confirm fields first'));
      if (mode === 'observation')
        return invoke('convert_observation_table', {
          src: form.src,
          dstDir: form.dst,
          options: {
            time_column: form.time_column,
            site_column: form.site_column || null,
            site_name: form.site_name || null,
            variables: observationVariables
              .filter((v) => v.column)
              .map((v) => ({ name: v.name, column: v.column, qc_column: v.qc_column || null })),
          },
        });
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
      if (mode === 'netcdf' || mode === 'gaps') setProduced((p) => ({ ...p, met: output }));
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
  const canUse = !!produced.site || !!produced.met;
  function useForNewCase() {
    if (!canUse) return false;
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
      const report = await invoke<{ unresolved: number }>('repair_forcing', {
        src: item.staged_path,
        dst: item.final_path,
        slots: canonical,
        options: {
          ...gapOptions(),
          latitude: item.latitude,
          longitude: item.longitude,
          utc_offset: item.timezone_offset_hours,
        },
      });
      if (report.unresolved)
        throw new Error(
          t(
            `仍有 ${report.unresolved} 个缺测；请提供 ERA5 后重试`,
            `${report.unresolved} gaps remain; provide ERA5 and retry`,
          ),
        );
      setImported((items) => items.filter((x) => x.staged_path !== item.staged_path));
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
        set: (v: string) =>
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
          ),
      })),
    ),
    ...(mode === 'observation'
      ? observationVariables.flatMap((v, i) =>
          ['column', 'qc_column'].map((key) => ({
            id: `prep-obs-${i}-${key}`,
            label: `${v.label} ${key === 'column' ? t('数据列', 'Data column') : t('质控列', 'QC column')}`,
            value: v[key as 'column' | 'qc_column'] ?? '',
            type: 'text' as const,
            disabled: busy,
            set: (value: string) =>
              setProbe((p) =>
                p
                  ? {
                      ...p,
                      variables: observationVariables.map((x, j) =>
                        i === j ? { ...x, [key]: value } : x,
                      ),
                    }
                  : p,
              ),
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
    <Field key={f.id} label={f.label}>
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
        (f.id === 'prep-dst' && ['table', 'observation'].includes(mode))) && (
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
      <Card>
        <div className="form-grid">
          {fields
            .filter((f) => f.id === 'prep-mode' || basic.some((k) => f.id === `prep-${k}`))
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
        </div>
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

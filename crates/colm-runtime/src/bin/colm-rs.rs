//! `colm` 第三段的 Rust 版本：从算例目录跑一个 POINT 窗口。
//!
//! ```text
//! colm-rs <case-dir|case.nml> --land-cover igbp|usgs --restart-out <path> \
//!         [--patch N] [--history-dir <dir>] [--history-stem <stem>] \
//!         [--allow-unported-branches]
//! colm-rs <case-dir|case.nml> --land-cover igbp|usgs --case-outputs [--preflight]
//! ```
//!
//! `--case-outputs` 是 `colm-cli run --engine rust` 用的形态：重启、history 目录与
//! 前缀都按 Fortran `colm.x` 在算例目录里的落点推出（见 [`CaseOutputs`]），于是下游的
//! `metrics`/`series`/指纹检查不必知道这一段是哪个引擎跑的。`--preflight` 只做
//! 能力检查（空间算例、未移植分支）就退出 —— 让 `colm-cli` 在跑前两段**之前**就能
//! 拒绝 Rust 引擎跑不了的算例，而不是等几分钟预处理之后才失败。
//!
//! **它只覆盖已经移植的那一条链**（`standard_lct` 的规则土壤 patch，无雪与积雪两支），
//! 其余一律报错。这不是保守：第三段的内核里有 PFT/PC/城市/BGC/湖/痕量物等十几条并行
//! 分支，静默挑一条相近的跑完，会得到一份看起来正常、实际上换了物理过程的结果。
//!
//! 三条刻意的接口选择：
//!
//! * **`--land-cover` 必须显式给。** 它来自内核编译期的 `LULC_IGBP`/`LULC_USGS`，
//!   namelist 里的 `DEF_USE_IGBP`/`DEF_USE_USGS` 只是只读镜像且默认两个都是 `.false.`
//!   （`MOD_Namelist.F90:163`），从算例里读不出来。
//! * **`--restart-out` 必须显式给，且必须与输入重启不同一个路径。** 上游续跑文件的
//!   路径由 `idate` 现算（`MOD_Vars_TimeVariables.F90:1123`），而初始化器写出的是带
//!   `_w180_s90` 块后缀的另一套名字；指向同一路径会让"以原文件为底、只换声明改过的
//!   变量"这条写出语义失去底稿。
//! * **不写 history 文件时也照常推进**：history 是旁路，不是状态的一部分。
//! * **`--allow-unported-branches` 默认关。** 算例要的分支里但凡有本仓库没实现的
//!   （VSF、植物水力、植被上的雪 —— 三者在**上游都是默认打开**），默认直接拒绝并
//!   一次列全；这个开关只为诊断性测量而存在，用了就会在 stderr 上打出警告。

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, ensure, Context, Result};
use colm_core::{CalendarTime, LandCoverScheme, StandardLctSnowSoilState};
use colm_namelist::{parse, Document, Value};
use colm_runtime::assembly::{
    assemble_standard_lct_snow_template, assemble_standard_lct_template, restart_has_snow_column,
    EvolvedStepOutput, MonthlyLeafAreaIndex, RestartStateFiles, StandardLctRestartTemplate,
    SurfaceDiagnosticsRow,
};
use colm_runtime::baseflow_optimizer::BaseflowOptimizer;
use colm_runtime::history::HistorySession;
use colm_runtime::physics::land_physics_parameters;
use colm_runtime::{read_point_runtime_config, PatchStepOutput, PointRuntime, PointRuntimeConfig};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // `{:#}` 打出 anyhow 的整条 context 链：这一层最需要的是"哪个文件/
            // 哪个字段"，而不是最外层那句概括。
            eprintln!("colm-rs: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// `colm-kernel` 的成败判定要求这一行（`Stage::Colm.success_marker()`），与
/// Fortran `colm.x` 的最后一行逐字相同。只在真正写完重启之后打印。
const SUCCESS_MARKER: &str = "CoLM Execution Completed.";

fn run() -> Result<()> {
    let arguments = Arguments::parse(std::env::args().skip(1))?;
    let layout = colm_case::Layout::new(&arguments.case_directory);
    let case_nml = layout.case_nml();
    ensure!(
        case_nml.is_file(),
        "{} has no case.nml; point the first argument at a prepared case directory",
        arguments.case_directory.display()
    );
    ensure!(
        !colm_case::is_spatial_case(&case_nml)?,
        "colm-rs only runs the SinglePoint chain; {} is a spatial case",
        arguments.case_directory.display()
    );
    let name = colm_case::case_name(&case_nml)?;

    let config = read_point_runtime_config(&case_nml)?;
    let document = read_document(&case_nml)?;
    let physics = land_physics_parameters(
        &document,
        arguments.land_cover,
        // 三级优先级已在 `read_point_runtime_config` 里解出来（文件 > forcing namelist
        // > schema 默认）。实测 CN-Cng 是 6/6/6，而 schema 默认是 100/50/50。
        colm_runtime::physics::ObservationHeights {
            wind_m: config.wind_height_m,
            temperature_m: config.temperature_height_m,
            humidity_m: config.humidity_height_m,
        },
    )?;
    // 本仓库没有实现的分支：**一次列全**，并且默认拒绝。
    //
    // 每一项在上游都是默认打开的，所以"只写了几行"的算例几乎必然会撞上其中一条。
    // 直接跑完不会报错，只会给出另一套物理下的"看起来正常"的数字 —— 这正是本仓库
    // 最忌讳的错。`--allow-unported-branches` 是为**诊断性测量**留的：它把不匹配
    // 显式写进命令行，而不是让它悄悄发生。
    let missing = colm_runtime::physics::unported_branches(&physics);
    if !missing.is_empty() {
        ensure!(
            arguments.allow_unported_branches,
            "this case needs {} branch(es) the Rust runtime does not implement:\n  - {}\n\
             Set the switches above to use the ported path, or pass \
             --allow-unported-branches to run anyway for a labelled diagnostic measurement.",
            missing.len(),
            missing.join("\n  - ")
        );
        eprintln!(
            "colm-rs: WARNING: running {} unported branch(es); results are NOT a faithful \
             reproduction:",
            missing.len()
        );
        for branch in &missing {
            eprintln!("  - {branch}");
        }
    }
    if arguments.preflight {
        println!("colm-rs preflight: ok");
        return Ok(());
    }
    let restarts = restart_files(&layout, &name, &document, &config)?;
    let outputs = arguments
        .outputs
        .resolve(&layout, &name, &document, &config)?;
    ensure!(
        outputs.restart_out != restarts.initial,
        "the evolved restart would overwrite the input restart {}; \
         DEF_simulation_time%end must be after %start",
        restarts.initial.display()
    );
    if let Some(parent) = outputs.restart_out.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    if let Some(directory) = &outputs.history_directory {
        std::fs::create_dir_all(directory)
            .with_context(|| format!("cannot create {}", directory.display()))?;
    }
    let files = RestartStateFiles {
        constant: restarts.constant.clone(),
        time: restarts.initial.clone(),
    };

    // 两支装配的**断言**不同（一支要求启动时有雪、另一支要求没有），但返回的是同一个
    // 模板类型；运行时只走通用入口（能长雪的那一支），所以这里按启动时的雪列选断言。
    let has_snow = restart_has_snow_column(&files, arguments.patch)
        .context("cannot tell whether the initial restart carries an active snow column")?;
    let mut template = if has_snow {
        assemble_standard_lct_snow_template(&files, arguments.patch, physics)
            .context("cannot assemble the snow-bearing standard LCT template")?
    } else {
        assemble_standard_lct_template(&files, arguments.patch, physics)
            .context("cannot assemble the snow-free standard LCT template")?
    };
    // `DEF_LAI_MONTHLY` 打开时每月重读 LAI（`CoLM.F90:595-605`）。**不装就等于关门**：
    // 跨月的运行会从第二个月起一直用第一天的叶面积，而且不会报错。
    if logical_field(&document, "DEF_LAI_MONTHLY")? {
        let path = layout.out().join(&name).join("landdata/srfdata.nc");
        ensure!(
            path.is_file(),
            "{} is missing; run mksrfdata for this case before a DEF_LAI_MONTHLY run",
            path.display()
        );
        template = template.with_monthly_leaf_area_index(MonthlyLeafAreaIndex::read(
            &path,
            logical_field(&document, "USE_SITE_LAI")?,
            logical_field(&document, "DEF_LAI_CHANGE_YEARLY")?,
            i32::try_from(integer_field(&document, "DEF_LC_YEAR")?)
                .context("DEF_LC_YEAR does not fit an i32")?,
        )?);
    }

    // `scale_baseflow`：上游 `Opt_Baseflow_init` 从
    // `DEF_dir_restart/ParaOpt/<case>_baseflow.nc` 读一个长度 `landpatch` 的向量，
    // 文件或变量缺失时取 `defval = 1.`（`MOD_Opt_Baseflow.F90:37-38`）。
    // 它直接乘在 `rsubst`/`rsub` 上，参数标定过的算例差别是物理量级的。
    let baseflow_scale = read_baseflow_scale(&layout, &name, arguments.patch)?;
    template = template.with_baseflow_scale(baseflow_scale);
    // `Opt_Baseflow_init` 无论开不开优化都先建 `ParaOpt/`（`MOD_Opt_Baseflow.F90:40-42`）。
    let para_opt = layout.out().join(&name).join("restart/ParaOpt");
    std::fs::create_dir_all(&para_opt)
        .with_context(|| format!("cannot create {}", para_opt.display()))?;
    let baseflow_optimizer = if logical_field(&document, "DEF_Optimize_Baseflow")? {
        // 上游对**所有** patch 一起迭代并整向量写回；本程序只跑一个 patch，
        // 多 patch 的重启若照写单元素文件，会把其余 patch 的标定值抹掉。
        let patches = colm_init::RestartFile::open(&restarts.initial)?
            .floats("zwt")?
            .len();
        ensure!(
            patches == 1,
            "DEF_Optimize_Baseflow rewrites `scale_baseflow` for every patch, but colm-rs runs \
             one patch of the {patches} in {}",
            restarts.initial.display()
        );
        Some(BaseflowOptimizer::new(
            baseflow_scale,
            template.snow_state().soil_water.water_table_depth_m,
            template.patch_type,
            para_opt,
            &name,
        ))
    } else {
        None
    };

    // 会话从**配置**开（窗口、站点、步长、频率都在里面），要在 `open` 消费掉
    // 配置之前建好 —— 而它自己不带 forcing，所以先后没有别的影响。
    let session = history_session(&config, &outputs)?;
    let mut runtime = PointRuntime::open(config)?;
    if let Some(optimizer) = baseflow_optimizer {
        runtime = runtime.with_baseflow_optimizer(optimizer);
    }
    // **一个入口跑到底。** 上游每步无条件先 `newsnow`（`CoLMMAIN.F90:976`）再造打包列，
    // 所以"这一步有没有雪"是状态、不是配置；这里同理 —— 从无雪起步的运行也必须能长雪。
    let summary = run_snow(
        &mut runtime,
        &template,
        &restarts.initial,
        &outputs.restart_out,
        outputs.periodic.as_ref(),
        session,
    )?;
    println!(
        "colm-rs: {} step(s) on patch {}; wrote {}",
        summary.steps,
        arguments.patch,
        outputs.restart_out.display()
    );
    if let Some(files) = summary.history_files {
        println!("colm-rs: {} history file(s)", files);
    }
    println!("{SUCCESS_MARKER}");
    Ok(())
}

struct RunSummary {
    steps: usize,
    /// `None` 表示这份算例没开 history（`DEF_HIST_FREQ = 'none'`）。
    history_files: Option<usize>,
}

/// 积雪分支：同一个窗口，内核换成 `standard_lct_snow_soil_step`，
/// 续跑写出多带雪段与四个雪标量。
fn run_snow(
    runtime: &mut PointRuntime,
    template: &StandardLctRestartTemplate,
    restart_in: &Path,
    restart_out: &Path,
    periodic: Option<&PeriodicRestarts>,
    session: Option<HistorySession>,
) -> Result<RunSummary> {
    let mut state = template.snow_state();
    // `smp`/`hk` 与表面诊断量只出现在步输出里（`intent(out)`），而续跑要写它们。
    let mut last: Option<RestartSnapshot> = None;
    let mut on_step = |step: colm_runtime::PointRuntimeStep,
                       state: &StandardLctSnowSoilState,
                       output: PatchStepOutput<'_>|
     -> Result<()> {
        let snapshot = RestartSnapshot::new(state, output, step.surface_cosine_zenith)?;
        // `save_to_restart`（`CoLM.F90:664`）：每个 `DEF_WRST_FREQ` 周期末、以及预热期
        // 每年末写一次 `WRITE_TimeVariables`。窗口终点那一次由循环结束后的写出负责。
        if let Some(periodic) = periodic {
            if step.clock.write_restart {
                let path = periodic.path(step.clock.end_time);
                if path != restart_out {
                    write_evolved_restart(template, state, &snapshot, restart_in, &path)?;
                }
            }
        }
        last = Some(snapshot);
        Ok(())
    };
    let (steps, history_files) = match session {
        Some(mut session) => {
            let outcome = runtime.run_restart_standard_lct_snow_with_history(
                template,
                &mut state,
                &mut session,
                &mut on_step,
            )?;
            (outcome.steps, Some(outcome.files.len()))
        }
        None => (
            runtime.run_restart_standard_lct_snow(template, &mut state, &mut on_step)?,
            None,
        ),
    };
    let last = last.context(NO_STEP)?;
    write_evolved_restart(template, &state, &last, restart_in, restart_out)?;
    Ok(RunSummary {
        steps,
        history_files,
    })
}

/// 续跑写出要的、状态里没有的那部分步输出。
struct RestartSnapshot {
    matric_potential_mm: Vec<f64>,
    hydraulic_conductivity_mm_s: Vec<f64>,
    diagnostics: SurfaceDiagnosticsRow,
}

impl RestartSnapshot {
    fn new(
        state: &StandardLctSnowSoilState,
        output: PatchStepOutput<'_>,
        cosine_zenith: f64,
    ) -> Result<Self> {
        Ok(match output {
            PatchStepOutput::Soil(output) => Self {
                matric_potential_mm: output.water.soil.matric_potential_mm.clone(),
                hydraulic_conductivity_mm_s: output.water.soil.hydraulic_conductivity_mm_s.clone(),
                diagnostics: SurfaceDiagnosticsRow::from_lct(&output.energy, cosine_zenith)?,
            },
            // 冰川分支不调 `soilwater`：`smp`/`hk` 保持重启里的值。
            PatchStepOutput::Glacier(output) => Self {
                matric_potential_mm: state.soil_water.matric_potential_mm.clone(),
                hydraulic_conductivity_mm_s: state.soil_water.hydraulic_conductivity_mm_s.clone(),
                diagnostics: SurfaceDiagnosticsRow::from_glacier(&output.thermal, cosine_zenith),
            },
            // 湖同样不调 `soilwater`。
            PatchStepOutput::Lake(output) => Self {
                matric_potential_mm: state.soil_water.matric_potential_mm.clone(),
                hydraulic_conductivity_mm_s: state.soil_water.hydraulic_conductivity_mm_s.clone(),
                diagnostics: SurfaceDiagnosticsRow::from_lake(&output.thermal, cosine_zenith),
            },
        })
    }
}

/// 把步末状态写成一份续跑文件（以输入重启为底，只替换推进过的变量）。
fn write_evolved_restart(
    template: &StandardLctRestartTemplate,
    state: &StandardLctSnowSoilState,
    snapshot: &RestartSnapshot,
    restart_in: &Path,
    restart_out: &Path,
) -> Result<()> {
    // 重启里的 `t_grnd` 是雪层合并之后重取的那个（`CoLMMAIN.F90:1452`）。
    let overrides = template.evolved_snow_overrides(
        state,
        EvolvedStepOutput {
            ground_temperature_k: state.surface_temperature_k(),
            matric_potential_mm: &snapshot.matric_potential_mm,
            hydraulic_conductivity_mm_s: &snapshot.hydraulic_conductivity_mm_s,
            diagnostics: snapshot.diagnostics,
        },
    )?;
    if let Some(parent) = restart_out.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    write_restart(restart_in, restart_out, overrides.as_slice())
}

/// 周期续跑文件的落点：与窗口终点那份同一套命名，`cdate` 取该步的 `jdate`
/// （步末 `idate` 经 `adj2begin`，即 86400 秒写成次日 0 秒）。
struct PeriodicRestarts {
    directory: PathBuf,
    name: String,
    land_cover_year: i64,
}

impl PeriodicRestarts {
    fn path(&self, end_time: CalendarTime) -> PathBuf {
        let label = date_label(normalized_day_end(end_time));
        self.directory.join(&label).join(format!(
            "{}_restart_{label}_lc{:04}_w180_s90.nc",
            self.name, self.land_cover_year
        ))
    }
}

const NO_STEP: &str = "the window produced no step, so there is no evolved state to write back; \
                       check that DEF_simulation_time%end is after %start";

fn write_restart(
    restart_in: &Path,
    restart_out: &Path,
    overrides: &[colm_init::RestartOverride],
) -> Result<()> {
    colm_init::RestartFile::open(restart_in)
        .with_context(|| format!("cannot open the input restart {}", restart_in.display()))?
        .write_with(restart_out, overrides)
        .with_context(|| {
            format!(
                "cannot write the evolved restart to {}",
                restart_out.display()
            )
        })
}

/// 算例开了 history 就开一个会话，否则返回 `None`（不建目录、不写文件）。
fn history_session(
    config: &PointRuntimeConfig,
    outputs: &ResolvedOutputs,
) -> Result<Option<HistorySession>> {
    let Some(directory) = &outputs.history_directory else {
        return Ok(None);
    };
    // `DEF_HIST_FREQ = 'none'`（声明默认值）：上游 `hist_out` 走 `CASE default`，
    // 从不写文件。建一个空调度会被当成"窗口与频率对不上"报错。
    if config.history_frequency == colm_hist::schedule::HistoryFrequency::None {
        return Ok(None);
    }
    config
        .history_session(directory, outputs.history_stem.clone())
        .map(Some)
}

/// 输出落点的两种来源：命令行逐项给，或按算例目录约定推出。
enum OutputSpec {
    Explicit {
        restart_out: PathBuf,
        history_directory: Option<PathBuf>,
        history_stem: String,
    },
    CaseOutputs,
}

struct ResolvedOutputs {
    restart_out: PathBuf,
    /// 按算例目录约定落盘时才写周期续跑；命令行逐项指定输出时只写终点那一份。
    periodic: Option<PeriodicRestarts>,
    history_directory: Option<PathBuf>,
    history_stem: String,
}

impl OutputSpec {
    /// `CaseOutputs` 照 Fortran `colm.x` 的落点：
    ///
    /// * 重启：`<out>/<case>/restart/<cdate>/<case>_restart_<cdate>_lc<year>_w180_s90.nc`，
    ///   `cdate` 取**窗口终点**（上游在最后一步之后按 `idate` 写续跑文件，
    ///   `MOD_Vars_TimeVariables.F90:1109`）；
    /// * history：`<out>/<case>/history/<case>_hist_*.nc` —— `colm-cli` 的
    ///   `history_files` 只认 `*_hist_*.nc`，前缀必须是算例名，而不是缺省的 `colm-rs`。
    fn resolve(
        &self,
        layout: &colm_case::Layout,
        name: &str,
        document: &Document,
        config: &PointRuntimeConfig,
    ) -> Result<ResolvedOutputs> {
        match self {
            Self::Explicit {
                restart_out,
                history_directory,
                history_stem,
            } => Ok(ResolvedOutputs {
                restart_out: restart_out.clone(),
                periodic: None,
                history_directory: history_directory.clone(),
                history_stem: history_stem.clone(),
            }),
            Self::CaseOutputs => {
                let year = integer_field(document, "DEF_LC_YEAR")?;
                let out = layout.out().join(name);
                let label = date_label(normalized_day_end(config.end));
                Ok(ResolvedOutputs {
                    restart_out: out
                        .join("restart")
                        .join(&label)
                        .join(format!("{name}_restart_{label}_lc{year:04}_w180_s90.nc")),
                    periodic: Some(PeriodicRestarts {
                        directory: out.join("restart"),
                        name: name.to_owned(),
                        land_cover_year: year,
                    }),
                    history_directory: Some(out.join("history")),
                    history_stem: name.to_owned(),
                })
            }
        }
    }
}

/// 算例目录 → 两份输入重启的路径。
///
/// 初始化器写的是**带块后缀**的名字：常数重启
/// `<site>_restart_const_lc<year>_w180_s90.nc`，时间重启
/// `<site>_restart_<cdate>_lc<year>_w180_s90.nc`，其中 `cdate = %04d-%03d-%05d`
/// （年-儒略日-当日秒），与上游 `MOD_Vars_TimeVariables.F90:1109` 的写法一致。
struct RestartFiles {
    constant: PathBuf,
    initial: PathBuf,
}

fn restart_files(
    layout: &colm_case::Layout,
    name: &str,
    document: &Document,
    config: &PointRuntimeConfig,
) -> Result<RestartFiles> {
    let year = integer_field(document, "DEF_LC_YEAR")?;
    let out = layout.out().join(name);
    let label = date_label(config.start);
    let constant = out
        .join("restart/const")
        .join(format!("{name}_restart_const_lc{year:04}_w180_s90.nc"));
    let initial = out
        .join("restart")
        .join(&label)
        .join(format!("{name}_restart_{label}_lc{year:04}_w180_s90.nc"));
    for path in [&constant, &initial] {
        ensure!(
            path.is_file(),
            "{} is missing; run mksrfdata and mkinidata for this case first",
            path.display()
        );
    }
    Ok(RestartFiles { constant, initial })
}

/// `ParaOpt/<case>_baseflow.nc` 里的 `scale_baseflow(patch)`。
///
/// 上游 `ncio_read_vector (file, 'scale_baseflow', landpatch, scale_baseflow, defval = 1.)`
/// （`MOD_Opt_Baseflow.F90:37-38`）：**文件不在、或文件里没有这个变量**，都取 1.0。
/// 本机三个黄金算例都属于前者（内核日志会打 "default value is used"），
/// 所以这道读取对它们没有影响；被标定过的算例则从此与内核一致。
fn read_baseflow_scale(layout: &colm_case::Layout, name: &str, patch: usize) -> Result<f64> {
    // 文件名与重启同一套块后缀约定：`MOD_Block.F90:641-645` 的
    // `get_filename_block` 把 `_<block>` 插在 `.nc` 之前，单点算例是 `w180_s90`。
    // 写成不带后缀的 `..._baseflow.nc` 内核根本不会读（会打 "not found" 走默认值）。
    let path = layout
        .out()
        .join(name)
        .join("restart/ParaOpt")
        .join(format!("{name}_baseflow_w180_s90.nc"));
    if !path.is_file() {
        return Ok(1.0);
    }
    let file = colm_init::RestartFile::open(&path)?;
    if file.variable_dimensions("scale_baseflow").is_err() {
        return Ok(1.0);
    }
    let values = file.floats("scale_baseflow")?;
    ensure!(
        patch < values.len(),
        "{} carries {} `scale_baseflow` value(s), too few for patch {patch}",
        path.display(),
        values.len()
    );
    Ok(values[patch])
}

/// 窗口终点写成 `sec = 86400` 时，上游写续跑文件用的 `idate` 已经被 `TICKTIME`
/// 进位成次日 0 秒：实测 1 月 31 日 86400 秒的窗口，`colm.x` 写的是
/// `2008-032-00000`，不是 `2008-031-86400`。两边名字不同，Fortran→Rust 切换后
/// 续跑就找不到文件。
fn normalized_day_end(time: CalendarTime) -> CalendarTime {
    if time.seconds < 86_400 {
        return time;
    }
    let leap = time.year % 4 == 0 && (time.year % 100 != 0 || time.year % 400 == 0);
    let days_in_year = if leap { 366 } else { 365 };
    if time.julian_day >= days_in_year {
        CalendarTime {
            year: time.year + 1,
            julian_day: 1,
            seconds: 0,
        }
    } else {
        CalendarTime {
            julian_day: time.julian_day + 1,
            seconds: 0,
            ..time
        }
    }
}

/// `%04d-%03d-%05d`：`MOD_Vars_TimeVariables.F90:1109` 的 `cdate`。
fn date_label(time: CalendarTime) -> String {
    format!(
        "{:04}-{:03}-{:05}",
        time.year, time.julian_day, time.seconds
    )
}

fn read_document(path: &Path) -> Result<Document> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read case namelist {}", path.display()))?;
    parse(&text).with_context(|| format!("cannot parse case namelist {}", path.display()))
}

/// 取一个逻辑字段：算例里写了就用算例的，否则用 schema 的声明默认值。
fn logical_field(document: &Document, field: &str) -> Result<bool> {
    if let Some(value) = document.get(field) {
        return match value {
            Value::Bool(value) => Ok(*value),
            other => bail!("{field} must be a logical, got {other:?}"),
        };
    }
    match colm_schema::find(field).map(|field| &field.default) {
        Some(colm_schema::Default::Logical(value)) => Ok(*value),
        _ => bail!("{field} is missing from the case namelist and has no logical default"),
    }
}

/// 取一个整数字段：算例里写了就用算例的，否则用 schema 的声明默认值。
fn integer_field(document: &Document, field: &str) -> Result<i64> {
    if let Some(value) = document.get(field) {
        return match value {
            Value::Int(value) => Ok(*value),
            other => bail!("{field} must be an integer, got {other:?}"),
        };
    }
    match colm_schema::find(field).map(|field| &field.default) {
        Some(colm_schema::Default::Integer(value)) => Ok(*value),
        _ => bail!("{field} is missing from the case namelist and has no integer default"),
    }
}

struct Arguments {
    case_directory: PathBuf,
    patch: usize,
    land_cover: LandCoverScheme,
    outputs: OutputSpec,
    /// 只做能力检查就退出，不读重启、不推进。
    preflight: bool,
    /// 显式允许跑"本仓库没实现的那些分支"。默认关。
    allow_unported_branches: bool,
}

impl Arguments {
    fn parse(arguments: impl Iterator<Item = String>) -> Result<Self> {
        let mut values = arguments.peekable();
        let mut case_directory = None;
        let mut patch = 0usize;
        let mut land_cover = None;
        let mut restart_out = None;
        let mut history_directory = None;
        let mut history_stem = None;
        let mut allow_unported_branches = false;
        let mut case_outputs = false;
        let mut preflight = false;
        while let Some(flag) = values.next() {
            let mut value = |name: &str| -> Result<String> {
                values
                    .next()
                    .with_context(|| format!("{name} needs a value"))
            };
            match flag.as_str() {
                "--patch" => {
                    patch = value("--patch")?
                        .parse()
                        .context("--patch must be a nonnegative integer")?;
                }
                "--land-cover" => {
                    land_cover = Some(match value("--land-cover")?.to_ascii_lowercase().as_str() {
                        "igbp" => LandCoverScheme::Igbp,
                        "usgs" => LandCoverScheme::Usgs,
                        other => bail!("--land-cover must be igbp or usgs, got {other:?}"),
                    });
                }
                "--restart-out" => restart_out = Some(PathBuf::from(value("--restart-out")?)),
                "--history-dir" => {
                    history_directory = Some(PathBuf::from(value("--history-dir")?));
                }
                "--history-stem" => history_stem = Some(value("--history-stem")?),
                "--allow-unported-branches" => allow_unported_branches = true,
                "--case-outputs" => case_outputs = true,
                "--preflight" => preflight = true,
                other if other.starts_with("--") => {
                    bail!("unknown option {other}; the accepted set is documented in this binary's module docs")
                }
                other => {
                    ensure!(
                        case_directory.is_none(),
                        "only one case directory is accepted, got {other:?} after another"
                    );
                    case_directory = Some(PathBuf::from(other));
                }
            }
        }
        let case_directory =
            case_directory.context("a case directory is required as the first argument")?;
        // `colm-kernel` 启动每一段时第一个参数总是 namelist 路径（与 `colm.x` 同一个
        // 调用约定），所以这里也收 `case.nml` 本身，取它所在的目录作算例目录。
        let case_directory = if case_directory.is_file() {
            case_directory
                .parent()
                .map(Path::to_path_buf)
                .context("the case namelist has no parent directory")?
        } else {
            case_directory
        };
        let land_cover = land_cover
            .context("--land-cover is required: the compiled LULC scheme is not in the namelist")?;
        // 一个只在有 `--history-dir` 时才生效的 `--history-stem` 是陷阱：
        // 用户以为改了名字，实际什么都没写。
        if history_directory.is_none() {
            ensure!(
                history_stem.is_none(),
                "--history-stem without --history-dir would be silently ignored"
            );
        }
        let outputs = if case_outputs {
            // 两套来源混用时谁说了算不明确，干脆不许。
            ensure!(
                restart_out.is_none() && history_directory.is_none(),
                "--case-outputs derives --restart-out and --history-dir; do not pass them too"
            );
            OutputSpec::CaseOutputs
        } else if preflight {
            // 预检不写任何东西，落点无意义。
            OutputSpec::CaseOutputs
        } else {
            OutputSpec::Explicit {
                restart_out: restart_out.context(
                    "--restart-out is required: the evolved restart has no derivable name \
                     (or pass --case-outputs to use the case directory layout)",
                )?,
                history_directory,
                // 上游 history 文件名的默认前缀是算例名，不是可执行名 —— 但是否写、
                // 写在哪由调用方决定，这里只给一个能认出来的中性名。
                history_stem: history_stem.unwrap_or_else(|| "colm-rs".to_owned()),
            }
        };
        Ok(Self {
            case_directory,
            patch,
            land_cover,
            outputs,
            preflight,
            allow_unported_branches,
        })
    }
}

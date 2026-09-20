//! `colm` 第三段的 Rust 版本：从算例目录跑一个 POINT 窗口。
//!
//! ```text
//! colm-rs <case-dir> --land-cover igbp|usgs --restart-out <path> \
//!         [--patch N] [--history-dir <dir>] [--history-stem <stem>] \
//!         [--allow-unported-branches]
//! ```
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
use colm_core::{CalendarTime, LandCoverScheme};
use colm_namelist::{parse, Document, Value};
use colm_runtime::assembly::{
    assemble_standard_lct_snow_template, assemble_standard_lct_template, restart_has_snow_column,
    EvolvedStepOutput, RestartStateFiles, StandardLctRestartTemplate,
};
use colm_runtime::history::HistorySession;
use colm_runtime::physics::land_physics_parameters;
use colm_runtime::{read_point_runtime_config, PointRuntime, PointRuntimeConfig};

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
    let restarts = restart_files(&layout, &name, &document, &config)?;
    let files = RestartStateFiles {
        constant: restarts.constant.clone(),
        time: restarts.initial.clone(),
    };

    // 两支装配的**断言**不同（一支要求启动时有雪、另一支要求没有），但返回的是同一个
    // 模板类型；运行时只走通用入口（能长雪的那一支），所以这里按启动时的雪列选断言。
    let has_snow = restart_has_snow_column(&files, arguments.patch)
        .context("cannot tell whether the initial restart carries an active snow column")?;
    let template = if has_snow {
        assemble_standard_lct_snow_template(&files, arguments.patch, physics)
            .context("cannot assemble the snow-bearing standard LCT template")?
    } else {
        assemble_standard_lct_template(&files, arguments.patch, physics)
            .context("cannot assemble the snow-free standard LCT template")?
    };

    // 会话从**配置**开（窗口、站点、步长、频率都在里面），要在 `open` 消费掉
    // 配置之前建好 —— 而它自己不带 forcing，所以先后没有别的影响。
    let session = history_session(&config, &arguments)?;
    let mut runtime = PointRuntime::open(config)?;
    // **一个入口跑到底。** 上游每步无条件先 `newsnow`（`CoLMMAIN.F90:976`）再造打包列，
    // 所以"这一步有没有雪"是状态、不是配置；这里同理 —— 从无雪起步的运行也必须能长雪。
    let summary = run_snow(
        &mut runtime,
        &template,
        &restarts.initial,
        &arguments,
        session,
    )?;
    println!(
        "colm-rs: {} step(s) on patch {}; wrote {}",
        summary.steps,
        arguments.patch,
        arguments.restart_out.display()
    );
    if let Some(files) = summary.history_files {
        println!("colm-rs: {} history file(s)", files);
    }
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
    arguments: &Arguments,
    session: Option<HistorySession>,
) -> Result<RunSummary> {
    let mut state = template.snow_state();
    let mut last_ground_temperature_k = None;
    // `smp`/`hk` 只出现在步输出里（`soilwater` 的 `intent(out)`），而续跑要写它们。
    let mut last_water = None;
    let mut last_energy = None;
    let mut last_cosine_zenith = 0.0;
    let (steps, history_files) = match session {
        Some(mut session) => {
            let outcome = runtime.run_restart_standard_lct_snow_with_history(
                template,
                &mut state,
                &mut session,
                |step, output| {
                    last_ground_temperature_k = Some(output.energy.ground.temperature_k[0]);
                    last_water = Some(output.water.clone());
                    last_energy = Some(output.energy.clone());
                    last_cosine_zenith = step.forcing.cosine_zenith;
                    Ok(())
                },
            )?;
            (outcome.steps, Some(outcome.files.len()))
        }
        None => (
            runtime.run_restart_standard_lct_snow(template, &mut state, |step, output| {
                last_ground_temperature_k = Some(output.energy.ground.temperature_k[0]);
                last_water = Some(output.water.clone());
                last_energy = Some(output.energy.clone());
                last_cosine_zenith = step.forcing.cosine_zenith;
                Ok(())
            })?,
            None,
        ),
    };
    let ground_temperature_k = last_ground_temperature_k.context(NO_STEP)?;
    let last_water = last_water.context(NO_STEP)?;
    let last_energy = last_energy.context(NO_STEP)?;
    let overrides = template.evolved_snow_overrides(
        &state,
        EvolvedStepOutput {
            ground_temperature_k,
            matric_potential_mm: &last_water.soil.matric_potential_mm,
            hydraulic_conductivity_mm_s: &last_water.soil.hydraulic_conductivity_mm_s,
            cosine_zenith: last_cosine_zenith,
            energy: &last_energy,
        },
    )?;
    write_restart(restart_in, &arguments.restart_out, &overrides)?;
    Ok(RunSummary {
        steps,
        history_files,
    })
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
    arguments: &Arguments,
) -> Result<Option<HistorySession>> {
    let Some(directory) = &arguments.history_directory else {
        return Ok(None);
    };
    config
        .history_session(directory, arguments.history_stem.clone())
        .map(Some)
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
    restart_out: PathBuf,
    history_directory: Option<PathBuf>,
    history_stem: String,
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
        let restart_out = restart_out
            .context("--restart-out is required: the evolved restart has no derivable name")?;
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
        Ok(Self {
            case_directory,
            patch,
            land_cover,
            restart_out,
            history_directory,
            // 上游 history 文件名的默认前缀是算例名，不是可执行名 —— 但是否写、
            // 写在哪由调用方决定，这里只给一个能认出来的中性名。
            history_stem: history_stem.unwrap_or_else(|| "colm-rs".to_owned()),
            allow_unported_branches,
        })
    }
}

//! `colm` 第三段的 Rust 版本：从算例目录跑一个 POINT 窗口。
//!
//! ```text
//! colm-rs <case-dir> --land-cover igbp|usgs --restart-out <path> \
//!         [--patch N] [--history-dir <dir>] [--history-stem <stem>]
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

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, ensure, Context, Result};
use colm_core::{CalendarTime, LandCoverScheme};
use colm_namelist::{parse, Document, Value};
use colm_runtime::assembly::{
    assemble_standard_lct_snow_template, assemble_standard_lct_template, restart_has_snow_column,
    RestartStateFiles, StandardLctRestartTemplate,
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
    let physics = land_physics_parameters(&document, arguments.land_cover)?;
    // **跑之前先挡住分支不匹配。** 上游在选了 van Genuchten 时强制打开
    // `DEF_USE_VariablySaturatedFlow`（`MOD_Namelist.F90:1767-1772`），而它的声明
    // 默认值本来就是 `.true.` —— 也就是说默认配置走 VSF 土壤水文。本仓库的
    // `variably_saturated_flow.rs` 有那 15 个内核，但**没有编排**（`WATER_VSF`
    // 的驱动没人调用），运行时只有经典 Richards 路径。
    //
    // 按经典路径跑完 VSF 算例不会报错，只会给出另一套水文下的"看起来正常"的结果 ——
    // 这正是本仓库最忌讳的那种错。所以在这里拒绝，并说清当前支持哪一种组合。
    ensure!(
        !physics.variably_saturated_flow,
        concat!(
            "this case runs the variably saturated flow (VSF) soil hydrology, which the ",
            "Rust runtime does not orchestrate yet: MOD_Namelist.F90:1767 forces ",
            "DEF_USE_VariablySaturatedFlow on whenever the van Genuchten soil model is ",
            "selected, and its own default is true. WATER_2014 (the ported routine) is the ",
            "Campbell/Richards path; the van Genuchten path is WATER_VSF (CoLMMAIN.F90:1215), ",
            "whose kernels exist in variably_saturated_flow.rs but have no driver. Running ",
            "this case through the classic path would produce plausible numbers under a ",
            "different hydrology. Set DEF_USE_Campbell_SOIL_MODEL = .true. and ",
            "DEF_USE_VariablySaturatedFlow = .false. to use the ported path."
        )
    );
    let restarts = restart_files(&layout, &name, &document, &config)?;
    let files = RestartStateFiles {
        constant: restarts.constant.clone(),
        time: restarts.initial.clone(),
    };

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
    let summary = if has_snow {
        run_snow(
            &mut runtime,
            &template,
            &restarts.initial,
            &arguments,
            session,
        )?
    } else {
        run_soil(
            &mut runtime,
            &template,
            &restarts.initial,
            &arguments,
            session,
        )?
    };
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

/// 无雪分支跑完整个窗口，并把最后一步的地表温度交给续跑写出。
fn run_soil(
    runtime: &mut PointRuntime,
    template: &StandardLctRestartTemplate,
    restart_in: &Path,
    arguments: &Arguments,
    session: Option<HistorySession>,
) -> Result<RunSummary> {
    let mut state = template.state();
    // `evolved_overrides` 要的是**最后一步**的地表温度：它只出现在步输出里，
    // 状态只带逐层土温。
    let mut last_ground_temperature_k = None;
    let (steps, history_files) = match session {
        Some(mut session) => {
            let outcome = runtime.run_restart_standard_lct_with_history(
                template,
                &mut state,
                &mut session,
                |_, output| {
                    last_ground_temperature_k = Some(output.energy.ground.temperature_k[0]);
                    Ok(())
                },
            )?;
            (outcome.steps, Some(outcome.files.len()))
        }
        None => (
            runtime.run_restart_standard_lct(template, &mut state, |_, output| {
                last_ground_temperature_k = Some(output.energy.ground.temperature_k[0]);
                Ok(())
            })?,
            None,
        ),
    };
    let ground_temperature_k = last_ground_temperature_k.context(NO_STEP)?;
    let overrides = template.evolved_overrides(&state, ground_temperature_k)?;
    write_restart(restart_in, &arguments.restart_out, &overrides)?;
    Ok(RunSummary {
        steps,
        history_files,
    })
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
    let (steps, history_files) = match session {
        Some(mut session) => {
            let outcome = runtime.run_restart_standard_lct_snow_with_history(
                template,
                &mut state,
                &mut session,
                |_, output| {
                    last_ground_temperature_k = Some(output.energy.ground.temperature_k[0]);
                    Ok(())
                },
            )?;
            (outcome.steps, Some(outcome.files.len()))
        }
        None => (
            runtime.run_restart_standard_lct_snow(template, &mut state, |_, output| {
                last_ground_temperature_k = Some(output.energy.ground.temperature_k[0]);
                Ok(())
            })?,
            None,
        ),
    };
    let ground_temperature_k = last_ground_temperature_k.context(NO_STEP)?;
    let overrides = template.evolved_snow_overrides(&state, ground_temperature_k)?;
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
        })
    }
}

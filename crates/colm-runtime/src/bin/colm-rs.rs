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
use colm_runtime::baseflow_optimizer::{BaseflowOptimizer, BaseflowPatchInit};
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
    let name = colm_case::case_name(&case_nml)?;
    if colm_case::is_spatial_case(&case_nml)? {
        return run_spatial(&arguments, &layout, &name, &case_nml);
    }

    let config = read_point_runtime_config(&case_nml)?;
    let document = read_document(&case_nml)?;
    let mut physics = land_physics_parameters(
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
    // `CROP` 内核：`DEF_USE_CROP` 是宏的只读映射，由 `--crop` 告知。只有它打开时
    // `DEF_USE_FERT`/`DEF_USE_CNSOYFIXN`/`DEF_USE_IRRIGATION` 才生效（`MOD_Namelist.F90` 在 CROP
    // 关闭时把它们强制关掉）。
    // 不置位而直接跑会把作物当成非作物 BGC 静默跑完（第 422 轮实测 36/39 份重启不同）。
    if !arguments.crop {
        physics.irrigation = None;
    }
    if arguments.crop {
        let switches = physics
            .bgc
            .context("CROP kernels need DEF_USE_BGC (the crop state lives in the BGC restarts)")?;
        let switches = colm_core::bgc_driver::BgcSwitches {
            crop: true,
            fert: logical_field(&document, "DEF_USE_FERT")?,
            cnsoyfixn: logical_field(&document, "DEF_USE_CNSOYFIXN")?,
            irrigation: physics.irrigation.is_some(),
            ..switches
        };
        physics.bgc = Some(switches);
    }
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
    // 终点续跑文件的目录由写出时再建（`write_evolved_restart`）：中途失败的运行不留空目录，与上游一致。
    if let Some(directory) = &outputs.history_directory {
        std::fs::create_dir_all(directory)
            .with_context(|| format!("cannot create {}", directory.display()))?;
    }
    let files = RestartStateFiles {
        constant: restarts.constant.clone(),
        time: restarts.initial.clone(),
    };

    // 多作物单点有多个 patch（每个一种作物、一个 PFT），逐个装配；`--patch` 只跑其中一个。
    let patch_count = colm_init::RestartFile::open(&files.constant)?.dimension("patch")?;
    let patches: Vec<usize> = match arguments.patch {
        Some(patch) => vec![patch],
        None => (0..patch_count).collect(),
    };
    let mut templates = patches
        .iter()
        .map(|&patch| {
            assemble_patch(
                &document,
                &layout,
                &name,
                &files,
                physics.clone(),
                patch,
                SINGLE_POINT_BLOCK,
                false,
            )
            .with_context(|| format!("cannot assemble patch {patch}"))
        })
        .collect::<Result<Vec<_>>>()?;
    // 网格元里各 patch 的面积份额 `elm_patch%subfrc`：多作物单点是归一化的 `pctcrop`，与 PFT 常数重启的
    // `cropfrac` 同值（`MOD_SingleSrfdata.F90:1493`）。单 patch 为 1。
    if patch_count > 1 {
        let fractions =
            colm_init::RestartFile::open(colm_runtime::pft::pft_restart_path(&files.constant)?)?
                .floats("cropfrac")
                .context("a multi-patch single point is a crop site and needs cropfrac")?
                .to_vec();
        for template in &mut templates {
            template.patch_fraction = *fractions
                .get(template.patch)
                .context("cropfrac is shorter than the patch count")?;
        }
    }
    // `Opt_Baseflow_init` 无论开不开优化都先建 `ParaOpt/`（`MOD_Opt_Baseflow.F90:40-42`）。
    let para_opt = layout.out().join(&name).join("restart/ParaOpt");
    std::fs::create_dir_all(&para_opt)
        .with_context(|| format!("cannot create {}", para_opt.display()))?;
    let baseflow_optimizer = if logical_field(&document, "DEF_Optimize_Baseflow")? {
        // 上游对**所有** patch 一起迭代并整向量写回 `ParaOpt/<case>_baseflow.nc`；`--patch` 只跑其中
        // 一个时照写会把其余 patch 的标定值抹掉，所以只接受整站点。
        ensure!(
            templates.len() == patch_count,
            "DEF_Optimize_Baseflow needs every patch of the site ({patch_count}), but --patch \
             selected {}: upstream rewrites the whole `scale_baseflow` vector",
            templates.len()
        );
        let patches = templates
            .iter()
            .map(|template| BaseflowPatchInit {
                scale: template.baseflow_scale,
                water_table_depth_m: template.snow_state().soil_water.water_table_depth_m,
                patch_type: template.patch_type,
            })
            .collect::<Vec<_>>();
        Some(BaseflowOptimizer::new(&patches, para_opt, &name))
    } else {
        None
    };
    let template = &templates[0];

    // 会话从**配置**开（窗口、站点、步长、频率都在里面），要在 `open` 消费掉
    // 配置之前建好 —— 而它自己不带 forcing，所以先后没有别的影响。
    let mut session = history_session(&config, &outputs)?
        .map(|session| session.with_patches(templates.len()))
        .transpose()?;
    let sidecar_config = colm_runtime::history_sidecar::SidecarConfig {
        frequency_code: history_frequency_code(config.history_frequency),
        urban_run: logical_field(&document, "DEF_URBAN_RUN")?,
        urban_patches: templates
            .iter()
            .filter(|template| template.urban.is_some())
            .count(),
        pft_or_pc: logical_field(&document, "DEF_USE_PFT")?
            || logical_field(&document, "DEF_USE_PC")?,
        bgc: template.physics.bgc.is_some(),
        crop: template.physics.bgc.is_some_and(|switches| switches.crop),
        river_lake_flow: false,
    };
    // `read_history_acc_restart`（`CoLM.F90:376`）：续跑重启带着未写完的历史区间时接着累加。
    let initial_window = colm_runtime::history_sidecar::read_sidecar(
        &restarts.initial,
        &history_sidecar_path(&restarts.initial)?,
        &sidecar_config,
    )?;
    if let Some(window) =
        initial_window.filter(|windows| windows.first().is_some_and(|window| window.steps > 0))
    {
        let session = session.as_mut().context(
            "the restart carries an open history window, but this run writes no history",
        )?;
        session.restore(window)?;
    }
    let history_restart = HistoryRestart {
        config: sidecar_config,
        window: session.as_ref().map(HistorySession::window_handle),
    };
    // 主循环的 `coszen`/`cosazi`/本地时间都读常数重启的 `patchlonr`/`patchlatr`（上游
    // `MOD_Vars_TimeInvariants`），不从度数现算 —— 两者差 1 ULP 时只有读重启才与内核同源。
    let config = {
        let constant = colm_init::RestartFile::open(&files.constant)
            .context("cannot open the constant restart for patchlonr/patchlatr")?;
        let pick = |name: &str| -> Result<f64> {
            constant
                .floats(name)?
                .get(patches[0])
                .copied()
                .with_context(|| format!("the constant restart has no {name} for this patch"))
        };
        PointRuntimeConfig {
            site_radians: Some((pick("patchlonr")?, pick("patchlatr")?)),
            ..config
        }
    };
    let mut runtime = PointRuntime::open(config)?;
    if let Some(optimizer) = baseflow_optimizer {
        runtime = runtime.with_baseflow_optimizer(optimizer);
    }
    // **一个入口跑到底。** 上游每步无条件先 `newsnow`（`CoLMMAIN.F90:976`）再造打包列，
    // 所以"这一步有没有雪"是状态、不是配置；这里同理 —— 从无雪起步的运行也必须能长雪。
    let summary = run_snow(
        &mut runtime,
        &templates,
        &restarts.initial,
        &outputs.restart_out,
        outputs.periodic.as_ref(),
        session,
        &history_restart,
    )?;
    println!(
        "colm-rs: {} step(s) on patch(es) {:?}; wrote {}",
        summary.steps,
        patches,
        outputs.restart_out.display()
    );
    if let Some(files) = summary.history_files {
        println!("colm-rs: {} history file(s)", files);
    }
    println!("{SUCCESS_MARKER}");
    Ok(())
}

/// 空间算例（`GRIDBASED`）：patch 拓扑来自 `landdata`，强迫是网格强迫经面积加权映射到每个 patch。
///
/// 物理与单点完全相同（同一个 `advance_patch`）；history 写成经纬网格（`HistForm = 'Gridded'`）。
/// 目前还没有：多分块、河湖流（`GridRiverLakeFlow` 的河道量与河道旁车）、城市/PFT 的网格 LAI；
/// 遇到就拒绝或明说。
fn run_spatial(
    arguments: &Arguments,
    layout: &colm_case::Layout,
    name: &str,
    case_nml: &Path,
) -> Result<()> {
    use colm_runtime::spatial::{
        forcing::GriddedForcing,
        history::{build_history_grid, ElementGroups, HistoryGridConfig, SpatialHistory},
        mapping::AreaWeightedMapping,
        runtime::SpatialRuntime,
        runtime::SpatialRuntimeConfig,
        topology::SpatialTopology,
    };
    ensure!(
        arguments.patch.is_none(),
        "--patch selects a patch of a single point; spatial cases run every patch"
    );
    ensure!(
        !arguments.crop,
        "CROP kernels are not ported to the Rust spatial runtime yet"
    );
    let document = read_document(case_nml)?;
    let config = SpatialRuntimeConfig::read(case_nml)?;
    let mut physics = land_physics_parameters(
        &document,
        arguments.land_cover,
        colm_runtime::physics::ObservationHeights {
            wind_m: config.forcing.height_wind_m,
            temperature_m: config.forcing.height_temperature_m,
            humidity_m: config.forcing.height_humidity_m,
        },
    )?;
    physics.irrigation = None;
    // GRID/UNSTRUCTURED 内核总是编进 `GridRiverLakeFlow`，它改变了几处收缩形状。
    physics.river_lake_flow_build = true;
    let missing = colm_runtime::physics::unported_branches(&physics);
    ensure!(
        missing.is_empty() || arguments.allow_unported_branches,
        "this case needs {} branch(es) the Rust runtime does not implement:\n  - {}",
        missing.len(),
        missing.join("\n  - ")
    );
    // GRID 内核总是编进 `GridRiverLakeFlow`：汇流默认路径（单向耦合），其余选项还没移植。
    for (field, unported) in [
        (
            "DEF_GridRiverLake_FloodFeedback",
            logical_field(&document, "DEF_GridRiverLake_FloodFeedback")?,
        ),
        (
            "DEF_GridRiverLake_FloodplainStorageFix",
            logical_field(&document, "DEF_GridRiverLake_FloodplainStorageFix")?,
        ),
        ("DEF_USE_LEVEE", logical_field(&document, "DEF_USE_LEVEE")?),
        (
            "DEF_USE_BIFURCATION",
            logical_field(&document, "DEF_USE_BIFURCATION")?,
        ),
        (
            "DEF_Reservoir_Method > 0",
            integer_field(&document, "DEF_Reservoir_Method")? > 0,
        ),
        (
            "DEF_USE_TRACER",
            logical_field(&document, "DEF_USE_TRACER")?,
        ),
        (
            "DEF_GRIDBASED_ROUTING_MOMENTUM_DT_LIMIT",
            logical_field(&document, "DEF_GRIDBASED_ROUTING_MOMENTUM_DT_LIMIT")?,
        ),
        (
            "DEF_UnitCatchment_regional",
            logical_field(&document, "DEF_UnitCatchment_regional")?,
        ),
    ] {
        ensure!(
            !unported,
            "{field} is not ported to the Rust river model; run this case with --engine fortran"
        );
    }
    if arguments.preflight {
        println!("colm-rs preflight: ok");
        return Ok(());
    }
    let year = integer_field(&document, "DEF_LC_YEAR")?;
    let out = layout.out().join(name);
    let topology = SpatialTopology::read(&out.join("landdata"), i32::try_from(year)?)?;
    let start_label = date_label(config.start);
    // 每个分块一份常数重启、一份时间重启；patch 在块内的下标就是它在那两份文件里的行。
    let block_files = topology
        .blocks
        .iter()
        .map(|(block, patches)| {
            let files = RestartStateFiles {
                constant: out
                    .join("restart/const")
                    .join(format!("{name}_restart_const_lc{year:04}_{block}.nc")),
                time: out.join("restart").join(&start_label).join(format!(
                    "{name}_restart_{start_label}_lc{year:04}_{block}.nc"
                )),
            };
            for path in [&files.constant, &files.time] {
                ensure!(
                    path.is_file(),
                    "{} is missing; run mksrfdata and mkinidata for this case first",
                    path.display()
                );
            }
            let count = colm_init::RestartFile::open(&files.constant)?.dimension("patch")?;
            ensure!(
                count == patches.len(),
                "the constant restart of block {block} has {count} patches but landpatch has {}",
                patches.len()
            );
            Ok(files)
        })
        .collect::<Result<Vec<_>>>()?;
    let patch_count = topology.patch_count();
    let mut templates = Vec::with_capacity(patch_count);
    let mut coordinates = Vec::with_capacity(patch_count);
    let mut patch_mask = Vec::with_capacity(patch_count);
    for ((block, patches), files) in topology.blocks.iter().zip(&block_files) {
        for patch in 0..patches.len() {
            templates.push(
                assemble_patch(
                    &document,
                    layout,
                    name,
                    files,
                    physics.clone(),
                    patch,
                    block,
                    true,
                )
                .with_context(|| format!("cannot assemble patch {patch} of block {block}"))?,
            );
        }
        let constant = colm_init::RestartFile::open(&files.constant)?;
        coordinates.extend(
            constant
                .floats("patchlonr")?
                .iter()
                .copied()
                .zip(constant.floats("patchlatr")?.iter().copied()),
        );
        if constant.contains("patchmask") {
            patch_mask.extend(
                constant
                    .integers("patchmask")?
                    .iter()
                    .map(|&mask| mask != 0),
            );
        } else {
            patch_mask.extend(std::iter::repeat_n(true, patches.len()));
        }
    }
    let grid = GriddedForcing::open_grid(&config.forcing, config.start)?;
    let mut history_grid =
        if config.history_frequency != colm_hist::schedule::HistoryFrequency::None {
            let patch_types = templates
                .iter()
                .map(|template| template.patch_type)
                .collect::<Vec<_>>();
            Some(build_history_grid(
                &HistoryGridConfig::read(&document)?,
                &grid,
                &topology,
                &patch_types,
                &patch_mask,
            )?)
        } else {
            None
        };
    let mapping = AreaWeightedMapping::build(
        &grid,
        &topology.pixel,
        &topology.cells,
        &topology.shared_fraction,
    )?;
    let cells = mapping
        .parts
        .iter()
        .flatten()
        .map(|part| (part.ilon, part.ilat))
        .collect::<Vec<_>>();
    let forcing = GriddedForcing::new(
        config.forcing.clone(),
        grid,
        cells,
        config.timestep_seconds as i32,
    )?;
    let mut runtime = SpatialRuntime::new(
        config.clock()?,
        forcing,
        mapping,
        coordinates,
        config.co2_scenario,
    )?;
    runtime.apply_mapped_heights(&mut templates)?;
    let network = colm_runtime::river::network::RiverNetwork::read(Path::new(&string_field(
        &document,
        "DEF_UnitCatchment_file",
    )?))?;
    let routing = colm_runtime::river::network::RunoffRouting::build(&network, &topology)?;
    let runoff_filter = templates
        .iter()
        .zip(&patch_mask)
        .map(|(template, &mask)| template.patch_type < 99 && mask)
        .collect::<Vec<_>>();
    // 网格 history 多一个静态场与 6 个河道量（`MOD_Hist.F90:4749-4790`）。
    let history_grid = history_grid.take().map(|mut grid| {
        grid.statics.push((
            "mask_complete_upstream_regird".to_owned(),
            "Mask of grids with all upstream located in simulation region".to_owned(),
            "100%".to_owned(),
            colm_runtime::river::history::RiverHistoryWriter::upstream_mask_static(
                &network,
                &routing,
                &grid,
                &runoff_filter,
            ),
        ));
        std::sync::Arc::new(grid)
    });
    let river_writer = history_grid
        .as_ref()
        .map(|grid| {
            colm_runtime::river::history::RiverHistoryWriter::new(
                &network,
                &routing,
                std::sync::Arc::clone(grid),
                &runoff_filter,
                out.join("history"),
                name,
            )
        })
        .transpose()?;
    let river_start = river_restart_path(&out, name, &start_label, year);
    let river_state = colm_runtime::river::restart::read_river_state(&river_start, &network)?;
    // `restore_river_history_acc_restart`：陆面旁车标记 `history_river_required = 1` 时读回河道累加。
    let river_history = {
        let sidecar = history_sidecar_path(&block_files[0].time)?;
        let required = sidecar.is_file()
            && netcdf::open(&sidecar)
                .ok()
                .and_then(|file| {
                    file.variable("history_river_required")
                        .map(|v| v.get_values::<f64, _>(..))
                })
                .transpose()?
                .is_some_and(|marker| marker.contains(&1.0));
        if required {
            let river_file = out
                .join("restart")
                .join(&start_label)
                .join(format!("{name}_restart_hist_{start_label}.nc.river"));
            ensure!(
                river_file.is_file(),
                "{} marks an open river-history window, but {} is missing",
                sidecar.display(),
                river_file.display()
            );
            Some(colm_runtime::river::restart::read_river_history(
                &river_file,
                network.len(),
            )?)
        } else {
            None
        }
    };
    let mut river = colm_runtime::river::RiverModel::new(
        network,
        routing,
        river_state,
        real_field(&document, "DEF_GRIDBASED_ROUTING_MAX_DT")?,
    )?;
    if let Some(history) = river_history {
        river.history = history;
    }
    runtime = runtime.with_river(river, runoff_filter)?;
    let rest_compression = u8::try_from(integer_field(&document, "DEF_REST_CompressLevel")?)
        .context("DEF_REST_CompressLevel must fit 0..=9")?;
    let para_opt = out.join("restart/ParaOpt");
    std::fs::create_dir_all(&para_opt)
        .with_context(|| format!("cannot create {}", para_opt.display()))?;
    if logical_field(&document, "DEF_Optimize_Baseflow")? {
        // 上游的 `ParaOpt/<case>_baseflow.nc` 是分块的向量文件；单块时与单点同形，多块还没接。
        ensure!(
            topology.blocks.len() == 1,
            "DEF_Optimize_Baseflow is ported for single-block spatial domains only"
        );
        let patches = templates
            .iter()
            .map(|template| BaseflowPatchInit {
                scale: template.baseflow_scale,
                water_table_depth_m: template.snow_state().soil_water.water_table_depth_m,
                patch_type: template.patch_type,
            })
            .collect::<Vec<_>>();
        runtime = runtime.with_baseflow_optimizer(BaseflowOptimizer::new(&patches, para_opt, name));
    }
    // 网格 history（`HistForm = 'Gridded'`）：会话与单点同一套，写文件时聚合到 `ghist`。
    // 区间跨过重启时的续跑旁车要带河道累加器，还没移植：`write_sidecar` 在那时拒绝。
    let mut history = match history_grid {
        Some(grid) => Some(SpatialHistory {
            session: colm_runtime::open_history_session(
                config.history_window(),
                colm_runtime::history::point_dimensions(),
                colm_hist::history::HistorySite {
                    latitude_degrees: 0.0,
                    longitude_degrees: 0.0,
                },
                out.join("history"),
                name,
            )?
            .with_patches(patch_count)?
            .with_grid(grid)
            .with_gridded(&colm_runtime::river::history::GRIDDED_RIVER_VARIABLES),
            river: river_writer,
            elements: ElementGroups::from_topology(&topology)?,
            files: Vec::new(),
        }),
        None => None,
    };
    let periodic = topology
        .blocks
        .iter()
        .map(|(block, _)| PeriodicRestarts {
            directory: out.join("restart"),
            name: name.to_owned(),
            land_cover_year: year,
            block: block.clone(),
        })
        .collect::<Vec<_>>();
    let end_time = normalized_day_end(config.end);
    // 每份续跑文件旁都写历史累加器旁车（`write_history_acc_restart`），空间构建是 schema 2。
    let history_restart = HistoryRestart {
        config: colm_runtime::history_sidecar::SidecarConfig {
            frequency_code: history_frequency_code(config.history_frequency),
            urban_run: logical_field(&document, "DEF_URBAN_RUN")?,
            urban_patches: templates
                .iter()
                .filter(|template| template.urban.is_some())
                .count(),
            pft_or_pc: logical_field(&document, "DEF_USE_PFT")?
                || logical_field(&document, "DEF_USE_PC")?,
            bgc: false,
            crop: false,
            river_lake_flow: true,
        },
        window: history
            .as_ref()
            .map(|history| history.session.window_handle()),
    };
    // `read_history_acc_restart`：续跑重启带着未写完的历史区间时接着累加（每块一份旁车，按块拼接）。
    let mut restored = Vec::with_capacity(patch_count);
    let mut open_window = false;
    for ((_, patches), files) in topology.blocks.iter().zip(&block_files) {
        match colm_runtime::history_sidecar::read_sidecar(
            &files.time,
            &history_sidecar_path(&files.time)?,
            &history_restart.config,
        )? {
            Some(windows) if windows.first().is_some_and(|window| window.steps > 0) => {
                ensure!(
                    windows.len() == patches.len(),
                    "the history sidecar of {} holds {} patches for a block of {}",
                    files.time.display(),
                    windows.len(),
                    patches.len()
                );
                open_window = true;
                restored.extend(windows);
            }
            _ => restored.extend(std::iter::repeat_n(
                colm_runtime::history_sidecar::HistoryWindow::default(),
                patches.len(),
            )),
        }
    }
    if open_window {
        history
            .as_mut()
            .context("the restart carries an open history window, but this run writes no history")?
            .session
            .restore(restored)?;
    }
    let mut states: Vec<StandardLctSnowSoilState> = templates
        .iter()
        .map(StandardLctRestartTemplate::snow_state)
        .collect();
    let mut last: Option<Vec<RestartSnapshot>> = None;
    let steps = runtime.run(
        &templates,
        &mut states,
        history.as_mut(),
        |steps, states, outputs, river| {
            let mut snapshots = states
                .iter()
                .zip(outputs)
                .zip(steps)
                .map(|((state, output), step)| {
                    let mut snapshot =
                        RestartSnapshot::new(state, *output, step.surface_cosine_zenith)?;
                    snapshot.lai_refreshed = step.clock.update_lai;
                    Ok(snapshot)
                })
                .collect::<Result<Vec<_>>>()?;
            if let Some(previous) = &last {
                for (snapshot, previous) in snapshots.iter_mut().zip(previous) {
                    snapshot.diagnostics.carry_forward(&previous.diagnostics);
                }
            }
            let step = steps[0];
            if step.clock.write_restart && normalized_day_end(step.clock.end_time) != end_time {
                write_block_restarts(
                    &topology.blocks,
                    &block_files,
                    &periodic,
                    step.clock.end_time,
                    &templates,
                    states,
                    &snapshots,
                    &history_restart,
                    river,
                )?;
                if let Some(river) = river {
                    let label = date_label(normalized_day_end(step.clock.end_time));
                    colm_runtime::river::restart::write_river_state(
                        &river_restart_path(&out, name, &label, year),
                        &river.network,
                        &river.state,
                        rest_compression,
                    )?;
                }
            }
            last = Some(snapshots);
            Ok(())
        },
    )?;
    let last = last.context(NO_STEP)?;
    let written = write_block_restarts(
        &topology.blocks,
        &block_files,
        &periodic,
        config.end,
        &templates,
        &states,
        &last,
        &history_restart,
        runtime.river(),
    )?;
    if let Some(river) = runtime.river() {
        let label = date_label(normalized_day_end(config.end));
        colm_runtime::river::restart::write_river_state(
            &river_restart_path(&out, name, &label, year),
            &river.network,
            &river.state,
            rest_compression,
        )?;
    }
    println!(
        "colm-rs: {steps} step(s) on {patch_count} spatial patch(es) in {} block(s); wrote {}",
        topology.blocks.len(),
        written
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    if let Some(mut history) = history {
        history.files.extend(history.session.finish()?);
        ensure!(
            history.session.remaining() == 0,
            "the run ended with {} history record(s) still unwritten",
            history.session.remaining()
        );
        println!("colm-rs: {} history file(s)", history.files.len());
    }
    println!("{SUCCESS_MARKER}");
    Ok(())
}

/// 河道续跑文件（不分块）：`restart/<date>/<case>_restart_gridriver_<date>_lc<year>.nc`。
fn river_restart_path(out: &Path, name: &str, label: &str, year: i64) -> PathBuf {
    out.join("restart")
        .join(label)
        .join(format!("{name}_restart_gridriver_{label}_lc{year:04}.nc"))
}

/// 每个分块各写一份续跑文件（从本块的时间重启复制结构）和它的历史累加器旁车。
#[allow(clippy::too_many_arguments)]
fn write_block_restarts(
    blocks: &[(String, std::ops::Range<usize>)],
    files: &[RestartStateFiles],
    periodic: &[PeriodicRestarts],
    end_time: CalendarTime,
    templates: &[StandardLctRestartTemplate],
    states: &[StandardLctSnowSoilState],
    snapshots: &[RestartSnapshot],
    history: &HistoryRestart,
    river: Option<&colm_runtime::river::RiverModel>,
) -> Result<Vec<PathBuf>> {
    // `river_active`：河道 history 累加器（`acctime_ucat`）有值。
    let river_history = river.map(colm_runtime::river::RiverModel::history_for_restart);
    let river_required =
        river_history.is_some_and(|history| history.acctime.iter().any(|&t| t > 0.0));
    let mut written = Vec::with_capacity(blocks.len());
    for (((_, patches), files), periodic) in blocks.iter().zip(files).zip(periodic) {
        let path = periodic.path(end_time);
        write_evolved_restart(
            &templates[patches.clone()],
            &states[patches.clone()],
            &snapshots[patches.clone()],
            &files.time,
            &path,
        )?;
        mark_history_restart_with_river(&path, history, Some(patches.clone()), river_required)?;
        written.push(path);
    }
    if let (Some(history), true) = (river_history, river_required) {
        // `history_river_acc_file`：陆面旁车的基名（不带块后缀）加 `.river`。
        let label = date_label(normalized_day_end(end_time));
        let base = periodic
            .first()
            .context("a spatial run has at least one block")?
            .directory
            .join(&label)
            .join(format!(
                "{}_restart_hist_{label}.nc.river",
                periodic[0].name
            ));
        colm_runtime::river::restart::write_river_history(&base, history)?;
    }
    Ok(written)
}

/// 第 `patch` 个 patch 的模板：主/常数重启、PFT 子网格、BGC 与灌溉、月度 LAI、`scale_baseflow`。
///
/// 多作物单点（`MOD_SingleSrfdata.F90:348`）每个 patch 各装一份；patch 之间除强迫外不共享任何东西。
#[allow(clippy::too_many_arguments)] // 单点与空间共用；分块名与 `spatial` 只决定文件名与 LAI 来源
fn assemble_patch(
    document: &Document,
    layout: &colm_case::Layout,
    name: &str,
    files: &RestartStateFiles,
    physics: colm_runtime::assembly::LandPhysicsParameters,
    patch: usize,
    block: &str,
    spatial: bool,
) -> Result<StandardLctRestartTemplate> {
    // 两支装配的**断言**不同（一支要求启动时有雪、另一支要求没有），但返回的是同一个
    // 模板类型；运行时只走通用入口（能长雪的那一支），所以这里按启动时的雪列选断言。
    let has_snow = restart_has_snow_column(files, patch)
        .context("cannot tell whether the initial restart carries an active snow column")?;
    let mut template = if has_snow {
        assemble_standard_lct_snow_template(files, patch, physics)
            .context("cannot assemble the snow-bearing standard LCT template")?
    } else {
        assemble_standard_lct_template(files, patch, physics)
            .context("cannot assemble the snow-free standard LCT template")?
    };
    // `DEF_USE_PFT`：土壤 patch 的 PFT 子网格来自同目录的 `*_restart_pft_*` 两份重启。
    if template.physics.use_pft {
        template = template
            .with_pft(
                &colm_runtime::pft::pft_restart_path(&files.constant)?,
                &colm_runtime::pft::pft_restart_path(&files.time)?,
                document,
            )
            .context("cannot assemble the PFT subgrid")?;
    }
    // `DEF_USE_BGC`：BGC 状态来自四份 BGC 重启，氮沉降来自 `DEF_dir_runtime/ndep`。
    if let Some(switches) = template.physics.bgc {
        let (bgc, irrigation) = assemble_bgc(document, files, patch, switches)?;
        template = template
            .with_bgc(bgc)
            .context("cannot assemble the BGC state")?;
        // `DEF_USE_IRRIGATION`：时间重启里的灌溉量，叠上 `CROP_readin` 读的灌溉方式（与配水比例）。
        if let Some(readin) = irrigation {
            let state = colm_runtime::irrigation::initial_state(
                &colm_init::RestartFile::open(&files.time)?,
                patch,
                readin,
            )?;
            template = template
                .with_irrigation(state)
                .context("cannot assemble the irrigation state")?;
        }
    }
    ensure!(
        template.physics.irrigation.is_none() || template.irrigation.is_some(),
        "DEF_USE_IRRIGATION is verified only on CROP BGC soil patches"
    );
    // `DEF_LAI_MONTHLY` 打开时每月重读 LAI（`CoLM.F90:595-605`）。**不装就等于关门**：
    // 跨月的运行会从第二个月起一直用第一天的叶面积，而且不会报错。
    // 空间算例的逐月 LAI 读 `landdata/LAI/<year>/` 的分块向量（`LAI_readin` 的非单点支）。
    if logical_field(document, "DEF_LAI_MONTHLY")? && spatial {
        ensure!(
            template.urban.is_none() && template.pft.is_none(),
            "the Rust spatial runtime reads LCT LAI only; urban and PFT/PC LAI are not ported"
        );
        let year = |key: &str| -> Result<i32> {
            i32::try_from(integer_field(document, key)?)
                .with_context(|| format!("{key} does not fit an i32"))
        };
        template = template.with_monthly_leaf_area_index(MonthlyLeafAreaIndex::read_grid(
            layout.out().join(name).join("landdata"),
            block,
            patch,
            logical_field(document, "DEF_LAI_CHANGE_YEARLY")?,
            year("DEF_LC_YEAR")?,
            (year("DEF_LAI_START_YEAR")?, year("DEF_LAI_END_YEAR")?),
        ));
    }
    if logical_field(document, "DEF_LAI_MONTHLY")? && !spatial {
        let path = layout.out().join(name).join("landdata/srfdata.nc");
        ensure!(
            path.is_file(),
            "{} is missing; run mksrfdata for this case before a DEF_LAI_MONTHLY run",
            path.display()
        );
        let change_yearly = logical_field(document, "DEF_LAI_CHANGE_YEARLY")?;
        let land_cover_year = i32::try_from(integer_field(document, "DEF_LC_YEAR")?)
            .context("DEF_LC_YEAR does not fit an i32")?;
        if template.urban.is_some() {
            // 城市 patch 走 `UrbanLAI_readin`（读 `TREE_LAI`/`TREE_SAI`）。
            let year = |key: &str| -> Result<i32> {
                i32::try_from(integer_field(document, key)?)
                    .with_context(|| format!("{key} does not fit an i32"))
            };
            template = template.with_monthly_leaf_area_index(MonthlyLeafAreaIndex::read_urban(
                &path,
                change_yearly,
                land_cover_year,
                year("DEF_LAI_START_YEAR")?,
                year("DEF_LAI_END_YEAR")?,
            )?);
        } else if template.pft.is_some() {
            // `LAI_readin` 的 PFT 段：逐 PFT 读 `LAI_pfts_monthly`，patch 值取聚合。
            let year = |key: &str| -> Result<i32> {
                i32::try_from(integer_field(document, key)?)
                    .with_context(|| format!("{key} does not fit an i32"))
            };
            template = template.with_pft_monthly_leaf_area_index(
                &path,
                logical_field(document, "USE_SITE_LAI")?,
                change_yearly,
                land_cover_year,
                (year("DEF_LAI_START_YEAR")?, year("DEF_LAI_END_YEAR")?),
            )?;
        } else if logical_field(document, "DEF_URBAN_RUN")? {
            // 城市单点里的非城市 patch：`LAI_readin` 的单点分支被 `.not. DEF_URBAN_RUN`
            // 整个跳过，`tlai`/`tsai` 保持重启值 —— 不装读取器就是这个行为。
        } else {
            template = template.with_monthly_leaf_area_index(MonthlyLeafAreaIndex::read(
                &path,
                logical_field(document, "USE_SITE_LAI")?,
                change_yearly,
                land_cover_year,
            )?);
        }
    }

    // `scale_baseflow`：上游 `Opt_Baseflow_init` 从
    // `DEF_dir_restart/ParaOpt/<case>_baseflow.nc` 读一个长度 `landpatch` 的向量，
    // 文件或变量缺失时取 `defval = 1.`（`MOD_Opt_Baseflow.F90:37-38`）。
    // 它直接乘在 `rsubst`/`rsub` 上，参数标定过的算例差别是物理量级的。
    let baseflow_scale = read_baseflow_scale(layout, name, patch, block)?;
    template = template.with_baseflow_scale(baseflow_scale);
    // `DEF_USE_SNICAR`：`SnowOptics_init`/`SnowAge_init` 读 `DEF_dir_runtime/snicar/` 下的两张表。
    if template.physics.snicar {
        let tables = colm_init::SnicarInitialization::from_document(document)?
            .context("DEF_USE_SNICAR is on but its tables were not loaded")?;
        // `AerosolDepInit`：本 patch 所在网格（单点就是含站点的那一格）。
        let aerosol = if template.physics.aerosol_readin {
            let constant = colm_init::RestartFile::open(&files.constant)?;
            let pick = |name: &str| -> Result<f64> {
                constant.floats(name)?.get(patch).copied().with_context(|| {
                    format!("the constant restart has no {name} for patch {patch}")
                })
            };
            Some(colm_runtime::aerosol::AerosolSource::open(
                std::path::Path::new(&string_field(document, "DEF_dir_runtime")?),
                pick("patchlatr")?.to_degrees(),
                pick("patchlonr")?.to_degrees(),
                template.physics.aerosol_climatology,
            )?)
        } else {
            None
        };
        template = template.with_snicar_tables(std::sync::Arc::new(tables), aerosol);
    }
    Ok(template)
}

/// `DEF_USE_BGC` 的运行期：BGC 重启、PFT 常数、BGC 用的静态量与氮沉降。
fn assemble_bgc(
    document: &Document,
    files: &RestartStateFiles,
    patch: usize,
    switches: colm_core::bgc_driver::BgcSwitches,
) -> Result<(
    colm_runtime::bgc_step::BgcRuntime,
    Option<colm_runtime::irrigation::IrrigationReadin>,
)> {
    // 各份 BGC 重启切出本 patch 与它的 PFT 区间（`colm_runtime::pft::open_patch`）。
    let pft_constant = colm_runtime::pft::pft_restart_path(&files.constant)?;
    let (patches, pfts) =
        { colm_runtime::pft::patch_and_pft_counts(&colm_init::RestartFile::open(&pft_constant)?)? };
    let pft_time = colm_runtime::pft::pft_restart_path(&files.time)?;
    let mut initial = colm_runtime::bgc::BgcTemplate::read(
        &files.constant,
        &files.time,
        &pft_time,
        patch,
        patches,
        pfts,
    )?
    .initial;
    let layers = initial.dims.nl_soil;
    let statics = colm_runtime::bgc_step::BgcStatics::read(&files.constant, patch, layers)?;
    let runtime_dir = std::path::PathBuf::from(string_field(document, "DEF_dir_runtime")?);
    let degrees = |radians: f64| radians * 180.0 / std::f64::consts::PI;
    // `CROP_readin`（`CoLM.F90:442`）：启动时覆盖作物的播种日与施肥量（与灌溉方式）。
    let mut irrigation = None;
    if switches.crop {
        let classes = colm_runtime::pft::open_patch(&pft_constant, patch, patches, pfts)?
            .integers("pftclass")?
            .iter()
            .map(|&class| i32::try_from(class))
            .collect::<Result<Vec<_>, _>>()?;
        irrigation = colm_runtime::bgc_step::crop_readin(
            &mut initial,
            &classes,
            real_field(document, "DEF_TUNING_CROP_PLANTING_DAY")?,
            switches,
            colm_runtime::bgc_step::CropReadinData {
                runtime_dir: &runtime_dir,
                latitude_deg: degrees(statics.patchlatr),
                longitude_deg: degrees(statics.patchlonr),
                fert_source: integer_field(document, "DEF_FERT_SOURCE")?,
                irrigation_allocation: i32::try_from(integer_field(
                    document,
                    "DEF_IRRIGATION_ALLOCATION",
                )?)?,
            },
        )?;
    }
    // `DEF_NDEP_FREQUENCY`：1 年度、2 月度；其余值上游 `CoLM_stop`（`CoLM.F90:425-428`）。
    let monthly_ndep = match integer_field(document, "DEF_NDEP_FREQUENCY")? {
        1 => false,
        2 => true,
        other => bail!("DEF_NDEP_FREQUENCY should be only 1-2, got {other} (upstream stops)"),
    };
    let ndep = colm_runtime::bgc_step::NdepSource::open(
        &runtime_dir,
        degrees(statics.patchlatr),
        degrees(statics.patchlonr),
        logical_field(document, "DEF_USE_PN")?,
        monthly_ndep,
    )?;
    // `init_ndep_data_*(sdate(1), …)`：`sdate` 经过 `adj2end`，00:00 的 1 月 1 日起步算上一年。
    let year = i32::try_from(integer_field(document, "DEF_simulation_time%start_year")?)?;
    let month = integer_field(document, "DEF_simulation_time%start_month")?;
    let day = integer_field(document, "DEF_simulation_time%start_day")?;
    let second = integer_field(document, "DEF_simulation_time%start_sec")?;
    let ndep_start_year = if month == 1 && day == 1 && second == 0 {
        year - 1
    } else {
        year
    };
    let deltim = real_field(document, "DEF_simulation_time%timestep")?;
    // `init_nitrif_data(ststamp)`：起始时刻（未经 adj2end）所在的月。
    let nitrif = if switches.nitrif {
        Some((
            colm_runtime::bgc_step::NitrifSource::open(
                &runtime_dir,
                degrees(statics.patchlatr),
                degrees(statics.patchlonr),
                layers,
            )?,
            u8::try_from(month).context("DEF_simulation_time%start_month is not a month")?,
        ))
    } else {
        None
    };
    // `init_fire_data`：`DEF_dir_runtime/fire/` 的静态场与逐年人口密度、3 小时闪电。
    let fire = if switches.fire {
        Some(colm_runtime::bgc_step::FireSource::open(
            &runtime_dir,
            degrees(statics.patchlatr),
            degrees(statics.patchlonr),
        )?)
    } else {
        None
    };
    colm_runtime::bgc_step::BgcRuntime::new(
        initial,
        colm_runtime::bgc::bgc_pft_constants(document)?,
        switches,
        statics,
        colm_runtime::bgc_step::BgcDataSources {
            ndep,
            ndep_start_year,
            ndep_start_month: u8::try_from(month)
                .context("DEF_simulation_time%start_month is not a month")?,
            nitrif,
            fire,
        },
        deltim,
    )
    .map(|runtime| (runtime, irrigation))
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
    templates: &[StandardLctRestartTemplate],
    restart_in: &Path,
    restart_out: &Path,
    periodic: Option<&PeriodicRestarts>,
    session: Option<HistorySession>,
    history_restart: &HistoryRestart,
) -> Result<RunSummary> {
    let mut states: Vec<StandardLctSnowSoilState> = templates
        .iter()
        .map(StandardLctRestartTemplate::snow_state)
        .collect();
    // `smp`/`hk` 与表面诊断量只出现在步输出里（`intent(out)`），而续跑要写它们。
    let mut last: Option<Vec<RestartSnapshot>> = None;
    let mut on_step = |step: colm_runtime::PointRuntimeStep,
                       states: &[StandardLctSnowSoilState],
                       outputs: &[PatchStepOutput<'_>]|
     -> Result<()> {
        let snapshots = states
            .iter()
            .zip(outputs)
            .map(|(state, output)| {
                let mut snapshot =
                    RestartSnapshot::new(state, *output, step.surface_cosine_zenith)?;
                snapshot.lai_refreshed = step.clock.update_lai;
                Ok(snapshot)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut snapshots = snapshots;
        if let Some(previous) = &last {
            for (snapshot, previous) in snapshots.iter_mut().zip(previous) {
                snapshot.diagnostics.carry_forward(&previous.diagnostics);
            }
        }
        // `save_to_restart`（`CoLM.F90:664`）：每个 `DEF_WRST_FREQ` 周期末、以及预热期
        // 每年末写一次 `WRITE_TimeVariables`。窗口终点那一次由循环结束后的写出负责。
        if let Some(periodic) = periodic {
            if step.clock.write_restart {
                let path = periodic.path(step.clock.end_time);
                if path != restart_out {
                    write_evolved_restart(templates, states, &snapshots, restart_in, &path)?;
                    mark_history_restart(&path, history_restart)?;
                }
            }
        }
        last = Some(snapshots);
        Ok(())
    };
    let (steps, history_files) = match session {
        Some(mut session) => {
            let outcome = runtime.run_restart_standard_lct_snow_with_history(
                templates,
                &mut states,
                &mut session,
                &mut on_step,
            )?;
            (outcome.steps, Some(outcome.files.len()))
        }
        None => (
            runtime.run_restart_standard_lct_snow(templates, &mut states, &mut on_step)?,
            None,
        ),
    };
    let last = last.context(NO_STEP)?;
    write_evolved_restart(templates, &states, &last, restart_in, restart_out)?;
    mark_history_restart(restart_out, history_restart)?;
    Ok(RunSummary {
        steps,
        history_files,
    })
}

/// 写续跑文件时要附带的历史累加器信息（`land_history_restart.inc`）。
struct HistoryRestart {
    config: colm_runtime::history_sidecar::SidecarConfig,
    /// 当前历史区间的原始累加状态；没开 history 时为 `None`（恒为空窗口）。
    window:
        Option<std::sync::Arc<std::sync::Mutex<Vec<colm_runtime::history_sidecar::HistoryWindow>>>>,
}

/// 续跑文件 → 同目录的旁车路径。
fn history_sidecar_path(restart: &Path) -> Result<PathBuf> {
    let name = restart
        .file_name()
        .and_then(|name| name.to_str())
        .context("a restart path has no file name")?;
    Ok(restart.with_file_name(colm_runtime::history::history_sidecar_name(name)?))
}

fn history_frequency_code(frequency: colm_hist::schedule::HistoryFrequency) -> u8 {
    use colm_hist::schedule::HistoryFrequency as F;
    match frequency {
        F::None => 0,
        F::Timestep => 1,
        F::Hourly => 2,
        F::Daily => 3,
        F::Monthly => 4,
        F::Yearly => 5,
    }
}

/// 上游每写一份续跑文件，都同时写历史累加器旁车 `<case>_restart_hist_<date>_<block>.nc`
/// （`write_history_acc_restart` + `complete_history_acc_restart`），并在主重启末尾写
/// `history_sidecar_required = 1`（`mark_history_acc_restart`）。区间跨过重启时旁车带全部
/// 已分配的累加器（[`colm_runtime::history_sidecar`]）。
fn mark_history_restart(restart: &Path, history: &HistoryRestart) -> Result<()> {
    mark_history_restart_with_river(restart, history, None, false)
}

/// [`mark_history_restart`]；空间构建另带 `history_river_required`。
/// `block` 是这个续跑文件在全部 patch 里的区间（多分块空间算例）；`None` 为整份窗口。
fn mark_history_restart_with_river(
    restart: &Path,
    history: &HistoryRestart,
    block: Option<std::ops::Range<usize>>,
    river_required: bool,
) -> Result<()> {
    let mut windows = history
        .window
        .as_ref()
        .map(|window| window.lock().expect("history window lock").clone())
        .unwrap_or_else(|| vec![colm_runtime::history_sidecar::HistoryWindow::default()]);
    if let Some(block) = block {
        if windows.len() > 1 {
            windows = windows[block].to_vec();
        }
    }
    let patches = colm_init::RestartFile::open(restart)?.dimension("patch")?;
    colm_runtime::history_sidecar::write_sidecar_with_river(
        &history_sidecar_path(restart)?,
        patches,
        &history.config,
        &windows,
        river_required,
    )?;
    let mut primary =
        netcdf::append(restart).with_context(|| format!("cannot reopen {}", restart.display()))?;
    // 续跑起点本身就是带标记的续跑文件时，写出是从它复制来的，标记已经在了。
    match primary.variable_mut("history_sidecar_required") {
        Some(mut marker) => marker.put_values(&vec![1.0; patches], ..)?,
        None => primary
            .add_variable::<f64>("history_sidecar_required", &["patch"])?
            .put_values(&vec![1.0; patches], ..)?,
    }
    Ok(())
}

/// 续跑写出要的、状态里没有的那部分步输出。
struct RestartSnapshot {
    matric_potential_mm: Vec<f64>,
    hydraulic_conductivity_mm_s: Vec<f64>,
    diagnostics: SurfaceDiagnosticsRow,
    /// 这一步末尾是否重读了 LAI（见 `EvolvedStepOutput::lai_refreshed`）。
    lai_refreshed: bool,
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
                lai_refreshed: false,
            },
            // 冰川分支不调 `soilwater`：`smp`/`hk` 保持重启里的值。
            PatchStepOutput::Glacier(output) => Self {
                matric_potential_mm: state.soil_water.matric_potential_mm.clone(),
                hydraulic_conductivity_mm_s: state.soil_water.hydraulic_conductivity_mm_s.clone(),
                diagnostics: SurfaceDiagnosticsRow::from_glacier(&output.thermal, cosine_zenith),
                lai_refreshed: false,
            },
            // 湖同样不调 `soilwater`。
            // 城市：透水地面的 `WATER_2014` 已把 `smp`/`hk` 写进状态。
            PatchStepOutput::Urban(output) => Self {
                matric_potential_mm: state.soil_water.matric_potential_mm.clone(),
                hydraulic_conductivity_mm_s: state.soil_water.hydraulic_conductivity_mm_s.clone(),
                diagnostics: SurfaceDiagnosticsRow::from_urban(output, cosine_zenith),
                lai_refreshed: false,
            },
            PatchStepOutput::Lake(output) => Self {
                matric_potential_mm: state.soil_water.matric_potential_mm.clone(),
                hydraulic_conductivity_mm_s: state.soil_water.hydraulic_conductivity_mm_s.clone(),
                diagnostics: SurfaceDiagnosticsRow::from_lake(&output.thermal, cosine_zenith),
                lai_refreshed: false,
            },
        })
    }
}

/// 把步末状态写成一份续跑文件（以输入重启为底，只替换推进过的变量）。
///
/// 每个 patch 的模板各出一份覆盖量，再按 `patch`/`pft` 维拼成整变量
/// （[`colm_runtime::multi_patch::merge_overrides`]）。
fn write_evolved_restart(
    templates: &[StandardLctRestartTemplate],
    states: &[StandardLctSnowSoilState],
    snapshots: &[RestartSnapshot],
    restart_in: &Path,
    restart_out: &Path,
) -> Result<()> {
    ensure!(
        templates.len() == states.len() && templates.len() == snapshots.len(),
        "every written patch needs a template, a state, and a snapshot"
    );
    let source = colm_init::RestartFile::open(restart_in)?;
    let pft_in = colm_runtime::pft::pft_restart_path(restart_in)?;
    let pft_source = templates[0]
        .pft
        .as_ref()
        .map(|_| colm_init::RestartFile::open(&pft_in))
        .transpose()?;
    let slots = templates
        .iter()
        .map(|template| {
            let pfts = match &pft_source {
                // PFT 时间重启没有 `patch` 维：patch 数取主重启的。
                Some(file) => colm_runtime::pft::patch_pft_range(
                    source.dimension("patch")?,
                    file.dimension("pft")?,
                    template.patch,
                )?,
                None => 0..0,
            };
            Ok(colm_runtime::multi_patch::PatchSlot {
                patch: template.patch,
                pfts,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let merge = |file: &colm_init::RestartFile, lists: Vec<Vec<colm_init::RestartOverride>>| {
        colm_runtime::multi_patch::merge_overrides(file, &slots, lists)
    };

    // 主重启。`t_grnd` 是雪层合并之后重取的那个（`CoLMMAIN.F90:1452`）。
    let mut lists = Vec::with_capacity(templates.len());
    for ((template, state), snapshot) in templates.iter().zip(states).zip(snapshots) {
        let mut overrides = template.evolved_snow_overrides(
            state,
            EvolvedStepOutput {
                ground_temperature_k: state.surface_temperature_k(),
                matric_potential_mm: &snapshot.matric_potential_mm,
                hydraulic_conductivity_mm_s: &snapshot.hydraulic_conductivity_mm_s,
                diagnostics: snapshot.diagnostics,
                lai_refreshed: snapshot.lai_refreshed,
            },
        )?;
        // `DEF_USE_IRRIGATION`：灌溉量与 `irrig_method_*`（后者是作物汇总写的 patch 量）。
        if let Some(irrigation) = &state.irrigation {
            let diagnostics = state
                .bgc
                .as_ref()
                .context("the irrigation state needs the CROP BGC state")?
                .irrigation_diagnostics;
            overrides.extend(colm_runtime::irrigation::time_overrides(
                irrigation,
                &diagnostics,
                &source,
                template.patch,
            )?);
        }
        lists.push(overrides);
    }
    let overrides = merge(&source, lists)?;
    if let Some(parent) = restart_out.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    write_restart(restart_in, restart_out, overrides.as_slice())?;

    // PFT 子网格另有一份时间重启（`<case>_restart_pft_<date>_…nc`），与主重启同目录。
    if let Some(pft_source) = &pft_source {
        let mut lists = Vec::with_capacity(templates.len());
        for (template, state) in templates.iter().zip(states) {
            let (Some(pft_template), Some(pft)) = (&template.pft, &state.energy.pft) else {
                anyhow::bail!("patch {} has no PFT subgrid to write back", template.patch);
            };
            let mut overrides = pft_template.overrides(pft);
            // `WRITE_PFTimeVariables` 在 BGC 下把 `WRITE_BGCPFTimeVariables` 写进同一份文件。
            if let Some(bgc) = &state.bgc {
                overrides.extend(colm_runtime::bgc::BgcTemplate::overrides(bgc, pft_source));
            }
            if let Some(irrigation) = &state.irrigation {
                overrides.push(colm_runtime::irrigation::pft_override(irrigation));
            }
            lists.push(overrides);
        }
        let overrides = merge(pft_source, lists)?;
        write_restart(
            &pft_in,
            &colm_runtime::pft::pft_restart_path(restart_out)?,
            &overrides,
        )?;
    }
    // BGC 的 patch 级时间变量另有一份（`<case>_restart_bgc_<date>_…nc`）。
    if states[0].bgc.is_some() {
        let bgc_in = colm_runtime::bgc::bgc_time_path(restart_in)?;
        let bgc_source = colm_init::RestartFile::open(&bgc_in)?;
        let lists = states
            .iter()
            .map(|state| {
                let bgc = state
                    .bgc
                    .as_ref()
                    .context("every BGC patch needs its BGC state")?;
                Ok(colm_runtime::bgc::BgcTemplate::overrides(bgc, &bgc_source))
            })
            .collect::<Result<Vec<_>>>()?;
        let overrides = merge(&bgc_source, lists)?;
        write_restart(
            &bgc_in,
            &colm_runtime::bgc::bgc_time_path(restart_out)?,
            &overrides,
        )?;
    }
    // 城市单元另有一份时间重启（`<case>_restart_urban_<date>_…nc`），与主重启同目录。
    if let (Some(urban_template), Some(urban)) = (&templates[0].urban, &states[0].urban) {
        ensure!(
            templates.len() == 1,
            "urban sites are single-patch; {} patches were run",
            templates.len()
        );
        let template = &templates[0];
        let state = &states[0];
        let urban_path = |path: &Path| -> Result<std::path::PathBuf> {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .context("a restart path has no file name")?;
            Ok(path.with_file_name(name.replacen("_restart_", "_restart_urban_", 1)))
        };
        // `UrbanLAI_readin` 同时写 `urb_lai(u)` 与 `tlai(npatch)`（两者恒等），所以装了城市月度
        // LAI 时 `tree_lai`/`tree_sai` 就是当前的 `tlai`/`tsai`；没装时二者都停在重启值。
        let tree = template
            .monthly_leaf_area_index
            .as_ref()
            .filter(|lai| lai.is_urban())
            .map(|_| {
                (
                    state.energy.temporal_canopy.leaf_area_index,
                    state.energy.temporal_canopy.stem_area_index,
                )
            });
        let overrides = urban_template.overrides(urban, tree)?;
        write_restart(
            &urban_path(restart_in)?,
            &urban_path(restart_out)?,
            overrides.as_slice(),
        )?;
    }
    Ok(())
}

/// 周期续跑文件的落点：与窗口终点那份同一套命名，`cdate` 取该步的 `jdate`
/// （步末 `idate` 经 `adj2begin`，即 86400 秒写成次日 0 秒）。
struct PeriodicRestarts {
    directory: PathBuf,
    name: String,
    land_cover_year: i64,
    /// 分块后缀：单点是 `w180_s90`，空间算例是 `e110_n20` 这样的块名。
    block: String,
}

impl PeriodicRestarts {
    fn path(&self, end_time: CalendarTime) -> PathBuf {
        let label = date_label(normalized_day_end(end_time));
        self.directory.join(&label).join(format!(
            "{}_restart_{label}_lc{:04}_{}.nc",
            self.name, self.land_cover_year, self.block
        ))
    }
}

/// 单点算例的分块后缀（`get_filename_block` 对单点给的块名）。
const SINGLE_POINT_BLOCK: &str = "w180_s90";

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
    // `DEF_HIST_FREQ = 'none'`（声明默认值）：上游 `hist_out` 走 `CASE default`，从不写文件，
    // 但照样累加 —— 续跑旁车要这个窗口，所以仍建一个只累加的会话（空调度）。
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
                        block: SINGLE_POINT_BLOCK.to_owned(),
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
fn read_baseflow_scale(
    layout: &colm_case::Layout,
    name: &str,
    patch: usize,
    block: &str,
) -> Result<f64> {
    // 文件名与重启同一套块后缀约定：`MOD_Block.F90:641-645` 的
    // `get_filename_block` 把 `_<block>` 插在 `.nc` 之前，单点算例是 `w180_s90`。
    // 写成不带后缀的 `..._baseflow.nc` 内核根本不会读（会打 "not found" 走默认值）。
    let path = layout
        .out()
        .join(name)
        .join("restart/ParaOpt")
        .join(format!("{name}_baseflow_{block}.nc"));
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

/// 取一个实数字段：算例里写了就用算例的，否则用 schema 的声明默认值。
fn real_field(document: &Document, field: &str) -> Result<f64> {
    if let Some(value) = document.get(field) {
        return value
            .as_f64()
            .with_context(|| format!("{field} must be a real value, got {value:?}"));
    }
    match colm_schema::find(field).map(|field| &field.default) {
        Some(colm_schema::Default::Real(text)) => text
            .replace(['d', 'D'], "e")
            .parse()
            .with_context(|| format!("{field} has an unreadable default {text}")),
        _ => bail!("{field} is missing from the case namelist and has no real default"),
    }
}

/// 取一个字符字段：算例里写了就用算例的，否则用 schema 的声明默认值。
fn string_field(document: &Document, field: &str) -> Result<String> {
    if let Some(value) = document.get(field) {
        return match value {
            Value::Str(value) => Ok(value.clone()),
            other => bail!("{field} must be a character value, got {other:?}"),
        };
    }
    match colm_schema::find(field).map(|field| &field.default) {
        Some(colm_schema::Default::Str(value)) => Ok((*value).to_owned()),
        _ => bail!("{field} is missing from the case namelist and has no character default"),
    }
}

struct Arguments {
    case_directory: PathBuf,
    /// 只跑这一个 patch；缺省跑全部（多作物单点）。
    patch: Option<usize>,
    land_cover: LandCoverScheme,
    outputs: OutputSpec,
    /// 只做能力检查就退出，不读重启、不推进。
    preflight: bool,
    /// 显式允许跑"本仓库没实现的那些分支"。默认关。
    allow_unported_branches: bool,
    /// 内核带 `CROP` 宏（`DEF_USE_CROP` 是它的只读映射，namelist 里没有）。
    crop: bool,
}

impl Arguments {
    fn parse(arguments: impl Iterator<Item = String>) -> Result<Self> {
        let mut values = arguments.peekable();
        let mut case_directory = None;
        let mut patch = None;
        let mut land_cover = None;
        let mut restart_out = None;
        let mut history_directory = None;
        let mut history_stem = None;
        let mut allow_unported_branches = false;
        let mut case_outputs = false;
        let mut preflight = false;
        let mut crop = false;
        while let Some(flag) = values.next() {
            let mut value = |name: &str| -> Result<String> {
                values
                    .next()
                    .with_context(|| format!("{name} needs a value"))
            };
            match flag.as_str() {
                "--patch" => {
                    patch = Some(
                        value("--patch")?
                            .parse()
                            .context("--patch must be a nonnegative integer")?,
                    );
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
                "--crop" => crop = true,
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
            crop,
        })
    }
}

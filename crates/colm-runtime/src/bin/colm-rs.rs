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
    install_history_selection(
        &read_document(&case_nml)?,
        &arguments.case_directory,
        arguments.crop,
    )?;
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
            mode: config.observation_height_mode,
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
                None,
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
    // 方案 5 的 GIEMS 按 patch 中心取像元（单点：常数重启里的站点坐标）。
    let patch_indices: Vec<usize> = templates.iter().map(|template| template.patch).collect();
    methane_giems(&document, &mut templates, || {
        let constant = colm_init::RestartFile::open(&files.constant)?;
        let (lon, lat) = (constant.floats("patchlonr")?, constant.floats("patchlatr")?);
        patch_indices
            .iter()
            .map(|&patch| -> Result<(f64, f64)> {
                Ok((
                    *lon.get(patch).context("patchlonr is too short")?,
                    *lat.get(patch).context("patchlatr is too short")?,
                ))
            })
            .collect()
    })?;
    // `land_tracer_init`（`CoLM.F90:339`）：注册示踪物；有输运示踪物时读续跑里的示踪物事务，
    // 读不到（冷启动重启只有空事务）就按水量冷启动。
    // `tracer_forcing_init`：POINT 不支持示踪物强迫变量（`MOD_Tracer_Forcing.F90`）。
    let tracer_runtime = colm_runtime::tracer::TracerRuntime::from_document(&document)?
        .map(|mut tracer| -> Result<_> {
            // 泥沙是河网汇流上的示踪物，单点内核不编进 `GridRiverLakeFlow`，上游没有它的 provider。
            ensure!(
                !tracer
                    .set
                    .tracers
                    .iter()
                    .any(colm_runtime::river::sediment::is_sediment_tracer),
                "SEDIMENT needs GridRiverLakeFlow, which single-point kernels do not build"
            );
            tracer.configure_forcing(None)?;
            Ok(std::sync::Arc::new(tracer))
        })
        .transpose()?;
    if let Some(tracer) = tracer_runtime
        .as_ref()
        .filter(|tracer| tracer.has_transport())
    {
        let restart = colm_init::RestartFile::open(&files.time)?;
        templates = attach_land_tracers(tracer, templates, &restart, patch_count)?;
    }
    restore_sidecar_tracers(
        &mut templates,
        &history_sidecar_path(&restarts.initial)?,
        patch_count,
        |_, template| template.patch,
    )?;
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
        Some(BaseflowOptimizer::new(
            &patches, para_opt, &name, "w180_s90",
        ))
    } else {
        None
    };
    let template = &templates[0];

    // 会话从**配置**开（窗口、站点、步长、频率都在里面），要在 `open` 消费掉
    // 配置之前建好 —— 而它自己不带 forcing，所以先后没有别的影响。
    let mut session = history_session(&config, &outputs)?
        .map(|session| session.with_patches(templates.len()))
        .transpose()?;
    // 开示踪物时另写 `<case>_hist_tracer_<cdate>.nc`（没有输运示踪物时只有文件骨架）。
    if let Some(tracer) = &tracer_runtime {
        let patch_types = templates
            .iter()
            .map(|template| template.patch_type)
            .collect();
        session =
            session.map(|session| session.with_tracer_variables(tracer.set.clone(), patch_types));
    }
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
    let urban_flags = templates
        .iter()
        .map(|template| template.urban.is_some())
        .collect::<Vec<_>>();
    let initial_window = colm_runtime::history_sidecar::read_sidecar(
        &restarts.initial,
        &history_sidecar_path(&restarts.initial)?,
        &sidecar_config,
        &urban_flags,
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
        tracer_raw: session.as_ref().map(HistorySession::tracer_raw_handle),
        urban: urban_flags,
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
    let downscaling =
        match colm_runtime::spatial::downscaling::DownscalingSettings::from_case(&document)? {
            Some(settings) => Some(point_downscaling(
                settings,
                &files.constant,
                &patches,
                config.temperature_height_m,
            )?),
            None => None,
        };
    let mut runtime = PointRuntime::open(config)?;
    if let Some(downscaling) = downscaling {
        runtime = runtime.with_downscaling(downscaling);
    }
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

/// 单点降尺度：从常数重启读本站点 patch 的 `elvmean` 与地形因子（`sf_lut_patches`）。
fn point_downscaling(
    settings: colm_runtime::spatial::downscaling::DownscalingSettings,
    constant_path: &Path,
    patches: &[usize],
    reference_height_m: f64,
) -> Result<colm_runtime::spatial::downscaling::PointDownscaling> {
    let constant = colm_init::RestartFile::open(constant_path)?;
    let count = constant.dimension("patch")?;
    let patch_types = constant
        .integers("patchtype")?
        .iter()
        .map(|&kind| i32::try_from(kind))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mask = if constant.contains("patchmask") {
        constant
            .integers("patchmask")?
            .iter()
            .map(|&mask| mask != 0)
            .collect()
    } else {
        vec![true; count]
    };
    let all = colm_runtime::spatial::downscaling::read_block_patches(
        &constant,
        settings.simple,
        &patch_types,
        &mask,
    )?;
    colm_runtime::spatial::downscaling::PointDownscaling::new(
        settings,
        patches.iter().map(|&patch| all[patch].clone()).collect(),
        reference_height_m,
    )
}

/// 空间算例（`GRIDBASED`）：patch 拓扑来自 `landdata`，强迫是网格强迫经面积加权映射到每个 patch。
///
/// 物理与单点完全相同（同一个 `advance_patch`）；history 写成经纬网格（`HistForm = 'Gridded'`），
/// 河湖流走 `GridRiverLakeFlow` 的默认汇流。还没有城市/PFT 的网格 LAI，遇到就拒绝或明说。
///
/// `DEF_USE_LULCC`：运行在每个年末切段，段间按新一年的土地覆盖冷启动、用 SAT 接回旧状态
/// （上游 `LulccDriver`），下一段从合并出来的续跑接着跑。
fn run_spatial(
    arguments: &Arguments,
    layout: &colm_case::Layout,
    name: &str,
    case_nml: &Path,
) -> Result<()> {
    use colm_runtime::spatial::runtime::SpatialRuntimeConfig;
    ensure!(
        arguments.patch.is_none(),
        "--patch selects a patch of a single point; spatial cases run every patch"
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
            mode: config.forcing.height_mode,
        },
    )?;
    // `CROP` 内核（与单点同一套开关，见 `run`）。空间的 `CROP_readin` 按网格映射读播种日、施肥图与
    // 灌溉方式（`grid2pset_dominant`）；上游在空间里拒绝播种日覆盖（"only supported in SinglePoint"）。
    if !arguments.crop {
        physics.irrigation = None;
    }
    if arguments.crop {
        let switches = physics
            .bgc
            .context("CROP kernels need DEF_USE_BGC (the crop state lives in the BGC restarts)")?;
        let fert = logical_field(&document, "DEF_USE_FERT")?;
        ensure!(
            real_field(&document, "DEF_TUNING_CROP_PLANTING_DAY")? <= 0.0,
            "Fatal ERROR: crop planting-day override is only supported in SinglePoint (upstream stops too)"
        );
        physics.bgc = Some(colm_core::bgc_driver::BgcSwitches {
            crop: true,
            fert,
            cnsoyfixn: logical_field(&document, "DEF_USE_CNSOYFIXN")?,
            irrigation: physics.irrigation.is_some(),
            ..switches
        });
    }
    ensure!(
        !(arguments.catchment && arguments.unstructured),
        "--catchment and --unstructured name different kernels"
    );
    // GRID/UNSTRUCTURED 内核总是编进 `GridRiverLakeFlow`，它改变了几处收缩形状。CATCHMENT 内核
    // 编进的是 `CatchLateralFlow`：上游强制变饱和流与动态湖（`MOD_Namelist.F90:1879-1884`、
    // `:2445-2459`），柱内不产流。
    physics.river_lake_flow_build = !arguments.catchment;
    if arguments.catchment {
        check_catchment(&document)?;
        physics.catch_lateral = true;
        physics.variably_saturated_flow = true;
        physics.dynamic_lake = true;
    }
    // 单点把 `DEF_TOPMOD_method` 强制为 0；空间构建照 namelist（`MOD_Namelist.F90:1899-1904`）。
    physics.topmodel_method = colm_runtime::physics::topmodel_method(&document)?;
    let missing = colm_runtime::physics::unported_branches(&physics);
    ensure!(
        missing.is_empty() || arguments.allow_unported_branches,
        "this case needs {} branch(es) the Rust runtime does not implement:\n  - {}",
        missing.len(),
        missing.join("\n  - ")
    );
    // GRID 内核总是编进 `GridRiverLakeFlow`：汇流默认路径（单向耦合，`FloodplainStorageFix` 两种曲线都行），其余选项还没移植。
    // 漫滩回馈：上游自己要求修正漫滩曲线、不与 LULCC 同开，并把产流方案强制成 0；
    // Rust 只接变饱和流、LCT 的那条路径。
    if logical_field(&document, "DEF_GridRiverLake_FloodFeedback")? {
        ensure!(
            logical_field(&document, "DEF_GridRiverLake_FloodplainStorageFix")?,
            "Grid flood feedback requires DEF_GridRiverLake_FloodplainStorageFix (upstream stops too)"
        );
        ensure!(
            !logical_field(&document, "DEF_USE_LULCC")?,
            "Grid flood feedback does not support LULCC (upstream stops too)"
        );
        ensure!(
            integer_field(&document, "DEF_Runoff_SCHEME")? == 0,
            "Grid flood feedback forces DEF_Runoff_SCHEME = 0 upstream; set it in the namelist"
        );
        ensure!(
            physics.variably_saturated_flow && !physics.use_pft && !physics.urban_run,
            "Grid flood feedback is ported for LCT variably saturated flow only"
        );
    }
    let reservoir_method = integer_field(&document, "DEF_Reservoir_Method")?;
    ensure!(
        reservoir_method <= 1,
        "unsupported reservoir operation method {reservoir_method}"
    );
    // 堤防、分汊、水库与漫滩回馈之间可以任意组合，也都可以与 LULCC 同开（漫滩回馈除外，
    // 上游自己拒绝，见上）。
    // 示踪物：陆面输运、河湖输运、网格示踪物强迫、漫滩回馈的示踪物账、LULCC（SAT）的示踪物迁移
    // 都已移植。还没接的组合在下面逐条拒绝，免得悄悄丢账。
    let tracer_set = colm_runtime::tracer::tracer_set_from_document(&document)?;
    // CH4 provider：与单点同一条 `soil_step`。空间内核编进了网格河湖，所以 `routing`/`hybrid` 可用；
    // `satellite`（GIEMS）由 `methane_giems` 读入；`only_wetland` 与稻田改的活跃掩膜由 `soil_step`
    // 每步写进 `MethanePatch::history_active`。这里先解析一遍配置，坏配置在入口就停。
    colm_runtime::methane::setup_from_document(&document, true)?;
    // 输运示踪物跨 LULCC：陆面状态见 `lulcc_land_tracers`（SAT/MEC），强迫缓存见 `lulcc_forcing_cache`。
    let methane_tracer = tracer_set.as_ref().is_some_and(|set| {
        set.tracers
            .iter()
            .any(colm_runtime::methane::is_methane_tracer)
    });
    ensure!(
        !(methane_tracer && logical_field(&document, "DEF_USE_LULCC")?),
        "methane with DEF_USE_LULCC: methane needs BGC, and upstream does not support LULCC with \
         BGC (MOD_Namelist stops too)"
    );
    // `MOD_Namelist.F90:1670-1679`：区域单元流域要 GridRiverLakeFlow（空间内核总有）且不能开 LULCC。
    ensure!(
        !(logical_field(&document, "DEF_UnitCatchment_regional")?
            && logical_field(&document, "DEF_USE_LULCC")?),
        "Regional unit catchment requires GridRiverLakeFlow without LULCC (upstream stops too)"
    );
    let lulcc = logical_field(&document, "DEF_USE_LULCC")?;
    if lulcc {
        check_spatial_lulcc(&document, &config, arguments.land_cover)?;
    }
    if arguments.preflight {
        println!("colm-rs preflight: ok");
        return Ok(());
    }
    let out = layout.out().join(name);
    // 上游 `CoLM.F90`：开了 LULCC 时土地覆盖年份取起始年，不看 `DEF_LC_YEAR`。
    let year = if lulcc {
        i64::from(config.start.year)
    } else {
        integer_field(&document, "DEF_LC_YEAR")?
    };
    let vector_history =
        arguments.unstructured && logical_field(&document, "DEF_HISTORY_IN_VECTOR")?;
    let case = SpatialCase {
        layout,
        name,
        document: &document,
        physics: &physics,
        out: &out,
        vector_history,
        catchment: arguments.catchment,
    };
    let restart_root = out.join("restart");
    let scratch = restart_root.join(LULCC_SCRATCH);
    // 多遍预热跨过 LULCC 年末（upstream-bugs 第 50 条，两侧都已修）：上游每轮回卷时把土地覆盖
    // 换回起始年（SAT）。Rust 把每一轮拆成一条段链：前几轮从起点跑到预热终点、换回起始年，
    // 最后一轮从起点跑到运行终点；每轮内都只预热一遍（不再由时钟回卷）。
    let rewinds = if lulcc && spinup_crosses_lulcc(&config) {
        config.spinup_repeats - 1
    } else {
        0
    };
    let mut input = restart_root.clone();
    let mut carried = Carried::default();
    for pass in 0..=rewinds {
        let rewind = pass < rewinds;
        let mut pass_config = config.clone();
        if rewinds > 0 {
            pass_config.spinup_repeats = 1;
            if rewind {
                pass_config.end = config.spinup_until;
            }
        }
        let end = run_spatial_chain(
            &case,
            pass_config,
            year,
            input,
            std::mem::take(&mut carried),
            rewind,
        )?;
        if !rewind {
            break;
        }
        println!(
            "colm-rs: spinup cycle {} of {} ends; LULCC back to {year}",
            pass + 1,
            config.spinup_repeats
        );
        let target = scratch.join("rewind");
        let Transition {
            baseflow, pairing, ..
        } = lulcc_transition(
            &case,
            LulccYears {
                old: end.year,
                new: year,
                history_frequency: config.history_frequency,
            },
            TransitionTimes {
                old: normalized_day_end(config.spinup_until),
                new: config.start,
                rewind: true,
            },
            &end.directory,
            &target,
            end.river,
            &end.baseflow,
        )?;
        // 预热终点恰在 LULCC 年末时，同一步先换到新一年、再换回起始年：配对按两次复合。
        let pairing = match end.pending_pairing {
            Some(first) => pairing
                .iter()
                .map(|middle| middle.and_then(|m| first.get(m).copied().flatten()))
                .collect(),
            None => pairing,
        };
        carried = Carried {
            baseflow: Some(baseflow),
            optimizer: end.optimizer.map(|optimizer| (optimizer, pairing)),
        };
        input = target;
    }
    if scratch.exists() {
        std::fs::remove_dir_all(&scratch)
            .with_context(|| format!("cannot remove {}", scratch.display()))?;
    }
    println!("{SUCCESS_MARKER}");
    Ok(())
}

/// 跨轮次带着走的东西：LULCC 换年后的 `scale_baseflow` 与优化器（连同配对）。
#[derive(Default)]
struct Carried {
    baseflow: Option<Vec<f64>>,
    optimizer: Option<(BaseflowOptimizer, Vec<Option<usize>>)>,
}

/// 一条段链跑完时的状态。
struct ChainEnd {
    /// 终点的土地覆盖年份。
    year: i64,
    /// 终态续跑所在的目录（其下是 `<date>/`）。
    directory: PathBuf,
    river: Option<SegmentRiverEnd>,
    baseflow: Vec<f64>,
    optimizer: Option<BaseflowOptimizer>,
    /// 终点恰是 LULCC 年末时那次换年的配对：优化器还没按它搬。
    pending_pairing: Option<Vec<Option<usize>>>,
}

/// 预热是否需要外层回卷（upstream-bugs 第 50 条）：多遍预热，且第一遍就过了第一个 LULCC 年末。
fn spinup_crosses_lulcc(config: &colm_runtime::spatial::runtime::SpatialRuntimeConfig) -> bool {
    let until = calendar_key(normalized_day_end(config.spinup_until));
    config.spinup_repeats > 1
        && until > calendar_key(normalized_day_end(config.start))
        && until >= calendar_key(normalized_day_end(lulcc_year_end(config.start)))
}

/// 从 `config.start` 跑到 `config.end`，途中在每个 LULCC 年末切段换年。
///
/// `rewind`：这是多遍预热中要回卷的一轮，终点是预热终点；终态留在临时目录交给回卷换年。
fn run_spatial_chain(
    case: &SpatialCase<'_>,
    config: colm_runtime::spatial::runtime::SpatialRuntimeConfig,
    year: i64,
    input: PathBuf,
    carried: Carried,
    rewind: bool,
) -> Result<ChainEnd> {
    let lulcc = logical_field(case.document, "DEF_USE_LULCC")?;
    let restart_root = case.out.join("restart");
    let scratch = restart_root.join(LULCC_SCRATCH);
    let run_end = normalized_day_end(config.end);
    let last_end = if rewind {
        SegmentEnd::Rewind
    } else {
        SegmentEnd::Run
    };
    let mut year = year;
    let mut segment = SpatialSegment {
        config: config.clone(),
        year,
        input,
        end: last_end,
        baseflow: carried.baseflow,
        optimizer: carried.optimizer,
    };
    loop {
        // LULCC 在一年最后一步之后做（`isendofyear`），运行在那里切段；2000 年以前只在换入
        // 5 的倍数年时做（见 [`lulcc_year_end`]）。
        let boundary = lulcc
            .then(|| lulcc_year_end(segment.config.start))
            .filter(|boundary| {
                calendar_key(normalized_day_end(*boundary)) <= calendar_key(run_end)
            });
        if let Some(boundary) = boundary {
            segment.config.end = boundary;
            segment.end = SegmentEnd::Lulcc;
        }
        let (river, baseflow, optimizer) = run_spatial_segment(case, &segment)?;
        let Some(boundary) = boundary else {
            return Ok(ChainEnd {
                year,
                directory: scratch.join("old"),
                river,
                baseflow,
                optimizer,
                pending_pairing: None,
            });
        };
        let next_start = normalized_day_end(boundary);
        // 合并出来的续跑落在哪：这一步本该写续跑（或运行就停在这里）时写进 `restart/`，
        // 否则放进临时目录，只供下一段起跑。回卷轮的终点不是运行终点。
        let target = if config.restart_frequency != colm_core::RestartFrequency::Never
            || (next_start == run_end && !rewind)
        {
            restart_root.clone()
        } else {
            scratch.join("new")
        };
        let Transition {
            baseflow,
            pairing,
            river,
        } = lulcc_transition(
            case,
            LulccYears {
                old: year,
                new: i64::from(boundary.year) + 1,
                history_frequency: config.history_frequency,
            },
            TransitionTimes {
                old: next_start,
                new: next_start,
                rewind: false,
            },
            &scratch.join("old"),
            &target,
            river,
            &baseflow,
        )?;
        year = i64::from(boundary.year) + 1;
        if next_start == run_end {
            return Ok(ChainEnd {
                year,
                directory: target,
                river,
                baseflow,
                optimizer,
                pending_pairing: Some(pairing),
            });
        }
        // 预热区间按绝对时刻延续到后面的段；已经结束的预热在新段里不再出现。
        let spinup_until =
            if calendar_key(normalized_day_end(config.spinup_until)) > calendar_key(next_start) {
                config.spinup_until
            } else {
                next_start
            };
        segment = SpatialSegment {
            config: colm_runtime::spatial::runtime::SpatialRuntimeConfig {
                start: next_start,
                spinup_until,
                ..config.clone()
            },
            year,
            input: target,
            end: last_end,
            baseflow: Some(baseflow),
            optimizer: optimizer.map(|optimizer| (optimizer, pairing)),
        };
    }
}

/// 空间算例里各段共用的东西。
struct SpatialCase<'a> {
    layout: &'a colm_case::Layout,
    name: &'a str,
    document: &'a Document,
    physics: &'a colm_runtime::assembly::LandPhysicsParameters,
    out: &'a Path,
    /// `HistForm = 'Vector'`：UNSTRUCTURED 内核且 `DEF_HISTORY_IN_VECTOR`。
    vector_history: bool,
    /// CATCHMENT 内核：流域侧向流代替网格河湖汇流。
    catchment: bool,
}

/// 空间算例里一个 patch 的块内信息：PFT 区间（`patch_pft_s/e`）与像元
/// （BGC 驱动数据的面积加权映射用）。
#[derive(Clone)]
struct SpatialPatch<'a> {
    pfts: std::ops::Range<usize>,
    pixel: &'a colm_runtime::spatial::mapping::PixelAxes,
    cells: &'a [(i32, i32)],
    shared_fraction: f64,
    /// 本 patch 各 PFT 的像元与 `pctshared`（`landpft`，区间同 `pfts`）。
    pft_cells: &'a [Vec<(i32, i32)>],
    pft_shared: &'a [f64],
}

/// 一段连续的运行（没有 LULCC 时就是整个运行）。
struct SpatialSegment {
    config: colm_runtime::spatial::runtime::SpatialRuntimeConfig,
    /// 这一段的土地覆盖年份（重启文件名里的 `lc<year>`、`landdata` 的年份目录）。
    year: i64,
    /// 起跑重启所在的目录（其下是 `<date>/`）：通常是 `restart/`，LULCC 合并出的临时续跑在别处。
    input: PathBuf,
    /// 这一段怎么结束：运行终点、LULCC 年末，或多遍预热的回卷点。
    end: SegmentEnd,
    /// LULCC 换年后按 SAT 配对重映射过的 `scale_baseflow`（全局 patch 次序）；`None` 时照常读
    /// `ParaOpt/<case>_baseflow.nc`（上游 `Opt_Baseflow_init` 只在启动时读一次）。
    baseflow: Option<Vec<f64>>,
    /// `DEF_Optimize_Baseflow`：上一段的优化器与"新 patch → 配上的旧 patch"（全局次序）。
    optimizer: Option<(BaseflowOptimizer, Vec<Option<usize>>)>,
}

/// 一段的终点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SegmentEnd {
    /// 运行终点（或本轮预热的终点之外的普通终点）：终态照常写进 `restart/`。
    Run,
    /// LULCC 年末：终态写进临时目录，交给 [`lulcc_transition`] 换到新一年。
    Lulcc,
    /// 多遍预热的回卷点（不在 LULCC 年末）：终态写进临时目录，交给 [`lulcc_transition`] 换回起始年。
    Rewind,
}

/// LULCC 临时文件（`restart/` 下）：旧年份的终态、不该留在 `restart/` 里的合并续跑、冷启动 namelist。
const LULCC_SCRATCH: &str = "lulcc-scratch";

/// 一段跑完时的河道终态：水量、示踪物与泥沙（后两者按开关有无）。
type SegmentRiverEnd = (
    colm_runtime::river::RiverState,
    Option<colm_runtime::river::tracer::RiverTracers>,
    Option<colm_runtime::river::sediment::Sediment>,
);

/// 跑一段，返回终点的河道状态（LULCC 过渡要接着用）。
fn run_spatial_segment(
    case: &SpatialCase<'_>,
    segment: &SpatialSegment,
) -> Result<(Option<SegmentRiverEnd>, Vec<f64>, Option<BaseflowOptimizer>)> {
    use colm_runtime::spatial::{
        forcing::GriddedForcing,
        history::{build_history_grid, ElementGroups, HistoryGridConfig, SpatialHistory},
        mapping::AreaWeightedMapping,
        runtime::SpatialRuntime,
        topology::SpatialTopology,
    };
    let SpatialCase {
        layout,
        name,
        document,
        physics,
        out,
        vector_history,
        catchment,
    } = *case;
    let config = &segment.config;
    let year = segment.year;
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
                time: segment.input.join(&start_label).join(format!(
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
    let mut downscaling_patches = Vec::new();
    // 同一份 namelist、同一套土层：PFT 参数按地类缓存，装配循环结束即关。
    let pft_cache = colm_runtime::pft::PftParameterCache::enable();
    for ((block, patches), files) in topology.blocks.iter().zip(&block_files) {
        // `DEF_USE_PFT`：土壤 patch 的 PFT 区间来自本块的 `landpft`。
        let pft_ranges = if physics.use_pft {
            Some(colm_runtime::pft::spatial_pft_ranges(
                &out.join("landdata"),
                i32::try_from(year)?,
                block,
                physics.land_cover_scheme,
                physics.bgc.is_some_and(|bgc| bgc.crop),
            )?)
        } else {
            None
        };
        // CROP 的 `mg2pft_*` 映射要每个 PFT 的像元。
        let pft_sets = if physics.bgc.is_some_and(|bgc| bgc.crop) {
            Some(colm_runtime::pft::spatial_pft_pixel_sets(
                &out.join("landdata"),
                i32::try_from(year)?,
                block,
            )?)
        } else {
            None
        };
        for patch in 0..patches.len() {
            let global = patches.start + patch;
            let pfts = pft_ranges
                .as_ref()
                .map_or(0..0, |ranges| ranges[patch].clone());
            let pft_cells: &[Vec<(i32, i32)>] = pft_sets
                .as_ref()
                .map_or(&[], |sets| &sets.cells[pfts.clone()]);
            let pft_shared: &[f64] = pft_sets
                .as_ref()
                .map_or(&[], |sets| &sets.shared_fraction[pfts.clone()]);
            let spatial_patch = SpatialPatch {
                pfts,
                pixel: &topology.pixel,
                cells: &topology.cells[global],
                shared_fraction: topology.shared_fraction[global],
                pft_cells,
                pft_shared,
            };
            templates.push(
                assemble_patch(
                    document,
                    layout,
                    name,
                    files,
                    physics.clone(),
                    patch,
                    block,
                    true,
                    Some(spatial_patch),
                )
                .with_context(|| format!("cannot assemble patch {patch} of block {block}"))?,
            );
            let worker_patch = templates.len() - 1;
            if let Some(template) = templates.last_mut() {
                template.worker_patch = worker_patch;
            }
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
        // 强迫降尺度的地形因子与 `elvmean`（`MOD_Vars_TimeInvariants` 写在常数重启里）。
        if let Some(settings) = config.downscaling {
            let first = templates.len() - patches.len();
            let patch_types = templates[first..]
                .iter()
                .map(|template| template.patch_type)
                .collect::<Vec<_>>();
            downscaling_patches.extend(colm_runtime::spatial::downscaling::read_block_patches(
                &constant,
                settings.simple,
                &patch_types,
                &patch_mask[first..],
            )?);
        }
    }
    drop(pft_cache);
    // LULCC 之后的段：`scale_baseflow` 用上一段按 SAT 配对搬过来的（upstream-bugs 第 43 条，vendor 已修），
    // 不再按新 patch 编号重读标定文件。
    if let Some(baseflow) = &segment.baseflow {
        ensure!(
            baseflow.len() == templates.len(),
            "the remapped scale_baseflow has {} values for {} patches",
            baseflow.len(),
            templates.len()
        );
        for (template, &scale) in templates.iter_mut().zip(baseflow) {
            template.baseflow_scale = scale;
        }
    }
    let baseflow: Vec<f64> = templates
        .iter()
        .map(|template| template.baseflow_scale)
        .collect();
    methane_wetland_fractions(&mut templates, &topology);
    methane_giems(document, &mut templates, || Ok(coordinates.clone()))?;
    // `land_tracer_init`：逐块读续跑里的示踪物事务（或按水量冷启动）。
    // `tracer_forcing_init`：网格主强迫给出总降水/总比湿的配置。
    let mut tracer_forcing_config = None;
    let tracer_runtime = colm_runtime::tracer::TracerRuntime::from_document(document)?
        .map(|mut tracer| -> Result<_> {
            let totals =
                colm_runtime::spatial::tracer_forcing::MainTotals::from_config(&config.forcing);
            tracer_forcing_config = Some(tracer.configure_forcing(Some(&totals))?);
            Ok(std::sync::Arc::new(tracer))
        })
        .transpose()?;
    let empty_land_tracer = tracer_runtime
        .as_ref()
        .filter(|tracer| !tracer.has_transport())
        .map(|tracer| tracer.aquifer_mixing_water_mm);
    if let Some(tracer) = tracer_runtime
        .as_ref()
        .filter(|tracer| tracer.has_transport())
    {
        let mut attached = Vec::with_capacity(templates.len());
        let mut rest = templates.into_iter();
        for ((_, patches), files) in topology.blocks.iter().zip(&block_files) {
            let block: Vec<_> = rest.by_ref().take(patches.len()).collect();
            let restart = colm_init::RestartFile::open(&files.time)?;
            attached.extend(attach_land_tracers(tracer, block, &restart, patches.len())?);
        }
        templates = attached;
    }
    let grid = GriddedForcing::open_grid(&config.forcing, config.start)?;
    let writes_history = config.history_frequency != colm_hist::schedule::HistoryFrequency::None;
    let mut mapping = if let Some(domain) = config.bilinear {
        AreaWeightedMapping::build_bilinear(
            &grid,
            &topology.pixel,
            &topology.cells,
            &topology.shared_fraction,
            &coordinates,
            domain,
        )?
    } else {
        AreaWeightedMapping::build(
            &grid,
            &topology.pixel,
            &topology.cells,
            &topology.shared_fraction,
        )?
    };
    let cells = mapping
        .parts
        .iter()
        .flatten()
        .map(|part| (part.ilon, part.ilat))
        .collect::<Vec<_>>();
    let mut forcing = GriddedForcing::new(
        config.forcing.clone(),
        grid.clone(),
        cells,
        config.timestep_seconds as i32,
    )?;
    // `DEF_forcing%has_missing_value`：映射去掉起始那条记录里缺测的格子（`set_missing_value`），
    // 足迹全是缺测格的 patch 的 `forcmask_pch` 为假：`CoLMDRIVER` 整步跳过它，汇流与各 history
    // `filter` 都与上它（见 `SpatialRuntime`、`HistorySession::push_masked`）。
    let forcing_mask = match forcing.missing_field(config.start)? {
        Some((missing, field, nlon)) => {
            mapping.set_missing_value(|ilon, ilat| field[ilat * nlon + ilon], missing)
        }
        None => vec![true; coordinates.len()],
    };
    let masked = forcing_mask.iter().filter(|&&active| !active).count();
    if masked > 0 {
        println!(
            "colm-rs: {masked} patch(es) lie entirely on missing forcing cells (forcmask_pch = .false.): {:?}",
            forcing_mask
                .iter()
                .enumerate()
                .filter(|(_, &active)| !active)
                .map(|(patch, _)| (patch, templates[patch].patch_type))
                .collect::<Vec<_>>()
        );
        // 已对齐：BGC（含 CROP）、火灾 `tsoi17` 广播（第 541、542 轮）、LULCC（第 539 轮）、漫滩回馈（第 540 轮）。
    }
    // 静态面积（`landarea` 等）的过滤也与上 `forcmask_pch`，所以 history 网格在掩膜之后建。
    let mut history_grid = if writes_history && !vector_history {
        let patch_types = templates
            .iter()
            .map(|template| template.patch_type)
            .collect::<Vec<_>>();
        let crop_classes = physics.bgc.is_some_and(|bgc| bgc.crop).then(|| {
            templates
                .iter()
                .map(|template| template.land_class)
                .collect::<Vec<_>>()
        });
        // `filter_irrig`：农田 patch 的首个 PFT 是灌溉型作物（`>= npcropmin` 且为偶数）。
        let irrigated = physics.irrigation.is_some().then(|| {
            templates
                .iter()
                .map(|template| {
                    template.land_class == 12
                        && template
                            .pft
                            .as_ref()
                            .and_then(|pft| pft.initial.parameters.first())
                            .is_some_and(|first| {
                                first.class >= colm_core::bgc_driver::NPCROPMIN
                                    && first.class % 2 == 0
                            })
                })
                .collect::<Vec<_>>()
        });
        Some(build_history_grid(
            &HistoryGridConfig::read(document)?,
            &grid,
            &topology,
            &patch_types,
            &patch_mask,
            &forcing_mask,
            crop_classes.as_deref(),
            irrigated.as_deref(),
        )?)
    } else {
        None
    };
    let mut runtime = SpatialRuntime::new(
        config.clock()?,
        forcing,
        mapping,
        coordinates,
        config.co2_scenario,
        forcing_mask.clone(),
    )?;
    if let Some(settings) = config.downscaling {
        runtime = runtime.with_downscaling(settings, downscaling_patches)?;
    }
    runtime.apply_mapped_heights(&mut templates)?;
    if let (Some(tracer), Some(forcing_config)) = (tracer_runtime.as_ref(), tracer_forcing_config) {
        if forcing_config.enabled() {
            let mut forcing = colm_runtime::spatial::tracer_forcing::GriddedTracerForcing::new(
                forcing_config,
                &tracer.set,
                patch_count,
                string_field(document, "DEF_Forcing_Interp_Method")?,
                runtime.forcing_config(),
            )?;
            // `tracer_forcing_read_restart`：续跑读到示踪物事务时一并读回最近一次有效比值。
            let mut first = 0;
            for ((_, patches), files) in topology.blocks.iter().zip(&block_files) {
                let restart = colm_init::RestartFile::open(&files.time)?;
                let loadable =
                    colm_runtime::tracer::land_tracer_restart_loadable(&restart, &tracer.set);
                if let Some((precip, vapor)) = colm_runtime::tracer::read_forcing_cache(
                    &restart,
                    loadable,
                    forcing.config().vars.len(),
                    forcing.identity(),
                )? {
                    forcing.restore(first, &precip, &vapor)?;
                }
                first += patches.len();
            }
            runtime = runtime.with_tracer_forcing(forcing);
        }
    }
    if segment.end == SegmentEnd::Lulcc {
        runtime = runtime.defer_lai_refresh_at(config.end);
    }
    let (history_grid, river_writer, history_vector, mut runtime) = if catchment {
        let history_grid = history_grid.take().map(std::sync::Arc::new);
        let landdata = out.join("landdata");
        let mesh = PathBuf::from(string_field(document, "DEF_CatchmentMesh_data")?);
        let neighbours = PathBuf::from(string_field(document, "DEF_ElementNeighbour_file")?);
        let runtime_dir = PathBuf::from(string_field(document, "DEF_dir_runtime")?);
        let basin_restart = colm_runtime::catchment::runtime::basin_restart_path(
            &segment.input,
            name,
            &start_label,
            year,
        );
        let catchment = colm_runtime::catchment::runtime::CatchmentRuntime::load(
            &colm_runtime::catchment::runtime::CatchmentSetup {
                landdata: &landdata,
                land_cover_year: i32::try_from(year)?,
                catchment_mesh: &mesh,
                neighbour_file: &neighbours,
                runtime_dir: &runtime_dir,
                estimated_river_depth: logical_field(document, "DEF_USE_EstimatedRiverDepth")?,
                block_names: topology
                    .blocks
                    .iter()
                    .map(|(block, _)| block.clone())
                    .collect(),
                constant_restarts: block_files.iter().map(|f| f.constant.clone()).collect(),
                time_restarts: block_files.iter().map(|f| f.time.clone()).collect(),
                basin_restart: &basin_restart,
                time_step_seconds: config.timestep_seconds,
            },
            &templates,
        )?;
        // `lateral_flow_init` 末尾 `write_catch_parameters`。
        catchment.write_parameters(
            &out.join("catch_parameters.nc"),
            &block_files
                .iter()
                .map(|f| f.constant.clone())
                .collect::<Vec<_>>(),
            u8::try_from(integer_field(document, "DEF_REST_CompressLevel")?)
                .context("DEF_REST_CompressLevel must fit 0..=9")?,
        )?;
        let runtime = runtime.with_catchment(catchment)?;
        (history_grid, None, None, runtime)
    } else {
        let mut runtime = runtime;
        let network = colm_runtime::river::network::RiverNetwork::read(
            &unit_catchment_file(document, out)?,
            logical_field(document, "DEF_GridRiverLake_FloodplainStorageFix")?,
        )?;
        let routing = colm_runtime::river::network::RunoffRouting::build(&network, &topology)?;
        // `filter_rnof`/`filter_basic`：`patchtype < 99 .and. patchmask .and. forcmask_pch`。
        let runoff_filter = templates
            .iter()
            .zip(&patch_mask)
            .zip(&forcing_mask)
            .map(|((template, &mask), &active)| template.patch_type < 99 && mask && active)
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
        let river_writer = if vector_history && writes_history {
            Some(colm_runtime::river::history::RiverHistoryWriter::new(
                &network,
                &routing,
                None,
                &runoff_filter,
                out.join("history"),
                name,
            )?)
        } else {
            history_grid
                .as_ref()
                .map(|grid| {
                    colm_runtime::river::history::RiverHistoryWriter::new(
                        &network,
                        &routing,
                        Some(std::sync::Arc::clone(grid)),
                        &runoff_filter,
                        out.join("history"),
                        name,
                    )
                })
                .transpose()?
        };
        // 向量 history（`HistForm = 'Vector'`）：单元按 `elmindex` 递增（`eindex_glb`），每个单元的 patch
        // 区间与 `subfrc` 来自拓扑；河道的静态掩码按 `filter_ucat` 聚合到单元。
        let history_vector = if vector_history && writes_history {
            let groups = ElementGroups::from_topology(&topology)?;
            let mut elements = groups
                .ranges
                .iter()
                .map(|range| (topology.element[range.start], range.clone()))
                .collect::<Vec<_>>();
            elements.sort_by_key(|(id, _)| *id);
            let vector = colm_hist::history::HistoryVector {
                elmindex: elements.iter().map(|(id, _)| *id).collect(),
                elements: elements.into_iter().map(|(_, range)| range).collect(),
                subfrc: groups.fractions.clone(),
                compress_level: colm_runtime::spatial::history::hist_compress_level(document)?,
            };
            let (mask, filter) =
                colm_runtime::river::history::RiverHistoryWriter::upstream_mask_patches(
                    &network,
                    &routing,
                    &runoff_filter,
                );
            let statics = vec![(
                "mask_complete_upstream_regird".to_owned(),
                "Mask of grids with all upstream located in simulation region".to_owned(),
                "100%".to_owned(),
                vector.aggregate(|p| mask[p], |p| filter[p], false),
            )];
            Some((std::sync::Arc::new(vector), statics))
        } else {
            None
        };
        let reservoir = if integer_field(document, "DEF_Reservoir_Method")? > 0 {
            Some(
                colm_runtime::river::reservoir::Reservoir::read_with_regional(
                    Path::new(&string_field(document, "DEF_ReservoirPara_file")?),
                    &network,
                    integer_field(document, "DEF_Reservoir_Method")?,
                    regional_catchment(document, out)?.as_deref(),
                )?,
            )
        } else {
            None
        };
        let levee = if logical_field(document, "DEF_USE_LEVEE")? {
            // 水库表里的单元流域（`lake_type == 2`）上游强制无堤。
            let reservoir_cells = reservoir.as_ref().map_or_else(
                || vec![false; network.len()],
                |r| r.of_catchment.iter().map(Option::is_some).collect(),
            );
            Some(colm_runtime::river::levee::Levee::read(
                &unit_catchment_file(document, out)?,
                &network,
                &reservoir_cells,
            )?)
        } else {
            None
        };
        let bifurcation = if logical_field(document, "DEF_USE_BIFURCATION")? {
            Some(colm_runtime::river::bifurcation::Bifurcation::read(
                &unit_catchment_file(document, out)?,
                &network,
            )?)
        } else {
            None
        };
        let river_start = river_restart_path(&segment.input, name, &start_label, year);
        let mut river_writer = river_writer;
        if let (Some(writer), Some(reservoir)) = (river_writer.as_mut(), reservoir.as_ref()) {
            writer.reservoir_ids = Some(reservoir.grand_id.clone());
        }
        // unitcat 文件的压缩：逐时间量用 `DEF_HIST_CompressLevel`，静态掩码用 `DEF_REST_CompressLevel`。
        if let Some(writer) = river_writer.as_mut() {
            writer.hist_compress_level =
                colm_runtime::spatial::history::hist_compress_level(document)?;
            writer.rest_compress_level =
                u8::try_from(integer_field(document, "DEF_REST_CompressLevel")?)
                    .context("DEF_REST_CompressLevel must fit 0..=9")?;
        }
        let river_state = colm_runtime::river::restart::read_river_state(
            &river_start,
            &network,
            reservoir.as_ref(),
        )?;
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
                let river_file = segment
                    .input
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
                    levee.is_some(),
                    bifurcation.as_ref().map(|bif| (bif.paths(), bif.levels)),
                    reservoir.as_ref().map(|r| r.len()),
                )?)
            } else {
                None
            }
        };
        let mut river = colm_runtime::river::RiverModel::new(
            network,
            routing,
            river_state,
            real_field(document, "DEF_GRIDBASED_ROUTING_MAX_DT")?,
            levee,
            bifurcation,
            reservoir,
        )?;
        if let Some(history) = river_history {
            river.history = history;
        }
        river.momentum_dt_limit =
            logical_field(document, "DEF_GRIDBASED_ROUTING_MOMENTUM_DT_LIMIT")?;
        // `river_lake_tracer_init` + `read_tracer_restart`/`tracer_init_from_water`
        // （`grid_riverlake_flow_init`）：有输运示踪物时河道示踪物与陆面一起开。
        if let Some(tracer) = tracer_runtime
            .as_ref()
            .filter(|tracer| tracer.has_transport())
        {
            let mut tracers = colm_runtime::river::tracer::RiverTracers::new(
                tracer.set.clone(),
                river.network.len(),
            );
            let loaded = colm_runtime::river::restart::read_river_tracers(
                &river_start,
                &river.network,
                river.levee.as_ref(),
                &mut tracers,
            )?;
            if !loaded {
                // `is_built_resv_init`：起始年份下已建成的水库。
                let built = (0..river.network.len())
                    .map(|i| {
                        river.reservoir.as_ref().is_some_and(|reservoir| {
                            reservoir.of_catchment[i]
                                .is_some_and(|r| reservoir.is_built(r, config.start.year))
                        })
                    })
                    .collect::<Vec<_>>();
                tracers.cold_start(
                    &river.network,
                    river.levee.as_ref(),
                    &river.state,
                    river.reservoir.as_ref(),
                    &built,
                );
            }
            river = river.with_tracers(tracers)?;
        }
        // `grid_riverlake_flow_init`：回馈打开时立刻按读回的状态发布一次（Rust 不在 spinup 里汇流）。
        // 上游在河湖示踪物初始化之后才分配回馈、发布，所以排在示踪物后面。
        if logical_field(document, "DEF_GridRiverLake_FloodFeedback")? {
            let infiltration_max_mm_day = match document.get("DEF_GridRiverLake_FloodInfiltMax") {
                Some(Value::Real { text }) => text
                    .trim()
                    .trim_end_matches("_r8")
                    .replace(['d', 'D'], "e")
                    .parse::<f64>()
                    .with_context(|| format!("DEF_GridRiverLake_FloodInfiltMax = {text}"))?,
                Some(Value::Int(value)) => *value as f64,
                Some(other) => bail!("DEF_GridRiverLake_FloodInfiltMax must be real, got {other}"),
                // `MOD_Namelist.F90` 的缺省 `5._r8`（mm/day）。
                None => 5.0,
            };
            // `grid_riverlake_flow_init(s_year, …)`：本次运行的起始年份。
            river = river.with_flood_feedback(infiltration_max_mm_day, config.start.year)?;
        }
        river.empty_tracer_transaction = tracer_runtime.is_some();
        // CH4 provider 注册了 `publish_flood`/`publish_levee_flood`：每次汇流末把淹没比例推到 patch。
        if tracer_runtime.as_ref().is_some_and(|tracer| {
            tracer
                .set
                .tracers
                .iter()
                .any(colm_runtime::methane::is_methane_tracer)
        }) {
            river = river.with_methane_flood();
        }
        // `grid_sediment_init` + `read_sediment_restart`：`SEDIMENT` provider 示踪物（河道泥沙）。
        if let Some(set) = tracer_runtime.as_ref().map(|tracer| &tracer.set) {
            if let Some(index) = set
                .tracers
                .iter()
                .position(colm_runtime::river::sediment::is_sediment_tracer)
            {
                let files = string_field(document, "DEF_TRACER_PARAM_FILES")?;
                let param = colm_core::tracer::descriptor::param_file_for_index(
                    &files,
                    &set.tracers,
                    index,
                )?
                .context(
                    "Cannot find sediment parameter file for SEDIMENT in DEF_TRACER_PARAM_FILES",
                )?;
                let sediment = colm_runtime::river::sediment::Sediment::init(
                    &river.network,
                    &unit_catchment_file(document, out)?,
                    &param,
                )?;
                river = river.with_sediment(sediment)?;
                let network = &river.network;
                if let Some(sediment) = river.sediment.as_mut() {
                    sediment.read_restart(&river_start, network)?;
                }
            }
        }
        runtime = runtime.with_river(river, runoff_filter)?;
        (history_grid, river_writer, history_vector, runtime)
    };
    let rest_compression = u8::try_from(integer_field(document, "DEF_REST_CompressLevel")?)
        .context("DEF_REST_CompressLevel must fit 0..=9")?;
    let para_opt = out.join("restart/ParaOpt");
    std::fs::create_dir_all(&para_opt)
        .with_context(|| format!("cannot create {}", para_opt.display()))?;
    if logical_field(document, "DEF_Optimize_Baseflow")? {
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
        // LULCC 之后的段：接着上一段的优化器（状态按 SAT 配对搬到新布局，迭代计数延续）。
        let optimizer = match &segment.optimizer {
            Some((previous, pairing)) => previous.carried_over(pairing, &patches)?,
            // 只支持单块（见上），文件名带该块的后缀（原来写死成单点的 `w180_s90`）。
            None => BaseflowOptimizer::new(&patches, para_opt, name, &topology.blocks[0].0),
        };
        runtime = runtime.with_baseflow_optimizer(optimizer);
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
            .with_gridded(if catchment {
                &[]
            } else {
                &colm_runtime::river::history::GRIDDED_RIVER_VARIABLES
            })
            .with_tracer_history(
                logical_field(document, "DEF_USE_TRACER")?
                    .then(|| real_field(document, "DEF_simulation_time%timestep"))
                    .transpose()?,
            )
            .map(|session| {
                // 有输运示踪物或 CH4 provider 时写变量（`tracer_hist_out` 与
                // `methane_reactive_history`），否则只有文件骨架。
                match tracer_runtime.as_ref().filter(|t| {
                    t.has_transport()
                        || t.set
                            .tracers
                            .iter()
                            .any(colm_runtime::methane::is_methane_tracer)
                }) {
                    Some(tracer) => session.with_tracer_variables(
                        tracer.set.clone(),
                        templates
                            .iter()
                            .map(|template| template.patch_type)
                            .collect(),
                    ),
                    None => session,
                }
            })?,
            river: river_writer,
            elements: ElementGroups::from_topology(&topology)?,
            files: Vec::new(),
            basin: catchment
                .then(|| -> Result<_> {
                    Ok(colm_runtime::spatial::history::BasinHistoryTarget {
                        directory: out.join("history"),
                        stem: name.to_owned(),
                        compress_level: colm_runtime::spatial::history::hist_compress_level(
                            document,
                        )?,
                    })
                })
                .transpose()?,
        }),
        None => match history_vector {
            Some((vector, statics)) => Some(SpatialHistory {
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
                .with_vector(
                    vector,
                    &colm_runtime::river::history::VECTOR_RIVER_VARIABLES,
                    statics,
                )
                .map(|session| {
                    // 向量写出（没有窗口变量，所以不调 `with_tracer_history`）：开了示踪物就写示踪物文件（`MOD_Tracer_Hist` 的 `Vector` 支），
                    // 没有输运示踪物也没有 CH4 时只有维与坐标。
                    match tracer_runtime.as_ref() {
                        Some(tracer) => session.with_tracer_variables(
                            tracer.set.clone(),
                            templates
                                .iter()
                                .map(|template| template.patch_type)
                                .collect(),
                        ),
                        None => session,
                    }
                })?,
                river: river_writer,
                elements: ElementGroups::from_topology(&topology)?,
                files: Vec::new(),
                basin: None,
            }),
            None => None,
        },
    };

    if let Some(history) = history.as_mut() {
        history
            .session
            .set_forcing_mask((masked > 0).then(|| forcing_mask.clone()));
    }
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
            urban_run: logical_field(document, "DEF_URBAN_RUN")?,
            urban_patches: templates
                .iter()
                .filter(|template| template.urban.is_some())
                .count(),
            pft_or_pc: logical_field(document, "DEF_USE_PFT")?
                || logical_field(document, "DEF_USE_PC")?,
            // BGC 的累加量（`a_leafc` … `a_*Cap`）也进旁车（第 541 轮）。
            bgc: physics.bgc.is_some(),
            crop: physics.bgc.is_some_and(|switches| switches.crop),
            river_lake_flow: !catchment,
        },
        window: history
            .as_ref()
            .map(|history| history.session.window_handle()),
        tracer_raw: history
            .as_ref()
            .map(|history| history.session.tracer_raw_handle()),
        urban: templates
            .iter()
            .map(|template| template.urban.is_some())
            .collect(),
    };
    // `read_history_acc_restart`：续跑重启带着未写完的历史区间时接着累加（每块一份旁车，按块拼接）。
    let mut restored = Vec::with_capacity(patch_count);
    let mut open_window = false;
    for ((_, patches), files) in topology.blocks.iter().zip(&block_files) {
        // 示踪物部分按块读回（块内顺序即旁车顺序）。
        restore_sidecar_tracers(
            &mut templates[patches.clone()],
            &history_sidecar_path(&files.time)?,
            patches.len(),
            |position, _| position,
        )?;
        match colm_runtime::history_sidecar::read_sidecar(
            &files.time,
            &history_sidecar_path(&files.time)?,
            &history_restart.config,
            &history_restart.urban[patches.clone()],
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
        |steps, states, outputs, river, tracer_cache, lateral| {
            let mut snapshots = states
                .iter()
                .zip(outputs)
                .zip(steps)
                .map(|((state, output), step)| {
                    let Some(output) = output else {
                        return Ok(RestartSnapshot::masked());
                    };
                    let mut snapshot =
                        RestartSnapshot::new(state, *output, step.surface_cosine_zenith)?;
                    // LULCC 年末那一步没重读 LAI（`defer_lai_refresh_at`）。
                    snapshot.lai_refreshed = step.clock.update_lai
                        && !(segment.end == SegmentEnd::Lulcc && step.clock.end_time == config.end);
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
                    tracer_cache,
                    empty_land_tracer,
                )?;
                if let Some(river) = river {
                    let label = date_label(normalized_day_end(step.clock.end_time));
                    let path = river_restart_path(&out.join("restart"), name, &label, year);
                    colm_runtime::river::restart::write_river_state(
                        &path,
                        &river.network,
                        &river.state,
                        river.reservoir.as_ref().map(|r| r.identity()).as_deref(),
                        rest_compression,
                    )?;
                    if let Some(tracers) = river.tracers.as_ref() {
                        colm_runtime::river::restart::write_river_tracers(
                            &path,
                            &river.network,
                            &mut tracers.clone(),
                            rest_compression,
                        )?;
                    }
                    if river.tracers.is_none() && river.empty_tracer_transaction {
                        colm_runtime::river::restart::write_empty_river_tracers(&path)?;
                    }
                    if let Some(sediment) = river.sediment.as_ref() {
                        sediment.write_restart(&path, &river.network, rest_compression)?;
                    }
                }
                if let Some(lateral) = lateral {
                    write_catchment_restarts(
                        lateral,
                        &topology.blocks,
                        &periodic,
                        step.clock.end_time,
                        name,
                        year,
                        states,
                        rest_compression,
                    )?;
                }
            }
            last = Some(snapshots);
            Ok(())
        },
    )?;
    let last = last.context(NO_STEP)?;
    // 停在 LULCC 年末或预热回卷点的一段：终态只是换年的输入，写进临时目录。回卷点上上游照常
    // 按频率写续跑，但最后一轮会在同一时刻写同名文件盖掉它，所以这里不写。
    let finals = if segment.end != SegmentEnd::Run {
        topology
            .blocks
            .iter()
            .map(|(block, _)| PeriodicRestarts {
                directory: out.join("restart").join(LULCC_SCRATCH).join("old"),
                name: name.to_owned(),
                land_cover_year: year,
                block: block.clone(),
            })
            .collect()
    } else {
        periodic
    };
    let written = write_block_restarts(
        &topology.blocks,
        &block_files,
        &finals,
        config.end,
        &templates,
        &states,
        &last,
        &history_restart,
        runtime.river(),
        runtime
            .tracer_forcing()
            .map(|forcing| forcing.cache())
            .as_ref(),
        empty_land_tracer,
    )?;
    if let Some(lateral) = runtime.catchment() {
        ensure!(
            segment.end == SegmentEnd::Run,
            "catchment lateral flow does not support LULCC segments"
        );
        write_catchment_restarts(
            lateral,
            &topology.blocks,
            &finals,
            config.end,
            name,
            year,
            &states,
            rest_compression,
        )?;
    }
    if let (Some(river), SegmentEnd::Run) = (runtime.river(), segment.end) {
        let label = date_label(normalized_day_end(config.end));
        let path = river_restart_path(&out.join("restart"), name, &label, year);
        colm_runtime::river::restart::write_river_state(
            &path,
            &river.network,
            &river.state,
            river.reservoir.as_ref().map(|r| r.identity()).as_deref(),
            rest_compression,
        )?;
        if let Some(tracers) = river.tracers.as_ref() {
            colm_runtime::river::restart::write_river_tracers(
                &path,
                &river.network,
                &mut tracers.clone(),
                rest_compression,
            )?;
        }
        if river.tracers.is_none() && river.empty_tracer_transaction {
            colm_runtime::river::restart::write_empty_river_tracers(&path)?;
        }
        if let Some(sediment) = river.sediment.as_ref() {
            sediment.write_restart(&path, &river.network, rest_compression)?;
        }
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
    if segment.end != SegmentEnd::Run {
        // 上游在 LULCC 时 `hist_final` 再 `hist_init`：没写完的区间就丢了。年末之前的区间
        // 都在这一步关上，跨年的区间不会出现；真出现就拒绝，而不是写一条上游没有的记录。
        let open = history_restart.window.as_ref().is_some_and(|window| {
            window
                .lock()
                .expect("history window lock")
                .iter()
                .any(|window| window.steps > 0)
        });
        ensure!(
            !open,
            "a history window stays open across the LULCC year end at {}; the Rust runtime does not carry it over",
            date_label(normalized_day_end(config.end))
        );
    }
    if let Some(mut history) = history {
        history.files.extend(history.session.finish()?);
        ensure!(
            history.session.remaining() == 0,
            "the run ended with {} history record(s) still unwritten",
            history.session.remaining()
        );
        println!("colm-rs: {} history file(s)", history.files.len());
    }
    let river = runtime.river().map(|river| {
        (
            river.state.clone(),
            river.tracers.clone(),
            river.sediment.clone(),
        )
    });
    // 优化器在场时 `scale_baseflow` 以它为准（年末结算可能刚改过）。
    let optimizer = runtime.take_baseflow_optimizer();
    let baseflow = optimizer
        .as_ref()
        .map_or(baseflow, BaseflowOptimizer::scales);
    Ok((river, baseflow, optimizer))
}

/// 流域续跑：`<case>_restart_basin_<date>_lc<year>.nc`，以及各块时间重启里非湖 patch 的湖层
/// （动态湖调整对全部 patch 做，`MOD_Catch_LateralFlow.F90:394-414`）。
#[allow(clippy::too_many_arguments)]
fn write_catchment_restarts(
    lateral: &colm_runtime::catchment::runtime::CatchmentRuntime,
    blocks: &[(String, std::ops::Range<usize>)],
    periodic: &[PeriodicRestarts],
    end_time: CalendarTime,
    name: &str,
    year: i64,
    states: &[StandardLctSnowSoilState],
    compression: u8,
) -> Result<()> {
    let label = date_label(normalized_day_end(end_time));
    let directory = &periodic
        .first()
        .context("a spatial run has at least one block")?
        .directory;
    let block_restarts = blocks
        .iter()
        .zip(periodic)
        .map(|((_, range), periodic)| (periodic.path(end_time), range.clone()))
        .collect::<Vec<_>>();
    lateral.write_restarts(
        &colm_runtime::catchment::runtime::basin_restart_path(directory, name, &label, year),
        &block_restarts,
        states,
        compression,
    )
}

/// CATCHMENT 内核（`CatchLateralFlow`）里 Rust 还没有的组合：入口一次拒绝。
fn check_catchment(document: &Document) -> Result<()> {
    ensure!(
        integer_field(document, "DEF_Reservoir_Method")? == 0,
        "catchment lateral flow with reservoirs (DEF_Reservoir_Method > 0) is not ported"
    );
    ensure!(
        !logical_field(document, "DEF_USE_LULCC")?,
        "catchment lateral flow with DEF_USE_LULCC is not ported"
    );
    ensure!(
        !logical_field(document, "DEF_USE_TRACER")?,
        "catchment lateral flow with tracers is not ported"
    );
    ensure!(
        !logical_field(document, "DEF_URBAN_RUN")?,
        "catchment lateral flow with the urban model is not ported"
    );
    ensure!(
        !logical_field(document, "DEF_HISTORY_IN_VECTOR")?,
        "catchment vector history (DEF_HISTORY_IN_VECTOR) is not ported; use gridded history"
    );
    ensure!(
        !logical_field(document, "DEF_Optimize_Baseflow")?,
        "catchment lateral flow with DEF_Optimize_Baseflow is not ported"
    );
    Ok(())
}

/// Rust 这边 LULCC 只接上游的 SAT 默认路径：LCT、IGBP、不 spinup、2000 年以后。
fn check_spatial_lulcc(
    document: &Document,
    config: &colm_runtime::spatial::runtime::SpatialRuntimeConfig,
    land_cover: LandCoverScheme,
) -> Result<()> {
    ensure!(
        matches!(integer_field(document, "DEF_LULCC_SCHEME")?, 1 | 2),
        "DEF_LULCC_SCHEME must be 1 (same type assignment) or 2 (mass and energy conserving)"
    );
    ensure!(
        land_cover == LandCoverScheme::Igbp,
        "upstream LULCC supports IGBP land cover only"
    );
    // `MOD_Namelist.F90:2272-2276`：`DEF_USE_USGS .or. DEF_USE_BGC` 时上游停机
    // （"LULCC is not supported for LULC_USGS/BGC at present"）。甲烷依赖 BGC，所以也跟着不支持。
    ensure!(
        !logical_field(document, "DEF_USE_BGC")?,
        "LULCC is not supported for BGC upstream (MOD_Namelist stops too)"
    );
    // 城市：SAT 与 MEC 的城市段已移植。示踪物 + 城市模型上游在 `CoLMDRIVER.F90:89-92` 第一步就停机，
    // Rust 在装配城市 patch 的示踪物时拒绝（`assembly.rs`），不再单列 LULCC 的拒绝。
    // 上游灌溉只在 `#ifdef CROP` 下按作物物候施水（`CoLMMAIN.F90:849-858`），要 BGC 作物状态；
    // 而 LULCC 与 BGC 上游互斥（见上），所以这个组合在上游走不到有意义的路径。
    ensure!(
        !logical_field(document, "DEF_USE_IRRIGATION")?,
        "DEF_USE_IRRIGATION needs CROP BGC, which upstream refuses together with DEF_USE_LULCC"
    );
    // `scale_baseflow` 与优化器的逐 patch 累加量换年后按 SAT 配对重映射（upstream-bugs 第 43 条，
    // 两侧都已修），见 [`lulcc_transition`] 与 `BaseflowOptimizer::carried_over`。
    // `MOD_Namelist` 在 LULCC 时强制月度 LAI、逐年换 LAI；Rust 不替 namelist 改，直接要求。
    for field in ["DEF_LAI_MONTHLY", "DEF_LAI_CHANGE_YEARLY"] {
        ensure!(
            logical_field(document, field)?,
            "DEF_USE_LULCC forces {field} = .true. upstream; set it in the namelist"
        );
    }
    // 多遍预热跨过 LULCC 年末时，回卷把土地覆盖换回起始年（upstream-bugs 第 50 条，两侧都已修），
    // 见 [`spinup_crosses_lulcc`]。
    // 上游运行期 `lc_year = s_year` 不取整，而 2000 年以前 mksrfdata 只写 `max(1985, 5 年取整)` 的
    // 土地覆盖：起始年不是那样的年份时，上游去读不存在的 landdata 年份。
    ensure!(
        config.start.year >= 2000 || (config.start.year >= 1985 && config.start.year % 5 == 0),
        "DEF_USE_LULCC starting in {} before 2000: upstream loads the land cover of the start year, \
         but mksrfdata writes it only for 1985, 1990, 1995 (and every year from 2000)",
        config.start.year
    );
    Ok(())
}

/// 区域模式下的区域网络文件（水库 `dam_seq` 按它的 `seq_src_index` 换号）；否则 `None`。
fn regional_catchment(document: &Document, out: &Path) -> Result<Option<std::path::PathBuf>> {
    Ok(logical_field(document, "DEF_UnitCatchment_regional")?
        .then(|| colm_init::unitcatchment_regional::regional_file(&out.join("landdata"))))
}

/// `get_unitcatchment_file`（`share/MOD_Namelist.F90:2884-2895`）：区域模式读 landdata 下裁好的区域网络，
/// 否则读 `DEF_UnitCatchment_file`。区域模式下先核对区域网络属于当前的源网络（`verify_regional_network`）。
fn unit_catchment_file(document: &Document, out: &Path) -> Result<std::path::PathBuf> {
    let source = std::path::PathBuf::from(string_field(document, "DEF_UnitCatchment_file")?);
    if !logical_field(document, "DEF_UnitCatchment_regional")? {
        return Ok(source);
    }
    let regional = colm_init::unitcatchment_regional::regional_file(&out.join("landdata"));
    colm_init::unitcatchment_regional::verify(&regional, &source)?;
    Ok(regional)
}

/// 主时间重启旁的城市时间重启（`<case>_restart_urban_<date>_…nc`）。
fn lulcc_urban_path(path: &Path) -> Result<std::path::PathBuf> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("a restart path has no file name")?;
    ensure!(name.contains("_restart_"), "{name} is not a CoLM restart");
    Ok(path.with_file_name(name.replacen("_restart_", "_restart_urban_", 1)))
}

/// 主常数重启旁的城市常数重启（`<case>_restart_urb_const_lc<year>_…nc`）。
fn lulcc_urban_const_path(path: &Path) -> Result<std::path::PathBuf> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("a restart path has no file name")?;
    ensure!(
        name.contains("_restart_const_"),
        "{name} is not a CoLM constant restart"
    );
    Ok(path.with_file_name(name.replacen("_restart_const_", "_restart_urb_const_", 1)))
}

/// 一块里每个 patch 的城市类型（`landurban%settyp`，非城市为 0）与城市单元号（`patch2urban`）。
/// `landurban` 按 landpatch 里城市 patch（`patchtype == 1`）的次序建，所以第 k 个城市 patch
/// 就是第 k 个城市单元。
fn lulcc_urban_layout(
    landdata: &Path,
    year: i64,
    block: &str,
    patch_type: &[i64],
) -> Result<(Vec<i64>, Vec<Option<usize>>)> {
    let path = landdata
        .join("landurban")
        .join(format!("{year:04}"))
        .join(format!("landurban_{block}.nc"));
    let classes = colm_init::RestartFile::open(&path)?
        .integers("settyp")?
        .to_vec();
    let mut class = vec![0; patch_type.len()];
    let mut urban = vec![None; patch_type.len()];
    let mut next = 0;
    for (p, &kind) in patch_type.iter().enumerate() {
        if kind == 1 {
            ensure!(
                next < classes.len(),
                "{} has fewer urban units than block {block} has urban patches",
                path.display()
            );
            class[p] = classes[next];
            urban[p] = Some(next);
            next += 1;
        }
    }
    ensure!(
        next == classes.len(),
        "{} has {} urban units but block {block} has {next} urban patches",
        path.display(),
        classes.len()
    );
    Ok((class, urban))
}

/// `lulcc_inventory_trace`（`MOD_Lulcc_Driver.F90:319-342`）：PFT（非 SOLO）或 FAST_PC 时把
/// `patchtypes == 0` 的各类份额按类号次序加进第 1 类（从 0 起的顺序加法；FAST_PC 下作物两类
/// 改加进 CROPLAND），其余类照抄；LCT 时原样返回。
/// `tracer_forcing_lulcc_map` 的 `source_class` 求和是同一条加法链，所以两处都用这份。
fn lulcc_inventory_trace(
    raw: Vec<Vec<f64>>,
    merge_soil_classes: bool,
    fast_pc: bool,
) -> Result<Vec<Vec<f64>>> {
    ensure!(
        raw.iter().flatten().all(|v| v.is_finite() && *v >= 0.0),
        "TRACER LULCC invalid transfer trace"
    );
    if !merge_soil_classes {
        return Ok(raw);
    }
    use colm_init::lulcc_mec::IGBP_PATCHTYPES;
    const CROPLAND: usize = 12;
    ensure!(
        raw.iter().all(|row| row.len() == IGBP_PATCHTYPES.len() + 1),
        "TRACER LULCC transfer trace class count mismatch"
    );
    Ok(raw
        .into_iter()
        .map(|row| {
            let mut mapped = vec![0.0; row.len()];
            mapped[0] = row[0];
            for (c, &value) in row.iter().enumerate().skip(1) {
                // FAST_PC：作物两类（CROPLAND 与 14）各自加进 CROPLAND。
                let dest = if fast_pc && (c == CROPLAND || c == 14) {
                    CROPLAND
                } else if IGBP_PATCHTYPES[c - 1] == 0 {
                    1
                } else {
                    c
                };
                mapped[dest] += value;
            }
            mapped
        })
        .collect())
}

/// MEC 的示踪物重映射输入：每个新 patch 的来源地类份额（`lccpct_patches(np, 0:N)`）与
/// 新旧两侧 patch 的物理面积（`lulcc_patch_areas`，m²）。
struct TracerMec {
    lccpct: Vec<Vec<f64>>,
    new_area: Vec<f64>,
    old_area: Vec<f64>,
}

/// `lulcc_patch_areas`：patch 各像元 `FMA(areaquad, 1e6, area)` 从 0 起累加，有共享像元时再乘
/// `pctshared`（不共享时 Rust 存的是 1，乘 1 是恒等）。
fn lulcc_patch_areas(
    topology: &colm_runtime::spatial::topology::SpatialTopology,
    patches: std::ops::Range<usize>,
) -> Vec<f64> {
    use colm_runtime::spatial::grid::areaquad;
    let pixel = &topology.pixel;
    patches
        .map(|p| {
            let area = topology.cells[p].iter().fold(0.0, |area, &(ilon, ilat)| {
                let (x, y) = (ilon as usize - 1, ilat as usize - 1);
                areaquad(
                    pixel.lat_s[y],
                    pixel.lat_n[y],
                    pixel.lon_w[x],
                    pixel.lon_e[x],
                )
                .mul_add(1.0e6, area)
            });
            area * topology.shared_fraction[p]
        })
        .collect()
}

/// `remap_land_tracer_lulcc_state`：每个新 patch 先取 `allocate_Tracer_Vars` 的分配值，再按方案搬旧池。
///
/// * SAT（没有 `lccpct_patches`）：同单元同类型的旧 patch（`fallback_source`，与 SAT 配对相同）整份抄过来。
/// * MEC（`lulcc_inventory_trace` 加新旧面积）：广延量按面积守恒加权 —— 旧 patch `op` 转给新 patch `np`
///   的面积 `w = old_area(op)·target(np,c)/Σ_nq target(nq,c)`，`target(np,c) = new_area(np)·lcc(np,c)/Σ lcc(np,:)`，
///   和 `FMA(w, old, new)` 依旧 patch 次序累加后除以新面积；强度量（叶片 δ、Péclet）用来源面积权重
///   `lcc(np,c)·old_area(op)/Σ_同类 old_area` 加权平均。没有权重时退回同类型的旧 patch，再没有就保持分配值。
///   之后核对每个输运示踪物的陆面水池质量守恒（`TRC_LULCC_BAL`）。
///
/// 同位素且开了含水层混合（变饱和流、`DEF_TRACER_AQUIFER_MIXING_WATER_MM > 0`）时，上游另查：
/// 特殊地类不能带参考水量，新建的土壤/湿地 patch 不能没有参考水量，否则停机。
// 按新 patch 下标 `np` 同时索引多张表，照上游的 `DO np` 写。
#[allow(clippy::needless_range_loop)]
fn lulcc_land_tracers(
    set: &colm_core::tracer::TracerSet,
    aquifer_mixing: bool,
    new: &colm_init::lulcc::SatSide<'_>,
    new_patch_type: &[i64],
    old: Option<(colm_init::lulcc::SatSide<'_>, &colm_init::RestartFile)>,
    mec: Option<TracerMec>,
    abort_nbad: i64,
) -> Result<Vec<colm_core::tracer::PatchTracerState>> {
    use colm_core::tracer::{PatchTracerState, TracerPools};
    const TINY: f64 = f64::MIN_POSITIVE;
    let nnew = new.patch_class.len();
    let mut states: Vec<PatchTracerState> = (0..nnew)
        .map(|_| PatchTracerState::allocated(set))
        .collect();
    let mut any_reference = false;
    if let Some((old, old_time)) = old {
        let old_states =
            colm_runtime::tracer::read_land_tracer_restart(old_time, set, old.patch_class.len())?
                .context("the old year's restart has no committed land tracer state")?;
        any_reference = old_states.iter().any(|state| state.aquifer_ref_water > 0.0);
        match mec {
            None => {
                for (n, o) in colm_init::lulcc::match_patches(new, &old)? {
                    states[n].pools.clone_from(&old_states[o].pools);
                    states[n].aquifer_ref_water = old_states[o].aquifer_ref_water;
                }
            }
            Some(mec) => {
                lulcc_check_inventory_transfer(new, &old, &mec)?;
                let nold = old.patch_class.len();
                let lcc = |np: usize, c: i64| mec.lccpct[np][c as usize].max(0.0);
                // `lulcc_target_class_area(np, c)`。
                let target = |np: usize, c: i64| -> f64 {
                    let class_sum: f64 = mec.lccpct[np].iter().fold(0.0, |s, &v| s + v.max(0.0));
                    if class_sum <= TINY {
                        0.0
                    } else {
                        mec.new_area[np].max(0.0) * lcc(np, c) / class_sum
                    }
                };
                // `lulcc_source_area_weight(np, op)`。
                let source_weight = |np: usize, op: usize| -> f64 {
                    let w = lcc(np, old.patch_class[op]);
                    if w <= 0.0 {
                        return 0.0;
                    }
                    let class_area = (0..nold)
                        .filter(|&oq| {
                            old.element[oq] == new.element[np]
                                && old.patch_class[oq] == old.patch_class[op]
                        })
                        .fold(0.0, |s, oq| s + mec.old_area[oq].max(0.0));
                    if class_area > TINY {
                        w * mec.old_area[op].max(0.0) / class_area
                    } else {
                        w
                    }
                };
                // `lulcc_mass_transfer_area(np, op)`。
                let transfer = |np: usize, op: usize| -> f64 {
                    let c = old.patch_class[op];
                    let t = target(np, c);
                    if t <= TINY {
                        return 0.0;
                    }
                    let class_target = (0..nnew)
                        .filter(|&nq| new.element[nq] == new.element[np])
                        .fold(0.0, |s, nq| s + target(nq, c));
                    if class_target <= TINY {
                        return 0.0;
                    }
                    mec.old_area[op].max(0.0) * t / class_target
                };
                type Weights = (Vec<(usize, f64)>, f64);
                let weights = |np: usize, intensive: bool| -> Weights {
                    let conserve = !intensive && mec.new_area[np] > TINY;
                    let mut list = Vec::new();
                    let mut wsum = 0.0;
                    for op in 0..nold {
                        if old.element[op] != new.element[np] {
                            continue;
                        }
                        let w = if conserve {
                            transfer(np, op)
                        } else {
                            source_weight(np, op)
                        };
                        if w <= 0.0 {
                            continue;
                        }
                        list.push((op, w));
                        wsum += w;
                    }
                    // `remap_denominator`。
                    let denom = if conserve {
                        mec.new_area[np]
                    } else {
                        wsum.max(TINY)
                    };
                    (list, if wsum > 0.0 { denom } else { 0.0 })
                };
                let fallback = |np: usize| {
                    (0..nold).find(|&op| {
                        old.element[op] == new.element[np]
                            && old.patch_class[op] == new.patch_class[np]
                    })
                };
                type Field = fn(&mut TracerPools) -> &mut f64;
                const MASS: [Field; 16] = [
                    |p| &mut p.ldew_rain,
                    |p| &mut p.ldew_snow,
                    |p| &mut p.wa,
                    |p| &mut p.aquifer_ref_mass,
                    |p| &mut p.wdsrf,
                    |p| &mut p.wetwat,
                    |p| &mut p.surface_residue,
                    |p| &mut p.subsurface_residue,
                    |p| &mut p.canopy_solid,
                    |p| &mut p.surface_solid,
                    |p| &mut p.subsurface_solid,
                    |p| &mut p.waterstorage_solid,
                    |p| &mut p.waterstorage,
                    |p| &mut p.scv,
                    |p| &mut p.leaf_water_moles,
                    |p| &mut p.leaf_iso_storage,
                ];
                const INTENSIVE: [Field; 3] = [
                    |p| &mut p.leaf_delta_e,
                    |p| &mut p.leaf_delta_b,
                    |p| &mut p.leaf_peclet,
                ];
                type Layers = fn(&mut TracerPools) -> &mut [f64; colm_core::tracer::SOISNO_LAYERS];
                const LAYERS: [Layers; 3] = [
                    |p| &mut p.wliq_soisno,
                    |p| &mut p.wice_soisno,
                    |p| &mut p.solid_soisno,
                ];
                let mut old_states = old_states;
                for np in 0..nnew {
                    let (mass_w, mass_denom) = weights(np, false);
                    let (int_w, int_denom) = weights(np, true);
                    let src = fallback(np);
                    // 一个量的重映射：`new = 0`，依旧 patch 次序 `FMA(w, old, new)`，再除分母；
                    // 没有权重时取同类型旧 patch，再没有就保持分配值。
                    let remap = |get: &dyn Fn(&mut PatchTracerState) -> f64,
                                 list: &[(usize, f64)],
                                 denom: f64,
                                 old_states: &mut Vec<PatchTracerState>,
                                 current: f64|
                     -> f64 {
                        if !list.is_empty() {
                            list.iter()
                                .fold(0.0, |v, &(op, w)| w.mul_add(get(&mut old_states[op]), v))
                                / denom
                        } else if let Some(op) = src {
                            get(&mut old_states[op])
                        } else {
                            current
                        }
                    };
                    for itrc in 0..set.len() {
                        for field in MASS {
                            let current = *field(&mut states[np].pools[itrc]);
                            *field(&mut states[np].pools[itrc]) = remap(
                                &|s| *field(&mut s.pools[itrc]),
                                &mass_w,
                                mass_denom,
                                &mut old_states,
                                current,
                            );
                        }
                        for field in INTENSIVE {
                            let current = *field(&mut states[np].pools[itrc]);
                            *field(&mut states[np].pools[itrc]) = remap(
                                &|s| *field(&mut s.pools[itrc]),
                                &int_w,
                                int_denom,
                                &mut old_states,
                                current,
                            );
                        }
                        for layers in LAYERS {
                            for slot in 0..colm_core::tracer::SOISNO_LAYERS {
                                let current = layers(&mut states[np].pools[itrc])[slot];
                                layers(&mut states[np].pools[itrc])[slot] = remap(
                                    &|s| layers(&mut s.pools[itrc])[slot],
                                    &mass_w,
                                    mass_denom,
                                    &mut old_states,
                                    current,
                                );
                            }
                        }
                    }
                    let current = states[np].aquifer_ref_water;
                    states[np].aquifer_ref_water = remap(
                        &|s| s.aquifer_ref_water,
                        &mass_w,
                        mass_denom,
                        &mut old_states,
                        current,
                    );
                }
                lulcc_check_land_water_mass(
                    set,
                    &old_states,
                    &mec.old_area,
                    &states,
                    &mec.new_area,
                    abort_nbad,
                )?;
            }
        }
    }
    let require_reference = aquifer_mixing
        && set
            .tracers
            .iter()
            .any(|tracer| tracer.uses_land_water_transport() && tracer.is_isotope());
    if require_reference || any_reference {
        for (state, &patch_type) in states.iter().zip(new_patch_type) {
            let soil_or_wetland = patch_type == 0 || patch_type == 2;
            ensure!(
                !(state.aquifer_ref_water > 0.0 && !soil_or_wetland),
                "LULCC cannot transfer isotope aquifer reference to special patch"
            );
            ensure!(
                !(require_reference && state.aquifer_ref_water <= 0.0 && soil_or_wetland),
                "LULCC cannot create soil or wetland isotope aquifer without reference"
            );
        }
    }
    Ok(states)
}

/// `compute_*_lulcc_land_water_mass` + `assert_lulcc_land_water_mass_conserved`：逐输运示踪物把各陆面
/// 水池按面积求和（`FMA(pool, area, total)`，面积 ≤ tiny 的 patch 不计），前后之差超过
/// `max(1e-8, 1e-10·max(|前|, |后|, 1))` 的示踪物个数多于 `DEF_TRACER_LULCC_ABORT_NBAD` 就停。
fn lulcc_check_land_water_mass(
    set: &colm_core::tracer::TracerSet,
    old_states: &[colm_core::tracer::PatchTracerState],
    old_area: &[f64],
    new_states: &[colm_core::tracer::PatchTracerState],
    new_area: &[f64],
    abort_nbad: i64,
) -> Result<()> {
    use colm_core::tracer::{PatchTracerState, TracerPools};
    const TINY: f64 = f64::MIN_POSITIVE;
    let total = |states: &[PatchTracerState], areas: &[f64]| -> Vec<f64> {
        let mut total = vec![0.0; set.len()];
        let scalar: [fn(&TracerPools) -> f64; 14] = [
            |p| p.ldew_rain,
            |p| p.ldew_snow,
            |p| p.wa,
            |p| p.aquifer_ref_mass,
            |p| p.wdsrf,
            |p| p.wetwat,
            |p| p.surface_residue,
            |p| p.subsurface_residue,
            |p| p.canopy_solid,
            |p| p.surface_solid,
            |p| p.subsurface_solid,
            |p| p.waterstorage_solid,
            |p| p.waterstorage,
            |p| p.scv,
        ];
        let layered: [fn(&TracerPools) -> &[f64; colm_core::tracer::SOISNO_LAYERS]; 3] =
            [|p| &p.wliq_soisno, |p| &p.wice_soisno, |p| &p.solid_soisno];
        // 上游的累加次序：ldew_rain, ldew_snow, wliq, wice, wa, aquifer_ref_mass, wdsrf, wetwat,
        // 两种残留, solid_soisno, 四种固相, waterstorage, scv。
        let order: [Result<usize, usize>; 17] = [
            Ok(0),
            Ok(1),
            Err(0),
            Err(1),
            Ok(2),
            Ok(3),
            Ok(4),
            Ok(5),
            Ok(6),
            Ok(7),
            Err(2),
            Ok(8),
            Ok(9),
            Ok(10),
            Ok(11),
            Ok(12),
            Ok(13),
        ];
        for item in order {
            for (state, &area) in states.iter().zip(areas) {
                let area = area.max(0.0);
                if area <= TINY {
                    continue;
                }
                match item {
                    Ok(k) => {
                        for (itrc, tracer) in set.tracers.iter().enumerate() {
                            if tracer.uses_land_water_transport() {
                                total[itrc] =
                                    scalar[k](&state.pools[itrc]).mul_add(area, total[itrc]);
                            }
                        }
                    }
                    Err(k) => {
                        for slot in 0..colm_core::tracer::SOISNO_LAYERS {
                            for (itrc, tracer) in set.tracers.iter().enumerate() {
                                if tracer.uses_land_water_transport() {
                                    total[itrc] = layered[k](&state.pools[itrc])[slot]
                                        .mul_add(area, total[itrc]);
                                }
                            }
                        }
                    }
                }
            }
        }
        total
    };
    let before = total(old_states, old_area);
    let after = total(new_states, new_area);
    let mut bad = 0;
    for (itrc, tracer) in set.tracers.iter().enumerate() {
        if !tracer.uses_land_water_transport() {
            continue;
        }
        let scale = before[itrc].abs().max(after[itrc].abs()).max(1.0);
        if (after[itrc] - before[itrc]).abs() > (1.0e-10 * scale).max(1.0e-8) {
            bad += 1;
        }
    }
    ensure!(
        bad <= abort_nbad,
        "TRACER LULCC remap land-water mass conservation failed ({bad} tracer(s))"
    );
    Ok(())
}

/// `lulcc_check_inventory_transfer`：每个单元的旧/新物理面积一致，且按来源地类，旧 patch 的面积与
/// 新 patch 按转移份额推出的来源面积一致（容差 1e-10）。
// 按类下标 `c` 同时索引 `target` 与 `lccpct` 行，照上游的 `DO` 写。
#[allow(clippy::needless_range_loop)]
fn lulcc_check_inventory_transfer(
    new: &colm_init::lulcc::SatSide<'_>,
    old: &colm_init::lulcc::SatSide<'_>,
    mec: &TracerMec,
) -> Result<()> {
    const TINY: f64 = f64::MIN_POSITIVE;
    const CLASSES: usize = 18;
    ensure!(
        mec.lccpct.len() == new.patch_class.len()
            && mec.new_area.len() == new.patch_class.len()
            && mec.old_area.len() == old.patch_class.len(),
        "TRACER LULCC inventory map shape mismatch"
    );
    ensure!(
        mec.old_area
            .iter()
            .chain(&mec.new_area)
            .all(|a| a.is_finite() && *a >= 0.0)
            && mec
                .lccpct
                .iter()
                .flatten()
                .all(|v| v.is_finite() && *v >= 0.0),
        "TRACER LULCC invalid inventory area or transfer trace"
    );
    let elements: std::collections::BTreeSet<i64> =
        old.element.iter().chain(new.element).copied().collect();
    for element in elements {
        let mut source = [0.0f64; CLASSES];
        let mut target = [0.0f64; CLASSES];
        let olds: Vec<usize> = (0..old.element.len())
            .filter(|&p| old.element[p] == element)
            .collect();
        let news: Vec<usize> = (0..new.element.len())
            .filter(|&p| new.element[p] == element)
            .collect();
        ensure!(
            !olds.is_empty() && !news.is_empty(),
            "TRACER LULCC element footprint changed"
        );
        for &op in &olds {
            if mec.old_area[op] <= TINY {
                continue;
            }
            source[old.patch_class[op] as usize] += mec.old_area[op];
        }
        for &np in &news {
            if mec.new_area[np] <= TINY {
                continue;
            }
            let rowsum: f64 = mec.lccpct[np].iter().sum();
            ensure!(
                (rowsum - 1.0).abs() <= 1.0e-10,
                "TRACER LULCC incomplete source trace"
            );
            for c in 0..CLASSES {
                target[c] += mec.new_area[np] * mec.lccpct[np][c] / rowsum;
            }
        }
        let old_total: f64 = source.iter().sum();
        let new_total: f64 = news.iter().map(|&np| mec.new_area[np]).sum();
        ensure!(
            (old_total - new_total).abs() <= 1.0e-10 * old_total.max(new_total).max(1.0),
            "TRACER LULCC element physical footprint changed (element {element}: {old_total} vs {new_total} m2)"
        );
        for c in 0..CLASSES {
            ensure!(
                (source[c] > 0.0) == (target[c] > 0.0)
                    && (source[c] - target[c]).abs() <= 1.0e-10 * source[c].max(target[c]),
                "TRACER LULCC source-class physical area mismatch (element {element}, class {c})"
            );
        }
    }
    Ok(())
}

/// `tracer_forcing_lulcc_map`（IGBP LCT：`source_class` 是恒等映射）。`old`/返回值是 `[patch][species]`
/// 平铺；新 patch 先取默认比值。
///
/// * MEC（有 `lccpct`）：同单元的旧 patch 以 `lcc(np, c_old)·subfrc(op)/Σ_同单元同类 subfrc` 加权；
/// * SAT：新 patch 的类型在旧侧有面积时只取同类旧 patch，否则取同单元全部旧 patch，以 `subfrc` 加权。
///
/// 只有一个来源时直接抄（不经乘除），否则 `Σ w·old / Σ w`；没有来源保持默认比值。
#[allow(clippy::too_many_arguments)]
fn lulcc_forcing_cache(
    old: &[f64],
    default: &[f64],
    new_class: &[i64],
    new_element: &[i64],
    old_class: &[i64],
    old_element: &[i64],
    old_area: &[f64],
    lccpct: Option<&[Vec<f64>]>,
) -> Result<Vec<f64>> {
    const TINY: f64 = f64::MIN_POSITIVE;
    let ns = default.len();
    let (nnew, nold) = (new_class.len(), old_class.len());
    ensure!(
        old.len() == nold * ns && old_area.len() == nold,
        "tracer forcing LULCC map shape mismatch"
    );
    ensure!(
        old.iter().all(|v| v.is_finite()),
        "tracer forcing LULCC old cache contains non-finite values"
    );
    let class_area: Vec<f64> = (0..nold)
        .map(|op| {
            (0..nold)
                .filter(|&oq| old_element[oq] == old_element[op] && old_class[oq] == old_class[op])
                .fold(0.0, |s, oq| s + old_area[oq])
        })
        .collect();
    let mut mapped: Vec<f64> = (0..nnew).flat_map(|_| default.iter().copied()).collect();
    for np in 0..nnew {
        let same_class = lccpct.is_none()
            && (0..nold).any(|op| {
                old_element[op] == new_element[np]
                    && old_class[op] == new_class[np]
                    && old_area[op] > TINY
            });
        let mut total = 0.0;
        let mut count = 0;
        let mut source = 0;
        let row = &mut mapped[np * ns..(np + 1) * ns];
        for op in 0..nold {
            if old_element[op] != new_element[np] {
                continue;
            }
            let weight = match lccpct {
                Some(lcc) => {
                    let w = lcc[np][old_class[op] as usize];
                    if w <= 0.0 || class_area[op] <= TINY {
                        continue;
                    }
                    w * old_area[op] / class_area[op]
                }
                None => {
                    if same_class && old_class[op] != new_class[np] {
                        continue;
                    }
                    old_area[op].max(0.0)
                }
            };
            if weight <= 0.0 {
                continue;
            }
            if total == 0.0 {
                row.fill(0.0);
            }
            for k in 0..ns {
                row[k] = weight.mul_add(old[op * ns + k], row[k]);
            }
            total += weight;
            count += 1;
            source = op;
        }
        if total > 0.0 {
            if count == 1 {
                row.copy_from_slice(&old[source * ns..(source + 1) * ns]);
            } else {
                for value in row.iter_mut() {
                    *value /= total;
                }
            }
        }
    }
    Ok(mapped)
}

/// `init_methane_wetland_fraction_cache`：每个单元里湿地 patch 的面积占土壤 + 湿地 patch 面积的份额，
/// 存进各 patch 的甲烷站点（`wetland_frac_per_patch`）。patch 面积是各像元 `areaquad` 从 0 起相加、
/// 共享时再乘 `pctshared`；单元里没有活跃面积时为 0。
// `max(0).min(1)` 照搬上游 `min(max(…))` 的 NaN 行为，不换成 `clamp`。
#[allow(clippy::manual_clamp)]
fn methane_wetland_fractions(
    templates: &mut [StandardLctRestartTemplate],
    topology: &colm_runtime::spatial::topology::SpatialTopology,
) {
    use colm_runtime::spatial::grid::areaquad;
    let pixel = &topology.pixel;
    let area: Vec<f64> = topology
        .cells
        .iter()
        .zip(&topology.shared_fraction)
        .map(|(cells, &shared)| {
            cells.iter().fold(0.0, |sum, &(ilon, ilat)| {
                let (x, y) = (ilon as usize - 1, ilat as usize - 1);
                sum + areaquad(
                    pixel.lat_s[y],
                    pixel.lat_n[y],
                    pixel.lon_w[x],
                    pixel.lon_e[x],
                )
            }) * shared.max(0.0)
        })
        .collect();
    let mut active = std::collections::HashMap::<i64, f64>::new();
    let mut wetland = std::collections::HashMap::<i64, f64>::new();
    for (p, template) in templates.iter().enumerate() {
        if area[p] <= 0.0 {
            continue;
        }
        let element = topology.element[p];
        if template.patch_type == 0 || template.patch_type == 2 {
            *active.entry(element).or_insert(0.0) += area[p];
        }
        if template.patch_type == 2 {
            *wetland.entry(element).or_insert(0.0) += area[p];
        }
    }
    for (p, template) in templates.iter_mut().enumerate() {
        let element = topology.element[p];
        let fraction = match active.get(&element) {
            Some(&act) if act > 0.0 => (wetland.get(&element).copied().unwrap_or(0.0) / act)
                .max(0.0)
                .min(1.0),
            _ => 0.0,
        };
        if let Some((_, site)) = template.bgc.as_mut().and_then(|bgc| bgc.methane.as_mut()) {
            site.wetland_fraction = fraction;
        }
    }
}

/// `read_methane_giems`：方案 5 时读 `DEF_file_GIEMS`，按每个 patch 的中心 `(patchlonr, patchlatr)`
/// 取最近像元的月序列挂到甲烷静态量上。文件只读一次，所有 patch 一起映射。
fn methane_giems(
    document: &colm_namelist::Document,
    templates: &mut [StandardLctRestartTemplate],
    coordinates: impl FnOnce() -> Result<Vec<(f64, f64)>>,
) -> Result<()> {
    let giems_patches = || {
        templates.iter().any(|template| {
            template
                .bgc
                .as_ref()
                .and_then(|bgc| bgc.methane.as_ref())
                .is_some_and(|(setup, _)| setup.scheme == 5)
        })
    };
    if !giems_patches() {
        return Ok(());
    }
    let path = string_field(document, "DEF_file_GIEMS")?;
    let path = path.trim();
    ensure!(
        !path.is_empty() && path != "null",
        " ***** ERROR: DEF_wetland_finundation_scheme=5 requires DEF_file_GIEMS."
    );
    let coordinates = coordinates()?;
    ensure!(
        coordinates.len() == templates.len(),
        "GIEMS methane input requires coordinates for every worker-local patch."
    );
    let series = colm_runtime::methane::read_giems(path, &coordinates)?;
    for (template, giems) in templates.iter_mut().zip(series) {
        if let Some((_, site)) = template.bgc.as_mut().and_then(|bgc| bgc.methane.as_mut()) {
            site.giems = Some(giems);
        }
    }
    Ok(())
}

/// `start` 之后第一个做 LULCC 的年末。上游的判据（`CoLM.F90:580-581`）看的是**下一年**
/// `jdate(1)`：`>= 2000` 每年换，`> 1985` 时只在下一年是 5 的倍数时换（换入 1990、1995、2000）。
fn lulcc_year_end(start: CalendarTime) -> CalendarTime {
    let mut end = year_end(start);
    while !lulcc_changes_into(end.year + 1) {
        end = year_end(CalendarTime {
            year: end.year + 1,
            julian_day: 1,
            seconds: 0,
        });
    }
    end
}

fn lulcc_changes_into(year: i32) -> bool {
    year >= 2000 || (year > 1985 && year % 5 == 0)
}

/// `start` 所在年份的最后一步的终点，按 CoLM 的写法 `(year, 365|366, 86400)`。
fn year_end(start: CalendarTime) -> CalendarTime {
    CalendarTime {
        year: start.year,
        julian_day: if colm_core::is_leap_year(start.year) {
            366
        } else {
            365
        },
        seconds: 86_400,
    }
}

fn calendar_key(time: CalendarTime) -> (i32, u16, u32) {
    (time.year, time.julian_day, time.seconds)
}

/// 换年两侧的时刻（续跑文件名里的日期）。
struct TransitionTimes {
    /// 旧状态所在的时刻（LULCC 年末或回卷点）。
    old: CalendarTime,
    /// 新状态的时刻：换年时与 `old` 相同，回卷时是运行起点。
    new: CalendarTime,
    /// 多遍预热的回卷（upstream-bugs 第 50 条）：换回起始年，一律 SAT，冷启动不落在 `restart/`。
    rewind: bool,
}

struct LulccYears {
    old: i64,
    new: i64,
    /// 过渡这一步写的历史旁车要记下 history 频率。
    history_frequency: colm_hist::schedule::HistoryFrequency,
}

/// 上游 `LulccDriver`（SAT）在两段之间做的事：
///
/// 1. `LulccInitialize`：按新一年的 landdata 整套冷启动（写新年份的常数重启）。这里调同目录的
///    `mkinidata-rs`，namelist 的起始日期改成新年第一天；它顺带写出的冷时间重启只作合并的底本。
/// 2. `REST_LulccTimeVariables`：同单元同类型的 patch 抄回旧状态，写成新年份的续跑。
/// 3. `grid_riverlake_flow_lulcc`：河道状态照旧，只补 `volwater_ucat`。
fn lulcc_transition(
    case: &SpatialCase<'_>,
    years: LulccYears,
    times: TransitionTimes,
    old_dir: &Path,
    target: &Path,
    river: Option<SegmentRiverEnd>,
    old_baseflow: &[f64],
) -> Result<Transition> {
    use colm_runtime::spatial::topology::SpatialTopology;
    let SpatialCase {
        name,
        document,
        out,
        ..
    } = *case;
    let old_label = date_label(times.old);
    let new_label = date_label(times.new);
    let landdata = out.join("landdata");
    let old_topology = SpatialTopology::read(&landdata, i32::try_from(years.old)?)?;
    let new_topology =
        SpatialTopology::read(&landdata, i32::try_from(years.new)?).with_context(|| {
            format!(
                "DEF_USE_LULCC needs the {} landdata; run mksrfdata with DEF_LC_YEAR = {} first",
                years.new, years.new
            )
        })?;
    let restart_root = out.join("restart");
    let scratch = restart_root.join(LULCC_SCRATCH);
    // 冷启动写在哪：换年写进 `restart/`（新年的日期目录本来就是续跑要落的地方）；回卷的冷启动日期
    // 是运行起点，写进 `restart/` 会盖掉初值，放到临时目录。
    let cold_root = if times.rewind {
        scratch.join("rewind-cold")
    } else {
        restart_root.clone()
    };
    // 1. 新一年的冷启动；回卷时就是起始年在运行起点的冷启动，namelist 原样用。
    let mut cold_document = document.clone();
    let cold_fields = if times.rewind {
        Vec::new()
    } else {
        vec![
            ("DEF_simulation_time%start_year", years.new),
            ("DEF_simulation_time%start_month", 1),
            ("DEF_simulation_time%start_day", 1),
            ("DEF_simulation_time%start_sec", 0),
            ("DEF_LC_YEAR", years.new),
        ]
    };
    for (field, value) in cold_fields {
        if cold_document.get(field).is_some() {
            cold_document.set(field, Value::Int(value))?;
        } else {
            cold_document.insert(field, Value::Int(value), "nl_colm")?;
        }
    }
    std::fs::create_dir_all(&scratch)
        .with_context(|| format!("cannot create {}", scratch.display()))?;
    let namelist = scratch.join(format!("mkinidata_lc{:04}.nml", years.new));
    std::fs::write(&namelist, cold_document.to_string())
        .with_context(|| format!("cannot write {}", namelist.display()))?;
    let executable = sibling_executable("mkinidata-rs")?;
    let output = std::process::Command::new(&executable)
        .arg(&namelist)
        .args(["--land-cover", "igbp"])
        .args(
            times
                .rewind
                .then(|| ["--restart-dir".as_ref(), cold_root.as_os_str()])
                .into_iter()
                .flatten(),
        )
        .output()
        .with_context(|| format!("cannot start {}", executable.display()))?;
    ensure!(
        output.status.success(),
        "the LULCC cold start for {} failed:\n{}{}",
        years.new,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // 2. SAT，逐块（单元不跨块，按块配对与上游按 worker 配对等价）。
    // 回卷没有从后一年到起始年的转移矩阵（mksrfdata 只写上一年到本年的），一律 SAT。
    let mec = (!times.rewind && integer_field(document, "DEF_LULCC_SCHEME")? == 2)
        .then(|| -> Result<_> {
            Ok(colm_init::lulcc_mec::MecOptions {
                plant_hydraulics: logical_field(document, "DEF_USE_PLANTHYDRAULICS")?,
                ozone_stress: logical_field(document, "DEF_USE_OZONESTRESS")?,
                // `MOD_Namelist` 在 van Genuchten 下强开变饱和流，用生效值。
                variably_saturated_flow: case.physics.variably_saturated_flow,
                vegetation_snow: case.physics.vegetation_snow,
                // 用已按 schema 解析好的值（缺省是 Fortran 字面量 `1.0_r8`）。
                snow_cover_exponent: case.physics.snow_cover_exponent,
                campbell_soil: case.physics.hydraulic_model == colm_core::HydraulicModel::Campbell,
            })
        })
        .transpose()?;
    let options = colm_init::lulcc::SatOptions {
        plant_hydraulics: logical_field(document, "DEF_USE_PLANTHYDRAULICS")?,
        ozone_stress: logical_field(document, "DEF_USE_OZONESTRESS")?,
        irrigation: false,
    };
    let const_path = |year: i64, block: &str| {
        restart_root
            .join("const")
            .join(format!("{name}_restart_const_lc{year:04}_{block}.nc"))
    };
    let time_name = |label: &str, year: i64, block: &str| {
        format!("{name}_restart_{label}_lc{year:04}_{block}.nc")
    };
    let old_time_path = |block: &str| {
        old_dir
            .join(&old_label)
            .join(time_name(&old_label, years.old, block))
    };
    // 过渡这一步的历史区间已关（`run_spatial_segment` 核对过），旁车是空窗口。
    // 新年各块的城市标记（`patchtype == 1`），按块顺序拼接；旁车 `urban` 维按块取。
    let new_urban = new_topology
        .blocks
        .iter()
        .map(|(block, _)| -> Result<Vec<bool>> {
            Ok(colm_init::RestartFile::open(const_path(years.new, block))?
                .integers("patchtype")?
                .iter()
                .map(|&kind| kind == 1)
                .collect())
        })
        .collect::<Result<Vec<_>>>()?
        .concat();
    let history_restart = HistoryRestart {
        config: colm_runtime::history_sidecar::SidecarConfig {
            frequency_code: history_frequency_code(years.history_frequency),
            urban_run: logical_field(document, "DEF_URBAN_RUN")?,
            urban_patches: new_urban.iter().filter(|&&is_urban| is_urban).count(),
            pft_or_pc: case.physics.use_pft || case.physics.use_pc,
            bgc: false,
            crop: false,
            river_lake_flow: true,
        },
        window: None,
        tracer_raw: None,
        urban: new_urban,
    };
    // 输运示踪物（`remap_land_tracer_lulcc_state` 的 SAT 形态，不带转移份额）。
    let land_tracers = colm_runtime::tracer::tracer_set_from_document(document)?
        .filter(|set| set.transport_indices().next().is_some())
        .map(|set| -> Result<_> {
            Ok((
                set,
                real_field(document, "DEF_TRACER_AQUIFER_MIXING_WATER_MM")?,
            ))
        })
        .transpose()?;
    // 示踪物强迫的"最近有效值"缓存（`tracer_forcing_lulcc_remap`）：默认比值与旧侧的 `subfrc`。
    let forcing_defaults = match &land_tracers {
        Some(_) => colm_runtime::tracer::TracerRuntime::from_document(document)?
            .filter(|runtime| runtime.forcing_specs.iter().any(|specs| !specs.is_empty()))
            .map(|runtime| (runtime.precip_ratio.clone(), runtime.vapor_ratio.clone())),
        None => None,
    };
    let old_subfrc = match forcing_defaults {
        Some(_) => Some(
            colm_runtime::spatial::history::ElementGroups::from_topology(&old_topology)?.fractions,
        ),
        None => None,
    };
    // `(DEF_USE_PFT .and. .not. DEF_SOLO_PFT) .or. DEF_FAST_PC`：份额按土壤 patch（第 1 类）归并。
    // `physics.use_pft` 在 PC 下也为真，这里看 namelist 本身；`MOD_Namelist` 在 PFT/LCT 下把
    // `DEF_FAST_PC` 置假、在 PC 下把 `DEF_SOLO_PFT` 置假，生效值照此推出。
    let use_pc = logical_field(document, "DEF_USE_PC")?;
    let fast_pc = use_pc && logical_field(document, "DEF_FAST_PC")?;
    let merge_soil_classes = (logical_field(document, "DEF_USE_PFT")?
        && !logical_field(document, "DEF_SOLO_PFT")?)
        || fast_pc;
    let mut written = Vec::with_capacity(new_topology.blocks.len());
    // `scale_baseflow` 跟着 SAT 配对走（`REST_LulccTimeVariables`，upstream-bugs 第 43 条 vendor 已修）：
    // 配上的新 patch 取旧值，新出现的取 `Opt_Baseflow_init` 的缺省 1。
    ensure!(
        old_baseflow.len() == old_topology.patch_count(),
        "the old year's scale_baseflow has {} values for {} patches",
        old_baseflow.len(),
        old_topology.patch_count()
    );
    let mut new_baseflow = vec![1.0; new_topology.patch_count()];
    let mut previous = vec![None; new_topology.patch_count()];
    for (block, patches) in &new_topology.blocks {
        let cold_path = cold_root
            .join(&new_label)
            .join(time_name(&new_label, years.new, block));
        let cold = colm_init::RestartFile::open(&cold_path)?;
        // PFT/PC：冷启动同时写了 PFT 时间重启，SAT 再逐 PFT 抄旧值（`REST_LulccTimeVariables`）。
        let use_pft = case.physics.use_pft;
        let use_urban = logical_field(document, "DEF_URBAN_RUN")?;
        let cold_pft_path = colm_runtime::pft::pft_restart_path(&cold_path)?;
        let cold_pft = use_pft
            .then(|| colm_init::RestartFile::open(&cold_pft_path))
            .transpose()?;
        std::fs::remove_file(&cold_path)
            .with_context(|| format!("cannot remove {}", cold_path.display()))?;
        if use_pft {
            std::fs::remove_file(&cold_pft_path)
                .with_context(|| format!("cannot remove {}", cold_pft_path.display()))?;
        }
        // 城市：冷启动同时写了城市时间重启，城市 SAT 以它为底。
        let cold_urban_path = lulcc_urban_path(&cold_path)?;
        let cold_urban = use_urban
            .then(|| colm_init::RestartFile::open(&cold_urban_path))
            .transpose()?;
        if use_urban {
            std::fs::remove_file(&cold_urban_path)
                .with_context(|| format!("cannot remove {}", cold_urban_path.display()))?;
        }
        let mut urban_overrides = Vec::new();
        let new_const = colm_init::RestartFile::open(const_path(years.new, block))?;
        let mut pft_overrides = Vec::new();
        let overrides = match old_topology.blocks.iter().find(|(old, _)| old == block) {
            Some((_, old_patches)) => {
                let old_const = colm_init::RestartFile::open(const_path(years.old, block))?;
                let old_time = colm_init::RestartFile::open(old_time_path(block))?;
                let new_element = &new_topology.element[patches.clone()];
                let old_element = &old_topology.element[old_patches.clone()];
                // 城市：每个 patch 的城市类型与城市单元号（两年各自的 `landurban`）。
                let urban_layouts = if use_urban {
                    Some((
                        lulcc_urban_layout(
                            &landdata,
                            years.new,
                            block,
                            new_const.integers("patchtype")?,
                        )?,
                        lulcc_urban_layout(
                            &landdata,
                            years.old,
                            block,
                            old_const.integers("patchtype")?,
                        )?,
                    ))
                } else {
                    None
                };
                let sat = colm_init::lulcc::same_type_assignment(
                    &colm_init::lulcc::SatSide {
                        time: &cold,
                        patch_class: new_const.integers("patchclass")?,
                        element: new_element,
                        urban_class: urban_layouts.as_ref().map(|(new, _)| new.0.as_slice()),
                    },
                    &colm_init::lulcc::SatSide {
                        time: &old_time,
                        patch_class: old_const.integers("patchclass")?,
                        element: old_element,
                        urban_class: urban_layouts.as_ref().map(|(_, old)| old.0.as_slice()),
                    },
                    options,
                )
                .with_context(|| {
                    format!("cannot carry the {} state of block {block} over", years.old)
                })?;
                for (n, o) in colm_init::lulcc::match_patches(
                    &colm_init::lulcc::SatSide {
                        time: &cold,
                        patch_class: new_const.integers("patchclass")?,
                        element: new_element,
                        urban_class: urban_layouts.as_ref().map(|(new, _)| new.0.as_slice()),
                    },
                    &colm_init::lulcc::SatSide {
                        time: &old_time,
                        patch_class: old_const.integers("patchclass")?,
                        element: old_element,
                        urban_class: urban_layouts.as_ref().map(|(_, old)| old.0.as_slice()),
                    },
                )? {
                    new_baseflow[patches.start + n] = old_baseflow[old_patches.start + o];
                    previous[patches.start + n] = Some(old_patches.start + o);
                }
                let mut sat = sat;
                // PFT 常数与区间留给 MEC 的 PFT 尾段用。
                let mut pft_side = None;
                if let Some(cold_pft) = &cold_pft {
                    let old_pft = colm_init::RestartFile::open(
                        colm_runtime::pft::pft_restart_path(&old_time_path(block))?,
                    )?;
                    let pft_const = |year: i64| -> Result<colm_init::RestartFile> {
                        colm_init::RestartFile::open(colm_runtime::pft::pft_restart_path(
                            &const_path(year, block),
                        )?)
                    };
                    let (new_pft_const, old_pft_const) =
                        (pft_const(years.new)?, pft_const(years.old)?);
                    let ranges = |year: i64| {
                        colm_runtime::pft::spatial_pft_ranges(
                            &landdata,
                            i32::try_from(year)?,
                            block,
                            case.physics.land_cover_scheme,
                            false,
                        )
                    };
                    let (new_ranges, old_ranges) = (ranges(years.new)?, ranges(years.old)?);
                    let (pft_sat, ldew) = colm_init::lulcc::pft_same_type_assignment(
                        &colm_init::lulcc::SatSide {
                            time: &cold,
                            patch_class: new_const.integers("patchclass")?,
                            element: new_element,
                            urban_class: None,
                        },
                        &colm_init::lulcc::SatSide {
                            time: &old_time,
                            patch_class: old_const.integers("patchclass")?,
                            element: old_element,
                            urban_class: None,
                        },
                        &colm_init::lulcc::PftSatSide {
                            time: cold_pft,
                            pft_class: new_pft_const.integers("pftclass")?,
                            ranges: &new_ranges,
                            patch_type: new_const.integers("patchtype")?,
                        },
                        &colm_init::lulcc::PftSatSide {
                            time: &old_pft,
                            pft_class: old_pft_const.integers("pftclass")?,
                            ranges: &old_ranges,
                            patch_type: old_const.integers("patchtype")?,
                        },
                        new_pft_const.floats("pftfrac")?,
                        options,
                    )
                    .with_context(|| {
                        format!(
                            "cannot carry the {} PFT state of block {block} over",
                            years.old
                        )
                    })?;
                    pft_overrides = pft_sat;
                    // `ldew(np) = sum(ldew_p*pftfrac)` 覆盖 patch 级照抄来的那个值。
                    let ldew_override = sat
                        .iter_mut()
                        .find(|o| o.name == "ldew")
                        .context("SAT always overrides ldew")?;
                    for (np, value) in ldew {
                        ldew_override.values[np] = value;
                    }
                    pft_side = Some((new_pft_const, new_ranges));
                }
                // 城市（`:863-979`）：同类型同城市类型的单元抄旧城市状态，再按新年的
                // `froof/fgper` 重组 patch 的 `wliq/wice_soisno` 与 `scv`。
                if let (Some(cold_urban), Some((new_urban, old_urban))) =
                    (&cold_urban, &urban_layouts)
                {
                    let old_urban_time =
                        colm_init::RestartFile::open(lulcc_urban_path(&old_time_path(block))?)?;
                    let pairs = colm_init::lulcc::match_patches(
                        &colm_init::lulcc::SatSide {
                            time: &cold,
                            patch_class: new_const.integers("patchclass")?,
                            element: new_element,
                            urban_class: Some(&new_urban.0),
                        },
                        &colm_init::lulcc::SatSide {
                            time: &old_time,
                            patch_class: old_const.integers("patchclass")?,
                            element: old_element,
                            urban_class: Some(&old_urban.0),
                        },
                    )?;
                    let (urban, targets) = colm_init::lulcc::urban_same_type_assignment(
                        &pairs,
                        &colm_init::lulcc::UrbanSide {
                            time: cold_urban,
                            patch_to_urban: &new_urban.1,
                        },
                        &colm_init::lulcc::UrbanSide {
                            time: &old_urban_time,
                            patch_to_urban: &old_urban.1,
                        },
                    )
                    .with_context(|| {
                        format!(
                            "cannot carry the {} urban state of block {block} over",
                            years.old
                        )
                    })?;
                    let urban_const = colm_init::RestartFile::open(lulcc_urban_const_path(
                        &const_path(years.new, block),
                    )?)?;
                    let patch_row = cold.dimension("soilsnow")?;
                    colm_init::lulcc::recompose_urban_patch_water(
                        &targets,
                        &urban,
                        cold_urban,
                        urban_const.floats("WT_ROOF")?,
                        urban_const.floats("WTROAD_PERV")?,
                        &mut sat,
                        patch_row,
                    )?;
                    urban_overrides = urban;
                }
                // MEC 的城市段要旧城市重启与两侧城市布局。
                let old_urban_time = match &urban_layouts {
                    Some(_) => Some(colm_init::RestartFile::open(lulcc_urban_path(
                        &old_time_path(block),
                    )?)?),
                    None => None,
                };
                let urban_const = match &urban_layouts {
                    Some(_) => Some(colm_init::RestartFile::open(lulcc_urban_const_path(
                        &const_path(years.new, block),
                    )?)?),
                    None => None,
                };
                // MEC（`DEF_LULCC_SCHEME = 2`）：SAT 之后按转移份额混合份额有变化的 patch。
                match mec {
                    Some(mec_options) => {
                        let lccpct =
                            read_lulcc_transfer_trace(&landdata, years.new, block, patches.len())?;
                        let pft = match (&cold_pft, &pft_side) {
                            (Some(cold_pft), Some((new_pft_const, new_ranges))) => {
                                Some(colm_init::lulcc_mec::MecPft {
                                    time: cold_pft,
                                    sat: std::mem::take(&mut pft_overrides),
                                    pft_class: new_pft_const.integers("pftclass")?,
                                    pftfrac: new_pft_const.floats("pftfrac")?,
                                    htop: new_pft_const.floats("htop_p")?,
                                    hbot: new_pft_const.floats("hbot_p")?,
                                    ranges: new_ranges,
                                    merge_soil_classes,
                                    fast_pc,
                                })
                            }
                            _ => None,
                        };
                        let urban =
                            match (&cold_urban, &urban_layouts, &old_urban_time, &urban_const) {
                                (
                                    Some(cold_urban),
                                    Some((new_urban, old_urban)),
                                    Some(old_urban_time),
                                    Some(urban_const),
                                ) => Some(colm_init::lulcc_mec::MecUrban {
                                    new_time: cold_urban,
                                    sat: std::mem::take(&mut urban_overrides),
                                    old_time: old_urban_time,
                                    new_class: &new_urban.0,
                                    new_urban: &new_urban.1,
                                    old_class: &old_urban.0,
                                    old_urban: &old_urban.1,
                                    froof: urban_const.floats("WT_ROOF")?,
                                    fgper: urban_const.floats("WTROAD_PERV")?,
                                }),
                                _ => None,
                            };
                        let result = colm_init::lulcc_mec::mass_energy_conserve(
                            &colm_init::lulcc_mec::MecInputs {
                                new_time: &cold,
                                new_const: &new_const,
                                new_element,
                                old_time: &old_time,
                                old_const: &old_const,
                                old_element,
                                lccpct: &lccpct,
                                pft,
                                urban,
                            },
                            sat,
                            mec_options,
                        )
                        .with_context(|| {
                            format!("cannot conserve mass and energy in block {block}")
                        })?;
                        if cold_pft.is_some() {
                            pft_overrides = result.pft;
                        }
                        if cold_urban.is_some() {
                            urban_overrides = result.urban;
                        }
                        result.patch
                    }
                    None => sat,
                }
            }
            None => Vec::new(),
        };
        let path = target
            .join(&new_label)
            .join(time_name(&new_label, years.new, block));
        std::fs::create_dir_all(path.parent().expect("a restart path has a parent"))?;
        cold.write_with(&path, &overrides)?;
        if let Some(cold_pft) = &cold_pft {
            cold_pft.write_with(&colm_runtime::pft::pft_restart_path(&path)?, &pft_overrides)?;
        }
        if let Some(cold_urban) = &cold_urban {
            cold_urban.write_with(&lulcc_urban_path(&path)?, &urban_overrides)?;
        }
        if let Some((set, mixing)) = &land_tracers {
            let old = match old_topology.blocks.iter().find(|(old, _)| old == block) {
                Some((_, old_patches)) => Some((
                    colm_init::RestartFile::open(const_path(years.old, block))?,
                    colm_init::RestartFile::open(old_time_path(block))?,
                    old_patches.clone(),
                )),
                None => None,
            };
            let states = lulcc_land_tracers(
                set,
                *mixing > 0.0 && case.physics.variably_saturated_flow,
                &colm_init::lulcc::SatSide {
                    time: &cold,
                    patch_class: new_const.integers("patchclass")?,
                    element: &new_topology.element[patches.clone()],
                    urban_class: None,
                },
                new_const.integers("patchtype")?,
                old.as_ref()
                    .map(|(old_const, old_time, old_patches)| -> Result<_> {
                        Ok((
                            colm_init::lulcc::SatSide {
                                time: old_time,
                                patch_class: old_const.integers("patchclass")?,
                                element: &old_topology.element[old_patches.clone()],
                                urban_class: None,
                            },
                            old_time,
                        ))
                    })
                    .transpose()?,
                // MEC：`lulcc_inventory_trace`（IGBP LCT 下就是 `lccpct_patches`）与新旧 patch 的物理面积。
                match (mec.is_some(), old.as_ref()) {
                    (true, Some((_, _, old_patches))) => Some(TracerMec {
                        lccpct: lulcc_inventory_trace(
                            read_lulcc_transfer_trace(&landdata, years.new, block, patches.len())?,
                            merge_soil_classes,
                            fast_pc,
                        )?,
                        new_area: lulcc_patch_areas(&new_topology, patches.clone()),
                        old_area: lulcc_patch_areas(&old_topology, old_patches.clone()),
                    }),
                    _ => None,
                },
                integer_field(document, "DEF_TRACER_LULCC_ABORT_NBAD")?,
            )
            .with_context(|| format!("cannot carry the tracers of block {block} over"))?;
            let cache = match (&forcing_defaults, &old, &old_subfrc) {
                (
                    Some((precip_default, vapor_default)),
                    Some((old_const, old_time, old_patches)),
                    Some(subfrc),
                ) => {
                    let nvars = usize::try_from(
                        old_time
                            .integers("trc_forcing_cache_count")?
                            .first()
                            .copied()
                            .unwrap_or(0),
                    )?;
                    let identity = old_time
                        .integers("trc_forcing_cache_identity")?
                        .iter()
                        .map(|&v| i32::try_from(v))
                        .collect::<Result<Vec<_>, _>>()?;
                    let new_class = new_const.integers("patchclass")?;
                    let old_class = old_const.integers("patchclass")?;
                    // `source_class` 把土壤类并进第 1 类，与库存映射是同一条加法链。
                    let lccpct = match mec {
                        Some(_) => Some(lulcc_inventory_trace(
                            read_lulcc_transfer_trace(&landdata, years.new, block, patches.len())?,
                            merge_soil_classes,
                            fast_pc,
                        )?),
                        None => None,
                    };
                    let remap = |old: &[f64], default: &[f64]| {
                        lulcc_forcing_cache(
                            old,
                            default,
                            new_class,
                            &new_topology.element[patches.clone()],
                            old_class,
                            &old_topology.element[old_patches.clone()],
                            &subfrc[old_patches.clone()],
                            lccpct.as_deref(),
                        )
                    };
                    let precip =
                        remap(old_time.floats("trc_forcing_precip_last")?, precip_default)?;
                    let vapor = remap(old_time.floats("trc_forcing_vapor_last")?, vapor_default)?;
                    Some((nvars, identity, precip, vapor))
                }
                _ => None,
            };
            colm_runtime::tracer::write_land_tracer_restart(
                &path,
                set,
                &states.iter().collect::<Vec<_>>(),
                *mixing,
                cache
                    .as_ref()
                    .map(
                        |(nvars, identity, precip, vapor)| colm_runtime::tracer::ForcingCache {
                            nvars: *nvars,
                            ntracers: set.len(),
                            identity: std::sync::Arc::new(identity.clone()),
                            precip,
                            vapor,
                        },
                    )
                    .as_ref(),
            )?;
        }
        // 过渡这一步的历史区间已关，旁车不带示踪物部分。
        mark_history_restart_with_river(
            &path,
            &history_restart,
            Some(patches.clone()),
            false,
            None,
        )?;
        written.push(path);
    }
    // 合并续跑不留在 `restart/` 时，冷启动建的日期目录空了就收掉。
    if times.rewind {
        std::fs::remove_dir_all(&cold_root)
            .with_context(|| format!("cannot remove {}", cold_root.display()))?;
    } else if target != restart_root.as_path() {
        let _ = std::fs::remove_dir(restart_root.join(&new_label));
    }
    // 3. 河道：网络不变，状态接着用。
    let river = if let Some((mut state, river_tracers, sediment)) = river {
        let network = colm_runtime::river::network::RiverNetwork::read(
            &unit_catchment_file(document, out)?,
            logical_field(document, "DEF_GridRiverLake_FloodplainStorageFix")?,
        )?;
        // `grid_riverlake_flow_lulcc`：河道时变量原样保留（上游 hold/restore），堤内蓄量与分汊
        // 路径状态在各自模块里本就不动；之后重建 `volwater_ucat`（有堤单元流域只补堤外可见那份），
        // 开分汊时上一子步水深取当前水深。漫滩回馈与 LULCC 同开上游自己就拒绝。
        let reservoir = if integer_field(document, "DEF_Reservoir_Method")? > 0 {
            Some(
                colm_runtime::river::reservoir::Reservoir::read_with_regional(
                    Path::new(&string_field(document, "DEF_ReservoirPara_file")?),
                    &network,
                    integer_field(document, "DEF_Reservoir_Method")?,
                    regional_catchment(document, out)?.as_deref(),
                )?,
            )
        } else {
            None
        };
        let levee = if logical_field(document, "DEF_USE_LEVEE")? {
            let reservoir_cells = reservoir.as_ref().map_or_else(
                || vec![false; network.len()],
                |r| r.of_catchment.iter().map(Option::is_some).collect(),
            );
            Some(colm_runtime::river::levee::Levee::read(
                &unit_catchment_file(document, out)?,
                &network,
                &reservoir_cells,
            )?)
        } else {
            None
        };
        colm_runtime::river::rebuild_volwater(&network, &mut state, levee.as_ref());
        if let Some(bif) = state.bifurcation.as_mut() {
            bif.wdsrf_prev.clone_from(&state.wdsrf);
        }
        let path = river_restart_path(target, name, &new_label, years.new);
        colm_runtime::river::restart::write_river_state(
            &path,
            &network,
            &state,
            reservoir.as_ref().map(|r| r.identity()).as_deref(),
            u8::try_from(integer_field(document, "DEF_REST_CompressLevel")?)
                .context("DEF_REST_CompressLevel must fit 0..=9")?,
        )?;
        // 河道示踪物：上游在内存里原样留着（`grid_riverlake_flow_lulcc` 不碰），续跑照常提交。
        match river_tracers.as_ref() {
            Some(tracers) => colm_runtime::river::restart::write_river_tracers(
                &path,
                &network,
                &mut tracers.clone(),
                u8::try_from(integer_field(document, "DEF_REST_CompressLevel")?)
                    .context("DEF_REST_CompressLevel must fit 0..=9")?,
            )?,
            None if logical_field(document, "DEF_USE_TRACER")? => {
                colm_runtime::river::restart::write_empty_river_tracers(&path)?;
            }
            None => {}
        }
        // 泥沙同样留在内存里，LULCC 年末重写的河道续跑带着它（`tracer_lifecycle_route_write_restart`）。
        if let Some(sediment) = sediment.as_ref() {
            sediment.write_restart(
                &path,
                &network,
                u8::try_from(integer_field(document, "DEF_REST_CompressLevel")?)
                    .context("DEF_REST_CompressLevel must fit 0..=9")?,
            )?;
        }
        written.push(path);
        Some((state, river_tracers, sediment))
    } else {
        None
    };
    println!(
        "colm-rs: LULCC {} -> {} ({} -> {} patches); wrote {}",
        years.old,
        years.new,
        old_topology.patch_count(),
        new_topology.patch_count(),
        written
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(Transition {
        baseflow: new_baseflow,
        pairing: previous,
        river,
    })
}

/// 换年的结果：新布局的 `scale_baseflow`、"新 patch → 配上的旧 patch"（全局次序）与换年后的河道状态。
struct Transition {
    baseflow: Vec<f64>,
    pairing: Vec<Option<usize>>,
    river: Option<SegmentRiverEnd>,
}

/// `LulccTransferTraceReadin`：`landdata/lulcc/<year>/lccpct_patches_lcXX_<block>.nc`，
/// `XX = 00..=17`（IGBP），返回每个新 patch 的 `lccpct[ilc]`。
fn read_lulcc_transfer_trace(
    landdata: &Path,
    year: i64,
    block: &str,
    patches: usize,
) -> Result<Vec<Vec<f64>>> {
    const IGBP_CLASSES: usize = 17;
    let mut lccpct = vec![vec![0.0; IGBP_CLASSES + 1]; patches];
    for ilc in 0..=IGBP_CLASSES {
        let path = landdata
            .join("lulcc")
            .join(format!("{year:04}"))
            .join(format!("lccpct_patches_lc{ilc:02}_{block}.nc"));
        let values = netcdf::open(&path)
            .with_context(|| {
                format!(
                    "cannot open {}; DEF_LULCC_SCHEME = 2 needs the transfer trace from mksrfdata",
                    path.display()
                )
            })?
            .variable("lccpct_patches")
            .with_context(|| format!("{} has no lccpct_patches", path.display()))?
            .get_values::<f64, _>(..)?;
        ensure!(
            values.len() == patches,
            "{} has {} patches for a block of {patches}",
            path.display(),
            values.len()
        );
        for (row, value) in lccpct.iter_mut().zip(values) {
            row[ilc] = value;
        }
    }
    Ok(lccpct)
}

/// 与本程序同目录的另一个 Rust 工具（`colm-cli` 找 sidecar 的同一条规则）。
fn sibling_executable(name: &str) -> Result<PathBuf> {
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    let path = std::env::current_exe()
        .context("cannot locate the running colm-rs executable")?
        .parent()
        .context("colm-rs has no parent directory")?
        .join(file);
    ensure!(
        path.is_file(),
        "{} is missing beside colm-rs; DEF_USE_LULCC runs it for each new land-cover year",
        path.display()
    );
    Ok(path)
}

/// 河道续跑文件（不分块）：`<restart>/<date>/<case>_restart_gridriver_<date>_lc<year>.nc`。
fn river_restart_path(restart: &Path, name: &str, label: &str, year: i64) -> PathBuf {
    restart
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
    tracer_cache: Option<&colm_runtime::tracer::ForcingCache<'_>>,
    empty_land_tracer: Option<f64>,
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
        append_tracer_restart(
            &templates[patches.clone()],
            &states[patches.clone()],
            &path,
            tracer_cache
                .map(|cache| cache.block(patches.clone()))
                .as_ref(),
        )?;
        // 开了示踪物却没有输运示踪物（例如只有 CH4 provider）：patch 上不挂示踪物状态，
        // 上游仍提交一份空的陆面示踪物事务。
        if let Some(mixing) = empty_land_tracer {
            colm_init::write_empty_land_tracer_transaction(&path, mixing)?;
        }
        mark_history_restart_with_river(
            &path,
            history,
            Some(patches.clone()),
            river_required,
            sidecar_tracers(&templates[patches.clone()], &states[patches.clone()])?.as_ref(),
        )?;
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
    spatial_patch: Option<SpatialPatch<'_>>,
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
    // 空间算例的 PFT 区间是所在单元的；只有土壤 patch 拥有它，其余 patch 没有 PFT。
    let spatial_patch = spatial_patch.map(|mut patch| {
        if template.patch_type != 0 {
            patch.pfts = 0..0;
        }
        patch
    });
    // `DEF_USE_PFT`：土壤 patch 的 PFT 子网格来自同目录的 `*_restart_pft_*` 两份重启。
    // 只有土壤 patch 有 PFT（`patch_pft_s/e`）；湿地等其余 patch 在 PFT 模式下仍走 patch 级 LAI。
    if template.physics.use_pft && template.patch_type == 0 {
        template = template
            .with_pft(
                &colm_runtime::pft::pft_restart_path(&files.constant)?,
                &colm_runtime::pft::pft_restart_path(&files.time)?,
                document,
                spatial_patch.as_ref().map(|patch| patch.pfts.clone()),
            )
            .context("cannot assemble the PFT subgrid")?;
    }
    // `DEF_USE_BGC`：BGC 状态来自四份 BGC 重启，氮沉降来自 `DEF_dir_runtime/ndep`。
    if let Some(switches) = template.physics.bgc {
        let (mut bgc, irrigation) =
            assemble_bgc(document, files, patch, switches, spatial_patch.as_ref())?;
        // `ch4_reactive_init`：注册了 CH4 示踪物时，BGC 之后跑甲烷（单点内核没有网格河湖汇流）。
        // 空间内核编进了网格河湖（`routing`/`hybrid` 可用），单点内核没有。
        if let Some(setup) =
            colm_runtime::methane::setup_from_document(document, spatial_patch.is_some())?
        {
            let constant = colm_init::RestartFile::open(&files.constant)?;
            let slpratio = *constant
                .floats("slpratio")?
                .get(patch)
                .context("the constant restart has no slpratio for this patch")?;
            let soil = |name: &str| -> Result<Vec<f64>> {
                bgc.statics
                    .soil
                    .iter()
                    .find(|(field, _)| *field == name)
                    .map(|(_, values)| values.clone())
                    .with_context(|| format!("the BGC statics have no {name}"))
            };
            let site = colm_runtime::methane::site(
                template.patch_type,
                bgc.statics.patchclass,
                bgc.statics.patchlatr,
                slpratio,
                &template.root_fraction,
                &bgc.statics.z_soi,
                &bgc.statics.dz_soi,
                &bgc.statics.zi_soi,
                &soil("bsw")?,
                &soil("porsl")?,
                bgc.initial.constants.organic_max,
                template.physics.wetland_water_capacity_mm,
            )?;
            // `ch4_reactive_read_restart`：时间重启里有甲烷事务就续跑，否则冷启动。
            let time = colm_init::RestartFile::open(&files.time)?;
            match colm_runtime::methane::read_restart(&time, patch, template.patch_type, &setup)
                .with_context(|| {
                    format!("cannot read the methane state of {}", files.time.display())
                })? {
                Some(restarted) => {
                    bgc.initial.methane = Some(Box::new(restarted.patch));
                    bgc.initial.methane_acc = restarted.accumulator;
                }
                None => {
                    let mut cold = colm_core::methane::driver::MethanePatch::cold(&setup.params);
                    // `initialize_methane_lake_soilc_from_surface`：冷启动的湖泊 patch 从常数重启的
                    // `lake_soilc_srf`（mkinidata 按有机质密度算的沉积碳）起步，缺了就停。
                    if template.patch_type == 4 && setup.params.methane.allowlakeprod {
                        let nl = colm_core::methane::physics::NL_SOIL;
                        let srf = constant
                            .layer_column("lake_soilc_srf", patch, nl)
                            .context("lake CH4 initialization requires lake_soilc surface data")?;
                        let invalid = |x: &f64| x.is_nan() || x.abs() >= 0.5e36 || *x < 0.0;
                        ensure!(
                            !srf.iter().any(invalid) && srf.iter().sum::<f64>() > 1.0e-12,
                            "lake CH4 production requires positive finite lake_soilc for every \
                             initialized lake patch"
                        );
                        for (target, value) in cold.lake_soilc.iter_mut().zip(&srf) {
                            *target = value.max(0.0);
                        }
                    }
                    bgc.initial.methane = Some(Box::new(cold));
                }
            }
            bgc.methane = Some((setup, site));
        }
        template = template
            .with_bgc(bgc)
            .context("cannot assemble the BGC state")?;
        // `DEF_USE_IRRIGATION`：时间重启里的灌溉量，叠上 `CROP_readin` 读的灌溉方式（与配水比例）。
        // 需水（`bgc_driver`）与施灌（`CalIrrigationApplicationFluxes`）都只在 `patchtype == 0` 上跑；
        // 其余 patch 只挂冻结的状态（history 照样逐 patch 累加）。
        if let Some(readin) = irrigation {
            let mut state = colm_runtime::irrigation::initial_state(
                &colm_init::RestartFile::open(&files.time)?,
                patch,
                readin,
            )?;
            if let Some(pft) = template.snow_state().energy.pft.as_ref() {
                let fractions: Vec<f64> = pft.parameters.iter().map(|p| p.fraction).collect();
                state.dominant_pft = colm_core::dominant_irrigation_pft(&fractions);
            }
            template = if template.patch_type == 0 {
                template.with_irrigation(state)
            } else {
                template.with_frozen_irrigation(state)
            }
            .context("cannot assemble the irrigation state")?;
        }
    }
    if template.patch_type != 0 {
        template.physics.irrigation = None;
    }
    ensure!(
        template.physics.irrigation.is_none() || template.irrigation.is_some(),
        "DEF_USE_IRRIGATION is verified only on CROP BGC soil patches"
    );
    // `DEF_LAI_MONTHLY` 打开时每月重读 LAI（`CoLM.F90:595-605`）。**不装就等于关门**：
    // 跨月的运行会从第二个月起一直用第一天的叶面积，而且不会报错。
    // 空间算例的逐月 LAI 读 `landdata/LAI/<year>/` 的分块向量（`LAI_readin` 的非单点支）。
    if logical_field(document, "DEF_LAI_MONTHLY")? && spatial {
        let year = |key: &str| -> Result<i32> {
            i32::try_from(integer_field(document, key)?)
                .with_context(|| format!("{key} does not fit an i32"))
        };
        // PFT 土壤 patch：`LAI/SAI_patches` 给 patch、`LAI/SAI_pfts` 给 PFT（`MOD_LAIReadin.F90:192-232`）。
        if template.pft.is_some() {
            template = template.with_pft_grid_monthly_leaf_area_index(
                &layout.out().join(name).join("landdata"),
                block,
                logical_field(document, "DEF_LAI_CHANGE_YEARLY")?,
                year("DEF_LC_YEAR")?,
                (year("DEF_LAI_START_YEAR")?, year("DEF_LAI_END_YEAR")?),
            )?;
        }
        // LAI 反馈（PFT/PC 构建）：`LAI_readin` 只读 `SAI_patches`。
        let feedback = (template.physics.use_pft || template.physics.use_pc)
            && template.physics.bgc.is_some_and(|bgc| bgc.laifeedback);
        // 城市 patch 走 `UrbanLAI_readin`（`CoLM.F90:637`）：树冠 LAI/SAI 按城市单元读。
        let urban_unit = template.urban.as_ref().map(|urban| urban.urban_index);
        template = template.with_monthly_leaf_area_index(if let Some(unit) = urban_unit {
            MonthlyLeafAreaIndex::read_urban_grid(
                layout.out().join(name).join("landdata"),
                block,
                unit,
                logical_field(document, "DEF_LAI_CHANGE_YEARLY")?,
                year("DEF_LC_YEAR")?,
                (year("DEF_LAI_START_YEAR")?, year("DEF_LAI_END_YEAR")?),
            )
        } else {
            MonthlyLeafAreaIndex::read_grid(
                layout.out().join(name).join("landdata"),
                block,
                patch,
                logical_field(document, "DEF_LAI_CHANGE_YEARLY")?,
                year("DEF_LC_YEAR")?,
                (year("DEF_LAI_START_YEAR")?, year("DEF_LAI_END_YEAR")?),
                !feedback,
            )
        });
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
    // `DEF_USE_OZONEDATA`：`init_ozone_data(sdate)` 读起始那一档，之后每跨 3 小时档换一次。
    // 取值位置与 BGC 驱动数据相同：单点取站点所在格，空间按 patch 的像元面积加权。
    if template.physics.ozone.is_some_and(|ozone| ozone.use_data) {
        let constant = colm_init::RestartFile::open(&files.constant)?;
        let degrees = |name: &str| -> Result<f64> {
            let radians = *constant
                .floats(name)?
                .get(patch)
                .with_context(|| format!("the constant restart has no {name} for patch {patch}"))?;
            Ok(radians * 180.0 / std::f64::consts::PI)
        };
        let locator = match spatial_patch.as_ref() {
            Some(spatial) => colm_runtime::bgc_step::Locator::Patch {
                pixel: spatial.pixel,
                cells: spatial.cells,
                shared_fraction: spatial.shared_fraction,
            },
            None => colm_runtime::bgc_step::Locator::Site {
                latitude_deg: degrees("patchlatr")?,
                longitude_deg: degrees("patchlonr")?,
            },
        };
        if let Some((source, initial)) =
            colm_runtime::ozone::init_ozone_data(document, &template.physics, locator)?
        {
            template = template.with_ozone_source(source, initial)?;
        }
    }
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
    spatial_patch: Option<&SpatialPatch<'_>>,
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
        spatial_patch.map(|patch| patch.pfts.clone()),
    )?
    .initial;
    let layers = initial.dims.nl_soil;
    // `MOD_Namelist.F90:1901`：单点构建把 `DEF_TOPMOD_method` 强制为 0。
    let switches = colm_core::bgc_driver::BgcSwitches {
        topmod_method: if spatial_patch.is_some() {
            switches.topmod_method
        } else {
            0
        },
        ..switches
    };
    let statics = colm_runtime::bgc_step::BgcStatics::read(&files.constant, patch, layers)?;
    let runtime_dir = std::path::PathBuf::from(string_field(document, "DEF_dir_runtime")?);
    let degrees = |radians: f64| radians * 180.0 / std::f64::consts::PI;
    // 驱动数据的取值位置：单点取站点所在格，空间按 patch 的像元面积加权（`build_arealweighted`）。
    let locator = match spatial_patch {
        Some(patch) => colm_runtime::bgc_step::Locator::Patch {
            pixel: patch.pixel,
            cells: patch.cells,
            shared_fraction: patch.shared_fraction,
        },
        None => colm_runtime::bgc_step::Locator::Site {
            latitude_deg: degrees(statics.patchlatr),
            longitude_deg: degrees(statics.patchlonr),
        },
    };
    // `CROP_readin`（`CoLM.F90:442`）：启动时覆盖作物的播种日与施肥量（与灌溉方式）。
    let mut irrigation = None;
    if switches.crop {
        let pft_file = match spatial_patch {
            Some(spatial) => colm_init::RestartFile::open(&pft_constant)?
                .select_patch(patch, spatial.pfts.clone())?,
            None => colm_runtime::pft::open_patch(&pft_constant, patch, patches, pfts)?,
        };
        let classes = pft_file
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
                patch: locator,
                // 空间：每个 PFT 按自己的像元（`landpft` 的 `ipxstt/ipxend/pctshared`）映射；单点同站点。
                pfts: match spatial_patch {
                    Some(spatial) => spatial
                        .pft_cells
                        .iter()
                        .zip(spatial.pft_shared)
                        .map(|(cells, &shared)| colm_runtime::bgc_step::Locator::Patch {
                            pixel: spatial.pixel,
                            cells,
                            shared_fraction: shared,
                        })
                        .collect(),
                    None => vec![locator; classes.len()],
                },
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
        locator,
        logical_field(document, "DEF_USE_PN")?,
        monthly_ndep,
    )?;
    // `init_ndep_data_*(s_year, …)`、`init_fire_data(s_year)`：namelist 的起始年。上游原来用
    // `sdate(1)`（经过 `adj2end`，00:00 的 1 月 1 日起步算上一年），只有开示踪物时读 `s_year`；
    // vendor 已解耦成一律读 `s_year`。
    let ndep_start_year =
        i32::try_from(integer_field(document, "DEF_simulation_time%start_year")?)?;
    let month = integer_field(document, "DEF_simulation_time%start_month")?;
    let deltim = real_field(document, "DEF_simulation_time%timestep")?;
    // `init_nitrif_data(ststamp)`：起始时刻（未经 adj2end）所在的月。
    let nitrif = if switches.nitrif {
        Some((
            colm_runtime::bgc_step::NitrifSource::open(&runtime_dir, locator, layers)?,
            u8::try_from(month).context("DEF_simulation_time%start_month is not a month")?,
        ))
    } else {
        None
    };
    // `init_fire_data`：`DEF_dir_runtime/fire/` 的静态场与逐年人口密度、3 小时闪电。
    let fire = if switches.fire {
        Some(colm_runtime::bgc_step::FireSource::open(
            &runtime_dir,
            locator,
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
                    append_tracer_restart(templates, states, &path, None)?;
                    mark_history_restart(
                        &path,
                        history_restart,
                        sidecar_tracers(templates, states)?.as_ref(),
                    )?;
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
    append_tracer_restart(templates, &states, restart_out, None)?;
    mark_history_restart(
        restart_out,
        history_restart,
        sidecar_tracers(templates, &states)?.as_ref(),
    )?;
    Ok(RunSummary {
        steps,
        history_files,
    })
}

/// `write_tracer_restart_all`：有输运示踪物时把示踪物事务追加进刚写好的陆面续跑文件。
/// `land_tracer_init`：给一份续跑文件里的各 patch 挂上示踪物状态。续跑里的示踪物事务可用就读
/// （`template.patch` 是该文件里的行，`count` 是行数），否则按水量冷启动。
fn attach_land_tracers(
    tracer: &std::sync::Arc<colm_runtime::tracer::TracerRuntime>,
    templates: Vec<StandardLctRestartTemplate>,
    restart: &colm_init::RestartFile,
    count: usize,
) -> Result<Vec<StandardLctRestartTemplate>> {
    templates
        .into_iter()
        .map(|template| {
            let state = template.snow_state();
            let wliq = colm_core::tracer::step::pack_soisno(
                &state.snow.liquid_water_kg_m2,
                &state.soil_water.liquid_water_kg_m2,
            );
            let wice = colm_core::tracer::step::pack_soisno(
                &state.snow.ice_water_kg_m2,
                &state.soil_water.ice_water_kg_m2,
            );
            let initial = tracer.initial_state(
                restart,
                template.patch,
                count,
                colm_core::tracer::WaterInventory {
                    patch_type: template.patch_type,
                    ldew_rain: state.energy.leaf.canopy_water.rain_mm,
                    ldew_snow: state.energy.leaf.canopy_water.snow_mm,
                    wliq_soisno: &wliq,
                    wice_soisno: &wice,
                    wa: state.soil_water.aquifer_water_mm,
                    wdsrf: state.soil_water.surface_water_mm,
                    wetwat: state.soil_water.wetland_water_mm,
                    scv: state.snow.water_equivalent_kg_m2,
                    waterstorage: None,
                },
            )?;
            template.with_tracer(std::sync::Arc::clone(tracer), initial)
        })
        .collect()
}

fn append_tracer_restart(
    templates: &[StandardLctRestartTemplate],
    states: &[StandardLctSnowSoilState],
    path: &Path,
    cache: Option<&colm_runtime::tracer::ForcingCache<'_>>,
) -> Result<()> {
    if let Some((tracer, _)) = templates
        .first()
        .and_then(|template| template.tracer.as_ref())
    {
        let tracks = states
            .iter()
            .map(|state| {
                state
                    .tracer
                    .as_deref()
                    .map(|track| &track.state)
                    .context("every patch of a tracer run carries tracer state")
            })
            .collect::<Result<Vec<_>>>()?;
        colm_runtime::tracer::write_land_tracer_restart(
            path,
            &tracer.set,
            &tracks,
            tracer.aquifer_mixing_water_mm,
            cache,
        )?;
    }
    // `tracer_lifecycle_land_write_restart`：CH4 provider 的状态接在示踪物事务之后。
    let methane = templates
        .first()
        .and_then(|template| template.bgc.as_ref())
        .and_then(|bgc| bgc.methane.as_ref());
    if let Some((setup, _)) = methane {
        let patches = states
            .iter()
            .map(|state| {
                let bgc = state
                    .bgc
                    .as_deref()
                    .context("a methane patch needs its BGC state")?;
                let patch = bgc
                    .methane
                    .as_deref()
                    .context("a methane patch needs its methane state")?;
                Ok((patch, &bgc.methane_acc))
            })
            .collect::<Result<Vec<_>>>()?;
        colm_runtime::methane::write_restart(path, setup, &patches)?;
    }
    Ok(())
}

/// 写续跑文件时要附带的历史累加器信息（`land_history_restart.inc`）。
struct HistoryRestart {
    config: colm_runtime::history_sidecar::SidecarConfig,
    /// 当前历史区间的原始累加状态；没开 history 时为 `None`（恒为空窗口）。
    window:
        Option<std::sync::Arc<std::sync::Mutex<Vec<colm_runtime::history_sidecar::HistoryWindow>>>>,
    /// 运行终点（非自然边界）清零前的示踪物/CH4 累加器；有值时旁车用它而不是状态里的。
    tracer_raw: Option<colm_runtime::tracer_sidecar::RawTracersHandle>,
    /// 每个 patch 是不是城市（城市累加器排在旁车的 `urban` 维上），全部分块按装配顺序。
    urban: Vec<bool>,
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
fn mark_history_restart(
    restart: &Path,
    history: &HistoryRestart,
    tracers: Option<&colm_runtime::tracer_sidecar::SidecarTracers<'_>>,
) -> Result<()> {
    mark_history_restart_with_river(restart, history, None, false, tracers)
}

/// `read_history_acc_restart` 的示踪物部分：区间跨过重启时，用旁车里的累加器覆盖起跑状态
/// （示踪物的 `a_trc_*`/`a_water_*` 只在旁车里；CH4 的累加量覆盖续跑文件读回的那份）。
/// `file_patches` 是旁车的 patch 数，`index` 给出模板在旁车里的位置。
fn restore_sidecar_tracers(
    templates: &mut [StandardLctRestartTemplate],
    sidecar: &Path,
    file_patches: usize,
    index: impl Fn(usize, &StandardLctRestartTemplate) -> usize,
) -> Result<()> {
    let Some(first) = templates.first() else {
        return Ok(());
    };
    let set = first.tracer.as_ref().map(|(tracer, _)| tracer.set.clone());
    let setup = first
        .bgc
        .as_ref()
        .and_then(|bgc| bgc.methane.as_ref())
        .map(|(setup, _)| setup.clone());
    if set.is_none() && setup.is_none() {
        return Ok(());
    }
    let restored =
        colm_runtime::tracer_sidecar::read(sidecar, set.as_ref(), setup.as_ref(), file_patches)?;
    for (position, template) in templates.iter_mut().enumerate() {
        let at = index(position, template);
        if let (Some(accumulators), Some((_, state))) =
            (restored.tracers.as_ref(), template.tracer.as_mut())
        {
            let patch = &accumulators[at];
            state.acc = patch.tracers.clone();
            state.water_acc = patch.water.clone();
        }
        if let (Some(accumulators), Some(bgc)) = (restored.methane.as_ref(), template.bgc.as_mut())
        {
            bgc.initial.methane_acc = accumulators[at];
        }
    }
    Ok(())
}

/// 旁车的示踪物部分（`tracer_history_write` + 生命周期钩子）要的累加器：有输运示踪物或 CH4
/// provider 时才有，取自各 patch 状态。
fn sidecar_tracers<'a>(
    templates: &'a [StandardLctRestartTemplate],
    states: &[StandardLctSnowSoilState],
) -> Result<Option<colm_runtime::tracer_sidecar::SidecarTracers<'a>>> {
    use colm_runtime::tracer_sidecar::PatchAccumulators;
    let first = templates.first();
    let set = first
        .and_then(|template| template.tracer.as_ref())
        .map(|(tracer, _)| &tracer.set);
    let setup = first
        .and_then(|template| template.bgc.as_ref())
        .and_then(|bgc| bgc.methane.as_ref())
        .map(|(setup, _)| setup);
    if set.is_none() && setup.is_none() {
        return Ok(None);
    }
    let patches = match set {
        Some(_) => states
            .iter()
            .map(|state| {
                state
                    .tracer
                    .as_deref()
                    .map(|track| PatchAccumulators::of(&track.state))
                    .context("every patch of a tracer run carries tracer state")
            })
            .collect::<Result<Vec<_>>>()?,
        None => Vec::new(),
    };
    let methane = match setup {
        Some(setup) => Some((
            setup,
            states
                .iter()
                .map(|state| {
                    state
                        .bgc
                        .as_deref()
                        .map(|bgc| bgc.methane_acc)
                        .context("a methane patch needs its BGC state")
                })
                .collect::<Result<Vec<_>>>()?,
        )),
        None => None,
    };
    Ok(Some(colm_runtime::tracer_sidecar::SidecarTracers {
        set,
        patches,
        methane,
    }))
}

/// [`mark_history_restart`]；空间构建另带 `history_river_required`。
/// `block` 是这个续跑文件在全部 patch 里的区间（多分块空间算例）；`None` 为整份窗口。
fn mark_history_restart_with_river(
    restart: &Path,
    history: &HistoryRestart,
    block: Option<std::ops::Range<usize>>,
    river_required: bool,
    tracers: Option<&colm_runtime::tracer_sidecar::SidecarTracers<'_>>,
) -> Result<()> {
    let mut windows = history
        .window
        .as_ref()
        .map(|window| window.lock().expect("history window lock").clone())
        .unwrap_or_else(|| vec![colm_runtime::history_sidecar::HistoryWindow::default()]);
    if let Some(block) = block.clone() {
        if windows.len() > 1 {
            windows = windows[block].to_vec();
        }
    }
    let patches = colm_init::RestartFile::open(restart)?.dimension("patch")?;
    let sidecar = history_sidecar_path(restart)?;
    let urban = match &block {
        Some(block) if history.urban.len() > 1 => &history.urban[block.clone()],
        _ => &history.urban[..],
    };
    colm_runtime::history_sidecar::write_sidecar_with_river(
        &sidecar,
        patches,
        &history.config,
        &windows,
        river_required,
        urban,
    )?;
    // 区间跨过重启（`window_active`）时旁车再带示踪物部分；运行终点（非自然边界）用清零前的快照。
    if let Some(tracers) = tracers.filter(|_| windows.first().is_some_and(|w| w.steps > 0)) {
        let raw = history
            .tracer_raw
            .as_ref()
            .and_then(|raw| raw.lock().expect("tracer raw lock").clone());
        match raw {
            Some(raw) => {
                let raw = match &block {
                    Some(block) if raw.len() > 1 => raw[block.clone()].to_vec(),
                    _ => raw,
                };
                let patches = match tracers.set {
                    Some(_) => raw
                        .iter()
                        .map(|patch| {
                            patch
                                .tracer
                                .clone()
                                .context("the raw tracer window lost a patch")
                        })
                        .collect::<Result<_>>()?,
                    None => Vec::new(),
                };
                let methane = match &tracers.methane {
                    Some((setup, _)) => Some((
                        *setup,
                        raw.iter()
                            .map(|patch| {
                                patch.methane.context("the raw methane window lost a patch")
                            })
                            .collect::<Result<_>>()?,
                    )),
                    None => None,
                };
                let raw_tracers = colm_runtime::tracer_sidecar::SidecarTracers {
                    set: tracers.set,
                    patches,
                    methane,
                };
                colm_runtime::tracer_sidecar::write(&sidecar, &raw_tracers)?;
            }
            None => colm_runtime::tracer_sidecar::write(&sidecar, tracers)?,
        }
    }
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
    /// 被强迫缺测遮蔽、整步跳过的 patch：续跑里只替换 `tlai`/`tsai`，上面三项不用。
    masked: bool,
}

impl RestartSnapshot {
    /// 被强迫缺测遮蔽的 patch：没有步输出，写回时保留起跑重启里的值。
    fn masked() -> Self {
        Self {
            matric_potential_mm: Vec::new(),
            hydraulic_conductivity_mm_s: Vec::new(),
            diagnostics: SurfaceDiagnosticsRow {
                cosine_zenith: f64::NAN,
                wet_snow_fraction: f64::NAN,
                tref: f64::NAN,
                qref: f64::NAN,
                stomatal_resistance: None,
                soil_surface_resistance: None,
                trad: f64::NAN,
                emis: f64::NAN,
                z0m: f64::NAN,
                zol: f64::NAN,
                rib: f64::NAN,
                ustar: f64::NAN,
                qstar: f64::NAN,
                tstar: f64::NAN,
                fm: f64::NAN,
                fh: f64::NAN,
                fq: f64::NAN,
                gs0sun: None,
                gs0sha: None,
            },
            lai_refreshed: false,
            masked: true,
        }
    }

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
                masked: false,
            },
            // 冰川分支不调 `soilwater`：`smp`/`hk` 保持重启里的值。
            PatchStepOutput::Glacier(output) => Self {
                matric_potential_mm: state.soil_water.matric_potential_mm.clone(),
                hydraulic_conductivity_mm_s: state.soil_water.hydraulic_conductivity_mm_s.clone(),
                diagnostics: SurfaceDiagnosticsRow::from_glacier(&output.thermal, cosine_zenith),
                lai_refreshed: false,
                masked: false,
            },
            // 湖同样不调 `soilwater`。
            // 城市：透水地面的 `WATER_2014` 已把 `smp`/`hk` 写进状态。
            PatchStepOutput::Urban(output) => Self {
                matric_potential_mm: state.soil_water.matric_potential_mm.clone(),
                hydraulic_conductivity_mm_s: state.soil_water.hydraulic_conductivity_mm_s.clone(),
                diagnostics: SurfaceDiagnosticsRow::from_urban(output, cosine_zenith),
                lai_refreshed: false,
                masked: false,
            },
            PatchStepOutput::Lake(output) => Self {
                matric_potential_mm: state.soil_water.matric_potential_mm.clone(),
                hydraulic_conductivity_mm_s: state.soil_water.hydraulic_conductivity_mm_s.clone(),
                diagnostics: SurfaceDiagnosticsRow::from_lake(&output.thermal, cosine_zenith),
                lai_refreshed: false,
                masked: false,
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
    // 空间算例的第一个 patch 不一定是土壤 patch：只要有一个 patch 带 PFT 就有 PFT 重启。
    let pft_source = templates
        .iter()
        .any(|template| template.pft.is_some())
        .then(|| colm_init::RestartFile::open(&pft_in))
        .transpose()?;
    let slots = templates
        .iter()
        .map(|template| {
            // 每个 patch 在 PFT 重启里的区间就是装配时用的那一段（单点按 patch 推，空间来自 `landpft`）。
            let pfts = match (&pft_source, template.pft.as_ref()) {
                (Some(_), Some(pft)) => pft.pft_range(),
                _ => 0..0,
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
        if snapshot.masked {
            lists.push(template.masked_overrides(state)?);
            continue;
        }
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
        for ((template, state), snapshot) in templates.iter().zip(states).zip(snapshots) {
            // 空间算例的非土壤 patch 没有 PFT（区间为空），不改 PFT 重启。
            if template.pft.is_none() {
                lists.push(Vec::new());
                continue;
            }
            let (Some(pft_template), Some(pft)) = (&template.pft, &state.energy.pft) else {
                anyhow::bail!("patch {} has no PFT subgrid to write back", template.patch);
            };
            let mut overrides = pft_template.overrides(pft);
            // 被遮蔽的 patch：只有起跑时对整列赋值的量 —— `LAI_readin` 的 `tlai_p`/`tsai_p`，
            // 以及 CROP 下 `CROP_readin`（`CoLM.F90:430`）的施肥、播种日与灌溉方式。
            if snapshot.masked {
                overrides.retain(|o| matches!(o.name.as_str(), "tlai_p" | "tsai_p"));
                if let Some(bgc) = &state.bgc {
                    overrides.extend(
                        colm_runtime::bgc::BgcTemplate::overrides(bgc, pft_source)
                            .into_iter()
                            .filter(|o| {
                                matches!(
                                    o.name.as_str(),
                                    "manunitro_p"
                                        | "fertnitro_p"
                                        | "plantdate_p"
                                        | "irrig_method_p"
                                )
                            }),
                    );
                }
                lists.push(overrides);
                continue;
            }
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
    // 空间算例一块里有多个城市单元：每个城市 patch 只改自己那一格（`urban_index`），
    // 以第一个的整变量为底，依次把其余单元的那一格拷进去。
    let urban_patches = templates
        .iter()
        .zip(states)
        .zip(snapshots)
        .filter_map(|((template, state), snapshot)| {
            template
                .urban
                .as_ref()
                .zip(state.urban.as_ref())
                .map(|(t, s)| (template, state, t, s, snapshot.masked))
        })
        .collect::<Vec<_>>();
    if !urban_patches.is_empty() {
        let urban_path = |path: &Path| -> Result<std::path::PathBuf> {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .context("a restart path has no file name")?;
            Ok(path.with_file_name(name.replacen("_restart_", "_restart_urban_", 1)))
        };
        let urban_count =
            colm_init::RestartFile::open(&urban_path(restart_in)?)?.dimension("urban")?;
        let mut merged: Option<Vec<colm_init::RestartOverride>> = None;
        for (template, state, urban_template, urban, masked) in urban_patches {
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
            // 被遮蔽的单元整步跳过：本单元那一格保留起跑值（`tree_lai/tree_sai` 除外）。
            let overrides = if masked {
                urban_template.masked_overrides(urban, tree)?
            } else {
                urban_template.overrides(urban, tree)?
            };
            match merged.as_mut() {
                None => merged = Some(overrides),
                Some(merged) => {
                    let unit = urban_template.urban_index;
                    for (into, from) in merged.iter_mut().zip(overrides) {
                        ensure!(into.name == from.name, "urban overrides are not aligned");
                        let width = from.values.len() / urban_count;
                        into.values[unit * width..(unit + 1) * width]
                            .copy_from_slice(&from.values[unit * width..(unit + 1) * width]);
                    }
                }
            }
        }
        write_restart(
            &urban_path(restart_in)?,
            &urban_path(restart_out)?,
            merged.expect("at least one urban patch").as_slice(),
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

/// 闸门 3（`DEF_hist_vars`，`MOD_Namelist.F90:2788-2870`）：开关先取声明默认值，`sync_hist_vars` 把同步到
/// 的置成 `DEF_HIST_vars_out_default`，`DEF_HIST_vars_namelist` 文件存在时读它的 `&nl_colm_history` 覆盖，
/// DiagMatrix 再强制打开一批容量量。Fortran 进程在算例目录里跑，相对路径按算例目录解析。
///
/// FIRE 的五个历史量写的是上一次 `vecacc` 的残留（upstream-bugs 第 26 条），残留取决于前面实际
/// 写了哪个量；开关一旦改变写出集合，这条链就变了 —— 这种组合拒绝。
fn install_history_selection(document: &Document, case_directory: &Path, crop: bool) -> Result<()> {
    let out_default = logical_field(document, "DEF_HIST_vars_out_default")?;
    let file = string_field(document, "DEF_HIST_vars_namelist")?;
    let file = case_directory.join(file.trim());
    let overrides = if file.is_file() {
        history_namelist_overrides(&file)?
    } else {
        Vec::new()
    };
    let bgc = logical_field(document, "DEF_USE_BGC")?;
    // `MOD_Namelist.F90`：FERT/灌溉只在 CROP 内核里生效，其余开关读 namelist 本身。
    let runtime = |condition: &str| -> Result<bool> {
        Ok(match condition {
            "DEF_USE_FERT" | "DEF_USE_IRRIGATION" => crop && logical_field(document, condition)?,
            other => logical_field(document, other)?,
        })
    };
    let diag_matrix = bgc && logical_field(document, "DEF_USE_DiagMatrix")?;
    let resolve = |out_default: bool, overrides: &[(String, bool)]| {
        colm_hist::selection::HistorySelection::resolve(&colm_hist::selection::SelectionInput {
            out_default,
            overrides,
            runtime: &runtime,
            // Rust 运行期不支持 HYPERSPECTRAL/DataAssimilation 内核。
            defined: &|_| false,
            diag_matrix,
        })
    };
    let selection = resolve(out_default, &overrides)?;
    colm_runtime::history::install_selection(selection)
}

/// `&nl_colm_history` 里的 `DEF_hist_vars%X = .true./.false.`（只读第一个这样的组，与 `read(nml=)` 一致）。
fn history_namelist_overrides(path: &Path) -> Result<Vec<(String, bool)>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read history namelist {}", path.display()))?;
    let document = parse(&text)
        .with_context(|| format!("cannot parse history namelist {}", path.display()))?;
    let mut inside = false;
    let mut seen = false;
    let mut overrides = Vec::new();
    for item in &document.items {
        match item {
            colm_namelist::document::Item::GroupStart(line) => {
                let name = line
                    .trim()
                    .trim_start_matches('&')
                    .trim()
                    .to_ascii_lowercase();
                inside = !seen && name.starts_with("nl_colm_history");
                seen |= inside;
            }
            colm_namelist::document::Item::GroupEnd(_) => inside = false,
            colm_namelist::document::Item::Entry(entry) if inside => {
                let field = entry.path.to_string();
                let member = field
                    .split_once('%')
                    .filter(|(name, _)| name.eq_ignore_ascii_case("DEF_hist_vars"))
                    .map(|(_, member)| member.to_owned())
                    .with_context(|| {
                        format!(
                            "{} sets {field}, which is not a DEF_hist_vars member",
                            path.display()
                        )
                    })?;
                let Value::Bool(value) = entry.value else {
                    bail!("DEF_hist_vars%{member} must be a logical");
                };
                overrides.push((member, value));
            }
            _ => {}
        }
    }
    ensure!(seen, "{} has no &nl_colm_history group", path.display());
    Ok(overrides)
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
        // Fortran 字面量可能带种类后缀（`0.0_r8`）与 `d` 指数。
        Some(colm_schema::Default::Real(text)) => text
            .split('_')
            .next()
            .unwrap_or(text)
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
    /// 内核带 `UNSTRUCTURED` 宏：`DEF_HISTORY_IN_VECTOR` 只在它下生效（`MOD_Hist.F90:71-75`）。
    unstructured: bool,
    /// 内核带 `CATCHMENT` 宏（`CatchLateralFlow`，没有 `GridRiverLakeFlow`）。
    catchment: bool,
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
        let mut unstructured = false;
        let mut catchment = false;
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
                "--unstructured" => unstructured = true,
                "--catchment" => catchment = true,
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
            unstructured,
            catchment,
        })
    }
}

#[cfg(test)]
#[path = "../colm_rs_tests.rs"]
mod colm_rs_tests;

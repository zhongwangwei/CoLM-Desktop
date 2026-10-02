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
        tracer_raw: session.as_ref().map(HistorySession::tracer_raw_handle),
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
    // GRID/UNSTRUCTURED 内核总是编进 `GridRiverLakeFlow`，它改变了几处收缩形状。
    physics.river_lake_flow_build = true;
    let missing = colm_runtime::physics::unported_branches(&physics);
    ensure!(
        missing.is_empty() || arguments.allow_unported_branches,
        "this case needs {} branch(es) the Rust runtime does not implement:\n  - {}",
        missing.len(),
        missing.join("\n  - ")
    );
    // GRID 内核总是编进 `GridRiverLakeFlow`：汇流默认路径（单向耦合，`FloodplainStorageFix` 两种曲线都行），其余选项还没移植。
    // 漫滩回馈：上游自己要求修正漫滩曲线、不与 LULCC 同开，并把产流方案强制成 0；
    // Rust 只接变饱和流、不拆雪土、LCT 的那条路径。
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
        ensure!(
            !logical_field(&document, "DEF_SPLIT_SOILSNOW")?,
            "Grid flood feedback with DEF_SPLIT_SOILSNOW is not ported"
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
    // CH4 provider：与单点同一条 `soil_step`，只移植了 `wetwat` 淹没方案（`routing`/`hybrid` 要接
    // 网格河湖的淹没比例，`satellite` 要读 GIEMS，`dynamic_wtd` 在内核里尚未移植）。
    // `only_wetland` 与稻田会改 history 的活跃掩膜（`methane_patch_active_mask`），网格写出尚未接。
    if let Some(setup) = colm_runtime::methane::setup_from_document(&document, false)? {
        ensure!(
            setup.scheme == 1,
            "spatial methane is ported for DEF_METHANE%inundation_mode = 'wetwat' only; run this \
             case with --engine fortran"
        );
        let m = &setup.params.methane;
        ensure!(
            !m.only_wetland && !m.enable_rice_paddy,
            "spatial methane with only_wetland or enable_rice_paddy (history active mask) is not \
             ported; run this case with --engine fortran"
        );
    }
    // 上游 LULCC 年末重写的河道续跑里带着泥沙（`WRITE_GridRiverLakeTimeVars` →
    // `tracer_lifecycle_route_write_restart`），Rust 的 LULCC 重写只写河道状态，泥沙会悄悄冷启动。
    let sediment = tracer_set.as_ref().is_some_and(|set| {
        set.tracers
            .iter()
            .any(colm_runtime::river::sediment::is_sediment_tracer)
    });
    ensure!(
        !(sediment && logical_field(&document, "DEF_USE_LULCC")?),
        "SEDIMENT with DEF_USE_LULCC (sediment state across the LULCC river restart) is not ported; \
         run this case with --engine fortran"
    );
    let transport_tracers = tracer_set
        .as_ref()
        .is_some_and(|set| set.transport_indices().next().is_some());
    if transport_tracers && logical_field(&document, "DEF_USE_LULCC")? {
        // SAT（`remap_land_tracer_lulcc_state` 不带转移份额）：同单元同类型的 patch 抄旧示踪物，
        // 其余取分配值。MEC 要按 `lulcc_inventory_trace` 做面积守恒的加权，示踪物强迫要
        // `tracer_forcing_lulcc_remap`，都还没接。
        ensure!(
            integer_field(&document, "DEF_LULCC_SCHEME")? == 1,
            "land tracers with DEF_LULCC_SCHEME = 2 (area-conserving MEC tracer remap) are not \
             ported; run this case with --engine fortran"
        );
        let runtime = colm_runtime::tracer::TracerRuntime::from_document(&document)?
            .context("transport tracers need a tracer runtime")?;
        ensure!(
            runtime.forcing_specs.iter().all(Vec::is_empty),
            "tracer runtime forcing with DEF_USE_LULCC (tracer_forcing_lulcc_remap) is not \
             ported; run this case with --engine fortran"
        );
    }
    let methane_tracer = tracer_set.as_ref().is_some_and(|set| {
        set.tracers
            .iter()
            .any(colm_runtime::methane::is_methane_tracer)
    });
    ensure!(
        !(methane_tracer && logical_field(&document, "DEF_USE_LULCC")?),
        "methane with DEF_USE_LULCC (save/remap_methane_lulcc_state) is not ported; run this \
         case with --engine fortran"
    );
    ensure!(
        !logical_field(&document, "DEF_UnitCatchment_regional")?,
        "DEF_UnitCatchment_regional is not ported to the Rust river model; run this case with \
         --engine fortran"
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
    let mut year = if lulcc {
        i64::from(config.start.year)
    } else {
        integer_field(&document, "DEF_LC_YEAR")?
    };
    let vector_history =
        arguments.unstructured && logical_field(&document, "DEF_HISTORY_IN_VECTOR")?;
    // 向量 history 下的示踪物 history（`MOD_Tracer_Hist` 的 `Vector` 分支）还没接。
    ensure!(
        !(vector_history && logical_field(&document, "DEF_USE_TRACER")?),
        "tracers with DEF_HISTORY_IN_VECTOR (vector tracer history) are not ported; run this case \
         with --engine fortran"
    );
    let case = SpatialCase {
        layout,
        name,
        document: &document,
        physics: &physics,
        out: &out,
        vector_history,
    };
    let restart_root = out.join("restart");
    let scratch = restart_root.join(LULCC_SCRATCH);
    let run_end = normalized_day_end(config.end);
    let mut segment = SpatialSegment {
        config: config.clone(),
        year,
        input: restart_root.clone(),
        lulcc_boundary: false,
    };
    loop {
        // LULCC 在一年最后一步之后做（`isendofyear`），运行在那里切段。
        let boundary = lulcc
            .then(|| year_end(segment.config.start))
            .filter(|boundary| {
                calendar_key(normalized_day_end(*boundary)) <= calendar_key(run_end)
            });
        if let Some(boundary) = boundary {
            segment.config.end = boundary;
            segment.lulcc_boundary = true;
        }
        let river = run_spatial_segment(&case, &segment)?;
        let Some(boundary) = boundary else {
            break;
        };
        let next_start = normalized_day_end(boundary);
        // 合并出来的续跑落在哪：这一步本该写续跑（或运行就停在这里）时写进 `restart/`，
        // 否则放进临时目录，只供下一段起跑。
        let target = if config.restart_frequency != colm_core::RestartFrequency::Never
            || next_start == run_end
        {
            restart_root.clone()
        } else {
            scratch.join("new")
        };
        lulcc_transition(
            &case,
            LulccYears {
                old: year,
                new: year + 1,
                history_frequency: config.history_frequency,
            },
            boundary,
            &scratch.join("old"),
            &target,
            river,
        )?;
        year += 1;
        if next_start == run_end {
            break;
        }
        segment = SpatialSegment {
            config: colm_runtime::spatial::runtime::SpatialRuntimeConfig {
                start: next_start,
                spinup_until: next_start,
                ..config.clone()
            },
            year,
            input: target,
            lulcc_boundary: false,
        };
    }
    if scratch.exists() {
        std::fs::remove_dir_all(&scratch)
            .with_context(|| format!("cannot remove {}", scratch.display()))?;
    }
    println!("{SUCCESS_MARKER}");
    Ok(())
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
    /// 这一段停在 LULCC 年末：终点的旧年份状态写进临时目录，交给 [`lulcc_transition`]。
    lulcc_boundary: bool,
}

/// LULCC 临时文件（`restart/` 下）：旧年份的终态、不该留在 `restart/` 里的合并续跑、冷启动 namelist。
const LULCC_SCRATCH: &str = "lulcc-scratch";

/// 跑一段，返回终点的河道状态（LULCC 过渡要接着用）。
fn run_spatial_segment(
    case: &SpatialCase<'_>,
    segment: &SpatialSegment,
) -> Result<
    Option<(
        colm_runtime::river::RiverState,
        Option<colm_runtime::river::tracer::RiverTracers>,
    )>,
> {
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
            crop_classes.as_deref(),
            irrigated.as_deref(),
        )?)
    } else {
        None
    };
    let mut mapping = AreaWeightedMapping::build(
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
    let mut forcing = GriddedForcing::new(
        config.forcing.clone(),
        grid,
        cells,
        config.timestep_seconds as i32,
    )?;
    // `DEF_forcing%has_missing_value`：映射去掉起始那条记录里缺测的格子（`set_missing_value`），
    // 足迹全是缺测格的 patch 的 `forcmask_pch` 为假 —— 上游整步跳过它们、并从累加与 history 里
    // 排除，这一支还没移植，遇到就拒绝。
    if let Some((missing, field, nlon)) = forcing.missing_field(config.start)? {
        let mask = mapping.set_missing_value(|ilon, ilat| field[ilat * nlon + ilon], missing);
        let masked = mask.iter().filter(|&&keep| !keep).count();
        ensure!(
            masked == 0,
            "{masked} patch(es) lie entirely on missing forcing cells (forcmask_pch = .false.); \
             skipping masked patches is not ported to the Rust spatial runtime yet, run this case \
             with --engine fortran or enlarge the forcing coverage"
        );
    }
    let mut runtime = SpatialRuntime::new(
        config.clock()?,
        forcing,
        mapping,
        coordinates,
        config.co2_scenario,
    )?;
    runtime.apply_mapped_heights(&mut templates)?;
    if let (Some(tracer), Some(forcing_config)) = (tracer_runtime.as_ref(), tracer_forcing_config) {
        if forcing_config.enabled() {
            ensure!(
                !logical_field(document, "DEF_USE_LULCC")?,
                "tracer runtime forcing with LULCC (tracer_forcing_lulcc_remap) is not ported"
            );
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
    if segment.lulcc_boundary {
        runtime = runtime.defer_lai_refresh_at(config.end);
    }
    let network = colm_runtime::river::network::RiverNetwork::read(
        Path::new(&string_field(document, "DEF_UnitCatchment_file")?),
        logical_field(document, "DEF_GridRiverLake_FloodplainStorageFix")?,
    )?;
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
        Some(colm_runtime::river::reservoir::Reservoir::read(
            Path::new(&string_field(document, "DEF_ReservoirPara_file")?),
            &network,
            integer_field(document, "DEF_Reservoir_Method")?,
        )?)
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
            Path::new(&string_field(document, "DEF_UnitCatchment_file")?),
            &network,
            &reservoir_cells,
        )?)
    } else {
        None
    };
    let bifurcation = if logical_field(document, "DEF_USE_BIFURCATION")? {
        Some(colm_runtime::river::bifurcation::Bifurcation::read(
            Path::new(&string_field(document, "DEF_UnitCatchment_file")?),
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
    let river_state =
        colm_runtime::river::restart::read_river_state(&river_start, &network, reservoir.as_ref())?;
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
    river.momentum_dt_limit = logical_field(document, "DEF_GRIDBASED_ROUTING_MOMENTUM_DT_LIMIT")?;
    // `river_lake_tracer_init` + `read_tracer_restart`/`tracer_init_from_water`
    // （`grid_riverlake_flow_init`）：有输运示踪物时河道示踪物与陆面一起开。
    if let Some(tracer) = tracer_runtime
        .as_ref()
        .filter(|tracer| tracer.has_transport())
    {
        let mut tracers =
            colm_runtime::river::tracer::RiverTracers::new(tracer.set.clone(), river.network.len());
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
            let param =
                colm_core::tracer::descriptor::param_file_for_index(&files, &set.tracers, index)?
                    .context(
                    "Cannot find sediment parameter file for SEDIMENT in DEF_TRACER_PARAM_FILES",
                )?;
            let sediment = colm_runtime::river::sediment::Sediment::init(
                &river.network,
                Path::new(&string_field(document, "DEF_UnitCatchment_file")?),
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
            .with_gridded(&colm_runtime::river::history::GRIDDED_RIVER_VARIABLES)
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
                )?,
                river: river_writer,
                elements: ElementGroups::from_topology(&topology)?,
                files: Vec::new(),
            }),
            None => None,
        },
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
            urban_run: logical_field(document, "DEF_URBAN_RUN")?,
            urban_patches: templates
                .iter()
                .filter(|template| template.urban.is_some())
                .count(),
            pft_or_pc: logical_field(document, "DEF_USE_PFT")?
                || logical_field(document, "DEF_USE_PC")?,
            bgc: false,
            crop: false,
            river_lake_flow: true,
        },
        window: history
            .as_ref()
            .map(|history| history.session.window_handle()),
        tracer_raw: history
            .as_ref()
            .map(|history| history.session.tracer_raw_handle()),
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
        |steps, states, outputs, river, tracer_cache| {
            let mut snapshots = states
                .iter()
                .zip(outputs)
                .zip(steps)
                .map(|((state, output), step)| {
                    let mut snapshot =
                        RestartSnapshot::new(state, *output, step.surface_cosine_zenith)?;
                    // LULCC 年末那一步没重读 LAI（`defer_lai_refresh_at`）。
                    snapshot.lai_refreshed = step.clock.update_lai
                        && !(segment.lulcc_boundary && step.clock.end_time == config.end);
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
            }
            last = Some(snapshots);
            Ok(())
        },
    )?;
    let last = last.context(NO_STEP)?;
    // 停在 LULCC 年末的一段：旧年份的终态只是合并的输入，写进临时目录。
    let finals = if segment.lulcc_boundary {
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
    if let (Some(river), false) = (runtime.river(), segment.lulcc_boundary) {
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
    if segment.lulcc_boundary {
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
    Ok(runtime
        .river()
        .map(|river| (river.state.clone(), river.tracers.clone())))
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
    for field in [
        "DEF_USE_PFT",
        "DEF_USE_PC",
        "DEF_URBAN_RUN",
        "DEF_USE_IRRIGATION",
        "DEF_Optimize_Baseflow",
    ] {
        ensure!(
            !logical_field(document, field)?,
            "{field} with DEF_USE_LULCC is not ported to the Rust runtime; run it with --engine fortran"
        );
    }
    // `MOD_Namelist` 在 LULCC 时强制月度 LAI、逐年换 LAI；Rust 不替 namelist 改，直接要求。
    for field in ["DEF_LAI_MONTHLY", "DEF_LAI_CHANGE_YEARLY"] {
        ensure!(
            logical_field(document, field)?,
            "DEF_USE_LULCC forces {field} = .true. upstream; set it in the namelist"
        );
    }
    ensure!(
        config.spinup_until == config.start,
        "DEF_USE_LULCC with a spinup interval is not ported to the Rust runtime"
    );
    // 2000 年以前上游每 5 年才换一次土地覆盖，重启年份也按 5 年取整；只接逐年的那段。
    ensure!(
        config.start.year >= 2000,
        "DEF_USE_LULCC before 2000 (five-yearly land cover) is not ported to the Rust runtime"
    );
    Ok(())
}

/// `remap_land_tracer_lulcc_state`（SAT：没有 `lccpct_patches`）：每个新 patch 先取
/// `allocate_Tracer_Vars` 的分配值，同单元同类型的旧 patch（`fallback_source`，与 SAT 配对相同）
/// 整份抄过来（含 `trc_aquifer_ref_water`）。
///
/// 同位素且开了含水层混合（变饱和流、`DEF_TRACER_AQUIFER_MIXING_WATER_MM > 0`）时，上游另查：
/// 特殊地类不能带参考水量，新建的土壤/湿地 patch 不能没有参考水量，否则停机。
fn lulcc_land_tracers(
    set: &colm_core::tracer::TracerSet,
    aquifer_mixing: bool,
    new: &colm_init::lulcc::SatSide<'_>,
    new_patch_type: &[i64],
    old: Option<(colm_init::lulcc::SatSide<'_>, &colm_init::RestartFile)>,
) -> Result<Vec<colm_core::tracer::PatchTracerState>> {
    use colm_core::tracer::PatchTracerState;
    let mut states: Vec<PatchTracerState> = (0..new.patch_class.len())
        .map(|_| PatchTracerState::allocated(set))
        .collect();
    let mut any_reference = false;
    if let Some((old, old_time)) = old {
        let old_states =
            colm_runtime::tracer::read_land_tracer_restart(old_time, set, old.patch_class.len())?
                .context("the old year's restart has no committed land tracer state")?;
        any_reference = old_states.iter().any(|state| state.aquifer_ref_water > 0.0);
        for (n, o) in colm_init::lulcc::match_patches(new, &old)? {
            states[n].pools.clone_from(&old_states[o].pools);
            states[n].aquifer_ref_water = old_states[o].aquifer_ref_water;
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
    boundary: CalendarTime,
    old_dir: &Path,
    target: &Path,
    river: Option<(
        colm_runtime::river::RiverState,
        Option<colm_runtime::river::tracer::RiverTracers>,
    )>,
) -> Result<()> {
    use colm_runtime::spatial::topology::SpatialTopology;
    let SpatialCase {
        name,
        document,
        out,
        ..
    } = *case;
    let label = date_label(normalized_day_end(boundary));
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
    // 1. 新一年的冷启动。
    let mut cold_document = document.clone();
    for (field, value) in [
        ("DEF_simulation_time%start_year", years.new),
        ("DEF_simulation_time%start_month", 1),
        ("DEF_simulation_time%start_day", 1),
        ("DEF_simulation_time%start_sec", 0),
        ("DEF_LC_YEAR", years.new),
    ] {
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
    let mec = (integer_field(document, "DEF_LULCC_SCHEME")? == 2)
        .then(|| -> Result<_> {
            Ok(colm_init::lulcc_mec::MecOptions {
                plant_hydraulics: logical_field(document, "DEF_USE_PLANTHYDRAULICS")?,
                ozone_stress: logical_field(document, "DEF_USE_OZONESTRESS")?,
                // `MOD_Namelist` 在 van Genuchten 下强开变饱和流，用生效值。
                variably_saturated_flow: case.physics.variably_saturated_flow,
                vegetation_snow: case.physics.vegetation_snow,
                // 用已按 schema 解析好的值（缺省是 Fortran 字面量 `1.0_r8`）。
                snow_cover_exponent: case.physics.snow_cover_exponent,
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
    let time_name =
        |year: i64, block: &str| format!("{name}_restart_{label}_lc{year:04}_{block}.nc");
    // 过渡这一步的历史区间已关（`run_spatial_segment` 核对过），旁车是空窗口。
    let history_restart = HistoryRestart {
        config: colm_runtime::history_sidecar::SidecarConfig {
            frequency_code: history_frequency_code(years.history_frequency),
            urban_run: false,
            urban_patches: 0,
            pft_or_pc: false,
            bgc: false,
            crop: false,
            river_lake_flow: true,
        },
        window: None,
        tracer_raw: None,
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
    let mut written = Vec::with_capacity(new_topology.blocks.len());
    for (block, patches) in &new_topology.blocks {
        let cold_path = restart_root.join(&label).join(time_name(years.new, block));
        let cold = colm_init::RestartFile::open(&cold_path)?;
        std::fs::remove_file(&cold_path)
            .with_context(|| format!("cannot remove {}", cold_path.display()))?;
        let new_const = colm_init::RestartFile::open(const_path(years.new, block))?;
        let overrides = match old_topology.blocks.iter().find(|(old, _)| old == block) {
            Some((_, old_patches)) => {
                let old_const = colm_init::RestartFile::open(const_path(years.old, block))?;
                let old_time = colm_init::RestartFile::open(
                    old_dir.join(&label).join(time_name(years.old, block)),
                )?;
                let new_element = &new_topology.element[patches.clone()];
                let old_element = &old_topology.element[old_patches.clone()];
                let sat = colm_init::lulcc::same_type_assignment(
                    &colm_init::lulcc::SatSide {
                        time: &cold,
                        patch_class: new_const.integers("patchclass")?,
                        element: new_element,
                    },
                    &colm_init::lulcc::SatSide {
                        time: &old_time,
                        patch_class: old_const.integers("patchclass")?,
                        element: old_element,
                    },
                    options,
                )
                .with_context(|| {
                    format!("cannot carry the {} state of block {block} over", years.old)
                })?;
                // MEC（`DEF_LULCC_SCHEME = 2`）：SAT 之后按转移份额混合份额有变化的 patch。
                match mec {
                    Some(mec_options) => {
                        let lccpct =
                            read_lulcc_transfer_trace(&landdata, years.new, block, patches.len())?;
                        colm_init::lulcc_mec::mass_energy_conserve(
                            &colm_init::lulcc_mec::MecInputs {
                                new_time: &cold,
                                new_const: &new_const,
                                new_element,
                                old_time: &old_time,
                                old_const: &old_const,
                                old_element,
                                lccpct: &lccpct,
                            },
                            sat,
                            mec_options,
                        )
                        .with_context(|| {
                            format!("cannot conserve mass and energy in block {block}")
                        })?
                    }
                    None => sat,
                }
            }
            None => Vec::new(),
        };
        let path = target.join(&label).join(time_name(years.new, block));
        std::fs::create_dir_all(path.parent().expect("a restart path has a parent"))?;
        cold.write_with(&path, &overrides)?;
        if let Some((set, mixing)) = &land_tracers {
            let old = match old_topology.blocks.iter().find(|(old, _)| old == block) {
                Some((_, old_patches)) => Some((
                    colm_init::RestartFile::open(const_path(years.old, block))?,
                    colm_init::RestartFile::open(
                        old_dir.join(&label).join(time_name(years.old, block)),
                    )?,
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
                },
                new_const.integers("patchtype")?,
                old.as_ref()
                    .map(|(old_const, old_time, old_patches)| -> Result<_> {
                        Ok((
                            colm_init::lulcc::SatSide {
                                time: old_time,
                                patch_class: old_const.integers("patchclass")?,
                                element: &old_topology.element[old_patches.clone()],
                            },
                            old_time,
                        ))
                    })
                    .transpose()?,
            )
            .with_context(|| format!("cannot carry the tracers of block {block} over"))?;
            colm_runtime::tracer::write_land_tracer_restart(
                &path,
                set,
                &states.iter().collect::<Vec<_>>(),
                *mixing,
                None,
            )?;
        }
        // 过渡这一步的历史区间已关，旁车不带示踪物部分。
        mark_history_restart_with_river(&path, &history_restart, None, false, None)?;
        written.push(path);
    }
    // 合并续跑不留在 `restart/` 时，冷启动建的日期目录空了就收掉。
    if target != restart_root.as_path() {
        let _ = std::fs::remove_dir(restart_root.join(&label));
    }
    // 3. 河道：网络不变，状态接着用。
    if let Some((mut state, river_tracers)) = river {
        let network = colm_runtime::river::network::RiverNetwork::read(
            Path::new(&string_field(document, "DEF_UnitCatchment_file")?),
            logical_field(document, "DEF_GridRiverLake_FloodplainStorageFix")?,
        )?;
        // `grid_riverlake_flow_lulcc`：河道时变量原样保留（上游 hold/restore），堤内蓄量与分汊
        // 路径状态在各自模块里本就不动；之后重建 `volwater_ucat`（有堤单元流域只补堤外可见那份），
        // 开分汊时上一子步水深取当前水深。漫滩回馈与 LULCC 同开上游自己就拒绝。
        let reservoir = if integer_field(document, "DEF_Reservoir_Method")? > 0 {
            Some(colm_runtime::river::reservoir::Reservoir::read(
                Path::new(&string_field(document, "DEF_ReservoirPara_file")?),
                &network,
                integer_field(document, "DEF_Reservoir_Method")?,
            )?)
        } else {
            None
        };
        let levee = if logical_field(document, "DEF_USE_LEVEE")? {
            let reservoir_cells = reservoir.as_ref().map_or_else(
                || vec![false; network.len()],
                |r| r.of_catchment.iter().map(Option::is_some).collect(),
            );
            Some(colm_runtime::river::levee::Levee::read(
                Path::new(&string_field(document, "DEF_UnitCatchment_file")?),
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
        let path = river_restart_path(target, name, &label, years.new);
        colm_runtime::river::restart::write_river_state(
            &path,
            &network,
            &state,
            reservoir.as_ref().map(|r| r.identity()).as_deref(),
            u8::try_from(integer_field(document, "DEF_REST_CompressLevel")?)
                .context("DEF_REST_CompressLevel must fit 0..=9")?,
        )?;
        // 河道示踪物：上游在内存里原样留着（`grid_riverlake_flow_lulcc` 不碰），续跑照常提交。
        match river_tracers {
            Some(mut tracers) => colm_runtime::river::restart::write_river_tracers(
                &path,
                &network,
                &mut tracers,
                u8::try_from(integer_field(document, "DEF_REST_CompressLevel")?)
                    .context("DEF_REST_CompressLevel must fit 0..=9")?,
            )?,
            None if logical_field(document, "DEF_USE_TRACER")? => {
                colm_runtime::river::restart::write_empty_river_tracers(&path)?;
            }
            None => {}
        }
        written.push(path);
    }
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
    Ok(())
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
        if let Some(setup) = colm_runtime::methane::setup_from_document(document, false)? {
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
            let state = colm_runtime::irrigation::initial_state(
                &colm_init::RestartFile::open(&files.time)?,
                patch,
                readin,
            )?;
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
        ensure!(
            !template.physics.use_pc,
            "the Rust spatial runtime reads LCT, PFT and urban LAI; PC LAI is not ported"
        );
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
    colm_runtime::history_sidecar::write_sidecar_with_river(
        &sidecar,
        patches,
        &history.config,
        &windows,
        river_required,
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
            // 空间算例的非土壤 patch 没有 PFT（区间为空），不改 PFT 重启。
            if template.pft.is_none() {
                lists.push(Vec::new());
                continue;
            }
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
    // 空间算例一块里有多个城市单元：每个城市 patch 只改自己那一格（`urban_index`），
    // 以第一个的整变量为底，依次把其余单元的那一格拷进去。
    let urban_patches = templates
        .iter()
        .zip(states)
        .filter_map(|(template, state)| {
            template
                .urban
                .as_ref()
                .zip(state.urban.as_ref())
                .map(|(t, s)| (template, state, t, s))
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
        for (template, state, urban_template, urban) in urban_patches {
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
    if bgc && logical_field(document, "DEF_USE_FIRE")? {
        ensure!(
            selection == resolve(true, &[])?,
            "DEF_USE_FIRE with a DEF_hist_vars selection is not ported (the fire history writes the \
             residual of whichever variable was written before it); run this case with --engine fortran"
        );
    }
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
        })
    }
}

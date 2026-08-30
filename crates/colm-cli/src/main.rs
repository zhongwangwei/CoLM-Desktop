//! CoLM 单点的唯一编排可执行文件。
//!
//! `design.md` §4.2：「GUI 只跟它说话」。所以这里是唯一一处同时依赖全部五层的
//! 地方，各层之间仍然互不依赖 —— 造算例的 `colm-case` 不认识内核，
//! 答「能产出什么」的 `colm-hist` 闸门表不认识 netcdf。
//!
//! ```text
//! colm-cli scan      --dir <Sitedata 目录> [--forcing-dir <Forcing 目录>]
//!                    [--out sites.json] [--quick 1]
//! colm-cli site-new  --out <site.nc> --lon <度> --lat <度> [--landtype N] [--rawdata <目录>] [--crop 1]
//!                    [--mode igbp|usgs|pft|pc|urban|urban-igbp|urban-usgs|urban-pft|urban-pc]
//! colm-cli site-pfts <site.nc> [--crop 1] [--landtype N]
//! colm-cli new       --site <站点文件> --out <目录> [--name N] [--start Y-M-D] [--end Y-M-D]
//!                    [--spinup-years N] [--spinup-repeat N]
//!                    [--mode igbp|usgs|pft|pc|urban|urban-igbp|urban-usgs|urban-pft|urban-pc]
//! colm-cli run       <算例目录> --kernel <目录> [--stream 1]
//!                    [--stage mksrfdata|mkinidata|colm]
//! colm-cli metrics   <算例目录> --obs <Flux.nc> [--spinup N] [--from UNIX] [--to UNIX]
//!                    [--json 1] [--corrected 1]
//!                    [--summary-only 1] [--pairs-var Rnet ...] [--max-points N]
//! colm-cli evaluation-catalog <算例目录> --obs <Flux.nc>
//! colm-cli evaluation-plan <算例目录> --obs <Flux.nc> --kernel <目录>
//! colm-cli history-catalog <算例目录>
//! colm-cli series    <算例目录> --vars f_rnet,f_fsena [--max-points N] [--out series.json]
//! colm-cli study-parameters
//! colm-cli study-preflight <case-root> --spec study.json
//! colm-cli study-create <case-root> --spec study.json
//! colm-cli study-status <study-dir>
//! colm-cli study-run <study-dir> --kernel <目录> [--stream 1]
//! colm-cli study-export <study-dir> --out <目录>
//! colm-cli study-pause|study-resume|study-cancel <study-dir>
//! colm-cli study-finalize-cancel <study-dir> --pid <pid>
//! colm-cli study-retry <study-dir> [--include-review 1]
//! colm-cli study-apply-preview <study-dir> --member <id|best>
//! colm-cli study-apply <study-dir> --member <id|best> --out <目录> [--name N]
//! colm-cli study-result <study-dir> --path <结果文件>
//! colm-cli all       --site ... --out ... --kernel ... [--obs ...]
//! ```
//!
//! `--start` / `--end` 不给就用强迫场覆盖的完整范围。预热默认重复头一年
//! 1 遍，而预热期是从窗口头上扣的（输出因此少一年）。经纬度与地类读自站点
//! 文件，时间步长读自强迫场文件 —— 这三样都不问用户。

mod fingerprint;
mod observation_table;
mod study;

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use colm_case::{fields, minimal::required, render, CaseSpec, Dirs, Layout, Spinup, Window};
use colm_kernel::outcome::Stage;
use colm_kernel::Kernel;

const USAGE: &str = "\
usage:
  colm-cli scan    --dir <Sitedata 目录> [--forcing-dir <Forcing 目录>]
                   [--out sites.json] [--quick 1]
                   # 列出目录下的站点；--quick 跳过强迫场，只读站点文件
  colm-cli site-new --out <site.nc> --lon <度> --lat <度> [--landtype N] [--crop 1]
                   [--rawdata <dir>]
                   [--mode igbp|usgs|pft|pc|urban|urban-igbp|urban-usgs|urban-pft|urban-pc] [--json 1]
                   # 建一份站点文件：经纬度必给，其余从 rawdata 抽或用
                   # 标称假设。--landtype 不给就不写，让 CoLM 回落
  colm-cli site-pfts <site.nc> [--crop 1] [--landtype N]
                   # 输出 PFT/PC 站点的非零 PFT 类型与归一化比例
  colm-cli new     --site <site.nc> --out <dir> [--name N] [--start Y-M-D] [--end Y-M-D]
                   [--met <Met.nc>]   # 前处理转出来的强迫场；不给就按命名约定
                                      # 在 ../Forcing/ 下找，那两套约定只覆盖
                                      # PLUMBER2 与 Urban-PLUMBER
                   [--spinup-years N] [--spinup-repeat N]   (默认 1 年 x 1 遍)
                   [--mode igbp|usgs|pft|pc|urban|urban-igbp|urban-usgs|urban-pft|urban-pc]
                   [--crop 1]   # CROP 内核：按 croptyp/pctcrop 审核农田站点
                   [--rawdata <dir>] [--runtime <dir>]
                   # 城市站点由文件形状自动识别。两个目录都可选：预抽表盖住的
                   # 21 个 Urban-PLUMBER 站不给也能跑，表外的站点才要 --rawdata
  colm-cli run     <case-dir> --kernel <dir> [--stream 1] [--force 1]
                   [--stage mksrfdata|mkinidata|colm]
                   # --force 忽略指纹，三段全部重跑
                   # --stage 只运行指定阶段；与 --force 合用时强制重跑该阶段
                   # --stream 把子进程每一行原样转发出来（GUI 用；终端下嫌吵）
  colm-cli metrics <case-dir> --obs <Flux.nc> [--spinup N] [--from UNIX] [--to UNIX]
                   [--json 1] [--corrected 1]
                   --corrected: 拿能量闭合订正后的观测比（Qle_cor / Qh_cor）
                   --summary-only: 只返回指标，不携带绘图用配对点
                   --pairs-var: 只评估指定观测变量；可重复给出
                   --max-points: 配对点保极值降采样上限（指标仍用完整样本）
  colm-cli evaluation-catalog <case-dir> --obs <Flux.nc>
                   # 列出全部支持的评估变量及当前算例/观测是否可用
  colm-cli evaluation-plan <case-dir> --obs <Flux.nc> --kernel <dir>
                   # 不依赖 history，按 case.nml + 内核预览可评估目标
  colm-cli history-catalog <case-dir>
  colm-cli series  <case-dir> --vars f_rnet,f_fsena [--from UNIX] [--to UNIX]
                   [--max-points N] [--out series.json]
  colm-cli study-parameters
                   # 输出可采样专家参数元数据
  colm-cli study-preflight <case-root> --spec study.json
                   # 只校验 Study，不创建目录
  colm-cli study-create <case-root> --spec study.json
                   # 创建不确定性分析/参数调优 Study，写采样设计
  colm-cli study-status <study-dir>
                   # 输出 Study manifest 与成员状态
  colm-cli study-run <study-dir> --kernel <dir> [--stream 1]
                   # 串行运行尚未完成的成员算例
  colm-cli study-export <study-dir> --out <dir>
                   # 导出 manifest、samples、status、report.md/html
  colm-cli study-apply-preview <study-dir> --member <id|best>
                   # 只读预览最佳/指定候选会改哪些参数
  colm-cli study-pause|study-resume|study-cancel <study-dir>
                   # 请求暂停派发、恢复派发或取消尚未开始任务
  colm-cli study-finalize-cancel <study-dir> --pid <pid>
                   # GUI 杀死对应调度进程后持久化取消终态
  colm-cli study-finalize-idle-cancel <study-dir>
                   # GUI 无活动登记时，确认调度器已退出后持久化取消终态
  colm-cli study-retry <study-dir> [--include-review 1]
                   # 把失败成员（可选含待复核成员）重新放回队列
  colm-cli study-apply <study-dir> --member <id|best> --out <dir> [--name N]
                   # 将指定成员参数写入新的独立算例
  colm-cli study-result <study-dir> --path <result.json>
                   # 读取并校验单个成员结果
  colm-cli all     --site <site.nc> --out <dir> --kernel <dir> [--obs <Flux.nc>] [--name N]
                   [--start Y-M-D] [--end Y-M-D] [--spinup N]
  colm-cli observation-table-probe <obs.csv|txt|tsv> [--json 1]
                           # 探测可选验证数据，自动识别站点/时间/评估变量
  colm-cli observation-table-convert <obs.csv|txt|tsv> <Observation-dir>
                           --time-column COLUMN [--site-column COLUMN|--site-name NAME]
                           [--variable VAR=column[:qc_column] ...] [--json 1]
                           # 输出 <site>_Flux.nc；缺 QC 时有限值=0/缺失=1
  colm-cli forcing-probe   <met.nc> [--json 1]
  colm-cli forcing-table-probe <forcing.csv|txt|tsv> [--json 1]
                           # 自动识别分隔符、站点/时间/经纬度列与变量候选
  colm-cli forcing-table-convert <src.csv|txt|tsv> <Forcing-dir>
                           --time COLUMN [--site COLUMN]
                           [--lat-column COLUMN --lon-column COLUMN]
                           [--landtype-column COLUMN] [--offset-column COLUMN]
                           [--land-cover-scheme IGBP|USGS|PFT|PC|Urban]
                           [--lat LAT --lon LON] [--utc-offset HOURS]
                           [--step-seconds N] [--height V,T,Q]
                           --slot N=column:units[+extra] ... [--json 1]
                           # 按站点拆分到 .colm-tabular，缺失整行时间补成缺测值
  colm-cli netcdf-probe    <data.nc> [--json 1]
                           # 探测一份强迫场文件：八个槽位各猜到了什么变量，
                           # 猜不到就是 null；三个观测高度缺失时也是 null，
                           # 不是 NaN —— GUI 前处理页据此决定问不问用户
  colm-cli mesh-new --out <mesh.nc> --nlon N --nlat N [--grid-kind latlon|unstructured]
                    [--west W --east E --south S --north N | --shp basin.shp]
                    [--non-ocean-mask mask.nc --non-ocean-var non_ocean_mask]
                    # 生成 GRIDBASED landmask 或 int64 UNSTRUCTURED elmindex；无 bbox/SHP 时为全球
  colm-cli spatial-preflight --grid-kind latlon|unstructured|catchment --input <mesh.nc>
                    [--out manifest.json]
                    # 在启动 CoLM 前校验空间文件字段并记录 sha256
  colm-cli forcing-convert <src.nc> <dst.nc> [--slot N=name:units[+extra] ...] [--height V,T,Q]
                           # 与独立 bin forcing-convert 同样的行为，供 GUI 走
                           # sidecar 调用；没给 --slot 的槽位走自动匹配
  colm-cli forcing-gap-probe <src.nc> [--slot N=name:units[+extra] ...]
                           [--short-gap N] [--utc-offset HOURS] [--lat LAT --lon LON] [--json 1]
                           # 只诊断缺口与时区，不修改源文件
  colm-cli forcing-repair <src.nc> <dst.nc> [--slot N=name:units[+extra] ...]
                           [--short-gap N] [--utc-offset HOURS] [--lat LAT --lon LON]
                           [--era5 FILE_OR_DIR] [--min-overlap N] [--json 1]
                           # 写一份带逐时 QC 的修复中间文件，原文件不动
  colm-cli era5land-download <dst-dir> --lat LAT --lon LON --start YYYY-MM-DD --end YYYY-MM-DD
                           # 用本机 Python cdsapi 和 ~/.cdsapirc 下载对应 0.1° 格点
";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        print!("{USAGE}");
        std::process::exit(2);
    };
    let opts = Opts::parse(&args[1..])?;
    match cmd.as_str() {
        "help" | "-h" | "--help" => print!("{USAGE}"),
        "scan" => {
            cmd_scan(
                &opts.need("--dir")?,
                opts.get("--forcing-dir").as_deref().map(Path::new),
                opts.get("--out").as_deref(),
                opts.get("--quick").is_some(),
            )?;
        }
        "site-new" => {
            cmd_site_new(&opts)?;
        }
        "site-pfts" => {
            let site = opts.positional_at(0, "a site.nc file")?;
            let crop = opts.get("--crop").is_some();
            let landtype = opts
                .get("--landtype")
                .map(|value| {
                    value
                        .parse::<i32>()
                        .context("--landtype must be an integer")
                })
                .transpose()?;
            let components = colm_srfdata::site::pft_components(&site, crop, landtype)?
                .into_iter()
                .map(|p| serde_json::json!({ "pft_type": p.pft_type, "fraction": p.fraction }))
                .collect::<Vec<_>>();
            println!("{}", serde_json::to_string(&components)?);
        }
        "new" => {
            let case = cmd_new(&opts)?;
            println!("case ready: {}", case.display());
        }
        "run" => {
            cmd_run(
                &opts.positional_case()?,
                &opts.need("--kernel")?,
                opts.get("--stream").is_some(),
                opts.get("--force").is_some(),
                requested_run_stage(opts.get("--stage").as_deref())?,
            )?;
        }
        "metrics" => {
            let case = opts.positional_case()?;
            let obs_path = opts.need("--obs")?;
            cmd_metrics(MetricsRequest {
                case: &case,
                obs_path: &obs_path,
                spinup: opts.spinup()?,
                json: opts.get("--json").is_some(),
                corrected: opts.get("--corrected").is_some(),
                summary_only: opts.get("--summary-only").is_some(),
                pair_vars: opts.get_all("--pairs-var"),
                pair_max_points: opts.get("--max-points").map(|v| v.parse()).transpose()?,
                from: opts.get("--from").map(|v| v.parse()).transpose()?,
                to: opts.get("--to").map(|v| v.parse()).transpose()?,
            })?;
        }
        "evaluation-catalog" => {
            cmd_evaluation_catalog(&opts.positional_case()?, &opts.need("--obs")?)?;
        }
        "evaluation-plan" => {
            cmd_evaluation_plan(
                &opts.positional_case()?,
                &opts.need("--obs")?,
                &opts.need("--kernel")?,
            )?;
        }
        "series" => {
            let case = opts.positional_case()?;
            cmd_series(
                &case,
                &opts.need_str("--vars")?,
                opts.get("--out").as_deref(),
                opts.get("--from").map(|v| v.parse()).transpose()?,
                opts.get("--to").map(|v| v.parse()).transpose()?,
                opts.get("--max-points").map(|v| v.parse()).transpose()?,
            )?;
        }
        "history-catalog" => {
            cmd_history_catalog(&opts.positional_case()?)?;
        }
        "study-params" | "study-parameters" => println!("{}", study::engine::parameters_json()?),
        "study-preflight" | "study-create" => {
            let case_root = opts.positional_case()?;
            let spec_file = match opts.get("--spec") {
                Some(path) => PathBuf::from(path),
                None => opts.positional_at(1, "a Study spec file")?,
            };
            study::runner::preflight_create(&case_root, &spec_file)?;
            if cmd == "study-preflight" {
                println!("ok");
            } else {
                let manifest = study::engine::create(&case_root, &spec_file)?;
                println!("{}", manifest.root);
            }
        }
        "study-status" => {
            #[derive(serde::Serialize)]
            struct StudyStatusJson {
                manifest: study::spec::Manifest,
                state: Option<study::state::StudyState>,
                pause_requested: bool,
                cancel_requested: bool,
                results: Vec<study::runner::ResultFile>,
                events: Vec<serde_json::Value>,
            }
            let study_dir = opts.positional_case()?;
            println!(
                "{}",
                serde_json::to_string(&StudyStatusJson {
                    manifest: study::engine::status(&study_dir)?,
                    state: study::runner::status_state(&study_dir)?,
                    pause_requested: study::state::pause_requested(&study_dir),
                    cancel_requested: study::state::cancel_requested(&study_dir),
                    results: study::runner::result_files(&study_dir)?,
                    events: study::runner::event_log_tail(&study_dir, 300)?,
                })?
            );
        }
        "study-result" => print!(
            "{}",
            study::runner::read_result(
                &opts.positional_case()?,
                Path::new(&opts.need_str("--path")?)
            )?
        ),
        "study-run" => {
            let study_dir = opts.positional_case()?;
            let manifest = study::engine::status(&study_dir)?;
            let kernel = opts
                .get("--kernel")
                .or_else(|| manifest.spec.kernel_dir.clone())
                .context("study-run needs --kernel or spec.kernel_dir")?;
            cmd_study_run(
                &study_dir,
                Path::new(&kernel),
                opts.get("--stream").is_some(),
                opts.get("--jobs")
                    .map(|v| v.parse())
                    .transpose()?
                    .unwrap_or(manifest.spec.budget.jobs),
                opts.get("--retry-failed").is_some(),
            )?;
        }
        "study-export" => {
            let output = match opts.get("--out") {
                Some(path) => PathBuf::from(path),
                None => opts.positional_at(1, "an export directory")?,
            };
            study::export::export(&opts.positional_case()?, &output)?;
        }
        "study-pause" => study::state::request_pause(&opts.positional_case()?)?,
        "study-resume" => study::state::resume(&opts.positional_case()?)?,
        "study-cancel" => study::state::request_cancel(&opts.positional_case()?)?,
        "study-finalize-cancel" => {
            study::runner::finalize_cancel(
                &opts.positional_case()?,
                opts.need_str("--pid")?
                    .parse()
                    .context("--pid must be an integer")?,
            )?;
        }
        "study-finalize-idle-cancel" => {
            study::runner::finalize_idle_cancel(&opts.positional_case()?)?;
        }
        "study-retry" => cmd_study_retry(
            &opts.positional_case()?,
            opts.get("--include-review").is_some(),
        )?,
        "study-apply" => cmd_study_apply(
            &opts.positional_case()?,
            &opts.need_str("--member")?,
            &opts.need("--out")?,
            opts.get("--name"),
        )?,
        "study-apply-preview" => {
            cmd_study_apply_preview(&opts.positional_case()?, &opts.need_str("--member")?)?
        }
        "all" => {
            let case = cmd_new(&opts)?;
            cmd_run(
                &case,
                &opts.need("--kernel")?,
                opts.get("--stream").is_some(),
                // `all` 刚造完算例，三段都没跑过 —— 指纹本来就是空的，
                // 这里传 false 与 true 等价，写 false 以免暗示它会跳过什么。
                false,
                None,
            )?;
            match opts.get("--obs") {
                Some(obs) => cmd_metrics(MetricsRequest {
                    case: &case,
                    obs_path: Path::new(&obs),
                    spinup: opts.spinup()?,
                    json: false,
                    corrected: false,
                    summary_only: false,
                    pair_vars: Vec::new(),
                    pair_max_points: None,
                    from: None,
                    to: None,
                })?,
                None => println!("no --obs given; skipping the metrics table"),
            }
        }
        "observation-table-probe" => {
            let result =
                observation_table::probe(&opts.positional_at(0, "an observation CSV/TXT file")?)?;
            if opts.get("--json").is_some() {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("{} row(s), {} site(s)", result.rows, result.sites.len());
            }
        }
        "observation-table-convert" => {
            let opts2 = observation_convert_options(&opts)?;
            let result = observation_table::convert(
                &opts.positional_at(0, "an observation CSV/TXT file")?,
                &opts.positional_at(1, "an Observation destination directory")?,
                &opts2,
            )?;
            if opts.get("--json").is_some() {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                for site in result {
                    println!(
                        "{}: {} row(s) -> {}",
                        site.site,
                        site.rows,
                        site.path.display()
                    );
                }
            }
        }
        "forcing-probe" => {
            cmd_forcing_probe(
                &opts.positional_at(0, "a forcing file")?,
                opts.get("--json").is_some(),
            )?;
        }
        "forcing-table-probe" => {
            cmd_forcing_table_probe(
                &opts.positional_at(0, "a CSV/TXT forcing file")?,
                opts.get("--json").is_some(),
            )?;
        }
        "forcing-table-convert" => {
            cmd_forcing_table_convert(
                &opts.positional_at(0, "a CSV/TXT forcing file")?,
                &opts.positional_at(1, "a forcing destination directory")?,
                &opts,
            )?;
        }
        "netcdf-probe" => {
            cmd_netcdf_probe(
                &opts.positional_at(0, "a NetCDF file")?,
                opts.get("--json").is_some(),
            )?;
        }
        "mesh-new" => cmd_mesh_new(&opts)?,
        "spatial-preflight" => cmd_spatial_preflight(&opts)?,
        "forcing-convert" => {
            cmd_forcing_convert(
                &opts.positional_at(0, "a source forcing file")?,
                &opts.positional_at(1, "a destination file")?,
                &opts.get_all("--slot"),
                opts.get("--height").as_deref(),
            )?;
        }
        "forcing-gap-probe" => {
            let src = opts.positional_at(0, "a source forcing file")?;
            let plan = forcing_repair_plan(&src, &opts)?;
            let summary = colm_forcing::diagnose_file(&src, &plan)?;
            print_repair_summary(&summary, opts.get("--json").is_some())?;
        }
        "forcing-repair" => {
            let src = opts.positional_at(0, "a source forcing file")?;
            let dst = opts.positional_at(1, "a repaired destination file")?;
            let plan = forcing_repair_plan(&src, &opts)?;
            let summary = colm_forcing::repair_file(&src, &dst, &plan)?;
            if opts.get("--json").is_some() {
                print_repair_summary(&summary, true)?;
            } else {
                println!("wrote {}", dst.display());
                print_repair_summary(&summary, false)?;
            }
        }
        "era5land-download" => {
            cmd_era5land_download(
                &opts.positional_at(0, "an ERA5-Land destination directory")?,
                opts.need_f64("--lat")?,
                opts.need_f64("--lon")?,
                &opts.need_str("--start")?,
                &opts.need_str("--end")?,
            )?;
        }
        other => {
            eprintln!("unknown command: {other}\n{USAGE}");
            std::process::exit(2);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- 参数

fn observation_convert_options(opts: &Opts) -> Result<observation_table::ConvertOptions> {
    let variables = opts
        .get_all("--variable")
        .into_iter()
        .map(|spec| {
            let (name, rest) = spec
                .split_once('=')
                .with_context(|| format!("--variable {spec:?} must be VAR=column[:qc_column]"))?;
            let (column, qc_column) = rest
                .split_once(':')
                .map(|(a, b)| (a.to_string(), Some(b.to_string())))
                .unwrap_or_else(|| (rest.to_string(), None));
            Ok(observation_table::VariableChoice {
                name: name.to_string(),
                column,
                qc_column,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(observation_table::ConvertOptions {
        time_column: opts.need_str("--time-column")?,
        site_column: opts.get("--site-column"),
        site_name: opts.get("--site-name"),
        variables,
    })
}

struct Opts {
    flags: Vec<(String, String)>,
    positional: Vec<String>,
}

impl Opts {
    fn parse(args: &[String]) -> Result<Opts> {
        let (mut flags, mut positional) = (Vec::new(), Vec::new());
        let mut i = 0;
        while i < args.len() {
            if let Some(name) = args[i].strip_prefix("--") {
                let v = args
                    .get(i + 1)
                    .with_context(|| format!("--{name} needs a value"))?;
                flags.push((format!("--{name}"), v.clone()));
                i += 2;
            } else {
                positional.push(args[i].clone());
                i += 1;
            }
        }
        Ok(Opts { flags, positional })
    }

    fn get(&self, name: &str) -> Option<String> {
        self.flags
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    }

    /// 同一个 flag 出现多次时的全部值，按出现顺序。`forcing-convert` 的
    /// `--slot` 每个槽位一条，`get` 只会拿到第一条。
    fn get_all(&self, name: &str) -> Vec<String> {
        self.flags
            .iter()
            .filter(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .collect()
    }

    fn need(&self, name: &str) -> Result<PathBuf> {
        match self.get(name) {
            Some(v) => Ok(PathBuf::from(v)),
            None => bail!("{name} is required\n{USAGE}"),
        }
    }

    fn need_str(&self, name: &str) -> Result<String> {
        match self.get(name) {
            Some(v) => Ok(v),
            None => bail!("{name} is required\n{USAGE}"),
        }
    }

    fn positional_case(&self) -> Result<PathBuf> {
        match self.positional.first() {
            Some(p) => Ok(PathBuf::from(p)),
            None => bail!("a case directory is required\n{USAGE}"),
        }
    }

    /// 第 `i` 个位置参数（0-based）。`what` 是找不到时该说它是什么——
    /// `forcing-probe`/`forcing-convert` 的位置参数是文件而不是算例目录，
    /// `positional_case` 那句「a case directory is required」在这里会说错话。
    fn positional_at(&self, i: usize, what: &str) -> Result<PathBuf> {
        match self.positional.get(i) {
            Some(p) => Ok(PathBuf::from(p)),
            None => bail!("{what} is required\n{USAGE}"),
        }
    }

    /// 一个带默认值的非负整数选项。
    ///
    /// 与 [`Opts::spinup`] 不同：那个的默认值 0 有语义（不剔除），
    /// 这个的默认值由调用方给 —— 预热的两项默认都不是 0。
    fn count(&self, name: &str, default: u32) -> Result<u32> {
        match self.get(name) {
            None => Ok(default),
            Some(v) => v
                .parse()
                .with_context(|| format!("{name} {v:?} is not a count")),
        }
    }

    /// 剔除的 spin-up 记录条数。design.md 两个窗口用的值不同
    /// （冬季 8 小时、湿季 4 天），所以它是参数不是常数。
    fn spinup(&self) -> Result<usize> {
        match self.get("--spinup") {
            None => Ok(0),
            Some(v) => v
                .parse()
                .with_context(|| format!("--spinup {v:?} is not a count")),
        }
    }

    /// 一个必需的浮点数选项——`site-new` 的 `--lon`/`--lat`。
    fn need_f64(&self, name: &str) -> Result<f64> {
        let v = self.need_str(name)?;
        v.parse()
            .with_context(|| format!("{name} {v:?} is not a number"))
    }

    /// 一个可选的整数选项——`site-new` 的 `--landtype`。不给就是
    /// `None`，不猜一个默认值：`site::skeleton` 对「没给地类」与
    /// 「给了某个地类」是两条不同的路径，猜一个会把用户没说的话说死。
    fn get_i32(&self, name: &str) -> Result<Option<i32>> {
        match self.get(name) {
            None => Ok(None),
            Some(v) => v
                .parse()
                .map(Some)
                .with_context(|| format!("{name} {v:?} is not an integer")),
        }
    }
}

// ------------------------------------------------------------- site-new

/// `colm-cli site-new`：从站点身份与经纬度建立标准命名的 site.nc。
///
/// 两步拼起来：[`colm_srfdata::site::skeleton_with_mode`] 写出只有经纬度
/// （可选地类）的最小文件，[`colm_srfdata::site::fill`] 按“站点自有 > 栅格 >
/// 模块默认”补齐 12 个结构字段。Urban 站再复用 `prepare_urban`
/// 补上内置 Urban-PLUMBER 表已有的土壤、LCZ 与树冠数据。
///
/// **结构完整不等于运行完整。** 随后的模式感知审计还会检查地类、LAI/SAI、
/// PFT/PC/城市专属数组与 24 项土壤水热变量，并明确返回 self-contained、
/// 依赖 rawdata 或 blocked。经纬度绝不能被用来编造这些科学输入。
fn cmd_site_new(o: &Opts) -> Result<PathBuf> {
    let out = o.need("--out")?;
    let lon = o.need_f64("--lon")?;
    let lat = o.need_f64("--lat")?;
    let landtype = o.get_i32("--landtype")?;
    let rawdata = o.get("--rawdata");
    let mode = parse_site_mode(o.get("--mode").as_deref())?;
    let crop = o.get("--crop").is_some();

    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }

    // skeleton 与 fill 之间的中间文件：用户不该知道它存在过。
    let skel = out.with_extension(format!("site-new-input-{}.nc", std::process::id()));
    let kind = if mode == colm_srfdata::site::SiteMode::Urban {
        colm_srfdata::site::SiteKind::Urban
    } else {
        colm_srfdata::site::SiteKind::Natural
    };
    colm_srfdata::site::skeleton_with_mode(&skel, lon, lat, landtype, kind, mode, crop)?;
    let generic = if mode == colm_srfdata::site::SiteMode::Urban {
        out.with_extension(format!("site-new-{}.nc", std::process::id()))
    } else {
        out.clone()
    };
    let filled = colm_srfdata::site::fill(&skel, &generic, rawdata.as_deref().map(Path::new), None);
    let _ = std::fs::remove_file(&skel);
    let mut r = filled?;
    if mode == colm_srfdata::site::SiteMode::Urban {
        let prepared = colm_srfdata::site::prepare_urban(&generic, &out);
        let _ = std::fs::remove_file(&generic);
        let prepared = prepared?;
        r.from_lookup.extend(prepared.soil_vars);
        r.from_lookup.extend(prepared.extra_vars);
        r.from_lookup.sort();
        r.from_lookup.dedup();
    }
    let audit = colm_srfdata::site::audit(&out, mode, rawdata.as_deref().map(Path::new), crop)?;

    // 人读输出仍单独说明 IGBP 冠层高度是否来自 CoLM 查表；完整运行缺项只由
    // `site::audit` 生成，不能在 CLI 再维护第二张会漂移的变量表。
    let canopy_height_ready = r.from_lookup.iter().any(|n| n == "canopy_height");

    // `--json 1` 给界面用：三个来源列表要能逐字段摆出来，而人读的那几行
    // 是拼给终端的。**别让界面去解析人读的文本** —— 那是 `scan` 与
    // `metrics` 早就立下的做法（两边各写各的结构体，靠一条拿真输出跑的
    // 测试盯着字段不脱钩）。
    if o.get("--json").is_some() {
        let j = serde_json::json!({
            "path": out.display().to_string(),
            "texture": r.texture,
            "texture_name": r.texture_name,
            "bvic": r.bvic,
            "sand_silt_clay": [r.fine_earth.0, r.fine_earth.1, r.fine_earth.2],
            // 地类没给就没写 —— 界面上要说清楚这不是遗漏，是有意的。
            "landtype": landtype,
            "from_site": r.from_site,
            "from_raster": r.from_raster,
            "from_default": r.from_default,
            "from_lookup": r.from_lookup,
            // 这份 site.nc 还需要哪些外部数据 mksrfdata 才能跑完 ——
            // 见上面 `needs_external` 的注释。界面要把这个显示出来。
            "needs_external": audit.needs_external,
            "site_kind": audit.kind.as_str(),
            "mode": audit.mode.as_str(),
            "readiness": audit.readiness.as_str(),
            "self_contained": audit.self_contained(),
        });
        println!("{}", serde_json::to_string_pretty(&j)?);
        return Ok(out);
    }

    println!(
        "soil texture: {} ({}), BVIC {} from sand {:.2}% / silt {:.2}% / clay {:.2}%",
        r.texture, r.texture_name, r.bvic, r.fine_earth.0, r.fine_earth.1, r.fine_earth.2
    );
    if landtype.is_none() {
        println!(
            "note: no --landtype given; IGBP_classification is not written, so CoLM falls \
             back on its own, and canopy height can't be looked up either (CoLM's IGBP \
             lookup table is indexed by land type) -- mksrfdata will need \
             <rawdata>/plant_15s/ for both canopy height and monthly LAI/SAI \
             (LAI_monthly/SAI_monthly), which this crate never supplies"
        );
    } else if canopy_height_ready {
        println!(
            "note: canopy height filled from CoLM's IGBP lookup table \
             (MOD_Const_LC.F90 htop0_igbp); monthly LAI (LAI_monthly/SAI_monthly) still \
             has to come from --rawdata's plant_15s/ or be present in the site file -- \
             mksrfdata will not finish without it"
        );
    }
    if !r.from_site.is_empty() {
        println!("from site   : {}", r.from_site.join(", "));
    }
    if !r.from_raster.is_empty() {
        println!("from raster : {}", r.from_raster.join(", "));
    }
    if !r.from_default.is_empty() {
        println!(
            "from default: {}  <-- nominal values, not measured at this site",
            r.from_default.join(", ")
        );
    }
    if !r.from_lookup.is_empty() {
        println!(
            "from lookup : {}  <-- evidence-backed CoLM tables, not measured at this site",
            r.from_lookup.join(", ")
        );
    }
    println!("wrote {}", out.display());
    println!(
        "readiness: {} (mode {}, kind {})",
        audit.readiness.as_str(),
        audit.mode.as_str(),
        audit.kind.as_str()
    );
    if !audit.needs_external.is_empty() {
        println!("external data: {}", audit.needs_external.join(", "));
    }
    Ok(out)
}

fn parse_site_mode(value: Option<&str>) -> Result<colm_srfdata::site::SiteMode> {
    use colm_srfdata::site::SiteMode;
    match value.unwrap_or("igbp").to_ascii_lowercase().as_str() {
        "igbp" => Ok(SiteMode::Igbp),
        "usgs" => Ok(SiteMode::Usgs),
        "pft" => Ok(SiteMode::Pft),
        "pc" => Ok(SiteMode::Pc),
        "urban" | "urban-igbp" | "urban-usgs" | "urban-pft" | "urban-pc" => {
            Ok(SiteMode::Urban)
        }
        other => bail!(
            "unsupported --mode {other:?}; use igbp, usgs, pft, pc, urban, urban-igbp, urban-usgs, urban-pft, or urban-pc"
        ),
    }
}

#[derive(Clone, Copy)]
struct NewMode {
    site: colm_srfdata::site::SiteMode,
    subgrid: Subgrid,
    urban_landtype: Option<i32>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Subgrid {
    Lct,
    Pft,
    Pc,
}

fn parse_new_mode(value: Option<&str>) -> Result<NewMode> {
    use colm_case::build::{URBAN_LANDTYPE_IGBP, URBAN_LANDTYPE_USGS};
    use colm_srfdata::site::SiteMode;
    Ok(match value.unwrap_or("igbp").to_ascii_lowercase().as_str() {
        "igbp" => NewMode {
            site: SiteMode::Igbp,
            subgrid: Subgrid::Lct,
            urban_landtype: None,
        },
        "usgs" => NewMode {
            site: SiteMode::Usgs,
            subgrid: Subgrid::Lct,
            urban_landtype: None,
        },
        "pft" => NewMode {
            site: SiteMode::Pft,
            subgrid: Subgrid::Pft,
            urban_landtype: None,
        },
        "pc" => NewMode {
            site: SiteMode::Pc,
            subgrid: Subgrid::Pc,
            urban_landtype: None,
        },
        "urban" | "urban-igbp" => NewMode {
            site: SiteMode::Urban,
            subgrid: Subgrid::Lct,
            urban_landtype: Some(URBAN_LANDTYPE_IGBP),
        },
        "urban-usgs" => NewMode {
            site: SiteMode::Urban,
            subgrid: Subgrid::Lct,
            urban_landtype: Some(URBAN_LANDTYPE_USGS),
        },
        "urban-pft" => NewMode {
            site: SiteMode::Urban,
            subgrid: Subgrid::Pft,
            urban_landtype: Some(URBAN_LANDTYPE_IGBP),
        },
        "urban-pc" => NewMode {
            site: SiteMode::Urban,
            subgrid: Subgrid::Pc,
            urban_landtype: Some(URBAN_LANDTYPE_IGBP),
        },
        other => bail!(
            "unsupported --mode {other:?}; use igbp, usgs, pft, pc, urban, urban-igbp, urban-usgs, urban-pft, or urban-pc"
        ),
    })
}

// ---------------------------------------------------------------- new

/// 从 `Sitedata/<X>_site.nc` 推出 `Forcing/<X>_Met.nc`。
///
/// 实测 PLUMBER2 的三个目录用同一个词干，只差后缀
/// （`_site.nc` / `_Met.nc` / `_Flux.nc`），所以用户只需要给一个文件。
/// 两套站点数据集的命名约定。各自的三个目录共用一个词干，只差后缀。
///
/// | | 站点 | 强迫场 | 观测 |
/// |---|---|---|---|
/// | PLUMBER2 | `<X>_site.nc` | `<X>_Met.nc` | `<X>_Flux.nc` |
/// | Urban-PLUMBER | `<X>_site_v1.nc` | `<X>_metforcing_v1.nc` | `<X>_clean_observations_v1.nc` |
const LAYOUTS: [(&str, &str, &str); 2] = [
    ("_site.nc", "_Met.nc", "_Flux.nc"),
    (
        "_site_v1.nc",
        "_metforcing_v1.nc",
        "_clean_observations_v1.nc",
    ),
];

/// 站点文件旁边的强迫场（`which = 0`）或观测（`which = 1`）文件。
///
/// 两套约定都试 —— 用户只给一个站点文件路径，其余两个推得出来。
fn sibling(site: &Path, dir: &str, which: usize) -> Option<PathBuf> {
    companion_name(site, which).and_then(|name| {
        let p = site.parent()?.parent()?.join(dir).join(name);
        p.exists().then_some(p)
    })
}

/// 按数据集命名约定，从站点文件名得到配套的强迫场或观测文件名。
fn companion_name(site: &Path, which: usize) -> Option<String> {
    let name = site.file_name()?.to_str()?;
    for (site_suffix, met, flux) in LAYOUTS {
        let Some(stem) = name.strip_suffix(site_suffix) else {
            continue;
        };
        return Some(format!("{stem}{}", if which == 0 { met } else { flux }));
    }
    None
}

/// 扫描时显式选择的强迫场目录优先；不给才使用 Sitedata 的兄弟目录。
fn forcing_for(site: &Path, forcing_dir: Option<&Path>) -> Option<PathBuf> {
    match forcing_dir {
        Some(dir) => companion_name(site, 0)
            .map(|name| dir.join(name))
            .filter(|path| path.is_file()),
        None => sibling(site, "Forcing", 0),
    }
}

/// 强迫场文件：显式给了就用它，没给才按命名约定推。
///
/// **显式优于约定。** `LAYOUTS` 那两套（`_Met.nc` / `_metforcing_v1.nc`）
/// 是 PLUMBER2 与 Urban-PLUMBER 的**内部约定** —— 对内置数据集是合理的
/// 默认，但拿自己数据的人没有理由把文件命名成那样。
///
/// 更要紧的是：不给显式路径时 `sibling()` 会推出**原始**强迫场并静默
/// 用它。用户在前处理页转换过一份（合并了降雪、补了观测高度），
/// 建算例时却跑的是原始文件 —— 不报错，模型跑得完，曲线照样是曲线。
///
/// 显式路径不存在时**报错并点名它**，不回落到约定：用户明确指了一个
/// 文件，悄悄换成别的比直接失败糟得多。
fn resolve_met(explicit: Option<&str>, site: &Path) -> Result<PathBuf> {
    if let Some(p) = explicit {
        let p = PathBuf::from(p);
        if !p.exists() {
            bail!("--met {} does not exist", p.display());
        }
        return colm_kernel::manifest::absolute(&p)
            .with_context(|| format!("cannot resolve --met {}", p.display()));
    }
    sibling(site, "Forcing", 0).with_context(|| {
        format!(
            "cannot find the forcing file next to {}; expected ../Forcing/<stem>_Met.nc \
             (or pass --met <path> to name it directly)",
            site.display()
        )
    })
}

fn complete_forcing_heights(
    summary: &mut colm_forcing::MetSummary,
    site: &Path,
    met: &Path,
) -> Result<()> {
    if valid_height(summary.height_v)
        && valid_height(summary.height_t)
        && valid_height(summary.height_q)
    {
        return Ok(());
    }

    let companion = forcing_height_nml(site, met);
    if let Some(path) = companion.as_ref().filter(|path| path.is_file()) {
        let doc = colm_namelist::parse(&std::fs::read_to_string(path)?)
            .with_context(|| format!("cannot parse {}", path.display()))?;
        if !valid_height(summary.height_v) {
            summary.height_v = nml_height(&doc, "DEF_forcing%HEIGHT_V").unwrap_or(summary.height_v);
        }
        if !valid_height(summary.height_t) {
            summary.height_t = nml_height(&doc, "DEF_forcing%HEIGHT_T").unwrap_or(summary.height_t);
        }
        if !valid_height(summary.height_q) {
            summary.height_q = nml_height(&doc, "DEF_forcing%HEIGHT_Q").unwrap_or(summary.height_q);
        }
    }

    let missing = [
        ("HEIGHT_V", summary.height_v),
        ("HEIGHT_T", summary.height_t),
        ("HEIGHT_Q", summary.height_q),
    ]
    .into_iter()
    .filter_map(|(name, value)| (!valid_height(value)).then_some(name))
    .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        bail!(
            "{} lacks finite positive forcing observation height(s): {}; expected them as reference_height_v/t/q in the NetCDF file or as DEF_forcing%HEIGHT_V/T/Q in {}",
            met.display(),
            missing.join(", "),
            companion
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "../Forcingnml/<site>.nml".to_string())
        )
    }
}

fn valid_height(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn nml_height(doc: &colm_namelist::Document, field: &str) -> Option<f64> {
    doc.get(field)
        .and_then(colm_namelist::Value::as_f64)
        .filter(|v| valid_height(*v))
}

fn forcing_height_nml(site: &Path, met: &Path) -> Option<PathBuf> {
    let stem = site.file_name()?.to_str().and_then(|name| {
        LAYOUTS
            .iter()
            .find_map(|(suffix, _, _)| name.strip_suffix(suffix))
    })?;
    let short = stem.split('_').next().unwrap_or(stem);
    Some(
        met.parent()?
            .parent()?
            .join("Forcingnml")
            .join(format!("{short}.nml")),
    )
}

/// 一个站点在界面上要显示的一切。
///
/// **一次把该读的都读了。** 界面要经纬度、地类、时间范围与步长，
/// 而这些分散在两个文件里；分两次扫等于把 90 个站点的文件各打开两遍。
#[derive(serde::Serialize)]
struct SiteInfo {
    /// 站点代号，词干第一段（`AT-Neu_2002-2012_..._site.nc` -> `AT-Neu`）
    name: String,
    site_file: String,
    /// 找不到就是 `None` —— 界面据此把「运行」置灰并说明原因，
    /// 而不是等用户点下去才报错。
    met_file: Option<String>,
    /// 观测文件。没有就不能自动评估，这一条决定评估按钮的死活。
    obs_file: Option<String>,
    /// 城市形状：站点文件不带 `IGBP_classification`。预抽表盖住的 21 个
    /// Urban-PLUMBER 站不需要 `--rawdata`/`--runtime`；表外的城市站点仍然
    /// 要 `--rawdata`。界面据此决定问不问。
    urban: bool,
    /// 用户明确创建的 CROP 站点，或带 croptyp/pctcrop 的 IGBP 农田站点。
    crop: bool,
    lon: f64,
    lat: f64,
    landtype: Option<i32>,
    /// 以下三项要打开强迫场文件。`--quick` 时全为 `None`。
    start: Option<String>,
    end: Option<String>,
    step_seconds: Option<f64>,
    /// 读这个站点时出的问题，原样带出去。**不让一个坏文件毁掉整次扫描** ——
    /// 90 个站点里有一个读不了，其余 89 个仍然要列出来。
    problem: Option<String>,
}

fn cmd_scan(dir: &Path, forcing_dir: Option<&Path>, out: Option<&str>, quick: bool) -> Result<()> {
    let mut sites: Vec<SiteInfo> = Vec::new();
    let rd = std::fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))?;
    let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    files.sort(); // 顺序稳定：界面上的列表不该每次刷新都换一个次序

    for p in files {
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // 站点文件的判据是命名约定，与 `sibling` 用的是同一张表。
        let Some(stem) = LAYOUTS
            .iter()
            .find_map(|(suffix, _, _)| name.strip_suffix(suffix))
        else {
            continue;
        };
        let short = stem.split('_').next().unwrap_or(stem).to_string();
        let met = forcing_for(&p, forcing_dir);
        let obs = sibling(&p, "Observation", 1);

        let (mut lon, mut lat, mut landtype, mut problem) = (f64::NAN, f64::NAN, None, None);
        match colm_srfdata::site::location(&p) {
            Ok(l) => {
                lon = l.lon;
                lat = l.lat;
                landtype = l.landtype;
            }
            Err(e) => problem = Some(format!("{e:#}")),
        }

        let (mut start, mut end, mut step) = (None, None, None);
        if !quick {
            if let Some(m) = &met {
                match colm_forcing::summarize(m) {
                    Ok(s) => {
                        let e = s.end();
                        start = Some(format!(
                            "{}-{:02}-{:02}",
                            s.start.year, s.start.month, s.start.day
                        ));
                        end = Some(format!("{}-{:02}-{:02}", e.year, e.month, e.day));
                        step = Some(s.step_seconds);
                    }
                    Err(e) => problem = Some(format!("{e:#}")),
                }
            }
        }

        let crop = match colm_srfdata::site::crop_site(&p, landtype) {
            Ok(crop) => crop,
            Err(error) => {
                problem.get_or_insert_with(|| format!("{error:#}"));
                false
            }
        };

        sites.push(SiteInfo {
            name: short,
            site_file: text(&p),
            met_file: met.as_deref().map(text),
            obs_file: obs.as_deref().map(text),
            // Missing land type is not an urban marker. Generated natural files may
            // intentionally leave it unresolved for rawdata; scan and `new` share the
            // same classifier so they cannot silently disagree.
            urban: problem.is_none()
                && colm_srfdata::site::site_kind(&p)
                    .is_ok_and(|kind| kind == colm_srfdata::site::SiteKind::Urban),
            crop,
            lon,
            lat,
            landtype,
            start,
            end,
            step_seconds: step,
            problem,
        });
    }

    let json = serde_json::to_string_pretty(&sites)?;
    match out {
        Some(f) => {
            std::fs::write(f, &json).with_context(|| format!("cannot write {f}"))?;
            let with_obs = sites.iter().filter(|s| s.obs_file.is_some()).count();
            println!(
                "wrote {} site(s) to {f} ({with_obs} with observations)",
                sites.len()
            );
        }
        None => println!("{json}"),
    }
    Ok(())
}

fn cmd_new(o: &Opts) -> Result<PathBuf> {
    // **站点文件的路径要先绝对化。** 强迫场文件的路径是从它推出来的，
    // 并原样写进 `forcing.nml`；而模型跑在算例目录里（`run_stage` 设了
    // `current_dir(work)`），相对路径到那里就解析不到了 ——
    // 报的还是 `<相对路径> does not exist`，看上去像文件缺失而不是路径问题。
    // 与 `Kernel::open` 和 `cmd_run` 的算例目录是同一类问题，第三次遇到。
    let site_raw = {
        let given = o.need("--site")?;
        colm_kernel::manifest::absolute(&given)
            .with_context(|| format!("cannot resolve --site {}", given.display()))?
    };
    let out = o.need("--out")?;
    // CoLM 有 55 处不加引号的 `CALL system('mkdir -p ' // trim(dir))`，
    // 路径含空格会被 shell 拆成两个参数 —— 建出一棵影子目录树，而报出来的
    // 是 netCDF 的 `Permission denied`，指向完全错误的方向。
    // **在这里拦住，比让人对着那句报错发呆强。**
    if out.to_string_lossy().contains(' ') {
        bail!(
            "the case directory must not contain spaces: {}\n  \
             CoLM builds its output tree with unquoted shell `mkdir -p`, so a path \
             with spaces silently creates the wrong directories and later fails with \
             a misleading `Permission denied` from netCDF",
            out.display()
        );
    }
    if o.get("--crop").is_some() {
        validate_default_crop_runtime(o.get("--runtime").as_deref())?;
    }
    let met = resolve_met(o.get("--met").as_deref(), &site_raw)?;

    std::fs::create_dir_all(&out)?;
    // **算例目录也要绝对化。** 四个路径（含 `DEF_forcing_namelist`）是照着
    // 它拼出来写进 case.nml 的，而模型跑在算例目录里 —— 相对的 `--out`
    // 会让 `probe-case\forcing.nml` 在那里解析成 `probe-case/probe-case/...`，
    // 报的是 `No such file or directory`，看着像文件没生成。
    //
    // 先 `create_dir_all` 再解析：`canonicalize` 要求路径已经存在。
    // 这是同一类问题的第四处（前三处是内核目录、`--site`、`cmd_run` 的算例目录）。
    let out = colm_kernel::manifest::absolute(&out)
        .with_context(|| format!("cannot resolve --out {}", out.display()))?;
    let layout = Layout::new(&out);
    std::fs::create_dir_all(layout.out())?;

    // 1. 补齐站点文件 —— CoLM 缺字段时会回落到几百 GB 的全球 rawdata。
    //
    // 但**只对 PLUMBER2 形状的文件做**。城市站点文件（Urban-PLUMBER）的变量集
    // 完全不同：23 个城市形态学量，没有土壤剖面也没有 `IGBP_classification`，
    // 而 CoLM 的 URBAN 路径本来就直接读原件（实测示例算例用的就是未经处理的
    // 原文件，逐字节相同）。对它做补齐既没有依据，也没有必要。
    let obs = sibling(&site_raw, "Observation", 1);
    let already_filled = colm_srfdata::site::missing_fields(&site_raw)?.is_empty();
    let kind = colm_srfdata::site::site_kind(&site_raw)?;
    let urban = kind == colm_srfdata::site::SiteKind::Urban;
    let new_mode = match o.get("--mode") {
        Some(value) => parse_new_mode(Some(&value))?,
        None if urban => parse_new_mode(Some("urban"))?,
        None => parse_new_mode(None)?,
    };
    let mode = new_mode.site;
    if urban != (mode == colm_srfdata::site::SiteMode::Urban) {
        bail!(
            "site kind {} does not match --mode {}; choose the same natural/urban mode used in the entry wizard",
            kind.as_str(),
            mode.as_str()
        );
    }
    let looks_like_plumber2 = !urban;
    // 这个城市站点在不在两张预抽表里 —— 决定 `--rawdata` 缺席时该说什么。
    if already_filled && urban {
        // `site-new --mode urban` 会先写 12 个结构占位字段（source=synthesized），
        // 但城市全流程还需要土壤、LCZ/LUCY、树 LAI/SAI 等城市专属量。
        // 不能因为“12 个结构字段齐了”就跳过 `prepare_urban`，否则这些占位值会
        // 被 CoLM 当作真实站点数据使用，且不会再读取 rawdata。`prepare_urban`
        // 只替换 synthesized 占位值，站点文件自带的真实变量仍然优先。
        let rep = colm_srfdata::site::prepare_urban(&site_raw, &layout.site_nc())?;
        println!("site: already has all 12 required fields; urban-specific inputs checked");
        if !rep.soil_vars.is_empty() {
            println!(
                "  soil: {} synthesized/absent variable(s) replaced from the built-in urban table",
                rep.soil_vars.len()
            );
        }
        if !rep.extra_vars.is_empty() {
            println!(
                "  urban: {} synthesized/absent variable(s) replaced from the built-in urban table",
                rep.extra_vars.len()
            );
        }
    } else if already_filled {
        // 已经补齐过的自然站点文件。原样拷过去，不再调用 `fill`：它的第一行就是
        // `fs::copy`，第二行会在已经存在的变量名上报 `NC_ENAMEINUSE`。
        std::fs::copy(&site_raw, layout.site_nc()).with_context(|| {
            format!(
                "cannot copy {} to {}",
                site_raw.display(),
                layout.site_nc().display()
            )
        })?;
        println!("site: already has all 12 required fields (from site-new or a prior fill); copied as-is");
    } else if looks_like_plumber2 {
        let rep = colm_srfdata::site::fill(
            &site_raw,
            &layout.site_nc(),
            o.get("--rawdata").as_deref().map(Path::new),
            obs.as_deref(),
        )?;
        println!("site: texture {} ({})", rep.texture, rep.texture_name);
        if !rep.from_default.is_empty() {
            println!(
                "  {} field(s) fell back to module defaults: {}",
                rep.from_default.len(),
                rep.from_default.join(", ")
            );
        }
    } else {
        // 城市站点文件：补高程与土壤剖面，见 `prepare_urban`。
        let rep = colm_srfdata::site::prepare_urban(&site_raw, &layout.site_nc())?;
        match rep.elevation {
            Some(h) => println!(
                "site: urban-shaped; elevation {h} m taken from ground_height so CoLM never \
                 needs the 7 GB elevation.nc"
            ),
            None => println!("site: urban-shaped; elevation left as the file has it"),
        }
        match rep.soil_site {
            Some(name) => println!(
                "  soil: {} variable(s) written from the built-in {name} profile \
                 (measured on the CoLM 2024 grid at this site) — the 122 GB soil/ rasters \
                 are not read",
                rep.soil_vars.len()
            ),
            // **不编数**：表外的站点一个土壤变量都不写，让 CoLM 回落栅格。
            None => println!(
                "  soil: this site is not in the pre-extracted table (21 Urban-PLUMBER sites); \
                 nothing written, so CoLM reads the global soil/ rasters"
            ),
        }
        match rep.extra_site {
            Some(name) => println!(
                "  urban: {} more variable(s) from the built-in {name} row (LCZ class, LUCY \
                 region, soil albedo, lake depth, topography, {} years of tree LAI/SAI) — \
                 the urban_type/ and urban_lai_500m/ tiles are not read",
                rep.extra_vars.len(),
                colm_srfdata::urban_extra::LAI_YEARS.len()
            ),
            None => println!(
                "  urban: this site is not in the pre-extracted table (21 Urban-PLUMBER sites); \
                 nothing written, so CoLM reads the LCZ, urban tree LAI, lake depth, soil \
                 albedo and topography rasters"
            ),
        }
    }

    // The 12-field fill is only the structural layer. Validate the actual
    // mksrfdata-facing contract after preparing the file, and keep every missing
    // dependency visible rather than failing minutes later inside Fortran.
    let site_audit = colm_srfdata::site::audit(
        &layout.site_nc(),
        mode,
        o.get("--rawdata").as_deref().map(Path::new),
        o.get("--crop").is_some(),
    )?;
    if site_audit.readiness == colm_srfdata::site::Readiness::Blocked {
        bail!(
            "{} is structurally valid but is not runnable in {} mode without --rawdata; missing: {}",
            site_raw.display(),
            mode.as_str(),
            site_audit.needs_external.join(", ")
        );
    }
    let urban_covered = urban && site_audit.self_contained();

    // 2. 强迫场 namelist —— 不转换数据，CoLM 直接读 PLUMBER2 的 Met 文件
    let mut summary = colm_forcing::summarize(&met)?;
    complete_forcing_heights(&mut summary, &site_raw, &met)?;
    let problems = colm_forcing::check(&summary, None);
    if !problems.is_empty() {
        for p in &problems {
            eprintln!("  {p}");
        }
        bail!("{} problem(s) with {}", problems.len(), met.display());
    }
    let met_name = met.file_name().unwrap().to_string_lossy().to_string();
    std::fs::write(
        layout.forcing_nml(),
        colm_forcing::render(&colm_forcing::ForcingSpec {
            dir: text(met.parent().unwrap()),
            file: met_name,
            met: summary.clone(),
        }),
    )?;

    // 3. 算例 namelist。窗口不给就用强迫场覆盖的完整范围。
    let start = match o.get("--start") {
        Some(s) => parse_date(&s)?,
        None => (summary.start.year, summary.start.month, summary.start.day),
    };
    let forc_start_sec =
        summary.start.hour * 3600 + summary.start.minute * 60 + summary.start.second;
    let start_sec = if start == (summary.start.year, summary.start.month, summary.start.day) {
        forc_start_sec
    } else {
        0
    };
    let e = summary.end();
    let end = match o.get("--end") {
        Some(s) => parse_date(&s)?,
        None => (e.year, e.month, e.day),
    };
    // **结束那天跑到第几秒，取决于强迫场最后一条记录落在哪。**
    // 写死 86400 时，末尾不在当天最后一步的强迫场会让 CoLM 在最后一段
    // 跑到一半时报 `Forcing does not cover simulation period!`
    // ——实测 AT-Neu 的末尾是 `2013 001 1800`。
    let forc_end_sec = e.hour * 3600 + e.minute * 60 + e.second;
    let end_sec = if end == (e.year, e.month, e.day) {
        forc_end_sec
    } else {
        86400
    };
    check_window(
        (start.0, start.1, start.2, start_sec),
        (end.0, end.1, end.2, end_sec),
        (
            summary.start.year,
            summary.start.month,
            summary.start.day,
            forc_start_sec,
        ),
        (e.year, e.month, e.day, forc_end_sec),
    )?;
    // 预热。默认重复第一年 1 遍 —— 陆面模式的土壤温湿与（开了 BGC 时的）
    // 碳库是慢变量，直接从初始场跑出来的头一段并不代表这个站点的气候态。
    //
    // **代价是输出少一年**：CoLM 的预热期是从窗口头上扣的，不是加在前面的
    // （`MOD_Hist.F90:235` 在预热期直接 RETURN）。所以周期只取一年，
    // 而不是常见的"重复整段"—— PLUMBER2 里最短的站点只有两年多。
    let spin_years = o.count("--spinup-years", 1)?;
    let spin_repeat = o.count("--spinup-repeat", 1)?;
    let mut spinup = Spinup {
        years: spin_years,
        repeat: spin_repeat,
    };
    // 预热周期盖过整个窗口时，输出会是空的 —— 而空输出与"跑失败了"在
    // 界面上长得一样。宁可不预热，也不能交出一个没有 history 的算例。
    if spinup.is_on() && (start.0 + spin_years as i32, start.1, start.2) >= end {
        eprintln!(
            "warning: spin-up would end at {}-{:02}-{:02}, at or past the window's end              {}-{:02}-{:02} — history is only written after spin-up, so this case would              produce nothing. Spin-up disabled; pass --spinup-years with a shorter period              to keep it.",
            start.0 + spin_years as i32,
            start.1,
            start.2,
            end.0,
            end.1,
            end.2
        );
        spinup = Spinup::OFF;
    }

    let loc = colm_srfdata::site::location(&layout.site_nc())?;
    let mode_landtype = new_mode
        .urban_landtype
        .or(colm_srfdata::site::landtype_for_mode(
            &layout.site_nc(),
            mode,
        )?);
    let name = o.get("--name").unwrap_or_else(|| {
        site_raw
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.split('_').next())
            .unwrap_or("case")
            .to_string()
    });

    // 全球栅格目录。自包含站点一个字节都不读它，因此未传 rawdata 时故意指向
    // 不存在的目录；需要外部地类、LAI/SAI 或土壤变量的站点已由上面的审计
    // 强制要求 `--rawdata`，这里必须原样保留该路径。
    //
    // **城市算例现在也一样。** 两张预抽表合起来盖住了 mksrfdata 会去开的
    // 每一处（`soil/` 的 24 个栅格实测 122 GB，加上 `urban_type/`、
    // `urban_lai_500m/`、`lake_depth.nc`、`soil_brightness.nc`、
    // `topography.nc`、`urban/LUCY_regionid.nc`）。两张很小的全局参数表随包发：
    // runtime 下的 LUCY（37 KB）和 rawdata 下的 NCAR 城市属性（62 KB）。
    //
    // 剩下的门槛只有一条：**站点在不在那 21 个里**。不在就照旧要 `--rawdata`，
    // 而且错误信息要说清楚是这个原因 —— 不是给没量过的站点编一个默认值。
    let dirs = if urban {
        let raw = match o.get("--rawdata") {
            // 给了就用。表外的站点靠它，表内的站点给了也无妨（site.nc
            // 里有的量 CoLM 不会再去开栅格）。
            Some(r) => slash(Path::new(&r)),
            // 两张站点表已经抽掉全球栅格；NCAR 的 33×3 城市属性表只有
            // 62 KB，直接铺进算例，LCZ/NCAR 两种分类都能重建地表数据。
            None if urban_covered => {
                if colm_srfdata::site::supports_ncar_urban(&layout.site_nc())? {
                    let root = out.join("rawdata");
                    let file = colm_srfdata::urban_runtime::stage_ncar(&root)?;
                    println!(
                        "  rawdata: {} written from the built-in copy",
                        file.display()
                    );
                    slash(&root)
                } else {
                    text(&out.join("rawdata_unused/"))
                }
            }
            None => bail!(
                "an urban case for a site outside the pre-extracted tables needs --rawdata: \
                 {name} is not one of the 21 Urban-PLUMBER sites, so CoLM will read the \
                 global soil/, urban_type/ and urban_lai_500m/ grids for it"
            ),
        };
        // `LUCY_rawdata.nc` 是张 37 KB 的全局区域参数表，与站点无关。
        // 即使用户给了 BGC/CROP runtime，里面也未必有 urban 子目录；缺时
        // 补上内置表，已有自定义表则原样保留。
        let requested = o.get("--runtime");
        let run = configured_or_unused(requested.clone(), &layout.runtime());
        let dir = Path::new(&run);
        if requested.is_none()
            || !dir
                .join(colm_srfdata::urban_runtime::LUCY_RELATIVE)
                .is_file()
        {
            let f = colm_srfdata::urban_runtime::stage(dir)?;
            println!("  runtime: {} written from the built-in copy", f.display());
        }
        let run = slash(dir);
        (raw, run)
    } else {
        let raw = configured_or_unused(o.get("--rawdata"), &out.join("rawdata_unused/"));
        let run = configured_or_unused(o.get("--runtime"), &out.join("runtime_unused/"));
        (raw, run)
    };

    let spec = CaseSpec {
        name: name.clone(),
        site_file: text(&layout.site_nc()),
        lon: loc.lon,
        lat: loc.lat,
        landtype: mode_landtype,
        window: Window {
            start_year: start.0,
            start_month: start.1,
            start_day: start.2,
            start_sec,
            end_year: end.0,
            end_month: end.1,
            end_day: end.2,
            end_sec,
        },
        timestep_seconds: summary.step_seconds,
        // 由文件说了算，不写死 —— PLUMBER2 是地方时，Urban-PLUMBER 是 UTC
        greenwich: summary.is_greenwich(),
        urban,
        spinup,
        dirs: Dirs {
            rawdata: dirs.0.clone(),
            runtime: dirs.1.clone(),
            output: format!("{}{}", text(&layout.out()), sep()),
            forcing_namelist: text(&layout.forcing_nml()),
        },
    };
    let mut all = fields(&spec);
    add_subgrid_fields(&mut all, new_mode.subgrid);
    if o.get("--crop").is_some() {
        add_crop_fields(&mut all);
    } else {
        add_inactive_process_fields(&mut all);
    }
    let req = required(&all);
    std::fs::write(layout.case_nml(), render(&req))?;
    println!(
        "case '{name}': {}-{:02}-{:02} to {}-{:02}-{:02}, {} s step, {} field(s) written ({} at CoLM's defaults)",
        start.0, start.1, start.2, end.0, end.1, end.2,
        summary.timestep_hint(),
        req.len(),
        all.len() - req.len()
    );
    Ok(out)
}

fn parse_date(s: &str) -> Result<(i32, u32, u32)> {
    let p: Vec<&str> = s.split('-').collect();
    if p.len() != 3 {
        bail!("date {s:?} is not YYYY-MM-DD");
    }
    Ok((p[0].parse()?, p[1].parse()?, p[2].parse()?))
}

/// `(年, 月, 日)` 写成 `YYYY-MM-DD`，只给报错用。
fn ymd((y, m, d): (i32, u32, u32)) -> String {
    format!("{y}-{m:02}-{d:02}")
}

fn ymds((y, m, d, sec): (i32, u32, u32, u32)) -> String {
    format!(
        "{} {:02}:{:02}:{:02}",
        ymd((y, m, d)),
        sec / 3600,
        sec / 60 % 60,
        sec % 60
    )
}

/// 要跑的窗口必须落在强迫场的覆盖范围之内。
///
/// **越界要当场说，不能让人等一次运行再看日志。** 越界时 CoLM 是跑到
/// 一半才报 `Forcing does not cover simulation period!` —— 那时候已经
/// 等了几分钟，而且那句话里看不出是哪个参数写错了。
///
/// 原先只校验 `--end`。起点没人管，于是 `--start` 早于强迫场就一路
/// 放行到 CoLM 里去了 —— 同一个理由，漏了一半。
fn check_window(
    start: (i32, u32, u32, u32),
    end: (i32, u32, u32, u32),
    forcing_start: (i32, u32, u32, u32),
    forcing_end: (i32, u32, u32, u32),
) -> Result<()> {
    // 这条与强迫场无关，纯粹是窗口本身不成立。不拦的话建出来的算例
    // 窗口是空的，而空输出与「跑失败了」在界面上长得一样。
    if start > end {
        bail!(
            "--start {} 晚于 --end {}：这个窗口是空的",
            ymds(start),
            ymds(end)
        );
    }
    if start < forcing_start {
        bail!(
            "--start {} 早于强迫场的起点（{} 起）",
            ymds(start),
            ymds(forcing_start)
        );
    }
    if end > forcing_end {
        bail!(
            "--end {} 超出强迫场的覆盖范围（到 {}）",
            ymds(end),
            ymds(forcing_end)
        );
    }
    Ok(())
}

fn text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// CoLM 把 `DEF_dir_rawdata` 与文件名直接拼接，中间不补分隔符
/// （`trim(DEF_dir_rawdata)//'urban/...'`），所以尾分隔符不是修饰，是必需的。
///
/// **Windows 上补的是反斜杠。** 这些路径会被原样交给 `cmd /c mkdir`
/// （CoLM 用 `CALL system` 建目录），而 cmd 把 `/` 当作开关前缀 ——
/// 一个以 `/` 结尾的路径会让它报 `The syntax of the command is incorrect.`。
/// 路径本身已经是反斜杠形式（`Path::display` 在 Windows 上如此），
/// 只有这个补上去的分隔符要跟着走。
fn sep() -> char {
    if cfg!(windows) {
        '\\'
    } else {
        '/'
    }
}

fn slash(p: &Path) -> String {
    let s = text(p);
    if s.ends_with(sep()) {
        s
    } else {
        format!("{s}{}", sep())
    }
}

fn configured_or_unused(configured: Option<String>, unused: &Path) -> String {
    configured
        .map(|path| {
            let path = Path::new(&path);
            slash(&if path.is_absolute() {
                path.to_path_buf()
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(path)
            })
        })
        .unwrap_or_else(|| text(unused))
}

fn add_crop_fields(fields: &mut Vec<(String, colm_namelist::Value)>) {
    fields.extend([
        ("DEF_USE_BGC".into(), colm_namelist::Value::Bool(true)),
        (
            "DEF_USE_LAIFEEDBACK".into(),
            colm_namelist::Value::Bool(true),
        ),
        ("DEF_USE_FERT".into(), colm_namelist::Value::Bool(false)),
        (
            "DEF_USE_CNSOYFIXN".into(),
            colm_namelist::Value::Bool(false),
        ),
        (
            "DEF_USE_IRRIGATION".into(),
            colm_namelist::Value::Bool(false),
        ),
        (
            "DEF_Aerosol_Readin".into(),
            colm_namelist::Value::Bool(false),
        ),
        (
            "DEF_TUNING_CROP_PLANTING_DAY".into(),
            colm_namelist::Value::Real { text: "120".into() },
        ),
    ]);
}

fn add_inactive_process_fields(fields: &mut Vec<(String, colm_namelist::Value)>) {
    fields.extend(
        [
            "DEF_USE_NITRIF",
            "DEF_USE_FERT",
            "DEF_USE_CNSOYFIXN",
            "DEF_Aerosol_Readin",
        ]
        .map(|name| (name.into(), colm_namelist::Value::Bool(false))),
    );
}

/// `colm-cli new --crop` writes BGC=true, NITRIF=true, while planting day,
/// fertilizer, and irrigation use file-free defaults. Validate exactly those
/// default runtime inputs before creating a case that can only fail in `colm`.
fn validate_default_crop_runtime(runtime: Option<&str>) -> Result<()> {
    let root = runtime
        .filter(|path| !path.trim().is_empty())
        .map(Path::new)
        .ok_or_else(|| anyhow::anyhow!("CROP 算例需要 --runtime 运行时数据目录"))?;
    let ndep = root
        .join("ndep")
        .join("fndep_colm_hist_simyr1849-2006_1.9x2.5_c100428.nc");
    if !ndep.is_file() {
        bail!("CROP 运行时目录缺少氮沉降数据：{}", ndep.display());
    }
    for family in ["CONC_O2_UNSAT", "O2_DECOMP_DEPTH_UNSAT"] {
        for layer in 1..=10 {
            let file = root
                .join("nitrif")
                .join(family)
                .join(format!("{family}_l{layer:02}.nc"));
            if !file.is_file() {
                bail!("CROP 运行时目录缺少硝化数据：{}", file.display());
            }
        }
    }
    Ok(())
}

fn add_subgrid_fields(fields: &mut Vec<(String, colm_namelist::Value)>, subgrid: Subgrid) {
    fields.extend([
        (
            "DEF_USE_LCT".into(),
            colm_namelist::Value::Bool(subgrid == Subgrid::Lct),
        ),
        (
            "DEF_USE_PFT".into(),
            colm_namelist::Value::Bool(subgrid == Subgrid::Pft),
        ),
        (
            "DEF_USE_PC".into(),
            colm_namelist::Value::Bool(subgrid == Subgrid::Pc),
        ),
    ]);
}

// ---------------------------------------------------------------- run

/// `stream` 打开时，子进程的每一行原样即时转发到本进程的 stdout。
///
/// 默认关着是有理由的：一次 528 步的运行，`colm.x` 打 5330 行，而人在终端
/// 想看到的是那 39 行摘要。但 GUI 那边正相反 —— 它的进度条靠
/// `TIMESTEP = n | DATE = ...`，日志窗要的就是原始行，而这两样都只在
/// 子进程的 stdout 里。同一个可执行文件同时服务两个诉求不同的调用方，
/// 于是由调用方说要哪一种。
fn requested_run_stage(value: Option<&str>) -> Result<Option<Stage>> {
    match value {
        None => Ok(None),
        Some("mksrfdata") => Ok(Some(Stage::MkSrfData)),
        Some("mkinidata") => Ok(Some(Stage::MkIniData)),
        Some("colm") => Ok(Some(Stage::Colm)),
        Some(other) => bail!("unknown run stage {other:?}; expected mksrfdata, mkinidata, or colm"),
    }
}

enum RunNotice<'a> {
    StageBegin(&'a str),
    StageSkipped(&'a str),
    Log { stage: &'a str, line: &'a str },
    StageDone { stage: &'a str, ok: bool },
}

fn cmd_run(
    case: &Path,
    kernel_dir: &Path,
    stream: bool,
    force: bool,
    only_stage: Option<Stage>,
) -> Result<()> {
    run_case(
        case,
        kernel_dir,
        stream,
        force,
        only_stage,
        false,
        &mut |_| {},
    )
}

fn run_case(
    case: &Path,
    kernel_dir: &Path,
    stream: bool,
    force: bool,
    only_stage: Option<Stage>,
    quiet: bool,
    notice: &mut dyn FnMut(RunNotice<'_>),
) -> Result<()> {
    // **绝对化算例目录。** `run_stage` 用 `current_dir(work)` 启动子进程，
    // 于是一个相对的 namelist 路径会被相对 `work` 解析而不是相对调用方的当前
    // 目录 —— `colm-cli run oracle/work/CN-Cng` 会让 CoLM 去
    // `oracle/work/CN-Cng/oracle/work/CN-Cng/case.nml` 找文件然后
    // `Cannot open file`。`Kernel::open` 早就为可执行文件做了同样的事
    // （见那里的注释），这一半当时漏了。
    // `absolute` 而不是 `canonicalize`：Windows 上后者返回 `\\?\C:\...`，
    // 而工作目录与 namelist 路径都要交给子进程 —— 那种形式两边都不认。
    let case = &colm_kernel::manifest::absolute(case)
        .with_context(|| format!("cannot resolve {}", case.display()))?;
    let kernel = Kernel::open(kernel_dir)?;
    if !quiet {
        println!(
            "kernel: {} ({})",
            kernel.manifest.identity(),
            kernel.manifest.platform
        );
    }
    let layout = Layout::new(case);
    let name = colm_case::case_name(&layout.case_nml())?;
    let out = layout.out().join(&name);
    let lc_year = land_cover_year(&layout.case_nml())?;
    // 产物必须列到**文件**：目录在程序写任何东西之前就已存在，
    // 只列目录的话「跑完了但什么都没写」恰好抓不到。
    let stages = stage_artifacts(&out, &name, lc_year);
    // 每段的输入指纹。**只看产物在不在是不够的** —— 改了站点文件或
    // rawdata 目录，srfdata.nc 就失效了而文件还在，跳过它等于拿旧地表数据
    // 算新算例，且没有任何迹象。见 `fingerprint.rs`。
    let kernel_id = kernel.manifest.stage_fingerprint_identity();
    let mut marks = fingerprint::load(case);
    if force {
        match only_stage {
            None => marks.clear(),
            Some(requested) => {
                // 显式重跑某一段后，它以及所有下游指纹都失效；上游仍然有效。
                // 例如只重建 mkinidata 不应让下一次“运行全部”白跑 mksrfdata，
                // 但旧 colm 输出不能继续被当作与新初始场一致。
                let first = stages
                    .iter()
                    .position(|(stage, _)| *stage == requested)
                    .expect("requested stage is one of the fixed stages");
                for (stage, _) in &stages[first..] {
                    marks.remove(stage.program());
                }
            }
        }
    }

    for (stage, artifacts) in &stages {
        if only_stage.is_some_and(|requested| requested != *stage) {
            continue;
        }
        let sname = stage.program();
        let (want, have_all, skip) = stage_fingerprint_status(
            *stage,
            artifacts,
            &layout.case_nml(),
            &out,
            &marks,
            &kernel_id,
        )?;
        if skip {
            if matches!(stage, Stage::Colm) {
                colm_case::clear_results_stale(case)
                    .with_context(|| format!("cannot mark {} results current", case.display()))?;
            }
            notice(RunNotice::StageSkipped(sname));
            if stream && !quiet {
                println!("=== colm-stage {sname} skipped ===");
            }
            if !quiet {
                println!("  {sname:<10} skipped (产物齐全且输入未变)");
            }
            continue;
        }
        // 说出**为什么**要重跑。「又跑了一遍」而不知道原因，
        // 会让人怀疑跳过功能根本没生效。
        if let Some(old) = marks.get(sname) {
            if have_all {
                if let Some(why) = fingerprint::first_difference(old, &want) {
                    if !quiet {
                        println!("  {sname:<10} 需要重跑：{why}");
                    }
                }
            }
        }
        // `colm` 要重跑时，先把上一次的 history 清掉。
        //
        // **不清的后果是一份混了两次配置的输出。** 实测：一个 2002-2012 的
        // 算例先无预热跑了一遍（132 个月度文件），改成开预热之后重跑 ——
        // 预热期不写 history，所以新的一遍只覆盖 2003-2012，而 2002 那 12 个
        // 文件原封不动留着。评估读到的是「2002 来自冷启动、2003 起来自预热」
        // 的拼接物，逐年偏差表上那道台阶看起来像模型行为，其实是两次运行的
        // 接缝。没有任何报错，两次运行都是成功的。
        //
        // 只删 `*_hist_*.nc`：restart 是 `mkinidata` 的产物，`colm` 要读它。
        if matches!(stage, Stage::Colm) {
            colm_case::mark_results_stale(case)
                .with_context(|| format!("cannot mark {} results stale", case.display()))?;
            let removed = clear_history(&out)?;
            if removed > 0 && !quiet {
                println!("  {sname:<10} 清掉上一次的 {removed} 个 history 文件");
            }
        }
        // 阶段标记。**由我们自己打，不去认 CoLM 的输出措辞** —— CoLM 把
        // automatically 拼成 automaticlly 这件事已经教过一次，上游随时会改。
        // 只有 `colm.x` 打 `TIMESTEP =`，所以没有这个标记，界面在前两段
        // 完全不知道进行到哪，进度条从 0 直接跳到 100。
        //
        // 前缀选得刻意难撞：CoLM 的输出里没有 `===` 开头的行（实测 34180 行
        // 一条都没有），而 `colm-stage` 这个词组也不出现在上游源码里。
        notice(RunNotice::StageBegin(sname));
        if stream && !quiet {
            println!("=== colm-stage {} begin ===", stage.program());
        }
        // 转发时**每行都 flush**。默认的行缓冲只在 stdout 连着终端时才生效；
        // GUI 拿到的是一根管道，那时缓冲变成块缓冲（8 KB），5330 行会攒成
        // 几大块一起吐出来 —— 从界面上看跟完全不转发几乎没有区别。
        let mut forward = |line: &str| {
            if stream {
                notice(RunNotice::Log { stage: sname, line });
            }
            if stream && !quiet {
                use std::io::Write as _;
                let mut o = std::io::stdout().lock();
                let _ = writeln!(o, "{line}");
                let _ = o.flush();
            }
        };
        let r = colm_kernel::run_stage_streaming(
            &kernel,
            *stage,
            &layout.case_nml(),
            case,
            artifacts,
            &mut forward,
        )?;
        notice(RunNotice::StageDone {
            stage: sname,
            ok: r.succeeded(),
        });
        if stream && !quiet {
            println!(
                "=== colm-stage {} {} ===",
                stage.program(),
                if r.succeeded() { "ok" } else { "failed" }
            );
        }
        if !quiet {
            if r.succeeded() {
                println!("  {:<10} ok", stage.program());
            } else {
                eprintln!("  {:<10} FAILED: {:?}", stage.program(), r.outcome);
            }
        }
        // CoLM 会不声不响地改掉你的配置然后继续跑 —— 失败时尤其要列，
        // 它恰恰会先改配置再死在别处。
        for o in &r.overrides {
            if !quiet {
                println!("             {}", o.text);
            }
        }
        if !r.succeeded() {
            if !quiet {
                eprintln!("  log: {}", r.log.display());
            }
            // 失败的那一段**不记指纹**，否则下次会把一个没跑成的阶段当成
            // 「已完成且输入未变」而跳过。
            marks.remove(sname);
            let _ = fingerprint::save(case, &marks);
            bail!("stage {} failed", stage.program());
        }
        marks.insert(sname.to_string(), want);
        fingerprint::save(case, &marks)?;
        if matches!(stage, Stage::Colm) {
            colm_case::clear_results_stale(case)
                .with_context(|| format!("cannot mark {} results current", case.display()))?;
        }
    }
    Ok(())
}

fn land_cover_year(case_nml: &Path) -> Result<i32> {
    let text = std::fs::read_to_string(case_nml)
        .with_context(|| format!("cannot read {}", case_nml.display()))?;
    let doc = colm_namelist::parse(&text)
        .with_context(|| format!("cannot parse {}", case_nml.display()))?;
    let year = match doc.get("DEF_LC_YEAR") {
        Some(colm_namelist::Value::Int(value)) => *value,
        Some(other) => bail!("DEF_LC_YEAR must be an integer, got {other}"),
        None => match colm_schema::find("DEF_LC_YEAR").map(|field| field.default) {
            Some(colm_schema::Default::Integer(value)) => value,
            _ => bail!("DEF_LC_YEAR default is missing from the generated schema"),
        },
    };
    if !(0..=9999).contains(&year) {
        bail!("DEF_LC_YEAR {year} cannot be formatted as a four-digit land-cover year");
    }
    Ok(year as i32)
}

fn stage_artifacts(out: &Path, name: &str, lc_year: i32) -> [(Stage, Vec<PathBuf>); 3] {
    let const_dir = out.join("restart/const");
    let lc = format!("lc{lc_year:04}");
    [
        (Stage::MkSrfData, vec![out.join("landdata/srfdata.nc")]),
        (
            Stage::MkIniData,
            vec![
                const_dir.join(format!("{name}_restart_const_{lc}_w180_s90.nc")),
                const_dir.join(format!("{name}_restart_const_{lc}.nc")),
            ],
        ),
        (Stage::Colm, vec![]),
    ]
}

fn stage_fingerprint_status(
    stage: Stage,
    artifacts: &[PathBuf],
    case_nml: &Path,
    out: &Path,
    marks: &std::collections::BTreeMap<String, fingerprint::Fingerprint>,
    kernel_id: &str,
) -> Result<(fingerprint::Fingerprint, bool, bool)> {
    let sname = stage.program();
    let want = fingerprint::compute(sname, case_nml, kernel_id)?;
    // 两个条件都要满足才跳过：指纹一致，**且**产物真的都在。
    let have_all = if stage == Stage::Colm {
        history_files(out).is_ok()
    } else {
        !artifacts.is_empty() && artifacts.iter().all(|path| path.is_file())
    };
    let current = have_all
        && marks
            .get(sname)
            .is_some_and(|old| fingerprint::first_difference(old, &want).is_none());
    Ok((want, have_all, current))
}

pub(crate) fn case_is_current(case: &Path, kernel_id: &str) -> Result<bool> {
    let layout = Layout::new(case);
    let name = colm_case::case_name(&layout.case_nml())?;
    let out = layout.out().join(&name);
    let lc_year = land_cover_year(&layout.case_nml())?;
    let marks = fingerprint::load(case);
    for (stage, artifacts) in stage_artifacts(&out, &name, lc_year) {
        if !stage_fingerprint_status(
            stage,
            &artifacts,
            &layout.case_nml(),
            &out,
            &marks,
            kernel_id,
        )?
        .2
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn cmd_study_run(
    study_dir: &Path,
    kernel_dir: &Path,
    stream: bool,
    jobs: usize,
    retry_failed: bool,
) -> Result<()> {
    let state = study::runner::run(
        study_dir,
        study::runner::RunOptions {
            kernel_dir,
            jobs,
            stream,
            retry_failed,
        },
    )?;
    if !stream {
        println!("{}", serde_json::to_string(&state)?);
    }
    Ok(())
}

fn cmd_study_retry(study_dir: &Path, include_review: bool) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string(&study::runner::retry(study_dir, include_review)?)?
    );
    Ok(())
}

fn cmd_study_apply(study_dir: &Path, member: &str, out: &Path, name: Option<String>) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string(&study::runner::apply(
            study_dir,
            member,
            out,
            name.as_deref()
        )?)?
    );
    Ok(())
}

fn cmd_study_apply_preview(study_dir: &Path, member: &str) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string(&study::runner::apply_preview(study_dir, member)?)?
    );
    Ok(())
}

// ---------------------------------------------------------------- metrics

/// 一个通量变量的评估结果，交给 GUI。
///
/// **连配对点一起给。** 界面要画三样东西：指标表、模型与观测的双线图、
/// 散点图，而它们用的是同一批配对结果。分三次跑等于把同一份 NetCDF
/// 读三遍，而且三者可能因为参数不一致而对不上。
#[derive(serde::Serialize)]
struct VarMetrics {
    /// 观测量的名字（`Qh` / `Qle` / …）
    name: String,
    /// 实际读的那个观测变量。**与 `name` 可以不同** —— 开了闭合订正时
    /// 读的是 `Qle_cor`。表里要写出来：拿哪一版观测比，决定了偏差的含义。
    obs_var: String,
    /// 模型 history 里的对应变量
    model_var: String,
    label_zh: String,
    label_en: String,
    units: String,
    /// `measured_only` 表示使用 qc==0；`finite_only` 表示源文件没有 QC。
    quality_control: String,
    n: usize,
    rmse: f64,
    mae: f64,
    bias: f64,
    r2: f64,
    correlation: f64,
    nse: f64,
    kge: f64,
    model_mean: f64,
    model_sd: f64,
    obs_mean: f64,
    obs_sd: f64,
    alpha: f64,
    beta: f64,
    /// KGE 的 β 不可信时的原因。**非空一定要显示** ——
    /// 藏起来等于给一个假指标。
    beta_warning: Option<String>,
    /// 配对之后的时刻（unix 秒），与下面两条等长
    #[serde(skip_serializing_if = "Option::is_none")]
    time: Option<Vec<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<Vec<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    obs: Option<Vec<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pair_source_n: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pair_n: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pair_downsampled: Option<bool>,
}

struct MetricsRequest<'a> {
    case: &'a Path,
    obs_path: &'a Path,
    spinup: usize,
    json: bool,
    corrected: bool,
    summary_only: bool,
    pair_vars: Vec<String>,
    pair_max_points: Option<usize>,
    from: Option<i64>,
    to: Option<i64>,
}

#[derive(Debug, serde::Serialize)]
struct EvaluationAvailability {
    name: String,
    label_zh: String,
    label_en: String,
    units: String,
    model_var: String,
    obs_var: String,
    qc_var: Option<String>,
    quality_control: String,
    available: bool,
    missing_model: Vec<String>,
    missing_observation: Vec<String>,
}

fn evaluation_availability(
    variable: &colm_hist::obs::EvaluationVariable,
    hists: &[PathBuf],
    observation: &netcdf::File,
) -> EvaluationAvailability {
    let resolved = resolve_model_source(variable.model, hists);
    let missing_model = resolved.map(|_| Vec::new()).unwrap_or_else(|| {
        variable
            .model
            .required()
            .into_iter()
            .filter(|name| !history_has_variables(hists, &[*name]))
            .map(str::to_string)
            .collect()
    });
    evaluation_availability_row(variable, resolved, missing_model, observation)
}

fn evaluation_availability_row(
    variable: &colm_hist::obs::EvaluationVariable,
    resolved: Option<colm_hist::obs::ModelSource>,
    missing_model: Vec<String>,
    observation: &netcdf::File,
) -> EvaluationAvailability {
    let source = observation_source(variable, observation);
    let missing_observation = source
        .as_ref()
        .map(|_| Vec::new())
        .unwrap_or_else(|| missing_observation_sources(variable, observation));
    EvaluationAvailability {
        name: variable.observation.to_string(),
        label_zh: variable.label_zh.to_string(),
        label_en: variable.label_en.to_string(),
        units: variable.units.to_string(),
        model_var: resolved.unwrap_or(variable.model).label(),
        obs_var: source
            .as_ref()
            .map(|source| (*source).label().to_string())
            .unwrap_or_else(|| variable.observation.to_string()),
        qc_var: source
            .as_ref()
            .and_then(|source| (*source).qc_label().map(str::to_string))
            .or_else(|| variable.qc.map(str::to_string)),
        quality_control: if source
            .as_ref()
            .map(|source| (*source).measured_qc())
            .unwrap_or(variable.qc.is_some())
        {
            "measured_only"
        } else {
            "finite_only"
        }
        .to_string(),
        available: missing_model.is_empty() && source.is_some(),
        missing_model,
        missing_observation,
    }
}

#[derive(Debug, Clone, Copy)]
enum ObservationSource {
    Direct {
        label: &'static str,
        qc_label: Option<&'static str>,
        measured_qc: bool,
    },
    DerivedUrbanRnet {
        label: &'static str,
        qc_label: Option<&'static str>,
        measured_qc: bool,
    },
}

impl ObservationSource {
    fn label(self) -> &'static str {
        match self {
            Self::Direct { label, .. } | Self::DerivedUrbanRnet { label, .. } => label,
        }
    }

    fn qc_label(self) -> Option<&'static str> {
        match self {
            Self::Direct { qc_label, .. } | Self::DerivedUrbanRnet { qc_label, .. } => qc_label,
        }
    }

    fn measured_qc(self) -> bool {
        match self {
            Self::Direct { measured_qc, .. } | Self::DerivedUrbanRnet { measured_qc, .. } => {
                measured_qc
            }
        }
    }
}

fn observation_source(
    variable: &colm_hist::obs::EvaluationVariable,
    observation: &netcdf::File,
) -> Option<ObservationSource> {
    if observation.variable(variable.observation).is_some()
        && variable
            .qc
            .is_none_or(|qc| observation.variable(qc).is_some())
    {
        return Some(ObservationSource::Direct {
            label: variable.observation,
            qc_label: variable.qc,
            measured_qc: variable.qc.is_some(),
        });
    }
    let components = colm_hist::obs::derived_observation_components(variable.observation)?;
    let qcs = colm_hist::obs::derived_observation_qc(variable.observation)?;
    if components
        .iter()
        .chain(qcs.iter())
        .all(|name| observation.variable(name).is_some())
    {
        Some(ObservationSource::DerivedUrbanRnet {
            label: colm_hist::obs::derived_observation_label(variable.observation)
                .unwrap_or(variable.observation),
            qc_label: Some("component_qc"),
            measured_qc: true,
        })
    } else {
        None
    }
}

fn missing_observation_sources(
    variable: &colm_hist::obs::EvaluationVariable,
    observation: &netcdf::File,
) -> Vec<String> {
    let mut missing = Vec::new();
    if observation.variable(variable.observation).is_none() {
        missing.push(variable.observation.to_string());
    }
    if let Some(qc) = variable.qc.filter(|qc| observation.variable(qc).is_none()) {
        missing.push(qc.to_string());
    }
    if let (Some(components), Some(qcs)) = (
        colm_hist::obs::derived_observation_components(variable.observation),
        colm_hist::obs::derived_observation_qc(variable.observation),
    ) {
        missing.extend(
            components
                .into_iter()
                .chain(qcs)
                .filter(|name| observation.variable(name).is_none())
                .map(str::to_string),
        );
        missing.sort();
        missing.dedup();
    }
    missing
}

fn resolve_model_source(
    source: colm_hist::obs::ModelSource,
    hists: &[PathBuf],
) -> Option<colm_hist::obs::ModelSource> {
    use colm_hist::obs::ModelSource;
    match source {
        ModelSource::Alternative {
            preferred,
            fallback,
        } => resolve_model_source(*preferred, hists)
            .or_else(|| resolve_model_source(*fallback, hists)),
        other => other
            .required_alternatives()
            .into_iter()
            .any(|required| history_has_variables(hists, &required))
            .then_some(other),
    }
}

fn evaluation_plan_availability(
    variable: &colm_hist::obs::EvaluationVariable,
    model_available: &dyn Fn(&str) -> bool,
    observation: &netcdf::File,
) -> EvaluationAvailability {
    let resolved = resolve_model_source_planned(variable.model, model_available);
    let missing_model = resolved.map(|_| Vec::new()).unwrap_or_else(|| {
        variable
            .model
            .required()
            .into_iter()
            .filter(|name| !model_available(name))
            .map(str::to_string)
            .collect()
    });
    evaluation_availability_row(variable, resolved, missing_model, observation)
}

fn resolve_model_source_planned(
    source: colm_hist::obs::ModelSource,
    model_available: &dyn Fn(&str) -> bool,
) -> Option<colm_hist::obs::ModelSource> {
    use colm_hist::obs::ModelSource;
    match source {
        ModelSource::Alternative {
            preferred,
            fallback,
        } => resolve_model_source_planned(*preferred, model_available)
            .or_else(|| resolve_model_source_planned(*fallback, model_available)),
        other => other
            .required_alternatives()
            .into_iter()
            .any(|required| required.iter().all(|name| model_available(name)))
            .then_some(other),
    }
}

fn planned_history_variables(
    case_nml: &Path,
    kernel_dir: &Path,
) -> Result<std::collections::BTreeSet<String>> {
    let kernel = Kernel::open(kernel_dir)?;
    let macros = kernel
        .manifest
        .macros
        .iter()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    let text = std::fs::read_to_string(case_nml)
        .with_context(|| format!("cannot read {}", case_nml.display()))?;
    let doc = colm_namelist::parse(&text)
        .with_context(|| format!("cannot parse {}", case_nml.display()))?;
    let truth = |path: &str| -> bool {
        match doc.get(path) {
            Some(colm_namelist::Value::Bool(value)) => *value,
            _ => matches!(
                colm_schema::find(path).map(|field| field.default),
                Some(colm_schema::Default::Logical(true))
            ),
        }
    };
    let gate_truth = |path: &str| -> Option<bool> { colm_schema::find(path).map(|_| truth(path)) };
    Ok(colm_hist::all()
        .iter()
        .filter(|var| var.macros.iter().all(|cond| cond.holds(&macros)))
        .filter(|var| {
            var.runtime
                .is_none_or(|expr| colm_hist::eval_runtime_gate(expr, &gate_truth) == Some(true))
        })
        .filter(|var| {
            let switch = format!("DEF_hist_vars%{}", var.name);
            colm_schema::find(&switch).is_none_or(|_| truth(&switch))
        })
        .map(|var| format!("f_{}", var.name))
        .collect())
}

fn cmd_evaluation_catalog(case: &Path, obs_path: &Path) -> Result<()> {
    let layout = Layout::new(case);
    let name = colm_case::case_name(&layout.case_nml())?;
    let hists = history_files(&layout.out().join(&name))?;
    let observation =
        netcdf::open(obs_path).with_context(|| format!("cannot open {}", obs_path.display()))?;
    let rows = colm_hist::obs::EVALUATION_VARIABLES
        .iter()
        .map(|variable| evaluation_availability(variable, &hists, &observation))
        .collect::<Vec<_>>();
    println!("{}", serde_json::to_string(&rows)?);
    Ok(())
}

fn evaluation_plan_rows(
    case_nml: &Path,
    obs_path: &Path,
    kernel_dir: &Path,
) -> Result<Vec<EvaluationAvailability>> {
    let planned = planned_history_variables(case_nml, kernel_dir)?;
    let observation =
        netcdf::open(obs_path).with_context(|| format!("cannot open {}", obs_path.display()))?;
    Ok(colm_hist::obs::EVALUATION_VARIABLES
        .iter()
        .map(|variable| {
            evaluation_plan_availability(variable, &|name| planned.contains(name), &observation)
        })
        .collect())
}

fn cmd_evaluation_plan(case: &Path, obs_path: &Path, kernel_dir: &Path) -> Result<()> {
    let rows = evaluation_plan_rows(&Layout::new(case).case_nml(), obs_path, kernel_dir)?;
    println!("{}", serde_json::to_string(&rows)?);
    Ok(())
}

fn model_values(
    source: colm_hist::obs::ModelSource,
    data: &std::collections::BTreeMap<String, Vec<f64>>,
) -> Option<Vec<f64>> {
    use colm_hist::obs::ModelSource;
    match source {
        ModelSource::Direct { variable, scale } => Some(
            data.get(variable)?
                .iter()
                .map(|value| value * scale)
                .collect(),
        ),
        ModelSource::Difference {
            minuend,
            subtrahend,
            scale,
        } => {
            let (a, b) = (data.get(minuend)?, data.get(subtrahend)?);
            (a.len() == b.len()).then(|| a.iter().zip(b).map(|(a, b)| (a - b) * scale).collect())
        }
        ModelSource::Sum { variables, scale } => {
            let first = data.get(*variables.first()?)?;
            let mut out = vec![0.0; first.len()];
            for variable in variables {
                let values = data.get(*variable)?;
                if values.len() != out.len() {
                    return None;
                }
                for (sum, value) in out.iter_mut().zip(values) {
                    *sum += value;
                }
            }
            Some(out.into_iter().map(|value| value * scale).collect())
        }
        ModelSource::SumDifference {
            positive,
            negative,
            scale,
        } => {
            let first = positive.first().or_else(|| negative.first())?;
            let mut out = vec![0.0; data.get(*first)?.len()];
            for variable in positive {
                let values = data.get(*variable)?;
                if values.len() != out.len() {
                    return None;
                }
                for (sum, value) in out.iter_mut().zip(values) {
                    *sum += value;
                }
            }
            for variable in negative {
                let values = data.get(*variable)?;
                if values.len() != out.len() {
                    return None;
                }
                for (sum, value) in out.iter_mut().zip(values) {
                    *sum -= value;
                }
            }
            Some(out.into_iter().map(|value| value * scale).collect())
        }
        ModelSource::Alternative {
            preferred,
            fallback,
        } => {
            let preferred_present = preferred
                .required()
                .into_iter()
                .all(|name| data.contains_key(name));
            if preferred_present {
                model_values(*preferred, data)
            } else {
                model_values(*fallback, data)
            }
        }
    }
}

struct ObservationData {
    label: String,
    values: Vec<f64>,
    qc: Vec<f64>,
}

fn observation_values(
    obs_file: &netcdf::File,
    obs_path: &Path,
    variable: &colm_hist::obs::EvaluationVariable,
    corrected: bool,
) -> Result<Option<ObservationData>> {
    let o_name = variable.observation;
    let direct = corrected
        .then(|| colm_hist::obs::corrected(o_name))
        .flatten()
        .filter(|candidate| obs_file.variable(candidate).is_some())
        .unwrap_or(o_name);
    if obs_file.variable(direct).is_some()
        && variable.qc.is_none_or(|qc| obs_file.variable(qc).is_some())
    {
        let values = read_file_1d(obs_file, obs_path, direct)?;
        let qc = match variable.qc {
            Some(qc) => read_file_1d(obs_file, obs_path, qc)?,
            None => vec![colm_hist::pair::QC_MEASURED; values.len()],
        };
        return Ok(Some(ObservationData {
            label: direct.to_string(),
            values,
            qc,
        }));
    }
    let Some(components) = colm_hist::obs::derived_observation_components(o_name) else {
        return Ok(None);
    };
    let Some(qc_names) = colm_hist::obs::derived_observation_qc(o_name) else {
        return Ok(None);
    };
    if !components
        .iter()
        .chain(qc_names.iter())
        .all(|name| obs_file.variable(name).is_some())
    {
        return Ok(None);
    }
    let swdown = read_file_1d(obs_file, obs_path, components[0])?;
    let lwdown = read_file_1d(obs_file, obs_path, components[1])?;
    let swup = read_file_1d(obs_file, obs_path, components[2])?;
    let lwup = read_file_1d(obs_file, obs_path, components[3])?;
    let swdown_qc = read_file_1d(obs_file, obs_path, qc_names[0])?;
    let lwdown_qc = read_file_1d(obs_file, obs_path, qc_names[1])?;
    let swup_qc = read_file_1d(obs_file, obs_path, qc_names[2])?;
    let lwup_qc = read_file_1d(obs_file, obs_path, qc_names[3])?;
    let (values, qc) = colm_hist::obs::derive_urban_rnet(
        [&swdown, &lwdown, &swup, &lwup],
        [&swdown_qc, &lwdown_qc, &swup_qc, &lwup_qc],
    )?;
    Ok(Some(ObservationData {
        label: colm_hist::obs::derived_observation_label(o_name)
            .unwrap_or(o_name)
            .to_string(),
        values,
        qc,
    }))
}

fn normalized_metric_window(
    model_minutes: &[f64],
    normalized_seconds: &[f64],
    from: Option<i64>,
    to: Option<i64>,
) -> Result<Option<colm_hist::pair::TimeWindow>> {
    if from.is_none() && to.is_none() {
        return Ok(None);
    }
    if from.zip(to).is_some_and(|(from, to)| from >= to) {
        bail!("metric --from must be earlier than --to");
    }
    let unix = colm_hist::time::unix_seconds(model_minutes);
    let offset = unix
        .first()
        .zip(normalized_seconds.first())
        .map(|(unix, normalized)| *unix as f64 - *normalized)
        .unwrap_or(0.0);
    Ok(Some(colm_hist::pair::TimeWindow {
        from: from.map_or(f64::NEG_INFINITY, |value| value as f64 - offset),
        to: to.map_or(f64::INFINITY, |value| value as f64 - offset),
    }))
}

fn compute_metric_rows(request: MetricsRequest<'_>) -> Result<Vec<VarMetrics>> {
    let MetricsRequest {
        case,
        obs_path,
        spinup,
        json,
        corrected,
        summary_only,
        pair_vars,
        pair_max_points,
        from,
        to,
    } = request;
    validate_pair_vars(&pair_vars)?;
    let layout = Layout::new(case);
    let name = colm_case::case_name(&layout.case_nml())?;
    let hists = history_files(&layout.out().join(&name))?;

    // Open every NetCDF file once. The previous implementation reopened every one
    // of 132 monthly history files for each flux, turning an AT-Neu evaluation into
    // minutes of native-I/O setup rather than numerical work.
    let obs_file =
        netcdf::open(obs_path).with_context(|| format!("cannot open {}", obs_path.display()))?;
    let o_t = read_file_1d(&obs_file, obs_path, "time")?;
    let selected = colm_hist::obs::EVALUATION_VARIABLES
        .iter()
        .filter(|variable| {
            pair_vars.is_empty()
                || pair_vars
                    .iter()
                    .any(|wanted| wanted == variable.observation)
        })
        .filter_map(|variable| {
            let available = evaluation_availability(variable, &hists, &obs_file);
            available.available.then(|| {
                (
                    *variable,
                    resolve_model_source(variable.model, &hists).unwrap(),
                )
            })
        })
        .collect::<Vec<_>>();
    let mut wanted = std::collections::BTreeSet::from(["time"]);
    for (_, source) in &selected {
        wanted.extend(source.required());
    }
    let wanted = wanted.into_iter().collect::<Vec<_>>();
    let mut model_data = read_history_many(&hists, &wanted)?;
    let m_t = model_data.remove("time").expect("time was requested");
    // 观测的 time 原点可能在年中（AU-Preston 是 2003-08-12 03:30），
    // 必须按完整日期时间换算；只取年份会把序列错配几个月。
    let units = variable_units(&obs_file, "time")
        .with_context(|| format!("time:units in {} is not a string", obs_path.display()))?;
    let m_sec = colm_hist::time::model_seconds_from_units(&m_t, &units)
        .with_context(|| format!("unsupported observation time units {units:?}"))?;
    let unix = colm_hist::time::unix_seconds(&m_t);
    let metric_window = normalized_metric_window(&m_t, &m_sec, from, to)?;
    let by_sec = if json && !summary_only {
        Some(
            m_sec
                .iter()
                .zip(unix)
                .map(|(seconds, unix)| (*seconds as i64, unix))
                .collect::<std::collections::BTreeMap<_, _>>(),
        )
    } else {
        None
    };

    if !json {
        println!(
            "{:<10} {:>7} {:>8} {:>9} {:>7} {:>9}",
            "obs var", "n", "RMSE", "bias", "R2", "KGE"
        );
    }
    let mut rows: Vec<VarMetrics> = Vec::new();
    for (variable, source) in selected {
        let o_name = variable.observation;
        let Ok(Some(obs_data)) = observation_values(&obs_file, obs_path, &variable, corrected)
        else {
            continue;
        };
        let m_v = model_values(source, &model_data).ok_or_else(|| {
            anyhow::anyhow!(
                "model history source {} is incomplete or has inconsistent lengths",
                source.label()
            )
        })?;
        let s = colm_hist::pair::Series {
            seconds: &o_t,
            values: &obs_data.values,
            qc: &obs_data.qc,
        };
        let with_time =
            colm_hist::pair::pair_with_time_in_window(&m_sec, &m_v, &s, spinup, metric_window);
        let pairs: Vec<(f64, f64)> = with_time.iter().map(|(_, a, b)| (*a, *b)).collect();
        let Some(m) = colm_hist::metric::compute(&pairs) else {
            continue;
        };
        if json {
            let pair_source_n = with_time.len();
            let pair_time = by_sec.as_ref().map(|by_sec| {
                with_time
                    .iter()
                    .map(|(seconds, _, _)| {
                        by_sec.get(&(*seconds as i64)).copied().unwrap_or_default()
                    })
                    .collect::<Vec<_>>()
            });
            let pair_model = (!summary_only).then(|| {
                with_time
                    .iter()
                    .map(|(_, value, _)| *value)
                    .collect::<Vec<_>>()
            });
            let pair_obs = (!summary_only).then(|| {
                with_time
                    .iter()
                    .map(|(_, _, value)| *value)
                    .collect::<Vec<_>>()
            });
            let pair_indexes = match (&pair_time, &pair_model, &pair_obs) {
                (Some(time), Some(model), Some(obs)) => Some(series_indices_multi(
                    time,
                    &[model.as_slice(), obs.as_slice()],
                    None,
                    None,
                    pair_max_points,
                )),
                _ => None,
            };
            let select_i64 = |values: Option<Vec<i64>>| {
                values.map(|values| {
                    pair_indexes
                        .as_ref()
                        .expect("pair indexes exist with pair values")
                        .iter()
                        .map(|&index| values[index])
                        .collect::<Vec<_>>()
                })
            };
            let select_f64 = |values: Option<Vec<f64>>| {
                values.map(|values| {
                    pair_indexes
                        .as_ref()
                        .expect("pair indexes exist with pair values")
                        .iter()
                        .map(|&index| values[index])
                        .collect::<Vec<_>>()
                })
            };
            rows.push(VarMetrics {
                name: o_name.to_string(),
                obs_var: obs_data.label.clone(),
                model_var: source.label(),
                label_zh: variable.label_zh.to_string(),
                label_en: variable.label_en.to_string(),
                units: variable.units.to_string(),
                quality_control: if variable.qc.is_some()
                    || colm_hist::obs::derived_observation_qc(o_name).is_some()
                {
                    "measured_only"
                } else {
                    "finite_only"
                }
                .to_string(),
                n: m.n,
                rmse: m.rmse,
                mae: m.mae,
                bias: m.bias,
                r2: m.r2,
                correlation: m.correlation,
                nse: m.nse,
                kge: m.kge,
                model_mean: m.model_mean,
                model_sd: m.model_sd,
                obs_mean: m.obs_mean,
                obs_sd: m.obs_sd,
                alpha: m.alpha,
                beta: m.beta,
                beta_warning: match m.beta_warning {
                    Some(colm_hist::metric::BetaWarning::NearZeroMean) => {
                        Some(format!("KGE 不可信：观测均值 {:.1} 接近零", m.obs_mean))
                    }
                    Some(colm_hist::metric::BetaWarning::OppositeSign) => {
                        Some("KGE 不可信：模型与观测的均值反号".to_string())
                    }
                    None => None,
                },
                // 指标始终来自完整配对；图表数据单独保极值降采样。
                time: select_i64(pair_time),
                model: select_f64(pair_model),
                obs: select_f64(pair_obs),
                pair_source_n: (!summary_only).then_some(pair_source_n),
                pair_n: pair_indexes.as_ref().map(Vec::len),
                pair_downsampled: pair_indexes
                    .as_ref()
                    .map(|indexes| indexes.len() < pair_source_n),
            });
            continue;
        }
        print!(
            "{:<10} {:>7} {:>8.1} {:>+9.2} {:>7.3} {:>+9.3}",
            obs_data.label, m.n, m.rmse, m.bias, m.r2, m.kge
        );
        // KGE 的 β 在观测均值接近零或与模型均值反号时没有意义。
        // 只标记，不改值 —— 改了就与 design.md 的参考表对不上。
        match m.beta_warning {
            Some(colm_hist::metric::BetaWarning::NearZeroMean) => {
                println!(
                    "   KGE unreliable: observed mean {:.1} is near zero",
                    m.obs_mean
                )
            }
            Some(colm_hist::metric::BetaWarning::OppositeSign) => {
                println!("   KGE unreliable: model and observed means have opposite signs")
            }
            None => println!(),
        }
    }
    Ok(rows)
}

fn validate_pair_vars(pair_vars: &[String]) -> Result<()> {
    let unknown = pair_vars
        .iter()
        .filter(|wanted| {
            !colm_hist::obs::EVALUATION_VARIABLES
                .iter()
                .any(|variable| variable.observation == wanted.as_str())
        })
        .cloned()
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        let known = colm_hist::obs::EVALUATION_VARIABLES
            .iter()
            .map(|variable| variable.observation)
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "unknown --pairs-var value(s): {}; choose from {known}",
            unknown.join(", ")
        );
    }
    Ok(())
}

pub(crate) fn observation_usable_count(
    obs_path: &Path,
    variable_name: &str,
    from: i64,
    to: i64,
) -> Result<usize> {
    if from >= to {
        bail!("observation window is empty");
    }
    validate_pair_vars(&[variable_name.to_string()])?;
    let obs_file =
        netcdf::open(obs_path).with_context(|| format!("cannot open {}", obs_path.display()))?;
    let variable = colm_hist::obs::EVALUATION_VARIABLES
        .iter()
        .find(|variable| variable.observation.eq_ignore_ascii_case(variable_name))
        .with_context(|| format!("unknown evaluation target {variable_name}"))?;
    let Some(obs_data) = observation_values(&obs_file, obs_path, variable, false)? else {
        bail!(
            "observation target {variable_name} is unavailable in {}",
            obs_path.display()
        );
    };
    let obs_time = read_file_1d(&obs_file, obs_path, "time")?;
    if obs_time.len() != obs_data.values.len() || obs_data.qc.len() != obs_data.values.len() {
        bail!("observation target {variable_name} has inconsistent time/value/QC lengths");
    }
    let units = variable_units(&obs_file, "time")
        .with_context(|| format!("time:units in {} is not a string", obs_path.display()))?;
    let epoch_minutes = colm_hist::time::minutes_from_1900(1970) as f64;
    let epoch_in_obs_seconds = colm_hist::time::model_seconds_from_units(&[epoch_minutes], &units)
        .and_then(|values| values.into_iter().next())
        .with_context(|| format!("unsupported observation time units {units:?}"))?;
    let origin_unix = -epoch_in_obs_seconds;
    Ok(obs_time
        .iter()
        .zip(obs_data.values.iter().zip(&obs_data.qc))
        .filter(|(seconds, (value, qc))| {
            let unix = origin_unix + **seconds;
            unix >= from as f64
                && unix < to as f64
                && **qc == colm_hist::pair::QC_MEASURED
                && value.is_finite()
                && **value > colm_hist::pair::FILL_VALUE + 1.0
        })
        .count())
}

fn cmd_metrics(request: MetricsRequest<'_>) -> Result<()> {
    let json = request.json;
    let rows = compute_metric_rows(request)?;
    if json {
        println!("{}", serde_json::to_string(&rows)?);
    }
    Ok(())
}

// ---------------------------------------------------------------- series

#[derive(serde::Serialize)]
struct HistoryCatalog {
    files: usize,
    steps: usize,
    start: i64,
    end: i64,
    variables: Vec<HistoryVariable>,
}

#[derive(serde::Serialize)]
struct HistoryVariable {
    name: String,
    units: Option<String>,
    dimensions: Vec<DimensionShape>,
    kind: &'static str,
}

fn history_kind(dimensions: &[(String, usize)]) -> &'static str {
    if dimensions.is_empty() {
        return "scalar";
    }
    let names: Vec<String> = dimensions
        .iter()
        .map(|(name, _)| name.to_ascii_lowercase())
        .collect();
    let non_time = dimensions
        .iter()
        .filter(|(name, _)| !name.eq_ignore_ascii_case("time"))
        .map(|(_, len)| *len)
        .product::<usize>();
    if non_time <= 1 {
        "series"
    } else if names.iter().any(|name| {
        name.contains("soil")
            || name.contains("snow")
            || name.contains("lake")
            || name.contains("lev")
            || name.contains("depth")
    }) {
        "profile"
    } else {
        "category"
    }
}

fn history_variables(files: &[PathBuf]) -> Result<Vec<HistoryVariable>> {
    let mut variables = std::collections::BTreeMap::new();
    for stream in history_streams(files) {
        let path = stream.first().expect("empty history streams were removed");
        let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        for variable in file.variables() {
            let name = variable.name();
            if variables.contains_key(&name) {
                continue;
            }
            let dimensions: Vec<(String, usize)> = variable
                .dimensions()
                .iter()
                .map(|dimension| (dimension.name(), dimension.len()))
                .collect();
            variables.insert(
                name.clone(),
                HistoryVariable {
                    units: variable_units(&file, &name),
                    name,
                    kind: history_kind(&dimensions),
                    dimensions: dimensions
                        .into_iter()
                        .map(|(name, len)| DimensionShape { name, len })
                        .collect(),
                },
            );
        }
    }
    Ok(variables.into_values().collect())
}

fn ensure_usable_history_catalog(time: &[f64], variables: &[HistoryVariable]) -> Result<()> {
    if time.is_empty() {
        bail!("history files are incomplete: no time steps");
    }
    if !variables.iter().any(|variable| {
        variable.name != "time"
            && variable
                .dimensions
                .iter()
                .any(|dimension| dimension.name.eq_ignore_ascii_case("time"))
    }) {
        bail!("history files are incomplete: no analyzable variables");
    }
    Ok(())
}

fn cmd_history_catalog(case: &Path) -> Result<()> {
    let layout = Layout::new(case);
    let name = colm_case::case_name(&layout.case_nml())?;
    let hists = history_files(&layout.out().join(&name))?;
    let time = read_history(&hists, "time")?;
    let unix = colm_hist::time::unix_seconds(&time);
    let variables = history_variables(&hists)?;
    ensure_usable_history_catalog(&time, &variables)?;
    let catalog = HistoryCatalog {
        files: hists.len(),
        steps: unix.len(),
        start: unix.first().copied().unwrap_or_default(),
        end: unix.last().copied().unwrap_or_default(),
        variables,
    };
    println!("{}", serde_json::to_string(&catalog)?);
    Ok(())
}

#[derive(serde::Serialize)]
struct SeriesBody {
    files: usize,
    source_n: usize,
    n: usize,
    downsampled: bool,
    time: Vec<i64>,
    vars: std::collections::BTreeMap<String, Vec<Option<f64>>>,
}

#[cfg(test)]
fn series_indices(
    unix: &[i64],
    values: &[f64],
    from: Option<i64>,
    to: Option<i64>,
    max_points: Option<usize>,
) -> Vec<usize> {
    series_indices_multi(unix, &[values], from, to, max_points)
}

fn series_indices_multi(
    unix: &[i64],
    values: &[&[f64]],
    from: Option<i64>,
    to: Option<i64>,
    max_points: Option<usize>,
) -> Vec<usize> {
    let candidates: Vec<usize> = unix
        .iter()
        .enumerate()
        .filter(|(_, time)| from.is_none_or(|start| **time >= start))
        .filter(|(_, time)| to.is_none_or(|end| **time <= end))
        .map(|(index, _)| index)
        .collect();
    let Some(limit) = max_points.map(|value| value.max(3)) else {
        return candidates;
    };
    if candidates.len() <= limit {
        return candidates;
    }
    // Reserve min/max slots for every requested variable in each bucket. Most GUI
    // requests contain one series; diagnostics request several, and choosing extrema
    // from only the first series can hide a spike in the remaining fluxes.
    let per_bucket = (2 * values.len().max(1)).min(limit - 2).max(1);
    let buckets = ((limit - 2) / per_bucket).max(1);
    let interior = candidates.len() - 2;
    let mut selected = vec![candidates[0]];
    for bucket in 0..buckets {
        let start = 1 + interior * bucket / buckets;
        let end = 1 + interior * (bucket + 1) / buckets;
        if start >= end {
            continue;
        }
        let mut extrema = Vec::with_capacity(per_bucket);
        for series in values {
            let mut min = candidates[start];
            let mut max = candidates[start];
            for &index in &candidates[start + 1..end] {
                if series[index].is_finite()
                    && (!series[min].is_finite() || series[index] < series[min])
                {
                    min = index;
                }
                if series[index].is_finite()
                    && (!series[max].is_finite() || series[index] > series[max])
                {
                    max = index;
                }
            }
            extrema.extend([min, max]);
        }
        extrema.sort_unstable();
        extrema.dedup();
        extrema.truncate(per_bucket);
        selected.extend(extrema);
    }
    selected.push(*candidates.last().unwrap());
    selected
}

/// 把 history 里的若干条序列导出成 JSON，供 GUI 画图。
///
/// GUI 进程**不链接 netcdf** —— 让它去读 history 会把整个 HDF5 拖进窗口进程。
/// 所以数值由这里（sidecar）读出来，以 JSON 过边界。
///
/// 时间轴给的是 **Unix 秒**，因为 uPlot 的 x 轴默认就是这个。但注意
/// PLUMBER2 是**地方时**（算例里 `greenwich = .false.`），所以这些秒数是
/// 「把地方时当成 UTC」算出来的 —— 前端必须按 UTC 格式化，才会显示成
/// 站点当地的钟点。按本地时区格式化会平移一个时区。
fn cmd_series(
    case: &Path,
    vars: &str,
    out: Option<&str>,
    from: Option<i64>,
    to: Option<i64>,
    max_points: Option<usize>,
) -> Result<()> {
    let layout = Layout::new(case);
    let name = colm_case::case_name(&layout.case_nml())?;
    let hists = history_files(&layout.out().join(&name))?;

    let mut names: Vec<&str> = vars
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    names.sort_unstable();
    names.dedup();
    if names.is_empty() {
        bail!("--vars needs at least one variable name");
    }
    if names.contains(&"time") {
        bail!("time is always returned as the x axis and must not be listed in --vars");
    }
    let mut wanted = vec!["time"];
    wanted.extend(names.iter().copied());
    let mut raw = read_history_many(&hists, &wanted)?;
    let t_min = raw.remove("time").expect("time was requested");
    // 换算住在 colm-hist::time —— 那个模块已经拥有「两种时间轴」这件事，
    // 而且它是 netcdf-free 的，GUI 后端将来也用得上。
    let unix = colm_hist::time::unix_seconds(&t_min);

    for v in &names {
        let values = &raw[*v];
        if values.len() != unix.len() {
            // 剖面变量是 (time, patch, soil) 之类，长度是时间步的数倍。
            // 它们要另一种画法，本轮不做 —— 但要说清楚而不是画出一条乱线。
            bail!(
                "{v} has {} values for {} time steps; it is not a (time, patch) series",
                values.len(),
                unix.len()
            );
        }
    }
    let series = raw.values().map(Vec::as_slice).collect::<Vec<_>>();
    let indexes = series_indices_multi(&unix, &series, from, to, max_points);
    let source_n = unix
        .iter()
        .filter(|time| from.is_none_or(|start| **time >= start))
        .filter(|time| to.is_none_or(|end| **time <= end))
        .count();
    let body = SeriesBody {
        files: hists.len(),
        source_n,
        n: indexes.len(),
        downsampled: indexes.len() < source_n,
        time: indexes.iter().map(|&index| unix[index]).collect(),
        vars: raw
            .into_iter()
            .map(|(name, values)| {
                let selected = indexes
                    .iter()
                    .map(|&index| values[index].is_finite().then_some(values[index]))
                    .collect();
                (name, selected)
            })
            .collect(),
    };
    let json = serde_json::to_string(&body)? + "\n";

    match out {
        Some(p) => {
            std::fs::write(p, &json)?;
            println!("wrote {} series x {} points to {p}", names.len(), body.n);
        }
        None => print!("{json}"),
    }
    Ok(())
}

// ---------------------------------------------------------------- forcing
//
// GUI 要能探测和转换强迫场，但 `gui/src-tauri` 不依赖 `colm-forcing` ——
// 加了会把静态 netcdf + HDF5 拖进 GUI 进程（见 `cmd_series` 上面那句
// 「GUI 进程不链接 netcdf」，以及 `oracle/Cargo.toml` 里同样为此立的
// 注释）。所以这两条子命令走 sidecar：GUI 起 colm-cli 子进程，解析它吐出
// 的 JSON——和 `scan` 那条路一样。

/// 变量的 `units` 属性原文，没有就是 `None`。
///
/// `forcing-probe`（猜到的槽位要报单位）与 `forcing-convert`（自动匹配的
/// 槽位要读源单位）都要这个，两条子命令共用一份读法。
fn variable_units(f: &netcdf::File, name: &str) -> Option<String> {
    f.variable(name)
        .and_then(|v| v.attribute_value("units"))
        .and_then(|r| r.ok())
        .and_then(|v| match v {
            netcdf::AttributeValue::Str(s) => Some(s),
            netcdf::AttributeValue::Strs(values) => values.into_iter().next(),
            _ => None,
        })
}

fn numeric_attribute_values(value: netcdf::AttributeValue) -> Vec<f64> {
    use netcdf::AttributeValue::*;
    match value {
        Uchar(value) => vec![f64::from(value)],
        Uchars(values) => values.into_iter().map(f64::from).collect(),
        Schar(value) => vec![f64::from(value)],
        Schars(values) => values.into_iter().map(f64::from).collect(),
        Ushort(value) => vec![f64::from(value)],
        Ushorts(values) => values.into_iter().map(f64::from).collect(),
        Short(value) => vec![f64::from(value)],
        Shorts(values) => values.into_iter().map(f64::from).collect(),
        Uint(value) => vec![f64::from(value)],
        Uints(values) => values.into_iter().map(f64::from).collect(),
        Int(value) => vec![f64::from(value)],
        Ints(values) => values.into_iter().map(f64::from).collect(),
        Ulonglong(value) => vec![value as f64],
        Ulonglongs(values) => values.into_iter().map(|value| value as f64).collect(),
        Longlong(value) => vec![value as f64],
        Longlongs(values) => values.into_iter().map(|value| value as f64).collect(),
        Float(value) => vec![f64::from(value)],
        Floats(values) => values.into_iter().map(f64::from).collect(),
        Double(value) => vec![value],
        Doubles(values) => values,
        Str(_) | Strs(_) => Vec::new(),
    }
}

fn variable_missing_markers(variable: &netcdf::Variable) -> Vec<f64> {
    ["_FillValue", "missing_value"]
        .into_iter()
        .filter_map(|name| variable.attribute_value(name))
        .filter_map(Result::ok)
        .flat_map(numeric_attribute_values)
        .collect()
}

fn variable_missing_count(variable: &netcdf::Variable) -> Result<usize> {
    let markers = variable_missing_markers(variable);
    let values: Vec<f64> = variable.get_values(netcdf::Extents::All)?;
    Ok(values
        .iter()
        .filter(|value| !value.is_finite() || markers.contains(value))
        .count())
}

fn read_file_1d(f: &netcdf::File, path: &Path, name: &str) -> Result<Vec<f64>> {
    let variable = f
        .variable(name)
        .with_context(|| format!("{} has no variable {name}", path.display()))?;
    let markers = variable_missing_markers(&variable);
    let mut values = variable
        .get_values::<f64, _>(..)
        .with_context(|| format!("cannot read {name} from {}", path.display()))?;
    for value in &mut values {
        if markers.contains(value) {
            *value = f64::NAN;
        }
    }
    Ok(values)
}

fn cmd_forcing_table_probe(file: &Path, json: bool) -> Result<()> {
    let probe = colm_forcing::probe_table(file)?;
    if json {
        let columns = probe
            .columns
            .iter()
            .map(|column| {
                serde_json::json!({
                    "name": column.name,
                    "units": column.units,
                })
            })
            .collect::<Vec<_>>();
        let sites = probe
            .sites
            .iter()
            .map(|site| {
                serde_json::json!({
                    "id": site.id,
                    "rows": site.rows,
                    "latitude": site.latitude,
                    "longitude": site.longitude,
                    "landtype": site.landtype,
                    "start": site.start,
                    "end": site.end,
                    "step_seconds": site.step_seconds,
                    "inserted_steps": site.inserted_steps,
                })
            })
            .collect::<Vec<_>>();
        let slots = probe
            .slots
            .iter()
            .map(|slot| {
                serde_json::json!({
                    "index": slot.index,
                    "meaning": slot.meaning,
                    "optional": slot.optional,
                    "column": slot.column,
                    "units": slot.units,
                    "wants": slot.wants,
                })
            })
            .collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "delimiter": probe.delimiter,
                "columns": columns,
                "rows": probe.rows,
                "site_column": probe.site_column,
                "time_column": probe.time_column,
                "latitude_column": probe.latitude_column,
                "longitude_column": probe.longitude_column,
                "landtype_column": probe.landtype_column,
                "utc_offset_column": probe.utc_offset_column,
                "sites": sites,
                "slots": slots,
            }))?
        );
        return Ok(());
    }
    println!(
        "{} row(s), {} site(s), {} delimiter",
        probe.rows,
        probe.sites.len(),
        probe.delimiter
    );
    println!(
        "columns: {}",
        probe
            .columns
            .iter()
            .map(|column| match &column.units {
                Some(units) => format!("{} [{units}]", column.name),
                None => column.name.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    );
    for site in &probe.sites {
        println!(
            "  {}: {} row(s), lat/lon {:?}/{:?}, step {:?}s, {} absent row(s)",
            site.id,
            site.rows,
            site.latitude,
            site.longitude,
            site.step_seconds,
            site.inserted_steps
        );
    }
    Ok(())
}

fn cmd_forcing_table_convert(src: &Path, destination: &Path, opts: &Opts) -> Result<()> {
    let slots = opts
        .get_all("--slot")
        .iter()
        .map(|spec| {
            let slot = colm_forcing::parse_slot_spec(spec)?;
            Ok(colm_forcing::TabularSlot {
                index: slot.index,
                column: slot.source_name,
                source_units: slot.source_units,
                also_add: slot.also_add,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let optional_f64 = |name: &str| -> Result<Option<f64>> {
        opts.get(name)
            .map(|value| {
                value
                    .parse::<f64>()
                    .with_context(|| format!("{name} {value:?} is not a number"))
            })
            .transpose()
    };
    let step_seconds = opts
        .get("--step-seconds")
        .map(|value| {
            value
                .parse::<i64>()
                .with_context(|| format!("--step-seconds {value:?} is not an integer"))
        })
        .transpose()?;
    let plan = colm_forcing::TabularPlan {
        time_column: opts.need_str("--time")?,
        site_column: opts.get("--site"),
        latitude_column: opts.get("--lat-column"),
        longitude_column: opts.get("--lon-column"),
        landtype_column: opts.get("--landtype-column"),
        utc_offset_column: opts.get("--offset-column"),
        manual_utc_offset: optional_f64("--utc-offset")?,
        latitude: optional_f64("--lat")?,
        longitude: optional_f64("--lon")?,
        step_seconds,
        land_cover_scheme: opts
            .get("--land-cover-scheme")
            .as_deref()
            .map(colm_forcing::LandCoverScheme::parse)
            .transpose()?,
        heights: opts
            .get("--height")
            .as_deref()
            .map(colm_forcing::parse_heights)
            .transpose()?,
        slots,
    };
    let imported = colm_forcing::import_table(src, destination, &plan)?;
    if opts.get("--json").is_some() {
        let sites = imported
            .iter()
            .map(|site| {
                serde_json::json!({
                    "site": site.site,
                    "safe_site": site.safe_site,
                    "staged_path": site.staged_path,
                    "final_path": site.final_path,
                    "rows": site.rows,
                    "inserted_steps": site.inserted_steps,
                    "latitude": site.latitude,
                    "longitude": site.longitude,
                    "landtype": site.landtype,
                    "timezone_offset_hours": site.timezone_offset_hours,
                    "timezone_source": site.timezone_source,
                    "start_utc": site.start_utc,
                    "end_utc": site.end_utc,
                })
            })
            .collect::<Vec<_>>();
        println!("{}", serde_json::to_string_pretty(&sites)?);
    } else {
        for site in &imported {
            println!(
                "{}: {} row(s) + {} absent time step(s) -> {}",
                site.site,
                site.rows,
                site.inserted_steps,
                site.staged_path.display()
            );
        }
    }
    Ok(())
}

/// 一个槽位探测到的结果，交给 GUI 的前处理页。
#[derive(serde::Serialize)]
struct SlotProbe {
    index: usize,
    meaning: &'static str,
    optional: bool,
    /// 猜不到是 `None`——JSON 里是 `null`。
    guessed: Option<&'static str>,
    /// 猜到的变量在源文件里的单位，读不到也是 `None`。
    units: Option<String>,
    /// CoLM 期望的单位（`convert::canonical_units`），与 `units` 对照着看。
    wants: &'static str,
}

/// 探测结果的整体，`forcing-probe --json 1` 吐出的就是这个形状。
#[derive(serde::Serialize)]
struct ForcingProbe {
    variables: Vec<String>,
    /// 每个变量的维度名与长度。GUI 用它阻止把整张区域网格误当成单点
    /// 边界层高度：`ncio_read_site_time` 对 POINT 只会取 (1,1,time)，
    /// 网格大于 1 时虽然能读，却会安静地读错站点。
    shapes: Vec<VariableShape>,
    slots: Vec<SlotProbe>,
    steps: usize,
    step_seconds: f64,
    step_uniform: bool,
    time_units: String,
    /// `time` 变量首末两个原始数值。仅比较 units/步数/步长仍可能漏掉整条
    /// 时间轴平移一个时间步的文件，POINT 边界层高度必须逐时对齐。
    time_first: f64,
    time_last: f64,
    /// 三个观测高度。源文件没有 `reference_height_*` 时是 `None`
    /// （JSON 里是 `null`），不是 `NaN`——`NaN` 不是合法的 JSON 数值。
    /// 实测 Urban-PLUMBER 的 21 个站全都没有这三个标量，PLUMBER2 的
    /// 90 个全有，两条路都要能经过这里而不炸。
    height_v: Option<f64>,
    height_t: Option<f64>,
    height_q: Option<f64>,
}

#[derive(serde::Serialize)]
struct VariableShape {
    name: String,
    dimensions: Vec<DimensionShape>,
}

#[derive(serde::Serialize)]
struct DimensionShape {
    name: String,
    len: usize,
}

#[derive(serde::Serialize)]
struct DatasetProbe {
    variables: Vec<String>,
    shapes: Vec<VariableShape>,
}

fn dataset_probe(f: &netcdf::File) -> DatasetProbe {
    let variables = f.variables().map(|variable| variable.name()).collect();
    let shapes = f
        .variables()
        .map(|variable| VariableShape {
            name: variable.name(),
            dimensions: variable
                .dimensions()
                .iter()
                .map(|dimension| DimensionShape {
                    name: dimension.name(),
                    len: dimension.len(),
                })
                .collect(),
        })
        .collect();
    DatasetProbe { variables, shapes }
}

/// 通用 NetCDF 结构探测，不要求气象强迫场的 time 坐标格式。臭氧文件只按
/// 索引读取 `OZONE`，合法文件可能有 time 维却没有同名坐标变量，不能拿
/// forcing-probe 的额外契约误拒绝。
fn cmd_netcdf_probe(file: &Path, json: bool) -> Result<()> {
    let f = netcdf::open(file).with_context(|| format!("cannot open {}", file.display()))?;
    let probe = dataset_probe(&f);
    if json {
        println!("{}", serde_json::to_string(&probe)?);
    } else {
        for shape in probe.shapes {
            let dimensions = shape
                .dimensions
                .iter()
                .map(|dimension| format!("{}={}", dimension.name, dimension.len))
                .collect::<Vec<_>>()
                .join(", ");
            println!("{} ({dimensions})", shape.name);
        }
    }
    Ok(())
}

fn cmd_mesh_new(opts: &Opts) -> Result<()> {
    let output = opts.need("--out")?;
    let grid_kind = opts
        .get("--grid-kind")
        .unwrap_or_else(|| "unstructured".to_string());
    let nlon = opts
        .need_str("--nlon")?
        .parse::<usize>()
        .context("--nlon must be a positive integer")?;
    let nlat = opts
        .need_str("--nlat")?
        .parse::<usize>()
        .context("--nlat must be a positive integer")?;
    let grid = colm_srfdata::Grid { nlon, nlat };

    let bbox = ["--west", "--east", "--south", "--north"]
        .map(|name| -> Result<Option<f64>> {
            opts.get(name)
                .map(|value| {
                    value
                        .parse::<f64>()
                        .with_context(|| format!("{name} must be a finite number"))
                })
                .transpose()
        })
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    let supplied = bbox.iter().filter(|value| value.is_some()).count();
    let shapefile = opts.get("--shp").map(PathBuf::from);
    if shapefile.is_some() && supplied > 0 {
        bail!("--shp and --west/--east/--south/--north are mutually exclusive");
    }
    let (mut mesh, domain_kind) = match (shapefile.as_ref(), supplied) {
        (Some(path), 0) => {
            let domain = colm_srfdata::shapefile::PolygonDomain::read(path)?;
            (
                colm_srfdata::mesh::EqualLatLonMesh::from_polygon(grid, &domain)?,
                "watershed",
            )
        }
        (None, 0) => {
            let window = colm_srfdata::mesh::MeshWindow::global(grid)?;
            (
                colm_srfdata::mesh::EqualLatLonMesh::all_active(grid, window)?,
                "global",
            )
        }
        (None, 4) => {
            let window = colm_srfdata::mesh::MeshWindow::covering_bbox(
                grid,
                bbox[0].unwrap(),
                bbox[1].unwrap(),
                bbox[2].unwrap(),
                bbox[3].unwrap(),
            )?;
            (
                colm_srfdata::mesh::EqualLatLonMesh::all_active(grid, window)?,
                "region",
            )
        }
        (None, _) => bail!("--west/--east/--south/--north must be supplied together"),
        (Some(_), _) => unreachable!(),
    };
    let non_ocean_mask = opts.get("--non-ocean-mask").map(PathBuf::from);
    let non_ocean_var = opts
        .get("--non-ocean-var")
        .unwrap_or_else(|| "non_ocean_mask".to_string());
    if opts.get("--non-ocean-var").is_some() && non_ocean_mask.is_none() {
        bail!("--non-ocean-var requires --non-ocean-mask");
    }
    if let Some(path) = non_ocean_mask.as_ref() {
        mesh = mesh.with_non_ocean_mask(path, &non_ocean_var)?;
    }
    let window = mesh.window;
    let (summary, schema, colm_mode) = match grid_kind.as_str() {
        "latlon" => (
            mesh.write_gridbased_netcdf(&output)?,
            "equal-lat-lon-landmask-v1",
            "GRIDBASED",
        ),
        "unstructured" => (
            mesh.write_netcdf(&output)?,
            "equal-lat-lon-elmindex-v1",
            "UNSTRUCTURED",
        ),
        "catchment" => bail!(
            "mesh-new cannot synthesize CATCHMENT data; provide DEF_CatchmentMesh_data with catchment and HRU fields"
        ),
        other => bail!("--grid-kind must be latlon or unstructured, got {other}"),
    };
    let manifest = serde_json::json!({
        "schema": schema,
        "grid_kind": grid_kind,
        "colm_mode": colm_mode,
        "element_id_type": "int64",
        "domain_kind": domain_kind,
        "shapefile": shapefile.as_ref().map(|path| path.display().to_string()),
        "non_ocean_mask": non_ocean_mask.as_ref().map(|path| path.display().to_string()),
        "non_ocean_var": non_ocean_mask.as_ref().map(|_| non_ocean_var),
        "output": output,
        "sha256": fingerprint::sha256_file(&output)?,
        "bytes": std::fs::metadata(&output)?.len(),
        "global_nlon": grid.nlon,
        "global_nlat": grid.nlat,
        "window_i0": window.i0,
        "window_j0": window.j0,
        "window_nlon": window.nlon,
        "window_nlat": window.nlat,
        "active_cells": summary.active_cells,
        "max_elmid": summary.max_elmid,
    });
    let manifest_path = PathBuf::from(format!("{}.manifest.json", output.display()));
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)
        .with_context(|| format!("cannot write {}", manifest_path.display()))?;
    println!("{}", serde_json::to_string(&manifest)?);
    Ok(())
}

fn cmd_spatial_preflight(opts: &Opts) -> Result<()> {
    let input = opts.need("--input")?;
    let input = input
        .canonicalize()
        .with_context(|| format!("cannot resolve {}", input.display()))?;
    let grid_kind = opts.need_str("--grid-kind")?;
    let summary = colm_srfdata::mesh::inspect_spatial_input(&input, &grid_kind)?;
    let manifest = serde_json::json!({
        "schema": "colm-spatial-input-manifest-v1",
        "grid_kind": grid_kind,
        "input_schema": summary.schema,
        "input": input,
        "sha256": fingerprint::sha256_file(&input)?,
        "bytes": std::fs::metadata(&input)?.len(),
        "element_id_type": "int64",
        "nlon": summary.nlon,
        "nlat": summary.nlat,
        "active_cells": summary.active_cells,
        "max_elmid": summary.max_elmid,
    });
    if let Some(output) = opts.get("--out") {
        std::fs::write(&output, serde_json::to_vec_pretty(&manifest)?)
            .with_context(|| format!("cannot write {output}"))?;
    }
    println!("{}", serde_json::to_string(&manifest)?);
    Ok(())
}

/// 非有限数不能进 JSON，所以在这里显式转成 `Option`，交给 `serde` 序列化。
fn present(x: f64) -> Option<f64> {
    x.is_finite().then_some(x)
}

/// 探测一份强迫场文件：八个槽位各猜到了什么变量、单位是什么，
/// 以及三个观测高度有没有。**只读元数据，不读场数据**——`colm_forcing::summarize`
/// 已经保证了这一点。
fn cmd_forcing_probe(file: &Path, json: bool) -> Result<()> {
    let summary = colm_forcing::summarize(file)?;
    let (resolved, _missing) = colm_forcing::resolve(&summary.variables);
    let f = netcdf::open(file).with_context(|| format!("cannot open {}", file.display()))?;
    let time_values: Vec<f64> = f
        .variable("time")
        .context("time variable disappeared after forcing summary")?
        .get_values(netcdf::Extents::All)?;

    let shapes = dataset_probe(&f).shapes;

    let slots: Vec<SlotProbe> = colm_forcing::SLOTS
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let guessed = resolved.vname[i];
            SlotProbe {
                index: s.index,
                meaning: s.meaning,
                optional: s.optional,
                units: guessed.and_then(|name| variable_units(&f, name)),
                guessed,
                wants: colm_forcing::canonical_units(s.index),
            }
        })
        .collect();

    let probe = ForcingProbe {
        variables: summary.variables.clone(),
        shapes,
        slots,
        steps: summary.steps,
        step_seconds: summary.step_seconds,
        step_uniform: summary.step_uniform,
        time_units: summary.time_units.clone(),
        time_first: *time_values.first().context("time axis is empty")?,
        time_last: *time_values.last().context("time axis is empty")?,
        height_v: present(summary.height_v),
        height_t: present(summary.height_t),
        height_q: present(summary.height_q),
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&probe)?);
    } else {
        println!(
            "{} variable(s): {}",
            probe.variables.len(),
            probe.variables.join(", ")
        );
        for s in &probe.slots {
            let g = s.guessed.unwrap_or("(none)");
            let u = s.units.as_deref().unwrap_or("?");
            println!(
                "  slot {} ({}){} <- {g} [{u}], wants {}",
                s.index,
                s.meaning,
                if s.optional { ", optional" } else { "" },
                s.wants
            );
        }
        println!(
            "steps={} step_seconds={} uniform={}",
            probe.steps, probe.step_seconds, probe.step_uniform
        );
        println!(
            "reference heights v/t/q = {:?}/{:?}/{:?}",
            probe.height_v, probe.height_t, probe.height_q
        );
    }
    Ok(())
}

/// 与独立 bin `forcing-convert` 同样的行为：必需槽位自动补齐；只要用户给过
/// `--slot`，未选择的可选槽位就保持未使用。带缺测的变量拦在入口，再转换。
/// **`--slot`/`--height` 的解析
/// 调 `colm_forcing::parse_slot_spec`/`parse_heights`**——那份解析原来在
/// `forcing-convert.rs` 里单独一份，抄第二遍意味着两处要同步改
/// （`convert.rs` 的 `copy_attributes` 就是同一段代码抄三遍、错也有三份
/// 的前车之鉴）。
fn cmd_forcing_convert(
    src: &Path,
    dst: &Path,
    slot_specs: &[String],
    height_spec: Option<&str>,
) -> Result<()> {
    let mut given: Vec<colm_forcing::convert::SlotPlan> = slot_specs
        .iter()
        .map(|s| colm_forcing::parse_slot_spec(s))
        .collect::<Result<_>>()?;
    let heights = height_spec.map(colm_forcing::parse_heights).transpose()?;

    let summary = colm_forcing::summarize(src)?;
    let overrides: Vec<(usize, String)> = given
        .iter()
        .map(|s| (s.index, s.source_name.clone()))
        .collect();
    let (resolved, missing) = colm_forcing::resolve_with(&summary.variables, &overrides);
    if !missing.is_empty() {
        for m in &missing {
            eprintln!("  {m}");
        }
        // 把文件里有什么列出来——只说「缺第 3 槽」用户无从下手。
        eprintln!("  {} has: {}", src.display(), summary.variables.join(", "));
        bail!("{} slot(s) unresolved", missing.len());
    }

    let f = netcdf::open(src).with_context(|| format!("cannot open {}", src.display()))?;
    let has_explicit_slots = !given.is_empty();
    for (i, slot) in colm_forcing::SLOTS.iter().enumerate() {
        if given.iter().any(|s| s.index == slot.index) {
            continue;
        }
        if slot.optional && has_explicit_slots {
            continue;
        }
        let Some(name) = resolved.vname[i] else {
            continue;
        };
        given.push(colm_forcing::convert::SlotPlan {
            index: slot.index,
            source_name: name.to_string(),
            source_units: variable_units(&f, name).unwrap_or_default(),
            also_add: Vec::new(),
        });
    }

    // 缺测拦在入口，不在转换里悄悄处理——道理与独立 bin 一致：fill value
    // 换算完还是个数，模型不会因此报错，只会跑出一份看着正常的错误结果。
    for sp in &given {
        for name in std::iter::once(&sp.source_name).chain(sp.also_add.iter()) {
            let Some(v) = f.variable(name) else { continue };
            let n = variable_missing_count(&v)?;
            if n > 0 {
                bail!(
                    "{name} has {n} non-finite or declared missing value(s); \
                     fill them before converting — a fill value survives unit \
                     conversion as a plausible-looking number and the model will \
                     run to completion with it"
                );
            }
        }
    }

    let plan = colm_forcing::convert::Plan {
        slots: given,
        heights,
    };
    colm_forcing::convert::convert(src, dst, &plan)?;
    println!("wrote {}", dst.display());
    for s in &plan.slots {
        println!(
            "  slot {} <- {} ({})",
            s.index, s.source_name, s.source_units
        );
    }
    Ok(())
}

fn forcing_repair_plan(src: &Path, opts: &Opts) -> Result<colm_forcing::RepairPlan> {
    let mut given = opts
        .get_all("--slot")
        .iter()
        .map(|spec| colm_forcing::parse_slot_spec(spec))
        .collect::<Result<Vec<_>>>()?;
    let summary = colm_forcing::summarize(src)?;
    let overrides = given
        .iter()
        .map(|slot| (slot.index, slot.source_name.clone()))
        .collect::<Vec<_>>();
    let (resolved, missing) = colm_forcing::resolve_with(&summary.variables, &overrides);
    if !missing.is_empty() {
        for problem in &missing {
            eprintln!("  {problem}");
        }
        bail!("{} forcing slot(s) unresolved", missing.len());
    }
    let file = netcdf::open(src).with_context(|| format!("cannot open {}", src.display()))?;
    let has_explicit_slots = !given.is_empty();
    for (position, slot) in colm_forcing::SLOTS.iter().enumerate() {
        if given.iter().any(|given| given.index == slot.index) {
            continue;
        }
        if slot.optional && has_explicit_slots {
            continue;
        }
        let Some(name) = resolved.vname[position] else {
            continue;
        };
        given.push(colm_forcing::convert::SlotPlan {
            index: slot.index,
            source_name: name.to_string(),
            source_units: variable_units(&file, name).unwrap_or_default(),
            also_add: Vec::new(),
        });
    }
    let parse_optional = |name: &str| -> Result<Option<f64>> {
        opts.get(name)
            .map(|value| {
                value
                    .parse::<f64>()
                    .with_context(|| format!("{name} {value:?} is not a number"))
            })
            .transpose()
    };
    Ok(colm_forcing::RepairPlan {
        slots: given
            .into_iter()
            .map(|slot| colm_forcing::RepairSlot {
                index: slot.index,
                source_name: slot.source_name,
                source_units: slot.source_units,
                also_add: slot.also_add,
            })
            .collect(),
        short_gap_max: opts
            .get("--short-gap")
            .map(|value| value.parse::<usize>())
            .transpose()
            .context("--short-gap must be a non-negative number of time steps")?
            .unwrap_or(3),
        manual_utc_offset: parse_optional("--utc-offset")?,
        latitude: parse_optional("--lat")?,
        longitude: parse_optional("--lon")?,
        era5: opts.get("--era5").map(PathBuf::from),
        min_overlap: opts
            .get("--min-overlap")
            .map(|value| value.parse::<usize>())
            .transpose()
            .context("--min-overlap must be a non-negative sample count")?
            .unwrap_or(24),
    })
}

#[derive(serde::Serialize)]
struct RepairSummaryJson<'a> {
    timezone_offset_hours: f64,
    timezone_source: &'static str,
    timezone_confidence: &'static str,
    timezone_conflict: bool,
    solar_noon_hour: Option<f64>,
    solar_noon_std_hours: Option<f64>,
    latitude: f64,
    longitude: f64,
    start_date: String,
    end_date: String,
    missing: usize,
    unresolved: usize,
    needs_era5: bool,
    variables: Vec<RepairVariableJson<'a>>,
}

#[derive(serde::Serialize)]
struct RepairVariableJson<'a> {
    slot: usize,
    variable: &'a str,
    missing: usize,
    quality_rejected: usize,
    short_missing: usize,
    long_missing: usize,
    longest_gap: usize,
    interpolated: usize,
    era5_corrected: usize,
    unresolved: usize,
}

fn repair_summary_json(summary: &colm_forcing::RepairSummary) -> RepairSummaryJson<'_> {
    RepairSummaryJson {
        timezone_offset_hours: summary.timezone.offset_hours,
        timezone_source: summary.timezone.source.as_str(),
        timezone_confidence: summary.timezone.confidence.as_str(),
        timezone_conflict: summary.timezone.conflict,
        solar_noon_hour: summary.timezone.solar_noon_hour,
        solar_noon_std_hours: summary.timezone.solar_noon_std_hours,
        latitude: summary.latitude,
        longitude: summary.longitude,
        start_date: unix_date(summary.start_utc),
        end_date: unix_date(summary.end_utc),
        missing: summary.missing(),
        unresolved: summary.unresolved(),
        needs_era5: summary.needs_era5(),
        variables: summary
            .variables
            .iter()
            .map(|variable| RepairVariableJson {
                slot: variable.slot,
                variable: &variable.variable,
                missing: variable.missing,
                quality_rejected: variable.quality_rejected,
                short_missing: variable.short_missing,
                long_missing: variable.long_missing,
                longest_gap: variable.longest_gap,
                interpolated: variable.interpolated,
                era5_corrected: variable.era5_corrected,
                unresolved: variable.unresolved,
            })
            .collect(),
    }
}

fn unix_date(seconds: i64) -> String {
    let (year, month, day) = colm_forcing::civil_from_days(seconds.div_euclid(86400));
    format!("{year:04}-{month:02}-{day:02}")
}

fn print_repair_summary(summary: &colm_forcing::RepairSummary, json: bool) -> Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&repair_summary_json(summary))?
        );
        return Ok(());
    }
    println!(
        "timezone UTC{:+} ({}) at {}, {}; missing={}, unresolved={}",
        summary.timezone.offset_hours,
        summary.timezone.source.as_str(),
        summary.latitude,
        summary.longitude,
        summary.missing(),
        summary.unresolved()
    );
    for variable in &summary.variables {
        println!(
            "  slot {} {}: missing={}, qc_rejected={}, short={}, long={}, interpolated={}, era5={}, unresolved={}",
            variable.slot,
            variable.variable,
            variable.missing,
            variable.quality_rejected,
            variable.short_missing,
            variable.long_missing,
            variable.interpolated,
            variable.era5_corrected,
            variable.unresolved
        );
    }
    Ok(())
}

fn cmd_era5land_download(
    destination: &Path,
    latitude: f64,
    longitude: f64,
    start: &str,
    end: &str,
) -> Result<()> {
    if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
        bail!("ERA5-Land point is outside the valid latitude/longitude range");
    }
    let start_date = parse_date(start)?;
    let end_date = parse_date(end)?;
    if start_date > end_date {
        bail!("ERA5-Land start date {start} is after end date {end}");
    }
    require_cds_api_config()?;
    std::fs::create_dir_all(destination)
        .with_context(|| format!("cannot create {}", destination.display()))?;
    let script = destination.join("download_era5land.py");
    std::fs::write(&script, ERA5LAND_DOWNLOADER)
        .with_context(|| format!("cannot write {}", script.display()))?;

    let (python, python_args) = python_candidates(cfg!(windows))
        .into_iter()
        .find(|(program, prefix)| {
            std::process::Command::new(program)
                .args(*prefix)
                .arg("-c")
                .arg("import cdsapi")
                .status()
                .is_ok_and(|status| status.success())
        })
        .context(
            "Python cdsapi is unavailable. Install it with `python -m pip install cdsapi`, \
             configure ~/.cdsapirc, and accept the ERA5-Land dataset terms in CDS",
        )?;
    let grid_lat = (latitude * 10.0).round() / 10.0;
    let grid_lon = (longitude * 10.0).round() / 10.0;
    let point_cache = destination.join(colm_forcing::era5_point_cache_name(grid_lat, grid_lon));
    let output = std::process::Command::new(python)
        .args(python_args)
        .arg(&script)
        .arg(point_cache)
        .arg(format!("{grid_lat:.1}"))
        .arg(format!("{grid_lon:.1}"))
        .arg(start)
        .arg(end)
        .output()
        .with_context(|| format!("cannot launch {python}"))?;
    if !output.status.success() {
        bail!(
            "ERA5-Land download failed. Check ~/.cdsapirc and accept the dataset terms at \
             https://cds.climate.copernicus.eu/datasets/reanalysis-era5-land-timeseries\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    print!("{}", String::from_utf8_lossy(&output.stdout));
    Ok(())
}

fn python_candidates(windows: bool) -> Vec<(&'static str, &'static [&'static str])> {
    if windows {
        vec![("py", &["-3"]), ("python", &[]), ("python3", &[])]
    } else {
        vec![("python3", &[]), ("python", &[])]
    }
}

fn require_cds_api_config() -> Result<()> {
    let variables = if cfg!(windows) {
        ["USERPROFILE", "HOME"]
    } else {
        ["HOME", "USERPROFILE"]
    };
    let home = variables
        .into_iter()
        .find_map(|name| std::env::var_os(name).filter(|value| !value.is_empty()))
        .context("无法确定用户主目录，不能查找 ~/.cdsapirc")?;
    require_cds_api_config_at(&PathBuf::from(home).join(".cdsapirc"))
}

fn require_cds_api_config_at(path: &Path) -> Result<()> {
    let metadata = std::fs::metadata(path).with_context(|| cds_api_config_help(path))?;
    if !metadata.is_file() || metadata.len() == 0 {
        bail!(cds_api_config_help(path));
    }
    std::fs::File::open(path).with_context(|| cds_api_config_help(path))?;
    Ok(())
}

fn cds_api_config_help(path: &Path) -> String {
    let install = if cfg!(windows) {
        "py -m pip install cdsapi"
    } else {
        "python -m pip install cdsapi"
    };
    format!(
        "没有找到可用的 CDS API 配置：{}\n\
         请按以下步骤配置：\n\
         1. 登录 https://cds.climate.copernicus.eu/how-to-api\n\
         2. 将该页面提供的配置内容原样保存到 {}\n\
         3. 运行 `{install}` 安装客户端\n\
         4. 打开 https://cds.climate.copernicus.eu/datasets/reanalysis-era5-land-timeseries 并接受数据许可\n\
         5. 返回 CoLM Desktop，再次点击 ERA5-Land 下载。\n\
         软件只检查该文件是否存在、非空且可读，不会显示或上传其中的凭据。",
        path.display(),
        path.display()
    )
}

#[cfg(test)]
mod era5land_download_tests {
    use super::*;

    #[test]
    fn a_missing_cds_config_gives_complete_setup_steps() {
        let path = std::env::temp_dir().join(format!(
            "colm-cdsapi-missing-{}-.cdsapirc",
            std::process::id()
        ));
        let message = require_cds_api_config_at(&path).unwrap_err().to_string();
        let install = if cfg!(windows) {
            "py -m pip install cdsapi"
        } else {
            "python -m pip install cdsapi"
        };
        for expected in [
            path.to_string_lossy().as_ref(),
            "https://cds.climate.copernicus.eu/how-to-api",
            install,
            "reanalysis-era5-land",
            "不会显示或上传",
        ] {
            assert!(
                message.contains(expected),
                "missing {expected:?}: {message}"
            );
        }
    }

    #[test]
    fn point_download_uses_one_long_timeseries_request() {
        assert!(ERA5LAND_DOWNLOADER.contains("reanalysis-era5-land-timeseries"));
        assert!(
            ERA5LAND_DOWNLOADER.contains(r#""date": [f"{start.isoformat()}/{end.isoformat()}"]"#)
        );
        assert!(!ERA5LAND_DOWNLOADER.contains("for (year, month)"));
        assert!(ERA5LAND_DOWNLOADER.contains("saved_start <= start and saved_end >= end"));
        assert!(ERA5LAND_DOWNLOADER.contains("fcntl.flock"));
        assert!(ERA5LAND_DOWNLOADER.contains("msvcrt.locking"));
        assert!(ERA5LAND_DOWNLOADER.contains("errno.EACCES, errno.EAGAIN"));
        assert!(!ERA5LAND_DOWNLOADER.contains("21600"));
    }

    #[test]
    fn windows_python_launcher_is_supported() {
        assert!(python_candidates(true).contains(&("py", &["-3"])));
    }
}

const ERA5LAND_DOWNLOADER: &str = r#"#!/usr/bin/env python3
import json
import errno
import os
import shutil
import sys
import time
from datetime import date, timedelta
from pathlib import Path
from zipfile import ZipFile, is_zipfile

import cdsapi

cache = Path(sys.argv[1])
lat = float(sys.argv[2])
lon = float(sys.argv[3])
start = date.fromisoformat(sys.argv[4]) - timedelta(days=1)
end = date.fromisoformat(sys.argv[5]) + timedelta(days=1)

variables = [
    "2m_temperature",
    "2m_dewpoint_temperature",
    "surface_pressure",
    "total_precipitation",
    "10m_u_component_of_wind",
    "10m_v_component_of_wind",
    "surface_solar_radiation_downwards",
    "surface_thermal_radiation_downwards",
]

cache.mkdir(parents=True, exist_ok=True)
key = f"era5land_timeseries_{start.isoformat()}_{end.isoformat()}"
manifest_path = cache / f"{key}.json"
request = {
    "variable": variables,
    "location": {"longitude": lon, "latitude": lat},
    "date": [f"{start.isoformat()}/{end.isoformat()}"],
    "data_format": "netcdf",
}

def cached_files_cover_request():
    for candidate in cache.glob("era5land_timeseries_*.json"):
        try:
            manifest = json.loads(candidate.read_text(encoding="utf-8"))
            saved = manifest.get("request", {})
            saved_dates = saved.get("date", [])
            if (saved.get("variable") != variables
                    or saved.get("location") != request["location"]
                    or len(saved_dates) != 1):
                continue
            saved_start, saved_end = map(date.fromisoformat, saved_dates[0].split("/"))
            files = [cache / name for name in manifest.get("files", [])]
            if saved_start <= start and saved_end >= end and files \
                    and all(path.stat().st_size > 0 for path in files):
                return files
        except (AttributeError, FileNotFoundError, OSError, TypeError, ValueError):
            continue
    return []

if cached_files_cover_request():
    print(f"cached {cache}", flush=True)
    print(f"ERA5-Land cache ready: {cache}")
    raise SystemExit(0)

# One point cache has one writer. OS advisory locks are released automatically
# when a downloader exits or crashes, so a long CDS queue can never be mistaken
# for a stale writer and two processes cannot install the same cache together.
lock = cache / ".download.lock"
lock_handle = lock.open("a+b")
lock_handle.seek(0, os.SEEK_END)
if lock_handle.tell() == 0:
    lock_handle.write(b"0")
    lock_handle.flush()
while True:
    try:
        lock_handle.seek(0)
        if os.name == "nt":
            import msvcrt
            msvcrt.locking(lock_handle.fileno(), msvcrt.LK_NBLCK, 1)
        else:
            import fcntl
            fcntl.flock(lock_handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        break
    except OSError as error:
        if error.errno not in (errno.EACCES, errno.EAGAIN):
            raise
        print("another ERA5-Land request is writing this shared point cache; waiting…", flush=True)
        time.sleep(2)

download = cache / f".{key}.{os.getpid()}.download"
created = []
try:
    if cached_files_cover_request():
        print(f"cached {cache}", flush=True)
        raise SystemExit(0)
    print(f"submitting {lat:.1f}, {lon:.1f}: {start} to {end}; CDS may queue the request", flush=True)
    cdsapi.Client().retrieve("reanalysis-era5-land-timeseries", request).download(str(download))
    if is_zipfile(download):
        with ZipFile(download) as archive:
            members = [member for member in archive.infolist()
                       if not member.is_dir() and Path(member.filename).suffix.lower() == ".nc"]
            if not members:
                raise RuntimeError("ERA5-Land response contains no NetCDF file")
            for index, member in enumerate(members):
                target = cache / f"{key}_{index:02d}_{Path(member.filename).name}"
                part = target.with_suffix(target.suffix + ".part")
                with archive.open(member) as source, part.open("wb") as destination:
                    shutil.copyfileobj(source, destination)
                part.replace(target)
                created.append(target)
    else:
        target = cache / f"{key}.nc"
        download.replace(target)
        created.append(target)
    if not created or any(path.stat().st_size == 0 for path in created):
        raise RuntimeError("ERA5-Land download produced an empty NetCDF file")
    manifest = {
        "dataset": "reanalysis-era5-land-timeseries",
        "request": request,
        "files": [path.name for path in created],
    }
    temporary_manifest = manifest_path.with_suffix(".json.part")
    temporary_manifest.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    temporary_manifest.replace(manifest_path)
    print(f"ERA5-Land cache ready: {cache}")
finally:
    if download.exists():
        download.unlink()
    lock_handle.seek(0)
    if os.name == "nt":
        msvcrt.locking(lock_handle.fileno(), msvcrt.LK_UNLCK, 1)
    else:
        fcntl.flock(lock_handle.fileno(), fcntl.LOCK_UN)
    lock_handle.close()
"#;

/// 删掉一个算例已有的 history 文件，返回删了几个。
///
/// 目录不存在（第一次跑）返回 0，不报错。
fn clear_history(out: &Path) -> Result<usize> {
    let dir = out.join("history");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Ok(0);
    };
    let mut n = 0;
    for e in rd.flatten() {
        let p = e.path();
        if p.file_name()
            .and_then(|x| x.to_str())
            .is_some_and(|x| x.contains("_hist_") && x.ends_with(".nc"))
        {
            std::fs::remove_file(&p).with_context(|| format!("cannot remove {}", p.display()))?;
            n += 1;
        }
    }
    Ok(n)
}

/// 算例的**全部** history 文件，按名字排序。
///
/// 这里原来有两个各取一个文件的函数：`metrics` 取第一个、`series` 取最后
/// 一个。**在只跑十天的黄金算例上永远只有一个文件**，所以两者从来没有
/// 分歧过；而一个 11 年的站点会写出 132 个月度文件，于是指标算的是
/// 2002 年 1 月、曲线画的是 2012 年 12 月，谁也不知道。
///
/// 实测 AT-Neu：只看 2002 年 1 月时 Rnet 的观测均值是 -32.6 W/m²
/// （阿尔卑斯草地的隆冬，本来就是负的），Qh/Qle 的 R² 近 0 ——
/// 看上去像模型烂掉了，其实是在拿一个雪被覆盖的月份评一个 11 年的模拟。
fn history_files(out: &Path) -> Result<Vec<PathBuf>> {
    let dir = out.join("history");
    let mut h: Vec<PathBuf> = std::fs::read_dir(&dir)
        .with_context(|| format!("no history at {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains("_hist_") && n.ends_with(".nc"))
        })
        .collect();
    // 文件名是 `NAME_hist_YYYY-MM.nc`，月份补零，所以字典序就是时间序。
    // 但不假设它一定如此 —— `read_history` 会验证拼出来的时间轴单调递增。
    h.sort();
    if h.is_empty() {
        bail!("no *_hist_*.nc under {}", dir.display());
    }
    Ok(h)
}

/// 跨全部 history 文件读一个变量，接成一条。
///
/// 各月度文件的 `time` 用同一个原点（`minutes since 1900-1-1`，实测
/// 2002-01 与 2012-12 都是），所以直接首尾相接即可，不需要逐文件换算。
///
/// **要验证单调递增。** 文件名排序碰巧等于时间序，但那是约定不是保证；
/// 顺序错了的结果是一条乱序的曲线和一批错配的指标 —— 两者都不会报错。
fn read_history(files: &[PathBuf], var: &str) -> Result<Vec<f64>> {
    Ok(read_history_many(files, &[var])?
        .remove(var)
        .expect("the requested variable is present in the result map"))
}

fn history_stream_name(path: &Path) -> &str {
    let suffix = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.split_once("_hist_"))
        .map(|(_, suffix)| suffix)
        .unwrap_or_default();
    let prefix = suffix.split('_').next().unwrap_or_default();
    if prefix.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        "primary"
    } else {
        prefix
    }
}

fn history_streams(files: &[PathBuf]) -> Vec<Vec<PathBuf>> {
    let mut streams = std::collections::BTreeMap::<&str, Vec<PathBuf>>::new();
    for path in files {
        streams
            .entry(history_stream_name(path))
            .or_default()
            .push(path.clone());
    }
    let mut out = Vec::with_capacity(streams.len());
    if let Some(primary) = streams.remove("primary") {
        out.push(primary);
    }
    out.extend(streams.into_values());
    out
}

fn primary_history_files(files: &[PathBuf]) -> Vec<PathBuf> {
    history_streams(files)
        .into_iter()
        .next()
        .unwrap_or_else(|| files.to_vec())
}

fn files_with_variables(files: &[PathBuf], vars: &[&str]) -> Result<Vec<PathBuf>> {
    for stream in history_streams(files) {
        let first = stream.first().expect("empty streams were removed");
        let file =
            netcdf::open(first).with_context(|| format!("cannot open {}", first.display()))?;
        if vars.iter().all(|name| file.variable(name).is_some()) {
            return Ok(stream);
        }
    }
    bail!("no history file contains {}", vars.join(", "))
}

fn history_has_variables(files: &[PathBuf], vars: &[&str]) -> bool {
    files_with_variables(files, vars).is_ok()
}

fn read_history_many_from(
    files: &[PathBuf],
    vars: &[&str],
) -> Result<std::collections::BTreeMap<String, Vec<f64>>> {
    let mut out = vars
        .iter()
        .map(|name| ((*name).to_string(), Vec::new()))
        .collect::<std::collections::BTreeMap<_, _>>();
    for f in files {
        let file = netcdf::open(f).with_context(|| format!("cannot open {}", f.display()))?;
        for name in vars {
            let values = read_file_1d(&file, f, name)?;
            out.get_mut(*name)
                .expect("all names were inserted")
                .extend(values);
        }
    }
    if let Some(time) = out.get("time") {
        check_increasing(time).with_context(|| {
            format!(
                "the history files in {} do not concatenate into an increasing time axis; sorting by file name is not giving chronological order here",
                files[0].parent().unwrap_or(Path::new(".")).display()
            )
        })?;
    }
    Ok(out)
}

/// Read scalar history variables from the file series that actually contains them.
/// Main and tracer history are separate NetCDF series with duplicate timestamps; do
/// not concatenate both just because both names match `*_hist_*.nc`.
fn read_history_many(
    files: &[PathBuf],
    vars: &[&str],
) -> Result<std::collections::BTreeMap<String, Vec<f64>>> {
    let wants_time = vars.contains(&"time");
    let data_vars = vars
        .iter()
        .copied()
        .filter(|name| *name != "time")
        .collect::<std::collections::BTreeSet<_>>();
    if data_vars.is_empty() {
        return read_history_many_from(&primary_history_files(files), vars);
    }

    let mut out = std::collections::BTreeMap::new();
    let mut found = std::collections::BTreeSet::new();
    let mut reference_time: Option<Vec<f64>> = None;
    for stream in history_streams(files) {
        let first = stream.first().expect("empty streams were removed");
        let file =
            netcdf::open(first).with_context(|| format!("cannot open {}", first.display()))?;
        let stream_vars = data_vars
            .iter()
            .copied()
            .filter(|name| !found.contains(name) && file.variable(name).is_some())
            .collect::<Vec<_>>();
        drop(file);
        if stream_vars.is_empty() {
            continue;
        }
        let mut request = Vec::new();
        if wants_time || reference_time.is_some() {
            request.push("time");
        }
        request.extend(stream_vars.iter().copied());
        let mut data = read_history_many_from(&stream, &request)?;
        if let Some(time) = data.remove("time") {
            if let Some(reference) = &reference_time {
                if reference != &time {
                    bail!(
                        "history streams use different time axes for {}",
                        stream_vars.join(", ")
                    );
                }
            } else {
                reference_time = Some(time);
            }
        }
        for name in stream_vars {
            out.insert(
                name.to_string(),
                data.remove(name).expect("stream variable requested"),
            );
            found.insert(name);
        }
    }
    for name in data_vars {
        if !found.contains(name) {
            bail!("{} has no variable {}", files[0].display(), name);
        }
    }
    if wants_time {
        out.insert(
            "time".into(),
            reference_time.unwrap_or_else(|| {
                read_history_many_from(&primary_history_files(files), &["time"])
                    .unwrap_or_default()
                    .remove("time")
                    .unwrap_or_default()
            }),
        );
    }
    Ok(out)
}

/// 拼出来的时间轴必须严格递增。
///
/// **顺序错了不会报错**，只会给出一条乱序的曲线和一批错配的指标。
/// 相等也不行：两个文件的时间重叠说明同一时刻被写了两次，
/// 而配对会把其中一个悄悄丢掉。
fn check_increasing(t: &[f64]) -> Result<()> {
    for (i, w) in t.windows(2).enumerate() {
        if w[1] <= w[0] {
            bail!("time goes from {} to {} at index {}", w[0], w[1], i);
        }
    }
    Ok(())
}

#[cfg(test)]
fn netcdf_test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

#[cfg(test)]
fn netcdf_test_guard() -> std::sync::MutexGuard<'static, ()> {
    netcdf_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod history_tests;

#[cfg(test)]
#[path = "run_stage_tests.rs"]
mod run_stage_tests;

#[cfg(test)]
#[path = "window_tests.rs"]
mod window_tests;

#[cfg(test)]
#[path = "forcing_probe_tests.rs"]
mod forcing_probe_tests;

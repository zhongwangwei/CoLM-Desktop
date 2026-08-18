//! CoLM 单点的唯一编排可执行文件。
//!
//! `design.md` §4.2：「GUI 只跟它说话」。所以这里是唯一一处同时依赖全部五层的
//! 地方，各层之间仍然互不依赖 —— 造算例的 `colm-case` 不认识内核，
//! 答「能产出什么」的 `colm-hist` 闸门表不认识 netcdf。
//!
//! ```text
//! colm-cli new     --site <站点文件> --out <目录> [--name N] [--start Y-M-D] [--end Y-M-D]
//! colm-cli run     <算例目录> --kernel <目录>
//! colm-cli metrics <算例目录> --obs <Flux.nc> [--spinup N]
//! colm-cli series  <算例目录> --vars f_rnet,f_fsena [--out series.json]
//! colm-cli all     --site ... --out ... --kernel ... [--obs ...]
//! ```
//!
//! `--start` / `--end` 不给就用强迫场覆盖的完整范围。经纬度与地类读自站点
//! 文件，时间步长读自强迫场文件 —— 这三样都不问用户。

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use colm_case::{fields, minimal::required, render, CaseSpec, Dirs, Layout, Window};
use colm_kernel::outcome::Stage;
use colm_kernel::Kernel;

const USAGE: &str = "\
usage:
  colm-cli new     --site <site.nc> --out <dir> [--name N] [--start Y-M-D] [--end Y-M-D]
                   [--rawdata <dir>] [--runtime <dir>]
                   # 城市站点由文件形状自动识别，那时 --rawdata/--runtime 必填
  colm-cli run     <case-dir> --kernel <dir>
  colm-cli metrics <case-dir> --obs <Flux.nc> [--spinup N]
  colm-cli series  <case-dir> --vars f_rnet,f_fsena [--out series.json]
  colm-cli all     --site <site.nc> --out <dir> --kernel <dir> [--obs <Flux.nc>] [--name N]
                   [--start Y-M-D] [--end Y-M-D] [--spinup N]
";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        print!("{USAGE}");
        std::process::exit(2);
    };
    let opts = Opts::parse(&args[1..])?;
    match cmd.as_str() {
        "new" => {
            let case = cmd_new(&opts)?;
            println!("case ready: {}", case.display());
        }
        "run" => {
            cmd_run(&opts.positional_case()?, &opts.need("--kernel")?)?;
        }
        "metrics" => {
            let case = opts.positional_case()?;
            cmd_metrics(&case, &opts.need("--obs")?, opts.spinup()?)?;
        }
        "series" => {
            let case = opts.positional_case()?;
            cmd_series(
                &case,
                &opts.need_str("--vars")?,
                opts.get("--out").as_deref(),
            )?;
        }
        "all" => {
            let case = cmd_new(&opts)?;
            cmd_run(&case, &opts.need("--kernel")?)?;
            match opts.get("--obs") {
                Some(obs) => cmd_metrics(&case, Path::new(&obs), opts.spinup()?)?,
                None => println!("no --obs given; skipping the metrics table"),
            }
        }
        other => {
            eprintln!("unknown command: {other}\n{USAGE}");
            std::process::exit(2);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- 参数

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
    let name = site.file_name()?.to_str()?;
    for (site_suffix, met, flux) in LAYOUTS {
        let Some(stem) = name.strip_suffix(site_suffix) else {
            continue;
        };
        let p = site
            .parent()?
            .parent()?
            .join(dir)
            .join(format!("{stem}{}", if which == 0 { met } else { flux }));
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn cmd_new(o: &Opts) -> Result<PathBuf> {
    let site_raw = o.need("--site")?;
    let out = o.need("--out")?;
    let met = sibling(&site_raw, "Forcing", 0).with_context(|| {
        format!(
            "cannot find the forcing file next to {}; expected ../Forcing/<stem>_Met.nc",
            site_raw.display()
        )
    })?;

    std::fs::create_dir_all(&out)?;
    let layout = Layout::new(&out);
    std::fs::create_dir_all(layout.out())?;

    // 1. 补齐站点文件 —— CoLM 缺字段时会回落到几百 GB 的全球 rawdata。
    //
    // 但**只对 PLUMBER2 形状的文件做**。城市站点文件（Urban-PLUMBER）的变量集
    // 完全不同：23 个城市形态学量，没有土壤剖面也没有 `IGBP_classification`，
    // 而 CoLM 的 URBAN 路径本来就直接读原件（实测示例算例用的就是未经处理的
    // 原文件，逐字节相同）。对它做补齐既没有依据，也没有必要。
    let obs = sibling(&site_raw, "Observation", 1);
    // 城市站点文件不带 `IGBP_classification` —— 那正是它的标志。
    let looks_like_plumber2 = colm_srfdata::site::location(&site_raw)?.landtype.is_some();
    // 没有 `--urban` 开关：拿一个草地站强行跑城市只会在 NCAR 属性表上越界，
    // 而一个城市站不跑城市模块也没有意义。判据完全交给站点文件的形状。
    let urban = !looks_like_plumber2;
    if looks_like_plumber2 {
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
        // 城市站点文件：只补一样东西，见 `prepare_urban`。
        let rep = colm_srfdata::site::prepare_urban(&site_raw, &layout.site_nc())?;
        match rep.elevation {
            Some(h) => println!(
                "site: urban-shaped; elevation {h} m taken from ground_height so CoLM never \
                 needs the 7 GB elevation.nc"
            ),
            None => println!("site: urban-shaped; copied as-is"),
        }
    }

    // 2. 强迫场 namelist —— 不转换数据，CoLM 直接读 PLUMBER2 的 Met 文件
    let summary = colm_forcing::summarize(&met)?;
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
    let e = summary.end();
    let end = match o.get("--end") {
        Some(s) => parse_date(&s)?,
        None => (e.year, e.month, e.day),
    };
    let loc = colm_srfdata::site::location(&layout.site_nc())?;
    let name = o.get("--name").unwrap_or_else(|| {
        site_raw
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.split('_').next())
            .unwrap_or("case")
            .to_string()
    });

    // 全球栅格目录。水热与 BGC 算例一个字节都不读它 —— `site::fill` 已经把
    // 该有的都写进 site.nc 了 —— 所以那里故意指向一个不存在的目录：跑通了就
    // **证明**没读。城市算例做不到这一点：土壤剖面、湖深、土壤反照率、LCZ
    // 分类、坡度都只能从栅格取（实测 mksrfdata 的来源清单里 30 项写着
    // "from CoLM 2024 raw data"），所以那两个目录变成必填。
    let dirs = if urban {
        let raw = o.get("--rawdata").ok_or_else(|| {
            anyhow!(
                "an urban case needs --rawdata: the site file carries only morphology, \
                 so soil, lake depth, albedo and the LCZ class all come from the global grid"
            )
        })?;
        let run = o.get("--runtime").ok_or_else(|| {
            anyhow!("an urban case needs --runtime: DEF_dir_runtime holds the urban LUCY tables")
        })?;
        (slash(Path::new(&raw)), slash(Path::new(&run)))
    } else {
        (
            text(&out.join("rawdata_unused/")),
            text(&out.join("runtime_unused/")),
        )
    };

    let spec = CaseSpec {
        name: name.clone(),
        site_file: text(&layout.site_nc()),
        lon: loc.lon,
        lat: loc.lat,
        landtype: loc.landtype,
        window: Window {
            start_year: start.0,
            start_month: start.1,
            start_day: start.2,
            end_year: end.0,
            end_month: end.1,
            end_day: end.2,
        },
        timestep_seconds: summary.step_seconds,
        // 由文件说了算，不写死 —— PLUMBER2 是地方时，Urban-PLUMBER 是 UTC
        greenwich: summary.is_greenwich(),
        urban,
        dirs: Dirs {
            rawdata: dirs.0.clone(),
            runtime: dirs.1.clone(),
            output: text(&layout.out()) + "/",
            forcing_namelist: text(&layout.forcing_nml()),
        },
    };
    let all = fields(&spec);
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

fn text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// CoLM 把 `DEF_dir_rawdata` 与文件名直接拼接，中间不补分隔符
/// （`trim(DEF_dir_rawdata)//'urban/...'`），所以尾斜杠不是修饰，是必需的。
fn slash(p: &Path) -> String {
    let s = text(p);
    if s.ends_with('/') {
        s
    } else {
        s + "/"
    }
}

// ---------------------------------------------------------------- run

fn cmd_run(case: &Path, kernel_dir: &Path) -> Result<()> {
    let kernel = Kernel::open(kernel_dir)?;
    println!(
        "kernel: {} {} ({})",
        kernel.manifest.preset, kernel.manifest.colm_git_sha, kernel.manifest.platform
    );
    let layout = Layout::new(case);
    let name = colm_case::case_name(&layout.case_nml())?;
    let out = layout.out().join(&name);
    let const_dir = out.join("restart/const");
    // 产物必须列到**文件**：目录在程序写任何东西之前就已存在，
    // 只列目录的话「跑完了但什么都没写」恰好抓不到。
    let stages = [
        (Stage::MkSrfData, vec![out.join("landdata/srfdata.nc")]),
        (
            Stage::MkIniData,
            vec![
                const_dir.join(format!("{name}_restart_const_lc2005_w180_s90.nc")),
                const_dir.join(format!("{name}_restart_const_lc2005.nc")),
            ],
        ),
        (Stage::Colm, vec![]),
    ];
    for (stage, artifacts) in &stages {
        let r = colm_kernel::run_stage(&kernel, *stage, &layout.case_nml(), case, artifacts)?;
        if r.succeeded() {
            println!("  {:<10} ok", stage.program());
        } else {
            eprintln!("  {:<10} FAILED: {:?}", stage.program(), r.outcome);
        }
        // CoLM 会不声不响地改掉你的配置然后继续跑 —— 失败时尤其要列，
        // 它恰恰会先改配置再死在别处。
        for o in &r.overrides {
            println!("             {}", o.text);
        }
        if !r.succeeded() {
            eprintln!("  log: {}", r.log.display());
            bail!("stage {} failed", stage.program());
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- metrics

fn cmd_metrics(case: &Path, obs_path: &Path, spinup: usize) -> Result<()> {
    let layout = Layout::new(case);
    let name = colm_case::case_name(&layout.case_nml())?;
    let hist_dir = layout.out().join(&name).join("history");
    let mut hists: Vec<PathBuf> = std::fs::read_dir(&hist_dir)
        .with_context(|| format!("no history at {}", hist_dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains("_hist_") && n.ends_with(".nc"))
        })
        .collect();
    hists.sort();
    let Some(hist) = hists.first() else {
        bail!("no *_hist_*.nc under {}", hist_dir.display());
    };

    let o_t = colm_hist::obs::read_1d(obs_path, "time")?;
    let m_t = colm_hist::obs::read_1d(hist, "time")?;
    // 观测的 time 原点是它自己记录的起始年；模型换算到同一原点
    let year = observation_year(obs_path)?;
    let m_sec = colm_hist::time::model_seconds(&m_t, year);

    println!(
        "{:<6} {:>5} {:>8} {:>9} {:>7} {:>9}",
        "var", "n", "RMSE", "bias", "R2", "KGE"
    );
    for (o_name, m_name) in colm_hist::obs::FLUX_PAIRS {
        let (Ok(o_v), Ok(o_q), Ok(m_v)) = (
            colm_hist::obs::read_1d(obs_path, o_name),
            colm_hist::obs::read_1d(obs_path, &format!("{o_name}_qc")),
            colm_hist::obs::read_1d(hist, m_name),
        ) else {
            continue; // 这一对里有一侧没有，跳过而不是报错
        };
        let s = colm_hist::pair::Series {
            seconds: &o_t,
            values: &o_v,
            qc: &o_q,
        };
        let Some(m) = colm_hist::metric::compute(&colm_hist::pair::pair(&m_sec, &m_v, &s, spinup))
        else {
            continue;
        };
        print!(
            "{o_name:<6} {:>5} {:>8.1} {:>+9.2} {:>7.3} {:>+9.3}",
            m.n, m.rmse, m.bias, m.r2, m.kge
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
    Ok(())
}

// ---------------------------------------------------------------- series

/// 把 history 里的若干条序列导出成 JSON，供 GUI 画图。
///
/// GUI 进程**不链接 netcdf** —— 让它去读 history 会把整个 HDF5 拖进窗口进程。
/// 所以数值由这里（sidecar）读出来，以 JSON 过边界。
///
/// 时间轴给的是 **Unix 秒**，因为 uPlot 的 x 轴默认就是这个。但注意
/// PLUMBER2 是**地方时**（算例里 `greenwich = .false.`），所以这些秒数是
/// 「把地方时当成 UTC」算出来的 —— 前端必须按 UTC 格式化，才会显示成
/// 站点当地的钟点。按本地时区格式化会平移一个时区。
fn cmd_series(case: &Path, vars: &str, out: Option<&str>) -> Result<()> {
    let layout = Layout::new(case);
    let name = colm_case::case_name(&layout.case_nml())?;
    let hist = newest_history(&layout.out().join(&name))?;

    let t_min = colm_hist::obs::read_1d(&hist, "time")?;
    // 换算住在 colm-hist::time —— 那个模块已经拥有「两种时间轴」这件事，
    // 而且它是 netcdf-free 的，GUI 后端将来也用得上。
    let unix = colm_hist::time::unix_seconds(&t_min);

    let mut body = String::from("{\n");
    body.push_str(&format!("  \"file\": {:?},\n", hist.display().to_string()));
    body.push_str(&format!("  \"n\": {},\n", unix.len()));
    body.push_str("  \"time\": [");
    body.push_str(
        &unix
            .iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join(","),
    );
    body.push_str("],\n  \"vars\": {\n");
    let names: Vec<&str> = vars
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if names.is_empty() {
        bail!("--vars needs at least one variable name");
    }
    let mut first = true;
    for v in &names {
        let values = colm_hist::obs::read_1d(&hist, v)
            .with_context(|| format!("{} has no variable {v}", hist.display()))?;
        if values.len() != unix.len() {
            // 剖面变量是 (time, patch, soil) 之类，长度是时间步的数倍。
            // 它们要另一种画法，本轮不做 —— 但要说清楚而不是画出一条乱线。
            bail!(
                "{v} has {} values for {} time steps; it is not a (time, patch) series",
                values.len(),
                unix.len()
            );
        }
        if !first {
            body.push_str(",\n");
        }
        first = false;
        body.push_str(&format!(
            "    {v:?}: [{}]",
            values
                .iter()
                .map(|x| format!("{x:?}"))
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    body.push_str("\n  }\n}\n");

    match out {
        Some(p) => {
            std::fs::write(p, &body)?;
            println!(
                "wrote {} series x {} points to {p}",
                names.len(),
                unix.len()
            );
        }
        None => print!("{body}"),
    }
    Ok(())
}

/// 算例里那个唯一的 `*_hist_*.nc`。
fn newest_history(out: &Path) -> Result<PathBuf> {
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
    h.sort();
    h.pop()
        .with_context(|| format!("no *_hist_*.nc under {}", dir.display()))
}

/// 观测文件 `time:units` 里的起始年。
fn observation_year(p: &Path) -> Result<i32> {
    let u = colm_hist::obs::time_units(p)?;
    let y = u
        .split("since")
        .nth(1)
        .and_then(|r| r.trim().split('-').next())
        .and_then(|y| y.trim().parse().ok())
        .with_context(|| format!("cannot read a start year out of {u:?}"))?;
    Ok(y)
}

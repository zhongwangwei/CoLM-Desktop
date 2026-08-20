//! CoLM 单点的唯一编排可执行文件。
//!
//! `design.md` §4.2：「GUI 只跟它说话」。所以这里是唯一一处同时依赖全部五层的
//! 地方，各层之间仍然互不依赖 —— 造算例的 `colm-case` 不认识内核，
//! 答「能产出什么」的 `colm-hist` 闸门表不认识 netcdf。
//!
//! ```text
//! colm-cli scan    --dir <Sitedata 目录> [--out sites.json] [--quick 1]
//! colm-cli new     --site <站点文件> --out <目录> [--name N] [--start Y-M-D] [--end Y-M-D]
//!                  [--spinup-years N] [--spinup-repeat N]
//! colm-cli run     <算例目录> --kernel <目录> [--stream 1]
//! colm-cli metrics <算例目录> --obs <Flux.nc> [--spinup N] [--json 1] [--corrected 1]
//! colm-cli series  <算例目录> --vars f_rnet,f_fsena [--out series.json]
//! colm-cli all     --site ... --out ... --kernel ... [--obs ...]
//! ```
//!
//! `--start` / `--end` 不给就用强迫场覆盖的完整范围。预热默认重复头一年
//! 10 遍，而预热期是从窗口头上扣的（输出因此少一年）。经纬度与地类读自站点
//! 文件，时间步长读自强迫场文件 —— 这三样都不问用户。

mod fingerprint;

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use colm_case::{fields, minimal::required, render, CaseSpec, Dirs, Layout, Spinup, Window};
use colm_kernel::outcome::Stage;
use colm_kernel::Kernel;

const USAGE: &str = "\
usage:
  colm-cli scan    --dir <Sitedata 目录> [--out sites.json] [--quick 1]
                   # 列出目录下的站点；--quick 跳过强迫场，只读站点文件
  colm-cli new     --site <site.nc> --out <dir> [--name N] [--start Y-M-D] [--end Y-M-D]
                   [--spinup-years N] [--spinup-repeat N]   (默认 1 年 x 10 遍)
                   [--rawdata <dir>] [--runtime <dir>]
                   # 城市站点由文件形状自动识别。两个目录都可选：预抽表盖住的
                   # 21 个 Urban-PLUMBER 站不给也能跑，表外的站点才要 --rawdata
  colm-cli run     <case-dir> --kernel <dir> [--stream 1] [--force 1]
                   # --force 忽略指纹，三段全部重跑
                   # --stream 把子进程每一行原样转发出来（GUI 用；终端下嫌吵）
  colm-cli metrics <case-dir> --obs <Flux.nc> [--spinup N] [--json 1] [--corrected 1]
                   --corrected: 拿能量闭合订正后的观测比（Qle_cor / Qh_cor）
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
        "scan" => {
            cmd_scan(
                &opts.need("--dir")?,
                opts.get("--out").as_deref(),
                opts.get("--quick").is_some(),
            )?;
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
            )?;
        }
        "metrics" => {
            let case = opts.positional_case()?;
            cmd_metrics(
                &case,
                &opts.need("--obs")?,
                opts.spinup()?,
                opts.get("--json").is_some(),
                opts.get("--corrected").is_some(),
            )?;
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
            cmd_run(
                &case,
                &opts.need("--kernel")?,
                opts.get("--stream").is_some(),
                // `all` 刚造完算例，三段都没跑过 —— 指纹本来就是空的，
                // 这里传 false 与 true 等价，写 false 以免暗示它会跳过什么。
                false,
            )?;
            match opts.get("--obs") {
                Some(obs) => cmd_metrics(&case, Path::new(&obs), opts.spinup()?, false, false)?,
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

fn cmd_scan(dir: &Path, out: Option<&str>, quick: bool) -> Result<()> {
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
        let met = sibling(&p, "Forcing", 0);
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

        sites.push(SiteInfo {
            name: short,
            site_file: text(&p),
            met_file: met.as_deref().map(text),
            obs_file: obs.as_deref().map(text),
            // 地类读不出来时不当成城市 —— 那会让一个坏文件被送进 URBAN 路径。
            urban: problem.is_none() && landtype.is_none(),
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
    let met = sibling(&site_raw, "Forcing", 0).with_context(|| {
        format!(
            "cannot find the forcing file next to {}; expected ../Forcing/<stem>_Met.nc",
            site_raw.display()
        )
    })?;

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
    // 城市站点文件不带 `IGBP_classification` —— 那正是它的标志。
    let looks_like_plumber2 = colm_srfdata::site::location(&site_raw)?.landtype.is_some();
    // 没有 `--urban` 开关：拿一个草地站强行跑城市只会在 NCAR 属性表上越界，
    // 而一个城市站不跑城市模块也没有意义。判据完全交给站点文件的形状。
    let urban = !looks_like_plumber2;
    // 这个城市站点在不在两张预抽表里 —— 决定 `--rawdata` 缺席时该说什么。
    let mut urban_covered = false;
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
        urban_covered = rep.needs_no_rawdata();
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
        start,
        end,
        (summary.start.year, summary.start.month, summary.start.day),
        (e.year, e.month, e.day),
    )?;
    // 预热。默认重复第一年 10 遍 —— 陆面模式的土壤温湿与（开了 BGC 时的）
    // 碳库是慢变量，直接从初始场跑出来的头一段并不代表这个站点的气候态。
    //
    // **代价是输出少一年**：CoLM 的预热期是从窗口头上扣的，不是加在前面的
    // （`MOD_Hist.F90:235` 在预热期直接 RETURN）。所以周期只取一年，
    // 而不是常见的"重复整段"—— PLUMBER2 里最短的站点只有两年多。
    let spin_years = o.count("--spinup-years", 1)?;
    let spin_repeat = o.count("--spinup-repeat", 10)?;
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
    // **证明**没读。
    //
    // **城市算例现在也一样。** 两张预抽表合起来盖住了 mksrfdata 会去开的
    // 每一处（`soil/` 的 24 个栅格实测 122 GB，加上 `urban_type/`、
    // `urban_lai_500m/`、`lake_depth.nc`、`soil_brightness.nc`、
    // `topography.nc`、`urban/LUCY_regionid.nc`），而 `DEF_dir_runtime` 下的
    // `urban/LUCY_rawdata.nc` 是随仓库发、由 `colm-cli` 铺进算例目录的。
    //
    // 剩下的门槛只有一条：**站点在不在那 21 个里**。不在就照旧要 `--rawdata`，
    // 而且错误信息要说清楚是这个原因 —— 不是给没量过的站点编一个默认值。
    let dirs = if urban {
        let raw = match o.get("--rawdata") {
            // 给了就用。表外的站点靠它，表内的站点给了也无妨（site.nc
            // 里有的量 CoLM 不会再去开栅格）。
            Some(r) => slash(Path::new(&r)),
            // 没给而两张表都命中 —— 指向一个不存在的目录，跑通了就**证明**
            // 一个栅格都没读。与水热算例用的是同一招。
            None if urban_covered => text(&out.join("rawdata_unused/")),
            None => bail!(
                "an urban case for a site outside the pre-extracted tables needs --rawdata: \
                 {name} is not one of the 21 Urban-PLUMBER sites, so CoLM will read the \
                 global soil/, urban_type/ and urban_lai_500m/ grids for it"
            ),
        };
        let run = match o.get("--runtime") {
            Some(r) => slash(Path::new(&r)),
            // 没给就自带一份。`LUCY_rawdata.nc` 是张 37 KB 的全局区域参数表，
            // 与站点无关，所以表内表外都能这样兜住。
            None => {
                let dir = layout.runtime();
                let f = colm_srfdata::urban_runtime::stage(&dir)?;
                println!("  runtime: {} written from the built-in copy", f.display());
                slash(&dir)
            }
        };
        (raw, run)
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

/// `(年, 月, 日)` 写成 `YYYY-MM-DD`，只给报错用。
fn ymd((y, m, d): (i32, u32, u32)) -> String {
    format!("{y}-{m:02}-{d:02}")
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
    start: (i32, u32, u32),
    end: (i32, u32, u32),
    forcing_start: (i32, u32, u32),
    forcing_end: (i32, u32, u32),
) -> Result<()> {
    // 这条与强迫场无关，纯粹是窗口本身不成立。不拦的话建出来的算例
    // 窗口是空的，而空输出与「跑失败了」在界面上长得一样。
    if start > end {
        bail!(
            "--start {} 晚于 --end {}：这个窗口是空的",
            ymd(start),
            ymd(end)
        );
    }
    if start < forcing_start {
        bail!(
            "--start {} 早于强迫场的起点（{} 起）",
            ymd(start),
            ymd(forcing_start)
        );
    }
    if end > forcing_end {
        bail!(
            "--end {} 超出强迫场的覆盖范围（到 {}）",
            ymd(end),
            ymd(forcing_end)
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

// ---------------------------------------------------------------- run

/// `stream` 打开时，子进程的每一行原样即时转发到本进程的 stdout。
///
/// 默认关着是有理由的：一次 528 步的运行，`colm.x` 打 5330 行，而人在终端
/// 想看到的是那 39 行摘要。但 GUI 那边正相反 —— 它的进度条靠
/// `TIMESTEP = n | DATE = ...`，日志窗要的就是原始行，而这两样都只在
/// 子进程的 stdout 里。同一个可执行文件同时服务两个诉求不同的调用方，
/// 于是由调用方说要哪一种。
fn cmd_run(case: &Path, kernel_dir: &Path, stream: bool, force: bool) -> Result<()> {
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
    // 每段的输入指纹。**只看产物在不在是不够的** —— 改了站点文件或
    // rawdata 目录，srfdata.nc 就失效了而文件还在，跳过它等于拿旧地表数据
    // 算新算例，且没有任何迹象。见 `fingerprint.rs`。
    let kernel_id = format!(
        "{}@{}",
        kernel.manifest.preset, kernel.manifest.colm_git_sha
    );
    let mut marks = if force {
        Default::default()
    } else {
        fingerprint::load(case)
    };

    for (stage, artifacts) in &stages {
        let sname = stage.program();
        let want = fingerprint::compute(sname, &layout.case_nml(), &kernel_id)?;
        // 两个条件都要满足才跳过：指纹一致，**且**产物真的都在。
        // 只看指纹的话，手动删掉输出目录之后会跳过一段本该重跑的。
        let have_all = !artifacts.is_empty() && artifacts.iter().all(|p| p.is_file());
        let skip = have_all
            && marks
                .get(sname)
                .is_some_and(|old| fingerprint::first_difference(old, &want).is_none());
        if skip {
            if stream {
                println!("=== colm-stage {sname} skipped ===");
            }
            println!("  {sname:<10} skipped (产物齐全且输入未变)");
            continue;
        }
        // 说出**为什么**要重跑。「又跑了一遍」而不知道原因，
        // 会让人怀疑跳过功能根本没生效。
        if let Some(old) = marks.get(sname) {
            if have_all {
                if let Some(why) = fingerprint::first_difference(old, &want) {
                    println!("  {sname:<10} 需要重跑：{why}");
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
            let removed = clear_history(&out)?;
            if removed > 0 {
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
        if stream {
            println!("=== colm-stage {} begin ===", stage.program());
        }
        // 转发时**每行都 flush**。默认的行缓冲只在 stdout 连着终端时才生效；
        // GUI 拿到的是一根管道，那时缓冲变成块缓冲（8 KB），5330 行会攒成
        // 几大块一起吐出来 —— 从界面上看跟完全不转发几乎没有区别。
        let mut forward = |line: &str| {
            if stream {
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
        if stream {
            println!(
                "=== colm-stage {} {} ===",
                stage.program(),
                if r.succeeded() { "ok" } else { "failed" }
            );
        }
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
            // 失败的那一段**不记指纹**，否则下次会把一个没跑成的阶段当成
            // 「已完成且输入未变」而跳过。
            marks.remove(sname);
            let _ = fingerprint::save(case, &marks);
            bail!("stage {} failed", stage.program());
        }
        marks.insert(sname.to_string(), want);
        fingerprint::save(case, &marks)?;
    }
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
    n: usize,
    rmse: f64,
    mae: f64,
    bias: f64,
    r2: f64,
    kge: f64,
    obs_mean: f64,
    obs_sd: f64,
    beta: f64,
    /// KGE 的 β 不可信时的原因。**非空一定要显示** ——
    /// 藏起来等于给一个假指标。
    beta_warning: Option<String>,
    /// 配对之后的时刻（unix 秒），与下面两条等长
    time: Vec<i64>,
    model: Vec<f64>,
    obs: Vec<f64>,
}

fn cmd_metrics(
    case: &Path,
    obs_path: &Path,
    spinup: usize,
    json: bool,
    corrected: bool,
) -> Result<()> {
    let layout = Layout::new(case);
    let name = colm_case::case_name(&layout.case_nml())?;
    let hists = history_files(&layout.out().join(&name))?;

    let o_t = colm_hist::obs::read_1d(obs_path, "time")?;
    let m_t = read_history(&hists, "time")?;
    // 观测的 time 原点是它自己记录的起始年；模型换算到同一原点
    let year = observation_year(obs_path)?;
    let m_sec = colm_hist::time::model_seconds(&m_t, year);

    if !json {
        println!(
            "{:<10} {:>7} {:>8} {:>9} {:>7} {:>9}",
            "obs var", "n", "RMSE", "bias", "R2", "KGE"
        );
    }
    let mut rows: Vec<VarMetrics> = Vec::new();
    for (o_name, m_name) in colm_hist::obs::FLUX_PAIRS {
        // 订正版没有自己的 qc 变量（文件里只有 `Qle_cor_uc_qc`，那是不确定度的），
        // 所以质量控制一律用原始通量那一个：它说的是"这一步是实测还是插补"，
        // 而订正只改数值不改这件事。
        let o_var = corrected
            .then(|| colm_hist::obs::corrected(o_name))
            .flatten()
            .filter(|c| colm_hist::obs::read_1d(obs_path, c).is_ok())
            .unwrap_or(o_name);
        let (Ok(o_v), Ok(o_q), Ok(m_v)) = (
            colm_hist::obs::read_1d(obs_path, o_var),
            colm_hist::obs::read_1d(obs_path, &format!("{o_name}_qc")),
            read_history(&hists, m_name),
        ) else {
            continue; // 这一对里有一侧没有，跳过而不是报错
        };
        let s = colm_hist::pair::Series {
            seconds: &o_t,
            values: &o_v,
            qc: &o_q,
        };
        let with_time = colm_hist::pair::pair_with_time(&m_sec, &m_v, &s, spinup);
        let pairs: Vec<(f64, f64)> = with_time.iter().map(|(_, a, b)| (*a, *b)).collect();
        let Some(m) = colm_hist::metric::compute(&pairs) else {
            continue;
        };
        if json {
            // 模型时间轴换算成 unix 秒，与 `series` 那条一致 ——
            // 界面把两者画在同一张图上，原点不同就对不齐。
            let unix = colm_hist::time::unix_seconds(&m_t);
            let by_sec: std::collections::BTreeMap<i64, i64> = m_sec
                .iter()
                .zip(unix.iter())
                .map(|(s, u)| (*s as i64, *u))
                .collect();
            rows.push(VarMetrics {
                name: o_name.to_string(),
                obs_var: o_var.to_string(),
                model_var: m_name.to_string(),
                n: m.n,
                rmse: m.rmse,
                mae: m.mae,
                bias: m.bias,
                r2: m.r2,
                kge: m.kge,
                obs_mean: m.obs_mean,
                obs_sd: m.obs_sd,
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
                time: with_time
                    .iter()
                    .map(|(s, _, _)| by_sec.get(&(*s as i64)).copied().unwrap_or_default())
                    .collect(),
                model: with_time.iter().map(|(_, a, _)| *a).collect(),
                obs: with_time.iter().map(|(_, _, b)| *b).collect(),
            });
            continue;
        }
        print!(
            "{o_var:<10} {:>7} {:>8.1} {:>+9.2} {:>7.3} {:>+9.3}",
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
    if json {
        println!("{}", serde_json::to_string(&rows)?);
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
    let hists = history_files(&layout.out().join(&name))?;

    let t_min = read_history(&hists, "time")?;
    // 换算住在 colm-hist::time —— 那个模块已经拥有「两种时间轴」这件事，
    // 而且它是 netcdf-free 的，GUI 后端将来也用得上。
    let unix = colm_hist::time::unix_seconds(&t_min);

    let mut body = String::from("{\n");
    // 文件数而不是文件名：一条曲线现在跨着 132 个文件，报其中一个的名字
    // 只会让人以为看到的就是那一个月。
    body.push_str(&format!("  \"files\": {},\n", hists.len()));
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
        let values = read_history(&hists, v)?;
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
    let mut out = Vec::new();
    for f in files {
        let v = colm_hist::obs::read_1d(f, var)
            .with_context(|| format!("{} has no variable {var}", f.display()))?;
        out.extend(v);
    }
    if var == "time" {
        check_increasing(&out).with_context(|| {
            format!(
                "the history files in {} do not concatenate into an increasing time axis; sorting by file name is not giving chronological order here",
                files[0].parent().unwrap_or(Path::new(".")).display()
            )
        })?;
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

#[cfg(test)]
#[path = "history_tests.rs"]
mod history_tests;

#[cfg(test)]
#[path = "window_tests.rs"]
mod window_tests;

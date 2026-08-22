//! CoLM 单点的唯一编排可执行文件。
//!
//! `design.md` §4.2：「GUI 只跟它说话」。所以这里是唯一一处同时依赖全部五层的
//! 地方，各层之间仍然互不依赖 —— 造算例的 `colm-case` 不认识内核，
//! 答「能产出什么」的 `colm-hist` 闸门表不认识 netcdf。
//!
//! ```text
//! colm-cli scan      --dir <Sitedata 目录> [--forcing-dir <Forcing 目录>]
//!                    [--out sites.json] [--quick 1]
//! colm-cli site-new  --out <site.nc> --lon <度> --lat <度> [--landtype N] [--rawdata <目录>]
//! colm-cli new       --site <站点文件> --out <目录> [--name N] [--start Y-M-D] [--end Y-M-D]
//!                    [--spinup-years N] [--spinup-repeat N]
//! colm-cli run       <算例目录> --kernel <目录> [--stream 1]
//!                    [--stage mksrfdata|mkinidata|colm]
//! colm-cli metrics   <算例目录> --obs <Flux.nc> [--spinup N] [--json 1] [--corrected 1]
//!                    [--summary-only 1] [--pairs-var Rnet] [--max-points N]
//! colm-cli history-catalog <算例目录>
//! colm-cli series    <算例目录> --vars f_rnet,f_fsena [--max-points N] [--out series.json]
//! colm-cli all       --site ... --out ... --kernel ... [--obs ...]
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
  colm-cli scan    --dir <Sitedata 目录> [--forcing-dir <Forcing 目录>]
                   [--out sites.json] [--quick 1]
                   # 列出目录下的站点；--quick 跳过强迫场，只读站点文件
  colm-cli site-new --out <site.nc> --lon <度> --lat <度> [--landtype N]
                   [--rawdata <dir>] [--json 1]
                   # 建一份站点文件：经纬度必给，其余从 rawdata 抽或用
                   # 标称假设。--landtype 不给就不写，让 CoLM 回落
  colm-cli new     --site <site.nc> --out <dir> [--name N] [--start Y-M-D] [--end Y-M-D]
                   [--met <Met.nc>]   # 前处理转出来的强迫场；不给就按命名约定
                                      # 在 ../Forcing/ 下找，那两套约定只覆盖
                                      # PLUMBER2 与 Urban-PLUMBER
                   [--spinup-years N] [--spinup-repeat N]   (默认 1 年 x 10 遍)
                   [--rawdata <dir>] [--runtime <dir>]
                   # 城市站点由文件形状自动识别。两个目录都可选：预抽表盖住的
                   # 21 个 Urban-PLUMBER 站不给也能跑，表外的站点才要 --rawdata
  colm-cli run     <case-dir> --kernel <dir> [--stream 1] [--force 1]
                   [--stage mksrfdata|mkinidata|colm]
                   # --force 忽略指纹，三段全部重跑
                   # --stage 只运行指定阶段；与 --force 合用时强制重跑该阶段
                   # --stream 把子进程每一行原样转发出来（GUI 用；终端下嫌吵）
  colm-cli metrics <case-dir> --obs <Flux.nc> [--spinup N] [--json 1] [--corrected 1]
                   --corrected: 拿能量闭合订正后的观测比（Qle_cor / Qh_cor）
                   --summary-only: 只返回指标，不携带绘图用配对点
                   --pairs-var: 只返回指定观测变量及其配对点
                   --max-points: 配对点保极值降采样上限（指标仍用完整样本）
  colm-cli history-catalog <case-dir>
  colm-cli series  <case-dir> --vars f_rnet,f_fsena [--from UNIX] [--to UNIX]
                   [--max-points N] [--out series.json]
  colm-cli all     --site <site.nc> --out <dir> --kernel <dir> [--obs <Flux.nc>] [--name N]
                   [--start Y-M-D] [--end Y-M-D] [--spinup N]
  colm-cli forcing-probe   <met.nc> [--json 1]
  colm-cli netcdf-probe    <data.nc> [--json 1]
                           # 探测一份强迫场文件：八个槽位各猜到了什么变量，
                           # 猜不到就是 null；三个观测高度缺失时也是 null，
                           # 不是 NaN —— GUI 前处理页据此决定问不问用户
  colm-cli forcing-convert <src.nc> <dst.nc> [--slot N=name:units[+extra] ...] [--height V,T,Q]
                           # 与独立 bin forcing-convert 同样的行为，供 GUI 走
                           # sidecar 调用；没给 --slot 的槽位走自动匹配
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
                opts.get("--forcing-dir").as_deref().map(Path::new),
                opts.get("--out").as_deref(),
                opts.get("--quick").is_some(),
            )?;
        }
        "site-new" => {
            cmd_site_new(&opts)?;
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
            let pair_var = opts.get("--pairs-var");
            cmd_metrics(MetricsRequest {
                case: &case,
                obs_path: &obs_path,
                spinup: opts.spinup()?,
                json: opts.get("--json").is_some(),
                corrected: opts.get("--corrected").is_some(),
                summary_only: opts.get("--summary-only").is_some(),
                pair_var: pair_var.as_deref(),
                pair_max_points: opts.get("--max-points").map(|v| v.parse()).transpose()?,
            })?;
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
                    pair_var: None,
                    pair_max_points: None,
                })?,
                None => println!("no --obs given; skipping the metrics table"),
            }
        }
        "forcing-probe" => {
            cmd_forcing_probe(
                &opts.positional_at(0, "a forcing file")?,
                opts.get("--json").is_some(),
            )?;
        }
        "netcdf-probe" => {
            cmd_netcdf_probe(
                &opts.positional_at(0, "a NetCDF file")?,
                opts.get("--json").is_some(),
            )?;
        }
        "forcing-convert" => {
            cmd_forcing_convert(
                &opts.positional_at(0, "a source forcing file")?,
                &opts.positional_at(1, "a destination file")?,
                &opts.get_all("--slot"),
                opts.get("--height").as_deref(),
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

/// `colm-cli site-new`：从一对经纬度建一份能跑的 site.nc。
///
/// 两步拼起来：[`colm_srfdata::site::skeleton`] 写出只有经纬度（可选地类）
/// 的最小文件，[`colm_srfdata::site::fill`] 照阶段 B 的三级优先级
/// （站点自有 > 栅格 > 模块默认）把 12 个必需字段补齐。中间那份 skeleton
/// 文件放系统临时目录，补完即删——用户只关心 `--out` 那一份。
///
/// 没有 `--rawdata` 时 12 个字段全部落到模块默认或本 crate 自己发明的
/// 标称假设；文件依然能跑，只是土壤/地形/反照率不是这个地点的实测值，
/// 输出里的 `from default:` 那一行会把它们逐个点名。
fn cmd_site_new(o: &Opts) -> Result<PathBuf> {
    let out = o.need("--out")?;
    let lon = o.need_f64("--lon")?;
    let lat = o.need_f64("--lat")?;
    let landtype = o.get_i32("--landtype")?;
    let rawdata = o.get("--rawdata");

    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }

    // skeleton 与 fill 之间的中间文件：用户不该知道它存在过。
    let skel = std::env::temp_dir().join(format!("colm-site-new-{}.nc", std::process::id()));
    colm_srfdata::site::skeleton(&skel, lon, lat, landtype)?;
    let filled = colm_srfdata::site::fill(&skel, &out, rawdata.as_deref().map(Path::new), None);
    let _ = std::fs::remove_file(&skel);
    let r = filled?;

    // 冠层高度查得到（`r.from_lookup` 里有 `canopy_height`）就不用外部数据；
    // 查不到（没给 `--landtype`，或给的不是有效 IGBP 类别）就还得靠
    // `<rawdata>/plant_15s/`。LAI/SAI 这个 crate 从不合成 —— `site-new` 造出
    // 来的文件里永远没有月气候态，所以它俩永远在这张单子上。
    // `SAI_monthly` 与 `LAI_monthly` 是 mksrfdata 绑定读取的一对
    // （`MOD_SingleSrfdata.F90:505-506`：缺一个另一个也作废），所以两个都要
    // 列出来，不能只写 LAI。
    let canopy_height_ready = r.from_lookup.iter().any(|n| n == "canopy_height");
    let mut needs_external: Vec<&str> = Vec::new();
    if !canopy_height_ready {
        needs_external.push("canopy_height");
    }
    needs_external.push("LAI_monthly");
    needs_external.push("SAI_monthly");

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
            "needs_external": needs_external,
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
            "from lookup : {}  <-- CoLM's IGBP table, not measured at this site",
            r.from_lookup.join(", ")
        );
    }
    println!("wrote {}", out.display());
    Ok(out)
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
        return Ok(p);
    }
    sibling(site, "Forcing", 0).with_context(|| {
        format!(
            "cannot find the forcing file next to {}; expected ../Forcing/<stem>_Met.nc \
             (or pass --met <path> to name it directly)",
            site.display()
        )
    })
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
    // 城市站点文件不带 `IGBP_classification` —— 那正是它的标志。**但**
    // `colm-cli site-new` 造出来的最小文件在用户不给 `--landtype` 时也没有
    // 它，而它绝不是城市站点：`site::skeleton` + `site::fill` 已经把 12 个
    // 必需字段都写好了，真正未处理的 Urban-PLUMBER 原始文件这时一个都没有
    // （那正是 `prepare_urban` 存在的理由）。所以先看这一条更硬的证据——
    // 地类缺席只在文件确实还缺这 12 个字段时，才说明它是城市站点。
    let already_filled = colm_srfdata::site::missing_fields(&site_raw)?.is_empty();
    // **但「已补齐」不等于「不是城市站」。** 一个已经建过算例的城市
    // `site.nc` 同样是 12/12 齐全的（`prepare_urban` 补的），把它重新喂回来
    // 会被当成 PLUMBER2，于是 `SITE_landtype = 13` 与
    // `DEF_URBAN_type_scheme = 2` 一个都不写 —— 站点文件里有城市数据、
    // namelist 却不跑城市模块，**跑得完，结果全错**。
    //
    // 实测过：改判据之前那条路会撞 `NC_ENAMEINUSE` 而失败，加了
    // `already_filled` 之后变成静默产出错误配置。**从报错退化成静默错误。**
    //
    // `LCZ_DOM`（局地气候区分类）只有城市路径写，拿它当第二条证据。
    let has_urban_marker = netcdf::open(&site_raw)
        .ok()
        .is_some_and(|f| f.variable("LCZ_DOM").is_some());
    let looks_like_plumber2 = !has_urban_marker
        && (already_filled || colm_srfdata::site::location(&site_raw)?.landtype.is_some());
    // 没有 `--urban` 开关：拿一个草地站强行跑城市只会在 NCAR 属性表上越界，
    // 而一个城市站不跑城市模块也没有意义。判据完全交给站点文件的形状。
    let urban = !looks_like_plumber2;
    // 这个城市站点在不在两张预抽表里 —— 决定 `--rawdata` 缺席时该说什么。
    let mut urban_covered = false;
    if already_filled {
        // 已经补齐过的文件——`site-new` 的产物，或者重新喂回来的一份旧
        // 增广站点文件。原样拷过去，不再调用 `fill`：它的第一行就是
        // `fs::copy`，第二行会在已经存在的变量名上报 `NC_ENAMEINUSE`。
        std::fs::copy(&site_raw, layout.site_nc()).with_context(|| {
            format!(
                "cannot copy {} to {}",
                site_raw.display(),
                layout.site_nc().display()
            )
        })?;
        println!("site: already has all 12 required fields (from site-new or a prior fill); copied as-is");
        // **已补齐的城市站点文件不必再查预抽表。** 那张表回答的是
        // 「这个站点的城市数据能不能不靠 `--rawdata` 拿到」，而这份文件
        // 里已经有了 —— `prepare_urban` 上一次建算例时就写进去了。
        //
        // 不置这一位的话，重新喂回来的城市 `site.nc` 会因为算例名不在
        // 那 21 个站里而被要求 `--rawdata`，尽管它一个字节都不需要。
        urban_covered = true;
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

fn cmd_run(
    case: &Path,
    kernel_dir: &Path,
    stream: bool,
    force: bool,
    only_stage: Option<Stage>,
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
    pair_var: Option<&'a str>,
    pair_max_points: Option<usize>,
}

fn cmd_metrics(request: MetricsRequest<'_>) -> Result<()> {
    let MetricsRequest {
        case,
        obs_path,
        spinup,
        json,
        corrected,
        summary_only,
        pair_var,
        pair_max_points,
    } = request;
    let layout = Layout::new(case);
    let name = colm_case::case_name(&layout.case_nml())?;
    let hists = history_files(&layout.out().join(&name))?;

    // Open every NetCDF file once. The previous implementation reopened every one
    // of 132 monthly history files for each flux, turning an AT-Neu evaluation into
    // minutes of native-I/O setup rather than numerical work.
    let obs_file =
        netcdf::open(obs_path).with_context(|| format!("cannot open {}", obs_path.display()))?;
    let o_t = read_file_1d(&obs_file, obs_path, "time")?;
    let first_history =
        netcdf::open(&hists[0]).with_context(|| format!("cannot open {}", hists[0].display()))?;
    let available_model = colm_hist::obs::FLUX_PAIRS
        .iter()
        .filter(|(observation, _)| pair_var.is_none_or(|wanted| *observation == wanted))
        .map(|(_, model)| *model)
        .filter(|model| first_history.variable(model).is_some())
        .collect::<Vec<_>>();
    drop(first_history);
    let mut wanted = vec!["time"];
    wanted.extend(available_model);
    let mut model_data = read_history_many(&hists, &wanted)?;
    let m_t = model_data.remove("time").expect("time was requested");
    // 观测的 time 原点可能在年中（AU-Preston 是 2003-08-12 03:30），
    // 必须按完整日期时间换算；只取年份会把序列错配几个月。
    let units = variable_units(&obs_file, "time")
        .with_context(|| format!("time:units in {} is not a string", obs_path.display()))?;
    let m_sec = colm_hist::time::model_seconds_from_units(&m_t, &units)
        .with_context(|| format!("unsupported observation time units {units:?}"))?;
    let by_sec = if json && !summary_only {
        let unix = colm_hist::time::unix_seconds(&m_t);
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
    for (o_name, m_name) in colm_hist::obs::FLUX_PAIRS {
        if pair_var.is_some_and(|wanted| wanted != o_name) {
            continue;
        }
        // 订正版没有自己的 qc 变量（文件里只有 `Qle_cor_uc_qc`，那是不确定度的），
        // 所以质量控制一律用原始通量那一个：它说的是"这一步是实测还是插补"，
        // 而订正只改数值不改这件事。
        let o_var = corrected
            .then(|| colm_hist::obs::corrected(o_name))
            .flatten()
            .filter(|candidate| obs_file.variable(candidate).is_some())
            .unwrap_or(o_name);
        let (Ok(o_v), Ok(o_q), Some(m_v)) = (
            read_file_1d(&obs_file, obs_path, o_var),
            read_file_1d(&obs_file, obs_path, &format!("{o_name}_qc")),
            model_data.get(m_name),
        ) else {
            continue; // 这一对里有一侧没有，跳过而不是报错
        };
        let s = colm_hist::pair::Series {
            seconds: &o_t,
            values: &o_v,
            qc: &o_q,
        };
        let with_time = colm_hist::pair::pair_with_time(&m_sec, m_v, &s, spinup);
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
                obs_var: o_var.to_string(),
                model_var: m_name.to_string(),
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

fn cmd_history_catalog(case: &Path) -> Result<()> {
    let layout = Layout::new(case);
    let name = colm_case::case_name(&layout.case_nml())?;
    let hists = history_files(&layout.out().join(&name))?;
    let time = read_history(&hists, "time")?;
    let unix = colm_hist::time::unix_seconds(&time);
    let file =
        netcdf::open(&hists[0]).with_context(|| format!("cannot open {}", hists[0].display()))?;
    let mut variables = file
        .variables()
        .map(|variable| {
            let dimensions: Vec<(String, usize)> = variable
                .dimensions()
                .iter()
                .map(|dimension| (dimension.name(), dimension.len()))
                .collect();
            HistoryVariable {
                name: variable.name(),
                units: variable_units(&file, &variable.name()),
                kind: history_kind(&dimensions),
                dimensions: dimensions
                    .into_iter()
                    .map(|(name, len)| DimensionShape { name, len })
                    .collect(),
            }
        })
        .collect::<Vec<_>>();
    variables.sort_by(|a, b| a.name.cmp(&b.name));
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
            _ => None,
        })
}

fn read_file_1d(f: &netcdf::File, path: &Path, name: &str) -> Result<Vec<f64>> {
    let variable = f
        .variable(name)
        .with_context(|| format!("{} has no variable {name}", path.display()))?;
    variable
        .get_values::<f64, _>(..)
        .with_context(|| format!("cannot read {name} from {}", path.display()))
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

/// `NaN` 不能进 JSON（`serde_json` 序列化会报错或写出不可靠的 `null`），
/// 所以在这里显式转成 `Option`，交给 `serde` 序列化。
fn present(x: f64) -> Option<f64> {
    if x.is_nan() {
        None
    } else {
        Some(x)
    }
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

/// 与独立 bin `forcing-convert` 同样的行为：没被 `--slot` 指定的槽位走
/// 自动匹配，带缺测的变量拦在入口，再转换。**`--slot`/`--height` 的解析
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
    for (i, slot) in colm_forcing::SLOTS.iter().enumerate() {
        if given.iter().any(|s| s.index == slot.index) {
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
            let fill = match v.attribute_value("_FillValue").and_then(|r| r.ok()) {
                Some(netcdf::AttributeValue::Float(x)) => f64::from(x),
                Some(netcdf::AttributeValue::Double(x)) => x,
                _ => continue,
            };
            let vals: Vec<f64> = v.get_values(netcdf::Extents::All)?;
            let n = vals.iter().filter(|x| (**x - fill).abs() < 1e-6).count();
            if n > 0 {
                bail!(
                    "{name} has {n} missing value(s) (_FillValue = {fill}); \
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

/// Read several scalar history variables while opening each monthly file once.
/// NetCDF/HDF5 open/close dominates multi-year evaluation; grouping by file makes
/// batch analysis scale with months rather than `months × variables`.
fn read_history_many(
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
#[path = "history_tests.rs"]
mod history_tests;

#[cfg(test)]
#[path = "run_stage_tests.rs"]
mod run_stage_tests;

#[cfg(test)]
#[path = "window_tests.rs"]
mod window_tests;

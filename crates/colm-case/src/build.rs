//! 从「一个站点 + 一个时间窗口 + 一份目录布局」造出算例的字段集合。
//!
//! 输出是有序的 `(路径, 值)` 列表，交给 `minimal::required` 过滤之后再序列化。
//! 顺序固定，否则每次重生成都是一个大 diff。

use colm_namelist::Value;

/// 造一个算例需要知道的全部东西。
///
/// 21 个必写字段里，只有 `name` 与 `window` 真正需要人来定；其余要么读自
/// 站点文件（位置与地类），要么由强迫场算出（可用的时间范围），
/// 要么由目录布局决定（四个路径），要么属于预设。
pub struct CaseSpec {
    pub name: String,
    /// 补齐之后的站点文件路径，写进 `SITE_fsitedata`
    pub site_file: String,
    pub lon: f64,
    pub lat: f64,
    /// IGBP 地类。`None` 表示站点文件没说 —— 城市站点文件都不带它，
    /// 而 URBAN 路径反正会强制成 13。那时两个 landtype 字段都不写，
    /// 让 CoLM 自己从文件或栅格去定。
    pub landtype: Option<i32>,
    pub window: Window,
    /// 强迫场文件的时间步长（秒）。实测 88/90 个站点是 1800，
    /// `US-Ne3` 与 `US-MMS` 是 3600 —— 它必须跟着走。
    pub timestep_seconds: f64,
    /// 时间轴是不是格林尼治时。**必须由强迫场文件说了算**，不能写死：
    /// PLUMBER2 是地方时、Urban-PLUMBER 是 UTC，而 design.md §2.8 量过
    /// 时区错 8 小时会把 Rnet 的 R² 从 0.986 打到 0.146 —— 跑得完，全错。
    /// 见 `colm_forcing::MetSummary::is_greenwich`。
    pub greenwich: bool,
    /// 这个算例跑不跑城市模块。
    ///
    /// 不是"要不要加几个字段"那么简单：URBAN 预设会把地类**强制**成 13
    /// （`MOD_SingleSrfdata.F90:1548`），所以它只能跑城市站点 —— 拿一个
    /// 草地站去跑会在 NCAR 属性表上越界（那里没有 `utyp >= 1` 的守卫，
    /// 崩溃只是因为构建开了 `-fcheck=all`）。
    pub urban: bool,
    pub dirs: Dirs,
}

#[derive(Debug, Clone, Copy)]
pub struct Window {
    pub start_year: i32,
    pub start_month: u32,
    pub start_day: u32,
    pub end_year: i32,
    pub end_month: u32,
    pub end_day: u32,
}

pub struct Dirs {
    pub rawdata: String,
    pub runtime: String,
    pub output: String,
    pub forcing_namelist: String,
}

/// 造出字段集合。**不做过滤** —— 过滤是 `minimal::required` 的事，
/// 分开是为了让「本来会写什么」与「实际写了什么」都能被看到。
pub fn fields(s: &CaseSpec) -> Vec<(String, Value)> {
    let r = |x: f64| Value::Real {
        text: format!("{x:?}"),
    };
    let mut out = vec![
        ("DEF_CASE_NAME".into(), Value::Str(s.name.clone())),
        // ---- 站点身份 ----
        ("SITE_fsitedata".into(), Value::Str(s.site_file.clone())),
        ("SITE_lon_location".into(), r(s.lon)),
        ("SITE_lat_location".into(), r(s.lat)),
        // SinglePoint 是唯一允许非格林尼治时的配置
        // （MOD_TimeManager.F90:74-79 的强制覆盖在 #ifndef SinglePoint 内），
        // 取值由强迫场文件决定，不写死。
        (
            "DEF_simulation_time%greenwich".into(),
            Value::Bool(s.greenwich),
        ),
        // ---- 时间窗口 ----
        (
            "DEF_simulation_time%start_year".into(),
            Value::Int(s.window.start_year as i64),
        ),
        (
            "DEF_simulation_time%start_month".into(),
            Value::Int(s.window.start_month as i64),
        ),
        (
            "DEF_simulation_time%start_day".into(),
            Value::Int(s.window.start_day as i64),
        ),
        ("DEF_simulation_time%start_sec".into(), Value::Int(0)),
        (
            "DEF_simulation_time%end_year".into(),
            Value::Int(s.window.end_year as i64),
        ),
        (
            "DEF_simulation_time%end_month".into(),
            Value::Int(s.window.end_month as i64),
        ),
        (
            "DEF_simulation_time%end_day".into(),
            Value::Int(s.window.end_day as i64),
        ),
        ("DEF_simulation_time%end_sec".into(), Value::Int(86400)),
        ("DEF_simulation_time%timestep".into(), r(s.timestep_seconds)),
        // spin-up 关掉：这三项的默认值不是「不做 spin-up」，必须显式写。
        ("DEF_simulation_time%spinup_day".into(), Value::Int(365)),
        ("DEF_simulation_time%spinup_sec".into(), Value::Int(86400)),
        ("DEF_simulation_time%spinup_repeat".into(), Value::Int(0)),
        // ---- 路径 ----
        ("DEF_dir_rawdata".into(), Value::Str(s.dirs.rawdata.clone())),
        ("DEF_dir_runtime".into(), Value::Str(s.dirs.runtime.clone())),
        ("DEF_dir_output".into(), Value::Str(s.dirs.output.clone())),
        (
            "DEF_forcing_namelist".into(),
            Value::Str(s.dirs.forcing_namelist.clone()),
        ),
        // ---- 预设级 ----
        // 臭氧是本项目唯一必须显式关掉的默认开关：CoLM 默认 .true.，
        // 要读 2.8 GB 的 Ozone/Global/OZONE-setgrid.nc。关掉之后
        // MOD_Ozone.F90:83 用常数 100 ppbv，臭氧胁迫仍生效。见 design.md §2.7。
        ("DEF_USE_OZONEDATA".into(), Value::Bool(false)),
        ("DEF_WRST_FREQ".into(), Value::Str("MONTHLY".into())),
        ("DEF_HIST_FREQ".into(), Value::Str("HOURLY".into())),
    ];
    // 地类只在站点文件说得出时才写。说不出就整条不写 —— 写一个猜的值
    // 比不写更糟，而 CoLM 有自己的回落路径（站点文件的分类变量，或栅格）。
    //
    // 城市算例是例外：地类由 URBAN 路径强制成 13，而城市站点文件都不带
    // `IGBP_classification`，所以这里显式写出来 —— 让配置文件说出实际会发生
    // 的事，而不是让人读源码才知道。
    let landtype = if s.urban {
        Some(URBAN_LANDTYPE)
    } else {
        s.landtype
    };
    if let Some(lt) = landtype {
        out.insert(4, ("SITE_landtype".into(), Value::Int(lt as i64)));
        out.insert(5, ("USE_SITE_landtype".into(), Value::Bool(true)));
    }
    if s.urban {
        // LCZ（局地气候区）方案，不是默认的 1（NCAR 城市密度分类）。
        // 实测默认那条路在栅格给不出城市类别时会越界崩溃，而 LCZ 分类
        // 的覆盖更完整。CoLM 自带的城市单点示例用的也是 2。
        out.push(("DEF_URBAN_type_scheme".into(), Value::Int(2)));
        // 这三项默认是 .true.（「站点文件里有，用它」），可城市站点文件里
        // 没有 —— Urban-PLUMBER 的 25 个变量全是形态学量，一个土壤剖面、
        // 一个湖深、一个土壤反照率都没有。留着默认值，CoLM 会去站点文件里
        // 找不存在的变量。改成 .false. 之后它改从栅格取，这也是城市算例
        // **必须**给出真实 rawdata 目录的原因（见 `Dirs::rawdata`）。
        for n in ["lakedepth", "soilreflectance", "soilparameters"] {
            out.push((format!("USE_SITE_{n}"), Value::Bool(false)));
        }
    }
    out
}

/// IGBP 的「城市与建成区」。URBAN 路径会把地类强制成它。
pub const URBAN_LANDTYPE: i32 = 13;

#[cfg(test)]
#[path = "build_tests.rs"]
mod build_tests;

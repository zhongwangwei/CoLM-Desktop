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
    /// 当前地类体系里的地类号。`None` 表示站点文件没说；非城市算例不写，
    /// 城市算例回落到 IGBP/PFT/PC 的城市类号。
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
    /// 预热。见 [`Spinup`]。
    pub spinup: Spinup,
    pub dirs: Dirs,
}

/// 预热：把窗口开头那几年反复跑几遍，让土壤温湿等慢变量趋于平衡。
///
/// **预热是从输出里扣掉的，不是加在前面的。** CoLM 跑完最后一遍之后从
/// 预热截止时刻接着往下跑到 end（`CoLM.F90:673`），而 `MOD_Hist.F90:235`
/// 在 `itstamp <= ptstamp` 时直接 RETURN —— history 从预热截止时刻才开始。
/// 预热一年，输出就少一年。这一条决定了默认值只敢取一年：
/// PLUMBER2 里最短的站点只有两年多，再多就没有输出了。
#[derive(Debug, Clone, Copy)]
pub struct Spinup {
    /// 预热周期的长度，单位年。截止时刻 = 起始时刻 + 这么多年。
    pub years: u32,
    /// 跑几遍。0 表示界面主动关闭；CoLM 自己会把手写的 0 提成 1。
    pub repeat: u32,
}

impl Spinup {
    /// 不预热。
    pub const OFF: Spinup = Spinup {
        years: 0,
        repeat: 0,
    };

    pub fn is_on(&self) -> bool {
        self.years > 0 && self.repeat > 0
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Window {
    pub start_year: i32,
    pub start_month: u32,
    pub start_day: u32,
    /// 起始当天的秒数，必须跟强迫场第一条记录一致或更晚。
    pub start_sec: u32,
    pub end_year: i32,
    pub end_month: u32,
    pub end_day: u32,
    /// 结束那一天跑到第几秒。
    ///
    /// **不能写死 86400。** 强迫场的最后一条记录未必落在当天最后一步上 ——
    /// 实测 PLUMBER2 的 AT-Neu 末尾是 `2013 001 1800`，而写死 86400 会让
    /// CoLM 在跑到那里时报 `Forcing does not cover simulation period!`
    /// 并中止，而那已经是三段里最后一段跑了一半的时候。
    pub end_sec: u32,
}

pub struct Dirs {
    pub rawdata: String,
    pub runtime: String,
    pub output: String,
    pub forcing_namelist: String,
}

/// 空间算例的网格合同。三种模式都仍由同一份 `case.nml` 驱动；这里不写
/// `SITE_*`，避免把流域、区域或全球算例伪装成单点站。
pub enum SpatialGrid {
    LatLon {
        mesh_file: String,
        dlon: f64,
        dlat: f64,
    },
    Unstructured {
        mesh_file: String,
    },
    Catchment {
        mesh_file: String,
    },
}

pub struct SpatialCaseSpec {
    pub name: String,
    pub grid: SpatialGrid,
    pub window: Window,
    pub timestep_seconds: f64,
    pub dirs: Dirs,
    pub domain: SpatialBounds,
}

pub struct SpatialBounds {
    pub west: f64,
    pub east: f64,
    pub south: f64,
    pub north: f64,
}

/// 生成流域、区域或全球算例所需的最小字段集合。
pub fn spatial_fields(s: &SpatialCaseSpec) -> Vec<(String, Value)> {
    let r = |x: f64| Value::Real {
        text: format!("{x:?}"),
    };
    let mut out = vec![
        ("DEF_CASE_NAME".into(), Value::Str(s.name.clone())),
        ("DEF_simulation_time%greenwich".into(), Value::Bool(true)),
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
        (
            "DEF_simulation_time%start_sec".into(),
            Value::Int(s.window.start_sec as i64),
        ),
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
        (
            "DEF_simulation_time%end_sec".into(),
            Value::Int(s.window.end_sec as i64),
        ),
        ("DEF_simulation_time%timestep".into(), r(s.timestep_seconds)),
        ("DEF_dir_rawdata".into(), Value::Str(s.dirs.rawdata.clone())),
        ("DEF_dir_runtime".into(), Value::Str(s.dirs.runtime.clone())),
        ("DEF_dir_output".into(), Value::Str(s.dirs.output.clone())),
        (
            "DEF_forcing_namelist".into(),
            Value::Str(s.dirs.forcing_namelist.clone()),
        ),
        ("DEF_USE_OZONESTRESS".into(), Value::Bool(false)),
        ("DEF_USE_OZONEDATA".into(), Value::Bool(false)),
        ("DEF_WRST_FREQ".into(), Value::Str("MONTHLY".into())),
        ("DEF_HIST_FREQ".into(), Value::Str("HOURLY".into())),
        ("DEF_domain%edgew".into(), r(s.domain.west)),
        ("DEF_domain%edgee".into(), r(s.domain.east)),
        ("DEF_domain%edges".into(), r(s.domain.south)),
        ("DEF_domain%edgen".into(), r(s.domain.north)),
    ];
    out.extend(spinup_fields(
        (
            s.window.start_year,
            s.window.start_month,
            s.window.start_day,
            s.window.start_sec,
        ),
        Spinup::OFF,
    ));
    match &s.grid {
        SpatialGrid::LatLon {
            mesh_file,
            dlon,
            dlat,
        } => out.extend([
            ("DEF_file_mesh".into(), Value::Str(mesh_file.clone())),
            ("DEF_GRIDBASED_lon_res".into(), r(*dlon)),
            ("DEF_GRIDBASED_lat_res".into(), r(*dlat)),
        ]),
        SpatialGrid::Unstructured { mesh_file } => {
            out.push(("DEF_file_mesh".into(), Value::Str(mesh_file.clone())))
        }
        SpatialGrid::Catchment { mesh_file } => out.push((
            "DEF_CatchmentMesh_data".into(),
            Value::Str(mesh_file.clone()),
        )),
    }
    out
}

/// 预热那五项，按写进 namelist 的顺序。
///
/// `start` 是模拟窗口的起始时刻 —— 预热截止时刻是**它加上若干年**，
/// 月日秒照抄。只改年而让其余部分留在 CoLM 的默认值上，
/// 会让截止时刻落在窗口之外，而窗口未必从 1 月 1 日开始。
pub fn spinup_fields(start: (i32, u32, u32, u32), sp: Spinup) -> Vec<(String, Value)> {
    // 关掉时写 year=0：`is_spinup = (ststamp < ptstamp)`（`CoLM.F90:300`），
    // 0 年早于任何真实起始时刻，判据为假。**这比 repeat=0 更可靠** ——
    // repeat 会被 `max(n,1)` 提成 1，真正决定开关的是那个比较。
    let (y, m, d, sec) = if sp.is_on() {
        (start.0 + sp.years as i32, start.1, start.2, start.3)
    } else {
        (0, 1, 1, 0)
    };
    vec![
        (
            "DEF_simulation_time%spinup_year".into(),
            Value::Int(y as i64),
        ),
        (
            "DEF_simulation_time%spinup_month".into(),
            Value::Int(m as i64),
        ),
        (
            "DEF_simulation_time%spinup_day".into(),
            Value::Int(d as i64),
        ),
        (
            "DEF_simulation_time%spinup_sec".into(),
            Value::Int(sec as i64),
        ),
        (
            "DEF_simulation_time%spinup_repeat".into(),
            Value::Int(if sp.is_on() { sp.repeat as i64 } else { 0 }),
        ),
    ]
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
        (
            "DEF_simulation_time%start_sec".into(),
            Value::Int(s.window.start_sec as i64),
        ),
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
        (
            "DEF_simulation_time%end_sec".into(),
            Value::Int(s.window.end_sec as i64),
        ),
        ("DEF_simulation_time%timestep".into(), r(s.timestep_seconds)),
    ];
    // 预热那五项。**单独一个函数**：界面上也要能改它，而分两处算的话，
    // 「关掉预热」在两边的写法迟早会不一样 —— 而两种写法只有一种是对的。
    out.extend(spinup_fields(
        (
            s.window.start_year,
            s.window.start_month,
            s.window.start_day,
            s.window.start_sec,
        ),
        s.spinup,
    ));
    out.extend([
        // ---- 路径 ----
        ("DEF_dir_rawdata".into(), Value::Str(s.dirs.rawdata.clone())),
        ("DEF_dir_runtime".into(), Value::Str(s.dirs.runtime.clone())),
        ("DEF_dir_output".into(), Value::Str(s.dirs.output.clone())),
        (
            "DEF_forcing_namelist".into(),
            Value::Str(s.dirs.forcing_namelist.clone()),
        ),
        // ---- 桌面端安全默认 ----
        // 臭氧胁迫不依赖 BGC，但 CoLM 的上游默认会立即读取 2.8 GB 全球场；
        // 新算例显式关闭两层开关，用户在 GUI 启用时再选择并校验数据文件。
        ("DEF_USE_OZONESTRESS".into(), Value::Bool(false)),
        ("DEF_USE_OZONEDATA".into(), Value::Bool(false)),
        ("DEF_WRST_FREQ".into(), Value::Str("MONTHLY".into())),
        ("DEF_HIST_FREQ".into(), Value::Str("HOURLY".into())),
    ]);
    // 地类只在站点文件说得出时才写。说不出就整条不写 —— 写一个猜的值
    // 比不写更糟，而 CoLM 有自己的回落路径（站点文件的分类变量，或栅格）。
    //
    // 城市算例是例外：城市站点文件都不带地类变量，所以这里显式写出
    // 所选分类体系的城市类号。调用方知道当前内核是 USGS 时传 `Some(1)`；
    // 旧调用方没传时保留 IGBP/PC/PFT 的城市类号 13。
    let landtype = if s.urban {
        Some(s.landtype.unwrap_or(URBAN_LANDTYPE_IGBP))
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
        // URBAN_MODEL 从编译期宏改成运行时开关之后，`DEF_URBAN_RUN`
        // 不再被"编译时带 URBAN_MODEL 就强制 .true."那条路径兜底
        // （main/URBAN/ 现在总是编译进去，MOD_Namelist.F90 只是原样
        // 尊重这个字段，不再强制）。写不写这一条会决定城市物理是否
        // 真的跑——不写就默认 .false.，城市算例会悄悄退化成非城市配置。
        out.push(("DEF_URBAN_RUN".into(), Value::Bool(true)));
        // `USE_SITE_lakedepth` / `USE_SITE_soilreflectance` /
        // `USE_SITE_soilparameters` 三项**保持 CoLM 默认的 `.true.`**
        // （「站点文件里有，就用它」），所以这里一个字都不写。
        //
        // **`USE_SITE_soilparameters` 尤其不能是 `.false.`** —— 城市段的
        // readflag 直接就是它（`MOD_SingleSrfdata.F90:2103`），没有自然段
        // 那个 `(.not. mksrfdata)` 逃生口。设成 `.false.`，site.nc 根本不会
        // 被查，`prepare_urban` 写多少土壤进去都没用，CoLM 照样去开
        // `<rawdata>/soil/` 下那 24 个全球栅格 —— 实测 122 GB。
        //
        // 另外两项写不写都一样，删掉只是因为「写一个与默认值相反的值」
        // 需要理由，而这里没有：城市站点文件里确实没有 `lakedepth` 与四个
        // `soil_*_alb`，`prepare_urban` 也不补它们（那两样的真值得量，
        // 而预抽表里只有土壤剖面），于是 `ncio_var_exist` 为假、CoLM 照旧
        // 回落到 `lake_depth.nc` 与 `soil_brightness.nc`。行为与先前逐位相同。
    }
    out
}

/// USGS 的「城市与建成区」。
pub const URBAN_LANDTYPE_USGS: i32 = 1;
/// IGBP/PFT/PC 的「城市与建成区」。
pub const URBAN_LANDTYPE_IGBP: i32 = 13;

#[cfg(test)]
#[path = "build_tests.rs"]
mod build_tests;

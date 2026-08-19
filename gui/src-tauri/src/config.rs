//! 配置层的进程内命令。
//!
//! 这几个都不碰文件系统之外的东西，也都不需要 netcdf ——
//! `colm-schema` 是一张生成的静态表，`colm-namelist` 是纯文本解析。

use serde::Serialize;

/// 把源码 namelist 字段放进用户看得懂的功能分组。
///
/// 返回 `None` 不是「其他」：测试要求当前 CoLM 源码里一个都不能剩。
/// 上游新增字段时 CI 会报出名字，要求读过它的用途后再归类。
pub(crate) fn field_section(name: &str, group: Option<&str>) -> Option<&'static str> {
    let n = name.to_ascii_uppercase();
    let has = |parts: &[&str]| parts.iter().any(|p| n.contains(p));

    if n.starts_with("DEF_HIST_VARS%") {
        return Some("输出变量");
    }
    if n.starts_with("DEF_SIMULATION_TIME%") {
        return Some("时间与预热");
    }
    if n.starts_with("DEF_HIST")
        || n.starts_with("DEF_WRST")
        || n.starts_with("DEF_REST")
        || n == "DEF_HISTORY_IN_VECTOR"
        || n == "DEF_OUTPUT_2MWMO"
        || n == "DEF_DIR_OUTPUT"
        || n == "DEF_DIR_HISTORY"
        || n == "DEF_DIR_RESTART"
        || n == "USE_SITE_HISTWRITEBACK"
    {
        return Some("输出与重启");
    }
    if group == Some("nl_colm_forcing")
        || has(&[
            "FORCING_INTERP",
            "FORCING_DOWNSCALING",
            "CLIMFORCING",
            "DEF_DS_",
            "CBL_HEIGHT",
        ])
    {
        return Some("强迫场");
    }
    if has(&["URBAN", "CANYON_HWR"]) {
        return Some("城市");
    }
    if n.starts_with("SITE_") || n.starts_with("USE_SITE_") {
        return Some("站点");
    }
    if has(&["TRACER", "GIEMS", "WETLAND_FINUNDATION"]) {
        return Some("示踪剂");
    }
    if n.starts_with("DEF_DA_") || n == "DEF_OPTIMIZE_BASEFLOW" {
        return Some("数据同化");
    }
    if has(&[
        "CAMA",
        "ELEMENTNEIGHBOUR",
        "UNITCATCHMENT",
        "RESERVOIR",
        "ROUTING",
        "RIVERDEPTH",
        "LEVEE",
        "BIFURCATION",
    ]) {
        return Some("河道与水库");
    }
    if has(&[
        "SOILINIT",
        "SNOWINIT",
        "CN_INIT",
        "WATERTABLEINIT",
        "FILE_WATERTABLE",
    ]) {
        return Some("初始场");
    }
    if n == "DEF_CASE_NAME" {
        return Some("算例");
    }
    if n.starts_with("DEF_DOMAIN%")
        || has(&[
            "BLOCKINFO",
            "AVERAGEELEMENTSIZE",
            "NX_BLOCKS",
            "NY_BLOCKS",
            "PIO_GROUPSIZE",
            "NIO_EQ_NBLOCK",
            "FILE_MESH",
            "GRIDBASED_LON",
            "GRIDBASED_LAT",
            "CATCHMENTMESH",
            "MESH_FILTER",
        ])
    {
        return Some("网格与并行");
    }
    if has(&[
        "SRFDATA",
        "DEF_LC_YEAR",
        "DEF_USE_USGS",
        "DEF_USE_IGBP",
        "DEF_USE_LCT",
        "DEF_USE_PFT",
        "DEF_USE_PC",
        "DEF_SOLO_PFT",
        "DEF_FAST_PC",
        "PC_CROP_SPLIT",
        "SUBGRID_SCHEME",
        "LANDONLY",
        "DOMINANT_PATCHTYPE",
        "SOILPAR_UPS_FIT",
        "SOIL_REFL_SCHEME",
        "ZIP_FOR_AGGREGATION",
        "DEF_LAI_",
        "LAIFEEDBACK",
        "HIGHRESSOIL",
        "HIGHRESVEG",
        "LULCC_SCHEME",
    ]) {
        return Some("地表数据");
    }
    if has(&[
        "INTERCEPTION",
        "MATSIRO",
        "THERMAL_CONDUCTIVITY",
        "SUPERCOOL",
        "RSS_SCHEME",
        "RUNOFF_SCHEME",
        "VIC_",
        "TOPMOD",
        "SPLIT_SOILSNOW",
        "VARIABLYSATURATEDFLOW",
        "BEDROCK",
        "PRECIP_PHASE",
        "DYNAMIC_LAKE",
        "DYNAMIC_WETLAND",
    ]) {
        return Some("水热过程");
    }
    if has(&[
        "VEG_SNOW",
        "OZONE",
        "SNICAR",
        "SNOWOPTICS",
        "SNOWAGING",
        "PROSPECT",
        "AEROSOL",
        "NDEP",
        "DEF_SSP",
        "IRRIGATION",
        "NOSTRESSNITROGEN",
        "DEF_RSTFAC",
        "PLANTHYDRAULICS",
        "MEDLYNST",
        "WUEST",
        "DEF_USE_SASU",
        "DIAGMATRIX",
        "DEF_USE_PN",
        "DEF_USE_FERT",
        "FERT_SOURCE",
        "NITRIF",
        "CNSOYFIXN",
        "DEF_USE_FIRE",
        "CHECKEQUILIBRIUM",
    ]) {
        return Some("生态与生地化");
    }
    if n.starts_with("DEF_DIR")
        || n.starts_with("DEF_FILE")
        || n.ends_with("_FILE")
        || n.ends_with("_NAMELIST")
    {
        return Some("文件与目录");
    }
    None
}

/// 页面加载时确认后端确实接上了。
///
/// 顺便往 stderr 记一行。这不是调试残留：GUI 出问题时最难分辨的两种情况是
/// 「窗口没开」与「窗口开了但页面是白的」—— 前者进程会退出，后者进程活着、
/// 窗口标题也在，从外面看一模一样。这一行是唯一能从外面区分它们的证据，
/// 因为只有 webview 真的加载并执行了 `index.html` 的 JS 才会调到这里。
/// 同一行还报出它解析到的 `colm-cli` 路径。`resolve_cli` 有四条回落，
/// 其中「仓库的 target/ 产物」那条在开发机上**永远命中**，于是打包版本
/// 找错了 sidecar 也看不出来 —— 实测就发生过：Tauri 把 sidecar 放进
/// `Contents/MacOS/`，而当时的代码找的是 `Contents/Resources/`。
#[tauri::command]
pub fn backend_ready() -> String {
    let msg = format!(
        "backend reachable — {} configuration fields known",
        colm_schema::all().len()
    );
    let cli = crate::sidecar::resolve_cli();
    eprintln!(
        "colm-desktop: the page reached the backend; {msg}; colm-cli resolved to {}",
        cli.display()
    );
    msg
}

/// 一个配置字段，交给前端渲染。
#[derive(Serialize)]
pub struct Field {
    pub name: &'static str,
    pub kind: String,
    pub default: String,
    pub doc: Option<&'static str>,
    /// 它属于哪个 namelist 组，也就是**该写进哪个文件**。
    pub group: Option<&'static str>,
    /// `true` 表示用户设了也没用 —— 有声明有默认值，但不在任何 namelist 组里。
    /// 实测 6 个，其中 `DEF_dir_history` 在 `MOD_Namelist.F90:1406` 被无条件覆盖。
    /// 界面该把它们显示成只读的派生值：给一个改了没用的输入框比不显示更糟。
    pub derived: bool,
    /// 合法取值，非空时界面给下拉框而不是文本框。实测 12 个字段有。
    pub values: &'static [&'static str],
    /// 需要哪些编译期宏。与所选内核 `manifest.json` 的 `macros` 求交，
    /// 交不上就说明这个字段在当前内核下**根本没用**。实测 68 个字段有依赖。
    pub requires: &'static [&'static str],
    /// 从 CoLM 源码字段名与 namelist 组推导的功能分组。
    pub section: &'static str,
}

#[tauri::command]
pub fn describe_fields() -> Vec<Field> {
    colm_schema::all()
        .iter()
        .map(|f| Field {
            name: f.name,
            kind: format!("{:?}", f.kind),
            default: format!("{:?}", f.default),
            doc: f.doc,
            group: f.group,
            derived: f.group.is_none(),
            values: f.values,
            requires: f.requires,
            section: field_section(f.name, f.group).unwrap_or("未分类（这应该被测试拦住）"),
        })
        .collect()
}

/// 在给定内核下，哪些字段**用不上**。
///
/// 判据是内核 `manifest.json` 里的 `macros` —— 那是**构建期就写下的事实**，
/// 不是运行时猜的。字段要求的宏有一个不在里面，它在这个内核下就没有意义：
/// 用户设了不会有任何效果，而界面上摆着它只会让人以为设了有用。
///
/// 返回的是**用不上的**那一批，不是能用的：前端拿同一份名单同时过滤
/// 参数与输出变量，切换内核后重新读取即可。
#[tauri::command]
pub fn irrelevant_fields(kernel_dir: String) -> Result<Vec<String>, String> {
    let k = colm_kernel::Kernel::open(std::path::Path::new(&kernel_dir))
        .map_err(|e| format!("{e:#}"))?;
    let have: std::collections::BTreeSet<&str> =
        k.manifest.macros.iter().map(String::as_str).collect();
    Ok(colm_schema::all()
        .iter()
        .filter(|f| !field_is_relevant(f, &have))
        .map(|f| f.name.to_string())
        .collect())
}

/// 一个源码字段是否对这组内核宏有意义。
fn field_is_relevant(field: &colm_schema::Field, have: &std::collections::BTreeSet<&str>) -> bool {
    // 这项在 MOD_Namelist.F90 里无条件派生 history/restart 路径；源码用法扫描
    // 排除了该文件，所以会误把它只归给 CatchLateralFlow。
    if field.name == "DEF_dir_output" {
        return true;
    }
    if !field.requires.iter().all(|m| have.contains(m)) {
        return false;
    }
    match field_section(field.name, field.group) {
        // 这些开关有一部分在公共 namelist 代码里无守护地出现，但对应子系统
        // 没编进内核时设置它们仍然不会产生任何效果。
        Some("城市") => have.contains("URBAN_MODEL"),
        Some("示踪剂") => have.contains("TRACER"),
        Some("数据同化") => have.contains("DataAssimilation"),
        _ => true,
    }
}

/// 一份 namelist 文本里 `colm-schema` 不认识的字段。
///
/// 不是装饰：上游**自己发布的**单点示例 `run/examples/SiteSYSUAtmos_IGBP_VG.nml`
/// 就设了 `USE_SITE_topostd` 与 `USE_SITE_BVIC` 两个已从 `MOD_Namelist.F90`
/// 删掉的字段，CoLM 读到会 `Cannot match namelist object name` 然后中止。
/// 界面该在开跑前点名它们，而不是让用户对着那句报错发呆。
#[tauri::command]
pub fn unknown_fields(text: String) -> Result<Vec<String>, String> {
    let doc = colm_namelist::parse(&text).map_err(|e| format!("{e:#}"))?;
    Ok(doc
        .paths()
        .into_iter()
        .filter(|p| colm_schema::find(p).is_none())
        .collect())
}

/// 一份 namelist 里的一个字段，交给前端渲染。
#[derive(Serialize)]
pub struct Entry {
    pub path: String,
    /// 值的**原文**，与文件里一模一样
    pub value: String,
    /// `colm-schema` 认不认识它
    pub known: bool,
    pub kind: Option<String>,
    pub group: Option<&'static str>,
    pub derived: bool,
}

/// 读一份 namelist 文本，列出它设了哪些字段。
#[tauri::command]
pub fn read_case(text: String) -> Result<Vec<Entry>, String> {
    let doc = colm_namelist::parse(&text).map_err(|e| format!("{e:#}"))?;
    Ok(doc
        .paths()
        .into_iter()
        .map(|p| {
            let f = colm_schema::find(&p);
            Entry {
                value: doc.get(&p).map(|v| v.to_string()).unwrap_or_default(),
                known: f.is_some(),
                kind: f.map(|f| format!("{:?}", f.kind)),
                group: f.and_then(|f| f.group),
                derived: f.is_some_and(|f| f.group.is_none()),
                path: p,
            }
        })
        .collect())
}

/// 改一个字段，返回**整份**文本。
///
/// 无状态往返：命令收整份文档加一个改动，返回重新校验过的整份文档。
/// 前端不持有配置状态，也**从不自己构造带类型的值** —— 类型由
/// `colm-schema` 决定，字符串怎么变成 `Value` 是这里的事。
///
/// 未被改动的行**逐字节不变**，这是 `colm-namelist` 的往返保证：
/// 用户算例文件里的注释是他们自己的笔记，保存一次不该把它们冲掉。
#[tauri::command]
pub fn set_field(text: String, path: String, value: String) -> Result<String, String> {
    let mut doc = colm_namelist::parse(&text).map_err(|e| format!("{e:#}"))?;
    let v = typed(&path, &value)?;
    doc.set(&path, v).map_err(|e| format!("{e:#}"))?;
    Ok(doc.to_string())
}

/// 按 schema 声明的类型把字符串变成 `Value`。
///
/// schema 不认识的字段一律当字符串 —— 让它写出去，由 CoLM 去表态。
/// 静默丢弃会让用户以为自己设了。
fn typed(path: &str, raw: &str) -> Result<colm_namelist::Value, String> {
    use colm_namelist::Value;
    use colm_schema::FieldKind as K;
    let s = raw.trim();
    let Some(f) = colm_schema::find(path) else {
        return Ok(Value::Str(s.to_string()));
    };
    match f.kind {
        K::Logical => match s.to_ascii_lowercase().trim_matches('.') {
            "true" | "t" => Ok(Value::Bool(true)),
            "false" | "f" => Ok(Value::Bool(false)),
            _ => Err(format!(
                "{path} is logical; {raw:?} is neither .true. nor .false."
            )),
        },
        K::Integer => s
            .parse()
            .map(Value::Int)
            .map_err(|_| format!("{path} is an integer; {raw:?} is not")),
        K::Real => {
            // 存原文：1800. 与 1800.0 与 1.8e3 等价，往返要还原用户写的那种。
            // 但先确认它确实是个数，否则会把一个打错的字悄悄写进文件。
            s.replace(['d', 'D'], "e")
                .parse::<f64>()
                .map_err(|_| format!("{path} is a real; {raw:?} is not a number"))?;
            Ok(Value::Real {
                text: s.to_string(),
            })
        }
        K::Character { len } => {
            let bare = s.trim_matches(|c| c == '\'' || c == '"');
            if bare.len() > len {
                return Err(format!(
                    "{path} holds character(len={len}); {:?} is {} characters",
                    bare,
                    bare.len()
                ));
            }
            Ok(Value::Str(bare.to_string()))
        }
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod config_tests;

/// 设一个字段：在文件里就改，不在就插进它该在的 namelist 组。
///
/// **必须能插。** 专家模式让用户改这份配置没设过的字段，而预热更是必然
/// 要插 —— 关掉预热时截止时刻那四项都不在文件里。只 `set` 的话，
/// 打开预热会报一句 `no such field in this namelist`，而那不是用户的错。
fn put(
    doc: &mut colm_namelist::Document,
    path: &str,
    v: colm_namelist::Value,
) -> Result<(), String> {
    // 组名从 schema 来 —— 那是从 CoLM 自己的声明里扫出来的。
    // schema 不认识的字段只能改不能插：不知道往哪个组插，而插错组等于没设。
    match colm_schema::find(path).and_then(|f| f.group) {
        Some(g) => doc.insert(path, v, g).map_err(|e| format!("{e:#}")),
        None => doc.set(path, v).map_err(|e| format!("{e:#}")),
    }
}

/// 读一批算例的 case.nml。
///
/// **一个读不了就整批失败。** 批量的坏处是"部分成功"——
/// 90 个算例里 3 个没改到，界面上看不出来，而它们会照旧跑一遍旧配置。
fn read_all(dirs: &[String]) -> Result<Vec<(String, String)>, String> {
    dirs.iter()
        .map(|d| {
            let p = std::path::Path::new(d).join("case.nml");
            std::fs::read_to_string(&p)
                .map(|t| (d.clone(), t))
                .map_err(|e| format!("{}: {e}", p.display()))
        })
        .collect()
}

/// 这一批算例里，哪些字段的取值不一致。
///
/// 界面据此在那些行上标出来 —— **不标的话，一个显示着某个值的输入框
/// 其实代表着 90 个不同的值**，而改它会把另外 89 个悄悄抹平。
#[tauri::command]
pub fn varying_fields(dirs: Vec<String>) -> Result<Vec<String>, String> {
    let all = read_all(&dirs)?;
    if all.len() < 2 {
        return Ok(Vec::new());
    }
    let docs: Vec<_> = all
        .iter()
        .map(|(d, t)| {
            colm_namelist::parse(t)
                .map(|doc| (d.clone(), doc))
                .map_err(|e| format!("{d}: {e:#}"))
        })
        .collect::<Result<_, _>>()?;
    // 并集而不是交集：某个算例**没设**某字段，本身就是一种不一致 ——
    // 它跑的是 CoLM 的默认值，而别的算例跑的是写出来的那个。
    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (_, doc) in &docs {
        names.extend(doc.paths());
    }
    let mut out = Vec::new();
    for n in names {
        let first = docs[0].1.get(&n).map(|v| v.to_string());
        if docs
            .iter()
            .any(|(_, d)| d.get(&n).map(|v| v.to_string()) != first)
        {
            out.push(n);
        }
    }
    Ok(out)
}

/// 一次批量写的结果。`text` 是**代表算例**（列表里第一个）改完之后的内容，
/// 界面拿它继续显示 —— 不回传的话前端还得再读一次文件。
#[derive(Debug, serde::Serialize)]
pub struct BatchWrite {
    pub written: usize,
    pub text: String,
}

/// 把一个字段写进这一批算例的每一份 case.nml。
///
/// **先全改完再落盘。** 中途出错就一份都不写 —— 半批配置好的算例
/// 与整批配置好的在界面上长得一样，而它们跑出来的东西不一样。
#[tauri::command]
pub fn set_field_batch(
    dirs: Vec<String>,
    path: String,
    value: String,
) -> Result<BatchWrite, String> {
    let all = read_all(&dirs)?;
    let mut done: Vec<(String, String)> = Vec::with_capacity(all.len());
    for (d, text) in all {
        let mut doc = colm_namelist::parse(&text).map_err(|e| format!("{d}: {e:#}"))?;
        put(&mut doc, &path, typed(&path, &value)?).map_err(|e| format!("{d}: {e}"))?;
        done.push((d, doc.to_string()));
    }
    write_all(&done)
}

pub(crate) fn write_all(done: &[(String, String)]) -> Result<BatchWrite, String> {
    for (d, text) in done {
        let p = std::path::Path::new(d).join("case.nml");
        std::fs::write(&p, text).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    Ok(BatchWrite {
        written: done.len(),
        text: done.first().map(|(_, t)| t.clone()).unwrap_or_default(),
    })
}

/// 一份配置里与「时间与预热」有关的东西，界面直接照着显示。
///
/// **算好了再交出去**，不让前端自己拼：预热截止时刻是起始年月日加上若干年，
/// 而输出从截止时刻才开始 —— 这两条算错了没人会发现，输出会安安静静地
/// 少一段。同一份算式在 `colm-case::spinup_fields` 里，两边共用它。
#[derive(serde::Serialize)]
pub struct Timing {
    /// 这一批有几个算例。
    pub count: usize,
    /// 各算例的窗口是否一致。**多站点时通常不一致** —— 每个站点的窗口
    /// 是它自己那份强迫场的完整覆盖范围，而各站点的记录长短本来就不同。
    pub window_varies: bool,
    pub start: String,
    pub end: String,
    pub spinup_years: u32,
    pub spinup_repeat: u32,
    /// 各算例的预热设置是否一致。
    pub spinup_varies: bool,
    /// history 从哪天开始。**不等于 start** —— 预热期不写 history
    /// （`MOD_Hist.F90:235` 在 `itstamp <= ptstamp` 时直接 RETURN）。
    pub output_start: String,
}

/// 读出时间窗与预热。
///
/// 取不到的项用 CoLM 的声明默认值，与 `read_case` 的口径一致 ——
/// 一个没写进文件的字段不是"没有值"，而是"用默认值"。
#[tauri::command]
pub fn read_timing(dirs: Vec<String>) -> Result<Timing, String> {
    let all = read_all(&dirs)?;
    let mut each = Vec::with_capacity(all.len());
    for (d, text) in &all {
        let doc = colm_namelist::parse(text).map_err(|e| format!("{d}: {e:#}"))?;
        each.push(one_timing(&doc));
    }
    let Some(first) = each.first().cloned() else {
        return Err("没有算例".into());
    };
    Ok(Timing {
        count: each.len(),
        window_varies: each.iter().any(|t| t.0 != first.0 || t.1 != first.1),
        start: first.0.clone(),
        end: first.1.clone(),
        spinup_years: first.2,
        spinup_repeat: first.3,
        spinup_varies: each.iter().any(|t| t.2 != first.2 || t.3 != first.3),
        output_start: first.4,
    })
}

/// 一份配置的 (start, end, 预热年数, 预热遍数, 输出起始日)。
fn one_timing(doc: &colm_namelist::Document) -> (String, String, u32, u32, String) {
    let int = |p: &str| -> i64 {
        match doc.get(p) {
            Some(colm_namelist::Value::Int(v)) => *v,
            _ => match colm_schema::find(p).map(|f| f.default) {
                Some(colm_schema::Default::Integer(v)) => v,
                _ => 0,
            },
        }
    };
    let (sy, sm, sd) = (
        int("DEF_simulation_time%start_year"),
        int("DEF_simulation_time%start_month"),
        int("DEF_simulation_time%start_day"),
    );
    let repeat = int("DEF_simulation_time%spinup_repeat").max(0) as u32;
    let py = int("DEF_simulation_time%spinup_year");
    // 预热开着的判据与 CoLM 一样：截止时刻晚于起始时刻（`CoLM.F90:314`）。
    // 光看 repeat 会把 `year = 0` 那种关法读成开着。
    let on = py > sy && repeat > 1;
    let ymd = |y: i64, m: i64, d: i64| format!("{y:04}-{m:02}-{d:02}");
    (
        ymd(sy, sm, sd),
        ymd(
            int("DEF_simulation_time%end_year"),
            int("DEF_simulation_time%end_month"),
            int("DEF_simulation_time%end_day"),
        ),
        if on { (py - sy) as u32 } else { 0 },
        if on { repeat } else { 0 },
        if on {
            ymd(
                py,
                int("DEF_simulation_time%spinup_month"),
                int("DEF_simulation_time%spinup_day"),
            )
        } else {
            ymd(sy, sm, sd)
        },
    )
}

/// 改这一批算例的预热。
///
/// 五个字段一起写 —— 单改一个会得到一个自相矛盾的截止时刻。
/// **每个算例按自己的起始年算截止年**：各站点的强迫场起点不同，
/// 用同一个绝对年份会让一部分算例的预热落在窗口之外（等于没预热），
/// 另一部分落得过深（等于把输出砍掉一大截）。
#[tauri::command]
pub fn set_spinup(dirs: Vec<String>, years: u32, repeat: u32) -> Result<BatchWrite, String> {
    let all = read_all(&dirs)?;
    let mut done = Vec::with_capacity(all.len());
    for (d, text) in all {
        let mut doc = colm_namelist::parse(&text).map_err(|e| format!("{d}: {e:#}"))?;
        let int = |p: &str| -> i64 {
            match doc.get(p) {
                Some(colm_namelist::Value::Int(v)) => *v,
                _ => match colm_schema::find(p).map(|f| f.default) {
                    Some(colm_schema::Default::Integer(v)) => v,
                    _ => 0,
                },
            }
        };
        let start = (
            int("DEF_simulation_time%start_year") as i32,
            int("DEF_simulation_time%start_month") as u32,
            int("DEF_simulation_time%start_day") as u32,
        );
        for (path, v) in colm_case::spinup_fields(start, colm_case::Spinup { years, repeat }) {
            put(&mut doc, &path, v).map_err(|e| format!("{d}: {e}"))?;
        }
        done.push((d, doc.to_string()));
    }
    write_all(&done)
}

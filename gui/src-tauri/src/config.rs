//! 配置层的进程内命令。
//!
//! 这几个都不碰文件系统之外的东西，也都不需要 netcdf ——
//! `colm-schema` 是一张生成的静态表，`colm-namelist` 是纯文本解析。

use serde::{Deserialize, Serialize};

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
    // 调试三件套（CoLMDEBUG / RangeCheck / SrfdataDiag）曾经是编译期宏，
    // 现在是运行时开关（`MOD_Namelist.F90` 里的 `DEF_USE_*`，默认
    // `.false.`）。三个都不属于任何单一物理过程，放在一起单独一栏，
    // 别被 `SrfdataDiag` 的 "SRFDATA" 子串顺手分进下面的「地表数据」。
    if n == "DEF_USE_COLMDEBUG" || n == "DEF_USE_RANGECHECK" || n == "DEF_USE_SRFDATADIAG" {
        return Some("调试与诊断");
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
    // LAI feedback is a prognostic BGC process, not a surface-data source.
    if n == "DEF_USE_LAIFEEDBACK" {
        return Some("生态与生地化");
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
        "DEF_USE_LULCC",
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
        "CAMPBELL_SOIL_MODEL",
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
        "DEF_USE_BGC",
        "DEF_USE_CROP",
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

/// 前端把话说到 stderr。**这是这台机器上 GUI 唯一可靠的观察通道** ——
/// AX 树读取时灵时不灵、`screencapture` 没有屏幕录制权限，两条都实测不可用。
///
/// 不引 `tauri-plugin-log`：那个插件要在 webview 侧注入 console 钩子，而这里
/// 恰恰要诊断「前端代码到底跑没跑」。诊断工具依赖被诊断的那一层，说明不了问题。
#[tauri::command]
pub fn probe_log(msg: String) {
    eprintln!("colm-desktop[probe]: {msg}");
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
    /// 合法取值，非空时界面给下拉框而不是文本框。当前 30 个字段有。
    pub values: &'static [&'static str],
    /// 需要哪些编译期宏。与所选内核 `manifest.json` 的 `macros` 求交，
    /// 交不上就说明这个字段在当前内核下**根本没用**。实测 68 个字段有依赖。
    pub requires: &'static [&'static str],
    /// 从 CoLM 源码字段名与 namelist 组推导的功能分组。
    pub section: &'static str,
}

fn default_literal(value: colm_schema::Default) -> String {
    match value {
        colm_schema::Default::Logical(value) => {
            if value { ".true." } else { ".false." }.to_string()
        }
        colm_schema::Default::Integer(value) => value.to_string(),
        colm_schema::Default::Real(value)
        | colm_schema::Default::Str(value)
        | colm_schema::Default::Array(value) => value.to_string(),
    }
}

#[tauri::command]
pub fn describe_fields() -> Vec<Field> {
    colm_schema::all()
        .iter()
        .map(|f| Field {
            name: f.name,
            kind: format!("{:?}", f.kind),
            // 前端会把未显式设置的字段默认值直接放进控件。Debug 文本
            // `Integer(3)` / `Logical(true)` 不是 Fortran 值，会生成无法保存的
            // 数字框或非法选项；必须传可直接写回 namelist 的字面量。
            default: default_literal(f.default),
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
        //
        // 没有「城市」这一条了：LULC/BGC/CROP/URBAN/LULCC 那组改造把
        // URBAN_MODEL 也变成运行时开关了（`DEF_URBAN_RUN`），
        // `main/URBAN/` 始终编译进去，`URBAN_MODEL` 本身从
        // `include/define.h` 里彻底消失——城市字段现在在每个内核下都
        // 「有意义」（能不能真的看到城市输出取决于 `DEF_URBAN_RUN`
        // 本身怎么设，那是运行时的事，不是这个函数管的编译期相关性）。
        Some("数据同化") => have.contains("DataAssimilation"),
        // 单点内核没有河网，整个分栏都没有意义。上游至少有
        // `DEF_ElementNeighbour_file` 和 `DEF_Reservoir_Method` 漏了 `requires`；
        // 只逐项补洞会让下一个漏标字段再次把空分栏撑出来。因此这里以过程
        // 是否编进内核为分栏总闸门，字段自身更细的 `requires` 已在上面检查。
        Some("河道与水库") => {
            have.contains("CaMa_Flood")
                || have.contains("GridRiverLakeFlow")
                || have.contains("CatchLateralFlow")
        }
        // SinglePoint 在时间管理器里自己固定 360×180 block 映射，不读区域边界、
        // mesh、PIO 分组或用户给的 block 划分。CPU 并发是 GUI 自己的批量设置，
        // 不属于这些 namelist 字段；入口保留，但这一整张无效参数表要隐藏。
        Some("网格与并行") => !have.contains("SinglePoint"),
        _ => true,
    }
}

/// 一个字段在当前算例里的交互状态。
///
/// `irrelevant_fields` 只回答编译期问题；这里把内核宏与 case.nml 当前值组合起来，
/// 避免前端各处分散维护一套迟早会漂移的依赖关系。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldMode {
    Editable,
    Disabled,
    Hidden,
}

#[derive(Debug, Clone, Serialize)]
pub struct FieldState {
    pub name: String,
    pub mode: FieldMode,
    pub reason: Option<&'static str>,
    /// 非空时覆盖 schema 的取值集合。用于表达运行时互斥和 SinglePoint
    /// 不支持的枚举值，而不是等 CoLM 启动以后再纠正。
    pub allowed_values: Vec<&'static str>,
    /// 批量中至少两个算例对这个字段的状态不同。前端仍按“任一算例有效就显示”
    /// 的安全方向处理，但必须明确警告，不能让代表算例掩盖差异。
    pub mixed: bool,
}

struct VisibilityContext<'a> {
    doc: &'a colm_namelist::Document,
    have: &'a std::collections::BTreeSet<&'a str>,
    single: bool,
    usgs: bool,
    lct: bool,
    pft: bool,
    pc: bool,
    vg: bool,
    bgc: bool,
    crop: bool,
    urban: bool,
    lulcc: bool,
    tracer: bool,
    site_landtype: i64,
    soil_init: bool,
    snow_init: bool,
    cn_init: bool,
    water_table_init: bool,
    downscale: bool,
    downscale_simple: bool,
    site_lai: bool,
    lai_feedback: bool,
    lai_change_yearly: bool,
    soil_reflectance_scheme: i64,
    runoff: i64,
    snicar: bool,
    aerosol_readin: bool,
    ozone_stress: bool,
    ozone_data: bool,
    interception: i64,
    medlyn: bool,
    wuest: bool,
}

fn logical(doc: &colm_namelist::Document, name: &str) -> bool {
    match doc.get(name) {
        Some(colm_namelist::Value::Bool(value)) => *value,
        _ => matches!(
            colm_schema::find(name).map(|field| field.default),
            Some(colm_schema::Default::Logical(true))
        ),
    }
}

fn integer(doc: &colm_namelist::Document, name: &str) -> i64 {
    match doc.get(name) {
        Some(colm_namelist::Value::Int(value)) => *value,
        _ => match colm_schema::find(name).map(|field| field.default) {
            Some(colm_schema::Default::Integer(value)) => value,
            _ => 0,
        },
    }
}

impl<'a> VisibilityContext<'a> {
    fn new(
        doc: &'a colm_namelist::Document,
        have: &'a std::collections::BTreeSet<&'a str>,
    ) -> Self {
        Self {
            doc,
            have,
            single: have.contains("SinglePoint"),
            usgs: have.contains("LULC_USGS"),
            lct: logical(doc, "DEF_USE_LCT"),
            pft: logical(doc, "DEF_USE_PFT"),
            pc: logical(doc, "DEF_USE_PC"),
            vg: !logical(doc, "DEF_USE_Campbell_SOIL_MODEL"),
            bgc: logical(doc, "DEF_USE_BGC"),
            // DEF_USE_CROP 是编译期数组尺寸选择的只读反映，不接受 case.nml
            // 里的伪开关；manifest 是唯一可信来源。
            crop: have.contains("CROP"),
            urban: logical(doc, "DEF_URBAN_RUN"),
            lulcc: logical(doc, "DEF_USE_LULCC"),
            tracer: logical(doc, "DEF_USE_TRACER"),
            site_landtype: integer(doc, "SITE_landtype"),
            soil_init: logical(doc, "DEF_USE_SoilInit"),
            snow_init: logical(doc, "DEF_USE_SnowInit"),
            cn_init: logical(doc, "DEF_USE_CN_INIT"),
            water_table_init: logical(doc, "DEF_USE_WaterTableInit"),
            downscale: logical(doc, "DEF_USE_Forcing_Downscaling"),
            downscale_simple: logical(doc, "DEF_USE_Forcing_Downscaling_Simple"),
            site_lai: logical(doc, "USE_SITE_LAI"),
            lai_feedback: logical(doc, "DEF_USE_LAIFEEDBACK"),
            lai_change_yearly: logical(doc, "DEF_LAI_CHANGE_YEARLY"),
            soil_reflectance_scheme: integer(doc, "DEF_SOIL_REFL_SCHEME"),
            runoff: integer(doc, "DEF_Runoff_SCHEME"),
            snicar: logical(doc, "DEF_USE_SNICAR"),
            aerosol_readin: logical(doc, "DEF_Aerosol_Readin"),
            ozone_stress: logical(doc, "DEF_USE_OZONESTRESS"),
            ozone_data: logical(doc, "DEF_USE_OZONEDATA"),
            interception: integer(doc, "DEF_Interception_scheme"),
            medlyn: logical(doc, "DEF_USE_MEDLYNST"),
            wuest: logical(doc, "DEF_USE_WUEST"),
        }
    }

    fn waterbody(&self) -> bool {
        self.site_landtype == if self.usgs { 16 } else { 17 }
    }

    fn wetland(&self) -> bool {
        self.site_landtype == if self.usgs { 17 } else { 11 }
    }

    fn cropland(&self) -> bool {
        self.site_landtype == if self.usgs { 7 } else { 12 }
    }

    fn urban_land(&self) -> bool {
        self.site_landtype == if self.usgs { 1 } else { 13 }
    }

    fn glacier(&self) -> bool {
        self.site_landtype == if self.usgs { 24 } else { 15 }
    }

    fn natural_pft_land(&self) -> bool {
        !self.waterbody()
            && !self.wetland()
            && !self.urban_land()
            && !self.glacier()
            && !(self.crop && self.cropland())
    }

    fn biological_land(&self) -> bool {
        self.natural_pft_land() || (self.crop && self.cropland())
    }
}

fn hidden(reason: &'static str) -> (FieldMode, Option<&'static str>, Vec<&'static str>) {
    (FieldMode::Hidden, Some(reason), Vec::new())
}

fn field_runtime_state(
    field: &colm_schema::Field,
    c: &VisibilityContext<'_>,
) -> (FieldMode, Option<&'static str>, Vec<&'static str>) {
    let name = field.name;
    let one_of = |names: &[&str]| names.contains(&name);

    if !field_is_relevant(field, c.have) {
        return hidden("当前内核未编入这个功能");
    }

    if name == "DEF_URBAN_geom_data" {
        return hidden("CoLM 当前只读取并广播此字段，没有任何计算路径使用它");
    }

    // SinglePoint 在读写完单点 surface data 后直接返回；这些字段只服务于
    // 区域聚合、分块或区域历史输出，继续展示会制造“配置已生效”的假象。
    if c.single
        && one_of(&[
            "USE_srfdata_from_larger_region",
            "DEF_dir_existing_srfdata",
            "USE_srfdata_from_3D_gridded_data",
            "DEF_SOLO_PFT",
            "DEF_FAST_PC",
            "DEF_SUBGRID_SCHEME",
            "DEF_LANDONLY",
            "DEF_USE_DOMINANT_PATCHTYPE",
            "DEF_USE_SOILPAR_UPS_FIT",
            "USE_zip_for_aggregation",
            "DEF_Srfdata_CompressLevel",
            "DEF_Forcing_Interp_Method",
            "DEF_TOPMOD_method",
            "DEF_HISTORY_IN_VECTOR",
            "DEF_HIST_grid_as_forcing",
            "DEF_HIST_lon_res",
            "DEF_HIST_lat_res",
            "DEF_HIST_mode",
            "DEF_HIST_WriteBack",
            "DEF_URBAN_ONLY",
            "DEF_USE_SrfdataDiag",
        ])
    {
        return hidden("SinglePoint 执行路径不会使用这个字段");
    }
    if c.single && name == "DEF_HIST_CompressLevel" && !c.tracer {
        return hidden("SinglePoint 普通历史文件不使用此压缩设置");
    }

    if one_of(&[
        "DEF_HighResSoil",
        "DEF_HighResVeg",
        "DEF_PROSPECT",
        "DEF_HighResUrban_albedo",
    ]) && !c.have.contains("HYPERSPECTRAL")
    {
        return hidden("当前内核未启用 HYPERSPECTRAL");
    }
    if name == "DEF_HighResUrban_albedo" && !c.urban {
        return hidden("仅城市高光谱模式使用");
    }

    // 站点身份在建例时逐站点写入。批量参数页若允许修改，会把多个站点的
    // 文件、坐标或地类悄悄统一成同一个值，因此 SinglePoint 一律不展示。
    // 自然站的地类来自站点 NetCDF 的 IGBP_classification；城市站固定为 13。
    if c.single
        && one_of(&[
            "SITE_fsitedata",
            "SITE_lon_location",
            "SITE_lat_location",
            "SITE_landtype",
            "USE_SITE_landtype",
        ])
    {
        return hidden("由选择站点并建算例按站点自动确定");
    }
    // 其余站点数据字段跟随实际分类。湖泊、湿地、作物和 PFT 比例只对
    // 对应地表有意义。
    if name == "USE_SITE_pctpfts" && (!(c.pft || c.pc) || !c.natural_pft_land()) {
        return hidden("仅自然地表的 PFT/PC 次网格使用");
    }
    if name == "USE_SITE_pctcrop" && !(c.crop && c.cropland()) {
        return hidden("仅 CROP 内核的作物地表使用");
    }
    if name == "USE_SITE_lakedepth" && !c.waterbody() {
        return hidden("仅水体站点使用");
    }
    if name == "USE_SITE_dbedrock" && !logical(c.doc, "DEF_USE_BEDROCK") {
        return hidden("需要先启用基岩过程");
    }

    if name == "DEF_PC_CROP_SPLIT" && (!c.pc || (c.single && !c.biological_land())) {
        return hidden("仅 PC 次网格使用");
    }
    // 单点站点优先读取 site.nc 里的 LAI；这时原始 LAI 数据的年份与时间分辨率
    // 不参与计算。关掉 USE_SITE_LAI 后，才显示对应的回退数据设置。
    if c.single
        && (c.site_lai || c.lai_feedback)
        && one_of(&[
            "DEF_LC_YEAR",
            "DEF_LAI_START_YEAR",
            "DEF_LAI_END_YEAR",
            "DEF_LAI_MONTHLY",
            "DEF_LAI_CHANGE_YEARLY",
        ])
    {
        return hidden(if c.lai_feedback {
            "LAI 由 BGC 叶碳反馈计算"
        } else {
            "当前按站点文件读取 LAI"
        });
    }
    if one_of(&["DEF_LAI_START_YEAR", "DEF_LAI_END_YEAR"]) && !c.lai_change_yearly {
        return hidden("需要先启用叶面积指数逐年变化");
    }
    if c.single && name == "DEF_LC_YEAR" && c.lai_change_yearly {
        return hidden("逐年 LAI 使用模拟年份，不使用单一地表数据年份");
    }
    if name == "DEF_LAI_MONTHLY" && (c.pft || c.pc || c.lulcc || c.urban) {
        return hidden("当前次网格会自动使用月尺度 LAI");
    }
    if name == "DEF_USE_LAIFEEDBACK" && !c.bgc {
        return hidden("需要 BGC");
    }
    if name == "DEF_LULCC_SCHEME" && !c.lulcc {
        return hidden("需要先启用 LULCC");
    }
    if name == "USE_SITE_soilreflectance" && c.soil_reflectance_scheme != 2 {
        return hidden("当前方案按地表覆盖类型估算土壤反照率");
    }

    // 初始场文件是父开关的子项。SoilInit 同时打开时，CoLM 明确忽略独立的
    // water-table 文件；CN 初始化则只在 BGC 下存在。
    if name == "DEF_file_SoilInit" && !c.soil_init {
        return hidden("需要先启用土壤初始场");
    }
    if name == "DEF_file_SnowInit" && !c.snow_init {
        return hidden("需要先启用积雪初始场");
    }
    if one_of(&["DEF_USE_CN_INIT", "DEF_file_cn_init"]) && !c.bgc {
        return hidden("需要 BGC");
    }
    if name == "DEF_file_cn_init" && !c.cn_init {
        return hidden("需要先启用 CN 初始场");
    }
    if name == "DEF_file_WaterTable" && (!c.water_table_init || c.soil_init) {
        return hidden("仅独立地下水位初始化使用");
    }
    if name == "DEF_USE_WaterTableInit" && c.soil_init {
        return hidden("土壤初始场已经包含地下水位初值");
    }

    // 完整与简单降尺度共用数组，不能同时开启。子项关闭时不显示；降水方案
    // III 的 MPI/Python 分支被 #ifndef SinglePoint 包围。
    if name == "DEF_DS_HiresTopographyDataDir" && !c.downscale {
        return hidden("仅完整地形强迫降尺度需要外部高分辨率地形目录");
    }
    if one_of(&[
        "DEF_DS_precipitation_adjust_scheme",
        "DEF_DS_longwave_adjust_scheme",
    ]) && !(c.downscale || c.downscale_simple)
    {
        return hidden("需要先选择一种强迫场降尺度模式");
    }
    if name == "DEF_USE_Forcing_Downscaling" && c.downscale_simple {
        return (
            FieldMode::Editable,
            Some("简单降尺度已开启；先关闭它才能开启完整降尺度"),
            vec![".false."],
        );
    }
    if name == "DEF_USE_Forcing_Downscaling_Simple" && c.downscale {
        return (
            FieldMode::Editable,
            Some("完整降尺度已开启；先关闭它才能开启简单降尺度"),
            vec![".false."],
        );
    }
    if name == "DEF_DS_precipitation_adjust_scheme" && c.single {
        return (FieldMode::Editable, None, vec!["I", "II"]);
    }
    // 站点工作流生成的 forcing.nml 固定使用 POINT 数据集。POINT 的文件名
    // 不含年份，ClimForcing 只把年份替换为 `clim`，因此打开也不会改变读入。
    if name == "DEF_USE_ClimForcing_for_Spinup" && c.single {
        return hidden("站点 POINT 强迫场始终循环同一文件，此开关不会改变读入");
    }

    // 完整扩展截留模块由 extend_interception 宏选择整套源文件。当前随软件
    // 发布的所有内核都带该宏，因此 1..8 都真实可用；若用户安装了不带宏的
    // 外部内核，回退模块只调用 CoLM2014，实现上只有方案 1。
    if name == "DEF_Interception_scheme" {
        return if c.have.contains("extend_interception") {
            (
                FieldMode::Editable,
                Some("当前内核已编入扩展截留模块，8 种方案均有实际计算路径"),
                vec!["1", "2", "3", "4", "5", "6", "7", "8"],
            )
        } else {
            (
                FieldMode::Editable,
                Some("当前内核未编入 extend_interception，只能使用 CoLM2014 方案"),
                vec!["1"],
            )
        };
    }

    if name == "DEF_MATSIRO_CWCAP_SCALE" && c.interception != 5 {
        return hidden("仅 MATSIRO 截留方案使用");
    }
    if name == "DEF_RSS_SCHEME" && c.lct && c.vg {
        return hidden("LCT + van Genuchten 下 CoLM 会自动关闭土壤表面阻抗");
    }
    if name == "DEF_USE_VariablySaturatedFlow" && c.vg {
        return hidden("van Genuchten 下 CoLM 会自动启用 VSF");
    }
    if one_of(&["DEF_VIC_OPT", "DEF_file_VIC_para", "DEF_file_VIC_OPT"]) && c.runoff != 1 {
        return hidden("仅 VIC runoff 使用");
    }
    if one_of(&["DEF_file_VIC_para", "DEF_file_VIC_OPT"]) {
        return hidden("CoLM 会从运行时目录派生 VIC 参数文件");
    }
    if name == "DEF_USE_Dynamic_Lake" && !c.waterbody() {
        return hidden("仅水体站点使用");
    }
    if name == "DEF_USE_Dynamic_Wetland" && !c.wetland() {
        return hidden("仅湿地站点使用");
    }

    // BGC/CROP 子过程必须跟随真实运行时/编译期能力，不能依赖整个页面的粗粒度
    // 开关。独立的积雪、臭氧和植被物理选项仍可在 BGC 关闭时使用。
    if one_of(&[
        "DEF_NDEP_FREQUENCY",
        "DEF_USE_NOSTRESSNITROGEN",
        "DEF_USE_SASU",
        "DEF_USE_DiagMatrix",
        "DEF_USE_PN",
        "DEF_USE_NITRIF",
        "DEF_USE_FIRE",
        "DEF_CheckEquilibrium",
    ]) && (!c.bgc || (c.single && !c.biological_land()))
    {
        return hidden("需要 BGC");
    }
    if one_of(&[
        "DEF_USE_FERT",
        "DEF_FERT_SOURCE",
        "DEF_USE_CNSOYFIXN",
        "DEF_USE_IRRIGATION",
        "DEF_IRRIGATION_ALLOCATION",
    ]) && (!c.crop || (c.single && !c.cropland()))
    {
        return hidden("当前内核未启用 CROP");
    }
    if name == "DEF_USE_CROP" && !c.crop {
        return hidden("当前内核未启用 CROP");
    }
    if c.single
        && !c.biological_land()
        && one_of(&[
            "DEF_VEG_SNOW",
            "DEF_USE_OZONESTRESS",
            "DEF_USE_OZONEDATA",
            "DEF_file_Ozone",
            "DEF_USE_MEDLYNST",
            "DEF_USE_WUEST",
        ])
    {
        return hidden("当前站点不是植被地表，不会使用叶片或冠层过程");
    }
    if name == "DEF_USE_OZONEDATA" && !c.ozone_stress {
        return hidden("需要先启用臭氧胁迫");
    }
    if name == "DEF_file_Ozone" && (!c.ozone_stress || !c.ozone_data) {
        return hidden("仅从文件读取臭氧数据时使用");
    }
    if one_of(&["DEF_file_snowoptics", "DEF_file_snowaging"]) {
        return hidden("CoLM 会从运行时目录派生 SNICAR 数据文件");
    }
    if one_of(&["DEF_Aerosol_Readin", "DEF_Aerosol_Clim"]) && !c.snicar {
        return hidden("需要先启用 SNICAR");
    }
    if name == "DEF_Aerosol_Clim" && !c.aerosol_readin {
        return hidden("需要先读取气溶胶数据");
    }
    if name == "DEF_USE_MEDLYNST" && c.wuest {
        return (
            FieldMode::Editable,
            Some("WUEST 已开启；两种气孔方案不能同时开启"),
            vec![".false."],
        );
    }
    if name == "DEF_USE_WUEST" && c.medlyn {
        return (
            FieldMode::Editable,
            Some("Medlyn 已开启；两种气孔方案不能同时开启"),
            vec![".false."],
        );
    }

    if field_section(name, field.group) == Some("城市") && !c.urban {
        return hidden("需要先启用城市模型");
    }
    if c.urban
        && one_of(&[
            "DEF_USE_WUEST",
            "DEF_USE_SUPERCOOL_WATER",
            "DEF_USE_PLANTHYDRAULICS",
            "DEF_USE_OZONESTRESS",
            "DEF_USE_OZONEDATA",
            "DEF_SPLIT_SOILSNOW",
        ])
    {
        return hidden("城市模式会自动关闭这个过程");
    }
    if field_section(name, field.group) == Some("示踪剂") && !c.tracer {
        return hidden("需要先启用 TRACER");
    }

    if field.group.is_none() {
        return (
            FieldMode::Disabled,
            Some("由内核或其他路径自动派生，只读显示"),
            Vec::new(),
        );
    }

    (FieldMode::Editable, None, Vec::new())
}

pub(crate) fn field_states_for(
    text: &str,
    have: &std::collections::BTreeSet<&str>,
) -> Result<Vec<FieldState>, String> {
    let doc = colm_namelist::parse(text).map_err(|e| format!("{e:#}"))?;
    let context = VisibilityContext::new(&doc, have);
    Ok(colm_schema::all()
        .iter()
        .map(|field| {
            let (mode, reason, allowed_values) = field_runtime_state(field, &context);
            FieldState {
                name: field.name.to_string(),
                mode,
                reason,
                allowed_values,
                mixed: false,
            }
        })
        .collect())
}

fn merge_field_states(groups: &[Vec<FieldState>]) -> Vec<FieldState> {
    let Some(first) = groups.first() else {
        return Vec::new();
    };
    first
        .iter()
        .enumerate()
        .map(|(index, template)| {
            let each: Vec<&FieldState> = groups.iter().map(|group| &group[index]).collect();
            debug_assert!(each.iter().all(|state| state.name == template.name));
            let visible: Vec<&FieldState> = each
                .iter()
                .copied()
                .filter(|state| state.mode != FieldMode::Hidden)
                .collect();
            let mut mode = if visible.is_empty() {
                FieldMode::Hidden
            } else if visible
                .iter()
                .any(|state| state.mode == FieldMode::Editable)
            {
                FieldMode::Editable
            } else {
                FieldMode::Disabled
            };

            // 空 allowed_values 表示“使用 schema 的完整集合”，因此它是交集运算
            // 的全集，不参与收窄；有多个非空约束时取交集。
            let mut constraints = visible
                .iter()
                .filter(|state| !state.allowed_values.is_empty())
                .map(|state| state.allowed_values.as_slice());
            let (allowed_values, had_constraints) = constraints.next().map_or_else(
                || (Vec::new(), false),
                |head| {
                    let rest: Vec<_> = constraints.collect();
                    (
                        head.iter()
                            .copied()
                            .filter(|value| rest.iter().all(|values| values.contains(value)))
                            .collect(),
                        true,
                    )
                },
            );
            let no_common_value = had_constraints && allowed_values.is_empty();
            if no_common_value && mode != FieldMode::Hidden {
                // 空 allowed_values 平时表示“schema 全部取值均可”。这里却是多个
                // 非空约束的交集为空，不能把它误解释为无限制；批量编辑必须锁住。
                mode = FieldMode::Disabled;
            }
            let mixed = no_common_value
                || each.iter().any(|state| {
                    state.mode != template.mode || state.allowed_values != template.allowed_values
                });
            let reason = if no_common_value {
                Some("所选算例对这个字段没有共同合法值；请缩小批量范围后分别配置")
            } else if mixed {
                Some("所选算例的父开关或站点类型不同；此字段仅对其中一部分算例有效")
            } else {
                template.reason
            };
            FieldState {
                name: template.name.clone(),
                mode,
                reason,
                allowed_values,
                mixed,
            }
        })
        .collect()
}

/// 当前内核 + 当前 case.nml 的统一字段状态。
#[tauri::command]
pub fn field_states(text: String, kernel_dir: String) -> Result<Vec<FieldState>, String> {
    let kernel = colm_kernel::Kernel::open(std::path::Path::new(&kernel_dir))
        .map_err(|e| format!("{e:#}"))?;
    let have: std::collections::BTreeSet<&str> =
        kernel.manifest.macros.iter().map(String::as_str).collect();
    field_states_for(&text, &have)
}

/// 批量编辑时按全部算例合并状态：只有全部无效才隐藏；任一算例有效就显示，
/// 同时用 `mixed` 标出条件差异。这样不会因代表算例 BGC=false 而把另一个
/// BGC=true 算例的子字段整个藏掉。
#[tauri::command]
pub fn field_states_batch(
    dirs: Vec<String>,
    kernel_dir: String,
) -> Result<Vec<FieldState>, String> {
    if dirs.is_empty() {
        return Err("没有可配置的算例".into());
    }
    let kernel = colm_kernel::Kernel::open(std::path::Path::new(&kernel_dir))
        .map_err(|e| format!("{e:#}"))?;
    let have: std::collections::BTreeSet<&str> =
        kernel.manifest.macros.iter().map(String::as_str).collect();
    let all = read_all(&dirs)?;
    let groups: Vec<Vec<FieldState>> = all
        .iter()
        .map(|(dir, text)| field_states_for(text, &have).map_err(|e| format!("{dir}: {e}")))
        .collect::<Result<_, _>>()?;
    Ok(merge_field_states(&groups))
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
#[cfg(test)]
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
    let bare = s.trim_matches(|c| c == '\'' || c == '"');
    if !f.values.is_empty() && !f.values.iter().any(|v| v.eq_ignore_ascii_case(bare)) {
        return Err(format!(
            "{path} only accepts {}; {raw:?} is invalid",
            f.values.join(", ")
        ));
    }
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

/// 向导在新算例里写入的一项运行时初值。
#[derive(Debug, Deserialize)]
pub struct FieldChange {
    pub path: String,
    pub value: String,
}

/// 一次校验并写入一组字段。任一值无效时，原文件保持不变。
pub(crate) fn apply_fields(dir: &str, fields: &[FieldChange]) -> Result<(), String> {
    let path = std::path::Path::new(dir).join("case.nml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut doc = colm_namelist::parse(&text).map_err(|e| format!("{dir}: {e:#}"))?;
    for field in fields {
        let value = typed(&field.path, &field.value).map_err(|e| format!("{dir}: {e}"))?;
        put(&mut doc, &field.path, value).map_err(|e| format!("{dir}: {e}"))?;
    }
    std::fs::write(&path, doc.to_string()).map_err(|e| format!("{}: {e}", path.display()))
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
    set_fields_batch(dirs, vec![FieldChange { path, value }])
}

/// 把一组有关联的字段原子地写进整批算例。
///
/// 用于“启用初始场并选择文件”和互斥开关：不能先把父开关写成 true，再因
/// 路径或另一开关写入失败而留下半套配置。
#[tauri::command]
pub fn set_fields_batch(dirs: Vec<String>, fields: Vec<FieldChange>) -> Result<BatchWrite, String> {
    if fields.is_empty() {
        return Err("没有要保存的字段".into());
    }
    let all = read_all(&dirs)?;
    let mut done: Vec<(String, String)> = Vec::with_capacity(all.len());
    for (d, text) in all {
        let mut doc = colm_namelist::parse(&text).map_err(|e| format!("{d}: {e:#}"))?;
        for field in &fields {
            let value = typed(&field.path, &field.value)?;
            put(&mut doc, &field.path, value).map_err(|e| format!("{d}: {e}"))?;
        }
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
    /// CoLM 会打印的 TIMESTEP 总数，含每一轮预热。进度事件里的 `step`
    /// 从 1 单调递增到这个数，所以前端不必再猜百分比。
    pub total_steps: u64,
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
        output_start: first.4.clone(),
        total_steps: first.5,
    })
}

/// 一份配置的 (start, end, 预热年数, 预热遍数, 输出起始日, 总步数)。
fn one_timing(doc: &colm_namelist::Document) -> (String, String, u32, u32, String, u64) {
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
    let (ey, em, ed) = (
        int("DEF_simulation_time%end_year"),
        int("DEF_simulation_time%end_month"),
        int("DEF_simulation_time%end_day"),
    );
    let repeat = int("DEF_simulation_time%spinup_repeat").max(0) as u32;
    let py = int("DEF_simulation_time%spinup_year");
    // 预热开着的判据与 CoLM 一样：截止时刻晚于起始时刻（`CoLM.F90:300`）。
    // `spinup_repeat = 1` 仍会把 start→spinup 截止这段当预热跑一遍且不写 history；
    // 手写 0 也会被 CoLM 提成 1。界面关闭预热靠写 `spinup_year = 0`。
    let on = py > sy;
    let repeat = if on { repeat.max(1) } else { repeat };
    let ymd = |y: i64, m: i64, d: i64| format!("{y:04}-{m:02}-{d:02}");
    let stamp = |y: i64, m: i64, d: i64, sec: i64| {
        // Howard Hinnant 的 days_from_civil。这里不能依赖 colm-forcing：那会把
        // netcdf/hdf5 拖进窗口进程，只为算两个日期的差。
        let y = if m <= 2 { y - 1 } else { y };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let mp = if m > 2 { m - 3 } else { m + 9 };
        let doy = (153 * mp + 2) / 5 + d - 1;
        (era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy) * 86_400 + sec
    };
    let start = stamp(sy, sm, sd, int("DEF_simulation_time%start_sec"));
    let end = stamp(ey, em, ed, int("DEF_simulation_time%end_sec"));
    let step = doc
        .get("DEF_simulation_time%timestep")
        .and_then(colm_namelist::Value::as_f64)
        .or_else(
            || match colm_schema::find("DEF_simulation_time%timestep")?.default {
                colm_schema::Default::Real(v) => v.parse().ok(),
                _ => None,
            },
        )
        .unwrap_or(0.0) as i64;
    let steps = |from: i64, to: i64| -> u64 {
        if step <= 0 || to <= from {
            0
        } else {
            ((to - from) as u64).div_ceil(step as u64)
        }
    };
    let normal_steps = steps(start, end);
    let total_steps = if on {
        let spinup_end = stamp(
            py,
            int("DEF_simulation_time%spinup_month"),
            int("DEF_simulation_time%spinup_day"),
            int("DEF_simulation_time%spinup_sec"),
        );
        // 截止时刻必须落在窗口内才会触发重置。最后一轮不重置，而是从
        // 截止处继续跑到 end；与 CoLM.F90 的 TIMELOOP 完全同序。
        if spinup_end < end {
            let spinup_steps = steps(start, spinup_end);
            spinup_steps * repeat as u64 + steps(start + spinup_steps as i64 * step, end)
        } else {
            normal_steps
        }
    } else {
        normal_steps
    };
    (
        ymd(sy, sm, sd),
        ymd(ey, em, ed),
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
        total_steps,
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
            int("DEF_simulation_time%start_sec") as u32,
        );
        for (path, v) in colm_case::spinup_fields(start, colm_case::Spinup { years, repeat }) {
            put(&mut doc, &path, v).map_err(|e| format!("{d}: {e}"))?;
        }
        done.push((d, doc.to_string()));
    }
    write_all(&done)
}

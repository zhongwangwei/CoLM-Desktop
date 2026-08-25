//! 强迫场的探测与转换。
//!
//! **走 sidecar 而不是直接调 `colm-forcing`。** GUI 进程里不能有 netcdf
//! （`Cargo.toml` 那条量化过的注释：`colm-forcing` 7 个 netcdf/hdf5 依赖
//! 节点、`colm-cli` 9 个，而窗口进程该链接的那几层都是 0），所以读 `.nc`
//! 的事一律交给 `colm-cli` 子进程，与 `sites.rs` 的 `scan_sites` 同一条路。

use serde::{Deserialize, Serialize};

/// 一个槽位探测到的结果。**字段必须与 `colm-cli` 的 `SlotProbe` 一一对应。**
///
/// 两边各声明一次是分层的代价：`colm-cli` 在引擎 workspace、GUI 在另一个，
/// 两者不互相依赖。代价由 `forcing_tests` 里那条拿真 CLI 输出跑的测试兜住 ——
/// 见 `sites_tests.rs` 上同一句话，这里抄一遍是因为道理完全一样。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotGuess {
    pub index: usize,
    pub meaning: String,
    pub optional: bool,
    /// 猜不到是 `None` —— JSON 里是 `null`。
    pub guessed: Option<String>,
    /// 猜到的变量在源文件里的单位，读不到也是 `None`。
    pub units: Option<String>,
    /// CoLM 期望的单位，与 `units` 对照着看。
    pub wants: String,
}

/// 探测结果的整体。**字段必须与 `colm-cli` 的 `ForcingProbe` 一一对应。**
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Probe {
    pub variables: Vec<String>,
    /// 变量形状由 sidecar 返回。边界层高度开关会用它确认 POINT 数据确实
    /// 是 `(time, 1, 1)`，而不是误把区域网格的第一个像元当成站点。
    #[serde(default)]
    pub shapes: Vec<VariableShape>,
    /// 恒为 8 个元素，对应 `colm_forcing::SLOTS` 的八个槽位。
    pub slots: Vec<SlotGuess>,
    pub steps: usize,
    pub step_seconds: f64,
    pub step_uniform: bool,
    pub time_units: String,
    pub time_first: f64,
    pub time_last: f64,
    /// 三个观测高度。源文件没有 `reference_height_*` 时是 `None`
    /// （JSON 里是 `null`），不是 `NaN`。实测 PLUMBER2 的 90 个站全有，
    /// Urban-PLUMBER 的 21 个站全没有 —— 两条路都要覆盖（见测试）。
    pub height_v: Option<f64>,
    pub height_t: Option<f64>,
    pub height_q: Option<f64>,
    /// 建议的产物目录，界面上预填。
    ///
    /// **不能留空让人手打。** 产物目录是转换的必填项，而路径打错一个
    /// 字符，报错会出现在完全无关的地方（`rfd` 那条依赖的注释就是为这个
    /// 加的）。这里给一个必定可写、必定不含空格、必定与源文件不同目录的
    /// 位置 —— 三条都是硬要求：后端拒绝同目录，而 CoLM 的
    /// `mkdir -p` 不加引号，路径含空格会建出影子目录树。
    ///
    /// 与 `~/CoLM-cases` 平行（见 `example.rs` 的 `cases_root`）。
    /// `colm-cli forcing-probe` 不输出这个字段，所以要 `serde(default)`
    /// —— 它是 GUI 侧补上的，不属于那份 JSON 契约。
    #[serde(default)]
    pub suggest_dst: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableShape {
    pub name: String,
    pub dimensions: Vec<DimensionShape>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DimensionShape {
    pub name: String,
    pub len: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableColumn {
    pub name: String,
    pub units: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableSite {
    pub id: String,
    pub rows: usize,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub landtype: Option<i32>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub step_seconds: Option<i64>,
    pub inserted_steps: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableSlot {
    pub index: usize,
    pub meaning: String,
    pub optional: bool,
    pub column: Option<String>,
    pub units: Option<String>,
    pub wants: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableProbe {
    pub delimiter: String,
    pub columns: Vec<TableColumn>,
    pub rows: usize,
    pub site_column: Option<String>,
    pub time_column: Option<String>,
    pub latitude_column: Option<String>,
    pub longitude_column: Option<String>,
    pub landtype_column: Option<String>,
    pub utc_offset_column: Option<String>,
    pub sites: Vec<TableSite>,
    pub slots: Vec<TableSlot>,
    #[serde(default)]
    pub suggest_dst: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedTableSite {
    pub site: String,
    pub safe_site: String,
    pub staged_path: String,
    pub final_path: String,
    pub rows: usize,
    pub inserted_steps: usize,
    pub latitude: f64,
    pub longitude: f64,
    pub landtype: Option<i32>,
    pub timezone_offset_hours: Option<f64>,
    pub timezone_source: String,
    pub start_utc: i64,
    pub end_utc: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TableImportOptions {
    pub time_column: String,
    pub site_column: Option<String>,
    pub latitude_column: Option<String>,
    pub longitude_column: Option<String>,
    pub landtype_column: Option<String>,
    pub utc_offset_column: Option<String>,
    pub utc_offset: Option<f64>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub step_seconds: Option<i64>,
    pub land_cover_scheme: Option<String>,
    pub heights: Option<[f64; 3]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DatasetProbe {
    variables: Vec<String>,
    shapes: Vec<VariableShape>,
}

/// 同步探测入口，供“启用边界层高度”在一次配置事务里同时校验主强迫场
/// 与用户选中的边界层文件。NetCDF 仍只在 sidecar 里打开。
pub(crate) fn probe_file(path: &str) -> Result<Probe, String> {
    let json = crate::sidecar::capture(&[
        "forcing-probe".into(),
        path.into(),
        "--json".into(),
        "1".into(),
    ])?;
    serde_json::from_str(&json).map_err(|e| {
        format!("colm-cli forcing-probe 的输出解析不了（两边的字段可能已经对不上）：{e}")
    })
}

fn probe_dataset(path: &str) -> Result<DatasetProbe, String> {
    let json = crate::sidecar::capture(&[
        "netcdf-probe".into(),
        path.into(),
        "--json".into(),
        "1".into(),
    ])?;
    serde_json::from_str(&json).map_err(|e| {
        format!("colm-cli netcdf-probe 的输出解析不了（两边的字段可能已经对不上）：{e}")
    })
}

/// 探一份强迫场文件：变量列表、自动猜出来的槽位映射、时间轴、高度。
///
/// **只探不改。** 用户要先看到猜的结果、能改，才允许转换 ——
/// 变量名猜错的后果是「跑得完、结果全错」，而曲线照样是曲线，
/// 界面上什么都看不出来。
#[tauri::command]
pub async fn probe_forcing(app: tauri::AppHandle, path: String) -> Result<Probe, String> {
    let mut probe = tauri::async_runtime::spawn_blocking(move || probe_file(&path))
        .await
        .map_err(|error| format!("强迫场探测任务异常终止：{error}"))??;
    probe.suggest_dst = suggested_dst(&app);
    Ok(probe)
}

#[tauri::command]
pub async fn probe_forcing_table(
    app: tauri::AppHandle,
    path: String,
) -> Result<TableProbe, String> {
    let json = crate::sidecar::capture_async(vec![
        "forcing-table-probe".into(),
        path,
        "--json".into(),
        "1".into(),
    ])
    .await?;
    let mut probe: TableProbe = serde_json::from_str(&json).map_err(|error| {
        format!("colm-cli forcing-table-probe 的输出解析不了（两边的字段可能已经对不上）：{error}")
    })?;
    probe.suggest_dst = suggested_dst(&app);
    Ok(probe)
}

/// 建议的产物目录：`~/CoLM-forcing`。
///
/// 与 `example.rs` 的 `cases_root` 同一套理由，不重复写 —— 简言之是
/// **不含空格**（CoLM 的 `mkdir -p` 不加引号）、**不在 TCC 保护目录下**
/// （`~/Documents` 会弹系统权限框）、**必定不与源文件同目录**
/// （后端会拒绝同目录，原始数据永远不动）。
///
/// 拿不到主目录时返回空串，界面上就是个空框 —— 那时候让人自己填，
/// 比塞一个猜的路径强。
fn suggested_dst(app: &tauri::AppHandle) -> String {
    use tauri::Manager;
    app.path()
        .home_dir()
        .map(|h| h.join("CoLM-forcing").display().to_string())
        .unwrap_or_default()
}

/// 用户对一个槽位的选择（或确认了猜测）。
#[derive(Debug, Clone, Deserialize)]
pub struct SlotChoice {
    pub index: usize,
    pub name: String,
    pub units: String,
    /// 要合并进同一个槽位的额外变量（合并降水相态：`Rainf` + `Snowf`）。
    pub also_add: Vec<String>,
}

/// 把用户的选择拼成 `colm-cli forcing-convert` 认的参数列表。
///
/// 抽成同步函数是为了不引入 tokio 就能测 —— `#[tauri::command]` 的
/// `async fn` 不好直接测，命令本身只做薄壳。
fn build_convert_args(
    src: &str,
    dst: &str,
    slots: &[SlotChoice],
    heights: Option<[f64; 3]>,
) -> Vec<String> {
    let mut args = vec![
        "forcing-convert".to_string(),
        src.to_string(),
        dst.to_string(),
    ];
    for s in slots {
        let mut spec = format!("{}={}:{}", s.index, s.name, s.units);
        for extra in &s.also_add {
            spec.push('+');
            spec.push_str(extra);
        }
        args.push("--slot".into());
        args.push(spec);
    }
    if let Some([v, t, q]) = heights {
        args.push("--height".into());
        args.push(format!("{v},{t},{q}"));
    }
    args
}

fn build_table_convert_args(
    src: &str,
    dst: &str,
    slots: &[SlotChoice],
    options: &TableImportOptions,
) -> Vec<String> {
    let mut args = vec![
        "forcing-table-convert".to_string(),
        src.to_string(),
        dst.to_string(),
        "--time".into(),
        options.time_column.clone(),
    ];
    for (flag, value) in [
        ("--site", options.site_column.as_deref()),
        ("--lat-column", options.latitude_column.as_deref()),
        ("--lon-column", options.longitude_column.as_deref()),
        ("--landtype-column", options.landtype_column.as_deref()),
        ("--offset-column", options.utc_offset_column.as_deref()),
    ] {
        if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
            args.push(flag.into());
            args.push(value.into());
        }
    }
    for (flag, value) in [
        ("--utc-offset", options.utc_offset),
        ("--lat", options.latitude),
        ("--lon", options.longitude),
    ] {
        if let Some(value) = value {
            args.push(flag.into());
            args.push(value.to_string());
        }
    }
    if let Some(step) = options.step_seconds {
        args.push("--step-seconds".into());
        args.push(step.to_string());
    }
    if let Some(scheme) = options
        .land_cover_scheme
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        args.push("--land-cover-scheme".into());
        args.push(scheme.into());
    }
    for slot in slots {
        let mut spec = format!("{}={}:{}", slot.index, slot.name, slot.units);
        for extra in &slot.also_add {
            spec.push('+');
            spec.push_str(extra);
        }
        args.push("--slot".into());
        args.push(spec);
    }
    if let Some([v, t, q]) = options.heights {
        args.push("--height".into());
        args.push(format!("{v},{t},{q}"));
    }
    args.push("--json".into());
    args.push("1".into());
    args
}

#[tauri::command]
pub async fn convert_forcing_table(
    src: String,
    dst: String,
    slots: Vec<SlotChoice>,
    options: TableImportOptions,
) -> Result<Vec<ImportedTableSite>, String> {
    if slots.is_empty() {
        return Err("CSV/TXT 至少要确认一个强迫场变量槽位".into());
    }
    std::fs::create_dir_all(&dst).map_err(|error| format!("产物目录 {dst} 无法创建：{error}"))?;
    let args = build_table_convert_args(&src, &dst, &slots, &options);
    let json = crate::sidecar::capture_async(args).await?;
    serde_json::from_str(&json).map_err(|error| {
        format!(
            "colm-cli forcing-table-convert 的输出解析不了（两边的字段可能已经对不上）：{error}"
        )
    })
}

/// 拒绝产物与源文件放在同一目录。
///
/// **先 `canonicalize()` 再比较。** macOS 上 `/tmp` 与 `/private/tmp` 是
/// 同一个地方（前者是指向后者的符号链接），不规范化的话，选一个「看起来
/// 不一样」的 `/tmp/...` 当产物目录会被放行，而磁盘上它跟源文件是同一处，
/// 转换产物照样把源文件所在目录搅乱。
///
/// 源文件必然存在，所以能直接规范化整条路径；产物往往还不存在（正是要
/// 写出来的那个文件），规范化的是它的**父目录**。
fn reject_same_dir(src: &std::path::Path, dst: &std::path::Path) -> Result<(), String> {
    let src_dir = src
        .canonicalize()
        .map_err(|e| format!("源文件 {} 打不开：{e}", src.display()))?
        .parent()
        .ok_or_else(|| format!("{} 没有父目录", src.display()))?
        .to_path_buf();
    let dst_parent = match dst.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => std::path::Path::new("."),
    };
    let dst_dir = dst_parent
        .canonicalize()
        .map_err(|e| format!("产物目录 {} 打不开：{e}", dst_parent.display()))?;
    if src_dir == dst_dir {
        return Err(format!(
            "转换产物不能与源文件放在同一目录（{}）：原始数据要原样留着，\
             产物另放一处，不然以后分不清哪份是没动过的原始数据。",
            src_dir.display()
        ));
    }
    Ok(())
}

/// 转换一份强迫场文件：按用户确认过的槽位映射与（可选的）观测高度，
/// 写出一份 CoLM 认的标准文件。
///
/// 产物路径由调用方给定；这里只负责拒绝与源文件同目录，其余交给
/// `colm-cli forcing-convert`。成功时返回产物路径，供界面显示。
#[tauri::command]
pub async fn convert_forcing(
    src: String,
    dst: String,
    slots: Vec<SlotChoice>,
    heights: Option<[f64; 3]>,
) -> Result<String, String> {
    reject_same_dir(std::path::Path::new(&src), std::path::Path::new(&dst))?;
    let args = build_convert_args(&src, &dst, &slots, heights);
    crate::sidecar::capture_async(args).await?;
    Ok(dst)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapVariable {
    pub slot: usize,
    pub variable: String,
    pub missing: usize,
    pub quality_rejected: usize,
    pub short_missing: usize,
    pub long_missing: usize,
    pub longest_gap: usize,
    pub interpolated: usize,
    pub era5_corrected: usize,
    pub unresolved: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapReport {
    pub timezone_offset_hours: f64,
    pub timezone_source: String,
    pub timezone_confidence: String,
    pub timezone_conflict: bool,
    pub solar_noon_hour: Option<f64>,
    pub solar_noon_std_hours: Option<f64>,
    pub latitude: f64,
    pub longitude: f64,
    pub start_date: String,
    pub end_date: String,
    pub missing: usize,
    pub unresolved: usize,
    pub needs_era5: bool,
    pub variables: Vec<GapVariable>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GapOptions {
    pub short_gap: usize,
    pub utc_offset: Option<f64>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub era5: Option<String>,
    pub min_overlap: usize,
}

fn build_gap_args(
    command: &str,
    src: &str,
    dst: Option<&str>,
    slots: &[SlotChoice],
    options: &GapOptions,
) -> Vec<String> {
    let mut args = vec![command.to_string(), src.to_string()];
    if let Some(dst) = dst {
        args.push(dst.to_string());
    }
    for slot in slots {
        let mut spec = format!("{}={}:{}", slot.index, slot.name, slot.units);
        for extra in &slot.also_add {
            spec.push('+');
            spec.push_str(extra);
        }
        args.push("--slot".into());
        args.push(spec);
    }
    args.push("--short-gap".into());
    args.push(options.short_gap.to_string());
    if let Some(offset) = options.utc_offset {
        args.push("--utc-offset".into());
        args.push(offset.to_string());
    }
    if let Some(latitude) = options.latitude {
        args.push("--lat".into());
        args.push(latitude.to_string());
    }
    if let Some(longitude) = options.longitude {
        args.push("--lon".into());
        args.push(longitude.to_string());
    }
    if let Some(era5) = options
        .era5
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        args.push("--era5".into());
        args.push(era5.to_string());
    }
    args.push("--min-overlap".into());
    args.push(options.min_overlap.to_string());
    args.push("--json".into());
    args.push("1".into());
    args
}

#[tauri::command]
pub async fn probe_forcing_gaps(
    src: String,
    slots: Vec<SlotChoice>,
    options: GapOptions,
) -> Result<GapReport, String> {
    let args = build_gap_args("forcing-gap-probe", &src, None, &slots, &options);
    let json = crate::sidecar::capture_async(args).await?;
    serde_json::from_str(&json).map_err(|error| {
        format!("colm-cli forcing-gap-probe 的输出解析不了（两边的字段可能已经对不上）：{error}")
    })
}

#[tauri::command]
pub async fn repair_forcing(
    src: String,
    dst: String,
    slots: Vec<SlotChoice>,
    options: GapOptions,
) -> Result<GapReport, String> {
    if std::path::Path::new(&src) == std::path::Path::new(&dst) {
        return Err("修复产物不能覆盖原始强迫场".into());
    }
    let args = build_gap_args("forcing-repair", &src, Some(&dst), &slots, &options);
    let json = crate::sidecar::capture_async(args).await?;
    serde_json::from_str(&json).map_err(|error| {
        format!("colm-cli forcing-repair 的输出解析不了（两边的字段可能已经对不上）：{error}")
    })
}

#[tauri::command]
pub async fn download_era5land(
    dst: String,
    latitude: f64,
    longitude: f64,
    start: String,
    end: String,
) -> Result<String, String> {
    let args = vec![
        "era5land-download".into(),
        dst.clone(),
        "--lat".into(),
        latitude.to_string(),
        "--lon".into(),
        longitude.to_string(),
        "--start".into(),
        start,
        "--end".into(),
        end,
    ];
    crate::sidecar::capture_async(args)
        .await
        .map_err(|error| format!("ERA5-Land 下载任务异常终止：{error}"))?;
    Ok(dst)
}

fn required_string(doc: &colm_namelist::Document, path: &str) -> Result<String, String> {
    match doc.get(path) {
        Some(colm_namelist::Value::Str(value)) if !value.trim().is_empty() => Ok(value.clone()),
        Some(value) => Err(format!("{path} 应该是路径字符串，当前是 {value}")),
        None => Err(format!("配置缺少 {path}")),
    }
}

fn put(
    doc: &mut colm_namelist::Document,
    path: &str,
    value: colm_namelist::Value,
    group: &str,
) -> Result<(), String> {
    if doc.get(path).is_some() {
        doc.set(path, value).map_err(|e| e.to_string())
    } else {
        doc.insert(path, value, group).map_err(|e| e.to_string())
    }
}

fn cbl_variable(probe: &Probe) -> Result<&str, String> {
    const CANDIDATES: &[&str] = &["blh", "hpbl", "pblh", "boundary_layer_height"];
    let name = CANDIDATES
        .iter()
        .find_map(|candidate| {
            probe
                .variables
                .iter()
                .find(|name| name.eq_ignore_ascii_case(candidate))
        })
        .ok_or_else(|| {
            "所选文件没有边界层高度变量；支持 blh、hpbl、PBLH 或 boundary_layer_height".to_string()
        })?;
    let shape = probe
        .shapes
        .iter()
        .find(|shape| shape.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| format!("探测结果缺少变量 {name} 的维度信息"))?;
    if !shape
        .dimensions
        .iter()
        .any(|dimension| dimension.name.eq_ignore_ascii_case("time"))
    {
        return Err(format!("边界层高度变量 {name} 没有 time 维"));
    }
    if let Some(dimension) = shape
        .dimensions
        .iter()
        .find(|dimension| !dimension.name.eq_ignore_ascii_case("time") && dimension.len != 1)
    {
        return Err(format!(
            "边界层高度变量 {name} 的 {} 维长度是 {}；SinglePoint 只能读取已提取到本站的 1×1 数据，不能静默取区域网格第一个像元",
            dimension.name, dimension.len
        ));
    }
    Ok(name)
}

fn common_parent(a: &std::path::Path, b: &std::path::Path) -> Option<std::path::PathBuf> {
    let a = a.parent()?;
    let b = b.parent()?;
    a.ancestors()
        .find(|ancestor| b.starts_with(ancestor))
        .map(std::path::Path::to_path_buf)
}

/// 为一个 POINT 算例接入独立的边界层高度文件。
///
/// 这不是单改 `DEF_USE_CBL_HEIGHT`：第九个强迫变量的文件名、变量名、时间步长
/// 都在 `forcing.nml`。先校验两份文件的时间轴与空间形状，再写 forcing.nml，
/// 最后才打开 case.nml 开关；即使最后一步写失败，也只会留下未启用的备用配置。
#[tauri::command]
pub fn configure_cbl_batch(
    dirs: Vec<String>,
    file: String,
    kernel_dir: Option<String>,
) -> Result<crate::config::BatchWrite, String> {
    if dirs.len() != 1 {
        return Err(
            "边界层高度文件是逐站点数据；请先把编辑范围切换为单个站点，再为该站选择文件".into(),
        );
    }
    let chosen = std::path::Path::new(&file)
        .canonicalize()
        .map_err(|e| format!("边界层高度文件 {file} 打不开：{e}"))?;
    let cbl_probe = probe_file(&chosen.display().to_string())?;
    if !cbl_probe.step_uniform || cbl_probe.steps < 2 || cbl_probe.step_seconds <= 0.0 {
        return Err("边界层高度文件必须有至少两个、等时间间隔的 time 记录".into());
    }
    let variable = cbl_variable(&cbl_probe)?.to_string();

    let dir = &dirs[0];
    let case_path = std::path::Path::new(dir).join("case.nml");
    let case_text =
        std::fs::read_to_string(&case_path).map_err(|e| format!("{}: {e}", case_path.display()))?;
    let case_doc = colm_namelist::parse(&case_text).map_err(|e| format!("{dir}: {e:#}"))?;
    let forcing_name = required_string(&case_doc, "DEF_forcing_namelist")?;
    let forcing_path = {
        let path = std::path::PathBuf::from(&forcing_name);
        if path.is_absolute() {
            path
        } else {
            std::path::Path::new(dir).join(path)
        }
    };
    let forcing_text = std::fs::read_to_string(&forcing_path)
        .map_err(|e| format!("{}: {e}", forcing_path.display()))?;
    let mut forcing_doc = colm_namelist::parse(&forcing_text)
        .map_err(|e| format!("{}: {e:#}", forcing_path.display()))?;
    let dataset = required_string(&forcing_doc, "DEF_forcing%dataset")?;
    if !dataset.eq_ignore_ascii_case("POINT") {
        return Err(format!(
            "当前强迫数据集是 {dataset}；文件选择器只支持 POINT 站点文件，区域强迫仍需按月份前缀配置"
        ));
    }
    let forcing_dir = std::path::PathBuf::from(required_string(&forcing_doc, "DEF_dir_forcing")?);
    if !forcing_dir.is_absolute() {
        return Err("DEF_dir_forcing 必须是绝对路径，才能安全接入另一个数据文件".into());
    }
    let primary_name = required_string(&forcing_doc, "DEF_forcing%fprefix(1)")?;
    let primary = forcing_dir
        .join(primary_name)
        .canonicalize()
        .map_err(|e| format!("主强迫场文件打不开：{e}"))?;
    let primary_probe = probe_file(&primary.display().to_string())?;
    let same_step = (primary_probe.step_seconds - cbl_probe.step_seconds).abs() < 1.0e-6;
    if primary_probe.steps != cbl_probe.steps
        || !same_step
        || primary_probe.time_units != cbl_probe.time_units
        || (primary_probe.time_first - cbl_probe.time_first).abs() >= 1.0e-6
        || (primary_probe.time_last - cbl_probe.time_last).abs() >= 1.0e-6
    {
        return Err(format!(
            "边界层高度与主强迫场必须共用完全相同的时间轴；主强迫场为 {} 条、{} 秒、{}（{}..{}），所选文件为 {} 条、{} 秒、{}（{}..{}）",
            primary_probe.steps,
            primary_probe.step_seconds,
            primary_probe.time_units,
            primary_probe.time_first,
            primary_probe.time_last,
            cbl_probe.steps,
            cbl_probe.step_seconds,
            cbl_probe.time_units,
            cbl_probe.time_first,
            cbl_probe.time_last
        ));
    }
    let root = common_parent(&primary, &chosen)
        .ok_or_else(|| "主强迫场与边界层高度文件没有可用的共同父目录".to_string())?;
    let primary_relative = primary
        .strip_prefix(&root)
        .map_err(|e| e.to_string())?
        .display()
        .to_string();
    let cbl_relative = chosen
        .strip_prefix(&root)
        .map_err(|e| e.to_string())?
        .display()
        .to_string();
    for (label, value) in [
        ("共同数据目录", root.display().to_string()),
        ("主强迫场相对路径", primary_relative.clone()),
        ("边界层高度相对路径", cbl_relative.clone()),
    ] {
        if value.len() > 256 {
            return Err(format!("{label}超过 CoLM 的 256 字符路径上限"));
        }
    }
    let seconds = cbl_probe.step_seconds.round();
    if (seconds - cbl_probe.step_seconds).abs() > 1.0e-6 || seconds > i64::MAX as f64 {
        return Err(format!(
            "边界层高度时间步长 {} 秒不能写入 CoLM 的整数 CBL_dtime",
            cbl_probe.step_seconds
        ));
    }
    let seconds = seconds as i64;
    for (path, value) in [
        (
            "DEF_dir_forcing",
            colm_namelist::Value::Str(root.display().to_string()),
        ),
        (
            "DEF_forcing%fprefix(1)",
            colm_namelist::Value::Str(primary_relative),
        ),
        (
            "DEF_forcing%CBL_fprefix",
            colm_namelist::Value::Str(cbl_relative),
        ),
        ("DEF_forcing%CBL_vname", colm_namelist::Value::Str(variable)),
        (
            "DEF_forcing%CBL_tintalgo",
            colm_namelist::Value::Str("linear".into()),
        ),
        ("DEF_forcing%CBL_dtime", colm_namelist::Value::Int(seconds)),
        (
            "DEF_forcing%CBL_offset",
            colm_namelist::Value::Int(seconds / 2),
        ),
    ] {
        put(&mut forcing_doc, path, value, "nl_colm_forcing")
            .map_err(|e| format!("{}: {e}", forcing_path.display()))?;
    }
    std::fs::write(&forcing_path, forcing_doc.to_string())
        .map_err(|e| format!("{}: {e}", forcing_path.display()))?;
    crate::config::set_field_batch(
        dirs,
        "DEF_USE_CBL_HEIGHT".into(),
        ".true.".into(),
        kernel_dir,
    )
}

/// 打开臭氧胁迫并绑定一份可读的 NetCDF 数据。臭氧不依赖 BGC，但只有植被
/// 叶片会使用它；可见性由 config.rs 的地表约束负责。
#[tauri::command]
pub fn configure_ozone_batch(
    dirs: Vec<String>,
    file: String,
    kernel_dir: Option<String>,
) -> Result<crate::config::BatchWrite, String> {
    let chosen = std::path::Path::new(&file)
        .canonicalize()
        .map_err(|e| format!("臭氧文件 {file} 打不开：{e}"))?;
    let probe = probe_dataset(&chosen.display().to_string())?;
    for needed in ["lat", "lon", "OZONE"] {
        if !probe
            .variables
            .iter()
            .any(|name| name.eq_ignore_ascii_case(needed))
        {
            return Err(format!("所选文件缺少 CoLM 臭氧读取所需的 {needed} 变量"));
        }
    }
    let ozone = probe
        .shapes
        .iter()
        .find(|shape| shape.name.eq_ignore_ascii_case("OZONE"))
        .ok_or_else(|| "探测结果缺少 OZONE 的维度信息".to_string())?;
    let time = ozone
        .dimensions
        .iter()
        .find(|dimension| dimension.name.eq_ignore_ascii_case("time"))
        .ok_or_else(|| "OZONE 变量必须有 time 维".to_string())?;
    if time.len < 365 * 8 {
        return Err(format!(
            "OZONE 的 time 维只有 {} 条；CoLM 按每日 8 个三小时记录索引，至少需要 2920 条",
            time.len
        ));
    }
    crate::config::set_fields_batch(
        dirs,
        vec![
            crate::config::FieldChange {
                path: "DEF_USE_OZONESTRESS".into(),
                value: ".true.".into(),
            },
            crate::config::FieldChange {
                path: "DEF_USE_OZONEDATA".into(),
                value: ".true.".into(),
            },
            crate::config::FieldChange {
                path: "DEF_file_Ozone".into(),
                value: chosen.display().to_string(),
            },
        ],
        kernel_dir,
    )
}

#[cfg(test)]
#[path = "forcing_tests.rs"]
mod forcing_tests;

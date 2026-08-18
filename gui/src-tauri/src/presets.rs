//! 参数预设：把一份算例里的**物理与输出设置**存下来，套到别的算例上。
//!
//! 两个决定值得写在这里。
//!
//! **存 `(路径, 值)` 列表，不存整份 `.nml`。** 预设的用处是跨算例复用，
//! 而整份覆盖会连站点身份一起换掉 —— 套一次预设，算例就变成了另一个站点的
//! 算例，而文件名没变。存字段列表则可以合并。
//!
//! **存在算例目录之外。** 存进算例目录里的预设只有那个算例看得见，
//! 而那正好是它唯一用不上的地方。

use serde::{Deserialize, Serialize};

/// 一份预设。
#[derive(Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    /// `(字段路径, Fortran 字面量原文)`。**原文**而不是解析后的值：
    /// `colm-namelist` 的往返保证建立在原文上，`1800.` 与 `1800.0` 都要能还原。
    pub fields: Vec<(String, String)>,
}

/// 这些字段**不进预设**。
///
/// 它们是**算例身份**，不是参数：站点是哪个、文件放在哪、强迫场配置在哪。
/// 把它们塞进预设，套用时就会把 A 站的算例悄悄指向 B 站的数据 ——
/// 而算例名、目录名都不会变，从外面完全看不出来。
///
/// 判据按前缀，不逐个列名字：上游加一个 `SITE_` 字段时，它自动也被挡在外面。
const IDENTITY_PREFIXES: &[&str] = &["SITE_", "DEF_dir", "DEF_CASE_NAME", "DEF_forcing_namelist"];

fn is_identity(path: &str) -> bool {
    IDENTITY_PREFIXES.iter().any(|p| path.starts_with(p))
}

fn dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    use tauri::Manager;
    let d = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("找不到配置目录：{e}"))?
        .join("presets");
    std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    Ok(d)
}

/// 从一份算例文本里抽出可复用的部分，存成预设。
///
/// 返回被挡下的身份字段，交给界面说明 —— **静默丢弃会让人以为存进去了**，
/// 然后在套用时发现站点没跟着过来。
#[tauri::command]
pub fn save_preset(
    app: tauri::AppHandle,
    name: String,
    text: String,
) -> Result<Vec<String>, String> {
    let name = name.trim();
    if name.is_empty() || name.contains(['/', '\\', '.']) {
        return Err("预设名不能为空，也不能含 / \\ 或点号".into());
    }
    let doc = colm_namelist::parse(&text).map_err(|e| format!("{e:#}"))?;
    let mut fields = Vec::new();
    let mut skipped = Vec::new();
    for p in doc.paths() {
        if is_identity(&p) {
            skipped.push(p);
            continue;
        }
        if let Some(v) = doc.get(&p) {
            fields.push((p, v.to_string()));
        }
    }
    let preset = Preset {
        name: name.to_string(),
        fields,
    };
    let path = dir(&app)?.join(format!("{name}.json"));
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&preset).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(skipped)
}

#[tauri::command]
pub fn list_presets(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    let d = dir(&app)?;
    let mut out: Vec<String> = std::fs::read_dir(&d)
        .map_err(|e| format!("{}: {e}", d.display()))?
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension()?.to_str()? == "json")
                .then(|| p.file_stem()?.to_str().map(str::to_string))?
        })
        .collect();
    out.sort();
    Ok(out)
}

/// 把预设套到一份算例文本上，返回新文本。
///
/// **合并，不覆盖。** 预设里没有的字段原样保留 —— 那正是「存字段列表而不是
/// 整份文件」换来的东西。未改动的行仍然逐字节不变（`colm-namelist` 的往返保证）。
#[tauri::command]
pub fn apply_preset(app: tauri::AppHandle, name: String, text: String) -> Result<String, String> {
    let path = dir(&app)?.join(format!("{name}.json"));
    let raw = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let preset: Preset =
        serde_json::from_str(&raw).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = text;
    for (p, v) in &preset.fields {
        // 逐个走 `set_field`，于是类型校验、大小写不敏感的字段查找、
        // 以及往返保证都与人手改一个字段时完全一样。
        //
        // **一个字段设不上就整份放弃。** `out` 是本地副本，出错时原文一个字节
        // 都没动 —— 而部分套用会留下一个既不是原状也不是预设的状态，
        // 且用户不知道套进去了哪几个。要么全套上，要么什么都没变。
        match crate::config::set_field(out.clone(), p.clone(), strip_quotes(v)) {
            Ok(t) => out = t,
            Err(e) => return Err(format!("套用 {p} 失败：{e}")),
        }
    }
    Ok(out)
}

/// 把一个预设套到一批算例上。
///
/// **先全套完再落盘**，与 `set_field_batch` 同一条规则：半批套上了预设、
/// 半批没有，在界面上与全批套上长得一样，而它们跑出来的东西不一样。
#[tauri::command]
pub fn apply_preset_batch(
    app: tauri::AppHandle,
    name: String,
    dirs: Vec<String>,
) -> Result<crate::config::BatchWrite, String> {
    let mut done = Vec::with_capacity(dirs.len());
    for d in &dirs {
        let p = std::path::Path::new(d).join("case.nml");
        let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        done.push((d.clone(), apply_preset(app.clone(), name.clone(), text)?));
    }
    crate::config::write_all(&done)
}

#[tauri::command]
pub fn delete_preset(app: tauri::AppHandle, name: String) -> Result<(), String> {
    let path = dir(&app)?.join(format!("{name}.json"));
    std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))
}

/// `set_field` 收的是**裸值**（它自己按类型加引号），而 `Value` 的
/// `Display` 会给字符串带上引号。存的是显示形式，套用时要脱一层。
fn strip_quotes(v: &str) -> String {
    v.trim().trim_matches('\'').to_string()
}

#[cfg(test)]
#[path = "presets_tests.rs"]
mod presets_tests;

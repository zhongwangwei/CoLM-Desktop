//! 配置层的进程内命令。
//!
//! 这几个都不碰文件系统之外的东西，也都不需要 netcdf ——
//! `colm-schema` 是一张生成的静态表，`colm-namelist` 是纯文本解析。

use serde::Serialize;

/// 页面加载时确认后端确实接上了。
#[tauri::command]
pub fn backend_ready() -> String {
    format!(
        "backend reachable — {} configuration fields known",
        colm_schema::all().len()
    )
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
        })
        .collect()
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

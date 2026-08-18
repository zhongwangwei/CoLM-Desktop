//! 配置层的进程内命令。
//!
//! 这几个都不碰文件系统之外的东西，也都不需要 netcdf ——
//! `colm-schema` 是一张生成的静态表，`colm-namelist` 是纯文本解析。

use serde::Serialize;

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
        })
        .collect()
}

/// 在给定内核下，哪些字段**用不上**。
///
/// 判据是内核 `manifest.json` 里的 `macros` —— 那是**构建期就写下的事实**，
/// 不是运行时猜的。字段要求的宏有一个不在里面，它在这个内核下就没有意义：
/// 用户设了不会有任何效果，而界面上摆着它只会让人以为设了有用。
///
/// 返回的是**用不上的**那一批，不是能用的。理由：能用的是绝大多数（737 里
/// 有 68 个带依赖），传回小的那一半省事，也让「藏起来的是哪些」这个问题
/// 在界面上答得出来 —— 专家模式要把它们折叠成一行可展开的，
/// **藏起来不等于假装不存在**。
#[tauri::command]
pub fn irrelevant_fields(kernel_dir: String) -> Result<Vec<String>, String> {
    let k = colm_kernel::Kernel::open(std::path::Path::new(&kernel_dir))
        .map_err(|e| format!("{e:#}"))?;
    let have: std::collections::BTreeSet<&str> =
        k.manifest.macros.iter().map(String::as_str).collect();
    Ok(colm_schema::all()
        .iter()
        .filter(|f| !f.requires.is_empty() && !f.requires.iter().all(|m| have.contains(m)))
        .map(|f| f.name.to_string())
        .collect())
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

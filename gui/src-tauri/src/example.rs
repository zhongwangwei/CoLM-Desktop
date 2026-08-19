//! 自带的示例站点。
//!
//! 一个刚装好程序的人手上**没有任何数据**。PLUMBER2 要注册才能下载、
//! 几十 GB，而在拿到数据之前他连"这程序能不能用"都判断不了。
//! 所以装一个站点进去：CN-Cng（内蒙古草地，2008–2009），装完就能跑通
//! 建算例 → 三段运行 → 与观测比对的完整流程。
//!
//! **只有一个，而且是自然站点。** 城市算例的土壤剖面、湖深、土壤反照率、
//! LCZ 分类都只能从全球栅格取，实测那套数据 698 GB —— 装不进任何安装包。

use std::path::{Path, PathBuf};

use tauri::Manager;

/// 示例数据在安装包里的位置。
///
/// 与 `list_kernels` 同一条路子：`bundle.resources` 落在 `resource_dir()`
/// 下（macOS 的 `Contents/Resources/`），而 `externalBin` 落在主二进制旁边
/// —— 两处不同，各按各的来。最后一条是仓库里的位置，`cargo tauri dev`
/// 走那条。
fn source(app: &tauri::AppHandle) -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(p) = std::env::var("COLM_EXAMPLES") {
        roots.push(PathBuf::from(p));
    }
    if let Ok(d) = app.path().resource_dir() {
        roots.push(d.join("examples"));
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples"));
    roots.into_iter().find(|r| r.join("Sitedata").is_dir())
}

/// 装好之后示例在哪。
#[derive(serde::Serialize)]
pub struct Example {
    /// 扫描用的 Sitedata 目录
    pub sitedir: String,
    /// 建议的算例根目录，放在示例旁边
    pub root: String,
    /// 已经在那儿了（这次没复制）
    pub already: bool,
}

/// 把示例数据放到一个**可写**的位置，返回路径。
///
/// 不直接用安装目录里那份：macOS 的 `.app` 与 Windows 的 `Program Files`
/// 都是只读的，而算例目录默认建在站点数据旁边 —— 指着只读目录的话，
/// 用户点「确定」之后拿到的是一个权限错误，而错误信息里根本看不出
/// 问题出在"这是安装目录"。
///
/// 目标是应用数据目录。已经存在就不再复制：那份是用户自己的了，
/// 他可能在里面建了算例，覆盖会把结果删掉。
#[tauri::command]
pub fn install_example(app: tauri::AppHandle) -> Result<Example, String> {
    let src =
        source(&app).ok_or("这个版本没有自带示例数据（examples/ 不在安装包里，也不在仓库里）")?;
    let dest = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("找不到应用数据目录：{e}"))?
        .join("examples");

    let already = dest.join("Sitedata").is_dir();
    if !already {
        copy_tree(&src, &dest)?;
    }
    Ok(Example {
        sitedir: dest.join("Sitedata").display().to_string(),
        root: dest.join("cases").display().to_string(),
        already,
    })
}

/// 递归复制。只有三个目录九个文件，不引第三方 crate。
fn copy_tree(src: &Path, dest: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    for e in std::fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))? {
        let e = e.map_err(|e| e.to_string())?;
        let to = dest.join(e.file_name());
        if e.path().is_dir() {
            copy_tree(&e.path(), &to)?;
        } else {
            std::fs::copy(e.path(), &to).map_err(|err| format!("{}: {err}", to.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "example_tests.rs"]
mod example_tests;

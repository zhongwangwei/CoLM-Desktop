//! 自带的示例站点。
//!
//! 一个刚装好程序的人手上**没有任何数据**。PLUMBER2 要注册才能下载、
//! 几十 GB，而在拿到数据之前他连"这程序能不能用"都判断不了。
//! 所以装两个站点进去：CN-Cng（内蒙古草地，2008–2009）与 AU-Preston
//! （墨尔本城市站）—— 后者是给选了 `urban` 内核的人准备的。
//!
//! **城市那个的数据门槛另说。** 土壤剖面、湖深、LCZ 分类都不在站点文件里，
//! 原本只能从 240 GB 的全球栅格取；那条链正在被拆掉（见 plan-gui3.md 的
//! Task 8c 系列），在它完成之前 AU-Preston 装完还不能直接跑。
//! CN-Cng 则装完就能跑通建算例 → 三段运行 → 与观测比对的完整流程。

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
    /// 建议的算例根目录（`~/CoLM-cases`，不放在示例数据旁边——见 [`cases_root`]）
    pub root: String,
    /// 已经在那儿了（这次没复制）
    pub already: bool,
}

/// 算例根目录**不能**放在 `app_data_dir()` 下。
///
/// macOS 的那个位置是 `~/Library/Application Support/…`，**含空格**，
/// 而 CoLM 用不加引号的 shell `mkdir -p` 建输出树 —— 路径一有空格就被
/// 拆成两个参数，建出一棵影子目录树，最后报一句指向完全错误方向的
/// `Permission denied`。实测踩过：一路点默认值跑不通。
///
/// 换成 `~/CoLM-cases`：既没有空格，也不落在 `~/Documents`——后者在
/// macOS 上是 TCC 保护的目录，一个没经过公证的开发版直接访问会弹出
/// 系统权限对话框，同样打断「装完就能跑」。示例数据本身（`Sitedata` /
/// `Forcing` / `Observation`）不受这条限制：那三样只被 netCDF 按路径
/// 打开读取，不经过 `mkdir -p`，留在 `app_data_dir()` 下没问题
/// （Step 1 实测过：强迫场在含空格路径下三段照样跑通）。
fn cases_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .home_dir()
        .map_err(|e| format!("找不到用户主目录：{e}"))?
        .join("CoLM-cases"))
}

/// 把示例数据放到一个**可写**的位置，返回路径。
///
/// 不直接用安装目录里那份：macOS 的 `.app` 与 Windows 的 `Program Files`
/// 都是只读的，而算例目录默认建在站点数据旁边 —— 指着只读目录的话，
/// 用户点「确定」之后拿到的是一个权限错误，而错误信息里根本看不出
/// 问题出在"这是安装目录"。
///
/// 站点数据的目标是应用数据目录；算例根目录另指到 [`cases_root`]，
/// 见那里为什么不能是同一个地方。两处都是「已经存在就不再复制/不再新建」——
/// 站点数据那份可能被用户改过，算例根目录下可能已经有他自己建的算例，
/// 覆盖或改址都会让人以为东西丢了。
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
    let root = cases_root(&app)?;
    std::fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    Ok(Example {
        sitedir: dest.join("Sitedata").display().to_string(),
        root: root.display().to_string(),
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

//! 自带的示例站点。
//!
//! 一个刚装好程序的人手上**没有任何数据**。PLUMBER2 要注册才能下载、
//! 几十 GB，而在拿到数据之前他连"这程序能不能用"都判断不了。
//! 所以装四个站点进去：CN-Cng（内蒙古草地，2008–2009）、AT-Neu（甲烷，
//! 2010–2012）、AU-Preston（墨尔本城市站）与 US-Ne3（农田站）。
//!
//! AU-Preston 缺少的城市土壤、地形与 LCZ 数据由程序内置点值补齐，
//! 不再要求用户下载 240 GB 全球栅格。
//! CN-Cng 装完即可跑完整流程；AT-Neu 自带甲烷建例与评估数据，
//! 真正运行 BGC / 甲烷前仍需用户指定 CoLM runtime 数据。

use std::path::{Path, PathBuf};

use tauri::Manager;

/// 示例数据在安装包里的位置。
///
/// 与 `list_kernels` 同一条路子：发行版优先读取 `bundle.resources`；开发版
/// 优先读取仓库，避免 `target/debug/examples` 中残留的旧暂存内容遮住新增站点。
fn source(app: &tauri::AppHandle) -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(p) = std::env::var("COLM_EXAMPLES") {
        roots.push(PathBuf::from(p));
    }
    roots.extend(example_roots(app.path().resource_dir().ok()));
    roots.into_iter().find(|r| r.join("Sitedata").is_dir())
}

fn example_roots(resource_dir: Option<PathBuf>) -> Vec<PathBuf> {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let resource = resource_dir.map(|dir| dir.join("examples"));
    if cfg!(debug_assertions) {
        std::iter::once(repository).chain(resource).collect()
    } else {
        resource.into_iter().chain(std::iter::once(repository)).collect()
    }
}

/// 装好之后示例在哪。
#[derive(serde::Serialize)]
pub struct Example {
    /// 扫描用的 Sitedata 目录
    pub sitedir: String,
    /// 与 Sitedata 配套的 Forcing 目录
    pub forcingdir: String,
    /// 建议的算例根目录（`~/CoLM-cases`，不放在示例数据旁边——见 [`cases_root`]）
    pub root: String,
    /// 与 BGC/CROP 配套的最小 Runtime 目录
    pub runtimedir: String,
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

    // 旧版可能只安装了 CN-Cng / AU-Preston。每次都补齐新文件，
    // 但不覆盖用户已经修改过的示例数据。
    let already = [
        "Sitedata/AT-Neu_2010-2012_FLUXNET-CH4_site.nc",
        "Sitedata/US-Ne3_2002-2003_FLUXNET2015_CROP_site.nc",
        "Runtime/ndep/fndep_colm_hist_simyr1849-2006_1.9x2.5_c100428.nc",
    ]
    .iter()
    .all(|path| dest.join(path).is_file());
    copy_tree(&src, &dest)?;
    let root = cases_root(&app)?;
    std::fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    Ok(Example {
        sitedir: dest.join("Sitedata").display().to_string(),
        forcingdir: dest.join("Forcing").display().to_string(),
        root: root.display().to_string(),
        runtimedir: dest.join("Runtime").display().to_string(),
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
        } else if !to.exists() {
            std::fs::copy(e.path(), &to).map_err(|err| format!("{}: {err}", to.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "example_tests.rs"]
mod example_tests;

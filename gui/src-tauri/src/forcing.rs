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
    /// 恒为 8 个元素，对应 `colm_forcing::SLOTS` 的八个槽位。
    pub slots: Vec<SlotGuess>,
    pub steps: usize,
    pub step_seconds: f64,
    pub step_uniform: bool,
    pub time_units: String,
    /// 三个观测高度。源文件没有 `reference_height_*` 时是 `None`
    /// （JSON 里是 `null`），不是 `NaN`。实测 PLUMBER2 的 90 个站全有，
    /// Urban-PLUMBER 的 21 个站全没有 —— 两条路都要覆盖（见测试）。
    pub height_v: Option<f64>,
    pub height_t: Option<f64>,
    pub height_q: Option<f64>,
}

/// 探一份强迫场文件：变量列表、自动猜出来的槽位映射、时间轴、高度。
///
/// **只探不改。** 用户要先看到猜的结果、能改，才允许转换 ——
/// 变量名猜错的后果是「跑得完、结果全错」，而曲线照样是曲线，
/// 界面上什么都看不出来。
#[tauri::command]
pub async fn probe_forcing(path: String) -> Result<Probe, String> {
    let json =
        crate::sidecar::capture(&["forcing-probe".into(), path, "--json".into(), "1".into()])?;
    serde_json::from_str(&json).map_err(|e| {
        // 说清楚是**解析**失败而不是探测失败 —— 照 `scan_sites` 的措辞。
        // 两者的处置完全不同：前者是我们两边的结构体对不上了，
        // 后者是用户给的文件有问题。
        format!("colm-cli forcing-probe 的输出解析不了（两边的字段可能已经对不上）：{e}")
    })
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
    crate::sidecar::capture(&args)?;
    Ok(dst)
}

#[cfg(test)]
#[path = "forcing_tests.rs"]
mod forcing_tests;

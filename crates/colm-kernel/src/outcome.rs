//! 判定一段 CoLM 运行是成功还是失败。
//!
//! 退出码不是证据。实测（设计文档 §2.4）：
//!
//! - namelist 文件不存在：退出码 2
//! - namelist 里有未声明的变量：退出码 0
//! - 缺 rawdata、NetCDF 打不开：退出码 0
//! - 时间窗非法、malloc failure：退出码 0
//!
//! 因此判定必须同时满足三件事：无错误标记、有正向成功标记、产物齐全。

use std::path::{Path, PathBuf};

/// CoLM 单点流程的三段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    MkSrfData,
    MkIniData,
    Colm,
}

impl Stage {
    /// 该段成功时必然打印的行。缺了它就不算成功，无论退出码是什么。
    pub fn success_marker(self) -> &'static str {
        match self {
            Stage::MkSrfData => "Successful in surface data making.",
            Stage::MkIniData => "CoLM Initialization Execution Completed",
            Stage::Colm => "CoLM Execution Completed.",
        }
    }

    /// 可执行文件名（不含平台后缀）。
    pub fn program(self) -> &'static str {
        match self {
            Stage::MkSrfData => "mksrfdata",
            Stage::MkIniData => "mkinidata",
            Stage::Colm => "colm",
        }
    }
}

/// 出现即判失败的子串。
///
/// **顺序是语义的**：同一行可能命中多个标记，报告的是第一个命中的，
/// 所以具体的标记必须排在笼统的之前。实例：CoLM 那行
/// `ERROR in /x.nml : Cannot match namelist object name def_foo`
/// 同时含 `ERROR in` 与 `Cannot match namelist object name`，
/// 后者信息量大得多，必须先命中。
///
/// 新增条目前先确认它不会命中 `BENIGN_LINES` 里的行。
const FAILURE_MARKERS: &[&str] = &[
    // 具体
    "Cannot match namelist object name",
    "Memory allocation (malloc) failure",
    "Fortran runtime error",
    "Error termination",
    // 能量/水量平衡越界。`CoLMDEBUG` 下 `CoLMMAIN.F90:1545` 与 `:1620` 会打印
    // `Warning: ... balance violation ...` 然后**继续跑** —— 与 RangeCheck 不同，
    // 这里没有 `CoLM_stop`。（design.md §6.5 原先写它「同样走 CoLM_stop」，是错的，
    // 已就地改正。）
    //
    // 于是一次能量不守恒的运行会跑到底并被判成功，而它的输出是错的。
    // §6.5 定的政策是「宁可炸也不要给出错的数」—— CoLM 自己不执行，就得这里执行。
    // 十种消息文本共享 `balance violation` 这一个子串，一条标记全覆盖。
    // 实测两次健康运行的 colm.log 里零次出现。
    //
    // 注意它以 `Warning:` 开头，所以 `overrides::extract` 也会把它列出来。
    // 那是刻意的：抽取只认前缀、原样上报，不去解释文本（见 overrides.rs）。
    // 判成败在这里，呈现在那里，两边说的是同一行。
    "balance violation",
    // RangeCheck 判定状态量有 NaN 或越界时，往那一行行尾追加的两句话
    // （`MOD_RangeCheck.F90:139,144`，六处 subroutine 各一份）。
    //
    // 定义了 `CoLMDEBUG` 时 RangeCheck 自己会 `CoLM_stop(' ***** ERROR: ...')`，
    // 那条已被下面的 `***** ERROR` 抓住。这里再列一遍是为了**把检测与那个
    // 编译期宏解耦**：预设若不带 `CoLMDEBUG`（§6.5 说它是「默认武装」，
    // 也就是可改的），RangeCheck 仍然会打印这两句，但不再中止 ——
    // 那样一次带 NaN 的运行会跑到底、写出产物、打出成功标记，被判成功。
    // 实测健康运行的 39215 行 colm.log 里这两句零次出现，所以不花代价。
    " with NAN",
    " Out of Range!",
    // 笼统
    "Netcdf error",
    "***** ERROR",
    "ERROR in",
    // `MOD_NetCDFSerial.F90:163`：输入文件不存在时打印这一句，然后
    // **无参数**调 `CoLM_stop()` —— 不打印任何别的东西，单点下退出码还是 0。
    // 少了这条标记，缺文件只会以「成功标记没出现」的形式被抓到，
    // 而用户需要看见的是缺了哪个文件。
    //
    // 唯一的无害来源是没配 `DEF_HIST_vars_namelist` 时那一行，见 `BENIGN_LINES`。
    // 注意豁免是整行精确匹配 `null` 那一版：用户若把它指向一个不存在的路径，
    // CoLM 会静默回落到默认变量集 —— 那正该判失败。
    "does not exist",
];

/// 长得像失败但无害的整行。逐行**完全匹配去空白后**的文本，不做子串匹配，
/// 以免一条宽松的豁免掩盖真实错误。
const BENIGN_LINES: &[&str] = &[
    // 没有设置 DEF_HIST_vars_namelist 时必然出现
    "History namelist file: null does not exist.",
];

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Succeeded,
    Failed(Failure),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    /// 进程以非零状态退出。
    NonZeroExit { status: i32, last_line: String },
    /// stdout 命中了已知的失败标记。
    ErrorMarker { marker: &'static str, line: String },
    /// 该段的成功标记从未出现。
    MissingSuccessMarker(Stage),
    /// 该段应产出的文件不存在。
    MissingArtifact(PathBuf),
}

/// 判定一段运行的结果。
///
/// `exit_status` 为 `None` 表示进程被信号终止（例如用户取消）。
/// `artifacts` 是该段必须产出的文件；顺序即检查顺序，第一个缺失者被报告。
pub fn adjudicate(
    stage: Stage,
    exit_status: Option<i32>,
    stdout: &str,
    artifacts: &[PathBuf],
) -> Outcome {
    // 1. 非零退出：直接失败。零退出**不构成**成功的证据。
    match exit_status {
        Some(0) => {}
        Some(status) => {
            return Outcome::Failed(Failure::NonZeroExit {
                status,
                last_line: last_nonempty_line(stdout).to_string(),
            });
        }
        None => {
            return Outcome::Failed(Failure::NonZeroExit {
                status: -1,
                last_line: last_nonempty_line(stdout).to_string(),
            });
        }
    }

    // 2. 错误标记扫描，逐行进行，先排除无害行。
    for line in stdout.lines() {
        if is_benign(line) {
            continue;
        }
        if let Some(marker) = FAILURE_MARKERS.iter().find(|m| line.contains(**m)) {
            return Outcome::Failed(Failure::ErrorMarker {
                marker,
                line: line.trim().to_string(),
            });
        }
    }

    // 3. 正向成功标记必须出现。
    if !stdout.contains(stage.success_marker()) {
        return Outcome::Failed(Failure::MissingSuccessMarker(stage));
    }

    // 4. 产物硬校验。
    for path in artifacts {
        if !path_is_present(path) {
            return Outcome::Failed(Failure::MissingArtifact(path.clone()));
        }
    }

    Outcome::Succeeded
}

fn is_benign(line: &str) -> bool {
    BENIGN_LINES.contains(&line.trim())
}

fn path_is_present(path: &Path) -> bool {
    path.exists()
}

fn last_nonempty_line(stdout: &str) -> &str {
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
}

#[cfg(test)]
#[path = "outcome_tests.rs"]
mod outcome_tests;

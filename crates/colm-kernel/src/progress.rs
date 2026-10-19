//! 运行日志里的阶段标记与模拟步。窗口进程边跑边认、远程作业从日志末尾认、
//! `colm-cli` 记台账时认最终进度，都用这一份：两处各写一份的话，哪一行算步数迟早会分叉。

/// `TIMESTEP = 1 | DATE = 2008-01-01-00000` -> 步数与日期。
///
/// 预热期间 CoLM 用另一种 format，行尾多一段 `Spinup (cycle 1 of 3)`
/// （`CoLM.F90:747` 与 `:749` 是两条不同的 format 语句）。两种都要认，
/// 否则那段尾巴会整个留在 `date` 里 —— 不崩，但界面分不出正在预热。
///
/// `case` 与 `stage` 由调用方补：这个函数只认得一行文本。
pub struct Step {
    pub step: u64,
    pub date: String,
    pub spinup: Option<(u32, u32)>,
}

pub fn parse_progress(line: &str) -> Option<Step> {
    let rest = line.trim().strip_prefix("TIMESTEP =")?;
    let (step, date) = rest.split_once('|')?;
    let step: u64 = step.trim().parse().ok()?;
    let rest = date.trim().strip_prefix("DATE =")?.trim();
    // 日期本身不含空格，所以第一个空格就是它的边界。
    let (date, tail) = match rest.split_once(' ') {
        Some((d, t)) => (d, t.trim()),
        None => (rest, ""),
    };
    let spinup = tail
        .strip_prefix("Spinup (cycle ")
        .and_then(|s| s.strip_suffix(')'))
        .and_then(|s| s.split_once(" of "))
        .and_then(|(a, b)| Some((a.trim().parse().ok()?, b.trim().parse().ok()?)));
    Some(Step {
        step,
        date: date.to_string(),
        spinup,
    })
}

/// 日志里最后一个模拟步：（步号，日期）。
pub type LastStep = (u64, String);

/// 把一段日志（例如远程作业的日志末尾）解析成各阶段的最新状态与最后一个模拟步：
/// 与本机运行用同一套 `parse_stage` / `parse_progress`。
pub fn parse_log(log: &str) -> (Vec<(String, String)>, Option<LastStep>) {
    let mut stages: Vec<(String, String)> = Vec::new();
    let mut progress = None;
    for line in log.lines() {
        if let Some((name, state)) = parse_stage(line) {
            match stages.iter_mut().find(|(n, _)| *n == name) {
                Some(entry) => entry.1 = state,
                None => stages.push((name, state)),
            }
        } else if let Some(step) = parse_progress(line) {
            progress = Some((step.step, step.date));
        }
    }
    (stages, progress)
}

/// `=== colm-stage mksrfdata begin ===` -> `("mksrfdata", "begin")`。
///
/// 标记由 `colm-cli run --stream` 自己打。实测 CoLM 的 34180 行输出里
/// 没有一行以 `===` 开头，也没有一处出现 `colm-stage`，所以不会撞。
pub fn parse_stage(line: &str) -> Option<(String, String)> {
    let s = line
        .trim()
        .strip_prefix("=== colm-stage ")?
        .strip_suffix(" ===")?;
    let (name, state) = s.rsplit_once(' ')?;
    Some((name.trim().to_string(), state.trim().to_string()))
}

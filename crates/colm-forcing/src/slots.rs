//! CoLM 的八个强迫场槽位，以及**从文件实际有的变量**把它们填上。
//!
//! 槽位本身是固定的（`MOD_UserSpecifiedForcing.F90`）：
//! `1=T 2=q 3=psrf 4=precip 5=u 6=v 7=SW 8=LW`。
//! 变化的是每个数据集用什么名字，以及**风是标量还是分量**。
//!
//! 里程碑 4 把「第 5 槽是 `NULL`」当成了 POINT 数据集的固有属性写死。
//! 实测那只是 **PLUMBER2** 的属性：它只有一个标量 `Wind`，进第 6 槽，
//! 第 5 槽空着。而 Urban-PLUMBER 给 `Wind_E` / `Wind_N` 两个分量，
//! 第 5 槽是实打实的东风分量。写死的话另一个数据集就用不了。
//!
//! 实测两个语料的差异：
//!
//! | 槽 | PLUMBER2 | Urban-PLUMBER |
//! |---|---|---|
//! | 3 | `Psurf` | `PSurf`（大小写不同） |
//! | 4 | `Precip` | `Rainf` |
//! | 5 | *（无）* | `Wind_E` |
//! | 6 | `Wind`（标量） | `Wind_N` |

/// 一个槽位：CoLM 里的位置、含义、以及各数据集用过的名字。
pub struct Slot {
    /// 1-based，与 CoLM 的 `vname(i)` 对齐
    pub index: usize,
    pub meaning: &'static str,
    /// 候选名，**按优先级**。第一个在文件里出现的胜出。
    pub candidates: &'static [&'static str],
    /// 时间插值算法。降水用 `nearest`，其余 `linear` —— 对累积量做线性插值
    /// 会把一场雨抹平到相邻时段上。
    pub tintalgo: &'static str,
    /// 空着是否可以。只有第 5 槽（u 风）可以 —— 标量风数据集没有它。
    pub optional: bool,
}

pub const SLOTS: [Slot; 8] = [
    Slot {
        index: 1,
        meaning: "air temperature",
        candidates: &["Tair"],
        tintalgo: "linear",
        optional: false,
    },
    Slot {
        index: 2,
        meaning: "specific humidity",
        candidates: &["Qair"],
        tintalgo: "linear",
        optional: false,
    },
    Slot {
        index: 3,
        meaning: "surface pressure",
        candidates: &["Psurf", "PSurf"],
        tintalgo: "linear",
        optional: false,
    },
    Slot {
        index: 4,
        meaning: "precipitation",
        candidates: &["Precip", "Rainf"],
        tintalgo: "nearest",
        optional: false,
    },
    Slot {
        index: 5,
        meaning: "eastward wind",
        candidates: &["Wind_E"],
        tintalgo: "linear",
        optional: true,
    },
    Slot {
        index: 6,
        meaning: "northward or scalar wind",
        candidates: &["Wind_N", "Wind"],
        tintalgo: "linear",
        optional: false,
    },
    Slot {
        index: 7,
        meaning: "downward shortwave",
        candidates: &["SWdown"],
        tintalgo: "linear",
        optional: false,
    },
    Slot {
        index: 8,
        meaning: "downward longwave",
        candidates: &["LWdown"],
        tintalgo: "linear",
        optional: false,
    },
];

/// 八个槽位解析之后的结果。`None` 表示那一槽写 `NULL`。
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub vname: [Option<&'static str>; 8],
}

impl Resolved {
    /// 写进 namelist 的名字，空槽是 `NULL`。
    pub fn names(&self) -> [&'static str; 8] {
        let mut out = ["NULL"; 8];
        for (i, v) in self.vname.iter().enumerate() {
            if let Some(n) = v {
                out[i] = n;
            }
        }
        out
    }

    /// 对应的插值算法，空槽也是 `NULL`。
    pub fn tintalgo(&self) -> [&'static str; 8] {
        let mut out = ["NULL"; 8];
        for (i, v) in self.vname.iter().enumerate() {
            if v.is_some() {
                out[i] = SLOTS[i].tintalgo;
            }
        }
        out
    }

    /// 风是分量的还是标量的。分量风两槽都有；标量风只有第 6 槽。
    pub fn wind_is_vector(&self) -> bool {
        self.vname[4].is_some()
    }
}

/// 按文件里实际有的变量填槽位。
///
/// **总是返回一个 `Resolved`**，同时把缺失的必填槽列出来。不返回 `Result`
/// 是因为两个调用方需要的东西不同：`check` 要那份问题清单，`render` 要
/// 尽可能填好的槽位表（它返回 `String`，没有报错的地方）。缺的槽会渲染成
/// `NULL`，而 CoLM 对必填槽是 `NULL` 会明确拒绝 —— 所以即便调用方忘了先
/// `check`，也不会静默跑出错误结果。
pub fn resolve(variables: &[String]) -> (Resolved, Vec<String>) {
    resolve_with(variables, &[])
}

/// 与 `resolve` 相同，但允许用户为某些槽位**指定**变量名。
///
/// `overrides` 是 `(槽位序号 1-based, 变量名)`。指定的名字文件里没有时
/// **报错而不是回落到自动匹配** —— 回落会让用户以为自己选了 A、
/// 实际跑的是 B，而那是「跑得完却给出错误结果」的典型。
///
/// `resolve` 保留为 `resolve_with(vars, &[])` 的薄封装：现有调用点不动。
pub fn resolve_with(
    variables: &[String],
    overrides: &[(usize, String)],
) -> (Resolved, Vec<String>) {
    let has = |n: &str| variables.iter().any(|v| v == n);
    let mut vname = [None; 8];
    let mut missing = Vec::new();

    for (i, s) in SLOTS.iter().enumerate() {
        // 用户指定优先。
        if let Some((_, name)) = overrides.iter().find(|(idx, _)| *idx == s.index) {
            if has(name) {
                // 名字来自调用方而不是 'static 表，所以要 leak 成 'static。
                // 这条路径每次运行只走 8 次，代价可以忽略。
                vname[i] = Some(Box::leak(name.clone().into_boxed_str()) as &'static str);
            } else {
                missing.push(format!(
                    "slot {} ({}) was told to use {:?}, which the file does not have",
                    s.index, s.meaning, name
                ));
            }
            continue;
        }
        match s.candidates.iter().find(|c| has(c)) {
            Some(c) => vname[i] = Some(*c),
            None if s.optional => {}
            None => missing.push(format!(
                "slot {} ({}) has none of {:?}",
                s.index, s.meaning, s.candidates
            )),
        }
    }
    (Resolved { vname }, missing)
}

#[cfg(test)]
#[path = "slots_tests.rs"]
mod slots_tests;

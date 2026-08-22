//! 从 CoLM 源码扫两件事：**字段的合法取值**，与**字段依赖哪些编译期宏**。
//!
//! 两者都只能从上游源码得到，而上游会变 —— 所以生成、入库、由 drift 测试守住，
//! 与 `colm-schema` 的字段表、`colm-hist` 的闸门表同一套办法。
//!
//! 扫的是 `vendor/CoLM202X` 下**除 `MOD_Namelist.F90` 之外**的全部 `.F90`。
//! 排除它是必须的：每个字段都在那里声明并出现在 `namelist` 语句里，
//! 一并扫的话「这个字段用在哪」这个问题的答案永远是「到处都用」。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Result;

/// 可选子系统：目录或文件名归属 -> 需要的宏。
///
/// 这是对「调用点被守」的一个近似——不完美（见 `curated`），但一次覆盖
/// 一批字段，且随上游加文件自动跟上。
///
/// **没有 `main/TRACER/` 条目。** TRACER 曾经在这里（宏开着才编得进去），
/// 但示踪物子系统改成运行时开关后，`main/TRACER/` 下的模块永远编译进去，
/// 不再有对应的编译期宏——这个目录本身不再暗示任何 `requires`。
/// `DEF_TRACER_*` 字段现在在每个内核下都可设，用户自己用运行时开关
/// `DEF_USE_TRACER`（MOD_Namelist.F90）决定要不要用，那不是这张表管的事：
/// 这张表只记录「编译期这个字段有没有意义」，不记录「运行时它生效吗」。
///
/// **同样没有 `main/BGC/` 条目。** LULC/BGC/CROP/URBAN/LULCC 那一组改造把
/// BGC 也变成运行时开关了（`DEF_USE_BGC`，MOD_Namelist.F90，默认
/// `.false.`）——`main/BGC/` 下的模块永远编译进去，`BGC` 这个宏本身从
/// `include/define.h`（`create_defineh.bash`）里彻底消失，这个目录同样
/// 不再暗示任何 `requires`。`main/URBAN/` 同理不在这里也不在 `BY_NAME`
/// 里——见下面 `BY_NAME` 的注释。`main/LULCC/` 从来没进过这张表（LULCC
/// 从来没被任何内核预设打开过），现在也一样——`DEF_USE_LULCC` 不需要
/// 任何宏。`CROP` 与 `LULC_USGS`/`LULC_IGBP` 不在这张表里，因为它们
/// **仍然是**编译期宏（N_PFT/N_CFT、N_land_classification 这类
/// `parameter` 数组尺寸不同，见 docs/plan-macro-runtime.md）——它们该有
/// 的 `requires` 由下面的通用 `#ifdef` 扫描直接抓到，不需要目录级近似。
const SUBSYSTEMS: &[(&str, &str)] = &[("main/DA/", "DataAssimilation"), ("CaMa/", "CaMa_Flood")];

/// 文件名里带这些词的，归到对应的宏。目录分不出来时用它。
///
/// **没有 `Urban` 条目。** URBAN_MODEL 同一批改造里也变成运行时开关了
/// （`DEF_URBAN_RUN`，已经存在，不再被 `#ifdef URBAN_MODEL` 强制赋值）——
/// `main/URBAN/` 下的文件名依然带 `Urban`，但那不再对应任何编译期宏，
/// 用文件名猜测只会把 `DEF_URBAN_*` 字段错误地标成「需要 URBAN_MODEL
/// 才有意义」，而实际上它们在每个内核下都可设，运行时开关决定要不要用。
const BY_NAME: &[(&str, &str)] = &[("Catch", "CATCHMENT")];

/// 词法扫描看不出、但确实只在某个宏下才有意义的字段。
///
/// **每一条都必须写出为什么。** 手工表会烂掉，除非它自己能发现自己烂了 ——
/// 第三列是 `文件:行`，`crates/colm-schema/tests/curated.rs` 会去读那一行，
/// 确认它仍然包含所声明的守护。上游把那个 `#ifdef` 挪走时测试红，
/// 而不是界面悄悄多显示一个没用的字段。
///
/// **目前是空的。** 原先唯一的一条——`DEF_URBAN_type_scheme` 需要
/// `URBAN_MODEL`，出处是 `landurban_build` 的调用点——在 LULC/BGC/CROP/
/// URBAN/LULCC 那组改造里失效了：`landurban_build` 的调用点从
/// `#ifdef URBAN_MODEL` 改成了运行时 `IF (DEF_URBAN_RUN) THEN`
/// （mksrfdata/MKSRFDATA.F90），`URBAN_MODEL` 本身也从 `include/define.h`
/// 里彻底消失（create_defineh.bash 现在无条件 `#define` 它）。
/// `DEF_URBAN_type_scheme` 现在在每个内核下都可设，不再需要人工表兜底。
pub const CURATED: &[(&str, &str, &str, &str)] = &[];

/// 数值枚举没有字符串引号，通用扫描器不能把任意 `== 1` 误判成完整取值域：
/// 很多整数同时也用于阈值、计数或数组下标。这里仅列源码声明注释或完整分支明确
/// 给出全部方案的字段。字符型但完整分支跨过扫描器 60 行窗口的 SSP，以及用
/// `ELSE` 表达第二种方案的长波降尺度也在这里补全。
///
/// 出处集中在 `MOD_Namelist.F90` 的方案说明，以及对应实现中的完整 CASE/IF：
/// `MOD_SoilSurfaceResistance.F90`、`MOD_SoilThermalParameters.F90`、
/// `MOD_SoilSnowHydrology.F90`、`MOD_RainSnowTemp.F90`、`MOD_ForcingDownscaling.F90`、
/// `MOD_Irrigation.F90`、`MOD_Eroot.F90`、`MOD_CropReadin.F90` 和 `main/DA/MOD_DA_RTM.F90`。
const CURATED_VALUES: &[(&str, &[&str])] = &[
    ("DEF_SOIL_REFL_SCHEME", &["1", "2"]),
    ("DEF_LULCC_SCHEME", &["1", "2"]),
    ("DEF_URBAN_type_scheme", &["1", "2"]),
    (
        "DEF_Interception_scheme",
        &["1", "2", "3", "4", "5", "6", "7", "8"],
    ),
    (
        "DEF_THERMAL_CONDUCTIVITY_SCHEME",
        &["1", "2", "3", "4", "5", "6", "7", "8"],
    ),
    ("DEF_RSS_SCHEME", &["0", "1", "2", "3", "4", "5"]),
    ("DEF_Runoff_SCHEME", &["0", "1", "2", "3"]),
    ("DEF_TOPMOD_method", &["0", "1", "2"]),
    ("DEF_NDEP_FREQUENCY", &["1", "2"]),
    ("DEF_Reservoir_Method", &["0", "1"]),
    (
        "DEF_wetland_finundation_scheme",
        &["1", "2", "3", "4", "5", "6", "7"],
    ),
    ("DEF_SSP", &["126", "245", "370", "585", "off"]),
    ("DEF_IRRIGATION_ALLOCATION", &["1", "2", "3"]),
    ("DEF_RSTFAC", &["1", "2"]),
    ("DEF_FERT_SOURCE", &["1", "2"]),
    ("DEF_DA_RTM_diel", &["0", "1", "2", "3"]),
    ("DEF_DA_RTM_rough", &["0", "1", "2", "3"]),
    ("DEF_DS_longwave_adjust_scheme", &["I", "II"]),
    (
        "DEF_WRST_FREQ",
        &["none", "TIMESTEP", "HOURLY", "DAILY", "MONTHLY", "YEARLY"],
    ),
    (
        "DEF_HIST_FREQ",
        &["none", "TIMESTEP", "HOURLY", "DAILY", "MONTHLY", "YEARLY"],
    ),
];

/// 扫出来的两张表。
#[derive(Default)]
pub struct Usage {
    /// 字段 -> 合法取值（有序、去重）
    pub values: BTreeMap<String, Vec<String>>,
    /// 字段 -> 需要的宏（有序、去重）
    pub requires: BTreeMap<String, Vec<String>>,
}

pub fn scan(root: &Path) -> Result<Usage> {
    let mut values: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    // 字段 -> 它出现过的「守护宏集合」的集合。空集合出现过一次，
    // 就说明它在某处**没有**被守，那么它在任何配置下都可能被用到。
    let mut guards: BTreeMap<String, Vec<BTreeSet<String>>> = BTreeMap::new();
    let mut files: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for path in walk(root)? {
        let is_namelist = path.file_name().and_then(|n| n.to_str()) == Some("MOD_Namelist.F90");
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let text = std::fs::read_to_string(&path)?;
        // **取值要扫 MOD_Namelist.F90，守护不能扫它** —— 两个问题的范围不同。
        //
        // 取值：几个枚举的规范取值只写在那里的校验分支里，连大小写归一一起做
        // （`CASE ('CAPPA2003', 'Cappa2003', 'cappa2003')`）。排除它就整个丢失。
        //
        // 守护：每个字段都在那里声明并出现在 `namelist` 语句里，
        // 一并扫的话「这个字段用在哪」的答案永远是「到处都用」，全部判成无守护。
        scan_values(&text, &mut values);
        if is_namelist {
            continue;
        }

        let mut stack: Vec<String> = Vec::new();
        for line in text.lines() {
            if let Some(m) = ifdef_macro(line) {
                stack.push(m);
                continue;
            }
            let t = line.trim_start();
            if t.starts_with("#endif") || t.starts_with("# endif") {
                stack.pop();
                continue;
            }
            // `#else` 之后那一段的守护条件是**反过来的**，也就是「这个宏没开」。
            // 我们只关心「开了才有」，所以整段按无守护处理 —— 保守方向：
            // 宁可多显示一个字段，也不要把一个有用的字段藏起来。
            if t.starts_with("#else") || t.starts_with("# else") {
                stack.pop();
                stack.push(String::new());
                continue;
            }
            for name in def_names(line) {
                let g: BTreeSet<String> = stack.iter().filter(|s| !s.is_empty()).cloned().collect();
                guards.entry(name.clone()).or_default().push(g);
                files.entry(name).or_default().insert(rel.clone());
            }
        }
    }

    let mut requires: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, gs) in &guards {
        // 有任何一处无守护 -> 这个字段不由宏决定
        if gs.iter().any(|g| g.is_empty()) {
            // 但仍可能整体落在某个可选子系统里
            if let Some(m) = subsystem_macro(files.get(name)) {
                requires.insert(name.clone(), vec![m]);
            }
            continue;
        }
        // 全部用法都被守着。取所有守护集合的**交集** —— 只有每一处都要求的
        // 宏才是真正的前提；某一处额外要求的宏不是。
        let mut it = gs.iter();
        let mut common = it.next().cloned().unwrap_or_default();
        for g in it {
            common = common.intersection(g).cloned().collect();
        }
        if !common.is_empty() {
            requires.insert(name.clone(), common.into_iter().collect());
        } else if let Some(m) = subsystem_macro(files.get(name)) {
            requires.insert(name.clone(), vec![m]);
        }
    }
    for (name, macro_, _, _) in CURATED {
        requires
            .entry((*name).to_string())
            .or_insert_with(|| vec![(*macro_).to_string()]);
    }
    for (name, allowed) in CURATED_VALUES {
        values
            .entry((*name).to_string())
            .or_default()
            .extend(allowed.iter().map(|v| (*v).to_string()));
    }

    Ok(Usage {
        // **只有一个取值的不是枚举**，是一次哨兵比较（`trim(DEF_file_GIEMS)
        // == 'null'` 那种）。留着它，界面会给一个只有一项的下拉框，
        // 而那比文本框更糟 —— 它宣称自己知道全部合法取值，其实不知道。
        values: values
            .into_iter()
            .map(|(k, v)| (k, canonical(v)))
            .filter(|(_, v)| v.len() >= 2)
            .collect(),
        requires,
    })
}

/// 大小写别名合并成一个选项。
///
/// `MOD_Namelist.F90` 的校验分支会同时接受三种写法
/// （`CASE ('CAPPA2003', 'Cappa2003', 'cappa2003')`），但那是**同一个选择**。
/// 不合并的话下拉框会给出 6 个选项而实际只有 2 种。
///
/// 保留全大写那个：紧跟在 `CASE` 之后的赋值写的就是它
/// （`DEF_TRACER_KINETIC_SCHEME = 'CAPPA2003'`），也与其余枚举的风格一致
/// （`HOURLY` / `POINT` / `DAILY`）。没有全大写变体时按字典序取第一个 ——
/// `day`/`month`/`year` 那种本来就没有别名，取哪个都一样。
fn canonical(vals: BTreeSet<String>) -> Vec<String> {
    let mut by_lower: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for v in vals {
        by_lower.entry(v.to_ascii_lowercase()).or_default().push(v);
    }
    let mut out: Vec<String> = by_lower
        .into_values()
        .map(|mut group| {
            group.sort();
            group
                .iter()
                .find(|v| v.chars().all(|c| !c.is_alphabetic() || c.is_uppercase()))
                .cloned()
                .unwrap_or_else(|| group[0].clone())
        })
        .collect();
    out.sort();
    out
}

fn subsystem_macro(files: Option<&BTreeSet<String>>) -> Option<String> {
    let files = files?;
    if files.is_empty() {
        return None;
    }
    for (frag, m) in SUBSYSTEMS {
        if files.iter().all(|f| f.contains(frag)) {
            return Some((*m).to_string());
        }
    }
    for (word, m) in BY_NAME {
        if files.iter().all(|f| f.contains(word)) {
            return Some((*m).to_string());
        }
    }
    None
}

/// `#ifdef X` / `#if defined(X)` -> `Some("X")`。`#ifndef` 不算 ——
/// 那是「没开这个宏才有」，与「开了才有」是相反的判据。
fn ifdef_macro(line: &str) -> Option<String> {
    let t = line.trim_start();
    let rest = t.strip_prefix('#')?.trim_start();
    if let Some(r) = rest.strip_prefix("ifdef") {
        return Some(r.split_whitespace().next()?.to_string());
    }
    if let Some(r) = rest.strip_prefix("ifndef") {
        let _ = r;
        return Some(String::new()); // 占位，保持嵌套深度对齐
    }
    if let Some(r) = rest.strip_prefix("if") {
        let r = r.trim();
        if let Some(d) = r.strip_prefix("defined") {
            let d = d.trim().trim_start_matches('(').trim();
            let name: String = d
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            // `#if (defined A || defined B)` 是「二选一」，不是「都要」——
            // 当成单个宏会把只开了 B 的配置误判成不相关。整段按无守护处理。
            if r.contains("||") {
                return Some(String::new());
            }
            if !name.is_empty() {
                return Some(name);
            }
        }
        return Some(String::new());
    }
    None
}

fn def_names(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b = line.as_bytes();
    let mut i = 0;
    while i + 4 <= b.len() {
        if &b[i..i + 4] == b"DEF_"
            && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_'))
        {
            let mut j = i + 4;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                j += 1;
            }
            out.push(line[i..j].to_string());
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// 两种写法都扫。**只扫 `select case` 会漏掉 7 个字段**
/// （`DEF_HIST_groupby`、`DEF_forcing%dataset` 等都是 `==` 比较）。
fn scan_values(text: &str, out: &mut BTreeMap<String, BTreeSet<String>>) {
    let lines: Vec<&str> = text.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        // `SELECT CASE (trim(adjustl(DEF_x)))`，也写作 `SELECTCASE(...)`。
        // 先按「去掉空格再小写」判形状，再从原文里取名字 —— 两步分开，
        // 免得为了兼容写法差异去猜下标（第一版就是这么猜错的：
        // TRACER 那几个字段只扫到一个取值）。
        let flat = line.to_ascii_lowercase().replace(' ', "");
        if flat.contains("selectcase(trim(adjustl(def_") || flat.contains("selectcase(trim(def_") {
            let Some(start) = line.find("DEF_") else {
                continue;
            };
            let name: String = line[start..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '%')
                .collect();
            for l in lines.iter().skip(i + 1).take(60) {
                let tl = l.trim().to_ascii_lowercase().replace(' ', "");
                if tl.starts_with("endselect") {
                    break;
                }
                if tl.starts_with("case(") {
                    for v in quoted(l) {
                        out.entry(name.clone()).or_default().insert(v);
                    }
                }
            }
        }
        // `trim(DEF_x) == 'v'` / `trim(adjustl(DEF_x)) == 'v'`
        let mut k = 0;
        while let Some(p) = line[k..].find("trim(") {
            let after = k + p + 5;
            let rest = line[after..].trim_start().trim_start_matches("adjustl(");
            if !rest.starts_with("DEF_") {
                k = after;
                continue;
            }
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '%')
                .collect();
            if let Some(eq) = line[after..].find("==") {
                let tail = &line[after + eq..];
                if let Some(v) = quoted(tail).into_iter().next() {
                    out.entry(name).or_default().insert(v);
                }
            }
            k = after;
        }
    }
}

fn quoted(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut it = s.char_indices();
    while let Some((i, c)) = it.next() {
        if c == '\'' {
            if let Some(end) = s[i + 1..].find('\'') {
                out.push(s[i + 1..i + 1 + end].to_string());
                for _ in 0..s[i + 1..=i + 1 + end].chars().count() {
                    it.next();
                }
            }
        }
    }
    out
}

/// 走 `vendor/CoLM202X` 下的 `.F90`，**跳过 `tests/`**。
///
/// 跳过它不是嫌它无关，是因为它**不在库里**：`vendor/CoLM202X/.gitignore`
/// 第 21 行写着 `/tests`，那 13 个 `.F90` 谁的工作树里有就有、克隆出来
/// 一定没有。扫进去的话，生成表就带上了只有本机看得见的东西 —— 本地
/// `gen-schema` 与入库产物一致，CI 上重新生成却少了那部分，于是
/// `colm-schema` 的 drift 测试在 macOS 与 Windows 上同时红（实测 ci run
/// 32445644279，报 "generated.rs is out of date with MOD_Namelist.F90"，
/// 而 `MOD_Namelist.F90` 那次根本没动）。生成物入库就必须只由入库的输入
/// 决定，否则 drift 守的不再是「上游改了没重新生成」，而是「谁最后跑的
/// 生成器」。
///
/// 只挡顶层那一个 `tests`（`.gitignore` 的 `/tests` 也只挡顶层），不是
/// 见到叫 tests 的目录就跳 —— 上游哪天在子目录里放入库的 `tests/`，
/// 那份该扫。
fn walk(dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    let untracked_tests = dir.join("tests");
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().and_then(|n| n.to_str()) != Some(".git") && p != untracked_tests {
                    stack.push(p);
                }
            } else if p.extension().and_then(|x| x.to_str()) == Some("F90") {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}

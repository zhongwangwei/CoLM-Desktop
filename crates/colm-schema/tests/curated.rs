//! 人工列出的宏依赖，每一条都要能证明自己还成立。
//!
//! `xtask/src/usage.rs` 的 `CURATED` 里每条都带出处（文件 + 那一行必须包含
//! 的文本）。手工表一定会烂 —— **除非它自己能发现自己烂了**。
//! 上游把那个守护挪走时，这里红；而不是界面悄悄多显示一个没用的字段。

use std::path::PathBuf;

/// 与 `xtask/src/usage.rs::CURATED` 保持一致。两处各写一份是因为 xtask 是
/// 二进制 crate，测试拿不到它的 const —— 由下面第二条测试把两份拴住。
///
/// **目前是空的。** 原先唯一的一条——`DEF_URBAN_type_scheme` 需要
/// `URBAN_MODEL`——在 LULC/BGC/CROP/URBAN/LULCC 那组改造里失效了：
/// `landurban_build` 的调用点从 `#ifdef URBAN_MODEL` 改成了运行时
/// `IF (DEF_URBAN_RUN) THEN`（mksrfdata/MKSRFDATA.F90），`URBAN_MODEL`
/// 本身也从 `include/define.h` 里彻底消失。详见 `xtask/src/usage.rs`
/// 的 `CURATED` 注释。
const CURATED: &[(&str, &str, &str, &str)] = &[];

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

#[test]
fn every_curated_gate_still_points_at_a_real_guard() {
    let root = repo().join("vendor/CoLM202X");
    for (field, macro_, file, needle) in CURATED {
        let path = root.join(file);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => return, // submodule 没取下来就跳过
        };
        let lines: Vec<&str> = text.lines().collect();
        let at = lines.iter().position(|l| l.contains(needle));
        let Some(at) = at else {
            panic!("{field}: {file} 里已经找不到 {needle:?} —— 出处失效了，重新查证");
        };
        // 那一行往上找最近的 #ifdef，必须是所声明的宏。
        let guard = lines[..at]
            .iter()
            .rev()
            .find_map(|l| {
                let t = l.trim_start();
                if t.starts_with("#endif") {
                    return Some(String::new()); // 中间有闭合块，说明没被守
                }
                t.strip_prefix("#ifdef ").map(|m| m.trim().to_string())
            })
            .unwrap_or_default();
        assert_eq!(
            guard, *macro_,
            "{field}: {file} 里 {needle:?} 上方最近的守护是 {guard:?}，\
             而人工表声明的是 {macro_:?}。上游改了条件，这条要重新查证。"
        );
    }
}

#[test]
fn the_generated_table_agrees_with_the_curated_one() {
    // 人工表在 xtask 里，生成结果在 colm-schema 里。两边对不上就说明
    // 生成器没把人工表并进去 —— 而那种失败是静默的：表还在，只是没生效。
    for (field, macro_, _, _) in CURATED {
        let f = colm_schema::find(field).unwrap_or_else(|| panic!("schema 里没有 {field}"));
        assert!(
            f.requires.contains(macro_),
            "{field} 的 requires 是 {:?}，不含人工表声明的 {macro_}",
            f.requires
        );
    }
}

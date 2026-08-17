//! 对 vendor/CoLM202X 里全部 55 个真实 .nml 做往返测试。
//!
//! 合成用例能证明语法被支持，只有真实文件能证明**用户的文件不会被改动**。
//! 55 个文件共 4167 行，最长的 354 行；覆盖 17 种 group 名，
//! 包括 CaMa-Flood 与 TRACER 那些本里程碑范围外的 —— 语法是共通的，
//! 多覆盖不花钱，而少覆盖会让"范围外"的文件在将来某天被悄悄改坏。

use std::path::{Path, PathBuf};

fn nml_files() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/CoLM202X/run")
        .canonicalize()
        .expect("vendor/CoLM202X/run must exist; run git submodule update --init");
    let mut out = Vec::new();
    collect(&root, &mut out);
    out.sort();
    assert!(
        out.len() >= 50,
        "expected ~55 namelists, found {}",
        out.len()
    );
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).expect("readable dir") {
        let p = e.expect("dir entry").path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "nml") {
            out.push(p);
        }
    }
}

#[test]
fn every_real_namelist_round_trips_byte_for_byte() {
    let mut failures = Vec::new();
    let files = nml_files();
    for f in &files {
        let src = std::fs::read_to_string(f).expect("readable file");
        match colm_namelist::parse(&src) {
            Ok(doc) => {
                let out = doc.to_string();
                if out != src {
                    let at = first_difference(&src, &out);
                    failures.push(format!("{}: differs at line {at}", f.display()));
                }
            }
            Err(e) => failures.push(format!("{}: parse failed: {e}", f.display())),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} namelists did not round-trip:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
}

#[test]
fn every_real_namelist_yields_at_least_one_field() {
    // 一个「什么都没解析出来但也没报错」的解析器会让往返测试全绿而毫无意义
    let mut empty = Vec::new();
    for f in nml_files() {
        let src = std::fs::read_to_string(&f).expect("readable file");
        let doc = colm_namelist::parse(&src).expect("parses");
        if doc.paths().is_empty() {
            empty.push(f.display().to_string());
        }
    }
    assert!(empty.is_empty(), "these parsed to zero fields:\n{empty:#?}");
}

#[test]
fn changing_one_field_changes_exactly_one_line() {
    // 拿一个真实的 forcing namelist：它同时含派生类型成员、下标赋值、
    // 空格分隔多字符串与行尾注释，是最能暴露格式丢失的样本
    let f = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/CoLM202X/run/forcing/POINT.nml");
    let src = std::fs::read_to_string(&f).expect("POINT.nml must exist");
    let mut doc = colm_namelist::parse(&src).expect("parses");
    doc.set(
        "DEF_forcing%dataset",
        colm_namelist::Value::Str("CHANGED".into()),
    )
    .expect("field exists");
    let out = doc.to_string();

    let differing: Vec<_> = src
        .lines()
        .zip(out.lines())
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i + 1)
        .collect();
    assert_eq!(
        differing.len(),
        1,
        "expected one changed line, got {differing:?}"
    );
    assert_eq!(src.lines().count(), out.lines().count());
}

fn first_difference(a: &str, b: &str) -> usize {
    for (i, (x, y)) in a.lines().zip(b.lines()).enumerate() {
        if x != y {
            return i + 1;
        }
    }
    a.lines().count().min(b.lines().count()) + 1
}

#[test]
fn every_field_in_every_file_is_findable_with_its_case_flipped() {
    // Fortran 的 namelist 变量名大小写不敏感，而上游语料自己就混用两种拼法：
    // 763 个不同的字段名里，DEF_hist_lat_res / DEF_hist_lon_res /
    // DEF_hist_vars_namelist 这三个各有大小写两种写法，且两种写法的文件
    // CoLM 都能跑。所以按大小写敏感查找的话，用户拿自己的文件进来，
    // 一半字段会被判成「不存在」—— 这条测试守住这件事。
    let mut misses = Vec::new();
    for f in nml_files() {
        let src = std::fs::read_to_string(&f).expect("readable file");
        let doc = colm_namelist::parse(&src).expect("parses");
        for p in doc.paths() {
            for probe in [p.to_uppercase(), p.to_lowercase()] {
                if doc.get(&probe).is_none() {
                    misses.push(format!("{}: {p} not found as {probe}", f.display()));
                }
            }
        }
    }
    assert!(
        misses.is_empty(),
        "{} field(s) were unreachable after changing case:\n{}",
        misses.len(),
        misses
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn setting_a_field_by_a_different_case_edits_the_line_that_is_there() {
    // 最危险的形态不是找不到，而是「找不到于是追加一行」—— 那会写出一个
    // 同名字段出现两次的文件，Fortran 取最后一个，用户看到的是第一个。
    // set 在字段不存在时报错而不追加，这条测试同时钉住这两件事：
    // 大小写不同也能命中，且命中的是原来那一行。
    let f = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/CoLM202X/run/forcing/POINT.nml");
    let src = std::fs::read_to_string(&f).expect("POINT.nml must exist");
    let mut doc = colm_namelist::parse(&src).expect("parses");
    doc.set(
        "def_FORCING%DataSet",
        colm_namelist::Value::Str("CHANGED".into()),
    )
    .expect("case-insensitive lookup must find DEF_forcing%dataset");
    let out = doc.to_string();
    assert_eq!(
        src.lines().count(),
        out.lines().count(),
        "no line was added"
    );
    let differing = src.lines().zip(out.lines()).filter(|(a, b)| a != b).count();
    assert_eq!(differing, 1, "expected exactly one edited line");
}

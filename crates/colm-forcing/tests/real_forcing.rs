//! 对全部 90 个真实强迫场文件跑一遍。
//!
//! 合成用例能证明每一步的算术，只有真实文件能证明**它对所有站点都成立**。
//! 本仓库先前两次栽在「对唯一验证过的站点成立」的常数上：土壤颜色档写死为 10
//! （90 个里只有 1 个是 10），参考高度写死为 6.0（90 个里只有 3 个是 6.0）。

use std::path::PathBuf;

use colm_forcing::{check, render, summarize, ForcingSpec};

fn forcing_dir() -> PathBuf {
    let p = PathBuf::from(
        std::env::var("PLUMBER2_ROOT")
            .unwrap_or_else(|_| "/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s".to_string()),
    )
    .join("Forcing");
    assert!(p.is_dir(), "PLUMBER2 forcing not found at {}", p.display());
    p
}

fn met_files() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(forcing_dir())
        .expect("readable")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.to_string_lossy().ends_with("_Met.nc"))
        .collect();
    out.sort();
    assert!(
        out.len() >= 85,
        "expected ~90 forcing files, found {}",
        out.len()
    );
    out
}

#[test]
fn every_forcing_file_passes_the_contract_check() {
    let mut bad = Vec::new();
    for f in met_files() {
        match summarize(&f) {
            Ok(m) => {
                let p = check(&m, None);
                if !p.is_empty() {
                    bad.push(format!("{}: {p:?}", f.display()));
                }
            }
            Err(e) => bad.push(format!("{}: {e:#}", f.display())),
        }
    }
    assert!(
        bad.is_empty(),
        "{} file(s) failed:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

#[test]
fn every_namelist_parses_back_and_names_its_own_file() {
    for f in met_files() {
        let m = summarize(&f).expect("summarizes");
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        let text = render(&ForcingSpec {
            dir: forcing_dir().display().to_string(),
            file: name.clone(),
            met: m,
        });
        let doc = colm_namelist::parse(&text)
            .unwrap_or_else(|e| panic!("{}: our own output did not parse: {e:#}", f.display()));
        let got = doc
            .get("DEF_forcing%fprefix(1)")
            .unwrap_or_else(|| panic!("{}: no fprefix(1)", f.display()))
            .to_string();
        assert_eq!(got, format!("'{name}'"), "{}", f.display());
    }
}

#[test]
fn the_time_steps_are_the_two_measured_values() {
    // 实测 88 个站点 1800 s、2 个 3600 s。这条不是要求它们都一样 ——
    // 而是钉住「有两种」这件事，因为算例里的 timestep 必须跟着走。
    let mut counts = std::collections::BTreeMap::new();
    for f in met_files() {
        let m = summarize(&f).expect("summarizes");
        *counts.entry(m.timestep_hint()).or_insert(0usize) += 1;
    }
    println!("time steps across sites: {counts:?}");
    assert_eq!(counts.get(&1800), Some(&88), "{counts:?}");
    assert_eq!(counts.get(&3600), Some(&2), "{counts:?}");
}

#[test]
fn the_three_heights_differ_at_about_a_third_of_sites() {
    // 实测 30/90。这条钉住的是「必须分别读三个」这件事：若某天变成 0，
    // 说明读法退化成了一个值，而那会在三分之一的站点上出错。
    let mut differ = 0usize;
    for f in met_files() {
        let m = summarize(&f).expect("summarizes");
        if m.height_v != m.height_t || m.height_t != m.height_q {
            differ += 1;
        }
    }
    println!("sites where the three reference heights differ: {differ}");
    assert!((20..=40).contains(&differ), "measured 30, got {differ}");
}

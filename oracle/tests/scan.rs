//! `colm-cli scan` 的验收。
//!
//! 要真数据集才有意义 —— 这条验的正是「两套命名约定都认得出」，
//! 而那只能拿真文件名验。没有数据就跳过，与本仓库其余需要 PLUMBER2 的测试一致。

use std::path::PathBuf;
use std::process::Command;

fn cli() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    // 测试可能在 debug 或 release 下跑，两处都找。
    for rel in ["target/debug/colm-cli", "target/release/colm-cli"] {
        let c = p.join(rel);
        if c.is_file() {
            return c;
        }
    }
    panic!("colm-cli 没构建：先 cargo build -p colm-cli");
}

fn scan(dir: &str) -> Option<serde_json::Value> {
    if !PathBuf::from(dir).is_dir() {
        return None;
    }
    let out = Command::new(cli())
        .args(["scan", "--dir", dir])
        .output()
        .expect("runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(serde_json::from_slice(&out.stdout).expect("输出必须是合法 JSON"))
}

#[test]
fn it_finds_every_plumber2_site_with_its_forcing_and_observation() {
    let root = match std::env::var("PLUMBER2_ROOT") {
        Ok(r) => r,
        Err(_) => return, // 没数据就跳过
    };
    let Some(v) = scan(&format!("{root}/Sitedata")) else {
        return;
    };
    let a = v.as_array().expect("数组");
    assert_eq!(a.len(), 90, "PLUMBER2 是 90 个站点");
    // 每个站都该配到强迫场与观测 —— 配不到说明 LAYOUTS 那张表漏了一种约定，
    // 而那会让界面把一个好站点报成「不能跑」。
    for s in a {
        assert!(s["met_file"].is_string(), "{} 没配到强迫场", s["name"]);
        assert!(s["obs_file"].is_string(), "{} 没配到观测", s["name"]);
        assert!(
            s["problem"].is_null(),
            "{} 读出了问题：{}",
            s["name"],
            s["problem"]
        );
        // PLUMBER2 站点文件都带 IGBP_classification，所以一个都不该判成城市。
        assert_eq!(
            s["urban"],
            serde_json::json!(false),
            "{} 被误判成城市",
            s["name"]
        );
        assert!(s["landtype"].is_i64());
    }
}

#[test]
fn it_recognises_the_urban_naming_convention_too() {
    // 三个后缀全不一样。只认 PLUMBER2 那套的话，21 个城市站点会被
    // 整体报成「没有强迫场」—— 而文件就在旁边。
    let dir = std::env::var("URBAN_PLUMBER_ROOT")
        .map(|r| format!("{r}/Sitedata"))
        .unwrap_or_else(|_| {
            format!(
                "{}/Desktop/colm-rust/Urban-PLUMBER/Sitedata",
                std::env::var("HOME").unwrap_or_default()
            )
        });
    let Some(v) = scan(&dir) else { return };
    let a = v.as_array().expect("数组");
    assert!(!a.is_empty());
    for s in a {
        assert!(s["met_file"].is_string(), "{} 没配到强迫场", s["name"]);
        // 城市站点文件不带 IGBP_classification —— 那正是判据。
        assert_eq!(
            s["urban"],
            serde_json::json!(true),
            "{} 没被认成城市",
            s["name"]
        );
        assert!(s["landtype"].is_null());
    }
}

#[test]
fn a_relative_case_directory_works_the_same_as_an_absolute_one() {
    // `run_stage` 用 `current_dir(work)` 起子进程，所以一个相对的 namelist
    // 路径会被相对 `work` 再解析一次 —— CoLM 去找
    // `<case>/<case>/case.nml` 然后 `Cannot open file`。
    // `Kernel::open` 早就为可执行文件绝对化过，算例目录当时漏了。
    //
    // 这里不真跑模型（那要内核与数据），只验「路径被解析成了同一个地方」：
    // 给一个不存在的内核，两种写法都该在**内核**上失败，而不是一个在
    // namelist 上失败、另一个在内核上失败。
    let repo = {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.pop();
        p
    };
    let case = repo.join("oracle/work/CN-Cng");
    if !case.join("case.nml").is_file() {
        return; // 没跑过黄金算例就跳过
    }
    let run = |dir: &str| {
        let out = Command::new(cli())
            .args(["run", dir, "--kernel", "no-such-kernel"])
            .current_dir(&repo)
            .output()
            .expect("runs");
        String::from_utf8_lossy(&out.stderr).into_owned()
    };
    let abs = run(case.to_str().unwrap());
    let rel = run("oracle/work/CN-Cng");
    for (which, msg) in [("绝对", &abs), ("相对", &rel)] {
        assert!(
            msg.contains("no-such-kernel"),
            "{which}路径的失败该出在内核上，实际是：{msg}"
        );
    }
}

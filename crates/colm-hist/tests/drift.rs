//! 入库的 generated.rs 必须与现在重新生成的结果一致。
//!
//! 没有这条，上游改了 MOD_Hist.F90 之后闸门表会静默过时：
//! 编译照过、测试照绿，只有 GUI 少报一个变量、或把一个已经被
//! `#ifdef` 拿掉的变量继续报成「这个内核能产出」。

use std::path::PathBuf;
use std::process::Command;

#[test]
fn regenerating_the_histmap_produces_the_committed_file() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root");
    let committed = root.join("crates/colm-hist/src/generated.rs");
    let before = std::fs::read_to_string(&committed).expect("generated.rs must exist");

    let out = Command::new(env!("CARGO"))
        .args(["run", "-q", "-p", "xtask", "--", "gen-histmap"])
        .current_dir(&root)
        .output()
        .expect("run xtask");
    assert!(
        out.status.success(),
        "gen-histmap failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let after = std::fs::read_to_string(&committed).expect("still readable");
    if before != after {
        // 还原，免得一次失败的测试把工作树弄脏
        std::fs::write(&committed, &before).expect("restore");
        panic!(
            "generated.rs is out of date with MOD_Hist.F90.\n\
             Run: cargo run -p xtask -- gen-histmap, then commit the result."
        );
    }
}

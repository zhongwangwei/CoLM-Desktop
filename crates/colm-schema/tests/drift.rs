//! 入库的 generated.rs 必须与现在重新生成的结果一致。
//!
//! 没有这条，上游改了 MOD_Namelist.F90 之后 schema 会静默过时：
//! 编译照过、测试照绿，只有 GUI 少显示一个选项、或显示一个错误的默认值。

use std::path::PathBuf;
use std::process::Command;

#[test]
fn regenerating_the_schema_produces_the_committed_file() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root");
    let committed = root.join("crates/colm-schema/src/generated.rs");
    let before = std::fs::read_to_string(&committed).expect("generated.rs must exist");

    let out = Command::new(env!("CARGO"))
        .args(["run", "-q", "-p", "xtask", "--", "gen-schema"])
        .current_dir(&root)
        .output()
        .expect("run xtask");
    assert!(
        out.status.success(),
        "gen-schema failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let after = std::fs::read_to_string(&committed).expect("still readable");
    if before != after {
        // 还原，免得一次失败的测试把工作树弄脏
        std::fs::write(&committed, &before).expect("restore");
        panic!(
            "generated.rs is out of date with MOD_Namelist.F90.\n\
             Run: cargo run -p xtask -- gen-schema, then commit the result."
        );
    }
}

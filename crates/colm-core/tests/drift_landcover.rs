//! 入库的 `land_cover_generated.rs` 必须与现在重新生成的结果一致。
//!
//! 没有这条，上游改了 `MOD_Const_LC.F90` 之后地类常量表会静默过时：编译照过、
//! 测试照绿，而某个地类的参数还是上一版的 —— 这类错误在数值上几乎看不出来。

use std::path::PathBuf;
use std::process::Command;

#[test]
fn regenerating_the_land_cover_tables_produces_the_committed_file() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root");
    let committed = root.join("crates/colm-core/src/land_cover_generated.rs");
    let before = std::fs::read_to_string(&committed).expect("land_cover_generated.rs must exist");

    let out = Command::new(env!("CARGO"))
        .args(["run", "-q", "-p", "xtask", "--", "gen-landcover"])
        .current_dir(&root)
        .output()
        .expect("run xtask");
    assert!(
        out.status.success(),
        "gen-landcover failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let after = std::fs::read_to_string(&committed).expect("still readable");
    if before != after {
        // 还原，免得一次失败的测试把工作树弄脏
        std::fs::write(&committed, &before).expect("restore");
        panic!(
            "land_cover_generated.rs is out of date with MOD_Const_LC.F90.\n\
             Run: cargo run -p xtask -- gen-landcover, then commit the result."
        );
    }
}

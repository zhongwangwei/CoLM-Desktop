//! vendor Makefile 末尾的同阶段模块依赖块必须与源码的 MODULE/USE 一致。
//!
//! 上游 Makefile 只靠对象在阶段变量里的排列顺序保证模块先编译：串行 make 能过，`make -j`
//! 会在 `.mod` 生成之前就去读它。依赖块由 `oracle/scripts/gen_fortran_deps.py` 生成，
//! 同步上游之后必须重跑；这里用它的 `--check` 守住漂移。

use std::path::PathBuf;
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("repo root")
}

#[test]
fn generated_fortran_module_prerequisites_are_current() {
    let script = root().join("oracle/scripts/gen_fortran_deps.py");
    let python = if cfg!(windows) { "python" } else { "python3" };
    let output = Command::new(python)
        .arg(&script)
        .arg("--check")
        .current_dir(root())
        .output()
        .unwrap_or_else(|error| panic!("cannot run {python} {}: {error}", script.display()));
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

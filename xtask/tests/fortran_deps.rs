//! vendor Makefile 末尾的同阶段模块依赖块必须与源码的 MODULE/USE 一致。
//!
//! 上游 Makefile 只靠对象在阶段变量里的排列顺序保证模块先编译：串行 make 能过，`make -j`
//! 会在 `.mod` 生成之前就去读它。依赖块由 `oracle/scripts/gen_fortran_deps.py` 生成，
//! 同步上游之后必须重跑；这里用它的 `--check` 守住漂移。

use std::path::PathBuf;
use std::process::Command;

fn root() -> PathBuf {
    // Keep foreign POSIX tools on regular paths, without canonicalize()'s \\?\ prefix.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf()
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

#[test]
fn configured_fortran_dependency_tools_must_be_complete() {
    let temp = std::env::temp_dir().join(format!(
        "colm-fortran-deps-tools-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&temp).unwrap();
    let python = if cfg!(windows) { "python" } else { "python3" };
    let output = Command::new(python)
        .arg(root().join("oracle/scripts/gen_fortran_deps.py"))
        .arg("--check")
        .env("COLM_FORTRAN_DEPS_BIN", &temp)
        .output()
        .expect("run dependency generator with incomplete tools");
    std::fs::remove_dir(&temp).unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("COLM_FORTRAN_DEPS_BIN is missing required tools"),
        "{error}"
    );
}

#[test]
fn stale_fortran_dependencies_report_the_changed_prerequisites() {
    let python = if cfg!(windows) { "python" } else { "python3" };
    let output = Command::new(python)
        .arg("-c")
        .arg(
            r#"
import importlib.util, pathlib, sys, tempfile
spec = importlib.util.spec_from_file_location("deps", sys.argv[1])
deps = importlib.util.module_from_spec(spec)
spec.loader.exec_module(deps)
text = pathlib.Path(deps.MAKEFILE).read_text(encoding="utf-8")
line = next(line for line in text.split(deps.BEGIN, 1)[1].splitlines() if ".o: " in line)
with tempfile.TemporaryDirectory(prefix="colm-stale-deps-") as directory:
    makefile = pathlib.Path(directory, "Makefile")
    makefile.write_text(text.replace(line, line + " stale_test.o", 1), encoding="utf-8")
    deps.MAKEFILE = str(makefile)
    sys.argv = [sys.argv[1], "--check"]
    sys.exit(deps.main())
"#,
        )
        .arg(root().join("oracle/scripts/gen_fortran_deps.py"))
        .current_dir(root())
        .output()
        .expect("check a private stale Makefile");
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("--- committed Makefile"), "{error}");
    assert!(error.contains("+++ generated Makefile"), "{error}");
    assert!(
        error
            .lines()
            .any(|line| line.starts_with('-') && line.contains("stale_test.o")),
        "{error}"
    );
}

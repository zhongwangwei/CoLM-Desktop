fn declared_msrv(manifest: &str) -> &str {
    manifest
        .lines()
        .find_map(|line| line.strip_prefix("rust-version = \"")?.strip_suffix('"'))
        .expect("the package must declare its minimum Rust version")
}

#[test]
fn minimum_compilers_check_both_locked_workspaces() {
    let workflow = include_str!("../../.github/workflows/ci.yml");
    let engine = workflow.split_once("\n  msrv-engine:\n").unwrap().1;
    let engine = engine.split_once("\n  msrv-gui:\n").unwrap().0;
    let gui = workflow.split_once("\n  msrv-gui:\n").unwrap().1;
    let gui = gui.split_once("\n  rust:\n").unwrap().0;
    let engine_msrv = declared_msrv(include_str!("../../Cargo.toml"));
    let gui_msrv = declared_msrv(include_str!("../../gui/src-tauri/Cargo.toml"));
    assert!(engine.contains(&format!("toolchain: '{engine_msrv}'")));
    assert!(gui.contains(&format!("toolchain: '{gui_msrv}.0'")));
    assert!(engine.contains("cargo check --locked --workspace --all-targets"));
    assert!(
        gui.contains("cargo check --locked --manifest-path gui/src-tauri/Cargo.toml --all-targets")
    );
    assert!(gui.contains("libwebkit2gtk-4.1-dev"));
    assert!(workflow.contains("dtolnay/rust-toolchain@stable"));
}

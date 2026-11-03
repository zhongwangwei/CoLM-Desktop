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
use sha2::{Digest, Sha256};

#[test]
fn lopdf_backport_changes_only_the_incompatible_let_chain() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../vendor/lopdf");
    let upstream = r#"                            if let Ok(base_encoding) = dict.get(b"BaseEncoding")
                                && let Ok(name) = base_encoding.as_name()
                            {
                                base = Some(self.base_encoding(doc, name)?);
                            }"#;
    let patched = r#"                            if let Ok(base_encoding) = dict.get(b"BaseEncoding") {
                                if let Ok(name) = base_encoding.as_name() {
                                    base = Some(self.base_encoding(doc, name)?);
                                }
                            }"#;
    let manifest = std::fs::read_to_string(root.join("UPSTREAM_FILES.sha256")).unwrap();
    let mut checked = 0;
    for line in manifest.lines() {
        let (expected, path) = line.split_once("  ").unwrap();
        let mut content = std::fs::read(root.join(path)).unwrap();
        if path == "src/object.rs" {
            let text = String::from_utf8(content).unwrap();
            assert_eq!(text.matches(patched).count(), 1);
            assert!(!text.contains(upstream));
            content = text.replace(patched, upstream).into_bytes();
        }
        assert_eq!(format!("{:x}", Sha256::digest(content)), expected, "{path}");
        checked += 1;
    }
    assert_eq!(checked, 89);
    let workspace = include_str!("../../Cargo.toml");
    assert!(workspace.contains("exclude = [\"vendor/lopdf\"]"));
    assert!(workspace.contains("lopdf = { path = \"vendor/lopdf\" }"));
}

use super::*;
use crate::layout::layout_tests::{source_repo, temp};

#[test]
fn kinds_are_parsed_and_labelled() {
    assert_eq!(
        Kind::parse("cargo", Some("colm-core")).unwrap().label(),
        "cargo:colm-core"
    );
    assert_eq!(Kind::parse("oracle", None).unwrap().label(), "oracle");
    assert_eq!(Kind::parse("check-gui", None).unwrap().label(), "check-gui");
    assert_eq!(Kind::parse("drift", None).unwrap().label(), "drift");
    assert!(Kind::parse("cargo", None).is_err());
    assert!(Kind::parse("cargo", Some("")).is_err());
    assert!(Kind::parse("shell", Some("ls")).is_err());
}

#[test]
fn only_real_crates_with_clean_names_can_be_tested() {
    let root = temp("tests");
    let repo = root.join("source");
    source_repo(&repo);
    // 工作区里 colm-core 有 Cargo.toml 才算真 crate。
    std::fs::write(
        repo.join("crates/colm-core/Cargo.toml"),
        "[package]\nname = \"colm-core\"\n",
    )
    .unwrap();
    crate::git::run(&repo, &["add", "-A"]).unwrap();
    crate::git::run(&repo, &["commit", "-q", "-m", "manifest"]).unwrap();
    let ws = Workspace::create(&root.join("ws"), "demo", repo.to_str().unwrap(), None).unwrap();
    let commands = commands(&ws, &Kind::Cargo("colm-core".into())).unwrap();
    assert_eq!(commands.len(), 1);
    let line = commands[0].join(" ");
    assert!(
        line.starts_with("test -p colm-core --lib --bins --offline --locked"),
        "{line}"
    );
    assert!(line.ends_with("-- --test-threads=1"));
    for bad in ["../colm-core", "colm-core; ls", "nonexistent", "", "-p"] {
        assert!(commands_for(&ws, bad).is_err(), "{bad:?}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

fn commands_for(ws: &Workspace, package: &str) -> Result<Vec<Vec<String>>> {
    commands(ws, &Kind::Cargo(package.into()))
}

/// 漂移检查：自动找出 `crates/*/tests/drift*.rs`，每个一条 `cargo test -p <包> --test <名字>`；别的集成测试不算。
#[test]
fn drift_runs_every_drift_integration_test_and_nothing_else() {
    let root = temp("drift");
    let repo = root.join("source");
    source_repo(&repo);
    for (package, test) in [
        ("colm-core", "drift_landcover"),
        ("colm-core", "drift_co2"),
        ("colm-hist", "drift"),
    ] {
        let dir = repo.join("crates").join(package).join("tests");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{test}.rs")), "#[test]\nfn same() {}\n").unwrap();
    }
    std::fs::write(repo.join("crates/colm-core/tests/golden.rs"), "").unwrap();
    crate::git::run(&repo, &["add", "-A"]).unwrap();
    crate::git::run(&repo, &["commit", "-q", "-m", "drift tests"]).unwrap();
    let ws = Workspace::create(&root.join("ws"), "demo", repo.to_str().unwrap(), None).unwrap();
    let lines: Vec<String> = commands(&ws, &Kind::Drift)
        .unwrap()
        .iter()
        .map(|c| c.join(" "))
        .collect();
    assert_eq!(
        lines,
        [
            "test -p colm-core --test drift_co2 --offline --locked",
            "test -p colm-core --test drift_landcover --offline --locked",
            "test -p colm-hist --test drift --offline --locked",
        ]
    );
    let _ = std::fs::remove_dir_all(&root);
}

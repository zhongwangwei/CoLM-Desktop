use super::*;

fn case_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("colm-provenance-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("case.nml"),
        "&nl_colm\n  DEF_CASE_NAME = 'demo'\n/\n",
    )
    .unwrap();
    dir
}

fn record<'a>(case: &'a Path, kernel: &'a Path, version: &'a Version) -> Record<'a> {
    Record {
        case,
        kernel,
        engine: "rust",
        preprocessors: "rust",
        ranks: 1,
        stage: None,
        force: false,
        location: "local",
        version,
        started: 10,
        execution_program_sha256: None,
    }
}

#[test]
fn an_official_run_records_its_version_engine_and_kernel() {
    let case = case_dir("official");
    let kernel = case.join("kernel");
    std::fs::create_dir_all(&kernel).unwrap();
    std::fs::write(
        kernel.join("manifest.json"),
        r#"{"schema":1,"preset":"default","platform":"Darwin-arm64","colm_git_sha":"f59ffdce","generator_args":"SinglePoint","macros":["SinglePoint"],"built_with":"gfortran","netcdf_c":"","netcdf_fortran":"","hdf5":"","sha256":{}}"#,
    )
    .unwrap();
    assert!(read(&case).unwrap().is_none(), "nothing ran yet");

    let version = Version::Official;
    write(&record(&case, &kernel, &version), true, None);
    let value = read(&case).unwrap().expect("written");
    assert_eq!(value["schema"], 1);
    assert_eq!(value["ok"], true);
    assert_eq!(value["version"]["kind"], "official");
    assert_eq!(value["engine"], "rust");
    assert_eq!(value["location"]["kind"], "local");
    assert_eq!(value["kernel"]["preset"], "default");
    assert_eq!(value["kernel"]["colm_git_sha"], "f59ffdce");
    assert!(case.join("out/demo").join(FILE).is_file());
    assert!(!case.join("out/demo").join(format!("{FILE}.tmp")).exists());
    // 每次运行都追加进运行历史（结果页列出这个算例的全部运行）。
    write(&record(&case, &kernel, &version), false, Some("again"));
    let history = std::fs::read_to_string(case.join(crate::case_runs::HISTORY)).unwrap();
    assert_eq!(history.lines().count(), 2);
}

#[test]
fn a_failed_run_keeps_its_reason_and_a_case_without_a_name_writes_nothing() {
    let case = case_dir("failed");
    let kernel = case.join("missing-kernel");
    let version = Version::Official;
    write(
        &record(&case, &kernel, &version),
        false,
        Some("stage colm failed"),
    );
    let value = read(&case).unwrap().unwrap();
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"], "stage colm failed");
    // 读不出内核清单时只记目录。
    assert!(value["kernel"]["identity"].is_null());

    let nameless =
        std::env::temp_dir().join(format!("colm-provenance-nameless-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&nameless);
    std::fs::create_dir_all(&nameless).unwrap();
    write(&record(&nameless, &kernel, &version), false, None);
    assert!(!nameless.join("out").exists());
}

/// 交给修改版自己的 colm-cli：同一条 `run`、换上它的内核、并告诉它出处由外层写。
#[cfg(unix)]
#[test]
fn a_version_run_hands_the_same_run_to_the_versions_own_cli() {
    use std::os::unix::fs::PermissionsExt;
    let dir = case_dir("delegate");
    let log = dir.join("args.txt");
    let cli = dir.join("colm-cli");
    std::fs::write(
        &cli,
        format!(
            "#!/bin/sh\necho \"$@ by-parent=$COLM_PROVENANCE_BY_PARENT\" > '{}'\nexit 3\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    let readiness = Readiness {
        name: "snow".into(),
        head: "abc".into(),
        base_commit: "def".into(),
        origin: "app".into(),
        note: String::new(),
        based_on: None,
        touches: colm_workspace::version::Touches {
            engine: true,
            kernel: false,
        },
        ready: true,
        items: Vec::new(),
        cli,
        kernel: PathBuf::from("/k/default"),
        preset: "default".into(),
    };
    let args: Vec<String> = ["/case", "--stream", "1", "--engine", "rust"]
        .map(String::from)
        .to_vec();
    assert_eq!(delegate(&readiness, &args).unwrap(), 3);
    assert_eq!(
        std::fs::read_to_string(&log).unwrap().trim(),
        "run /case --stream 1 --engine rust --kernel /k/default by-parent=1"
    );
}

#[test]
fn output_content_identity_changes_even_when_filename_and_length_do_not() {
    let root = case_dir("output-content");
    std::fs::create_dir(root.join("history")).unwrap();
    std::fs::write(root.join("history/result.nc"), b"first").unwrap();
    let before = output_identities(&root).unwrap();
    std::fs::write(root.join("history/result.nc"), b"other").unwrap();
    let after = output_identities(&root).unwrap();
    assert_ne!(before["history/result.nc"], after["history/result.nc"]);
    assert_eq!(
        before.len(),
        1,
        "configuration files are not scientific outputs"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn frozen_child_program_identity_is_not_the_current_package_identity() {
    let case = case_dir("frozen-programs");
    let current = case.join("current-bin");
    let frozen = case.join("frozen-bin");
    let program = |name: &str| {
        if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.into()
        }
    };
    for (dir, bytes) in [
        (&current, b"current".as_slice()),
        (&frozen, b"frozen".as_slice()),
    ] {
        std::fs::create_dir(dir).unwrap();
        std::fs::write(dir.join(program("colm-cli")), bytes).unwrap();
        std::fs::write(dir.join(program("colm-rs")), bytes).unwrap();
    }
    let hashes = program_identities(&frozen.join(program("colm-cli"))).unwrap();
    assert_ne!(
        hashes,
        program_identities(&current.join(program("colm-cli"))).unwrap()
    );
    let version = Version::Official;
    let mut record = record(&case, &current, &version);
    record.execution_program_sha256 = Some(&hashes);
    let value = value(&record, true, None);
    assert_eq!(
        value["execution_program_sha256"]["colm-rs"],
        hashes["colm-rs"]
    );
    std::fs::write(frozen.join(program("colm-rs")), b"mutated").unwrap();
    assert_ne!(
        hashes,
        program_identities(&frozen.join(program("colm-cli"))).unwrap()
    );
    std::fs::remove_dir_all(case).unwrap();
}

#![allow(clippy::field_reassign_with_default)]
use super::*;
use crate::compare::{Options, Status};
use crate::layout::layout_tests::{nc_lock, source_repo, temp};

fn history(path: &Path, vars: &[(&str, Vec<f64>)]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = netcdf::create(path).unwrap();
    file.add_unlimited_dimension("time").unwrap();
    file.add_dimension("patch", 3).unwrap();
    {
        let mut time = file.add_variable::<f64>("time", &["time"]).unwrap();
        time.put_values(&[0.0, 30.0, 60.0, 90.0], netcdf::Extents::All)
            .unwrap();
    }
    for (name, data) in vars {
        let mut v = file.add_variable::<f64>(name, &["time", "patch"]).unwrap();
        v.put_values(data, netcdf::Extents::All).unwrap();
    }
}

fn series(f: impl Fn(usize) -> f64) -> Vec<f64> {
    (0..12).map(f).collect()
}

/// 一个参考算例：只有 case.nml（输出目录指向自己）和一个输入文件。
fn reference_case(dir: &Path) -> PathBuf {
    let case = dir.join("refcase");
    std::fs::create_dir_all(case.join("out")).unwrap();
    std::fs::write(case.join("input.txt"), "data").unwrap();
    std::fs::write(case.join("stages.json"), "{}").unwrap();
    std::fs::write(case.join("out/old.txt"), "stale").unwrap();
    std::fs::write(
        case.join("case.nml"),
        format!(
            "&nl_colm\n DEF_CASE_NAME = 'REF'\n DEF_dir_output = '{}/out/'\n DEF_file_site = '{}/input.txt'\n/\n",
            case.display(),
            case.display()
        ),
    )
    .unwrap();
    case
}

/// 假的 `colm-cli`：收到 `run <算例> --kernel K --engine E …` 就把夹具里 E 对应的 history 拷到算例的输出里。
fn fake_cli(path: &Path, fixture_rust: &Path, fixture_fortran: &Path, exit: i32) {
    let script = format!(
        "#!/bin/sh\ncase=\"$2\"\nengine=\"$6\"\n[ \"$1\" = run ] || exit 9\nif [ \"$engine\" = fortran ]; then src={f}; else src={r}; fi\nmkdir -p \"$case/out/REF/history\"\ncp \"$src\"/*.nc \"$case/out/REF/history/\"\nexit {exit}\n",
        f = fixture_fortran.display(),
        r = fixture_rust.display(),
    );
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn workspace(root: &Path) -> Workspace {
    let repo = root.join("source");
    source_repo(&repo);
    Workspace::create(&root.join("ws"), "demo", repo.to_str().unwrap(), None).unwrap()
}

/// 记一次“在当前提交上编译通过”（假的 colm-cli 与内核没有真编过）。两版一致与回归要求有它。
fn mark_built(ws: &mut Workspace, preset: &str) {
    let commit = ws.head().unwrap();
    let run = crate::gates::GateRun {
        ok: true,
        at: 0,
        commit,
        detail: "test".into(),
    };
    ws.update(|info| {
        info.gates.engine = Some(run.clone());
        info.gates.kernels.insert(preset.to_owned(), run.clone());
    })
    .unwrap();
}

fn fake_kernel(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("manifest.json"), "{}").unwrap();
}

#[test]
fn a_case_copy_writes_its_output_into_itself() {
    let root = temp("copy");
    let case = reference_case(&root);
    let copy = root.join("copy");
    copy_case(&case, &copy).unwrap();
    assert!(copy.join("input.txt").is_file());
    assert!(
        !copy.join("out").exists() && !copy.join("stages.json").exists(),
        "outputs and fingerprints are not copied"
    );
    let nml = std::fs::read_to_string(copy.join("case.nml")).unwrap();
    let new = copy.canonicalize().unwrap().to_string_lossy().into_owned();
    assert!(
        nml.contains(&format!("{new}/out/")) && nml.contains(&format!("{new}/input.txt")),
        "{nml}"
    );
    assert!(!nml.contains("refcase"));
    assert_eq!(case_name(&copy).unwrap(), "REF");
    assert!(copy_case(&root.join("nothing"), &root.join("x")).is_err());
    assert!(case_name(&root)
        .unwrap_err()
        .to_string()
        .contains("case.nml"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn parity_is_bitwise_when_both_engines_agree_and_names_the_first_difference_when_not() {
    let _nc = nc_lock();
    let root = temp("parity");
    let case = reference_case(&root);
    let mut ws = workspace(&root);
    fake_kernel(&ws.kernels().join("default"));
    let (a, b) = (root.join("fx-a"), root.join("fx-b"));
    let good = series(|i| 1.0 + i as f64);
    history(
        &a.join("REF_hist_2004-01.nc"),
        &[("f_t", good.clone()), ("f_q", good.clone())],
    );
    history(
        &b.join("REF_hist_2004-01.nc"),
        &[("f_t", good.clone()), ("f_q", good.clone())],
    );
    fake_cli(&ws.bin().join("colm-cli"), &a, &b, 0);
    mark_built(&mut ws, "default");
    let ok = parity_check(&mut ws, &case, "default", &Options::default(), None).unwrap();
    assert!(ok.ok, "{:?}", ok.first_difference);
    assert!(ok.compare.as_ref().unwrap().bitwise_identical());
    assert!(ws.info.gates.parity.as_ref().unwrap().ok);
    assert!(ok.report.is_file());
    // 两边的算例副本互相独立，输出各写各的。
    assert!(
        ok.rust.history.is_dir()
            && ok.fortran.history.is_dir()
            && ok.rust.history != ok.fortran.history
    );

    // Fortran 那边在第 2 步（元素 7）之后偏了：报出变量与时间步。
    let mut off = good.clone();
    off[7] += 1e-12;
    history(
        &b.join("REF_hist_2004-01.nc"),
        &[("f_t", good.clone()), ("f_q", off)],
    );
    let bad = parity_check(&mut ws, &case, "default", &Options::default(), None).unwrap();
    assert!(!bad.ok);
    let first = bad
        .compare
        .as_ref()
        .unwrap()
        .first_difference
        .clone()
        .unwrap();
    assert_eq!((first.variable.as_str(), first.step), ("f_q", Some(2)));
    assert_eq!(first.time, Some(60.0));
    assert!(bad.first_difference.unwrap().contains("f_q @ step 2"));
    assert!(!ws.info.gates.parity.as_ref().unwrap().ok);

    // 一边跑崩了：不比较，如实说。
    fake_cli(&ws.bin().join("colm-cli"), &a, &b, 1);
    let crashed = parity_check(&mut ws, &case, "default", &Options::default(), None).unwrap();
    assert!(!crashed.ok && crashed.compare.is_none());
    assert!(crashed.first_difference.unwrap().contains("a run failed"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_refactor_must_be_bitwise_but_a_physics_change_only_has_to_stay_sane() {
    let _nc = nc_lock();
    let root = temp("regress");
    let case = reference_case(&root);
    let mut ws = workspace(&root);
    let kernel = root.join("kernel");
    fake_kernel(&kernel);
    let (base, cand) = (root.join("fx-base"), root.join("fx-cand"));
    let t = series(|i| 280.0 + i as f64);
    let closure = series(|_| 1e-12);
    history(
        &base.join("REF_hist_2004-01.nc"),
        &[
            ("f_t", t.clone()),
            ("f_xerr", closure.clone()),
            ("f_zerr", closure.clone()),
        ],
    );
    let baseline_cli = root.join("baseline/colm-cli");
    fake_cli(&baseline_cli, &base, &base, 0);
    let baseline = Baseline {
        cli: baseline_cli,
        kernel,
    };

    // 候选和基线一样：重构通过。
    history(
        &cand.join("REF_hist_2004-01.nc"),
        &[
            ("f_t", t.clone()),
            ("f_xerr", closure.clone()),
            ("f_zerr", closure.clone()),
        ],
    );
    fake_cli(&ws.bin().join("colm-cli"), &cand, &cand, 0);
    mark_built(&mut ws, "default");
    let same = regress(
        &mut ws,
        &case,
        "default",
        "rust",
        &baseline,
        ChangeKind::Refactor,
        None,
    )
    .unwrap();
    assert!(same.ok, "{}", same.verdict);
    assert!(same.verdict.starts_with("bitwise identical"));

    // 温度变了：按重构不通过，按物理修改通过并列出变了什么。
    let warmer = series(|i| 280.5 + i as f64);
    history(
        &cand.join("REF_hist_2004-01.nc"),
        &[
            ("f_t", warmer),
            ("f_xerr", closure.clone()),
            ("f_zerr", closure.clone()),
        ],
    );
    let refactor = regress(
        &mut ws,
        &case,
        "default",
        "rust",
        &baseline,
        ChangeKind::Refactor,
        None,
    )
    .unwrap();
    assert!(
        !refactor.ok && refactor.verdict.contains("must be bitwise identical"),
        "{}",
        refactor.verdict
    );
    assert!(refactor.verdict.contains("f_t @ step 0"));
    let physics = regress(
        &mut ws,
        &case,
        "default",
        "rust",
        &baseline,
        ChangeKind::Physics,
        None,
    )
    .unwrap();
    assert!(
        physics.ok && physics.verdict.contains("f_t"),
        "{}",
        physics.verdict
    );
    let record = ws.info.gates.regression.clone().unwrap();
    assert_eq!(
        (record.kind, record.ok, record.changed, record.identical),
        (ChangeKind::Physics, true, 1, 3)
    );

    // 闭合变差（水量不平衡从 1e-12 涨到 1e-3）：物理修改也不通过。
    let leaky = series(|_| 1e-3);
    history(
        &cand.join("REF_hist_2004-01.nc"),
        &[
            ("f_t", series(|i| 280.5 + i as f64)),
            ("f_xerr", leaky),
            ("f_zerr", closure.clone()),
        ],
    );
    let worse = regress(
        &mut ws,
        &case,
        "default",
        "rust",
        &baseline,
        ChangeKind::Physics,
        None,
    )
    .unwrap();
    assert!(
        !worse.ok && worse.verdict.contains("water balance"),
        "{}",
        worse.verdict
    );
    assert!(worse
        .closure
        .iter()
        .any(|c| c.variable == "f_xerr" && c.meaning == "water balance error [mm/s]"));
    assert_eq!(closure_meaning("f_zerr"), "energy balance error [W/m2]");
    assert!(!worse.closure.iter().all(|c| c.ok));

    // 出现 NaN：直接不通过。
    let mut nan = series(|i| 280.0 + i as f64);
    nan[3] = f64::NAN;
    history(
        &cand.join("REF_hist_2004-01.nc"),
        &[
            ("f_t", nan),
            ("f_xerr", closure.clone()),
            ("f_zerr", closure.clone()),
        ],
    );
    let broken = regress(
        &mut ws,
        &case,
        "default",
        "rust",
        &baseline,
        ChangeKind::Physics,
        None,
    )
    .unwrap();
    assert!(
        !broken.ok && broken.verdict.contains("NaN"),
        "{}",
        broken.verdict
    );

    // Identical output without closure diagnostics is incomplete even for physics changes.
    for dir in [&base, &cand] {
        history(
            &dir.join("REF_hist_2004-01.nc"),
            &[("f_t", series(|_| 280.0))],
        );
    }
    let missing = regress(
        &mut ws,
        &case,
        "default",
        "rust",
        &baseline,
        ChangeKind::Physics,
        None,
    )
    .unwrap();
    assert!(!missing.ok && missing.verdict.contains("missing"));
    assert!(!ws.info.gates.regression.as_ref().unwrap().ok);

    // Fortran 引擎的回归必须有工作区自己编的内核。
    assert!(regress(
        &mut ws,
        &case,
        "default",
        "fortran",
        &baseline,
        ChangeKind::Physics,
        None
    )
    .is_err());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_closure_limit_is_ten_times_the_baseline_but_never_below_a_floor() {
    assert_eq!(closure_limit(1e-12), 1e-6);
    assert_eq!(closure_limit(5e-6), 5e-5);
    let _ = Status::Identical;
}

/// 带容差：舍入噪声通过，真正的错位不通过；报告里带上容差和平台提示。
#[test]
fn parity_with_a_tolerance_accepts_rounding_noise_and_still_catches_a_real_misalignment() {
    let _nc = nc_lock();
    let root = temp("paritytol");
    let case = reference_case(&root);
    let mut ws = workspace(&root);
    fake_kernel(&ws.kernels().join("default"));
    let (a, b) = (root.join("fx-a"), root.join("fx-b"));
    let good = series(|i| 280.0 + i as f64);
    let noise: Vec<f64> = good.iter().map(|v| v * (1.0 + 4e-16)).collect();
    history(&a.join("REF_hist_2004-01.nc"), &[("f_t", good.clone())]);
    history(&b.join("REF_hist_2004-01.nc"), &[("f_t", noise)]);
    fake_cli(&ws.bin().join("colm-cli"), &a, &b, 0);
    mark_built(&mut ws, "default");
    let strict = parity_check(&mut ws, &case, "default", &Options::default(), None).unwrap();
    assert!(!strict.ok, "bitwise parity fails on rounding noise");
    let loose = parity_check(
        &mut ws,
        &case,
        "default",
        &Options::from(Tolerance {
            rtol: 1e-9,
            atol: 0.0,
        }),
        None,
    )
    .unwrap();
    assert!(
        loose.ok && loose.rtol == 1e-9,
        "{:?} {:?}",
        loose.first_difference,
        loose.compare.as_ref().map(|c| (
            c.differs,
            c.new_nonfinite,
            &c.only_in_a,
            &c.only_in_b,
            c.files
        ))
    );
    let record = ws.info.gates.parity.clone().unwrap();
    assert!(
        record.ok && record.rtol == 1e-9,
        "the gate remembers the tolerance it was judged with"
    );
    // 0.5 K 的偏差是真错位：1e-9 的容差抓得住。
    history(
        &b.join("REF_hist_2004-01.nc"),
        &[("f_t", series(|i| 280.5 + i as f64))],
    );
    let real = parity_check(
        &mut ws,
        &case,
        "default",
        &Options::from(Tolerance {
            rtol: 1e-9,
            atol: 0.0,
        }),
        None,
    )
    .unwrap();
    assert!(!real.ok && real.first_difference.unwrap().contains("f_t"));
    // 提示只在没验证过逐位一致的平台出现（Apple Silicon 与 x86_64 Linux 已验证）。
    assert_eq!(
        real.platform_note.is_some(),
        !(cfg!(all(target_arch = "aarch64", target_os = "macos"))
            || cfg!(all(target_arch = "x86_64", target_os = "linux")))
    );
    if let Some(note) = platform_note() {
        assert!(
            note.contains("libmvec")
                && note.contains("first_records=2")
                && note.contains("rtol=1e-9")
                && note.contains("f_frcsat")
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// 只看最早的记录时，算例副本要改成每步写 history；其余设置不动。
#[test]
fn the_case_copy_can_write_history_every_step() {
    let root = temp("everystep");
    let case = reference_case(&root);
    let nml = case.join("case.nml");
    let text = std::fs::read_to_string(&nml).unwrap();
    std::fs::write(
        &nml,
        text.replace("DEF_CASE_NAME", " DEF_HIST_FREQ = 'DAILY'\n DEF_CASE_NAME"),
    )
    .unwrap();
    let copy = root.join("copy");
    copy_case(&case, &copy).unwrap();
    assert!(std::fs::read_to_string(copy.join("case.nml"))
        .unwrap()
        .contains("'DAILY'"));
    set_history_every_step(&copy).unwrap();
    let after = std::fs::read_to_string(copy.join("case.nml")).unwrap();
    assert!(
        after.contains("DEF_HIST_FREQ = 'TIMESTEP'")
            && !after.contains("'DAILY'")
            && after.contains("DEF_CASE_NAME = 'REF'"),
        "{after}"
    );
    // 没有这一项就报错，而不是悄悄不改。
    let bare = reference_case(&root.join("bare"));
    let bare_copy = root.join("bare-copy");
    copy_case(&bare, &bare_copy).unwrap();
    assert!(set_history_every_step(&bare_copy).is_err());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn physics_regression_rejects_incomplete_histories_and_variable_sets() {
    for report in [
        Report::default(),
        Report {
            files: 1,
            only_in_a: vec!["missing-month.nc".into()],
            ..Report::default()
        },
        Report {
            files: 1,
            only_in_b: vec!["extra-month.nc".into()],
            ..Report::default()
        },
        Report {
            files: 1,
            structural_differences: 1,
            ..Report::default()
        },
    ] {
        let (ok, reason) = judge(ChangeKind::Physics, &report, true);
        assert!(!ok && reason.contains("incomplete"), "{reason}");
    }
}

/// 编完又打了补丁、没重编：两版一致与回归跑的是旧程序，必须拒绝，不能记成新提交通过。
#[test]
fn checks_refuse_binaries_built_on_an_older_commit() {
    let _nc = nc_lock();
    let root = temp("stalebuild");
    let case = reference_case(&root);
    let mut ws = workspace(&root);
    fake_kernel(&ws.kernels().join("default"));
    let a = root.join("fx-a");
    history(
        &a.join("REF_hist_2004-01.nc"),
        &[("f_t", series(|i| i as f64))],
    );
    fake_cli(&ws.bin().join("colm-cli"), &a, &a, 0);
    mark_built(&mut ws, "default");
    std::fs::write(ws.src().join("NEW.txt"), "later change\n").unwrap();
    crate::git::run(&ws.src(), &["add", "-A"]).unwrap();
    crate::git::run(&ws.src(), &["commit", "-q", "-m", "after the build"]).unwrap();
    let err = parity_check(&mut ws, &case, "default", &Options::default(), None).unwrap_err();
    assert!(err.to_string().contains("build_engine"), "{err}");
    // 有未提交的改动时也不跑。
    mark_built(&mut ws, "default");
    std::fs::write(ws.src().join("NEW.txt"), "dirty\n").unwrap();
    let err = parity_check(&mut ws, &case, "default", &Options::default(), None).unwrap_err();
    assert!(err.to_string().contains("commit or revert"), "{err}");
    let _ = std::fs::remove_dir_all(&root);
}

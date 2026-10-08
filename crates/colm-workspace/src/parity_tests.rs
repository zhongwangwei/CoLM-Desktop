#![allow(clippy::field_reassign_with_default)]
use super::*;
use crate::compare::Status;
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
    let ok = parity_check(&mut ws, &case, "default", None).unwrap();
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
    let bad = parity_check(&mut ws, &case, "default", None).unwrap();
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
    let crashed = parity_check(&mut ws, &case, "default", None).unwrap();
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
        &[("f_t", t.clone()), ("f_xerr", closure.clone())],
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
        &[("f_t", t.clone()), ("f_xerr", closure.clone())],
    );
    fake_cli(&ws.bin().join("colm-cli"), &cand, &cand, 0);
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
        &[("f_t", warmer), ("f_xerr", closure.clone())],
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
        (ChangeKind::Physics, true, 1, 2)
    );

    // 闭合变差（水量不平衡从 1e-12 涨到 1e-3）：物理修改也不通过。
    let leaky = series(|_| 1e-3);
    history(
        &cand.join("REF_hist_2004-01.nc"),
        &[("f_t", series(|i| 280.5 + i as f64)), ("f_xerr", leaky)],
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
        &[("f_t", nan), ("f_xerr", closure)],
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

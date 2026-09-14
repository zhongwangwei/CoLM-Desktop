#![cfg(unix)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use sha2::{Digest, Sha256};

static NETCDF_LOCK: Mutex<()> = Mutex::new(());

fn netcdf_lock() -> MutexGuard<'static, ()> {
    NETCDF_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_colm-cli")
}

fn temp_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let label = name.chars().take(6).collect::<String>();
    let root = std::env::temp_dir().join(format!("ct-{label}-{}-{nanos:x}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    root
}

fn run_ok(args: &[&str]) -> String {
    let output = Command::new(bin()).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{args:?}\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn run_fail(args: &[&str]) -> String {
    let output = Command::new(bin()).args(args).output().unwrap();
    assert!(
        !output.status.success(),
        "{args:?} unexpectedly succeeded: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn run_with_bin_ok(program: &Path, args: &[&str]) -> String {
    let output = Command::new(program).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{} {args:?}\nstdout={}\nstderr={}",
        program.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn run_with_bin_fail(program: &Path, args: &[&str]) -> String {
    let output = Command::new(program).args(args).output().unwrap();
    assert!(
        !output.status.success(),
        "{} {args:?} unexpectedly succeeded: {}",
        program.display(),
        String::from_utf8_lossy(&output.stdout)
    );
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn status_json(study: &str) -> Option<Value> {
    let output = Command::new(bin())
        .args(["study-status", study])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| serde_json::from_slice(&output.stdout).ok())
        .flatten()
}

fn wait_for_dispatch_window(study: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Some(status) = status_json(study) {
            let tasks = status["state"]["tasks"]
                .as_object()
                .cloned()
                .unwrap_or_default();
            let running = tasks
                .values()
                .any(|task| matches!(task["status"].as_str(), Some("running" | "evaluating")));
            let pending = tasks
                .values()
                .any(|task| matches!(task["status"].as_str(), Some("queued" | "materialized")));
            if running && pending {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn wait_for_task_state(study: &str, member: &str, wanted: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if let Some(status) = status_json(study) {
            if status["state"]["tasks"][format!("{member}/siteA")]["status"] == wanted {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("{member} never reached {wanted}");
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(unix)]
fn fake_kernel(root: &Path, delay: Option<&str>) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let kernel = root.join("kernel");
    fs::create_dir_all(&kernel).unwrap();
    let golden = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../oracle/golden/CN-Cng_hist_2008-01.nc")
        .canonicalize()
        .unwrap();
    let sleep_line = delay.map(|s| format!("sleep {s}\n")).unwrap_or_default();
    let script = format!(
        r#"#!/bin/sh
set -eu
{sleep_line}nml="$1"
case_name=$(sed -n "s/^[[:space:]]*DEF_CASE_NAME[[:space:]]*=[[:space:]]*'\([^']*\)'.*/\1/p" "$nml")
output_root=$(sed -n "s/^[[:space:]]*DEF_dir_output[[:space:]]*=[[:space:]]*'\([^']*\)'.*/\1/p" "$nml")
lc_year=$(sed -n "s/^[[:space:]]*DEF_LC_YEAR[[:space:]]*=[[:space:]]*\([0-9][0-9]*\).*/\1/p" "$nml")
lc_year=${{lc_year:-2005}}
lc=$(printf 'lc%04d' "$lc_year")
out="$output_root/$case_name"
program=$(basename "$0")
printf '%s\n' "$program" >> '{marker}'
case "$program" in
  mksrfdata*) mkdir -p "$out/landdata"; : > "$out/landdata/srfdata.nc"; echo 'Successful in surface data making.' ;;
  mkinidata*) mkdir -p "$out/restart/const"; : > "$out/restart/const/${{case_name}}_restart_const_${{lc}}_w180_s90.nc"; : > "$out/restart/const/${{case_name}}_restart_const_${{lc}}.nc"; echo 'CoLM Initialization Execution Completed' ;;
  colm*) if [ -f '{fail_marker}' ] && printf '%s' "$case_name" | grep -q '^m000000-'; then echo 'baseline forced failure' >&2; exit 7; fi; mkdir -p "$out/history"; cp '{}' "$out/history/${{case_name}}_hist_2008-01.nc"; echo 'TIMESTEP = 1 | DATE = 2008-01-01'; echo 'CoLM Execution Completed.' ;;
esac
"#,
        golden.display(),
        marker = root.join("kernel-programs.log").display(),
        fail_marker = root.join("fail-baseline").display()
    );
    let mut hashes = serde_json::Map::new();
    for program in colm_kernel::PROGRAMS {
        let path = kernel.join(colm_kernel::program_file(program));
        fs::write(&path, &script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        hashes.insert(program.into(), Value::String(sha256(script.as_bytes())));
    }
    fs::write(
        kernel.join("manifest.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": 1,
            "preset": "cli-study-test",
            "platform": "test",
            "colm_git_sha": "deadbeef",
            "generator_args": "SinglePoint LULC_IGBP",
            "macros": ["SinglePoint", "LULC_IGBP"],
            "built_with": "test",
            "netcdf_c": "test",
            "netcdf_fortran": "test",
            "hdf5": "test",
            "sha256": hashes,
        }))
        .unwrap(),
    )
    .unwrap();
    kernel
}

#[cfg(unix)]
fn private_cli(root: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let bin_dir = root.join("private-bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let cli = bin_dir.join("colm-cli");
    fs::copy(bin(), &cli).unwrap();
    fs::set_permissions(&cli, fs::Permissions::from_mode(0o755)).unwrap();
    cli
}

#[cfg(unix)]
fn install_rust_sidecars(private_cli: &Path, kernel: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let bin_dir = private_cli.parent().unwrap();
    for (source, target) in [
        ("mksrfdata.x", "mksrfdata-rs"),
        ("mkinidata.x", "mkinidata-rs"),
    ] {
        let path = bin_dir.join(target);
        fs::copy(kernel.join(source), &path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn add_kernel_macro(kernel: &Path, name: &str) {
    let manifest = kernel.join("manifest.json");
    let mut json: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    json["macros"]
        .as_array_mut()
        .unwrap()
        .push(Value::String(name.to_owned()));
    fs::write(&manifest, serde_json::to_vec_pretty(&json).unwrap()).unwrap();
}

fn write_case(root: &Path, site: &str) {
    let dir = root.join(site);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("case.nml"),
        format!("&nl_colm\n DEF_CASE_NAME = '{site}'\n DEF_dir_output = 'out'\n DEF_forcing_namelist = 'forcing.nml'\n DEF_LC_YEAR = 2010\n DEF_TUNING_CNFAC = 0.5\n/\n"),
    )
    .unwrap();
    fs::write(dir.join("forcing.nml"), "&nl_colm_forcing\n/\n").unwrap();
}

fn write_obs(path: &Path, good_pairs: usize) {
    write_obs_with_offset(path, good_pairs, 0.0);
}

fn write_obs_with_offset(path: &Path, good_pairs: usize, offset: f64) {
    let golden = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../oracle/golden/CN-Cng_hist_2008-01.nc")
        .canonicalize()
        .unwrap();
    let source = netcdf::open(golden).unwrap();
    let time: Vec<i32> = source.variable("time").unwrap().get_values(..).unwrap();
    let qle: Vec<f64> = source.variable("f_lfevpa").unwrap().get_values(..).unwrap();

    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("time", time.len()).unwrap();
    let mut t = file.add_variable::<f64>("time", &["time"]).unwrap();
    t.put_attribute("units", "seconds since 1900-01-01 00:00:00")
        .unwrap();
    let seconds = time
        .iter()
        .map(|v| *v as f64 * 60.0 + offset)
        .collect::<Vec<_>>();
    t.put_values(&seconds, ..).unwrap();
    file.add_variable::<f64>("Qle", &["time"])
        .unwrap()
        .put_values(&qle, ..)
        .unwrap();
    let qc = (0..time.len())
        .map(|i| if i < good_pairs { 0.0 } else { 1.0 })
        .collect::<Vec<_>>();
    file.add_variable::<f64>("Qle_qc", &["time"])
        .unwrap()
        .put_values(&qc, ..)
        .unwrap();
}

fn write_simple_history(path: &Path, values: &[f64]) {
    let golden = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../oracle/golden/CN-Cng_hist_2008-01.nc")
        .canonicalize()
        .unwrap();
    let source = netcdf::open(golden).unwrap();
    let time: Vec<i32> = source.variable("time").unwrap().get_values(..).unwrap();
    let n = values.len();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("time", n).unwrap();
    file.add_variable::<i32>("time", &["time"])
        .unwrap()
        .put_values(&time[..n], ..)
        .unwrap();
    file.add_variable::<f64>("f_lfevpa", &["time"])
        .unwrap()
        .put_values(values, ..)
        .unwrap();
}

fn write_obs_values(path: &Path, values: &[f64]) {
    let golden = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../oracle/golden/CN-Cng_hist_2008-01.nc")
        .canonicalize()
        .unwrap();
    let source = netcdf::open(golden).unwrap();
    let time: Vec<i32> = source.variable("time").unwrap().get_values(..).unwrap();
    let n = values.len();
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("time", n).unwrap();
    let mut t = file.add_variable::<f64>("time", &["time"]).unwrap();
    t.put_attribute("units", "seconds since 1900-01-01 00:00:00")
        .unwrap();
    let seconds = time[..n]
        .iter()
        .map(|v| *v as f64 * 60.0)
        .collect::<Vec<_>>();
    t.put_values(&seconds, ..).unwrap();
    file.add_variable::<f64>("Qle", &["time"])
        .unwrap()
        .put_values(values, ..)
        .unwrap();
    file.add_variable::<f64>("Qle_qc", &["time"])
        .unwrap()
        .put_values(&vec![0.0; n], ..)
        .unwrap();
}

#[cfg(unix)]
fn parameterized_fake_kernel(root: &Path, low: &Path, high: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let kernel = root.join("param-kernel");
    fs::create_dir_all(&kernel).unwrap();
    let script = format!(
        r#"#!/bin/sh
set -eu
nml="$1"
case_name=$(sed -n "s/^[[:space:]]*DEF_CASE_NAME[[:space:]]*=[[:space:]]*'\([^']*\)'.*/\1/p" "$nml")
output_root=$(sed -n "s/^[[:space:]]*DEF_dir_output[[:space:]]*=[[:space:]]*'\([^']*\)'.*/\1/p" "$nml")
cnfac=$(sed -n "s/^[[:space:]]*DEF_TUNING_CNFAC[[:space:]]*=[[:space:]]*\([-+0-9.eE]*\).*/\1/p" "$nml")
cnfac=${{cnfac:-0.4}}
out="$output_root/$case_name"
program=$(basename "$0")
case "$program" in
  mksrfdata*) mkdir -p "$out/landdata"; : > "$out/landdata/srfdata.nc"; echo 'Successful in surface data making.' ;;
  mkinidata*) mkdir -p "$out/restart/const"; : > "$out/restart/const/${{case_name}}_restart_const_lc2010_w180_s90.nc"; : > "$out/restart/const/${{case_name}}_restart_const_lc2010.nc"; echo 'CoLM Initialization Execution Completed' ;;
  colm*) if [ -f '{slow}' ]; then sleep 0.08; fi; if [ -f '{fail_trial}' ] && printf '%s' "$case_name" | grep -q '^m000005-'; then echo 'trial forced failure' >&2; exit 9; fi; mkdir -p "$out/history"; threshold=0.4000001
[ -f '{threshold}' ] && threshold=$(cat '{threshold}')
if awk -v x="$cnfac" -v t="$threshold" 'BEGIN {{ exit !(x > t) }}'; then cp '{}' "$out/history/${{case_name}}_hist_2008-01.nc"; else cp '{}' "$out/history/${{case_name}}_hist_2008-01.nc"; fi; echo 'CoLM Execution Completed.' ;;
esac
"#,
        high.display(),
        low.display(),
        fail_trial = root.join("fail-m000005").display(),
        slow = root.join("slow-kernel").display(),
        threshold = root.join("threshold").display()
    );
    let mut hashes = serde_json::Map::new();
    for program in colm_kernel::PROGRAMS {
        let path = kernel.join(colm_kernel::program_file(program));
        fs::write(&path, &script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        hashes.insert(program.into(), Value::String(sha256(script.as_bytes())));
    }
    fs::write(
        kernel.join("manifest.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": 1,
            "preset": "cli-study-parameterized-test",
            "platform": "test",
            "colm_git_sha": "deadbeef",
            "generator_args": "SinglePoint LULC_IGBP",
            "macros": ["SinglePoint", "LULC_IGBP"],
            "built_with": "test",
            "netcdf_c": "test",
            "netcdf_fortran": "test",
            "hdf5": "test",
            "sha256": hashes,
        }))
        .unwrap(),
    )
    .unwrap();
    kernel
}

fn write_tuning_spec(root: &Path, kernel: &Path, min_pairs: usize) -> PathBuf {
    write_tuning_spec_for_sites(root, kernel, min_pairs, &["siteA", "siteB"])
}

fn write_tuning_spec_for_sites(
    root: &Path,
    kernel: &Path,
    min_pairs: usize,
    sites: &[&str],
) -> PathBuf {
    let spec = root.join(format!("tuning-{min_pairs}.json"));
    let observations = sites
        .iter()
        .map(|site| ((*site).to_string(), root.join(format!("{site}-obs.nc"))))
        .collect::<std::collections::BTreeMap<_, _>>();
    fs::write(
        &spec,
        serde_json::to_vec_pretty(&serde_json::json!({
            "kind": "tuning",
            "method": "differential-evolution",
            "seed": 11,
            "kernel_dir": kernel,
            "base_cases": sites,
            "observations": observations,
            "parameters": [{"name":"DEF_TUNING_CNFAC","sample_min":0.4,"sample_max":0.6}],
            "targets": [{"key":"Qle","variable":"Qle","from":1199145600,"to":1199577600,"validation_from":1199577600,"validation_to":1200009600,"min_pairs":min_pairs}],
            "budget": {"population":4,"generations":1,"jobs":2}
        }))
        .unwrap(),
    )
    .unwrap();
    spec
}

fn marker_lines(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn first_task_stages_json(state: &Value) -> String {
    let case_dir = state["tasks"]
        .as_object()
        .unwrap()
        .values()
        .find_map(|task| task["case_dir"].as_str())
        .unwrap();
    fs::read_to_string(Path::new(case_dir).join("stages.json")).unwrap()
}

fn study_state_status(program: &Path, study: &str) -> String {
    let status: Value =
        serde_json::from_str(&run_with_bin_ok(program, &["study-status", study])).unwrap();
    status["state"]["status"].as_str().unwrap().to_owned()
}

#[cfg(unix)]
#[test]
fn run_rust_mkinidata_sidecar_change_invalidates_initial_and_runtime_only() {
    let _guard = netcdf_lock();
    let root = temp_root("run-init");
    let cli = private_cli(&root);
    let kernel = fake_kernel(&root, None);
    install_rust_sidecars(&cli, &kernel);
    write_case(&root, "siteA");
    let case = root.join("siteA");
    let call_log = root.join("kernel-programs.log");

    run_with_bin_ok(
        &cli,
        &[
            "run",
            case.to_str().unwrap(),
            "--kernel",
            kernel.to_str().unwrap(),
            "--preprocessors",
            "rust",
        ],
    );
    let first_count = marker_lines(&call_log).len();

    fs::OpenOptions::new()
        .append(true)
        .open(cli.parent().unwrap().join("mkinidata-rs"))
        .unwrap()
        .write_all(b"\n# changed Rust mkinidata sidecar\n")
        .unwrap();
    run_with_bin_ok(
        &cli,
        &[
            "run",
            case.to_str().unwrap(),
            "--kernel",
            kernel.to_str().unwrap(),
            "--preprocessors",
            "rust",
        ],
    );
    let rerun = marker_lines(&call_log)
        .into_iter()
        .skip(first_count)
        .collect::<Vec<_>>();
    assert!(
        !rerun.iter().any(|line| line == "mksrfdata-rs"),
        "changing mkinidata-rs must not invalidate the surface stage: {rerun:?}"
    );
    assert!(
        rerun.iter().any(|line| line == "mkinidata-rs"),
        "changing mkinidata-rs should rerun the initial stage: {rerun:?}"
    );
    assert!(
        rerun.iter().any(|line| line.starts_with("colm")),
        "changing mkinidata-rs should invalidate the downstream runtime stage: {rerun:?}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn run_rust_mksrfdata_sidecar_change_invalidates_all_preprocessing_downstream() {
    let _guard = netcdf_lock();
    let root = temp_root("run-surf");
    let cli = private_cli(&root);
    let kernel = fake_kernel(&root, None);
    install_rust_sidecars(&cli, &kernel);
    write_case(&root, "siteA");
    let case = root.join("siteA");
    let call_log = root.join("kernel-programs.log");

    run_with_bin_ok(
        &cli,
        &[
            "run",
            case.to_str().unwrap(),
            "--kernel",
            kernel.to_str().unwrap(),
            "--preprocessors",
            "rust",
        ],
    );
    let first_count = marker_lines(&call_log).len();

    fs::OpenOptions::new()
        .append(true)
        .open(cli.parent().unwrap().join("mksrfdata-rs"))
        .unwrap()
        .write_all(b"\n# changed Rust mksrfdata sidecar\n")
        .unwrap();
    run_with_bin_ok(
        &cli,
        &[
            "run",
            case.to_str().unwrap(),
            "--kernel",
            kernel.to_str().unwrap(),
            "--preprocessors",
            "rust",
        ],
    );
    let rerun = marker_lines(&call_log)
        .into_iter()
        .skip(first_count)
        .collect::<Vec<_>>();
    assert!(
        rerun.iter().any(|line| line == "mksrfdata-rs"),
        "changing mksrfdata-rs should rerun the surface stage: {rerun:?}"
    );
    assert!(
        rerun.iter().any(|line| line == "mkinidata-rs"),
        "changing mksrfdata-rs should invalidate the initial stage: {rerun:?}"
    );
    assert!(
        rerun.iter().any(|line| line.starts_with("colm")),
        "changing mksrfdata-rs should invalidate the runtime stage: {rerun:?}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn run_stage_colm_with_rust_preprocessors_does_not_require_rust_sidecars() {
    let _guard = netcdf_lock();
    let root = temp_root("run-colm");
    let cli = private_cli(&root);
    let kernel = fake_kernel(&root, None);
    write_case(&root, "siteA");
    let case = root.join("siteA");

    run_with_bin_ok(
        &cli,
        &[
            "run",
            case.to_str().unwrap(),
            "--kernel",
            kernel.to_str().unwrap(),
            "--stage",
            "colm",
            "--preprocessors",
            "rust",
        ],
    );
    let calls = marker_lines(&root.join("kernel-programs.log"));
    assert_eq!(
        calls,
        vec!["colm.x".to_owned()],
        "--stage colm should not inspect or execute Rust preprocessor sidecars"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn study_run_defaults_to_rust_preprocessors_and_reuses_rust_stage_fingerprints() {
    let _guard = netcdf_lock();
    let root = temp_root("rust-default");
    let cli = private_cli(&root);
    let kernel = fake_kernel(&root, None);
    install_rust_sidecars(&cli, &kernel);
    write_case(&root, "siteA");
    write_obs(&root.join("siteA-obs.nc"), 264);
    let spec = write_tuning_spec_for_sites(&root, &kernel, 10, &["siteA"]);

    let study = run_with_bin_ok(
        &cli,
        &[
            "study-create",
            root.to_str().unwrap(),
            "--spec",
            spec.to_str().unwrap(),
        ],
    )
    .trim()
    .to_string();
    let first: Value = serde_json::from_str(&run_with_bin_ok(
        &cli,
        &[
            "study-run",
            &study,
            "--kernel",
            kernel.to_str().unwrap(),
            "--jobs",
            "1",
        ],
    ))
    .unwrap();
    assert_eq!(first["status"], "completed");

    let call_log = root.join("kernel-programs.log");
    let calls = marker_lines(&call_log);
    assert!(
        calls.iter().any(|line| line == "mksrfdata-rs"),
        "default study-run should execute the Rust mksrfdata sidecar: {calls:?}"
    );
    assert!(
        calls.iter().any(|line| line == "mkinidata-rs"),
        "default study-run should execute the Rust mkinidata sidecar: {calls:?}"
    );
    assert!(
        calls.iter().any(|line| line.starts_with("colm")),
        "Rust preprocessing should still execute the Fortran colm runtime: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|line| line == "mksrfdata.x" || line == "mkinidata.x"),
        "Rust preprocessing must not fall back to kernel preprocessors: {calls:?}"
    );
    let stages = first_task_stages_json(&first);
    assert!(
        stages.contains("preprocessors=rust"),
        "study stage fingerprint should record Rust preprocessors: {stages}"
    );
    assert!(
        !stages.contains("preprocessors=fortran"),
        "Rust study stage fingerprint must not retain Fortran identity: {stages}"
    );

    let call_count = calls.len();
    let second: Value = serde_json::from_str(&run_with_bin_ok(
        &cli,
        &[
            "study-run",
            &study,
            "--kernel",
            kernel.to_str().unwrap(),
            "--jobs",
            "1",
        ],
    ))
    .unwrap();
    assert_eq!(second["status"], "completed");
    assert_eq!(
        marker_lines(&call_log).len(),
        call_count,
        "a second unchanged Rust study-run should reuse stage fingerprints instead of rerunning stages"
    );

    fs::OpenOptions::new()
        .append(true)
        .open(cli.parent().unwrap().join("mkinidata-rs"))
        .unwrap()
        .write_all(b"\n# changed Rust preprocessor binary\n")
        .unwrap();
    let stale: Value = serde_json::from_str(&run_with_bin_ok(
        &cli,
        &[
            "study-run",
            &study,
            "--kernel",
            kernel.to_str().unwrap(),
            "--jobs",
            "1",
        ],
    ))
    .unwrap();
    let closed_member = &stale["tasks"]["m000005/siteA"];
    assert_eq!(
        closed_member["status"], "failed",
        "changing the Rust sidecar binary should stale closed DE members instead of reusing old results"
    );
    let reason = closed_member["reason"].as_str().unwrap();
    assert!(reason.contains("closed DE generation"), "{reason}");
    assert!(reason.contains("create a new Study"), "{reason}");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn study_run_default_rust_fails_when_sidecars_are_missing_without_fortran_fallback() {
    let _guard = netcdf_lock();
    let root = temp_root("rust-missing");
    let cli = private_cli(&root);
    let kernel = fake_kernel(&root, None);
    write_case(&root, "siteA");
    write_obs(&root.join("siteA-obs.nc"), 264);
    let spec = write_tuning_spec_for_sites(&root, &kernel, 10, &["siteA"]);
    let study = run_with_bin_ok(
        &cli,
        &[
            "study-create",
            root.to_str().unwrap(),
            "--spec",
            spec.to_str().unwrap(),
        ],
    )
    .trim()
    .to_string();

    let stderr = run_with_bin_fail(
        &cli,
        &[
            "study-run",
            &study,
            "--kernel",
            kernel.to_str().unwrap(),
            "--jobs",
            "1",
        ],
    );
    assert!(
        stderr.contains("Rust preprocessor is missing"),
        "missing Rust sidecars must be reported instead of silently falling back: {stderr}"
    );
    assert_ne!(
        study_state_status(&cli, &study),
        "completed_with_failures",
        "missing sidecars should fail before dispatch, not mark the Study completed with failures"
    );
    let kernel_calls = marker_lines(&root.join("kernel-programs.log"));
    assert!(
        !kernel_calls
            .iter()
            .any(|line| line == "mksrfdata.x" || line == "mkinidata.x"),
        "missing Rust sidecars must not fall back to Fortran preprocessors: {kernel_calls:?}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn study_run_default_rust_rejects_hyperspectral_before_dispatching_tasks() {
    let _guard = netcdf_lock();
    let root = temp_root("rust-hyper");
    let cli = private_cli(&root);
    let kernel = fake_kernel(&root, None);
    add_kernel_macro(&kernel, "HYPERSPECTRAL");
    write_case(&root, "siteA");
    write_obs(&root.join("siteA-obs.nc"), 264);
    let spec = write_tuning_spec_for_sites(&root, &kernel, 10, &["siteA"]);
    let study = run_with_bin_ok(
        &cli,
        &[
            "study-create",
            root.to_str().unwrap(),
            "--spec",
            spec.to_str().unwrap(),
        ],
    )
    .trim()
    .to_string();

    let stderr = run_with_bin_fail(
        &cli,
        &[
            "study-run",
            &study,
            "--kernel",
            kernel.to_str().unwrap(),
            "--jobs",
            "1",
        ],
    );
    assert!(
        stderr.contains("Study Rust preprocessing does not yet support HYPERSPECTRAL"),
        "HYPERSPECTRAL Rust Study should fail with the explicit input-fingerprint guard: {stderr}"
    );
    assert_ne!(
        study_state_status(&cli, &study),
        "completed_with_failures",
        "HYPERSPECTRAL Rust rejection should happen before task dispatch"
    );
    assert!(
        marker_lines(&root.join("kernel-programs.log")).is_empty(),
        "HYPERSPECTRAL Rust rejection should not launch preprocessors or runtime"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn switching_completed_closed_de_study_from_fortran_to_default_rust_marks_closed_members_stale() {
    let _guard = netcdf_lock();
    let root = temp_root("backend-stale");
    let cli = private_cli(&root);
    let kernel = fake_kernel(&root, None);
    install_rust_sidecars(&cli, &kernel);
    write_case(&root, "siteA");
    write_obs(&root.join("siteA-obs.nc"), 264);
    let spec = write_tuning_spec_for_sites(&root, &kernel, 10, &["siteA"]);
    let study = run_with_bin_ok(
        &cli,
        &[
            "study-create",
            root.to_str().unwrap(),
            "--spec",
            spec.to_str().unwrap(),
        ],
    )
    .trim()
    .to_string();

    let completed: Value = serde_json::from_str(&run_with_bin_ok(
        &cli,
        &[
            "study-run",
            &study,
            "--kernel",
            kernel.to_str().unwrap(),
            "--jobs",
            "1",
            "--preprocessors",
            "fortran",
        ],
    ))
    .unwrap();
    assert_eq!(completed["status"], "completed");

    let rerun: Value = serde_json::from_str(&run_with_bin_ok(
        &cli,
        &[
            "study-run",
            &study,
            "--kernel",
            kernel.to_str().unwrap(),
            "--jobs",
            "1",
        ],
    ))
    .unwrap();
    let task = &rerun["tasks"]["m000005/siteA"];
    assert_eq!(task["status"], "failed");
    let reason = task["reason"].as_str().unwrap();
    assert!(reason.contains("closed DE generation"), "{reason}");
    assert!(reason.contains("create a new Study"), "{reason}");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn study_run_explicit_fortran_keeps_kernel_preprocessors_and_fortran_fingerprints() {
    let _guard = netcdf_lock();
    let root = temp_root("explicit-fortran");
    let cli = private_cli(&root);
    let kernel = fake_kernel(&root, None);
    install_rust_sidecars(&cli, &kernel);
    write_case(&root, "siteA");
    write_obs(&root.join("siteA-obs.nc"), 264);
    let spec = write_tuning_spec_for_sites(&root, &kernel, 10, &["siteA"]);

    let study = run_with_bin_ok(
        &cli,
        &[
            "study-create",
            root.to_str().unwrap(),
            "--spec",
            spec.to_str().unwrap(),
        ],
    )
    .trim()
    .to_string();
    let state: Value = serde_json::from_str(&run_with_bin_ok(
        &cli,
        &[
            "study-run",
            &study,
            "--kernel",
            kernel.to_str().unwrap(),
            "--jobs",
            "1",
            "--preprocessors",
            "fortran",
        ],
    ))
    .unwrap();
    assert_eq!(state["status"], "completed");

    let kernel_calls = marker_lines(&root.join("kernel-programs.log"));
    assert!(
        kernel_calls.iter().any(|line| line == "mksrfdata.x")
            && kernel_calls.iter().any(|line| line == "mkinidata.x")
            && kernel_calls.iter().any(|line| line.starts_with("colm")),
        "explicit Fortran preprocessing should use kernel preprocessors and runtime: {kernel_calls:?}"
    );
    assert!(
        !kernel_calls
            .iter()
            .any(|line| line == "mksrfdata-rs" || line == "mkinidata-rs"),
        "explicit Fortran preprocessing should not execute Rust sidecars: {kernel_calls:?}"
    );
    let stages = first_task_stages_json(&state);
    assert!(
        stages.contains("preprocessors=fortran"),
        "Fortran study stage fingerprint should record Fortran preprocessors: {stages}"
    );
    assert!(
        !stages.contains("preprocessors=rust"),
        "Fortran study stage fingerprint must not be marked Rust: {stages}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn multi_site_tuning_runs_and_applies_best_member() {
    let _guard = netcdf_lock();
    let root = temp_root("tuning-apply");
    let kernel = fake_kernel(&root, None);
    for site in ["siteA", "siteB"] {
        write_case(&root, site);
        write_obs(&root.join(format!("{site}-obs.nc")), 264);
    }
    let spec = write_tuning_spec(&root, &kernel, 10);

    run_ok(&[
        "study-preflight",
        root.to_str().unwrap(),
        "--spec",
        spec.to_str().unwrap(),
    ]);
    let study = run_ok(&[
        "study-create",
        root.to_str().unwrap(),
        "--spec",
        spec.to_str().unwrap(),
    ])
    .trim()
    .to_string();
    let state: Value = serde_json::from_str(&run_ok(&[
        "study-run",
        &study,
        "--kernel",
        kernel.to_str().unwrap(),
        "--jobs",
        "2",
        "--preprocessors",
        "fortran",
    ]))
    .unwrap();
    assert_eq!(state["status"], "completed");
    assert!(state["best_member"].as_str().unwrap().starts_with('m'));

    let preview: Value = serde_json::from_str(&run_ok(&[
        "study-apply-preview",
        &study,
        "--member",
        "best",
    ]))
    .unwrap();
    assert_eq!(preview.as_array().unwrap().len(), 2);
    let out = root.join("best-applied");
    let applied: Value = serde_json::from_str(&run_ok(&[
        "study-apply",
        &study,
        "--member",
        "best",
        "--out",
        out.to_str().unwrap(),
    ]))
    .unwrap();
    assert_eq!(applied.as_array().unwrap().len(), 2);
    assert!(out.join("siteA/case.nml").is_file());
    assert!(out.join("siteB/case.nml").is_file());
    for row in preview.as_array().unwrap() {
        let saved =
            fs::read_to_string(out.join(row["site"].as_str().unwrap()).join("case.nml")).unwrap();
        assert!(saved.contains(row["new"].as_str().unwrap()), "{saved}");
    }
    fs::remove_dir_all(root).unwrap();
}

fn parameter_study(
    root: &Path,
    generations: usize,
    jobs: usize,
    patience: usize,
) -> (PathBuf, PathBuf) {
    let values = (0..24).map(|i| 10.0 + i as f64).collect::<Vec<_>>();
    let bad = values.iter().map(|value| value + 50.0).collect::<Vec<_>>();
    let low = root.join("low.nc");
    let high = root.join("high.nc");
    write_simple_history(&low, &bad);
    write_simple_history(&high, &values);
    let kernel = parameterized_fake_kernel(root, &low, &high);
    fs::create_dir_all(root.join("siteA")).unwrap();
    fs::write(
        root.join("siteA/case.nml"),
        "&nl_colm\n DEF_CASE_NAME = 'siteA'\n DEF_dir_output = 'out'\n DEF_forcing_namelist = 'forcing.nml'\n DEF_LC_YEAR = 2010\n DEF_TUNING_CNFAC = 0.4\n/\n",
    )
    .unwrap();
    fs::write(root.join("siteA/forcing.nml"), "&nl_colm_forcing\n/\n").unwrap();
    write_obs_values(&root.join("siteA-obs.nc"), &values);
    let spec = root.join("param-study.json");
    fs::write(
        &spec,
        serde_json::to_vec_pretty(&serde_json::json!({
            "kind": "tuning",
            "method": "differential-evolution",
            "seed": 11,
            "kernel_dir": kernel,
            "base_cases": ["siteA"],
            "observations": {"siteA": root.join("siteA-obs.nc")},
            "parameters": [{"name":"DEF_TUNING_CNFAC","sample_min":0.4,"sample_max":0.6}],
            "targets": [{"key":"Qle","variable":"Qle","from":1199145600,"to":1199232000,"min_pairs":10}],
            "budget": {"population":4,"generations":generations,"jobs":jobs,"patience":patience}
        }))
        .unwrap(),
    )
    .unwrap();
    (kernel, spec)
}

#[cfg(unix)]
#[test]
fn tuning_prefers_parameter_dependent_candidate_over_baseline() {
    let _guard = netcdf_lock();
    let root = temp_root("param-opt");
    let values = (0..24).map(|i| 10.0 + i as f64).collect::<Vec<_>>();
    let bad = values.iter().map(|value| value + 50.0).collect::<Vec<_>>();
    let low = root.join("low.nc");
    let high = root.join("high.nc");
    write_simple_history(&low, &bad);
    write_simple_history(&high, &values);
    let kernel = parameterized_fake_kernel(&root, &low, &high);
    fs::create_dir_all(root.join("siteA")).unwrap();
    fs::write(
        root.join("siteA/case.nml"),
        "&nl_colm\n DEF_CASE_NAME = 'siteA'\n DEF_dir_output = 'out'\n DEF_forcing_namelist = 'forcing.nml'\n DEF_LC_YEAR = 2010\n DEF_TUNING_CNFAC = 0.4\n/\n",
    )
    .unwrap();
    fs::write(root.join("siteA/forcing.nml"), "&nl_colm_forcing\n/\n").unwrap();
    write_obs_values(&root.join("siteA-obs.nc"), &values);
    let spec = root.join("param-tuning.json");
    fs::write(
        &spec,
        serde_json::to_vec_pretty(&serde_json::json!({
            "kind": "tuning",
            "method": "differential-evolution",
            "seed": 11,
            "kernel_dir": kernel,
            "base_cases": ["siteA"],
            "observations": {"siteA": root.join("siteA-obs.nc")},
            "parameters": [{"name":"DEF_TUNING_CNFAC","sample_min":0.4,"sample_max":0.6}],
            "targets": [{"key":"Qle","variable":"Qle","from":1199145600,"to":1199232000,"min_pairs":10}],
            "budget": {"population":4,"generations":1,"jobs":2}
        }))
        .unwrap(),
    )
    .unwrap();
    let study = run_ok(&[
        "study-create",
        root.to_str().unwrap(),
        "--spec",
        spec.to_str().unwrap(),
    ])
    .trim()
    .to_string();
    let state: Value = serde_json::from_str(&run_ok(&[
        "study-run",
        &study,
        "--kernel",
        kernel.to_str().unwrap(),
        "--jobs",
        "2",
        "--preprocessors",
        "fortran",
    ]))
    .unwrap();
    assert_eq!(state["status"], "completed");
    let best = state["best_member"].as_str().unwrap();
    assert_ne!(best, "m000000");
    let baseline = state["candidates"]["m000000"]["calibration"]
        .as_f64()
        .unwrap();
    let best_score = state["candidates"][best]["calibration"].as_f64().unwrap();
    assert!(
        best_score < baseline,
        "best={best_score} baseline={baseline}"
    );
    let task_result = Path::new(&study)
        .join("results/tasks")
        .join(best)
        .join("siteA.json");
    let mut cached: Value = serde_json::from_slice(&fs::read(&task_result).unwrap()).unwrap();
    cached["calibration"][0]["support_hash"] = serde_json::json!("");
    fs::write(&task_result, serde_json::to_vec_pretty(&cached).unwrap()).unwrap();
    let error = run_fail(&["study-apply-preview", &study, "--member", best]);
    assert!(
        error.contains("not feasible") || error.contains("missing pair support"),
        "{error}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn tuning_validation_failure_warns_without_breaking_calibration_selection() {
    let _guard = netcdf_lock();
    let root = temp_root("val-warn");
    let calibration = (0..24).map(|i| 10.0 + i as f64).collect::<Vec<_>>();
    let mut observation = calibration.clone();
    observation.extend((0..24).map(|i| 100.0 + i as f64));
    let low = root.join("low.nc");
    let high = root.join("high.nc");
    write_simple_history(
        &low,
        &calibration
            .iter()
            .map(|value| value + 50.0)
            .collect::<Vec<_>>(),
    );
    write_simple_history(&high, &calibration);
    let kernel = parameterized_fake_kernel(&root, &low, &high);
    fs::create_dir_all(root.join("siteA")).unwrap();
    fs::write(
        root.join("siteA/case.nml"),
        "&nl_colm\n DEF_CASE_NAME = 'siteA'\n DEF_dir_output = 'out'\n DEF_forcing_namelist = 'forcing.nml'\n DEF_LC_YEAR = 2010\n DEF_TUNING_CNFAC = 0.4\n/\n",
    )
    .unwrap();
    fs::write(root.join("siteA/forcing.nml"), "&nl_colm_forcing\n/\n").unwrap();
    write_obs_values(&root.join("siteA-obs.nc"), &observation);
    let spec = root.join("validation-warning.json");
    fs::write(
        &spec,
        serde_json::to_vec_pretty(&serde_json::json!({
            "kind": "tuning",
            "method": "differential-evolution",
            "seed": 11,
            "kernel_dir": kernel,
            "base_cases": ["siteA"],
            "observations": {"siteA": root.join("siteA-obs.nc")},
            "parameters": [{"name":"DEF_TUNING_CNFAC","sample_min":0.4,"sample_max":0.6}],
            "targets": [{"key":"Qle","variable":"Qle","from":1199145600,"to":1199232000,"validation_from":1199232000,"validation_to":1199318400,"min_pairs":10}],
            "budget": {"population":4,"generations":1,"jobs":2}
        }))
        .unwrap(),
    )
    .unwrap();
    let study = run_ok(&[
        "study-create",
        root.to_str().unwrap(),
        "--spec",
        spec.to_str().unwrap(),
    ])
    .trim()
    .to_string();
    let state: Value = serde_json::from_str(&run_ok(&[
        "study-run",
        &study,
        "--kernel",
        kernel.to_str().unwrap(),
        "--jobs",
        "2",
        "--preprocessors",
        "fortran",
    ]))
    .unwrap();
    assert_eq!(state["status"], "completed");
    assert!(state["best_member"].as_str().is_some());
    assert!(state["warnings"].as_array().unwrap().iter().any(|warning| {
        warning
            .as_str()
            .is_some_and(|text| text.contains("validation target Qle unavailable"))
    }));
    let best = state["best_member"].as_str().unwrap();
    assert!(state["candidates"][best]["calibration"].as_f64().is_some());
    assert!(state["candidates"][best]["validation"].is_null());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn retry_failed_does_not_reopen_closed_de_generation() {
    let _guard = netcdf_lock();
    let root = temp_root("closed-retry");
    let values = (0..24).map(|i| 10.0 + i as f64).collect::<Vec<_>>();
    let bad = values.iter().map(|value| value + 50.0).collect::<Vec<_>>();
    let low = root.join("low.nc");
    let high = root.join("high.nc");
    write_simple_history(&low, &bad);
    write_simple_history(&high, &values);
    fs::write(root.join("fail-m000005"), b"fail").unwrap();
    let kernel = parameterized_fake_kernel(&root, &low, &high);
    fs::create_dir_all(root.join("siteA")).unwrap();
    fs::write(
        root.join("siteA/case.nml"),
        "&nl_colm\n DEF_CASE_NAME = 'siteA'\n DEF_dir_output = 'out'\n DEF_forcing_namelist = 'forcing.nml'\n DEF_LC_YEAR = 2010\n DEF_TUNING_CNFAC = 0.4\n/\n",
    )
    .unwrap();
    fs::write(root.join("siteA/forcing.nml"), "&nl_colm_forcing\n/\n").unwrap();
    write_obs_values(&root.join("siteA-obs.nc"), &values);
    let spec = root.join("closed-retry.json");
    fs::write(
        &spec,
        serde_json::to_vec_pretty(&serde_json::json!({
            "kind": "tuning",
            "method": "differential-evolution",
            "seed": 11,
            "kernel_dir": kernel,
            "base_cases": ["siteA"],
            "observations": {"siteA": root.join("siteA-obs.nc")},
            "parameters": [{"name":"DEF_TUNING_CNFAC","sample_min":0.4,"sample_max":0.6}],
            "targets": [{"key":"Qle","variable":"Qle","from":1199145600,"to":1199232000,"min_pairs":10}],
            "budget": {"population":4,"generations":1,"jobs":2}
        }))
        .unwrap(),
    )
    .unwrap();
    let study = run_ok(&[
        "study-create",
        root.to_str().unwrap(),
        "--spec",
        spec.to_str().unwrap(),
    ])
    .trim()
    .to_string();
    let failed: Value = serde_json::from_str(&run_ok(&[
        "study-run",
        &study,
        "--kernel",
        kernel.to_str().unwrap(),
        "--jobs",
        "2",
        "--preprocessors",
        "fortran",
    ]))
    .unwrap();
    assert_eq!(failed["generation"], 1);
    assert_eq!(failed["tasks"]["m000005/siteA"]["status"], "failed");
    fs::remove_file(root.join("fail-m000005")).unwrap();
    let retry: Value = serde_json::from_str(&run_ok(&[
        "study-run",
        &study,
        "--kernel",
        kernel.to_str().unwrap(),
        "--jobs",
        "2",
        "--preprocessors",
        "fortran",
        "--retry-failed",
        "1",
    ]))
    .unwrap();
    assert_eq!(retry["tasks"]["m000005/siteA"]["status"], "failed");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn stale_success_in_closed_de_generation_is_failed_without_rerun() {
    let _guard = netcdf_lock();
    let root = temp_root("stale-closed");
    let (kernel, spec) = parameter_study(&root, 1, 2, 3);
    let study = run_ok(&[
        "study-create",
        root.to_str().unwrap(),
        "--spec",
        spec.to_str().unwrap(),
    ])
    .trim()
    .to_string();
    let completed: Value = serde_json::from_str(&run_ok(&[
        "study-run",
        &study,
        "--kernel",
        kernel.to_str().unwrap(),
        "--jobs",
        "2",
        "--preprocessors",
        "fortran",
    ]))
    .unwrap();
    assert_eq!(completed["status"], "completed");
    assert_eq!(completed["tasks"]["m000005/siteA"]["status"], "succeeded");

    let forcing_path = Path::new(&study).join("members/m000005/siteA/forcing.nml");
    fs::OpenOptions::new()
        .append(true)
        .open(&forcing_path)
        .unwrap()
        .write_all(b"\n! stale closed generation\n")
        .unwrap();

    let rerun: Value = serde_json::from_str(&run_ok(&[
        "study-run",
        &study,
        "--kernel",
        kernel.to_str().unwrap(),
        "--jobs",
        "2",
        "--preprocessors",
        "fortran",
    ]))
    .unwrap();
    let task = &rerun["tasks"]["m000005/siteA"];
    assert_eq!(task["status"], "failed");
    assert!(task["objective"].is_null());
    assert!(task["validation_objective"].is_null());
    let reason = task["reason"].as_str().unwrap();
    assert!(reason.contains("closed DE generation"), "{reason}");
    assert!(reason.contains("create a new Study"), "{reason}");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn pause_resume_keeps_open_de_generation_selection_and_patience_stable() {
    let _guard = netcdf_lock();
    let completed_root = temp_root("resume-full");
    fs::write(completed_root.join("threshold"), b"0.59").unwrap();
    let (completed_kernel, completed_spec) = parameter_study(&completed_root, 2, 1, 1);
    let completed_study = run_ok(&[
        "study-create",
        completed_root.to_str().unwrap(),
        "--spec",
        completed_spec.to_str().unwrap(),
    ])
    .trim()
    .to_string();
    let completed: Value = serde_json::from_str(&run_ok(&[
        "study-run",
        &completed_study,
        "--kernel",
        completed_kernel.to_str().unwrap(),
        "--jobs",
        "1",
        "--preprocessors",
        "fortran",
    ]))
    .unwrap();
    assert_eq!(completed["status"], "completed");
    assert_eq!(completed["generation"], 2);

    let paused_root = temp_root("resume-pause");
    fs::write(paused_root.join("threshold"), b"0.59").unwrap();
    fs::write(paused_root.join("slow-kernel"), b"slow").unwrap();
    let (paused_kernel, paused_spec) = parameter_study(&paused_root, 2, 1, 1);
    let paused_study = run_ok(&[
        "study-create",
        paused_root.to_str().unwrap(),
        "--spec",
        paused_spec.to_str().unwrap(),
    ])
    .trim()
    .to_string();
    let child = Command::new(bin())
        .args([
            "study-run",
            &paused_study,
            "--kernel",
            paused_kernel.to_str().unwrap(),
            "--jobs",
            "1",
            "--preprocessors",
            "fortran",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for_task_state(&paused_study, "m000005", "running");
    run_ok(&["study-pause", &paused_study]);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    fs::remove_file(paused_root.join("slow-kernel")).unwrap();
    run_ok(&["study-resume", &paused_study]);
    let resumed: Value = serde_json::from_str(&run_ok(&[
        "study-run",
        &paused_study,
        "--kernel",
        paused_kernel.to_str().unwrap(),
        "--jobs",
        "1",
        "--preprocessors",
        "fortran",
    ]))
    .unwrap();
    assert_eq!(resumed["status"], "completed");
    assert_eq!(resumed["generation"], completed["generation"]);
    assert_eq!(resumed["population"], completed["population"]);
    assert_eq!(resumed["best_member"], completed["best_member"]);
    assert_eq!(resumed["best_objective"], completed["best_objective"]);
    for sample in ["g000000.csv", "g000001.csv", "g000002.csv"] {
        let completed_sample =
            fs::read(Path::new(&completed_study).join("samples").join(sample)).unwrap();
        let resumed_sample =
            fs::read(Path::new(&paused_study).join("samples").join(sample)).unwrap();
        assert_eq!(resumed_sample, completed_sample, "{sample}");
    }
    assert_eq!(
        resumed["no_improvement_generations"],
        completed["no_improvement_generations"]
    );
    let initial_best = resumed["candidates"]
        .as_object()
        .unwrap()
        .values()
        .filter(|candidate| {
            candidate["generation"] == 0 && candidate["calibration"].as_f64().is_some()
        })
        .filter_map(|candidate| candidate["calibration"].as_f64())
        .fold(f64::INFINITY, f64::min);
    let best = resumed["best_objective"].as_f64().unwrap();
    assert!(best < initial_best, "best={best} initial={initial_best}");
    fs::remove_dir_all(completed_root).unwrap();
    fs::remove_dir_all(paused_root).unwrap();
}

#[cfg(unix)]
#[test]
fn tuning_preflight_rejects_windows_with_too_few_usable_observations() {
    let _guard = netcdf_lock();
    let root = temp_root("pair-preflight");
    let kernel = fake_kernel(&root, None);
    for site in ["siteA", "siteB"] {
        write_case(&root, site);
        write_obs(&root.join(format!("{site}-obs.nc")), 1);
    }
    let spec = write_tuning_spec(&root, &kernel, 10);

    let error = run_fail(&[
        "study-preflight",
        root.to_str().unwrap(),
        "--spec",
        spec.to_str().unwrap(),
    ]);
    assert!(
        error.contains("Qle")
            && (error.contains("pairs")
                || error.contains("usable observation points")
                || error.contains("配对")),
        "{error}"
    );

    for site in ["siteA", "siteB"] {
        fs::remove_file(root.join(format!("{site}-obs.nc"))).unwrap();
        write_obs(&root.join(format!("{site}-obs.nc")), 120);
    }
    let validation_error = run_fail(&[
        "study-preflight",
        root.to_str().unwrap(),
        "--spec",
        spec.to_str().unwrap(),
    ]);
    assert!(
        validation_error.contains("validation"),
        "{validation_error}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn tuning_baseline_blocks_candidates_when_real_pairs_do_not_align() {
    let _guard = netcdf_lock();
    let root = temp_root("pair-alignment");
    let kernel = fake_kernel(&root, None);
    for site in ["siteA", "siteB"] {
        write_case(&root, site);
        write_obs_with_offset(&root.join(format!("{site}-obs.nc")), 264, 900.0);
    }
    let spec = write_tuning_spec(&root, &kernel, 10);
    run_ok(&[
        "study-preflight",
        root.to_str().unwrap(),
        "--spec",
        spec.to_str().unwrap(),
    ]);
    let study = run_ok(&[
        "study-create",
        root.to_str().unwrap(),
        "--spec",
        spec.to_str().unwrap(),
    ])
    .trim()
    .to_string();
    let state: Value = serde_json::from_str(&run_ok(&[
        "study-run",
        &study,
        "--kernel",
        kernel.to_str().unwrap(),
        "--jobs",
        "2",
        "--preprocessors",
        "fortran",
    ]))
    .unwrap();
    assert_eq!(state["status"], "completed_with_failures");
    assert!(state["tasks"]
        .as_object()
        .unwrap()
        .iter()
        .any(|(id, task)| {
            id.starts_with("m000000/")
                && task["status"] == "failed"
                && task["reason"].as_str().is_some_and(|reason| {
                    reason.contains("unavailable") || reason.contains("pairs")
                })
        }));
    assert!(state["tasks"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(id, _)| !id.starts_with("m000000/"))
        .all(|(_, task)| task["status"] == "materialized"));
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn jobs_greater_than_one_pause_resume_and_cancel_are_recoverable() {
    let _guard = netcdf_lock();
    let root = temp_root("pause-cancel");
    let kernel = fake_kernel(&root, Some("0.08"));
    let sites = ["siteA", "siteB", "siteC"];
    for site in sites {
        write_case(&root, site);
        write_obs(&root.join(format!("{site}-obs.nc")), 264);
    }
    let spec = write_tuning_spec_for_sites(&root, &kernel, 10, &sites);
    let study = run_ok(&[
        "study-create",
        root.to_str().unwrap(),
        "--spec",
        spec.to_str().unwrap(),
    ])
    .trim()
    .to_string();

    let mut child = Command::new(bin())
        .args([
            "study-run",
            &study,
            "--kernel",
            kernel.to_str().unwrap(),
            "--jobs",
            "2",
            "--preprocessors",
            "fortran",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for_dispatch_window(&study);
    run_ok(&["study-pause", &study]);
    assert!(child.wait().unwrap().success());
    let paused: Value = serde_json::from_str(&run_ok(&["study-status", &study])).unwrap();
    assert_eq!(paused["state"]["status"], "paused");

    run_ok(&["study-resume", &study]);
    let resumed: Value = serde_json::from_str(&run_ok(&[
        "study-run",
        &study,
        "--kernel",
        kernel.to_str().unwrap(),
        "--jobs",
        "2",
        "--preprocessors",
        "fortran",
    ]))
    .unwrap();
    assert!(matches!(
        resumed["status"].as_str(),
        Some("completed" | "completed_with_failures")
    ));

    let cancel_root = temp_root("cancel");
    let cancel_kernel = fake_kernel(&cancel_root, Some("0.08"));
    for site in sites {
        write_case(&cancel_root, site);
        write_obs(&cancel_root.join(format!("{site}-obs.nc")), 264);
    }
    let cancel_spec = write_tuning_spec_for_sites(&cancel_root, &cancel_kernel, 10, &sites);
    let cancel_study = run_ok(&[
        "study-create",
        cancel_root.to_str().unwrap(),
        "--spec",
        cancel_spec.to_str().unwrap(),
    ])
    .trim()
    .to_string();
    let mut child = Command::new(bin())
        .args([
            "study-run",
            &cancel_study,
            "--kernel",
            cancel_kernel.to_str().unwrap(),
            "--jobs",
            "2",
            "--preprocessors",
            "fortran",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for_dispatch_window(&cancel_study);
    run_ok(&["study-cancel", &cancel_study]);
    assert!(child.wait().unwrap().success());
    let cancelled: Value = serde_json::from_str(&run_ok(&["study-status", &cancel_study])).unwrap();
    assert_eq!(cancelled["state"]["status"], "cancelled");

    let recovery_root = temp_root("recovery");
    let recovery_kernel = fake_kernel(&recovery_root, Some("0.08"));
    for site in sites {
        write_case(&recovery_root, site);
        write_obs(&recovery_root.join(format!("{site}-obs.nc")), 264);
    }
    let recovery_spec = write_tuning_spec_for_sites(&recovery_root, &recovery_kernel, 10, &sites);
    let recovery_study = run_ok(&[
        "study-create",
        recovery_root.to_str().unwrap(),
        "--spec",
        recovery_spec.to_str().unwrap(),
    ])
    .trim()
    .to_string();
    let mut child = Command::new(bin())
        .args([
            "study-run",
            &recovery_study,
            "--kernel",
            recovery_kernel.to_str().unwrap(),
            "--jobs",
            "2",
            "--preprocessors",
            "fortran",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for_dispatch_window(&recovery_study);
    child.kill().unwrap();
    child.wait().unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let lock = Path::new(&recovery_study).join("run.lock");
    let mut owner: Value = serde_json::from_slice(&fs::read(&lock).unwrap()).unwrap();
    owner["heartbeat_unix"] = Value::from(0);
    fs::write(&lock, serde_json::to_vec(&owner).unwrap()).unwrap();
    let interrupted: Value =
        serde_json::from_str(&run_ok(&["study-status", &recovery_study])).unwrap();
    assert_eq!(interrupted["state"]["status"], "needs_review");
    run_ok(&["study-retry", &recovery_study, "--include-review", "1"]);
    let recovered: Value = serde_json::from_str(&run_ok(&[
        "study-run",
        &recovery_study,
        "--kernel",
        recovery_kernel.to_str().unwrap(),
        "--jobs",
        "2",
        "--preprocessors",
        "fortran",
    ]))
    .unwrap();
    assert_eq!(recovered["status"], "completed");

    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(cancel_root).unwrap();
    fs::remove_dir_all(recovery_root).unwrap();
}

#[cfg(unix)]
#[test]
fn unfinished_tuning_member_cannot_be_applied() {
    let _guard = netcdf_lock();
    let root = temp_root("premature-apply");
    let kernel = fake_kernel(&root, None);
    for site in ["siteA", "siteB"] {
        write_case(&root, site);
        write_obs(&root.join(format!("{site}-obs.nc")), 264);
    }
    let spec = write_tuning_spec(&root, &kernel, 10);
    let study = run_ok(&[
        "study-create",
        root.to_str().unwrap(),
        "--spec",
        spec.to_str().unwrap(),
    ])
    .trim()
    .to_string();
    let error = run_fail(&["study-apply-preview", &study, "--member", "m000001"]);
    assert!(error.contains("only be applied after"), "{error}");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn failed_tuning_baseline_finishes_retryable_without_running_candidates() {
    let _guard = netcdf_lock();
    let root = temp_root("baseline-retry");
    let kernel = fake_kernel(&root, None);
    fs::write(root.join("fail-baseline"), b"fail").unwrap();
    for site in ["siteA", "siteB"] {
        write_case(&root, site);
        write_obs(&root.join(format!("{site}-obs.nc")), 264);
    }
    let spec = write_tuning_spec(&root, &kernel, 10);
    let study = run_ok(&[
        "study-create",
        root.to_str().unwrap(),
        "--spec",
        spec.to_str().unwrap(),
    ])
    .trim()
    .to_string();
    let failed: Value = serde_json::from_str(&run_ok(&[
        "study-run",
        &study,
        "--kernel",
        kernel.to_str().unwrap(),
        "--jobs",
        "2",
        "--preprocessors",
        "fortran",
    ]))
    .unwrap();
    assert_eq!(failed["status"], "completed_with_failures");
    assert!(failed["tasks"]
        .as_object()
        .unwrap()
        .iter()
        .any(|(id, task)| { id.starts_with("m000001/") && task["status"] == "materialized" }));

    fs::remove_file(root.join("fail-baseline")).unwrap();
    run_ok(&["study-retry", &study]);
    let recovered: Value = serde_json::from_str(&run_ok(&[
        "study-run",
        &study,
        "--kernel",
        kernel.to_str().unwrap(),
        "--jobs",
        "2",
        "--preprocessors",
        "fortran",
    ]))
    .unwrap();
    assert_eq!(recovered["status"], "completed");
    fs::remove_dir_all(root).unwrap();
}

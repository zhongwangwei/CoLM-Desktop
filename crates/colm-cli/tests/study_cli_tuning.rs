use std::fs;
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
    let root = std::env::temp_dir().join(format!("colm-cli-{name}-{}-{nanos}", std::process::id()));
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
case "$program" in
  mksrfdata*) mkdir -p "$out/landdata"; : > "$out/landdata/srfdata.nc"; echo 'Successful in surface data making.' ;;
  mkinidata*) mkdir -p "$out/restart/const"; : > "$out/restart/const/${{case_name}}_restart_const_${{lc}}_w180_s90.nc"; : > "$out/restart/const/${{case_name}}_restart_const_${{lc}}.nc"; echo 'CoLM Initialization Execution Completed' ;;
  colm*) if [ -f '{fail_marker}' ] && printf '%s' "$case_name" | grep -q '^m000000-'; then echo 'baseline forced failure' >&2; exit 7; fi; mkdir -p "$out/history"; cp '{}' "$out/history/${{case_name}}_hist_2008-01.nc"; echo 'TIMESTEP = 1 | DATE = 2008-01-01'; echo 'CoLM Execution Completed.' ;;
esac
"#,
        golden.display(),
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
    ]))
    .unwrap();
    assert_eq!(recovered["status"], "completed");
    fs::remove_dir_all(root).unwrap();
}

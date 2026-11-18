#![cfg(unix)]

use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[test]
fn operator_stop_over_fake_ssh_preserves_identity_and_rechecks_real_result() {
    let temp = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("colm-operator-stop-{}", std::process::id()));
    std::fs::create_dir_all(&temp).unwrap();
    let root = temp.join("server");
    let case = temp.join("case");
    let job = root.join("jobs/station");
    let pool = root.join("pool");
    for dir in [
        &case,
        &job,
        &pool.join("claimed/station"),
        &pool.join("carriers/c0/done"),
    ] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let ssh = temp.join("ssh");
    std::fs::write(&ssh, "#!/bin/bash\nexec /bin/bash -s\n").unwrap();
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        case.join("case.nml"),
        "&nl_colm DEF_CASE_NAME='station' /\n",
    )
    .unwrap();
    let record = json!({"host":"fake-server","root":root,"job":"station","remote_case":root.join("cases/station"),"case_name":"station","engine":"engine-one","submitted_at":1,"scheduler":"bare-pack","scheduler_id":"station"});
    std::fs::write(
        case.join(".colm-remote.json"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    std::fs::write(
        job.join("script.sh"),
        "# colm-engine-snapshot: engine-one\n",
    )
    .unwrap();
    std::fs::write(job.join("scheduler"), "bare-pack").unwrap();
    std::fs::write(job.join("sched_id"), "station").unwrap();
    std::fs::write(job.join("pool"), pool.to_str().unwrap()).unwrap();
    std::fs::write(pool.join("claimed/station/carrier"), "2147483647").unwrap();
    std::fs::write(pool.join("carriers/c0/sched_id"), "2147483647").unwrap();
    let call = || {
        Command::new(env!("CARGO_BIN_EXE_colm-cli"))
            .args([
                "remote-confirm-stopped",
                case.to_str().unwrap(),
                "--job",
                "station",
                "--confirmed-stopped",
                "1",
                "--history-db",
                temp.join("history.sqlite").to_str().unwrap(),
            ])
            .env("PATH", format!("{}:/usr/bin:/bin", temp.display()))
            .env_remove("COLM_REMOTE_ENV_SCRIPT")
            .output()
            .unwrap()
    };
    std::fs::write(job.join("sched_id"), "replaced").unwrap();
    let wrong = call();
    assert!(!wrong.status.success());
    assert!(String::from_utf8_lossy(&wrong.stderr).contains("scheduler job identity changed"));
    assert!(!job.join("operator_stop").exists());
    assert!(!root.join(".engine-lifecycle-lock").exists());
    std::fs::write(job.join("sched_id"), "station").unwrap();
    let stopped = call();
    assert!(
        stopped.status.success(),
        "{}",
        String::from_utf8_lossy(&stopped.stderr)
    );
    let value: Value = serde_json::from_slice(&stopped.stdout).unwrap();
    assert_eq!(value["ended"], "cancelled");
    assert_eq!(value["operator_confirmed_stopped"], true);
    assert!(!job.join("exit_code").exists());
    let audit = std::fs::read_to_string(job.join("operator_stop")).unwrap();
    assert!(call().status.success());
    assert_eq!(
        std::fs::read_to_string(job.join("operator_stop")).unwrap(),
        audit
    );
    std::fs::write(job.join("exit_code"), "0").unwrap();
    let real = call();
    assert!(
        real.status.success(),
        "{}",
        String::from_utf8_lossy(&real.stderr)
    );
    let value: Value = serde_json::from_slice(&real.stdout).unwrap();
    assert_eq!(value["status"]["exit_code"], 0);
    assert_eq!(value["operator_confirmed_stopped"], false);
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(case.join(".colm-remote.json")).unwrap())
            .unwrap(),
        record
    );
    assert!(!root.join(".engine-lifecycle-lock").exists());
    std::fs::remove_dir_all(temp).unwrap();
}

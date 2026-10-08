use super::*;

#[test]
fn job_scripts_detach_record_and_report() {
    let script = submit_script("/data/colm/", "j-1", "echo hi > out.txt");
    assert!(script.contains("mkdir -p '/data/colm/jobs/j-1'"));
    assert!(script.contains("setsid nohup ./script.sh > log 2>&1 < /dev/null &"));
    assert!(script.contains("echo hi > out.txt") && script.contains("echo $? > exit_code"));
    assert!(status_script("/data/colm", "j-1", 40).contains("tail -n 40 log"));
    assert!(cancel_script("/data/colm", "j-1").contains("kill -TERM -- -\"$P\""));
    assert!(check_id("20261008-ca-qfo_1").is_ok());
    for bad in ["", "../x", "a/b", "x y"] {
        assert!(check_id(bad).is_err(), "{bad}");
    }
}

#[test]
fn status_output_is_parsed() {
    let running = parse_status(
        "state=running\nphase=building the Rust engine\n---log---\nCompiling colm-core\n",
    );
    assert_eq!(running.state, State::Running);
    assert_eq!(running.phase.as_deref(), Some("building the Rust engine"));
    assert_eq!(running.log_tail, "Compiling colm-core\n");
    let done = parse_status("state=finished 0\nphase=\n---log---\n");
    assert_eq!(done.state, State::Finished { exit_code: 0 });
    assert_eq!(done.phase, None);
    assert_eq!(parse_status("state=lost\n---log---\n").state, State::Lost);
    assert_eq!(parse_status("state=unknown\n").state, State::Unknown);
    let json = serde_json::to_value(&done).unwrap();
    assert_eq!(json["state"], "finished");
    assert_eq!(json["exit_code"], 0);
}

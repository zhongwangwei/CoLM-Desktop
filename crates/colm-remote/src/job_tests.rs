use std::io::Write;
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use super::*;

fn slurm_spec() -> Spec {
    Spec {
        scheduler: Scheduler::Slurm,
        resources: Resources {
            cpus: 8,
            ranks: 0,
            nodes: 0,
            cores_per_node: 0,
            memory_gb: Some(32),
            walltime: Some("02:00:00".into()),
            partition: Some("cpu".into()),
            account: None,
            env_script: Some("module load rust".into()),
            directives: Vec::new(),
        },
        name: "colm_t".into(),
    }
}

#[test]
fn job_scripts_detach_record_and_report() {
    let script = submit_script("/data/colm/", "j-1", "echo hi > out.txt", &Spec::bare()).unwrap();
    assert!(script.contains("mkdir -p '/data/colm/jobs'"));
    assert!(script.contains("mkdir '/data/colm/jobs/j-1' ||"));
    assert!(script.contains("setsid nohup ./script.sh > log 2>&1 < /dev/null &"));
    assert!(script.contains("echo hi > out.txt") && script.contains("echo $? > exit_code"));
    assert!(!script.contains("#SBATCH"));
    assert!(status_script("/data/colm", "j-1", 40).contains("tail -n 40 log"));
    assert!(cancel_script("/data/colm", "j-1").contains("kill -TERM -- -\"$P\""));
    assert!(check_id("20261008-ca-qfo_1").is_ok());
    for bad in ["", "../x", "a/b", "x y"] {
        assert!(check_id(bad).is_err(), "{bad}");
        assert!(job_script("/r", bad, "true", &Spec::bare()).is_err());
    }
}

#[test]
fn scheduler_job_script_has_directives_environment_then_body() {
    let script = job_script("/r", "j-1", "echo body", &slurm_spec()).unwrap();
    let at = |needle: &str| {
        script
            .find(needle)
            .unwrap_or_else(|| panic!("{needle}\n{script}"))
    };
    assert!(script.starts_with("#!/bin/bash\n#SBATCH --job-name=colm_t\n"));
    assert!(at("#SBATCH --time=02:00:00") < at("cd '/r/jobs/j-1'"));
    assert!(at("cd '/r/jobs/j-1'") < at("module load rust"));
    assert!(at("module load rust") < at("echo body"));
    assert!(at("echo body") < at("echo $? > exit_code"));
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
    let queued = parse_status("state=queued\ndetail=PENDING\nphase=\n---log---\n");
    assert_eq!(queued.state, State::Queued);
    assert_eq!(queued.detail.as_deref(), Some("PENDING"));
    let json = serde_json::to_value(&done).unwrap();
    assert_eq!(json["state"], "finished");
    assert_eq!(json["exit_code"], 0);
    assert!(json.get("detail").is_none());
    assert_eq!(serde_json::to_value(&queued).unwrap()["state"], "queued");
}

// ---- 用模拟的调度命令真正执行生成的脚本（bash 与 sed 在 macOS 与 Linux 上都有；调度命令的输出格式照各家文档） ----

// 沙箱用 bash 与 POSIX 工具扮演调度器。Windows 上 `bash` 会落到没装发行版的 WSL，
// 真正提交、轮询作业的测试只在 Unix 上跑。
struct Sandbox {
    root: PathBuf,
    bin: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("colm-remote-sched-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let sandbox = Self { root, bin };
        // Slurm
        sandbox.stub(
            "sbatch",
            "echo \"$@\" > \"$STUBS/sbatch_args\"\necho '4242;cluster1'",
        );
        sandbox.stub("squeue", "cat \"$STUBS/slurm_state\" 2>/dev/null; true");
        sandbox.stub("sacct", "cat \"$STUBS/sacct_out\" 2>/dev/null; true");
        sandbox.stub("scancel", "echo \"$@\" >> \"$STUBS/cancelled\"");
        // PBS
        sandbox.stub(
            "qsub",
            "echo \"$@\" > \"$STUBS/qsub_args\"\necho '777.pbsserver'",
        );
        sandbox.stub(
            "qstat",
            "[ -f \"$STUBS/pbs_state\" ] && echo \"Job Id: 777.pbsserver\" && echo \"    job_state = $(cat \"$STUBS/pbs_state\")\"; true",
        );
        sandbox.stub("qdel", "echo \"$@\" >> \"$STUBS/cancelled\"");
        // LSF
        sandbox.stub(
            "bsub",
            "cat > \"$STUBS/bsub_script\"\necho 'Job <555> is submitted to queue <normal>.'",
        );
        sandbox.stub("bjobs", "cat \"$STUBS/lsf_state\" 2>/dev/null; true");
        sandbox.stub("bkill", "echo \"$@\" >> \"$STUBS/cancelled\"");
        sandbox
    }

    fn stub(&self, name: &str, body: &str) {
        let path = self.bin.join(name);
        std::fs::write(&path, format!("#!/bin/bash\n{body}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[cfg(unix)]
    fn set(&self, file: &str, value: &str) {
        std::fs::write(self.bin.join(file), value).unwrap();
    }

    #[cfg(unix)]
    fn unset(&self, file: &str) {
        let _ = std::fs::remove_file(self.bin.join(file));
    }

    #[cfg(unix)]
    fn read(&self, file: &str) -> String {
        std::fs::read_to_string(self.bin.join(file)).unwrap_or_default()
    }

    fn work(&self) -> String {
        self.root.join("work").to_string_lossy().into_owned()
    }

    /// 和 `Ssh::run` 一样把脚本经 stdin 交给 `bash -s`。
    fn run(&self, script: &str) -> (String, String, bool) {
        let mut child = Command::new("bash")
            .arg("-s")
            .env("STUBS", &self.bin)
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", self.bin.display()),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(script.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.success(),
        )
    }

    #[cfg(unix)]
    fn status(&self, id: &str) -> Status {
        let (out, err, ok) = self.run(&status_script(&self.work(), id, 5));
        assert!(ok, "{err}");
        parse_status(&out)
    }

    #[cfg(unix)]
    fn job_dir(&self, id: &str) -> PathBuf {
        Path::new(&job_dir(&self.work(), id)).to_path_buf()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[cfg(unix)]
#[test]
fn slurm_job_goes_queued_running_finished() {
    let sb = Sandbox::new("slurm");
    let spec = slurm_spec();
    let (out, err, ok) =
        sb.run(&submit_script(&sb.work(), "j-1", "echo run; echo going > phase", &spec).unwrap());
    assert!(ok, "{err}");
    assert_eq!(
        out.trim().lines().last(),
        Some("4242"),
        "the cluster suffix is dropped"
    );
    let dir = sb.job_dir("j-1");
    assert_eq!(
        std::fs::read_to_string(dir.join("sched_id"))
            .unwrap()
            .trim(),
        "4242"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("scheduler"))
            .unwrap()
            .trim(),
        "slurm"
    );
    assert!(sb.read("sbatch_args").contains("--parsable script.sh"));

    sb.set("slurm_state", "PENDING\n");
    let queued = sb.status("j-1");
    assert_eq!(queued.state, State::Queued);
    assert_eq!(queued.detail.as_deref(), Some("PENDING"));

    sb.set("slurm_state", "RUNNING\n");
    assert_eq!(sb.status("j-1").state, State::Running);

    // 调度器真正执行作业脚本：它必须在作业目录之外也能跑（sbatch 会拷贝脚本）。
    let run = Command::new("bash")
        .arg(dir.join("script.sh"))
        .env("PATH", "/usr/bin:/bin")
        .current_dir("/")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    sb.unset("slurm_state");
    let done = sb.status("j-1");
    assert_eq!(done.state, State::Finished { exit_code: 0 });
    assert_eq!(done.phase.as_deref(), Some("going"));
}

#[cfg(unix)]
#[test]
fn retrying_a_reserved_job_never_launches_it_twice() {
    let sb = Sandbox::new("duplicate-submit");
    let spec = slurm_spec();
    let script = submit_script(&sb.work(), "once", "echo run", &spec).unwrap();
    let (_, error, success) = sb.run(&script);
    assert!(success, "{error}");
    let original = std::fs::read(sb.job_dir("once").join("script.sh")).unwrap();
    let (_, error, success) =
        sb.run(&submit_script(&sb.work(), "once", "echo changed", &spec).unwrap());
    assert!(!success);
    assert!(error.contains("job identity already exists"));
    assert_eq!(
        std::fs::read(sb.job_dir("once").join("script.sh")).unwrap(),
        original
    );
}

#[cfg(unix)]
#[test]
fn slurm_job_the_scheduler_killed_is_lost_with_its_reason() {
    let sb = Sandbox::new("slurm-lost");
    let spec = slurm_spec();
    assert!(
        sb.run(&submit_script(&sb.work(), "j-2", "true", &spec).unwrap())
            .2
    );
    sb.unset("slurm_state");
    sb.set("sacct_out", "TIMEOUT|0:0\n");
    let lost = sb.status("j-2");
    assert_eq!(lost.state, State::Lost);
    assert_eq!(lost.detail.as_deref(), Some("TIMEOUT|0:0"));
}

#[cfg(unix)]
#[test]
fn slurm_cancel_acceptance_does_not_forge_a_terminal_state() {
    let sb = Sandbox::new("slurm-cancel");
    let spec = slurm_spec();
    assert!(
        sb.run(&submit_script(&sb.work(), "j-3", "sleep 100", &spec).unwrap())
            .2
    );
    sb.set("slurm_state", "PENDING\n");
    let (out, err, ok) = sb.run(&cancel_script(&sb.work(), "j-3"));
    assert!(ok, "{err}");
    assert!(out.contains("cancellation_requested"));
    assert_eq!(sb.read("cancelled").trim(), "4242");
    assert!(!sb.job_dir("j-3").join("exit_code").exists());
    assert_eq!(sb.status("j-3").state, State::Queued);
    sb.set("slurm_state", "RUNNING\n");
    assert_eq!(sb.status("j-3").state, State::Running);
    sb.unset("slurm_state");
    assert_eq!(sb.status("j-3").state, State::Lost);
}

#[cfg(unix)]
#[test]
fn a_failed_submission_is_reported_not_swallowed() {
    let sb = Sandbox::new("slurm-fail");
    sb.stub(
        "sbatch",
        "echo 'sbatch: error: invalid partition specified: nope' >&2; exit 1",
    );
    let (_, err, ok) = sb.run(&submit_script(&sb.work(), "j-4", "true", &slurm_spec()).unwrap());
    assert!(!ok);
    assert!(
        err.contains("sbatch failed") && err.contains("invalid partition"),
        "{err}"
    );
}

#[cfg(unix)]
#[test]
fn a_job_slurm_accepted_despite_an_sbatch_error_is_found_by_its_directory() {
    let sb = Sandbox::new("slurm-garbled");
    sb.stub(
        "sbatch",
        "echo 'sbatch: error: Batch job submission failed: Unexpected message received' >&2; exit 1",
    );
    // squeue lists another job of the user too; only the one in this job's directory counts.
    sb.stub(
        "squeue",
        "echo \"8675098|/elsewhere\"; echo \"8675099|$(pwd -P)\"",
    );
    let (out, err, ok) = sb.run(&submit_script(&sb.work(), "j-6", "true", &slurm_spec()).unwrap());
    assert!(ok, "{err}");
    assert_eq!(out.trim().lines().last(), Some("8675099"));
    assert!(sb.job_dir("j-6").join("sched_id").exists());
    assert!(err.contains("accepted (8675099)"), "{err}");

    // A job that already finished is only in sacct.
    let sb = Sandbox::new("slurm-garbled-done");
    sb.stub("sbatch", "echo 'Unexpected message received' >&2; exit 1");
    sb.stub("sacct", "echo \"8675101|$(pwd -P)\"");
    let (out, err, ok) = sb.run(&submit_script(&sb.work(), "j-7", "true", &slurm_spec()).unwrap());
    assert!(ok, "{err}");
    assert_eq!(out.trim().lines().last(), Some("8675101"));

    // Two candidates are not guessed between.
    let sb = Sandbox::new("slurm-garbled-two");
    sb.stub("sbatch", "echo 'Unexpected message received' >&2; exit 1");
    sb.stub("squeue", "echo \"1|$(pwd -P)\"; echo \"2|$(pwd -P)\"");
    let (_, err, ok) = sb.run(&submit_script(&sb.work(), "j-8", "true", &slurm_spec()).unwrap());
    assert!(!ok);
    assert!(err.contains("sbatch failed"), "{err}");
    assert!(!sb.job_dir("j-8").join("sched_id").exists());
}

#[cfg(unix)]
#[test]
fn pbs_job_states_and_cancel() {
    let sb = Sandbox::new("pbs");
    let mut spec = slurm_spec();
    spec.scheduler = Scheduler::Pbs;
    let (out, err, ok) = sb.run(&submit_script(&sb.work(), "j-5", "true", &spec).unwrap());
    assert!(ok, "{err}");
    assert_eq!(out.trim().lines().last(), Some("777.pbsserver"));
    assert!(sb.read("qsub_args").contains("script.sh"));
    let script = std::fs::read_to_string(sb.job_dir("j-5").join("script.sh")).unwrap();
    assert!(script.contains("#PBS -l select=1:ncpus=8:mpiprocs=1:mem=32gb"));

    sb.set("pbs_state", "Q");
    assert_eq!(sb.status("j-5").state, State::Queued);
    sb.set("pbs_state", "R");
    assert_eq!(sb.status("j-5").state, State::Running);
    sb.set("pbs_state", "E");
    assert_eq!(sb.status("j-5").state, State::Running);
    sb.unset("pbs_state");
    assert_eq!(sb.status("j-5").state, State::Lost);

    assert!(sb.run(&cancel_script(&sb.work(), "j-5")).2);
    assert_eq!(sb.read("cancelled").trim(), "777.pbsserver");
    assert!(!sb.job_dir("j-5").join("exit_code").exists());
}

#[cfg(unix)]
#[test]
fn lsf_job_states_and_cancel() {
    let sb = Sandbox::new("lsf");
    let mut spec = slurm_spec();
    spec.scheduler = Scheduler::Lsf;
    let (out, err, ok) = sb.run(&submit_script(&sb.work(), "j-6", "true", &spec).unwrap());
    assert!(ok, "{err}");
    assert_eq!(out.trim().lines().last(), Some("555"));
    assert!(
        sb.read("bsub_script").contains("#BSUB -n 8"),
        "bsub reads the script on stdin"
    );

    sb.set("lsf_state", "PEND\n");
    assert_eq!(sb.status("j-6").state, State::Queued);
    sb.set("lsf_state", "RUN\n");
    assert_eq!(sb.status("j-6").state, State::Running);
    sb.set("lsf_state", "EXIT\n");
    let lost = sb.status("j-6");
    assert_eq!(lost.state, State::Lost);
    assert_eq!(lost.detail.as_deref(), Some("EXIT"));

    assert!(sb.run(&cancel_script(&sb.work(), "j-6")).2);
    assert_eq!(sb.read("cancelled").trim(), "555");
    assert!(!sb.job_dir("j-6").join("exit_code").exists());
}

#[test]
fn submitted_reports_the_right_id_kind() {
    // `submit` 解析最后一行：有调度系统时是字符串作业号，没有时是进程号。
    let bare = Submitted {
        scheduler: "bare",
        pid: Some(12),
        scheduler_id: None,
    };
    assert_eq!(serde_json::to_value(&bare).unwrap()["pid"], 12);
}

#[cfg(unix)]
#[test]
fn cancellation_failures_never_forge_a_terminal_state() {
    for (scheduler, command) in [("slurm", "scancel"), ("pbs", "qdel"), ("lsf", "bkill")] {
        let sb = Sandbox::new(&format!("cancel-fail-{scheduler}"));
        let dir = PathBuf::from(sb.work()).join("jobs/j-fail");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("scheduler"), scheduler).unwrap();
        std::fs::write(dir.join("sched_id"), "123").unwrap();
        sb.stub(command, "echo permission-denied >&2; exit 7");
        let (out, err, ok) = sb.run(&cancel_script(&sb.work(), "j-fail"));
        assert!(!ok && err.contains("permission-denied"), "{out} {err}");
        assert!(!dir.join("exit_code").exists());
        std::fs::write(dir.join("exit_code"), "0").unwrap();
        assert!(sb.run(&cancel_script(&sb.work(), "j-fail")).2);
        assert_eq!(std::fs::read_to_string(dir.join("exit_code")).unwrap(), "0");
    }
}

#[test]
fn cancellation_rejects_unknown_schedulers_and_unsafe_bare_pids() {
    for scheduler in ["unknown", "bare"] {
        let sb = Sandbox::new(&format!("cancel-invalid-{scheduler}"));
        let dir = PathBuf::from(sb.work()).join("jobs/j-invalid");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("scheduler"), scheduler).unwrap();
        std::fs::write(dir.join("pid"), "0").unwrap();
        assert!(!sb.run(&cancel_script(&sb.work(), "j-invalid")).2);
        assert!(!dir.join("exit_code").exists());
    }
}

#[test]
fn engine_markers_accept_source_and_prebuilt_and_refuse_injection() {
    for id in ["0123456789abcdef", "pre-0123456789abcdef"] {
        assert!(engine_body(id, "true")
            .unwrap()
            .starts_with(&format!("# colm-engine-snapshot: {id}\n")));
    }
    for id in [
        "",
        "../engine",
        "pre-0123456789abcdeF",
        "0123456789abcdef\nfalse",
    ] {
        assert!(engine_body(id, "true").is_err());
    }
}

#[cfg(unix)]
#[test]
fn stale_bare_pid_cannot_cancel_an_unrelated_process() {
    let sb = Sandbox::new("stale-bare-pid");
    let dir = sb.job_dir("stale");
    std::fs::create_dir_all(&dir).unwrap();
    // Our test process cannot be this job's session leader with this job's cwd.
    std::fs::write(dir.join("pid"), std::process::id().to_string()).unwrap();
    let (_, error, ok) = sb.run(&format!("set -e\n{}", cancel_script(&sb.work(), "stale")));
    assert!(!ok && error.contains("job process"), "{error}");
    assert!(!dir.join("exit_code").exists());
}

// ---- 按节点打包（crate::pack）：载体脚本真的在沙箱里跑，flock 与 setsid 用替身（macOS 没有） ----

#[cfg(unix)]
fn pack_sandbox(tag: &str, partition_line: &str) -> Sandbox {
    let sb = Sandbox::new(tag);
    sb.stub("scontrol", &format!("echo '{partition_line}'"));
    sb.stub("sinfo", "echo 2");
    sb.stub("flock", "exit 0");
    sb.stub("setsid", "exec \"$@\"");
    // Each submission gets the next id and is pending until a test marks it gone.
    sb.stub(
        "sbatch",
        "n=$(cat \"$STUBS/next\" 2>/dev/null || echo 100); echo $((n + 1)) > \"$STUBS/next\"\necho \"$n\" >> \"$STUBS/live\"; pwd >> \"$STUBS/submitted_from\"; echo \"$n\"",
    );
    sb.stub(
        "squeue",
        "while [ $# -gt 0 ]; do [ \"$1\" = -j ] && id=$2; shift; done\n[ -n \"$id\" ] && grep -qx \"$id\" \"$STUBS/live\" 2>/dev/null && echo RUNNING; true",
    );
    sb
}

#[cfg(unix)]
fn pack_spec() -> (Spec, crate::pack::Pack) {
    (
        slurm_spec(),
        crate::pack::Pack {
            pool: "batch-1".into(),
            slots: 0,
        },
    )
}

#[cfg(unix)]
#[test]
fn an_exclusive_partition_queues_sites_and_adds_a_node_job_per_node_of_sites() {
    let sb = pack_sandbox(
        "pack",
        "PartitionName=cpu AllowGroups=ALL OverSubscribe=EXCLUSIVE State=UP",
    );
    let (spec, pack) = pack_spec();
    let bodies = [
        ("j-a", "true"),
        ("j-b", "exit 3"),
        ("j-c", "sleep 20"),
        ("j-d", "true"),
    ];
    for (id, body) in bodies {
        let script = packed_submit_script(&sb.work(), id, body, &spec, &pack).unwrap();
        let (out, err, ok) = sb.run(&script);
        assert!(ok, "{id}: {err}");
        assert_eq!(out.trim().lines().last(), Some("pack batch-1"), "{id}");
        assert_eq!(
            std::fs::read_to_string(sb.job_dir(id).join("scheduler")).unwrap(),
            "slurm-pack\n"
        );
    }
    // Two slots per node (sinfo): a and b share the first node job, c needs a second; d fits.
    assert_eq!(sb.read("live").lines().collect::<Vec<_>>(), ["100", "101"]);
    let pool = std::path::PathBuf::from(crate::pack::pool_dir(&sb.work(), "batch-1"));
    assert_eq!(
        std::fs::read_to_string(pool.join("slots")).unwrap().trim(),
        "2"
    );
    let carrier = std::fs::read_to_string(pool.join("carriers/c0/script.sh")).unwrap();
    assert!(carrier.contains("#SBATCH --partition=cpu"), "{carrier}");
    assert!(carrier.contains("#SBATCH --cpus-per-task=1"), "{carrier}");
    let queued = sb.status("j-a");
    assert_eq!(queued.state, State::Queued);
    assert!(queued.detail.unwrap().contains("waiting in node pack"));

    // Cancelling a site no node job has taken: nothing will run it, and nothing forges an exit code.
    let (out, err, ok) = sb.run(&cancel_script(&sb.work(), "j-d"));
    assert!(ok, "{err}");
    assert!(out.contains("cancellation_requested"));
    assert!(
        sb.read("cancelled").is_empty(),
        "the node job itself is never scancelled"
    );
    let cancelled = sb.status("j-d");
    assert_eq!(cancelled.state, State::Lost);
    assert_eq!(
        cancelled.detail.as_deref(),
        Some("cancelled before it started")
    );

    // Run the first node job as Slurm would, with two cores.
    let child = Command::new("bash")
        .arg(pool.join("carriers/c0/script.sh"))
        .current_dir(pool.join("carriers/c0"))
        .env("STUBS", &sb.bin)
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", sb.bin.display()),
        )
        .env("SLURM_JOB_ID", "100")
        .env("SLURM_CPUS_ON_NODE", "2")
        .env("COLM_PACK_LINGER", "2")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let wait_for = |what: &str, done: &dyn Fn() -> bool| {
        for _ in 0..200 {
            if done() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("timed out waiting for {what}");
    };
    wait_for("c to start", &|| pool.join("claimed/j-c/carrier").is_file());
    let running = sb.status("j-c");
    assert_eq!(running.state, State::Running);
    assert!(running.detail.unwrap().contains("node pack job 100"));
    // Cancelling a running site stops only that site; the node job goes on and then ends.
    let (_, err, ok) = sb.run(&cancel_script(&sb.work(), "j-c"));
    assert!(ok, "{err}");
    let child = std::cell::RefCell::new(child);
    wait_for("the node job to end", &|| {
        child.borrow_mut().try_wait().unwrap().is_some()
    });
    assert_eq!(sb.status("j-a").state, State::Finished { exit_code: 0 });
    assert_eq!(sb.status("j-b").state, State::Finished { exit_code: 3 });
    assert_eq!(sb.status("j-c").state, State::Finished { exit_code: 143 });
    assert!(
        !pool.join("claimed/j-d/carrier").exists(),
        "a cancelled site never runs"
    );
    assert!(pool.join("carriers/c0/closing").is_file());
    // Every site ran exactly once, in its own directory, with its own log.
    for id in ["j-a", "j-b", "j-c"] {
        assert_eq!(
            std::fs::read_to_string(pool.join(format!("claimed/{id}/carrier"))).unwrap(),
            "100\n"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_shared_partition_keeps_one_job_per_site() {
    let sb = pack_sandbox("pack-shared", "PartitionName=cpu OverSubscribe=NO State=UP");
    let (spec, pack) = pack_spec();
    let script = packed_submit_script(&sb.work(), "j-s", "true", &spec, &pack).unwrap();
    let (out, err, ok) = sb.run(&script);
    assert!(ok, "{err}");
    assert_eq!(out.trim().lines().last(), Some("100"));
    assert_eq!(
        std::fs::read_to_string(sb.job_dir("j-s").join("scheduler")).unwrap(),
        "slurm\n"
    );
    assert_eq!(
        sb.read("submitted_from").trim(),
        sb.job_dir("j-s").to_string_lossy()
    );
}

#[cfg(unix)]
#[test]
fn a_site_whose_node_job_cannot_be_submitted_leaves_the_queue() {
    let sb = pack_sandbox("pack-fail", "PartitionName=cpu OverSubscribe=EXCLUSIVE");
    sb.stub(
        "sbatch",
        "echo 'sbatch: error: invalid account' >&2; exit 1",
    );
    let (spec, pack) = pack_spec();
    let script = packed_submit_script(&sb.work(), "j-f", "true", &spec, &pack).unwrap();
    let (_, err, ok) = sb.run(&script);
    assert!(!ok);
    assert!(
        err.contains("sbatch failed") && err.contains("invalid account"),
        "{err}"
    );
    let pool = std::path::PathBuf::from(crate::pack::pool_dir(&sb.work(), "batch-1"));
    assert!(
        !pool.join("queue/j-f").exists(),
        "no node job would ever run it"
    );
    assert!(!sb.job_dir("j-f").join("sched_id").exists());
}

#[cfg(unix)]
#[test]
fn a_partition_that_hands_out_whole_node_memory_gets_a_fair_share_per_cpu() {
    // land02 (Slurm 19.05, CR_CORE_MEMORY): without --mem a job takes the node's whole memory.
    let line = |extra: &str| format!("PartitionName=cpu Default=YES MaxNodes=3 {extra} State=UP");
    let submit = |tag: &str, partition: &str, memory: Option<u32>| {
        let sb = Sandbox::new(tag);
        sb.stub("scontrol", &format!("echo '{partition}'"));
        sb.stub("sinfo", "printf '320000 96\\n384000 96+\\n'");
        let mut spec = slurm_spec();
        spec.resources.memory_gb = memory;
        let (_, err, ok) = sb.run(&submit_script(&sb.work(), "j-m", "true", &spec).unwrap());
        assert!(ok, "{err}");
        sb.read("sbatch_args")
    };
    let args = submit(
        "mem-unlimited",
        &line("OverSubscribe=NO DefMemPerNode=UNLIMITED"),
        None,
    );
    assert!(
        args.contains("--mem-per-cpu=3333M"),
        "smallest node per CPU: {args}"
    );
    for (tag, partition, memory) in [
        (
            "mem-set",
            line("OverSubscribe=NO DefMemPerNode=UNLIMITED"),
            Some(32),
        ),
        (
            "mem-exclusive",
            line("OverSubscribe=EXCLUSIVE DefMemPerNode=UNLIMITED"),
            None,
        ),
        (
            "mem-default",
            line("OverSubscribe=NO DefMemPerCPU=2000"),
            None,
        ),
    ] {
        let args = submit(tag, &partition, memory);
        assert!(!args.contains("--mem-per-cpu"), "{tag}: {args}");
    }
}

#[test]
fn a_job_the_scheduler_ended_without_an_exit_code_is_over() {
    let lost = |detail: Option<&str>| Status {
        state: State::Lost,
        phase: None,
        detail: detail.map(str::to_owned),
        log_tail: String::new(),
    };
    // land02: queued jobs cancelled before they started (sacct `CANCELLED by 1026|0:0`).
    assert_eq!(
        ended(&lost(Some("CANCELLED by 1026|0:0"))),
        Some("cancelled")
    );
    assert_eq!(
        ended(&lost(Some("cancelled before it started"))),
        Some("cancelled")
    );
    assert_eq!(ended(&lost(Some("TIMEOUT|0:0"))), Some("failed"));
    assert_eq!(
        ended(&lost(Some("node pack job 100 OUT_OF_MEMORY|0:125"))),
        Some("failed")
    );
    // Unclear endings stay lost: a COMPLETED job without an exit code, or no record at all.
    assert_eq!(ended(&lost(Some("COMPLETED|0:0"))), None);
    assert_eq!(ended(&lost(None)), None);
    let finished = Status {
        state: State::Finished { exit_code: 0 },
        ..lost(Some("CANCELLED"))
    };
    assert_eq!(ended(&finished), None, "an exit code decides by itself");
}

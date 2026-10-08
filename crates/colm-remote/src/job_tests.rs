use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::*;

fn slurm_spec() -> Spec {
    Spec {
        scheduler: Scheduler::Slurm,
        resources: Resources {
            cpus: 8,
            ranks: 0,
            nodes: 0,
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
    assert!(script.contains("mkdir -p '/data/colm/jobs/j-1'"));
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

    fn set(&self, file: &str, value: &str) {
        std::fs::write(self.bin.join(file), value).unwrap();
    }

    fn unset(&self, file: &str) {
        let _ = std::fs::remove_file(self.bin.join(file));
    }

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

    fn status(&self, id: &str) -> Status {
        let (out, err, ok) = self.run(&status_script(&self.work(), id, 5));
        assert!(ok, "{err}");
        parse_status(&out)
    }

    fn job_dir(&self, id: &str) -> PathBuf {
        Path::new(&job_dir(&self.work(), id)).to_path_buf()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

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

#[test]
fn slurm_cancel_asks_the_scheduler_and_leaves_an_exit_code() {
    let sb = Sandbox::new("slurm-cancel");
    let spec = slurm_spec();
    assert!(
        sb.run(&submit_script(&sb.work(), "j-3", "sleep 100", &spec).unwrap())
            .2
    );
    sb.set("slurm_state", "PENDING\n");
    let (out, err, ok) = sb.run(&cancel_script(&sb.work(), "j-3"));
    assert!(ok, "{err}");
    assert!(out.contains("cancelled"));
    assert_eq!(sb.read("cancelled").trim(), "4242");
    sb.unset("slurm_state");
    assert_eq!(sb.status("j-3").state, State::Finished { exit_code: 143 });
}

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
}

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

use super::*;

/// 第 637 轮在 T7920 上探到的结果（`cargo` 等的路径照实）。
#[test]
fn probe_output_is_parsed_and_gaps_are_named() {
    let text = "hostname=T7920\nos=Ubuntu 26.04.1 LTS\narch=x86_64\ncpus=96\nmemory_kb=790000000\n\
cargo=cargo 1.95.0 (f2d3ce0bd 2026-03-21)\ngfortran=GNU Fortran 15.2.0\nmpi=\ncmake=/usr/bin/cmake\ncc=/usr/bin/cc\n\
slurm=\npbs=\nlsf=\nqueues=\nroot_exists=\nroot_writable=1\nroot_free_kb=27917287424\n";
    let probe = parse("/media/zhwei/data02/zhwei/colm-desktop", text);
    assert_eq!(probe.hostname, "T7920");
    assert_eq!(probe.cpus, Some(96));
    assert_eq!(probe.memory_gb, Some(753));
    assert_eq!(
        probe.cargo.as_deref(),
        Some("cargo 1.95.0 (f2d3ce0bd 2026-03-21)")
    );
    assert!(probe.mpi.is_none() && probe.schedulers.is_empty());
    assert!(!probe.root_exists && probe.root_writable);
    assert_eq!(probe.root_free_gb, Some(26624));
    assert!(probe.problems.is_empty(), "{:?}", probe.problems);

    let bare = parse(
        "/r",
        "os=Ubuntu\narch=x86_64\nslurm=/usr/bin/sbatch\nroot_writable=\n",
    );
    assert_eq!(bare.schedulers, ["slurm"]);
    // 缺 cargo、cmake 与编译器不再算“问题”（有预编包就能用），记在 build_problems 里。
    assert_eq!(bare.problems.len(), 1, "{:?}", bare.problems);
    assert_eq!(bare.build_problems.len(), 2, "{:?}", bare.build_problems);
    assert!(probe.build_problems.is_empty());
    assert!(script("/r").contains("R='/r'"));
    assert!(probe.queues.is_empty());
    let queues = parse(
        "/r",
        "os=Ubuntu\narch=x86_64\nslurm=/usr/bin/sbatch\nqueues=cpu gpu long \n",
    );
    assert_eq!(queues.queues, ["cpu", "gpu", "long"]);
}

#[test]
fn slurm_partitions_carry_their_node_sizes_and_limits() {
    // land02 as it answered on 2026-10-19 (Slurm 19.05; first is the default).
    let text = "os=Ubuntu\narch=x86_64\nslurm=/usr/bin/sbatch\n\
partition=first|1|0|3|7-00:00:00|1\npartition=largemem|0|0|1|7-00:00:00|1\n\
partition=second|0|0|1|7-00:00:00|1\npartition=deimos|0|1|UNLIMITED|UNLIMITED|0\n\
partnode=first|4|12|320000\npartnode=first|4|12|384000\npartnode=largemem|4|12|1548000\n\
partnode=second|2|32|515000\npartnode=second|4|20|1031000\npartnode=second|4|20|1031000+\n\
partnode=ghost|1|1|1\npartnode=second|x|20|1\n";
    let probe = parse("/r", text);
    let by = |name: &str| probe.partitions.iter().find(|p| p.name == name).unwrap();
    let first = by("first");
    assert!(first.default && !first.exclusive && first.whole_node_memory);
    assert_eq!((first.nodes, first.cores_min, first.cores_max), (2, 48, 48));
    assert_eq!((first.memory_gb_min, first.memory_gb_max), (312, 375));
    assert_eq!(
        (first.max_nodes, first.max_time.as_deref()),
        (Some(3), Some("7-00:00:00"))
    );
    let second = by("second");
    assert_eq!(
        (second.nodes, second.cores_min, second.cores_max),
        (3, 64, 80)
    );
    assert_eq!(second.memory_gb_max, 1006);
    assert_eq!(by("largemem").memory_gb_min, 1511);
    let deimos = by("deimos");
    assert!(deimos.exclusive && !deimos.default);
    assert_eq!(
        (deimos.max_nodes, deimos.max_time.clone(), deimos.nodes),
        (None, None, 0)
    );
    assert_eq!(
        probe.partitions.len(),
        4,
        "nodes of unknown partitions are ignored"
    );
}

#[test]
fn connection_only_probe_does_not_require_a_work_directory() {
    let probe = parse("", "hostname=server\nos=Linux\narch=x86_64\n");
    assert_eq!(probe.hostname, "server");
    assert!(probe.problems.is_empty());
    assert!(script("").contains("if [ -n \"$R\" ]; then\necho \"root_exists="));
}

#[test]
fn missing_build_tools_can_be_installable_without_becoming_existing_tools() {
    let probe = parse(
        "",
        "hostname=server\nos=Linux\narch=x86_64\ncc=/usr/bin/cc\ntoolchain_installable=1\n",
    );
    assert!(probe.toolchain_installable);
    assert!(probe.cargo.is_none() && !probe.cmake);
    assert!(!probe.build_problems.is_empty());
}

#[test]
fn path_probe_quotes_special_characters_and_uses_numeric_keys() {
    let command = paths_script("/data/root", &["/data/it's here\n$(touch bad)".into()]);
    assert!(command.contains("test -e '/data/it'\\''s here\n$(touch bad)'"));
    assert!(command.contains("echo map_0=1"));
    assert!(command.contains("echo map_0=0"));
}

#[cfg(unix)]
mod shell {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Sandbox {
        root: PathBuf,
        bin: PathBuf,
    }

    impl Sandbox {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "colm-probe-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let bin = root.join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            let sandbox = Self { root, bin };
            // An isolated PATH prevents installed schedulers or login profiles leaking into fixtures.
            for tool in ["awk", "grep", "head", "hostname", "uname", "tr", "ls"] {
                let path = ["/usr/bin", "/bin"]
                    .into_iter()
                    .map(|dir| PathBuf::from(dir).join(tool))
                    .find(|path| path.is_file())
                    .unwrap();
                std::os::unix::fs::symlink(path, sandbox.bin.join(tool)).unwrap();
            }
            sandbox.stub("cargo", "exit 0");
            sandbox.stub("rustc", "exit 0");
            sandbox.stub("nproc", "echo 4");
            sandbox.stub("qsub", "echo qsub >> \"$STUBS/submitted\"; exit 99");
            // Emulate the bounded timeout command on macOS as well as Linux.
            sandbox.stub(
                "timeout",
                r#"[ "$1 $2 $3" = '-k 1 3' ] || exit 98
shift 3
"$@" & command_pid=$!
(/bin/sleep 3; kill -KILL "$command_pid" 2>/dev/null) >/dev/null 2>&1 & watchdog=$!
wait "$command_pid" 2>/dev/null; result=$?
kill "$watchdog" 2>/dev/null
wait "$watchdog" 2>/dev/null
exit "$result""#,
            );
            sandbox
        }

        fn stub(&self, name: &str, body: &str) {
            let path = self.bin.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        fn scheduler(&self, name: &str) {
            self.stub(
                name,
                &format!("echo {name} >> \"$STUBS/submitted\"; exit 99"),
            );
        }

        fn probe(&self) -> Probe {
            let output = Command::new("/bin/sh")
                .args(["-c", &script("")])
                .env("PATH", &self.bin)
                .env("HOME", &self.root)
                .env_remove("CARGO_HOME")
                .env_remove("RUSTUP_HOME")
                .env("STUBS", &self.root)
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            assert!(
                !self.root.join("submitted").exists(),
                "probe submitted a job"
            );
            parse("", &String::from_utf8(output.stdout).unwrap())
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn user_installed_build_tools_are_visible_without_a_login_shell() {
        let sandbox = Sandbox::new();
        sandbox.scheduler("sbatch");
        sandbox.stub("cargo", "echo 'cargo user-toolchain'");
        sandbox.stub("cmake", "echo 'cmake version 3.28.0'");
        sandbox.stub("cc", "exit 0");
        for (tool, directory) in [
            ("cargo", ".cargo/bin"),
            ("rustc", ".cargo/bin"),
            ("cmake", ".local/bin"),
        ] {
            let directory = sandbox.root.join(directory);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::rename(sandbox.bin.join(tool), directory.join(tool)).unwrap();
        }
        let probe = sandbox.probe();
        assert_eq!(probe.cargo.as_deref(), Some("cargo user-toolchain"));
        assert!(probe.cmake && probe.cc);
        assert!(
            probe.build_problems.is_empty(),
            "{:?}",
            probe.build_problems
        );
    }

    #[test]
    fn explicit_build_toolchain_stays_ahead_of_user_fallbacks() {
        let sandbox = Sandbox::new();
        sandbox.scheduler("sbatch");
        sandbox.stub("cargo", "echo 'cargo selected-toolchain'");
        sandbox.stub("cmake", "echo 'cmake version 3.28.0'");
        sandbox.stub("cc", "exit 0");
        let fallback = sandbox.root.join(".cargo/bin");
        std::fs::create_dir_all(&fallback).unwrap();
        let executable = fallback.join("cargo");
        std::fs::write(&executable, "#!/bin/sh\necho 'cargo fallback-toolchain'\n").unwrap();
        std::fs::set_permissions(executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        let probe = sandbox.probe();
        assert_eq!(probe.cargo.as_deref(), Some("cargo selected-toolchain"));
        assert!(probe.build_problems.is_empty());
    }

    #[test]
    fn a_system_cmake_too_old_for_hdf5_is_not_a_usable_build_tool() {
        let sandbox = Sandbox::new();
        sandbox.scheduler("bsub");
        sandbox.stub("cargo", "echo 'cargo 1.97.1'");
        sandbox.stub("cmake", "echo 'cmake version 3.11.4'");
        sandbox.stub("cc", "exit 0");
        let probe = sandbox.probe();
        assert!(!probe.cmake, "CMake 3.11 cannot build the HDF5 sources");
        assert!(probe.build_problems.iter().any(|p| p.contains("3.26")));
    }

    #[test]
    fn lsf_and_slurm_compatibility_wrappers_are_not_pbs() {
        for (submit, expected) in [("bsub", "lsf"), ("sbatch", "slurm")] {
            let sandbox = Sandbox::new();
            sandbox.scheduler(submit);
            sandbox.stub(
                "qstat",
                "echo \"$1\" >> \"$STUBS/qstat_calls\"; echo 'TORQUE; type=BATCH; [ENABLED]'; echo '123 user job-named-OpenPBS'; echo 'a job named PBS Professional'; exit 1",
            );
            sandbox.stub("sinfo", "echo 'cpu*'");
            sandbox.stub("bqueues", "echo QUEUE_NAME; echo normal");
            let probe = sandbox.probe();
            assert_eq!(probe.schedulers, [expected]);
            assert!(probe.qsub_unverified);
            let calls = std::fs::read_to_string(sandbox.root.join("qstat_calls")).unwrap();
            assert_eq!(calls, "--version\n-Bf\n");
        }
    }

    #[test]
    fn explicit_pbs_client_identity_does_not_need_a_live_server() {
        for version in [
            "pbs_version = 23.06.06",
            "OpenPBS 23.06.06",
            "PBS Professional 2024.1",
            "Version: 6.1.3 TORQUE",
        ] {
            let sandbox = Sandbox::new();
            sandbox.stub("qstat", &format!(
                "echo \"$1\" >> \"$STUBS/qstat_calls\"\n[ \"$1\" = --version ] && echo '{version}' && exit 0\nexit 1"
            ));
            let probe = sandbox.probe();
            assert_eq!(probe.schedulers, ["pbs"], "{version}");
            assert!(!probe.qsub_unverified);
            assert!(probe.queues.is_empty());
            assert_eq!(
                std::fs::read_to_string(sandbox.root.join("qstat_calls")).unwrap(),
                "--version\n-Q\n"
            );
        }
    }

    #[test]
    fn pbs_server_identity_and_real_mixed_schedulers_are_preserved() {
        let sandbox = Sandbox::new();
        sandbox.scheduler("bsub");
        sandbox.stub(
            "qstat",
            r#"case "$1" in
--version) exit 1;;
-Bf) echo 'Server: head'; echo '    pbs_version = 23.06.06';;
-Q) echo Queue; echo -----; echo 'workq unlimited';;
esac"#,
        );
        sandbox.stub("bqueues", "echo QUEUE_NAME; echo normal");
        let probe = sandbox.probe();
        assert_eq!(probe.schedulers, ["pbs", "lsf"]);
        assert_eq!(probe.queues, ["workq", "normal"]);
        assert!(!probe.qsub_unverified);
    }

    #[test]
    fn unknown_or_hung_qstat_stays_unverified() {
        for body in ["echo 'not_pbs_version = 1'; exit 1", "while :; do :; done"] {
            let sandbox = Sandbox::new();
            sandbox.stub("qstat", body);
            let probe = sandbox.probe();
            assert!(probe.schedulers.is_empty());
            assert!(probe.qsub_unverified);
            assert!(probe.queues.is_empty());
        }
    }

    #[test]
    fn without_timeout_or_qstat_no_unbounded_identity_query_runs() {
        let sandbox = Sandbox::new();
        assert!(sandbox.probe().qsub_unverified);
        sandbox.stub(
            "qstat",
            "echo queried > \"$STUBS/queried\"; while :; do :; done",
        );
        std::fs::remove_file(sandbox.bin.join("timeout")).unwrap();
        let probe = sandbox.probe();
        assert!(probe.qsub_unverified);
        assert!(probe.schedulers.is_empty());
        assert!(!sandbox.root.join("queried").exists());
    }
}

use super::*;

fn resources() -> Resources {
    Resources {
        cpus: 16,
        ranks: 0,
        nodes: 0,
        cores_per_node: 0,
        memory_gb: Some(64),
        walltime: Some("1-02:30:00".into()),
        partition: Some("cpu".into()),
        account: Some("proj_a".into()),
        env_script: None,
        directives: vec!["--constraint=ib".into()],
    }
}

#[test]
fn schedulers_parse_by_name() {
    assert_eq!(Scheduler::parse("Slurm").unwrap(), Scheduler::Slurm);
    assert_eq!(Scheduler::parse("none").unwrap(), Scheduler::Bare);
    assert_eq!(Scheduler::parse("").unwrap(), Scheduler::Bare);
    assert!(Scheduler::parse("condor").is_err());
    for s in [
        Scheduler::Bare,
        Scheduler::Slurm,
        Scheduler::Pbs,
        Scheduler::Lsf,
    ] {
        assert_eq!(Scheduler::parse(s.name()).unwrap(), s);
    }
}

#[test]
fn walltime_forms_are_understood() {
    assert_eq!(parse_walltime("01:30:00").unwrap(), 5400);
    assert_eq!(parse_walltime("1-02:30:00").unwrap(), 86_400 + 9000);
    assert_eq!(parse_walltime("48:00").unwrap(), 48 * 3600);
    for bad in ["", "abc", "1:2:3:4", "0:00:00", "x-01:00:00"] {
        assert!(parse_walltime(bad).is_err(), "{bad}");
    }
}

#[test]
fn slurm_directives_carry_every_resource() {
    let lines = directives(Scheduler::Slurm, "colm_x", "/r/jobs/j/log", &resources()).unwrap();
    let text = lines.join("\n");
    for expected in [
        "#SBATCH --job-name=colm_x",
        "#SBATCH --output=/r/jobs/j/log",
        "#SBATCH --cpus-per-task=16",
        "#SBATCH --mem=64G",
        "#SBATCH --time=1-02:30:00",
        "#SBATCH --partition=cpu",
        "#SBATCH --account=proj_a",
        "#SBATCH --constraint=ib",
    ] {
        assert!(text.contains(expected), "{expected} in\n{text}");
    }
}

#[test]
fn pbs_and_lsf_use_their_own_syntax() {
    let pbs = directives(Scheduler::Pbs, "colm_x", "/r/log", &resources())
        .unwrap()
        .join("\n");
    for expected in [
        "#PBS -N colm_x",
        "#PBS -j oe",
        "#PBS -o /r/log",
        "#PBS -l select=1:ncpus=16:mpiprocs=1:mem=64gb",
        "#PBS -l walltime=26:30:00",
        "#PBS -q cpu",
        "#PBS -A proj_a",
        "#PBS --constraint=ib",
    ] {
        assert!(pbs.contains(expected), "{expected} in\n{pbs}");
    }
    let lsf = directives(Scheduler::Lsf, "colm_x", "/r/log", &resources())
        .unwrap()
        .join("\n");
    for expected in [
        "#BSUB -J colm_x",
        "#BSUB -oo /r/log",
        "#BSUB -n 16",
        "#BSUB -R \"span[hosts=1]\"",
        "#BSUB -R \"rusage[mem=65536]\"",
        "#BSUB -W 26:30",
        "#BSUB -q cpu",
        "#BSUB -P proj_a",
    ] {
        assert!(lsf.contains(expected), "{expected} in\n{lsf}");
    }
    assert!(directives(Scheduler::Bare, "n", "/l", &resources())
        .unwrap()
        .is_empty());
}

#[test]
fn bad_requests_are_refused_before_anything_is_written() {
    let mut r = resources();
    r.cpus = 0;
    assert!(r.validate().is_err());
    for bad in ["-evil", "a b", "x;rm", ""] {
        let mut r = resources();
        r.partition = Some(bad.into());
        assert!(r.validate().is_err(), "partition {bad:?}");
        let mut r = resources();
        r.account = Some(bad.into());
        assert!(r.validate().is_err(), "account {bad:?}");
    }
    let mut r = resources();
    r.directives = vec!["--ok\n#SBATCH --wrap=rm -rf".into()];
    assert!(r.validate().is_err());
    r.directives = vec!["no-dash".into()];
    assert!(r.validate().is_err());
    let mut r = resources();
    r.walltime = Some("soon".into());
    assert!(r.validate().is_err());
}

#[test]
fn job_names_are_short_and_safe() {
    assert_eq!(job_name("CAQfo1m"), "colm_CAQfo1m");
    assert!(job_name("a/b c;d")
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_'));
    assert!(job_name("a-very-long-case-name").len() <= 15);
}

#[test]
fn mpi_jobs_ask_for_ranks_and_spread_them_over_nodes() {
    let mut r = resources();
    r.cpus = 4;
    r.ranks = 8;
    r.nodes = 2;
    let slurm = directives(Scheduler::Slurm, "n", "/l", &r)
        .unwrap()
        .join("\n");
    assert!(
        slurm.contains("#SBATCH --nodes=2") && slurm.contains("#SBATCH --ntasks=8"),
        "{slurm}"
    );
    assert!(slurm.contains("#SBATCH --cpus-per-task=4"));
    let pbs = directives(Scheduler::Pbs, "n", "/l", &r)
        .unwrap()
        .join("\n");
    // 每个节点 4 个进程、每进程 4 核：16 核。
    assert!(
        pbs.contains("#PBS -l select=2:ncpus=16:mpiprocs=4:mem=64gb"),
        "{pbs}"
    );
    let lsf = directives(Scheduler::Lsf, "n", "/l", &r)
        .unwrap()
        .join("\n");
    assert!(
        lsf.contains("#BSUB -n 32") && lsf.contains("span[ptile=16]"),
        "{lsf}"
    );
    // 不指定节点：Slurm 自己分配，不写 --nodes。
    r.nodes = 0;
    let free = directives(Scheduler::Slurm, "n", "/l", &r)
        .unwrap()
        .join("\n");
    assert!(
        !free.contains("--nodes") && free.contains("--ntasks=8"),
        "{free}"
    );
    // 进程数分不匀就拒绝。
    r.nodes = 3;
    assert!(r.validate().is_err());
    // 不用 MPI 时和以前一样。
    let single = directives(Scheduler::Slurm, "n", "/l", &resources())
        .unwrap()
        .join("\n");
    assert!(single.contains("--nodes=1") && single.contains("--ntasks=1"));
}

#[cfg(unix)]
#[test]
fn reading_the_login_profile_keeps_the_path_the_caller_set_first() {
    // The server's own environment preparation (conda, modules) runs before this prelude; a
    // cluster whose /etc/profile rewrites PATH (摆渡船) must not undo it.
    let output = std::process::Command::new("bash")
        .args([
            "-c",
            &format!(
                "export PATH=/colm-test-first:$PATH\n{}\nprintf '%s' \"$PATH\"",
                PATH_PRELUDE
            ),
        ])
        .output()
        .unwrap();
    let path = String::from_utf8(output.stdout).unwrap();
    assert!(path.starts_with("/colm-test-first:"), "{path}");
}

#[test]
fn nodes_are_planned_from_the_cores_a_node_has() {
    let plan = |cpus: u32, ranks: u32, per_node: u32, nodes: u32| {
        Resources {
            cpus,
            ranks,
            nodes,
            cores_per_node: per_node,
            ..Resources::default()
        }
        .planned_nodes()
    };
    // Fits on one node, or the node size is unknown: nothing to plan.
    assert_eq!(plan(4, 1, 48, 0).unwrap(), 0);
    assert_eq!(plan(1, 48, 48, 0).unwrap(), 0);
    assert_eq!(plan(1, 96, 0, 0).unwrap(), 0);
    // 摆渡船 (48 cores a node): 96 ranks need two nodes, 48 cores each.
    assert_eq!(plan(1, 96, 48, 0).unwrap(), 2);
    assert_eq!(plan(2, 48, 48, 0).unwrap(), 2);
    assert_eq!(plan(1, 144, 48, 0).unwrap(), 3);
    // The ranks have to divide evenly: 100 ranks of one core use four nodes of 25.
    assert_eq!(plan(1, 100, 48, 0).unwrap(), 4);
    // An explicit node count wins.
    assert_eq!(plan(1, 96, 48, 4).unwrap(), 4);
    assert_eq!(plan(1, 8, 48, 3).unwrap(), 3);
    // One process cannot be spread over nodes.
    assert!(plan(96, 1, 48, 0)
        .unwrap_err()
        .to_string()
        .contains("single process"));
    assert!(plan(64, 4, 48, 0)
        .unwrap_err()
        .to_string()
        .contains("each MPI process"));
    assert!(plan(1, 97, 48, 0).is_err());
}

#[test]
fn lsf_spreads_a_big_mpi_job_over_the_planned_nodes() {
    let lines = |ranks: u32, nodes: u32| {
        directives(
            Scheduler::Lsf,
            "colm_t",
            "/log",
            &Resources {
                cpus: 1,
                ranks,
                nodes,
                cores_per_node: 48,
                ..Resources::default()
            },
        )
        .unwrap()
    };
    let big = lines(96, 0);
    assert!(big.contains(&"#BSUB -n 96".to_string()), "{big:?}");
    assert!(
        big.contains(&"#BSUB -R \"span[ptile=48]\"".to_string()),
        "{big:?}"
    );
    let small = lines(24, 0);
    assert!(
        small.contains(&"#BSUB -R \"span[hosts=1]\"".to_string()),
        "{small:?}"
    );
    let slurm = directives(
        Scheduler::Slurm,
        "colm_t",
        "/log",
        &Resources {
            cpus: 1,
            ranks: 96,
            cores_per_node: 48,
            ..Resources::default()
        },
    )
    .unwrap();
    assert!(
        slurm.contains(&"#SBATCH --nodes=2".to_string()),
        "{slurm:?}"
    );
}

use super::*;

fn resources() -> Resources {
    Resources {
        cpus: 16,
        ranks: 0,
        nodes: 0,
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

use super::*;

/// 第 637 轮在 T7920 上探到的结果（`cargo` 等的路径照实）。
#[test]
fn probe_output_is_parsed_and_gaps_are_named() {
    let text = "hostname=T7920\nos=Ubuntu 26.04.1 LTS\narch=x86_64\ncpus=96\nmemory_kb=790000000\n\
cargo=cargo 1.95.0 (f2d3ce0bd 2026-03-21)\ngfortran=GNU Fortran 15.2.0\nmpi=\ncmake=/usr/bin/cmake\ncc=/usr/bin/cc\n\
slurm=\npbs=\nlsf=\nroot_exists=\nroot_writable=1\nroot_free_kb=27917287424\n";
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
    assert_eq!(bare.problems.len(), 3, "{:?}", bare.problems);
    assert!(script("/r").contains("R='/r'"));
}

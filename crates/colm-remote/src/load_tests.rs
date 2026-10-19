use super::*;

#[test]
fn slurm_partitions_queues_and_my_waiting_jobs_are_read() {
    // land02 实测输出（2026-10-20），"mine" 为构造的排队与运行作业。
    let text = "hostname=login01\ncpus=40\nloadavg=3.68 3.96 4.28\nmem_kb=98876228 78565344\nscheduler=slurm\npart=first|3/4/0/7|104/568/0/672\npart=largemem|1/0/0/1|48/48/0/96\npart=second|7/0/0/7|700/388/0/1088\nqueue=first|PENDING|3\nqueue=first|RUNNING|3\nqueue=first|COMPLETING|1\nqueue=second|PENDING|3\nmine=first|PENDING|QOSMaxJobsPerUserLimit|2\nmine=first|RUNNING|None|3\n";
    let load = parse(text);
    assert_eq!(load.hostname, "login01");
    assert_eq!(load.cpus, Some(40));
    assert_eq!(load.load, Some([3.68, 3.96, 4.28]));
    assert_eq!(load.memory_total_gb, Some(94.3));
    assert_eq!(load.memory_available_gb, Some(74.9));
    assert_eq!(load.scheduler.as_deref(), Some("slurm"));
    assert_eq!(
        load.partitions[0],
        PartitionLoad {
            name: "first".into(),
            nodes: [3, 4, 0, 7],
            cpus: [104, 568, 0, 672]
        }
    );
    assert_eq!(load.partitions.len(), 3);
    assert_eq!(
        load.queues,
        [
            QueueLoad {
                name: "first".into(),
                pending: 3,
                running: 3
            },
            QueueLoad {
                name: "second".into(),
                pending: 3,
                running: 0
            }
        ]
    );
    assert_eq!(load.mine[0].reason, "QOSMaxJobsPerUserLimit");
    assert_eq!(load.mine[1].count, 3);
}

#[test]
fn lsf_hosts_and_job_states_use_the_slurm_words() {
    let text = "hostname=bd-login\ncpus=32\nscheduler=lsf\nqueue=normal|PENDING|5137\nqueue=normal|RUNNING|2257\nhosts=66|47|7|120|5000|5760\nmine=normal|PEND||4\nmine=normal|RUN||2\n";
    let load = parse(text);
    assert_eq!(
        load.hosts,
        Some(HostsLoad {
            ok: 66,
            full: 47,
            closed: 7,
            total: 120,
            slots_used: 5000,
            slots_total: 5760
        })
    );
    assert_eq!(load.queues[0].pending, 5137);
    assert_eq!(
        load.mine
            .iter()
            .map(|j| j.state.as_str())
            .collect::<Vec<_>>(),
        ["PENDING", "RUNNING"]
    );
    assert!(load.partitions.is_empty());
}

#[test]
fn malformed_or_missing_lines_leave_fields_empty() {
    let load = parse("cpus=\nloadavg=\nmem_kb=\npart=first|x/1|2\nhosts=1|2\nnoise\nscheduler=\n");
    assert_eq!(load, Load::default());
}

#[test]
fn the_script_loads_scheduler_paths_and_limits_each_query() {
    let script = script();
    assert!(script.contains(crate::sched::PATH_PRELUDE));
    for query in [
        "sinfo -h",
        "squeue -h -u",
        "bqueues -w",
        "bhosts -w",
        "qstat -u",
    ] {
        assert!(script.contains(&format!("colm_t {query}")), "{query}");
    }
}

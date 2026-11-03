use super::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

fn scratch_rendezvous(tag: &str) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after the epoch")
        .as_nanos();
    let directory =
        std::env::temp_dir().join(format!("colm-spmd-{tag}-{}-{unique}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("create the scratch directory");
    directory.join("endpoint")
}

#[test]
fn rank_info_accepts_explicit_values() {
    let info = RankInfo::resolve(Some(2), Some(4)).unwrap();
    assert_eq!(info.rank(), 2);
    assert_eq!(info.size(), 4);
    assert!(!info.is_root());
    assert!(RankInfo::new(0, 1).unwrap().is_root());
}

#[test]
fn rank_info_rejects_partial_or_out_of_range_values() {
    assert!(RankInfo::resolve(Some(1), None).is_err());
    assert!(RankInfo::resolve(None, Some(2)).is_err());
    assert!(RankInfo::new(4, 4).is_err());
    assert!(RankInfo::new(0, 0).is_err());
}

#[test]
fn rank_info_reads_the_launcher_environment_in_priority_order() {
    let env = |pairs: &[(&str, &str)]| {
        let map = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect::<HashMap<_, _>>();
        move |name: &str| map.get(name).cloned()
    };
    let slurm_and_openmpi = env(&[
        ("SLURM_PROCID", "2"),
        ("SLURM_NTASKS", "4"),
        ("OMPI_COMM_WORLD_RANK", "1"),
        ("OMPI_COMM_WORLD_SIZE", "8"),
    ]);
    let info = RankInfo::resolve_with(None, None, slurm_and_openmpi).unwrap();
    assert_eq!((info.rank(), info.size()), (2, 4));

    let only_pmi = env(&[("PMI_RANK", "3"), ("PMI_SIZE", "7")]);
    let info = RankInfo::resolve_with(None, None, only_pmi).unwrap();
    assert_eq!((info.rank(), info.size()), (3, 7));

    let explicit = env(&[("SLURM_PROCID", "2"), ("SLURM_NTASKS", "4")]);
    let info = RankInfo::resolve_with(Some(0), Some(2), explicit).unwrap();
    assert_eq!((info.rank(), info.size()), (0, 2));

    let nothing = env(&[]);
    assert!(RankInfo::resolve_with(None, None, nothing).is_err());
}

#[test]
fn optional_rank_info_rejects_invalid_or_incomplete_launcher_identity() {
    for (rank_name, size_name) in LAUNCHER_ENV {
        for (rank, size) in [
            (Some("2"), Some("2")),
            (Some("bad"), Some("2")),
            (None, Some("2")),
            (Some("0"), None),
            (Some("0"), Some("0")),
        ] {
            let env = |name: &str| {
                if name == rank_name {
                    rank.map(str::to_owned)
                } else if name == size_name {
                    size.map(str::to_owned)
                } else {
                    None
                }
            };
            assert!(
                RankInfo::resolve_optional_with(None, None, env).is_err(),
                "{rank_name}={rank:?}, {size_name}={size:?} must not become a single-process writer"
            );
            assert_eq!(
                RankInfo::resolve_optional_with(Some(0), Some(2), env).unwrap(),
                Some(RankInfo::new(0, 2).unwrap())
            );
        }
    }
}

#[test]
fn optional_rank_info_distinguishes_absent_single_and_multiple_processes() {
    assert_eq!(
        RankInfo::resolve_optional_with(None, None, |_| None).unwrap(),
        None
    );
    for (rank_name, size_name) in LAUNCHER_ENV {
        for size in [1, 4] {
            let env = |name: &str| {
                if name == rank_name {
                    Some("0".to_owned())
                } else if name == size_name {
                    Some(size.to_string())
                } else {
                    None
                }
            };
            assert_eq!(
                RankInfo::resolve_optional_with(None, None, env).unwrap(),
                (size > 1).then(|| RankInfo::new(0, size).unwrap())
            );
        }
    }
}

#[test]
fn f64_and_bool_payloads_round_trip_in_order() {
    let values = [0.0, -1.5, f64::NAN, 1.0e300];
    let bytes = encode_f64s(&values);
    let decoded = decode_f64s(&bytes).unwrap();
    // NaN 不能直接比相等，按位比较其余项。
    for (expected, actual) in values.iter().zip(&decoded) {
        if expected.is_nan() {
            assert!(actual.is_nan());
        } else {
            assert_eq!(expected, actual);
        }
    }
    assert!(decode_f64s(&bytes[..bytes.len() - 1]).is_err());

    let flags = [true, false, true, true];
    let bytes = encode_bools(&flags);
    assert_eq!(decode_bools(&bytes).unwrap(), flags);
    assert!(decode_bools(&[0, 2]).is_err());
}

#[test]
fn u64_payloads_round_trip_in_little_endian_order() {
    let values = [0u64, 1, u64::MAX, 0x0102_0304_0506_0708];
    let bytes = encode_u64s(&values);
    assert_eq!(decode_u64s(&bytes).unwrap(), values);
    assert_eq!(&bytes[..8], &0u64.to_le_bytes());
    assert!(decode_u64s(&bytes[..bytes.len() - 1]).is_err());
}

#[test]
fn gather_scatter_broadcast_and_barrier_preserve_rank_order() {
    let rendezvous = scratch_rendezvous("collective");
    let size = 3;
    let workers = (1..size)
        .map(|rank| {
            let rendezvous = rendezvous.clone();
            std::thread::spawn(move || {
                let info = RankInfo::new(rank, size).unwrap();
                let mut comms =
                    Comms::open_with_timeout(&rendezvous, info, None, Duration::from_secs(10))
                        .unwrap();
                let local = vec![rank as u8; rank + 1];
                assert_eq!(comms.gather(&local).unwrap(), None);
                assert_eq!(
                    comms.scatter(None, None).unwrap(),
                    vec![(10 + rank) as u8; rank + 1]
                );
                assert_eq!(comms.broadcast(None).unwrap(), b"hello".to_vec());
                comms.barrier().unwrap();
            })
        })
        .collect::<Vec<_>>();

    let info = RankInfo::new(0, size).unwrap();
    let mut root = Comms::open_with_timeout(
        &rendezvous,
        info,
        Some("127.0.0.1"),
        Duration::from_secs(10),
    )
    .unwrap();
    let global = root.gather(&[0u8]).unwrap().unwrap();
    assert_eq!(global, [0, 1, 1, 2, 2, 2]);
    let global = [10u8, 11, 11, 12, 12, 12];
    let lengths = [1usize, 2, 3];
    assert_eq!(root.scatter(Some(&global), Some(&lengths)).unwrap(), [10u8]);
    assert_eq!(root.broadcast(Some(b"hello")).unwrap(), b"hello".to_vec());
    root.barrier().unwrap();
    for worker in workers {
        worker.join().unwrap();
    }
    let _ = std::fs::remove_dir_all(rendezvous.parent().unwrap());
}

#[test]
fn a_worker_times_out_when_root_never_starts() {
    let rendezvous = scratch_rendezvous("no-root");
    let info = RankInfo::new(1, 2).unwrap();
    let started = std::time::Instant::now();
    let error = Comms::open_with_timeout(&rendezvous, info, None, Duration::from_millis(150))
        .err()
        .expect("a worker without root must time out");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(
        format!("{error:#}").contains("timed out"),
        "unexpected error: {error:#}"
    );
    let _ = std::fs::remove_dir_all(rendezvous.parent().unwrap());
}

#[test]
fn root_times_out_when_a_worker_never_connects() {
    let rendezvous = scratch_rendezvous("no-worker");
    let started = std::time::Instant::now();
    let error = Comms::root_with_timeout(
        &rendezvous,
        2,
        Some("127.0.0.1"),
        Duration::from_millis(150),
    )
    .err()
    .expect("root without a worker must time out");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(
        format!("{error:#}").contains("timed out"),
        "unexpected error: {error:#}"
    );
    let _ = std::fs::remove_dir_all(rendezvous.parent().unwrap());
}

fn unused_port() -> u16 {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind a scratch port");
    listener.local_addr().expect("read the scratch port").port()
}

#[test]
fn a_worker_follows_a_rendezvous_that_root_rewrites() {
    let rendezvous = scratch_rendezvous("stale-endpoint");
    // 上一次运行（被 kill）会留下指向旧端口的端点文件。worker 先读到它时不能一直连旧地址，
    // 要随 root 覆盖的新端点重连 —— bd 上真实挂死过（worker 连 46185，root 在 41525 等）。
    std::fs::write(&rendezvous, format!("127.0.0.1\n{}\n", unused_port()))
        .expect("write the stale endpoint");
    let worker = {
        let rendezvous = rendezvous.clone();
        std::thread::spawn(move || {
            let info = RankInfo::new(1, 2).unwrap();
            let mut comms =
                Comms::open_with_timeout(&rendezvous, info, None, Duration::from_secs(10))?;
            comms.barrier()?;
            Ok::<_, anyhow::Error>(comms)
        })
    };
    std::thread::sleep(Duration::from_millis(100));
    let info = RankInfo::new(0, 2).unwrap();
    let mut root = Comms::open_with_timeout(
        &rendezvous,
        info,
        Some("127.0.0.1"),
        Duration::from_secs(10),
    )
    .expect("root must open even when a stale endpoint file exists");
    root.barrier().unwrap();
    worker
        .join()
        .unwrap()
        .expect("worker must follow the rewritten endpoint");
    let _ = std::fs::remove_dir_all(rendezvous.parent().unwrap());
}

#[test]
fn optional_resolution_only_returns_multi_process_ranks() {
    let explicit = RankInfo::resolve_optional(Some(1), Some(2))
        .unwrap()
        .unwrap();
    assert_eq!((explicit.rank(), explicit.size()), (1, 2));
    assert!(RankInfo::resolve_optional(None, None).unwrap().is_none());
    assert!(RankInfo::resolve_optional(Some(0), None).is_err());
}

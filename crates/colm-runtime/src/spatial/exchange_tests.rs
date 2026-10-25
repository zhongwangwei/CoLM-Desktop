use super::*;
use colm_spmd::{Comms, RankInfo};
use std::path::PathBuf;
use std::time::Duration;

fn scratch_rendezvous(tag: &str) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after the epoch")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "colm-exchange-{tag}-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    directory.join("endpoint")
}

#[test]
fn three_processes_all_gather_patch_arrays_in_rank_order() {
    let rendezvous = scratch_rendezvous("three");
    let size = 3;
    let local_lengths = [2usize, 3, 1];
    let global = local_lengths.iter().sum::<usize>();
    let offsets = [0usize, 2, 5];
    let workers = (1..size)
        .map(|rank| {
            let rendezvous = rendezvous.clone();
            std::thread::spawn(move || {
                let info = RankInfo::new(rank, size).unwrap();
                let comms =
                    Comms::open_with_timeout(&rendezvous, info, None, Duration::from_secs(10))
                        .unwrap();
                let mut exchange =
                    PatchExchange::new(comms, offsets[rank], local_lengths[rank], global).unwrap();
                let local = (0..local_lengths[rank])
                    .map(|i| (rank * 100 + i) as f64)
                    .collect::<Vec<_>>();
                let gathered = exchange.all_gather_f64(&local).unwrap();
                let local_bools = vec![rank % 2 == 1; local_lengths[rank]];
                let gathered_bools = exchange.all_gather_bools(&local_bools).unwrap();
                assert_eq!(exchange.patch_offset(), offsets[rank]);
                assert_eq!(exchange.global_patches(), global);
                (gathered, gathered_bools)
            })
        })
        .collect::<Vec<_>>();

    let info = RankInfo::new(0, size).unwrap();
    let comms = Comms::open_with_timeout(
        &rendezvous,
        info,
        Some("127.0.0.1"),
        Duration::from_secs(10),
    )
    .unwrap();
    let mut exchange = PatchExchange::new(comms, 0, local_lengths[0], global).unwrap();
    let local = (0..local_lengths[0]).map(|i| i as f64).collect::<Vec<_>>();
    let expected_f64 = vec![0.0, 1.0, 100.0, 101.0, 102.0, 200.0];
    assert_eq!(exchange.all_gather_f64(&local).unwrap(), expected_f64);
    let local_bools = vec![false, false];
    let expected_bools = vec![false, false, true, true, true, false];
    assert_eq!(
        exchange.all_gather_bools(&local_bools).unwrap(),
        expected_bools
    );
    for worker in workers {
        let (f64s, bools) = worker.join().unwrap();
        assert_eq!(f64s, expected_f64);
        assert_eq!(bools, expected_bools);
    }
    let _ = std::fs::remove_dir_all(rendezvous.parent().unwrap());
}

#[test]
fn a_wrong_local_length_is_rejected_before_touching_the_network() {
    let rendezvous = scratch_rendezvous("length");
    let info = RankInfo::new(0, 1).unwrap();
    let comms = Comms::open_with_timeout(
        &rendezvous,
        info,
        Some("127.0.0.1"),
        Duration::from_secs(10),
    )
    .unwrap();
    let mut exchange = PatchExchange::new(comms, 0, 2, 2).unwrap();
    let error = exchange.all_gather_f64(&[1.0]).unwrap_err();
    assert!(format!("{error:#}").contains("local patches"));
    let _ = std::fs::remove_dir_all(rendezvous.parent().unwrap());
}

#[test]
fn a_patch_range_outside_the_global_count_is_rejected() {
    let rendezvous = scratch_rendezvous("range");
    let info = RankInfo::new(0, 1).unwrap();
    let comms = Comms::open_with_timeout(
        &rendezvous,
        info,
        Some("127.0.0.1"),
        Duration::from_secs(10),
    )
    .unwrap();
    assert!(PatchExchange::new(comms, 1, 2, 2).is_err());
    let _ = std::fs::remove_dir_all(rendezvous.parent().unwrap());
}

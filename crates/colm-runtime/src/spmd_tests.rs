use super::*;
use crate::spatial::partition::BlockEntry;
use std::path::PathBuf;

fn inventory(patches: &[(&str, usize)]) -> BlockInventory {
    BlockInventory::from_entries(
        patches
            .iter()
            .map(|&(name, patches)| BlockEntry {
                name: name.to_owned(),
                patches,
            })
            .collect(),
    )
    .unwrap()
}

fn plan(inventory: BlockInventory, rank: usize, size: usize) -> SpmdPlan {
    SpmdPlan::from_inventory(inventory, RankInfo::new(rank, size).unwrap()).unwrap()
}

fn scratch_rendezvous(tag: &str) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after the epoch")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "colm-spmd-plan-{tag}-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    directory.join("endpoint")
}

#[test]
fn plans_cover_every_block_exactly_once() {
    let blocks = [
        ("e000_n00", 5usize),
        ("e010_n00", 1),
        ("e020_n00", 9),
        ("e030_n00", 2),
        ("e040_n00", 6),
        ("e050_n00", 3),
    ];
    let inventory = inventory(&blocks);
    let mut seen = Vec::new();
    for rank in 0..3 {
        let plan = plan(inventory.clone(), rank, 3);
        assert_eq!(plan.local_block_names().len(), plan.local().blocks().len());
        assert_eq!(
            plan.local_global_offsets().len(),
            plan.local().blocks().len()
        );
        assert!(
            plan.local_patch_count() > 0,
            "rank {rank} received no patch in a case with six blocks"
        );
        seen.extend(plan.local().blocks().iter().copied());
    }
    seen.sort_unstable();
    assert_eq!(seen, (0..inventory.len()).collect::<Vec<_>>());
    assert_eq!(
        (0..3)
            .map(|rank| plan(inventory.clone(), rank, 3).local_patch_count())
            .sum::<usize>(),
        inventory.total_patches()
    );
}

#[test]
fn the_encoded_plan_is_stable_and_rank_specific() {
    let inventory = inventory(&[("e000_n00", 5), ("e010_n00", 1), ("e020_n00", 9)]);
    let first = plan(inventory.clone(), 0, 2);
    let second = plan(inventory.clone(), 0, 2);
    assert_eq!(first.encode(), second.encode());
    let other = plan(inventory, 1, 2);
    assert_ne!(first.encode(), other.encode());
}

#[test]
fn a_single_rank_opens_without_peers() {
    let rendezvous = scratch_rendezvous("single");
    let plan = plan(inventory(&[("e000_n00", 5), ("e010_n00", 1)]), 0, 1);
    let session = SpmdSession::open_with_plan(plan, &rendezvous, Some("127.0.0.1")).unwrap();
    assert_eq!(session.plan().local_block_names(), ["e000_n00", "e010_n00"]);
    let _ = std::fs::remove_dir_all(rendezvous.parent().unwrap());
}

#[test]
fn three_real_processes_agree_on_the_plan() {
    let rendezvous = scratch_rendezvous("agree");
    let size = 3;
    let inventory = inventory(&[
        ("e000_n00", 5),
        ("e010_n00", 1),
        ("e020_n00", 9),
        ("e030_n00", 2),
        ("e040_n00", 6),
        ("e050_n00", 3),
    ]);
    let workers = (1..size)
        .map(|rank| {
            let inventory = inventory.clone();
            let rendezvous = rendezvous.clone();
            std::thread::spawn(move || {
                let plan = plan(inventory, rank, size);
                let session = SpmdSession::open_with_plan(plan, &rendezvous, None).unwrap();
                (
                    session.plan().rank(),
                    session.plan().local_block_names().join(","),
                    session.plan().local_patch_count(),
                )
            })
        })
        .collect::<Vec<_>>();

    let root_plan = plan(inventory.clone(), 0, size);
    let root = SpmdSession::open_with_plan(root_plan, &rendezvous, Some("127.0.0.1")).unwrap();
    assert_eq!(root.plan().size(), size);
    assert_eq!(
        root.plan().local_block_names().len(),
        root.plan().local().blocks().len()
    );

    let mut worker_patches = 0;
    for worker in workers {
        let (rank, names, patches) = worker.join().unwrap();
        assert_eq!(names, root.plan().local_block_names_of(rank).join(","));
        worker_patches += patches;
    }
    assert_eq!(
        root.plan().local_patch_count() + worker_patches,
        inventory.total_patches()
    );
    let _ = std::fs::remove_dir_all(rendezvous.parent().unwrap());
}

#[test]
fn a_rank_with_a_different_inventory_is_rejected_by_everyone() {
    let rendezvous = scratch_rendezvous("mismatch");
    let size = 2;
    let good = inventory(&[("e000_n00", 5), ("e010_n00", 1), ("e020_n00", 9)]);
    let bad = inventory(&[("e000_n00", 5), ("e010_n00", 1), ("e020_n00", 2)]);
    let worker = {
        let rendezvous = rendezvous.clone();
        std::thread::spawn(move || {
            let plan = plan(bad, 1, size);
            SpmdSession::open_with_plan(plan, &rendezvous, None).map(|_| ())
        })
    };
    let root = SpmdSession::open_with_plan(plan(good, 0, size), &rendezvous, Some("127.0.0.1"));
    let root_error = format!(
        "{:#}",
        root.map(|_| ()).expect_err("root must reject the bad plan")
    );
    let worker_error = format!(
        "{:#}",
        worker
            .join()
            .unwrap()
            .expect_err("the worker must learn that the plan was rejected")
    );
    assert!(
        root_error.contains("block plan mismatch"),
        "unexpected root error: {root_error}"
    );
    assert!(
        worker_error.contains("block plan"),
        "unexpected worker error: {worker_error}"
    );
    let _ = std::fs::remove_dir_all(rendezvous.parent().unwrap());
}

/// 真实 landdata 上读块清单与分片；`COLM_SPATIAL_FIXTURE=<case>/out/<case>/landdata`。
#[test]
#[ignore = "needs COLM_SPATIAL_FIXTURE=<case>/out/<case>/landdata"]
fn a_real_landdata_inventory_round_trips_through_the_plan() {
    let landdata = PathBuf::from(
        std::env::var_os("COLM_SPATIAL_FIXTURE")
            .expect("set COLM_SPATIAL_FIXTURE to a case's landdata directory"),
    );
    let year = std::fs::read_dir(landdata.join("landpatch"))
        .expect("read landpatch root")
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().to_string_lossy().parse::<i32>().ok())
        .next()
        .expect("a <year> directory under landpatch");
    let plan = SpmdPlan::read(&landdata, year, Some(0), Some(1)).unwrap();
    let assignment = plan.assignment();
    assert_eq!(assignment.len(), 1);
    assert_eq!(
        plan.local().blocks(),
        (0..plan.inventory().len()).collect::<Vec<_>>()
    );
    assert_eq!(plan.local_patch_count(), plan.inventory().total_patches());
    assert_eq!(plan.local_block_names().len(), plan.inventory().len());
}

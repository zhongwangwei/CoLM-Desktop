use super::*;

fn entry(name: &str, patches: usize) -> BlockEntry {
    BlockEntry {
        name: name.to_owned(),
        patches,
    }
}

fn inventory(patches: &[(&str, usize)]) -> BlockInventory {
    BlockInventory::from_entries(
        patches
            .iter()
            .map(|&(name, patches)| entry(name, patches))
            .collect(),
    )
    .unwrap()
}

#[test]
fn offsets_are_the_prefix_sums_of_the_patch_counts() {
    let inventory = inventory(&[("e000_n00", 4), ("e010_n00", 7), ("e020_n00", 2)]);
    assert_eq!(inventory.total_patches(), 13);
    assert_eq!(inventory.offsets(), [0, 4, 11]);
}

#[test]
fn one_process_owns_every_block_in_global_order() {
    let inventory = inventory(&[("e000_n00", 4), ("e010_n00", 7), ("e020_n00", 2)]);
    let assignment = inventory.split(1).unwrap();
    assert_eq!(assignment.len(), 1);
    let rank = assignment.rank(0).unwrap();
    assert_eq!(rank.blocks(), [0, 1, 2]);
    assert_eq!(rank.global_offsets(), inventory.offsets());
    assert_eq!(rank.patch_count(), 13);
}

#[test]
fn every_block_is_assigned_exactly_once_and_ranks_keep_contiguous_global_ranges() {
    let inventory = inventory(&[
        ("e000_n00", 5),
        ("e010_n00", 1),
        ("e020_n00", 9),
        ("e030_n00", 2),
        ("e040_n00", 6),
        ("e050_n00", 3),
    ]);
    let assignment = inventory.split(3).unwrap();
    let mut seen = Vec::new();
    let mut total = 0;
    let mut next_expected_block = 0usize;
    for rank in assignment.iter() {
        let blocks = rank.blocks().to_vec();
        // 连续分片：每个 rank 的块必须紧接上一段，且段内递增。
        assert_eq!(blocks.first().copied(), Some(next_expected_block));
        for (offset, &block) in blocks.iter().enumerate() {
            assert_eq!(block, next_expected_block + offset);
        }
        next_expected_block += blocks.len();
        assert_eq!(rank.global_offsets().len(), blocks.len());
        total += rank.patch_count();
        seen.extend(blocks);
    }
    assert_eq!(next_expected_block, inventory.len());
    seen.sort_unstable();
    assert_eq!(seen, (0..inventory.len()).collect::<Vec<_>>());
    assert_eq!(total, inventory.total_patches());
}

/// 连续切分的界：最重的 rank 不超过平均值加最大块。
#[test]
fn contiguous_split_keeps_the_load_within_the_same_bound() {
    let inventory = inventory(&[
        ("e000_n00", 100),
        ("e010_n00", 1),
        ("e020_n00", 1),
        ("e030_n00", 1),
        ("e040_n00", 1),
    ]);
    let assignment = inventory.split(2).unwrap();
    let loads = assignment.loads();
    let average = inventory.total_patches() as f64 / 2.0;
    assert!(
        *loads.iter().max().unwrap() as f64 <= average + 100.0,
        "loads {loads:?} exceed the LPT bound"
    );
    // 第一个 block 就超过目标，边界落在它之后：100 单独一组，四个 1 在另一组。
    assert_eq!(loads, [100, 4]);
}

#[test]
fn the_split_is_deterministic() {
    let inventory = inventory(&[
        ("e000_n00", 5),
        ("e010_n00", 1),
        ("e020_n00", 9),
        ("e030_n00", 2),
    ]);
    assert_eq!(inventory.split(3).unwrap(), inventory.split(3).unwrap());
}

#[test]
fn a_rank_can_be_looked_up_and_out_of_range_is_rejected() {
    let inventory = inventory(&[("e000_n00", 1), ("e010_n00", 2)]);
    let assignment = inventory.split(2).unwrap();
    assert_eq!(assignment.rank(0).unwrap().blocks().len(), 1);
    assert_eq!(assignment.rank(1).unwrap().blocks().len(), 1);
    assert!(assignment.rank(2).is_err());
}

#[test]
fn too_many_processes_or_zero_processes_are_rejected() {
    let inventory = inventory(&[("e000_n00", 1)]);
    assert!(inventory.split(0).is_err());
    assert!(inventory.split(2).is_err());
}

#[test]
fn an_empty_or_duplicate_or_empty_block_inventory_is_rejected() {
    assert!(BlockInventory::from_entries(Vec::new()).is_err());
    assert!(BlockInventory::from_entries(vec![entry("e000_n00", 0)]).is_err());
    assert!(
        BlockInventory::from_entries(vec![entry("e000_n00", 1), entry("e000_n00", 2)]).is_err()
    );
}

#[test]
fn splitting_two_blocks_over_two_processes_gives_one_each() {
    let inventory = inventory(&[("e000_n00", 3), ("e010_n00", 4)]);
    let assignment = inventory.split(2).unwrap();
    assert_eq!(assignment.loads(), [3, 4]);
    assert_eq!(assignment.rank(0).unwrap().global_offsets(), [0]);
    assert_eq!(assignment.rank(1).unwrap().global_offsets(), [3]);
    assert_eq!(assignment.rank(0).unwrap().blocks(), [0]);
    assert_eq!(assignment.rank(1).unwrap().blocks(), [1]);
}

use super::{block_shards, resolve_rank};
use colm_spmd::RankInfo;

fn labels(count: usize) -> Vec<String> {
    (0..count).map(|i| format!("b{i}")).collect()
}

#[test]
fn single_rank_returns_every_block_in_order() {
    let blocks = labels(7);
    let rank = RankInfo::new(0, 1).unwrap();
    assert_eq!(block_shards(&blocks, rank).unwrap(), blocks);
}

#[test]
fn contiguous_ranges_cover_each_block_exactly_once() {
    let blocks = labels(7);
    let mut seen = Vec::new();
    for rank in 0..3 {
        let shard = block_shards(&blocks, RankInfo::new(rank, 3).unwrap()).unwrap();
        assert!(!shard.is_empty());
        seen.extend(shard);
    }
    assert_eq!(seen, blocks);
}

#[test]
fn ranges_stay_contiguous_and_keep_global_order() {
    let blocks = labels(10);
    for rank in 0..4 {
        let shard = block_shards(&blocks, RankInfo::new(rank, 4).unwrap()).unwrap();
        let first = blocks.iter().position(|b| b == &shard[0]).unwrap();
        assert_eq!(shard, blocks[first..first + shard.len()]);
    }
}

#[test]
fn empty_block_list_is_not_an_error() {
    let rank = RankInfo::new(1, 2).unwrap();
    assert!(block_shards(&[], rank).unwrap().is_empty());
}

#[test]
fn more_ranks_than_blocks_is_rejected() {
    let blocks = labels(3);
    let error = block_shards(&blocks, RankInfo::new(1, 4).unwrap()).unwrap_err();
    assert!(error.to_string().contains("at least as many blocks"));
}

#[test]
fn explicit_parameters_override_launcher_environment() {
    // 显式 --rank/--ranks 不读环境；即使环境里有别的值，也以显式为准。
    let info = resolve_rank(Some(1), Some(3)).unwrap().unwrap();
    assert_eq!((info.rank(), info.size()), (1, 3));
}

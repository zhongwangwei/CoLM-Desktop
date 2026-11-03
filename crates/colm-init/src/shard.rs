//! 空间预处理的多进程分片。
//!
//! 多进程只做“按整块分工”（`docs/design-multi-node.md` §3.2）：每个 rank 认领一段
//! **连续的块区间**，各写各的 `restart/<日期>/…_w<lon>_s<lat>.nc` 与 `mesh`/`landpatch`
//! 文件；块内内容与单进程逐字节相同，因此 1 vs N rank 的全部产物逐文件一致。
//!
//! 分片只依赖块的全局顺序与 rank/size，不读文件系统遍历顺序；N=1 时返回输入本身，
//! 调用方不多走一条代码路径（保证单进程行为不变）。

use anyhow::{bail, Result};
use colm_spmd::RankInfo;

/// 从启动器环境或显式参数解析多进程身份。
///
/// 显式 `--rank/--ranks` 优先；两者都没给时读启动器变量。常见情况是单进程
/// （没有 `SLURM_*`/`PMI_*` 或 size=1），此时返回 `None`，调用方完全走原来的单进程
/// 路径。环境变量里 size>1 但解析失败时直接报错，不静默退化成单进程——那会让每个
/// rank 都去写全部块，覆盖彼此的输出。
pub fn resolve_rank(rank: Option<usize>, ranks: Option<usize>) -> Result<Option<RankInfo>> {
    RankInfo::resolve_optional(rank, ranks)
}

/// 当前 rank 负责的块（连续区间、保持全局顺序）。
///
/// rank 数多于块数是调用方错误：多出来的 rank 只会拿到空区间，什么也不写，白占
/// 节点。这里返回明确错误而不是静默跳过；空块列表是合法的（纯海洋区域），此时
/// 每个 rank 都得到空表，调用方正常结束。
pub fn block_shards(blocks: &[String], rank: RankInfo) -> Result<Vec<String>> {
    if blocks.is_empty() {
        return Ok(Vec::new());
    }
    if rank.size() > blocks.len() {
        bail!(
            "multi-process preprocessing with {} ranks needs at least as many blocks, but this case has {}",
            rank.size(),
            blocks.len()
        );
    }
    let length = blocks.len();
    let start = length * rank.rank() / rank.size();
    let end = length * (rank.rank() + 1) / rank.size();
    Ok(blocks[start..end].to_vec())
}

#[cfg(test)]
#[path = "shard_tests.rs"]
mod shard_tests;

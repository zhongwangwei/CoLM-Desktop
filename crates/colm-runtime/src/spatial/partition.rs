//! 多进程分片账本：块清单、块到 rank 的确定性分配、每块的全局 patch 偏移。
//!
//! 分片单位是完整 block。理由是重启与旁车本来就每块一份：整块归属后每个 rank 直接写自己的
//! `_<block>.nc`，文件名与内容都不用改，逐位对照的代价最小。按连续 patch 区间切分要把边界块的
//! 重启/旁车再汇聚一遍，留到 Phase 5 真出现 >20% 负载不均时再评估。
//!
//! 分配必须**确定性**：所有 rank 只凭 `landdata` 与 rank 数算出同一张表，不依赖目录遍历顺序，
//! 也不落额外文件；这样换 rank 数不需要重跑预处理。

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{ensure, Context, Result};

use super::topology::{block_names, landpatch_path};

/// 一个块：名字（`e100_n35`）与它名下的 patch 数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockEntry {
    pub name: String,
    pub patches: usize,
}

/// 某一土地覆盖年份的完整块清单，按上游顺序（外层经度、内层纬度，自西南向东北）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BlockInventory {
    entries: Vec<BlockEntry>,
}

impl BlockInventory {
    /// 从已知条目构造；测试与需要绕过磁盘的调用方用。
    pub fn from_entries(entries: Vec<BlockEntry>) -> Result<Self> {
        ensure!(
            !entries.is_empty(),
            "a spatial case needs at least one landpatch block"
        );
        ensure!(
            entries.iter().all(|entry| entry.patches > 0),
            "a landpatch block has no patch"
        );
        let mut seen = BTreeSet::new();
        for entry in &entries {
            ensure!(
                seen.insert(entry.name.as_str()),
                "duplicate landpatch block {:?}",
                entry.name
            );
        }
        Ok(Self { entries })
    }

    /// 读 `landdata/landpatch/<year>/` 下每块的 patch 数。
    ///
    /// 只打开每块读 `ipxstt` 的长度，不组装 patch 状态；启动期是 O(块数) 而不是 O(patch 数)。
    pub fn read(landdata: &Path, year: i32) -> Result<Self> {
        let names = block_names(landdata, year)?;
        let mut entries = Vec::with_capacity(names.len());
        for name in names {
            let path = landpatch_path(landdata, year, &name);
            let file =
                netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
            let patches = file
                .variable("ipxstt")
                .with_context(|| format!("{} has no ipxstt", path.display()))?
                .get_values::<i32, _>(..)
                .with_context(|| format!("cannot read ipxstt from {}", path.display()))?
                .len();
            ensure!(patches > 0, "{} has no patch", path.display());
            entries.push(BlockEntry { name, patches });
        }
        Self::from_entries(entries)
    }

    pub fn entries(&self) -> &[BlockEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn total_patches(&self) -> usize {
        self.entries.iter().map(|entry| entry.patches).sum()
    }

    /// 每块在全局 patch 顺序里的起点；局部 patch 下标加上它才是全局下标。
    pub fn offsets(&self) -> Vec<usize> {
        let mut offsets = Vec::with_capacity(self.entries.len());
        let mut next = 0;
        for entry in &self.entries {
            offsets.push(next);
            next += entry.patches;
        }
        offsets
    }

    /// 把块按全局顺序切成 `ranks` 段连续区间（按 patch 数尽量均分）。
    ///
    /// **连续**是每步交换的前提：各 rank 的局部 patch 数组按 rank 顺序拼接起来就是全局 patch
    /// 顺序，不需要每步按全局索引重排。上游 FLAT_SPMD 对 element 也是连续切分；这里粒度是块。
    /// 结果只取决于清单与 rank 数，与目录遍历顺序无关。
    pub fn split(&self, ranks: usize) -> Result<BlockAssignment> {
        ensure!(ranks >= 1, "at least one process is needed");
        ensure!(
            ranks <= self.entries.len(),
            "{} landpatch blocks cannot be split over {} processes",
            self.entries.len(),
            ranks
        );
        let total = self.total_patches();
        let mut prefix = Vec::with_capacity(self.entries.len() + 1);
        let mut acc = 0usize;
        prefix.push(0);
        for entry in &self.entries {
            acc += entry.patches;
            prefix.push(acc);
        }
        // 第 r 个边界是“前 r 个 rank 一共拿多少块”；目标是最接近 r/ranks 总量的块边界。
        let mut boundaries = Vec::with_capacity(ranks - 1);
        let mut previous = 0usize;
        for r in 1..ranks {
            let target_numerator = total.saturating_mul(r);
            // ceil(total * r / ranks)，避免整数除法把目标压低。
            let target = target_numerator.div_ceil(ranks);
            let smallest = previous + 1;
            let largest = self.entries.len() - (ranks - r);
            ensure!(
                smallest <= largest,
                "cannot split {total} patches over {ranks} processes"
            );
            // 从 smallest 往后找第一个前缀和 >= target 的边界；找不到就取 largest。
            let mut boundary = smallest;
            while boundary < largest && prefix[boundary] < target {
                boundary += 1;
            }
            boundaries.push(boundary);
            previous = boundary;
        }
        let mut ranks_output = Vec::with_capacity(ranks);
        let mut start = 0usize;
        for &end in boundaries
            .iter()
            .chain(std::iter::once(&self.entries.len()))
        {
            let blocks = (start..end).collect::<Vec<_>>();
            let patches = prefix[end] - prefix[start];
            let global_offsets = (start..end).map(|index| prefix[index]).collect();
            ranks_output.push(RankBlocks {
                blocks,
                global_offsets,
                patches,
            });
            start = end;
        }
        Ok(BlockAssignment {
            ranks: ranks_output,
        })
    }
}

/// 所有 rank 的分片表。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockAssignment {
    ranks: Vec<RankBlocks>,
}

impl BlockAssignment {
    pub fn len(&self) -> usize {
        self.ranks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ranks.is_empty()
    }

    pub fn rank(&self, rank: usize) -> Result<&RankBlocks> {
        self.ranks.get(rank).with_context(|| {
            format!(
                "rank {rank} is outside 0..{}; the block split has {} processes",
                self.ranks.len(),
                self.ranks.len()
            )
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = &RankBlocks> {
        self.ranks.iter()
    }

    /// 每个 rank 分到的 patch 数，按 rank 顺序；日志与负载校验用。
    pub fn loads(&self) -> Vec<usize> {
        self.ranks.iter().map(|rank| rank.patches).collect()
    }
}

/// 一个 rank 分到的块；块下标按全局顺序排列。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankBlocks {
    blocks: Vec<usize>,
    global_offsets: Vec<usize>,
    patches: usize,
}

impl RankBlocks {
    pub fn blocks(&self) -> &[usize] {
        &self.blocks
    }

    /// 与 [`RankBlocks::blocks`] 对齐的全局 patch 起点。
    pub fn global_offsets(&self) -> &[usize] {
        &self.global_offsets
    }

    pub fn patch_count(&self) -> usize {
        self.patches
    }
}

#[cfg(test)]
#[path = "partition_tests.rs"]
mod partition_tests;

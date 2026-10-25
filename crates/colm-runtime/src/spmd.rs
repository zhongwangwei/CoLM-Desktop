//! 多进程启动期的分片账本与一致性握手。
//!
//! Phase 2 的第一层：每个 rank 独立算出同一张块分片表，再经 TCP 把表收给 root 逐项比对。
//! 这一步先于状态装配与每步交换 —— 如果各 rank 看到的 landdata 或分片表不一致，后面的
//! 逐位比较都没有意义，而错误往往以“某个 rank 的 patch 数不对”这种间接形式出现。
//!
//! 这里只用 `colm-spmd` 的字节接口：不碰 MPI，也不做每步交换。每步交换接在
//! `spatial::runtime::SpatialRuntime` 上，是下一层的事。

use std::path::Path;

use anyhow::{bail, ensure, Context, Result};

use colm_spmd::{decode_u64s, encode_u64s, Comms, RankInfo};

use crate::spatial::partition::{BlockAssignment, BlockInventory, RankBlocks};

/// 计划表的格式版本；字段布局改了必须加一，否则混跑新旧版本会静默错位。
const PLAN_VERSION: u64 = 1;

/// 一个 rank 的分片计划：块清单、确定性分片表、自己的那一份。
#[derive(Debug, Clone)]
pub struct SpmdPlan {
    info: RankInfo,
    inventory: BlockInventory,
    assignment: BlockAssignment,
}

impl SpmdPlan {
    /// 读 `landdata` 的块清单并分片。rank/size 若都不给，从启动器环境解析。
    pub fn read(
        landdata: &Path,
        year: i32,
        rank: Option<usize>,
        ranks: Option<usize>,
    ) -> Result<Self> {
        let info = RankInfo::resolve(rank, ranks)?;
        let inventory = BlockInventory::read(landdata, year).with_context(|| {
            format!(
                "cannot read the landpatch block inventory under {}",
                landdata.display()
            )
        })?;
        Self::from_inventory(inventory, info)
    }

    /// 已有清单时直接分片（测试与需要复用清单的调用方用）。
    pub fn from_inventory(inventory: BlockInventory, info: RankInfo) -> Result<Self> {
        let assignment = inventory.split(info.size())?;
        Ok(Self {
            info,
            inventory,
            assignment,
        })
    }

    pub fn info(&self) -> RankInfo {
        self.info
    }

    pub fn rank(&self) -> usize {
        self.info.rank()
    }

    pub fn size(&self) -> usize {
        self.info.size()
    }

    pub fn is_root(&self) -> bool {
        self.info.is_root()
    }

    pub fn inventory(&self) -> &BlockInventory {
        &self.inventory
    }

    pub fn assignment(&self) -> &BlockAssignment {
        &self.assignment
    }

    /// 本 rank 分到的块与全局 patch 起点。
    pub fn local(&self) -> &RankBlocks {
        self.assignment
            .rank(self.info.rank())
            .expect("the block split covers every rank")
    }

    pub fn local_block_names(&self) -> Vec<&str> {
        self.local_block_names_of(self.info.rank())
    }

    /// 指定 rank 分到的块名，按全局顺序。root 的启动期握手用它算期望值。
    pub fn local_block_names_of(&self, rank: usize) -> Vec<&str> {
        self.assignment
            .rank(rank)
            .expect("the block split covers every rank")
            .blocks()
            .iter()
            .map(|&index| self.inventory.entries()[index].name.as_str())
            .collect()
    }

    pub fn local_global_offsets(&self) -> &[usize] {
        self.local().global_offsets()
    }

    pub fn local_patch_count(&self) -> usize {
        self.local().patch_count()
    }

    /// 本 rank 计划表的 u64 序列；root 用同一函数算每个 rank 的期望值。
    fn plan_values(&self, rank: usize) -> Vec<u64> {
        let blocks = self
            .assignment
            .rank(rank)
            .expect("the block split covers every rank");
        let mut values = vec![
            PLAN_VERSION,
            u64::try_from(rank).expect("rank fits in u64"),
            u64::try_from(self.inventory.len()).expect("block count fits in u64"),
            u64::try_from(self.inventory.total_patches()).expect("patch count fits in u64"),
            inventory_fingerprint(&self.inventory),
            u64::try_from(blocks.blocks().len()).expect("block count fits in u64"),
            u64::try_from(blocks.patch_count()).expect("patch count fits in u64"),
        ];
        values.extend(
            blocks
                .blocks()
                .iter()
                .map(|&index| u64::try_from(index).expect("block index fits in u64")),
        );
        values.extend(
            blocks
                .global_offsets()
                .iter()
                .map(|&offset| u64::try_from(offset).expect("patch offset fits in u64")),
        );
        values
    }

    /// 本 rank 计划表的字节串，用于启动期一致性握手。
    pub fn encode(&self) -> Vec<u8> {
        encode_u64s(&self.plan_values(self.rank()))
    }
}

/// 分片计划 + 已建立的 rank 连接。启动一致性检查通过后才返回。
pub struct SpmdSession {
    plan: SpmdPlan,
    comms: Comms,
}

impl SpmdSession {
    /// 读清单、分片、建连，并让 root 逐 rank 比对计划表。
    pub fn open(
        landdata: &Path,
        year: i32,
        rank: Option<usize>,
        ranks: Option<usize>,
        rendezvous: &Path,
        advertise_host: Option<&str>,
    ) -> Result<Self> {
        let plan = SpmdPlan::read(landdata, year, rank, ranks)?;
        Self::open_with_plan(plan, rendezvous, advertise_host)
    }

    /// 已算出计划表时打开会话（测试用；生产路径走 [`SpmdSession::open`]）。
    pub(crate) fn open_with_plan(
        plan: SpmdPlan,
        rendezvous: &Path,
        advertise_host: Option<&str>,
    ) -> Result<Self> {
        let mut comms = Comms::open(rendezvous, plan.info(), advertise_host)?;
        check_plan_agreement(&plan, &mut comms)?;
        Ok(Self { plan, comms })
    }

    pub fn plan(&self) -> &SpmdPlan {
        &self.plan
    }

    pub fn comms_mut(&mut self) -> &mut Comms {
        &mut self.comms
    }

    pub fn into_parts(self) -> (SpmdPlan, Comms) {
        (self.plan, self.comms)
    }
}

/// root 收齐各 rank 的计划表逐项比对；任何不一致都先广播错误再返回，避免 worker 空等。
fn check_plan_agreement(plan: &SpmdPlan, comms: &mut Comms) -> Result<()> {
    let gathered = comms.gather(&plan.encode())?;
    if plan.is_root() {
        let gathered = gathered.context("root gather returned no plan bytes")?;
        let result = compare_plans(plan, &gathered);
        match result {
            Ok(()) => {
                comms.broadcast(Some(&[]))?;
                Ok(())
            }
            Err(error) => {
                // 先告诉 worker 失败原因；它们收到非空状态就返回错误，不会等到超时。
                let message = format!("{error:#}");
                let _ = comms.broadcast(Some(message.as_bytes()));
                Err(error)
            }
        }
    } else {
        let status = comms.broadcast(None)?;
        ensure!(
            status.is_empty(),
            "rank 0 rejected the block plan: {}",
            String::from_utf8_lossy(&status)
        );
        Ok(())
    }
}

fn compare_plans(plan: &SpmdPlan, gathered: &[u8]) -> Result<()> {
    let values = decode_u64s(gathered).context("the gathered plan is not a u64 sequence")?;
    let mut offset = 0;
    for rank in 0..plan.size() {
        ensure!(
            offset + 7 <= values.len(),
            "rank {rank} plan is truncated at value {offset} of {}",
            values.len()
        );
        let blocks = usize::try_from(values[offset + 5])
            .context("the block count in a plan does not fit in usize")?;
        let length = 7 + 2 * blocks;
        ensure!(
            offset + length <= values.len(),
            "rank {rank} plan needs {length} values but only {} remain",
            values.len() - offset
        );
        let chunk = &values[offset..offset + length];
        offset += length;
        let expected = plan.plan_values(rank);
        if chunk != expected.as_slice() {
            bail!("block plan mismatch on rank {rank}: got {chunk:?}, expected {expected:?}");
        }
    }
    ensure!(
        offset == values.len(),
        "the gathered plan has {} extra values",
        values.len() - offset
    );
    Ok(())
}

/// 块清单指纹：块名 + patch 数，FNV-1a 64 位。只用于发现“各 rank 看到不同 landdata”，
/// 不做安全用途。
fn inventory_fingerprint(inventory: &BlockInventory) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for entry in inventory.entries() {
        for byte in entry.name.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= u64::try_from(entry.patches).expect("patch count fits in u64");
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
#[path = "spmd_tests.rs"]
mod spmd_tests;

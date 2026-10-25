//! 多进程每步的 patch 数组 all-gather。
//!
//! 分片是**全局连续的块区间**（[`super::partition::BlockInventory::split`]），所以各 rank 的
//! 局部 patch 数组按 rank 顺序拼起来就是全局 patch 顺序：这里不做全局索引重排，也不要求调用方
//! 知道别的 rank 的块。root 先 gather 再 broadcast，所有 rank 拿到同一份全局数组。
//!
//! 只传 byte 接口；`f64`/`bool` 的帧编码在 `colm-spmd` 里。0.1° 全球的每步数组约 0.3 GB，
//! 实测 tianhe 两节点 TCP 40 Gb/s，gather+broadcast 约 0.12 s，相对每步 5–11 s 可接受。

use anyhow::{ensure, Context, Result};

use colm_spmd::{decode_bools, decode_f64s, encode_bools, encode_f64s, Comms};

/// 一个 rank 的 patch 数组交换端。
pub struct PatchExchange {
    comms: Comms,
    patch_offset: usize,
    local_patches: usize,
    global_patches: usize,
}

impl PatchExchange {
    /// `patch_offset` 是本 rank 第一个 patch 的全局下标；连续分片下它等于前面所有 rank 的 patch 数。
    pub fn new(
        comms: Comms,
        patch_offset: usize,
        local_patches: usize,
        global_patches: usize,
    ) -> Result<Self> {
        ensure!(
            patch_offset
                .checked_add(local_patches)
                .is_some_and(|end| end <= global_patches),
            "rank patch range {patch_offset}..{} exceeds the global patch count {global_patches}",
            patch_offset + local_patches
        );
        ensure!(
            global_patches >= local_patches,
            "a rank has {local_patches} patches but the global count is only {global_patches}"
        );
        Ok(Self {
            comms,
            patch_offset,
            local_patches,
            global_patches,
        })
    }

    pub fn patch_offset(&self) -> usize {
        self.patch_offset
    }

    pub fn local_patches(&self) -> usize {
        self.local_patches
    }

    pub fn global_patches(&self) -> usize {
        self.global_patches
    }

    /// 所有 rank 的 `f64` 数组按全局 patch 顺序拼成一份，所有 rank 都拿到同样的结果。
    pub fn all_gather_f64(&mut self, local: &[f64]) -> Result<Vec<f64>> {
        ensure!(
            local.len() == self.local_patches,
            "all_gather_f64 got {} values for {} local patches",
            local.len(),
            self.local_patches
        );
        let gathered = self.comms.gather(&encode_f64s(local))?;
        let global = self.comms.broadcast(gathered.as_deref())?;
        let values = decode_f64s(&global).context("a gathered f64 array is malformed")?;
        ensure!(
            values.len() == self.global_patches,
            "all-gather returned {} f64 values, expected {}",
            values.len(),
            self.global_patches
        );
        Ok(values)
    }

    /// 同 [`PatchExchange::all_gather_f64`]，用于 `included`/掩膜这类布尔数组。
    pub fn all_gather_bools(&mut self, local: &[bool]) -> Result<Vec<bool>> {
        ensure!(
            local.len() == self.local_patches,
            "all_gather_bools got {} values for {} local patches",
            local.len(),
            self.local_patches
        );
        let gathered = self.comms.gather(&encode_bools(local))?;
        let global = self.comms.broadcast(gathered.as_deref())?;
        let values = decode_bools(&global).context("a gathered bool array is malformed")?;
        ensure!(
            values.len() == self.global_patches,
            "all-gather returned {} bool values, expected {}",
            values.len(),
            self.global_patches
        );
        Ok(values)
    }

    /// 不定长字节数组 all-gather：调用方自己带长度前缀（history 累加器用）。
    pub fn all_gather_bytes(&mut self, local: &[u8]) -> Result<Vec<u8>> {
        let gathered = self.comms.gather(local)?;
        self.comms.broadcast(gathered.as_deref())
    }

    pub fn rank(&self) -> usize {
        self.comms.rank()
    }

    pub fn size(&self) -> usize {
        self.comms.size()
    }

    pub fn is_root(&self) -> bool {
        self.comms.is_root()
    }

    pub fn comms_mut(&mut self) -> &mut Comms {
        &mut self.comms
    }

    pub fn into_comms(self) -> Comms {
        self.comms
    }
}

#[cfg(test)]
#[path = "exchange_tests.rs"]
mod exchange_tests;

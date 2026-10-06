//! 逐 patch 写均值的写入清单。
//!
//! 每条记录要把每个 patch 的每个变量写进缓冲区。直接调 [`HistoryBuffers::set_patch_scalar`] 等
//! 方法时，每次都按名字查表，而且只能在主线程串行做；高分辨率、高频输出时这一段比写盘还慢
//! （第 602 轮实测）。清单把这一段拆成两步：
//!
//! 1. [`HistoryBuffers::patch_writes`] 只读缓冲，可以按 patch 并行生成。名字在这一步解析成下标，
//!    长度、记录号等检查也在这一步做，报错与直接写相同。
//! 2. [`HistoryBuffers::apply`] 在主线程按 patch 顺序把清单拷进缓冲，不再查名字。
//!
//! 同一 patch 内按调用顺序、各 patch 按次序落盘，与逐个直接写的结果逐位相同。

use anyhow::{ensure, Context, Result};

use super::HistoryBuffers;

#[derive(Debug, Clone, Copy)]
enum Op {
    /// 选中的 patch 计入第 `slot` 个变量的 `filter`。
    Include(usize),
    /// 本条记录的 `nac`。
    Steps(f64),
    /// 把 `data[offset..offset + len]` 拷到第 `slot` 个变量的 `values[start..]`。
    Set {
        slot: usize,
        start: usize,
        offset: usize,
        len: usize,
    },
}

/// 一条记录、一个 patch（或全部 patch）的写入清单。
#[derive(Debug)]
pub struct PatchWrites<'a> {
    buffers: &'a HistoryBuffers,
    record: usize,
    selected: Option<usize>,
    ops: Vec<Op>,
    data: Vec<f64>,
}

/// 生成完的清单，交给 [`HistoryBuffers::apply`]。
#[derive(Debug)]
pub struct PatchOps {
    record: usize,
    selected: Option<usize>,
    ops: Vec<Op>,
    data: Vec<f64>,
}

impl HistoryBuffers {
    /// 为第 `record` 条记录开一份写入清单。`selected` 同 [`Self::select_patch`]：`Some(patch)` 只写这一格，
    /// `None` 一次写全部 patch。
    pub fn patch_writes(&self, record: usize, selected: Option<usize>) -> Result<PatchWrites<'_>> {
        ensure!(
            record < self.records,
            "record {record} is outside the {}-record group",
            self.records
        );
        if let Some(patch) = selected {
            ensure!(
                patch < self.dims.patch,
                "patch {patch} is outside the {} history patches",
                self.dims.patch
            );
        }
        Ok(PatchWrites {
            buffers: self,
            record,
            selected,
            ops: Vec::new(),
            data: Vec::new(),
        })
    }

    /// 把一份清单拷进缓冲。
    pub fn apply(&mut self, writes: PatchOps) -> Result<()> {
        let PatchOps {
            record,
            selected,
            ops,
            data,
        } = writes;
        ensure!(
            record < self.records,
            "record {record} is outside the {}-record group",
            self.records
        );
        let (first, count) = match selected {
            Some(patch) => (patch, 1),
            None => (0, self.dims.patch),
        };
        let patches = self.dims.patch;
        for op in ops {
            match op {
                Op::Include(slot) => {
                    let start = record * patches + first;
                    self.slot_mut(slot).included[start..start + count].fill(true);
                }
                Op::Steps(steps) => self.steps[record] = steps,
                Op::Set {
                    slot,
                    start,
                    offset,
                    len,
                } => self.slot_mut(slot).values[start..start + len]
                    .copy_from_slice(&data[offset..offset + len]),
            }
        }
        Ok(())
    }
}

impl PatchWrites<'_> {
    pub fn finish(self) -> PatchOps {
        PatchOps {
            record: self.record,
            selected: self.selected,
            ops: self.ops,
            data: self.data,
        }
    }

    pub fn declares(&self, name: &str) -> bool {
        self.buffers.declares(name)
    }

    pub fn wants_raw_layers(&self, name: &str) -> bool {
        self.buffers.wants_raw_layers(name)
    }

    fn span(&self) -> (usize, usize) {
        match self.selected {
            Some(patch) => (patch, 1),
            None => (0, self.buffers.dims.patch),
        }
    }

    fn check_record(&self, record: usize) -> Result<()> {
        ensure!(
            record == self.record,
            "this patch writer is for record {}, not {record}",
            self.record
        );
        Ok(())
    }

    /// 同 [`HistoryBuffers::include`]。
    pub fn include(&mut self, name: &str, record: usize) -> Result<()> {
        if self.buffers.deselected(name) {
            return Ok(());
        }
        self.check_record(record)?;
        let slot = self
            .buffers
            .slot_of(name)
            .with_context(|| format!("{name} was not declared with declare()"))?;
        self.ops.push(Op::Include(slot));
        Ok(())
    }

    /// 同 [`HistoryBuffers::set_steps`]。
    pub fn set_steps(&mut self, record: usize, steps: f64) -> Result<()> {
        self.check_record(record)?;
        self.ops.push(Op::Steps(steps));
        Ok(())
    }

    /// 同 [`HistoryBuffers::set_patch_scalar`]。
    pub fn set_patch_scalar(&mut self, name: &str, record: usize, value: f64) -> Result<()> {
        if self.buffers.deselected(name) {
            return Ok(());
        }
        let layers = self.buffers.layers_of(name)?;
        ensure!(
            layers == 1,
            "{name} has {layers} values per patch; use set_layered"
        );
        self.set_layered(name, record, &[value])
    }

    /// 同 [`HistoryBuffers::set_layered`]。
    pub fn set_layered(&mut self, name: &str, record: usize, values: &[f64]) -> Result<()> {
        if self.buffers.deselected(name) {
            return Ok(());
        }
        let slot = self
            .buffers
            .slot_of(name)
            .with_context(|| format!("{name} was not declared with declare()"))?;
        let layers = self.buffers.slot(slot).layers;
        let (first, count) = self.span();
        ensure!(
            values.len() == count * layers,
            "{name} needs {} values (patch × layers), got {}",
            count * layers,
            values.len()
        );
        self.check_record(record)?;
        self.ops.push(Op::Set {
            slot,
            start: (record * self.buffers.dims.patch + first) * layers,
            offset: self.data.len(),
            len: values.len(),
        });
        self.data.extend_from_slice(values);
        Ok(())
    }
}

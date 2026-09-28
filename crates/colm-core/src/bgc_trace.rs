//! BGC 逐阶段追踪：与 `oracle/scripts/gen_bgc_trace.py` 插桩的 Fortran driver 同格式。
//!
//! 一条记录 = 32 字节阶段名（空格补齐）+ 两段字段表：先是物理输入（driver 实参与 BGC 读写的
//! 非 BGC 变量），再是 BGC 状态（`MOD_BGC_Vars_*` 全部数组，按声明顺序）。每个字段是
//! `[i32 名长][名][i32 n][n × f64]`，名长 0 表示该段结束；整数、逻辑量转成 f64（逻辑 1/0）。
//!
//! 用途有二：Rust 引擎写同样的记录，`bgc_trace_cmp.py` 找第一个分叉；以及逐过程回放——
//! 拿 Fortran 某阶段之前的记录当输入、只跑这一个过程，再与之后的记录逐位比，每个过程都能
//! 脱离上游误差单独验证。

use std::io::{self, Write};

use anyhow::{bail, ensure, Context, Result};

use crate::bgc_state::{
    BgcPatchFluxes, BgcPatchTimeInvariants, BgcPatchTimeVariables, BgcPftFluxes,
    BgcPftTimeVariables, BgcState,
};

/// 一条追踪记录。
#[derive(Debug, Clone, PartialEq)]
pub struct TraceRecord {
    pub tag: String,
    pub inputs: Vec<(String, Vec<f64>)>,
    pub state: Vec<(String, Vec<f64>)>,
}

impl TraceRecord {
    pub fn input(&self, name: &str) -> Option<&[f64]> {
        self.inputs
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, values)| values.as_slice())
    }
}

/// 解析整份追踪文件。
pub fn read_trace(bytes: &[u8]) -> Result<Vec<TraceRecord>> {
    let mut pos = 0;
    let mut records = Vec::new();
    while pos < bytes.len() {
        ensure!(bytes.len() >= pos + 32, "truncated trace tag at byte {pos}");
        let tag = std::str::from_utf8(&bytes[pos..pos + 32])
            .context("trace tag is not UTF-8")?
            .trim_end()
            .to_string();
        pos += 32;
        let inputs = read_section(bytes, &mut pos)?;
        let state = read_section(bytes, &mut pos)?;
        records.push(TraceRecord { tag, inputs, state });
    }
    Ok(records)
}

fn read_i32(bytes: &[u8], pos: &mut usize) -> Result<i32> {
    let raw = bytes
        .get(*pos..*pos + 4)
        .with_context(|| format!("truncated trace at byte {pos}"))?;
    *pos += 4;
    Ok(i32::from_le_bytes(raw.try_into().expect("four bytes")))
}

fn read_section(bytes: &[u8], pos: &mut usize) -> Result<Vec<(String, Vec<f64>)>> {
    let mut fields = Vec::new();
    loop {
        let length = usize::try_from(read_i32(bytes, pos)?).context("negative name length")?;
        if length == 0 {
            return Ok(fields);
        }
        let name = bytes
            .get(*pos..*pos + length)
            .context("truncated field name")?;
        let name = std::str::from_utf8(name)
            .context("field name is not UTF-8")?
            .to_string();
        *pos += length;
        let n = usize::try_from(read_i32(bytes, pos)?).context("negative field length")?;
        let raw = bytes
            .get(*pos..*pos + 8 * n)
            .with_context(|| format!("truncated values of {name}"))?;
        *pos += 8 * n;
        let values = raw
            .chunks_exact(8)
            .map(|chunk| f64::from_le_bytes(chunk.try_into().expect("eight bytes")))
            .collect();
        fields.push((name, values));
    }
}

/// 写一条记录。`inputs` 由调用方按 Fortran 的物理量名给出。
pub fn write_record(
    out: &mut impl Write,
    tag: &str,
    inputs: &[(&str, Vec<f64>)],
    state: &BgcState,
) -> io::Result<()> {
    let mut label = [b' '; 32];
    let tag = &tag.as_bytes()[..tag.len().min(32)];
    label[..tag.len()].copy_from_slice(tag);
    out.write_all(&label)?;
    for (name, values) in inputs {
        write_field(out, name, values)?;
    }
    out.write_all(&0_i32.to_le_bytes())?;
    for (name, values) in state.trace_fields() {
        write_field(out, name, &values)?;
    }
    out.write_all(&0_i32.to_le_bytes())
}

fn write_field(out: &mut impl Write, name: &str, values: &[f64]) -> io::Result<()> {
    let length = i32::try_from(name.len()).expect("short field name");
    let n = i32::try_from(values.len()).expect("trace field fits i32");
    out.write_all(&length.to_le_bytes())?;
    out.write_all(name.as_bytes())?;
    out.write_all(&n.to_le_bytes())?;
    for value in values {
        out.write_all(&value.to_le_bytes())?;
    }
    Ok(())
}

/// 五组结构的字段表，顺序与 Fortran 插桩（`gen_bgc_state.MODULES`）一致。
const GROUPS: [&[(&str, &[&str])]; 5] = [
    BgcPftTimeVariables::FIELDS,
    BgcPftFluxes::FIELDS,
    BgcPatchTimeVariables::FIELDS,
    BgcPatchFluxes::FIELDS,
    BgcPatchTimeInvariants::FIELDS,
];

impl BgcState {
    /// 全部数组按追踪顺序转成 f64。
    pub fn trace_fields(&self) -> Vec<(&'static str, Vec<f64>)> {
        let mut out = Vec::new();
        for group in GROUPS {
            for (name, _) in group {
                let values = if let Some(values) = self.f64_field(name) {
                    values.clone()
                } else if let Some(values) = self.i32_field(name) {
                    values.iter().map(|value| f64::from(*value)).collect()
                } else if let Some(values) = self.bool_field(name) {
                    values
                        .iter()
                        .map(|value| f64::from(u8::from(*value)))
                        .collect()
                } else {
                    unreachable!("{name} is listed in FIELDS but has no accessor")
                };
                out.push((*name, values));
            }
        }
        out
    }

    /// 用追踪记录覆盖状态。n=0 的字段（Fortran 未分配或宏关闭）保持原值。
    pub fn load_trace_fields(&mut self, fields: &[(String, Vec<f64>)]) -> Result<()> {
        for (name, values) in fields {
            if values.is_empty() {
                continue;
            }
            if let Some(slot) = self.f64_field_mut(name) {
                ensure!(
                    slot.len() == values.len(),
                    "{name}: trace length {} vs state {}",
                    values.len(),
                    slot.len()
                );
                slot.copy_from_slice(values);
            } else if let Some(slot) = self.i32_field_mut(name) {
                ensure!(slot.len() == values.len(), "{name}: trace length differs");
                for (slot, value) in slot.iter_mut().zip(values) {
                    *slot = *value as i32;
                }
            } else if let Some(slot) = self.bool_field_mut(name) {
                ensure!(slot.len() == values.len(), "{name}: trace length differs");
                for (slot, value) in slot.iter_mut().zip(values) {
                    *slot = *value != 0.0;
                }
            } else {
                bail!("trace field {name} is not part of the BGC state");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "bgc_trace_tests.rs"]
mod bgc_trace_tests;

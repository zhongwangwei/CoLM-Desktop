//! 混合模型插槽在引擎里的接入点（`docs/design-hybrid.md` 第 4 节）。
//!
//! 两个参数插槽，都在装配**之前**完成：网络给出参数覆盖，装配照原路径从覆盖值派生一切，物理代码不改。
//!
//! - `land_class`（LCT）：每个土壤 patch 一行，输出地类表的 `DEF_LC_*` 列，作为这个 patch 的
//!   [`colm_core::LandClassOverrides`]。PFT/PC 下地类表不起作用，配置了就报错。
//! - `pft`（PFT/PC）：每个土壤 patch 的每个 PFT 一行，输出 `DEF_PFT_*` 参数，作为这个 PFT 的覆盖
//!   （`LandPhysicsParameters::pft_overrides`，在 `crate::pft` 里走与 namelist `DEF_PFT_*(class)` 相同的查表
//!   路径）。LCT 下没有 PFT，配置了就报错。
//!
//! 特征按名字从常数重启读：`name` 是 `(patch,)` 量（整数变量如 `patchclass` 也行），`name[k]` 是
//! `(patch, 层)` 量的第 `k` 层（1 起）。`pft` 插槽另外先在 PFT 常数重启里找 `(pft,)` 量（如 `pftclass`、
//! `pftfrac`、`htop_p`）。只作用于土壤 patch（`patchtype == 0`）。把有效值原样写回与不覆盖逐位相同，所以
//! "模仿物理"的模型给出与纯物理逐位相同的结果。

use std::collections::BTreeMap;
use std::io::Write;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use anyhow::{bail, ensure, Context, Result};
use colm_hybrid::{HybridConfig, Matrix, Slot, SlotConfig, SlotKind, Surrogate};

use crate::assembly::LandPhysicsParameters;

/// 引擎认得的参数插槽名。
pub const LAND_CLASS_SLOT: &str = "land_class";
pub const PFT_SLOT: &str = "pft";

/// 加载好的混合设置。
pub struct Hybrid {
    pub config: HybridConfig,
    land_class: Option<Slot>,
    pft: Option<Slot>,
}

fn check_param(config: &SlotConfig) -> Result<()> {
    ensure!(
        config.kind == SlotKind::Param,
        "slot {} is a param slot, not {:?}",
        config.name,
        config.kind
    );
    Ok(())
}

fn known_slot(config: &SlotConfig) -> Result<()> {
    match config.name.as_str() {
        LAND_CLASS_SLOT => {
            check_param(config)?;
            for output in &config.outputs {
                ensure!(
                    colm_core::LandClassOverrides::REAL_NAMES.contains(&output.name.as_str()),
                    "slot {LAND_CLASS_SLOT} output {} is not one of the DEF_LC_* land-class columns",
                    output.name
                );
            }
            Ok(())
        }
        PFT_SLOT => {
            check_param(config)?;
            for output in &config.outputs {
                ensure!(
                    colm_case::pft::is_parameter(&output.name),
                    "slot {PFT_SLOT} output {} is not a DEF_PFT_* parameter",
                    output.name
                );
            }
            Ok(())
        }
        other => bail!(
            "slot {other} is not available in this engine (it provides {LAND_CLASS_SLOT} and {PFT_SLOT})"
        ),
    }
}

impl Hybrid {
    /// 正式运行：校验插槽并加载模型。
    pub fn load(path: &Path) -> Result<Self> {
        let config = HybridConfig::load(path)?;
        Self::build(config, |slot| Slot::load(slot.clone()))
    }

    /// 用给定后端代替模型文件（测试与"模仿物理"检查）。
    pub fn with_backend(config: HybridConfig, backend: Arc<dyn Surrogate>) -> Result<Self> {
        Self::build(config, |slot| {
            Slot::with_backend(slot.clone(), Arc::clone(&backend))
        })
    }

    fn build(config: HybridConfig, load: impl Fn(&SlotConfig) -> Result<Slot>) -> Result<Self> {
        let (mut land_class, mut pft) = (None, None);
        for slot in &config.slots {
            known_slot(slot)?;
            match slot.name.as_str() {
                LAND_CLASS_SLOT => land_class = Some(load(slot)?),
                _ => pft = Some(load(slot)?),
            }
        }
        Ok(Self {
            config,
            land_class,
            pft,
        })
    }

    /// 各 patch 装配用的物理参数，次序与 `patches` 相同。`pft_ranges` 是各 patch 在 PFT 常数重启里的
    /// PFT 区间（与 `patches` 对齐；非土壤 patch 为空区间），只有 `pft` 插槽要它。
    pub fn patch_physics(
        &self,
        constant: &Path,
        patches: &[usize],
        pft_ranges: &[Range<usize>],
        physics: &LandPhysicsParameters,
    ) -> Result<Vec<LandPhysicsParameters>> {
        let mut out = vec![physics.clone(); patches.len()];
        if self.land_class.is_none() && self.pft.is_none() {
            return Ok(out);
        }
        let restart = colm_init::RestartFile::open(constant)?;
        let soil = soil_rows(&restart, patches)?;
        if let Some(slot) = &self.land_class {
            ensure!(
                !physics.use_pft,
                "slot {LAND_CLASS_SLOT} overrides land-class table columns, which only drive LCT \
                 soil patches; this case uses PFT/PC (use the {PFT_SLOT} slot)"
            );
            let soil_patches: Vec<usize> = soil.iter().map(|&row| patches[row]).collect();
            let features = feature_matrix(&restart, &slot.config.features, &soil_patches)?;
            let values = slot.evaluate(&features)?;
            for (index, &row) in soil.iter().enumerate() {
                let overrides = &mut out[row].land_class_overrides;
                for (output, &value) in slot.config.outputs.iter().zip(values.row(index)) {
                    overrides.set_real(&output.name, value)?;
                }
            }
        }
        if let Some(slot) = &self.pft {
            ensure!(
                physics.use_pft,
                "slot {PFT_SLOT} overrides PFT parameters, but this LCT case has no PFTs (use the \
                 {LAND_CLASS_SLOT} slot)"
            );
            ensure!(
                pft_ranges.len() == patches.len(),
                "{} PFT ranges for {} patches",
                pft_ranges.len(),
                patches.len()
            );
            let pft_restart =
                colm_init::RestartFile::open(crate::pft::pft_restart_path(constant)?)?;
            let rows = pft_rows(&soil, patches, pft_ranges);
            let features =
                pft_feature_matrix(&restart, &pft_restart, &slot.config.features, &rows)?;
            let values = slot.evaluate(&features)?;
            for (index, &(row, _, _)) in rows.iter().enumerate() {
                let mut overrides = BTreeMap::new();
                for (output, &value) in slot.config.outputs.iter().zip(values.row(index)) {
                    colm_case::pft::validate_override(&output.name, value)
                        .with_context(|| format!("slot {PFT_SLOT} row {index}"))?;
                    overrides.insert(output.name.clone(), value);
                }
                out[row].pft_overrides.push(overrides);
            }
        }
        Ok(out)
    }
}

/// `patches` 里土壤 patch（`patchtype == 0`）的行号。
fn soil_rows(restart: &colm_init::RestartFile, patches: &[usize]) -> Result<Vec<usize>> {
    patches
        .iter()
        .enumerate()
        .filter_map(
            |(row, &patch)| match patch_integer(restart, "patchtype", patch) {
                Ok(0) => Some(Ok(row)),
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            },
        )
        .collect()
}

/// `pft` 插槽的行：`(patches 里的行号, patch, PFT 在 PFT 常数重启里的下标)`，按 patch、再按 PFT 次序。
fn pft_rows(
    soil: &[usize],
    patches: &[usize],
    pft_ranges: &[Range<usize>],
) -> Vec<(usize, usize, usize)> {
    soil.iter()
        .flat_map(|&row| {
            pft_ranges[row]
                .clone()
                .map(move |pft| (row, patches[row], pft))
        })
        .collect()
}

/// PFT 常数重启里 `(pft,)` 量的第 `pft` 个值；没有这个量时返回 `None`（改按 patch 取）。
fn pft_value(restart: &colm_init::RestartFile, name: &str, pft: usize) -> Result<Option<f64>> {
    if !restart.contains(name) || restart.variable_dimensions(name)? != ["pft"] {
        return Ok(None);
    }
    let value = if restart.integer_names().iter().any(|n| n == name) {
        restart.integers(name)?.get(pft).map(|&v| v as f64)
    } else {
        restart.floats(name)?.get(pft).copied()
    };
    value
        .map(Some)
        .with_context(|| format!("{name} has no PFT {pft}"))
}

fn pft_feature_matrix(
    restart: &colm_init::RestartFile,
    pft_restart: &colm_init::RestartFile,
    features: &[String],
    rows: &[(usize, usize, usize)],
) -> Result<Matrix> {
    let mut data = Vec::with_capacity(rows.len() * features.len());
    for &(_, patch, pft) in rows {
        for feature in features {
            data.push(match pft_value(pft_restart, feature, pft)? {
                Some(value) => value,
                None => feature_value(restart, feature, patch)?,
            });
        }
    }
    Matrix::new(rows.len(), features.len(), data)
}

/// `(patch,)` 整数量的第 `patch` 个值。
fn patch_integer(restart: &colm_init::RestartFile, name: &str, patch: usize) -> Result<i64> {
    let dims = restart.variable_dimensions(name)?;
    ensure!(
        dims == ["patch"],
        "{name} should be a (patch,) field, but it is {dims:?}"
    );
    restart
        .integers(name)?
        .get(patch)
        .copied()
        .with_context(|| format!("{name} has no patch {patch}"))
}

/// 特征名 → `(变量名, 层下标 0 起)`；`name[k]` 的 `k` 从 1 起。
fn parse_feature(feature: &str) -> Result<(&str, Option<usize>)> {
    match feature.split_once('[') {
        None => Ok((feature, None)),
        Some((name, rest)) => {
            let layer: usize = rest
                .strip_suffix(']')
                .and_then(|k| k.parse().ok())
                .with_context(|| format!("feature {feature} is not name[k]"))?;
            ensure!(layer >= 1, "feature {feature}: layers count from 1");
            Ok((name, Some(layer - 1)))
        }
    }
}

/// 从常数重启读一个特征在第 `patch` 个 patch 上的值。
fn feature_value(restart: &colm_init::RestartFile, feature: &str, patch: usize) -> Result<f64> {
    let (name, layer) = parse_feature(feature)?;
    if restart.integer_names().iter().any(|n| n == name) {
        ensure!(
            layer.is_none(),
            "feature {feature}: integer variables have no layers"
        );
        return Ok(patch_integer(restart, name, patch)? as f64);
    }
    let block = restart
        .patch_block(name, patch)
        .with_context(|| format!("feature {feature}"))?;
    match layer {
        None => {
            ensure!(
                block.len() == 1,
                "feature {feature} has {} values per patch; name a layer as {name}[k]",
                block.len()
            );
            Ok(block[0])
        }
        Some(layer) => block
            .get(layer)
            .copied()
            .with_context(|| format!("feature {feature}: {name} has {} layers", block.len())),
    }
}

fn feature_matrix(
    restart: &colm_init::RestartFile,
    features: &[String],
    patches: &[usize],
) -> Result<Matrix> {
    let mut data = Vec::with_capacity(patches.len() * features.len());
    for &patch in patches {
        for feature in features {
            data.push(feature_value(restart, feature, patch)?);
        }
    }
    Matrix::new(patches.len(), features.len(), data)
}

/// 特征抓取（训练前准备数据）：按配置把每一行（土壤 patch，`pft` 插槽是每个 PFT）的特征与物理值写成
/// CSV，不改任何参数。一次抓取只对应一个插槽。
///
/// 列：`patch`、`pft`（`land_class` 插槽为空）、各特征、各输出的物理值（`physics:<名字>`；地类表值或
/// PFT 参数的有效值）。追加写，首次写表头。
#[allow(clippy::too_many_arguments)]
pub fn write_tap(
    config: &HybridConfig,
    constant: &Path,
    patches: &[usize],
    pft_ranges: &[Range<usize>],
    physics: &LandPhysicsParameters,
    document: &colm_namelist::Document,
    out: &Path,
) -> Result<()> {
    let [slot] = config.slots.as_slice() else {
        bail!(
            "--hybrid-tap records one slot at a time, but {} declares {}",
            config.path.display(),
            config.slots.len()
        );
    };
    known_slot(slot)?;
    let restart = colm_init::RestartFile::open(constant)?;
    let soil = soil_rows(&restart, patches)?;
    let new_file = !out.exists();
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(out)
        .with_context(|| format!("cannot open {}", out.display()))?;
    if new_file {
        let mut header = vec!["patch".to_owned(), "pft".to_owned()];
        header.extend(slot.features.iter().cloned());
        header.extend(
            slot.outputs
                .iter()
                .map(|output| format!("physics:{}", output.name)),
        );
        writeln!(file, "{}", header.join(","))?;
    }
    if slot.name == LAND_CLASS_SLOT {
        for &row in &soil {
            let patch = patches[row];
            let class = usize::try_from(patch_integer(&restart, "patchclass", patch)?)
                .context("patchclass is negative")?;
            let constants = colm_core::ClassConstants::new(physics.land_cover_scheme, class)?;
            let mut line = vec![patch.to_string(), String::new()];
            for feature in &slot.features {
                line.push(format!("{:e}", feature_value(&restart, feature, patch)?));
            }
            for output in &slot.outputs {
                line.push(format!("{:e}", constants.table_value(&output.name)?));
            }
            writeln!(file, "{}", line.join(","))?;
        }
        return Ok(());
    }
    ensure!(
        physics.use_pft,
        "slot {PFT_SLOT} needs a PFT/PC case; this one is LCT"
    );
    ensure!(
        pft_ranges.len() == patches.len(),
        "{} PFT ranges for {} patches",
        pft_ranges.len(),
        patches.len()
    );
    let pft_restart = colm_init::RestartFile::open(crate::pft::pft_restart_path(constant)?)?;
    let campbell = physics.hydraulic_model == colm_core::HydraulicModel::Campbell;
    for (_, patch, pft) in pft_rows(&soil, patches, pft_ranges) {
        let class = pft_value(&pft_restart, "pftclass", pft)?
            .context("the PFT constant restart has no pftclass")? as i32;
        let mut line = vec![patch.to_string(), pft.to_string()];
        for feature in &slot.features {
            let value = match pft_value(&pft_restart, feature, pft)? {
                Some(value) => value,
                None => feature_value(&restart, feature, patch)?,
            };
            line.push(format!("{value:e}"));
        }
        for output in &slot.outputs {
            let value =
                colm_init::pft_parameter(document, &output.name, class, campbell, physics.use_pc)?;
            line.push(format!("{value:e}"));
        }
        writeln!(file, "{}", line.join(","))?;
    }
    Ok(())
}

/// 续跑文件旁的混合模型标记：`<重启文件>.hybrid`，内容是 [`HybridConfig::fingerprint`]。
pub fn restart_marker(restart: &Path) -> std::path::PathBuf {
    let mut path = restart.as_os_str().to_owned();
    path.push(".hybrid");
    std::path::PathBuf::from(path)
}

/// 写完一份续跑文件后调用：用了混合模型就写标记，没用就删掉可能残留的旧标记（同名文件被纯物理
/// 运行覆盖时，旧标记会说错话）。没用混合模型、也没有旧标记时什么都不做，现有输出不变。
pub fn mark_restart(restart: &Path, fingerprint: Option<&str>) -> Result<()> {
    let marker = restart_marker(restart);
    match fingerprint {
        Some(fingerprint) => std::fs::write(&marker, format!("{fingerprint}\n"))
            .with_context(|| format!("cannot write {}", marker.display())),
        None if marker.exists() => std::fs::remove_file(&marker)
            .with_context(|| format!("cannot remove the stale {}", marker.display())),
        None => Ok(()),
    }
}

/// 开跑前检查初始续跑文件：它是混合运行写出的，这次就必须用同一套模型（指纹相同）。
/// 没有标记（mkinidata 的冷启动、纯物理 spin-up 的结果）时不限制：从纯物理状态接混合模型是正常用法。
pub fn check_restart(restart: &Path, fingerprint: Option<&str>) -> Result<()> {
    let marker = restart_marker(restart);
    if !marker.exists() {
        return Ok(());
    }
    let recorded = std::fs::read_to_string(&marker)
        .with_context(|| format!("cannot read {}", marker.display()))?;
    let recorded = recorded.trim();
    match fingerprint {
        Some(current) if current == recorded => Ok(()),
        Some(current) => bail!(
            "{} was written with hybrid models {recorded}, but this run uses {current}; \
             continuing would mix two models in one trajectory. Restart from a cold or \
             physics-only state, or delete {} if the change is deliberate",
            restart.display(),
            marker.display()
        ),
        None => bail!(
            "{} was written with hybrid models {recorded}, but this run has no hybrid.toml; \
             add the same hybrid.toml back, or delete {} if continuing with physics only is \
             deliberate",
            restart.display(),
            marker.display()
        ),
    }
}

#[cfg(test)]
#[path = "hybrid_tests.rs"]
mod hybrid_tests;

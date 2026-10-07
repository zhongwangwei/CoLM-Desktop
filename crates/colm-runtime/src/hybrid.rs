//! 混合模型插槽在引擎里的接入点（`docs/design-hybrid.md` 第 4 节）。
//!
//! P0 只有一个参数插槽 `land_class`：网络按 patch 给出地类表的若干列（输出名是 `DEF_LC_*`），作为
//! 这个 patch 自己的 [`colm_core::LandClassOverrides`] 带进装配。装配照原路径从覆盖值派生一切
//! （Vcmax 的换算、`d50`/`beta` 改根系分布……），物理代码一行不改。
//!
//! - 特征按名字从常数重启读：`name` 是 `(patch,)` 量（整数变量如 `patchclass` 也行），`name[k]`
//!   是 `(patch, 层)` 量的第 `k` 层（1 起）。
//! - 只作用于土壤 patch（`patchtype == 0`）；其余 patch 不覆盖。
//! - 只支持 LCT：PFT/PC 下土壤 patch 的参数来自 PFT 表，地类表的这些列不起作用，配置了就报错。
//! - 把表值原样写回与不覆盖逐位相同（`ClassConstants::table_value`），所以"模仿物理"的模型
//!   给出与纯物理逐位相同的结果。

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use anyhow::{bail, ensure, Context, Result};
use colm_hybrid::{HybridConfig, Matrix, Slot, SlotConfig, SlotKind, Surrogate};

use crate::assembly::LandPhysicsParameters;

/// 引擎认得的参数插槽名。
pub const LAND_CLASS_SLOT: &str = "land_class";

/// 加载好的混合设置。
pub struct Hybrid {
    pub config: HybridConfig,
    land_class: Option<Slot>,
}

fn check_land_class(config: &SlotConfig) -> Result<()> {
    ensure!(
        config.kind == SlotKind::Param,
        "slot {LAND_CLASS_SLOT} is a param slot, not {:?}",
        config.kind
    );
    for output in &config.outputs {
        ensure!(
            colm_core::LandClassOverrides::REAL_NAMES.contains(&output.name.as_str()),
            "slot {LAND_CLASS_SLOT} output {} is not one of the DEF_LC_* land-class columns",
            output.name
        );
    }
    Ok(())
}

fn known_slot(config: &SlotConfig) -> Result<()> {
    match config.name.as_str() {
        LAND_CLASS_SLOT => check_land_class(config),
        other => bail!(
            "slot {other} is not available in this engine (P0 provides only {LAND_CLASS_SLOT})"
        ),
    }
}

impl Hybrid {
    /// 正式运行：校验插槽并加载 ONNX 模型。
    pub fn load(path: &Path) -> Result<Self> {
        let config = HybridConfig::load(path)?;
        let mut land_class = None;
        for slot in &config.slots {
            known_slot(slot)?;
            land_class = Some(Slot::load(slot.clone())?);
        }
        Ok(Self { config, land_class })
    }

    /// 用给定后端代替 ONNX（测试与"模仿物理"检查）。
    pub fn with_backend(config: HybridConfig, backend: Arc<dyn Surrogate>) -> Result<Self> {
        let mut land_class = None;
        for slot in &config.slots {
            known_slot(slot)?;
            land_class = Some(Slot::with_backend(slot.clone(), Arc::clone(&backend))?);
        }
        Ok(Self { config, land_class })
    }

    /// 各 patch 的物理参数：土壤 patch 叠上插槽给出的地类表列，其余照抄 `physics`。
    /// 返回的次序与 `patches` 相同。
    pub fn patch_physics(
        &self,
        constant: &Path,
        patches: &[usize],
        physics: &LandPhysicsParameters,
    ) -> Result<Vec<LandPhysicsParameters>> {
        let mut out = vec![physics.clone(); patches.len()];
        let Some(slot) = &self.land_class else {
            return Ok(out);
        };
        ensure!(
            !physics.use_pft,
            "slot {LAND_CLASS_SLOT} overrides land-class table columns, which only drive LCT \
             soil patches; this case uses PFT/PC"
        );
        let restart = colm_init::RestartFile::open(constant)?;
        let soil: Vec<usize> = patches
            .iter()
            .enumerate()
            .filter_map(
                |(row, &patch)| match patch_integer(&restart, "patchtype", patch) {
                    Ok(0) => Some(Ok(row)),
                    Ok(_) => None,
                    Err(error) => Some(Err(error)),
                },
            )
            .collect::<Result<_>>()?;
        let soil_patches: Vec<usize> = soil.iter().map(|&row| patches[row]).collect();
        let features = feature_matrix(&restart, &slot.config.features, &soil_patches)?;
        let values = slot.evaluate(&features)?;
        for (index, &row) in soil.iter().enumerate() {
            let overrides = &mut out[row].land_class_overrides;
            for (output, &value) in slot.config.outputs.iter().zip(values.row(index)) {
                overrides.set_real(&output.name, value)?;
            }
        }
        Ok(out)
    }
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

/// 特征抓取（训练前准备数据）：按配置把每个土壤 patch 的特征与物理表值写成 CSV，不改任何参数。
///
/// 列：`patch`、各特征、各输出的物理表值（`physics:DEF_LC_*`）。追加写，首次写表头。
pub fn write_land_class_tap(
    config: &HybridConfig,
    constant: &Path,
    patches: &[usize],
    physics: &LandPhysicsParameters,
    out: &Path,
) -> Result<()> {
    let Some(slot) = config
        .slots
        .iter()
        .find(|slot| slot.name == LAND_CLASS_SLOT)
    else {
        return Ok(());
    };
    check_land_class(slot)?;
    let restart = colm_init::RestartFile::open(constant)?;
    let new_file = !out.exists();
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(out)
        .with_context(|| format!("cannot open {}", out.display()))?;
    if new_file {
        let mut header = vec!["patch".to_owned()];
        header.extend(slot.features.iter().cloned());
        header.extend(
            slot.outputs
                .iter()
                .map(|output| format!("physics:{}", output.name)),
        );
        writeln!(file, "{}", header.join(","))?;
    }
    for &patch in patches {
        if patch_integer(&restart, "patchtype", patch)? != 0 {
            continue;
        }
        let class = usize::try_from(patch_integer(&restart, "patchclass", patch)?)
            .context("patchclass is negative")?;
        let constants = colm_core::ClassConstants::new(physics.land_cover_scheme, class)?;
        let mut row = vec![patch.to_string()];
        for feature in &slot.features {
            row.push(format!("{:e}", feature_value(&restart, feature, patch)?));
        }
        for output in &slot.outputs {
            row.push(format!("{:e}", constants.table_value(&output.name)?));
        }
        writeln!(file, "{}", row.join(","))?;
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

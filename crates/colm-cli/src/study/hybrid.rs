//! Study 训练混合模型（docs/design-hybrid.md 第 12 节）：网络的每个权重是 DE 决策向量的一维
//! （`hybrid:w00000`…，排在采样参数之后）。成员物化时把权重拼成 `models/study.mlp.json`，
//! 连同冻结在 spec 里的特征标准化写出 `hybrid.toml`；基线成员没有权重，是纯物理的参照。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;

use super::spec::{HybridNormalization, HybridStudySpec, StudySpec};

const WEIGHT_PREFIX: &str = "hybrid:w";
const MODEL: &str = "models/study.mlp.json";
const NORMALIZE: &str = "models/study.norm.json";

/// 第 `index` 个权重的成员参数名；补零到五位，字典序即序号。
pub fn weight_key(index: usize) -> String {
    format!("{WEIGHT_PREFIX}{index:05}")
}

pub fn is_weight_key(key: &str) -> bool {
    key.starts_with(WEIGHT_PREFIX)
}

/// 成员参数里的权重（按序号）；序号必须连续。
fn weights(parameters: &BTreeMap<String, f64>) -> Result<Vec<f64>> {
    parameters
        .iter()
        .filter(|(key, _)| is_weight_key(key))
        .enumerate()
        .map(|(index, (key, value))| {
            ensure!(
                *key == weight_key(index),
                "hybrid weight {key} is out of sequence"
            );
            Ok(*value)
        })
        .collect()
}

/// 有权重的成员写出网络、标准化与 `hybrid.toml`；没有权重（基线）什么都不写。
pub fn write_member_files(
    destination: &Path,
    spec: &StudySpec,
    parameters: &BTreeMap<String, f64>,
) -> Result<()> {
    let weights = weights(parameters)?;
    if weights.is_empty() {
        return Ok(());
    }
    let hybrid = spec
        .hybrid
        .as_ref()
        .context("the member has hybrid weights but the Study trains no hybrid network")?;
    let normalize = hybrid
        .normalize
        .as_ref()
        .context("the Study's hybrid feature normalization was never frozen")?;
    let mlp = hybrid.mlp(&weights)?;
    std::fs::create_dir_all(destination.join("models"))?;
    mlp.save(&destination.join(MODEL))?;
    std::fs::write(
        destination.join(NORMALIZE),
        serde_json::to_string_pretty(normalize)?,
    )?;
    let sha = colm_hybrid::sha256_hex(&std::fs::read(destination.join(MODEL))?);
    let outputs = hybrid
        .outputs
        .iter()
        .map(|output| {
            let transform = serde_json::to_value(output.transform)?
                .as_str()
                .context("transform is not a string")?
                .to_owned();
            Ok((output.name.clone(), Some(output.range), transform))
        })
        .collect::<Result<Vec<_>>>()?;
    let config = destination.join("hybrid.toml");
    std::fs::write(
        &config,
        crate::hybrid_cmd::slot_toml(
            &hybrid.slot,
            Path::new(MODEL),
            &sha,
            &hybrid.features,
            Some(Path::new(NORMALIZE)),
            &outputs,
        ),
    )
    .with_context(|| format!("cannot write {}", config.display()))?;
    colm_hybrid::HybridConfig::load(&config)?;
    Ok(())
}

/// 续跑时核对已物化的成员：有权重就要有能加载（sha256 对得上）的 `hybrid.toml`，没有就不能有。
pub fn verify_member_files(destination: &Path, parameters: &BTreeMap<String, f64>) -> Result<()> {
    let config = destination.join("hybrid.toml");
    if weights(parameters)?.is_empty() {
        ensure!(
            !config.exists(),
            "Study member {} has a hybrid.toml but no hybrid weights",
            destination.display()
        );
        return Ok(());
    }
    colm_hybrid::HybridConfig::load(&config).with_context(|| {
        format!(
            "incomplete Study member {}: its hybrid model does not load",
            destination.display()
        )
    })?;
    Ok(())
}

#[derive(Deserialize)]
struct DryRunSlot {
    rows: usize,
    features: Vec<DryRunColumn>,
}

#[derive(Deserialize)]
struct DryRunColumn {
    mean: f64,
    std: f64,
}

/// 建 Study 时冻结特征标准化：没给 `hybrid.normalize` 就在每个基础算例上空跑 `colm-rs`，
/// 把各算例的特征汇总合并成总体的均值与标准差。常数特征（标准差为 0）的标准差取 1。
pub fn freeze_normalization(spec: &mut StudySpec, base_cases: &[PathBuf]) -> Result<()> {
    let Some(hybrid) = spec.hybrid.as_ref() else {
        return Ok(());
    };
    for case in base_cases {
        if case.join("hybrid.toml").exists() {
            bail!(
                "{} already has a hybrid.toml; a Study that trains a hybrid network needs physics-only base cases",
                case.display()
            );
        }
    }
    if hybrid.normalize.is_some() {
        return Ok(());
    }
    let kernel_dir = spec.kernel_dir.as_deref().context(
        "computing the hybrid feature normalization needs kernel_dir (or give hybrid.normalize)",
    )?;
    let kernel = colm_kernel::Kernel::open(Path::new(kernel_dir))?;
    let scratch = std::env::temp_dir().join(format!(
        "colm-study-hybrid-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&scratch)?;
    let config = scratch.join("hybrid.toml");
    let result = (|| {
        std::fs::write(&config, spec_toml(hybrid))?;
        let summaries = base_cases
            .iter()
            .map(|case| {
                let text =
                    crate::hybrid_cmd::dry_run(case, &config, &kernel).with_context(|| {
                        format!("cannot read hybrid features of {}", case.display())
                    })?;
                let mut slots: Vec<DryRunSlot> = serde_json::from_str(&text)
                    .context("cannot parse the colm-rs dry-run summary")?;
                ensure!(
                    slots.len() == 1,
                    "the dry run reported {} slots",
                    slots.len()
                );
                Ok(slots.remove(0))
            })
            .collect::<Result<Vec<_>>>()?;
        pooled(&summaries, hybrid.features.len())
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    let normalize = result?;
    spec.hybrid.as_mut().expect("checked above").normalize = Some(normalize);
    Ok(())
}

/// 没有模型的插槽配置（`colm-rs` 空跑只汇总特征）。
fn spec_toml(hybrid: &HybridStudySpec) -> String {
    use crate::hybrid_cmd::toml_string;
    let mut text = String::from("[[slot]]\n");
    text += &format!("name = {}\nkind = \"param\"\n", toml_string(&hybrid.slot));
    text += &format!(
        "features = [{}]\n",
        hybrid
            .features
            .iter()
            .map(|f| toml_string(f))
            .collect::<Vec<_>>()
            .join(", ")
    );
    text += "outputs = [\n";
    for output in &hybrid.outputs {
        text += &format!(
            "  {{ name = {}, range = [{:?}, {:?}] }},\n",
            toml_string(&output.name),
            output.range[0],
            output.range[1]
        );
    }
    text += "]\n";
    text
}

/// 按行数加权合并各算例的均值与（总体）标准差。
fn pooled(summaries: &[DryRunSlot], features: usize) -> Result<HybridNormalization> {
    let rows: usize = summaries.iter().map(|s| s.rows).sum();
    ensure!(
        rows > 0,
        "the base cases have no patches for the hybrid slot to train on"
    );
    let mut mean = vec![0.0; features];
    let mut std = vec![0.0; features];
    for column in 0..features {
        let mut sum = 0.0;
        let mut squares = 0.0;
        for summary in summaries.iter().filter(|s| s.rows > 0) {
            let stats = summary
                .features
                .get(column)
                .context("the dry run reported too few features")?;
            let n = summary.rows as f64;
            sum += n * stats.mean;
            squares += n * (stats.std * stats.std + stats.mean * stats.mean);
        }
        let m = sum / rows as f64;
        let variance = (squares / rows as f64 - m * m).max(0.0);
        let s = variance.sqrt();
        mean[column] = m;
        std[column] = if s > 1e-12 * m.abs().max(1.0) { s } else { 1.0 };
    }
    Ok(HybridNormalization { mean, std })
}

#[cfg(test)]
#[path = "hybrid_tests.rs"]
mod hybrid_tests;

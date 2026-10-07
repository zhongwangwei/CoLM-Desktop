//! `hybrid.toml`：放在算例目录、与 `case.nml` 并列的混合模型配置。
//!
//! 不放进 namelist：Fortran 读 namelist 遇到不认识的变量会报错。模型与归一化文件的路径相对配置文件
//! 所在目录；加载时按 `sha256` 校验模型文件，换了模型而没改配置会被拒绝。

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// 插槽的类型（docs/design-hybrid.md 第 1 节）。P0 只接参数插槽。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SlotKind {
    Param,
    Process,
    Correction,
}

/// 输出的变换：网络的原始输出 `y` → 物理量。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Transform {
    /// 原样；给了 `range` 时超出范围是错误。
    #[default]
    Identity,
    /// 截到 `range` 内。
    Clamp,
    /// `lo + (hi - lo) / (1 + exp(-y))`。
    Sigmoid,
    /// `lo + ln(1 + exp(y))`；给了上界时超出是错误。
    Softplus,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputSpec {
    pub name: String,
    #[serde(default)]
    pub range: Option<[f64; 2]>,
    #[serde(default)]
    pub transform: Transform,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotConfig {
    /// 引擎注册过的插槽名，例如 `land_class`。
    pub name: String,
    pub kind: SlotKind,
    /// ONNX 模型；相对配置文件所在目录。训练前只做特征抓取时可以不给（见 [`HybridConfig::load_spec`]）。
    #[serde(default)]
    pub model: Option<PathBuf>,
    /// 模型文件的 sha256（十六进制）。
    #[serde(default)]
    pub sha256: Option<String>,
    pub features: Vec<String>,
    /// 特征归一化（JSON：`{"mean": [...], "std": [...]}`），按 `features` 的次序。
    #[serde(default)]
    pub normalize: Option<PathBuf>,
    pub outputs: Vec<OutputSpec>,
    /// 超出训练范围（见归一化文件的 `min`/`max`）的行怎么办。
    #[serde(default)]
    pub outside: Outside,
}

/// 超出训练范围的行：照用模型（缺省，与没有这个选项时相同），或退回纯物理参数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Outside {
    #[default]
    Apply,
    Physics,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileContents {
    #[serde(rename = "slot", default)]
    slots: Vec<SlotConfig>,
}

/// 读进来、路径已解析、模型已校验的配置。
#[derive(Debug, Clone, PartialEq)]
pub struct HybridConfig {
    pub path: PathBuf,
    pub slots: Vec<SlotConfig>,
    /// 配置文件与所有模型、归一化文件内容的合成指纹：进阶段指纹与续跑文件。
    pub fingerprint: String,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl HybridConfig {
    /// 正式运行：每个插槽都要有模型，且模型的 sha256 与配置一致。
    pub fn load(path: &Path) -> Result<Self> {
        Self::read(path, true)
    }

    /// 只读插槽的特征与输出声明（特征抓取用，训练前还没有模型）。给了模型也照样校验。
    pub fn load_spec(path: &Path) -> Result<Self> {
        Self::read(path, false)
    }

    /// 只解析配置里声明的插槽（模型与归一化路径按配置所在目录解析），不读、不校验任何文件。
    /// 用于展示与清理：模型被换掉或丢了也能看到配置写的是什么。
    pub fn declared(path: &Path) -> Result<Vec<SlotConfig>> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        let contents: FileContents =
            toml::from_str(&text).with_context(|| format!("cannot parse {}", path.display()))?;
        let base = path.parent().unwrap_or(Path::new("."));
        Ok(contents
            .slots
            .into_iter()
            .map(|mut slot| {
                slot.model = slot.model.map(|model| base.join(model));
                slot.normalize = slot.normalize.map(|normalize| base.join(normalize));
                slot
            })
            .collect())
    }

    fn read(path: &Path, require_model: bool) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        let contents: FileContents =
            toml::from_str(&text).with_context(|| format!("cannot parse {}", path.display()))?;
        ensure!(
            !contents.slots.is_empty(),
            "{} declares no [[slot]]",
            path.display()
        );
        let base = path.parent().unwrap_or(Path::new("."));
        let mut fingerprint = Sha256::new();
        fingerprint.update(text.as_bytes());
        let mut slots = Vec::with_capacity(contents.slots.len());
        for mut slot in contents.slots {
            let name = slot.name.clone();
            if slots.iter().any(|other: &SlotConfig| other.name == name) {
                bail!("slot {name} is declared twice in {}", path.display());
            }
            ensure!(!slot.features.is_empty(), "slot {name} has no features");
            ensure!(!slot.outputs.is_empty(), "slot {name} has no outputs");
            for output in &slot.outputs {
                if let Some([lo, hi]) = output.range {
                    ensure!(
                        lo.is_finite() && hi.is_finite() && lo < hi,
                        "slot {name} output {} has an invalid range [{lo}, {hi}]",
                        output.name
                    );
                }
                ensure!(
                    !matches!(
                        output.transform,
                        Transform::Clamp | Transform::Sigmoid | Transform::Softplus
                    ) || output.range.is_some(),
                    "slot {name} output {} needs a range for its {:?} transform",
                    output.name,
                    output.transform
                );
            }
            match (&mut slot.model, &slot.sha256) {
                (Some(model), Some(expected)) => {
                    *model = base.join(&*model);
                    let bytes = std::fs::read(&*model)
                        .with_context(|| format!("cannot read model {}", model.display()))?;
                    let actual = sha256_hex(&bytes);
                    ensure!(
                        actual.eq_ignore_ascii_case(expected.trim()),
                        "slot {name}: {} has sha256 {actual}, but {} says {expected}",
                        model.display(),
                        path.display()
                    );
                    fingerprint.update(&bytes);
                }
                (None, None) if !require_model => {}
                (None, None) => bail!("slot {name} has no model"),
                _ => bail!("slot {name} needs both model and sha256"),
            }
            if let Some(normalize) = &mut slot.normalize {
                *normalize = base.join(&*normalize);
                let bytes = std::fs::read(&*normalize)
                    .with_context(|| format!("cannot read {}", normalize.display()))?;
                fingerprint.update(&bytes);
            }
            slots.push(slot);
        }
        Ok(Self {
            path: path.to_path_buf(),
            slots,
            fingerprint: fingerprint
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        })
    }
}

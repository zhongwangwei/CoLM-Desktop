//! 读 PLUMBER2 / FLUXNET-CH4 的 `Observation/*_Flux.nc` 与模型的
//! `*_hist_*.nc`。
//!
//! 实测的观测文件形状（CN-Cng，2008-2009）：
//! `time = 35088` 半小时步长、`x = y = 1`；能量通量与 `Ustar` / `NEE`
//! 带 `<name>_qc`，而 `GPP` / `GPP_DT` / `Resp` 只有不确定度、没有 QC。
//! 后三者仍可评估，但只能明确标记为“使用全部有限观测值”，不能伪装成
//! 只使用实测（qc == 0）。`_FillValue = -9999`。
//!
//! **默认不用 `_cor` 能量闭合订正版本**：design.md §2.8 / §2.8b 的目标值是用
//! 未订正版算的，用订正版复现不出来。但订正版能问答一个别的问题，见
//! [`corrected`]。

use anyhow::{Context, Result};
use std::path::Path;

/// 模型侧如何得到一个可与观测比较的序列。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ModelSource {
    /// 一个 history 变量乘单位换算系数。
    Direct { variable: &'static str, scale: f64 },
    /// `(minuend - subtrahend) * scale`。NEE 没有独立 history 变量，
    /// 但总呼吸减总同化就是模型的净生态系统交换。
    Difference {
        minuend: &'static str,
        subtrahend: &'static str,
        scale: f64,
    },
}

impl ModelSource {
    pub fn label(self) -> String {
        match self {
            Self::Direct { variable, .. } => variable.to_string(),
            Self::Difference {
                minuend,
                subtrahend,
                ..
            } => format!("{minuend} - {subtrahend}"),
        }
    }

    pub fn required(self) -> [&'static str; 2] {
        match self {
            Self::Direct { variable, .. } => [variable, ""],
            Self::Difference {
                minuend,
                subtrahend,
                ..
            } => [minuend, subtrahend],
        }
    }
}

/// 一个可评估变量的唯一元数据表。GUI 的复选框与 CLI 的计算都从这里得到，
/// 避免一边增加变量、另一边仍停留在旧的五项清单。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EvaluationVariable {
    pub observation: &'static str,
    pub label_zh: &'static str,
    pub label_en: &'static str,
    pub units: &'static str,
    pub model: ModelSource,
    /// `None` 表示观测文件没有对应 QC，只能使用全部有限值。
    pub qc: Option<&'static str>,
}

const MOL_TO_MICROMOL: f64 = 1_000_000.0;
const MOL_TO_NANOMOL: f64 = 1_000_000_000.0;

pub const EVALUATION_VARIABLES: [EvaluationVariable; 11] = [
    EvaluationVariable {
        observation: "Rnet",
        label_zh: "净辐射",
        label_en: "Net radiation",
        units: "W/m²",
        model: ModelSource::Direct {
            variable: "f_rnet",
            scale: 1.0,
        },
        qc: Some("Rnet_qc"),
    },
    EvaluationVariable {
        observation: "Qh",
        label_zh: "感热通量",
        label_en: "Sensible heat flux",
        units: "W/m²",
        model: ModelSource::Direct {
            variable: "f_fsena",
            scale: 1.0,
        },
        qc: Some("Qh_qc"),
    },
    EvaluationVariable {
        observation: "Qle",
        label_zh: "潜热通量",
        label_en: "Latent heat flux",
        units: "W/m²",
        model: ModelSource::Direct {
            variable: "f_lfevpa",
            scale: 1.0,
        },
        qc: Some("Qle_qc"),
    },
    EvaluationVariable {
        observation: "Qg",
        label_zh: "地表热通量",
        label_en: "Ground heat flux",
        units: "W/m²",
        model: ModelSource::Direct {
            variable: "f_fgrnd",
            scale: 1.0,
        },
        qc: Some("Qg_qc"),
    },
    EvaluationVariable {
        observation: "SWup",
        label_zh: "反射短波辐射",
        label_en: "Reflected shortwave radiation",
        units: "W/m²",
        model: ModelSource::Direct {
            variable: "f_sr",
            scale: 1.0,
        },
        qc: Some("SWup_qc"),
    },
    EvaluationVariable {
        observation: "Ustar",
        label_zh: "摩擦速度",
        label_en: "Friction velocity",
        units: "m/s",
        model: ModelSource::Direct {
            variable: "f_ustar",
            scale: 1.0,
        },
        qc: Some("Ustar_qc"),
    },
    EvaluationVariable {
        observation: "GPP",
        label_zh: "总初级生产力（夜间分割）",
        label_en: "GPP (nighttime partitioning)",
        units: "µmol/m²/s",
        model: ModelSource::Direct {
            variable: "f_assim",
            scale: MOL_TO_MICROMOL,
        },
        qc: None,
    },
    EvaluationVariable {
        observation: "GPP_DT",
        label_zh: "总初级生产力（日间分割）",
        label_en: "GPP (daytime partitioning)",
        units: "µmol/m²/s",
        model: ModelSource::Direct {
            variable: "f_assim",
            scale: MOL_TO_MICROMOL,
        },
        qc: None,
    },
    EvaluationVariable {
        observation: "Resp",
        label_zh: "生态系统呼吸",
        label_en: "Ecosystem respiration",
        units: "µmol/m²/s",
        model: ModelSource::Direct {
            variable: "f_respc",
            scale: MOL_TO_MICROMOL,
        },
        qc: None,
    },
    EvaluationVariable {
        observation: "NEE",
        label_zh: "净生态系统交换",
        label_en: "Net ecosystem exchange",
        units: "µmol/m²/s",
        model: ModelSource::Difference {
            minuend: "f_respc",
            subtrahend: "f_assim",
            scale: MOL_TO_MICROMOL,
        },
        qc: Some("NEE_qc"),
    },
    EvaluationVariable {
        observation: "FCH4_f_ann",
        label_zh: "甲烷通量（FLUXNET-CH4 插补）",
        label_en: "Methane flux (FLUXNET-CH4 gap-filled)",
        units: "nmol CH₄/m²/s",
        model: ModelSource::Direct {
            variable: "f_methane_surf_flux_tot",
            scale: MOL_TO_NANOMOL,
        },
        // ANNOPTLM 的 QC 编码是 1/3，不是 PLUMBER2 的 0=实测约定；这里比较
        // 数据集明确提供的连续插补序列，不能套用另一套 QC 后把样本全滤光。
        qc: None,
    },
];

/// 这个观测量的能量闭合订正版本叫什么，没有就是 `None`。
///
/// 涡度相关的湍流通量普遍**关不上能量收支**：实测 AT-Neu 的
/// `Qle_cor` 比 `Qle` 高 25.5 W/m²、`Qh_cor` 比 `Qh` 高 3.7 W/m²
/// （qc==0 的 9 万个样本上）。拿未订正的观测去评一个能量守恒的模型，
/// 模型会**看上去偏湿**，而偏差的大小恰好是那个缺口 —— 实测 AT-Neu 的
/// KGE β 是 1.39，而 88.0/62.5 = 1.41。
///
/// 所以两者都要能算：未订正的对得上 design.md 的目标值，订正的才回答
/// 「模型到底偏不偏」。默认仍是未订正 —— 换默认会让那些目标值集体失效。
///
/// 辐射量（`Rnet` / `SWup`）与地表热通量（`Qg`）没有订正版：闭合订正
/// 是把可用能量的残差按 Bowen 比分给两个湍流通量，它改的只有那两个。
pub fn corrected(o_name: &str) -> Option<&'static str> {
    match o_name {
        "Qle" => Some("Qle_cor"),
        "Qh" => Some("Qh_cor"),
        _ => None,
    }
}

pub fn read_1d(path: &Path, name: &str) -> Result<Vec<f64>> {
    let f = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let v = f
        .variable(name)
        .with_context(|| format!("{} has no variable {name}", path.display()))?;
    Ok(v.get_values::<f64, _>(..)?)
}

/// 读 `time` 变量的 `units` 属性。
///
/// 观测文件的时间原点写在这里（实测 `"seconds since 2008-01-01 00:00:00"`），
/// 而模型 history 的原点固定是 1900 —— 两边换算到同一原点才谈得上配对。
pub fn time_units(path: &Path) -> Result<String> {
    let f = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let v = f
        .variable("time")
        .with_context(|| format!("{} has no time variable", path.display()))?;
    match v.attribute("units").and_then(|a| a.value().ok()) {
        Some(netcdf::AttributeValue::Str(s)) => Ok(s),
        other => anyhow::bail!(
            "time:units in {} is {other:?}, not a string",
            path.display()
        ),
    }
}

#[cfg(test)]
#[path = "obs_tests.rs"]
mod obs_tests;

//! 网格河湖水库（`MOD_Grid_Reservoir`，`DEF_Reservoir_Method = 1 | 2`）。
//!
//! 参数表（`DEF_ReservoirPara_file`，通常就是单元流域文件）里每座坝有所在单元流域 `dam_seq`、
//! 建成年份与库容、流量参数。网络里出现的坝按参数表行序编号（续跑里 `volresv` 的次序，
//! `resv_global_id`）。
//!
//! 两套调度方案：
//! * 1 = CaMa-Flood v4.20 的四段方案（Funato et al.），只用总/调节/正常库容；
//! * 2 = H22（Hanazaki et al. 2022，CaMa-Flood PR124 的 legacy LDAMH22）。它额外需要参数表里的
//!   `dam_upreal`（坝以上集水面积，km²），并派生 `volresv_flood`（洪水控制起始库容 C）与
//!   `resv_h22_coeff`（入流泄放系数 k）。没有这两套静态量、只照抄调度公式会退化成另一条
//!   出流曲线而不是 H22，所以参数缺失时 [`Reservoir::read_with_regional`] 直接报错，不静默退回 1。

use colm_numeric::Contract;
use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use colm_core::LibmPow;

use super::network::RiverNetwork;

/// `gridriver_reservoir_identity` 的版本号。
pub const IDENTITY_VERSION: f64 = 1.0;

/// `DEF_Reservoir_Method` 的取值（上游 `MOD_Grid_Reservoir.F90:99-101`）。
const METHOD_FUNATO: i64 = 1;
const METHOD_H22: i64 = 2;

/// H22 的紧急库容 = C + `H22_EMERGENCY_FLOOD_FRACTION` × (总库容 − C)。
/// 上游明确沿用 CaMa-Flood PR124 的 legacy LDAMH22 分支（`MOD_Grid_Reservoir.F90:388` 的注释
/// "0.95, not 0.8"）：写成 0.8 会让汛限以上的调蓄曲线整体偏低。
const H22_EMERGENCY_FLOOD_FRACTION: f64 = 0.95;

/// H22 的正常库容 = 该系数 × C（C 即 `volresv_flood`）。
const H22_NORMAL_FLOOD_FRACTION: f64 = 0.5;

/// H22 入流泄放系数 `k = max(1 - flood_capacity*1e-6/(upstream_area*0.2), 0)` 的两个尺度：
/// `1e-6` 把 m³ 的库容差折成 km³，`0.2` 是坝以上面积里参与调蓄的比例。两者都直接抄上游
/// （`MOD_Grid_Reservoir.F90:389`），没有可推导的物理含义，所以不内联。
const H22_COEFF_VOLUME_SCALE: f64 = 1.0e-6;
const H22_COEFF_AREA_FRACTION: f64 = 0.2;

/// 调度方案。上游把 `method` 作为每次 `reservoir_operation` 的实参；Rust 的调用点
/// （`river/mod.rs` 的汇流循环）只持有参数表，所以方案存在表上，在 `read` 里过
/// [`check_method`] 后才构造 —— 枚举分派不需要兜底分支。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Method {
    Funato,
    H22,
}

/// 网络里出现的水库（按参数表行序）。
#[derive(Debug, Clone)]
pub struct Reservoir {
    /// 调度方案（`DEF_Reservoir_Method`）。
    method: Method,
    /// 每个单元流域的水库号（参数表行序里的序号，0 起）；不是水库时为 `None`。
    pub of_catchment: Vec<Option<usize>>,
    /// 每座水库所在的单元流域（0 起）。
    pub catchment: Vec<usize>,
    /// `dam_seq`（1 起的单元流域号），写续跑标识用。
    pub dam_seq: Vec<i32>,
    /// `dam_GRAND_ID`，写进 unitcat history 的 `resv_GRAND_ID`。
    pub grand_id: Vec<i32>,
    pub build_year: Vec<i32>,
    pub volume_total: Vec<f64>,
    pub volume_emergency: Vec<f64>,
    pub volume_adjust: Vec<f64>,
    pub volume_normal: Vec<f64>,
    pub q_flood: Vec<f64>,
    pub q_adjust: Vec<f64>,
    pub q_normal: Vec<f64>,
    /// `volresv_flood`：H22 的洪水控制起始库容 C（m³）。上游只对方法 2 分配
    /// （`MOD_Grid_Reservoir.F90:326`），所以方法 1 下是 `None`。
    pub volume_flood: Option<Vec<f64>>,
    /// `resv_h22_coeff`：H22 的入流泄放系数 k（无量纲）。同样只有方法 2 有。
    pub h22_coeff: Option<Vec<f64>>,
}

fn read<T: netcdf::NcTypeDescriptor + Copy>(
    file: &netcdf::File,
    name: &str,
    path: &Path,
) -> Result<Vec<T>> {
    file.variable(name)
        .with_context(|| {
            format!(
                "{} has no {name}; DEF_Reservoir_Method needs it",
                path.display()
            )
        })?
        .get_values::<T, _>(..)
        .with_context(|| format!("cannot read {name} from {}", path.display()))
}

/// `reservoir_init` 的方案检查（上游 `MOD_Grid_Reservoir.F90:99-101`）：只接受 1（Funato）
/// 与 2（H22）。返回枚举给 [`Reservoir::operation`] 分派；0（关闭）由调用方在进
/// `reservoir_init` 之前挡掉，这里不当作合法方案。
fn check_method(method: i64) -> Result<Method> {
    match method {
        METHOD_FUNATO => Ok(Method::Funato),
        METHOD_H22 => Ok(Method::H22),
        other => bail!("unsupported reservoir operation method {other}"),
    }
}

/// H22 的派生库容与入流泄放系数（`MOD_Grid_Reservoir.F90:378-392`）。
///
/// 返回 `(volume_flood, volume_normal, volume_emergency, h22_coeff)`：
/// C = 参数表的 `dam_ConVol_mcm`，normal = 0.5·C，emergency = C + 0.95·(total − C)。
/// 上游把这几条校验收在同一处 `CoLM_stop('invalid active H22 reservoir storage, discharge or
/// dam_upreal')` 里：集水面积非正、Conservation > Total、Qf < Qn 都是参数表错误，
/// 不是可以夹紧的边界情况。
fn h22_statics(
    volume_total: f64,
    conservation_volume: f64,
    q_normal: f64,
    q_flood: f64,
    upstream_area: f64,
) -> Result<(f64, f64, f64, f64)> {
    ensure!(
        upstream_area.is_finite()
            && upstream_area > 0.0
            && conservation_volume <= volume_total
            && q_flood >= q_normal,
        "invalid active H22 reservoir storage, discharge or dam_upreal"
    );
    let flood_capacity = volume_total - conservation_volume;
    Ok((
        conservation_volume,
        H22_NORMAL_FLOOD_FRACTION * conservation_volume,
        conservation_volume + H22_EMERGENCY_FLOOD_FRACTION * flood_capacity,
        (1.0 - flood_capacity * H22_COEFF_VOLUME_SCALE / (upstream_area * H22_COEFF_AREA_FRACTION))
            .max(0.0),
    ))
}

impl Reservoir {
    /// `reservoir_init`（单进程、全球网络）。
    pub fn read(path: &Path, network: &RiverNetwork, method: i64) -> Result<Self> {
        Self::read_with_regional(path, network, method, None)
    }

    /// [`Self::read`]；`regional` 是区域网络文件时，参数表的 `dam_seq` 先按它的 `seq_src_index`
    /// 换成区域编号（`MOD_Grid_Reservoir.F90:91-110`）。
    pub fn read_with_regional(
        path: &Path,
        network: &RiverNetwork,
        method: i64,
        regional: Option<&Path>,
    ) -> Result<Self> {
        let method = check_method(method)?;
        let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        let grand_id = read::<i32>(&file, "dam_GRAND_ID", path)?;
        let mut dam_seq_all = read::<i32>(&file, "dam_seq", path)?;
        if let Some(regional) = regional {
            colm_init::unitcatchment_regional::translate_dam_seq(&mut dam_seq_all, path, regional)?;
        }
        let year = read::<i32>(&file, "dam_year", path)?;
        let total = read::<f64>(&file, "dam_TotalVol_mcm", path)?;
        let conservation = read::<f64>(&file, "dam_ConVol_mcm", path)?;
        let q_normal_all = read::<f64>(&file, "dam_Qn", path)?;
        let q_flood_all = read::<f64>(&file, "dam_Qf", path)?;
        let catalogue_rows = dam_seq_all.len();
        ensure!(
            grand_id.len() == catalogue_rows,
            "reservoir dam_GRAND_ID and dam_seq lengths differ"
        );
        for (name, len) in [
            ("dam_year", year.len()),
            ("dam_TotalVol_mcm", total.len()),
            ("dam_ConVol_mcm", conservation.len()),
            ("dam_Qn", q_normal_all.len()),
            ("dam_Qf", q_flood_all.len()),
        ] {
            ensure!(
                len == catalogue_rows,
                "reservoir {name} and dam_seq lengths differ"
            );
        }
        let keep = colm_init::gridriver::first_dam_seq_rows(&dam_seq_all);
        if keep.len() < catalogue_rows {
            eprintln!(
                "Warning: {} reservoir rows share a dam_seq with an earlier row; keeping the first in the file",
                catalogue_rows - keep.len()
            );
        }
        let pick_i32 = |values: &[i32]| keep.iter().map(|&i| values[i]).collect::<Vec<_>>();
        let pick_f64 = |values: &[f64]| keep.iter().map(|&i| values[i]).collect::<Vec<_>>();
        let grand_id = pick_i32(&grand_id);
        let dam_seq_all = pick_i32(&dam_seq_all);
        let year = pick_i32(&year);
        let total = pick_f64(&total);
        let conservation = pick_f64(&conservation);
        let q_normal_all = pick_f64(&q_normal_all);
        let q_flood_all = pick_f64(&q_flood_all);
        let rows = keep.len();
        let n = network.len();
        let mut of_catchment = vec![None; n];
        let mut volume_flood = Vec::new();
        let mut h22_coeff = Vec::new();
        let mut reservoir = Self {
            method,
            of_catchment: Vec::new(),
            catchment: Vec::new(),
            dam_seq: Vec::new(),
            grand_id: Vec::new(),
            build_year: Vec::new(),
            volume_total: Vec::new(),
            volume_emergency: Vec::new(),
            volume_adjust: Vec::new(),
            volume_normal: Vec::new(),
            q_flood: Vec::new(),
            q_adjust: Vec::new(),
            q_normal: Vec::new(),
            volume_flood: None,
            h22_coeff: None,
        };
        // 上游只在 `DEF_Reservoir_Method == 2` 且全域至少有一座活跃水库时才读 `dam_upreal`
        // （`MOD_Grid_Reservoir.F90:370-375`：`totalnumresv > 0`）。区域网络套全球坝表时
        // "坝表非空、但没有一座落在本区域"是正常配置，不能因为缺 H22 参数就报错；
        // 反过来，有活跃水库却缺变量/长度不符是参数表错误，必须报出来而不是退回方法 1。
        let active = (0..rows)
            .filter(|&row| (1..=n as i32).contains(&dam_seq_all[row]))
            .collect::<Vec<_>>();
        let upstream_area = if method == Method::H22 && !active.is_empty() {
            let values = read::<f64>(&file, "dam_upreal", path)?;
            ensure!(
                values.len() == catalogue_rows,
                "reservoir dam_upreal and dam_seq lengths differ"
            );
            Some(pick_f64(&values))
        } else {
            None
        };
        // 参数表行序里落在网络中的坝依次编号（`catalogue_to_active`）。
        for row in active {
            let seq = dam_seq_all[row];
            let volume_total = total[row] * 1.0e6;
            let conservation_volume = conservation[row] * 1.0e6;
            let (q_normal, q_flood) = (q_normal_all[row], q_flood_all[row]);
            ensure!(
                volume_total.is_finite()
                    && volume_total > 0.0
                    && conservation_volume.is_finite()
                    && conservation_volume > 0.0
                    && q_normal.is_finite()
                    && q_normal >= 0.0
                    && q_flood.is_finite()
                    && q_flood >= 0.0,
                "invalid active reservoir volume or outflow parameter"
            );
            let i = seq as usize - 1;
            of_catchment[i] = Some(reservoir.catchment.len());
            reservoir.catchment.push(i);
            reservoir.dam_seq.push(seq);
            reservoir.grand_id.push(grand_id[row]);
            reservoir.build_year.push(year[row]);
            reservoir.volume_total.push(volume_total);
            reservoir.q_flood.push(q_flood);
            reservoir.q_normal.push(q_normal);
            match method {
                Method::Funato => {
                    reservoir.volume_emergency.push(volume_total * 0.94);
                    reservoir.volume_adjust.push(volume_total * 0.77);
                    reservoir
                        .volume_normal
                        .push((volume_total * 0.7).min(conservation_volume));
                    reservoir.q_adjust.push((q_normal + q_flood) * 0.5);
                }
                Method::H22 => {
                    let area = upstream_area.as_ref().expect("active H22 reservoir")[row];
                    let (flood, normal, emergency, coeff) =
                        h22_statics(volume_total, conservation_volume, q_normal, q_flood, area)?;
                    volume_flood.push(flood);
                    reservoir.volume_normal.push(normal);
                    reservoir.volume_emergency.push(emergency);
                    // 方法 2 不用调节参数（上游同样置 0）。
                    reservoir.volume_adjust.push(0.0);
                    reservoir.q_adjust.push(0.0);
                    h22_coeff.push(coeff);
                }
            }
        }
        reservoir.of_catchment = of_catchment;
        if method == Method::H22 {
            reservoir.volume_flood = Some(volume_flood);
            reservoir.h22_coeff = Some(h22_coeff);
        }
        Ok(reservoir)
    }

    pub fn len(&self) -> usize {
        self.catchment.len()
    }

    pub fn is_empty(&self) -> bool {
        self.catchment.is_empty()
    }

    /// `gridriver_reservoir_identity`：逐水库 `(版本, dam_seq)`。
    pub fn identity(&self) -> Vec<f64> {
        self.dam_seq
            .iter()
            .flat_map(|&seq| [IDENTITY_VERSION, f64::from(seq)])
            .collect()
    }

    /// `year >= dam_build_year`：这一年水库是否已建成。
    pub fn is_built(&self, r: usize, year: i32) -> bool {
        year >= self.build_year[r]
    }

    /// `reservoir_operation`：由入流与库容求出流（m³/s）。
    pub fn operation(&self, r: usize, qin: f64, vol: f64) -> f64 {
        match self.method {
            Method::Funato => self.operation_funato(r, qin, vol),
            Method::H22 => self.operation_h22(r, qin, vol),
        }
    }

    /// `reservoir_operation` 方法 1（Funato 等，CaMa-Flood v4.20）：四段方案。形状取自 GIMPLE。
    fn operation_funato(&self, r: usize, qin: f64, vol: f64) -> f64 {
        let (ve, va, vn) = (
            self.volume_emergency[r],
            self.volume_adjust[r],
            self.volume_normal[r],
        );
        let (qf, qa, qn) = (self.q_flood[r], self.q_adjust[r], self.q_normal[r]);
        if vol > ve {
            qin.max(qf)
        } else if vol > va {
            let qout = (qf - qa).contract(((vol - va) / (ve - va)).lpow(0.1), qa);
            if qin > qf {
                let q1 = qn + (qin - qn) * (vol - vn) / (ve - vn);
                qout.max(q1)
            } else {
                qout
            }
        } else if vol > vn {
            (qa - qn).contract(((vol - vn) / (va - vn)).lpow(3.0), qn)
        } else {
            (vol / vn).lpow(0.5) * qn
        }
    }

    /// `reservoir_operation` 方法 2（H22，Hanazaki et al. 2022，CaMa-Flood PR124 的 legacy
    /// LDAMH22，`MOD_Grid_Reservoir.F90:409-440`）。
    ///
    /// 边界与跳变照抄上游，不要"修"成连续曲线：`vol == normal` 走第一段；`vol == flood`
    /// 走第二段（入流大时在 C 处**跳变**，上游注释专门要求保留）；`vol == emergency` 不满足
    /// 严格的 `<`，直接落到紧急段。
    fn operation_h22(&self, r: usize, qin: f64, vol: f64) -> f64 {
        let flood = self.volume_flood.as_ref().expect("H22 reservoir statics")[r];
        let normal = self.volume_normal[r];
        let emergency = self.volume_emergency[r];
        let (qf, qn) = (self.q_flood[r], self.q_normal[r]);
        let coeff = self.h22_coeff.as_ref().expect("H22 reservoir statics")[r];
        if vol <= normal {
            qn * (vol / flood)
        } else if vol <= flood {
            if qin >= qf {
                0.5 * qn + (vol - normal) / (flood - normal) * (qf - qn)
            } else {
                // 上游是 `(...)**2`（整数指数）：GIMPLE 里就是自乘，不能换成 libm pow
                // （见 `colm-core/src/material/frac.rs` 关于 `**2` 的约定）。
                let ratio = (vol - normal) / (emergency - normal);
                0.5 * qn + ratio * ratio * (qf - qn)
            }
        } else if vol < emergency {
            if qin >= qf {
                qf + coeff * (vol - flood) / (emergency - flood) * (qin - qf)
            } else {
                let ratio = (vol - normal) / (emergency - normal);
                0.5 * qn + ratio * ratio * (qf - qn)
            }
        } else {
            qin.max(qf)
        }
    }
}

#[cfg(test)]
#[path = "reservoir_tests.rs"]
mod reservoir_tests;

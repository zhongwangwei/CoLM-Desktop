//! 网格河湖水库（`DEF_Reservoir_Method = 1`，`MOD_Grid_Reservoir`）。
//!
//! 参数表（`DEF_ReservoirPara_file`，通常就是单元流域文件）里每座坝有所在单元流域 `dam_seq`、
//! 建成年份与库容、流量参数。网络里出现的坝按参数表行序编号（续跑里 `volresv` 的次序，
//! `resv_global_id`）。调度用 CaMa-Flood v4.20 的四段方案（Funato et al.）。

use colm_numeric::Contract;
use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use colm_core::LibmPow;

use super::network::RiverNetwork;

/// `gridriver_reservoir_identity` 的版本号。
pub const IDENTITY_VERSION: f64 = 1.0;

/// 网络里出现的水库（按参数表行序）。
#[derive(Debug, Clone)]
pub struct Reservoir {
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
        if method != 1 {
            bail!("unsupported reservoir operation method {method}");
        }
        let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        let grand_id = read::<i32>(&file, "dam_GRAND_ID", path)?;
        let mut dam_seq_all = read::<i32>(&file, "dam_seq", path)?;
        if let Some(regional) = regional {
            colm_init::unitcatchment_regional::translate_dam_seq(&mut dam_seq_all, path, regional)?;
        }
        let rows = dam_seq_all.len();
        ensure!(
            grand_id.len() == rows,
            "reservoir dam_GRAND_ID and dam_seq lengths differ"
        );
        let mut sorted = dam_seq_all.clone();
        sorted.sort_unstable();
        ensure!(
            sorted.windows(2).all(|pair| pair[0] != pair[1]),
            "duplicate dam_seq entries in reservoir parameter file"
        );
        let year = read::<i32>(&file, "dam_year", path)?;
        let total = read::<f64>(&file, "dam_TotalVol_mcm", path)?;
        let conservation = read::<f64>(&file, "dam_ConVol_mcm", path)?;
        let q_normal_all = read::<f64>(&file, "dam_Qn", path)?;
        let q_flood_all = read::<f64>(&file, "dam_Qf", path)?;
        for (name, len) in [
            ("dam_year", year.len()),
            ("dam_TotalVol_mcm", total.len()),
            ("dam_ConVol_mcm", conservation.len()),
            ("dam_Qn", q_normal_all.len()),
            ("dam_Qf", q_flood_all.len()),
        ] {
            ensure!(len == rows, "reservoir {name} and dam_seq lengths differ");
        }
        let n = network.len();
        let mut of_catchment = vec![None; n];
        let mut reservoir = Self {
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
        };
        // 参数表行序里落在网络中的坝依次编号（`catalogue_to_active`）。
        for row in 0..rows {
            let seq = dam_seq_all[row];
            if !(1..=n as i32).contains(&seq) {
                continue;
            }
            let volume_total = total[row] * 1.0e6;
            let volume_normal = conservation[row] * 1.0e6;
            let (q_normal, q_flood) = (q_normal_all[row], q_flood_all[row]);
            ensure!(
                volume_total.is_finite()
                    && volume_total > 0.0
                    && volume_normal.is_finite()
                    && volume_normal > 0.0
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
            reservoir.volume_emergency.push(volume_total * 0.94);
            reservoir.volume_adjust.push(volume_total * 0.77);
            reservoir
                .volume_normal
                .push((volume_total * 0.7).min(volume_normal));
            reservoir.q_flood.push(q_flood);
            reservoir.q_adjust.push((q_normal + q_flood) * 0.5);
            reservoir.q_normal.push(q_normal);
        }
        reservoir.of_catchment = of_catchment;
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

    /// `reservoir_operation` 方法 1：由入流与库容求出流（m³/s）。形状取自 GIMPLE。
    pub fn operation(&self, r: usize, qin: f64, vol: f64) -> f64 {
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
}

#[cfg(test)]
#[path = "reservoir_tests.rs"]
mod reservoir_tests;

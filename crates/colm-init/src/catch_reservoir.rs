//! `MOD_Catch_Reservoir`：流域网格上的水库（`DEF_Reservoir_Method > 0`）。
//!
//! 参数来自 `DEF_dir_runtime/HydroLAKES_Reservoir.nc`（`volresv/qmean/qflood/build_year/dam_height`），
//! 按流域的 `lake_id` 与出口格点 `ilat/ilon_outlet` 同时相等匹配（`reservoir_init`）；网格文件
//! 本身只给 `lake_id` 与出口坐标。`lake_type >= 2`（水库与受控湖）的流域各占一个水库序号。
//! 舍入形状取自 GIMPLE（`MOD_Catch_Reservoir.F90` 行号见注释）。

use std::path::Path;

use anyhow::{ensure, Context, Result};
use colm_core::LibmPow;

use crate::catch_network::{read_f64_any, read_i32_any, RiverLakeNetwork};

const SPVAL: f64 = colm_core::MISSING;

/// 流域水库的静态参数（水库序号 = `lake_type >= 2` 的流域按流域序）。
#[derive(Debug, Clone, Default)]
pub struct CatchReservoirs {
    /// 每个流域的水库序号（`bsn2resv`）。
    pub bsn2resv: Vec<Option<usize>>,
    pub dam_elv: Vec<f64>,
    pub volresv_total: Vec<f64>,
    pub volresv_emerg: Vec<f64>,
    pub volresv_adjust: Vec<f64>,
    pub volresv_normal: Vec<f64>,
    pub qresv_mean: Vec<f64>,
    pub qresv_flood: Vec<f64>,
    pub qresv_adjust: Vec<f64>,
    pub qresv_normal: Vec<f64>,
    pub dam_build_year: Vec<i32>,
    /// 升序去重的 HydroLAKES 编号（history/参数文件的 `reservoir` 维）。
    pub resv_hylak_id: Vec<i32>,
    /// 每个水库在 `resv_hylak_id` 里的位置（`resv_loc2glb`）。
    pub resv_loc2glb: Vec<usize>,
}

impl CatchReservoirs {
    pub fn numresv(&self) -> usize {
        self.dam_elv.len()
    }

    /// `reservoir_init`（单进程）。
    pub fn read(mesh_file: &Path, runtime_dir: &Path, network: &RiverLakeNetwork) -> Result<Self> {
        let mesh = netcdf::open(mesh_file)
            .with_context(|| format!("cannot open {}", mesh_file.display()))?;
        let lake_id_basin = read_i32_any(&mesh, "lake_id")?;
        let ilat_basin = read_i32_any(&mesh, "ilat_outlet")?;
        let ilon_basin = read_i32_any(&mesh, "ilon_outlet")?;
        let nbasin = lake_id_basin.len();
        ensure!(
            nbasin == network.lake_id.len()
                && ilat_basin.len() == nbasin
                && ilon_basin.len() == nbasin,
            "{} has basin vectors of different lengths",
            mesh_file.display()
        );
        let maxlakeid = lake_id_basin.iter().copied().max().unwrap_or(0);

        let mut vol = vec![SPVAL; nbasin];
        let mut qmean = vec![SPVAL; nbasin];
        let mut qflood = vec![SPVAL; nbasin];
        let mut year = vec![-99; nbasin];
        let mut dhgt = vec![SPVAL; nbasin];
        // `lake_id2typ(-1:maxlakeid)`，同一编号后写的覆盖先写的。
        let mut lake_id2typ = vec![0; (maxlakeid.max(0) + 2) as usize];
        let typ = |table: &[i32], id: i32| -> i32 {
            usize::try_from(id + 1)
                .ok()
                .and_then(|k| table.get(k).copied())
                .unwrap_or(0)
        };
        if maxlakeid > 0 {
            let path = runtime_dir.join("HydroLAKES_Reservoir.nc");
            let file =
                netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
            let ids = read_i32_any(&file, "hylak_id")?;
            let types = read_i32_any(&file, "lake_type")?;
            let ilat = read_i32_any(&file, "ilat_outlet")?;
            let ilon = read_i32_any(&file, "ilon_outlet")?;
            let r_vol = read_f64_any(&file, "volresv")?;
            let r_qmean = read_f64_any(&file, "qmean")?;
            let r_qflood = read_f64_any(&file, "qflood")?;
            let r_year = read_i32_any(&file, "build_year")?;
            let r_dhgt = read_f64_any(&file, "dam_height")?;
            let n = ids.len();
            ensure!(
                [
                    types.len(),
                    ilat.len(),
                    ilon.len(),
                    r_vol.len(),
                    r_qmean.len()
                ]
                .iter()
                .chain(&[r_qflood.len(), r_year.len(), r_dhgt.len()])
                .all(|&len| len == n),
                "{} has reservoir vectors of different lengths",
                path.display()
            );
            for (&id, &kind) in ids.iter().zip(&types) {
                if id <= maxlakeid && id >= -1 {
                    lake_id2typ[(id + 1) as usize] = kind;
                }
            }
            for b in 0..nbasin {
                if typ(&lake_id2typ, lake_id_basin[b]) < 2 {
                    continue;
                }
                for r in 0..n {
                    if lake_id_basin[b] == ids[r]
                        && ilat_basin[b] == ilat[r]
                        && ilon_basin[b] == ilon[r]
                    {
                        vol[b] = r_vol[r];
                        qmean[b] = r_qmean[r];
                        qflood[b] = r_qflood[r];
                        year[b] = r_year[r];
                        dhgt[b] = r_dhgt[r];
                    }
                }
            }
        }

        let mut out = Self {
            bsn2resv: vec![None; nbasin],
            ..Self::default()
        };
        for b in 0..nbasin {
            if network.lake_type[b] < 2 {
                continue;
            }
            out.bsn2resv[b] = Some(out.dam_elv.len());
            let info = &network.lakeinfo[b];
            // `:327-329` `MIN (MAX (MAX (dh, wtsrfelv - bedelv), surface (vol)), 335)`
            let mut height = dhgt[b].max(network.wtsrfelv[b] - network.bedelv[b]);
            height = height.max(info.surface(vol[b]));
            height = height.min(335.0);
            let total = info.volume(height);
            let normal = total * 0.7;
            // `:338` `FMA (qmean, 0.25, (normal * 0.7) / 15552000)`
            let qnormal = qmean[b].mul_add(0.25, (normal * 0.7) / 1.5552e7);
            out.dam_elv.push(network.bedelv[b] + height);
            out.volresv_total.push(total);
            out.volresv_emerg.push(total * 0.94);
            out.volresv_adjust.push(total * 0.77);
            out.volresv_normal.push(normal);
            out.qresv_mean.push(qmean[b]);
            out.qresv_flood.push(qflood[b]);
            out.qresv_normal.push(qnormal);
            out.qresv_adjust.push((qnormal + qflood[b]) * 0.5);
            out.dam_build_year.push(year[b]);
        }

        // `:345-389`：`lake_id2typ >= 2` 的流域编号去重升序；每个水库找到它的位置。
        let mut uniq: Vec<i32> = lake_id_basin
            .iter()
            .copied()
            .filter(|&id| typ(&lake_id2typ, id) >= 2)
            .collect();
        uniq.sort_unstable();
        uniq.dedup();
        for b in 0..nbasin {
            if out.bsn2resv[b].is_some() {
                let k = uniq
                    .binary_search(&network.lake_id[b])
                    .ok()
                    .with_context(|| {
                        format!("reservoir lake {} is not listed", network.lake_id[b])
                    })?;
                out.resv_loc2glb.push(k);
            }
        }
        out.resv_hylak_id = uniq;
        Ok(out)
    }

    /// `reservoir_operation (method, irsv, qin, vol, qout)`；只有方法 1（Funato 等，CaMa-Flood v4.20）。
    pub fn operation(&self, irsv: usize, qin: f64, vol: f64) -> f64 {
        let emerg = self.volresv_emerg[irsv];
        let adjust = self.volresv_adjust[irsv];
        let normal = self.volresv_normal[irsv];
        let qflood = self.qresv_flood[irsv];
        let qadjust = self.qresv_adjust[irsv];
        let qnormal = self.qresv_normal[irsv];
        if vol > emerg {
            qin.max(qflood)
        } else if vol > adjust {
            // `:437-438` `FMA (qflood - qadjust, pow ((vol - adjust) / (emerg - adjust), 0.1), qadjust)`
            let qout =
                (qflood - qadjust).mul_add(((vol - adjust) / (emerg - adjust)).lpow(0.1), qadjust);
            if qin > qflood {
                // `:440-441` `qnormal + ((qin - qnormal) * (vol - normal)) / (emerg - normal)`
                let q1 = qnormal + ((qin - qnormal) * (vol - normal)) / (emerg - normal);
                qout.max(q1)
            } else {
                qout
            }
        } else if vol > normal {
            // `:445-446`
            (qadjust - qnormal).mul_add(((vol - normal) / (adjust - normal)).lpow(3.0), qnormal)
        } else {
            // `:448` `pow (vol / normal, 0.5) * qnormal`
            (vol / normal).lpow(0.5) * qnormal
        }
    }

    /// `reservoir_gather_var`：同一湖泊的多个水库按序相加（`spval` 不计）。
    pub fn gather(&self, values: &[f64]) -> Vec<f64> {
        let mut out = vec![SPVAL; self.resv_hylak_id.len()];
        for (irsv, &v) in values.iter().enumerate() {
            if v != SPVAL {
                let g = self.resv_loc2glb[irsv];
                out[g] = if out[g] != SPVAL { out[g] + v } else { v };
            }
        }
        out
    }
}

#[cfg(test)]
#[path = "catch_reservoir_tests.rs"]
mod catch_reservoir_tests;

//! 单元流域网络（`MOD_Grid_RiverLakeNetwork` 的 `build_riverlake_network`）。
//!
//! 单进程下全部单元流域归同一个 worker，顺序就是文件顺序；河系按河口分组（`irivsys`），
//! 子步长在河系内取最小，所以只有分组影响结果，河系编号不影响。

use std::collections::HashMap;
use std::path::Path;

use anyhow::{ensure, Context, Result};

use crate::spatial::grid::LatLonGrid;
use crate::spatial::mapping::AreaWeightedMapping;
use crate::spatial::topology::SpatialTopology;

/// 河口入海（`seq_next = -9`）。
pub const RIVER_MOUTH: i32 = -9;
/// 内陆洼地（`seq_next = -10`）。
pub const INLAND_DEPRESSION: i32 = -10;

/// 一个单元流域的水位—蓄量—淹没面积曲线（`vol_dep_curve_type`）。下标 0 是河槽顶。
#[derive(Debug, Clone, PartialEq)]
pub struct FloodplainCurve {
    pub rivhgt: f64,
    pub rivare: f64,
    pub rivstomax: f64,
    pub flphgt: Vec<f64>,
    pub flparea: Vec<f64>,
    pub flpaccare: Vec<f64>,
    pub flpstomax: Vec<f64>,
}

impl FloodplainCurve {
    /// `build_riverlake_network` 里的曲线构造。
    ///
    /// `flpstomax(j) = FMA(h(j)-h(j-1), (A(j)+A(j-1))*0.5, s(j-1))`；`flparea(0) = 0`，其余层是
    /// `topo_area/nlfp`；累积面积从 0 起逐层相加。`A` 在 `DEF_GridRiverLake_FloodplainStorageFix`
    /// 为假时是逐层面积 `flparea`（上游默认，深度与体积不互逆），为真时是累积面积 `flpaccare`。
    pub fn new(rivhgt: f64, rivstomax: f64, area: f64, fldhgt: &[f64], storage_fix: bool) -> Self {
        let nlfp = fldhgt.len();
        let rivare = rivstomax / rivhgt;
        let mut flphgt = Vec::with_capacity(nlfp + 1);
        flphgt.push(0.0);
        flphgt.extend_from_slice(fldhgt);
        let layer_area = area / nlfp as f64;
        let mut flparea = vec![layer_area; nlfp + 1];
        flparea[0] = 0.0;
        let mut flpaccare = vec![0.0; nlfp + 1];
        for j in 1..=nlfp {
            flpaccare[j] = flparea[j] + flpaccare[j - 1];
        }
        let trapezoid = if storage_fix { &flpaccare } else { &flparea };
        let mut flpstomax = vec![0.0; nlfp + 1];
        for j in 1..=nlfp {
            flpstomax[j] = (flphgt[j] - flphgt[j - 1])
                .mul_add((trapezoid[j] + trapezoid[j - 1]) * 0.5, flpstomax[j - 1]);
        }
        Self {
            rivhgt,
            rivare,
            rivstomax,
            flphgt,
            flparea,
            flpaccare,
            flpstomax,
        }
    }

    fn nlfp(&self) -> usize {
        self.flphgt.len() - 1
    }

    /// `retrieve_depth_from_volume`。
    pub fn depth(&self, volume: f64) -> f64 {
        let v0 = volume - self.rivstomax;
        if v0 <= 0.0 {
            return volume / self.rivare;
        }
        let n = self.nlfp();
        let mut i = 1;
        while i <= n && v0 > self.flpstomax[i] {
            i += 1;
        }
        if i == n + 1 {
            (self.rivhgt + self.flphgt[n]) + (v0 - self.flpstomax[n]) / self.flpaccare[n]
        } else {
            let g = (self.flphgt[i] - self.flphgt[i - 1]) / self.flparea[i];
            let a = self.flpaccare[i - 1];
            let root = a.mul_add(a, (v0 - self.flpstomax[i - 1]) * 2.0 / g).sqrt();
            (root - a).mul_add(g, self.rivhgt + self.flphgt[i - 1])
        }
    }

    /// 第一个 `depth <= rivhgt + flphgt(i)` 的层（全都更深时是 `nlfp + 1`）。
    fn layer_of(&self, depth: f64) -> usize {
        let n = self.nlfp();
        let mut i = 1;
        while i <= n && depth > self.rivhgt + self.flphgt[i] {
            i += 1;
        }
        i
    }

    /// `retrieve_volume_from_depth`。
    pub fn volume(&self, depth: f64) -> f64 {
        if depth <= self.rivhgt {
            return self.rivare * depth;
        }
        let n = self.nlfp();
        let i = self.layer_of(depth);
        let d = depth - self.rivhgt - self.flphgt[i - 1];
        if i == n + 1 {
            self.flpaccare[n].mul_add(d, self.rivstomax + self.flpstomax[n])
        } else {
            let h = self.flphgt[i] - self.flphgt[i - 1];
            let inner = (d / h).mul_add(self.flparea[i], self.flpaccare[i - 1] * 2.0);
            (inner * d).mul_add(0.5, self.rivstomax + self.flpstomax[i - 1])
        }
    }

    /// `retrieve_area_from_depth`。
    pub fn floodarea(&self, depth: f64) -> f64 {
        if depth <= self.rivhgt {
            return 0.0;
        }
        let n = self.nlfp();
        let i = self.layer_of(depth);
        if i == n + 1 {
            self.flpaccare[n]
        } else {
            let h = self.flphgt[i] - self.flphgt[i - 1];
            let d = depth - self.rivhgt - self.flphgt[i - 1];
            (d / h).mul_add(self.flparea[i], self.flpaccare[i - 1])
        }
    }
}

/// 一个河系（共用一个河口的单元流域）：`cells` 是全局序号（递增），`next`/`upstream` 用局部下标，
/// 河口/洼地保持原码。上游次序与全局相同（按序号递增）。
#[derive(Debug, Clone)]
pub struct RiverSystem {
    pub cells: Vec<usize>,
    pub next: Vec<i32>,
    pub upstream: Vec<Vec<usize>>,
}

/// 全球单元流域网络与区域 patch 到它的径流映射。
#[derive(Debug, Clone)]
pub struct RiverNetwork {
    pub x: Vec<i32>,
    pub y: Vec<i32>,
    /// 下游单元流域（0 起）；河口 [`RIVER_MOUTH`]、洼地 [`INLAND_DEPRESSION`] 保持原码。
    pub next: Vec<i32>,
    /// 每个单元流域的上游（0 起，按单元流域序号递增）。
    pub upstream: Vec<Vec<usize>>,
    /// 河系分组（按河口）。
    pub river_system: Vec<usize>,
    pub river_systems: usize,
    /// 每个河系的局部拓扑：河系之间没有任何耦合，汇流可以逐河系独立推进。
    pub systems: Vec<RiverSystem>,
    pub rivelv: Vec<f64>,
    pub rivhgt: Vec<f64>,
    pub rivlen: Vec<f64>,
    pub rivman: Vec<f64>,
    pub rivwth: Vec<f64>,
    pub rivare: Vec<f64>,
    pub rivstomax: Vec<f64>,
    pub area: Vec<f64>,
    pub curves: Vec<FloodplainCurve>,
    /// 下游河床高程；没有下游时是 `spval`。
    pub bedelv_next: Vec<f64>,
    /// 出口宽度：有下游时取与下游河宽的平均。
    pub outletwth: Vec<f64>,
    /// 单元流域网格（`griducat`）的经纬格数。
    pub nlon: usize,
    pub nlat: usize,
    /// `inpmat`：每个单元流域的 `(网格号, 面积)`，网格号 1 起 `(y-1)*nlon + x`，无效项为 `(0, 0)`。
    pub inpmat: Vec<Vec<(i64, f64)>>,
}

fn read<T: netcdf::NcTypeDescriptor + Copy>(file: &netcdf::File, name: &str) -> Result<Vec<T>> {
    file.variable(name)
        .with_context(|| format!("the unit-catchment file has no {name}"))?
        .get_values::<T, _>(..)
        .with_context(|| format!("cannot read {name} from the unit-catchment file"))
}

impl RiverNetwork {
    /// 读 `DEF_UnitCatchment_file`（全球网络，`DEF_UnitCatchment_regional = .false.`）。
    pub fn read(path: &Path, storage_fix: bool) -> Result<Self> {
        let file = netcdf::open(path)
            .with_context(|| format!("cannot open unit-catchment file {}", path.display()))?;
        let x = read::<i32>(&file, "seq_x")?;
        let y = read::<i32>(&file, "seq_y")?;
        let next_raw = read::<i32>(&file, "seq_next")?;
        let n = x.len();
        ensure!(
            n > 0 && y.len() == n && next_raw.len() == n,
            "the unit-catchment vectors disagree in length"
        );
        let nlon = file.dimension("nx").context("no nx dimension")?.len();
        let nlat = file.dimension("ny").context("no ny dimension")?.len();
        let nlfp = file.dimension("nlfp").context("no nlfp dimension")?.len();
        let inpn = file.dimension("inpn").context("no inpn dimension")?.len();
        let field = |name: &str| -> Result<Vec<f64>> {
            let values = read::<f64>(&file, name)?;
            ensure!(
                values.len() == n,
                "{name} does not cover every unit catchment"
            );
            Ok(values)
        };
        let rivelv = field("topo_rivelv")?;
        let rivhgt = field("topo_rivhgt")?;
        let rivlen = field("topo_rivlen")?;
        let rivman = field("topo_rivman")?;
        let rivwth = field("topo_rivwth")?;
        let rivstomax = field("topo_rivstomax")?;
        let area = field("topo_area")?;
        let fldhgt = read::<f64>(&file, "topo_fldhgt")?;
        ensure!(
            fldhgt.len() == n * nlfp,
            "topo_fldhgt is not (nseqmax, nlfp)"
        );
        let inp_x = read::<i32>(&file, "inpmat_x")?;
        let inp_y = read::<i32>(&file, "inpmat_y")?;
        let inp_area = read::<f64>(&file, "inpmat_area")?;
        ensure!(
            inp_x.len() == n * inpn && inp_y.len() == n * inpn && inp_area.len() == n * inpn,
            "inpmat is not (nseqmax, inpn)"
        );
        // `seq_next` 是 1 起的下游号；非正值是河口/洼地的码。
        let next = next_raw
            .iter()
            .map(|&j| {
                if j > 0 {
                    ensure!((j as usize) <= n, "seq_next points outside the network");
                    Ok(j - 1)
                } else {
                    ensure!(
                        j == RIVER_MOUTH || j == INLAND_DEPRESSION,
                        "seq_next {j} is neither a river mouth (-9) nor an inland depression (-10)"
                    );
                    Ok(j)
                }
            })
            .collect::<Result<Vec<_>>>()?;
        let mut upstream = vec![Vec::new(); n];
        for (i, &j) in next.iter().enumerate() {
            if j >= 0 {
                upstream[j as usize].push(i);
            }
        }
        // 河口：沿下游走到头；记忆化，网络是森林。
        let mut mouth = vec![usize::MAX; n];
        for start in 0..n {
            let mut path = Vec::new();
            let mut i = start;
            let found = loop {
                if mouth[i] != usize::MAX {
                    break mouth[i];
                }
                path.push(i);
                ensure!(path.len() <= n, "the river network has a cycle");
                match next[i] {
                    j if j >= 0 => i = j as usize,
                    _ => break i,
                }
            };
            for i in path {
                mouth[i] = found;
            }
        }
        let mut system_of_mouth = HashMap::new();
        let river_system = mouth
            .iter()
            .map(|&m| {
                let next_id = system_of_mouth.len();
                *system_of_mouth.entry(m).or_insert(next_id)
            })
            .collect::<Vec<_>>();
        let mut system_cells = vec![Vec::new(); system_of_mouth.len()];
        for (i, &system) in river_system.iter().enumerate() {
            system_cells[system].push(i);
        }
        let mut local = vec![usize::MAX; n];
        for cells in &system_cells {
            for (k, &i) in cells.iter().enumerate() {
                local[i] = k;
            }
        }
        let systems = system_cells
            .into_iter()
            .map(|cells| RiverSystem {
                next: cells
                    .iter()
                    .map(|&i| {
                        let j = next[i];
                        if j >= 0 {
                            local[j as usize] as i32
                        } else {
                            j
                        }
                    })
                    .collect(),
                upstream: cells
                    .iter()
                    .map(|&i| upstream[i].iter().map(|&u| local[u]).collect())
                    .collect(),
                cells,
            })
            .collect::<Vec<_>>();
        let curves = (0..n)
            .map(|i| {
                FloodplainCurve::new(
                    rivhgt[i],
                    rivstomax[i],
                    area[i],
                    &fldhgt[i * nlfp..(i + 1) * nlfp],
                    storage_fix,
                )
            })
            .collect::<Vec<_>>();
        let rivare = curves.iter().map(|curve| curve.rivare).collect();
        let spval = colm_core::MISSING;
        let bedelv_next = next
            .iter()
            .map(|&j| if j >= 0 { rivelv[j as usize] } else { spval })
            .collect();
        let outletwth = next
            .iter()
            .enumerate()
            .map(|(i, &j)| {
                if j >= 0 {
                    (rivwth[j as usize] + rivwth[i]) * 0.5
                } else {
                    rivwth[i]
                }
            })
            .collect();
        let inpmat = (0..n)
            .map(|i| {
                (0..inpn)
                    .map(|k| {
                        let (gx, gy, a) = (
                            inp_x[i * inpn + k],
                            inp_y[i * inpn + k],
                            inp_area[i * inpn + k],
                        );
                        let id = (i64::from(gy) - 1) * nlon as i64 + i64::from(gx);
                        if a <= 0.0 || id <= 0 {
                            (0, 0.0)
                        } else {
                            (id, a)
                        }
                    })
                    .collect()
            })
            .collect();
        Ok(Self {
            river_systems: system_of_mouth.len(),
            systems,
            x,
            y,
            next,
            upstream,
            river_system,
            rivelv,
            rivhgt,
            rivlen,
            rivman,
            rivwth,
            rivare,
            rivstomax,
            area,
            curves,
            bedelv_next,
            outletwth,
            nlon,
            nlat,
            inpmat,
        })
    }

    pub fn len(&self) -> usize {
        self.x.len()
    }

    pub fn is_empty(&self) -> bool {
        self.x.is_empty()
    }
}

/// `inpmat` 的一项：落在区域网格里时是它在 `grids` 里的下标，否则 `None`；以及份面积。
pub type InpmatEntry = (Option<usize>, f64);

/// 区域 patch 的径流进入网络的路径：`remap_patch2inpm` → `push_inpm2ucat`。
#[derive(Debug, Clone)]
pub struct RunoffRouting {
    /// 被 patch 覆盖的单元流域网格（1 起的网格号，按号排序）。
    pub grids: Vec<i64>,
    /// 每个 patch 的份：`(grids 里的下标, 面积 m²)`。
    pub patch_parts: Vec<Vec<(usize, f64)>>,
    /// `push_ucat2inpm%sum_area`：每个网格里单元流域的份面积之和。
    pub grid_area: Vec<f64>,
    /// 用到这些网格的单元流域：`(单元流域, [(grids 下标或 None, 面积)])`，保持 `inpmat` 的次序。
    pub catchments: Vec<(usize, Vec<InpmatEntry>)>,
    /// `idmap_uc2gd`：每个网格里的 `(单元流域, 份面积)`，按单元流域序号、再按 `inpmat` 列递增。
    pub grid_catchments: Vec<Vec<(usize, f64)>>,
    /// `push_ucat2grid`：格心落在这个网格里的单元流域（`ucat_gdid`）。
    pub ucat_at_grid: Vec<Option<usize>>,
    /// `push_inpm2ucat%sum_area`：每个单元流域落在区域网格里的份面积之和（全网络）。
    pub catchment_area: Vec<f64>,
}

impl RunoffRouting {
    pub fn build(network: &RiverNetwork, topology: &SpatialTopology) -> Result<Self> {
        let grid = LatLonGrid::define_by_ndims(network.nlon, network.nlat)?;
        let mapping = AreaWeightedMapping::build(
            &grid,
            &topology.pixel,
            &topology.cells,
            &topology.shared_fraction,
        )?;
        let id_of = |ilon: usize, ilat: usize| ilat as i64 * network.nlon as i64 + ilon as i64 + 1;
        let mut grids = mapping
            .parts
            .iter()
            .flatten()
            .map(|part| id_of(part.ilon, part.ilat))
            .collect::<Vec<_>>();
        grids.sort_unstable();
        grids.dedup();
        let index = grids
            .iter()
            .enumerate()
            .map(|(k, &id)| (id, k))
            .collect::<HashMap<_, _>>();
        // `areapart` 是 km²，`build_worker_remapdata` 乘 1e6 成 m²。
        let patch_parts = mapping
            .parts
            .iter()
            .map(|parts| {
                parts
                    .iter()
                    .map(|part| (index[&id_of(part.ilon, part.ilat)], part.area * 1.0e6))
                    .collect()
            })
            .collect();
        let mut grid_area = vec![0.0; grids.len()];
        let mut grid_catchments = vec![Vec::new(); grids.len()];
        let mut catchment_area = vec![0.0; network.len()];
        let mut catchments = Vec::new();
        for (i, entries) in network.inpmat.iter().enumerate() {
            if !entries.iter().any(|(id, _)| index.contains_key(id)) {
                continue;
            }
            for &(id, area) in entries {
                if let Some(&k) = index.get(&id) {
                    grid_area[k] += area;
                    grid_catchments[k].push((i, area));
                    catchment_area[i] += area;
                }
            }
            catchments.push((
                i,
                entries
                    .iter()
                    .map(|&(id, area)| (index.get(&id).copied(), area))
                    .collect(),
            ));
        }
        let mut ucat_at_grid = vec![None; grids.len()];
        for i in 0..network.len() {
            let id = (i64::from(network.y[i]) - 1) * network.nlon as i64 + i64::from(network.x[i]);
            if let Some(&k) = index.get(&id) {
                ucat_at_grid[k] = Some(i);
            }
        }
        Ok(Self {
            grids,
            patch_parts,
            grid_area,
            catchments,
            grid_catchments,
            ucat_at_grid,
            catchment_area,
        })
    }
}

#[cfg(test)]
#[path = "network_tests.rs"]
mod network_tests;

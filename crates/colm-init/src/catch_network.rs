//! `CatchLateralFlow`（流域网格）的网络：单元/HRU/patch 三层拓扑，以及 `build_basin_network`、
//! `element_neighbour_init`、`hillslope_network_init`、`river_lake_network_init`、
//! `subsurface_network_init` 在**单进程**下的结果。
//!
//! 上游按 MPI 把流域分给各进程，单进程时退化成：`basinindex = 1..N`，`worker_push_data` 是按全局编号
//! 在单元与流域之间搬运（单元 `ie` 的 `eindex` 就是它对应的流域编号）。这里只做这一种情形——
//! Rust 主循环本来就是单进程。
//!
//! 每条浮点语句的舍入形状取自 `-fdump-tree-optimized-lineno` 的 GIMPLE（行号见注释）。

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};

use colm_core::LibmPow;

use crate::spatial_grid::areaquad;
use crate::spatial_static::{block_path, read_spatial_pixel_sets, values_i32, values_i64};

/// 一个流域网格算例的单元/HRU/patch 拓扑（`landelm`、`landhru`、`landpatch` 的单进程顺序）。
#[derive(Debug, Clone)]
pub struct CatchTopology {
    /// 按上游顺序排列的分块与各自的 patch 区间。
    pub blocks: Vec<(String, Range<usize>)>,
    pub pixel_lon_w: Vec<f64>,
    pub pixel_lon_e: Vec<f64>,
    pub pixel_lat_s: Vec<f64>,
    pub pixel_lat_n: Vec<f64>,
    /// 单元的全局编号 `landelm%eindex`（= 流域编号）。
    pub elements: Vec<i64>,
    /// `elm_patch%substt:subend`。
    pub elm_patch: Vec<Range<usize>>,
    /// `elm_hru%substt:subend`。
    pub elm_hru: Vec<Range<usize>>,
    /// `hru_patch%substt:subend`。
    pub hru_patch: Vec<Range<usize>>,
    /// `landhru%settyp`：HRU 编号（湖泊流域取负）。
    pub hru_type: Vec<i32>,
    /// `landpatch%settyp`。
    pub patch_type: Vec<i32>,
    /// 每个 patch 的像元 `(ilon, ilat)`（1 起），按网格内顺序。
    pub patch_cells: Vec<Vec<(i32, i32)>>,
    /// `patcharea`：`FMA(areaquad, 1e6, acc)` 的链（`MOD_Catch_LateralFlow.F90:69-71`），m²。
    pub patch_area: Vec<f64>,
    /// `elm_patch%subfrc`。
    pub elm_patch_frc: Vec<f64>,
    /// `hru_patch%subfrc`。
    pub hru_patch_frc: Vec<f64>,
    /// `elm_hru%subfrc`。
    pub elm_hru_frc: Vec<f64>,
}

impl CatchTopology {
    pub fn read(landdata: &Path, year: i32) -> Result<Self> {
        let blocks = sorted_blocks(landdata, "landpatch", "landpatch_", year)?;
        let mut topology = Self {
            blocks: Vec::new(),
            pixel_lon_w: Vec::new(),
            pixel_lon_e: Vec::new(),
            pixel_lat_s: Vec::new(),
            pixel_lat_n: Vec::new(),
            elements: Vec::new(),
            elm_patch: Vec::new(),
            elm_hru: Vec::new(),
            hru_patch: Vec::new(),
            hru_type: Vec::new(),
            patch_type: Vec::new(),
            patch_cells: Vec::new(),
            patch_area: Vec::new(),
            elm_patch_frc: Vec::new(),
            hru_patch_frc: Vec::new(),
            elm_hru_frc: Vec::new(),
        };
        for block in blocks {
            let patch_path = block_path(landdata, "landpatch", "landpatch", year, &block);
            let file = netcdf::open(&patch_path)
                .with_context(|| format!("cannot open {}", patch_path.display()))?;
            let patch_element = values_i64(&file, "eindex")?;
            let patch_start = values_i32(&file, "ipxstt")?;
            let patch_end = values_i32(&file, "ipxend")?;
            let patch_type = values_i32(&file, "settyp")?;
            ensure!(
                patch_element.len() == patch_start.len()
                    && patch_element.len() == patch_end.len()
                    && patch_element.len() == patch_type.len(),
                "{} has inconsistent patch vectors",
                patch_path.display()
            );
            ensure!(
                patch_start.iter().all(|&start| start > 0),
                "CatchLateralFlow patches cannot be WMO or shared-pixel patches"
            );
            let hru_path = block_path(landdata, "landhru", "landhru", year, &block);
            let file = netcdf::open(&hru_path)
                .with_context(|| format!("cannot open {}", hru_path.display()))?;
            let hru_element = values_i64(&file, "eindex")?;
            let hru_start = values_i32(&file, "ipxstt")?;
            let hru_end = values_i32(&file, "ipxend")?;
            let hru_type = values_i32(&file, "settyp")?;
            ensure!(
                hru_element.len() == hru_start.len()
                    && hru_element.len() == hru_end.len()
                    && hru_element.len() == hru_type.len(),
                "{} has inconsistent HRU vectors",
                hru_path.display()
            );
            let shared = vec![1.0; patch_element.len()];
            let sets = read_spatial_pixel_sets(
                landdata,
                year,
                &block,
                &patch_element,
                &patch_start,
                &patch_end,
                &shared,
                "patch",
            )?;
            if topology.blocks.is_empty() {
                topology.pixel_lon_w = sets.lon_w;
                topology.pixel_lon_e = sets.lon_e;
                topology.pixel_lat_s = sets.lat_s;
                topology.pixel_lat_n = sets.lat_n;
            }
            let patch_offset = topology.patch_type.len();
            let hru_offset = topology.hru_type.len();
            // `subset_build`：同一单元里，patch 的起始像元落在 HRU 的像元区间里就属于它。
            let mut ip = 0;
            let mut ih = 0;
            while ip < patch_element.len() {
                let element = patch_element[ip];
                let element_patches = ip..patch_element[ip..]
                    .iter()
                    .position(|&e| e != element)
                    .map_or(patch_element.len(), |n| ip + n);
                ensure!(
                    ih < hru_element.len() && hru_element[ih] == element,
                    "block {block}: element {element} has patches but its HRUs are out of order"
                );
                let element_hrus = ih..hru_element[ih..]
                    .iter()
                    .position(|&e| e != element)
                    .map_or(hru_element.len(), |n| ih + n);
                let elm_hru_start = topology.hru_type.len();
                let mut p = element_patches.start;
                for h in element_hrus.clone() {
                    let first = p;
                    while p < element_patches.end
                        && patch_start[p] >= hru_start[h]
                        && patch_start[p] <= hru_end[h]
                    {
                        p += 1;
                    }
                    ensure!(
                        p > first,
                        "block {block}: HRU {} of element {element} has no patch",
                        hru_type[h]
                    );
                    topology
                        .hru_patch
                        .push(patch_offset + first..patch_offset + p);
                    topology.hru_type.push(hru_type[h]);
                }
                ensure!(
                    p == element_patches.end,
                    "block {block}: element {element} has patches outside its HRUs"
                );
                topology.elements.push(element);
                topology
                    .elm_patch
                    .push(patch_offset + element_patches.start..patch_offset + element_patches.end);
                topology
                    .elm_hru
                    .push(elm_hru_start..topology.hru_type.len());
                ip = element_patches.end;
                ih = element_hrus.end;
            }
            ensure!(
                ih == hru_element.len(),
                "block {block}: some HRUs have no patch"
            );
            let _ = hru_offset;
            topology.patch_type.extend(patch_type);
            topology.patch_cells.extend(sets.cells);
            let first = topology.blocks.last().map_or(0, |(_, range)| range.end);
            topology
                .blocks
                .push((block, first..topology.patch_type.len()));
        }
        topology.compute_areas();
        Ok(topology)
    }

    fn cell_area(&self, (ilon, ilat): (i32, i32)) -> f64 {
        let x = (ilon - 1) as usize;
        let y = (ilat - 1) as usize;
        areaquad(
            self.pixel_lat_s[y],
            self.pixel_lat_n[y],
            self.pixel_lon_w[x],
            self.pixel_lon_e[x],
        )
    }

    fn compute_areas(&mut self) {
        // `patcharea(ip) = patcharea(ip) + 1.0e6 * areaquad(...)`：`.FMA (areaquad, 1.0e+6, acc)`。
        self.patch_area = self
            .patch_cells
            .iter()
            .map(|cells| {
                cells
                    .iter()
                    .fold(0.0, |acc, &cell| self.cell_area(cell).mul_add(1.0e6, acc))
            })
            .collect();
        // `subset_build`（`MOD_Pixelset.F90:660-681`）：子集逐像元 `subfrc + areaquad`（从 0 起），
        // 再除以父集内的和（从 0 起）。
        let patch_raw: Vec<f64> = self
            .patch_cells
            .iter()
            .map(|cells| {
                cells
                    .iter()
                    .fold(0.0, |acc, &cell| acc + self.cell_area(cell))
            })
            .collect();
        let normalize = |ranges: &[Range<usize>], raw: &[f64]| {
            let mut out = raw.to_vec();
            for range in ranges {
                let total = raw[range.clone()].iter().fold(0.0, |acc, v| acc + v);
                for value in &mut out[range.clone()] {
                    *value /= total;
                }
            }
            out
        };
        self.elm_patch_frc = normalize(&self.elm_patch, &patch_raw);
        self.hru_patch_frc = normalize(&self.hru_patch, &patch_raw);
        // HRU 的原始面积：HRU 的像元（= 其 patch 的像元按序拼接）逐个相加。
        let hru_raw: Vec<f64> = self
            .hru_patch
            .iter()
            .map(|patches| {
                self.patch_cells[patches.clone()]
                    .iter()
                    .flatten()
                    .fold(0.0, |acc, &cell| acc + self.cell_area(cell))
            })
            .collect();
        self.elm_hru_frc = normalize(&self.elm_hru, &hru_raw);
    }

    /// 单元的像元（= 其 patch 的像元按序拼接）。
    pub fn element_cells(&self, element: usize) -> Vec<(i32, i32)> {
        self.patch_cells[self.elm_patch[element].clone()]
            .iter()
            .flatten()
            .copied()
            .collect()
    }

    pub fn numelm(&self) -> usize {
        self.elements.len()
    }

    pub fn numhru(&self) -> usize {
        self.hru_type.len()
    }

    pub fn numpatch(&self) -> usize {
        self.patch_type.len()
    }

    /// `hru_patch` 的反查：patch → HRU。
    pub fn patch_hru(&self) -> Vec<usize> {
        let mut out = vec![0; self.numpatch()];
        for (h, range) in self.hru_patch.iter().enumerate() {
            for p in range.clone() {
                out[p] = h;
            }
        }
        out
    }
}

/// 某个分块目录下的块名，按 `block_set_local_blocks` 的顺序（外层经度、内层纬度，自西南向东北）。
pub fn sorted_blocks(
    landdata: &Path,
    directory: &str,
    prefix: &str,
    year: i32,
) -> Result<Vec<String>> {
    let dir: PathBuf = landdata.join(directory).join(format!("{year:04}"));
    let mut keyed = std::fs::read_dir(&dir)
        .with_context(|| format!("cannot list {}", dir.display()))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.strip_prefix(prefix)
                .and_then(|rest| rest.strip_suffix(".nc"))
                .map(str::to_string)
        })
        .map(|block| block_origin(&block).map(|origin| (origin, block)))
        .collect::<Result<Vec<_>>>()?;
    ensure!(!keyed.is_empty(), "{} holds no block file", dir.display());
    keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).expect("block origins are finite"));
    Ok(keyed.into_iter().map(|(_, block)| block).collect())
}

/// 块名 → `(lon_w, lat_s)`：`e110_n20` 是 (110, 20)。
fn block_origin(name: &str) -> Result<(f64, f64)> {
    let parse = |part: &str, positive: char, negative: char| -> Result<f64> {
        let (sign, digits) = match part.chars().next() {
            Some(c) if c == positive => (1.0, &part[1..]),
            Some(c) if c == negative => (-1.0, &part[1..]),
            _ => bail!("block name {name:?} does not follow <e|w>DDD_<n|s>DD"),
        };
        let value = digits
            .parse::<u32>()
            .with_context(|| format!("block name {name:?} has a non-numeric edge"))?;
        Ok(sign * f64::from(value))
    };
    let (lon, lat) = name
        .split_once('_')
        .with_context(|| format!("block name {name:?} does not follow <e|w>DDD_<n|s>DD"))?;
    Ok((parse(lon, 'e', 'w')?, parse(lat, 'n', 's')?))
}

/// 按流域编号（1..N）索引单元：`push_elm2bsn` 的单进程形式。没有陆地单元的流域为 `None`。
pub fn basin_to_element(topology: &CatchTopology, numbasin: usize) -> Result<Vec<Option<usize>>> {
    let mut out = vec![None; numbasin];
    for (ie, &id) in topology.elements.iter().enumerate() {
        let index = usize::try_from(id - 1).context("element index must be positive")?;
        ensure!(
            index < numbasin,
            "element {id} exceeds the basin count {numbasin}"
        );
        ensure!(out[index].is_none(), "element {id} appears twice");
        out[index] = Some(ie);
    }
    Ok(out)
}

/// `basin_hru`：流域 i 的 HRU 区间（按流域顺序连续排列），以及流域 HRU ↔ 单元 HRU 的对应。
#[derive(Debug, Clone)]
pub struct BasinHru {
    pub basin_hru: Vec<Range<usize>>,
    /// 流域 HRU 下标 → 单元 HRU 下标。
    pub to_element_hru: Vec<usize>,
}

impl BasinHru {
    pub fn build(topology: &CatchTopology, basin_element: &[Option<usize>]) -> Self {
        let mut basin_hru = Vec::with_capacity(basin_element.len());
        let mut to_element_hru = Vec::new();
        for element in basin_element {
            let start = to_element_hru.len();
            if let Some(ie) = element {
                to_element_hru.extend(topology.elm_hru[*ie].clone());
            }
            basin_hru.push(start..to_element_hru.len());
        }
        Self {
            basin_hru,
            to_element_hru,
        }
    }

    pub fn numbsnhru(&self) -> usize {
        self.to_element_hru.len()
    }

    /// `worker_push_data (push_elmhru2bsnhru, hru, bsnhru, fill)`。
    pub fn to_basin<T: Copy>(&self, hru: &[T]) -> Vec<T> {
        self.to_element_hru.iter().map(|&h| hru[h]).collect()
    }

    /// `worker_push_data (push_bsnhru2elmhru, bsnhru, hru, fill)`：没有对应流域 HRU 的单元 HRU 取 `fill`。
    pub fn to_element<T: Copy>(&self, bsnhru: &[T], numhru: usize, fill: T) -> Vec<T> {
        let mut out = vec![fill; numhru];
        for (b, &h) in self.to_element_hru.iter().enumerate() {
            out[h] = bsnhru[b];
        }
        out
    }
}

/// 流域按编号读入的静态参数（`river_lake_network_init` 第 1 步与 `build_basin_network` 的湖泊类型）。
#[derive(Debug, Clone)]
pub struct BasinInputs {
    pub lake_id: Vec<i32>,
    /// 0：非湖；1：天然湖；2：水库；3：受控湖。
    pub lake_type: Vec<i32>,
    pub riverdown: Vec<i32>,
    /// `river_length * 1e3`（km → m）。
    pub riverlen: Vec<f64>,
    pub riverelv: Vec<f64>,
    pub basinelv: Vec<f64>,
    /// `DEF_USE_EstimatedRiverDepth = .false.` 时从网格文件读的 `river_depth`。
    pub riverdpth: Option<Vec<f64>>,
}

impl BasinInputs {
    pub fn read(mesh_file: &Path, runtime_dir: &Path, estimated_depth: bool) -> Result<Self> {
        let file = netcdf::open(mesh_file)
            .with_context(|| format!("cannot open {}", mesh_file.display()))?;
        let lake_id = read_i32_any(&file, "lake_id")?;
        let riverdown = read_i32_any(&file, "basin_downstream")?;
        // `riverlen = riverlen * 1.e3`（`RiverLakeNetwork.F90:176`）：变量是 float，读成 r8 后乘。
        let riverlen = read_f64_any(&file, "river_length")?
            .into_iter()
            .map(|x| x * 1.0e3)
            .collect::<Vec<_>>();
        let riverelv = read_f64_any(&file, "river_elevation")?;
        let basinelv = read_f64_any(&file, "basin_elevation")?;
        let riverdpth = if estimated_depth {
            None
        } else {
            Some(read_f64_any(&file, "river_depth")?)
        };
        let n = riverdown.len();
        ensure!(
            lake_id.len() == n && riverlen.len() == n && riverelv.len() == n && basinelv.len() == n,
            "catchment basin vectors have different lengths"
        );
        let lake_type = lake_types(&lake_id, runtime_dir)?;
        Ok(Self {
            lake_id,
            lake_type,
            riverdown,
            riverlen,
            riverelv,
            basinelv,
            riverdpth,
        })
    }

    pub fn numbasin(&self) -> usize {
        self.riverdown.len()
    }
}

/// `build_basin_network`（`:376-401`）：`lake_id > 0` 的流域默认是天然湖（1）；在
/// `HydroLAKES_Reservoir.nc` 里查到 `hylak_id` 的取它的 `lake_type`。
fn lake_types(lake_id: &[i32], runtime_dir: &Path) -> Result<Vec<i32>> {
    let path = runtime_dir.join("HydroLAKES_Reservoir.nc");
    let file = netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
    let ids = read_i32_any(&file, "hylak_id")?;
    let types = read_i32_any(&file, "lake_type")?;
    ensure!(
        ids.len() == types.len(),
        "{} vectors differ",
        path.display()
    );
    // 上游先 quicksort 再二分；同一个 hylak_id 出现多次时取哪一个取决于排序，这里要求唯一。
    let mut table = BTreeMap::new();
    for (&id, &kind) in ids.iter().zip(&types) {
        if table.insert(id, kind).is_some_and(|old| old != kind) {
            bail!(
                "{} lists hylak_id {id} twice with different types",
                path.display()
            );
        }
    }
    Ok(lake_id
        .iter()
        .map(|&id| {
            if id > 0 {
                table.get(&id).copied().unwrap_or(1)
            } else {
                0
            }
        })
        .collect())
}

pub(crate) fn read_i32_any(file: &netcdf::File, name: &str) -> Result<Vec<i32>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("missing variable {name}"))?;
    variable
        .get_values::<i32, _>(..)
        .with_context(|| format!("cannot read {name}"))
}

pub(crate) fn read_i64_any(file: &netcdf::File, name: &str) -> Result<Vec<i64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("missing variable {name}"))?;
    variable
        .get_values::<i64, _>(..)
        .with_context(|| format!("cannot read {name}"))
}

pub(crate) fn read_f64_any(file: &netcdf::File, name: &str) -> Result<Vec<f64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("missing variable {name}"))?;
    variable
        .get_values::<f64, _>(..)
        .with_context(|| format!("cannot read {name}"))
}

/// `element_neighbour_type`（单进程）：邻居的全局编号、距离、边界长度、面积、高程、坡度。
#[derive(Debug, Clone, Default)]
pub struct ElementNeighbour {
    pub myarea: f64,
    pub myelva: f64,
    /// 邻居的全局编号；`-9` 是海洋。
    pub glbindex: Vec<i64>,
    /// 邻居在本进程单元里的位置（海洋或非正编号为 `None`）。
    pub local: Vec<Option<usize>>,
    pub dist: Vec<f64>,
    pub lenbdr: Vec<f64>,
    pub area: Vec<f64>,
    pub elva: Vec<f64>,
    pub slope: Vec<f64>,
}

impl ElementNeighbour {
    pub fn nnb(&self) -> usize {
        self.glbindex.len()
    }
}

/// `element_neighbour_init`：读 `num_neighbour`/`idx_neighbour`/`len_border`（`len_border * 1e3`，
/// km → m，`:134`），距离是两单元中心的 `arclen * 1e3` 且不小于 90 m（`:606`），面积是 patch 面积
/// 从 0 起相加（`:617`），高程是 `FMA(elv_patches, subfrc, acc)` 的链（`:618`），坡度
/// `|Δ高程| / 距离`（`:637`）。
pub fn element_neighbours(
    topology: &CatchTopology,
    neighbour_file: &Path,
    elevation_patches: &[f64],
) -> Result<Vec<ElementNeighbour>> {
    let file = netcdf::open(neighbour_file)
        .with_context(|| format!("cannot open {}", neighbour_file.display()))?;
    let nnball = read_i32_any(&file, "num_neighbour")?;
    let idx = file
        .variable("idx_neighbour")
        .context("missing variable idx_neighbour")?;
    let maxnnb = idx
        .dimensions()
        .get(1)
        .map(netcdf::Dimension::len)
        .context("idx_neighbour must be 2-D")?;
    let idxnball = idx.get_values::<i64, _>(..)?;
    let lenbdall = read_f64_any(&file, "len_border")?;
    ensure!(
        idxnball.len() == nnball.len() * maxnnb && lenbdall.len() == idxnball.len(),
        "neighbour vectors have inconsistent shapes"
    );
    ensure!(
        elevation_patches.len() == topology.numpatch(),
        "one patch elevation per patch is needed"
    );
    let mut local_of = BTreeMap::new();
    for (ie, &id) in topology.elements.iter().enumerate() {
        local_of.insert(id, ie);
    }
    let pixel_sets = crate::spatial_static::SpatialPixelSets {
        lon_w: topology.pixel_lon_w.clone(),
        lon_e: topology.pixel_lon_e.clone(),
        lat_s: topology.pixel_lat_s.clone(),
        lat_n: topology.pixel_lat_n.clone(),
        cells: Vec::new(),
        shared_fraction: Vec::new(),
    };
    // `landelm%get_lonlat_radian`：单元的全部像元按网格顺序。
    let mut rlon = Vec::with_capacity(topology.numelm());
    let mut rlat = Vec::with_capacity(topology.numelm());
    for ie in 0..topology.numelm() {
        let (lon, lat) = pixel_sets.mean(&topology.element_cells(ie))?;
        rlon.push(lon * std::f64::consts::PI / 180.0);
        rlat.push(lat * std::f64::consts::PI / 180.0);
    }
    let mut neighbours = Vec::with_capacity(topology.numelm());
    for (ie, &id) in topology.elements.iter().enumerate() {
        let index = usize::try_from(id - 1).context("element index must be positive")?;
        ensure!(
            index < nnball.len(),
            "element {id} is outside the neighbour file"
        );
        let nnb = usize::try_from(nnball[index]).context("num_neighbour is negative")?;
        ensure!(
            nnb <= maxnnb,
            "element {id} lists more neighbours than the file holds"
        );
        let row = index * maxnnb;
        let glbindex = idxnball[row..row + nnb].to_vec();
        let lenbdr = lenbdall[row..row + nnb]
            .iter()
            .map(|x| x * 1.0e3)
            .collect::<Vec<_>>();
        let local =
            glbindex
                .iter()
                .map(|&g| {
                    if g <= 0 {
                        return Ok(None);
                    }
                    // 上游单进程时编号为正却不在本进程的邻居会越界（`p_itis_worker(-1)`）；这里直接报错。
                    local_of.get(&g).copied().map(Some).with_context(|| {
                        format!("element {id} neighbour {g} is not a land element")
                    })
                })
                .collect::<Result<Vec<_>>>()?;
        let patches = topology.elm_patch[ie].clone();
        let myarea = topology.patch_area[patches.clone()]
            .iter()
            .fold(0.0, |acc, a| acc + a);
        let myelva = elevation_patches[patches.clone()]
            .iter()
            .zip(&topology.elm_patch_frc[patches])
            .fold(0.0, |acc, (e, f)| e.mul_add(*f, acc));
        neighbours.push(ElementNeighbour {
            myarea,
            myelva,
            glbindex,
            local,
            dist: vec![0.0; nnb],
            lenbdr,
            area: vec![0.0; nnb],
            elva: vec![0.0; nnb],
            slope: vec![0.0; nnb],
        });
    }
    for ie in 0..neighbours.len() {
        for inb in 0..neighbours[ie].nnb() {
            if let Some(je) = neighbours[ie].local[inb] {
                let dist = (crate::spatial_mapping::arclen(rlat[ie], rlon[ie], rlat[je], rlon[je])
                    * 1.0e3)
                    .max(90.0);
                let (area, elva) = (neighbours[je].myarea, neighbours[je].myelva);
                let me = &mut neighbours[ie];
                me.dist[inb] = dist;
                me.area[inb] = area;
                me.elva[inb] = elva;
                me.slope[inb] = (elva - me.myelva).abs() / dist;
            }
        }
    }
    Ok(neighbours)
}

/// `hillslope_network_type`。湖泊流域（`hydrounit_index` 全为负）只有 `nhru`，数组为空。
#[derive(Debug, Clone, Default)]
pub struct Hillslope {
    /// `nhru_in_bsn`（文件里的 `basin_numhru`）。
    pub nhru: usize,
    /// 湖泊：上游 `ihru` 等为 `null()`。
    pub is_lake: bool,
    /// HRU 在本组（流域 HRU 或单元 HRU）向量里的位置（0 起）。
    pub ihru: Vec<usize>,
    pub indx: Vec<i32>,
    pub area: Vec<f64>,
    pub agwt: Vec<f64>,
    pub hand: Vec<f64>,
    pub elva: Vec<f64>,
    pub plen: Vec<f64>,
    pub flen: Vec<f64>,
    /// 下游 HRU 在本组里的位置（0 起）；没有下游为 `None`。
    pub inext: Vec<Option<usize>>,
    /// `fldprof(nfldstep, nhru)`：`fldprof[i][j]`。
    pub fldprof: Vec<Vec<f64>>,
}

/// 网格文件里按流域编号的坡面参数。
#[derive(Debug, Clone)]
pub struct HillslopeInputs {
    pub maxnumhru: usize,
    pub nfldstep: usize,
    pub numhru: Vec<i32>,
    pub index: Vec<i32>,
    pub hand: Vec<f64>,
    pub elva: Vec<f64>,
    pub plen: Vec<f64>,
    pub flen: Vec<f64>,
    pub next: Vec<i32>,
    pub fldstep: Vec<f64>,
}

impl HillslopeInputs {
    pub fn read(mesh_file: &Path) -> Result<Self> {
        let file = netcdf::open(mesh_file)
            .with_context(|| format!("cannot open {}", mesh_file.display()))?;
        let fld = file
            .variable("hydrounit_flood_step")
            .context("missing variable hydrounit_flood_step")?;
        let dims = fld.dimensions();
        ensure!(dims.len() == 3, "hydrounit_flood_step must be 3-D");
        let maxnumhru = dims[1].len();
        let nfldstep = dims[2].len();
        let fldstep = fld.get_values::<f64, _>(..)?;
        let inputs = Self {
            maxnumhru,
            nfldstep,
            numhru: read_i32_any(&file, "basin_numhru")?,
            index: read_i32_any(&file, "hydrounit_index")?,
            hand: read_f64_any(&file, "hydrounit_hand")?,
            elva: read_f64_any(&file, "hydrounit_elva")?,
            plen: read_f64_any(&file, "hydrounit_pathlen")?,
            flen: read_f64_any(&file, "hydrounit_facelen")?,
            next: read_i32_any(&file, "hydrounit_downstream")?,
            fldstep,
        };
        let n = inputs.numhru.len();
        ensure!(
            inputs.index.len() == n * maxnumhru
                && inputs.hand.len() == n * maxnumhru
                && inputs.elva.len() == n * maxnumhru
                && inputs.plen.len() == n * maxnumhru
                && inputs.flen.len() == n * maxnumhru
                && inputs.next.len() == n * maxnumhru
                && inputs.fldstep.len() == n * maxnumhru * nfldstep,
            "hydrounit vectors have inconsistent shapes"
        );
        Ok(inputs)
    }
}

/// `hillslope_network_init (ne, elmindex, ...)`：按 `ids`（流域或单元的全局编号）的顺序建，
/// `ihru` 按 `nhru_in_bsn` 累计（`hs`），与调用方的 HRU 区间要一致（由调用方核对）。
pub fn hillslope_networks(inputs: &HillslopeInputs, ids: &[i64]) -> Result<Vec<Hillslope>> {
    let m = inputs.maxnumhru;
    let nf = inputs.nfldstep;
    let mut hs = 0;
    let mut out = Vec::with_capacity(ids.len());
    for &id in ids {
        let b = usize::try_from(id - 1).context("basin index must be positive")?;
        ensure!(
            b < inputs.numhru.len(),
            "basin {id} is outside the catchment file"
        );
        let index = &inputs.index[b * m..(b + 1) * m];
        let nhru = index.iter().filter(|&&x| x >= 0).count();
        let nhru_in_bsn = usize::try_from(inputs.numhru[b]).context("basin_numhru is negative")?;
        let mut net = Hillslope {
            nhru: nhru_in_bsn,
            ..Hillslope::default()
        };
        if nhru > 0 {
            // 上游只打印 'numbers of hydro units from file mismatch'；两者不同时后面的下标就错位了。
            ensure!(
                nhru == nhru_in_bsn,
                "basin {id}: {nhru} hydro units in hydrounit_index but basin_numhru is {nhru_in_bsn}"
            );
            net.indx = index[..nhru].to_vec();
            net.hand = inputs.hand[b * m..b * m + nhru].to_vec();
            net.elva = inputs.elva[b * m..b * m + nhru].to_vec();
            // `plen = plenhru * 1.0e3`、`flen = lfachru * 1.0e3`（`:397-398`）。
            net.plen = inputs.plen[b * m..b * m + nhru]
                .iter()
                .map(|x| x * 1.0e3)
                .collect();
            net.flen = inputs.flen[b * m..b * m + nhru]
                .iter()
                .map(|x| x * 1.0e3)
                .collect();
            net.ihru = (hs..hs + nhru).collect();
            net.area = vec![0.0; nhru];
            net.agwt = vec![0.0; nhru];
            net.fldprof = (0..nhru)
                .map(|i| {
                    let step = |j: usize| inputs.fldstep[(b * m + i) * nf + j];
                    let mut prof = vec![0.0; nf];
                    for j in 0..nf {
                        prof[j] = if j == 0 {
                            // `MAX_EXPR ((fldstep * 5.0e-1) / r(nfldstep), 1e-3)`（`:407`）
                            ((step(0) * 0.5) / nf as f64).max(1.0e-3)
                        } else {
                            // `prof(j-1) + ((Δfldstep * (r(j) - 0.5)) / r(nfldstep))`（`:410`）
                            prof[j - 1]
                                + ((step(j) - step(j - 1)) * ((j + 1) as f64 - 0.5)) / nf as f64
                        };
                    }
                    prof
                })
                .collect();
            let next = &inputs.next[b * m..b * m + nhru];
            net.inext = next
                .iter()
                .map(|&n| {
                    if n >= 0 {
                        // `findloc_ud(indxhru(1:nhru) == next)`：找不到时上游得 0（再当作 `<= 0`）。
                        index[..nhru].iter().position(|&x| x == n)
                    } else {
                        None
                    }
                })
                .collect();
        } else {
            net.is_lake = true;
        }
        hs += nhru_in_bsn;
        out.push(net);
    }
    Ok(out)
}

/// `calc_riverdepth_from_runoff`（单进程）：`runoff_clim.nc` 的 `ro`（m/day）取 `max(ro, 0)`
/// （`:1006`）后面积加权映射到单元（`grid2pset`），`(x / 24) / 3600`（`:1020`）、乘单元面积
/// （patch 面积从 0 起相加，`:1024-1026`），沿河网自上而下累加（`:1122-1129`），河深
/// `max(FMA(pow(Q, 0.5), 0.1, 0.0), 1.0)`（`:1133`）。返回按流域编号（1..N）的河深。
pub fn estimate_river_depths(
    topology: &CatchTopology,
    riverdown: &[i32],
    runtime_dir: &Path,
) -> Result<Vec<f64>> {
    let path = runtime_dir.join("runoff_clim.nc");
    let file = netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
    let lat = read_f64_any(&file, "lat")?;
    let lon = read_f64_any(&file, "lon")?;
    let grid = crate::spatial_grid::LatLonGrid::define_by_center(&lat, &lon, None)?;
    let ro = file
        .variable("ro")
        .context("runoff_clim.nc has no ro")?
        .get_values::<f64, _>(..)?;
    ensure!(ro.len() == lat.len() * lon.len(), "ro must be (lat, lon)");
    let nlon = lon.len();
    let pixel = crate::spatial_mapping::PixelAxes {
        lon_w: topology.pixel_lon_w.clone(),
        lon_e: topology.pixel_lon_e.clone(),
        lat_s: topology.pixel_lat_s.clone(),
        lat_n: topology.pixel_lat_n.clone(),
    };
    let cells = (0..topology.numelm())
        .map(|ie| topology.element_cells(ie))
        .collect::<Vec<_>>();
    let mapping = crate::spatial_mapping::AreaWeightedMapping::build(
        &grid,
        &pixel,
        &cells,
        &vec![1.0; cells.len()],
    )?;
    let n = riverdown.len();
    let mut bsnrnof = vec![0.0; n];
    for (ie, &id) in topology.elements.iter().enumerate() {
        let value = mapping.grid_to_set(ie, |ilon, ilat| ro[ilat * nlon + ilon].max(0.0));
        let myarea = topology.patch_area[topology.elm_patch[ie].clone()]
            .iter()
            .fold(0.0, |acc, a| acc + a);
        let index = usize::try_from(id - 1)?;
        ensure!(index < n, "element {id} exceeds the basin count");
        bsnrnof[index] = ((value / 24.0) / 3600.0) * myarea;
    }
    // 自上而下的次序 `b_up2down`（`:1084-1120`）。
    let mut nups = vec![0usize; n];
    for &down in riverdown {
        if down > 0 {
            nups[(down - 1) as usize] += 1;
        }
    }
    let mut iups = vec![0usize; n];
    let mut order = Vec::with_capacity(n);
    for i in 0..n {
        if iups[i] == nups[i] {
            order.push(i);
            let mut j = riverdown[i];
            while j > 0 {
                let jj = (j - 1) as usize;
                iups[jj] += 1;
                if iups[jj] == nups[jj] {
                    order.push(jj);
                    j = riverdown[jj];
                } else {
                    break;
                }
            }
        }
    }
    ensure!(order.len() == n, "basin_downstream contains a loop");
    let mut bsndis = vec![0.0; n];
    for &j in &order {
        bsndis[j] += bsnrnof[j];
        if riverdown[j] > 0 {
            let d = (riverdown[j] - 1) as usize;
            bsndis[d] += bsndis[j];
        }
    }
    Ok(bsndis
        .into_iter()
        .map(|q| q.lpow(0.5).mul_add(0.1, 0.0).max(1.0))
        .collect())
}

/// `lake_info_type`。
#[derive(Debug, Clone, Default)]
pub struct LakeInfo {
    pub nsub: usize,
    /// HRU 次序。
    pub area0: Vec<f64>,
    pub depth0: Vec<f64>,
    /// 自深到浅。
    pub area: Vec<f64>,
    pub depth: Vec<f64>,
    pub dep_vol_curve: Vec<f64>,
}

impl LakeInfo {
    /// `retrieve_lake_surface_from_volume`（`:1158-1176`）。
    pub fn surface(&self, volume: f64) -> f64 {
        if volume <= 0.0 {
            return 0.0;
        }
        if self.nsub == 1 {
            volume / self.area[0]
        } else {
            let mut i = 0;
            while i + 1 < self.nsub {
                if volume >= self.dep_vol_curve[i + 1] {
                    i += 1;
                } else {
                    break;
                }
            }
            let sum = self.area[..=i].iter().fold(0.0, |acc, a| acc + a);
            (self.depth[0] - self.depth[i]) + (volume - self.dep_vol_curve[i]) / sum
        }
    }

    /// `retrieve_lake_volume_from_surface`（`:1192-1210`）：`.FMA (surface - (d1 - di), sum, curve)`。
    pub fn volume(&self, surface: f64) -> f64 {
        if surface <= 0.0 {
            return 0.0;
        }
        if self.nsub == 1 {
            surface * self.area[0]
        } else {
            let mut i = 0;
            while i + 1 < self.nsub {
                if surface >= self.depth[0] - self.depth[i + 1] {
                    i += 1;
                } else {
                    break;
                }
            }
            let sum = self.area[..=i].iter().fold(0.0, |acc, a| acc + a);
            (surface - (self.depth[0] - self.depth[i])).mul_add(sum, self.dep_vol_curve[i])
        }
    }
}

/// 流域（河道/湖泊）网络：`river_lake_network_init` 的结果，按流域编号 1..N（0 起下标）。
#[derive(Debug, Clone)]
pub struct RiverLakeNetwork {
    pub lake_id: Vec<i32>,
    pub lake_type: Vec<i32>,
    pub riverdown: Vec<i32>,
    /// 下游流域的位置（0 起）；`riverdown <= 0` 为 `None`。
    pub ilocdown: Vec<Option<usize>>,
    pub to_lake: Vec<bool>,
    pub irivsys: Vec<usize>,
    pub numrivsys: usize,
    pub riverlen: Vec<f64>,
    pub riverelv: Vec<f64>,
    pub riverdpth: Vec<f64>,
    pub basinelv: Vec<f64>,
    pub riverarea: Vec<f64>,
    pub riverwth: Vec<f64>,
    pub bedelv: Vec<f64>,
    pub handmin: Vec<f64>,
    pub wtsrfelv: Vec<f64>,
    pub riverlen_ds: Vec<f64>,
    pub wtsrfelv_ds: Vec<f64>,
    pub riverwth_ds: Vec<f64>,
    pub bedelv_ds: Vec<f64>,
    pub outletwth: Vec<f64>,
    pub lakeinfo: Vec<LakeInfo>,
    /// `hillslope_basin`（`hand` 已加上河深，`area` 是流域 HRU 面积）。
    pub hillslope_basin: Vec<Hillslope>,
    pub basin_hru: BasinHru,
}

/// `river_lake_network_init (patcharea)` 的单进程形式。`lakedepth` 是读入处理后的 patch 湖深
/// （`MOD_LakeDepthReadin`），`riverdpth` 按流域编号。
#[allow(clippy::too_many_arguments)]
pub fn river_lake_network(
    topology: &CatchTopology,
    inputs: &BasinInputs,
    riverdpth: Vec<f64>,
    hillslope_inputs: &HillslopeInputs,
    neighbours: &[ElementNeighbour],
    lakedepth: &[f64],
) -> Result<RiverLakeNetwork> {
    let n = inputs.numbasin();
    ensure!(riverdpth.len() == n, "one river depth per basin is needed");
    let basin_element = basin_to_element(topology, n)?;
    let basin_hru = BasinHru::build(topology, &basin_element);
    let riverdown = inputs.riverdown.clone();
    let to_lake = riverdown
        .iter()
        .map(|&d| d > 0 && inputs.lake_id[(d - 1) as usize] > 0)
        .collect::<Vec<_>>();
    // 单进程：`basin_sorted = basinindex = 1..N`，`ilocdown = riverdown`。
    let ilocdown = riverdown
        .iter()
        .map(|&d| if d > 0 { Some((d - 1) as usize) } else { None })
        .collect::<Vec<_>>();
    // `riversystem == -1`：沿下游找到已编号的系统或河口（`:575-598`）。
    let mut irivsys: Vec<Option<usize>> = vec![None; n];
    let mut numrivsys = 0;
    for ibasin in 0..n {
        if irivsys[ibasin].is_none() {
            let mut route = vec![ibasin];
            let mut j = ibasin;
            while riverdown[j] > 0 && irivsys[j].is_none() {
                route.push(j);
                j = ilocdown[j].expect("riverdown > 0");
            }
            let sys = match irivsys[j] {
                Some(s) => s,
                None => {
                    numrivsys += 1;
                    irivsys[j] = Some(numrivsys - 1);
                    numrivsys - 1
                }
            };
            for r in route {
                irivsys[r] = Some(sys);
            }
        }
    }
    let irivsys = irivsys.into_iter().map(|s| s.expect("assigned")).collect();
    let ids = (1..=n as i64).collect::<Vec<_>>();
    let mut hillslope_basin = hillslope_networks(hillslope_inputs, &ids)?;
    for (b, net) in hillslope_basin.iter().enumerate() {
        let count = basin_hru.basin_hru[b].len();
        ensure!(
            count == 0 || count == net.nhru,
            "basin {}: {count} HRUs in landhru but basin_numhru is {}",
            b + 1,
            net.nhru
        );
        // `ihru` 按 `nhru_in_bsn` 累计；流域 HRU 区间按单元 HRU 数累计。两者必须一致。
        if !net.is_lake && count > 0 {
            ensure!(
                net.ihru.first() == Some(&basin_hru.basin_hru[b].start),
                "basin {}: hillslope HRU offsets disagree with landhru",
                b + 1
            );
        }
    }
    // `lake_id_elm`、`lakedown_id`、`lakeoutlet`（`:628-677`）。
    let numhru = topology.numhru();
    let mut unitarea_hru = vec![0.0; numhru];
    let mut lakedepth_hru = vec![0.0; numhru];
    let mut lakeoutlet_bsn = vec![0.0; n];
    for (ie, &id) in topology.elements.iter().enumerate() {
        let b = (id - 1) as usize;
        let lake_id_elm = inputs.lake_id[b];
        let mut lakedown = 0;
        if inputs.lake_id[b] != 0 && to_lake[b] {
            lakedown = riverdown[b];
        }
        if inputs.lake_id[b] > 0 && riverdown[b] == 0 {
            lakedown = -9;
        }
        for h in topology.elm_hru[ie].clone() {
            let patches = topology.hru_patch[h].clone();
            unitarea_hru[h] = topology.patch_area[patches.clone()]
                .iter()
                .fold(0.0, |acc, a| acc + a);
            if lake_id_elm > 0 {
                // `maxval(lakedepth(ps:pe))`
                lakedepth_hru[h] = lakedepth[patches]
                    .iter()
                    .copied()
                    .fold(f64::NEG_INFINITY, f64::max);
            }
        }
        if lakedown != 0 {
            let nb = &neighbours[ie];
            if let Some(inb) = nb.glbindex.iter().position(|&g| g == i64::from(lakedown)) {
                lakeoutlet_bsn[b] = nb.lenbdr[inb];
            }
        }
    }
    let unitarea_bsnhru = basin_hru.to_basin(&unitarea_hru);
    let lakedepth_bsnhru = basin_hru.to_basin(&lakedepth_hru);
    let mut riverlen = inputs.riverlen.clone();
    let mut riverarea = vec![0.0; n];
    let mut riverwth = vec![0.0; n];
    let mut bedelv = vec![0.0; n];
    let mut handmin = vec![0.0; n];
    // 未赋值的位置上游是未初始化内存，只在下游拉取里被读到；这里用 NaN，误用会显形。
    let mut wtsrfelv = vec![f64::NAN; n];
    let mut lakeinfo = vec![LakeInfo::default(); n];
    let minval = |v: &[f64]| v.iter().copied().fold(f64::INFINITY, f64::min);
    for b in 0..n {
        let range = basin_hru.basin_hru[b].clone();
        let net = &mut hillslope_basin[b];
        match inputs.lake_id[b].cmp(&0) {
            std::cmp::Ordering::Equal => {
                net.area = unitarea_bsnhru[range].to_vec();
                riverarea[b] = net.area[0];
                riverwth[b] = riverarea[b] / riverlen[b];
                if net.nhru > 1 {
                    for h in net.hand.iter_mut().skip(1) {
                        *h += riverdpth[b];
                    }
                }
                wtsrfelv[b] = inputs.riverelv[b];
                bedelv[b] = inputs.riverelv[b] - riverdpth[b];
                handmin[b] = minval(&net.hand);
            }
            std::cmp::Ordering::Greater => {
                wtsrfelv[b] = inputs.basinelv[b];
                let depth = &lakedepth_bsnhru[range.clone()];
                bedelv[b] =
                    inputs.basinelv[b] - depth.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                let nsub = range.len();
                let area0 = unitarea_bsnhru[range].to_vec();
                let depth0 = depth.to_vec();
                // `quicksort (nsublake, depth, order)` 后倒序：自深到浅。
                let mut sorted = depth0.clone();
                let mut order: Vec<usize> = (0..nsub).collect();
                quicksort_f64(&mut sorted, &mut order);
                let area_sorted: Vec<f64> = order.iter().map(|&o| area0[o]).collect();
                let depth_desc: Vec<f64> = sorted.into_iter().rev().collect();
                let area_desc: Vec<f64> = area_sorted.into_iter().rev().collect();
                let mut curve = vec![0.0; nsub];
                for i in 1..nsub {
                    let sum = area_desc[..i].iter().fold(0.0, |acc, a| acc + a);
                    // `.FMA (depth(i-1) - depth(i), sum, curve(i-1))`（`:767`）
                    curve[i] = (depth_desc[i - 1] - depth_desc[i]).mul_add(sum, curve[i - 1]);
                }
                lakeinfo[b] = LakeInfo {
                    nsub,
                    area0,
                    depth0,
                    area: area_desc,
                    depth: depth_desc,
                    dep_vol_curve: curve,
                };
                riverlen[b] = 0.0;
            }
            std::cmp::Ordering::Less => {
                net.area = unitarea_bsnhru[range].to_vec();
                handmin[b] = minval(&net.hand);
            }
        }
    }
    let pull = |data: &[f64]| {
        (0..n)
            .map(|b| match ilocdown[b] {
                Some(d) => data[d],
                None => f64::NAN,
            })
            .collect::<Vec<_>>()
    };
    let riverlen_ds = pull(&riverlen);
    let wtsrfelv_ds = pull(&wtsrfelv);
    let riverwth_ds = pull(&riverwth);
    let bedelv_ds = pull(&bedelv);
    for b in 0..n {
        if inputs.lake_id[b] < 0 {
            bedelv[b] = wtsrfelv_ds[b] + minval(&hillslope_basin[b].hand);
        }
    }
    let mut outletwth = vec![f64::NAN; n];
    for b in 0..n {
        if inputs.lake_id[b] == 0 {
            outletwth[b] = if to_lake[b] || riverdown[b] <= 0 {
                riverwth[b]
            } else {
                (riverwth[b] + riverwth_ds[b]) * 0.5
            };
        } else if !to_lake[b] && riverdown[b] != 0 {
            if riverdown[b] > 0 {
                outletwth[b] = riverwth_ds[b];
            } else if riverdown[b] == -1 {
                outletwth[b] = 0.0;
            }
        } else if to_lake[b] || riverdown[b] == 0 {
            outletwth[b] = lakeoutlet_bsn[b];
        }
    }
    Ok(RiverLakeNetwork {
        lake_id: inputs.lake_id.clone(),
        lake_type: inputs.lake_type.clone(),
        riverdown,
        ilocdown,
        to_lake,
        irivsys,
        numrivsys,
        riverlen,
        riverelv: inputs.riverelv.clone(),
        riverdpth,
        basinelv: inputs.basinelv.clone(),
        riverarea,
        riverwth,
        bedelv,
        handmin,
        wtsrfelv,
        riverlen_ds,
        wtsrfelv_ds,
        riverwth_ds,
        bedelv_ds,
        outletwth,
        lakeinfo,
        hillslope_basin,
        basin_hru,
    })
}

/// `MOD_Utils::quicksort_real8`（`MOD_Utils.F90:890-940`）的逐句移植：排序值与次序一起换。
/// 相等元素的次序取决于这个算法本身，不能换成稳定排序。
fn quicksort_f64(values: &mut [f64], order: &mut [usize]) {
    let length = values.len();
    if length <= 1 {
        return;
    }
    // Fortran 下标从 1 起：`A(nA/2)` 是 `length / 2 - 1`。
    let pivot = values[length / 2 - 1];
    let (mut left, mut right) = (0_usize, length + 1);
    while left < right {
        right -= 1;
        while values[right - 1] > pivot {
            right -= 1;
        }
        left += 1;
        while values[left - 1] < pivot {
            left += 1;
        }
        if left < right {
            values.swap(left - 1, right - 1);
            order.swap(left - 1, right - 1);
        }
    }
    let marker = right;
    quicksort_f64(&mut values[..marker], &mut order[..marker]);
    quicksort_f64(&mut values[marker..], &mut order[marker..]);
}

/// `subsurface_network_init (patcharea)` 的结果：单元级坡面网络与邻居数据。
#[derive(Debug, Clone)]
pub struct SubsurfaceNetwork {
    /// `hillslope_element`：按单元，`ihru` 指向单元 HRU 向量。
    pub hillslope_element: Vec<Hillslope>,
    pub lake_id_elm: Vec<i32>,
    pub riverdpth_elm: Vec<f64>,
    pub lakedepth_elm: Vec<f64>,
    pub agwt_nb: Vec<Vec<f64>>,
    pub islake_nb: Vec<Vec<bool>>,
    pub lakedp_nb: Vec<Vec<f64>>,
}

/// `subsurface_network_init`（`MOD_Catch_SubsurfaceFlow.F90:52-174`）。`patchtype`、`lakedepth`
/// 是读入处理后的 patch 量。
pub fn subsurface_network(
    topology: &CatchTopology,
    network: &RiverLakeNetwork,
    hillslope_inputs: &HillslopeInputs,
    neighbours: &[ElementNeighbour],
    patchtype: &[i32],
    lakedepth: &[f64],
) -> Result<SubsurfaceNetwork> {
    let mut hillslope_element = hillslope_networks(hillslope_inputs, &topology.elements)?;
    let lake_id_elm: Vec<i32> = topology
        .elements
        .iter()
        .map(|&id| network.lake_id[(id - 1) as usize])
        .collect();
    let riverdpth_elm: Vec<f64> = topology
        .elements
        .iter()
        .map(|&id| network.riverdpth[(id - 1) as usize])
        .collect();
    for (ie, net) in hillslope_element.iter_mut().enumerate() {
        let hrus = topology.elm_hru[ie].clone();
        ensure!(
            hrus.len() == net.nhru,
            "element {}: {} HRUs in landhru but basin_numhru is {}",
            topology.elements[ie],
            hrus.len(),
            net.nhru
        );
        if !net.is_lake {
            ensure!(
                net.ihru.first() == Some(&hrus.start),
                "element {}: hillslope HRU offsets disagree with landhru",
                topology.elements[ie]
            );
        }
        if lake_id_elm[ie] <= 0 {
            for i in 0..net.nhru {
                let h = net.ihru[i];
                let mut area = 0.0;
                let mut agwt = 0.0;
                for p in topology.hru_patch[h].clone() {
                    area += topology.patch_area[p];
                    if patchtype[p] <= 2 {
                        agwt += topology.patch_area[p];
                    }
                }
                net.area[i] = area;
                net.agwt[i] = agwt;
            }
        }
    }
    let lakedepth_elm: Vec<f64> = (0..topology.numelm())
        .map(|ie| {
            if lake_id_elm[ie] > 0 {
                let patches = topology.elm_patch[ie].clone();
                // `.FMA (lakedepth, subfrc, acc)`（`:128`）
                lakedepth[patches.clone()]
                    .iter()
                    .zip(&topology.elm_patch_frc[patches])
                    .fold(0.0, |acc, (d, f)| d.mul_add(*f, acc))
            } else {
                0.0
            }
        })
        .collect();
    let agwt_b: Vec<f64> = (0..topology.numelm())
        .map(|ie| {
            if lake_id_elm[ie] <= 0 {
                hillslope_element[ie]
                    .agwt
                    .iter()
                    .fold(0.0, |acc, a| acc + a)
            } else {
                0.0
            }
        })
        .collect();
    let mut agwt_nb = Vec::with_capacity(neighbours.len());
    let mut islake_nb = Vec::with_capacity(neighbours.len());
    let mut lakedp_nb = Vec::with_capacity(neighbours.len());
    for nb in neighbours {
        ensure!(
            nb.glbindex.iter().all(|&g| g > 0 || g == -9),
            "a neighbour index other than -9 (ocean) is not positive"
        );
        // 海洋邻居在上游是未初始化内存，且总被跳过；这里放 NaN/false。
        agwt_nb.push(
            nb.local
                .iter()
                .map(|l| l.map_or(f64::NAN, |je| agwt_b[je]))
                .collect(),
        );
        islake_nb.push(
            nb.local
                .iter()
                .map(|l| l.is_some_and(|je| lake_id_elm[je] > 0))
                .collect(),
        );
        lakedp_nb.push(
            nb.local
                .iter()
                .map(|l| l.map_or(f64::NAN, |je| lakedepth_elm[je]))
                .collect(),
        );
    }
    Ok(SubsurfaceNetwork {
        hillslope_element,
        lake_id_elm,
        riverdpth_elm,
        lakedepth_elm,
        agwt_nb,
        islake_nb,
        lakedp_nb,
    })
}

/// 流域状态（流域顺序）：`MOD_Catch_Vars_TimeVariables` 里按流域/流域 HRU 存的量。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CatchState {
    pub wdsrf_bsnhru: Vec<f64>,
    pub veloc_bsnhru: Vec<f64>,
    pub momen_bsnhru: Vec<f64>,
    pub wdsrf_bsnhru_prev: Vec<f64>,
    pub wdsrf_bsn: Vec<f64>,
    pub veloc_riv: Vec<f64>,
    pub momen_riv: Vec<f64>,
    pub wdsrf_bsn_prev: Vec<f64>,
}

/// `MOD_Initialize.F90:1641-1670`：河道 HRU 1 的水深取河深，湖泊各 HRU 取湖深；流域水位
/// 河道/湖泊汇流区取 `minval(hand + wdsrf) - handmin`，湖泊取 `surface(Σ FMA(wdsrf, area0))`。
pub fn cold_start(network: &RiverLakeNetwork) -> CatchState {
    let n = network.lake_id.len();
    let nb = network.basin_hru.numbsnhru();
    let mut state = CatchState {
        wdsrf_bsnhru: vec![0.0; nb],
        veloc_bsnhru: vec![0.0; nb],
        momen_bsnhru: vec![0.0; nb],
        wdsrf_bsnhru_prev: vec![0.0; nb],
        wdsrf_bsn: vec![0.0; n],
        veloc_riv: vec![0.0; n],
        momen_riv: vec![0.0; n],
        wdsrf_bsn_prev: vec![0.0; n],
    };
    for b in 0..n {
        let range = network.basin_hru.basin_hru[b].clone();
        for h in range.clone() {
            state.wdsrf_bsnhru[h] = 0.0;
        }
        if network.lake_id[b] == 0 {
            if !range.is_empty() {
                state.wdsrf_bsnhru[range.start] = network.riverdpth[b];
            }
        } else if network.lake_id[b] > 0 {
            for (h, d) in range.clone().zip(&network.lakeinfo[b].depth0) {
                state.wdsrf_bsnhru[h] = *d;
            }
        }
        for h in range {
            state.veloc_bsnhru[h] = 0.0;
            state.wdsrf_bsnhru_prev[h] = state.wdsrf_bsnhru[h];
        }
    }
    for b in 0..n {
        let range = network.basin_hru.basin_hru[b].clone();
        state.wdsrf_bsn[b] = basin_surface(network, b, &state.wdsrf_bsnhru[range]);
        state.veloc_riv[b] = 0.0;
        state.wdsrf_bsn_prev[b] = state.wdsrf_bsn[b];
    }
    state
}

/// 流域水位：河道/湖泊汇流区 `minval(hand + wdsrf) - handmin`，湖泊 `surface(Σ wdsrf·area0)`
/// （`river_lake_flow` 开头与 `MOD_Initialize` 同式）。
pub fn basin_surface(network: &RiverLakeNetwork, b: usize, wdsrf: &[f64]) -> f64 {
    if network.lake_id[b] <= 0 {
        let hand = &network.hillslope_basin[b].hand;
        hand.iter()
            .zip(wdsrf)
            .map(|(h, w)| h + w)
            .fold(f64::INFINITY, f64::min)
            - network.handmin[b]
    } else {
        let info = &network.lakeinfo[b];
        let volume = wdsrf
            .iter()
            .zip(&info.area0)
            .fold(0.0, |acc, (w, a)| w.mul_add(*a, acc));
        info.surface(volume)
    }
}

/// 冷启动的输入。
#[derive(Debug, Clone, Copy)]
pub struct CatchColdStartConfig<'a> {
    pub landdata: &'a Path,
    pub restart_dir: &'a Path,
    pub case_name: &'a str,
    pub land_cover_year: i32,
    /// `YYYY-DDD-SSSSS`
    pub date: &'a str,
    pub catchment_mesh: &'a Path,
    pub neighbour_file: &'a Path,
    pub runtime_dir: &'a Path,
    pub estimated_river_depth: bool,
    pub compression_level: u8,
}

/// 读按块写的 patch 向量（块顺序同 [`CatchTopology`]）。
pub fn read_block_vector(
    topology: &CatchTopology,
    path_of: impl Fn(&str) -> PathBuf,
    name: &str,
) -> Result<Vec<f64>> {
    let mut out = Vec::with_capacity(topology.numpatch());
    for (block, range) in &topology.blocks {
        let path = path_of(block);
        let file =
            netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
        let values = read_f64_any(&file, name)?;
        ensure!(
            values.len() == range.len(),
            "{} holds {} values of {name}, expected {}",
            path.display(),
            values.len(),
            range.len()
        );
        out.extend(values);
    }
    Ok(out)
}

/// 网络的全部静态部分（运行期 `lateral_flow_init` 与冷启动共用）。
pub struct CatchNetworks {
    pub topology: CatchTopology,
    pub neighbours: Vec<ElementNeighbour>,
    pub river: RiverLakeNetwork,
    pub hillslope_inputs: HillslopeInputs,
}

/// `element_neighbour_init` + `river_lake_network_init`（含 `calc_riverdepth_from_runoff`）。
pub fn build_networks(
    landdata: &Path,
    year: i32,
    catchment_mesh: &Path,
    neighbour_file: &Path,
    runtime_dir: &Path,
    estimated_river_depth: bool,
    lakedepth: impl FnOnce(&CatchTopology) -> Result<Vec<f64>>,
) -> Result<CatchNetworks> {
    let topology = CatchTopology::read(landdata, year)?;
    let inputs = BasinInputs::read(catchment_mesh, runtime_dir, estimated_river_depth)?;
    let riverdpth = match &inputs.riverdpth {
        Some(depth) => depth.clone(),
        None => estimate_river_depths(&topology, &inputs.riverdown, runtime_dir)?,
    };
    let elevation = read_block_vector(
        &topology,
        |block| block_path(landdata, "topography", "elevation_patches", year, block),
        "elevation_patches",
    )?;
    let neighbours = element_neighbours(&topology, neighbour_file, &elevation)?;
    let hillslope_inputs = HillslopeInputs::read(catchment_mesh)?;
    let lakedepth = lakedepth(&topology)?;
    let river = river_lake_network(
        &topology,
        &inputs,
        riverdpth,
        &hillslope_inputs,
        &neighbours,
        &lakedepth,
    )?;
    Ok(CatchNetworks {
        topology,
        neighbours,
        river,
        hillslope_inputs,
    })
}

/// `MOD_Initialize.F90:1619-1700` 的单进程形式：建网络、算流域初始水深，改写各块时间重启里的
/// `wdsrf`/`dz_lake`（时间重启在这段之后才写，`:1785`；常量重启在之前，`:785`），写流域重启。
pub fn write_catch_cold_restart(config: CatchColdStartConfig<'_>) -> Result<PathBuf> {
    let restart_const = config.restart_dir.join("const");
    let networks = build_networks(
        config.landdata,
        config.land_cover_year,
        config.catchment_mesh,
        config.neighbour_file,
        config.runtime_dir,
        config.estimated_river_depth,
        |topology| {
            read_block_vector(
                topology,
                |block| {
                    restart_const.join(format!(
                        "{}_restart_const_lc{:04}_{block}.nc",
                        config.case_name, config.land_cover_year
                    ))
                },
                "lakedepth",
            )
        },
    )?;
    let topology = &networks.topology;
    let state = cold_start(&networks.river);
    let wdsrf_hru = networks.river.basin_hru.to_element(
        &state.wdsrf_bsnhru,
        topology.numhru(),
        colm_core::MISSING,
    );
    let date_dir = config.restart_dir.join(config.date);
    let layers = colm_core::static_state::DEFAULT_LAKE_LAYERS;
    let dzlak = colm_core::static_state::DEFAULT_LAKE_THICKNESS_M;
    let dzlak_sum = colm_core::static_state::DEFAULT_LAKE_DEPTH_M;
    for (block, range) in &topology.blocks {
        let path = date_dir.join(format!(
            "{}_restart_{}_lc{:04}_{block}.nc",
            config.case_name, config.date, config.land_cover_year
        ));
        let mut file =
            netcdf::append(&path).with_context(|| format!("cannot open {}", path.display()))?;
        let mut dz_lake = file
            .variable("dz_lake")
            .context("time restart has no dz_lake")?
            .get_values::<f64, _>(..)?;
        ensure!(
            dz_lake.len() == range.len() * layers,
            "dz_lake has the wrong shape"
        );
        let mut wdsrf = file
            .variable("wdsrf")
            .context("time restart has no wdsrf")?
            .get_values::<f64, _>(..)?;
        let patch_hru = topology.patch_hru();
        for (local, p) in range.clone().enumerate() {
            // `wdsrf(ps:pe) = wdsrf_hru(i) * 1.0e3`（`:1678`）
            wdsrf[local] = wdsrf_hru[patch_hru[p]] * 1.0e3;
            let dz = &mut dz_lake[local * layers..(local + 1) * layers];
            if wdsrf[local] > 0.0 {
                // `wdsrfm = wdsrf * 1e-3`（`:1684`）
                let wdsrfm = wdsrf[local] * 1.0e-3;
                if wdsrfm > 1.0 && wdsrfm < 2000.0 {
                    let ratio = wdsrfm / dzlak_sum;
                    dz[0] = dzlak[0];
                    for l in 1..layers - 1 {
                        dz[l] = dzlak[l] * ratio;
                    }
                    // `.FMS (ratio, dzlak(nl), .FNMA (ratio, dzlak(1), dz_lake(1)))`（`:1689`）
                    let top_excess = (-ratio).mul_add(dzlak[0], dz[0]);
                    dz[layers - 1] = ratio.mul_add(dzlak[layers - 1], -top_excess);
                } else if wdsrfm > 0.0 && wdsrfm <= 1.0 {
                    for value in dz.iter_mut() {
                        *value = wdsrfm / layers as f64;
                    }
                }
            } else {
                dz.fill(0.0);
            }
        }
        file.variable_mut("wdsrf")
            .expect("checked above")
            .put_values(&wdsrf, ..)?;
        file.variable_mut("dz_lake")
            .expect("checked above")
            .put_values(&dz_lake, ..)?;
    }
    write_basin_restart(
        &date_dir.join(format!(
            "{}_restart_basin_{}_lc{:04}.nc",
            config.case_name, config.date, config.land_cover_year
        )),
        &networks.topology,
        &networks.river,
        &state,
        config.compression_level,
    )
}

/// `WRITE_CatchTimeVariables`：单元（按 `eindex` 升序）与 HRU（单元内按 `landhru` 次序）的全局向量。
pub fn write_basin_restart(
    path: &Path,
    topology: &CatchTopology,
    river: &RiverLakeNetwork,
    state: &CatchState,
    compression_level: u8,
) -> Result<PathBuf> {
    let mut order: Vec<usize> = (0..topology.numelm()).collect();
    order.sort_by_key(|&ie| topology.elements[ie]);
    let veloc_elm = river.basin_hru_value(&state.veloc_riv, topology);
    let wdsrf_elm_prev = river.basin_hru_value(&state.wdsrf_bsn_prev, topology);
    let veloc_hru =
        river
            .basin_hru
            .to_element(&state.veloc_bsnhru, topology.numhru(), colm_core::MISSING);
    let wdsrf_hru_prev = river.basin_hru.to_element(
        &state.wdsrf_bsnhru_prev,
        topology.numhru(),
        colm_core::MISSING,
    );
    let basin: Vec<i64> = order.iter().map(|&ie| topology.elements[ie]).collect();
    let mut hrus = Vec::new();
    for &ie in &order {
        hrus.extend(topology.elm_hru[ie].clone());
    }
    let mut file =
        netcdf::create(path).with_context(|| format!("cannot create {}", path.display()))?;
    file.add_dimension("basin", basin.len())?;
    file.add_dimension("hydrounit", hrus.len())?;
    let mut v = file.add_variable::<i64>("basin", &["basin"])?;
    v.put_values(&basin, ..)?;
    v.put_attribute("long_name", "basin index")?;
    let bsn_hru: Vec<i64> = hrus
        .iter()
        .map(|&h| {
            let ie = topology
                .elm_hru
                .iter()
                .position(|r| r.contains(&h))
                .expect("every HRU belongs to an element");
            topology.elements[ie]
        })
        .collect();
    let mut v = file.add_variable::<i64>("bsn_hru", &["hydrounit"])?;
    v.put_values(&bsn_hru, ..)?;
    v.put_attribute("long_name", "basin index of hydrological units")?;
    // `MOD_HRUVector.F90:153`：`htype_hru = abs(htype_hru)`（湖泊流域的 HRU 在 landhru 里是负的）。
    let hru_type: Vec<i32> = hrus.iter().map(|&h| topology.hru_type[h].abs()).collect();
    let mut v = file.add_variable::<i32>("hru_type", &["hydrounit"])?;
    v.put_values(&hru_type, ..)?;
    v.put_attribute("long_name", "index of hydrological units inside basin")?;
    let mut put = |name: &str, dim: &str, values: Vec<f64>| -> Result<()> {
        let mut v = file.add_variable::<f64>(name, &[dim])?;
        if compression_level > 0 {
            v.set_compression(i32::from(compression_level), true)?;
        }
        v.put_values(&values, ..)?;
        Ok(())
    };
    put(
        "veloc_riv",
        "basin",
        order.iter().map(|&ie| veloc_elm[ie]).collect(),
    )?;
    put(
        "wdsrf_bsn_prev",
        "basin",
        order.iter().map(|&ie| wdsrf_elm_prev[ie]).collect(),
    )?;
    put(
        "veloc_hru",
        "hydrounit",
        hrus.iter().map(|&h| veloc_hru[h]).collect(),
    )?;
    put(
        "wdsrf_hru_prev",
        "hydrounit",
        hrus.iter().map(|&h| wdsrf_hru_prev[h]).collect(),
    )?;
    file.close()?;
    Ok(path.to_path_buf())
}

impl RiverLakeNetwork {
    /// `worker_push_data (push_bsn2elm, basin_value, elm_value, spval)`：单元 ie 取流域 `eindex` 的值。
    pub fn basin_hru_value(&self, basin: &[f64], topology: &CatchTopology) -> Vec<f64> {
        topology
            .elements
            .iter()
            .map(|&id| basin[(id - 1) as usize])
            .collect()
    }
}

/// `READ_CatchTimeVariables`：读流域重启（单元按 `eindex` 升序、HRU 按单元内 `landhru` 次序），
/// 推到流域顺序（`push_elm2bsn`、`push_elmhru2bsnhru`）。水深与动量每步从 patch 重新聚合，这里只
/// 恢复 `veloc_riv`、`wdsrf_bsn_prev`、`veloc_bsnhru`、`wdsrf_bsnhru_prev`。
pub fn read_basin_restart(
    path: &Path,
    topology: &CatchTopology,
    river: &RiverLakeNetwork,
) -> Result<CatchState> {
    let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let basin = read_i64_any(&file, "basin")?;
    let bsn_hru = read_i64_any(&file, "bsn_hru")?;
    let veloc_riv = read_f64_any(&file, "veloc_riv")?;
    let wdsrf_bsn_prev = read_f64_any(&file, "wdsrf_bsn_prev")?;
    let veloc_hru = read_f64_any(&file, "veloc_hru")?;
    let wdsrf_hru_prev = read_f64_any(&file, "wdsrf_hru_prev")?;
    ensure!(
        basin.len() == veloc_riv.len() && basin.len() == wdsrf_bsn_prev.len(),
        "{} has inconsistent basin vectors",
        path.display()
    );
    ensure!(
        bsn_hru.len() == veloc_hru.len() && bsn_hru.len() == wdsrf_hru_prev.len(),
        "{} has inconsistent hydrounit vectors",
        path.display()
    );
    let position: std::collections::HashMap<i64, usize> =
        basin.iter().enumerate().map(|(i, &id)| (id, i)).collect();
    // 单元 HRU 在文件里的起点：单元按 `basin` 的次序，各自连续排着本单元的 HRU。
    let mut hru_start = std::collections::HashMap::new();
    let mut offset = 0;
    for &id in &basin {
        hru_start.insert(id, offset);
        offset += bsn_hru[offset..].iter().take_while(|&&b| b == id).count();
    }
    ensure!(
        offset == bsn_hru.len(),
        "{} has unordered hydrounits",
        path.display()
    );
    let numbasin = river.lake_id.len();
    let mut state = CatchState {
        wdsrf_bsnhru: vec![0.0; river.basin_hru.numbsnhru()],
        veloc_bsnhru: Vec::new(),
        momen_bsnhru: vec![0.0; river.basin_hru.numbsnhru()],
        wdsrf_bsnhru_prev: Vec::new(),
        wdsrf_bsn: vec![0.0; numbasin],
        veloc_riv: vec![colm_core::MISSING; numbasin],
        momen_riv: vec![0.0; numbasin],
        wdsrf_bsn_prev: vec![colm_core::MISSING; numbasin],
    };
    let mut veloc_elmhru = vec![0.0; topology.numhru()];
    let mut prev_elmhru = vec![0.0; topology.numhru()];
    for (ie, &id) in topology.elements.iter().enumerate() {
        let i = *position
            .get(&id)
            .with_context(|| format!("{} has no basin {id}", path.display()))?;
        let b = usize::try_from(id - 1).context("basin ids start at 1")?;
        ensure!(b < numbasin, "basin {id} is outside the river network");
        state.veloc_riv[b] = veloc_riv[i];
        state.wdsrf_bsn_prev[b] = wdsrf_bsn_prev[i];
        let start = hru_start[&id];
        let hrus = topology.elm_hru[ie].clone();
        ensure!(
            start + hrus.len() <= bsn_hru.len()
                && bsn_hru[start..start + hrus.len()].iter().all(|&b| b == id),
            "{}: basin {id} holds a different HRU count",
            path.display()
        );
        for (k, h) in hrus.enumerate() {
            veloc_elmhru[h] = veloc_hru[start + k];
            prev_elmhru[h] = wdsrf_hru_prev[start + k];
        }
    }
    state.veloc_bsnhru = river.basin_hru.to_basin(&veloc_elmhru);
    state.wdsrf_bsnhru_prev = river.basin_hru.to_basin(&prev_elmhru);
    Ok(state)
}

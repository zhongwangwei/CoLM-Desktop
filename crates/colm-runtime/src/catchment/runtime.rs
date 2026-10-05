//! 流域侧向流接进空间主循环：`lateral_flow_init`（网络、流域重启）、每步 `lateral_flow` 与
//! patch 状态的往返、basin history（`MOD_Catch_Hist`）以及续跑（`WRITE_CatchTimeVariables`，
//! 和非湖 patch 上被动态湖调整改过的 `dz_lake`/`t_lake`/`lake_icefrc`）。

use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};
use colm_core::{LakeColumn, StandardLctSnowSoilState};
use colm_init::catch_network::{
    build_networks, read_basin_restart, read_block_vector, subsurface_network, write_basin_restart,
};

use super::lateral::{CatchmentModel, PatchLateralFluxes, PatchLateralState, PatchSoilData};
use super::subsurface::{PatchWater, SubsurfaceParams};
use crate::assembly::StandardLctRestartTemplate;
use crate::history::HistoryOverrides;

const SPVAL: f64 = colm_core::MISSING;

/// 流域构建的输入文件与开关。
pub struct CatchmentSetup<'a> {
    pub landdata: &'a Path,
    pub land_cover_year: i32,
    pub catchment_mesh: &'a Path,
    pub neighbour_file: &'a Path,
    pub runtime_dir: &'a Path,
    pub estimated_river_depth: bool,
    /// 主循环的分块次序（`SpatialTopology`）；流域拓扑必须同序，patch 才一一对应。
    pub block_names: Vec<String>,
    /// 每块的常数重启与起跑时间重启（块顺序同拓扑）。
    pub constant_restarts: Vec<PathBuf>,
    pub time_restarts: Vec<PathBuf>,
    /// 起跑的流域重启（`<case>_restart_basin_<date>_lc<year>.nc`）。
    pub basin_restart: &'a Path,
    pub time_step_seconds: f64,
}

/// basin history 的累加器（`MOD_Catch_Hist`：`a_*` 与 `nac_basin`，起点都是 `spval`）。
#[derive(Debug, Clone)]
struct BasinHistory {
    steps: usize,
    wdsrf_bsn: Vec<f64>,
    veloc_riv: Vec<f64>,
    discharge: Vec<f64>,
    xsubs_elm: Vec<f64>,
    wdsrf_bsnhru: Vec<f64>,
    veloc_bsnhru: Vec<f64>,
    xsubs_hru: Vec<f64>,
}

impl BasinHistory {
    fn new(numbasin: usize, numelm: usize, numbsnhru: usize, numhru: usize) -> Self {
        Self {
            steps: 0,
            wdsrf_bsn: vec![SPVAL; numbasin],
            veloc_riv: vec![SPVAL; numbasin],
            discharge: vec![SPVAL; numbasin],
            xsubs_elm: vec![SPVAL; numelm],
            wdsrf_bsnhru: vec![SPVAL; numbsnhru],
            veloc_bsnhru: vec![SPVAL; numbsnhru],
            xsubs_hru: vec![SPVAL; numhru],
        }
    }

    fn flush(&mut self) {
        self.steps = 0;
        for values in [
            &mut self.wdsrf_bsn,
            &mut self.veloc_riv,
            &mut self.discharge,
            &mut self.xsubs_elm,
            &mut self.wdsrf_bsnhru,
            &mut self.veloc_bsnhru,
            &mut self.xsubs_hru,
        ] {
            values.fill(SPVAL);
        }
    }
}

/// `acc1d_basin`
fn acc1d(var: &[f64], sum: &mut [f64]) {
    for (s, &v) in sum.iter_mut().zip(var) {
        if v != SPVAL {
            *s = if *s != SPVAL { *s + v } else { v };
        }
    }
}

/// `WHERE (a /= spval) a = a / nac_basin`
fn mean(sum: &[f64], steps: usize) -> Vec<f64> {
    sum.iter()
        .map(|&s| if s != SPVAL { s / steps as f64 } else { s })
        .collect()
}

/// 空间主循环里的流域侧向流。
pub struct CatchmentRuntime {
    pub model: CatchmentModel,
    /// 每个 patch 的湖层（湖 patch 的在它自己的状态里，这里那一格不用）。
    lake_columns: Vec<LakeColumn>,
    /// 本步 `lateral_flow` 写给 patch 的量（history 覆盖用）。
    pub fluxes: Option<PatchLateralFluxes>,
    history: BasinHistory,
    /// 单元按 `eindex` 升序（basin history 与流域重启的 `basin` 维次序）。
    element_order: Vec<usize>,
    /// 本步侧向流之前的土壤水（`xerr` 在 `CoLMMAIN` 里算，见 [`HistoryOverrides`]）。
    pre_lateral: Vec<colm_core::Water2014SoilState>,
}

impl CatchmentRuntime {
    /// `lateral_flow_init` + `READ_CatchTimeVariables`。
    pub fn load(
        setup: &CatchmentSetup<'_>,
        templates: &[StandardLctRestartTemplate],
    ) -> Result<Self> {
        let block_index = |topology: &colm_init::catch_network::CatchTopology, block: &str| {
            topology
                .blocks
                .iter()
                .position(|(name, _)| name == block)
                .expect("the block comes from this topology")
        };
        let networks = build_networks(
            setup.landdata,
            setup.land_cover_year,
            setup.catchment_mesh,
            setup.neighbour_file,
            setup.runtime_dir,
            setup.estimated_river_depth,
            |topology| {
                read_block_vector(
                    topology,
                    |block| setup.constant_restarts[block_index(topology, block)].clone(),
                    "lakedepth",
                )
            },
        )?;
        let topology = networks.topology;
        ensure!(
            topology
                .blocks
                .iter()
                .map(|(name, _)| name)
                .eq(setup.block_names.iter()),
            "the catchment topology orders its blocks differently from the patch topology"
        );
        ensure!(
            topology.numpatch() == templates.len(),
            "the catchment topology has {} patches, the restarts {}",
            topology.numpatch(),
            templates.len()
        );
        ensure!(
            topology.blocks.len() == setup.constant_restarts.len(),
            "the catchment topology has {} blocks, the restarts {}",
            topology.blocks.len(),
            setup.constant_restarts.len()
        );
        let patch_types: Vec<i32> = templates.iter().map(|t| t.patch_type).collect();
        let lakedepth = read_block_vector(
            &topology,
            |block| setup.constant_restarts[block_index(&topology, block)].clone(),
            "lakedepth",
        )?;
        let soiltext = read_block_vector(
            &topology,
            |block| setup.constant_restarts[block_index(&topology, block)].clone(),
            "soiltext",
        )?;
        let sub = subsurface_network(
            &topology,
            &networks.river,
            &networks.hillslope_inputs,
            &networks.neighbours,
            &patch_types,
            &lakedepth,
        )?;
        let soil = templates
            .iter()
            .zip(&soiltext)
            .map(|(template, &text)| {
                let (porsl, hksati, psi0, theta_r, models) = template.lateral_soil();
                // `:668-678`：Campbell 时 `vl_r = 0`。
                let residual_water = models
                    .iter()
                    .zip(theta_r)
                    .map(|(model, &r)| match model {
                        colm_core::SoilHydraulicModel::Campbell { .. } => 0.0,
                        _ => r,
                    })
                    .collect();
                PatchSoilData {
                    patchtype: template.patch_type,
                    soiltext: text as i32,
                    porsl: porsl.to_vec(),
                    hksati: hksati.to_vec(),
                    psi0: psi0.to_vec(),
                    residual_water,
                    hydraulic_model: models.to_vec(),
                }
            })
            .collect();
        let physics = &templates
            .first()
            .context("a catchment case has at least one patch")?
            .physics;
        let grid = colm_init::colm_soil_grid(colm_core::tracer::state::SOIL_LAYERS)?;
        let params = SubsurfaceParams {
            deltime: setup.time_step_seconds,
            ice_impedance: physics.soil_ice_impedance,
            dynamic_lake: physics.dynamic_lake,
            wimp: physics.impermeable_porosity,
            wetwatmax: physics.wetland_water_capacity_mm,
            dz_soi: grid.thickness_m.clone(),
            zi_soi: grid.interface_depth_m[1..].to_vec(),
        };
        let state = read_basin_restart(setup.basin_restart, &topology, &networks.river)?;
        // 非湖 patch 的湖层：时间重启（动态湖）里的 `dz_lake`、`t_lake`、`lake_icefrc`。
        let mut lake_columns = Vec::with_capacity(templates.len());
        for path in &setup.time_restarts {
            let file = colm_init::RestartFile::open(path)?;
            let layers = file.dimension("lake")?;
            let patches = file.dimension("patch")?;
            for patch in 0..patches {
                lake_columns.push(LakeColumn {
                    thickness_m: file.layer_column("dz_lake", patch, layers)?,
                    temperature_k: file.layer_column("t_lake", patch, layers)?,
                    ice_fraction: file.layer_column("lake_icefrc", patch, layers)?,
                });
            }
        }
        ensure!(
            lake_columns.len() == templates.len(),
            "the time restarts hold {} patches for {} templates",
            lake_columns.len(),
            templates.len()
        );
        let mut element_order: Vec<usize> = (0..topology.numelm()).collect();
        element_order.sort_by_key(|&ie| topology.elements[ie]);
        let history = BasinHistory::new(
            networks.river.lake_id.len(),
            topology.numelm(),
            networks.river.basin_hru.numbsnhru(),
            topology.numhru(),
        );
        let model = CatchmentModel::new(
            topology,
            networks.neighbours,
            networks.river,
            sub,
            soil,
            params,
            state,
        )?;
        Ok(Self {
            model,
            lake_columns,
            fluxes: None,
            history,
            element_order,
            pre_lateral: Vec::new(),
        })
    }

    /// 一步 `lateral_flow`：从 patch 状态取水量、推进、写回。
    pub fn step(&mut self, states: &mut [StandardLctSnowSoilState]) -> Result<()> {
        self.pre_lateral = states.iter().map(|s| s.soil_water.clone()).collect();
        let mut patches = states
            .iter()
            .zip(&self.lake_columns)
            .map(|(state, lake)| {
                let water = &state.soil_water;
                PatchLateralState {
                    water: PatchWater {
                        wliq: water.liquid_water_kg_m2.clone(),
                        wice: water.ice_water_kg_m2.clone(),
                        wa: water.aquifer_water_mm,
                        zwt: water.water_table_depth_m,
                        wdsrf: water.surface_water_mm,
                        wetwat: water.wetland_water_mm,
                    },
                    top_soil_temperature_k: state.soil_temperature_k[0],
                    canopy_water_mm: state.energy.leaf.canopy_water.total_mm,
                    snow_water_equivalent_mm: state.snow.water_equivalent_kg_m2,
                    lake: state
                        .lake
                        .as_ref()
                        .map_or_else(|| lake.clone(), |l| l.column.clone()),
                }
            })
            .collect::<Vec<_>>();
        let fluxes = self.model.step(&mut patches)?;
        for ((state, patch), lake) in states
            .iter_mut()
            .zip(patches)
            .zip(self.lake_columns.iter_mut())
        {
            let water = &mut state.soil_water;
            water.liquid_water_kg_m2 = patch.water.wliq;
            water.ice_water_kg_m2 = patch.water.wice;
            water.aquifer_water_mm = patch.water.wa;
            water.water_table_depth_m = patch.water.zwt;
            water.surface_water_mm = patch.water.wdsrf;
            water.wetland_water_mm = patch.water.wetwat;
            match state.lake.as_mut() {
                Some(l) => l.column = patch.lake,
                None => *lake = patch.lake,
            }
        }
        self.fluxes = Some(fluxes);
        Ok(())
    }

    /// 第 `patch` 个 patch 本步的 history 覆盖量。
    pub fn history_overrides(&self, patch: usize) -> Option<HistoryOverrides> {
        let f = self.fluxes.as_ref()?;
        Some(HistoryOverrides {
            scalars: vec![
                ("rsur", f.rsur[patch]),
                ("rsub", f.rsub[patch]),
                ("rnof", f.rnof[patch]),
                ("xwsur", f.xwsur[patch]),
                ("xwsub", f.xwsub[patch]),
                ("fldarea", f.fldarea[patch]),
                ("wat", f.wat[patch]),
                ("wat_inst", f.wat[patch]),
            ],
            layers: vec![("h2osoi", f.h2osoi[patch].clone())],
            balance_water: self.pre_lateral.get(patch).cloned(),
        })
    }

    /// `accumulate_fluxes_basin`（`hist_out` 里，与 patch 累加同一步）。
    pub fn accumulate_history(&mut self) {
        let m = &self.model;
        let h = &mut self.history;
        h.steps += 1;
        acc1d(&m.acc.wdsrf_bsn_ta, &mut h.wdsrf_bsn);
        acc1d(&m.veloc_riv_ta, &mut h.veloc_riv);
        acc1d(&m.acc.discharge_ta, &mut h.discharge);
        acc1d(&m.xsubs_elm, &mut h.xsubs_elm);
        acc1d(&m.acc.wdsrf_bsnhru_ta, &mut h.wdsrf_bsnhru);
        acc1d(&m.veloc_bsnhru_ta, &mut h.veloc_bsnhru);
        acc1d(&m.xsubs_hru, &mut h.xsubs_hru);
    }

    /// `hist_basin_out`：写一条记录（`<case>_hist_basin_<suffix>.nc`），然后清零。
    pub fn write_history(
        &mut self,
        directory: &Path,
        stem: &str,
        record: &colm_hist::schedule::ScheduledRecord,
        compress_level: u8,
    ) -> Result<PathBuf> {
        let topology = &self.model.topology;
        let river = &self.model.river;
        let numhru = topology.numhru();
        let order = &self.element_order;
        let hrus: Vec<usize> = order
            .iter()
            .flat_map(|&ie| topology.elm_hru[ie].clone())
            .collect();
        let steps = self.history.steps;
        let basin_to_elm = |values: &[f64]| -> Vec<f64> {
            let elm = river.basin_hru_value(values, topology);
            order.iter().map(|&ie| elm[ie]).collect()
        };
        let bsnhru_to_hru = |values: &[f64]| -> Vec<f64> {
            let hru = river.basin_hru.to_element(values, numhru, SPVAL);
            hrus.iter().map(|&h| hru[h]).collect()
        };
        let h = &self.history;
        let fields: Vec<(&str, &str, &str, &str, Vec<f64>)> = vec![
            (
                "v_wdsrf_bsn",
                "basin",
                "River Height",
                "m",
                basin_to_elm(&mean(&h.wdsrf_bsn, steps)),
            ),
            (
                "v_veloc_riv",
                "basin",
                "River Velocity",
                "m/s",
                basin_to_elm(&mean(&h.veloc_riv, steps)),
            ),
            (
                "v_discharge",
                "basin",
                "River Discharge",
                "m^3/s",
                basin_to_elm(&mean(&h.discharge, steps)),
            ),
            (
                "timesteps",
                "basin",
                "Number of accumulated timesteps for each basin",
                "-",
                basin_to_elm(&self.model.acc.ntacc_bsn),
            ),
            (
                "v_wdsrf_hru",
                "hydrounit",
                "Depth of Surface Water in Hydro unit",
                "m",
                bsnhru_to_hru(&mean(&h.wdsrf_bsnhru, steps)),
            ),
            (
                "v_veloc_hru",
                "hydrounit",
                "Surface Flow Velocity in Hydro unit",
                "m/s",
                bsnhru_to_hru(&mean(&h.veloc_bsnhru, steps)),
            ),
            (
                "v_xsubs_bsn",
                "basin",
                "Subsurface lateral flow between basins",
                "m/s",
                order
                    .iter()
                    .map(|&ie| mean(&h.xsubs_elm, steps)[ie])
                    .collect(),
            ),
            (
                "v_xsubs_hru",
                "hydrounit",
                "SubSurface lateral flow between HRUs",
                "m/s",
                {
                    let m = mean(&h.xsubs_hru, steps);
                    hrus.iter().map(|&k| m[k]).collect()
                },
            ),
        ];
        std::fs::create_dir_all(directory)
            .with_context(|| format!("cannot create {}", directory.display()))?;
        let path = directory.join(format!("{stem}_hist_basin_{}.nc", record.suffix));
        let first = record.record == 0;
        let mut file = if first {
            let mut file = netcdf::create(&path)
                .with_context(|| format!("cannot create {}", path.display()))?;
            file.add_unlimited_dimension("time")?;
            file.add_dimension("basin", order.len())?;
            file.add_dimension("hydrounit", hrus.len())?;
            let ids: Vec<i64> = order.iter().map(|&ie| topology.elements[ie]).collect();
            let mut v = file.add_variable::<i64>("basin", &["basin"])?;
            v.put_values(&ids, ..)?;
            v.put_attribute("long_name", "basin index")?;
            let hru_basin: Vec<i64> = order
                .iter()
                .flat_map(|&ie| {
                    std::iter::repeat_n(topology.elements[ie], topology.elm_hru[ie].len())
                })
                .collect();
            let mut v = file.add_variable::<i64>("basin_hru", &["hydrounit"])?;
            v.put_values(&hru_basin, ..)?;
            v.put_attribute("long_name", "basin index of hydrological units")?;
            let hru_type: Vec<i32> = hrus.iter().map(|&k| topology.hru_type[k].abs()).collect();
            let mut v = file.add_variable::<i32>("hru_type", &["hydrounit"])?;
            v.put_values(&hru_type, ..)?;
            v.put_attribute("long_name", "index of hydrological units inside basin")?;
            let mut time = file.add_variable::<i32>("time", &["time"])?;
            time.put_attribute("long_name", "time")?;
            time.put_attribute("units", "minutes since 1900-1-1 0:0:0")?;
            for (name, dim, long_name, units, _) in &fields {
                let mut v = file.add_variable::<f64>(name, &["time", dim])?;
                if compress_level > 0 {
                    v.set_compression(i32::from(compress_level), true)?;
                }
                v.put_attribute("missing_value", SPVAL)?;
                v.put_attribute("long_name", *long_name)?;
                v.put_attribute("units", *units)?;
            }
            file
        } else {
            netcdf::append(&path).with_context(|| format!("cannot reopen {}", path.display()))?
        };
        let t = record.record;
        let label = i32::try_from(record.label_minutes).context("history label overflows i32")?;
        file.variable_mut("time")
            .context("time disappeared")?
            .put_values(&[label], t..t + 1)?;
        for (name, _, _, _, values) in &fields {
            file.variable_mut(name)
                .with_context(|| format!("{name} disappeared"))?
                .put_values(values, (t..t + 1, ..))?;
        }
        self.model.acc.ntacc_bsn.fill(0.0);
        self.history.flush();
        Ok(path)
    }

    /// `WRITE_CatchTimeVariables`，并把非湖 patch 的湖层写进刚写好的各块时间重启。
    pub fn write_restarts(
        &self,
        basin_path: &Path,
        block_restarts: &[(PathBuf, std::ops::Range<usize>)],
        states: &[StandardLctSnowSoilState],
        compression_level: u8,
    ) -> Result<()> {
        if let Some(parent) = basin_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        write_basin_restart(
            basin_path,
            &self.model.topology,
            &self.model.river,
            &self.model.state,
            compression_level,
        )?;
        for (path, range) in block_restarts {
            let mut file =
                netcdf::append(path).with_context(|| format!("cannot open {}", path.display()))?;
            for name in ["dz_lake", "t_lake", "lake_icefrc"] {
                let mut values = file
                    .variable(name)
                    .with_context(|| format!("{} has no {name}", path.display()))?
                    .get_values::<f64, _>(..)?;
                let layers = values.len() / range.len();
                for (local, p) in range.clone().enumerate() {
                    if states[p].lake.is_some() {
                        continue;
                    }
                    let column = &self.lake_columns[p];
                    let source = match name {
                        "dz_lake" => &column.thickness_m,
                        "t_lake" => &column.temperature_k,
                        _ => &column.ice_fraction,
                    };
                    values[local * layers..(local + 1) * layers].copy_from_slice(source);
                }
                file.variable_mut(name)
                    .expect("checked above")
                    .put_values(&values, ..)?;
            }
        }
        Ok(())
    }
}

impl CatchmentRuntime {
    /// `write_catch_parameters`（`MOD_Catch_WriteParameters.F90`）：`<out>/<case>/catch_parameters.nc`，
    /// HRU 上的土壤组分（按 `subfrc` 加权，GIMPLE 是从 0 起的 `FMA` 累加）、地类、坡长、高程与坡度。
    pub fn write_parameters(
        &self,
        path: &Path,
        constant_restarts: &[PathBuf],
        compression_level: u8,
    ) -> Result<()> {
        let topology = &self.model.topology;
        let block_index = |block: &str| {
            topology
                .blocks
                .iter()
                .position(|(name, _)| name == block)
                .expect("the block comes from this topology")
        };
        // 各块常数重启里整变量按块拼接（`(patch, soil)` 的量逐 patch 连续）。
        let read = |name: &str| -> Result<Vec<f64>> {
            let mut out = Vec::new();
            for (block, range) in &topology.blocks {
                let file = colm_init::RestartFile::open(&constant_restarts[block_index(block)])?;
                ensure!(
                    file.dimension("patch")? == range.len(),
                    "the constant restart of block {block} disagrees on the patch count"
                );
                match file.floats(name) {
                    Ok(values) => out.extend_from_slice(values),
                    Err(_) => out.extend(file.integers(name)?.iter().map(|&v| v as f64)),
                }
            }
            Ok(out)
        };
        let (sand, clay, om, gravels) = (
            read("wf_sand")?,
            read("wf_clay")?,
            read("wf_om")?,
            read("wf_gravels")?,
        );
        let patchclass = read("patchclass")?;
        let numpatch = topology.numpatch();
        let layers = sand.len() / numpatch;
        let hrus: Vec<usize> = self
            .element_order
            .iter()
            .flat_map(|&ie| topology.elm_hru[ie].clone())
            .collect();
        let weighted = |value: &dyn Fn(usize) -> f64| -> Vec<f64> {
            hrus.iter()
                .map(|&h| {
                    topology.hru_patch[h].clone().fold(0.0, |acc, p| {
                        value(p).mul_add(topology.hru_patch_frc[p], acc)
                    })
                })
                .collect()
        };
        let mut fields: Vec<(String, Vec<f64>, Option<&str>)> = Vec::new();
        for l in 0..5.min(layers) {
            let at = |v: &[f64], p: usize| v[p * layers + l];
            fields.push((
                format!("wf_sand_l{}", l + 1),
                weighted(&|p| at(&sand, p)),
                None,
            ));
            fields.push((
                format!("wf_clay_l{}", l + 1),
                weighted(&|p| at(&clay, p)),
                None,
            ));
            fields.push((format!("wf_om_l{}", l + 1), weighted(&|p| at(&om, p)), None));
            fields.push((
                format!("wf_silt_l{}", l + 1),
                weighted(&|p| {
                    (((1.0 - at(&sand, p)) - at(&gravels, p)) - at(&om, p)) - at(&clay, p)
                }),
                None,
            ));
        }
        fields.push((
            "lulc_igbp".to_owned(),
            hrus.iter()
                .map(|&h| patchclass[topology.hru_patch[h].start])
                .collect(),
            None,
        ));
        let mut slope_length = vec![SPVAL; topology.numhru()];
        let mut elevation = vec![SPVAL; topology.numhru()];
        let mut slope_ratio = vec![SPVAL; topology.numhru()];
        for (ie, hs) in self.model.sub.hillslope_element.iter().enumerate() {
            if self.model.sub.lake_id_elm[ie] > 0 {
                continue;
            }
            for i in 0..hs.nhru {
                if hs.indx[i] == 0 {
                    continue;
                }
                let h = hs.ihru[i];
                slope_length[h] = hs.plen[i] * 2.0;
                elevation[h] = hs.elva[i];
                slope_ratio[h] = match hs.inext[i] {
                    Some(j) => (hs.hand[i] - hs.hand[j]) / (hs.plen[i] + hs.plen[j]),
                    None => hs.hand[i] / hs.plen[i],
                };
            }
        }
        let order = |v: Vec<f64>| hrus.iter().map(|&h| v[h]).collect::<Vec<_>>();
        fields.push(("slope_length".to_owned(), order(slope_length), Some("m")));
        fields.push(("elevation".to_owned(), order(elevation), Some("m")));
        fields.push(("slope_ratio".to_owned(), order(slope_ratio), Some("-")));

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        let mut file =
            netcdf::create(path).with_context(|| format!("cannot create {}", path.display()))?;
        file.add_dimension("hydrounit", hrus.len())?;
        let hru_basin: Vec<i64> = self
            .element_order
            .iter()
            .flat_map(|&ie| std::iter::repeat_n(topology.elements[ie], topology.elm_hru[ie].len()))
            .collect();
        let mut v = file.add_variable::<i64>("bsn_hru", &["hydrounit"])?;
        v.put_values(&hru_basin, ..)?;
        v.put_attribute("long_name", "basin index of hydrological units")?;
        let hru_type: Vec<i32> = hrus.iter().map(|&k| topology.hru_type[k].abs()).collect();
        let mut v = file.add_variable::<i32>("hru_type", &["hydrounit"])?;
        v.put_values(&hru_type, ..)?;
        v.put_attribute("long_name", "index of hydrological units inside basin")?;
        for (name, values, units) in &fields {
            let mut v = file.add_variable::<f64>(name, &["hydrounit"])?;
            if compression_level > 0 {
                v.set_compression(i32::from(compression_level), true)?;
            }
            v.put_values(values, ..)?;
            v.put_attribute("missing_value", SPVAL)?;
            if let Some(units) = units {
                v.put_attribute("units", *units)?;
            }
        }
        file.close()?;
        Ok(())
    }
}

/// 流域重启的文件名（`<case>_restart_basin_<date>_lc<year>.nc`）。
pub fn basin_restart_path(directory: &Path, name: &str, label: &str, year: i64) -> PathBuf {
    directory
        .join(label)
        .join(format!("{name}_restart_basin_{label}_lc{year:04}.nc"))
}

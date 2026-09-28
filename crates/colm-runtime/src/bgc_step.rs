//! `DEF_USE_BGC` 的逐步推进：`CoLMDRIVER.F90:238` 在每个土壤 patch 的 `CoLMMAIN` 之后调
//! `bgc_driver`。本模块把 Rust 物理步的结果组装成 [`BgcPhysics`]、推进 [`BgcState`]，再把 BGC
//! 改写的物理量（`tsai_p`，LAI 反馈时还有 `tlai_p`/`lai_p`，以及 patch 的 `lai`/`tlai`）写回。
//!
//! 顺序与上游一致：`advance_patch` 里物理步与"为下一步准备"的表面光学都做完之后才调这里，
//! 所以 BGC 改写的 `tsai_p` 从下一步末尾的准备段才生效（`sai_p = tsai_p·sigf_p`）。
//!
//! 氮沉降（`MOD_NdepData`）：启动时按 `adj2end` 后的起始年读一次；此后只在结束于
//! 12 月 31 日 24:00 的那一步重读，读的是刚结束的那一年（上游的时序，实际滞后一年），
//! 年份钳在 1849–2006。单点 patch 的面积加权映射退化为取包含站点的那个源网格。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{bail, ensure, Context, Result};
use colm_core::bgc_driver::{bgc_driver, BgcPftConstants, BgcPhysics, BgcStep, BgcSwitches};
use colm_core::bgc_state::BgcState;
use colm_core::{StandardLctSnowSoilOutput, StandardLctSnowSoilState, MISSING};
use colm_init::RestartFile;

/// BGC 用到、但不随步变化的物理量（常数重启与土层网格）。
#[derive(Debug, Clone, PartialEq)]
pub struct BgcStatics {
    pub patchclass: i32,
    pub patchlatr: f64,
    pub patchlonr: f64,
    pub smpmax_hr: f64,
    pub smpmin_hr: f64,
    pub z_soi: Vec<f64>,
    pub dz_soi: Vec<f64>,
    pub zi_soi: Vec<f64>,
    /// `porsl`/`psi0`/`bsw`/`theta_r`/`*_vgm`/`BD_all`/`wfc`/`OM_density`，按 Fortran 名。
    pub soil: Vec<(&'static str, Vec<f64>)>,
}

impl BgcStatics {
    /// 从主常数重启读第 `patch` 个 patch（0 起）的静态量。
    pub fn read(constant: &Path, patch: usize, layers: usize) -> Result<Self> {
        let file = RestartFile::open(constant)?;
        // `smpmax_hr`/`smpmin_hr` 是模块级标量，写在不带 `_w180_s90` 的那份全局常数重启里。
        let name = constant
            .file_name()
            .and_then(|name| name.to_str())
            .context("the constant restart path has no file name")?;
        let global = RestartFile::open(constant.with_file_name(name.replacen("_w180_s90", "", 1)))?;
        let scalar = |name: &str| -> Result<f64> {
            let values = global.floats(name)?;
            ensure!(values.len() == 1, "{name} is not a scalar");
            Ok(values[0])
        };
        let at = |name: &str| -> Result<f64> {
            file.floats(name)?
                .get(patch)
                .copied()
                .with_context(|| format!("{name} has no entry for patch {patch}"))
        };
        let soil = |name: &'static str| -> Result<(&'static str, Vec<f64>)> {
            let values = file.floats(name)?;
            let column = values
                .get(patch * layers..(patch + 1) * layers)
                .with_context(|| format!("{name} has no soil column for patch {patch}"))?;
            Ok((name, column.to_vec()))
        };
        let grid = colm_core::colm_soil_grid(layers)?;
        let patchclass = *file
            .integers("patchclass")?
            .get(patch)
            .context("patchclass has no entry for this patch")?;
        Ok(Self {
            patchclass: i32::try_from(patchclass).context("patchclass overflows i32")?,
            patchlatr: at("patchlatr")?,
            patchlonr: at("patchlonr")?,
            smpmax_hr: scalar("smpmax_hr")?,
            smpmin_hr: scalar("smpmin_hr")?,
            z_soi: grid.node_depth_m,
            dz_soi: grid.thickness_m,
            zi_soi: grid.interface_depth_m[1..].to_vec(),
            soil: [
                "porsl",
                "psi0",
                "bsw",
                "theta_r",
                "alpha_vgm",
                "n_vgm",
                "L_vgm",
                "sc_vgm",
                "fc_vgm",
                "BD_all",
                "wfc",
                "OM_density",
            ]
            .into_iter()
            .filter(|name| file.float_names().iter().any(|present| present == name))
            .map(soil)
            .collect::<Result<_>>()?,
        })
    }
}

/// `MOD_NdepData` 的年度氮沉降（`DEF_NDEP_FREQUENCY = 1`）。
#[derive(Debug, Clone, PartialEq)]
pub struct NdepSource {
    path: PathBuf,
    lat: usize,
    lon: usize,
    /// `DEF_USE_PN`：沉降乘 5（加速 spin-up）。
    punctuated: bool,
}

impl NdepSource {
    /// `DEF_dir_runtime/ndep/fndep_colm_hist_simyr1849-2006_1.9x2.5_c100428.nc`。
    pub fn open(
        runtime_dir: &Path,
        latitude_deg: f64,
        longitude_deg: f64,
        punctuated: bool,
    ) -> Result<Self> {
        let path = runtime_dir.join("ndep/fndep_colm_hist_simyr1849-2006_1.9x2.5_c100428.nc");
        let file = netcdf::open(&path)
            .with_context(|| format!("cannot open the N deposition file {}", path.display()))?;
        let axis = |name: &str| -> Result<Vec<f64>> {
            file.variable(name)
                .with_context(|| format!("{} has no {name}", path.display()))?
                .get_values::<f64, _>(..)
                .with_context(|| format!("cannot read {name} from {}", path.display()))
        };
        let lat = containing_cell(&axis("lat")?, latitude_deg, false)?;
        let lon = containing_cell(&axis("lon")?, longitude_deg, true)?;
        Ok(Self {
            path,
            lat,
            lon,
            punctuated,
        })
    }

    /// `ndep` 与 `ndep_to_sminn`（gN/m²/s）。
    pub fn annual(&self, year: i32, patchclass: i32) -> Result<(f64, f64)> {
        let itime = usize::try_from(year.clamp(1849, 2006) - 1849).expect("clamped");
        let file = netcdf::open(&self.path)
            .with_context(|| format!("cannot open {}", self.path.display()))?;
        let ndep: f64 = file
            .variable("NDEP_year")
            .context("the N deposition file has no NDEP_year")?
            .get_value([itime, self.lat, self.lon])
            .context("cannot read NDEP_year")?;
        // `ndep / 3600. / 365. / 24.`：依次相除，不合并成一个常数。
        let to_sminn = if patchclass == 0 {
            0.0
        } else if self.punctuated {
            ndep / 3600.0 / 365.0 / 24.0 * 5.0
        } else {
            ndep / 3600.0 / 365.0 / 24.0
        };
        Ok((ndep, to_sminn))
    }
}

/// `MOD_NitrifData`：`DEF_USE_NITRIF` 的月度土壤 O₂ 浓度与分解深度（逐层文件）。
#[derive(Debug, Clone, PartialEq)]
pub struct NitrifSource {
    dir: PathBuf,
    lat: usize,
    lon: usize,
    layers: usize,
}

impl NitrifSource {
    /// 网格取自 `nitrif/CONC_O2_UNSAT/CONC_O2_UNSAT_l01.nc`（`init_nitrif_data`）。
    pub fn open(
        runtime_dir: &Path,
        latitude_deg: f64,
        longitude_deg: f64,
        layers: usize,
    ) -> Result<Self> {
        let dir = runtime_dir.join("nitrif");
        let path = dir.join("CONC_O2_UNSAT/CONC_O2_UNSAT_l01.nc");
        let file = netcdf::open(&path)
            .with_context(|| format!("cannot open the nitrification data {}", path.display()))?;
        let axis = |name: &str| -> Result<Vec<f64>> {
            // 坐标是 float，`ncio_read_bcast_serial` 读进 real(r8)：逐个精确扩成双精度。
            Ok(file
                .variable(name)
                .with_context(|| format!("{} has no {name}", path.display()))?
                .get_values::<f32, _>(..)
                .with_context(|| format!("cannot read {name} from {}", path.display()))?
                .into_iter()
                .map(f64::from)
                .collect())
        };
        let lat = containing_cell(&axis("lat")?, latitude_deg, false)?;
        let lon = containing_cell(&axis("lon")?, longitude_deg, true)?;
        Ok(Self {
            dir,
            lat,
            lon,
            layers,
        })
    }

    /// `update_nitrif_data(month)`：返回 `(tCONC_O2_UNSAT, tO2_DECOMP_DEPTH_UNSAT)` 两列。
    pub fn monthly(&self, month: u8, patchclass: i32) -> Result<(Vec<f64>, Vec<f64>)> {
        let read = |variable: &str| -> Result<Vec<f64>> {
            (1..=self.layers)
                .map(|layer| {
                    let path = self
                        .dir
                        .join(format!("{variable}/{variable}_l{layer:02}.nc"));
                    let file = netcdf::open(&path)
                        .with_context(|| format!("cannot open {}", path.display()))?;
                    let value: f32 = file
                        .variable(variable)
                        .with_context(|| format!("{} has no {variable}", path.display()))?
                        .get_value([usize::from(month) - 1, self.lat, self.lon])
                        .with_context(|| {
                            format!("cannot read {variable} from {}", path.display())
                        })?;
                    // 非土壤 patch 清零；`< 1E-10` 也清零（`MOD_NitrifData.F90`）。
                    let value = if patchclass == 0 {
                        0.0
                    } else {
                        f64::from(value)
                    };
                    Ok(if value < 1.0e-10 { 0.0 } else { value })
                })
                .collect()
        };
        Ok((read("CONC_O2_UNSAT")?, read("O2_DECOMP_DEPTH_UNSAT")?))
    }
}

/// `grid%define_by_center`：网格边界取相邻中心的中点，返回包含 `x` 的格子。
fn containing_cell(centers: &[f64], x: f64, periodic: bool) -> Result<usize> {
    ensure!(!centers.is_empty(), "an empty coordinate axis");
    let x = if periodic { x.rem_euclid(360.0) } else { x };
    let distance = |c: f64| {
        let d = (c - x).abs();
        if periodic {
            d.min(360.0 - d)
        } else {
            d
        }
    };
    let (index, _) = centers
        .iter()
        .enumerate()
        .map(|(i, c)| (i, distance(if periodic { c.rem_euclid(360.0) } else { *c })))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .expect("non-empty");
    Ok(index)
}

/// `COLM_BGC_TRACE`：按 Fortran 插桩相同的约定写追踪（见 `oracle/scripts/gen_bgc_trace.py`）。
struct TraceWriter {
    file: std::io::BufWriter<std::fs::File>,
    calls: usize,
    from: usize,
    every: usize,
    limit: usize,
    on: bool,
}

impl TraceWriter {
    fn from_env() -> Result<Option<Self>> {
        let Ok(path) = std::env::var("COLM_BGC_TRACE") else {
            return Ok(None);
        };
        let number = |name: &str, default: usize| -> Result<usize> {
            std::env::var(name).map_or(Ok(default), |value| {
                value
                    .trim()
                    .parse()
                    .with_context(|| format!("{name} must be a count"))
            })
        };
        let file = std::fs::File::create(&path).with_context(|| format!("cannot create {path}"))?;
        Ok(Some(Self {
            file: std::io::BufWriter::new(file),
            calls: 0,
            from: number("COLM_BGC_TRACE_FROM", 1)?,
            every: number("COLM_BGC_TRACE_EVERY", usize::MAX)?,
            limit: number("COLM_BGC_TRACE_CALLS", 1)?,
            on: false,
        }))
    }

    fn record(&mut self, tag: &str, state: &BgcState, physics: &BgcPhysics) -> std::io::Result<()> {
        if tag == "begin" {
            self.calls += 1;
            self.on = self.calls >= self.from && (self.calls - self.from) % self.every < self.limit;
        }
        if self.on {
            colm_core::bgc_trace::write_record(
                &mut self.file,
                tag,
                &physics.trace_inputs(),
                state,
            )?;
            if tag == "end" {
                self.file.flush()?;
            }
        }
        Ok(())
    }
}

/// BGC 的外部数据源：氮沉降（总在）与硝化 O₂（`DEF_USE_NITRIF`，附起始月份）。
pub struct BgcDataSources {
    pub ndep: NdepSource,
    /// 启动时读氮沉降用的年份：`adj2end` 之后的起始年（00:00 1 月 1 日起步时是上一年）。
    pub ndep_start_year: i32,
    pub nitrif: Option<(NitrifSource, u8)>,
}

/// 一个 BGC 土壤 patch 的运行期设置。
pub struct BgcRuntime {
    pub initial: BgcState,
    pub pft: BgcPftConstants,
    pub switches: BgcSwitches,
    pub statics: BgcStatics,
    pub ndep: NdepSource,
    /// `DEF_USE_NITRIF` 打开时的 O₂ 数据。
    pub nitrif: Option<NitrifSource>,
    /// 启动时读氮沉降用的年份：`adj2end` 之后的起始年（00:00 1 月 1 日起步时是上一年）。
    pub ndep_start_year: i32,
    /// `deltim`（秒）。
    pub deltim: f64,
    trace: Mutex<Option<TraceWriter>>,
}

impl std::fmt::Debug for BgcRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BgcRuntime")
            .field("switches", &self.switches)
            .field("statics", &self.statics)
            .field("ndep", &self.ndep)
            .finish_non_exhaustive()
    }
}

impl Clone for BgcRuntime {
    fn clone(&self) -> Self {
        Self {
            initial: self.initial.clone(),
            pft: self.pft.clone(),
            switches: self.switches,
            statics: self.statics.clone(),
            ndep: self.ndep.clone(),
            nitrif: self.nitrif.clone(),
            ndep_start_year: self.ndep_start_year,
            deltim: self.deltim,
            // 追踪文件只属于第一个实例。
            trace: Mutex::new(None),
        }
    }
}

impl BgcRuntime {
    pub fn new(
        mut initial: BgcState,
        pft: BgcPftConstants,
        switches: BgcSwitches,
        statics: BgcStatics,
        sources: BgcDataSources,
        deltim: f64,
    ) -> Result<Self> {
        let BgcDataSources {
            ndep,
            ndep_start_year,
            nitrif,
        } = sources;
        // `init_ndep_data_annually`：步进之前就写好 `ndep`/`ndep_to_sminn`。
        let (ndep_value, to_sminn) = ndep.annual(ndep_start_year, statics.patchclass)?;
        initial.patch.ndep[0] = ndep_value;
        initial.patch_flux.ndep_to_sminn[0] = to_sminn;
        // `init_nitrif_data(ststamp)`：起始时刻所在的月。
        if let Some((source, month)) = &nitrif {
            let (conc, depth) = source.monthly(*month, statics.patchclass)?;
            initial.patch.tconc_o2_unsat.copy_from_slice(&conc);
            initial.patch.to2_decomp_depth_unsat.copy_from_slice(&depth);
        }
        let nitrif = nitrif.map(|(source, _)| source);
        Ok(Self {
            initial,
            pft,
            switches,
            statics,
            ndep,
            nitrif,
            ndep_start_year,
            deltim,
            trace: Mutex::new(TraceWriter::from_env()?),
        })
    }

    /// 一步：（必要时）更新氮沉降 → `bgc_driver` → 写回物理量。
    pub fn step(
        &self,
        begin: colm_core::calendar::CalendarTime,
        idate: [i32; 3],
        forcing: &colm_core::RuntimeForcing,
        state: &mut StandardLctSnowSoilState,
        output: &StandardLctSnowSoilOutput,
    ) -> Result<()> {
        let deltim = self.deltim;
        let mut bgc = state
            .bgc
            .take()
            .context("a BGC patch needs its BGC state")?;
        // `update_nitrif_data`：步首所在月与上一步步首所在月不同时（`CoLM.F90:495-501`）。
        if let Some(source) = &self.nitrif {
            let month = |time: colm_core::calendar::CalendarTime| {
                colm_core::calendar::month_day(time).map(|(month, _)| month)
            };
            let previous = previous_step_start(begin, deltim);
            if month(begin)? != month(previous)? {
                let (conc, depth) = source.monthly(month(begin)?, self.statics.patchclass)?;
                bgc.patch.tconc_o2_unsat.copy_from_slice(&conc);
                bgc.patch.to2_decomp_depth_unsat.copy_from_slice(&depth);
            }
        }
        if colm_core::bgc_driver::is_end_of_year(idate, deltim) {
            let (ndep, to_sminn) = self.ndep.annual(idate[0], self.statics.patchclass)?;
            bgc.patch.ndep[0] = ndep;
            bgc.patch_flux.ndep_to_sminn[0] = to_sminn;
        }
        let mut physics = self.physics(idate, deltim, forcing, state, output)?;
        {
            let mut trace = self.trace.lock().expect("trace lock");
            let mut record = |tag: &str, s: &BgcState, p: &BgcPhysics| {
                if let Some(writer) = trace.as_mut() {
                    // 追踪只是诊断；写失败时停写，不影响模式。
                    if writer.record(tag, s, p).is_err() {
                        *trace = None;
                    }
                }
            };
            let mut step = BgcStep {
                state: &mut bgc,
                physics: &mut physics,
                pft: &self.pft,
                switches: self.switches,
            };
            bgc_driver(&mut step, &mut record)?;
        }
        self.write_back(&physics, state)?;
        // 汇总写的分 PFT 类型 LAI 只供历史，跟着 BGC 状态走。
        let inputs = physics.trace_inputs();
        for (slot, name) in bgc
            .lai_diagnostics
            .iter_mut()
            .zip(colm_core::bgc_state::LAI_DIAGNOSTICS)
        {
            if let Some((_, values)) = inputs.iter().find(|(field, _)| *field == name) {
                *slot = values[0];
            }
        }
        state.bgc = Some(bgc);
        Ok(())
    }

    fn physics(
        &self,
        idate: [i32; 3],
        deltim: f64,
        forcing: &colm_core::RuntimeForcing,
        state: &StandardLctSnowSoilState,
        output: &StandardLctSnowSoilOutput,
    ) -> Result<BgcPhysics> {
        let pft = state
            .energy
            .pft
            .as_ref()
            .context("DEF_USE_BGC needs the PFT subgrid")?;
        let nl = self.initial.dims.nl_soil;
        let columns = &pft.columns;
        let per_pft =
            |f: fn(&colm_core::PftColumn) -> f64| columns.iter().map(f).collect::<Vec<_>>();
        let soil = |name: &str| {
            self.statics
                .soil
                .iter()
                .find(|(field, _)| *field == name)
                .map_or_else(|| vec![MISSING; nl], |(_, values)| values.clone())
        };
        let water = &state.soil_water;
        let h2osoi = (0..nl)
            .map(|j| {
                let dz = self.statics.dz_soi[j];
                water.liquid_water_kg_m2[j] / (dz * 1000.0)
                    + water.ice_water_kg_m2[j] / (dz * 917.0)
            })
            .collect();
        let degrees = |radians: f64| radians * 180.0 / std::f64::consts::PI;
        let npft = columns.len();
        let optional_patch = || vec![MISSING];
        Ok(BgcPhysics {
            idate,
            pftclass: pft.parameters.iter().map(|p| p.class).collect(),
            patchclass: self.statics.patchclass,
            z_soi: self.statics.z_soi.clone(),
            dz_soi: self.statics.dz_soi.clone(),
            zi_soi: self.statics.zi_soi.clone(),
            pftfrac: pft.parameters.iter().map(|p| p.fraction).collect(),
            rootfr_p: pft
                .parameters
                .iter()
                .flat_map(|p| p.root_fraction.iter().copied())
                .collect(),
            tsai_p: per_pft(|c| c.temporal_stem_area_index),
            tlai_p: per_pft(|c| c.temporal_leaf_area_index),
            lai_p: per_pft(|c| c.leaf_area_index),
            laisun_p: per_pft(|c| c.sunlit_leaf_area_index),
            laisha_p: per_pft(|c| c.shaded_leaf_area_index),
            sigf_p: per_pft(|c| c.vegetation_free_fraction),
            tref_p: per_pft(|c| c.reference_temperature_k),
            assim_p: per_pft(|c| c.assimilation_mol_m2_s),
            respc_p: per_pft(|c| c.respiration_mol_m2_s),
            patchlatr: vec![self.statics.patchlatr],
            porsl: soil("porsl"),
            psi0: soil("psi0"),
            bsw: soil("bsw"),
            theta_r: soil("theta_r"),
            alpha_vgm: soil("alpha_vgm"),
            n_vgm: soil("n_vgm"),
            L_vgm: soil("L_vgm"),
            sc_vgm: soil("sc_vgm"),
            fc_vgm: soil("fc_vgm"),
            BD_all: soil("BD_all"),
            wfc: soil("wfc"),
            OM_density: soil("OM_density"),
            lai: vec![state.energy.canopy.leaf_area_index],
            tlai: vec![state.energy.temporal_canopy.leaf_area_index],
            tref: vec![output.energy.leaf.air_temperature_2m_k],
            t_soisno: state.soil_temperature_k[..nl].to_vec(),
            wliq_soisno: water.liquid_water_kg_m2[..nl].to_vec(),
            wice_soisno: water.ice_water_kg_m2[..nl].to_vec(),
            smp: water.matric_potential_mm[..nl].to_vec(),
            h2osoi,
            rsur: vec![output.water.soil.surface_runoff_mm_s],
            rnof: vec![output.water.soil.total_runoff_mm_s],
            forc_t: vec![forcing.air_temperature_k],
            forc_q: vec![forcing.specific_humidity],
            forc_psrf: vec![forcing.surface_pressure_pa],
            forc_prc: vec![forcing.convective_precipitation_kg_m2_s],
            forc_prl: vec![forcing.large_scale_precipitation_kg_m2_s],
            forc_us: vec![forcing.eastward_wind_m_s],
            forc_vs: vec![forcing.northward_wind_m_s],
            deltim,
            dlat: degrees(self.statics.patchlatr),
            dlon: degrees(self.statics.patchlonr),
            smpmax_hr: self.statics.smpmax_hr,
            smpmin_hr: self.statics.smpmin_hr,
            lai_enftemp: optional_patch(),
            lai_enfboreal: optional_patch(),
            lai_dnfboreal: optional_patch(),
            lai_ebftrop: optional_patch(),
            lai_ebftemp: optional_patch(),
            lai_dbftrop: optional_patch(),
            lai_dbftemp: optional_patch(),
            lai_dbfboreal: optional_patch(),
            lai_ebstemp: optional_patch(),
            lai_dbstemp: optional_patch(),
            lai_dbsboreal: optional_patch(),
            lai_c3arcgrass: optional_patch(),
            lai_c3grass: optional_patch(),
            lai_c4grass: optional_patch(),
            irrig_method_corn: optional_patch(),
            irrig_method_swheat: optional_patch(),
            irrig_method_wwheat: optional_patch(),
            irrig_method_soybean: optional_patch(),
            irrig_method_cotton: optional_patch(),
            irrig_method_rice1: optional_patch(),
            irrig_method_rice2: optional_patch(),
            irrig_method_sugarcane: optional_patch(),
            irrig_method_p: vec![MISSING; npft],
        })
    }

    /// `CNVegStructUpdate` 改写的物理量回到 Rust 状态。
    fn write_back(&self, physics: &BgcPhysics, state: &mut StandardLctSnowSoilState) -> Result<()> {
        let pft = state
            .energy
            .pft
            .as_mut()
            .context("DEF_USE_BGC needs the PFT subgrid")?;
        for (m, column) in pft.columns.iter_mut().enumerate() {
            column.temporal_stem_area_index = physics.tsai_p[m];
            column.temporal_leaf_area_index = physics.tlai_p[m];
            column.leaf_area_index = physics.lai_p[m];
        }
        state.energy.canopy.leaf_area_index = physics.lai[0];
        state.energy.temporal_canopy.leaf_area_index = physics.tlai[0];
        Ok(())
    }
}

/// `itstamp + int(-deltim)`：上一步的步首。
fn previous_step_start(
    begin: colm_core::calendar::CalendarTime,
    deltim: f64,
) -> colm_core::calendar::CalendarTime {
    let step = deltim as i64;
    let mut seconds = i64::from(begin.seconds) - step;
    let (mut year, mut day) = (begin.year, i64::from(begin.julian_day));
    while seconds < 0 {
        seconds += 86400;
        day -= 1;
        if day < 1 {
            year -= 1;
            day = if colm_core::calendar::is_leap_year(year) {
                366
            } else {
                365
            };
        }
    }
    colm_core::calendar::CalendarTime {
        year,
        julian_day: u16::try_from(day).expect("julian day"),
        seconds: u32::try_from(seconds).expect("seconds of day"),
    }
}

/// 未移植的 BGC 分支：遇到就拒绝，而不是静默跑成另一个模式。
pub fn refuse_unported(switches: BgcSwitches) -> Result<()> {
    let unported = [
        (switches.fire, "DEF_USE_FIRE"),
        (switches.diag_matrix, "DEF_USE_DiagMatrix"),
        (switches.crop, "CROP"),
    ];
    for (on, name) in unported {
        if on {
            bail!("{name} is on, but the Rust BGC driver has not been verified on that branch yet");
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "bgc_step_tests.rs"]
mod bgc_step_tests;

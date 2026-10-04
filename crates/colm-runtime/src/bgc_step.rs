//! `DEF_USE_BGC` 的逐步推进：`CoLMDRIVER.F90:238` 在每个土壤 patch 的 `CoLMMAIN` 之后调
//! `bgc_driver`。本模块把 Rust 物理步的结果组装成 [`BgcPhysics`]、推进 [`BgcState`]，再把 BGC
//! 改写的物理量（`tsai_p`，LAI 反馈时还有 `tlai_p`/`lai_p`，以及 patch 的 `lai`/`tlai`）写回。
//!
//! 顺序与上游一致：`advance_patch` 里物理步与"为下一步准备"的表面光学都做完之后才调这里，
//! 所以 BGC 改写的 `tsai_p` 从下一步末尾的准备段才生效（`sai_p = tsai_p·sigf_p`）。
//!
//! 氮沉降（`MOD_NdepData`）：启动时按 namelist 的起始年读一次；此后只在结束于
//! 12 月 31 日 24:00 的那一步重读，读的是刚结束的那一年（上游的时序，实际滞后一年），
//! 年份钳在 1849–2006。单点 patch 的面积加权映射退化为取包含站点的那个源网格。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{bail, ensure, Context, Result};
use colm_core::bgc_driver::{
    bgc_driver, BgcIrrigation, BgcPftConstants, BgcPhysics, BgcStep, BgcSwitches,
};
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
    /// `CNFireArea` 算饱和面积比用的 `fsatmax`/`fsatdcf`/`topoweti`/`alp_twi`/`chi_twi`/`mu_twi`：
    /// 常数重启里没有的（非 TOPMODEL 产流）按 `spval`，与上游的回退判断一致。
    pub topmodel: [f64; 6],
}

/// [`BgcStatics::topmodel`] 的 Fortran 名。
pub const TOPMODEL_STATICS: [&str; 6] = [
    "fsatmax", "fsatdcf", "topoweti", "alp_twi", "chi_twi", "mu_twi",
];

impl BgcStatics {
    /// 从主常数重启读第 `patch` 个 patch（0 起）的静态量。
    pub fn read(constant: &Path, patch: usize, layers: usize) -> Result<Self> {
        let file = RestartFile::open(constant)?;
        // `smpmax_hr`/`smpmin_hr` 是模块级标量，写在不带块后缀的那份全局常数重启里。
        let name = constant
            .file_name()
            .and_then(|name| name.to_str())
            .context("the constant restart path has no file name")?;
        let global =
            RestartFile::open(constant.with_file_name(crate::bgc::without_block_suffix(name)))?;
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
            topmodel: {
                let mut values = [MISSING; 6];
                for (slot, name) in values.iter_mut().zip(TOPMODEL_STATICS) {
                    if file.float_names().iter().any(|present| present == name) {
                        *slot = at(name)?;
                    }
                }
                values
            },
        })
    }
}

/// BGC 驱动数据（氮沉降、硝化、火灾）在数据网格上取值的位置。
///
/// 上游都是 `build_arealweighted(grid, landpatch)` + `grid2pset`：单点里 patch 只落在一格，就取那一格；
/// 空间算例里 patch 的像元可能跨好几格，取 `Σ FMA(areapart, v, sum) / Σ areapart`
/// （与强迫映射 [`crate::spatial::mapping::AreaWeightedMapping::grid_to_set`] 同一实现）。
#[derive(Debug, Clone, PartialEq)]
pub enum Footprint {
    /// 包含站点的那一格 `(ilat, ilon)`。
    Cell(usize, usize),
    /// patch 覆盖的格子 `(ilat, ilon, 重叠面积)` 与面积和。
    Parts {
        parts: Vec<(usize, usize, f64)>,
        area: f64,
    },
}

impl Footprint {
    /// `set_missing_value`：网格值等于 `missing` 的格子的份面积清零，面积和按剩下的份从 0 起重算。
    /// 单点的那一格是缺测时变成空的映射（之后取值都是 `spval`）。
    pub fn mask_missing(
        &mut self,
        read: &impl Fn(usize, usize) -> Result<f64>,
        missing: f64,
    ) -> Result<()> {
        match self {
            Footprint::Cell(lat, lon) => {
                if read(*lat, *lon)? == missing {
                    *self = Footprint::Parts {
                        parts: Vec::new(),
                        area: 0.0,
                    };
                }
            }
            Footprint::Parts { parts, area } => {
                *area = 0.0;
                for part in parts.iter_mut() {
                    if read(part.0, part.1)? == missing {
                        part.2 = 0.0;
                    } else {
                        *area += part.2;
                    }
                }
            }
        }
        Ok(())
    }

    /// 按 `read(ilat, ilon)` 取网格值，再映射到 patch。
    pub fn sample(&self, read: impl Fn(usize, usize) -> Result<f64>) -> Result<f64> {
        match self {
            Footprint::Cell(lat, lon) => read(*lat, *lon),
            Footprint::Parts { parts, area } => {
                if *area <= 0.0 {
                    return Ok(colm_core::MISSING);
                }
                let mut sum = 0.0;
                for &(lat, lon, part) in parts {
                    if part > 0.0 {
                        sum = part.mul_add(read(lat, lon)?, sum);
                    }
                }
                Ok(sum / area)
            }
        }
    }

    /// `grid2pset_dominant`：面积最大的那一份（`maxloc`，并列取第一个）所在格的值；
    /// 面积和不为正时是 `None`（上游给 −9999）。单点就是所在那一格。
    pub fn dominant(&self, read: impl Fn(usize, usize) -> Result<f64>) -> Result<Option<f64>> {
        match self {
            Footprint::Cell(lat, lon) => read(*lat, *lon).map(Some),
            Footprint::Parts { parts, area } => {
                if *area <= 0.0 {
                    return Ok(None);
                }
                let mut best: Option<&(usize, usize, f64)> = None;
                for part in parts {
                    if best.is_none_or(|top| part.2 > top.2) {
                        best = Some(part);
                    }
                }
                match best {
                    Some(&(lat, lon, _)) => read(lat, lon).map(Some),
                    None => Ok(None),
                }
            }
        }
    }
}

/// 一个 patch 在数据网格上的定位方式：单点按站点经纬度，空间按它的像元。
#[derive(Debug, Clone, Copy)]
pub enum Locator<'a> {
    Site {
        latitude_deg: f64,
        longitude_deg: f64,
    },
    Patch {
        pixel: &'a crate::spatial::mapping::PixelAxes,
        cells: &'a [(i32, i32)],
        shared_fraction: f64,
    },
}

impl Locator<'_> {
    /// 数据网格（中心 `lat`/`lon`，`define_by_center`）上的取值位置。
    pub fn footprint(&self, lat: &[f64], lon: &[f64]) -> Result<Footprint> {
        match *self {
            Locator::Site {
                latitude_deg,
                longitude_deg,
            } => Ok(Footprint::Cell(
                containing_cell(lat, latitude_deg, false)?,
                containing_cell(lon, longitude_deg, true)?,
            )),
            Locator::Patch {
                pixel,
                cells,
                shared_fraction,
            } => {
                let grid = crate::spatial::grid::LatLonGrid::define_by_center(lat, lon, None)?;
                let mapping = crate::spatial::mapping::AreaWeightedMapping::build(
                    &grid,
                    pixel,
                    &[cells.to_vec()],
                    &[shared_fraction],
                )?;
                Ok(Footprint::Parts {
                    parts: mapping.parts[0]
                        .iter()
                        .map(|part| (part.ilat, part.ilon, part.area))
                        .collect(),
                    area: mapping.area[0],
                })
            }
        }
    }
}

/// `MOD_NdepData` 的氮沉降：年度（`DEF_NDEP_FREQUENCY = 1`）或月度（`= 2`）文件。
#[derive(Debug, Clone, PartialEq)]
pub struct NdepSource {
    path: PathBuf,
    footprint: Footprint,
    /// `DEF_USE_PN`：沉降乘 5（加速 spin-up）。
    punctuated: bool,
    /// 月度文件 `fndep_colm_monthly.nc` 的 `NDEP_month`。
    monthly: bool,
}

impl NdepSource {
    /// 年度 `DEF_dir_runtime/ndep/fndep_colm_hist_simyr1849-2006_1.9x2.5_c100428.nc`，
    /// 月度 `DEF_dir_runtime/ndep/fndep_colm_monthly.nc`（`init_ndep_data_*`）。
    pub fn open(
        runtime_dir: &Path,
        locator: Locator<'_>,
        punctuated: bool,
        monthly: bool,
    ) -> Result<Self> {
        let path = runtime_dir.join(if monthly {
            "ndep/fndep_colm_monthly.nc"
        } else {
            "ndep/fndep_colm_hist_simyr1849-2006_1.9x2.5_c100428.nc"
        });
        let file = netcdf::open(&path)
            .with_context(|| format!("cannot open the N deposition file {}", path.display()))?;
        let axis = |name: &str| -> Result<Vec<f64>> {
            file.variable(name)
                .with_context(|| format!("{} has no {name}", path.display()))?
                .get_values::<f64, _>(..)
                .with_context(|| format!("cannot read {name} from {}", path.display()))
        };
        let footprint = locator.footprint(&axis("lat")?, &axis("lon")?)?;
        Ok(Self {
            path,
            footprint,
            punctuated,
            monthly,
        })
    }

    pub fn is_monthly(&self) -> bool {
        self.monthly
    }

    /// `update_ndep_data_annually(year)`：`ndep` 与 `ndep_to_sminn`（gN/m²/s）。
    pub fn annual(&self, year: i32, patchclass: i32) -> Result<(f64, f64)> {
        ensure!(
            !self.monthly,
            "the monthly N deposition file has no NDEP_year"
        );
        let itime = usize::try_from(year.clamp(1849, 2006) - 1849).expect("clamped");
        self.read("NDEP_year", itime, patchclass)
    }

    /// `update_ndep_data_monthly(year, month)`：`itime = (clamp(year) - 1849)*12 + month`。
    pub fn month(&self, year: i32, month: u8, patchclass: i32) -> Result<(f64, f64)> {
        ensure!(
            self.monthly,
            "the annual N deposition file has no NDEP_month"
        );
        ensure!(
            (1..=12).contains(&month),
            "N deposition month {month} is not 1..=12"
        );
        let itime = usize::try_from((year.clamp(1849, 2006) - 1849) * 12).expect("clamped")
            + usize::from(month - 1);
        self.read("NDEP_month", itime, patchclass)
    }

    fn read(&self, variable: &str, itime: usize, patchclass: i32) -> Result<(f64, f64)> {
        let file = netcdf::open(&self.path)
            .with_context(|| format!("cannot open {}", self.path.display()))?;
        let values = file
            .variable(variable)
            .with_context(|| format!("the N deposition file has no {variable}"))?;
        let ndep = self.footprint.sample(|lat, lon| {
            values
                .get_value::<f64, _>([itime, lat, lon])
                .with_context(|| format!("cannot read {variable}"))
        })?;
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
    footprint: Footprint,
    layers: usize,
}

impl NitrifSource {
    /// 网格取自 `nitrif/CONC_O2_UNSAT/CONC_O2_UNSAT_l01.nc`（`init_nitrif_data`）。
    pub fn open(runtime_dir: &Path, locator: Locator<'_>, layers: usize) -> Result<Self> {
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
        let footprint = locator.footprint(&axis("lat")?, &axis("lon")?)?;
        Ok(Self {
            dir,
            footprint,
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
                    let values = file
                        .variable(variable)
                        .with_context(|| format!("{} has no {variable}", path.display()))?;
                    // 文件是 float，读进 real(r8) 的块再映射。
                    let value = self.footprint.sample(|lat, lon| {
                        values
                            .get_value::<f32, _>([usize::from(month) - 1, lat, lon])
                            .map(f64::from)
                            .with_context(|| {
                                format!("cannot read {variable} from {}", path.display())
                            })
                    })?;
                    // 非土壤 patch 清零；`< 1E-10` 也清零（`MOD_NitrifData.F90`）。
                    let value = if patchclass == 0 { 0.0 } else { value };
                    Ok(if value < 1.0e-10 { 0.0 } else { value })
                })
                .collect()
        };
        Ok((read("CONC_O2_UNSAT")?, read("O2_DECOMP_DEPTH_UNSAT")?))
    }
}

/// `MOD_FireData` 与 `MOD_LightningData`：`DEF_USE_FIRE` 的火灾驱动数据（`DEF_dir_runtime/fire/`）。
///
/// 上游用 `abm` 文件的经纬度建一张网格，`peatf`/`gdp`/`hdm` 都按这张网格读块（假定同网格）；闪电另有
/// 一张网格。单点的面积加权映射就是取包含站点的那一格（与 [`NdepSource`] 同）。
#[derive(Debug, Clone, PartialEq)]
pub struct FireSource {
    dir: PathBuf,
    cell: Footprint,
    lightning_cell: Footprint,
    /// `abm_lf`（作物火高峰月）、`peatf_lf`（泥炭地比例）、`gdp_lf`：启动时读一次。
    pub abm: f64,
    pub peatf: f64,
    pub gdp: f64,
}

const FIRE_ABM: &str = "abm_colm_double_fillcoast.nc";
const FIRE_PEATF: &str = "peatf_colm_360x720_c100428.nc";
const FIRE_GDP: &str = "gdp_colm_360x720_c100428.nc";
const FIRE_HDM: &str =
    "colmforc.Li_2017_HYDEv3.2_CMIP6_hdm_0.5x0.5_AVHRR_simyr1850-2016_c180202.nc";
const FIRE_LIGHTNING: &str = "clmforc.Li_2012_climo1995-2011.T62.lnfm_Total_c140423.nc";

impl FireSource {
    /// `init_fire_data` 的静态部分：`abm`/`peatf`/`gdp`。
    pub fn open(runtime_dir: &Path, locator: Locator<'_>) -> Result<Self> {
        let dir = runtime_dir.join("fire");
        let cell = |file: &str| -> Result<Footprint> {
            let path = dir.join(file);
            let nc = netcdf::open(&path)
                .with_context(|| format!("cannot open the fire data {}", path.display()))?;
            let axis = |name: &str| -> Result<Vec<f64>> {
                nc.variable(name)
                    .with_context(|| format!("{} has no {name}", path.display()))?
                    .get_values::<f64, _>(..)
                    .with_context(|| format!("cannot read {name} from {}", path.display()))
            };
            locator.footprint(&axis("lat")?, &axis("lon")?)
        };
        let grid = cell(FIRE_ABM)?;
        let lightning_cell = cell(FIRE_LIGHTNING)?;
        let mut source = Self {
            dir,
            cell: grid,
            lightning_cell,
            abm: 0.0,
            peatf: 0.0,
            gdp: 0.0,
        };
        source.abm = source.read(FIRE_ABM, "abm", None, &source.cell)?;
        source.peatf = source.read(FIRE_PEATF, "peatf", None, &source.cell)?;
        source.gdp = source.read(FIRE_GDP, "gdp", None, &source.cell)?;
        Ok(source)
    }

    fn read(
        &self,
        file: &str,
        variable: &str,
        time: Option<usize>,
        footprint: &Footprint,
    ) -> Result<f64> {
        let path = self.dir.join(file);
        let nc = netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
        let variable_ref = nc
            .variable(variable)
            .with_context(|| format!("{} has no {variable}", path.display()))?;
        footprint.sample(|lat, lon| {
            match time {
                Some(t) => variable_ref.get_value::<f64, _>([t, lat, lon]),
                None => variable_ref.get_value::<f64, _>([lat, lon]),
            }
            .with_context(|| format!("cannot read {variable} from {}", path.display()))
        })
    }

    /// `update_hdm_data(YY)`：`itime = max(1850, min(YY, 2016)) - 1849`（1 起）。
    pub fn hdm(&self, year: i32) -> Result<f64> {
        let itime = usize::try_from(year.clamp(1850, 2016) - 1850).expect("clamped");
        self.read(FIRE_HDM, "hdm", Some(itime), &self.cell)
    }

    /// 闪电气候态的第 `itime` 条（1 起，3 小时一条，一年 2920 条）。
    pub fn lightning(&self, itime: usize) -> Result<f64> {
        ensure!(itime >= 1, "the lightning record index starts at 1");
        self.read(
            FIRE_LIGHTNING,
            "lnfm",
            Some(itime - 1),
            &self.lightning_cell,
        )
    }
}

/// `update_lightning_data(itstamp, deltim)`：步首与步末落在不同的 3 小时档时返回要读的那一档（1 起）。
///
/// 步首档：`(day-1)*8 + min(sec/10800+1, 8)`，恰在档边界上时减 1（算作上一档）；步末档：
/// `(day'-1)*8 + max(0, sec'-1)/10800 + 1`，不超过 2920（闰年最后一天沿用前一天的档）。
pub fn lightning_record_due(
    begin: colm_core::calendar::CalendarTime,
    deltim: f64,
) -> Result<Option<usize>> {
    let day = i64::from(begin.julian_day);
    let sec = i64::from(begin.seconds);
    let mut itime = (day - 1) * 8 + (sec / 10800 + 1).min(8);
    if sec % 10800 == 0 {
        itime -= 1;
    }
    // `time_next = time + int(deltim)`：上游 `addsec` 只在秒数**超过** 86400 时进位，23:30 起步的步末是
    // "当天 86400 秒"而不是"次日 0 秒"——前者与步首同档、不换，后者会提前读下一档。
    let (mut next_year, mut next_day, mut next_sec) =
        (begin.year, i64::from(begin.julian_day), sec + deltim as i64);
    while next_sec > 86400 {
        next_sec -= 86400;
        next_day += 1;
        let days = if colm_core::calendar::is_leap_year(next_year) {
            366
        } else {
            365
        };
        if next_day > days {
            next_year += 1;
            next_day = 1;
        }
    }
    let itime_next = (next_day - 1) * 8 + (next_sec - 1).max(0) / 10800 + 1;
    if itime_next == itime {
        return Ok(None);
    }
    Ok(Some(
        usize::try_from(itime_next.min(2920)).context("a negative lightning index")?,
    ))
}

/// `grid%define_by_center`：网格边界取相邻中心的中点，返回包含 `x` 的格子。
pub(crate) fn containing_cell(centers: &[f64], x: f64, periodic: bool) -> Result<usize> {
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
    /// 启动时读氮沉降（与火的人口密度）用的年份：namelist 的起始年 `s_year`。
    pub ndep_start_year: i32,
    /// 月度氮沉降起步读的月：namelist 的 `start_month`，**不**经 `adj2end`
    /// （`init_ndep_data_monthly(sdate(1), s_month)`，年与月可能不在同一个时刻上）。
    pub ndep_start_month: u8,
    pub nitrif: Option<(NitrifSource, u8)>,
    /// `DEF_USE_FIRE` 打开时的火灾数据。
    pub fire: Option<FireSource>,
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
    /// `DEF_USE_FIRE` 打开时的火灾数据。
    pub fire: Option<FireSource>,
    /// 启动时读氮沉降（与火的人口密度）用的年份：namelist 的起始年 `s_year`。
    pub ndep_start_year: i32,
    /// `deltim`（秒）。
    pub deltim: f64,
    /// `DEF_USE_IRRIGATION`（CROP）的设置；灌溉状态本身在 patch 状态上。
    pub irrigation: Option<colm_core::IrrigationSettings>,
    /// CH4 provider（注册了 CH4 示踪物时）：配置与本 patch 的静态量。
    pub methane: Option<(
        crate::methane::MethaneSetup,
        colm_core::methane::driver::MethaneSite,
    )>,
    /// 本 patch 的 `patchtype`（装到模板上时写入）：只有土壤 patch（0）跑 `bgc_driver`
    /// （`CoLMDRIVER.F90:237-244`），其余 patch 的 BGC 状态只随步首的数据更新。
    pub patch_type: i32,
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
            fire: self.fire.clone(),
            ndep_start_year: self.ndep_start_year,
            deltim: self.deltim,
            irrigation: self.irrigation,
            methane: self.methane.clone(),
            patch_type: self.patch_type,
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
            ndep_start_month,
            nitrif,
            fire,
        } = sources;
        // `init_ndep_data_annually`/`init_ndep_data_monthly`：步进之前就写好 `ndep`/`ndep_to_sminn`。
        let (ndep_value, to_sminn) = if ndep.is_monthly() {
            ndep.month(ndep_start_year, ndep_start_month, statics.patchclass)?
        } else {
            ndep.annual(ndep_start_year, statics.patchclass)?
        };
        initial.patch.ndep[0] = ndep_value;
        initial.patch_flux.ndep_to_sminn[0] = to_sminn;
        // `init_nitrif_data(ststamp)`：起始时刻所在的月。
        if let Some((source, month)) = &nitrif {
            let (conc, depth) = source.monthly(*month, statics.patchclass)?;
            initial.patch.tconc_o2_unsat.copy_from_slice(&conc);
            initial.patch.to2_decomp_depth_unsat.copy_from_slice(&depth);
        }
        let nitrif = nitrif.map(|(source, _)| source);
        // `init_fire_data(s_year)`：`abm`/`gdp`/`peatf` 与起始年（同 ndep）的 `hdm`。
        // `lnfm` 不在重启里、分配时是 `spval`；`init_lightning_data` 读了闪电却没映射到 patch，要等
        // 第一次 `update_lightning_data` 换档才有值。
        if let Some(source) = &fire {
            initial.invariants.abm_lf[0] = source.abm;
            initial.invariants.gdp_lf[0] = source.gdp;
            initial.invariants.peatf_lf[0] = source.peatf;
            initial.patch.hdm_lf[0] = source.hdm(ndep_start_year)?;
            initial.patch.lnfm[0] = MISSING;
        }
        Ok(Self {
            initial,
            pft,
            switches,
            statics,
            ndep,
            nitrif,
            fire,
            ndep_start_year,
            deltim,
            irrigation: None,
            methane: None,
            patch_type: 0,
            trace: Mutex::new(TraceWriter::from_env()?),
        })
    }

    /// 步首的数据更新（`CoLM.F90:495-541`，对所有 patch 都做）：硝化 O2、闪电、月度/年度氮沉降与
    /// 人口密度。湖泊、冰川 patch 只走这一段（[`Self::update_non_soil`]）。
    fn update_data(
        &self,
        begin: colm_core::calendar::CalendarTime,
        idate: [i32; 3],
        bgc: &mut BgcState,
    ) -> Result<()> {
        let deltim = self.deltim;
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
        // `update_lightning_data(itstamp, deltim)`（`CoLM.F90:503-505`）：步首与步末跨了 3 小时档就换。
        if let Some(fire) = &self.fire {
            if let Some(itime) = lightning_record_due(begin, deltim)? {
                bgc.patch.lnfm[0] = fire.lightning(itime)?;
            }
        }
        // `CoLM.F90:523-531`：`TICKTIME` 之后、`CoLMDRIVER` 之前。月度按步末（`adj2begin`）与步首
        // 的年月比，读步末那个月；年度见下（读的是 `idate(1)`，跨年那一步仍是旧年）。
        if self.ndep.is_monthly() {
            let (year, month) = year_month(step_end_begin_form(idate)?)?;
            if (year, month) != year_month(begin)? {
                let (ndep, to_sminn) = self.ndep.month(year, month, self.statics.patchclass)?;
                bgc.patch.ndep[0] = ndep;
                bgc.patch_flux.ndep_to_sminn[0] = to_sminn;
            }
        }
        if colm_core::bgc_driver::is_end_of_year(idate, deltim) {
            if !self.ndep.is_monthly() {
                let (ndep, to_sminn) = self.ndep.annual(idate[0], self.statics.patchclass)?;
                bgc.patch.ndep[0] = ndep;
                bgc.patch_flux.ndep_to_sminn[0] = to_sminn;
            }
            // `update_hdm_data(idate(1))`：与年度 ndep 同一个条件、同一个年份（`CoLM.F90:537-541`）。
            if let Some(fire) = &self.fire {
                bgc.patch.hdm_lf[0] = fire.hdm(idate[0])?;
            }
        }
        Ok(())
    }

    /// 湖泊、冰川 patch（`CoLMDRIVER` 里 `bgc_driver` 与 CH4 都不跑）：只做步首的数据更新。
    pub fn update_non_soil(
        &self,
        begin: colm_core::calendar::CalendarTime,
        idate: [i32; 3],
        state: &mut StandardLctSnowSoilState,
    ) -> Result<()> {
        let bgc = state
            .bgc
            .as_deref_mut()
            .context("a BGC patch needs its BGC state")?;
        self.update_data(begin, idate, bgc)
    }

    /// 湖泊 patch 的甲烷（`tracer_lake_step` → `ch4_impl_lake_step`）：`CoLMMAIN` 之后、在第
    /// `isub`/`nsub` 个水体子步里以子步步长跑；`nsub > 1` 时诊断量按时间加权平均（`mean`）。
    /// 动态湖：本子步物理之后 `wdsrf < 100 .or. zwt > 0` 时走干湖支
    /// （[`colm_core::methane::driver::dry_lake_substep`]），不论这个子步跑的是湖面还是土壤物理。
    #[allow(clippy::too_many_arguments)]
    pub fn lake_methane(
        &self,
        idate: [i32; 3],
        forcing: &colm_core::RuntimeForcing,
        partial_pressures_pa: (f64, f64),
        state: &mut StandardLctSnowSoilState,
        surface: crate::methane::LakeMethaneSurface,
        lakedepth: f64,
        dynamic_lake: bool,
        (isub, nsub): (usize, usize),
        mean: &mut colm_core::methane::driver::LakeSubstepMean,
    ) -> Result<()> {
        let Some((setup, site)) = &self.methane else {
            return Ok(());
        };
        if !setup.params.methane.allowlakeprod {
            return Ok(());
        }
        let substep_dt = self.deltim / nsub as f64;
        let dry = dynamic_lake
            && (state.soil_water.surface_water_mm < 100.0
                || state.soil_water.water_table_depth_m > 0.0);
        let mut bgc = state
            .bgc
            .take()
            .context("a BGC patch needs its BGC state")?;
        let mut patch = bgc
            .methane
            .take()
            .context("a methane patch needs its methane state")?;
        let outcome = (|| {
            if dry {
                colm_core::methane::driver::dry_lake_substep(&setup.params, &mut patch, substep_dt);
                return Ok(());
            }
            let arrays = crate::methane::HostArrays::from_lake_state(state)?;
            let host = crate::methane::lake_host_inputs(
                &arrays,
                idate,
                substep_dt,
                state,
                surface,
                forcing,
                partial_pressures_pa,
                lakedepth,
                setup.dynamic_wetland,
            )?;
            colm_core::methane::driver::soil_step(
                &setup.params,
                setup.scheme,
                site,
                &host,
                &mut bgc,
                &mut patch,
            )
        })();
        if outcome.is_ok() && nsub > 1 {
            if isub == 1 {
                *mean = colm_core::methane::driver::LakeSubstepMean::default();
            }
            mean.add(&patch, substep_dt);
            if isub == nsub {
                mean.finish(&mut patch, substep_dt * nsub as f64);
            }
        }
        bgc.methane = Some(patch);
        state.bgc = Some(bgc);
        outcome
    }

    /// 一步：（必要时）更新氮沉降 → `bgc_driver` → 写回物理量。
    pub fn step(
        &self,
        begin: colm_core::calendar::CalendarTime,
        idate: [i32; 3],
        forcing: &colm_core::RuntimeForcing,
        partial_pressures_pa: (f64, f64),
        state: &mut StandardLctSnowSoilState,
        output: &StandardLctSnowSoilOutput,
    ) -> Result<()> {
        let deltim = self.deltim;
        let mut bgc = state
            .bgc
            .take()
            .context("a BGC patch needs its BGC state")?;
        self.update_data(begin, idate, &mut bgc)?;
        if self.patch_type != 0 {
            // 湿地 CH4（`CoLMDRIVER.F90:245-247`）：`tracer_wetland_decomp` 借土壤分解级联算逐层
            // 异养呼吸，`tracer_soil_step` 跑甲烷，finalize 按这一步的分解通量直接推进分解池。
            if let (2, Some((setup, site))) = (self.patch_type, &self.methane) {
                let physics = self.wetland_physics(idate, deltim, state);
                colm_core::bgc_wetland::wetland_decomp(&mut bgc, &physics, self.switches, deltim);
                let mut patch = bgc
                    .methane
                    .take()
                    .context("a methane patch needs its methane state")?;
                let arrays = crate::methane::HostArrays::from_state(state, output)?;
                let host = crate::methane::host_inputs(
                    &arrays,
                    idate,
                    deltim,
                    state,
                    output,
                    forcing,
                    partial_pressures_pa,
                    colm_core::methane::bgc_link::PftInputs::default(),
                    setup.dynamic_wetland,
                );
                colm_core::methane::driver::soil_step(
                    &setup.params,
                    setup.scheme,
                    site,
                    &host,
                    &mut bgc,
                    &mut patch,
                )?;
                // finalize 的湿地前半：甲烷读完这一步的池（`cellorg`）之后才推进分解池。
                colm_core::bgc_wetland::wetland_state_update(
                    &mut bgc,
                    &physics,
                    &self.pft,
                    self.switches,
                    deltim,
                );
                bgc.methane = Some(patch);
            }
            state.bgc = Some(bgc);
            return Ok(());
        }
        let mut physics = self.physics(idate, deltim, forcing, state, output, &bgc)?;
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
            let irrigation = match (self.irrigation, state.irrigation.as_deref_mut()) {
                (Some(settings), Some(irrigation)) => Some(BgcIrrigation {
                    state: irrigation,
                    settings,
                    water_table_depth_m: &mut state.soil_water.water_table_depth_m,
                    aquifer_water_mm: &mut state.soil_water.aquifer_water_mm,
                }),
                (None, None) => None,
                _ => bail!("DEF_USE_IRRIGATION needs both the irrigation settings and state"),
            };
            let mut step = BgcStep {
                state: &mut bgc,
                physics: &mut physics,
                pft: &self.pft,
                switches: self.switches,
                irrigation,
            };
            bgc_driver(&mut step, &mut record)?;
        }
        self.write_back(&physics, state)?;
        // `tracer_soil_step`（`CoLMDRIVER.F90:246-248`）：`bgc_driver` 之后跑甲烷，改写
        // `decomp_hr`/`er`。
        if let Some((setup, site)) = &self.methane {
            let mut patch = bgc
                .methane
                .take()
                .context("a methane patch needs its methane state")?;
            let arrays = crate::methane::HostArrays::from_state(state, output)?;
            let host = crate::methane::host_inputs(
                &arrays,
                idate,
                deltim,
                state,
                output,
                forcing,
                partial_pressures_pa,
                colm_core::methane::bgc_link::PftInputs {
                    class: &physics.pftclass,
                    fraction: &physics.pftfrac,
                    lai: &physics.lai_p,
                    irrig_method: &physics.irrig_method_p,
                },
                setup.dynamic_wetland,
            );
            colm_core::methane::driver::soil_step(
                &setup.params,
                setup.scheme,
                site,
                &host,
                &mut bgc,
                &mut patch,
            )?;
            bgc.methane = Some(patch);
        }
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
        if self.switches.crop {
            for (slot, name) in bgc
                .irrigation_diagnostics
                .iter_mut()
                .zip(colm_core::bgc_state::IRRIGATION_DIAGNOSTICS)
            {
                if let Some((_, values)) = inputs.iter().find(|(field, _)| *field == name) {
                    *slot = values[0];
                }
            }
        }
        state.bgc = Some(bgc);
        Ok(())
    }

    /// 湿地 patch（没有 PFT）分解级联与非植被汇总要的那部分物理量。
    fn wetland_physics(
        &self,
        idate: [i32; 3],
        deltim: f64,
        state: &StandardLctSnowSoilState,
    ) -> BgcPhysics {
        let nl = self.initial.dims.nl_soil;
        let soil = |name: &str| {
            self.statics
                .soil
                .iter()
                .find(|(field, _)| *field == name)
                .map_or_else(|| vec![MISSING; nl], |(_, values)| values.clone())
        };
        BgcPhysics {
            idate,
            patchclass: self.statics.patchclass,
            z_soi: self.statics.z_soi.clone(),
            dz_soi: self.statics.dz_soi.clone(),
            zi_soi: self.statics.zi_soi.clone(),
            BD_all: soil("BD_all"),
            t_soisno: state.soil_temperature_k[..nl].to_vec(),
            smp: state.soil_water.matric_potential_mm[..nl].to_vec(),
            deltim,
            smpmax_hr: self.statics.smpmax_hr,
            smpmin_hr: self.statics.smpmin_hr,
            ..BgcPhysics::default()
        }
    }

    fn physics(
        &self,
        idate: [i32; 3],
        deltim: f64,
        forcing: &colm_core::RuntimeForcing,
        state: &StandardLctSnowSoilState,
        output: &StandardLctSnowSoilOutput,
        previous: &BgcState,
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
        // 分 PFT 类型 LAI（`MOD_Thermal.F90:891-935`）：LAI 反馈关闭时 Thermal 每步先清零，再按 PFT
        // 类别 1..=14 填 `lai_p`（同类多个 PFT 时后者覆盖前者）；打开时不碰，沿用上一步汇总写的值。
        let lai_diagnostics = if self.switches.laifeedback {
            previous.lai_diagnostics
        } else {
            let mut values = [0.0; 14];
            for (parameters, column) in pft.parameters.iter().zip(columns) {
                if (1..=14).contains(&parameters.class) {
                    values[parameters.class as usize - 1] = column.leaf_area_index;
                }
            }
            values
        };
        // CROP 内核里灌溉诊断量分配为整型 spval（`spval_i4 = -9999`），`CROP_readin` 把
        // `irrig_method_p` 置为 −99999999；灌溉关闭时二者都不再变。非 CROP 内核没有这些量。
        let irrigation = |k: usize| {
            vec![if self.switches.crop {
                previous.irrigation_diagnostics[k]
            } else {
                MISSING
            }]
        };
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
            zwt: vec![state.soil_water.water_table_depth_m],
            // `WATER_2014` 不写 `frcsat`（保持 `spval`）；干湖等未定义的值同样按 `spval`。
            frcsat: vec![if self.switches.variably_saturated
                && output.water.soil.saturated_fraction.is_finite()
            {
                output.water.soil.saturated_fraction
            } else {
                MISSING
            }],
            fsatmax: vec![self.statics.topmodel[0]],
            fsatdcf: vec![self.statics.topmodel[1]],
            topoweti: vec![self.statics.topmodel[2]],
            alp_twi: vec![self.statics.topmodel[3]],
            chi_twi: vec![self.statics.topmodel[4]],
            mu_twi: vec![self.statics.topmodel[5]],
            deltim,
            dlat: degrees(self.statics.patchlatr),
            dlon: degrees(self.statics.patchlonr),
            smpmax_hr: self.statics.smpmax_hr,
            smpmin_hr: self.statics.smpmin_hr,
            lai_enftemp: vec![lai_diagnostics[0]],
            lai_enfboreal: vec![lai_diagnostics[1]],
            lai_dnfboreal: vec![lai_diagnostics[2]],
            lai_ebftrop: vec![lai_diagnostics[3]],
            lai_ebftemp: vec![lai_diagnostics[4]],
            lai_dbftrop: vec![lai_diagnostics[5]],
            lai_dbftemp: vec![lai_diagnostics[6]],
            lai_dbfboreal: vec![lai_diagnostics[7]],
            lai_ebstemp: vec![lai_diagnostics[8]],
            lai_dbstemp: vec![lai_diagnostics[9]],
            lai_dbsboreal: vec![lai_diagnostics[10]],
            lai_c3arcgrass: vec![lai_diagnostics[11]],
            lai_c3grass: vec![lai_diagnostics[12]],
            lai_c4grass: vec![lai_diagnostics[13]],
            irrig_method_corn: irrigation(0),
            irrig_method_swheat: irrigation(1),
            irrig_method_wwheat: irrigation(2),
            irrig_method_soybean: irrigation(3),
            irrig_method_cotton: irrigation(4),
            irrig_method_rice1: irrigation(5),
            irrig_method_rice2: irrigation(6),
            irrig_method_sugarcane: irrigation(7),
            // 灌溉打开时是 `CROP_readin` 读进、`PointNeedsCheckForIrrig` 可能改过的 `irrig_method_p`。
            irrig_method_p: match &state.irrigation {
                Some(irrigation) => irrigation.methods.iter().map(|&m| f64::from(m)).collect(),
                None => vec![
                    if self.switches.crop {
                        -99_999_999.0
                    } else {
                        MISSING
                    };
                    npft
                ],
            },
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
        // 非 VSF 的灌溉取水（`CalWithdrawalWATER`）改土壤液态水；`zwt`/`wa` 已就地改过。
        if state.irrigation.is_some() {
            let nl = physics.wliq_soisno.len();
            state.soil_water.liquid_water_kg_m2[..nl].copy_from_slice(&physics.wliq_soisno);
        }
        Ok(())
    }
}

/// 日历时刻所在的 `(年, 月)`。
fn year_month(time: colm_core::calendar::CalendarTime) -> Result<(i32, u8)> {
    Ok((time.year, colm_core::calendar::month_day(time)?.0))
}

/// 步末 `idate` 按 `adj2begin` 换写：一天的末尾（86400 秒）写成次日 0 秒。
fn step_end_begin_form(idate: [i32; 3]) -> Result<colm_core::calendar::CalendarTime> {
    let (mut year, mut day, mut seconds) = (idate[0], idate[1], idate[2]);
    if seconds >= 86_400 {
        seconds -= 86_400;
        day += 1;
        if day
            > if colm_core::calendar::is_leap_year(year) {
                366
            } else {
                365
            }
        {
            year += 1;
            day = 1;
        }
    }
    Ok(colm_core::calendar::CalendarTime {
        year,
        julian_day: u16::try_from(day)?,
        seconds: u32::try_from(seconds)?,
    })
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
/// `CROP_readin` 读数据时需要的站点与目录（`DEF_dir_runtime/crop/`）。
#[derive(Debug, Clone)]
pub struct CropReadinData<'a> {
    pub runtime_dir: &'a Path,
    /// patch 在作物数据网格上的定位（`mg2patch_crop`）。
    pub patch: Locator<'a>,
    /// 本 patch 每个 PFT 的定位（`mg2pft_crop`/`mg2pft_fert`），与 PFT 一一对应。
    pub pfts: Vec<Locator<'a>>,
    /// `DEF_FERT_SOURCE`（1 或 2）。
    pub fert_source: i64,
    /// `DEF_IRRIGATION_ALLOCATION`（灌溉打开时才用；3 才读配水比例图）。
    pub irrigation_allocation: i32,
}

const CROP_PLANTING: &str = "crop/plantdt-colm-64cfts-rice2_fillcoast.nc";
const CROP_FERT_ONE: &str = "crop/fertnitro_fillcoast.nc";
const CROP_FERT_TWO: &str = "crop/fertilizer_2015soc.nc";
const CROP_IRRIGATION_METHOD: &str = "crop/surfdata_irrigation_method_96x144.nc";
const CROP_IRRIGATION_ALLOCATION: &str = "crop/surfdata_irrigation_allocation.nc";
const CROP_ABSENT: f64 = -99_999_999.0;

/// 作物数据文件里单点所在格点的读取（`define_by_center` + 面积加权，单点即包含站点的那一格）。
/// 一张作物数据网格上的映射：patch 一份（`mg2patch_*`）、每个 PFT 一份（`mg2pft_*`）。
struct CropGrid {
    patch: Footprint,
    pfts: Vec<Footprint>,
}

impl CropGrid {
    fn open(path: &Path, patch: Locator<'_>, pfts: &[Locator<'_>]) -> Result<Self> {
        let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        let axis = |name: &str| -> Result<Vec<f64>> {
            file.variable(name)
                .with_context(|| format!("{} has no {name}", path.display()))?
                .get_values::<f64, _>(..)
                .with_context(|| format!("cannot read {name} from {}", path.display()))
        };
        let (lat, lon) = (axis("lat")?, axis("lon")?);
        Ok(Self {
            patch: patch.footprint(&lat, &lon)?,
            pfts: pfts
                .iter()
                .map(|locator| locator.footprint(&lat, &lon))
                .collect::<Result<_>>()?,
        })
    }

    /// 上游 `set_missing_value`：种植日文件用 `pdrice2` 的 `missing_value`，按 `pdrice2` 在各格的值
    /// 把落在缺测格上的份面积清零、重算面积和；之后按同一映射读的量（种植日、施肥来源 1）都用它。
    fn mask_missing(&mut self, path: &Path, name: &str, missing: f64) -> Result<()> {
        let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        let variable = file
            .variable(name)
            .with_context(|| format!("{} has no {name}", path.display()))?;
        let read = |lat: usize, lon: usize| -> Result<f64> {
            variable
                .get_value::<f64, _>([lat, lon])
                .with_context(|| format!("cannot read {name} from {}", path.display()))
        };
        self.patch.mask_missing(&read, missing)?;
        for footprint in &mut self.pfts {
            footprint.mask_missing(&read, missing)?;
        }
        Ok(())
    }

    fn sample(footprint: &Footprint, path: &Path, name: &str, index: Option<usize>) -> Result<f64> {
        let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        let variable = file
            .variable(name)
            .with_context(|| format!("{} has no {name}", path.display()))?;
        // `ncio_read_block_time` 对没有时间维的二维变量就读那一张（种植日文件的 `PLANTDATE_CFT_xx`）。
        let leading = variable.dimensions().len() > 2;
        footprint.sample(|lat, lon| {
            match index.filter(|_| leading) {
                Some(k) => variable.get_value::<f64, _>([k, lat, lon]),
                None => variable.get_value::<f64, _>([lat, lon]),
            }
            .with_context(|| format!("cannot read {name} from {}", path.display()))
        })
    }

    /// patch 映射下第 `index` 个前导切片（二维变量为 `None`）的值；没有有效面积时是 `spval`。
    fn read(&self, path: &Path, name: &str, index: Option<usize>) -> Result<f64> {
        Self::sample(&self.patch, path, name, index)
    }

    /// 第 `m` 个 PFT 的映射下的值。
    fn read_pft(&self, m: usize, path: &Path, name: &str, index: Option<usize>) -> Result<f64> {
        Self::sample(&self.pfts[m], path, name, index)
    }

    /// 第 `m` 个 PFT 的映射下第 `index` 个前导切片的主导值（`grid2pset_dominant`）。
    fn dominant_pft(&self, m: usize, path: &Path, name: &str, index: usize) -> Result<Option<f64>> {
        let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        let variable = file
            .variable(name)
            .with_context(|| format!("{} has no {name}", path.display()))?;
        self.pfts[m].dominant(|lat, lon| {
            variable
                .get_value::<f64, _>([index, lat, lon])
                .with_context(|| format!("cannot read {name} from {}", path.display()))
        })
    }
}

/// `CROP_readin`（`MOD_CropReadin.F90`，`CoLM.F90:442` 启动时调用）：覆盖作物的播种日与施肥量。
///
/// - 设了播种日（`DEF_TUNING_CROP_PLANTING_DAY > 0`）且施肥、灌溉都关：作物 PFT（类别 15..=78）的
///   `plantdate_p` 取播种日、其余 −99999999，`fertnitro_p = manunitro_p = 0`，`pdrice2 = 0`，不读文件。
/// - 否则读 `crop/` 的种植日文件：`pdrice2`（缺测为 0，否则截断取整）、`PLANTDATE_CFT_xx`（非正或缺测为
///   −99999999），再按播种日覆盖；`DEF_USE_FERT` 时来源 1 读 `CONST_FERTNITRO_CFT_xx`（种植日文件的网格与缺测值，
///   非正为 0，非作物 PFT 保持 −99999999），来源 2 读 `fertilizer_2015soc.nc` 的 `manure` 与 `fertilizer`（负值为 0）。
///
/// - `DEF_USE_IRRIGATION`：`irrigation_method(cft, lat, lon)` 按自己的网格取作物 PFT 的灌溉方式（负值为
///   −99999999，其余 PFT 也是 −99999999）；`DEF_IRRIGATION_ALLOCATION = 3` 时再读地下水/地表水配比。
///   这些不在 BGC 状态里，以返回值交给调用方（[`crate::irrigation::initial_state`]）。
///
/// 冷启动（mkinidata）也调同一个过程，但之后 `IniTimeVariable` 还会写 `manunitro_p = manure·1000`；运行期
/// 这里重读，所以来源 1 下重启里的 `manunitro_p` 在运行期被清零（第 424 轮）。
pub fn crop_readin(
    state: &mut BgcState,
    classes: &[i32],
    planting_day: f64,
    switches: BgcSwitches,
    data: CropReadinData<'_>,
) -> Result<Option<crate::irrigation::IrrigationReadin>> {
    ensure!(
        classes.len() == state.pft.plantdate_p.len(),
        "the PFT class list does not match the BGC PFT state"
    );
    let crop = |class: i32| (15..=78).contains(&class);
    if planting_day > 0.0 && !switches.fert && !switches.irrigation {
        state.patch.pdrice2[0] = 0.0;
        for (m, &class) in classes.iter().enumerate() {
            state.pft.plantdate_p[m] = if crop(class) {
                planting_day
            } else {
                CROP_ABSENT
            };
            state.pft.fertnitro_p[m] = 0.0;
            state.pft.manunitro_p[m] = 0.0;
        }
        return Ok(None);
    }
    ensure!(
        data.pfts.len() == classes.len(),
        "CROP_readin needs one PFT locator per PFT"
    );
    let planting_path = data.runtime_dir.join(CROP_PLANTING);
    let mut grid = CropGrid::open(&planting_path, data.patch, &data.pfts)?;
    let missing = {
        let file = netcdf::open(&planting_path)
            .with_context(|| format!("cannot open {}", planting_path.display()))?;
        let variable = file
            .variable("pdrice2")
            .with_context(|| format!("{} has no pdrice2", planting_path.display()))?;
        match variable.attribute_value("missing_value").transpose()? {
            Some(netcdf::AttributeValue::Double(value)) => Some(value),
            Some(netcdf::AttributeValue::Float(value)) => Some(f64::from(value)),
            Some(other) => bail!("unsupported pdrice2 missing_value {other:?}"),
            None => None,
        }
    };
    if let Some(missing) = missing {
        grid.mask_missing(&planting_path, "pdrice2", missing)?;
    }
    let rice2 = grid.read(&planting_path, "pdrice2", None)?;
    // `int(pdrice2_tmp)`：向零截断。
    state.patch.pdrice2[0] = if rice2 == MISSING { 0.0 } else { rice2.trunc() };
    for (m, &class) in classes.iter().enumerate() {
        state.pft.plantdate_p[m] = CROP_ABSENT;
        if crop(class) {
            let day = grid.read_pft(
                m,
                &planting_path,
                &format!("PLANTDATE_CFT_{class:02}"),
                Some(0),
            )?;
            state.pft.plantdate_p[m] = if day <= 0.0 { CROP_ABSENT } else { day };
        }
    }
    if planting_day > 0.0 {
        for (m, &class) in classes.iter().enumerate() {
            if crop(class) {
                state.pft.plantdate_p[m] = planting_day;
            }
        }
    }
    state.pft.fertnitro_p.fill(0.0);
    state.pft.manunitro_p.fill(0.0);
    if switches.fert {
        match data.fert_source {
            1 => {
                let path = data.runtime_dir.join(CROP_FERT_ONE);
                state.pft.fertnitro_p.fill(CROP_ABSENT);
                for (m, &class) in classes.iter().enumerate() {
                    if crop(class) {
                        let fert = grid.read_pft(
                            m,
                            &path,
                            &format!("CONST_FERTNITRO_CFT_{class:02}"),
                            Some(0),
                        )?;
                        state.pft.fertnitro_p[m] = if fert <= 0.0 { 0.0 } else { fert };
                    }
                }
            }
            2 => {
                let path = data.runtime_dir.join(CROP_FERT_TWO);
                let grid_two = CropGrid::open(&path, data.patch, &data.pfts)?;
                state.pft.fertnitro_p.fill(CROP_ABSENT);
                state.pft.manunitro_p.fill(CROP_ABSENT);
                for (m, &class) in classes.iter().enumerate() {
                    if class >= 15 {
                        let manure = grid_two.read_pft(m, &path, "manure", None)?;
                        state.pft.manunitro_p[m] = if manure < 0.0 { 0.0 } else { manure };
                    }
                    if crop(class) {
                        let index = usize::try_from(class - 15).expect("crop class");
                        let fert = grid_two.read_pft(m, &path, "fertilizer", Some(index))?;
                        state.pft.fertnitro_p[m] = if fert < 0.0 { 0.0 } else { fert };
                    }
                }
            }
            source => bail!("DEF_FERT_SOURCE must be 1 or 2, got {source}"),
        }
    }
    if !switches.irrigation {
        return Ok(None);
    }
    let path = data.runtime_dir.join(CROP_IRRIGATION_METHOD);
    let grid = CropGrid::open(&path, data.patch, &data.pfts)?;
    let mut methods = vec![-99_999_999; classes.len()];
    for (m, &class) in classes.iter().enumerate() {
        if crop(class) {
            let index = usize::try_from(class - 15).expect("crop class");
            // `grid2pset_dominant`：面积最大的那一格的整数值；没有有效面积时上游给 −9999，随后与负值一样
            // 变成 −99999999。
            methods[m] = match grid.dominant_pft(m, &path, "irrigation_method", index)? {
                Some(method) if method >= 0.0 => method as i32,
                _ => -99_999_999,
            };
        }
    }
    let allocation = if data.irrigation_allocation == 3 {
        let path = data.runtime_dir.join(CROP_IRRIGATION_ALLOCATION);
        let grid = CropGrid::open(&path, data.patch, &data.pfts)?;
        Some((
            grid.read(&path, "irrig_gw_alloc", None)?,
            grid.read(&path, "irrig_sw_alloc", None)?,
        ))
    } else {
        None
    };
    Ok(Some(crate::irrigation::IrrigationReadin {
        methods,
        allocation,
    }))
}

#[cfg(test)]
#[path = "bgc_step_tests.rs"]
mod bgc_step_tests;

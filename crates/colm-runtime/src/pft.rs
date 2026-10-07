//! `DEF_USE_PFT` 的装配：PFT 常数/时间重启 → [`colm_core::PftPatch`]，续跑时写回。
//!
//! 单点的 PFT 重启只有一个 patch，文件里的 `pft` 维就是这个 patch 的全部 PFT
//! （`patch_pft_s = 1`、`patch_pft_e = npft`），所以这里不读 `patch_pft_s/e`。
//! 参数与 `mkinidata` 走同一张表：`MOD_Const_PFT` 的默认值加 `DEF_PFT_*(class+1)` 覆盖
//! （[`colm_init::pft_parameter`]）。

use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};
use colm_core::{
    HydraulicModel, LeafBiochemistry, LeafOptics, LeafTemperatureState, PftColumn, PftParameters,
    PftPatch, PlantHydraulicState, PlantHydraulicTraits, VEGETATION_SEGMENTS,
};
use colm_init::{RestartFile, RestartOverride};
use colm_namelist::Document;

use crate::assembly::LandPhysicsParameters;

/// 一个 PFT patch 的初始子网格状态与月度 LAI 源。
#[derive(Debug, Clone)]
pub struct PftTemplate {
    pub initial: PftPatch,
    /// PHS 打开时时间重启里才有 `vegwp_p`/`gs0sun_p`/`gs0sha_p`。
    plant_hydraulics: bool,
    /// `DEF_USE_OZONESTRESS`：PFT 时间重启里有 `lai_old_p` 等七个臭氧变量。
    ozone: bool,
    monthly: Option<PftMonthlyLeafAreaIndex>,
    /// 本 patch 在全站 PFT 里的区间（多作物单点每个 patch 一个 PFT；单 patch 时是全部）。
    site_pfts: std::ops::Range<usize>,
    /// 全站 PFT 数。
    site_pft_count: usize,
}

/// `LAI_readin` 的 PFT 段的数据源。
#[derive(Debug, Clone)]
enum PftLeafAreaSource {
    /// 单点（`MOD_LAIReadin.F90:166-185`）：站点表；patch 的 `tlai`/`tsai` 是按站点份额的和。
    SinglePoint {
        vegetation: colm_init::SinglePointPftMonthlyVegetation,
        /// 全站 PFT 的站点份额 `SITE_pctpfts`（作物站点恒为 1）。
        site_fraction: Vec<f64>,
        use_site_lai: bool,
    },
    /// 空间（`MOD_LAIReadin.F90:192-232`）：`landdata/LAI/<年>/` 的分块向量 `LAI/SAI_patches<月>`
    /// （patch 的 `tlai`/`tsai`，第 `patch` 个）与 `LAI/SAI_pfts<月>`（本 patch 的 PFT 区间）。
    Grid {
        directory: PathBuf,
        block: String,
        patch: usize,
    },
}

/// `LAI_readin` 的 PFT 段。
#[derive(Debug, Clone)]
struct PftMonthlyLeafAreaIndex {
    source: PftLeafAreaSource,
    change_yearly: bool,
    land_cover_year: i32,
    start_year: i32,
    end_year: i32,
}

/// 主重启路径对应的 PFT 重启：`<case>_restart_…` → `<case>_restart_pft_…`，
/// 常数文件 `<case>_restart_const_…` → `<case>_restart_pft_const_…`，同目录。
pub fn pft_restart_path(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("a restart path has no file name")?;
    Ok(path.with_file_name(name.replacen("_restart_", "_restart_pft_", 1)))
}

/// 单点第 `patch` 个 patch 的 PFT 区间（0 基、半开）。
///
/// 上游单点只有一种多 patch 情形：CROP 内核、站点是农田、有多种作物（`MOD_SingleSrfdata.F90:348`），
/// 每个作物 patch 恰好一个 PFT。单 patch 时它拥有全部 PFT；其它组合没有来源，报错。
pub fn patch_pft_range(
    patches: usize,
    pfts: usize,
    patch: usize,
) -> Result<std::ops::Range<usize>> {
    ensure!(
        patch < patches,
        "patch {patch} is outside the {patches} patches"
    );
    if patches == 1 {
        Ok(0..pfts)
    } else {
        ensure!(
            pfts == patches,
            "{patches} single-point patches with {pfts} PFTs: only one PFT per crop patch is supported"
        );
        Ok(patch..patch + 1)
    }
}

/// 空间算例里每个（块内）patch 的 PFT 区间（`map_patch_to_pft`，`MOD_LandPFT.F90:264-320`）：
/// 沿 `landpft` 顺序走一个指针 ——
/// - 自然土壤 patch（`patchtype == 0`，CROP 内核下还要不是 `CROPLAND`）吃掉后面同 `eindex`、
///   同 `ipxstt`、`settyp < N_PFT` 的那一串；
/// - CROP 内核的农田 patch 恰好吃一个；
/// - 其余 patch 为空。
///
/// `N_PFT` 不开 CROP 是 16，开 CROP 是 15（`MOD_Vars_Global.F90:41-47`）。
pub fn spatial_pft_ranges(
    landdata: &Path,
    year: i32,
    block: &str,
    scheme: colm_core::LandCoverScheme,
    crop: bool,
) -> Result<Vec<std::ops::Range<usize>>> {
    /// IGBP 的 `CROPLAND`。
    const CROPLAND: i64 = 12;
    let open = |kind: &str| -> Result<(PathBuf, netcdf::File)> {
        let path = landdata
            .join(kind)
            .join(format!("{year:04}"))
            .join(format!("{kind}_{block}.nc"));
        let file =
            netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
        Ok((path, file))
    };
    let read = |(path, file): &(PathBuf, netcdf::File), name: &str| -> Result<Vec<i64>> {
        file.variable(name)
            .with_context(|| format!("{} has no {name}", path.display()))?
            .get_values::<i64, _>(..)
            .with_context(|| format!("cannot read {name} from {}", path.display()))
    };
    let patches = open("landpatch")?;
    let pfts = open("landpft")?;
    let (patch_element, patch_start, patch_type) = (
        read(&patches, "eindex")?,
        read(&patches, "ipxstt")?,
        read(&patches, "settyp")?,
    );
    let (pft_element, pft_start, pft_type) = (
        read(&pfts, "eindex")?,
        read(&pfts, "ipxstt")?,
        read(&pfts, "settyp")?,
    );
    let n_pft: i64 = if crop { 15 } else { 16 };
    let mut ipft = 0usize;
    let mut ranges = Vec::with_capacity(patch_element.len());
    for p in 0..patch_element.len() {
        let class = usize::try_from(patch_type[p]).context("a negative patch settyp")?;
        let kind = colm_core::ClassConstants::new(scheme, class)?.patch_type();
        let cropland = crop && patch_type[p] == CROPLAND;
        if kind == 0 && !cropland {
            let start = ipft;
            while ipft < pft_element.len()
                && pft_element[ipft] == patch_element[p]
                && pft_start[ipft] == patch_start[p]
                && pft_type[ipft] < n_pft
            {
                ipft += 1;
            }
            ranges.push(start..ipft);
        } else if cropland {
            ensure!(
                ipft < pft_element.len(),
                "a cropland patch has no PFT left in landpft"
            );
            ranges.push(ipft..ipft + 1);
            ipft += 1;
        } else {
            ranges.push(0..0);
        }
    }
    Ok(ranges)
}

/// 一块里每个 PFT 的像元与 `pctshared`（`landpft` 的 `eindex/ipxstt/ipxend/pctshared`，按块内顺序）。
pub fn spatial_pft_pixel_sets(
    landdata: &Path,
    year: i32,
    block: &str,
) -> Result<colm_init::spatial_static::SpatialPixelSets> {
    let path = landdata
        .join("landpft")
        .join(format!("{year:04}"))
        .join(format!("landpft_{block}.nc"));
    let file = netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
    let read_i64 = |name: &str| -> Result<Vec<i64>> {
        file.variable(name)
            .with_context(|| format!("{} has no {name}", path.display()))?
            .get_values::<i64, _>(..)
            .with_context(|| format!("cannot read {name} from {}", path.display()))
    };
    let read_i32 = |name: &str| -> Result<Vec<i32>> {
        file.variable(name)
            .with_context(|| format!("{} has no {name}", path.display()))?
            .get_values::<i32, _>(..)
            .with_context(|| format!("cannot read {name} from {}", path.display()))
    };
    let element = read_i64("eindex")?;
    let shared = match file.variable("pctshared") {
        Some(variable) => variable
            .get_values::<f64, _>(..)
            .with_context(|| format!("cannot read pctshared from {}", path.display()))?,
        None => vec![1.0; element.len()],
    };
    colm_init::spatial_static::read_spatial_pixel_sets(
        landdata,
        year,
        block,
        &element,
        &read_i32("ipxstt")?,
        &read_i32("ipxend")?,
        &shared,
        "pft",
    )
}

/// PFT 重启里的 `(patch 数, PFT 数)`。非 CROP 的 PFT 重启没有 `patch` 维，那时就是一个 patch。
pub fn patch_and_pft_counts(file: &RestartFile) -> Result<(usize, usize)> {
    Ok((
        file.dimensions().get("patch").copied().unwrap_or(1),
        file.dimension("pft")?,
    ))
}

/// 打开一份重启并切出第 `patch` 个 patch（见 [`RestartFile::select_patch`]）。
///
/// 没有 `pft` 维的文件（主重启、BGC 重启）只切 `patch`；PFT 区间按 `pfts` 推。
pub fn open_patch(path: &Path, patch: usize, patches: usize, pfts: usize) -> Result<RestartFile> {
    let file = RestartFile::open(path)?;
    let range = patch_pft_range(patches, pfts, patch)?;
    file.select_patch(patch, range)
        .with_context(|| format!("cannot select patch {patch} of {}", path.display()))
}

impl PftTemplate {
    /// 读 PFT 常数与时间重启，按 `physics` 与 namelist 组出逐 PFT 参数。
    ///
    /// `interface_depth_m` 是 `zi_soi(0:nl_soil)`，`rootfr_p` 按它算。
    pub fn read(
        constant_path: &Path,
        time_path: &Path,
        document: &Document,
        physics: &LandPhysicsParameters,
        interface_depth_m: &[f64],
        patch: usize,
        spatial_pfts: Option<std::ops::Range<usize>>,
    ) -> Result<Self> {
        let whole = RestartFile::open(constant_path)?;
        let (patches, site_pft_count) = patch_and_pft_counts(&whole)?;
        // 空间算例：PFT 重启没有 `patch` 维，区间来自 `landpft`（见 [`spatial_pft_ranges`]）。
        let (constant, time, site_pfts) = match spatial_pfts {
            Some(range) => {
                let pick = |path: &Path| -> Result<RestartFile> {
                    RestartFile::open(path)?
                        .select_patch(0, range.clone())
                        .with_context(|| {
                            format!("cannot select PFTs {range:?} of {}", path.display())
                        })
                };
                (pick(constant_path)?, pick(time_path)?, range.clone())
            }
            None => (
                open_patch(constant_path, patch, patches, site_pft_count)?,
                open_patch(time_path, patch, patches, site_pft_count)?,
                patch_pft_range(patches, site_pft_count, patch)?,
            ),
        };
        let classes = constant
            .integers("pftclass")?
            .iter()
            .map(|&class| i32::try_from(class).context("pftclass does not fit an i32"))
            .collect::<Result<Vec<_>>>()?;
        let pfts = classes.len();
        ensure!(pfts > 0, "{} has no PFT", constant_path.display());
        let fraction = constant.floats("pftfrac")?;
        let top = constant.floats("htop_p")?;
        let bottom = constant.floats("hbot_p")?;
        ensure!(
            fraction.len() == pfts && top.len() == pfts && bottom.len() == pfts,
            "the PFT constant restart vectors disagree on the PFT count"
        );
        let campbell = physics.hydraulic_model == HydraulicModel::Campbell;
        let pc = physics.use_pc;
        // 混合模型 `pft` 插槽：按本 patch 的 PFT 次序给出的 `DEF_PFT_*` 覆盖（见 `crate::hybrid`）。
        ensure!(
            physics.pft_overrides.is_empty() || physics.pft_overrides.len() == pfts,
            "the hybrid pft slot gives {} PFT override sets for a patch with {pfts} PFTs",
            physics.pft_overrides.len()
        );
        // `READ_PFTimeInvariants`：`DEF_Interception_scheme == 8` 时读 `ncd_p`/`ncw_p`/`bcw_p`
        // （`MOD_Vars_TimeInvariants.F90:93-97`，没有 `defval`，缺了就停）。
        let crown = if physics.colm2024_interception {
            let read = |name: &str| -> Result<Vec<f64>> {
                let values = constant.floats(name).with_context(|| {
                    format!(
                        "DEF_Interception_scheme = 8 needs {name} in {}",
                        constant_path.display()
                    )
                })?;
                ensure!(
                    values.len() == pfts,
                    "{name} holds {} values for {pfts} PFTs",
                    values.len()
                );
                Ok(values.to_vec())
            };
            Some([read("ncd_p")?, read("ncw_p")?, read("bcw_p")?])
        } else {
            None
        };
        let parameters = classes
            .iter()
            .enumerate()
            .map(|(index, &class)| {
                pft_parameters(
                    document,
                    class,
                    campbell,
                    pc,
                    fraction[index],
                    top[index],
                    bottom[index],
                    interface_depth_m,
                    physics.pft_overrides.get(index),
                )
                .map(|parameters| PftParameters {
                    crown_m: crown
                        .as_ref()
                        .map(|[ncd, ncw, bcw]| [ncd[index], ncw[index], bcw[index]]),
                    ..parameters
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let vector = |name: &str| -> Result<Vec<f64>> {
            let values = time.floats(name)?.to_vec();
            ensure!(
                values.len() == pfts,
                "{name} holds {} values for {pfts} PFTs",
                values.len()
            );
            Ok(values)
        };
        let [tleaf, ldew, ldew_rain, ldew_snow, fwet_snow, sigf, tlai, lai, tsai, sai, thermk, fshade, extkb, extkd, tref, qref, rst, z0m] =
            PFT_VECTORS.map(&vector);
        let (tleaf, ldew, ldew_rain, ldew_snow, fwet_snow, sigf) =
            (tleaf?, ldew?, ldew_rain?, ldew_snow?, fwet_snow?, sigf?);
        let (tlai, lai, tsai, sai, thermk, fshade) = (tlai?, lai?, tsai?, sai?, thermk?, fshade?);
        let (extkb, extkd, tref, qref, rst, z0m) = (extkb?, extkd?, tref?, qref?, rst?, z0m?);
        let ssun = absorption_matrix(&time, "ssun_p", pfts)?;
        let ssha = absorption_matrix(&time, "ssha_p", pfts)?;
        let plant_hydraulics = physics.plant_hydraulics;
        let (vegwp, gs0sun, gs0sha) = if plant_hydraulics {
            let vegwp = time.floats("vegwp_p")?;
            ensure!(
                vegwp.len() == pfts * VEGETATION_SEGMENTS,
                "vegwp_p must hold {VEGETATION_SEGMENTS} nodes per PFT"
            );
            (vegwp.to_vec(), vector("gs0sun_p")?, vector("gs0sha_p")?)
        } else {
            (Vec::new(), vec![0.0; pfts], vec![0.0; pfts])
        };
        // `READ_PFTimeVariables`（`MOD_Vars_TimeVariables.F90:199-210`）：缺变量时 `lai_old_p`/
        // `o3uptake*_p` 取 0、`o3coef*_p` 取 1（`defval`）。
        let ozone = physics.ozone.is_some();
        let ozone_fields = if ozone {
            PFT_OZONE_RESTART_FIELDS
                .iter()
                .zip([0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0])
                .map(|(name, default)| {
                    if time.variable_dimensions(name).is_ok() {
                        vector(name)
                    } else {
                        Ok(vec![default; pfts])
                    }
                })
                .collect::<Result<Vec<_>>>()?
        } else {
            Vec::new()
        };
        let columns = (0..pfts)
            .map(|p| PftColumn {
                leaf: LeafTemperatureState {
                    leaf_temperature_k: tleaf[p],
                    canopy_water: colm_core::CanopyWater {
                        total_mm: ldew[p],
                        rain_mm: ldew_rain[p],
                        snow_mm: ldew_snow[p],
                    },
                    // netCDF 维序是 `(pft, vegnodes)`：节点在内层。
                    plant_hydraulics: plant_hydraulics.then(|| PlantHydraulicState {
                        vegetation_water_potential_mm: std::array::from_fn(|node| {
                            vegwp[p * VEGETATION_SEGMENTS + node]
                        }),
                    }),
                    ozone: ozone.then(|| {
                        crate::assembly::ozone_state_from_fields(std::array::from_fn(|field| {
                            ozone_fields[field][p]
                        }))
                    }),
                },
                wet_snow_fraction: fwet_snow[p],
                vegetation_free_fraction: sigf[p],
                temporal_leaf_area_index: tlai[p],
                temporal_stem_area_index: tsai[p],
                leaf_area_index: lai[p],
                stem_area_index: sai[p],
                sunlit_absorption: ssun[p],
                shaded_absorption: ssha[p],
                thermal_gap_fraction: thermk[p],
                shade_fraction: fshade[p],
                direct_extinction: extkb[p],
                diffuse_extinction: extkd[p],
                reference_temperature_k: tref[p],
                reference_humidity: qref[p],
                stomatal_resistance_s_m: rst[p],
                momentum_roughness_m: z0m[p],
                maximum_sunlit_leaf_conductance: gs0sun[p],
                maximum_shaded_leaf_conductance: gs0sha[p],
                sunlit_leaf_area_index: 0.0,
                shaded_leaf_area_index: 0.0,
                assimilation_mol_m2_s: 0.0,
                respiration_mol_m2_s: 0.0,
            })
            .collect();
        let mut initial = PftPatch::new(parameters, columns, interface_depth_m.len() - 1)?;
        initial.plant_community = pc;
        // `DEF_PC_CROP_SPLIT`（缺省 `.true.`）：PC 的作物 PFT 走一维冠层。
        initial.pc_crop_split = match document.get("DEF_PC_CROP_SPLIT") {
            Some(colm_namelist::Value::Bool(value)) => *value,
            Some(_) => anyhow::bail!("DEF_PC_CROP_SPLIT must be a logical value"),
            None => true,
        };
        Ok(Self {
            initial,
            plant_hydraulics,
            ozone,
            monthly: None,
            site_pfts,
            site_pft_count,
        })
    }

    /// 装上 `LAI_readin` 的 PFT 月度数据源（`landdata/srfdata.nc`）。
    pub fn with_monthly_leaf_area_index(
        mut self,
        path: impl AsRef<Path>,
        use_site_lai: bool,
        change_yearly: bool,
        land_cover_year: i32,
        start_year: i32,
        end_year: i32,
    ) -> Result<Self> {
        let data = colm_init::read_single_point_pft_data(path)?;
        ensure!(
            data.class.len() == self.site_pft_count
                && data.class[self.site_pfts.clone()]
                    .iter()
                    .zip(&self.initial.parameters)
                    .all(|(class, pft)| *class == pft.class),
            "srfdata.nc lists PFT classes {:?}, but the PFT restart has {:?} (site PFTs {:?} of {})",
            data.class,
            self.initial
                .parameters
                .iter()
                .map(|pft| pft.class)
                .collect::<Vec<_>>(),
            self.site_pfts,
            self.site_pft_count
        );
        self.monthly = Some(PftMonthlyLeafAreaIndex {
            source: PftLeafAreaSource::SinglePoint {
                vegetation: data.monthly,
                site_fraction: data.fraction,
                use_site_lai,
            },
            change_yearly,
            land_cover_year,
            start_year,
            end_year,
        });
        Ok(self)
    }

    /// 装上空间算例的 PFT 月度数据源（`landdata/LAI`，块 `block` 里第 `patch` 个 patch）。
    pub fn with_grid_monthly_leaf_area_index(
        mut self,
        landdata: &Path,
        block: &str,
        patch: usize,
        change_yearly: bool,
        land_cover_year: i32,
        (start_year, end_year): (i32, i32),
    ) -> Self {
        self.monthly = Some(PftMonthlyLeafAreaIndex {
            source: PftLeafAreaSource::Grid {
                directory: landdata.join("LAI"),
                block: block.to_owned(),
                patch,
            },
            change_yearly,
            land_cover_year,
            start_year,
            end_year,
        });
        self
    }

    /// 本 patch 在 PFT 重启里的区间。
    pub fn pft_range(&self) -> std::ops::Range<usize> {
        self.site_pfts.clone()
    }

    pub fn has_monthly_leaf_area_index(&self) -> bool {
        self.monthly.is_some()
    }

    /// 本月的 `tlai_p`/`tsai_p` 写进列，返回 patch 的 `tlai`/`tsai`。
    ///
    /// `tlai = sum(SITE_LAI_pfts_monthly(:,time,iyear)*SITE_pctpfts)` 是按站点顺序的
    /// 标量 FMA 链（`MOD_LAIReadin.F90:175-176` 的 GIMPLE），份额为 0 的项
    /// `FMA(x, 0, acc) = acc`，所以只对打包后的 PFT 求和与之逐位相同。
    ///
    /// `DEF_USE_LAIFEEDBACK`（`lai_feedback`）时 `tlai_p`/`tlai` 由 BGC 的 `CNVegStructUpdate`
    /// 负责，这里只换 `tsai_p`，返回的 patch `tlai` 为 `None`（`MOD_LAIReadin.F90:170-186`）。
    pub fn refresh_monthly_leaf_area_index(
        &self,
        time: colm_core::CalendarTime,
        patch: &mut PftPatch,
        lai_feedback: bool,
    ) -> Result<Option<(Option<f64>, f64)>> {
        let Some(monthly) = &self.monthly else {
            return Ok(None);
        };
        let (month, _) = colm_core::month_day(time)?;
        let year = if monthly.change_yearly {
            time.year
        } else {
            monthly.land_cover_year
        };
        let (vegetation, site_fraction, use_site_lai) = match &monthly.source {
            PftLeafAreaSource::SinglePoint {
                vegetation,
                site_fraction,
                use_site_lai,
            } => (vegetation, site_fraction, *use_site_lai),
            PftLeafAreaSource::Grid {
                directory,
                block,
                patch: index,
            } => {
                // 年份夹到 `[DEF_LAI_START_YEAR, DEF_LAI_END_YEAR]`；LAI 反馈时不读 `LAI_*`。
                let year = year.max(monthly.start_year).min(monthly.end_year);
                let read = |stem: &str| -> Result<Vec<f64>> {
                    let path = directory
                        .join(format!("{year:04}"))
                        .join(format!("{stem}{month:02}_{block}.nc"));
                    let file = RestartFile::open(&path)
                        .with_context(|| format!("cannot open {}", path.display()))?;
                    Ok(file.floats(stem)?.to_vec())
                };
                let at = |values: Vec<f64>, stem: &str| -> Result<f64> {
                    values
                        .get(*index)
                        .copied()
                        .with_context(|| format!("{stem} has no value for patch {index}"))
                };
                let range = self.site_pfts.clone();
                ensure!(
                    range.len() == patch.columns.len(),
                    "the patch PFT range does not match its PFT columns"
                );
                let tlai = if lai_feedback {
                    None
                } else {
                    Some(at(read("LAI_patches")?, "LAI_patches")?)
                };
                let tsai = at(read("SAI_patches")?, "SAI_patches")?;
                if !lai_feedback {
                    let lai = read("LAI_pfts")?;
                    ensure!(
                        range.end <= lai.len(),
                        "LAI_pfts is shorter than the PFT range"
                    );
                    for (column, &value) in patch.columns.iter_mut().zip(&lai[range.clone()]) {
                        column.temporal_leaf_area_index = value;
                    }
                }
                let sai = read("SAI_pfts")?;
                ensure!(
                    range.end <= sai.len(),
                    "SAI_pfts is shorter than the PFT range"
                );
                for (column, &value) in patch.columns.iter_mut().zip(&sai[range]) {
                    column.temporal_stem_area_index = value;
                }
                return Ok(Some((tlai, tsai)));
            }
        };
        let (lai, sai) = vegetation.for_year(
            year,
            month,
            use_site_lai,
            monthly.start_year,
            monthly.end_year,
        )?;
        ensure!(
            lai.len() == self.site_pft_count && sai.len() == self.site_pft_count,
            "the monthly PFT LAI does not match the site PFT count"
        );
        let range = self.site_pfts.clone();
        ensure!(
            range.len() == patch.columns.len(),
            "the patch PFT range does not match its PFT columns"
        );
        for (column, (lai, sai)) in patch
            .columns
            .iter_mut()
            .zip(lai[range.clone()].iter().zip(&sai[range.clone()]))
        {
            if !lai_feedback {
                column.temporal_leaf_area_index = *lai;
            }
            column.temporal_stem_area_index = *sai;
        }
        if range.len() == self.site_pft_count {
            return Ok(Some((
                (!lai_feedback).then(|| patch.sum(|column| column.temporal_leaf_area_index)),
                patch.sum(|column| column.temporal_stem_area_index),
            )));
        }
        // 多作物单点：每个 patch 只对自己的 PFT 区间求 `sum(SITE_LAI_pfts_monthly(ps:pe)*SITE_pctpfts(ps:pe))`。
        // 上游原来给每个 patch 赋同一个全站和，即各作物 LAI 之和（upstream-bugs 第 33 条，vendor 已修）。
        let site_sum = |values: &[f64]| {
            colm_core::pft_sum(
                values[range.clone()]
                    .iter()
                    .copied()
                    .zip(site_fraction[range.clone()].iter().copied()),
            )
        };
        Ok(Some((
            (!lai_feedback).then(|| site_sum(&lai)),
            site_sum(&sai),
        )))
    }

    /// PFT 时间重启里被主循环推进过的全部变量。
    pub fn overrides(&self, patch: &PftPatch) -> Vec<RestartOverride> {
        let columns = &patch.columns;
        let field = |name: &str, value: fn(&PftColumn) -> f64| {
            RestartOverride::new(name, columns.iter().map(value).collect())
        };
        let mut overrides = vec![
            field("tleaf_p", |c| c.leaf.leaf_temperature_k),
            field("ldew_p", |c| c.leaf.canopy_water.total_mm),
            field("ldew_rain_p", |c| c.leaf.canopy_water.rain_mm),
            field("ldew_snow_p", |c| c.leaf.canopy_water.snow_mm),
            field("fwet_snow_p", |c| c.wet_snow_fraction),
            field("sigf_p", |c| c.vegetation_free_fraction),
            field("tlai_p", |c| c.temporal_leaf_area_index),
            field("lai_p", |c| c.leaf_area_index),
            field("tsai_p", |c| c.temporal_stem_area_index),
            field("sai_p", |c| c.stem_area_index),
            RestartOverride::new(
                "ssun_p",
                columns
                    .iter()
                    .flat_map(|c| flatten_absorption(c.sunlit_absorption))
                    .collect(),
            ),
            RestartOverride::new(
                "ssha_p",
                columns
                    .iter()
                    .flat_map(|c| flatten_absorption(c.shaded_absorption))
                    .collect(),
            ),
            field("thermk_p", |c| c.thermal_gap_fraction),
            field("fshade_p", |c| c.shade_fraction),
            field("extkb_p", |c| c.direct_extinction),
            field("extkd_p", |c| c.diffuse_extinction),
            field("tref_p", |c| c.reference_temperature_k),
            field("qref_p", |c| c.reference_humidity),
            field("rst_p", |c| c.stomatal_resistance_s_m),
            field("z0m_p", |c| c.momentum_roughness_m),
        ];
        if self.plant_hydraulics {
            overrides.push(RestartOverride::new(
                "vegwp_p",
                columns
                    .iter()
                    .flat_map(|c| {
                        c.leaf
                            .plant_hydraulics
                            .map_or([colm_core::MISSING; VEGETATION_SEGMENTS], |state| {
                                state.vegetation_water_potential_mm
                            })
                    })
                    .collect(),
            ));
            overrides.push(field("gs0sun_p", |c| c.maximum_sunlit_leaf_conductance));
            overrides.push(field("gs0sha_p", |c| c.maximum_shaded_leaf_conductance));
        }
        if self.ozone {
            for (index, name) in PFT_OZONE_RESTART_FIELDS.iter().enumerate() {
                overrides.push(RestartOverride::new(
                    *name,
                    columns
                        .iter()
                        .map(|c| {
                            c.leaf.ozone.map_or(colm_core::MISSING, |state| {
                                crate::assembly::ozone_state_fields(&state)[index]
                            })
                        })
                        .collect(),
                ));
            }
        }
        overrides
    }
}

/// `MOD_Vars_PFTimeVariables` 的臭氧时间变量，次序同 [`crate::assembly::OZONE_RESTART_FIELDS`]。
const PFT_OZONE_RESTART_FIELDS: [&str; 7] = [
    "lai_old_p",
    "o3uptakesun_p",
    "o3uptakesha_p",
    "o3coefv_sun_p",
    "o3coefv_sha_p",
    "o3coefg_sun_p",
    "o3coefg_sha_p",
];

const PFT_VECTORS: [&str; 18] = [
    "tleaf_p",
    "ldew_p",
    "ldew_rain_p",
    "ldew_snow_p",
    "fwet_snow_p",
    "sigf_p",
    "tlai_p",
    "lai_p",
    "tsai_p",
    "sai_p",
    "thermk_p",
    "fshade_p",
    "extkb_p",
    "extkd_p",
    "tref_p",
    "qref_p",
    "rst_p",
    "z0m_p",
];

/// `ssun_p(band, rtyp, pft)` 在 netCDF 里是 `(pft, rtyp, band)`：波段在最内层。
fn absorption_matrix(time: &RestartFile, name: &str, pfts: usize) -> Result<Vec<[[f64; 2]; 2]>> {
    let values = time.floats(name)?;
    ensure!(
        values.len() == pfts * 4,
        "{name} must hold 2 bands x 2 radiation types per PFT"
    );
    Ok((0..pfts)
        .map(|p| {
            std::array::from_fn(|band| std::array::from_fn(|kind| values[p * 4 + kind * 2 + band]))
        })
        .collect())
}

fn flatten_absorption(matrix: [[f64; 2]; 2]) -> [f64; 4] {
    [matrix[0][0], matrix[1][0], matrix[0][1], matrix[1][1]]
}

/// 按 `(地类, Campbell, PC)` 存的 PFT 参数。
type PftParameterTable = std::collections::HashMap<(i32, bool, bool), PftParameters>;

thread_local! {
    /// 装配期间的 PFT 参数缓存（[`PftParameterCache`] 打开时才用）：同一次运行里参数只取决于地类与
    /// Campbell/PC 两个开关，空间算例上万个 patch 不必每个都重新查 namelist 与默认表。
    static PFT_PARAMETER_CACHE: std::cell::RefCell<Option<PftParameterTable>> =
        const { std::cell::RefCell::new(None) };
}

/// 在本线程上打开 PFT 参数缓存，守卫析构时关闭并清空。只在同一份算例 namelist 与同一套土层下的
/// 装配循环里用。
pub struct PftParameterCache(());

impl PftParameterCache {
    pub fn enable() -> Self {
        PFT_PARAMETER_CACHE.with(|cache| *cache.borrow_mut() = Some(Default::default()));
        Self(())
    }
}

impl Drop for PftParameterCache {
    fn drop(&mut self) {
        PFT_PARAMETER_CACHE.with(|cache| *cache.borrow_mut() = None);
    }
}

/// 一个 PFT 的 `MOD_Const_PFT` 参数（默认值 + `DEF_PFT_*` 覆盖）。`hybrid` 是混合模型给这个 PFT
/// 的覆盖：有就不走按类别的缓存，派生量（光学、根系分布、换算）照原路径从覆盖值算。
#[allow(clippy::too_many_arguments)]
fn pft_parameters(
    document: &Document,
    class: i32,
    campbell: bool,
    pc: bool,
    fraction: f64,
    canopy_top_m: f64,
    canopy_bottom_m: f64,
    interface_depth_m: &[f64],
    hybrid: Option<&std::collections::BTreeMap<String, f64>>,
) -> Result<PftParameters> {
    if let Some(overrides) = hybrid.filter(|overrides| !overrides.is_empty()) {
        let parameters = pft_parameters_with(
            document,
            class,
            campbell,
            pc,
            interface_depth_m,
            Some(overrides),
        )?;
        return Ok(PftParameters {
            fraction,
            canopy_top_m,
            canopy_bottom_m,
            ..parameters
        });
    }
    let key = (class, campbell, pc);
    let cached = PFT_PARAMETER_CACHE.with(|cache| {
        cache
            .borrow()
            .as_ref()
            .and_then(|map| map.get(&key).cloned())
    });
    let parameters = match cached {
        Some(parameters) => parameters,
        None => {
            let parameters =
                pft_parameters_uncached(document, class, campbell, pc, interface_depth_m)?;
            PFT_PARAMETER_CACHE.with(|cache| {
                if let Some(map) = cache.borrow_mut().as_mut() {
                    map.insert(key, parameters.clone());
                }
            });
            parameters
        }
    };
    Ok(PftParameters {
        fraction,
        canopy_top_m,
        canopy_bottom_m,
        ..parameters
    })
}

fn pft_parameters_uncached(
    document: &Document,
    class: i32,
    campbell: bool,
    pc: bool,
    interface_depth_m: &[f64],
) -> Result<PftParameters> {
    pft_parameters_with(document, class, campbell, pc, interface_depth_m, None)
}

fn pft_parameters_with(
    document: &Document,
    class: i32,
    campbell: bool,
    pc: bool,
    interface_depth_m: &[f64],
    hybrid: Option<&std::collections::BTreeMap<String, f64>>,
) -> Result<PftParameters> {
    let (fraction, canopy_top_m, canopy_bottom_m) = (0.0, 0.0, 0.0);
    // PC 用另一组叶片光学（`rhol_*_p_pc`/`taul_*_p_pc`，`MOD_Const_PFT.F90:1833-1843`）。
    // 混合模型的覆盖优先，其余照 namelist 的 `DEF_PFT_*(class)` 与默认表。
    let value = |name: &str| match hybrid.and_then(|overrides| overrides.get(name)) {
        Some(&value) => Ok(value),
        None => colm_init::pft_parameter(document, name, class, campbell, pc),
    };
    Ok(PftParameters {
        class,
        fraction,
        canopy_top_m,
        canopy_bottom_m,
        optics: LeafOptics {
            chil: value("DEF_PFT_CHIL")?,
            reflectance: [
                [value("DEF_PFT_RHOL_VIS")?, value("DEF_PFT_RHOS_VIS")?],
                [value("DEF_PFT_RHOL_NIR")?, value("DEF_PFT_RHOS_NIR")?],
            ],
            transmittance: [
                [value("DEF_PFT_TAUL_VIS")?, value("DEF_PFT_TAUS_VIS")?],
                [value("DEF_PFT_TAUL_NIR")?, value("DEF_PFT_TAUS_NIR")?],
            ],
        },
        biochemistry: LeafBiochemistry {
            quantum_efficiency: value("DEF_PFT_EFFCON")?,
            // 表值与覆盖值都是 µmol m-2 s-1，`Init_PFT_Const` 统一乘 `1.e-6`
            // （`MOD_Const_PFT.F90:1779/1812` 与 `pft_override_fields.inc` 的 scale）。
            maximum_carboxylation_25c_mol_m2_s: value("DEF_PFT_VMAX25")? * 1.0e-6,
            c3c4: value("DEF_PFT_C3C4")? as i32,
            respiration_fraction_override: None,
            low_temperature_slope: value("DEF_PFT_SLTI")?,
            low_temperature_half_k: value("DEF_PFT_HLTI")?,
            high_temperature_slope: value("DEF_PFT_SHTI")?,
            high_temperature_half_k: value("DEF_PFT_HHTI")?,
            respiration_temperature_slope: value("DEF_PFT_TRDA")?,
            respiration_temperature_half_k: value("DEF_PFT_TRDM")?,
            optimum_temperature_k: value("DEF_PFT_TROP")?,
            medlyn_g1: value("DEF_PFT_G1")?,
            medlyn_g0: value("DEF_PFT_G0")?,
            ball_berry_slope: value("DEF_PFT_GRADM")?,
            ball_berry_intercept: value("DEF_PFT_BINTER")?,
        },
        inverse_sqrt_leaf_dimension_m_neg_half: value("DEF_PFT_SQRTDI")?,
        wue_lambda: value("DEF_PFT_LAMBDA")?,
        // `ROOTFR_SCHEME` 在 `MOD_Const_PFT` 里是私有常量 1。
        root_fraction: colm_core::schenk_jackson_root_fraction(
            value("DEF_PFT_D50")?,
            value("DEF_PFT_BETA")?,
            interface_depth_m,
        ),
        plant_hydraulic_traits: PlantHydraulicTraits {
            maximum_sunlit_leaf_conductance: value("DEF_PFT_KMAX_SUN")?,
            maximum_shaded_leaf_conductance: value("DEF_PFT_KMAX_SHA")?,
            maximum_xylem_conductance: value("DEF_PFT_KMAX_XYL")?,
            maximum_root_conductance: value("DEF_PFT_KMAX_ROOT")?,
            sunlit_leaf_psi50_mm: value("DEF_PFT_PSI50_SUN")?,
            shaded_leaf_psi50_mm: value("DEF_PFT_PSI50_SHA")?,
            xylem_psi50_mm: value("DEF_PFT_PSI50_XYL")?,
            root_psi50_mm: value("DEF_PFT_PSI50_ROOT")?,
            vulnerability_shape: value("DEF_PFT_CK")?,
        },
        // `canlay_p`（`MOD_Const_PFT`）：乔木（1..=8）在第 2 层，其余在第 1 层，裸地 0。
        canopy_layer: match class {
            0 => 0,
            1..=8 => 2,
            _ => 1,
        },
        // 臭氧胁迫（`MOD_Ozone.F90`）的类别参数：`isevg` 写死，`leaf_long` 可被 `DEF_PFT_LEAF_LONG` 覆盖。
        evergreen: colm_case::pft::fixed_value(
            "isevg",
            u8::try_from(class).context("pftclass must be nonnegative")?,
        )? != 0.0,
        leaf_longevity_years: value("DEF_PFT_LEAF_LONG")?,
        crown_m: None,
    })
}

#[cfg(test)]
#[path = "pft_tests.rs"]
mod pft_tests;

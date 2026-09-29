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
    monthly: Option<PftMonthlyLeafAreaIndex>,
    /// 本 patch 在全站 PFT 里的区间（多作物单点每个 patch 一个 PFT；单 patch 时是全部）。
    site_pfts: std::ops::Range<usize>,
    /// 全站 PFT 数。
    site_pft_count: usize,
}

/// `LAI_readin` 的 PFT 段（`MOD_LAIReadin.F90:166-185`，单点）。
#[derive(Debug, Clone)]
struct PftMonthlyLeafAreaIndex {
    vegetation: colm_init::SinglePointPftMonthlyVegetation,
    /// 全站 PFT 的站点份额 `SITE_pctpfts`（作物站点恒为 1）。
    site_fraction: Vec<f64>,
    use_site_lai: bool,
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
pub fn patch_pft_range(patches: usize, pfts: usize, patch: usize) -> Result<std::ops::Range<usize>> {
    ensure!(patch < patches, "patch {patch} is outside the {patches} patches");
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
    ) -> Result<Self> {
        let whole = RestartFile::open(constant_path)?;
        let (patches, site_pft_count) = patch_and_pft_counts(&whole)?;
        let site_pfts = patch_pft_range(patches, site_pft_count, patch)?;
        let constant = open_patch(constant_path, patch, patches, site_pft_count)?;
        let time = open_patch(time_path, patch, patches, site_pft_count)?;
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
                )
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
        Ok(Self {
            initial,
            plant_hydraulics,
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
            vegetation: data.monthly,
            site_fraction: data.fraction,
            use_site_lai,
            change_yearly,
            land_cover_year,
            start_year,
            end_year,
        });
        Ok(self)
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
        let (lai, sai) = monthly.vegetation.for_year(
            year,
            month,
            monthly.use_site_lai,
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
        // 多作物单点：上游给**每个** patch 赋同一个全站和 `sum(SITE_LAI_pfts_monthly*SITE_pctpfts)`
        // （`MOD_LAIReadin.F90:174-175`），作物站点 `SITE_pctpfts = 1`，于是是各作物 LAI 之和
        // （upstream-bugs 第 33 条）。照写。
        let site_sum = |values: &[f64]| {
            colm_core::pft_sum(values.iter().copied().zip(monthly.site_fraction.iter().copied()))
        };
        Ok(Some(((!lai_feedback).then(|| site_sum(&lai)), site_sum(&sai))))
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
        overrides
    }
}

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

/// 一个 PFT 的 `MOD_Const_PFT` 参数（默认值 + `DEF_PFT_*` 覆盖）。
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
) -> Result<PftParameters> {
    // PC 用另一组叶片光学（`rhol_*_p_pc`/`taul_*_p_pc`，`MOD_Const_PFT.F90:1833-1843`）。
    let value = |name: &str| colm_init::pft_parameter(document, name, class, campbell, pc);
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
    })
}

#[cfg(test)]
#[path = "pft_tests.rs"]
mod pft_tests;

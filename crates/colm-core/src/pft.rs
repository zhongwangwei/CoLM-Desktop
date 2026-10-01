//! `DEF_USE_PFT` 的子网格：一个土壤 patch 里并排的若干植物功能型。
//!
//! 上游在五处把 LCT 的单冠层换成逐 PFT 计算再按 `pftfrac` 聚合：
//! 截留（`LEAF_interception_pftwrap`）、短波吸收（`netsolar` 的 PFT 段）、
//! 冠层能量（`THERMAL` 的 PFT 循环）、雪盖（`snowfraction_pftwrap`）与反照率
//! （`twostream_wrap`）。地面温度、土壤水与雪都仍是 patch 级的 —— 聚合之后
//! 的量（`fseng`/`fevpg`/`cgrnd`/`rootr`…）直接交给 LCT 那条路径，所以这里只放
//! 逐 PFT 的那几段，外加聚合。
//!
//! 聚合的数值形状：`main/` 的 GIMPLE 里每一个 `sum(x_p(ps:pe)*pftfrac(ps:pe))`
//! 都是从 0 起、按 PFT 顺序的 FMA 链（`FMA(x_p, pftfrac, acc)`），pftwrap 里手写的
//! `tmp = tmp + x*pftfrac(i)` 也一样。见 [`pft_sum`]。

use anyhow::{ensure, Result};

use crate::radiation::{broadband_radiation_from_ground_using, TwoStreamKind};
use crate::{
    CanopyInterceptionFluxes, CanopyInterceptionInput, CanopyWater, ColdStartGroundAlbedo,
    ColdStartRadiation, LeafBiochemistry, LeafOptics, LeafTemperatureState, PlantHydraulicTraits,
    MISSING,
};

/// `vegwp_p(:,i) = -2.5e4`：无冠层 PFT 的植物水势（`MOD_Thermal.F90:1028-1030`）。
pub const BARE_PFT_WATER_POTENTIAL_MM: f64 = -2.5e4;

/// 一个 PFT 的静态参数（`MOD_Const_PFT` 按 `pftclass` 查表，再按 `DEF_PFT_*` 覆盖）。
#[derive(Debug, Clone, PartialEq)]
pub struct PftParameters {
    /// `pftclass`：0 是裸地，1..=15 是自然 PFT。
    pub class: i32,
    /// `pftfrac`
    pub fraction: f64,
    /// `htop_p`/`hbot_p`：常数重启里的值（单点时树木按观测高度折算过）。
    pub canopy_top_m: f64,
    pub canopy_bottom_m: f64,
    /// `chil_p`/`rho_p`/`tau_p`
    pub optics: LeafOptics,
    pub biochemistry: LeafBiochemistry,
    /// `sqrtdi_p`
    pub inverse_sqrt_leaf_dimension_m_neg_half: f64,
    /// `lambda_p`
    pub wue_lambda: f64,
    /// `rootfr_p(:,p)`
    pub root_fraction: Vec<f64>,
    /// `kmax_*_p`/`psi50_*_p`/`ck_p`：只在 `DEF_USE_PLANTHYDRAULICS` 下读。
    pub plant_hydraulic_traits: PlantHydraulicTraits,
    /// `canlay_p`（PC 的三层冠层分层：0 裸地、1 草本与灌木及作物、2 乔木）；PFT 不读。
    pub canopy_layer: usize,
}

impl PftParameters {
    /// `lai_p+sai_p > 1e-6` 的判据被五处共用，放在一起免得有一处写成 `>=`。
    fn has_canopy(leaf_area_index: f64, stem_area_index: f64) -> bool {
        leaf_area_index + stem_area_index > 1.0e-6
    }
}

/// 一个 PFT 的时间变量（`MOD_Vars_PFTimeVariables`，全部进 PFT 时间重启）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PftColumn {
    /// `tleaf_p`、`ldew_p`/`ldew_rain_p`/`ldew_snow_p` 与 PHS 的 `vegwp_p`。
    pub leaf: LeafTemperatureState,
    /// `fwet_snow_p`
    pub wet_snow_fraction: f64,
    /// `sigf_p`
    pub vegetation_free_fraction: f64,
    /// `tlai_p`/`tsai_p`：`LAI_readin` 每月覆盖。
    pub temporal_leaf_area_index: f64,
    pub temporal_stem_area_index: f64,
    /// `lai_p`/`sai_p`：步末按雪盖折算后的有效值。
    pub leaf_area_index: f64,
    pub stem_area_index: f64,
    /// `ssun_p`/`ssha_p`：`[band][rtyp]`。
    pub sunlit_absorption: [[f64; 2]; 2],
    pub shaded_absorption: [[f64; 2]; 2],
    /// `thermk_p`/`fshade_p`/`extkb_p`/`extkd_p`
    pub thermal_gap_fraction: f64,
    pub shade_fraction: f64,
    pub direct_extinction: f64,
    pub diffuse_extinction: f64,
    /// `tref_p`/`qref_p`/`rst_p`/`z0m_p`
    pub reference_temperature_k: f64,
    pub reference_humidity: f64,
    pub stomatal_resistance_s_m: f64,
    pub momentum_roughness_m: f64,
    /// `gs0sun_p`/`gs0sha_p`：PHS 的最大叶导度；PHS 关掉时上游不赋值。
    pub maximum_sunlit_leaf_conductance: f64,
    pub maximum_shaded_leaf_conductance: f64,
    /// `laisun_p`/`laisha_p`/`assim_p`/`respc_p`：本步冠层能量求解的结果。上游是 module 数组，
    /// `bgc_driver` 在 `CoLMMAIN` 之后读它们（维持呼吸、光合产物）；不进重启。
    pub sunlit_leaf_area_index: f64,
    pub shaded_leaf_area_index: f64,
    pub assimilation_mol_m2_s: f64,
    pub respiration_mol_m2_s: f64,
}

/// 一个 PFT patch 的全部子网格状态。
#[derive(Debug, Clone, PartialEq)]
pub struct PftPatch {
    pub parameters: Vec<PftParameters>,
    pub columns: Vec<PftColumn>,
    /// 土壤层数（`rootr`/`rootflux` 的长度）。
    pub layers: usize,
    /// `DEF_USE_PC`：冠层能量走 `LeafTemperaturePC`、反照率走 `ThreeDCanopy_wrap`。
    pub plant_community: bool,
}

impl PftPatch {
    pub fn new(
        parameters: Vec<PftParameters>,
        columns: Vec<PftColumn>,
        layers: usize,
    ) -> Result<Self> {
        ensure!(
            !parameters.is_empty() && parameters.len() == columns.len(),
            "a PFT patch needs one column per PFT and at least one PFT"
        );
        ensure!(
            parameters
                .iter()
                .all(|pft| pft.root_fraction.len() == layers),
            "every PFT root fraction must cover the {layers} soil layers"
        );
        Ok(Self {
            parameters,
            columns,
            layers,
            plant_community: false,
        })
    }

    pub fn fractions(&self) -> impl Iterator<Item = f64> + '_ {
        self.parameters.iter().map(|pft| pft.fraction)
    }

    /// `sum(x_p(ps:pe)*pftfrac(ps:pe))`，见 [`pft_sum`]。
    pub fn sum(&self, value: impl Fn(&PftColumn) -> f64) -> f64 {
        pft_sum(self.columns.iter().map(value).zip(self.fractions()))
    }
}

/// `sum(x_p*pftfrac)`：从 0 起、按 PFT 顺序的 FMA 链。
///
/// gfortran 把这个内在函数展开成 `val = .FMA(x_p(i), pftfrac(i), val)`，初值 0
/// —— 不是先求乘积数组再求和。第一项因此是 `x*frac` 的一次舍入（`FMA(x,f,0)`）。
pub fn pft_sum(terms: impl IntoIterator<Item = (f64, f64)>) -> f64 {
    terms
        .into_iter()
        .fold(0.0, |sum, (value, fraction)| value.mul_add(fraction, sum))
}

/// `LEAF_interception_pftwrap`（`MOD_LeafInterception.F90:629-727`）。
///
/// 逐 PFT 用自己的 `chil_p`/`lai_p`/`sai_p`/`tleaf_p` 跑一次 CoLM2014 截留，
/// 地面降水按 `pftfrac` 聚合，patch 的冠层水量取三个 `ldew*_p` 的聚合。
/// 返回 patch 级通量与逐 PFT 通量（叶温要 `qintr_rain_p`/`qintr_snow_p`）。
pub(crate) fn intercept_pfts(
    template: CanopyInterceptionInput,
    patch: &mut PftPatch,
    patch_water: &mut CanopyWater,
) -> Result<(CanopyInterceptionFluxes, Vec<CanopyInterceptionFluxes>)> {
    ensure!(
        template.colm2024.is_none(),
        "DEF_Interception_scheme = 8 with DEF_USE_PFT needs the per-PFT crown sizes \
         (ncd_p/ncw_p/bcw_p), which the Rust PFT path does not assemble yet"
    );
    let mut fluxes = Vec::with_capacity(patch.columns.len());
    for (parameters, column) in patch.parameters.iter().zip(patch.columns.iter_mut()) {
        fluxes.push(crate::intercept_canopy(
            CanopyInterceptionInput {
                leaf_angle_distribution: parameters.optics.chil,
                leaf_area_index: column.leaf_area_index,
                stem_area_index: column.stem_area_index,
                leaf_temperature_k: column.leaf.leaf_temperature_k,
                ..template
            },
            &mut column.leaf.canopy_water,
        )?);
    }
    // `:699-718`：地面雨雪与 `qintr*` 都是 `FMA(x_i, pftfrac(i), tmp)` 的链。
    let sum = |select: fn(&CanopyInterceptionFluxes) -> f64| {
        pft_sum(fluxes.iter().map(select).zip(patch.fractions()))
    };
    let aggregate = CanopyInterceptionFluxes {
        ground_rain_kg_m2_s: sum(|f| f.ground_rain_kg_m2_s),
        ground_snow_kg_m2_s: sum(|f| f.ground_snow_kg_m2_s),
        retained_kg_m2_s: sum(|f| f.retained_kg_m2_s),
        retained_rain_kg_m2_s: sum(|f| f.retained_rain_kg_m2_s),
        retained_snow_kg_m2_s: sum(|f| f.retained_snow_kg_m2_s),
        released_rain_kg_m2_s: sum(|f| f.released_rain_kg_m2_s),
        released_snow_kg_m2_s: sum(|f| f.released_snow_kg_m2_s),
    };
    *patch_water = CanopyWater {
        total_mm: patch.sum(|column| column.leaf.canopy_water.total_mm),
        rain_mm: patch.sum(|column| column.leaf.canopy_water.rain_mm),
        snow_mm: patch.sum(|column| column.leaf.canopy_water.snow_mm),
    };
    Ok((aggregate, fluxes))
}

/// `netsolar` 的 PFT 前半段（`MOD_NetSolar.F90:145-170`）。
///
/// 无冠层 PFT 的 `ssun_p`/`ssha_p` 清零（**持久**：这是 module 变量，会进重启），
/// 再把 patch 的 `ssun`/`ssha` 换成聚合值 —— 之后的 patch 级 `sabvsun`/`sabvsha`
/// 就是按它算的。
pub(crate) fn aggregate_pft_absorption(patch: &mut PftPatch, radiation: &mut ColdStartRadiation) {
    for column in &mut patch.columns {
        if !PftParameters::has_canopy(column.leaf_area_index, column.stem_area_index) {
            column.sunlit_absorption = [[0.0; 2]; 2];
            column.shaded_absorption = [[0.0; 2]; 2];
        }
    }
    radiation.sunlit_absorption = std::array::from_fn(|band| {
        std::array::from_fn(|kind| patch.sum(|column| column.sunlit_absorption[band][kind]))
    });
    radiation.shaded_absorption = std::array::from_fn(|band| {
        std::array::from_fn(|kind| patch.sum(|column| column.shaded_absorption[band][kind]))
    });
}

/// 一个 PFT 的冠层短波（`parsun_p`/`parsha_p`/`sabvsun_p`/`sabvsha_p`）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PftShortwave {
    pub par_sunlit_w_m2: f64,
    pub par_shaded_w_m2: f64,
    pub sunlit_absorbed_w_m2: f64,
    pub shaded_absorbed_w_m2: f64,
}

/// `netsolar` 的 PFT 后半段（`:188-196`）：与 patch 级同一组式子，只是系数换成
/// `ssun_p`/`ssha_p`；无太阳或非陆面时保持入口处的 0。
pub(crate) fn pft_shortwave(
    patch: &PftPatch,
    forcing: crate::ShortwaveForcing,
    patch_type: i32,
) -> Vec<PftShortwave> {
    patch
        .columns
        .iter()
        .map(|column| {
            if forcing.total() > 0.0 && patch_type < 4 {
                PftShortwave {
                    par_sunlit_w_m2: crate::net_solar::visible_absorption(
                        forcing,
                        column.sunlit_absorption,
                    ),
                    par_shaded_w_m2: crate::net_solar::visible_absorption(
                        forcing,
                        column.shaded_absorption,
                    ),
                    sunlit_absorbed_w_m2: crate::net_solar::absorption(
                        forcing,
                        column.sunlit_absorption,
                    ),
                    shaded_absorbed_w_m2: crate::net_solar::absorption(
                        forcing,
                        column.shaded_absorption,
                    ),
                }
            } else {
                PftShortwave::default()
            }
        })
        .collect()
}

/// `snowfraction_pftwrap` 的结果（`MOD_SnowFraction.F90:80-161`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PftSnowFraction {
    /// `wt`：`sum(wt_i*pftfrac)`。
    pub vegetation_snow_fraction: f64,
    /// `sigf`：`sum(sigf_p*pftfrac)`。
    pub vegetation_free_fraction: f64,
    /// `fsno`：patch 级，与 PFT 无关。
    pub ground_snow_fraction: f64,
}

/// `snowfraction_pftwrap`：逐 PFT 的 `sigf_p` 写回列，返回 patch 聚合。
pub fn pft_snow_fraction(
    patch: &mut PftPatch,
    soil_roughness_m: f64,
    snow_water_equivalent_mm: f64,
    snow_depth_m: f64,
    cover_exponent: f64,
    vegetation_snow: bool,
) -> Result<PftSnowFraction> {
    let mut vegetation_snow_fraction = 0.0;
    let mut ground_snow_fraction = 0.0;
    for (parameters, column) in patch.parameters.iter().zip(patch.columns.iter_mut()) {
        // 同一个 `snowfraction` 式子，只是粗糙度换成 `z0m_p`；`fsno` 与 PFT 无关，
        // 每次都是同一个值。
        let fraction = crate::snow::snow_fraction(
            column.temporal_leaf_area_index,
            column.temporal_stem_area_index,
            column.momentum_roughness_m,
            soil_roughness_m,
            snow_water_equivalent_mm,
            snow_depth_m,
            cover_exponent,
        )?;
        ground_snow_fraction = fraction.ground_snow_fraction;
        let mut buried = fraction.vegetation_snow_fraction;
        column.vegetation_free_fraction = fraction.vegetation_free_fraction;
        // `:139-146`：`DEF_VEG_SNOW` 下树木（1..=8）按冠层上下界算埋没比例。
        if vegetation_snow
            && PftParameters::has_canopy(
                column.temporal_leaf_area_index,
                column.temporal_stem_area_index,
            )
            && (1..=8).contains(&parameters.class)
        {
            buried = ((snow_depth_m - parameters.canopy_bottom_m).max(0.0)
                / (parameters.canopy_top_m - parameters.canopy_bottom_m))
                .min(1.0);
            column.vegetation_free_fraction = 1.0 - buried;
        }
        // `:148`：`wt_tmp = FMA(wt, pftfrac(i), wt_tmp)`。
        vegetation_snow_fraction = buried.mul_add(parameters.fraction, vegetation_snow_fraction);
    }
    Ok(PftSnowFraction {
        vegetation_snow_fraction,
        vegetation_free_fraction: patch.sum(|column| column.vegetation_free_fraction),
        ground_snow_fraction,
    })
}

/// `twostream_wrap` 的聚合（`MOD_Albedo.F90:1242-1266`），外加 `albland` 随后按聚合
/// 透射率重算的 `ssoi`/`ssno`。
///
/// `ground` 为 `None` 时是高光谱调用方：它们自己保留逐波长的地面吸收。
/// 返回值的 `thermal_gap_fraction` 是**冷启动**的约定（叶面积足够时为 `spval`），
/// 运行期调用方要换成上一步的 patch 值。
pub fn aggregate_pft_radiation(
    states: &[ColdStartRadiation],
    fraction: &[f64],
    leaf_stem_area: f64,
    ground: Option<&ColdStartGroundAlbedo>,
) -> Result<ColdStartRadiation> {
    ensure!(
        !states.is_empty() && states.len() == fraction.len(),
        "PFT radiation states and fractions must be nonempty and have matching lengths"
    );
    let aggregate = |select: fn(&ColdStartRadiation) -> [[f64; 2]; 2]| {
        std::array::from_fn(|band| {
            std::array::from_fn(|radiation_type| {
                states
                    .iter()
                    .zip(fraction)
                    .map(|(state, fraction)| select(state)[band][radiation_type] * fraction)
                    .sum()
            })
        })
    };
    // Original twostream_wrap uses stored-order FMA for its broadband sums.
    let absorption = |select: fn(&ColdStartRadiation) -> [[f64; 2]; 2]| {
        std::array::from_fn(|band| {
            std::array::from_fn(|radiation_type| {
                pft_sum(
                    states
                        .iter()
                        .map(|state| select(state)[band][radiation_type])
                        .zip(fraction.iter().copied()),
                )
            })
        })
    };
    let transmission =
        states
            .iter()
            .zip(fraction)
            .try_fold([[0.0; 3]; 2], |mut sum, (state, &weight)| {
                let transmission = state.transmission?;
                for band in 0..2 {
                    for beam in 0..3 {
                        sum[band][beam] = transmission[band][beam].mul_add(weight, sum[band][beam]);
                    }
                }
                Some(sum)
            });
    // albland derives ssoi/ssno only after twostream_wrap has summed tran.
    // Spectral callers retain their wavelength-resolved absorption instead.
    let (soil_absorption, snow_absorption) = if let Some(ground) = ground {
        ground.absorption(transmission.ok_or_else(|| {
            anyhow::anyhow!(
                "broadband PFT ground absorption requires canopy transmission for every PFT"
            )
        })?)
    } else {
        (
            aggregate(|state| state.soil_absorption),
            aggregate(|state| state.snow_absorption),
        )
    };
    Ok(ColdStartRadiation {
        albedo: if ground.is_some() {
            absorption(|state| state.albedo)
        } else {
            aggregate(|state| state.albedo)
        },
        transmission,
        sunlit_absorption: absorption(|state| state.sunlit_absorption),
        shaded_absorption: absorption(|state| state.shaded_absorption),
        soil_absorption,
        snow_absorption,
        snow_age: states[0].snow_age,
        // `albland` leaves this common field untouched for leafy PFT patches;
        // only the PFT-vector thermal gap is a live initial state.
        thermal_gap_fraction: if leaf_stem_area <= 1.0e-6 {
            1.0
        } else {
            MISSING
        },
        direct_extinction: 1.0,
        diffuse_extinction: 0.718,
    })
}

/// `albland` 在 PFT patch 上的冠层部分：逐 PFT `twostream_mod`（无冠层时取地面值），
/// 再由 [`aggregate_pft_radiation`] 聚合成 patch 的 `alb`/`ssun`/`ssha`/`ssoi`/`ssno`。
///
/// `radiation` 进来时是**上一步**的 patch 光学（只用它的 `thermk`），出去时是本步的。
pub(crate) fn pft_canopy_radiation(
    patch: &mut PftPatch,
    ground: ColdStartGroundAlbedo,
    cosine_zenith: f64,
    patch_leaf_stem_area: f64,
    vegetation_snow: bool,
    radiation: &mut ColdStartRadiation,
) -> Result<()> {
    if patch.plant_community {
        return pc_canopy_radiation(
            patch,
            ground,
            cosine_zenith,
            patch_leaf_stem_area,
            vegetation_snow,
            radiation,
        );
    }
    let mut states = Vec::with_capacity(patch.columns.len());
    for (parameters, column) in patch.parameters.iter().zip(patch.columns.iter_mut()) {
        let state = broadband_radiation_from_ground_using(
            0,
            ground,
            parameters.optics,
            column.leaf_area_index,
            column.stem_area_index,
            column.wet_snow_fraction,
            cosine_zenith,
            true,
            vegetation_snow,
            TwoStreamKind::Pft,
            // 无冠层时 `albland` 入口已把它置 1（`:254`），`broadband_…` 自己会处理。
            column.thermal_gap_fraction,
        )?;
        column.sunlit_absorption = state.sunlit_absorption;
        column.shaded_absorption = state.shaded_absorption;
        column.thermal_gap_fraction = state.thermal_gap_fraction;
        column.direct_extinction = state.direct_extinction;
        column.diffuse_extinction = state.diffuse_extinction;
        states.push(state);
    }
    let fractions = patch.fractions().collect::<Vec<_>>();
    let previous_thermal_gap_fraction = radiation.thermal_gap_fraction;
    *radiation = aggregate_pft_radiation(&states, &fractions, patch_leaf_stem_area, Some(&ground))?;
    // patch 的 `thermk` 只在 `lai+sai <= 1e-6` 时被 `albland` 置 1，其余时候原样保留。
    if patch_leaf_stem_area > 1.0e-6 {
        radiation.thermal_gap_fraction = previous_thermal_gap_fraction;
    }
    Ok(())
}

/// PC 的 `albland`（`MOD_Albedo.F90:432-440`）：`ThreeDCanopy_wrap` 解自然 PFT，随后照旧调
/// `twostream_wrap` —— 自然 PFT 在里面 `CYCLE`、`albv_p`/`tran_p` 取三维结果，再按 `pftfrac`
/// 做一遍 FMA 聚合，所以 patch 的 `alb`/`tran` 与三维原值差在这一步的舍入上。
fn pc_canopy_radiation(
    patch: &mut PftPatch,
    ground: ColdStartGroundAlbedo,
    cosine_zenith: f64,
    patch_leaf_stem_area: f64,
    vegetation_snow: bool,
    radiation: &mut ColdStartRadiation,
) -> Result<()> {
    ensure!(
        patch.parameters.iter().all(|pft| pft.class < 15),
        "PC patches with crop PFTs (DEF_PC_CROP_SPLIT) are not ported"
    );
    let inputs: Vec<crate::PcPftInput> = patch
        .parameters
        .iter()
        .zip(&patch.columns)
        .map(|(parameters, column)| crate::PcPftInput {
            canopy_layer: parameters.canopy_layer,
            fraction: parameters.fraction,
            canopy_top_m: parameters.canopy_top_m,
            canopy_bottom_m: parameters.canopy_bottom_m,
            optics: parameters.optics,
            lai: column.leaf_area_index,
            sai: column.stem_area_index,
            wet_snow_fraction: column.wet_snow_fraction,
        })
        .collect();
    let total = patch.fractions().fold(0.0, |sum, value| value + sum);
    let fcover: Vec<f64> = patch.fractions().map(|value| value / total).collect();
    let three_d = crate::pc_radiation::three_d_canopy_wrap(
        &inputs,
        &fcover,
        cosine_zenith,
        ground.ground,
        vegetation_snow,
    );
    let mut states = Vec::with_capacity(patch.columns.len());
    for (column, pft) in patch.columns.iter_mut().zip(&three_d.pft) {
        column.sunlit_absorption = pft.sunlit_absorption;
        column.shaded_absorption = pft.shaded_absorption;
        column.thermal_gap_fraction = pft.thermal_gap_fraction;
        column.shade_fraction = pft.shade_fraction;
        column.direct_extinction = pft.direct_extinction;
        column.diffuse_extinction = pft.diffuse_extinction;
        states.push(ColdStartRadiation {
            albedo: three_d.albedo,
            sunlit_absorption: pft.sunlit_absorption,
            shaded_absorption: pft.shaded_absorption,
            soil_absorption: [[0.0; 2]; 2],
            snow_absorption: [[0.0; 2]; 2],
            transmission: Some(three_d.transmission),
            snow_age: ground.snow_age,
            thermal_gap_fraction: pft.thermal_gap_fraction,
            direct_extinction: pft.direct_extinction,
            diffuse_extinction: pft.diffuse_extinction,
        });
    }
    let fractions = patch.fractions().collect::<Vec<_>>();
    let previous_thermal_gap_fraction = radiation.thermal_gap_fraction;
    *radiation = aggregate_pft_radiation(&states, &fractions, patch_leaf_stem_area, Some(&ground))?;
    if patch_leaf_stem_area > 1.0e-6 {
        radiation.thermal_gap_fraction = previous_thermal_gap_fraction;
    }
    Ok(())
}

/// `albland` 夜间提前返回之前对 PFT 量做的默认化（`MOD_Albedo.F90:247-257`）。
pub(crate) fn reset_pft_radiation(patch: &mut PftPatch) {
    for column in &mut patch.columns {
        column.sunlit_absorption = [[0.0; 2]; 2];
        column.shaded_absorption = [[0.0; 2]; 2];
        if !PftParameters::has_canopy(column.leaf_area_index, column.stem_area_index) {
            column.thermal_gap_fraction = 1.0;
        }
        column.direct_extinction = 1.0;
        column.diffuse_extinction = 0.718;
    }
}

/// PFT 冠层能量循环的 patch 级输入：地面边界与前置 `GroundFluxes` 在所有 PFT 间共用。
pub(crate) struct PftCanopyContext<'a> {
    pub(crate) input: crate::StandardLctEnergyInput<'a>,
    pub(crate) precipitation_temperature_k: f64,
    pub(crate) soil_surface_resistance_s_m: f64,
    pub(crate) ground_flux: crate::GroundFluxInput,
    pub(crate) preliminary_ground_flux: crate::GroundFluxState,
    pub(crate) shortwave: &'a [PftShortwave],
    pub(crate) interception: &'a [CanopyInterceptionFluxes],
}

/// 一个 PFT 的冠层结果里被 `THERMAL` 聚合的那一组（`MOD_Thermal.F90:1154-1214`）。
#[derive(Debug, Clone, Default)]
struct PftLeafRecord {
    laisun: f64,
    laisha: f64,
    rst: f64,
    assim: f64,
    respc: f64,
    fsenl: f64,
    fevpl: f64,
    etr: f64,
    dlrad: f64,
    ulrad: f64,
    taux: f64,
    tauy: f64,
    fseng: f64,
    fseng_soil: f64,
    fseng_snow: f64,
    fevpg: f64,
    fevpg_soil: f64,
    fevpg_snow: f64,
    cgrnd: f64,
    cgrndl: f64,
    cgrnds: f64,
    zol: f64,
    rib: f64,
    ustar: f64,
    qstar: f64,
    tstar: f64,
    fm: f64,
    fh: f64,
    fq: f64,
    rstfacsun: f64,
    rstfacsha: f64,
    gssun: f64,
    gssha: f64,
    assimsun: f64,
    etrsun: f64,
    assimsha: f64,
    etrsha: f64,
    hprl: f64,
    dheatl: f64,
    rootr: Vec<f64>,
    rootflux: Vec<f64>,
}

/// `THERMAL` 的 PFT 段（`MOD_Thermal.F90:790-1232`，不含 PC）。
///
/// 逐 PFT：有冠层时 `eroot` + `fsun` + `LeafTemperature`（参数换成本 PFT 的），
/// 无冠层时取前置 `GroundFluxes` 的结果（同一组实参，所以同一组值）。
/// 之后按 `pftfrac` 聚合成 patch 级的 [`crate::LeafTemperatureOutput`] 与
/// [`crate::RootUptakeState`]，patch 的 `tleaf`/`ldew*`/`vegwp` 写回 `patch_leaf`。
pub(crate) fn pft_canopy_energy(
    context: PftCanopyContext<'_>,
    patch: &mut PftPatch,
    patch_leaf: &mut LeafTemperatureState,
) -> Result<(crate::LeafTemperatureOutput, crate::RootUptakeState)> {
    let input = context.input;
    let forcing = input.forcing;
    let plant_hydraulics = input.leaf_temperature.plant_hydraulics;
    let layers = patch.layers;
    let preliminary = context.preliminary_ground_flux;
    let ground = context.ground_flux;
    let patch_is_pc = patch.plant_community;
    // `:830-840`：不分冠层雪时，按 patch 叶温把各 PFT 的冠层水整体划成雨或雪。
    if !input.leaf_temperature.options.vegetation_snow {
        let warm = patch_leaf.leaf_temperature_k > crate::FREEZING_K;
        for column in &mut patch.columns {
            let water = &mut column.leaf.canopy_water;
            (water.rain_mm, water.snow_mm) = if warm {
                (water.total_mm, 0.0)
            } else {
                (0.0, water.total_mm)
            };
        }
    }
    let mut records = Vec::with_capacity(patch.columns.len());
    if patch.plant_community {
        records = pc_records(&context, patch)?;
    }
    for (index, (parameters, column)) in patch
        .parameters
        .iter()
        .zip(patch.columns.iter_mut())
        .enumerate()
        .filter(|_| !patch_is_pc)
    {
        if !has_canopy(column.leaf_area_index, column.stem_area_index) {
            // `:876-887` 与 `:1004-1031`。
            column.leaf.canopy_water = CanopyWater {
                total_mm: 0.0,
                rain_mm: 0.0,
                snow_mm: 0.0,
            };
            column.wet_snow_fraction = 0.0;
            column.leaf.leaf_temperature_k = forcing.air_temperature_k;
            column.stomatal_resistance_s_m = 2.0e4;
            column.reference_temperature_k = preliminary.reference_temperature_k;
            column.reference_humidity = preliminary.reference_humidity;
            column.momentum_roughness_m = preliminary.momentum_roughness_m;
            if let Some(state) = column.leaf.plant_hydraulics.as_mut() {
                state.vegetation_water_potential_mm =
                    [BARE_PFT_WATER_POTENTIAL_MM; crate::VEGETATION_SEGMENTS];
            }
            records.push(bare_record(&context, layers));
            continue;
        }
        let root = crate::standard_lct_step::root_uptake_input(input, &parameters.root_fraction)?;
        let shortwave = context.shortwave[index];
        let interception = context.interception[index];
        let template = crate::LeafTemperatureInput {
            leaf_area_index: column.leaf_area_index,
            stem_area_index: column.stem_area_index,
            canopy_top_height_m: parameters.canopy_top_m,
            inverse_sqrt_leaf_dimension_m_neg_half: parameters
                .inverse_sqrt_leaf_dimension_m_neg_half,
            biochemistry: parameters.biochemistry,
            wue_lambda: parameters.wue_lambda,
            plant_hydraulics: plant_hydraulics.map(|shared| crate::LeafPlantHydraulicInput {
                root_fraction: &parameters.root_fraction,
                maximum_sunlit_leaf_hydraulic_conductance: parameters
                    .plant_hydraulic_traits
                    .maximum_sunlit_leaf_conductance,
                maximum_shaded_leaf_hydraulic_conductance: parameters
                    .plant_hydraulic_traits
                    .maximum_shaded_leaf_conductance,
                maximum_xylem_hydraulic_conductance: parameters
                    .plant_hydraulic_traits
                    .maximum_xylem_conductance,
                maximum_root_hydraulic_conductance: parameters
                    .plant_hydraulic_traits
                    .maximum_root_conductance,
                sunlit_leaf_psi50_mm: parameters.plant_hydraulic_traits.sunlit_leaf_psi50_mm,
                shaded_leaf_psi50_mm: parameters.plant_hydraulic_traits.shaded_leaf_psi50_mm,
                xylem_psi50_mm: parameters.plant_hydraulic_traits.xylem_psi50_mm,
                root_psi50_mm: parameters.plant_hydraulic_traits.root_psi50_mm,
                vulnerability_shape: parameters.plant_hydraulic_traits.vulnerability_shape,
                ..shared
            }),
            ..input.leaf_temperature
        };
        let leaf_input = crate::standard_lct_step::leaf_input(
            template,
            forcing,
            crate::standard_lct_step::CanopyDrive {
                direct_extinction: column.direct_extinction,
                diffuse_extinction: column.diffuse_extinction,
                thermal_gap_fraction: column.thermal_gap_fraction,
                par_sunlit_w_m2: shortwave.par_sunlit_w_m2,
                par_shaded_w_m2: shortwave.par_shaded_w_m2,
                sunlit_absorbed_w_m2: shortwave.sunlit_absorbed_w_m2,
                shaded_absorbed_w_m2: shortwave.shaded_absorbed_w_m2,
                retained_rain_kg_m2_s: interception.retained_rain_kg_m2_s,
                retained_snow_kg_m2_s: interception.retained_snow_kg_m2_s,
            },
            root.soil_water_stress,
            root.maximum_transpiration_mm_s,
            context.precipitation_temperature_k,
            context.soil_surface_resistance_s_m,
            ground,
            preliminary,
        );
        let output = crate::leaf_temperature(leaf_input, &mut column.leaf)?;
        column.wet_snow_fraction = output.wet_snow_fraction;
        column.reference_temperature_k = output.air_temperature_2m_k;
        column.reference_humidity = output.air_specific_humidity_2m;
        column.stomatal_resistance_s_m = output.canopy_stomatal_resistance_s_m;
        column.momentum_roughness_m = output.momentum_roughness_m;
        if let Some(value) = output.maximum_sunlit_leaf_conductance_umol_m2_s {
            column.maximum_sunlit_leaf_conductance = value;
        }
        if let Some(value) = output.maximum_shaded_leaf_conductance_umol_m2_s {
            column.maximum_shaded_leaf_conductance = value;
        }
        records.push(PftLeafRecord {
            laisun: output.sunlit_leaf_area_index,
            laisha: output.shaded_leaf_area_index,
            rst: output.canopy_stomatal_resistance_s_m,
            assim: output.assimilation_mol_m2_s,
            respc: output.respiration_mol_m2_s,
            fsenl: output.leaf_sensible_heat_w_m2,
            fevpl: output.leaf_evaporation_kg_m2_s,
            etr: output.transpiration_kg_m2_s,
            dlrad: output.downward_longwave_w_m2,
            ulrad: output.upward_longwave_w_m2,
            taux: output.eastward_stress_kg_m_s2,
            tauy: output.northward_stress_kg_m_s2,
            fseng: output.ground_sensible_heat_w_m2,
            fseng_soil: output.soil_sensible_heat_w_m2,
            fseng_snow: output.snow_sensible_heat_w_m2,
            fevpg: output.ground_evaporation_kg_m2_s,
            fevpg_soil: output.soil_evaporation_kg_m2_s,
            fevpg_snow: output.snow_evaporation_kg_m2_s,
            cgrnd: output.ground_flux_temperature_slope_w_m2_k,
            cgrndl: output.ground_latent_temperature_slope_kg_m2_s_k,
            cgrnds: output.ground_sensible_temperature_slope_w_m2_k,
            zol: output.zol,
            rib: output.bulk_richardson,
            ustar: output.friction_velocity_m_s,
            qstar: output.humidity_scale,
            tstar: output.temperature_scale_k,
            fm: output.momentum_similarity,
            fh: output.heat_similarity,
            fq: output.moisture_similarity,
            rstfacsun: output.sunlit_soil_water_stress,
            rstfacsha: output.shaded_soil_water_stress,
            gssun: output.sunlit_stomatal_conductance_mol_m2_s,
            gssha: output.shaded_stomatal_conductance_mol_m2_s,
            assimsun: output.sunlit_assimilation_mol_m2_s,
            etrsun: output.sunlit_transpiration_kg_m2_s,
            assimsha: output.shaded_assimilation_mol_m2_s,
            etrsha: output.shaded_transpiration_kg_m2_s,
            hprl: output.precipitation_heat_w_m2,
            dheatl: output.canopy_heat_storage_w_m2,
            rootr: root.layer_fraction,
            rootflux: if output.root_flux_kg_m2_s.is_empty() {
                vec![0.0; layers]
            } else {
                output.root_flux_kg_m2_s
            },
        });
    }

    for (column, record) in patch.columns.iter_mut().zip(&records) {
        column.sunlit_leaf_area_index = record.laisun;
        column.shaded_leaf_area_index = record.laisha;
        column.assimilation_mol_m2_s = record.assim;
        column.respiration_mol_m2_s = record.respc;
    }
    let fractions = patch.fractions().collect::<Vec<_>>();
    let sum = |select: fn(&PftLeafRecord) -> f64| {
        pft_sum(records.iter().map(select).zip(fractions.iter().copied()))
    };
    let etr = sum(|r| r.etr);
    // `:1216-1232`：`abs(etr) > 0` 才聚合；否则保持 THERMAL 入口（`:558-559`）清成的 0。
    // `rootr` 那条是 `FMA(rootr_p*etr_p, pftfrac, acc) / etr`（乘积先舍入）；
    // PHS 打开时聚合的是 `rootflux`，`rootr` 整步停在 0。
    let mut root_fraction = vec![0.0; layers];
    let mut root_flux = vec![0.0; layers];
    if etr.abs() > 0.0 {
        if plant_hydraulics.is_some() {
            for (layer, flux) in root_flux.iter_mut().enumerate() {
                *flux = pft_sum(
                    records
                        .iter()
                        .map(|r| r.rootflux[layer])
                        .zip(fractions.iter().copied()),
                );
            }
        } else {
            for (layer, fraction) in root_fraction.iter_mut().enumerate() {
                *fraction = pft_sum(
                    records
                        .iter()
                        .map(|r| r.rootr[layer] * r.etr)
                        .zip(fractions.iter().copied()),
                ) / etr;
            }
        }
    }

    *patch_leaf = LeafTemperatureState {
        leaf_temperature_k: patch.sum(|column| column.leaf.leaf_temperature_k),
        canopy_water: CanopyWater {
            total_mm: patch.sum(|column| column.leaf.canopy_water.total_mm),
            rain_mm: patch.sum(|column| column.leaf.canopy_water.rain_mm),
            snow_mm: patch.sum(|column| column.leaf.canopy_water.snow_mm),
        },
        plant_hydraulics: patch_leaf
            .plant_hydraulics
            .map(|_| crate::PlantHydraulicState {
                vegetation_water_potential_mm: std::array::from_fn(|node| {
                    patch.sum(|column| {
                        column
                            .leaf
                            .plant_hydraulics
                            .map_or(MISSING, |state| state.vegetation_water_potential_mm[node])
                    })
                }),
            }),
    };
    let output = crate::LeafTemperatureOutput {
        wet_snow_fraction: patch.sum(|column| column.wet_snow_fraction),
        eastward_stress_kg_m_s2: sum(|r| r.taux),
        northward_stress_kg_m_s2: sum(|r| r.tauy),
        ground_sensible_heat_w_m2: sum(|r| r.fseng),
        soil_sensible_heat_w_m2: sum(|r| r.fseng_soil),
        snow_sensible_heat_w_m2: sum(|r| r.fseng_snow),
        ground_evaporation_kg_m2_s: sum(|r| r.fevpg),
        soil_evaporation_kg_m2_s: sum(|r| r.fevpg_soil),
        snow_evaporation_kg_m2_s: sum(|r| r.fevpg_snow),
        ground_flux_temperature_slope_w_m2_k: sum(|r| r.cgrnd),
        ground_sensible_temperature_slope_w_m2_k: sum(|r| r.cgrnds),
        ground_latent_temperature_slope_kg_m2_s_k: sum(|r| r.cgrndl),
        air_temperature_2m_k: patch.sum(|column| column.reference_temperature_k),
        air_specific_humidity_2m: patch.sum(|column| column.reference_humidity),
        canopy_stomatal_resistance_s_m: sum(|r| r.rst),
        ground_latent_heat_j_kg: ground.vaporization_heat_j_kg,
        leaf_latent_heat_j_kg: crate::leaf_temperature::LATENT_HEAT_VAPORIZATION_J_KG,
        sunlit_leaf_area_index: sum(|r| r.laisun),
        shaded_leaf_area_index: sum(|r| r.laisha),
        sunlit_stomatal_conductance_mol_m2_s: sum(|r| r.gssun),
        shaded_stomatal_conductance_mol_m2_s: sum(|r| r.gssha),
        assimilation_mol_m2_s: sum(|r| r.assim),
        respiration_mol_m2_s: sum(|r| r.respc),
        leaf_sensible_heat_w_m2: sum(|r| r.fsenl),
        leaf_evaporation_kg_m2_s: sum(|r| r.fevpl),
        transpiration_kg_m2_s: etr,
        sunlit_transpiration_kg_m2_s: sum(|r| r.etrsun),
        shaded_transpiration_kg_m2_s: sum(|r| r.etrsha),
        root_flux_kg_m2_s: if plant_hydraulics.is_some() {
            root_flux
        } else {
            Vec::new()
        },
        sunlit_soil_water_stress: sum(|r| r.rstfacsun),
        shaded_soil_water_stress: sum(|r| r.rstfacsha),
        // patch 的 `gs0sun`/`gs0sha` 在 PFT 模式下不被 `THERMAL` 碰（只写 `_p`）。
        maximum_sunlit_leaf_conductance_umol_m2_s: None,
        maximum_shaded_leaf_conductance_umol_m2_s: None,
        sunlit_assimilation_mol_m2_s: sum(|r| r.assimsun),
        shaded_assimilation_mol_m2_s: sum(|r| r.assimsha),
        downward_longwave_w_m2: sum(|r| r.dlrad),
        upward_longwave_w_m2: sum(|r| r.ulrad),
        precipitation_heat_w_m2: sum(|r| r.hprl),
        canopy_heat_storage_w_m2: sum(|r| r.dheatl),
        momentum_roughness_m: patch.sum(|column| column.momentum_roughness_m),
        zol: sum(|r| r.zol),
        bulk_richardson: sum(|r| r.rib),
        friction_velocity_m_s: sum(|r| r.ustar),
        humidity_scale: sum(|r| r.qstar),
        temperature_scale_k: sum(|r| r.tstar),
        momentum_similarity: sum(|r| r.fm),
        heat_similarity: sum(|r| r.fh),
        moisture_similarity: sum(|r| r.fq),
        // 这两项只是单冠层求解的诊断，PFT 聚合里没有对应量。
        reference_to_canopy_moisture_resistance_s_m: MISSING,
        energy_balance_error_w_m2: 0.0,
        iterations: 0,
    };
    let root_uptake = crate::RootUptakeState {
        layer_fraction: root_fraction,
        maximum_transpiration_mm_s: 0.0,
        soil_water_stress: output.sunlit_soil_water_stress,
    };
    Ok((output, root_uptake))
}

/// `THERMAL` 的 PC 段（`MOD_Thermal.F90:1052-1150`）：自然 PFT 在第一个 PFT 循环里被 `CYCLE`
/// 掉，改由 `LeafTemperaturePC` 一次解完；随后 patch 级的湍流量抄进每个 PFT 的 `_p`。
///
/// 第一个 PFT 循环（`eroot`、`fsun_p`、`laisun_p`）对 PC 照常执行；它算出的 `rootr_p`/`etrc_p`/
/// `rstfac_p` 随后被 PC 段的初始化（`vendor/` 本地修补 FIX 2026-08-16）覆盖成 0/0/1，
/// 所以关掉 PHS 时 `etr >= etrc = 0` 恒成立、蒸腾被截成 0 —— 照样复现。
fn pc_records(context: &PftCanopyContext<'_>, patch: &mut PftPatch) -> Result<Vec<PftLeafRecord>> {
    let input = context.input;
    let forcing = input.forcing;
    let layers = patch.layers;
    let preliminary = context.preliminary_ground_flux;
    // `:1088`：`fcover = pftfrac/sum(pftfrac)`，求和是普通加法。
    let total = patch.fractions().fold(0.0, |sum, value| value + sum);
    let drive: Vec<crate::leaf_temperature_pc::PcPftDrive> = patch
        .parameters
        .iter()
        .zip(&patch.columns)
        .enumerate()
        .map(|(index, (parameters, column))| {
            let shortwave = context.shortwave[index];
            let interception = context.interception[index];
            let absorbed = shortwave.sunlit_absorbed_w_m2 + shortwave.shaded_absorbed_w_m2;
            // `:1081-1086`
            let depth = (column.direct_extinction * column.leaf_area_index).min(40.0);
            let sunlit_fraction = if forcing.cosine_zenith <= 0.0 || absorbed < 1.0 {
                0.5
            } else {
                (1.0 - (-depth).exp()) / depth.max(1.0e-6)
            };
            crate::leaf_temperature_pc::PcPftDrive {
                canopy_layer: parameters.canopy_layer,
                fcover: parameters.fraction / total,
                par_sunlit_w_m2: shortwave.par_sunlit_w_m2,
                par_shaded_w_m2: shortwave.par_shaded_w_m2,
                sunlit_fraction,
                absorbed_solar_w_m2: absorbed,
                retained_rain_kg_m2_s: interception.retained_rain_kg_m2_s,
                retained_snow_kg_m2_s: interception.retained_snow_kg_m2_s,
            }
        })
        .collect();
    // 第一个 PFT 循环对无冠层 PFT 的清零（`:876-887`）。
    for column in &mut patch.columns {
        if !has_canopy(column.leaf_area_index, column.stem_area_index) {
            column.leaf.canopy_water = CanopyWater {
                total_mm: 0.0,
                rain_mm: 0.0,
                snow_mm: 0.0,
            };
            column.wet_snow_fraction = 0.0;
        }
    }
    // `:1092-1094`：PHS 下每步先把 `vegwp_p` 重置成 -2.5e4 再解。
    for column in &mut patch.columns {
        if let Some(state) = column.leaf.plant_hydraulics.as_mut() {
            state.vegetation_water_potential_mm =
                [BARE_PFT_WATER_POTENTIAL_MM; crate::VEGETATION_SEGMENTS];
        }
    }
    let template = crate::standard_lct_step::leaf_input(
        input.leaf_temperature,
        forcing,
        crate::standard_lct_step::CanopyDrive {
            direct_extinction: 1.0,
            diffuse_extinction: 1.0,
            thermal_gap_fraction: 1.0,
            par_sunlit_w_m2: 0.0,
            par_shaded_w_m2: 0.0,
            sunlit_absorbed_w_m2: 0.0,
            shaded_absorbed_w_m2: 0.0,
            retained_rain_kg_m2_s: 0.0,
            retained_snow_kg_m2_s: 0.0,
        },
        1.0,
        0.0,
        context.precipitation_temperature_k,
        context.soil_surface_resistance_s_m,
        context.ground_flux,
        preliminary,
    );
    let (shared, fluxes) = crate::leaf_temperature_pc::leaf_temperature_pc(
        template,
        forcing.air_temperature_k,
        preliminary.heat_roughness_m,
        preliminary.friction_velocity_m_s,
        &patch.parameters,
        &mut patch.columns,
        &drive,
    )?;
    let mut records = Vec::with_capacity(fluxes.len());
    for ((column, flux), pft) in patch.columns.iter_mut().zip(fluxes).zip(&drive) {
        // 第一个 PFT 循环（`:853-889`）：有冠层时 `laisun_p = lai_p*fsun_p`，否则清零。
        let (laisun, laisha) = if has_canopy(column.leaf_area_index, column.stem_area_index) {
            (
                column.leaf_area_index * pft.sunlit_fraction,
                column.leaf_area_index * (1.0 - pft.sunlit_fraction),
            )
        } else {
            (0.0, 0.0)
        };
        // `:1129-1150`：patch 值抄进 `_p`。
        column.reference_temperature_k = shared.tref;
        column.reference_humidity = shared.qref;
        column.momentum_roughness_m = shared.z0m;
        column.stomatal_resistance_s_m = flux.rst;
        if let Some(value) = flux.gs0sun {
            column.maximum_sunlit_leaf_conductance = value;
        }
        if let Some(value) = flux.gs0sha {
            column.maximum_shaded_leaf_conductance = value;
        }
        records.push(PftLeafRecord {
            laisun,
            laisha,
            rst: flux.rst,
            assim: flux.assim,
            respc: flux.respc,
            fsenl: flux.fsenl,
            fevpl: flux.fevpl,
            etr: flux.etr,
            dlrad: shared.dlrad,
            ulrad: shared.ulrad,
            taux: shared.taux,
            tauy: shared.tauy,
            fseng: shared.fseng,
            fseng_soil: shared.fseng_soil,
            fseng_snow: shared.fseng_snow,
            fevpg: shared.fevpg,
            fevpg_soil: shared.fevpg_soil,
            fevpg_snow: shared.fevpg_snow,
            cgrnd: shared.cgrnd,
            cgrndl: shared.cgrndl,
            cgrnds: shared.cgrnds,
            zol: shared.zol,
            rib: shared.rib,
            ustar: shared.ustar,
            qstar: shared.qstar,
            tstar: shared.tstar,
            fm: shared.fm,
            fh: shared.fh,
            fq: shared.fq,
            rstfacsun: flux.rstfacsun,
            rstfacsha: flux.rstfacsha,
            gssun: flux.gssun,
            gssha: flux.gssha,
            assimsun: flux.assimsun,
            etrsun: flux.etrsun,
            assimsha: flux.assimsha,
            etrsha: flux.etrsha,
            hprl: flux.hprl,
            dheatl: flux.dheatl,
            rootr: vec![0.0; layers],
            rootflux: if flux.rootflux.is_empty() {
                vec![0.0; layers]
            } else {
                flux.rootflux
            },
        });
    }
    Ok(records)
}

/// 无冠层 PFT 的记录：湍流量取前置 `GroundFluxes`，长波按裸地（`:1004-1026`）。
fn bare_record(context: &PftCanopyContext<'_>, layers: usize) -> PftLeafRecord {
    let preliminary = context.preliminary_ground_flux;
    let ground = context.ground_flux;
    let longwave = context.input.forcing.downward_longwave_w_m2;
    let emissivity = context.input.leaf_temperature.ground_emissivity;
    let fourth = |t: f64| {
        let square = t * t;
        square * square
    };
    // GIMPLE：`FMA(t^4, emg*stefnc, frl*(1-emg))`；split 时两项依次
    // `FMA(fsno*emg*stefnc, t_snow^4, ·)`、`FMA((1-fsno)*emg*stefnc, t_soil^4, ·)`，
    // `t^4` 都是 `(t*t)*(t*t)`。
    let base = longwave * (1.0 - emissivity);
    let stefan = crate::leaf_temperature::STEFAN_BOLTZMANN;
    let ulrad = if context.input.ground_temperature.use_split_soil_snow {
        let fsno = ground.snow_cover_fraction;
        let snow = (fsno * emissivity * stefan).mul_add(fourth(ground.snow_temperature_k), base);
        ((1.0 - fsno) * emissivity * stefan).mul_add(fourth(ground.soil_temperature_k), snow)
    } else {
        fourth(ground.ground_temperature_k).mul_add(emissivity * stefan, base)
    };
    PftLeafRecord {
        rst: 2.0e4,
        dlrad: longwave,
        ulrad,
        taux: preliminary.eastward_stress_kg_m_s2,
        tauy: preliminary.northward_stress_kg_m_s2,
        fseng: preliminary.sensible_heat_w_m2,
        fseng_soil: preliminary.soil_sensible_heat_w_m2,
        fseng_snow: preliminary.snow_sensible_heat_w_m2,
        fevpg: preliminary.evaporation_kg_m2_s,
        fevpg_soil: preliminary.soil_evaporation_kg_m2_s,
        fevpg_snow: preliminary.snow_evaporation_kg_m2_s,
        cgrnd: preliminary.ground_flux_temperature_derivative_w_m2_k,
        cgrndl: preliminary.latent_temperature_derivative_kg_m2_s_k,
        cgrnds: preliminary.sensible_temperature_derivative_w_m2_k,
        zol: preliminary.dimensionless_height,
        rib: preliminary.bulk_richardson_number,
        ustar: preliminary.friction_velocity_m_s,
        qstar: preliminary.humidity_scale,
        tstar: preliminary.temperature_scale_k,
        fm: preliminary.momentum_integral,
        fh: preliminary.heat_integral,
        fq: preliminary.moisture_integral,
        rootr: vec![0.0; layers],
        rootflux: vec![0.0; layers],
        ..PftLeafRecord::default()
    }
}

pub(crate) fn has_canopy(leaf_area_index: f64, stem_area_index: f64) -> bool {
    PftParameters::has_canopy(leaf_area_index, stem_area_index)
}

#[cfg(test)]
#[path = "pft_tests.rs"]
mod pft_tests;

//! Per-layer soil thermal properties from MOD_SoilThermalParameters.F90.

use crate::LibmPow;
use anyhow::{ensure, Result};

use crate::FREEZING_K;

use crate::f77;

/// CoLM runtime selector DEF_THERMAL_CONDUCTIVITY_SCHEME.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalConductivityScheme {
    Oleson,
    Johansen,
    CoteKonrad,
    BallandArp,
    Lu,
    TarnawskiLeong,
    DeVries,
    YanHe,
}

/// Scalar inputs to soil_hcap_cond for one soil layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoilThermalInput {
    pub gravel_volume_fraction_of_solids: f64,
    pub organic_volume_fraction_of_solids: f64,
    pub sand_volume_fraction_of_solids: f64,
    pub pore_volume_fraction: f64,
    pub gravel_mass_fraction: f64,
    pub sand_mass_fraction: f64,
    pub solid_conductivity_w_m_k: f64,
    pub dry_heat_capacity_j_m3_k: f64,
    pub dry_conductivity_w_m_k: f64,
    pub saturated_unfrozen_conductivity_w_m_k: f64,
    pub saturated_frozen_conductivity_w_m_k: f64,
    pub balland_alpha: f64,
    pub balland_beta: f64,
    pub temperature_k: f64,
    pub liquid_volume_fraction: f64,
    pub ice_volume_fraction: f64,
}

/// soil_hcap_cond outputs for one soil layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoilThermalProperties {
    pub heat_capacity_j_m3_k: f64,
    pub conductivity_w_m_k: f64,
}

/// Port of MOD_SoilThermalParameters:soil_hcap_cond.
pub fn soil_thermal_properties(
    input: SoilThermalInput,
    scheme: ThermalConductivityScheme,
) -> Result<SoilThermalProperties> {
    validate(input)?;
    // `hcap = csol + vf_water*c_water + vf_ice*c_ice`（`MOD_SoilThermalParameters.F90:299`）。
    // 两个乘积都被收进加法（GIMPLE：`FMA(vfw, cw, csol)` 再 `FMA(vfi, ci, ·)`）——
    // `csol` 是变量不是乘积，所以第一级收的是右边那个。
    let heat_capacity_j_m3_k = input.ice_volume_fraction.mul_add(
        f77(1.94153e6),
        input
            .liquid_volume_fraction
            .mul_add(f77(4.188e6), input.dry_heat_capacity_j_m3_k),
    );
    let saturation = ((input.liquid_volume_fraction + input.ice_volume_fraction)
        / input.pore_volume_fraction)
        .min(1.0);
    // `sr < 1e-10` 那道门（`MOD_SoilThermalParameters.F90:314-420`）只把 `ke` 置 0，
    // 而方案 6/7 的两段 `IF`（`:432-497`）在它**外面**、且根本不读 `ke` ——
    // 干土上它们照样按自己的式子算。所以这两档不能走早返回，其余各档等价
    // （1–5 的 `thk = (ksat-kdry)*0+kdry`，8 自己重算 `ke`，结果都是 kdry）。
    // 8 档方案 × 5000 组随机/边界输入的双侧差分
    // 覆盖了这条干土路径 3134 次，两个输出全部逐位一致。
    let conductivity_w_m_k = match scheme {
        ThermalConductivityScheme::TarnawskiLeong | ThermalConductivityScheme::DeVries => {
            conductivity_for_saturated_soil(input, saturation, scheme)
        }
        _ => {
            if saturation < f77(1.0e-10) {
                input.dry_conductivity_w_m_k
            } else {
                conductivity_for_saturated_soil(input, saturation, scheme)
            }
        }
    };
    Ok(SoilThermalProperties {
        heat_capacity_j_m3_k,
        conductivity_w_m_k,
    })
}

fn conductivity_for_saturated_soil(
    input: SoilThermalInput,
    saturation: f64,
    scheme: ThermalConductivityScheme,
) -> f64 {
    let unfrozen = input.temperature_k > FREEZING_K;
    match scheme {
        ThermalConductivityScheme::Oleson
        | ThermalConductivityScheme::Johansen
        | ThermalConductivityScheme::CoteKonrad
        | ThermalConductivityScheme::BallandArp
        | ThermalConductivityScheme::Lu => {
            let kersten = kersten_number(input, saturation, scheme, unfrozen).clamp(0.0, 1.0);
            let saturated = if unfrozen {
                input.saturated_unfrozen_conductivity_w_m_k
            } else {
                input.saturated_frozen_conductivity_w_m_k
            };
            // `thk = (ksat_u-kdry)*ke + kdry`（`MOD_SoilThermalParameters.F90:407-411`）：
            // 乘积被吸收（GIMPLE 的 `FMA(ksat-kdry, ke, kdry)`）。这条在**两个**
            // 方案分支里各出现一次（1–5 一组、6–8 那组），两处都要。
            (saturated - input.dry_conductivity_w_m_k)
                .mul_add(kersten, input.dry_conductivity_w_m_k)
        }
        ThermalConductivityScheme::TarnawskiLeong => tarnawski_leong(input, saturation, unfrozen),
        ThermalConductivityScheme::DeVries => de_vries(input, saturation, unfrozen),
        ThermalConductivityScheme::YanHe => {
            // `beta = -0.303*ksat_u - 0.201*wf_sand + 1.532`：GIMPLE 是
            // `FNMS(ksat_u, 0.303, 0.201*wf_sand)` —— 第一个乘积被吸收，
            // 第二个是**先舍入**的普通乘积（它被提成公共量）。1.532 最后加。
            let beta = (-input.saturated_unfrozen_conductivity_w_m_k)
                .mul_add(f77(0.303), -(f77(0.201) * input.sand_mass_fraction))
                + f77(1.532);
            let kersten = if input.liquid_volume_fraction > f77(0.01) {
                (1.0 + (input.pore_volume_fraction / beta).lpow(-beta))
                    / (1.0 + (input.liquid_volume_fraction / beta).lpow(-beta))
            } else {
                0.0
            }
            .clamp(0.0, 1.0);
            let saturated = if unfrozen {
                input.saturated_unfrozen_conductivity_w_m_k
            } else {
                input.saturated_frozen_conductivity_w_m_k
            };
            // `thk = (ksat_u-kdry)*ke + kdry`（`MOD_SoilThermalParameters.F90:407-411`）：
            // 乘积被吸收（GIMPLE 的 `FMA(ksat-kdry, ke, kdry)`）。这条在**两个**
            // 方案分支里各出现一次（1–5 一组、6–8 那组），两处都要。
            (saturated - input.dry_conductivity_w_m_k)
                .mul_add(kersten, input.dry_conductivity_w_m_k)
        }
    }
}

fn kersten_number(
    input: SoilThermalInput,
    saturation: f64,
    scheme: ThermalConductivityScheme,
    unfrozen: bool,
) -> f64 {
    let coarse_fraction =
        input.gravel_volume_fraction_of_solids + input.sand_volume_fraction_of_solids;
    match scheme {
        ThermalConductivityScheme::Oleson => {
            if unfrozen {
                saturation.log10() + 1.0
            } else {
                saturation
            }
        }
        ThermalConductivityScheme::Johansen => {
            if !unfrozen {
                saturation
            } else if coarse_fraction > f77(0.4) {
                // 粗粒支是 `ke = 0.7*log10(max(sr,0.05)) + 1.0`，乘积被吸收
                // （GIMPLE：`FMA(log10, 0.7, 1.0)`）；紧邻的细粒支没有乘积
                // （`log10(max(sr,0.1)) + 1.0`），保持普通加法。
                f77(0.7).mul_add(saturation.max(f77(0.05)).log10(), 1.0)
            } else {
                saturation.max(f77(0.1)).log10() + 1.0
            }
        }
        ThermalConductivityScheme::CoteKonrad => {
            let kappa = if unfrozen {
                if coarse_fraction > f77(0.40) {
                    f77(4.60)
                } else if coarse_fraction > f77(0.25) {
                    f77(3.55)
                } else if coarse_fraction > f77(0.01) {
                    f77(1.90)
                } else {
                    f77(0.60)
                }
            } else if coarse_fraction > f77(0.40) {
                f77(1.70)
            } else if coarse_fraction > f77(0.25) {
                f77(0.95)
            } else if coarse_fraction > f77(0.01) {
                f77(0.85)
            } else {
                f77(0.25)
            };
            // `ke = kappa*sr/(1.0+(kappa-1.0)*sr)`：分子是普通乘积，分母的
            // `(kappa-1.0)*sr` 被吸收（GIMPLE：`FMA(sr, kappa-1, 1.0)`）。
            kappa * saturation / (kappa - 1.0).mul_add(saturation, 1.0)
        }
        ThermalConductivityScheme::BallandArp => {
            if unfrozen {
                // `ke = sr**(0.5*(1.+vf_om-BA_alpha*vf_sand-vf_gravels))
                //        * ((1/(1+exp(-BA_beta*sr)))**3 - ((1-sr)/2)**3)**(1-vf_om)`
                // （`MOD_SoilThermalParameters.F90:369-372`）。三处收缩（GIMPLE 实测）：
                // `FNMA(BA_alpha, vf_sand, 1+vf_om)`、`FMS(wet, wet*wet, dry**3)`、
                // 以及最后 `thk` 那条 `FMA(ksat-kdry, ke, kdry)`。
                let exponent_sum = (-input.balland_alpha).mul_add(
                    input.sand_volume_fraction_of_solids,
                    1.0 + input.organic_volume_fraction_of_solids,
                ) - input.gravel_volume_fraction_of_solids;
                let wet_cube_base = 1.0 / (1.0 + (-input.balland_beta * saturation).exp());
                let dry_cube_base = (1.0 - saturation) / f77(2.0);
                let cube_difference = wet_cube_base.mul_add(
                    wet_cube_base * wet_cube_base,
                    -(dry_cube_base * dry_cube_base * dry_cube_base),
                );
                saturation.lpow(f77(0.5) * exponent_sum)
                    * cube_difference.lpow(1.0 - input.organic_volume_fraction_of_solids)
            } else {
                saturation.lpow(1.0 + input.organic_volume_fraction_of_solids)
            }
        }
        ThermalConductivityScheme::Lu => {
            let (alpha, beta) = if coarse_fraction > f77(0.4) {
                (f77(0.728), f77(1.165))
            } else {
                (f77(0.37), f77(1.29))
            };
            if unfrozen {
                (alpha * (1.0 - saturation.lpow(alpha - beta))).exp()
            } else {
                saturation
            }
        }
        ThermalConductivityScheme::TarnawskiLeong
        | ThermalConductivityScheme::DeVries
        | ThermalConductivityScheme::YanHe => unreachable!("handled by its direct formula"),
    }
}

fn tarnawski_leong(input: SoilThermalInput, saturation: f64, unfrozen: bool) -> f64 {
    let coarse_fraction = input.gravel_mass_fraction + input.sand_mass_fraction;
    let coarse_cube = coarse_fraction.powi(3);
    // `aa = 0.0237-0.0175*a**3`、`nwm = 0.088-0.037*a**3`：两个乘积都被吸收
    // （GIMPLE：`FNMA(a**3, 0.0175, 0.0237)` 与 `FNMA(a**3, 0.037, 0.088)`）。
    let solid_path = (-f77(0.0175)).mul_add(coarse_cube, f77(0.0237));
    let micro_pore_fraction = (-f77(0.037)).mul_add(coarse_cube, f77(0.088));
    // `x = 0.6-0.3*a**3`，而 `sr**(-x)` 里 GCC 把负号并进了收缩：
    // `FNMA(a**3, 0.3, 0.6)` 算的正是 `0.3*a**3-0.6 = -x`，所以这里直接算 `-x`。
    let negated_exponent = f77(0.3).mul_add(coarse_cube, -f77(0.6));
    let wet_micro_pores = if saturation < f77(1.0e-6) {
        0.0
    } else {
        (1.0 - saturation.lpow(negated_exponent)).exp()
    };
    let fluid_conductivity = if unfrozen { f77(0.57) } else { f77(2.29) };
    let air_conductivity = f77(0.024);
    // `kf*nw_nwm + ka*(1-nw_nwm)`：被加数那个乘积被吸收
    // （GIMPLE：`FMA(nw_nwm, kf, ka*(1-nw_nwm))`）。
    let pore_fluid_conductivity = wet_micro_pores.mul_add(
        fluid_conductivity,
        air_conductivity * (1.0 - wet_micro_pores),
    );
    let free_pores = 1.0 - input.pore_volume_fraction - solid_path;
    // `sr*vf_pores - nwm*nw_nwm`：减数被吸收（`FNMA(nw_nwm, nwm, vf_pores*sr)`）。
    let liquid_excess =
        (-micro_pore_fraction).mul_add(wet_micro_pores, input.pore_volume_fraction * saturation);
    // `vf_pores*(1-sr) - nwm*(1-nw_nwm)`：被减数那个乘积被吸收（`FMS`），
    // 减数是**普通乘积**（它先被 CSE 成 `nwm*(1-nw_nwm)` 供两条分支共用）。
    let air_excess = input.pore_volume_fraction.mul_add(
        1.0 - saturation,
        -(micro_pore_fraction * (1.0 - wet_micro_pores)),
    );
    // 加和顺序按 GIMPLE：`(2次项/分母 + k_solids*aa)` 先成和，再 `FMA(liquid_excess, kf, ·)`，
    // 最后才加 `ka*air_excess`。
    liquid_excess.mul_add(
        fluid_conductivity,
        (free_pores + micro_pore_fraction).powi(2)
            / (free_pores / input.solid_conductivity_w_m_k
                + micro_pore_fraction / pore_fluid_conductivity)
            + input.solid_conductivity_w_m_k * solid_path,
    ) + air_excess * air_conductivity
}

fn de_vries(input: SoilThermalInput, saturation: f64, unfrozen: bool) -> f64 {
    let air_conductivity = f77(0.024);
    let fluid_conductivity = if unfrozen { f77(0.57) } else { f77(2.29) };
    // `sr*vf_pores` 在形状因子与分子/分母里都是同一个（已舍入的）量。
    let pore_liquid = saturation * input.pore_volume_fraction;
    let shape_air = if pore_liquid <= f77(0.09) {
        // `ga = 0.013+0.944*sr*vf_pores`：GIMPLE 先把 `0.944*sr` 舍成 `_163`，
        // 再 `FMA(vf_pores, _163, 0.013)` —— 分组不是"0.944*(sr*vf_pores)"。
        (f77(0.944) * saturation).mul_add(input.pore_volume_fraction, f77(0.013))
    } else {
        // `ga = 0.333-(1-sr)*vf_pores/vf_pores*(0.333-0.035)` ⇒
        // `FNMA((1-sr)*vf_pores/vf_pores, 0.29800000000000004, 0.333)`。
        (-f77(0.333 - 0.035)).mul_add(
            (1.0 - saturation) * input.pore_volume_fraction / input.pore_volume_fraction,
            f77(0.333),
        )
    };
    // `gc = 1-2*ga`：乘积被吸收（`FNMA(ga, 2.0, 1.0)`）。
    let shape_solid = (-f77(2.0)).mul_add(shape_air, 1.0);
    let air_ratio = air_conductivity / fluid_conductivity - 1.0;
    let solid_ratio = input.solid_conductivity_w_m_k / fluid_conductivity - 1.0;
    // 四个 `1/(1+(比值)*形状因子)` 都是 `FMA(形状因子, 比值, 1.0)`。
    let fluid_shape = (f77(2.0) / air_ratio.mul_add(shape_air, 1.0)
        + 1.0 / air_ratio.mul_add(shape_solid, 1.0))
        / f77(3.0);
    let solid_shape = (f77(2.0) / solid_ratio.mul_add(f77(0.125), 1.0)
        + 1.0 / solid_ratio.mul_add(1.0 - f77(2.0) * f77(0.125), 1.0))
        / f77(3.0);
    // 分子：`sr*vf_pores*k_water + (1-sr)*vf_pores*aa*k_air + (1-vf_pores)*aaa*k_solids`
    // ⇒ `FMA(sr*vf_pores, kf, (1-sr)*vf_pores*aa*k_air)` 再 `FMA((1-vf_pores)*aaa, ks, ·)`；
    // 分母里的两个乘积都是普通乘积，和式顺序也与分子一致。
    let pore_one_minus = input.pore_volume_fraction * (1.0 - saturation);
    let solid_term = solid_shape * (1.0 - input.pore_volume_fraction);
    let numerator = solid_term.mul_add(
        input.solid_conductivity_w_m_k,
        pore_liquid.mul_add(
            fluid_conductivity,
            (fluid_shape * pore_one_minus) * air_conductivity,
        ),
    );
    numerator / (solid_term + (pore_liquid + fluid_shape * pore_one_minus))
}

fn validate(input: SoilThermalInput) -> Result<()> {
    let values = [
        input.gravel_volume_fraction_of_solids,
        input.organic_volume_fraction_of_solids,
        input.sand_volume_fraction_of_solids,
        input.pore_volume_fraction,
        input.gravel_mass_fraction,
        input.sand_mass_fraction,
        input.solid_conductivity_w_m_k,
        input.dry_heat_capacity_j_m3_k,
        input.dry_conductivity_w_m_k,
        input.saturated_unfrozen_conductivity_w_m_k,
        input.saturated_frozen_conductivity_w_m_k,
        input.balland_alpha,
        input.balland_beta,
        input.temperature_k,
        input.liquid_volume_fraction,
        input.ice_volume_fraction,
    ];
    ensure!(
        values.iter().all(|value| value.is_finite()),
        "soil thermal inputs must be finite"
    );
    ensure!(
        input.pore_volume_fraction > 0.0
            && input.solid_conductivity_w_m_k > 0.0
            && input.dry_conductivity_w_m_k >= 0.0
            && input.saturated_unfrozen_conductivity_w_m_k >= 0.0
            && input.saturated_frozen_conductivity_w_m_k >= 0.0
            && input.liquid_volume_fraction >= -crate::SOIL_WATER_ROUNDOFF_KG_M2
            && input.ice_volume_fraction >= 0.0,
        "soil thermal inputs are physically invalid"
    );
    Ok(())
}

/// Builds the per-layer `soil_hcap_cond` scalar inputs for one patch.
///
/// The static half of `SoilThermalInput` is `MOD_Vars_TimeInvariants`' soil state and
/// lives in the constant restart; the dynamic half is the current column.  Only the
/// volume fractions are derived here, with the source's own expressions
/// (`MOD_GroundTemperature.F90`: `vf_water = wliq / (dz * denh2o)`,
/// `vf_ice = wice / (dz * denice)`), so the density constants cannot drift between
/// the driver and the physics.
pub fn soil_thermal_inputs(
    soil: &crate::SoilState,
    patch: usize,
    temperature_k: &[f64],
    liquid_water_kg_m2: &[f64],
    ice_water_kg_m2: &[f64],
    layer_thickness_m: &[f64],
) -> Result<Vec<SoilThermalInput>> {
    ensure!(
        patch < soil.patches,
        "soil thermal input patch {patch} is outside {} patches",
        soil.patches
    );
    for (name, values) in [
        ("temperature_k", temperature_k),
        ("liquid_water_kg_m2", liquid_water_kg_m2),
        ("ice_water_kg_m2", ice_water_kg_m2),
        ("layer_thickness_m", layer_thickness_m),
    ] {
        ensure!(
            values.len() == soil.layers,
            "soil thermal input {name} has {} entries, expected {}",
            values.len(),
            soil.layers
        );
    }
    let field = |name: crate::SoilField, layer: usize| soil.get(name, layer, patch);
    (0..soil.layers)
        .map(|layer| {
            let thickness_m = layer_thickness_m[layer];
            ensure!(
                thickness_m > 0.0,
                "soil thermal input layer {layer} has no thickness"
            );
            Ok(SoilThermalInput {
                gravel_volume_fraction_of_solids: field(crate::SoilField::VfGravels, layer),
                organic_volume_fraction_of_solids: field(crate::SoilField::VfOm, layer),
                sand_volume_fraction_of_solids: field(crate::SoilField::VfSand, layer),
                pore_volume_fraction: field(crate::SoilField::Porosity, layer),
                gravel_mass_fraction: field(crate::SoilField::WfGravels, layer),
                sand_mass_fraction: field(crate::SoilField::WfSand, layer),
                solid_conductivity_w_m_k: field(crate::SoilField::SolidThermalConductivity, layer),
                dry_heat_capacity_j_m3_k: field(crate::SoilField::HeatCapacity, layer),
                dry_conductivity_w_m_k: field(crate::SoilField::DryConductivity, layer),
                saturated_unfrozen_conductivity_w_m_k: field(
                    crate::SoilField::SaturatedUnfrozenConductivity,
                    layer,
                ),
                saturated_frozen_conductivity_w_m_k: field(
                    crate::SoilField::SaturatedFrozenConductivity,
                    layer,
                ),
                balland_alpha: field(crate::SoilField::BaAlpha, layer),
                balland_beta: field(crate::SoilField::BaBeta, layer),
                temperature_k: temperature_k[layer],
                liquid_volume_fraction: liquid_water_kg_m2[layer]
                    / (thickness_m * WATER_DENSITY_KG_M3),
                ice_volume_fraction: ice_water_kg_m2[layer] / (thickness_m * ICE_DENSITY_KG_M3),
            })
        })
        .collect()
}

const WATER_DENSITY_KG_M3: f64 = f77(1000.0);
const ICE_DENSITY_KG_M3: f64 = f77(917.0);

#[cfg(test)]
#[path = "thermal_properties_tests.rs"]
mod thermal_properties_tests;

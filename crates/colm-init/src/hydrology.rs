//! Soil hydraulic functions from `main/HYDRO/MOD_Hydro_SoilFunction.F90`.

/// CoLM's lower bound for soil matric potential (mm).
pub const MIN_SOIL_PSI: f64 = -1.0e8;

/// Runtime hydraulic parameters; values are already per soil layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SoilHydraulicModel {
    Campbell {
        bsw: f64,
    },
    VanGenuchten {
        alpha_vgm: f64,
        n_vgm: f64,
        l_vgm: f64,
        sc_vgm: f64,
        fc_vgm: f64,
    },
}

/// `get_water_equilibrium_state` output; all vectors are layer-major in model order.
#[derive(Debug, Clone, PartialEq)]
pub struct EquilibriumWaterState {
    pub liquid_water_kg_m2: Vec<f64>,
    pub matric_potential_mm: Vec<f64>,
    pub hydraulic_conductivity_mm_s: Vec<f64>,
    pub aquifer_water_mm: f64,
}

/// Applies `MOD_Hydro_SoilWater::get_water_equilibrium_state`.
#[allow(clippy::too_many_arguments)]
pub fn equilibrium_water_state(
    water_table_mm: f64,
    center_mm: &[f64],
    interface_mm: &[f64],
    porosity: &[f64],
    residual_water: &[f64],
    psi_s_mm: &[f64],
    saturated_conductivity_mm_s: &[f64],
    model: &[SoilHydraulicModel],
) -> Result<EquilibriumWaterState, &'static str> {
    let layers = porosity.len();
    if layers == 0
        || center_mm.len() != layers
        || interface_mm.len() != layers + 1
        || residual_water.len() != layers
        || psi_s_mm.len() != layers
        || saturated_conductivity_mm_s.len() != layers
        || model.len() != layers
    {
        return Err("equilibrium soil fields have incompatible dimensions");
    }
    let water_layer = interface_mm
        .iter()
        .rposition(|&z| water_table_mm >= z)
        .unwrap_or(0)
        + 1;
    let psi_at_water_table = psi_s_mm[(water_layer - 1).min(layers - 1)];
    let mut liquid_water_kg_m2 = vec![0.0; layers];
    let mut matric_potential_mm = vec![0.0; layers];
    let mut hydraulic_conductivity_mm_s = vec![0.0; layers];
    for layer in 0..layers {
        if layer + 1 < water_layer {
            let psi = psi_at_water_table - (water_table_mm - center_mm[layer]);
            matric_potential_mm[layer] = psi;
            liquid_water_kg_m2[layer] = soil_vliq_from_psi(
                psi,
                porosity[layer],
                residual_water[layer],
                psi_s_mm[layer],
                model[layer],
            ) * (interface_mm[layer + 1] - interface_mm[layer]);
        } else if layer + 1 == water_layer {
            let upper_psi = psi_at_water_table
                - (water_table_mm - interface_mm[layer])
                    * (interface_mm[layer + 1] - center_mm[layer])
                    / (interface_mm[layer + 1] - interface_mm[layer]);
            let upper_water = soil_vliq_from_psi(
                upper_psi,
                porosity[layer],
                residual_water[layer],
                psi_s_mm[layer],
                model[layer],
            );
            liquid_water_kg_m2[layer] = upper_water * (water_table_mm - interface_mm[layer])
                + porosity[layer] * (interface_mm[layer + 1] - water_table_mm);
            matric_potential_mm[layer] = soil_psi_from_vliq(
                liquid_water_kg_m2[layer] / (interface_mm[layer + 1] - interface_mm[layer]),
                porosity[layer],
                residual_water[layer],
                psi_s_mm[layer],
                model[layer],
            );
        } else {
            liquid_water_kg_m2[layer] =
                porosity[layer] * (interface_mm[layer + 1] - interface_mm[layer]);
            matric_potential_mm[layer] = psi_s_mm[layer];
        }
        hydraulic_conductivity_mm_s[layer] = soil_hydraulic_conductivity(
            matric_potential_mm[layer],
            psi_s_mm[layer],
            saturated_conductivity_mm_s[layer],
            model[layer],
        );
    }
    let aquifer_water_mm = if water_layer == layers + 1 {
        let psi = psi_at_water_table - (water_table_mm - interface_mm[layers]) * 0.5;
        let water = soil_vliq_from_psi(
            psi,
            porosity[layers - 1],
            residual_water[layers - 1],
            psi_s_mm[layers - 1],
            model[layers - 1],
        );
        -(water_table_mm - interface_mm[layers]) * (porosity[layers - 1] - water)
    } else {
        0.0
    };
    Ok(EquilibriumWaterState {
        liquid_water_kg_m2,
        matric_potential_mm,
        hydraulic_conductivity_mm_s,
        aquifer_water_mm,
    })
}

/// `soil_psi_from_vliq`: matric potential (mm) from volumetric liquid water.
pub fn soil_psi_from_vliq(
    vliq: f64,
    porosity: f64,
    residual_water: f64,
    psi_s: f64,
    model: SoilHydraulicModel,
) -> f64 {
    if vliq >= porosity {
        return psi_s;
    }
    if vliq <= residual_water.max(1.0e-8) {
        return MIN_SOIL_PSI;
    }
    let psi = match model {
        SoilHydraulicModel::Campbell { bsw } => psi_s * (vliq / porosity).powf(-bsw),
        SoilHydraulicModel::VanGenuchten {
            alpha_vgm,
            n_vgm,
            sc_vgm,
            ..
        } => {
            let m_vgm = 1.0 - 1.0 / n_vgm;
            let esat = (vliq - residual_water) / (porosity - residual_water);
            -((esat * sc_vgm).powf(-1.0 / m_vgm) - 1.0).powf(1.0 / n_vgm) / alpha_vgm
        }
    };
    psi.max(MIN_SOIL_PSI)
}

/// `soil_vliq_from_psi`: volumetric liquid water from matric potential (mm).
pub fn soil_vliq_from_psi(
    psi: f64,
    porosity: f64,
    residual_water: f64,
    psi_s: f64,
    model: SoilHydraulicModel,
) -> f64 {
    if psi >= psi_s {
        return porosity;
    }
    match model {
        SoilHydraulicModel::Campbell { bsw } => porosity * (psi / psi_s).powf(-1.0 / bsw),
        SoilHydraulicModel::VanGenuchten {
            alpha_vgm,
            n_vgm,
            sc_vgm,
            ..
        } => {
            let m_vgm = 1.0 - 1.0 / n_vgm;
            let esat = (1.0 + (psi * -alpha_vgm).powf(n_vgm)).powf(-m_vgm) / sc_vgm;
            (porosity - residual_water) * esat + residual_water
        }
    }
}

/// `soil_hk_from_psi`: hydraulic conductivity at a matric potential.
pub fn soil_hydraulic_conductivity(
    psi: f64,
    psi_s: f64,
    saturated_conductivity: f64,
    model: SoilHydraulicModel,
) -> f64 {
    if psi >= psi_s {
        return saturated_conductivity;
    }
    match model {
        SoilHydraulicModel::Campbell { bsw } => {
            saturated_conductivity * (psi / psi_s).powf(-3.0 / bsw - 2.0)
        }
        SoilHydraulicModel::VanGenuchten {
            alpha_vgm,
            n_vgm,
            l_vgm,
            sc_vgm,
            fc_vgm,
        } => {
            let m_vgm = 1.0 - 1.0 / n_vgm;
            let esat = (1.0 + (-alpha_vgm * psi).powf(n_vgm)).powf(-m_vgm) / sc_vgm;
            saturated_conductivity
                * esat.powf(l_vgm)
                * ((1.0 - (1.0 - (esat * sc_vgm).powf(1.0 / m_vgm)).powf(m_vgm)) / fc_vgm).powi(2)
        }
    }
}

#[cfg(test)]
#[path = "hydrology_tests.rs"]
mod hydrology_tests;

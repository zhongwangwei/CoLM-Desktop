//! `DEF_USE_SNICAR` 在时间循环里的雪柱状态与步内环节（土壤分支）。
//!
//! 冷启动那一次（`IniTimeVar → AerosolMasses → SnowAge_grain → SnowAlbedo`）在 colm-init；
//! 这里是每一步的那几处：`netsolar` 的分层吸收（`MOD_NetSolar.F90:236-275`）、地温之后的
//! 再冻结速率 `snofrz`（`MOD_GroundTemperature.F90:419-425`）、`SnowWater_snicar` 的气溶胶
//! （`MOD_SoilSnowHydrology.F90:1649-2100`，bulk 气溶胶、无 MODAL_AER），以及步末 `albland`
//! 里的 `AerosolMasses → SnowAge_grain → SnowAlbedo`（`MOD_Albedo.F90:266-373`）。
//!
//! 雪槽一律按 Fortran `-4:0` 存（活动层在末尾）；`ssno_lyr` 多一个土壤层 `1`。

use anyhow::{ensure, Result};

use crate::{
    age_snow_grains, snicar_ad_rt, snow_aerosol_concentrations, ShortwaveForcing, SnicarAgingTable,
    SnicarIncident, SnicarInput, SnicarOptics, SnowGrainAgingInput,
};

/// 气溶胶物种数（BC 亲水/疏水、OC 亲水/疏水、粉尘 1–4）。
pub const SNICAR_AEROSOL_SPECIES: usize = 8;
/// `forc_aerdep` 的 14 项沉降通量 [kg m-2 s-1]（`MOD_Aerosol.F90` 的读入顺序）。
pub const AEROSOL_DEPOSITION_FIELDS: usize = 14;

/// 融水冲刷系数 `scvng_fct_mlt_*`（`MOD_SoilSnowHydrology.F90:1744-1751`），物种顺序同上。
const MELT_SCAVENGING: [f64; SNICAR_AEROSOL_SPECIES] =
    [0.20, 0.03, 0.20, 0.03, 0.02, 0.02, 0.01, 0.01];

/// SNICAR 打开时每个 patch 额外携带的雪柱状态（时间重启里的量）。
#[derive(Debug, Clone, PartialEq)]
pub struct SnicarColumnState {
    /// `snw_rds(-4:0)` [µm]。
    pub grain_radius_um: [f64; 5],
    /// `mss_bcphi/bcpho/ocphi/ocpho/dst1..4(-4:0)` [kg m-2]，`[slot][species]`。
    pub aerosol_mass_kg_m2: [[f64; SNICAR_AEROSOL_SPECIES]; 5],
    /// `ssno_lyr(band, rtyp, -4:1)`，存成 `[band][direct/diffuse][slot]`。
    pub layer_absorption: [[[f64; 6]; 2]; 2],
    /// 本步的 `snofrz(-4:0)` [kg m-2 s-1]：步首清零、地温相变后写入，步末的雪粒老化读它。
    /// 不随雪层合并/分裂搬动（上游按槽位传整根数组）。
    pub refreezing_kg_m2_s: [f64; 5],
}

/// 两张只读表（运行期加载一次）。
#[derive(Debug, Clone, Copy)]
pub struct SnicarTables<'a> {
    pub optics: &'a SnicarOptics,
    pub aging: &'a SnicarAgingTable,
}

/// 每步的 SNICAR 输入：表与本步的气溶胶沉降（`DEF_Aerosol_Readin = .false.` 时全 0）。
#[derive(Debug, Clone, Copy)]
pub struct SnicarStepInput<'a> {
    pub tables: SnicarTables<'a>,
    pub aerosol_deposition_kg_m2_s: [f64; AEROSOL_DEPOSITION_FIELDS],
}

/// `netsolar` 的 SNICAR 段：把 `ssno_lyr` 按（调整过的）`ssno` 重标，算出各层吸收
/// `sabg_snow_lyr(-4:1)`，并把土壤那一层并进 `sabg_soil`。
///
/// 只在有入射短波时调用（上游整段在 `forc_sols+... > 0` 之内，夜间 `sabg_snow_lyr = 0`、
/// `ssno_lyr` 不动）。GIMPLE：每个 `(band, rtyp)` 顺序求和，`(x*ssno)/sum`；
/// `sabg_snow_lyr = FMA(solld, (2,2), FMA(soll, (2,1), FMA(sols, (1,1), solsd*(1,2))))`，
/// 再乘 `fsno`（不融合）。
pub fn snicar_net_solar(
    layer_absorption: &mut [[[f64; 6]; 2]; 2],
    snow_absorption: [[f64; 2]; 2],
    forcing: ShortwaveForcing,
    snow_fraction: f64,
    soil_absorbed_w_m2: &mut f64,
    snow_absorbed_w_m2: &mut f64,
) -> [f64; 6] {
    for (band, band_layers) in layer_absorption.iter_mut().enumerate() {
        for (kind, layers) in band_layers.iter_mut().enumerate() {
            let total = layers.iter().fold(0.0, |sum, value| value + sum);
            let target = snow_absorption[band][kind];
            if total > 0.0 {
                for value in layers.iter_mut() {
                    *value = *value * target / total;
                }
            } else {
                layers[5] = target;
            }
        }
    }
    let mut absorbed = [0.0; 6];
    for (slot, value) in absorbed.iter_mut().enumerate() {
        let direct_visible = layer_absorption[0][0][slot];
        let diffuse_visible = layer_absorption[0][1][slot];
        let direct_near_infrared = layer_absorption[1][0][slot];
        let diffuse_near_infrared = layer_absorption[1][1][slot];
        let sum = forcing.direct_visible_w_m2.mul_add(
            direct_visible,
            forcing.diffuse_visible_w_m2 * diffuse_visible,
        );
        let sum = forcing
            .direct_near_infrared_w_m2
            .mul_add(direct_near_infrared, sum);
        let sum = forcing
            .diffuse_near_infrared_w_m2
            .mul_add(diffuse_near_infrared, sum);
        *value = sum * snow_fraction;
    }
    *soil_absorbed_w_m2 += absorbed[5];
    *snow_absorbed_w_m2 -= absorbed[5];
    absorbed[5] = *soil_absorbed_w_m2;
    absorbed
}

/// 地温之后的 `snofrz(j) = max(0, wice - wice_bef)/deltim`，只写 `imelt == 2` 的雪层。
///
/// `packed_*` 是打包列（雪层在前）；`snow_layers` 是本步的活动雪层数。
pub fn snow_refreezing_rate(
    state: &mut SnicarColumnState,
    snow_layers: usize,
    ice_before_kg_m2: &[f64],
    ice_after_kg_m2: &[f64],
    phase_flag: &[i32],
    time_step_seconds: f64,
) {
    let top = 5 - snow_layers;
    for row in 0..snow_layers {
        if phase_flag[row] == 2 {
            state.refreezing_kg_m2_s[top + row] =
                (ice_after_kg_m2[row] - ice_before_kg_m2[row]).max(0.0) / time_step_seconds;
        }
    }
}

/// `SnowWater_snicar` 的气溶胶部分（水分部分与 `snowwater` 相同，由 [`crate::snow_water`] 做）。
///
/// 上游在逐层出流的同一个循环里搬气溶胶；某一层的气溶胶只读这一层**出流之后**的液/冰量
/// 与上一层带下来的量，而这一层的液/冰量此后不再变，所以在 `snow_water` 之后用最终的液冰量
/// 与逐层出流 `layer_drainage_kg_m2`（已乘 1000）补算是等价的。最后把本步沉降加到最上层。
pub fn snicar_snow_water_aerosols(
    state: &mut SnicarColumnState,
    snow_layers: usize,
    liquid_water_kg_m2: &[f64],
    ice_water_kg_m2: &[f64],
    layer_drainage_kg_m2: &[f64],
    deposition_kg_m2_s: &[f64; AEROSOL_DEPOSITION_FIELDS],
    time_step_seconds: f64,
) -> Result<()> {
    ensure!(
        (1..=5).contains(&snow_layers) && layer_drainage_kg_m2.len() == snow_layers,
        "SNICAR snow-water aerosol transport needs one drainage value per active snow layer"
    );
    let top = 5 - snow_layers;
    let mut inflow = [0.0; SNICAR_AEROSOL_SPECIES];
    for (row, &outflow) in layer_drainage_kg_m2.iter().enumerate() {
        let slot = top + row;
        let masses = &mut state.aerosol_mass_kg_m2[slot];
        for (mass, incoming) in masses.iter_mut().zip(inflow) {
            *mass += incoming;
        }
        let mut water = liquid_water_kg_m2[slot] + ice_water_kg_m2[slot];
        if water < 1.0e-30 {
            water = 1.0e-30;
        }
        for (species, mass) in masses.iter_mut().enumerate() {
            let flushed = (outflow * MELT_SCAVENGING[species] * (*mass / water)).min(*mass);
            *mass -= flushed;
            inflow[species] = flushed;
        }
    }
    // bulk 气溶胶（无 MODAL_AER）：BC/OC 的亲水 = 1+3 / 4+6，疏水 = 2 / 5；粉尘湿+干。
    let f = deposition_kg_m2_s;
    let dt = time_step_seconds;
    let masses = &mut state.aerosol_mass_kg_m2[top];
    masses[0] += (f[0] + f[2]) * dt;
    masses[1] += f[1] * dt;
    masses[2] += (f[3] + f[5]) * dt;
    masses[3] += f[4] * dt;
    masses[4] += (f[7] + f[6]) * dt;
    masses[5] += (f[9] + f[8]) * dt;
    masses[6] += (f[11] + f[10]) * dt;
    masses[7] += (f[13] + f[12]) * dt;
    Ok(())
}

/// 步末 `albland` 的 SNICAR 钩子。
///
/// [`Self::before_night_return`] 在上游 `albland` 的夜间返回**之前**执行
/// （`ssno_lyr = 0`、`AerosolMasses`、`SnowAge_grain` 每步都跑）；
/// [`Self::snow_albedo`] 只在白天且 `scv > 0` 时调用（`SnowAlbedo`，只有反馈那两次
/// `SNICAR_AD_RT`，`use_snicar_frc = .false.`）。
pub struct SnicarAlbedoHook<'a> {
    pub tables: SnicarTables<'a>,
    pub state: &'a mut SnicarColumnState,
    pub time_step_seconds: f64,
    /// `snl` 的绝对值。
    pub snow_layers: usize,
    /// `dz_soisno(-4:1)`：五个雪槽加第一层土。
    pub thickness_m: [f64; 6],
    /// `t_soisno(-4:1)`。
    pub temperature_k: [f64; 6],
    pub liquid_water_kg_m2: [f64; 5],
    pub ice_water_kg_m2: [f64; 5],
    pub snow_water_equivalent_kg_m2: f64,
    /// 本步落到地面的雪 `pg_snow` [kg m-2 s-1]。
    pub snowfall_kg_m2_s: f64,
    /// `t_grnd`。
    pub ground_temperature_k: f64,
    /// `forc_t`。
    pub air_temperature_k: f64,
    /// `AerosolMasses` 算出的浓度，交给 `SnowAlbedo`。
    concentration: [[f64; SNICAR_AEROSOL_SPECIES]; 5],
}

impl<'a> SnicarAlbedoHook<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tables: SnicarTables<'a>,
        state: &'a mut SnicarColumnState,
        time_step_seconds: f64,
        snow_layers: usize,
        thickness_m: [f64; 6],
        temperature_k: [f64; 6],
        liquid_water_kg_m2: [f64; 5],
        ice_water_kg_m2: [f64; 5],
        snow_water_equivalent_kg_m2: f64,
        snowfall_kg_m2_s: f64,
        ground_temperature_k: f64,
        air_temperature_k: f64,
    ) -> Self {
        Self {
            tables,
            state,
            time_step_seconds,
            snow_layers,
            thickness_m,
            temperature_k,
            liquid_water_kg_m2,
            ice_water_kg_m2,
            snow_water_equivalent_kg_m2,
            snowfall_kg_m2_s,
            ground_temperature_k,
            air_temperature_k,
            concentration: [[0.0; SNICAR_AEROSOL_SPECIES]; 5],
        }
    }

    /// `ssno_lyr = 0` → `AerosolMasses`（不封顶）→ `SnowAge_grain`。
    pub fn before_night_return(&mut self, snow_fraction: f64) -> Result<()> {
        self.state.layer_absorption = [[[0.0; 6]; 2]; 2];
        self.concentration = snow_aerosol_concentrations(
            self.snow_layers,
            self.time_step_seconds,
            None,
            &self.ice_water_kg_m2,
            &self.liquid_water_kg_m2,
            &mut self.state.grain_radius_um,
            &mut self.state.aerosol_mass_kg_m2,
        )?;
        age_snow_grains(
            self.tables.aging,
            SnowGrainAgingInput {
                timestep_seconds: self.time_step_seconds,
                snow_layers: self.snow_layers,
                thickness_m: &self.thickness_m,
                snowfall_kg_m2_s: self.snowfall_kg_m2_s,
                snowcap_ice_kg_m2_s: 0.0,
                refreezing_kg_m2_s: &self.state.refreezing_kg_m2_s,
                snow_capping: false,
                snow_fraction,
                snow_water_equivalent_kg_m2: self.snow_water_equivalent_kg_m2,
                liquid_water_kg_m2: &self.liquid_water_kg_m2,
                ice_water_kg_m2: &self.ice_water_kg_m2,
                temperature_k: &self.temperature_k,
                air_temperature_k: self.air_temperature_k,
            },
            &mut self.state.grain_radius_um,
        )
    }

    /// `SnowAlbedo`：下垫面用土壤的漫射反照率（`albsfc = albsoi`，直射与漫射两次都是），
    /// 返回 `albsno[band][direct/diffuse]` 并写 `ssno_lyr`；`snl == 0` 时临时雪层并进土壤层。
    pub fn snow_albedo(
        &mut self,
        cosine_zenith: f64,
        soil_diffuse_albedo: [f64; 2],
    ) -> Result<[[f64; 2]; 2]> {
        let layers = self.snow_layers;
        let top = 5 - layers;
        let mut input = SnicarInput {
            incident: SnicarIncident::Direct,
            cosine_zenith,
            snow_water_equivalent_kg_m2: self.snow_water_equivalent_kg_m2,
            active_layers: layers,
            liquid_water_kg_m2: [0.0; 5],
            ice_water_kg_m2: [0.0; 5],
            snow_radius_microns: [0; 5],
            aerosol_mass_concentration: [[0.0; SNICAR_AEROSOL_SPECIES]; 5],
            underlying_albedo_5band: [
                soil_diffuse_albedo[0],
                soil_diffuse_albedo[1],
                soil_diffuse_albedo[1],
                soil_diffuse_albedo[1],
                soil_diffuse_albedo[1],
            ],
        };
        // `snw_rds_in = nint(snw_rds)` 取全部五个槽；`mss_cnc_aer_in_fdb` 只放 BC 与粉尘
        // （`DO_SNO_OC = .false.`，OC 两列保持 0）。
        for (row, slot) in (top..5).enumerate() {
            input.liquid_water_kg_m2[row] = self.liquid_water_kg_m2[slot];
            input.ice_water_kg_m2[row] = self.ice_water_kg_m2[slot];
            input.snow_radius_microns[row] = self.state.grain_radius_um[slot].round() as i32;
            let mut concentration = self.concentration[slot];
            concentration[2] = 0.0;
            concentration[3] = 0.0;
            input.aerosol_mass_concentration[row] = concentration;
        }
        if layers == 0 {
            // 无雪层时 SNICAR 用第 0 槽做临时层。
            input.snow_radius_microns[0] = self.state.grain_radius_um[4].round() as i32;
        }
        let mut albedo = [[1.0; 2]; 2];
        let absorption = &mut self.state.layer_absorption;
        for (incident, index) in [(SnicarIncident::Direct, 0), (SnicarIncident::Diffuse, 1)] {
            input.incident = incident;
            let result = snicar_ad_rt(self.tables.optics, &input)?;
            for band in 0..2 {
                albedo[band][index] = result.albedo_broadband[band];
                for row in 0..layers {
                    absorption[band][index][top + row] = result.absorbed_broadband[row][band];
                }
                absorption[band][index][5] =
                    result.absorbed_broadband[result.absorption_rows - 1][band];
                if result.temporary_snow_layer {
                    absorption[band][index][5] += result.absorbed_broadband[0][band];
                }
            }
        }
        Ok(albedo)
    }
}

#[cfg(test)]
#[path = "snicar_column_tests.rs"]
mod snicar_column_tests;

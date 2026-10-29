//! 陆面材料运移（`main/MATERIAL/`，上游 7d2fca93 前叫 `main/TRACER/`）。
//!
//! 材料只记账、不改宿主（第 477 轮解耦之后）：每个过程读宿主本步的水量快照，
//! 按同样的路径搬运材料质量。

pub mod conservation;
pub mod descriptor;
pub mod evap_limit;
pub mod evapo;
pub mod frac;
pub mod hist;
pub mod precip;
pub mod snow;
pub mod soil_water;
pub mod special_patches;
pub mod state;
pub mod step;

pub use descriptor::{
    delta_to_ratio, ReactionMode, StateOwner, TracerDescriptor, TracerFamily, TracerNamelist,
    TracerParamFiles, TracerParameterOverrides, TracerSet, DESCRIPTOR_IDENTITY_WIDTH, TRC_TINY,
    TRC_WATER_MIN_FOR_RATIO,
};
pub use precip::{tracer_precip, PrecipInput};
pub use state::{
    soisno_slot, EvapKind, PatchTracerState, TracerAccumulators, TracerColdStart, TracerPools,
    TracerStep, WaterAccumulators, WaterInventory, MAX_SNOW_LAYERS, SOIL_LAYERS, SOISNO_LAYERS,
};

/// 材料分馏的全局开关与参数（`&nl_colm_isotope_parameter` 的 `DEF_ISO_*`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TracerPhysics {
    /// `DEF_ISO_USE_FRACTIONATION`。
    pub fractionation: bool,
    /// `DEF_ISO_KINETIC_SCHEME`。
    pub kinetic_scheme: frac::KineticScheme,
    /// `DEF_ISO_ICE_SUPERSAT_SLOPE`。
    pub ice_supersat_slope: f64,
    /// `DEF_ISO_CG_RELHUM_MAX`。
    pub cg_relhum_max: f64,
    /// `DEF_ISO_OPEN_WATER_KINETIC`。
    pub open_water_kinetic: frac::OpenWaterKinetic,
    /// `DEF_ISO_NSS_LEAF_WATER_PER_LAI`、`_PATH_LENGTH`、`_RB`。
    pub nss_leaf_water_per_lai: f64,
    pub nss_leaf_path_length: f64,
    pub nss_leaf_rb: f64,
}

impl Default for TracerPhysics {
    /// `MOD_MT_Namelist.F90` 里 `DEF_ISO_*` 的声明默认值。
    fn default() -> Self {
        Self {
            fractionation: false,
            kinetic_scheme: frac::KineticScheme::Merlivat1978,
            ice_supersat_slope: 0.003,
            cg_relhum_max: 0.99,
            open_water_kinetic: frac::OpenWaterKinetic::Mj79,
            nss_leaf_water_per_lai: 0.12,
            nss_leaf_path_length: 0.01,
            nss_leaf_rb: 100.0,
        }
    }
}

impl TracerPhysics {
    /// `tracer_fractionation_active(itrc)`：开关打开、是同位素、且在分馏注册表里（O18/HDO）。
    pub fn fractionation_active(&self, tracer: &TracerDescriptor) -> bool {
        self.fractionation && frac::IsotopeSpecies::find(tracer).is_some()
    }
}

//! 陆面示踪物（`main/TRACER/`）。
//!
//! 示踪物只记账、不改宿主（第 477 轮解耦之后）：每个过程读宿主本步的水量快照，
//! 按同样的路径搬运示踪物质量。

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
    TracerParameterOverrides, TracerSet, DESCRIPTOR_IDENTITY_WIDTH, TRC_TINY,
    TRC_WATER_MIN_FOR_RATIO,
};
pub use precip::{tracer_precip, PrecipInput};
pub use state::{
    soisno_slot, EvapKind, PatchTracerState, TracerAccumulators, TracerColdStart, TracerPools,
    TracerStep, WaterAccumulators, WaterInventory, MAX_SNOW_LAYERS, SOIL_LAYERS, SOISNO_LAYERS,
};

/// 示踪物物理的全局开关与分馏参数（`DEF_TRACER_USE_FRACTIONATION` 等）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TracerPhysics {
    /// `DEF_TRACER_USE_FRACTIONATION`。
    pub fractionation: bool,
    /// `DEF_TRACER_KINETIC_SCHEME`。
    pub kinetic_scheme: frac::KineticScheme,
    /// `DEF_TRACER_ICE_SUPERSAT_SLOPE`。
    pub ice_supersat_slope: f64,
    /// `DEF_TRACER_CG_RELHUM_MAX`。
    pub cg_relhum_max: f64,
    /// `DEF_TRACER_OPEN_WATER_KINETIC`。
    pub open_water_kinetic: frac::OpenWaterKinetic,
    /// `DEF_TRACER_NSS_LEAF_WATER_PER_LAI`、`_PATH_LENGTH`、`_RB`。
    pub nss_leaf_water_per_lai: f64,
    pub nss_leaf_path_length: f64,
    pub nss_leaf_rb: f64,
}

impl Default for TracerPhysics {
    /// `MOD_Namelist.F90:421-442` 的默认值。
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

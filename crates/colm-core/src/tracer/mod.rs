//! 陆面示踪物（`main/TRACER/`）。
//!
//! 示踪物只记账、不改宿主（第 477 轮解耦之后）：每个过程读宿主本步的水量快照，
//! 按同样的路径搬运示踪物质量。

pub mod conservation;
pub mod descriptor;
pub mod evap_limit;
pub mod evapo;
pub mod hist;
pub mod precip;
pub mod snow;
pub mod soil_water;
pub mod special_patches;
pub mod state;

pub use descriptor::{
    delta_to_ratio, ReactionMode, StateOwner, TracerDescriptor, TracerFamily, TracerNamelist,
    TracerParameterOverrides, TracerSet, DESCRIPTOR_IDENTITY_WIDTH, TRC_TINY,
    TRC_WATER_MIN_FOR_RATIO,
};
pub use state::{
    soisno_slot, EvapKind, PatchTracerState, TracerAccumulators, TracerColdStart, TracerPools,
    TracerStep, WaterAccumulators, WaterInventory, MAX_SNOW_LAYERS, SOISNO_LAYERS, SOIL_LAYERS,
};
pub use precip::{tracer_precip, PrecipInput};

/// 示踪物物理的全局开关（`DEF_TRACER_USE_FRACTIONATION` 等）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TracerPhysics {
    /// `DEF_TRACER_USE_FRACTIONATION`。
    pub fractionation: bool,
}

impl TracerPhysics {
    /// `tracer_fractionation_active(itrc)`：开关打开且是同位素。
    ///
    /// 上游还要求该同位素在分馏注册表里（O18/HDO）；分馏物理属于 T2，现在各过程在
    /// 分馏生效时拒绝运行。
    pub fn fractionation_active(&self, tracer: &TracerDescriptor) -> bool {
        self.fractionation && tracer.is_isotope()
    }
}

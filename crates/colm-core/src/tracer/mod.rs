//! 陆面示踪物（`main/TRACER/`）。
//!
//! 示踪物只记账、不改宿主（第 477 轮解耦之后）：每个过程读宿主本步的水量快照，
//! 按同样的路径搬运示踪物质量。

pub mod conservation;
pub mod descriptor;
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

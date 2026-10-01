//! 陆面示踪物（`main/TRACER/`）。
//!
//! 示踪物只记账、不改宿主（第 477 轮解耦之后）：每个过程读宿主本步的水量快照，
//! 按同样的路径搬运示踪物质量。

pub mod descriptor;

pub use descriptor::{
    delta_to_ratio, ReactionMode, StateOwner, TracerDescriptor, TracerFamily, TracerNamelist,
    TracerParameterOverrides, TracerSet, DESCRIPTOR_IDENTITY_WIDTH, TRC_TINY,
    TRC_WATER_MIN_FOR_RATIO,
};

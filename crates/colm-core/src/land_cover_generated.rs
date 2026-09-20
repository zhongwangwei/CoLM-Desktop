//! 地类常量表，由 `cargo run -p xtask -- gen-landcover` 从
//! `vendor/CoLM202X/main/MOD_Const_LC.F90` 生成。**不要手改**：
//! `crates/colm-core/tests/drift_landcover.rs` 会重新生成并逐字节比对。
//!
//! 取值是 `Init_LC_Const` 里那些 `*_igbp` / `*_usgs` 常量的**字面副本**，
//! 不含 `vmax25 * 1.e-6` 这类赋值处的缩放 —— 缩放写在访问器里，这样表
//! 与上游源码可以直接对照。`#ifdef LULC_USGS` 的两支都在这里，选哪一支
//! 由调用方决定。

pub const IGBP_CLASSES: usize = 17;
pub const USGS_CLASSES: usize = 24;

#[rustfmt::skip]
pub const BETA_IGBP: [f64; IGBP_CLASSES] = [
    -1.623, -1.623, -1.681, -1.681, -1.652, -1.336, -1.909, -1.582,
    -1.798, -1.359, -1.359, -1.796, -1.757, -1.796, -1.000, -2.261,
    -1.000,
];

#[rustfmt::skip]
pub const BETA_USGS: [f64; USGS_CLASSES] = [
    -1.757, -1.835, -1.757, -1.796, -1.577, -1.738, -1.359, -3.245,
    -2.302, -1.654, -1.681, -1.681, -1.632, -1.632, -1.656, -1.000,
    -1.359, -1.656, -2.051, -2.621, -2.621, -2.621, -2.621, -1.000,
];

#[rustfmt::skip]
pub const BINTER_IGBP: [f64; IGBP_CLASSES] = [
    0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01,
    0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01,
    0.01,
];

#[rustfmt::skip]
pub const BINTER_USGS: [f64; USGS_CLASSES] = [
    0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01,
    0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01,
    0.01, 0.01, 0.01, 0.04, 0.04, 0.04, 0.04, 0.04,
];

#[rustfmt::skip]
pub const C3C4_IGBP: [i32; IGBP_CLASSES] = [
    1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1,
    1,
];

#[rustfmt::skip]
pub const C3C4_USGS: [i32; USGS_CLASSES] = [
    1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 0, 0, 0, 0, 0,
];

#[rustfmt::skip]
pub const CHIL_IGBP: [f64; IGBP_CLASSES] = [
    0.010, 0.100, 0.010, 0.250, 0.125, 0.010, 0.010, 0.010,
    0.010, -0.300, 0.100, -0.300, 0.010, -0.300, 0.010, 0.010,
    0.010,
];

#[rustfmt::skip]
pub const CHIL_USGS: [f64; USGS_CLASSES] = [
    -0.300, -0.300, -0.300, -0.300, -0.300, -0.300, -0.300, 0.010,
    0.010, -0.300, 0.250, 0.010, 0.100, 0.010, 0.125, -0.300,
    -0.300, 0.100, 0.010, -0.300, -0.300, -0.300, -0.300, -0.300,
];

#[rustfmt::skip]
pub const CK0_IGBP: [f64; IGBP_CLASSES] = [
    3.95, 3.95, 3.95, 3.95, 3.95, 3.95, 3.95, 3.95,
    3.95, 3.95, 3.95, 3.95, 3.95, 3.95, 3.95, 3.95,
    3.95,
];

#[rustfmt::skip]
pub const CK0_USGS: [f64; USGS_CLASSES] = [
    0.0, 3.95, 3.95, 3.95, 3.95, 3.95, 3.95, 3.95,
    3.95, 3.95, 3.95, 3.95, 3.95, 3.95, 3.95, 0.0,
    3.95, 3.95, 0.0, 3.95, 3.95, 3.95, 0.0, 0.0,
];

#[rustfmt::skip]
pub const D50_IGBP: [f64; IGBP_CLASSES] = [
    15.0, 15.0, 16.0, 16.0, 15.5, 19.0, 28.0, 18.5,
    28.0, 9.0, 9.0, 22.0, 23.0, 22.0, 1.0, 9.0,
    1.0,
];

#[rustfmt::skip]
pub const D50_USGS: [f64; USGS_CLASSES] = [
    23.0, 21.0, 23.0, 22.0, 15.7, 19.0, 9.3, 47.0,
    28.2, 21.7, 16.0, 16.0, 15.0, 15.0, 15.5, 1.0,
    9.3, 15.5, 27.0, 9.0, 9.0, 9.0, 9.0, 1.0,
];

#[rustfmt::skip]
pub const DISPLAR_IGBP: [f64; IGBP_CLASSES] = [
    0.667, 0.667, 0.667, 0.667, 0.667, 0.667, 0.667, 0.667,
    0.667, 0.667, 0.667, 0.667, 0.667, 0.667, 0.667, 0.667,
    0.667,
];

#[rustfmt::skip]
pub const DISPLAR_USGS: [f64; USGS_CLASSES] = [
    0.667, 0.667, 0.667, 0.667, 0.667, 0.667, 0.667, 0.667,
    0.667, 0.667, 0.667, 0.667, 0.667, 0.667, 0.667, 0.667,
    0.667, 0.667, 0.667, 0.667, 0.667, 0.667, 0.667, 0.667,
];

#[rustfmt::skip]
pub const EFFCON_IGBP: [f64; IGBP_CLASSES] = [
    0.08, 0.08, 0.08, 0.08, 0.08, 0.08, 0.08, 0.08,
    0.08, 0.08, 0.08, 0.08, 0.08, 0.08, 0.08, 0.08,
    0.08,
];

#[rustfmt::skip]
pub const EFFCON_USGS: [f64; USGS_CLASSES] = [
    0.08, 0.08, 0.08, 0.08, 0.08, 0.08, 0.08, 0.08,
    0.08, 0.08, 0.08, 0.08, 0.08, 0.08, 0.08, 0.08,
    0.08, 0.08, 0.08, 0.05, 0.05, 0.05, 0.05, 0.05,
];

#[rustfmt::skip]
pub const EXTKN_IGBP: [f64; IGBP_CLASSES] = [
    0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5,
    0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5,
    0.5,
];

#[rustfmt::skip]
pub const EXTKN_USGS: [f64; USGS_CLASSES] = [
    0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5,
    0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5,
    0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5,
];

#[rustfmt::skip]
pub const FVEG0_IGBP: [f64; IGBP_CLASSES] = [
    1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
    1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
    1.0,
];

#[rustfmt::skip]
pub const FVEG0_USGS: [f64; USGS_CLASSES] = [
    1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
    1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
    1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
];

#[rustfmt::skip]
pub const G0_IGBP: [f64; IGBP_CLASSES] = [
    100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0,
    100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0,
    100.0,
];

#[rustfmt::skip]
pub const G0_USGS: [f64; USGS_CLASSES] = [
    100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0,
    100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0,
    100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0, 100.0,
];

#[rustfmt::skip]
pub const G1_IGBP: [f64; IGBP_CLASSES] = [
    9.0, 9.0, 9.0, 9.0, 9.0, 9.0, 9.0, 9.0,
    9.0, 9.0, 9.0, 9.0, 9.0, 9.0, 9.0, 9.0,
    9.0,
];

#[rustfmt::skip]
pub const G1_USGS: [f64; USGS_CLASSES] = [
    4.0, 4.0, 4.0, 4.0, 4.0, 4.0, 4.0, 4.0,
    4.0, 4.0, 4.0, 4.0, 4.0, 4.0, 4.0, 4.0,
    4.0, 4.0, 4.0, 4.0, 4.0, 4.0, 4.0, 4.0,
];

#[rustfmt::skip]
pub const GRADM_IGBP: [f64; IGBP_CLASSES] = [
    9.0, 9.0, 9.0, 9.0, 9.0, 9.0, 9.0, 9.0,
    9.0, 9.0, 9.0, 9.0, 9.0, 9.0, 9.0, 9.0,
    9.0,
];

#[rustfmt::skip]
pub const GRADM_USGS: [f64; USGS_CLASSES] = [
    9.0, 9.0, 9.0, 9.0, 9.0, 9.0, 9.0, 9.0,
    9.0, 9.0, 9.0, 9.0, 9.0, 9.0, 9.0, 9.0,
    9.0, 9.0, 9.0, 4.0, 4.0, 4.0, 4.0, 4.0,
];

#[rustfmt::skip]
pub const HBOT0_IGBP: [f64; IGBP_CLASSES] = [
    1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0,
    0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
    0.0,
];

#[rustfmt::skip]
pub const HBOT0_USGS: [f64; USGS_CLASSES] = [
    0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
    0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0,
    0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
];

#[rustfmt::skip]
pub const HHTI_IGBP: [f64; IGBP_CLASSES] = [
    303.0, 313.0, 303.0, 311.0, 307.0, 308.0, 313.0, 313.0,
    313.0, 308.0, 313.0, 308.0, 308.0, 308.0, 303.0, 313.0,
    308.0,
];

#[rustfmt::skip]
pub const HHTI_USGS: [f64; USGS_CLASSES] = [
    308.0, 308.0, 308.0, 308.0, 308.0, 308.0, 308.0, 313.0,
    313.0, 308.0, 311.0, 303.0, 313.0, 303.0, 307.0, 308.0,
    308.0, 313.0, 313.0, 313.0, 313.0, 313.0, 313.0, 308.0,
];

#[rustfmt::skip]
pub const HLTI_IGBP: [f64; IGBP_CLASSES] = [
    278.0, 288.0, 278.0, 283.0, 281.0, 281.0, 288.0, 288.0,
    288.0, 281.0, 283.0, 281.0, 281.0, 281.0, 278.0, 288.0,
    281.0,
];

#[rustfmt::skip]
pub const HLTI_USGS: [f64; USGS_CLASSES] = [
    281.0, 281.0, 281.0, 281.0, 281.0, 281.0, 281.0, 283.0,
    283.0, 281.0, 283.0, 278.0, 288.0, 278.0, 281.0, 281.0,
    281.0, 288.0, 283.0, 288.0, 288.0, 288.0, 288.0, 281.0,
];

#[rustfmt::skip]
pub const HTOP0_IGBP: [f64; IGBP_CLASSES] = [
    17.0, 35.0, 17.0, 20.0, 20.0, 0.5, 0.5, 1.0,
    0.5, 0.5, 0.5, 0.5, 1.0, 0.5, 0.5, 0.5,
    0.5,
];

#[rustfmt::skip]
pub const HTOP0_USGS: [f64; USGS_CLASSES] = [
    1.0, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5,
    0.5, 0.5, 20.0, 17.0, 35.0, 17.0, 20.0, 0.5,
    0.5, 17.0, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5,
];

#[rustfmt::skip]
pub const KMAX_ROOT0_IGBP: [f64; IGBP_CLASSES] = [
    2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008,
    2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008,
    2.0e-008,
];

#[rustfmt::skip]
pub const KMAX_ROOT0_USGS: [f64; USGS_CLASSES] = [
    0.0, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008,
    2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 0.0,
    2.0e-008, 2.0e-008, 0.0, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008,
];

#[rustfmt::skip]
pub const KMAX_SHA0_IGBP: [f64; IGBP_CLASSES] = [
    2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008,
    2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008,
    2.0e-008,
];

#[rustfmt::skip]
pub const KMAX_SHA0_USGS: [f64; USGS_CLASSES] = [
    0.0, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008,
    2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 0.0,
    2.0e-008, 2.0e-008, 0.0, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008,
];

#[rustfmt::skip]
pub const KMAX_SUN0_IGBP: [f64; IGBP_CLASSES] = [
    2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008,
    2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008,
    2.0e-008,
];

#[rustfmt::skip]
pub const KMAX_SUN0_USGS: [f64; USGS_CLASSES] = [
    0.0, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008,
    2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 0.0,
    2.0e-008, 2.0e-008, 0.0, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008,
];

#[rustfmt::skip]
pub const KMAX_XYL0_IGBP: [f64; IGBP_CLASSES] = [
    2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008,
    2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008,
    2.0e-008,
];

#[rustfmt::skip]
pub const KMAX_XYL0_USGS: [f64; USGS_CLASSES] = [
    0.0, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008,
    2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 0.0,
    2.0e-008, 2.0e-008, 0.0, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008, 2.0e-008,
];

#[rustfmt::skip]
pub const LAMBDA_IGBP: [f64; IGBP_CLASSES] = [
    1000.0, 1000.0, 1000.0, 1000.0, 1000.0, 1000.0, 1000.0, 1000.0,
    1000.0, 1000.0, 1000.0, 1000.0, 1000.0, 1000.0, 1000.0, 1000.0,
    1000.0,
];

#[rustfmt::skip]
pub const LAMBDA_USGS: [f64; USGS_CLASSES] = [
    1000.0, 1000.0, 1000.0, 1000.0, 1000.0, 1000.0, 1000.0, 1000.0,
    1000.0, 1000.0, 1000.0, 1000.0, 1000.0, 1000.0, 1000.0, 1000.0,
    1000.0, 1000.0, 1000.0, 1000.0, 1000.0, 1000.0, 1000.0, 1000.0,
];

#[rustfmt::skip]
pub const PATCHTYPES_IGBP: [i32; IGBP_CLASSES] = [
    0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 2, 0, 1, 0, 3, 0,
    4,
];

#[rustfmt::skip]
pub const PATCHTYPES_USGS: [i32; USGS_CLASSES] = [
    1, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 4,
    2, 2, 0, 0, 0, 0, 0, 3,
];

#[rustfmt::skip]
pub const PSI50_ROOT0_IGBP: [f64; IGBP_CLASSES] = [
    -465000.0, -260000.0, -380000.0, -270000.0, -330000.0, -393333.3, -393333.3, -340000.0,
    -340000.0, -340000.0, -343636.4, -340000.0, -200000.0, -343636.4, -200000.0, -200000.0,
    -200000.0,
];

#[rustfmt::skip]
pub const PSI50_ROOT0_USGS: [f64; USGS_CLASSES] = [
    -200000.0, -340000.0, -340000.0, -340000.0, -340000.0, -343636.4, -340000.0, -393333.3,
    -366666.7, -340000.0, -270000.0, -380000.0, -260000.0, -465000.0, -330000.0, -200000.0,
    -340000.0, -347272.7, -200000.0, -340000.0, -342500.0, -341250.0, -200000.0, -200000.0,
];

#[rustfmt::skip]
pub const PSI50_SHA0_IGBP: [f64; IGBP_CLASSES] = [
    -465000.0, -260000.0, -380000.0, -270000.0, -330000.0, -393333.3, -393333.3, -340000.0,
    -340000.0, -340000.0, -343636.4, -340000.0, -150000.0, -343636.4, -150000.0, -150000.0,
    -150000.0,
];

#[rustfmt::skip]
pub const PSI50_SHA0_USGS: [f64; USGS_CLASSES] = [
    -150000.0, -340000.0, -340000.0, -340000.0, -340000.0, -343636.4, -340000.0, -393333.3,
    -366666.7, -340000.0, -270000.0, -380000.0, -260000.0, -465000.0, -330000.0, -150000.0,
    -340000.0, -347272.7, -150000.0, -340000.0, -342500.0, -341250.0, -150000.0, -150000.0,
];

#[rustfmt::skip]
pub const PSI50_SUN0_IGBP: [f64; IGBP_CLASSES] = [
    -465000.0, -260000.0, -380000.0, -270000.0, -330000.0, -393333.3, -393333.3, -340000.0,
    -340000.0, -340000.0, -343636.4, -340000.0, -150000.0, -343636.4, -150000.0, -150000.0,
    -150000.0,
];

#[rustfmt::skip]
pub const PSI50_SUN0_USGS: [f64; USGS_CLASSES] = [
    -150000.0, -340000.0, -340000.0, -340000.0, -340000.0, -343636.4, -340000.0, -393333.3,
    -366666.7, -340000.0, -270000.0, -380000.0, -260000.0, -465000.0, -330000.0, -150000.0,
    -340000.0, -347272.7, -150000.0, -340000.0, -342500.0, -341250.0, -150000.0, -150000.0,
];

#[rustfmt::skip]
pub const PSI50_XYL0_IGBP: [f64; IGBP_CLASSES] = [
    -465000.0, -260000.0, -380000.0, -270000.0, -330000.0, -393333.3, -393333.3, -340000.0,
    -340000.0, -340000.0, -343636.4, -340000.0, -200000.0, -343636.4, -200000.0, -200000.0,
    -200000.0,
];

#[rustfmt::skip]
pub const PSI50_XYL0_USGS: [f64; USGS_CLASSES] = [
    -200000.0, -340000.0, -340000.0, -340000.0, -340000.0, -343636.4, -340000.0, -393333.3,
    -366666.7, -340000.0, -270000.0, -380000.0, -260000.0, -465000.0, -330000.0, -200000.0,
    -340000.0, -347272.7, -200000.0, -340000.0, -342500.0, -341250.0, -200000.0, -200000.0,
];

#[rustfmt::skip]
pub const RESPCP_IGBP: [f64; IGBP_CLASSES] = [
    0.015, 0.015, 0.015, 0.015, 0.015, 0.015, 0.015, 0.015,
    0.015, 0.015, 0.015, 0.015, 0.015, 0.015, 0.015, 0.015,
    0.015,
];

#[rustfmt::skip]
pub const RESPCP_USGS: [f64; USGS_CLASSES] = [
    0.015, 0.015, 0.015, 0.015, 0.015, 0.015, 0.015, 0.015,
    0.015, 0.015, 0.015, 0.015, 0.015, 0.015, 0.015, 0.015,
    0.015, 0.015, 0.015, 0.025, 0.025, 0.025, 0.025, 0.025,
];

#[rustfmt::skip]
pub const RHOL_NIR_IGBP: [f64; IGBP_CLASSES] = [
    0.350, 0.450, 0.350, 0.450, 0.400, 0.450, 0.450, 0.580,
    0.580, 0.580, 0.450, 0.580, 0.450, 0.580, 0.450, 0.450,
    0.580,
];

#[rustfmt::skip]
pub const RHOL_NIR_USGS: [f64; USGS_CLASSES] = [
    0.580, 0.580, 0.580, 0.580, 0.580, 0.580, 0.580, 0.450,
    0.450, 0.580, 0.450, 0.350, 0.450, 0.350, 0.400, 0.580,
    0.580, 0.450, 0.450, 0.580, 0.580, 0.580, 0.580, 0.580,
];

#[rustfmt::skip]
pub const RHOL_VIS_IGBP: [f64; IGBP_CLASSES] = [
    0.070, 0.100, 0.070, 0.100, 0.070, 0.105, 0.105, 0.105,
    0.105, 0.105, 0.105, 0.105, 0.105, 0.105, 0.105, 0.105,
    0.105,
];

#[rustfmt::skip]
pub const RHOL_VIS_USGS: [f64; USGS_CLASSES] = [
    0.105, 0.105, 0.105, 0.105, 0.105, 0.105, 0.105, 0.100,
    0.100, 0.105, 0.100, 0.070, 0.100, 0.070, 0.070, 0.105,
    0.105, 0.100, 0.100, 0.105, 0.105, 0.105, 0.105, 0.105,
];

#[rustfmt::skip]
pub const RHOS_NIR_IGBP: [f64; IGBP_CLASSES] = [
    0.390, 0.390, 0.390, 0.390, 0.390, 0.390, 0.390, 0.390,
    0.390, 0.580, 0.390, 0.580, 0.390, 0.580, 0.390, 0.390,
    0.580,
];

#[rustfmt::skip]
pub const RHOS_NIR_USGS: [f64; USGS_CLASSES] = [
    0.580, 0.580, 0.580, 0.580, 0.580, 0.580, 0.580, 0.390,
    0.390, 0.580, 0.390, 0.390, 0.390, 0.390, 0.390, 0.580,
    0.580, 0.390, 0.390, 0.580, 0.580, 0.580, 0.580, 0.580,
];

#[rustfmt::skip]
pub const RHOS_VIS_IGBP: [f64; IGBP_CLASSES] = [
    0.160, 0.160, 0.160, 0.160, 0.160, 0.160, 0.160, 0.160,
    0.160, 0.360, 0.160, 0.360, 0.160, 0.360, 0.160, 0.160,
    0.160,
];

#[rustfmt::skip]
pub const RHOS_VIS_USGS: [f64; USGS_CLASSES] = [
    0.360, 0.360, 0.360, 0.360, 0.360, 0.360, 0.360, 0.160,
    0.160, 0.360, 0.160, 0.160, 0.160, 0.160, 0.160, 0.360,
    0.360, 0.160, 0.160, 0.360, 0.360, 0.360, 0.360, 0.360,
];

#[rustfmt::skip]
pub const ROOTA_IGBP: [f64; IGBP_CLASSES] = [
    6.706, 7.344, 7.066, 5.990, 4.453, 6.326, 7.718, 7.604,
    8.235, 10.740, 10.740, 5.558, 5.558, 5.558, 10.740, 4.372,
    10.740,
];

#[rustfmt::skip]
pub const ROOTA_USGS: [f64; USGS_CLASSES] = [
    5.558, 5.558, 5.558, 5.558, 8.149, 5.558, 10.740, 7.022,
    8.881, 7.920, 5.990, 7.066, 7.344, 6.706, 4.453, 10.740,
    10.740, 4.453, 8.992, 8.992, 8.992, 8.992, 4.372, 10.740,
];

#[rustfmt::skip]
pub const ROOTB_IGBP: [f64; IGBP_CLASSES] = [
    2.175, 1.303, 1.953, 1.955, 1.631, 1.567, 1.262, 2.300,
    1.627, 2.608, 2.608, 2.614, 2.614, 2.614, 2.608, 0.978,
    2.608,
];

#[rustfmt::skip]
pub const ROOTB_USGS: [f64; USGS_CLASSES] = [
    2.614, 2.614, 2.614, 2.614, 2.611, 2.614, 2.608, 1.415,
    2.012, 1.964, 1.955, 1.953, 1.303, 2.175, 1.631, 2.608,
    2.608, 1.631, 8.992, 8.992, 8.992, 8.992, 0.978, 2.608,
];

#[rustfmt::skip]
pub const SAI0_IGBP: [f64; IGBP_CLASSES] = [
    2.0, 2.0, 2.0, 2.0, 2.0, 0.5, 0.5, 0.5,
    0.5, 0.2, 0.2, 0.2, 0.2, 0.2, 0.0, 0.0,
    0.0,
];

#[rustfmt::skip]
pub const SAI0_USGS: [f64; USGS_CLASSES] = [
    0.2, 0.2, 0.3, 0.3, 0.5, 0.5, 1.0, 0.5,
    1.0, 0.5, 2.0, 2.0, 2.0, 2.0, 2.0, 0.0,
    0.2, 2.0, 0.2, 0.2, 0.2, 0.2, 0.0, 0.0,
];

#[rustfmt::skip]
pub const SHTI_IGBP: [f64; IGBP_CLASSES] = [
    0.3, 0.3, 0.3, 0.3, 0.3, 0.3, 0.3, 0.3,
    0.3, 0.3, 0.3, 0.3, 0.3, 0.3, 0.3, 0.3,
    0.3,
];

#[rustfmt::skip]
pub const SHTI_USGS: [f64; USGS_CLASSES] = [
    0.3, 0.3, 0.3, 0.3, 0.3, 0.3, 0.3, 0.3,
    0.3, 0.3, 0.3, 0.3, 0.3, 0.3, 0.3, 0.3,
    0.3, 0.3, 0.3, 0.3, 0.3, 0.3, 0.3, 0.3,
];

#[rustfmt::skip]
pub const SLTI_IGBP: [f64; IGBP_CLASSES] = [
    0.2, 0.2, 0.2, 0.2, 0.2, 0.2, 0.2, 0.2,
    0.2, 0.2, 0.2, 0.2, 0.2, 0.2, 0.2, 0.2,
    0.2,
];

#[rustfmt::skip]
pub const SLTI_USGS: [f64; USGS_CLASSES] = [
    0.2, 0.2, 0.2, 0.2, 0.2, 0.2, 0.2, 0.2,
    0.2, 0.2, 0.2, 0.2, 0.2, 0.2, 0.2, 0.2,
    0.2, 0.2, 0.2, 0.2, 0.2, 0.2, 0.2, 0.2,
];

#[rustfmt::skip]
pub const SQRTDI_IGBP: [f64; IGBP_CLASSES] = [
    5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0,
    5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0,
    5.0,
];

#[rustfmt::skip]
pub const SQRTDI_USGS: [f64; USGS_CLASSES] = [
    5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0,
    5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0,
    5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0,
];

#[rustfmt::skip]
pub const TAUL_NIR_IGBP: [f64; IGBP_CLASSES] = [
    0.100, 0.250, 0.100, 0.250, 0.150, 0.250, 0.250, 0.250,
    0.250, 0.250, 0.250, 0.250, 0.250, 0.250, 0.250, 0.250,
    0.250,
];

#[rustfmt::skip]
pub const TAUL_NIR_USGS: [f64; USGS_CLASSES] = [
    0.250, 0.250, 0.250, 0.250, 0.250, 0.250, 0.250, 0.250,
    0.250, 0.250, 0.250, 0.100, 0.250, 0.100, 0.150, 0.250,
    0.250, 0.250, 0.250, 0.250, 0.250, 0.250, 0.250, 0.250,
];

#[rustfmt::skip]
pub const TAUL_VIS_IGBP: [f64; IGBP_CLASSES] = [
    0.050, 0.050, 0.050, 0.050, 0.050, 0.050, 0.050, 0.050,
    0.050, 0.070, 0.050, 0.070, 0.050, 0.070, 0.050, 0.050,
    0.050,
];

#[rustfmt::skip]
pub const TAUL_VIS_USGS: [f64; USGS_CLASSES] = [
    0.070, 0.070, 0.070, 0.070, 0.070, 0.070, 0.070, 0.070,
    0.070, 0.070, 0.050, 0.050, 0.050, 0.050, 0.050, 0.070,
    0.070, 0.050, 0.070, 0.070, 0.070, 0.070, 0.070, 0.070,
];

#[rustfmt::skip]
pub const TAUS_NIR_IGBP: [f64; IGBP_CLASSES] = [
    0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001,
    0.001, 0.380, 0.001, 0.380, 0.001, 0.380, 0.001, 0.001,
    0.001,
];

#[rustfmt::skip]
pub const TAUS_NIR_USGS: [f64; USGS_CLASSES] = [
    0.380, 0.380, 0.380, 0.380, 0.380, 0.380, 0.380, 0.001,
    0.001, 0.380, 0.001, 0.001, 0.001, 0.001, 0.001, 0.380,
    0.380, 0.001, 0.001, 0.380, 0.380, 0.380, 0.380, 0.380,
];

#[rustfmt::skip]
pub const TAUS_VIS_IGBP: [f64; IGBP_CLASSES] = [
    0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001, 0.001,
    0.001, 0.220, 0.001, 0.220, 0.001, 0.220, 0.001, 0.001,
    0.001,
];

#[rustfmt::skip]
pub const TAUS_VIS_USGS: [f64; USGS_CLASSES] = [
    0.220, 0.220, 0.220, 0.220, 0.220, 0.220, 0.220, 0.001,
    0.001, 0.220, 0.001, 0.001, 0.001, 0.001, 0.001, 0.220,
    0.220, 0.001, 0.001, 0.220, 0.220, 0.220, 0.220, 0.220,
];

#[rustfmt::skip]
pub const TRDA_IGBP: [f64; IGBP_CLASSES] = [
    1.3, 1.3, 1.3, 1.3, 1.3, 1.3, 1.3, 1.3,
    1.3, 1.3, 1.3, 1.3, 1.3, 1.3, 1.3, 1.3,
    1.3,
];

#[rustfmt::skip]
pub const TRDA_USGS: [f64; USGS_CLASSES] = [
    1.3, 1.3, 1.3, 1.3, 1.3, 1.3, 1.3, 1.3,
    1.3, 1.3, 1.3, 1.3, 1.3, 1.3, 1.3, 1.3,
    1.3, 1.3, 1.3, 1.3, 1.3, 1.3, 1.3, 1.3,
];

#[rustfmt::skip]
pub const TRDM_IGBP: [f64; IGBP_CLASSES] = [
    328.0, 328.0, 328.0, 328.0, 328.0, 328.0, 328.0, 328.0,
    328.0, 328.0, 328.0, 328.0, 328.0, 328.0, 328.0, 328.0,
    328.0,
];

#[rustfmt::skip]
pub const TRDM_USGS: [f64; USGS_CLASSES] = [
    328.0, 328.0, 328.0, 328.0, 328.0, 328.0, 328.0, 328.0,
    328.0, 328.0, 328.0, 328.0, 328.0, 328.0, 328.0, 328.0,
    328.0, 328.0, 328.0, 328.0, 328.0, 328.0, 328.0, 328.0,
];

#[rustfmt::skip]
pub const TROP_IGBP: [f64; IGBP_CLASSES] = [
    298.0, 298.0, 298.0, 298.0, 298.0, 298.0, 298.0, 298.0,
    298.0, 298.0, 298.0, 298.0, 298.0, 298.0, 298.0, 298.0,
    298.0,
];

#[rustfmt::skip]
pub const TROP_USGS: [f64; USGS_CLASSES] = [
    298.0, 298.0, 298.0, 298.0, 298.0, 298.0, 298.0, 298.0,
    298.0, 298.0, 298.0, 298.0, 298.0, 298.0, 298.0, 298.0,
    298.0, 298.0, 298.0, 298.0, 298.0, 298.0, 298.0, 298.0,
];

#[rustfmt::skip]
pub const VMAX25_IGBP: [f64; IGBP_CLASSES] = [
    54.0, 72.0, 57.0, 52.0, 52.0, 52.0, 52.0, 52.0,
    52.0, 52.0, 52.0, 57.0, 100.0, 57.0, 52.0, 52.0,
    52.0,
];

#[rustfmt::skip]
pub const VMAX25_USGS: [f64; USGS_CLASSES] = [
    100.0, 57.0, 57.0, 57.0, 52.0, 52.0, 52.0, 52.0,
    52.0, 52.0, 52.0, 57.0, 72.0, 54.0, 52.0, 57.0,
    52.0, 52.0, 52.0, 52.0, 52.0, 52.0, 52.0, 52.0,
];

#[rustfmt::skip]
pub const Z0MR_IGBP: [f64; IGBP_CLASSES] = [
    0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1,
    0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1,
    0.1,
];

#[rustfmt::skip]
pub const Z0MR_USGS: [f64; USGS_CLASSES] = [
    0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1,
    0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1,
    0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1,
];

/// 一个地类分类体系下的全部常量。字段名与上游被选中的数组同名（小写保持
/// 上游拼写），单位与 `MOD_Const_LC.F90` 一致。
#[derive(Debug, Clone, Copy)]
pub struct LandCoverTables {
    pub beta: &'static [f64],
    pub binter: &'static [f64],
    pub c3c4: &'static [i32],
    pub chil: &'static [f64],
    pub ck0: &'static [f64],
    pub d50: &'static [f64],
    pub displar: &'static [f64],
    pub effcon: &'static [f64],
    pub extkn: &'static [f64],
    pub fveg0: &'static [f64],
    pub g0: &'static [f64],
    pub g1: &'static [f64],
    pub gradm: &'static [f64],
    pub hbot0: &'static [f64],
    pub hhti: &'static [f64],
    pub hlti: &'static [f64],
    pub htop0: &'static [f64],
    pub kmax_root0: &'static [f64],
    pub kmax_sha0: &'static [f64],
    pub kmax_sun0: &'static [f64],
    pub kmax_xyl0: &'static [f64],
    pub lambda: &'static [f64],
    pub patchtypes: &'static [i32],
    pub psi50_root0: &'static [f64],
    pub psi50_sha0: &'static [f64],
    pub psi50_sun0: &'static [f64],
    pub psi50_xyl0: &'static [f64],
    pub respcp: &'static [f64],
    pub rhol_nir: &'static [f64],
    pub rhol_vis: &'static [f64],
    pub rhos_nir: &'static [f64],
    pub rhos_vis: &'static [f64],
    pub roota: &'static [f64],
    pub rootb: &'static [f64],
    pub sai0: &'static [f64],
    pub shti: &'static [f64],
    pub slti: &'static [f64],
    pub sqrtdi: &'static [f64],
    pub taul_nir: &'static [f64],
    pub taul_vis: &'static [f64],
    pub taus_nir: &'static [f64],
    pub taus_vis: &'static [f64],
    pub trda: &'static [f64],
    pub trdm: &'static [f64],
    pub trop: &'static [f64],
    pub vmax25: &'static [f64],
    pub z0mr: &'static [f64],
}

pub const IGBP: LandCoverTables = LandCoverTables {
    beta: &BETA_IGBP,
    binter: &BINTER_IGBP,
    c3c4: &C3C4_IGBP,
    chil: &CHIL_IGBP,
    ck0: &CK0_IGBP,
    d50: &D50_IGBP,
    displar: &DISPLAR_IGBP,
    effcon: &EFFCON_IGBP,
    extkn: &EXTKN_IGBP,
    fveg0: &FVEG0_IGBP,
    g0: &G0_IGBP,
    g1: &G1_IGBP,
    gradm: &GRADM_IGBP,
    hbot0: &HBOT0_IGBP,
    hhti: &HHTI_IGBP,
    hlti: &HLTI_IGBP,
    htop0: &HTOP0_IGBP,
    kmax_root0: &KMAX_ROOT0_IGBP,
    kmax_sha0: &KMAX_SHA0_IGBP,
    kmax_sun0: &KMAX_SUN0_IGBP,
    kmax_xyl0: &KMAX_XYL0_IGBP,
    lambda: &LAMBDA_IGBP,
    patchtypes: &PATCHTYPES_IGBP,
    psi50_root0: &PSI50_ROOT0_IGBP,
    psi50_sha0: &PSI50_SHA0_IGBP,
    psi50_sun0: &PSI50_SUN0_IGBP,
    psi50_xyl0: &PSI50_XYL0_IGBP,
    respcp: &RESPCP_IGBP,
    rhol_nir: &RHOL_NIR_IGBP,
    rhol_vis: &RHOL_VIS_IGBP,
    rhos_nir: &RHOS_NIR_IGBP,
    rhos_vis: &RHOS_VIS_IGBP,
    roota: &ROOTA_IGBP,
    rootb: &ROOTB_IGBP,
    sai0: &SAI0_IGBP,
    shti: &SHTI_IGBP,
    slti: &SLTI_IGBP,
    sqrtdi: &SQRTDI_IGBP,
    taul_nir: &TAUL_NIR_IGBP,
    taul_vis: &TAUL_VIS_IGBP,
    taus_nir: &TAUS_NIR_IGBP,
    taus_vis: &TAUS_VIS_IGBP,
    trda: &TRDA_IGBP,
    trdm: &TRDM_IGBP,
    trop: &TROP_IGBP,
    vmax25: &VMAX25_IGBP,
    z0mr: &Z0MR_IGBP,
};

pub const USGS: LandCoverTables = LandCoverTables {
    beta: &BETA_USGS,
    binter: &BINTER_USGS,
    c3c4: &C3C4_USGS,
    chil: &CHIL_USGS,
    ck0: &CK0_USGS,
    d50: &D50_USGS,
    displar: &DISPLAR_USGS,
    effcon: &EFFCON_USGS,
    extkn: &EXTKN_USGS,
    fveg0: &FVEG0_USGS,
    g0: &G0_USGS,
    g1: &G1_USGS,
    gradm: &GRADM_USGS,
    hbot0: &HBOT0_USGS,
    hhti: &HHTI_USGS,
    hlti: &HLTI_USGS,
    htop0: &HTOP0_USGS,
    kmax_root0: &KMAX_ROOT0_USGS,
    kmax_sha0: &KMAX_SHA0_USGS,
    kmax_sun0: &KMAX_SUN0_USGS,
    kmax_xyl0: &KMAX_XYL0_USGS,
    lambda: &LAMBDA_USGS,
    patchtypes: &PATCHTYPES_USGS,
    psi50_root0: &PSI50_ROOT0_USGS,
    psi50_sha0: &PSI50_SHA0_USGS,
    psi50_sun0: &PSI50_SUN0_USGS,
    psi50_xyl0: &PSI50_XYL0_USGS,
    respcp: &RESPCP_USGS,
    rhol_nir: &RHOL_NIR_USGS,
    rhol_vis: &RHOL_VIS_USGS,
    rhos_nir: &RHOS_NIR_USGS,
    rhos_vis: &RHOS_VIS_USGS,
    roota: &ROOTA_USGS,
    rootb: &ROOTB_USGS,
    sai0: &SAI0_USGS,
    shti: &SHTI_USGS,
    slti: &SLTI_USGS,
    sqrtdi: &SQRTDI_USGS,
    taul_nir: &TAUL_NIR_USGS,
    taul_vis: &TAUL_VIS_USGS,
    taus_nir: &TAUS_NIR_USGS,
    taus_vis: &TAUS_VIS_USGS,
    trda: &TRDA_USGS,
    trdm: &TRDM_USGS,
    trop: &TROP_USGS,
    vmax25: &VMAX25_USGS,
    z0mr: &Z0MR_USGS,
};

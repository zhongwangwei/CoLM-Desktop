//! `MOD_Const_LC.F90` 的地类常量：取值与派生。
//!
//! 表在 `land_cover_generated.rs`（`xtask gen-landcover` 生成、drift 测试守住），
//! 这里放**不能靠抄表得到的东西**：分类体系的选择、`vmax25` 的 `1.e-6` 缩放、
//! `rho`/`tau` 的二维铺排，以及 `rootfr` 的两种根系分布。
//!
//! 上游把这些写在 `Init_LC_Const` 里；本模块逐条照做，不自作聪明地"简化"。
//! `ROOTFR_SCHEME` 的两支公式在 `MOD_Const_LC.F90` 里各自成段，取值必须一起看。

use anyhow::{ensure, Result};

use crate::land_cover_generated::{LandCoverTables, IGBP, IGBP_CLASSES, USGS, USGS_CLASSES};
use crate::{LandCoverScheme, LeafOptics};

pub use crate::land_cover_generated;

/// 选中分类体系的常量表。
pub fn land_cover_tables(scheme: LandCoverScheme) -> &'static LandCoverTables {
    match scheme {
        LandCoverScheme::Igbp => &IGBP,
        LandCoverScheme::Usgs => &USGS,
    }
}

/// 一个分类体系的地类数。
pub fn land_cover_classes(scheme: LandCoverScheme) -> usize {
    match scheme {
        LandCoverScheme::Igbp => IGBP_CLASSES,
        LandCoverScheme::Usgs => USGS_CLASSES,
    }
}

/// `DEF_RootReachScheme` / `ROOTFR_SCHEME`：上默认与备用的两套根系分布。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootFractionScheme {
    /// `ROOTFR_SCHEME == 1`：Schenk & Jackson (2002) 的 `d50`/`beta` 形式。
    SchenkJackson,
    /// 其余取值：`roota`/`rootb` 的双指数形式。
    Exponential,
}

/// `Init_LC_Const` 里被选中的那些数组，按 1 基的 Fortran 下标访问。
///
/// 上游的数组是 `(i_class)`，`i_class = 1..N_land_classification`，对应 0 基的
/// 地类 `i_class - 1`。这里保留 1 基下标，免得调用方在两套约定之间来回换算。
pub struct ClassConstants {
    scheme: LandCoverScheme,
    class: usize,
}

impl ClassConstants {
    pub fn new(scheme: LandCoverScheme, fortran_index: usize) -> Result<Self> {
        let classes = land_cover_classes(scheme);
        ensure!(
            (1..=classes).contains(&fortran_index),
            "land class index {fortran_index} is outside 1..={classes} for {scheme:?}"
        );
        Ok(Self {
            scheme,
            class: fortran_index,
        })
    }

    fn tables(&self) -> &'static LandCoverTables {
        land_cover_tables(self.scheme)
    }

    /// 0 基的地类号，即上游的 `patchclass`。
    pub fn class_zero_based(&self) -> usize {
        self.class - 1
    }

    fn value(&self, column: fn(&'static LandCoverTables) -> &'static [f64]) -> f64 {
        column(self.tables())[self.class - 1]
    }

    /// `patchtypes`：0 土壤、1 城市、2 湿地、3 冰、4 湖。
    pub fn patch_type(&self) -> i32 {
        self.tables().patchtypes[self.class - 1]
    }

    /// `Init_LC_Const` 把植被最大羧化速率从 `umol/m2/s` 折成 `mol/m2/s`。
    pub fn maximum_carboxylation_25c_mol_m2_s(&self) -> f64 {
        self.value(|table| table.vmax25) * 1.0e-6
    }

    pub fn canopy_top_m(&self) -> f64 {
        self.value(|table| table.htop0)
    }

    pub fn canopy_bottom_m(&self) -> f64 {
        self.value(|table| table.hbot0)
    }

    pub fn vegetation_fraction(&self) -> f64 {
        self.value(|table| table.fveg0)
    }

    pub fn stem_area_index(&self) -> f64 {
        self.value(|table| table.sai0)
    }

    /// `z0mr`：粗糙度长与冠层高度之比。
    pub fn roughness_to_height_ratio(&self) -> f64 {
        self.value(|table| table.z0mr)
    }

    /// `displar`：零平面位移与冠层高度之比。
    pub fn displacement_to_height_ratio(&self) -> f64 {
        self.value(|table| table.displar)
    }

    /// `sqrtdi`：叶片尺度的 `m**-0.5`。
    pub fn inverse_sqrt_leaf_dimension_m_neg_half(&self) -> f64 {
        self.value(|table| table.sqrtdi)
    }

    /// `chil`：叶倾角分布参数，即内核里的 `leaf_angle_distribution`。
    pub fn leaf_angle_distribution(&self) -> f64 {
        self.value(|table| table.chil)
    }

    pub fn extinction_coefficient(&self) -> f64 {
        self.value(|table| table.extkn)
    }

    /// `d50`：根系分布的特征深度。
    pub fn root_d50(&self) -> f64 {
        self.value(|table| table.d50)
    }

    /// `beta`：根系分布的形参。
    pub fn root_beta(&self) -> f64 {
        self.value(|table| table.beta)
    }

    /// `roota` / `rootb`：双指数根系分布的两个衰减率。
    pub fn root_exponential_rates(&self) -> (f64, f64) {
        (
            self.value(|table| table.roota),
            self.value(|table| table.rootb),
        )
    }

    /// `Init_LC_Const` 的 `rho(1,1,:)=rhol_vis` 那一组赋值。
    pub fn leaf_optics(&self) -> LeafOptics {
        LeafOptics {
            chil: self.value(|table| table.chil),
            reflectance: [
                [
                    self.value(|table| table.rhol_vis),
                    self.value(|table| table.rhos_vis),
                ],
                [
                    self.value(|table| table.rhol_nir),
                    self.value(|table| table.rhos_nir),
                ],
            ],
            transmittance: [
                [
                    self.value(|table| table.taul_vis),
                    self.value(|table| table.taus_vis),
                ],
                [
                    self.value(|table| table.taul_nir),
                    self.value(|table| table.taus_nir),
                ],
            ],
        }
    }
}

/// `Init_LC_Const` 的 `rootfr`：逐层根系比例，求和恰为 1。
///
/// `interface_depth_m` 是共享土层网格的**层底界面深度**（`zi_soi`，`[0] = 0`），
/// 长度必须是 `layers + 1`。上游的 `zi_soi(nsl)` 就是这里的
/// `interface_depth_m[nsl]`（`nsl` 自 1 起），也就是第 `nsl` 层的层底深度。
///
/// 两支的分母都写成 `1 + f`，所以中间层是**差分**；正因为是差分，求和必然为 1
/// （实测两支都到 1e-15 以内），这一条也写成了测试。
pub fn root_fraction(
    scheme: LandCoverScheme,
    land_class: i32,
    root_scheme: RootFractionScheme,
    interface_depth_m: &[f64],
) -> Result<Vec<f64>> {
    let classes = land_cover_classes(scheme);
    let index = usize::try_from(land_class)
        .ok()
        .filter(|index| (1..=classes).contains(index))
        .ok_or_else(|| {
            anyhow::anyhow!("land class index {land_class} is outside 1..={classes} for {scheme:?}")
        })?;
    let layers = interface_depth_m.len().checked_sub(1).ok_or_else(|| {
        anyhow::anyhow!("the interface depth array must hold one more entry than layers")
    })?;
    ensure!(layers >= 2, "root fractions need at least two soil layers");
    ensure!(
        interface_depth_m.windows(2).all(|pair| pair[0] < pair[1]),
        "soil interface depths must increase downwards"
    );
    // `zi_soi(nsl)`，`nsl` 自 1 起。
    let zi = |nsl: usize| interface_depth_m[nsl];
    // 上游的 `i` 是 1 基地类下标。
    let class = ClassConstants::new(scheme, index)?;
    let mut fractions = vec![0.0; layers];
    match root_scheme {
        RootFractionScheme::SchenkJackson => {
            let d50 = class.root_d50();
            let beta = class.root_beta();
            ensure!(
                d50 > 0.0,
                "land class {land_class} has a non-positive d50, so its root distribution is undefined"
            );
            let cumulative = |nsl: usize| 1.0 / (1.0 + (zi(nsl) * 100.0 / d50).powf(beta));
            fractions[0] = cumulative(1);
            fractions[layers - 1] = 1.0 - cumulative(layers - 1);
            for nsl in 2..layers {
                fractions[nsl - 1] = cumulative(nsl) - cumulative(nsl - 1);
            }
        }
        RootFractionScheme::Exponential => {
            let (roota, rootb) = class.root_exponential_rates();
            let decay = |nsl: usize| 0.5 * ((-roota * zi(nsl)).exp() + (-rootb * zi(nsl)).exp());
            fractions[0] = 1.0 - decay(1);
            fractions[layers - 1] = decay(layers);
            for nsl in 2..layers {
                fractions[nsl - 1] = decay(nsl - 1) - decay(nsl);
            }
        }
    }
    Ok(fractions)
}

#[cfg(test)]
#[path = "land_cover_tests.rs"]
mod land_cover_tests;

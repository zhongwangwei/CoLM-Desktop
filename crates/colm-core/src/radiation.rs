//! Broadband cold-start radiation from `MOD_Albedo.F90`.
//!
//! The arrays use CoLM's native `(band, radiation_type)` layout.  NetCDF reversal
//! remains the responsibility of the restart writer.

use anyhow::{ensure, Result};

use crate::{update_snow_age, LandCoverScheme, SoilReflectance};

const BANDS: usize = 2;
const RADIATION_TYPES: usize = 2;
// `twostream_mod` uses this literal rather than the platform PI constant.
const FORTRAN_PI: f64 = 314_159.0 / 100_000.0;

/// Per-land-class optical constants passed by `MOD_Const_LC` to `twostream`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LeafOptics {
    pub chil: f64,
    /// `[band][green leaf, dead stem]`.
    pub reflectance: [[f64; BANDS]; BANDS],
    /// `[band][green leaf, dead stem]`.
    pub transmittance: [[f64; BANDS]; BANDS],
}

/// Ground optical state before a canopy two-stream calculation.
///
/// All matrices use CoLM's `[visible-or-near-infrared][direct-or-diffuse]`
/// layout.  Keeping this boundary separate lets broadband and hyperspectral
/// canopy drivers use exactly the same soil, water, and snow treatment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColdStartGroundAlbedo {
    pub soil: [[f64; RADIATION_TYPES]; BANDS],
    pub snow: [[f64; RADIATION_TYPES]; BANDS],
    pub ground: [[f64; RADIATION_TYPES]; BANDS],
    pub snow_age: f64,
}

impl ColdStartGroundAlbedo {
    /// Original `albland` soil/snow absorption after the canopy transmission sum.
    pub fn absorption(
        &self,
        transmission: [[f64; 3]; BANDS],
    ) -> (
        [[f64; RADIATION_TYPES]; BANDS],
        [[f64; RADIATION_TYPES]; BANDS],
    ) {
        let mut soil_absorption = [[0.0; RADIATION_TYPES]; BANDS];
        let mut snow_absorption = [[0.0; RADIATION_TYPES]; BANDS];
        for band in 0..BANDS {
            soil_absorption[band][0] = transmission[band][2].mul_add(
                1.0 - self.soil[band][0],
                transmission[band][0] * (1.0 - self.soil[band][1]),
            );
            soil_absorption[band][1] = transmission[band][1] * (1.0 - self.soil[band][1]);
            snow_absorption[band][0] = (1.0 - self.snow[band][0]).mul_add(
                transmission[band][2],
                transmission[band][0] * (1.0 - self.snow[band][1]),
            );
            snow_absorption[band][1] = transmission[band][1] * (1.0 - self.snow[band][1]);
        }
        (soil_absorption, snow_absorption)
    }
}

/// Looks up CoLM's native broadband leaf optical constants for one land class.
///
/// This is the `rho`/`tau` assignment in `MOD_Const_LC.F90`. 取 `fortran_class_index`，
/// 即上游数组的 **1 基下标**：`chil(fortran_class_index)`。上游的访问方式是
/// `chil(patchclass(ipatch)+1)`，其中 `patchclass` 是 0 基的 IGBP/USGS 类号。
/// 函数名带 `_one_based` 是仓库约定（CLAUDE.md 代码风格一节）。
///
/// **不要**与 [`crate::land_cover_soil_reflectance`] 混淆：那个取 0 基的
/// `patchclass`。两个参数以前都叫 `land_class`，实测很容易把 1 和 0 喂反 ——
/// 喂反了不会报错，只会拿到隔壁地类的光学参数。
///
/// 取值来自 `land_cover_generated.rs`（由 `xtask gen-landcover` 从上游源码生成），
/// 那里是**唯一的**一份地类常量表。
///
/// `overrides` 是单点 LCT 本站的 `DEF_LC_*`（`rho`/`tau`/`chil` 那九列）；其余情形给默认（全空）。
pub fn leaf_optics_from_land_cover_one_based(
    scheme: LandCoverScheme,
    fortran_class_index: i32,
    overrides: crate::LandClassOverrides,
) -> Result<LeafOptics> {
    let land_class = fortran_class_index;
    let index = usize::try_from(land_class)
        .map_err(|_| anyhow::anyhow!("land class {land_class} is negative"))?;
    ensure!(index >= 1, "land class must start at one");
    Ok(crate::ClassConstants::new(scheme, index)?
        .with_overrides(overrides)
        .leaf_optics())
}

/// Broadband arrays produced by CoLM's cold-start `albland` path.
#[derive(Debug, Clone, PartialEq)]
pub struct ColdStartRadiation {
    /// `[band][direct, diffuse]`.
    pub albedo: [[f64; RADIATION_TYPES]; BANDS],
    pub sunlit_absorption: [[f64; RADIATION_TYPES]; BANDS],
    pub shaded_absorption: [[f64; RADIATION_TYPES]; BANDS],
    pub soil_absorption: [[f64; RADIATION_TYPES]; BANDS],
    pub snow_absorption: [[f64; RADIATION_TYPES]; BANDS],
    /// `[band][direct-to-diffuse, diffuse-to-diffuse, direct-to-direct]`.
    ///
    /// Present for broadband `albland`/PFT/PC; the spectral adapter leaves it unset.
    /// PC can retain broadband optics in a high-resolution run, so presence alone
    /// does not permit replacing its 211-band ground absorption with this array.
    pub transmission: Option<[[f64; 3]; BANDS]>,
    /// CoLM's dimensionless snow age after its first 1800-second update.
    pub snow_age: f64,
    pub thermal_gap_fraction: f64,
    pub direct_extinction: f64,
    pub diffuse_extinction: f64,
}

/// Applies the no-snow cold-start branch of `albland` and `twostream`.
///
/// `cosine_zenith` must already be clamped like the `IniTimeVar` caller does
/// (`max(0.001, coszen)`).  A non-LCT natural patch keeps the soil albedo, exactly
/// as the source's `DEF_USE_LCT` guard does.
#[allow(clippy::too_many_arguments)]
pub fn cold_start_broadband_radiation(
    patch_type: i32,
    soil: SoilReflectance,
    soil_liquid_water_kg_m2: f64,
    soil_thickness_m: f64,
    optics: LeafOptics,
    lai: f64,
    sai: f64,
    wet_snow_fraction: f64,
    cosine_zenith: f64,
    use_lct: bool,
    usgs_land_cover: bool,
    vegetation_snow: bool,
) -> Result<ColdStartRadiation> {
    cold_start_broadband_radiation_with_snow(
        patch_type,
        soil,
        soil_liquid_water_kg_m2,
        soil_thickness_m,
        optics,
        lai,
        sai,
        wet_snow_fraction,
        cosine_zenith,
        use_lct,
        usgs_land_cover,
        vegetation_snow,
        0.0,
        0.0,
        273.16,
    )
}

/// Applies `albland` with the non-SNICAR snow initialization used by `mkinidata`.
///
/// The supplied snow depth is converted with the source's fixed 250 kg m-3
/// initialization density.  SNICAR remains a distinct feature because its optical
/// lookup tables and layer absorption model are not part of this broadband kernel.
#[allow(clippy::too_many_arguments)]
pub fn cold_start_broadband_radiation_with_snow(
    patch_type: i32,
    soil: SoilReflectance,
    soil_liquid_water_kg_m2: f64,
    soil_thickness_m: f64,
    optics: LeafOptics,
    lai: f64,
    sai: f64,
    wet_snow_fraction: f64,
    cosine_zenith: f64,
    use_lct: bool,
    usgs_land_cover: bool,
    vegetation_snow: bool,
    snow_depth_m: f64,
    ground_snow_fraction: f64,
    ground_temperature_k: f64,
) -> Result<ColdStartRadiation> {
    cold_start_broadband_radiation_with_snow_using(
        patch_type,
        soil,
        soil_liquid_water_kg_m2,
        soil_thickness_m,
        optics,
        lai,
        sai,
        wet_snow_fraction,
        cosine_zenith,
        use_lct,
        vegetation_snow,
        snow_depth_m,
        ground_snow_fraction,
        ground_temperature_k,
        TwoStreamKind::LandCover { usgs_land_cover },
    )
}

/// Applies the PFT-specific `twostream_mod` path from `MOD_Albedo.F90`.
///
/// CoLM uses a different two-stream implementation for PFT vectors than for
/// land-cover tiles; using the land-cover routine here changes initialized
/// canopy absorption even when the optical constants are correct.
#[allow(clippy::too_many_arguments)]
pub fn cold_start_pft_broadband_radiation_with_snow(
    patch_type: i32,
    soil: SoilReflectance,
    soil_liquid_water_kg_m2: f64,
    soil_thickness_m: f64,
    optics: LeafOptics,
    lai: f64,
    sai: f64,
    wet_snow_fraction: f64,
    cosine_zenith: f64,
    vegetation_snow: bool,
    snow_depth_m: f64,
    ground_snow_fraction: f64,
    ground_temperature_k: f64,
) -> Result<ColdStartRadiation> {
    cold_start_broadband_radiation_with_snow_using(
        patch_type,
        soil,
        soil_liquid_water_kg_m2,
        soil_thickness_m,
        optics,
        lai,
        sai,
        wet_snow_fraction,
        cosine_zenith,
        true,
        vegetation_snow,
        snow_depth_m,
        ground_snow_fraction,
        ground_temperature_k,
        TwoStreamKind::Pft,
    )
}

/// 上游 `twostream`（地块）与 `twostream_mod`（PFT 向量）是**两套**实现，
/// 换用会让初始化出来的冠层吸收率变化，所以这里把选择做成显式枚举而不是一个 bool。
pub(crate) enum TwoStreamKind {
    LandCover { usgs_land_cover: bool },
    Pft,
}

#[allow(clippy::too_many_arguments)]
fn cold_start_broadband_radiation_with_snow_using(
    patch_type: i32,
    soil: SoilReflectance,
    soil_liquid_water_kg_m2: f64,
    soil_thickness_m: f64,
    optics: LeafOptics,
    lai: f64,
    sai: f64,
    wet_snow_fraction: f64,
    cosine_zenith: f64,
    use_lct: bool,
    vegetation_snow: bool,
    snow_depth_m: f64,
    ground_snow_fraction: f64,
    ground_temperature_k: f64,
    two_stream_kind: TwoStreamKind,
) -> Result<ColdStartRadiation> {
    ensure!(
        soil_liquid_water_kg_m2.is_finite()
            && soil_thickness_m.is_finite()
            && soil_thickness_m > 0.0
            && lai.is_finite()
            && lai >= 0.0
            && sai.is_finite()
            && sai >= 0.0
            && wet_snow_fraction.is_finite()
            && (0.0..=1.0).contains(&wet_snow_fraction)
            && snow_depth_m.is_finite()
            && snow_depth_m >= 0.0
            && ground_snow_fraction.is_finite()
            && (0.0..=1.0).contains(&ground_snow_fraction)
            && ground_temperature_k.is_finite()
            && cosine_zenith.is_finite()
            && cosine_zenith > 0.0
            && optics.chil.is_finite(),
        "cold-start radiation inputs are invalid"
    );
    for value in [
        soil.saturated_visible,
        soil.dry_visible,
        soil.saturated_near_infrared,
        soil.dry_near_infrared,
    ] {
        ensure!(value.is_finite(), "soil reflectance must be finite");
    }
    for values in optics.reflectance.into_iter().chain(optics.transmittance) {
        for value in values {
            ensure!(value.is_finite(), "leaf optical constants must be finite");
        }
    }

    let ground_state = cold_start_ground_albedo(
        patch_type,
        soil,
        soil_liquid_water_kg_m2,
        soil_thickness_m,
        cosine_zenith,
        snow_depth_m,
        ground_snow_fraction,
        ground_temperature_k,
    )?;
    broadband_radiation_from_ground_using(
        patch_type,
        ground_state,
        optics,
        lai,
        sai,
        wet_snow_fraction,
        cosine_zenith,
        use_lct,
        vegetation_snow,
        two_stream_kind,
        cold_start_thermal_gap_fraction(patch_type, lai, sai),
    )
}

/// Applies the LCT broadband canopy cold-start path to an already computed ground albedo.
///
/// This is the shared trust boundary used by SNICAR and non-SNICAR ground drivers:
/// callers own the ground optical state, while this function preserves CoLM's LCT
/// canopy/two-stream and soil/snow absorption tail.
#[allow(clippy::too_many_arguments)]
pub fn cold_start_broadband_radiation_from_ground(
    patch_type: i32,
    ground: ColdStartGroundAlbedo,
    optics: LeafOptics,
    lai: f64,
    sai: f64,
    wet_snow_fraction: f64,
    cosine_zenith: f64,
    use_lct: bool,
    usgs_land_cover: bool,
    vegetation_snow: bool,
) -> Result<ColdStartRadiation> {
    broadband_radiation_from_ground_using(
        patch_type,
        ground,
        optics,
        lai,
        sai,
        wet_snow_fraction,
        cosine_zenith,
        use_lct,
        vegetation_snow,
        TwoStreamKind::LandCover { usgs_land_cover },
        cold_start_thermal_gap_fraction(patch_type, lai, sai),
    )
}

/// Applies the PFT-vector broadband canopy cold-start path to an already computed ground albedo.
#[allow(clippy::too_many_arguments)]
pub fn cold_start_pft_broadband_radiation_from_ground(
    patch_type: i32,
    ground: ColdStartGroundAlbedo,
    optics: LeafOptics,
    lai: f64,
    sai: f64,
    wet_snow_fraction: f64,
    cosine_zenith: f64,
    vegetation_snow: bool,
) -> Result<ColdStartRadiation> {
    broadband_radiation_from_ground_using(
        patch_type,
        ground,
        optics,
        lai,
        sai,
        wet_snow_fraction,
        cosine_zenith,
        true,
        vegetation_snow,
        TwoStreamKind::Pft,
        cold_start_thermal_gap_fraction(patch_type, lai, sai),
    )
}

/// 冷启动时 `thermk` 的起点。
///
/// 上游 `albland` 一进来就把 `thermk` 无条件写 1 只发生在 `lai+sai <= 1e-6` 时
/// （`MOD_Albedo.F90:225-228`，注释写明"夜间长波用上一步的值"）；有冠层时它保留上一次
/// 调用留下的值，而第一次调用之前它只有 `MOD_Vars_*` 分配时的 `spval`。
/// `patchtype >= 3` 又不跑冠层解算器，所以那一支留下的就是 `spval`。
fn cold_start_thermal_gap_fraction(patch_type: i32, lai: f64, sai: f64) -> f64 {
    if lai + sai <= 1.0e-6 {
        1.0
    } else if patch_type >= 3 {
        crate::MISSING
    } else {
        0.0
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn broadband_radiation_from_ground_using(
    patch_type: i32,
    ground_state: ColdStartGroundAlbedo,
    optics: LeafOptics,
    lai: f64,
    sai: f64,
    wet_snow_fraction: f64,
    cosine_zenith: f64,
    use_lct: bool,
    vegetation_snow: bool,
    two_stream_kind: TwoStreamKind,
    previous_thermal_gap_fraction: f64,
) -> Result<ColdStartRadiation> {
    validate_canopy_from_ground_inputs(
        ground_state,
        optics,
        lai,
        sai,
        wet_snow_fraction,
        cosine_zenith,
    )?;
    let snow_age = ground_state.snow_age;
    let mut sunlit_absorption = [[0.0; RADIATION_TYPES]; BANDS];
    let mut shaded_absorption = [[0.0; RADIATION_TYPES]; BANDS];
    let mut transmission = [[0.0, 1.0, 1.0]; BANDS];
    // 无冠层时上游把它重置为 1；有冠层时它保留上一次的值，直到 `twostream` 覆盖它。
    let mut thermal_gap_fraction = if lai + sai <= 1.0e-6 {
        1.0
    } else {
        previous_thermal_gap_fraction
    };
    let mut direct_extinction = 1.0;
    let mut diffuse_extinction = 0.718;

    let mut albedo = ground_state.ground;

    if lai + sai > 1.0e-6 && patch_type < 3 && (patch_type != 0 || use_lct) {
        let two_stream = match two_stream_kind {
            TwoStreamKind::LandCover { usgs_land_cover } => two_stream(
                optics,
                lai,
                sai,
                wet_snow_fraction,
                cosine_zenith,
                ground_state.ground,
                usgs_land_cover,
                vegetation_snow,
            )?,
            TwoStreamKind::Pft => two_stream_mod(
                optics,
                lai,
                sai,
                wet_snow_fraction,
                cosine_zenith,
                ground_state.ground,
                vegetation_snow,
            )?,
        };
        albedo = two_stream.albedo;
        transmission = two_stream.transmission;
        sunlit_absorption = two_stream.sunlit_absorption;
        shaded_absorption = two_stream.shaded_absorption;
        thermal_gap_fraction = two_stream.thermal_gap_fraction;
        direct_extinction = two_stream.direct_extinction;
        diffuse_extinction = two_stream.diffuse_extinction;
    }

    let (soil_absorption, snow_absorption) = ground_state.absorption(transmission);

    Ok(ColdStartRadiation {
        albedo,
        sunlit_absorption,
        shaded_absorption,
        soil_absorption,
        snow_absorption,
        transmission: Some(transmission),
        snow_age,
        thermal_gap_fraction,
        direct_extinction,
        diffuse_extinction,
    })
}

fn validate_canopy_from_ground_inputs(
    ground: ColdStartGroundAlbedo,
    optics: LeafOptics,
    lai: f64,
    sai: f64,
    wet_snow_fraction: f64,
    cosine_zenith: f64,
) -> Result<()> {
    ensure!(
        lai.is_finite()
            && lai >= 0.0
            && sai.is_finite()
            && sai >= 0.0
            && wet_snow_fraction.is_finite()
            && (0.0..=1.0).contains(&wet_snow_fraction)
            && cosine_zenith.is_finite()
            && cosine_zenith > 0.0
            && optics.chil.is_finite()
            && ground.snow_age.is_finite(),
        "cold-start radiation inputs are invalid"
    );
    for value in ground
        .soil
        .into_iter()
        .chain(ground.snow)
        .chain(ground.ground)
        .flatten()
    {
        ensure!(value.is_finite(), "ground optical constants must be finite");
    }
    for values in optics.reflectance.into_iter().chain(optics.transmittance) {
        for value in values {
            ensure!(value.is_finite(), "leaf optical constants must be finite");
        }
    }
    Ok(())
}

/// `albland` 第 2 节的土壤/水体地面反照率。
///
/// `soil_surface_wetness` 就是上游的 `ssw = min(1., 1e-3*wliq_soisno(1)/dz_soisno(1))`
/// —— 冷启动时由土壤第一层的水量与厚度现算，运行时由 `CoLMMAIN.F90:2135` 直接给。
pub(crate) fn soil_albedo(
    patch_type: i32,
    soil: SoilReflectance,
    soil_surface_wetness: f64,
    ground_temperature_k: f64,
    cosine_zenith: f64,
) -> Result<[[f64; RADIATION_TYPES]; BANDS]> {
    ensure!(
        soil_surface_wetness.is_finite()
            && (0.0..=1.0).contains(&soil_surface_wetness)
            && ground_temperature_k.is_finite()
            && cosine_zenith.is_finite()
            && cosine_zenith > 0.0,
        "ground-albedo inputs are invalid"
    );
    for value in [
        soil.saturated_visible,
        soil.dry_visible,
        soil.saturated_near_infrared,
        soil.dry_near_infrared,
    ] {
        ensure!(value.is_finite(), "soil reflectance must be finite");
    }
    Ok(if patch_type <= 2 {
        // `MOD_Albedo.F90:305`：`alb_s_inc = max(0.11-0.40*ssw, 0.)`。
        // GIMPLE（`albland` 第 1 处）是 `_434 = FNMA(ssw, 0.40, 0.11)` ——
        // 乘积被吸收、常数 `0.11` 是已舍入的加数；`max` 留在外面。
        let increase = (-soil_surface_wetness).mul_add(0.40, 0.11).max(0.0);
        let visible = (soil.saturated_visible + increase).min(soil.dry_visible);
        let near_infrared = (soil.saturated_near_infrared + increase).min(soil.dry_near_infrared);
        [[visible; RADIATION_TYPES], [near_infrared; RADIATION_TYPES]]
    } else if patch_type == 3 {
        [[0.8; RADIATION_TYPES], [0.55; RADIATION_TYPES]]
    } else if ground_temperature_k < 273.16 {
        [[0.6; RADIATION_TYPES], [0.4; RADIATION_TYPES]]
    } else {
        let albedo_water = 0.05 / (cosine_zenith + 0.15);
        [[albedo_water, 0.1], [albedo_water, 0.1]]
    })
}

/// Applies CoLM's cold-start soil/water and non-SNICAR snow albedo branches.
#[allow(clippy::too_many_arguments)]
pub fn cold_start_ground_albedo(
    patch_type: i32,
    soil: SoilReflectance,
    soil_liquid_water_kg_m2: f64,
    soil_thickness_m: f64,
    cosine_zenith: f64,
    snow_depth_m: f64,
    ground_snow_fraction: f64,
    ground_temperature_k: f64,
) -> Result<ColdStartGroundAlbedo> {
    ensure!(
        soil_liquid_water_kg_m2.is_finite()
            && soil_thickness_m.is_finite()
            && soil_thickness_m > 0.0
            && snow_depth_m.is_finite()
            && snow_depth_m >= 0.0
            && ground_snow_fraction.is_finite()
            && (0.0..=1.0).contains(&ground_snow_fraction)
            && ground_temperature_k.is_finite()
            && cosine_zenith.is_finite()
            && cosine_zenith > 0.0,
        "cold-start ground-albedo inputs are invalid"
    );
    let soil = soil_albedo(
        patch_type,
        soil,
        (1.0e-3 * soil_liquid_water_kg_m2 / soil_thickness_m).min(1.0),
        ground_temperature_k,
        cosine_zenith,
    )?;
    let (snow, snow_age) =
        generic_snow_albedo(snow_depth_m * 250.0, ground_temperature_k, cosine_zenith)?;
    Ok(ColdStartGroundAlbedo {
        ground: mix_ground_albedo(soil, snow, ground_snow_fraction),
        soil,
        snow,
        snow_age,
    })
}

/// Mix validated soil/snow albedos using the caller's ground snow fraction.
pub fn mix_ground_albedo(
    soil: [[f64; RADIATION_TYPES]; BANDS],
    snow: [[f64; RADIATION_TYPES]; BANDS],
    snow_fraction: f64,
) -> [[f64; RADIATION_TYPES]; BANDS] {
    // `main/MOD_Albedo.F90:394` 的 `albg(:,:) = (1.-fsno)*albg(:,:) + fsno*albsno(:,:)`
    // 被 GCC **拆成两半**：直射那一列（`albg(1:2,1)`，内存前 16 字节）向量化成
    // `.FMA (fsno, albsno, (1-fsno)*albg)`；散射那一列是两条标量
    // `.FMA (1-fsno, albg, fsno*albsno)` —— 熔进去的乘积正好相反。
    std::array::from_fn(|band| {
        std::array::from_fn(|radiation_type| {
            let soil = soil[band][radiation_type];
            let snow = snow[band][radiation_type];
            if radiation_type == 0 {
                snow.mul_add(snow_fraction, (1.0 - snow_fraction) * soil)
            } else {
                (1.0 - snow_fraction).mul_add(soil, snow_fraction * snow)
            }
        })
    })
}

/// `albland` 的非 SNICAR 雪面反照率，用于**运行期**（雪龄是状态、不是从零起算）。
///
/// 上游 `MOD_Albedo.F90:341-370`：`albsno` 的四个值由 `sanal0 = 0.85`（可见光）与
/// `snal1 = 0.65`（近红外）经雪龄衰减与天顶角订正得到。冷启动的
/// [`generic_snow_albedo`] 是它的特例（`deltim = 1800`、`scvold = scv`、`sag = 0`），
/// 两者必须给出同一组数，否则"从重启续跑"和"从冷启动起跑"在第一个晴天就会分开。
pub(crate) fn aged_snow_albedo(
    snow_water_equivalent_mm: f64,
    previous_snow_water_equivalent_mm: f64,
    ground_temperature_k: f64,
    cosine_zenith: f64,
    time_step_seconds: f64,
    snow_age: f64,
) -> Result<([[f64; RADIATION_TYPES]; BANDS], f64)> {
    if snow_water_equivalent_mm <= 0.0 {
        return Ok(([[1.0; RADIATION_TYPES]; BANDS], snow_age));
    }
    let snow_age = update_snow_age(
        time_step_seconds,
        ground_temperature_k,
        snow_water_equivalent_mm,
        previous_snow_water_equivalent_mm,
        snow_age,
    )?;
    let age = 1.0 - 1.0 / (1.0 + snow_age);
    // `MOD_Albedo.F90:2036-2038/350-357` 的三处收缩（`albland` 的
    // 第 3、4 处）：`1 + 4*coszrs` 是 `FMA(2*c, 2.0, 1.0)`（先算 `2*c`）；
    // `1 - age_factor*age` 是 `FNMA(age, age_factor, 1.0)`；
    // 直射修正是 `FMA(0.4*corr, 1-diffuse, diffuse)`（收左边那个乘积）。
    let doubled_zenith = 2.0 * cosine_zenith;
    let direct_correction = ((1.5 / doubled_zenith.mul_add(2.0, 1.0)) - 0.5).max(0.0);
    let snow_band = |new_snow_albedo: f64, age_factor: f64| {
        let diffuse = new_snow_albedo * (-age_factor).mul_add(age, 1.0);
        let direct = (0.4 * direct_correction).mul_add(1.0 - diffuse, diffuse);
        [direct, diffuse]
    };
    Ok(([snow_band(0.85, 0.2), snow_band(0.65, 0.5)], snow_age))
}

/// `albland`'s non-SNICAR snow-age/albedo branch for a freshly initialized column.
///
/// 冷启动就是 [`aged_snow_albedo`] 的一次求值：上游 `mkinidata` 首次调用 `albland` 时
/// `deltim = 1800`、`scvold = scv`、`sag = 0`。两处**必须**共用同一个式子 —— 分开写
/// 会让"从冷启动起跑"和"从重启续跑"在第一个晴天给出两组雪面反照率。
pub(crate) fn generic_snow_albedo(
    snow_water_equivalent_mm: f64,
    ground_temperature_k: f64,
    cosine_zenith: f64,
) -> Result<([[f64; RADIATION_TYPES]; BANDS], f64)> {
    aged_snow_albedo(
        snow_water_equivalent_mm,
        snow_water_equivalent_mm,
        ground_temperature_k,
        cosine_zenith,
        1800.0,
        0.0,
    )
}

// Shared by broadband and spectral LCT/PFT kernels in MOD_Albedo(_HiRes).
pub(crate) fn two_stream_zmu(phi1: f64, phi2: f64) -> f64 {
    if phi1.abs() > 1.0e-6 && phi2.abs() > 1.0e-6 {
        let log_term = ((phi1 + phi2) / phi1).ln();
        (phi1 / phi2).mul_add(-log_term, 1.0) * (1.0 / phi2)
    } else if phi1.abs() <= 1.0e-6 {
        1.0 / 0.877
    } else {
        1.0 / (2.0 * phi1)
    }
}

pub(crate) struct TwoStreamRadiation {
    pub(crate) albedo: [[f64; RADIATION_TYPES]; BANDS],
    pub(crate) transmission: [[f64; 3]; BANDS],
    pub(crate) thermal_gap_fraction: f64,
    pub(crate) direct_extinction: f64,
    pub(crate) diffuse_extinction: f64,
    pub(crate) sunlit_absorption: [[f64; RADIATION_TYPES]; BANDS],
    pub(crate) shaded_absorption: [[f64; RADIATION_TYPES]; BANDS],
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn two_stream(
    optics: LeafOptics,
    lai: f64,
    sai: f64,
    wet_snow_fraction: f64,
    cosine_zenith: f64,
    ground: [[f64; RADIATION_TYPES]; BANDS],
    usgs_land_cover: bool,
    vegetation_snow: bool,
) -> Result<TwoStreamRadiation> {
    // `MOD_Albedo.F90:611` 的两级链式 FMA（`twostream` dump 第 1、2 处）：
    // `_3 = FNMA(chil, 0.633, 0.5)`（`0.5-0.633*chil`，乘积被吸收），
    // `_4 = chil*0.33`（这一项**先单独舍入**、被复用），`phi1 = FNMA(_4, chil, _3)`。
    let phi1 = (-(0.33 * optics.chil)).mul_add(optics.chil, (-optics.chil).mul_add(0.633, 0.5));
    // `phi2` 那一句 dump 里**没有**收缩（`_7 = phi1*2; _8 = 1-_7`），保持不融合。
    let phi2 = 0.877 * (1.0 - 2.0 * phi1);
    let projection = phi1 + phi2 * cosine_zenith;
    let direct_extinction = projection / cosine_zenith;
    let diffuse_extinction = 0.719;
    let zmu = two_stream_zmu(phi1, phi2);
    ensure!(
        zmu.is_finite() && zmu > 0.0,
        "invalid leaf angle distribution"
    );
    let stem_area = if usgs_land_cover { 0.0 } else { sai };
    let leaf_stem_area = lai + stem_area;
    let thermal_gap_fraction = (-(lai + sai).clamp(1.0e-5 * zmu, 50.0 * zmu) / zmu).exp();
    ensure!(
        leaf_stem_area > 1.0e-6,
        "two-stream canopy needs positive leaf or stem area"
    );

    let mut albedo = ground;
    let mut transmission = [[0.0; 3]; BANDS];
    let mut sunlit_absorption = [[0.0; RADIATION_TYPES]; BANDS];
    let mut shaded_absorption = [[0.0; RADIATION_TYPES]; BANDS];
    for band in 0..BANDS {
        let mut scattering = (lai / leaf_stem_area).mul_add(
            optics.transmittance[band][0] + optics.reflectance[band][0],
            stem_area / leaf_stem_area
                * (optics.transmittance[band][1] + optics.reflectance[band][1]),
        );
        let directional_scattering = scattering / 2.0 * projection
            / (projection + cosine_zenith * phi2)
            * (-(cosine_zenith * phi1 / (projection + cosine_zenith * phi2))).mul_add(
                ((projection + cosine_zenith * phi2 + cosine_zenith * phi1)
                    / (cosine_zenith * phi1))
                    .ln(),
                1.0,
            );
        let mut upward_scattering = (lai / leaf_stem_area).mul_add(
            optics.transmittance[band][0],
            stem_area / leaf_stem_area * optics.transmittance[band][1],
        );
        let upward_factor = ((1.0 + optics.chil) / 2.0).powi(2);
        upward_scattering =
            0.5 * upward_factor.mul_add(upward_scattering.mul_add(-2.0, scattering), scattering);
        let mut beta0 = (1.0 + zmu * direct_extinction) / (scattering * zmu * direct_extinction)
            * directional_scattering;
        if vegetation_snow {
            let snow_scattering = if band == 0 { 0.8 } else { 0.4 };
            // `MOD_Albedo.F90:629-631`：三处同型。GIMPLE 是
            // `_108 = scat_sno*fwet`（**先舍入**、被复用两次：`scat` 的加数，
            // 以及 `*0.5` 那一项）、`_110 = (1-fwet)*scat`（用更新后的 `scat`），
            // 然后三处都对**旧值**做 `FMA(旧值, _110, _113)`。
            let wet_snow_term = wet_snow_fraction * snow_scattering;
            scattering = scattering.mul_add(1.0 - wet_snow_fraction, wet_snow_term);
            let snow_scaled_scattering = (1.0 - wet_snow_fraction) * scattering;
            upward_scattering =
                upward_scattering.mul_add(snow_scaled_scattering, wet_snow_term * 0.5) / scattering;
            beta0 = beta0.mul_add(snow_scaled_scattering, wet_snow_term * 0.5) / scattering;
        }

        let be = 1.0 - scattering + upward_scattering;
        let ce = upward_scattering;
        // `_135 = ce*ce` 在 dump 里只算一次，被 `be²-ce²`（psi）与 `ce²-be²`
        // （sigma）两处复用；两处都把 `be*be` 收进 FMA、只让 `ce*ce` 先舍入。
        let ce_squared = ce * ce;
        let de = scattering * zmu * direct_extinction * beta0;
        let fe = scattering * zmu * direct_extinction * (1.0 - beta0);
        let psi = be.mul_add(be, -ce_squared).sqrt() / zmu;
        let power1 = (psi * leaf_stem_area).min(50.0);
        let power2 = (direct_extinction * leaf_stem_area).min(50.0);
        let s1 = (-power1).exp();
        let s2 = (-power2).exp();
        let p1 = zmu.mul_add(psi, be);
        let p2 = (-zmu).mul_add(psi, be);
        let p3 = be + zmu * direct_extinction;
        let p4 = be - zmu * direct_extinction;
        let f1 = 1.0 - ground[band][1] * p1 / ce;
        let f2 = 1.0 - ground[band][1] * p2 / ce;
        let h1 = -de.mul_add(p4, ce * fe);
        let h4 = -fe.mul_add(p3, ce * de);
        let sigma = (zmu * direct_extinction).powi(2) + (-be).mul_add(be, ce_squared);
        // `m1`/`m2`/`n1`/`n2` 与两个 Cramer 分母在直接支与漫射支都要用，GIMPLE
        // 只算一次（`_804.._817` 在分支之前）：`_813 = m2*n1`（已舍入、被复用），
        // `_814 = FMS(m1,n2,_813)`、`_816 = FNMA(m1,n2,_813)` —— 即
        // `m1*n2` 被吸收、`m2*n1` 先舍入（另一支反之）。
        let m1 = f1 * s1;
        let m2 = f2 / s1;
        let n1 = p1 / ce;
        let n2 = p2 / ce;
        let m2_n1 = m2 * n1;
        let cramer_direct = m1.mul_add(n2, -m2_n1);
        let cramer_reverse = (-m1).mul_add(n2, m2_n1);
        let (albedo_direct, transmission_direct, eup_direct, edown_direct) = if sigma.abs()
            > 1.0e-10
        {
            let hh1 = h1 / sigma;
            let hh4 = h4 / sigma;
            let m3 = (ground[band][0] - (-ground[band][1]).mul_add(hh4, hh1)) * s2;
            let n3 = -hh4;
            // `_192 = FMS(m3, n2, n3*m2)`、`_201 = FMS(m3, n1, n3*m1)`。
            let hh2 = m3.mul_add(n2, -(m2 * n3)) / cramer_direct;
            let hh3 = m3.mul_add(n1, -(m1 * n3)) / cramer_reverse;
            let hh5 = hh2 * p1 / ce;
            let hh6 = hh3 * p2 / ce;
            (
                hh1 + hh2 + hh3,
                s2.mul_add(hh4, hh5 * s1) + hh6 / s1,
                hh1 * s2.mul_add(-s2, 1.0) / (2.0 * direct_extinction)
                    + hh2 * s1.mul_add(-s2, 1.0) / (direct_extinction + psi)
                    + hh3 * (1.0 - s2 / s1) / (direct_extinction - psi),
                hh4 * s2.mul_add(-s2, 1.0) / (2.0 * direct_extinction)
                    + hh5 * s1.mul_add(-s2, 1.0) / (direct_extinction + psi)
                    + hh6 * (1.0 - s2 / s1) / (direct_extinction - psi),
            )
        } else {
            let zmu2 = zmu * zmu;
            let m3 = h1 / zmu2 * (leaf_stem_area + 1.0 / (2.0 * direct_extinction)) * s2
                + ground[band][1] / ce
                    * (-h1 / (2.0 * direct_extinction) / zmu2
                        * (p3 * leaf_stem_area + p4 / (2.0 * direct_extinction))
                        - de)
                    * s2
                + ground[band][0] * s2;
            let n3 =
                1.0 / ce * (h1 * p4 / (4.0 * direct_extinction * direct_extinction) / zmu2 + de);
            // 同一对 Cramer 分子（`_289`/`_298`），形状与直接支一致。
            let hh2 = m3.mul_add(n2, -(m2 * n3)) / cramer_direct;
            let hh3 = m3.mul_add(n1, -(m1 * n3)) / cramer_reverse;
            let hh5 = hh2 * p1 / ce;
            let hh6 = hh3 * p2 / ce;
            (
                -h1 / (2.0 * direct_extinction * zmu2) + hh2 + hh3,
                1.0 / ce
                    * (-h1 / (2.0 * direct_extinction * zmu2)
                        * (p3 * leaf_stem_area + p4 / (2.0 * direct_extinction))
                        - de)
                    * s2
                    + hh5 * s1
                    + hh6 / s1,
                (hh2 - h1 / (2.0 * direct_extinction * zmu2)) * (1.0 - s2 * s2)
                    / (2.0 * direct_extinction)
                    + hh3 * leaf_stem_area
                    + h1 / (2.0 * direct_extinction * zmu2)
                        * (leaf_stem_area * s2 * s2 - (1.0 - s2 * s2) / (2.0 * direct_extinction)),
                (hh5 - (h1 * p4 / (4.0 * direct_extinction * direct_extinction * zmu) + de) / ce)
                    * (1.0 - s2 * s2)
                    / (2.0 * direct_extinction)
                    + hh6 * leaf_stem_area
                    + h1 * p3 / (ce * 4.0 * direct_extinction * direct_extinction * zmu2)
                        * (leaf_stem_area * s2 * s2 - (1.0 - s2 * s2) / (2.0 * direct_extinction)),
            )
        };
        let direct_sunlit_bracket = (eup_direct + edown_direct).mul_add(1.0 / zmu, 1.0 - s2);
        sunlit_absorption[band][0] = direct_sunlit_bracket * (1.0 - scattering);
        // Preserve the linked original's fused products and evaluation order.
        let absorption_scale = (1.0 - scattering) / zmu;
        let reflected_direct = ground[band][1].mul_add(transmission_direct, ground[band][0] * s2)
            - transmission_direct;
        let shaded_direct = scattering.mul_add(1.0 - s2, reflected_direct) - albedo_direct;
        shaded_absorption[band][0] =
            (-(eup_direct + edown_direct)).mul_add(absorption_scale, shaded_direct);
        albedo[band][0] = albedo_direct;
        transmission[band][0] = transmission_direct;

        // 漫射支：`m1`/`m2`/`n1`/`n2` 与两个 Cramer 分母沿用上面那一组
        // （GIMPLE 在这里没有重算），`m3 = 0`、`n3 = 1`。
        let hh7 = -m2 / cramer_direct;
        let hh8 = -m1 / cramer_reverse;
        let hh9 = hh7 * p1 / ce;
        let hh10 = hh8 * p2 / ce;
        let transmission_diffuse = s1.mul_add(hh9, hh10 / s1);
        let (eup_diffuse, edown_diffuse) = if sigma.abs() > 1.0e-10 {
            (
                hh7 * s1.mul_add(-s2, 1.0) / (direct_extinction + psi)
                    + hh8 * (1.0 - s2 / s1) / (direct_extinction - psi),
                hh9 * s1.mul_add(-s2, 1.0) / (direct_extinction + psi)
                    + hh10 * (1.0 - s2 / s1) / (direct_extinction - psi),
            )
        } else {
            (
                hh7 * (1.0 - s1 * s2) / (direct_extinction + psi) + hh8 * leaf_stem_area,
                hh9 * (1.0 - s1 * s2) / (direct_extinction + psi) + hh10 * leaf_stem_area,
            )
        };
        let albedo_diffuse = hh7 + hh8;
        sunlit_absorption[band][1] = absorption_scale * (eup_diffuse + edown_diffuse);
        shaded_absorption[band][1] = transmission_diffuse
            .mul_add(ground[band][1] - 1.0, -(albedo_diffuse - 1.0))
            - absorption_scale * (eup_diffuse + edown_diffuse);
        albedo[band][1] = albedo_diffuse;
        transmission[band][1] = transmission_diffuse;
        transmission[band][2] = s2;
    }

    Ok(TwoStreamRadiation {
        albedo,
        transmission,
        thermal_gap_fraction,
        direct_extinction,
        diffuse_extinction,
        sunlit_absorption,
        shaded_absorption,
    })
}

/// PFT-vector `twostream_mod`, distinct from the land-cover `twostream` above.
///
/// 逐句照 `MOD_Albedo.F90:783-1166` 的 GIMPLE（`twostream_mod` 内联进 `twostream_wrap`）重写；
/// 每处 `mul_add` 旁注行号。几条容易看漏的：`beta0` 的第一个定义是死代码；叶面雪混合用的是
/// **更新后**的 `scat`；`ic = 1` 的多次反射修正读的是**修正前**的 `sall(iw,2)`。
fn two_stream_mod(
    optics: LeafOptics,
    lai: f64,
    sai: f64,
    wet_snow_fraction: f64,
    cosine_zenith: f64,
    ground: [[f64; RADIATION_TYPES]; BANDS],
    vegetation_snow: bool,
) -> Result<TwoStreamRadiation> {
    const BLACK: f64 = 1.0e-6;
    let chil = optics.chil;
    // `:909-910` `phi1 = .FNMA (chil, chil*0.33, .FNMA (chil, 0.633, 0.5))`
    let phi1 = (-chil).mul_add(chil * 0.33, (-chil).mul_add(0.633, 0.5));
    let phi2 = (1.0 - phi1 * 2.0) * 0.877;
    let zmu = two_stream_zmu(phi1, phi2);
    let lsai = lai + sai;
    ensure!(
        zmu.is_finite() && zmu > 0.0 && lsai > 1.0e-6,
        "invalid PFT two-stream canopy"
    );
    let zmu2 = zmu * zmu;
    let inv_zmu = 1.0 / zmu;
    // `power3 = max(1e-5, min(50, lsai/zmu))`：两步写法（`clamp` 在 NaN 上语义不同）
    #[allow(clippy::manual_clamp)]
    let thermal_gap_fraction = (-(lsai / zmu).min(50.0).max(1.0e-5)).exp();
    // `:929-930` `cosdif = -(tmptau / log(exp(-tmptau*0.87) / .FMA (tmptau, 0.92, 1)))`
    let tmptau = lsai * 0.5;
    let cosdif = -(tmptau / ((-(tmptau * 0.87)).exp() / tmptau.mul_add(0.92, 1.0)).ln());
    let lai_weight = lai / lsai;
    let sai_weight = sai / lsai;
    // `:975/979` `((1+chil)*0.5)**2`、`(1+chil)**2`
    let chil_half_sq = {
        let half = (chil + 1.0) * 0.5;
        half * half
    };
    let chil_sq = (chil + 1.0) * (chil + 1.0);

    let mut albedo = [[0.0; RADIATION_TYPES]; BANDS];
    let mut transmission = [[0.0; 3]; BANDS];
    let mut sunlit_absorption = [[0.0; RADIATION_TYPES]; BANDS];
    let mut shaded_absorption = [[0.0; RADIATION_TYPES]; BANDS];
    let mut direct_extinction = 0.0;
    // `sigma` 很小的那一支不给 `hh1`/`hh4` 赋值，上游沿用上一次迭代（或上一波段）的值
    let (mut hh1_kept, mut hh4_kept) = (0.0, 0.0);

    for band in 0..BANDS {
        let mut sall = [0.0; RADIATION_TYPES];
        let mut s2d = 0.0;
        let mut extkbd = 0.0;
        // 第二个 `ic` 循环要用的量（上游是循环变量，退出 `ic` 循环后仍是 `ic = 2` 的值）
        let mut last = (0.0, 0.0, [0.0; 6], 0.0, 0.0, 0.0);
        for ic in 0..RADIATION_TYPES {
            let cosz = if ic == 1 {
                // `:941-946` `theta = .FMA (acos(max(cosdif,0.001))/3.14159, 180, chil*5)`
                let theta = (cosdif.max(0.001).acos() / FORTRAN_PI).mul_add(180.0, chil * 5.0);
                (theta / 180.0 * FORTRAN_PI).cos()
            } else {
                cosine_zenith
            };
            // `:951` `proj = .FMA (phi2, cosz, phi1)`
            let proj = phi2.mul_add(cosz, phi1);
            let extkb = proj / cosz;
            // `:960-961` `.FMA (lai/lsai, tau1, (sai/lsai)*tau2)`
            let wtau = lai_weight.mul_add(
                optics.transmittance[band][0],
                sai_weight * optics.transmittance[band][1],
            );
            let wrho = lai_weight.mul_add(
                optics.reflectance[band][0],
                sai_weight * optics.reflectance[band][1],
            );
            let mut scat = wtau + wrho;
            // `:975` `0.5 * .FMA (((1+chil)*0.5)**2, .FNMA (wtau, 2, scat), scat)`
            let mut upscat = chil_half_sq.mul_add((-wtau).mul_add(2.0, scat), scat) * 0.5;
            // `:979` `(.FMA (((1+chil)**2*(1/extkb))*0.25, wrho-wtau, scat)*0.5)/scat`
            let mut beta0 =
                (((chil_sq * (1.0 / extkb)) * 0.25).mul_add(wrho - wtau, scat) * 0.5) / scat;
            if vegetation_snow {
                // `:985-987`：`scat_sno = (0.8, 0.4)`，`upscat_sno = beta0_sno = 0.5`
                let snow = if band == 0 { 0.8 } else { 0.4 };
                let dry = 1.0 - wet_snow_fraction;
                let wet = wet_snow_fraction * snow;
                scat = scat.mul_add(dry, wet);
                let dry_scat = dry * scat;
                let wet_half = wet * 0.5;
                upscat = upscat.mul_add(dry_scat, wet_half) / scat;
                beta0 = beta0.mul_add(dry_scat, wet_half) / scat;
            }
            let be = (1.0 - scat) + upscat;
            let ce = upscat;
            // `:996-997` `de = (extkb*(zmu*scat))*beta0`
            let zmu_scat_extkb = extkb * (zmu * scat);
            let de = zmu_scat_extkb * beta0;
            let fe = zmu_scat_extkb * (1.0 - beta0);
            // `:999` `psi = sqrt(.FMS (be, be, ce*ce))/zmu`
            let ce_sq = ce * ce;
            let psi = be.mul_add(be, -ce_sq).sqrt() / zmu;
            let s1 = (-(lsai * psi).min(50.0)).exp();
            let s2 = (-(lsai * extkb).min(50.0)).exp();
            // `:1011-1014`
            let zmu_extkb = zmu * extkb;
            let p1 = zmu.mul_add(psi, be);
            let p2 = (-zmu).mul_add(psi, be);
            let p3 = zmu_extkb + be;
            let p4 = be - zmu_extkb;
            let f1 = 1.0 - (p1 * BLACK) / ce;
            let f2 = 1.0 - (p2 * BLACK) / ce;
            // `:1019-1022`
            let h1 = -de.mul_add(p4, ce * fe);
            let h4 = -fe.mul_add(p3, ce * de);
            let sigma = zmu_extkb.mul_add(zmu_extkb, (-be).mul_add(be, ce_sq));
            if ic == 0 {
                s2d = s2;
                extkbd = extkb;
            }
            // 公共因子（`:1034-1085`，两支共用）
            let m1 = s1 * f1;
            let m2 = f2 / s1;
            let n1 = p1 / ce;
            let n2 = p2 / ce;
            let m2n1 = m2 * n1;
            let denominator_2 = m1.mul_add(n2, -m2n1);
            let denominator_3 = (-m1).mul_add(n2, m2n1);
            let one_minus_s2s2d = (-s2).mul_add(s2d, 1.0);
            let extkb_sum = extkb + extkbd;
            let (hh, eup, edw, albv, tran);
            if sigma.abs() > 1.0e-10 {
                let hh1 = h1 / sigma;
                let hh4 = h4 / sigma;
                // `:1036` `m3 = s2 * (1e-6 - .FNMA (hh4, 1e-6, hh1))`
                let m3 = s2 * (BLACK - (-hh4).mul_add(BLACK, hh1));
                let n3 = -hh4;
                let hh2 = m3.mul_add(n2, -(n3 * m2)) / denominator_2;
                let hh3 = m3.mul_add(n1, -(n3 * m1)) / denominator_3;
                let hh5 = (p1 * hh2) / ce;
                let hh6 = (p2 * hh3) / ce;
                albv = (hh1 + hh2) + hh3;
                // `:1049` `.FMA (s2, hh4, s1*hh5) + hh6/s1`
                tran = s2.mul_add(hh4, s1 * hh5) + hh6 / s1;
                // `:1053/1057`
                let one_minus_s1s2d = (-s1).mul_add(s2d, 1.0);
                let psi_sum = psi + extkbd;
                let one_minus_ratio = 1.0 - s2d / s1;
                let psi_diff = extkbd - psi;
                eup = ((hh1 * one_minus_s2s2d) / extkb_sum + (hh2 * one_minus_s1s2d) / psi_sum)
                    + (hh3 * one_minus_ratio) / psi_diff;
                edw = ((hh4 * one_minus_s2s2d) / extkb_sum + (hh5 * one_minus_s1s2d) / psi_sum)
                    + (hh6 * one_minus_ratio) / psi_diff;
                hh1_kept = hh1;
                hh4_kept = hh4;
                hh = [hh1, hh2, hh3, hh4, hh5, hh6];
            } else {
                // `:1066`
                let term_a = (h1 / zmu2) * (lsai + 1.0 / extkb_sum);
                let p_combo = lsai.mul_add(p3, p4 / extkb_sum);
                let inner = (-((h1 / extkb_sum) / zmu2)).mul_add(p_combo, -de);
                let m3 = s2.mul_add(BLACK, s2.mul_add(term_a, s2 * ((BLACK / ce) * inner)));
                // `:1070`
                let p4h1 = p4 * h1;
                let extkb_sum_sq = extkb_sum * extkb_sum;
                let n3 = (1.0 / ce) * (de + (p4h1 / extkb_sum_sq) / zmu2);
                let hh2 = m3.mul_add(n2, -(n3 * m2)) / denominator_2;
                let hh3 = m3.mul_add(n1, -(n3 * m1)) / denominator_3;
                let hh5 = (p1 * hh2) / ce;
                let hh6 = (p2 * hh3) / ce;
                // `:1078` `albv = (hh2 - h1/(zmu2*e)) + hh3`
                let h1_over = h1 / (zmu2 * extkb_sum);
                let hh2_shift = hh2 - h1_over;
                albv = hh3 + hh2_shift;
                // `:1081`
                let bracket = (-p_combo).mul_add(h1_over, -de);
                tran = s2.mul_add((1.0 / ce) * bracket, s1 * hh5) + hh6 / s1;
                // `:1085` `eup = .FMA (h1/(zmu2 e), .FMS (s2d, lsai*s2, (1-s2 s2d)/e), .FMA (hh3, lsai, (hh2-x)(1-s2 s2d)/e))`
                let lsai_term = s2d.mul_add(lsai * s2, -(one_minus_s2s2d / extkb_sum));
                eup = h1_over.mul_add(
                    lsai_term,
                    hh3.mul_add(lsai, (hh2_shift * one_minus_s2s2d) / extkb_sum),
                );
                // `:1090`
                let hh5_shift = hh5 - (de + p4h1 / (zmu * extkb_sum_sq)) / ce;
                let p3_term = (p3 * h1) / (((ce * extkb_sum) * extkb_sum) * zmu2);
                edw = lsai_term.mul_add(
                    p3_term,
                    hh6.mul_add(lsai, (hh5_shift * one_minus_s2s2d) / extkb_sum),
                );
                hh = [hh1_kept, hh2, hh3, hh4_kept, hh5, hh6];
            }
            albedo[band][ic] = albv;
            transmission[band][ic] = tran;
            // `:1094` `sall = .FNMA (tran + s2, 1-1e-6, 1-albv)`
            sall[ic] = (-(s2 + tran)).mul_add(1.0 - BLACK, 1.0 - albv);
            // `:1097/1099`
            let fluxes = inv_zmu * (edw + eup);
            let sunlit = if ic == 0 {
                ((1.0 - s2) + fluxes) * (1.0 - scat)
            } else {
                ((extkb * one_minus_s2s2d) / extkb_sum + fluxes) * (1.0 - scat)
            };
            sunlit_absorption[band][ic] = sunlit;
            shaded_absorption[band][ic] = sall[ic] - sunlit;
            last = (extkb, psi, hh, s1, s2, scat);
        }
        // `:1109-1116`（`ic = 2` 退出循环后的变量）
        let (extkb, psi, hh, s1, s2, scat) = last;
        let one_minus_s2 = 1.0 - s2 / s2d;
        let extkb_diff = extkb - extkbd;
        let one_minus_s1 = 1.0 - s1 / s2d;
        let psi_diff = psi - extkbd;
        let inverse_term = (1.0 / s1) / s2d - 1.0;
        let psi_sum = psi + extkbd;
        let eup = ((one_minus_s2 * hh[0]) / extkb_diff + (one_minus_s1 * hh[1]) / psi_diff)
            + (inverse_term * hh[2]) / psi_sum;
        let edw = ((one_minus_s2 * hh[3]) / extkb_diff + (one_minus_s1 * hh[4]) / psi_diff)
            + (inverse_term * hh[5]) / psi_sum;
        let ssun_rev = (s2d * (1.0 - scat))
            * (eup + edw).mul_add(inv_zmu, (extkb * one_minus_s2) / extkb_diff);
        // `:1135-1156`：先 `ic = 1`（读修正前的 `sall(iw,2)`），再 `ic = 2`
        let albg_dir = ground[band][0];
        let albg_dif = ground[band][1];
        let albv_dif = albedo[band][1];
        let one_minus_q = (-albg_dif).mul_add(albv_dif, 1.0);
        let s2d_albg = s2d * albg_dir;
        let tran_dir = albv_dif.mul_add(s2d_albg, transmission[band][0]) / one_minus_q;
        transmission[band][0] = tran_dir;
        let reflected = albg_dif.mul_add(tran_dir, s2d_albg);
        let sall_dif_before = sall[1];
        sall[0] = reflected.mul_add(sall_dif_before, sall[0]);
        albedo[band][0] = (-s2d).mul_add(
            1.0 - albg_dir,
            (-tran_dir).mul_add(1.0 - albg_dif, 1.0 - sall[0]),
        );
        sunlit_absorption[band][0] = ssun_rev.mul_add(reflected, sunlit_absorption[band][0]);
        shaded_absorption[band][0] = sall[0] - sunlit_absorption[band][0];
        let tran_dif = (s2 + transmission[band][1]) / one_minus_q;
        transmission[band][1] = tran_dif;
        let reflected_dif = albg_dif * tran_dif;
        sall[1] = sall[1].mul_add(reflected_dif, sall[1]);
        albedo[band][1] = (-tran_dif).mul_add(1.0 - albg_dif, 1.0 - sall[1]);
        sunlit_absorption[band][1] = ssun_rev.mul_add(reflected_dif, sunlit_absorption[band][1]);
        shaded_absorption[band][1] = sall[1] - sunlit_absorption[band][1];
        transmission[band][2] = s2d;
        direct_extinction = extkbd;
    }
    Ok(TwoStreamRadiation {
        albedo,
        transmission,
        thermal_gap_fraction,
        direct_extinction,
        diffuse_extinction: 0.719,
        sunlit_absorption,
        shaded_absorption,
    })
}

#[cfg(test)]
#[path = "radiation_tests.rs"]
mod radiation_tests;

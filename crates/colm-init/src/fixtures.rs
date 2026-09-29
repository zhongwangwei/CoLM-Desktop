//! One synthetic restart pair that downstream stages can assemble from.
//!
//! The initializer's own tests build their inputs inline and then throw the files
//! away.  A later stage — the runtime's restart-to-driver-template assembly — needs
//! the opposite: files whose every value is known, so it can prove that what it read
//! is what was written.  Keeping that fixture here means there is one definition of
//! "a written CoLM restart" instead of one per consumer, and the values are chosen
//! to be distinguishable per patch, per layer and per band so a transposed axis
//! cannot pass by coincidence.
//!
//! Compiled only under the `fixtures` feature; nothing here ships in a binary.

use std::path::{Path, PathBuf};

use anyhow::{ensure, Result};

use crate::{
    derive_lake_layers, derive_soil_parameters, write_constant_restart, write_time_restart,
    CanopyState, CanopyStructureFields, ConstantRestartFiles, HydraulicModel, LakeState,
    RestartDate, RestartDimensions, RestartPatchFields, RestartTuning, SoilAlbedo, SoilLayerInput,
    SoilState, TimeRadiationFields, TimeRestartFile, TimeRestartInput, TopmodelFields,
};

pub const PATCHES: usize = 2;
pub const SOIL_LAYERS: usize = 10;
pub const SNOW_LAYERS: usize = 5;
/// CoLM 的湖泊层数是编译期常量，`derive_lake_layers` 只接受 10。
pub const LAKE_LAYERS: usize = 10;
pub const BANDS: usize = 2;
pub const RADIATION_TYPES: usize = 2;
pub const WAVELENGTHS: usize = 3;

/// A written constant/time restart pair plus every value that went into it.
///
/// Fields are public on purpose: a consumer asserts against the same numbers the
/// writer received, not against numbers it recomputed from the files it is testing.
#[derive(Debug, Clone)]
pub struct SyntheticRestart {
    pub root: PathBuf,
    pub constant: ConstantRestartFiles,
    pub time: TimeRestartFile,
    pub tuning: RestartTuning,
    pub soil: SoilState,
    pub soil_albedo: SoilAlbedoValues,
    pub canopy: CanopyState,
    pub canopy_structure: CanopyStructureValues,
    pub lake: LakeState,
    pub topmodel: TopmodelValues,
    /// `soilsnow * patches`, snow slots first — the restart's own column layout.
    pub temperature_k: Vec<f64>,
    pub liquid_water_kg_m2: Vec<f64>,
    pub ice_water_kg_m2: Vec<f64>,
    /// `patches`.
    pub leaf_temperature_k: Vec<f64>,
    pub canopy_water_mm: Vec<f64>,
    pub canopy_rain_mm: Vec<f64>,
    pub canopy_snow_mm: Vec<f64>,
    pub wet_snow_fraction: Vec<f64>,
    pub lai: Vec<f64>,
    pub sai: Vec<f64>,
    pub water_table_depth_m: Vec<f64>,
    pub aquifer_water_mm: Vec<f64>,
    pub surface_water_mm: Vec<f64>,
    pub snow_age: Vec<f64>,
    pub thermal_gap_fraction: Vec<f64>,
    pub direct_extinction: Vec<f64>,
    pub diffuse_extinction: Vec<f64>,
    /// `band * radiation_types * patches`, patch last — the writer's memory order.
    pub albedo: Vec<f64>,
    pub sunlit_absorption: Vec<f64>,
    pub shaded_absorption: Vec<f64>,
    pub soil_absorption: Vec<f64>,
    pub snow_absorption: Vec<f64>,
    /// `Some` 表示时间重启里带了一份自洽的雪列；`None` 是无雪算例。
    pub snow: Option<SyntheticSnowValues>,
}

/// 写进去的雪列，按槽位（Fortran `-4..0`）给出，供下游逐项核对。
#[derive(Debug, Clone)]
pub struct SyntheticSnowValues {
    pub depth_m: f64,
    pub water_equivalent_kg_m2: f64,
    pub ground_snow_fraction: f64,
    /// 上游从水量数出来的层数（负值），是装配层要自己复现的那个量。
    pub layer_count: i32,
    pub node_depth_m: Vec<f64>,
    pub thickness_m: Vec<f64>,
    pub temperature_k: Vec<f64>,
    pub liquid_water_kg_m2: Vec<f64>,
    pub ice_water_kg_m2: Vec<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoilAlbedoValues {
    pub saturated_visible: f64,
    pub dry_visible: f64,
    pub saturated_near_infrared: f64,
    pub dry_near_infrared: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanopyStructureValues {
    pub needleleaf_crown_depth_m: f64,
    pub needleleaf_crown_width_m: f64,
    pub broadleaf_crown_width_m: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TopmodelValues {
    pub topographic_index: f64,
    pub saturated_fraction_max: f64,
    pub saturated_fraction_decay: f64,
    pub alpha_twi: f64,
    pub chi_twi: f64,
    pub mu_twi: f64,
}

/// 合成算例的雪配置。
///
/// 默认是**无雪**（雪槽全零）：装配层会核对这一点，带着雪槽跑无雪分支正是它该拒绝的。
/// `write_with_snow` 给的是一份自洽的雪列 —— 层数由共享的 `initialize_snow_layers`
/// 从雪深推出，水量按层厚分摊，装配层则要**自己**从水量把层数数回来
/// （上游 `CoLMMAIN.F90:816-818` 就是这么做的，重启里没有层数）。
#[derive(Debug, Clone, Copy)]
pub struct SyntheticSnow {
    pub depth_m: f64,
    pub water_equivalent_kg_m2: f64,
    pub ground_snow_fraction: f64,
    pub temperature_k: f64,
}

impl SyntheticRestart {
    /// Writes both restart families under `root` and returns their values.
    pub fn write(root: impl AsRef<Path>) -> Result<Self> {
        Self::write_inner(root, None, true)
    }

    /// 与 [`Self::write`] 相同，但常数重启**不带**那五个 van Genuchten 场。
    ///
    /// 上游只在选中该关系时才写它们（`restart.rs` 的 `uses_van_genuchten`
    /// 来自 `ConstantRestartInput`），所以 Campbell 算例的重启就是少这五个变量 ——
    /// 实测真实算例同样如此（`alpha_vgm`/`n_vgm`/`L_vgm`/`sc_vgm`/`fc_vgm`）。
    pub fn write_campbell(root: impl AsRef<Path>) -> Result<Self> {
        Self::write_inner(root, None, false)
    }

    /// 与 [`Self::write`] 相同，但时间重启带一份自洽的雪列。
    pub fn write_with_snow(root: impl AsRef<Path>, snow: SyntheticSnow) -> Result<Self> {
        ensure!(
            snow.depth_m > 0.0 && snow.water_equivalent_kg_m2 > 0.0,
            "a synthetic snow column needs a positive depth and water equivalent"
        );
        Self::write_inner(root, Some(snow), true)
    }

    fn write_inner(
        root: impl AsRef<Path>,
        snow: Option<SyntheticSnow>,
        uses_van_genuchten: bool,
    ) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let dimensions = RestartDimensions {
            lake_layers: LAKE_LAYERS,
            wavelengths: WAVELENGTHS,
            ..RestartDimensions::default()
        };
        let soil = soil_state();
        let canopy = CanopyState {
            patch_top_m: per_patch(10.0, 1.0),
            patch_bottom_m: per_patch(1.0, 0.5),
            pft_top_m: Vec::new(),
            pft_bottom_m: Vec::new(),
        };
        let canopy_structure = CanopyStructureValues {
            needleleaf_crown_depth_m: 2.5,
            needleleaf_crown_width_m: 3.5,
            broadleaf_crown_width_m: 4.5,
        };
        let lake = derive_lake_layers(&per_patch(20.0, 5.0), LAKE_LAYERS)?;
        let soil_albedo = SoilAlbedoValues {
            saturated_visible: 0.11,
            dry_visible: 0.21,
            saturated_near_infrared: 0.31,
            dry_near_infrared: 0.41,
        };
        let topmodel = TopmodelValues {
            topographic_index: 6.5,
            saturated_fraction_max: 0.55,
            saturated_fraction_decay: 0.45,
            alpha_twi: 7.5,
            chi_twi: 8.5,
            mu_twi: 9.5,
        };
        let tuning = RestartTuning {
            zlnd: 0.01,
            zsno: 0.002,
            csoilc: 0.004,
            dewmx: 0.1,
            capr: 0.34,
            cnfac: 0.5,
            ssi: 2.0,
            wimp: 0.05,
            pondmx: 10.0,
            smpmax: -1.0e8,
            smpmin: -1.0e8,
            smpmax_hr: -1.0e8,
            smpmin_hr: -1.0e8,
            trsmx0: 2.0e-4,
            tcrit: 2.5,
            wetwatmax: 0.4,
            land_class: colm_core::LandClassOverrides::default(),
        };
        let mut temperature_k = column(280.0, 0.5, 1.0);
        let mut liquid_water_kg_m2 = soil_water_column(0.4 * 0.5, 1000.0, 2.0);
        let mut ice_water_kg_m2 = soil_water_column(0.4 * 0.05, 917.0, 0.5);
        // 土壤水必须相对**共享土层厚度**是合理的：`colm_soil_grid` 的表层只有约
        // 2 cm，随手给 10 kg/m2 会得到 0.5 以上的体积含水率（超过孔隙度），
        // 下游的应力和通量核对就会拒绝这份"重启"。这里按孔隙度 0.4 取 50% 饱和。
        let leaf_temperature_k = per_patch(290.0, 3.0);
        let albedo = patch_last_3d_axis(|band, radiation_type, patch| {
            0.01 + 0.1 * band as f64 + 0.2 * radiation_type as f64 + 0.3 * patch as f64
        });
        let sunlit_absorption = patch_last_3d_axis(|band, radiation_type, patch| {
            0.02 + 0.1 * band as f64 + 0.2 * radiation_type as f64 + 0.3 * patch as f64
        });
        let shaded_absorption = patch_last_3d_axis(|band, radiation_type, patch| {
            0.03 + 0.1 * band as f64 + 0.2 * radiation_type as f64 + 0.3 * patch as f64
        });
        let soil_absorption = patch_last_3d_axis(|band, radiation_type, patch| {
            0.04 + 0.1 * band as f64 + 0.2 * radiation_type as f64 + 0.3 * patch as f64
        });
        let snow_absorption = patch_last_3d_axis(|band, radiation_type, patch| {
            0.05 + 0.1 * band as f64 + 0.2 * radiation_type as f64 + 0.3 * patch as f64
        });
        let snow_aerosol = vec![0.0; SNOW_LAYERS * PATCHES];
        // 雪列的槽位下标与时间重启里的数组下标恒等（Fortran `-4..0`）。
        let (snow_node_depth, snow_layer_thickness, snow_temperature, snow_liquid, snow_ice) =
            match snow {
                None => (
                    vec![0.0; SNOW_LAYERS * PATCHES],
                    vec![0.0; SNOW_LAYERS * PATCHES],
                    vec![0.0; SNOW_LAYERS * PATCHES],
                    vec![0.0; SNOW_LAYERS * PATCHES],
                    vec![0.0; SNOW_LAYERS * PATCHES],
                ),
                Some(snow) => {
                    // 层数与厚度用共享的冷启分层；这里只是要一份**自洽**的列。
                    let state = crate::initialize_snow_layers(0, snow.depth_m, SNOW_LAYERS)?;
                    let layers = state.layer_count.unsigned_abs() as usize;
                    ensure!(
                        layers > 0,
                        "the requested snow depth produces no snow layer"
                    );
                    let mut node = vec![0.0; SNOW_LAYERS * PATCHES];
                    let mut thickness = vec![0.0; SNOW_LAYERS * PATCHES];
                    let mut temperature = vec![0.0; SNOW_LAYERS * PATCHES];
                    let mut liquid = vec![0.0; SNOW_LAYERS * PATCHES];
                    let mut ice = vec![0.0; SNOW_LAYERS * PATCHES];
                    let used = SNOW_LAYERS - layers;
                    let total_thickness: f64 = state.thickness_m[used..].iter().sum();
                    for slot in used..SNOW_LAYERS {
                        let share = state.thickness_m[slot] / total_thickness;
                        for patch in 0..PATCHES {
                            let index = slot * PATCHES + patch;
                            node[index] = state.node_depth_m[slot];
                            thickness[index] = state.thickness_m[slot];
                            temperature[index] = snow.temperature_k;
                            // 水量按层厚分摊，层数才能被装配层从水量数回来。
                            ice[index] = snow.water_equivalent_kg_m2 * share;
                            // 留一点液态水，好让固态分数不是 1。
                            liquid[index] = 0.1 * snow.water_equivalent_kg_m2 * share;
                        }
                    }
                    (node, thickness, temperature, liquid, ice)
                }
            };
        let soil_only = vec![0.0; SOIL_LAYERS * PATCHES];
        let lake_column = vec![0.0; LAKE_LAYERS * PATCHES];
        let snow_layer_absorption =
            vec![0.0; BANDS * RADIATION_TYPES * (SNOW_LAYERS + 1) * PATCHES];

        // 雪槽按构造出来的列覆盖；无雪时它们本来就是 0。
        for slot in 0..SNOW_LAYERS {
            for patch in 0..PATCHES {
                let index = slot * PATCHES + patch;
                temperature_k[index] = snow_temperature[index];
                liquid_water_kg_m2[index] = snow_liquid[index];
                ice_water_kg_m2[index] = snow_ice[index];
            }
        }

        let soil_albedo_patch = [
            per_patch(soil_albedo.saturated_visible, 0.01),
            per_patch(soil_albedo.dry_visible, 0.01),
            per_patch(soil_albedo.saturated_near_infrared, 0.01),
            per_patch(soil_albedo.dry_near_infrared, 0.01),
        ];
        let structure = [
            per_patch(canopy_structure.needleleaf_crown_depth_m, 0.5),
            per_patch(canopy_structure.needleleaf_crown_width_m, 0.5),
            per_patch(canopy_structure.broadleaf_crown_width_m, 0.5),
        ];
        let topmodel_values = [
            per_patch(topmodel.topographic_index, 1.0),
            per_patch(topmodel.saturated_fraction_max, 0.05),
            per_patch(topmodel.saturated_fraction_decay, 0.05),
            per_patch(topmodel.alpha_twi, 1.0),
            per_patch(topmodel.chi_twi, 1.0),
            per_patch(topmodel.mu_twi, 1.0),
        ];
        // `patchclass` 就是查表用的地类号：`MOD_Const_LC` 的数值表维度是
        // `(N_land_classification)`（位置 1..17），上游按 `patchtypes(SITE_landtype)`
        // 查，所以位置号 = 地类号，不要再加一。取 1 与 3 而不是 2 与 3：
        // IGBP 第 1 类是土壤，而 USGS 第 1 类是城市（patchtype 1）—— 于是
        // 「地类表与重启对不上」这条防线在换成 USGS 时真的会被触发，测试才有意义。
        let class = vec![1, 3];
        let kind = vec![0, 0];
        let mask = vec![true, false];
        let longitude = per_patch(1.0, 1.0);
        let latitude = per_patch(3.0, 1.0);
        let soil_texture = vec![3, 4];
        let bvic = per_patch(0.9, 1.0);
        let vic = [
            per_patch(1.0, 1.0),
            per_patch(3.0, 1.0),
            per_patch(5.0, 1.0),
            per_patch(7.0, 1.0),
            per_patch(9.0, 1.0),
        ];
        let elevation_mean = per_patch(11.0, 1.0);
        let elevation_std = per_patch(13.0, 1.0);
        let slope_ratio = per_patch(15.0, 1.0);
        let canopy_water_mm = per_patch(0.4, 0.1);
        let canopy_rain_mm = per_patch(0.5, 0.1);
        let canopy_snow_mm = per_patch(0.6, 0.1);
        let wet_snow_fraction = per_patch(0.7, 0.1);
        let lai = per_patch(2.0, 0.5);
        let sai = per_patch(0.5, 0.25);
        let water_table_depth_m = per_patch(1.0, 0.5);
        let aquifer_water_mm = per_patch(100.0, 10.0);
        let surface_water_mm = per_patch(0.0, 1.0);
        let snow_age = per_patch(0.1, 0.1);
        let snow_water_equivalent_mm = match snow {
            None => zeros(),
            Some(snow) => vec![snow.water_equivalent_kg_m2; PATCHES],
        };
        let snow_depth_column = match snow {
            None => zeros(),
            Some(snow) => vec![snow.depth_m; PATCHES],
        };
        let ground_snow_fraction = match snow {
            None => zeros(),
            Some(snow) => vec![snow.ground_snow_fraction; PATCHES],
        };
        let thermal_gap_fraction = per_patch(0.2, 0.1);
        let direct_extinction = per_patch(0.3, 0.1);
        let diffuse_extinction = per_patch(0.4, 0.1);

        let constant = write_constant_restart(
            &root,
            "CN-Cng",
            2005,
            "w180_s90",
            crate::ConstantRestartInput {
                dimensions,
                compression_level: 1,
                patch: RestartPatchFields {
                    class: &class,
                    kind: &kind,
                    mask: &mask,
                    longitude_radians: &longitude,
                    latitude_radians: &latitude,
                    albedo: SoilAlbedo {
                        saturated_visible: &soil_albedo_patch[0],
                        dry_visible: &soil_albedo_patch[1],
                        saturated_near_infrared: &soil_albedo_patch[2],
                        dry_near_infrared: &soil_albedo_patch[3],
                    },
                    bvic: &bvic,
                    soil_texture: &soil_texture,
                    vic_b_infilt: &vic[0],
                    vic_dsmax: &vic[1],
                    vic_ds: &vic[2],
                    vic_ws: &vic[3],
                    vic_c: &vic[4],
                    elevation_mean_m: &elevation_mean,
                    elevation_std_m: &elevation_std,
                    slope_ratio: &slope_ratio,
                },
                soil: &soil,
                lake: &lake,
                lake_soil_carbon: None,
                canopy: &canopy,
                canopy_structure: Some(CanopyStructureFields {
                    needleleaf_crown_depth_m: &structure[0],
                    needleleaf_crown_width_m: &structure[1],
                    broadleaf_crown_width_m: &structure[2],
                }),
                tuning,
                uses_van_genuchten,
                bedrock: None,
                topmodel: Some(TopmodelFields {
                    topographic_index: &topmodel_values[0],
                    saturated_fraction_max: &topmodel_values[1],
                    saturated_fraction_decay: &topmodel_values[2],
                    alpha_twi: &topmodel_values[3],
                    chi_twi: &topmodel_values[4],
                    mu_twi: &topmodel_values[5],
                }),
                terrain: None,
                simple_terrain: None,
                hyperspectral_albedo: None,
            },
        )?;

        let time = write_time_restart(
            &root,
            "CN-Cng",
            2005,
            RestartDate {
                year: 2005,
                julian_day: 1,
                seconds: 0,
            },
            "w180_s90",
            TimeRestartInput {
                dimensions: crate::TimeRestartDimensions {
                    soil_layers: SOIL_LAYERS,
                    lake_layers: LAKE_LAYERS,
                    snow_layers: SNOW_LAYERS,
                    bands: BANDS,
                    radiation_types: RADIATION_TYPES,
                },
                compression_level: 1,
                snow_soil: crate::SnowSoilRestartFields {
                    snow_node_depth_m: &snow_node_depth,
                    snow_layer_thickness_m: &snow_layer_thickness,
                    temperature_k: &temperature_k,
                    liquid_water_kg_m2: &liquid_water_kg_m2,
                    ice_water_kg_m2: &ice_water_kg_m2,
                    matric_potential_mm: &soil_only,
                    hydraulic_conductivity_mm_s: &soil_only,
                },
                patch: crate::TimePatchFields {
                    ground_temperature_k: &leaf_temperature_k,
                    leaf_temperature_k: &leaf_temperature_k,
                    canopy_water_mm: &canopy_water_mm,
                    canopy_rain_mm: &canopy_rain_mm,
                    canopy_snow_mm: &canopy_snow_mm,
                    wet_snow_fraction: &wet_snow_fraction,
                    snow_age: &snow_age,
                    snow_water_equivalent_mm: &snow_water_equivalent_mm,
                    snow_depth_m: &snow_depth_column,
                    vegetation_fraction: &per_patch(0.8, 0.1),
                    ground_snow_fraction: &ground_snow_fraction,
                    snow_free_vegetation_fraction: &per_patch(0.8, 0.1),
                    greenness: &per_patch(0.9, 0.05),
                    lai: &lai,
                    total_lai: &lai,
                    sai: &sai,
                    total_sai: &sai,
                    cosine_zenith: &per_patch(0.5, 0.1),
                    thermal_gap_fraction: &thermal_gap_fraction,
                    direct_extinction: &direct_extinction,
                    diffuse_extinction: &diffuse_extinction,
                    water_table_depth_m: &water_table_depth_m,
                    aquifer_water_mm: &aquifer_water_mm,
                    wetland_water_mm: &surface_water_mm,
                    surface_water_mm: &surface_water_mm,
                    soil_surface_resistance_s_m: &per_patch(100.0, 10.0),
                    saved_tke: &per_patch(0.0, 1.0),
                    radiative_temperature_k: &leaf_temperature_k,
                    reference_temperature_k: &leaf_temperature_k,
                    reference_humidity: &per_patch(0.01, 0.001),
                    stomatal_resistance_s_m: &per_patch(100.0, 10.0),
                    emissivity: &per_patch(0.96, 0.01),
                    roughness_length_m: &per_patch(0.1, 0.01),
                    monin_obukhov_height: &per_patch(0.0, 1.0),
                    bulk_richardson: &per_patch(0.0, 1.0),
                    friction_velocity: &per_patch(0.3, 0.01),
                    humidity_scale: &per_patch(0.0, 1.0),
                    temperature_scale_k: &per_patch(0.0, 1.0),
                    momentum_integral: &per_patch(0.0, 1.0),
                    heat_integral: &per_patch(0.0, 1.0),
                    moisture_integral: &per_patch(0.0, 1.0),
                },
                radiation: TimeRadiationFields {
                    albedo: &albedo,
                    sunlit_absorption: &sunlit_absorption,
                    shaded_absorption: &shaded_absorption,
                    soil_absorption: &soil_absorption,
                    snow_absorption: &snow_absorption,
                    snow_layer_absorption: &snow_layer_absorption,
                },
                hyperspectral: None,
                lake: crate::TimeLakeFields {
                    temperature_k: &lake_column,
                    ice_fraction: &lake_column,
                    layer_thickness_m: None,
                },
                snow_aerosol: crate::SnowAerosolFields {
                    grain_radius: &snow_aerosol,
                    black_carbon_hydrophobic: &snow_aerosol,
                    black_carbon_hydrophilic: &snow_aerosol,
                    organic_carbon_hydrophobic: &snow_aerosol,
                    organic_carbon_hydrophilic: &snow_aerosol,
                    dust_1: &snow_aerosol,
                    dust_2: &snow_aerosol,
                    dust_3: &snow_aerosol,
                    dust_4: &snow_aerosol,
                },
                plant_hydraulics: None,
                ozone: None,
                irrigation: None,
            },
        )?;

        Ok(Self {
            root,
            constant,
            time,
            tuning,
            soil,
            soil_albedo,
            canopy,
            canopy_structure,
            lake,
            topmodel,
            temperature_k,
            liquid_water_kg_m2,
            ice_water_kg_m2,
            leaf_temperature_k,
            canopy_water_mm,
            canopy_rain_mm,
            canopy_snow_mm,
            wet_snow_fraction,
            lai,
            sai,
            water_table_depth_m,
            aquifer_water_mm,
            surface_water_mm,
            snow_age,
            thermal_gap_fraction,
            direct_extinction,
            diffuse_extinction,
            albedo,
            sunlit_absorption,
            shaded_absorption,
            soil_absorption,
            snow_absorption,
            snow: snow.map(|snow| SyntheticSnowValues {
                depth_m: snow.depth_m,
                water_equivalent_kg_m2: snow.water_equivalent_kg_m2,
                ground_snow_fraction: snow.ground_snow_fraction,
                // 层数是**每个 patch** 的量：这里两个 patch 的雪列相同，取 patch 0 数。
                layer_count: -((0..SNOW_LAYERS)
                    .filter(|slot| {
                        let index = slot * PATCHES;
                        snow_ice[index] + snow_liquid[index] > 0.0
                    })
                    .count() as i32),
                node_depth_m: snow_node_depth,
                thickness_m: snow_layer_thickness,
                temperature_k: snow_temperature,
                liquid_water_kg_m2: snow_liquid,
                ice_water_kg_m2: snow_ice,
            }),
        })
    }

    /// `landpatch` count, i.e. the length of every per-patch file variable.
    pub fn patches(&self) -> usize {
        PATCHES
    }

    /// 传给写出器的内存缓冲区里，一个 `soilsnow` 列的值。
    ///
    /// **这是内存序，不是盘上序**：写出器收的是 axis-major（patch 最后）的
    /// `[[slot]][patch]`，落盘时才转成 `(patch, slot)`。装配层读盘得到的是后者，
    /// 所以断言时要分清参照的是哪一侧 —— 这里按内存序取。
    pub fn soil_column(&self, values: &[f64], patch: usize, layer: usize) -> f64 {
        values[(SNOW_LAYERS + layer) * PATCHES + patch]
    }

    /// 内存缓冲区里一个 `soil` 层维场的值（同样 patch 在最后）。
    pub fn soil_value(&self, values: &[f64], patch: usize, layer: usize) -> f64 {
        values[layer * PATCHES + patch]
    }

    /// One value of a `band`/`rtyp` field in the writer's `[band][rtyp][patch]` order.
    pub fn radiation_value(
        &self,
        values: &[f64],
        patch: usize,
        band: usize,
        radiation_type: usize,
    ) -> f64 {
        values[(band * RADIATION_TYPES + radiation_type) * PATCHES + patch]
    }
}

/// 无雪算例里恒为零的 patch 场。
fn zeros() -> Vec<f64> {
    vec![0.0; PATCHES]
}

fn per_patch(base: f64, step: f64) -> Vec<f64> {
    (0..PATCHES)
        .map(|patch| base + step * patch as f64)
        .collect()
}

/// 按共享土层的厚度生成一列土壤水，`fraction * density` 即单位体积含水当量。
fn soil_water_column(fraction: f64, density_kg_m3: f64, patch_step: f64) -> Vec<f64> {
    let grid = crate::colm_soil_grid(SOIL_LAYERS).expect("CoLM's ten-layer soil grid");
    let slots = SNOW_LAYERS + SOIL_LAYERS;
    let mut column = Vec::with_capacity(slots * PATCHES);
    for slot in 0..slots {
        for patch in 0..PATCHES {
            let value = if slot < SNOW_LAYERS {
                0.0
            } else {
                fraction * density_kg_m3 * grid.thickness_m[slot - SNOW_LAYERS]
                    + patch_step * patch as f64
            };
            column.push(value);
        }
    }
    column
}

/// `soilsnow * patches` with snow slots first, mirroring the initializer's own columns.
///
/// 雪槽恒为 0：这套算例是无雪 patch，而装配层会核对这一点 —— 雪槽带值却按无雪
/// 分支驱动，正是它该拒绝的输入。
fn column(base: f64, layer_step: f64, patch_step: f64) -> Vec<f64> {
    let slots = SNOW_LAYERS + SOIL_LAYERS;
    (0..slots)
        .flat_map(|slot| {
            (0..PATCHES).map(move |patch| {
                if slot < SNOW_LAYERS {
                    0.0
                } else {
                    base + layer_step * slot as f64 + patch_step * patch as f64
                }
            })
        })
        .collect()
}

fn patch_last_3d_axis(value: impl Fn(usize, usize, usize) -> f64) -> Vec<f64> {
    let mut output = Vec::with_capacity(BANDS * RADIATION_TYPES * PATCHES);
    for band in 0..BANDS {
        for radiation_type in 0..RADIATION_TYPES {
            for patch in 0..PATCHES {
                output.push(value(band, radiation_type, patch));
            }
        }
    }
    output
}

/// A source layer whose derived parameters are distinct per layer and per patch.
fn soil_layer(value: f64) -> SoilLayerInput {
    SoilLayerInput {
        vf_quartz: value,
        vf_gravels: value + 1.0,
        vf_om: value + 2.0,
        vf_sand: value + 3.0,
        vf_clay: value + 4.0,
        wf_gravels: value + 5.0,
        wf_sand: value + 6.0,
        wf_clay: value + 7.0,
        wf_om: value + 8.0,
        om_density: value + 9.0,
        bulk_density: value + 10.0,
        theta_s: 0.4,
        psi_s_cm: -10.0,
        lambda: 0.2,
        theta_r: 0.05,
        alpha_vgm: 0.02,
        l_vgm: 0.5,
        n_vgm: 1.5,
        k_s_cm_day: 86.4,
        csol: value + 11.0,
        k_solids: value + 12.0,
        tksatu: value + 13.0,
        tksatf: value + 14.0,
        tkdry: value + 15.0,
        ba_alpha: value + 16.0,
        ba_beta: value + 17.0,
    }
}

fn soil_state() -> SoilState {
    let source = (0..8)
        .flat_map(|layer| {
            [
                soil_layer(10.0 * (layer + 1) as f64),
                soil_layer(100.0 + 10.0 * (layer + 1) as f64),
            ]
        })
        .collect::<Vec<_>>();
    derive_soil_parameters(&source, &[1, 1], SOIL_LAYERS, HydraulicModel::VanGenuchten)
        .expect("the synthetic soil source is well formed")
}

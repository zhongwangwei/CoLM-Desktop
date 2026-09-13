//! Single-point `srfdata.nc` adapter used by the Rust `mkinidata` path.
//!
//! This is intentionally a strict reader: a missing static field is a scientific input
//! error, not a cue to substitute a plausible value.  The rawdata fallback remains the
//! responsibility of the Rust `mksrfdata` stage that creates `srfdata.nc`.

use std::path::Path;

use anyhow::{ensure, Context, Result};

use crate::{HydraulicModel, LandCoverScheme, SoilLayerInput, SoilReflectance};

const SOURCE_SOIL_LAYERS: usize = 8;

/// All single-point landdata needed by the static Rust initialization kernels.
#[derive(Debug, Clone, PartialEq)]
pub struct SinglePointSurfaceData {
    pub latitude_degrees: f64,
    pub longitude_degrees: f64,
    pub land_class: i32,
    pub canopy_height_m: f64,
    pub lake_depth_m: f64,
    pub albedo: SoilReflectance,
    pub soil_texture: i32,
    pub elevation_m: f64,
    pub elevation_std_m: f64,
    pub slope_ratio: f64,
    /// The first eight CoLM soil layers, from top to bottom.
    pub soil_layers: Vec<SoilLayerInput>,
}

/// Site monthly vegetation records stored alongside single-point surface data.
///
/// Values retain the NetCDF `(LAI_year, month)` order.  This is the source order
/// exposed by `MOD_SingleSrfdata.F90`, not a Rust-specific transpose.
#[derive(Debug, Clone, PartialEq)]
pub struct SinglePointMonthlyVegetation {
    pub years: Vec<i32>,
    pub lai: Vec<f64>,
    pub sai: Vec<f64>,
}

/// The positive PFT tiles and monthly state used by a non-CROP single-point run.
///
/// `MOD_SingleSrfdata` packs only positive `SITE_pctpfts` entries into `landpft`.
/// Keeping that compact order here lets the PFT restart vectors use the same order.
#[derive(Debug, Clone, PartialEq)]
pub struct SinglePointPftData {
    pub class: Vec<i32>,
    pub fraction: Vec<f64>,
    pub canopy_height_m: Vec<f64>,
    pub monthly: SinglePointPftMonthlyVegetation,
}

/// PFT-resolved monthly LAI and SAI in compact `landpft` order.
#[derive(Debug, Clone, PartialEq)]
pub struct SinglePointPftMonthlyVegetation {
    pub years: Vec<i32>,
    pub lai: Vec<f64>,
    pub sai: Vec<f64>,
    pfts: usize,
}

/// Complete single-point urban contract emitted by `mksrfdata`.
///
/// The common soil/topography fields retain the normal single-point layout;
/// urban geometry and materials are separate because `srfdata.nc` deliberately
/// has `URBAN_TYPE` instead of an IGBP/USGS classification variable.
#[derive(Debug, Clone, PartialEq)]
pub struct SinglePointUrbanData {
    pub common: SinglePointSurfaceData,
    pub urban_type: i32,
    pub lucy_region_id: i32,
    pub tree_percent: f64,
    pub tree_top_m: f64,
    pub water_percent: f64,
    pub roof_fraction: f64,
    pub roof_height_m: f64,
    pub pervious_road_fraction: f64,
    pub building_height_to_width: f64,
    pub population_density: f64,
    pub roof_emissivity: f64,
    pub wall_emissivity: f64,
    pub impervious_emissivity: f64,
    pub pervious_emissivity: f64,
    pub room_max_k: f64,
    pub room_min_k: f64,
    pub roof_thickness_m: f64,
    pub wall_thickness_m: f64,
    /// `[band][direct, diffuse]` in CoLM order.
    pub roof_albedo: Vec<f64>,
    pub wall_albedo: Vec<f64>,
    pub impervious_albedo: Vec<f64>,
    pub pervious_albedo: Vec<f64>,
    /// The ten `ulev` values, top to bottom.
    pub roof_heat_capacity: Vec<f64>,
    pub wall_heat_capacity: Vec<f64>,
    pub impervious_heat_capacity: Vec<f64>,
    pub roof_thermal_conductivity: Vec<f64>,
    pub wall_thermal_conductivity: Vec<f64>,
    pub impervious_thermal_conductivity: Vec<f64>,
    pub monthly: SinglePointMonthlyVegetation,
}

/// Region-major LUCY tables, normalized from the runtime NetCDF layout.
#[derive(Debug, Clone, PartialEq)]
pub struct UrbanLucyRawData {
    pub region_count: usize,
    pub vehicles_per_thousand: Vec<f64>,
    pub week_holiday: Vec<f64>,
    pub weekend_traffic_profile: Vec<f64>,
    pub weekday_traffic_profile: Vec<f64>,
    pub human_metabolic_profile: Vec<f64>,
    pub fixed_holiday: Vec<f64>,
}

impl SinglePointMonthlyVegetation {
    /// Resolves the selected one-based calendar month exactly as `LAI_readin` does.
    pub fn for_year(
        &self,
        target_year: i32,
        month: u8,
        use_site_lai: bool,
        configured_start_year: i32,
        configured_end_year: i32,
    ) -> Result<(f64, f64)> {
        ensure!((1..=12).contains(&month), "month must be in 1..=12");
        ensure!(
            configured_start_year <= configured_end_year,
            "LAI configured start year exceeds end year"
        );
        let selected_year = if use_site_lai {
            *self
                .years
                .iter()
                .min_by_key(|&&year| (i64::from(year) - i64::from(target_year)).abs())
                .context("single-point surface has no LAI years")?
        } else {
            target_year.clamp(configured_start_year, configured_end_year)
        };
        let year_index = self
            .years
            .iter()
            .position(|&year| year == selected_year)
            .with_context(|| {
                format!("single-point surface has no LAI record for {selected_year}")
            })?;
        let index = year_index * 12 + usize::from(month - 1);
        Ok((self.lai[index], self.sai[index]))
    }
}

impl SinglePointPftMonthlyVegetation {
    /// Resolves a calendar month with the same site-year selection as `LAI_readin`.
    pub fn for_year(
        &self,
        target_year: i32,
        month: u8,
        use_site_lai: bool,
        configured_start_year: i32,
        configured_end_year: i32,
    ) -> Result<(Vec<f64>, Vec<f64>)> {
        ensure!((1..=12).contains(&month), "month must be in 1..=12");
        ensure!(
            configured_start_year <= configured_end_year,
            "LAI configured start year exceeds end year"
        );
        let selected_year = if use_site_lai {
            *self
                .years
                .iter()
                .min_by_key(|&&year| (i64::from(year) - i64::from(target_year)).abs())
                .context("single-point surface has no LAI years")?
        } else {
            target_year.clamp(configured_start_year, configured_end_year)
        };
        let year_index = self
            .years
            .iter()
            .position(|&year| year == selected_year)
            .with_context(|| {
                format!("single-point surface has no LAI record for {selected_year}")
            })?;
        let start = (year_index * 12 + usize::from(month - 1)) * self.pfts;
        let end = start + self.pfts;
        Ok((self.lai[start..end].to_vec(), self.sai[start..end].to_vec()))
    }
}

/// Reads the `LAI_year`, `LAI_monthly`, and `SAI_monthly` single-point contract.
pub fn read_single_point_monthly_vegetation(
    path: impl AsRef<Path>,
) -> Result<SinglePointMonthlyVegetation> {
    let path = path.as_ref();
    let file = netcdf::open(path)
        .with_context(|| format!("cannot open single-point surface data {}", path.display()))?;
    let years = vector_i32(&file, "LAI_year")?;
    validate_lai_years(&years)?;
    let lai = monthly_vector(&file, "LAI_monthly", years.len())?;
    let sai = monthly_vector(&file, "SAI_monthly", years.len())?;
    Ok(SinglePointMonthlyVegetation { years, lai, sai })
}

/// Reads the PFT composition and monthly PFT LAI/SAI single-point contract.
///
/// This is the non-CROP `SITE_pfttyp`/`SITE_pctpfts` path.  CROP owns a
/// different `croptyp`/`pctcrop` contract and remains a separate restart family.
pub fn read_single_point_pft_data(path: impl AsRef<Path>) -> Result<SinglePointPftData> {
    let path = path.as_ref();
    let file = netcdf::open(path)
        .with_context(|| format!("cannot open single-point surface data {}", path.display()))?;
    let years = vector_i32(&file, "LAI_year")?;
    validate_lai_years(&years)?;
    let class = vector_i32(&file, "pfttyp")?;
    let fraction = vector(&file, "pctpfts")?;
    let canopy_height_m = vector(&file, "canopy_height_pfts")?;
    ensure!(
        !class.is_empty() && class.len() == fraction.len() && class.len() == canopy_height_m.len(),
        "pfttyp, pctpfts, and canopy_height_pfts must be nonempty equal-length vectors"
    );
    ensure!(
        class.iter().all(|class| (0..=15).contains(class))
            && fraction
                .iter()
                .all(|fraction| fraction.is_finite() && *fraction >= 0.0)
            && canopy_height_m
                .iter()
                .all(|height| height.is_finite() && *height >= 0.0),
        "single-point PFT classes, fractions, or canopy heights are invalid"
    );
    let indices = fraction
        .iter()
        .enumerate()
        .filter_map(|(index, fraction)| (*fraction > 0.0).then_some(index))
        .collect::<Vec<_>>();
    ensure!(
        !indices.is_empty(),
        "single-point surface has no positive PFT fractions"
    );
    let fraction = indices
        .iter()
        .map(|&index| fraction[index])
        .collect::<Vec<_>>();
    let total: f64 = fraction.iter().sum();
    ensure!(
        (total - 1.0).abs() <= 1.0e-6,
        "positive single-point PFT fractions must sum to one, got {total}"
    );
    // `MOD_SingleSrfdata` normalizes the packed positive SITE components.
    let fraction = fraction
        .into_iter()
        .map(|fraction| fraction / total)
        .collect::<Vec<_>>();
    let raw_pfts = class.len();
    let lai = pft_monthly_vector(&file, "LAI_pfts_monthly", years.len(), raw_pfts, &indices)?;
    let sai = pft_monthly_vector(&file, "SAI_pfts_monthly", years.len(), raw_pfts, &indices)?;
    Ok(SinglePointPftData {
        class: indices.iter().map(|&index| class[index]).collect(),
        fraction,
        canopy_height_m: indices
            .iter()
            .map(|&index| canopy_height_m[index])
            .collect(),
        monthly: SinglePointPftMonthlyVegetation {
            years,
            lai,
            sai,
            pfts: indices.len(),
        },
    })
}

/// Reads the static single-point contract produced by `MOD_SingleSrfdata.F90`.
pub fn read_single_point_surface(
    path: impl AsRef<Path>,
    land_cover: LandCoverScheme,
    hydraulic_model: HydraulicModel,
) -> Result<SinglePointSurfaceData> {
    let path = path.as_ref();
    let file = netcdf::open(path)
        .with_context(|| format!("cannot open single-point surface data {}", path.display()))?;
    let land_name = match land_cover {
        LandCoverScheme::Igbp => "IGBP_classification",
        LandCoverScheme::Usgs => "USGS_classification",
    };
    let land_class = scalar_i32(&file, land_name)?;
    single_point_surface_from_file(&file, land_class, hydraulic_model, None)
}

/// Reads the non-classification fields of a single-point urban surface.
pub fn read_single_point_urban_data(
    path: impl AsRef<Path>,
    land_cover: LandCoverScheme,
    hydraulic_model: HydraulicModel,
) -> Result<SinglePointUrbanData> {
    let path = path.as_ref();
    let file = netcdf::open(path)
        .with_context(|| format!("cannot open single-point surface data {}", path.display()))?;
    let urban_land_class = match land_cover {
        LandCoverScheme::Igbp => 13,
        LandCoverScheme::Usgs => 1,
    };
    let years = vector_i32(&file, "LAI_year")?;
    validate_lai_years(&years)?;
    let monthly = SinglePointMonthlyVegetation {
        lai: monthly_vector(&file, "TREE_LAI", years.len())?,
        sai: monthly_vector(&file, "TREE_SAI", years.len())?,
        years,
    };
    let lucy_raw = scalar(&file, "LUCY_id")?;
    ensure!(
        lucy_raw.fract() == 0.0 && lucy_raw >= 0.0 && lucy_raw <= i32::MAX as f64,
        "LUCY_id must be a nonnegative integer"
    );
    let data = SinglePointUrbanData {
        common: single_point_surface_from_file(
            &file,
            urban_land_class,
            hydraulic_model,
            Some(0.0),
        )?,
        urban_type: scalar_i32(&file, "URBAN_TYPE")?,
        lucy_region_id: lucy_raw as i32,
        tree_percent: scalar(&file, "PCT_Tree")?,
        tree_top_m: scalar(&file, "URBAN_TREE_TOP")?,
        water_percent: scalar(&file, "PCT_Water")?,
        roof_fraction: scalar(&file, "WT_ROOF")?,
        roof_height_m: scalar(&file, "HT_ROOF")?,
        pervious_road_fraction: scalar(&file, "WTROAD_PERV")?,
        building_height_to_width: scalar(&file, "BUILDING_HLR")?,
        population_density: scalar(&file, "POP_DEN")?,
        roof_emissivity: scalar(&file, "EM_ROOF")?,
        wall_emissivity: scalar(&file, "EM_WALL")?,
        impervious_emissivity: scalar(&file, "EM_IMPROAD")?,
        pervious_emissivity: scalar(&file, "EM_PERROAD")?,
        room_max_k: scalar(&file, "T_BUILDING_MAX")?,
        room_min_k: scalar(&file, "T_BUILDING_MIN")?,
        roof_thickness_m: scalar(&file, "THICK_ROOF")?,
        wall_thickness_m: scalar(&file, "THICK_WALL")?,
        roof_albedo: vector_with_len(&file, "ALB_ROOF", 4)?,
        wall_albedo: vector_with_len(&file, "ALB_WALL", 4)?,
        impervious_albedo: vector_with_len(&file, "ALB_IMPROAD", 4)?,
        pervious_albedo: vector_with_len(&file, "ALB_PERROAD", 4)?,
        roof_heat_capacity: vector_with_len(&file, "CV_ROOF", 10)?,
        wall_heat_capacity: vector_with_len(&file, "CV_WALL", 10)?,
        impervious_heat_capacity: vector_with_len(&file, "CV_IMPROAD", 10)?,
        roof_thermal_conductivity: vector_with_len(&file, "TK_ROOF", 10)?,
        wall_thermal_conductivity: vector_with_len(&file, "TK_WALL", 10)?,
        impervious_thermal_conductivity: vector_with_len(&file, "TK_IMPROAD", 10)?,
        monthly,
    };
    validate_urban_data(&data)?;
    Ok(data)
}

/// Reads `runtime/urban/LUCY_rawdata.nc` into the region-major core contract.
pub fn read_urban_lucy_raw_data(path: impl AsRef<Path>) -> Result<UrbanLucyRawData> {
    let path = path.as_ref();
    let file = netcdf::open(path)
        .with_context(|| format!("cannot open LUCY runtime data {}", path.display()))?;
    let vehicles = lucy_f64(&file, "NUMS_VEHC", 3)?;
    let week_holiday = lucy_i32(&file, "WEEKEND_DAY", 7)?;
    let weekend = lucy_f64(&file, "TraffProf_24hr_holiday", 24)?;
    let weekday = lucy_f64(&file, "TraffProf_24hr_work", 24)?;
    let metabolic = lucy_f64(&file, "HumMetabolic_24hr", 24)?;
    let holiday = lucy_f64(&file, "FIXED_HOLIDAY", 365)?;
    let region_count = vehicles.0;
    for (name, regions) in [
        ("WEEKEND_DAY", week_holiday.0),
        ("TraffProf_24hr_holiday", weekend.0),
        ("TraffProf_24hr_work", weekday.0),
        ("HumMetabolic_24hr", metabolic.0),
        ("FIXED_HOLIDAY", holiday.0),
    ] {
        ensure!(
            regions == region_count,
            "{name} has {regions} regions; expected {region_count}"
        );
    }
    Ok(UrbanLucyRawData {
        region_count,
        vehicles_per_thousand: vehicles.1,
        week_holiday: week_holiday.1,
        weekend_traffic_profile: weekend.1,
        weekday_traffic_profile: weekday.1,
        human_metabolic_profile: metabolic.1,
        fixed_holiday: holiday.1,
    })
}

fn single_point_surface_from_file(
    file: &netcdf::File,
    land_class: i32,
    hydraulic_model: HydraulicModel,
    canopy_height_fallback_m: Option<f64>,
) -> Result<SinglePointSurfaceData> {
    let source = SoilSourceFields {
        vf_quartz: vector(file, "soil_vf_quartz_mineral")?,
        vf_gravels: vector(file, "soil_vf_gravels")?,
        vf_om: vector(file, "soil_vf_om")?,
        vf_sand: vector(file, "soil_vf_sand")?,
        vf_clay: vector(file, "soil_vf_clay")?,
        wf_gravels: vector(file, "soil_wf_gravels")?,
        wf_sand: vector(file, "soil_wf_sand")?,
        wf_clay: vector(file, "soil_wf_clay")?,
        wf_om: vector(file, "soil_wf_om")?,
        om_density: vector(file, "soil_OM_density")?,
        bulk_density: vector(file, "soil_BD_all")?,
        theta_s: vector(file, "soil_theta_s")?,
        psi_s_cm: vector(file, "soil_psi_s")?,
        lambda: vector(file, "soil_lambda")?,
        theta_r: optional_vgm(file, "soil_theta_r", hydraulic_model)?,
        alpha_vgm: optional_vgm(file, "soil_alpha_vgm", hydraulic_model)?,
        l_vgm: optional_vgm(file, "soil_L_vgm", hydraulic_model)?,
        n_vgm: optional_vgm(file, "soil_n_vgm", hydraulic_model)?,
        k_s_cm_day: vector(file, "soil_k_s")?,
        csol: vector(file, "soil_csol")?,
        k_solids: vector(file, "soil_k_solids")?,
        tksatu: vector(file, "soil_tksatu")?,
        tksatf: vector(file, "soil_tksatf")?,
        tkdry: vector(file, "soil_tkdry")?,
        ba_alpha: vector(file, "soil_BA_alpha")?,
        ba_beta: vector(file, "soil_BA_beta")?,
    };
    source.validate()?;

    Ok(SinglePointSurfaceData {
        latitude_degrees: scalar(file, "latitude")?,
        longitude_degrees: scalar(file, "longitude")?,
        land_class,
        canopy_height_m: match canopy_height_fallback_m {
            Some(value) => value,
            None => scalar(file, "canopy_height")?,
        },
        lake_depth_m: scalar(file, "lakedepth")?,
        albedo: SoilReflectance {
            saturated_visible: scalar(file, "soil_s_v_alb")?,
            dry_visible: scalar(file, "soil_d_v_alb")?,
            saturated_near_infrared: scalar(file, "soil_s_n_alb")?,
            dry_near_infrared: scalar(file, "soil_d_n_alb")?,
        },
        soil_texture: scalar_i32(file, "soil_texture")?,
        elevation_m: scalar(file, "elevation")?,
        elevation_std_m: scalar(file, "elvstd")?,
        slope_ratio: scalar(file, "sloperatio")?,
        soil_layers: source.into_layers(),
    })
}

fn validate_urban_data(data: &SinglePointUrbanData) -> Result<()> {
    ensure!(
        data.urban_type > 0,
        "URBAN_TYPE must be a positive urban classification"
    );
    let scalar_values = [
        data.tree_percent,
        data.tree_top_m,
        data.water_percent,
        data.roof_fraction,
        data.roof_height_m,
        data.pervious_road_fraction,
        data.building_height_to_width,
        data.population_density,
        data.roof_emissivity,
        data.wall_emissivity,
        data.impervious_emissivity,
        data.pervious_emissivity,
        data.room_max_k,
        data.room_min_k,
        data.roof_thickness_m,
        data.wall_thickness_m,
    ];
    ensure!(
        scalar_values.into_iter().all(f64::is_finite),
        "urban scalar fields must be finite"
    );
    ensure!(
        (0.0..=100.0).contains(&data.tree_percent)
            && (0.0..=100.0).contains(&data.water_percent)
            && (0.0..=1.0).contains(&data.roof_fraction)
            && (0.0..=1.0).contains(&data.pervious_road_fraction)
            && data.tree_top_m >= 0.0
            && data.roof_height_m > 0.0
            && data.building_height_to_width > 0.0
            && data.roof_thickness_m > 0.0
            && data.wall_thickness_m > 0.0
            && data.room_max_k >= data.room_min_k,
        "urban geometry or thermal bounds are invalid"
    );
    for (name, values) in [
        ("ALB_ROOF", &data.roof_albedo),
        ("ALB_WALL", &data.wall_albedo),
        ("ALB_IMPROAD", &data.impervious_albedo),
        ("ALB_PERROAD", &data.pervious_albedo),
    ] {
        ensure!(
            values.iter().all(|value| (0.0..1.0).contains(value)),
            "{name} must contain albedos in [0, 1)"
        );
    }
    for (name, values) in [
        ("CV_ROOF", &data.roof_heat_capacity),
        ("CV_WALL", &data.wall_heat_capacity),
        ("CV_IMPROAD", &data.impervious_heat_capacity),
        ("TK_ROOF", &data.roof_thermal_conductivity),
        ("TK_WALL", &data.wall_thermal_conductivity),
        ("TK_IMPROAD", &data.impervious_thermal_conductivity),
    ] {
        ensure!(
            values
                .iter()
                .all(|value| value.is_finite() && *value >= 0.0),
            "{name} must contain finite nonnegative thermal values"
        );
    }
    Ok(())
}

fn lucy_f64(file: &netcdf::File, name: &str, components: usize) -> Result<(usize, Vec<f64>)> {
    let variable = file
        .variable(name)
        .with_context(|| format!("LUCY runtime data is missing {name}"))?;
    let dimensions = variable.dimensions();
    ensure!(
        dimensions.len() == 2 && dimensions[0].len() == components && dimensions[1].len() > 0,
        "{name} must have ({components}, region) dimensions"
    );
    let source = variable
        .get_values::<f64, _>(..)
        .with_context(|| format!("cannot read LUCY {name}"))?;
    transpose_lucy(name, source, components, dimensions[1].len())
}

fn lucy_i32(file: &netcdf::File, name: &str, components: usize) -> Result<(usize, Vec<f64>)> {
    let variable = file
        .variable(name)
        .with_context(|| format!("LUCY runtime data is missing {name}"))?;
    let dimensions = variable.dimensions();
    ensure!(
        dimensions.len() == 2 && dimensions[0].len() == components && dimensions[1].len() > 0,
        "{name} must have ({components}, region) dimensions"
    );
    let source = variable
        .get_values::<i32, _>(..)
        .with_context(|| format!("cannot read LUCY {name}"))?
        .into_iter()
        .map(f64::from)
        .collect();
    transpose_lucy(name, source, components, dimensions[1].len())
}

fn transpose_lucy(
    name: &str,
    source: Vec<f64>,
    components: usize,
    regions: usize,
) -> Result<(usize, Vec<f64>)> {
    ensure!(
        source.len() == components * regions && source.iter().all(|value| value.is_finite()),
        "LUCY {name} must contain finite values for every component and region"
    );
    let mut result = vec![0.0; source.len()];
    for region in 0..regions {
        for component in 0..components {
            result[region * components + component] = source[component * regions + region];
        }
    }
    Ok((regions, result))
}

struct SoilSourceFields {
    vf_quartz: Vec<f64>,
    vf_gravels: Vec<f64>,
    vf_om: Vec<f64>,
    vf_sand: Vec<f64>,
    vf_clay: Vec<f64>,
    wf_gravels: Vec<f64>,
    wf_sand: Vec<f64>,
    wf_clay: Vec<f64>,
    wf_om: Vec<f64>,
    om_density: Vec<f64>,
    bulk_density: Vec<f64>,
    theta_s: Vec<f64>,
    psi_s_cm: Vec<f64>,
    lambda: Vec<f64>,
    theta_r: Vec<f64>,
    alpha_vgm: Vec<f64>,
    l_vgm: Vec<f64>,
    n_vgm: Vec<f64>,
    k_s_cm_day: Vec<f64>,
    csol: Vec<f64>,
    k_solids: Vec<f64>,
    tksatu: Vec<f64>,
    tksatf: Vec<f64>,
    tkdry: Vec<f64>,
    ba_alpha: Vec<f64>,
    ba_beta: Vec<f64>,
}

impl SoilSourceFields {
    fn validate(&self) -> Result<()> {
        for (name, values) in [
            ("soil_vf_quartz_mineral", &self.vf_quartz),
            ("soil_vf_gravels", &self.vf_gravels),
            ("soil_vf_om", &self.vf_om),
            ("soil_vf_sand", &self.vf_sand),
            ("soil_vf_clay", &self.vf_clay),
            ("soil_wf_gravels", &self.wf_gravels),
            ("soil_wf_sand", &self.wf_sand),
            ("soil_wf_clay", &self.wf_clay),
            ("soil_wf_om", &self.wf_om),
            ("soil_OM_density", &self.om_density),
            ("soil_BD_all", &self.bulk_density),
            ("soil_theta_s", &self.theta_s),
            ("soil_psi_s", &self.psi_s_cm),
            ("soil_lambda", &self.lambda),
            ("soil_theta_r", &self.theta_r),
            ("soil_alpha_vgm", &self.alpha_vgm),
            ("soil_L_vgm", &self.l_vgm),
            ("soil_n_vgm", &self.n_vgm),
            ("soil_k_s", &self.k_s_cm_day),
            ("soil_csol", &self.csol),
            ("soil_k_solids", &self.k_solids),
            ("soil_tksatu", &self.tksatu),
            ("soil_tksatf", &self.tksatf),
            ("soil_tkdry", &self.tkdry),
            ("soil_BA_alpha", &self.ba_alpha),
            ("soil_BA_beta", &self.ba_beta),
        ] {
            ensure!(
                values.len() >= SOURCE_SOIL_LAYERS,
                "{name} has {} values; expected at least {SOURCE_SOIL_LAYERS} source soil layers",
                values.len()
            );
        }
        Ok(())
    }

    fn into_layers(self) -> Vec<SoilLayerInput> {
        // PLUMBER site files carry ten physical layers while CoLM's land
        // initialization consumes its documented first eight layers.
        (0..SOURCE_SOIL_LAYERS)
            .map(|layer| SoilLayerInput {
                vf_quartz: self.vf_quartz[layer],
                vf_gravels: self.vf_gravels[layer],
                vf_om: self.vf_om[layer],
                vf_sand: self.vf_sand[layer],
                vf_clay: self.vf_clay[layer],
                wf_gravels: self.wf_gravels[layer],
                wf_sand: self.wf_sand[layer],
                wf_clay: self.wf_clay[layer],
                wf_om: self.wf_om[layer],
                om_density: self.om_density[layer],
                bulk_density: self.bulk_density[layer],
                theta_s: self.theta_s[layer],
                psi_s_cm: self.psi_s_cm[layer],
                lambda: self.lambda[layer],
                theta_r: self.theta_r[layer],
                alpha_vgm: self.alpha_vgm[layer],
                l_vgm: self.l_vgm[layer],
                n_vgm: self.n_vgm[layer],
                k_s_cm_day: self.k_s_cm_day[layer],
                csol: self.csol[layer],
                k_solids: self.k_solids[layer],
                tksatu: self.tksatu[layer],
                tksatf: self.tksatf[layer],
                tkdry: self.tkdry[layer],
                ba_alpha: self.ba_alpha[layer],
                ba_beta: self.ba_beta[layer],
            })
            .collect()
    }
}

fn optional_vgm(
    file: &netcdf::File,
    name: &str,
    hydraulic_model: HydraulicModel,
) -> Result<Vec<f64>> {
    match hydraulic_model {
        HydraulicModel::VanGenuchten => vector(file, name),
        HydraulicModel::Campbell => Ok(vec![0.0; SOURCE_SOIL_LAYERS]),
    }
}

fn scalar(file: &netcdf::File, name: &str) -> Result<f64> {
    let values = vector(file, name)?;
    ensure!(
        values.len() == 1,
        "{name} must be scalar, got {} values",
        values.len()
    );
    Ok(values[0])
}

fn scalar_i32(file: &netcdf::File, name: &str) -> Result<i32> {
    let values = vector_i32(file, name)?;
    ensure!(
        values.len() == 1,
        "{name} must be scalar, got {} values",
        values.len()
    );
    Ok(values[0])
}

fn vector_i32(file: &netcdf::File, name: &str) -> Result<Vec<i32>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("single-point surface data is missing {name}"))?;
    variable
        .get_values::<i32, _>(..)
        .with_context(|| format!("cannot read {name}"))
}

fn vector(file: &netcdf::File, name: &str) -> Result<Vec<f64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("single-point surface data is missing {name}"))?;
    variable
        .get_values::<f64, _>(..)
        .with_context(|| format!("cannot read {name}"))
}

fn vector_with_len(file: &netcdf::File, name: &str, expected: usize) -> Result<Vec<f64>> {
    let values = vector(file, name)?;
    ensure!(
        values.len() == expected && values.iter().all(|value| value.is_finite()),
        "{name} must contain exactly {expected} finite values"
    );
    Ok(values)
}

fn monthly_vector(file: &netcdf::File, name: &str, years: usize) -> Result<Vec<f64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("single-point surface data is missing {name}"))?;
    let dimensions = variable.dimensions();
    ensure!(
        dimensions.len() == 2
            && dimensions[0].name() == "LAI_year"
            && dimensions[0].len() == years
            && dimensions[1].name() == "month"
            && dimensions[1].len() == 12,
        "{name} must have dimensions (LAI_year, month=12)"
    );
    let values = variable
        .get_values::<f64, _>(..)
        .with_context(|| format!("cannot read {name}"))?;
    ensure!(
        values.len() == years * 12 && values.iter().all(|value| value.is_finite()),
        "{name} must contain finite monthly values for every LAI year"
    );
    Ok(values)
}

fn validate_lai_years(years: &[i32]) -> Result<()> {
    ensure!(!years.is_empty(), "LAI_year must not be empty");
    for window in years.windows(2) {
        ensure!(
            window[0] < window[1],
            "LAI_year must be strictly increasing"
        );
    }
    Ok(())
}

fn pft_monthly_vector(
    file: &netcdf::File,
    name: &str,
    years: usize,
    raw_pfts: usize,
    indices: &[usize],
) -> Result<Vec<f64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("single-point surface data is missing {name}"))?;
    let dimensions = variable.dimensions();
    ensure!(
        dimensions.len() == 3
            && dimensions[0].name() == "LAI_year"
            && dimensions[0].len() == years
            && dimensions[1].name() == "month"
            && dimensions[1].len() == 12
            && dimensions[2].name() == "pft"
            && dimensions[2].len() == raw_pfts,
        "{name} must have dimensions (LAI_year, month=12, pft)"
    );
    let raw = variable
        .get_values::<f64, _>(..)
        .with_context(|| format!("cannot read {name}"))?;
    ensure!(
        raw.len() == years * 12 * raw_pfts && raw.iter().all(|value| value.is_finite()),
        "{name} must contain finite monthly values for every LAI year and PFT"
    );
    let mut values = Vec::with_capacity(years * 12 * indices.len());
    for record in 0..years * 12 {
        values.extend(indices.iter().map(|&index| raw[record * raw_pfts + index]));
    }
    Ok(values)
}

#[cfg(test)]
#[path = "surface_data_tests.rs"]
mod surface_data_tests;

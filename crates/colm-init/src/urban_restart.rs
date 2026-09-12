//! NetCDF constant-restart output for CoLM's urban model.
//!
//! The upstream urban restart is separate from the patch restart.  This adapter
//! consumes the flat state already produced by [`crate::derive_urban_geometry`]
//! and [`crate::derive_urban_lucy`] and preserves the vector NetCDF layout.

use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};

use crate::{UrbanLucyState, UrbanState};

const URBAN_LAYERS: usize = 10;
const NUM_SOLAR: usize = 2;
const NUM_RAD: usize = 2;

/// Urban material and radiative inputs not derived by the geometry/LUCY kernels.
#[derive(Debug, Clone, Copy)]
pub struct UrbanThermalFields<'a> {
    /// `numsolar * numrad * urban`, matching the Fortran array order.
    pub roof_albedo: &'a [f64],
    pub wall_albedo: &'a [f64],
    pub impervious_albedo: &'a [f64],
    pub pervious_albedo: &'a [f64],
    pub roof_emissivity: &'a [f64],
    pub wall_emissivity: &'a [f64],
    pub impervious_emissivity: &'a [f64],
    pub pervious_emissivity: &'a [f64],
    /// Every layer field is `ulev * urban`, matching Fortran `(layer, urban)`.
    pub roof_heat_capacity: &'a [f64],
    pub wall_heat_capacity: &'a [f64],
    pub impervious_heat_capacity: &'a [f64],
    pub roof_thermal_conductivity: &'a [f64],
    pub wall_thermal_conductivity: &'a [f64],
    pub impervious_thermal_conductivity: &'a [f64],
}

/// Values written by `WRITE_UrbanTimeInvariants`.
#[derive(Debug, Clone, Copy)]
pub struct UrbanConstantRestartInput<'a> {
    pub state: &'a UrbanState,
    pub lucy: &'a UrbanLucyState,
    pub thermal: UrbanThermalFields<'a>,
}

/// Writes `const/<case>_restart_urb_const_lcYYYY_<block>.nc`.
pub fn write_urban_constant_restart(
    restart_dir: impl AsRef<Path>,
    case_name: &str,
    land_cover_year: i32,
    block_label: &str,
    input: UrbanConstantRestartInput<'_>,
) -> Result<PathBuf> {
    validate_name(case_name, "case name")?;
    validate_name(block_label, "block label")?;
    ensure!(
        (0..=9999).contains(&land_cover_year),
        "land-cover year {land_cover_year} is outside the four-digit restart filename range"
    );
    let path = restart_dir.as_ref().join("const").join(format!(
        "{case_name}_restart_urb_const_lc{land_cover_year:04}_{block_label}.nc"
    ));
    write_urban_constant_restart_block(&path, input)?;
    Ok(path)
}

/// Writes one already-addressed urban constant-restart vector block.
pub fn write_urban_constant_restart_block(
    path: impl AsRef<Path>,
    input: UrbanConstantRestartInput<'_>,
) -> Result<()> {
    let urban = validate_input(input)?;
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let mut file = netcdf::create(path)
        .with_context(|| format!("cannot create urban constant restart {}", path.display()))?;
    for (name, length) in [
        ("urban", urban),
        ("numsolar", NUM_SOLAR),
        ("numrad", NUM_RAD),
        ("ulev", URBAN_LAYERS),
        ("ityp", 3),
        ("iweek", 7),
        ("ihour", 24),
        ("iday", 365),
    ] {
        file.add_dimension(name, length)?;
    }

    let state = input.state;
    for (name, values) in [
        ("PCT_Tree", state.tree_fraction.as_slice()),
        ("URBAN_TREE_TOP", state.tree_top_m.as_slice()),
        ("URBAN_TREE_BOT", state.tree_bottom_m.as_slice()),
        ("PCT_Water", state.water_fraction.as_slice()),
        ("POP_DEN", input.lucy.population_density.as_slice()),
        ("WT_ROOF", state.roof_fraction.as_slice()),
        ("HT_ROOF", state.roof_height_m.as_slice()),
        ("BUILDING_HLR", state.building_height_to_width.as_slice()),
        ("WTROAD_PERV", state.pervious_road_fraction.as_slice()),
        ("EM_ROOF", input.thermal.roof_emissivity),
        ("EM_WALL", input.thermal.wall_emissivity),
        ("EM_IMPROAD", input.thermal.impervious_emissivity),
        ("EM_PERROAD", input.thermal.pervious_emissivity),
        ("T_BUILDING_MIN", state.room_min_k.as_slice()),
        ("T_BUILDING_MAX", state.room_max_k.as_slice()),
    ] {
        put_urban_values(&mut file, name, values)?;
    }
    for (name, axis, values) in [
        (
            "VEHC_NUM",
            ("ityp", 3),
            input.lucy.vehicles_per_thousand.as_slice(),
        ),
        (
            "week_holiday",
            ("iweek", 7),
            input.lucy.week_holiday.as_slice(),
        ),
        (
            "weekendhour",
            ("ihour", 24),
            input.lucy.weekend_traffic_profile.as_slice(),
        ),
        (
            "weekdayhour",
            ("ihour", 24),
            input.lucy.weekday_traffic_profile.as_slice(),
        ),
        (
            "metabolism",
            ("ihour", 24),
            input.lucy.human_metabolic_profile.as_slice(),
        ),
        (
            "holiday",
            ("iday", 365),
            input.lucy.fixed_holiday.as_slice(),
        ),
        (
            "ROOF_DEPTH_L",
            ("ulev", URBAN_LAYERS),
            state.roof_node_depth_m.as_slice(),
        ),
        (
            "ROOF_THICK_L",
            ("ulev", URBAN_LAYERS),
            state.roof_layer_thickness_m.as_slice(),
        ),
        (
            "WALL_DEPTH_L",
            ("ulev", URBAN_LAYERS),
            state.wall_node_depth_m.as_slice(),
        ),
        (
            "WALL_THICK_L",
            ("ulev", URBAN_LAYERS),
            state.wall_layer_thickness_m.as_slice(),
        ),
        (
            "CV_ROOF",
            ("ulev", URBAN_LAYERS),
            input.thermal.roof_heat_capacity,
        ),
        (
            "CV_WALL",
            ("ulev", URBAN_LAYERS),
            input.thermal.wall_heat_capacity,
        ),
        (
            "TK_ROOF",
            ("ulev", URBAN_LAYERS),
            input.thermal.roof_thermal_conductivity,
        ),
        (
            "TK_WALL",
            ("ulev", URBAN_LAYERS),
            input.thermal.wall_thermal_conductivity,
        ),
        (
            "TK_IMPROAD",
            ("ulev", URBAN_LAYERS),
            input.thermal.impervious_thermal_conductivity,
        ),
        (
            "CV_IMPROAD",
            ("ulev", URBAN_LAYERS),
            input.thermal.impervious_heat_capacity,
        ),
    ] {
        put_axis_major(&mut file, name, axis, urban, values)?;
    }
    for (name, values) in [
        ("ALB_ROOF", input.thermal.roof_albedo),
        ("ALB_WALL", input.thermal.wall_albedo),
        ("ALB_IMPROAD", input.thermal.impervious_albedo),
        ("ALB_PERROAD", input.thermal.pervious_albedo),
    ] {
        put_urban_last_3d(&mut file, name, urban, values)?;
    }
    Ok(())
}

fn validate_input(input: UrbanConstantRestartInput<'_>) -> Result<usize> {
    let state = input.state;
    let urban = state.tree_fraction.len();
    ensure!(
        urban > 0,
        "an urban restart block needs at least one urban patch"
    );
    for (name, values) in [
        ("roof_fraction", state.roof_fraction.as_slice()),
        ("roof_height", state.roof_height_m.as_slice()),
        (
            "building_height_to_width",
            state.building_height_to_width.as_slice(),
        ),
        (
            "pervious_road_fraction",
            state.pervious_road_fraction.as_slice(),
        ),
        ("water_fraction", state.water_fraction.as_slice()),
        ("tree_top", state.tree_top_m.as_slice()),
        ("tree_bottom", state.tree_bottom_m.as_slice()),
        ("room_max", state.room_max_k.as_slice()),
        ("room_min", state.room_min_k.as_slice()),
        (
            "population_density",
            input.lucy.population_density.as_slice(),
        ),
        ("roof_emissivity", input.thermal.roof_emissivity),
        ("wall_emissivity", input.thermal.wall_emissivity),
        ("impervious_emissivity", input.thermal.impervious_emissivity),
        ("pervious_emissivity", input.thermal.pervious_emissivity),
    ] {
        ensure!(
            values.len() == urban,
            "urban {name} has {} entries; expected {urban}",
            values.len()
        );
    }
    for (name, axis, values) in [
        ("VEHC_NUM", 3, input.lucy.vehicles_per_thousand.as_slice()),
        ("week_holiday", 7, input.lucy.week_holiday.as_slice()),
        (
            "weekendhour",
            24,
            input.lucy.weekend_traffic_profile.as_slice(),
        ),
        (
            "weekdayhour",
            24,
            input.lucy.weekday_traffic_profile.as_slice(),
        ),
        (
            "metabolism",
            24,
            input.lucy.human_metabolic_profile.as_slice(),
        ),
        ("holiday", 365, input.lucy.fixed_holiday.as_slice()),
        (
            "ROOF_DEPTH_L",
            URBAN_LAYERS,
            state.roof_node_depth_m.as_slice(),
        ),
        (
            "ROOF_THICK_L",
            URBAN_LAYERS,
            state.roof_layer_thickness_m.as_slice(),
        ),
        (
            "WALL_DEPTH_L",
            URBAN_LAYERS,
            state.wall_node_depth_m.as_slice(),
        ),
        (
            "WALL_THICK_L",
            URBAN_LAYERS,
            state.wall_layer_thickness_m.as_slice(),
        ),
        ("CV_ROOF", URBAN_LAYERS, input.thermal.roof_heat_capacity),
        ("CV_WALL", URBAN_LAYERS, input.thermal.wall_heat_capacity),
        (
            "TK_ROOF",
            URBAN_LAYERS,
            input.thermal.roof_thermal_conductivity,
        ),
        (
            "TK_WALL",
            URBAN_LAYERS,
            input.thermal.wall_thermal_conductivity,
        ),
        (
            "TK_IMPROAD",
            URBAN_LAYERS,
            input.thermal.impervious_thermal_conductivity,
        ),
        (
            "CV_IMPROAD",
            URBAN_LAYERS,
            input.thermal.impervious_heat_capacity,
        ),
    ] {
        validate_axis_major(name, values, axis, urban)?;
    }
    for (name, values) in [
        ("ALB_ROOF", input.thermal.roof_albedo),
        ("ALB_WALL", input.thermal.wall_albedo),
        ("ALB_IMPROAD", input.thermal.impervious_albedo),
        ("ALB_PERROAD", input.thermal.pervious_albedo),
    ] {
        ensure!(
            values.len() == NUM_SOLAR * NUM_RAD * urban,
            "urban {name} has {} entries; expected {NUM_SOLAR} x {NUM_RAD} x {urban}",
            values.len()
        );
    }
    Ok(urban)
}

fn validate_name(value: &str, name: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')),
        "{name} must be a nonempty filename component"
    );
    Ok(())
}

fn validate_axis_major(name: &str, values: &[f64], axis: usize, urban: usize) -> Result<()> {
    ensure!(
        values.len() == axis * urban,
        "urban {name} has {} entries; expected {axis} x {urban}",
        values.len()
    );
    Ok(())
}

fn put_urban_values(file: &mut netcdf::FileMut, name: &str, values: &[f64]) -> Result<()> {
    file.add_variable::<f64>(name, &["urban"])?
        .put_values(values, ..)?;
    Ok(())
}

fn put_axis_major(
    file: &mut netcdf::FileMut,
    name: &str,
    (axis_name, axis): (&str, usize),
    urban: usize,
    values: &[f64],
) -> Result<()> {
    validate_axis_major(name, values, axis, urban)?;
    let mut on_disk = Vec::with_capacity(values.len());
    for patch in 0..urban {
        for index in 0..axis {
            on_disk.push(values[index * urban + patch]);
        }
    }
    file.add_variable::<f64>(name, &["urban", axis_name])?
        .put_values(&on_disk, (.., ..))?;
    Ok(())
}

fn put_urban_last_3d(
    file: &mut netcdf::FileMut,
    name: &str,
    urban: usize,
    values: &[f64],
) -> Result<()> {
    let mut on_disk = Vec::with_capacity(values.len());
    for patch in 0..urban {
        for rad in 0..NUM_RAD {
            for solar in 0..NUM_SOLAR {
                on_disk.push(values[(solar * NUM_RAD + rad) * urban + patch]);
            }
        }
    }
    file.add_variable::<f64>(name, &["urban", "numrad", "numsolar"])?
        .put_values(&on_disk, (.., .., ..))?;
    Ok(())
}

#[cfg(test)]
#[path = "urban_restart_tests.rs"]
mod urban_restart_tests;

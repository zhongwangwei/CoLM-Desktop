//! Native common static-restart entry point for a complete single-point surface.
//!
//! `mkinidata-rs case.nml` follows CoLM's case-directory convention.  The explicit
//! form is retained for a caller that needs a nonstandard surface or restart location.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use colm_init::{
    single_point_cold_start_run_from_namelist, write_single_point_cold_time_restarts,
    write_single_point_constant_restart, write_single_point_constant_restarts,
    write_spatial_lct_cold_time_restart, write_spatial_lct_constant_restart,
    write_spatial_pft_cold_time_restarts, write_spatial_pft_constant_restarts, HydraulicModel,
    LandCoverScheme, RestartDate, SinglePointStaticConfig, SpatialLctStaticConfig,
    SpatialLctTimeConfig, SpatialPftStaticConfig, SpatialPftTimeConfig,
};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let first = required(&mut args, "case namelist, surface, or spatial-lct")?;
    if first.as_os_str() == "spatial-lct" {
        return run_spatial_lct(args);
    }
    if first.as_os_str() == "spatial-pft" {
        return run_spatial_pft(args);
    }
    if first
        .extension()
        .is_some_and(|extension| extension == "nml")
    {
        return run_namelist(first, args);
    }
    run_explicit(first, args)
}

fn run_namelist(namelist: PathBuf, mut args: impl Iterator<Item = String>) -> Result<()> {
    let mut land_cover = None;
    let mut block = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--land-cover" => {
                land_cover = Some(parse_land_cover(
                    &args.next().context("--land-cover needs igbp or usgs")?,
                )?);
            }
            "--block" => block = Some(args.next().context("--block needs a CoLM block label")?),
            value => bail!("unknown mkinidata-rs option {value}"),
        }
    }
    let run = single_point_cold_start_run_from_namelist(&namelist, land_cover, block.as_deref())?;
    let files = write_single_point_constant_restarts(&run)?;
    let time = write_single_point_cold_time_restarts(&run)?;
    println!("wrote {}", files.common.constants.display());
    println!("wrote {}", files.common.block.display());
    if let Some(path) = files.pft {
        println!("wrote {}", path.display());
    }
    if let Some(files) = files.bgc {
        println!("wrote {}", files.constants.display());
        println!("wrote {}", files.block.display());
    }
    if let Some(path) = files.urban {
        println!("wrote {}", path.display());
    }
    println!("wrote {}", time.common.block.display());
    if let Some(path) = time.pft {
        println!("wrote {}", path.display());
    }
    if let Some(file) = time.bgc {
        println!("wrote {}", file.block.display());
    }
    if let Some(path) = time.urban {
        println!("wrote {}", path.display());
    }
    Ok(())
}

fn run_explicit(surface: PathBuf, mut args: impl Iterator<Item = String>) -> Result<()> {
    let restart = required(&mut args, "restart directory")?;
    let case_name = args.next().context(USAGE)?;
    let land_cover_year = args
        .next()
        .context("missing land-cover year")?
        .parse()
        .context("land-cover year must be an integer")?;
    let block = args.next().context("missing CoLM block label")?;
    let land_cover = parse_land_cover(&args.next().context("missing land-cover scheme")?)?;
    let hydraulic_model = parse_hydraulic_model(args.next().as_deref())?;
    if let Some(value) = args.next() {
        bail!("unexpected argument {value}");
    }

    let files = write_single_point_constant_restart(
        surface,
        restart,
        SinglePointStaticConfig::new(
            &case_name,
            land_cover_year,
            &block,
            land_cover,
            hydraulic_model,
        ),
    )?;
    println!("wrote {}", files.constants.display());
    println!("wrote {}", files.block.display());
    Ok(())
}

fn run_spatial_lct(mut args: impl Iterator<Item = String>) -> Result<()> {
    let landdata = required(&mut args, "landdata directory")?;
    let restart = required(&mut args, "restart directory")?;
    let case_name = args.next().context("missing case name")?;
    let land_cover_year = args
        .next()
        .context("missing land-cover year")?
        .parse()
        .context("land-cover year must be an integer")?;
    let block = args.next().context("missing CoLM block label")?;
    let land_cover = parse_land_cover(&args.next().context("missing land-cover scheme")?)?;
    let hydraulic_model = parse_hydraulic_model(args.next().as_deref())?;
    let mut config = SpatialLctStaticConfig::new(
        &landdata,
        &restart,
        &case_name,
        land_cover_year,
        &block,
        land_cover,
        hydraulic_model,
    );
    let mut cold_time = None;
    let mut lai_year = land_cover_year;
    let mut greenwich = false;
    let mut dynamic_lake = false;
    let mut plant_hydraulics = true;
    let mut ozone_stress = false;
    let mut variably_saturated_flow = false;
    let mut vegetation_snow = true;
    while let Some(value) = args.next() {
        match value.as_str() {
            "--bedrock" => config.use_bedrock = true,
            "--hyperspectral" => config.use_hyperspectral = true,
            "--cold-time" => {
                cold_time = Some(parse_restart_date(
                    &args.next().context("--cold-time needs YYYY-JJJ-SSSSS")?,
                )?)
            }
            "--lai-year" => {
                lai_year = args
                    .next()
                    .context("--lai-year needs a year")?
                    .parse()
                    .context("--lai-year must be an integer")?
            }
            "--greenwich" => greenwich = true,
            "--dynamic-lake" => dynamic_lake = true,
            "--no-plant-hydraulics" => plant_hydraulics = false,
            "--ozone-stress" => ozone_stress = true,
            "--variably-saturated-flow" => variably_saturated_flow = true,
            "--no-vegetation-snow" => vegetation_snow = false,
            _ => bail!("unexpected argument {value}"),
        }
    }
    let files = write_spatial_lct_constant_restart(config)?;
    println!("wrote {}", files.constants.display());
    println!("wrote {}", files.block.display());
    if let Some(date) = cold_time {
        let mut time = SpatialLctTimeConfig::new(
            &landdata,
            &restart,
            &case_name,
            land_cover_year,
            &block,
            land_cover,
            hydraulic_model,
            date,
        );
        time.lai_year = lai_year;
        time.greenwich = greenwich;
        time.dynamic_lake = dynamic_lake;
        time.plant_hydraulics = plant_hydraulics;
        time.ozone_stress = ozone_stress;
        time.variably_saturated_flow = variably_saturated_flow;
        time.vegetation_snow = vegetation_snow;
        let file = write_spatial_lct_cold_time_restart(time)?;
        println!("wrote {}", file.block.display());
    }
    Ok(())
}

fn run_spatial_pft(mut args: impl Iterator<Item = String>) -> Result<()> {
    let namelist = required(&mut args, "case namelist")?;
    let landdata = required(&mut args, "landdata directory")?;
    let restart = required(&mut args, "restart directory")?;
    let case_name = args.next().context("missing case name")?;
    let land_cover_year = args
        .next()
        .context("missing land-cover year")?
        .parse()
        .context("land-cover year must be an integer")?;
    let block = args.next().context("missing CoLM block label")?;
    let mut use_bedrock = false;
    let mut use_hyperspectral = false;
    let mut cold_time = None;
    let mut lai_year = land_cover_year;
    let mut greenwich = false;
    let mut dynamic_lake = false;
    let mut plant_hydraulics = true;
    let mut ozone_stress = false;
    let mut variably_saturated_flow = false;
    let mut vegetation_snow = true;
    while let Some(value) = args.next() {
        match value.as_str() {
            "--bedrock" => use_bedrock = true,
            "--hyperspectral" => use_hyperspectral = true,
            "--cold-time" => {
                cold_time = Some(parse_restart_date(
                    &args.next().context("--cold-time needs YYYY-JJJ-SSSSS")?,
                )?)
            }
            "--lai-year" => {
                lai_year = args
                    .next()
                    .context("--lai-year needs a year")?
                    .parse()
                    .context("--lai-year must be an integer")?
            }
            "--greenwich" => greenwich = true,
            "--dynamic-lake" => dynamic_lake = true,
            "--no-plant-hydraulics" => plant_hydraulics = false,
            "--ozone-stress" => ozone_stress = true,
            "--variably-saturated-flow" => variably_saturated_flow = true,
            "--no-vegetation-snow" => vegetation_snow = false,
            _ => bail!("unexpected argument {value}"),
        }
    }
    if cold_time.is_some() && use_hyperspectral {
        bail!("spatial PFT cold-time restart does not yet support --hyperspectral");
    }
    let static_config = SpatialPftStaticConfig::new(
        &namelist,
        &landdata,
        &restart,
        &case_name,
        land_cover_year,
        &block,
    );
    let files = write_spatial_pft_constant_restarts(static_config, use_bedrock, use_hyperspectral)?;
    println!("wrote {}", files.common.constants.display());
    println!("wrote {}", files.common.block.display());
    println!("wrote {}", files.pft.display());
    if let Some(bgc) = files.bgc {
        println!("wrote {}", bgc.constants.display());
        println!("wrote {}", bgc.block.display());
    }
    if let Some(date) = cold_time {
        let mut time = SpatialPftTimeConfig::new(static_config, date);
        time.lai_year = lai_year;
        time.greenwich = greenwich;
        time.dynamic_lake = dynamic_lake;
        time.plant_hydraulics = plant_hydraulics;
        time.ozone_stress = ozone_stress;
        time.variably_saturated_flow = variably_saturated_flow;
        time.vegetation_snow = vegetation_snow;
        let output = write_spatial_pft_cold_time_restarts(time)?;
        println!("wrote {}", output.common.block.display());
        println!("wrote {}", output.pft.display());
        if let Some(bgc) = output.bgc {
            println!("wrote {}", bgc.block.display());
        }
    }
    Ok(())
}

fn parse_hydraulic_model(value: Option<&str>) -> Result<HydraulicModel> {
    match value {
        Some("campbell") => Ok(HydraulicModel::Campbell),
        Some("vg") => Ok(HydraulicModel::VanGenuchten),
        Some(value) => bail!("hydraulic model must be campbell or vg, got {value}"),
        None => bail!("missing hydraulic model"),
    }
}

const USAGE: &str = "usage: mkinidata-rs <case.nml> [--land-cover igbp|usgs] [--block label]\n       mkinidata-rs <srfdata.nc> <restart-dir> <case> <lc-year> <block> <igbp|usgs> <campbell|vg>\n       mkinidata-rs spatial-lct <landdata-dir> <restart-dir> <case> <lc-year> <block> <igbp|usgs> <campbell|vg> [--bedrock] [--hyperspectral] [--cold-time YYYY-JJJ-SSSSS] [--lai-year YYYY] [--greenwich] [--dynamic-lake] [--no-plant-hydraulics] [--ozone-stress] [--variably-saturated-flow] [--no-vegetation-snow]\n       mkinidata-rs spatial-pft <case.nml> <landdata-dir> <restart-dir> <case> <lc-year> <block> [--bedrock] [--hyperspectral] [--cold-time YYYY-JJJ-SSSSS] [--lai-year YYYY] [--greenwich] [--dynamic-lake] [--no-plant-hydraulics] [--ozone-stress] [--variably-saturated-flow] [--no-vegetation-snow]";

fn parse_restart_date(value: &str) -> Result<RestartDate> {
    let mut fields = value.split('-');
    let year = fields
        .next()
        .context("cold restart date is missing year")?
        .parse()
        .context("cold restart year must be an integer")?;
    let julian_day = fields
        .next()
        .context("cold restart date is missing Julian day")?
        .parse()
        .context("cold restart Julian day must be an integer")?;
    let seconds = fields
        .next()
        .context("cold restart date is missing seconds")?
        .parse()
        .context("cold restart seconds must be an integer")?;
    if fields.next().is_some() {
        bail!("cold restart date must be YYYY-JJJ-SSSSS");
    }
    Ok(RestartDate {
        year,
        julian_day,
        seconds,
    })
}

fn parse_land_cover(value: &str) -> Result<LandCoverScheme> {
    match value {
        "igbp" => Ok(LandCoverScheme::Igbp),
        "usgs" => Ok(LandCoverScheme::Usgs),
        value => bail!("land-cover scheme must be igbp or usgs, got {value}"),
    }
}

fn required(args: &mut impl Iterator<Item = String>, field: &str) -> Result<PathBuf> {
    args.next()
        .map(PathBuf::from)
        .with_context(|| format!("missing {field}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cold_time_parser_preserves_the_colm_restart_label() {
        assert_eq!(
            parse_restart_date("2005-001-00000").unwrap(),
            RestartDate {
                year: 2005,
                julian_day: 1,
                seconds: 0,
            }
        );
        assert!(parse_restart_date("2005-001").is_err());
        assert!(parse_restart_date("2005-001-00000-extra").is_err());
    }
}

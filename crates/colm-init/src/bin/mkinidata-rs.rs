//! Native common static-restart entry point for a complete single-point surface.
//!
//! `mkinidata-rs case.nml` follows CoLM's case-directory convention.  The explicit
//! form is retained for a caller that needs a nonstandard surface or restart location.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use colm_init::{
    single_point_cold_start_run_from_namelist, write_single_point_cold_time_restarts,
    write_single_point_constant_restart, write_single_point_constant_restarts,
    write_spatial_lct_constant_restart, write_spatial_pft_constant_restarts, HydraulicModel,
    LandCoverScheme, SinglePointStaticConfig, SpatialLctStaticConfig, SpatialPftStaticConfig,
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
    for value in args {
        match value.as_str() {
            "--bedrock" => config.use_bedrock = true,
            "--hyperspectral" => config.use_hyperspectral = true,
            _ => bail!("unexpected argument {value}"),
        }
    }
    let files = write_spatial_lct_constant_restart(config)?;
    println!("wrote {}", files.constants.display());
    println!("wrote {}", files.block.display());
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
    for value in args {
        match value.as_str() {
            "--bedrock" => use_bedrock = true,
            "--hyperspectral" => use_hyperspectral = true,
            _ => bail!("unexpected argument {value}"),
        }
    }
    let files = write_spatial_pft_constant_restarts(
        SpatialPftStaticConfig::new(
            &namelist,
            &landdata,
            &restart,
            &case_name,
            land_cover_year,
            &block,
        ),
        use_bedrock,
        use_hyperspectral,
    )?;
    println!("wrote {}", files.common.constants.display());
    println!("wrote {}", files.common.block.display());
    println!("wrote {}", files.pft.display());
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

const USAGE: &str = "usage: mkinidata-rs <case.nml> [--land-cover igbp|usgs] [--block label]\n       mkinidata-rs <srfdata.nc> <restart-dir> <case> <lc-year> <block> <igbp|usgs> <campbell|vg>
       mkinidata-rs spatial-lct <landdata-dir> <restart-dir> <case> <lc-year> <block> <igbp|usgs> <campbell|vg> [--bedrock] [--hyperspectral]\n       mkinidata-rs spatial-pft <case.nml> <landdata-dir> <restart-dir> <case> <lc-year> <block> [--bedrock] [--hyperspectral]";

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

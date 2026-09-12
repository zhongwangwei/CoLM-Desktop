//! Native common static-restart entry point for a complete single-point surface.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use colm_init::{
    write_single_point_constant_restart, HydraulicModel, LandCoverScheme, SinglePointStaticConfig,
};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let surface = required(&mut args, "surface")?;
    let restart = required(&mut args, "restart directory")?;
    let case_name = args
        .next()
        .context("usage: mkinidata-rs <srfdata.nc> <restart-dir> <case> <lc-year> <block> <igbp|usgs> <campbell|vg>")?;
    let land_cover_year = args
        .next()
        .context("missing land-cover year")?
        .parse()
        .context("land-cover year must be an integer")?;
    let block = args.next().context("missing CoLM block label")?;
    let land_cover = match args.next().as_deref() {
        Some("igbp") => LandCoverScheme::Igbp,
        Some("usgs") => LandCoverScheme::Usgs,
        Some(value) => bail!("land-cover scheme must be igbp or usgs, got {value}"),
        None => bail!("missing land-cover scheme"),
    };
    let hydraulic_model = match args.next().as_deref() {
        Some("campbell") => HydraulicModel::Campbell,
        Some("vg") => HydraulicModel::VanGenuchten,
        Some(value) => bail!("hydraulic model must be campbell or vg, got {value}"),
        None => bail!("missing hydraulic model"),
    };
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

fn required(args: &mut impl Iterator<Item = String>, field: &str) -> Result<PathBuf> {
    args.next()
        .map(PathBuf::from)
        .with_context(|| format!("missing {field}"))
}

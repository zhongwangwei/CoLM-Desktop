//! Native Rust `mksrfdata → mkinidata` entry point for one single-point case.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use colm_init::prepare_single_point_case;
use colm_srfdata::SiteMode;

const USAGE: &str = "usage: colm-preprocess-rs <case.nml> [--land-cover igbp|usgs] [--crop] [--observation observation.nc]";

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let namelist = PathBuf::from(args.next().context(USAGE)?);
    if namelist.as_os_str() == "--help" || namelist.as_os_str() == "-h" {
        println!("{USAGE}");
        return Ok(());
    }
    let mut land_cover = None;
    let mut crop = false;
    let mut observation = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--land-cover" => {
                land_cover = Some(
                    match args
                        .next()
                        .context("--land-cover needs igbp or usgs")?
                        .as_str()
                    {
                        "igbp" => SiteMode::Igbp,
                        "usgs" => SiteMode::Usgs,
                        value => bail!("--land-cover must be igbp or usgs, got {value}"),
                    },
                );
            }
            "--crop" => crop = true,
            "--observation" => {
                observation = Some(PathBuf::from(
                    args.next().context("--observation needs a NetCDF path")?,
                ));
            }
            value => bail!("unknown colm-preprocess-rs option {value}\n{USAGE}"),
        }
    }
    let (run, files) =
        prepare_single_point_case(namelist, land_cover, crop, observation.as_deref())?;
    println!("wrote {}", files.surface.display());
    println!("wrote {}", files.constants.common.constants.display());
    println!("wrote {}", files.constants.common.block.display());
    println!("wrote {}", files.time.common.block.display());
    println!("prepared {}", run.cold_start.static_run.case_name);
    Ok(())
}

//! Native single-point surface-data materializer.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use colm_srfdata::{
    materialize_single_point_surface, materialize_single_point_surface_from_namelist, SiteMode,
};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let first = args.first().context(usage())?;
    if first.ends_with(".nml") {
        return materialize_case(&args);
    }
    materialize_legacy(&args)
}

fn materialize_case(args: &[String]) -> Result<()> {
    let namelist = PathBuf::from(args.first().expect("nonempty args"));
    let mut lct_mode = None;
    let mut crop = false;
    let mut observation = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--land-cover" => {
                let value = args
                    .get(index + 1)
                    .context("--land-cover needs igbp or usgs")?;
                lct_mode = Some(match value.as_str() {
                    "igbp" => SiteMode::Igbp,
                    "usgs" => SiteMode::Usgs,
                    _ => bail!("--land-cover must be igbp or usgs"),
                });
                index += 2;
            }
            "--crop" => {
                crop = true;
                index += 1;
            }
            "--observation" => {
                observation = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--observation needs a NetCDF path")?,
                ));
                index += 2;
            }
            other => bail!("unknown mksrfdata-rs case option {other:?}\n{}", usage()),
        }
    }
    let (run, report) = materialize_single_point_surface_from_namelist(
        &namelist,
        lct_mode,
        crop,
        observation.as_deref(),
    )?;
    print_result(report, &run.landdata_dir);
    Ok(())
}

fn materialize_legacy(args: &[String]) -> Result<()> {
    if !(2..=4).contains(&args.len()) {
        bail!("{}", usage());
    }
    let source = PathBuf::from(&args[0]);
    let landdata = PathBuf::from(&args[1]);
    let rawdata = args.get(2).map(PathBuf::from);
    let observation = args.get(3).map(PathBuf::from);
    let report = materialize_single_point_surface(
        &source,
        &landdata,
        SiteMode::Igbp,
        rawdata.as_deref(),
        observation.as_deref(),
        false,
    )?;
    print_result(report, &landdata);
    Ok(())
}

fn print_result(report: Option<colm_srfdata::site::Report>, landdata: &std::path::Path) {
    if let Some(report) = report {
        println!(
            "filled {} field(s), then wrote {}",
            report.from_site.len()
                + report.from_raster.len()
                + report.from_default.len()
                + report.from_lookup.len(),
            landdata.join("srfdata.nc").display()
        );
    } else {
        println!("wrote {}", landdata.join("srfdata.nc").display());
    }
}

fn usage() -> &'static str {
    "usage:\n  mksrfdata-rs <case.nml> [--land-cover igbp|usgs] [--crop] [--observation observation.nc]\n  mksrfdata-rs <site.nc> <landdata-dir> [rawdata] [observation.nc]"
}

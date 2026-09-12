//! Native single-point surface-data materializer.

use std::path::PathBuf;

use anyhow::{Context, Result};
use colm_srfdata::{materialize_single_point_surface, SiteMode};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let source = PathBuf::from(
        args.next()
            .context("usage: mksrfdata-rs <site.nc> <landdata-dir> [rawdata] [observation.nc]")?,
    );
    let landdata = PathBuf::from(
        args.next()
            .context("usage: mksrfdata-rs <site.nc> <landdata-dir> [rawdata] [observation.nc]")?,
    );
    let rawdata = args.next().map(PathBuf::from);
    let observation = args.next().map(PathBuf::from);
    let report = materialize_single_point_surface(
        &source,
        &landdata,
        SiteMode::Igbp,
        rawdata.as_deref(),
        observation.as_deref(),
        false,
    )?;
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
    Ok(())
}

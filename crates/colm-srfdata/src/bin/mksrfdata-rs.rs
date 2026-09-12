//! Native single-point surface-data materializer.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use colm_srfdata::{
    build_lct_land_patches_from_raster, build_spatial_topology, materialize_single_point_surface,
    materialize_single_point_surface_from_namelist, write_spatial_topology, BlockLayout, SiteMode,
    SpatialInputKind, COLM_500M,
};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let first = args.first().context(usage())?;
    if first == "spatial-lct" {
        return materialize_spatial_lct(&args[1..]);
    }
    if first.ends_with(".nml") {
        return materialize_case(&args);
    }
    materialize_legacy(&args)
}

struct SpatialLctArgs {
    kind: SpatialInputKind,
    mesh: PathBuf,
    landtype: PathBuf,
    landdata: PathBuf,
    year: i32,
    blocks: BlockLayout,
    dominant: bool,
}

fn materialize_spatial_lct(args: &[String]) -> Result<()> {
    let args = parse_spatial_lct(args)?;
    let topology = build_spatial_topology(&args.mesh, args.kind, COLM_500M)?;
    let (topology, patches) = build_lct_land_patches_from_raster(
        topology,
        &args.landtype,
        "landtype",
        COLM_500M,
        args.dominant,
    )?;
    write_spatial_topology(&args.landdata, args.year, &topology, &patches, &args.blocks)?;
    println!(
        "wrote {} spatial land elements and {} LCT patches to {}",
        topology.land_elements.element_ids.len(),
        patches.set_type.len(),
        args.landdata.display()
    );
    Ok(())
}

fn parse_spatial_lct(args: &[String]) -> Result<SpatialLctArgs> {
    if args.len() < 5 {
        bail!("{}", usage());
    }
    let kind = match args[0].as_str() {
        "latlon" => SpatialInputKind::GridBased,
        "unstructured" => SpatialInputKind::Unstructured,
        other => bail!("spatial-lct mesh kind must be latlon or unstructured, got {other:?}"),
    };
    let year = args[4]
        .parse::<i32>()
        .with_context(|| format!("invalid land-cover year {:?}", args[4]))?;
    let mut blocks = BlockLayout::regular(1, 1)?;
    let mut dominant = false;
    let mut index = 5;
    while index < args.len() {
        match args[index].as_str() {
            "--blocks" => {
                let nx = args
                    .get(index + 1)
                    .context("--blocks needs longitude and latitude block counts")?
                    .parse::<usize>()
                    .context("invalid longitude block count")?;
                let ny = args
                    .get(index + 2)
                    .context("--blocks needs longitude and latitude block counts")?
                    .parse::<usize>()
                    .context("invalid latitude block count")?;
                blocks = BlockLayout::regular(nx, ny)?;
                index += 3;
            }
            "--dominant" => {
                dominant = true;
                index += 1;
            }
            other => bail!("unknown spatial-lct option {other:?}\n{}", usage()),
        }
    }
    Ok(SpatialLctArgs {
        kind,
        mesh: PathBuf::from(&args[1]),
        landtype: PathBuf::from(&args[2]),
        landdata: PathBuf::from(&args[3]),
        year,
        blocks,
        dominant,
    })
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
    "usage:\n  mksrfdata-rs <case.nml> [--land-cover igbp|usgs] [--crop] [--observation observation.nc]\n  mksrfdata-rs <site.nc> <landdata-dir> [rawdata] [observation.nc]\n  mksrfdata-rs spatial-lct <latlon|unstructured> <mesh.nc> <landtype.nc> <landdata-dir> <lc-year> [--blocks nx ny] [--dominant]"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spatial_lct_parser_requires_explicit_mesh_contract() {
        let parsed = parse_spatial_lct(&[
            "unstructured".into(),
            "mesh.nc".into(),
            "landtype.nc".into(),
            "landdata".into(),
            "2005".into(),
            "--blocks".into(),
            "4".into(),
            "2".into(),
            "--dominant".into(),
        ])
        .unwrap();
        assert_eq!(parsed.kind, SpatialInputKind::Unstructured);
        assert!(parsed.dominant);
        assert_eq!(parsed.blocks.lon_w.len(), 4);
        assert_eq!(parsed.blocks.lat_s.len(), 2);
    }
}

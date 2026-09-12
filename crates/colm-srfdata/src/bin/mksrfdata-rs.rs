//! Native single-point surface-data materializer.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use colm_srfdata::{
    build_lct_land_patches_from_raster, build_spatial_topology, materialize_single_point_surface,
    materialize_single_point_surface_from_namelist, read_mesh_raster_f64, read_mesh_raster_i32,
    write_landpatch_scalar, write_spatial_topology, BlockLayout, SiteMode, SpatialInputKind,
    COLM_1KM, COLM_500M,
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
    land_cover: SiteMode,
    lake_depth: Option<PathBuf>,
    soil_texture: Option<PathBuf>,
    topography: Option<PathBuf>,
}

fn materialize_spatial_lct(args: &[String]) -> Result<()> {
    let args = parse_spatial_lct(args)?;
    let topology = build_spatial_topology(&args.mesh, args.kind, COLM_500M)?;
    let lct_grid = match args.land_cover {
        SiteMode::Igbp => COLM_500M,
        SiteMode::Usgs => COLM_1KM,
        SiteMode::Pft | SiteMode::Pc | SiteMode::Urban => {
            bail!("spatial-lct supports only IGBP or USGS land cover")
        }
    };
    let (topology, patches) = build_lct_land_patches_from_raster(
        topology,
        &args.landtype,
        "landtype",
        lct_grid,
        args.dominant,
    )?;
    let lake_depth = if let Some(path) = &args.lake_depth {
        let waterbody = match args.land_cover {
            SiteMode::Igbp => 17,
            SiteMode::Usgs => 16,
            SiteMode::Pft | SiteMode::Pc | SiteMode::Urban => {
                bail!("--lake-depth supports only LCT IGBP or USGS land cover")
            }
        };
        let layout = patches.aggregation_layout(&topology.mesh, vec![None; patches.len()])?;
        let raw = read_mesh_raster_f64(
            path,
            "lake_depth",
            &topology.mesh,
            &topology.pixel,
            COLM_500M,
        )?;
        Some(layout.aggregate_lake_depth(&raw, waterbody)?)
    } else {
        None
    };
    let soil_texture = if let Some(path) = &args.soil_texture {
        let layout = patches.aggregation_layout(&topology.mesh, vec![None; patches.len()])?;
        let raw = read_mesh_raster_i32(
            path,
            "soiltexture",
            &topology.mesh,
            &topology.pixel,
            COLM_500M,
        )?;
        Some(layout.aggregate_soil_texture(&raw)?)
    } else {
        None
    };
    let topography = if let Some(path) = &args.topography {
        let layout = patches.aggregation_layout(&topology.mesh, vec![None; patches.len()])?;
        Some(layout.aggregate_topography(
            &read_mesh_raster_f64(path, "landarea", &topology.mesh, &topology.pixel, COLM_500M)?,
            &read_mesh_raster_f64(
                path,
                "elevation",
                &topology.mesh,
                &topology.pixel,
                COLM_500M,
            )?,
            &read_mesh_raster_f64(path, "elvstd", &topology.mesh, &topology.pixel, COLM_500M)?,
            &read_mesh_raster_f64(path, "slope", &topology.mesh, &topology.pixel, COLM_500M)?,
        )?)
    } else {
        None
    };
    write_spatial_topology(&args.landdata, args.year, &topology, &patches, &args.blocks)?;
    if let Some(lake_depth) = lake_depth {
        write_landpatch_scalar(
            &args.landdata,
            args.year,
            &topology,
            &patches,
            &args.blocks,
            "lakedepth",
            "lakedepth_patches",
            &lake_depth,
        )?;
    }
    if let Some(soil_texture) = soil_texture {
        write_landpatch_scalar(
            &args.landdata,
            args.year,
            &topology,
            &patches,
            &args.blocks,
            "soil",
            "soiltext_patches",
            &soil_texture,
        )?;
    }
    if let Some(topography) = topography {
        for (variable, values) in [
            ("elevation_patches", &topography.elevation),
            ("elvstd_patches", &topography.elevation_std),
            ("sloperatio_patches", &topography.slope_ratio),
        ] {
            write_landpatch_scalar(
                &args.landdata,
                args.year,
                &topology,
                &patches,
                &args.blocks,
                "topography",
                variable,
                values,
            )?;
        }
    }
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
    let mut land_cover = None;
    let mut lake_depth = None;
    let mut soil_texture = None;
    let mut topography = None;
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
            "--land-cover" => {
                land_cover = Some(parse_land_cover(
                    args.get(index + 1)
                        .context("--land-cover needs igbp or usgs")?,
                )?);
                index += 2;
            }
            "--lake-depth" => {
                lake_depth = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--lake-depth needs a NetCDF path")?,
                ));
                index += 2;
            }
            "--soil-texture" => {
                soil_texture = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--soil-texture needs a NetCDF path")?,
                ));
                index += 2;
            }
            "--topography" => {
                topography = Some(PathBuf::from(
                    args.get(index + 1)
                        .context("--topography needs a NetCDF path")?,
                ));
                index += 2;
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
        land_cover: land_cover.context("spatial-lct requires --land-cover igbp or usgs")?,
        lake_depth,
        soil_texture,
        topography,
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
                lct_mode = Some(parse_land_cover(
                    args.get(index + 1)
                        .context("--land-cover needs igbp or usgs")?,
                )?);
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

fn parse_land_cover(value: &str) -> Result<SiteMode> {
    match value {
        "igbp" => Ok(SiteMode::Igbp),
        "usgs" => Ok(SiteMode::Usgs),
        _ => bail!("--land-cover must be igbp or usgs"),
    }
}

fn usage() -> &'static str {
    "usage:\n  mksrfdata-rs <case.nml> [--land-cover igbp|usgs] [--crop] [--observation observation.nc]\n  mksrfdata-rs <site.nc> <landdata-dir> [rawdata] [observation.nc]\n  mksrfdata-rs spatial-lct <latlon|unstructured> <mesh.nc> <landtype.nc> <landdata-dir> <lc-year> --land-cover <igbp|usgs> [--blocks nx ny] [--dominant] [--lake-depth lake_depth.nc] [--soil-texture soiltexture_0cm-60cm_mean.nc] [--topography topography.nc]"
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
            "--land-cover".into(),
            "igbp".into(),
            "--lake-depth".into(),
            "lake_depth.nc".into(),
            "--soil-texture".into(),
            "soiltexture.nc".into(),
            "--topography".into(),
            "topography.nc".into(),
        ])
        .unwrap();
        assert_eq!(parsed.kind, SpatialInputKind::Unstructured);
        assert!(parsed.dominant);
        assert_eq!(parsed.land_cover, SiteMode::Igbp);
        assert_eq!(parsed.lake_depth, Some(PathBuf::from("lake_depth.nc")));
        assert_eq!(parsed.soil_texture, Some(PathBuf::from("soiltexture.nc")));
        assert_eq!(parsed.topography, Some(PathBuf::from("topography.nc")));
        assert_eq!(parsed.blocks.lon_w.len(), 4);
        assert_eq!(parsed.blocks.lat_s.len(), 2);
        assert!(parse_spatial_lct(&[
            "latlon".into(),
            "mesh.nc".into(),
            "landtype.nc".into(),
            "landdata".into(),
            "2005".into(),
        ])
        .is_err());
    }
}

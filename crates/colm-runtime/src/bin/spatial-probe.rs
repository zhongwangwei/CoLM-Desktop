//! 临时诊断：打印空间算例前几步每个 patch 的强迫（按位），与 Fortran 探针对比。
use anyhow::{Context, Result};
use colm_runtime::spatial::{forcing, mapping, topology};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let case = std::path::PathBuf::from(&args[1]);
    let steps: usize = args[2].parse()?;
    let landdata = case.join("out").join(&args[3]).join("landdata");
    let forcing_nml = std::path::PathBuf::from(&args[4]);
    let topo = topology::SpatialTopology::read(&landdata, 2005)?;
    let text = std::fs::read_to_string(&forcing_nml)?;
    let doc = colm_namelist::parse(&text)?;
    let config = forcing::GriddedForcingConfig::from_document(&doc)?;
    let start = colm_core::CalendarTime {
        year: 2010,
        julian_day: 1,
        seconds: 0,
    };
    let grid = forcing::GriddedForcing::open_grid(&config, start)?;
    let map = mapping::AreaWeightedMapping::build(
        &grid,
        &topo.pixel,
        &topo.cells,
        &topo.shared_fraction,
    )?;
    let cells = map
        .parts
        .iter()
        .flatten()
        .map(|p| (p.ilon, p.ilat))
        .collect::<Vec<_>>();
    let mut gf = forcing::GriddedForcing::new(config, grid, cells, 1800)?;
    let mut now = forcing::Stamp::from_calendar(start);
    for step in 1..=steps {
        let t = colm_core::CalendarTime {
            year: now.year,
            julian_day: now.day as u16,
            seconds: now.sec as u32,
        };
        let co2 = colm_core::monthly_co2_ppm(colm_core::Co2Scenario::Off, 2010, 1)? * 1.0e-6;
        let cf = gf.step(t, co2).context("step")?;
        let pf = forcing::map_to_patches(&map, &gf, &cf);
        for (i, p) in pf.iter().enumerate() {
            let v = [
                p.t, p.q, p.psrf, p.pbot, p.prc, p.prl, p.us, p.vs, p.sols, p.soll, p.solsd,
                p.solld, p.frl, p.rhoair, p.pco2m, p.po2m, p.hgt_u,
            ];
            println!(
                "PRBF {:4}{:5}{}",
                step,
                i + 1,
                v.iter()
                    .map(|x| format!("{:>17X}", x.to_bits()))
                    .collect::<String>()
            );
        }
        now = now.add_seconds(1800);
    }
    Ok(())
}

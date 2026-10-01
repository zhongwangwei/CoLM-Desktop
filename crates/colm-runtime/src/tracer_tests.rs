//! 示踪物重启的往返与参数文件解析。

use super::*;

/// 每个测试一个独立目录（进程号 + 标签）。
fn temp_dir(label: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("colm-tracer-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn solute_set(dir: &Path) -> TracerSet {
    let param = dir.join("sol1.nml");
    std::fs::write(
        &param,
        "&nl_colm_tracer_parameter\n   DEF_TRACER%init_conc = 1.0\n   DEF_TRACER%precip_default_conc = 2.0\n/\n",
    )
    .unwrap();
    let namelist = TracerNamelist {
        num: 1,
        names: "sol1".to_owned(),
        types: "solute".to_owned(),
        param_files: param.display().to_string(),
        aquifer_mixing_water_mm: -1.0,
        ..TracerNamelist::default()
    };
    TracerSet::build(&namelist, read_tracer_parameter_file).unwrap()
}

#[test]
fn parameter_files_override_the_concentrations() {
    let dir = temp_dir("param");
    let set = solute_set(&dir);
    let tracer = &set.tracers[0];
    assert_eq!(tracer.init_conc, 1.0);
    assert_eq!(tracer.precip_default_conc, 2.0);
    // 没写的回落到 `init_delta`（默认 0）。
    assert_eq!(tracer.vapor_default_conc, 0.0);
}

#[test]
fn land_tracer_restart_round_trips() {
    let dir = temp_dir("roundtrip");
    let set = solute_set(&dir);
    let path = dir.join("restart.nc");
    {
        let mut file = netcdf::create(&path).unwrap();
        file.add_dimension("patch", 2).unwrap();
        file.add_dimension("soilsnow", SOISNO_LAYERS).unwrap();
    }
    let mut states = vec![PatchTracerState::allocated(&set); 2];
    for (patch, state) in states.iter_mut().enumerate() {
        let pools = &mut state.pools[0];
        pools.ldew_rain = 0.5 + patch as f64;
        pools.wa = -3.25;
        pools.wliq_soisno[7] = 11.0 + patch as f64;
        pools.scv = 0.125;
        state.aquifer_ref_water = 2.0;
    }
    let refs: Vec<&PatchTracerState> = states.iter().collect();
    write_land_tracer_restart(&path, &set, &refs, -1.0).unwrap();
    let restart = colm_init::RestartFile::open(&path).unwrap();
    let read = read_land_tracer_restart(&restart, &set, 2).unwrap().unwrap();
    for (a, b) in read.iter().zip(&states) {
        assert_eq!(a.pools, b.pools);
        assert_eq!(a.aquifer_ref_water, b.aquifer_ref_water);
    }
}

#[test]
fn an_empty_transaction_is_not_a_compatible_restart() {
    let dir = temp_dir("cold");
    let set = solute_set(&dir);
    let path = dir.join("cold.nc");
    {
        let mut file = netcdf::create(&path).unwrap();
        file.add_dimension("patch", 1).unwrap();
    }
    colm_init::write_empty_land_tracer_transaction(&path, -1.0).unwrap();
    let restart = colm_init::RestartFile::open(&path).unwrap();
    assert!(read_land_tracer_restart(&restart, &set, 1).unwrap().is_none());
}

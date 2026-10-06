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

/// 不带宿主量的检查：跳过含水层同位素与 `patchtype` 两项。
const NO_HOST_CHECK: RestartStateCheck<'static> = RestartStateCheck {
    wa: &[],
    patch_types: &[],
    variably_saturated_flow: false,
};

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
        // 没有同位素时参考水量必须为 0（`validate_land_tracer_restart_state` 的 reference 检查）。
        state.aquifer_ref_water = 0.0;
    }
    let refs: Vec<&PatchTracerState> = states.iter().collect();
    write_land_tracer_restart(&path, &set, &refs, -1.0, None, &NO_HOST_CHECK).unwrap();
    let restart = colm_init::RestartFile::open(&path).unwrap();
    let read = read_land_tracer_restart(&restart, &set, 2)
        .unwrap()
        .unwrap();
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
    assert!(read_land_tracer_restart(&restart, &set, 1)
        .unwrap()
        .is_none());
}

#[test]
fn the_forcing_cache_round_trips_and_checks_its_identity() {
    let dir = temp_dir("forcing-cache");
    let set = solute_set(&dir);
    let path = dir.join("restart.nc");
    {
        let mut file = netcdf::create(&path).unwrap();
        file.add_dimension("patch", 2).unwrap();
        file.add_dimension("soilsnow", SOISNO_LAYERS).unwrap();
    }
    let states = vec![PatchTracerState::allocated(&set); 2];
    let refs: Vec<&PatchTracerState> = states.iter().collect();
    let identity = std::sync::Arc::new(vec![7; crate::spatial::tracer_forcing::ID_WIDTH * 2]);
    let precip = [0.5, 0.25];
    let vapor = [0.125, 0.0625];
    let cache = ForcingCache {
        nvars: 1,
        ntracers: 1,
        identity: std::sync::Arc::clone(&identity),
        precip: &precip,
        vapor: &vapor,
    };
    write_land_tracer_restart(&path, &set, &refs, -1.0, Some(&cache), &NO_HOST_CHECK).unwrap();
    let restart = colm_init::RestartFile::open(&path).unwrap();
    let loadable = land_tracer_restart_loadable(&restart, &set);
    assert!(loadable);
    let (p, v) = read_forcing_cache(&restart, loadable, 1, &identity)
        .unwrap()
        .unwrap();
    assert_eq!(p, precip);
    assert_eq!(v, vapor);
    // 配置变了（变量数或指纹不同）就停机，与上游一致。
    assert!(read_forcing_cache(&restart, loadable, 2, &identity).is_err());
    let mut other = (*identity).clone();
    other[3] = 8;
    assert!(read_forcing_cache(&restart, loadable, 1, &other).is_err());
    // 没读到示踪物事务（冷启动）时不读缓存。
    assert!(read_forcing_cache(&restart, false, 1, &identity)
        .unwrap()
        .is_none());
}

fn restart_file(dir: &Path, patches: usize) -> std::path::PathBuf {
    let path = dir.join("restart.nc");
    let mut file = netcdf::create(&path).unwrap();
    file.add_dimension("patch", patches).unwrap();
    file.add_dimension("soilsnow", SOISNO_LAYERS).unwrap();
    path
}

/// 舍入负值（不低于 -1e-12）放行，写出的是归零后的值；有符号量 `trc_wa` 不动。
#[test]
fn restart_write_zeroes_negative_dust() {
    let dir = temp_dir("dust");
    let set = solute_set(&dir);
    let path = restart_file(&dir, 1);
    let mut state = PatchTracerState::allocated(&set);
    state.pools[0].wliq_soisno[9] = -8.67e-19;
    state.pools[0].scv = -1.0e-12;
    state.pools[0].wa = -3.0;
    write_land_tracer_restart(&path, &set, &[&state], -1.0, None, &NO_HOST_CHECK).unwrap();
    let restart = colm_init::RestartFile::open(&path).unwrap();
    let read = read_land_tracer_restart(&restart, &set, 1)
        .unwrap()
        .unwrap();
    assert_eq!(read[0].pools[0].wliq_soisno[9].to_bits(), 0.0_f64.to_bits());
    assert_eq!(read[0].pools[0].scv.to_bits(), 0.0_f64.to_bits());
    assert_eq!(read[0].pools[0].wa, -3.0);
    // 内存里的那份由调用方归零；写出函数只读。
    assert_eq!(state.pools[0].wliq_soisno[9], -8.67e-19);
    clamp_land_tracer_restart_dust(&set, &mut state);
    assert_eq!(state.pools[0].wliq_soisno[9].to_bits(), 0.0_f64.to_bits());
    assert_eq!(state.pools[0].wa, -3.0);
}

/// 超出舍入的负值、非有限值、参考水量不一致都拒绝写出，计数与上游同序。
#[test]
fn restart_write_rejects_corrupt_state() {
    let dir = temp_dir("corrupt");
    let set = solute_set(&dir);
    let path = restart_file(&dir, 1);
    let write = |state: &PatchTracerState, check: &RestartStateCheck<'_>| {
        write_land_tracer_restart(&path, &set, &[state], -1.0, None, check)
            .unwrap_err()
            .to_string()
    };
    let mut state = PatchTracerState::allocated(&set);
    state.pools[0].wliq_soisno[9] = -1.0e-9;
    assert!(write(&state, &NO_HOST_CHECK).ends_with("0 1 0 0 0 0 0"));
    let mut state = PatchTracerState::allocated(&set);
    state.pools[0].wdsrf = f64::NAN;
    state.pools[0].leaf_delta_e = f64::INFINITY;
    state.pools[0].leaf_peclet = 1.5;
    assert!(write(&state, &NO_HOST_CHECK).ends_with("1 0 1 1 0 0 0"));
    // 没有同位素时参考水量必须为 0；有同位素、变饱和流下土壤/湿地 patch 必须为正，其它类型为 0。
    let mut state = PatchTracerState::allocated(&set);
    state.aquifer_ref_water = 2.0;
    let message = write(&state, &NO_HOST_CHECK);
    assert!(message.ends_with("0 0 0 0 0 0 1"), "{message}");
}

#[test]
fn restart_write_checks_the_isotope_aquifer_reference() {
    let dir = temp_dir("isotope-reference");
    let param = dir.join("o18.nml");
    std::fs::write(
        &param,
        "&nl_colm_tracer_parameter\n   DEF_TRACER%ref_ratio = 2.0052e-3\n   DEF_TRACER%init_delta = -10.0\n/\n",
    )
    .unwrap();
    let set = TracerSet::build(
        &TracerNamelist {
            num: 1,
            names: "H2_18O".to_owned(),
            types: "isotope".to_owned(),
            param_files: param.display().to_string(),
            aquifer_mixing_water_mm: 1000.0,
            ..TracerNamelist::default()
        },
        read_tracer_parameter_file,
    )
    .unwrap();
    let path = restart_file(&dir, 2);
    let ratio = set.tracers[0].ref_ratio;
    let mut soil = PatchTracerState::allocated(&set);
    soil.aquifer_ref_water = 1000.0;
    soil.pools[0].aquifer_ref_mass = 1000.0 * ratio;
    soil.pools[0].wa = -50.0 * ratio;
    let mut lake = PatchTracerState::allocated(&set);
    lake.pools[0].wa = 0.0;
    let wa = [-50.0, 0.0];
    let check = RestartStateCheck {
        wa: &wa,
        patch_types: &[0, 4],
        variably_saturated_flow: true,
    };
    write_land_tracer_restart(&path, &set, &[&soil, &lake], 1000.0, None, &check).unwrap();
    // 土壤 patch 丢了参考水量；参考质量与参考水量对不上。
    let mut bad = soil.clone();
    bad.aquifer_ref_water = 0.0;
    let message = write_land_tracer_restart(&path, &set, &[&bad, &lake], 1000.0, None, &check)
        .unwrap_err()
        .to_string();
    assert!(message.ends_with(" 2"), "{message}");
    // 含水层里同位素与水的符号相反：没有物理的水载体。
    let mut debt = soil.clone();
    debt.pools[0].wa = 900.0 * ratio;
    let wa = [-1050.0, 0.0];
    let message = write_land_tracer_restart(
        &path,
        &set,
        &[&debt, &lake],
        1000.0,
        None,
        &RestartStateCheck { wa: &wa, ..check },
    )
    .unwrap_err()
    .to_string();
    assert!(message.ends_with("0 0 0 0 0 1 0"), "{message}");
}

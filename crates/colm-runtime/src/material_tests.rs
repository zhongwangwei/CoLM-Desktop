//! 材料重启的往返与参数文件解析。

use super::*;

/// 每个测试一个独立目录（进程号 + 标签）。
fn temp_dir(label: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("colm-material-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn solute_set(dir: &Path) -> MaterialSet {
    let param = dir.join("sol1.nml");
    std::fs::write(
        &param,
        "&nl_colm_solute_parameter\n   DEF_SOLUTE%init_conc = 1.0\n   DEF_SOLUTE%precip_default_conc = 2.0\n/\n",
    )
    .unwrap();
    let namelist = MaterialNamelist {
        solutes: "sol1".to_owned(),
        param_files: MaterialParamFiles {
            solutes: param.display().to_string(),
            ..MaterialParamFiles::default()
        },
        aquifer_mixing_water_mm: -1.0,
        ..MaterialNamelist::default()
    };
    MaterialSet::build(&namelist, read_solute_parameter_file).unwrap()
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
    let material = &set.materials[0];
    assert_eq!(material.init_conc, 1.0);
    assert_eq!(material.precip_default_conc, 2.0);
    // 没写的回落到 `init_delta`（默认 0）。
    assert_eq!(material.vapor_default_conc, 0.0);
}

#[test]
fn land_material_restart_round_trips() {
    let dir = temp_dir("roundtrip");
    let set = solute_set(&dir);
    let path = dir.join("restart.nc");
    {
        let mut file = netcdf::create(&path).unwrap();
        file.add_dimension("patch", 2).unwrap();
        file.add_dimension("soilsnow", SOISNO_LAYERS).unwrap();
    }
    let mut states = vec![PatchMaterialState::allocated(&set); 2];
    for (patch, state) in states.iter_mut().enumerate() {
        let pools = &mut state.pools[0];
        pools.ldew_rain = 0.5 + patch as f64;
        pools.wa = -3.25;
        pools.wliq_soisno[7] = 11.0 + patch as f64;
        pools.scv = 0.125;
        // 没有同位素时参考水量必须为 0（`validate_land_material_restart_state` 的 reference 检查）。
        state.aquifer_ref_water = 0.0;
    }
    let refs: Vec<&PatchMaterialState> = states.iter().collect();
    write_land_material_restart(&path, &set, &refs, -1.0, None, &NO_HOST_CHECK).unwrap();
    let restart = colm_init::RestartFile::open(&path).unwrap();
    let read = read_land_material_restart(&restart, &set, 2)
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
    colm_init::write_empty_land_material_transaction(&path, -1.0).unwrap();
    let restart = colm_init::RestartFile::open(&path).unwrap();
    assert!(read_land_material_restart(&restart, &set, 1)
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
    let states = vec![PatchMaterialState::allocated(&set); 2];
    let refs: Vec<&PatchMaterialState> = states.iter().collect();
    let identity = std::sync::Arc::new(vec![7; crate::spatial::material_forcing::ID_WIDTH * 2]);
    let precip = [0.5, 0.25];
    let vapor = [0.125, 0.0625];
    let reliable = [1.0, 0.0];
    let cache = ForcingCache {
        nvars: 1,
        nmaterials: 1,
        identity: std::sync::Arc::clone(&identity),
        precip: &precip,
        vapor: &vapor,
        reliable: &reliable,
    };
    write_land_material_restart(&path, &set, &refs, -1.0, Some(&cache), &NO_HOST_CHECK).unwrap();
    let restart = colm_init::RestartFile::open(&path).unwrap();
    let loadable = land_material_restart_loadable(&restart, &set);
    assert!(loadable);
    let (p, v, r) = read_forcing_cache(&restart, loadable, 1, &identity)
        .unwrap()
        .unwrap();
    assert_eq!(p, precip);
    assert_eq!(v, vapor);
    assert_eq!(r, Some(reliable.to_vec()));
    // 配置变了（变量数或指纹不同）就停机，与上游一致。
    assert!(read_forcing_cache(&restart, loadable, 2, &identity).is_err());
    let mut other = (*identity).clone();
    other[3] = 8;
    assert!(read_forcing_cache(&restart, loadable, 1, &other).is_err());
    // 没读到材料事务（冷启动）时不读缓存。
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
    let mut state = PatchMaterialState::allocated(&set);
    state.pools[0].wliq_soisno[9] = -8.67e-19;
    state.pools[0].scv = -1.0e-12;
    state.pools[0].wa = -3.0;
    write_land_material_restart(&path, &set, &[&state], -1.0, None, &NO_HOST_CHECK).unwrap();
    let restart = colm_init::RestartFile::open(&path).unwrap();
    let read = read_land_material_restart(&restart, &set, 1)
        .unwrap()
        .unwrap();
    assert_eq!(read[0].pools[0].wliq_soisno[9].to_bits(), 0.0_f64.to_bits());
    assert_eq!(read[0].pools[0].scv.to_bits(), 0.0_f64.to_bits());
    assert_eq!(read[0].pools[0].wa, -3.0);
    // 内存里的那份由调用方归零；写出函数只读。
    assert_eq!(state.pools[0].wliq_soisno[9], -8.67e-19);
    clamp_land_material_restart_dust(&set, &mut state);
    assert_eq!(state.pools[0].wliq_soisno[9].to_bits(), 0.0_f64.to_bits());
    assert_eq!(state.pools[0].wa, -3.0);
}

/// 超出舍入的负值、非有限值、参考水量不一致都拒绝写出，计数与上游同序。
#[test]
fn restart_write_rejects_corrupt_state() {
    let dir = temp_dir("corrupt");
    let set = solute_set(&dir);
    let path = restart_file(&dir, 1);
    let write = |state: &PatchMaterialState, check: &RestartStateCheck<'_>| {
        write_land_material_restart(&path, &set, &[state], -1.0, None, check)
            .unwrap_err()
            .to_string()
    };
    let mut state = PatchMaterialState::allocated(&set);
    state.pools[0].wliq_soisno[9] = -1.0e-9;
    assert!(write(&state, &NO_HOST_CHECK).ends_with("0 1 0 0 0 0 0"));
    let mut state = PatchMaterialState::allocated(&set);
    state.pools[0].wdsrf = f64::NAN;
    state.pools[0].leaf_delta_e = f64::INFINITY;
    state.pools[0].leaf_peclet = 1.5;
    assert!(write(&state, &NO_HOST_CHECK).ends_with("1 0 1 1 0 0 0"));
    // 没有同位素时参考水量必须为 0；有同位素、变饱和流下土壤/湿地 patch 必须为正，其它类型为 0。
    let mut state = PatchMaterialState::allocated(&set);
    state.aquifer_ref_water = 2.0;
    let message = write(&state, &NO_HOST_CHECK);
    assert!(message.ends_with("0 0 0 0 0 0 1"), "{message}");
}

#[test]
fn restart_write_checks_the_isotope_aquifer_reference() {
    let dir = temp_dir("isotope-reference");
    let set = MaterialSet::build(
        &MaterialNamelist {
            use_isotope: true,
            isotope_species: "H2_18O".to_owned(),
            ref_ratio_h2_18o: 2.0052e-3,
            init_delta_h2_18o: -10.0,
            aquifer_mixing_water_mm: 1000.0,
            ..MaterialNamelist::default()
        },
        read_solute_parameter_file,
    )
    .unwrap();
    let path = restart_file(&dir, 2);
    let ratio = set.materials[0].ref_ratio;
    let mut soil = PatchMaterialState::allocated(&set);
    soil.aquifer_ref_water = 1000.0;
    soil.pools[0].aquifer_ref_mass = 1000.0 * ratio;
    soil.pools[0].wa = -50.0 * ratio;
    let mut lake = PatchMaterialState::allocated(&set);
    lake.pools[0].wa = 0.0;
    let wa = [-50.0, 0.0];
    let check = RestartStateCheck {
        wa: &wa,
        patch_types: &[0, 4],
        variably_saturated_flow: true,
    };
    write_land_material_restart(&path, &set, &[&soil, &lake], 1000.0, None, &check).unwrap();
    // 土壤 patch 丢了参考水量；参考质量与参考水量对不上。
    let mut bad = soil.clone();
    bad.aquifer_ref_water = 0.0;
    let message = write_land_material_restart(&path, &set, &[&bad, &lake], 1000.0, None, &check)
        .unwrap_err()
        .to_string();
    assert!(message.ends_with(" 2"), "{message}");
    // 含水层里同位素与水的符号相反：没有物理的水载体。
    let mut debt = soil.clone();
    debt.pools[0].wa = 900.0 * ratio;
    let wa = [-1050.0, 0.0];
    let message = write_land_material_restart(
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

/// GUI 用 `colm_hist::methane` 判断输出变量页里 CH4 变量的开关，它必须与引擎实际写出的一致。
#[test]
fn methane_history_rules_match_the_engine() {
    let names: Vec<String> = colm_core::methane::driver::CoreAccumulator::default()
        .core_values(false, false, false)
        .into_iter()
        .map(|(name, ..)| name.to_owned())
        .collect();
    let shared: Vec<String> = colm_hist::methane::METHANE_CORE_HISTORY
        .iter()
        .map(|name| format!("f_{name}"))
        .collect();
    assert_eq!(names, shared);
    for name in colm_hist::methane::METHANE_CORE_HISTORY {
        assert!(
            colm_hist::generated::VARS
                .iter()
                .any(|var| var.name == name),
            "{name}"
        );
    }
    let mut params = colm_core::methane::config::MethaneParameters::default();
    for (write, vars) in [
        (true, "core"),
        (true, " NONE "),
        (false, "core"),
        (true, "diagnostic"),
        (true, "f_methane_surf_flux_tot"),
        (true, "fast"),
    ] {
        params.methane.write_ch4_history = write;
        params.methane.ch4_history_vars = vars.to_owned();
        assert_eq!(
            params.history_accumulation_mode(),
            colm_hist::methane::methane_history_mode(write, vars),
            "{write} {vars}"
        );
    }
}

/// 某个 solute 名的 `MaterialSet`（参数文件只放空 `&nl_colm_solute_parameter`）。
fn named_solute_set(dir: &Path, name: &str) -> MaterialSet {
    let param = dir.join(format!("{name}.nml"));
    std::fs::write(&param, "&nl_colm_solute_parameter\n/\n").unwrap();
    let namelist = MaterialNamelist {
        solutes: name.to_owned(),
        param_files: MaterialParamFiles {
            solutes: param.display().to_string(),
            ..MaterialParamFiles::default()
        },
        aquifer_mixing_water_mm: -1.0,
        ..MaterialNamelist::default()
    };
    MaterialSet::build(&namelist, read_solute_parameter_file).unwrap()
}

#[test]
fn nh4_leaching_switch_follows_the_material_namelist_mode() {
    let parse = |text: &str| colm_namelist::parse(text).unwrap();
    // 没有材料组时 DEF_NH4_LEACHING 取 schema 默认 'auto'，没有 NH4 token → 关。
    assert!(!resolve_nh4_leaching(&parse("&nl_colm\n/\n")).unwrap());
    // 'auto' 看 DEF_SOLUTE_NAMES 里的独立 NH4 token（大小写/逗号空格不敏感）。
    assert!(resolve_nh4_leaching(&parse(
        "&nl_colm_material\n DEF_SOLUTE_NAMES = 'CL, nh4 '\n/\n"
    ))
    .unwrap());
    assert!(
        !resolve_nh4_leaching(&parse("&nl_colm_material\n DEF_SOLUTE_NAMES = 'NH4X'\n/\n"))
            .unwrap()
    );
    // 'on'/'off' 强制；非法模式报错（上游 CoLM_Stop）。
    assert!(
        resolve_nh4_leaching(&parse("&nl_colm_material\n DEF_NH4_LEACHING = 'on'\n/\n")).unwrap()
    );
    assert!(!resolve_nh4_leaching(&parse(
        "&nl_colm_material\n DEF_SOLUTE_NAMES = 'NH4'\n DEF_NH4_LEACHING = 'off'\n/\n"
    ))
    .unwrap());
    assert!(resolve_nh4_leaching(&parse(
        "&nl_colm_material\n DEF_NH4_LEACHING = 'maybe'\n/\n"
    ))
    .is_err());
}

#[test]
fn bgc_nitrogen_leaching_books_species_mass_like_the_fortran_coupling() {
    // gN m-3 s-1 × dz × deltim / 1000 = kg N m-2；NITRIF 开时 NH4 只走地下、
    // NO3 走地下+地表；NITRIF 关时聚合通量记到 NH4（MOD_MT_NitrogenLeaching.F90:49-115）。
    let dir = temp_dir("bgc-nh4-mass");
    let dz = colm_core::colm_soil_grid(colm_core::bgc_state::BgcDims::default().nl_soil)
        .unwrap()
        .thickness_m;
    let mass = |flux: f64| -> f64 { flux * dz.iter().sum::<f64>() * 1800.0 / 1000.0 };
    let dims = colm_core::bgc_state::BgcDims::default();

    let set = named_solute_set(&dir, "NH4");
    let mut bgc = colm_core::bgc_state::BgcState::new(0, dims);
    bgc.patch_flux.smin_nh4_leached_vr[..dz.len()].fill(1.0e-4);
    let masses = bgc_nitrogen_leaching_masses(
        &set,
        &bgc,
        BgcSwitches {
            nitrif: true,
            nh4_leaching: true,
            ..BgcSwitches::default()
        },
        &dz,
        1800.0,
    );
    assert_eq!(masses.len(), 1);
    assert_eq!(masses[0].0, 0);
    assert!((masses[0].1 - mass(1.0e-4)).abs() < 1.0e-18);
    assert_eq!(masses[0].2, 0.0);

    let set = named_solute_set(&dir, "NO3");
    let mut bgc = colm_core::bgc_state::BgcState::new(0, dims);
    for j in 0..dz.len() {
        bgc.patch_flux.smin_no3_leached_vr[j] = 2.0e-4;
        bgc.patch_flux.smin_no3_runoff_vr[j] = 3.0e-4;
    }
    let masses = bgc_nitrogen_leaching_masses(
        &set,
        &bgc,
        BgcSwitches {
            nitrif: true,
            ..BgcSwitches::default()
        },
        &dz,
        1800.0,
    );
    assert_eq!(masses.len(), 1);
    assert!((masses[0].1 - mass(2.0e-4)).abs() < 1.0e-18);
    assert!((masses[0].2 - mass(3.0e-4)).abs() < 1.0e-18);

    let set = named_solute_set(&dir, "NH4");
    let mut bgc = colm_core::bgc_state::BgcState::new(0, dims);
    bgc.patch_flux.sminn_leached_vr[..dz.len()].fill(5.0e-4);
    let masses = bgc_nitrogen_leaching_masses(
        &set,
        &bgc,
        BgcSwitches {
            nitrif: false,
            ..BgcSwitches::default()
        },
        &dz,
        1800.0,
    );
    assert_eq!(masses.len(), 1);
    assert!((masses[0].1 - mass(5.0e-4)).abs() < 1.0e-18);
    assert_eq!(masses[0].2, 0.0);
}

#[test]
fn bgc_nitrogen_leaching_masses_are_booked_into_the_runoff_accumulators() {
    // 上游 `mt_bgc_n_leaching` 把同一份质量记进 a_mt_rsub/a_mt_rsur、a_mt_rnof、
    // a_mt_bgc_leach 与 mt_rnof_step；<=0 的条目跳过（2026-11-12 移植）。
    let dir = temp_dir("bgc-nh4-book");
    let set = named_solute_set(&dir, "NH4");
    let mut state = colm_core::material::PatchMaterialState::allocated(&set);
    let close = |a: f64, b: f64| assert!((a - b).abs() < 1.0e-15, "{a} != {b}");
    apply_bgc_leaching_masses(&mut state, &[(0, 2.0e-5, 3.0e-5)]);
    close(state.acc[0].rsub, 2.0e-5);
    close(state.acc[0].rsur, 3.0e-5);
    close(state.acc[0].rnof, 5.0e-5);
    close(state.acc[0].bgc_leach, 5.0e-5);
    close(state.step[0].rnof_step, 5.0e-5);
    apply_bgc_leaching_masses(&mut state, &[(0, 1.0e-5, 0.0), (0, 0.0, 0.0)]);
    close(state.acc[0].rsub, 3.0e-5);
    close(state.acc[0].bgc_leach, 6.0e-5);
    close(state.step[0].rnof_step, 6.0e-5);
}

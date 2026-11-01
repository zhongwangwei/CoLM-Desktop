//! 旁车示踪物部分的往返：44 个累加器的盘上布局、描述符校验与旧名回退。

use super::*;
use colm_core::tracer::{ReactionMode, StateOwner, TracerDescriptor, TracerFamily};

fn temp_file(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("colm-trc-sidecar-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("sidecar.nc")
}

fn descriptor(name: &str, family: TracerFamily) -> TracerDescriptor {
    TracerDescriptor {
        name: name.to_owned(),
        category: if family == TracerFamily::Isotope {
            "isotope"
        } else {
            "solute"
        }
        .to_owned(),
        family,
        state_owner: StateOwner::GenericWater,
        reaction_mode: ReactionMode::None,
        unit_kind: "ratio".to_owned(),
        mol_weight: 18.0,
        ref_ratio: 1.0,
        init_delta: 0.0,
        init_conc: 0.0,
        precip_default_conc: 0.0,
        vapor_default_conc: 0.0,
        max_dissolved_conc: f64::MAX,
        reactive_decay_rate: 0.0,
        n2o_ef5: 0.0,
        charge: 0,
    }
}

/// 主机部分的最小旁车：`patch` 维与 `hacc_nac` 标记。
fn host_sidecar(path: &Path, patches: usize, nac: f64) {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("patch", patches).unwrap();
    file.add_variable::<f64>("hacc_nac", &["patch"])
        .unwrap()
        .put_values(&vec![nac; patches], ..)
        .unwrap();
}

/// 旧 Rust 旁车用 `history_nac` 标记；读回要能认。
fn legacy_host_sidecar(path: &Path, patches: usize, nac: f64) {
    let mut file = netcdf::create(path).unwrap();
    file.add_dimension("patch", patches).unwrap();
    file.add_variable::<f64>("history_nac", &["patch"])
        .unwrap()
        .put_values(&vec![nac; patches], ..)
        .unwrap();
}

fn filled_state(set: &TracerSet, seed: f64) -> PatchTracerState {
    let mut state = PatchTracerState::allocated(set);
    for (i, acc) in state.acc.iter_mut().enumerate() {
        let base = seed + 100.0 * i as f64;
        acc.precip = base + 1.0;
        acc.scv_mass = base + 2.0;
        for j in 0..SOIL_LAYERS {
            acc.soil_mass[j] = base + 10.0 + j as f64;
        }
        for j in 0..MAX_SNOW_LAYERS {
            acc.snow_mass[j] = base + 30.0 + j as f64;
        }
    }
    state.water_acc.ldew = seed + 0.5;
    state.water_acc.soil = std::array::from_fn(|j| seed + 50.0 + j as f64);
    state.water_acc.snow = std::array::from_fn(|j| seed + 70.0 + j as f64);
    state
}

#[test]
fn the_tracer_window_round_trips_with_the_upstream_layout() {
    let set = TracerSet {
        tracers: vec![
            descriptor("HDO", TracerFamily::Isotope),
            descriptor("Cl", TracerFamily::Solute),
        ],
    };
    let states = [filled_state(&set, 0.0), filled_state(&set, 1000.0)];
    let path = temp_file("roundtrip");
    host_sidecar(&path, 2, 3.0);
    write(
        &path,
        &SidecarTracers {
            set: Some(&set),
            patches: states.iter().map(PatchAccumulators::of).collect(),
            methane: None,
        },
    )
    .unwrap();

    // 盘上：`a_mt_soil_mass` 是 (patch, d2 = 层, d1 = 示踪物)，示踪物变化最快。
    let file = netcdf::open(&path).unwrap();
    let soil = file.variable("a_mt_soil_mass").unwrap();
    let lens: Vec<usize> = soil.dimensions().iter().map(|d| d.len()).collect();
    assert_eq!(lens, [2, SOIL_LAYERS, 2]);
    let values = soil.get_values::<f64, _>(..).unwrap();
    assert_eq!(&values[..4], &[10.0, 110.0, 11.0, 111.0]);
    let water = file.variable("a_water_snow").unwrap();
    assert_eq!(water.dimensions().len(), 2);
    assert_eq!(
        file.variable("mt_hist_count")
            .unwrap()
            .get_value::<i32, _>(())
            .unwrap(),
        2
    );
    drop(file);

    let restored = read(&path, Some(&set), None, 2).unwrap();
    let tracers = restored.tracers.unwrap();
    for (patch, state) in states.iter().enumerate() {
        assert_eq!(tracers[patch].tracers, state.acc);
        assert_eq!(tracers[patch].water, state.water_acc);
    }
    assert!(restored.methane.is_none());
}

#[test]
fn a_closed_window_or_another_tracer_set_is_handled() {
    let set = TracerSet {
        tracers: vec![descriptor("HDO", TracerFamily::Isotope)],
    };
    let states = [filled_state(&set, 0.0)];
    let path = temp_file("descriptor");
    host_sidecar(&path, 1, 2.0);
    write(
        &path,
        &SidecarTracers {
            set: Some(&set),
            patches: states.iter().map(PatchAccumulators::of).collect(),
            methane: None,
        },
    )
    .unwrap();
    // 示踪物名字变了：描述符对不上就拒绝。
    let renamed = TracerSet {
        tracers: vec![descriptor("H218O", TracerFamily::Isotope)],
    };
    let error = read(&path, Some(&renamed), None, 1)
        .unwrap_err()
        .to_string();
    assert!(error.contains("incompatible"), "{error}");

    // 区间已关：什么都不读。
    let closed = temp_file("closed");
    host_sidecar(&closed, 1, 0.0);
    let restored = read(&closed, Some(&set), None, 1).unwrap();
    assert!(restored.tracers.is_none());
}

/// 旁车复制一份、`a_mt_*` 全换成旧名 `a_trc_*`：模拟 MATERIAL 改名前的旁车文件。
fn legacy_copy(source: &Path, target: &Path) {
    let source = netcdf::open(source).unwrap();
    let mut target = netcdf::create(target).unwrap();
    let mut defined = std::collections::HashSet::new();
    for variable in source.variables() {
        let name = variable.name();
        let dims: Vec<(String, usize)> = variable
            .dimensions()
            .iter()
            .map(|dim| (dim.name(), dim.len()))
            .collect();
        for (dim, len) in &dims {
            if defined.insert(dim.clone()) {
                target.add_dimension(dim, *len).unwrap();
            }
        }
        let legacy = name
            .strip_prefix("a_mt_")
            .map_or_else(|| name.clone(), |rest| format!("a_trc_{rest}"));
        let dim_names: Vec<&str> = dims.iter().map(|(dim, _)| dim.as_str()).collect();
        match variable.vartype() {
            netcdf::types::NcVariableType::Int(_) => {
                let mut written = target.add_variable::<i32>(&legacy, &dim_names).unwrap();
                if dims.is_empty() {
                    let value = variable.get_value::<i32, _>(()).unwrap();
                    written.put_value(value, ()).unwrap();
                } else {
                    let values = variable.get_values::<i32, _>(..).unwrap();
                    written.put_values(&values, ..).unwrap();
                }
            }
            netcdf::types::NcVariableType::Float(_) => {
                let mut written = target.add_variable::<f64>(&legacy, &dim_names).unwrap();
                if dims.is_empty() {
                    let value = variable.get_value::<f64, _>(()).unwrap();
                    written.put_value(value, ()).unwrap();
                } else {
                    let values = variable.get_values::<f64, _>(..).unwrap();
                    written.put_values(&values, ..).unwrap();
                }
            }
            other => panic!("unexpected sidecar type {other:?}"),
        }
    }
}

#[test]
fn the_legacy_a_trc_names_are_read_one_field_at_a_time() {
    let set = TracerSet {
        tracers: vec![
            descriptor("HDO", TracerFamily::Isotope),
            descriptor("Cl", TracerFamily::Solute),
        ],
    };
    let states = [filled_state(&set, 0.0), filled_state(&set, 1000.0)];
    let path = temp_file("legacy-source");
    host_sidecar(&path, 2, 3.0);
    write(
        &path,
        &SidecarTracers {
            set: Some(&set),
            patches: states.iter().map(PatchAccumulators::of).collect(),
            methane: None,
        },
    )
    .unwrap();
    let legacy = temp_file("legacy-names");
    legacy_copy(&path, &legacy);
    assert!(
        netcdf::open(&legacy)
            .unwrap()
            .variable("a_mt_precip")
            .is_none(),
        "the legacy copy must not keep the new names"
    );
    // 整份旧名旁车照样读得回来，值与原名那份逐位相同。
    let restored = read(&legacy, Some(&set), None, 2).unwrap();
    for (patch, state) in states.iter().enumerate() {
        assert_eq!(restored.tracers.as_ref().unwrap()[patch].tracers, state.acc);
    }

    // 混一半新名：新名存在时必须读新名，其余字段仍回退旧名。维名沿用了复制前那份，
    // 读回只按长度校验，所以直接挂到已有维上。
    let mut mixed = netcdf::append(&legacy).unwrap();
    mixed
        .add_variable::<f64>("a_mt_precip", &["patch", "d1_a_mt_precip"])
        .unwrap()
        .put_values(&[777.0, 888.0, 999.0, 1000.0], ..)
        .unwrap();
    drop(mixed);
    let restored = read(&legacy, Some(&set), None, 2).unwrap();
    let tracers = restored.tracers.as_ref().unwrap();
    assert_eq!(tracers[0].tracers[0].precip, 777.0);
    assert_eq!(tracers[0].tracers[1].precip, 888.0);
    assert_eq!(tracers[1].tracers[0].precip, 999.0);
    assert_eq!(tracers[1].tracers[1].precip, 1000.0);
    // 其余字段仍来自旧名。
    assert_eq!(tracers[0].tracers[0].evap, states[0].acc[0].evap);
}

#[test]
fn the_read_name_falls_back_per_field() {
    let path = temp_file("read-name");
    let mut file = netcdf::create(&path).unwrap();
    file.add_dimension("patch", 1).unwrap();
    file.add_variable::<f64>("a_trc_precip", &["patch"])
        .unwrap();
    file.add_variable::<f64>("a_water_precip", &["patch"])
        .unwrap();
    drop(file);
    let file = netcdf::open(&path).unwrap();
    // 新名不在、旧名在：逐字段回退。没改过名的 `a_water_*` 照旧。
    assert_eq!(read_name(&file, "a_mt_precip").unwrap(), "a_trc_precip");
    assert_eq!(
        read_name(&file, "a_water_precip").unwrap(),
        "a_water_precip"
    );
    // 新旧名都不在：报错里两个名字都要有，方便对出是哪一段旁车缺字段。
    let error = read_name(&file, "a_mt_evap").unwrap_err().to_string();
    assert!(
        error.contains("a_mt_evap") && error.contains("a_trc_evap"),
        "{error}"
    );
}

#[test]
fn the_legacy_history_nac_marker_still_opens_the_tracer_window() {
    let set = TracerSet {
        tracers: vec![descriptor("Cl", TracerFamily::Solute)],
    };
    let states = [filled_state(&set, 0.0)];
    let path = temp_file("legacy-nac");
    legacy_host_sidecar(&path, 1, 2.0);
    write(
        &path,
        &SidecarTracers {
            set: Some(&set),
            patches: states.iter().map(PatchAccumulators::of).collect(),
            methane: None,
        },
    )
    .unwrap();
    let restored = read(&path, Some(&set), None, 1).unwrap();
    assert_eq!(restored.tracers.as_ref().unwrap()[0].tracers, states[0].acc);
}

/// 上游 Fortran 写的旁车只有 `a_mt_*`/`a_water_*`、没有 `mt_hist_*` 私有描述符；
/// 读取应回退到上游布局（按名字、`(patch, d1/d2)` 形状整组读回）。
#[test]
fn an_upstream_fortran_sidecar_is_read_by_field_name() {
    let set = TracerSet {
        tracers: vec![descriptor("H2_18O", TracerFamily::Isotope)],
    };
    let patches = 2;
    let path = temp_file("upstream-layout");
    let mut file = netcdf::create(&path).unwrap();
    file.add_dimension("patch", patches).unwrap();
    let mut defined = std::collections::HashSet::new();
    let mut expected = vec![
        PatchAccumulators {
            tracers: vec![TracerAccumulators::default(); set.len()],
            water: WaterAccumulators::default(),
        };
        patches
    ];
    for (index, &(name, slot)) in FIELDS.iter().enumerate() {
        let (dims, width) = shape(name, slot, set.len());
        for (dim, len) in &dims {
            if defined.insert(dim.clone()) {
                file.add_dimension(dim, *len).unwrap();
            }
        }
        let mut names = vec!["patch"];
        names.extend(dims.iter().map(|(dim, _)| dim.as_str()));
        let values: Vec<f64> = (0..patches * width)
            .map(|k| (index * 1000 + k) as f64)
            .collect();
        let mut variable = file.add_variable::<f64>(name, &names).unwrap();
        variable.put_values(&values, ..).unwrap();
        for (patch, accumulators) in expected.iter_mut().enumerate() {
            accumulators.set(slot, &values[patch * width..(patch + 1) * width]);
        }
    }
    let mut nac = file.add_variable::<f64>("hacc_nac", &["patch"]).unwrap();
    nac.put_values(&vec![1.0; patches], ..).unwrap();
    drop(file);

    let restored = read(&path, Some(&set), None, patches).unwrap();
    let read_back = restored
        .tracers
        .expect("upstream layout should be restored");
    assert_eq!(read_back, expected);
}

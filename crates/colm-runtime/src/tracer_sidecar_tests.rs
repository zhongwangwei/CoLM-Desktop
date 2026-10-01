//! 旁车示踪物部分的往返：44 个累加器的盘上布局与描述符校验。

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
        charge: 0,
    }
}

/// 主机部分的最小旁车：`patch` 维与 `history_nac`。
fn host_sidecar(path: &Path, patches: usize, nac: f64) {
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

    // 盘上：`a_trc_soil_mass` 是 (patch, d2 = 层, d1 = 示踪物)，示踪物变化最快。
    let file = netcdf::open(&path).unwrap();
    let soil = file.variable("a_trc_soil_mass").unwrap();
    let lens: Vec<usize> = soil.dimensions().iter().map(|d| d.len()).collect();
    assert_eq!(lens, [2, SOIL_LAYERS, 2]);
    let values = soil.get_values::<f64, _>(..).unwrap();
    assert_eq!(&values[..4], &[10.0, 110.0, 11.0, 111.0]);
    let water = file.variable("a_water_snow").unwrap();
    assert_eq!(water.dimensions().len(), 2);
    assert_eq!(
        file.variable("trc_hist_count")
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

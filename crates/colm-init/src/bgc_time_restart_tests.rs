use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

const PATCH: [f64; 2] = [10.0, 20.0];
const SOIL: [f64; 4] = [0.0, 1.0, 2.0, 3.0];
const FULL_POOL: [f64; 12] = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0];
const DAILY: [f64; 8] = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
const PATCH_I32: [i32; 2] = [3, 4];
const PATCH_I8: [i8; 2] = [0, 1];

#[test]
fn bgc_time_restart_matches_fortran_axis_layout_and_optional_fields() {
    let root = temp_dir("bgc-time");
    let date = RestartDate {
        year: 2008,
        julian_day: 1,
        seconds: 0,
    };
    let output =
        write_bgc_time_restart(&root, "CN-Cng", 2005, date, "w180_s90", sample_input()).unwrap();
    assert_eq!(
        output.block,
        root.join("2008-001-00000/CN-Cng_restart_bgc_2008-001-00000_lc2005_w180_s90.nc")
    );

    let file = netcdf::open(&output.block).unwrap();
    assert_eq!(file.dimension_len("patch"), Some(2));
    assert_eq!(file.dimension_len("soil"), Some(2));
    assert_eq!(file.dimension_len("soil_full"), Some(3));
    assert_eq!(file.dimension_len("ndecomp_pools"), Some(2));
    assert_eq!(file.dimension_len("doy"), Some(4));
    assert_eq!(
        file.variables()
            .map(|variable| variable.name())
            .collect::<Vec<_>>(),
        [
            "totlitc",
            "totvegc",
            "totsomc",
            "totcwdc",
            "totcolc",
            "totlitn",
            "totvegn",
            "totsomn",
            "totcwdn",
            "totcoln",
            "sminn",
            "ndep",
            "decomp_cpools_vr",
            "ctrunc_vr",
            "ctrunc_veg",
            "ctrunc_soil",
            "altmax",
            "altmax_lastyear",
            "altmax_lastyear_indx",
            "decomp_npools_vr",
            "totsoiln_vr",
            "ntrunc_vr",
            "ntrunc_veg",
            "ntrunc_soil",
            "sminn_vr",
            "smin_no3_vr",
            "smin_nh4_vr",
            "lag_npp",
            "tCONC_O2_UNSAT",
            "tO2_DECOMP_DEPTH_UNSAT",
            "prec10",
            "prec60",
            "prec365",
            "prec_today",
            "prec_daily",
            "tsoi17",
            "rh30",
            "accumnstep",
            "skip_balance_check",
        ]
    );
    assert_eq!(
        file.variable("decomp_cpools_vr")
            .unwrap()
            .dimensions()
            .iter()
            .map(|dimension| dimension.name())
            .collect::<Vec<_>>(),
        ["patch", "ndecomp_pools", "soil_full"]
    );
    assert_eq!(
        file.variable("decomp_cpools_vr")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        [0.0, 4.0, 8.0, 2.0, 6.0, 10.0, 1.0, 5.0, 9.0, 3.0, 7.0, 11.0]
    );
    assert_eq!(
        file.variable("ctrunc_vr")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        [0.0, 2.0, 1.0, 3.0]
    );
    assert_eq!(
        file.variable("prec_daily")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        [0.0, 2.0, 4.0, 6.0, 1.0, 3.0, 5.0, 7.0]
    );
    assert_eq!(
        file.variable("skip_balance_check")
            .unwrap()
            .get_values::<i8, _>(..)
            .unwrap(),
        PATCH_I8
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn bgc_time_restart_rejects_invalid_contract_before_creating_output() {
    let root = temp_dir("bgc-time-invalid");
    let mut input = sample_input();
    input.climate.precipitation_daily = &DAILY[..7];
    assert!(write_bgc_time_restart_block(root.join("restart.nc"), input).is_err());
    assert!(!root.exists());
    assert!(write_bgc_time_restart(
        &root,
        "bad/name",
        2005,
        RestartDate {
            year: 2008,
            julian_day: 1,
            seconds: 0,
        },
        "w180_s90",
        sample_input(),
    )
    .is_err());
}

#[test]
#[ignore = "requires the local BGC kernel and CoLMruntime cnsteadystate.nc reference data"]
fn bgc_time_restart_matches_the_upstream_fortran_reference() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let runtime = std::path::Path::new("/Users/zhongwangwei/Desktop/Data/CoLMruntime");
    let upstream_surface = root.join("kernels/bgc/mksrfdata.x");
    let upstream_init = root.join("kernels/bgc/mkinidata.x");
    assert!(runtime.join("cnsteadystate.nc").is_file());
    assert!(upstream_surface.is_file());
    assert!(upstream_init.is_file());

    let directory = temp_dir("bgc-time-fortran-reference");
    std::fs::create_dir_all(&directory).unwrap();
    let output = directory.join("out");
    let template = root.join("oracle/work/generated/case.nml");
    let original_output = format!("{}/oracle/work/generated/out/", root.display());
    let original_runtime = format!("{}/oracle/work/generated/runtime_unused/", root.display());
    let namelist = std::fs::read_to_string(template)
        .unwrap()
        .replace(
            &format!("DEF_dir_output = '{original_output}'"),
            &format!("DEF_dir_output = '{}/'", output.display()),
        )
        .replace(
            &format!("DEF_dir_runtime = '{original_runtime}'"),
            &format!("DEF_dir_runtime = '{}/'", runtime.display()),
        )
        .replace(
            "&nl_colm",
            &format!(
                "&nl_colm\nDEF_USE_LCT = .false.\nDEF_USE_PFT = .true.\nDEF_USE_BGC = .true.\nDEF_USE_CN_INIT = .true.\nDEF_file_cn_init = '{}/cnsteadystate.nc'",
                runtime.display()
            ),
        );
    let case = directory.join("case.nml");
    std::fs::write(&case, namelist).unwrap();
    for executable in [&upstream_surface, &upstream_init] {
        let result = std::process::Command::new(executable)
            .arg(&case)
            .current_dir(&directory)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{} failed:\n{}\n{}",
            executable.display(),
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }

    let reference_path = output
        .join("CN-Cng/restart/2008-001-00000/CN-Cng_restart_bgc_2008-001-00000_lc2005_w180_s90.nc");
    let reference = netcdf::open(&reference_path).unwrap();
    let dimensions = BgcTimeRestartDimensions {
        soil_layers: reference.dimension_len("soil").unwrap(),
        full_soil_layers: reference.dimension_len("soil_full").unwrap(),
        decomposition_pools: reference.dimension_len("ndecomp_pools").unwrap(),
        days_per_year: reference.dimension_len("doy").unwrap(),
    };
    let patches = reference.dimension_len("patch").unwrap();
    let f64_values = |name| {
        reference
            .variable(name)
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap()
    };
    let i32_values = |name| {
        reference
            .variable(name)
            .unwrap()
            .get_values::<i32, _>(..)
            .unwrap()
    };
    let i8_values = |name| {
        reference
            .variable(name)
            .unwrap()
            .get_values::<i8, _>(..)
            .unwrap()
    };
    let totlitc = f64_values("totlitc");
    let totvegc = f64_values("totvegc");
    let totsomc = f64_values("totsomc");
    let totcwdc = f64_values("totcwdc");
    let totcolc = f64_values("totcolc");
    let totlitn = f64_values("totlitn");
    let totvegn = f64_values("totvegn");
    let totsomn = f64_values("totsomn");
    let totcwdn = f64_values("totcwdn");
    let totcoln = f64_values("totcoln");
    let sminn = f64_values("sminn");
    let ndep = f64_values("ndep");
    let carbon = inverse_pool(
        f64_values("decomp_cpools_vr"),
        dimensions.full_soil_layers,
        dimensions.decomposition_pools,
        patches,
    );
    let nitrogen = inverse_pool(
        f64_values("decomp_npools_vr"),
        dimensions.full_soil_layers,
        dimensions.decomposition_pools,
        patches,
    );
    let soil = |name| inverse_axis(f64_values(name), dimensions.soil_layers, patches);
    let totsoiln_vr = soil("totsoiln_vr");
    let ctrunc_vr = soil("ctrunc_vr");
    let ntrunc_vr = soil("ntrunc_vr");
    let sminn_vr = soil("sminn_vr");
    let smin_no3_vr = soil("smin_no3_vr");
    let smin_nh4_vr = soil("smin_nh4_vr");
    let oxygen = reference.variable("tCONC_O2_UNSAT").map(|variable| {
        inverse_axis(
            variable.get_values::<f64, _>(..).unwrap(),
            dimensions.soil_layers,
            patches,
        )
    });
    let oxygen_depth = reference
        .variable("tO2_DECOMP_DEPTH_UNSAT")
        .map(|variable| {
            inverse_axis(
                variable.get_values::<f64, _>(..).unwrap(),
                dimensions.soil_layers,
                patches,
            )
        });
    let nitrification = match (&oxygen, &oxygen_depth) {
        (Some(oxygen), Some(oxygen_depth)) => Some(BgcNitrificationFields {
            oxygen_concentration_unsaturated: oxygen,
            oxygen_decomposition_depth_unsaturated: oxygen_depth,
        }),
        (None, None) => None,
        _ => panic!("upstream BGC restart has only one nitrification field"),
    };
    let lag_npp = f64_values("lag_npp");
    let ctrunc_veg = f64_values("ctrunc_veg");
    let ctrunc_soil = f64_values("ctrunc_soil");
    let ntrunc_veg = f64_values("ntrunc_veg");
    let ntrunc_soil = f64_values("ntrunc_soil");
    let altmax = f64_values("altmax");
    let altmax_lastyear = f64_values("altmax_lastyear");
    let altmax_lastyear_indx = i32_values("altmax_lastyear_indx");
    let prec10 = f64_values("prec10");
    let prec60 = f64_values("prec60");
    let prec365 = f64_values("prec365");
    let prec_today = f64_values("prec_today");
    let prec_daily = inverse_axis(f64_values("prec_daily"), dimensions.days_per_year, patches);
    let tsoi17 = f64_values("tsoi17");
    let rh30 = f64_values("rh30");
    let accumnstep = f64_values("accumnstep");
    let skip_balance_check = i8_values("skip_balance_check");
    let input = BgcTimeRestartInput {
        dimensions,
        totals: BgcTotals {
            litter_carbon: &totlitc,
            vegetation_carbon: &totvegc,
            soil_carbon: &totsomc,
            coarse_woody_carbon: &totcwdc,
            total_carbon: &totcolc,
            litter_nitrogen: &totlitn,
            vegetation_nitrogen: &totvegn,
            soil_nitrogen: &totsomn,
            coarse_woody_nitrogen: &totcwdn,
            total_nitrogen: &totcoln,
            mineral_nitrogen: &sminn,
            deposition: &ndep,
        },
        pools: BgcPoolFields {
            carbon: &carbon,
            nitrogen: &nitrogen,
            total_soil_nitrogen: &totsoiln_vr,
            mineral_nitrogen: &sminn_vr,
            nitrate: &smin_no3_vr,
            ammonium: &smin_nh4_vr,
            lagged_npp: &lag_npp,
        },
        truncation: BgcTruncationFields {
            carbon_profile: &ctrunc_vr,
            carbon_vegetation: &ctrunc_veg,
            carbon_soil: &ctrunc_soil,
            nitrogen_profile: &ntrunc_vr,
            nitrogen_vegetation: &ntrunc_veg,
            nitrogen_soil: &ntrunc_soil,
        },
        permafrost: BgcPermafrostFields {
            maximum_active_layer_depth: &altmax,
            previous_maximum_active_layer_depth: &altmax_lastyear,
            previous_maximum_active_layer_index: &altmax_lastyear_indx,
        },
        climate: BgcClimateFields {
            precipitation_10_day: &prec10,
            precipitation_60_day: &prec60,
            precipitation_365_day: &prec365,
            precipitation_today: &prec_today,
            precipitation_daily: &prec_daily,
            soil_temperature_17: &tsoi17,
            relative_humidity_30_day: &rh30,
            accumulated_steps: &accumnstep,
            skip_balance_check: &skip_balance_check,
        },
        nitrification,
    };
    let native = write_bgc_time_restart(
        directory.join("native/restart"),
        "CN-Cng",
        2005,
        RestartDate {
            year: 2008,
            julian_day: 1,
            seconds: 0,
        },
        "w180_s90",
        input,
    )
    .unwrap();
    compare_netcdf(&native.block, &reference_path);
    std::fs::remove_dir_all(directory).unwrap();
}

fn sample_input() -> BgcTimeRestartInput<'static> {
    BgcTimeRestartInput {
        dimensions: BgcTimeRestartDimensions {
            soil_layers: 2,
            full_soil_layers: 3,
            decomposition_pools: 2,
            days_per_year: 4,
        },
        totals: BgcTotals {
            litter_carbon: &PATCH,
            vegetation_carbon: &PATCH,
            soil_carbon: &PATCH,
            coarse_woody_carbon: &PATCH,
            total_carbon: &PATCH,
            litter_nitrogen: &PATCH,
            vegetation_nitrogen: &PATCH,
            soil_nitrogen: &PATCH,
            coarse_woody_nitrogen: &PATCH,
            total_nitrogen: &PATCH,
            mineral_nitrogen: &PATCH,
            deposition: &PATCH,
        },
        pools: BgcPoolFields {
            carbon: &FULL_POOL,
            nitrogen: &FULL_POOL,
            total_soil_nitrogen: &SOIL,
            mineral_nitrogen: &SOIL,
            nitrate: &SOIL,
            ammonium: &SOIL,
            lagged_npp: &PATCH,
        },
        truncation: BgcTruncationFields {
            carbon_profile: &SOIL,
            carbon_vegetation: &PATCH,
            carbon_soil: &PATCH,
            nitrogen_profile: &SOIL,
            nitrogen_vegetation: &PATCH,
            nitrogen_soil: &PATCH,
        },
        permafrost: BgcPermafrostFields {
            maximum_active_layer_depth: &PATCH,
            previous_maximum_active_layer_depth: &PATCH,
            previous_maximum_active_layer_index: &PATCH_I32,
        },
        climate: BgcClimateFields {
            precipitation_10_day: &PATCH,
            precipitation_60_day: &PATCH,
            precipitation_365_day: &PATCH,
            precipitation_today: &PATCH,
            precipitation_daily: &DAILY,
            soil_temperature_17: &PATCH,
            relative_humidity_30_day: &PATCH,
            accumulated_steps: &PATCH,
            skip_balance_check: &PATCH_I8,
        },
        nitrification: Some(BgcNitrificationFields {
            oxygen_concentration_unsaturated: &SOIL,
            oxygen_decomposition_depth_unsaturated: &SOIL,
        }),
    }
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "colm-init-{name}-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ))
}

fn inverse_axis(on_disk: Vec<f64>, axis: usize, patches: usize) -> Vec<f64> {
    let mut fortran = vec![0.0; on_disk.len()];
    for patch in 0..patches {
        for index in 0..axis {
            fortran[index * patches + patch] = on_disk[patch * axis + index];
        }
    }
    fortran
}

fn inverse_pool(on_disk: Vec<f64>, soils: usize, pools: usize, patches: usize) -> Vec<f64> {
    let mut fortran = vec![0.0; on_disk.len()];
    for patch in 0..patches {
        for pool in 0..pools {
            for soil in 0..soils {
                fortran[(soil * pools + pool) * patches + patch] =
                    on_disk[(patch * pools + pool) * soils + soil];
            }
        }
    }
    fortran
}

fn compare_netcdf(actual_path: &std::path::Path, expected_path: &std::path::Path) {
    let actual = netcdf::open(actual_path).unwrap();
    let expected = netcdf::open(expected_path).unwrap();
    let dimensions = |file: &netcdf::File| {
        file.dimensions()
            .map(|dimension| (dimension.name(), dimension.len()))
            .collect::<Vec<_>>()
    };
    let names = |file: &netcdf::File| {
        file.variables()
            .map(|variable| variable.name())
            .collect::<Vec<_>>()
    };
    assert_eq!(dimensions(&actual), dimensions(&expected));
    assert_eq!(names(&actual), names(&expected));
    for name in names(&expected) {
        let actual_var = actual.variable(&name).unwrap();
        let expected_var = expected.variable(&name).unwrap();
        assert_eq!(
            actual_var
                .dimensions()
                .iter()
                .map(|dimension| dimension.name())
                .collect::<Vec<_>>(),
            expected_var
                .dimensions()
                .iter()
                .map(|dimension| dimension.name())
                .collect::<Vec<_>>(),
            "dimension mismatch for {name}"
        );
        if name == "altmax_lastyear_indx" {
            assert_eq!(
                actual_var.get_values::<i32, _>(..).unwrap(),
                expected_var.get_values::<i32, _>(..).unwrap(),
                "value mismatch for {name}"
            );
        } else if name == "skip_balance_check" {
            assert_eq!(
                actual_var.get_values::<i8, _>(..).unwrap(),
                expected_var.get_values::<i8, _>(..).unwrap(),
                "value mismatch for {name}"
            );
        } else {
            assert_eq!(
                actual_var.get_values::<f64, _>(..).unwrap(),
                expected_var.get_values::<f64, _>(..).unwrap(),
                "value mismatch for {name}"
            );
        }
    }
}

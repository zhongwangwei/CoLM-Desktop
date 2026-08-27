use crate::tuning::{self, ReviewState, Scale, StudyParameter};

#[test]
fn registers_only_the_current_runtime_expert_fields() {
    let fields = tuning::all().unwrap();
    assert_eq!(fields.len(), 44);
    assert!(fields
        .iter()
        .all(|p| p.review == ReviewState::ExpertRangeOnly));
    assert!(fields.iter().any(|p| p.name == "DEF_BALL_BERRY_GRADM"));
    assert!(fields
        .iter()
        .any(|p| p.name == "DEF_DS_SHORTWAVE_SIMPLE_LIMIT"));
    assert!(fields
        .iter()
        .any(|p| p.name == "DEF_TUNING_IRRIGATION_START_SEC"));
    assert!(fields
        .iter()
        .any(|p| p.name == "DEF_TUNING_CROP_PLANTING_DAY"));
    for dead in [
        "DEF_TUNING_SMPMAX",
        "DEF_TUNING_SIMPLE_VIC_DS",
        "DEF_TUNING_SIMPLE_VIC_WS",
    ] {
        assert!(fields.iter().all(|p| p.name != dead), "{dead}");
    }
    assert!(fields.iter().all(|p| p.name != "DEF_MATSIRO_CWCAP_SCALE"));
}

#[test]
fn crop_planting_day_keeps_runtime_data_as_an_explicit_sentinel() {
    let parameter = tuning::find("DEF_TUNING_CROP_PLANTING_DAY")
        .unwrap()
        .unwrap();
    assert_eq!(parameter.default, 0.0);
    assert_eq!(parameter.sentinel.unwrap().value, 0.0);
    tuning::validate_value(parameter.name, 0.0).unwrap();
    tuning::validate_value(parameter.name, 1.0).unwrap();
    tuning::validate_value(parameter.name, 366.0).unwrap();
    assert!(tuning::validate_value(parameter.name, -1.0).is_err());
    assert!(tuning::validate_value(parameter.name, 120.5).is_err());
    assert!(tuning::validate_value(parameter.name, 367.0).is_err());
    assert!(tuning::validate_study_parameters(&[StudyParameter {
        name: parameter.name,
        sample_min: 0.0,
        sample_max: 200.0,
        scale: Scale::Linear,
    }])
    .is_err());
}

#[test]
fn stomata_sentinel_is_a_value_not_a_sample_range() {
    tuning::validate_value("DEF_MEDLYN_G1", -1.0).unwrap();
    tuning::validate_value("DEF_MEDLYN_G1", 0.0).unwrap();
    assert!(tuning::validate_value("DEF_BALL_BERRY_GRADM", 1.6).is_err());
    assert!(tuning::validate_study_parameters(&[StudyParameter {
        name: "DEF_MEDLYN_G1",
        sample_min: -1.0,
        sample_max: 2.0,
        scale: Scale::Linear,
    }])
    .is_err());
}

#[test]
fn finite_sample_ranges_must_respect_hard_bounds_and_log_scale() {
    tuning::validate_study_parameters(&[StudyParameter {
        name: "DEF_TUNING_ZLND",
        sample_min: 0.001,
        sample_max: 0.1,
        scale: Scale::Log,
    }])
    .unwrap();
    assert!(tuning::validate_study_parameters(&[StudyParameter {
        name: "DEF_TUNING_ZLND",
        sample_min: 0.0,
        sample_max: 0.1,
        scale: Scale::Log,
    }])
    .is_err());
    assert!(tuning::validate_study_parameters(&[StudyParameter {
        name: "DEF_DS_SHORTWAVE_LIMIT",
        sample_min: 0.2,
        sample_max: 1.2,
        scale: Scale::Linear,
    }])
    .is_err());
    assert!(tuning::validate_study_parameters(&[StudyParameter {
        name: "DEF_OZONE_KO3",
        sample_min: f64::NAN,
        sample_max: 1.0,
        scale: Scale::Linear,
    }])
    .is_err());
    for dead in [
        "DEF_TUNING_SMPMAX",
        "DEF_TUNING_SIMPLE_VIC_DS",
        "DEF_TUNING_SIMPLE_VIC_WS",
    ] {
        assert!(tuning::validate_study_parameters(&[StudyParameter {
            name: dead,
            sample_min: 0.1,
            sample_max: 0.4,
            scale: Scale::Linear,
        }])
        .is_err());
    }
}

#[test]
fn smp_ranges_must_be_disjoint_for_every_sample() {
    tuning::validate_study_parameters(&[StudyParameter {
        name: "DEF_TUNING_SMPMIN",
        sample_min: -2.0e8,
        sample_max: -2.0e5,
        scale: Scale::Linear,
    }])
    .unwrap();
    assert!(tuning::validate_study_parameters(&[
        StudyParameter {
            name: "DEF_TUNING_SMPMIN_HR",
            sample_min: -3.0e5,
            sample_max: -100.0,
            scale: Scale::Linear,
        },
        StudyParameter {
            name: "DEF_TUNING_SMPMAX_HR",
            sample_min: -200.0,
            sample_max: -10.0,
            scale: Scale::Linear,
        },
    ])
    .is_err());
}

#[test]
fn sampled_vectors_reject_duplicates_and_crossed_soil_potentials() {
    assert!(tuning::validate_values(&[
        ("DEF_TUNING_ZLND".into(), 0.01),
        ("def_tuning_zlnd".into(), 0.02),
    ])
    .is_err());
    assert!(tuning::validate_values(&[("DEF_TUNING_SMPMAX".into(), -200.0)]).is_err());
    assert!(tuning::validate_values(&[("DEF_TUNING_SIMPLE_VIC_DS".into(), 0.6)]).is_err());
    assert!(tuning::validate_values(&[
        ("DEF_TUNING_IRRIGATION_MIN_CPHASE".into(), 3.0),
        ("DEF_TUNING_IRRIGATION_MAX_CPHASE".into(), 2.0),
    ])
    .is_err());
}

#[test]
fn applies_runtime_values_without_touching_the_source_on_error() {
    let root = std::env::temp_dir().join(format!("colm-tuning-{}-apply", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("case.nml");
    let original = "&nl_colm\n   DEF_CASE_NAME = 'base'\n/\n";
    std::fs::write(&path, original).unwrap();
    tuning::apply_case_values(&path, &[("DEF_TUNING_ZLND".into(), 0.025)]).unwrap();
    let changed = std::fs::read_to_string(&path).unwrap();
    assert!(changed.contains("DEF_TUNING_ZLND = 2.50000000000000014e-2"));

    let before_error = changed;
    assert!(tuning::apply_case_values(&path, &[("DEF_TUNING_ZLND".into(), 0.0)]).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before_error);
    assert!(tuning::apply_case_values(
        &path,
        &[("DEF_TUNING_IRRIGATION_DURATION_SEC".into(), 900.0)]
    )
    .is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before_error);
    for dead in [
        "DEF_TUNING_SMPMAX",
        "DEF_TUNING_SIMPLE_VIC_DS",
        "DEF_TUNING_SIMPLE_VIC_WS",
    ] {
        assert!(tuning::apply_case_values(&path, &[(dead.into(), 0.6)]).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before_error);
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn study_rejects_sentinel_and_inactive_scheme_parameters() {
    let root = std::env::temp_dir().join(format!("colm-tuning-active-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("case.nml");
    std::fs::write(
        &path,
        "&nl_colm\n DEF_USE_MEDLYNST=.true.\n DEF_USE_WUEST=.false.\n DEF_MEDLYN_G1=-1.0\n DEF_BALL_BERRY_GRADM=9.0\n/\n",
    )
    .unwrap();
    assert!(tuning::validate_case_parameter_activity(
        &path,
        &["DEF_MEDLYN_G1".into()],
        &["SinglePoint".into()]
    )
    .is_err());
    assert!(tuning::validate_case_parameter_activity(
        &path,
        &["DEF_BALL_BERRY_GRADM".into()],
        &["SinglePoint".into()]
    )
    .is_err());
    std::fs::write(
        &path,
        "&nl_colm\n DEF_USE_MEDLYNST=.true.\n DEF_USE_WUEST=.false.\n DEF_MEDLYN_G1=4.0\n/\n",
    )
    .unwrap();
    tuning::validate_case_parameter_activity(
        &path,
        &["DEF_MEDLYN_G1".into()],
        &["SinglePoint".into()],
    )
    .unwrap();

    std::fs::write(
        &path,
        "&nl_colm\n SITE_landtype=12\n DEF_USE_IRRIGATION=.true.\n/\n",
    )
    .unwrap();
    tuning::validate_case_parameter_activity(
        &path,
        &["DEF_TUNING_IRRIGATION_START_SEC".into()],
        &["SinglePoint".into(), "LULC_IGBP".into(), "CROP".into()],
    )
    .unwrap();
    std::fs::write(
        &path,
        "&nl_colm\n SITE_landtype=12\n DEF_TUNING_CROP_PLANTING_DAY=120.\n/\n",
    )
    .unwrap();
    tuning::validate_case_parameter_activity(
        &path,
        &["DEF_TUNING_CROP_PLANTING_DAY".into()],
        &["SinglePoint".into(), "LULC_IGBP".into(), "CROP".into()],
    )
    .unwrap();
    assert!(tuning::validate_case_parameter_activity(
        &path,
        &["DEF_TUNING_CROP_PLANTING_DAY".into()],
        &["SinglePoint".into(), "LULC_IGBP".into()],
    )
    .is_err());

    std::fs::write(
        &path,
        "&nl_colm\n SITE_landtype=1\n DEF_Runoff_SCHEME=3\n/\n",
    )
    .unwrap();
    assert!(tuning::validate_case_parameter_activity(
        &path,
        &["DEF_TUNING_TOPMOD_DECAY".into()],
        &["SinglePoint".into(), "LULC_IGBP".into()],
    )
    .is_err());
    std::fs::write(
        &path,
        "&nl_colm\n SITE_landtype=1\n DEF_Runoff_SCHEME=0\n/\n",
    )
    .unwrap();
    tuning::validate_case_parameter_activity(
        &path,
        &["DEF_TUNING_TOPMOD_DECAY".into()],
        &["SinglePoint".into(), "LULC_IGBP".into()],
    )
    .unwrap();
    std::fs::write(&path, "&nl_colm\n SITE_landtype=17\n/\n").unwrap();
    assert!(tuning::validate_case_parameter_activity(
        &path,
        &["DEF_TUNING_SMPMIN".into()],
        &["SinglePoint".into(), "LULC_IGBP".into()],
    )
    .is_err());
    std::fs::write(
        &path,
        "&nl_colm\n SITE_landtype=17\n DEF_USE_Dynamic_Lake=.true.\n/\n",
    )
    .unwrap();
    tuning::validate_case_parameter_activity(
        &path,
        &["DEF_TUNING_SOIL_ICE_IMPEDANCE".into()],
        &["SinglePoint".into(), "LULC_IGBP".into()],
    )
    .unwrap();
    let _ = std::fs::remove_dir_all(root);
}

use super::*;

#[test]
fn methane_parameter_mapping_keeps_the_first_matching_alias() {
    assert_eq!(
        tracer_parameter_file("CH4:null; METHANE:later.nml", 0, &["CH4"]).unwrap(),
        None
    );
    assert_eq!(
        tracer_parameter_file("other.nml, standard_ch4.nml", 1, &["CL", "METHANE"]).unwrap(),
        Some("standard_ch4.nml".into())
    );
}

/// 没有甲烷时两项都不生成；参数文件里打开 `allowlakeprod` 就要生成湖泊土壤碳。
#[test]
fn requirements_follow_the_ch4_parameter_file() {
    let dir = std::env::temp_dir().join(format!("colm-methane-pre-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let namelist = dir.join("case.nml");
    let plain = colm_namelist::parse("&nl_colm\n DEF_USE_BGC = .true.\n/\n").unwrap();
    assert_eq!(
        requirements(&plain, &namelist).unwrap(),
        MethanePreprocessing {
            lake_soil_carbon: false,
            spatial_ph: false
        }
    );
    std::fs::write(
        dir.join("ch4.nml"),
        "&nl_colm_methane_parameter\n DEF_METHANE%allowlakeprod = .true.\n/\n",
    )
    .unwrap();
    let methane = colm_namelist::parse(
        "&nl_colm\n DEF_USE_BGC = .true.\n DEF_USE_TRACER = .true.\n DEF_TRACER_NUM = 1\n \
         DEF_TRACER_NAMES = 'CH4'\n DEF_TRACER_TYPES = 'gas'\n DEF_TRACER_PARAM_FILES = 'ch4.nml'\n/\n",
    )
    .unwrap();
    assert_eq!(
        requirements(&methane, &namelist).unwrap(),
        MethanePreprocessing {
            lake_soil_carbon: true,
            spatial_ph: false
        }
    );
    let _ = std::fs::remove_dir_all(&dir);
}

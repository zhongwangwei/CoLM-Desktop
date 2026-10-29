use super::*;

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
        "&nl_colm\n DEF_USE_BGC = .true.\n DEF_USE_METHANE = .true.\n \
         DEF_METHANE_PARAM_FILE = 'ch4.nml'\n/\n",
    )
    .unwrap();
    assert_eq!(
        requirements(&methane, &namelist).unwrap(),
        MethanePreprocessing {
            lake_soil_carbon: true,
            spatial_ph: false
        }
    );
    // 甲烷开了但没给参数文件：上游 `check_settings` 停机，这里也不静默跳过。
    let missing =
        colm_namelist::parse("&nl_colm\n DEF_USE_BGC = .true.\n DEF_USE_METHANE = .true.\n/\n")
            .unwrap();
    assert!(requirements(&missing, &namelist)
        .unwrap_err()
        .to_string()
        .contains("DEF_METHANE_PARAM_FILE"));
    let _ = std::fs::remove_dir_all(&dir);
}

use super::*;

#[test]
fn every_template_renders_and_parses_with_the_chosen_directory() {
    for name in forcing_datasets() {
        let text = render_forcing_namelist(name, "/data/forcing/x").unwrap();
        let document = colm_namelist::parse(&text).unwrap();
        assert_eq!(
            document.get("DEF_dir_forcing"),
            Some(&colm_namelist::Value::Str("/data/forcing/x/".into())),
            "{name}"
        );
        assert!(document.get("DEF_forcing%dataset").is_some(), "{name}");
    }
}

#[test]
fn unknown_datasets_and_bad_directories_are_refused() {
    assert!(render_forcing_namelist("POINT", "/x").is_err());
    assert!(render_forcing_namelist("JRA3Q", "  ").is_err());
    assert!(render_forcing_namelist("JRA3Q", "/a'b").is_err());
}

#[test]
fn dataset_identifiers_are_template_values_not_template_names() {
    let identifiers = forcing_dataset_identifiers();
    // POINT 不是标准模板（GUI 从 schema 并集补上），这里断言模板标识集合本身。
    for expected in ["CMFD", "ERA5", "CMIP6", "CLDAS", "JRA55", "WFDEI"] {
        assert!(
            identifiers.contains(&expected),
            "{expected} in {identifiers:?}"
        );
    }
    // 模板名不是合法 dataset 标识：引擎不认识 ERA5_LEddy / MPI-ESM1-2-HR_ssp585。
    for template_name in ["ERA5_LEddy", "MPI-ESM1-2-HR_ssp585"] {
        assert!(
            !identifiers.contains(&template_name),
            "{template_name} must not become an allowed dataset"
        );
    }
    assert_eq!(template_dataset_identifier("ERA5_LEddy"), Some("ERA5"));
    assert_eq!(
        template_dataset_identifier("MPI-ESM1-2-HR_ssp585"),
        Some("CMIP6")
    );
    assert_eq!(template_dataset_identifier("no-such-template"), None);
    // ERA5 与 ERA5_LEddy 共用一个标识，只出现一次。
    assert_eq!(
        identifiers
            .iter()
            .filter(|value| value.eq_ignore_ascii_case("ERA5"))
            .count(),
        1
    );
    assert!(identifiers.len() >= 20, "{identifiers:?}");
}

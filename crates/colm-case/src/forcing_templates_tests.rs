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

use super::*;

fn named(name: &'static str) -> impl Fn(&str) -> bool {
    move |key: &str| key.trim().eq_ignore_ascii_case(name)
}

#[test]
fn null_placeholders_keep_their_position() {
    let raw = "null,ch4.nml";
    assert_eq!(param_file_for(raw, 0, named("A")).unwrap(), None);
    assert_eq!(
        param_file_for(raw, 1, named("CH4")).unwrap().as_deref(),
        Some("ch4.nml")
    );
    assert_eq!(param_file_paths(raw).unwrap(), ["ch4.nml"]);
}

#[test]
fn semicolons_separate_like_commas() {
    let raw = "a.nml; CH4:ch4.nml";
    assert_eq!(param_file_paths(raw).unwrap(), ["a.nml", "ch4.nml"]);
    assert_eq!(
        param_file_for(raw, 1, named("CH4")).unwrap().as_deref(),
        Some("ch4.nml")
    );
}

#[test]
fn the_first_matching_entry_wins_and_null_means_none() {
    let raw = "CH4:null,first.nml,CH4:late.nml";
    assert_eq!(param_file_for(raw, 0, named("CH4")).unwrap(), None);
    assert_eq!(
        param_file_for(raw, 0, named("HDO")).unwrap().as_deref(),
        Some("first.nml")
    );
}

#[test]
fn a_windows_drive_is_a_positional_path() {
    let raw = r"C:\data\a.nml,B:/x.nml";
    assert_eq!(
        param_file_for(raw, 0, named("B")).unwrap().as_deref(),
        Some(r"C:\data\a.nml")
    );
    assert_eq!(
        param_file_for(raw, 1, named("B")).unwrap().as_deref(),
        Some("/x.nml")
    );
}

#[test]
fn empty_keys_or_paths_are_errors_and_null_lists_are_empty() {
    assert!(param_file_entries(":a.nml").is_err());
    assert!(param_file_entries("CH4: ").is_err());
    assert!(param_file_entries(" null ").unwrap().is_empty());
    assert!(param_file_entries("").unwrap().is_empty());
}

#[test]
fn rewriting_keeps_keys_nulls_and_positions() {
    let raw = "null; CH4:ch4.nml ,b.nml";
    let out = rewrite_param_files(raw, |path| Ok(format!("/new/{path}"))).unwrap();
    assert_eq!(out, "null,CH4:/new/ch4.nml,/new/b.nml");
}

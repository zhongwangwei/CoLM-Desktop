use super::*;

#[test]
fn only_the_three_real_kernel_stages_are_accepted() {
    assert_eq!(requested_run_stage(None).unwrap(), None);
    assert_eq!(
        requested_run_stage(Some("mksrfdata")).unwrap(),
        Some(Stage::MkSrfData)
    );
    assert_eq!(
        requested_run_stage(Some("mkinidata")).unwrap(),
        Some(Stage::MkIniData)
    );
    assert_eq!(
        requested_run_stage(Some("colm")).unwrap(),
        Some(Stage::Colm)
    );
    assert!(requested_run_stage(Some("all")).is_err());
}

#[test]
fn rust_preprocessors_are_the_default_and_fortran_is_an_explicit_fallback() {
    assert_eq!(requested_preprocessors(None).unwrap(), PreprocessorMode::Rust);
    assert_eq!(
        requested_preprocessors(Some("fortran")).unwrap(),
        PreprocessorMode::Fortran
    );
    assert!(requested_preprocessors(Some("other")).is_err());
}

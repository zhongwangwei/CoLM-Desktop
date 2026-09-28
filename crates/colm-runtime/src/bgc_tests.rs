use super::*;

#[test]
fn bgc_restart_paths_follow_the_upstream_naming() {
    let constant = Path::new("/r/const/at_restart_const_lc2005_w180_s90.nc");
    let (global, patch) = bgc_constant_paths(constant).unwrap();
    assert_eq!(global, Path::new("/r/const/at_restart_bgc_const_lc2005.nc"));
    assert_eq!(
        patch,
        Path::new("/r/const/at_restart_bgc_const_lc2005_w180_s90.nc")
    );
    let time = Path::new("/r/2010-001-00000/at_restart_2010-001-00000_lc2005_w180_s90.nc");
    assert_eq!(
        bgc_time_path(time).unwrap(),
        Path::new("/r/2010-001-00000/at_restart_bgc_2010-001-00000_lc2005_w180_s90.nc")
    );
}

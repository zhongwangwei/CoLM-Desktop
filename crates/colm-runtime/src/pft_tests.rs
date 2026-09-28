use super::*;

#[test]
fn pft_restart_path_inserts_the_pft_tag_once() {
    let path = Path::new("/r/2010-001-00000/at_restart_2010-001-00000_lc2005_w180_s90.nc");
    assert_eq!(
        pft_restart_path(path).unwrap(),
        Path::new("/r/2010-001-00000/at_restart_pft_2010-001-00000_lc2005_w180_s90.nc")
    );
    let constant = Path::new("/r/const/at_restart_const_lc2005_w180_s90.nc");
    assert_eq!(
        pft_restart_path(constant).unwrap(),
        Path::new("/r/const/at_restart_pft_const_lc2005_w180_s90.nc")
    );
}

#[test]
fn absorption_layout_round_trips_the_netcdf_order() {
    let matrix = [[1.0, 2.0], [3.0, 4.0]];
    // (rtyp, band) 行主序：rtyp=0 的两个波段在前。
    assert_eq!(flatten_absorption(matrix), [1.0, 3.0, 2.0, 4.0]);
}

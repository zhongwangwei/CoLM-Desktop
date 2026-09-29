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

/// 单 patch 拥有全部 PFT；多作物单点每个 patch 恰好一个 PFT；其它组合报错。
#[test]
fn patch_pft_ranges_follow_the_single_point_crop_layout() {
    assert_eq!(patch_pft_range(1, 3, 0).unwrap(), 0..3);
    assert_eq!(patch_pft_range(2, 2, 1).unwrap(), 1..2);
    assert!(patch_pft_range(2, 3, 0).is_err());
    assert!(patch_pft_range(2, 2, 2).is_err());
}

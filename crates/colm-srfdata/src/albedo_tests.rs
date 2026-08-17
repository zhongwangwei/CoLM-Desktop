use super::*;

#[test]
fn the_tables_are_the_twenty_entry_ones_colm_carries() {
    // mkinidata/MOD_SoilColorRefl.F90:42-55
    assert_eq!(SOIL_S_V_REFL.len(), 20);
    assert_eq!(SOIL_S_V_REFL[0], 0.26);
    assert_eq!(SOIL_S_V_REFL[19], 0.04);
    assert_eq!(SOIL_D_N_REFL[0], 0.63);
    assert_eq!(SOIL_D_N_REFL[19], 0.19);
}

#[test]
fn class_ten_is_the_set_the_old_script_hardcoded() {
    // 0.14/0.25/0.28/0.39。脚本把它写死了，而 90 个站点里只有 CN-Cng 是 10。
    let a = albedo(10, 4).expect("a land type");
    assert_eq!((a.s_v, a.d_v, a.s_n, a.d_n), (0.14, 0.25, 0.28, 0.39));
}

#[test]
fn a_different_colour_class_gives_different_albedo() {
    // 实测 90 站分布集中在 14-16，不是 10 —— 若两者相同，说明查表没生效。
    let a10 = albedo(10, 4).expect("land");
    let a15 = albedo(15, 4).expect("land");
    assert_ne!(a10.s_v, a15.s_v);
    assert_eq!(a15.s_v, 0.09);
}

#[test]
fn water_and_ice_have_no_soil_albedo() {
    // MOD_SingleSrfdata.F90:733-741：IGBP 17=水体、15=冰盖时保持 spval。
    assert!(albedo(10, 17).is_none());
    assert!(albedo(10, 15).is_none());
}

#[test]
fn a_colour_class_outside_one_to_twenty_is_rejected() {
    // MOD_SingleSrfdata.F90:737 的 (isc >= 1) .and. (isc <= 20)。
    // 越界时 CoLM 让四个值停在 spval，所以这里也不能凑一个出来。
    assert!(albedo(0, 4).is_none());
    assert!(albedo(21, 4).is_none());
}

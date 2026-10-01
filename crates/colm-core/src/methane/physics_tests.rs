use super::*;
use crate::methane::config::MethaneConfig;

#[test]
fn tridiagonal_solves_a_small_system() {
    // 2x2 + 固定首行：u0 = 1，-u0 + 2u1 - u2 = 0，-u1 + 2u2 = 1 → u = [1, 1, 1]。
    let a = [0.0, -1.0, -1.0];
    let b = [1.0, 2.0, 2.0];
    let c = [0.0, -1.0, 0.0];
    let r = [1.0, 0.0, 1.0];
    let mut u = [0.0; 3];
    tridiagonal(0, 2, 0, &a, &b, &c, &r, &mut u);
    for x in u {
        assert!((x - 1.0).abs() < 1e-15, "{u:?}");
    }
}

#[test]
fn henry_coefficients_follow_the_reference_temperature() {
    let t = [298.15; SOISNO];
    let k = henry_law(298.15, &t);
    // 298.15 K 时 `k_h = kh_theta`，无量纲系数是 `kh_theta * R * T`。
    assert!((k[0][0] - 1.4e-3 * 0.08206 * 298.15).abs() < 1e-15);
    assert_eq!(k[3], k[0]);
    // 低于 200 K 时按 200 K 算。
    let cold = henry_law(150.0, &[150.0; SOISNO]);
    assert_eq!(cold[0], henry_law(200.0, &[200.0; SOISNO])[0]);
}

#[test]
fn phases_split_without_ice_storage() {
    let mut dz = [0.0; SOISNO];
    let mut wliq = [0.0; SOISNO];
    let wice = [0.0; SOISNO];
    for k in 0..NL_SOIL {
        dz[sn(k as i32 + 1)] = 0.1;
        wliq[sn(k as i32 + 1)] = 20.0; // 0.2 m3/m3
    }
    let porsl = [0.5; NL_SOIL];
    let k_h = henry_law(288.0, &[288.0; SOISNO]);
    let p = split_phases(
        &dz,
        &wliq,
        &wice,
        &porsl,
        &[1.0; NL_SOIL],
        &[2.0; NL_SOIL],
        &k_h,
    );
    assert!((p.vol_aqu[0] - 0.2).abs() < 1e-15);
    assert!((p.vol_gas[0] - 0.3).abs() < 1e-15);
    // 总量守恒：f_aqu*C_aqu + f_gas*C_gas = C/porsl。
    let total = p.f_aqu[0] * p.conc_ch4_aqu[0] + p.f_gas[0] * p.conc_ch4_gas[0];
    assert!((total - 1.0).abs() < 1e-12, "{total}");
}

#[test]
fn ebullition_needs_a_gas_excess_below_the_water_table() {
    let m = MethaneConfig::default();
    let z: [f64; SOISNO] = std::array::from_fn(|i| if i >= 5 { 0.1 * (i - 4) as f64 } else { 0.0 });
    let zi = [0.0; SOISNO + 1];
    let t = [290.0; SOISNO];
    let none = ebul(
        &m,
        0,
        0,
        1,
        1.0,
        1800.0,
        &z,
        &zi,
        101325.0,
        0.0,
        0.0,
        &t,
        0.0,
        &[1.0; NL_SOIL],
        &[0.1; NL_SOIL],
    );
    assert!(none.iter().all(|&x| x == 0.0));
    let some = ebul(
        &m,
        0,
        0,
        1,
        1.0,
        1800.0,
        &z,
        &zi,
        101325.0,
        0.0,
        0.0,
        &t,
        0.0,
        &[1.0; NL_SOIL],
        &[100.0; NL_SOIL],
    );
    assert!(some[0] > 0.0);
}

#[test]
fn annual_update_turns_over_at_the_year_boundary() {
    let mut acc = AnnualAccumulators {
        annavg_agnpp: 0.0,
        annavg_bgnpp: 0.0,
        annavg_somhr: 0.0,
        annavg_finrw: 0.0,
        tempavg_agnpp: 0.0,
        tempavg_bgnpp: 0.0,
        annsum_counter: 0.0,
        tempavg_somhr: 0.0,
        tempavg_finrw: 0.0,
    };
    annual_update(&mut acc, [2010, 200, 0], 0.5, 1800.0, 1.0, 2.0, 3.0);
    assert!(acc.tempavg_somhr > 0.0 && acc.annavg_somhr == 0.0);
    // 步末落在新一年的第 0 秒：前一年那段先结转。
    annual_update(&mut acc, [2011, 1, 0], 0.5, 1800.0, 1.0, 2.0, 3.0);
    assert!(acc.annavg_somhr > 0.0);
    assert_eq!(acc.tempavg_somhr, 0.0);
}

#[test]
fn surface_water_fraction_grows_with_ponding() {
    let h = crate::methane::config::MethaneHydrology::default();
    assert_eq!(f_h2osfc(&h, 0.01, 0.0), 0.0);
    let small = f_h2osfc(&h, 0.01, 1.0);
    let large = f_h2osfc(&h, 0.01, 50.0);
    assert!(
        small > 0.0 && small < large && large <= 1.0,
        "{small} {large}"
    );
    assert_eq!(f_h2osfc(&h, 0.01, 1.0e6), 1.0);
}

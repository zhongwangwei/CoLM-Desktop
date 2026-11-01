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

/// #82：湖泊沉积物分解改成一阶形式后，`dt` 内分解的碳不能超过库存。
///
/// 原实现先按 `k*C*dz*Q10` 算产量、事后才把库存截到 0，`k*dt > 1` 时会凭空造碳。
/// 这里故意把 `lake_decomp_fact` 调到 `rate*dt` 远大于 1，断言产量仍 ≤ 库存。
#[test]
fn lake_decomposition_never_exceeds_the_sediment_stock() {
    use crate::methane::config::CATOMW;
    let mut m = MethaneConfig::default();
    m.allowlakeprod = true;
    m.lake_decomp_fact = 1.0;

    let mut dz = [0.0; SOISNO];
    let mut t = [0.0; SOISNO];
    let mut z = [0.0; SOISNO];
    let mut lake_soilc = [0.0; NL_SOIL];
    for k in 0..NL_SOIL {
        let j = k as i32 + 1;
        dz[sn(j)] = 0.1;
        z[sn(j)] = 0.1 * j as f64 - 0.05;
        t[sn(j)] = 288.15;
        lake_soilc[k] = 100.0; // gC/m3
    }
    let zeros = [0.0; NL_SOIL];
    let deltim = 1800.0;
    let out = prod(
        &m,
        &ProdInput {
            patchtype: 4,
            sat: 1,
            jwt: 0,
            finundated: 0.0,
            finundated_lag: 0.0,
            rr: 0.0,
            z_soisno: &z,
            dz_soisno: &dz,
            t_soisno: &t,
            conc_o2: &zeros,
            annavg_finrw: 0.0,
            crootfr: &zeros,
            somhr: 0.0,
            lithr: 0.0,
            hr_vr: &zeros,
            o_scalar: &zeros,
            fphr: &zeros,
            pot_f_nit_vr: &zeros,
            ph: 6.2,
            layer_sat_lag: &zeros,
            lake_soilc: &lake_soilc,
            microbial_prod_potential: &zeros,
            biome_f_methane: None,
            deltim,
        },
    );
    let stock_mol: f64 = lake_soilc.iter().map(|c| c * 0.1 / CATOMW).sum();
    let decomposed_mol: f64 = (0..NL_SOIL)
        .map(|k| (out.methane_prod_depth[k] + out.co2_decomp_depth[k]) * dz[sn(k as i32 + 1)])
        .sum::<f64>()
        * deltim;
    assert!(
        decomposed_mol <= stock_mol * (1.0 + 1e-12),
        "decomposed {decomposed_mol} exceeds stock {stock_mol}"
    );
    // 缺省参数下应退化成线性产量（二阶项 ~1e-14）。
    let mut m = MethaneConfig::default();
    m.allowlakeprod = true;
    let out = prod(
        &m,
        &ProdInput {
            patchtype: 4,
            sat: 1,
            jwt: 0,
            finundated: 0.0,
            finundated_lag: 0.0,
            rr: 0.0,
            z_soisno: &z,
            dz_soisno: &dz,
            t_soisno: &t,
            conc_o2: &zeros,
            annavg_finrw: 0.0,
            crootfr: &zeros,
            somhr: 0.0,
            lithr: 0.0,
            hr_vr: &zeros,
            o_scalar: &zeros,
            fphr: &zeros,
            pot_f_nit_vr: &zeros,
            ph: 6.2,
            layer_sat_lag: &zeros,
            lake_soilc: &lake_soilc,
            microbial_prod_potential: &zeros,
            biome_f_methane: None,
            deltim,
        },
    );
    let linear_mol: f64 = (0..NL_SOIL)
        .map(|k| {
            m.lake_decomp_fact
                * m.cnscalefactor
                * lake_soilc[k]
                * 0.1
                * (m.q10methane * 1.5).powf((288.15 - m.q10lakebase) / 10.0)
                * 1.0
                / CATOMW
                * deltim
        })
        .sum();
    let exponential_mol: f64 = (0..NL_SOIL)
        .map(|k| (out.methane_prod_depth[k] + out.co2_decomp_depth[k]) * 0.1)
        .sum::<f64>()
        * deltim;
    let rel = (exponential_mol - linear_mol).abs() / linear_mol;
    assert!(
        rel < 1e-6,
        "default exponential differs from linear by {rel}"
    );
}

/// #81：甲烷的雪层几何必须用宿主真实的 `dz_sno`，不能再按各层质量把 `snowdp`
/// 摊成均匀密度。这里两层雪给 1 cm（冰壳）+ 19 cm（松雪），断言几何原样保留。
#[test]
fn methane_snow_geometry_uses_the_host_layer_thickness() {
    let mut site = crate::methane::driver::MethaneSite {
        patchtype: 0,
        patchclass: 1,
        dlat: 0.0,
        slpratio: 0.0,
        rootfr: [0.1; NL_SOIL],
        z_soi: [0.0; NL_SOIL],
        dz_soi: [0.1; NL_SOIL],
        zi_soi: [0.0; NL_SOIL],
        bsw: [0.4; NL_SOIL],
        porsl: [0.4; NL_SOIL],
        organic_max: 0.0,
        wetwatmax: 0.0,
        wetland_fraction: 1.0,
        giems: None,
    };
    for k in 0..NL_SOIL {
        let j = k as i32 + 1;
        site.z_soi[k] = sn(j) as f64; // 只用于填满数组，几何测试不读
        site.zi_soi[k] = sn(j) as f64;
    }
    let mut wliq = [0.0; SOISNO];
    let mut wice = [0.0; SOISNO];
    // 两层雪：j=-1（slot 3）1 cm 冰壳、j=0（slot 4）19 cm 松雪；质量相同。
    let (lower, upper) = (sn(-1), sn(0));
    wice[lower] = 9.0;
    wliq[lower] = 1.0;
    wice[upper] = 5.0;
    wliq[upper] = 5.0;
    let mut thickness = [0.0; crate::material::MAX_SNOW_LAYERS];
    thickness[lower] = 0.01;
    thickness[upper] = 0.19;
    let (snl, _z, dz, _zi) =
        crate::methane::driver::soisno_geometry(&site, &wliq, &wice, &thickness);
    assert_eq!(snl, -2);
    assert!((dz[lower] - 0.01).abs() < 1e-15, "{}", dz[lower]);
    assert!((dz[upper] - 0.19).abs() < 1e-15, "{}", dz[upper]);
    // 若还按旧实现摊厚度，两层都会是 snowdp/2 = 0.1 m。
    assert!((dz[lower] - 0.1).abs() > 1e-6);
    // 用了真实层厚后，同样的质量下两层的空气比例差一个量级：
    let airfrac = |k: usize| 1.0 - wice[k] / DENICE / dz[k] - wliq[k] / DENH2O / dz[k];
    assert!(airfrac(upper) > 0.8, "{}", airfrac(upper));
    assert!(airfrac(lower) < 0.1, "{}", airfrac(lower));
}

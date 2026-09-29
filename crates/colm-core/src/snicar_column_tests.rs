use super::*;

fn state() -> SnicarColumnState {
    SnicarColumnState {
        grain_radius_um: [54.526; 5],
        aerosol_mass_kg_m2: [[0.0; SNICAR_AEROSOL_SPECIES]; 5],
        layer_absorption: [[[0.0; 6]; 2]; 2],
        refreezing_kg_m2_s: [0.0; 5],
    }
}

fn forcing() -> ShortwaveForcing {
    ShortwaveForcing {
        direct_visible_w_m2: 200.0,
        direct_near_infrared_w_m2: 150.0,
        diffuse_visible_w_m2: 60.0,
        diffuse_near_infrared_w_m2: 40.0,
    }
}

#[test]
fn net_solar_rescales_layers_and_moves_the_soil_row_into_soil_absorption() {
    // `MOD_NetSolar.F90:236-275`：各 (band, rtyp) 按 `ssno` 重标，土壤那一格并进 `sabg_soil`。
    let mut layers = [[[0.0; 6]; 2]; 2];
    for kinds in &mut layers {
        for slots in kinds.iter_mut() {
            slots[3] = 0.2;
            slots[4] = 0.1;
            slots[5] = 0.1;
        }
    }
    let ssno = [[0.8, 0.6], [0.4, 0.2]];
    let (mut soil, mut snow) = (10.0, 100.0);
    let absorbed = snicar_net_solar(&mut layers, ssno, forcing(), 0.5, &mut soil, &mut snow);
    for band in 0..2 {
        for kind in 0..2 {
            let total: f64 = layers[band][kind].iter().sum();
            assert!((total - ssno[band][kind]).abs() < 1.0e-15);
        }
    }
    // 雪层的总吸收 + 并进土壤的那份 = 原 `sabg_snow`，`absorbed[5]` 就是新的 `sabg_soil`。
    assert_eq!(absorbed[5], soil);
    assert!((snow + (soil - 10.0) - 100.0).abs() < 1.0e-12);
    assert_eq!(absorbed[0], 0.0);
}

#[test]
fn net_solar_puts_everything_in_the_soil_row_when_no_layer_absorbs() {
    let mut layers = [[[0.0; 6]; 2]; 2];
    let ssno = [[0.5, 0.4], [0.3, 0.2]];
    let (mut soil, mut snow) = (0.0, 0.0);
    snicar_net_solar(&mut layers, ssno, forcing(), 1.0, &mut soil, &mut snow);
    assert_eq!(layers[0][0][5], 0.5);
    assert_eq!(layers[1][1][5], 0.2);
}

#[test]
fn snow_water_aerosols_follow_meltwater_and_deposition_lands_on_top() {
    // 两层雪：上层出流 2 mm 把气溶胶按冲刷系数带到下层；沉降加在最上层。
    let mut column = state();
    column.aerosol_mass_kg_m2[3] = [1.0e-6; 8];
    let liquid = [0.0, 0.0, 0.0, 1.0, 1.0];
    let ice = [0.0, 0.0, 0.0, 9.0, 9.0];
    let mut deposition = [0.0; AEROSOL_DEPOSITION_FIELDS];
    deposition[1] = 1.0e-12;
    snicar_snow_water_aerosols(
        &mut column,
        2,
        &liquid,
        &ice,
        &[2.0, 0.0],
        &deposition,
        1800.0,
    )
    .unwrap();
    let moved_bcphi = 2.0 * 0.20 * (1.0e-6 / 10.0);
    assert!((column.aerosol_mass_kg_m2[4][0] - moved_bcphi).abs() < 1.0e-22);
    assert!((column.aerosol_mass_kg_m2[3][0] - (1.0e-6 - moved_bcphi)).abs() < 1.0e-22);
    // BC 疏水 = forc_aer(2)。
    let moved_bcpho = 2.0 * 0.03 * (1.0e-6 / 10.0);
    assert!(
        (column.aerosol_mass_kg_m2[3][1] - (1.0e-6 - moved_bcpho + 1.0e-12 * 1800.0)).abs()
            < 1.0e-20
    );
    assert!(
        snicar_snow_water_aerosols(&mut column, 2, &liquid, &ice, &[1.0], &deposition, 1.0)
            .is_err()
    );
}

#[test]
fn refreezing_rate_only_counts_freezing_snow_layers() {
    let mut column = state();
    snow_refreezing_rate(
        &mut column,
        2,
        &[5.0, 5.0, 1.0],
        &[5.5, 6.0, 1.0],
        &[2, 1, 0],
        100.0,
    );
    assert_eq!(column.refreezing_kg_m2_s[3], 0.005);
    assert_eq!(column.refreezing_kg_m2_s[4], 0.0);
}

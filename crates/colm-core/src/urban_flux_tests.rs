use super::*;

#[test]
fn bare_flux_matches_current_fortran_with_partial_roofs_and_dew() {
    let fixture = include_str!("../../../oracle/fixtures/urban_bare_flux_reference.csv");
    for (case, line) in fixture
        .lines()
        .filter(|line| !line.starts_with('#'))
        .skip(1)
        .enumerate()
    {
        let expected: Vec<f64> = line
            .split(',')
            .map(|value| value.parse().unwrap())
            .collect();
        let out = urban_bare_flux(input(case)).unwrap();
        let actual = common_values(out);
        assert_eq!(actual.len(), expected.len());
        for (component, (actual, expected)) in actual.into_iter().zip(expected).enumerate() {
            assert!(
                (actual - expected).abs() < 5.0e-12,
                "case {case}, component {component}: {actual} != {expected}"
            );
        }
    }
}

fn input(case: usize) -> UrbanFluxInput {
    let froof = [0.12, 0.35, 0.6, 0.35][case];
    let dew = case == 3;
    UrbanFluxInput {
        time_step_seconds: 1800.0,
        roof_has_snow_layers: false,
        impervious_has_snow_layers: false,
        wind_height_m: 30.0,
        temperature_height_m: 2.0,
        humidity_height_m: 2.0,
        absolute_heights: false,
        eastward_wind_m_s: 3.2,
        northward_wind_m_s: 0.7,
        thm: if dew { 302.0 } else { 295.0 },
        th: if dew { 301.0 } else { 294.0 },
        thv: if dew { 304.0 } else { 295.2 },
        qm: if dew { 0.016 } else { 0.007 },
        psrf: 101325.0,
        rhoair: 1.2,
        fhac: 30.0,
        fwst: 9.0,
        fach: 4.0,
        vehc: 6.0,
        meta: 3.0,
        hroof: 12.0,
        hlr: 0.6,
        fcover: [
            froof,
            0.1,
            0.08,
            (1.0 - froof) * 0.6,
            (1.0 - froof) * 0.4,
            0.0,
        ],
        z0h_g: 0.01,
        obug: -100.0,
        ustarg: 0.2,
        zlnd: 0.03,
        zsno: 0.002,
        fsno_roof: 0.0,
        fsno_gimp: 0.0,
        fsno_gper: 0.0,
        wliq_roof: 0.2,
        wliq_gimp: 0.1,
        wice_roof: 0.0,
        wice_gimp: 0.0,
        htvp_roof: HVAP,
        htvp_gimp: HVAP,
        htvp_gper: HVAP,
        troof: 301.0,
        twsun: 303.0,
        twsha: 299.0,
        tgimp: 300.0,
        tgper: 297.0,
        qroof: 0.014,
        qgimp: 0.013,
        qgper: 0.011,
        dqroofdt: 0.0007,
        dqgimpdt: 0.0007,
        dqgperdt: 0.0006,
        rss: 100.0,
        surface_layer_scheme: SurfaceLayerScheme::Standard,
    }
}

fn common_values(out: UrbanFluxOutput) -> [f64; 32] {
    [
        out.taux,
        out.tauy,
        out.fsenroof,
        out.fsenwsun,
        out.fsenwsha,
        out.fsengimp,
        out.fsengper,
        out.fevproof,
        out.fevpgimp,
        out.fevpgper,
        out.croofs,
        out.cwsuns,
        out.cwshas,
        out.cgrnds,
        out.croofl,
        out.cgimpl,
        out.cgperl,
        out.croof,
        out.cgimp,
        out.cgper,
        out.tref,
        out.qref,
        out.z0m,
        out.zol,
        out.rib,
        out.ustar,
        out.qstar,
        out.tstar,
        out.fm,
        out.fh,
        out.fq,
        out.tafu,
    ]
}

#[test]
fn vegetated_flux_matches_full_vendor_procedure_with_fixed_radiation_boundary() {
    let references = include_str!("../../../oracle/fixtures/urban_vegetated_flux_reference.csv");
    let radiation = include_str!("../../../oracle/fixtures/urban_vegetated_radiation_inputs.csv");
    let boundaries: Vec<Vec<f64>> = radiation
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(|line| {
            line.split(',')
                .map(|value| value.parse().unwrap())
                .collect()
        })
        .collect();
    let mode = if colm_numeric::FUSED {
        "fused"
    } else {
        "unfused"
    };
    let mut covered = 0;
    let mut max_absolute: f64 = 0.0;
    let mut exact = 0;
    for line in references
        .lines()
        .filter(|line| line.starts_with(&format!("{mode},")))
    {
        let mut fields = line.split(',');
        assert_eq!(fields.next(), Some(mode));
        let case: usize = fields.next().unwrap().parse().unwrap();
        let pinned: Vec<f64> = fields.map(|value| value.parse().unwrap()).collect();
        covered += 1;
        let rad = &boundaries[case];
        let mut base = input(case);
        base.fcover = rad[51..57].try_into().unwrap();
        if case == 3 {
            base.qm = 0.02;
            base.thm = 298.0;
            base.th = 297.0;
            base.thv = 300.0;
        }
        let (lai, htop, ldew, stress) = match case {
            1 => (3.0, 8.0, 0.05, 0.5),
            2 => (1.5, 4.0, 0.1, 1.0),
            3 => (3.0, 8.0, 0.05, 0.8),
            _ => (2.0, 6.0, 0.0, 0.8),
        };
        let mut state = UrbanTreeState {
            tl: 299.0,
            ldew,
            ldew_rain: ldew,
            ldew_snow: 0.0,
            fwet_snow: 0.0,
        };
        let tree = UrbanTreeInput {
            frl: 350.0,
            po2m: 21200.0,
            pco2m: 40.0,
            par: 200.0,
            sabv: 120.0,
            rstfac: stress,
            ewall: 0.95,
            egimp: 0.96,
            egper: 0.97,
            ev: rad[50],
            htop,
            hbot: 2.0,
            lai,
            sai: 0.5,
            sqrtdi: 5.0,
            extkd: 0.5,
            dewmx: 0.1,
            etrc: 0.0002,
            trsmx0: 0.0002,
            // UrbanVegFlux omits c3c4; match its effcon fallback in both pathways.
            biochemistry: crate::LeafBiochemistry {
                quantum_efficiency: if case == 2 { 0.05 } else { 0.08 },
                maximum_carboxylation_25c_mol_m2_s: 60e-6,
                c3c4: if case == 2 { 0 } else { 1 },
                respiration_fraction_override: None,
                low_temperature_slope: 0.2,
                low_temperature_half_k: 288.16,
                high_temperature_slope: 0.3,
                high_temperature_half_k: 313.16,
                respiration_temperature_slope: 1.3,
                respiration_temperature_half_k: 328.16,
                optimum_temperature_k: 298.16,
                medlyn_g1: 4.0,
                medlyn_g0: 0.01,
                ball_berry_slope: 9.0,
                ball_berry_intercept: 0.01,
            },
            stomata_options: crate::StomataOptions::default(),
            wue_lambda: 2000.0,
            vegetation_snow: case == 2,
            ainv: std::array::from_fn(|row| std::array::from_fn(|col| rad[row * 5 + col])),
            b: rad[25..30].try_into().unwrap(),
            b1: rad[30..35].try_into().unwrap(),
            dbdt: rad[35..40].try_into().unwrap(),
            sky_vf: rad[40..45].try_into().unwrap(),
            veg_vf: rad[45..50].try_into().unwrap(),
            lwsun: 0.0,
            lwsha: 0.0,
            lgimp: 0.0,
            lgper: 0.0,
            lveg: 0.0,
        };
        let (common, leaf) = urban_vegetated_flux(base, tree, &mut state).unwrap();
        let mut actual = common_values(common).to_vec();
        actual.extend([
            leaf.fsenl,
            leaf.fevpl,
            leaf.etr,
            leaf.rst,
            leaf.assim,
            leaf.respc,
            leaf.lwsun,
            leaf.lwsha,
            leaf.lgimp,
            leaf.lgper,
            leaf.lveg,
            leaf.lout,
            leaf.etr_deficit,
            leaf.dheatl,
            state.tl,
            state.ldew,
            state.ldew_rain,
            state.ldew_snow,
        ]);
        assert_eq!(actual.len(), pinned.len());
        for (component, (actual, pinned)) in actual.into_iter().zip(pinned).enumerate() {
            max_absolute = max_absolute.max((actual - pinned).abs());
            exact += usize::from(actual.to_bits() == pinned.to_bits());
            assert!(
                (actual - pinned).abs() < 5e-12,
                "case {case}, component {component}: {actual:.17e} != {pinned:.17e}"
            );
        }
    }
    assert_eq!(covered, 4);
    eprintln!("UrbanVegFlux {mode}: {exact}/200 bitwise equal, max absolute {max_absolute:.17e}");
}

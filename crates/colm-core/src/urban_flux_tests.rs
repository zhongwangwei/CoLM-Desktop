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

// This exact captured reference is GNU/Linux x86_64 without contraction. Other
// targets/contraction modes retain the full-procedure bounds exercised above.
#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
#[test]
fn captured_urban_dew_case_matches_fortran_humidity_and_stomatal_diagnostics() {
    if colm_numeric::FUSED {
        return;
    }
    let fixture = include_str!("../../../oracle/fixtures/urban_au_preston_first_step.csv");
    let (values, input, flux, leaf, _) = replay_captured_urban_case(fixture);
    let v = |name: &str| values[name];
    let normalized_qref = flux.qref
        / crate::saturation_specific_humidity(flux.tref, input.psrf)
            .unwrap()
            .specific_humidity;
    for (name, actual) in [
        ("qstar", flux.qstar),
        ("rst", leaf.rst),
        ("qref", normalized_qref),
    ] {
        assert_eq!(
            actual.to_bits(),
            v(&format!("expected_{name}")).to_bits(),
            "{name}"
        );
    }
}

// Both captured cases use the same immutable input schema and public solver.
#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
fn replay_captured_urban_case(
    fixture: &str,
) -> (
    std::collections::BTreeMap<&str, f64>,
    UrbanFluxInput,
    UrbanFluxOutput,
    UrbanTreeOutput,
    UrbanTreeState,
) {
    let values: std::collections::BTreeMap<_, _> = fixture
        .lines()
        .filter(|line| !line.starts_with('#'))
        .skip(1)
        .map(|line| {
            let (name, bits) = line.split_once(',').unwrap();
            (name, f64::from_bits(u64::from_str_radix(bits, 16).unwrap()))
        })
        .collect();
    let v = |name: &str| values[name];
    assert_eq!(v("use_wue"), 0.0);
    assert_eq!(v("use_medlyn"), 0.0);
    assert_eq!(v("respcp_present"), 0.0);
    assert_eq!(v("surface_standard"), 1.0);
    let input = UrbanFluxInput {
        time_step_seconds: v("deltim"),
        roof_has_snow_layers: v("roof_snow") != 0.0,
        impervious_has_snow_layers: v("impervious_snow") != 0.0,
        wind_height_m: v("hu"),
        temperature_height_m: v("ht"),
        humidity_height_m: v("hq"),
        absolute_heights: v("absolute_heights") != 0.0,
        eastward_wind_m_s: v("us"),
        northward_wind_m_s: v("vs"),
        thm: v("thm"),
        th: v("th"),
        thv: v("thv"),
        qm: v("qm"),
        psrf: v("psrf"),
        rhoair: v("rhoair"),
        fhac: v("fhac"),
        fwst: v("fwst"),
        fach: v("fach"),
        vehc: v("vehc"),
        meta: v("meta"),
        hroof: v("hroof"),
        hlr: v("hlr"),
        fcover: std::array::from_fn(|i| v(&format!("fcover_{i}"))),
        z0h_g: v("z0h_g"),
        obug: v("obug"),
        ustarg: v("ustarg"),
        zlnd: v("zlnd"),
        zsno: v("zsno"),
        fsno_roof: v("fsno_roof"),
        fsno_gimp: v("fsno_gimp"),
        fsno_gper: v("fsno_gper"),
        wliq_roof: v("wliq_roofsno"),
        wliq_gimp: v("wliq_gimpsno"),
        wice_roof: v("wice_roofsno"),
        wice_gimp: v("wice_gimpsno"),
        htvp_roof: v("htvp_roof"),
        htvp_gimp: v("htvp_gimp"),
        htvp_gper: v("htvp_gper"),
        troof: v("troof"),
        twsun: v("twsun"),
        twsha: v("twsha"),
        tgimp: v("tgimp"),
        tgper: v("tgper"),
        qroof: v("qroof"),
        qgimp: v("qgimp"),
        qgper: v("qgper"),
        dqroofdt: v("dqroofdt"),
        dqgimpdt: v("dqgimpdt"),
        dqgperdt: v("dqgperdt"),
        rss: v("rss"),
        surface_layer_scheme: crate::SurfaceLayerScheme::Standard,
    };
    let tree = UrbanTreeInput {
        frl: v("frl"),
        po2m: v("po2m"),
        pco2m: v("pco2m"),
        par: v("par"),
        sabv: v("sabv"),
        rstfac: v("rstfac"),
        ewall: v("ewall"),
        egimp: v("egimp"),
        egper: v("egper"),
        ev: v("ev"),
        htop: v("htop"),
        hbot: v("hbot"),
        lai: v("lai"),
        sai: v("sai"),
        sqrtdi: v("sqrtdi"),
        extkd: v("extkd"),
        dewmx: v("dewmx"),
        etrc: v("etrc"),
        trsmx0: v("trsmx0"),
        biochemistry: crate::LeafBiochemistry {
            quantum_efficiency: v("effcon"),
            maximum_carboxylation_25c_mol_m2_s: v("vmax25"),
            low_temperature_slope: v("slti"),
            low_temperature_half_k: v("hlti"),
            high_temperature_slope: v("shti"),
            high_temperature_half_k: v("hhti"),
            respiration_temperature_slope: v("trda"),
            respiration_temperature_half_k: v("trdm"),
            optimum_temperature_k: v("trop"),
            medlyn_g1: v("g1"),
            medlyn_g0: v("g0"),
            ball_berry_slope: v("gradm"),
            ball_berry_intercept: v("binter"),
            c3c4: v("c3c4") as i32,
            respiration_fraction_override: None,
        },
        stomata_options: crate::StomataOptions::default(),
        wue_lambda: v("lambda_wue"),
        vegetation_snow: v("vegetation_snow") != 0.0,
        ainv: std::array::from_fn(|i| std::array::from_fn(|j| v(&format!("ainv_{i}_{j}")))),
        b: std::array::from_fn(|i| v(&format!("b_{i}"))),
        b1: std::array::from_fn(|i| v(&format!("b1_{i}"))),
        dbdt: std::array::from_fn(|i| v(&format!("dbdt_{i}"))),
        sky_vf: std::array::from_fn(|i| v(&format!("sky_vf_{i}"))),
        veg_vf: std::array::from_fn(|i| v(&format!("veg_vf_{i}"))),
        lwsun: v("lwsun"),
        lwsha: v("lwsha"),
        lgimp: v("lgimp"),
        lgper: v("lgper"),
        lveg: v("lveg"),
    };
    let mut state = UrbanTreeState {
        tl: v("tl"),
        ldew: v("ldew"),
        ldew_rain: v("ldew_rain"),
        ldew_snow: v("ldew_snow"),
        fwet_snow: 0.0,
    };
    let (flux, leaf) = urban_vegetated_flux(input, tree, &mut state).unwrap();
    (values, input, flux, leaf, state)
}

// Source-ordered temperature sums must preserve the next real canopy step.
#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
#[test]
fn captured_urban_second_step_matches_all_fortran_flux_and_state_outputs() {
    assert_captured_urban_outputs(include_str!(
        "../../../oracle/fixtures/urban_au_preston_second_step.csv"
    ));
}

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
#[test]
fn captured_urban_third_step_matches_all_fortran_flux_and_state_outputs() {
    assert_captured_urban_outputs(include_str!(
        "../../../oracle/fixtures/urban_au_preston_third_step.csv"
    ));
}

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
fn assert_captured_urban_outputs(fixture: &str) {
    if colm_numeric::FUSED {
        return;
    }
    let (values, _, flux, leaf, state) = replay_captured_urban_case(fixture);
    for (name, actual) in [
        ("taux", flux.taux),
        ("tauy", flux.tauy),
        ("fsenroof", flux.fsenroof),
        ("fsenwsun", flux.fsenwsun),
        ("fsenwsha", flux.fsenwsha),
        ("fsengimp", flux.fsengimp),
        ("fsengper", flux.fsengper),
        ("fevproof", flux.fevproof),
        ("fevpgimp", flux.fevpgimp),
        ("fevpgper", flux.fevpgper),
        ("croofs", flux.croofs),
        ("cwsuns", flux.cwsuns),
        ("cwshas", flux.cwshas),
        ("cgrnds", flux.cgrnds),
        ("croofl", flux.croofl),
        ("cgimpl", flux.cgimpl),
        ("cgperl", flux.cgperl),
        ("croof", flux.croof),
        ("cgimp", flux.cgimp),
        ("cgper", flux.cgper),
        ("tref", flux.tref),
        ("qref", flux.qref),
        ("z0m", flux.z0m),
        ("zol", flux.zol),
        ("rib", flux.rib),
        ("ustar", flux.ustar),
        ("qstar", flux.qstar),
        ("tstar", flux.tstar),
        ("fm", flux.fm),
        ("fh", flux.fh),
        ("fq", flux.fq),
        ("tafu", flux.tafu),
        ("fsenl", leaf.fsenl),
        ("fevpl", leaf.fevpl),
        ("etr", leaf.etr),
        ("rst", leaf.rst),
        ("assim", leaf.assim),
        ("respc", leaf.respc),
        ("lwsun", leaf.lwsun),
        ("lwsha", leaf.lwsha),
        ("lgimp", leaf.lgimp),
        ("lgper", leaf.lgper),
        ("lveg", leaf.lveg),
        ("lout", leaf.lout),
        ("etr_deficit", leaf.etr_deficit),
        ("dheatl", leaf.dheatl),
        ("state_tl", state.tl),
        ("state_ldew", state.ldew),
        ("state_ldew_rain", state.ldew_rain),
        ("state_ldew_snow", state.ldew_snow),
        ("state_fwet_snow", state.fwet_snow),
    ] {
        assert_eq!(
            actual.to_bits(),
            values[&format!("expected_{name}")[..]].to_bits(),
            "{name}"
        );
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
#[test]
fn captured_urban_slope_steps_match_all_fortran_flux_and_state_outputs() {
    let fixtures = include_str!("../../../oracle/fixtures/urban_au_preston_slope_steps.csv");
    let mut covered = 0;
    for fixture in fixtures.split("\n\n").filter(|s| !s.trim().is_empty()) {
        assert_captured_urban_outputs(fixture);
        covered += 1;
    }
    assert_eq!(covered, 8);
}

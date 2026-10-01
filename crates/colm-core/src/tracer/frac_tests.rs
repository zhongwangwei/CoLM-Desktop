use super::*;
use crate::tracer::{ReactionMode, StateOwner, TracerFamily};

fn isotope(name: &str, ref_ratio: f64) -> TracerDescriptor {
    TracerDescriptor {
        name: name.to_owned(),
        category: "isotope".to_owned(),
        family: TracerFamily::Isotope,
        state_owner: StateOwner::GenericWater,
        reaction_mode: ReactionMode::None,
        unit_kind: "ratio".to_owned(),
        mol_weight: 20.0,
        ref_ratio,
        init_delta: 0.0,
        init_conc: 0.0,
        precip_default_conc: 0.0,
        vapor_default_conc: 0.0,
        max_dissolved_conc: f64::MAX,
        reactive_decay_rate: 0.0,
        charge: 0,
    }
}

#[test]
fn species_matching_follows_name_patterns_then_ref_ratio() {
    assert_eq!(
        IsotopeSpecies::find(&isotope("H2_18O", 1.0)),
        Some(IsotopeSpecies::O18)
    );
    assert_eq!(
        IsotopeSpecies::find(&isotope("HDO", 1.0)),
        Some(IsotopeSpecies::Hdo)
    );
    assert_eq!(
        IsotopeSpecies::find(&isotope("deuterium", 1.0)),
        Some(IsotopeSpecies::Hdo)
    );
    // `=h2` 是全等匹配：`h2` 命中，`h2x` 不命中，但 `ref_ratio` 接近 RSMOW_D 时仍按比值认出。
    assert_eq!(
        IsotopeSpecies::find(&isotope("h2", 1.0)),
        Some(IsotopeSpecies::Hdo)
    );
    assert_eq!(IsotopeSpecies::find(&isotope("h2x", 1.0)), None);
    assert_eq!(
        IsotopeSpecies::find(&isotope("h2x", 1.6e-4)),
        Some(IsotopeSpecies::Hdo)
    );
    assert_eq!(
        IsotopeSpecies::find(&isotope("iso", 2.0e-3)),
        Some(IsotopeSpecies::O18)
    );
}

#[test]
fn fractionation_needs_switch_isotope_and_registration() {
    let physics = TracerPhysics {
        fractionation: true,
        ..Default::default()
    };
    assert!(physics.fractionation_active(&isotope("O18", RSMOW_18O)));
    assert!(!physics.fractionation_active(&isotope("unknown", 1.0)));
    assert!(!TracerPhysics::default().fractionation_active(&isotope("O18", RSMOW_18O)));
}

#[test]
fn equilibrium_alphas_have_textbook_magnitudes_at_20c() {
    // Majoube (1971)：20 °C 时 α(18O) ≈ 1.0098、α(D) ≈ 1.0850。
    let t = 293.15;
    assert!((IsotopeSpecies::O18.alpha_liq_vap(t) - 1.0098).abs() < 2e-4);
    assert!((IsotopeSpecies::Hdo.alpha_liq_vap(t) - 1.0850).abs() < 2e-3);
}

#[test]
fn ice_deposition_alpha_is_continuous_at_both_joins() {
    let o18 = IsotopeSpecies::O18;
    let alpha = |t: f64| {
        ice_deposition_alpha(
            t,
            0.003,
            1.0285,
            o18.alpha_ice_vap(t),
            o18.alpha_ice_vap(273.15),
            o18.alpha_ice_vap(253.15),
        )
    };
    assert!((alpha(273.15 - 1e-9) - alpha(273.15)).abs() < 1e-9);
    assert!((alpha(253.15 + 1e-9) - alpha(253.15)).abs() < 1e-9);
    // 过饱和下有效 α 低于平衡 α。
    assert!(alpha(240.0) < o18.alpha_ice_vap(240.0));
}

#[test]
fn craig_gordon_core_limits() {
    // h = 0：R_E = R_s/(α_eq α_k)。
    let r = craig_gordon_ratio_core(1.0, 0.9, 1.01, 1.02, 0.0, 0.99);
    assert_eq!(r, (1.0 / 1.01) / 1.02);
    // α 非正时原样返回源比值。
    assert_eq!(craig_gordon_ratio_core(1.0, 0.9, 0.0, 1.02, 0.5, 0.99), 1.0);
    // 结果限幅在 ±10·|R_s/α_eq|。
    let capped = craig_gordon_ratio_core(1.0, 0.0, 1.0, 1.0e-6, 0.5, 0.99);
    assert_eq!(capped, 10.0);
}

#[test]
fn rayleigh_freezing_reduces_to_proportional_without_fractionation() {
    let physics = TracerPhysics::default();
    let tracer = isotope("O18", RSMOW_18O);
    let loss = physics.rayleigh_freezing_loss(&tracer, 2.0, 10.0, 3.0, 270.0);
    assert_eq!(loss, 3.0 * (2.0 / 10.0));
    // 全冻：整池转走。
    assert_eq!(
        physics.rayleigh_freezing_loss(&tracer, 2.0, 10.0, 10.0, 270.0),
        2.0
    );
}

#[test]
fn diffusive_transfer_never_overshoots_equal_ratios() {
    let moved = soil_diffusive_transfer(2.0, 1.0, 5.0, 5.0, 1.0, 1.0, 0.01, 0.01, 1.0e6);
    let equalise = 1.0 / (1.0 / 5.0 + 1.0 / 5.0);
    assert_eq!(moved, equalise);
}

#[test]
fn equilibration_exchange_never_empties_more_than_the_pool() {
    let gain = equilibration_exchange(1.0, 1.0, 0.0, 1.01, 1.0);
    assert_eq!(gain, -1.0);
}

#[test]
fn nss_returns_source_ratio_when_inactive() {
    let physics = TracerPhysics::default();
    let tracer = isotope("O18", RSMOW_18O);
    let out = physics.transpiration_nss_ratio(
        &tracer,
        &NssInput {
            source_ratio: RSMOW_18O,
            vapor_ratio: RSMOW_18O * 0.99,
            temp_k: 290.0,
            relhum: 0.6,
            psrf: 1.0e5,
            transp_water: 0.1,
            deltim: 1800.0,
            leaf_area: 2.0,
            aerodynamic_resistance: 50.0,
            stomatal_resistance: 100.0,
            prev_delta_e: 0.0,
            prev_delta_b: 0.0,
            prev_peclet: 1.0,
            prev_leaf_moles: 0.0,
        },
    );
    assert_eq!(out.trans_ratio, RSMOW_18O);
    assert_eq!(out.new_peclet, 1.0);
    assert_eq!(out.new_leaf_moles, 0.0);
}

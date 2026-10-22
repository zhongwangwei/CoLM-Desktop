use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("repo root")
}

#[test]
fn methane_provider_registration_is_not_guarded_by_the_removed_bgc_macro() {
    let providers = std::fs::read_to_string(
        root().join("vendor/CoLM202X/include/tracer_lifecycle_providers.inc"),
    )
    .unwrap();
    assert!(providers.contains("ch4_register_tracer_provider"));
    assert!(
        !providers.contains("#ifdef BGC"),
        "BGC is a runtime switch; this guard builds CH4 but omits its provider"
    );
}

#[test]
fn methane_routing_modes_reject_kernels_without_river_routing() {
    let methane = std::fs::read_to_string(
        root().join("vendor/CoLM202X/main/TRACER/MOD_Tracer_Reactive_Methane_Const.F90"),
    )
    .unwrap();
    for mode in ["routing", "hybrid"] {
        assert!(
            methane.contains(&format!(
                "{mode} methane inundation mode requires a GridRiverLakeFlow-enabled kernel"
            )),
            "{mode} silently falls back to zero routing state in CaMaOFF kernels"
        );
    }
}

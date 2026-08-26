use std::path::PathBuf;

fn read(rel: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    std::fs::read_to_string(root.join(rel)).unwrap_or_else(|error| panic!("{rel}: {error}"))
}

#[test]
fn singlepoint_bgc_runtime_guards_unsafe_zero_and_mesh_paths() {
    let summary = read("vendor/CoLM202X/main/BGC/MOD_BGC_CNSummary.F90");
    assert!(summary.contains("IF(nfixlags > 0._r8 .and. lag_npp(i) /= spval)THEN"));

    let equilibrium = read("vendor/CoLM202X/main/MOD_CheckEquilibrium.F90");
    let area = equilibrium
        .split_once("allocate (patcharea (numpatch))")
        .expect("patch-area allocation")
        .1
        .split_once("nyearcheck = 0")
        .expect("patch-area initialization end")
        .0;
    let single = area.find("#ifdef SinglePoint").expect("SinglePoint guard");
    let mesh = area.find("landpatch%ielm(ip)").expect("mesh lookup");
    assert!(single < mesh);
    assert!(area[..mesh].contains("#else"));
}

#[test]
fn pft_carbon_flux_and_grass_lai_history_units_match_their_values() {
    let history = read("vendor/CoLM202X/main/MOD_Hist.F90");
    let lines = history.lines().collect::<Vec<_>>();
    let mut carbon_fluxes = 0;
    for (index, line) in lines.iter().enumerate() {
        if line.contains("file_hist, 'f_npp_") || line.contains("file_hist, 'f_npptoleafc_") {
            carbon_fluxes += 1;
            assert!(
                lines[index + 1].contains("'gC/m2/s')"),
                "wrong carbon-flux units after {line}"
            );
        }
    }
    assert_eq!(carbon_fluxes, 28);
    for name in ["c3arcgrass", "c3grass", "c4grass"] {
        let variable = format!("'f_lai_{name}'");
        let index = lines
            .iter()
            .position(|line| line.contains(&variable))
            .unwrap_or_else(|| panic!("missing {variable}"));
        assert!(lines[index + 1].contains("'m2/m2')"));
    }
}

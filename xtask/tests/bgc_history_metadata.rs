use std::fs;
use std::path::Path;

fn hist_source() -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    fs::read_to_string(root.join("vendor/CoLM202X/main/MOD_Hist.F90")).unwrap()
}

fn metadata_line<'a>(text: &'a str, var: &str) -> &'a str {
    let marker = format!("'{var}'");
    let tail = text
        .split(&marker)
        .nth(1)
        .unwrap_or_else(|| panic!("missing {var}"));
    tail.lines()
        .find(|line| line.trim_start().starts_with("'"))
        .unwrap_or_else(|| panic!("missing metadata for {var}"))
}

#[test]
fn bgc_pft_history_units_match_carbon_fluxes_and_lai_area() {
    let text = hist_source();
    for pft in [
        "enftemp",
        "enfboreal",
        "dnfboreal",
        "ebftrop",
        "ebftemp",
        "dbftrop",
        "dbftemp",
        "dbfboreal",
        "ebstemp",
        "dbstemp",
        "dbsboreal",
        "c3arcgrass",
        "c3grass",
        "c4grass",
    ] {
        assert!(
            metadata_line(&text, &format!("f_lai_{pft}")).contains("'m2/m2'"),
            "LAI {pft} unit must be m2/m2"
        );
        for name in ["npp", "npptoleafc"] {
            assert!(
                metadata_line(&text, &format!("f_{name}_{pft}")).contains("'gC/m2/s'"),
                "{name} {pft} unit must be gC/m2/s"
            );
        }
    }
}

#[test]
fn bgc_pft_history_descriptions_do_not_reuse_neighbor_pft_labels() {
    let text = hist_source();
    assert!(metadata_line(&text, "f_lai_ebftemp").contains("broadleaf evergreen temperate tree"));
    assert!(metadata_line(&text, "f_lai_dbftrop").contains("broadleaf deciduous tropical tree"));
    assert!(metadata_line(&text, "f_lai_c3arcgrass").contains("c3 arctic grass"));
    assert!(metadata_line(&text, "f_lai_c3grass").contains("c3 grass"));
    assert!(metadata_line(&text, "f_lai_c4grass").contains("c4 grass"));
    assert!(!metadata_line(&text, "f_lai_c3grass").contains("arctic"));
}

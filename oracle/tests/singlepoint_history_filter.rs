fn source(path: &str) -> String {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(path);
    std::fs::read_to_string(path).expect("Fortran source must be readable")
}

fn flat(path: &str) -> String {
    source(path)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn singlepoint_history_writers_apply_the_patch_filter() {
    let single = flat("vendor/CoLM202X/main/MOD_HistSingle.F90");
    assert!(single.contains("WHERE (.not. filter) acc_vec = spval"));
    assert!(single.contains("WHERE (.not. filter) acc_vec(i1,:) = spval"));
    assert!(single.contains("WHERE (.not. filter) acc_vec(i1,i2,:) = spval"));

    for path in [
        "vendor/CoLM202X/main/MOD_Hist.F90",
        "vendor/CoLM202X/main/TRACER/MOD_Tracer_Hist.F90",
    ] {
        let calls = flat(path);
        assert!(!calls.contains("itime_in_file, longname, units)"), "{path}");
        assert!(!calls.contains("ndim1, longname, units)"), "{path}");
        assert!(!calls.contains("ndim2, longname, units)"), "{path}");
    }
}

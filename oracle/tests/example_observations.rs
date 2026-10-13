//! Installed examples must retain water/energy observations as well as methane.
use std::path::Path;

#[test]
fn at_neu_keeps_complete_observations_on_the_distributed_clock() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples");
    let obs =
        netcdf::open(examples.join("Observation/AT-Neu_2010-2012_FLUXNET-CH4_Flux.nc")).unwrap();
    let met = netcdf::open(examples.join("Forcing/AT-Neu_2010-2012_FLUXNET-CH4_Met.nc")).unwrap();
    assert_eq!(obs.dimension("time").unwrap().len(), 52_608);
    assert_eq!(
        obs.variable("time")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap(),
        met.variable("time")
            .unwrap()
            .get_values::<f64, _>(..)
            .unwrap()
    );
    for name in ["Qh", "Qle", "Rnet", "Qg", "SWup", "Ustar", "NEE"] {
        let values = obs
            .variable(name)
            .expect("raw observations must be bundled")
            .get_values::<f32, _>(..)
            .unwrap();
        let flags = obs
            .variable(&format!("{name}_qc"))
            .expect("raw presence classification")
            .get_values::<i8, _>(..)
            .unwrap();
        assert_eq!(values.len(), flags.len());
        assert!(
            flags.iter().filter(|&&flag| flag == 0).count() > 20_000,
            "{name}"
        );
        for (value, flag) in values.iter().zip(flags) {
            assert_eq!(
                flag,
                if value.is_finite() && *value != -9999.0 {
                    0
                } else {
                    3
                },
                "{name}"
            );
        }
    }
    for name in [
        "FCH4",
        "FCH4_f",
        "FCH4_f_ann",
        "FCH4_f_ann_qc",
        "GPP",
        "GPP_DT",
        "Resp",
    ] {
        assert!(obs.variable(name).is_some(), "missing {name}");
    }
}

#[test]
fn only_the_turbulent_fluxes_have_a_corrected_twin() {
    // 闭合订正把可用能量的残差按 Bowen 比分给感热与潜热，
    // 辐射量与地表热通量不受影响 —— 给它们编一个 `_cor` 名字会读不到变量。
    assert_eq!(super::corrected("Qle"), Some("Qle_cor"));
    assert_eq!(super::corrected("Qh"), Some("Qh_cor"));
    for n in ["Rnet", "SWup", "Qg"] {
        assert_eq!(super::corrected(n), None, "{n} 没有订正版");
    }
    // 订正版的名字就是原名加 `_cor` —— 不是另起一个名字。
    // 拼错的话读到的是"没有这个变量"，然后那一行会被静默跳过。
    for variable in super::EVALUATION_VARIABLES {
        if let Some(c) = super::corrected(variable.observation) {
            assert_eq!(c, format!("{}_cor", variable.observation));
        }
    }
}

#[test]
fn every_scientific_observation_variable_has_one_model_definition() {
    let names = super::EVALUATION_VARIABLES
        .iter()
        .map(|variable| variable.observation)
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "Rnet",
            "Qh",
            "Qle",
            "Qg",
            "SWup",
            "Ustar",
            "GPP",
            "GPP_DT",
            "Resp",
            "NEE",
            "FCH4_f_ann"
        ]
    );
    assert_eq!(
        super::EVALUATION_VARIABLES
            .iter()
            .filter(|variable| variable.qc.is_none())
            .map(|variable| variable.observation)
            .collect::<Vec<_>>(),
        ["GPP", "GPP_DT", "Resp", "FCH4_f_ann"]
    );
}

#[test]
fn urban_plumber_rnet_is_derived_from_radiative_components_with_strict_qc() {
    assert_eq!(
        super::derived_observation_components("Rnet"),
        Some(["SWdown", "LWdown", "SWup", "LWup"])
    );
    assert_eq!(
        super::derived_observation_label("Rnet"),
        Some("SWdown+LWdown-SWup-LWup")
    );
    let (values, qc) = super::derive_urban_rnet(
        [
            &[500.0, 500.0, crate::pair::FILL_VALUE][..],
            &[350.0, 350.0, 350.0][..],
            &[100.0, 100.0, 100.0][..],
            &[420.0, 420.0, 420.0][..],
        ],
        [
            &[
                crate::pair::QC_MEASURED,
                crate::pair::QC_MEASURED,
                crate::pair::QC_MEASURED,
            ][..],
            &[
                crate::pair::QC_MEASURED,
                crate::pair::QC_MEASURED,
                crate::pair::QC_MEASURED,
            ][..],
            &[crate::pair::QC_MEASURED, 1.0, crate::pair::QC_MEASURED][..],
            &[
                crate::pair::QC_MEASURED,
                crate::pair::QC_MEASURED,
                crate::pair::QC_MEASURED,
            ][..],
        ],
    )
    .unwrap();
    assert_eq!(
        values,
        [330.0, crate::pair::FILL_VALUE, crate::pair::FILL_VALUE]
    );
    assert_eq!(qc, [crate::pair::QC_MEASURED, 1.0, 1.0]);
}

#[test]
fn carbon_flux_definitions_convert_moles_and_derive_nee_explicitly() {
    use super::ModelSource;

    let gpp = super::EVALUATION_VARIABLES
        .iter()
        .find(|variable| variable.observation == "GPP")
        .unwrap();
    assert!(matches!(
        gpp.model,
        ModelSource::Alternative {
            preferred: &ModelSource::Direct {
                variable: "f_gpp",
                ..
            },
            fallback: &ModelSource::Direct {
                variable: "f_assim",
                ..
            },
        }
    ));
    assert_eq!(
        gpp.model.required_alternatives(),
        [vec!["f_gpp"], vec!["f_assim"]]
    );
    let nee = super::EVALUATION_VARIABLES
        .iter()
        .find(|variable| variable.observation == "NEE")
        .unwrap();
    assert!(matches!(
        nee.model,
        ModelSource::Alternative {
            preferred: &ModelSource::SumDifference {
                positive: &["f_ar", "f_hr"],
                negative: &["f_gpp"],
                ..
            },
            fallback: &ModelSource::Difference {
                minuend: "f_respc",
                subtrahend: "f_assim",
                ..
            },
        }
    ));
    assert_eq!(
        nee.model.required_alternatives(),
        [vec!["f_ar", "f_hr", "f_gpp"], vec!["f_respc", "f_assim"]]
    );

    let methane = super::EVALUATION_VARIABLES
        .iter()
        .find(|variable| variable.observation == "FCH4_f_ann")
        .unwrap();
    assert_eq!(
        methane.model,
        ModelSource::Direct {
            variable: "f_methane_surf_flux_tot",
            scale: 1_000_000_000.0,
        }
    );
}

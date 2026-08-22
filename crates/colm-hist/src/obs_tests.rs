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
        ["Rnet", "Qh", "Qle", "Qg", "SWup", "Ustar", "GPP", "GPP_DT", "Resp", "NEE"]
    );
    assert_eq!(
        super::EVALUATION_VARIABLES
            .iter()
            .filter(|variable| variable.qc.is_none())
            .map(|variable| variable.observation)
            .collect::<Vec<_>>(),
        ["GPP", "GPP_DT", "Resp"]
    );
}

#[test]
fn carbon_flux_definitions_convert_moles_and_derive_nee_explicitly() {
    use super::ModelSource;

    let gpp = super::EVALUATION_VARIABLES
        .iter()
        .find(|variable| variable.observation == "GPP")
        .unwrap();
    assert_eq!(
        gpp.model,
        ModelSource::Direct {
            variable: "f_assim",
            scale: 1_000_000.0,
        }
    );
    let nee = super::EVALUATION_VARIABLES
        .iter()
        .find(|variable| variable.observation == "NEE")
        .unwrap();
    assert_eq!(nee.model.required(), ["f_respc", "f_assim"]);
    assert_eq!(nee.model.label(), "f_respc - f_assim");
}

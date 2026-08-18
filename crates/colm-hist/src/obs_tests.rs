
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
    for (o, _) in super::FLUX_PAIRS {
        if let Some(c) = super::corrected(o) {
            assert_eq!(c, format!("{o}_cor"));
        }
    }
}

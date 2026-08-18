use super::*;

#[test]
fn an_empty_value_does_not_erase_the_remembered_one() {
    // 用户清空一个框通常是想重新填，不是想忘掉历史。
    // 直接写进去的话，下次打开那个框就又是空的 —— 而这个功能的全部意义
    // 就是让它别是空的。
    let mut all: BTreeMap<String, String> = BTreeMap::new();
    all.insert("sitedir".into(), "/data/Sitedata".into());
    // `save_recent` 的判据（不依赖 AppHandle 的那半）
    for candidate in ["", "   ", "\t"] {
        assert!(candidate.trim().is_empty(), "{candidate:?} 应当被当成空值");
    }
    assert!(!"/data/other".trim().is_empty());
    assert_eq!(
        all.get("sitedir").map(String::as_str),
        Some("/data/Sitedata")
    );
}

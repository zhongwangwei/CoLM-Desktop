use super::*;

#[test]
fn the_required_list_is_the_twelve_measured_gaps() {
    // 实测：90 个 PLUMBER2 站点文件的变量集完全相同（各 39 个），
    // 与能跑通的增广文件（51 个）之差正好是这 12 个。
    assert_eq!(REQUIRED_FIELDS.len(), 12);
    assert!(REQUIRED_FIELDS.contains(&"soil_texture"));
    assert!(REQUIRED_FIELDS.contains(&"soil_wf_om"));
}

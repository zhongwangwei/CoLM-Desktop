use super::*;

#[test]
fn block_names_decode_to_their_south_west_corner() {
    assert_eq!(block_origin("e110_n20").unwrap(), (110.0, 20.0));
    assert_eq!(block_origin("w180_s90").unwrap(), (-180.0, -90.0));
    assert!(block_origin("x110_n20").is_err());
    assert!(block_origin("e110").is_err());
}

/// 上游 `block_set_local_blocks`：外层经度块、内层纬度块，都从西南往东北。
#[test]
fn blocks_sort_longitude_first_then_latitude() {
    let mut names = ["e115_n20", "e110_n25", "w005_s10", "e110_n20"]
        .map(|name| (block_origin(name).unwrap(), name));
    names.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let order = names.map(|(_, name)| name);
    assert_eq!(order, ["w005_s10", "e110_n20", "e110_n25", "e115_n20"]);
}

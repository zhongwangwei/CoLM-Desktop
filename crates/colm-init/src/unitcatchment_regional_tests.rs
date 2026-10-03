//! 区域单元流域的选择：两条河流系统 1→2→3、4→5，单元流域 i 的径流来自格子 (i, 1)。

use super::*;

/// 每个单元流域的 `inpmat_x`/`inpmat_y` 列。
type Inpmat = Vec<(Vec<i32>, Vec<i32>)>;

fn network() -> (Vec<i32>, Inpmat) {
    let seq_next = vec![2, 3, 0, 5, 0];
    let inpmat = (1..=5).map(|i| (vec![i, -1], vec![1, -1])).collect();
    (seq_next, inpmat)
}

/// 只有单元流域 1 接收陆面径流：整条 1→2→3 都留下，另一条系统去掉；下标重编号。
#[test]
fn a_touched_catchment_keeps_its_whole_river_system() {
    let (seq_next, inpmat) = network();
    let touched = [true, false, false, false, false];
    let selection = select(&seq_next, &inpmat, 5, 1, &touched, None, None, true).unwrap();
    assert_eq!(selection.seq, vec![1, 2, 3]);
    assert_eq!(selection.systems, 1);
    assert_eq!(selection.new_index, vec![0, 1, 2, 3, 0, 0]);
}

/// 分汊路径 2→4 连通两条系统：closure 时把另一条也拉进来，drop 时丢掉这条路径。
#[test]
fn bifurcation_closure_grows_the_selection() {
    let (seq_next, inpmat) = network();
    let touched = [true, false, false, false, false];
    let (up, down) = (vec![2], vec![4]);
    let closed = select(
        &seq_next,
        &inpmat,
        5,
        1,
        &touched,
        Some((&up, &down)),
        Some(&[5, 4]),
        true,
    )
    .unwrap();
    assert_eq!(closed.seq, vec![1, 2, 3, 4, 5]);
    assert_eq!(closed.path, vec![1]);
    assert_eq!(closed.dam, vec![1, 2]);
    assert_eq!(closed.systems, 2);

    let dropped = select(
        &seq_next,
        &inpmat,
        5,
        1,
        &touched,
        Some((&up, &down)),
        Some(&[5, 2]),
        false,
    )
    .unwrap();
    assert_eq!(dropped.seq, vec![1, 2, 3]);
    assert!(dropped.path.is_empty());
    assert_eq!(dropped.dam, vec![2]);
}

/// 没有任何单元流域接收径流时报错（网络与区域对不上）。
#[test]
fn an_untouched_domain_is_refused() {
    let (seq_next, inpmat) = network();
    let error = select(&seq_next, &inpmat, 5, 1, &[false; 5], None, None, true).unwrap_err();
    assert!(format!("{error}").contains("no unit catchment receives runoff"));
}

/// 沿 C 序的某一轴取子集。
#[test]
fn cut_selects_along_one_axis() {
    // shape (3, 2)：沿第 0 轴取第 1、3 行。
    let values = [1, 2, 3, 4, 5, 6];
    assert_eq!(cut(&values, &[3, 2], Some((0, &[1, 3]))), vec![1, 2, 5, 6]);
    // 沿第 1 轴取第 2 列。
    assert_eq!(cut(&values, &[3, 2], Some((1, &[2]))), vec![2, 4, 6]);
}

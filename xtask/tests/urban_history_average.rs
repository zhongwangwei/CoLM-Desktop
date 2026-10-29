use std::path::PathBuf;

fn source() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    std::fs::read_to_string(root.join("vendor/CoLM202X/main/MOD_Hist.F90"))
        .expect("read MOD_Hist.F90")
}

#[test]
fn singlepoint_urban_history_is_averaged_once_before_writing() {
    let source = source();
    let writer = source
        .split_once("SUBROUTINE write_history_variable_urb_2d")
        .expect("urban history writer")
        .1
        .split_once("END SUBROUTINE write_history_variable_urb_2d")
        .expect("urban history writer end")
        .0;
    // 上游 7d2fca93 把累加量的除法提到 SELECT CASE 之前，按 SinglePoint
    // 宏分两支编译（同一时刻只有一支生效，所以运行期只除一次）。
    // 本地原先在 CASE ('Single') 里另除一次，照上游结构会变成除两次，
    // 因此这里守的是：SELECT 之前有除法，Single 分支里不能再有。
    let average = "acc_vec = acc_vec / nac";
    let (before_select, select) = writer
        .split_once("select CASE (HistForm)")
        .expect("HistForm select");
    assert!(before_select.contains(average));
    let single = select
        .split_once("CASE ('Single')")
        .expect("SinglePoint branch")
        .1;
    assert!(!single.contains(average), "SinglePoint would average twice");
    assert!(
        single.find("CALL single_write_2d").is_some(),
        "SinglePoint writer"
    );
}

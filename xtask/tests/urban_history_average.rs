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
    let (before_single, single) = writer
        .split_once("CASE ('Single')")
        .expect("SinglePoint branch");
    let average = "acc_vec = acc_vec / nac";

    assert!(!before_single.contains(average));
    assert!(
        single.find(average).expect("SinglePoint urban average")
            < single
                .find("CALL single_write_2d")
                .expect("SinglePoint writer")
    );
}

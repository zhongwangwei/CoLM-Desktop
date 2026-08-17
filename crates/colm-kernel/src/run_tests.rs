use super::*;

#[test]
fn a_report_knows_whether_it_succeeded() {
    // run_stage 本身要跑真二进制，由黄金回归验；这里只钉住这个小判据，
    // 免得它将来被改成「只要没崩就算成功」。
    let r = StageReport {
        stage: Stage::Colm,
        outcome: Outcome::Succeeded,
        log: PathBuf::from("/tmp/colm.log"),
        overrides: Vec::new(),
    };
    assert!(r.succeeded());
}

#[test]
fn the_stage_names_and_the_manifest_names_are_the_same_three() {
    // 程序名有两个真相来源：`Stage::program()` 与 `manifest::PROGRAMS`。
    // 二者必须一致 —— 改了一处没改另一处，`Kernel::open` 会去校验一个
    // 不存在的文件，或 `run_stage` 会去跑一个没被校验过的文件，
    // 而两边各自的测试仍然全绿。这条把它们拴在一起。
    use crate::manifest::PROGRAMS;
    let from_stages = [
        Stage::MkSrfData.program(),
        Stage::MkIniData.program(),
        Stage::Colm.program(),
    ];
    assert_eq!(from_stages, PROGRAMS);
}

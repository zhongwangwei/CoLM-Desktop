//! 两个都叫 spin-up 的东西，在界面上必须分得开。
//!
//! 它们不是一回事：模型预热（`DEF_simulation_time%spinup_*`，单位轮数，
//! 不写 history）与评估丢弃（`colm-cli metrics --spinup N`，单位输出记录条数）。
//! 两处都写「spinup」的话，第一个看界面的人一定会搞混，而搞混的代价是
//! 「我明明设了 spinup 为什么指标没变」这类查不出来的困惑。

use std::path::PathBuf;

fn read(rel: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("repo root");
    std::fs::read_to_string(root.join(rel)).unwrap_or_default()
}

#[test]
fn the_two_spinups_have_different_names_in_the_interface() {
    let html = read("gui/dist/index.html");
    // 只看包着那个输入框的 `<label>`，不取字节窗口。
    //
    // 窗口做法错在这里：紧跟其后的说明段**故意**提到「预热」来指认另一个
    // 同名的东西 —— 那是应该的，而按窗口检查会把它判成违规。
    // 第一版就是这么误报的。
    let at = html.find("id=\"spinup\"").expect("评估的输入框");
    let open = html[..at].rfind("<label").expect("它外面的 label");
    let close = html[open..].find("</label>").expect("label 的收尾") + open;
    let label = &html[open..close];
    // 只看**可见文本**，也就是开标签之后的部分。属性里提到另一个名字是
    // 应该的 —— tooltip 正是用来指认「那个跟我同名的东西在别处」。
    // 连属性一起查的话，一条写得更清楚的提示反而会让测试红。
    let text = &label[label.find('>').expect("开标签的收尾") + 1..];
    assert!(
        text.contains("丢弃"),
        "评估侧的可见标签要叫「丢弃」：{text}"
    );
    assert!(
        !text.contains("预热"),
        "评估侧的可见标签不该叫「预热」，那是模型那一侧的名字：{text}"
    );
}

#[test]
fn the_model_side_says_it_writes_no_history() {
    // 不说这句的话，用户会以为预热期的输出被算进了指标 —— 而那正好是
    // 两个 spin-up 最容易混起来的那一点。
    let js = read("gui/dist/app/params.js");
    let i = js
        .find("DEF_simulation_time%spinup_repeat")
        .expect("模型预热的说明");
    let block = &js[i..(i + 600).min(js.len())];
    assert!(block.contains("预热"), "模型侧要叫「预热」");
    assert!(
        block.contains("history"),
        "要说清楚预热期不写 history：{block}"
    );
    assert!(
        block.contains("丢弃前"),
        "要指认另一个同名的东西，免得用户自己去猜：{block}"
    );
}

//! Windows 上不该弹出任何控制台窗口。
//!
//! **这几条只能靠源文本守。** 开发机是 macOS，`#[cfg(windows)]` 里的代码
//! 本地既不编译也不运行，漏了哪一处要等有人在 Windows 上双击才看得见 ——
//! 而那时黑框已经弹出去了。

use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("repo root")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

#[test]
fn the_gui_binary_is_not_a_console_app() {
    // 缺了这一行，双击安装好的程序会先弹一个黑框再出界面，
    // 而关掉那个黑框会把程序一起杀掉。
    let m = read("gui/src-tauri/src/main.rs");
    assert!(
        m.contains(r#"windows_subsystem = "windows""#),
        "main.rs 少了 windows_subsystem 属性"
    );
    // 调试构建要留着控制台，`println!` 得有地方去。
    assert!(
        m.contains("not(debug_assertions)"),
        "windows_subsystem 不该在调试构建里也生效"
    );
}

#[test]
fn every_spawned_process_is_told_not_to_open_a_window() {
    // 一次运行起四个进程：界面起 colm-cli，它再依次起 mksrfdata.x /
    // mkinidata.x / colm.x。四个都是控制台程序，漏掉一处就是一个黑框。
    for f in [
        "gui/src-tauri/src/sidecar.rs",
        "crates/colm-kernel/src/run.rs",
    ] {
        let t = read(f);
        let spawns = t.matches("Command::new(").count();
        let guarded = t.matches("no_console(").count();
        assert!(
            guarded >= spawns,
            "{f}: {spawns} 处 Command::new 只有 {guarded} 处过了 no_console"
        );
    }
    // 判断只写一份 —— 四处各写一遍 #[cfg(windows)] 的话，漏掉一处
    // 在 macOS 上永远复现不出来。
    let k = read("crates/colm-kernel/src/run.rs");
    assert!(k.contains("pub fn no_console"));
    // 传给 `creation_flags` 的**只能是** CREATE_NO_WINDOW。
    // DETACHED_PROCESS(0x8) 会让子进程脱离作业对象，界面退出时模型还在
    // 后台跑；CREATE_NEW_CONSOLE(0x10) 则恰好是要避免的那件事。
    //
    // 看实际参数而不是搜关键字：注释里解释「为什么不用 DETACHED_PROCESS」
    // 会让搜关键字的检查红掉，而那是一条把话说清楚的注释。实测踩过。
    let flags: Vec<&str> = k
        .match_indices("creation_flags(")
        .map(|(i, m)| {
            let rest = &k[i + m.len()..];
            &rest[..rest.find(')').expect("creation_flags 的右括号")]
        })
        .collect();
    assert_eq!(flags, vec!["0x0800_0000"], "creation_flags 的取值不对");
}

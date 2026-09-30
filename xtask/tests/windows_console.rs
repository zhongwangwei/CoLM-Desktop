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

#[test]
fn the_frontend_does_not_assume_forward_slashes() {
    // Windows 上算例目录是 `C:\Users\…\CN-Cng`。只按 `/` 切会原样返回整条
    // 路径 —— 横幅上写的就不是「CN-Cng」而是一长串绝对路径。
    // 这一条在 macOS 上永远看不出来。
    for f in [
        "params.js",
        "sites.js",
        "runner.js",
        "results.js",
        "timing.js",
    ] {
        let t = read(&format!("gui/dist/app/{f}"));
        assert!(
            !t.contains(".split('/')"),
            "{f} 按 `/` 切路径 —— Windows 上切不开"
        );
        assert!(!t.contains("+ '/' +"), "{f} 用 `/` 拼路径 —— 该走 joinPath");
    }
    // 判断只写一份，在没有依赖的那一层。
    let ui = read("gui/dist/app/ui.js");
    assert!(ui.contains("export function baseName"));
    assert!(ui.contains("export function joinPath"));
}

#[test]
fn the_kernel_creates_directories_without_cmd_expansion() {
    let sh = read("oracle/scripts/build_kernel.sh");
    assert!(sh.contains("tar -h --exclude=.git -cf - ."));
    assert!(
        !sh.contains("has upstream changed?"),
        "obsolete mkdir rewrite must be removed"
    );
    let helper = read("vendor/CoLM202X/share/CoLM_Mkdir.c");
    assert!(helper.contains("_mkdir(path)"));
    assert!(helper.contains("mkdir(path, 0777)"));
    assert!(helper.contains("FindFirstFileA"));
    assert!(helper.contains(
        "if (dir_len == 2 && prefix[1] == ':' && colm_is_separator(prefix[2])) dir_len = 3;"
    ));
    assert!(helper.contains("return c == '/';"));
    assert!(helper.contains("colm_same_file"));
    assert!(helper.contains("GetFileInformationByHandle"));
    assert!(helper.contains("MoveFileExA(src, dst, MOVEFILE_REPLACE_EXISTING)"));
    assert!(!helper.contains("remove(dst)"));
    assert!(helper.contains("st_dev == b.st_dev && a.st_ino == b.st_ino"));
    assert!(helper.contains("rename(src, dst)"));
    assert!(!helper.contains("system("));
    let module = read("vendor/CoLM202X/share/MOD_Filesystem.F90");
    assert!(module.contains("BIND(C, name=\"colm_is_separator\")"));
    assert!(module
        .contains("is_separator = separator_native(iachar(character, kind=c_int)) /= 0_c_int"));
    assert!(!module.contains("#ifdef _WIN32"));
    assert!(module.contains("PUBLIC :: make_directory, copy_file, list_matching_paths, move_file"));
    assert!(module.contains("SUBROUTINE copy_file"));
    assert!(module.contains("Refusing to copy file onto itself"));
    let makefile = read("vendor/CoLM202X/Makefile");
    assert!(makefile.contains("CoLM_Mkdir.o: share/CoLM_Mkdir.c"));
    // C 助手得进 OBJS_SHARED，否则三个可执行文件都链接不上 colm_make_directory。
    assert!(
        make_list(&makefile, "OBJS_SHARED").contains(&"CoLM_Mkdir.o".to_string()),
        "CoLM_Mkdir.o 不在 OBJS_SHARED 里"
    );
    // 要守的是编译顺序：谁 USE 了 MOD_Filesystem、MOD_Filesystem 又 USE 了谁，
    // 对应的 .mod 就得先生成。不锁依赖行的原文 —— 上游把
    // `MOD_Namelist.o: MOD_SPMD_Task.o MOD_Filesystem.o` 拆成了
    // `MOD_Filesystem.o: MOD_SPMD_Task.o` + `MOD_Namelist.o: MOD_Filesystem.o`，
    // 对 make 来说是一回事（传递前置照样先建），逐字比对却会红。
    let graph = make_prerequisites(&makefile);
    assert!(
        fortran_uses(&read("vendor/CoLM202X/share/MOD_Namelist.F90")).contains("MOD_Filesystem"),
        "MOD_Namelist 不再 USE MOD_Filesystem —— 建目录换了路子，这条测试要跟着重看"
    );
    let mut edges = vec![("MOD_Namelist", "MOD_Filesystem".to_string())];
    edges.extend(
        fortran_uses(&module)
            .into_iter()
            .filter(|m| m.starts_with("MOD_"))
            .map(|m| ("MOD_Filesystem", m)),
    );
    assert!(edges
        .iter()
        .any(|(a, b)| *a == "MOD_Filesystem" && b == "MOD_SPMD_Task"));
    for (from, to) in edges {
        assert!(
            reaches(&graph, &format!("{from}.o"), &format!("{to}.o")),
            "Makefile 里 {from}.o 够不着 {to}.o —— 并行构建时 {to}.mod 可能还没生成"
        );
    }
}

/// Fortran 源里 `USE` 的模块名（去掉 `, ONLY: ...`）。
fn fortran_uses(src: &str) -> std::collections::BTreeSet<String> {
    src.lines()
        .filter_map(|l| {
            let mut w = l
                .trim_start()
                .split(|c: char| c.is_whitespace() || c == ',');
            w.next()
                .filter(|k| k.eq_ignore_ascii_case("use"))
                .and_then(|_| w.find(|t| !t.is_empty()))
                .map(str::to_string)
        })
        .collect()
}

/// 把 `\` 续行拼回一行。
fn logical_lines(makefile: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for l in makefile.lines() {
        match l.strip_suffix('\\') {
            Some(head) => {
                cur.push_str(head);
                cur.push(' ');
            }
            None => {
                cur.push_str(l);
                out.push(std::mem::take(&mut cur));
            }
        }
    }
    out
}

/// `NAME = a b c` 这种变量的取值。
fn make_list(makefile: &str, name: &str) -> Vec<String> {
    logical_lines(makefile)
        .iter()
        .find_map(|l| {
            let (lhs, rhs) = l.split_once('=')?;
            (lhs.trim() == name).then(|| rhs.split_whitespace().map(str::to_string).collect())
        })
        .unwrap_or_else(|| panic!("Makefile 里没有 {name}"))
}

/// 显式规则的前置依赖图。同一目标写在多行上时按 make 的规矩取并集；
/// 模式规则、变量赋值、命令行都跳过；order-only（`|` 之后）也算顺序。
fn make_prerequisites(
    makefile: &str,
) -> std::collections::HashMap<String, std::collections::BTreeSet<String>> {
    let mut g: std::collections::HashMap<String, std::collections::BTreeSet<String>> =
        Default::default();
    for l in logical_lines(makefile) {
        if l.starts_with('\t') || l.trim_start().starts_with('#') {
            continue;
        }
        let Some((lhs, rhs)) = l.split_once(':') else {
            continue;
        };
        if lhs.contains(['=', '%', '$']) || rhs.starts_with('=') || rhs.contains('%') {
            continue;
        }
        let rhs = rhs.split(';').next().unwrap_or("");
        let deps: Vec<String> = rhs
            .split_whitespace()
            .filter(|d| *d != "|")
            .map(str::to_string)
            .collect();
        for t in lhs.split_whitespace() {
            g.entry(t.to_string())
                .or_default()
                .extend(deps.iter().cloned());
        }
    }
    g
}

fn reaches(
    g: &std::collections::HashMap<String, std::collections::BTreeSet<String>>,
    from: &str,
    to: &str,
) -> bool {
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![from.to_string()];
    while let Some(n) = stack.pop() {
        if n == to {
            return true;
        }
        if seen.insert(n.clone()) {
            stack.extend(g.get(&n).into_iter().flatten().cloned());
        }
    }
    false
}

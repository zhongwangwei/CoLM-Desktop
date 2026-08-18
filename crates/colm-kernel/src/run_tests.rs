use super::*;

/// 一个跑得起来的假内核：三个 `.x` 都是同一个 shell 脚本。
///
/// `#[cfg(unix)]` 跟着两个使用者走 —— 它们都要 `#!/bin/sh` 与
/// `set_permissions(0o755)`。不跟着标的话，Windows 上这个函数没人用，
/// `clippy -D warnings` 会以 `never used` 报错（实测 CI 上就是这么挂的）。
#[cfg(unix)]
///
/// 直接构造 `Kernel` 而不走 `Kernel::open` —— 这里要验的是 `run_stage*`，
/// 二进制完整性另有 `manifest_tests` 管。
fn fake_kernel(name: &str, script: &str) -> (Kernel, PathBuf) {
    let d = std::env::temp_dir().join(format!("colm-kernel-run-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("work")).expect("create workdir");
    for prog in crate::manifest::PROGRAMS {
        let p = d.join(format!("{prog}.x"));
        std::fs::write(&p, script).expect("write script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
    }
    let manifest = serde_json::from_str(crate::manifest::manifest_tests::SAMPLE).expect("manifest");
    (
        Kernel {
            dir: d.clone(),
            manifest,
        },
        d.join("work"),
    )
}

#[test]
#[cfg(unix)]
fn a_line_reaches_the_caller_while_the_child_is_still_running() {
    // 这条是整个流式改造的**唯一**理由。用 `Command::output()` 的话，回调
    // 要等子进程退出才第一次被调到，于是脚本永远等不到握手文件，
    // 打出 "no-handshake" —— 一个只看「行数对不对」的测试对此毫无察觉。
    let (k, work) = fake_kernel(
        "live",
        "#!/bin/sh\n\
         echo first\n\
         i=0; while [ $i -lt 100 ]; do [ -f \"$PWD/handshake\" ] && break; sleep 0.05; i=$((i+1)); done\n\
         if [ -f \"$PWD/handshake\" ]; then echo saw-handshake; else echo no-handshake; fi\n\
         echo 'CoLM Execution Completed.'\n",
    );
    let mut seen = Vec::new();
    let touch = work.join("handshake");
    let r = run_stage_streaming(
        &k,
        Stage::Colm,
        Path::new("case.nml"),
        &work,
        &[],
        &mut |l| {
            if l == "first" {
                std::fs::write(&touch, b"").expect("touch");
            }
            seen.push(l.to_string());
        },
    )
    .expect("runs");
    assert_eq!(
        seen,
        ["first", "saw-handshake", "CoLM Execution Completed."]
    );
    assert!(r.succeeded());
}

#[test]
#[cfg(unix)]
fn the_log_is_byte_identical_whether_or_not_anyone_is_watching() {
    // 黄金回归读的是这份日志。流式那条路重新拼接了 stdout，所以「拼回来的
    // 字节跟原来一样」必须被钉住 —— 末行不带换行、以及 stderr 分隔符两处
    // 最容易在重新拼接时走样。
    let script = "#!/bin/sh\n\
                  echo 'Note: something was overridden'\n\
                  echo 'to stderr' >&2\n\
                  printf 'a last line with no newline'\n";
    let (k1, w1) = fake_kernel("log-a", script);
    let (k2, w2) = fake_kernel("log-b", script);
    let a = run_stage(&k1, Stage::Colm, Path::new("case.nml"), &w1, &[]).expect("plain");
    let b = run_stage_streaming(
        &k2,
        Stage::Colm,
        Path::new("case.nml"),
        &w2,
        &[],
        &mut |_| {},
    )
    .expect("streaming");
    let ba = std::fs::read(&a.log).expect("read a");
    let bb = std::fs::read(&b.log).expect("read b");
    assert_eq!(String::from_utf8_lossy(&ba), String::from_utf8_lossy(&bb));
    assert!(ba.ends_with(b"to stderr\n"), "stderr 该在末尾");
    // 覆盖抽取读的是同一份文本，两条路必须抽出同样的东西
    assert_eq!(a.overrides.len(), b.overrides.len());
    assert_eq!(a.overrides.len(), 1);
}

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

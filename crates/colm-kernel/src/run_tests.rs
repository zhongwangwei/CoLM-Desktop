use super::*;

#[test]
fn mpi_uses_plain_spmd_launch_without_process_roles() {
    let exe = Path::new("/kernel/colm.x");
    let (program, args) = launch_command(exe, Path::new("/case/case.nml"), 4).expect("mpi");
    assert_eq!(program, PathBuf::from("mpiexec"));
    assert_eq!(args, ["-n", "4", "/kernel/colm.x", "/case/case.nml"]);
    assert!(!args
        .iter()
        .any(|arg| matches!(arg.as_str(), "master" | "io" | "worker")));
    let (program, args) = launch_command(exe, Path::new("/case/case.nml"), 1).expect("serial");
    assert_eq!(program, exe);
    assert_eq!(args, ["/case/case.nml"]);
}

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
    // **目录名要带进程号。** 同一台机器上并行跑两个 `cargo test`
    // （两个仓库、或者一个会话在验证另一个会话在改）会共用 `/tmp`，
    // 固定名字就成了两个进程抢同一个目录：一个 `remove_dir_all`，
    // 另一个正在写，报出来是 `Os { code: 22, kind: InvalidInput }` ——
    // 看着像本模块的 bug，其实是隔壁进程删了你的目录。
    let d = std::env::temp_dir().join(format!("colm-kernel-run-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("work")).expect("create workdir");
    for prog in crate::manifest::PROGRAMS {
        let p = d.join(crate::manifest::program_file(prog));
        // Linux CI 偶发 `Text file busy`：在 overlayfs 上，刚关闭写句柄就直接
        // exec 同一路径仍可能撞到 ETXTBSY。先写临时文件、关句柄，再原子改名，
        // 让最终可执行路径从未以写模式打开过。
        let tmp = p.with_extension("new");
        std::fs::write(&tmp, script).expect("write script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        std::fs::rename(&tmp, &p).expect("install script");
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
#[cfg(unix)]
fn a_completed_mingw_stage_does_not_wait_for_broken_dll_cleanup() {
    let (mut k, work) = fake_kernel(
        "mingw-shutdown",
        "#!/bin/sh\necho 'Successful in surface data making.'\nexec sleep 30\n",
    );
    k.manifest.platform = "MINGW64_NT-test-x86_64".into();

    let started = std::time::Instant::now();
    let r = run_stage(&k, Stage::MkSrfData, Path::new("case.nml"), &work, &[]).expect("runs");

    assert!(r.succeeded());
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
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

#[test]
fn a_real_kernel_can_actually_be_spawned() {
    // 回答一个此前只靠推断的问题：**这个后缀的文件，操作系统真的肯启动吗？**
    //
    // Windows 的 `PATHEXT` 不含 `.x`，PowerShell 就因此拒绝执行它。我们赌的是
    // `Command::new(绝对路径)` 走 `CreateProcessW`、对显式路径不查 `PATHEXT` ——
    // 赌得对不对，只有真在 Windows 上起一次才知道。CI 的 windows-kernel 作业
    // 会带着 `COLM_KERNEL_DIR` 跑这条。
    //
    // 判据是 `run_stage` 返回 `Ok`：它只在**起不来**的时候返回 `Err`。
    // 进程起来了随后自己死掉（这里必然如此，因为 namelist 不存在）算 `Ok`，
    // 只是 `Outcome` 是失败。也就是说这条测的正是「能不能启动」，
    // 不掺杂模型跑得对不对。
    // 路径要**绝对**：`cargo test` 的当前目录是 crate 目录而不是仓库根，
    // 给相对路径会找不到（第一次就是这么挂的）。
    let Ok(dir) = std::env::var("COLM_KERNEL_DIR") else {
        return; // 没有真内核就不测 —— 大多数开发机上如此
    };
    let k = Kernel::open(Path::new(&dir)).expect("the kernel directory verifies");
    let work = std::env::temp_dir().join("colm-kernel-spawn-probe");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).expect("create workdir");

    let r = run_stage(&k, Stage::Colm, Path::new("no-such.nml"), &work, &[])
        .expect("the operating system launched the binary");
    assert!(!r.succeeded(), "没给 namelist 却成功了，判成败那一段有问题");
    // 顺带钉住实际用的文件名，免得将来后缀改了却没人发现。
    let name = k.program("colm");
    assert!(name
        .to_string_lossy()
        .ends_with(crate::manifest::EXE_SUFFIX));
}

#[test]
fn only_the_rangecheck_lines_without_an_exception_are_muted() {
    use super::is_benign_rangecheck as benign;
    // 实测日志里的原样两行（`MOD_RangeCheck.F90:272` 的格式）。
    assert!(benign(
        "Check vector data:            o3uptakesun         is in (   -0.1000000000E+37,   -0.1000000000E+37)"
    ));
    // block 变体中间是两个空格（`MOD_RangeCheck.F90:148`）。
    assert!(benign(
        "Check block  data:      lai is in (   0.0000000000E+00,   0.5000000000E+01)"
    ));

    // 带标记的一律留下 —— 那是唯一的线索，而 CoLMDEBUG 下它还会终止运行。
    for bad in [
        "Check vector data:      wliq is in (   0.0000000000E+00,   0.5000000000E+01) Out of Range!",
        "Check vector data:      wliq is in (   0.0000000000E+00,   0.5000000000E+01) with NAN",
        "Check vector data:      wliq is in (   0.0000000000E+00,   0.5000000000E+01) with NAN Out of Range!",
    ] {
        assert!(!benign(bad), "这一行不该被丢：{bad}");
    }

    // 别的行一概不碰。
    for keep in [
        "TIMESTEP = 1 | DATE = 2002-01-01-00000",
        "Error: Forcing does not cover simulation period!",
        "",
        "Check vector data: 这一行没有右括号收尾",
    ] {
        assert!(!benign(keep), "这一行不该被丢：{keep}");
    }
}

#[test]
fn a_muted_run_says_how_many_lines_it_dropped() {
    // 静默地少掉 2200 万行，与「这一段根本没做范围检查」在日志上长得一样。
    let noisy = (0..50)
        .map(|i| {
            format!(
                "echo 'Check vector data:      v{i} is in (   0.1000000000E+01,   0.2000000000E+01)'"
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let dir = std::env::temp_dir().join("colm-mute-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{noisy}; echo 'TIMESTEP = 1 | DATE = x'"))
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let kept: Vec<&str> = text
        .lines()
        .filter(|l| !super::is_benign_rangecheck(l))
        .collect();
    assert_eq!(kept.len(), 1, "只该留下那一行 TIMESTEP：{kept:?}");
    assert!(kept[0].contains("TIMESTEP"));
}

#[cfg(unix)]
#[test]
fn a_top_level_sidecar_leads_its_own_process_group() {
    let mut child = super::top_level_sidecar(&mut std::process::Command::new("sh"))
        .args(["-c", "sleep 30"])
        .spawn()
        .expect("spawn probe");
    let group = format!("-{}", child.id());
    let exists = std::process::Command::new("kill")
        .args(["-0", group.as_str()])
        .status()
        .expect("probe process group");
    let _ = std::process::Command::new("kill")
        .args(["-TERM", group.as_str()])
        .status();
    let _ = child.wait();
    assert!(exists.success(), "sidecar must be its process-group leader");
}

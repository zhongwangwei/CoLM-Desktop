use super::*;

#[test]
fn the_seatbelt_profile_denies_network_and_lists_the_writable_places() {
    let dir = std::env::temp_dir().join("colm-sbx-profile");
    std::fs::create_dir_all(&dir).unwrap();
    let profile = seatbelt_profile(&Policy {
        writable: vec![dir.clone()],
        allow_network: false,
    });
    assert!(profile.contains("(deny network*)") && profile.contains("(deny file-write*)"));
    assert!(profile.contains(&canonical(&dir).to_string_lossy().into_owned()));
    assert!(profile.contains("\"/private/tmp\""));
    let online = seatbelt_profile(&Policy {
        writable: vec![],
        allow_network: true,
    });
    assert!(!online.contains("(deny network*)"));
    // 路径里的引号与反斜杠被转义，不会提前结束字符串。
    assert_eq!(sb_string(Path::new("/a\"b\\c")), "\"/a\\\"b\\\\c\"");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 真正套上沙箱跑一条命令：工作区里能写，别处不能写，不能联网。没有沙箱的机器上跳过。
#[test]
fn a_sandboxed_command_cannot_write_outside_the_workspace() {
    if detect().kind == "none" {
        eprintln!("skipped: no sandbox tool on this machine");
        return;
    }
    let root = std::env::temp_dir().join(format!("colm-sbx-run-{}", std::process::id()));
    let inside = root.join("ws");
    std::fs::create_dir_all(&inside).unwrap();
    // 在 $HOME 下找一个沙箱应该拒绝写的位置（不是临时目录，也不是工作区）。
    let forbidden = home()
        .unwrap()
        .join(format!(".colm-sbx-probe-{}", std::process::id()));
    let script = format!(
        "echo ok > {inside}/a && echo inside=ok; (echo no > {forbidden} 2>/dev/null && echo outside=WRITTEN) || echo outside=denied",
        inside = inside.display(),
        forbidden = forbidden.display()
    );
    let (mut command, info) = wrap(
        Path::new("/bin/sh"),
        &["-c".into(), script],
        &Policy {
            writable: vec![inside.clone()],
            allow_network: false,
        },
    );
    assert!(info.network_blocked);
    let output = command.output().unwrap();
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains("inside=ok"),
        "{text} {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains("outside=denied"), "{text}");
    assert!(!forbidden.exists());
    assert!(inside.join("a").is_file());
    let _ = std::fs::remove_dir_all(&root);
}

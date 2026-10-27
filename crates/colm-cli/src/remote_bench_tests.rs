use super::*;

fn request(config: &str) -> RunRequest {
    RunRequest {
        name: "versions_vs_flux".into(),
        config: config.into(),
        sites: vec![Site {
            id: "AT_Neu".into(),
            station: "AT-Neu".into(),
            history_dir: "/data/x y/out/AT-Neu/history".into(),
            prefix: "AT-Neu_hist_".into(),
        }],
        ref_root: Some("/data/ref".into()),
    }
}

/// 生成的作业脚本整份交给 `bash -n` 查语法：作业体是拼出来的 shell，引号或 here-document 出错要在这里暴露。
#[cfg(unix)]
fn assert_bash_parses(body: &str) {
    use std::io::Write;
    let script = job::job_script("/srv/colm", "bench-run-1", body, &Spec::bare()).unwrap();
    let mut child = std::process::Command::new("bash")
        .arg("-n")
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{script}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_run_job_writes_the_config_merges_sites_then_checks_and_runs() {
    let config = "general:\n  name: \"versions_vs_flux\"\n  note: it's $HOME `x`\n";
    let body = run_body("/srv/colm/openbench", &request(config)).unwrap();
    // 配置原样落盘：带引号、`$`、反引号的行都在 here-document 里，不被展开。
    assert!(body.contains("note: it's $HOME `x`"), "{body}");
    assert!(body.contains("<<'COLM_BENCH_CONFIG_"), "{body}");
    // 环境与本机相同：OPENBENCH_HOME 改指到 home/。
    assert!(
        body.contains("export OPENBENCH_HOME='/srv/colm/openbench/home'"),
        "{body}"
    );
    assert!(
        body.contains("export OPENBENCH_REF_ROOT='/data/ref'"),
        "{body}"
    );
    // 带空格的服务器路径被引起来。
    assert!(body.contains("'/data/x y/out/AT-Neu/history'"), "{body}");
    let check = body.find(" check openbench.yaml").unwrap();
    let run = body.find(" run openbench.yaml").unwrap();
    let merge = body.find("合并 AT_Neu 的 history").unwrap();
    assert!(merge < check && check < run);
    #[cfg(unix)]
    assert_bash_parses(&body);
}

#[test]
fn the_run_job_refuses_unsafe_names_paths_and_terminators() {
    let root = "/srv/colm/openbench";
    let mut bad = request("x");
    bad.name = "../escape".into();
    assert!(run_body(root, &bad).is_err());
    let mut bad = request("x");
    bad.sites[0].station = "AT,Neu".into();
    assert!(run_body(root, &bad).is_err());
    let mut bad = request("x");
    bad.sites[0].history_dir = "relative/history".into();
    assert!(run_body(root, &bad).is_err());
    let mut bad = request("x");
    bad.sites[0].prefix = "a*b_hist_".into();
    assert!(run_body(root, &bad).is_err());
    let mut bad = request("x");
    bad.ref_root = Some("data/ref".into());
    assert!(run_body(root, &bad).is_err());
}

#[test]
fn the_uv_install_verifies_the_pinned_checksum_before_unpacking() {
    let (archive, sha) = uv_archive("linux-x86_64").unwrap();
    let body = install_uv_body("/srv/colm/openbench", archive, sha);
    let verify = body.find(sha).unwrap();
    let unpack = body.find("tar -xzf").unwrap();
    assert!(verify < unpack, "{body}");
    assert!(body.contains(&format!("releases/download/{UV_VERSION}/{archive}")));
    assert!(body.contains("sha256sum -c -") && body.contains("shasum -a 256 -c -"));
    #[cfg(unix)]
    assert_bash_parses(&body);
}

#[test]
fn registration_passes_every_field_quoted() {
    let request = Register {
        name: "OpenBench_LH_Daily".into(),
        root_dir: "/data/ref/LH dir".into(),
        data_type: "stn".into(),
        tim_res: "Day".into(),
        years: (1996, 2022),
        category: None,
        fulllist: Some("/data/ref/list.csv".into()),
        variables: vec!["Latent_Heat:LE:W m-2".into()],
    };
    let script = register_script("/srv/colm/openbench", &request).unwrap();
    assert!(script.contains("--root-dir '/data/ref/LH dir'"), "{script}");
    assert!(script.contains("-v 'Latent_Heat:LE:W m-2'"), "{script}");
    assert!(script.contains("--years 1996 2022"), "{script}");
    let mut bad = request;
    bad.data_type = "grid; rm -rf /".into();
    assert!(register_script("/srv/colm/openbench", &bad).is_err());
}

#[test]
fn server_status_lines_become_the_same_shape_as_the_local_status() {
    let text = "uname=Linux x86_64\ncurl=/usr/bin/curl\nsha=/usr/bin/sha256sum\nuv=0.12.5\npython=Python 3.12.14\nopenbench=3.0.8\nproject=a|/srv/ob/projects/a/output/a/reports/evaluation_report_standalone.html\nproject=b|\n";
    let value = parse_status("/srv/ob", text);
    assert_eq!(value["platform"], "linux-x86_64");
    assert_eq!(value["uv_version"], "0.12.5");
    assert_eq!(value["openbench"], "3.0.8");
    assert_eq!(value["openbench_pinned"], OPENBENCH_VERSION);
    assert_eq!(value["projects"][0]["name"], "a");
    assert!(value["projects"][1]["report"].is_null());
    // 什么都没装：版本都是空的，平台照报。
    let empty = parse_status("/srv/ob", "uname=Linux aarch64\ncurl=\nsha=\n");
    assert!(empty["uv_version"].is_null() && empty["downloader"].is_null());
    assert_eq!(empty["platform"], "linux-aarch64");
}

#[test]
fn uninstall_removes_only_the_named_folders_under_the_environment_root() {
    let script = uninstall_script("/srv/colm/openbench", &["env", "tools"]);
    assert!(
        script.contains("rm -rf '/srv/colm/openbench/env' '/srv/colm/openbench/tools'"),
        "{script}"
    );
    // 登记表与比较不在其中。
    assert!(
        !script.contains("/home") && !script.contains("/projects"),
        "{script}"
    );
    #[cfg(unix)]
    assert_bash_parses(&script);
}

#[test]
fn the_openbench_install_prefetches_basemaps_for_offline_compute_nodes() {
    let body = python_here("/srv/ob", PREFETCH_SCRIPT, &[], "COLM_BENCH_PREFETCH");
    assert!(body.contains("natural_earth"), "{body}");
    #[cfg(unix)]
    assert_bash_parses(&body);
}

#[test]
fn a_model_variable_mapping_refuses_colons_and_trims_fields() {
    assert_eq!(
        model_variable_spec(" Latent_Heat", "f_lfevpa ", "W m-2").unwrap(),
        "Latent_Heat:f_lfevpa:W m-2"
    );
    for (variable, varname, unit) in [
        ("Latent_Heat", "f:lfevpa", "W m-2"),
        ("Latent_Heat", "f_lfevpa", "W m-2:x"),
        ("", "f_lfevpa", "W m-2"),
        ("Latent_Heat", "f_lfevpa", "W\nm-2"),
    ] {
        assert!(
            model_variable_spec(variable, varname, unit).is_err(),
            "{variable:?} {varname:?} {unit:?}"
        );
    }
}

// ---- GUI-07 服务器侧：同名比较的活动作业占位 ----

fn job_status(state: job::State) -> job::Status {
    job::Status {
        state,
        phase: None,
        detail: None,
        log_tail: String::new(),
    }
}

#[test]
fn an_unfinished_comparison_job_has_one_stable_refusal_prefix() {
    let message = active_job_error("bench-run-84692-1792749735388362000");
    assert_eq!(
        message,
        "这个比较还有未完成的评估作业（作业号 bench-run-84692-1792749735388362000）；\
         先核对终态或停止它，避免重复写同一个成果目录。"
    );
    // 前端按这个前缀把中文原句映射成英文提示；改字要同步改比较页的映射。
    assert!(
        message.starts_with("这个比较还有未完成的评估作业（作业号 "),
        "{message}"
    );
}

#[test]
fn only_a_confirmed_terminal_job_frees_the_stale_placeholder() {
    // 排队中/运行中：拒绝重复提交，错误里带的就是占位记的作业号。
    for state in [job::State::Queued, job::State::Running] {
        let status = job_status(state);
        assert_eq!(
            active_job_refusal("bench-run-1", &status, false).as_deref(),
            Some(active_job_error("bench-run-1").as_str()),
            "{:?}",
            status.state
        );
    }
    // 已确认终态（含调度系统记下的 lost / cancelled / failed）→ 陈旧占位可以清掉重新提交。
    for state in [
        job::State::Finished { exit_code: 0 },
        job::State::Finished { exit_code: 2 },
        job::State::Lost,
    ] {
        let status = job_status(state);
        assert_eq!(
            active_job_refusal("bench-run-1", &status, true),
            None,
            "{:?}",
            status.state
        );
        assert_eq!(
            active_job_refusal("bench-run-1", &status, false),
            None,
            "{:?}",
            status.state
        );
    }
    // Unknown 不能一律当终态：作业目录还在，可能只是这次查不到，宁可拒绝。
    assert!(active_job_refusal("bench-run-1", &job_status(job::State::Unknown), true).is_some());
    // 作业目录都没有：占位是提交在 mkdir 与 submit 之间崩掉的残留，可以清掉。
    assert_eq!(
        active_job_refusal("bench-run-1", &job_status(job::State::Unknown), false),
        None
    );
}

#[test]
fn a_comparison_folder_is_the_only_place_a_placeholder_can_live() {
    assert_eq!(
        project_dir("/srv/colm/openbench", "versions_vs_flux").unwrap(),
        "/srv/colm/openbench/projects/versions_vs_flux"
    );
    for bad in ["", "../escape", "a/b", "a b", "名字"] {
        assert!(project_dir("/srv/colm/openbench", bad).is_err(), "{bad:?}");
    }
}

#[test]
fn the_claim_script_takes_the_slot_atomically_and_reports_what_it_found() {
    let project = "/srv/colm/openbench/projects/versions_vs_flux";
    let script = acquire_script(project, "bench-run-1");
    // `mkdir` 是原子的：只有一个提交能建成，建成后才写作业号并读回核对（与清陈旧占位并发时，
    // 一方可能删掉另一方刚建的目录，读回验证让两边至多一方报 acquired）。
    let mkdir = script
        .find("if mkdir '/srv/colm/openbench/projects/versions_vs_flux/.active-job'")
        .unwrap();
    let write = script.find("printf '%s\\n' 'bench-run-1' >").unwrap();
    let verify = script.find("[ \"$(cat").unwrap();
    assert!(mkdir < write && write < verify, "{script}");
    assert!(script.contains("echo acquired"), "{script}");
    assert!(script.contains("echo lost"), "{script}");
    assert!(script.contains("printf 'busy %s\\n'"), "{script}");
    assert!(script.contains("echo empty"), "{script}");
    assert!(
        script.contains("mkdir -p '/srv/colm/openbench/projects/versions_vs_flux'"),
        "{script}"
    );
    #[cfg(unix)]
    assert_bash_parses(&script);
}

#[test]
fn the_release_command_only_removes_the_placeholder_it_owns() {
    let script = release_script("/srv/colm dir/openbench/projects/x y", "bench-run-1");
    assert!(
        script.contains(
            "if [ \"$(cat '/srv/colm dir/openbench/projects/x y/.active-job'/id 2>/dev/null)\" = 'bench-run-1' ]; then rm -rf '/srv/colm dir/openbench/projects/x y/.active-job'; fi"
        ),
        "{script}"
    );
    // 先核对作业号再删：上一轮作业收尾时不能删掉新一轮提交占的位。
    assert!(script.contains("= 'bench-run-1'"), "{script}");
    #[cfg(unix)]
    assert_bash_parses(&script);
}

#[test]
fn the_run_job_releases_its_placeholder_on_exit() {
    let root = "/srv/colm dir/openbench";
    let project = project_dir(root, "versions_vs_flux").unwrap();
    let body = format!(
        "{}{}",
        active_job_trap(&project, "bench-run-7"),
        run_body(root, &request("x")).unwrap()
    );
    // trap 必须在作业体第一行、真正干活之前装好：评估失败或取消时也要释放占位。
    assert!(body.starts_with("trap '"), "{body}");
    let trap = body.find(" EXIT\n").unwrap();
    let first_work = body.find("test -x").unwrap();
    assert!(trap < first_work, "{body}");
    assert!(body.contains("bench-run-7"), "{body}");
    assert!(body.contains(".active-job"), "{body}");
    // 根目录带空格时，trap 里那层单引号套单引号也必须能过 bash。
    #[cfg(unix)]
    assert_bash_parses(&body);
}

#[test]
fn placeholder_output_is_parsed_strictly() {
    // 远程脚本的 stdout 可能带环境准备的杂音，取最后一个非空行。
    assert_eq!(
        parse_slot("module: loaded\nacquired\n\n"),
        Some(Slot::Acquired)
    );
    assert_eq!(
        parse_slot("busy bench-run-2\n"),
        Some(Slot::Busy("bench-run-2".into()))
    );
    assert_eq!(parse_slot("empty\n"), Some(Slot::Empty));
    // 看不懂的输出不能当成占住：否则会白白提交一份、写同一个成果目录。
    assert_eq!(parse_slot("something else\n"), None);
    assert_eq!(parse_slot(""), None);
    assert_eq!(parse_probe("none\n"), Some(None));
    assert_eq!(parse_probe("empty\n"), Some(None));
    assert_eq!(
        parse_probe("busy bench-run-3\n"),
        Some(Some("bench-run-3".into()))
    );
    assert_eq!(parse_probe("?"), None);
}

#[test]
fn the_probe_used_before_fetching_never_mutates_the_placeholder() {
    let script = active_job_probe_script("/srv/colm/openbench/projects/x");
    assert!(
        script.contains("echo none") && script.contains("printf 'busy %s\\n'"),
        "{script}"
    );
    // 取回是只读动作：创建、删除、改写占位都会直接破坏活动作业保护。
    for forbidden in ["mkdir", "rm ", "> '"] {
        assert!(!script.contains(forbidden), "{forbidden} in {script}");
    }
    #[cfg(unix)]
    assert_bash_parses(&script);
}

#[test]
fn a_freshly_claimed_slot_is_not_mistaken_for_a_crash_leftover() {
    let script = placeholder_expired_script("/srv/colm openbench/projects/x");
    // 只看占位目录的修改时间：刚占住、作业还没提交出去的窗口里，并发的另一份提交不能清掉它。
    assert!(
        script.contains("find '/srv/colm openbench/projects/x/.active-job' -maxdepth 0 -mmin +1")
    );
    assert!(
        script.contains("echo stale") && script.contains("echo fresh"),
        "{script}"
    );
    #[cfg(unix)]
    assert_bash_parses(&script);
}

/// 在真实 bash 下跑一遍占位脚本（临时目录，不连 SSH）：证明 `mkdir` 原子占位、重复提交读到原
/// 作业号、释放只删自己的作业号、释放后能重新占位。之前只有"脚本字符串 + bash -n + 决策纯函数"
/// 的证据，这一条把脚本本身真的执行到磁盘上（交付审计提出的可选提升）。
#[cfg(unix)]
#[test]
fn the_active_job_scripts_really_claim_and_release_on_disk() {
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    let base = std::env::temp_dir().join(format!(
        "colm-bench-active-job-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("projects")).unwrap();
    let project = base.join("projects").join("versions_vs_flux");
    let project_text = project.to_string_lossy().into_owned();
    let run = |script: &str| {
        let output = Command::new("bash").arg("-c").arg(script).output().unwrap();
        (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };

    let first = run(&acquire_script(&project_text, "bench-run-1"));
    assert!(first.0, "{}", first.2);
    assert_eq!(first.1, "acquired");
    assert_eq!(
        std::fs::read_to_string(project.join(".active-job/id"))
            .unwrap()
            .trim(),
        "bench-run-1"
    );

    // 另一份提交拿同一个比较：看到占位里原作业号，并且不能覆盖它。
    let second = run(&acquire_script(&project_text, "bench-run-2"));
    assert_eq!(second.1, "busy bench-run-1");
    assert_eq!(
        std::fs::read_to_string(project.join(".active-job/id"))
            .unwrap()
            .trim(),
        "bench-run-1"
    );

    // 释放只认自己的作业号：别人的 release 不能删掉占位。
    let foreign = run(&release_script(&project_text, "bench-run-2"));
    assert!(foreign.0, "{}", foreign.2);
    assert!(
        project.join(".active-job").exists(),
        "a foreign release must not remove the claim"
    );
    let own = run(&release_script(&project_text, "bench-run-1"));
    assert!(own.0, "{}", own.2);
    assert!(
        !project.join(".active-job").exists(),
        "the owner release removes the claim"
    );

    // 释放后可以重新占位；取回前的只读探针分别看到 none 与 busy <作业号>。
    assert_eq!(run(&active_job_probe_script(&project_text)).1, "none");
    assert_eq!(
        run(&acquire_script(&project_text, "bench-run-3")).1,
        "acquired"
    );
    assert_eq!(
        run(&active_job_probe_script(&project_text)).1,
        "busy bench-run-3"
    );
    // 作业体退出时执行的 trap 命令同样只删自己的占位。
    let trap = release_placeholder_command(&project_text, "bench-run-3");
    assert_eq!(run(&trap).1, "");
    assert!(!project.join(".active-job").exists());

    let _ = std::fs::remove_dir_all(&base);
}

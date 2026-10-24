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

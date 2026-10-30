use super::*;

#[test]
fn every_supported_platform_has_one_pinned_uv_archive() {
    let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    let (archive, sha) = uv_archive(&platform).unwrap();
    assert!(archive.starts_with("uv-"));
    assert_eq!(sha.len(), 64);
    assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
    let mut keys = UV_ARCHIVES
        .iter()
        .map(|(key, _, _)| *key)
        .collect::<Vec<_>>();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), UV_ARCHIVES.len());
}

#[test]
fn server_uname_maps_onto_the_archive_table() {
    for (uname, platform) in [
        ("Linux x86_64\n", "linux-x86_64"),
        ("Linux aarch64", "linux-aarch64"),
        ("Darwin arm64", "macos-aarch64"),
        ("Darwin x86_64", "macos-x86_64"),
    ] {
        assert_eq!(platform_from_uname(uname), Some(platform), "{uname}");
        assert!(uv_archive(platform).is_some(), "{platform}");
    }
    // 服务器上只有 Linux 与 macOS；Windows 的 uv 是 zip，服务器路径也不是 POSIX。
    assert_eq!(platform_from_uname("Linux ppc64le"), None);
    assert_eq!(platform_from_uname(""), None);
}

#[test]
fn local_and_server_environments_point_at_the_same_places() {
    let home = PROGRAM_ENV
        .iter()
        .find(|(key, _)| *key == "OPENBENCH_HOME")
        .unwrap()
        .1;
    assert_eq!(posix_value("/srv/ob/", home), "/srv/ob/home");
    let cache = UV_ENV
        .iter()
        .find(|(key, _)| *key == "UV_PYTHON_BIN_DIR")
        .unwrap()
        .1;
    assert_eq!(posix_value("/srv/ob", cache), "/srv/ob/python/bin");
    assert_eq!(
        std::path::PathBuf::from(local_value(std::path::Path::new("/a/ob"), cache)),
        std::path::Path::new("/a/ob").join("python").join("bin")
    );
    assert_eq!(posix_value("/x", EnvValue::Fixed("1")), "1");
    // 每个变量只出现一次。
    let mut keys = UV_ENV
        .iter()
        .chain(PROGRAM_ENV.iter())
        .map(|(key, _)| *key)
        .collect::<Vec<_>>();
    keys.sort_unstable();
    let n = keys.len();
    keys.dedup();
    assert_eq!(keys.len(), n);
}

#[test]
fn names_and_stations_stay_safe_in_paths_and_csv() {
    assert!(valid_name("AT-Neu_vs_draft2"));
    for bad in ["", "../x", "a/b", "a b", "名字"] {
        assert!(!valid_name(bad), "{bad:?}");
    }
    assert!(valid_station("AT-Neu"));
    for bad in ["", "a,b", "a\"b", "a\nb"] {
        assert!(!valid_station(bad), "{bad:?}");
    }
}

#[test]
fn python_style_infinity_and_nan_become_null_outside_strings() {
    let text = r#"[{"max_uparea": Infinity, "low": -Infinity, "x": [NaN, 1], "note": "NaN, Infinity stay"}]"#;
    let value: serde_json::Value = serde_json::from_str(&strict_json(text)).unwrap();
    assert!(value[0]["max_uparea"].is_null());
    assert!(value[0]["low"].is_null());
    assert!(value[0]["x"][0].is_null());
    assert_eq!(value[0]["x"][1], 1);
    assert_eq!(value[0]["note"], "NaN, Infinity stay");
}

#[test]
fn only_plain_http_index_urls_are_accepted() {
    assert!(valid_index_url("https://pypi.tuna.tsinghua.edu.cn/simple"));
    assert!(valid_index_url("http://mirror.local/pypi/simple"));
    for bad in [
        "",
        "ftp://x",
        "https://a b",
        "https://x'; rm",
        "file:///etc",
        "https://x\n",
    ] {
        assert!(!valid_index_url(bad), "{bad:?}");
    }
}

#[test]
fn embedded_bench_scripts_stay_plain_lf_python() {
    // 两个脚本由 include_str! 从 src/scripts/*.py 嵌进来：远程 heredoc、本机 `python -c`、
    // scripts/test-bench-scripts.py 跑的是同一份。它们不是 Rust 代码，rustfmt 管不到，这里守住
    // 最基本的三条：非空、LF（逐字节比较的规矩）、可直接当 Python 源执行。
    for (name, script) in [("inspect", INSPECT_SCRIPT), ("prepare", PREPARE_SCRIPT)] {
        assert!(script.len() > 1000, "{name} script looks truncated");
        assert!(script.starts_with("#!/usr/bin/env python3"), "{name}");
        assert!(script.ends_with('\n'), "{name}");
        assert!(!script.contains('\r'), "{name} must stay LF-only");
    }
    // 这些 JSON 字段与参数位置是窗口进程/CLI 与脚本之间的契约，改名要两侧一起改。
    // tim_res_source 现在可能是 time/metadata/filename，tim_res_metadata 是新增的元数据值：
    // 前端按这两个字段提示"分辨率来源"和"元数据与实测冲突"，缺一个就退化成静默。
    // layered_variables 是带 soilsnow/soil/lake 层维度的变量（制备会保留维度，OpenBench CoLM2024
    // profile 的 compute 直接读），前端据此提示层处理；它和 unsupported_variables（制备会跳过）
    // 是两种不同语义，字段名丢了会让层变量重新被当成"映射就会丢"。
    for field in [
        "spatial_type",
        "unsupported_variables",
        "layered_variables",
        "crosses_dateline",
        "families",
        "tim_res_source",
        "tim_res_metadata",
    ] {
        assert!(
            INSPECT_SCRIPT.contains(&format!("\"{field}\"")),
            "inspect.py no longer reports {field}"
        );
    }
    // 层维度保留口径必须两边一致：inspect 报 layered_variables、prepare 保留同名维度，任何一边
    // 改回"全跳过"都会让 OpenBench profile compute 读不到 f_wliq_soisno 这类变量。
    for dim in ["soilsnow", "soil", "lake"] {
        assert!(
            INSPECT_SCRIPT.contains(&format!("\"{dim}\"")),
            "inspect.py no longer knows the {dim} layer dimension"
        );
        assert!(
            PREPARE_SCRIPT.contains(&format!("\"{dim}\"")),
            "prepare.py no longer keeps the {dim} layer dimension"
        );
    }
    assert!(
        PREPARE_SCRIPT.contains("[LON] [LAT]"),
        "prepare.py usage changed"
    );
}

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

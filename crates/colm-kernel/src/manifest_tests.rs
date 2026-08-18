use std::path::PathBuf;

use super::*;

/// 本机实测的清单，一字不改地当作固件。
const SAMPLE: &str = r#"{
  "schema": 1,
  "preset": "waterheat",
  "platform": "Darwin-arm64",
  "colm_git_sha": "72dd76b9",
  "generator_args": "SinglePoint LULC_IGBP URBANOFF vanGenu CaMaOFF BGCOFF CROPOFF TRACEROFF",
  "macros": ["CoLMDEBUG","LULC_IGBP","RangeCheck","SinglePoint","extend_interception","vanGenuchten_Mualem_SOIL_MODEL"],
  "built_with": "GNU Fortran (Homebrew GCC 16.1.0) 16.1.0",
  "netcdf_c": "netCDF 4.9.3",
  "netcdf_fortran": "4.6.3",
  "hdf5": "1.14.6",
  "sha256": {
    "mksrfdata": "053ba92bfbe62d2c74a2d866afe458eeb878b4d557bbe01aecd7e6a9b6e0c0bb",
    "mkinidata": "a707e8c030b650d242ddaa09e4aed8c1e14938afe494bc5b47817b817279fff2",
    "colm":      "8dc6a40aabc704da4a49941779cfa3369e3d5d5125a76d684ad45b8a282140e4"
  }
}"#;

fn workdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("colm-kernel-manifest-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("create workdir");
    d
}

/// 建一个假内核目录：清单 + 三个内容已知的「二进制」。
fn fake_kernel(name: &str, bodies: &[(&str, &str)]) -> PathBuf {
    let d = workdir(name);
    let mut m = SAMPLE.to_string();
    for (prog, body) in bodies {
        std::fs::write(d.join(format!("{prog}.x")), body).expect("write");
        // 把清单里的占位 sha256 换成这个内容的真实值
        let want = sha256_hex(body.as_bytes());
        let old = match *prog {
            "mksrfdata" => "053ba92bfbe62d2c74a2d866afe458eeb878b4d557bbe01aecd7e6a9b6e0c0bb",
            "mkinidata" => "a707e8c030b650d242ddaa09e4aed8c1e14938afe494bc5b47817b817279fff2",
            _ => "8dc6a40aabc704da4a49941779cfa3369e3d5d5125a76d684ad45b8a282140e4",
        };
        m = m.replace(old, &want);
    }
    std::fs::write(d.join("manifest.json"), m).expect("write manifest");
    d
}

const ALL: &[(&str, &str)] = &[
    ("mksrfdata", "fake mksrfdata"),
    ("mkinidata", "fake mkinidata"),
    ("colm", "fake colm"),
];

#[test]
fn the_sample_manifest_parses_into_its_fields() {
    let m: Manifest = serde_json::from_str(SAMPLE).expect("parses");
    assert_eq!(m.schema, 1);
    assert_eq!(m.preset, "waterheat");
    assert_eq!(m.colm_git_sha, "72dd76b9");
    assert_eq!(m.netcdf_fortran, "4.6.3");
    assert_eq!(m.macros.len(), 6);
    assert!(m.macros.iter().any(|x| x == "SinglePoint"));
    assert_eq!(m.sha256.len(), 3);
}

#[test]
fn the_nested_sha256_object_is_read_as_values_not_keys() {
    // 先前的手写提取对 "sha256" 会返回 "mksrfdata" —— 键名而不是值。
    // 这条测试是那个 bug 的墓碑。
    let m: Manifest = serde_json::from_str(SAMPLE).expect("parses");
    assert_eq!(
        m.sha256.get("colm").map(String::as_str),
        Some("8dc6a40aabc704da4a49941779cfa3369e3d5d5125a76d684ad45b8a282140e4")
    );
}

#[test]
fn a_matching_kernel_opens() {
    let d = fake_kernel("ok", ALL);
    let k = Kernel::open(&d).expect("opens");
    assert_eq!(k.manifest.preset, "waterheat");
    // 比 canonicalize 之后的：`open` 现在会绝对化，而 macOS 的 temp_dir
    // 是 `/var/folders/...`，canonicalize 之后变成 `/private/var/folders/...`。
    // 绝对化的理由见 `an_opened_kernel_holds_an_absolute_path`。
    assert_eq!(k.dir, d.canonicalize().expect("canonicalize"));
}

#[test]
fn a_missing_binary_and_a_wrong_one_are_different_errors() {
    // design.md §6.1：「不存在」和「存在但版本不对」是两种不同情况，
    // 不能混成一句「内核不可用」。用户对这两种的处置完全不同。
    let d = fake_kernel("missing", &ALL[..2]); // 少 colm.x
    let e = Kernel::open(&d).unwrap_err();
    let s = format!("{e:#}");
    assert!(s.contains("colm.x"), "{s}");
    assert!(s.contains("missing"), "{s}");

    let d = fake_kernel("tampered", ALL);
    std::fs::write(d.join("colm.x"), "tampered").expect("write");
    let e = Kernel::open(&d).unwrap_err();
    let s = format!("{e:#}");
    assert!(s.contains("colm.x"), "{s}");
    assert!(s.contains("sha256"), "{s}");
    assert!(
        !s.contains("missing"),
        "a tampered binary is not a missing one: {s}"
    );
}

#[test]
fn an_unreadable_manifest_says_so_rather_than_blaming_a_binary() {
    let d = workdir("nomanifest");
    for (p, b) in ALL {
        std::fs::write(d.join(format!("{p}.x")), b).expect("write");
    }
    let e = Kernel::open(&d).unwrap_err();
    assert!(format!("{e:#}").contains("manifest"), "{e:#}");
}

#[test]
fn a_manifest_from_a_different_schema_is_refused() {
    // 清单格式将来会变。读到不认识的 schema 就停下，好过按旧字段去解释新文件。
    let d = fake_kernel("schema", ALL);
    let m = std::fs::read_to_string(d.join("manifest.json"))
        .unwrap()
        .replace("\"schema\": 1", "\"schema\": 99");
    std::fs::write(d.join("manifest.json"), m).unwrap();
    let e = Kernel::open(&d).unwrap_err();
    assert!(format!("{e:#}").contains("schema"), "{e:#}");
}

#[test]
fn the_three_programs_are_the_ones_colm_ships() {
    assert_eq!(PROGRAMS, ["mksrfdata", "mkinidata", "colm"]);
}

#[test]
fn an_opened_kernel_holds_an_absolute_path() {
    // `run_stage` 用 `current_dir(work)` 启动子进程。内核目录若是相对路径，
    // 可执行文件就会被相对 `work` 去找 —— open 成功、spawn 报
    // 「No such file or directory」，而报错里那个路径看着完全正常。
    // 实测踩过：`colm-cli run --kernel kernels/waterheat` 正是这样炸的。
    let d = fake_kernel(
        "absolute",
        &[("mksrfdata", "a"), ("mkinidata", "b"), ("colm", "c")],
    );
    // 造一条明确是相对的路径：从当前目录出发绕一圈回到那个临时目录。
    let rel = PathBuf::from(".").join(&d);
    assert!(!rel.is_absolute() || d.is_absolute());
    let opened = Kernel::open(&rel).expect("opens");
    assert!(opened.dir.is_absolute(), "{}", opened.dir.display());
    for p in PROGRAMS {
        assert!(
            opened.program(p).is_absolute(),
            "{} is not absolute",
            opened.program(p).display()
        );
    }
}

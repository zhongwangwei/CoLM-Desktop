//! 预热、输出和网格可批量写；逐站点基本设定与过程参数由下拉菜单选择站点或全部。

use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("repo root")
}

#[test]
fn all_frontend_writes_still_go_through_validated_backend_commands() {
    // `write_text` 已经从后端删掉、`set_field` 已经从命令表摘掉，那条路只写得动一个文件。
    // 前端调用了未注册的命令会被 `check-gui` 拦下，所以这里只守后端命令表。
    let lib = std::fs::read_to_string(root().join("gui/src-tauri/src/lib.rs")).expect("lib.rs");
    for bad in ["            set_field,", "            write_text,"] {
        assert!(!lib.contains(bad), "命令表里又出现了单份写入：{bad}");
    }
}

#[test]
fn parameter_presets_are_removed_instead_of_left_half_wired() {
    let root = root();
    assert!(!root.join("gui/src-tauri/src/presets.rs").exists());
    let lib = std::fs::read_to_string(root.join("gui/src-tauri/src/lib.rs")).unwrap();
    for removed in [
        "save_preset",
        "list_presets",
        "delete_preset",
        "apply_preset",
    ] {
        assert!(!lib.contains(removed), "参数预设后端仍残留：{removed}");
    }
}

#[test]
fn the_installer_carries_a_runnable_example() {
    // 一个刚装好程序的人手上没有任何数据。PLUMBER2 要注册、几十 GB ——
    // 在拿到数据之前他连「这程序能不能用」都判断不了。
    let conf = std::fs::read_to_string(root().join("gui/src-tauri/tauri.bundle.conf.json"))
        .expect("tauri.bundle.conf.json");
    assert!(conf.contains("examples"), "示例数据没进 bundle.resources");

    // 自然站与甲烷站的三件套缺一不可：少了 Forcing 界面会说「没有强迫场，跑不了」，
    // 少了 Observation 则是评估按钮一直灰着 —— 都不像是打包漏了。
    for s in [
        "CN-Cng_2008-2009_FLUXNET2015",
        "AT-Neu_2010-2012_FLUXNET-CH4",
    ] {
        for (d, suf) in [
            ("Sitedata", "site"),
            ("Forcing", "Met"),
            ("Observation", "Flux"),
        ] {
            let p = root()
                .join("examples")
                .join(d)
                .join(format!("{s}_{suf}.nc"));
            assert!(p.is_file(), "{} 不在", p.display());
        }
    }
    assert!(root().join("examples/Forcingnml/AT-Neu.nml").is_file());

    // 目录形状就是 `colm-cli scan` 依赖的那个：它顺着命名约定从 Sitedata
    // 找到 ../Forcing 与 ../Observation。压平了扫描仍列得出站点，
    // 但强迫场找不到。
    let project = std::fs::read_to_string(root().join("gui/src/features/projects/ProjectPage.tsx"))
        .expect("ProjectPage.tsx");
    assert!(project.contains("install_example"), "界面上没有入口");

    // 四个站点保留完整观测（含 AT-Neu 水热），仍限制安装包的数据体积。
    let bytes: u64 = ["Sitedata", "Forcing", "Observation"]
        .iter()
        .flat_map(|d| std::fs::read_dir(root().join("examples").join(d)).unwrap())
        .map(|e| e.unwrap().metadata().unwrap().len())
        .sum();
    assert!(
        bytes < 16 * 1024 * 1024,
        "示例数据 {} MB，太大了 —— 重新 nccopy -d 5 压一遍",
        bytes / 1048576
    );
}

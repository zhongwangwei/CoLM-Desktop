use super::*;
use crate::layout::layout_tests::{source_repo, temp};

fn fresh(tag: &str) -> (PathBuf, Workspace) {
    let root = temp(tag);
    let repo = root.join("source");
    source_repo(&repo);
    let ws = Workspace::create(&root.join("ws"), "demo", repo.to_str().unwrap(), None).unwrap();
    (root, ws)
}

const MODULE: &str = "vendor/CoLM202X/main/MOD_Demo.F90";

#[test]
fn a_file_is_listed_read_edited_and_committed() {
    let (_root, ws) = fresh("files-edit");
    let top = list(&ws, "").unwrap();
    assert!(top.iter().any(|e| e.name == "vendor" && e.kind == "dir"));
    assert!(!top.iter().any(|e| e.name == ".git"));
    let main = list(&ws, "vendor/CoLM202X/main").unwrap();
    assert_eq!(main[0].path, MODULE);
    let before = read(&ws, MODULE).unwrap();
    let edited = before.text.unwrap().replace("k = 2.0", "k = 3.0");
    let applied = write_text(&ws, MODULE, &edited, "k is 3").unwrap();
    assert_eq!(applied.files, [MODULE]);
    assert_eq!(read(&ws, MODULE).unwrap().text.unwrap(), edited);
    assert!(!git::is_dirty(&ws.src()).unwrap());
    assert_eq!(ws.head().unwrap(), applied.commit);
    // Writing the same text again changes nothing and is said so.
    let error = write_text(&ws, MODULE, &edited, "again")
        .unwrap_err()
        .to_string();
    assert!(error.contains("nothing changed"), "{error}");
    // New files can be created in new folders.
    write_text(
        &ws,
        "vendor/CoLM202X/new/MOD_New.F90",
        "MODULE MOD_New\nEND MODULE\n",
        "new",
    )
    .unwrap();
    assert!(ws.src().join("vendor/CoLM202X/new/MOD_New.F90").is_file());
}

#[test]
fn whole_files_and_folders_are_replaced_and_binary_files_are_allowed() {
    let (root, ws) = fresh("files-replace");
    let table = root.join("table.nc");
    std::fs::write(&table, [0u8, 1, 2, 255, 0]).unwrap();
    put_file(
        &ws,
        "vendor/CoLM202X/data/lookup.nc",
        &table,
        "lookup table",
    )
    .unwrap();
    let read_back = read(&ws, "vendor/CoLM202X/data/lookup.nc").unwrap();
    assert_eq!(
        (read_back.size, read_back.text),
        (5, None),
        "binary is kept, not opened as text"
    );
    // A whole module folder replaced: the old file is gone, the new ones are in.
    let module = root.join("my-main");
    std::fs::create_dir_all(module.join("sub")).unwrap();
    std::fs::write(module.join("MOD_Mine.F90"), "MODULE MOD_Mine\nEND MODULE\n").unwrap();
    std::fs::write(module.join("sub/helper.F90"), "! helper\n").unwrap();
    put_dir(&ws, "vendor/CoLM202X/main", &module, "my physics").unwrap();
    assert!(!ws.src().join(MODULE).exists());
    assert!(ws
        .src()
        .join("vendor/CoLM202X/main/sub/helper.F90")
        .is_file());
    assert!(!git::is_dirty(&ws.src()).unwrap());
    remove(&ws, "vendor/CoLM202X/main/sub", "drop helper").unwrap();
    assert!(!ws.src().join("vendor/CoLM202X/main/sub").exists());
    assert!(!git::is_dirty(&ws.src()).unwrap());
}

#[test]
fn protected_paths_links_and_dirty_trees_are_refused() {
    // root 只在下面 Unix 才跑的符号链接检查里用到。
    #[cfg_attr(not(unix), allow(unused_variables))]
    let (root, ws) = fresh("files-refuse");
    for bad in [
        "oracle/golden/CN-Cng.nc",
        ".git/config",
        "../outside",
        "/etc/passwd",
    ] {
        assert!(write_text(&ws, bad, "x", "bad").is_err(), "{bad}");
    }
    assert!(
        write_text(&ws, MODULE, "x", "").is_err(),
        "a change needs a message"
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&root, ws.src().join("outside")).unwrap();
        git::run(&ws.src(), &["add", "outside"]).unwrap();
        git::run(&ws.src(), &["commit", "-q", "-m", "link"]).unwrap();
        let error = write_text(&ws, "outside/x.txt", "x", "through a link")
            .unwrap_err()
            .to_string();
        assert!(error.contains("symbolic link"), "{error}");
        assert!(!root.join("x.txt").exists());
        // A folder with links is not copied.
        let folder = root.join("with-link");
        std::fs::create_dir_all(&folder).unwrap();
        std::os::unix::fs::symlink("/etc", folder.join("etc")).unwrap();
        assert!(put_dir(&ws, "vendor/x", &folder, "links").is_err());
    }
    std::fs::write(ws.src().join(MODULE), "uncommitted\n").unwrap();
    let error = write_text(&ws, "other.txt", "x", "dirty")
        .unwrap_err()
        .to_string();
    assert!(error.contains("uncommitted"), "{error}");
}

#[test]
fn edits_made_with_outside_editors_are_snapshotted_against_the_official_source() {
    let (_root, ws) = fresh("files-external");
    assert!(
        snapshot_external(&ws).unwrap().is_none(),
        "nothing changed yet"
    );
    // VS Code 改了一个模块、新建了一个文件，还留下了它自己的设置和交换文件。
    let module = ws.src().join(MODULE);
    let text = std::fs::read_to_string(&module).unwrap();
    std::fs::write(&module, text.replace("k = 2.0", "k = 4.0")).unwrap();
    std::fs::create_dir_all(ws.src().join("vendor/CoLM202X/extra")).unwrap();
    std::fs::write(
        ws.src().join("vendor/CoLM202X/extra/MOD_X.F90"),
        "MODULE X\nEND MODULE\n",
    )
    .unwrap();
    std::fs::create_dir_all(ws.src().join(".vscode")).unwrap();
    std::fs::write(ws.src().join(".vscode/settings.json"), "{}").unwrap();
    std::fs::write(ws.src().join("vendor/CoLM202X/main/.MOD_Demo.F90.swp"), "x").unwrap();
    let applied = snapshot_external(&ws).unwrap().unwrap();
    let mut files = applied.files.clone();
    files.sort();
    assert_eq!(files, ["vendor/CoLM202X/extra/MOD_X.F90", MODULE]);
    assert!(!git::is_dirty(&ws.src()).unwrap());
    let changed = ws
        .changes_from_official()
        .unwrap()
        .into_iter()
        .map(|change| change.path)
        .collect::<Vec<_>>();
    assert!(changed.contains(&MODULE.to_string()), "{changed:?}");
    assert!(!changed
        .iter()
        .any(|path| path.contains(".vscode") || path.ends_with(".swp")));
    assert!(
        snapshot_external(&ws).unwrap().is_none(),
        "a second look finds nothing new"
    );
    // 黄金参考在外部被改了：拒绝记下，并说出是哪个文件。
    std::fs::create_dir_all(ws.src().join("oracle/golden")).unwrap();
    std::fs::write(ws.src().join("oracle/golden/answer.txt"), "changed").unwrap();
    let error = format!("{:#}", snapshot_external(&ws).unwrap_err());
    assert!(error.contains("oracle/golden/answer.txt"), "{error}");
}

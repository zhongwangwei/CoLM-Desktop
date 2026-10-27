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
    let baseline = before
        .hash
        .clone()
        .expect("editable text carries a content hash");
    let edited = before.text.unwrap().replace("k = 2.0", "k = 3.0");
    let written = write_text(&ws, MODULE, &edited, "k is 3", Some(&baseline)).unwrap();
    assert_eq!(written.applied.files, [MODULE]);
    assert_eq!(ws.head().unwrap(), written.applied.commit);
    let after = read(&ws, MODULE).unwrap();
    assert_eq!(after.text.unwrap(), edited);
    assert_eq!(
        after.hash.unwrap(),
        written.hash,
        "the hash returned by the write is the new baseline of the file on disk"
    );
    assert!(!git::is_dirty(&ws.src()).unwrap());
    // 序列化形状是前端契约的一部分：workspace_write_file 的结果带 hash（与 Applied 的字段平铺）。
    let json = serde_json::to_value(&written).unwrap();
    assert_eq!(json["hash"], written.hash, "{json}");
    assert!(
        json["commit"].is_string() && json["files"].is_array(),
        "{json}"
    );
    // 连续保存：用上一次写入返回的哈希当基线，第二次保存不能被误判成冲突。
    let second_text = edited.replace("k = 3.0", "k = 4.0");
    let second = write_text(&ws, MODULE, &second_text, "k is 4", Some(&written.hash)).unwrap();
    assert_eq!(read(&ws, MODULE).unwrap().hash.unwrap(), second.hash);
    // Writing the same text again changes nothing and is said so.
    let error = write_text(&ws, MODULE, &second_text, "again", Some(&second.hash))
        .unwrap_err()
        .to_string();
    assert!(error.contains("nothing changed"), "{error}");
    // New files can be created in new folders.
    let created = create_text(
        &ws,
        "vendor/CoLM202X/new/MOD_New.F90",
        "MODULE MOD_New\nEND MODULE\n",
        "new",
    )
    .unwrap();
    assert!(ws.src().join("vendor/CoLM202X/new/MOD_New.F90").is_file());
    assert_eq!(
        read(&ws, "vendor/CoLM202X/new/MOD_New.F90")
            .unwrap()
            .hash
            .unwrap(),
        created.hash
    );
}

#[test]
fn creating_a_file_never_replaces_an_existing_one() {
    let (_root, ws) = fresh("files-create");
    let before = read(&ws, MODULE).unwrap();
    let head = ws.head().unwrap();
    // 重名新建：原内容、哈希、提交状态都不变。
    let error = create_text(&ws, MODULE, "overwritten\n", "new")
        .unwrap_err()
        .to_string();
    assert!(error.starts_with("already-exists:"), "{error}");
    let after = read(&ws, MODULE).unwrap();
    assert_eq!(after.text, before.text);
    assert_eq!(after.hash, before.hash);
    assert_eq!(ws.head().unwrap(), head);
    assert!(!git::is_dirty(&ws.src()).unwrap());

    let created = create_text(
        &ws,
        "vendor/CoLM202X/main/MOD_Second.F90",
        "MODULE MOD_Second\nEND MODULE\n",
        "second",
    )
    .unwrap();
    assert_eq!(
        created.applied.files,
        ["vendor/CoLM202X/main/MOD_Second.F90"]
    );
    let head_after = ws.head().unwrap();
    assert_ne!(head_after, head);
    // 同名再建一次仍被拒；失败那次没有留下未提交内容。
    assert!(create_text(&ws, "vendor/CoLM202X/main/MOD_Second.F90", "x\n", "again").is_err());
    assert_eq!(ws.head().unwrap(), head_after);
    assert!(!git::is_dirty(&ws.src()).unwrap());
}

#[test]
fn concurrent_creates_of_the_same_file_have_at_most_one_winner() {
    let (_root, ws) = fresh("files-create-race");
    let head = ws.head().unwrap();
    let path = "vendor/CoLM202X/main/MOD_Race.F90";
    let barrier = std::sync::Barrier::new(2);
    // 真并发：两条线程同时走"检查工作区干净 → create_new → 提交"。create_new 是内核保证的
    // 原子"存在则失败"，所以最多一个成功；失败的那次既不覆盖内容也不留半成品。
    let workspace = &ws;
    let attempts = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for text in ["MODULE A\n", "MODULE B\n"] {
            let barrier = &barrier;
            handles.push(scope.spawn(move || {
                barrier.wait();
                create_text(workspace, path, text, "race")
            }));
        }
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        attempts.iter().filter(|attempt| attempt.is_ok()).count(),
        1,
        "exactly one concurrent create wins: {attempts:?}"
    );
    assert_eq!(git::commits_since(&ws.src(), &head).unwrap().len(), 1);
    assert!(!git::is_dirty(&ws.src()).unwrap());
    let content = read(&ws, path).unwrap().text.unwrap();
    assert!(
        content == "MODULE A\n" || content == "MODULE B\n",
        "{content:?}"
    );
}

#[test]
fn saving_never_overwrites_an_external_edit_even_after_a_snapshot_commit() {
    let (_root, ws) = fresh("files-conflict");
    let baseline = read(&ws, MODULE).unwrap().hash.unwrap();
    // 外部编辑器改了它：窗口回到前台时快照已把外部改动记成一次提交——界面里的提交列表因此变了，
    // 但文本框里还是旧内容。保存必须仍然被拒绝（GUI-03 的触发路径）。
    let module = ws.src().join(MODULE);
    let external = std::fs::read_to_string(&module)
        .unwrap()
        .replace("k = 2.0", "k = 9.0");
    std::fs::write(&module, &external).unwrap();
    let applied = snapshot_external(&ws).unwrap().unwrap();
    assert_eq!(applied.files, [MODULE]);
    let head_after_snapshot = ws.head().unwrap();
    let error = write_text(
        &ws,
        MODULE,
        "module edited in the GUI\n",
        "gui edit",
        Some(&baseline),
    )
    .unwrap_err()
    .to_string();
    assert!(error.starts_with("content-conflict:"), "{error}");
    assert_eq!(
        std::fs::read_to_string(&module).unwrap(),
        external,
        "a conflict writes nothing at all"
    );
    assert_eq!(
        ws.head().unwrap(),
        head_after_snapshot,
        "a conflict creates no commit"
    );
    // 用重新读到的基线可以保存：冲突处理完（重载/明确覆盖）之后保存要能继续。
    let fresh = read(&ws, MODULE).unwrap().hash.unwrap();
    let written = write_text(
        &ws,
        MODULE,
        "module edited in the GUI\n",
        "gui edit",
        Some(&fresh),
    )
    .unwrap();
    assert_eq!(read(&ws, MODULE).unwrap().hash.unwrap(), written.hash);
}

#[test]
fn an_external_delete_is_reported_and_recreating_is_explicit() {
    let (_root, ws) = fresh("files-deleted");
    let baseline = read(&ws, MODULE).unwrap().hash.unwrap();
    std::fs::remove_file(ws.src().join(MODULE)).unwrap();
    snapshot_external(&ws).unwrap();
    let error = write_text(&ws, MODULE, "back\n", "recreate", Some(&baseline))
        .unwrap_err()
        .to_string();
    assert!(error.starts_with("content-conflict:"), "{error}");
    assert!(
        !ws.src().join(MODULE).exists(),
        "a save based on a deleted file does not silently resurrect it"
    );
    assert!(!git::is_dirty(&ws.src()).unwrap());
    // 明确重建：不给基线才允许把文件建回来（界面上的"用我的文本重建"）。
    let written = write_text(&ws, MODULE, "back\n", "recreate", None).unwrap();
    assert_eq!(read(&ws, MODULE).unwrap().hash.unwrap(), written.hash);
    assert!(!git::is_dirty(&ws.src()).unwrap());
}

#[test]
fn adding_a_local_file_never_replaces_a_same_named_file() {
    let (root, ws) = fresh("files-put-create");
    let first = root.join("first.bin");
    std::fs::write(&first, [1u8, 2, 3]).unwrap();
    let second = root.join("second.bin");
    std::fs::write(&second, [9u8, 9]).unwrap();
    put_file(&ws, "vendor/CoLM202X/data/first.bin", &first, "add", true).unwrap();
    let head = ws.head().unwrap();
    let error = put_file(
        &ws,
        "vendor/CoLM202X/data/first.bin",
        &second,
        "add again",
        true,
    )
    .unwrap_err()
    .to_string();
    assert!(error.starts_with("already-exists:"), "{error}");
    assert_eq!(
        std::fs::read(ws.src().join("vendor/CoLM202X/data/first.bin")).unwrap(),
        [1, 2, 3]
    );
    assert_eq!(ws.head().unwrap(), head);
    assert!(!git::is_dirty(&ws.src()).unwrap());
    // 明确的"替换"仍然可用：二进制内容整个换掉。
    put_file(
        &ws,
        "vendor/CoLM202X/data/first.bin",
        &second,
        "replace",
        false,
    )
    .unwrap();
    assert_eq!(
        std::fs::read(ws.src().join("vendor/CoLM202X/data/first.bin")).unwrap(),
        [9, 9]
    );
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
        false,
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
        assert!(write_text(&ws, bad, "x", "bad", None).is_err(), "{bad}");
    }
    assert!(
        write_text(&ws, MODULE, "x", "", None).is_err(),
        "a change needs a message"
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&root, ws.src().join("outside")).unwrap();
        git::run(&ws.src(), &["add", "outside"]).unwrap();
        git::run(&ws.src(), &["commit", "-q", "-m", "link"]).unwrap();
        let error = write_text(&ws, "outside/x.txt", "x", "through a link", None)
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
    let error = write_text(&ws, "other.txt", "x", "dirty", None)
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

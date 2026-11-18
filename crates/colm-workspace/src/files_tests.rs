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

const LINK_TARGET: &str = "vendor/CoLM202X/include/Makeoptions.github";
const LINK_ALIAS: &str = "vendor/CoLM202X/include/Makeoptions";
const LINK_CHAIN: &str = "scripts/Makeoptions";

fn linked_workspace(tag: &str) -> (PathBuf, Workspace) {
    let root = temp(tag);
    let repo = root.join("source");
    source_repo(&repo);
    std::fs::write(repo.join("Cargo.toml"), "[workspace]\n").unwrap();
    for path in [LINK_TARGET, LINK_ALIAS, LINK_CHAIN] {
        std::fs::create_dir_all(repo.join(path).parent().unwrap()).unwrap();
        std::fs::write(repo.join(path), "FF=old\n").unwrap();
    }
    std::fs::write(
        repo.join(".colm-source-links.json"),
        serde_json::to_vec(&std::collections::BTreeMap::from([
            (LINK_ALIAS, "Makeoptions.github"),
            (LINK_CHAIN, "../vendor/CoLM202X/include/Makeoptions"),
        ]))
        .unwrap(),
    )
    .unwrap();
    git::run(&repo, &["add", "-A"]).unwrap();
    git::run(&repo, &["commit", "-q", "-m", "portable links"]).unwrap();
    let ws = Workspace::create(&root.join("ws"), "demo", repo.to_str().unwrap(), None).unwrap();
    (root, ws)
}

fn assert_linked_bytes(ws: &Workspace, expected: &str) {
    for path in [LINK_TARGET, LINK_ALIAS, LINK_CHAIN] {
        assert_eq!(
            std::fs::read_to_string(ws.src().join(path)).unwrap(),
            expected,
            "{path}"
        );
    }
    assert!(!git::is_dirty(&ws.src()).unwrap());
    colm_remote::engine::source_content_sha256(&ws.src()).unwrap();
}

#[test]
fn materialized_links_follow_text_edits_and_cannot_be_written_directly() {
    let (_root, ws) = linked_workspace("links-write");
    assert!(write_text(&ws, LINK_ALIAS, "bad\n", "alias", None).is_err());
    assert!(write_text(&ws, ".colm-source-links.json", "{}", "metadata", None).is_err());
    let result = write_text(&ws, LINK_TARGET, "FF=new\n", "compiler", None).unwrap();
    assert!(result.applied.files.iter().any(|path| path == LINK_CHAIN));
    assert_linked_bytes(&ws, "FF=new\n");
    assert!(list(&ws, "vendor/CoLM202X/include")
        .unwrap()
        .iter()
        .any(|e| e.path == LINK_ALIAS && e.kind == "link"));
}

#[test]
fn materialized_links_follow_replacement_patch_and_single_file_restore() {
    let (root, ws) = linked_workspace("links-patch");
    let replacement = root.join("replacement");
    std::fs::write(&replacement, "FF=put\n").unwrap();
    put_file(&ws, LINK_TARGET, &replacement, "replace", false).unwrap();
    assert_linked_bytes(&ws, "FF=put\n");
    let diff = format!("diff --git a/{LINK_TARGET} b/{LINK_TARGET}\n--- a/{LINK_TARGET}\n+++ b/{LINK_TARGET}\n@@ -1 +1 @@\n-FF=put\n+FF=patch\n");
    crate::patch::apply(&ws, &diff, "patch").unwrap();
    assert_linked_bytes(&ws, "FF=patch\n");
    let exported = crate::patch::export(&ws).unwrap();
    assert!(!exported.contains(&format!("diff --git a/{LINK_ALIAS} ")));
    assert!(!exported.contains(&format!("diff --git a/{LINK_CHAIN} ")));
    crate::patch::revert_to(&ws, &ws.info.base_commit).unwrap();
    crate::patch::apply(&ws, &exported, "import target edit").unwrap();
    assert_linked_bytes(&ws, "FF=patch\n");
    crate::patch::revert_file(&ws, LINK_TARGET, "restore").unwrap();
    assert_linked_bytes(&ws, "FF=old\n");
}

#[test]
fn materialized_links_follow_external_target_edits_but_preserve_conflicting_copies() {
    let (_root, ws) = linked_workspace("links-external");
    std::fs::write(ws.src().join(LINK_TARGET), "FF=external\n").unwrap();
    snapshot_external(&ws).unwrap().unwrap();
    assert_linked_bytes(&ws, "FF=external\n");
    let head = ws.head().unwrap();
    std::fs::write(ws.src().join(LINK_TARGET), "FF=target-edit\n").unwrap();
    std::fs::write(ws.src().join(LINK_ALIAS), "FF=alias-edit\n").unwrap();
    assert!(snapshot_external(&ws).is_err());
    assert_eq!(ws.head().unwrap(), head);
    assert_eq!(
        std::fs::read_to_string(ws.src().join(LINK_ALIAS)).unwrap(),
        "FF=alias-edit\n"
    );
    assert_eq!(
        std::fs::read_to_string(ws.src().join(LINK_CHAIN)).unwrap(),
        "FF=external\n"
    );
}

#[test]
fn materialized_link_deletions_update_metadata_and_target_deletions_are_rejected() {
    let (_root, ws) = linked_workspace("links-delete");
    let head = ws.head().unwrap();
    assert!(remove(&ws, LINK_TARGET, "delete target").is_err());
    assert_eq!(ws.head().unwrap(), head);
    assert_linked_bytes(&ws, "FF=old\n");
    remove(&ws, LINK_CHAIN, "delete chain").unwrap();
    assert!(crate::patch::revert_file(&ws, LINK_CHAIN, "restore alias").is_err());
    assert!(!git::is_dirty(&ws.src()).unwrap());
    let links = colm_remote::engine::source_links(&ws.src()).unwrap();
    assert!(!links.contains_key(Path::new(LINK_CHAIN)));
    assert!(links.contains_key(Path::new(LINK_ALIAS)));
    std::fs::remove_file(ws.src().join(LINK_ALIAS)).unwrap();
    snapshot_external(&ws).unwrap().unwrap();
    assert!(colm_remote::engine::source_links(&ws.src())
        .unwrap()
        .is_empty());
    remove(&ws, LINK_TARGET, "delete unused target").unwrap();
}

#[test]
fn materialized_links_follow_folder_replacement_without_losing_external_aliases() {
    let (root, ws) = linked_workspace("links-folder");
    let replacement = root.join("include");
    std::fs::create_dir(&replacement).unwrap();
    std::fs::write(replacement.join("Makeoptions.github"), "FF=folder\n").unwrap();
    std::fs::write(replacement.join("Makeoptions"), "FF=folder\n").unwrap();
    put_dir(&ws, "vendor/CoLM202X/include", &replacement, "folder").unwrap();
    assert_linked_bytes(&ws, "FF=folder\n");
    std::fs::remove_file(replacement.join("Makeoptions.github")).unwrap();
    let head = ws.head().unwrap();
    assert!(put_dir(
        &ws,
        "vendor/CoLM202X/include",
        &replacement,
        "broken folder"
    )
    .is_err());
    assert_eq!(ws.head().unwrap(), head);
    assert_linked_bytes(&ws, "FF=folder\n");
}

#[test]
fn materialized_link_patches_preserve_atomic_refusal_and_reject_nonportable_deletion_exports() {
    let (_root, ws) = linked_workspace("links-patch-invalid");
    let head = ws.head().unwrap();
    let deletion = format!("diff --git a/{LINK_TARGET} b/{LINK_TARGET}\ndeleted file mode 100644\n--- a/{LINK_TARGET}\n+++ /dev/null\n@@ -1 +0,0 @@\n-FF=old\n");
    assert!(crate::patch::apply(&ws, &deletion, "delete target").is_err());
    assert_eq!(ws.head().unwrap(), head);
    assert_linked_bytes(&ws, "FF=old\n");
    let alias_edit = format!("diff --git a/{LINK_ALIAS} b/{LINK_ALIAS}\n--- a/{LINK_ALIAS}\n+++ b/{LINK_ALIAS}\n@@ -1 +1 @@\n-FF=old\n+FF=conflict\n");
    assert!(crate::patch::apply(&ws, &alias_edit, "alias edit").is_err());
    assert_eq!(ws.head().unwrap(), head);
    assert_linked_bytes(&ws, "FF=old\n");
    let deletion = format!("diff --git a/{LINK_CHAIN} b/{LINK_CHAIN}\ndeleted file mode 100644\n--- a/{LINK_CHAIN}\n+++ /dev/null\n@@ -1 +0,0 @@\n-FF=old\n");
    crate::patch::apply(&ws, &deletion, "delete chain").unwrap();
    assert!(!colm_remote::engine::source_links(&ws.src())
        .unwrap()
        .contains_key(Path::new(LINK_CHAIN)));
    let error = crate::patch::export(&ws).unwrap_err().to_string();
    assert!(error.contains("full source package"), "{error}");
    assert!(!git::is_dirty(&ws.src()).unwrap());
    colm_remote::engine::source_content_sha256(&ws.src()).unwrap();
}

#[test]
fn materialized_link_folder_conflicts_are_rejected_before_replacement() {
    let (root, ws) = linked_workspace("links-folder-conflict");
    let replacement = root.join("include");
    std::fs::create_dir(&replacement).unwrap();
    std::fs::write(replacement.join("Makeoptions.github"), "FF=new\n").unwrap();
    std::fs::write(replacement.join("Makeoptions"), "FF=other\n").unwrap();
    assert!(put_dir(&ws, "vendor/CoLM202X/include", &replacement, "conflict").is_err());
    assert_linked_bytes(&ws, "FF=old\n");
}

#[cfg(unix)]
#[test]
fn materialized_target_edit_exports_apply_to_native_links() {
    let (root, ws) = linked_workspace("links-native-export");
    write_text(&ws, LINK_TARGET, "FF=portable\n", "portable", None).unwrap();
    let exported = crate::patch::export(&ws).unwrap();
    let repo = root.join("source");
    std::fs::remove_file(repo.join(".colm-source-links.json")).unwrap();
    for (alias, target) in [
        (LINK_ALIAS, "Makeoptions.github"),
        (LINK_CHAIN, "../vendor/CoLM202X/include/Makeoptions"),
    ] {
        std::fs::remove_file(repo.join(alias)).unwrap();
        std::os::unix::fs::symlink(target, repo.join(alias)).unwrap();
    }
    git::run(&repo, &["add", "-A"]).unwrap();
    git::run(&repo, &["commit", "-q", "-m", "native links"]).unwrap();
    let native =
        Workspace::create(&root.join("ws"), "native", repo.to_str().unwrap(), None).unwrap();
    crate::patch::apply(&native, &exported, "import portable target edit").unwrap();
    assert_eq!(
        colm_remote::engine::source_content_sha256(&native.src()).unwrap(),
        colm_remote::engine::source_content_sha256(&ws.src()).unwrap()
    );
}

#[test]
fn materialized_link_metadata_cannot_be_edited_even_with_different_case() {
    let (_root, ws) = linked_workspace("links-metadata");
    let original = std::fs::read(ws.src().join(".colm-source-links.json")).unwrap();
    let head = ws.head().unwrap();
    for path in [".colm-source-links.json", ".COLM-SOURCE-LINKS.JSON"] {
        assert!(write_text(&ws, path, "{}", "metadata", None).is_err());
        let diff = format!("diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1 @@\n+{{}}\n");
        assert!(crate::patch::apply(&ws, &diff, "metadata patch").is_err());
        assert_eq!(
            std::fs::read(ws.src().join(".colm-source-links.json")).unwrap(),
            original
        );
    }
    std::fs::write(ws.src().join(".colm-source-links.json"), "{}").unwrap();
    assert!(snapshot_external(&ws).is_err());
    assert_eq!(ws.head().unwrap(), head);
    assert_eq!(
        std::fs::read_to_string(ws.src().join(".colm-source-links.json")).unwrap(),
        "{}"
    );
    std::fs::write(ws.src().join(".colm-source-links.json"), original).unwrap();
    assert_linked_bytes(&ws, "FF=old\n");
}

#[test]
fn materialized_link_guards_use_existing_file_identity_without_folding_distinct_names() {
    let (_root, ws) = linked_workspace("links-case-identity");
    let alias = "vendor/CoLM202X/include/MAKEOPTIONS";
    let target = "vendor/CoLM202X/include/MAKEOPTIONS.GITHUB";
    let head = ws.head().unwrap();
    if ws.src().join(alias).exists() {
        assert!(write_text(&ws, alias, "FF=bad\n", "alternate case", None).is_err());
        assert_eq!(ws.head().unwrap(), head);
        assert_linked_bytes(&ws, "FF=old\n");
        assert!(remove(&ws, target, "alternate case target").is_err());
        assert_eq!(ws.head().unwrap(), head);
        assert_linked_bytes(&ws, "FF=old\n");
        assert!(remove(&ws, "VENDOR/COLM202X/INCLUDE", "alternate case directory").is_err());
        assert_eq!(ws.head().unwrap(), head);
        assert_linked_bytes(&ws, "FF=old\n");
    } else {
        create_text(&ws, alias, "independent\n", "distinct file").unwrap();
        write_text(&ws, alias, "independent edit\n", "distinct edit", None).unwrap();
        assert_eq!(
            std::fs::read_to_string(ws.src().join(alias)).unwrap(),
            "independent edit\n"
        );
        remove(&ws, alias, "remove distinct").unwrap();
        assert_linked_bytes(&ws, "FF=old\n");
    }
}

fn assert_mutation_rejected(ws: &Workspace, action: impl FnOnce() -> Result<Applied>) {
    let head = ws.head().unwrap();
    let status = git::run(&ws.src(), &["status", "--porcelain", "-z"]).unwrap();
    assert!(action().is_err(), "unsafe mutation was accepted");
    assert_eq!(ws.head().unwrap(), head);
    assert_eq!(
        git::run(&ws.src(), &["status", "--porcelain", "-z"]).unwrap(),
        status
    );
}

#[test]
fn mutation_preflight_rejects_same_file_and_hardlink_without_changing_bytes() {
    let (root, ws) = fresh("same-file-preflight");
    let file = ws.src().join(MODULE);
    let bytes = std::fs::read(&file).unwrap();
    let hardlink = root.join("hardlink.F90");
    std::fs::hard_link(&file, &hardlink).unwrap();
    for source in [&file, &hardlink] {
        assert_mutation_rejected(&ws, || put_file(&ws, MODULE, source, "same file", false));
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
        assert_eq!(std::fs::read(source).unwrap(), bytes);
    }
    #[cfg(unix)]
    {
        let alias = root.join("source-alias");
        std::os::unix::fs::symlink(file.parent().unwrap(), &alias).unwrap();
        assert_mutation_rejected(&ws, || {
            put_file(&ws, MODULE, &alias.join("MOD_Demo.F90"), "alias", false)
        });
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
    }
    let case_alias = ws.src().join(MODULE.to_ascii_lowercase());
    if case_alias.exists() {
        assert_mutation_rejected(&ws, || {
            put_file(&ws, MODULE, &case_alias, "case alias", false)
        });
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
    }
}

#[test]
fn mutation_preflight_rejects_overlapping_folders_without_changing_bytes() {
    let (_root, ws) = fresh("overlap-preflight");
    let base = "vendor/CoLM202X";
    let from = ws.src().join(base);
    let bytes = std::fs::read(ws.src().join(MODULE)).unwrap();
    for (path, source) in [
        (base.to_owned(), from.clone()),
        (base.to_owned(), from.join("main")),
        (format!("{base}/main"), from.clone()),
        (format!("{base}/new/nested"), from.clone()),
    ] {
        assert_mutation_rejected(&ws, || put_dir(&ws, &path, &source, "overlap"));
        assert_eq!(std::fs::read(ws.src().join(MODULE)).unwrap(), bytes);
    }
    #[cfg(unix)]
    {
        let alias = _root.join("folder-alias");
        std::os::unix::fs::symlink(&from, &alias).unwrap();
        assert_mutation_rejected(&ws, || {
            put_dir(&ws, base, &alias.join("main"), "alias overlap")
        });
        assert_eq!(std::fs::read(ws.src().join(MODULE)).unwrap(), bytes);
    }
    let case_alias = ws.src().join(base.to_ascii_lowercase());
    if case_alias.exists() {
        assert_mutation_rejected(&ws, || put_dir(&ws, base, &case_alias, "case overlap"));
        assert_eq!(std::fs::read(ws.src().join(MODULE)).unwrap(), bytes);
    }
}

fn add_protected_answer(ws: &Workspace) -> PathBuf {
    let answer = ws.src().join("oracle/golden/answer.txt");
    std::fs::create_dir_all(answer.parent().unwrap()).unwrap();
    std::fs::write(&answer, "protected answer\n").unwrap();
    git::run(&ws.src(), &["add", "-f", "oracle/golden/answer.txt"]).unwrap();
    git::run(&ws.src(), &["commit", "-qm", "fixture answer"]).unwrap();
    answer
}

#[test]
fn mutation_preflight_protects_old_subtrees_from_removal_and_replacement() {
    let (root, ws) = fresh("protected-subtree-preflight");
    let answer = add_protected_answer(&ws);
    let replacement = root.join("replacement");
    std::fs::create_dir(&replacement).unwrap();
    std::fs::write(replacement.join("ordinary.txt"), "replacement").unwrap();
    assert!(
        list(&ws, "oracle").is_ok(),
        "ancestor browsing must remain available"
    );
    for path in ["oracle", "oracle/golden"] {
        assert_mutation_rejected(&ws, || remove(&ws, path, "remove"));
        assert_eq!(
            std::fs::read_to_string(&answer).unwrap(),
            "protected answer\n"
        );
        assert_mutation_rejected(&ws, || put_dir(&ws, path, &replacement, "replace"));
        assert_eq!(
            std::fs::read_to_string(&answer).unwrap(),
            "protected answer\n"
        );
    }
}

#[test]
fn mutation_preflight_snapshot_checks_both_sides_of_staged_renames() {
    for move_out in [true, false] {
        let (_root, ws) = fresh(if move_out {
            "rename-out-preflight"
        } else {
            "rename-in-preflight"
        });
        let answer = add_protected_answer(&ws);
        if !move_out {
            git::run(&ws.src(), &["mv", "oracle/golden/answer.txt", "answer.txt"]).unwrap();
            git::run(&ws.src(), &["commit", "-qm", "fixture outside answer"]).unwrap();
            std::fs::create_dir_all(answer.parent().unwrap()).unwrap();
        }
        let (from, to) = if move_out {
            ("oracle/golden/answer.txt", "answer.txt")
        } else {
            ("answer.txt", "oracle/golden/answer.txt")
        };
        git::run(&ws.src(), &["mv", from, to]).unwrap();
        assert_mutation_rejected(&ws, || snapshot_external(&ws).map(Option::unwrap));
        assert_eq!(
            std::fs::read_to_string(ws.src().join(to)).unwrap(),
            "protected answer\n"
        );
        assert!(!ws.src().join(from).exists());
    }
}

#[test]
fn ordinary_staged_renames_still_snapshot() {
    let (_root, ws) = fresh("ordinary-rename-preflight");
    let bytes = std::fs::read(ws.src().join(MODULE)).unwrap();
    git::run(&ws.src(), &["mv", MODULE, "renamed.F90"]).unwrap();
    let applied = snapshot_external(&ws).unwrap().unwrap();
    assert!(applied.files.iter().any(|path| path == MODULE));
    assert!(applied.files.iter().any(|path| path == "renamed.F90"));
    assert_eq!(std::fs::read(ws.src().join("renamed.F90")).unwrap(), bytes);
    assert!(!git::is_dirty(&ws.src()).unwrap());
}

#[cfg(unix)]
#[test]
fn native_links_preflight_target_and_traversal_removal_and_replacement() {
    let (root, ws) = fresh("native-link-removal");
    let src = ws.src();
    std::fs::create_dir_all(src.join("crates/config/empty")).unwrap();
    std::fs::write(src.join("crates/config/target"), "old\n").unwrap();
    std::os::unix::fs::symlink("config/empty/../target", src.join("crates/alias")).unwrap();
    git::run(&src, &["add", "-A"]).unwrap();
    git::run(&src, &["commit", "-qm", "native link"]).unwrap();
    let head = ws.head().unwrap();
    for path in [
        "crates/config/target",
        "crates/config/empty",
        "crates/config",
    ] {
        assert!(remove(&ws, path, "remove target").is_err(), "{path}");
        assert_eq!(ws.head().unwrap(), head);
        assert_eq!(
            std::fs::read_to_string(src.join("crates/alias")).unwrap(),
            "old\n"
        );
    }
    let replacement = root.join("replacement");
    std::fs::create_dir_all(&replacement).unwrap();
    std::fs::write(replacement.join("target"), "new\n").unwrap();
    assert!(put_dir(&ws, "crates/config", &replacement, "missing traversal").is_err());
    assert_eq!(ws.head().unwrap(), head);
    std::fs::create_dir(replacement.join("empty")).unwrap();
    put_dir(&ws, "crates/config", &replacement, "valid replacement").unwrap();
    assert_eq!(
        std::fs::read_to_string(src.join("crates/alias")).unwrap(),
        "new\n"
    );
    let deletion = "diff --git a/crates/config/target b/crates/config/target\ndeleted file mode 100644\n--- a/crates/config/target\n+++ /dev/null\n@@ -1 +0,0 @@\n-new\n";
    assert!(crate::patch::apply(&ws, deletion, "delete linked target").is_err());
    assert_eq!(
        std::fs::read_to_string(src.join("crates/alias")).unwrap(),
        "new\n"
    );
    assert!(!git::is_dirty(&src).unwrap());
    remove(&ws, "crates", "remove aliases with targets").unwrap();
}

#[test]
fn materialized_links_preserve_required_empty_directories_during_replacement() {
    let (root, ws) = fresh("logical-link-traversal");
    let src = ws.src();
    std::fs::create_dir_all(src.join("crates/config/empty")).unwrap();
    std::fs::write(src.join("crates/config/target"), "old\n").unwrap();
    std::fs::write(src.join("crates/alias"), "old\n").unwrap();
    std::fs::write(
        src.join(".colm-source-links.json"),
        r#"{"crates/alias":"config/empty/../target"}"#,
    )
    .unwrap();
    git::run(&src, &["add", "-A"]).unwrap();
    git::run(&src, &["commit", "-qm", "logical link"]).unwrap();
    let head = ws.head().unwrap();
    assert!(remove(&ws, "crates/config/empty", "remove traversed dir").is_err());
    let replacement = root.join("replacement");
    std::fs::create_dir_all(&replacement).unwrap();
    std::fs::write(replacement.join("target"), "new\n").unwrap();
    assert!(put_dir(&ws, "crates/config", &replacement, "missing traversal").is_err());
    assert_eq!(ws.head().unwrap(), head);
    std::fs::create_dir(replacement.join("empty")).unwrap();
    put_dir(&ws, "crates/config", &replacement, "valid traversal").unwrap();
    assert_eq!(
        std::fs::read_to_string(src.join("crates/alias")).unwrap(),
        "new\n"
    );
    assert!(!git::is_dirty(&src).unwrap());
}

#[cfg(unix)]
#[test]
fn unrelated_invalid_native_links_do_not_block_removal_or_replacement() {
    let (root, ws) = fresh("invalid-native-unrelated");
    let src = ws.src();
    std::fs::create_dir(src.join("bad-links")).unwrap();
    for (alias, target) in [("absolute", "/tmp"), ("dangling", "missing")] {
        std::os::unix::fs::symlink(target, src.join("bad-links").join(alias)).unwrap();
    }
    git::run(&src, &["add", "-A"]).unwrap();
    git::run(&src, &["commit", "-qm", "legacy links"]).unwrap();
    remove(&ws, MODULE, "remove unrelated module").unwrap();
    let replacement = root.join("replacement");
    std::fs::create_dir(&replacement).unwrap();
    std::fs::write(replacement.join("new.rs"), "new\n").unwrap();
    put_dir(
        &ws,
        "crates/colm-core/src",
        &replacement,
        "replace unrelated tree",
    )
    .unwrap();
    remove(&ws, "bad-links", "remove unsupported aliases").unwrap();
    assert!(!src.join("bad-links").exists());
    assert!(!git::is_dirty(&src).unwrap());
}

#[test]
fn failed_file_commits_restore_owned_data_and_ignored_files() {
    let (root, ws) = fresh("files-rollback");
    let original = std::fs::read(ws.src().join(MODULE)).unwrap();
    let replacement = root.join("replacement");
    std::fs::create_dir_all(&replacement).unwrap();
    std::fs::write(replacement.join("new.txt"), "replacement").unwrap();
    let ignored = ws.src().join("vendor/CoLM202X/main/personal.tmp");
    std::fs::write(ws.src().join(".git/info/exclude"), "personal.tmp\n").unwrap();
    std::fs::write(&ignored, "keep my ignored notes").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            ignored.parent().unwrap(),
            std::fs::Permissions::from_mode(0o750),
        )
        .unwrap();
    }
    git::run(&ws.src(), &["config", "user.name", ""]).unwrap();
    assert!(write_text(&ws, MODULE, "changed\n", "edit", None).is_err());
    assert!(create_text(&ws, "new.txt", "new\n", "create").is_err());
    assert!(!ws.src().join("new.txt").exists());
    assert!(put_dir(&ws, "vendor/CoLM202X/main", &replacement, "replace").is_err());
    assert!(remove(&ws, "vendor/CoLM202X/main", "remove").is_err());
    assert_eq!(std::fs::read(ws.src().join(MODULE)).unwrap(), original);
    assert_eq!(
        std::fs::read_to_string(ignored).unwrap(),
        "keep my ignored notes"
    );
    assert!(!ws.src().join("vendor/CoLM202X/main/new.txt").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(ws.src().join("vendor/CoLM202X/main"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o750
        );
    }
    assert!(!git::is_dirty(&ws.src()).unwrap());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn failed_snapshot_preserves_external_edits_and_prior_staging() {
    let (root, ws) = fresh("snapshot-rollback");
    std::fs::write(ws.src().join(MODULE), "external edit\n").unwrap();
    git::run(&ws.src(), &["add", "--", MODULE]).unwrap();
    let staged = git::run(&ws.src(), &["diff", "--cached"]).unwrap();
    git::run(&ws.src(), &["config", "user.name", ""]).unwrap();
    assert!(snapshot_external(&ws).is_err());
    assert_eq!(git::run(&ws.src(), &["diff", "--cached"]).unwrap(), staged);
    assert_eq!(
        std::fs::read_to_string(ws.src().join(MODULE)).unwrap(),
        "external edit\n"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn source_write_lock_serializes_separate_processes() {
    const CHILD_ROOT: &str = "COLM_WORKSPACE_LOCK_TEST_ROOT";
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let root = PathBuf::from(root);
        let ws = Workspace::open(&root.join("ws"), "demo").unwrap();
        std::fs::write(root.join("child-ready"), "ready").unwrap();
        create_text(&ws, "child.txt", "child\n", "child").unwrap();
        return;
    }
    let (root, ws) = fresh("process-lock");
    let guard = git::WriteLock::acquire(&ws).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "files::files_tests::source_write_lock_serializes_separate_processes",
        ])
        .env(CHILD_ROOT, &root)
        .spawn()
        .unwrap();
    let started = std::time::Instant::now();
    while !root.join("child-ready").exists() {
        assert!(started.elapsed().as_secs() < 5, "child did not start");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(!ws.src().join("child.txt").exists());
    drop(guard);
    assert!(child.wait().unwrap().success());
    assert_eq!(
        std::fs::read_to_string(ws.src().join("child.txt")).unwrap(),
        "child\n"
    );
    assert!(!git::is_dirty(&ws.src()).unwrap());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn rollback_preserves_a_later_external_edit_and_keeps_original_backup() {
    let (root, ws) = fresh("rollback-later-edit");
    let mut edit = git::Edit::begin(&ws, vec![MODULE.into()]).unwrap();
    std::fs::write(ws.src().join(MODULE), "our edit\n").unwrap();
    edit.record(&[MODULE.into()]).unwrap();
    edit.checkpoint().unwrap();
    std::fs::write(ws.src().join(MODULE), "later external edit\n").unwrap();
    let error = edit
        .finish::<()>(Err(anyhow::anyhow!("commit failed")))
        .unwrap_err();
    assert!(format!("{error:#}").contains("originals retained"));
    assert_eq!(
        std::fs::read_to_string(ws.src().join(MODULE)).unwrap(),
        "later external edit\n"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn failed_linked_edit_restores_copies_and_metadata() {
    let (root, ws) = linked_workspace("links-rollback");
    let metadata = std::fs::read(ws.src().join(".colm-source-links.json")).unwrap();
    git::run(&ws.src(), &["config", "user.name", ""]).unwrap();
    assert!(write_text(&ws, LINK_TARGET, "FF=changed\n", "compiler", None).is_err());
    assert_linked_bytes(&ws, "FF=old\n");
    assert!(remove(&ws, LINK_CHAIN, "remove alias").is_err());
    assert_linked_bytes(&ws, "FF=old\n");
    assert_eq!(
        std::fs::read(ws.src().join(".colm-source-links.json")).unwrap(),
        metadata
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn refused_edit_does_not_claim_an_external_new_file() {
    let (root, ws) = fresh("rollback-external-create");
    let edit = git::Edit::begin(&ws, vec!["notes.txt".into()]).unwrap();
    std::fs::write(ws.src().join("notes.txt"), "external notes\n").unwrap();
    assert!(edit
        .finish::<()>(Err(anyhow::anyhow!("already exists")))
        .is_err());
    assert_eq!(
        std::fs::read_to_string(ws.src().join("notes.txt")).unwrap(),
        "external notes\n"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn rollback_preserves_new_external_alias_changes_and_index_changes() {
    let (root, ws) = linked_workspace("rollback-external-alias");
    let mut edit = git::Edit::begin(&ws, vec![LINK_TARGET.into()]).unwrap();
    edit.change(&[LINK_TARGET.into()], || {
        Ok(std::fs::write(ws.src().join(LINK_TARGET), "FF=ours\n")?)
    })
    .unwrap();
    std::fs::write(ws.src().join(LINK_ALIAS), "external alias\n").unwrap();
    let result = crate::source_links::synchronize(&ws.src(), &mut edit);
    assert!(edit.finish(result).is_err());
    assert_eq!(
        std::fs::read_to_string(ws.src().join(LINK_ALIAS)).unwrap(),
        "external alias\n"
    );
    let _ = std::fs::remove_dir_all(root);

    let (root, ws) = fresh("rollback-external-index");
    let mut edit = git::Edit::begin(&ws, vec![MODULE.into()]).unwrap();
    edit.change(&[MODULE.into()], || {
        Ok(std::fs::write(ws.src().join(MODULE), "our edit\n")?)
    })
    .unwrap();
    edit.stage(&["add", "--", MODULE]).unwrap();
    git::run(&ws.src(), &["reset", "--", MODULE]).unwrap();
    let error = edit
        .finish::<()>(Err(anyhow::anyhow!("commit failed")))
        .unwrap_err();
    assert!(format!("{error:#}").contains("preserved its staging"));
    assert!(git::run(&ws.src(), &["diff", "--cached"])
        .unwrap()
        .is_empty());
    assert_eq!(
        std::fs::read_to_string(ws.src().join(MODULE)).unwrap(),
        "our edit\n"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn partial_owned_io_failure_restores_original_bytes() {
    let (root, ws) = fresh("rollback-partial-write");
    let before = std::fs::read(ws.src().join(MODULE)).unwrap();
    let mut edit = git::Edit::begin(&ws, vec![MODULE.into()]).unwrap();
    let result = edit.change(&[MODULE.into()], || -> anyhow::Result<()> {
        std::fs::write(ws.src().join(MODULE), "partial write")?;
        anyhow::bail!("simulated write failure")
    });
    assert!(edit.finish(result).is_err());
    assert_eq!(std::fs::read(ws.src().join(MODULE)).unwrap(), before);
    assert!(!git::is_dirty(&ws.src()).unwrap());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn failed_nested_create_removes_only_its_empty_parents() {
    let (root, ws) = fresh("failed-create-parents");
    git::run(&ws.src(), &["config", "user.name", ""]).unwrap();
    assert!(create_text(&ws, "new/nested/module.F90", "module x\n", "create").is_err());
    assert!(!ws.src().join("new").exists());
    assert!(!git::is_dirty(&ws.src()).unwrap());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn local_ignore_rules_are_preserved_when_they_cannot_be_decoded() {
    let (root, ws) = fresh("ignore-invalid-utf8");
    let path = ws.src().join(".git/info/exclude");
    let original = b"notes\n\xff\n";
    std::fs::write(&path, original).unwrap();
    assert!(snapshot_external(&ws).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(target_os = "linux")]
#[test]
fn snapshots_refuse_non_utf8_paths_without_dropping_them() {
    use std::os::unix::ffi::OsStringExt;
    let (root, ws) = fresh("snapshot-nonutf8");
    let path = ws
        .src()
        .join(std::ffi::OsString::from_vec(b"file-\xff".to_vec()));
    std::fs::write(&path, "keep me").unwrap();
    let error = snapshot_external(&ws).unwrap_err();
    assert!(format!("{error:#}").contains("non-UTF-8"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "keep me");
    assert!(git::is_dirty(&ws.src()).unwrap());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn replacing_a_hardlinked_file_keeps_other_paths_unchanged() {
    let (root, ws) = fresh("hardlink-edit");
    let original = std::fs::read(ws.src().join(MODULE)).unwrap();
    let alias = ws.src().join("alias.F90");
    std::fs::hard_link(ws.src().join(MODULE), &alias).unwrap();
    git::run(&ws.src(), &["add", "alias.F90"]).unwrap();
    git::run(&ws.src(), &["commit", "-qm", "hardlink fixture"]).unwrap();
    write_text(&ws, MODULE, "updated\n", "edit", None).unwrap();
    assert_eq!(std::fs::read(&alias).unwrap(), original);
    assert!(!git::is_dirty(&ws.src()).unwrap());
    git::run(&ws.src(), &["config", "user.name", ""]).unwrap();
    assert!(write_text(&ws, MODULE, "failed edit\n", "edit", None).is_err());
    assert_eq!(std::fs::read(&alias).unwrap(), original);
    assert_eq!(
        std::fs::read_to_string(ws.src().join(MODULE)).unwrap(),
        "updated\n"
    );
    assert!(!git::is_dirty(&ws.src()).unwrap());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn killed_workspace_writer_does_not_leave_the_lock_held() {
    const CHILD_ROOT: &str = "COLM_KILLED_WORKSPACE_WRITER";
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let root = PathBuf::from(root);
        let ws = Workspace::open(&root.join("ws"), "demo").unwrap();
        let _guard = git::WriteLock::acquire(&ws).unwrap();
        std::fs::write(root.join("ready"), "ready").unwrap();
        std::thread::sleep(std::time::Duration::from_secs(30));
        return;
    }
    let (root, ws) = fresh("killed-writer");
    let legacy = ws.dir.join("source-write.lock");
    let alive = std::process::id().to_string();
    std::fs::write(&legacy, &alive).unwrap();
    assert!(git::WriteLock::acquire(&ws).is_err());
    assert_eq!(std::fs::read_to_string(&legacy).unwrap(), alive);
    std::fs::remove_file(&legacy).unwrap();
    let recovery = ws.dir.join(".edit-backup-recovery");
    std::fs::create_dir(&recovery).unwrap();
    std::fs::write(recovery.join("original"), "keep for recovery").unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "files::files_tests::killed_workspace_writer_does_not_leave_the_lock_held",
        ])
        .env(CHILD_ROOT, &root)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let began = std::time::Instant::now();
    while !root.join("ready").exists() && began.elapsed().as_secs() < 5 {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let ready = root.join("ready").exists();
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(ready);
    // The PID-file marker is a legacy version's format; a killed owner is recoverable.
    std::fs::write(ws.dir.join("source-write.lock"), child.id().to_string()).unwrap();
    create_text(&ws, "recovered.txt", "ok\n", "after crash").unwrap();
    assert_eq!(
        std::fs::read_to_string(recovery.join("original")).unwrap(),
        "keep for recovery"
    );
    assert_eq!(
        std::fs::read_to_string(ws.src().join("recovered.txt")).unwrap(),
        "ok\n"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn private_index_handles_split_index_and_rollback() {
    let (root, ws) = fresh("private-split-index");
    git::run(&ws.src(), &["update-index", "--split-index"]).unwrap();
    let changed = read(&ws, MODULE)
        .unwrap()
        .text
        .unwrap()
        .replace("2.0", "3.0");
    write_text(&ws, MODULE, &changed, "split edit", None).unwrap();
    git::run(&ws.src(), &["config", "user.name", ""]).unwrap();
    assert!(write_text(&ws, MODULE, "failed\n", "split failure", None).is_err());
    assert_eq!(
        std::fs::read_to_string(ws.src().join(MODULE)).unwrap(),
        changed
    );
    assert!(!git::is_dirty(&ws.src()).unwrap());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn private_index_keeps_a_sparse_worktree_sparse() {
    let (root, ws) = fresh("private-sparse-index");
    git::run(
        &ws.src(),
        &["sparse-checkout", "init", "--cone", "--sparse-index"],
    )
    .unwrap();
    git::run(&ws.src(), &["sparse-checkout", "set", "vendor"]).unwrap();
    assert!(!ws.src().join("crates").exists());
    let changed = read(&ws, MODULE)
        .unwrap()
        .text
        .unwrap()
        .replace("2.0", "3.0");
    write_text(&ws, MODULE, &changed, "sparse edit", None).unwrap();
    assert!(!ws.src().join("crates").exists());
    assert!(!git::is_dirty(&ws.src()).unwrap());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn relative_workspace_roots_support_edit_patch_restore_and_rollback() {
    const CHILD: &str = "COLM_RELATIVE_WORKSPACE_TEST";
    if std::env::var_os(CHILD).is_some() {
        let ws = Workspace::open(Path::new("ws"), "demo").unwrap();
        assert!(ws.dir.is_relative());
        let original = read(&ws, MODULE).unwrap().text.unwrap();
        let changed = original.replace("2.0", "3.0");
        write_text(&ws, MODULE, &changed, "relative edit", None).unwrap();
        let diff = "diff --git a/nested/new.txt b/nested/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/nested/new.txt\n@@ -0,0 +1 @@\n+new\n";
        crate::patch::apply(&ws, diff, "relative patch").unwrap();
        crate::patch::revert_file(&ws, "nested/new.txt", "relative restore").unwrap();
        assert!(!ws.src().join("nested/new.txt").exists());
        git::run(&ws.src(), &["config", "user.name", ""]).unwrap();
        assert!(write_text(&ws, MODULE, "failed\n", "relative failure", None).is_err());
        assert_eq!(
            std::fs::read_to_string(ws.src().join(MODULE)).unwrap(),
            changed
        );
        assert!(!git::is_dirty(&ws.src()).unwrap());
        return;
    }
    let (root, _ws) = fresh("relative-edit-root");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "files::files_tests::relative_workspace_roots_support_edit_patch_restore_and_rollback",
        ])
        .env(CHILD, "1")
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn replacing_imports_keeps_source_permissions_and_text_edits_keep_them() {
    use std::os::unix::fs::PermissionsExt;
    let (root, ws) = fresh("import-executable");
    let source = root.join("tool.sh");
    std::fs::write(&source, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o755)).unwrap();
    for destination in ["tool.sh", MODULE] {
        put_file(&ws, destination, &source, "import executable", false).unwrap();
        assert_eq!(
            std::fs::metadata(ws.src().join(destination))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }
    write_text(&ws, "tool.sh", "#!/bin/sh\nexit 1\n", "edit script", None).unwrap();
    assert_eq!(
        std::fs::metadata(ws.src().join("tool.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    assert!(!git::is_dirty(&ws.src()).unwrap());
    let _ = std::fs::remove_dir_all(root);
}

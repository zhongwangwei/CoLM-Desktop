use super::*;
use colm_model::{CodeRef, ParameterScopeInstance};

fn record(key: &str, kind: &str, index: Option<u8>, value: &str) -> ParameterOverrideRecord {
    ParameterOverrideRecord {
        parameter_id: format!("x:{key}"),
        raw_key: key.into(),
        scope_instance: ParameterScopeInstance {
            kind: kind.into(),
            scheme: (kind == "land-cover-class").then(|| "IGBP".into()),
            index,
            type_name: None,
            process_file: None,
        },
        value: value.into(),
        expected_old_value: None,
        unit: None,
    }
}

fn revision(id: &str, source: &str) -> FrozenRevision {
    FrozenRevision {
        schema: colm_model::SCHEMA,
        version: VersionRef {
            id: id.into(),
            revision: 1,
        },
        name: id.into(),
        note: String::new(),
        parent: None,
        code: CodeRef {
            kind: CodeKind::Official,
            workspace: None,
            commit: None,
            source_sha256: source.into(),
        },
        overlay: Overlay::default(),
        overlay_sha256: String::new(),
        fingerprint: String::new(),
        created_unix: 0,
        applicability: Default::default(),
        derived_from: None,
    }
}

#[test]
fn parameter_and_physics_changes_are_listed_per_variable_and_scope() {
    let mut before = Overlay {
        parameters: vec![
            record("DEF_TUNING_CNFAC", "case-scalar", None, "0.5"),
            record("DEF_LC_VMAX25", "land-cover-class", Some(10), "5e-5"),
        ],
        ..Default::default()
    };
    before
        .physics
        .insert("DEF_USE_WUEST".into(), ".false.".into());
    let mut after = Overlay {
        parameters: vec![
            record("DEF_TUNING_CNFAC", "case-scalar", None, "0.26"),
            // Same variable, another class: a different override.
            record("DEF_LC_VMAX25", "land-cover-class", Some(11), "5e-5"),
        ],
        ..Default::default()
    };
    after
        .physics
        .insert("DEF_USE_WUEST".into(), ".false.".into());
    after
        .physics
        .insert("DEF_USE_PLANTHYDRAULICS".into(), ".true.".into());
    let parameters = diff(&parameter_map(&before), &parameter_map(&after));
    let shown: Vec<(&str, Option<&str>, Option<&str>)> = parameters
        .iter()
        .map(|c| (c.key.as_str(), c.before.as_deref(), c.after.as_deref()))
        .collect();
    assert_eq!(
        shown,
        [
            (
                "DEF_LC_VMAX25 [land-cover-class IGBP 10]",
                Some("5e-5"),
                None
            ),
            (
                "DEF_LC_VMAX25 [land-cover-class IGBP 11]",
                None,
                Some("5e-5")
            ),
            ("DEF_TUNING_CNFAC", Some("0.5"), Some("0.26")),
        ]
    );
    let physics = diff(&before.physics, &after.physics);
    assert_eq!(
        physics,
        [Change {
            key: "DEF_USE_PLANTHYDRAULICS".into(),
            before: None,
            after: Some(".true.".into()),
        }]
    );
    assert!(network(&before, &after).is_none());
    after.hybrid = Some(colm_model::HybridBundle {
        config: "[model]".into(),
        assets: Vec::new(),
    });
    let added = network(&before, &after).unwrap();
    assert!(added.before.is_none() && added.after.as_deref().is_some_and(|id| id.len() == 12));
}

#[test]
fn declared_studies_are_read_and_other_origins_are_passed_through() {
    let text = serde_json::json!({"declared_training_studies":[{
        "study":"/p/.colm/studies/s-1","use":"parameters","sites":["cng"],
        "targets":[{"variable":"Qle","metric":"nrmse","from":0,"to":1}],
        "state":{"best_member":"m000001","best_objective":1.658}
    }, {"study":"/p/.colm/studies/s-0","sites":[],"targets":[],"state":null}]})
    .to_string();
    let (studies, origin) = studies(Some(&text));
    assert!(origin.is_none());
    assert_eq!(studies.len(), 2);
    assert_eq!(studies[0].purpose, "parameters");
    assert_eq!(studies[0].best_member.as_deref(), Some("m000001"));
    assert_eq!(studies[0].targets, ["Qle nrmse"]);
    assert_eq!(
        studies[1].purpose, "network",
        "entries written before the purpose was recorded came from attached networks"
    );
    let (none, origin) = super::studies(Some("study:/p/s/member:m000003"));
    assert!(none.is_empty());
    assert_eq!(origin.as_deref(), Some("study:/p/s/member:m000003"));
}

fn repo(dir: &Path) -> String {
    std::fs::create_dir_all(dir.join("vendor")).unwrap();
    std::fs::write(dir.join("vendor/MOD_A.F90"), "a\n").unwrap();
    std::fs::write(dir.join("vendor/MOD_B.F90"), "b\n").unwrap();
    git::run(dir, &["init", "-q"]).unwrap();
    git::set_identity(dir).unwrap();
    git::run(dir, &["add", "-A"]).unwrap();
    // 各自建仓库时的那次提交（消息里带目录名，同一秒提交也不会撞成同一个 id）。
    let message = format!("official source in {}", dir.display());
    git::run(dir, &["commit", "-q", "-m", &message]).unwrap();
    git::head(dir).unwrap()
}

fn change(dir: &Path, file: &str, text: &str) -> String {
    std::fs::write(dir.join(file), text).unwrap();
    git::run(dir, &["commit", "-q", "-am", "change"]).unwrap();
    git::head(dir).unwrap()
}

#[test]
fn two_versions_differ_only_where_their_files_differ() {
    let root = std::env::temp_dir().join(format!("colm-version-changes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let official = VersionRef {
        id: "official".into(),
        revision: 99,
    };
    // Two separate clones of the same official source: their base commits differ, the content does not.
    let (one, two) = (root.join("one"), root.join("two"));
    let base_one = repo(&one);
    let base_two = repo(&two);
    assert_ne!(base_one, base_two);
    let soil = change(&one, "vendor/MOD_A.F90", "soil\n");
    change(&two, "vendor/MOD_A.F90", "soil\n");
    let roots = change(&two, "vendor/MOD_B.F90", "roots\n");
    let side = |dir: &Path, commit: &str, base: &str| Side::Workspace {
        dir: dir.into(),
        commit: commit.into(),
        base: base.into(),
        root: Some(official.clone()),
    };
    let (a, b) = (side(&one, &soil, &base_one), side(&two, &roots, &base_two));
    let rev = revision("v", "0");
    let (differs, note) = code_differs(&b, &a, &rev, &rev).unwrap();
    assert_eq!(
        differs,
        ["vendor/MOD_B.F90"],
        "MOD_A was changed the same way"
    );
    assert!(note.is_none());
    assert_eq!(
        b.changed()
            .unwrap()
            .iter()
            .map(|c| c.path.as_str())
            .collect::<Vec<_>>(),
        ["vendor/MOD_A.F90", "vendor/MOD_B.F90"]
    );
    // Against the official source it was based on: every file the version changed.
    let plain = Side::Official {
        plain: None,
        root: official.clone(),
    };
    let (differs, note) = code_differs(&b, &plain, &rev, &rev).unwrap();
    assert_eq!(differs, ["vendor/MOD_A.F90", "vendor/MOD_B.F90"]);
    assert!(note.is_none());
    // Based on another official revision: still compared file by file, with a note.
    let other = Side::Workspace {
        dir: one.clone(),
        commit: soil.clone(),
        base: base_one.clone(),
        root: Some(VersionRef {
            id: "official".into(),
            revision: 95,
        }),
    };
    let (differs, note) = code_differs(&b, &other, &rev, &rev).unwrap();
    assert_eq!(differs, ["vendor/MOD_B.F90"]);
    assert_eq!(note, Some("different_official_source"));
    // Two official revisions: equal sources, nothing to list.
    let (differs, note) =
        code_differs(&plain, &plain, &revision("o", "1"), &revision("o", "1")).unwrap();
    assert!(differs.is_empty() && note.is_none());
    let (_, note) = code_differs(&plain, &plain, &revision("o", "1"), &revision("o", "2")).unwrap();
    assert_eq!(note, Some("official_revisions_differ"));
    let _ = std::fs::remove_dir_all(&root);
}

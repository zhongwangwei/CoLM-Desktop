//! What an operation acts on, named the way a person recognizes it.
//!
//! The ledger records paths and identifiers; a history list made of those reads as hashes
//! and temporary folders. Each operation therefore also records its subject: a case (station)
//! and the project folder it belongs to, a study, a model version, a workspace or a server.

use serde_json::{json, Value};
use std::path::Path;

fn name_of(path: &str) -> Option<String> {
    Path::new(path.trim_end_matches(['/', '\\']))
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
}

fn parent_name(path: &str) -> Option<String> {
    Path::new(path.trim_end_matches(['/', '\\']))
        .parent()
        .and_then(|parent| parent.to_str())
        .and_then(name_of)
}

fn subject(kind: &str, name: Option<String>, project: Option<String>) -> Option<Value> {
    let name = name?;
    let mut value = json!({ "kind": kind, "name": name });
    if let Some(project) = project.filter(|project| !project.is_empty()) {
        value["project"] = json!(project);
    }
    Some(value)
}

/// `action` is the command (`run`, `new`, `study-run`, …), `object` its first positional
/// argument and `flag` looks up a public flag value such as `--out` or `--name`.
pub fn operation_subject(
    action: &str,
    object: Option<&str>,
    flag: &dyn Fn(&str) -> Option<String>,
) -> Option<Value> {
    let object = object.filter(|value| !value.is_empty() && !value.starts_with('-'));
    match action {
        "new" | "site-new" | "spatial-new" => {
            let out = flag("--out").or_else(|| object.map(str::to_owned))?;
            subject(
                "case",
                flag("--name").or_else(|| name_of(&out)),
                parent_name(&out),
            )
        }
        "mesh-new" => {
            let out = flag("--out")?;
            let case = Path::new(&out).parent()?.to_str()?.to_owned();
            subject("case", name_of(&case), parent_name(&case))
        }
        "model-create" | "model-save-draft" | "model-freeze" => subject(
            "version",
            flag("--name")
                .or_else(|| flag("--model-id"))
                .or_else(|| object.and_then(name_of)),
            None,
        ),
        _ if action.starts_with("study-") || action.starts_with("remote-study-") => {
            let study = object.map(str::to_owned).or_else(|| flag("--study"))?;
            subject("study", name_of(&study), parent_name(&study))
        }
        _ if action.starts_with("ws-") || action == "remote-workspace" => subject(
            "workspace",
            flag("--name")
                .or_else(|| flag("--from").as_deref().and_then(name_of))
                .or_else(|| object.and_then(name_of)),
            None,
        ),
        _ => match object {
            Some(path) => subject("case", name_of(path), parent_name(path)),
            None => subject("server", flag("--host"), None),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn cases_studies_versions_and_servers_get_readable_subjects() {
        let none = flags(&[]);
        assert_eq!(
            operation_subject("run", Some("/data/PLUMBER2/AT-Neu/"), &none),
            Some(json!({"kind":"case","name":"AT-Neu","project":"PLUMBER2"}))
        );
        assert_eq!(
            operation_subject(
                "new",
                None,
                &flags(&[("--out", "/p/heihe/basin"), ("--name", "Heihe")])
            ),
            Some(json!({"kind":"case","name":"Heihe","project":"heihe"}))
        );
        assert_eq!(
            operation_subject(
                "mesh-new",
                None,
                &flags(&[("--out", "/p/region/caseA/mesh.nc")])
            ),
            Some(json!({"kind":"case","name":"caseA","project":"region"}))
        );
        assert_eq!(
            operation_subject("study-run", Some("/studies/calibrate-lai"), &none),
            Some(json!({"kind":"study","name":"calibrate-lai","project":"studies"}))
        );
        assert_eq!(
            operation_subject(
                "model-freeze",
                None,
                &flags(&[("--name", "My snow scheme")])
            ),
            Some(json!({"kind":"version","name":"My snow scheme"}))
        );
        assert_eq!(
            operation_subject("ws-create", None, &flags(&[("--from", "/src/snow-source")])),
            Some(json!({"kind":"workspace","name":"snow-source"}))
        );
        assert_eq!(
            operation_subject("remote-probe", None, &flags(&[("--host", "tms17l")])),
            Some(json!({"kind":"server","name":"tms17l"}))
        );
        assert_eq!(operation_subject("remote-probe", None, &none), None);
        assert_eq!(operation_subject("run", Some("--engine"), &none), None);
    }
}

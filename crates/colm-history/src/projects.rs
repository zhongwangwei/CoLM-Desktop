//! History grouped by simulation directory: one entry per project folder (the folder the
//! Project page opens), with that project's operations as members. A multi-site project's
//! stations share the folder; a lone case is its own project. Members stay the immutable
//! audit operations; only the listing is folded.
use super::*;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectGroup {
    /// Local project folder (for server projects, their local configuration cache).
    pub path: String,
    /// What people call it: the server folder's name for server projects, else the folder name.
    pub name: String,
    /// Members, newest first.
    pub members: Vec<Operation>,
}

fn file_name(path: &str) -> Option<String> {
    Path::new(path.trim_end_matches(['/', '\\']))
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
}

/// The project folder an operation belongs to: the recorded `detail.subject.project_path`, else
/// the folder containing its case.
pub(super) fn project_path(event: &Event) -> Option<String> {
    if let Some(path) = event.detail["subject"]["project_path"].as_str() {
        return Some(path.to_owned()).filter(|path| !path.is_empty());
    }
    let case = event.case_path.as_deref()?.trim_end_matches(['/', '\\']);
    Path::new(case)
        .parent()
        .map(|parent| parent.to_string_lossy().into_owned())
        .filter(|parent| !parent.is_empty())
}

/// A server project's local cache is named by a hash; its cases record the server folder.
fn project_name(path: &str, members: &[Operation]) -> String {
    for member in members {
        let Some(case) = member.latest.case_path.as_deref() else {
            continue;
        };
        let record = Path::new(case).join(".colm-desktop.json");
        let Ok(text) = std::fs::read_to_string(record) else {
            continue;
        };
        if let Some(remote) = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|value| value["remote_directory"].as_str().map(str::to_owned))
        {
            if let Some(name) = Path::new(remote.trim_end_matches('/'))
                .parent()
                .and_then(|parent| parent.to_str())
                .and_then(file_name)
            {
                return name;
            }
        }
    }
    file_name(path).unwrap_or_else(|| path.to_owned())
}

/// The same precedence the run grouping uses in SQL.
fn aggregate_status(statuses: &[&str]) -> String {
    let all = |status: &str| statuses.iter().all(|s| *s == status);
    let any = |status: &str| statuses.contains(&status);
    if let Some(first) = statuses.first() {
        if statuses.iter().all(|s| s == first) {
            return (*first).to_owned();
        }
    }
    if any("running") {
        "running"
    } else if statuses.iter().any(|s| {
        !matches!(
            *s,
            "succeeded" | "completed" | "cancelled" | "failed" | "completed_with_failures"
        )
    }) {
        "unknown"
    } else if any("completed_with_failures") {
        "completed_with_failures"
    } else if all("failed") {
        "failed"
    } else if any("failed") {
        "completed_with_failures"
    } else if any("cancelled") {
        "cancelled"
    } else {
        "succeeded"
    }
    .to_owned()
}

fn time_of(operation: &Operation) -> i64 {
    operation
        .started_ms
        .or(operation.latest.time_ms)
        .unwrap_or_default()
}

/// Folds members into one listing entry. Imported placeholders count only when nothing else
/// was recorded: a case import repeats a server job the run record already tracks.
fn fold(members: &mut [Operation]) -> Operation {
    members.sort_by_key(|member| std::cmp::Reverse(time_of(member)));
    let real = members
        .iter()
        .filter(|member| member.latest.phase != EventPhase::Imported)
        .collect::<Vec<_>>();
    let considered = if real.is_empty() {
        members.iter().collect::<Vec<_>>()
    } else {
        real
    };
    let mut operation = considered[0].clone();
    let statuses = considered
        .iter()
        .map(|member| member.latest.status.as_str())
        .collect::<Vec<_>>();
    operation.latest.status = aggregate_status(&statuses);
    operation.needs_reconciliation = considered.iter().any(|m| m.needs_reconciliation);
    operation.started_ms = considered.iter().filter_map(|m| m.started_ms).min();
    operation.finished_ms = (!operation.needs_reconciliation
        && considered.iter().all(|m| m.finished_ms.is_some()))
    .then(|| considered.iter().filter_map(|m| m.finished_ms).max())
    .flatten();
    operation.latest.phase = if operation.needs_reconciliation
        || operation.finished_ms.is_none()
        || !matches!(
            operation.latest.status.as_str(),
            "succeeded" | "completed" | "failed" | "completed_with_failures" | "cancelled"
        ) {
        EventPhase::Status
    } else {
        EventPhase::Finished
    };
    operation.event_count = members.iter().map(|m| m.event_count).sum();
    operation.latest.time_ms = members.iter().map(time_of).max();
    operation.latest.duration_ms = None;
    operation.latest.case_id = None;
    operation.latest.case_path = None;
    operation.latest.command = None;
    operation.latest.parent_operation_id = None;
    operation.latest.source_reference = None;
    macro_rules! common {
        ($field:ident) => {
            if considered
                .iter()
                .any(|m| m.latest.$field != operation.latest.$field)
            {
                operation.latest.$field = None;
            }
        };
    }
    common!(version_id);
    common!(revision);
    common!(host);
    common!(location);
    common!(initiator);
    operation
}

pub(super) fn query_page(
    connection: &Connection,
    query: &Query,
    view: OperationView,
) -> Result<Page> {
    let mut member_query = query.clone();
    member_query.status = None;
    member_query.from_ms = None;
    member_query.to_ms = None;
    member_query.needs_reconciliation = None;
    let (filter, values) = super::filter(connection, &member_query, view)?;
    let mut statement = connection.prepare(&format!(
        "SELECT payload,started_ms,finished_ms,event_count,needs_reconciliation FROM {} WHERE {filter}",
        view.name()
    ))?;
    let operations = statement
        .query_map(rusqlite::params_from_iter(&values), read_operation)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut groups: BTreeMap<String, Vec<Operation>> = BTreeMap::new();
    for operation in operations {
        let key = if let Some(path) = project_path(&operation.latest) {
            format!("project:{path}")
        } else if let Some(run) = grouped::run_id(&operation.latest) {
            format!("run:{run}")
        } else {
            format!("op:{}", operation.latest.operation_id)
        };
        groups.entry(key).or_default().push(operation);
    }
    let mut entries = Vec::with_capacity(groups.len());
    for (key, mut members) in groups {
        let entry = if let Some(path) = key.strip_prefix("project:") {
            let mut operation = fold(&mut members);
            let name = project_name(path, &members);
            operation.latest.kind = "project".into();
            operation.latest.detail = serde_json::json!({
                "project_path": path,
                "subject": { "kind": "project", "name": name },
            });
            operation.project_group = Some(ProjectGroup {
                path: path.to_owned(),
                name,
                members,
            });
            operation
        } else if let (Some(run_id), true) = (key.strip_prefix("run:"), members.len() > 1) {
            let mut operation = fold(&mut members);
            operation.latest.detail = serde_json::json!({ "run_id": run_id });
            operation.run_group = Some(RunGroup {
                run_id: run_id.to_owned(),
                members,
            });
            operation
        } else {
            members.remove(0)
        };
        entries.push(entry);
    }
    entries.retain(|entry| {
        let time = time_of(entry);
        query
            .status
            .as_ref()
            .is_none_or(|status| &entry.latest.status == status)
            && query.from_ms.is_none_or(|from| time >= from)
            && query.to_ms.is_none_or(|to| time <= to)
            && query
                .needs_reconciliation
                .is_none_or(|wanted| entry.needs_reconciliation == wanted)
    });
    entries.sort_by(|a, b| {
        let order = time_of(a)
            .cmp(&time_of(b))
            .then_with(|| a.latest.operation_id.cmp(&b.latest.operation_id));
        if query.ascending {
            order
        } else {
            order.reverse()
        }
    });
    let total = entries.len() as u64;
    let items = entries
        .into_iter()
        .skip(usize::try_from(query.offset).unwrap_or(usize::MAX))
        .take(usize::try_from(query.limit).unwrap_or(usize::MAX))
        .collect();
    Ok(Page { items, total })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn finished(history: &History, kind: &str, case: &str, status: &str, time: i64) {
        let mut event = Event::start(kind);
        event.case_path = Some(case.into());
        event.time_ms = Some(time);
        history.append(&event).unwrap();
        let mut done = event.next(EventPhase::Finished, status);
        done.time_ms = Some(time + 10);
        history.append(&done).unwrap();
    }

    #[test]
    fn one_entry_per_project_folder_with_its_operations_as_members() {
        let root = std::env::temp_dir().join(format!("colm-projects-{}", new_id()));
        let server = root.join("cache-hash");
        for site in ["AT-Neu", "AU-Cpr"] {
            std::fs::create_dir_all(server.join(site)).unwrap();
            std::fs::write(
                server.join(site).join(".colm-desktop.json"),
                json!({"remote_directory": format!("/tera11/colm-desktop/222/{site}")}).to_string(),
            )
            .unwrap();
        }
        let at = server.join("AT-Neu").to_string_lossy().into_owned();
        let au = server.join("AU-Cpr").to_string_lossy().into_owned();
        let h = History::open(":memory:").unwrap();
        finished(&h, "run", &at, "succeeded", 1_000);
        finished(&h, "run", &au, "failed", 2_000);
        // A case import repeats a server job as an unconfirmed placeholder.
        let mut imported =
            Event::start("remote_run").next(EventPhase::Imported, "needs_reconciliation");
        imported.case_path = Some(au.clone());
        imported.time_ms = Some(2_500);
        h.append(&imported).unwrap();
        finished(&h, "run", "/local/single/CN-Cng", "succeeded", 500);
        let mut study = Event::start("study");
        study.time_ms = Some(3_000);
        h.append(&study.next(EventPhase::Finished, "succeeded"))
            .unwrap();

        let page = h
            .query(&Query {
                group_projects: true,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.total, 3, "server project, local project, lone study");
        assert_eq!(page.items[0].latest.kind, "study", "newest activity first");
        let project = page.items[1].project_group.as_ref().unwrap();
        assert_eq!(
            project.name, "222",
            "named by the server folder, not the cache hash"
        );
        assert_eq!(project.members.len(), 3);
        assert_eq!(page.items[1].latest.kind, "project");
        assert_eq!(page.items[1].latest.status, "completed_with_failures");
        assert!(
            !page.items[1].needs_reconciliation,
            "imported placeholders do not decide"
        );
        assert!(page.items[1].latest.case_path.is_none());
        assert_eq!(page.items[2].project_group.as_ref().unwrap().name, "single");

        let failed = h
            .query(&Query {
                group_projects: true,
                status: Some("completed_with_failures".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(failed.total, 1);
        let paged = h
            .query(&Query {
                group_projects: true,
                offset: 1,
                limit: 1,
                ..Default::default()
            })
            .unwrap();
        assert_eq!((paged.total, paged.items.len()), (3, 1));
        assert!(paged.items[0].project_group.is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn statuses_fold_with_the_run_grouping_precedence() {
        assert_eq!(aggregate_status(&["succeeded", "succeeded"]), "succeeded");
        assert_eq!(aggregate_status(&["succeeded", "running"]), "running");
        assert_eq!(
            aggregate_status(&["succeeded", "failed"]),
            "completed_with_failures"
        );
        assert_eq!(aggregate_status(&["failed", "failed"]), "failed");
        assert_eq!(aggregate_status(&["succeeded", "cancelled"]), "cancelled");
        assert_eq!(aggregate_status(&["succeeded", "queued"]), "unknown");
    }
}

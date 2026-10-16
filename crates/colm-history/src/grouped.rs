//! User-facing run projection; immutable station operations remain the audit source.
use super::*;

const RUN_ID: &str = "CASE WHEN json_extract(payload,'$.kind') IN ('run','remote-run') AND json_type(payload,'$.detail.run_id')='text' AND length(trim(json_extract(payload,'$.detail.run_id')))>0 THEN json_extract(payload,'$.detail.run_id') END";

pub(super) fn run_id(event: &Event) -> Option<&str> {
    matches!(event.kind.as_str(), "run" | "remote-run")
        .then(|| event.detail.get("run_id")?.as_str())
        .flatten()
        .filter(|id| !id.trim_matches(' ').is_empty())
}

fn cte(view: OperationView) -> String {
    format!(
        "WITH keyed AS (SELECT *, CASE WHEN ({RUN_ID}) IS NOT NULL THEN 'run:'||({RUN_ID}) ELSE 'op:'||operation_id END group_key FROM {}),
        stats AS (SELECT group_key, count(*) member_count, min(operation_id) operation_id,
            min(started_ms) started_ms,
            CASE WHEN count(finished_ms)=count(*) AND max(needs_reconciliation)=0 THEN max(finished_ms) END finished_ms,
            min(COALESCE(started_ms,json_extract(payload,'$.time_ms'))) sort_ms,
            max(needs_reconciliation) needs_reconciliation,
            CASE
                WHEN count(DISTINCT json_extract(payload,'$.status'))=1 THEN min(json_extract(payload,'$.status'))
                WHEN sum(json_extract(payload,'$.status')='running')>0 THEN 'running'
                WHEN sum(json_extract(payload,'$.status') NOT IN ('succeeded','completed','cancelled','failed','completed_with_failures'))>0 THEN 'unknown'
                WHEN sum(json_extract(payload,'$.status')='completed_with_failures')>0 THEN 'completed_with_failures'
                WHEN sum(json_extract(payload,'$.status')='failed')=count(*) THEN 'failed'
                WHEN sum(json_extract(payload,'$.status')='failed')>0 THEN 'completed_with_failures'
                WHEN sum(json_extract(payload,'$.status')='cancelled')>0 THEN 'cancelled'
                ELSE 'succeeded' END status
            FROM keyed GROUP BY group_key),
        grouped AS (SELECT s.group_key,s.operation_id,s.started_ms,s.finished_ms,s.needs_reconciliation,s.sort_ms,
            json_set(k.payload,'$.status',CASE WHEN s.member_count>1 THEN s.status ELSE json_extract(k.payload,'$.status') END,'$.time_ms',s.sort_ms) payload
            FROM stats s JOIN keyed k ON k.operation_id=s.operation_id) ",
        view.name()
    )
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
    let (member_filter, mut values) = filter(connection, &member_query, view)?;
    let aggregate_query = Query {
        status: query.status.clone(),
        from_ms: query.from_ms,
        to_ms: query.to_ms,
        needs_reconciliation: query.needs_reconciliation,
        ..Query::default()
    };
    let (aggregate_filter, aggregate_values) = filter(connection, &aggregate_query, view)?;
    values.extend(aggregate_values);
    let filter = format!(
        "group_key IN (SELECT group_key FROM keyed WHERE {member_filter}) AND {aggregate_filter}"
    );
    let cte = cte(view);
    let total = connection.query_row(
        &format!("{cte} SELECT count(*) FROM grouped WHERE {filter}"),
        rusqlite::params_from_iter(&values),
        |row| row.get(0),
    )?;
    let order = if query.ascending { "ASC" } else { "DESC" };
    let mut statement = connection.prepare(&format!("{cte} SELECT group_key,json_extract(payload,'$.status'),started_ms,finished_ms,needs_reconciliation FROM grouped WHERE {filter} ORDER BY sort_ms {order},operation_id {order} LIMIT {} OFFSET {}", query.limit,query.offset))?;
    let selected = statement
        .query_map(rusqlite::params_from_iter(&values), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, bool>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut page_members: std::collections::HashMap<String, Vec<Operation>> =
        std::collections::HashMap::with_capacity(selected.len());
    // Bound bindings for SQLite builds with the traditional 999-variable limit.
    for chunk in selected.chunks(500) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let mut statement = connection.prepare(&format!("WITH keyed AS (SELECT *, CASE WHEN ({RUN_ID}) IS NOT NULL THEN 'run:'||({RUN_ID}) ELSE 'op:'||operation_id END group_key FROM {}) SELECT payload,started_ms,finished_ms,event_count,needs_reconciliation,group_key FROM keyed WHERE group_key IN ({placeholders}) ORDER BY operation_id",view.name()))?;
        for member in statement.query_map(
            rusqlite::params_from_iter(chunk.iter().map(|entry| &entry.0)),
            |row| Ok((row.get::<_, String>(5)?, read_operation(row)?)),
        )? {
            let (key, operation) = member?;
            page_members.entry(key).or_default().push(operation);
        }
    }
    let mut items = Vec::with_capacity(selected.len());
    for (key, status, started_ms, finished_ms, needs_reconciliation) in selected {
        let members = page_members
            .remove(&key)
            .context("selected history group has no members")?;
        let mut operation = members
            .first()
            .context("selected history group is empty")?
            .clone();
        if members.len() > 1 {
            operation.latest.status = status;
            operation.started_ms = started_ms;
            operation.finished_ms = finished_ms;
            operation.needs_reconciliation = needs_reconciliation;
            operation.latest.phase = if needs_reconciliation
                || finished_ms.is_none()
                || !matches!(
                    operation.latest.status.as_str(),
                    "succeeded" | "completed" | "failed" | "completed_with_failures" | "cancelled"
                ) {
                EventPhase::Status
            } else {
                EventPhase::Finished
            };
            operation.event_count = members.iter().map(|op| op.event_count).sum();
            operation.latest.time_ms = members.iter().filter_map(|op| op.latest.time_ms).max();
            operation.latest.duration_ms = started_ms
                .zip(finished_ms)
                .and_then(|(s, f)| f.checked_sub(s))
                .and_then(|n| u64::try_from(n).ok());
            operation.latest.case_id = None;
            operation.latest.case_path = None;
            operation.latest.command = None;
            operation.latest.parent_operation_id = None;
            operation.latest.source_reference = None;
            macro_rules! common {
                ($field:ident) => {
                    if members
                        .iter()
                        .any(|op| op.latest.$field != operation.latest.$field)
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
            let run_id = key
                .strip_prefix("run:")
                .context("group without run ID")?
                .to_owned();
            // Station-specific paths and model_run IDs belong only to members.
            operation.latest.detail = serde_json::json!({"run_id": run_id});
            operation.run_group = Some(RunGroup { run_id, members });
        }
        items.push(operation);
    }
    Ok(Page { items, total })
}

impl History {
    /// Hide every visible station operation of this run atomically, retaining audit events.
    pub fn remove_run(&self, operation_id: &str) -> Result<bool> {
        ensure!(!operation_id.is_empty(), "operation ID must be nonempty");
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let event: Event = transaction
            .query_row(
                "SELECT payload FROM history_operations WHERE operation_id=?",
                [operation_id],
                |row| serde_json::from_str(&row.get::<_, String>(0)?).map_err(json_error),
            )
            .optional()?
            .context("history operation not found")?;
        let Some(run_id) = run_id(&event) else {
            transaction.commit()?;
            return self.remove_operation(operation_id);
        };
        let predicate = format!("({RUN_ID})=?1");
        let uncertain: bool = transaction.query_row(&format!("SELECT EXISTS(SELECT 1 FROM history_visible_operations WHERE {predicate} AND (needs_reconciliation!=0 OR json_extract(payload,'$.status') NOT IN ('succeeded','failed','completed','completed_with_failures','cancelled')))"),[run_id],|row| row.get(0))?;
        ensure!(
            !uncertain,
            "cannot remove run whose completion is unconfirmed; reconcile it first"
        );
        let inserted = transaction.execute(&format!("INSERT INTO history_removed_operations(operation_id,removed_ms) SELECT operation_id,?2 FROM history_visible_operations WHERE {predicate} ON CONFLICT(operation_id) DO NOTHING"),params![run_id,now_ms()])?;
        transaction.commit()?;
        Ok(inserted > 0)
    }
}

#[cfg(test)]
mod tests;

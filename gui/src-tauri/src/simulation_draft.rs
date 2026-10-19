//! 新建模拟向导的进度记进历史。向导填的内容只在页面组件里，切到别的页面或关掉软件就没了
//! （用户报：走到第三步切出去就找不回来）；记成一条历史记录，从历史页就能接着填。
use colm_history::{Event, EventPhase, History};
use serde_json::{json, Value};

pub(crate) const KIND: &str = "new-simulation";

/// 每次保存追加一条结束事件，状态用 `paused`：它本来就是"停在中途、可以接着做"的已确认状态，
/// 能删除，也不会被当成待核对。不另造 `draft` 状态，是因为已确认状态的名单在
/// `confirmed_status` 和 `remove_operation` 的 SQL 里各写了一遍，加一个就要两处同步。
fn save(
    ledger: &History,
    id: Option<&str>,
    name: &str,
    step: u8,
    seed: Value,
) -> Result<String, String> {
    let subject = json!({ "kind": "simulation", "name": name });
    let previous = match id {
        // 用户已经在历史里删掉这条时读不到，就另起一条，不让删掉的记录复活。
        Some(id) => ledger
            .detail(id)
            .map_err(|e| e.to_string())?
            .map(|detail| detail.operation.latest)
            .filter(|latest| latest.kind == KIND),
        None => None,
    };
    let mut event = match previous {
        Some(latest) => latest.next(EventPhase::Finished, "paused"),
        None => {
            let mut start = Event::start(KIND);
            start.detail = json!({ "action": KIND, "subject": subject });
            ledger.append(&start).map_err(|e| e.to_string())?;
            start.next(EventPhase::Finished, "paused")
        }
    };
    event.detail = json!({ "action": KIND, "subject": subject, "step": step, "seed": seed });
    ledger.append(&event).map_err(|e| e.to_string())?;
    Ok(event.operation_id)
}

/// 模拟建好后，这条进度就由新项目自己的记录接替。
fn close(ledger: &History, id: &str) -> Result<(), String> {
    let Some(detail) = ledger.detail(id).map_err(|e| e.to_string())? else {
        return Ok(());
    };
    if detail.operation.latest.kind != KIND {
        return Err("不是新建模拟的进度记录".into());
    }
    ledger
        .remove_operation(id)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn new_simulation_draft_save(
    id: Option<String>,
    name: String,
    step: u8,
    seed: Value,
) -> Result<String, String> {
    let ledger = History::open_default().map_err(|e| e.to_string())?;
    save(&ledger, id.as_deref(), &name, step, seed)
}

#[tauri::command]
pub fn new_simulation_draft_close(id: String) -> Result<(), String> {
    close(&History::open_default().map_err(|e| e.to_string())?, &id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visible(ledger: &History) -> Vec<colm_history::Operation> {
        ledger.query(&Default::default()).unwrap().items
    }

    #[test]
    fn one_record_follows_the_wizard_and_closes_when_the_simulation_is_created() {
        let ledger = History::open(":memory:").unwrap();
        let id = save(
            &ledger,
            None,
            "demo",
            2,
            json!({ "form": { "name": "demo" } }),
        )
        .unwrap();
        let again = save(
            &ledger,
            Some(&id),
            "demo",
            3,
            json!({ "form": { "name": "demo" } }),
        );
        assert_eq!(again.unwrap(), id, "later steps update the same record");
        let items = visible(&ledger);
        assert_eq!(items.len(), 1);
        let latest = &items[0].latest;
        assert_eq!(
            (latest.kind.as_str(), latest.status.as_str()),
            (KIND, "paused")
        );
        assert_eq!(latest.detail["step"], 3);
        assert_eq!(latest.detail["seed"]["form"]["name"], "demo");
        assert!(!items[0].needs_reconciliation, "a draft is never 待核对");
        close(&ledger, &id).unwrap();
        assert!(visible(&ledger).is_empty());
        close(&ledger, &id).unwrap();
    }

    #[test]
    fn a_draft_deleted_from_history_is_not_brought_back() {
        let ledger = History::open(":memory:").unwrap();
        let id = save(&ledger, None, "demo", 2, json!({})).unwrap();
        ledger.remove_operation(&id).unwrap();
        let next = save(&ledger, Some(&id), "demo", 4, json!({})).unwrap();
        assert_ne!(next, id);
        assert_eq!(visible(&ledger).len(), 1);
    }

    #[test]
    fn closing_never_hides_another_kind_of_record() {
        let ledger = History::open(":memory:").unwrap();
        let run = Event::start("run");
        ledger.append(&run).unwrap();
        ledger
            .append(&run.next(EventPhase::Finished, "succeeded"))
            .unwrap();
        assert!(close(&ledger, &run.operation_id).is_err());
        assert_eq!(visible(&ledger).len(), 1);
        // Saving under another record's id starts a separate draft instead of extending it.
        let id = save(&ledger, Some(&run.operation_id), "demo", 2, json!({})).unwrap();
        assert_ne!(id, run.operation_id);
    }
}

//! P3 goes through the window so SSH credentials stay in its session.
use super::{object, opt_str, req_str, Tier, Tool, ToolContext};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};

#[derive(Clone, Copy)]
enum Operation {
    Resources,
    Submit,
    Status,
    Cancel,
    Fetch,
}
struct RemoteWorkspace(Operation);

pub(super) fn tools() -> Vec<Box<dyn Tool>> {
    [
        Operation::Resources,
        Operation::Submit,
        Operation::Status,
        Operation::Cancel,
        Operation::Fetch,
    ]
    .into_iter()
    .map(|op| Box::new(RemoteWorkspace(op)) as Box<dyn Tool>)
    .collect()
}
fn string(description: &str) -> Value {
    json!({"type":"string", "description":description})
}
fn nullable(kind: &str, description: &str) -> Value {
    json!({"type":[kind,"null"], "description":description})
}

impl Tool for RemoteWorkspace {
    fn name(&self) -> &'static str {
        match self.0 {
            Operation::Resources => "remote_workspace_resources",
            Operation::Submit => "remote_workspace_submit",
            Operation::Status => "remote_workspace_status",
            Operation::Cancel => "remote_workspace_cancel",
            Operation::Fetch => "remote_workspace_fetch",
        }
    }
    fn description(&self) -> &'static str {
        match self.0 {
            Operation::Resources => "List configured remote servers without credentials. Source edits remain in a local development workspace; remote jobs use committed snapshots.",
            Operation::Submit => "Submit a fixed remote workspace action on a configured Linux server. verify builds candidate and baseline, tests, runs regression and Rust/Fortran parity. case is an existing absolute SERVER case directory; no external data are uploaded. Requires approval. Saves a job ID before launch; submission is not gate success. Inspect status and fetch reports; never resubmit an uncertain job automatically.",
            Operation::Status => "Inspect a remote workspace job's source identity, state, log and stale status; job=null lists saved jobs without contacting the server. Exit 0 alone does not prove all scientific acceptance gates passed.",
            Operation::Cancel => "Cancel only this saved remote workspace job. Closing a conversation does not cancel a detached server job. Requires approval.",
            Operation::Fetch => "Retrieve terminal remote workspace reports into a separate local report directory and verify their identity. Does not adopt Linux binaries or overwrite local gates. Stale, failed or partial comparisons are not current full acceptance. Requires approval.",
        }
    }
    fn parameters(&self) -> Value {
        match self.0 {
            Operation::Resources => object(json!({})),
            Operation::Submit => object(json!({
                "name":string("local workspace name"), "host":string("configured host from remote_workspace_resources"),
                "action":{"type":"string","enum":["build-engine","build-kernel","test","run","parity","regress","verify"]},
                "case":nullable("string","absolute SERVER case directory; required for run/parity/regress/verify"),
                "preset":nullable("string","Fortran preset; null for default"),
                "kind":nullable("string","test: cargo/drift/oracle/check-gui; regress/verify: refactor/physics; null for defaults"),
                "package":nullable("string","test package; null for colm-core"),
                "engine":nullable("string","rust or fortran; null for rust"),
                "rtol":nullable("number","explicit relative parity tolerance; null for bitwise"),
                "atol":nullable("number","explicit absolute parity tolerance; null for bitwise"),
                "first_records":nullable("integer","limited record count; null for all; limited comparison is not full parity"),
            })),
            Operation::Status => object(
                json!({"name":string("workspace name"),"job":nullable("string","saved job ID, or null to list jobs")}),
            ),
            _ => object(json!({"name":string("workspace name"),"job":string("saved job ID")})),
        }
    }
    fn tier(&self) -> Tier {
        match self.0 {
            Operation::Resources | Operation::Status => Tier::Read,
            _ => Tier::Code,
        }
    }
    fn session_allowance(&self) -> bool {
        false
    }
    fn summary(&self, args: &Value) -> String {
        format!(
            "{} · {} · {} · {}",
            self.name(),
            args["name"].as_str().unwrap_or(""),
            args["host"]
                .as_str()
                .or_else(|| args["job"].as_str())
                .unwrap_or(""),
            args["action"].as_str().unwrap_or("")
        )
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let window = ctx.ui.as_ref().context(
            "remote workspace tools require the application window and its server settings",
        )?;
        if matches!(self.0, Operation::Resources) {
            return window.0.request("workspace_remote_servers", json!({}));
        }
        let name = req_str(args, "name")?;
        let (operation, request) = match self.0 {
            Operation::Submit => {
                let action = req_str(args, "action")?;
                ensure!(
                    [
                        "build-engine",
                        "build-kernel",
                        "test",
                        "run",
                        "parity",
                        "regress",
                        "verify"
                    ]
                    .contains(&action),
                    "unsupported remote workspace action"
                );
                let mut request = json!({"action":action});
                for key in [
                    "case",
                    "preset",
                    "kind",
                    "package",
                    "engine",
                    "rtol",
                    "atol",
                    "first_records",
                ] {
                    if let Some(value) = args.get(key).filter(|v| !v.is_null()) {
                        request[key] = value.clone();
                    }
                }
                req_str(args, "host")?;
                ("submit", request)
            }
            Operation::Status => (
                if opt_str(args, "job").is_some() {
                    "status"
                } else {
                    "list"
                },
                Value::Null,
            ),
            Operation::Cancel => {
                req_str(args, "job")?;
                ("cancel", Value::Null)
            }
            Operation::Fetch => {
                req_str(args, "job")?;
                ("fetch", Value::Null)
            }
            Operation::Resources => unreachable!(),
        };
        window.0.request(
            "workspace_remote",
            json!({"name":name,"operation":operation,
            "host":opt_str(args,"host"),"job":opt_str(args,"job"),"request":request}),
        )
    }
}

#[cfg(test)]
#[path = "remote_workspace_tests.rs"]
mod tests;

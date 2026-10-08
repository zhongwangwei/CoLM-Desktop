//! C 级工具：开发工作区里的代码操作（docs/design-ai-assistant.md 第 4–6 节）。
//!
//! 内置后端**没有任意 shell**：这里每个工具都对应 `colm-cli ws-*` 的一个预先定义好的子命令，参数受校验。
//! 读代码、搜索、对比不用审批；改文件（补丁、撤回）、建工作区**每次都问**，不提供“本会话都允许”；编译、测试、
//! 运行、对齐检查按会话批一次。把实验内核设为默认、导出补丁、删除工作区是 D 级：只有界面上的按钮，没有工具。

use anyhow::{bail, Result};
use serde_json::{json, Value};

use super::{object, opt_str, req_str, Tier, Tool, ToolContext};

pub(super) fn tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(WorkspaceList),
        Box::new(WorkspaceStatus),
        Box::new(WorkspaceCreate),
        Box::new(SearchCode),
        Box::new(ReadFile),
        Box::new(ListSymbols),
        Box::new(ApplyPatch),
        Box::new(Revert),
        Box::new(BuildEngine),
        Box::new(BuildKernel),
        Box::new(RunTests),
        Box::new(RunCaseWith),
        Box::new(CompareOutputs),
        Box::new(ParityCheck),
        Box::new(RegressionCheck),
    ]
}

fn string(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

fn nullable(kind: &str, description: &str) -> Value {
    json!({ "type": [kind, "null"], "description": description })
}

fn name_arg() -> Value {
    string("workspace name (see workspace_list)")
}

/// 调 `colm-cli ws-<命令>`，工作区根目录来自上下文（不给就用 colm-cli 的默认）。
fn ws(ctx: &ToolContext, command: &str, args: &[&str], long: bool) -> Result<Value> {
    let mut all: Vec<String> = vec![command.to_owned()];
    all.extend(args.iter().map(|a| (*a).to_owned()));
    if let Some(root) = &ctx.workspace_root {
        all.extend(["--root".to_owned(), root.display().to_string()]);
    }
    let refs: Vec<&str> = all.iter().map(String::as_str).collect();
    if long {
        let done = ctx.cli_long(&refs)?;
        if !done.success {
            bail!(
                "{command} failed: {}",
                if done.stderr_tail.trim().is_empty() {
                    &done.stdout_tail
                } else {
                    &done.stderr_tail
                }
            );
        }
        serde_json::from_str(done.stdout_tail.lines().last().unwrap_or("{}")).map_err(|e| {
            anyhow::anyhow!("{command} did not print JSON ({e}): {}", done.stdout_tail)
        })
    } else {
        ctx.cli_json(&refs)
    }
}

struct WorkspaceList;
impl Tool for WorkspaceList {
    fn name(&self) -> &'static str {
        "workspace_list"
    }
    fn description(&self) -> &'static str {
        "List the development workspaces: name, head commit, number of own commits, whether there are uncommitted changes, and the state of the four gates (compile, tests, regression, parity) on the current commit."
    }
    fn parameters(&self) -> Value {
        object(json!({}))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, _args: &Value, ctx: &ToolContext) -> Result<Value> {
        ws(ctx, "ws-list", &[], false)
    }
}

struct WorkspaceStatus;
impl Tool for WorkspaceStatus {
    fn name(&self) -> &'static str {
        "workspace_status"
    }
    fn description(&self) -> &'static str {
        "Show one workspace: its commits since the base, changed files with added/removed lines, built kernels, the gates on the current commit, and which sandbox compiles and tests run in."
    }
    fn parameters(&self) -> Value {
        object(json!({ "name": name_arg() }))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        ws(ctx, "ws-status", &["--name", req_str(args, "name")?], false)
    }
}

struct WorkspaceCreate;
impl Tool for WorkspaceCreate {
    fn name(&self) -> &'static str {
        "workspace_create"
    }
    fn description(&self) -> &'static str {
        "Create a development workspace: an isolated git clone of the CoLM sources (Fortran upstream under vendor/CoLM202X and the Rust engine under crates/) on its own branch ws/<name>. All code changes happen there; the installed kernels and the application are never touched. `from` is a local git repository, a repository URL or a colm-src.tar.gz; `rev` a tag, branch or commit (null for the current one). Needs approval."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "name": string("new workspace name: letters, digits, _ and -, at most 40 characters"),
            "from": string("local git repository path, repository URL, or colm-src.tar.gz"),
            "rev": nullable("string", "tag, branch or commit; null for the source's current commit"),
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Code
    }
    fn session_allowance(&self) -> bool {
        false
    }
    fn summary(&self, args: &Value) -> String {
        format!(
            "新建开发工作区 {}，来自 {}{}",
            args["name"].as_str().unwrap_or("?"),
            args["from"].as_str().unwrap_or("?"),
            opt_str(args, "rev")
                .map(|r| format!("（{r}）"))
                .unwrap_or_default()
        )
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let mut cli = vec![
            "--name",
            req_str(args, "name")?,
            "--from",
            req_str(args, "from")?,
        ];
        if let Some(rev) = opt_str(args, "rev") {
            cli.extend(["--rev", rev]);
        }
        ws(ctx, "ws-create", &cli, false)
    }
}

struct SearchCode;
impl Tool for SearchCode {
    fn name(&self) -> &'static str {
        "search_code"
    }
    fn description(&self) -> &'static str {
        "Search the workspace sources with an extended regular expression (git grep). Returns path, line and text of up to 200 hits; `glob` limits the paths, e.g. *.F90 or crates/colm-core/*."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "name": name_arg(),
            "pattern": string("extended regular expression"),
            "glob": nullable("string", "path glob such as *.F90, or null"),
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let mut cli = vec![
            "--name",
            req_str(args, "name")?,
            "--pattern",
            req_str(args, "pattern")?,
        ];
        if let Some(glob) = opt_str(args, "glob") {
            cli.extend(["--glob", glob]);
        }
        ws(ctx, "ws-search", &cli, false)
    }
}

struct ReadFile;
impl Tool for ReadFile {
    fn name(&self) -> &'static str {
        "read_file"
    }
    fn description(&self) -> &'static str {
        "Read lines of a text file in the workspace (path relative to the repository root), with line numbers. At most 400 lines per call; give from_line and to_line (1-based, inclusive)."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "name": name_arg(),
            "path": string("file path relative to the repository root"),
            "from_line": nullable("integer", "first line, 1-based; null for 1"),
            "to_line": nullable("integer", "last line; null for 400 lines from from_line"),
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let from = args["from_line"].as_u64().map(|n| n.to_string());
        let to = args["to_line"].as_u64().map(|n| n.to_string());
        let mut cli = vec![
            "--name",
            req_str(args, "name")?,
            "--path",
            req_str(args, "path")?,
        ];
        if let Some(from) = &from {
            cli.extend(["--from", from.as_str()]);
        }
        if let Some(to) = &to {
            cli.extend(["--to", to.as_str()]);
        }
        ws(ctx, "ws-read", &cli, false)
    }
}

struct ListSymbols;
impl Tool for ListSymbols {
    fn name(&self) -> &'static str {
        "list_symbols"
    }
    fn description(&self) -> &'static str {
        "List the symbols of a Fortran (.F90) or Rust (.rs) file: modules, subroutines, functions, types, structs, enums, impls, consts, with line numbers. Use it to find where to read before reading."
    }
    fn parameters(&self) -> Value {
        object(
            json!({ "name": name_arg(), "path": string("file path relative to the repository root") }),
        )
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        ws(
            ctx,
            "ws-symbols",
            &[
                "--name",
                req_str(args, "name")?,
                "--path",
                req_str(args, "path")?,
            ],
            false,
        )
    }
}

/// 补丁的路径行（给审批卡片列出会改哪些文件）。
fn patch_files(diff: &str) -> Vec<String> {
    let mut files = Vec::new();
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("+++ ") {
            let path = rest.split('\t').next().unwrap_or(rest).trim();
            if path != "/dev/null" {
                let path = path.strip_prefix("b/").unwrap_or(path).to_owned();
                if !files.contains(&path) {
                    files.push(path);
                }
            }
        }
    }
    files
}

struct ApplyPatch;
impl Tool for ApplyPatch {
    fn name(&self) -> &'static str {
        "apply_patch"
    }
    fn description(&self) -> &'static str {
        "Apply a unified diff to the workspace sources and commit it (one commit per patch). Paths are relative to the repository root; both Fortran (vendor/CoLM202X/...) and Rust (crates/...) are open. The Rust engine and the Fortran kernel are kept bit-for-bit aligned, so a physics change normally needs a matching patch on BOTH sides, then parity_check. oracle/golden/ and .git cannot be patched. Every patch needs the user's approval and the card shows the diff."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "name": name_arg(),
            "message": string("one-line commit message"),
            "diff": string("the unified diff (git diff format, a/ b/ paths)"),
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Code
    }
    fn session_allowance(&self) -> bool {
        false
    }
    fn summary(&self, args: &Value) -> String {
        let diff = args["diff"].as_str().unwrap_or_default();
        let shown: String = diff.chars().take(6000).collect();
        format!(
            "在工作区 {} 打补丁：{}\n改动的文件：{}\n{}{}",
            args["name"].as_str().unwrap_or("?"),
            args["message"].as_str().unwrap_or("?"),
            patch_files(diff).join("、"),
            shown,
            if diff.chars().count() > 6000 {
                "\n…（补丁太长，只显示前 6000 个字符）"
            } else {
                ""
            }
        )
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let diff = req_str(args, "diff")?;
        let file = std::env::temp_dir().join(format!(
            "colm-patch-{}-{}.diff",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::write(&file, diff)?;
        let path = file.display().to_string();
        let result = ws(
            ctx,
            "ws-patch",
            &[
                "--name",
                req_str(args, "name")?,
                "--message",
                req_str(args, "message")?,
                "--diff-file",
                &path,
            ],
            false,
        );
        let _ = std::fs::remove_file(&file);
        result
    }
}

struct Revert;
impl Tool for Revert {
    fn name(&self) -> &'static str {
        "revert"
    }
    fn description(&self) -> &'static str {
        "Reset the workspace to one of its own earlier commits (the base commit or a later one); later commits are discarded. Needs approval every time."
    }
    fn parameters(&self) -> Value {
        object(
            json!({ "name": name_arg(), "commit": string("commit id (hex) from workspace_status") }),
        )
    }
    fn tier(&self) -> Tier {
        Tier::Code
    }
    fn session_allowance(&self) -> bool {
        false
    }
    fn summary(&self, args: &Value) -> String {
        format!(
            "把工作区 {} 撤回到提交 {}（之后的提交会被丢弃）",
            args["name"].as_str().unwrap_or("?"),
            args["commit"].as_str().unwrap_or("?")
        )
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        ws(
            ctx,
            "ws-revert",
            &[
                "--name",
                req_str(args, "name")?,
                "--commit",
                req_str(args, "commit")?,
            ],
            false,
        )
    }
}

struct BuildEngine;
impl Tool for BuildEngine {
    fn name(&self) -> &'static str {
        "build_engine"
    }
    fn description(&self) -> &'static str {
        "Build the Rust engine (colm-cli, mksrfdata-rs, mkinidata-rs, colm-rs) from the workspace sources in release mode, offline, inside the sandbox (no network, writes only to the workspace). The first build takes minutes. Records the compile gate on the current commit. Approval can be given once per session."
    }
    fn parameters(&self) -> Value {
        object(json!({ "name": name_arg() }))
    }
    fn tier(&self) -> Tier {
        Tier::Code
    }
    fn summary(&self, args: &Value) -> String {
        format!(
            "在工作区 {} 里编译 Rust 引擎（沙箱，断网）",
            args["name"].as_str().unwrap_or("?")
        )
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        ws(
            ctx,
            "ws-build-engine",
            &["--name", req_str(args, "name")?],
            true,
        )
    }
}

struct BuildKernel;
impl Tool for BuildKernel {
    fn name(&self) -> &'static str {
        "build_kernel"
    }
    fn description(&self) -> &'static str {
        "Build a Fortran kernel preset (default, usgs, crop, latlon, unstructured, catchment, … ) from the workspace sources with build_kernel.sh, inside the sandbox. Needs gfortran and netCDF-Fortran (spatial presets also MPI); check with environment_doctor first. Takes minutes. Records the compile gate for that preset. Approval can be given once per session."
    }
    fn parameters(&self) -> Value {
        object(
            json!({ "name": name_arg(), "preset": string("kernel preset, e.g. default or latlon") }),
        )
    }
    fn tier(&self) -> Tier {
        Tier::Code
    }
    fn summary(&self, args: &Value) -> String {
        format!(
            "在工作区 {} 里编译 Fortran 内核 {}（沙箱，断网）",
            args["name"].as_str().unwrap_or("?"),
            args["preset"].as_str().unwrap_or("?")
        )
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        ws(
            ctx,
            "ws-build-kernel",
            &[
                "--name",
                req_str(args, "name")?,
                "--preset",
                req_str(args, "preset")?,
            ],
            true,
        )
    }
}

struct RunTests;
impl Tool for RunTests {
    fn name(&self) -> &'static str {
        "run_tests"
    }
    fn description(&self) -> &'static str {
        "Run one whitelisted kind of tests in the workspace sandbox: kind=cargo with a package (cargo test -p <package> --lib --bins, serial), kind=oracle (history gate tests and the tier check on the golden files), or kind=check-gui. Records the test gate for that kind on the current commit. Approval can be given once per session."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "name": name_arg(),
            "kind": json!({ "type": "string", "enum": ["cargo", "oracle", "check-gui"], "description": "which tests" }),
            "package": nullable("string", "crate name for kind=cargo, e.g. colm-core; null otherwise"),
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Code
    }
    fn summary(&self, args: &Value) -> String {
        format!(
            "在工作区 {} 里运行测试：{}{}",
            args["name"].as_str().unwrap_or("?"),
            args["kind"].as_str().unwrap_or("?"),
            opt_str(args, "package")
                .map(|p| format!(" {p}"))
                .unwrap_or_default()
        )
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let mut cli = vec![
            "--name",
            req_str(args, "name")?,
            "--kind",
            req_str(args, "kind")?,
        ];
        if let Some(package) = opt_str(args, "package") {
            cli.extend(["--package", package]);
        }
        ws(ctx, "ws-test", &cli, true)
    }
}

struct RunCaseWith;
impl Tool for RunCaseWith {
    fn name(&self) -> &'static str {
        "run_case_with"
    }
    fn description(&self) -> &'static str {
        "Run a COPY of a case with the engine or kernel built in the workspace (runs/<id>/<engine>/ inside the workspace; the original case is untouched). engine=rust uses the workspace's colm-rs, engine=fortran its Fortran kernel for the given preset. Returns the history directory for compare_outputs. Approval can be given once per session."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "name": name_arg(),
            "case": string("case directory to copy and run"),
            "engine": json!({ "type": "string", "enum": ["rust", "fortran"], "description": "which engine runs the colm stage" }),
            "preset": string("kernel preset built in the workspace, e.g. default"),
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Code
    }
    fn summary(&self, args: &Value) -> String {
        format!(
            "用工作区 {} 编出的{}跑算例 {} 的副本（预设 {}）",
            args["name"].as_str().unwrap_or("?"),
            if args["engine"] == "fortran" {
                "Fortran 内核"
            } else {
                "Rust 引擎"
            },
            args["case"].as_str().unwrap_or("?"),
            args["preset"].as_str().unwrap_or("?")
        )
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let case = ctx.resolve(req_str(args, "case")?).display().to_string();
        ws(
            ctx,
            "ws-run",
            &[
                "--name",
                req_str(args, "name")?,
                "--case",
                &case,
                "--engine",
                req_str(args, "engine")?,
                "--preset",
                req_str(args, "preset")?,
            ],
            true,
        )
    }
}

struct CompareOutputs;
impl Tool for CompareOutputs {
    fn name(&self) -> &'static str {
        "compare_outputs"
    }
    fn description(&self) -> &'static str {
        "Compare two history directories (or two NetCDF files) variable by variable: bitwise identical, within tolerance, or different; flags new NaN/Inf and names the first differing variable and time step. rtol/atol are optional (null = bitwise only)."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "a": string("first history directory or file (the reference)"),
            "b": string("second history directory or file"),
            "rtol": nullable("number", "relative tolerance, or null"),
            "atol": nullable("number", "absolute tolerance, or null"),
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Read
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let a = ctx.resolve(req_str(args, "a")?).display().to_string();
        let b = ctx.resolve(req_str(args, "b")?).display().to_string();
        let rtol = args["rtol"].as_f64().map(|v| v.to_string());
        let atol = args["atol"].as_f64().map(|v| v.to_string());
        let mut cli = vec!["--a", a.as_str(), "--b", b.as_str()];
        if let Some(rtol) = &rtol {
            cli.extend(["--rtol", rtol.as_str()]);
        }
        if let Some(atol) = &atol {
            cli.extend(["--atol", atol.as_str()]);
        }
        ws(ctx, "ws-compare", &cli, false)
    }
}

struct ParityCheck;
impl Tool for ParityCheck {
    fn name(&self) -> &'static str {
        "parity_check"
    }
    fn description(&self) -> &'static str {
        "Run the same case (copies) with the workspace's Rust engine and its Fortran kernel and compare the histories bit for bit; report the first variable and time step that differ. Needs the engine built (build_engine) and the preset's kernel built (build_kernel). Both engines use the same Rust preprocessing, so a difference comes from the colm main loop. Records the parity gate. Approval can be given once per session."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "name": name_arg(),
            "case": string("case directory (a small single-site case is best)"),
            "preset": string("kernel preset built in the workspace, e.g. default"),
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Code
    }
    fn summary(&self, args: &Value) -> String {
        format!(
            "在工作区 {} 里对算例 {} 做 Rust/Fortran 逐位对齐检查（预设 {}）",
            args["name"].as_str().unwrap_or("?"),
            args["case"].as_str().unwrap_or("?"),
            args["preset"].as_str().unwrap_or("?")
        )
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let case = ctx.resolve(req_str(args, "case")?).display().to_string();
        ws(
            ctx,
            "ws-parity",
            &[
                "--name",
                req_str(args, "name")?,
                "--case",
                &case,
                "--preset",
                req_str(args, "preset")?,
            ],
            true,
        )
    }
}

struct RegressionCheck;
impl Tool for RegressionCheck {
    fn name(&self) -> &'static str {
        "regression_check"
    }
    fn description(&self) -> &'static str {
        "The regression gate: run a reference case with the baseline (the application's own colm-cli and the currently selected kernel) and with the workspace build, then compare. kind=refactor demands bitwise identical output; kind=physics lists which variables changed and by how much, and fails on NaN/Inf or when the water balance error (f_xerr, mm/s) or the energy balance error (f_zerr, W/m2) gets worse. Records the regression gate. Approval can be given once per session."
    }
    fn parameters(&self) -> Value {
        object(json!({
            "name": name_arg(),
            "case": string("reference case directory (default: a small single-site case with its own forcing)"),
            "preset": string("kernel preset, e.g. default"),
            "kind": json!({ "type": "string", "enum": ["refactor", "physics"], "description": "refactor claims no physics change; physics changes results on purpose" }),
            "engine": nullable("string", "rust or fortran for the colm stage; null for rust"),
        }))
    }
    fn tier(&self) -> Tier {
        Tier::Code
    }
    fn summary(&self, args: &Value) -> String {
        format!(
            "在工作区 {} 里做回归检查：算例 {}，类型 {}",
            args["name"].as_str().unwrap_or("?"),
            args["case"].as_str().unwrap_or("?"),
            args["kind"].as_str().unwrap_or("?")
        )
    }
    fn call(&self, args: &Value, ctx: &ToolContext) -> Result<Value> {
        let case = ctx.resolve(req_str(args, "case")?).display().to_string();
        let kernel = ctx
            .kernel_dir
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no kernel is selected in the application; it is the baseline for the comparison"))?
            .display()
            .to_string();
        let baseline_cli = ctx.cli.display().to_string();
        let mut cli = vec![
            "--name",
            req_str(args, "name")?,
            "--case",
            case.as_str(),
            "--preset",
            req_str(args, "preset")?,
            "--kind",
            req_str(args, "kind")?,
            "--baseline-cli",
            baseline_cli.as_str(),
            "--baseline-kernel",
            kernel.as_str(),
        ];
        if let Some(engine) = opt_str(args, "engine") {
            cli.extend(["--engine", engine]);
        }
        ws(ctx, "ws-regress", &cli, true)
    }
}

#[cfg(test)]
#[path = "code_tests.rs"]
mod code_tests;

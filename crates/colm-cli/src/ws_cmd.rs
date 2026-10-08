//! `colm-cli ws-*`：开发工作区（docs/design-ai-assistant.md 第 4–6 节）。助手的 C 级工具和 GUI 的工作区面板都调它们。
//!
//! ```text
//! colm-cli ws-create   --name N --from <本地仓库|地址|源码包> [--rev R] [--root DIR]
//! colm-cli ws-list     [--root DIR]
//! colm-cli ws-status   --name N [--root DIR]
//! colm-cli ws-search   --name N --pattern RE [--glob G]
//! colm-cli ws-read     --name N --path P [--from L] [--to L]
//! colm-cli ws-symbols  --name N --path P
//! colm-cli ws-patch    --name N --message M (--diff-file F | --diff TEXT)
//! colm-cli ws-revert   --name N --commit HEX
//! colm-cli ws-build-engine --name N [--network 1]
//! colm-cli ws-build-kernel --name N --preset P
//! colm-cli ws-test     --name N --kind cargo|oracle|check-gui [--package P]
//! colm-cli ws-run      --name N --case DIR --engine rust|fortran --preset P
//! colm-cli ws-compare  --a DIR --b DIR [--rtol X] [--atol X]
//! colm-cli ws-parity   --name N --case DIR --preset P
//! colm-cli ws-regress  --name N --case DIR --preset P --kind refactor|physics
//!                      --baseline-cli F --baseline-kernel DIR [--engine rust|fortran]
//! colm-cli ws-kernels  [--root DIR]
//! colm-cli ws-export   --name N [--out FILE]            # D 级
//! colm-cli ws-delete   --name N                          # D 级
//! ```
//!
//! `--root` 不给就用 `COLM_WORKSPACES` 或 `~/CoLM-Workspaces`。结果都是一行 JSON。

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use colm_workspace::compare::Tolerance;
use colm_workspace::gates::ChangeKind;
use colm_workspace::parity::Baseline;
use colm_workspace::testrun::Kind;
use colm_workspace::{build, code, compare, kernels, layout, parity, patch, testrun, Workspace};
use serde_json::json;

use super::Opts;

fn root(opts: &Opts) -> PathBuf {
    opts.get("--root")
        .map(PathBuf::from)
        .unwrap_or_else(layout::default_root)
}

fn open(opts: &Opts) -> Result<Workspace> {
    Workspace::open(&root(opts), &opts.need_str("--name")?)
}

fn print(value: serde_json::Value) {
    println!("{value}");
}

fn flag(opts: &Opts, name: &str) -> bool {
    opts.get(name).is_some_and(|v| v == "1")
}

fn number(opts: &Opts, name: &str) -> Result<Option<usize>> {
    opts.get(name)
        .map(|v| v.parse().with_context(|| format!("{name} must be a whole number")))
        .transpose()
}

pub(super) fn dispatch(command: &str, opts: &Opts) -> Result<()> {
    match command {
        "ws-create" => {
            let workspace = Workspace::create(
                &root(opts),
                &opts.need_str("--name")?,
                &opts.need_str("--from")?,
                opts.get("--rev").as_deref(),
            )?;
            print(json!({ "workspace": workspace.info, "dir": workspace.dir }));
        }
        "ws-list" => print(json!({ "workspaces": Workspace::list(&root(opts))? })),
        "ws-status" => {
            let workspace = open(opts)?;
            let src = workspace.src();
            let head = workspace.head()?;
            print(json!({
                "workspace": workspace.info,
                "head": head,
                "lights": workspace.info.gates.lights(&head),
                "commits": colm_workspace::git::commits_since(&src, &workspace.info.base_commit)?,
                "changed_files": colm_workspace::git::changed_files(&src, &workspace.info.base_commit)?,
                "dirty": colm_workspace::git::is_dirty(&src)?,
                "built_kernels": kernels::built_presets(&workspace),
                "sandbox": colm_workspace::sandbox::detect(),
            }));
        }
        "ws-search" => {
            let hits = code::search(
                &open(opts)?,
                &opts.need_str("--pattern")?,
                opts.get("--glob").as_deref(),
            )?;
            print(json!({ "hits": hits, "truncated": hits.len() >= code::MAX_HITS }));
        }
        "ws-read" => {
            let from = number(opts, "--from")?.unwrap_or(1);
            let to = number(opts, "--to")?.unwrap_or(from + code::MAX_READ_LINES - 1);
            print(json!(code::read_lines(&open(opts)?, &opts.need_str("--path")?, from, to)?));
        }
        "ws-symbols" => print(json!({
            "symbols": code::symbols(&open(opts)?, &opts.need_str("--path")?)?
        })),
        "ws-patch" => {
            let workspace = open(opts)?;
            let diff = match (opts.get("--diff-file"), opts.get("--diff")) {
                (Some(file), _) => std::fs::read_to_string(&file)
                    .with_context(|| format!("cannot read {file}"))?,
                (None, Some(text)) => text,
                (None, None) => bail!("give the patch with --diff-file FILE or --diff TEXT"),
            };
            print(json!(patch::apply(&workspace, &diff, &opts.need_str("--message")?)?));
        }
        "ws-revert" => {
            let workspace = open(opts)?;
            let now = patch::revert_to(&workspace, &opts.need_str("--commit")?)?;
            print(json!({ "head": now }));
        }
        "ws-build-engine" => {
            let mut workspace = open(opts)?;
            print(json!(build::build_engine(&mut workspace, flag(opts, "--network"), None)?));
        }
        "ws-build-kernel" => {
            let mut workspace = open(opts)?;
            print(json!(build::build_kernel(&mut workspace, &opts.need_str("--preset")?, None)?));
        }
        "ws-test" => {
            let mut workspace = open(opts)?;
            let kind = Kind::parse(&opts.need_str("--kind")?, opts.get("--package").as_deref())?;
            print(json!(testrun::run(&mut workspace, &kind, None)?));
        }
        "ws-run" => {
            let workspace = open(opts)?;
            print(json!(parity::run_case_with(
                &workspace,
                &PathBuf::from(opts.need_str("--case")?),
                &opts.need_str("--engine")?,
                &opts.need_str("--preset")?,
                None,
            )?));
        }
        "ws-compare" => {
            let tolerance = Tolerance {
                rtol: opts.get("--rtol").map(|v| v.parse()).transpose()?.unwrap_or(0.0),
                atol: opts.get("--atol").map(|v| v.parse()).transpose()?.unwrap_or(0.0),
            };
            let report = compare::compare(
                &PathBuf::from(opts.need_str("--a")?),
                &PathBuf::from(opts.need_str("--b")?),
                tolerance,
            )?;
            print(json!({ "bitwise_identical": report.bitwise_identical(), "report": report }));
        }
        "ws-parity" => {
            let mut workspace = open(opts)?;
            print(json!(parity::parity_check(
                &mut workspace,
                &PathBuf::from(opts.need_str("--case")?),
                &opts.need_str("--preset")?,
                None,
            )?));
        }
        "ws-regress" => {
            let mut workspace = open(opts)?;
            let kind = match opts.need_str("--kind")?.as_str() {
                "refactor" => ChangeKind::Refactor,
                "physics" => ChangeKind::Physics,
                other => bail!("--kind must be refactor or physics, got {other:?}"),
            };
            let baseline = Baseline {
                cli: PathBuf::from(opts.need_str("--baseline-cli")?),
                kernel: PathBuf::from(opts.need_str("--baseline-kernel")?),
            };
            print(json!(parity::regress(
                &mut workspace,
                &PathBuf::from(opts.need_str("--case")?),
                &opts.need_str("--preset")?,
                &opts.get("--engine").unwrap_or_else(|| "rust".into()),
                &baseline,
                kind,
                None,
            )?));
        }
        "ws-kernels" => print(json!({ "kernels": kernels::experimental(&root(opts))? })),
        "ws-export" => {
            let mut workspace = open(opts)?;
            let diff = patch::export(&workspace)?;
            let out = opts.get("--out").map(PathBuf::from);
            if let Some(path) = &out {
                std::fs::write(path, format!("{diff}\n"))
                    .with_context(|| format!("cannot write {}", path.display()))?;
            }
            workspace.record_adoption(
                "export",
                &out.as_ref().map_or_else(|| "stdout".into(), |p| p.display().to_string()),
            )?;
            print(json!({ "bytes": diff.len(), "out": out, "patch": if out.is_none() { Some(diff) } else { None } }));
        }
        "ws-delete" => {
            let name = opts.need_str("--name")?;
            Workspace::delete(&root(opts), &name)?;
            print(json!({ "deleted": name }));
        }
        other => bail!("unknown workspace command {other}"),
    }
    Ok(())
}

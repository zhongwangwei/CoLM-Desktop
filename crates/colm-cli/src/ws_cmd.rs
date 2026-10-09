//! `colm-cli ws-*`：开发工作区（docs/design-ai-assistant.md 第 4–6 节）。助手的 C 级工具和 GUI 的工作区面板都调它们。
//!
//! ```text
//! colm-cli ws-create   --name N --from <本地仓库|地址|源码包> [--rev R] [--root DIR]
//! colm-cli ws-list     [--root DIR]
//! colm-cli ws-status   --name N [--root DIR]
//! colm-cli ws-search   (--name N | --source DIR) --pattern RE [--glob G]
//! colm-cli ws-read     (--name N | --source DIR) --path P [--from L] [--to L]
//! colm-cli ws-symbols  (--name N | --source DIR) --path P      # --source：应用自己的源码，只读，不建工作区
//! colm-cli ws-patch    --name N --message M (--diff-file F | --diff TEXT)
//! colm-cli ws-revert   --name N --commit HEX
//! colm-cli ws-build-engine --name N [--network 1]
//! colm-cli ws-build-kernel --name N --preset P
//! colm-cli ws-test     --name N --kind cargo|drift|oracle|check-gui [--package P]
//! colm-cli ws-run      --name N --case DIR --engine rust|fortran --preset P
//! colm-cli ws-compare  --a DIR --b DIR [--rtol X] [--atol X] [--first-records K] [--ignore f_a,f_b]
//! colm-cli ws-parity   --name N --case DIR --preset P [--rtol X] [--atol X]
//!                        [--first-records K] [--ignore f_a,f_b]    # x86_64：只看最早的 K 条记录并忽略对阈值敏感的诊断量
//! colm-cli ws-regress  --name N --case DIR --preset P --kind refactor|physics
//!                      --baseline-cli F --baseline-kernel DIR [--engine rust|fortran]
//! colm-cli ws-kernels  [--root DIR]
//! colm-cli ws-export   --name N [--out FILE]            # D 级
//! colm-cli ws-adopt    --name N --preset P                # D 级：把实验内核设为默认（只记一笔采纳）
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

/// 只读代码命令的根目录：`--source app`（应用自己正在运行的那份源码）、`--source DIR`，或 `--name N`（工作区的 `src/`）。
fn source_root(opts: &Opts) -> Result<PathBuf> {
    match opts.get("--source").as_deref() {
        Some("app") => super::remote_cmd::app_source_dir(),
        Some(dir) => Ok(PathBuf::from(dir)),
        None => Ok(open(opts)?.src()),
    }
}

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
        .map(|v| {
            v.parse()
                .with_context(|| format!("{name} must be a whole number"))
        })
        .transpose()
}

/// 对比报告的摘要：计数、第一个差异、差得最多的十个变量。完整报告写在 `reports/` 里，路径随结果给出。
fn compact(report: &colm_workspace::compare::Report) -> serde_json::Value {
    json!({
        "files": report.files,
        "identical": report.identical,
        "within_tolerance": report.within_tolerance,
        "differs": report.differs,
        "new_nonfinite": report.new_nonfinite,
        "only_in_a": report.only_in_a,
        "only_in_b": report.only_in_b,
        "first_difference": report.first_difference,
        "largest_changes": report.largest_changes(10),
    })
}

/// 一次运行的摘要：成败、日志路径、耗时；失败时带输出末尾。
fn run_summary(run: &colm_workspace::parity::RunResult) -> serde_json::Value {
    json!({
        "engine": run.engine,
        "ok": run.outcome.ok,
        "seconds": run.outcome.seconds,
        "log": run.outcome.log,
        "case_copy": run.case_copy,
        "history": run.history,
        "tail": if run.outcome.ok { None } else { Some(&run.outcome.tail) },
    })
}

/// `--rtol`、`--atol`、`--first-records`、`--ignore a,b` 合成对比选项。
fn compare_options(opts: &Opts) -> Result<compare::Options> {
    let number = |name: &str| -> Result<f64> {
        Ok(opts
            .get(name)
            .map(|v| v.parse())
            .transpose()
            .with_context(|| format!("{name} must be a number"))?
            .unwrap_or(0.0))
    };
    Ok(compare::Options {
        tolerance: Tolerance {
            rtol: number("--rtol")?,
            atol: number("--atol")?,
        },
        first_records: self::number(opts, "--first-records")?,
        ignore: opts
            .get("--ignore")
            .map(|v| {
                v.split(',')
                    .map(|n| n.trim().to_owned())
                    .filter(|n| !n.is_empty())
                    .collect()
            })
            .unwrap_or_default(),
    })
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
                "lights": workspace.info.gates.lights_for(&head, workspace.parity_needed()?),
                "parity_needed": workspace.parity_needed()?,
                "commits": colm_workspace::git::commits_since(&src, &workspace.info.base_commit)?,
                "changed_files": colm_workspace::git::changed_files(&src, &workspace.info.base_commit)?,
                "dirty": colm_workspace::git::is_dirty(&src)?,
                "built_kernels": kernels::built_presets(&workspace),
                "sandbox": colm_workspace::sandbox::detect(),
            }));
        }
        "ws-search" => {
            let hits = code::search_in(
                &source_root(opts)?,
                &opts.need_str("--pattern")?,
                opts.get("--glob").as_deref(),
            )?;
            print(json!({ "hits": hits, "truncated": hits.len() >= code::MAX_HITS }));
        }
        "ws-read" => {
            let from = number(opts, "--from")?.unwrap_or(1);
            let to = number(opts, "--to")?.unwrap_or(from + code::MAX_READ_LINES - 1);
            print(json!(code::read_lines_in(
                &source_root(opts)?,
                &opts.need_str("--path")?,
                from,
                to
            )?));
        }
        "ws-symbols" => print(json!({
            "symbols": code::symbols_in(&source_root(opts)?, &opts.need_str("--path")?)?
        })),
        "ws-patch" => {
            let workspace = open(opts)?;
            let diff = match (opts.get("--diff-file"), opts.get("--diff")) {
                (Some(file), _) => {
                    std::fs::read_to_string(&file).with_context(|| format!("cannot read {file}"))?
                }
                (None, Some(text)) => text,
                (None, None) => bail!("give the patch with --diff-file FILE or --diff TEXT"),
            };
            print(json!(patch::apply(
                &workspace,
                &diff,
                &opts.need_str("--message")?
            )?));
        }
        "ws-revert" => {
            let workspace = open(opts)?;
            let now = patch::revert_to(&workspace, &opts.need_str("--commit")?)?;
            print(json!({ "head": now }));
        }
        "ws-build-engine" => {
            let mut workspace = open(opts)?;
            print(json!(build::build_engine(
                &mut workspace,
                flag(opts, "--network"),
                None
            )?));
        }
        "ws-build-kernel" => {
            let mut workspace = open(opts)?;
            print(json!(build::build_kernel(
                &mut workspace,
                &opts.need_str("--preset")?,
                None
            )?));
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
            let options = compare_options(opts)?;
            let report = compare::compare_with(
                &PathBuf::from(opts.need_str("--a")?),
                &PathBuf::from(opts.need_str("--b")?),
                &options,
            )?;
            print(json!({
                "bitwise_identical": report.bitwise_identical(),
                "acceptable": report.acceptable(),
                "compare": compact(&report),
            }));
        }
        "ws-parity" => {
            let mut workspace = open(opts)?;
            let options = compare_options(opts)?;
            let report = parity::parity_check(
                &mut workspace,
                &PathBuf::from(opts.need_str("--case")?),
                &opts.need_str("--preset")?,
                &options,
                None,
            )?;
            print(json!({
                "ok": report.ok,
                "rtol": report.rtol,
                "atol": report.atol,
                "first_records": report.first_records,
                "ignored": report.ignored,
                "platform_note": report.platform_note,
                "first_difference": report.first_difference,
                "preset": report.preset,
                "case": report.case,
                "report": report.report,
                "compare": report.compare.as_ref().map(compact),
                "runs": { "rust": run_summary(&report.rust), "fortran": run_summary(&report.fortran) },
            }));
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
            let report = parity::regress(
                &mut workspace,
                &PathBuf::from(opts.need_str("--case")?),
                &opts.need_str("--preset")?,
                &opts.get("--engine").unwrap_or_else(|| "rust".into()),
                &baseline,
                kind,
                None,
            )?;
            print(json!({
                "ok": report.ok,
                "verdict": report.verdict,
                "kind": report.kind,
                "engine": report.engine,
                "report": report.report,
                "closure": report.closure,
                "compare": report.compare.as_ref().map(compact),
                "runs": { "baseline": run_summary(&report.baseline), "candidate": run_summary(&report.candidate) },
            }));
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
                &out.as_ref()
                    .map_or_else(|| "stdout".into(), |p| p.display().to_string()),
            )?;
            print(
                json!({ "bytes": diff.len(), "out": out, "patch": if out.is_none() { Some(diff) } else { None } }),
            );
        }
        "ws-adopt" => {
            let mut workspace = open(opts)?;
            let preset = opts.need_str("--preset")?;
            // 只有登记过的实验内核（编译与测试在当前提交上通过、回归没有判不通过）才能采纳。
            let entry = kernels::experimental(&root(opts))?
                .into_iter()
                .find(|k| k.workspace == workspace.info.name && k.preset == preset)
                .with_context(|| {
                    format!(
                        "the {preset} kernel of workspace {} is not registered: the compile and test gates must pass on the current commit and the regression must not have failed",
                        workspace.info.name
                    )
                })?;
            workspace.record_adoption("adopt-kernel", &format!("{preset} @ {}", entry.head))?;
            print(json!({ "adopted": entry }));
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

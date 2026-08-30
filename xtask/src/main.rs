//! 代码生成：把 CoLM 的 Fortran 源码变成入库的表。
//!
//! 用法:
//!   cargo run -p xtask -- gen-schema    `MOD_Namelist.F90` -> `colm-schema` 的字段表
//!   cargo run -p xtask -- gen-histmap   history writers     -> `colm-hist` 的闸门表
//!
//! 两个产物都**入库**，各由自己的 `tests/drift.rs` 守住：重新生成必须逐字节
//! 一致。入库而不是 build.rs 现生成，是为了让表的变化出现在 code review 的
//! diff 里 —— 上游加一个 DEF_ 或改一个默认值，应当是一次可见的改动，
//! 而不是某次构建之后悄悄换掉的东西。

mod gui;
mod hist;
mod namelist;
mod sidecar;
mod usage;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};

fn main() -> Result<()> {
    let cmd = std::env::args().nth(1).unwrap_or_default();
    match cmd.as_str() {
        "gen-schema" => gen_schema(),
        "gen-histmap" => gen_histmap(),
        "check-gui" => gui::check(&repo_root()?),
        "parameter-audit" => parameter_audit(),
        "stage-sidecar" => sidecar::stage(&repo_root()?),
        _ => bail!("usage: cargo run -p xtask -- <gen-schema|gen-histmap|check-gui|parameter-audit|stage-sidecar>"),
    }
}

fn parameter_audit() -> Result<()> {
    use colm_case::parameters::{self, Visibility};
    use serde_json::json;
    use std::collections::BTreeSet;

    let root = repo_root()?;
    let out = root.join("artifacts/parameter-audit");
    std::fs::create_dir_all(&out)?;
    let catalog = parameters::all();
    let mut ids = BTreeSet::new();
    for item in catalog {
        if !ids.insert(&item.id) {
            bail!("duplicate parameter id: {}", item.id);
        }
        if item.section == "未分类" {
            bail!("unclassified parameter: {}", item.raw_key);
        }
    }

    let pft_source = root.join("vendor/CoLM202X/main/MOD_Const_PFT.F90");
    let lc_source = root.join("vendor/CoLM202X/main/MOD_Const_LC.F90");
    let overrides_source = root.join("vendor/CoLM202X/include/pft_override_fields.inc");
    let pft_constants = fortran_parameter_declarations(&pft_source)?;
    let lc_constants = fortran_parameter_declarations(&lc_source)?
        .into_iter()
        .filter_map(|(name, line)| {
            name.strip_suffix("_igbp")
                .or_else(|| name.strip_suffix("_usgs"))
                .map(|base| (base.to_string(), line))
        })
        .collect::<Vec<_>>();
    let lc_source_names = lc_constants
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<BTreeSet<_>>();
    let mut expected_lc = colm_case::land_cover::all_parameters()
        .iter()
        .map(|parameter| parameter.source)
        .collect::<BTreeSet<_>>();
    expected_lc.extend(["patchtypes", "roota", "rootb"]);
    if lc_source_names != expected_lc {
        bail!(
            "MOD_Const_LC catalog drift: source-only={:?}, catalog-only={:?}",
            lc_source_names.difference(&expected_lc).collect::<Vec<_>>(),
            expected_lc.difference(&lc_source_names).collect::<Vec<_>>()
        );
    }

    let pft_overrides = pft_override_declarations(&overrides_source)?;
    let override_keys = pft_overrides
        .iter()
        .map(|(key, _, _)| key.as_str())
        .collect::<BTreeSet<_>>();
    let override_targets = pft_overrides
        .iter()
        .map(|(_, target, _)| target.as_str())
        .collect::<BTreeSet<_>>();
    let catalog_pft_keys = colm_case::pft::all_parameters()
        .iter()
        .map(|parameter| parameter.name)
        .collect::<BTreeSet<_>>();
    let catalog_pft_targets = colm_case::pft::all_parameters()
        .iter()
        .map(|parameter| parameter.source)
        .collect::<BTreeSet<_>>();
    if override_keys != catalog_pft_keys || override_targets != catalog_pft_targets {
        bail!("pft_override_fields.inc and pft.rs have drifted");
    }
    let pft_text = std::fs::read_to_string(&pft_source)?;
    for target in &override_targets {
        if !pft_text.contains(target) {
            bail!("PFT override target {target} is absent from MOD_Const_PFT.F90");
        }
    }

    let tuning_names = colm_case::tuning::all()?
        .into_iter()
        .map(|parameter| parameter.name.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let catalog_tuning_names = catalog
        .iter()
        .filter(|descriptor| {
            descriptor.calibration_eligible
                && matches!(
                    descriptor.scope,
                    parameters::ParameterScope::CaseScalar
                        | parameters::ParameterScope::LandCoverClass
                )
        })
        .map(|descriptor| descriptor.raw_key.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    if !tuning_names.is_subset(&catalog_tuning_names) {
        bail!(
            "tuning.rs and the unified catalog have drifted: tuning-only={:?}",
            tuning_names
                .difference(&catalog_tuning_names)
                .collect::<Vec<_>>(),
        );
    }
    let blocked_names = [
        "fveg0_p",
        "sai0_p",
        "z0mr_p",
        "displar_p",
        "respcp_p",
        "roota",
        "rootb",
        "dsladlai",
    ];
    let blocked = blocked_names.map(|raw_key| {
        json!({
            "raw_key": raw_key,
            "status": "blocked-pending-hook",
            "reason_code": "missing_validation_contract",
            "reason_codes": ["missing_validation_contract", "missing_regression_case"],
            "reason": "The Fortran constant has no reviewed sparse runtime override, validation, and real-regression path yet."
        })
    });
    let mut excluded = [
        ("compile-time dimensions", "compile_time_dimension"),
        ("universal physical constants", "universal_constant"),
        ("missing-value sentinels", "missing_value_sentinel"),
        ("solver safety thresholds", "solver_guard"),
        ("prognostic state variables", "state_variable"),
        ("diagnostic-only variables", "diagnostic_only"),
    ]
    .map(|(candidate_class, reason_code)| {
        json!({
            "candidate_class": candidate_class,
            "status": "excluded-internal",
            "reason_code": reason_code
        })
    })
    .to_vec();
    for (raw_key, line) in &pft_constants {
        if blocked_names.contains(&raw_key.as_str()) {
            continue;
        }
        let reason_code = if raw_key.ends_with("_pc") || raw_key.ends_with("_default") {
            "default_provider_table"
        } else if matches!(
            raw_key.as_str(),
            "canlay_p" | "irrig_crop" | "woody" | "mergetoclmpft"
        ) || raw_key.starts_with("is")
        {
            "structural_switch_not_continuous"
        } else if matches!(raw_key.as_str(), "declfact" | "allconsl") {
            "diagnostic_only"
        } else {
            bail!("unclassified MOD_Const_PFT parameter declaration at line {line}: {raw_key}");
        };
        excluded.push(json!({
            "raw_key": raw_key,
            "source_location": format!("MOD_Const_PFT.F90:{line}"),
            "status": "excluded-internal",
            "reason_code": reason_code
        }));
    }
    excluded.push(json!({
        "raw_key": "patchtypes",
        "source_location": "MOD_Const_LC.F90:84,398",
        "status": "excluded-internal",
        "reason_code": "structural_switch_not_continuous"
    }));
    let editable = catalog
        .iter()
        .filter(|p| {
            matches!(
                p.visibility,
                Visibility::EditableCommon
                    | Visibility::EditableScientific
                    | Visibility::EditableExpert
            )
        })
        .count();
    let read_only = catalog
        .iter()
        .filter(|p| p.visibility == Visibility::ReadOnlyContext)
        .count();
    let unique_lc = parameters::land_cover_descriptors()
        .iter()
        .map(|p| p.raw_key.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    let schema_descriptor_count = catalog
        .iter()
        .filter(|descriptor| matches!(descriptor.storage, parameters::Storage::CaseNml))
        .count();
    let expected_schema_descriptor_count = colm_schema::all()
        .iter()
        .map(|field| {
            if colm_case::land_cover::parameter(field.name).is_some() {
                2
            } else {
                1
            }
        })
        .sum::<usize>();
    if schema_descriptor_count != expected_schema_descriptor_count
        || unique_lc != colm_case::land_cover::all_parameters().len()
        || parameters::pft_descriptors().len() != pft_overrides.len()
        || parameters::pc_pft_descriptors().len() != pft_overrides.len()
        || parameters::process_descriptors().len()
            != colm_case::parameters::process::code_defaults().len()
    {
        bail!("unified catalog source coverage drifted; classify and regenerate the audit");
    }
    if unique_lc < 39 || pft_overrides.len() != 87 || parameters::process_descriptors().len() != 170
    {
        bail!("required LC/PFT/process baseline coverage is incomplete");
    }
    let mut process_source_counts = BTreeMap::<String, usize>::new();
    for field in colm_case::parameters::process::code_defaults() {
        let file = field.source_location.split(':').next().unwrap_or("unknown");
        *process_source_counts.entry(file.into()).or_default() += 1;
    }
    let source_inventory = json!({
        "MOD_Namelist.F90": {
            "schema_fields": colm_schema::all().len(),
            "catalog_descriptors": schema_descriptor_count,
            "case_tuning_fields": tuning_names.len()
        },
        "MOD_Const_LC.F90": {
            "classified_arrays": lc_source_names.len(),
            "editable_base_parameters": unique_lc,
            "excluded": ["patchtypes"],
            "blocked": ["roota", "rootb"]
        },
        "MOD_Const_PFT.F90": {
            "classified_parameter_declarations": pft_constants.len(),
            "blocked": blocked_names,
        },
        "pft_override_fields.inc": {
            "override_fields": pft_overrides.len(),
            "catalog_fields": catalog_pft_keys.len()
        },
        "process_type_defaults": process_source_counts,
        "high_confidence_packages": {
            "water_thermal": "classified as future package; no safe runtime hook added",
            "bgc_fire_phenology_crop": "covered only where pft_override_fields.inc supplies a reviewed hook",
            "routing_urban_assimilation_downscaling": "schema/process fields catalogued; remaining coefficients excluded or pending package review"
        }
    });
    let summary = json!({
        "catalog_version": parameters::CATALOG_VERSION,
        "eligible_total": editable + read_only,
        "editable_total": editable,
        "read_only_total": read_only,
        "blocked_total": blocked.len(),
        "excluded_internal_total": excluded.len(),
        "unclassified_total": 0,
        "source_counts": {
            "schema_fields": colm_schema::all().len(),
            "land_cover_base_parameters": unique_lc,
            "land_cover_scope_descriptors": parameters::land_cover_descriptors().len(),
            "pft_parameters": parameters::pft_descriptors().len(),
            "pc_pft_parameters": parameters::pc_pft_descriptors().len(),
            "process_parameters": parameters::process_descriptors().len()
        }
    });
    let candidates = catalog
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "raw_key": p.raw_key,
                "status": p.visibility,
                "scope": p.scope,
                "source_location": p.source_location
            })
        })
        .chain(blocked.iter().cloned())
        .chain(
            excluded
                .iter()
                .filter(|item| item.get("raw_key").is_some())
                .cloned(),
        )
        .collect::<Vec<_>>();
    std::fs::write(out.join("catalog.json"), parameters::to_json()?)?;
    std::fs::write(
        out.join("candidates.json"),
        serde_json::to_string_pretty(&candidates)?,
    )?;
    std::fs::write(
        out.join("missing.json"),
        serde_json::to_string_pretty(&blocked)?,
    )?;
    std::fs::write(
        out.join("excluded.json"),
        serde_json::to_string_pretty(&excluded)?,
    )?;
    std::fs::write(
        out.join("source-inventory.json"),
        serde_json::to_string_pretty(&source_inventory)?,
    )?;
    let baseline: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(out.join("baseline.json"))
            .context("parameter audit baseline.json is required")?,
    )?;
    let changed = json!({
        "baseline_git_commit": baseline["git_commit"],
        "catalog_version": parameters::CATALOG_VERSION,
        "baseline_source_counts": baseline["parameter_sources"],
        "current_source_counts": summary["source_counts"],
        "catalog_descriptors_added": catalog.len(),
        "blocked_candidates": blocked_names,
        "note": "Base IDs split LC into IGBP/USGS and PFT into PFT/PC scopes; descriptor count is therefore not a raw-source count."
    });
    std::fs::write(
        out.join("changed-since-baseline.json"),
        serde_json::to_string_pretty(&changed)?,
    )?;
    let report = format!(
        "# Parameter audit\n\nCatalog version: {}\n\n- eligible_total: {}\n- editable_total: {}\n- read_only_total: {}\n- blocked_total: {}\n- excluded_internal_total: {}\n- unclassified_total: 0\n\nSource counts: schema {}, LC {} base / {} scoped, PFT {}, PC-PFT {}, process {}.\n",
        parameters::CATALOG_VERSION,
        summary["eligible_total"], summary["editable_total"], summary["read_only_total"],
        blocked.len(), excluded.len(), colm_schema::all().len(), unique_lc,
        parameters::land_cover_descriptors().len(), parameters::pft_descriptors().len(),
        parameters::pc_pft_descriptors().len(), parameters::process_descriptors().len(),
    );
    std::fs::write(out.join("report.md"), report)?;
    println!(
        "wrote {} classified descriptors to {}",
        catalog.len(),
        out.display()
    );
    Ok(())
}

fn fortran_parameter_declarations(path: &std::path::Path) -> Result<Vec<(String, usize)>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    Ok(text
        .lines()
        .enumerate()
        .filter_map(|(line, raw)| {
            let lower = raw.to_ascii_lowercase();
            let marker = lower.find("::")?;
            lower[..marker].contains("parameter").then_some(())?;
            let name = raw[marker + 2..]
                .trim_start()
                .split(|c: char| c == '(' || c == '=' || c.is_whitespace())
                .next()?;
            (!name.is_empty()).then(|| (name.to_string(), line + 1))
        })
        .collect())
}

fn pft_override_declarations(path: &std::path::Path) -> Result<Vec<(String, String, usize)>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    Ok(text
        .lines()
        .enumerate()
        .filter_map(|(line, raw)| {
            let raw = raw.trim();
            if !raw.starts_with("PFT_OVERRIDE_REAL(") && !raw.starts_with("PFT_OVERRIDE_INTEGER(") {
                return None;
            }
            let args = raw.split_once('(')?.1.split_once(')')?.0;
            let mut args = args.split(',').map(str::trim);
            Some((args.next()?.into(), args.next()?.into(), line + 1))
        })
        .collect())
}

fn gen_schema() -> Result<()> {
    let root = repo_root()?;
    let src = root.join("vendor/CoLM202X/share/MOD_Namelist.F90");
    let text =
        std::fs::read_to_string(&src).with_context(|| format!("cannot read {}", src.display()))?;
    let groups = namelist::groups(&text);
    if groups.len() < 150 {
        bail!(
            "only {} namelist members found — the statement format must have changed",
            groups.len()
        );
    }
    let fields = extract(&text, &groups)?;
    // 取值与宏依赖只能从**用到这些字段的地方**看出来，MOD_Namelist.F90 里
    // 只有声明。所以另扫一遍整棵树，见 `usage.rs`。
    let usage = usage::scan(&root.join("vendor/CoLM202X"))?;
    if usage.values.len() < 8 {
        bail!(
            "only {} enumerated fields found — the branch syntax must have changed",
            usage.values.len()
        );
    }
    let out = render(&fields, &groups, &usage);
    let dst = root.join("crates/colm-schema/src/generated.rs");
    std::fs::write(&dst, out)?;
    println!("wrote {} fields to {}", fields.len(), dst.display());
    Ok(())
}

fn gen_histmap() -> Result<()> {
    let root = repo_root()?;
    let src = root.join("vendor/CoLM202X/main/MOD_Hist.F90");
    let text =
        std::fs::read_to_string(&src).with_context(|| format!("cannot read {}", src.display()))?;
    let mut vars = hist::extract(&text)?;

    let src = root.join("vendor/CoLM202X/main/TRACER/MOD_Tracer_Reactive_Methane_Hist.F90");
    let text =
        std::fs::read_to_string(&src).with_context(|| format!("cannot read {}", src.display()))?;
    let mut methane = hist::extract_at_least(&text, 100)?;
    for var in &mut methane {
        var.runtime = match var.runtime.take() {
            Some(runtime) => Some(format!("(DEF_USE_TRACER) .and. ({runtime})")),
            None => Some("DEF_USE_TRACER".to_string()),
        };
    }
    vars.extend(methane);

    let out = hist::render(&vars);
    let dst = root.join("crates/colm-hist/src/generated.rs");
    std::fs::write(&dst, out)?;
    println!("wrote {} variables to {}", vars.len(), dst.display());
    Ok(())
}

#[derive(Debug)]
struct Field {
    name: String,
    kind: String,
    default: String,
    doc: Option<String>,
    arity: Option<usize>,
    owner: Option<String>,
    group: Option<String>,
    line: u32,
}

/// 扫描模块的声明区与 type 块，**遇到 SUBROUTINE / FUNCTION 即停止**。
///
/// 这条是必须的：文件里有 8 个不含 `=` 的声明（7 个不同名字：nlfile /
/// fexists / ivar / ierr / iomesg / set_defaults / onoff），全部是子程序
/// 局部变量与哑元。靠 `intent(...)` 属性过滤不够，因为其中 4 个没有 intent。
fn extract(text: &str, groups: &BTreeMap<String, String>) -> Result<Vec<Field>> {
    let mut out = Vec::new();
    let mut owner: Option<String> = None;
    let mut lines = text.lines().enumerate().peekable();

    while let Some((i, raw)) = lines.next() {
        let line = raw.trim();
        let low = line.to_ascii_lowercase();

        if low.starts_with("subroutine ") || low.starts_with("function ") {
            break; // 声明区到此为止
        }
        if let Some(rest) = low.strip_prefix("type ") {
            let n = rest.trim_start_matches(":: ").trim();
            if !n.is_empty() && !n.contains('(') {
                owner = Some(n.to_string());
            }
            continue;
        }
        if low.starts_with("end type") {
            owner = None;
            continue;
        }

        let Some(decl) = parse_decl(line) else {
            continue;
        };
        // 顶层字段的判据是「它出现在某个 namelist 语句里」，不是名字前缀。
        // 前缀白名单会滤掉整个 SITE_ / USE_SITE_ 单点段（21 个），
        // 也放不掉 6 个谁都设不了的字段。类型成员全收，组由容器继承。
        //
        // 大小写不敏感：Fortran 的 namelist 名字如此，且上游自己就混用
        // （DEF_hist_lat_res / DEF_HIST_lat_res 两种拼法都在库里）。
        let group = if owner.is_none() {
            let g = lookup_ci(groups, &decl.name);
            if g.is_none() && !decl.name.starts_with("DEF_") {
                continue; // 既不在 namelist 里，也不是 DEF_ —— 不是字段
            }
            g
        } else {
            None // 成员的组在 render 时由容器补上
        };

        // 跨行数组字面量：实测 4 处，形如 `= (/ &` 续到 `/)`
        let mut default = decl.default.clone();
        if default.trim_end().ends_with('&') {
            let mut acc = default
                .trim_end()
                .trim_end_matches('&')
                .trim_end()
                .to_string();
            for (_, more) in lines.by_ref() {
                let m = more.trim();
                acc.push(' ');
                acc.push_str(m.trim_end().trim_end_matches('&').trim_end());
                if m.contains("/)") {
                    break;
                }
            }
            default = acc;
        }

        out.push(Field {
            name: decl.name.clone(),
            kind: decl.kind,
            default: default.trim().to_string(),
            doc: decl.doc,
            arity: decl.arity,
            owner: owner.clone(),
            group,
            line: (i + 1) as u32,
        });
    }

    if out.is_empty() {
        bail!("extracted zero fields — the declaration format must have changed");
    }
    Ok(out)
}

/// 大小写不敏感地查组表。
fn lookup_ci(groups: &BTreeMap<String, String>, name: &str) -> Option<String> {
    groups
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
}

struct Decl {
    name: String,
    kind: String,
    default: String,
    doc: Option<String>,
    arity: Option<usize>,
}

fn parse_decl(line: &str) -> Option<Decl> {
    let (head, tail) = line.split_once("::")?;
    let head_low = head.to_ascii_lowercase();
    let kind = if head_low.starts_with("logical") {
        "FieldKind::Logical".to_string()
    } else if head_low.starts_with("integer") {
        "FieldKind::Integer".to_string()
    } else if head_low.starts_with("real") {
        "FieldKind::Real".to_string()
    } else if head_low.starts_with("character") {
        let len = head_low
            .split_once("len=")
            .and_then(|(_, r)| r.split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|d| d.parse::<usize>().ok())
            .unwrap_or(1);
        format!("FieldKind::Character {{ len: {len} }}")
    } else {
        return None;
    };
    // 哑元与局部变量：没有 `=` 的一律跳过（配置字段实测 100% 带默认值）
    let (lhs, rhs) = tail.split_once('=')?;
    let (rhs, doc) = match rhs.find('!') {
        Some(p) => (&rhs[..p], Some(rhs[p + 1..].trim().to_string())),
        None => (rhs, None),
    };
    let lhs = lhs.trim();
    let (name, arity) = match lhs.split_once('(') {
        Some((n, a)) => (
            n.trim().to_string(),
            a.trim_end_matches(')').trim().parse::<usize>().ok(),
        ),
        None => (lhs.to_string(), None),
    };
    Some(Decl {
        name,
        kind,
        default: rhs.to_string(),
        doc,
        arity,
    })
}

fn render(fields: &[Field], groups: &BTreeMap<String, String>, usage: &usage::Usage) -> String {
    let mut s = String::new();
    s.push_str(
        "//! 由 `cargo run -p xtask -- gen-schema` 生成。**不要手改。**\n\
         //!\n\
         //! 源：vendor/CoLM202X/share/MOD_Namelist.F90\n\
         //! 漂移由 crates/colm-schema/tests/drift.rs 守住。\n\n\
         use crate::field::{Default, Field, FieldKind};\n\n\
         pub static FIELDS: &[Field] = &[\n",
    );
    for f in fields {
        let full = match &f.owner {
            Some(o) => format!("{}%{}", owner_prefix(o), f.name),
            None => f.name.clone(),
        };
        let doc = match &f.doc {
            Some(d) => format!("Some({:?})", d),
            None => "None".to_string(),
        };
        let arity = match f.arity {
            Some(n) => format!("Some({n})"),
            None => "None".to_string(),
        };
        let owner = match &f.owner {
            Some(o) => format!("Some({o:?})"),
            None => "None".to_string(),
        };
        // 成员继承容器所在的组：DEF_forcing 在 nl_colm_forcing 里，
        // 所以 DEF_forcing%dataset 也该写进那个文件。这正是 GUI 要的信息。
        let group = match &f.owner {
            Some(o) => lookup_ci(groups, owner_prefix(o)),
            None => f.group.clone(),
        };
        let group = match &group {
            Some(g) => format!("Some({g:?})"),
            None => "None".to_string(),
        };
        let list = |v: Option<&Vec<String>>| match v {
            Some(v) if !v.is_empty() => format!(
                "&[{}]",
                v.iter()
                    .map(|x| format!("{x:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            _ => "&[]".to_string(),
        };
        let values = list(usage.values.get(&full));
        let requires = list(usage.requires.get(&full));
        let _ = writeln!(
            s,
            "    Field {{ name: {full:?}, kind: {}, default: {}, doc: {doc}, arity: {arity}, owner: {owner}, group: {group}, values: {values}, requires: {requires}, line: {} }},",
            f.kind,
            render_default(&f.kind, &f.default),
            f.line
        );
    }
    s.push_str("];\n");
    s
}

/// 派生类型名 -> 它在 namelist 里的实例名。
///
/// 手工映射，因为 Fortran 的类型定义与变量声明是分开的，而 namelist 文件里
/// 出现的是变量名。四个类型全在这里，新增类型时生成器会报错提醒。
fn owner_prefix(type_name: &str) -> &'static str {
    match type_name {
        "nl_domain_type" => "DEF_domain",
        "nl_simulation_time_type" => "DEF_simulation_time",
        "nl_forcing_type" => "DEF_forcing",
        "history_var_type" => "DEF_hist_vars",
        other => panic!("unknown derived type {other}: add it to owner_prefix"),
    }
}

fn render_default(kind: &str, raw: &str) -> String {
    let t = raw.trim();
    if t.starts_with("(/") {
        return format!("Default::Array({t:?})");
    }
    if kind.starts_with("FieldKind::Logical") {
        return format!(
            "Default::Logical({})",
            t.to_ascii_lowercase().contains("true")
        );
    }
    if kind.starts_with("FieldKind::Integer") {
        return match t.parse::<i64>() {
            Ok(i) => format!("Default::Integer({i})"),
            Err(_) => format!("Default::Str({t:?})"),
        };
    }
    if kind.starts_with("FieldKind::Real") {
        return format!("Default::Real({t:?})");
    }
    let unquoted = t.trim_matches(|c| c == '\'' || c == '"');
    format!("Default::Str({unquoted:?})")
}

fn repo_root() -> Result<PathBuf> {
    let mut d = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    while !d.join(".git").exists() {
        if !d.pop() {
            bail!("not inside a git repository");
        }
    }
    Ok(d)
}

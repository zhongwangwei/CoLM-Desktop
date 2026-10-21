use crate::*;
use anyhow::{bail, ensure, Context, Result};
use colm_case::parameters::{ParameterScope, Storage};
use colm_namelist::Value;
use std::collections::BTreeSet;
use std::path::{Component, Path};

pub(crate) fn identifier(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 100
            && id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'),
        "invalid model/operation identifier: {id}"
    );
    Ok(())
}
pub(crate) fn relative(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty()
            && !path.contains(['\\', ':'])
            && Path::new(path)
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
        "asset path must be relative without traversal: {path}"
    );
    ensure!(
        !matches!(
            path,
            "case.nml"
                | BINDING_FILE
                | CASE_DATA_FILE
                | RUN_MANIFEST
                | "revision.json"
                | "hybrid.toml"
        ),
        "reserved asset path: {path}"
    );
    Ok(())
}
pub(crate) fn hash(hash: &str) -> Result<()> {
    ensure!(
        hash.len() == 64
            && hash
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
        "expected lowercase SHA-256"
    );
    Ok(())
}
pub(crate) fn code(code: &CodeRef) -> Result<()> {
    hash(&code.source_sha256)?;
    if let Some(commit) = &code.commit {
        ensure!(
            matches!(commit.len(), 40 | 64) && commit.bytes().all(|b| b.is_ascii_hexdigit()),
            "code commit must be a frozen full Git object ID"
        );
    }
    match code.kind {
        CodeKind::Official => ensure!(
            code.workspace.is_none(),
            "official code cannot name a workspace"
        ),
        CodeKind::Workspace => identifier(
            code.workspace
                .as_deref()
                .context("workspace code needs an ID")?,
        )?,
    }
    Ok(())
}
pub(crate) fn applicability(value: &Applicability) -> Result<()> {
    for name in value.sites.iter().chain(&value.land_classes) {
        ensure!(
            !name.trim().is_empty() && !name.chars().any(char::is_control),
            "invalid applicability name"
        );
    }
    for period in [&value.calibration, &value.validation]
        .into_iter()
        .flatten()
    {
        validate_date(&period.start)?;
        validate_date(&period.end)?;
        ensure!(period.start <= period.end, "period end precedes start");
    }
    Ok(())
}
fn validate_date(s: &str) -> Result<()> {
    ensure!(
        s.len() == 10 && s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-',
        "period dates must be YYYY-MM-DD"
    );
    let y: u32 = s.get(..4).context("date")?.parse()?;
    let m: usize = s.get(5..7).context("date")?.parse()?;
    let d: u32 = s.get(8..).context("date")?.parse()?;
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    ensure!(
        y > 0 && (1..=12).contains(&m) && d > 0 && d <= days[m - 1],
        "invalid date {s}"
    );
    Ok(())
}
/// Historical revisions keep their stored identity, but case settings no longer override cases.
pub fn model_overlay(overlay: &Overlay) -> Overlay {
    let mut overlay = overlay.clone();
    let is_case_control = |key: &str| {
        colm_schema::find(key).is_some_and(|f| f.group.is_some()) && !is_model_field(key)
    };
    overlay.physics.retain(|key, _| !is_case_control(key));
    overlay.parameters.retain(|record| {
        record.scope_instance.kind != "case-scalar" || !is_case_control(&record.raw_key)
    });
    overlay
}

/// Only actual coefficients belong to a model; selecting existing schemes is case configuration.
pub fn is_model_field(key: &str) -> bool {
    if colm_case::pft::is_parameter(key) || colm_case::land_cover::is_parameter(key) {
        return true;
    }
    if colm_case::tuning::find(key)
        .ok()
        .flatten()
        .is_some_and(|p| p.supports_continuous_sampling())
    {
        return true;
    }
    colm_schema::find(key).is_some_and(|field| {
        field.group.is_some()
            && (field.name.starts_with("DEF_TRACER_")
                && matches!(field.kind, colm_schema::FieldKind::Real)
                || field.name == "DEF_GridRiverLake_FloodInfiltMax")
    })
}

pub(crate) fn literal(text: &str) -> Result<Value> {
    ensure!(
        !text.trim().is_empty() && !text.contains(['\n', '\r']),
        "expected a single namelist literal"
    );
    let mut quote = None;
    for ch in text.chars() {
        if matches!(ch, '\'' | '"') {
            if quote == Some(ch) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(ch);
            }
        } else if quote.is_none() {
            ensure!(
                !matches!(ch, '&' | '/' | '!' | '='),
                "unexpected syntax in literal"
            );
        }
    }
    ensure!(quote.is_none(), "unterminated string literal");
    let doc = colm_namelist::parse(&format!("&model\nvalue = {text}\n/\n"))?;
    ensure!(
        doc.paths() == ["value"],
        "literal contains another assignment"
    );
    doc.get("value").cloned().context("missing literal")
}
fn scalar_kind(value: &Value, kind: &str) -> Result<()> {
    let valid = match kind {
        "real" => value.as_f64().is_some_and(f64::is_finite),
        "integer" => matches!(value, Value::Int(_)),
        "logical" => matches!(value, Value::Bool(_)),
        "character" => matches!(value, Value::Str(_)),
        _ => false,
    };
    ensure!(valid, "invalid {kind} literal: {value}");
    Ok(())
}
pub(crate) fn schema_value(key: &str, text: &str) -> Result<Value> {
    let field = colm_schema::find(key).context("unknown model field")?;
    ensure!(
        is_model_field(key),
        "field is not a writable model control: {key}"
    );
    let value = literal(text)?;
    let values = match &value {
        Value::List(values) => {
            ensure!(
                field.arity.is_some_and(|n| values.len() <= n),
                "wrong array length for {key}"
            );
            values.as_slice()
        }
        v => std::slice::from_ref(v),
    };
    for value in values {
        let kind = match field.kind {
            colm_schema::FieldKind::Real => "real",
            colm_schema::FieldKind::Integer => "integer",
            colm_schema::FieldKind::Logical => "logical",
            colm_schema::FieldKind::Character { .. } => "character",
        };
        scalar_kind(value, kind)?;
        if let (colm_schema::FieldKind::Character { len }, Value::Str(s)) = (field.kind, value) {
            ensure!(s.len() <= len, "{key} exceeds {len} bytes");
        }
        let token = match value {
            Value::Str(s) => s.clone(),
            v => v.to_string(),
        };
        ensure!(
            field.values.is_empty() || field.values.iter().any(|v| v.eq_ignore_ascii_case(&token)),
            "unsupported {key} value: {token}"
        );
        if matches!(
            field.name,
            "DEF_TRACER_MRAT"
                | "DEF_TRACER_REF_RATIO"
                | "DEF_TRACER_INIT_DELTA"
                | "DEF_TRACER_REACTIVE_DECAY_RATE"
        ) {
            // Fortran otherwise warns and silently substitutes scientific defaults.
            if let Value::Str(csv) = value {
                if !csv.trim().is_empty() {
                    for number in csv.split(',') {
                        ensure!(
                            literal(number.trim())?.as_f64().is_some_and(f64::is_finite),
                            "{key}: tracer values must be finite numbers"
                        );
                    }
                }
            }
        }
        if let Some(number) = value.as_f64() {
            if colm_case::tuning::find(key)?.is_some() {
                colm_case::tuning::validate_value(key, number)?;
            }
            if colm_case::land_cover::is_parameter(key) {
                colm_case::land_cover::validate_override(key, number)?;
            }
        }
    }
    Ok(value)
}
pub(crate) fn parameter_value(record: &ParameterOverrideRecord) -> Result<Value> {
    let d = colm_case::parameters::all()
        .iter()
        .find(|d| d.id == record.parameter_id)
        .context("unknown parameter ID")?;
    ensure!(
        d.raw_key.eq_ignore_ascii_case(&record.raw_key),
        "parameter ID and raw_key disagree"
    );
    let s = &record.scope_instance;
    let value = literal(&record.value)?;
    match d.storage {
        Storage::CaseNml => {
            if d.scope == ParameterScope::LandCoverClass {
                ensure!(
                    s.kind == "land-cover-class"
                        && matches!(s.scheme.as_deref(), Some("IGBP" | "USGS"))
                        && s.index.is_some_and(|v| v > 0
                            && v <= if s.scheme.as_deref() == Some("USGS") {
                                24
                            } else {
                                17
                            }),
                    "invalid land-cover scope"
                );
                ensure!(
                    record
                        .parameter_id
                        .starts_with(&format!("lct:{}:", s.scheme.as_deref().unwrap_or_default())),
                    "land-cover scheme and ID disagree"
                );
            } else {
                ensure!(
                    s.kind == "case-scalar" && s.scheme.is_none() && s.index.is_none(),
                    "invalid scalar scope"
                );
            }
            ensure!(
                s.process_file.is_none(),
                "case scalar cannot reference process file"
            );
            schema_value(&record.raw_key, &record.value)?;
        }
        Storage::PftOverride | Storage::PcPftOverride => {
            let expected = if d.storage == Storage::PftOverride {
                "pft"
            } else {
                "pc-pft"
            };
            ensure!(
                s.kind == expected
                    && s.scheme.is_none()
                    && s.process_file.is_none()
                    && s.index
                        .is_some_and(|v| colm_case::pft::pft_name(v).is_some()),
                "invalid PFT scope"
            );
            colm_case::pft::validate_override(
                &record.raw_key,
                value.as_f64().context("PFT value must be numeric")?,
            )?;
        }
        Storage::ProcessParameterFile => {
            ensure!(
                s.kind == "process-file" && s.scheme.is_none() && s.index.is_none(),
                "invalid process scope"
            );
            relative(s.process_file.as_deref().context("missing process file")?)?;
            match &value {
                Value::List(values) => {
                    for v in values {
                        scalar_kind(v, &d.value_kind)?;
                    }
                }
                v => scalar_kind(v, &d.value_kind)?,
            }
        }
        Storage::ReadOnly => bail!("read-only parameter {}", d.id),
    }
    Ok(value)
}
pub(crate) fn parameter_key(r: &ParameterOverrideRecord) -> String {
    format!(
        "{}:{:?}:{:?}:{}",
        r.parameter_id,
        r.scope_instance.scheme,
        r.scope_instance.index,
        r.scope_instance.process_file.as_deref().unwrap_or("")
    )
}
pub fn validate_overlay(overlay: &Overlay) -> Result<()> {
    let mut keys = BTreeSet::new();
    let mut process_defaults = None;
    for record in &overlay.parameters {
        parameter_value(record)?;
        ensure!(
            keys.insert(parameter_key(record)),
            "duplicate parameter override"
        );
        ensure!(
            !overlay
                .physics
                .keys()
                .any(|key| key.eq_ignore_ascii_case(&record.raw_key)),
            "parameter duplicated in physics"
        );
        if let Some(file) = &record.scope_instance.process_file {
            // 不带文件的改动只能记在示踪物标准参数文件上，运行时改到算例自己的那份；限定文件名和组，
            // 免得拼错的文件名或放错组的参数静默地不起作用。
            if !overlay
                .process_files
                .iter()
                .any(|a| a.relative_path == *file)
            {
                let group = process_defaults
                    .get_or_insert_with(colm_case::parameters::process::code_defaults)
                    .iter()
                    .find(|p| p.path.eq_ignore_ascii_case(&record.raw_key))
                    .map(|p| p.group)
                    .context("unknown process parameter")?;
                ensure!(
                    colm_case::parameters::process::standard_file_accepts(
                        file,
                        &record.raw_key,
                        group
                    ),
                    "process override has no frozen process asset and is not a standard tracer parameter of {file}"
                );
            }
        }
    }
    let tuning = overlay
        .parameters
        .iter()
        .map(|r| (&r.raw_key, &r.value))
        .chain(overlay.physics.iter())
        .filter_map(|(k, v)| colm_case::tuning::find(k).ok().flatten().map(|_| (k, v)))
        .map(|(k, v)| Ok((k.clone(), literal(v)?.as_f64().context("tuning value")?)))
        .collect::<Result<Vec<_>>>()?;
    colm_case::tuning::validate_values(&tuning)?;
    let mut fields = BTreeSet::new();
    for (key, value) in &overlay.physics {
        ensure!(
            !colm_case::land_cover::is_parameter(key) && !colm_case::pft::is_parameter(key),
            "land-cover and PFT parameters require explicit scope records"
        );
        ensure!(
            fields.insert(key.to_ascii_lowercase()),
            "duplicate physics field"
        );
        schema_value(key, value)?;
    }
    let mut assets = BTreeSet::new();
    for asset in overlay
        .process_files
        .iter()
        .chain(overlay.hybrid.iter().flat_map(|h| &h.assets))
    {
        relative(&asset.relative_path)?;
        hash(&asset.sha256)?;
        ensure!(assets.insert(&asset.relative_path), "duplicate asset path");
    }
    for (key, text) in overlay.physics.iter().chain(
        overlay
            .parameters
            .iter()
            .filter(|record| record.scope_instance.kind == "case-scalar")
            .map(|record| (&record.raw_key, &record.value)),
    ) {
        if key.eq_ignore_ascii_case("DEF_TRACER_PARAM_FILES") {
            if let Value::Str(raw) = literal(text)? {
                for path in colm_namelist::tracer_files::param_file_paths(&raw)? {
                    relative(&path)?;
                    ensure!(
                        overlay
                            .process_files
                            .iter()
                            .any(|a| a.relative_path == path),
                        "process file reference lacks frozen asset: {path}"
                    );
                }
            }
        }
    }
    if let Some(hybrid) = &overlay.hybrid {
        let _: toml::Value = toml::from_str(&hybrid.config).context("invalid hybrid TOML")?;
    }
    Ok(())
}

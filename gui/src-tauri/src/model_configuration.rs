//! In-memory model configuration. Source defaults remain references, not execution facts.
use super::*;
use crate::model::{ConfigurationSource, PftSelection};
use colm_model::{Overlay, Registry};
use colm_namelist::{Document, Value as NmlValue};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub(crate) fn query(
    root: &Path,
    source: ConfigurationSource,
    case_dir: Option<&str>,
    kernel_dir: Option<&str>,
    selection: Option<PftSelection>,
    case_values: Option<BTreeMap<String, String>>,
) -> Result<Value, String> {
    // Registry::open creates these directories; a read query requires an existing registry.
    if !root.join("versions").is_dir() || !root.join("drafts").is_dir() {
        return Err("model registry is not initialized".into());
    }
    let registry = Registry::open(root).map_err(|e| e.to_string())?;
    let (identity, stored_overlay, roots) = resolve(&registry, source)?;
    let assets = read_assets(&stored_overlay, &roots)?;
    let case = case_dir.filter(|p| !p.trim().is_empty()).map(PathBuf::from);
    let mut doc = match case.as_ref() {
        Some(dir) => colm_namelist::parse(
            &std::fs::read_to_string(dir.join("case.nml")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?,
        None => colm_namelist::parse("").map_err(|e| e.to_string())?,
    };
    for (key, raw) in case_values.unwrap_or_default() {
        let field = colm_schema::find(&key).ok_or_else(|| format!("unknown case field: {key}"))?;
        if field.group.is_none() || colm_model::is_model_field(&key) {
            return Err(format!(
                "caseValues cannot override model or derived field: {key}"
            ));
        }
        let value = super::typed(&key, &raw)?;
        put(&mut doc, field.name, &value.to_string())?;
    }
    if let Some(value) = colm_model::case_process_references(&stored_overlay, &doc, case.as_deref())
        .map_err(|e| e.to_string())?
    {
        put(&mut doc, "DEF_TRACER_PARAM_FILES", &value.to_string())?;
    }
    let overlay = colm_model::model_overlay(&stored_overlay);
    for path in doc.paths() {
        if colm_model::is_model_field(&path) {
            doc.remove(&path).map_err(|e| e.to_string())?;
        }
    }
    for (key, raw) in &overlay.physics {
        put(&mut doc, key, raw)?;
    }
    for record in &overlay.parameters {
        if record.scope_instance.kind == "case-scalar" {
            put(&mut doc, &record.raw_key, &record.value)?;
        }
    }
    let kernel = kernel_dir
        .filter(|p| !p.trim().is_empty())
        .map(|p| colm_kernel::Kernel::open_manifest(Path::new(p)).map_err(|e| e.to_string()))
        .transpose()?;
    let have: BTreeSet<&str> = kernel
        .as_ref()
        .map(|k| k.manifest.macros.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let files = process_files(
        &overlay,
        &assets,
        &doc,
        &have,
        case.is_some(),
        kernel.is_some(),
    )?;
    let mut context = VisibilityContext::new_at(&doc, &have, case.as_deref());
    // Resolve CH4 from verified model assets after process overrides, not the source case file.
    context.methane_mode = files
        .iter()
        .filter(|file| file["active_methane_asset"] == true)
        .flat_map(|f| f["entries"].as_array().into_iter().flatten())
        .find(|entry| {
            entry["unset"] == false
                && entry["path"]
                    .as_str()
                    .is_some_and(|p| p.eq_ignore_ascii_case(METHANE_MODE))
        })
        .and_then(|entry| entry["value"].as_str())
        .map(|s| s.trim_matches(['\'', '"']).to_ascii_lowercase());
    let mut fields = Vec::new();
    for field in colm_schema::all() {
        let configured = doc.get(field.name).map(ToString::to_string);
        let (mut candidate, mut candidate_reason, _) = field_runtime_state(field, &context);
        if field.name.starts_with("DEF_GridRiverLake_")
            && matches!(
                doc.get("DEF_USE_GridRiverLakeFlow"),
                Some(NmlValue::Bool(false))
            )
        {
            candidate = FieldMode::Hidden;
            candidate_reason = Some("河湖流已关闭");
        }
        // Runtime rules rely on source defaults. Only explicit disabling gates and known
        // compile capabilities are conclusive here; retain all other fields for discovery.
        let known = confirmed_gate(field, &doc, &have, kernel.is_some(), candidate_reason);
        let disabled = field.group.is_none() || known && candidate != FieldMode::Editable;
        let mut state = json!({
            "name":field.name,"mode":if disabled {"disabled"} else {"editable"},
            "reason": if field.group.is_none() { Some("由内核或其他路径自动派生，只读显示") } else if known {candidate_reason} else {None},
            "allowed_values":field.values,"mixed":false,"context_default":null,"default_mixed":false,
            "scope_label":null,"built_in_default":default_literal(field.default),
            "override_value":configured,"configured_value":configured,"effective_value":null,
            "provenance":"current-catalog-reference","override_mixed":false,"effective_mixed":false,
            "status":if configured.is_some() {"configured"} else {"pending"},"pending":!known,
        });
        if !known {
            state["reason"] = json!("适用条件待按所选源码、算例和内核核对；目录默认值仅供参考");
        }
        if let Some(parent) = parent_gate(field, candidate_reason) {
            state["parent_gate"] = json!(parent);
            state["parent_gate_enabled"] = json!(if parent == "DEF_USE_CROP" {
                // CROP is a read-only compile capability, not a writable case switch.
                have.contains("CROP")
            } else {
                matches!(doc.get(parent), Some(NmlValue::Bool(true)))
            });
        }
        fields.push(state);
    }
    let pft = pft_parameters(&overlay, selection, &doc, kernel.as_ref().map(|_| &have))?;
    Ok(
        json!({"identity":identity,"fields":fields,"process_files":files,"pft_parameters":pft,
        "context_complete":false,"warnings":["当前目录用于发现配置；未读取所选冻结源码的默认值，最终适用性仍需运行前检查。"]}),
    )
}

fn resolve(
    registry: &Registry,
    source: ConfigurationSource,
) -> Result<(Value, Overlay, Vec<PathBuf>), String> {
    match source {
        ConfigurationSource::Frozen { version } => {
            let frozen = registry.get(&version).map_err(|e| e.to_string())?;
            if let Some(parent) = &frozen.parent {
                registry.get(parent).map_err(|e| e.to_string())?;
            }
            let dir = registry
                .root()
                .join("versions")
                .join(&version.id)
                .join(version.revision.to_string());
            Ok((
                json!({"kind":"frozen","id":version.id,"revision":version.revision,"source_sha256":frozen.code.source_sha256,"overlay_sha256":frozen.overlay_sha256}),
                frozen.overlay,
                vec![dir],
            ))
        }
        ConfigurationSource::Draft {
            id,
            generation,
            overlay,
        } => {
            let draft = registry.draft(&id).map_err(|e| e.to_string())?;
            if generation != draft.generation {
                return Err("model draft changed; reload before querying".into());
            }
            registry.get(&draft.parent).map_err(|e| e.to_string())?;
            colm_model::validate_overlay(&overlay).map_err(|e| e.to_string())?;
            let mut roots = vec![registry
                .root()
                .join("versions")
                .join(&draft.parent.id)
                .join(draft.parent.revision.to_string())];
            let staging = registry.root().join("draft-assets").join(&id);
            if staging.is_dir() {
                for entry in std::fs::read_dir(staging).map_err(|e| e.to_string())? {
                    let entry = entry.map_err(|e| e.to_string())?;
                    if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                        roots.push(entry.path());
                    }
                }
            }
            for prior in registry
                .list()
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter(|r| r.version.id == id)
            {
                roots.push(
                    registry
                        .root()
                        .join("versions")
                        .join(&id)
                        .join(prior.version.revision.to_string()),
                );
            }
            let hash =
                colm_kernel::sha256_hex(&serde_json::to_vec(&overlay).map_err(|e| e.to_string())?);
            Ok((
                json!({"kind":"draft","id":id,"generation":generation,"source_sha256":draft.code.source_sha256,"overlay_sha256":hash}),
                overlay,
                roots,
            ))
        }
    }
}

fn read_assets(overlay: &Overlay, roots: &[PathBuf]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut result = BTreeMap::new();
    for asset in overlay
        .process_files
        .iter()
        .chain(overlay.hybrid.iter().flat_map(|h| &h.assets))
    {
        let relative = Path::new(&asset.relative_path);
        if relative.as_os_str().is_empty()
            || !relative
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
        {
            return Err("invalid model asset path".into());
        }
        for root in roots {
            let file = root.join(relative);
            if !file.is_file() {
                continue;
            }
            if !file
                .canonicalize()
                .map_err(|e| e.to_string())?
                .starts_with(root.canonicalize().map_err(|e| e.to_string())?)
            {
                return Err("model asset escapes registry".into());
            }
            let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
            if colm_kernel::sha256_hex(&bytes) == asset.sha256 {
                result.insert(asset.relative_path.clone(), bytes);
                break;
            }
        }
        if !result.contains_key(&asset.relative_path) {
            return Err(format!(
                "model asset unavailable or changed: {}",
                asset.relative_path
            ));
        }
    }
    Ok(result)
}

fn parse_value(raw: &str) -> Result<NmlValue, String> {
    let doc =
        colm_namelist::parse(&format!("&query\nvalue = {raw}\n/\n")).map_err(|e| e.to_string())?;
    if doc.paths().len() != 1 {
        return Err("expected a single configuration value".into());
    }
    doc.get("value")
        .cloned()
        .ok_or_else(|| "invalid configuration value".into())
}

fn put(doc: &mut Document, key: &str, raw: &str) -> Result<(), String> {
    let parsed =
        colm_namelist::parse(&format!("&query\n{key} = {raw}\n/\n")).map_err(|e| e.to_string())?;
    let value = parsed
        .get(key)
        .cloned()
        .ok_or("invalid configuration value")?;
    if doc.get(key).is_some() {
        doc.set(key, value).map_err(|e| e.to_string())?;
    } else {
        // This document is never emitted; append parsed items to avoid inventing namelist groups.
        doc.items.extend(parsed.items);
    }
    Ok(())
}

fn confirmed_gate(
    field: &colm_schema::Field,
    doc: &Document,
    have: &BTreeSet<&str>,
    kernel: bool,
    reason: Option<&str>,
) -> bool {
    if field.group.is_none() {
        return true;
    }
    if kernel
        && (!field_is_relevant(field, have)
            || matches!(reason, Some("当前内核未启用 CROP")) && !have.contains("CROP")
            || matches!(
                reason,
                Some("当前内核未编入河湖汇流") | Some("当前内核未编入网格河湖汇流")
            ) && !have.contains("GridRiverLakeFlow"))
    {
        return true;
    }
    gate_switch(reason).is_some_and(|key| matches!(doc.get(key), Some(NmlValue::Bool(false))))
}

fn gate_switch(reason: Option<&str>) -> Option<&'static str> {
    match reason {
        Some("河湖流已关闭") => Some("DEF_USE_GridRiverLakeFlow"),
        Some("需要先启用 TRACER") => Some("DEF_USE_TRACER"),
        Some("需要先启用城市模型") => Some("DEF_URBAN_RUN"),
        Some("需要 BGC") | Some("需要先启用 BGC") => Some("DEF_USE_BGC"),
        Some("需要先启用土壤初始场") => Some("DEF_USE_SoilInit"),
        Some("需要先启用积雪初始场") => Some("DEF_USE_SnowInit"),
        Some("需要先启用 CN 初始场") => Some("DEF_USE_CN_INIT"),
        Some("需要先启用 LULCC") => Some("DEF_USE_LULCC"),
        Some("需要先启用植物水力过程") => Some("DEF_USE_PLANTHYDRAULICS"),
        Some("需要先读取气溶胶数据") => Some("DEF_Aerosol_Readin"),
        Some("需要先启用基岩过程") => Some("DEF_USE_BEDROCK"),
        Some("仅漫滩回馈陆面时使用") => Some("DEF_GridRiverLake_FloodFeedback"),
        _ => None,
    }
}

// Presentation dependencies stay available even when runtime rules return no disabling reason.
// An absent parent is deliberately not treated as an enabled catalog default.
fn parent_gate(field: &colm_schema::Field, reason: Option<&str>) -> Option<&'static str> {
    let name = field.name;
    let parent = match name {
        "DEF_Aerosol_Readin" | "DEF_file_snowoptics" | "DEF_file_snowaging" => {
            Some("DEF_USE_SNICAR")
        }
        "DEF_Aerosol_Clim" => Some("DEF_Aerosol_Readin"),
        "DEF_USE_OZONEDATA" | "DEF_OZONE_KO3" => Some("DEF_USE_OZONESTRESS"),
        "DEF_file_Ozone" => Some("DEF_USE_OZONEDATA"),
        "DEF_file_SoilInit" => Some("DEF_USE_SoilInit"),
        "DEF_file_SnowInit" => Some("DEF_USE_SnowInit"),
        "DEF_file_cn_init" => Some("DEF_USE_CN_INIT"),
        "DEF_file_WaterTable" => Some("DEF_USE_WaterTableInit"),
        "DEF_TRACER_SOIL_INIT_FILE" | "DEF_TRACER_SOIL_INIT_VARS" => {
            Some("DEF_TRACER_USE_SOIL_INIT")
        }
        "DEF_FERT_SOURCE" => Some("DEF_USE_FERT"),
        "DEF_IRRIGATION_ALLOCATION" => Some("DEF_USE_IRRIGATION"),
        "DEF_USE_FERT"
        | "DEF_USE_CNSOYFIXN"
        | "DEF_USE_IRRIGATION"
        | "USE_SITE_pctcrop"
        | "DEF_TUNING_CROP_PLANTING_DAY" => Some("DEF_USE_CROP"),
        "DEF_USE_CN_INIT"
        | "DEF_USE_LAIFEEDBACK"
        | "DEF_NDEP_FREQUENCY"
        | "DEF_USE_NOSTRESSNITROGEN"
        | "DEF_USE_SASU"
        | "DEF_USE_DiagMatrix"
        | "DEF_USE_PN"
        | "DEF_USE_NITRIF"
        | "DEF_USE_FIRE"
        | "DEF_TUNING_SMPMAX_HR"
        | "DEF_TUNING_SMPMIN_HR" => Some("DEF_USE_BGC"),
        "DEF_LULCC_SCHEME" => Some("DEF_USE_LULCC"),
        "DEF_LAI_START_YEAR" | "DEF_LAI_END_YEAR" => Some("DEF_LAI_CHANGE_YEARLY"),
        "USE_SITE_dbedrock" => Some("DEF_USE_BEDROCK"),
        "DEF_MEDLYN_G1" | "DEF_MEDLYN_G0" => Some("DEF_USE_MEDLYNST"),
        "DEF_WUE_LAMBDA" => Some("DEF_USE_WUEST"),
        "DEF_PC_CROP_SPLIT" => Some("DEF_USE_PC"),
        "DEF_GridRiverLake_FloodInfiltMax" => Some("DEF_GridRiverLake_FloodFeedback"),
        _ if name.starts_with("DEF_TUNING_IRRIGATION_") => Some("DEF_USE_IRRIGATION"),
        _ if name.starts_with("DEF_PH_") || colm_case::land_cover::needs_plant_hydraulics(name) => {
            Some("DEF_USE_PLANTHYDRAULICS")
        }
        _ if name.starts_with("DEF_GridRiverLake_") => Some("DEF_USE_GridRiverLakeFlow"),
        _ if field_section(name, field.group) == Some("示踪剂") => Some("DEF_USE_TRACER"),
        _ if field_section(name, field.group) == Some("城市") => Some("DEF_URBAN_RUN"),
        _ => gate_switch(reason),
    };
    parent.filter(|parent| !parent.eq_ignore_ascii_case(name))
}

fn process_files(
    overlay: &Overlay,
    assets: &BTreeMap<String, Vec<u8>>,
    doc: &Document,
    have: &BTreeSet<&str>,
    has_case: bool,
    has_kernel: bool,
) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    for asset in &overlay.process_files {
        let mut params = colm_namelist::parse(
            std::str::from_utf8(&assets[&asset.relative_path]).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let original_values: BTreeMap<String, String> = params
            .paths()
            .into_iter()
            .map(|p| (p.to_ascii_lowercase(), params.get(&p).unwrap().to_string()))
            .collect();
        for record in overlay.parameters.iter().filter(|r| {
            r.scope_instance.kind == "process-file"
                && r.scope_instance.process_file.as_deref() == Some(asset.relative_path.as_str())
        }) {
            if params.get(&record.raw_key).is_some() {
                params
                    .set(&record.raw_key, parse_value(&record.value)?)
                    .map_err(|e| e.to_string())?;
            } else {
                let defaults = process_code_defaults();
                let meta = defaults
                    .iter()
                    .find(|f| f.path.eq_ignore_ascii_case(&record.raw_key) && f.insertable)
                    .ok_or("process field cannot be inserted")?;
                params
                    .insert(&record.raw_key, parse_value(&record.value)?, meta.group)
                    .map_err(|e| e.to_string())?;
            }
        }
        let mut file = serde_json::to_value(process_entries_document(
            &params,
            asset.relative_path.clone(),
        )?)
        .map_err(|e| e.to_string())?;
        file["verified"] = json!(true);
        let explicit_mapping = ["DEF_TRACER_NAMES", "DEF_TRACER_PARAM_FILES"]
            .iter()
            .all(|key| matches!(doc.get(key), Some(NmlValue::Str(_))));
        let active = explicit_mapping.then(|| {
            tracer_param_file(doc, |name, _| {
                matches!(name.to_ascii_uppercase().as_str(), "CH4" | "METHANE")
            })
            .as_deref()
                == Some(asset.relative_path.as_str())
        });
        file["active_methane_asset"] = json!(active);
        for entry in file["entries"].as_array_mut().unwrap() {
            let path = entry["path"].as_str().unwrap().to_string();
            let descriptor = colm_case::parameters::all().iter().find(|d| {
                d.storage == colm_case::parameters::Storage::ProcessParameterFile
                    && d.raw_key.eq_ignore_ascii_case(&path)
                    && d.subgroup.as_deref().is_some_and(|g| {
                        g.eq_ignore_ascii_case(entry["group"].as_str().unwrap_or(""))
                    })
            });
            entry["asset_value"] = json!(original_values.get(&path.to_ascii_lowercase()));
            entry["editable"] = json!(descriptor.is_some());
            entry["descriptor_id"] = json!(descriptor.map(|d| &d.id));
            entry["pending"] = json!(true);
            if path.eq_ignore_ascii_case(METHANE_MODE) {
                let river = match doc.get("DEF_USE_GridRiverLakeFlow") {
                    Some(NmlValue::Bool(v)) => Some(*v),
                    _ => None,
                };
                let single = has_kernel.then(|| have.contains("SinglePoint"));
                let known = has_case
                    && has_kernel
                    && river.is_some()
                    && (single == Some(false) || doc.get("SITE_landtype").is_some());
                let options = if known {
                    let wetland = matches!(
                        (doc.get("SITE_landtype"), have.contains("LULC_USGS")),
                        (Some(NmlValue::Int(11)), false) | (Some(NmlValue::Int(17)), true)
                    );
                    methane_inundation_options(
                        single == Some(false) || wetland,
                        single == Some(false)
                            && have.contains("GridRiverLakeFlow")
                            && river == Some(true),
                    )
                } else if river == Some(false) {
                    methane_inundation_options(true, false)
                } else {
                    methane_inundation_options(true, true)
                };
                entry["options"] = json!(options);
                entry["pending"] = json!(!known);
            }
        }
        out.push(file);
    }
    Ok(out)
}

fn pft_parameters(
    overlay: &Overlay,
    selection: Option<PftSelection>,
    doc: &Document,
    have: Option<&BTreeSet<&str>>,
) -> Result<Vec<Value>, String> {
    let Some(selection) = selection else {
        return Ok(Vec::new());
    };
    if !matches!(selection.kind.as_str(), "pft" | "pc-pft") || selection.index > 78 {
        return Err("invalid PFT scope/index".into());
    }
    let mut out = Vec::new();
    for meta in colm_case::pft::all_parameters() {
        let record = overlay.parameters.iter().find(|r| {
            r.raw_key.eq_ignore_ascii_case(meta.name)
                && r.scope_instance.kind == selection.kind
                && r.scope_instance.index == Some(selection.index)
        });
        let value = record.map(|r| r.value.as_str());
        let pc = selection.kind == "pc-pft";
        let default = match doc.get("DEF_USE_Campbell_SOIL_MODEL") {
            Some(NmlValue::Bool(campbell)) => {
                colm_case::pft::default_literal(meta.name, selection.index, *campbell, pc)
                    .map_err(|e| e.to_string())?
            }
            _ => None,
        };
        let structural = if pc { "DEF_USE_PC" } else { "DEF_USE_PFT" };
        let off = |key| matches!(doc.get(key), Some(NmlValue::Bool(false)));
        let on = |key| matches!(doc.get(key), Some(NmlValue::Bool(true)));
        use colm_case::pft::Condition;
        let process_inactive = match meta.condition {
            Condition::Always => false,
            Condition::BallBerry => on("DEF_USE_MEDLYNST") || on("DEF_USE_WUEST"),
            Condition::Medlyn => off("DEF_USE_MEDLYNST") || on("DEF_USE_WUEST"),
            Condition::Wue => off("DEF_USE_WUEST") || on("DEF_USE_MEDLYNST"),
            Condition::PlantHydraulics => off("DEF_USE_PLANTHYDRAULICS"),
            Condition::Bgc => off("DEF_USE_BGC"),
            Condition::Fire => off("DEF_USE_BGC") || off("DEF_USE_FIRE"),
            Condition::Crop => {
                off("DEF_USE_BGC")
                    || selection.index < 15
                    || have.is_some_and(|macros| !macros.contains("CROP"))
            }
        };
        let unsupported_index =
            have.is_some_and(|macros| !macros.contains("CROP") && selection.index > 15);
        let number = |key| {
            doc.get(key)
                .and_then(|v| v.as_f64().or_else(|| parse_real(&v.to_string())))
        };
        let scalar_override = match meta.name {
            "DEF_PFT_GRADM" => number("DEF_BALL_BERRY_GRADM").is_some_and(|v| v > 1.6),
            "DEF_PFT_BINTER" => number("DEF_BALL_BERRY_BINTER").is_some_and(|v| v >= 0.0),
            "DEF_PFT_G1" => number("DEF_MEDLYN_G1").is_some_and(|v| v >= 0.0),
            "DEF_PFT_G0" => number("DEF_MEDLYN_G0").is_some_and(|v| v >= 0.0),
            "DEF_PFT_LAMBDA" => number("DEF_WUE_LAMBDA").is_some_and(|v| v > 0.0),
            _ => false,
        };
        let parameter_inactive = match meta.name {
            "DEF_PFT_LIVEWDCN" | "DEF_PFT_DEADWDCN" | "DEF_PFT_CROOT_STEM" | "DEF_PFT_FLIVEWD" => {
                !(1..=11).contains(&selection.index) && selection.index < 15
            }
            "DEF_PFT_STEM_LEAF" => !(1..=11).contains(&selection.index),
            "DEF_PFT_MANURE" => {
                off("DEF_USE_FERT")
                    || matches!(doc.get("DEF_FERT_SOURCE"), Some(NmlValue::Int(value)) if *value != 1)
            }
            _ => false,
        };
        let inactive = unsupported_index
            || selection.index == 0
            || off(structural)
            || process_inactive
            || scalar_override
            || parameter_inactive;
        out.push(json!({"name":meta.name,"parameter_id":format!("{}:{}",selection.kind,meta.name),"label_zh":meta.label_zh,"label_en":meta.label_en,"group_zh":meta.group_zh,"group_en":meta.group_en,"unit":meta.unit,"kind":match meta.kind {colm_case::pft::Kind::Real=>"real",colm_case::pft::Kind::Integer=>"integer"},"default":default,"value":value,"configured_value":value,"effective_value":null,"scope_kind":selection.kind,"scope_label":format!("{}-{}",selection.kind,selection.index),"allowed_values":if meta.name=="DEF_PFT_C3C4" {vec!["0","1"]}else{vec![]},"mode":if inactive {"disabled"}else{"editable"},"reason":if unsupported_index {"当前候选内核未编入 CROP，只支持 PFT 0..15"}else if scalar_override {"已显式设置算例级系数，覆盖此 PFT 系数"}else if inactive {"当前配置未启用所需次网格或过程，或所选类型不适用"}else{"适用过程与类型表默认值须按所选源码核对"},"pending":true,"provenance":"current-catalog-reference"}));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use colm_model::{
        Applicability, Asset, CodeKind, CodeRef, ModelDraft, ParameterOverrideRecord,
        ParameterScopeInstance,
    };

    fn fixture() -> (PathBuf, Registry, ModelDraft) {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "colm-configuration-{}-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let registry = Registry::open(&root).unwrap();
        let code = CodeRef {
            kind: CodeKind::Official,
            workspace: None,
            commit: None,
            source_sha256: "a".repeat(64),
        };
        let baseline = registry.install_official(code.clone()).unwrap();
        let draft = registry
            .create_draft(ModelDraft {
                id: "query-test".into(),
                name: "Query".into(),
                note: "read-only query test".into(),
                parent: baseline.version,
                code,
                overlay: Overlay::default(),
                applicability: Applicability::default(),
                generation: 0,
                derived_from: None,
                created_unix: 0,
            })
            .unwrap();
        (root, registry, draft)
    }
    fn source(draft: &ModelDraft, overlay: Overlay) -> ConfigurationSource {
        ConfigurationSource::Draft {
            id: draft.id.clone(),
            generation: draft.generation,
            overlay,
        }
    }
    fn field<'a>(result: &'a Value, name: &str) -> &'a Value {
        result["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == name)
            .unwrap()
    }

    #[test]
    fn parent_visibility_requires_explicit_selection_and_survives_enabled_state() {
        let (root, _, draft) = fixture();
        let pairs = [
            ("DEF_Aerosol_Readin", "DEF_USE_SNICAR"),
            ("DEF_file_snowoptics", "DEF_USE_SNICAR"),
            ("DEF_file_snowaging", "DEF_USE_SNICAR"),
            ("DEF_Aerosol_Clim", "DEF_Aerosol_Readin"),
            ("DEF_USE_OZONEDATA", "DEF_USE_OZONESTRESS"),
            ("DEF_file_Ozone", "DEF_USE_OZONEDATA"),
            ("DEF_TRACER_SOIL_INIT_FILE", "DEF_TRACER_USE_SOIL_INIT"),
            ("DEF_file_cn_init", "DEF_USE_CN_INIT"),
            ("DEF_USE_FIRE", "DEF_USE_BGC"),
            ("DEF_FERT_SOURCE", "DEF_USE_FERT"),
            ("DEF_IRRIGATION_ALLOCATION", "DEF_USE_IRRIGATION"),
            ("DEF_MEDLYN_G1", "DEF_USE_MEDLYNST"),
        ];
        let case = root.join("case");
        std::fs::create_dir(&case).unwrap();
        for selected in [None, Some(".false."), Some(".true.")] {
            let entries = selected
                .map(|raw| {
                    pairs
                        .iter()
                        .map(|(_, parent)| format!("{parent}={raw}\n"))
                        .collect::<String>()
                })
                .unwrap_or_default();
            std::fs::write(case.join("case.nml"), format!("&nl_colm\n{entries}/\n")).unwrap();
            let result = query(
                &root,
                source(&draft, Overlay::default()),
                case.to_str(),
                None,
                None,
                None,
            )
            .unwrap();
            for (child, parent) in pairs {
                assert_eq!(field(&result, child)["parent_gate"], parent, "{child}");
                assert_eq!(
                    field(&result, child)["parent_gate_enabled"],
                    selected == Some(".true."),
                    "{child}"
                );
            }
            assert!(field(&result, "DEF_USE_SNICAR")
                .get("parent_gate")
                .is_none());
            assert!(field(&result, "DEF_USE_SNICAR")["pending"]
                .as_bool()
                .unwrap());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn crop_parent_visibility_uses_selected_kernel_capability() {
        let (root, _, draft) = fixture();
        let kernel = root.join("kernel");
        std::fs::create_dir(&kernel).unwrap();
        for crop in [false, true] {
            let mut macros = vec!["LULC_IGBP"];
            if crop {
                macros.push("CROP");
            }
            let manifest = json!({"schema":1,"preset":"crop-test","platform":std::env::consts::OS,
                "colm_git_sha":"test","generator_args":"","build_profile":"debug",
                "macros":macros,"built_with":"test","netcdf_c":"test","netcdf_fortran":"test",
                "hdf5":"","sha256":{}});
            std::fs::write(
                kernel.join("manifest.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            let result = query(
                &root,
                source(&draft, Overlay::default()),
                None,
                kernel.to_str(),
                None,
                None,
            )
            .unwrap();
            assert!(field(&result, "DEF_USE_CROP")["configured_value"].is_null());
            for child in [
                "DEF_USE_FERT",
                "DEF_USE_CNSOYFIXN",
                "DEF_USE_IRRIGATION",
                "USE_SITE_pctcrop",
                "DEF_TUNING_CROP_PLANTING_DAY",
            ] {
                assert_eq!(field(&result, child)["parent_gate"], "DEF_USE_CROP");
                assert_eq!(
                    field(&result, child)["parent_gate_enabled"],
                    crop,
                    "{child}"
                );
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unknown_parents_remain_discoverable_and_explicit_tracer_bgc_gates_disable() {
        let (root, _, draft) = fixture();
        let result = query(
            &root,
            source(&draft, Overlay::default()),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            result["fields"].as_array().unwrap().len(),
            colm_schema::all().len()
        );
        assert_eq!(field(&result, "DEF_TRACER_TYPES")["mode"], "editable");
        assert_eq!(field(&result, "DEF_TRACER_TYPES")["pending"], true);
        assert!(field(&result, "DEF_USE_TRACER")["effective_value"].is_null());
        let values = BTreeMap::from([
            ("DEF_USE_TRACER".into(), ".false.".into()),
            ("DEF_USE_BGC".into(), ".false.".into()),
        ]);
        let result = query(
            &root,
            source(&draft, Overlay::default()),
            None,
            None,
            None,
            Some(values),
        )
        .unwrap();
        assert_eq!(field(&result, "DEF_TRACER_TYPES")["mode"], "disabled");
        assert_eq!(field(&result, "DEF_USE_FIRE")["mode"], "disabled");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn source_case_processes_are_inherited_and_case_values_are_typed() {
        let (root, _, draft) = fixture();
        let case = root.join("source");
        std::fs::create_dir(&case).unwrap();
        let original="&nl_colm\nDEF_USE_BGC = .true.\nDEF_BALL_BERRY_GRADM=12.\nDEF_USE_CN_INIT = .false.\nDEF_file_cn_init = 'old.nc'\n/\n";
        std::fs::write(case.join("case.nml"), original).unwrap();
        let values = BTreeMap::from([
            ("DEF_USE_CN_INIT".into(), ".true.".into()),
            ("DEF_file_cn_init".into(), "new path.nc".into()),
        ]);
        let result = query(
            &root,
            source(&draft, Overlay::default()),
            case.to_str(),
            None,
            None,
            Some(values),
        )
        .unwrap();
        assert_eq!(field(&result, "DEF_USE_BGC")["configured_value"], ".true.");
        assert!(field(&result, "DEF_BALL_BERRY_GRADM")["configured_value"].is_null());
        assert_eq!(
            field(&result, "DEF_USE_CN_INIT")["configured_value"],
            ".true."
        );
        assert_eq!(
            field(&result, "DEF_file_cn_init")["configured_value"],
            "'new path.nc'"
        );
        assert_eq!(
            std::fs::read_to_string(case.join("case.nml")).unwrap(),
            original
        );
        let invalid = BTreeMap::from([("DEF_BALL_BERRY_GRADM".into(), "12.".into())]);
        assert!(query(
            &root,
            source(&draft, Overlay::default()),
            case.to_str(),
            None,
            None,
            Some(invalid)
        )
        .unwrap_err()
        .contains("cannot override"));
        assert!(!root.join("runs").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn vegetation_scheme_comes_from_case_and_unsaved_case_values() {
        let (root, _, draft) = fixture();
        let case = root.join("case");
        std::fs::create_dir(&case).unwrap();
        let original = "&nl_colm\nDEF_USE_LCT=.false.\nDEF_USE_PFT=.true.\nDEF_USE_PC=.false.\nDEF_SUBGRID_SCHEME='PFT'\n/\n";
        std::fs::write(case.join("case.nml"), original).unwrap();
        let values = || {
            BTreeMap::from([
                ("DEF_USE_LCT".into(), ".false.".into()),
                ("DEF_USE_PFT".into(), ".false.".into()),
                ("DEF_USE_PC".into(), ".true.".into()),
                ("DEF_SUBGRID_SCHEME".into(), "PC".into()),
            ])
        };
        for (kind, changed, expected_mode) in [
            ("pft", false, "editable"),
            ("pc-pft", false, "disabled"),
            ("pft", true, "disabled"),
            ("pc-pft", true, "editable"),
        ] {
            let result = query(
                &root,
                source(&draft, Overlay::default()),
                case.to_str(),
                None,
                Some(PftSelection {
                    kind: kind.into(),
                    index: 1,
                }),
                changed.then(values),
            )
            .unwrap();
            assert_eq!(
                field(&result, "DEF_USE_PFT")["configured_value"],
                if changed { ".false." } else { ".true." }
            );
            assert_eq!(
                field(&result, "DEF_SUBGRID_SCHEME")["configured_value"],
                if changed { "'PC'" } else { "'PFT'" }
            );
            let parameter = result["pft_parameters"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["name"] == "DEF_PFT_VMAX25")
                .unwrap();
            assert_eq!(parameter["mode"], expected_mode);
        }
        assert_eq!(
            std::fs::read_to_string(case.join("case.nml")).unwrap(),
            original
        );
        assert!(draft.overlay.physics.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn case_scheme_save_preserves_classification_when_switching_to_pft() {
        let (root, _, _) = fixture();
        let case = root.join("case");
        std::fs::create_dir(&case).unwrap();
        std::fs::write(case.join("case.nml"), "&nl_colm\nDEF_CASE_NAME='case'\n/\n").unwrap();
        crate::project::record_case(
            case.to_str().unwrap(),
            &crate::project::CaseRecord {
                mode: Some("urban-igbp".into()),
                domain: Some("watershed".into()),
                shapefile: Some("watershed.shp".into()),
            },
        );
        for scheme in ["IGBP", "USGS", "PFT", "PC"] {
            let fields = [
                ("DEF_SUBGRID_SCHEME", scheme.to_owned()),
                (
                    "DEF_USE_LCT",
                    format!(".{}.", matches!(scheme, "IGBP" | "USGS")),
                ),
                ("DEF_USE_PFT", format!(".{}.", scheme == "PFT")),
                ("DEF_USE_PC", format!(".{}.", scheme == "PC")),
            ]
            .into_iter()
            .map(|(path, value)| FieldChange {
                path: path.into(),
                value,
            })
            .collect();
            super::super::set_fields_with_context(
                vec![case.to_string_lossy().into_owned()],
                fields,
                None,
                None,
            )
            .unwrap();
            let opened = crate::project::open_case(case.to_string_lossy().into_owned()).unwrap();
            assert_eq!(opened.profile.subgrid, scheme);
            assert_eq!(
                opened.profile.classification,
                if scheme == "IGBP" { "IGBP" } else { "USGS" }
            );
            assert_eq!(opened.profile.domain_kind.as_deref(), Some("watershed"));
            assert_eq!(opened.profile.shapefile.as_deref(), Some("watershed.shp"));
        }
        let metadata: Value =
            serde_json::from_slice(&std::fs::read(case.join(".colm-desktop.json")).unwrap())
                .unwrap();
        assert_eq!(metadata["mode"], "urban-usgs");
        let before = std::fs::read(case.join("case.nml")).unwrap();
        std::fs::write(case.join(".colm-desktop.json"), "invalid").unwrap();
        assert!(super::super::set_fields_with_context(
            vec![case.to_string_lossy().into_owned()],
            vec![FieldChange {
                path: "DEF_SUBGRID_SCHEME".into(),
                value: "IGBP".into()
            }],
            None,
            None,
        )
        .is_err());
        assert_eq!(std::fs::read(case.join("case.nml")).unwrap(), before);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn case_scheme_edits_validate_parameter_scopes_before_saving_and_repair_mismatches() {
        for (kind, matching, incompatible) in [("pft", "PFT", "PC"), ("pc-pft", "PC", "PFT")] {
            let (root, registry, mut draft) = fixture();
            let case = root.join("case");
            std::fs::create_dir(&case).unwrap();
            let fields = |scheme: &str| {
                vec![
                    ("DEF_USE_LCT".into(), NmlValue::Bool(false)),
                    ("DEF_USE_PFT".into(), NmlValue::Bool(scheme == "PFT")),
                    ("DEF_USE_PC".into(), NmlValue::Bool(scheme == "PC")),
                    ("DEF_SUBGRID_SCHEME".into(), NmlValue::Str(scheme.into())),
                ]
            };
            let case_text = |scheme: &str| {
                format!(
                "&nl_colm\nDEF_CASE_NAME='case'\nDEF_USE_LCT=.false.\nDEF_USE_PFT=.{}.\nDEF_USE_PC=.{}.\nDEF_SUBGRID_SCHEME='{scheme}'\n/\n",
                scheme == "PFT", scheme == "PC",
            )
            };
            std::fs::write(case.join("case.nml"), case_text(matching)).unwrap();
            std::fs::write(case.join(".colm-desktop.json"), r#"{"mode":"usgs"}"#).unwrap();
            draft.overlay.parameters.push(ParameterOverrideRecord {
                parameter_id: format!("{kind}:DEF_PFT_VMAX25"),
                raw_key: "DEF_PFT_VMAX25".into(),
                scope_instance: ParameterScopeInstance {
                    kind: kind.into(),
                    scheme: None,
                    index: Some(1),
                    type_name: None,
                    process_file: None,
                },
                value: "40.".into(),
                expected_old_value: None,
                unit: None,
            });
            let generation = draft.generation;
            let draft = registry.save_draft(draft, generation).unwrap();
            let frozen = registry.freeze(&draft.id, &case).unwrap();
            let before = std::fs::read(case.join("case.nml")).unwrap();
            let metadata = std::fs::read(case.join(".colm-desktop.json")).unwrap();
            let rejected = root.join("rejected");
            assert!(registry
                .materialize_run_with_case_fields(
                    &frozen.version,
                    &case,
                    &rejected,
                    "reject-scheme",
                    &fields(incompatible),
                )
                .unwrap_err()
                .to_string()
                .contains("requires runtime"));
            assert!(!rejected.exists());
            assert_eq!(std::fs::read(case.join("case.nml")).unwrap(), before);
            assert_eq!(
                std::fs::read(case.join(".colm-desktop.json")).unwrap(),
                metadata
            );

            std::fs::write(case.join("case.nml"), case_text(incompatible)).unwrap();
            assert!(registry
                .materialize_run(
                    &frozen.version,
                    &case,
                    &root.join("old-context"),
                    "old-context"
                )
                .is_err());
            let preview = registry
                .materialize_run_with_case_fields(
                    &frozen.version,
                    &case,
                    &root.join("repaired-preview"),
                    "repair-scheme",
                    &fields(matching),
                )
                .unwrap();
            let edits = fields(matching)
                .into_iter()
                .map(|(path, value)| FieldChange {
                    path,
                    value: match value {
                        NmlValue::Str(value) => value,
                        _ => value.to_string(),
                    },
                })
                .collect();
            super::super::set_fields_with_context(
                vec![case.to_string_lossy().into_owned()],
                edits,
                None,
                Some(&[preview.run_case.to_string_lossy().into_owned()]),
            )
            .unwrap();
            registry
                .materialize_run(
                    &frozen.version,
                    &case,
                    &root.join("saved-context"),
                    "saved-context",
                )
                .unwrap();
            assert_eq!(
                std::fs::read(case.join(".colm-desktop.json")).unwrap(),
                metadata
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn case_process_patch_and_clear_are_atomic_and_validate_candidate_contract() {
        let (root, registry, draft) = fixture();
        let case = root.join("case");
        std::fs::create_dir(&case).unwrap();
        let original = "&nl_colm\nDEF_CASE_NAME='case'\nDEF_USE_LCT=.true.\nDEF_USE_PFT=.false.\nDEF_USE_PC=.false.\nDEF_USE_BGC=.false.\nDEF_USE_TRACER=.false.\nDEF_TRACER_PARAM_FILES='old.nml'\nDEF_TUNING_CROP_PLANTING_DAY=120\n/\n";
        std::fs::write(case.join("case.nml"), original).unwrap();
        let frozen = registry.freeze(&draft.id, &case).unwrap();
        let remove = vec![
            "DEF_TUNING_CROP_PLANTING_DAY".into(),
            "DEF_TRACER_PARAM_FILES".into(),
        ];
        let rejected = vec![("DEF_USE_BGC".into(), NmlValue::Bool(true))];
        let preview = registry
            .materialize_run_with_case_changes(
                &frozen.version,
                &case,
                &root.join("invalid"),
                "invalid-process",
                &rejected,
                &remove,
            )
            .unwrap();
        let error = super::super::set_fields_with_changes(
            vec![case.to_string_lossy().into_owned()],
            vec![FieldChange {
                path: "DEF_USE_BGC".into(),
                value: ".true.".into(),
            }],
            &remove,
            None,
            Some(&[preview.run_case.to_string_lossy().into_owned()]),
        )
        .unwrap_err();
        assert!(error.contains("DEF_USE_BGC"));
        assert_eq!(
            std::fs::read_to_string(case.join("case.nml")).unwrap(),
            original
        );
        let candidate = vec![("DEF_USE_BGC".into(), NmlValue::Bool(false))];
        let preview = registry
            .materialize_run_with_case_changes(
                &frozen.version,
                &case,
                &root.join("valid"),
                "valid-process",
                &candidate,
                &remove,
            )
            .unwrap();
        let preview_doc = colm_namelist::parse(
            &std::fs::read_to_string(preview.run_case.join("case.nml")).unwrap(),
        )
        .unwrap();
        assert!(preview_doc.get(&remove[0]).is_none());
        super::super::set_fields_with_changes(
            vec![case.to_string_lossy().into_owned()],
            vec![FieldChange {
                path: "DEF_USE_BGC".into(),
                value: ".false.".into(),
            }],
            &remove,
            None,
            Some(&[preview.run_case.to_string_lossy().into_owned()]),
        )
        .unwrap();
        let saved =
            colm_namelist::parse(&std::fs::read_to_string(case.join("case.nml")).unwrap()).unwrap();
        assert!(saved.get(&remove[0]).is_none());
        assert_eq!(
            saved.get("DEF_TRACER_PARAM_FILES"),
            Some(&NmlValue::Str(String::new()))
        );
        assert_eq!(saved.get("DEF_USE_BGC"), Some(&NmlValue::Bool(false)));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configuration_remaps_verified_process_assets_and_keeps_new_case_references() {
        let (root, registry, mut draft) = fixture();
        let case = root.join("source");
        let stage = root.join("stage");
        std::fs::create_dir_all(case.join("process")).unwrap();
        std::fs::create_dir_all(stage.join("process")).unwrap();
        let bytes = b"&nl_colm_methane_parameter\nDEF_METHANE%inundation_mode='wetwat'\n/\n";
        std::fs::write(case.join("process/source.nml"), bytes).unwrap();
        std::fs::write(stage.join("process/frozen.nml"), bytes).unwrap();
        let original =
            "&nl_colm\nDEF_TRACER_NAMES='CH4'\nDEF_TRACER_PARAM_FILES='process/source.nml'\n/\n";
        std::fs::write(case.join("case.nml"), original).unwrap();
        draft.overlay.process_files.push(Asset {
            relative_path: "process/frozen.nml".into(),
            sha256: colm_kernel::sha256_hex(bytes),
        });
        let generation = draft.generation;
        let draft = registry.save_draft(draft, generation).unwrap();
        let frozen = registry.freeze(&draft.id, &stage).unwrap();
        let source = || ConfigurationSource::Frozen {
            version: frozen.version.clone(),
        };
        let result = query(&root, source(), case.to_str(), None, None, None).unwrap();
        assert_eq!(
            field(&result, "DEF_TRACER_PARAM_FILES")["configured_value"],
            "'process/frozen.nml'"
        );
        assert_eq!(result["process_files"][0]["active_methane_asset"], true);
        let edited = BTreeMap::from([("DEF_TRACER_PARAM_FILES".into(), "custom.nml".into())]);
        let result = query(&root, source(), case.to_str(), None, None, Some(edited)).unwrap();
        assert_eq!(
            field(&result, "DEF_TRACER_PARAM_FILES")["configured_value"],
            "'custom.nml'"
        );
        assert_eq!(result["process_files"][0]["active_methane_asset"], false);
        assert_eq!(
            std::fs::read_to_string(case.join("case.nml")).unwrap(),
            original
        );
        let frozen_asset = root
            .join("versions")
            .join(&frozen.version.id)
            .join(frozen.version.revision.to_string())
            .join("process/frozen.nml");
        assert_eq!(std::fs::read(frozen_asset).unwrap(), bytes);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_draft_and_changed_assets_are_rejected() {
        let (root, registry, draft) = fixture();
        let stale = ConfigurationSource::Draft {
            id: draft.id.clone(),
            generation: draft.generation + 1,
            overlay: Overlay::default(),
        };
        assert!(query(&root, stale, None, None, None, None)
            .unwrap_err()
            .contains("changed"));
        let assets = registry
            .root()
            .join("draft-assets")
            .join(&draft.id)
            .join("import");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::write(
            assets.join("x.nml"),
            "&nl_colm_tracer_parameter\nDEF_TRACER%init_conc=1.\n/\n",
        )
        .unwrap();
        let mut overlay = Overlay::default();
        overlay.process_files.push(Asset {
            relative_path: "x.nml".into(),
            sha256: "b".repeat(64),
        });
        assert!(
            query(&root, source(&draft, overlay), None, None, None, None)
                .unwrap_err()
                .contains("unavailable or changed")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn frozen_asset_tampering_fails_and_query_preserves_registry_bytes() {
        let (root, registry, mut draft) = fixture();
        let stage = root.join("stage");
        std::fs::create_dir(&stage).unwrap();
        let bytes = b"&nl_colm_tracer_parameter\nDEF_TRACER%init_conc=1.\n/\n";
        std::fs::write(stage.join("tracer.nml"), bytes).unwrap();
        draft.overlay.process_files.push(Asset {
            relative_path: "tracer.nml".into(),
            sha256: colm_kernel::sha256_hex(bytes),
        });
        let generation = draft.generation;
        let draft = registry.save_draft(draft, generation).unwrap();
        let frozen = registry.freeze(&draft.id, &stage).unwrap();
        let revision_dir = root
            .join("versions")
            .join(&frozen.version.id)
            .join(frozen.version.revision.to_string());
        let before = std::fs::read(revision_dir.join("revision.json")).unwrap();
        let result = query(
            &root,
            ConfigurationSource::Frozen {
                version: frozen.version.clone(),
            },
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(result["identity"]["overlay_sha256"], frozen.overlay_sha256);
        assert_eq!(
            std::fs::read(revision_dir.join("revision.json")).unwrap(),
            before
        );
        assert_eq!(
            std::fs::read(revision_dir.join("tracer.nml")).unwrap(),
            bytes
        );
        std::fs::write(revision_dir.join("tracer.nml"), b"changed").unwrap();
        assert!(query(
            &root,
            ConfigurationSource::Frozen {
                version: frozen.version
            },
            None,
            None,
            None,
            None
        )
        .unwrap_err()
        .contains("hash mismatch"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pft_selection_keeps_inactive_bgc_parameters_and_only_selected_override() {
        let mut overlay = Overlay::default();
        overlay.parameters.push(ParameterOverrideRecord {
            parameter_id: "pft:DEF_PFT_G1".into(),
            raw_key: "DEF_PFT_G1".into(),
            scope_instance: ParameterScopeInstance {
                kind: "pft".into(),
                scheme: None,
                index: Some(2),
                type_name: None,
                process_file: None,
            },
            value: "42.".into(),
            expected_old_value: None,
            unit: None,
        });
        let doc =
            colm_namelist::parse("&nl_colm\nDEF_USE_PFT=.true.\nDEF_USE_BGC=.false.\n/\n").unwrap();
        let states = pft_parameters(
            &overlay,
            Some(PftSelection {
                kind: "pft".into(),
                index: 1,
            }),
            &doc,
            None,
        )
        .unwrap();
        assert_eq!(states.len(), colm_case::pft::all_parameters().len());
        assert!(states.iter().all(|f| f["configured_value"].is_null()));
        let bgc_name = colm_case::pft::all_parameters()
            .iter()
            .find(|f| f.condition == colm_case::pft::Condition::Bgc)
            .unwrap()
            .name;
        assert_eq!(
            states.iter().find(|f| f["name"] == bgc_name).unwrap()["mode"],
            "disabled"
        );
        let states = pft_parameters(
            &overlay,
            Some(PftSelection {
                kind: "pft".into(),
                index: 2,
            }),
            &doc,
            None,
        )
        .unwrap();
        assert_eq!(
            states.iter().find(|f| f["name"] == "DEF_PFT_G1").unwrap()["configured_value"],
            "42."
        );
    }

    #[test]
    fn pft_crop_indices_respect_known_kernel_capability() {
        let doc = colm_namelist::parse("&nl_colm\nDEF_USE_PFT=.true.\n/\n").unwrap();
        let overlay = Overlay::default();
        let macros = BTreeSet::from(["LULC_IGBP"]);
        let selection = || {
            Some(PftSelection {
                kind: "pft".into(),
                index: 16,
            })
        };
        let states = pft_parameters(&overlay, selection(), &doc, Some(&macros)).unwrap();
        assert!(states
            .iter()
            .all(|s| s["mode"] == "disabled" && s["reason"].as_str().unwrap().contains("CROP")));
        let unknown = pft_parameters(&overlay, selection(), &doc, None).unwrap();
        assert!(unknown
            .iter()
            .any(|s| s["mode"] == "editable" && s["pending"] == true));
        let crop = BTreeSet::from(["LULC_IGBP", "CROP"]);
        let supported = pft_parameters(&overlay, selection(), &doc, Some(&crop)).unwrap();
        assert!(supported.iter().any(|s| s["mode"] == "editable"));
    }

    #[test]
    fn pft_known_crop_wood_and_fertilizer_conditions_do_not_use_missing_defaults() {
        let mut doc = colm_namelist::parse("").unwrap();
        let overlay = Overlay::default();
        let state = |doc: &Document, index, macros: Option<&BTreeSet<&str>>| {
            pft_parameters(
                &overlay,
                Some(PftSelection {
                    kind: "pft".into(),
                    index,
                }),
                doc,
                macros,
            )
            .unwrap()
        };
        let mode = |states: &Vec<Value>, name: &str| {
            states.iter().find(|s| s["name"] == name).unwrap()["mode"].clone()
        };
        let crop_name = colm_case::pft::all_parameters()
            .iter()
            .find(|p| p.condition == colm_case::pft::Condition::Crop)
            .unwrap()
            .name;
        assert_eq!(mode(&state(&doc, 15, None), crop_name), "editable");
        assert_eq!(
            mode(
                &state(&doc, 15, Some(&BTreeSet::from(["LULC_IGBP"]))),
                crop_name
            ),
            "disabled"
        );
        for name in [
            "DEF_PFT_LIVEWDCN",
            "DEF_PFT_DEADWDCN",
            "DEF_PFT_CROOT_STEM",
            "DEF_PFT_FLIVEWD",
            "DEF_PFT_STEM_LEAF",
        ] {
            assert_eq!(mode(&state(&doc, 13, None), name), "disabled");
            assert_eq!(mode(&state(&doc, 1, None), name), "editable");
        }
        assert_eq!(
            mode(&state(&doc, 15, None), "DEF_PFT_STEM_LEAF"),
            "disabled"
        );
        assert_eq!(mode(&state(&doc, 15, None), "DEF_PFT_MANURE"), "editable");
        put(&mut doc, "DEF_USE_FERT", ".false.").unwrap();
        assert_eq!(mode(&state(&doc, 15, None), "DEF_PFT_MANURE"), "disabled");
        put(&mut doc, "DEF_USE_FERT", ".true.").unwrap();
        put(&mut doc, "DEF_FERT_SOURCE", "2").unwrap();
        assert_eq!(mode(&state(&doc, 15, None), "DEF_PFT_MANURE"), "disabled");
        put(&mut doc, "DEF_FERT_SOURCE", "1").unwrap();
        assert_eq!(mode(&state(&doc, 15, None), "DEF_PFT_MANURE"), "editable");
    }

    #[test]
    fn pft_explicit_case_wide_coefficients_disable_only_overridden_parameters() {
        let overlay = Overlay::default();
        for (pft, scalar, override_value, sentinel) in [
            ("DEF_PFT_GRADM", "DEF_BALL_BERRY_GRADM", "1.7", "1.6"),
            ("DEF_PFT_BINTER", "DEF_BALL_BERRY_BINTER", "0.", "-1."),
            ("DEF_PFT_G1", "DEF_MEDLYN_G1", "0.", "-1."),
            ("DEF_PFT_G0", "DEF_MEDLYN_G0", "0.", "-1."),
            ("DEF_PFT_LAMBDA", "DEF_WUE_LAMBDA", "1.", "0."),
        ] {
            let mut doc = colm_namelist::parse("").unwrap();
            let selected = |doc: &Document| {
                pft_parameters(
                    &overlay,
                    Some(PftSelection {
                        kind: "pft".into(),
                        index: 1,
                    }),
                    doc,
                    None,
                )
                .unwrap()
                .into_iter()
                .find(|s| s["name"] == pft)
                .unwrap()
            };
            assert_eq!(selected(&doc)["mode"], "editable");
            assert_eq!(selected(&doc)["pending"], true);
            put(&mut doc, scalar, override_value).unwrap();
            assert_eq!(selected(&doc)["mode"], "disabled");
            assert!(selected(&doc)["reason"].as_str().unwrap().contains("覆盖"));
            put(&mut doc, scalar, sentinel).unwrap();
            assert_eq!(selected(&doc)["mode"], "editable");
        }
    }

    #[test]
    fn case_data_save_reads_frozen_process_assets_and_source_relative_data() {
        let (root, registry, mut draft) = fixture();
        let source = root.join("source");
        let stage = root.join("stage");
        std::fs::create_dir_all(source.join("process")).unwrap();
        std::fs::create_dir_all(source.join("runtime/ndep")).unwrap();
        std::fs::create_dir_all(stage.join("process")).unwrap();
        std::fs::write(
            source.join("runtime/ndep/fndep_colm_hist_simyr1849-2006_1.9x2.5_c100428.nc"),
            b"fixture",
        )
        .unwrap();
        std::fs::write(source.join("fresh-soil.nc"), b"fixture").unwrap();
        let original="&nl_colm\nDEF_CASE_NAME='original'\nDEF_dir_runtime='runtime'\nDEF_USE_SoilInit=.false.\nDEF_USE_LCT=.false.\nDEF_USE_PFT=.true.\nDEF_USE_PC=.false.\n/\n";
        std::fs::write(source.join("case.nml"), original).unwrap();
        let stale = b"&nl_colm_methane_parameter\nDEF_METHANE%inundation_mode='hybrid'\n/\n";
        let frozen_asset = b"&nl_colm_methane_parameter\nDEF_METHANE%inundation_mode='wetwat'\n/\n";
        std::fs::write(source.join("process/source-ch4.nml"), stale).unwrap();
        std::fs::write(stage.join("process/ch4.nml"), frozen_asset).unwrap();
        let source_alias = source
            .join("process/source-ch4.nml")
            .to_string_lossy()
            .into_owned();
        let reference = format!("'{source_alias}'");
        let mut case_doc = colm_namelist::parse(original).unwrap();
        for (key, value) in [
            ("DEF_USE_BGC", ".true."),
            ("DEF_USE_TRACER", ".true."),
            ("DEF_USE_Campbell_SOIL_MODEL", ".false."),
            ("DEF_USE_VariablySaturatedFlow", ".true."),
            ("DEF_TRACER_NAMES", "'CH4'"),
            ("DEF_TRACER_TYPES", "'gas'"),
            ("DEF_TRACER_NUM", "1"),
            ("DEF_TRACER_PARAM_FILES", reference.as_str()),
            ("DEF_USE_Dynamic_Wetland", ".false."),
            ("DEF_USE_NITRIF", ".false."),
            ("DEF_USE_FIRE", ".false."),
        ] {
            case_doc
                .insert(key, parse_value(value).unwrap(), "nl_colm")
                .unwrap();
        }
        std::fs::write(source.join("case.nml"), case_doc.to_string()).unwrap();
        std::fs::write(source.join(colm_model::CASE_DATA_FILE), serde_json::to_vec(&json!({
            "schema": 1, "source_case": source, "original_case_sha256": "a".repeat(64), "fields": {},
            "process_file_bindings": { (source_alias.clone()): "process/ch4.nml" }
        })).unwrap()).unwrap();
        draft.overlay.process_files.push(Asset {
            relative_path: "process/ch4.nml".into(),
            sha256: colm_kernel::sha256_hex(frozen_asset),
        });
        let generation = draft.generation;
        let draft = registry.save_draft(draft, generation).unwrap();
        let frozen = registry.freeze(&draft.id, &stage).unwrap();
        let preview = root.join("preview");
        registry
            .materialize_run(&frozen.version, &source, &preview, "data-query-test")
            .unwrap();
        let before_preview = std::fs::read(preview.join("case.nml")).unwrap();
        let before_revision = std::fs::read(
            root.join("versions")
                .join(&frozen.version.id)
                .join(frozen.version.revision.to_string())
                .join("revision.json"),
        )
        .unwrap();
        let context = vec![preview.to_string_lossy().into_owned()];
        let changes = vec![
            FieldChange {
                path: "DEF_TRACER_PARAM_FILES".into(),
                value: source_alias,
            },
            FieldChange {
                path: "DEF_CASE_NAME".into(),
                value: "updated".into(),
            },
            FieldChange {
                path: "DEF_USE_SoilInit".into(),
                value: ".true.".into(),
            },
            FieldChange {
                path: "DEF_file_SoilInit".into(),
                value: "fresh-soil.nc".into(),
            },
        ];
        super::super::set_fields_with_context(
            vec![source.to_string_lossy().into_owned()],
            changes,
            None,
            Some(&context),
        )
        .unwrap();
        let saved =
            colm_namelist::parse(&std::fs::read_to_string(source.join("case.nml")).unwrap())
                .unwrap();
        assert_eq!(
            saved.get("DEF_file_SoilInit").unwrap().to_string(),
            "'fresh-soil.nc'"
        );
        assert_eq!(
            std::fs::read(source.join("process/source-ch4.nml")).unwrap(),
            stale
        );
        assert_eq!(
            std::fs::read(preview.join("process/ch4.nml")).unwrap(),
            frozen_asset
        );
        assert_eq!(
            std::fs::read(preview.join("case.nml")).unwrap(),
            before_preview
        );
        assert_eq!(
            std::fs::read(
                root.join("versions")
                    .join(&frozen.version.id)
                    .join(frozen.version.revision.to_string())
                    .join("revision.json")
            )
            .unwrap(),
            before_revision
        );
        // A model-only process file also resolves after removing the obsolete source file.
        std::fs::remove_file(source.join("process/source-ch4.nml")).unwrap();
        super::super::set_fields_with_context(
            vec![source.to_string_lossy().into_owned()],
            vec![FieldChange {
                path: "DEF_CASE_NAME".into(),
                value: "updated-again".into(),
            }],
            None,
            Some(&context),
        )
        .unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn process_override_is_file_scoped_and_unknown_entries_are_read_only() {
        let bytes = b"&nl_colm_tracer_parameter\nDEF_TRACER%init_conc=1.\nunknown=7\n/\n";
        let assets = BTreeMap::from([
            ("a.nml".into(), bytes.to_vec()),
            ("b.nml".into(), bytes.to_vec()),
        ]);
        let mut overlay = Overlay::default();
        for name in ["a.nml", "b.nml"] {
            overlay.process_files.push(Asset {
                relative_path: name.into(),
                sha256: colm_kernel::sha256_hex(bytes),
            });
        }
        overlay.parameters.push(ParameterOverrideRecord {
            parameter_id: "process:tracer:DEF_TRACER%init_conc".into(),
            raw_key: "DEF_TRACER%init_conc".into(),
            scope_instance: ParameterScopeInstance {
                kind: "process-file".into(),
                scheme: None,
                index: None,
                type_name: None,
                process_file: Some("a.nml".into()),
            },
            value: "2.".into(),
            expected_old_value: None,
            unit: None,
        });
        let doc = colm_namelist::parse("").unwrap();
        let files = process_files(&overlay, &assets, &doc, &BTreeSet::new(), false, false).unwrap();
        let entry = |i: usize, key: &str| {
            files[i]["entries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["path"] == key)
                .unwrap()
        };
        assert_eq!(entry(0, "DEF_TRACER%init_conc")["value"], "2.");
        assert_eq!(entry(1, "DEF_TRACER%init_conc")["value"], "1.");
        assert_eq!(entry(0, "unknown")["editable"], false);
        assert_eq!(entry(0, "DEF_TRACER%init_conc")["editable"], true);
        assert_eq!(entry(0, "DEF_TRACER%init_conc")["asset_value"], "1.");
    }

    #[test]
    fn methane_activity_requires_explicit_mapping_and_selects_only_one_asset() {
        let bytes = b"&nl_colm_methane_parameter\nDEF_METHANE%inundation_mode='wetwat'\n/\n";
        let assets = BTreeMap::from([
            ("first.nml".into(), bytes.to_vec()),
            ("second.nml".into(), bytes.to_vec()),
        ]);
        let overlay = Overlay {
            process_files: ["first.nml", "second.nml"]
                .into_iter()
                .map(|name| Asset {
                    relative_path: name.into(),
                    sha256: colm_kernel::sha256_hex(bytes),
                })
                .collect(),
            ..Overlay::default()
        };
        let mut doc = colm_namelist::parse("").unwrap();
        let files = process_files(&overlay, &assets, &doc, &BTreeSet::new(), false, false).unwrap();
        assert!(files
            .iter()
            .all(|file| file["active_methane_asset"].is_null()));
        put(&mut doc, "DEF_TRACER_NAMES", "'CH4'").unwrap();
        let files = process_files(&overlay, &assets, &doc, &BTreeSet::new(), false, false).unwrap();
        assert!(files
            .iter()
            .all(|file| file["active_methane_asset"].is_null()));
        put(&mut doc, "DEF_TRACER_PARAM_FILES", "'second.nml'").unwrap();
        let files = process_files(&overlay, &assets, &doc, &BTreeSet::new(), false, false).unwrap();
        assert_eq!(files[0]["active_methane_asset"], false);
        assert_eq!(files[1]["active_methane_asset"], true);
        assert!(files.iter().all(|file| file["verified"] == true));
        assert!(assets.values().all(|asset| asset == bytes));
        put(&mut doc, "DEF_TRACER_NAMES", "'METHANE'").unwrap();
        let files = process_files(&overlay, &assets, &doc, &BTreeSet::new(), false, false).unwrap();
        assert_eq!(files[1]["active_methane_asset"], true);
    }

    #[test]
    fn methane_options_do_not_assume_missing_context_and_known_river_off_excludes_routing() {
        let bytes = b"&nl_colm_methane_parameter\nDEF_METHANE%inundation_mode='hybrid'\n/\n";
        let assets = BTreeMap::from([("ch4.nml".into(), bytes.to_vec())]);
        let overlay = Overlay {
            process_files: vec![Asset {
                relative_path: "ch4.nml".into(),
                sha256: colm_kernel::sha256_hex(bytes),
            }],
            ..Overlay::default()
        };
        let mut doc = colm_namelist::parse("").unwrap();
        let modes = |files: Vec<Value>| {
            files[0]["entries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["path"] == METHANE_MODE)
                .unwrap()["options"]
                .clone()
        };
        let all =
            modes(process_files(&overlay, &assets, &doc, &BTreeSet::new(), false, false).unwrap());
        assert!(all.as_array().unwrap().contains(&json!("routing")));
        assert!(all.as_array().unwrap().contains(&json!("hybrid")));
        put(&mut doc, "DEF_USE_GridRiverLakeFlow", ".false.").unwrap();
        let known =
            modes(process_files(&overlay, &assets, &doc, &BTreeSet::new(), false, false).unwrap());
        assert!(!known.as_array().unwrap().contains(&json!("routing")));
        assert!(!known.as_array().unwrap().contains(&json!("hybrid")));
        put(&mut doc, "DEF_USE_GridRiverLakeFlow", ".true.").unwrap();
        let spatial = BTreeSet::from(["GRIDBASED", "GridRiverLakeFlow", "LULC_IGBP"]);
        let known = modes(process_files(&overlay, &assets, &doc, &spatial, true, true).unwrap());
        assert!(known.as_array().unwrap().contains(&json!("hybrid")));
        put(&mut doc, "SITE_landtype", "5").unwrap();
        let single = BTreeSet::from(["SinglePoint", "LULC_IGBP"]);
        let known = modes(process_files(&overlay, &assets, &doc, &single, true, true).unwrap());
        assert_eq!(known, json!(["wetwat", "satellite"]));
    }
}

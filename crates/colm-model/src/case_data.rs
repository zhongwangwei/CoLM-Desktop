use crate::{validation, CaseData, CaseFieldEdit, Overlay, CASE_DATA_FILE, SCHEMA};
use anyhow::{ensure, Result};
use colm_namelist::Document;
use std::collections::BTreeMap;
use std::path::Path;

fn load_case_data(case_dir: &Path) -> Result<Option<CaseData>> {
    let path = case_dir.join(CASE_DATA_FILE);
    let data: CaseData = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(data.schema == SCHEMA, "unsupported case-data schema");
    for key in data.case_field_edits.keys() {
        ensure!(
            validation::legacy_model_case_control(key),
            "unsupported historical case field edit: {key}"
        );
    }
    Ok(Some(data))
}

/// 返回待保存的元数据，由调用方与 case.nml 放在同一写入事务中。
pub fn case_data_with_field_edits(
    case_dir: &Path,
    edited_fields: &[String],
    removed_fields: &[String],
) -> Result<Option<CaseData>> {
    if !edited_fields
        .iter()
        .chain(removed_fields)
        .any(|key| validation::legacy_model_case_control(key))
    {
        return Ok(None);
    }
    let Some(mut data) = load_case_data(case_dir)? else {
        return Ok(None);
    };
    for (keys, edit) in [
        (edited_fields, CaseFieldEdit::Set),
        (removed_fields, CaseFieldEdit::Removed),
    ] {
        for key in keys {
            if validation::legacy_model_case_control(key) {
                let field = colm_schema::find(key).expect("validated case field");
                data.case_field_edits.insert(field.name.into(), edit);
            }
        }
    }
    Ok(Some(data))
}

/// 旧算例保留原模型覆盖；已接受的新编辑和删除由算例元数据决定。
pub fn apply_legacy_case_overrides(
    doc: &mut Document,
    overlay: &Overlay,
    case_dir: Option<&Path>,
    edited_fields: &[String],
    removed_fields: &[String],
) -> Result<BTreeMap<String, String>> {
    let data = case_dir.map(load_case_data).transpose()?.flatten();
    let mut applied = BTreeMap::new();
    for (key, raw) in &overlay.legacy_case_overrides {
        let contains = |keys: &[String]| keys.iter().any(|field| field.eq_ignore_ascii_case(key));
        if contains(removed_fields) {
            doc.remove(key)?;
            continue;
        }
        if contains(edited_fields) {
            continue;
        }
        match data.as_ref().and_then(|data| {
            data.case_field_edits
                .iter()
                .find(|(field, _)| field.eq_ignore_ascii_case(key))
                .map(|(_, edit)| edit)
        }) {
            Some(CaseFieldEdit::Removed) => {
                doc.remove(key)?;
            }
            Some(CaseFieldEdit::Set) => {}
            None => {
                let value = validation::legacy_case_value(key, raw)?;
                let group = colm_schema::find(key)
                    .and_then(|field| field.group)
                    .expect("validated legacy case field group");
                doc.ensure_group(group)?;
                doc.insert(key, value, group)?;
                applied.insert(key.clone(), raw.clone());
            }
        }
    }
    Ok(applied)
}

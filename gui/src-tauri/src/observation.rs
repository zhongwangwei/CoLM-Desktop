//! 可选验证数据的表格探测与转换。窗口进程不链接 netcdf，仍走 colm-cli sidecar。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationColumn {
    pub name: String,
    pub units: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationSite {
    pub id: String,
    pub rows: usize,
    pub start: Option<String>,
    pub end: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationVariable {
    pub name: String,
    pub label: String,
    pub units: String,
    pub column: Option<String>,
    pub qc_column: Option<String>,
    pub requires_qc: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationProbe {
    pub delimiter: String,
    pub columns: Vec<ObservationColumn>,
    pub rows: usize,
    pub site_column: Option<String>,
    pub time_column: Option<String>,
    pub sites: Vec<ObservationSite>,
    pub variables: Vec<ObservationVariable>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ObservationConvertOptions {
    pub time_column: String,
    pub site_column: Option<String>,
    pub site_name: Option<String>,
    pub variables: Vec<ObservationChoice>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ObservationChoice {
    pub name: String,
    pub column: String,
    pub qc_column: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertedObservationSite {
    pub site: String,
    pub path: String,
    pub rows: usize,
    pub start: String,
    pub end: String,
    pub variables: Vec<String>,
    pub warnings: Vec<String>,
}

#[tauri::command]
pub async fn probe_observation_table(path: String) -> Result<ObservationProbe, String> {
    let json = crate::sidecar::capture_async(vec![
        "observation-table-probe".into(),
        path,
        "--json".into(),
        "1".into(),
    ])
    .await?;
    serde_json::from_str(&json).map_err(|error| {
        format!(
            "colm-cli observation-table-probe 的输出解析不了（两边的字段可能已经对不上）：{error}"
        )
    })
}

#[tauri::command]
pub async fn convert_observation_table(
    src: String,
    dst_dir: String,
    options: ObservationConvertOptions,
) -> Result<Vec<ConvertedObservationSite>, String> {
    std::fs::create_dir_all(&dst_dir)
        .map_err(|error| format!("验证数据目录 {dst_dir} 无法创建：{error}"))?;
    let mut args = vec![
        "observation-table-convert".into(),
        src,
        dst_dir,
        "--time-column".into(),
        options.time_column,
    ];
    if let Some(site_column) = options.site_column.filter(|v| !v.trim().is_empty()) {
        args.push("--site-column".into());
        args.push(site_column);
    }
    if let Some(site_name) = options.site_name.filter(|v| !v.trim().is_empty()) {
        args.push("--site-name".into());
        args.push(site_name);
    }
    for variable in options.variables {
        let mut spec = format!("{}={}", variable.name, variable.column);
        if let Some(qc) = variable.qc_column.filter(|v| !v.trim().is_empty()) {
            spec.push(':');
            spec.push_str(&qc);
        }
        args.push("--variable".into());
        args.push(spec);
    }
    args.push("--json".into());
    args.push("1".into());
    let json = crate::sidecar::capture_async(args).await?;
    serde_json::from_str(&json).map_err(|error| {
        format!("colm-cli observation-table-convert 的输出解析不了（两边的字段可能已经对不上）：{error}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_options_use_the_frontend_snake_case_contract() {
        let options: ObservationConvertOptions = serde_json::from_value(serde_json::json!({
            "time_column": "time",
            "site_column": "site",
            "site_name": null,
            "variables": [{ "name": "Qle", "column": "LE", "qc_column": "LE_qc" }]
        }))
        .unwrap();
        assert_eq!(options.time_column, "time");
        assert_eq!(options.variables[0].qc_column.as_deref(), Some("LE_qc"));
    }
}

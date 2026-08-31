//! 配置层的进程内命令。
//!
//! 这几个都不碰文件系统之外的东西，也都不需要 netcdf ——
//! `colm-schema` 是一张生成的静态表，`colm-namelist` 是纯文本解析。

use serde::{Deserialize, Serialize};

/// 把源码 namelist 字段放进用户看得懂的功能分组。
///
/// 返回 `None` 不是「其他」：测试要求当前 CoLM 源码里一个都不能剩。
/// 上游新增字段时 CI 会报出名字，要求读过它的用途后再归类。
pub(crate) fn field_section(name: &str, group: Option<&str>) -> Option<&'static str> {
    colm_case::parameters::field_section(name, group)
}

/// 页面加载时确认后端确实接上了。
///
/// 顺便往 stderr 记一行。这不是调试残留：GUI 出问题时最难分辨的两种情况是
/// 「窗口没开」与「窗口开了但页面是白的」—— 前者进程会退出，后者进程活着、
/// 窗口标题也在，从外面看一模一样。这一行是唯一能从外面区分它们的证据，
/// 因为只有 webview 真的加载并执行了 `index.html` 的 JS 才会调到这里。
/// 同一行还报出它解析到的 `colm-cli` 路径。`resolve_cli` 有四条回落，
/// 其中「仓库的 target/ 产物」那条在开发机上**永远命中**，于是打包版本
/// 找错了 sidecar 也看不出来 —— 实测就发生过：Tauri 把 sidecar 放进
/// `Contents/MacOS/`，而当时的代码找的是 `Contents/Resources/`。
#[tauri::command]
pub fn backend_ready() -> String {
    let msg = format!(
        "backend reachable — {} configuration fields known",
        colm_schema::all().len()
    );
    let cli = crate::sidecar::resolve_cli();
    eprintln!(
        "colm-desktop: the page reached the backend; {msg}; colm-cli resolved to {}",
        cli.display()
    );
    msg
}

/// 前端把话说到 stderr。**这是这台机器上 GUI 唯一可靠的观察通道** ——
/// AX 树读取时灵时不灵、`screencapture` 没有屏幕录制权限，两条都实测不可用。
///
/// 不引 `tauri-plugin-log`：那个插件要在 webview 侧注入 console 钩子，而这里
/// 恰恰要诊断「前端代码到底跑没跑」。诊断工具依赖被诊断的那一层，说明不了问题。
#[tauri::command]
pub fn probe_log(msg: String) {
    eprintln!("colm-desktop[probe]: {msg}");
}

/// 一个配置字段，交给前端渲染。
#[derive(Serialize)]
pub struct Field {
    pub name: &'static str,
    pub kind: String,
    pub default: String,
    pub doc: Option<&'static str>,
    /// 它属于哪个 namelist 组，也就是**该写进哪个文件**。
    pub group: Option<&'static str>,
    /// `true` 表示用户设了也没用 —— 有声明有默认值，但不在任何 namelist 组里。
    /// 实测 6 个，其中 `DEF_dir_history` 在 `MOD_Namelist.F90:1406` 被无条件覆盖。
    /// 界面该把它们显示成只读的派生值：给一个改了没用的输入框比不显示更糟。
    pub derived: bool,
    /// 合法取值，非空时界面给下拉框而不是文本框。当前 30 个字段有。
    pub values: &'static [&'static str],
    /// 需要哪些编译期宏。与所选内核 `manifest.json` 的 `macros` 求交，
    /// 交不上就说明这个字段在当前内核下**根本没用**。实测 68 个字段有依赖。
    pub requires: &'static [&'static str],
    /// 从 CoLM 源码字段名与 namelist 组推导的功能分组。
    pub section: &'static str,
}

fn default_literal(value: colm_schema::Default) -> String {
    match value {
        colm_schema::Default::Logical(value) => {
            if value { ".true." } else { ".false." }.to_string()
        }
        colm_schema::Default::Integer(value) => value.to_string(),
        colm_schema::Default::Real(value)
        | colm_schema::Default::Str(value)
        | colm_schema::Default::Array(value) => value.to_string(),
    }
}

#[tauri::command]
pub fn describe_fields() -> Vec<Field> {
    colm_schema::all()
        .iter()
        .map(|f| Field {
            name: f.name,
            kind: format!("{:?}", f.kind),
            // 前端会把未显式设置的字段默认值直接放进控件。Debug 文本
            // `Integer(3)` / `Logical(true)` 不是 Fortran 值，会生成无法保存的
            // 数字框或非法选项；必须传可直接写回 namelist 的字面量。
            default: default_literal(f.default),
            doc: f.doc,
            group: f.group,
            derived: f.group.is_none(),
            values: f.values,
            requires: f.requires,
            section: field_section(f.name, f.group).unwrap_or("未分类（这应该被测试拦住）"),
        })
        .collect()
}

#[tauri::command]
pub fn parameter_catalog() -> Vec<colm_case::parameters::ParameterDescriptor> {
    colm_case::parameters::all().to_vec()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterScopeInstance {
    pub kind: String,
    pub scheme: Option<String>,
    pub index: Option<u8>,
    pub type_name: Option<String>,
    pub process_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterOverrideRecord {
    pub parameter_id: String,
    pub raw_key: String,
    pub scope_instance: ParameterScopeInstance,
    pub value: String,
    pub expected_old_value: Option<String>,
    pub unit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseParameterOverrides {
    pub source_case: String,
    pub classification_scheme: Option<String>,
    pub records: Vec<ParameterOverrideRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterOverrideBundle {
    pub schema_version: u32,
    pub catalog_version: u32,
    pub kernel_identity: Option<String>,
    pub created_unix: u64,
    pub cases: Vec<CaseParameterOverrides>,
}

fn exported_record(
    descriptor: &colm_case::parameters::ParameterDescriptor,
    value: String,
    scope_instance: ParameterScopeInstance,
) -> ParameterOverrideRecord {
    ParameterOverrideRecord {
        parameter_id: descriptor.id.clone(),
        raw_key: descriptor.raw_key.clone(),
        scope_instance,
        // The optimistic-concurrency value belongs to the import target and is
        // captured by preview's required version token, not by the source case.
        expected_old_value: None,
        value,
        unit: descriptor.unit.clone(),
    }
}

/// Export only explicit overrides. Inherited defaults never enter this bundle.
#[tauri::command]
pub fn export_parameter_overrides(
    dirs: Vec<String>,
    kernel_dir: Option<String>,
) -> Result<ParameterOverrideBundle, String> {
    if dirs.is_empty() {
        return Err("没有可导出的算例".into());
    }
    let kernel = kernel_dir
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .map(|path| {
            colm_kernel::Kernel::open(std::path::Path::new(path))
                .map_err(|error| format!("{error:#}"))
        })
        .transpose()?;
    let kernel_identity = kernel.as_ref().map(|kernel| kernel.manifest.identity());
    let have = kernel
        .as_ref()
        .map(|kernel| kernel.manifest.macros.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let mut cases = Vec::with_capacity(dirs.len());
    for dir in dirs {
        let text = std::fs::read_to_string(std::path::Path::new(&dir).join("case.nml"))
            .map_err(|error| format!("{dir}: {error}"))?;
        let doc = colm_namelist::parse(&text).map_err(|error| format!("{dir}: {error:#}"))?;
        let context = VisibilityContext::new(&doc, &have);
        let usgs = if have.is_empty() {
            logical(&doc, "DEF_USE_USGS")
        } else {
            context.usgs
        };
        let land_scheme = if usgs { "USGS" } else { "IGBP" }.to_string();
        let scheme = context.lct.then(|| land_scheme.clone());
        let mut records = Vec::new();
        for path in doc.paths() {
            let Some(value) = doc.get(&path).map(ToString::to_string) else {
                continue;
            };
            if let Some((raw_key, pft_type)) = colm_case::pft::override_instance(&path) {
                let id = format!("{}:{raw_key}", if context.pc { "pc-pft" } else { "pft" });
                let Some(descriptor) = colm_case::parameters::find(&id) else {
                    continue;
                };
                let names = colm_case::pft::pft_name(pft_type);
                records.push(exported_record(
                    descriptor,
                    value,
                    ParameterScopeInstance {
                        kind: if context.pc { "pc-pft" } else { "pft" }.into(),
                        scheme: None,
                        index: Some(pft_type),
                        type_name: names.map(|name| name.en.to_string()),
                        process_file: None,
                    },
                ));
                continue;
            }
            let Some(field) = colm_schema::find(&path).filter(|field| field.group.is_some()) else {
                continue;
            };
            let id = if colm_case::land_cover::is_parameter(field.name) {
                format!("lct:{land_scheme}:{}", field.name)
            } else {
                format!("case:{}", field.name)
            };
            let Some(descriptor) = colm_case::parameters::find(&id) else {
                continue;
            };
            records.push(exported_record(
                descriptor,
                value,
                ParameterScopeInstance {
                    kind: if colm_case::land_cover::is_parameter(field.name) {
                        "land-cover-class"
                    } else {
                        "case-scalar"
                    }
                    .into(),
                    scheme: colm_case::land_cover::is_parameter(field.name)
                        .then(|| land_scheme.clone()),
                    index: (context.lct && context.valid_landtype())
                        .then_some(context.site_landtype as u8),
                    type_name: None,
                    process_file: None,
                },
            ));
        }
        for file in process_parameter_files(dir.clone())? {
            for entry in file.entries.into_iter().filter(|entry| {
                !entry.unset
                    && !entry.default.as_deref().is_some_and(|default| {
                        process_values_equal(entry.kind, &entry.value, default)
                    })
            }) {
                let Some(descriptor) = colm_case::parameters::process_descriptors()
                    .into_iter()
                    .find(|item| item.raw_key.eq_ignore_ascii_case(&entry.path))
                else {
                    continue;
                };
                records.push(exported_record(
                    descriptor,
                    entry.value,
                    ParameterScopeInstance {
                        kind: "process-file".into(),
                        scheme: None,
                        index: None,
                        type_name: None,
                        process_file: Some(file.file.clone()),
                    },
                ));
            }
        }
        records.sort_by(|left, right| {
            left.parameter_id
                .cmp(&right.parameter_id)
                .then(left.scope_instance.index.cmp(&right.scope_instance.index))
        });
        cases.push(CaseParameterOverrides {
            source_case: std::path::Path::new(&dir)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(&dir)
                .to_string(),
            classification_scheme: scheme,
            records,
        });
    }
    Ok(ParameterOverrideBundle {
        schema_version: 1,
        catalog_version: colm_case::parameters::CATALOG_VERSION,
        kernel_identity,
        created_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        cases,
    })
}

fn process_values_equal(kind: &str, left: &str, right: &str) -> bool {
    match kind {
        "logical" => logical_literal(left) == logical_literal(right),
        "integer" => left.trim().parse::<i64>().ok() == right.trim().parse::<i64>().ok(),
        "real" => match (parse_real(left), parse_real(right)) {
            (Some(left), Some(right)) => left.to_bits() == right.to_bits(),
            _ => false,
        },
        _ => left.trim().trim_matches(['\'', '"']) == right.trim().trim_matches(['\'', '"']),
    }
}

fn logical_literal(raw: &str) -> Option<bool> {
    match raw.trim().trim_matches('.').to_ascii_lowercase().as_str() {
        "true" | "t" => Some(true),
        "false" | "f" => Some(false),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ParameterImportItem {
    pub target_case: String,
    pub parameter_id: String,
    pub status: String,
    pub reason: Option<String>,
    pub current_value: Option<String>,
    pub new_value: String,
    pub file: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParameterImportPreview {
    pub can_apply: bool,
    pub catalog_version_compatible: bool,
    pub version_token: String,
    pub items: Vec<ParameterImportItem>,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParameterImportResult {
    pub records: usize,
    pub changed: usize,
    pub files: Vec<String>,
}

struct PreparedParameterImport {
    preview: ParameterImportPreview,
    writes: Vec<(std::path::PathBuf, String)>,
}

fn read_parameter_override_bundle(file: &str) -> Result<ParameterOverrideBundle, String> {
    let text = if file.trim_start().starts_with('{') {
        file.to_string()
    } else {
        std::fs::read_to_string(file).map_err(|error| format!("{file}: {error}"))?
    };
    serde_json::from_str(&text).map_err(|error| format!("参数覆盖文件无效：{error}"))
}

fn import_source_for<'a>(
    bundle: &'a ParameterOverrideBundle,
    target: &str,
) -> Result<&'a CaseParameterOverrides, String> {
    if bundle.cases.len() == 1 {
        return Ok(&bundle.cases[0]);
    }
    bundle
        .cases
        .iter()
        .find(|case| case.source_case == target)
        .ok_or_else(|| format!("导入包有多个来源算例，但没有与 {target} 同名的记录"))
}

fn files_version(files: &[(std::path::PathBuf, String)]) -> Result<String, String> {
    use std::hash::{Hash, Hasher};
    let mut paths = files.iter().map(|(path, _)| path).collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for path in paths {
        path.hash(&mut hash);
        std::fs::read(path)
            .map_err(|error| format!("{}: {error}", path.display()))?
            .hash(&mut hash);
    }
    Ok(format!("{:016x}", hash.finish()))
}

fn prepare_parameter_import(
    dirs: &[String],
    file: &str,
    kernel_dir: Option<&str>,
) -> Result<PreparedParameterImport, String> {
    if dirs.is_empty() {
        return Err("没有可导入的算例".into());
    }
    let bundle = read_parameter_override_bundle(file)?;
    let catalog_ok = bundle.catalog_version == colm_case::parameters::CATALOG_VERSION;
    let kernel = kernel_dir
        .filter(|path| !path.trim().is_empty())
        .map(|path| {
            colm_kernel::Kernel::open(std::path::Path::new(path))
                .map_err(|error| format!("{error:#}"))
        })
        .transpose()?;
    let kernel_identity = kernel.as_ref().map(|kernel| kernel.manifest.identity());
    let kernel_ok = bundle.kernel_identity.is_none() || bundle.kernel_identity == kernel_identity;
    let have = kernel
        .as_ref()
        .map(|kernel| kernel.manifest.macros.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let facts = kernel_facts(kernel_dir)?;
    let mut items = Vec::new();
    let mut writes = Vec::new();

    for dir in dirs {
        let target = std::path::Path::new(dir)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(dir);
        let source = import_source_for(&bundle, target)?;
        let case_file = std::path::Path::new(dir).join("case.nml");
        let text = std::fs::read_to_string(&case_file)
            .map_err(|error| format!("{}: {error}", case_file.display()))?;
        let mut doc = colm_namelist::parse(&text)
            .map_err(|error| format!("{}: {error:#}", case_file.display()))?;
        let (context_lct, context_pft, context_pc, site_landtype, context_usgs) = {
            let context = VisibilityContext::new_at(&doc, &have, Some(std::path::Path::new(dir)));
            (
                context.lct,
                context.pft,
                context.pc,
                context.site_landtype,
                context.usgs,
            )
        };
        let usgs = if have.is_empty() {
            logical(&doc, "DEF_USE_USGS")
        } else {
            context_usgs
        };
        let target_scheme = if usgs { "USGS" } else { "IGBP" };
        let mut changed_fields = Vec::new();
        let mut process_docs =
            std::collections::BTreeMap::<std::path::PathBuf, colm_namelist::Document>::new();

        for record in &source.records {
            let mut item = ParameterImportItem {
                target_case: target.to_string(),
                parameter_id: record.parameter_id.clone(),
                status: "incompatible".into(),
                reason: None,
                current_value: None,
                new_value: record.value.clone(),
                file: case_file.to_string_lossy().into_owned(),
            };
            let Some(descriptor) = colm_case::parameters::all()
                .iter()
                .find(|descriptor| descriptor.id == record.parameter_id)
            else {
                item.reason = Some("当前目录不存在这个稳定参数 ID".into());
                items.push(item);
                continue;
            };
            if !descriptor.raw_key.eq_ignore_ascii_case(&record.raw_key) {
                item.reason = Some("稳定 ID 与原始 CoLM 键不匹配".into());
                items.push(item);
                continue;
            }
            if !catalog_ok {
                item.reason = Some("参数目录版本不兼容".into());
                items.push(item);
                continue;
            }
            if !kernel_ok {
                item.reason = Some("内核身份与导出包不一致".into());
                items.push(item);
                continue;
            }

            use colm_case::parameters::Storage;
            let result: Result<(Option<String>, std::path::PathBuf), String> = match descriptor
                .storage
            {
                Storage::CaseNml => {
                    if matches!(
                        descriptor.scope,
                        colm_case::parameters::ParameterScope::LandCoverClass
                    ) {
                        if !context_lct {
                            Err("目标算例当前不是 LCT 模式".into())
                        } else if record.scope_instance.scheme.as_deref() != Some(target_scheme) {
                            Err("禁止在 IGBP 与 USGS 之间按数字索引导入".into())
                        } else if record.scope_instance.index.map(i64::from) != Some(site_landtype)
                        {
                            Err("目标算例地类与导出作用域不同".into())
                        } else {
                            let current = doc.get(&record.raw_key).map(ToString::to_string);
                            let value = typed(&record.raw_key, &record.value)?;
                            put(&mut doc, &record.raw_key, value)?;
                            changed_fields.push(FieldChange {
                                path: record.raw_key.clone(),
                                value: record.value.clone(),
                            });
                            Ok((current, case_file.clone()))
                        }
                    } else {
                        let current = doc.get(&record.raw_key).map(ToString::to_string);
                        let value = typed(&record.raw_key, &record.value)?;
                        put(&mut doc, &record.raw_key, value)?;
                        changed_fields.push(FieldChange {
                            path: record.raw_key.clone(),
                            value: record.value.clone(),
                        });
                        Ok((current, case_file.clone()))
                    }
                }
                Storage::PftOverride | Storage::PcPftOverride => {
                    let wants_pc = matches!(descriptor.storage, Storage::PcPftOverride);
                    if context_pc != wants_pc || context_pft == wants_pc {
                        Err("PFT 与 PC-PFT 作用域不匹配".into())
                    } else if source.source_case != target {
                        Err("PFT/PC 导入只允许回到同名算例，避免修改站点不存在的组分".into())
                    } else {
                        let pft_type = record
                            .scope_instance
                            .index
                            .ok_or_else(|| "PFT/PC 导入缺少类型索引".to_string())?;
                        let meta = colm_case::pft::parameter(&record.raw_key)
                            .ok_or_else(|| format!("未知 PFT 参数：{}", record.raw_key))?;
                        let applies = {
                            let context = VisibilityContext::new_at(
                                &doc,
                                &have,
                                Some(std::path::Path::new(dir)),
                            );
                            pft_parameter_applies(meta, &context, pft_type)
                                && pft_parameter_has_default(meta, &context, pft_type)?
                        };
                        if !applies {
                            Err("参数在目标 PFT/PC 上当前不生效".into())
                        } else {
                            let path = pft_override_path(meta.name, pft_type);
                            let current = doc.get(&path).map(ToString::to_string);
                            doc.insert(&path, typed_pft_value(meta, &record.value)?, "nl_colm")
                                .map_err(|error| format!("{error:#}"))?;
                            Ok((current, case_file.clone()))
                        }
                    }
                }
                Storage::ProcessParameterFile => {
                    let file = record
                        .scope_instance
                        .process_file
                        .as_deref()
                        .ok_or_else(|| "过程参数缺少 case-local 文件名".to_string())?;
                    let path = safe_process_file(std::path::Path::new(dir), file)?;
                    if !process_docs.contains_key(&path) {
                        let text = std::fs::read_to_string(&path)
                            .map_err(|error| format!("{}: {error}", path.display()))?;
                        process_docs.insert(
                            path.clone(),
                            colm_namelist::parse(&text)
                                .map_err(|error| format!("{}: {error:#}", path.display()))?,
                        );
                    }
                    let process_doc = process_docs.get_mut(&path).expect("inserted above");
                    let current = process_doc.get(&record.raw_key).map(ToString::to_string);
                    let code = process_code_defaults()
                        .into_iter()
                        .find(|field| field.path.eq_ignore_ascii_case(&record.raw_key))
                        .ok_or_else(|| format!("未知过程参数：{}", record.raw_key))?;
                    if let Some(existing) = process_doc.get(&record.raw_key).cloned() {
                        process_doc
                            .set(
                                &record.raw_key,
                                typed_process_known(code.kind, &existing, &record.value)?,
                            )
                            .map_err(|error| format!("{error:#}"))?;
                    } else if code.insertable {
                        process_doc
                            .insert(
                                &record.raw_key,
                                typed_process(code.kind, &record.value)?,
                                code.group,
                            )
                            .map_err(|error| format!("{error:#}"))?;
                    } else {
                        return Err(format!("{} 不是可安全新增的过程参数", record.raw_key));
                    }
                    Ok((current, path))
                }
                Storage::ReadOnly => Err("只读参数不能导入".into()),
            };
            match result {
                Ok((current, path)) => {
                    item.current_value = current.clone();
                    item.file = path.to_string_lossy().into_owned();
                    item.status = if current.as_deref() == Some(record.value.as_str()) {
                        "no-change"
                    } else if current.is_some() {
                        "overwrite"
                    } else {
                        "applicable"
                    }
                    .into();
                }
                Err(reason) => item.reason = Some(reason),
            }
            items.push(item);
        }

        if let Err(reason) = validate_runtime_contract(&doc, std::path::Path::new(dir), facts)
            .and_then(|_| validate_changed_fields(&doc, &changed_fields))
        {
            items.push(ParameterImportItem {
                target_case: target.to_string(),
                parameter_id: "<batch>".into(),
                status: "incompatible".into(),
                reason: Some(reason),
                current_value: None,
                new_value: String::new(),
                file: case_file.to_string_lossy().into_owned(),
            });
        }
        writes.push((case_file, doc.to_string()));
        writes.extend(
            process_docs
                .into_iter()
                .map(|(path, doc)| (path, doc.to_string())),
        );
    }
    writes.sort_by(|left, right| left.0.cmp(&right.0));
    writes.dedup_by(|left, right| left.0 == right.0);
    let version_token = files_version(&writes)?;
    let can_apply = items.iter().all(|item| item.status != "incompatible");
    let files = writes
        .iter()
        .filter(|(path, text)| {
            std::fs::read_to_string(path)
                .map(|current| current != *text)
                .unwrap_or(true)
        })
        .map(|(path, _)| path.to_string_lossy().into_owned())
        .collect();
    Ok(PreparedParameterImport {
        preview: ParameterImportPreview {
            can_apply,
            catalog_version_compatible: catalog_ok,
            version_token,
            items,
            files,
        },
        writes,
    })
}

#[tauri::command]
pub fn preview_import_parameter_overrides(
    dirs: Vec<String>,
    file: String,
    kernel_dir: Option<String>,
) -> Result<ParameterImportPreview, String> {
    Ok(prepare_parameter_import(&dirs, &file, kernel_dir.as_deref())?.preview)
}

#[tauri::command]
pub fn apply_import_parameter_overrides(
    dirs: Vec<String>,
    file: String,
    expected_version: String,
    kernel_dir: Option<String>,
) -> Result<ParameterImportResult, String> {
    let prepared = prepare_parameter_import(&dirs, &file, kernel_dir.as_deref())?;
    if !prepared.preview.can_apply {
        return Err("导入预检失败；没有修改任何文件".into());
    }
    if expected_version != prepared.preview.version_token {
        return Err("预检后配置已被外部修改；请重新预览导入".into());
    }
    let changed = write_files_atomic(&prepared.writes)?;
    Ok(ParameterImportResult {
        records: prepared.preview.items.len(),
        changed,
        files: prepared.preview.files,
    })
}

/// 在给定内核下，哪些字段**用不上**。
///
/// 判据是内核 `manifest.json` 里的 `macros` —— 那是**构建期就写下的事实**，
/// 不是运行时猜的。字段要求的宏有一个不在里面，它在这个内核下就没有意义：
/// 用户设了不会有任何效果，而界面上摆着它只会让人以为设了有用。
///
/// 返回的是**用不上的**那一批，不是能用的：前端拿同一份名单同时过滤
/// 参数与输出变量，切换内核后重新读取即可。
#[tauri::command]
pub fn irrelevant_fields(kernel_dir: String) -> Result<Vec<String>, String> {
    let k = colm_kernel::Kernel::open(std::path::Path::new(&kernel_dir))
        .map_err(|e| format!("{e:#}"))?;
    let have: std::collections::BTreeSet<&str> =
        k.manifest.macros.iter().map(String::as_str).collect();
    Ok(colm_schema::all()
        .iter()
        .filter(|f| !field_is_relevant(f, &have))
        .map(|f| f.name.to_string())
        .collect())
}

/// 一个源码字段是否对这组内核宏有意义。
fn field_is_relevant(field: &colm_schema::Field, have: &std::collections::BTreeSet<&str>) -> bool {
    // 这项在 MOD_Namelist.F90 里无条件派生 history/restart 路径；源码用法扫描
    // 排除了该文件，所以会误把它只归给 CatchLateralFlow。
    if field.name == "DEF_dir_output" {
        return true;
    }
    if !field.requires.iter().all(|m| have.contains(m)) {
        return false;
    }
    match field_section(field.name, field.group) {
        // 这些开关有一部分在公共 namelist 代码里无守护地出现，但对应子系统
        // 没编进内核时设置它们仍然不会产生任何效果。
        //
        // 没有「城市」这一条了：LULC/BGC/CROP/URBAN/LULCC 那组改造把
        // URBAN_MODEL 也变成运行时开关了（`DEF_URBAN_RUN`），
        // `main/URBAN/` 始终编译进去，`URBAN_MODEL` 本身从
        // `include/define.h` 里彻底消失——城市字段现在在每个内核下都
        // 「有意义」（能不能真的看到城市输出取决于 `DEF_URBAN_RUN`
        // 本身怎么设，那是运行时的事，不是这个函数管的编译期相关性）。
        Some("数据同化") => have.contains("DataAssimilation"),
        // 单点内核没有河网，整个分栏都没有意义。上游至少有
        // `DEF_ElementNeighbour_file` 和 `DEF_Reservoir_Method` 漏了 `requires`；
        // 只逐项补洞会让下一个漏标字段再次把空分栏撑出来。因此这里以过程
        // 是否编进内核为分栏总闸门，字段自身更细的 `requires` 已在上面检查。
        Some("河道与水库") => {
            have.contains("CaMa_Flood")
                || have.contains("GridRiverLakeFlow")
                || have.contains("CatchLateralFlow")
        }
        // SinglePoint 在时间管理器里自己固定 360×180 block 映射，不读区域边界、
        // mesh、PIO 分组或用户给的 block 划分。CPU 并发是 GUI 自己的批量设置，
        // 不属于这些 namelist 字段；入口保留，但这一整张无效参数表要隐藏。
        Some("网格与并行") => !have.contains("SinglePoint"),
        _ => true,
    }
}

/// 一个字段在当前算例里的交互状态。
///
/// `irrelevant_fields` 只回答编译期问题；这里把内核宏与 case.nml 当前值组合起来，
/// 避免前端各处分散维护一套迟早会漂移的依赖关系。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldMode {
    Editable,
    Disabled,
    Hidden,
}

#[derive(Debug, Clone, Serialize)]
pub struct FieldState {
    pub name: String,
    pub mode: FieldMode,
    pub reason: Option<&'static str>,
    /// 非空时覆盖 schema 的取值集合。用于表达运行时互斥和 SinglePoint
    /// 不支持的枚举值，而不是等 CoLM 启动以后再纠正。
    pub allowed_values: Vec<&'static str>,
    /// 批量中至少两个算例对这个字段的状态不同。前端仍按“任一算例有效就显示”
    /// 的安全方向处理，但必须明确警告，不能让代表算例掩盖差异。
    pub mixed: bool,
    /// `MOD_Const_LC.F90` 按分类体系和地类解析出的内置值。它不是 case.nml
    /// 里的显式覆盖，因此与 schema 中的“未设置”哨兵分开传给前端。
    pub context_default: Option<String>,
    /// 批量站点的内置地类默认值不同。仍允许 All 写同一个显式覆盖，但前端
    /// 必须先提醒用户，不能把第一个站点的值冒充整批默认值。
    pub default_mixed: bool,
    /// 当前字段实际指向的类型/槽位。普通算例标量为空；LCT 参数例如
    /// `IGBP-5` / `USGS-14`。前端只展示，不自行推断作用域。
    pub scope_label: Option<String>,
    /// 模型或类型表的内置值，与 case.nml 中是否显式设置分开。
    pub built_in_default: Option<String>,
    /// 当前文件中的显式覆盖；`None` 表示继承内置值。
    pub override_value: Option<String>,
    /// 当前运行实际使用的值。
    pub effective_value: Option<String>,
    /// 有效值来源，例如 `case.nml`、`MOD_Const_LC.F90` 或
    /// `MOD_Namelist.F90`。
    pub provenance: String,
    /// 批量目标的显式覆盖或有效值是否不同。
    pub override_mixed: bool,
    pub effective_mixed: bool,
}

struct VisibilityContext<'a> {
    doc: &'a colm_namelist::Document,
    have: &'a std::collections::BTreeSet<&'a str>,
    single: bool,
    usgs: bool,
    lct: bool,
    pft: bool,
    pc: bool,
    vg: bool,
    bgc: bool,
    crop: bool,
    urban: bool,
    ncar_urban: bool,
    lulcc: bool,
    tracer: bool,
    isotope_tracer: bool,
    site_landtype: i64,
    soil_init: bool,
    snow_init: bool,
    cn_init: bool,
    water_table_init: bool,
    downscale: bool,
    downscale_simple: bool,
    site_lai: bool,
    lai_feedback: bool,
    lai_change_yearly: bool,
    soil_reflectance_scheme: i64,
    runoff: i64,
    snicar: bool,
    aerosol_readin: bool,
    ozone_stress: bool,
    ozone_data: bool,
    plant_hydraulics: bool,
    dynamic_wetland: bool,
    interception: i64,
    medlyn: bool,
    wuest: bool,
}

fn logical(doc: &colm_namelist::Document, name: &str) -> bool {
    match doc.get(name) {
        Some(colm_namelist::Value::Bool(value)) => *value,
        _ => matches!(
            colm_schema::find(name).map(|field| field.default),
            Some(colm_schema::Default::Logical(true))
        ),
    }
}

fn integer(doc: &colm_namelist::Document, name: &str) -> i64 {
    match doc.get(name) {
        Some(colm_namelist::Value::Int(value)) => *value,
        _ => match colm_schema::find(name).map(|field| field.default) {
            Some(colm_schema::Default::Integer(value)) => value,
            _ => 0,
        },
    }
}

fn parse_real(value: &str) -> Option<f64> {
    value
        .split('_')
        .next()
        .unwrap_or(value)
        .replace(['d', 'D'], "e")
        .parse()
        .ok()
}

fn real(doc: &colm_namelist::Document, name: &str) -> f64 {
    match doc.get(name) {
        Some(value) => value.as_f64().or_else(|| parse_real(&value.to_string())),
        None => match colm_schema::find(name).map(|field| field.default) {
            Some(colm_schema::Default::Real(value)) => parse_real(value),
            Some(colm_schema::Default::Integer(value)) => Some(value as f64),
            _ => None,
        },
    }
    .unwrap_or(f64::NAN)
}

fn character(doc: &colm_namelist::Document, name: &str) -> String {
    match doc.get(name) {
        Some(colm_namelist::Value::Str(value)) => value.clone(),
        _ => match colm_schema::find(name).map(|field| field.default) {
            Some(colm_schema::Default::Str(value)) => value.to_string(),
            _ => String::new(),
        },
    }
}

fn ncar_urban_properties(
    doc: &colm_namelist::Document,
    case_dir: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    let raw = character(doc, "DEF_dir_rawdata");
    let raw = raw.trim();
    if raw.is_empty() || raw.eq_ignore_ascii_case("null") {
        return None;
    }
    let root = std::path::Path::new(raw);
    let root = if root.is_absolute() {
        root.to_path_buf()
    } else {
        case_dir?.join(root)
    };
    Some(root.join("urban/NCAR_urban_properties.nc"))
}

impl<'a> VisibilityContext<'a> {
    fn new(
        doc: &'a colm_namelist::Document,
        have: &'a std::collections::BTreeSet<&'a str>,
    ) -> Self {
        Self::new_at(doc, have, None)
    }

    fn new_at(
        doc: &'a colm_namelist::Document,
        have: &'a std::collections::BTreeSet<&'a str>,
        case_dir: Option<&std::path::Path>,
    ) -> Self {
        Self {
            doc,
            have,
            single: have.contains("SinglePoint"),
            usgs: have.contains("LULC_USGS"),
            lct: logical(doc, "DEF_USE_LCT"),
            pft: logical(doc, "DEF_USE_PFT"),
            pc: logical(doc, "DEF_USE_PC"),
            vg: !logical(doc, "DEF_USE_Campbell_SOIL_MODEL"),
            bgc: logical(doc, "DEF_USE_BGC"),
            // DEF_USE_CROP 是编译期数组尺寸选择的只读反映，不接受 case.nml
            // 里的伪开关；manifest 是唯一可信来源。
            crop: have.contains("CROP"),
            urban: logical(doc, "DEF_URBAN_RUN"),
            ncar_urban: ncar_urban_properties(doc, case_dir).is_some_and(|path| path.is_file()),
            lulcc: logical(doc, "DEF_USE_LULCC"),
            tracer: logical(doc, "DEF_USE_TRACER"),
            isotope_tracer: character(doc, "DEF_TRACER_TYPES")
                .split(',')
                .any(|kind| kind.trim().eq_ignore_ascii_case("isotope")),
            site_landtype: integer(doc, "SITE_landtype"),
            soil_init: logical(doc, "DEF_USE_SoilInit"),
            snow_init: logical(doc, "DEF_USE_SnowInit"),
            cn_init: logical(doc, "DEF_USE_CN_INIT"),
            water_table_init: logical(doc, "DEF_USE_WaterTableInit"),
            downscale: logical(doc, "DEF_USE_Forcing_Downscaling"),
            downscale_simple: logical(doc, "DEF_USE_Forcing_Downscaling_Simple"),
            site_lai: logical(doc, "USE_SITE_LAI"),
            lai_feedback: logical(doc, "DEF_USE_LAIFEEDBACK"),
            lai_change_yearly: logical(doc, "DEF_LAI_CHANGE_YEARLY"),
            soil_reflectance_scheme: integer(doc, "DEF_SOIL_REFL_SCHEME"),
            runoff: integer(doc, "DEF_Runoff_SCHEME"),
            snicar: logical(doc, "DEF_USE_SNICAR"),
            aerosol_readin: logical(doc, "DEF_Aerosol_Readin"),
            ozone_stress: logical(doc, "DEF_USE_OZONESTRESS"),
            ozone_data: logical(doc, "DEF_USE_OZONEDATA"),
            plant_hydraulics: logical(doc, "DEF_USE_PLANTHYDRAULICS"),
            dynamic_wetland: logical(doc, "DEF_USE_Dynamic_Wetland"),
            interception: integer(doc, "DEF_Interception_scheme"),
            medlyn: logical(doc, "DEF_USE_MEDLYNST"),
            wuest: logical(doc, "DEF_USE_WUEST"),
        }
    }

    fn waterbody(&self) -> bool {
        self.site_landtype == if self.usgs { 16 } else { 17 }
    }

    fn wetland(&self) -> bool {
        self.site_landtype == if self.usgs { 17 } else { 11 }
    }

    fn cropland(&self) -> bool {
        self.site_landtype == if self.usgs { 7 } else { 12 }
    }

    fn urban_land(&self) -> bool {
        self.site_landtype == if self.usgs { 1 } else { 13 }
    }

    fn glacier(&self) -> bool {
        self.site_landtype == if self.usgs { 24 } else { 15 }
    }

    fn natural_pft_land(&self) -> bool {
        !self.waterbody()
            && !self.wetland()
            && !self.urban_land()
            && !self.glacier()
            && !(self.crop && self.cropland())
    }

    fn biological_land(&self) -> bool {
        self.natural_pft_land() || (self.crop && self.cropland())
    }

    fn soil_hydrology(&self) -> bool {
        !self.single
            || (!self.glacier() && (!self.waterbody() || logical(self.doc, "DEF_USE_Dynamic_Lake")))
    }

    fn valid_landtype(&self) -> bool {
        (1..=if self.usgs { 24 } else { 17 }).contains(&self.site_landtype)
    }
}

fn simulation_stamp(doc: &colm_namelist::Document, prefix: &str) -> i64 {
    let int = |suffix: &str| integer(doc, &format!("{prefix}{suffix}"));
    civil_stamp(int("year"), int("month"), int("day"), int("sec"))
}

fn civil_stamp(y: i64, m: i64, d: i64, sec: i64) -> i64 {
    // Howard Hinnant days_from_civil; duplicated here to keep GUI free of chrono.
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    (era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy) * 86_400 + sec
}

fn active_path(
    doc: &colm_namelist::Document,
    case_dir: &std::path::Path,
    switch: &str,
    path: &str,
    want_dir: bool,
) -> Result<(), String> {
    if !matches!(doc.get(switch), Some(colm_namelist::Value::Bool(true))) {
        return Ok(());
    }
    let value = character(doc, path);
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("null") {
        return Err(format!("{switch} 已开启，请选择 {path}"));
    }
    let configured = std::path::Path::new(trimmed);
    let p = if configured.is_absolute() {
        configured.to_path_buf()
    } else {
        case_dir.join(configured)
    };
    let meta = std::fs::metadata(&p).map_err(|e| format!("{path}: {}: {e}", p.display()))?;
    if want_dir && !meta.is_dir() {
        return Err(format!("{path}: {} 不是目录", p.display()));
    }
    if !want_dir && !meta.is_file() {
        return Err(format!("{path}: {} 不是文件", p.display()));
    }
    Ok(())
}

fn is_expert_tuning_name(name: &str) -> bool {
    colm_case::land_cover::is_parameter(name)
        || name.starts_with("DEF_TUNING_")
        || name.starts_with("DEF_PH_")
        || name == "DEF_OZONE_KO3"
        || matches!(
            name,
            "DEF_DS_TEMP_LAPSE_RATE"
                | "DEF_DS_LONGWAVE_LAPSE_RATE"
                | "DEF_DS_LONGWAVE_LIMIT"
                | "DEF_DS_SHORTWAVE_LIMIT"
                | "DEF_DS_SHORTWAVE_SIMPLE_LIMIT"
        )
}

fn validate_real_value(name: &str, value: f64) -> Result<(), String> {
    if colm_case::land_cover::is_parameter(name) {
        return colm_case::land_cover::validate_override(name, value).map_err(|e| format!("{e:#}"));
    }
    let one_of = |names: &[&str]| names.contains(&name);
    if !value.is_finite() {
        return Err(format!("{name} 必须是有限数值"));
    }
    if one_of(&[
        "DEF_TUNING_ZLND",
        "DEF_TUNING_ZSNO",
        "DEF_TUNING_CSOILC",
        "DEF_TUNING_DEWMX",
        "DEF_TUNING_CAPR",
        "DEF_TUNING_TRSMX0",
        "DEF_TUNING_WETWATMAX",
        "DEF_TUNING_SOIL_ICE_IMPEDANCE",
        "DEF_TUNING_TOPMOD_DECAY",
        "DEF_TUNING_SNOW_COVER_EXPONENT",
        "DEF_TUNING_IRRIGATION_DURATION_SEC",
        "DEF_TUNING_IRRIGATION_MAX_DEPTH",
        "DEF_PH_CROOT_LATERAL_LENGTH",
        "DEF_PH_K_AXS",
        "DEF_PH_FROOT_CARBON",
        "DEF_PH_ROOT_RADIUS",
        "DEF_PH_ROOT_DENSITY",
        "DEF_PH_FROOT_LEAF",
        "DEF_PH_KRMAX",
    ]) && value <= 0.0
    {
        return Err(format!("{name} 必须大于 0"));
    }
    if name == "DEF_TUNING_PONDMX" && value < 0.0 {
        return Err(format!("{name} 必须不小于 0"));
    }
    if name == "DEF_TUNING_IRRIGATION_PONDMX" && value < 0.0 {
        return Err(format!("{name} 必须不小于 0"));
    }
    if one_of(&[
        "DEF_TUNING_CNFAC",
        "DEF_TUNING_SSI",
        "DEF_DS_LONGWAVE_LIMIT",
        "DEF_DS_SHORTWAVE_LIMIT",
        "DEF_DS_SHORTWAVE_SIMPLE_LIMIT",
    ]) && !(0.0..=1.0).contains(&value)
    {
        return Err(format!("{name} 必须在 0 到 1 之间"));
    }
    if name == "DEF_TUNING_WIMP" && !(0.0..1.0).contains(&value) {
        return Err(format!("{name} 必须大于等于 0 且小于 1"));
    }
    if matches!(
        name,
        "DEF_TUNING_SIMPLE_VIC_DS" | "DEF_TUNING_SIMPLE_VIC_WS"
    ) && !(value > 0.0 && value < 1.0)
    {
        return Err(format!("{name} 必须大于 0 且小于 1"));
    }
    if matches!(
        name,
        "DEF_TUNING_IRRIGATION_THRESHOLD_FRACTION" | "DEF_TUNING_IRRIGATION_SUPPLY_FRACTION"
    ) && !(0.0..=1.0).contains(&value)
    {
        return Err(format!("{name} 必须在 0 到 1 之间"));
    }
    if name == "DEF_TUNING_IRRIGATION_START_SEC" && !(0.0..86_400.0).contains(&value) {
        return Err(format!("{name} 必须在 0（含）到 86400（不含）秒之间"));
    }
    if name == "DEF_TUNING_IRRIGATION_DURATION_SEC" && value > 86_400.0 {
        return Err(format!("{name} 不能超过 86400 秒"));
    }
    if name == "DEF_TUNING_IRRIGATION_MIN_CPHASE" && !(0.0..=4.0).contains(&value) {
        return Err(format!("{name} 必须在 0 到 4 之间"));
    }
    if name == "DEF_TUNING_IRRIGATION_MAX_CPHASE" && !(value > 0.0 && value <= 4.0) {
        return Err(format!("{name} 必须大于 0 且不超过 4"));
    }
    if name == "DEF_TUNING_CROP_PLANTING_DAY"
        && value != 0.0
        && (!(1.0..=366.0).contains(&value) || value.fract() != 0.0)
    {
        return Err(format!("{name} 必须为 0，或 1 到 366 之间的整数"));
    }
    if one_of(&[
        "DEF_TUNING_SMPMAX",
        "DEF_TUNING_SMPMIN",
        "DEF_TUNING_SMPMAX_HR",
        "DEF_TUNING_SMPMIN_HR",
    ]) && value >= 0.0
    {
        return Err(format!("{name} 必须小于 0"));
    }
    if name == "DEF_OZONE_KO3" && value < 0.0 {
        return Err(format!("{name} 必须不小于 0"));
    }
    if one_of(&["DEF_DS_TEMP_LAPSE_RATE", "DEF_DS_LONGWAVE_LAPSE_RATE"]) && value < 0.0 {
        return Err(format!("{name} 必须不小于 0"));
    }
    Ok(())
}

fn validate_expert_tuning(
    doc: &colm_namelist::Document,
    names: impl IntoIterator<Item = String>,
) -> Result<(), String> {
    let names: std::collections::BTreeSet<String> = names
        .into_iter()
        .filter(|name| is_expert_tuning_name(name))
        .collect();
    for name in &names {
        validate_real_value(name, real(doc, name))?;
    }
    if names.contains("DEF_TUNING_SMPMAX") || names.contains("DEF_TUNING_SMPMIN") {
        let max = real(doc, "DEF_TUNING_SMPMAX");
        let min = real(doc, "DEF_TUNING_SMPMIN");
        if min >= max {
            return Err("DEF_TUNING_SMPMIN 必须小于 DEF_TUNING_SMPMAX".into());
        }
    }
    if names.contains("DEF_TUNING_SMPMAX_HR") || names.contains("DEF_TUNING_SMPMIN_HR") {
        let max = real(doc, "DEF_TUNING_SMPMAX_HR");
        let min = real(doc, "DEF_TUNING_SMPMIN_HR");
        if min >= max {
            return Err("DEF_TUNING_SMPMIN_HR 必须小于 DEF_TUNING_SMPMAX_HR".into());
        }
    }
    if names.contains("DEF_TUNING_SIMPLE_VIC_DS") || names.contains("DEF_TUNING_SIMPLE_VIC_WS") {
        let ds = real(doc, "DEF_TUNING_SIMPLE_VIC_DS");
        let ws = real(doc, "DEF_TUNING_SIMPLE_VIC_WS");
        if ds > ws {
            return Err("DEF_TUNING_SIMPLE_VIC_DS 必须小于等于 DEF_TUNING_SIMPLE_VIC_WS".into());
        }
    }
    if names.contains("DEF_TUNING_IRRIGATION_MIN_CPHASE")
        || names.contains("DEF_TUNING_IRRIGATION_MAX_CPHASE")
    {
        let min = real(doc, "DEF_TUNING_IRRIGATION_MIN_CPHASE");
        let max = real(doc, "DEF_TUNING_IRRIGATION_MAX_CPHASE");
        if min >= max {
            return Err("灌溉起始作物阶段必须小于结束阶段".into());
        }
    }
    if names.contains("DEF_TUNING_IRRIGATION_DURATION_SEC") {
        let duration = real(doc, "DEF_TUNING_IRRIGATION_DURATION_SEC");
        let timestep = real(doc, "DEF_simulation_time%timestep");
        if duration < timestep {
            return Err("灌溉持续时间不能短于一个模型时间步".into());
        }
    }
    Ok(())
}

fn validate_changed_fields(
    doc: &colm_namelist::Document,
    fields: &[FieldChange],
) -> Result<(), String> {
    for field in fields {
        match field.path.as_str() {
            "DEF_file_SoilInit" if !logical(doc, "DEF_USE_SoilInit") => {
                return Err("DEF_file_SoilInit 只对已启用土壤初始场的算例有效".into())
            }
            "DEF_file_SnowInit" if !logical(doc, "DEF_USE_SnowInit") => {
                return Err("DEF_file_SnowInit 只对已启用积雪初始场的算例有效".into())
            }
            "DEF_file_cn_init"
                if !logical(doc, "DEF_USE_CN_INIT") || !logical(doc, "DEF_USE_BGC") =>
            {
                return Err("DEF_file_cn_init 只对已启用 BGC 与 CN 初始场的算例有效".into())
            }
            "DEF_file_WaterTable"
                if !logical(doc, "DEF_USE_WaterTableInit") || logical(doc, "DEF_USE_SoilInit") =>
            {
                return Err("DEF_file_WaterTable 只对独立地下水位初始场有效".into())
            }
            "DEF_DS_HiresTopographyDataDir" if !logical(doc, "DEF_USE_Forcing_Downscaling") => {
                return Err("DEF_DS_HiresTopographyDataDir 只对完整地形强迫降尺度有效".into())
            }
            "DEF_file_Ozone"
                if !(logical(doc, "DEF_USE_OZONESTRESS") && logical(doc, "DEF_USE_OZONEDATA")) =>
            {
                return Err("DEF_file_Ozone 只对已启用臭氧胁迫和臭氧数据读取的算例有效".into())
            }
            "DEF_BALL_BERRY_GRADM" | "DEF_BALL_BERRY_BINTER"
                if logical(doc, "DEF_USE_MEDLYNST") || logical(doc, "DEF_USE_WUEST") =>
            {
                return Err("Ball–Berry 系数只对 Ball–Berry 气孔导度方案有效".into())
            }
            "DEF_MEDLYN_G1" | "DEF_MEDLYN_G0"
                if !logical(doc, "DEF_USE_MEDLYNST") || logical(doc, "DEF_USE_WUEST") =>
            {
                return Err("Medlyn 系数只对 Medlyn 气孔导度方案有效".into())
            }
            "DEF_WUE_LAMBDA"
                if !logical(doc, "DEF_USE_WUEST") || logical(doc, "DEF_USE_MEDLYNST") =>
            {
                return Err("lambda 只对水分利用效率（WUE）气孔导度方案有效".into())
            }
            _ => {}
        }
    }
    validate_expert_tuning(doc, fields.iter().map(|field| field.path.clone()))?;
    Ok(())
}

#[derive(Clone, Copy)]
struct KernelFacts {
    single: bool,
    usgs: bool,
    crop: bool,
}

fn kernel_facts(kernel_dir: Option<&str>) -> Result<Option<KernelFacts>, String> {
    let Some(dir) = kernel_dir.filter(|dir| !dir.trim().is_empty()) else {
        return Ok(None);
    };
    let kernel = colm_kernel::Kernel::open(std::path::Path::new(dir))
        .map_err(|error| format!("{error:#}"))?;
    let has = |name: &str| kernel.manifest.macros.iter().any(|macro_| macro_ == name);
    Ok(Some(KernelFacts {
        single: has("SinglePoint"),
        usgs: has("LULC_USGS"),
        crop: has("CROP"),
    }))
}

fn validate_runtime_contract(
    doc: &colm_namelist::Document,
    case_dir: &std::path::Path,
    kernel: Option<KernelFacts>,
) -> Result<(), String> {
    validate_expert_tuning(
        doc,
        colm_schema::all()
            .iter()
            .filter(|field| is_expert_tuning_name(field.name))
            .map(|field| field.name.to_string()),
    )?;
    let timestep = real(doc, "DEF_simulation_time%timestep");
    if !timestep.is_finite() || timestep <= 0.0 || timestep > 3600.0 {
        return Err("DEF_simulation_time%timestep 必须是大于 0 且不超过 3600 秒的有限数值".into());
    }

    let lct = logical(doc, "DEF_USE_LCT");
    let pft = logical(doc, "DEF_USE_PFT");
    let pc = logical(doc, "DEF_USE_PC");
    if [lct, pft, pc].into_iter().filter(|on| *on).count() != 1 {
        return Err("DEF_USE_LCT / DEF_USE_PFT / DEF_USE_PC 必须且只能开启一个".into());
    }

    let bgc = logical(doc, "DEF_USE_BGC");
    let tracer = logical(doc, "DEF_USE_TRACER");
    if logical(doc, "DEF_USE_MEDLYNST") && logical(doc, "DEF_USE_WUEST") {
        return Err("Medlyn 与 WUE 气孔导度方案不能同时开启".into());
    }
    let methane = character(doc, "DEF_TRACER_NAMES")
        .split(',')
        .any(|name| matches!(name.trim().to_ascii_uppercase().as_str(), "CH4" | "METHANE"));
    let urban = logical(doc, "DEF_URBAN_RUN");
    if urban
        && integer(doc, "DEF_URBAN_type_scheme") == 1
        && !ncar_urban_properties(doc, Some(case_dir)).is_some_and(|path| path.is_file())
    {
        return Err(
            "NCAR 城市分类需要 DEF_dir_rawdata/urban/NCAR_urban_properties.nc；当前数据只支持 LCZ"
                .into(),
        );
    }
    let single = kernel.map(|facts| facts.single).unwrap_or_else(|| {
        doc.get("SITE_fsitedata").is_some()
            || doc.get("SITE_lon_location").is_some()
            || doc.get("SITE_lat_location").is_some()
    });
    let usgs = kernel
        .map(|facts| facts.usgs)
        .unwrap_or_else(|| logical(doc, "DEF_USE_USGS"));
    let crop = kernel
        .map(|facts| facts.crop)
        .unwrap_or_else(|| logical(doc, "DEF_USE_CROP"));

    if bgc && !(pft || pc) {
        return Err("DEF_USE_BGC 需要 DEF_USE_PFT 或 DEF_USE_PC".into());
    }
    if crop && !bgc {
        return Err("当前 CROP 内核需要同时开启 DEF_USE_BGC".into());
    }
    if tracer {
        if urban {
            return Err("城市模式暂不支持 TRACER".into());
        }
        if logical(doc, "DEF_USE_Campbell_SOIL_MODEL")
            || !logical(doc, "DEF_USE_VariablySaturatedFlow")
        {
            return Err("TRACER 需要 van Genuchten/VariablySaturatedFlow 土壤水方案".into());
        }
        if methane && (!bgc || !(pft || pc)) {
            return Err("甲烷 TRACER 需要 BGC 且使用 PFT 或 PC 次网格".into());
        }
        if logical(doc, "DEF_USE_BIFURCATION") && integer(doc, "DEF_TRACER_NUM") > 0 {
            return Err("TRACER 不能与河道分汊 DEF_USE_BIFURCATION 同时开启".into());
        }
        let tracer_num = integer(doc, "DEF_TRACER_NUM");
        if !(0..=1000).contains(&tracer_num) {
            return Err("DEF_TRACER_NUM 必须在 0 到 1000 之间".into());
        }
        let relative_humidity = real(doc, "DEF_TRACER_CG_RELHUM_MAX");
        if !(relative_humidity > 0.0 && relative_humidity < 1.0) {
            return Err("DEF_TRACER_CG_RELHUM_MAX 必须严格位于 0 与 1 之间".into());
        }
        for name in [
            "DEF_TRACER_SNOWMELT_EQUILIBRATION",
            "DEF_TRACER_CANOPY_EQUILIBRATION",
        ] {
            let value = real(doc, name);
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(format!("{name} 必须在 0 到 1 之间"));
            }
        }
        for name in ["DEF_TRACER_SUBL_SKIN_MM", "DEF_TRACER_ICE_SUPERSAT_SLOPE"] {
            let value = real(doc, name);
            if !value.is_finite() || value < 0.0 {
                return Err(format!("{name} 必须为非负有限数值"));
            }
        }
        for name in [
            "DEF_TRACER_BALANCE_ABORT_NBAD",
            "DEF_TRACER_RESID_ABORT_NBAD",
            "DEF_TRACER_LULCC_ABORT_NBAD",
        ] {
            if integer(doc, name) < 0 {
                return Err(format!("{name} 必须为非负整数"));
            }
        }
    }
    if single && urban && bgc {
        return Err("纯城市 SinglePoint 当前不运行 BGC，请关闭 BGC 或改用自然/混合区域配置".into());
    }
    if logical(doc, "DEF_USE_LULCC") {
        if single {
            return Err("SinglePoint 当前不支持 LULCC".into());
        }
        if usgs || bgc {
            return Err("LULCC 当前不支持 USGS 或 BGC".into());
        }
    }

    active_path(
        doc,
        case_dir,
        "DEF_USE_SoilInit",
        "DEF_file_SoilInit",
        false,
    )?;
    active_path(
        doc,
        case_dir,
        "DEF_USE_SnowInit",
        "DEF_file_SnowInit",
        false,
    )?;
    active_path(doc, case_dir, "DEF_USE_CN_INIT", "DEF_file_cn_init", false)?;
    if !logical(doc, "DEF_USE_SoilInit") {
        active_path(
            doc,
            case_dir,
            "DEF_USE_WaterTableInit",
            "DEF_file_WaterTable",
            false,
        )?;
    }
    active_path(
        doc,
        case_dir,
        "DEF_USE_Forcing_Downscaling",
        "DEF_DS_HiresTopographyDataDir",
        true,
    )?;
    if logical(doc, "DEF_USE_OZONESTRESS") {
        active_path(doc, case_dir, "DEF_USE_OZONEDATA", "DEF_file_Ozone", false)?;
    }
    let runtime = character(doc, "DEF_dir_runtime");
    let runtime = runtime.trim();
    let runtime = (!runtime.is_empty() && !runtime.eq_ignore_ascii_case("null")).then(|| {
        let path = std::path::Path::new(runtime);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            case_dir.join(path)
        }
    });
    validate_bgc_runtime_dir(
        runtime.as_deref(),
        bgc,
        integer(doc, "DEF_NDEP_FREQUENCY"),
        logical(doc, "DEF_USE_NITRIF"),
        logical(doc, "DEF_USE_FIRE"),
    )?;
    if crop {
        for (name, label) in crop_runtime_files(
            real(doc, "DEF_TUNING_CROP_PLANTING_DAY"),
            logical(doc, "DEF_USE_FERT"),
            integer(doc, "DEF_FERT_SOURCE"),
            logical(doc, "DEF_USE_IRRIGATION"),
            integer(doc, "DEF_IRRIGATION_ALLOCATION"),
        ) {
            runtime_file(doc, case_dir, name, label)?;
        }
    }
    Ok(())
}

pub(crate) fn validate_bgc_runtime_dir(
    root: Option<&std::path::Path>,
    bgc: bool,
    ndep_frequency: i64,
    nitrif: bool,
    fire: bool,
) -> Result<(), String> {
    if !bgc {
        return Ok(());
    }
    let root =
        root.ok_or("BGC/甲烷算例需要运行时数据目录；请在“基本设定 / 文件与目录”选择 runtime。")?;
    let ndep = match ndep_frequency {
        1 => "ndep/fndep_colm_hist_simyr1849-2006_1.9x2.5_c100428.nc",
        2 => "ndep/fndep_colm_monthly.nc",
        _ => return Err("DEF_NDEP_FREQUENCY 只能选择年尺度（1）或月尺度（2）".into()),
    };
    let require = |name: &str, label: &str| {
        let file = root.join(name);
        file.is_file()
            .then_some(())
            .ok_or_else(|| format!("BGC/甲烷运行时目录缺少{label}：{}", file.display()))
    };
    require(ndep, "氮沉降数据")?;
    if nitrif {
        for family in ["CONC_O2_UNSAT", "O2_DECOMP_DEPTH_UNSAT"] {
            for layer in 1..=10 {
                require(
                    &format!("nitrif/{family}/{family}_l{layer:02}.nc"),
                    "硝化数据",
                )?;
            }
        }
    }
    if fire {
        for name in [
            "fire/abm_colm_double_fillcoast.nc",
            "fire/peatf_colm_360x720_c100428.nc",
            "fire/gdp_colm_360x720_c100428.nc",
            "fire/colmforc.Li_2017_HYDEv3.2_CMIP6_hdm_0.5x0.5_AVHRR_simyr1850-2016_c180202.nc",
            "fire/clmforc.Li_2012_climo1995-2011.T62.lnfm_Total_c140423.nc",
        ] {
            require(name, "火灾过程数据")?;
        }
    }
    Ok(())
}

pub(crate) fn crop_runtime_files(
    planting_day: f64,
    fertilisation: bool,
    fertilisation_source: i64,
    irrigation: bool,
    irrigation_allocation: i64,
) -> Vec<(&'static str, &'static str)> {
    let mut files = Vec::new();
    if planting_day <= 0.0 || fertilisation || irrigation {
        files.push((
            "crop/plantdt-colm-64cfts-rice2_fillcoast.nc",
            "CROP 播种日期",
        ));
    }
    if fertilisation {
        files.push((
            if fertilisation_source == 2 {
                "crop/fertilizer_2015soc.nc"
            } else {
                "crop/fertnitro_fillcoast.nc"
            },
            "CROP 施肥",
        ));
    }
    if irrigation {
        files.push(("crop/surfdata_irrigation_method_96x144.nc", "CROP 灌溉"));
        if irrigation_allocation == 3 {
            files.push(("crop/surfdata_irrigation_allocation.nc", "CROP 灌溉分配"));
        }
    }
    files
}

fn runtime_file(
    doc: &colm_namelist::Document,
    case_dir: &std::path::Path,
    name: &str,
    label: &str,
) -> Result<(), String> {
    let root = character(doc, "DEF_dir_runtime");
    let root = root.trim();
    if root.is_empty() || root.eq_ignore_ascii_case("null") {
        return Err(format!("{label} 已开启，请选择 DEF_dir_runtime"));
    }
    let root = std::path::Path::new(root);
    let file = if root.is_absolute() {
        root.join(name)
    } else {
        case_dir.join(root).join(name)
    };
    if !file.is_file() {
        return Err(format!("{label} 运行时目录缺少数据：{}", file.display()));
    }
    Ok(())
}

fn hidden(reason: &'static str) -> (FieldMode, Option<&'static str>, Vec<&'static str>) {
    (FieldMode::Hidden, Some(reason), Vec::new())
}

fn expert_tuning_runtime_state(
    name: &str,
    c: &VisibilityContext<'_>,
) -> Option<(FieldMode, Option<&'static str>, Vec<&'static str>)> {
    let one_of = |names: &[&str]| names.contains(&name);
    if name.starts_with("DEF_LC_") && colm_case::land_cover::is_parameter(name) {
        if !(c.single && c.lct && c.valid_landtype() && c.biological_land()) {
            return Some(hidden(
                "仅单点 LCT 植被地表使用；PFT/PC 参数来自 MOD_Const_PFT",
            ));
        }
        if colm_case::land_cover::needs_plant_hydraulics(name) && !c.plant_hydraulics {
            return Some(hidden("需要先启用植物水力过程"));
        }
        if name == "DEF_LC_C3C4" {
            return Some((FieldMode::Editable, None, vec!["0", "1"]));
        }
    }
    if c.single
        && !c.biological_land()
        && one_of(&["DEF_TUNING_CSOILC", "DEF_TUNING_DEWMX", "DEF_TUNING_TRSMX0"])
        && !c.urban
    {
        return Some(hidden("仅植被或城市地表使用"));
    }
    if name == "DEF_TUNING_WETWATMAX" && !(c.wetland() || c.dynamic_wetland) {
        return Some(hidden("仅湿地或动态湿地过程使用"));
    }
    if matches!(
        name,
        "DEF_TUNING_SMPMAX" | "DEF_TUNING_SIMPLE_VIC_DS" | "DEF_TUNING_SIMPLE_VIC_WS"
    ) {
        return Some(hidden("当前内核没有活动计算路径使用此字段"));
    }
    if matches!(
        name,
        "DEF_TUNING_SMPMIN" | "DEF_TUNING_SOIL_ICE_IMPEDANCE" | "DEF_TUNING_TOPMOD_DECAY"
    ) && !c.soil_hydrology()
    {
        return Some(hidden("当前单点地表不运行土壤水分过程"));
    }
    if name == "DEF_TUNING_TOPMOD_DECAY" && c.runoff != 0 {
        return Some(hidden("仅 TOPMODEL 产流方案使用"));
    }
    if name.starts_with("DEF_TUNING_IRRIGATION_")
        && !(c.crop && (!c.single || c.cropland()) && logical(c.doc, "DEF_USE_IRRIGATION"))
    {
        return Some(hidden("需要 CROP 农田并启用灌溉"));
    }
    if name == "DEF_TUNING_CROP_PLANTING_DAY" && !(c.single && c.crop && c.cropland()) {
        return Some(hidden("仅单点 CROP 农田使用"));
    }
    if one_of(&["DEF_TUNING_SMPMAX_HR", "DEF_TUNING_SMPMIN_HR"]) && !c.bgc {
        return Some(hidden("需要 BGC"));
    }
    if name.starts_with("DEF_PH_") && !(c.plant_hydraulics && c.biological_land()) {
        return Some(hidden("需要植被地表并启用植物水力过程"));
    }
    if name == "DEF_OZONE_KO3" && !(c.ozone_stress && c.biological_land()) {
        return Some(hidden("需要植被地表并启用臭氧胁迫"));
    }
    if one_of(&[
        "DEF_DS_TEMP_LAPSE_RATE",
        "DEF_DS_LONGWAVE_LAPSE_RATE",
        "DEF_DS_LONGWAVE_LIMIT",
    ]) && !(c.downscale || c.downscale_simple)
    {
        return Some(hidden("需要先选择一种强迫场降尺度模式"));
    }
    if name == "DEF_DS_LONGWAVE_LAPSE_RATE"
        && character(c.doc, "DEF_DS_longwave_adjust_scheme").eq_ignore_ascii_case("I")
    {
        return Some(hidden("仅长波降尺度方案 II 使用"));
    }
    if name == "DEF_DS_LONGWAVE_LAPSE_RATE" && c.single && !c.glacier() {
        return Some(hidden("方案 II 仅在冰川地表使用此长波递减率"));
    }
    if name == "DEF_DS_SHORTWAVE_LIMIT" && !c.downscale {
        return Some(hidden("仅完整地形强迫降尺度使用"));
    }
    if name == "DEF_DS_SHORTWAVE_SIMPLE_LIMIT" && !c.downscale_simple {
        return Some(hidden("仅简化地形强迫降尺度使用"));
    }
    None
}

fn field_runtime_state(
    field: &colm_schema::Field,
    c: &VisibilityContext<'_>,
) -> (FieldMode, Option<&'static str>, Vec<&'static str>) {
    let name = field.name;
    let one_of = |names: &[&str]| names.contains(&name);

    if let Some(state) = expert_tuning_runtime_state(name, c) {
        return state;
    }

    if !field_is_relevant(field, c.have) {
        return hidden("当前内核未编入这个功能");
    }

    if name == "DEF_URBAN_geom_data" {
        return hidden("CoLM 当前只读取并广播此字段，没有任何计算路径使用它");
    }

    // SinglePoint 在读写完单点 surface data 后直接返回；这些字段只服务于
    // 区域聚合、分块或区域历史输出，继续展示会制造“配置已生效”的假象。
    if c.single
        && one_of(&[
            "USE_srfdata_from_larger_region",
            "DEF_dir_existing_srfdata",
            "USE_srfdata_from_3D_gridded_data",
            "DEF_SOLO_PFT",
            "DEF_FAST_PC",
            "DEF_SUBGRID_SCHEME",
            "DEF_LANDONLY",
            "DEF_USE_DOMINANT_PATCHTYPE",
            "DEF_USE_SOILPAR_UPS_FIT",
            "USE_zip_for_aggregation",
            "DEF_Srfdata_CompressLevel",
            "DEF_Forcing_Interp_Method",
            "DEF_TOPMOD_method",
            "DEF_HISTORY_IN_VECTOR",
            "DEF_HIST_grid_as_forcing",
            "DEF_HIST_lon_res",
            "DEF_HIST_lat_res",
            "DEF_HIST_mode",
            "DEF_HIST_WriteBack",
            "DEF_URBAN_ONLY",
            "DEF_USE_SrfdataDiag",
        ])
    {
        return hidden("SinglePoint 执行路径不会使用这个字段");
    }
    if c.single && name == "DEF_HIST_CompressLevel" && !c.tracer {
        return hidden("SinglePoint 普通历史文件不使用此压缩设置");
    }
    if name == "DEF_TOPMOD_method" && c.runoff != 0 {
        return hidden("仅 TOPMODEL 产流方案使用");
    }

    if one_of(&[
        "DEF_HighResSoil",
        "DEF_HighResVeg",
        "DEF_PROSPECT",
        "DEF_HighResUrban_albedo",
    ]) && !c.have.contains("HYPERSPECTRAL")
    {
        return hidden("当前内核未启用 HYPERSPECTRAL");
    }
    if name == "DEF_HighResUrban_albedo" && !c.urban {
        return hidden("仅城市高光谱模式使用");
    }

    // 站点身份在建例时逐站点写入。批量参数页若允许修改，会把多个站点的
    // 文件、坐标或地类悄悄统一成同一个值，因此 SinglePoint 一律不展示。
    // 自然站的地类来自站点 NetCDF 的 IGBP_classification；城市站固定为 13。
    if c.single
        && one_of(&[
            "SITE_fsitedata",
            "SITE_lon_location",
            "SITE_lat_location",
            "SITE_landtype",
            "USE_SITE_landtype",
        ])
    {
        return hidden("由选择站点并建算例按站点自动确定");
    }
    // 其余站点数据字段跟随实际分类。湖泊、湿地、作物和 PFT 比例只对
    // 对应地表有意义。
    if name == "USE_SITE_pctpfts" && (!(c.pft || c.pc) || !c.natural_pft_land()) {
        return hidden("仅自然地表的 PFT/PC 次网格使用");
    }
    if name == "USE_SITE_pctcrop" && !(c.crop && c.cropland()) {
        return hidden("仅 CROP 内核的作物地表使用");
    }
    if name == "USE_SITE_lakedepth" && !c.waterbody() {
        return hidden("仅水体站点使用");
    }
    if name == "USE_SITE_dbedrock" && !logical(c.doc, "DEF_USE_BEDROCK") {
        return hidden("需要先启用基岩过程");
    }

    if name == "DEF_PC_CROP_SPLIT" && (!c.pc || (c.single && !c.biological_land())) {
        return hidden("仅 PC 次网格使用");
    }
    // 单点站点优先读取 site.nc 里的 LAI；这时原始 LAI 数据的年份与时间分辨率
    // 不参与计算。关掉 USE_SITE_LAI 后，才显示对应的回退数据设置。
    if c.single
        && (c.site_lai || c.lai_feedback)
        && one_of(&[
            "DEF_LC_YEAR",
            "DEF_LAI_START_YEAR",
            "DEF_LAI_END_YEAR",
            "DEF_LAI_MONTHLY",
            "DEF_LAI_CHANGE_YEARLY",
        ])
    {
        return hidden(if c.lai_feedback {
            "LAI 由 BGC 叶碳反馈计算"
        } else {
            "当前按站点文件读取 LAI"
        });
    }
    if one_of(&["DEF_LAI_START_YEAR", "DEF_LAI_END_YEAR"]) && !c.lai_change_yearly {
        return hidden("需要先启用叶面积指数逐年变化");
    }
    if c.single && name == "DEF_LC_YEAR" && c.lai_change_yearly {
        return hidden("逐年 LAI 使用模拟年份，不使用单一地表数据年份");
    }
    if name == "DEF_LAI_MONTHLY" && (c.pft || c.pc || c.lulcc || c.urban) {
        return hidden("当前次网格会自动使用月尺度 LAI");
    }
    if name == "DEF_USE_LAIFEEDBACK" && !c.bgc {
        return hidden("需要 BGC");
    }
    if name == "DEF_LULCC_SCHEME" && !c.lulcc {
        return hidden("需要先启用 LULCC");
    }
    if name == "USE_SITE_soilreflectance" && c.soil_reflectance_scheme != 2 {
        return hidden("当前方案按地表覆盖类型估算土壤反照率");
    }

    // 初始场文件是父开关的子项。SoilInit 同时打开时，CoLM 明确忽略独立的
    // water-table 文件；CN 初始化则只在 BGC 下存在。
    if name == "DEF_file_SoilInit" && !c.soil_init {
        return hidden("需要先启用土壤初始场");
    }
    if name == "DEF_file_SnowInit" && !c.snow_init {
        return hidden("需要先启用积雪初始场");
    }
    if one_of(&["DEF_USE_CN_INIT", "DEF_file_cn_init"]) && !c.bgc {
        return hidden("需要 BGC");
    }
    if name == "DEF_file_cn_init" && !c.cn_init {
        return hidden("需要先启用 CN 初始场");
    }
    if name == "DEF_file_WaterTable" && (!c.water_table_init || c.soil_init) {
        return hidden("仅独立地下水位初始化使用");
    }
    if name == "DEF_USE_WaterTableInit" && c.soil_init {
        return hidden("土壤初始场已经包含地下水位初值");
    }

    // 完整与简单降尺度共用数组，不能同时开启。方案 III 依赖仓库中不存在的
    // Python MPI server launcher；内核会 fail-fast，界面只暴露可运行的 I/II。
    if name == "DEF_DS_HiresTopographyDataDir" && !c.downscale {
        return hidden("仅完整地形强迫降尺度需要外部高分辨率地形目录");
    }
    if one_of(&[
        "DEF_DS_precipitation_adjust_scheme",
        "DEF_DS_longwave_adjust_scheme",
    ]) && !(c.downscale || c.downscale_simple)
    {
        return hidden("需要先选择一种强迫场降尺度模式");
    }
    if name == "DEF_USE_Forcing_Downscaling" && c.downscale_simple {
        return (
            FieldMode::Editable,
            Some("简单降尺度已开启；先关闭它才能开启完整降尺度"),
            vec![".false."],
        );
    }
    if name == "DEF_USE_Forcing_Downscaling_Simple" && c.downscale {
        return (
            FieldMode::Editable,
            Some("完整降尺度已开启；先关闭它才能开启简单降尺度"),
            vec![".false."],
        );
    }
    if name == "DEF_DS_precipitation_adjust_scheme" {
        return (FieldMode::Editable, None, vec!["I", "II"]);
    }
    // 站点工作流生成的 forcing.nml 固定使用 POINT 数据集。POINT 的文件名
    // 不含年份，ClimForcing 只把年份替换为 `clim`，因此打开也不会改变读入。
    if name == "DEF_USE_ClimForcing_for_Spinup" && c.single {
        return hidden("站点 POINT 强迫场始终循环同一文件，此开关不会改变读入");
    }

    // 完整扩展截留模块由 extend_interception 宏选择整套源文件。当前随软件
    // 发布的所有内核都带该宏，因此 1..8 都真实可用；若用户安装了不带宏的
    // 外部内核，回退模块只调用 CoLM2014，实现上只有方案 1。
    if name == "DEF_Interception_scheme" {
        return if c.have.contains("extend_interception") {
            (
                FieldMode::Editable,
                Some("当前内核已编入扩展截留模块，8 种方案均有实际计算路径"),
                vec!["1", "2", "3", "4", "5", "6", "7", "8"],
            )
        } else {
            (
                FieldMode::Editable,
                Some("当前内核未编入 extend_interception，只能使用 CoLM2014 方案"),
                vec!["1"],
            )
        };
    }

    if name == "DEF_MATSIRO_CWCAP_SCALE" && c.interception != 5 {
        return hidden("仅 MATSIRO 截留方案使用");
    }
    if name == "DEF_RSS_SCHEME" && c.lct && c.vg {
        return hidden("LCT + van Genuchten 下 CoLM 会自动关闭土壤表面阻抗");
    }
    if name == "DEF_USE_VariablySaturatedFlow" && c.vg {
        return hidden("van Genuchten 下 CoLM 会自动启用 VSF");
    }
    if one_of(&["DEF_VIC_OPT", "DEF_file_VIC_para", "DEF_file_VIC_OPT"]) && c.runoff != 1 {
        return hidden("仅 VIC runoff 使用");
    }
    if one_of(&["DEF_file_VIC_para", "DEF_file_VIC_OPT"]) {
        return hidden("CoLM 会从运行时目录派生 VIC 参数文件");
    }
    if name == "DEF_USE_Dynamic_Lake" && !c.waterbody() {
        return hidden("仅水体站点使用");
    }
    if name == "DEF_USE_Dynamic_Wetland" && !c.wetland() {
        return hidden("仅湿地站点使用");
    }

    // BGC/CROP 子过程必须跟随真实运行时/编译期能力，不能依赖整个页面的粗粒度
    // 开关。独立的积雪、臭氧和植被物理选项仍可在 BGC 关闭时使用。
    if one_of(&[
        "DEF_NDEP_FREQUENCY",
        "DEF_USE_NOSTRESSNITROGEN",
        "DEF_USE_SASU",
        "DEF_USE_DiagMatrix",
        "DEF_USE_PN",
        "DEF_USE_NITRIF",
        "DEF_USE_FIRE",
    ]) && (!c.bgc || (c.single && !c.biological_land()))
    {
        return hidden("需要 BGC");
    }
    if one_of(&[
        "DEF_USE_FERT",
        "DEF_FERT_SOURCE",
        "DEF_USE_CNSOYFIXN",
        "DEF_USE_IRRIGATION",
        "DEF_IRRIGATION_ALLOCATION",
    ]) && (!c.crop || (c.single && !c.cropland()))
    {
        return hidden("当前内核未启用 CROP");
    }
    if name == "DEF_USE_CROP" && !c.crop {
        return hidden("当前内核未启用 CROP");
    }
    if c.single
        && !c.biological_land()
        && one_of(&[
            "DEF_VEG_SNOW",
            "DEF_USE_OZONESTRESS",
            "DEF_USE_OZONEDATA",
            "DEF_file_Ozone",
            "DEF_USE_MEDLYNST",
            "DEF_USE_WUEST",
            "DEF_BALL_BERRY_GRADM",
            "DEF_BALL_BERRY_BINTER",
            "DEF_MEDLYN_G1",
            "DEF_MEDLYN_G0",
            "DEF_WUE_LAMBDA",
        ])
    {
        return hidden("当前站点不是植被地表，不会使用叶片或冠层过程");
    }
    if name == "DEF_USE_OZONEDATA" && !c.ozone_stress {
        return hidden("需要先启用臭氧胁迫");
    }
    if name == "DEF_file_Ozone" && (!c.ozone_stress || !c.ozone_data) {
        return hidden("仅从文件读取臭氧数据时使用");
    }
    if c.medlyn
        && c.wuest
        && one_of(&[
            "DEF_BALL_BERRY_GRADM",
            "DEF_BALL_BERRY_BINTER",
            "DEF_MEDLYN_G1",
            "DEF_MEDLYN_G0",
            "DEF_WUE_LAMBDA",
        ])
    {
        return hidden("请先解决 Medlyn 与 WUE 同时开启的方案冲突");
    }
    if one_of(&["DEF_BALL_BERRY_GRADM", "DEF_BALL_BERRY_BINTER"]) && (c.medlyn || c.wuest) {
        return hidden("仅 Ball–Berry 气孔导度方案使用");
    }
    if one_of(&["DEF_MEDLYN_G1", "DEF_MEDLYN_G0"]) && !c.medlyn {
        return hidden("仅 Medlyn 气孔导度方案使用");
    }
    if name == "DEF_WUE_LAMBDA" && !c.wuest {
        return hidden("仅水分利用效率（WUE）气孔导度方案使用");
    }
    if one_of(&["DEF_file_snowoptics", "DEF_file_snowaging"]) {
        return hidden("CoLM 会从运行时目录派生 SNICAR 数据文件");
    }
    if one_of(&["DEF_Aerosol_Readin", "DEF_Aerosol_Clim"]) && !c.snicar {
        return hidden("需要先启用 SNICAR");
    }
    if name == "DEF_Aerosol_Clim" && !c.aerosol_readin {
        return hidden("需要先读取气溶胶数据");
    }
    if name == "DEF_USE_MEDLYNST" && c.wuest {
        return (
            FieldMode::Editable,
            Some("WUEST 已开启；两种气孔方案不能同时开启"),
            vec![".false."],
        );
    }
    if name == "DEF_USE_WUEST" && c.medlyn {
        return (
            FieldMode::Editable,
            Some("Medlyn 已开启；两种气孔方案不能同时开启"),
            vec![".false."],
        );
    }

    if field_section(name, field.group) == Some("城市") && !c.urban {
        return hidden("需要先启用城市模型");
    }
    if name == "DEF_URBAN_type_scheme" && !c.ncar_urban {
        return (
            FieldMode::Editable,
            Some("当前 rawdata 未提供 NCAR 城市属性，只能使用 LCZ"),
            vec!["2"],
        );
    }
    if c.urban
        && one_of(&[
            "DEF_USE_WUEST",
            "DEF_USE_SUPERCOOL_WATER",
            "DEF_USE_PLANTHYDRAULICS",
            "DEF_USE_OZONESTRESS",
            "DEF_USE_OZONEDATA",
            "DEF_SPLIT_SOILSNOW",
        ])
    {
        return hidden("城市模式会自动关闭这个过程");
    }
    if field_section(name, field.group) == Some("示踪剂") && !c.tracer {
        return hidden("需要先启用 TRACER");
    }
    if c.tracer
        && !c.isotope_tracer
        && one_of(&[
            "DEF_TRACER_USE_FRACTIONATION",
            "DEF_TRACER_KINETIC_SCHEME",
            "DEF_TRACER_ICE_SUPERSAT_SLOPE",
            "DEF_TRACER_CG_RELHUM_MAX",
            "DEF_TRACER_OPEN_WATER_KINETIC",
            "DEF_TRACER_SUBL_SKIN_MM",
            "DEF_TRACER_SOIL_KINETIC",
            "DEF_TRACER_SOIL_DIFFUSION",
            "DEF_TRACER_SOIL_VAPOR_DIFFUSION",
            "DEF_TRACER_CANOPY_EQUILIBRATION",
            "DEF_TRACER_SNOWMELT_EQUILIBRATION",
            "DEF_TRACER_NSS_LEAF_WATER_PER_LAI",
            "DEF_TRACER_NSS_LEAF_PATH_LENGTH",
            "DEF_TRACER_NSS_LEAF_RB",
            "DEF_TRACER_USE_SOIL_INIT",
            "DEF_TRACER_SOIL_INIT_FILE",
            "DEF_TRACER_SOIL_INIT_VARS",
        ])
    {
        return hidden("当前选择的是气体示踪，不使用水同位素参数");
    }

    if field.group.is_none() {
        return (
            FieldMode::Disabled,
            Some("由内核或其他路径自动派生，只读显示"),
            Vec::new(),
        );
    }

    (FieldMode::Editable, None, Vec::new())
}

fn field_states_for_at(
    text: &str,
    have: &std::collections::BTreeSet<&str>,
    case_dir: Option<&std::path::Path>,
) -> Result<Vec<FieldState>, String> {
    let doc = colm_namelist::parse(text).map_err(|e| format!("{e:#}"))?;
    let context = VisibilityContext::new_at(&doc, have, case_dir);
    colm_schema::all()
        .iter()
        .map(|field| {
            let (mode, reason, allowed_values) = field_runtime_state(field, &context);
            let context_default = if mode != FieldMode::Hidden
                && context.single
                && context.lct
                && context.valid_landtype()
                && context.biological_land()
                && colm_case::land_cover::is_parameter(field.name)
            {
                colm_case::land_cover::default_literal(
                    field.name,
                    context.usgs,
                    context.site_landtype,
                )
                .map_err(|error| format!("{}: {error:#}", field.name))?
            } else {
                None
            };
            let override_value = doc.get(field.name).map(ToString::to_string);
            let built_in_default = context_default
                .clone()
                .or_else(|| Some(default_literal(field.default)));
            let effective_value = override_value.clone().or_else(|| built_in_default.clone());
            let is_land_cover = context.single
                && context.lct
                && context.valid_landtype()
                && colm_case::land_cover::is_parameter(field.name);
            let scope_label = is_land_cover.then(|| {
                format!(
                    "{}-{}",
                    if context.usgs { "USGS" } else { "IGBP" },
                    context.site_landtype
                )
            });
            let provenance = if override_value.is_some() {
                "case.nml"
            } else if is_land_cover {
                "MOD_Const_LC.F90"
            } else {
                "MOD_Namelist.F90"
            };
            Ok(FieldState {
                name: field.name.to_string(),
                mode,
                reason,
                allowed_values,
                mixed: false,
                context_default,
                default_mixed: false,
                scope_label,
                built_in_default,
                override_value,
                effective_value,
                provenance: provenance.into(),
                override_mixed: false,
                effective_mixed: false,
            })
        })
        .collect()
}

fn merge_field_states(groups: &[Vec<FieldState>]) -> Vec<FieldState> {
    let Some(first) = groups.first() else {
        return Vec::new();
    };
    first
        .iter()
        .enumerate()
        .map(|(index, template)| {
            let each: Vec<&FieldState> = groups.iter().map(|group| &group[index]).collect();
            debug_assert!(each.iter().all(|state| state.name == template.name));
            let visible: Vec<&FieldState> = each
                .iter()
                .copied()
                .filter(|state| state.mode != FieldMode::Hidden)
                .collect();
            let mut mode = if visible.is_empty() {
                FieldMode::Hidden
            } else if visible
                .iter()
                .any(|state| state.mode == FieldMode::Editable)
            {
                FieldMode::Editable
            } else {
                FieldMode::Disabled
            };

            // 空 allowed_values 表示“使用 schema 的完整集合”，因此它是交集运算
            // 的全集，不参与收窄；有多个非空约束时取交集。
            let mut constraints = visible
                .iter()
                .filter(|state| !state.allowed_values.is_empty())
                .map(|state| state.allowed_values.as_slice());
            let (allowed_values, had_constraints) = constraints.next().map_or_else(
                || (Vec::new(), false),
                |head| {
                    let rest: Vec<_> = constraints.collect();
                    (
                        head.iter()
                            .copied()
                            .filter(|value| rest.iter().all(|values| values.contains(value)))
                            .collect(),
                        true,
                    )
                },
            );
            let no_common_value = had_constraints && allowed_values.is_empty();
            if no_common_value && mode != FieldMode::Hidden {
                // 空 allowed_values 平时表示“schema 全部取值均可”。这里却是多个
                // 非空约束的交集为空，不能把它误解释为无限制；批量编辑必须锁住。
                mode = FieldMode::Disabled;
            }
            let mixed = no_common_value
                || each.iter().any(|state| {
                    state.mode != template.mode || state.allowed_values != template.allowed_values
                });
            let context_default = visible
                .first()
                .and_then(|state| state.context_default.clone());
            let default_mixed = visible
                .iter()
                .any(|state| state.context_default != context_default);
            let scope_label = visible.first().and_then(|state| state.scope_label.clone());
            let scope_mixed = visible.iter().any(|state| state.scope_label != scope_label);
            let override_value = visible
                .first()
                .and_then(|state| state.override_value.clone());
            let override_mixed = visible
                .iter()
                .any(|state| state.override_value != override_value);
            let effective_value = visible
                .first()
                .and_then(|state| state.effective_value.clone());
            let effective_mixed = visible
                .iter()
                .any(|state| state.effective_value != effective_value);
            let provenance = visible
                .first()
                .map(|state| state.provenance.clone())
                .unwrap_or_else(|| template.provenance.clone());
            let provenance_mixed = visible.iter().any(|state| state.provenance != provenance);
            if mixed
                && matches!(
                    template.name.as_str(),
                    "DEF_BALL_BERRY_GRADM"
                        | "DEF_BALL_BERRY_BINTER"
                        | "DEF_MEDLYN_G1"
                        | "DEF_MEDLYN_G0"
                        | "DEF_WUE_LAMBDA"
                )
            {
                mode = FieldMode::Hidden;
            }
            let reason = if no_common_value {
                Some("所选算例对这个字段没有共同合法值；请缩小批量范围后分别配置")
            } else if mixed {
                Some("所选算例的父开关或站点类型不同；此字段仅对其中一部分算例有效")
            } else {
                template.reason
            };
            FieldState {
                name: template.name.clone(),
                mode,
                reason,
                allowed_values,
                mixed,
                context_default: (!default_mixed).then_some(context_default).flatten(),
                default_mixed,
                scope_label: if scope_mixed {
                    Some("mixed".into())
                } else {
                    scope_label
                },
                built_in_default: (!default_mixed)
                    .then(|| {
                        visible
                            .first()
                            .and_then(|state| state.built_in_default.clone())
                    })
                    .flatten(),
                override_value: (!override_mixed).then_some(override_value).flatten(),
                effective_value: (!effective_mixed).then_some(effective_value).flatten(),
                provenance: if provenance_mixed {
                    "mixed".into()
                } else {
                    provenance
                },
                override_mixed,
                effective_mixed,
            }
        })
        .collect()
}

/// 批量编辑时按全部算例合并状态：只有全部无效才隐藏；任一算例有效就显示，
/// 同时用 `mixed` 标出条件差异。这样不会因代表算例 BGC=false 而把另一个
/// BGC=true 算例的子字段整个藏掉。
#[tauri::command]
pub fn field_states_batch(
    dirs: Vec<String>,
    kernel_dir: String,
) -> Result<Vec<FieldState>, String> {
    if dirs.is_empty() {
        return Err("没有可配置的算例".into());
    }
    let kernel = colm_kernel::Kernel::open(std::path::Path::new(&kernel_dir))
        .map_err(|e| format!("{e:#}"))?;
    let have: std::collections::BTreeSet<&str> =
        kernel.manifest.macros.iter().map(String::as_str).collect();
    let all = read_all(&dirs)?;
    let groups: Vec<Vec<FieldState>> = all
        .iter()
        .map(|(dir, text)| {
            field_states_for_at(text, &have, Some(std::path::Path::new(dir)))
                .map_err(|e| format!("{dir}: {e}"))
        })
        .collect::<Result<_, _>>()?;
    Ok(merge_field_states(&groups))
}

#[derive(Debug, Serialize)]
pub struct LandCoverContext {
    pub dir: String,
    pub scheme: &'static str,
    pub class_index: u8,
}

/// Return each case's actual LCT identity; PFT/PC cases are omitted rather than
/// being assigned a fabricated global land-cover class.
#[tauri::command]
pub fn land_cover_contexts(
    dirs: Vec<String>,
    kernel_dir: String,
) -> Result<Vec<LandCoverContext>, String> {
    let kernel = colm_kernel::Kernel::open(std::path::Path::new(&kernel_dir))
        .map_err(|error| format!("{error:#}"))?;
    let have = kernel
        .manifest
        .macros
        .iter()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    read_all(&dirs)?
        .into_iter()
        .filter_map(|(dir, text)| {
            let doc = match colm_namelist::parse(&text) {
                Ok(doc) => doc,
                Err(error) => return Some(Err(format!("{dir}: {error:#}"))),
            };
            let context = VisibilityContext::new_at(&doc, &have, Some(std::path::Path::new(&dir)));
            (context.lct && context.valid_landtype()).then_some(Ok(LandCoverContext {
                dir,
                scheme: if context.usgs { "USGS" } else { "IGBP" },
                class_index: context.site_landtype as u8,
            }))
        })
        .collect()
}

/// 一份 namelist 文本里 `colm-schema` 不认识的字段。
///
/// 不是装饰：上游**自己发布的**单点示例 `run/examples/SiteSYSUAtmos_IGBP_VG.nml`
/// 就设了 `USE_SITE_topostd` 与 `USE_SITE_BVIC` 两个已从 `MOD_Namelist.F90`
/// 删掉的字段，CoLM 读到会 `Cannot match namelist object name` 然后中止。
/// 界面该在开跑前点名它们，而不是让用户对着那句报错发呆。
#[tauri::command]
pub fn unknown_fields(text: String) -> Result<Vec<String>, String> {
    let doc = colm_namelist::parse(&text).map_err(|e| format!("{e:#}"))?;
    Ok(doc
        .paths()
        .into_iter()
        .filter(|p| colm_schema::find(p).is_none() && !colm_case::pft::is_override_path(p))
        .collect())
}

#[derive(Debug, Serialize)]
pub struct PftParameterState {
    pub name: &'static str,
    pub label_zh: &'static str,
    pub label_en: &'static str,
    pub group_zh: &'static str,
    pub group_en: &'static str,
    pub unit: Option<&'static str>,
    pub kind: &'static str,
    pub default: String,
    pub default_mixed: bool,
    pub value: Option<String>,
    pub mixed: bool,
    pub allowed_values: Vec<&'static str>,
    pub scope_kind: &'static str,
    pub scope_label: String,
    pub normal_pft_default: Option<String>,
    pub effective_value: String,
    pub provenance: &'static str,
}

fn pft_parameter_applies(
    meta: &colm_case::pft::ParameterMeta,
    context: &VisibilityContext<'_>,
    pft_type: u8,
) -> bool {
    use colm_case::pft::Condition;
    if pft_type == 0 {
        return false;
    }
    let process_applies = match meta.condition {
        Condition::Always => true,
        Condition::BallBerry => !context.medlyn && !context.wuest,
        Condition::Medlyn => context.medlyn && !context.wuest,
        Condition::Wue => context.wuest && !context.medlyn,
        Condition::PlantHydraulics => context.plant_hydraulics,
        Condition::Bgc => context.bgc,
        Condition::Fire => context.bgc && logical(context.doc, "DEF_USE_FIRE"),
        Condition::Crop => context.crop && context.bgc && pft_type >= 15,
    };
    process_applies
        && match meta.name {
            // A case-wide override wins inside MOD_AssimStomataConductance.
            // Do not offer a per-PFT value that the running model will ignore.
            "DEF_PFT_GRADM" => real(context.doc, "DEF_BALL_BERRY_GRADM") <= 1.6,
            "DEF_PFT_BINTER" => real(context.doc, "DEF_BALL_BERRY_BINTER") < 0.0,
            "DEF_PFT_G1" => real(context.doc, "DEF_MEDLYN_G1") < 0.0,
            "DEF_PFT_G0" => real(context.doc, "DEF_MEDLYN_G0") < 0.0,
            "DEF_PFT_LAMBDA" => real(context.doc, "DEF_WUE_LAMBDA") <= 0.0,
            "DEF_PFT_LIVEWDCN" | "DEF_PFT_DEADWDCN" | "DEF_PFT_CROOT_STEM" | "DEF_PFT_FLIVEWD" => {
                (1..=11).contains(&pft_type) || pft_type >= 15
            }
            "DEF_PFT_STEM_LEAF" => (1..=11).contains(&pft_type),
            "DEF_PFT_MANURE" => {
                logical(context.doc, "DEF_USE_FERT") && integer(context.doc, "DEF_FERT_SOURCE") == 1
            }
            _ => true,
        }
}

fn pft_parameter_has_default(
    meta: &colm_case::pft::ParameterMeta,
    context: &VisibilityContext<'_>,
    pft_type: u8,
) -> Result<bool, String> {
    let value = colm_case::pft::default_value(
        meta.name,
        pft_type,
        logical(context.doc, "DEF_USE_Campbell_SOIL_MODEL"),
        context.pc,
    )
    .map_err(|error| format!("{}: {error:#}", meta.name))?;
    Ok(value.is_some_and(|value| {
        // CoLM's crop tables use exactly -999/-999.9 for unavailable CFT entries;
        // large negative hydraulic potentials are real defaults and must remain visible.
        let unavailable = value == -999.0 || (value + 999.9).abs() < 1e-9;
        !unavailable && colm_case::pft::validate_override(meta.name, value).is_ok()
    }))
}

fn pft_override_path(name: &str, pft_type: u8) -> String {
    format!("{name}({})", usize::from(pft_type) + 1)
}

/// Return the authoritative MOD_Const_PFT defaults plus sparse overrides for
/// one PFT type.  Only parameters active under every selected case are shown;
/// this keeps an All-sites edit from silently writing a coefficient some cases
/// cannot use.
#[tauri::command]
pub fn pft_parameter_states(
    dirs: Vec<String>,
    pft_type: u8,
    kernel_dir: String,
) -> Result<Vec<PftParameterState>, String> {
    if dirs.is_empty() {
        return Err("没有可配置的算例".into());
    }
    let kernel = colm_kernel::Kernel::open(std::path::Path::new(&kernel_dir))
        .map_err(|error| format!("{error:#}"))?;
    let have: std::collections::BTreeSet<&str> =
        kernel.manifest.macros.iter().map(String::as_str).collect();
    let crop = have.contains("CROP");
    let max = if crop { 78 } else { 15 };
    if pft_type > max {
        return Err(format!("当前内核只支持 PFT 0..={max}，收到 {pft_type}"));
    }
    let all = read_all(&dirs)?;
    let docs = all
        .iter()
        .map(|(dir, text)| {
            let doc = colm_namelist::parse(text).map_err(|e| format!("{dir}: {e:#}"))?;
            let context = VisibilityContext::new(&doc, &have);
            if !(context.single && (context.pft || context.pc) && context.biological_land()) {
                return Err(format!("{dir}: 当前算例不是可编辑的单点 PFT/PC 植被算例"));
            }
            Ok(doc)
        })
        .collect::<Result<Vec<_>, String>>()?;

    let modes = docs
        .iter()
        .map(|doc| VisibilityContext::new(doc, &have).pc)
        .collect::<Vec<_>>();
    if modes.iter().any(|pc| pc != &modes[0]) {
        return Err("不能在同一次批量编辑中混合普通 PFT 与 PC 组分；请缩小范围".into());
    }
    let pc_mode = modes[0];

    let mut out = Vec::new();
    for meta in colm_case::pft::all_parameters() {
        let contexts = docs
            .iter()
            .map(|doc| VisibilityContext::new(doc, &have))
            .collect::<Vec<_>>();
        let mut available = true;
        for context in &contexts {
            available &= pft_parameter_applies(meta, context, pft_type)
                && pft_parameter_has_default(meta, context, pft_type)?;
        }
        if !available {
            continue;
        }
        let defaults = contexts
            .iter()
            .map(|context| {
                colm_case::pft::default_literal(
                    meta.name,
                    pft_type,
                    logical(context.doc, "DEF_USE_Campbell_SOIL_MODEL"),
                    context.pc,
                )
                .map_err(|error| format!("{}: {error:#}", meta.name))?
                .ok_or_else(|| format!("{} 没有 PFT 默认值", meta.name))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let path = pft_override_path(meta.name, pft_type);
        let values = docs
            .iter()
            .map(|doc| doc.get(&path).map(ToString::to_string))
            .collect::<Vec<_>>();
        let normal_pft_default = if pc_mode {
            colm_case::pft::default_literal(
                meta.name,
                pft_type,
                logical(contexts[0].doc, "DEF_USE_Campbell_SOIL_MODEL"),
                false,
            )
            .map_err(|error| format!("{}: {error:#}", meta.name))?
        } else {
            None
        };
        let effective_value = values[0].clone().unwrap_or_else(|| defaults[0].clone());
        out.push(PftParameterState {
            name: meta.name,
            label_zh: meta.label_zh,
            label_en: meta.label_en,
            group_zh: meta.group_zh,
            group_en: meta.group_en,
            unit: meta.unit,
            kind: match meta.kind {
                colm_case::pft::Kind::Real => "real",
                colm_case::pft::Kind::Integer => "integer",
            },
            default: defaults[0].clone(),
            default_mixed: defaults.iter().any(|value| value != &defaults[0]),
            value: values[0].clone(),
            mixed: values.iter().any(|value| value != &values[0]),
            allowed_values: if meta.name == "DEF_PFT_C3C4" {
                vec!["0", "1"]
            } else {
                Vec::new()
            },
            scope_kind: if pc_mode { "pc-pft" } else { "pft" },
            scope_label: format!("{}-{}", if pc_mode { "PC/PFT" } else { "PFT" }, pft_type),
            normal_pft_default,
            effective_value,
            provenance: if values[0].is_some() {
                "case.nml"
            } else {
                "MOD_Const_PFT.F90"
            },
        });
    }
    Ok(out)
}

#[tauri::command]
pub fn set_pft_parameter_batch(
    dirs: Vec<String>,
    pft_type: u8,
    name: String,
    value: Option<String>,
    kernel_dir: String,
) -> Result<BatchWrite, String> {
    set_pft_parameters_batch(
        vec![PftBatchChange {
            dirs,
            pft_type,
            name,
            value,
        }],
        kernel_dir,
    )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PftBatchChange {
    pub dirs: Vec<String>,
    pub pft_type: u8,
    pub name: String,
    pub value: Option<String>,
}

/// Validate every PFT/PC cell first, then commit every affected case.nml atomically.
#[tauri::command]
pub fn set_pft_parameters_batch(
    changes: Vec<PftBatchChange>,
    kernel_dir: String,
) -> Result<BatchWrite, String> {
    if changes.is_empty() || changes.iter().any(|change| change.dirs.is_empty()) {
        return Err("没有可配置的 PFT/PC 单元格".into());
    }
    let kernel = colm_kernel::Kernel::open(std::path::Path::new(&kernel_dir))
        .map_err(|error| format!("{error:#}"))?;
    let have: std::collections::BTreeSet<&str> =
        kernel.manifest.macros.iter().map(String::as_str).collect();
    let max = if have.contains("CROP") { 78 } else { 15 };
    let mut by_dir = std::collections::BTreeMap::<String, Vec<&PftBatchChange>>::new();
    for change in &changes {
        if change.pft_type > max {
            return Err(format!(
                "当前内核只支持 PFT 0..={max}，收到 {}",
                change.pft_type
            ));
        }
        let meta = colm_case::pft::parameter(&change.name)
            .ok_or_else(|| format!("未知 PFT 参数：{}", change.name))?;
        if let Some(value) = change.value.as_deref() {
            typed_pft_value(meta, value)?;
        }
        for dir in &change.dirs {
            by_dir.entry(dir.clone()).or_default().push(change);
        }
    }
    let kernel_facts = KernelFacts {
        single: have.contains("SinglePoint"),
        usgs: have.contains("LULC_USGS"),
        crop: have.contains("CROP"),
    };
    let dirs = by_dir.keys().cloned().collect::<Vec<_>>();
    let texts = read_all(&dirs)?
        .into_iter()
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut done = Vec::with_capacity(by_dir.len());
    let mut pc_mode = None;
    for (dir, scoped_changes) in by_dir {
        let text = texts
            .get(&dir)
            .expect("read_all returned every requested case");
        let mut doc = colm_namelist::parse(text).map_err(|e| format!("{dir}: {e:#}"))?;
        for change in scoped_changes {
            let meta = colm_case::pft::parameter(&change.name).expect("validated above");
            let (valid_case, mode) = {
                let context = VisibilityContext::new(&doc, &have);
                (
                    context.single
                        && (context.pft || context.pc)
                        && context.biological_land()
                        && pft_parameter_applies(meta, &context, change.pft_type)
                        && pft_parameter_has_default(meta, &context, change.pft_type)?,
                    context.pc,
                )
            };
            if !valid_case {
                return Err(format!(
                    "{dir}: {} 在当前 PFT/PC 或过程配置下不生效",
                    meta.name
                ));
            }
            if pc_mode.is_some_and(|current| current != mode) {
                return Err("不能在同一次批量编辑中混合普通 PFT 与 PC 组分；请缩小范围".into());
            }
            pc_mode = Some(mode);
            let path = pft_override_path(meta.name, change.pft_type);
            match change.value.as_deref() {
                Some(raw) => doc
                    .insert(&path, typed_pft_value(meta, raw)?, "nl_colm")
                    .map_err(|e| format!("{dir}: {e:#}"))?,
                None => {
                    doc.remove(&path).map_err(|e| format!("{dir}: {e:#}"))?;
                }
            }
        }
        validate_runtime_contract(&doc, std::path::Path::new(&dir), Some(kernel_facts))
            .map_err(|e| format!("{dir}: {e}"))?;
        done.push((dir, doc.to_string()));
    }
    write_all(&done)
}

fn typed_pft_value(
    meta: &colm_case::pft::ParameterMeta,
    raw: &str,
) -> Result<colm_namelist::Value, String> {
    let number = parse_real(raw).ok_or_else(|| format!("{} 需要数值，收到 {raw:?}", meta.name))?;
    colm_case::pft::validate_override(meta.name, number).map_err(|error| format!("{error:#}"))?;
    Ok(match meta.kind {
        colm_case::pft::Kind::Real => colm_namelist::Value::Real {
            text: raw.trim().to_string(),
        },
        colm_case::pft::Kind::Integer => {
            if number.fract() != 0.0 {
                return Err(format!("{} 必须是整数", meta.name));
            }
            if !(i32::MIN as f64..=i32::MAX as f64).contains(&number) {
                return Err(format!("{} 超出 Fortran 默认整数/i32 范围", meta.name));
            }
            colm_namelist::Value::Int(number as i64)
        }
    })
}

#[derive(Debug, Serialize)]
pub struct ProcessParamFile {
    pub file: String,
    pub title: String,
    pub section: &'static str,
    pub entries: Vec<ProcessParamEntry>,
}

#[derive(Debug, Serialize)]
pub struct ProcessParamEntry {
    pub path: String,
    pub value: String,
    pub default: Option<String>,
    pub kind: &'static str,
    pub group: String,
    pub unset: bool,
    pub doc: Option<String>,
}

type ProcessCodeDefault = colm_case::parameters::process::ProcessDefault;

fn process_section(name: &str, groups: &std::collections::BTreeSet<String>) -> &'static str {
    let n = name.to_ascii_lowercase();
    if groups.iter().any(|group| {
        group.starts_with("nl_colm_tracer")
            || group.contains("methane")
            || group.contains("sediment")
    }) || n.contains("methane")
        || n.contains("ch4")
        || n.contains("tracer")
        || n.contains("sediment")
        || n.contains("chloride")
        || n.contains("hdo")
        || n.contains("o18")
    {
        "示踪剂"
    } else if n.contains("cama") || n.contains("flood") || n.contains("dam") {
        "河道与水库"
    } else if n.contains("urban") {
        "城市"
    } else if n.contains("bgc") || n.contains("carbon") || n.contains("nitrogen") {
        "生态与生地化"
    } else {
        "水热过程"
    }
}

fn value_kind(value: &colm_namelist::Value) -> &'static str {
    match value {
        colm_namelist::Value::Bool(_) => "logical",
        colm_namelist::Value::Int(_) => "integer",
        colm_namelist::Value::Real { .. } => "real",
        colm_namelist::Value::Str(_) => "character",
        colm_namelist::Value::List(_) => "list",
    }
}

fn typed_like(reference: &colm_namelist::Value, raw: &str) -> Result<colm_namelist::Value, String> {
    use colm_namelist::Value;
    let s = raw.trim();
    match reference {
        Value::Bool(_) => match s.to_ascii_lowercase().trim_matches('.') {
            "true" | "t" => Ok(Value::Bool(true)),
            "false" | "f" => Ok(Value::Bool(false)),
            _ => Err(format!("{raw:?} 不是逻辑值")),
        },
        Value::Int(_) => s
            .parse()
            .map(Value::Int)
            .map_err(|_| format!("{raw:?} 不是整数")),
        Value::Real { .. } => parse_real(s)
            .filter(|v| v.is_finite())
            .map(|_| Value::Real {
                text: s.to_string(),
            })
            .ok_or_else(|| format!("{raw:?} 不是实数")),
        Value::Str(_) => Ok(Value::Str(s.trim_matches(['\'', '"']).to_string())),
        Value::List(items) => {
            let parts: Vec<&str> = s
                .split(',')
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .collect();
            if parts.is_empty() {
                return Err("列表不能为空".into());
            }
            let fallback_string;
            let fallback = match items.first() {
                Some(value) => value,
                None => {
                    fallback_string = Value::Str(String::new());
                    &fallback_string
                }
            };
            Ok(Value::List(
                parts
                    .iter()
                    .enumerate()
                    .map(|(i, part)| typed_like(items.get(i).unwrap_or(fallback), part))
                    .collect::<Result<_, _>>()?,
            ))
        }
    }
}

fn typed_process(kind: &str, raw: &str) -> Result<colm_namelist::Value, String> {
    use colm_namelist::Value;
    let reference = match kind {
        "logical" => Value::Bool(false),
        "integer" => Value::Int(0),
        "real" => Value::Real { text: "0".into() },
        "character" => Value::Str(String::new()),
        _ => return Err(format!("不支持新增 {kind} 类型的过程参数")),
    };
    typed_like(&reference, raw)
}

fn typed_process_known(
    kind: &str,
    current: &colm_namelist::Value,
    raw: &str,
) -> Result<colm_namelist::Value, String> {
    if matches!(current, colm_namelist::Value::List(_)) {
        let values: Vec<_> = raw
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| typed_process(kind, value))
            .collect::<Result<_, _>>()?;
        if values.is_empty() {
            return Err("列表不能为空".into());
        }
        Ok(colm_namelist::Value::List(values))
    } else {
        typed_process(kind, raw)
    }
}

fn tracer_param_files(doc: &colm_namelist::Document) -> Vec<String> {
    let Some(colm_namelist::Value::Str(raw)) = doc.get("DEF_TRACER_PARAM_FILES") else {
        return Vec::new();
    };
    raw.split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty() && !x.eq_ignore_ascii_case("null"))
        .map(|x| {
            x.rsplit_once(':')
                .map_or(x, |(_, file)| file)
                .trim()
                .trim_matches(['\'', '"'])
                .to_string()
        })
        .collect()
}

fn safe_process_file(case_dir: &std::path::Path, file: &str) -> Result<std::path::PathBuf, String> {
    let path = std::path::Path::new(file);
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        case_dir.join(path)
    };
    let canon = candidate
        .canonicalize()
        .map_err(|e| format!("{}: {e}", candidate.display()))?;
    let case = case_dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", case_dir.display()))?;
    if !canon.starts_with(&case) {
        return Err(format!("过程参数文件不在算例目录内：{}", canon.display()));
    }
    Ok(canon)
}

fn process_code_defaults() -> Vec<ProcessCodeDefault> {
    colm_case::parameters::process::code_defaults()
}

fn process_entries(path: &std::path::Path, file_id: String) -> Result<ProcessParamFile, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let doc = colm_namelist::parse(&text).map_err(|e| format!("{}: {e:#}", path.display()))?;
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("parameter.nml")
        .to_string();
    let defaults = process_code_defaults();
    let by_path: std::collections::HashMap<String, &ProcessCodeDefault> = defaults
        .iter()
        .map(|field| (field.path.to_ascii_lowercase(), field))
        .collect();
    let mut group = String::new();
    let mut groups = std::collections::BTreeSet::new();
    let mut present = std::collections::BTreeSet::new();
    let mut entries = Vec::new();
    for item in &doc.items {
        match item {
            colm_namelist::document::Item::GroupStart(line) => {
                group = line.trim().trim_start_matches('&').to_string();
                groups.insert(group.to_ascii_lowercase());
            }
            colm_namelist::document::Item::Entry(entry) => {
                let path = entry.path.to_string();
                present.insert(path.to_ascii_lowercase());
                let code = by_path.get(&path.to_ascii_lowercase()).copied();
                entries.push(ProcessParamEntry {
                    default: code.map(|field| field.value.clone()),
                    kind: value_kind(&entry.value),
                    value: entry.value.to_string(),
                    group: group.clone(),
                    unset: false,
                    doc: code.and_then(|field| field.doc.clone()),
                    path,
                });
            }
            _ => {}
        }
    }
    for field in defaults {
        if field.insertable
            && groups.contains(&field.group.to_ascii_lowercase())
            && !present.contains(&field.path.to_ascii_lowercase())
        {
            entries.push(ProcessParamEntry {
                path: field.path,
                value: field.value.clone(),
                default: Some(field.value),
                kind: field.kind,
                group: field.group.into(),
                unset: true,
                doc: field.doc,
            });
        }
    }
    Ok(ProcessParamFile {
        section: process_section(&name, &groups),
        title: name.clone(),
        file: file_id,
        entries,
    })
}

#[tauri::command]
pub fn process_parameter_files(dir: String) -> Result<Vec<ProcessParamFile>, String> {
    let case_dir = std::path::Path::new(&dir);
    let case_text = std::fs::read_to_string(case_dir.join("case.nml"))
        .map_err(|e| format!("{}: {e}", case_dir.join("case.nml").display()))?;
    let case_doc = colm_namelist::parse(&case_text).map_err(|e| format!("{dir}: {e:#}"))?;
    let mut files: std::collections::BTreeSet<String> =
        tracer_param_files(&case_doc).into_iter().collect();
    for entry in std::fs::read_dir(case_dir).map_err(|e| format!("{dir}: {e}"))? {
        let entry = entry.map_err(|e| format!("{dir}: {e}"))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with("_parameter.nml") || name.contains("parameter") && name.ends_with(".nml")
        {
            files.insert(name);
        }
    }
    files
        .into_iter()
        .map(|file| {
            safe_process_file(case_dir, &file).and_then(|path| process_entries(&path, file))
        })
        .collect()
}

#[tauri::command]
pub fn set_process_parameter_field_batch(
    dirs: Vec<String>,
    file: String,
    path: String,
    value: String,
) -> Result<BatchWrite, String> {
    if dirs.is_empty() {
        return Err("没有可配置的算例".into());
    }
    let mut done = Vec::with_capacity(dirs.len());
    for dir in &dirs {
        let case_dir = std::path::Path::new(dir);
        let path_file = safe_process_file(case_dir, &file)?;
        let text = std::fs::read_to_string(&path_file)
            .map_err(|e| format!("{}: {e}", path_file.display()))?;
        let mut doc =
            colm_namelist::parse(&text).map_err(|e| format!("{}: {e:#}", path_file.display()))?;
        let code = process_code_defaults()
            .into_iter()
            .find(|field| field.path.eq_ignore_ascii_case(&path));
        if let Some(current) = doc.get(&path).cloned() {
            let value = match code {
                Some(field) => typed_process_known(field.kind, &current, &value)?,
                None => typed_like(&current, &value)?,
            };
            doc.set(&path, value)
                .map_err(|e| format!("{path_file:?}: {e:#}"))?;
        } else {
            let field = code
                .filter(|field| field.insertable)
                .ok_or_else(|| format!("{} 里没有可安全新增的字段 {path}", path_file.display()))?;
            doc.insert(&path, typed_process(field.kind, &value)?, field.group)
                .map_err(|e| format!("{}: {e:#}", path_file.display()))?;
        }
        done.push((path_file, doc.to_string()));
    }
    let changed = write_process_files(&done)?;
    Ok(BatchWrite {
        written: done.len(),
        changed,
        text: std::fs::read_to_string(std::path::Path::new(&dirs[0]).join("case.nml"))
            .unwrap_or_default(),
    })
}

/// 删除 case-local 过程参数显式覆盖，让模型重新使用 Fortran 代码默认值。
#[tauri::command]
pub fn reset_process_parameter_field_batch(
    dirs: Vec<String>,
    file: String,
    path: String,
) -> Result<BatchWrite, String> {
    if dirs.is_empty() {
        return Err("没有可配置的算例".into());
    }
    if !process_code_defaults()
        .iter()
        .any(|field| field.path.eq_ignore_ascii_case(&path))
    {
        return Err(format!("未知过程参数：{path}"));
    }
    let mut done = Vec::with_capacity(dirs.len());
    for dir in &dirs {
        let case_dir = std::path::Path::new(dir);
        let parameter_file = safe_process_file(case_dir, &file)?;
        let text = std::fs::read_to_string(&parameter_file)
            .map_err(|e| format!("{}: {e}", parameter_file.display()))?;
        let mut doc = colm_namelist::parse(&text)
            .map_err(|e| format!("{}: {e:#}", parameter_file.display()))?;
        doc.remove(&path)
            .map_err(|e| format!("{}: {e:#}", parameter_file.display()))?;
        done.push((parameter_file, doc.to_string()));
    }
    let changed = write_process_files(&done)?;
    Ok(BatchWrite {
        written: done.len(),
        changed,
        text: std::fs::read_to_string(std::path::Path::new(&dirs[0]).join("case.nml"))
            .unwrap_or_default(),
    })
}

fn write_files_atomic(done: &[(std::path::PathBuf, String)]) -> Result<usize, String> {
    use std::io::Write as _;

    fn replace(staged: &std::path::Path, target: &std::path::Path) -> std::io::Result<()> {
        #[cfg(windows)]
        if target.exists() {
            // ponytail: std has no ReplaceFileW; same-directory remove+rename avoids
            // partial contents. Add a Windows syscall wrapper only if crash recovery
            // during this tiny rename window becomes a measured requirement.
            std::fs::remove_file(target)?;
        }
        std::fs::rename(staged, target)
    }

    // ponytail: edits are user-driven and rare; split this global lock only if measured contention appears.
    static WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    static BACKUP_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let _guard = WRITE_LOCK
        .lock()
        .map_err(|_| "配置写入锁已损坏；请重启 CoLM Desktop".to_string())?;
    let tag = format!(
        "{}.{}",
        std::process::id(),
        BACKUP_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let changed = done
        .iter()
        .filter(|(path, text)| {
            std::fs::read_to_string(path)
                .map(|current| current != text.as_str())
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();
    if changed.is_empty() {
        return Ok(0);
    }
    for (path, _) in &changed {
        if std::fs::metadata(path)
            .map_err(|error| format!("{}: {error}", path.display()))?
            .permissions()
            .readonly()
        {
            return Err(format!("{}: 文件为只读", path.display()));
        }
    }
    let mut backups = Vec::with_capacity(changed.len());
    let mut staged = Vec::with_capacity(changed.len());
    for (index, (path, text)) in changed.iter().enumerate() {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("parameter.nml");
        let backup = path.with_file_name(format!(".{name}.bak-{tag}-{index}"));
        let temporary = path.with_file_name(format!(".{name}.tmp-{tag}-{index}"));
        if let Err(error) = std::fs::copy(path, &backup) {
            for backup in &backups {
                let _ = std::fs::remove_file(backup);
            }
            return Err(format!("{}: {error}", backup.display()));
        }
        let write = (|| -> std::io::Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.set_permissions(std::fs::metadata(path)?.permissions())?;
            file.write_all(text.as_bytes())?;
            file.sync_all()
        })();
        if let Err(error) = write {
            let _ = std::fs::remove_file(&temporary);
            for path in staged.iter().chain(backups.iter()) {
                let _ = std::fs::remove_file(path);
            }
            return Err(format!("{}: {error}", temporary.display()));
        }
        backups.push(backup);
        staged.push(temporary);
    }
    for (index, ((path, _), temporary)) in changed.iter().zip(staged.iter()).enumerate() {
        if let Err(error) = replace(temporary, path) {
            for ((prior, _), backup) in changed.iter().zip(backups.iter()).take(index + 1) {
                let _ = replace(backup, prior);
            }
            for path in staged.iter().chain(backups.iter()) {
                let _ = std::fs::remove_file(path);
            }
            return Err(format!("{}: {error}", path.display()));
        }
    }
    for path in staged.iter().chain(backups.iter()) {
        let _ = std::fs::remove_file(path);
    }
    for parent in changed
        .iter()
        .filter_map(|(path, _)| path.parent())
        .collect::<std::collections::BTreeSet<_>>()
    {
        let _ = std::fs::File::open(parent).and_then(|directory| directory.sync_all());
    }
    Ok(changed.len())
}

fn write_process_files(done: &[(std::path::PathBuf, String)]) -> Result<usize, String> {
    write_files_atomic(done)
}

/// 一份 namelist 里的一个字段，交给前端渲染。
#[derive(Serialize)]
pub struct Entry {
    pub path: String,
    /// 值的**原文**，与文件里一模一样
    pub value: String,
    /// `colm-schema` 认不认识它
    pub known: bool,
    pub kind: Option<String>,
    pub group: Option<&'static str>,
    pub derived: bool,
}

/// 读一份 namelist 文本，列出它设了哪些字段。
#[tauri::command]
pub fn read_case(text: String) -> Result<Vec<Entry>, String> {
    let doc = colm_namelist::parse(&text).map_err(|e| format!("{e:#}"))?;
    Ok(doc
        .paths()
        .into_iter()
        .filter(|path| !colm_case::pft::is_override_path(path))
        .map(|p| {
            let f = colm_schema::find(&p);
            Entry {
                value: doc.get(&p).map(|v| v.to_string()).unwrap_or_default(),
                known: f.is_some(),
                kind: f.map(|f| format!("{:?}", f.kind)),
                group: f.and_then(|f| f.group),
                derived: f.is_some_and(|f| f.group.is_none()),
                path: p,
            }
        })
        .collect())
}

/// 改一个字段，返回**整份**文本。
///
/// 无状态往返：命令收整份文档加一个改动，返回重新校验过的整份文档。
/// 前端不持有配置状态，也**从不自己构造带类型的值** —— 类型由
/// `colm-schema` 决定，字符串怎么变成 `Value` 是这里的事。
///
/// 未被改动的行**逐字节不变**，这是 `colm-namelist` 的往返保证：
/// 用户算例文件里的注释是他们自己的笔记，保存一次不该把它们冲掉。
#[tauri::command]
#[cfg(test)]
pub fn set_field(text: String, path: String, value: String) -> Result<String, String> {
    let mut doc = colm_namelist::parse(&text).map_err(|e| format!("{e:#}"))?;
    let v = typed(&path, &value)?;
    doc.set(&path, v).map_err(|e| format!("{e:#}"))?;
    Ok(doc.to_string())
}

/// 按 schema 声明的类型把字符串变成 `Value`。
///
/// schema 不认识的字段一律当字符串 —— 让它写出去，由 CoLM 去表态。
/// 静默丢弃会让用户以为自己设了。
fn typed(path: &str, raw: &str) -> Result<colm_namelist::Value, String> {
    use colm_namelist::Value;
    use colm_schema::FieldKind as K;
    let s = raw.trim();
    let Some(f) = colm_schema::find(path) else {
        return Ok(Value::Str(s.to_string()));
    };
    let bare = s.trim_matches(|c| c == '\'' || c == '"');
    if !f.values.is_empty() && !f.values.iter().any(|v| v.eq_ignore_ascii_case(bare)) {
        return Err(format!(
            "{path} only accepts {}; {raw:?} is invalid",
            f.values.join(", ")
        ));
    }
    match f.kind {
        K::Logical => match s.to_ascii_lowercase().trim_matches('.') {
            "true" | "t" => Ok(Value::Bool(true)),
            "false" | "f" => Ok(Value::Bool(false)),
            _ => Err(format!(
                "{path} is logical; {raw:?} is neither .true. nor .false."
            )),
        },
        K::Integer => s
            .parse()
            .map(Value::Int)
            .map_err(|_| format!("{path} is an integer; {raw:?} is not")),
        K::Real => {
            // 存原文：1800. 与 1800.0 与 1.8e3 等价，往返要还原用户写的那种。
            // 但先确认它确实是个数，否则会把一个打错的字悄悄写进文件。
            let value = parse_real(s)
                .ok_or_else(|| format!("{path} is a real; {raw:?} is not a number"))?;
            if !value.is_finite() {
                return Err(format!("{path} is a real; {raw:?} is not finite"));
            }
            Ok(Value::Real {
                text: s.to_string(),
            })
        }
        K::Character { len } => {
            if bare.len() > len {
                return Err(format!(
                    "{path} holds character(len={len}); {:?} is {} characters",
                    bare,
                    bare.len()
                ));
            }
            Ok(Value::Str(bare.to_string()))
        }
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod config_tests;

/// 设一个字段：在文件里就改，不在就插进它该在的 namelist 组。
///
/// **必须能插。** 专家模式让用户改这份配置没设过的字段，而预热更是必然
/// 要插 —— 关掉预热时截止时刻那四项都不在文件里。只 `set` 的话，
/// 打开预热会报一句 `no such field in this namelist`，而那不是用户的错。
fn put(
    doc: &mut colm_namelist::Document,
    path: &str,
    v: colm_namelist::Value,
) -> Result<(), String> {
    // 组名从 schema 来 —— 那是从 CoLM 自己的声明里扫出来的。
    // schema 不认识的字段只能改不能插：不知道往哪个组插，而插错组等于没设。
    match colm_schema::find(path).and_then(|f| f.group) {
        Some(g) => doc.insert(path, v, g).map_err(|e| format!("{e:#}")),
        None => doc.set(path, v).map_err(|e| format!("{e:#}")),
    }
}

/// 读一批算例的 case.nml。
///
/// **一个读不了就整批失败。** 批量的坏处是"部分成功"——
/// 90 个算例里 3 个没改到，界面上看不出来，而它们会照旧跑一遍旧配置。
fn read_all(dirs: &[String]) -> Result<Vec<(String, String)>, String> {
    dirs.iter()
        .map(|d| {
            let p = std::path::Path::new(d).join("case.nml");
            std::fs::read_to_string(&p)
                .map(|t| (d.clone(), t))
                .map_err(|e| format!("{}: {e}", p.display()))
        })
        .collect()
}

/// 这一批算例里，哪些字段的取值不一致。
///
/// 界面据此在那些行上标出来 —— **不标的话，一个显示着某个值的输入框
/// 其实代表着 90 个不同的值**，而改它会把另外 89 个悄悄抹平。
#[tauri::command]
pub fn varying_fields(dirs: Vec<String>) -> Result<Vec<String>, String> {
    let all = read_all(&dirs)?;
    if all.len() < 2 {
        return Ok(Vec::new());
    }
    let docs: Vec<_> = all
        .iter()
        .map(|(d, t)| {
            colm_namelist::parse(t)
                .map(|doc| (d.clone(), doc))
                .map_err(|e| format!("{d}: {e:#}"))
        })
        .collect::<Result<_, _>>()?;
    // 并集而不是交集：某个算例**没设**某字段，本身就是一种不一致 ——
    // 它跑的是 CoLM 的默认值，而别的算例跑的是写出来的那个。
    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (_, doc) in &docs {
        names.extend(doc.paths());
    }
    let mut out = Vec::new();
    for n in names {
        let first = docs[0].1.get(&n).map(|v| v.to_string());
        if docs
            .iter()
            .any(|(_, d)| d.get(&n).map(|v| v.to_string()) != first)
        {
            out.push(n);
        }
    }
    Ok(out)
}

/// 一次批量写的结果。`text` 是**代表算例**（列表里第一个）改完之后的内容，
/// 界面拿它继续显示 —— 不回传的话前端还得再读一次文件。
#[derive(Debug, serde::Serialize)]
pub struct BatchWrite {
    pub written: usize,
    /// 实际内容发生变化的文件数；0 表示没有写文件、时间戳和结果状态不应改变。
    pub changed: usize,
    pub text: String,
}

/// 向导在新算例里写入的一项运行时初值。
#[derive(Debug, Deserialize)]
pub struct FieldChange {
    pub path: String,
    pub value: String,
}

/// 一次校验并写入一组字段。任一值无效时，原文件保持不变。
pub(crate) fn apply_fields(dir: &str, fields: &[FieldChange]) -> Result<(), String> {
    let path = std::path::Path::new(dir).join("case.nml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut doc = colm_namelist::parse(&text).map_err(|e| format!("{dir}: {e:#}"))?;
    for field in fields {
        let value = typed(&field.path, &field.value).map_err(|e| format!("{dir}: {e}"))?;
        put(&mut doc, &field.path, value).map_err(|e| format!("{dir}: {e}"))?;
    }
    validate_runtime_contract(&doc, std::path::Path::new(dir), None)
        .map_err(|e| format!("{dir}: {e}"))?;
    validate_changed_fields(&doc, fields).map_err(|e| format!("{dir}: {e}"))?;
    stage_ch4_parameter(dir, fields)?;
    std::fs::write(&path, doc.to_string()).map_err(|e| format!("{}: {e}", path.display()))
}

/// 复用上游 CH4 参数，只关闭单点没有的路由/CROP/空间 pH 输入。
fn stage_ch4_parameter(dir: &str, fields: &[FieldChange]) -> Result<(), String> {
    let wants_builtin = fields.iter().any(|field| {
        field.path == "DEF_TRACER_PARAM_FILES"
            && field.value.trim_matches(['\'', '"']) == "CH4:standard_ch4_parameter.nml"
    });
    if !wants_builtin {
        return Ok(());
    }

    let mut text =
        include_str!("../../../vendor/CoLM202X/run/standard_ch4_parameter.nml").to_string();
    for (from, to) in [
        (
            "DEF_METHANE%inundation_mode  = 'hybrid'",
            "DEF_METHANE%inundation_mode  = 'wetwat'",
        ),
        (
            "DEF_METHANE%enable_rice_paddy = .true.",
            "DEF_METHANE%enable_rice_paddy = .false.",
        ),
        (
            "DEF_METHANE%use_spatial_ph   = .true.",
            "DEF_METHANE%use_spatial_ph   = .false.",
        ),
    ] {
        if text.matches(from).count() != 1 {
            return Err(format!("内置 CH4 参数模板缺少唯一设置：{from}"));
        }
        text = text.replacen(from, to, 1);
    }
    let path = std::path::Path::new(dir).join("standard_ch4_parameter.nml");
    std::fs::write(&path, text).map_err(|error| format!("{}: {error}", path.display()))
}

/// 把一个字段写进这一批算例的每一份 case.nml。
///
/// **先全改完再落盘。** 中途出错就一份都不写 —— 半批配置好的算例
/// 与整批配置好的在界面上长得一样，而它们跑出来的东西不一样。
#[tauri::command]
pub fn set_field_batch(
    dirs: Vec<String>,
    path: String,
    value: String,
    kernel_dir: Option<String>,
) -> Result<BatchWrite, String> {
    set_fields_batch(dirs, vec![FieldChange { path, value }], kernel_dir)
}

/// 删除一个 case.nml 显式覆盖。恢复内置值必须走删除语义，不能把当前默认值
/// 固化回文件；任一目标校验失败时整批保持不变。
#[tauri::command]
pub fn reset_field_batch(
    dirs: Vec<String>,
    path: String,
    kernel_dir: Option<String>,
) -> Result<BatchWrite, String> {
    let field = colm_schema::find(&path).ok_or_else(|| format!("未知字段：{path}"))?;
    if field.group.is_none() {
        return Err(format!("{path} 是只读派生字段，不能重置"));
    }
    let kernel = kernel_facts(kernel_dir.as_deref())?;
    let all = read_all(&dirs)?;
    let mut done = Vec::with_capacity(all.len());
    for (dir, text) in all {
        let mut doc = colm_namelist::parse(&text).map_err(|e| format!("{dir}: {e:#}"))?;
        doc.remove(&path).map_err(|e| format!("{dir}: {e:#}"))?;
        validate_runtime_contract(&doc, std::path::Path::new(&dir), kernel)
            .map_err(|e| format!("{dir}: {e}"))?;
        done.push((dir, doc.to_string()));
    }
    write_all(&done)
}

/// 把一组有关联的字段原子地写进整批算例。
///
/// 用于“启用初始场并选择文件”和互斥开关：不能先把父开关写成 true，再因
/// 路径或另一开关写入失败而留下半套配置。
#[tauri::command]
pub fn set_fields_batch(
    dirs: Vec<String>,
    fields: Vec<FieldChange>,
    kernel_dir: Option<String>,
) -> Result<BatchWrite, String> {
    if fields.is_empty() {
        return Err("没有要保存的字段".into());
    }
    let kernel = kernel_facts(kernel_dir.as_deref())?;
    let all = read_all(&dirs)?;
    let mut done: Vec<(String, String)> = Vec::with_capacity(all.len());
    for (d, text) in all {
        let mut doc = colm_namelist::parse(&text).map_err(|e| format!("{d}: {e:#}"))?;
        for field in &fields {
            let value = typed(&field.path, &field.value)?;
            put(&mut doc, &field.path, value).map_err(|e| format!("{d}: {e}"))?;
        }
        validate_runtime_contract(&doc, std::path::Path::new(&d), kernel)
            .map_err(|e| format!("{d}: {e}"))?;
        validate_changed_fields(&doc, &fields).map_err(|e| format!("{d}: {e}"))?;
        done.push((d, doc.to_string()));
    }
    write_all(&done)
}

pub(crate) fn write_all(done: &[(String, String)]) -> Result<BatchWrite, String> {
    let files = done
        .iter()
        .map(|(dir, text)| (std::path::Path::new(dir).join("case.nml"), text.clone()))
        .collect::<Vec<_>>();
    let changed = write_files_atomic(&files)?;
    Ok(BatchWrite {
        written: done.len(),
        changed,
        text: done
            .first()
            .map(|(_, text)| text.clone())
            .unwrap_or_default(),
    })
}

/// 一份配置里与「时间与预热」有关的东西，界面直接照着显示。
///
/// **算好了再交出去**，不让前端自己拼：预热截止时刻是起始年月日加上若干年，
/// 而输出从截止时刻才开始 —— 这两条算错了没人会发现，输出会安安静静地
/// 少一段。同一份算式在 `colm-case::spinup_fields` 里，两边共用它。
#[derive(serde::Serialize)]
pub struct Timing {
    /// 这一批有几个算例。
    pub count: usize,
    /// 各算例的窗口是否一致。**多站点时通常不一致** —— 每个站点的窗口
    /// 是它自己那份强迫场的完整覆盖范围，而各站点的记录长短本来就不同。
    pub window_varies: bool,
    pub start: String,
    pub end: String,
    pub spinup_years: u32,
    pub spinup_repeat: u32,
    /// 各算例的预热设置是否一致。
    pub spinup_varies: bool,
    /// history 从哪天开始。**不等于 start** —— 预热期不写 history
    /// （`MOD_Hist.F90:235` 在 `itstamp <= ptstamp` 时直接 RETURN）。
    pub output_start: String,
    /// CoLM 会打印的 TIMESTEP 总数，含每一轮预热。进度事件里的 `step`
    /// 从 1 单调递增到这个数，所以前端不必再猜百分比。
    pub total_steps: u64,
}

/// 读出时间窗与预热。
///
/// 取不到的项用 CoLM 的声明默认值，与 `read_case` 的口径一致 ——
/// 一个没写进文件的字段不是"没有值"，而是"用默认值"。
#[tauri::command]
pub fn read_timing(dirs: Vec<String>) -> Result<Timing, String> {
    let all = read_all(&dirs)?;
    let mut each = Vec::with_capacity(all.len());
    for (d, text) in &all {
        let doc = colm_namelist::parse(text).map_err(|e| format!("{d}: {e:#}"))?;
        each.push(one_timing(&doc));
    }
    let Some(first) = each.first().cloned() else {
        return Err("没有算例".into());
    };
    Ok(Timing {
        count: each.len(),
        window_varies: each.iter().any(|t| t.0 != first.0 || t.1 != first.1),
        start: first.0.clone(),
        end: first.1.clone(),
        spinup_years: first.2,
        spinup_repeat: first.3,
        spinup_varies: each.iter().any(|t| t.2 != first.2 || t.3 != first.3),
        output_start: first.4.clone(),
        total_steps: first.5,
    })
}

/// 一份配置的 (start, end, 预热年数, 预热遍数, 输出起始日, 总步数)。
fn one_timing(doc: &colm_namelist::Document) -> (String, String, u32, u32, String, u64) {
    let int = |p: &str| -> i64 {
        match doc.get(p) {
            Some(colm_namelist::Value::Int(v)) => *v,
            _ => match colm_schema::find(p).map(|f| f.default) {
                Some(colm_schema::Default::Integer(v)) => v,
                _ => 0,
            },
        }
    };
    let (sy, sm, sd) = (
        int("DEF_simulation_time%start_year"),
        int("DEF_simulation_time%start_month"),
        int("DEF_simulation_time%start_day"),
    );
    let (ey, em, ed) = (
        int("DEF_simulation_time%end_year"),
        int("DEF_simulation_time%end_month"),
        int("DEF_simulation_time%end_day"),
    );
    let repeat = int("DEF_simulation_time%spinup_repeat").max(0) as u32;
    let py = int("DEF_simulation_time%spinup_year");
    // 预热开着的判据与 CoLM 一样：截止时刻晚于起始时刻（`CoLM.F90:300`）。
    // `spinup_repeat = 1` 仍会把 start→spinup 截止这段当预热跑一遍且不写 history；
    // 手写 0 也会被 CoLM 提成 1。界面关闭预热靠写 `spinup_year = 0`。
    let on = py > sy;
    let repeat = if on { repeat.max(1) } else { repeat };
    let ymd = |y: i64, m: i64, d: i64| format!("{y:04}-{m:02}-{d:02}");
    let stamp = |y: i64, m: i64, d: i64, sec: i64| {
        // Howard Hinnant 的 days_from_civil。这里不能依赖 colm-forcing：那会把
        // netcdf/hdf5 拖进窗口进程，只为算两个日期的差。
        let y = if m <= 2 { y - 1 } else { y };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let mp = if m > 2 { m - 3 } else { m + 9 };
        let doy = (153 * mp + 2) / 5 + d - 1;
        (era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy) * 86_400 + sec
    };
    let start = stamp(sy, sm, sd, int("DEF_simulation_time%start_sec"));
    let end = stamp(ey, em, ed, int("DEF_simulation_time%end_sec"));
    let step = doc
        .get("DEF_simulation_time%timestep")
        .and_then(colm_namelist::Value::as_f64)
        .or_else(
            || match colm_schema::find("DEF_simulation_time%timestep")?.default {
                colm_schema::Default::Real(v) => v.parse().ok(),
                _ => None,
            },
        )
        .unwrap_or(0.0) as i64;
    let steps = |from: i64, to: i64| -> u64 {
        if step <= 0 || to <= from {
            0
        } else {
            ((to - from) as u64).div_ceil(step as u64)
        }
    };
    let normal_steps = steps(start, end);
    let total_steps = if on {
        let spinup_end = stamp(
            py,
            int("DEF_simulation_time%spinup_month"),
            int("DEF_simulation_time%spinup_day"),
            int("DEF_simulation_time%spinup_sec"),
        );
        // 截止时刻必须落在窗口内才会触发重置。最后一轮不重置，而是从
        // 截止处继续跑到 end；与 CoLM.F90 的 TIMELOOP 完全同序。
        if spinup_end < end {
            let spinup_steps = steps(start, spinup_end);
            spinup_steps * repeat as u64 + steps(start + spinup_steps as i64 * step, end)
        } else {
            normal_steps
        }
    } else {
        normal_steps
    };
    (
        ymd(sy, sm, sd),
        ymd(ey, em, ed),
        if on { (py - sy) as u32 } else { 0 },
        if on { repeat } else { 0 },
        if on {
            ymd(
                py,
                int("DEF_simulation_time%spinup_month"),
                int("DEF_simulation_time%spinup_day"),
            )
        } else {
            ymd(sy, sm, sd)
        },
        total_steps,
    )
}

/// 改这一批算例的预热。
///
/// 五个字段一起写 —— 单改一个会得到一个自相矛盾的截止时刻。
/// **每个算例按自己的起始年算截止年**：各站点的强迫场起点不同，
/// 用同一个绝对年份会让一部分算例的预热落在窗口之外（等于没预热），
/// 另一部分落得过深（等于把输出砍掉一大截）。
#[tauri::command]
pub fn set_spinup(
    dirs: Vec<String>,
    years: u32,
    repeat: u32,
    kernel_dir: Option<String>,
) -> Result<BatchWrite, String> {
    let kernel = kernel_facts(kernel_dir.as_deref())?;
    let all = read_all(&dirs)?;
    let mut done = Vec::with_capacity(all.len());
    for (d, text) in all {
        let mut doc = colm_namelist::parse(&text).map_err(|e| format!("{d}: {e:#}"))?;
        let int = |p: &str| -> i64 {
            match doc.get(p) {
                Some(colm_namelist::Value::Int(v)) => *v,
                _ => match colm_schema::find(p).map(|f| f.default) {
                    Some(colm_schema::Default::Integer(v)) => v,
                    _ => 0,
                },
            }
        };
        let start = (
            int("DEF_simulation_time%start_year") as i32,
            int("DEF_simulation_time%start_month") as u32,
            int("DEF_simulation_time%start_day") as u32,
            int("DEF_simulation_time%start_sec") as u32,
        );
        let end_stamp = simulation_stamp(&doc, "DEF_simulation_time%end_");
        let spinup_end = civil_stamp(
            start.0 as i64 + years as i64,
            start.1 as i64,
            start.2 as i64,
            start.3 as i64,
        );
        if years > 0 && repeat > 0 && spinup_end >= end_stamp {
            return Err(format!(
                "{d}: 预热截止时间必须早于模拟结束时间；请缩短预热年数或延长模拟窗口"
            ));
        }
        let spinup = colm_case::Spinup { years, repeat };
        for (path, v) in colm_case::spinup_fields(start, spinup) {
            put(&mut doc, &path, v).map_err(|e| format!("{d}: {e}"))?;
        }
        validate_runtime_contract(&doc, std::path::Path::new(&d), kernel)
            .map_err(|e| format!("{d}: {e}"))?;
        done.push((d, doc.to_string()));
    }
    write_all(&done)
}

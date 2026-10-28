//! 外部观测（NetCDF / 导入产物）解码、单位与 QC 语义的单一入口。
//!
//! 结果页以前有三条口径不同的观测读取路径：metrics 直读只按属性标缺测、
//! 不解 scale/offset、从不校验单位；CSV 导入识别了列头单位却只改标签；
//! 能量诊断又会拒绝非恒等 packing。三条路都「跑得完」，但同一个文件经
//! 不同路径进入图表时，数值含义可能已经不同 —— 这正是本模块要消掉的分叉。
//!
//! 处理顺序固定，每一步都要能说清为什么：
//!
//! 1. **原始存储域标缺测**：`_FillValue` / `missing_value` 属性与变量声明的
//!    （或默认）fill 值都是**打包前**的存储值，必须先于 scale/offset 比较。
//!    先解包再比 fill 会漏掉 -32767 这类填充值。
//! 2. **解 scale/offset**：netcdf-c 4.10 的 typed getter 不再自动应用
//!    packing（本仓库用的 0.12.1 crate 也一样，服务端已核对过实现），
//!    漏掉这一步会把 1000 当 100 用。恒等 packing 走原样，保证逐位复现。
//! 3. **换规范单位**：`EvaluationVariable::canonical_units` 是纯 ASCII 的
//!    比较目标；不认识的单位拒绝（NaN 保留），缺 `units` 属性则告警并按
//!    规范单位使用 —— 量纲未经验证这件事必须留在结果里。
//! 4. **QC**：区分「源文件自带的实测/插补标记」「CSV 导入生成的有效性掩码」
//!    与「没有 QC」。三者对指标的含义不同，不能一律标成 measured_only。
//! 5. **订正版本**：请求订正但文件里没有订正变量时记录 `NotApplied`，
//!    保留未订正变量并告警 —— 静默回退会让界面声称的口径与实际不符。
//!
//! 缺测统一用 `NaN` 表示，而不是保留各文件自己的填充值：下游（配对、
//! 均值、降采样）只要判有限性就能工作，不必知道这个文件用了 -9999 还是
//! 9.969e36。
//!
//! 观测的 `time` 变量不在这里解码：时间基准与标签语义由调用方按
//! `TimeBasis` / `ObsLabelPosition` 处理，本模块只负责「值 + 质量」的语义。

use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use colm_hist::obs::EvaluationVariable;
use colm_hist::pair::{ObsLabelPosition, FILL_VALUE, QC_MEASURED};
use netcdf::File;

/// 解码一次观测请求的条件。字段都是显式的：调用方不能「猜」口径。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ObsDecodeRequest {
    /// 是否请求能量闭合订正版观测（`Qle_cor` / `Qh_cor`）。
    pub corrected: bool,
    /// 观测时间标签代表区间起点还是终点。解码本身用不到它，但要和
    /// 文件里记录的口径交叉核对（导入产物会写 `obs_time_label_position`）；
    /// 不一致时把差异留在 warnings 里，而不是等配对阶段才发现。
    pub label_position: ObsLabelPosition,
}

/// QC 从哪来。决定指标表上写 `measured_only` 还是 `generated_validity`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QcProvenance {
    /// 源文件自带的 QC 变量（实测/插补标记，例如 PLUMBER2 的 0=实测）。
    Source { variable: String },
    /// 导入时**生成**的有效性掩码（有限值=0/缺失=1）：它只说明值在不在，
    /// 不是仪器质量。把它标成实测质量是对指标含义的误述。
    ImportedGenerated { variable: String, rule: String },
    /// 该观测没有 QC 契约，只能使用全部有限值。
    Absent,
}

/// 订正版观测实际用没用上。缺变量时不再静默回退。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorrectionStatus {
    NotRequested,
    Applied { variable: String },
    NotApplied { requested: String, reason: String },
}

/// 解码后的观测序列：值已换到规范单位，缺测是 `NaN`。
#[derive(Debug, Clone)]
pub struct DecodedObs {
    /// 实际读的观测变量（可能带 `_cor` 后缀），或派生表达式。
    pub label: String,
    pub values: Vec<f64>,
    /// 与 `values` 等长：0 表示实测，非 0/NaN 表示不可用。
    pub qc: Vec<f64>,
    /// 物理量的源单位：优先取导入器记录的 `source_units`（CSV 导入前的
    /// 原始单位），否则取文件 `units` 属性；`None` 表示没有声明、
    /// 按规范单位假定。
    pub source_units: Option<String>,
    /// 解码后使用的规范单位（纯 ASCII）。
    pub units: String,
    /// 源文件缺少 `units` 属性、按规范单位假定。
    pub units_assumed: bool,
    pub qc_provenance: QcProvenance,
    /// 实际采用的 QC 变量名（派生 Rnet 是四个分量 QC 的联合表达式）。
    pub qc_variable: Option<String>,
    pub correction: CorrectionStatus,
    pub warnings: Vec<String>,
}

/// `QcProvenance` 对应的指标表口径标签。
///
/// 保留旧字符串 `measured_only` / `finite_only`，新增 `generated_validity`；
/// 导出与界面据此说明「这批指标的质量到底按什么筛的」。
pub fn qc_label(provenance: &QcProvenance) -> &'static str {
    match provenance {
        QcProvenance::Source { .. } => "measured_only",
        QcProvenance::ImportedGenerated { .. } => "generated_validity",
        QcProvenance::Absent => "finite_only",
    }
}

/// 解码一个评估变量的观测。`Ok(None)` 表示文件里没有这个量（含派生 Rnet
/// 的分量不齐），调用方据此标不可用；返回 `Err` 表示文件里有这个量但
/// 语义不能安全解读（单位未知、packing 非法、声明了 QC 却没有 QC 变量等）。
pub fn decode(
    file: &File,
    path: &Path,
    variable: &EvaluationVariable,
    request: &ObsDecodeRequest,
) -> Result<Option<DecodedObs>> {
    let mut warnings = Vec::new();
    check_label_position(file, path, request, &mut warnings)?;
    let (target, correction) =
        resolve_correction(file, path, variable, request.corrected, &mut warnings);

    if file.variable(&target).is_some() {
        let decoded = decode_direct(file, path, variable, &target, correction, warnings)?;
        return Ok(Some(decoded));
    }

    // 直读变量不在：Urban-PLUMBER 一类的文件不写 Rnet，只给四个辐射分量。
    decode_derived_rnet(file, path, variable, correction, warnings)
}

fn decode_direct(
    file: &File,
    path: &Path,
    variable: &EvaluationVariable,
    target: &str,
    correction: CorrectionStatus,
    mut warnings: Vec<String>,
) -> Result<DecodedObs> {
    let (values, source_units, units_assumed) =
        read_values_in_units(file, path, target, variable.canonical_units, &mut warnings)?;
    let (qc, qc_provenance, qc_variable) = read_qc(file, path, variable, target, values.len())?;
    Ok(DecodedObs {
        label: target.to_string(),
        values,
        qc,
        source_units,
        units: variable.canonical_units.to_string(),
        units_assumed,
        qc_provenance,
        qc_variable,
        correction,
        warnings,
    })
}

/// 观测文件没有直接 Rnet 时，用 `SWdown+LWdown-SWup-LWup` 派生（Urban-PLUMBER）。
///
/// 保持原来的严格规则：四个分量都有效且 QC 都是实测，派生样本才算实测；
/// 否则该时刻缺测。分量各自先换到规范单位再相减 —— 混合单位相加是错的。
fn decode_derived_rnet(
    file: &File,
    path: &Path,
    variable: &EvaluationVariable,
    correction: CorrectionStatus,
    mut warnings: Vec<String>,
) -> Result<Option<DecodedObs>> {
    let o_name = variable.observation;
    let Some(components) = colm_hist::obs::derived_observation_components(o_name) else {
        return Ok(None);
    };
    let Some(qc_names) = colm_hist::obs::derived_observation_qc(o_name) else {
        return Ok(None);
    };
    if !components
        .iter()
        .chain(qc_names.iter())
        .all(|name| file.variable(name).is_some())
    {
        return Ok(None);
    }

    let mut component_values: Vec<Vec<f64>> = Vec::with_capacity(components.len());
    let mut component_qc: Vec<Vec<f64>> = Vec::with_capacity(qc_names.len());
    let mut shared_units: Option<String> = None;
    let mut mixed_units = false;
    let mut units_assumed = false;
    for (component, qc_name) in components.iter().zip(qc_names.iter()) {
        let (values, source_units, assumed) = read_values_in_units(
            file,
            path,
            component,
            variable.canonical_units,
            &mut warnings,
        )?;
        if let Some(units) = source_units.as_deref() {
            match shared_units.as_deref() {
                None => shared_units = Some(units.to_string()),
                Some(previous) if previous != units => mixed_units = true,
                Some(_) => {}
            }
        }
        units_assumed |= assumed || source_units.is_none();
        component_values.push(values);
        component_qc.push(read_stored_series(file, path, qc_name)?);
    }
    if mixed_units {
        shared_units = None;
        warnings.push(format!(
            "{}: the four urban Rnet components declare different source units; they were each converted to the canonical unit before deriving Rnet",
            path.display()
        ));
    }

    let n = component_values[0].len();
    ensure!(
        component_values.iter().all(|values| values.len() == n)
            && component_qc.iter().all(|values| values.len() == n),
        "{}: urban Rnet components have inconsistent time lengths",
        path.display()
    );

    let (derived, qc) = colm_hist::obs::derive_urban_rnet(
        [
            &component_values[0],
            &component_values[1],
            &component_values[2],
            &component_values[3],
        ],
        [
            &component_qc[0],
            &component_qc[1],
            &component_qc[2],
            &component_qc[3],
        ],
    )?;
    // `derive_urban_rnet` 用 FILL_VALUE 标派生不出的时刻；本模块统一用 NaN。
    let values = derived
        .into_iter()
        .map(|value| {
            if value.is_finite() && value > FILL_VALUE + 1.0 {
                value
            } else {
                f64::NAN
            }
        })
        .collect();

    let qc_variables = qc_names.join("+");
    let mut generated = Vec::new();
    for qc_name in qc_names {
        if let QcProvenance::ImportedGenerated { rule, .. } = qc_provenance(file, qc_name)? {
            generated.push(rule);
        }
    }
    let qc_provenance = if generated.is_empty() {
        QcProvenance::Source {
            variable: qc_variables.clone(),
        }
    } else {
        QcProvenance::ImportedGenerated {
            variable: qc_variables.clone(),
            rule: generated.join("; "),
        }
    };

    Ok(Some(DecodedObs {
        label: colm_hist::obs::derived_observation_label(o_name)
            .unwrap_or(o_name)
            .to_string(),
        values,
        qc,
        source_units: shared_units,
        units: variable.canonical_units.to_string(),
        units_assumed,
        qc_provenance,
        qc_variable: Some(qc_variables),
        correction,
        warnings,
    }))
}

/// 决定读哪一条观测序列：请求订正且文件里有订正变量就用订正的，
/// 否则保留原始变量，并把「没订正成」写进结果。
fn resolve_correction(
    file: &File,
    path: &Path,
    variable: &EvaluationVariable,
    corrected_requested: bool,
    warnings: &mut Vec<String>,
) -> (String, CorrectionStatus) {
    let observation = variable.observation.to_string();
    if !corrected_requested {
        return (observation, CorrectionStatus::NotRequested);
    }
    let Some(requested) = colm_hist::obs::corrected(variable.observation) else {
        // 这个量本身没有订正版（辐射量、地表热通量）：不算「没订正成」。
        return (observation, CorrectionStatus::NotRequested);
    };
    if file.variable(requested).is_some() {
        return (
            requested.to_string(),
            CorrectionStatus::Applied {
                variable: requested.to_string(),
            },
        );
    }
    let reason = format!(
        "{} has no {}; the uncorrected {} is still used",
        path.display(),
        requested,
        variable.observation
    );
    warnings.push(format!(
        "requested the energy-closure corrected {requested}, but {} has no such variable; evaluating the uncorrected {} instead",
        path.display(),
        variable.observation
    ));
    (
        observation,
        CorrectionStatus::NotApplied {
            requested: requested.to_string(),
            reason,
        },
    )
}

/// 解析实际要用的 QC 变量。
///
/// 订正版优先找 `Qle_cor_qc` 这类同名 QC；没有就回退到原始变量的 QC
/// （PLUMBER2 的 `Qle_cor` 与 `Qle` 出自同一份涡度观测，质量标记同源）。
/// 实际用了哪一个会写进 `DecodedObs::qc_variable`，不能只留一个布尔。
fn resolve_qc_variable(file: &File, variable: &EvaluationVariable, target: &str) -> Option<String> {
    if target != variable.observation {
        let corrected_qc = format!("{target}_qc");
        if file.variable(&corrected_qc).is_some() {
            return Some(corrected_qc);
        }
    }
    variable
        .qc
        .filter(|name| file.variable(name).is_some())
        .map(str::to_string)
}

fn read_qc(
    file: &File,
    path: &Path,
    variable: &EvaluationVariable,
    target: &str,
    n: usize,
) -> Result<(Vec<f64>, QcProvenance, Option<String>)> {
    let Some(qc_name) = resolve_qc_variable(file, variable, target) else {
        if variable.qc.is_some() {
            bail!(
                "{}: {target} declares a QC contract ({:?}) but the file has no usable QC variable; refusing to treat it as QC-free",
                path.display(),
                variable.qc
            );
        }
        // 没有 QC 契约：全部按实测可用，缺测由 values 的 NaN 承担。
        return Ok((vec![QC_MEASURED; n], QcProvenance::Absent, None));
    };
    let qc = read_stored_series(file, path, &qc_name)?;
    ensure!(
        qc.len() == n,
        "{}: QC variable {qc_name} has {} values for {n} observations",
        path.display(),
        qc.len()
    );
    let provenance = qc_provenance(file, &qc_name)?;
    Ok((qc, provenance, Some(qc_name)))
}

/// QC 变量的来源：先看导入器写的 `qc_kind`，再看旧的 `source` 自由文本。
/// 两者都没有时按源文件 QC 处理（历史上只有源文件会带 QC 变量）。
fn qc_provenance(file: &File, qc_name: &str) -> Result<QcProvenance> {
    let kind = text_attribute(file, qc_name, "qc_kind")?;
    let source = text_attribute(file, qc_name, "source")?;
    if let Some(kind) = kind {
        let normalized = kind.trim().to_ascii_lowercase();
        if normalized.starts_with("generated") {
            return Ok(QcProvenance::ImportedGenerated {
                variable: qc_name.to_string(),
                rule: source.unwrap_or(kind),
            });
        }
        return Ok(QcProvenance::Source {
            variable: qc_name.to_string(),
        });
    }
    if let Some(source) = source {
        if source.trim().to_ascii_lowercase().starts_with("generated") {
            return Ok(QcProvenance::ImportedGenerated {
                variable: qc_name.to_string(),
                rule: source,
            });
        }
    }
    Ok(QcProvenance::Source {
        variable: qc_name.to_string(),
    })
}

/// 读一条观测序列并换到规范单位。返回 `(值, 源单位, 是否假定单位)`。
fn read_values_in_units(
    file: &File,
    path: &Path,
    name: &str,
    canonical_units: &str,
    warnings: &mut Vec<String>,
) -> Result<(Vec<f64>, Option<String>, bool)> {
    let values = read_stored_series(file, path, name)?;
    let declared_units = text_attribute(file, name, "units")?;
    // 导入器会把换算前的列头单位保存到 `source_units`；它比换算后的
    // `units` 更能回答「这个数最早是什么单位写的」。
    let imported_from = text_attribute(file, name, "source_units")?;
    match declared_units
        .as_deref()
        .map(str::trim)
        .filter(|units| !units.is_empty())
    {
        Some(source) => {
            let converted = colm_forcing::units::convert_units(source, canonical_units, &values)
                .with_context(|| {
                    format!(
                        "{name} in {}: cannot convert source units {source:?} to {canonical_units:?}; \
                         fix the units attribute or add the conversion to forcing::units::TABLE",
                        path.display()
                    )
                })?;
            Ok((
                converted,
                imported_from.or_else(|| Some(source.to_string())),
                false,
            ))
        }
        None => {
            warnings.push(format!(
                "{name} in {} has no units attribute; assuming canonical units {canonical_units} — the physical quantity is unverified",
                path.display()
            ));
            Ok((values, imported_from, true))
        }
    }
}

/// 读一条变量的原始存储值：先在存储域标缺测，再解 packing。
///
/// 两步顺序不能反：`_FillValue`/`missing_value` 记的是打包后的存储值。
fn read_stored_series(file: &File, path: &Path, name: &str) -> Result<Vec<f64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("{} has no variable {name}", path.display()))?;
    let mut values = variable
        .get_values::<f64, _>(..)
        .with_context(|| format!("cannot read {name} from {}", path.display()))?;

    let markers = missing_markers(&variable)?;
    if !markers.is_empty() {
        for value in &mut values {
            if markers.iter().any(|marker| same_domain(*value, *marker)) {
                *value = f64::NAN;
            }
        }
    }

    let (scale, offset) = packing(&variable, name)?;
    if !(scale == 1.0 && offset == 0.0) {
        for value in &mut values {
            // NaN * scale + offset 仍是 NaN：缺测不会被 packing 复活。
            *value = *value * scale + offset;
        }
    }
    Ok(values)
}

/// 缺测标记：声明的 `_FillValue` / `missing_value`，再加上变量声明的
/// （netcdf 默认）fill 值兜底。未写入的默认填充值是 netcdf-c 直接返回的，
/// 不带属性时也必须按缺测处理。
fn missing_markers(variable: &netcdf::Variable) -> Result<Vec<f64>> {
    let mut markers = Vec::new();
    for name in ["_FillValue", "missing_value"] {
        if let Some(raw) = variable.attribute(name) {
            let value = raw
                .value()
                .with_context(|| format!("malformed {name} attribute on {}", variable.name()))?;
            markers.extend(crate::numeric_attribute_values(value));
        }
    }
    if let Some(fill) = declared_fill_value(variable) {
        if !markers.iter().any(|marker| same_domain(*marker, fill)) {
            markers.push(fill);
        }
    }
    Ok(markers)
}

/// 变量声明的 fill 值（`nc_inq_var_fill`）。netcdf 0.12 的 `fill_value`
/// 要求传入与变量**存储类型**完全一致的类型，所以这里按类型分发。
fn declared_fill_value(variable: &netcdf::Variable) -> Option<f64> {
    use netcdf::types::{FloatType, IntType, NcVariableType};
    match variable.vartype() {
        NcVariableType::Float(FloatType::F64) => variable.fill_value::<f64>().ok().flatten(),
        NcVariableType::Float(FloatType::F32) => {
            variable.fill_value::<f32>().ok().flatten().map(f64::from)
        }
        NcVariableType::Int(IntType::U8) => {
            variable.fill_value::<u8>().ok().flatten().map(f64::from)
        }
        NcVariableType::Int(IntType::I8) => {
            variable.fill_value::<i8>().ok().flatten().map(f64::from)
        }
        NcVariableType::Int(IntType::U16) => {
            variable.fill_value::<u16>().ok().flatten().map(f64::from)
        }
        NcVariableType::Int(IntType::I16) => {
            variable.fill_value::<i16>().ok().flatten().map(f64::from)
        }
        NcVariableType::Int(IntType::U32) => {
            variable.fill_value::<u32>().ok().flatten().map(f64::from)
        }
        NcVariableType::Int(IntType::I32) => {
            variable.fill_value::<i32>().ok().flatten().map(f64::from)
        }
        NcVariableType::Int(IntType::U64) => variable
            .fill_value::<u64>()
            .ok()
            .flatten()
            .map(|value| value as f64),
        NcVariableType::Int(IntType::I64) => variable
            .fill_value::<i64>()
            .ok()
            .flatten()
            .map(|value| value as f64),
        _ => None,
    }
}

/// 读 packing 属性。恒等之外的任何取值都必须能证明是有限标量且 scale != 0，
/// 否则解包后的物理量没有定义。
fn packing(variable: &netcdf::Variable, name: &str) -> Result<(f64, f64)> {
    let scale = scalar_attribute(variable, "scale_factor")?.unwrap_or(1.0);
    let offset = scalar_attribute(variable, "add_offset")?.unwrap_or(0.0);
    if !scale.is_finite() || !offset.is_finite() {
        bail!("{name}: scale_factor / add_offset must be finite");
    }
    if scale == 0.0 {
        bail!("{name}: scale_factor is 0, so the packed values cannot be decoded");
    }
    Ok((scale, offset))
}

fn scalar_attribute(variable: &netcdf::Variable, attribute: &str) -> Result<Option<f64>> {
    let Some(attribute) = variable.attribute(attribute) else {
        return Ok(None);
    };
    let values = crate::numeric_attribute_values(
        attribute
            .value()
            .with_context(|| format!("malformed {attribute:?} attribute"))?,
    );
    match values.as_slice() {
        [] => bail!("{attribute:?} attribute is not numeric or is empty"),
        [value] if value.is_finite() => Ok(Some(*value)),
        [_] => bail!("{attribute:?} attribute is not finite"),
        _ => bail!(
            "{attribute:?} attribute must be a scalar, found {} values",
            values.len()
        ),
    }
}

fn text_attribute(
    file: &File,
    variable_name: &str,
    attribute_name: &str,
) -> Result<Option<String>> {
    let Some(variable) = file.variable(variable_name) else {
        return Ok(None);
    };
    let Some(attribute) = variable.attribute(attribute_name) else {
        return Ok(None);
    };
    text_value(
        attribute
            .value()
            .with_context(|| format!("malformed {attribute_name} attribute on {variable_name}"))?,
    )
}

fn global_text_attribute(file: &File, attribute: &str) -> Result<Option<String>> {
    let Some(value) = file.attribute(attribute) else {
        return Ok(None);
    };
    text_value(value.value()?)
}

fn text_value(value: netcdf::AttributeValue) -> Result<Option<String>> {
    match value {
        netcdf::AttributeValue::Str(text) => Ok(Some(text)),
        netcdf::AttributeValue::Strs(texts) => Ok(texts.into_iter().next()),
        other => bail!("expected a text attribute, found {other:?}"),
    }
}

/// 导入产物会记录时间标签口径；请求方声明的口径与文件不一致时告警。
/// 原生观测产品（PLUMBER2 / Urban-PLUMBER）不带这个属性，保持安静。
fn check_label_position(
    file: &File,
    path: &Path,
    request: &ObsDecodeRequest,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let Some(declared) = global_text_attribute(file, "obs_time_label_position")? else {
        return Ok(());
    };
    let declared = declared.trim().to_ascii_lowercase();
    let declared = match declared.as_str() {
        "start" | "begin" | "interval_start" => Some(ObsLabelPosition::Start),
        "end" | "interval_end" => Some(ObsLabelPosition::End),
        _ => None,
    };
    match declared {
        Some(declared) if declared == request.label_position => Ok(()),
        Some(declared) => {
            warnings.push(format!(
                "{} records observation labels as interval {} but this evaluation uses interval {}; check the observation time-label setting",
                path.display(),
                position_name(declared),
                position_name(request.label_position)
            ));
            Ok(())
        }
        None => {
            warnings.push(format!(
                "{} has an unrecognized obs_time_label_position attribute; the evaluation assumes interval {}",
                path.display(),
                position_name(request.label_position)
            ));
            Ok(())
        }
    }
}

fn position_name(position: ObsLabelPosition) -> &'static str {
    match position {
        ObsLabelPosition::Start => "start",
        ObsLabelPosition::End => "end",
    }
}

/// 缺测比较要保持在存储域的精确同一性；NaN 标记按 NaN 处理。
fn same_domain(left: f64, right: f64) -> bool {
    (left.is_nan() && right.is_nan()) || left == right
}

#[cfg(test)]
#[path = "observation_input_tests.rs"]
mod observation_input_tests;

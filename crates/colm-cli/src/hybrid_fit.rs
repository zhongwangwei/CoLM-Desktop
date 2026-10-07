//! `hybrid-fit`：两步训练的第二步（docs/design-hybrid.md 第 13 节）。
//!
//! 第一步是各站独立的参数调优 Study（每个 Study 一个或几个站点，率定 `DEF_PFT_*(k)` 或 `DEF_LC_*`）。
//! 这里读每个 Study 的最优成员，在它的基础算例上抓取逐行特征（`colm-rs` 空跑加 tap，行与运行时
//! 网络作用的行相同），按行的分类（PFT 或地类）对上率定值，拼成“特征 → 参数”样本，拟合网络。
//! 有两个以上 Study 时再做留一站交叉验证：每次留出一个 Study 的全部行，用其余的拟合、预测它。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use colm_hybrid::fit::{Dataset, FitOptions, Fitted};
use colm_hybrid::{Matrix, OutputSpec};

use super::study::spec::HybridStudySpec;
use super::{
    rust_model_crop_arguments, rust_model_executable, rust_model_land_cover, Kernel, Opts,
};

/// 一行样本：来自哪个 Study（交叉验证按它分组）、特征、目标、权重。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Row {
    pub group: usize,
    pub features: Vec<f64>,
    pub targets: Vec<f64>,
    pub weight: f64,
}

/// tap CSV 的一行：分类与各列的值。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TapRow {
    pub class: i64,
    pub values: BTreeMap<String, f64>,
}

pub(crate) fn parse_tap(text: &str) -> Result<Vec<TapRow>> {
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let header: Vec<&str> = lines
        .next()
        .context("the tap file is empty")?
        .split(',')
        .collect();
    let class = header
        .iter()
        .position(|name| *name == "class")
        .context("the tap file has no class column")?;
    lines
        .map(|line| {
            let cells: Vec<&str> = line.split(',').collect();
            ensure!(
                cells.len() == header.len(),
                "tap row has {} cells, header {}",
                cells.len(),
                header.len()
            );
            let mut values = BTreeMap::new();
            for (name, cell) in header.iter().zip(&cells) {
                if !cell.is_empty() && !["patch", "pft", "class"].contains(name) {
                    values.insert((*name).to_owned(), cell.parse::<f64>()?);
                }
            }
            Ok(TapRow {
                class: cells[class].parse()?,
                values,
            })
        })
        .collect()
}

/// 一行的目标：每个输出在最优成员里对应的率定值。`pft` 插槽的键是 `DEF_PFT_X(k+1)`（`k` 为 PFT 类，
/// 见 `ParameterSpec::member_key`）；`land_class` 插槽是不带下标的 `DEF_LC_X`，只用于与它的地类相同
/// 的行。缺任一个输出就返回 `None`（这一行没有率定值，不进样本）。
pub(crate) fn row_targets(
    network: &HybridStudySpec,
    parameters: &BTreeMap<String, f64>,
    land_classes: &BTreeMap<String, i64>,
    class: i64,
) -> Option<Vec<f64>> {
    network
        .outputs
        .iter()
        .map(|output| {
            if network.slot == "pft" {
                parameters
                    .get(&format!("{}({})", output.name, class + 1))
                    .copied()
            } else {
                let value = parameters.get(&output.name).copied()?;
                land_classes
                    .get(&output.name)
                    .is_none_or(|&scoped| scoped == class)
                    .then_some(value)
            }
        })
        .collect()
}

fn options(opts: &Opts, network: &HybridStudySpec) -> Result<FitOptions> {
    let number = |name: &str| -> Result<Option<f64>> {
        opts.get(name)
            .map(|value| {
                value
                    .parse::<f64>()
                    .with_context(|| format!("{name} {value:?}"))
            })
            .transpose()
    };
    let defaults = FitOptions::default();
    Ok(FitOptions {
        hidden: network.hidden.clone(),
        activation: network.activation,
        epochs: number("--epochs")?.map_or(defaults.epochs, |v| v as usize),
        learning_rate: number("--learning-rate")?.unwrap_or(defaults.learning_rate),
        ridge: number("--ridge")?.unwrap_or(defaults.ridge),
        seed: number("--seed")?.map_or(defaults.seed, |v| v as u64),
    })
}

pub(crate) fn output_specs(network: &HybridStudySpec) -> Vec<OutputSpec> {
    network
        .outputs
        .iter()
        .map(|output| OutputSpec {
            name: output.name.clone(),
            range: Some(output.range),
            transform: output.transform,
        })
        .collect()
}

fn dataset(rows: &[&Row]) -> Result<Dataset> {
    let f = rows.first().map_or(0, |row| row.features.len());
    let o = rows.first().map_or(0, |row| row.targets.len());
    Ok(Dataset {
        features: Matrix::new(
            rows.len(),
            f,
            rows.iter().flat_map(|row| row.features.clone()).collect(),
        )?,
        targets: Matrix::new(
            rows.len(),
            o,
            rows.iter().flat_map(|row| row.targets.clone()).collect(),
        )?,
        weights: rows.iter().map(|row| row.weight).collect(),
    })
}

/// 各输出的（加权）均方根误差。
fn rmse(fitted: &Fitted, outputs: &[OutputSpec], rows: &[&Row]) -> Result<Vec<f64>> {
    let data = dataset(rows)?;
    let predicted = colm_hybrid::fit::predict(fitted, outputs, &data.features)?;
    let total: f64 = data.weights.iter().sum();
    Ok((0..outputs.len())
        .map(|out| {
            ((0..data.features.rows)
                .map(|row| {
                    data.weights[row]
                        * (predicted.row(row)[out] - data.targets.row(row)[out]).powi(2)
                })
                .sum::<f64>()
                / total)
                .sqrt()
        })
        .collect())
}

/// 全部行拟合出的网络、它在训练行上各输出的误差、每个分组留出时的误差（不做交叉验证时为 `None`）。
pub(crate) type FitOutcome = (Fitted, Vec<f64>, Vec<Option<Vec<f64>>>);

/// 拟合全部行，并在两个以上分组时做留一组交叉验证。
pub(crate) fn fit_rows(
    rows: &[Row],
    outputs: &[OutputSpec],
    options: &FitOptions,
    groups: usize,
) -> Result<FitOutcome> {
    let all: Vec<&Row> = rows.iter().collect();
    let fitted = colm_hybrid::fit::fit(&dataset(&all)?, outputs, options)?;
    let train = rmse(&fitted, outputs, &all)?;
    let mut held_out = Vec::with_capacity(groups);
    for group in 0..groups {
        let (test, rest): (Vec<&Row>, Vec<&Row>) = all.iter().partition(|row| row.group == group);
        held_out.push(if groups < 2 || test.is_empty() || rest.is_empty() {
            None
        } else {
            let model = colm_hybrid::fit::fit(&dataset(&rest)?, outputs, options)?;
            Some(rmse(&model, outputs, &test)?)
        });
    }
    Ok((fitted, train, held_out))
}

/// 在算例上空跑加 tap，返回 CSV 文本。
fn tap(case: &Path, spec: &Path, kernel: &Kernel, scratch: &Path) -> Result<String> {
    let out = scratch.join("tap.csv");
    let _ = std::fs::remove_file(&out);
    let executable = rust_model_executable()?;
    let output = std::process::Command::new(&executable)
        .current_dir(case)
        .arg(case.join("case.nml"))
        .args([
            "--land-cover",
            rust_model_land_cover(kernel)?,
            "--case-outputs",
        ])
        .args(["--hybrid-dry-run", "--hybrid"])
        .arg(spec)
        .arg("--hybrid-tap")
        .arg(&out)
        .args(rust_model_crop_arguments(kernel))
        .output()
        .with_context(|| format!("cannot start {}", executable.display()))?;
    if !output.status.success() {
        bail!(
            "cannot read the features of {}:\n{}",
            case.display(),
            String::from_utf8_lossy(&output.stderr).trim_end()
        );
    }
    std::fs::read_to_string(&out).with_context(|| format!("cannot read {}", out.display()))
}

/// 只声明特征与输出的插槽配置（tap 用）。
fn spec_toml(network: &HybridStudySpec) -> String {
    use crate::hybrid_cmd::toml_string;
    let mut text = format!(
        "[[slot]]\nname = {}\nkind = \"param\"\nfeatures = [{}]\noutputs = [\n",
        toml_string(&network.slot),
        network
            .features
            .iter()
            .map(|f| toml_string(f))
            .collect::<Vec<_>>()
            .join(", ")
    );
    for output in &network.outputs {
        text += &format!(
            "  {{ name = {}, range = [{:?}, {:?}] }},\n",
            toml_string(&output.name),
            output.range[0],
            output.range[1]
        );
    }
    text + "]\n"
}

pub(super) fn cmd_hybrid_fit(opts: &Opts) -> Result<()> {
    let studies: Vec<PathBuf> = opts
        .need_str("--studies")?
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect();
    ensure!(!studies.is_empty(), "--studies names no Study directory");
    let network_path = opts.need("--network")?;
    let network: HybridStudySpec = serde_json::from_str(
        &std::fs::read_to_string(&network_path)
            .with_context(|| format!("cannot read {}", network_path.display()))?,
    )
    .with_context(|| format!("cannot parse {}", network_path.display()))?;
    network.validate(&[])?;
    let weight_column = opts.get("--weight");
    let kernel = Kernel::open(&opts.need("--kernel")?)?;
    let out = opts.need("--out")?;
    ensure!(
        out.to_string_lossy().ends_with(".mlp.json"),
        "--out must end with .mlp.json"
    );
    let options = options(opts, &network)?;
    let outputs = output_specs(&network);

    let scratch = std::env::temp_dir().join(format!("colm-hybrid-fit-{}", std::process::id()));
    std::fs::create_dir_all(&scratch)?;
    let spec_path = scratch.join("hybrid.toml");
    std::fs::write(&spec_path, spec_toml(&network))?;

    let mut rows = Vec::new();
    let mut sources = Vec::new();
    let collected = (|| -> Result<()> {
        for (group, study) in studies.iter().enumerate() {
            let study = study
                .canonicalize()
                .with_context(|| format!("cannot find Study {}", study.display()))?;
            let manifest = crate::study::engine::status(&study)?;
            let member = crate::study::runner::resolve_apply_member(&study, &manifest, "best")?;
            let land_classes: BTreeMap<String, i64> = manifest
                .spec
                .parameters
                .iter()
                .filter_map(|p| {
                    let index = p.scope_instance.as_ref()?.index?;
                    p.name
                        .starts_with("DEF_LC_")
                        .then(|| (p.name.clone(), i64::from(index)))
                })
                .collect();
            let case_root = crate::study::runner::study_case_root(&manifest)?;
            for site in &manifest.spec.base_cases {
                let case = case_root.join(site).canonicalize()?;
                let tapped = parse_tap(&tap(&case, &spec_path, &kernel, &scratch)?)?;
                let (mut kept, mut dropped) = (0usize, 0usize);
                for row in tapped {
                    let Some(targets) =
                        row_targets(&network, &member.parameters, &land_classes, row.class)
                    else {
                        dropped += 1;
                        continue;
                    };
                    let features = network
                        .features
                        .iter()
                        .map(|name| {
                            row.values
                                .get(name)
                                .copied()
                                .with_context(|| format!("the tap of {site} has no {name}"))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    let weight = match &weight_column {
                        Some(name) => *row.values.get(name).with_context(|| {
                            format!("the tap of {site} has no weight column {name}")
                        })?,
                        None => 1.0,
                    };
                    if weight <= 0.0 {
                        dropped += 1;
                        continue;
                    }
                    rows.push(Row {
                        group,
                        features,
                        targets,
                        weight,
                    });
                    kept += 1;
                }
                sources.push(serde_json::json!({
                    "study": study.display().to_string(),
                    "member": member.id,
                    "site": site,
                    "rows": kept,
                    "dropped": dropped,
                }));
            }
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    collected?;
    ensure!(
        !rows.is_empty(),
        "no row has calibrated values for every output; check that each Study tuned {} for the classes present",
        network
            .outputs
            .iter()
            .map(|o| o.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let (fitted, train, held_out) = fit_rows(&rows, &outputs, &options, studies.len())?;
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let stem = out
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".mlp.json"))
        .context("--out must end with .mlp.json")?;
    let normalize = parent.join(format!("{stem}.norm.json"));
    let report_path = parent.join(format!("{stem}.fit.json"));
    fitted.mlp.save(&out)?;
    std::fs::write(
        &normalize,
        serde_json::to_string_pretty(&fitted.normalization)?,
    )?;
    let names: Vec<&str> = network.outputs.iter().map(|o| o.name.as_str()).collect();
    let report = serde_json::json!({
        "network": network,
        "rows": rows.len(),
        "sources": sources,
        "outputs": names,
        "train_rmse": train,
        "held_out_rmse": studies.iter().zip(&held_out).map(|(study, rmse)| serde_json::json!({
            "study": study.display().to_string(),
            "rmse": rmse,
        })).collect::<Vec<_>>(),
    });
    std::fs::write(&report_path, serde_json::to_string_pretty(&report)?)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    println!(
        "wrote {}, {}, {}",
        out.display(),
        normalize.display(),
        report_path.display()
    );
    println!(
        "install it with: colm-cli hybrid-install <case> --model {} --normalize {} --slot {} --features {} {}",
        out.display(),
        normalize.display(),
        network.slot,
        network.features.join(","),
        network
            .outputs
            .iter()
            .map(|o| format!(
                "--output {}:{}:{}:{}",
                o.name,
                o.range[0],
                o.range[1],
                serde_json::to_value(o.transform)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default()
            ))
            .collect::<Vec<_>>()
            .join(" ")
    );
    Ok(())
}

#[cfg(test)]
#[path = "hybrid_fit_tests.rs"]
mod hybrid_fit_tests;

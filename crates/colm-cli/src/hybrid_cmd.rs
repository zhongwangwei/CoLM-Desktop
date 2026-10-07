//! `hybrid-install` 与 `hybrid-check`：把训练好的模型（外部的 ONNX，或 Study 训练出的 `*.mlp.json`）
//! 装到算例上并检查（docs/design-hybrid.md）。

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};

use super::{
    rust_model_crop_arguments, rust_model_executable, rust_model_land_cover, Kernel, Opts,
};

/// 一个输出：名字、范围、变换、是否相对查表值（乘数）。
pub(crate) type OutputLine = (String, Option<[f64; 2]>, String, bool);

/// `--output NAME[:lo:hi[:transform[:relative]]]`。
fn parse_output(spec: &str) -> Result<OutputLine> {
    let parts: Vec<&str> = spec.split(':').collect();
    match parts.as_slice() {
        [name] => Ok((name.to_string(), None, "identity".into(), false)),
        [name, lo, hi] => Ok((
            name.to_string(),
            Some([lo.parse()?, hi.parse()?]),
            "identity".into(),
            false,
        )),
        [name, lo, hi, transform] => Ok((
            name.to_string(),
            Some([lo.parse()?, hi.parse()?]),
            transform.to_string(),
            false,
        )),
        [name, lo, hi, transform, "relative"] => Ok((
            name.to_string(),
            Some([lo.parse()?, hi.parse()?]),
            transform.to_string(),
            true,
        )),
        _ => bail!(
            "--output {spec:?} is not NAME, NAME:lo:hi, NAME:lo:hi:transform or NAME:lo:hi:transform:relative"
        ),
    }
}

fn copy_into(case: &Path, source: &Path) -> Result<PathBuf> {
    let name = source
        .file_name()
        .with_context(|| format!("{} has no file name", source.display()))?;
    let models = case.join("models");
    std::fs::create_dir_all(&models)
        .with_context(|| format!("cannot create {}", models.display()))?;
    let target = models.join(name);
    if source.canonicalize().ok() != target.canonicalize().ok() {
        std::fs::copy(source, &target)
            .with_context(|| format!("cannot copy {} to {}", source.display(), target.display()))?;
    }
    Ok(Path::new("models").join(name))
}

pub(crate) fn toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// 一个参数插槽的 `hybrid.toml` 文本。
pub(crate) fn slot_toml(
    slot: &str,
    model: &Path,
    sha256: &str,
    features: &[String],
    normalize: Option<&Path>,
    outputs: &[OutputLine],
    outside: Option<&str>,
) -> String {
    let mut text = String::from("[[slot]]\n");
    text += &format!("name = {}\n", toml_string(slot));
    text += "kind = \"param\"\n";
    text += &format!("model = {}\n", toml_string(&model.display().to_string()));
    text += &format!("sha256 = \"{sha256}\"\n");
    text += &format!(
        "features = [{}]\n",
        features
            .iter()
            .map(|f| toml_string(f))
            .collect::<Vec<_>>()
            .join(", ")
    );
    if let Some(normalize) = normalize {
        text += &format!(
            "normalize = {}\n",
            toml_string(&normalize.display().to_string())
        );
    }
    if let Some(outside) = outside {
        text += &format!("outside = {}\n", toml_string(outside));
    }
    text += "outputs = [\n";
    for (name, range, transform, relative) in outputs {
        let range = range.map_or(String::new(), |[lo, hi]| {
            format!(", range = [{lo:?}, {hi:?}]")
        });
        let relative = if *relative { ", relative = true" } else { "" };
        text += &format!(
            "  {{ name = {}{range}, transform = {}{relative} }},\n",
            toml_string(name),
            toml_string(transform)
        );
    }
    text += "]\n";
    text
}

pub(super) fn cmd_hybrid_install(opts: &Opts) -> Result<()> {
    let case = opts.positional_case()?;
    ensure!(
        case.join("case.nml").is_file(),
        "{} has no case.nml",
        case.display()
    );
    let model = opts.need("--model")?;
    let slot = opts.need_str("--slot")?;
    let features: Vec<String> = opts
        .need_str("--features")?
        .split(',')
        .map(|f| f.trim().to_owned())
        .filter(|f| !f.is_empty())
        .collect();
    let outputs = opts
        .get_all("--output")
        .iter()
        .map(|spec| parse_output(spec))
        .collect::<Result<Vec<_>>>()?;
    ensure!(!features.is_empty(), "--features names no feature");
    ensure!(!outputs.is_empty(), "give at least one --output");
    let outside = opts.get("--outside");
    ensure!(
        outside
            .as_deref()
            .is_none_or(|value| ["apply", "physics"].contains(&value)),
        "--outside must be apply or physics"
    );
    let config = case.join("hybrid.toml");
    ensure!(
        !config.exists() || opts.get("--force").is_some(),
        "{} already exists; pass --force 1 to replace it",
        config.display()
    );
    let model_rel = copy_into(&case, &model)?;
    let normalize_rel = opts
        .get("--normalize")
        .map(|path| copy_into(&case, Path::new(&path)))
        .transpose()?;
    let sha = colm_hybrid::sha256_hex(
        &std::fs::read(case.join(&model_rel))
            .with_context(|| format!("cannot read {}", model.display()))?,
    );
    let text = slot_toml(
        &slot,
        &model_rel,
        &sha,
        &features,
        normalize_rel.as_deref(),
        &outputs,
        outside.as_deref(),
    );
    std::fs::write(&config, &text).with_context(|| format!("cannot write {}", config.display()))?;
    // 按正式运行的规则读回：sha256、变换与范围；原生 MLP 再核对维度。ONNX 的维度由 hybrid-check 核对。
    let loaded = colm_hybrid::HybridConfig::load(&config).inspect_err(|_| {
        let _ = std::fs::remove_file(&config);
    })?;
    for slot in &loaded.slots {
        if let Some(model) = slot.model.as_deref().filter(|m| colm_hybrid::is_mlp(m)) {
            let mlp = colm_hybrid::Mlp::load(model)?;
            if mlp.inputs() != slot.features.len() || mlp.outputs() != slot.outputs.len() {
                let _ = std::fs::remove_file(&config);
                bail!(
                    "{} maps {} inputs to {} outputs, but {} features and {} outputs were given",
                    model.display(),
                    mlp.inputs(),
                    mlp.outputs(),
                    slot.features.len(),
                    slot.outputs.len()
                );
            }
        }
    }
    println!("wrote {}", config.display());
    println!(
        "check it with: colm-cli hybrid-check {} --kernel <dir>",
        case.display()
    );
    Ok(())
}

pub(super) fn cmd_hybrid_check(opts: &Opts) -> Result<()> {
    // 绝对路径：colm-rs 在算例目录里启动（算例里的相对路径按它解析），相对路径会被叠两次。
    let case = opts
        .positional_case()?
        .canonicalize()
        .context("cannot resolve the case directory")?;
    let config = case.join("hybrid.toml");
    ensure!(
        config.is_file(),
        "{} has no hybrid.toml; install a model with colm-cli hybrid-install first",
        case.display()
    );
    let kernel = Kernel::open(&opts.need("--kernel")?)?;
    print!("{}", dry_run(&case, &config, &kernel)?);
    Ok(())
}

/// 算例目录下的气候特征文件（`colm-rs --hybrid-climate` 写出，`clim_*` 特征读它）。
pub(crate) const CLIMATE_DIR: &str = "hybrid_climate";

/// 算例已有的气候文件（按名字排序）。
pub(crate) fn climate_files(case: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(case.join(CLIMATE_DIR))
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .filter(|path| path.extension().is_some_and(|ext| ext == "nc"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

pub(super) fn cmd_hybrid_climate(opts: &Opts) -> Result<()> {
    let case = opts
        .positional_case()?
        .canonicalize()
        .context("cannot resolve the case directory")?;
    let kernel = Kernel::open(&opts.need("--kernel")?)?;
    print!("{}", climate(&case, &kernel)?);
    Ok(())
}

/// 用 `colm-rs --hybrid-climate` 给算例算气候特征（运行时段内逐步累积强迫）。`case` 要是绝对路径。
pub(crate) fn climate(case: &Path, kernel: &Kernel) -> Result<String> {
    let executable = rust_model_executable()?;
    let output = std::process::Command::new(&executable)
        .current_dir(case)
        .arg(case.join("case.nml"))
        .args([
            "--land-cover",
            rust_model_land_cover(kernel)?,
            "--case-outputs",
            "--hybrid-climate",
        ])
        .args(rust_model_crop_arguments(kernel))
        .output()
        .with_context(|| format!("cannot start {}", executable.display()))?;
    if !output.status.success() {
        bail!(
            "cannot compute the climate features:\n{}\n(this needs the case's restarts and forcing; \
             if preprocessing has not run yet: colm-cli run {} --kernel <dir> --stage mkinidata)",
            String::from_utf8_lossy(&output.stderr).trim_end(),
            case.display()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// 在算例上空跑 `colm-rs --hybrid-dry-run`，返回它打印的 JSON 汇总。`case` 要是绝对路径。
pub(crate) fn dry_run(case: &Path, config: &Path, kernel: &Kernel) -> Result<String> {
    let land_cover = rust_model_land_cover(kernel)?;
    let executable = rust_model_executable()?;
    let output = std::process::Command::new(&executable)
        .current_dir(case)
        .arg(case.join("case.nml"))
        .args([
            "--land-cover",
            land_cover,
            "--case-outputs",
            "--hybrid-dry-run",
            "--hybrid",
        ])
        .arg(config)
        .args(rust_model_crop_arguments(kernel))
        .output()
        .with_context(|| format!("cannot start {}", executable.display()))?;
    if !output.status.success() {
        bail!(
            "the hybrid models cannot run on this case:\n{}\n(the check needs the case's restarts; \
             if preprocessing has not run yet: colm-cli run {} --kernel <dir> --stage mkinidata)",
            String::from_utf8_lossy(&output.stderr).trim_end(),
            case.display()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// 算例上的混合模型配置（GUI 读这个，不自己解析 TOML）：没有配置时 `installed` 为假；
/// 配置读得出来但模型校验不过（sha256 不符、文件缺失）时照样列出内容，并在 `error` 里说明。
pub(crate) fn hybrid_info(case: &Path) -> Result<serde_json::Value> {
    let land_mode = case_land_mode(case)?;
    let config = case.join("hybrid.toml");
    if !config.is_file() {
        return Ok(serde_json::json!({
            "installed": false,
            "land_mode": land_mode,
            "climate": !climate_files(case).is_empty(),
            "slots": [],
        }));
    }
    let slots = colm_hybrid::HybridConfig::declared(&config)?;
    let error = colm_hybrid::HybridConfig::load(&config)
        .err()
        .map(|e| format!("{e:#}"));
    let file_name = |path: &Path| {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
    };
    let slots = slots
        .iter()
        .map(|slot| {
            let model = slot.model.as_deref();
            serde_json::json!({
                "name": slot.name,
                "model": model.and_then(file_name),
                "format": model.map(|m| if colm_hybrid::is_mlp(m) { "mlp" } else { "onnx" }),
                "trained_by_study": model.and_then(file_name).as_deref() == Some("study.mlp.json"),
                "features": slot.features,
                "normalized": slot.normalize.is_some(),
                "outside": slot.outside,
                "training_range": slot.normalize.as_deref().is_some_and(|path| {
                    colm_hybrid::Normalization::load(path, slot.features.len())
                        .is_ok_and(|n| n.min.is_some() && n.max.is_some())
                }),
                "outputs": slot.outputs.iter().map(|output| serde_json::json!({
                    "name": output.name,
                    "range": output.range,
                    "transform": output.transform,
                    "relative": output.relative,
                })).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({
        "installed": true,
        "land_mode": land_mode,
        "climate": !climate_files(case).is_empty(),
        "uses_climate": slots.iter().any(|slot| slot["features"]
            .as_array()
            .is_some_and(|features| features.iter().any(|f| f.as_str().is_some_and(|f| f.starts_with("clim_"))))),
        "error": error,
        "slots": slots,
    }))
}

/// 算例的地表模式：`lct`、`pft` 或 `pc`。它决定能用哪个插槽（`land_class` 或 `pft`）。
fn case_land_mode(case: &Path) -> Result<&'static str> {
    let nml = case.join("case.nml");
    let text =
        std::fs::read_to_string(&nml).with_context(|| format!("cannot read {}", nml.display()))?;
    let document = colm_namelist::parse(&text)?;
    let on = |name: &str| match document.get(name) {
        Some(colm_namelist::Value::Bool(value)) => *value,
        _ => matches!(
            colm_schema::find(name).map(|field| field.default),
            Some(colm_schema::Default::Logical(true))
        ),
    };
    Ok(if on("DEF_USE_PC") {
        "pc"
    } else if on("DEF_USE_PFT") {
        "pft"
    } else {
        "lct"
    })
}

pub(super) fn cmd_hybrid_info(opts: &Opts) -> Result<()> {
    let case = opts.positional_case()?;
    println!("{}", serde_json::to_string(&hybrid_info(&case)?)?);
    Ok(())
}

/// 删掉 `hybrid.toml` 与它引用的、放在算例 `models/` 里的模型和标准化文件；`models/` 空了一并删掉。
pub(crate) fn hybrid_remove(case: &Path) -> Result<Vec<PathBuf>> {
    let config = case.join("hybrid.toml");
    ensure!(config.is_file(), "{} has no hybrid.toml", case.display());
    let models = case.join("models");
    let mut removed = Vec::new();
    // 配置坏了也要能删：读不出来就只删配置本身。
    if let Ok(slots) = colm_hybrid::HybridConfig::declared(&config) {
        let canonical_models = models.canonicalize().ok();
        for path in slots
            .iter()
            .flat_map(|slot| slot.model.iter().chain(&slot.normalize))
        {
            let inside = path
                .canonicalize()
                .ok()
                .zip(canonical_models.as_ref())
                .is_some_and(|(path, models)| path.starts_with(models));
            if inside && std::fs::remove_file(path).is_ok() {
                removed.push(path.clone());
            }
        }
    }
    std::fs::remove_file(&config).with_context(|| format!("cannot remove {}", config.display()))?;
    removed.push(config);
    if std::fs::read_dir(&models).is_ok_and(|mut entries| entries.next().is_none()) {
        let _ = std::fs::remove_dir(&models);
    }
    Ok(removed)
}

pub(super) fn cmd_hybrid_remove(opts: &Opts) -> Result<()> {
    let case = opts.positional_case()?;
    for path in hybrid_remove(&case)? {
        println!("removed {}", path.display());
    }
    Ok(())
}

#[cfg(test)]
#[path = "hybrid_cmd_tests.rs"]
mod hybrid_cmd_tests;

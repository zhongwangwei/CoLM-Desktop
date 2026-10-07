//! `hybrid-install` 与 `hybrid-check`：把训练好的模型（外部的 ONNX，或 Study 训练出的 `*.mlp.json`）
//! 装到算例上并检查（docs/design-hybrid.md）。

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};

use super::{
    rust_model_crop_arguments, rust_model_executable, rust_model_land_cover, Kernel, Opts,
};

/// `--output NAME[:lo:hi[:transform]]`。
fn parse_output(spec: &str) -> Result<(String, Option<[f64; 2]>, String)> {
    let parts: Vec<&str> = spec.split(':').collect();
    match parts.as_slice() {
        [name] => Ok((name.to_string(), None, "identity".into())),
        [name, lo, hi] => Ok((
            name.to_string(),
            Some([lo.parse()?, hi.parse()?]),
            "identity".into(),
        )),
        [name, lo, hi, transform] => Ok((
            name.to_string(),
            Some([lo.parse()?, hi.parse()?]),
            transform.to_string(),
        )),
        _ => bail!("--output {spec:?} is not NAME, NAME:lo:hi or NAME:lo:hi:transform"),
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

/// 一个参数插槽的 `hybrid.toml` 文本；`outputs` 是 `(名字, 范围, 变换)`。
pub(crate) fn slot_toml(
    slot: &str,
    model: &Path,
    sha256: &str,
    features: &[String],
    normalize: Option<&Path>,
    outputs: &[(String, Option<[f64; 2]>, String)],
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
    text += "outputs = [\n";
    for (name, range, transform) in outputs {
        let range = range.map_or(String::new(), |[lo, hi]| {
            format!(", range = [{lo:?}, {hi:?}]")
        });
        text += &format!(
            "  {{ name = {}{range}, transform = {} }},\n",
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

#[cfg(test)]
#[path = "hybrid_cmd_tests.rs"]
mod hybrid_cmd_tests;

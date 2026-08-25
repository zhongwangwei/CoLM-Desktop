//! 跑一个黄金算例的三段，用 colm-kernel 判成败。
//!
//! 用法: golden-run <case-name> [--kernel <dir>] [--write-golden]
//!
//! 环境变量 PLUMBER2_ROOT 必须指向含 Forcing/ Sitedata/ Observation/ 的目录。
//!
//! 本程序不再自己认内核、自己拼命令、自己判成败 —— 那三件事住在 `colm-kernel`，
//! 桌面端与这里共用同一份。留在本文件里的只有黄金回归特有的部分：
//! 校验第三方输入、展开算例模板、发现 history 文件、以及比对**溯源**
//! （见 `check_kernel_provenance`，它与 `Kernel::open` 验的不是同一件事）。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use colm_kernel::outcome::Stage;
use colm_kernel::{sha256_hex, Kernel, Manifest};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let case = args
        .next()
        .context("usage: golden-run <case-name> [--kernel <dir>] [--write-golden]")?;
    let mut kernel_dir = PathBuf::from("kernels/default");
    let mut write_golden = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--kernel" => kernel_dir = PathBuf::from(args.next().context("--kernel needs a path")?),
            "--write-golden" => write_golden = true,
            other => bail!("unknown argument: {other}"),
        }
    }

    let repo = repo_root()?;
    let plumber2 =
        PathBuf::from(std::env::var("PLUMBER2_ROOT").context("PLUMBER2_ROOT is not set")?);

    verify_inputs(&repo, &plumber2)?;
    let kernel = Kernel::open(&repo.join(&kernel_dir))?;
    println!(
        "  kernel: {} ({})",
        kernel.manifest.identity(),
        kernel.manifest.platform
    );
    check_kernel_provenance(&repo, &kernel.manifest)?;

    let case_dir = repo.join("oracle/cases").join(&case);
    if !case_dir.is_dir() {
        bail!("no such case: {}", case_dir.display());
    }
    let work = repo.join("oracle/work").join(&case);
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(work.join("out"))?;

    // namelist 模板展开。用 @NAME@ 而不是 shell 变量，是为了让模板本身
    // 就是一份合法的 namelist 文本，便于人眼审阅。
    let subst = |s: &str| -> String {
        s.replace("@PLUMBER2_ROOT@", plumber2.to_str().unwrap())
            .replace("@CASE_DIR@", case_dir.to_str().unwrap())
            .replace("@WORK_DIR@", work.to_str().unwrap())
    };
    fs::write(
        work.join("case.nml"),
        subst(&fs::read_to_string(case_dir.join("case.nml"))?),
    )?;
    // 强迫场 namelist 现生成，而不是展开一份手写模板。让黄金回归用生成器，
    // 等于每次回归都在验它 —— 生成的 namelist 若改变了语义，history 会先变。
    let met_name = fs::read_to_string(case_dir.join("met.txt"))
        .with_context(|| format!("no met.txt in {}", case_dir.display()))?
        .trim()
        .to_string();
    let forcing_dir = plumber2.join("Forcing");
    let met = forcing_dir.join(&met_name);
    let summary = colm_forcing::summarize(&met)?;
    let problems = colm_forcing::check(&summary, None);
    if !problems.is_empty() {
        for p in &problems {
            eprintln!("  {p}");
        }
        bail!("{} problem(s) with {}", problems.len(), met.display());
    }
    fs::write(
        work.join("forcing.nml"),
        colm_forcing::render(&colm_forcing::ForcingSpec {
            dir: forcing_dir.display().to_string(),
            file: met_name,
            met: summary,
        }),
    )?;

    let nml = work.join("case.nml");
    let case_name = read_case_name(&nml)?;
    let out = work.join("out").join(&case_name);

    // mkinidata 的产物必须列到**文件**，不能只列 restart/const 目录：
    // adjudicate 用的是 Path::exists，而目录在 mkinidata 写任何东西之前就已存在，
    // 于是「跑完了但什么都没写」——正是产物校验这条腿存在的理由——恰好抓不到。
    // 两个文件名见 design.md §6.2；block 后缀实测是 _w180_s90。
    let lc = land_cover_label(&nml)?;
    let const_dir = out.join("restart/const");
    let stages = [
        (Stage::MkSrfData, vec![out.join("landdata/srfdata.nc")]),
        (
            Stage::MkIniData,
            vec![
                const_dir.join(format!("{case_name}_restart_const_{lc}_w180_s90.nc")),
                const_dir.join(format!("{case_name}_restart_const_{lc}.nc")),
            ],
        ),
        (Stage::Colm, vec![]), // history 文件名含日期，单独发现
    ];

    for (stage, artifacts) in &stages {
        let r = colm_kernel::run_stage(&kernel, *stage, &nml, &work, artifacts)?;
        // 先报这一段的结论，覆盖消息缩进列在它下面 —— 反过来的话，读的人
        // 得先看完一屏消息才知道是哪一段说的。
        // 整行原样打印：每行自带 `Note:`/`Warning:` 前缀，再报一次 kind 只是重复。
        // 见 design.md §6.4。
        if r.succeeded() {
            println!("  {:<10} ok", stage.program());
        } else {
            eprintln!("  {:<10} FAILED: {:?}", stage.program(), r.outcome);
        }
        // 失败时也要列：CoLM 恰恰会先悄悄改掉配置，然后死在别处。
        for o in &r.overrides {
            println!("             {}", o.text);
        }
        if !r.succeeded() {
            eprintln!("  log: {}", r.log.display());
            bail!("stage {} failed", stage.program());
        }
    }

    // history 文件：唯一一个 *_hist_*.nc
    let hist_dir = out.join("history");
    let mut hists: Vec<PathBuf> = fs::read_dir(&hist_dir)
        .with_context(|| format!("no history dir at {}", hist_dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains("_hist_") && n.ends_with(".nc"))
        })
        .collect();
    hists.sort();
    if hists.len() != 1 {
        bail!(
            "expected exactly one history file, found {}: {hists:?}",
            hists.len()
        );
    }
    let produced = &hists[0];
    println!("  history: {}", produced.display());

    let golden = repo
        .join("oracle/golden")
        .join(produced.file_name().unwrap());
    if write_golden {
        fs::create_dir_all(golden.parent().unwrap())?;
        fs::copy(produced, &golden)?;
        println!("  wrote golden: {}", golden.display());
        // 黄金文件的字节由 Fortran 侧写出，而 kernels/ 是 gitignore 的，
        // 所以「是什么工具链产出了这些字节」在仓库里本来没有任何记录。
        // 工具链一变（Homebrew 升 gcc、conda 升 netcdf），比对会在全部变量上
        // 全红，而没人分得清是物理改了还是编译器改了。把 manifest 一并入库。
        let src = kernel.dir.join("manifest.json");
        let dst = repo.join("oracle/golden/kernel-manifest.json");
        fs::copy(&src, &dst)?;
        println!("  wrote provenance: {}", dst.display());
    } else {
        println!(
            "  compare with: golden-compare {} {}",
            golden.display(),
            produced.display()
        );
    }
    Ok(())
}

/// 把当前内核的清单与产出黄金文件时入库的那份对照。
///
/// 这**不是** `Kernel::open` 的重复。`Kernel::open` 问的是「二进制和紧挨着它的
/// 清单对得上吗」，只比 `sha256`，不符即失败；本函数问的是「当前内核和产出黄金
/// 文件的那个是同一配置吗」，比的是可复现的字段，不符只警告。
///
/// 刻意不比 `sha256`：Fortran 构建不逐字节可复现（实测同一路径连跑两次，三个
/// 二进制的摘要全变），拿它比配置身份只会永远告警。只警告不失败也是刻意的：
/// 工具链变了未必意味着结果错了，但必须让人看见，否则一场全部变量全红的
/// 比对会被误读成物理回归。
fn check_kernel_provenance(repo: &Path, have: &Manifest) -> Result<()> {
    let recorded = repo.join("oracle/golden/kernel-manifest.json");
    if !recorded.exists() {
        return Ok(()); // 还没产出过黄金文件
    }
    let text = fs::read_to_string(&recorded)?;
    // 读不动入库的那份不该让整场回归失败 —— 它是一条历史记录，不是门禁。
    let want: Manifest = match serde_json::from_str(&text) {
        Ok(m) => m,
        Err(e) => {
            eprintln!(
                "  WARNING: cannot read the recorded kernel manifest at {}: {e}",
                recorded.display()
            );
            eprintln!("    provenance was not checked");
            return Ok(());
        }
    };

    let mut drift = Vec::new();
    let mut cmp = |key: &str, a: &str, b: &str| {
        if a != b {
            drift.push(format!("{key}: recorded {a:?}, current {b:?}"));
        }
    };
    cmp("preset", &want.preset, &have.preset);
    cmp("platform", &want.platform, &have.platform);
    cmp("colm_git_sha", &want.colm_git_sha, &have.colm_git_sha);
    cmp("generator_args", &want.generator_args, &have.generator_args);
    cmp("build_profile", &want.build_profile, &have.build_profile);
    cmp("built_with", &want.built_with, &have.built_with);
    cmp("netcdf_c", &want.netcdf_c, &have.netcdf_c);
    cmp("netcdf_fortran", &want.netcdf_fortran, &have.netcdf_fortran);
    cmp("hdf5", &want.hdf5, &have.hdf5);
    if want.schema != have.schema {
        drift.push(format!(
            "schema: recorded {}, current {}",
            want.schema, have.schema
        ));
    }
    if want.macros != have.macros {
        drift.push(format!(
            "macros: recorded {:?}, current {:?}",
            want.macros, have.macros
        ));
    }

    if drift.is_empty() {
        println!("  provenance matches the recorded kernel");
    } else {
        eprintln!("  WARNING: kernel differs from the one that produced the golden files:");
        for d in &drift {
            eprintln!("    {d}");
        }
        eprintln!("    a comparison failure below may be toolchain drift, not a physics change");
    }
    Ok(())
}

fn repo_root() -> Result<PathBuf> {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()?;
    if !out.status.success() {
        bail!("not inside a git repository");
    }
    Ok(PathBuf::from(String::from_utf8(out.stdout)?.trim()))
}

fn land_cover_label(nml: &Path) -> Result<String> {
    let text = fs::read_to_string(nml).with_context(|| format!("cannot read {}", nml.display()))?;
    let doc =
        colm_namelist::parse(&text).with_context(|| format!("cannot parse {}", nml.display()))?;
    let year = match doc.get("DEF_LC_YEAR") {
        Some(colm_namelist::Value::Int(value)) => *value,
        Some(other) => bail!("DEF_LC_YEAR is {other:?}, not an integer"),
        None => match colm_schema::find("DEF_LC_YEAR").map(|field| field.default) {
            Some(colm_schema::Default::Integer(value)) => value,
            _ => bail!("DEF_LC_YEAR default is missing from the generated schema"),
        },
    };
    if !(0..=9999).contains(&year) {
        bail!("DEF_LC_YEAR {year} cannot be formatted as a four-digit land-cover year");
    }
    Ok(format!("lc{year:04}"))
}

/// 算例名决定所有产物路径，所以取错了会一路错到「找不到 history」。
/// 用真解析器而不是字符串查找：后者会被一行注释掉的 `DEF_CASE_NAME` 骗过去。
fn read_case_name(nml: &Path) -> Result<String> {
    let text = fs::read_to_string(nml).with_context(|| format!("cannot read {}", nml.display()))?;
    let doc =
        colm_namelist::parse(&text).with_context(|| format!("cannot parse {}", nml.display()))?;
    match doc.get("DEF_CASE_NAME") {
        Some(colm_namelist::Value::Str(s)) => Ok(s.clone()),
        Some(other) => bail!("DEF_CASE_NAME is {other:?}, not a string"),
        None => bail!("no DEF_CASE_NAME in {}", nml.display()),
    }
}

/// 校验外部 PLUMBER2 文件。换了份数据就该在这里炸，而不是等黄金文件对不上。
fn verify_inputs(repo: &Path, plumber2: &Path) -> Result<()> {
    let manifest = fs::read_to_string(repo.join("oracle/fixtures/inputs.sha256"))?;
    for line in manifest.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (want, rel) = line.split_once("  ").context("malformed sha256 line")?;
        let path = plumber2.join(rel);
        let bytes = fs::read(&path)
            .with_context(|| format!("cannot read {} to hash it", path.display()))?;
        let got = sha256_hex(&bytes);
        if got != want {
            bail!(
                "input checksum mismatch for {}\n  expected {want}\n  got      {got}",
                path.display()
            );
        }
    }
    println!("  inputs verified");
    Ok(())
}

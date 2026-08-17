//! 跑一个黄金算例的三段，用 colm-kernel 判成败。
//!
//! 用法: golden-run <case-name> [--kernel <dir>] [--write-golden]
//!
//! 环境变量 PLUMBER2_ROOT 必须指向含 Forcing/ Sitedata/ Observation/ 的目录。

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use colm_kernel::outcome::{adjudicate, Outcome, Stage};
use sha2::{Digest, Sha256};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let case = args
        .next()
        .context("usage: golden-run <case-name> [--kernel <dir>] [--write-golden]")?;
    let mut kernel = PathBuf::from("kernels/waterheat");
    let mut write_golden = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--kernel" => kernel = PathBuf::from(args.next().context("--kernel needs a path")?),
            "--write-golden" => write_golden = true,
            other => bail!("unknown argument: {other}"),
        }
    }

    let repo = repo_root()?;
    let plumber2 =
        PathBuf::from(std::env::var("PLUMBER2_ROOT").context("PLUMBER2_ROOT is not set")?);

    verify_inputs(&repo, &plumber2)?;
    verify_kernel(&repo.join(&kernel))?;
    check_kernel_provenance(&repo, &repo.join(&kernel))?;

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

    let case_name = read_case_name(&work.join("case.nml"))?;
    let out = work.join("out").join(&case_name);

    // mkinidata 的产物必须列到**文件**，不能只列 restart/const 目录：
    // adjudicate 用的是 Path::exists，而目录在 mkinidata 写任何东西之前就已存在，
    // 于是「跑完了但什么都没写」——正是产物校验这条腿存在的理由——恰好抓不到。
    // 两个文件名见 design.md §6.2；block 后缀实测是 _w180_s90。
    let lc = "lc2005";
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
        let exe = repo.join(&kernel).join(format!("{}.x", stage.program()));
        let output = Command::new(&exe)
            .arg(work.join("case.nml"))
            .current_dir(&work)
            .output()
            .with_context(|| format!("failed to spawn {}", exe.display()))?;

        // stdout 与 stderr 都要收。gfortran 运行时的错误只走 stderr，
        // 所以 FAILURE_MARKERS 里的 `Fortran runtime error` 与 `Error termination`
        // 在只读 stdout 时**永远不可能命中**；实测 namelist 文件缺失时
        // stdout 是 0 字节而 stderr 有 302 字节，日志会空得看不出原因。
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.is_empty() {
            text.push_str("\n--- stderr ---\n");
            text.push_str(&stderr);
        }
        let log = work.join(format!("{}.log", stage.program()));
        fs::write(&log, text.as_bytes())?;

        let verdict = adjudicate(*stage, output.status.code(), &text, artifacts);
        match verdict {
            Outcome::Succeeded => println!("  {:<10} ok", stage.program()),
            Outcome::Failed(f) => {
                eprintln!("  {:<10} FAILED: {f:?}", stage.program());
                eprintln!("  log: {}", log.display());
                bail!("stage {} failed", stage.program());
            }
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
        // 工具链一变（Homebrew 升 gcc、conda 升 netcdf），比对会在 129 个变量上
        // 全红，而没人分得清是物理改了还是编译器改了。把 manifest 一并入库。
        let src = repo.join(&kernel).join("manifest.json");
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

/// 把当前内核的 manifest 与产出黄金文件时入库的那份对照。
///
/// 只比**可复现**的字段。`sha256` 每次构建都不同（实测：同一路径连跑两次，
/// 三个二进制的摘要全变），它认定的是完整性而非配置身份，所以不参与比较。
/// 不匹配只警告不失败：工具链变了未必意味着结果错了，但必须让人看见，
/// 否则一场 129 个变量全红的比对会被误读成物理回归。
fn check_kernel_provenance(repo: &Path, kernel_dir: &Path) -> Result<()> {
    let recorded = repo.join("oracle/golden/kernel-manifest.json");
    if !recorded.exists() {
        return Ok(()); // 还没产出过黄金文件
    }
    let want = fs::read_to_string(&recorded)?;
    let have = fs::read_to_string(kernel_dir.join("manifest.json"))?;

    let mut drift = Vec::new();
    for key in [
        "preset",
        "platform",
        "colm_git_sha",
        "generator_args",
        "built_with",
        "netcdf_c",
        "netcdf_fortran",
        "hdf5",
    ] {
        let (a, b) = (
            extract_json_string(&want, key),
            extract_json_string(&have, key),
        );
        if a != b {
            drift.push(format!("{key}: recorded {a:?}, current {b:?}"));
        }
    }
    let (ma, mb) = (
        extract_json_array(&want, "macros"),
        extract_json_array(&have, "macros"),
    );
    if ma != mb {
        drift.push(format!("macros: recorded {ma:?}, current {mb:?}"));
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

/// 从 manifest 里取 `"key": [ ... ]` 的原文。与 `extract_json_string` 一样，
/// 刻意不引入 serde_json：manifest 是我们自己按固定格式生成的。
fn extract_json_array(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":");
    let start = text.find(&needle)? + needle.len();
    let rest = &text[start..];
    let open = rest.find('[')?;
    let close = rest[open..].find(']')? + open;
    Some(rest[open..=close].to_string())
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

/// `DEF_CASE_NAME = 'X'` -> `X`。够用即止：Plan 2 的 colm-namelist 会做完整解析。
fn read_case_name(nml: &Path) -> Result<String> {
    let text = fs::read_to_string(nml)?;
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("DEF_CASE_NAME") {
            if let Some(v) = rest.split('=').nth(1) {
                return Ok(v.trim().trim_matches('\'').trim_matches('"').to_string());
            }
        }
    }
    bail!("DEF_CASE_NAME not found in {}", nml.display())
}

fn sha256_file(p: &Path) -> Result<String> {
    let mut f = fs::File::open(p).with_context(|| format!("cannot open {}", p.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
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
        let got = sha256_file(&path)?;
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

/// 校验内核。「不存在」和「存在但不是我们构建的那个」是两种不同的情况。
fn verify_kernel(dir: &Path) -> Result<()> {
    let manifest_path = dir.join("manifest.json");
    if !manifest_path.exists() {
        bail!(
            "no kernel manifest at {}\n  run: ./oracle/scripts/build_kernel.sh waterheat",
            manifest_path.display()
        );
    }
    let text = fs::read_to_string(&manifest_path)?;
    for prog in ["mksrfdata", "mkinidata", "colm"] {
        let exe = dir.join(format!("{prog}.x"));
        if !exe.exists() {
            bail!("kernel manifest present but {} is missing", exe.display());
        }
        let want = extract_json_string(&text, prog)
            .with_context(|| format!("manifest has no sha256 for {prog}"))?;
        let got = sha256_file(&exe)?;
        if got != want {
            bail!(
                "kernel binary {} does not match its manifest\n  expected {want}\n  got      {got}\n  rebuild with: ./oracle/scripts/build_kernel.sh",
                exe.display()
            );
        }
    }
    println!("  kernel verified ({})", dir.display());
    Ok(())
}

/// 从 manifest.json 里取 `"key": "value"`。刻意不引入 serde_json：
/// manifest 是我们自己按固定格式生成的，两行字符串查找足够，且少一个依赖。
fn extract_json_string(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":");
    let start = text.find(&needle)? + needle.len();
    let rest = &text[start..];
    let q1 = rest.find('"')? + 1;
    let q2 = rest[q1..].find('"')? + q1;
    Some(rest[q1..q2].to_string())
}

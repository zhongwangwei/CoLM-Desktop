//! Bounded, names-only spatial forcing inventory, shared by local and SSH paths.
use super::Opts;
use anyhow::{ensure, Context, Result};
use colm_case::forcing_templates::{forcing_directory_candidates, forcing_directory_parents};
use colm_remote::ssh::{quote, Ssh};
use std::{collections::BTreeSet, path::Path};

const ENTRY_LIMIT: usize = 20_000;
#[derive(serde::Deserialize, serde::Serialize, Debug, PartialEq)]
struct Inventory {
    files: BTreeSet<String>,
    truncated: bool,
}

fn local_inventory(path: &Path, parents: &[String], limit: usize) -> Result<Inventory> {
    ensure!(
        path.is_dir(),
        "Expected a forcing data directory: {}",
        path.display()
    );
    let mut result = Inventory {
        files: BTreeSet::new(),
        truncated: false,
    };
    let mut seen = 0;
    for parent in parents {
        let entries = match std::fs::read_dir(path.join(parent)) {
            Ok(entries) => entries,
            Err(e)
                if !parent.is_empty()
                    && matches!(
                        e.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                    ) =>
            {
                continue
            }
            Err(e) => {
                return Err(e).with_context(|| {
                    format!(
                        "Cannot inspect forcing directory {}",
                        path.join(parent).display()
                    )
                })
            }
        };
        for entry in entries {
            if seen >= limit {
                result.truncated = true;
                return Ok(result);
            }
            seen += 1;
            let entry = entry?;
            let metadata = match std::fs::metadata(entry.path()) {
                Ok(metadata) => metadata,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                    ) =>
                {
                    continue
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("Cannot inspect forcing file {}", entry.path().display())
                    })
                }
            };
            if metadata.is_file() {
                if let Some(name) = entry.file_name().to_str() {
                    result.files.insert(if parent.is_empty() {
                        name.to_owned()
                    } else {
                        format!("{parent}/{name}")
                    });
                }
            }
        }
    }
    Ok(result)
}

const INVENTORY_PY: &str = r#"import json, os, sys
path, parents, limit = sys.argv[1], json.loads(sys.argv[2]), int(sys.argv[3])
if not os.path.isdir(path):
    raise ValueError('Expected a forcing data directory: ' + path)
files, seen, truncated = [], 0, False
for parent in parents:
    try:
        entries = os.scandir(os.path.join(path, parent))
    except (FileNotFoundError, NotADirectoryError):
        if not parent:
            raise
        continue
    with entries:
        for entry in entries:
            if seen >= limit:
                truncated = True
                break
            seen += 1
            if entry.is_file():
                files.append(parent + '/' + entry.name if parent else entry.name)
    if truncated:
        break
print(json.dumps(dict(files=sorted(files), truncated=truncated)))
"#;

fn remote_script(path: &str, parents: &[String], limit: usize) -> Result<String> {
    ensure!(
        path.starts_with('/') && !path.chars().any(char::is_control),
        "Expected an absolute server path without control characters"
    );
    Ok(format!(
        "python3 - {} {} {} <<'COLM_FORCING_INVENTORY_PY'\n{}\nCOLM_FORCING_INVENTORY_PY\n",
        quote(path),
        quote(&serde_json::to_string(parents)?),
        limit,
        INVENTORY_PY
    ))
}

#[derive(serde::Deserialize, serde::Serialize)]
struct CoverageResult {
    coverage: Option<crate::forcing_coverage::Coverage>,
    coverage_error: Option<String>,
}

const REMOTE_COVERAGE_PY: &str = r#"import json, os, subprocess, sys
root, path, dataset = sys.argv[1:]
def failure(cli, reason):
    print(json.dumps(dict(coverage=None, coverage_error=cli + ': ' + reason[:800] + '；请手工填写日期。')))
    sys.exit(0)
engines = []
try:
    with os.scandir(os.path.join(root, 'engine')) as entries:
        for index, entry in enumerate(entries):
            if index >= 128:
                break
            cli = os.path.join(entry.path, 'bin', 'colm-cli')
            if entry.is_dir(follow_symlinks=False) and os.path.isfile(cli) and os.access(cli, os.X_OK):
                engines.append((os.stat(cli).st_mtime, cli))
except FileNotFoundError:
    pass
for _, cli in sorted(engines, reverse=True)[:3]:
    try:
        result = subprocess.run([cli, 'forcing-directory-probe', '--path', path, '--dataset', dataset], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=20)
        if result.returncode:
            detail = result.stderr.strip() or result.stdout.strip()
            if 'unknown command' in detail.lower() and 'forcing-directory-probe' in detail:
                continue
            failure(cli, 'exit ' + str(result.returncode) + ': ' + detail)
        value = json.loads(result.stdout.strip().splitlines()[-1])
        if not isinstance(value, dict):
            failure(cli, 'invalid coverage response')
        if 'coverage' in value and 'coverage_error' in value:
            print(json.dumps(dict(coverage=value['coverage'], coverage_error=value['coverage_error'])))
            sys.exit(0)
        if 'candidates' not in value:
            failure(cli, 'missing coverage fields in response')
    except subprocess.TimeoutExpired:
        failure(cli, 'time-coordinate check timed out after 20 seconds')
    except (OSError, ValueError, IndexError) as error:
        failure(cli, str(error))
print(json.dumps(dict(coverage=None, coverage_error='服务器尚无支持时间检查的新版 colm-cli，或检查超时；请手工填写日期。目录识别仍可使用。')))
"#;

fn remote_coverage_script(root: &str, path: &str, dataset: &str) -> Result<String> {
    for value in [root, path] {
        ensure!(
            value.starts_with('/') && !value.chars().any(char::is_control),
            "expected absolute server paths without control characters"
        );
    }
    Ok(format!(
        "python3 - {} {} {} <<'COLM_FORCING_COVERAGE_PY'\n{}\nCOLM_FORCING_COVERAGE_PY\n",
        quote(root),
        quote(path),
        quote(dataset),
        REMOTE_COVERAGE_PY
    ))
}

fn coverage_result(
    opts: &Opts,
    path: &str,
    inventory: &Inventory,
    candidates: &[String],
) -> CoverageResult {
    let calculate = || -> Result<CoverageResult> {
        ensure!(
            !inventory.truncated,
            "目录清单已达到检查上限，请手工填写日期"
        );
        let explicit = opts.get("--dataset");
        let selected = explicit
            .as_deref()
            .filter(|v| !v.is_empty())
            .or_else(|| (candidates.len() == 1).then(|| candidates[0].as_str()))
            .context("请选择已识别的数据集，再确认可用时段")?;
        ensure!(
            candidates.iter().any(|v| v == selected),
            "所选模板与目录中的完整文件组不匹配，请手工填写日期"
        );
        if let Some(host) = opts.get("--host") {
            let root = opts
                .get("--root")
                .context("服务器工作目录未设置，无法检查时间坐标；请手工填写日期")?;
            let output =
                Ssh::new(&host)?.run_ok(&remote_coverage_script(&root, path, selected)?)?;
            let result: CoverageResult = serde_json::from_str(&output)?;
            ensure!(
                result
                    .coverage
                    .as_ref()
                    .is_none_or(|coverage| coverage.dataset == selected),
                "server returned coverage for a different template"
            );
            Ok(result)
        } else {
            Ok(CoverageResult {
                coverage: Some(crate::forcing_coverage::probe(
                    Path::new(path),
                    selected,
                    &inventory.files,
                )?),
                coverage_error: None,
            })
        }
    };
    calculate().unwrap_or_else(|error| CoverageResult {
        coverage: None,
        coverage_error: Some(format!("无法确认可用时段，请手工填写日期：{error:#}")),
    })
}

pub(super) fn run(opts: &Opts) -> Result<()> {
    let path = opts.need_str("--path")?;
    let parents = forcing_directory_parents()?;
    let inventory = if let Some(host) = opts.get("--host") {
        let output = Ssh::new(&host)?.run_ok(&remote_script(&path, &parents, ENTRY_LIMIT)?)?;
        serde_json::from_str::<Inventory>(&output).context("Invalid server forcing inventory")?
    } else {
        local_inventory(Path::new(&path), &parents, ENTRY_LIMIT)?
    };
    let candidates = forcing_directory_candidates(&inventory.files)?;
    let result = coverage_result(opts, &path, &inventory, &candidates);
    println!(
        "{}",
        serde_json::json!({"path":path,"candidates":candidates,"truncated":inventory.truncated,"coverage":result.coverage,"coverage_error":result.coverage_error})
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsafe_remote_paths_are_rejected() {
        for path in ["", "relative", "/data\ncommand"] {
            assert!(remote_script(path, &[], 10).is_err());
        }
        assert!(remote_script("/data/气象 'x'", &[], 10).is_ok());
    }
    #[cfg(unix)]
    #[test]
    fn local_and_remote_inventory_agree_and_are_bounded() {
        let root = std::env::temp_dir().join(format!(
            "colm-forcing-probe-{}-气象 'x'",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("tas")).unwrap();
        std::fs::write(root.join("tas/a.nc"), []).unwrap();
        std::fs::write(root.join("tas/b.nc"), []).unwrap();
        std::fs::create_dir_all(root.join("unrelated/deep")).unwrap();
        std::fs::write(root.join("unrelated/deep/ignored.nc"), []).unwrap();
        std::os::unix::fs::symlink(root.join("tas/a.nc"), root.join("tas/link.nc")).unwrap();
        std::os::unix::fs::symlink(root.join("absent.nc"), root.join("tas/dangling.nc")).unwrap();
        std::os::unix::fs::symlink(root.join("tas"), root.join("alias")).unwrap();
        std::fs::write(root.join("collision"), []).unwrap();
        let parents = vec![
            "tas".into(),
            "missing".into(),
            "alias".into(),
            "collision".into(),
        ];
        let local = local_inventory(&root, &parents, 20).unwrap();
        let output = std::process::Command::new("sh")
            .args([
                "-c",
                &remote_script(root.to_str().unwrap(), &parents, 20).unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            local,
            serde_json::from_slice::<Inventory>(&output.stdout).unwrap()
        );
        assert_eq!(local.files.len(), 6);
        assert!(local.files.contains("tas/link.nc"));
        assert!(local.files.contains("alias/link.nc"));
        assert!(!local.files.contains("tas/dangling.nc"));
        assert!(local_inventory(&root, &parents, 1).unwrap().truncated);
        let output = std::process::Command::new("sh")
            .args([
                "-c",
                &remote_script(root.to_str().unwrap(), &parents, 1).unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            serde_json::from_slice::<Inventory>(&output.stdout)
                .unwrap()
                .truncated
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn flat_dataset_survives_unrelated_template_parent_file() {
        let root = std::env::temp_dir().join(format!("colm-flat-forcing-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        for prefix in ["Temp", "SpecHum", "Pres", "P", "Wind", "SWd", "LWd"] {
            std::fs::write(root.join(format!("{prefix}_2003_07.nc")), []).unwrap();
        }
        std::fs::write(root.join("tas"), []).unwrap();
        let parents = forcing_directory_parents().unwrap();
        let local = local_inventory(&root, &parents, ENTRY_LIMIT).unwrap();
        let output = std::process::Command::new("sh")
            .args([
                "-c",
                &remote_script(root.to_str().unwrap(), &parents, ENTRY_LIMIT).unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            local,
            serde_json::from_slice::<Inventory>(&output.stdout).unwrap()
        );
        assert_eq!(
            forcing_directory_candidates(&local.files).unwrap(),
            ["MSWX"]
        );
        assert!(local_inventory(&root.join("tas"), &parents, ENTRY_LIMIT).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn optional_coverage_failure_does_not_change_layout_result() {
        let opts = Opts::parse(&["--path".into(), "/absent".into()]).unwrap();
        let inventory = Inventory {
            files: BTreeSet::new(),
            truncated: false,
        };
        let result = coverage_result(&opts, "/absent", &inventory, &["MSWX".into()]);
        assert!(result.coverage.is_none());
        assert!(result.coverage_error.is_some());
        assert!(coverage_result(
            &opts,
            "/absent",
            &Inventory {
                files: BTreeSet::new(),
                truncated: true
            },
            &["MSWX".into()]
        )
        .coverage_error
        .unwrap()
        .contains("上限"));
    }

    #[cfg(unix)]
    #[test]
    fn remote_coverage_reuses_deployed_cli_without_python_netcdf_or_building() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!(
            "colm-coverage-server-{}-气象 'x'",
            std::process::id()
        ));
        let bin = root.join("engine/test/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let cli = bin.join("colm-cli");
        std::fs::write(&cli,r#"#!/bin/sh
printf '%s\n' '{"coverage":{"start":"2003-01-02","end":"2003-01-30","dataset":"MSWX"},"coverage_error":null}'
"#).unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        let script =
            remote_coverage_script(root.to_str().unwrap(), "/data/气象 'x'", "MSWX").unwrap();
        let run = || {
            let output = std::process::Command::new("sh")
                .args(["-c", &script])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            serde_json::from_slice::<CoverageResult>(&output.stdout).unwrap()
        };
        assert_eq!(run().coverage.unwrap().dataset, "MSWX");
        std::fs::write(
            &cli,
            "#!/bin/sh\nprintf '%s\\n' '{\"candidates\":[\"MSWX\"]}'\n",
        )
        .unwrap();
        assert!(run().coverage_error.unwrap().contains("新版"));
        std::fs::write(
            &cli,
            "#!/bin/sh\necho 'libnetcdf.so missing' >&2\nexit 127\n",
        )
        .unwrap();
        let failure = run().coverage_error.unwrap();
        assert!(failure.contains("libnetcdf.so missing"));
        assert!(failure.contains("exit 127"));
        assert!(failure.contains(cli.to_str().unwrap()));
        std::fs::remove_dir_all(root).unwrap();
        assert!(remote_coverage_script("relative", "/data", "MSWX").is_err());
    }
}

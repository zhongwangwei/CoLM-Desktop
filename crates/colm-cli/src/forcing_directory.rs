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
    println!(
        "{}",
        serde_json::json!({"path": path, "candidates": candidates, "truncated": inventory.truncated})
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
}

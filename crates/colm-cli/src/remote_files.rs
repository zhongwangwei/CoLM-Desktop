//! Read-only server filesystem browsing, using the configured SSH transport.
use super::Opts;
use anyhow::{ensure, Context, Result};
use colm_remote::ssh::{quote, Ssh};

const LIST_DIRECTORY: &str = r#"import json, os, sys, unicodedata
def safe(text):
    return not any(unicodedata.category(c) in ('Cc', 'Cs') for c in text)
try:
    path = sys.argv[1] or os.path.expanduser('~')
    if not os.path.isabs(path) or not safe(path):
        raise ValueError('Expected an absolute server path without control characters')
    path = os.path.normpath(path)
    entries = []
    skipped = 0
    truncated = False
    with os.scandir(path) as listing:
        for index, entry in enumerate(listing):
            if index >= 5000:
                truncated = True
                break
            if not safe(entry.name):
                skipped += 1
                continue
            entries.append(dict(name=entry.name, path=os.path.join(path, entry.name), directory=entry.is_dir()))
    entries.sort(key=lambda entry: (not entry['directory'], entry['name'].casefold(), entry['name']))
    parent = os.path.dirname(path)
    print(json.dumps(dict(path=path, parent=None if parent == path else parent, entries=entries, truncated=truncated, skipped=skipped)))
except (OSError, ValueError) as error:
    print('Cannot browse server directory: ' + str(error), file=sys.stderr)
    sys.exit(1)
"#;

fn browse_script(path: &str) -> Result<String> {
    ensure!(
        path.is_empty() || (path.starts_with('/') && !path.chars().any(char::is_control)),
        "Expected an absolute server path without control characters"
    );
    Ok(format!(
        "python3 - {} <<'COLM_BROWSE_PY'\n{}\nCOLM_BROWSE_PY\n",
        quote(path),
        LIST_DIRECTORY
    ))
}

const CHECK_PATHS: &str = r#"import json, os, stat, sys
try:
    checks = json.loads(sys.argv[1])
    for check in checks:
        path = check['path']
        try:
            mode = os.stat(path).st_mode
        except OSError as error:
            raise ValueError('Cannot access server path ' + repr(path) + ': ' + str(error))
        expected = 'directory' if check['directory'] else 'regular file'
        if not (stat.S_ISDIR(mode) if check['directory'] else stat.S_ISREG(mode)):
            raise ValueError('Expected server ' + expected + ': ' + repr(path))
    print(json.dumps(dict(checked=len(checks))))
except (OSError, ValueError) as error:
    print('Server input validation failed: ' + str(error), file=sys.stderr)
    sys.exit(1)
"#;

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct PathCheck {
    path: String,
    directory: bool,
}

fn check_paths_script(raw: &str) -> Result<String> {
    ensure!(raw.len() <= 65536, "Server path checks exceed 64 KiB");
    let checks: Vec<PathCheck> = serde_json::from_str(raw).context("Invalid server path checks")?;
    ensure!(
        checks.len() <= 256,
        "At most 256 server paths can be checked at once"
    );
    for check in &checks {
        ensure!(
            check.path.starts_with('/') && !check.path.chars().any(char::is_control),
            "Expected an absolute server path without control characters: {:?}",
            check.path
        );
    }
    Ok(format!(
        "python3 - {} <<'COLM_CHECK_PATHS_PY'\n{}\nCOLM_CHECK_PATHS_PY\n",
        quote(&serde_json::to_string(&checks)?),
        CHECK_PATHS
    ))
}

pub(super) fn check_paths(opts: &Opts) -> Result<()> {
    let script = check_paths_script(&opts.need_str("--checks")?)?;
    let output = Ssh::new(&opts.need_str("--host")?)?.run_ok(&script)?;
    let value: serde_json::Value =
        serde_json::from_str(&output).context("Invalid server path check result")?;
    println!("{value}");
    Ok(())
}

pub(super) fn run(opts: &Opts) -> Result<()> {
    let script = browse_script(&opts.get("--path").unwrap_or_default())?;
    let output = Ssh::new(&opts.need_str("--host")?)?.run_ok(&script)?;
    let value: serde_json::Value =
        serde_json::from_str(&output).context("Invalid server directory listing")?;
    println!("{value}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::process::{Command, Output};

    #[cfg(unix)]
    fn execute(path: &str) -> Output {
        Command::new("bash")
            .args(["-c", &browse_script(path).unwrap()])
            .output()
            .unwrap()
    }

    #[test]
    fn rejects_non_absolute_and_control_paths() {
        for path in [
            "relative",
            "~/data",
            "-option",
            "/tmp/\nvalue",
            "/tmp/\0",
            "/tmp/\u{85}",
        ] {
            assert!(browse_script(path).is_err(), "{path:?}");
        }
        assert!(browse_script("").is_ok());
        assert!(browse_script("/").is_ok());
    }

    #[test]
    #[cfg(unix)]
    fn empty_path_uses_home_and_large_listings_report_truncation() {
        let root = std::env::temp_dir().join(format!("colm-browse-limit-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        for index in 0..5001 {
            std::fs::write(root.join(format!("{index}.nc")), "").unwrap();
        }
        let output = Command::new("bash")
            .args(["-c", &browse_script("").unwrap()])
            .env("HOME", &root)
            .output()
            .unwrap();
        assert!(output.status.success());
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["path"], root.to_str().unwrap());
        assert_eq!(value["entries"].as_array().unwrap().len(), 5000);
        assert_eq!(value["truncated"], true);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn lists_unicode_symlinks_and_shell_metacharacters_without_execution() {
        let root = std::env::temp_dir().join(format!("colm-browse-{}", std::process::id()));
        let path = root.join("数据 ' $(touch INJECTED) `touch INJECTED` ;");
        std::fs::create_dir_all(path.join("directory")).unwrap();
        std::fs::write(path.join("file.nc"), "").unwrap();
        std::fs::write(path.join("bad\nname"), "").unwrap();
        std::os::unix::fs::symlink("directory", path.join("linked")).unwrap();
        let output = Command::new("bash")
            .args(["-c", &browse_script(path.to_str().unwrap()).unwrap()])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(output.status.success(), "{:?}", output);
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["path"], path.to_str().unwrap());
        assert_eq!(value["skipped"], 1);
        assert_eq!(value["truncated"], false);
        assert_eq!(value["entries"][1]["name"], "linked");
        assert_eq!(value["entries"][1]["directory"], true);
        assert_eq!(value["entries"][2]["directory"], false);
        assert!(!root.join("INJECTED").exists());
        assert!(!execute(path.join("missing").to_str().unwrap())
            .status
            .success());
        assert!(!execute(path.join("file.nc").to_str().unwrap())
            .status
            .success());
        let root_listing: serde_json::Value = serde_json::from_slice(&execute("/").stdout).unwrap();
        assert!(root_listing["parent"].is_null());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn validates_batched_check_requests_before_ssh() {
        for raw in [
            "{}",
            r#"[{"path":"/tmp","directory":"true"}]"#,
            r#"[{"path":"relative","directory":true}]"#,
            r#"[{"path":"/tmp/\n","directory":true}]"#,
            r#"[{"path":"/tmp","directory":true,"extra":1}]"#,
        ] {
            assert!(check_paths_script(raw).is_err(), "{raw}");
        }
        let many = serde_json::to_string(&vec![
            serde_json::json!({"path":"/tmp", "directory":true});
            257
        ])
        .unwrap();
        assert!(check_paths_script(&many).is_err());
        assert!(check_paths_script(&" ".repeat(65537)).is_err());
        assert!(check_paths_script("[]").is_ok());
    }

    #[test]
    #[cfg(unix)]
    fn checks_files_directories_missing_types_and_shell_quotes() {
        let root = std::env::temp_dir().join(format!("colm-check-paths-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("数据 ' $(touch INJECTED) `touch INJECTED` ;.nc");
        std::fs::write(&file, "fixture").unwrap();
        let execute_checks = |checks: serde_json::Value| {
            Command::new("bash")
                .args(["-c", &check_paths_script(&checks.to_string()).unwrap()])
                .current_dir(&root)
                .output()
                .unwrap()
        };
        let good = execute_checks(serde_json::json!([
            {"path":root, "directory":true}, {"path":file, "directory":false}
        ]));
        assert!(good.status.success(), "{:?}", good);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&good.stdout).unwrap()["checked"],
            2
        );
        assert!(!root.join("INJECTED").exists());
        for (path, directory, expected) in [
            (root.join("missing"), false, "Cannot access server path"),
            (root.clone(), false, "Expected server regular file"),
            (file.clone(), true, "Expected server directory"),
        ] {
            let output = execute_checks(serde_json::json!([{"path":path,"directory":directory}]));
            assert!(!output.status.success());
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(error.contains(expected), "{error}");
            assert!(error.contains(path.to_str().unwrap()), "{error}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

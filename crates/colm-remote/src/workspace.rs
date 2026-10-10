//! Pinned developer sources and fixed remote workspace operations.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Component, Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub action: String,
    #[serde(default = "preset")]
    pub preset: String,
    pub kind: Option<String>,
    pub package: Option<String>,
    pub case: Option<String>,
    pub engine: Option<String>,
    pub rtol: Option<f64>,
    pub atol: Option<f64>,
    pub first_records: Option<usize>,
    pub ignore: Option<Vec<String>>,
}
fn preset() -> String {
    "default".into()
}
impl Request {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            [
                "build-engine",
                "build-kernel",
                "test",
                "run",
                "parity",
                "regress",
                "verify"
            ]
            .contains(&self.action.as_str()),
            "unknown remote workspace action"
        );
        for s in [
            Some(&self.preset),
            self.kind.as_ref(),
            self.package.as_ref(),
            self.case.as_ref(),
            self.engine.as_ref(),
        ]
        .into_iter()
        .flatten()
        .chain(self.ignore.iter().flatten())
        {
            ensure!(
                !s.contains(['\r', '\n', '\0']),
                "arguments cannot contain control characters"
            );
        }
        ensure!(
            self.rtol
                .into_iter()
                .chain(self.atol)
                .all(|n| n.is_finite() && n >= 0.0),
            "tolerances must be finite and nonnegative"
        );
        ensure!(
            self.first_records != Some(0),
            "first_records must be positive"
        );
        if matches!(
            self.action.as_str(),
            "run" | "parity" | "regress" | "verify"
        ) {
            check_absolute(
                self.case
                    .as_deref()
                    .context("this action needs an existing remote case directory")?,
            )?;
        }
        if let Some(engine) = &self.engine {
            ensure!(
                ["rust", "fortran"].contains(&engine.as_str()),
                "unknown engine"
            );
        }
        if let Some(package) = &self.package {
            ensure!(
                !package.is_empty()
                    && package
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
                "invalid package"
            );
        }
        Ok(())
    }
}
pub fn check_absolute(s: &str) -> Result<()> {
    ensure!(
        s.starts_with('/')
            && s != "/"
            && !s.contains(['\r', '\n', '\0'])
            && !s.split('/').any(|p| p == ".."),
        "use an absolute remote directory without parent traversal"
    );
    Ok(())
}
pub fn safe_source_path(path: &Path) -> Result<()> {
    ensure!(
        path.components().all(|c| matches!(c, Component::Normal(_))),
        "unsafe archive path"
    );
    let text = path.to_str().context("non-UTF8 source path")?;
    ensure!(!text.contains(['\r', '\n', '\0']), "unsafe source path");
    for part in text.split('/') {
        let p = part.to_ascii_lowercase();
        ensure!(
            ![
                ".git",
                ".ssh",
                ".aws",
                ".azure",
                ".kube",
                ".netrc",
                ".npmrc",
                ".pypirc",
                ".gitconfig",
                ".env",
                "credentials",
                "credentials.json",
                "id_rsa",
                "id_ed25519",
                "id_ecdsa",
                "id_dsa"
            ]
            .contains(&p.as_str())
                && !p.starts_with(".env.")
                && !p.ends_with(".pem")
                && !p.ends_with(".key"),
            "credential-like source path rejected: {text}"
        );
    }
    Ok(())
}
/// Validate the complete archive before extracting; recreate internal links on Linux.
/// Keeping links as metadata also permits submission from Windows without symlink privileges.
pub fn unpack_source(
    bytes: &[u8],
    dest: &Path,
) -> Result<std::collections::BTreeMap<String, String>> {
    let mut regular = std::collections::BTreeSet::new();
    let mut links = std::collections::BTreeMap::new();
    for entry in tar::Archive::new(bytes).entries()? {
        let entry = entry?;
        let kind = entry.header().entry_type();
        if kind.is_pax_global_extensions() {
            continue;
        }
        let path = entry.path()?.into_owned();
        safe_source_path(&path)?;
        if kind.is_dir() {
            continue;
        }
        if kind.is_file() {
            regular.insert(path);
            continue;
        }
        ensure!(kind.is_symlink(), "special source file rejected");
        let target = entry
            .link_name()?
            .context("missing link target")?
            .into_owned();
        // shortcut: only direct relative file links; extend when a real source tree needs link chains.
        safe_source_path(&target)?;
        links.insert(
            path.to_string_lossy().into_owned(),
            target.to_string_lossy().into_owned(),
        );
    }
    ensure!(!regular.is_empty(), "empty source archive");
    for (path, target) in &links {
        let path = Path::new(path);
        ensure!(
            regular.contains(&path.parent().unwrap_or(Path::new("")).join(target)),
            "source link must target a committed regular file"
        );
        ensure!(
            !regular.iter().any(|p| p.starts_with(path)),
            "source file below a symlink"
        );
    }
    for entry in tar::Archive::new(bytes).entries()? {
        let mut entry = entry?;
        if entry.header().entry_type().is_file() {
            ensure!(entry.unpack_in(dest)?, "archive escaped destination");
        }
    }
    Ok(links)
}
pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn archive_links_stay_inside_committed_files() {
        for (target, accepted) in [
            ("target.txt", true),
            ("../secret", false),
            ("/etc/passwd", false),
            ("missing.txt", false),
        ] {
            let mut builder = tar::Builder::new(Vec::new());
            let mut regular = tar::Header::new_gnu();
            regular.set_size(1);
            regular.set_mode(0o644);
            regular.set_cksum();
            builder
                .append_data(&mut regular, "target.txt", &b"x"[..])
                .unwrap();
            let mut link = tar::Header::new_gnu();
            link.set_entry_type(tar::EntryType::Symlink);
            link.set_size(0);
            link.set_mode(0o777);
            link.set_link_name(target).unwrap();
            link.set_cksum();
            builder
                .append_data(&mut link, "link", std::io::empty())
                .unwrap();
            let bytes = builder.into_inner().unwrap();
            let dest = std::env::temp_dir().join(format!(
                "colm-source-links-{}-{}",
                std::process::id(),
                sha256(&bytes)
            ));
            std::fs::create_dir_all(&dest).unwrap();
            let result = unpack_source(&bytes, &dest);
            assert_eq!(result.is_ok(), accepted);
            assert!(!dest.join("link").exists());
            if accepted {
                assert_eq!(
                    result.unwrap().get("link").map(String::as_str),
                    Some(target)
                );
            }
            std::fs::remove_dir_all(dest).unwrap();
        }
    }
    #[test]
    fn boundary_validation() {
        for p in [
            "../x",
            "/tmp/x",
            ".ssh/id_rsa",
            "foo/.env.prod",
            "x/private.key",
        ] {
            assert!(safe_source_path(Path::new(p)).is_err());
        }
        assert!(safe_source_path(Path::new("crates/a/src/main.rs")).is_ok());
        assert!(check_absolute("/data/../root").is_err());
        let mut req: Request =
            serde_json::from_str(r#"{"action":"verify","case":"/data/case"}"#).unwrap();
        assert!(req.validate().is_ok());
        req.case = Some("/data/x\nCOLM_JOB_EOF".into());
        assert!(req.validate().is_err());
        assert!(serde_json::from_str::<Request>(r#"{"action":"test","shell":"id"}"#).is_err());
    }
}

use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use colm_namelist::{document::Item, Value};

/// Whether a completed spinup should replay the simulation from its original start.
/// Older cases without this optional file retain the original cutoff behavior.
pub fn spinup_replay_enabled(case_dir: &Path) -> Result<bool> {
    let path = case_dir.join("spinup.nml");
    let source = match std::fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    let doc = colm_namelist::parse(&source).with_context(|| format!("parse {}", path.display()))?;
    let mut groups = 0;
    let mut replay = None;
    for item in &doc.items {
        match item {
            Item::GroupStart(line) => {
                let name = line.split('!').next().unwrap_or_default().trim();
                ensure!(
                    name.eq_ignore_ascii_case("&nl_colm_spinup") && groups == 0,
                    "{}: expected one nl_colm_spinup group",
                    path.display()
                );
                groups += 1;
            }
            Item::Entry(entry) => {
                ensure!(
                    entry.path.to_string().eq_ignore_ascii_case("replay_start") && replay.is_none(),
                    "{}: expected only one replay_start field",
                    path.display()
                );
                let Value::Bool(enabled) = entry.value else {
                    bail!("{}: replay_start must be .true. or .false.", path.display());
                };
                replay = Some(enabled);
            }
            _ => {}
        }
    }
    replay.with_context(|| format!("{}: missing replay_start", path.display()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn spinup_replay_is_opt_in_and_rejects_invalid_configuration() {
        let root = std::env::temp_dir().join(format!(
            "colm-spinup-replay-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("spinup.nml");
        let _ = std::fs::remove_file(&path);
        assert!(!super::spinup_replay_enabled(&root).unwrap());
        for (value, expected) in [(".true.", true), (".false.", false)] {
            std::fs::write(
                &path,
                format!("&nl_colm_spinup\n replay_start={value}\n/\n"),
            )
            .unwrap();
            assert_eq!(super::spinup_replay_enabled(&root).unwrap(), expected);
        }
        for source in [
            "",
            "&nl_colm_spinup\n/\n",
            "&nl_colm_spinup\nreplay_start=1\n/\n",
            "&nl_colm_spinup\nreplay_start='true'\n/\n",
            "&nl_colm_spinup\nreplay_start=.true.\nunknown=.true.\n/\n",
            "&wrong\nreplay_start=.true.\n/\n",
            "&nl_colm_spinup\nreplay_start=.true.\n",
            "&nl_colm_spinup\nreplay_start=.true.\nreplay_start=.false.\n/\n",
            "&nl_colm_spinup\nreplay_start=.true.\n/\n&nl_colm_spinup\n/\n",
        ] {
            std::fs::write(&path, source).unwrap();
            assert!(super::spinup_replay_enabled(&root).is_err(), "{source:?}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

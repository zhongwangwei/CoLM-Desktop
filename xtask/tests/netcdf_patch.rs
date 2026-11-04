use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path};

#[test]
fn fault_checks_refuse_optimized_python_before_running_writers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let python = if cfg!(windows) { "python" } else { "python3" };
    for script in [
        "scripts/test-netcdf-close-faults.py",
        "scripts/test-netcdf-libc-close-faults.py",
    ] {
        for optimized_by_env in [false, true] {
            let mut command = std::process::Command::new(python);
            command.env_remove("PYTHONOPTIMIZE");
            if optimized_by_env {
                command.env("PYTHONOPTIMIZE", "1");
            } else {
                command.arg("-O");
            }
            let output = command
                .arg(root.join(script))
                .arg("--help")
                .output()
                .unwrap();
            assert!(
                !output.status.success(),
                "{script} accepted optimized Python"
            );
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("require assertions"),
                "{script}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(output.stdout.is_empty());
        }
    }
}

#[test]
fn netcdf_patch_preserves_the_upstream_source_and_close_error_return() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../vendor/netcdf-src");
    let hashes: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(root.join("UPSTREAM-SHA256.json")).unwrap()).unwrap();
    let guard = r#"    println!("cargo::rerun-if-changed=source/libhdf5/hdf5file.c");
    // Keep the close-error fix when refreshing this vendored dependency.
    let close_source = std::fs::read_to_string("source/libhdf5/hdf5file.c").unwrap();
    assert!(close_source.contains(
        "/* H5Fclose can fail after tearing down the file: do not inspect its objects. */"
    ));
    assert!(!close_source.contains("        dumpopenobjects(h5);"));
"#;
    let patched =
        "/* H5Fclose can fail after tearing down the file: do not inspect its objects. */";
    let dap_note = r#"    /* CoLM audit: unchanged upstream Hyrax compatibility field. Only the explicit
     * URL #hyrax opt-in sets it (d4file.c:NCD4_applychecksumcontrols). CoLM's
     * dependency feature graph disables DAP/DAP4, so this path is not compiled.
     * Compatibility only: DAP4 is disabled in CoLM; the SHA guard protects this
     * unchanged control. Audit remote integrity before enabling DAP. */
"#;
    for (file, expected) in hashes {
        let mut bytes = fs::read(root.join(&file)).unwrap();
        if file == "build.rs" {
            let text = String::from_utf8(bytes).unwrap();
            assert_eq!(text.matches(guard).count(), 1);
            bytes = text.replacen(guard, "", 1).into_bytes();
        } else if file == "source/libhdf5/hdf5file.c" {
            let text = String::from_utf8(bytes).unwrap();
            assert_eq!(text.matches(patched).count(), 1);
            assert!(text.contains("return NC_EHDFERR;"));
            bytes = text
                .replacen(patched, "dumpopenobjects(h5);", 1)
                .into_bytes();
        } else if file == "source/libdap4/ncd4types.h" {
            let text = String::from_utf8(bytes).unwrap();
            assert_eq!(text.matches(dap_note).count(), 1);
            bytes = text.replacen(dap_note, "", 1).into_bytes();
        }
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{file}");
    }
    let manifest = include_str!("../../Cargo.toml");
    let excluded = manifest
        .lines()
        .find(|line| line.starts_with("exclude = "))
        .unwrap();
    assert!(excluded.contains("\"vendor/netcdf-src\""));
    assert!(manifest.contains("netcdf-src = { path = \"vendor/netcdf-src\" }"));
}

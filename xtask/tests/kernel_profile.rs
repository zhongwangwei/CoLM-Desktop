use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("repo root")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn make_line_continuations_one_line(text: &str) -> String {
    let mut out = String::new();
    let mut continued = false;
    for raw in text.lines() {
        let line = raw.trim_end();
        if continued {
            out.push(' ');
        }
        if let Some(stripped) = line.strip_suffix('\\') {
            out.push_str(stripped.trim_end());
            continued = true;
        } else {
            out.push_str(line);
            out.push('\n');
            continued = false;
        }
    }
    out
}

fn make_var_raw(text: &str, name: &str) -> String {
    let prefix = format!("{name} ");
    for line in make_line_continuations_one_line(text).lines() {
        let line = line.trim_start();
        if !line.starts_with(&prefix) {
            continue;
        }
        if let Some((_, value)) = line.split_once('=') {
            return value.trim().to_string();
        }
    }
    panic!("missing Make variable {name}");
}

fn make_var(text: &str, name: &str) -> String {
    let mut value = make_var_raw(text, name);
    for _ in 0..4 {
        let Some(start) = value.find("$(") else { break };
        let rest = &value[start + 2..];
        let Some(end) = rest.find(')') else { break };
        let var = &rest[..end];
        let replacement = make_var_raw(text, var);
        value.replace_range(start..start + 3 + end, &replacement);
    }
    value
}

fn tokens(flags: &str) -> Vec<&str> {
    flags.split_whitespace().collect()
}

fn assert_has_all(flags: &str, required: &[&str]) {
    let tokens = tokens(flags);
    for flag in required {
        assert!(tokens.contains(flag), "{flags:?} missing {flag}");
    }
}

fn assert_has_none(flags: &str, forbidden: &[&str]) {
    let tokens = tokens(flags);
    for flag in forbidden {
        assert!(
            !tokens.contains(flag),
            "{flags:?} should not contain {flag}"
        );
    }
}

#[test]
fn makeoptions_default_to_production_o2_and_keep_debug_profiles() {
    for (rel, debug_checks) in [
        (
            "vendor/CoLM202X/include/Makeoptions.Mac-arm",
            [
                "-O0",
                "-C",
                "-fbounds-check",
                "-ffpe-trap=invalid,zero,overflow",
                "-fbacktrace",
                "-fdump-core",
            ]
            .as_slice(),
        ),
        (
            "vendor/CoLM202X/include/Makeoptions.github",
            [
                "-O0",
                "-fcheck=all",
                "-ffpe-trap=invalid,zero,overflow",
                "-fbacktrace",
            ]
            .as_slice(),
        ),
        (
            "oracle/scripts/makeoptions/Makeoptions.msys2",
            [
                "-O0",
                "-fcheck=all",
                "-ffpe-trap=zero,overflow",
                "-fbacktrace",
            ]
            .as_slice(),
        ),
    ] {
        let makeoptions = read(rel);
        assert!(
            makeoptions.contains("COLM_KERNEL_PROFILE ?= production"),
            "{rel}: default kernel profile must be production"
        );

        let production = make_var(&makeoptions, "FOPTS_PRODUCTION");
        assert_has_all(&production, &["-O2"]);
        assert_has_none(&production, &["-fcheck=all", "-fbounds-check", "-C"]);

        let debug = make_var(&makeoptions, "FOPTS_DEBUG");
        assert_has_all(&debug, debug_checks);
    }
}

#[test]
fn kernel_build_passes_profile_to_make_and_manifest() {
    let script = read("oracle/scripts/build_kernel.sh");
    assert!(
        script.contains("PROFILE=\"${COLM_KERNEL_PROFILE:-production}\"")
            || script.contains("COLM_KERNEL_PROFILE=\"${COLM_KERNEL_PROFILE:-production}\"")
            || script.contains("COLM_KERNEL_PROFILE=\"${COLM_KERNEL_PROFILE-production}\""),
        "build_kernel.sh must default the kernel profile to production"
    );
    assert!(
        script.contains("COLM_KERNEL_PROFILE=\"$PROFILE\"")
            || script.contains("COLM_KERNEL_PROFILE=\"$COLM_KERNEL_PROFILE\""),
        "make invocation must receive COLM_KERNEL_PROFILE"
    );
    assert!(
        script.contains(r#""build_profile": "$PROFILE""#)
            || script.contains(r#""kernel_profile": "$COLM_KERNEL_PROFILE""#),
        "manifest.json must record the selected kernel profile"
    );
}

#[test]
fn crop_preset_is_real_cropon_kernel() {
    let script = read("oracle/scripts/build_kernel.sh");
    assert!(script.contains("crop)    ARGS=(SinglePoint LULC_IGBP CaMaOFF CROPON)"));
    assert!(script.contains("CROPON) echo CROP"));
    assert!(
        script.contains("/*) OUT_BASE=\"$OUTDIR\"")
            && script.contains("BUILD=\"$OUT_BASE/build-$PRESET\"")
            && script.contains("DEST=\"$OUT_BASE/$PRESET\""),
        "an absolute output directory must not be prefixed with the repository path"
    );
}

#[test]
fn release_and_ci_cover_crop_kernel_bundle() {
    let release = read(".github/workflows/release.yml");
    assert!(
        release.contains(
            "for p in default usgs crop; do ./oracle/scripts/build_kernel.sh \"$p\"; done"
        ),
        "release workflow must build default, usgs, and crop kernels"
    );
    assert!(
        release.contains("for p in default usgs crop; do\n            test -x \"$app/Contents/Resources/kernels/$p/colm.x\""),
        "macOS bundle check must require the crop kernel"
    );

    let crop_example = "US-Ne3_2002-2003_FLUXNET2015_CROP";
    assert_eq!(
        release.matches(crop_example).count(),
        1,
        "CROP example asset block must use the canonical name once, without duplicates"
    );
    assert!(
        release.contains("Runtime/ndep/fndep_colm_hist_simyr1849-2006_1.9x2.5_c100428.nc"),
        "release bundle check must include the CROP runtime dependency"
    );

    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.contains("cargo test -p xtask --test kernel_profile"),
        "CI must run the kernel/release profile contract tests"
    );
    assert!(
        ci.contains("for p in default crop; do ./oracle/scripts/build_kernel.sh \"$p\"; done"),
        "golden CI must compile the CROP kernel as well as default"
    );

    let windows = read(".github/workflows/windows-kernel.yml");
    assert!(
        windows.contains("for p in default crop; do ./oracle/scripts/build_kernel.sh \"$p\"; done"),
        "Windows kernel CI must compile the CROP kernel as well as default"
    );
}

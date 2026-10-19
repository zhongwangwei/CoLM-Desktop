#[cfg(unix)]
mod shell {
    use super::super::*;
    use std::{
        fs,
        process::Command,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn run(setup: &str, twice: bool) -> (std::process::Output, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "colm-toolchain-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let mocks = r#"
uname() { if [ "$1" = -s ]; then echo Linux; else echo x86_64; fi; }
getconf() { echo 'glibc 2.39'; }
cc() { return 0; }
gcc() { return 0; }
flock() { echo lock >> "$HOME/events"; }
cargo() { [ -f "$HOME/rust-ready" ]; }
rustc() { [ -f "$HOME/rust-ready" ]; }
cmake() {
  if [ -x "$HOME/.local/share/colm/toolchain/cmake/bin/cmake" ]; then colm_mock_version=4.4.4
  elif [ -f "$HOME/cmake-ready" ]; then colm_mock_version=${TEST_CMAKE:-4.4.4}
  else return 1; fi
  [ "$1" = --version ] && echo "cmake version $colm_mock_version"
  return 0
}
curl() {
  echo download >> "$HOME/events"
  while [ "$1" != --output ]; do shift; done
  shift
  cat > "$1" <<'SCRIPT'
#!/bin/bash
echo rust-executed >> "$HOME/events"
touch "$HOME/rust-ready"
SCRIPT
}
sha256sum() {
  case "$1" in
    */rustup-init) echo '4acc9acc76d5079515b46346a485974457b5a79893cfb01112423c89aeb5aa10  file';;
    *) echo 'e5bb807f7728cb60cd8b27ebc97a2edb469b68655f21e844a600c3575b76f5bb  file';;
  esac
}
tar() {
  echo unpack >> "$HOME/events"
  mkdir -p "$4/cmake-4.4.4-linux-x86_64/bin"
  printf '#!/bin/sh\nexit 0\n' > "$4/cmake-4.4.4-linux-x86_64/bin/cmake"
  chmod +x "$4/cmake-4.4.4-linux-x86_64/bin/cmake"
}
# macOS mv lacks -T; emulate only the tested no-existing-destination publish.
mv() { shift 3; command mv "$1" "$2"; }
"#;
        let script = format!(
            "set -e\n{mocks}\n{setup}\n{}\n{}",
            install_script(),
            if twice {
                install_script()
            } else {
                String::new()
            }
        );
        let output = Command::new("bash")
            .args(["-c", &script])
            .current_dir(&root)
            .env("HOME", &root)
            .env_remove("CARGO_HOME")
            .env_remove("RUSTUP_HOME")
            .output()
            .unwrap();
        (output, root)
    }

    #[test]
    fn eligibility_is_posix_and_checks_platform_libc_and_rust_homes() {
        let mocks = r#"
uname() { if [ "$1" = -s ]; then echo "$TEST_OS"; else echo "$TEST_ARCH"; fi; }
getconf() { echo "$TEST_LIBC"; }
cc() { return "$TEST_CC"; }
gcc() { return "$TEST_CC"; }
cargo() { return "$TEST_RUST"; }
rustc() { return "$TEST_RUST"; }
curl() { return 0; }
tar() { return 0; }
gzip() { return 0; }
flock() { return 0; }
mktemp() { return 0; }
sha256sum() { return 0; }
"#;
        for (os, arch, libc, cc, rust, custom, expected) in [
            ("Linux", "x86_64", "glibc 2.17", "0", "1", "", true),
            ("Linux", "x86_64", "glibc 2.16", "0", "1", "", false),
            ("Linux", "aarch64", "glibc 2.27", "0", "1", "", false),
            ("Linux", "aarch64", "glibc 2.28", "0", "1", "", true),
            ("Linux", "aarch64", "glibc 3.0", "0", "1", "", true),
            ("Linux", "aarch64", "glibc 1.99", "0", "1", "", false),
            ("Darwin", "x86_64", "glibc 2.39", "0", "1", "", false),
            ("Linux", "riscv64", "glibc 2.39", "0", "1", "", false),
            ("Linux", "x86_64", "glibc 2.39", "1", "1", "", false),
            ("Linux", "x86_64", "musl 2.39", "0", "1", "", false),
            ("Linux", "x86_64", "glibc 2.39.1", "0", "1", "", false),
            ("Linux", "x86_64", "glibc .39", "0", "1", "", false),
            ("Linux", "x86_64", "glibc 2.x", "0", "1", "", false),
            (
                "Linux",
                "x86_64",
                "glibc 2.39",
                "0",
                "1",
                "CARGO_HOME",
                false,
            ),
            (
                "Linux",
                "x86_64",
                "glibc 2.39",
                "0",
                "1",
                "RUSTUP_HOME",
                false,
            ),
            (
                "Linux",
                "x86_64",
                "glibc 2.39",
                "0",
                "0",
                "CARGO_HOME",
                true,
            ),
        ] {
            let mut command = Command::new("/bin/sh");
            command
                .args([
                    "-c",
                    &format!(
                        "{mocks}\n{}\ncolm_toolchain_installable",
                        requirements_script()
                    ),
                ])
                .env("HOME", std::env::temp_dir())
                .env_remove("CARGO_HOME")
                .env_remove("RUSTUP_HOME")
                .env("TEST_OS", os)
                .env("TEST_ARCH", arch)
                .env("TEST_LIBC", libc)
                .env("TEST_CC", cc)
                .env("TEST_RUST", rust);
            if !custom.is_empty() {
                command.env(custom, "/custom-rust-home");
            }
            let output = command.output().unwrap();
            assert_eq!(
                output.status.success(),
                expected,
                "{os}/{arch}/{libc}/cc={cc}/rust={rust}/custom={custom}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(output.stdout.is_empty());
        }
    }

    #[test]
    fn usable_tools_do_not_download_or_create_installation() {
        let (output, root) = run("touch \"$HOME/rust-ready\" \"$HOME/cmake-ready\"", false);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!root.join("events").exists());
        assert!(!root.join(".local").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_tools_install_once_and_recheck_on_repeated_runs() {
        let (output, root) = run("", true);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let events = fs::read_to_string(root.join("events")).unwrap();
        assert_eq!(events.matches("download").count(), 2);
        assert_eq!(events.matches("rust-executed").count(), 1);
        assert_eq!(events.matches("unpack").count(), 1);
        assert!(root
            .join(".local/share/colm/toolchain/cmake/bin/cmake")
            .is_file());
        assert!(!root.join(".local/bin/cmake").exists());
        assert_eq!(
            fs::read_to_string(root.join("phase")).unwrap(),
            "installing Rust/Cargo\ninstalling CMake\n"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_system_cmake_older_than_3_26_is_replaced_by_our_own() {
        // CentOS 8 (the 摆渡船 login node) ships CMake 3.11; the HDF5 sources need 3.26.
        for (version, installs) in [
            ("3.11.4", true),
            ("3.25.9", true),
            ("3.26.0", false),
            ("3.31.2", false),
            ("4.0.1", false),
        ] {
            let (output, root) = run(
                &format!(
                    "touch \"$HOME/rust-ready\" \"$HOME/cmake-ready\"; export TEST_CMAKE={version}"
                ),
                false,
            );
            assert!(
                output.status.success(),
                "{version}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let events = fs::read_to_string(root.join("events")).unwrap_or_default();
            assert_eq!(
                events.matches("unpack").count(),
                usize::from(installs),
                "{version}"
            );
            assert_eq!(
                root.join(".local/share/colm/toolchain/cmake/bin/cmake")
                    .is_file(),
                installs,
                "{version}"
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn checksum_and_network_failures_never_execute_or_unpack() {
        for setup in [
            "sha256sum() { echo 'badchecksum file'; }",
            "touch \"$HOME/rust-ready\"; sha256sum() { echo 'badchecksum file'; }",
            "curl() { echo download >> \"$HOME/events\"; return 22; }",
        ] {
            let (output, root) = run(setup, false);
            assert!(!output.status.success());
            let events = fs::read_to_string(root.join("events")).unwrap();
            assert!(!events.contains("rust-executed"));
            assert!(!events.contains("unpack"));
            let entries = fs::read_dir(root.join(".local/share/colm/toolchain")).unwrap();
            assert!(entries.into_iter().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".install.")));
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn unsupported_hosts_and_custom_rust_home_do_not_download() {
        for setup in [
            "getconf() { echo 'glibc 2.16'; }",
            "uname() { echo Darwin; }",
            "cc() { return 1; }; gcc() { return 1; }",
            "export CARGO_HOME=\"$HOME/custom-rust\"",
        ] {
            let (output, root) = run(setup, false);
            assert!(!output.status.success());
            let events = fs::read_to_string(root.join("events")).unwrap_or_default();
            assert!(!events.contains("download"));
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn a_server_without_curl_can_still_be_given_tools_but_never_downloads() {
        // bash so that `command` can be overridden; the real curl of the test machine stays hidden.
        let hide = "command() { if [ \"$1\" = -v ] && [ \"$2\" = curl ]; then return 1; fi; builtin command \"$@\"; }";
        let (output, root) = run(hide, false);
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            crate::offline::looks_like_network_failure(&stderr),
            "{stderr}"
        );
        assert!(!fs::read_to_string(root.join("events"))
            .unwrap_or_default()
            .contains("download"));
        fs::remove_dir_all(root).unwrap();

        let output = Command::new("bash")
            .args([
                "-c",
                &format!(
                    "{hide}\nuname() {{ if [ \"$1\" = -s ]; then echo Linux; else echo x86_64; fi; }}\n\
                     getconf() {{ echo 'glibc 2.39'; }}\ncc() {{ :; }}\ncargo() {{ return 1; }}\nrustc() {{ return 1; }}\n\
                     tar() {{ :; }}\ngzip() {{ :; }}\nflock() {{ :; }}\nmktemp() {{ :; }}\nsha256sum() {{ :; }}\n\
                     {}\ncolm_toolchain_installable",
                    requirements_script()
                ),
            ])
            .env("HOME", std::env::temp_dir())
            .env_remove("CARGO_HOME")
            .env_remove("RUSTUP_HOME")
            .output()
            .unwrap();
        assert!(output.status.success(), "no curl is still installable");
    }

    #[test]
    fn existing_cmake_destination_is_preserved() {
        let (output, root) = run("touch \"$HOME/rust-ready\"; mkdir -p \"$HOME/.local/share/colm/toolchain\"; ln -s /nonexistent \"$HOME/.local/share/colm/toolchain/cmake\"", false);
        assert!(!output.status.success());
        assert_eq!(
            fs::read_link(root.join(".local/share/colm/toolchain/cmake")).unwrap(),
            std::path::PathBuf::from("/nonexistent")
        );
        assert!(!fs::read_to_string(root.join("events"))
            .unwrap()
            .contains("download"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lock_recheck_uses_installation_completed_by_another_job() {
        let (output, root) = run(
            "flock() { touch \"$HOME/rust-ready\" \"$HOME/cmake-ready\"; }",
            false,
        );
        assert!(output.status.success());
        assert!(!root.join("events").exists());
        fs::remove_dir_all(root).unwrap();
    }
}

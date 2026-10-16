//! User-local prerequisites for the first remote source build.

/// Read-only eligibility check, shared with the connection probe.
pub fn requirements_script() -> &'static str {
    r#"colm_toolchain_installable() {
  [ "$(uname -s)" = Linux ] || return 1
  case "$(uname -m)" in x86_64) colm_glibc_minor=17;; aarch64) colm_glibc_minor=28;; *) return 1;; esac
  colm_glibc=$(getconf GNU_LIBC_VERSION 2>/dev/null) || return 1
  case "$colm_glibc" in 'glibc '*.*) colm_glibc=${colm_glibc#glibc };; *) return 1;; esac
  colm_major=${colm_glibc%%.*}
  colm_minor=${colm_glibc#*.}
  case "$colm_major" in ''|*[!0-9]*) return 1;; esac
  case "$colm_minor" in ''|*[!0-9]*) return 1;; esac
  { [ "$colm_major" -gt 2 ] || { [ "$colm_major" -eq 2 ] && [ "$colm_minor" -ge "$colm_glibc_minor" ]; }; } 2>/dev/null || return 1
  if ! cargo --version >/dev/null 2>&1 || ! rustc --version >/dev/null 2>&1; then
    [ "${CARGO_HOME:-$HOME/.cargo}" = "$HOME/.cargo" ] && [ "${RUSTUP_HOME:-$HOME/.rustup}" = "$HOME/.rustup" ] || return 1
  fi
  cc --version >/dev/null 2>&1 || gcc --version >/dev/null 2>&1 || return 1
  for colm_tool in curl tar gzip flock mktemp; do command -v "$colm_tool" >/dev/null 2>&1 || return 1; done
  command -v sha256sum >/dev/null 2>&1 || command -v shasum >/dev/null 2>&1 || return 1
  [ -d "$HOME" ] && [ -w "$HOME" ]
}
"#
}

/// Caller supplies the toolchain PATH and invokes this only for source builds.
pub fn install_script() -> String {
    format!(
        "{}{}",
        requirements_script(),
        r#"(
set -e
colm_tools_ready() { cargo --version >/dev/null 2>&1 && rustc --version >/dev/null 2>&1 && cmake --version >/dev/null 2>&1; }
colm_tools_ready && exit 0
if ! cargo --version >/dev/null 2>&1 || ! rustc --version >/dev/null 2>&1; then
  if [ "${CARGO_HOME:-$HOME/.cargo}" != "$HOME/.cargo" ] || [ "${RUSTUP_HOME:-$HOME/.rustup}" != "$HOME/.rustup" ]; then
    echo 'Automatic Rust installation requires the standard ~/.cargo and ~/.rustup directories. Install the custom CARGO_HOME/RUSTUP_HOME toolchain manually.' >&2
    exit 1
  fi
fi
if ! colm_toolchain_installable; then
  echo 'Automatic Rust/CMake installation requires Linux x86_64 (glibc >= 2.17) or aarch64 (glibc >= 2.28), a C compiler, curl, tar, gzip, flock, mktemp, SHA-256 verification and a writable home. Install these prerequisites manually or supply an exact-source prebuilt engine.' >&2
  exit 1
fi
colm_base="$HOME/.local/share/colm/toolchain"
mkdir -p "$colm_base"
exec 8>"$colm_base/install.lock"
flock 8
colm_tools_ready && exit 0
colm_stage=$(mktemp -d "$colm_base/.install.XXXXXXXX")
trap 'rm -rf -- "$colm_stage"' EXIT
colm_download() {
  curl --fail --location --proto '=https' --proto-redir '=https' --connect-timeout 30 --max-time 600 --retry 2 --output "$colm_stage/$2" "$1" || {
    echo 'Toolchain download failed. Check server HTTPS access or install Rust/CMake manually; an exact-source prebuilt engine is also supported.' >&2
    return 1
  }
  if command -v sha256sum >/dev/null 2>&1; then
    colm_actual=$(sha256sum "$colm_stage/$2")
  else
    colm_actual=$(shasum -a 256 "$colm_stage/$2")
  fi
  [ "${colm_actual%% *}" = "$3" ] || { echo "Toolchain checksum mismatch: $2; refusing to install." >&2; return 1; }
}
case "$(uname -m)" in
  x86_64)
    colm_arch=x86_64
    colm_rust_sha=4acc9acc76d5079515b46346a485974457b5a79893cfb01112423c89aeb5aa10
    colm_cmake_sha=e5bb807f7728cb60cd8b27ebc97a2edb469b68655f21e844a600c3575b76f5bb;;
  aarch64)
    colm_arch=aarch64
    colm_rust_sha=9732d6c5e2a098d3521fca8145d826ae0aaa067ef2385ead08e6feac88fa5792
    colm_cmake_sha=a1b6cc63636a0e55c63257cf3315a8a5f129e42fade25db1afea4ff8ab06f25e;;
esac
if ! cargo --version >/dev/null 2>&1 || ! rustc --version >/dev/null 2>&1; then
  echo 'installing Rust/Cargo' >> phase
  echo 'installing Rust/Cargo'
  colm_download "https://static.rust-lang.org/rustup/archive/1.29.0/$colm_arch-unknown-linux-gnu/rustup-init" rustup-init "$colm_rust_sha" || exit 1
  chmod 700 "$colm_stage/rustup-init"
  RUSTUP_DIST_SERVER=https://static.rust-lang.org RUSTUP_UPDATE_ROOT=https://static.rust-lang.org/rustup "$colm_stage/rustup-init" -y --no-modify-path --profile minimal --default-toolchain 1.97.1 || {
    echo 'Rust installation failed; check server HTTPS access and install Rust 1.97.1 manually before retrying.' >&2
    exit 1
  }
fi
if ! cmake --version >/dev/null 2>&1; then
  if [ -e "$colm_base/cmake" ] || [ -L "$colm_base/cmake" ]; then
    echo "Existing CoLM CMake installation is unusable: $colm_base/cmake. Repair it manually; it will not be overwritten." >&2
    exit 1
  fi
  echo 'installing CMake' >> phase
  echo 'installing CMake'
  colm_download "https://github.com/Kitware/CMake/releases/download/v4.4.4/cmake-4.4.4-linux-$colm_arch.tar.gz" cmake.tar.gz "$colm_cmake_sha" || exit 1
  tar -xzf "$colm_stage/cmake.tar.gz" -C "$colm_stage"
  "$colm_stage/cmake-4.4.4-linux-$colm_arch/bin/cmake" --version
  mv -T -n -- "$colm_stage/cmake-4.4.4-linux-$colm_arch" "$colm_base/cmake"
fi
colm_tools_ready || { echo 'Installed Rust/CMake tools are not usable on the server. Check the toolchain PATH and system libraries, or use an exact-source prebuilt engine.' >&2; exit 1; }
)
colm_install_status=$?
[ "$colm_install_status" -eq 0 ] || exit "$colm_install_status"
"#
    )
}

#[cfg(test)]
#[path = "toolchain_tests.rs"]
mod tests;

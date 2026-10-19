# First remote source build

The connection check is read-only. When a run needs a source build, CoLM reuses
working Rust/Cargo and CMake on the server. If they are missing, the first build
installs them for the current user, then continues building the selected model.
Later builds reuse these tools. A per-user lock serializes installation across
simultaneous runs; separate model source identities remain separate.

- Rust/Cargo 1.97.1 uses the official rustup 1.29.0 installer, the minimal profile,
  and `--no-modify-path`. It uses `~/.cargo` and `~/.rustup`.
- CMake 4.4.4 is unpacked into `~/.local/share/colm/toolchain/cmake`. Existing
  system tools and `~/.local/bin/cmake` are preserved.
- Official installer/archive SHA-256 values are pinned in
  `crates/colm-remote/src/toolchain.rs` and checked before execution or extraction.
  Update the versions and hashes together after validating a replacement.
- No sudo, system package changes or shell profile edits are needed. The job PATH
  includes the installation directories after the user's existing PATH.

Automatic installation supports GNU Linux x86_64 with glibc >=2.17, and aarch64
with glibc >=2.28. A C compiler, tar, gzip, flock, mktemp, a SHA-256 utility and a
writable home are required; curl and outbound HTTPS only for the online route below. Missing compilers must be
provided by the server administrator or an existing environment module. Custom
Rust homes are reused if working; missing custom installations require manual
setup. Failed downloads or checksum checks stop the job with a diagnostic.
Unusable existing CoLM CMake directories are preserved for manual repair.

## Servers without internet

After uploading a source snapshot the desktop checks, in one SSH call, whether the
engine is already built, whether the snapshot's crates are already on the server, and
otherwise whether the server's curl reaches crates.io and static.rust-lang.org. When it
cannot (no curl, no DNS, a blocked proxy), the desktop downloads instead and sends
everything over the same SSH pipe:

- the official Rust 1.97.1 `rustc`, `cargo` and `rust-std` archives, checksums pinned in
  `crates/colm-remote/src/offline.rs`, installed with their own `install.sh` into
  `~/.local/share/colm/toolchain/rust`, and the same pinned CMake archive as above;
- every crates.io package in the snapshot's `Cargo.lock`, checked against the lock
  file's checksums and repacked as a cargo directory source in
  `<work root>/offline/vendor-<first 16 hex digits of SHA-256(Cargo.lock)>`.

Tools the server already has are skipped, downloads are cached on the desktop, and a
build finds the vendor directory by hashing its own `Cargo.lock`, so jobs on compute
nodes build offline too. A login-node build that still fails with a download error
(a proxy that passes the check but is too slow) is retried once after the dependencies
are sent. The desktop needs internet access, but not cargo.

Automatic engine selection prefers a matching prebuilt package when build tools
are missing. Otherwise it installs the prerequisites and builds from the selected
source. Modified model versions always use their own source. The run view shows
installation and build phases; matching prebuilt execution needs no installation.

References: [Rust installation flags](https://rust-lang.github.io/rustup/installation/other.html),
[minimal profile](https://rust-lang.github.io/rustup/concepts/profiles.html),
[CMake downloads](https://cmake.org/download/),
[CMake aarch64 requirements](https://cmake.org/cmake/help/v4.4/release/4.3.html).

#!/usr/bin/env bash
# 在一台联网的 Linux 机器上，为 x86_64 与 aarch64 预编 Rust 引擎的四个程序（R3），目标 glibc 2.17：
# 没有 cargo、或不能联网的服务器与超算直接用这些程序，不必在那里编译。
#
#   scripts/build-engine-linux.sh <源码目录> <输出目录> <工具目录> <x86_64|aarch64>...
#
# - 工具链（rustup、cargo-zigbuild、zig）全部装在 <工具目录>，不碰用户主目录下的 ~/.cargo、~/.rustup。
# - 用 zig 作 C 编译器与链接器，把目标钉在 glibc 2.17；netCDF 与 HDF5 本来就静态编进程序，所以除 libc、libm
#   外没有别的运行时依赖。
# - 调试信息与符号表去掉（只影响文件大小，不影响数值）。
# - 每个目标产出 <输出目录>/colm-engine-linux-<arch>.tar.gz，解开是 bin/ 下的四个程序与一个 ENGINE 说明文件。
set -euo pipefail

if [ "$#" -lt 4 ]; then
  echo "usage: $0 SRC OUT TOOLS x86_64|aarch64..." >&2
  exit 2
fi
SRC=$(cd "$1" && pwd); OUT=$2; TOOLS=$3; shift 3
GLIBC=2.17
mkdir -p "$OUT" "$TOOLS"
OUT=$(cd "$OUT" && pwd); TOOLS=$(cd "$TOOLS" && pwd)

export RUSTUP_HOME="$TOOLS/rustup" CARGO_HOME="$TOOLS/cargo"
export PATH="$CARGO_HOME/bin:$TOOLS/bin:$PATH"

HOST=$(uname -m)
if [ ! -x "$CARGO_HOME/bin/rustup" ]; then
  echo "== installing rustup into $TOOLS"
  curl --proto '=https' --tlsv1.2 -sSf -o "$TOOLS/rustup-init" \
    "https://static.rust-lang.org/rustup/dist/${HOST}-unknown-linux-gnu/rustup-init"
  chmod +x "$TOOLS/rustup-init"
  "$TOOLS/rustup-init" -y --profile minimal --default-toolchain stable --no-modify-path
fi

# zig：从 PyPI 的官方接口取 ziglang 的轮子（里面就是 zig 编译器），核对 sha256，用 Python 标准库解开。
# 服务器上常常没有 pip 与 venv，所以不依赖它们。
if [ ! -x "$TOOLS/zig/ziglang/zig" ]; then
  echo "== installing zig (PyPI wheel 'ziglang')"
  mkdir -p "$TOOLS/zig" "$TOOLS/bin"
  python3 - "$TOOLS/zig" <<'PY'
import hashlib, json, os, platform, sys, urllib.request, zipfile
dest = sys.argv[1]
arch = platform.machine()
info = json.load(urllib.request.urlopen("https://pypi.org/pypi/ziglang/json", timeout=60))
wheels = [f for f in info["urls"] if f["filename"].endswith(".whl") and "manylinux" in f["filename"] and arch in f["filename"]]
if not wheels:
    sys.exit(f"no ziglang wheel for {arch}")
wheel = wheels[0]
data = urllib.request.urlopen(wheel["url"], timeout=600).read()
if hashlib.sha256(data).hexdigest() != wheel["digests"]["sha256"]:
    sys.exit("ziglang wheel: sha256 mismatch")
path = os.path.join(dest, wheel["filename"])
open(path, "wb").write(data)
zipfile.ZipFile(path).extractall(dest)
os.remove(path)
os.chmod(os.path.join(dest, "ziglang", "zig"), 0o755)
print("zig", info["info"]["version"])
PY
fi
ln -sf "$TOOLS/zig/ziglang/zig" "$TOOLS/bin/zig"
ZIG_VERSION=$("$TOOLS/bin/zig" version)

if ! command -v cargo-zigbuild >/dev/null 2>&1; then
  echo "== installing cargo-zigbuild"
  cargo install cargo-zigbuild --locked
fi

SOURCE_ID=$(cd "$SRC" && cat .colm-snapshot-id 2>/dev/null || echo unknown)
for arch in "$@"; do
  case "$arch" in
    x86_64|aarch64) ;;
    *) echo "unknown target $arch" >&2; exit 2 ;;
  esac
  triple="${arch}-unknown-linux-gnu"
  rustup target add "$triple" >/dev/null
  echo "== building $arch (glibc $GLIBC)"
  CARGO_TARGET_DIR="$OUT/target-$arch" \
  CARGO_PROFILE_RELEASE_DEBUG=0 CARGO_PROFILE_RELEASE_STRIP=symbols \
    cargo zigbuild --release --locked --manifest-path "$SRC/Cargo.toml" \
      --target "$triple.$GLIBC" \
      -p colm-cli --bin colm-cli -p colm-srfdata --bin mksrfdata-rs \
      -p colm-init --bin mkinidata-rs -p colm-runtime --bin colm-rs
  stage="$OUT/stage-$arch"
  rm -rf "$stage"; mkdir -p "$stage/bin"
  for b in colm-cli mksrfdata-rs mkinidata-rs colm-rs; do
    cp "$OUT/target-$arch/$triple/release/$b" "$stage/bin/$b"
  done
  cat > "$stage/ENGINE" <<EOF
arch=$arch
glibc=$GLIBC
source=$SOURCE_ID
rustc=$(rustc --version)
zig=$ZIG_VERSION
built=$(date -u +%Y-%m-%dT%H:%M:%SZ)
EOF
  tar -czf "$OUT/colm-engine-linux-$arch.tar.gz" -C "$stage" .
  rm -rf "$stage"
  echo "== done $arch: $(ls -lh "$OUT/colm-engine-linux-$arch.tar.gz" | awk '{print $5}')"
done

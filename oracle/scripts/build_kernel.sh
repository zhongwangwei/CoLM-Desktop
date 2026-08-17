#!/usr/bin/env bash
# 从 vendor/CoLM202X 构建一个 SinglePoint 物理预设，并写出内核清单。
#
# colm.x 只接受一个参数（namelist 路径，getarg(1)），没有 --version。
# 因此版本握手靠构建期生成的 manifest.json + sha256，而不是问二进制。
set -euo pipefail

PRESET="${1:?usage: build_kernel.sh <waterheat|bgc|urban> [outdir]}"
OUTDIR="${2:-kernels}"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC="$REPO_ROOT/vendor/CoLM202X"

case "$PRESET" in
  waterheat) ARGS=(SinglePoint LULC_IGBP     URBANOFF vanGenu CaMaOFF BGCOFF CROPOFF TRACEROFF) ;;
  bgc)       ARGS=(SinglePoint LULC_IGBP_PFT URBANOFF vanGenu CaMaOFF BGCON  CROPOFF TRACEROFF) ;;
  urban)     ARGS=(SinglePoint LULC_IGBP     URBANON  vanGenu CaMaOFF BGCOFF CROPOFF TRACEROFF) ;;
  *) echo "unknown preset: $PRESET" >&2; exit 2 ;;
esac

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) MAKEOPTS=Makeoptions.Mac-arm ;;
  Linux-*)      MAKEOPTS=Makeoptions.github  ;;
  *) echo "unsupported host; add a Makeoptions preset" >&2; exit 2 ;;
esac

BUILD="$REPO_ROOT/$OUTDIR/build-$PRESET"
rm -rf "$BUILD"
git -C "$SRC" worktree add --detach --force "$BUILD" HEAD >/dev/null
trap 'git -C "$SRC" worktree remove --force "$BUILD" >/dev/null 2>&1 || true' EXIT

cd "$BUILD"
ln -sf "$MAKEOPTS" include/Makeoptions
./.github/workflows/create_defineh.bash "${ARGS[@]}" >/dev/null

# FF=gfortran 而非 mpif90：SinglePoint 已 #undef USEMPI，用 mpif90 只会白链 4 个 MPI 库。
# 实测去掉后依赖只剩 netcdff/netcdf/LAPACK/BLAS/libgfortran/libgomp/libquadmath。
make FF="gfortran -fopenmp" mksrfdata.x mkinidata.x colm.x

DEST="$REPO_ROOT/$OUTDIR/$PRESET"
mkdir -p "$DEST"
cp run/mksrfdata.x run/mkinidata.x run/colm.x "$DEST/"

# 宏集合必须取**预处理后的生效集**，不能 grep define.h 的 #define 原文。
# 实测：原文 grep 会把 USEMPI / GridRiverLakeFlow / LATERAL_FLOW 都报成已定义，
# 而三者在 SinglePoint 下实际都是关闭的（前两个被 conflict 块 #undef，
# 第三个是个源码里根本不存在的宏名）。manifest 的全部作用是记录构建配置供
# Task 6 做版本握手，谎报 MPI 是开的会让这份记录反过来误导人。
printf '#include <define.h>\n' > "$BUILD/.macro_probe.F90"
# LC_ALL=C 不是装饰：裸 sort 用 locale 的字典序，en_US.UTF-8 下
# extend_interception 会排在 CoLMDEBUG 与 LULC_IGBP 之间，而 C locale 用字节序
# 把大写排在小写前。manifest 是版本握手的记录物，顺序随环境变意味着同一份构建
# 在不同机器上产出的 manifest 字节不同，任何「这两个内核配置一样吗」的比较都会假报差异。
MACROS=$(gfortran -E -dM -cpp -ffree-form -I include "$BUILD/.macro_probe.F90" 2>/dev/null \
  | awk '$1=="#define" && $2 !~ /^(_|__)/ && NF==2 {print "\""$2"\""}' | LC_ALL=C sort | paste -sd, -)
rm -f "$BUILD/.macro_probe.F90"
GIT_SHA=$(git -C "$SRC" rev-parse --short HEAD)
# macOS 有 shasum 没 sha256sum，多数 Linux 反之。两者都不通用，所以先探测。
if command -v shasum >/dev/null 2>&1; then
  sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
elif command -v sha256sum >/dev/null 2>&1; then
  sha() { sha256sum "$1" | cut -d' ' -f1; }
else
  echo "need shasum or sha256sum on PATH" >&2; exit 2
fi

cat > "$DEST/manifest.json" <<JSON
{
  "schema": 1,
  "preset": "$PRESET",
  "platform": "$(uname -s)-$(uname -m)",
  "colm_git_sha": "$GIT_SHA",
  "generator_args": "${ARGS[*]}",
  "macros": [$MACROS],
  "built_with": "$(gfortran --version | head -1)",
  "netcdf_c": "$(nc-config --version 2>/dev/null)",
  "netcdf_fortran": "$(nf-config --version 2>/dev/null)",
  "hdf5": "$(H=$(nc-config --includedir 2>/dev/null)/H5public.h; [ -f "$H" ] && grep -hE '#define H5_VERS_(MAJOR|MINOR|RELEASE)' "$H" | awk '{printf "%s.", $3}' | sed 's/[.]$//')",
  "sha256": {
    "mksrfdata": "$(sha "$DEST/mksrfdata.x")",
    "mkinidata": "$(sha "$DEST/mkinidata.x")",
    "colm":      "$(sha "$DEST/colm.x")"
  }
}
JSON

echo "built $PRESET -> $DEST"
cat "$DEST/manifest.json"

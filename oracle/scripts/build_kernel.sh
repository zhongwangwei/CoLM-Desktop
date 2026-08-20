#!/usr/bin/env bash
# 从 vendor/CoLM202X 构建一个 SinglePoint 物理预设，并写出内核清单。
#
# colm.x 只接受一个参数（namelist 路径，getarg(1)），没有 --version。
# 因此版本握手靠构建期生成的 manifest.json + sha256，而不是问二进制。
set -euo pipefail

PRESET="${1:?usage: build_kernel.sh <default|bgc|urban> [outdir]}"
OUTDIR="${2:-kernels}"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC="$REPO_ROOT/vendor/CoLM202X"

# Soil hydraulic scheme (Campbell vs. vanGenuchten) used to be a 4th
# positional argument to create_defineh.bash here. It is a runtime
# namelist switch now (DEF_USE_Campbell_SOIL_MODEL, MOD_Namelist.F90,
# default .false. i.e. vanGenuchten) -- both code paths are always
# compiled in, so create_defineh.bash no longer takes that argument and
# the presets below don't pass it either.
#
# Same story for TRACER (used to be a 7th positional argument,
# TRACERON/TRACEROFF). It is a runtime namelist switch now (DEF_USE_TRACER,
# MOD_Namelist.F90, default .false.) -- every main/TRACER module file is
# always compiled in, so create_defineh.bash no longer takes that argument
# either.
case "$PRESET" in
  default) ARGS=(SinglePoint LULC_IGBP     URBANOFF CaMaOFF BGCOFF CROPOFF) ;;
  bgc)       ARGS=(SinglePoint LULC_IGBP_PFT URBANOFF CaMaOFF BGCON  CROPOFF) ;;
  urban)     ARGS=(SinglePoint LULC_IGBP     URBANON  CaMaOFF BGCOFF CROPOFF) ;;
  *) echo "unknown preset: $PRESET" >&2; exit 2 ;;
esac

# MSYS2 的 uname 报 MINGW64_NT-10.0-…，且 uname -m 同样是 x86_64 ——
# 后者会让 Makeoptions.github 加上 -mcmodel=medium，而 MinGW 的 gfortran
# 不认这个选项。所以 Windows 用本仓库自带的那份，不复用上游的。
OWN_MAKEOPTS=""
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)  MAKEOPTS=Makeoptions.Mac-arm ;;
  Linux-*)       MAKEOPTS=Makeoptions.github  ;;
  MINGW64_NT-*|MSYS_NT-*)
                 MAKEOPTS=Makeoptions.msys2
                 OWN_MAKEOPTS="$REPO_ROOT/oracle/scripts/makeoptions/Makeoptions.msys2" ;;
  *) echo "unsupported host; add a Makeoptions preset" >&2; exit 2 ;;
esac

BUILD="$REPO_ROOT/$OUTDIR/build-$PRESET"
rm -rf "$BUILD"
# **拷贝而不是 `git worktree`。** `vendor/CoLM202X` 曾经是 submodule，
# 那时用 worktree 从它的 HEAD 建一棵临时树。入库之后它就是普通文件了
# （见 `vendor/PROVENANCE.md`），没有独立的 git 仓库可以 worktree。
#
# 用 `tar` 管道而不是 `cp -r`：CoLM 的源码树里有符号链接
# （`include/Makeoptions`、`run/scripts/batch.config`），`cp -r` 在
# 不同平台上对它们的处理不一致，而 `tar` 原样复制。下面反正会把
# `include/Makeoptions` 整个换掉，`batch.config` 单点路径根本不读。
#
# 每次都从头拷：构建会往树里写 `.o`、`.mod` 与生成的 `include/define.h`，
# 直接在 `vendor/` 里编会污染入库的源码。
mkdir -p "$BUILD"
(cd "$SRC" && tar --exclude=.git -cf - .) | (cd "$BUILD" && tar -xf -)
trap 'rm -rf "$BUILD"' EXIT

cd "$BUILD"
# 本仓库自带的预设直接拷进临时 worktree —— 它不在 submodule 里，
# 所以不能用符号链接指过去（worktree 跑完就删）。
if [ -n "$OWN_MAKEOPTS" ]; then
  cp "$OWN_MAKEOPTS" "include/$MAKEOPTS"
fi
# 用拷贝而不是 `ln -sf`：Windows 上建符号链接要特权，而 MSYS2 的 `ln -s`
# 视 MSYS 环境变量而定，可能建链接也可能复制 —— 让构建结果取决于一个
# 环境变量不值得。先删再拷，免得 cp 顺着旧的符号链接写回它自己的目标。
rm -f include/Makeoptions
cp "include/$MAKEOPTS" include/Makeoptions
./.github/workflows/create_defineh.bash "${ARGS[@]}" >/dev/null

# 自检：ARGS 里「要求打开」的宏，预处理之后是不是真的打开了。
#
# define.h 里有静默的条件 #undef —— 比如 BGCON 只在 LULC 选了
# LULC_IGBP_PFT 或 LULC_IGBP_PC 时才真的生效（"Conflicts" 注释写着
# "only used when LULC_IGBP_PFT or LULC_IGBP_PC is defined"）。
# **这份 Conflicts 逻辑来自 create_defineh.bash 自己生成的模板**——
# 上一行已经把 include/define.h 整个覆盖掉了，此后再也不会读到
# vendor/CoLM202X/include/define.h 里入库的那份，所以对不对以生成的
# 这份为准。配错的话内核照样编得出、跑得完，只是悄悄少了它名字里说
# 有的那部分物理，而且这个自检必须在 `make` 之前做——等编完了才发现，
# 每次配错都要先陪一次全量 Fortran 编译。
#
# 取「预处理后的生效集」而不是 grep define.h 的 #define 原文：原文 grep
# 会把 USEMPI / GridRiverLakeFlow / LATERAL_FLOW 都报成已定义，而它们在
# SinglePoint 下实际都被下游的 conflict 块关掉了。下面算出来的 $EFFECTIVE
# 后面 manifest.json 的 macros 字段直接复用，不重算第二遍——重算两遍还要
# 保证两边算法一致，本身就是又一个出错点。
printf '#include <define.h>\n' > "$BUILD/.macro_probe.F90"
if ! MACRO_PROBE=$(gfortran -E -dM -cpp -ffree-form -I include "$BUILD/.macro_probe.F90" 2>&1); then
  rm -f "$BUILD/.macro_probe.F90"
  echo "cannot preprocess include/define.h to see which macros take effect:" >&2
  echo "$MACRO_PROBE" >&2
  exit 3
fi
rm -f "$BUILD/.macro_probe.F90"
# LC_ALL=C 不是装饰：见下面 manifest.json 那节的说明——这里先排好序，
# 后面直接复用，顺序不会因为 EFFECTIVE 复用的位置不同而变。
EFFECTIVE=$(echo "$MACRO_PROBE" | awk '$1=="#define" && $2 !~ /^(_|__)/ && NF==2 {print $2}' | LC_ALL=C sort)

is_effective() { printf '%s\n' "$EFFECTIVE" | grep -qxF "$1"; }

# 预设参数值 -> 它应该打开的宏名。以 create_defineh.bash 的实际映射为准
# （该脚本按位置取 $1..$7，每个位置各自 case 出一对 #define/#undef——
# 去读那个脚本才知道，不能靠猜）。这里只列「要求打开」的取值，OFF 类
# 取值（BGCOFF、CROPOFF……）不隐含任何宏，用不着查。
#
# 没有 Campbell/vanGenu 条目——土壤水力方案改成运行时开关之后，
# create_defineh.bash 不再吃这个参数，两条物理路径始终一起编进去。
# 也没有 TRACERON 条目——TRACER 同样改成运行时开关了（DEF_USE_TRACER），
# create_defineh.bash 不再吃这个参数，main/TRACER/ 底下的模块始终编进去。
macro_for_arg() {
  case "$1" in
    GRID) echo GRIDBASED ;;
    CATCHMENT) echo CATCHMENT ;;
    UNSTRUCTURED) echo UNSTRUCTURED ;;
    SinglePoint) echo SinglePoint ;;
    LULC_USGS) echo LULC_USGS ;;
    LULC_IGBP) echo LULC_IGBP ;;
    LULC_IGBP_PFT) echo LULC_IGBP_PFT ;;
    LULC_IGBP_PC) echo LULC_IGBP_PC ;;
    URBANON) echo URBAN_MODEL ;;
    CaMaON) echo CaMa_Flood ;;
    BGCON) echo BGC ;;
    CROPON) echo CROP ;;
    *) : ;;
  esac
}

for arg in "${ARGS[@]}"; do
  want=$(macro_for_arg "$arg")
  [ -z "$want" ] && continue
  if ! is_effective "$want"; then
    case "$want" in
      BGC)
        cat >&2 <<MSG
$arg was requested but BGC is not in effect -- define.h turns it off
unless LULC_IGBP_PFT or LULC_IGBP_PC is defined (see the "Conflicts"
comment for BGC in create_defineh.bash, which is what generates the
include/define.h actually compiled here). The kernel would build fine
and run fine, and silently have no biogeochemistry.
MSG
        ;;
      CROP)
        cat >&2 <<MSG
$arg was requested but CROP is not in effect -- define.h turns it off
unless BGC is defined (see the "Conflicts" comment for CROP), and BGC
itself turns off unless LULC_IGBP_PFT or LULC_IGBP_PC is defined. The
kernel would build fine and run fine, and silently have no crop model.
MSG
        ;;
      *)
        echo "$arg was requested but $want is not in effect after" >&2
        echo "define.h's conditional #undef blocks. Context from the" >&2
        echo "generated include/define.h:" >&2
        grep -n -B2 -A4 "$want\b" include/define.h >&2 || true
        echo "The kernel would build fine and run fine while silently" >&2
        echo "missing $want." >&2
        ;;
    esac
    exit 3
  fi
done

# Windows 上给 CoLM 建目录的路径**加引号**。
#
# CoLM 在 55 处用 `CALL system('mkdir -p ' // 路径)`。`system()` 在 MinGW 上
# 走 `cmd.exe /c`，而 cmd 在**未加引号**的参数里把 `/` 当开关前缀 ——
# 于是 `mkdir D:\x\out/CN-Cng/landdata` 报
# `The syntax of the command is incorrect.`，模型随后往不存在的目录里写。
#
# CI 的探针把这件事量清楚了（见 windows-kernel.yml）：
#
#   mkdir -p <反斜杠嵌套>   退出码 0   目录建了
#   mkdir    <尾正斜杠>     退出码 1   目录没建
#   mkdir    <全正斜杠>     退出码 1   目录没建
#
# 所以**问题不在 `-p`**（cmd 把它当成另一个目录名，无害），在斜杠。
# 而斜杠有一半是上游自己拼的（`DEF_dir_output // '/' // ...`，
# `MOD_Namelist.F90:1403`），我们在 namelist 里改不掉。加引号能一并挡住
# 两边的来源：Win32 的 CreateDirectory 本来就认正斜杠，只有 cmd 的**解析器**
# 不认，而引号正是绕过解析器的办法。
#
# **改的是构建出来的那一份源码，不是仓库里的。** `build_kernel.sh` 已经
# 在改 `include/Makeoptions` 与 `include/define.h` 了，这一处同理；
# 上游的写法在 Linux/macOS 上完全正确，不该为 Windows 去改它。
#
# 顺带说明为什么只处理 mkdir：三个二进制里另有 3 处 `cp`
# （`MOD_RegionClip.F90`），但它在 `USE_srfdata_from_larger_region` 分支里，
# 而 SinglePoint 在到达那里之前就 `CoLM_stop` 了（`MKSRFDATA.F90:149`）。
# `postprocess/` 里的 `mv` 与文件列举也不在这三个二进制内。
if [ -n "$OWN_MAKEOPTS" ]; then
  # `|| true` 不是保险，是必需：脚本开着 `set -euo pipefail`，而 grep
  # 找不到东西时返回 1 —— 下面那次核对**恰恰期望找不到**（都改完了），
  # 于是「改对了」会让构建以退出码 1 结束，且一个字都不打。实测踩过。
  files=$(grep -rl "mkdir -p " --include='*.F90' . | grep -vE '/preprocess/|/extends/CaMa/' || true)
  n=$(echo "$files" | grep -c . || true)
  # 一处都没有就是这个假设过期了 —— 静默跳过会让 Windows 再次在运行到
  # 一半时死掉，而构建看上去一切正常。
  if [ "$n" = "0" ]; then
    echo "no 'mkdir -p' in the Fortran sources -- has upstream changed?" >&2
    exit 3
  fi
  echo "$files" | xargs sed -i -E "s|CALL system\('mkdir -p ' // (.*)\)[[:space:]]*\$|CALL system('mkdir \"' // \1 // '\"')|"
  left=$(grep -r "mkdir -p " --include='*.F90' . 2>/dev/null | grep -vE '/preprocess/|/extends/CaMa/' | wc -l | tr -d ' ' || true)
  echo "Windows: $n 个文件里的 mkdir 路径已加引号，剩余未改 $left 处"
  if [ "$left" != "0" ]; then
    echo "some 'mkdir -p' lines did not match the rewrite -- they would fail at run time" >&2
    exit 3
  fi
fi

# MPI 的**头文件路径**。三个预设一个都不用 MPI，编译却仍然要它：
# `MOD_SPMD_Task.F90:34` 的 `include 'mpif.h'` 在任何 `#ifdef` 之外，
# 而 `#ifndef USEMPI` 从下一行才开始 —— SinglePoint 把 USEMPI 关掉了，
# 头文件照样要找得到。
#
# 路径不写死：macOS 的 Homebrew 放在 /opt/homebrew/include（Makeoptions 已经带上），
# 而 Ubuntu 的 libopenmpi-dev 放在 /usr/lib/x86_64-linux-gnu/openmpi/include，
# 不在默认搜索路径上。问 mpif90 自己要 —— 它正是为此存在的包装器。
# `--showme:incdirs` 在 OpenMPI 5 上返回空（实测 5.0.9），所以解析 `-show`。
MPI_INC=""
if command -v mpif90 >/dev/null 2>&1; then
  MPI_INC=$(mpif90 -show 2>/dev/null | tr ' ' '\n' | grep '^-I' | sort -u | tr '\n' ' ')
fi

# FF=gfortran 而非 mpif90：SinglePoint 已 #undef USEMPI，用 mpif90 只会白链 4 个 MPI 库。
# 实测去掉后依赖只剩 netcdff/netcdf/LAPACK/BLAS/libgfortran/libgomp/libquadmath。
# 只借它的 -I，不借它的 -l。
make FF="gfortran -fopenmp $MPI_INC" mksrfdata.x mkinidata.x colm.x

DEST="$REPO_ROOT/$OUTDIR/$PRESET"
mkdir -p "$DEST"
cp run/mksrfdata.x run/mkinidata.x run/colm.x "$DEST/"

# Windows 上改成 `.exe`。CoLM 的 Makefile 在所有平台都写 `.x`，而 Windows 的
# `PATHEXT` 不含它 —— 系统于是不把这个文件当可执行文件，PowerShell 拒绝执行
# （实测 `Cannot run a document in the middle of a pipeline`），双击也没反应，
# 安全软件对「带 PE 头却顶着陌生后缀」的文件也更不客气。
#
# 改名只动**拷进内核目录的那份**，不碰 `run/` 里 Makefile 的产物，
# 于是 submodule 保持在一个干净的上游 commit 上。
#
# 名字的唯一真相在 `colm_kernel::program_file()`；这里必须与它一致，
# 否则 `Kernel::open` 会去校验一个不存在的文件。
case "$(uname -s)" in
  MINGW*|MSYS*)
    for p in mksrfdata mkinidata colm; do mv "$DEST/$p.x" "$DEST/$p.exe"; done
    EXE=.exe ;;
  *) EXE=.x ;;
esac

# 宏集合已经在自检那步（create_defineh.bash 之后、编译之前）算过一次，
# 存在 $EFFECTIVE 里——直接复用，不重新跑一次 gfortran 预处理。重算
# 两遍还要保证两边算法一致，本身就是又一个出错点，而 include/define.h
# 从自检那步到这里从未被改过。LC_ALL=C 不是装饰：裸 sort 用 locale 的
# 字典序，en_US.UTF-8 下 extend_interception 会排在 CoLMDEBUG 与
# LULC_IGBP 之间，而 C locale 用字节序把大写排在小写前——已经在自检
# 那步排过一次序了，这里不用再排。manifest 是版本握手的记录物，顺序
# 随环境变意味着同一份构建在不同机器上产出的 manifest 字节不同，任何
# 「这两个内核配置一样吗」的比较都会假报差异。
MACROS=$(printf '%s\n' "$EFFECTIVE" | awk 'NF{print "\""$0"\""}' | paste -sd, -)
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
    "mksrfdata": "$(sha "$DEST/mksrfdata$EXE")",
    "mkinidata": "$(sha "$DEST/mkinidata$EXE")",
    "colm":      "$(sha "$DEST/colm$EXE")"
  }
}
JSON

echo "built $PRESET -> $DEST"
cat "$DEST/manifest.json"

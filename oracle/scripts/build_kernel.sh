#!/usr/bin/env bash
# 从 vendor/CoLM202X 构建一个站点或空间预设，并写出内核清单。
# 默认是 production (-O2)；定位编译期越界时用
# `COLM_KERNEL_PROFILE=debug ./oracle/scripts/build_kernel.sh ...`。
#
# colm.x 只接受一个参数（namelist 路径，getarg(1)），没有 --version。
# 因此版本握手靠构建期生成的 manifest.json + sha256，而不是问二进制。
set -euo pipefail

PRESET="${1:?usage: build_kernel.sh <default|usgs|bgc|urban|crop|latlon[-usgs|-crop]|unstructured[-usgs|-crop]|catchment[-usgs|-crop]> [outdir]}"
OUTDIR="${2:-kernels}"
PROFILE="${COLM_KERNEL_PROFILE:-production}"
case "$PROFILE" in
  production|debug) ;;
  *) echo "COLM_KERNEL_PROFILE must be production or debug, got: $PROFILE" >&2; exit 2 ;;
esac

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
case "$OUTDIR" in
  /*) OUT_BASE="$OUTDIR" ;;
  *)  OUT_BASE="$REPO_ROOT/$OUTDIR" ;;
esac
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
#
# Same story again for the subgrid structure (LULC_IGBP_PFT/LULC_IGBP_PC,
# used to be part of the old 2nd argument), URBAN_MODEL (used to be a 3rd
# argument, URBANON/URBANOFF) and BGC (used to be a 5th argument,
# BGCON/BGCOFF): all runtime switches now (DEF_USE_PFT/DEF_USE_PC,
# DEF_URBAN_RUN, DEF_USE_BGC -- MOD_Namelist.F90). main/BGC/, main/URBAN/,
# main/LULCC/ and the PFT/PC subgrid modules are always compiled in, so
# create_defineh.bash's 2nd argument only picks land *classification* now
# (LULC_USGS/LULC_IGBP -- still a real compile-time choice, see that
# script's header comment and docs/plan-macro-runtime.md for why), and the
# URBANON/OFF and BGCON/OFF argument slots are gone entirely.
#
# default/bgc/urban 编成同一份 IGBP 产物；bgc/urban 只为旧测试名保留别名。
# 空间预设也按 IGBP / USGS / CROP 三种编译能力展开；范围（流域/区域/全球）
# 是运行时 domain mask，不产生九份重复内核。
SPATIAL=0
case "$PRESET" in
  default) ARGS=(SinglePoint LULC_IGBP CaMaOFF CROPOFF) ;;
  usgs)    ARGS=(SinglePoint LULC_USGS CaMaOFF CROPOFF) ;;
  bgc)     ARGS=(SinglePoint LULC_IGBP CaMaOFF CROPOFF) ;;
  urban)   ARGS=(SinglePoint LULC_IGBP CaMaOFF CROPOFF) ;;
  crop)    ARGS=(SinglePoint LULC_IGBP CaMaOFF CROPON) ;;
  latlon)              ARGS=(GRID LULC_IGBP CaMaOFF CROPOFF); SPATIAL=1 ;;
  latlon-usgs)         ARGS=(GRID LULC_USGS CaMaOFF CROPOFF); SPATIAL=1 ;;
  latlon-crop)         ARGS=(GRID LULC_IGBP CaMaOFF CROPON);  SPATIAL=1 ;;
  unstructured)        ARGS=(UNSTRUCTURED LULC_IGBP CaMaOFF CROPOFF); SPATIAL=1 ;;
  unstructured-usgs)   ARGS=(UNSTRUCTURED LULC_USGS CaMaOFF CROPOFF); SPATIAL=1 ;;
  unstructured-crop)   ARGS=(UNSTRUCTURED LULC_IGBP CaMaOFF CROPON);  SPATIAL=1 ;;
  catchment)           ARGS=(CATCHMENT LULC_IGBP CaMaOFF CROPOFF); SPATIAL=1 ;;
  catchment-usgs)      ARGS=(CATCHMENT LULC_USGS CaMaOFF CROPOFF); SPATIAL=1 ;;
  catchment-crop)      ARGS=(CATCHMENT LULC_IGBP CaMaOFF CROPON);  SPATIAL=1 ;;
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

BUILD="$OUT_BASE/build-$PRESET"
rm -rf "$BUILD"
# **拷贝而不是 `git worktree`。** `vendor/CoLM202X` 曾经是 submodule，
# 那时用 worktree 从它的 HEAD 建一棵临时树。入库之后它就是普通文件了
# （见 `vendor/PROVENANCE.md`），没有独立的 git 仓库可以 worktree。
#
# 用 `tar` 管道而不是 `cp -r`：CoLM 的源码树里有符号链接
# （`include/Makeoptions`、`run/scripts/batch.config`、
# `run/scripts/machine.config`），`cp -r` 在不同平台上对它们的处理不一致。
#
# 打包端的 `-h`（解引用）不是可选项。MSYS2 的 tar **建不了真正的符号
# 链接**，它退而复制目标文件 —— 而目标只有在归档里排在链接前面时才
# 已经落地。归档顺序是目录项顺序，不是字母序：vendor 里增删任何一个
# 文件都可能把 `batch.github.config` 挪到 `batch.config` 后面，于是
# Windows 构建报 `Cannot create symlink to 'batch.github.config':
# No such file or directory` 当场退出（实测 windows-kernel run
# 32445644366，同一份脚本此前多次通过）。解引用之后归档里只剩普通
# 文件，顺序怎么变都无所谓；三个链接指的都是同目录下现存的文件，
# 内容一模一样。下面反正会把 `include/Makeoptions` 整个换掉，
# `batch.config` 与 `machine.config` 单点路径根本不读。
#
# 每次都从头拷：构建会往树里写 `.o`、`.mod` 与生成的 `include/define.h`，
# 直接在 `vendor/` 里编会污染入库的源码。
mkdir -p "$BUILD"
(cd "$SRC" && tar -h --exclude=.git -cf - .) | (cd "$BUILD" && tar -xf -)
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
if [ "$SPATIAL" -eq 1 ]; then
  # 空间版使用普通 SPMD；站点预设保持原有 Master/IO/Worker 路径。
  printf '\n#define FLAT_SPMD\n' >> include/define.h
  # 空间版明确不编译 extends/interception；站点预设保持原行为。
  sed -i.bak 's/^#define extend_interception$/#undef extend_interception/' include/define.h
fi

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
# 会把 USEMPI / GridRiverLakeFlow / CatchLateralFlow 都报成已定义，而它们在
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
# （该脚本按位置取 $1..$4，每个位置各自 case 出一对 #define/#undef——
# 去读那个脚本才知道，不能靠猜）。这里只列「要求打开」的取值，OFF 类
# 取值（CROPOFF……）不隐含任何宏，用不着查。
#
# 没有 Campbell/vanGenu 条目——土壤水力方案改成运行时开关之后，
# create_defineh.bash 不再吃这个参数，两条物理路径始终一起编进去。
# 也没有 TRACERON 条目——TRACER 同样改成运行时开关了（DEF_USE_TRACER），
# create_defineh.bash 不再吃这个参数，main/TRACER/ 底下的模块始终编进去。
# 也没有 LULC_IGBP_PFT/LULC_IGBP_PC/URBANON/BGCON 条目——同一批理由：
# 次网格结构（DEF_USE_PFT/DEF_USE_PC）、URBAN_MODEL（DEF_URBAN_RUN）、
# BGC（DEF_USE_BGC）都改成运行时开关了，main/BGC/、main/URBAN/、
# main/LULCC/ 与 PFT/PC 次网格模块始终编进去，create_defineh.bash 的
# 第 2 个参数现在只选地类分类（LULC_USGS/LULC_IGBP，仍是编译期选择，
# 理由见 create_defineh.bash 的头注释与 docs/plan-macro-runtime.md）。
# URBAN_MODEL 现在随第 2 个参数（地类分类）无条件 #define，不再需要
# 单独核对「有没有打开」。
macro_for_arg() {
  case "$1" in
    GRID) echo GRIDBASED ;;
    CATCHMENT) echo CATCHMENT ;;
    UNSTRUCTURED) echo UNSTRUCTURED ;;
    SinglePoint) echo SinglePoint ;;
    LULC_USGS) echo LULC_USGS ;;
    LULC_IGBP) echo LULC_IGBP ;;
    CaMaON) echo CaMa_Flood ;;
    CROPON) echo CROP ;;
    *) : ;;
  esac
}

for arg in "${ARGS[@]}"; do
  want=$(macro_for_arg "$arg")
  [ -z "$want" ] && continue
  if ! is_effective "$want"; then
    echo "$arg was requested but $want is not in effect after" >&2
    echo "define.h's conditional #undef blocks. Context from the" >&2
    echo "generated include/define.h:" >&2
    grep -n -B2 -A4 "$want\b" include/define.h >&2 || true
    echo "The kernel would build fine and run fine while silently" >&2
    echo "missing $want." >&2
    exit 3
  fi
done

if [ "$SPATIAL" -eq 1 ]; then
  is_effective USEMPI || { echo "spatial kernel must enable USEMPI" >&2; exit 3; }
  is_effective FLAT_SPMD || { echo "spatial kernel must enable FLAT_SPMD" >&2; exit 3; }
  is_effective extend_interception && { echo "spatial kernel must disable extend_interception" >&2; exit 3; }
  is_effective CaMa_Flood && { echo "spatial kernel must disable CaMa_Flood" >&2; exit 3; }
  if [ "${ARGS[0]}" = CATCHMENT ]; then
    is_effective CatchLateralFlow || { echo "CATCHMENT must enable CatchLateralFlow" >&2; exit 3; }
    is_effective GridRiverLakeFlow && { echo "CATCHMENT must disable GridRiverLakeFlow" >&2; exit 3; }
  else
    is_effective GridRiverLakeFlow || { echo "GRID/UNSTRUCTURED must enable GridRiverLakeFlow" >&2; exit 3; }
    is_effective CatchLateralFlow && { echo "GRID/UNSTRUCTURED must disable CatchLateralFlow" >&2; exit 3; }
  fi
fi

# BGC / LULCC no longer appear in $EFFECTIVE at all; URBAN_MODEL is instead
# unconditionally defined by create_defineh.bash. This
# self-check used to also confirm URBAN_MODEL/BGC took effect for the
# "urban"/"bgc" presets specifically; that check is gone along with the
# macros themselves. What actually turns urban/bgc physics on now is the
# case.nml each preset's test points the kernel at (DEF_URBAN_RUN,
# DEF_USE_BGC) -- not anything checkable from define.h at build time.
if ! is_effective URBAN_MODEL; then
  echo "URBAN_MODEL is not in effect -- create_defineh.bash should" >&2
  echo "unconditionally #define it now; has that script regressed?" >&2
  exit 3
fi

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

# 注释里的 `/` 紧跟 `*` 会让 cpp 从那里开始**吞代码**。
#
# `-cpp` 走的是 C 预处理器，而它不认识 Fortran 的 `!`：一行注释里写
# `main/BGC/*.F90`，那个 `/*` 就开了一段 C 注释，一直吞到下一个 `*/`
# 为止 —— 中间几百行代码照吞不误，不警告，退出码 0。实测
# `share/MOD_Namelist.F90` 里 4 处这种写法（`main/*.F90`、
# `main/TRACER/*.F90` ……）把第 338 行到第 649 行整段吃掉，
# `type nl_forcing_type` 的定义没了，只剩它的 `END type` 孤零零留在
# 那里，Linux 构建报 `Expecting END MODULE statement`（release run
# 32445653124）。同一个坑此前在 `create_defineh.bash` 的头注释里踩过
# 一次（见 06543f8），所以这次立一道检查，不再靠记性。
#
# **本机看不见这件事。** `Makeoptions.Mac-arm` 带 `-C`（预处理保留注释），
# C 注释于是原样交给 Fortran 前端，而前端只看行首的 `!`，一切正常；
# `Makeoptions.github` 不带 `-C`。所以同一份源码 macOS 编得过、Linux
# 编不过，本地全量构建绿着也证明不了什么 —— 这道检查因此放在这里，
# 在**每个**平台的构建里跑。
#
# 只报「跨行**且**吞掉代码」的那种：`main/HYDRO/MOD_Hydro_VIC*.F90` 从
# VIC 移植来的 doxygen 块注释也是跨行 `/* ... */`，但它每一行都是注释，
# 无害 —— 报出来只会训练人忽略这道检查。`tests/` 不看：它在上游的
# `.gitignore` 里（`/tests`），谁的工作树里有就有，不参与构建。
SWALLOWED=$(find . -name '*.F90' ! -path './tests/*' -print0 | xargs -0 awk '
  FNR == 1 { if (inc) printf "%s:%d: unterminated C comment eats the rest of the file\n", prev, start; inc = 0; eaten = 0 }
  {
    prev = FILENAME
    if (inc) { t = $0; sub(/^[ \t]+/, "", t); if (t != "" && substr(t, 1, 1) != "!") eaten = 1 }
    i = 1
    while (i < length($0)) {
      two = substr($0, i, 2)
      if (!inc && two == "/*") { inc = 1; start = FNR; eaten = 0; i += 2; continue }
      if (inc && two == "*/") {
        if (start != FNR && eaten) printf "%s:%d: C comment opened here is closed on line %d, eating the code in between\n", FILENAME, start, FNR
        inc = 0; eaten = 0; i += 2; continue
      }
      i++
    }
  }
  END { if (inc) printf "%s:%d: unterminated C comment eats the rest of the file\n", FILENAME, start }
')
if [ -n "$SWALLOWED" ]; then
  echo "$SWALLOWED" >&2
  echo "cpp would delete Fortran code above -- a comment contains '/' followed" >&2
  echo "by '*'. Rewrite it in words (\"every module under main/BGC/\"), the way" >&2
  echo "create_defineh.bash's header comment was rewritten in 06543f8." >&2
  exit 3
fi

# MPI 的**头文件路径**。SinglePoint 产物不用 MPI，编译却仍然要它：
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

# SinglePoint 不白链 MPI；空间预设必须由 mpif90 同时提供头文件和链接参数。
if [ "$SPATIAL" -eq 1 ]; then
  command -v mpif90 >/dev/null 2>&1 || { echo "spatial kernel build requires mpif90" >&2; exit 2; }
  MAKE_FF="mpif90 -fopenmp"
else
  MAKE_FF="gfortran -fopenmp $MPI_INC"
fi
make FF="$MAKE_FF" COLM_KERNEL_PROFILE="$PROFILE" \
  mksrfdata.x mkinidata.x colm.x

DEST="$OUT_BASE/$PRESET"
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
  "build_profile": "$PROFILE",
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

echo "built $PRESET ($PROFILE) -> $DEST"
cat "$DEST/manifest.json"

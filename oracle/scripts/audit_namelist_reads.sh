#!/usr/bin/env bash
# 审计"声明里的开关，运行时到底读没读" —— **运行期 trace**，不是 grep。
#
# 为什么不能用 grep：注释里提一句就算命中（`DEF_Interception_scheme` 就是这样被漏掉的），
# 而名字经常经常量/helper 传下去、正则也看不见（见 docs 第 389 轮）。
# `Document::get` 加了 `COLM_NML_TRACE=<文件>` 埋点，记的是**真被问过的名字**。
#
# 用法: oracle/scripts/audit_namelist_reads.sh [<case>]      # 默认 CN-Cng，只跑 1 步
#       （namelist 在启动时一次读完，1 步足够）
#
# 输出：声明里**从没被问过**的字段，按组归类。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
export NETCDF_DIR=/opt/homebrew/opt/netcdf
case=${1:-CN-Cng}
d=/tmp/gf/nml_audit
rm -rf "$d"; mkdir -p "$d/run"
cp -R "$BASE/oracle/work/$case" "$d/case"
python3 - "$d" "$case" <<'PY'
import re, sys
d, case = sys.argv[1], sys.argv[2]
p = f"{d}/case/case.nml"
s = open(p).read()
s = re.sub(r"^   DEF_forcing_namelist.*$",
           f"   DEF_forcing_namelist = '{d}/case/forcing.nml'", s, flags=re.M)
s = re.sub(r"^   DEF_simulation_time%end_(day|sec).*$",
           lambda m: f"   DEF_simulation_time%end_{m.group(1)}       = {1 if m.group(1)=='day' else 1800}",
           s, flags=re.M)
open(p, "w").write(s)
PY
sed -i '' "s#^   DEF_dir_forcing.*#   DEF_dir_forcing              = '$BASE/examples/Forcing/'#" "$d/case/forcing.nml"
sed -e "s#^   DEF_dir_output.*#   DEF_dir_output  = '$d/case/out/'#" \
    -e "s#^   DEF_dir_rawdata.*#   DEF_dir_rawdata = '$d/rawdata_unused/'#" \
    -e "s#^   DEF_dir_runtime.*#   DEF_dir_runtime = '$d/runtime_unused/'#" \
    "$d/case/case.nml" > "$d/case.nml"
: > "$d/keys.txt"
( cd "$BASE" && COLM_NML_TRACE="$d/keys.txt" cargo run -q -p colm-runtime --bin colm-rs -- \
    "$d/case" --land-cover igbp --restart-out "$d/rust_restart.nc" --history-dir "$d" \
    > "$d/r.log" 2>&1 ) || { echo "!! rust run failed"; tail -5 "$d/r.log"; exit 3; }
python3 - "$BASE" "$d/keys.txt" <<'PY'
import collections, re, sys
base, trace = sys.argv[1], sys.argv[2]
schema = open(f"{base}/crates/colm-schema/src/generated.rs").read()
fields = re.findall(
    r'Field \{ name: "([^"]+)",.*?group: Some\("([^"]+)"\)', schema, re.S)
asked = {line.strip() for line in open(trace) if line.strip()}
print(f"运行期被问到的 key: {len(asked)} 个")
by_group = collections.defaultdict(list)
for name, group in fields:
    if name in asked:
        continue
    # 路径式字段（DEF_forcing%x）在 trace 里就是原样问的；数组下标不在这里展开
    by_group[group].append(name)
total = sum(len(v) for v in by_group.values())
print(f"声明里从没被问过: {total} / {len(fields)}")
for group, names in sorted(by_group.items(), key=lambda kv: -len(kv[1])):
    shown = ", ".join(sorted(names)[:12])
    more = f" …(+{len(names)-12})" if len(names) > 12 else ""
    print(f"  [{group}] {len(names)}: {shown}{more}")
PY

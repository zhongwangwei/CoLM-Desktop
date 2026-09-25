#!/usr/bin/env bash
# 审计"声明里的开关，运行时到底读没读" —— **运行期 trace**，不是 grep。
#
# 为什么不能用 grep / 静态提取：假阴性率实测很高，精炼过的访问器正则仍漏掉
# 27/87 个**真被读到**的名字（表驱动读、单参数 `.get("X")`、名字经常量传下去）；
# 而按裸名字 grep 更糟 —— 注释里提一句就算命中（`DEF_Interception_scheme`
# 就是这样被漏掉过，见 docs 第 388/389 轮）。所以门禁只能建在"真跑一遍拿 trace"上。
#
# 用法:
#   oracle/scripts/audit_namelist_reads.sh              # 与快照对差（有新增的"没被读"就退出 1）
#   oracle/scripts/audit_namelist_reads.sh --update     # 重写快照 oracle/nml-unread.snapshot
#   oracle/scripts/audit_namelist_reads.sh --report     # 只打印按组分类的报告
#
# 快照与仓库既有的 drift 测试同型：**没有新增的"声明了却没被读"就不许变**。
# 只跑 1 步（namelist 在启动时一次读完）。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
export NETCDF_DIR=/opt/homebrew/opt/netcdf
mode=${1:---check}
case=${2:-CN-Cng}
case "$mode" in
  --check|--update|--report) ;;
  *) echo "usage: $0 [--check|--update|--report] [<case>]" >&2; exit 2 ;;
esac
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
s = re.sub(r"^   DEF_simulation_time%end_day.*$",
           "   DEF_simulation_time%end_day       = 1", s, flags=re.M)
s = re.sub(r"^   DEF_simulation_time%end_sec.*$",
           "   DEF_simulation_time%end_sec       = 1800", s, flags=re.M)
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
SNAPSHOT="$BASE/oracle/nml-unread.snapshot"
python3 - "$BASE" "$d/keys.txt" "$mode" "$SNAPSHOT" <<'PY'
import collections
import pathlib
import re
import sys

base, trace, mode, snapshot = sys.argv[1:5]
schema = pathlib.Path(base, "crates/colm-schema/src/generated.rs").read_text()
fields = re.findall(r'Field \{ name: "([^"]+)",.*?group: Some\("([^"]+)"\)', schema, re.S)
asked = {line.strip() for line in open(trace) if line.strip()}
unread = [name for name, _ in fields if name not in asked]

if mode == "--report":
    by_group = collections.defaultdict(list)
    for name, group in fields:
        if name not in asked:
            by_group[group].append(name)
    print(f"runtime keys queried: {len(asked)}")
    print(f"declared but never queried: {len(unread)} / {len(fields)}")
    for group, names in sorted(by_group.items(), key=lambda kv: -len(kv[1])):
        shown = ", ".join(sorted(names)[:12])
        more = f" ...(+{len(names) - 12})" if len(names) > 12 else ""
        print(f"  [{group}] {len(names)}: {shown}{more}")
    raise SystemExit(0)

if mode == "--update":
    pathlib.Path(snapshot).write_text("\n".join(sorted(unread)) + "\n")
    print(f"snapshot rewritten: {snapshot} ({len(unread)} unread)")
    raise SystemExit(0)

recorded = set(pathlib.Path(snapshot).read_text().split())
current = set(unread)
added = sorted(current - recorded)
removed = sorted(recorded - current)
print(f"runtime keys queried: {len(asked)}; unread: {len(unread)} (snapshot {len(recorded)})")
if added:
    print(f"NEW declared-but-unread: {len(added)}")
    for name in added[:20]:
        print("   +", name)
if removed:
    print(f"in snapshot, now read: {len(removed)}")
    for name in removed[:20]:
        print("   -", name)
if added or removed:
    raise SystemExit(1)
print("matches the snapshot: no new declared-but-unread switch")
PY

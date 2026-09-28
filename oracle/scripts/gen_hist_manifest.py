#!/usr/bin/env python3
"""由上游历史累加器旁车清单生成 `crates/colm-runtime/src/history_manifest.rs`。

    python3 oracle/scripts/gen_hist_manifest.py

来源：
- `include/land_history_restart.inc` 的 `history_acc_manifest`：字段顺序、名字、秩、是否城市量；
- `main/MOD_Vars_1DAccFluxes.F90` 的 `allocate_acc_fluxes`：每个 `a_*` 的分配条件与形状。

两处各自带一层 `#ifdef`/`IF` 条件，脚本用同一个条件栈解析两边并**逐项核对**：条件不一致或
一边缺项就失败，上游改了条件不会悄悄漏过。形状按单点内核的编译期常数折成数字（与
`history::point_dimensions()` 一致，由 `history_manifest_tests.rs` 核对）。
"""
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
VENDOR = ROOT / "vendor/CoLM202X"
OUT = ROOT / "crates/colm-runtime/src/history_manifest.rs"

# 单点内核的编译期常数（`MOD_Vars_Global`：nl_soil = 10、maxsnl = -5、nl_lake = 10、nvegwcs = 4；
# `MOD_Vars_1DFluxes`：nsensor = 1）。
CONST = {"nl_soil": 10, "maxsnl": -5, "nl_lake": 10, "nvegwcs": 4, "nsensor": 1}

# 条件记号 → Rust `Requires` 变体。没列出的宏在 Rust 支持的内核里都没定义。
MACROS_UNSUPPORTED = {"CatchLateralFlow", "HYPERSPECTRAL", "DataAssimilation", "EXTERNAL_LAKE"}


def norm_if(expr):
    e = re.sub(r"[()\s]", "", expr).upper()
    if e.startswith("ALLOCATED"):
        return None   # 清单自己的 `IF (allocated(a_x))`：运行期存在性，不是配置条件
    return {
        "DEF_URBAN_RUN": "urban",
        "NUMURBAN>0": "urban",
        "DEF_USE_PFT.OR.DEF_USE_PC": "pft_or_pc",
        "DEF_USE_BGC": "bgc",
        "P_IS_WORKER": None,
        "NUMPATCH>0": None,
    }[e]


def norm_macro(line):
    m = re.match(r"#\s*if\s*\(\s*defined\s+(\w+)\s*\)", line) or re.match(r"#\s*ifdef\s+(\w+)", line)
    return m.group(1)


def walk(lines):
    """逐行给出 (当前条件集合, 行)。只认本文件用到的几种结构。"""
    stack = []
    for raw in lines:
        line = raw.split("!")[0].strip()
        low = line.lower()
        if low.startswith("#if") or low.startswith("#ifdef"):
            stack.append(("#", norm_macro(line)))
            continue
        if low.startswith("#endif"):
            assert stack and stack[-1][0] == "#", raw
            stack.pop()
            continue
        if low.startswith("#else"):
            raise SystemExit(f"unexpected #else: {raw}")
        m = re.match(r"if\s*\((.*)\)\s*then$", low)
        if m:
            stack.append(("IF", norm_if(m.group(1))))
            continue
        if re.match(r"end\s*if$", low):
            assert stack and stack[-1][0] == "IF", raw
            stack.pop()
            continue
        conds = frozenset(c for _, c in stack if c is not None)
        yield conds, line


def section(text, start, end):
    s = text.index(start)
    return text[s:text.index(end, s)].splitlines()


def manifest():
    text = (VENDOR / "include/land_history_restart.inc").read_text()
    body = section(text, "SUBROUTINE history_acc_manifest", "END SUBROUTINE history_acc_manifest")[1:]
    out = []
    for conds, line in walk(body):
        m = re.match(r"i=i\+1; names\(i\)='(\w+)'; ranks\(i\)=(\d); urban\(i\)=\.(true|false)\.", line)
        if m:
            out.append((m.group(1), int(m.group(2)), m.group(3) == "true", conds))
    return out


def dim_extent(spec):
    spec = spec.strip()
    if ":" in spec:
        lo, hi = spec.split(":")
        return value(hi) - value(lo) + 1
    return value(spec)


def value(expr):
    expr = expr.strip()
    for name, v in CONST.items():
        expr = re.sub(rf"\b{name}\b", str(v), expr)
    if not re.fullmatch(r"[-+\d ]+", expr):
        raise SystemExit(f"cannot evaluate extent {expr!r}")
    return eval(expr)


def allocations():
    text = (VENDOR / "main/MOD_Vars_1DAccFluxes.F90").read_text()
    body = section(text, "SUBROUTINE allocate_acc_fluxes", "END SUBROUTINE allocate_acc_fluxes")[1:]
    out = {}
    for conds, line in walk(body):
        m = re.match(r"allocate\s*\(\s*(\w+)\s*\((.*)\)\s*\)$", line, re.I)
        if not m or not m.group(1).startswith("a_"):
            continue
        dims = [d for d in re.split(r",", m.group(2))]
        vector = dims[-1].strip().lower()
        assert vector in ("numpatch", "numurban"), line
        extents = []
        for d in dims[:-1]:
            if "DEF_DA_ENS_NUM" in d:
                extents.append(None)
            else:
                extents.append(dim_extent(d))
        out[m.group(1)] = (conds, extents, vector == "numurban")
    return out


def history_names():
    """`MOD_Hist.F90` 里 `write_history_variable_*(…, a_X, file_hist, 'f_Y', …)`：累加器 → 历史名。

    Fortran 名字不分大小写（`a_CONC_O2_UNSAT` 写出时是 `a_conc_o2_unsat`），按小写配对。
    一个累加器写成两个历史量的只有河网的 `a_floodfrc_pch`（Rust 不支持），取字典序第一个。
    """
    text = (VENDOR / "main/MOD_Hist.F90").read_text()
    text = re.sub(r"&\s*\n\s*&?", " ", text)
    out = {}
    for acc, hist in re.findall(r"\b(a_\w+)\s*,\s*file_hist\s*,\s*'f_(\w+)'", text):
        out.setdefault(acc.lower(), set()).add(hist)
    return {acc: sorted(names)[0] for acc, names in out.items()}


def requires(conds):
    macros = {c for c in conds if c not in ("urban", "pft_or_pc", "bgc")}
    if macros & MACROS_UNSUPPORTED:
        return "Unsupported"
    rest = conds - macros
    crop = "CROP" in macros
    assert macros <= {"CROP"}, conds
    key = (frozenset(rest), crop)
    table = {
        (frozenset(), False): "Always",
        (frozenset({"urban"}), False): "Urban",
        (frozenset({"pft_or_pc"}), False): "PftOrPc",
        (frozenset({"bgc"}), False): "Bgc",
        (frozenset({"bgc"}), True): "BgcCrop",
    }
    if key not in table:
        raise SystemExit(f"unmapped condition {conds}")
    return table[key]


def main():
    fields = manifest()
    alloc = allocations()
    hist = history_names()
    rows = []
    for name, rank, urban, conds in fields:
        if name in ("nac_ln", "nac_dt"):
            rows.append((name, rank, urban, "Always", (0, 0), None))
            continue
        if name not in alloc:
            raise SystemExit(f"{name} is in the manifest but not allocated in allocate_acc_fluxes")
        aconds, extents, aurban = alloc[name]
        if requires(conds) != requires(aconds) or urban != aurban:
            raise SystemExit(f"{name}: manifest condition {sorted(conds)} vs allocation {sorted(aconds)}")
        if len(extents) != rank - 1:
            raise SystemExit(f"{name}: rank {rank} but allocated with {len(extents)} leading dims")
        req = requires(conds)
        dims = [e if e is not None else 0 for e in extents] + [0, 0]
        if req != "Unsupported" and 0 in dims[:rank - 1]:
            raise SystemExit(f"{name}: unresolved extent")
        rows.append((name, rank, urban, req, (dims[0], dims[1]), hist.get(name.lower())))
    missing = sorted(set(alloc) - {r[0] for r in rows})
    if missing:
        raise SystemExit(f"allocated but not in the manifest: {missing}")
    lines = [
        "//! 历史累加器旁车清单（上游 `history_acc_manifest` 与 `allocate_acc_fluxes`）。",
        "//!",
        "//! GENERATED by `oracle/scripts/gen_hist_manifest.py` — do not edit by hand.",
        "",
        "use crate::history_sidecar::{ManifestEntry, Requires};",
        "",
        "/// 上游清单顺序；旁车按这个顺序写出已分配的字段。",
        f"pub static MANIFEST: [ManifestEntry; {len(rows)}] = [",
    ]
    for name, rank, urban, req, (n1, n2), history in rows:
        history = f"Some(\"{history}\")" if history else "None"
        lines.append(
            f"    ManifestEntry {{ name: \"{name}\", rank: {rank}, urban: {str(urban).lower()}, "
            f"requires: Requires::{req}, n1: {n1}, n2: {n2}, history: {history} }},"
        )
    lines.append("];")
    OUT.write_text("\n".join(lines) + "\n")
    subprocess.run(["rustfmt", "--edition", "2021", str(OUT)], check=True)
    print(f"wrote {OUT.relative_to(ROOT)}: {len(rows)} fields")


if __name__ == "__main__":
    sys.exit(main())

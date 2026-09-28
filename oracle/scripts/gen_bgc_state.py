#!/usr/bin/env python3
"""从 `vendor/CoLM202X/main/BGC/MOD_BGC_Vars_*.F90` 生成 Rust 的 BGC 状态结构。

生成物是 `crates/colm-core/src/bgc_state_generated.rs`，**不要手改**：改上游声明后重跑本脚本。

为什么生成而不是手写：五个模块共约 1000 个 module 数组，形状与初值都写在 `allocate` 行里
（`allocate (x (nl_soil,numpft)) ; x(:,:) = spval`）。手抄既慢又会漏，而逐位移植要求每个数组的
初值（`spval`/`spval_i4`/`.false.`）与维度顺序都和上游一致。

约定：
- 字段名沿用 Fortran 名，移植代码因此能与上游逐行对照；
- 多维数组按 Fortran 列主序展平；`numpatch` 维被去掉（运行期一个状态对应一个 patch），
  `numpft` 维保留为最外层（最后一维）；
- `real(r8)` → `Vec<f64>`，`integer` → `Vec<i32>`，`logical` → `Vec<bool>`。
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BGC = ROOT / "vendor/CoLM202X/main/BGC"
OUT = ROOT / "crates/colm-core/src/bgc_state_generated.rs"

MODULES = [
    ("MOD_BGC_Vars_PFTimeVariables.F90", "BgcPftTimeVariables", "pft"),
    ("MOD_BGC_Vars_1DPFTFluxes.F90", "BgcPftFluxes", "pft"),
    ("MOD_BGC_Vars_TimeVariables.F90", "BgcPatchTimeVariables", "patch"),
    ("MOD_BGC_Vars_1DFluxes.F90", "BgcPatchFluxes", "patch"),
    ("MOD_BGC_Vars_TimeInvariants.F90", "BgcPatchTimeInvariants", "patch"),
]

DIMS = {
    "nl_soil": "dims.nl_soil",
    "nl_soil_full": "dims.nl_soil_full",
    "ndecomp_pools": "dims.ndecomp_pools",
    "ndecomp_transitions": "dims.ndecomp_transitions",
    "numpft": "npft",
    "numpatch": None,
    "365": "365",
    "2": "2",
}

DECL = re.compile(
    r"^\s*(real\(r8\)|integer|logical)\s*,\s*allocatable\s*::\s*([A-Za-z0-9_]+)", re.I
)
ALLOC = re.compile(
    r"allocate\s*\(\s*([A-Za-z0-9_]+)\s*\(([^)]*)\)\s*\)\s*;?\s*(?:[A-Za-z0-9_]+\s*\([^)]*\)\s*=\s*([-.A-Za-z0-9_]+))?",
    re.I,
)


def parse(path):
    types, shapes, inits, order = {}, {}, {}, []
    for line in path.read_text(errors="replace").splitlines():
        code = line.split("!")[0]
        m = DECL.match(code)
        if m:
            kind = m.group(1).lower()
            types[m.group(2)] = {"real(r8)": "f64", "integer": "i32", "logical": "bool"}[kind]
            continue
        for m in ALLOC.finditer(code):
            name = m.group(1)
            if name in shapes:
                continue
            shapes[name] = [d.strip() for d in m.group(2).split(",")]
            inits[name] = (m.group(3) or "").strip()
            order.append(name)
    return [(n, types[n], shapes[n], inits[n]) for n in order if n in types]


def init_value(kind, init):
    low = init.lower()
    if kind == "f64":
        if low in ("spval", ""):
            return "MISSING"
        return f"{float(low.replace('_r8', ''))!r}_f64" if low else "MISSING"
    if kind == "i32":
        if low in ("spval_i4", ""):
            return "-9999"
        return str(int(low))
    return "true" if low == ".true." else "false"


def length(shape):
    parts = []
    for dim in shape:
        mapped = DIMS.get(dim)
        if dim not in DIMS:
            sys.exit(f"unknown dimension {dim}")
        if mapped is not None:
            parts.append(mapped)
    return " * ".join(parts) if parts else "1"


def emit(struct, level, fields):
    out = []
    out.append(f"/// `{struct}`：见文件头。")
    out.append("#[derive(Debug, Clone, PartialEq)]")
    out.append(f"pub struct {struct} {{")
    for name, kind, shape, _ in fields:
        out.append(f"    /// `{name}({', '.join(shape)})`")
        out.append(f"    pub {name}: Vec<{kind}>,")
    out.append("}")
    out.append("")
    args = "npft: usize, dims: BgcDims" if level == "pft" else "dims: BgcDims"
    out.append(f"impl {struct} {{")
    out.append(f"    pub fn new({args}) -> Self {{")
    if level == "pft":
        out.append("        let _ = npft;")
    out.append("        let _ = dims;")
    out.append("        Self {")
    for name, kind, shape, init in fields:
        out.append(f"            {name}: vec![{init_value(kind, init)}; {length(shape)}],")
    out.append("        }")
    out.append("    }")
    for kind in ("f64", "i32", "bool"):
        names = [n for n, k, _, _ in fields if k == kind]
        out.append("")
        out.append(f"    /// 按 Fortran 名取 `{kind}` 数组（重启 I/O 用）。")
        out.append(f"    pub fn {kind}_field(&self, name: &str) -> Option<&Vec<{kind}>> {{")
        out.append("        match name {")
        for n in names:
            out.append(f'            "{n}" => Some(&self.{n}),')
        out.append("            _ => None,")
        out.append("        }")
        out.append("    }")
        out.append("")
        out.append(f"    pub fn {kind}_field_mut(&mut self, name: &str) -> Option<&mut Vec<{kind}>> {{")
        out.append("        match name {")
        for n in names:
            out.append(f'            "{n}" => Some(&mut self.{n}),')
        out.append("            _ => None,")
        out.append("        }")
        out.append("    }")
    out.append("")
    out.append("    /// 全部字段名与 Fortran 形状（按声明顺序）。")
    out.append("    pub const FIELDS: &'static [(&'static str, &'static [&'static str])] = &[")
    for name, _, shape, _ in fields:
        dims = ", ".join(f'"{d}"' for d in shape)
        out.append(f'        ("{name}", &[{dims}]),')
    out.append("    ];")
    out.append("}")
    out.append("")
    return out


SCALAR = re.compile(r"^\s*(real\(r8\)|integer)\s*::\s*([A-Za-z0-9_]+)\s*(?:!.*)?$", re.I)


def parse_scalars(path):
    """模块级标量（`CONTAINS` 之前、非 allocatable、非 parameter）。"""
    scalars = []
    for line in path.read_text(errors="replace").splitlines():
        if line.strip().upper().startswith("CONTAINS"):
            break
        m = SCALAR.match(line)
        if m:
            kind = "f64" if m.group(1).lower().startswith("real") else "i32"
            scalars.append((m.group(2), kind))
    return scalars


def emit_constants(scalars):
    out = ["/// `MOD_BGC_Vars_TimeInvariants` 的模块级标量（全局常数重启 `*_restart_bgc_const_lc*.nc`）。"]
    out.append("#[derive(Debug, Clone, PartialEq, Default)]")
    out.append("pub struct BgcConstants {")
    for name, kind in scalars:
        out.append(f"    pub {name}: {kind},")
    out.append("}")
    out.append("")
    out.append("impl BgcConstants {")
    for kind in ("f64", "i32"):
        names = [n for n, k in scalars if k == kind]
        out.append(f"    pub fn {kind}_field_mut(&mut self, name: &str) -> Option<&mut {kind}> {{")
        out.append("        match name {")
        for n in names:
            out.append(f'            "{n}" => Some(&mut self.{n}),')
        out.append("            _ => None,")
        out.append("        }")
        out.append("    }")
        out.append("")
        out.append(f"    pub fn {kind}_field(&self, name: &str) -> Option<{kind}> {{")
        out.append("        match name {")
        for n in names:
            out.append(f'            "{n}" => Some(self.{n}),')
        out.append("            _ => None,")
        out.append("        }")
        out.append("    }")
        out.append("")
    out.append("}")
    out.append("")
    return out


def main():
    lines = [
        "// @generated by oracle/scripts/gen_bgc_state.py from vendor/CoLM202X/main/BGC/MOD_BGC_Vars_*.F90.",
        "// Do not edit by hand; rerun the generator after the upstream declarations change.",
        "#![allow(clippy::all, non_snake_case, missing_docs)]",
        "",
        "use crate::MISSING;",
        "",
        "/// BGC 状态的非 patch、非 PFT 维度（`MOD_Vars_Global`）。",
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]",
        "pub struct BgcDims {",
        "    pub nl_soil: usize,",
        "    pub nl_soil_full: usize,",
        "    pub ndecomp_pools: usize,",
        "    pub ndecomp_transitions: usize,",
        "}",
        "",
        "impl Default for BgcDims {",
        "    fn default() -> Self {",
        "        Self { nl_soil: 10, nl_soil_full: 15, ndecomp_pools: 7, ndecomp_transitions: 10 }",
        "    }",
        "}",
        "",
    ]
    for file, struct, level in MODULES:
        fields = parse(BGC / file)
        lines.extend(emit(struct, level, fields))
    lines.extend(emit_constants(parse_scalars(BGC / "MOD_BGC_Vars_TimeInvariants.F90")))
    OUT.write_text("\n".join(lines))
    print(f"wrote {OUT} ({sum(1 for l in lines if l.strip().startswith('pub ') and ': Vec<' in l)} fields)")


if __name__ == "__main__":
    main()

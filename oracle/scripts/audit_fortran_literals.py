#!/usr/bin/env python3
"""审计 Fortran 数值字面量与 Rust 端口的一致性（按 **f64 位** 比，不按文本）。

为什么需要它：`MOD_Qsadv.F90` 的 `c8`（第 83 轮）与 `d5`（第 86 轮）都是
**小数点后多写/少写一个 0**，也就是 10 倍的系数错，两次都是靠这套机械比较
抓到的 —— 肉眼审 9 个系数的 4 张表不现实，而读代码时两者长得一模一样。

用法::

    python3 oracle/scripts/audit_fortran_literals.py            # 两种模式都跑
    python3 oracle/scripts/audit_fortran_literals.py --tree     # 只跑"整棵树"模式
    python3 oracle/scripts/audit_fortran_literals.py --tables   # 只跑"系数表"模式

两种模式：

1. **整棵树**（`--tree`）：对每个 Rust 源文件，取出它文档注释里引用的
   `MOD_xxx.F90`，把这些 Fortran 文件里的十进制字面量转成 f64 位，再去
   **整个 Rust 树**里找同值的字面量。找不到的就是嫌疑（或"那一支没移植"）。
   只保留有效位数 ≥4 的字面量，压掉 `0.5`/`2.0` 这种噪声。

2. **系数表**（`--tables`）：只扫 Fortran 的 `data` / `parameter ::` 行，
   但**不设位数下限** —— `data scat_sno /0.8, 0.4/` 这种短系数正是模式 1 的盲区。

已知的"正常缺失"（不是缺陷）：

* `3.14159`：Rust 写成 `314_159.0 / 100_000.0`（`radiation.rs` 的 `FORTRAN_PI`）；
* `2.2204460492503131E-16`：Rust 用 `f64::EPSILON`；
* `MOD_UserSpecifiedForcing` 里的 `212.`/`10800.`/`21600.`：那些是 ERA5/WFDE5
  等**其它数据集**的预处理分支，本仓库只支持 `POINT`；
* `MOD_SnowLayersCombineDivide` 的 `c1`/`c6`/`eta0`：上游只在**注释掉的行**里用
  （`!* ddz2 = .../eta0`），Rust 用的是同一函数里真正生效的那条公式。

**已知的粒度局限**：模式 1 按**文件**取"应出现的字面量"，所以引用 `MOD_Utils.F90`
这种三千行的工具库时，文件里别的子程序（`pnorm`、`cotan` 之类）的字面量也会被算进来，
自然找不到。看到这一类，先确认那些行是不是你要找的那个子程序。
"""

from __future__ import annotations

import argparse
import glob
import os
import re
import struct

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
VENDOR = os.path.join(ROOT, "vendor", "CoLM202X")

# Fortran 的 `1.5e3` / `.5` / Rust 的 `1.5f64` 都要认；下划线是 Rust 的数字分隔符，
# 先剥掉再解析（`f77(0.014_306_423_4)` 这种写法在这个仓库里到处都是）。
LITERAL = re.compile(
    r"(?<![\w.])(\d+\.\d*(?:[eEdD][-+]?\d+)?|\.\d+(?:[eEdD][-+]?\d+)?)(?:f32|f64)?(?!\w)"
)

MIN_DIGITS_TREE = 4
MIN_DIGITS_TABLES = 1

# 这些模块的表由 `xtask gen-*` 生成、并由四个 drift 测试**逐字节**守住
# （`cargo test -p colm-schema --test drift` 等），拿字面量集合比没有意义：
# 生成器会把 Fortran 的 `1.306E+02` 写成 `130.6`，文本不同、值相同，
# 而真正的保证在那四个测试里。模式 1 跳过它们。
GENERATED_TABLE_MODULES = {
    "MOD_Const_LC",
    "MOD_Const_PFT",
    "MOD_MonthlyinSituCO2MaunaLoa",
    "MOD_dataSpec_PDB",
}

# 这些模块的常数在 Rust 里是**具名**常量（`LATENT_HEAT_VAPORIZATION_J_KG` 之类），
# 不再重复字面量，所以按值比一定"找不到"。
NAMED_CONSTANT_MODULES = {"MOD_Const_Physical"}


def literals(text: str, *, fortran: bool, min_digits: int) -> dict[str, set[str]]:
    """把文本里的十进制字面量映射成 {f64 位的十六进制: {原始写法}}。"""
    if fortran:
        text = re.sub(r"!.*", "", text)
    else:
        text = re.sub(r"//.*", "", text)
        text = re.sub(r"(?<=\d)_(?=\d)", "", text)
    out: dict[str, set[str]] = {}
    for match in LITERAL.finditer(text):
        raw = match.group(1)
        try:
            value = float(raw.replace("d", "e").replace("D", "e"))
        except ValueError:
            continue
        if value == 0.0 or abs(value) >= 1e30:  # spval 之类不参与
            continue
        mantissa = raw.split("e")[0].split("E")[0].lstrip("0.")
        if len(re.sub(r"[^0-9]", "", mantissa)) < min_digits:
            continue
        out.setdefault(struct.pack(">d", value).hex(), set()).add(raw)
    return out


def rust_tree(min_digits: int) -> dict[str, set[str]]:
    have: dict[str, set[str]] = {}
    for pattern in ("crates/*/src/*.rs", "crates/*/src/**/*.rs", "oracle/src/**/*.rs", "xtask/src/*.rs"):
        for path in glob.glob(os.path.join(ROOT, pattern), recursive=True):
            if path.endswith("_tests.rs"):
                continue
            for key, raws in literals(open(path, errors="ignore").read(), fortran=False, min_digits=min_digits).items():
                have.setdefault(key, set()).update(raws)
    return have


def find_fortran(module: str) -> str | None:
    hits = glob.glob(os.path.join(VENDOR, "**", module + ".F90"), recursive=True)
    return hits[0] if hits else None


def report(missing: dict[str, set[str]], have: dict[str, set[str]]) -> None:
    for key in sorted(missing, key=lambda k: -abs(struct.unpack(">d", bytes.fromhex(k))[0]))[:12]:
        value = struct.unpack(">d", bytes.fromhex(key))[0]
        print(f"     {value!r:26s} 出现于 {sorted(missing[key])[:2]}")


def audit_tree() -> int:
    print("=== 模式 1：整棵树（每个 Rust 文件 vs 它引用的 Fortran 模块）")
    have = rust_tree(MIN_DIGITS_TREE)
    total = 0
    for path in sorted(glob.glob(os.path.join(ROOT, "crates/colm-core/src/*.rs"))):
        if path.endswith("_tests.rs"):
            continue
        text = open(path, errors="ignore").read()
        modules = sorted(set(re.findall(r"(MOD_[A-Za-z0-9_]+)\.F90", text)))
        modules = [
            m
            for m in modules
            if m not in GENERATED_TABLE_MODULES and m not in NAMED_CONSTANT_MODULES
        ]
        files = [f for f in (find_fortran(m) for m in modules) if f]
        if not files:
            continue
        want: dict[str, set[str]] = {}
        for fortran in files:
            for key, raws in literals(open(fortran, errors="ignore").read(), fortran=True, min_digits=MIN_DIGITS_TREE).items():
                want.setdefault(key, set()).update(raws)
        missing = {k: v for k, v in want.items() if k not in have}
        if missing:
            print(f"== {os.path.basename(path)}: {len(missing)}/{len(want)} 个 Fortran 字面量在 Rust 树里找不到")
            report(missing, have)
            total += len(missing)
    print(f"--- 合计 {total} 个未匹配")
    return total


TABLE_MODULES = [
    "MOD_Albedo",
    "MOD_SoilSnowHydrology",
    "MOD_NewSnow",
    "MOD_RainSnowTemp",
    "MOD_SoilThermalParameters",
    "MOD_SoilSurfaceResistance",
    "MOD_Const_LC",
    "MOD_PhaseChange",
    "MOD_GroundTemperature",
    "MOD_LeafInterception_Extended",
    "MOD_Qsadv",
    "MOD_PlantHydraulic",
    "MOD_OrbCoszen",
    "MOD_SnowLayersCombineDivide",
]


def audit_tables() -> int:
    print("=== 模式 2：系数表（`data` / `parameter ::` 行，位数不限）")
    have = rust_tree(MIN_DIGITS_TABLES)
    total = 0
    for module in TABLE_MODULES:
        path = find_fortran(module)
        if not path:
            print(f"!! {module}.F90 没找到")
            continue
        rows = []
        for line in open(path, errors="ignore").read().splitlines():
            stripped = re.sub(r"!.*", "", line)
            if "data " in stripped.lower() or "parameter ::" in stripped:
                rows.append(stripped)
        want = literals("\n".join(rows), fortran=True, min_digits=MIN_DIGITS_TABLES)
        missing = {k: v for k, v in want.items() if k not in have}
        if not missing:
            print(f"== {module}.F90: {len(want)} 个字面量全部找到")
            continue
        print(f"== {module}.F90: {len(missing)}/{len(want)} 个找不到")
        report(missing, have)
        total += len(missing)
    print(f"--- 合计 {total} 个未匹配")
    return total


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tree", action="store_true", help="只跑整棵树模式")
    parser.add_argument("--tables", action="store_true", help="只跑系数表模式")
    args = parser.parse_args()
    total = 0
    if not args.tables:
        total += audit_tree()
    if not args.tree:
        total += audit_tables()
    print(f"总计 {total} 个未匹配字面量（先按文档里的正常缺失清单排除，再逐个看）")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

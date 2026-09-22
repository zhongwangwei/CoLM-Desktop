#!/usr/bin/env python3
"""两侧 history 的**分步分歧**度量：比 "超容差变量数" 细，用来裁决 1 ULP 级改动。

用法：
    python3 oracle/scripts/window_divergence.py <内核 history.nc> <Rust history.nc> [--top N]

输出两类东西：

1. **全局首次分歧步**（`first divergence step`）—— 所有变量、所有时刻里最早出现
   逐位不同的那个时间下标。对"两个实现只差若干次舍入"的比较，这是最锐利的标量：
   某个表达式的形状不对，就会在某一步把它自己暴露出来；改对了它就该往后推。
   11 天窗口的 `tier2 变量数` 做不到这一点 —— 干窗是混沌的，1 ULP 扰动会在
   某一步放大，`ot_vars` 于是被个别变量翻转（见 `docs/implementation-verification.md`
   里 `MOD_LeafTemperature` 那轮的五个变体实测）。

2. **逐变量**：首次分歧步、逐位不同的元素个数、最大绝对差、最大**相对**差。
   相对差用来把 `f_olrg`（量级几百）和 `f_gssun`（量级 1e-5）放在同一把尺子上。

排序是 `(首次分歧步, maxrel)` **升序**：`maxrel` 停在 2e-16 附近的变量就是**种子本身**
（1 ULP ≈ 2.2e-16），1e-14 以上的都是被放大的下游。**不要按变量名截断看** ——
按名字排会把种子挤到 `--top` 之外（本工具第一版就是这么误导了一次记录）。

约定：把 `--top` 之外的变量折叠成一行计数；`shape` 不一致的变量单独标出。
"""

import argparse
import sys

import netCDF4 as nc
import numpy as np


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("kernel")
    parser.add_argument("rust")
    parser.add_argument("--top", type=int, default=20)
    args = parser.parse_args()

    kernel = nc.Dataset(args.kernel)
    rust = nc.Dataset(args.rust)
    names = sorted(set(kernel.variables) & set(rust.variables))

    rows = []
    total = exact = 0
    for name in names:
        a = np.ma.filled(kernel.variables[name][:].astype("f8"), np.nan)
        b = np.ma.filled(rust.variables[name][:].astype("f8"), np.nan)
        if a.shape != b.shape:
            rows.append((0, name, -1, float("nan"), float("nan"), "shape"))
            continue
        if a.size == 0:
            continue
        same = a.view(np.int64) == b.view(np.int64)
        total += a.size
        exact += int(same.sum())
        if same.all():
            continue
        different = ~same
        index = np.argwhere(different)
        first = int(index[:, 0].min())
        count = int(different.sum())
        delta = np.abs(a - b)
        max_abs = float(np.nanmax(delta))
        scale = np.abs(a)
        with np.errstate(divide="ignore", invalid="ignore"):
            relative = np.where(scale > 0, delta / scale, np.where(delta > 0, np.inf, 0.0))
        max_rel = float(np.nanmax(relative))
        rows.append((first, name, count, max_abs, max_rel, ""))

    rows.sort(key=lambda row: (row[0], row[4] if row[5] != "shape" else float("-inf")))
    print(f"{'first':>6s} {'variable':30s} {'ndiff':>7s} {'maxabs':>12s} {'maxrel':>10s}")
    for first, name, count, max_abs, max_rel, note in rows[: args.top]:
        if note == "shape":
            print(f"{first:6d} {name:30s} {'shape':>7s}")
        else:
            print(f"{first:6d} {name:30s} {count:7d} {max_abs:12.4e} {max_rel:10.2e}")
    if len(rows) > args.top:
        rest = rows[args.top :]
        print(f"      ... 另有 {len(rest)} 个变量有差异（maxrel "
              f"{min(float(r[4]) for r in rest):.2e} … "
              f"{max(float(r[4]) for r in rest):.2e}）")

    differing = [r for r in rows if r[5] != "shape"]
    if differing:
        print(f"first divergence step: {min(r[0] for r in differing)}")
    else:
        print("first divergence step: none (all values bitwise identical)")
    print(f"variables differing: {len(rows)}; bitwise identical: {exact}/{total} "
          f"({100.0 * exact / total:.4f}%)")
    return 0


if __name__ == "__main__":
    sys.exit(main())

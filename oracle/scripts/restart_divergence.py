#!/usr/bin/env python3
"""内核与 Rust 的 **restart 文件**逐位比对：比 `window_divergence.py` 更锐的种子口径。

用法：
    python3 oracle/scripts/restart_divergence.py <内核 restart.nc> <Rust restart.nc> [--top N]

为什么单列一个口径：干窗第 0 步的 history 有 44 个变量差、且带 11 天窗口的混沌放大；
而第 0 步结束时的 **restart** 只有 19/68 个变量差，多数只有一个元素差（`t_soisno`/`tleaf`
恰是 1 ULP）。候选形状改动先用这个口径过一遍，改对了这个数应当下降；history 与黄金窗口
再用作二次确认。

输出按 `maxrel` 升序：≈2e-16 是**种子本身**（1 ULP ≈ 2.2e-16），越大越是被放大的下游。
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
    for name in names:
        a = np.ma.filled(kernel.variables[name][:].astype("f8"), np.nan)
        b = np.ma.filled(rust.variables[name][:].astype("f8"), np.nan)
        if a.shape != b.shape or a.size == 0:
            continue
        same = a.view(np.int64) == b.view(np.int64)
        if same.all():
            continue
        delta = np.abs(a - b)
        scale = np.abs(a)
        with np.errstate(divide="ignore", invalid="ignore"):
            relative = np.where(scale > 0, delta / scale, np.where(delta > 0, np.inf, 0.0))
        rows.append(
            (float(np.nanmax(relative)), name, int((~same).sum()), float(np.nanmax(delta)))
        )

    rows.sort()
    print(f"{'maxrel':>10s} {'variable':28s} {'ndiff':>7s} {'maxabs':>12s}")
    for max_rel, name, count, max_abs in rows[: args.top]:
        print(f"{max_rel:10.3e} {name:28s} {count:7d} {max_abs:12.4e}")
    if len(rows) > args.top:
        rest = rows[args.top :]
        print(f"      ... 另有 {len(rest)} 个变量有差异（maxrel "
              f"{rest[0][0]:.2e} … {rest[-1][0]:.2e}）")
    print(f"differing restart variables: {len(rows)} / {len(names)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

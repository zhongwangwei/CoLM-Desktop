#!/usr/bin/env python3
"""两份第三段 restart 的三分类比较。

```bash
python3 oracle/scripts/stage3_diff.py <initial.nc> <fortran-final.nc> <rust-final.nc>
```

把「逐变量相同 / Rust 没写（保持初值）/ 两边都算但不同」分开，是判读 Rust 第三段
唯一可行的办法：把三者混成一个「有多少变量不等」，会把

* **写出集合的差**（Rust 有意只换它推进过的量，见
  `assembly.rs::evolved_overrides` 的注释）与
* **数值发散**（同一个从同一初值出发的窗口，两边算出了不同的状态）

混成一堆，而这两件的处置完全不同：前者是把该写的量补上，后者是内核里的公式或接线错了。

`<initial.nc>` 是两份比较共同的起点（mkinidata 写出的那一份）。第三份文件必须与
Fortran 那份的窗口终点一致，否则比较的不是同一个窗口。
"""
import sys

import netCDF4
import numpy as np


def flat(dataset, name):
    return np.asarray(dataset.variables[name][:], dtype=float).ravel()


def main(argv):
    if len(argv) != 4:
        print(__doc__)
        return 2
    initial, fortran, rust = (netCDF4.Dataset(path) for path in argv[1:])
    shared = sorted(set(fortran.variables) & set(rust.variables))
    only_fortran = sorted(set(fortran.variables) - set(rust.variables))
    only_rust = sorted(set(rust.variables) - set(fortran.variables))
    if only_fortran or only_rust:
        print(f"变量集合不同：只在 Fortran {only_fortran}；只在 Rust {only_rust}")

    exact, untouched, diverge = [], [], []
    for name in shared:
        f, r, i = flat(fortran, name), flat(rust, name), flat(initial, name)
        if f.size != r.size:
            diverge.append((name, float("nan"), f, r))
        elif np.array_equal(f, r):
            exact.append(name)
        elif np.array_equal(r, i):
            untouched.append(name)
        else:
            scale = np.maximum(np.abs(f), 1e-30)
            diverge.append((name, float(np.nanmax(np.abs(f - r) / scale)), f, r))

    print(f"共享变量 {len(shared)}：逐位相同 {len(exact)}，"
          f"Rust 未写出（保持初值）{len(untouched)}，两边都算但不同 {len(diverge)}")
    if untouched:
        print("\nRust 未写出的变量（写出集合的差）：")
        print("  " + ", ".join(untouched))
    if diverge:
        print("\n两边都算但不同（按相对偏差排序）：")
        np.set_printoptions(precision=6, linewidth=200)
        for name, rel, f, r in sorted(diverge, key=lambda row: -(row[1] or 0.0)):
            print(f"  {name:16s} rel={rel:9.3e}  fort={f[:3]}  rust={r[:3]}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))

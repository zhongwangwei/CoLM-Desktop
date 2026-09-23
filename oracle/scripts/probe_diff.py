#!/usr/bin/env python3
"""通用两侧探针比较器：格式 `TAG n k1 k2 f1 f2 ... fN`。

用法: python3 oracle/scripts/probe_diff.py <fort.txt> <rust.txt> [--max N]

* 键是 `(tag, n, k1, k2)`；tag 的次序按它在**内核文件里首次出现**的先后，
  所以报出来的"第一条不同"就是执行序上的第一条。
* 值一律按 IEEE 位型比较（打印用 17 位有效数字，`float()` 解析后逐位相等才算相同）。
* 两侧行数/字段数不一致时直接报出来 —— 那是补丁落点不同（第 279 轮的坑），
  不是数据差。
"""

import struct
import sys


def bits(value: float) -> bytes:
    return struct.pack("<d", value)


def parse(path: str):
    order = []
    records = {}
    for line in open(path, errors="replace"):
        parts = line.split()
        if len(parts) < 4:
            continue
        tag = parts[0]
        key = (tag, int(parts[1]), int(parts[2]), int(parts[3]))
        if tag not in order:
            order.append(tag)
        records.setdefault(key, []).append([float(x) for x in parts[4:]])
    return order, records


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__)
        return 2
    limit = 8
    if "--max" in sys.argv:
        limit = int(sys.argv[sys.argv.index("--max") + 1])
    fort_order, fort = parse(sys.argv[1])
    rust_order, rust = parse(sys.argv[2])
    if fort_order != rust_order:
        print(f"!! tag sets differ: kernel={fort_order} rust={rust_order}")
    counts: dict[str, list[int]] = {}
    for tag, _, _, _ in fort:
        counts.setdefault(tag, [0, 0])[0] += 1
    for tag, _, _, _ in rust:
        counts.setdefault(tag, [0, 0])[1] += 1
    for tag in fort_order:
        k, r = counts.get(tag, [0, 0])
        mark = "" if k == r else "   <-- 行数不同（补丁落点不同？）"
        print(f"  {tag:8s} kernel={k:5d} rust={r:5d}{mark}")
    rank = {tag: index for index, tag in enumerate(fort_order)}
    shared = sorted(set(fort) & set(rust), key=lambda key: (rank.get(key[0], 99), key[1], key[2], key[3]))
    if not shared:
        print("no shared records")
        return 1
    ndiff = 0
    first = None
    for key in shared:
        tag = key[0]
        if len(fort[key]) != 1 or len(rust[key]) != 1:
            print(f"  {key} 重复行（内核 {len(fort[key])} / 本仓库 {len(rust[key])}）")
            continue
        a, b = fort[key][0], rust[key][0]
        if len(a) != len(b):
            ndiff += 1
            first = first or key
            if ndiff <= limit:
                print(f"  {key} 字段数不同 K={len(a)} R={len(b)}")
            continue
        diffs = [(i, x, y) for i, (x, y) in enumerate(zip(a, b)) if bits(x) != bits(y)]
        if diffs:
            ndiff += 1
            if first is None:
                first = key
            if ndiff <= limit:
                for index, x, y in diffs:
                    print(f"  {key} f{index} K={x!r:26s} R={y!r:26s}")
    print(f"  {ndiff}/{len(shared)} records differ; first = {first}")
    return 0 if ndiff == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""`stomata` 两侧同位型比较（hex 位型 / 十进制两档，自动识别）。

内核行：`STIN/STPH/STIT/STOUT  n  [ic]  f...`
Rust 行：`SRIN/SRPH/SRIT/SROUT  n  [ic]  f...`（第 351 轮起两侧字段布局逐位对齐）

**hex 档**（第 351 轮）：两侧字段都是 16 位十六进制 ⇒ **逐位**比较（`==`），
这才是 1 ULP 的判据 —— 原版的 `ES23.15`/`{:e}` + 2e-14 相对容差对 1 ULP 是盲的。
**十进制档**（第 293 轮的老用法）保留：`key=value` 形式 + 2e-14 相对容差。

用法: python3 oracle/scripts/stomata_cmp.py <kernel_probe.txt> <rust_probe.txt>
"""
import re
import sys

PAIR = {"STIN": "SRIN", "STPH": "SRPH", "STIT": "SRIT", "STOUT": "SROUT"}
REV = {v: k for k, v in PAIR.items()}   # Rust 标签 -> 内核标签（两侧都按内核名归档）
K_FIELDS = {
    "STIN": ["tlef", "psrf", "po2m", "pco2m", "pco2a", "ea", "ei", "par", "rb", "ra",
             "rstfac", "c1", "c2", "c3"],
    "STPH": ["vm", "epar", "respc", "omss", "gbh2o", "gammas", "rrkk", "c3", "c4", "bintc", "range"],
    "STIT": ["pco2i", "omc", "ome", "oms", "assim", "assimn", "co2s", "co2st", "assmt",
             "hcdma", "gsh2o", "es", "pco2in", "eyy"],
    "STOUT": ["gsh2o", "rst", "tprcor", "tlef"],
}
# hex 档的字段名（上表的子集，去掉上游未用/未在两侧同时导出的量）
H_FIELDS = {
    "STIN": ["tlef", "psrf", "po2m", "pco2m", "pco2a", "ea", "ei", "par", "rb", "rstfac",
             "c1", "c2", "c3"],
    "STPH": K_FIELDS["STPH"],
    "STIT": ["pco2i", "omc", "ome", "assim", "assimn", "co2s", "assmt", "gsh2o", "pco2in", "eyy"],
    "STOUT": K_FIELDS["STOUT"],
}
ALIAS = {"co2st": "co2s"}
HEX = re.compile(r"^[0-9A-Fa-f]{16}$")


def to_float(x):
    """gfortran 的 `ES23.15` 对 3 位指数（次正规数）会打成 `1.23e-314` -> `1.23-314`。"""
    try:
        return float(x)
    except ValueError:
        m = re.match(r"^([-\d.]+)([+-]\d+)$", x)
        if m:
            return float(m.group(1) + "E" + m.group(2))
        raise


def split_rows(path, kernel):
    """返回 (mode, rows)：rows[tag][key] = [token...]；key=(n, ic)。"""
    rows = {}
    mode = None
    for line in open(path, errors="replace"):
        q = line.split()
        if not q:
            continue
        tag = q[0]
        if tag not in PAIR and tag not in PAIR.values():
            continue
        n = int(q[1]) - (1 if kernel else 0)
        if tag in ("STIT", "SRIT"):
            ic, rest = int(q[2]), q[3:]
        else:
            ic, rest = 0, q[2:]
        # 内核行本来就带内核标签；Rust 行要映回内核名，两侧才按同一个键归档
        key_tag = tag if kernel else REV.get(tag, tag)
        if rest and all(HEX.match(x) for x in rest):
            mode = "hex"
            rows.setdefault(key_tag, {})[(n, ic)] = [x.upper() for x in rest]
        else:
            mode = mode or "dec"
            kv = {}
            for k, v in re.findall(r"(\w+)=\s*([-\w.eE+]+)", line):
                if k != "ic":
                    kv[k] = float(v)
            if kv:
                rows.setdefault(key_tag, {})[(n, ic)] = kv
            else:
                rows.setdefault(key_tag, {})[(n, ic)] = [to_float(x) for x in rest]
    return mode, rows


def close(a, b):
    if a == b:
        return True
    return abs(a - b) / max(abs(a), abs(b), 1e-300) <= 2e-14


def main():
    kernel, rust = sys.argv[1], sys.argv[2]
    kmode, kk = split_rows(kernel, True)
    rmode, rr = split_rows(rust, False)
    mode = rmode if rmode == "hex" else kmode
    names = H_FIELDS if mode == "hex" else K_FIELDS
    print(f"模式: {mode}")
    bad = 0
    for tag in PAIR:
        k_rows, r_rows = kk.get(tag, {}), rr.get(tag, {})
        keys = sorted(set(k_rows) & set(r_rows))
        first, ndiff = None, 0
        for key in keys:
            a, b = k_rows[key], r_rows[key]
            diffs = []
            for i, name in enumerate(names[tag]):
                if mode == "hex":
                    if i >= len(a) or i >= len(b):
                        continue
                    if a[i] != b[i]:
                        diffs.append((name, a[i], b[i]))
                else:
                    rname = ALIAS.get(name, name)
                    if i >= len(a) or rname not in b:
                        continue
                    if not close(a[i], b[rname]):
                        diffs.append((name, a[i], b[rname]))
            if diffs:
                ndiff += 1
                if first is None:
                    first = (key, diffs)
        head = f"{tag:6s} {len(k_rows):7d} {len(r_rows):7d} {len(keys):7d}  "
        if first is None:
            print(head + f"identical ({len(keys)} calls)")
        else:
            key, diffs = first
            name, a, b = diffs[0]
            print(head + f"n={key[0]} ic={key[1]} 首个差异 {name}: K={a} R={b}"
                         f"；{ndiff}/{len(keys)} 组有差异，字段 {[d[0] for d in diffs]}")
            bad += 1
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())

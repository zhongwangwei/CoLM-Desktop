#!/usr/bin/env python3
"""第 293 轮：`stomata` 遮荫支两侧同位型比较。

内核行：STIN/STPH/STIT/STOUT  n  [ic]  f...
Rust 行：SRIN/SRPH/SRIT/SROUT  n  [ic=..]  key=value...

按 (tag, n, ic) 对齐；内核计数器从 1 起、Rust 从 0 起，故内核 n-1。
只比两侧都打印的量（相对 tol 2e-14）。
"""
import re
import sys

PAIR = {"STIN": "SRIN", "STPH": "SRPH", "STIT": "SRIT", "STOUT": "SROUT"}
K_FIELDS = {
    "STIN": ["tlef", "psrf", "po2m", "pco2m", "pco2a", "ea", "ei", "par", "rb", "ra",
             "rstfac", "c1", "c2", "c3"],
    "STPH": ["vm", "epar", "respc", "omss", "gbh2o", "gammas", "rrkk", "c3", "c4", "bintc", "range"],
    "STIT": ["pco2i", "omc", "ome", "oms", "assim", "assimn", "co2s", "co2st", "assmt",
             "hcdma", "gsh2o", "es", "pco2in", "eyy"],
    "STOUT": ["gsh2o", "rst", "tprcor", "tlef"],
}
# 内核字段名 -> Rust 键名（缺省同名）
ALIAS = {"co2st": "co2s"}


def to_float(x):
    """gfortran 的 `ES23.15` 对 3 位指数（次正规数）会打成 `1.23e-314` -> `1.23-314`。"""
    try:
        return float(x)
    except ValueError:
        m = re.match(r"^([-\d.]+)([+-]\d+)$", x)
        if m:
            return float(m.group(1) + "E" + m.group(2))
        raise


def parse_kernel(path):
    out = {}
    for line in open(path, errors="replace"):
        q = line.split()
        if not q or q[0] not in PAIR:
            continue
        tag = q[0]
        n = int(q[1]) - 1
        if tag == "STIT":
            key = (n, int(q[2]))
            floats = [to_float(x) for x in q[3:]]
        else:
            key = (n, 0)
            floats = [to_float(x) for x in q[2:]]
        out.setdefault(tag, {})[key] = floats
    return out


def parse_rust(path):
    out = {}
    for line in open(path, errors="replace"):
        q = line.split()
        if not q or q[0] not in PAIR.values():
            continue
        tag = q[0]
        n = int(q[1])
        kv = {}
        for k, v in re.findall(r"(\w+)=\s*([-\w.eE+]+)", line):
            if k == "ic":
                continue
            kv[k] = float(v)
        ic = 0
        m = re.search(r"\bic=\s*(\d+)", line)
        if m:
            ic = int(m.group(1))
        out.setdefault(tag, {})[(n, ic)] = kv
    return out


def close(a, b):
    if a == b:
        return True
    return abs(a - b) / max(abs(a), abs(b), 1e-300) <= 2e-14


def main():
    fk, fr = parse_kernel(sys.argv[1]), parse_rust(sys.argv[2])
    print(f"{'tag':6s} {'kernel':>7s} {'rust':>7s} {'shared':>7s}  first-divergence")
    bad = 0
    for ktag, rtag in PAIR.items():
        kk, rr = fk.get(ktag, {}), fr.get(rtag, {})
        keys = sorted(set(kk) & set(rr))
        first = None
        ndiff = 0
        for key in keys:
            kf, rv = kk[key], rr[key]
            diffs = []
            for i, name in enumerate(K_FIELDS[ktag]):
                rname = ALIAS.get(name, name)
                if i >= len(kf) or rname not in rv:
                    continue
                if not close(kf[i], rv[rname]):
                    diffs.append((name, kf[i], rv[rname]))
            if diffs:
                ndiff += 1
                if first is None:
                    first = (key, diffs)
        head = f"{ktag:6s} {len(kk):7d} {len(rr):7d} {len(keys):7d}  "
        if first is None:
            print(head + f"identical ({len(keys)} calls)")
        else:
            key, diffs = first
            name, a, b = diffs[0]
            print(head + f"n={key[0]} ic={key[1]} ({name}): K={a!r} R={b!r}; {ndiff}/{len(keys)} differ")
            bad += 1
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""比较 `vsf_richards_probe.sh` 两侧的 `Richards_solver` 内部探针记录。

用法: python3 oracle/scripts/vsf_richards_cmp.py <fort.txt> <rust.txt> [--max N]

格式统一为 `TAG call key1 key2 f1 f2 ... fN`（键是整数，值是 17 位有效数字的浮点）。
按**执行顺序**（tag rank → call → key1 → key2）逐条比对，逐位相等才算相同；
只报前若干条不同的记录以及每条里第一个不同的字段 —— 第一条不同的记录就是
分歧的出生点。

判读：`RCHL/RCHE/RCHZ` 是状态、`RCHB` 是质量残差、`RCHD` 是 Newton 步长。
第一个不同的 tag 决定下一枪：
  WSF1/WSFE ⇒ 分歧在**入场之前**（`etr`/`rootflux`/`rsubst` 就是上游给的），
              别再往 Richards 里钻；
  RCHF/RCHB ⇒ 分歧在 `flux_all`/`water_balance`；
  RCHD      ⇒ 分歧在 Jacobian/最小二乘；
  RCHE      ⇒ 分歧在 Newton 更新或 `check_and_update_level`；
  RCHX      ⇒ 走到显式回退那一支了。
"""

import struct
import sys

TAG_RANK = {
    "WSF1": 0,
    "WSFE": 1,
    "RCH0": 2,
    "RCHL": 3,
    "RCHF": 4,
    "RCHB": 5,
    "RCHD": 6,
    "RCHE": 7,
    "RCHX": 8,
    "RCHZ": 9,
}

FIELDS = {
    "WSF1": ["qgtop", "etr", "rsubst", "ss_dp", "zwt", "wa", "tolerance"],
    "WSFE": ["ss_vliq", "rootflux", "porsl", "psi_s", "hksat"],
    "RCH0": [
        "layers",
        "ubc_typ",
        "lbc_typ",
        "dt",
        "ubc_val",
        "lbc_val",
        "ss_dp",
        "waquifer",
    ],
    "RCHL": ["ss_vl", "ss_wt", "ss_wf"],
    "RCHF": ["dt_this", "f2", "ss_dp", "waquifer"],
    "RCHB": ["blc"],
    "RCHD": ["dv", "dr_dv_ii", "vact"],
    "RCHE": ["ss_vl", "ss_wt", "ss_wf", "psi", "hk", "is_sat"],
    "RCHX": ["dt_this", "ss_vl", "ss_wt", "ss_wf"],
    "RCHZ": ["ss_vl", "ss_wt", "ss_wf", "ss_q_l"],
}


def bits(value: float) -> bytes:
    return struct.pack("<d", value)


def parse(path: str):
    records = {}
    counts = {}
    with open(path, errors="replace") as handle:
        for line in handle:
            parts = line.split()
            if not parts or parts[0] not in TAG_RANK:
                continue
            tag = parts[0]
            key = (tag, int(parts[1]), int(parts[2]), int(parts[3]))
            records[key] = [float(x) for x in parts[4:]]
            counts[tag] = counts.get(tag, 0) + 1
    return records, counts


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__)
        return 2
    limit = 6
    if "--max" in sys.argv:
        limit = int(sys.argv[sys.argv.index("--max") + 1])
    fort, fort_counts = parse(sys.argv[1])
    rust, rust_counts = parse(sys.argv[2])
    print("kernel records:", " ".join(f"{k}={fort_counts[k]}" for k in sorted(fort_counts)))
    print("rust   records:", " ".join(f"{k}={rust_counts[k]}" for k in sorted(rust_counts)))
    shared = sorted(set(fort) & set(rust), key=lambda k: (TAG_RANK[k[0]], k[1], k[2], k[3]))
    if not shared:
        print("no shared records")
        return 1
    ndiff = 0
    first = None
    for key in shared:
        tag = key[0]
        a, b = fort[key], rust[key]
        names = FIELDS[tag]
        if len(a) != len(b):
            print(f"  {key} field-count mismatch K={len(a)} R={len(b)}")
            ndiff += 1
            first = first or key
            continue
        diffs = [(n, x, y) for n, x, y in zip(names, a, b) if bits(x) != bits(y)]
        if diffs:
            ndiff += 1
            if first is None:
                first = key
            if ndiff <= limit:
                for name, x, y in diffs:
                    print(f"  {key} {name:10s} K={x!r:26s} R={y!r:26s}")
    print(f"  {ndiff}/{len(shared)} records differ; first differing record = {first}")
    return 0 if ndiff == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""比较 `vsf_probe.sh` 两侧的 `WSF`/`WSF0` 记录。

用法: python3 oracle/scripts/vsf_cmp.py <fort.txt> <rust.txt> [--max-diff N]

判读纪律：**只信逐位**。两侧的打印都用 17 位有效数字，
`float()` 解析后按位比较，不用任何相对误差。
先按 call 号找**第一条**有差异的记录，再报出该 call 内**第一个**不同的字段 ——
`soil_water_vertical_movement` 出场处的 `ss_vliq`/`ss_wt`/`smp`/`hk`/`qlayer`
就是这条链的出口，第一个不同的字段决定下一枪往哪钻。
"""

import struct
import sys

LEVEL_FIELDS = [
    "ss_vliq",
    "ss_wt",
    "smp",
    "hk",
    "qlayer_l",
    "qlayer_u",
    "porsl",
]
CALL_FIELDS = ["nlev", "wa", "zwt", "ss_dp", "qinfl", "wblc", "tol_v"]


def bits(x: float) -> bytes:
    return struct.pack("<d", x)


def parse(path: str):
    """返回 {call: {"call": [...], "levels": {ilev: [...]}}}。"""
    out = {}
    with open(path, errors="replace") as handle:
        for line in handle:
            q = line.split()
            if not q:
                continue
            if q[0] == "WSF0":
                n = int(q[1])
                out.setdefault(n, {})["call"] = [int(q[2])] + [float(x) for x in q[3:]]
            elif q[0] == "WSF":
                n, ilev = int(q[1]), int(q[2])
                out.setdefault(n, {}).setdefault("levels", {})[ilev] = [float(x) for x in q[3:]]
    return out


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__)
        return 2
    fort, rust = parse(sys.argv[1]), parse(sys.argv[2])
    print(f"kernel calls {len(fort)} / rust calls {len(rust)}")
    shared = sorted(set(fort) & set(rust))
    if not shared:
        print("no shared call numbers")
        return 1
    first = None
    ndiff = 0
    for call in shared:
        f, r = fort[call], rust[call]
        diffs = []
        if "call" in f and "call" in r and len(f["call"]) == len(r["call"]):
            diffs += [
                (name, a, b)
                for name, a, b in zip(CALL_FIELDS, f["call"], r["call"])
                if bits(a) != bits(b)
            ]
        fl, rl = f.get("levels", {}), r.get("levels", {})
        for ilev in sorted(set(fl) & set(rl)):
            diffs += [
                (f"L{ilev}.{name}", a, b)
                for name, a, b in zip(LEVEL_FIELDS, fl[ilev], rl[ilev])
                if bits(a) != bits(b)
            ]
        if diffs:
            ndiff += 1
            if first is None:
                first = call
            if first == call or ndiff <= 4:
                for name, a, b in diffs[:12]:
                    print(f"  call {call} {name:16s} K={a!r:26s} R={b!r:26s}")
    print(f"  {ndiff}/{len(shared)} calls differ; first differing call = {first}")
    return 0 if ndiff == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""第 292 轮：gssun 六点探针两侧对齐比较。

内核行格式：TAG  n  [it]  f1 f2 ...
Rust 行格式：TAG  n  it=.. key=value ...
按 (tag, n) 对齐；对每个标签只比两侧都有的量（相对 tol 2e-14）。
"""
import re
import sys

KERNEL_TAGS = {"GSIN", "GSDEM", "GSOUT", "GS908", "GS919", "GS941", "GS1320", "GSTO"}
RUST_TAGS = {"GRIN", "GRDEM", "GROUT", "GR908", "GR919", "GR941", "GR1320", "GRST"}
PAIR = {"GSIN": "GRIN", "GSDEM": "GRDEM", "GSOUT": "GROUT", "GS908": "GR908",
        "GS919": "GR919", "GS941": "GR941", "GS1320": "GR1320", "GSTO": "GRST"}
K_FIELDS = {
    "GSIN": ["gs0sun", "gs0sha", "gssun", "gssha", "laisun", "laisha", "tl", "psrf", "sai", "fwet"],
    "GSDEM": ["qflx_sun", "qflx_sha", "gssun", "gssha", "gb", "tl", "psrf", "laisun", "laisha", "rss"],
    "GSOUT": ["gssun", "gssha", "etrsun", "etrsha", "qflx_sun", "qflx_sha", "psrf", "tl", "laisun", "laisha"],
    "GS908": ["gssun_can", "gssha_can", "laisun", "laisha", "tl", "tprcor", "sai", "fwet", "lai", "o3coefg"],
    "GS919": ["gssun", "gssha", "rssun", "rssha", "tl", "tprcor", "laisun", "laisha", "rb", "rbsun"],
    "GS941": ["rssun", "rssha", "laisun", "laisha", "rb", "tprcor", "tl", "gssun", "gssha", "lai"],
    "GS1320": ["gssun", "gssha", "rssun", "rssha", "laisun", "laisha", "tlbef", "tprcor", "tl", "lai"],
    "GSTO": ["parsun", "parsha", "rssun", "rssha", "eah", "tl", "psrf", "rb", "rbsha", "lai"],
}
R_KEYS = {
    "GSIN": [("gs0sun", "gs0sun"), ("gs0sha", "gs0sha"), ("laisun", "laisun"),
             ("laisha", "laisha"), ("tl", "tl"), ("psrf", "psrf")],
    "GSDEM": [("qflx_sun", "qflx_sun"), ("qflx_sha", "qflx_sha"), ("gssun", "gssun"),
              ("gssha", "gssha"), ("gb", "gb"), ("tl", "tl"),
              ("laisun", "laisun"), ("laisha", "laisha")],
    "GSOUT": [("gssun", "gssun"), ("gssha", "gssha"), ("etrsun", "etrsun"),
              ("etrsha", "etrsha"), ("qflx_sun", "qflx_sun"), ("qflx_sha", "qflx_sha"),
              ("tl", "tl"), ("laisun", "laisun"), ("laisha", "laisha")],
    "GS908": [("gssun_can", "gssun_can"), ("gssha_can", "gssha_can"),
              ("laisun", "laisun"), ("laisha", "laisha"), ("tl", "tl")],
    "GS919": [("rssun", "rssun"), ("rssha", "rssha"), ("tl", "tl"),
              ("laisun", "laisun"), ("laisha", "laisha")],
    "GS941": [("rssun", "rssun"), ("rssha", "rssha"), ("laisun", "laisun"), ("laisha", "laisha")],
    "GS1320": [("gssun", "gssun"), ("gssha", "gssha"), ("rssun", "rssun"),
               ("rssha", "rssha"), ("laisun", "laisun"), ("laisha", "laisha"),
               ("tlbef", "tlbef"), ("tprcor", "tprcor")],
    "GSTO": [("parsun", "parsun"), ("parsha", "parsha"), ("rssun", "rssun"), ("rssha", "rssha"),
             ("eah", "eah"), ("tl", "tl"), ("psrf", "psrf"), ("rb", "rb"), ("rbsha", "rbsha")],
}


def parse_kernel(path):
    out = {}
    for line in open(path, errors="replace"):
        q = line.split()
        if not q or q[0] not in KERNEL_TAGS:
            continue
        tag = q[0]
        vals = q[1:]
        # 有 `it` 的标签：n it f...
        if tag in ("GS908", "GS919", "GS941", "GS1320", "GSTO"):
            n, it, floats = int(vals[0]), int(vals[1]), [float(x) for x in vals[2:]]
        else:
            n, it, floats = int(vals[0]), None, [float(x) for x in vals[1:]]
        if tag == "GS1320":
            n = len(out.get(tag, {}))  # 该标签按出现次序对齐（Rust 侧是 lt_call）
        elif tag != "GSTO":
            n -= 1  # 内核计数器从 1 起，Rust 从 0 起；GSTO 在 PHS 调用之前打，已是 0 基
        out.setdefault(tag, {})[n] = (it, floats)
    return out


def parse_rust(path):
    out = {}
    for line in open(path, errors="replace"):
        q = line.split()
        if not q or q[0] not in RUST_TAGS:
            continue
        tag = q[0]
        n = int(q[1])
        it = None
        kv = {}
        for k, v in re.findall(r"(\w+)=\s*([-\w.eE+]+)", line):
            if k == "it":
                it = int(v)
            else:
                kv[k] = float(v)
        out.setdefault(tag, {})[n] = (it, kv)
    return out


def close(a, b):
    if a == b:
        return True
    scale = max(abs(a), abs(b), 1e-300)
    return abs(a - b) / scale <= 2e-14


def main():
    fk, fr = parse_kernel(sys.argv[1]), parse_rust(sys.argv[2])
    print(f"{'tag':8s} {'kernel':>7s} {'rust':>7s} {'shared':>7s}  first-divergence")
    bad = 0
    for ktag, rtag in PAIR.items():
        kk, rr = fk.get(ktag, {}), fr.get(rtag, {})
        keys = sorted(set(kk) & set(rr))
        first = None
        ndiff = 0
        for n in keys:
            kit, kf = kk[n]
            rit, rv = rr[n]
            for kname, rname in R_KEYS[ktag]:
                idx = K_FIELDS[ktag].index(kname)
                if idx >= len(kf) or rname not in rv:
                    continue
                if not close(kf[idx], rv[rname]):
                    ndiff += 1
                    if first is None:
                        first = (n, kname, kf[idx], rv[rname], kit, rit)
                    break
        line = f"{ktag:8s} {len(kk):7d} {len(rr):7d} {len(keys):7d}  "
        if first is None:
            line += f"identical ({len(keys)} calls)"
        else:
            n, kname, a, b, kit, rit = first
            line += (f"call {n} ({kname}): K={a!r} R={b!r} "
                     f"(it K={kit} R={rit}); {ndiff}/{len(keys)} calls differ")
            bad += 1
        print(line)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())

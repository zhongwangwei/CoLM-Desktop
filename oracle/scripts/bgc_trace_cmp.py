#!/usr/bin/env python3
"""读取/比对 `gen_bgc_trace.py` 格式的 BGC 逐阶段追踪。

    bgc_trace_cmp.py A.bin            # 列出记录
    bgc_trace_cmp.py A.bin B.bin      # 逐位比对，报告第一个分叉的记录及该记录里所有不同的字段
    bgc_trace_cmp.py A.bin B.bin -a   # 报告每条记录的不同字段

两边都有值（n>0）且长度相同的字段才比；一边 n=0（数组未分配）不算分叉。比较按位进行，
NaN 同位即相等。
"""

import struct
import sys
from pathlib import Path


def read(path):
    """返回 [(阶段名, {字段名: 原始字节})]；物理输入与 BGC 状态合在一张表里。"""
    data = Path(path).read_bytes()
    pos, records = 0, []
    while pos < len(data):
        tag = data[pos:pos + 32].decode().strip()
        pos += 32
        values = {}
        for _section in range(2):
            while True:
                (length,) = struct.unpack_from("<i", data, pos)
                pos += 4
                if length == 0:
                    break
                name = data[pos:pos + length].decode()
                pos += length
                (n,) = struct.unpack_from("<i", data, pos)
                pos += 4
                values[name] = data[pos:pos + 8 * n]
                pos += 8 * n
        records.append((tag, values))
    return records


def floats(raw):
    return struct.unpack(f"<{len(raw) // 8}d", raw)


def diff_fields(a, b):
    out = []
    for name, ra in a.items():
        rb = b.get(name, b"")
        if not ra or not rb or ra == rb:
            continue
        if len(ra) != len(rb):
            out.append((name, None, len(ra) // 8, len(rb) // 8))
            continue
        fa, fb = floats(ra), floats(rb)
        first = next(k for k in range(len(fa)) if ra[8 * k:8 * k + 8] != rb[8 * k:8 * k + 8])
        count = sum(ra[8 * k:8 * k + 8] != rb[8 * k:8 * k + 8] for k in range(len(fa)))
        out.append((name, first, fa[first], fb[first], count, len(fa)))
    return out


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("-")]
    every = "-a" in sys.argv
    ra = read(args[0])
    if len(args) == 1:
        for k, (tag, _) in enumerate(ra):
            print(k, tag)
        return
    rb = read(args[1])
    call = 0
    for k, ((ta, va), (tb, vb)) in enumerate(zip(ra, rb)):
        if ta == "begin":
            call += 1
        if ta != tb:
            print(f"record {k}: stage mismatch {ta!r} vs {tb!r}")
            return
        d = diff_fields(va, vb)
        if not d:
            continue
        print(f"record {k} (call {call}, after {ta}): {len(d)} fields differ")
        for item in d:
            if item[1] is None:
                print(f"  {item[0]}: length {item[2]} vs {item[3]}")
            else:
                name, idx, x, y, count, n = item
                print(f"  {name}[{idx}]: {x!r} vs {y!r}  ({count}/{n} differ)")
        if not every:
            return
    if len(ra) != len(rb):
        print(f"record counts differ: {len(ra)} vs {len(rb)}")
    else:
        print(f"{len(ra)} records identical")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""gx.py FILE FUNC [FROM TO]: GIMPLE (-fdump-tree-optimized-lineno) -> readable expressions.

Stores to arrays and assignments to named locals are printed with SSA temporaries expanded.
"""
import re
import sys

path, func = sys.argv[1], sys.argv[2].lower()
lo = int(sys.argv[3]) if len(sys.argv) > 3 else 0
hi = int(sys.argv[4]) if len(sys.argv) > 4 else 10**9

lines = open(path).read().splitlines()
body, on = [], False
for l in lines:
    if l.startswith(";; Function "):
        on = l.split()[2].lower() == func
        continue
    if on:
        body.append(l)

LOC = re.compile(r"^\s*\[[^\]]*\.F90:(\d+):\d+(?: discrim \d+)?\]\s*(.*)$")
defs = {}   # ssa -> expr string (raw rhs)
order = []  # (line, lhs, rhs)
for l in body:
    m = LOC.match(l)
    if m:
        ln, stmt = int(m.group(1)), m.group(2).rstrip(";").strip()
    elif l.startswith("  ") and "=" in l:
        ln, stmt = -1, l.strip().rstrip(";")
    else:
        continue
    stmt = re.sub(r"\[[^\]]*\.F90:\d+:\d+(?: discrim \d+)?\]\s*", "", stmt)
    if stmt.startswith("# DEBUG") or "CLOBBER" in stmt:
        continue
    phi = re.match(r"#\s*(\S+)\s*=\s*PHI\s*<(.*)>", stmt)
    if phi:
        args = [re.sub(r"\(\d+\)$", "", a.strip()) for a in phi.group(2).split(",")]
        defs[phi.group(1)] = "phi(" + ", ".join(args) + ")"
        continue
    a = re.match(r"(.+?)\s=\s(.+)$", stmt)
    if not a:
        continue
    lhs, rhs = a.group(1).strip(), a.group(2).strip()
    if re.match(r"^[A-Za-z_][\w.]*$", lhs):
        defs[lhs] = rhs
    if ln >= 0:
        order.append((ln, lhs, rhs))

TOKEN = re.compile(r"[A-Za-z_][\w.]*(?:\(D\))?")


def base(name):
    """Named SSA like rootfr_tot_584 or q10.5_12 -> rootfr_tot / q10."""
    n = re.sub(r"\(D\)$", "", name)
    n = re.sub(r"\.\d+_\d+$", "", n)
    n = re.sub(r"_\d+$", "", n)
    return n


def is_temp(name):
    return re.match(r"^(_|pretmp_|prephitmp_|M\.\d+_|cstore_)\d+$", name) is not None


def array_of(ptr, depth=0, seen=None):
    """Resolve a data pointer SSA to the array name (follows casts and pointer arithmetic)."""
    seen = seen or set()
    if ptr in seen or depth > 12:
        return None
    seen.add(ptr)
    rhs = defs.get(ptr)
    if rhs is None or (depth > 0 and rhs.startswith("MEM")):
        # 载入的值是下标（如 donor_pool(k)），不是基址
        return None
    m = re.match(r"^([A-Za-z_][\w]*)\.data$", rhs)
    if m:
        return m.group(1)
    m = re.match(r"^&([A-Za-z_]\w*)", rhs)
    if m:
        return m.group(1)
    for tok in re.findall(r"[A-Za-z_][\w.]*_\d+|_\d+", rhs):
        name = array_of(tok, depth + 1, seen)
        if name:
            return name
    return None


def arr(ptr):
    return array_of(ptr) or base(ptr)


def mem(text):
    # MEM <real(kind=8)[0:]> [(real(kind=8)[0:] *)_1][_5]  or MEM[(real(kind=8) *)_7 + 8B]
    m = re.match(r"MEM\s*<[^>]*>\s*\[\(.*?\*\)([\w.]+)\]\[([^\]]+)\]", text)
    if m:
        return f"{arr(m.group(1))}[..]"
    m = re.match(r"MEM\s*\[\(.*?\*\)([\w.]+)(.*)\]$", text)
    if m:
        return f"{arr(m.group(1))}[..]"
    m = re.match(r"MEM\s*<[^>]*>\s*\[\(.*?\*\)([\w.]+)(?:\s*\+\s*(\d+)B)?\]", text)
    if m:
        return f"{arr(m.group(1))}[..]"
    return None


def expand(expr, depth=0):
    expr = expr.strip()
    if depth > 40:
        return expr
    mm = mem(expr)
    if mm:
        return mm
    m = re.match(r"^\*\s*([\w.]+(?:\(D\))?)$", expr)
    if m:
        return "*" + base(m.group(1))
    m = re.match(r"^([\w.]+)\[([^\]]+)\]$", expr)  # module array woody[_52]
    if m:
        return f"{m.group(1)}[..]"
    m = re.match(r"^\(\((.*)\)\)$", expr)
    if m:
        return expand(m.group(1), depth + 1)
    m = re.match(r"^\((real|integer|logical)\(kind=\d\)\)\s*(\S+)$", expr)
    if m:
        return f"{m.group(1)[0]}({expand(m.group(2), depth + 1)})"
    m = re.match(r"^(\S+)\s+([-+*/]|w\*)\s+(\S+)$", expr)
    if m:
        return f"({expand(m.group(1), depth + 1)} {m.group(2)} {expand(m.group(3), depth + 1)})"
    m = re.match(r"^-(\S+)$", expr)
    if m and not re.match(r"^-?[\d.]", expr):
        return f"-{expand(m.group(1), depth + 1)}"
    m = re.match(r"^phi\((.*)\)$", expr)
    if m:
        # phi 的各入口只展开一层，避免指数级膨胀
        args = split_args(m.group(1))
        return "phi(" + ", ".join(a.strip() if depth > 1 else expand(a, 39) for a in args) + ")"
    m = re.match(r"^(\.?[A-Za-z_]\w*)\s*[<(](.*)[>)]$", expr)
    if m:
        args = split_args(m.group(2))
        fn = m.group(1).replace("__builtin_", "")
        return f"{fn}(" + ", ".join(expand(a, depth + 1) for a in args) + ")"
    if is_temp(expr) and expr in defs:
        return expand(defs[expr], depth + 1)
    if re.match(r"^[\w.]+(\(D\))?$", expr):
        if expr in defs and re.match(r"^[A-Za-z_]\w*$", defs[expr]):
            return defs[expr]
        if re.match(r"^-?\d", expr):
            return short_num(expr)
        return base(expr)
    return expr


def short_num(s):
    if re.match(r"^-?\d+$", s):
        return s
    try:
        v = float(s)
        r = repr(v)
        return r
    except ValueError:
        return s


def split_args(s):
    out, depth, cur = [], 0, ""
    for ch in s:
        if ch in "(<[":
            depth += 1
        elif ch in ")>]":
            depth -= 1
        if ch == "," and depth == 0:
            out.append(cur)
            cur = ""
        else:
            cur += ch
    if cur.strip():
        out.append(cur)
    return out


for ln, lhs, rhs in order:
    if not (lo <= ln <= hi):
        continue
    target = None
    mm = mem(lhs)
    if mm:
        target = mm
    elif re.match(r"^D__I_lsm\.\d+_\d+$", lhs):
        target = "lsm"
    elif re.match(r"^[A-Za-z]\w*_\d+$", lhs) and not lhs.startswith(("ivtmp", "pretmp", "prephitmp", "D.", "M.", "val.", "S.")):
        target = base(lhs)
    elif re.match(r"^[A-Za-z_][\w]*\[", lhs):
        target = re.sub(r"\[.*", "[..]", lhs)
    if target is None:
        continue
    text = expand(rhs)
    if re.search(r"\.(data|offset|dim|span)\b|stride", rhs):
        continue
    print(f"{ln}: {target} = {text}")

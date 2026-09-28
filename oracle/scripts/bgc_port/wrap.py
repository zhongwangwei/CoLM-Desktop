#!/usr/bin/env python3
"""wrap.py DRAFT RUST_FN 'doc line' -> Rust function text on stdout."""
import sys, re
draft, name, doc = sys.argv[1], sys.argv[2], sys.argv[3]
body = open(draft).read().rstrip("\n")


def match(s, i):
    depth = 0
    for k in range(i, len(s)):
        if s[k] == "{":
            depth += 1
        elif s[k] == "}":
            depth -= 1
            if depth == 0:
                return k
    raise SystemExit("unbalanced")


# `IF (DEF_USE_TRACER) ... ELSE ... ENDIF`：只保留 ELSE 分支。
while True:
    i = body.find("if false /*TRACER*/")
    if i < 0:
        break
    a = body.index("{", i)
    b = match(body, a)
    m = re.match(r"\s*else\s*\{", body[b + 1:])
    if m:
        c = b + 1 + m.end() - 1
        e = match(body, c)
        inner = body[c + 1:e]
    else:
        e, inner = b, ""
    ls = body.rfind("\n", 0, i) + 1
    ind = body[ls:i]
    body = body[:ls] + ind + "// DEF_USE_TRACER 的示踪物分解分支未移植（运行时拒绝 DEF_USE_TRACER）。\n" + ind + "{" + inner + "}" + body[e + 1:]
# `#ifdef FUN` 在任何内核里都不定义。
while True:
    i = body.find("if sw.fun {")
    if i < 0:
        break
    e = match(body, body.index("{", i))
    ls = body.rfind("\n", 0, i) + 1
    body = body[:ls] + body[e + 2:]
uses_c = "c." in re.sub(r"\bs\.constants\.", "", body) and re.search(r"(?<![\w.])c\.", body)
uses_sw = re.search(r"\bsw\.", body)
uses_d = re.search(r"\bd\.", body)
uses_npft = "npft" in body
# 删掉从未出现在声明之外的局部变量
for local in re.findall(r"^\s*let mut (\w+): \w+;$", body, re.M):
    if len(re.findall(rf"\b{local}\b", body)) == 1:
        body = re.sub(rf"^\s*let mut {local}: \w+;\n", "", body, flags=re.M)
sig_c = "c" if uses_c else "_c"
sig_sw = "sw" if uses_sw else "_sw"
pre = []
if uses_d:
    pre.append("    let d = s.dims;")
if uses_npft:
    pre.append("    let npft = p.pftclass.len();")
print(f"/// {doc}")
print(f"pub fn {name}(s: &mut BgcState, p: &BgcPhysics, {sig_c}: &BgcPftConstants, {sig_sw}: BgcSwitches) {{")
print("\n".join(pre))
print(body)
print("}")

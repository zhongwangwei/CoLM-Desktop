#!/usr/bin/env python3
"""wrap.py DRAFT RUST_FN DOC：把 f2rs 的函数体套上签名，输出 Rust 函数。

RUST_FN 以 `_` 开头表示模块私有（上游模块内部调用的子程序）。
"""
import re
import sys

draft, name, doc = sys.argv[1], sys.argv[2], sys.argv[3]
body = open(draft).read().rstrip("\n")

extra, result = "", False
lines = body.split("\n")
while lines and lines[0].startswith("// ") and lines[0][3:].split(":")[0] in ("EXTRA_ARGS", "RESULT"):
    tag = lines.pop(0)[3:]
    if tag.startswith("EXTRA_ARGS: "):
        extra = ", " + tag[len("EXTRA_ARGS: "):]
    elif tag == "RESULT":
        result = True
body = "\n".join(lines)


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
    mm = re.search(r"if false /\*(\w+)\*/", body)
    i = mm.start() if mm else -1
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
    note = ("// DEF_USE_TRACER 的示踪物分解分支未移植（运行时拒绝 DEF_USE_TRACER）。"
            if mm.group(1) == "TRACER" else f"// `#ifdef {mm.group(1)}` 分支：单点 Rust 引擎里该宏未定义。")
    body = body[:ls] + ind + note + "\n" + ind + "{" + inner + "}" + body[e + 1:]
# `#ifdef FUN` 在任何内核里都不定义。
while True:
    i = body.find("if sw.fun {")
    if i < 0:
        break
    e = match(body, body.index("{", i))
    ls = body.rfind("\n", 0, i) + 1
    body = body[:ls] + body[e + 2:]

ASSIGN = r"^\s*{v}\s*(=|\+=|-=|\*=|/=)\s"


def code(line):
    return line.split("//")[0]


def tidy(body):
    """转写产物的清理（不改变任何算式）：删掉只写不读的局部变量及其赋值、多余的 `mut`、
    循环里没用到的 `class`、只剩注释的循环。草稿一行一条语句，逐行处理即可。"""
    lines = body.split("\n")
    for decl in [l for l in lines if re.match(r"^\s*let mut \w+(: \w+|\s*=.*);$", l)]:
        v = re.match(r"^\s*let mut (\w+)", decl).group(1)
        assigns = [k for k, l in enumerate(lines) if re.match(ASSIGN.format(v=v), code(l))]
        reads = 0
        for k, l in enumerate(lines):
            c = code(l)
            if l is decl or re.match(rf"^\s*let mut {v}\b", c):
                c = re.sub(rf"^\s*let mut {v}\b", "", c)
            elif k in assigns:
                c = re.sub(ASSIGN.format(v=v).replace("^", "^", 1), " ", c, count=1)
                if re.match(rf"^\s*{v}\s*(\+=|-=|\*=|/=)", code(l)):
                    pass
            reads += len(re.findall(rf"\b{v}\b", c))
        if reads == 0:
            drop = set(assigns) | {lines.index(decl)}
            lines = [l for k, l in enumerate(lines) if k not in drop]
            continue
        # 需要 mut：多次赋值、复合赋值、或在循环里赋值
        in_loop = False
        stack = []
        for k, l in enumerate(lines):
            c = code(l).strip()
            if k in assigns and any(s == "for" for s in stack):
                in_loop = True
            if c.startswith("}"):
                if stack:
                    stack.pop()
            if c.endswith("{"):
                stack.append("for" if c.startswith("for ") or ".fold(" in c else "block")
        compound = any(re.match(rf"^\s*{v}\s*(\+=|-=|\*=|/=)", code(lines[k])) for k in assigns)
        init = "=" in decl.split(":")[0] if ":" in decl else True
        count = len(assigns) + (1 if re.match(rf"^\s*let mut {v}\s*=", decl) else 0)
        if count <= 1 and not in_loop and not compound:
            k = lines.index(decl)
            indent = len(decl) - len(decl.lstrip())
            if len(assigns) == 1 and re.match(r"^\s*let mut \w+: \w+;$", decl):
                a = assigns[0]
                line = lines[a]
                if len(line) - len(line.lstrip()) == indent and re.match(rf"^\s*{v} = ", line):
                    # 声明与唯一一次同层赋值合并
                    lines[a] = line.replace(f"{v} = ", f"let {v} = ", 1)
                    del lines[k]
                    continue
            lines[k] = decl.replace("let mut ", "let ", 1)
    # 循环体里没用到的 `let class = ivt as usize;`
    k = 0
    while k < len(lines):
        if lines[k].strip() == "let class = ivt as usize;":
            depth, used, j = 0, False, k + 1
            while j < len(lines):
                c = code(lines[j])
                if re.search(r"\bclass\b", c):
                    used = True
                depth += c.count("{") - c.count("}")
                if depth < 0:
                    break
                j += 1
            if not used:
                del lines[k]
                continue
        k += 1
    # 只剩注释的 for 循环：去掉循环，保留注释
    k = 0
    while k < len(lines):
        if re.match(r"^\s*for \w+ in .*\{$", lines[k]):
            j, depth, only_comments = k + 1, 1, True
            while j < len(lines):
                c = code(lines[j]).strip()
                depth += c.count("{") - c.count("}")
                if depth == 0:
                    break
                if c:
                    only_comments = False
                j += 1
            if only_comments:
                lines = lines[:k] + lines[k + 1:j] + lines[j + 1:]
                continue
        k += 1
    return "\n".join(lines)


body = tidy(body)

scan = re.sub(r"\bs\.constants\.", "", body)
uses_c = re.search(r"(?<![\w.])c[.,)]", scan)
uses_sw = re.search(r"(?<![\w.])sw\b", scan)
uses_p = re.search(r"(?<![\w.])p[.,)]", scan)
pre = []
if re.search(r"\bd\.", body):
    pre.append("    let d = s.dims;")
if "npft" in body:
    pre.append("    let npft = p.pftclass.len();")
writes_p = re.search(r"^\s*p\.\w+\[[^\]]*\]\s*(=|\+=|-=|\*=|/=)", body, re.M)
p_ty = "&mut BgcPhysics" if writes_p else "&BgcPhysics"
vis = "fn" if name.startswith("_") else "pub fn"
name = name.lstrip("_")
ret = " -> anyhow::Result<()>" if result else ""
sig_c = "c" if uses_c else "_c"
sig_sw = "sw" if uses_sw else "_sw"
print(f"/// {doc}")
sig_p = "p" if uses_p else "_p"
print(f"{vis} {name}(s: &mut BgcState, {sig_p}: {p_ty}, {sig_c}: &BgcPftConstants, "
      f"{sig_sw}: BgcSwitches{extra}){ret} {{")
if pre:
    print("\n".join(pre))
print(body)
print("}")

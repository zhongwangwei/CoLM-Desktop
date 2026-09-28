#!/usr/bin/env python3
"""mkmod.py OUT.rs HEADER.txt  DRAFT FN DOC [DRAFT FN DOC ...]：拼模块、删未用的 use、rustfmt。"""
import re
import subprocess
import sys
from pathlib import Path

out, header = sys.argv[1], Path(sys.argv[2]).read_text()
args = sys.argv[3:]
funcs = []
for k in range(0, len(args), 3):
    funcs.append(subprocess.run([sys.executable, str(Path(__file__).with_name("wrap.py")), *args[k:k + 3]],
                                check=True, capture_output=True, text=True).stdout)
# 调用了"写 p"的函数的函数自己也要 `&mut BgcPhysics`（直到不再变化）。
changed = True
while changed:
    changed = False
    mut = {re.search(r"fn (\w+)\(", f).group(1) for f in funcs if "p: &mut BgcPhysics" in f}
    for k, f in enumerate(funcs):
        if "p: &BgcPhysics" in f and any(re.search(rf"\b{n}\(s, p", f) for n in mut):
            funcs[k] = f.replace("p: &BgcPhysics", "p: &mut BgcPhysics", 1)
            changed = True
# 调用了返回 Result 的函数的函数也返回 Result，调用处加 `?`。
changed = True
while changed:
    changed = False
    res = {re.search(r"fn (\w+)\(", f).group(1) for f in funcs if "-> anyhow::Result<()>" in f}
    for k, f in enumerate(funcs):
        for n in res:
            g = re.sub(rf"^(\s*{n}\([^;{{}}]*\));", r"\1?;", f, flags=re.M)
            g = g.replace(")??;", ")?;")
            if g != f:
                if "-> anyhow::Result<()>" not in g:
                    g = re.sub(r"\) \{\n", ") -> anyhow::Result<()> {\n", g, count=1)
                    g = g.rstrip().rstrip("}").rstrip() + "\n    Ok(())\n}\n"
                    changed = True
                if g != f:
                    funcs[k] = g
body = "\n".join(funcs)
if "vectorized_dot(" in body:
    header = header.replace("use crate::bgc_driver::{", "use crate::bgc_driver::{vectorized_dot, ", 1)
for name in ("NPCROPMIN", "MISSING", "BgcSwitches", "BgcPftConstants"):
    if not re.search(rf"\b{name}\b", body):
        header = re.sub(rf"\b{name}, |, {name}\b", "", header)
        header = re.sub(rf"^use crate::{name};\n", "", header, flags=re.M)
Path(out).write_text(header.rstrip("\n") + "\n\n" + body)
subprocess.run(["rustfmt", "--edition", "2021", out], check=True)

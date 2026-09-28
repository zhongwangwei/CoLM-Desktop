#!/usr/bin/env python3
"""mkmod.py OUT.rs HEADER.txt  DRAFT FN DOC [DRAFT FN DOC ...]"""
import subprocess, sys
out, header = sys.argv[1], open(sys.argv[2]).read()
parts = [header.rstrip("\n") + "\n"]
args = sys.argv[3:]
for k in range(0, len(args), 3):
    parts.append(subprocess.run([sys.executable, __file__.replace("mkmod.py", "wrap.py"), *args[k:k + 3]],
                                check=True, capture_output=True, text=True).stdout)
open(out, "w").write("\n".join(parts))
subprocess.run(["rustfmt", "--edition", "2021", out], check=True)

#!/usr/bin/env python3
"""f2rs.py FILE SUBROUTINE [--name rust_fn]: draft Rust for a BGC subroutine.

Restricted Fortran subset: assignments, IF/ELSE IF/ELSE/ENDIF, single-line IF, DO loops,
#ifdef CROP/#else/#endif, IF(DEF_USE_*). Expressions: + - * / **, intrinsics, comparisons,
logical ops. FMA contraction follows GCC's convert_mult_to_fma: a product whose value feeds
a +/- (not through parentheses) is fused; when both operands of +/- are products, the left
one is fused.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "oracle/scripts"))
import gen_bgc_state  # noqa

MEMBER = {"BgcPftTimeVariables": "pft", "BgcPftFluxes": "pft_flux", "BgcPatchTimeVariables": "patch",
          "BgcPatchFluxes": "patch_flux", "BgcPatchTimeInvariants": "invariants"}
FIELDS = {}
for f, struct, _ in gen_bgc_state.MODULES:
    for name, kind, shape, _ in gen_bgc_state.parse(gen_bgc_state.BGC / f):
        FIELDS[name.lower()] = (name, MEMBER[struct], kind, shape)
SCALARS = {n.lower(): (n, k) for n, k in gen_bgc_state.parse_scalars(gen_bgc_state.BGC / "MOD_BGC_Vars_TimeInvariants.F90")}

PHYS_PFT = "pftfrac tsai_p tlai_p lai_p laisun_p laisha_p sigf_p tref_p assim_p respc_p".split()
PHYS_SOIL = "porsl psi0 bsw theta_r alpha_vgm n_vgm l_vgm sc_vgm fc_vgm bd_all wfc om_density t_soisno wliq_soisno wice_soisno smp h2osoi".split()
PHYS_PATCH = "patchlatr lai tlai tref rsur rnof forc_t forc_q forc_psrf forc_prc forc_prl forc_us forc_vs".split()
PHYS_GRID = "z_soi dz_soi zi_soi".split()
PHYS_SCALAR = {"deltim": "p.deltim", "dlat": "p.dlat", "dlon": "p.dlon", "smpmax_hr": "p.smpmax_hr", "smpmin_hr": "p.smpmin_hr"}
PHYS_CASE = {"l_vgm": "L_vgm", "bd_all": "BD_all", "om_density": "OM_density"}

PFTC = set("""woody isevg issed isstd isbare iscrop isnatveg isshrub isgrass isbetr isbdtr dsladlai declfact allconsl
cc_dstem cc_leaf cc_lstem cc_other croot_stem deadwdcn fcur2 fd_pft flivewd fm_droot fm_leaf fm_lroot fm_lstem fm_other fm_root
fr_fcel fr_flab fr_flig froot_leaf frootcn fsr_pft graincn grperc grpnow laimx leaf_long leafcn lf_fcel lf_flab lf_flig lflitcn
livewdcn slatop stem_leaf""".split())
PFTC_LOGICAL = set("isevg issed isstd isbare iscrop isnatveg isshrub isgrass isbetr isbdtr".split())

DIMS = {"nl_soil": "d.nl_soil", "nl_soil_full": "d.nl_soil_full", "ndecomp_pools": "d.ndecomp_pools",
        "ndecomp_transitions": "d.ndecomp_transitions"}
INTCONST = {"npcropmin": "NPCROPMIN", "noveg": "0", "nc3crop": "15", "nc3irrig": "16", "nbedrock": "10"}
FLOATCONST = {"spval": "MISSING", "tfrz": "273.16", "zmin_bedrock": "0.4", "denh2o": "1000.0", "denice": "917.0"}
SWITCH = {"def_use_nitrif": "sw.nitrif", "def_use_sasu": "sw.sasu", "def_use_diagmatrix": "sw.diag_matrix",
          "def_use_fire": "sw.fire", "def_use_cnsoyfixn": "sw.cnsoyfixn", "def_use_fert": "sw.fert",
          "def_use_irrigation": "sw.irrigation", "def_use_laifeedback": "sw.laifeedback",
          "def_use_nostressnitrogen": "sw.nostressnitrogen", "def_use_tracer": "false /*TRACER*/"}


# ---------------------------------------------------------------- lexer/parser
TOK = re.compile(r"\s*(?:(\d+\.\d*(?:[eEdD][+-]?\d+)?(?:_r8)?|\.\d+(?:[eEdD][+-]?\d+)?(?:_r8)?|\d+[eEdD][+-]?\d+(?:_r8)?|\d+(?:_r8)?)"
                 r"|(\.and\.|\.or\.|\.not\.|\.eq\.|\.ne\.|\.lt\.|\.le\.|\.gt\.|\.ge\.|\.true\.|\.false\.)"
                 r"|(\*\*|==|/=|<=|>=|[-+*/(),<>:])|([A-Za-z_]\w*))", re.I)


def lex(s):
    out, pos = [], 0
    s = s.strip()
    while pos < len(s):
        m = TOK.match(s, pos)
        if not m or m.end() == pos:
            raise SystemExit(f"lex error at {s[pos:]!r} in {s!r}")
        pos = m.end()
        num, dot, op, name = m.groups()
        if num:
            out.append(("num", num))
        elif dot:
            d = dot.lower()
            out.append(("op", {".and.": "&&", ".or.": "||", ".not.": "!", ".eq.": "==", ".ne.": "/=", ".lt.": "<",
                               ".le.": "<=", ".gt.": ">", ".ge.": ">=", ".true.": "TRUE", ".false.": "FALSE"}[d]))
        elif op:
            out.append(("op", op))
        else:
            out.append(("name", name))
    return out


class P:
    def __init__(self, toks):
        self.t, self.i = toks, 0

    def peek(self):
        return self.t[self.i] if self.i < len(self.t) else (None, None)

    def take(self, v=None):
        tok = self.peek()
        if v is not None and tok[1] != v:
            raise SystemExit(f"expected {v} got {tok} in {self.t}")
        self.i += 1
        return tok

    def expr(self):
        return self.or_()

    def or_(self):
        a = self.and_()
        while self.peek()[1] == "||":
            self.take()
            a = ("||", a, self.and_())
        return a

    def and_(self):
        a = self.not_()
        while self.peek()[1] == "&&":
            self.take()
            a = ("&&", a, self.not_())
        return a

    def not_(self):
        if self.peek()[1] == "!":
            self.take()
            return ("!", self.not_())
        return self.cmp()

    def cmp(self):
        a = self.add()
        if self.peek()[1] in ("==", "/=", "<", "<=", ">", ">="):
            op = self.take()[1]
            return ("cmp", op, a, self.add())
        return a

    def add(self):
        if self.peek()[1] in ("-", "+"):
            op = self.take()[1]
            a = self.mul()
            a = ("neg", a) if op == "-" else a
        else:
            a = self.mul()
        while self.peek()[1] in ("+", "-"):
            op = self.take()[1]
            a = (op, a, self.mul())
        return a

    def mul(self):
        a = self.pow()
        while self.peek()[1] in ("*", "/"):
            op = self.take()[1]
            a = (op, a, self.pow())
        return a

    def pow(self):
        a = self.atom()
        if self.peek()[1] == "**":
            self.take()
            if self.peek()[1] == "-":
                self.take()
                b = ("neg", self.pow())
            else:
                b = self.pow()
            return ("**", a, b)
        return a

    def atom(self):
        kind, v = self.take()
        if kind == "num":
            return ("num", v)
        if v == "(":
            e = self.expr()
            self.take(")")
            return ("paren", e)
        if v in ("TRUE", "FALSE"):
            return ("bool", v == "TRUE")
        if kind == "name":
            if self.peek()[1] == "(":
                self.take()
                args = []
                if self.peek()[1] != ")":
                    while True:
                        if self.peek()[1] == ":":
                            self.take()
                            args.append(("colon",))
                        else:
                            args.append(self.expr())
                        if self.peek()[1] == ",":
                            self.take()
                            continue
                        break
                self.take(")")
                return ("call", v.lower(), args)
            return ("name", v.lower())
        raise SystemExit(f"bad atom {kind} {v} in {self.t}")


def parse_expr(s):
    p = P(lex(s))
    e = p.expr()
    if p.i != len(p.t):
        raise SystemExit(f"trailing tokens in {s!r}: {p.t[p.i:]}")
    return e


# ---------------------------------------------------------------- emitter
class Ctx:
    def __init__(self, locals_, loops):
        self.locals = locals_          # name -> type (f64/i32/bool)
        self.loops = loops             # set of active loop vars (0-based usize)


def is_int_lit(s):
    return re.fullmatch(r"\d+", s) is not None


def num(s, want):
    s = s.lower().replace("_r8", "").replace("d", "e")
    if want == "i32" and is_int_lit(s):
        return s
    if is_int_lit(s):
        return s + ".0"
    if s.startswith("."):
        s = "0" + s
    if re.match(r"^\d+\.e", s):
        s = s.replace(".e", ".0e")
    if s.endswith("."):
        s += "0"
    return s


def typeof(e, ctx):
    k = e[0]
    if k == "num":
        return "i32" if is_int_lit(e[1].lower().replace("_r8", "")) and "_r8" not in e[1].lower() else "f64"
    if k == "bool":
        return "bool"
    if k in ("cmp", "&&", "||", "!"):
        return "bool"
    if k == "paren":
        return typeof(e[1], ctx)
    if k == "neg":
        return typeof(e[1], ctx)
    if k in ("+", "-", "*", "/", "**"):
        a, b = typeof(e[1], ctx), typeof(e[2], ctx)
        return "f64" if "f64" in (a, b) else "i32"
    if k == "name":
        n = e[1]
        if n in ctx.loops:
            return "i32"
        if n in ctx.locals:
            return ctx.locals[n]
        if n == "ivt":
            return "i32"
        if n in INTCONST or n in DIMS:
            return "i32"
        if n in SCALARS:
            return SCALARS[n][1]
        if n in FIELDS:
            return FIELDS[n][2]
        return "f64"
    if k == "call":
        n = e[1]
        if n in FIELDS:
            return FIELDS[n][2]
        if n in PFTC_LOGICAL:
            return "bool"
        if n in ("int", "nint", "floor"):
            return "i32"
        if n in ("real", "dble", "exp", "log", "sqrt", "abs", "amin1", "amax1", "sin", "cos", "log10"):
            return "f64" if n != "abs" else typeof(e[2][0], ctx)
        if n in ("max", "min"):
            ts = [typeof(a, ctx) for a in e[2]]
            return "f64" if "f64" in ts else "i32"
        if n == "pftclass":
            return "i32"
        if n in ("isleapyear", "isendofyear"):
            return "bool"
        return "f64"
    return "f64"


def index(arg, ctx):
    """0-based usize index expression for a Fortran subscript."""
    if arg[0] == "name" and arg[1] in ctx.loops:
        return arg[1]
    if arg[0] == "num":
        return str(int(arg[1]) - 1)
    if arg[0] in ("+", "-") and arg[1][0] == "name" and arg[1][1] in ctx.loops and arg[2][0] == "num":
        return f"{arg[1][1]} {arg[0]} {arg[2][1]}"
    return f"({emit(arg, ctx, 'i32')} - 1) as usize"


DIMSIZE = {"nl_soil": "d.nl_soil", "nl_soil_full": "d.nl_soil_full", "ndecomp_pools": "d.ndecomp_pools",
           "ndecomp_transitions": "d.ndecomp_transitions", "numpft": "npft", "365": "365", "2": "2"}


def field_ref(name, args, ctx):
    fname, member, kind, shape = FIELDS[name]
    dims = [d for d in shape if d != "numpatch"]
    idx = []
    for a, d in zip(args, shape):
        if d == "numpatch":
            continue
        if d == "numpft":
            idx.append(index(a, ctx))
        else:
            idx.append(index(a, ctx))
    if not dims:
        flat = "0"
    else:
        terms, stride = [], []
        for n, (ix, d) in enumerate(zip(idx, dims)):
            if n == 0:
                terms.append(ix)
            else:
                stride = ' * '.join(DIMSIZE[x] for x in dims[:n])
                if ix == "0":
                    continue
                if ix == "1":
                    terms.append(stride)
                else:
                    terms.append(f"{stride} * {ix}" if re.fullmatch(r"\w+", ix) else f"{stride} * ({ix})")
        flat = " + ".join(terms)
    return f"s.{member}.{fname}[{flat}]"


def phys_ref(name, args, ctx):
    rust = PHYS_CASE.get(name, name)
    if name in PHYS_PFT:
        return f"p.{rust}[{index(args[0], ctx)}]"
    if name in PHYS_SOIL:
        return f"p.{rust}[{index(args[0], ctx)}]"
    if name in PHYS_PATCH:
        return f"p.{rust}[0]"
    if name in PHYS_GRID:
        return f"p.{rust}[{index(args[0], ctx)}]"
    if name == "rootfr_p":
        return f"p.rootfr_p[{index(args[0], ctx)} + d.nl_soil * m]"
    if name == "pftclass":
        return f"p.pftclass[{index(args[0], ctx)}]"
    if name == "idate":
        return f"p.idate[{index(args[0], ctx)}]"
    return None


def emit(e, ctx, want="f64", fma_ok=True):
    k = e[0]
    if k == "num":
        return num(e[1], want)
    if k == "bool":
        return "true" if e[1] else "false"
    if k == "paren":
        inner = emit(e[1], ctx, want, fma_ok=True)
        return f"({inner})"
    if k == "neg":
        return f"-{wrap(emit(e[1], ctx, want), e[1])}"
    if k in ("+", "-"):
        t = typeof(e, ctx)
        if t == "f64":
            fused = try_fma(e, ctx)
            if fused:
                return fused
        return f"{emit(e[1], ctx, t)} {k} {wrap_r(emit(e[2], ctx, t), e[2], k)}"
    if k in ("*", "/"):
        t = typeof(e, ctx)
        return f"{wrap_m(emit(e[1], ctx, t), e[1])} {k} {wrap_r(emit(e[2], ctx, t), e[2], k)}"
    if k == "**":
        base, ex = e[1], e[2]
        if ex[0] == "num" and is_int_lit(ex[1]):
            n = int(ex[1])
            if n == 2:
                b = wrap_recv(emit(base, ctx), base)
                return f"{b} * {b}"
            return f"{wrap_recv(emit(base, ctx), base)}.powi({n})"
        return f"{wrap_recv(emit(base, ctx), base)}.lpow({emit(ex, ctx)})"
    if k == "cmp":
        ta, tb = typeof(e[2], ctx), typeof(e[3], ctx)
        t = "f64" if "f64" in (ta, tb) else "i32"
        op = {"/=": "!="}.get(e[1], e[1])
        return f"{emit(e[2], ctx, t)} {op} {emit(e[3], ctx, t)}"
    if k in ("&&", "||"):
        def side(x):
            r = emit(x, ctx)
            return f"({r})" if k == "&&" and x[0] == "||" else r
        return f"{side(e[1])} {k} {side(e[2])}"
    if k == "!":
        inner = emit(e[1], ctx)
        return f"!{inner}" if e[1][0] in ("name", "call", "paren", "bool") else f"!({inner})"
    if k == "name":
        n = e[1]
        if n in ctx.loops:
            return f"({n} as i32 + 1)"
        if n in ctx.locals or n == "ivt":
            return n
        if n in SWITCH:
            return SWITCH[n]
        if n in DIMS:
            return f"({DIMS[n]} as i32)"
        if n in INTCONST:
            return INTCONST[n]
        if n in FLOATCONST:
            return FLOATCONST[n]
        if n in PHYS_SCALAR:
            return PHYS_SCALAR[n]
        if n in SCALARS:
            return f"s.constants.{SCALARS[n][0]}"
        if n in FIELDS:
            return f"s.{FIELDS[n][1]}.{FIELDS[n][0]}"
        return f"/*?{n}*/{n}"
    if k == "call":
        n, args = e[1], e[2]
        if n in FIELDS:
            ref = field_ref(n, args, ctx)
            return ref
        pr = phys_ref(n, args, ctx)
        if pr:
            return pr
        if n in PFTC:
            ix = "class" if args[0] == ("name", "ivt") else f"{emit(args[0], ctx, 'i32')} as usize"
            ref = f"c.{n}[{ix}]"
            return f"({ref} != 0.0)" if n in PFTC_LOGICAL else ref
        if n in ("max", "min", "amax1", "amin1"):
            fn = "max" if n in ("max", "amax1") else "min"
            t = typeof(e, ctx)
            out = wrap_recv(emit(args[0], ctx, t), args[0])
            for a in args[1:]:
                out = f"{out}.{fn}({emit(a, ctx, t)})"
            return out
        if n in ("exp", "log", "sqrt", "abs", "sin", "cos", "log10"):
            fn = {"log": "ln"}.get(n, n)
            return f"{wrap_recv(emit(args[0], ctx), args[0])}.{fn}()"
        if n in ("real", "dble"):
            return f"f64::from({emit(args[0], ctx, 'i32')})"
        if n in ("int",):
            return f"({emit(args[0], ctx)}) as i32"
        if n == "isleapyear":
            return f"is_leap_year({emit(args[0], ctx, 'i32')})"
        return f"/*?call {n}*/{n}({', '.join(emit(a, ctx) for a in args)})"
    raise SystemExit(f"cannot emit {e}")


def wrap(s, e):
    return s if e[0] in ("num", "name", "call", "paren") else f"({s})"


def wrap_m(s, e):
    return s if e[0] in ("num", "name", "call", "paren", "*", "/") and not s.startswith("-") else f"({s})"


def wrap_recv(s, e):
    """方法调用的接收者：只有原子（数、名字、调用、括号）可以不加括号。"""
    return s if e[0] in ("num", "name", "call", "paren") and not s.startswith("-") else f"({s})"


def wrap_r(s, e, op):
    if e[0] in ("+", "-") or (op in ("/", "-") and e[0] in ("*", "/")) or s.startswith("-"):
        return f"({s})"
    return s


def arg(e, ctx, t="f64"):
    """函数/方法实参：外层括号是多余的。"""
    return emit(e[1] if e[0] == "paren" else e, ctx, t)


def is_mult(e):
    return e[0] == "*"


def try_fma(e, ctx):
    """GCC convert_mult_to_fma on a +/- node."""
    if NOFUSE[0]:
        return None
    r = try_fma_inner(e, ctx)
    if r is not None:
        FUSED[0] += 1
    return r


def try_fma_inner(e, ctx):
    op, a, b = e
    t = "f64"
    # neg(product) used in +/-  ->  FNMA
    def prod(x):
        return x if is_mult(x) else None

    def negprod(x):
        return x[1] if x[0] == "neg" and is_mult(x[1]) else None

    # left operand is evaluated (defined) first
    if prod(a):
        x, y = a[1], a[2]
        addend = emit(b, ctx, t)
        if op == "+":
            return f"{wrap_recv(emit(x, ctx, t), x)}.mul_add({arg(y, ctx, t)}, {addend})"
        return f"{wrap_recv(emit(x, ctx, t), x)}.mul_add({arg(y, ctx, t)}, -{wrap(addend, b)})"
    if negprod(a):
        pa = negprod(a)
        x, y = pa[1], pa[2]
        addend = emit(b, ctx, t)
        if op == "+":
            return f"(-{wrap_m(emit(x, ctx, t), x)}).mul_add({arg(y, ctx, t)}, {addend})"
        return f"(-{wrap_m(emit(x, ctx, t), x)}).mul_add({arg(y, ctx, t)}, -{wrap(addend, b)})"
    if prod(b):
        x, y = b[1], b[2]
        addend = emit(a, ctx, t)
        if op == "+":
            return f"{wrap_recv(emit(x, ctx, t), x)}.mul_add({arg(y, ctx, t)}, {addend})"
        return f"(-{wrap_m(emit(x, ctx, t), x)}).mul_add({arg(y, ctx, t)}, {addend})"
    return None


# ---------------------------------------------------------------- statements
def join_continuations(lines, first):
    out, buf, start = [], "", None
    for n, l in enumerate(lines):
        code = strip_comment(l).rstrip()
        if buf:
            code = code.lstrip()
            if code.startswith("&"):
                code = code[1:]
        else:
            start = first + n
        if code.endswith("&"):
            buf += code[:-1] + " "
            continue
        out.append((start, first + n, buf + code))
        buf = ""
    return out


GIMPLE_FMA = None


def gimple_fma(path, func):
    counts = {}
    on = False
    for l in open(path):
        if l.startswith(";; Function "):
            on = l.split()[2].lower() == func
            continue
        if not on:
            continue
        m = re.match(r"\s*\[[^\]]*\.F90:(\d+):\d+(?: discrim \d+)?\].*=\s*(?:\[[^\]]*\]\s*)*\.(FMA|FNMA|FMS|FNMS)\b", l)
        if m:
            counts[int(m.group(1))] = counts.get(int(m.group(1)), 0) + 1
    return counts


FUSED = [0]
LINES = [(0, 0)]
NOFUSE = [False]


def strip_comment(l):
    out, q = [], None
    for ch in l:
        if q:
            out.append(ch)
            if ch == q:
                q = None
            continue
        if ch in "'\"":
            q = ch
        if ch == "!":
            break
        out.append(ch)
    return "".join(out)


def main():
    path, sub = sys.argv[1], sys.argv[2].lower()
    lines = Path(path).read_text(errors="replace").splitlines()
    start = next(i for i, l in enumerate(lines) if re.match(rf"\s*SUBROUTINE\s+{sub}\b", l, re.I))
    end = next(i for i in range(start, len(lines)) if re.match(rf"\s*END\s+SUBROUTINE", lines[i], re.I))
    body = join_continuations(lines[start + 1:end], start + 2)
    global GIMPLE_FMA
    if "--gimple" in sys.argv:
        GIMPLE_FMA = gimple_fma(sys.argv[sys.argv.index("--gimple") + 1], sub)
    locals_ = {}
    out = []
    indent = 1
    loops = []
    ifdef_stack = []

    def w(s):
        out.append("    " * indent + s)

    for first_line, last_line, raw in body:
        LINES[0] = (first_line, last_line)
        s = raw.strip()
        if not s:
            continue
        low = s.lower()
        m = re.match(r"(real\(r8\)|integer|logical)\s*(?:,\s*[^:]*)?::\s*(.*)", s, re.I) or \
            re.match(r"(real\(r8\)|integer|logical)\s+([A-Za-z].*)", s, re.I)
        if m:
            kind = {"real(r8)": "f64", "integer": "i32", "logical": "bool"}[m.group(1).lower()]
            if "intent" in s.lower():
                continue
            for part in re.split(r",(?![^(]*\))", m.group(2)):
                part = part.strip()
                nm = re.match(r"(\w+)(\s*\([^)]*\))?\s*(=\s*(.*))?", part)
                name = nm.group(1).lower()
                locals_[name] = kind if not nm.group(2) else kind + "[]"
                if name in ("m", "j", "k", "l", "ivt", "i", "ps", "pe", "fc", "fp", "p"):
                    continue
                if nm.group(4):
                    w(f"let mut {name} = {emit(parse_expr(nm.group(4)), Ctx(locals_, set(loops)), kind)};")
                elif not nm.group(2):
                    w(f"let mut {name}: {kind};")
                else:
                    w(f"let mut {name} = vec![0.0; d.nl_soil_full]; // {part}")
            continue
        if low.startswith(("use ", "implicit")):
            continue
        if low.startswith("#ifdef"):
            ifdef_stack.append(s.split()[1])
            w(f"if sw.{s.split()[1].lower()} {{")
            indent += 1
            continue
        if low.startswith("#ifndef"):
            ifdef_stack.append(s.split()[1])
            w(f"if !sw.{s.split()[1].lower()} {{")
            indent += 1
            continue
        if low.startswith("#else"):
            indent -= 1
            w("} else {")
            indent += 1
            continue
        if low.startswith("#endif"):
            ifdef_stack.pop()
            indent -= 1
            w("}")
            continue
        ctx = Ctx(locals_, set(loops))
        m = re.match(r"do\s+(\w+)\s*=\s*([^,]+),\s*([^,]+)(?:,\s*(-?\d+))?$", low)
        if m:
            var, lo, hi, step = m.group(1), m.group(2).strip(), m.group(3).strip(), m.group(4)
            if var == "m" and lo == "ps":
                w("for m in 0..npft {")
            elif step is None and re.fullmatch(r"\d+", lo) and hi in DIMS:
                w(f"for {var} in {int(lo) - 1}..{DIMS[hi]} {{")
            elif step == "-1" and lo in DIMS and re.fullmatch(r"\d+", hi):
                w(f"for {var} in ({int(hi) - 1}..{DIMS[lo]}).rev() {{")
            else:
                lo_e = emit(parse_expr(lo), ctx, "i32")
                hi_e = emit(parse_expr(hi), ctx, "i32")
                if step == "-1":
                    w(f"for {var} in (({hi_e} - 1) as usize..{lo_e} as usize).rev() {{")
                else:
                    w(f"for {var} in ({lo_e} - 1) as usize..{hi_e} as usize {{")
            loops.append(var)
            indent += 1
            continue
        if low in ("enddo", "end do"):
            loops.pop()
            indent -= 1
            w("}")
            continue
        m = re.match(r"if\s*\((.*)\)\s*then$", s, re.I)
        if m:
            w(f"if {emit(parse_expr(m.group(1)), ctx)} {{")
            indent += 1
            continue
        m = re.match(r"else\s*if\s*\((.*)\)\s*then$", s, re.I)
        if m:
            indent -= 1
            w(f"}} else if {emit(parse_expr(m.group(1)), ctx)} {{")
            indent += 1
            continue
        if low == "else":
            indent -= 1
            w("} else {")
            indent += 1
            continue
        if low in ("endif", "end if"):
            indent -= 1
            w("}")
            continue
        m = re.match(r"if\s*\((.*)\)\s*(\w.*=.*)$", s, re.I)
        if m and not low.endswith("then"):
            # single-line IF: find the matching paren
            depth, cut = 0, None
            for n, ch in enumerate(s):
                if ch == "(":
                    depth += 1
                elif ch == ")":
                    depth -= 1
                    if depth == 0:
                        cut = n
                        break
            cond, stmt = s[s.index("(") + 1:cut], s[cut + 1:].strip()
            w(f"if {emit(parse_expr(cond), ctx)} {{")
            indent += 1
            assign(stmt, ctx, w)
            indent -= 1
            w("}")
            continue
        if low.startswith(("write", "print", "call abort", "call coLM_stop".lower())):
            w(f"// {s}")
            continue
        if low.startswith("call "):
            w(f"/*?*/ // {s}")
            continue
        if "=" in s:
            assign(s, ctx, w)
            continue
        w(f"/*?stmt*/ // {s}")
    print("\n".join(out))


def assign(s, ctx, w):
    if re.match(r"ivt\s*=\s*pftclass\s*\(\s*m\s*\)$", s, re.I):
        w("let ivt = p.pftclass[m];")
        w("let class = ivt as usize;")
        return
    depth = 0
    for n, ch in enumerate(s):
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
        elif ch == "=" and depth == 0 and s[n + 1:n + 2] != "=" and s[n - 1:n] not in "<>/=":
            lhs, rhs = s[:n].strip(), s[n + 1:].strip()
            break
    else:
        w(f"/*?assign*/ // {s}")
        return
    le = parse_expr(lhs)
    t = typeof(le, ctx)
    if le[0] == "call" and any(a[0] == "colon" for a in le[2]):
        w(f"/*?slice*/ // {s}")
        return
    lhs_r = emit(le, ctx, t)
    rhs_e = parse_expr(rhs)
    FUSED[0] = 0
    NOFUSE[0] = False
    rhs_r = emit(rhs_e, ctx, t)
    note = ""
    if GIMPLE_FMA is not None:
        lo, hi = LINES[0]
        want = sum(GIMPLE_FMA.get(n, 0) for n in range(lo, hi + 1))
        if want == 0 and FUSED[0] > 0:
            NOFUSE[0] = True
            rhs_r = emit(rhs_e, ctx, t)
            NOFUSE[0] = False
            note = f" // 无 FMA（上游第 {lo} 行，乘积被 CSE 共享）"
        elif want > 0 and FUSED[0] == 0:
            note = f" /*FMA? gimple={want} rust={FUSED[0]} L{lo}*/"
    if t == "bool" and typeof(rhs_e, ctx) == "f64":
        rhs_r = f"{rhs_r} != 0.0"
    if (rhs_e[0] in ("+", "-", "*", "/") and rhs_e[1] == le and " = " not in rhs_r
            and not rhs_r.lstrip("(").startswith("-") and ".mul_add(" not in rhs_r.split(" ", 1)[0]):
        top = emit(rhs_e[1], ctx, t) + f" {rhs_e[0]} "
        if rhs_r.startswith(top):
            rest = rhs_r[len(top):]
            if rest.startswith("(") and rest.endswith(")") and rhs_e[2][0] != "paren":
                rest = rest[1:-1]
            w(f"{lhs_r} {rhs_e[0]}= {rest};{note}")
            return
    w(f"{lhs_r} = {rhs_r};{note}")


if __name__ == "__main__":
    main()

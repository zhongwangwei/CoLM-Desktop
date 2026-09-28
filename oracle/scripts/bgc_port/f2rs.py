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

PHYS_PFT = "pftfrac tsai_p tlai_p lai_p laisun_p laisha_p sigf_p tref_p assim_p respc_p irrig_method_p".split()
PHYS_SOIL = "porsl psi0 bsw theta_r alpha_vgm n_vgm l_vgm sc_vgm fc_vgm bd_all wfc om_density t_soisno wliq_soisno wice_soisno smp h2osoi".split()
PHYS_PATCH = ("patchlatr lai tlai tref rsur rnof forc_t forc_q forc_psrf forc_prc forc_prl forc_us forc_vs "
              "lai_enftemp lai_enfboreal lai_dnfboreal lai_ebftrop lai_ebftemp lai_dbftrop lai_dbftemp lai_dbfboreal lai_ebstemp lai_dbstemp lai_dbsboreal lai_c3arcgrass lai_c3grass lai_c4grass irrig_method_corn irrig_method_swheat irrig_method_wwheat irrig_method_soybean irrig_method_cotton irrig_method_rice1 irrig_method_rice2 irrig_method_sugarcane").split()
PHYS_GRID = "z_soi dz_soi zi_soi".split()
PHYS_SCALAR = {"deltim": "p.deltim", "dlat": "p.dlat", "dlon": "p.dlon", "smpmax_hr": "p.smpmax_hr", "smpmin_hr": "p.smpmin_hr"}
PHYS_CASE = {"l_vgm": "L_vgm", "bd_all": "BD_all", "om_density": "OM_density"}

PFTC = set("""woody isevg issed isstd isbare iscrop isnatveg isshrub isgrass isbetr isbdtr dsladlai declfact allconsl
cc_dstem cc_leaf cc_lstem cc_other croot_stem deadwdcn fcur2 fd_pft flivewd fm_droot fm_leaf fm_lroot fm_lstem fm_other fm_root
fr_fcel fr_flab fr_flig froot_leaf frootcn fsr_pft graincn grperc grpnow laimx leaf_long leafcn lf_fcel lf_flab lf_flig lflitcn
livewdcn slatop stem_leaf lfemerg grnfill mxmat baset allconss arootf arooti astemf bfact ffrootcn fleafcn fleafi
fstemcn""".split())
PFTC_LOGICAL = set("isevg issed isstd isbare iscrop isnatveg isshrub isgrass isbetr isbdtr".split())

DIMS = {"nl_soil": "d.nl_soil", "nl_soil_full": "d.nl_soil_full", "ndecomp_pools": "d.ndecomp_pools",
        "ndecomp_transitions": "d.ndecomp_transitions"}
INTCONST = {"npcropmin": "NPCROPMIN"}
for _m in re.finditer(r"integer\s*,\s*parameter\s*::\s*(\w+)\s*=\s*(\d+)",
                      (ROOT / "vendor/CoLM202X/main/MOD_Vars_Global.F90").read_text(), re.I):
    INTCONST.setdefault(_m.group(1).lower(), _m.group(2))
FLOATCONST = {"spval": "MISSING", "tfrz": "273.16", "zmin_bedrock": "0.4", "denh2o": "1000.0", "denice": "917.0"}
SWITCH = {"def_use_nitrif": "sw.nitrif", "def_use_sasu": "sw.sasu", "def_use_diagmatrix": "sw.diag_matrix",
          "def_use_fire": "sw.fire", "def_use_cnsoyfixn": "sw.cnsoyfixn", "def_use_fert": "sw.fert",
          "def_use_irrigation": "sw.irrigation", "def_use_laifeedback": "sw.laifeedback",
          "def_use_nostressnitrogen": "sw.nostressnitrogen", "def_use_tracer": "false /*TRACER*/"}


# ---------------------------------------------------------------- lexer/parser
TOK = re.compile(r"\s*(?:(\d+\.\d*(?:[eEdD][+-]?\d+)?(?:_r8)?|\.\d+(?:[eEdD][+-]?\d+)?(?:_r8)?|\d+[eEdD][+-]?\d+(?:_r8)?|\d+(?:_r8)?)"
                 r"|(\.and\.|\.or\.|\.not\.|\.eq\.|\.ne\.|\.lt\.|\.le\.|\.gt\.|\.ge\.|\.true\.|\.false\.)"
                 r"|(\*\*|==|/=|<=|>=|[-+*/(),<>:=])|([A-Za-z_]\w*))", re.I)


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
                        if self.peek()[0] == "name" and self.i + 1 < len(self.t) and self.t[self.i + 1][1] == "=":
                            key = self.take()[1].lower()
                            self.take("=")
                            args.append(("kw", key, self.expr()))
                        elif self.peek()[1] == ":":
                            self.take()
                            args.append(("colon",))
                        else:
                            lo = self.expr()
                            if self.peek()[1] == ":":
                                self.take()
                                args.append(("range", lo, self.expr()))
                            else:
                                args.append(lo)
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
        if n in ("ivt", "ps", "pe", "patchclass"):
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
        if n in ctx.locals and ctx.locals[n].endswith("[]"):
            return ctx.locals[n][:-2]
        if n in PFTC_LOGICAL:
            return "bool"
        if n in ("int", "nint", "floor"):
            return "i32"
        if n in ("real", "dble", "exp", "log", "sqrt", "abs", "amin1", "amax1", "sin", "cos", "log10"):
            return "f64" if n != "abs" else typeof(e[2][0], ctx)
        if n in ("max", "min"):
            ts = [typeof(a, ctx) for a in e[2]]
            return "f64" if "f64" in ts else "i32"
        if n in ("pftclass", "patchclass", "idate"):
            return "i32"
        if n in ("isleapyear", "isendofyear", "any", "all"):
            return "bool"
        return "f64"
    return "f64"


def index(arg, ctx):
    """0-based usize index expression for a Fortran subscript."""
    if arg[0] == "name" and arg[1] in ctx.loops:
        return arg[1]
    if arg[0] == "num":
        return str(int(arg[1]) - 1)
    if arg == ("name", "ps"):
        return "0"
    if arg == ("name", "pe"):
        return "npft - 1"
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
    if name == "zi_soi" and ZI_FROM_ZERO[0]:
        k = index(args[0], ctx)
        k = k[:-4] if k.endswith(" - 1") else f"{k} + 1"
        return f"p.zi_soi_from_zero({k})"
    if name in PHYS_GRID:
        return f"p.{rust}[{index(args[0], ctx)}]"
    if name == "rootfr_p":
        return f"p.rootfr_p[{index(args[0], ctx)} + d.nl_soil * m]"
    if name == "pftclass":
        return f"p.pftclass[{index(args[0], ctx)}]"
    if name == "patchclass":
        return "p.patchclass"
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
        return f"{wrap_recv(emit(base, ctx), base)}.lpow({strip_parens(emit(ex, ctx))})"
    if k == "cmp":
        ta, tb = typeof(e[2], ctx), typeof(e[3], ctx)
        t = "f64" if "f64" in (ta, tb) else "i32"
        op = {"/=": "!="}.get(e[1], e[1])
        # 整型与实型比较时整型一侧提升为实型（Fortran 的混合运算规则）。
        def side(x, tx):
            text = emit(x, ctx, t)
            if t == "f64" and tx == "i32" and x[0] != "num":
                return f"f64::from({strip_parens(text)})"
            return text
        return f"{side(e[2], ta)} {op} {side(e[3], tb)}"
    if k in ("&&", "||"):
        def side(x):
            r = emit(x, ctx)
            return f"({r})" if k == "&&" and x[0] == "||" else r
        return f"{side(e[1])} {k} {side(e[2])}"
    if k == "!":
        inner = emit(e[1], ctx)
        return f"!{inner}" if e[1][0] in ("name", "call", "paren", "bool") else f"!({inner})"
    if k == "name" and e[1] in FORWARD_EXPR:
        return emit(FORWARD_EXPR[e[1]], ctx, want, fma_ok)
    if k == "name":
        n = e[1]
        if n in ctx.loops:
            return f"({n} as i32 + 1)"
        if n in ctx.locals or n == "ivt":
            return n
        if n == "ps":
            return "0"
        if n == "pe":
            return "(npft as i32 - 1)"
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
        if n in LOCAL_ARRAYS and len(args) == 1:
            k = index(args[0], ctx)
            lower = LOCAL_ARRAYS[n]
            if lower != 1:
                shift = 1 - lower
                k = k[:-4] if (shift == 1 and k.endswith(" - 1")) else f"{k} + {shift}"
            return f"{n}[{k}]"
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
                out = f"{out}.{fn}({strip_parens(emit(a, ctx, t))})"
            return out
        if n in ("exp", "log", "sqrt", "abs", "sin", "cos", "log10"):
            fn = {"log": "ln"}.get(n, n)
            return f"{wrap_recv(emit(args[0], ctx), args[0])}.{fn}()"
        if n in ("real", "dble"):
            return f"f64::from({emit(args[0], ctx, 'i32')})"
        if n in ("int",):
            return f"({emit(args[0], ctx)}) as i32"
        if n == "nint":
            # Fortran `nint` 四舍五入、半数远离零，与 `f64::round` 相同。
            return f"({emit(args[0], ctx)}).round() as i32"
        if n == "daylength":
            return f"crate::bgc_phenology::daylength({emit(args[0], ctx)}, {emit(args[1], ctx, 'i32')})"
        if n == "sum" and var_line_vectorized() and args[0][0] == "*" and len(args) == 1:
            var, bound = section_var(args[0])
            ctx2 = Ctx(ctx.locals, ctx.loops | {var})
            inner = replace_ranges(args[0], var)
            a, b = (arg(x, ctx2) for x in inner[1:])
            n_expr = "npft" if var == "m" else bound.split("..")[1]
            return f"vectorized_dot({n_expr}, |{var}| ({a}, {b}))"
        if n == "sum":
            var, bound = section_var(args[0])
            inner = replace_ranges(args[0], var)
            ctx2 = Ctx(dict(ctx.locals, acc="f64"), ctx.loops | {var})
            body = emit(("+", ("name", "acc"), inner), ctx2)
            mask = [a for a in args[1:] if a[0] == "kw" and a[1] == "mask"]
            if mask:
                cond = emit(replace_ranges(mask[0][2], var), ctx2)
                body = f"if {cond} {{ {body} }} else {{ acc }}"
            return f"({bound}).fold(0.0, |acc, {var}| {body})"
        if n in ("any", "all") and len(args) == 1:
            var, bound = section_var(args[0])
            ctx2 = Ctx(ctx.locals, ctx.loops | {var})
            return f"({bound}).{n}(|{var}| {emit(replace_ranges(args[0], var), ctx2)})"
        if n == "isendofyear":
            return "is_end_of_year(p.idate, p.deltim)"
        if n == "isleapyear":
            return f"is_leap_year({emit(args[0], ctx, 'i32')})"
        return f"/*?call {n}*/{n}({', '.join(emit(a, ctx) for a in args)})"
    raise SystemExit(f"cannot emit {e}")


def var_line_vectorized():
    lo, hi = LINES[0]
    return any(n in VECT for n in range(lo, hi + 1))


def section_var(e):
    """第一个数组段决定求和变量：`(ps:pe)` → m，`(1:nl_soil)` → j。"""
    found = []

    def walk(x):
        if isinstance(x, tuple):
            if x and x[0] == "range":
                found.append(x)
            for y in x[1:]:
                if isinstance(y, (tuple, list)):
                    walk(y) if isinstance(y, tuple) else [walk(z) for z in y]
    walk(e)
    r = found[0]
    if r[1] == ("name", "ps"):
        return "m", "0..npft"
    if r[1] == ("num", "1") and r[2][0] == "name" and r[2][1] in DIMS:
        return "j", f"0..{DIMS[r[2][1]]}"
    raise SystemExit(f"unsupported section {r}")


def replace_ranges(e, var):
    if not isinstance(e, tuple):
        return e
    if e and e[0] == "range":
        return ("name", var)
    if e and e[0] == "call":
        return ("call", e[1], [replace_ranges(a, var) for a in e[2]])
    return tuple(replace_ranges(x, var) if isinstance(x, tuple) else x for x in e)


def wrap(s, e):
    return s if e[0] in ("num", "name", "call", "paren") else f"({s})"


def wrap_m(s, e):
    return s if e[0] in ("num", "name", "call", "paren", "*", "/") and not s.startswith("-") else f"({s})"


def wrap_recv(s, e):
    """方法调用的接收者：只有原子（数、名字、调用、括号）可以不加括号；浮点字面量要标类型。"""
    if e[0] == "num" and re.fullmatch(r"[\d.eE+-]+", s) and not s.startswith("-"):
        return s + "_f64" if ("." in s or "e" in s.lower()) else s
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


def neg_recv(x, ctx, t):
    """取负后作 `mul_add` 的接收者：负的数字字面量要带 `_f64`，否则是不定浮点类型。"""
    text = emit(x, ctx, t)
    if x[0] == "num":
        return f"(-{text}_f64)" if "_f64" not in text else f"(-{text})"
    return f"(-{wrap_m(text, x)})"


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
    a, b = resolve(a), resolve(b)
    t = "f64"
    # neg(product) used in +/-  ->  FNMA
    # `y**2` 在 gimplify 时就展开成 `y*y`，和普通乘积一样可被收缩
    # （CNVegStructUpdate 的 `(natlaimx+slatop*leafc)**2 - 4*theta*…` 是 `.FMS (_75, _75, _26)`）。
    def prod(x):
        if x[0] == "**" and x[2][0] == "num" and is_int_lit(x[2][1]) and int(x[2][1]) == 2:
            return ("*", x[1], x[1])
        return x if is_mult(x) else None

    def negprod(x):
        return x[1] if x[0] == "neg" and is_mult(x[1]) else None

    # left operand is evaluated (defined) first
    if prod(a):
        x, y = prod(a)[1], prod(a)[2]
        addend = emit(b, ctx, t)
        if op == "+":
            return f"{wrap_recv(emit(x, ctx, t), x)}.mul_add({arg(y, ctx, t)}, {addend})"
        return f"{wrap_recv(emit(x, ctx, t), x)}.mul_add({arg(y, ctx, t)}, -{wrap(addend, b)})"
    if negprod(a):
        pa = negprod(a)
        x, y = pa[1], pa[2]
        addend = emit(b, ctx, t)
        if op == "+":
            return f"{neg_recv(x, ctx, t)}.mul_add({arg(y, ctx, t)}, {addend})"
        return f"{neg_recv(x, ctx, t)}.mul_add({arg(y, ctx, t)}, -{wrap(addend, b)})"
    if prod(b):
        x, y = prod(b)[1], prod(b)[2]
        addend = emit(a, ctx, t)
        if op == "+":
            return f"{wrap_recv(emit(x, ctx, t), x)}.mul_add({arg(y, ctx, t)}, {addend})"
        return f"{neg_recv(x, ctx, t)}.mul_add({arg(y, ctx, t)}, {addend})"
    return None


# ---------------------------------------------------------------- statements
def join_continuations(lines, first):
    """拼续行。续行中间的 `#ifdef X ... [#else ...] #endif` 拆成语句级的两个版本。"""
    out, start = [], None
    bufs = None          # {True: 宏开时的文本, False: 宏关时的文本}
    macro, state = None, None   # state: None / True（#ifdef 段）/ False（#else 段）
    for n, l in enumerate(lines):
        stripped = l.strip()
        if bufs is not None and stripped.lower().startswith(("#ifdef", "#ifndef", "#else", "#endif")):
            low = stripped.lower()
            if low.startswith(("#ifdef", "#ifndef")):
                macro, state = stripped.split()[1], low.startswith("#ifdef")
            elif low.startswith("#else"):
                state = not state
            else:
                state = None
            continue
        code = strip_comment(l).rstrip()
        if bufs is not None:
            code = code.lstrip()
            if code.startswith("&"):
                code = code[1:]
        else:
            start = first + n
            bufs = {True: "", False: ""}
        cont = code.endswith("&")
        piece = (code[:-1] + " ") if cont else code
        for key in (True, False):
            if state is None or state == key:
                bufs[key] += piece
        if cont:
            continue
        if bufs[True] == bufs[False]:
            out.append((start, first + n, bufs[True]))
        else:
            out.append((start, start, f"#ifdef {macro}"))
            out.append((start, first + n, bufs[True]))
            out.append((start, start, "#else"))
            out.append((start, first + n, bufs[False]))
            out.append((start, start, "#endif"))
        bufs, macro, state = None, None, None
    return out


GIMPLE_FMA = None
VECT = set()   # 被向量化成保序归约的行


def gimple_fma(path):
    """按源码行统计整个 dump 里的 FMA 族运算（被内联的子程序也算在自己的行号上）。

    返回 (FMA 计数, 出现过浮点运算的行)。只有一条存储、值是别处算好后被 CSE 复用的行，
    按被存储 SSA 值的定义语句判断（同一函数内）：定义是 FMA 族就算融合，否则算"出现但不融合"。
    """
    counts, seen = {}, set()
    defs, stores = {}, []
    loc = re.compile(r"\s*\[[^\]]*\.F90:(\d+):\d+(?: discrim \d+)?\]\s*(.*)$")
    func = None
    for l in open(path):
        if l.startswith(";; Function "):
            func = l.split()[2]
            continue
        m = loc.match(l)
        stmt = m.group(2) if m else l.strip()
        stmt = re.sub(r"\[[^\]]*\.F90:\d+:\d+(?: discrim \d+)?\]\s*", "", stmt)
        d = re.match(r"([\w.]+)\s*=\s*(.*);$", stmt)
        if d:
            defs[(func, d.group(1))] = d.group(2)
        if not m:
            continue
        line = int(m.group(1))
        if re.search(r"\bvect_", stmt):
            VECT.add(line)
        if re.search(r"\.(FMA|FNMA|FMS|FNMS)\b", stmt):
            counts[line] = counts.get(line, 0) + 1
            seen.add(line)
        elif re.search(r"=\s*\S+\s[-+*]\s\S+;", stmt):
            seen.add(line)
        else:
            s = re.match(r"MEM.*\]\s*=\s*([\w.]+);$", stmt)
            if s:
                stores.append((line, func, s.group(1)))
    for line, fn, name in stores:
        if line in seen:
            continue
        rhs = defs.get((fn, name), "")
        for _ in range(8):
            # 沿 `((x))`（PAREN_EXPR）与纯复制追到真正的运算
            m = re.match(r"^\(*([\w.]+)\)*$", rhs)
            if not m or (fn, m.group(1)) not in defs:
                break
            rhs = defs[(fn, m.group(1))]
        if re.search(r"\.(FMA|FNMA|FMS|FNMS)\b", rhs):
            counts[line] = counts.get(line, 0) + 1
            seen.add(line)
        elif re.search(r"^\S+\s[-+*]\s\S+$", rhs):
            seen.add(line)
    return counts, seen


FUSED = [0]
LOCAL_ARRAYS = {}   # 局部数组名 -> Fortran 下界
ZI_FROM_ZERO = [False]   # 形参 `zi_soi(0:…)`：见 BgcPhysics::zi_soi_from_zero
# driver 统一传的实参；其余的哑元成为 Rust 函数的额外参数
STANDARD = {"i", "ps", "pe", "nl_soil", "nl_soil_full", "dz_soi", "z_soi", "zi_soi", "ndecomp_pools",
            "ndecomp_transitions", "ndecomp_pools_vr", "deltim", "npcropmin", "idate", "dlat", "dlon",
            "nbedrock", "zmin_bedrock"}
SUBS = {}
RESULT = [False]


RUST_NAMES = {}


def strip_parens(text):
    """实参位置的外层括号是多余的（仅当最外一对括号互相匹配时去掉）。"""
    while text.startswith("(") and text.endswith(")"):
        depth = 0
        for k, ch in enumerate(text):
            depth += ch == "("
            depth -= ch == ")"
            if depth == 0 and k < len(text) - 1:
                return text
        text = text[1:-1]
    return text


ZERO_INIT = set()
# regen.py 登记的前向代入：GCC 把这些局部乘积代入使用点并与那里的加减收缩成 FMA 族，
# 赋值行本身不再有乘法。不发射赋值，使用处换成右侧表达式树。
FORWARD = set()
FORWARD_EXPR = {}


def resolve(x):
    return FORWARD_EXPR[x[1]][1] if x[0] == "name" and x[1] in FORWARD_EXPR else x


def snake(name):
    """内部子程序的 Rust 名由 `--names fortran=rust,...` 给出（regen.py 的表）。"""
    return RUST_NAMES.get(name.lower(), name.lower())


def subroutines(lines):
    out = {}
    for l in lines:
        m = re.match(r"\s*SUBROUTINE\s+(\w+)\s*\(([^)]*)\)", l, re.I)
        if m:
            out[m.group(1).lower()] = [a.strip().lower() for a in m.group(2).split(",") if a.strip()]
    return out
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
    SUBS.update(subroutines(lines))
    # 模块自己的整型参数（`NOT_Harvested = 999` 之类），仅在转写本模块时可见。
    for _m in re.finditer(r"integer\s*,\s*parameter\s*::\s*(\w+)\s*=\s*(\d+)", "\n".join(lines), re.I):
        INTCONST.setdefault(_m.group(1).lower(), _m.group(2))
    extras = [a for a in SUBS[sub] if a not in STANDARD]
    global GIMPLE_FMA
    if "--names" in sys.argv:
        for pair in sys.argv[sys.argv.index("--names") + 1].split(","):
            f, r = pair.split("=")
            RUST_NAMES[f.lower()] = r.lstrip("_")
    if "--zero-init" in sys.argv:
        ZERO_INIT.update(n.lower() for n in sys.argv[sys.argv.index("--zero-init") + 1].split(","))
    if "--forward" in sys.argv:
        FORWARD.update(n.lower() for n in sys.argv[sys.argv.index("--forward") + 1].split(","))
    if "--gimple" in sys.argv:
        GIMPLE_FMA = gimple_fma(sys.argv[sys.argv.index("--gimple") + 1])
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
            if "intent" in s.lower() and re.search(r"\bzi_soi\s*\(\s*0\s*:", s, re.I):
                ZI_FROM_ZERO[0] = True
            if "intent" in s.lower():
                for part in re.split(r",(?![^(]*\))", m.group(2)):
                    nm = re.match(r"(\w+)", part.strip())
                    if nm and nm.group(1).lower() not in STANDARD:
                        locals_.setdefault(nm.group(1).lower(), kind)
                continue
            for part in re.split(r",(?![^(]*\))", m.group(2)):
                part = part.strip()
                nm = re.match(r"(\w+)(\s*\([^)]*\))?\s*(=\s*(.*))?", part)
                name = nm.group(1).lower()
                locals_[name] = kind if not nm.group(2) else kind + "[]"
                if nm.group(2):
                    lower = re.match(r"\s*\(\s*(-?\d+)\s*:", nm.group(2))
                    LOCAL_ARRAYS[name] = int(lower.group(1)) if lower else 1
                if name in ("m", "j", "k", "l", "ivt", "i", "ps", "pe", "fc", "fp", "p", "c", "g", "s", "d", "sw"):
                    continue
                if name in FORWARD:
                    continue
                if nm.group(4):
                    w(f"let mut {name}: {kind} = {emit(parse_expr(nm.group(4)), Ctx(locals_, set(loops)), kind)};")
                elif not nm.group(2) and name in ZERO_INIT:
                    # regen.py 登记的：上游每条会读到它的路径都先赋值，但 Rust 的流分析看不出来。
                    w(f"let mut {name}: {kind} = {'0.0' if kind == 'f64' else '0'};")
                elif not nm.group(2):
                    w(f"let mut {name}: {kind};")
                else:
                    zero = {"f64": "0.0", "i32": "0", "bool": "false"}[kind]
                    # 上界都不超过 nl_soil_full+1，统一开到 nl_soil_full+2（下标按下界平移）。
                    w(f"let mut {name} = vec![{zero}; d.nl_soil_full + 2];")
            continue
        if low.startswith(("use ", "implicit")):
            continue
        if low.startswith(("#ifdef", "#ifndef")):
            macro = s.split()[1]
            ifdef_stack.append(macro)
            # CROP 是运行时开关；其余宏（USEMPI、FUN…）在单点 Rust 引擎里都视为未定义。
            cond = "sw.crop" if macro.upper() == "CROP" else f"false /*{macro}*/"
            if low.startswith("#ifndef"):
                cond = "!sw.crop" if macro.upper() == "CROP" else f"true /*!{macro}*/"
            w(f"if {cond} {{")
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
        if low.startswith(("write", "print")):
            w(f"// {s}")
            continue
        if re.match(r"call\s+(mpi_abort|colm_stop)\b", low):
            continue
        if re.match(r"call\s+abort\b", low):
            RESULT[0] = True
            w(f'anyhow::bail!("{sys.argv[2]}：上游在此 abort（收支/廓线检查失败）");')
            continue
        m = re.match(r"call\s+(\w+)\s*\((.*)\)\s*$", s, re.I)
        if m and m.group(1).lower() in SUBS:
            callee = m.group(1).lower()
            actual = [a.strip() for a in re.split(r",(?![^(]*\))", m.group(2))]
            dummies = SUBS[callee]
            extra = [strip_parens(emit(parse_expr(a), ctx, locals_.get(d, "f64")))
                     for d, a in zip(dummies, actual) if d not in STANDARD]
            args = ", ".join(["s", "p", "c", "sw"] + extra)
            w(f"{snake(m.group(1))}({args});")
            continue
        m = re.match(r"call\s+julian2monthday\s*\((.*)\)$", s, re.I)
        if m:
            y, dd, mo, da = [a.strip() for a in m.group(1).split(",")]
            w(f"({mo.lower()}, {da.lower()}) = crate::bgc_driver::julian_month_day("
              f"{emit(parse_expr(y), ctx, 'i32')}, {emit(parse_expr(dd), ctx, 'i32')});")
            continue
        if low.startswith("call "):
            w(f"/*?*/ // {s}")
            continue
        if "=" in s:
            assign(s, ctx, w)
            continue
        w(f"/*?stmt*/ // {s}")
    types = {}
    for d in extras:
        types[d] = locals_.get(d, "f64")
    meta = []
    if extras:
        meta.append("// EXTRA_ARGS: " + ", ".join(f"{d}: {types[d]}" for d in extras))
    if RESULT[0]:
        meta.append("// RESULT")
        out.append("    Ok(())")
    print("\n".join(meta + out))


def assign(s, ctx, w):
    if re.match(r"(ps|pe)\s*=\s*patch_pft_[se]\s*\(\s*i\s*\)$", s, re.I):
        return   # 单 patch：PFT 范围就是 0..npft
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
    if le[0] == "name" and le[1] in FORWARD:
        FORWARD_EXPR[le[1]] = ("paren", parse_expr(rhs))
        return
    t = typeof(le, ctx)
    if le[0] == "call" and any(a[0] == "colon" for a in le[2]):
        w(f"/*?slice*/ // {s}")
        return
    if le[0] == "call" and any(a[0] == "range" for a in le[2]):
        var, bound = section_var(le)
        w(f"for {var} in {bound} {{")
        ctx2 = Ctx(ctx.locals, ctx.loops | {var})
        rhs_e = replace_ranges(parse_expr(rhs), var)
        w(f"    {emit(replace_ranges(le, var), ctx2, t)} = {emit(rhs_e, ctx2, t)};")
        w("}")
        return
    lhs_r = emit(le, ctx, t)
    rhs_e = parse_expr(rhs)
    if rhs_e[0] == "paren":
        rhs_e = rhs_e[1]
    FUSED[0] = 0
    NOFUSE[0] = False
    rhs_r = emit(rhs_e, ctx, t)
    note = ""
    if GIMPLE_FMA is not None:
        lo, hi = LINES[0]
        counts, seen = GIMPLE_FMA
        want = sum(counts.get(n, 0) for n in range(lo, hi + 1))
        present = any(n in seen for n in range(lo, hi + 1))
        if want == 0 and FUSED[0] > 0 and present:
            NOFUSE[0] = True
            rhs_r = emit(rhs_e, ctx, t)
            NOFUSE[0] = False
            note = f" // 无 FMA（上游第 {lo} 行，乘积被 CSE 共享）"
        elif want > 0 and FUSED[0] == 0:
            note = f" /*FMA? gimple={want} rust={FUSED[0]} L{lo}*/"
    if t == "bool" and typeof(rhs_e, ctx) == "f64":
        rhs_r = f"{rhs_r} != 0.0"
    # 整型右值赋给实型左值：Fortran 隐式转换（`harvdate_p(m) = jday`）。
    if t == "f64" and typeof(rhs_e, ctx) == "i32":
        rhs_r = f"{float(rhs_e[1])!r}" if rhs_e[0] == "num" else f"f64::from({strip_parens(rhs_r)})"
    if (rhs_e[0] in ("+", "-", "*", "/") and rhs_e[1] == le and " = " not in rhs_r
            and not rhs_r.lstrip("(").startswith("-") and ".mul_add(" not in rhs_r.split(" ", 1)[0]):
        top = emit(rhs_e[1], ctx, t) + f" {rhs_e[0]} "
        if rhs_r.startswith(top):
            rest = rhs_r[len(top):]
            rest = strip_parens(rest)
            w(f"{lhs_r} {rhs_e[0]}= {rest};{note}")
            return
    w(f"{lhs_r} = {rhs_r};{note}")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Execute full vendor UrbanVegFlux with fixed, physical longwave interface inputs.

Four cases cover C3/C4, foliage wetness, irrigation, partial roofs and vegetation
snow. Active numerical dependencies are actual vendor procedures. These are flux
procedure references, not a city-site or full UrbanTHERMAL validation.
"""

from pathlib import Path
import re
import platform
import subprocess
import tempfile

root = Path(__file__).resolve().parents[2]
v = root / "vendor/CoLM202X"
source = (v / "main/URBAN/MOD_Urban_Flux.F90").read_text(encoding="utf-8")
sub = re.search(
    r"(?is)   SUBROUTINE\s+UrbanVegFlux\b.*?   END SUBROUTINE UrbanVegFlux", source
).group(0)
dew = re.search(
    r"(?is)   SUBROUTINE dewfraction\b.*?   END SUBROUTINE dewfraction", source
).group(0)
constants = "\n".join(
    line
    for line in source[: source.index("CONTAINS")].splitlines()
    if re.match(r"\s*(integer|logical|real\(r8\)),\s*parameter", line, re.I)
)
# These modules provide only switches/constants and an unreachable fatal diagnostic.
# Every active numerical procedure (including stomata and wetness) is actual vendor code.
stubs = """
module MOD_SPMD_Task
contains
 subroutine CoLM_stop()
 stop 1
 end subroutine
end module
module MOD_Namelist
 use MOD_Precision
 integer::DEF_RSS_SCHEME=1
 logical::DEF_VEG_SNOW=.false.,DEF_USE_CoLMDEBUG=.false.
 logical::DEF_USE_MEDLYNST=.false.,DEF_USE_WUEST=.false.,DEF_USE_LCT=.false.
 integer,parameter::LC_OVERRIDE_SLOTS=24
 real(r8),parameter::LC_OVERRIDE_UNSET=-1.e36_r8
 real(r8)::DEF_LC_RESPCP(24)=-1.e36_r8
 real(r8)::DEF_MEDLYN_G1=-1._r8,DEF_MEDLYN_G0=-1._r8,DEF_WUE_LAMBDA=-1._r8
 real(r8)::DEF_BALL_BERRY_GRADM=-1._r8,DEF_BALL_BERRY_BINTER=-1._r8
end module
module MOD_Vars_Global
 use MOD_Precision
 real(r8),parameter::PI=acos(-1._r8),spval=-1.e36_r8
end module
module MOD_UserSpecifiedForcing
 character(len=8)::HEIGHT_mode='relative'
end module
"""
inputs = "deltim hu ht hq us vs thm th thv qm psrf rhoair frl po2m pco2m par sabv rstfac Fhac Fwst Fach vehc meta hroof hlr ewall egimp egper ev htop hbot lai sai sqrtdi effcon vmax25 slti hlti shti hhti trda trdm trop g1 g0 gradm binter extkn extkd dewmx etrc trsmx0 lambda_wue z0h_g obug ustarg zlnd zsno fsno_roof fsno_gimp fsno_gper wliq_roofsno wliq_gimpsno wice_roofsno wice_gimpsno htvp_roof htvp_gimp htvp_gper troof twsun twsha tgimp tgper qroof qgimp qgper dqroofdT dqgimpdT dqgperdT sigf tl ldew ldew_rain ldew_snow fwet_snow dheatl rss etr_deficit".split()
values = [
    1800,
    30,
    2,
    2,
    3.2,
    0.7,
    295,
    294,
    295.2,
    0.007,
    101325,
    1.2,
    350,
    21200,
    40,
    200,
    120,
    0.8,
    30,
    9,
    4,
    6,
    3,
    12,
    0.6,
    1,
    1,
    1,
    1,
    6,
    2,
    2,
    0.5,
    5,
    0.05,
    0.000060,
    0.2,
    288.16,
    0.3,
    313.16,
    1.3,
    328.16,
    298.16,
    4,
    0.01,
    9,
    0.01,
    0.11,
    0.5,
    0.1,
    0.0002,
    0.0002,
    2000,
    0.01,
    -100,
    0.2,
    0.03,
    0.002,
    0,
    0,
    0,
    0.2,
    0.1,
    0,
    0,
    2510400,
    2510400,
    2510400,
    301,
    303,
    299,
    300,
    297,
    0.014,
    0.013,
    0.011,
    0.0007,
    0.0007,
    0.0006,
    1,
    299,
    0,
    0,
    0,
    0,
    0,
    100,
    0,
]
assert len(inputs) == len(values), (len(inputs), len(values))
for name, value in {
    "ewall": 0.95,
    "egimp": 0.96,
    "egper": 0.97,
    "ev": 0.98,
    "effcon": 0.08,
}.items():
    values[inputs.index(name)] = value
common = "taux tauy fsenroof fsenwsun fsenwsha fsengimp fsengper fevproof fevpgimp fevpgper croofs cwsuns cwshas cgrnds croofl cgimpl cgperl croof cgimp cgper tref qref z0m zol rib ustar qstar tstar fm fh fq tafu".split()
extra = "fsenl fevpl etr rst assim respc lwsun lwsha lgimp lgper lveg lout etr_deficit dheatl tl ldew ldew_rain ldew_snow fwet_snow".split()
extra.remove(
    "fwet_snow"
)  # Vendor leaves this output undefined when DEF_VEG_SNOW is false.
outputs = common + extra
locals_ = [n for n in common + extra if n not in inputs]
program = (
    "program reference\nuse MOD_Urban_Flux\nimplicit none\ninteger::i,j\nreal(r8)::"
    + ",".join(inputs + locals_)
    + ",fcover(0:5),Ainv(5,5),B(5),B1(5),dBdT(5),SkyVF(5),VegVF(5),froofs(4)\n"
)
program += "froofs=(/.12_r8,.35_r8,.6_r8,.35_r8/)\ndo i=1,4\n"
program += (
    "\n".join(f"{n}=" + f"{x:.17e}".replace("e", "d") for n, x in zip(inputs, values))
    + "\n"
)
program += "\n".join(f"{name}=0._r8" for name in locals_) + "\n"
program += """
DEF_VEG_SNOW=(i==3)
fcover=(/froofs(i),.1_r8,.08_r8,(1-froofs(i))*.6_r8,(1-froofs(i))*.4_r8,.2_r8/)
if (i==2) then
 lai=3._r8;htop=8._r8;ldew=.05_r8;ldew_rain=ldew;rstfac=.5_r8
endif
if (i==3) then
 lai=1.5_r8;htop=4._r8;ldew=.1_r8;ldew_rain=ldew;rstfac=1._r8;effcon=.05_r8
endif
if (i==4) then
 qm=.02_r8;thm=298._r8;th=297._r8;thv=300._r8
 lai=3._r8;htop=8._r8;ldew=.05_r8;ldew_rain=ldew
endif
"""
radiation = [
    list(map(float, line.split(",")))
    for line in (root / "oracle/fixtures/urban_vegetated_radiation_inputs.csv")
    .read_text(encoding="utf-8")
    .splitlines()
    if not line.startswith("#")
]
assert len(radiation) == 4 and all(len(row) == 57 for row in radiation)
for case, row in enumerate(radiation, 1):
    program += f"if (i=={case}) then\n"
    for n, value in enumerate(row[:25]):
        program += (
            f"Ainv({n // 5 + 1},{n % 5 + 1})="
            + f"{value:.17e}".replace("e", "d")
            + "\n"
        )
    for n, name in enumerate(["B", "B1", "dBdT", "SkyVF", "VegVF"]):
        for j, value in enumerate(row[25 + n * 5 : 30 + n * 5], 1):
            program += f"{name}({j})=" + f"{value:.17e}".replace("e", "d") + "\n"
    program += "ev=" + f"{row[50]:.17e}".replace("e", "d") + "\n"
    for j, value in enumerate(row[51:]):
        program += f"fcover({j})=" + f"{value:.17e}".replace("e", "d") + "\n"
    program += "endif\n"
signature = sub[: sub.index(")") + 1]
signature = re.sub(r"!.*", "", signature)
signature = re.sub(r"&|\n", "", signature)
args = signature[signature.index("(") + 1 : -1].strip()
args = re.sub(r"\bipatch\b|\blbr\b|\blbi\b", "1", args, flags=re.I)
args = re.sub(r"\bnurb\b", "3", args, flags=re.I)
program += (
    "call UrbanVegFlux("
    + args
    + ")\nwrite(*,'("
    + str(len(outputs))
    + "(ES26.17E3,1X))') "
    + ",".join(outputs)
    + "\nenddo\nend program\n"
)
all_rows = []
for fused in [True, False]:
    with tempfile.TemporaryDirectory(prefix="colm-urbanveg-reference-") as directory:
        tmp = Path(directory)
        units = [
            ("precision", (v / "share/MOD_Precision.F90").read_text(encoding="utf-8")),
            ("stubs", stubs),
        ]
        for name in [
            "MOD_Const_Physical",
            "MOD_Qsadv",
            "MOD_FrictionVelocity",
            "MOD_CanopyLayerProfile",
            "MOD_AssimStomataConductance",
        ]:
            units.append((name, (v / f"main/{name}.F90").read_text(encoding="utf-8")))
        units.append(
            (
                "flux",
                "module MOD_Urban_Flux\nuse MOD_SPMD_Task\nuse MOD_Precision\nuse MOD_Vars_Global\nuse MOD_Qsadv\nuse MOD_Namelist\n"
                + constants
                + "\ncontains\n"
                + sub
                + "\n"
                + dew
                + "\nend module\n",
            )
        )
        units.append(("driver", program))
        (tmp / "define.h").write_text("", encoding="utf-8")
        flags = [
            "-O2",
            "-fopenmp",
            "-cpp",
            "-fdefault-real-8",
            "-fdefault-double-8",
            "-ffree-line-length-none",
            "-fno-tree-vectorize",
            "-ffpe-trap=invalid,zero,overflow",
            "-fbacktrace",
            "-ffp-contract=fast" if fused else "-ffp-contract=off",
            "-I",
            str(tmp),
        ]
        if fused and platform.machine().lower() in ("x86_64", "amd64"):
            flags.append("-mfma")
        objects = []
        for name, text in units:
            file = tmp / (name + ".F90")
            file.write_text(text, encoding="utf-8")
            obj = tmp / (name + ".o")
            subprocess.run(
                ["gfortran", *flags, "-c", str(file), "-o", str(obj)],
                cwd=tmp,
                check=True,
            )
            objects.append(str(obj))
        subprocess.run(
            ["gfortran", "-fopenmp", *objects, "-o", str(tmp / "reference")],
            cwd=tmp,
            check=True,
        )
        output = subprocess.check_output([str(tmp / "reference")], text=True, cwd=tmp)
    rows = [
        list(map(float, line.split())) for line in output.splitlines() if line.strip()
    ]
    assert len(rows) == 4 and all(len(row) == len(outputs) for row in rows)
    all_rows.extend(
        ("fused" if fused else "unfused", case, row) for case, row in enumerate(rows)
    )
fixture = root / "oracle/fixtures/urban_vegetated_flux_reference.csv"
expected = [
    line.split(",") for line in fixture.read_text(encoding="utf-8").splitlines()[2:]
]
assert len(expected) == len(all_rows)
for (mode, case, actual), pinned in zip(all_rows, expected):
    assert pinned[:2] == [mode, str(case)]
    for component, (value, reference) in enumerate(zip(actual, map(float, pinned[2:]))):
        assert abs(value - reference) < 5.0e-12, (
            mode,
            case,
            outputs[component],
            value,
            reference,
        )
print(
    f"UrbanVegFlux: 4 full-procedure cases / {len(outputs) * 4} outputs per contraction mode PASS"
)

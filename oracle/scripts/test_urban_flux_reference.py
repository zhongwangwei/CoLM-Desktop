#!/usr/bin/env python3
"""Compile the actual vendor urban bare-flux procedure against pinned cross-language cases."""

from pathlib import Path
import re
import subprocess
import tempfile

root = Path(__file__).resolve().parents[2]
v = root / "vendor/CoLM202X"
s = (v / "main/URBAN/MOD_Urban_Flux.F90").read_text(encoding="utf-8")
sub = re.search(
    r"(?is)   SUBROUTINE UrbanOnlyFlux\b.*?   END SUBROUTINE UrbanOnlyFlux", s
).group(0)
constants = "\n".join(
    line
    for line in s[: s.index("CONTAINS")].splitlines()
    if re.match(r"\s*(integer|logical|real\(r8\)),\s*parameter", line, re.I)
)
stubs = """
module MOD_SPMD_Task
end module
module MOD_Namelist
 integer::DEF_RSS_SCHEME=1
 logical::DEF_VEG_SNOW=.false.
end module
module MOD_Vars_Global
 use MOD_Precision
 real(r8),parameter::PI=acos(-1._r8),spval=-1.e36_r8
end module
module MOD_UserSpecifiedForcing
 character(len=8)::HEIGHT_mode='relative'
end module
"""
inputnames = "deltim hu ht hq us vs thm th thv qm psrf rhoair Fhac Fwst Fach vehc meta hroof hlr z0h_g obug ustarg zlnd zsno fsno_roof fsno_gimp fsno_gper wliq_roofsno wliq_gimpsno wice_roofsno wice_gimpsno htvp_roof htvp_gimp htvp_gper troof twsun twsha tgimp tgper qroof qgimp qgper dqroofdT dqgimpdT dqgperdT rss".split()
outputnames = "taux tauy fsenroof fsenwsun fsenwsha fsengimp fsengper fevproof fevpgimp fevpgper croofs cwsuns cwshas cgrnds croofl cgimpl cgperl croof cgimp cgper tref qref z0m zol rib ustar qstar tstar fm fh fq tafu".split()
vals = [
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
    30,
    9,
    4,
    6,
    3,
    12,
    0.6,
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
    100,
]
base = dict(zip(inputnames, vals))
program = (
    "program reference\nuse MOD_Urban_Flux\nimplicit none\ninteger::i\nreal(r8)::"
    + ",".join(inputnames + outputnames)
    + ",fcover(0:4),froofs(4)\nfroofs=(/.12_r8,.35_r8,.6_r8,.35_r8/)\ndo i=1,4\n"
)
program += "\n".join(f"{name}={value}d0" for name, value in base.items()) + "\n"
program += "fcover(0)=froofs(i)\nfcover(1)=.1_r8\nfcover(2)=.08_r8\nfcover(3)=(1-froofs(i))*.6_r8\nfcover(4)=(1-froofs(i))*.4_r8\nif (i==4) then\n qm=.016_r8;thm=302._r8;th=301._r8;thv=304._r8\nendif\n"
# Actual order uses lbr/lbi and nurb in the full signature.
args = (
    "1,deltim,1,1,hu,ht,hq,us,vs,thm,th,thv,qm,psrf,rhoair,Fhac,Fwst,Fach,vehc,meta,hroof,hlr,2,fcover,z0h_g,obug,ustarg,zlnd,zsno,fsno_roof,fsno_gimp,fsno_gper,wliq_roofsno,wliq_gimpsno,wice_roofsno,wice_gimpsno,htvp_roof,htvp_gimp,htvp_gper,troof,twsun,twsha,tgimp,tgper,qroof,qgimp,qgper,dqroofdT,dqgimpdT,dqgperdT,rss,"
    + ",".join(outputnames)
)
program += (
    "call UrbanOnlyFlux("
    + args
    + ")\nwrite(*,'(32(ES26.17,1X))') "
    + ",".join(outputnames)
    + "\nenddo\nend program\n"
)
with tempfile.TemporaryDirectory(prefix="colm-urbanonly-reference-") as directory:
    tmp = Path(directory)
    text = (v / "share/MOD_Precision.F90").read_text(encoding="utf-8") + "\n" + stubs
    for name in [
        "MOD_Const_Physical",
        "MOD_Qsadv",
        "MOD_FrictionVelocity",
        "MOD_CanopyLayerProfile",
    ]:
        text += "\n" + (v / f"main/{name}.F90").read_text(encoding="utf-8")
    text += (
        "\nmodule MOD_Urban_Flux\nuse MOD_Precision\nuse MOD_Vars_Global\nuse MOD_Qsadv\n"
        + constants
        + "\ncontains\n"
        + sub
        + "\nend module\n"
        + program
    )
    (tmp / "reference.F90").write_text(text, encoding="utf-8")
    (tmp / "define.h").write_text("", encoding="utf-8")
    subprocess.run(
        [
            "gfortran",
            "-O2",
            "-cpp",
            "-ffree-line-length-none",
            "-fdefault-real-8",
            "-fdefault-double-8",
            "-fno-tree-vectorize",
            "-ffpe-trap=invalid,zero,overflow",
            "-fcheck=all",
            str(tmp / "reference.F90"),
            "-I",
            str(tmp),
            "-o",
            str(tmp / "reference"),
        ],
        cwd=tmp,
        check=True,
    )
    output = subprocess.check_output([str(tmp / "reference")], text=True)
fixture = root / "oracle/fixtures/urban_bare_flux_reference.csv"
rows = [list(map(float, line.split())) for line in output.splitlines() if line.strip()]
expected = [
    list(map(float, line.split(",")))
    for line in fixture.read_text(encoding="utf-8").splitlines()[2:]
]
assert len(rows) == len(expected) == 4
for case, (actual, reference) in enumerate(zip(rows, expected)):
    assert len(actual) == len(reference) == 32
    for component, (value, pinned) in enumerate(zip(actual, reference)):
        assert abs(value - pinned) < 5.0e-12, (case, component, value, pinned)
print("UrbanOnlyFlux: 4 full-procedure cases / 128 outputs PASS")

thermal = (v / "main/URBAN/MOD_Urban_Thermal.F90").read_text(encoding="utf-8")
components = "fsen_roof fsen_wsun fsen_wsha fsen_gimp fsen_gper lfevp_roof lfevp_gimp lfevp_gper fsen_urbl lfevp_urbl".split()
assignments = []
for name in components:
    match = re.search(rf"(?im)^\s*{name}\s*=.*$", thermal)
    assert match, name
    assignments.append(match.group(0))
program = (
    """
program components_reference
implicit none
integer::i
real(8)::flake,flakes(3),fcover(0:4),fsenroof,fsenwsun,fsenwsha,fsengimp,fsengper
real(8)::htvp_roof,htvp_gimp,htvp_gper,fevproof,fevpgimp,fevpgper,fsenl,fevpl,hvap
real(8)::"""
    + ",".join(components)
    + """
flakes=(/0.d0,.3d0,1.d0/)
fcover=(/.2d0,.12d0,.08d0,.42d0,.38d0/)
fsenroof=100.d0;fsenwsun=80.d0;fsenwsha=60.d0;fsengimp=40.d0;fsengper=20.d0
htvp_roof=2500000.d0;htvp_gimp=2500000.d0;htvp_gper=2500000.d0
fevproof=.0002d0;fevpgimp=.0001d0;fevpgper=-.00003d0
fsenl=10.d0;fevpl=.000004d0;hvap=2510400.d0
do i=1,3
flake=flakes(i)
"""
    + "\n".join(assignments)
    + "\nwrite(*,'(10(ES26.17,1X))') "
    + ",".join(components)
    + "\nenddo\nend program\n"
)
with tempfile.TemporaryDirectory(prefix="colm-urban-components-") as directory:
    tmp = Path(directory)
    (tmp / "reference.F90").write_text(program, encoding="utf-8")
    subprocess.run(
        [
            "gfortran",
            "-O2",
            "-fdefault-real-8",
            "-fdefault-double-8",
            "-ffree-line-length-none",
            str(tmp / "reference.F90"),
            "-o",
            str(tmp / "reference"),
        ],
        check=True,
    )
    output = subprocess.check_output([str(tmp / "reference")], text=True)
rows = [list(map(float, line.split())) for line in output.splitlines() if line.strip()]
expected = [
    list(map(float, line.split(",")))
    for line in (root / "oracle/fixtures/urban_thermal_components_reference.csv")
    .read_text(encoding="utf-8")
    .splitlines()[2:]
]
assert len(rows) == len(expected) == 3
for case, (actual, reference) in enumerate(zip(rows, expected)):
    assert len(actual) == len(reference) == 10
    for component, (value, pinned) in enumerate(zip(actual, reference)):
        assert abs(value - pinned) < 1.0e-12, (case, component, value, pinned)
print("UrbanTHERMAL component area weights: 3 lake fractions / 30 outputs PASS")

# Both engines share the fractions through their urban flux module.
assert re.search(r"(?im)^\s*USE MOD_Urban_Flux\b", thermal)
for module in ["urban_flux", "urban_thermal"]:
    rust = (root / f"crates/colm-core/src/{module}.rs").read_text(encoding="utf-8")
    if module == "urban_thermal":
        imported = re.search(r"use crate::urban_flux::\{([^}]+)\};", rust)
        assert imported and {"FSH", "FLH"} <= {
            name.strip() for name in imported.group(1).split(",")
        }, "UrbanTHERMAL must import the shared fractions"
        rust = (root / "crates/colm-core/src/urban_flux.rs").read_text(encoding="utf-8")
    vendor_text = s
    for name in ["fsh", "flh"]:
        source = re.search(
            rf"(?im)^\s*real\(r8\),\s*parameter\s*::\s*{name}\s*=\s*([.0-9]+)",
            vendor_text,
        )
        target = re.search(rf"const {name.upper()}: f64 = ([.0-9]+);", rust)
        assert source and target, (module, name)
        assert float(source.group(1)) == float(target.group(1)), (module, name)
print(
    "Anthropogenic sensible/latent heat fractions match the vendor in both Rust modules"
)

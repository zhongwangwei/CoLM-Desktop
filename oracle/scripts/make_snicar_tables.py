#!/usr/bin/env python3
"""合成 SNICAR 光学表与雪粒老化表（本机没有 CESM 的原始文件）。

用法：make_snicar_tables.py <runtime-dir>
写出 <runtime-dir>/snicar/snicar_optics_5bnd_mam_c211006.nc 与 snicar_drdt_bst_fit_60_c070416.nc。

数值只求物理上合理、随粒径/温度平滑变化，用于 Rust 与纯 Fortran 的逐位对比，
不能拿来做科学结论。维度与变量名照 `MOD_SnowSnicar.F90:SnowOptics_init/SnowAge_init`
（bulk 气溶胶，无 MODAL_AER）：冰晶表 C 序 (band=5, radius=1471)，
`flx_wgt_dir` (5, 90, 6)、`flx_wgt_dif` (5, 6)，八种气溶胶各 (5,)；老化表 (11, 31, 8)。
"""
import sys
from pathlib import Path

import numpy as np
from netCDF4 import Dataset

root = Path(sys.argv[1]) / "snicar"
root.mkdir(parents=True, exist_ok=True)

bands = 5
radius = np.arange(30, 1501, dtype=float)  # 30..1500 µm，1471 个
assert radius.size == 1471
# 冰的吸收随波段增强：可见光几乎不吸收，近红外越往后越强。
absorption = np.array([1.0e-6, 1.0e-4, 1.0e-3, 5.0e-3, 2.0e-2])

with Dataset(root / "snicar_optics_5bnd_mam_c211006.nc", "w") as nc:
    nc.createDimension("bnd", bands)
    nc.createDimension("rds", radius.size)
    nc.createDimension("sza", 90)
    nc.createDimension("atm", 6)
    for kind, bump in (("drc", 1.0), ("dfs", 1.02)):
        # (r/30)^0.5 ≤ 7.1，所以单散射反照率落在 (0.85, 1)。
        ssa = 1.0 - bump * absorption[:, None] * (radius[None, :] / 30.0) ** 0.5
        asm = 0.885 + 0.004 * np.log(radius[None, :] / 30.0) + 0.002 * np.arange(bands)[:, None]
        ext = 3.0 / (2.0 * 917.0 * radius[None, :] * 1.0e-6) * (1.0 + 0.01 * np.arange(bands)[:, None])
        for name, values in (("ss_alb", ssa), ("asm_prm", asm), ("ext_cff_mss", ext)):
            var = nc.createVariable(f"{name}_ice_{kind}", "f8", ("bnd", "rds"))
            var[:] = values
    # 入射通量权重：各波段按天顶角缓变，每个 (sza, atm) 上归一。
    sza = np.arange(90, dtype=float)
    base = np.array([0.52, 0.25, 0.12, 0.08, 0.03])
    weights = base[:, None, None] * (1.0 + 0.002 * sza[None, :, None] * (np.arange(bands)[:, None, None] - 1.5))
    weights = weights * (1.0 + 0.01 * np.arange(6)[None, None, :])
    weights /= weights.sum(axis=0, keepdims=True)
    nc.createVariable("flx_wgt_dir", "f8", ("bnd", "sza", "atm"))[:] = weights
    diffuse = base[:, None] * (1.0 + 0.01 * np.arange(6)[None, :])
    nc.createVariable("flx_wgt_dif", "f8", ("bnd", "atm"))[:] = diffuse / diffuse.sum(axis=0, keepdims=True)
    species = {
        "bcphil": (0.20, 0.35, 11000.0),
        "bcphob": (0.28, 0.40, 9000.0),
        "ocphil": (0.95, 0.70, 5000.0),
        "ocphob": (0.90, 0.65, 4500.0),
        "dust01": (0.97, 0.75, 2500.0),
        "dust02": (0.95, 0.78, 900.0),
        "dust03": (0.92, 0.80, 450.0),
        "dust04": (0.88, 0.82, 200.0),
    }
    decay = np.array([1.0, 0.8, 0.6, 0.45, 0.3])
    for suffix, (ssa0, asm0, ext0) in species.items():
        nc.createVariable(f"ss_alb_{suffix}", "f8", ("bnd",))[:] = np.clip(ssa0 * (1.0 - 0.05 * np.arange(bands)), 0.0, 1.0)
        nc.createVariable(f"asm_prm_{suffix}", "f8", ("bnd",))[:] = asm0 * decay ** 0.3
        nc.createVariable(f"ext_cff_mss_{suffix}", "f8", ("bnd",))[:] = ext0 * decay

# 老化表：维度 (T=11, dTdz=31, rho=8)。量级取原始表的典型范围：
# tau ~ 1e0–1e3 h，kappa ~ 1–10，drdsdt0 ~ 1e-3–1 µm/h（温度越高、梯度越大越快）。
t = np.linspace(0.0, 1.0, 11)[:, None, None]
g = np.linspace(0.0, 1.0, 31)[None, :, None]
r = np.linspace(0.0, 1.0, 8)[None, None, :]
with Dataset(root / "snicar_drdt_bst_fit_60_c070416.nc", "w") as nc:
    nc.createDimension("TVals", 11)
    nc.createDimension("dTdzVals", 31)
    nc.createDimension("rhoVals", 8)
    dims = ("TVals", "dTdzVals", "rhoVals")
    nc.createVariable("tau", "f8", dims)[:] = 10.0 ** (0.5 + 2.0 * (1.0 - t) + 0.5 * r - 0.8 * g)
    nc.createVariable("kappa", "f8", dims)[:] = 1.0 + 8.0 * (0.3 + 0.7 * t) * (1.0 - 0.5 * g) * (1.0 - 0.3 * r)
    nc.createVariable("drdsdt0", "f8", dims)[:] = 10.0 ** (-3.0 + 2.5 * t + 0.6 * g - 0.4 * r)
print(f"wrote {root}")

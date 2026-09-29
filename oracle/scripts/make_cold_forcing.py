#!/usr/bin/env python3
"""把一份 PLUMBER2 点强迫整体降温，合成持续积雪的冬季（本机没有深雪站的强迫）。

用法：make_cold_forcing.py <in_Met.nc> <out_Met.nc> <dT_K> [precip_factor]
`Tair` 减 dT，`Precip` 乘 precip_factor（默认 1）；`Qair` 按饱和比湿之比缩放以保持相对湿度（Tetens 公式，冰面/水面按 0 °C 分）；
其余变量原样复制。只用于 Rust 与纯 Fortran 的逐位对比。
"""
import shutil
import sys

import numpy as np
from netCDF4 import Dataset

source, target, delta = sys.argv[1], sys.argv[2], float(sys.argv[3])
factor = float(sys.argv[4]) if len(sys.argv) > 4 else 1.0
shutil.copyfile(source, target)


def qsat(t, p):
    tc = t - 273.15
    e = np.where(tc >= 0.0, 611.2 * np.exp(17.67 * tc / (tc + 243.5)), 611.2 * np.exp(22.46 * tc / (tc + 272.62)))
    return 0.622 * e / (p - 0.378 * e)


def pick(nc, *names):
    """PLUMBER2 的 FLUXNET 类文件叫 `Psurf`/`Precip`，城市站（如 AU-Preston）叫 `PSurf`/`Rainf`。"""
    for name in names:
        if name in nc.variables:
            return name
    raise SystemExit(f"none of {names} in {source}")


with Dataset(target, "a") as nc:
    precip = pick(nc, "Precip", "Rainf")
    t = nc["Tair"][:].astype(float)
    p = nc[pick(nc, "Psurf", "PSurf")][:].astype(float)
    q = nc["Qair"][:].astype(float)
    cold = t - delta
    nc["Tair"][:] = cold
    nc["Qair"][:] = q * qsat(cold, p) / qsat(t, p)
    nc[precip][:] = nc[precip][:].astype(float) * factor
print(f"wrote {target} (Tair - {delta} K, {precip} x {factor})")

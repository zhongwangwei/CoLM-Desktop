#!/usr/bin/env python3
"""合成 `MOD_Aerosol` 的月度气溶胶沉降文件（本机没有 CESM 的原始文件）。

用法：make_aerosol_deposition.py <runtime-dir> [clim|hist]
写 <runtime-dir>/aerosol/ 下与原文件同名的文件：
  clim → aerosoldep_monthly_2000_mean_0.9x1.25_c090529.nc（12 个月）
  hist → aerosoldep_monthly_1849-2001_0.9x1.25_c090529.nc（1836 个月）

上游 `AerosolDepInit` 按文件自带的 `lat`/`lon` 定义网格，所以这里用 10°×10° 的粗网格（原文件 0.9°×1.25°、
逐年版约 5.7 GB）。量级取 BC/OC ~1e-11、粉尘 ~1e-10 kg m-2 s-1，随月份与网格平滑变化；只用于 Rust 与纯 Fortran
的逐位对比，不能拿来做科学结论。变量为 float（与原文件一致），维度 `(time, lat, lon)`。
"""
import sys
from pathlib import Path

import numpy as np
from netCDF4 import Dataset

root = Path(sys.argv[1]) / "aerosol"
kind = sys.argv[2] if len(sys.argv) > 2 else "clim"
root.mkdir(parents=True, exist_ok=True)
name = {
    "clim": "aerosoldep_monthly_2000_mean_0.9x1.25_c090529.nc",
    "hist": "aerosoldep_monthly_1849-2001_0.9x1.25_c090529.nc",
}[kind]
months = 12 if kind == "clim" else (2001 - 1849 + 1) * 12
lat = np.arange(-85.0, 90.0, 10.0)
lon = np.arange(5.0, 360.0, 10.0)
variables = {
    "BCPHIDRY": 2.0e-12, "BCPHODRY": 3.0e-12, "BCDEPWET": 8.0e-12,
    "OCPHIDRY": 5.0e-12, "OCPHODRY": 6.0e-12, "OCDEPWET": 2.0e-11,
    "DSTX01WD": 3.0e-11, "DSTX01DD": 2.0e-11, "DSTX02WD": 8.0e-11, "DSTX02DD": 6.0e-11,
    "DSTX03WD": 1.2e-10, "DSTX03DD": 9.0e-11, "DSTX04WD": 1.5e-10, "DSTX04DD": 1.0e-10,
}
t = np.arange(months)[:, None, None]
y = lat[None, :, None]
x = lon[None, None, :]
with Dataset(root / name, "w") as nc:
    nc.createDimension("time", months)
    nc.createDimension("lat", lat.size)
    nc.createDimension("lon", lon.size)
    nc.createVariable("lat", "f8", ("lat",))[:] = lat
    nc.createVariable("lon", "f8", ("lon",))[:] = lon
    for index, (var, scale) in enumerate(variables.items()):
        field = scale * (1.0 + 0.5 * np.sin(2 * np.pi * (t % 12) / 12.0 + index)) \
            * (1.0 + 0.3 * np.cos(np.radians(y))) * (1.0 + 0.1 * np.sin(np.radians(x)))
        if kind == "hist":
            field = field * (0.5 + t / months)
        nc.createVariable(var, "f4", ("time", "lat", "lon"))[:] = field.astype(np.float32)
print(f"wrote {root / name}")

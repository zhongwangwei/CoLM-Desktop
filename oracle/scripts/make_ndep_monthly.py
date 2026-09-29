#!/usr/bin/env python3
"""为 `DEF_NDEP_FREQUENCY = 2` 的 Fortran/Rust 对照生成合成的月度氮沉降（本机没有真实文件）。

    python3 oracle/scripts/make_ndep_monthly.py <runtime 目录>

写出上游 `init_ndep_data_monthly`（`main/MOD_NdepData.F90`）读的 `<runtime>/ndep/fndep_colm_monthly.nc`：
`NDEP_month(time, lat, lon)`，`time` 为 1849-01 … 2006-12 共 1896 条（`itime = (年-1849)*12 + 月`）。
网格取全球 10°（18×36），`define_by_center` 与面积加权在它上面的行为与原网格相同。

数值只为走遍代码分支：每个格点、每个月都不同（取错格点或错一个月会直接出现在对比里），
量级与年度文件相当（gN/m²/yr）。
"""
import sys
from pathlib import Path

import numpy as np
import netCDF4 as nc

LAT = np.arange(-85.0, 90.0, 10.0)
LON = np.arange(5.0, 360.0, 10.0)


def main():
    ndep = Path(sys.argv[1]) / "ndep"
    ndep.mkdir(parents=True, exist_ok=True)
    ilat, ilon = np.meshgrid(np.arange(LAT.size), np.arange(LON.size), indexing="ij")
    k = (ilat * LON.size + ilon).astype(float)
    t = np.arange(1896)
    month = t % 12
    year = 1849 + t // 12
    seasonal = 1.0 + 0.5 * np.sin(2.0 * np.pi * (month + 0.5) / 12.0)
    trend = 0.2 + (year - 1849) / 157.0 * 1.3
    data = (trend * seasonal)[:, None, None] * (1.0 + 0.01 * k)[None, :, :] + 0.001 * month[:, None, None]
    with nc.Dataset(ndep / "fndep_colm_monthly.nc", "w") as f:
        f.createDimension("time", None)
        f.createDimension("lat", LAT.size)
        f.createDimension("lon", LON.size)
        f.createVariable("lat", "f8", ("lat",))[:] = LAT
        f.createVariable("lon", "f8", ("lon",))[:] = LON
        v = f.createVariable("NDEP_month", "f8", ("time", "lat", "lon"))
        v.units = "g(N)/m2/yr"
        v[:] = data
    print(f"wrote {ndep / 'fndep_colm_monthly.nc'}")


if __name__ == "__main__":
    main()

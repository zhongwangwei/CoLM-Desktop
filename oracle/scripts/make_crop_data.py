#!/usr/bin/env python3
"""为 `DEF_USE_FERT`/`DEF_USE_IRRIGATION` 的 Fortran/Rust 对照生成合成作物管理数据（本机没有真实的 `crop/`）。

    python3 oracle/scripts/make_crop_data.py <runtime 目录>

在 `<runtime>/crop/` 下写出上游 `CROP_readin`（`main/MOD_CropReadin.F90`）读的五个文件，文件名与变量名照上游：
- `plantdt-colm-64cfts-rice2_fillcoast.nc`：`pdrice2(lat,lon)`（带 `missing_value`，上游把它设给整张映射）与
  `PLANTDATE_CFT_15..78(time,lat,lon)`（上游用 `ncio_read_block_time(…, 1, …)` 读，所以带长度 1 的前导维）；
- `fertnitro_fillcoast.nc`：`CONST_FERTNITRO_CFT_15..78(time,lat,lon)`——`DEF_FERT_SOURCE = 1` 时按**种植日文件的网格**读，
  两个文件网格必须相同；
- `fertilizer_2015soc.nc`：自己的网格，`manure(lat,lon)` 与 `fertilizer(cft=64,lat,lon)`，`float`；
- `surfdata_irrigation_method_96x144.nc`：`irrigation_method(cft=64,lat,lon)`，`int`（上游取众数映射）；
- `surfdata_irrigation_allocation.nc`：`irrig_gw_alloc`/`irrig_sw_alloc(lat,lon)`。

数值只为走遍分支：每个格点不同（取错格点会暴露），缺测值、非正值散布在别的格点上。
"""
import sys
from pathlib import Path

import numpy as np
import netCDF4 as nc

MISSING = -9999.0
LAT = np.arange(-85.0, 90.0, 10.0)
LON = np.arange(5.0, 360.0, 10.0)
# `fertilizer_2015soc.nc` 另一张网格，验证它确实按自己的经纬度映射。
LAT2 = np.arange(-82.5, 90.0, 15.0)
LON2 = np.arange(7.5, 360.0, 15.0)
CFTS = range(15, 79)


def cells(lat, lon):
    ilat, ilon = np.meshgrid(np.arange(lat.size), np.arange(lon.size), indexing="ij")
    return (ilat * lon.size + ilon).astype(float)


def cft_missing(k, cft):
    """与 CFT 有关的缺测：US-Ne3 所在格点（k = 494）上大豆（23）缺测、玉米（17）有值。"""
    return ((k + cft) % 43.0) == 1.0


def cft_nonpositive(k, cft):
    """与 CFT 有关的非正值：US-Ne3 格点上冬小麦（21）为负。"""
    return ((k + cft) % 41.0) == 23.0


def grid(f, lat, lon):
    f.createDimension("lat", lat.size)
    f.createDimension("lon", lon.size)
    f.createVariable("lat", "f8", ("lat",))[:] = lat
    f.createVariable("lon", "f8", ("lon",))[:] = lon


def main():
    crop = Path(sys.argv[1]) / "crop"
    crop.mkdir(parents=True, exist_ok=True)
    k = cells(LAT, LON)
    # 每 17 个格点一个缺测，每 23 个格点一个非正值
    missing = (k % 17.0) == 5.0
    nonpositive = (k % 23.0) == 7.0

    with nc.Dataset(crop / "plantdt-colm-64cfts-rice2_fillcoast.nc", "w") as f:
        grid(f, LAT, LON)
        f.createDimension("time", 1)
        v = f.createVariable("pdrice2", "f8", ("lat", "lon"))
        v.missing_value = MISSING
        v[:] = np.where(missing, MISSING, 200.0 + (k % 40.0) + 0.7)
        for cft in CFTS:
            v = f.createVariable(f"PLANTDATE_CFT_{cft:02d}", "f8", ("time", "lat", "lon"))
            v.missing_value = MISSING
            day = 60.0 + ((k * 3.0 + cft) % 150.0) + 0.25
            v[:] = np.where(missing | cft_missing(k, cft), MISSING,
                            np.where(nonpositive | cft_nonpositive(k, cft), -1.0, day))[None]

    with nc.Dataset(crop / "fertnitro_fillcoast.nc", "w") as f:
        grid(f, LAT, LON)
        f.createDimension("time", 1)
        for cft in CFTS:
            v = f.createVariable(f"CONST_FERTNITRO_CFT_{cft:02d}", "f8", ("time", "lat", "lon"))
            v.missing_value = MISSING
            fert = 2.0 + ((k * 5.0 + cft) % 17.0) + 0.125
            v[:] = np.where(missing | cft_missing(k, cft), MISSING,
                            np.where(nonpositive | cft_nonpositive(k, cft), -3.0, fert))[None]

    k2 = cells(LAT2, LON2)
    with nc.Dataset(crop / "fertilizer_2015soc.nc", "w") as f:
        grid(f, LAT2, LON2)
        f.createDimension("cft", 64)
        f.createVariable("manure", "f4", ("lat", "lon"))[:] = np.where(
            (k2 % 13.0) == 3.0, -1.0, 0.5 + (k2 % 9.0) * 0.3)
        f.createVariable("fertilizer", "f4", ("cft", "lat", "lon"))[:] = np.stack(
            [np.where((k2 % 11.0) == 2.0, -2.0, 3.0 + ((k2 + c) % 19.0) * 0.5) for c in range(64)])

    with nc.Dataset(crop / "surfdata_irrigation_method_96x144.nc", "w") as f:
        grid(f, LAT, LON)
        f.createDimension("cft", 64)
        f.createVariable("irrigation_method", "i4", ("cft", "lat", "lon"))[:] = np.stack(
            [np.where((k % 19.0) == 4.0, -1, ((k + c) % 4.0)).astype(np.int32) for c in range(64)])

    with nc.Dataset(crop / "surfdata_irrigation_allocation.nc", "w") as f:
        grid(f, LAT, LON)
        gw = 0.1 + (k % 7.0) * 0.1
        f.createVariable("irrig_gw_alloc", "f8", ("lat", "lon"))[:] = gw
        f.createVariable("irrig_sw_alloc", "f8", ("lat", "lon"))[:] = 1.0 - gw
    print(f"wrote 5 files to {crop}")


if __name__ == "__main__":
    main()

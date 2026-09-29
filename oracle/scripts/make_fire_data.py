#!/usr/bin/env python3
"""为 `DEF_USE_FIRE` 的 Fortran/Rust 对照生成合成火灾数据（本机没有真实的 `fire/` 目录）。

    python3 oracle/scripts/make_fire_data.py <runtime 目录> [泥炭地比例缩放，默认 1]

在 `<runtime>/fire/` 下写出上游 `MOD_FireData`/`MOD_LightningData` 读的五个文件（文件名、变量名、维度与上游
一致）：静态的 `abm`/`peatf`/`gdp`、逐年的 `hdm`（1850–2016）、3 小时气候态闪电 `lnfm`（2920 条）。
网格取全球 10°（18×36），`define_by_center` 与面积加权在它上面的行为与 0.5° 的真实数据相同。

数值只为走遍代码分支，不代表真实世界：
- 每个格点取不同的值，取错格点会直接出现在对比里；
- `hdm` 随年份线性增长：AT-Neu 所在格点 2010 年 ≤ 0.1、2011 年 > 0.1，覆盖 `CNFireArea` 两个分支；
- `lnfm` 带日变化与季节变化，3 小时一换，覆盖 `update_lightning_data` 的换档逻辑。
"""
import sys
from pathlib import Path

import numpy as np
import netCDF4 as nc

LAT = np.arange(-85.0, 90.0, 10.0)
LON = np.arange(5.0, 360.0, 10.0)
YEARS = np.arange(1850, 2017)
NTIME_LIGHTNING = 2920


def cell_index():
    """每个格点一个 0 起的编号（`lat` 快、`lon` 慢无所谓，只要互不相同）。"""
    ilat, ilon = np.meshgrid(np.arange(LAT.size), np.arange(LON.size), indexing="ij")
    return ilat * LON.size + ilon


def write(path, name, data, dims, units):
    with nc.Dataset(path, "w") as f:
        f.createDimension("lat", LAT.size)
        f.createDimension("lon", LON.size)
        if "time" in dims:
            f.createDimension("time", data.shape[0])
        f.createVariable("lat", "f8", ("lat",))[:] = LAT
        f.createVariable("lon", "f8", ("lon",))[:] = LON
        v = f.createVariable(name, "f8", dims)
        v.units = units
        v[:] = data


def main():
    # 可选第二个参数：泥炭地比例的缩放。上游的泥炭火烧掉凋落物与粗木质残体却不计入碳收支检查，误差随火烧
    # 面积累积、到阈值就 abort；缩小它可以得到一个跑满两年的对照。
    peat_scale = float(sys.argv[2]) if len(sys.argv) > 2 else 1.0
    fire = Path(sys.argv[1]) / "fire"
    fire.mkdir(parents=True, exist_ok=True)
    k = cell_index().astype(float)
    # 作物火高峰月（1–12，整数值的实数）
    write(fire / "abm_colm_double_fillcoast.nc", "abm", 1.0 + (k % 12.0), ("lat", "lon"), "month")
    # 泥炭地比例、人均 GDP（千 1995 美元）
    write(fire / "peatf_colm_360x720_c100428.nc", "peatf",
          peat_scale * (0.05 + 0.4 * ((k * 7.0) % 11.0) / 11.0),
          ("lat", "lon"), "fraction")
    write(fire / "gdp_colm_360x720_c100428.nc", "gdp", 1.0 + 30.0 * ((k * 5.0) % 13.0) / 13.0,
          ("lat", "lon"), "1e3 1995US$/capita")
    # 人口密度：格点基数 × (年份 − 2008) × 0.045 → AT-Neu 格点 2010 年 0.09·s、2011 年 0.135·s（s ≈ 1）
    scale = 1.0 + 0.001 * k
    hdm = np.stack([0.045 * max(y - 2008, 0.2) * scale for y in YEARS])
    write(fire / "colmforc.Li_2017_HYDEv3.2_CMIP6_hdm_0.5x0.5_AVHRR_simyr1850-2016_c180202.nc",
          "hdm", hdm, ("time", "lat", "lon"), "counts/km^2")
    # 闪电（次/km²/h）：日变化 × 季节变化 × 格点系数
    t = np.arange(NTIME_LIGHTNING)
    diurnal = 1.0 + 0.8 * np.sin(2.0 * np.pi * (t % 8) / 8.0)
    seasonal = 1.0 + 0.6 * np.sin(2.0 * np.pi * t / NTIME_LIGHTNING)
    lnfm = 0.002 * (diurnal * seasonal)[:, None, None] * (1.0 + (k % 7.0))[None, :, :]
    write(fire / "clmforc.Li_2012_climo1995-2011.T62.lnfm_Total_c140423.nc", "lnfm", lnfm,
          ("time", "lat", "lon"), "counts/km^2/hr")
    print(f"wrote 5 files to {fire}")


if __name__ == "__main__":
    main()

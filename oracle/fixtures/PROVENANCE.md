# oracle/cases/CN-Cng/site.nc 的来历

由 `oracle/scripts/make_site_nc.py` 从
`$PLUMBER2_ROOT/Sitedata/CN-Cng_2008-2009_FLUXNET2015_site.nc` 生成。
sha256: 6132cf1e56e57b01ec7129558eef5c51bb56cdf8c42d85f45bf7a49f3534f507

该脚本**逐字节可复现**（同一输入连跑两次得到同一 sha256，已实测），所以下面那条
「colm-srfdata 必须逐位重现本文件」的要求是可达成的 —— 但**前提是单次写完**。
注意：把同样的数据分多次追加进 NetCDF 会得到数据逐位相同、字节不同的文件
（HDF5 布局差异）。本 fixture 的首版就是那样建的，与脚本产出相差 51 个变量
数据全同、仅文件布局不同。若日后 sha256 对不上，先逐变量比对再怀疑数据。

PLUMBER2 的站点文件不足以驱动 CoLM 单点：`MOD_SingleSrfdata` 对每个字段做
`u_site_x = USE_SITE_x .and. ncio_var_exist(...)`，变量缺失时**没有第三条路**，
直接回落到全球 rawdata 树。本仓库不携带那几百 GB，故合成以下 12 个字段。

**每个字段在 NetCDF 里都带 `source` 属性，标明是合成值而非观测值。**

| 字段 | 值 | 出处 |
|---|---|---|
| `lakedepth` | 1.0 | `MOD_SingleSrfdata.F90:47` 模块默认值 |
| `elevation` | 138.0 | 同站 `Observation/*_Flux.nc` 的 `elevation` |
| `elvstd` | 0.0 | `MOD_SingleSrfdata.F90:88` 模块默认值 |
| `sloperatio` | 0.0 | `MOD_SingleSrfdata.F90:89` 模块默认值（平地） |
| `soil_s_v_alb` / `soil_d_v_alb` / `soil_s_n_alb` / `soil_d_n_alb` | 0.14 / 0.25 / 0.28 / 0.39 | `MOD_SoilColorRefl` 第 10 档 |
| `soil_vf_clay` / `soil_wf_clay` | 非砂/砾/有机质剩余量的 25% | 壤土 1:3 黏:粉假设 |
| `soil_wf_om` | `vf_om × OM_density / BD_all` | 由文件已有量推导 |
| `soil_texture` | 4（Silt loam） | USDA 三角，0–60 cm 深度加权：砂 14.3% / 粉 64.3% / 黏 21.4% |

未合成的字段及原因：
- `depth_to_bedrock` —— `DEF_USE_BEDROCK` 默认 `.false.`，不读
- 降尺度字段（`SITE_svf` / `SITE_cur` / `SITE_sf_lut` / `SITE_slp_type` /
  `SITE_asp_type` / `SITE_area_type`）—— `DEF_USE_Forcing_Downscaling` 默认 `.false.`

**Plan 2 的约束**：`colm-srfdata` 必须能逐位重现本文件，并有测试断言之。
`soil_wf_om` 的推导有一处未确证的语义分歧（`OM_density` 是否已是「每单位土体的
有机质质量」，若是则应为 `OM_density / BD_all` ≈ 0.0154 而非 0.0005）——
见 design.md §11 第 5 条。改动它会使黄金文件失效，必须同时更新黄金文件。

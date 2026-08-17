# oracle/cases/CN-Cng/site.nc 的来历

由 `cargo run -p colm-srfdata --bin site-fill` 从
`$PLUMBER2_ROOT/Sitedata/CN-Cng_2008-2009_FLUXNET2015_site.nc` 加
`$COLM_RAWDATA` 的四张全球栅格生成。
sha256: `6c9f29531254aeb368f426dd55ebf97e1cb7405f4250cb645195e542fda04b2c`

该 sha256 是文档，不是门禁。CI 的检查是重新生成后用 `golden-compare` 做**逐变量
结构比对**，而不是比哈希 —— 因为把同样的数据分多次追加进 NetCDF 会得到数据逐位
相同、字节不同的文件（HDF5 布局差异，实测）。`site-fill` 在单个 `netcdf::append`
会话里写完全部 12 个变量，所以它自身是可复现的；但换一个写入方式哈希就会变，
而那不代表数据变了。

PLUMBER2 的站点文件不足以驱动 CoLM 单点：`MOD_SingleSrfdata` 对每个字段做
`u_site_x = USE_SITE_x .and. ncio_var_exist(...)`，变量缺失时**没有第三条路**，
直接回落到全球 rawdata 树。实测 90 个站点文件的变量集完全相同（各 39 个），
都缺同样的 12 个字段。

取值优先级是**站点自有 > 栅格 > 模块默认**。站点自己有数的地方不该被全球产品
顶掉；栅格只在站点没有对应值时才上场。

**每个字段在 NetCDF 里都带 `source` 属性，写明它走了哪一条路。**

| 字段 | 值 | 来源 |
|---|---|---|
| `soil_texture` | 8（silty loam，BVIC 0.100） | **站点自有**：CoLM 的 USDA 三角作用于站点文件自己的土壤剖面 |
| `elevation` | 138.0 | **站点自有**：同站 `Observation/*_Flux.nc` 的 `Site elevation` |
| `lakedepth` | 0.0 | `rawdata/lake_depth.nc` |
| `elvstd` | 0.49634310603141785 | `rawdata/topography.nc` |
| `sloperatio` | 0.003575807437300682 | 同上（`slope`） |
| `soil_s_v_alb` / `d_v` / `s_n` / `d_n` | 0.14 / 0.25 / 0.28 / 0.39 | `rawdata/soil_brightness.nc` 给出颜色档 10，查 `MOD_SoilColorRefl` 的四张表 |
| `soil_vf_clay` | 固体内体积剩余量的 25% | 由 `vf_sand`/`vf_gravels`/`vf_om` 推导；**黏:粉 1:3 是假设** |
| `soil_wf_clay` | 细土质量剩余量的 25% | 由 `wf_sand` 推导；同一假设 |
| `soil_wf_om` | `OM_density / BD_all` | CoLM 恒等式 `OM_density = BD_ave × wf_om_s × 1000` 的变形 |

未合成的字段及原因：
- `depth_to_bedrock` —— `DEF_USE_BEDROCK` 默认 `.false.`，不读
- 降尺度字段（`SITE_svf` / `SITE_cur` / `SITE_sf_lut` / `SITE_slp_type` /
  `SITE_asp_type` / `SITE_area_type`）—— `DEF_USE_Forcing_Downscaling` 默认 `.false.`

## 取代 `make_site_nc.py`（已删除）

本文件先前由 `oracle/scripts/make_site_nc.py` 生成，那个脚本在四处是错的。
**留着一个已知错误的脚本比删掉更糟**，所以它已被删除；错在哪记在这里，
免得有人从 git 历史里把它挖出来当参考。

1. **USDA 类别编号反了。** 脚本用 1=Sand…12=Clay，CoLM 用
   1=clay…12=sand（`preprocess/rawdata_soil_solids_fractions.F90:253-264`）。
   `MOD_Initialize.F90:420` 拿这个编号直接索引 `BVIC_USDA`，所以编号错一位，
   VIC 入渗形状参数就静默换一个值。CN-Cng 被写成 4（CoLM 读作 clay loam，
   BVIC 0.230）。这一点有三个独立印证：CoLM 源码、`BVIC_USDA` 表的物理排序、
   以及质地栅格自带的 `text_name` 属性。
2. **土壤颜色档硬编码为 10。** 实测 90 个站点里只有 1 个是 10 —— 正是
   CN-Cng，当初唯一验证过的站。其余 89 站的土壤反照率都是错的，而反照率
   直接进地表能量平衡。
3. **`lakedepth` 一律填 1.0。** 栅格在 90 个站点上全是 0。
4. **`wf_om` 多乘了一个 `vf_om`**，小了约 31 倍。本文件旧版第 36 行就把这条
   标成「未确证的语义分歧」并写下了正确答案 0.0154；CoLM 源码现已坐实：
   `OM_density = BD_ave × wf_om_s × 1000`，故 `wf_om = OM_density / BD_all`。

还有一处不算错但基准混用：脚本把 `wf_sand`、`wf_gravels`、`wf_om` 当同一个
分母互减。CoLM 里 `wf_gravels`/`wf_om` 是全土基准，而 `wf_sand` 在
`rd_soil_properties.F90:504` 被 `soil_sand_l / 100.0` 覆盖过，是细土基准。
按同基准去减，17/90 个站点会算出负的粉粒分数。

## 质地类别：站点自己的土壤说了算，栅格只是兜底

`colm-srfdata` 用 CoLM 自己的 USDA 三角作用于**站点文件的土壤剖面**，
栅格只在输入落到三角之外时才上场。两者在 90 个站点里只有 26 个一致，
CN-Cng 上站点自己的土壤给 8 而栅格给 6。

这不是谁错了。栅格的 `:method` 说它出自 SoilGrids v2，站点文件的全局属性说
`Shangguan et al., 2014`，两者在 CN-Cng 像元上连基础参数都对不上
（`theta_s` 0.502 vs 0.428、`BD_all` 1332 vs 1552、`k_s` 11.2 vs 7.88）。
站点文件里也**没有任何一项土壤参数标为站点实测**（只有植被变量带 `source`/`qc`）。
判据是**站点文件的内部一致性**：它的其余土壤参数（`theta_s`、`k_s`、`psi_s`、
`csol`…）会被 CoLM 原样采用，质地类别再从另一个产品取，同一份土壤就自相矛盾了。
所以站点有自己的数时以站点为准。

命令行在两者不同时会把栅格的答案也打出来，不藏这个选择。
`tests/real_sites.rs` 有一条测试钉住一致率的量级（26/90），一致率大幅偏移
说明有一侧变了，值得有人看一眼。

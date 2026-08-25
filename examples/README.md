# 自带的示例站点

CN-Cng（内蒙古草地，FLUXNET2015）、AT-Neu（奥地利草地甲烷站，
FLUXNET-CH4）、AU-Preston（墨尔本城市站）和 US-Ne3（农田站）随安装包一起分发。
CN-Cng 可直接跑完整流程；AT-Neu 用于甲烷建例与评估，运行 BGC /
甲烷前仍需在基本设定中指定 CoLM runtime 数据。US-Ne3 自带最小 BGC runtime，
用于验证 CROP 内核、作物初始化和主模拟链路。

| 文件 | 是什么 |
|---|---|
| `Sitedata/CN-Cng_2008-2009_FLUXNET2015_site.nc` | 站点属性（经纬度、地类、土壤、LAI） |
| `Forcing/CN-Cng_2008-2009_FLUXNET2015_Met.nc` | 半小时强迫场，两年 |
| `Observation/CN-Cng_2008-2009_FLUXNET2015_Flux.nc` | 通量观测，用来算指标 |
| `Sitedata/AT-Neu_2010-2012_FLUXNET-CH4_site.nc` | 甲烷站点属性 |
| `Forcing/AT-Neu_2010-2012_FLUXNET-CH4_Met.nc` | 半小时强迫场，三年 |
| `Observation/AT-Neu_2010-2012_FLUXNET-CH4_Flux.nc` | 含 `FCH4_f_ann` 的甲烷通量观测 |
| `Forcingnml/AT-Neu.nml` | AT-Neu 的风、温、湿观测高度 |
| `Sitedata/US-Ne3_2002-2003_FLUXNET2015_CROP_site.nc` | US-Ne3 农田属性与 CFT 信息 |
| `Forcing/US-Ne3_2002-2003_FLUXNET2015_CROP_Met.nc` | US-Ne3 小时强迫场，两年 |
| `Forcingnml/US-Ne3.nml` | US-Ne3 的风、温、湿观测高度 |
| `Runtime/` | 示例所需的年氮沉降与硝化过程数据 |

## 为什么是这四个站点

CN-Cng 就是黄金回归用的那个站点（`oracle/cases/CN-Cng/`）。示例与测试覆盖
的是同一份数据 —— 示例跑不通，回归测试会先一步红。

AT-Neu 同时有完整的 PLUMBER2 站点属性和 FLUXNET-CH4 半小时
甲烷通量；强迫场与观测只取 2010–2012，观测文件仅保留甲烷
评估所需变量，以控制安装包大小。

US-Ne3 来自已有 CoLM CROP 单点样例。分发版保留 2002–2003 两年强迫，
默认按雨养农田运行，并通过单点种植日覆盖值避免依赖全球作物管理图。

AU-Preston 是 URBAN 预设验收用的那个（README「三个物理预设的实际状态」
一节）。选了 `urban` 内核的人手上得有个能试的东西 —— 而现在这件事不再
需要先下 240 GB 栅格。

## 城市站的数据门槛：已经拆掉了

城市算例的土壤剖面、湖深、土壤反照率、LCZ 分类都不在站点文件里 ——
Urban-PLUMBER 的 23 个变量全是形态学量（建筑高度、道路面积比、树高…）。
这些量原本只能从全球栅格取，而那套数据实测 240 GB。

**现在不用了。** 修掉两个让站点文件分支不可达的上游 Fortran bug
（`vendor/CoLM202X` 的 `fix/urban-site-fallbacks`，尚未 push），再把 21 个站
在七个栅格上的点值预抽成随仓库发的两张表（`urban_soil.rs` 90 KB、
`urban_extra.rs` 250 KB）加一份 `LUCY_rawdata.nc`（37 KB）—— 城市算例一次
只读一格，把那一格抽出来就够。

实测 AU-Preston **完全不给 `--rawdata` / `--runtime`**：三段全 `ok`、
264 条记录，与直接读 122 GB 栅格的参照 run `identical: 146 variables`。
详见根 README「三个物理预设的实际状态」。

**边界：这只覆盖 Urban-PLUMBER 那 21 个站**，表外的城市站点仍然需要
`--rawdata`。

两条还没抹平的坑（都与 rawdata 无关，实测复现，细节在根 README）：
**算例目录不能有空格**（CoLM 用不加引号的 `mkdir -p`，而 GUI 默认把算例
放在 `~/Library/Application Support/…`），以及 **AU-Preston 的默认时间窗口
比强迫场早一天**，要显式给 `--start`。所以严格说，AU-Preston 现在是
「数据齐了、命令给对了就能跑」，还不是「双击进去一路点到底就能跑」。
CN-Cng 装完就能跑通建算例 → 三段运行 → 与观测比对的完整流程
（前提同样是算例目录里没有空格）。AT-Neu 的站点、强迫与甲烷观测
自包含，但 BGC / 甲烷内核还会读取 runtime 中的过程数据。

## 文件动过什么

数据文件都用 netCDF deflate 无损压缩；AT-Neu 观测只保留时间、位置、
`FCH4` / `FCH4_f` / `FCH4_f_ann` 及其 QC，数值未改动。AT-Neu 站点属性来自
PLUMBER2，强迫场与甲烷观测来自 FLUXNET-CH4 2010–2012 半小时产品。

US-Ne3 使用 CROP 内核、BGC 与 PFT/PC；默认关闭施肥和灌溉，并设置
`DEF_TUNING_CROP_PLANTING_DAY=120`。`Runtime/` 中仅保留该示例运行所需的
年氮沉降与十层硝化输入。

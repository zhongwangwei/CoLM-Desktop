# 自带的示例站点

CN-Cng（内蒙古草地，2008–2009，FLUXNET2015），随安装包一起分发，
**装完就能跑一遍完整流程**：建算例 → 三段运行 → 与观测比对。

| 文件 | 是什么 |
|---|---|
| `Sitedata/CN-Cng_2008-2009_FLUXNET2015_site.nc` | 站点属性（经纬度、地类、土壤、LAI） |
| `Forcing/CN-Cng_2008-2009_FLUXNET2015_Met.nc` | 半小时强迫场，两年 |
| `Observation/CN-Cng_2008-2009_FLUXNET2015_Flux.nc` | 通量观测，用来算指标 |

## 为什么是这两个站点

CN-Cng 就是黄金回归用的那个站点（`oracle/cases/CN-Cng/`）。示例与测试覆盖
的是同一份数据 —— 示例跑不通，回归测试会先一步红。

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
CN-Cng 则装完就能跑通建算例 → 三段运行 → 与观测比对的完整流程
（前提同样是算例目录里没有空格）。

## 文件动过什么

三个文件都用 `nccopy -d 5` 重新压缩过，14.9 MB → 680 KB（22 倍）。
**数值逐位相同** —— deflate 是无损的，压缩只改存储不改内容，
且逐变量比对过。除此之外与 PLUMBER2 发布的原始文件一致。

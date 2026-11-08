# 附录 B：参数库索引（1369 项）

本表由 `parameter_catalog` 命令导出，覆盖模型可调参数（参数值、物理/过程参数、参数与资产来源）。`id` 是研究页与助手工具使用的稳定标识；`scope` 说明参数作用范围；`visibility` 说明界面何时显示（`editable-common` = 常用可编辑，其余为专家/派生/只读等）。默认值与来源（`source_location`）来自 CoLM 源码声明。

## 初始场（8 项）

| # | 参数 ID | 原始键 | 中文名 | 默认值 | 单位 | 作用域 | 可见性 | 源码位置 |
|---|---|---|---|---|---|---|---|---|
| 1 | `case:DEF_USE_CN_INIT` | `DEF_USE_CN_INIT` | DEF USE CN INIT | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:325 |
| 2 | `case:DEF_USE_SnowInit` | `DEF_USE_SnowInit` | DEF USE SnowInit | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:322 |
| 3 | `case:DEF_USE_SoilInit` | `DEF_USE_SoilInit` | DEF USE SoilInit | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:319 |
| 4 | `case:DEF_USE_WaterTableInit` | `DEF_USE_WaterTableInit` | DEF USE WaterTableInit | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:328 |
| 5 | `case:DEF_file_SnowInit` | `DEF_file_SnowInit` | DEF file SnowInit | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:323 |
| 6 | `case:DEF_file_SoilInit` | `DEF_file_SoilInit` | DEF file SoilInit | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:320 |
| 7 | `case:DEF_file_WaterTable` | `DEF_file_WaterTable` | DEF file WaterTable | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:329 |
| 8 | `case:DEF_file_cn_init` | `DEF_file_cn_init` | DEF file cn init | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:326 |

## 地表数据（31 项）

| # | 参数 ID | 原始键 | 中文名 | 默认值 | 单位 | 作用域 | 可见性 | 源码位置 |
|---|---|---|---|---|---|---|---|---|
| 1 | `case:DEF_FAST_PC` | `DEF_FAST_PC` | DEF FAST PC | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:271 |
| 2 | `case:DEF_HighResSoil` | `DEF_HighResSoil` | DEF HighResSoil | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:463 |
| 3 | `case:DEF_HighResUrban_albedo` | `DEF_HighResUrban_albedo` | DEF HighResUrban albedo | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:467 |
| 4 | `case:DEF_HighResVeg` | `DEF_HighResVeg` | DEF HighResVeg | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:464 |
| 5 | `case:DEF_LAI_CHANGE_YEARLY` | `DEF_LAI_CHANGE_YEARLY` | DEF LAI CHANGE YEARLY | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:310 |
| 6 | `case:DEF_LAI_END_YEAR` | `DEF_LAI_END_YEAR` | DEF LAI END YEAR | `2020` |  | case-scalar | editable-common | MOD_Namelist.F90:301 |
| 7 | `case:DEF_LAI_MONTHLY` | `DEF_LAI_MONTHLY` | DEF LAI MONTHLY | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:306 |
| 8 | `case:DEF_LAI_START_YEAR` | `DEF_LAI_START_YEAR` | DEF LAI START YEAR | `2000` |  | case-scalar | editable-common | MOD_Namelist.F90:300 |
| 9 | `case:DEF_LANDONLY` | `DEF_LANDONLY` | DEF LANDONLY | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:277 |
| 10 | `case:DEF_LULCC_SCHEME` | `DEF_LULCC_SCHEME` | DEF LULCC SCHEME | `1` |  | case-scalar | editable-common | MOD_Namelist.F90:338 |
| 11 | `case:DEF_PC_CROP_SPLIT` | `DEF_PC_CROP_SPLIT` | DEF PC CROP SPLIT | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:272 |
| 12 | `case:DEF_SOIL_REFL_SCHEME` | `DEF_SOIL_REFL_SCHEME` | DEF SOIL REFL SCHEME | `2` |  | case-scalar | editable-common | MOD_Namelist.F90:287 |
| 13 | `case:DEF_SOLO_PFT` | `DEF_SOLO_PFT` | DEF SOLO PFT | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:270 |
| 14 | `case:DEF_SUBGRID_SCHEME` | `DEF_SUBGRID_SCHEME` | DEF SUBGRID SCHEME | `LCT` |  | case-scalar | editable-common | MOD_Namelist.F90:275 |
| 15 | `case:DEF_Srfdata_CompressLevel` | `DEF_Srfdata_CompressLevel` | DEF Srfdata CompressLevel | `1` |  | case-scalar | editable-common | MOD_Namelist.F90:293 |
| 16 | `case:DEF_USE_DOMINANT_PATCHTYPE` | `DEF_USE_DOMINANT_PATCHTYPE` | DEF USE DOMINANT PATCHTYPE | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:278 |
| 17 | `case:DEF_USE_ESACCI` | `DEF_USE_ESACCI` | DEF USE ESACCI | `.false.` |  | case-scalar | read-only-context | MOD_Namelist.F90:255 |
| 18 | `case:DEF_USE_GLC30` | `DEF_USE_GLC30` | DEF USE GLC30 | `.false.` |  | case-scalar | read-only-context | MOD_Namelist.F90:254 |
| 19 | `case:DEF_USE_IGBP` | `DEF_USE_IGBP` | DEF USE IGBP | `.false.` |  | case-scalar | read-only-context | MOD_Namelist.F90:262 |
| 20 | `case:DEF_USE_LCT` | `DEF_USE_LCT` | DEF USE LCT | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:267 |
| 21 | `case:DEF_USE_LULCC` | `DEF_USE_LULCC` | DEF USE LULCC | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:344 |
| 22 | `case:DEF_USE_PC` | `DEF_USE_PC` | DEF USE PC | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:269 |
| 23 | `case:DEF_USE_PFT` | `DEF_USE_PFT` | DEF USE PFT | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:268 |
| 24 | `case:DEF_USE_SOILPAR_UPS_FIT` | `DEF_USE_SOILPAR_UPS_FIT` | DEF USE SOILPAR UPS FIT | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:282 |
| 25 | `case:DEF_USE_USGS` | `DEF_USE_USGS` | DEF USE USGS | `.false.` |  | case-scalar | read-only-context | MOD_Namelist.F90:261 |
| 26 | `case:DEF_dir_existing_srfdata` | `DEF_dir_existing_srfdata` | DEF dir existing srfdata | `path/to/landdata` |  | case-scalar | editable-common | MOD_Namelist.F90:150 |
| 27 | `case:DEF_dir_rawdata` | `DEF_dir_rawdata` | DEF dir rawdata | `path/to/rawdata/` |  | case-scalar | editable-common | MOD_Namelist.F90:127 |
| 28 | `case:DEF_rawdata_namelist` | `DEF_rawdata_namelist` | DEF rawdata namelist | `./rawdata/colm2024.nml` |  | case-scalar | editable-common | MOD_Namelist.F90:168 |
| 29 | `case:USE_srfdata_from_3D_gridded_data` | `USE_srfdata_from_3D_gridded_data` | USE srfdata from 3D gridded data | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:153 |
| 30 | `case:USE_srfdata_from_larger_region` | `USE_srfdata_from_larger_region` | USE srfdata from larger region | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:149 |
| 31 | `case:USE_zip_for_aggregation` | `USE_zip_for_aggregation` | USE zip for aggregation | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:290 |

## 城市（13 项）

| # | 参数 ID | 原始键 | 中文名 | 默认值 | 单位 | 作用域 | 可见性 | 源码位置 |
|---|---|---|---|---|---|---|---|---|
| 1 | `case:DEF_URBAN_BEM` | `DEF_URBAN_BEM` | DEF URBAN BEM | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:356 |
| 2 | `case:DEF_URBAN_LUCY` | `DEF_URBAN_LUCY` | DEF URBAN LUCY | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:359 |
| 3 | `case:DEF_URBAN_ONLY` | `DEF_URBAN_ONLY` | DEF URBAN ONLY | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:354 |
| 4 | `case:DEF_URBAN_RUN` | `DEF_URBAN_RUN` | DEF URBAN RUN | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:355 |
| 5 | `case:DEF_URBAN_TREE` | `DEF_URBAN_TREE` | DEF URBAN TREE | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:357 |
| 6 | `case:DEF_URBAN_WATER` | `DEF_URBAN_WATER` | DEF URBAN WATER | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:358 |
| 7 | `case:DEF_URBAN_type_scheme` | `DEF_URBAN_type_scheme` | DEF URBAN type scheme | `2` |  | case-scalar | editable-common | MOD_Namelist.F90:353 |
| 8 | `case:DEF_USE_CANYON_HWR` | `DEF_USE_CANYON_HWR` | DEF USE CANYON HWR | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:360 |
| 9 | `case:USE_SITE_urban_ecology` | `USE_SITE_urban_ecology` | USE SITE urban ecology | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:92 |
| 10 | `case:USE_SITE_urban_geometry` | `USE_SITE_urban_geometry` | USE SITE urban geometry | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:91 |
| 11 | `case:USE_SITE_urban_human` | `USE_SITE_urban_human` | USE SITE urban human | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:95 |
| 12 | `case:USE_SITE_urban_radiation` | `USE_SITE_urban_radiation` | USE SITE urban radiation | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:93 |
| 13 | `case:USE_SITE_urban_thermal` | `USE_SITE_urban_thermal` | USE SITE urban thermal | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:94 |

## 强迫场（48 项）

| # | 参数 ID | 原始键 | 中文名 | 默认值 | 单位 | 作用域 | 可见性 | 源码位置 |
|---|---|---|---|---|---|---|---|---|
| 1 | `case:DEF_DS_HiresTopographyDataDir` | `DEF_DS_HiresTopographyDataDir` | DEF DS HiresTopographyDataDir | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:792 |
| 2 | `case:DEF_DS_LONGWAVE_LAPSE_RATE` | `DEF_DS_LONGWAVE_LAPSE_RATE` | DEF DS LONGWAVE LAPSE RATE | `0.032_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:664 |
| 3 | `case:DEF_DS_LONGWAVE_LIMIT` | `DEF_DS_LONGWAVE_LIMIT` | DEF DS LONGWAVE LIMIT | `0.5_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:665 |
| 4 | `case:DEF_DS_SHORTWAVE_LIMIT` | `DEF_DS_SHORTWAVE_LIMIT` | DEF DS SHORTWAVE LIMIT | `0.5_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:666 |
| 5 | `case:DEF_DS_SHORTWAVE_SIMPLE_LIMIT` | `DEF_DS_SHORTWAVE_SIMPLE_LIMIT` | DEF DS SHORTWAVE SIMPLE LIMIT | `0.2_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:667 |
| 6 | `case:DEF_DS_TEMP_LAPSE_RATE` | `DEF_DS_TEMP_LAPSE_RATE` | DEF DS TEMP LAPSE RATE | `0.006_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:663 |
| 7 | `case:DEF_DS_longwave_adjust_scheme` | `DEF_DS_longwave_adjust_scheme` | DEF DS longwave adjust scheme | `II` |  | case-scalar | editable-common | MOD_Namelist.F90:794 |
| 8 | `case:DEF_DS_precipitation_adjust_scheme` | `DEF_DS_precipitation_adjust_scheme` | DEF DS precipitation adjust scheme | `I` |  | case-scalar | editable-common | MOD_Namelist.F90:793 |
| 9 | `case:DEF_Forcing_Interp_Method` | `DEF_Forcing_Interp_Method` | DEF Forcing Interp Method | `arealweight` |  | case-scalar | editable-common | MOD_Namelist.F90:788 |
| 10 | `case:DEF_USE_CBL_HEIGHT` | `DEF_USE_CBL_HEIGHT` | DEF USE CBL HEIGHT | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:786 |
| 11 | `case:DEF_USE_ClimForcing_for_Spinup` | `DEF_USE_ClimForcing_for_Spinup` | DEF USE ClimForcing for Spinup | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:796 |
| 12 | `case:DEF_USE_Forcing_Downscaling` | `DEF_USE_Forcing_Downscaling` | DEF USE Forcing Downscaling | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:790 |
| 13 | `case:DEF_USE_Forcing_Downscaling_Simple` | `DEF_USE_Forcing_Downscaling_Simple` | DEF USE Forcing Downscaling Simple | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:791 |
| 14 | `case:DEF_dir_forcing` | `DEF_dir_forcing` | DEF dir forcing | `path/to/forcing/data` |  | case-scalar | editable-common | MOD_Namelist.F90:722 |
| 15 | `case:DEF_forcing%CBL_dtime` | `DEF_forcing%CBL_dtime` | DEF forcing CBL dtime | `21600` |  | case-scalar | editable-common | MOD_Namelist.F90:779 |
| 16 | `case:DEF_forcing%CBL_fprefix` | `DEF_forcing%CBL_fprefix` | DEF forcing CBL fprefix | `TPHWL6Hrly/clmforc.cruncep.V4.c2011.0.5d.TPQWL.` |  | case-scalar | editable-common | MOD_Namelist.F90:776 |
| 17 | `case:DEF_forcing%CBL_offset` | `DEF_forcing%CBL_offset` | DEF forcing CBL offset | `10800` |  | case-scalar | editable-common | MOD_Namelist.F90:780 |
| 18 | `case:DEF_forcing%CBL_tintalgo` | `DEF_forcing%CBL_tintalgo` | DEF forcing CBL tintalgo | `linear` |  | case-scalar | editable-common | MOD_Namelist.F90:778 |
| 19 | `case:DEF_forcing%CBL_vname` | `DEF_forcing%CBL_vname` | DEF forcing CBL vname | `blh` |  | case-scalar | editable-common | MOD_Namelist.F90:777 |
| 20 | `case:DEF_forcing%HEIGHT_Q` | `DEF_forcing%HEIGHT_Q` | DEF forcing HEIGHT Q | `50.` |  | case-scalar | editable-common | MOD_Namelist.F90:731 |
| 21 | `case:DEF_forcing%HEIGHT_T` | `DEF_forcing%HEIGHT_T` | DEF forcing HEIGHT T | `50.` |  | case-scalar | editable-common | MOD_Namelist.F90:730 |
| 22 | `case:DEF_forcing%HEIGHT_V` | `DEF_forcing%HEIGHT_V` | DEF forcing HEIGHT V | `100.0` |  | case-scalar | editable-common | MOD_Namelist.F90:729 |
| 23 | `case:DEF_forcing%HEIGHT_mode` | `DEF_forcing%HEIGHT_mode` | DEF forcing HEIGHT mode | `absolute` |  | case-scalar | editable-common | MOD_Namelist.F90:728 |
| 24 | `case:DEF_forcing%NVAR` | `DEF_forcing%NVAR` | DEF forcing NVAR | `8` |  | case-scalar | editable-common | MOD_Namelist.F90:738 |
| 25 | `case:DEF_forcing%data2d` | `DEF_forcing%data2d` | DEF forcing data2d | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:748 |
| 26 | `case:DEF_forcing%dataset` | `DEF_forcing%dataset` | DEF forcing dataset | `CRUNCEP` |  | case-scalar | editable-common | MOD_Namelist.F90:726 |
| 27 | `case:DEF_forcing%dim2d` | `DEF_forcing%dim2d` | DEF forcing dim2d | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:750 |
| 28 | `case:DEF_forcing%dtime` | `DEF_forcing%dtime` | DEF forcing dtime | `(/21600,21600,21600,21600,0,21600,21600,21600/)` |  | case-scalar | editable-common | MOD_Namelist.F90:743 |
| 29 | `case:DEF_forcing%endmo` | `DEF_forcing%endmo` | DEF forcing endmo | `12` |  | case-scalar | editable-common | MOD_Namelist.F90:742 |
| 30 | `case:DEF_forcing%endyr` | `DEF_forcing%endyr` | DEF forcing endyr | `2003` |  | case-scalar | editable-common | MOD_Namelist.F90:741 |
| 31 | `case:DEF_forcing%fprefix` | `DEF_forcing%fprefix` | DEF forcing fprefix | `(/ 'TPHWL6Hrly/clmforc.cruncep.V4.c2011.0.5d.TPQWL.', 'TPHWL6Hrly/clmforc.cruncep.V4.c2011.0.5d.TPQWL.', 'TPHWL6Hrly/clmforc.cruncep.V4.c2011.0.5d.TPQWL.', 'Precip6Hrly/clmforc.cruncep.V4.c2011.0.5d.Prec.', 'NULL                                           ', 'TPHWL6Hrly/clmforc.cruncep.V4.c2011.0.5d.TPQWL.', 'Solar6Hrly/clmforc.cruncep.V4.c2011.0.5d.Solr. ', 'TPHWL6Hrly/clmforc.cruncep.V4.c2011.0.5d.TPQWL.' /)` |  | case-scalar | editable-common | MOD_Namelist.F90:757 |
| 32 | `case:DEF_forcing%groupby` | `DEF_forcing%groupby` | DEF forcing groupby | `month` |  | case-scalar | editable-common | MOD_Namelist.F90:755 |
| 33 | `case:DEF_forcing%has_missing_value` | `DEF_forcing%has_missing_value` | DEF forcing has missing value | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:735 |
| 34 | `case:DEF_forcing%hightdim` | `DEF_forcing%hightdim` | DEF forcing hightdim | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:749 |
| 35 | `case:DEF_forcing%latname` | `DEF_forcing%latname` | DEF forcing latname | `LATIXY` |  | case-scalar | editable-common | MOD_Namelist.F90:752 |
| 36 | `case:DEF_forcing%leapyear` | `DEF_forcing%leapyear` | DEF forcing leapyear | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:747 |
| 37 | `case:DEF_forcing%lonname` | `DEF_forcing%lonname` | DEF forcing lonname | `LONGXY` |  | case-scalar | editable-common | MOD_Namelist.F90:753 |
| 38 | `case:DEF_forcing%missing_value_name` | `DEF_forcing%missing_value_name` | DEF forcing missing value name | `missing_value` |  | case-scalar | editable-common | MOD_Namelist.F90:736 |
| 39 | `case:DEF_forcing%nlands` | `DEF_forcing%nlands` | DEF forcing nlands | `1` |  | case-scalar | editable-common | MOD_Namelist.F90:745 |
| 40 | `case:DEF_forcing%offset` | `DEF_forcing%offset` | DEF forcing offset | `(/10800,10800,10800,10800,0,10800,0,10800/)` |  | case-scalar | editable-common | MOD_Namelist.F90:744 |
| 41 | `case:DEF_forcing%regbnd` | `DEF_forcing%regbnd` | DEF forcing regbnd | `(/-90.0, 90.0, -180.0, 180.0/)` |  | case-scalar | editable-common | MOD_Namelist.F90:734 |
| 42 | `case:DEF_forcing%regional` | `DEF_forcing%regional` | DEF forcing regional | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:733 |
| 43 | `case:DEF_forcing%solarin_all_band` | `DEF_forcing%solarin_all_band` | DEF forcing solarin all band | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:727 |
| 44 | `case:DEF_forcing%startmo` | `DEF_forcing%startmo` | DEF forcing startmo | `1` |  | case-scalar | editable-common | MOD_Namelist.F90:740 |
| 45 | `case:DEF_forcing%startyr` | `DEF_forcing%startyr` | DEF forcing startyr | `2000` |  | case-scalar | editable-common | MOD_Namelist.F90:739 |
| 46 | `case:DEF_forcing%timelog` | `DEF_forcing%timelog` | DEF forcing timelog | `(/ 'instant ','instant ','instant ','forward ', 'NULL    ','instant ','forward ','forward ' /)` |  | case-scalar | editable-common | MOD_Namelist.F90:769 |
| 47 | `case:DEF_forcing%tintalgo` | `DEF_forcing%tintalgo` | DEF forcing tintalgo | `(/ 'linear ','linear ','linear ','nearest', 'NULL   ','linear ','coszen ','linear ' /)` |  | case-scalar | editable-common | MOD_Namelist.F90:772 |
| 48 | `case:DEF_forcing%vname` | `DEF_forcing%vname` | DEF forcing vname | `(/ 'TBOT    ','QBOT    ','PSRF    ','PRECTmms', 'NULL    ','WIND    ','FSDS    ','FLDS    ' /)` |  | case-scalar | editable-common | MOD_Namelist.F90:766 |

## 数据同化（11 项）

| # | 参数 ID | 原始键 | 中文名 | 默认值 | 单位 | 作用域 | 可见性 | 源码位置 |
|---|---|---|---|---|---|---|---|---|
| 1 | `case:DEF_DA_ENS_NUM` | `DEF_DA_ENS_NUM` | DEF DA ENS NUM | `20` |  | case-scalar | editable-common | MOD_Namelist.F90:805 |
| 2 | `case:DEF_DA_ENS_SM` | `DEF_DA_ENS_SM` | DEF DA ENS SM | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:806 |
| 3 | `case:DEF_DA_RTM_diel` | `DEF_DA_RTM_diel` | DEF DA RTM diel | `0` |  | case-scalar | editable-common | MOD_Namelist.F90:810 |
| 4 | `case:DEF_DA_RTM_rough` | `DEF_DA_RTM_rough` | DEF DA RTM rough | `0` |  | case-scalar | editable-common | MOD_Namelist.F90:811 |
| 5 | `case:DEF_DA_SM` | `DEF_DA_SM` | DEF DA SM | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:804 |
| 6 | `case:DEF_DA_SM_FY` | `DEF_DA_SM_FY` | DEF DA SM FY | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:808 |
| 7 | `case:DEF_DA_SM_SMAP` | `DEF_DA_SM_SMAP` | DEF DA SM SMAP | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:807 |
| 8 | `case:DEF_DA_SM_SYNOP` | `DEF_DA_SM_SYNOP` | DEF DA SM SYNOP | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:809 |
| 9 | `case:DEF_DA_TWS` | `DEF_DA_TWS` | DEF DA TWS | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:802 |
| 10 | `case:DEF_DA_TWS_GRACE` | `DEF_DA_TWS_GRACE` | DEF DA TWS GRACE | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:803 |
| 11 | `case:DEF_DA_obsdir` | `DEF_DA_obsdir` | DEF DA obsdir | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:801 |

## 文件与目录（3 项）

| # | 参数 ID | 原始键 | 中文名 | 默认值 | 单位 | 作用域 | 可见性 | 源码位置 |
|---|---|---|---|---|---|---|---|---|
| 1 | `case:DEF_dir_landdata` | `DEF_dir_landdata` | DEF dir landdata | `path/to/landdata` |  | case-scalar | read-only-context | MOD_Namelist.F90:131 |
| 2 | `case:DEF_dir_runtime` | `DEF_dir_runtime` | DEF dir runtime | `path/to/runtime/` |  | case-scalar | editable-common | MOD_Namelist.F90:128 |
| 3 | `case:DEF_forcing_namelist` | `DEF_forcing_namelist` | DEF forcing namelist | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:720 |

## 时间与预热（15 项）

| # | 参数 ID | 原始键 | 中文名 | 默认值 | 单位 | 作用域 | 可见性 | 源码位置 |
|---|---|---|---|---|---|---|---|---|
| 1 | `case:DEF_simulation_time%end_day` | `DEF_simulation_time%end_day` | DEF simulation time end day | `1` |  | case-scalar | editable-common | MOD_Namelist.F90:111 |
| 2 | `case:DEF_simulation_time%end_month` | `DEF_simulation_time%end_month` | DEF simulation time end month | `1` |  | case-scalar | editable-common | MOD_Namelist.F90:110 |
| 3 | `case:DEF_simulation_time%end_sec` | `DEF_simulation_time%end_sec` | DEF simulation time end sec | `0` |  | case-scalar | editable-common | MOD_Namelist.F90:112 |
| 4 | `case:DEF_simulation_time%end_year` | `DEF_simulation_time%end_year` | DEF simulation time end year | `2003` |  | case-scalar | editable-common | MOD_Namelist.F90:109 |
| 5 | `case:DEF_simulation_time%greenwich` | `DEF_simulation_time%greenwich` | DEF simulation time greenwich | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:104 |
| 6 | `case:DEF_simulation_time%spinup_day` | `DEF_simulation_time%spinup_day` | DEF simulation time spinup day | `1` |  | case-scalar | editable-common | MOD_Namelist.F90:115 |
| 7 | `case:DEF_simulation_time%spinup_month` | `DEF_simulation_time%spinup_month` | DEF simulation time spinup month | `1` |  | case-scalar | editable-common | MOD_Namelist.F90:114 |
| 8 | `case:DEF_simulation_time%spinup_repeat` | `DEF_simulation_time%spinup_repeat` | DEF simulation time spinup repeat | `1` |  | case-scalar | editable-common | MOD_Namelist.F90:117 |
| 9 | `case:DEF_simulation_time%spinup_sec` | `DEF_simulation_time%spinup_sec` | DEF simulation time spinup sec | `0` |  | case-scalar | editable-common | MOD_Namelist.F90:116 |
| 10 | `case:DEF_simulation_time%spinup_year` | `DEF_simulation_time%spinup_year` | DEF simulation time spinup year | `0` |  | case-scalar | editable-common | MOD_Namelist.F90:113 |
| 11 | `case:DEF_simulation_time%start_day` | `DEF_simulation_time%start_day` | DEF simulation time start day | `1` |  | case-scalar | editable-common | MOD_Namelist.F90:107 |
| 12 | `case:DEF_simulation_time%start_month` | `DEF_simulation_time%start_month` | DEF simulation time start month | `1` |  | case-scalar | editable-common | MOD_Namelist.F90:106 |
| 13 | `case:DEF_simulation_time%start_sec` | `DEF_simulation_time%start_sec` | DEF simulation time start sec | `0` |  | case-scalar | editable-common | MOD_Namelist.F90:108 |
| 14 | `case:DEF_simulation_time%start_year` | `DEF_simulation_time%start_year` | DEF simulation time start year | `2000` |  | case-scalar | editable-common | MOD_Namelist.F90:105 |
| 15 | `case:DEF_simulation_time%timestep` | `DEF_simulation_time%timestep` | DEF simulation time timestep | `1800.` |  | case-scalar | editable-common | MOD_Namelist.F90:118 |

## 材料（229 项）

| # | 参数 ID | 原始键 | 中文名 | 默认值 | 单位 | 作用域 | 可见性 | 源码位置 |
|---|---|---|---|---|---|---|---|---|
| 1 | `case:DEF_ISOTOPE_PARAM_FILE` | `DEF_ISOTOPE_PARAM_FILE` | DEF ISOTOPE PARAM FILE | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:45 |
| 2 | `case:DEF_ISOTOPE_SPECIES` | `DEF_ISOTOPE_SPECIES` | DEF ISOTOPE SPECIES | `H2_18O,HDO` |  | case-scalar | editable-common | MOD_Namelist.F90:44 |
| 3 | `case:DEF_ISO_CANOPY_EQUILIBRATION` | `DEF_ISO_CANOPY_EQUILIBRATION` | DEF ISO CANOPY EQUILIBRATION | `0.0_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:133 |
| 4 | `case:DEF_ISO_CG_RELHUM_MAX` | `DEF_ISO_CG_RELHUM_MAX` | DEF ISO CG RELHUM MAX | `0.99_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:127 |
| 5 | `case:DEF_ISO_ICE_SUPERSAT_SLOPE` | `DEF_ISO_ICE_SUPERSAT_SLOPE` | DEF ISO ICE SUPERSAT SLOPE | `0.003_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:126 |
| 6 | `case:DEF_ISO_INIT_DELTA_H2_18O` | `DEF_ISO_INIT_DELTA_H2_18O` | DEF ISO INIT DELTA H2 18O | `-10._r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:141 |
| 7 | `case:DEF_ISO_INIT_DELTA_HDO` | `DEF_ISO_INIT_DELTA_HDO` | DEF ISO INIT DELTA HDO | `-70._r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:143 |
| 8 | `case:DEF_ISO_KINETIC_SCHEME` | `DEF_ISO_KINETIC_SCHEME` | DEF ISO KINETIC SCHEME | `MERLIVAT1978` |  | case-scalar | editable-common | MOD_Namelist.F90:125 |
| 9 | `case:DEF_ISO_NSS_LEAF_PATH_LENGTH` | `DEF_ISO_NSS_LEAF_PATH_LENGTH` | DEF ISO NSS LEAF PATH LENGTH | `0.01_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:136 |
| 10 | `case:DEF_ISO_NSS_LEAF_RB` | `DEF_ISO_NSS_LEAF_RB` | DEF ISO NSS LEAF RB | `100._r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:137 |
| 11 | `case:DEF_ISO_NSS_LEAF_WATER_PER_LAI` | `DEF_ISO_NSS_LEAF_WATER_PER_LAI` | DEF ISO NSS LEAF WATER PER LAI | `0.12_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:135 |
| 12 | `case:DEF_ISO_OPEN_WATER_KINETIC` | `DEF_ISO_OPEN_WATER_KINETIC` | DEF ISO OPEN WATER KINETIC | `MJ79` |  | case-scalar | editable-common | MOD_Namelist.F90:128 |
| 13 | `case:DEF_ISO_REF_RATIO_H2_18O` | `DEF_ISO_REF_RATIO_H2_18O` | DEF ISO REF RATIO H2 18O | `2.0052e-3_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:140 |
| 14 | `case:DEF_ISO_REF_RATIO_HDO` | `DEF_ISO_REF_RATIO_HDO` | DEF ISO REF RATIO HDO | `1.5576e-4_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:142 |
| 15 | `case:DEF_ISO_SNOWMELT_EQUILIBRATION` | `DEF_ISO_SNOWMELT_EQUILIBRATION` | DEF ISO SNOWMELT EQUILIBRATION | `0.0_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:134 |
| 16 | `case:DEF_ISO_SOIL_DIFFUSION` | `DEF_ISO_SOIL_DIFFUSION` | DEF ISO SOIL DIFFUSION | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:131 |
| 17 | `case:DEF_ISO_SOIL_KINETIC` | `DEF_ISO_SOIL_KINETIC` | DEF ISO SOIL KINETIC | `RESISTANCE` |  | case-scalar | editable-common | MOD_Namelist.F90:130 |
| 18 | `case:DEF_ISO_SOIL_VAPOR_DIFFUSION` | `DEF_ISO_SOIL_VAPOR_DIFFUSION` | DEF ISO SOIL VAPOR DIFFUSION | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:132 |
| 19 | `case:DEF_ISO_SUBL_SKIN_MM` | `DEF_ISO_SUBL_SKIN_MM` | DEF ISO SUBL SKIN MM | `5.0_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:129 |
| 20 | `case:DEF_ISO_USE_FRACTIONATION` | `DEF_ISO_USE_FRACTIONATION` | DEF ISO USE FRACTIONATION | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:124 |
| 21 | `case:DEF_METHANE_PARAM_FILE` | `DEF_METHANE_PARAM_FILE` | DEF METHANE PARAM FILE | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:59 |
| 22 | `case:DEF_MT_AQUIFER_MIXING_WATER_MM` | `DEF_MT_AQUIFER_MIXING_WATER_MM` | DEF MT AQUIFER MIXING WATER MM | `-1._r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:67 |
| 23 | `case:DEF_MT_BALANCE_ABORT_NBAD` | `DEF_MT_BALANCE_ABORT_NBAD` | DEF MT BALANCE ABORT NBAD | `0` |  | case-scalar | editable-expert | MOD_Namelist.F90:62 |
| 24 | `case:DEF_MT_FORCING_MEMORY_PRECIP` | `DEF_MT_FORCING_MEMORY_PRECIP` | DEF MT FORCING MEMORY PRECIP | `2.e-5_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:88 |
| 25 | `case:DEF_MT_FORCING_MIN_PRECIP` | `DEF_MT_FORCING_MIN_PRECIP` | DEF MT FORCING MIN PRECIP | `5.e-6_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:78 |
| 26 | `case:DEF_MT_LULCC_ABORT_NBAD` | `DEF_MT_LULCC_ABORT_NBAD` | DEF MT LULCC ABORT NBAD | `0` |  | case-scalar | editable-expert | MOD_Namelist.F90:64 |
| 27 | `case:DEF_MT_N2O_EF5_NH4` | `DEF_MT_N2O_EF5_NH4` | DEF MT N2O EF5 NH4 | `-1._r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:117 |
| 28 | `case:DEF_MT_N2O_EF5_NO3` | `DEF_MT_N2O_EF5_NO3` | DEF MT N2O EF5 NO3 | `-1._r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:118 |
| 29 | `case:DEF_MT_RESID_ABORT_NBAD` | `DEF_MT_RESID_ABORT_NBAD` | DEF MT RESID ABORT NBAD | `0` |  | case-scalar | editable-expert | MOD_Namelist.F90:63 |
| 30 | `case:DEF_MT_SOIL_INIT_FILE` | `DEF_MT_SOIL_INIT_FILE` | DEF MT SOIL INIT FILE | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:120 |
| 31 | `case:DEF_MT_SOIL_INIT_VARS` | `DEF_MT_SOIL_INIT_VARS` | DEF MT SOIL INIT VARS | `` |  | case-scalar | editable-common | MOD_Namelist.F90:121 |
| 32 | `case:DEF_MT_USE_SOIL_INIT` | `DEF_MT_USE_SOIL_INIT` | DEF MT USE SOIL INIT | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:119 |
| 33 | `case:DEF_NH4_LEACHING` | `DEF_NH4_LEACHING` | DEF NH4 LEACHING | `auto` |  | case-scalar | editable-common | MOD_Namelist.F90:51 |
| 34 | `case:DEF_SEDIMENT_PARAM_FILE` | `DEF_SEDIMENT_PARAM_FILE` | DEF SEDIMENT PARAM FILE | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:56 |
| 35 | `case:DEF_SOLUTE_NAMES` | `DEF_SOLUTE_NAMES` | DEF SOLUTE NAMES | `` |  | case-scalar | editable-common | MOD_Namelist.F90:46 |
| 36 | `case:DEF_SOLUTE_PARAM_FILES` | `DEF_SOLUTE_PARAM_FILES` | DEF SOLUTE PARAM FILES | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:55 |
| 37 | `case:DEF_USE_ISOTOPE` | `DEF_USE_ISOTOPE` | DEF USE ISOTOPE | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:42 |
| 38 | `case:DEF_USE_METHANE` | `DEF_USE_METHANE` | DEF USE METHANE | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:58 |
| 39 | `case:DEF_USE_NH4_LEACHING` | `DEF_USE_NH4_LEACHING` | DEF USE NH4 LEACHING | `.false.` |  | case-scalar | read-only-context | MOD_Namelist.F90:52 |
| 40 | `case:DEF_USE_SEDIMENT` | `DEF_USE_SEDIMENT` | DEF USE SEDIMENT | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:519 |
| 41 | `case:DEF_file_GIEMS` | `DEF_file_GIEMS` | DEF file GIEMS | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:533 |
| 42 | `case:DEF_wetland_finundation_scheme` | `DEF_wetland_finundation_scheme` | DEF wetland finundation scheme | `1` |  | case-scalar | read-only-context | MOD_Namelist.F90:534 |
| 43 | `process:material:DEF_ISO_CANOPY_EQUILIBRATION` | `DEF_ISO_CANOPY_EQUILIBRATION` | DEF ISO CANOPY EQUILIBRATION | `0.0` |  | process-file | editable-expert | MOD_MT_Namelist.F90:133 |
| 44 | `process:material:DEF_ISO_CG_RELHUM_MAX` | `DEF_ISO_CG_RELHUM_MAX` | DEF ISO CG RELHUM MAX | `0.99` |  | process-file | editable-expert | MOD_MT_Namelist.F90:127 |
| 45 | `process:material:DEF_ISO_ICE_SUPERSAT_SLOPE` | `DEF_ISO_ICE_SUPERSAT_SLOPE` | DEF ISO ICE SUPERSAT SLOPE | `0.003` |  | process-file | editable-expert | MOD_MT_Namelist.F90:126 |
| 46 | `process:material:DEF_ISO_INIT_DELTA_H2_18O` | `DEF_ISO_INIT_DELTA_H2_18O` | DEF ISO INIT DELTA H2 18O | `-10.` |  | process-file | editable-expert | MOD_MT_Namelist.F90:141 |
| 47 | `process:material:DEF_ISO_INIT_DELTA_HDO` | `DEF_ISO_INIT_DELTA_HDO` | DEF ISO INIT DELTA HDO | `-70.` |  | process-file | editable-expert | MOD_MT_Namelist.F90:143 |
| 48 | `process:material:DEF_ISO_KINETIC_SCHEME` | `DEF_ISO_KINETIC_SCHEME` | DEF ISO KINETIC SCHEME | `'MERLIVAT1978'` |  | process-file | editable-expert | MOD_MT_Namelist.F90:125 |
| 49 | `process:material:DEF_ISO_NSS_LEAF_PATH_LENGTH` | `DEF_ISO_NSS_LEAF_PATH_LENGTH` | DEF ISO NSS LEAF PATH LENGTH | `0.01` |  | process-file | editable-expert | MOD_MT_Namelist.F90:136 |
| 50 | `process:material:DEF_ISO_NSS_LEAF_RB` | `DEF_ISO_NSS_LEAF_RB` | DEF ISO NSS LEAF RB | `100.` |  | process-file | editable-expert | MOD_MT_Namelist.F90:137 |
| 51 | `process:material:DEF_ISO_NSS_LEAF_WATER_PER_LAI` | `DEF_ISO_NSS_LEAF_WATER_PER_LAI` | DEF ISO NSS LEAF WATER PER LAI | `0.12` |  | process-file | editable-expert | MOD_MT_Namelist.F90:135 |
| 52 | `process:material:DEF_ISO_OPEN_WATER_KINETIC` | `DEF_ISO_OPEN_WATER_KINETIC` | DEF ISO OPEN WATER KINETIC | `'MJ79'` |  | process-file | editable-expert | MOD_MT_Namelist.F90:128 |
| 53 | `process:material:DEF_ISO_REF_RATIO_H2_18O` | `DEF_ISO_REF_RATIO_H2_18O` | DEF ISO REF RATIO H2 18O | `2.0052e-3` |  | process-file | editable-expert | MOD_MT_Namelist.F90:140 |
| 54 | `process:material:DEF_ISO_REF_RATIO_HDO` | `DEF_ISO_REF_RATIO_HDO` | DEF ISO REF RATIO HDO | `1.5576e-4` |  | process-file | editable-expert | MOD_MT_Namelist.F90:142 |
| 55 | `process:material:DEF_ISO_SNOWMELT_EQUILIBRATION` | `DEF_ISO_SNOWMELT_EQUILIBRATION` | DEF ISO SNOWMELT EQUILIBRATION | `0.0` |  | process-file | editable-expert | MOD_MT_Namelist.F90:134 |
| 56 | `process:material:DEF_ISO_SOIL_DIFFUSION` | `DEF_ISO_SOIL_DIFFUSION` | DEF ISO SOIL DIFFUSION | `.true.` |  | process-file | editable-expert | MOD_MT_Namelist.F90:131 |
| 57 | `process:material:DEF_ISO_SOIL_KINETIC` | `DEF_ISO_SOIL_KINETIC` | DEF ISO SOIL KINETIC | `'RESISTANCE'` |  | process-file | editable-expert | MOD_MT_Namelist.F90:130 |
| 58 | `process:material:DEF_ISO_SOIL_VAPOR_DIFFUSION` | `DEF_ISO_SOIL_VAPOR_DIFFUSION` | DEF ISO SOIL VAPOR DIFFUSION | `.true.` |  | process-file | editable-expert | MOD_MT_Namelist.F90:132 |
| 59 | `process:material:DEF_ISO_SUBL_SKIN_MM` | `DEF_ISO_SUBL_SKIN_MM` | DEF ISO SUBL SKIN MM | `5.0` |  | process-file | editable-expert | MOD_MT_Namelist.F90:129 |
| 60 | `process:material:DEF_ISO_USE_FRACTIONATION` | `DEF_ISO_USE_FRACTIONATION` | DEF ISO USE FRACTIONATION | `.false.` |  | process-file | editable-expert | MOD_MT_Namelist.F90:124 |
| 61 | `process:material:DEF_SOLUTE%charge` | `DEF_SOLUTE%charge` | DEF SOLUTE charge | `0` |  | process-file | editable-expert | MOD_MT_Solute.F90:34 |
| 62 | `process:material:DEF_SOLUTE%decay_rate` | `DEF_SOLUTE%decay_rate` | DEF SOLUTE decay rate | `0.0` |  | process-file | editable-expert | MOD_MT_Solute.F90:30 |
| 63 | `process:material:DEF_SOLUTE%init_conc` | `DEF_SOLUTE%init_conc` | DEF SOLUTE init conc | `0.0` |  | process-file | editable-expert | MOD_MT_Solute.F90:26 |
| 64 | `process:material:DEF_SOLUTE%max_dissolved_conc` | `DEF_SOLUTE%max_dissolved_conc` | DEF SOLUTE max dissolved conc | `huge(1.0)` |  | process-file | editable-expert | MOD_MT_Solute.F90:29 |
| 65 | `process:material:DEF_SOLUTE%mol_weight` | `DEF_SOLUTE%mol_weight` | DEF SOLUTE mol weight | `0.0` |  | process-file | editable-expert | MOD_MT_Solute.F90:25 |
| 66 | `process:material:DEF_SOLUTE%n2o_ef5` | `DEF_SOLUTE%n2o_ef5` | DEF SOLUTE n2o ef5 | `0.0` |  | process-file | editable-expert | MOD_MT_Solute.F90:33 |
| 67 | `process:material:DEF_SOLUTE%precip_default_conc` | `DEF_SOLUTE%precip_default_conc` | DEF SOLUTE precip default conc | `0.0` |  | process-file | editable-expert | MOD_MT_Solute.F90:27 |
| 68 | `process:material:DEF_SOLUTE%unit_kind` | `DEF_SOLUTE%unit_kind` | DEF SOLUTE unit kind | `'material_per_water'` |  | process-file | editable-expert | MOD_MT_Solute.F90:24 |
| 69 | `process:material:DEF_SOLUTE%vapor_default_conc` | `DEF_SOLUTE%vapor_default_conc` | DEF SOLUTE vapor default conc | `0.0` |  | process-file | editable-expert | MOD_MT_Solute.F90:28 |
| 70 | `process:material:forcing_dtime` | `forcing_dtime` | forcing dtime | `21600` |  | process-file | editable-expert | MOD_MT_ForcingInput.F90:nl_colm_mt_forcing |
| 71 | `process:material:forcing_fprefix` | `forcing_fprefix` | forcing fprefix | `'null'` |  | process-file | editable-expert | MOD_MT_ForcingInput.F90:nl_colm_mt_forcing |
| 72 | `process:material:forcing_input_mode` | `forcing_input_mode` | forcing input mode | `'normalized_over_total'` |  | process-file | editable-expert | MOD_MT_ForcingInput.F90:nl_colm_mt_forcing |
| 73 | `process:material:forcing_num` | `forcing_num` | forcing num | `0` |  | process-file | editable-expert | MOD_MT_ForcingInput.F90:nl_colm_mt_forcing |
| 74 | `process:material:forcing_offset` | `forcing_offset` | forcing offset | `0` |  | process-file | editable-expert | MOD_MT_ForcingInput.F90:nl_colm_mt_forcing |
| 75 | `process:material:forcing_role` | `forcing_role` | forcing role | `'none'` |  | process-file | editable-expert | MOD_MT_ForcingInput.F90:nl_colm_mt_forcing |
| 76 | `process:material:forcing_tintalgo` | `forcing_tintalgo` | forcing tintalgo | `'linear'` |  | process-file | editable-expert | MOD_MT_ForcingInput.F90:nl_colm_mt_forcing |
| 77 | `process:material:forcing_vname` | `forcing_vname` | forcing vname | `'null'` |  | process-file | editable-expert | MOD_MT_ForcingInput.F90:nl_colm_mt_forcing |
| 78 | `process:methane:DEF_METHANE%B_init_methanogen` | `DEF_METHANE%B_init_methanogen` | DEF METHANE B init methanogen | `1.0` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:239 |
| 79 | `process:methane:DEF_METHANE%B_init_methanotroph` | `DEF_METHANE%B_init_methanotroph` | DEF METHANE B init methanotroph | `1.0` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:240 |
| 80 | `process:methane:DEF_METHANE%B_max_fraction_methanogen` | `DEF_METHANE%B_max_fraction_methanogen` | DEF METHANE B max fraction methanogen | `-1.0` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:246 |
| 81 | `process:methane:DEF_METHANE%B_max_fraction_methanotroph` | `DEF_METHANE%B_max_fraction_methanotroph` | DEF METHANE B max fraction methanotroph | `-1.0` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:247 |
| 82 | `process:methane:DEF_METHANE%B_min_methanogen` | `DEF_METHANE%B_min_methanogen` | DEF METHANE B min methanogen | `1.0e-3` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:241 |
| 83 | `process:methane:DEF_METHANE%B_min_methanotroph` | `DEF_METHANE%B_min_methanotroph` | DEF METHANE B min methanotroph | `1.0e-3` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:242 |
| 84 | `process:methane:DEF_METHANE%K_inh_O2_methanogen` | `DEF_METHANE%K_inh_O2_methanogen` | DEF METHANE K inh O2 methanogen | `1.0e-3` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:255 |
| 85 | `process:methane:DEF_METHANE%K_substrate_methanogen_pool` | `DEF_METHANE%K_substrate_methanogen_pool` | DEF METHANE K substrate methanogen pool | `0.04` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:254 |
| 86 | `process:methane:DEF_METHANE%T_ref_microbe` | `DEF_METHANE%T_ref_microbe` | DEF METHANE T ref microbe | `298.15` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:262 |
| 87 | `process:methane:DEF_METHANE%aere_radius` | `DEF_METHANE%aere_radius` | DEF METHANE aere radius | `2.9e-3` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:278 |
| 88 | `process:methane:DEF_METHANE%aereoxid` | `DEF_METHANE%aereoxid` | DEF METHANE aereoxid | `0.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:312 |
| 89 | `process:methane:DEF_METHANE%allowlakeprod` | `DEF_METHANE%allowlakeprod` | DEF METHANE allowlakeprod | `.false.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:337 |
| 90 | `process:methane:DEF_METHANE%anoxia` | `DEF_METHANE%anoxia` | DEF METHANE anoxia | `.true.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:406 |
| 91 | `process:methane:DEF_METHANE%anoxicmicrosites` | `DEF_METHANE%anoxicmicrosites` | DEF METHANE anoxicmicrosites | `.false.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:396 |
| 92 | `process:methane:DEF_METHANE%atm_methane` | `DEF_METHANE%atm_methane` | DEF METHANE atm methane | `1.7e-6` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:320 |
| 93 | `process:methane:DEF_METHANE%atm_methane_file` | `DEF_METHANE%atm_methane_file` | DEF METHANE atm methane file | `'null'` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:322 |
| 94 | `process:methane:DEF_METHANE%atm_methane_file_units` | `DEF_METHANE%atm_methane_file_units` | DEF METHANE atm methane file units | `'auto'` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:323 |
| 95 | `process:methane:DEF_METHANE%bgc_anoxia_limits_decomp` | `DEF_METHANE%bgc_anoxia_limits_decomp` | DEF METHANE bgc anoxia limits decomp | `.false.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:412 |
| 96 | `process:methane:DEF_METHANE%bubble_f` | `DEF_METHANE%bubble_f` | DEF METHANE bubble f | `0.57` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:309 |
| 97 | `process:methane:DEF_METHANE%capthick` | `DEF_METHANE%capthick` | DEF METHANE capthick | `100.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:317 |
| 98 | `process:methane:DEF_METHANE%ch4_history_vars` | `DEF_METHANE%ch4_history_vars` | DEF METHANE ch4 history vars | `'core'` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:426 |
| 99 | `process:methane:DEF_METHANE%cnscalefactor` | `DEF_METHANE%cnscalefactor` | DEF METHANE cnscalefactor | `1.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:296 |
| 100 | `process:methane:DEF_METHANE%dormancy_rate_active` | `DEF_METHANE%dormancy_rate_active` | DEF METHANE dormancy rate active | `0.1` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:263 |
| 101 | `process:methane:DEF_METHANE%dormancy_rate_revive` | `DEF_METHANE%dormancy_rate_revive` | DEF METHANE dormancy rate revive | `1.0` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:264 |
| 102 | `process:methane:DEF_METHANE%dormancy_threshold_methanogen_fO2` | `DEF_METHANE%dormancy_threshold_methanogen_fO2` | DEF METHANE dormancy threshold methanogen fO2 | `0.1` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:266 |
| 103 | `process:methane:DEF_METHANE%dormancy_threshold_methanogen_fS` | `DEF_METHANE%dormancy_threshold_methanogen_fS` | DEF METHANE dormancy threshold methanogen fS | `0.1` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:265 |
| 104 | `process:methane:DEF_METHANE%dormancy_threshold_methanotroph_fO2` | `DEF_METHANE%dormancy_threshold_methanotroph_fO2` | DEF METHANE dormancy threshold methanotroph fO2 | `0.1` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:268 |
| 105 | `process:methane:DEF_METHANE%dormancy_threshold_methanotroph_fS` | `DEF_METHANE%dormancy_threshold_methanotroph_fS` | DEF METHANE dormancy threshold methanotroph fS | `0.1` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:267 |
| 106 | `process:methane:DEF_METHANE%enable_rice_paddy` | `DEF_METHANE%enable_rice_paddy` | DEF METHANE enable rice paddy | `.false.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:152 |
| 107 | `process:methane:DEF_METHANE%enable_wetwat_finundated_override` | `DEF_METHANE%enable_wetwat_finundated_override` | DEF METHANE enable wetwat finundated override | `.false.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:467 |
| 108 | `process:methane:DEF_METHANE%f_methane` | `DEF_METHANE%f_methane` | DEF METHANE f methane | `0.2` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:159 |
| 109 | `process:methane:DEF_METHANE%f_methane_boreal_bog` | `DEF_METHANE%f_methane_boreal_bog` | DEF METHANE f methane boreal bog | `0.08` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:178 |
| 110 | `process:methane:DEF_METHANE%f_methane_boreal_fen` | `DEF_METHANE%f_methane_boreal_fen` | DEF METHANE f methane boreal fen | `0.12` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:177 |
| 111 | `process:methane:DEF_METHANE%f_methane_floodplain` | `DEF_METHANE%f_methane_floodplain` | DEF METHANE f methane floodplain | `0.10` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:175 |
| 112 | `process:methane:DEF_METHANE%f_methane_rice_paddy` | `DEF_METHANE%f_methane_rice_paddy` | DEF METHANE f methane rice paddy | `0.10` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:179 |
| 113 | `process:methane:DEF_METHANE%f_methane_temperate_marsh` | `DEF_METHANE%f_methane_temperate_marsh` | DEF METHANE f methane temperate marsh | `0.15` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:176 |
| 114 | `process:methane:DEF_METHANE%f_methane_tropical_floodplain` | `DEF_METHANE%f_methane_tropical_floodplain` | DEF METHANE f methane tropical floodplain | `0.07` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:174 |
| 115 | `process:methane:DEF_METHANE%f_methane_tropical_peat` | `DEF_METHANE%f_methane_tropical_peat` | DEF METHANE f methane tropical peat | `0.12` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:173 |
| 116 | `process:methane:DEF_METHANE%f_methane_upland_soil` | `DEF_METHANE%f_methane_upland_soil` | DEF METHANE f methane upland soil | `0.05` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:180 |
| 117 | `process:methane:DEF_METHANE%gamma_methanogen` | `DEF_METHANE%gamma_methanogen` | DEF METHANE gamma methanogen | `0.05` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:250 |
| 118 | `process:methane:DEF_METHANE%gamma_methanotroph` | `DEF_METHANE%gamma_methanotroph` | DEF METHANE gamma methanotroph | `0.10` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:251 |
| 119 | `process:methane:DEF_METHANE%gamma_microbial_dormant` | `DEF_METHANE%gamma_microbial_dormant` | DEF METHANE gamma microbial dormant | `0.005` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:252 |
| 120 | `process:methane:DEF_METHANE%gamma_microbial_freeze` | `DEF_METHANE%gamma_microbial_freeze` | DEF METHANE gamma microbial freeze | `0.001` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:253 |
| 121 | `process:methane:DEF_METHANE%grnd_methane_cond_default` | `DEF_METHANE%grnd_methane_cond_default` | DEF METHANE grnd methane cond default | `1.e-6` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:289 |
| 122 | `process:methane:DEF_METHANE%host_water_tolerance` | `DEF_METHANE%host_water_tolerance` | DEF METHANE host water tolerance | `0.05` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:449 |
| 123 | `process:methane:DEF_METHANE%hybrid_soil_threshold` | `DEF_METHANE%hybrid_soil_threshold` | DEF METHANE hybrid soil threshold | `0.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:381 |
| 124 | `process:methane:DEF_METHANE%inundation_mode` | `DEF_METHANE%inundation_mode` | DEF METHANE inundation mode | `'hybrid'` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:151 |
| 125 | `process:methane:DEF_METHANE%k_m` | `DEF_METHANE%k_m` | DEF METHANE k m | `5.e-3` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:222 |
| 126 | `process:methane:DEF_METHANE%k_m_o2` | `DEF_METHANE%k_m_o2` | DEF METHANE k m o2 | `2.e-2` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:224 |
| 127 | `process:methane:DEF_METHANE%k_m_unsat` | `DEF_METHANE%k_m_unsat` | DEF METHANE k m unsat | `5.e-4` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:223 |
| 128 | `process:methane:DEF_METHANE%kappa_m_methanogen` | `DEF_METHANE%kappa_m_methanogen` | DEF METHANE kappa m methanogen | `1.0e-2` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:256 |
| 129 | `process:methane:DEF_METHANE%kappa_m_methanotroph` | `DEF_METHANE%kappa_m_methanotroph` | DEF METHANE kappa m methanotroph | `1.0e-2` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:257 |
| 130 | `process:methane:DEF_METHANE%lake_decomp_fact` | `DEF_METHANE%lake_decomp_fact` | DEF METHANE lake decomp fact | `9.e-11` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:299 |
| 131 | `process:methane:DEF_METHANE%lake_k_m_o2` | `DEF_METHANE%lake_k_m_o2` | DEF METHANE lake k m o2 | `-1.0` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:230 |
| 132 | `process:methane:DEF_METHANE%lake_liqdiff_scale` | `DEF_METHANE%lake_liqdiff_scale` | DEF METHANE lake liqdiff scale | `1.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:285 |
| 133 | `process:methane:DEF_METHANE%lake_o2_liqdiff_scale` | `DEF_METHANE%lake_o2_liqdiff_scale` | DEF METHANE lake o2 liqdiff scale | `1.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:286 |
| 134 | `process:methane:DEF_METHANE%lake_oxic_sediment_depth` | `DEF_METHANE%lake_oxic_sediment_depth` | DEF METHANE lake oxic sediment depth | `-1.0` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:232 |
| 135 | `process:methane:DEF_METHANE%lake_oxid_scale` | `DEF_METHANE%lake_oxid_scale` | DEF METHANE lake oxid scale | `1.0` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:229 |
| 136 | `process:methane:DEF_METHANE%lake_restart_debug` | `DEF_METHANE%lake_restart_debug` | DEF METHANE lake restart debug | `.false.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:458 |
| 137 | `process:methane:DEF_METHANE%lake_restart_debug_end_doy` | `DEF_METHANE%lake_restart_debug_end_doy` | DEF METHANE lake restart debug end doy | `3` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:461 |
| 138 | `process:methane:DEF_METHANE%lake_restart_debug_sec` | `DEF_METHANE%lake_restart_debug_sec` | DEF METHANE lake restart debug sec | `1800` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:462 |
| 139 | `process:methane:DEF_METHANE%lake_restart_debug_start_doy` | `DEF_METHANE%lake_restart_debug_start_doy` | DEF METHANE lake restart debug start doy | `1` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:460 |
| 140 | `process:methane:DEF_METHANE%lake_restart_debug_year` | `DEF_METHANE%lake_restart_debug_year` | DEF METHANE lake restart debug year | `1996` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:459 |
| 141 | `process:methane:DEF_METHANE%lake_vmax_methane_oxid` | `DEF_METHANE%lake_vmax_methane_oxid` | DEF METHANE lake vmax methane oxid | `-1.0` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:231 |
| 142 | `process:methane:DEF_METHANE%lake_zero_depth_fatal` | `DEF_METHANE%lake_zero_depth_fatal` | DEF METHANE lake zero depth fatal | `.false.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:453 |
| 143 | `process:methane:DEF_METHANE%max_microbe_prod_multiplier` | `DEF_METHANE%max_microbe_prod_multiplier` | DEF METHANE max microbe prod multiplier | `3.0` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:260 |
| 144 | `process:methane:DEF_METHANE%methane_frzout` | `DEF_METHANE%methane_frzout` | DEF METHANE methane frzout | `.false.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:399 |
| 145 | `process:methane:DEF_METHANE%methane_offline` | `DEF_METHANE%methane_offline` | DEF METHANE methane offline | `.true.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:387 |
| 146 | `process:methane:DEF_METHANE%methane_rmcnlim` | `DEF_METHANE%methane_rmcnlim` | DEF METHANE methane rmcnlim | `.false.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:391 |
| 147 | `process:methane:DEF_METHANE%mino2lim` | `DEF_METHANE%mino2lim` | DEF METHANE mino2lim | `0.2` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:293 |
| 148 | `process:methane:DEF_METHANE%mu_max_methanogen` | `DEF_METHANE%mu_max_methanogen` | DEF METHANE mu max methanogen | `0.2` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:248 |
| 149 | `process:methane:DEF_METHANE%mu_max_methanotroph` | `DEF_METHANE%mu_max_methanotroph` | DEF METHANE mu max methanotroph | `0.5` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:249 |
| 150 | `process:methane:DEF_METHANE%nongrassporosratio` | `DEF_METHANE%nongrassporosratio` | DEF METHANE nongrassporosratio | `0.333333333333` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:274 |
| 151 | `process:methane:DEF_METHANE%numerical_correction_fatal_threshold` | `DEF_METHANE%numerical_correction_fatal_threshold` | DEF METHANE numerical correction fatal threshold | `-1.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:430 |
| 152 | `process:methane:DEF_METHANE%om_frac_sf` | `DEF_METHANE%om_frac_sf` | DEF METHANE om frac sf | `1.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:324 |
| 153 | `process:methane:DEF_METHANE%only_wetland` | `DEF_METHANE%only_wetland` | DEF METHANE only wetland | `.false.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:153 |
| 154 | `process:methane:DEF_METHANE%oxinhib` | `DEF_METHANE%oxinhib` | DEF METHANE oxinhib | `400.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:303 |
| 155 | `process:methane:DEF_METHANE%pHmax` | `DEF_METHANE%pHmax` | DEF METHANE pHmax | `9.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:301 |
| 156 | `process:methane:DEF_METHANE%pHmin` | `DEF_METHANE%pHmin` | DEF METHANE pHmin | `2.2` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:302 |
| 157 | `process:methane:DEF_METHANE%poros_tiller` | `DEF_METHANE%poros_tiller` | DEF METHANE poros tiller | `0.3` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:275 |
| 158 | `process:methane:DEF_METHANE%porosmin` | `DEF_METHANE%porosmin` | DEF METHANE porosmin | `0.05` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:277 |
| 159 | `process:methane:DEF_METHANE%q10_methane_oxid` | `DEF_METHANE%q10_methane_oxid` | DEF METHANE q10 methane oxid | `1.9` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:225 |
| 160 | `process:methane:DEF_METHANE%q10_microbe_growth` | `DEF_METHANE%q10_microbe_growth` | DEF METHANE q10 microbe growth | `3.0` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:261 |
| 161 | `process:methane:DEF_METHANE%q10lakebase` | `DEF_METHANE%q10lakebase` | DEF METHANE q10lakebase | `298.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:295 |
| 162 | `process:methane:DEF_METHANE%q10methane` | `DEF_METHANE%q10methane` | DEF METHANE q10methane | `2.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:158 |
| 163 | `process:methane:DEF_METHANE%q10methane_base` | `DEF_METHANE%q10methane_base` | DEF METHANE q10methane base | `295.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:294 |
| 164 | `process:methane:DEF_METHANE%redoxlag` | `DEF_METHANE%redoxlag` | DEF METHANE redoxlag | `30.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:298 |
| 165 | `process:methane:DEF_METHANE%redoxlag_boreal_bog` | `DEF_METHANE%redoxlag_boreal_bog` | DEF METHANE redoxlag boreal bog | `45.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:198 |
| 166 | `process:methane:DEF_METHANE%redoxlag_boreal_fen` | `DEF_METHANE%redoxlag_boreal_fen` | DEF METHANE redoxlag boreal fen | `30.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:197 |
| 167 | `process:methane:DEF_METHANE%redoxlag_rice_paddy` | `DEF_METHANE%redoxlag_rice_paddy` | DEF METHANE redoxlag rice paddy | `25.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:199 |
| 168 | `process:methane:DEF_METHANE%redoxlag_temperate_marsh` | `DEF_METHANE%redoxlag_temperate_marsh` | DEF METHANE redoxlag temperate marsh | `20.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:196 |
| 169 | `process:methane:DEF_METHANE%redoxlag_tropical_floodplain` | `DEF_METHANE%redoxlag_tropical_floodplain` | DEF METHANE redoxlag tropical floodplain | `7.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:195 |
| 170 | `process:methane:DEF_METHANE%redoxlag_tropical_peat` | `DEF_METHANE%redoxlag_tropical_peat` | DEF METHANE redoxlag tropical peat | `12.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:194 |
| 171 | `process:methane:DEF_METHANE%redoxlag_upland_soil` | `DEF_METHANE%redoxlag_upland_soil` | DEF METHANE redoxlag upland soil | `30.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:200 |
| 172 | `process:methane:DEF_METHANE%redoxlag_vertical` | `DEF_METHANE%redoxlag_vertical` | DEF METHANE redoxlag vertical | `30.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:300 |
| 173 | `process:methane:DEF_METHANE%replenishlakec` | `DEF_METHANE%replenishlakec` | DEF METHANE replenishlakec | `.false.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:383 |
| 174 | `process:methane:DEF_METHANE%rice_drain_window_days` | `DEF_METHANE%rice_drain_window_days` | DEF METHANE rice drain window days | `30.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:476 |
| 175 | `process:methane:DEF_METHANE%rice_substrate_boost` | `DEF_METHANE%rice_substrate_boost` | DEF METHANE rice substrate boost | `1.0` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:491 |
| 176 | `process:methane:DEF_METHANE%rob` | `DEF_METHANE%rob` | DEF METHANE rob | `3.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:279 |
| 177 | `process:methane:DEF_METHANE%satpow` | `DEF_METHANE%satpow` | DEF METHANE satpow | `2.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:316 |
| 178 | `process:methane:DEF_METHANE%scale_factor_aere` | `DEF_METHANE%scale_factor_aere` | DEF METHANE scale factor aere | `1.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:280 |
| 179 | `process:methane:DEF_METHANE%scale_factor_gasdiff` | `DEF_METHANE%scale_factor_gasdiff` | DEF METHANE scale factor gasdiff | `1.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:283 |
| 180 | `process:methane:DEF_METHANE%scale_factor_liqdiff` | `DEF_METHANE%scale_factor_liqdiff` | DEF METHANE scale factor liqdiff | `1.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:284 |
| 181 | `process:methane:DEF_METHANE%smp_crit` | `DEF_METHANE%smp_crit` | DEF METHANE smp crit | `-2.4e5` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:306 |
| 182 | `process:methane:DEF_METHANE%tiller_C` | `DEF_METHANE%tiller_C` | DEF METHANE tiller C | `0.22` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:313 |
| 183 | `process:methane:DEF_METHANE%transpirationloss` | `DEF_METHANE%transpirationloss` | DEF METHANE transpirationloss | `.true.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:331 |
| 184 | `process:methane:DEF_METHANE%unsat_aere_ratio` | `DEF_METHANE%unsat_aere_ratio` | DEF METHANE unsat aere ratio | `0.166666666667` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:276 |
| 185 | `process:methane:DEF_METHANE%use_aereoxid_prog` | `DEF_METHANE%use_aereoxid_prog` | DEF METHANE use aereoxid prog | `.true.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:327 |
| 186 | `process:methane:DEF_METHANE%use_biome_f_methane` | `DEF_METHANE%use_biome_f_methane` | DEF METHANE use biome f methane | `.false.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:172 |
| 187 | `process:methane:DEF_METHANE%use_biome_redoxlag` | `DEF_METHANE%use_biome_redoxlag` | DEF METHANE use biome redoxlag | `.false.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:193 |
| 188 | `process:methane:DEF_METHANE%use_ch4_sif` | `DEF_METHANE%use_ch4_sif` | DEF METHANE use ch4 sif | `.true.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:413 |
| 189 | `process:methane:DEF_METHANE%use_microbial_dormancy` | `DEF_METHANE%use_microbial_dormancy` | DEF METHANE use microbial dormancy | `.false.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:238 |
| 190 | `process:methane:DEF_METHANE%use_microbial_flux_override` | `DEF_METHANE%use_microbial_flux_override` | DEF METHANE use microbial flux override | `.false.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:237 |
| 191 | `process:methane:DEF_METHANE%use_microbial_pools` | `DEF_METHANE%use_microbial_pools` | DEF METHANE use microbial pools | `.false.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:236 |
| 192 | `process:methane:DEF_METHANE%use_nitrif_denitrif` | `DEF_METHANE%use_nitrif_denitrif` | DEF METHANE use nitrif denitrif | `.true.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:404 |
| 193 | `process:methane:DEF_METHANE%use_routing_for_soil` | `DEF_METHANE%use_routing_for_soil` | DEF METHANE use routing for soil | `.false.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:368 |
| 194 | `process:methane:DEF_METHANE%use_spatial_ph` | `DEF_METHANE%use_spatial_ph` | DEF METHANE use spatial ph | `.false.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:154 |
| 195 | `process:methane:DEF_METHANE%use_transient_atm_methane` | `DEF_METHANE%use_transient_atm_methane` | DEF METHANE use transient atm methane | `.false.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:321 |
| 196 | `process:methane:DEF_METHANE%use_vertical_redoxlag` | `DEF_METHANE%use_vertical_redoxlag` | DEF METHANE use vertical redoxlag | `.true.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:409 |
| 197 | `process:methane:DEF_METHANE%usephfact` | `DEF_METHANE%usephfact` | DEF METHANE usephfact | `.true.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:343 |
| 198 | `process:methane:DEF_METHANE%vgc_max` | `DEF_METHANE%vgc_max` | DEF METHANE vgc max | `0.15` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:271 |
| 199 | `process:methane:DEF_METHANE%vmax_methane_oxid` | `DEF_METHANE%vmax_methane_oxid` | DEF METHANE vmax methane oxid | `1.25e-5` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:220 |
| 200 | `process:methane:DEF_METHANE%vmax_oxid_unsat` | `DEF_METHANE%vmax_oxid_unsat` | DEF METHANE vmax oxid unsat | `1.25e-6` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:221 |
| 201 | `process:methane:DEF_METHANE%wetland_dry_unsat_branch` | `DEF_METHANE%wetland_dry_unsat_branch` | DEF METHANE wetland dry unsat branch | `.false.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:472 |
| 202 | `process:methane:DEF_METHANE%write_ch4_history` | `DEF_METHANE%write_ch4_history` | DEF METHANE write ch4 history | `.true.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:425 |
| 203 | `process:methane:DEF_METHANE%wtd_inflection` | `DEF_METHANE%wtd_inflection` | DEF METHANE wtd inflection | `0.30` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:352 |
| 204 | `process:methane:DEF_METHANE%wtd_inflection_soil` | `DEF_METHANE%wtd_inflection_soil` | DEF METHANE wtd inflection soil | `0.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:359 |
| 205 | `process:methane:DEF_METHANE%wtd_steepness` | `DEF_METHANE%wtd_steepness` | DEF METHANE wtd steepness | `0.05` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:353 |
| 206 | `process:methane:DEF_METHANE%wtd_steepness_soil` | `DEF_METHANE%wtd_steepness_soil` | DEF METHANE wtd steepness soil | `0.3` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:360 |
| 207 | `process:methane:DEF_METHANE%z0_methane_prod` | `DEF_METHANE%z0_methane_prod` | DEF METHANE z0 methane prod | `0.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:215 |
| 208 | `process:methane:DEF_METHANE_hydrology%pc` | `DEF_METHANE_hydrology%pc` | DEF METHANE hydrology pc | `0.4` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:501 |
| 209 | `process:methane:DEF_METHANE_hydrology%slopebeta` | `DEF_METHANE_hydrology%slopebeta` | DEF METHANE hydrology slopebeta | `-3.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:499 |
| 210 | `process:methane:DEF_METHANE_hydrology%slopemax` | `DEF_METHANE_hydrology%slopemax` | DEF METHANE hydrology slopemax | `0.4` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:500 |
| 211 | `process:methane:DEF_METHANE_hydrology%vdcf` | `DEF_METHANE_hydrology%vdcf` | DEF METHANE hydrology vdcf | `2.` |  | process-file | editable-expert | MOD_MT_Methane_Const.F90:498 |
| 212 | `process:sediment:DEF_SEDIMENT%active_layer_depth` | `DEF_SEDIMENT%active_layer_depth` | DEF SEDIMENT active layer depth | `-1.` |  | process-file | editable-expert | MOD_MT_Sediment.F90:216 |
| 213 | `process:sediment:DEF_SEDIMENT%bed_depth` | `DEF_SEDIMENT%bed_depth` | DEF SEDIMENT bed depth | `-1.` |  | process-file | editable-expert | MOD_MT_Sediment.F90:226 |
| 214 | `process:sediment:DEF_SEDIMENT%cfl_adv` | `DEF_SEDIMENT%cfl_adv` | DEF SEDIMENT cfl adv | `-1.` |  | process-file | editable-expert | MOD_MT_Sediment.F90:224 |
| 215 | `process:sediment:DEF_SEDIMENT%grain_density` | `DEF_SEDIMENT%grain_density` | DEF SEDIMENT grain density | `-1.` |  | process-file | editable-expert | MOD_MT_Sediment.F90:211 |
| 216 | `process:sediment:DEF_SEDIMENT%grain_diameter` | `DEF_SEDIMENT%grain_diameter` | DEF SEDIMENT grain diameter | `-1.` |  | process-file | editable-expert | MOD_MT_Sediment.F90:210 |
| 217 | `process:sediment:DEF_SEDIMENT%ignore_depth_m` | `DEF_SEDIMENT%ignore_depth_m` | DEF SEDIMENT ignore depth m | `-1.` |  | process-file | editable-expert | MOD_MT_Sediment.F90:215 |
| 218 | `process:sediment:DEF_SEDIMENT%max_timestep_s` | `DEF_SEDIMENT%max_timestep_s` | DEF SEDIMENT max timestep s | `-1.` |  | process-file | editable-expert | MOD_MT_Sediment.F90:225 |
| 219 | `process:sediment:DEF_SEDIMENT%ndeposit_layers` | `DEF_SEDIMENT%ndeposit_layers` | DEF SEDIMENT ndeposit layers | `-1` |  | process-file | editable-expert | MOD_MT_Sediment.F90:214 |
| 220 | `process:sediment:DEF_SEDIMENT%nsed` | `DEF_SEDIMENT%nsed` | DEF SEDIMENT nsed | `-1` |  | process-file | editable-expert | MOD_MT_Sediment.F90:209 |
| 221 | `process:sediment:DEF_SEDIMENT%porosity` | `DEF_SEDIMENT%porosity` | DEF SEDIMENT porosity | `-1.` |  | process-file | editable-expert | MOD_MT_Sediment.F90:213 |
| 222 | `process:sediment:DEF_SEDIMENT%precipitation_exponent` | `DEF_SEDIMENT%precipitation_exponent` | DEF SEDIMENT precipitation exponent | `-1.` |  | process-file | editable-expert | MOD_MT_Sediment.F90:222 |
| 223 | `process:sediment:DEF_SEDIMENT%settling_multiplier` | `DEF_SEDIMENT%settling_multiplier` | DEF SEDIMENT settling multiplier | `-1.` |  | process-file | editable-expert | MOD_MT_Sediment.F90:219 |
| 224 | `process:sediment:DEF_SEDIMENT%slope_exponent` | `DEF_SEDIMENT%slope_exponent` | DEF SEDIMENT slope exponent | `-1.` |  | process-file | editable-expert | MOD_MT_Sediment.F90:221 |
| 225 | `process:sediment:DEF_SEDIMENT%unit_conversion` | `DEF_SEDIMENT%unit_conversion` | DEF SEDIMENT unit conversion | `-1.` |  | process-file | editable-expert | MOD_MT_Sediment.F90:223 |
| 226 | `process:sediment:DEF_SEDIMENT%viscosity` | `DEF_SEDIMENT%viscosity` | DEF SEDIMENT viscosity | `-1.` |  | process-file | editable-expert | MOD_MT_Sediment.F90:217 |
| 227 | `process:sediment:DEF_SEDIMENT%von_karman` | `DEF_SEDIMENT%von_karman` | DEF SEDIMENT von karman | `-1.` |  | process-file | editable-expert | MOD_MT_Sediment.F90:218 |
| 228 | `process:sediment:DEF_SEDIMENT%water_density` | `DEF_SEDIMENT%water_density` | DEF SEDIMENT water density | `-1.` |  | process-file | editable-expert | MOD_MT_Sediment.F90:212 |
| 229 | `process:sediment:DEF_SEDIMENT%yield_coefficient` | `DEF_SEDIMENT%yield_coefficient` | DEF SEDIMENT yield coefficient | `-1.` |  | process-file | editable-expert | MOD_MT_Sediment.F90:220 |

## 水热过程（36 项）

| # | 参数 ID | 原始键 | 中文名 | 默认值 | 单位 | 作用域 | 可见性 | 源码位置 |
|---|---|---|---|---|---|---|---|---|
| 1 | `case:DEF_CheckEquilibrium` | `DEF_CheckEquilibrium` | DEF CheckEquilibrium | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:712 |
| 2 | `case:DEF_Interception_scheme` | `DEF_Interception_scheme` | DEF Interception scheme | `1` |  | case-scalar | editable-common | MOD_Namelist.F90:366 |
| 3 | `case:DEF_Optimize_Baseflow` | `DEF_Optimize_Baseflow` | DEF Optimize Baseflow | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:817 |
| 4 | `case:DEF_RSS_SCHEME` | `DEF_RSS_SCHEME` | DEF RSS SCHEME | `1` |  | case-scalar | editable-common | MOD_Namelist.F90:389 |
| 5 | `case:DEF_RS_CROWN_STRUCTURE` | `DEF_RS_CROWN_STRUCTURE` | DEF RS CROWN STRUCTURE | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:274 |
| 6 | `case:DEF_Runoff_SCHEME` | `DEF_Runoff_SCHEME` | DEF Runoff SCHEME | `3` |  | case-scalar | editable-common | MOD_Namelist.F90:397 |
| 7 | `case:DEF_SPLIT_SOILSNOW` | `DEF_SPLIT_SOILSNOW` | DEF SPLIT SOILSNOW | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:408 |
| 8 | `case:DEF_THERMAL_CONDUCTIVITY_SCHEME` | `DEF_THERMAL_CONDUCTIVITY_SCHEME` | DEF THERMAL CONDUCTIVITY SCHEME | `4` |  | case-scalar | editable-common | MOD_Namelist.F90:369 |
| 9 | `case:DEF_TOPMOD_method` | `DEF_TOPMOD_method` | DEF TOPMOD method | `0` |  | case-scalar | editable-common | MOD_Namelist.F90:402 |
| 10 | `case:DEF_TUNING_CAPR` | `DEF_TUNING_CAPR` | DEF TUNING CAPR | `0.34_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:630 |
| 11 | `case:DEF_TUNING_CNFAC` | `DEF_TUNING_CNFAC` | DEF TUNING CNFAC | `0.5_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:631 |
| 12 | `case:DEF_TUNING_PONDMX` | `DEF_TUNING_PONDMX` | DEF TUNING PONDMX | `10.0_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:634 |
| 13 | `case:DEF_TUNING_SIMPLE_VIC_DS` | `DEF_TUNING_SIMPLE_VIC_DS` | DEF TUNING SIMPLE VIC DS | `0.061_r8` |  | case-scalar | editable-common | MOD_Namelist.F90:643 |
| 14 | `case:DEF_TUNING_SIMPLE_VIC_WS` | `DEF_TUNING_SIMPLE_VIC_WS` | DEF TUNING SIMPLE VIC WS | `0.646_r8` |  | case-scalar | editable-common | MOD_Namelist.F90:644 |
| 15 | `case:DEF_TUNING_SMPMAX` | `DEF_TUNING_SMPMAX` | DEF TUNING SMPMAX | `-1.5e5_r8` |  | case-scalar | editable-common | MOD_Namelist.F90:635 |
| 16 | `case:DEF_TUNING_SMPMAX_HR` | `DEF_TUNING_SMPMAX_HR` | DEF TUNING SMPMAX HR | `-2.e2_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:637 |
| 17 | `case:DEF_TUNING_SMPMIN` | `DEF_TUNING_SMPMIN` | DEF TUNING SMPMIN | `-1.e8_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:636 |
| 18 | `case:DEF_TUNING_SMPMIN_HR` | `DEF_TUNING_SMPMIN_HR` | DEF TUNING SMPMIN HR | `-2.e5_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:638 |
| 19 | `case:DEF_TUNING_SNOW_COVER_EXPONENT` | `DEF_TUNING_SNOW_COVER_EXPONENT` | DEF TUNING SNOW COVER EXPONENT | `1.0_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:645 |
| 20 | `case:DEF_TUNING_SOIL_ICE_IMPEDANCE` | `DEF_TUNING_SOIL_ICE_IMPEDANCE` | DEF TUNING SOIL ICE IMPEDANCE | `6.0_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:641 |
| 21 | `case:DEF_TUNING_SSI` | `DEF_TUNING_SSI` | DEF TUNING SSI | `0.033_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:632 |
| 22 | `case:DEF_TUNING_TOPMOD_DECAY` | `DEF_TUNING_TOPMOD_DECAY` | DEF TUNING TOPMOD DECAY | `2.0_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:642 |
| 23 | `case:DEF_TUNING_WETWATMAX` | `DEF_TUNING_WETWATMAX` | DEF TUNING WETWATMAX | `200.0_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:640 |
| 24 | `case:DEF_TUNING_WIMP` | `DEF_TUNING_WIMP` | DEF TUNING WIMP | `0.05_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:633 |
| 25 | `case:DEF_TUNING_ZLND` | `DEF_TUNING_ZLND` | DEF TUNING ZLND | `0.01_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:626 |
| 26 | `case:DEF_TUNING_ZSNO` | `DEF_TUNING_ZSNO` | DEF TUNING ZSNO | `0.0024_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:627 |
| 27 | `case:DEF_USE_BEDROCK` | `DEF_USE_BEDROCK` | DEF USE BEDROCK | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:417 |
| 28 | `case:DEF_USE_Campbell_SOIL_MODEL` | `DEF_USE_Campbell_SOIL_MODEL` | DEF USE Campbell SOIL MODEL | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:429 |
| 29 | `case:DEF_USE_Dynamic_Lake` | `DEF_USE_Dynamic_Lake` | DEF USE Dynamic Lake | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:708 |
| 30 | `case:DEF_USE_Dynamic_Wetland` | `DEF_USE_Dynamic_Wetland` | DEF USE Dynamic Wetland | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:710 |
| 31 | `case:DEF_USE_SUPERCOOL_WATER` | `DEF_USE_SUPERCOOL_WATER` | DEF USE SUPERCOOL WATER | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:379 |
| 32 | `case:DEF_USE_VariablySaturatedFlow` | `DEF_USE_VariablySaturatedFlow` | DEF USE VariablySaturatedFlow | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:416 |
| 33 | `case:DEF_VIC_OPT` | `DEF_VIC_OPT` | DEF VIC OPT | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:400 |
| 34 | `case:DEF_file_VIC_OPT` | `DEF_file_VIC_OPT` | DEF file VIC OPT | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:399 |
| 35 | `case:DEF_file_VIC_para` | `DEF_file_VIC_para` | DEF file VIC para | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:398 |
| 36 | `case:DEF_precip_phase_discrimination_scheme` | `DEF_precip_phase_discrimination_scheme` | DEF precip phase discrimination scheme | `I` |  | case-scalar | editable-common | MOD_Namelist.F90:537 |

## 河道与水库（18 项）

| # | 参数 ID | 原始键 | 中文名 | 默认值 | 单位 | 作用域 | 可见性 | 源码位置 |
|---|---|---|---|---|---|---|---|---|
| 1 | `case:DEF_CaMa_FloodFeedback` | `DEF_CaMa_FloodFeedback` | DEF CaMa FloodFeedback | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:488 |
| 2 | `case:DEF_CaMa_Namelist` | `DEF_CaMa_Namelist` | DEF CaMa Namelist | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:486 |
| 3 | `case:DEF_CaMa_Restart_file` | `DEF_CaMa_Restart_file` | DEF CaMa Restart file | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:487 |
| 4 | `case:DEF_CaMa_StrictDomain` | `DEF_CaMa_StrictDomain` | DEF CaMa StrictDomain | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:489 |
| 5 | `case:DEF_ElementNeighbour_file` | `DEF_ElementNeighbour_file` | DEF ElementNeighbour file | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:496 |
| 6 | `case:DEF_GRIDBASED_ROUTING_MAX_DT` | `DEF_GRIDBASED_ROUTING_MAX_DT` | DEF GRIDBASED ROUTING MAX DT | `3600.` |  | case-scalar | editable-expert | MOD_Namelist.F90:505 |
| 7 | `case:DEF_GRIDBASED_ROUTING_MOMENTUM_DT_LIMIT` | `DEF_GRIDBASED_ROUTING_MOMENTUM_DT_LIMIT` | DEF GRIDBASED ROUTING MOMENTUM DT LIMIT | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:506 |
| 8 | `case:DEF_GRIDBASED_ROUTING_STORAGE_DT_LIMIT` | `DEF_GRIDBASED_ROUTING_STORAGE_DT_LIMIT` | DEF GRIDBASED ROUTING STORAGE DT LIMIT | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:510 |
| 9 | `case:DEF_GridRiverLake_FloodFeedback` | `DEF_GridRiverLake_FloodFeedback` | DEF GridRiverLake FloodFeedback | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:511 |
| 10 | `case:DEF_GridRiverLake_FloodInfiltMax` | `DEF_GridRiverLake_FloodInfiltMax` | DEF GridRiverLake FloodInfiltMax | `5._r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:512 |
| 11 | `case:DEF_ReservoirPara_file` | `DEF_ReservoirPara_file` | DEF ReservoirPara file | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:500 |
| 12 | `case:DEF_Reservoir_Method` | `DEF_Reservoir_Method` | DEF Reservoir Method | `0` |  | case-scalar | editable-common | MOD_Namelist.F90:504 |
| 13 | `case:DEF_USE_BIFURCATION` | `DEF_USE_BIFURCATION` | DEF USE BIFURCATION | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:514 |
| 14 | `case:DEF_USE_EstimatedRiverDepth` | `DEF_USE_EstimatedRiverDepth` | DEF USE EstimatedRiverDepth | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:502 |
| 15 | `case:DEF_USE_GridRiverLakeFlow` | `DEF_USE_GridRiverLakeFlow` | DEF USE GridRiverLakeFlow | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:495 |
| 16 | `case:DEF_USE_LEVEE` | `DEF_USE_LEVEE` | DEF USE LEVEE | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:513 |
| 17 | `case:DEF_UnitCatchment_file` | `DEF_UnitCatchment_file` | DEF UnitCatchment file | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:497 |
| 18 | `case:DEF_UnitCatchment_regional` | `DEF_UnitCatchment_regional` | DEF UnitCatchment regional | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:498 |

## 生态与生地化（318 项）

| # | 参数 ID | 原始键 | 中文名 | 默认值 | 单位 | 作用域 | 可见性 | 源码位置 |
|---|---|---|---|---|---|---|---|---|
| 1 | `case:DEF_Aerosol_Clim` | `DEF_Aerosol_Clim` | DEF Aerosol Clim | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:477 |
| 2 | `case:DEF_Aerosol_Readin` | `DEF_Aerosol_Readin` | DEF Aerosol Readin | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:474 |
| 3 | `case:DEF_BALL_BERRY_BINTER` | `DEF_BALL_BERRY_BINTER` | DEF BALL BERRY BINTER | `-1.0_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:552 |
| 4 | `case:DEF_BALL_BERRY_GRADM` | `DEF_BALL_BERRY_GRADM` | DEF BALL BERRY GRADM | `-1.0_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:551 |
| 5 | `case:DEF_FERT_SOURCE` | `DEF_FERT_SOURCE` | DEF FERT SOURCE | `1` |  | case-scalar | editable-common | MOD_Namelist.F90:702 |
| 6 | `case:DEF_IRRIGATION_ALLOCATION` | `DEF_IRRIGATION_ALLOCATION` | DEF IRRIGATION ALLOCATION | `1` |  | case-scalar | editable-common | MOD_Namelist.F90:542 |
| 7 | `case:DEF_LC_YEAR` | `DEF_LC_YEAR` | DEF LC YEAR | `2005` |  | case-scalar | editable-common | MOD_Namelist.F90:249 |
| 8 | `case:DEF_MEDLYN_G0` | `DEF_MEDLYN_G0` | DEF MEDLYN G0 | `-1.0_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:554 |
| 9 | `case:DEF_MEDLYN_G1` | `DEF_MEDLYN_G1` | DEF MEDLYN G1 | `-1.0_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:553 |
| 10 | `case:DEF_NDEP_FREQUENCY` | `DEF_NDEP_FREQUENCY` | DEF NDEP FREQUENCY | `1` |  | case-scalar | editable-common | MOD_Namelist.F90:483 |
| 11 | `case:DEF_OZONE_KO3` | `DEF_OZONE_KO3` | DEF OZONE KO3 | `1.51_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:662 |
| 12 | `case:DEF_PH_CROOT_LATERAL_LENGTH` | `DEF_PH_CROOT_LATERAL_LENGTH` | DEF PH CROOT LATERAL LENGTH | `0.25_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:655 |
| 13 | `case:DEF_PH_FROOT_CARBON` | `DEF_PH_FROOT_CARBON` | DEF PH FROOT CARBON | `288.392056287006_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:657 |
| 14 | `case:DEF_PH_FROOT_LEAF` | `DEF_PH_FROOT_LEAF` | DEF PH FROOT LEAF | `1.5_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:660 |
| 15 | `case:DEF_PH_KRMAX` | `DEF_PH_KRMAX` | DEF PH KRMAX | `3.981071705534969e-009_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:661 |
| 16 | `case:DEF_PH_K_AXS` | `DEF_PH_K_AXS` | DEF PH K AXS | `2.0e-1_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:656 |
| 17 | `case:DEF_PH_ROOT_DENSITY` | `DEF_PH_ROOT_DENSITY` | DEF PH ROOT DENSITY | `310000._r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:659 |
| 18 | `case:DEF_PH_ROOT_RADIUS` | `DEF_PH_ROOT_RADIUS` | DEF PH ROOT RADIUS | `2.9e-4_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:658 |
| 19 | `case:DEF_PROSPECT` | `DEF_PROSPECT` | DEF PROSPECT | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:465 |
| 20 | `case:DEF_RSTFAC` | `DEF_RSTFAC` | DEF RSTFAC | `1` |  | case-scalar | editable-common | MOD_Namelist.F90:545 |
| 21 | `case:DEF_SSP` | `DEF_SSP` | DEF SSP | `off` |  | case-scalar | editable-common | MOD_Namelist.F90:539 |
| 22 | `case:DEF_TUNING_CROP_PLANTING_DAY` | `DEF_TUNING_CROP_PLANTING_DAY` | DEF TUNING CROP PLANTING DAY | `0.0_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:654 |
| 23 | `case:DEF_TUNING_CSOILC` | `DEF_TUNING_CSOILC` | DEF TUNING CSOILC | `0.004_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:628 |
| 24 | `case:DEF_TUNING_DEWMX` | `DEF_TUNING_DEWMX` | DEF TUNING DEWMX | `0.1_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:629 |
| 25 | `case:DEF_TUNING_IRRIGATION_DURATION_SEC` | `DEF_TUNING_IRRIGATION_DURATION_SEC` | DEF TUNING IRRIGATION DURATION SEC | `14400.0_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:647 |
| 26 | `case:DEF_TUNING_IRRIGATION_MAX_CPHASE` | `DEF_TUNING_IRRIGATION_MAX_CPHASE` | DEF TUNING IRRIGATION MAX CPHASE | `4.0_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:652 |
| 27 | `case:DEF_TUNING_IRRIGATION_MAX_DEPTH` | `DEF_TUNING_IRRIGATION_MAX_DEPTH` | DEF TUNING IRRIGATION MAX DEPTH | `1.0_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:648 |
| 28 | `case:DEF_TUNING_IRRIGATION_MIN_CPHASE` | `DEF_TUNING_IRRIGATION_MIN_CPHASE` | DEF TUNING IRRIGATION MIN CPHASE | `1.0_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:651 |
| 29 | `case:DEF_TUNING_IRRIGATION_PONDMX` | `DEF_TUNING_IRRIGATION_PONDMX` | DEF TUNING IRRIGATION PONDMX | `100.0_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:653 |
| 30 | `case:DEF_TUNING_IRRIGATION_START_SEC` | `DEF_TUNING_IRRIGATION_START_SEC` | DEF TUNING IRRIGATION START SEC | `21600.0_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:646 |
| 31 | `case:DEF_TUNING_IRRIGATION_SUPPLY_FRACTION` | `DEF_TUNING_IRRIGATION_SUPPLY_FRACTION` | DEF TUNING IRRIGATION SUPPLY FRACTION | `1.0_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:650 |
| 32 | `case:DEF_TUNING_IRRIGATION_THRESHOLD_FRACTION` | `DEF_TUNING_IRRIGATION_THRESHOLD_FRACTION` | DEF TUNING IRRIGATION THRESHOLD FRACTION | `1.0_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:649 |
| 33 | `case:DEF_TUNING_TRSMX0` | `DEF_TUNING_TRSMX0` | DEF TUNING TRSMX0 | `2.e-4_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:639 |
| 34 | `case:DEF_USE_BGC` | `DEF_USE_BGC` | DEF USE BGC | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:676 |
| 35 | `case:DEF_USE_CNSOYFIXN` | `DEF_USE_CNSOYFIXN` | DEF USE CNSOYFIXN | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:704 |
| 36 | `case:DEF_USE_CROP` | `DEF_USE_CROP` | DEF USE CROP | `.false.` |  | case-scalar | read-only-context | MOD_Namelist.F90:695 |
| 37 | `case:DEF_USE_DiagMatrix` | `DEF_USE_DiagMatrix` | DEF USE DiagMatrix | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:698 |
| 38 | `case:DEF_USE_FERT` | `DEF_USE_FERT` | DEF USE FERT | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:701 |
| 39 | `case:DEF_USE_FIRE` | `DEF_USE_FIRE` | DEF USE FIRE | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:706 |
| 40 | `case:DEF_USE_IRRIGATION` | `DEF_USE_IRRIGATION` | DEF USE IRRIGATION | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:541 |
| 41 | `case:DEF_USE_LAIFEEDBACK` | `DEF_USE_LAIFEEDBACK` | DEF USE LAIFEEDBACK | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:313 |
| 42 | `case:DEF_USE_MEDLYNST` | `DEF_USE_MEDLYNST` | DEF USE MEDLYNST | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:547 |
| 43 | `case:DEF_USE_NITRIF` | `DEF_USE_NITRIF` | DEF USE NITRIF | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:703 |
| 44 | `case:DEF_USE_NOSTRESSNITROGEN` | `DEF_USE_NOSTRESSNITROGEN` | DEF USE NOSTRESSNITROGEN | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:544 |
| 45 | `case:DEF_USE_OZONEDATA` | `DEF_USE_OZONEDATA` | DEF USE OZONEDATA | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:454 |
| 46 | `case:DEF_USE_OZONESTRESS` | `DEF_USE_OZONESTRESS` | DEF USE OZONESTRESS | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:453 |
| 47 | `case:DEF_USE_PLANTHYDRAULICS` | `DEF_USE_PLANTHYDRAULICS` | DEF USE PLANTHYDRAULICS | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:546 |
| 48 | `case:DEF_USE_PN` | `DEF_USE_PN` | DEF USE PN | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:699 |
| 49 | `case:DEF_USE_SASU` | `DEF_USE_SASU` | DEF USE SASU | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:697 |
| 50 | `case:DEF_USE_SNICAR` | `DEF_USE_SNICAR` | DEF USE SNICAR | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:458 |
| 51 | `case:DEF_USE_WUEST` | `DEF_USE_WUEST` | DEF USE WUEST | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:548 |
| 52 | `case:DEF_VEG_SNOW` | `DEF_VEG_SNOW` | DEF VEG SNOW | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:413 |
| 53 | `case:DEF_WUE_LAMBDA` | `DEF_WUE_LAMBDA` | DEF WUE LAMBDA | `-1.0_r8` |  | case-scalar | editable-expert | MOD_Namelist.F90:555 |
| 54 | `case:DEF_file_Ozone` | `DEF_file_Ozone` | DEF file Ozone | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:455 |
| 55 | `case:DEF_file_snowaging` | `DEF_file_snowaging` | DEF file snowaging | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:460 |
| 56 | `case:DEF_file_snowoptics` | `DEF_file_snowoptics` | DEF file snowoptics | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:459 |
| 57 | `lct:IGBP:DEF_LC_BETA` | `DEF_LC_BETA` | 根系分布形状参数 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:592 |
| 58 | `lct:IGBP:DEF_LC_BINTER` | `DEF_LC_BINTER` | Ball–Berry 截距 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:596 |
| 59 | `lct:IGBP:DEF_LC_C3C4` | `DEF_LC_C3C4` | C3/C4 标志 | `-1` | - | land-cover-class | editable-expert | MOD_Namelist.F90:609 |
| 60 | `lct:IGBP:DEF_LC_CHIL` | `DEF_LC_CHIL` | 叶倾角分布参数 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:571 |
| 61 | `lct:IGBP:DEF_LC_CK` | `DEF_LC_CK` | 脆弱性曲线形状参数 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:608 |
| 62 | `lct:IGBP:DEF_LC_D50` | `DEF_LC_D50` | 50% 根系深度 | `-1.e36_r8` | cm | land-cover-class | editable-expert | MOD_Namelist.F90:591 |
| 63 | `lct:IGBP:DEF_LC_DISPLAR` | `DEF_LC_DISPLAR` | 零平面位移高度比例 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:569 |
| 64 | `lct:IGBP:DEF_LC_EFFCON` | `DEF_LC_EFFCON` | 量子效率 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:581 |
| 65 | `lct:IGBP:DEF_LC_EXTKN` | `DEF_LC_EXTKN` | 叶氮分配系数 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:590 |
| 66 | `lct:IGBP:DEF_LC_FVEG0` | `DEF_LC_FVEG0` | 植被覆盖度 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:566 |
| 67 | `lct:IGBP:DEF_LC_G0` | `DEF_LC_G0` | Medlyn 截距 g0 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:598 |
| 68 | `lct:IGBP:DEF_LC_G1` | `DEF_LC_G1` | Medlyn 斜率 g1 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:597 |
| 69 | `lct:IGBP:DEF_LC_GRADM` | `DEF_LC_GRADM` | Ball–Berry 斜率 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:595 |
| 70 | `lct:IGBP:DEF_LC_HBOT0` | `DEF_LC_HBOT0` | 冠层底部高度 | `-1.e36_r8` | m | land-cover-class | editable-expert | MOD_Namelist.F90:565 |
| 71 | `lct:IGBP:DEF_LC_HHTI` | `DEF_LC_HHTI` | 高温抑制半响应温度 | `-1.e36_r8` | K | land-cover-class | editable-expert | MOD_Namelist.F90:588 |
| 72 | `lct:IGBP:DEF_LC_HLTI` | `DEF_LC_HLTI` | 低温抑制半响应温度 | `-1.e36_r8` | K | land-cover-class | editable-expert | MOD_Namelist.F90:589 |
| 73 | `lct:IGBP:DEF_LC_HTOP0` | `DEF_LC_HTOP0` | 冠层顶部高度 | `-1.e36_r8` | m | land-cover-class | editable-expert | MOD_Namelist.F90:564 |
| 74 | `lct:IGBP:DEF_LC_KMAX_ROOT` | `DEF_LC_KMAX_ROOT` | 根最大导水率 | `-1.e36_r8` |  | land-cover-class | editable-expert | MOD_Namelist.F90:603 |
| 75 | `lct:IGBP:DEF_LC_KMAX_SHA` | `DEF_LC_KMAX_SHA` | 阴叶最大导水率 | `-1.e36_r8` |  | land-cover-class | editable-expert | MOD_Namelist.F90:601 |
| 76 | `lct:IGBP:DEF_LC_KMAX_SUN` | `DEF_LC_KMAX_SUN` | 阳叶最大导水率 | `-1.e36_r8` |  | land-cover-class | editable-expert | MOD_Namelist.F90:600 |
| 77 | `lct:IGBP:DEF_LC_KMAX_XYL` | `DEF_LC_KMAX_XYL` | 木质部最大导水率 | `-1.e36_r8` |  | land-cover-class | editable-expert | MOD_Namelist.F90:602 |
| 78 | `lct:IGBP:DEF_LC_LAMBDA` | `DEF_LC_LAMBDA` | WUE 边际水分成本 | `-1.e36_r8` | mol mol-1 | land-cover-class | editable-expert | MOD_Namelist.F90:599 |
| 79 | `lct:IGBP:DEF_LC_PSI50_ROOT` | `DEF_LC_PSI50_ROOT` | 根 50% 失导水势 | `-1.e36_r8` | mmH2O | land-cover-class | editable-expert | MOD_Namelist.F90:607 |
| 80 | `lct:IGBP:DEF_LC_PSI50_SHA` | `DEF_LC_PSI50_SHA` | 阴叶 50% 失导水势 | `-1.e36_r8` | mmH2O | land-cover-class | editable-expert | MOD_Namelist.F90:605 |
| 81 | `lct:IGBP:DEF_LC_PSI50_SUN` | `DEF_LC_PSI50_SUN` | 阳叶 50% 失导水势 | `-1.e36_r8` | mmH2O | land-cover-class | editable-expert | MOD_Namelist.F90:604 |
| 82 | `lct:IGBP:DEF_LC_PSI50_XYL` | `DEF_LC_PSI50_XYL` | 木质部 50% 失导水势 | `-1.e36_r8` | mmH2O | land-cover-class | editable-expert | MOD_Namelist.F90:606 |
| 83 | `lct:IGBP:DEF_LC_RESPCP` | `DEF_LC_RESPCP` | 叶呼吸比例 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:582 |
| 84 | `lct:IGBP:DEF_LC_RHOL_NIR` | `DEF_LC_RHOL_NIR` | 绿叶近红外反射率 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:573 |
| 85 | `lct:IGBP:DEF_LC_RHOL_VIS` | `DEF_LC_RHOL_VIS` | 绿叶可见光反射率 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:572 |
| 86 | `lct:IGBP:DEF_LC_RHOS_NIR` | `DEF_LC_RHOS_NIR` | 枯叶近红外反射率 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:575 |
| 87 | `lct:IGBP:DEF_LC_RHOS_VIS` | `DEF_LC_RHOS_VIS` | 枯叶可见光反射率 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:574 |
| 88 | `lct:IGBP:DEF_LC_SAI0` | `DEF_LC_SAI0` | 茎面积指数 | `-1.e36_r8` | m2 m-2 | land-cover-class | editable-expert | MOD_Namelist.F90:567 |
| 89 | `lct:IGBP:DEF_LC_SHTI` | `DEF_LC_SHTI` | 高温抑制斜率 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:583 |
| 90 | `lct:IGBP:DEF_LC_SLTI` | `DEF_LC_SLTI` | 低温抑制斜率 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:584 |
| 91 | `lct:IGBP:DEF_LC_SQRTDI` | `DEF_LC_SQRTDI` | 叶片特征尺寸倒平方根 | `-1.e36_r8` | m-0.5 | land-cover-class | editable-expert | MOD_Namelist.F90:570 |
| 92 | `lct:IGBP:DEF_LC_TAUL_NIR` | `DEF_LC_TAUL_NIR` | 绿叶近红外透射率 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:577 |
| 93 | `lct:IGBP:DEF_LC_TAUL_VIS` | `DEF_LC_TAUL_VIS` | 绿叶可见光透射率 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:576 |
| 94 | `lct:IGBP:DEF_LC_TAUS_NIR` | `DEF_LC_TAUS_NIR` | 枯叶近红外透射率 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:579 |
| 95 | `lct:IGBP:DEF_LC_TAUS_VIS` | `DEF_LC_TAUS_VIS` | 枯叶可见光透射率 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:578 |
| 96 | `lct:IGBP:DEF_LC_TRDA` | `DEF_LC_TRDA` | 气孔模型温度系数 A | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:585 |
| 97 | `lct:IGBP:DEF_LC_TRDM` | `DEF_LC_TRDM` | 气孔模型温度系数 M | `-1.e36_r8` | K | land-cover-class | editable-expert | MOD_Namelist.F90:586 |
| 98 | `lct:IGBP:DEF_LC_TROP` | `DEF_LC_TROP` | 最适温度 | `-1.e36_r8` | K | land-cover-class | editable-expert | MOD_Namelist.F90:587 |
| 99 | `lct:IGBP:DEF_LC_VMAX25` | `DEF_LC_VMAX25` | 25°C 最大羧化速率 | `-1.e36_r8` | umol m-2 s-1 | land-cover-class | editable-expert | MOD_Namelist.F90:580 |
| 100 | `lct:IGBP:DEF_LC_Z0MR` | `DEF_LC_Z0MR` | 粗糙度长度比例 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:568 |
| 101 | `lct:USGS:DEF_LC_BETA` | `DEF_LC_BETA` | 根系分布形状参数 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:592 |
| 102 | `lct:USGS:DEF_LC_BINTER` | `DEF_LC_BINTER` | Ball–Berry 截距 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:596 |
| 103 | `lct:USGS:DEF_LC_C3C4` | `DEF_LC_C3C4` | C3/C4 标志 | `-1` | - | land-cover-class | editable-expert | MOD_Namelist.F90:609 |
| 104 | `lct:USGS:DEF_LC_CHIL` | `DEF_LC_CHIL` | 叶倾角分布参数 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:571 |
| 105 | `lct:USGS:DEF_LC_CK` | `DEF_LC_CK` | 脆弱性曲线形状参数 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:608 |
| 106 | `lct:USGS:DEF_LC_D50` | `DEF_LC_D50` | 50% 根系深度 | `-1.e36_r8` | cm | land-cover-class | editable-expert | MOD_Namelist.F90:591 |
| 107 | `lct:USGS:DEF_LC_DISPLAR` | `DEF_LC_DISPLAR` | 零平面位移高度比例 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:569 |
| 108 | `lct:USGS:DEF_LC_EFFCON` | `DEF_LC_EFFCON` | 量子效率 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:581 |
| 109 | `lct:USGS:DEF_LC_EXTKN` | `DEF_LC_EXTKN` | 叶氮分配系数 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:590 |
| 110 | `lct:USGS:DEF_LC_FVEG0` | `DEF_LC_FVEG0` | 植被覆盖度 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:566 |
| 111 | `lct:USGS:DEF_LC_G0` | `DEF_LC_G0` | Medlyn 截距 g0 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:598 |
| 112 | `lct:USGS:DEF_LC_G1` | `DEF_LC_G1` | Medlyn 斜率 g1 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:597 |
| 113 | `lct:USGS:DEF_LC_GRADM` | `DEF_LC_GRADM` | Ball–Berry 斜率 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:595 |
| 114 | `lct:USGS:DEF_LC_HBOT0` | `DEF_LC_HBOT0` | 冠层底部高度 | `-1.e36_r8` | m | land-cover-class | editable-expert | MOD_Namelist.F90:565 |
| 115 | `lct:USGS:DEF_LC_HHTI` | `DEF_LC_HHTI` | 高温抑制半响应温度 | `-1.e36_r8` | K | land-cover-class | editable-expert | MOD_Namelist.F90:588 |
| 116 | `lct:USGS:DEF_LC_HLTI` | `DEF_LC_HLTI` | 低温抑制半响应温度 | `-1.e36_r8` | K | land-cover-class | editable-expert | MOD_Namelist.F90:589 |
| 117 | `lct:USGS:DEF_LC_HTOP0` | `DEF_LC_HTOP0` | 冠层顶部高度 | `-1.e36_r8` | m | land-cover-class | editable-expert | MOD_Namelist.F90:564 |
| 118 | `lct:USGS:DEF_LC_KMAX_ROOT` | `DEF_LC_KMAX_ROOT` | 根最大导水率 | `-1.e36_r8` |  | land-cover-class | editable-expert | MOD_Namelist.F90:603 |
| 119 | `lct:USGS:DEF_LC_KMAX_SHA` | `DEF_LC_KMAX_SHA` | 阴叶最大导水率 | `-1.e36_r8` |  | land-cover-class | editable-expert | MOD_Namelist.F90:601 |
| 120 | `lct:USGS:DEF_LC_KMAX_SUN` | `DEF_LC_KMAX_SUN` | 阳叶最大导水率 | `-1.e36_r8` |  | land-cover-class | editable-expert | MOD_Namelist.F90:600 |
| 121 | `lct:USGS:DEF_LC_KMAX_XYL` | `DEF_LC_KMAX_XYL` | 木质部最大导水率 | `-1.e36_r8` |  | land-cover-class | editable-expert | MOD_Namelist.F90:602 |
| 122 | `lct:USGS:DEF_LC_LAMBDA` | `DEF_LC_LAMBDA` | WUE 边际水分成本 | `-1.e36_r8` | mol mol-1 | land-cover-class | editable-expert | MOD_Namelist.F90:599 |
| 123 | `lct:USGS:DEF_LC_PSI50_ROOT` | `DEF_LC_PSI50_ROOT` | 根 50% 失导水势 | `-1.e36_r8` | mmH2O | land-cover-class | editable-expert | MOD_Namelist.F90:607 |
| 124 | `lct:USGS:DEF_LC_PSI50_SHA` | `DEF_LC_PSI50_SHA` | 阴叶 50% 失导水势 | `-1.e36_r8` | mmH2O | land-cover-class | editable-expert | MOD_Namelist.F90:605 |
| 125 | `lct:USGS:DEF_LC_PSI50_SUN` | `DEF_LC_PSI50_SUN` | 阳叶 50% 失导水势 | `-1.e36_r8` | mmH2O | land-cover-class | editable-expert | MOD_Namelist.F90:604 |
| 126 | `lct:USGS:DEF_LC_PSI50_XYL` | `DEF_LC_PSI50_XYL` | 木质部 50% 失导水势 | `-1.e36_r8` | mmH2O | land-cover-class | editable-expert | MOD_Namelist.F90:606 |
| 127 | `lct:USGS:DEF_LC_RESPCP` | `DEF_LC_RESPCP` | 叶呼吸比例 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:582 |
| 128 | `lct:USGS:DEF_LC_RHOL_NIR` | `DEF_LC_RHOL_NIR` | 绿叶近红外反射率 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:573 |
| 129 | `lct:USGS:DEF_LC_RHOL_VIS` | `DEF_LC_RHOL_VIS` | 绿叶可见光反射率 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:572 |
| 130 | `lct:USGS:DEF_LC_RHOS_NIR` | `DEF_LC_RHOS_NIR` | 枯叶近红外反射率 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:575 |
| 131 | `lct:USGS:DEF_LC_RHOS_VIS` | `DEF_LC_RHOS_VIS` | 枯叶可见光反射率 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:574 |
| 132 | `lct:USGS:DEF_LC_SAI0` | `DEF_LC_SAI0` | 茎面积指数 | `-1.e36_r8` | m2 m-2 | land-cover-class | editable-expert | MOD_Namelist.F90:567 |
| 133 | `lct:USGS:DEF_LC_SHTI` | `DEF_LC_SHTI` | 高温抑制斜率 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:583 |
| 134 | `lct:USGS:DEF_LC_SLTI` | `DEF_LC_SLTI` | 低温抑制斜率 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:584 |
| 135 | `lct:USGS:DEF_LC_SQRTDI` | `DEF_LC_SQRTDI` | 叶片特征尺寸倒平方根 | `-1.e36_r8` | m-0.5 | land-cover-class | editable-expert | MOD_Namelist.F90:570 |
| 136 | `lct:USGS:DEF_LC_TAUL_NIR` | `DEF_LC_TAUL_NIR` | 绿叶近红外透射率 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:577 |
| 137 | `lct:USGS:DEF_LC_TAUL_VIS` | `DEF_LC_TAUL_VIS` | 绿叶可见光透射率 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:576 |
| 138 | `lct:USGS:DEF_LC_TAUS_NIR` | `DEF_LC_TAUS_NIR` | 枯叶近红外透射率 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:579 |
| 139 | `lct:USGS:DEF_LC_TAUS_VIS` | `DEF_LC_TAUS_VIS` | 枯叶可见光透射率 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:578 |
| 140 | `lct:USGS:DEF_LC_TRDA` | `DEF_LC_TRDA` | 气孔模型温度系数 A | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:585 |
| 141 | `lct:USGS:DEF_LC_TRDM` | `DEF_LC_TRDM` | 气孔模型温度系数 M | `-1.e36_r8` | K | land-cover-class | editable-expert | MOD_Namelist.F90:586 |
| 142 | `lct:USGS:DEF_LC_TROP` | `DEF_LC_TROP` | 最适温度 | `-1.e36_r8` | K | land-cover-class | editable-expert | MOD_Namelist.F90:587 |
| 143 | `lct:USGS:DEF_LC_VMAX25` | `DEF_LC_VMAX25` | 25°C 最大羧化速率 | `-1.e36_r8` | umol m-2 s-1 | land-cover-class | editable-expert | MOD_Namelist.F90:580 |
| 144 | `lct:USGS:DEF_LC_Z0MR` | `DEF_LC_Z0MR` | 粗糙度长度比例 | `-1.e36_r8` | - | land-cover-class | editable-expert | MOD_Namelist.F90:568 |
| 145 | `pc-pft:DEF_PFT_ALLCONSS` | `DEF_PFT_ALLCONSS` | 茎分配衰减指数 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 146 | `pc-pft:DEF_PFT_AROOTF` | `DEF_PFT_AROOTF` | 最终根分配比例 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 147 | `pc-pft:DEF_PFT_AROOTI` | `DEF_PFT_AROOTI` | 初始根分配比例 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 148 | `pc-pft:DEF_PFT_ASTEMF` | `DEF_PFT_ASTEMF` | 成熟期最低茎分配比例 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 149 | `pc-pft:DEF_PFT_BASET` | `DEF_PFT_BASET` | 积温基准温度 | `` | °C | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 150 | `pc-pft:DEF_PFT_BETA` | `DEF_PFT_BETA` | 根系分布形状参数 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 151 | `pc-pft:DEF_PFT_BFACT` | `DEF_PFT_BFACT` | 叶片分配曲线形状系数 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 152 | `pc-pft:DEF_PFT_BINTER` | `DEF_PFT_BINTER` | Ball–Berry 截距 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 153 | `pc-pft:DEF_PFT_C3C4` | `DEF_PFT_C3C4` | C3/C4 标志 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 154 | `pc-pft:DEF_PFT_CC_DSTEM` | `DEF_PFT_CC_DSTEM` | 死茎燃烧完全度 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 155 | `pc-pft:DEF_PFT_CC_LEAF` | `DEF_PFT_CC_LEAF` | 叶片燃烧完全度 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 156 | `pc-pft:DEF_PFT_CC_LSTEM` | `DEF_PFT_CC_LSTEM` | 活茎燃烧完全度 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 157 | `pc-pft:DEF_PFT_CC_OTHER` | `DEF_PFT_CC_OTHER` | 其他组织燃烧完全度 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 158 | `pc-pft:DEF_PFT_CHIL` | `DEF_PFT_CHIL` | 叶倾角分布参数 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 159 | `pc-pft:DEF_PFT_CK` | `DEF_PFT_CK` | 脆弱性曲线形状参数 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 160 | `pc-pft:DEF_PFT_CROOT_STEM` | `DEF_PFT_CROOT_STEM` | 粗根与茎分配比 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 161 | `pc-pft:DEF_PFT_D50` | `DEF_PFT_D50` | 50%根系深度 | `` | cm | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 162 | `pc-pft:DEF_PFT_DEADWDCN` | `DEF_PFT_DEADWDCN` | 死木质部碳氮比 | `` | gC gN-1 | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 163 | `pc-pft:DEF_PFT_EFFCON` | `DEF_PFT_EFFCON` | 量子效率 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 164 | `pc-pft:DEF_PFT_EXTKN` | `DEF_PFT_EXTKN` | 叶氮分配系数 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 165 | `pc-pft:DEF_PFT_FCUR2` | `DEF_PFT_FCUR2` | 当前光合产物直接生长比例 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 166 | `pc-pft:DEF_PFT_FD_PFT` | `DEF_PFT_FD_PFT` | 火灾持续时间 | `` | h | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 167 | `pc-pft:DEF_PFT_FFROOTCN` | `DEF_PFT_FFROOTCN` | 灌浆期细根碳氮比 | `` | gC gN-1 | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 168 | `pc-pft:DEF_PFT_FLEAFCN` | `DEF_PFT_FLEAFCN` | 灌浆期叶片碳氮比 | `` | gC gN-1 | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 169 | `pc-pft:DEF_PFT_FLEAFI` | `DEF_PFT_FLEAFI` | 初始叶片分配比例 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 170 | `pc-pft:DEF_PFT_FLIVEWD` | `DEF_PFT_FLIVEWD` | 活木质部比例 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 171 | `pc-pft:DEF_PFT_FM_DROOT` | `DEF_PFT_FM_DROOT` | 死粗根火致死亡比例 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 172 | `pc-pft:DEF_PFT_FM_LEAF` | `DEF_PFT_FM_LEAF` | 叶片火致死亡比例 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 173 | `pc-pft:DEF_PFT_FM_LROOT` | `DEF_PFT_FM_LROOT` | 活粗根火致死亡比例 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 174 | `pc-pft:DEF_PFT_FM_LSTEM` | `DEF_PFT_FM_LSTEM` | 活茎火致死亡比例 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 175 | `pc-pft:DEF_PFT_FM_OTHER` | `DEF_PFT_FM_OTHER` | 其他组织火致死亡比例 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 176 | `pc-pft:DEF_PFT_FM_ROOT` | `DEF_PFT_FM_ROOT` | 细根火致死亡比例 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 177 | `pc-pft:DEF_PFT_FROOTCN` | `DEF_PFT_FROOTCN` | 细根碳氮比 | `` | gC gN-1 | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 178 | `pc-pft:DEF_PFT_FROOT_LEAF` | `DEF_PFT_FROOT_LEAF` | 细根与叶片分配比 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 179 | `pc-pft:DEF_PFT_FR_FCEL` | `DEF_PFT_FR_FCEL` | 细根凋落物纤维素组分比例 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 180 | `pc-pft:DEF_PFT_FR_FLAB` | `DEF_PFT_FR_FLAB` | 细根凋落物易分解组分比例 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 181 | `pc-pft:DEF_PFT_FR_FLIG` | `DEF_PFT_FR_FLIG` | 细根凋落物木质素组分比例 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 182 | `pc-pft:DEF_PFT_FSR_PFT` | `DEF_PFT_FSR_PFT` | 火灾存活率 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 183 | `pc-pft:DEF_PFT_FSTEMCN` | `DEF_PFT_FSTEMCN` | 灌浆期茎碳氮比 | `` | gC gN-1 | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 184 | `pc-pft:DEF_PFT_G0` | `DEF_PFT_G0` | Medlyn 截距 g0 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 185 | `pc-pft:DEF_PFT_G1` | `DEF_PFT_G1` | Medlyn 斜率 g1 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 186 | `pc-pft:DEF_PFT_GRADM` | `DEF_PFT_GRADM` | Ball–Berry 斜率 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 187 | `pc-pft:DEF_PFT_GRAINCN` | `DEF_PFT_GRAINCN` | 籽粒碳氮比 | `` | gC gN-1 | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 188 | `pc-pft:DEF_PFT_GRNFILL` | `DEF_PFT_GRNFILL` | 灌浆热量指数阈值 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 189 | `pc-pft:DEF_PFT_GRPERC` | `DEF_PFT_GRPERC` | 生长呼吸比例 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 190 | `pc-pft:DEF_PFT_GRPNOW` | `DEF_PFT_GRPNOW` | 生长呼吸即时支付比例 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 191 | `pc-pft:DEF_PFT_HBOT0` | `DEF_PFT_HBOT0` | 冠层底部高度 | `` | m | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 192 | `pc-pft:DEF_PFT_HHTI` | `DEF_PFT_HHTI` | 高温抑制半响应温度 | `` | K | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 193 | `pc-pft:DEF_PFT_HLTI` | `DEF_PFT_HLTI` | 低温抑制半响应温度 | `` | K | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 194 | `pc-pft:DEF_PFT_HTOP0` | `DEF_PFT_HTOP0` | 冠层顶部高度 | `` | m | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 195 | `pc-pft:DEF_PFT_KMAX_ROOT` | `DEF_PFT_KMAX_ROOT` | 根最大导水率 | `` |  | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 196 | `pc-pft:DEF_PFT_KMAX_SHA` | `DEF_PFT_KMAX_SHA` | 阴叶最大导水率 | `` |  | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 197 | `pc-pft:DEF_PFT_KMAX_SUN` | `DEF_PFT_KMAX_SUN` | 阳叶最大导水率 | `` |  | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 198 | `pc-pft:DEF_PFT_KMAX_XYL` | `DEF_PFT_KMAX_XYL` | 木质部最大导水率 | `` |  | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 199 | `pc-pft:DEF_PFT_LAIMX` | `DEF_PFT_LAIMX` | 最大叶面积指数 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 200 | `pc-pft:DEF_PFT_LAMBDA` | `DEF_PFT_LAMBDA` | WUE 边际水分成本 | `` | mol mol-1 | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 201 | `pc-pft:DEF_PFT_LEAFCN` | `DEF_PFT_LEAFCN` | 叶片碳氮比 | `` | gC gN-1 | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 202 | `pc-pft:DEF_PFT_LEAF_LONG` | `DEF_PFT_LEAF_LONG` | 叶片寿命 | `` | yr | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 203 | `pc-pft:DEF_PFT_LFEMERG` | `DEF_PFT_LFEMERG` | 出苗热量指数阈值 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 204 | `pc-pft:DEF_PFT_LFLITCN` | `DEF_PFT_LFLITCN` | 叶凋落物碳氮比 | `` | gC gN-1 | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 205 | `pc-pft:DEF_PFT_LF_FCEL` | `DEF_PFT_LF_FCEL` | 叶凋落物纤维素组分比例 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 206 | `pc-pft:DEF_PFT_LF_FLAB` | `DEF_PFT_LF_FLAB` | 叶凋落物易分解组分比例 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 207 | `pc-pft:DEF_PFT_LF_FLIG` | `DEF_PFT_LF_FLIG` | 叶凋落物木质素组分比例 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 208 | `pc-pft:DEF_PFT_LIVEWDCN` | `DEF_PFT_LIVEWDCN` | 活木质部碳氮比 | `` | gC gN-1 | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 209 | `pc-pft:DEF_PFT_MANURE` | `DEF_PFT_MANURE` | 最大有机肥氮施用量 | `` | kg N m-2 | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 210 | `pc-pft:DEF_PFT_MXMAT` | `DEF_PFT_MXMAT` | 最大成熟天数 | `` | d | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 211 | `pc-pft:DEF_PFT_PSI50_ROOT` | `DEF_PFT_PSI50_ROOT` | 根50%失导水势 | `` | mmH2O | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 212 | `pc-pft:DEF_PFT_PSI50_SHA` | `DEF_PFT_PSI50_SHA` | 阴叶50%失导水势 | `` | mmH2O | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 213 | `pc-pft:DEF_PFT_PSI50_SUN` | `DEF_PFT_PSI50_SUN` | 阳叶50%失导水势 | `` | mmH2O | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 214 | `pc-pft:DEF_PFT_PSI50_XYL` | `DEF_PFT_PSI50_XYL` | 木质部50%失导水势 | `` | mmH2O | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 215 | `pc-pft:DEF_PFT_RHOL_NIR` | `DEF_PFT_RHOL_NIR` | 绿叶近红外反射率 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 216 | `pc-pft:DEF_PFT_RHOL_VIS` | `DEF_PFT_RHOL_VIS` | 绿叶可见光反射率 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 217 | `pc-pft:DEF_PFT_RHOS_NIR` | `DEF_PFT_RHOS_NIR` | 枯叶近红外反射率 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 218 | `pc-pft:DEF_PFT_RHOS_VIS` | `DEF_PFT_RHOS_VIS` | 枯叶可见光反射率 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 219 | `pc-pft:DEF_PFT_SHTI` | `DEF_PFT_SHTI` | 高温抑制斜率 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 220 | `pc-pft:DEF_PFT_SLATOP` | `DEF_PFT_SLATOP` | 冠层顶部比叶面积 | `` | m2 gC-1 | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 221 | `pc-pft:DEF_PFT_SLTI` | `DEF_PFT_SLTI` | 低温抑制斜率 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 222 | `pc-pft:DEF_PFT_SQRTDI` | `DEF_PFT_SQRTDI` | 叶片特征尺寸倒平方根 | `` | m-0.5 | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 223 | `pc-pft:DEF_PFT_STEM_LEAF` | `DEF_PFT_STEM_LEAF` | 茎与叶片分配比 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 224 | `pc-pft:DEF_PFT_TAUL_NIR` | `DEF_PFT_TAUL_NIR` | 绿叶近红外透射率 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 225 | `pc-pft:DEF_PFT_TAUL_VIS` | `DEF_PFT_TAUL_VIS` | 绿叶可见光透射率 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 226 | `pc-pft:DEF_PFT_TAUS_NIR` | `DEF_PFT_TAUS_NIR` | 枯叶近红外透射率 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 227 | `pc-pft:DEF_PFT_TAUS_VIS` | `DEF_PFT_TAUS_VIS` | 枯叶可见光透射率 | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 228 | `pc-pft:DEF_PFT_TRDA` | `DEF_PFT_TRDA` | 气孔温度系数 A | `` | - | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 229 | `pc-pft:DEF_PFT_TRDM` | `DEF_PFT_TRDM` | 气孔温度系数 M | `` | K | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 230 | `pc-pft:DEF_PFT_TROP` | `DEF_PFT_TROP` | 最适温度 | `` | K | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 231 | `pc-pft:DEF_PFT_VMAX25` | `DEF_PFT_VMAX25` | 25°C最大羧化速率 | `` | umol m-2 s-1 | pc-pft-component | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 232 | `pft:DEF_PFT_ALLCONSS` | `DEF_PFT_ALLCONSS` | 茎分配衰减指数 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 233 | `pft:DEF_PFT_AROOTF` | `DEF_PFT_AROOTF` | 最终根分配比例 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 234 | `pft:DEF_PFT_AROOTI` | `DEF_PFT_AROOTI` | 初始根分配比例 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 235 | `pft:DEF_PFT_ASTEMF` | `DEF_PFT_ASTEMF` | 成熟期最低茎分配比例 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 236 | `pft:DEF_PFT_BASET` | `DEF_PFT_BASET` | 积温基准温度 | `` | °C | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 237 | `pft:DEF_PFT_BETA` | `DEF_PFT_BETA` | 根系分布形状参数 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 238 | `pft:DEF_PFT_BFACT` | `DEF_PFT_BFACT` | 叶片分配曲线形状系数 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 239 | `pft:DEF_PFT_BINTER` | `DEF_PFT_BINTER` | Ball–Berry 截距 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 240 | `pft:DEF_PFT_C3C4` | `DEF_PFT_C3C4` | C3/C4 标志 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 241 | `pft:DEF_PFT_CC_DSTEM` | `DEF_PFT_CC_DSTEM` | 死茎燃烧完全度 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 242 | `pft:DEF_PFT_CC_LEAF` | `DEF_PFT_CC_LEAF` | 叶片燃烧完全度 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 243 | `pft:DEF_PFT_CC_LSTEM` | `DEF_PFT_CC_LSTEM` | 活茎燃烧完全度 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 244 | `pft:DEF_PFT_CC_OTHER` | `DEF_PFT_CC_OTHER` | 其他组织燃烧完全度 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 245 | `pft:DEF_PFT_CHIL` | `DEF_PFT_CHIL` | 叶倾角分布参数 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 246 | `pft:DEF_PFT_CK` | `DEF_PFT_CK` | 脆弱性曲线形状参数 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 247 | `pft:DEF_PFT_CROOT_STEM` | `DEF_PFT_CROOT_STEM` | 粗根与茎分配比 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 248 | `pft:DEF_PFT_D50` | `DEF_PFT_D50` | 50%根系深度 | `` | cm | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 249 | `pft:DEF_PFT_DEADWDCN` | `DEF_PFT_DEADWDCN` | 死木质部碳氮比 | `` | gC gN-1 | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 250 | `pft:DEF_PFT_EFFCON` | `DEF_PFT_EFFCON` | 量子效率 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 251 | `pft:DEF_PFT_EXTKN` | `DEF_PFT_EXTKN` | 叶氮分配系数 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 252 | `pft:DEF_PFT_FCUR2` | `DEF_PFT_FCUR2` | 当前光合产物直接生长比例 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 253 | `pft:DEF_PFT_FD_PFT` | `DEF_PFT_FD_PFT` | 火灾持续时间 | `` | h | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 254 | `pft:DEF_PFT_FFROOTCN` | `DEF_PFT_FFROOTCN` | 灌浆期细根碳氮比 | `` | gC gN-1 | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 255 | `pft:DEF_PFT_FLEAFCN` | `DEF_PFT_FLEAFCN` | 灌浆期叶片碳氮比 | `` | gC gN-1 | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 256 | `pft:DEF_PFT_FLEAFI` | `DEF_PFT_FLEAFI` | 初始叶片分配比例 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 257 | `pft:DEF_PFT_FLIVEWD` | `DEF_PFT_FLIVEWD` | 活木质部比例 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 258 | `pft:DEF_PFT_FM_DROOT` | `DEF_PFT_FM_DROOT` | 死粗根火致死亡比例 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 259 | `pft:DEF_PFT_FM_LEAF` | `DEF_PFT_FM_LEAF` | 叶片火致死亡比例 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 260 | `pft:DEF_PFT_FM_LROOT` | `DEF_PFT_FM_LROOT` | 活粗根火致死亡比例 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 261 | `pft:DEF_PFT_FM_LSTEM` | `DEF_PFT_FM_LSTEM` | 活茎火致死亡比例 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 262 | `pft:DEF_PFT_FM_OTHER` | `DEF_PFT_FM_OTHER` | 其他组织火致死亡比例 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 263 | `pft:DEF_PFT_FM_ROOT` | `DEF_PFT_FM_ROOT` | 细根火致死亡比例 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 264 | `pft:DEF_PFT_FROOTCN` | `DEF_PFT_FROOTCN` | 细根碳氮比 | `` | gC gN-1 | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 265 | `pft:DEF_PFT_FROOT_LEAF` | `DEF_PFT_FROOT_LEAF` | 细根与叶片分配比 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 266 | `pft:DEF_PFT_FR_FCEL` | `DEF_PFT_FR_FCEL` | 细根凋落物纤维素组分比例 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 267 | `pft:DEF_PFT_FR_FLAB` | `DEF_PFT_FR_FLAB` | 细根凋落物易分解组分比例 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 268 | `pft:DEF_PFT_FR_FLIG` | `DEF_PFT_FR_FLIG` | 细根凋落物木质素组分比例 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 269 | `pft:DEF_PFT_FSR_PFT` | `DEF_PFT_FSR_PFT` | 火灾存活率 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 270 | `pft:DEF_PFT_FSTEMCN` | `DEF_PFT_FSTEMCN` | 灌浆期茎碳氮比 | `` | gC gN-1 | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 271 | `pft:DEF_PFT_G0` | `DEF_PFT_G0` | Medlyn 截距 g0 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 272 | `pft:DEF_PFT_G1` | `DEF_PFT_G1` | Medlyn 斜率 g1 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 273 | `pft:DEF_PFT_GRADM` | `DEF_PFT_GRADM` | Ball–Berry 斜率 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 274 | `pft:DEF_PFT_GRAINCN` | `DEF_PFT_GRAINCN` | 籽粒碳氮比 | `` | gC gN-1 | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 275 | `pft:DEF_PFT_GRNFILL` | `DEF_PFT_GRNFILL` | 灌浆热量指数阈值 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 276 | `pft:DEF_PFT_GRPERC` | `DEF_PFT_GRPERC` | 生长呼吸比例 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 277 | `pft:DEF_PFT_GRPNOW` | `DEF_PFT_GRPNOW` | 生长呼吸即时支付比例 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 278 | `pft:DEF_PFT_HBOT0` | `DEF_PFT_HBOT0` | 冠层底部高度 | `` | m | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 279 | `pft:DEF_PFT_HHTI` | `DEF_PFT_HHTI` | 高温抑制半响应温度 | `` | K | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 280 | `pft:DEF_PFT_HLTI` | `DEF_PFT_HLTI` | 低温抑制半响应温度 | `` | K | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 281 | `pft:DEF_PFT_HTOP0` | `DEF_PFT_HTOP0` | 冠层顶部高度 | `` | m | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 282 | `pft:DEF_PFT_KMAX_ROOT` | `DEF_PFT_KMAX_ROOT` | 根最大导水率 | `` |  | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 283 | `pft:DEF_PFT_KMAX_SHA` | `DEF_PFT_KMAX_SHA` | 阴叶最大导水率 | `` |  | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 284 | `pft:DEF_PFT_KMAX_SUN` | `DEF_PFT_KMAX_SUN` | 阳叶最大导水率 | `` |  | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 285 | `pft:DEF_PFT_KMAX_XYL` | `DEF_PFT_KMAX_XYL` | 木质部最大导水率 | `` |  | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 286 | `pft:DEF_PFT_LAIMX` | `DEF_PFT_LAIMX` | 最大叶面积指数 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 287 | `pft:DEF_PFT_LAMBDA` | `DEF_PFT_LAMBDA` | WUE 边际水分成本 | `` | mol mol-1 | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 288 | `pft:DEF_PFT_LEAFCN` | `DEF_PFT_LEAFCN` | 叶片碳氮比 | `` | gC gN-1 | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 289 | `pft:DEF_PFT_LEAF_LONG` | `DEF_PFT_LEAF_LONG` | 叶片寿命 | `` | yr | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 290 | `pft:DEF_PFT_LFEMERG` | `DEF_PFT_LFEMERG` | 出苗热量指数阈值 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 291 | `pft:DEF_PFT_LFLITCN` | `DEF_PFT_LFLITCN` | 叶凋落物碳氮比 | `` | gC gN-1 | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 292 | `pft:DEF_PFT_LF_FCEL` | `DEF_PFT_LF_FCEL` | 叶凋落物纤维素组分比例 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 293 | `pft:DEF_PFT_LF_FLAB` | `DEF_PFT_LF_FLAB` | 叶凋落物易分解组分比例 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 294 | `pft:DEF_PFT_LF_FLIG` | `DEF_PFT_LF_FLIG` | 叶凋落物木质素组分比例 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 295 | `pft:DEF_PFT_LIVEWDCN` | `DEF_PFT_LIVEWDCN` | 活木质部碳氮比 | `` | gC gN-1 | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 296 | `pft:DEF_PFT_MANURE` | `DEF_PFT_MANURE` | 最大有机肥氮施用量 | `` | kg N m-2 | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 297 | `pft:DEF_PFT_MXMAT` | `DEF_PFT_MXMAT` | 最大成熟天数 | `` | d | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 298 | `pft:DEF_PFT_PSI50_ROOT` | `DEF_PFT_PSI50_ROOT` | 根50%失导水势 | `` | mmH2O | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 299 | `pft:DEF_PFT_PSI50_SHA` | `DEF_PFT_PSI50_SHA` | 阴叶50%失导水势 | `` | mmH2O | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 300 | `pft:DEF_PFT_PSI50_SUN` | `DEF_PFT_PSI50_SUN` | 阳叶50%失导水势 | `` | mmH2O | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 301 | `pft:DEF_PFT_PSI50_XYL` | `DEF_PFT_PSI50_XYL` | 木质部50%失导水势 | `` | mmH2O | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 302 | `pft:DEF_PFT_RHOL_NIR` | `DEF_PFT_RHOL_NIR` | 绿叶近红外反射率 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 303 | `pft:DEF_PFT_RHOL_VIS` | `DEF_PFT_RHOL_VIS` | 绿叶可见光反射率 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 304 | `pft:DEF_PFT_RHOS_NIR` | `DEF_PFT_RHOS_NIR` | 枯叶近红外反射率 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 305 | `pft:DEF_PFT_RHOS_VIS` | `DEF_PFT_RHOS_VIS` | 枯叶可见光反射率 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 306 | `pft:DEF_PFT_SHTI` | `DEF_PFT_SHTI` | 高温抑制斜率 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 307 | `pft:DEF_PFT_SLATOP` | `DEF_PFT_SLATOP` | 冠层顶部比叶面积 | `` | m2 gC-1 | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 308 | `pft:DEF_PFT_SLTI` | `DEF_PFT_SLTI` | 低温抑制斜率 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 309 | `pft:DEF_PFT_SQRTDI` | `DEF_PFT_SQRTDI` | 叶片特征尺寸倒平方根 | `` | m-0.5 | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 310 | `pft:DEF_PFT_STEM_LEAF` | `DEF_PFT_STEM_LEAF` | 茎与叶片分配比 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 311 | `pft:DEF_PFT_TAUL_NIR` | `DEF_PFT_TAUL_NIR` | 绿叶近红外透射率 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 312 | `pft:DEF_PFT_TAUL_VIS` | `DEF_PFT_TAUL_VIS` | 绿叶可见光透射率 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 313 | `pft:DEF_PFT_TAUS_NIR` | `DEF_PFT_TAUS_NIR` | 枯叶近红外透射率 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 314 | `pft:DEF_PFT_TAUS_VIS` | `DEF_PFT_TAUS_VIS` | 枯叶可见光透射率 | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 315 | `pft:DEF_PFT_TRDA` | `DEF_PFT_TRDA` | 气孔温度系数 A | `` | - | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 316 | `pft:DEF_PFT_TRDM` | `DEF_PFT_TRDM` | 气孔温度系数 M | `` | K | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 317 | `pft:DEF_PFT_TROP` | `DEF_PFT_TROP` | 最适温度 | `` | K | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |
| 318 | `pft:DEF_PFT_VMAX25` | `DEF_PFT_VMAX25` | 25°C最大羧化速率 | `` | umol m-2 s-1 | pft-type | editable-expert | MOD_Const_PFT.F90 + include/pft_override_fields.inc |

## 站点（15 项）

| # | 参数 ID | 原始键 | 中文名 | 默认值 | 单位 | 作用域 | 可见性 | 源码位置 |
|---|---|---|---|---|---|---|---|---|
| 1 | `case:SITE_fsitedata` | `SITE_fsitedata` | SITE fsitedata | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:74 |
| 2 | `case:SITE_landtype` | `SITE_landtype` | SITE landtype | `-1` |  | case-scalar | editable-common | MOD_Namelist.F90:79 |
| 3 | `case:SITE_lat_location` | `SITE_lat_location` | SITE lat location | `-1.e36_r8` |  | case-scalar | editable-common | MOD_Namelist.F90:77 |
| 4 | `case:SITE_lon_location` | `SITE_lon_location` | SITE lon location | `-1.e36_r8` |  | case-scalar | editable-common | MOD_Namelist.F90:76 |
| 5 | `case:USE_SITE_ForcingReadAhead` | `USE_SITE_ForcingReadAhead` | USE SITE ForcingReadAhead | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:97 |
| 6 | `case:USE_SITE_LAI` | `USE_SITE_LAI` | USE SITE LAI | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:85 |
| 7 | `case:USE_SITE_dbedrock` | `USE_SITE_dbedrock` | USE SITE dbedrock | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:89 |
| 8 | `case:USE_SITE_htop` | `USE_SITE_htop` | USE SITE htop | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:84 |
| 9 | `case:USE_SITE_lakedepth` | `USE_SITE_lakedepth` | USE SITE lakedepth | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:86 |
| 10 | `case:USE_SITE_landtype` | `USE_SITE_landtype` | USE SITE landtype | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:81 |
| 11 | `case:USE_SITE_pctcrop` | `USE_SITE_pctcrop` | USE SITE pctcrop | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:83 |
| 12 | `case:USE_SITE_pctpfts` | `USE_SITE_pctpfts` | USE SITE pctpfts | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:82 |
| 13 | `case:USE_SITE_soilparameters` | `USE_SITE_soilparameters` | USE SITE soilparameters | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:88 |
| 14 | `case:USE_SITE_soilreflectance` | `USE_SITE_soilreflectance` | USE SITE soilreflectance | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:87 |
| 15 | `case:USE_SITE_topography` | `USE_SITE_topography` | USE SITE topography | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:90 |

## 算例（1 项）

| # | 参数 ID | 原始键 | 中文名 | 默认值 | 单位 | 作用域 | 可见性 | 源码位置 |
|---|---|---|---|---|---|---|---|---|
| 1 | `case:DEF_CASE_NAME` | `DEF_CASE_NAME` | DEF CASE NAME | `CASENAME` |  | case-scalar | editable-common | MOD_Namelist.F90:23 |

## 网格与并行（15 项）

| # | 参数 ID | 原始键 | 中文名 | 默认值 | 单位 | 作用域 | 可见性 | 源码位置 |
|---|---|---|---|---|---|---|---|---|
| 1 | `case:DEF_AverageElementSize` | `DEF_AverageElementSize` | DEF AverageElementSize | `-1.` |  | case-scalar | editable-common | MOD_Namelist.F90:61 |
| 2 | `case:DEF_BlockInfoFile` | `DEF_BlockInfoFile` | DEF BlockInfoFile | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:60 |
| 3 | `case:DEF_CatchmentMesh_data` | `DEF_CatchmentMesh_data` | DEF CatchmentMesh data | `path/to/catchment/data` |  | case-scalar | editable-common | MOD_Namelist.F90:143 |
| 4 | `case:DEF_GRIDBASED_lat_res` | `DEF_GRIDBASED_lat_res` | DEF GRIDBASED lat res | `0.5` |  | case-scalar | editable-common | MOD_Namelist.F90:141 |
| 5 | `case:DEF_GRIDBASED_lon_res` | `DEF_GRIDBASED_lon_res` | DEF GRIDBASED lon res | `0.5` |  | case-scalar | editable-common | MOD_Namelist.F90:140 |
| 6 | `case:DEF_PIO_groupsize` | `DEF_PIO_groupsize` | DEF PIO groupsize | `12` |  | case-scalar | editable-common | MOD_Namelist.F90:67 |
| 7 | `case:DEF_domain%edgee` | `DEF_domain%edgee` | DEF domain edgee | `180.0` |  | case-scalar | editable-common | MOD_Namelist.F90:46 |
| 8 | `case:DEF_domain%edgen` | `DEF_domain%edgen` | DEF domain edgen | `90.0` |  | case-scalar | editable-common | MOD_Namelist.F90:44 |
| 9 | `case:DEF_domain%edges` | `DEF_domain%edges` | DEF domain edges | `-90.0` |  | case-scalar | editable-common | MOD_Namelist.F90:43 |
| 10 | `case:DEF_domain%edgew` | `DEF_domain%edgew` | DEF domain edgew | `-180.0` |  | case-scalar | editable-common | MOD_Namelist.F90:45 |
| 11 | `case:DEF_file_mesh` | `DEF_file_mesh` | DEF file mesh | `path/to/mesh/file` |  | case-scalar | editable-common | MOD_Namelist.F90:139 |
| 12 | `case:DEF_file_mesh_filter` | `DEF_file_mesh_filter` | DEF file mesh filter | `path/to/mesh/filter` |  | case-scalar | editable-common | MOD_Namelist.F90:145 |
| 13 | `case:DEF_nIO_eq_nBlock` | `DEF_nIO_eq_nBlock` | DEF nIO eq nBlock | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:68 |
| 14 | `case:DEF_nx_blocks` | `DEF_nx_blocks` | DEF nx blocks | `72` |  | case-scalar | editable-common | MOD_Namelist.F90:62 |
| 15 | `case:DEF_ny_blocks` | `DEF_ny_blocks` | DEF ny blocks | `36` |  | case-scalar | editable-common | MOD_Namelist.F90:63 |

## 调试与诊断（3 项）

| # | 参数 ID | 原始键 | 中文名 | 默认值 | 单位 | 作用域 | 可见性 | 源码位置 |
|---|---|---|---|---|---|---|---|---|
| 1 | `case:DEF_USE_CoLMDEBUG` | `DEF_USE_CoLMDEBUG` | DEF USE CoLMDEBUG | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:34 |
| 2 | `case:DEF_USE_RangeCheck` | `DEF_USE_RangeCheck` | DEF USE RangeCheck | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:35 |
| 3 | `case:DEF_USE_SrfdataDiag` | `DEF_USE_SrfdataDiag` | DEF USE SrfdataDiag | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:36 |

## 输出与重启（19 项）

| # | 参数 ID | 原始键 | 中文名 | 默认值 | 单位 | 作用域 | 可见性 | 源码位置 |
|---|---|---|---|---|---|---|---|---|
| 1 | `case:DEF_HISTORY_IN_VECTOR` | `DEF_HISTORY_IN_VECTOR` | DEF HISTORY IN VECTOR | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:823 |
| 2 | `case:DEF_HIST_CompressLevel` | `DEF_HIST_CompressLevel` | DEF HIST CompressLevel | `1` |  | case-scalar | editable-common | MOD_Namelist.F90:836 |
| 3 | `case:DEF_HIST_FREQ` | `DEF_HIST_FREQ` | DEF HIST FREQ | `none` |  | case-scalar | editable-common | MOD_Namelist.F90:831 |
| 4 | `case:DEF_HIST_WriteBack` | `DEF_HIST_WriteBack` | DEF HIST WriteBack | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:834 |
| 5 | `case:DEF_HIST_grid_as_forcing` | `DEF_HIST_grid_as_forcing` | DEF HIST grid as forcing | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:826 |
| 6 | `case:DEF_HIST_grid_as_model_mesh` | `DEF_HIST_grid_as_model_mesh` | DEF HIST grid as model mesh | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:825 |
| 7 | `case:DEF_HIST_groupby` | `DEF_HIST_groupby` | DEF HIST groupby | `MONTH` |  | case-scalar | editable-common | MOD_Namelist.F90:832 |
| 8 | `case:DEF_HIST_lat_res` | `DEF_HIST_lat_res` | DEF HIST lat res | `0.5` |  | case-scalar | editable-common | MOD_Namelist.F90:828 |
| 9 | `case:DEF_HIST_lon_res` | `DEF_HIST_lon_res` | DEF HIST lon res | `0.5` |  | case-scalar | editable-common | MOD_Namelist.F90:827 |
| 10 | `case:DEF_HIST_mode` | `DEF_HIST_mode` | DEF HIST mode | `one` |  | case-scalar | editable-common | MOD_Namelist.F90:833 |
| 11 | `case:DEF_HIST_vars_namelist` | `DEF_HIST_vars_namelist` | DEF HIST vars namelist | `null` |  | case-scalar | editable-common | MOD_Namelist.F90:838 |
| 12 | `case:DEF_HIST_vars_out_default` | `DEF_HIST_vars_out_default` | DEF HIST vars out default | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:839 |
| 13 | `case:DEF_Output_2mWMO` | `DEF_Output_2mWMO` | DEF Output 2mWMO | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:714 |
| 14 | `case:DEF_REST_CompressLevel` | `DEF_REST_CompressLevel` | DEF REST CompressLevel | `1` |  | case-scalar | editable-common | MOD_Namelist.F90:835 |
| 15 | `case:DEF_WRST_FREQ` | `DEF_WRST_FREQ` | DEF WRST FREQ | `none` |  | case-scalar | editable-common | MOD_Namelist.F90:830 |
| 16 | `case:DEF_dir_history` | `DEF_dir_history` | DEF dir history | `path/to/history` |  | case-scalar | read-only-context | MOD_Namelist.F90:133 |
| 17 | `case:DEF_dir_output` | `DEF_dir_output` | DEF dir output | `path/to/output/data` |  | case-scalar | editable-common | MOD_Namelist.F90:129 |
| 18 | `case:DEF_dir_restart` | `DEF_dir_restart` | DEF dir restart | `path/to/restart` |  | case-scalar | read-only-context | MOD_Namelist.F90:132 |
| 19 | `case:USE_SITE_HistWriteBack` | `USE_SITE_HistWriteBack` | USE SITE HistWriteBack | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:96 |

## 输出变量（586 项）

| # | 参数 ID | 原始键 | 中文名 | 默认值 | 单位 | 作用域 | 可见性 | 源码位置 |
|---|---|---|---|---|---|---|---|---|
| 1 | `case:DEF_hist_vars%BD_all` | `DEF_hist_vars%BD_all` | DEF hist vars BD all | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1336 |
| 2 | `case:DEF_hist_vars%CONC_O2_UNSAT` | `DEF_hist_vars%CONC_O2_UNSAT` | DEF hist vars CONC O2 UNSAT | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1171 |
| 3 | `case:DEF_hist_vars%DA_t_brt_fy3d` | `DEF_hist_vars%DA_t_brt_fy3d` | DEF hist vars DA t brt fy3d | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1361 |
| 4 | `case:DEF_hist_vars%DA_t_brt_fy3d_a` | `DEF_hist_vars%DA_t_brt_fy3d_a` | DEF hist vars DA t brt fy3d a | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1362 |
| 5 | `case:DEF_hist_vars%DA_t_brt_fy3d_ens` | `DEF_hist_vars%DA_t_brt_fy3d_ens` | DEF hist vars DA t brt fy3d ens | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1363 |
| 6 | `case:DEF_hist_vars%DA_t_brt_fy3d_ens_std` | `DEF_hist_vars%DA_t_brt_fy3d_ens_std` | DEF hist vars DA t brt fy3d ens std | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1364 |
| 7 | `case:DEF_hist_vars%DA_t_brt_smap` | `DEF_hist_vars%DA_t_brt_smap` | DEF hist vars DA t brt smap | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1357 |
| 8 | `case:DEF_hist_vars%DA_t_brt_smap_a` | `DEF_hist_vars%DA_t_brt_smap_a` | DEF hist vars DA t brt smap a | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1358 |
| 9 | `case:DEF_hist_vars%DA_t_brt_smap_ens` | `DEF_hist_vars%DA_t_brt_smap_ens` | DEF hist vars DA t brt smap ens | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1359 |
| 10 | `case:DEF_hist_vars%DA_t_brt_smap_ens_std` | `DEF_hist_vars%DA_t_brt_smap_ens_std` | DEF hist vars DA t brt smap ens std | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1360 |
| 11 | `case:DEF_hist_vars%DA_t_soisno_5cm` | `DEF_hist_vars%DA_t_soisno_5cm` | DEF hist vars DA t soisno 5cm | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1351 |
| 12 | `case:DEF_hist_vars%DA_t_soisno_5cm_a` | `DEF_hist_vars%DA_t_soisno_5cm_a` | DEF hist vars DA t soisno 5cm a | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1352 |
| 13 | `case:DEF_hist_vars%DA_t_soisno_5cm_ens_std` | `DEF_hist_vars%DA_t_soisno_5cm_ens_std` | DEF hist vars DA t soisno 5cm ens std | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1356 |
| 14 | `case:DEF_hist_vars%DA_t_soisno_ens` | `DEF_hist_vars%DA_t_soisno_ens` | DEF hist vars DA t soisno ens | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1354 |
| 15 | `case:DEF_hist_vars%DA_wliq_h2osoi_5cm` | `DEF_hist_vars%DA_wliq_h2osoi_5cm` | DEF hist vars DA wliq h2osoi 5cm | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1349 |
| 16 | `case:DEF_hist_vars%DA_wliq_h2osoi_5cm_a` | `DEF_hist_vars%DA_wliq_h2osoi_5cm_a` | DEF hist vars DA wliq h2osoi 5cm a | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1350 |
| 17 | `case:DEF_hist_vars%DA_wliq_soisno_5cm_ens_std` | `DEF_hist_vars%DA_wliq_soisno_5cm_ens_std` | DEF hist vars DA wliq soisno 5cm ens std | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1355 |
| 18 | `case:DEF_hist_vars%DA_wliq_soisno_ens` | `DEF_hist_vars%DA_wliq_soisno_ens` | DEF hist vars DA wliq soisno ens | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1353 |
| 19 | `case:DEF_hist_vars%O2_DECOMP_DEPTH_UNSAT` | `DEF_hist_vars%O2_DECOMP_DEPTH_UNSAT` | DEF hist vars O2 DECOMP DEPTH UNSAT | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1172 |
| 20 | `case:DEF_hist_vars%OM_density` | `DEF_hist_vars%OM_density` | DEF hist vars OM density | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1338 |
| 21 | `case:DEF_hist_vars%abm` | `DEF_hist_vars%abm` | DEF hist vars abm | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1278 |
| 22 | `case:DEF_hist_vars%alb` | `DEF_hist_vars%alb` | DEF hist vars alb | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:909 |
| 23 | `case:DEF_hist_vars%alb_hires` | `DEF_hist_vars%alb_hires` | DEF hist vars alb hires | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:910 |
| 24 | `case:DEF_hist_vars%ar` | `DEF_hist_vars%ar` | DEF hist vars ar | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:995 |
| 25 | `case:DEF_hist_vars%assim` | `DEF_hist_vars%assim` | DEF hist vars assim | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:893 |
| 26 | `case:DEF_hist_vars%assimsha` | `DEF_hist_vars%assimsha` | DEF hist vars assimsha | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:943 |
| 27 | `case:DEF_hist_vars%assimsun` | `DEF_hist_vars%assimsun` | DEF hist vars assimsun | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:942 |
| 28 | `case:DEF_hist_vars%baf_crop` | `DEF_hist_vars%baf_crop` | DEF hist vars baf crop | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1175 |
| 29 | `case:DEF_hist_vars%baf_peatf` | `DEF_hist_vars%baf_peatf` | DEF hist vars baf peatf | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1176 |
| 30 | `case:DEF_hist_vars%bedout` | `DEF_hist_vars%bedout` | DEF hist vars bedout | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1449 |
| 31 | `case:DEF_hist_vars%btran2` | `DEF_hist_vars%btran2` | DEF hist vars btran2 | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1179 |
| 32 | `case:DEF_hist_vars%col_fire_closs` | `DEF_hist_vars%col_fire_closs` | DEF hist vars col fire closs | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1182 |
| 33 | `case:DEF_hist_vars%col_fire_nloss` | `DEF_hist_vars%col_fire_nloss` | DEF hist vars col fire nloss | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1183 |
| 34 | `case:DEF_hist_vars%cphase` | `DEF_hist_vars%cphase` | DEF hist vars cphase | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1083 |
| 35 | `case:DEF_hist_vars%cropprod1c` | `DEF_hist_vars%cropprod1c` | DEF hist vars cropprod1c | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1088 |
| 36 | `case:DEF_hist_vars%cropprod1c_loss` | `DEF_hist_vars%cropprod1c_loss` | DEF hist vars cropprod1c loss | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1089 |
| 37 | `case:DEF_hist_vars%cropprodc_irrigated_cotton` | `DEF_hist_vars%cropprodc_irrigated_cotton` | DEF hist vars cropprodc irrigated cotton | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1120 |
| 38 | `case:DEF_hist_vars%cropprodc_irrigated_rice` | `DEF_hist_vars%cropprodc_irrigated_rice` | DEF hist vars cropprodc irrigated rice | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1122 |
| 39 | `case:DEF_hist_vars%cropprodc_irrigated_spwheat` | `DEF_hist_vars%cropprodc_irrigated_spwheat` | DEF hist vars cropprodc irrigated spwheat | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1114 |
| 40 | `case:DEF_hist_vars%cropprodc_irrigated_sugarcane` | `DEF_hist_vars%cropprodc_irrigated_sugarcane` | DEF hist vars cropprodc irrigated sugarcane | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1124 |
| 41 | `case:DEF_hist_vars%cropprodc_irrigated_temp_corn` | `DEF_hist_vars%cropprodc_irrigated_temp_corn` | DEF hist vars cropprodc irrigated temp corn | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1112 |
| 42 | `case:DEF_hist_vars%cropprodc_irrigated_temp_soybean` | `DEF_hist_vars%cropprodc_irrigated_temp_soybean` | DEF hist vars cropprodc irrigated temp soybean | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1118 |
| 43 | `case:DEF_hist_vars%cropprodc_irrigated_trop_corn` | `DEF_hist_vars%cropprodc_irrigated_trop_corn` | DEF hist vars cropprodc irrigated trop corn | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1126 |
| 44 | `case:DEF_hist_vars%cropprodc_irrigated_trop_soybean` | `DEF_hist_vars%cropprodc_irrigated_trop_soybean` | DEF hist vars cropprodc irrigated trop soybean | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1128 |
| 45 | `case:DEF_hist_vars%cropprodc_irrigated_wtwheat` | `DEF_hist_vars%cropprodc_irrigated_wtwheat` | DEF hist vars cropprodc irrigated wtwheat | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1116 |
| 46 | `case:DEF_hist_vars%cropprodc_rainfed_cotton` | `DEF_hist_vars%cropprodc_rainfed_cotton` | DEF hist vars cropprodc rainfed cotton | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1119 |
| 47 | `case:DEF_hist_vars%cropprodc_rainfed_rice` | `DEF_hist_vars%cropprodc_rainfed_rice` | DEF hist vars cropprodc rainfed rice | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1121 |
| 48 | `case:DEF_hist_vars%cropprodc_rainfed_spwheat` | `DEF_hist_vars%cropprodc_rainfed_spwheat` | DEF hist vars cropprodc rainfed spwheat | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1113 |
| 49 | `case:DEF_hist_vars%cropprodc_rainfed_sugarcane` | `DEF_hist_vars%cropprodc_rainfed_sugarcane` | DEF hist vars cropprodc rainfed sugarcane | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1123 |
| 50 | `case:DEF_hist_vars%cropprodc_rainfed_temp_corn` | `DEF_hist_vars%cropprodc_rainfed_temp_corn` | DEF hist vars cropprodc rainfed temp corn | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1111 |
| 51 | `case:DEF_hist_vars%cropprodc_rainfed_temp_soybean` | `DEF_hist_vars%cropprodc_rainfed_temp_soybean` | DEF hist vars cropprodc rainfed temp soybean | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1117 |
| 52 | `case:DEF_hist_vars%cropprodc_rainfed_trop_corn` | `DEF_hist_vars%cropprodc_rainfed_trop_corn` | DEF hist vars cropprodc rainfed trop corn | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1125 |
| 53 | `case:DEF_hist_vars%cropprodc_rainfed_trop_soybean` | `DEF_hist_vars%cropprodc_rainfed_trop_soybean` | DEF hist vars cropprodc rainfed trop soybean | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1127 |
| 54 | `case:DEF_hist_vars%cropprodc_rainfed_wtwheat` | `DEF_hist_vars%cropprodc_rainfed_wtwheat` | DEF hist vars cropprodc rainfed wtwheat | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1115 |
| 55 | `case:DEF_hist_vars%cropprodc_unmanagedcrop` | `DEF_hist_vars%cropprodc_unmanagedcrop` | DEF hist vars cropprodc unmanagedcrop | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1129 |
| 56 | `case:DEF_hist_vars%cropseedc_deficit` | `DEF_hist_vars%cropseedc_deficit` | DEF hist vars cropseedc deficit | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1090 |
| 57 | `case:DEF_hist_vars%cwdcCap_vr` | `DEF_hist_vars%cwdcCap_vr` | DEF hist vars cwdcCap vr | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1387 |
| 58 | `case:DEF_hist_vars%cwdc_vr` | `DEF_hist_vars%cwdc_vr` | DEF hist vars cwdc vr | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1372 |
| 59 | `case:DEF_hist_vars%cwddecomp` | `DEF_hist_vars%cwddecomp` | DEF hist vars cwddecomp | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:997 |
| 60 | `case:DEF_hist_vars%cwdnCap_vr` | `DEF_hist_vars%cwdnCap_vr` | DEF hist vars cwdnCap vr | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1394 |
| 61 | `case:DEF_hist_vars%cwdn_vr` | `DEF_hist_vars%cwdn_vr` | DEF hist vars cwdn vr | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1379 |
| 62 | `case:DEF_hist_vars%cwdprod` | `DEF_hist_vars%cwdprod` | DEF hist vars cwdprod | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:996 |
| 63 | `case:DEF_hist_vars%deadcrootc` | `DEF_hist_vars%deadcrootc` | DEF hist vars deadcrootc | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:965 |
| 64 | `case:DEF_hist_vars%deadcrootcCap` | `DEF_hist_vars%deadcrootcCap` | DEF hist vars deadcrootcCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1299 |
| 65 | `case:DEF_hist_vars%deadcrootc_storage` | `DEF_hist_vars%deadcrootc_storage` | DEF hist vars deadcrootc storage | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:966 |
| 66 | `case:DEF_hist_vars%deadcrootc_storageCap` | `DEF_hist_vars%deadcrootc_storageCap` | DEF hist vars deadcrootc storageCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1300 |
| 67 | `case:DEF_hist_vars%deadcrootc_xfer` | `DEF_hist_vars%deadcrootc_xfer` | DEF hist vars deadcrootc xfer | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:967 |
| 68 | `case:DEF_hist_vars%deadcrootc_xferCap` | `DEF_hist_vars%deadcrootc_xferCap` | DEF hist vars deadcrootc xferCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1301 |
| 69 | `case:DEF_hist_vars%deadcrootn` | `DEF_hist_vars%deadcrootn` | DEF hist vars deadcrootn | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:986 |
| 70 | `case:DEF_hist_vars%deadcrootnCap` | `DEF_hist_vars%deadcrootnCap` | DEF hist vars deadcrootnCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1317 |
| 71 | `case:DEF_hist_vars%deadcrootn_storage` | `DEF_hist_vars%deadcrootn_storage` | DEF hist vars deadcrootn storage | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:987 |
| 72 | `case:DEF_hist_vars%deadcrootn_storageCap` | `DEF_hist_vars%deadcrootn_storageCap` | DEF hist vars deadcrootn storageCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1318 |
| 73 | `case:DEF_hist_vars%deadcrootn_xfer` | `DEF_hist_vars%deadcrootn_xfer` | DEF hist vars deadcrootn xfer | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:988 |
| 74 | `case:DEF_hist_vars%deadcrootn_xferCap` | `DEF_hist_vars%deadcrootn_xferCap` | DEF hist vars deadcrootn xferCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1319 |
| 75 | `case:DEF_hist_vars%deadstemc` | `DEF_hist_vars%deadstemc` | DEF hist vars deadstemc | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:959 |
| 76 | `case:DEF_hist_vars%deadstemcCap` | `DEF_hist_vars%deadstemcCap` | DEF hist vars deadstemcCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1293 |
| 77 | `case:DEF_hist_vars%deadstemc_storage` | `DEF_hist_vars%deadstemc_storage` | DEF hist vars deadstemc storage | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:960 |
| 78 | `case:DEF_hist_vars%deadstemc_storageCap` | `DEF_hist_vars%deadstemc_storageCap` | DEF hist vars deadstemc storageCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1294 |
| 79 | `case:DEF_hist_vars%deadstemc_xfer` | `DEF_hist_vars%deadstemc_xfer` | DEF hist vars deadstemc xfer | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:961 |
| 80 | `case:DEF_hist_vars%deadstemc_xferCap` | `DEF_hist_vars%deadstemc_xferCap` | DEF hist vars deadstemc xferCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1295 |
| 81 | `case:DEF_hist_vars%deadstemn` | `DEF_hist_vars%deadstemn` | DEF hist vars deadstemn | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:980 |
| 82 | `case:DEF_hist_vars%deadstemnCap` | `DEF_hist_vars%deadstemnCap` | DEF hist vars deadstemnCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1311 |
| 83 | `case:DEF_hist_vars%deadstemn_storage` | `DEF_hist_vars%deadstemn_storage` | DEF hist vars deadstemn storage | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:981 |
| 84 | `case:DEF_hist_vars%deadstemn_storageCap` | `DEF_hist_vars%deadstemn_storageCap` | DEF hist vars deadstemn storageCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1312 |
| 85 | `case:DEF_hist_vars%deadstemn_xfer` | `DEF_hist_vars%deadstemn_xfer` | DEF hist vars deadstemn xfer | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:982 |
| 86 | `case:DEF_hist_vars%deadstemn_xferCap` | `DEF_hist_vars%deadstemn_xferCap` | DEF hist vars deadstemn xferCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1313 |
| 87 | `case:DEF_hist_vars%discharge` | `DEF_hist_vars%discharge` | DEF hist vars discharge | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1438 |
| 88 | `case:DEF_hist_vars%downreg` | `DEF_hist_vars%downreg` | DEF hist vars downreg | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:994 |
| 89 | `case:DEF_hist_vars%dz_lake` | `DEF_hist_vars%dz_lake` | DEF hist vars dz lake | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1345 |
| 90 | `case:DEF_hist_vars%emis` | `DEF_hist_vars%emis` | DEF hist vars emis | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:913 |
| 91 | `case:DEF_hist_vars%etr` | `DEF_hist_vars%etr` | DEF hist vars etr | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:866 |
| 92 | `case:DEF_hist_vars%etrsha` | `DEF_hist_vars%etrsha` | DEF hist vars etrsha | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:945 |
| 93 | `case:DEF_hist_vars%etrsun` | `DEF_hist_vars%etrsun` | DEF hist vars etrsun | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:944 |
| 94 | `case:DEF_hist_vars%fach` | `DEF_hist_vars%fach` | DEF hist vars fach | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:933 |
| 95 | `case:DEF_hist_vars%farea_burned` | `DEF_hist_vars%farea_burned` | DEF hist vars farea burned | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1174 |
| 96 | `case:DEF_hist_vars%fert_to_sminn` | `DEF_hist_vars%fert_to_sminn` | DEF hist vars fert to sminn | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1132 |
| 97 | `case:DEF_hist_vars%fertnitro_corn` | `DEF_hist_vars%fertnitro_corn` | DEF hist vars fertnitro corn | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1144 |
| 98 | `case:DEF_hist_vars%fertnitro_cotton` | `DEF_hist_vars%fertnitro_cotton` | DEF hist vars fertnitro cotton | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1148 |
| 99 | `case:DEF_hist_vars%fertnitro_rice1` | `DEF_hist_vars%fertnitro_rice1` | DEF hist vars fertnitro rice1 | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1149 |
| 100 | `case:DEF_hist_vars%fertnitro_rice2` | `DEF_hist_vars%fertnitro_rice2` | DEF hist vars fertnitro rice2 | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1150 |
| 101 | `case:DEF_hist_vars%fertnitro_soybean` | `DEF_hist_vars%fertnitro_soybean` | DEF hist vars fertnitro soybean | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1147 |
| 102 | `case:DEF_hist_vars%fertnitro_sugarcane` | `DEF_hist_vars%fertnitro_sugarcane` | DEF hist vars fertnitro sugarcane | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1151 |
| 103 | `case:DEF_hist_vars%fertnitro_swheat` | `DEF_hist_vars%fertnitro_swheat` | DEF hist vars fertnitro swheat | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1145 |
| 104 | `case:DEF_hist_vars%fertnitro_wwheat` | `DEF_hist_vars%fertnitro_wwheat` | DEF hist vars fertnitro wwheat | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1146 |
| 105 | `case:DEF_hist_vars%fevpa` | `DEF_hist_vars%fevpa` | DEF hist vars fevpa | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:863 |
| 106 | `case:DEF_hist_vars%fevpg` | `DEF_hist_vars%fevpg` | DEF hist vars fevpg | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:868 |
| 107 | `case:DEF_hist_vars%fevpl` | `DEF_hist_vars%fevpl` | DEF hist vars fevpl | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:865 |
| 108 | `case:DEF_hist_vars%fgrnd` | `DEF_hist_vars%fgrnd` | DEF hist vars fgrnd | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:869 |
| 109 | `case:DEF_hist_vars%fh` | `DEF_hist_vars%fh` | DEF hist vars fh | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1405 |
| 110 | `case:DEF_hist_vars%fhac` | `DEF_hist_vars%fhac` | DEF hist vars fhac | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:931 |
| 111 | `case:DEF_hist_vars%fhah` | `DEF_hist_vars%fhah` | DEF hist vars fhah | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:934 |
| 112 | `case:DEF_hist_vars%fldarea` | `DEF_hist_vars%fldarea` | DEF hist vars fldarea | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:884 |
| 113 | `case:DEF_hist_vars%floodarea` | `DEF_hist_vars%floodarea` | DEF hist vars floodarea | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1439 |
| 114 | `case:DEF_hist_vars%floodfrc` | `DEF_hist_vars%floodfrc` | DEF hist vars floodfrc | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1440 |
| 115 | `case:DEF_hist_vars%fm` | `DEF_hist_vars%fm` | DEF hist vars fm | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1404 |
| 116 | `case:DEF_hist_vars%fm10m` | `DEF_hist_vars%fm10m` | DEF hist vars fm10m | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1409 |
| 117 | `case:DEF_hist_vars%fpg` | `DEF_hist_vars%fpg` | DEF hist vars fpg | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:999 |
| 118 | `case:DEF_hist_vars%fpi` | `DEF_hist_vars%fpi` | DEF hist vars fpi | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1000 |
| 119 | `case:DEF_hist_vars%fq` | `DEF_hist_vars%fq` | DEF hist vars fq | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1406 |
| 120 | `case:DEF_hist_vars%frcsat` | `DEF_hist_vars%frcsat` | DEF hist vars frcsat | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:902 |
| 121 | `case:DEF_hist_vars%frootc` | `DEF_hist_vars%frootc` | DEF hist vars frootc | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:953 |
| 122 | `case:DEF_hist_vars%frootcCap` | `DEF_hist_vars%frootcCap` | DEF hist vars frootcCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1287 |
| 123 | `case:DEF_hist_vars%frootc_storage` | `DEF_hist_vars%frootc_storage` | DEF hist vars frootc storage | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:954 |
| 124 | `case:DEF_hist_vars%frootc_storageCap` | `DEF_hist_vars%frootc_storageCap` | DEF hist vars frootc storageCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1288 |
| 125 | `case:DEF_hist_vars%frootc_xfer` | `DEF_hist_vars%frootc_xfer` | DEF hist vars frootc xfer | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:955 |
| 126 | `case:DEF_hist_vars%frootc_xferCap` | `DEF_hist_vars%frootc_xferCap` | DEF hist vars frootc xferCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1289 |
| 127 | `case:DEF_hist_vars%frootn` | `DEF_hist_vars%frootn` | DEF hist vars frootn | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:974 |
| 128 | `case:DEF_hist_vars%frootnCap` | `DEF_hist_vars%frootnCap` | DEF hist vars frootnCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1305 |
| 129 | `case:DEF_hist_vars%frootn_storage` | `DEF_hist_vars%frootn_storage` | DEF hist vars frootn storage | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:975 |
| 130 | `case:DEF_hist_vars%frootn_storageCap` | `DEF_hist_vars%frootn_storageCap` | DEF hist vars frootn storageCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1306 |
| 131 | `case:DEF_hist_vars%frootn_xfer` | `DEF_hist_vars%frootn_xfer` | DEF hist vars frootn xfer | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:976 |
| 132 | `case:DEF_hist_vars%frootn_xferCap` | `DEF_hist_vars%frootn_xferCap` | DEF hist vars frootn xferCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1307 |
| 133 | `case:DEF_hist_vars%fsen_gimp` | `DEF_hist_vars%fsen_gimp` | DEF hist vars fsen gimp | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:924 |
| 134 | `case:DEF_hist_vars%fsen_gper` | `DEF_hist_vars%fsen_gper` | DEF hist vars fsen gper | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:925 |
| 135 | `case:DEF_hist_vars%fsen_roof` | `DEF_hist_vars%fsen_roof` | DEF hist vars fsen roof | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:921 |
| 136 | `case:DEF_hist_vars%fsen_urbl` | `DEF_hist_vars%fsen_urbl` | DEF hist vars fsen urbl | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:926 |
| 137 | `case:DEF_hist_vars%fsen_wsha` | `DEF_hist_vars%fsen_wsha` | DEF hist vars fsen wsha | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:923 |
| 138 | `case:DEF_hist_vars%fsen_wsun` | `DEF_hist_vars%fsen_wsun` | DEF hist vars fsen wsun | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:922 |
| 139 | `case:DEF_hist_vars%fsena` | `DEF_hist_vars%fsena` | DEF hist vars fsena | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:861 |
| 140 | `case:DEF_hist_vars%fseng` | `DEF_hist_vars%fseng` | DEF hist vars fseng | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:867 |
| 141 | `case:DEF_hist_vars%fsenl` | `DEF_hist_vars%fsenl` | DEF hist vars fsenl | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:864 |
| 142 | `case:DEF_hist_vars%fsno` | `DEF_hist_vars%fsno` | DEF hist vars fsno | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:901 |
| 143 | `case:DEF_hist_vars%fuelc` | `DEF_hist_vars%fuelc` | DEF hist vars fuelc | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1178 |
| 144 | `case:DEF_hist_vars%fwst` | `DEF_hist_vars%fwst` | DEF hist vars fwst | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:932 |
| 145 | `case:DEF_hist_vars%gddmaturity` | `DEF_hist_vars%gddmaturity` | DEF hist vars gddmaturity | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1084 |
| 146 | `case:DEF_hist_vars%gddplant` | `DEF_hist_vars%gddplant` | DEF hist vars gddplant | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1085 |
| 147 | `case:DEF_hist_vars%gdp` | `DEF_hist_vars%gdp` | DEF hist vars gdp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1279 |
| 148 | `case:DEF_hist_vars%gpp` | `DEF_hist_vars%gpp` | DEF hist vars gpp | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:993 |
| 149 | `case:DEF_hist_vars%gpp_c3arcgrass` | `DEF_hist_vars%gpp_c3arcgrass` | DEF hist vars gpp c3arcgrass | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1023 |
| 150 | `case:DEF_hist_vars%gpp_c3grass` | `DEF_hist_vars%gpp_c3grass` | DEF hist vars gpp c3grass | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1024 |
| 151 | `case:DEF_hist_vars%gpp_c4grass` | `DEF_hist_vars%gpp_c4grass` | DEF hist vars gpp c4grass | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1025 |
| 152 | `case:DEF_hist_vars%gpp_dbfboreal` | `DEF_hist_vars%gpp_dbfboreal` | DEF hist vars gpp dbfboreal | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1019 |
| 153 | `case:DEF_hist_vars%gpp_dbftemp` | `DEF_hist_vars%gpp_dbftemp` | DEF hist vars gpp dbftemp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1018 |
| 154 | `case:DEF_hist_vars%gpp_dbftrop` | `DEF_hist_vars%gpp_dbftrop` | DEF hist vars gpp dbftrop | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1017 |
| 155 | `case:DEF_hist_vars%gpp_dbsboreal` | `DEF_hist_vars%gpp_dbsboreal` | DEF hist vars gpp dbsboreal | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1022 |
| 156 | `case:DEF_hist_vars%gpp_dbstemp` | `DEF_hist_vars%gpp_dbstemp` | DEF hist vars gpp dbstemp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1021 |
| 157 | `case:DEF_hist_vars%gpp_dnfboreal` | `DEF_hist_vars%gpp_dnfboreal` | DEF hist vars gpp dnfboreal | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1014 |
| 158 | `case:DEF_hist_vars%gpp_ebftemp` | `DEF_hist_vars%gpp_ebftemp` | DEF hist vars gpp ebftemp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1016 |
| 159 | `case:DEF_hist_vars%gpp_ebftrop` | `DEF_hist_vars%gpp_ebftrop` | DEF hist vars gpp ebftrop | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1015 |
| 160 | `case:DEF_hist_vars%gpp_ebstemp` | `DEF_hist_vars%gpp_ebstemp` | DEF hist vars gpp ebstemp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1020 |
| 161 | `case:DEF_hist_vars%gpp_enfboreal` | `DEF_hist_vars%gpp_enfboreal` | DEF hist vars gpp enfboreal | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1013 |
| 162 | `case:DEF_hist_vars%gpp_enftemp` | `DEF_hist_vars%gpp_enftemp` | DEF hist vars gpp enftemp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1012 |
| 163 | `case:DEF_hist_vars%grainc` | `DEF_hist_vars%grainc` | DEF hist vars grainc | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:968 |
| 164 | `case:DEF_hist_vars%grainc_storage` | `DEF_hist_vars%grainc_storage` | DEF hist vars grainc storage | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:969 |
| 165 | `case:DEF_hist_vars%grainc_to_cropprodc` | `DEF_hist_vars%grainc_to_cropprodc` | DEF hist vars grainc to cropprodc | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1091 |
| 166 | `case:DEF_hist_vars%grainc_to_seed` | `DEF_hist_vars%grainc_to_seed` | DEF hist vars grainc to seed | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1131 |
| 167 | `case:DEF_hist_vars%grainc_xfer` | `DEF_hist_vars%grainc_xfer` | DEF hist vars grainc xfer | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:970 |
| 168 | `case:DEF_hist_vars%grainn` | `DEF_hist_vars%grainn` | DEF hist vars grainn | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:989 |
| 169 | `case:DEF_hist_vars%grainn_storage` | `DEF_hist_vars%grainn_storage` | DEF hist vars grainn storage | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:990 |
| 170 | `case:DEF_hist_vars%grainn_xfer` | `DEF_hist_vars%grainn_xfer` | DEF hist vars grainn xfer | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:991 |
| 171 | `case:DEF_hist_vars%green` | `DEF_hist_vars%green` | DEF hist vars green | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:904 |
| 172 | `case:DEF_hist_vars%groundwater_demand` | `DEF_hist_vars%groundwater_demand` | DEF hist vars groundwater demand | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1165 |
| 173 | `case:DEF_hist_vars%groundwater_supply` | `DEF_hist_vars%groundwater_supply` | DEF hist vars groundwater supply | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1166 |
| 174 | `case:DEF_hist_vars%gssha` | `DEF_hist_vars%gssha` | DEF hist vars gssha | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1333 |
| 175 | `case:DEF_hist_vars%gssun` | `DEF_hist_vars%gssun` | DEF hist vars gssun | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1332 |
| 176 | `case:DEF_hist_vars%h2osoi` | `DEF_hist_vars%h2osoi` | DEF hist vars h2osoi | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1327 |
| 177 | `case:DEF_hist_vars%hdm` | `DEF_hist_vars%hdm` | DEF hist vars hdm | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1281 |
| 178 | `case:DEF_hist_vars%hr` | `DEF_hist_vars%hr` | DEF hist vars hr | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:998 |
| 179 | `case:DEF_hist_vars%hui` | `DEF_hist_vars%hui` | DEF hist vars hui | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1087 |
| 180 | `case:DEF_hist_vars%huiswheat` | `DEF_hist_vars%huiswheat` | DEF hist vars huiswheat | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1134 |
| 181 | `case:DEF_hist_vars%irrig_method_corn` | `DEF_hist_vars%irrig_method_corn` | DEF hist vars irrig method corn | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1152 |
| 182 | `case:DEF_hist_vars%irrig_method_cotton` | `DEF_hist_vars%irrig_method_cotton` | DEF hist vars irrig method cotton | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1156 |
| 183 | `case:DEF_hist_vars%irrig_method_rice1` | `DEF_hist_vars%irrig_method_rice1` | DEF hist vars irrig method rice1 | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1157 |
| 184 | `case:DEF_hist_vars%irrig_method_rice2` | `DEF_hist_vars%irrig_method_rice2` | DEF hist vars irrig method rice2 | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1158 |
| 185 | `case:DEF_hist_vars%irrig_method_soybean` | `DEF_hist_vars%irrig_method_soybean` | DEF hist vars irrig method soybean | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1155 |
| 186 | `case:DEF_hist_vars%irrig_method_sugarcane` | `DEF_hist_vars%irrig_method_sugarcane` | DEF hist vars irrig method sugarcane | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1159 |
| 187 | `case:DEF_hist_vars%irrig_method_swheat` | `DEF_hist_vars%irrig_method_swheat` | DEF hist vars irrig method swheat | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1153 |
| 188 | `case:DEF_hist_vars%irrig_method_wwheat` | `DEF_hist_vars%irrig_method_wwheat` | DEF hist vars irrig method wwheat | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1154 |
| 189 | `case:DEF_hist_vars%lai` | `DEF_hist_vars%lai` | DEF hist vars lai | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:905 |
| 190 | `case:DEF_hist_vars%lai_c3arcgrass` | `DEF_hist_vars%lai_c3arcgrass` | DEF hist vars lai c3arcgrass | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1051 |
| 191 | `case:DEF_hist_vars%lai_c3grass` | `DEF_hist_vars%lai_c3grass` | DEF hist vars lai c3grass | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1052 |
| 192 | `case:DEF_hist_vars%lai_c4grass` | `DEF_hist_vars%lai_c4grass` | DEF hist vars lai c4grass | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1053 |
| 193 | `case:DEF_hist_vars%lai_dbfboreal` | `DEF_hist_vars%lai_dbfboreal` | DEF hist vars lai dbfboreal | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1047 |
| 194 | `case:DEF_hist_vars%lai_dbftemp` | `DEF_hist_vars%lai_dbftemp` | DEF hist vars lai dbftemp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1046 |
| 195 | `case:DEF_hist_vars%lai_dbftrop` | `DEF_hist_vars%lai_dbftrop` | DEF hist vars lai dbftrop | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1045 |
| 196 | `case:DEF_hist_vars%lai_dbsboreal` | `DEF_hist_vars%lai_dbsboreal` | DEF hist vars lai dbsboreal | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1050 |
| 197 | `case:DEF_hist_vars%lai_dbstemp` | `DEF_hist_vars%lai_dbstemp` | DEF hist vars lai dbstemp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1049 |
| 198 | `case:DEF_hist_vars%lai_dnfboreal` | `DEF_hist_vars%lai_dnfboreal` | DEF hist vars lai dnfboreal | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1042 |
| 199 | `case:DEF_hist_vars%lai_ebftemp` | `DEF_hist_vars%lai_ebftemp` | DEF hist vars lai ebftemp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1044 |
| 200 | `case:DEF_hist_vars%lai_ebftrop` | `DEF_hist_vars%lai_ebftrop` | DEF hist vars lai ebftrop | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1043 |
| 201 | `case:DEF_hist_vars%lai_ebstemp` | `DEF_hist_vars%lai_ebstemp` | DEF hist vars lai ebstemp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1048 |
| 202 | `case:DEF_hist_vars%lai_enfboreal` | `DEF_hist_vars%lai_enfboreal` | DEF hist vars lai enfboreal | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1041 |
| 203 | `case:DEF_hist_vars%lai_enftemp` | `DEF_hist_vars%lai_enftemp` | DEF hist vars lai enftemp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1040 |
| 204 | `case:DEF_hist_vars%laisha` | `DEF_hist_vars%laisha` | DEF hist vars laisha | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:907 |
| 205 | `case:DEF_hist_vars%laisun` | `DEF_hist_vars%laisun` | DEF hist vars laisun | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:906 |
| 206 | `case:DEF_hist_vars%lake_deficit` | `DEF_hist_vars%lake_deficit` | DEF hist vars lake deficit | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1329 |
| 207 | `case:DEF_hist_vars%lake_icefrac` | `DEF_hist_vars%lake_icefrac` | DEF hist vars lake icefrac | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1347 |
| 208 | `case:DEF_hist_vars%ldew` | `DEF_hist_vars%ldew` | DEF hist vars ldew | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:898 |
| 209 | `case:DEF_hist_vars%leafc` | `DEF_hist_vars%leafc` | DEF hist vars leafc | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:950 |
| 210 | `case:DEF_hist_vars%leafcCap` | `DEF_hist_vars%leafcCap` | DEF hist vars leafcCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1284 |
| 211 | `case:DEF_hist_vars%leafc_c3arcgrass` | `DEF_hist_vars%leafc_c3arcgrass` | DEF hist vars leafc c3arcgrass | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1037 |
| 212 | `case:DEF_hist_vars%leafc_c3grass` | `DEF_hist_vars%leafc_c3grass` | DEF hist vars leafc c3grass | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1038 |
| 213 | `case:DEF_hist_vars%leafc_c4grass` | `DEF_hist_vars%leafc_c4grass` | DEF hist vars leafc c4grass | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1039 |
| 214 | `case:DEF_hist_vars%leafc_dbfboreal` | `DEF_hist_vars%leafc_dbfboreal` | DEF hist vars leafc dbfboreal | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1033 |
| 215 | `case:DEF_hist_vars%leafc_dbftemp` | `DEF_hist_vars%leafc_dbftemp` | DEF hist vars leafc dbftemp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1032 |
| 216 | `case:DEF_hist_vars%leafc_dbftrop` | `DEF_hist_vars%leafc_dbftrop` | DEF hist vars leafc dbftrop | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1031 |
| 217 | `case:DEF_hist_vars%leafc_dbsboreal` | `DEF_hist_vars%leafc_dbsboreal` | DEF hist vars leafc dbsboreal | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1036 |
| 218 | `case:DEF_hist_vars%leafc_dbstemp` | `DEF_hist_vars%leafc_dbstemp` | DEF hist vars leafc dbstemp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1035 |
| 219 | `case:DEF_hist_vars%leafc_dnfboreal` | `DEF_hist_vars%leafc_dnfboreal` | DEF hist vars leafc dnfboreal | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1028 |
| 220 | `case:DEF_hist_vars%leafc_ebftemp` | `DEF_hist_vars%leafc_ebftemp` | DEF hist vars leafc ebftemp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1030 |
| 221 | `case:DEF_hist_vars%leafc_ebftrop` | `DEF_hist_vars%leafc_ebftrop` | DEF hist vars leafc ebftrop | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1029 |
| 222 | `case:DEF_hist_vars%leafc_ebstemp` | `DEF_hist_vars%leafc_ebstemp` | DEF hist vars leafc ebstemp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1034 |
| 223 | `case:DEF_hist_vars%leafc_enfboreal` | `DEF_hist_vars%leafc_enfboreal` | DEF hist vars leafc enfboreal | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1027 |
| 224 | `case:DEF_hist_vars%leafc_enftemp` | `DEF_hist_vars%leafc_enftemp` | DEF hist vars leafc enftemp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1026 |
| 225 | `case:DEF_hist_vars%leafc_storage` | `DEF_hist_vars%leafc_storage` | DEF hist vars leafc storage | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:951 |
| 226 | `case:DEF_hist_vars%leafc_storageCap` | `DEF_hist_vars%leafc_storageCap` | DEF hist vars leafc storageCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1285 |
| 227 | `case:DEF_hist_vars%leafc_xfer` | `DEF_hist_vars%leafc_xfer` | DEF hist vars leafc xfer | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:952 |
| 228 | `case:DEF_hist_vars%leafc_xferCap` | `DEF_hist_vars%leafc_xferCap` | DEF hist vars leafc xferCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1286 |
| 229 | `case:DEF_hist_vars%leafn` | `DEF_hist_vars%leafn` | DEF hist vars leafn | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:971 |
| 230 | `case:DEF_hist_vars%leafnCap` | `DEF_hist_vars%leafnCap` | DEF hist vars leafnCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1302 |
| 231 | `case:DEF_hist_vars%leafn_storage` | `DEF_hist_vars%leafn_storage` | DEF hist vars leafn storage | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:972 |
| 232 | `case:DEF_hist_vars%leafn_storageCap` | `DEF_hist_vars%leafn_storageCap` | DEF hist vars leafn storageCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1303 |
| 233 | `case:DEF_hist_vars%leafn_xfer` | `DEF_hist_vars%leafn_xfer` | DEF hist vars leafn xfer | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:973 |
| 234 | `case:DEF_hist_vars%leafn_xferCap` | `DEF_hist_vars%leafn_xferCap` | DEF hist vars leafn xferCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1304 |
| 235 | `case:DEF_hist_vars%lfevp_gimp` | `DEF_hist_vars%lfevp_gimp` | DEF hist vars lfevp gimp | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:928 |
| 236 | `case:DEF_hist_vars%lfevp_gper` | `DEF_hist_vars%lfevp_gper` | DEF hist vars lfevp gper | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:929 |
| 237 | `case:DEF_hist_vars%lfevp_roof` | `DEF_hist_vars%lfevp_roof` | DEF hist vars lfevp roof | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:927 |
| 238 | `case:DEF_hist_vars%lfevp_urbl` | `DEF_hist_vars%lfevp_urbl` | DEF hist vars lfevp urbl | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:930 |
| 239 | `case:DEF_hist_vars%lfevpa` | `DEF_hist_vars%lfevpa` | DEF hist vars lfevpa | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:862 |
| 240 | `case:DEF_hist_vars%litfire` | `DEF_hist_vars%litfire` | DEF hist vars litfire | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1185 |
| 241 | `case:DEF_hist_vars%litr1cCap_vr` | `DEF_hist_vars%litr1cCap_vr` | DEF hist vars litr1cCap vr | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1381 |
| 242 | `case:DEF_hist_vars%litr1c_vr` | `DEF_hist_vars%litr1c_vr` | DEF hist vars litr1c vr | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1366 |
| 243 | `case:DEF_hist_vars%litr1nCap_vr` | `DEF_hist_vars%litr1nCap_vr` | DEF hist vars litr1nCap vr | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1388 |
| 244 | `case:DEF_hist_vars%litr1n_vr` | `DEF_hist_vars%litr1n_vr` | DEF hist vars litr1n vr | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1373 |
| 245 | `case:DEF_hist_vars%litr2cCap_vr` | `DEF_hist_vars%litr2cCap_vr` | DEF hist vars litr2cCap vr | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1382 |
| 246 | `case:DEF_hist_vars%litr2c_vr` | `DEF_hist_vars%litr2c_vr` | DEF hist vars litr2c vr | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1367 |
| 247 | `case:DEF_hist_vars%litr2nCap_vr` | `DEF_hist_vars%litr2nCap_vr` | DEF hist vars litr2nCap vr | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1389 |
| 248 | `case:DEF_hist_vars%litr2n_vr` | `DEF_hist_vars%litr2n_vr` | DEF hist vars litr2n vr | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1374 |
| 249 | `case:DEF_hist_vars%litr3cCap_vr` | `DEF_hist_vars%litr3cCap_vr` | DEF hist vars litr3cCap vr | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1383 |
| 250 | `case:DEF_hist_vars%litr3c_vr` | `DEF_hist_vars%litr3c_vr` | DEF hist vars litr3c vr | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1368 |
| 251 | `case:DEF_hist_vars%litr3nCap_vr` | `DEF_hist_vars%litr3nCap_vr` | DEF hist vars litr3nCap vr | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1390 |
| 252 | `case:DEF_hist_vars%litr3n_vr` | `DEF_hist_vars%litr3n_vr` | DEF hist vars litr3n vr | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1375 |
| 253 | `case:DEF_hist_vars%livecrootc` | `DEF_hist_vars%livecrootc` | DEF hist vars livecrootc | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:962 |
| 254 | `case:DEF_hist_vars%livecrootcCap` | `DEF_hist_vars%livecrootcCap` | DEF hist vars livecrootcCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1296 |
| 255 | `case:DEF_hist_vars%livecrootc_storage` | `DEF_hist_vars%livecrootc_storage` | DEF hist vars livecrootc storage | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:963 |
| 256 | `case:DEF_hist_vars%livecrootc_storageCap` | `DEF_hist_vars%livecrootc_storageCap` | DEF hist vars livecrootc storageCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1297 |
| 257 | `case:DEF_hist_vars%livecrootc_xfer` | `DEF_hist_vars%livecrootc_xfer` | DEF hist vars livecrootc xfer | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:964 |
| 258 | `case:DEF_hist_vars%livecrootc_xferCap` | `DEF_hist_vars%livecrootc_xferCap` | DEF hist vars livecrootc xferCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1298 |
| 259 | `case:DEF_hist_vars%livecrootn` | `DEF_hist_vars%livecrootn` | DEF hist vars livecrootn | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:983 |
| 260 | `case:DEF_hist_vars%livecrootnCap` | `DEF_hist_vars%livecrootnCap` | DEF hist vars livecrootnCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1314 |
| 261 | `case:DEF_hist_vars%livecrootn_storage` | `DEF_hist_vars%livecrootn_storage` | DEF hist vars livecrootn storage | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:984 |
| 262 | `case:DEF_hist_vars%livecrootn_storageCap` | `DEF_hist_vars%livecrootn_storageCap` | DEF hist vars livecrootn storageCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1315 |
| 263 | `case:DEF_hist_vars%livecrootn_xfer` | `DEF_hist_vars%livecrootn_xfer` | DEF hist vars livecrootn xfer | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:985 |
| 264 | `case:DEF_hist_vars%livecrootn_xferCap` | `DEF_hist_vars%livecrootn_xferCap` | DEF hist vars livecrootn xferCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1316 |
| 265 | `case:DEF_hist_vars%livestemc` | `DEF_hist_vars%livestemc` | DEF hist vars livestemc | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:956 |
| 266 | `case:DEF_hist_vars%livestemcCap` | `DEF_hist_vars%livestemcCap` | DEF hist vars livestemcCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1290 |
| 267 | `case:DEF_hist_vars%livestemc_storage` | `DEF_hist_vars%livestemc_storage` | DEF hist vars livestemc storage | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:957 |
| 268 | `case:DEF_hist_vars%livestemc_storageCap` | `DEF_hist_vars%livestemc_storageCap` | DEF hist vars livestemc storageCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1291 |
| 269 | `case:DEF_hist_vars%livestemc_xfer` | `DEF_hist_vars%livestemc_xfer` | DEF hist vars livestemc xfer | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:958 |
| 270 | `case:DEF_hist_vars%livestemc_xferCap` | `DEF_hist_vars%livestemc_xferCap` | DEF hist vars livestemc xferCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1292 |
| 271 | `case:DEF_hist_vars%livestemn` | `DEF_hist_vars%livestemn` | DEF hist vars livestemn | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:977 |
| 272 | `case:DEF_hist_vars%livestemnCap` | `DEF_hist_vars%livestemnCap` | DEF hist vars livestemnCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1308 |
| 273 | `case:DEF_hist_vars%livestemn_storage` | `DEF_hist_vars%livestemn_storage` | DEF hist vars livestemn storage | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:978 |
| 274 | `case:DEF_hist_vars%livestemn_storageCap` | `DEF_hist_vars%livestemn_storageCap` | DEF hist vars livestemn storageCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1309 |
| 275 | `case:DEF_hist_vars%livestemn_xfer` | `DEF_hist_vars%livestemn_xfer` | DEF hist vars livestemn xfer | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:979 |
| 276 | `case:DEF_hist_vars%livestemn_xferCap` | `DEF_hist_vars%livestemn_xferCap` | DEF hist vars livestemn xferCap | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1310 |
| 277 | `case:DEF_hist_vars%lnfm` | `DEF_hist_vars%lnfm` | DEF hist vars lnfm | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1282 |
| 278 | `case:DEF_hist_vars%m_cwd_c_to_fire` | `DEF_hist_vars%m_cwd_c_to_fire` | DEF hist vars m cwd c to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1276 |
| 279 | `case:DEF_hist_vars%m_cwd_n_to_fire` | `DEF_hist_vars%m_cwd_n_to_fire` | DEF hist vars m cwd n to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1277 |
| 280 | `case:DEF_hist_vars%m_deadcrootc_storage_to_fire` | `DEF_hist_vars%m_deadcrootc_storage_to_fire` | DEF hist vars m deadcrootc storage to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1199 |
| 281 | `case:DEF_hist_vars%m_deadcrootc_storage_to_litter_fire` | `DEF_hist_vars%m_deadcrootc_storage_to_litter_fire` | DEF hist vars m deadcrootc storage to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1221 |
| 282 | `case:DEF_hist_vars%m_deadcrootc_to_fire` | `DEF_hist_vars%m_deadcrootc_to_fire` | DEF hist vars m deadcrootc to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1193 |
| 283 | `case:DEF_hist_vars%m_deadcrootc_to_litter_fire` | `DEF_hist_vars%m_deadcrootc_to_litter_fire` | DEF hist vars m deadcrootc to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1215 |
| 284 | `case:DEF_hist_vars%m_deadcrootc_xfer_to_fire` | `DEF_hist_vars%m_deadcrootc_xfer_to_fire` | DEF hist vars m deadcrootc xfer to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1206 |
| 285 | `case:DEF_hist_vars%m_deadcrootc_xfer_to_litter_fire` | `DEF_hist_vars%m_deadcrootc_xfer_to_litter_fire` | DEF hist vars m deadcrootc xfer to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1228 |
| 286 | `case:DEF_hist_vars%m_deadcrootn_storage_to_fire` | `DEF_hist_vars%m_deadcrootn_storage_to_fire` | DEF hist vars m deadcrootn storage to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1241 |
| 287 | `case:DEF_hist_vars%m_deadcrootn_storage_to_litter_fire` | `DEF_hist_vars%m_deadcrootn_storage_to_litter_fire` | DEF hist vars m deadcrootn storage to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1262 |
| 288 | `case:DEF_hist_vars%m_deadcrootn_to_fire` | `DEF_hist_vars%m_deadcrootn_to_fire` | DEF hist vars m deadcrootn to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1235 |
| 289 | `case:DEF_hist_vars%m_deadcrootn_to_litter_fire` | `DEF_hist_vars%m_deadcrootn_to_litter_fire` | DEF hist vars m deadcrootn to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1256 |
| 290 | `case:DEF_hist_vars%m_deadcrootn_xfer_to_fire` | `DEF_hist_vars%m_deadcrootn_xfer_to_fire` | DEF hist vars m deadcrootn xfer to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1247 |
| 291 | `case:DEF_hist_vars%m_deadcrootn_xfer_to_litter_fire` | `DEF_hist_vars%m_deadcrootn_xfer_to_litter_fire` | DEF hist vars m deadcrootn xfer to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1268 |
| 292 | `case:DEF_hist_vars%m_deadstemc_storage_to_fire` | `DEF_hist_vars%m_deadstemc_storage_to_fire` | DEF hist vars m deadstemc storage to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1197 |
| 293 | `case:DEF_hist_vars%m_deadstemc_storage_to_litter_fire` | `DEF_hist_vars%m_deadstemc_storage_to_litter_fire` | DEF hist vars m deadstemc storage to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1219 |
| 294 | `case:DEF_hist_vars%m_deadstemc_to_fire` | `DEF_hist_vars%m_deadstemc_to_fire` | DEF hist vars m deadstemc to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1191 |
| 295 | `case:DEF_hist_vars%m_deadstemc_to_litter_fire` | `DEF_hist_vars%m_deadstemc_to_litter_fire` | DEF hist vars m deadstemc to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1213 |
| 296 | `case:DEF_hist_vars%m_deadstemc_xfer_to_fire` | `DEF_hist_vars%m_deadstemc_xfer_to_fire` | DEF hist vars m deadstemc xfer to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1204 |
| 297 | `case:DEF_hist_vars%m_deadstemc_xfer_to_litter_fire` | `DEF_hist_vars%m_deadstemc_xfer_to_litter_fire` | DEF hist vars m deadstemc xfer to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1226 |
| 298 | `case:DEF_hist_vars%m_deadstemn_storage_to_fire` | `DEF_hist_vars%m_deadstemn_storage_to_fire` | DEF hist vars m deadstemn storage to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1239 |
| 299 | `case:DEF_hist_vars%m_deadstemn_storage_to_litter_fire` | `DEF_hist_vars%m_deadstemn_storage_to_litter_fire` | DEF hist vars m deadstemn storage to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1260 |
| 300 | `case:DEF_hist_vars%m_deadstemn_to_fire` | `DEF_hist_vars%m_deadstemn_to_fire` | DEF hist vars m deadstemn to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1233 |
| 301 | `case:DEF_hist_vars%m_deadstemn_to_litter_fire` | `DEF_hist_vars%m_deadstemn_to_litter_fire` | DEF hist vars m deadstemn to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1254 |
| 302 | `case:DEF_hist_vars%m_deadstemn_xfer_to_fire` | `DEF_hist_vars%m_deadstemn_xfer_to_fire` | DEF hist vars m deadstemn xfer to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1245 |
| 303 | `case:DEF_hist_vars%m_deadstemn_xfer_to_litter_fire` | `DEF_hist_vars%m_deadstemn_xfer_to_litter_fire` | DEF hist vars m deadstemn xfer to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1266 |
| 304 | `case:DEF_hist_vars%m_frootc_storage_to_fire` | `DEF_hist_vars%m_frootc_storage_to_fire` | DEF hist vars m frootc storage to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1195 |
| 305 | `case:DEF_hist_vars%m_frootc_storage_to_litter_fire` | `DEF_hist_vars%m_frootc_storage_to_litter_fire` | DEF hist vars m frootc storage to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1217 |
| 306 | `case:DEF_hist_vars%m_frootc_to_fire` | `DEF_hist_vars%m_frootc_to_fire` | DEF hist vars m frootc to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1189 |
| 307 | `case:DEF_hist_vars%m_frootc_to_litter_fire` | `DEF_hist_vars%m_frootc_to_litter_fire` | DEF hist vars m frootc to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1211 |
| 308 | `case:DEF_hist_vars%m_frootc_xfer_to_fire` | `DEF_hist_vars%m_frootc_xfer_to_fire` | DEF hist vars m frootc xfer to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1202 |
| 309 | `case:DEF_hist_vars%m_frootc_xfer_to_litter_fire` | `DEF_hist_vars%m_frootc_xfer_to_litter_fire` | DEF hist vars m frootc xfer to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1224 |
| 310 | `case:DEF_hist_vars%m_frootn_storage_to_fire` | `DEF_hist_vars%m_frootn_storage_to_fire` | DEF hist vars m frootn storage to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1237 |
| 311 | `case:DEF_hist_vars%m_frootn_storage_to_litter_fire` | `DEF_hist_vars%m_frootn_storage_to_litter_fire` | DEF hist vars m frootn storage to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1258 |
| 312 | `case:DEF_hist_vars%m_frootn_to_fire` | `DEF_hist_vars%m_frootn_to_fire` | DEF hist vars m frootn to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1231 |
| 313 | `case:DEF_hist_vars%m_frootn_to_litter_fire` | `DEF_hist_vars%m_frootn_to_litter_fire` | DEF hist vars m frootn to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1252 |
| 314 | `case:DEF_hist_vars%m_frootn_xfer_to_fire` | `DEF_hist_vars%m_frootn_xfer_to_fire` | DEF hist vars m frootn xfer to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1243 |
| 315 | `case:DEF_hist_vars%m_frootn_xfer_to_litter_fire` | `DEF_hist_vars%m_frootn_xfer_to_litter_fire` | DEF hist vars m frootn xfer to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1264 |
| 316 | `case:DEF_hist_vars%m_gresp_storage_to_fire` | `DEF_hist_vars%m_gresp_storage_to_fire` | DEF hist vars m gresp storage to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1200 |
| 317 | `case:DEF_hist_vars%m_gresp_storage_to_litter_fire` | `DEF_hist_vars%m_gresp_storage_to_litter_fire` | DEF hist vars m gresp storage to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1222 |
| 318 | `case:DEF_hist_vars%m_gresp_xfer_to_fire` | `DEF_hist_vars%m_gresp_xfer_to_fire` | DEF hist vars m gresp xfer to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1207 |
| 319 | `case:DEF_hist_vars%m_gresp_xfer_to_litter_fire` | `DEF_hist_vars%m_gresp_xfer_to_litter_fire` | DEF hist vars m gresp xfer to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1229 |
| 320 | `case:DEF_hist_vars%m_leafc_storage_to_fire` | `DEF_hist_vars%m_leafc_storage_to_fire` | DEF hist vars m leafc storage to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1194 |
| 321 | `case:DEF_hist_vars%m_leafc_storage_to_litter_fire` | `DEF_hist_vars%m_leafc_storage_to_litter_fire` | DEF hist vars m leafc storage to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1216 |
| 322 | `case:DEF_hist_vars%m_leafc_to_fire` | `DEF_hist_vars%m_leafc_to_fire` | DEF hist vars m leafc to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1188 |
| 323 | `case:DEF_hist_vars%m_leafc_to_litter_fire` | `DEF_hist_vars%m_leafc_to_litter_fire` | DEF hist vars m leafc to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1210 |
| 324 | `case:DEF_hist_vars%m_leafc_xfer_to_fire` | `DEF_hist_vars%m_leafc_xfer_to_fire` | DEF hist vars m leafc xfer to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1201 |
| 325 | `case:DEF_hist_vars%m_leafc_xfer_to_litter_fire` | `DEF_hist_vars%m_leafc_xfer_to_litter_fire` | DEF hist vars m leafc xfer to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1223 |
| 326 | `case:DEF_hist_vars%m_leafn_storage_to_fire` | `DEF_hist_vars%m_leafn_storage_to_fire` | DEF hist vars m leafn storage to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1236 |
| 327 | `case:DEF_hist_vars%m_leafn_storage_to_litter_fire` | `DEF_hist_vars%m_leafn_storage_to_litter_fire` | DEF hist vars m leafn storage to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1257 |
| 328 | `case:DEF_hist_vars%m_leafn_to_fire` | `DEF_hist_vars%m_leafn_to_fire` | DEF hist vars m leafn to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1230 |
| 329 | `case:DEF_hist_vars%m_leafn_to_litter_fire` | `DEF_hist_vars%m_leafn_to_litter_fire` | DEF hist vars m leafn to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1251 |
| 330 | `case:DEF_hist_vars%m_leafn_xfer_to_fire` | `DEF_hist_vars%m_leafn_xfer_to_fire` | DEF hist vars m leafn xfer to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1242 |
| 331 | `case:DEF_hist_vars%m_leafn_xfer_to_litter_fire` | `DEF_hist_vars%m_leafn_xfer_to_litter_fire` | DEF hist vars m leafn xfer to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1263 |
| 332 | `case:DEF_hist_vars%m_litr1_c_to_fire` | `DEF_hist_vars%m_litr1_c_to_fire` | DEF hist vars m litr1 c to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1270 |
| 333 | `case:DEF_hist_vars%m_litr1_n_to_fire` | `DEF_hist_vars%m_litr1_n_to_fire` | DEF hist vars m litr1 n to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1271 |
| 334 | `case:DEF_hist_vars%m_litr2_c_to_fire` | `DEF_hist_vars%m_litr2_c_to_fire` | DEF hist vars m litr2 c to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1272 |
| 335 | `case:DEF_hist_vars%m_litr2_n_to_fire` | `DEF_hist_vars%m_litr2_n_to_fire` | DEF hist vars m litr2 n to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1273 |
| 336 | `case:DEF_hist_vars%m_litr3_c_to_fire` | `DEF_hist_vars%m_litr3_c_to_fire` | DEF hist vars m litr3 c to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1274 |
| 337 | `case:DEF_hist_vars%m_litr3_n_to_fire` | `DEF_hist_vars%m_litr3_n_to_fire` | DEF hist vars m litr3 n to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1275 |
| 338 | `case:DEF_hist_vars%m_livecrootc_storage_to_fire` | `DEF_hist_vars%m_livecrootc_storage_to_fire` | DEF hist vars m livecrootc storage to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1198 |
| 339 | `case:DEF_hist_vars%m_livecrootc_storage_to_litter_fire` | `DEF_hist_vars%m_livecrootc_storage_to_litter_fire` | DEF hist vars m livecrootc storage to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1220 |
| 340 | `case:DEF_hist_vars%m_livecrootc_to_deadcrootc_fire` | `DEF_hist_vars%m_livecrootc_to_deadcrootc_fire` | DEF hist vars m livecrootc to deadcrootc fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1209 |
| 341 | `case:DEF_hist_vars%m_livecrootc_to_fire` | `DEF_hist_vars%m_livecrootc_to_fire` | DEF hist vars m livecrootc to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1192 |
| 342 | `case:DEF_hist_vars%m_livecrootc_to_litter_fire` | `DEF_hist_vars%m_livecrootc_to_litter_fire` | DEF hist vars m livecrootc to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1214 |
| 343 | `case:DEF_hist_vars%m_livecrootc_xfer_to_fire` | `DEF_hist_vars%m_livecrootc_xfer_to_fire` | DEF hist vars m livecrootc xfer to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1205 |
| 344 | `case:DEF_hist_vars%m_livecrootc_xfer_to_litter_fire` | `DEF_hist_vars%m_livecrootc_xfer_to_litter_fire` | DEF hist vars m livecrootc xfer to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1227 |
| 345 | `case:DEF_hist_vars%m_livecrootn_storage_to_fire` | `DEF_hist_vars%m_livecrootn_storage_to_fire` | DEF hist vars m livecrootn storage to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1240 |
| 346 | `case:DEF_hist_vars%m_livecrootn_storage_to_litter_fire` | `DEF_hist_vars%m_livecrootn_storage_to_litter_fire` | DEF hist vars m livecrootn storage to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1261 |
| 347 | `case:DEF_hist_vars%m_livecrootn_to_deadcrootn_fire` | `DEF_hist_vars%m_livecrootn_to_deadcrootn_fire` | DEF hist vars m livecrootn to deadcrootn fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1249 |
| 348 | `case:DEF_hist_vars%m_livecrootn_to_fire` | `DEF_hist_vars%m_livecrootn_to_fire` | DEF hist vars m livecrootn to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1234 |
| 349 | `case:DEF_hist_vars%m_livecrootn_to_litter_fire` | `DEF_hist_vars%m_livecrootn_to_litter_fire` | DEF hist vars m livecrootn to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1255 |
| 350 | `case:DEF_hist_vars%m_livecrootn_xfer_to_fire` | `DEF_hist_vars%m_livecrootn_xfer_to_fire` | DEF hist vars m livecrootn xfer to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1246 |
| 351 | `case:DEF_hist_vars%m_livecrootn_xfer_to_litter_fire` | `DEF_hist_vars%m_livecrootn_xfer_to_litter_fire` | DEF hist vars m livecrootn xfer to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1267 |
| 352 | `case:DEF_hist_vars%m_livestemc_storage_to_fire` | `DEF_hist_vars%m_livestemc_storage_to_fire` | DEF hist vars m livestemc storage to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1196 |
| 353 | `case:DEF_hist_vars%m_livestemc_storage_to_litter_fire` | `DEF_hist_vars%m_livestemc_storage_to_litter_fire` | DEF hist vars m livestemc storage to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1218 |
| 354 | `case:DEF_hist_vars%m_livestemc_to_deadstemc_fire` | `DEF_hist_vars%m_livestemc_to_deadstemc_fire` | DEF hist vars m livestemc to deadstemc fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1208 |
| 355 | `case:DEF_hist_vars%m_livestemc_to_fire` | `DEF_hist_vars%m_livestemc_to_fire` | DEF hist vars m livestemc to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1190 |
| 356 | `case:DEF_hist_vars%m_livestemc_to_litter_fire` | `DEF_hist_vars%m_livestemc_to_litter_fire` | DEF hist vars m livestemc to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1212 |
| 357 | `case:DEF_hist_vars%m_livestemc_xfer_to_fire` | `DEF_hist_vars%m_livestemc_xfer_to_fire` | DEF hist vars m livestemc xfer to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1203 |
| 358 | `case:DEF_hist_vars%m_livestemc_xfer_to_litter_fire` | `DEF_hist_vars%m_livestemc_xfer_to_litter_fire` | DEF hist vars m livestemc xfer to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1225 |
| 359 | `case:DEF_hist_vars%m_livestemn_storage_to_fire` | `DEF_hist_vars%m_livestemn_storage_to_fire` | DEF hist vars m livestemn storage to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1238 |
| 360 | `case:DEF_hist_vars%m_livestemn_storage_to_litter_fire` | `DEF_hist_vars%m_livestemn_storage_to_litter_fire` | DEF hist vars m livestemn storage to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1259 |
| 361 | `case:DEF_hist_vars%m_livestemn_to_deadstemn_fire` | `DEF_hist_vars%m_livestemn_to_deadstemn_fire` | DEF hist vars m livestemn to deadstemn fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1248 |
| 362 | `case:DEF_hist_vars%m_livestemn_to_fire` | `DEF_hist_vars%m_livestemn_to_fire` | DEF hist vars m livestemn to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1232 |
| 363 | `case:DEF_hist_vars%m_livestemn_to_litter_fire` | `DEF_hist_vars%m_livestemn_to_litter_fire` | DEF hist vars m livestemn to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1253 |
| 364 | `case:DEF_hist_vars%m_livestemn_xfer_to_fire` | `DEF_hist_vars%m_livestemn_xfer_to_fire` | DEF hist vars m livestemn xfer to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1244 |
| 365 | `case:DEF_hist_vars%m_livestemn_xfer_to_litter_fire` | `DEF_hist_vars%m_livestemn_xfer_to_litter_fire` | DEF hist vars m livestemn xfer to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1265 |
| 366 | `case:DEF_hist_vars%m_retransn_to_fire` | `DEF_hist_vars%m_retransn_to_fire` | DEF hist vars m retransn to fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1250 |
| 367 | `case:DEF_hist_vars%m_retransn_to_litter_fire` | `DEF_hist_vars%m_retransn_to_litter_fire` | DEF hist vars m retransn to litter fire | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1269 |
| 368 | `case:DEF_hist_vars%manunitro` | `DEF_hist_vars%manunitro` | DEF hist vars manunitro | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1143 |
| 369 | `case:DEF_hist_vars%meta` | `DEF_hist_vars%meta` | DEF hist vars meta | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:935 |
| 370 | `case:DEF_hist_vars%ndep_to_sminn` | `DEF_hist_vars%ndep_to_sminn` | DEF hist vars ndep to sminn | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1170 |
| 371 | `case:DEF_hist_vars%netflw` | `DEF_hist_vars%netflw` | DEF hist vars netflw | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1451 |
| 372 | `case:DEF_hist_vars%nfire` | `DEF_hist_vars%nfire` | DEF hist vars nfire | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1177 |
| 373 | `case:DEF_hist_vars%npp_c3arcgrass` | `DEF_hist_vars%npp_c3arcgrass` | DEF hist vars npp c3arcgrass | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1065 |
| 374 | `case:DEF_hist_vars%npp_c3grass` | `DEF_hist_vars%npp_c3grass` | DEF hist vars npp c3grass | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1066 |
| 375 | `case:DEF_hist_vars%npp_c4grass` | `DEF_hist_vars%npp_c4grass` | DEF hist vars npp c4grass | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1067 |
| 376 | `case:DEF_hist_vars%npp_dbfboreal` | `DEF_hist_vars%npp_dbfboreal` | DEF hist vars npp dbfboreal | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1061 |
| 377 | `case:DEF_hist_vars%npp_dbftemp` | `DEF_hist_vars%npp_dbftemp` | DEF hist vars npp dbftemp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1060 |
| 378 | `case:DEF_hist_vars%npp_dbftrop` | `DEF_hist_vars%npp_dbftrop` | DEF hist vars npp dbftrop | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1059 |
| 379 | `case:DEF_hist_vars%npp_dbsboreal` | `DEF_hist_vars%npp_dbsboreal` | DEF hist vars npp dbsboreal | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1064 |
| 380 | `case:DEF_hist_vars%npp_dbstemp` | `DEF_hist_vars%npp_dbstemp` | DEF hist vars npp dbstemp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1063 |
| 381 | `case:DEF_hist_vars%npp_dnfboreal` | `DEF_hist_vars%npp_dnfboreal` | DEF hist vars npp dnfboreal | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1056 |
| 382 | `case:DEF_hist_vars%npp_ebftemp` | `DEF_hist_vars%npp_ebftemp` | DEF hist vars npp ebftemp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1058 |
| 383 | `case:DEF_hist_vars%npp_ebftrop` | `DEF_hist_vars%npp_ebftrop` | DEF hist vars npp ebftrop | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1057 |
| 384 | `case:DEF_hist_vars%npp_ebstemp` | `DEF_hist_vars%npp_ebstemp` | DEF hist vars npp ebstemp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1062 |
| 385 | `case:DEF_hist_vars%npp_enfboreal` | `DEF_hist_vars%npp_enfboreal` | DEF hist vars npp enfboreal | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1055 |
| 386 | `case:DEF_hist_vars%npp_enftemp` | `DEF_hist_vars%npp_enftemp` | DEF hist vars npp enftemp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1054 |
| 387 | `case:DEF_hist_vars%npptoleafc_c3arcgrass` | `DEF_hist_vars%npptoleafc_c3arcgrass` | DEF hist vars npptoleafc c3arcgrass | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1079 |
| 388 | `case:DEF_hist_vars%npptoleafc_c3grass` | `DEF_hist_vars%npptoleafc_c3grass` | DEF hist vars npptoleafc c3grass | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1080 |
| 389 | `case:DEF_hist_vars%npptoleafc_c4grass` | `DEF_hist_vars%npptoleafc_c4grass` | DEF hist vars npptoleafc c4grass | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1081 |
| 390 | `case:DEF_hist_vars%npptoleafc_dbfboreal` | `DEF_hist_vars%npptoleafc_dbfboreal` | DEF hist vars npptoleafc dbfboreal | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1075 |
| 391 | `case:DEF_hist_vars%npptoleafc_dbftemp` | `DEF_hist_vars%npptoleafc_dbftemp` | DEF hist vars npptoleafc dbftemp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1074 |
| 392 | `case:DEF_hist_vars%npptoleafc_dbftrop` | `DEF_hist_vars%npptoleafc_dbftrop` | DEF hist vars npptoleafc dbftrop | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1073 |
| 393 | `case:DEF_hist_vars%npptoleafc_dbsboreal` | `DEF_hist_vars%npptoleafc_dbsboreal` | DEF hist vars npptoleafc dbsboreal | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1078 |
| 394 | `case:DEF_hist_vars%npptoleafc_dbstemp` | `DEF_hist_vars%npptoleafc_dbstemp` | DEF hist vars npptoleafc dbstemp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1077 |
| 395 | `case:DEF_hist_vars%npptoleafc_dnfboreal` | `DEF_hist_vars%npptoleafc_dnfboreal` | DEF hist vars npptoleafc dnfboreal | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1070 |
| 396 | `case:DEF_hist_vars%npptoleafc_ebftemp` | `DEF_hist_vars%npptoleafc_ebftemp` | DEF hist vars npptoleafc ebftemp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1072 |
| 397 | `case:DEF_hist_vars%npptoleafc_ebftrop` | `DEF_hist_vars%npptoleafc_ebftrop` | DEF hist vars npptoleafc ebftrop | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1071 |
| 398 | `case:DEF_hist_vars%npptoleafc_ebstemp` | `DEF_hist_vars%npptoleafc_ebstemp` | DEF hist vars npptoleafc ebstemp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1076 |
| 399 | `case:DEF_hist_vars%npptoleafc_enfboreal` | `DEF_hist_vars%npptoleafc_enfboreal` | DEF hist vars npptoleafc enfboreal | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1069 |
| 400 | `case:DEF_hist_vars%npptoleafc_enftemp` | `DEF_hist_vars%npptoleafc_enftemp` | DEF hist vars npptoleafc enftemp | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1068 |
| 401 | `case:DEF_hist_vars%o3uptakesha` | `DEF_hist_vars%o3uptakesha` | DEF hist vars o3uptakesha | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:948 |
| 402 | `case:DEF_hist_vars%o3uptakesun` | `DEF_hist_vars%o3uptakesun` | DEF hist vars o3uptakesun | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:947 |
| 403 | `case:DEF_hist_vars%olrg` | `DEF_hist_vars%olrg` | DEF hist vars olrg | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:873 |
| 404 | `case:DEF_hist_vars%pdcorn` | `DEF_hist_vars%pdcorn` | DEF hist vars pdcorn | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1135 |
| 405 | `case:DEF_hist_vars%pdcotton` | `DEF_hist_vars%pdcotton` | DEF hist vars pdcotton | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1139 |
| 406 | `case:DEF_hist_vars%pdrice1` | `DEF_hist_vars%pdrice1` | DEF hist vars pdrice1 | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1140 |
| 407 | `case:DEF_hist_vars%pdrice2` | `DEF_hist_vars%pdrice2` | DEF hist vars pdrice2 | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1141 |
| 408 | `case:DEF_hist_vars%pdsoybean` | `DEF_hist_vars%pdsoybean` | DEF hist vars pdsoybean | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1138 |
| 409 | `case:DEF_hist_vars%pdsugarcane` | `DEF_hist_vars%pdsugarcane` | DEF hist vars pdsugarcane | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1142 |
| 410 | `case:DEF_hist_vars%pdswheat` | `DEF_hist_vars%pdswheat` | DEF hist vars pdswheat | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1136 |
| 411 | `case:DEF_hist_vars%pdwwheat` | `DEF_hist_vars%pdwwheat` | DEF hist vars pdwwheat | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1137 |
| 412 | `case:DEF_hist_vars%peatf` | `DEF_hist_vars%peatf` | DEF hist vars peatf | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1280 |
| 413 | `case:DEF_hist_vars%pft_fire_closs` | `DEF_hist_vars%pft_fire_closs` | DEF hist vars pft fire closs | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1180 |
| 414 | `case:DEF_hist_vars%pft_fire_nloss` | `DEF_hist_vars%pft_fire_nloss` | DEF hist vars pft fire nloss | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1181 |
| 415 | `case:DEF_hist_vars%plantdate_irrigated_cotton` | `DEF_hist_vars%plantdate_irrigated_cotton` | DEF hist vars plantdate irrigated cotton | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1101 |
| 416 | `case:DEF_hist_vars%plantdate_irrigated_rice` | `DEF_hist_vars%plantdate_irrigated_rice` | DEF hist vars plantdate irrigated rice | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1103 |
| 417 | `case:DEF_hist_vars%plantdate_irrigated_spwheat` | `DEF_hist_vars%plantdate_irrigated_spwheat` | DEF hist vars plantdate irrigated spwheat | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1095 |
| 418 | `case:DEF_hist_vars%plantdate_irrigated_sugarcane` | `DEF_hist_vars%plantdate_irrigated_sugarcane` | DEF hist vars plantdate irrigated sugarcane | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1105 |
| 419 | `case:DEF_hist_vars%plantdate_irrigated_temp_corn` | `DEF_hist_vars%plantdate_irrigated_temp_corn` | DEF hist vars plantdate irrigated temp corn | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1093 |
| 420 | `case:DEF_hist_vars%plantdate_irrigated_temp_soybean` | `DEF_hist_vars%plantdate_irrigated_temp_soybean` | DEF hist vars plantdate irrigated temp soybean | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1099 |
| 421 | `case:DEF_hist_vars%plantdate_irrigated_trop_corn` | `DEF_hist_vars%plantdate_irrigated_trop_corn` | DEF hist vars plantdate irrigated trop corn | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1107 |
| 422 | `case:DEF_hist_vars%plantdate_irrigated_trop_soybean` | `DEF_hist_vars%plantdate_irrigated_trop_soybean` | DEF hist vars plantdate irrigated trop soybean | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1109 |
| 423 | `case:DEF_hist_vars%plantdate_irrigated_wtwheat` | `DEF_hist_vars%plantdate_irrigated_wtwheat` | DEF hist vars plantdate irrigated wtwheat | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1097 |
| 424 | `case:DEF_hist_vars%plantdate_rainfed_cotton` | `DEF_hist_vars%plantdate_rainfed_cotton` | DEF hist vars plantdate rainfed cotton | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1100 |
| 425 | `case:DEF_hist_vars%plantdate_rainfed_rice` | `DEF_hist_vars%plantdate_rainfed_rice` | DEF hist vars plantdate rainfed rice | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1102 |
| 426 | `case:DEF_hist_vars%plantdate_rainfed_spwheat` | `DEF_hist_vars%plantdate_rainfed_spwheat` | DEF hist vars plantdate rainfed spwheat | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1094 |
| 427 | `case:DEF_hist_vars%plantdate_rainfed_sugarcane` | `DEF_hist_vars%plantdate_rainfed_sugarcane` | DEF hist vars plantdate rainfed sugarcane | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1104 |
| 428 | `case:DEF_hist_vars%plantdate_rainfed_temp_corn` | `DEF_hist_vars%plantdate_rainfed_temp_corn` | DEF hist vars plantdate rainfed temp corn | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1092 |
| 429 | `case:DEF_hist_vars%plantdate_rainfed_temp_soybean` | `DEF_hist_vars%plantdate_rainfed_temp_soybean` | DEF hist vars plantdate rainfed temp soybean | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1098 |
| 430 | `case:DEF_hist_vars%plantdate_rainfed_trop_corn` | `DEF_hist_vars%plantdate_rainfed_trop_corn` | DEF hist vars plantdate rainfed trop corn | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1106 |
| 431 | `case:DEF_hist_vars%plantdate_rainfed_trop_soybean` | `DEF_hist_vars%plantdate_rainfed_trop_soybean` | DEF hist vars plantdate rainfed trop soybean | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1108 |
| 432 | `case:DEF_hist_vars%plantdate_rainfed_wtwheat` | `DEF_hist_vars%plantdate_rainfed_wtwheat` | DEF hist vars plantdate rainfed wtwheat | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1096 |
| 433 | `case:DEF_hist_vars%plantdate_unmanagedcrop` | `DEF_hist_vars%plantdate_unmanagedcrop` | DEF hist vars plantdate unmanagedcrop | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1110 |
| 434 | `case:DEF_hist_vars%qcharge` | `DEF_hist_vars%qcharge` | DEF hist vars qcharge | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:895 |
| 435 | `case:DEF_hist_vars%qdrip` | `DEF_hist_vars%qdrip` | DEF hist vars qdrip | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:887 |
| 436 | `case:DEF_hist_vars%qinfl` | `DEF_hist_vars%qinfl` | DEF hist vars qinfl | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:886 |
| 437 | `case:DEF_hist_vars%qintr` | `DEF_hist_vars%qintr` | DEF hist vars qintr | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:885 |
| 438 | `case:DEF_hist_vars%qlayer` | `DEF_hist_vars%qlayer` | DEF hist vars qlayer | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1328 |
| 439 | `case:DEF_hist_vars%qref` | `DEF_hist_vars%qref` | DEF hist vars qref | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:919 |
| 440 | `case:DEF_hist_vars%qresv_in` | `DEF_hist_vars%qresv_in` | DEF hist vars qresv in | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1444 |
| 441 | `case:DEF_hist_vars%qresv_out` | `DEF_hist_vars%qresv_out` | DEF hist vars qresv out | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1445 |
| 442 | `case:DEF_hist_vars%qstar` | `DEF_hist_vars%qstar` | DEF hist vars qstar | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1401 |
| 443 | `case:DEF_hist_vars%reflectance_out` | `DEF_hist_vars%reflectance_out` | DEF hist vars reflectance out | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:911 |
| 444 | `case:DEF_hist_vars%reservoirriver_demand` | `DEF_hist_vars%reservoirriver_demand` | DEF hist vars reservoirriver demand | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1167 |
| 445 | `case:DEF_hist_vars%reservoirriver_supply` | `DEF_hist_vars%reservoirriver_supply` | DEF hist vars reservoirriver supply | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1168 |
| 446 | `case:DEF_hist_vars%respc` | `DEF_hist_vars%respc` | DEF hist vars respc | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:894 |
| 447 | `case:DEF_hist_vars%retrasn` | `DEF_hist_vars%retrasn` | DEF hist vars retrasn | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:992 |
| 448 | `case:DEF_hist_vars%rib` | `DEF_hist_vars%rib` | DEF hist vars rib | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1403 |
| 449 | `case:DEF_hist_vars%riv_height` | `DEF_hist_vars%riv_height` | DEF hist vars riv height | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1436 |
| 450 | `case:DEF_hist_vars%riv_veloct` | `DEF_hist_vars%riv_veloct` | DEF hist vars riv veloct | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1437 |
| 451 | `case:DEF_hist_vars%rnet` | `DEF_hist_vars%rnet` | DEF hist vars rnet | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:874 |
| 452 | `case:DEF_hist_vars%rnof` | `DEF_hist_vars%rnof` | DEF hist vars rnof | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:881 |
| 453 | `case:DEF_hist_vars%rootr` | `DEF_hist_vars%rootr` | DEF hist vars rootr | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1334 |
| 454 | `case:DEF_hist_vars%rss` | `DEF_hist_vars%rss` | DEF hist vars rss | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:916 |
| 455 | `case:DEF_hist_vars%rstfacsha` | `DEF_hist_vars%rstfacsha` | DEF hist vars rstfacsha | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1331 |
| 456 | `case:DEF_hist_vars%rstfacsun` | `DEF_hist_vars%rstfacsun` | DEF hist vars rstfacsun | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1330 |
| 457 | `case:DEF_hist_vars%rsub` | `DEF_hist_vars%rsub` | DEF hist vars rsub | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:880 |
| 458 | `case:DEF_hist_vars%rsur` | `DEF_hist_vars%rsur` | DEF hist vars rsur | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:877 |
| 459 | `case:DEF_hist_vars%rsur_ie` | `DEF_hist_vars%rsur_ie` | DEF hist vars rsur ie | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:879 |
| 460 | `case:DEF_hist_vars%rsur_se` | `DEF_hist_vars%rsur_se` | DEF hist vars rsur se | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:878 |
| 461 | `case:DEF_hist_vars%sabg` | `DEF_hist_vars%sabg` | DEF hist vars sabg | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:872 |
| 462 | `case:DEF_hist_vars%sabvsha` | `DEF_hist_vars%sabvsha` | DEF hist vars sabvsha | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:871 |
| 463 | `case:DEF_hist_vars%sabvsun` | `DEF_hist_vars%sabvsun` | DEF hist vars sabvsun | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:870 |
| 464 | `case:DEF_hist_vars%sai` | `DEF_hist_vars%sai` | DEF hist vars sai | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:908 |
| 465 | `case:DEF_hist_vars%scv` | `DEF_hist_vars%scv` | DEF hist vars scv | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:899 |
| 466 | `case:DEF_hist_vars%sedcon` | `DEF_hist_vars%sedcon` | DEF hist vars sedcon | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1447 |
| 467 | `case:DEF_hist_vars%sedinp` | `DEF_hist_vars%sedinp` | DEF hist vars sedinp | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1450 |
| 468 | `case:DEF_hist_vars%sedlayer` | `DEF_hist_vars%sedlayer` | DEF hist vars sedlayer | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1452 |
| 469 | `case:DEF_hist_vars%sedout` | `DEF_hist_vars%sedout` | DEF hist vars sedout | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1448 |
| 470 | `case:DEF_hist_vars%sensors` | `DEF_hist_vars%sensors` | DEF hist vars sensors | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1455 |
| 471 | `case:DEF_hist_vars%shearvel` | `DEF_hist_vars%shearvel` | DEF hist vars shearvel | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1453 |
| 472 | `case:DEF_hist_vars%sigf` | `DEF_hist_vars%sigf` | DEF hist vars sigf | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:903 |
| 473 | `case:DEF_hist_vars%sminn_vr` | `DEF_hist_vars%sminn_vr` | DEF hist vars sminn vr | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1396 |
| 474 | `case:DEF_hist_vars%snowdp` | `DEF_hist_vars%snowdp` | DEF hist vars snowdp | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:900 |
| 475 | `case:DEF_hist_vars%soil1cCap_vr` | `DEF_hist_vars%soil1cCap_vr` | DEF hist vars soil1cCap vr | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1384 |
| 476 | `case:DEF_hist_vars%soil1c_vr` | `DEF_hist_vars%soil1c_vr` | DEF hist vars soil1c vr | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1369 |
| 477 | `case:DEF_hist_vars%soil1nCap_vr` | `DEF_hist_vars%soil1nCap_vr` | DEF hist vars soil1nCap vr | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1391 |
| 478 | `case:DEF_hist_vars%soil1n_vr` | `DEF_hist_vars%soil1n_vr` | DEF hist vars soil1n vr | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1376 |
| 479 | `case:DEF_hist_vars%soil2cCap_vr` | `DEF_hist_vars%soil2cCap_vr` | DEF hist vars soil2cCap vr | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1385 |
| 480 | `case:DEF_hist_vars%soil2c_vr` | `DEF_hist_vars%soil2c_vr` | DEF hist vars soil2c vr | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1370 |
| 481 | `case:DEF_hist_vars%soil2nCap_vr` | `DEF_hist_vars%soil2nCap_vr` | DEF hist vars soil2nCap vr | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1392 |
| 482 | `case:DEF_hist_vars%soil2n_vr` | `DEF_hist_vars%soil2n_vr` | DEF hist vars soil2n vr | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1377 |
| 483 | `case:DEF_hist_vars%soil3cCap_vr` | `DEF_hist_vars%soil3cCap_vr` | DEF hist vars soil3cCap vr | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1386 |
| 484 | `case:DEF_hist_vars%soil3c_vr` | `DEF_hist_vars%soil3c_vr` | DEF hist vars soil3c vr | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1371 |
| 485 | `case:DEF_hist_vars%soil3nCap_vr` | `DEF_hist_vars%soil3nCap_vr` | DEF hist vars soil3nCap vr | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1393 |
| 486 | `case:DEF_hist_vars%soil3n_vr` | `DEF_hist_vars%soil3n_vr` | DEF hist vars soil3n vr | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1378 |
| 487 | `case:DEF_hist_vars%sol_dif_ln_hires` | `DEF_hist_vars%sol_dif_ln_hires` | DEF hist vars sol dif ln hires | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1430 |
| 488 | `case:DEF_hist_vars%sol_dir_ln_hires` | `DEF_hist_vars%sol_dir_ln_hires` | DEF hist vars sol dir ln hires | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1429 |
| 489 | `case:DEF_hist_vars%solnd` | `DEF_hist_vars%solnd` | DEF hist vars solnd | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1413 |
| 490 | `case:DEF_hist_vars%solndln` | `DEF_hist_vars%solndln` | DEF hist vars solndln | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1422 |
| 491 | `case:DEF_hist_vars%solni` | `DEF_hist_vars%solni` | DEF hist vars solni | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1414 |
| 492 | `case:DEF_hist_vars%solniln` | `DEF_hist_vars%solniln` | DEF hist vars solniln | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1423 |
| 493 | `case:DEF_hist_vars%solvd` | `DEF_hist_vars%solvd` | DEF hist vars solvd | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1411 |
| 494 | `case:DEF_hist_vars%solvdln` | `DEF_hist_vars%solvdln` | DEF hist vars solvdln | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1420 |
| 495 | `case:DEF_hist_vars%solvi` | `DEF_hist_vars%solvi` | DEF hist vars solvi | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1412 |
| 496 | `case:DEF_hist_vars%solviln` | `DEF_hist_vars%solviln` | DEF hist vars solviln | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1421 |
| 497 | `case:DEF_hist_vars%somc_fire` | `DEF_hist_vars%somc_fire` | DEF hist vars somc fire | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1184 |
| 498 | `case:DEF_hist_vars%somfire` | `DEF_hist_vars%somfire` | DEF hist vars somfire | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1186 |
| 499 | `case:DEF_hist_vars%sr` | `DEF_hist_vars%sr` | DEF hist vars sr | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1410 |
| 500 | `case:DEF_hist_vars%sr_dif_ln_hires` | `DEF_hist_vars%sr_dif_ln_hires` | DEF hist vars sr dif ln hires | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1432 |
| 501 | `case:DEF_hist_vars%sr_dir_ln_hires` | `DEF_hist_vars%sr_dir_ln_hires` | DEF hist vars sr dir ln hires | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1431 |
| 502 | `case:DEF_hist_vars%srnd` | `DEF_hist_vars%srnd` | DEF hist vars srnd | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1417 |
| 503 | `case:DEF_hist_vars%srndln` | `DEF_hist_vars%srndln` | DEF hist vars srndln | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1426 |
| 504 | `case:DEF_hist_vars%srni` | `DEF_hist_vars%srni` | DEF hist vars srni | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1418 |
| 505 | `case:DEF_hist_vars%srniln` | `DEF_hist_vars%srniln` | DEF hist vars srniln | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1427 |
| 506 | `case:DEF_hist_vars%srvd` | `DEF_hist_vars%srvd` | DEF hist vars srvd | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1415 |
| 507 | `case:DEF_hist_vars%srvdln` | `DEF_hist_vars%srvdln` | DEF hist vars srvdln | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1424 |
| 508 | `case:DEF_hist_vars%srvi` | `DEF_hist_vars%srvi` | DEF hist vars srvi | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1416 |
| 509 | `case:DEF_hist_vars%srviln` | `DEF_hist_vars%srviln` | DEF hist vars srviln | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1425 |
| 510 | `case:DEF_hist_vars%sum_deficit_irrig` | `DEF_hist_vars%sum_deficit_irrig` | DEF hist vars sum deficit irrig | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1162 |
| 511 | `case:DEF_hist_vars%sum_irrig` | `DEF_hist_vars%sum_irrig` | DEF hist vars sum irrig | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1161 |
| 512 | `case:DEF_hist_vars%sum_irrig_count` | `DEF_hist_vars%sum_irrig_count` | DEF hist vars sum irrig count | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1163 |
| 513 | `case:DEF_hist_vars%t2m_wmo` | `DEF_hist_vars%t2m_wmo` | DEF hist vars t2m wmo | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:918 |
| 514 | `case:DEF_hist_vars%t_grnd` | `DEF_hist_vars%t_grnd` | DEF hist vars t grnd | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:896 |
| 515 | `case:DEF_hist_vars%t_lake` | `DEF_hist_vars%t_lake` | DEF hist vars t lake | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1346 |
| 516 | `case:DEF_hist_vars%t_roof` | `DEF_hist_vars%t_roof` | DEF hist vars t roof | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:939 |
| 517 | `case:DEF_hist_vars%t_room` | `DEF_hist_vars%t_room` | DEF hist vars t room | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:937 |
| 518 | `case:DEF_hist_vars%t_scalar` | `DEF_hist_vars%t_scalar` | DEF hist vars t scalar | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1320 |
| 519 | `case:DEF_hist_vars%t_soisno` | `DEF_hist_vars%t_soisno` | DEF hist vars t soisno | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1323 |
| 520 | `case:DEF_hist_vars%t_wall` | `DEF_hist_vars%t_wall` | DEF hist vars t wall | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:940 |
| 521 | `case:DEF_hist_vars%tafu` | `DEF_hist_vars%tafu` | DEF hist vars tafu | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:938 |
| 522 | `case:DEF_hist_vars%taux` | `DEF_hist_vars%taux` | DEF hist vars taux | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:859 |
| 523 | `case:DEF_hist_vars%tauy` | `DEF_hist_vars%tauy` | DEF hist vars tauy | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:860 |
| 524 | `case:DEF_hist_vars%tleaf` | `DEF_hist_vars%tleaf` | DEF hist vars tleaf | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:897 |
| 525 | `case:DEF_hist_vars%totcolc` | `DEF_hist_vars%totcolc` | DEF hist vars totcolc | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1005 |
| 526 | `case:DEF_hist_vars%totcoln` | `DEF_hist_vars%totcoln` | DEF hist vars totcoln | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1010 |
| 527 | `case:DEF_hist_vars%totcwdc` | `DEF_hist_vars%totcwdc` | DEF hist vars totcwdc | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1003 |
| 528 | `case:DEF_hist_vars%totcwdn` | `DEF_hist_vars%totcwdn` | DEF hist vars totcwdn | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1008 |
| 529 | `case:DEF_hist_vars%totfire` | `DEF_hist_vars%totfire` | DEF hist vars totfire | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1187 |
| 530 | `case:DEF_hist_vars%totlitc` | `DEF_hist_vars%totlitc` | DEF hist vars totlitc | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1002 |
| 531 | `case:DEF_hist_vars%totlitn` | `DEF_hist_vars%totlitn` | DEF hist vars totlitn | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1007 |
| 532 | `case:DEF_hist_vars%totsoiln_vr` | `DEF_hist_vars%totsoiln_vr` | DEF hist vars totsoiln vr | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1011 |
| 533 | `case:DEF_hist_vars%totsomc` | `DEF_hist_vars%totsomc` | DEF hist vars totsomc | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1004 |
| 534 | `case:DEF_hist_vars%totsomn` | `DEF_hist_vars%totsomn` | DEF hist vars totsomn | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1009 |
| 535 | `case:DEF_hist_vars%totvegc` | `DEF_hist_vars%totvegc` | DEF hist vars totvegc | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1001 |
| 536 | `case:DEF_hist_vars%totvegn` | `DEF_hist_vars%totvegn` | DEF hist vars totvegn | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1006 |
| 537 | `case:DEF_hist_vars%trad` | `DEF_hist_vars%trad` | DEF hist vars trad | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:915 |
| 538 | `case:DEF_hist_vars%transmittance_out` | `DEF_hist_vars%transmittance_out` | DEF hist vars transmittance out | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:912 |
| 539 | `case:DEF_hist_vars%tref` | `DEF_hist_vars%tref` | DEF hist vars tref | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:917 |
| 540 | `case:DEF_hist_vars%tstar` | `DEF_hist_vars%tstar` | DEF hist vars tstar | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1400 |
| 541 | `case:DEF_hist_vars%us10m` | `DEF_hist_vars%us10m` | DEF hist vars us10m | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1407 |
| 542 | `case:DEF_hist_vars%ustar` | `DEF_hist_vars%ustar` | DEF hist vars ustar | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1398 |
| 543 | `case:DEF_hist_vars%ustar2` | `DEF_hist_vars%ustar2` | DEF hist vars ustar2 | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1399 |
| 544 | `case:DEF_hist_vars%vegwp` | `DEF_hist_vars%vegwp` | DEF hist vars vegwp | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1335 |
| 545 | `case:DEF_hist_vars%vehc` | `DEF_hist_vars%vehc` | DEF hist vars vehc | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:936 |
| 546 | `case:DEF_hist_vars%veloc_hru` | `DEF_hist_vars%veloc_hru` | DEF hist vars veloc hru | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1442 |
| 547 | `case:DEF_hist_vars%vf` | `DEF_hist_vars%vf` | DEF hist vars vf | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1086 |
| 548 | `case:DEF_hist_vars%volresv` | `DEF_hist_vars%volresv` | DEF hist vars volresv | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1443 |
| 549 | `case:DEF_hist_vars%vs10m` | `DEF_hist_vars%vs10m` | DEF hist vars vs10m | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1408 |
| 550 | `case:DEF_hist_vars%w_scalar` | `DEF_hist_vars%w_scalar` | DEF hist vars w scalar | `.false.` |  | case-scalar | editable-common | MOD_Namelist.F90:1321 |
| 551 | `case:DEF_hist_vars%wa` | `DEF_hist_vars%wa` | DEF hist vars wa | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1342 |
| 552 | `case:DEF_hist_vars%wa_inst` | `DEF_hist_vars%wa_inst` | DEF hist vars wa inst | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1343 |
| 553 | `case:DEF_hist_vars%wat` | `DEF_hist_vars%wat` | DEF hist vars wat | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:888 |
| 554 | `case:DEF_hist_vars%wat_inst` | `DEF_hist_vars%wat_inst` | DEF hist vars wat inst | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:889 |
| 555 | `case:DEF_hist_vars%waterstorage` | `DEF_hist_vars%waterstorage` | DEF hist vars waterstorage | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1164 |
| 556 | `case:DEF_hist_vars%wdsrf` | `DEF_hist_vars%wdsrf` | DEF hist vars wdsrf | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1339 |
| 557 | `case:DEF_hist_vars%wdsrf_hru` | `DEF_hist_vars%wdsrf_hru` | DEF hist vars wdsrf hru | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1441 |
| 558 | `case:DEF_hist_vars%wdsrf_inst` | `DEF_hist_vars%wdsrf_inst` | DEF hist vars wdsrf inst | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1340 |
| 559 | `case:DEF_hist_vars%wetwat` | `DEF_hist_vars%wetwat` | DEF hist vars wetwat | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:890 |
| 560 | `case:DEF_hist_vars%wetwat_inst` | `DEF_hist_vars%wetwat_inst` | DEF hist vars wetwat inst | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:891 |
| 561 | `case:DEF_hist_vars%wetzwt` | `DEF_hist_vars%wetzwt` | DEF hist vars wetzwt | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:892 |
| 562 | `case:DEF_hist_vars%wfc` | `DEF_hist_vars%wfc` | DEF hist vars wfc | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1337 |
| 563 | `case:DEF_hist_vars%wice_soisno` | `DEF_hist_vars%wice_soisno` | DEF hist vars wice soisno | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1325 |
| 564 | `case:DEF_hist_vars%wliq_soisno` | `DEF_hist_vars%wliq_soisno` | DEF hist vars wliq soisno | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1324 |
| 565 | `case:DEF_hist_vars%xerr` | `DEF_hist_vars%xerr` | DEF hist vars xerr | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:875 |
| 566 | `case:DEF_hist_vars%xsubs_bsn` | `DEF_hist_vars%xsubs_bsn` | DEF hist vars xsubs bsn | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1434 |
| 567 | `case:DEF_hist_vars%xsubs_hru` | `DEF_hist_vars%xsubs_hru` | DEF hist vars xsubs hru | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1435 |
| 568 | `case:DEF_hist_vars%xwsub` | `DEF_hist_vars%xwsub` | DEF hist vars xwsub | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:883 |
| 569 | `case:DEF_hist_vars%xwsur` | `DEF_hist_vars%xwsur` | DEF hist vars xwsur | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:882 |
| 570 | `case:DEF_hist_vars%xy_frl` | `DEF_hist_vars%xy_frl` | DEF hist vars xy frl | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:852 |
| 571 | `case:DEF_hist_vars%xy_hpbl` | `DEF_hist_vars%xy_hpbl` | DEF hist vars xy hpbl | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:857 |
| 572 | `case:DEF_hist_vars%xy_ozone` | `DEF_hist_vars%xy_ozone` | DEF hist vars xy ozone | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:856 |
| 573 | `case:DEF_hist_vars%xy_pbot` | `DEF_hist_vars%xy_pbot` | DEF hist vars xy pbot | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:851 |
| 574 | `case:DEF_hist_vars%xy_prc` | `DEF_hist_vars%xy_prc` | DEF hist vars xy prc | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:849 |
| 575 | `case:DEF_hist_vars%xy_prl` | `DEF_hist_vars%xy_prl` | DEF hist vars xy prl | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:850 |
| 576 | `case:DEF_hist_vars%xy_q` | `DEF_hist_vars%xy_q` | DEF hist vars xy q | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:848 |
| 577 | `case:DEF_hist_vars%xy_rain` | `DEF_hist_vars%xy_rain` | DEF hist vars xy rain | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:854 |
| 578 | `case:DEF_hist_vars%xy_snow` | `DEF_hist_vars%xy_snow` | DEF hist vars xy snow | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:855 |
| 579 | `case:DEF_hist_vars%xy_solarin` | `DEF_hist_vars%xy_solarin` | DEF hist vars xy solarin | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:853 |
| 580 | `case:DEF_hist_vars%xy_t` | `DEF_hist_vars%xy_t` | DEF hist vars xy t | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:847 |
| 581 | `case:DEF_hist_vars%xy_us` | `DEF_hist_vars%xy_us` | DEF hist vars xy us | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:845 |
| 582 | `case:DEF_hist_vars%xy_vs` | `DEF_hist_vars%xy_vs` | DEF hist vars xy vs | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:846 |
| 583 | `case:DEF_hist_vars%z0m` | `DEF_hist_vars%z0m` | DEF hist vars z0m | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:914 |
| 584 | `case:DEF_hist_vars%zerr` | `DEF_hist_vars%zerr` | DEF hist vars zerr | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:876 |
| 585 | `case:DEF_hist_vars%zol` | `DEF_hist_vars%zol` | DEF hist vars zol | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1402 |
| 586 | `case:DEF_hist_vars%zwt` | `DEF_hist_vars%zwt` | DEF hist vars zwt | `.true.` |  | case-scalar | editable-common | MOD_Namelist.F90:1341 |

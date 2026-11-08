# 附录 A：配置字段总表（964 项）

本表由桌面端 `describe_fields` 命令导出，覆盖 CoLM namelist 的全部字段。`文件` 列表示字段写进哪个 namelist（`nl_colm` = case.nml，`nl_colm_forcing` = forcing.nml，`nl_colm_history` = history.nml，其余为材料/同位素等扩展 namelist）。`派生` 为“是”的字段由程序覆盖，界面上应视为只读。`依赖宏` 表示该字段只有在对应编译宏开启的内核里才有意义。

| # | 字段 | 分组 | 文件 | 类型 | 默认值 | 可选值 | 依赖宏 | 派生 | 说明 |
|---|---|---|---|---|---|---|---|---|---|
## 初始场

| # | 字段 | 分组 | 文件 | 类型 | 默认值 | 可选值 | 依赖宏 | 派生 | 说明 |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `DEF_USE_CN_INIT` | 初始场 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 2 | `DEF_USE_SnowInit` | 初始场 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 3 | `DEF_USE_SoilInit` | 初始场 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 4 | `DEF_USE_WaterTableInit` | 初始场 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 5 | `DEF_file_SnowInit` | 初始场 | `nl_colm` | Character { len: 256 } | `null` |  |  |  |  |
| 6 | `DEF_file_SoilInit` | 初始场 | `nl_colm` | Character { len: 256 } | `null` |  |  |  |  |
| 7 | `DEF_file_WaterTable` | 初始场 | `nl_colm` | Character { len: 256 } | `null` |  |  |  |  |
| 8 | `DEF_file_cn_init` | 初始场 | `nl_colm` | Character { len: 256 } | `null` |  |  |  |  |

## 地表数据

| # | 字段 | 分组 | 文件 | 类型 | 默认值 | 可选值 | 依赖宏 | 派生 | 说明 |
|---|---|---|---|---|---|---|---|---|---|
| 9 | `DEF_USE_ESACCI` | 地表数据 | `` | Logical | `.false.` |  |  | 是 |  |
| 10 | `DEF_USE_GLC30` | 地表数据 | `` | Logical | `.false.` |  |  | 是 |  |
| 11 | `DEF_USE_IGBP` | 地表数据 | `` | Logical | `.false.` |  |  | 是 |  |
| 12 | `DEF_USE_USGS` | 地表数据 | `` | Logical | `.false.` |  |  | 是 |  |
| 13 | `DEF_FAST_PC` | 地表数据 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 14 | `DEF_HighResSoil` | 地表数据 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 15 | `DEF_HighResUrban_albedo` | 地表数据 | `nl_colm` | Character { len: 256 } | `null` |  |  |  |  |
| 16 | `DEF_HighResVeg` | 地表数据 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 17 | `DEF_LAI_CHANGE_YEARLY` | 地表数据 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 18 | `DEF_LAI_END_YEAR` | 地表数据 | `nl_colm` | Integer | `2020` |  |  |  |  |
| 19 | `DEF_LAI_MONTHLY` | 地表数据 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 20 | `DEF_LAI_START_YEAR` | 地表数据 | `nl_colm` | Integer | `2000` |  |  |  |  |
| 21 | `DEF_LANDONLY` | 地表数据 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 22 | `DEF_LULCC_SCHEME` | 地表数据 | `nl_colm` | Integer | `1` | 1, 2 |  |  |  |
| 23 | `DEF_PC_CROP_SPLIT` | 地表数据 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 24 | `DEF_SOIL_REFL_SCHEME` | 地表数据 | `nl_colm` | Integer | `2` | 1, 2 |  |  |  |
| 25 | `DEF_SOLO_PFT` | 地表数据 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 26 | `DEF_SUBGRID_SCHEME` | 地表数据 | `nl_colm` | Character { len: 256 } | `LCT` |  |  |  |  |
| 27 | `DEF_Srfdata_CompressLevel` | 地表数据 | `nl_colm` | Integer | `1` |  |  |  |  |
| 28 | `DEF_USE_DOMINANT_PATCHTYPE` | 地表数据 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 29 | `DEF_USE_LCT` | 地表数据 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 30 | `DEF_USE_LULCC` | 地表数据 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 31 | `DEF_USE_PC` | 地表数据 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 32 | `DEF_USE_PFT` | 地表数据 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 33 | `DEF_USE_SOILPAR_UPS_FIT` | 地表数据 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 34 | `DEF_dir_existing_srfdata` | 地表数据 | `nl_colm` | Character { len: 256 } | `path/to/landdata` |  |  |  |  |
| 35 | `DEF_dir_rawdata` | 地表数据 | `nl_colm` | Character { len: 256 } | `path/to/rawdata/` |  |  |  |  |
| 36 | `DEF_rawdata_namelist` | 地表数据 | `nl_colm` | Character { len: 256 } | `./rawdata/colm2024.nml` |  |  |  |  |
| 37 | `USE_srfdata_from_3D_gridded_data` | 地表数据 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 38 | `USE_srfdata_from_larger_region` | 地表数据 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 39 | `USE_zip_for_aggregation` | 地表数据 | `nl_colm` | Logical | `.true.` |  |  |  |  |

## 城市

| # | 字段 | 分组 | 文件 | 类型 | 默认值 | 可选值 | 依赖宏 | 派生 | 说明 |
|---|---|---|---|---|---|---|---|---|---|
| 40 | `DEF_URBAN_BEM` | 城市 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 41 | `DEF_URBAN_LUCY` | 城市 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 42 | `DEF_URBAN_ONLY` | 城市 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 43 | `DEF_URBAN_RUN` | 城市 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 44 | `DEF_URBAN_TREE` | 城市 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 45 | `DEF_URBAN_WATER` | 城市 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 46 | `DEF_URBAN_type_scheme` | 城市 | `nl_colm` | Integer | `2` | 1, 2 |  |  |  |
| 47 | `DEF_USE_CANYON_HWR` | 城市 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 48 | `USE_SITE_urban_ecology` | 城市 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 49 | `USE_SITE_urban_geometry` | 城市 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 50 | `USE_SITE_urban_human` | 城市 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 51 | `USE_SITE_urban_radiation` | 城市 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 52 | `USE_SITE_urban_thermal` | 城市 | `nl_colm` | Logical | `.false.` |  |  |  |  |

## 强迫场

| # | 字段 | 分组 | 文件 | 类型 | 默认值 | 可选值 | 依赖宏 | 派生 | 说明 |
|---|---|---|---|---|---|---|---|---|---|
| 53 | `DEF_DS_HiresTopographyDataDir` | 强迫场 | `nl_colm` | Character { len: 256 } | `null` |  |  |  |  |
| 54 | `DEF_DS_LONGWAVE_LAPSE_RATE` | 强迫场 | `nl_colm` | Real | `0.032_r8` |  |  |  | Glacier longwave lapse rate [W m-2 m-1] |
| 55 | `DEF_DS_LONGWAVE_LIMIT` | 强迫场 | `nl_colm` | Real | `0.5_r8` |  |  |  | Relative longwave correction limit [0,1] |
| 56 | `DEF_DS_SHORTWAVE_LIMIT` | 强迫场 | `nl_colm` | Real | `0.5_r8` |  |  |  | Full-mode relative shortwave correction limit [0,1] |
| 57 | `DEF_DS_SHORTWAVE_SIMPLE_LIMIT` | 强迫场 | `nl_colm` | Real | `0.2_r8` |  |  |  | Simple-mode relative shortwave correction limit [0,1] |
| 58 | `DEF_DS_TEMP_LAPSE_RATE` | 强迫场 | `nl_colm` | Real | `0.006_r8` |  |  |  | Temperature lapse rate [K m-1] |
| 59 | `DEF_DS_longwave_adjust_scheme` | 强迫场 | `nl_colm` | Character { len: 5 } | `II` | I, II |  |  |  |
| 60 | `DEF_DS_precipitation_adjust_scheme` | 强迫场 | `nl_colm` | Character { len: 5 } | `I` | I, II, III |  |  |  |
| 61 | `DEF_Forcing_Interp_Method` | 强迫场 | `nl_colm` | Character { len: 20 } | `arealweight` | arealweight, bilinear |  |  | 'arealweight' (default) or 'bilinear' |
| 62 | `DEF_USE_CBL_HEIGHT` | 强迫场 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 63 | `DEF_USE_ClimForcing_for_Spinup` | 强迫场 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 64 | `DEF_USE_Forcing_Downscaling` | 强迫场 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 65 | `DEF_USE_Forcing_Downscaling_Simple` | 强迫场 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 66 | `DEF_dir_forcing` | 强迫场 | `nl_colm_forcing` | Character { len: 256 } | `path/to/forcing/data` |  |  |  |  |
| 67 | `DEF_forcing%CBL_dtime` | 强迫场 | `nl_colm_forcing` | Integer | `21600` |  |  |  |  |
| 68 | `DEF_forcing%CBL_fprefix` | 强迫场 | `nl_colm_forcing` | Character { len: 256 } | `TPHWL6Hrly/clmforc.cruncep.V4.c2011.0.5d.TPQWL.` |  |  |  |  |
| 69 | `DEF_forcing%CBL_offset` | 强迫场 | `nl_colm_forcing` | Integer | `10800` |  |  |  |  |
| 70 | `DEF_forcing%CBL_tintalgo` | 强迫场 | `nl_colm_forcing` | Character { len: 256 } | `linear` |  |  |  |  |
| 71 | `DEF_forcing%CBL_vname` | 强迫场 | `nl_colm_forcing` | Character { len: 256 } | `blh` |  |  |  |  |
| 72 | `DEF_forcing%HEIGHT_Q` | 强迫场 | `nl_colm_forcing` | Real | `50.` |  |  |  |  |
| 73 | `DEF_forcing%HEIGHT_T` | 强迫场 | `nl_colm_forcing` | Real | `50.` |  |  |  |  |
| 74 | `DEF_forcing%HEIGHT_V` | 强迫场 | `nl_colm_forcing` | Real | `100.0` |  |  |  |  |
| 75 | `DEF_forcing%HEIGHT_mode` | 强迫场 | `nl_colm_forcing` | Character { len: 256 } | `absolute` |  |  |  |  |
| 76 | `DEF_forcing%NVAR` | 强迫场 | `nl_colm_forcing` | Integer | `8` |  |  |  | variable number of forcing data |
| 77 | `DEF_forcing%data2d` | 强迫场 | `nl_colm_forcing` | Logical | `.true.` |  |  |  | data in 2 dimension (lon, lat) |
| 78 | `DEF_forcing%dataset` | 强迫场 | `nl_colm_forcing` | Character { len: 256 } | `CRUNCEP` | CPL7, CRUNCEPV4, CRUNCEPV7, GSWP2, GSWP3, POINT, PRINCETON, QIAN |  |  |  |
| 79 | `DEF_forcing%dim2d` | 强迫场 | `nl_colm_forcing` | Logical | `.true.` |  |  |  | lat/lon value in 2 dimension (lon, lat) |
| 80 | `DEF_forcing%dtime` | 强迫场 | `nl_colm_forcing` | Integer | `(/21600,21600,21600,21600,0,21600,21600,21600/)` |  |  |  |  |
| 81 | `DEF_forcing%endmo` | 强迫场 | `nl_colm_forcing` | Integer | `12` |  |  |  | end month of forcing data |
| 82 | `DEF_forcing%endyr` | 强迫场 | `nl_colm_forcing` | Integer | `2003` |  |  |  | end year of forcing data |
| 83 | `DEF_forcing%fprefix` | 强迫场 | `nl_colm_forcing` | Character { len: 256 } | `(/ 'TPHWL6Hrly/clmforc.cruncep.V4.c2011.0.5d.TPQWL.', 'TPHWL6Hrly/clmforc.cruncep.V4.c2011.0.5d.TPQWL.', 'TPHWL6Hrly/clmforc.cruncep.V4.c2011.0.5d.TPQWL.', 'Precip6Hrly/clmforc.cruncep.V4.c2011.0.5d.Prec.', 'NULL                                           ', 'TPHWL6Hrly/clmforc.cruncep.V4.c2011.0.5d.TPQWL.', 'Solar6Hrly/clmforc.cruncep.V4.c2011.0.5d.Solr. ', 'TPHWL6Hrly/clmforc.cruncep.V4.c2011.0.5d.TPQWL.' /)` |  |  |  |  |
| 84 | `DEF_forcing%groupby` | 强迫场 | `nl_colm_forcing` | Character { len: 256 } | `month` | day, month, year |  |  | file grouped by year/month |
| 85 | `DEF_forcing%has_missing_value` | 强迫场 | `nl_colm_forcing` | Logical | `.false.` |  |  |  |  |
| 86 | `DEF_forcing%hightdim` | 强迫场 | `nl_colm_forcing` | Logical | `.false.` |  |  |  | have "z" dimension |
| 87 | `DEF_forcing%latname` | 强迫场 | `nl_colm_forcing` | Character { len: 256 } | `LATIXY` |  |  |  | dimension name of latitude |
| 88 | `DEF_forcing%leapyear` | 强迫场 | `nl_colm_forcing` | Logical | `.false.` |  |  |  | leapyear calendar |
| 89 | `DEF_forcing%lonname` | 强迫场 | `nl_colm_forcing` | Character { len: 256 } | `LONGXY` |  |  |  | dimension name of longitude |
| 90 | `DEF_forcing%missing_value_name` | 强迫场 | `nl_colm_forcing` | Character { len: 256 } | `missing_value` |  |  |  |  |
| 91 | `DEF_forcing%nlands` | 强迫场 | `nl_colm_forcing` | Integer | `1` |  |  |  | land grid number in 1d |
| 92 | `DEF_forcing%offset` | 强迫场 | `nl_colm_forcing` | Integer | `(/10800,10800,10800,10800,0,10800,0,10800/)` |  |  |  |  |
| 93 | `DEF_forcing%regbnd` | 强迫场 | `nl_colm_forcing` | Real | `(/-90.0, 90.0, -180.0, 180.0/)` |  |  |  |  |
| 94 | `DEF_forcing%regional` | 强迫场 | `nl_colm_forcing` | Logical | `.false.` |  |  |  |  |
| 95 | `DEF_forcing%solarin_all_band` | 强迫场 | `nl_colm_forcing` | Logical | `.true.` |  |  |  |  |
| 96 | `DEF_forcing%startmo` | 强迫场 | `nl_colm_forcing` | Integer | `1` |  |  |  | start month of forcing data |
| 97 | `DEF_forcing%startyr` | 强迫场 | `nl_colm_forcing` | Integer | `2000` |  |  |  | start year of forcing data |
| 98 | `DEF_forcing%timelog` | 强迫场 | `nl_colm_forcing` | Character { len: 256 } | `(/ 'instant ','instant ','instant ','forward ', 'NULL    ','instant ','forward ','forward ' /)` |  |  |  |  |
| 99 | `DEF_forcing%tintalgo` | 强迫场 | `nl_colm_forcing` | Character { len: 256 } | `(/ 'linear ','linear ','linear ','nearest', 'NULL   ','linear ','coszen ','linear ' /)` |  |  |  |  |
| 100 | `DEF_forcing%vname` | 强迫场 | `nl_colm_forcing` | Character { len: 256 } | `(/ 'TBOT    ','QBOT    ','PSRF    ','PRECTmms', 'NULL    ','WIND    ','FSDS    ','FLDS    ' /)` |  |  |  |  |

## 数据同化

| # | 字段 | 分组 | 文件 | 类型 | 默认值 | 可选值 | 依赖宏 | 派生 | 说明 |
|---|---|---|---|---|---|---|---|---|---|
| 101 | `DEF_DA_ENS_NUM` | 数据同化 | `nl_colm` | Integer | `20` |  | DataAssimilation |  |  |
| 102 | `DEF_DA_ENS_SM` | 数据同化 | `nl_colm` | Logical | `.false.` |  | DataAssimilation |  |  |
| 103 | `DEF_DA_RTM_diel` | 数据同化 | `nl_colm` | Integer | `0` | 0, 1, 2, 3 | DataAssimilation |  |  |
| 104 | `DEF_DA_RTM_rough` | 数据同化 | `nl_colm` | Integer | `0` | 0, 1, 2, 3 | DataAssimilation |  |  |
| 105 | `DEF_DA_SM` | 数据同化 | `nl_colm` | Logical | `.false.` |  | DataAssimilation |  |  |
| 106 | `DEF_DA_SM_FY` | 数据同化 | `nl_colm` | Logical | `.false.` |  | DataAssimilation |  |  |
| 107 | `DEF_DA_SM_SMAP` | 数据同化 | `nl_colm` | Logical | `.false.` |  | DataAssimilation |  |  |
| 108 | `DEF_DA_SM_SYNOP` | 数据同化 | `nl_colm` | Logical | `.false.` |  | DataAssimilation |  |  |
| 109 | `DEF_DA_TWS` | 数据同化 | `nl_colm` | Logical | `.false.` |  | DataAssimilation |  |  |
| 110 | `DEF_DA_TWS_GRACE` | 数据同化 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 111 | `DEF_DA_obsdir` | 数据同化 | `nl_colm` | Character { len: 256 } | `null` |  | DataAssimilation |  |  |

## 文件与目录

| # | 字段 | 分组 | 文件 | 类型 | 默认值 | 可选值 | 依赖宏 | 派生 | 说明 |
|---|---|---|---|---|---|---|---|---|---|
| 112 | `DEF_dir_landdata` | 文件与目录 | `` | Character { len: 256 } | `path/to/landdata` |  |  | 是 |  |
| 113 | `DEF_dir_runtime` | 文件与目录 | `nl_colm` | Character { len: 256 } | `path/to/runtime/` |  |  |  |  |
| 114 | `DEF_forcing_namelist` | 文件与目录 | `nl_colm` | Character { len: 256 } | `null` |  |  |  |  |

## 时间与预热

| # | 字段 | 分组 | 文件 | 类型 | 默认值 | 可选值 | 依赖宏 | 派生 | 说明 |
|---|---|---|---|---|---|---|---|---|---|
| 115 | `DEF_simulation_time%end_day` | 时间与预热 | `nl_colm` | Integer | `1` |  |  |  |  |
| 116 | `DEF_simulation_time%end_month` | 时间与预热 | `nl_colm` | Integer | `1` |  |  |  |  |
| 117 | `DEF_simulation_time%end_sec` | 时间与预热 | `nl_colm` | Integer | `0` |  |  |  |  |
| 118 | `DEF_simulation_time%end_year` | 时间与预热 | `nl_colm` | Integer | `2003` |  |  |  |  |
| 119 | `DEF_simulation_time%greenwich` | 时间与预热 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 120 | `DEF_simulation_time%spinup_day` | 时间与预热 | `nl_colm` | Integer | `1` |  |  |  |  |
| 121 | `DEF_simulation_time%spinup_month` | 时间与预热 | `nl_colm` | Integer | `1` |  |  |  |  |
| 122 | `DEF_simulation_time%spinup_repeat` | 时间与预热 | `nl_colm` | Integer | `1` |  |  |  |  |
| 123 | `DEF_simulation_time%spinup_sec` | 时间与预热 | `nl_colm` | Integer | `0` |  |  |  |  |
| 124 | `DEF_simulation_time%spinup_year` | 时间与预热 | `nl_colm` | Integer | `0` |  |  |  |  |
| 125 | `DEF_simulation_time%start_day` | 时间与预热 | `nl_colm` | Integer | `1` |  |  |  |  |
| 126 | `DEF_simulation_time%start_month` | 时间与预热 | `nl_colm` | Integer | `1` |  |  |  |  |
| 127 | `DEF_simulation_time%start_sec` | 时间与预热 | `nl_colm` | Integer | `0` |  |  |  |  |
| 128 | `DEF_simulation_time%start_year` | 时间与预热 | `nl_colm` | Integer | `2000` |  |  |  |  |
| 129 | `DEF_simulation_time%timestep` | 时间与预热 | `nl_colm` | Real | `1800.` |  |  |  |  |

## 材料

| # | 字段 | 分组 | 文件 | 类型 | 默认值 | 可选值 | 依赖宏 | 派生 | 说明 |
|---|---|---|---|---|---|---|---|---|---|
| 130 | `DEF_USE_NH4_LEACHING` | 材料 | `` | Logical | `.false.` |  |  | 是 |  |
| 131 | `DEF_wetland_finundation_scheme` | 材料 | `` | Integer | `1` | 1, 2, 3, 4, 5, 6, 7 |  | 是 |  |
| 132 | `DEF_file_GIEMS` | 材料 | `nl_colm` | Character { len: 256 } | `null` |  |  |  |  |
| 133 | `DEF_ISO_CANOPY_EQUILIBRATION` | 材料 | `nl_colm_isotope_parameter` | Real | `0.0_r8` |  |  |  |  |
| 134 | `DEF_ISO_CG_RELHUM_MAX` | 材料 | `nl_colm_isotope_parameter` | Real | `0.99_r8` |  |  |  |  |
| 135 | `DEF_ISO_ICE_SUPERSAT_SLOPE` | 材料 | `nl_colm_isotope_parameter` | Real | `0.003_r8` |  |  |  |  |
| 136 | `DEF_ISO_INIT_DELTA_H2_18O` | 材料 | `nl_colm_isotope_parameter` | Real | `-10._r8` |  |  |  |  |
| 137 | `DEF_ISO_INIT_DELTA_HDO` | 材料 | `nl_colm_isotope_parameter` | Real | `-70._r8` |  |  |  |  |
| 138 | `DEF_ISO_KINETIC_SCHEME` | 材料 | `nl_colm_isotope_parameter` | Character { len: 16 } | `MERLIVAT1978` | CAPPA2003, MERLIVAT1978 |  |  |  |
| 139 | `DEF_ISO_NSS_LEAF_PATH_LENGTH` | 材料 | `nl_colm_isotope_parameter` | Real | `0.01_r8` |  |  |  |  |
| 140 | `DEF_ISO_NSS_LEAF_RB` | 材料 | `nl_colm_isotope_parameter` | Real | `100._r8` |  |  |  |  |
| 141 | `DEF_ISO_NSS_LEAF_WATER_PER_LAI` | 材料 | `nl_colm_isotope_parameter` | Real | `0.12_r8` |  |  |  |  |
| 142 | `DEF_ISO_OPEN_WATER_KINETIC` | 材料 | `nl_colm_isotope_parameter` | Character { len: 16 } | `MJ79` | EXPONENT, MJ79 |  |  |  |
| 143 | `DEF_ISO_REF_RATIO_H2_18O` | 材料 | `nl_colm_isotope_parameter` | Real | `2.0052e-3_r8` |  |  |  |  |
| 144 | `DEF_ISO_REF_RATIO_HDO` | 材料 | `nl_colm_isotope_parameter` | Real | `1.5576e-4_r8` |  |  |  |  |
| 145 | `DEF_ISO_SNOWMELT_EQUILIBRATION` | 材料 | `nl_colm_isotope_parameter` | Real | `0.0_r8` |  |  |  |  |
| 146 | `DEF_ISO_SOIL_DIFFUSION` | 材料 | `nl_colm_isotope_parameter` | Logical | `.true.` |  |  |  |  |
| 147 | `DEF_ISO_SOIL_KINETIC` | 材料 | `nl_colm_isotope_parameter` | Character { len: 16 } | `RESISTANCE` | EXPONENT, RESISTANCE |  |  |  |
| 148 | `DEF_ISO_SOIL_VAPOR_DIFFUSION` | 材料 | `nl_colm_isotope_parameter` | Logical | `.true.` |  |  |  |  |
| 149 | `DEF_ISO_SUBL_SKIN_MM` | 材料 | `nl_colm_isotope_parameter` | Real | `5.0_r8` |  |  |  |  |
| 150 | `DEF_ISO_USE_FRACTIONATION` | 材料 | `nl_colm_isotope_parameter` | Logical | `.false.` |  |  |  |  |
| 151 | `DEF_ISOTOPE_PARAM_FILE` | 材料 | `nl_colm_material` | Character { len: 256 } | `null` |  |  |  |  |
| 152 | `DEF_ISOTOPE_SPECIES` | 材料 | `nl_colm_material` | Character { len: 256 } | `H2_18O,HDO` |  |  |  |  |
| 153 | `DEF_METHANE_PARAM_FILE` | 材料 | `nl_colm_material` | Character { len: 256 } | `null` |  |  |  |  |
| 154 | `DEF_MT_AQUIFER_MIXING_WATER_MM` | 材料 | `nl_colm_material` | Real | `-1._r8` |  |  |  |  |
| 155 | `DEF_MT_BALANCE_ABORT_NBAD` | 材料 | `nl_colm_material` | Integer | `0` |  |  |  |  |
| 156 | `DEF_MT_FORCING_MEMORY_PRECIP` | 材料 | `nl_colm_material` | Real | `2.e-5_r8` |  |  |  |  |
| 157 | `DEF_MT_FORCING_MIN_PRECIP` | 材料 | `nl_colm_material` | Real | `5.e-6_r8` |  |  |  |  |
| 158 | `DEF_MT_LULCC_ABORT_NBAD` | 材料 | `nl_colm_material` | Integer | `0` |  |  |  |  |
| 159 | `DEF_MT_N2O_EF5_NH4` | 材料 | `nl_colm_material` | Real | `-1._r8` |  |  |  |  |
| 160 | `DEF_MT_N2O_EF5_NO3` | 材料 | `nl_colm_material` | Real | `-1._r8` |  |  |  |  |
| 161 | `DEF_MT_RESID_ABORT_NBAD` | 材料 | `nl_colm_material` | Integer | `0` |  |  |  |  |
| 162 | `DEF_MT_SOIL_INIT_FILE` | 材料 | `nl_colm_material` | Character { len: 256 } | `null` |  |  |  |  |
| 163 | `DEF_MT_SOIL_INIT_VARS` | 材料 | `nl_colm_material` | Character { len: 256 } | `` |  |  |  |  |
| 164 | `DEF_MT_USE_SOIL_INIT` | 材料 | `nl_colm_material` | Logical | `.false.` |  |  |  |  |
| 165 | `DEF_NH4_LEACHING` | 材料 | `nl_colm_material` | Character { len: 16 } | `auto` |  |  |  |  |
| 166 | `DEF_SEDIMENT_PARAM_FILE` | 材料 | `nl_colm_material` | Character { len: 256 } | `null` |  |  |  |  |
| 167 | `DEF_SOLUTE_NAMES` | 材料 | `nl_colm_material` | Character { len: 256 } | `` |  |  |  |  |
| 168 | `DEF_SOLUTE_PARAM_FILES` | 材料 | `nl_colm_material` | Character { len: 2048 } | `null` |  |  |  |  |
| 169 | `DEF_USE_ISOTOPE` | 材料 | `nl_colm_material` | Logical | `.false.` |  |  |  |  |
| 170 | `DEF_USE_METHANE` | 材料 | `nl_colm_material` | Logical | `.false.` |  |  |  |  |
| 171 | `DEF_USE_SEDIMENT` | 材料 | `nl_colm_material` | Logical | `.false.` |  |  |  |  |

## 水热过程

| # | 字段 | 分组 | 文件 | 类型 | 默认值 | 可选值 | 依赖宏 | 派生 | 说明 |
|---|---|---|---|---|---|---|---|---|---|
| 172 | `DEF_CheckEquilibrium` | 水热过程 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 173 | `DEF_Interception_scheme` | 水热过程 | `nl_colm` | Integer | `1` | 1, 2, 3, 4, 5, 6, 7, 8 |  |  |  |
| 174 | `DEF_Optimize_Baseflow` | 水热过程 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 175 | `DEF_RSS_SCHEME` | 水热过程 | `nl_colm` | Integer | `1` | 0, 1, 2, 3, 4, 5, 6 |  |  |  |
| 176 | `DEF_RS_CROWN_STRUCTURE` | 水热过程 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 177 | `DEF_Runoff_SCHEME` | 水热过程 | `nl_colm` | Integer | `3` | 0, 1, 2, 3 |  |  |  |
| 178 | `DEF_SPLIT_SOILSNOW` | 水热过程 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 179 | `DEF_THERMAL_CONDUCTIVITY_SCHEME` | 水热过程 | `nl_colm` | Integer | `4` | 1, 2, 3, 4, 5, 6, 7, 8 |  |  | Options for soil thermal conductivity schemes |
| 180 | `DEF_TOPMOD_method` | 水热过程 | `nl_colm` | Integer | `0` | 0, 1, 2 |  |  |  |
| 181 | `DEF_TUNING_CAPR` | 水热过程 | `nl_colm` | Real | `0.34_r8` |  |  |  | First-layer-to-surface temperature tuning factor [-] |
| 182 | `DEF_TUNING_CNFAC` | 水热过程 | `nl_colm` | Real | `0.5_r8` |  |  |  | Crank-Nicolson weighting factor [0,1] |
| 183 | `DEF_TUNING_PONDMX` | 水热过程 | `nl_colm` | Real | `10.0_r8` |  |  |  | Maximum surface ponding depth [mm] |
| 184 | `DEF_TUNING_SIMPLE_VIC_DS` | 水热过程 | `nl_colm` | Real | `0.061_r8` |  |  |  | Simple VIC baseflow fraction [-] |
| 185 | `DEF_TUNING_SIMPLE_VIC_WS` | 水热过程 | `nl_colm` | Real | `0.646_r8` |  |  |  | Simple VIC baseflow threshold fraction [-] |
| 186 | `DEF_TUNING_SMPMAX` | 水热过程 | `nl_colm` | Real | `-1.5e5_r8` |  |  |  | Simple VIC wilting matric potential [mm] |
| 187 | `DEF_TUNING_SMPMAX_HR` | 水热过程 | `nl_colm` | Real | `-2.e2_r8` |  |  |  | Heterotrophic-respiration soil-potential upper limit [mm] |
| 188 | `DEF_TUNING_SMPMIN` | 水热过程 | `nl_colm` | Real | `-1.e8_r8` |  |  |  | Minimum allowed soil matric potential [mm] |
| 189 | `DEF_TUNING_SMPMIN_HR` | 水热过程 | `nl_colm` | Real | `-2.e5_r8` |  |  |  | Heterotrophic-respiration soil-potential lower limit [mm] |
| 190 | `DEF_TUNING_SNOW_COVER_EXPONENT` | 水热过程 | `nl_colm` | Real | `1.0_r8` |  |  |  | Snow-cover density exponent [-] |
| 191 | `DEF_TUNING_SOIL_ICE_IMPEDANCE` | 水热过程 | `nl_colm` | Real | `6.0_r8` |  |  |  | Frozen-soil hydraulic impedance exponent [-] |
| 192 | `DEF_TUNING_SSI` | 水热过程 | `nl_colm` | Real | `0.033_r8` |  |  |  | Irreducible snow-water saturation fraction [0,1] |
| 193 | `DEF_TUNING_TOPMOD_DECAY` | 水热过程 | `nl_colm` | Real | `2.0_r8` |  |  |  | TOPMODEL water-table decay coefficient [m-1] |
| 194 | `DEF_TUNING_WETWATMAX` | 水热过程 | `nl_colm` | Real | `200.0_r8` |  |  |  | Maximum wetland surface-water storage [mm] |
| 195 | `DEF_TUNING_WIMP` | 水热过程 | `nl_colm` | Real | `0.05_r8` |  |  |  | Impermeable-soil porosity threshold [0,1) |
| 196 | `DEF_TUNING_ZLND` | 水热过程 | `nl_colm` | Real | `0.01_r8` |  |  |  | Soil aerodynamic roughness length [m] |
| 197 | `DEF_TUNING_ZSNO` | 水热过程 | `nl_colm` | Real | `0.0024_r8` |  |  |  | Snow aerodynamic roughness length [m] |
| 198 | `DEF_USE_BEDROCK` | 水热过程 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 199 | `DEF_USE_Campbell_SOIL_MODEL` | 水热过程 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 200 | `DEF_USE_Dynamic_Lake` | 水热过程 | `nl_colm` | Logical | `.false.` |  |  |  | Dynamic Lake model |
| 201 | `DEF_USE_Dynamic_Wetland` | 水热过程 | `nl_colm` | Logical | `.false.` |  |  |  | Dynamic wetland model |
| 202 | `DEF_USE_SUPERCOOL_WATER` | 水热过程 | `nl_colm` | Logical | `.true.` |  |  |  | supercooled soil water scheme, Niu & Yang (2006) |
| 203 | `DEF_USE_VariablySaturatedFlow` | 水热过程 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 204 | `DEF_VIC_OPT` | 水热过程 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 205 | `DEF_file_VIC_OPT` | 水热过程 | `nl_colm` | Character { len: 256 } | `null` |  |  |  |  |
| 206 | `DEF_file_VIC_para` | 水热过程 | `nl_colm` | Character { len: 256 } | `null` |  |  |  |  |
| 207 | `DEF_precip_phase_discrimination_scheme` | 水热过程 | `nl_colm` | Character { len: 5 } | `I` | I, II, III |  |  |  |

## 河道与水库

| # | 字段 | 分组 | 文件 | 类型 | 默认值 | 可选值 | 依赖宏 | 派生 | 说明 |
|---|---|---|---|---|---|---|---|---|---|
| 208 | `DEF_CaMa_FloodFeedback` | 河道与水库 | `nl_colm` | Logical | `.false.` |  | CaMa_Flood |  |  |
| 209 | `DEF_CaMa_Namelist` | 河道与水库 | `nl_colm` | Character { len: 256 } | `null` |  | CaMa_Flood |  |  |
| 210 | `DEF_CaMa_Restart_file` | 河道与水库 | `nl_colm` | Character { len: 256 } | `null` |  | CaMa_Flood |  |  |
| 211 | `DEF_CaMa_StrictDomain` | 河道与水库 | `nl_colm` | Logical | `.false.` |  | CaMa_Flood |  |  |
| 212 | `DEF_ElementNeighbour_file` | 河道与水库 | `nl_colm` | Character { len: 256 } | `null` |  |  |  |  |
| 213 | `DEF_GRIDBASED_ROUTING_MAX_DT` | 河道与水库 | `nl_colm` | Real | `3600.` |  | GridRiverLakeFlow |  |  |
| 214 | `DEF_GRIDBASED_ROUTING_MOMENTUM_DT_LIMIT` | 河道与水库 | `nl_colm` | Logical | `.false.` |  | GridRiverLakeFlow |  |  |
| 215 | `DEF_GRIDBASED_ROUTING_STORAGE_DT_LIMIT` | 河道与水库 | `nl_colm` | Logical | `.true.` |  | GridRiverLakeFlow |  |  |
| 216 | `DEF_GridRiverLake_FloodFeedback` | 河道与水库 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 217 | `DEF_GridRiverLake_FloodInfiltMax` | 河道与水库 | `nl_colm` | Real | `5._r8` |  |  |  |  |
| 218 | `DEF_ReservoirPara_file` | 河道与水库 | `nl_colm` | Character { len: 256 } | `null` |  | GridRiverLakeFlow |  |  |
| 219 | `DEF_Reservoir_Method` | 河道与水库 | `nl_colm` | Integer | `0` | 0, 1, 2 |  |  |  |
| 220 | `DEF_USE_BIFURCATION` | 河道与水库 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 221 | `DEF_USE_EstimatedRiverDepth` | 河道与水库 | `nl_colm` | Logical | `.true.` |  | CatchLateralFlow |  |  |
| 222 | `DEF_USE_GridRiverLakeFlow` | 河道与水库 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 223 | `DEF_USE_LEVEE` | 河道与水库 | `nl_colm` | Logical | `.false.` |  | GridRiverLakeFlow |  |  |
| 224 | `DEF_UnitCatchment_file` | 河道与水库 | `nl_colm` | Character { len: 256 } | `null` | , NONE, NULL |  |  |  |
| 225 | `DEF_UnitCatchment_regional` | 河道与水库 | `nl_colm` | Logical | `.false.` |  |  |  |  |

## 生态与生地化

| # | 字段 | 分组 | 文件 | 类型 | 默认值 | 可选值 | 依赖宏 | 派生 | 说明 |
|---|---|---|---|---|---|---|---|---|---|
| 226 | `DEF_USE_CROP` | 生态与生地化 | `` | Logical | `.false.` |  |  | 是 |  |
| 227 | `DEF_Aerosol_Clim` | 生态与生地化 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 228 | `DEF_Aerosol_Readin` | 生态与生地化 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 229 | `DEF_BALL_BERRY_BINTER` | 生态与生地化 | `nl_colm` | Real | `-1.0_r8` |  |  |  | Ball-Berry intercept; valid override >= 0 |
| 230 | `DEF_BALL_BERRY_GRADM` | 生态与生地化 | `nl_colm` | Real | `-1.0_r8` |  |  |  | Ball-Berry slope; valid override > 1.6 |
| 231 | `DEF_FERT_SOURCE` | 生态与生地化 | `nl_colm` | Integer | `1` | 1, 2 | CROP |  | Fertilisation data source |
| 232 | `DEF_IRRIGATION_ALLOCATION` | 生态与生地化 | `nl_colm` | Integer | `1` | 1, 2, 3 | CROP |  | irrigation allocated method |
| 233 | `DEF_LC_BETA` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 234 | `DEF_LC_BINTER` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 235 | `DEF_LC_C3C4` | 生态与生地化 | `nl_colm` | Integer | `-1` |  |  |  |  |
| 236 | `DEF_LC_CHIL` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 237 | `DEF_LC_CK` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 238 | `DEF_LC_D50` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 239 | `DEF_LC_DISPLAR` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 240 | `DEF_LC_EFFCON` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 241 | `DEF_LC_EXTKN` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 242 | `DEF_LC_FVEG0` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 243 | `DEF_LC_G0` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 244 | `DEF_LC_G1` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 245 | `DEF_LC_GRADM` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 246 | `DEF_LC_HBOT0` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 247 | `DEF_LC_HHTI` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 248 | `DEF_LC_HLTI` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 249 | `DEF_LC_HTOP0` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 250 | `DEF_LC_KMAX_ROOT` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 251 | `DEF_LC_KMAX_SHA` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 252 | `DEF_LC_KMAX_SUN` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 253 | `DEF_LC_KMAX_XYL` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 254 | `DEF_LC_LAMBDA` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 255 | `DEF_LC_PSI50_ROOT` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 256 | `DEF_LC_PSI50_SHA` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 257 | `DEF_LC_PSI50_SUN` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 258 | `DEF_LC_PSI50_XYL` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 259 | `DEF_LC_RESPCP` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 260 | `DEF_LC_RHOL_NIR` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 261 | `DEF_LC_RHOL_VIS` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 262 | `DEF_LC_RHOS_NIR` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 263 | `DEF_LC_RHOS_VIS` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 264 | `DEF_LC_SAI0` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 265 | `DEF_LC_SHTI` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 266 | `DEF_LC_SLTI` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 267 | `DEF_LC_SQRTDI` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 268 | `DEF_LC_TAUL_NIR` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 269 | `DEF_LC_TAUL_VIS` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 270 | `DEF_LC_TAUS_NIR` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 271 | `DEF_LC_TAUS_VIS` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 272 | `DEF_LC_TRDA` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 273 | `DEF_LC_TRDM` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 274 | `DEF_LC_TROP` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 275 | `DEF_LC_VMAX25` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 276 | `DEF_LC_YEAR` | 生态与生地化 | `nl_colm` | Integer | `2005` |  |  |  |  |
| 277 | `DEF_LC_Z0MR` | 生态与生地化 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 278 | `DEF_MEDLYN_G0` | 生态与生地化 | `nl_colm` | Real | `-1.0_r8` |  |  |  | Medlyn intercept; valid override >= 0 |
| 279 | `DEF_MEDLYN_G1` | 生态与生地化 | `nl_colm` | Real | `-1.0_r8` |  |  |  | Medlyn slope; valid override >= 0 |
| 280 | `DEF_NDEP_FREQUENCY` | 生态与生地化 | `nl_colm` | Integer | `1` | 1, 2 |  |  |  |
| 281 | `DEF_OZONE_KO3` | 生态与生地化 | `nl_colm` | Real | `1.51_r8` |  |  |  | Ozone stomatal-resistance coefficient [-] |
| 282 | `DEF_PH_CROOT_LATERAL_LENGTH` | 生态与生地化 | `nl_colm` | Real | `0.25_r8` |  |  |  | Coarse-root lateral length [m] |
| 283 | `DEF_PH_FROOT_CARBON` | 生态与生地化 | `nl_colm` | Real | `288.392056287006_r8` |  |  |  | Fine-root carbon stock [g C m-2] |
| 284 | `DEF_PH_FROOT_LEAF` | 生态与生地化 | `nl_colm` | Real | `1.5_r8` |  |  |  | Fine-root to leaf-area allocation factor [-] |
| 285 | `DEF_PH_KRMAX` | 生态与生地化 | `nl_colm` | Real | `3.981071705534969e-009_r8` |  |  |  | Maximum radial-root conductance coefficient |
| 286 | `DEF_PH_K_AXS` | 生态与生地化 | `nl_colm` | Real | `2.0e-1_r8` |  |  |  | Root axial hydraulic-conductivity coefficient |
| 287 | `DEF_PH_ROOT_DENSITY` | 生态与生地化 | `nl_colm` | Real | `310000._r8` |  |  |  | Root tissue density [g biomass m-3] |
| 288 | `DEF_PH_ROOT_RADIUS` | 生态与生地化 | `nl_colm` | Real | `2.9e-4_r8` |  |  |  | Fine-root radius [m] |
| 289 | `DEF_PROSPECT` | 生态与生地化 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 290 | `DEF_RSTFAC` | 生态与生地化 | `nl_colm` | Integer | `1` | 1, 2 |  |  | root resistance factors option |
| 291 | `DEF_SSP` | 生态与生地化 | `nl_colm` | Character { len: 256 } | `off` | 126, 245, 370, 585, off |  |  | CMIP6 future CO2 path; off holds the last observed value. |
| 292 | `DEF_TUNING_CROP_PLANTING_DAY` | 生态与生地化 | `nl_colm` | Real | `0.0_r8` |  | CROP |  | Single-point planting day; 0 reads runtime data [day of year] |
| 293 | `DEF_TUNING_CSOILC` | 生态与生地化 | `nl_colm` | Real | `0.004_r8` |  |  |  | Soil drag coefficient below canopy [-] |
| 294 | `DEF_TUNING_DEWMX` | 生态与生地化 | `nl_colm` | Real | `0.1_r8` |  |  |  | Maximum canopy dew storage [mm] |
| 295 | `DEF_TUNING_IRRIGATION_DURATION_SEC` | 生态与生地化 | `nl_colm` | Real | `14400.0_r8` |  | CROP |  | Irrigation application duration [s] |
| 296 | `DEF_TUNING_IRRIGATION_MAX_CPHASE` | 生态与生地化 | `nl_colm` | Real | `4.0_r8` |  | CROP |  | Exclusive last irrigated crop phase [-] |
| 297 | `DEF_TUNING_IRRIGATION_MAX_DEPTH` | 生态与生地化 | `nl_colm` | Real | `1.0_r8` |  | CROP |  | Maximum irrigated soil depth [m] |
| 298 | `DEF_TUNING_IRRIGATION_MIN_CPHASE` | 生态与生地化 | `nl_colm` | Real | `1.0_r8` |  | CROP |  | First irrigated crop phase [-] |
| 299 | `DEF_TUNING_IRRIGATION_PONDMX` | 生态与生地化 | `nl_colm` | Real | `100.0_r8` |  |  |  | Paddy maximum ponding depth [mm] |
| 300 | `DEF_TUNING_IRRIGATION_START_SEC` | 生态与生地化 | `nl_colm` | Real | `21600.0_r8` |  | CROP |  | Local irrigation start second [s] |
| 301 | `DEF_TUNING_IRRIGATION_SUPPLY_FRACTION` | 生态与生地化 | `nl_colm` | Real | `1.0_r8` |  | CROP |  | Irrigation deficit supply fraction [-] |
| 302 | `DEF_TUNING_IRRIGATION_THRESHOLD_FRACTION` | 生态与生地化 | `nl_colm` | Real | `1.0_r8` |  | CROP |  | Irrigation trigger fraction [-] |
| 303 | `DEF_TUNING_TRSMX0` | 生态与生地化 | `nl_colm` | Real | `2.e-4_r8` |  |  |  | Maximum transpiration rate [mm s-1] |
| 304 | `DEF_USE_BGC` | 生态与生地化 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 305 | `DEF_USE_CNSOYFIXN` | 生态与生地化 | `nl_colm` | Logical | `.true.` |  |  |  | Soy nitrogen fixation |
| 306 | `DEF_USE_DiagMatrix` | 生态与生地化 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 307 | `DEF_USE_FERT` | 生态与生地化 | `nl_colm` | Logical | `.true.` |  |  |  | Fertilisation on crop |
| 308 | `DEF_USE_FIRE` | 生态与生地化 | `nl_colm` | Logical | `.false.` |  |  |  | Fire MODULE |
| 309 | `DEF_USE_IRRIGATION` | 生态与生地化 | `nl_colm` | Logical | `.false.` |  |  |  | use irrigation |
| 310 | `DEF_USE_LAIFEEDBACK` | 生态与生地化 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 311 | `DEF_USE_MEDLYNST` | 生态与生地化 | `nl_colm` | Logical | `.false.` |  |  |  | Medlyn stomata model |
| 312 | `DEF_USE_NITRIF` | 生态与生地化 | `nl_colm` | Logical | `.true.` |  |  |  | Nitrification and denitrification switch |
| 313 | `DEF_USE_NOSTRESSNITROGEN` | 生态与生地化 | `nl_colm` | Logical | `.false.` |  |  |  | photosynthesis stress option |
| 314 | `DEF_USE_OZONEDATA` | 生态与生地化 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 315 | `DEF_USE_OZONESTRESS` | 生态与生地化 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 316 | `DEF_USE_PLANTHYDRAULICS` | 生态与生地化 | `nl_colm` | Logical | `.true.` |  |  |  | Plant Hydraulics |
| 317 | `DEF_USE_PN` | 生态与生地化 | `nl_colm` | Logical | `.false.` |  |  |  | Punctuated nitrogen addition Spin up |
| 318 | `DEF_USE_SASU` | 生态与生地化 | `nl_colm` | Logical | `.false.` |  |  |  | Semi-Analytic-Spin-Up |
| 319 | `DEF_USE_SNICAR` | 生态与生地化 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 320 | `DEF_USE_WUEST` | 生态与生地化 | `nl_colm` | Logical | `.true.` |  |  |  | WUE stomata model |
| 321 | `DEF_VEG_SNOW` | 生态与生地化 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 322 | `DEF_WUE_LAMBDA` | 生态与生地化 | `nl_colm` | Real | `-1.0_r8` |  |  |  | WUE marginal water cost; valid override > 0 |
| 323 | `DEF_file_Ozone` | 生态与生地化 | `nl_colm` | Character { len: 256 } | `null` |  |  |  |  |
| 324 | `DEF_file_snowaging` | 生态与生地化 | `nl_colm` | Character { len: 256 } | `null` |  |  |  |  |
| 325 | `DEF_file_snowoptics` | 生态与生地化 | `nl_colm` | Character { len: 256 } | `null` |  |  |  |  |

## 站点

| # | 字段 | 分组 | 文件 | 类型 | 默认值 | 可选值 | 依赖宏 | 派生 | 说明 |
|---|---|---|---|---|---|---|---|---|---|
| 326 | `SITE_fsitedata` | 站点 | `nl_colm` | Character { len: 256 } | `null` |  |  |  |  |
| 327 | `SITE_landtype` | 站点 | `nl_colm` | Integer | `-1` |  |  |  |  |
| 328 | `SITE_lat_location` | 站点 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 329 | `SITE_lon_location` | 站点 | `nl_colm` | Real | `-1.e36_r8` |  |  |  |  |
| 330 | `USE_SITE_ForcingReadAhead` | 站点 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 331 | `USE_SITE_LAI` | 站点 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 332 | `USE_SITE_dbedrock` | 站点 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 333 | `USE_SITE_htop` | 站点 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 334 | `USE_SITE_lakedepth` | 站点 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 335 | `USE_SITE_landtype` | 站点 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 336 | `USE_SITE_pctcrop` | 站点 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 337 | `USE_SITE_pctpfts` | 站点 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 338 | `USE_SITE_soilparameters` | 站点 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 339 | `USE_SITE_soilreflectance` | 站点 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 340 | `USE_SITE_topography` | 站点 | `nl_colm` | Logical | `.true.` |  |  |  |  |

## 算例

| # | 字段 | 分组 | 文件 | 类型 | 默认值 | 可选值 | 依赖宏 | 派生 | 说明 |
|---|---|---|---|---|---|---|---|---|---|
| 341 | `DEF_CASE_NAME` | 算例 | `nl_colm` | Character { len: 256 } | `CASENAME` |  |  |  |  |

## 网格与并行

| # | 字段 | 分组 | 文件 | 类型 | 默认值 | 可选值 | 依赖宏 | 派生 | 说明 |
|---|---|---|---|---|---|---|---|---|---|
| 342 | `DEF_AverageElementSize` | 网格与并行 | `nl_colm` | Real | `-1.` |  |  |  |  |
| 343 | `DEF_BlockInfoFile` | 网格与并行 | `nl_colm` | Character { len: 256 } | `null` |  |  |  |  |
| 344 | `DEF_CatchmentMesh_data` | 网格与并行 | `nl_colm` | Character { len: 256 } | `path/to/catchment/data` |  |  |  |  |
| 345 | `DEF_GRIDBASED_lat_res` | 网格与并行 | `nl_colm` | Real | `0.5` |  | GRIDBASED |  |  |
| 346 | `DEF_GRIDBASED_lon_res` | 网格与并行 | `nl_colm` | Real | `0.5` |  | GRIDBASED |  |  |
| 347 | `DEF_PIO_groupsize` | 网格与并行 | `nl_colm` | Integer | `12` |  |  |  |  |
| 348 | `DEF_domain%edgee` | 网格与并行 | `nl_colm` | Real | `180.0` |  |  |  |  |
| 349 | `DEF_domain%edgen` | 网格与并行 | `nl_colm` | Real | `90.0` |  |  |  |  |
| 350 | `DEF_domain%edges` | 网格与并行 | `nl_colm` | Real | `-90.0` |  |  |  |  |
| 351 | `DEF_domain%edgew` | 网格与并行 | `nl_colm` | Real | `-180.0` |  |  |  |  |
| 352 | `DEF_file_mesh` | 网格与并行 | `nl_colm` | Character { len: 256 } | `path/to/mesh/file` |  |  |  |  |
| 353 | `DEF_file_mesh_filter` | 网格与并行 | `nl_colm` | Character { len: 256 } | `path/to/mesh/filter` |  |  |  |  |
| 354 | `DEF_nIO_eq_nBlock` | 网格与并行 | `nl_colm` | Logical | `.false.` |  | USEMPI |  |  |
| 355 | `DEF_nx_blocks` | 网格与并行 | `nl_colm` | Integer | `72` |  |  |  |  |
| 356 | `DEF_ny_blocks` | 网格与并行 | `nl_colm` | Integer | `36` |  |  |  |  |

## 调试与诊断

| # | 字段 | 分组 | 文件 | 类型 | 默认值 | 可选值 | 依赖宏 | 派生 | 说明 |
|---|---|---|---|---|---|---|---|---|---|
| 357 | `DEF_USE_CoLMDEBUG` | 调试与诊断 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 358 | `DEF_USE_RangeCheck` | 调试与诊断 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 359 | `DEF_USE_SrfdataDiag` | 调试与诊断 | `nl_colm` | Logical | `.false.` |  |  |  |  |

## 输出与重启

| # | 字段 | 分组 | 文件 | 类型 | 默认值 | 可选值 | 依赖宏 | 派生 | 说明 |
|---|---|---|---|---|---|---|---|---|---|
| 360 | `DEF_dir_history` | 输出与重启 | `` | Character { len: 256 } | `path/to/history` |  |  | 是 |  |
| 361 | `DEF_dir_restart` | 输出与重启 | `` | Character { len: 256 } | `path/to/restart` |  |  | 是 |  |
| 362 | `DEF_HISTORY_IN_VECTOR` | 输出与重启 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 363 | `DEF_HIST_CompressLevel` | 输出与重启 | `nl_colm` | Integer | `1` |  |  |  |  |
| 364 | `DEF_HIST_FREQ` | 输出与重启 | `nl_colm` | Character { len: 256 } | `none` | DAILY, HOURLY, MONTHLY, TIMESTEP, YEARLY, none |  |  | write history file frequency: TIMESTEP/HOURLY/DAILY/MONTHLY/YEARLY |
| 365 | `DEF_HIST_WriteBack` | 输出与重启 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 366 | `DEF_HIST_grid_as_forcing` | 输出与重启 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 367 | `DEF_HIST_grid_as_model_mesh` | 输出与重启 | `nl_colm` | Logical | `.false.` |  |  |  |  |
| 368 | `DEF_HIST_groupby` | 输出与重启 | `nl_colm` | Character { len: 256 } | `MONTH` | DAY, MONTH, YEAR |  |  | history file in one file: DAY/MONTH/YEAR |
| 369 | `DEF_HIST_lat_res` | 输出与重启 | `nl_colm` | Real | `0.5` |  |  |  |  |
| 370 | `DEF_HIST_lon_res` | 输出与重启 | `nl_colm` | Real | `0.5` |  |  |  |  |
| 371 | `DEF_HIST_mode` | 输出与重启 | `nl_colm` | Character { len: 256 } | `one` | block, one |  |  |  |
| 372 | `DEF_HIST_vars_namelist` | 输出与重启 | `nl_colm` | Character { len: 256 } | `null` |  |  |  |  |
| 373 | `DEF_HIST_vars_out_default` | 输出与重启 | `nl_colm` | Logical | `.true.` |  |  |  |  |
| 374 | `DEF_Output_2mWMO` | 输出与重启 | `nl_colm` | Logical | `.false.` |  |  |  | 2m WMO temperature |
| 375 | `DEF_REST_CompressLevel` | 输出与重启 | `nl_colm` | Integer | `1` |  |  |  |  |
| 376 | `DEF_WRST_FREQ` | 输出与重启 | `nl_colm` | Character { len: 256 } | `none` | DAILY, HOURLY, MONTHLY, TIMESTEP, YEARLY, none |  |  | write restart file frequency: TIMESTEP/HOURLY/DAILY/MONTHLY/YEARLY |
| 377 | `DEF_dir_output` | 输出与重启 | `nl_colm` | Character { len: 256 } | `path/to/output/data` |  | CatchLateralFlow |  |  |
| 378 | `USE_SITE_HistWriteBack` | 输出与重启 | `nl_colm` | Logical | `.true.` |  |  |  |  |

## 输出变量

| # | 字段 | 分组 | 文件 | 类型 | 默认值 | 可选值 | 依赖宏 | 派生 | 说明 |
|---|---|---|---|---|---|---|---|---|---|
| 379 | `DEF_hist_vars%BD_all` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 380 | `DEF_hist_vars%CONC_O2_UNSAT` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 381 | `DEF_hist_vars%DA_t_brt_fy3d` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 382 | `DEF_hist_vars%DA_t_brt_fy3d_a` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 383 | `DEF_hist_vars%DA_t_brt_fy3d_ens` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 384 | `DEF_hist_vars%DA_t_brt_fy3d_ens_std` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 385 | `DEF_hist_vars%DA_t_brt_smap` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 386 | `DEF_hist_vars%DA_t_brt_smap_a` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 387 | `DEF_hist_vars%DA_t_brt_smap_ens` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 388 | `DEF_hist_vars%DA_t_brt_smap_ens_std` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 389 | `DEF_hist_vars%DA_t_soisno_5cm` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 390 | `DEF_hist_vars%DA_t_soisno_5cm_a` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 391 | `DEF_hist_vars%DA_t_soisno_5cm_ens_std` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 392 | `DEF_hist_vars%DA_t_soisno_ens` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 393 | `DEF_hist_vars%DA_wliq_h2osoi_5cm` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 394 | `DEF_hist_vars%DA_wliq_h2osoi_5cm_a` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 395 | `DEF_hist_vars%DA_wliq_soisno_5cm_ens_std` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 396 | `DEF_hist_vars%DA_wliq_soisno_ens` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 397 | `DEF_hist_vars%O2_DECOMP_DEPTH_UNSAT` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 398 | `DEF_hist_vars%OM_density` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 399 | `DEF_hist_vars%abm` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 400 | `DEF_hist_vars%alb` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 401 | `DEF_hist_vars%alb_hires` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 402 | `DEF_hist_vars%ar` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 403 | `DEF_hist_vars%assim` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 404 | `DEF_hist_vars%assimsha` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 405 | `DEF_hist_vars%assimsun` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 406 | `DEF_hist_vars%baf_crop` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 407 | `DEF_hist_vars%baf_peatf` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 408 | `DEF_hist_vars%bedout` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 409 | `DEF_hist_vars%btran2` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 410 | `DEF_hist_vars%col_fire_closs` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 411 | `DEF_hist_vars%col_fire_nloss` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 412 | `DEF_hist_vars%cphase` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 413 | `DEF_hist_vars%cropprod1c` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 414 | `DEF_hist_vars%cropprod1c_loss` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 415 | `DEF_hist_vars%cropprodc_irrigated_cotton` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 416 | `DEF_hist_vars%cropprodc_irrigated_rice` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 417 | `DEF_hist_vars%cropprodc_irrigated_spwheat` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 418 | `DEF_hist_vars%cropprodc_irrigated_sugarcane` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 419 | `DEF_hist_vars%cropprodc_irrigated_temp_corn` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 420 | `DEF_hist_vars%cropprodc_irrigated_temp_soybean` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 421 | `DEF_hist_vars%cropprodc_irrigated_trop_corn` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 422 | `DEF_hist_vars%cropprodc_irrigated_trop_soybean` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 423 | `DEF_hist_vars%cropprodc_irrigated_wtwheat` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 424 | `DEF_hist_vars%cropprodc_rainfed_cotton` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 425 | `DEF_hist_vars%cropprodc_rainfed_rice` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 426 | `DEF_hist_vars%cropprodc_rainfed_spwheat` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 427 | `DEF_hist_vars%cropprodc_rainfed_sugarcane` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 428 | `DEF_hist_vars%cropprodc_rainfed_temp_corn` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 429 | `DEF_hist_vars%cropprodc_rainfed_temp_soybean` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 430 | `DEF_hist_vars%cropprodc_rainfed_trop_corn` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 431 | `DEF_hist_vars%cropprodc_rainfed_trop_soybean` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 432 | `DEF_hist_vars%cropprodc_rainfed_wtwheat` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 433 | `DEF_hist_vars%cropprodc_unmanagedcrop` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 434 | `DEF_hist_vars%cropseedc_deficit` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 435 | `DEF_hist_vars%cwdcCap_vr` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 436 | `DEF_hist_vars%cwdc_vr` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 437 | `DEF_hist_vars%cwddecomp` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 438 | `DEF_hist_vars%cwdnCap_vr` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 439 | `DEF_hist_vars%cwdn_vr` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 440 | `DEF_hist_vars%cwdprod` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 441 | `DEF_hist_vars%deadcrootc` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 442 | `DEF_hist_vars%deadcrootcCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 443 | `DEF_hist_vars%deadcrootc_storage` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 444 | `DEF_hist_vars%deadcrootc_storageCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 445 | `DEF_hist_vars%deadcrootc_xfer` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 446 | `DEF_hist_vars%deadcrootc_xferCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 447 | `DEF_hist_vars%deadcrootn` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 448 | `DEF_hist_vars%deadcrootnCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 449 | `DEF_hist_vars%deadcrootn_storage` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 450 | `DEF_hist_vars%deadcrootn_storageCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 451 | `DEF_hist_vars%deadcrootn_xfer` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 452 | `DEF_hist_vars%deadcrootn_xferCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 453 | `DEF_hist_vars%deadstemc` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 454 | `DEF_hist_vars%deadstemcCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 455 | `DEF_hist_vars%deadstemc_storage` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 456 | `DEF_hist_vars%deadstemc_storageCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 457 | `DEF_hist_vars%deadstemc_xfer` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 458 | `DEF_hist_vars%deadstemc_xferCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 459 | `DEF_hist_vars%deadstemn` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 460 | `DEF_hist_vars%deadstemnCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 461 | `DEF_hist_vars%deadstemn_storage` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 462 | `DEF_hist_vars%deadstemn_storageCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 463 | `DEF_hist_vars%deadstemn_xfer` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 464 | `DEF_hist_vars%deadstemn_xferCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 465 | `DEF_hist_vars%discharge` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 466 | `DEF_hist_vars%downreg` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 467 | `DEF_hist_vars%dz_lake` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 468 | `DEF_hist_vars%emis` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 469 | `DEF_hist_vars%etr` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 470 | `DEF_hist_vars%etrsha` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 471 | `DEF_hist_vars%etrsun` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 472 | `DEF_hist_vars%fach` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 473 | `DEF_hist_vars%farea_burned` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 474 | `DEF_hist_vars%fert_to_sminn` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 475 | `DEF_hist_vars%fertnitro_corn` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 476 | `DEF_hist_vars%fertnitro_cotton` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 477 | `DEF_hist_vars%fertnitro_rice1` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 478 | `DEF_hist_vars%fertnitro_rice2` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 479 | `DEF_hist_vars%fertnitro_soybean` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 480 | `DEF_hist_vars%fertnitro_sugarcane` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 481 | `DEF_hist_vars%fertnitro_swheat` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 482 | `DEF_hist_vars%fertnitro_wwheat` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 483 | `DEF_hist_vars%fevpa` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 484 | `DEF_hist_vars%fevpg` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 485 | `DEF_hist_vars%fevpl` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 486 | `DEF_hist_vars%fgrnd` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 487 | `DEF_hist_vars%fh` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 488 | `DEF_hist_vars%fhac` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 489 | `DEF_hist_vars%fhah` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 490 | `DEF_hist_vars%fldarea` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 491 | `DEF_hist_vars%floodarea` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 492 | `DEF_hist_vars%floodfrc` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 493 | `DEF_hist_vars%fm` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 494 | `DEF_hist_vars%fm10m` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 495 | `DEF_hist_vars%fpg` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 496 | `DEF_hist_vars%fpi` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 497 | `DEF_hist_vars%fq` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 498 | `DEF_hist_vars%frcsat` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 499 | `DEF_hist_vars%frootc` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 500 | `DEF_hist_vars%frootcCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 501 | `DEF_hist_vars%frootc_storage` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 502 | `DEF_hist_vars%frootc_storageCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 503 | `DEF_hist_vars%frootc_xfer` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 504 | `DEF_hist_vars%frootc_xferCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 505 | `DEF_hist_vars%frootn` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 506 | `DEF_hist_vars%frootnCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 507 | `DEF_hist_vars%frootn_storage` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 508 | `DEF_hist_vars%frootn_storageCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 509 | `DEF_hist_vars%frootn_xfer` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 510 | `DEF_hist_vars%frootn_xferCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 511 | `DEF_hist_vars%fsen_gimp` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 512 | `DEF_hist_vars%fsen_gper` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 513 | `DEF_hist_vars%fsen_roof` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 514 | `DEF_hist_vars%fsen_urbl` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 515 | `DEF_hist_vars%fsen_wsha` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 516 | `DEF_hist_vars%fsen_wsun` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 517 | `DEF_hist_vars%fsena` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 518 | `DEF_hist_vars%fseng` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 519 | `DEF_hist_vars%fsenl` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 520 | `DEF_hist_vars%fsno` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 521 | `DEF_hist_vars%fuelc` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 522 | `DEF_hist_vars%fwst` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 523 | `DEF_hist_vars%gddmaturity` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 524 | `DEF_hist_vars%gddplant` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 525 | `DEF_hist_vars%gdp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 526 | `DEF_hist_vars%gpp` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 527 | `DEF_hist_vars%gpp_c3arcgrass` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 12 |
| 528 | `DEF_hist_vars%gpp_c3grass` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 13 |
| 529 | `DEF_hist_vars%gpp_c4grass` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 14 |
| 530 | `DEF_hist_vars%gpp_dbfboreal` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 8 |
| 531 | `DEF_hist_vars%gpp_dbftemp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 7 |
| 532 | `DEF_hist_vars%gpp_dbftrop` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 6 |
| 533 | `DEF_hist_vars%gpp_dbsboreal` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 11 |
| 534 | `DEF_hist_vars%gpp_dbstemp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 10 |
| 535 | `DEF_hist_vars%gpp_dnfboreal` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 3 |
| 536 | `DEF_hist_vars%gpp_ebftemp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 5 |
| 537 | `DEF_hist_vars%gpp_ebftrop` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 4 |
| 538 | `DEF_hist_vars%gpp_ebstemp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 9 |
| 539 | `DEF_hist_vars%gpp_enfboreal` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 2 |
| 540 | `DEF_hist_vars%gpp_enftemp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 1 |
| 541 | `DEF_hist_vars%grainc` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 542 | `DEF_hist_vars%grainc_storage` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 543 | `DEF_hist_vars%grainc_to_cropprodc` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 544 | `DEF_hist_vars%grainc_to_seed` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 545 | `DEF_hist_vars%grainc_xfer` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 546 | `DEF_hist_vars%grainn` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 547 | `DEF_hist_vars%grainn_storage` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 548 | `DEF_hist_vars%grainn_xfer` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 549 | `DEF_hist_vars%green` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 550 | `DEF_hist_vars%groundwater_demand` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 551 | `DEF_hist_vars%groundwater_supply` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 552 | `DEF_hist_vars%gssha` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 553 | `DEF_hist_vars%gssun` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 554 | `DEF_hist_vars%h2osoi` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 555 | `DEF_hist_vars%hdm` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 556 | `DEF_hist_vars%hr` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 557 | `DEF_hist_vars%hui` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 558 | `DEF_hist_vars%huiswheat` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 559 | `DEF_hist_vars%irrig_method_corn` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 560 | `DEF_hist_vars%irrig_method_cotton` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 561 | `DEF_hist_vars%irrig_method_rice1` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 562 | `DEF_hist_vars%irrig_method_rice2` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 563 | `DEF_hist_vars%irrig_method_soybean` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 564 | `DEF_hist_vars%irrig_method_sugarcane` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 565 | `DEF_hist_vars%irrig_method_swheat` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 566 | `DEF_hist_vars%irrig_method_wwheat` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 567 | `DEF_hist_vars%lai` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 568 | `DEF_hist_vars%lai_c3arcgrass` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 12 |
| 569 | `DEF_hist_vars%lai_c3grass` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 13 |
| 570 | `DEF_hist_vars%lai_c4grass` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 14 |
| 571 | `DEF_hist_vars%lai_dbfboreal` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 8 |
| 572 | `DEF_hist_vars%lai_dbftemp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 7 |
| 573 | `DEF_hist_vars%lai_dbftrop` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 6 |
| 574 | `DEF_hist_vars%lai_dbsboreal` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 11 |
| 575 | `DEF_hist_vars%lai_dbstemp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 10 |
| 576 | `DEF_hist_vars%lai_dnfboreal` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 3 |
| 577 | `DEF_hist_vars%lai_ebftemp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 5 |
| 578 | `DEF_hist_vars%lai_ebftrop` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 4 |
| 579 | `DEF_hist_vars%lai_ebstemp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 9 |
| 580 | `DEF_hist_vars%lai_enfboreal` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 2 |
| 581 | `DEF_hist_vars%lai_enftemp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 1 |
| 582 | `DEF_hist_vars%laisha` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 583 | `DEF_hist_vars%laisun` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 584 | `DEF_hist_vars%lake_deficit` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 585 | `DEF_hist_vars%lake_icefrac` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 586 | `DEF_hist_vars%ldew` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 587 | `DEF_hist_vars%leafc` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 588 | `DEF_hist_vars%leafcCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 589 | `DEF_hist_vars%leafc_c3arcgrass` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 12 |
| 590 | `DEF_hist_vars%leafc_c3grass` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 13 |
| 591 | `DEF_hist_vars%leafc_c4grass` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 14 |
| 592 | `DEF_hist_vars%leafc_dbfboreal` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 8 |
| 593 | `DEF_hist_vars%leafc_dbftemp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 7 |
| 594 | `DEF_hist_vars%leafc_dbftrop` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 6 |
| 595 | `DEF_hist_vars%leafc_dbsboreal` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 11 |
| 596 | `DEF_hist_vars%leafc_dbstemp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 10 |
| 597 | `DEF_hist_vars%leafc_dnfboreal` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 3 |
| 598 | `DEF_hist_vars%leafc_ebftemp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 5 |
| 599 | `DEF_hist_vars%leafc_ebftrop` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 4 |
| 600 | `DEF_hist_vars%leafc_ebstemp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 9 |
| 601 | `DEF_hist_vars%leafc_enfboreal` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 2 |
| 602 | `DEF_hist_vars%leafc_enftemp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 1 |
| 603 | `DEF_hist_vars%leafc_storage` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 604 | `DEF_hist_vars%leafc_storageCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 605 | `DEF_hist_vars%leafc_xfer` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 606 | `DEF_hist_vars%leafc_xferCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 607 | `DEF_hist_vars%leafn` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 608 | `DEF_hist_vars%leafnCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 609 | `DEF_hist_vars%leafn_storage` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 610 | `DEF_hist_vars%leafn_storageCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 611 | `DEF_hist_vars%leafn_xfer` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 612 | `DEF_hist_vars%leafn_xferCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 613 | `DEF_hist_vars%lfevp_gimp` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 614 | `DEF_hist_vars%lfevp_gper` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 615 | `DEF_hist_vars%lfevp_roof` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 616 | `DEF_hist_vars%lfevp_urbl` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 617 | `DEF_hist_vars%lfevpa` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 618 | `DEF_hist_vars%litfire` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 619 | `DEF_hist_vars%litr1cCap_vr` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 620 | `DEF_hist_vars%litr1c_vr` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 621 | `DEF_hist_vars%litr1nCap_vr` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 622 | `DEF_hist_vars%litr1n_vr` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 623 | `DEF_hist_vars%litr2cCap_vr` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 624 | `DEF_hist_vars%litr2c_vr` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 625 | `DEF_hist_vars%litr2nCap_vr` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 626 | `DEF_hist_vars%litr2n_vr` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 627 | `DEF_hist_vars%litr3cCap_vr` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 628 | `DEF_hist_vars%litr3c_vr` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 629 | `DEF_hist_vars%litr3nCap_vr` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 630 | `DEF_hist_vars%litr3n_vr` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 631 | `DEF_hist_vars%livecrootc` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 632 | `DEF_hist_vars%livecrootcCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 633 | `DEF_hist_vars%livecrootc_storage` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 634 | `DEF_hist_vars%livecrootc_storageCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 635 | `DEF_hist_vars%livecrootc_xfer` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 636 | `DEF_hist_vars%livecrootc_xferCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 637 | `DEF_hist_vars%livecrootn` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 638 | `DEF_hist_vars%livecrootnCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 639 | `DEF_hist_vars%livecrootn_storage` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 640 | `DEF_hist_vars%livecrootn_storageCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 641 | `DEF_hist_vars%livecrootn_xfer` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 642 | `DEF_hist_vars%livecrootn_xferCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 643 | `DEF_hist_vars%livestemc` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 644 | `DEF_hist_vars%livestemcCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 645 | `DEF_hist_vars%livestemc_storage` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 646 | `DEF_hist_vars%livestemc_storageCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 647 | `DEF_hist_vars%livestemc_xfer` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 648 | `DEF_hist_vars%livestemc_xferCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 649 | `DEF_hist_vars%livestemn` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 650 | `DEF_hist_vars%livestemnCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 651 | `DEF_hist_vars%livestemn_storage` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 652 | `DEF_hist_vars%livestemn_storageCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 653 | `DEF_hist_vars%livestemn_xfer` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 654 | `DEF_hist_vars%livestemn_xferCap` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 655 | `DEF_hist_vars%lnfm` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 656 | `DEF_hist_vars%m_cwd_c_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 657 | `DEF_hist_vars%m_cwd_n_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 658 | `DEF_hist_vars%m_deadcrootc_storage_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 659 | `DEF_hist_vars%m_deadcrootc_storage_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 660 | `DEF_hist_vars%m_deadcrootc_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 661 | `DEF_hist_vars%m_deadcrootc_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 662 | `DEF_hist_vars%m_deadcrootc_xfer_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 663 | `DEF_hist_vars%m_deadcrootc_xfer_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 664 | `DEF_hist_vars%m_deadcrootn_storage_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 665 | `DEF_hist_vars%m_deadcrootn_storage_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 666 | `DEF_hist_vars%m_deadcrootn_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 667 | `DEF_hist_vars%m_deadcrootn_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 668 | `DEF_hist_vars%m_deadcrootn_xfer_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 669 | `DEF_hist_vars%m_deadcrootn_xfer_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 670 | `DEF_hist_vars%m_deadstemc_storage_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 671 | `DEF_hist_vars%m_deadstemc_storage_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 672 | `DEF_hist_vars%m_deadstemc_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 673 | `DEF_hist_vars%m_deadstemc_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 674 | `DEF_hist_vars%m_deadstemc_xfer_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 675 | `DEF_hist_vars%m_deadstemc_xfer_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 676 | `DEF_hist_vars%m_deadstemn_storage_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 677 | `DEF_hist_vars%m_deadstemn_storage_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 678 | `DEF_hist_vars%m_deadstemn_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 679 | `DEF_hist_vars%m_deadstemn_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 680 | `DEF_hist_vars%m_deadstemn_xfer_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 681 | `DEF_hist_vars%m_deadstemn_xfer_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 682 | `DEF_hist_vars%m_frootc_storage_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 683 | `DEF_hist_vars%m_frootc_storage_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 684 | `DEF_hist_vars%m_frootc_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 685 | `DEF_hist_vars%m_frootc_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 686 | `DEF_hist_vars%m_frootc_xfer_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 687 | `DEF_hist_vars%m_frootc_xfer_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 688 | `DEF_hist_vars%m_frootn_storage_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 689 | `DEF_hist_vars%m_frootn_storage_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 690 | `DEF_hist_vars%m_frootn_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 691 | `DEF_hist_vars%m_frootn_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 692 | `DEF_hist_vars%m_frootn_xfer_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 693 | `DEF_hist_vars%m_frootn_xfer_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 694 | `DEF_hist_vars%m_gresp_storage_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 695 | `DEF_hist_vars%m_gresp_storage_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 696 | `DEF_hist_vars%m_gresp_xfer_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 697 | `DEF_hist_vars%m_gresp_xfer_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 698 | `DEF_hist_vars%m_leafc_storage_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 699 | `DEF_hist_vars%m_leafc_storage_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 700 | `DEF_hist_vars%m_leafc_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 701 | `DEF_hist_vars%m_leafc_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 702 | `DEF_hist_vars%m_leafc_xfer_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 703 | `DEF_hist_vars%m_leafc_xfer_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 704 | `DEF_hist_vars%m_leafn_storage_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 705 | `DEF_hist_vars%m_leafn_storage_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 706 | `DEF_hist_vars%m_leafn_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 707 | `DEF_hist_vars%m_leafn_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 708 | `DEF_hist_vars%m_leafn_xfer_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 709 | `DEF_hist_vars%m_leafn_xfer_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 710 | `DEF_hist_vars%m_litr1_c_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 711 | `DEF_hist_vars%m_litr1_n_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 712 | `DEF_hist_vars%m_litr2_c_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 713 | `DEF_hist_vars%m_litr2_n_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 714 | `DEF_hist_vars%m_litr3_c_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 715 | `DEF_hist_vars%m_litr3_n_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 716 | `DEF_hist_vars%m_livecrootc_storage_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 717 | `DEF_hist_vars%m_livecrootc_storage_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 718 | `DEF_hist_vars%m_livecrootc_to_deadcrootc_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 719 | `DEF_hist_vars%m_livecrootc_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 720 | `DEF_hist_vars%m_livecrootc_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 721 | `DEF_hist_vars%m_livecrootc_xfer_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 722 | `DEF_hist_vars%m_livecrootc_xfer_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 723 | `DEF_hist_vars%m_livecrootn_storage_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 724 | `DEF_hist_vars%m_livecrootn_storage_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 725 | `DEF_hist_vars%m_livecrootn_to_deadcrootn_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 726 | `DEF_hist_vars%m_livecrootn_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 727 | `DEF_hist_vars%m_livecrootn_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 728 | `DEF_hist_vars%m_livecrootn_xfer_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 729 | `DEF_hist_vars%m_livecrootn_xfer_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 730 | `DEF_hist_vars%m_livestemc_storage_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 731 | `DEF_hist_vars%m_livestemc_storage_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 732 | `DEF_hist_vars%m_livestemc_to_deadstemc_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 733 | `DEF_hist_vars%m_livestemc_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 734 | `DEF_hist_vars%m_livestemc_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 735 | `DEF_hist_vars%m_livestemc_xfer_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 736 | `DEF_hist_vars%m_livestemc_xfer_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 737 | `DEF_hist_vars%m_livestemn_storage_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 738 | `DEF_hist_vars%m_livestemn_storage_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 739 | `DEF_hist_vars%m_livestemn_to_deadstemn_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 740 | `DEF_hist_vars%m_livestemn_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 741 | `DEF_hist_vars%m_livestemn_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 742 | `DEF_hist_vars%m_livestemn_xfer_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 743 | `DEF_hist_vars%m_livestemn_xfer_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 744 | `DEF_hist_vars%m_retransn_to_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 745 | `DEF_hist_vars%m_retransn_to_litter_fire` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 746 | `DEF_hist_vars%manunitro` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 747 | `DEF_hist_vars%meta` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 748 | `DEF_hist_vars%ndep_to_sminn` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 749 | `DEF_hist_vars%netflw` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 750 | `DEF_hist_vars%nfire` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 751 | `DEF_hist_vars%npp_c3arcgrass` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 12 |
| 752 | `DEF_hist_vars%npp_c3grass` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 13 |
| 753 | `DEF_hist_vars%npp_c4grass` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 14 |
| 754 | `DEF_hist_vars%npp_dbfboreal` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 8 |
| 755 | `DEF_hist_vars%npp_dbftemp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 7 |
| 756 | `DEF_hist_vars%npp_dbftrop` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 6 |
| 757 | `DEF_hist_vars%npp_dbsboreal` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 11 |
| 758 | `DEF_hist_vars%npp_dbstemp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 10 |
| 759 | `DEF_hist_vars%npp_dnfboreal` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 3 |
| 760 | `DEF_hist_vars%npp_ebftemp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 5 |
| 761 | `DEF_hist_vars%npp_ebftrop` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 4 |
| 762 | `DEF_hist_vars%npp_ebstemp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 9 |
| 763 | `DEF_hist_vars%npp_enfboreal` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 2 |
| 764 | `DEF_hist_vars%npp_enftemp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 1 |
| 765 | `DEF_hist_vars%npptoleafc_c3arcgrass` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 12 |
| 766 | `DEF_hist_vars%npptoleafc_c3grass` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 13 |
| 767 | `DEF_hist_vars%npptoleafc_c4grass` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 14 |
| 768 | `DEF_hist_vars%npptoleafc_dbfboreal` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 8 |
| 769 | `DEF_hist_vars%npptoleafc_dbftemp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 7 |
| 770 | `DEF_hist_vars%npptoleafc_dbftrop` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 6 |
| 771 | `DEF_hist_vars%npptoleafc_dbsboreal` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 11 |
| 772 | `DEF_hist_vars%npptoleafc_dbstemp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 10 |
| 773 | `DEF_hist_vars%npptoleafc_dnfboreal` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 3 |
| 774 | `DEF_hist_vars%npptoleafc_ebftemp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 5 |
| 775 | `DEF_hist_vars%npptoleafc_ebftrop` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 4 |
| 776 | `DEF_hist_vars%npptoleafc_ebstemp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 9 |
| 777 | `DEF_hist_vars%npptoleafc_enfboreal` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 2 |
| 778 | `DEF_hist_vars%npptoleafc_enftemp` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  | 1 |
| 779 | `DEF_hist_vars%o3uptakesha` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 780 | `DEF_hist_vars%o3uptakesun` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 781 | `DEF_hist_vars%olrg` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 782 | `DEF_hist_vars%pdcorn` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 783 | `DEF_hist_vars%pdcotton` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 784 | `DEF_hist_vars%pdrice1` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 785 | `DEF_hist_vars%pdrice2` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 786 | `DEF_hist_vars%pdsoybean` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 787 | `DEF_hist_vars%pdsugarcane` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 788 | `DEF_hist_vars%pdswheat` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 789 | `DEF_hist_vars%pdwwheat` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 790 | `DEF_hist_vars%peatf` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 791 | `DEF_hist_vars%pft_fire_closs` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 792 | `DEF_hist_vars%pft_fire_nloss` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 793 | `DEF_hist_vars%plantdate_irrigated_cotton` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 794 | `DEF_hist_vars%plantdate_irrigated_rice` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 795 | `DEF_hist_vars%plantdate_irrigated_spwheat` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 796 | `DEF_hist_vars%plantdate_irrigated_sugarcane` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 797 | `DEF_hist_vars%plantdate_irrigated_temp_corn` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 798 | `DEF_hist_vars%plantdate_irrigated_temp_soybean` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 799 | `DEF_hist_vars%plantdate_irrigated_trop_corn` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 800 | `DEF_hist_vars%plantdate_irrigated_trop_soybean` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 801 | `DEF_hist_vars%plantdate_irrigated_wtwheat` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 802 | `DEF_hist_vars%plantdate_rainfed_cotton` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 803 | `DEF_hist_vars%plantdate_rainfed_rice` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 804 | `DEF_hist_vars%plantdate_rainfed_spwheat` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 805 | `DEF_hist_vars%plantdate_rainfed_sugarcane` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 806 | `DEF_hist_vars%plantdate_rainfed_temp_corn` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 807 | `DEF_hist_vars%plantdate_rainfed_temp_soybean` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 808 | `DEF_hist_vars%plantdate_rainfed_trop_corn` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 809 | `DEF_hist_vars%plantdate_rainfed_trop_soybean` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 810 | `DEF_hist_vars%plantdate_rainfed_wtwheat` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 811 | `DEF_hist_vars%plantdate_unmanagedcrop` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 812 | `DEF_hist_vars%qcharge` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 813 | `DEF_hist_vars%qdrip` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 814 | `DEF_hist_vars%qinfl` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 815 | `DEF_hist_vars%qintr` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 816 | `DEF_hist_vars%qlayer` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 817 | `DEF_hist_vars%qref` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 818 | `DEF_hist_vars%qresv_in` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 819 | `DEF_hist_vars%qresv_out` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 820 | `DEF_hist_vars%qstar` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 821 | `DEF_hist_vars%reflectance_out` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 822 | `DEF_hist_vars%reservoirriver_demand` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 823 | `DEF_hist_vars%reservoirriver_supply` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 824 | `DEF_hist_vars%respc` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 825 | `DEF_hist_vars%retrasn` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 826 | `DEF_hist_vars%rib` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 827 | `DEF_hist_vars%riv_height` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 828 | `DEF_hist_vars%riv_veloct` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 829 | `DEF_hist_vars%rnet` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 830 | `DEF_hist_vars%rnof` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 831 | `DEF_hist_vars%rootr` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 832 | `DEF_hist_vars%rss` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 833 | `DEF_hist_vars%rstfacsha` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 834 | `DEF_hist_vars%rstfacsun` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 835 | `DEF_hist_vars%rsub` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 836 | `DEF_hist_vars%rsur` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 837 | `DEF_hist_vars%rsur_ie` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 838 | `DEF_hist_vars%rsur_se` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 839 | `DEF_hist_vars%sabg` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 840 | `DEF_hist_vars%sabvsha` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 841 | `DEF_hist_vars%sabvsun` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 842 | `DEF_hist_vars%sai` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 843 | `DEF_hist_vars%scv` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 844 | `DEF_hist_vars%sedcon` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 845 | `DEF_hist_vars%sedinp` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 846 | `DEF_hist_vars%sedlayer` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 847 | `DEF_hist_vars%sedout` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 848 | `DEF_hist_vars%sensors` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 849 | `DEF_hist_vars%shearvel` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 850 | `DEF_hist_vars%sigf` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 851 | `DEF_hist_vars%sminn_vr` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 852 | `DEF_hist_vars%snowdp` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 853 | `DEF_hist_vars%soil1cCap_vr` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 854 | `DEF_hist_vars%soil1c_vr` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 855 | `DEF_hist_vars%soil1nCap_vr` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 856 | `DEF_hist_vars%soil1n_vr` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 857 | `DEF_hist_vars%soil2cCap_vr` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 858 | `DEF_hist_vars%soil2c_vr` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 859 | `DEF_hist_vars%soil2nCap_vr` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 860 | `DEF_hist_vars%soil2n_vr` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 861 | `DEF_hist_vars%soil3cCap_vr` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 862 | `DEF_hist_vars%soil3c_vr` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 863 | `DEF_hist_vars%soil3nCap_vr` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 864 | `DEF_hist_vars%soil3n_vr` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 865 | `DEF_hist_vars%sol_dif_ln_hires` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 866 | `DEF_hist_vars%sol_dir_ln_hires` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 867 | `DEF_hist_vars%solnd` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 868 | `DEF_hist_vars%solndln` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 869 | `DEF_hist_vars%solni` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 870 | `DEF_hist_vars%solniln` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 871 | `DEF_hist_vars%solvd` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 872 | `DEF_hist_vars%solvdln` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 873 | `DEF_hist_vars%solvi` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 874 | `DEF_hist_vars%solviln` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 875 | `DEF_hist_vars%somc_fire` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 876 | `DEF_hist_vars%somfire` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 877 | `DEF_hist_vars%sr` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 878 | `DEF_hist_vars%sr_dif_ln_hires` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 879 | `DEF_hist_vars%sr_dir_ln_hires` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 880 | `DEF_hist_vars%srnd` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 881 | `DEF_hist_vars%srndln` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 882 | `DEF_hist_vars%srni` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 883 | `DEF_hist_vars%srniln` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 884 | `DEF_hist_vars%srvd` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 885 | `DEF_hist_vars%srvdln` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 886 | `DEF_hist_vars%srvi` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 887 | `DEF_hist_vars%srviln` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 888 | `DEF_hist_vars%sum_deficit_irrig` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 889 | `DEF_hist_vars%sum_irrig` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 890 | `DEF_hist_vars%sum_irrig_count` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 891 | `DEF_hist_vars%t2m_wmo` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 892 | `DEF_hist_vars%t_grnd` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 893 | `DEF_hist_vars%t_lake` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 894 | `DEF_hist_vars%t_roof` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 895 | `DEF_hist_vars%t_room` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 896 | `DEF_hist_vars%t_scalar` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 897 | `DEF_hist_vars%t_soisno` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 898 | `DEF_hist_vars%t_wall` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 899 | `DEF_hist_vars%tafu` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 900 | `DEF_hist_vars%taux` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 901 | `DEF_hist_vars%tauy` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 902 | `DEF_hist_vars%tleaf` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 903 | `DEF_hist_vars%totcolc` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 904 | `DEF_hist_vars%totcoln` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 905 | `DEF_hist_vars%totcwdc` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 906 | `DEF_hist_vars%totcwdn` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 907 | `DEF_hist_vars%totfire` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 908 | `DEF_hist_vars%totlitc` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 909 | `DEF_hist_vars%totlitn` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 910 | `DEF_hist_vars%totsoiln_vr` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 911 | `DEF_hist_vars%totsomc` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 912 | `DEF_hist_vars%totsomn` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 913 | `DEF_hist_vars%totvegc` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 914 | `DEF_hist_vars%totvegn` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 915 | `DEF_hist_vars%trad` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 916 | `DEF_hist_vars%transmittance_out` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 917 | `DEF_hist_vars%tref` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 918 | `DEF_hist_vars%tstar` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 919 | `DEF_hist_vars%us10m` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 920 | `DEF_hist_vars%ustar` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 921 | `DEF_hist_vars%ustar2` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 922 | `DEF_hist_vars%vegwp` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 923 | `DEF_hist_vars%vehc` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 924 | `DEF_hist_vars%veloc_hru` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 925 | `DEF_hist_vars%vf` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 926 | `DEF_hist_vars%volresv` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 927 | `DEF_hist_vars%vs10m` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 928 | `DEF_hist_vars%w_scalar` | 输出变量 | `nl_colm_history` | Logical | `.false.` |  |  |  |  |
| 929 | `DEF_hist_vars%wa` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 930 | `DEF_hist_vars%wa_inst` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 931 | `DEF_hist_vars%wat` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 932 | `DEF_hist_vars%wat_inst` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 933 | `DEF_hist_vars%waterstorage` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 934 | `DEF_hist_vars%wdsrf` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 935 | `DEF_hist_vars%wdsrf_hru` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 936 | `DEF_hist_vars%wdsrf_inst` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 937 | `DEF_hist_vars%wetwat` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 938 | `DEF_hist_vars%wetwat_inst` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 939 | `DEF_hist_vars%wetzwt` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 940 | `DEF_hist_vars%wfc` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 941 | `DEF_hist_vars%wice_soisno` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 942 | `DEF_hist_vars%wliq_soisno` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 943 | `DEF_hist_vars%xerr` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 944 | `DEF_hist_vars%xsubs_bsn` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 945 | `DEF_hist_vars%xsubs_hru` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 946 | `DEF_hist_vars%xwsub` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 947 | `DEF_hist_vars%xwsur` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 948 | `DEF_hist_vars%xy_frl` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 949 | `DEF_hist_vars%xy_hpbl` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 950 | `DEF_hist_vars%xy_ozone` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 951 | `DEF_hist_vars%xy_pbot` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 952 | `DEF_hist_vars%xy_prc` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 953 | `DEF_hist_vars%xy_prl` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 954 | `DEF_hist_vars%xy_q` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 955 | `DEF_hist_vars%xy_rain` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 956 | `DEF_hist_vars%xy_snow` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 957 | `DEF_hist_vars%xy_solarin` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 958 | `DEF_hist_vars%xy_t` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 959 | `DEF_hist_vars%xy_us` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 960 | `DEF_hist_vars%xy_vs` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 961 | `DEF_hist_vars%z0m` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 962 | `DEF_hist_vars%zerr` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 963 | `DEF_hist_vars%zol` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |
| 964 | `DEF_hist_vars%zwt` | 输出变量 | `nl_colm_history` | Logical | `.true.` |  |  |  |  |

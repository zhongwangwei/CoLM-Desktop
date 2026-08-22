//! 参数字段的用户界面名称与离散方案说明。
//!
//! case.nml 仍保存 CoLM 的原始键和值；这里仅负责展示。技术键放在 tooltip，
//! 主界面不再要求用户先学会 `DEF_*` 命名。方案标签必须带回原始值，便于与
//! 文档和旧 namelist 对照，也保证 onchange 写回的仍是精确 Fortran 字面量。

const pair = (zh, en) => Object.freeze([zh, en]);
const pick = (value, lang) => value?.[lang === 'en' ? 1 : 0];

const LABELS = Object.freeze({
  SITE_fsitedata: pair('站点属性文件', 'Site-property file'),
  SITE_lon_location: pair('站点经度', 'Site longitude'),
  SITE_lat_location: pair('站点纬度', 'Site latitude'),
  SITE_landtype: pair('地表覆盖类型', 'Land-cover type'),
  USE_SITE_landtype: pair('从站点文件读取地表类型', 'Read land-cover type from site file'),
  USE_SITE_pctpfts: pair('从站点文件读取 PFT 比例', 'Read PFT fractions from site file'),
  USE_SITE_pctcrop: pair('从站点文件读取作物比例', 'Read crop fractions from site file'),
  USE_SITE_htop: pair('从站点文件读取植被高度', 'Read canopy height from site file'),
  USE_SITE_LAI: pair('从站点文件读取叶面积指数', 'Read leaf-area index from site file'),
  USE_SITE_lakedepth: pair('从站点文件读取湖泊深度', 'Read lake depth from site file'),
  USE_SITE_soilreflectance: pair('从站点文件读取土壤反射率', 'Read soil reflectance from site file'),
  USE_SITE_soilparameters: pair('从站点文件读取土壤参数', 'Read soil parameters from site file'),
  USE_SITE_dbedrock: pair('从站点文件读取基岩深度', 'Read bedrock depth from site file'),
  USE_SITE_topography: pair('从站点文件读取地形', 'Read topography from site file'),
  USE_SITE_urban_geometry: pair('从站点文件读取城市几何参数', 'Read urban geometry from site file'),
  USE_SITE_urban_ecology: pair('从站点文件读取城市生态参数', 'Read urban ecology from site file'),
  USE_SITE_urban_radiation: pair('从站点文件读取城市辐射参数', 'Read urban radiation from site file'),
  USE_SITE_urban_thermal: pair('从站点文件读取城市热力参数', 'Read urban thermal parameters from site file'),
  USE_SITE_urban_human: pair('从站点文件读取城市人为活动参数', 'Read urban human-activity parameters from site file'),
  USE_SITE_HistWriteBack: pair('站点输出回写', 'Write site history back'),
  USE_SITE_ForcingReadAhead: pair('强迫场预读', 'Read forcing ahead'),

  DEF_LC_YEAR: pair('地表数据年份', 'Land-data year'),
  DEF_SOIL_REFL_SCHEME: pair('土壤反照率来源', 'Soil-albedo source'),
  DEF_LAI_START_YEAR: pair('叶面积指数起始年', 'LAI start year'),
  DEF_LAI_END_YEAR: pair('叶面积指数结束年', 'LAI end year'),
  DEF_LAI_MONTHLY: pair('使用月尺度叶面积指数', 'Use monthly LAI'),
  DEF_LAI_CHANGE_YEARLY: pair('叶面积指数逐年变化', 'Vary LAI by year'),
  DEF_USE_LAIFEEDBACK: pair('启用叶面积指数反馈', 'Enable LAI feedback'),
  DEF_LULCC_SCHEME: pair('土地利用变化状态转移方案', 'Land-use-change transfer scheme'),

  DEF_USE_SoilInit: pair('使用土壤初始场', 'Use soil initial state'),
  DEF_file_SoilInit: pair('土壤初始场文件', 'Soil initial-state file'),
  DEF_USE_SnowInit: pair('使用积雪初始场', 'Use snow initial state'),
  DEF_file_SnowInit: pair('积雪初始场文件', 'Snow initial-state file'),
  DEF_USE_CN_INIT: pair('使用碳氮初始场', 'Use carbon-nitrogen initial state'),
  DEF_file_cn_init: pair('碳氮初始场文件', 'Carbon-nitrogen initial-state file'),
  DEF_USE_WaterTableInit: pair('使用地下水位初始场', 'Use water-table initial state'),
  DEF_file_WaterTable: pair('地下水位初始场文件', 'Water-table initial-state file'),

  DEF_Interception_scheme: pair('冠层截留方案', 'Canopy-interception scheme'),
  DEF_MATSIRO_CWCAP_SCALE: pair('MATSIRO 冠层持水容量系数', 'MATSIRO canopy-water capacity scale'),
  DEF_THERMAL_CONDUCTIVITY_SCHEME: pair('土壤热导率方案', 'Soil thermal-conductivity scheme'),
  DEF_USE_SUPERCOOL_WATER: pair('启用土壤过冷水', 'Enable supercooled soil water'),
  DEF_RSS_SCHEME: pair('土壤表面阻抗方案', 'Soil-surface resistance scheme'),
  DEF_Runoff_SCHEME: pair('产流方案', 'Runoff scheme'),
  DEF_VIC_OPT: pair('使用空间变化的 VIC 参数', 'Use spatially varying VIC parameters'),
  DEF_TOPMOD_method: pair('TOPMODEL 参数来源', 'TOPMODEL parameter source'),
  DEF_SPLIT_SOILSNOW: pair('分别计算裸土与积雪表面', 'Treat bare soil and snow separately'),
  DEF_VEG_SNOW: pair('启用植被积雪过程', 'Enable vegetation-snow process'),
  DEF_USE_VariablySaturatedFlow: pair('启用变饱和土壤水流', 'Enable variably saturated soil flow'),
  DEF_USE_BEDROCK: pair('启用基岩过程', 'Enable bedrock process'),
  DEF_USE_Campbell_SOIL_MODEL: pair('土壤水力模型', 'Soil-hydraulics model'),
  DEF_precip_phase_discrimination_scheme: pair('降水雨雪拆分方案', 'Rain–snow partitioning scheme'),
  DEF_USE_Dynamic_Lake: pair('启用动态湖泊', 'Enable dynamic lake'),
  DEF_USE_Dynamic_Wetland: pair('启用动态湿地', 'Enable dynamic wetland'),

  DEF_USE_OZONESTRESS: pair('启用臭氧胁迫', 'Enable ozone stress'),
  DEF_USE_OZONEDATA: pair('读取臭氧数据', 'Read ozone data'),
  DEF_file_Ozone: pair('臭氧数据文件', 'Ozone-data file'),
  DEF_USE_SNICAR: pair('启用 SNICAR 积雪辐射模型', 'Enable SNICAR snow-radiation model'),
  DEF_Aerosol_Readin: pair('读取气溶胶数据', 'Read aerosol data'),
  DEF_Aerosol_Clim: pair('使用气溶胶气候态', 'Use aerosol climatology'),
  DEF_HighResSoil: pair('高光谱土壤反照率', 'Hyperspectral soil albedo'),
  DEF_HighResVeg: pair('高光谱植被反射率', 'Hyperspectral vegetation reflectance'),
  DEF_PROSPECT: pair('启用 PROSPECT 叶片光学模型', 'Enable PROSPECT leaf optics'),
  DEF_NDEP_FREQUENCY: pair('氮沉降数据时间分辨率', 'Nitrogen-deposition frequency'),
  DEF_SSP: pair('未来 CO₂ 排放情景', 'Future CO₂ scenario'),
  DEF_USE_IRRIGATION: pair('启用灌溉', 'Enable irrigation'),
  DEF_IRRIGATION_ALLOCATION: pair('灌溉供水分配方案', 'Irrigation-water allocation scheme'),
  DEF_USE_NOSTRESSNITROGEN: pair('关闭氮胁迫', 'Disable nitrogen stress'),
  DEF_RSTFAC: pair('根区水分胁迫方案', 'Root-zone water-stress scheme'),
  DEF_USE_PLANTHYDRAULICS: pair('启用植物水力过程', 'Enable plant hydraulics'),
  DEF_USE_MEDLYNST: pair('启用 Medlyn 气孔导度', 'Enable Medlyn stomatal conductance'),
  DEF_USE_WUEST: pair('启用水分利用效率气孔方案', 'Enable WUE stomatal scheme'),
  GUI_STOMATAL_CONDUCTANCE_SCHEME: pair('气孔导度方案', 'Stomatal-conductance scheme'),
  DEF_USE_SASU: pair('启用半解析预热', 'Enable semi-analytic spin-up'),
  DEF_USE_DiagMatrix: pair('输出生地化诊断矩阵', 'Output biogeochemical diagnostic matrix'),
  DEF_USE_PN: pair('启用脉冲加氮预热', 'Enable punctuated-N spin-up'),
  DEF_USE_FERT: pair('启用施肥', 'Enable fertilization'),
  DEF_FERT_SOURCE: pair('施肥数据来源', 'Fertilizer-data source'),
  DEF_USE_NITRIF: pair('启用硝化与反硝化', 'Enable nitrification and denitrification'),
  DEF_USE_CNSOYFIXN: pair('启用大豆固氮', 'Enable soybean nitrogen fixation'),
  DEF_USE_FIRE: pair('启用火灾过程', 'Enable fire process'),
  DEF_CheckEquilibrium: pair('检查碳氮平衡', 'Check carbon-nitrogen equilibrium'),

  DEF_URBAN_type_scheme: pair('城市分类体系', 'Urban classification'),
  DEF_URBAN_RUN: pair('启用城市模型', 'Enable urban model'),
  DEF_URBAN_BEM: pair('启用建筑能耗模型', 'Enable building-energy model'),
  DEF_URBAN_TREE: pair('模拟城市树木', 'Simulate urban trees'),
  DEF_URBAN_WATER: pair('模拟城市水体', 'Simulate urban water'),
  DEF_URBAN_LUCY: pair('启用 LUCY 人为热模型', 'Enable LUCY anthropogenic heat'),
  DEF_USE_CANYON_HWR: pair('计算街谷高宽比', 'Use canyon height-to-width ratio'),
  DEF_HighResUrban_albedo: pair('城市高光谱反照率', 'Hyperspectral urban albedo'),

  DEF_USE_Forcing_Downscaling: pair('完整地形强迫降尺度', 'Full topographic forcing downscaling'),
  DEF_USE_Forcing_Downscaling_Simple: pair('简化地形强迫降尺度', 'Simple topographic forcing downscaling'),
  DEF_DS_HiresTopographyDataDir: pair('高分辨率地形数据目录', 'High-resolution topography directory'),
  DEF_DS_precipitation_adjust_scheme: pair('降水降尺度方案', 'Precipitation-downscaling scheme'),
  DEF_DS_longwave_adjust_scheme: pair('长波辐射降尺度方案', 'Longwave-downscaling scheme'),
  DEF_USE_ClimForcing_for_Spinup: pair('预热期循环使用气候态强迫', 'Use climatological forcing for spin-up'),
  DEF_USE_CBL_HEIGHT: pair('读取边界层高度', 'Read boundary-layer height'),

  DEF_USE_TRACER: pair('启用示踪剂', 'Enable tracers'),
  DEF_TRACER_USE_FRACTIONATION: pair('启用同位素分馏', 'Enable isotope fractionation'),
  DEF_TRACER_KINETIC_SCHEME: pair('气液动理分馏方案', 'Gas–liquid kinetic fractionation'),
  DEF_TRACER_OPEN_WATER_KINETIC: pair('开阔水面动理分馏方案', 'Open-water kinetic fractionation'),
  DEF_TRACER_SOIL_KINETIC: pair('土壤蒸发动理分馏方案', 'Soil-evaporation kinetic fractionation'),
  DEF_TRACER_SOIL_DIFFUSION: pair('启用土壤液相扩散', 'Enable soil liquid diffusion'),
  DEF_TRACER_SOIL_VAPOR_DIFFUSION: pair('启用土壤水汽扩散', 'Enable soil-vapor diffusion'),
  DEF_TRACER_NUM: pair('示踪剂数量', 'Number of tracers'),
  DEF_TRACER_NAMES: pair('示踪剂名称', 'Tracer names'),
  DEF_TRACER_TYPES: pair('示踪剂类型', 'Tracer types'),
  DEF_TRACER_PARAM_FILES: pair('示踪剂参数文件', 'Tracer parameter files'),
  DEF_TRACER_USE_SOIL_INIT: pair('使用示踪剂土壤初始场', 'Use tracer soil initial state'),
  DEF_TRACER_SOIL_INIT_FILE: pair('示踪剂土壤初始场文件', 'Tracer soil initial-state file'),
  DEF_wetland_finundation_scheme: pair('甲烷湿地淹水范围来源', 'Methane wetland-inundation source'),

  DEF_Reservoir_Method: pair('水库调度方案', 'Reservoir-operation scheme'),
  DEF_USE_EstimatedRiverDepth: pair('估算河道深度', 'Estimate river depth'),
  DEF_USE_LEVEE: pair('启用堤防过程', 'Enable levees'),
  DEF_USE_BIFURCATION: pair('启用分汊河道', 'Enable river bifurcation'),

  DEF_DA_obsdir: pair('同化观测目录', 'Assimilation-observation directory'),
  DEF_DA_TWS: pair('同化陆地总储水量', 'Assimilate terrestrial water storage'),
  DEF_DA_TWS_GRACE: pair('使用 GRACE 储水量观测', 'Use GRACE water-storage observations'),
  DEF_DA_SM: pair('同化土壤水分', 'Assimilate soil moisture'),
  DEF_DA_ENS_NUM: pair('同化集合成员数', 'Assimilation ensemble size'),
  DEF_DA_RTM_diel: pair('微波介电模型', 'Microwave dielectric model'),
  DEF_DA_RTM_rough: pair('微波粗糙面模型', 'Microwave rough-surface model'),

  DEF_Output_2mWMO: pair('输出 WMO 标准 2 米气象量', 'Output WMO-standard 2 m diagnostics'),
  DEF_WRST_FREQ: pair('重启文件写出频率', 'Restart-write frequency'),
  DEF_HIST_FREQ: pair('历史输出频率', 'History-output frequency'),
  DEF_HIST_groupby: pair('历史文件分组周期', 'History-file grouping period'),
  DEF_REST_CompressLevel: pair('重启文件压缩级别', 'Restart compression level'),
  DEF_HIST_CompressLevel: pair('历史文件压缩级别', 'History compression level'),
  DEF_HIST_vars_namelist: pair('历史变量配置文件', 'History-variable namelist'),
  DEF_HIST_vars_out_default: pair('默认输出历史变量', 'Output default history variables'),
  DEF_dir_output: pair('输出根目录', 'Output root directory'),
  DEF_dir_restart: pair('重启文件目录', 'Restart directory'),
  DEF_dir_history: pair('历史结果目录', 'History directory'),
  DEF_forcing_namelist: pair('强迫场配置文件', 'Forcing namelist'),
});

const OPTIONS = Object.freeze({
  DEF_SOIL_REFL_SCHEME: {
    1: pair('按地表覆盖类型估算', 'Infer from land cover'),
    2: pair('使用数据中的土壤反照率', 'Use soil albedo from input data'),
  },
  DEF_LULCC_SCHEME: {
    1: pair('STA：同类型状态直接继承', 'STA: assign state within the same type'),
    2: pair('MEC：质量与能量守恒转移', 'MEC: conserve mass and energy'),
  },
  DEF_URBAN_type_scheme: {
    1: pair('NCAR 三类城市分类', 'NCAR three-class urban classification'),
    2: pair('LCZ 1–10 城市分类', 'LCZ 1–10 urban classification'),
  },
  DEF_Interception_scheme: {
    1: pair('CoLM 冠层截留', 'CoLM interception'),
    2: pair('CLM 4.5 冠层截留', 'CLM 4.5 interception'),
    3: pair('CLM 5 冠层截留', 'CLM 5 interception'),
    4: pair('Noah-MP 冠层截留', 'Noah-MP interception'),
    5: pair('MATSIRO 冠层截留', 'MATSIRO interception'),
    6: pair('VIC 冠层截留', 'VIC interception'),
    7: pair('JULES 冠层截留', 'JULES interception'),
    8: pair('CoLM 202x 冠层截留', 'CoLM 202x interception'),
  },
  DEF_THERMAL_CONDUCTIVITY_SCHEME: {
    1: pair('Farouki（1981）', 'Farouki (1981)'),
    2: pair('Johansen（1975）', 'Johansen (1975)'),
    3: pair('Côté–Konrad（2005）', 'Côté–Konrad (2005)'),
    4: pair('Balland–Arp（2005）', 'Balland–Arp (2005)'),
    5: pair('Lu 等（2007）', 'Lu et al. (2007)'),
    6: pair('Tarnawski–Leong（2012）', 'Tarnawski–Leong (2012)'),
    7: pair('De Vries（1963）', 'De Vries (1963)'),
    8: pair('Yan–He 等（2019）', 'Yan–He et al. (2019)'),
  },
  DEF_RSS_SCHEME: {
    0: pair('不计算土壤表面阻抗', 'No soil-surface resistance'),
    1: pair('SL14：Swenson–Lawrence（2014）', 'SL14: Swenson–Lawrence (2014)'),
    2: pair('SZ09：Sakaguchi–Zeng（2009）', 'SZ09: Sakaguchi–Zeng (2009)'),
    3: pair('TR13：Tang–Riley（2013）', 'TR13: Tang–Riley (2013)'),
    4: pair('LP92：Lee–Pielke（1992）', 'LP92: Lee–Pielke (1992)'),
    5: pair('S92：Sellers 等（1992）', 'S92: Sellers et al. (1992)'),
  },
  DEF_Runoff_SCHEME: {
    0: pair('TOPMODEL（CoLM 2014）', 'TOPMODEL (CoLM 2014)'),
    1: pair('VIC', 'VIC'),
    2: pair('新安江 / ECMWF', 'XinAnJiang / ECMWF'),
    3: pair('Simple VIC（Noah-MP 5.0）', 'Simple VIC (Noah-MP 5.0)'),
  },
  DEF_TOPMOD_method: {
    0: pair('固定默认参数', 'Fixed default parameters'),
    1: pair('读取 fsatmax / 衰减系数 / 湿度指数', 'Read fsatmax, decay factor, and wetness index'),
    2: pair('读取平均地形湿度指数', 'Read mean topographic wetness index'),
  },
  DEF_NDEP_FREQUENCY: {
    1: pair('年尺度氮沉降', 'Annual nitrogen deposition'),
    2: pair('月尺度氮沉降', 'Monthly nitrogen deposition'),
  },
  DEF_Reservoir_Method: {
    0: pair('关闭水库调度', 'Disable reservoir operation'),
    1: pair('CaMa-Flood 改进水库调度', 'Improved CaMa-Flood reservoir operation'),
  },
  DEF_wetland_finundation_scheme: {
    1: pair('湿地类型 / wetwat', 'Wetland type / wetwat'),
    2: pair('饱和面积比例', 'Saturated-area fraction'),
    3: pair('地表水面积比例', 'Surface-water fraction'),
    4: pair('湿地类型 + 地表水比例', 'Wetland type plus surface-water fraction'),
    5: pair('GIEMS 卫星月气候态', 'GIEMS satellite monthly climatology'),
    6: pair('动态地下水位', 'Dynamic water table'),
    7: pair('河网洪泛比例', 'River-routing flood fraction'),
  },
  DEF_precip_phase_discrimination_scheme: {
    I: pair('湿球温度经验方案（Wang / Behrangi）', 'Wet-bulb empirical scheme (Wang / Behrangi)'),
    II: pair('气温线性阈值方案', 'Air-temperature linear-threshold scheme'),
    III: pair('湿空气能量平衡方案（Harder–Pomeroy）', 'Psychrometric energy-balance scheme (Harder–Pomeroy)'),
  },
  DEF_SSP: {
    off: pair('关闭（保持 2022 年末浓度）', 'Off (hold the late-2022 concentration)'),
    126: pair('SSP1-2.6：低排放', 'SSP1-2.6: low emissions'),
    245: pair('SSP2-4.5：中等排放', 'SSP2-4.5: intermediate emissions'),
    370: pair('SSP3-7.0：高排放', 'SSP3-7.0: high emissions'),
    585: pair('SSP5-8.5：极高排放', 'SSP5-8.5: very high emissions'),
  },
  DEF_IRRIGATION_ALLOCATION: {
    1: pair('不限供水，完全满足灌溉需求', 'Unlimited supply; meet the full demand'),
    2: pair('依次使用本地储水、河库和地下水', 'Use local storage, river/reservoir, then groundwater'),
    3: pair('按比例分配地表水与地下水', 'Split demand between surface water and groundwater'),
  },
  DEF_RSTFAC: {
    1: pair('土壤水势加权方案', 'Soil-water-potential weighting'),
    2: pair('萎蔫点—田间持水量方案', 'Wilting-point to field-capacity scheme'),
  },
  DEF_FERT_SOURCE: {
    1: pair('CoLM 作物氮肥数据', 'CoLM crop nitrogen-fertilizer data'),
    2: pair('2015SOC 肥料与粪肥数据', '2015SOC fertilizer and manure data'),
  },
  DEF_DS_precipitation_adjust_scheme: {
    I: pair('Tesfa 等：按相对高差线性调整', 'Tesfa et al.: linear relative-elevation adjustment'),
    II: pair('MicroMet：Liston–Elder 地形调整', 'MicroMet: Liston–Elder terrain adjustment'),
    III: pair('Chen 等：MPI / Python 区域降尺度', 'Chen et al.: MPI/Python regional downscaling'),
  },
  DEF_DS_longwave_adjust_scheme: {
    I: pair('TopoSCALE：晴空发射率调整', 'TopoSCALE: clear-sky emissivity adjustment'),
    II: pair('Van Tricht：随海拔递减', 'Van Tricht: elevation lapse-rate adjustment'),
  },
  DEF_DA_RTM_diel: {
    0: pair('Wang–Schmugge（1980）', 'Wang–Schmugge (1980)'),
    1: pair('Dobson 等（1985）', 'Dobson et al. (1985)'),
    2: pair('Mironov（2004）', 'Mironov (2004)'),
    3: pair('Mironov（2009）', 'Mironov (2009)'),
  },
  DEF_DA_RTM_rough: {
    0: pair('默认粗糙面参数化', 'Default rough-surface parameterization'),
    1: pair('SMOS 粗糙面参数', 'SMOS rough-surface parameters'),
    2: pair('SMAP 粗糙面参数', 'SMAP rough-surface parameters'),
    3: pair('P16 粗糙面参数', 'P16 rough-surface parameters'),
  },
  DEF_WRST_FREQ: {
    none: pair('不写重启文件', 'Do not write restart files'),
    TIMESTEP: pair('每个时间步', 'Every time step'), HOURLY: pair('每小时', 'Hourly'),
    DAILY: pair('每天', 'Daily'), MONTHLY: pair('每月', 'Monthly'), YEARLY: pair('每年', 'Yearly'),
  },
  DEF_HIST_FREQ: {
    none: pair('不写历史结果', 'Do not write history output'),
    TIMESTEP: pair('每个时间步', 'Every time step'), HOURLY: pair('每小时', 'Hourly'),
    DAILY: pair('每天', 'Daily'), MONTHLY: pair('每月', 'Monthly'), YEARLY: pair('每年', 'Yearly'),
  },
  DEF_HIST_groupby: {
    DAY: pair('每天一个文件', 'One file per day'), MONTH: pair('每月一个文件', 'One file per month'),
    YEAR: pair('每年一个文件', 'One file per year'),
  },
  DEF_Forcing_Interp_Method: {
    arealweight: pair('面积权重', 'Area-weighted'), bilinear: pair('双线性插值', 'Bilinear'),
  },
  DEF_TRACER_KINETIC_SCHEME: {
    CAPPA2003: pair('Cappa 等（2003）', 'Cappa et al. (2003)'),
    MERLIVAT1978: pair('Merlivat（1978）', 'Merlivat (1978)'),
  },
  DEF_TRACER_OPEN_WATER_KINETIC: {
    EXPONENT: pair('指数形式', 'Exponent form'), MJ79: pair('Merlivat–Jouzel（1979）简写', 'Merlivat–Jouzel (1979), short form'),
    MERLIVAT_JOUZEL1979: pair('Merlivat–Jouzel（1979）', 'Merlivat–Jouzel (1979)'),
  },
  DEF_TRACER_SOIL_KINETIC: {
    EXPONENT: pair('指数形式', 'Exponent form'), RESISTANCE: pair('阻力形式', 'Resistance form'),
  },
  DEF_USE_Campbell_SOIL_MODEL: {
    '.true.': pair('Campbell（1974）', 'Campbell (1974)'),
    '.false.': pair('van Genuchten–Mualem（Ippisch 2006）', 'van Genuchten–Mualem (Ippisch 2006)'),
  },
  DEF_USE_Forcing_Downscaling: {
    '.true.': pair('启用完整方案（自动关闭简化方案）', 'Enable full mode (disables simple mode)'),
    '.false.': pair('关闭完整方案', 'Disable full mode'),
  },
  DEF_USE_Forcing_Downscaling_Simple: {
    '.true.': pair('启用简化方案（自动关闭完整方案）', 'Enable simple mode (disables full mode)'),
    '.false.': pair('关闭简化方案', 'Disable simple mode'),
  },
  DEF_USE_OZONEDATA: {
    '.true.': pair('从所选 NetCDF 文件读取', 'Read from the selected NetCDF file'),
    '.false.': pair('使用固定臭氧浓度（100 ppbv）', 'Use a fixed ozone concentration (100 ppbv)'),
  },
  GUI_STOMATAL_CONDUCTANCE_SCHEME: {
    BALL_BERRY: pair('Ball–Berry（CoLM 默认）', 'Ball–Berry (CoLM default)'),
    MEDLYN: pair('Medlyn', 'Medlyn'),
    WUE: pair('水分利用效率（WUE）', 'Water-use efficiency (WUE)'),
    INVALID: pair('配置冲突：Medlyn 与 WUE 同时开启', 'Invalid: Medlyn and WUE are both enabled'),
  },
});

const ZH_TOKENS = Object.freeze({
  use: '启用', file: '文件', dir: '目录', scheme: '方案', model: '模型', runoff: '产流',
  soil: '土壤', snow: '积雪', water: '水分', table: '水位', init: '初始场', forcing: '强迫场',
  urban: '城市', tracer: '示踪剂', dynamic: '动态', lake: '湖泊', wetland: '湿地',
  history: '历史输出', hist: '历史输出', rest: '重启', output: '输出', compress: '压缩', level: '级别',
  frequency: '频率', type: '类型', data: '数据', readin: '读取', clim: '气候态',
  optimize: '优化', baseflow: '基流', check: '检查', equilibrium: '平衡',
});

function words(path) {
  return path
    .replace(/^DEF_/i, '')
    .replace(/^USE_SITE_/i, '')
    .replace(/^SITE_/i, '')
    .replace(/%/g, '_')
    .replace(/([a-z])([A-Z])/g, '$1_$2')
    .split('_')
    .filter(Boolean);
}

/** 所有字段都保证不再把 DEF_/USE_SITE_ 当作主标签。 */
export function fieldLabel(path, lang = 'zh') {
  const exact = pick(LABELS[path], lang);
  if (exact) return exact;
  const prefix = /^USE_SITE_/i.test(path)
    ? (lang === 'en' ? 'Read from site file: ' : '从站点文件读取：')
    : '';
  const parts = words(path);
  if (lang === 'en') return prefix + parts.join(' ').replace(/\b\w/g, c => c.toUpperCase());
  return prefix + parts.map(word => ZH_TOKENS[word.toLowerCase()] ?? word).join(' · ');
}

/** 下拉框显示方案名，value 仍保持 CoLM 的原始字面量。 */
export function optionLabel(path, value, lang = 'zh') {
  const raw = String(value).replace(/^'|'$/g, '');
  const described = pick(OPTIONS[path]?.[raw], lang);
  if (described) return `${described}${lang === 'en' ? ` (${raw})` : `（${raw}）`}`;
  if (/^(\.true\.|true|\.t\.)$/i.test(raw)) return lang === 'en' ? 'Enabled' : '启用';
  if (/^(\.false\.|false|\.f\.)$/i.test(raw)) return lang === 'en' ? 'Disabled' : '关闭';
  return raw;
}

export function technicalFieldHint(path, lang = 'zh') {
  return lang === 'en' ? `CoLM key: ${path}` : `CoLM 配置键：${path}`;
}

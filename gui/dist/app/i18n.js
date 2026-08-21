//! 轻量双语层。与 EarthMesh 一样不引入前端构建器：一个词典、两个分段按钮，
//! 运行时切换。区别是本界面大量内容由 JS 动态生成，所以用 MutationObserver
//! 同步翻译新节点，而不是要求每个 render 函数再维护一套 DOM。

const ZH_EN = [
  // Complete sentences come before short labels. This keeps prose grammatical and
  // prevents a label such as “运行” from creating half-translated paragraphs.
  ['把原始数据批量转成模型要的格式，', 'Convert raw data in batches to the format required by the model,'],
  ['产出的就是下一步要扫的站点数据', 'the output is the site data scanned in the next step'],
  ['已经转好的会跳过 ——', 'Inputs already converted are skipped —'],
  ['判据与第 4 步的阶段跳过一样，是输入指纹而不是文件在不在。', 'as with stage skipping in Step 4, the decision uses an input fingerprint rather than mere file existence.'],
  ['选一份原始强迫场，探测里面有哪些变量、', 'Choose a raw forcing file and inspect its variables,'],
  ['选一份原始强迫场', 'Choose a raw forcing file'],
  ['，探测里面有哪些变量、', ', and inspect its variables,'],
  ['八个槽位怎么对应、时间轴是否均匀、观测高度有没有。探完才看得到下面几张卡片。', 'the eight-slot mapping, time-axis regularity, and observation heights. The cards below appear after inspection.'],
  ['只给一对经纬度就能建出一份能跑的', 'A runnable'],
  ['CoLM 无条件要读的 12 个必需字段，由 rawdata 栅格或标称假设补齐，', 'The 12 fields CoLM always reads are filled from rawdata rasters or nominal assumptions,'],
  ['每个都带来自哪里的说明', 'with the source recorded for every field'],
  ['经纬度是唯一必填的两项。', 'Longitude and latitude are the only required fields.'],
  ['（可选，IGBP 分类号）', '(optional, IGBP class)'],
  ['不填就让 CoLM 按自己的规则回落 ——', 'Leave it empty to use CoLM’s own fallback —'],
  ['写一个猜的值比不写更糟', 'a guessed value is worse than no value'],
  ['只有确实知道这个站点的地类时才填这一项；城市站点不受这条影响，', 'Only enter a class when it is known for this site. Urban sites are unaffected;'],
  ['CoLM 会把地类强制成 13。', 'CoLM forces their land-cover class to 13.'],
  ['写出一份新的', 'Create a new'],
  ['给了 rawdata 目录，能抽到的字段', 'When a rawdata directory is provided, available fields'],
  ['就从栅格抽；抽不到的（以及压根没给 rawdata 的全部）走标称假设 ——', 'come from rasters; unavailable fields (or all fields when rawdata is omitted) use nominal assumptions —'],
  ['文件依然能跑，只是这些数不是这个站点的实测值。', 'the file remains runnable, but those values are not site measurements.'],
  ['站点、路径、时间和建例所需的数据设置都收在这里；', 'Site, path, time, and case-creation data settings are collected here;'],
  ['过程参数留在下一步，避免同一个字段出现两次。', 'process parameters stay in the next step so each field appears only once.'],
  ['先选站点数据和算例根目录，再在同一页建算例。', 'Choose the site data and case root, then create cases on this page.'],
  ['CN-Cng 自然站或 AU-Preston 城市站', 'CN-Cng natural site or AU-Preston urban site'],
  ['找', 'Find'],
  ['同目录约定下的强迫场和观测会一并匹配。', 'Forcing and observations following the sibling-directory convention are matched automatically.'],
  ['；同目录约定下的强迫场和观测会一并匹配。', '; forcing and observations following the sibling-directory convention are matched automatically.'],
  ['扫描时按站点名称在这里匹配强迫场；留空则使用站点目录旁的 Forcing 目录。', 'Forcing is matched here by site name; leave this empty to use the Forcing directory beside the site directory.'],
  ['本次向导打开了', 'The current wizard enables'],
  ['；表外城市站点需要这些数据目录。', '; urban sites outside the built-in table need these data directories.'],
  ['列表只显示与向导一致的自然站或城市站；点行高亮，勾选用于批量。', 'The list shows only natural or urban sites matching the wizard; click a row to focus it and tick boxes for batch actions.'],
  ['选择产物目录；上面“建算例”按钮会把选中的站点写到这里。', 'Choose the output directory; the “Create cases” button above writes the selected sites here.'],
  ['路径含空格：CoLM 的 shell 建目录会拆错路径，请换一个不含空格的位置。', 'The path contains spaces, which CoLM shell commands split incorrectly. Choose a location without spaces.'],
  ['站点文件、位置、地类及 USE_SITE 开关。', 'Site file, location, land-cover class, and USE_SITE switches.'],
  ['单点内核本身是串行的；这里控制批量运行时同时启动多少个算例，每个算例使用一个 CPU 核。', 'The single-point kernel is serial. This controls how many cases run concurrently; each case uses one CPU core.'],
  ['按当前配置显示水热、生态生地化、河道水库、数据同化、示踪剂和城市过程；', 'Show hydrothermal, ecological and biogeochemical, routing and reservoir, data-assimilation, tracer, and urban processes for the current configuration;'],
  ['没有启用或没有可设置项的分栏会自动隐藏。', 'sections that are disabled or have no configurable fields are hidden automatically.'],
  ['预设不会替换向导选项、站点身份与路径', 'Presets do not replace wizard choices, site identity, or paths'],
  ['设置输出，然后依次运行', 'Configure output, then run'],
  ['输入没变的阶段会按输入指纹跳过', 'Stages with unchanged inputs are skipped using input fingerprints'],
  ['时间与预热在第 2 步“基本设定”。', 'Time and spin-up are configured in Step 2, “Basic setup”.'],
  ['勾选决定「全部运行」作用于谁；一个没勾就是全部。', 'Checked cases define the “Run all” target; with none checked, all current cases are used.'],
  ['没建过的算例会在运行前自动建。', 'Cases not yet created are created automatically before running.'],
  ['设置输出频率、目录与重启。先运行时不必翻完几百个输出变量。', 'Configure output frequency, directories, and restart settings. You do not need to review hundreds of output variables before the first run.'],
  ['选择需要写入 history 的变量；默认收起，避免把运行按钮推到页面底部。', 'Choose variables to write to history. This section is collapsed by default so the run controls stay near the top.'],
  ['画模型输出，或与观测配对算指标。', 'Plot model output or pair it with observations to calculate metrics.'],
  ['「丢弃前 N 条」丢的是输出记录', '“Discard first N” removes output records'],
  ['与第 2 步基本设定里的模型预热（spin-up）不是一回事。', 'and is different from model spin-up in Step 2, Basic setup.'],
  ['图表缩放：在图上拖选一段横向范围可放大；双击图表恢复全范围。', 'Chart zoom: drag across a horizontal range to zoom in; double-click the chart to restore the full range.'],
  ['观测文件按命名约定自动找；命名不合约定时可以自己指。', 'Observation files are found by naming convention; choose one manually when its name does not follow the convention.'],
  ['丢弃前', 'Discard first'],
  ['条 ⓘ', 'records ⓘ'],
  ['用能量闭合订正后的观测 ⓘ', 'Use energy-closure-corrected observations ⓘ'],
  ['对比图同样支持拖选横向范围放大，双击恢复全范围。', 'Comparison charts also support drag-to-zoom and double-click to restore the full range.'],
  ['现在只有站点能跑。区域与全球的步骤链还没有实现。', 'Only site simulations are currently available. Regional and global workflows are not yet implemented.'],
  ['丢的是输出记录，单位是条。与第 2 步基本设定里的预热（spin-up）不是一回事 —— 那个的单位是轮数，且预热期本来就不写 history。', 'This discards output records. It is separate from spin-up in Step 2, whose unit is cycles and which does not write history.'],
  ['涡度相关的湍流通量普遍关不上能量收支，观测文件自带订正版 Qle_cor / Qh_cor。实测 AT-Neu：订正把观测 Qle 抬高 25.5 W/m²，而模型对未订正观测的偏差是 +19.8 —— 偏差的大小基本就是那个缺口。默认关，因为 design.md 的目标值是拿未订正版算的。', 'Eddy-covariance turbulent fluxes often do not close the energy balance, and observation files include corrected Qle_cor / Qh_cor values. At AT-Neu the correction raises observed Qle by 25.5 W/m², close to the model bias of +19.8 against uncorrected observations. This is off by default because design.md targets use uncorrected data.'],
  ['① 位置', '① Location'],
  ['② 产物', '② Output'],
  ['地类', 'Land-cover class'],
  ['留空 = 不写', 'Leave empty = do not write'],
  ['例如', 'e.g.'],
  ['自然站', 'natural site'],
  ['城市站', 'urban site'],
  ['还没选', 'Nothing selected'],
  ['运行全部', 'Run all'],
  ['评估全部已跑算例', 'Evaluate all completed cases'],
  ['示踪剂', 'Tracer'],
  ['强迫场', 'Forcing'],
  ['留空 = 使用站点目录旁的 ../Forcing/', 'Leave empty = use ../Forcing/ beside the site directory'],
  ['中文', '中文'],
  ['这次要跑什么？', 'What would you like to run?'],
  ['空间结构先定；现在只有站点步骤链能跑。', 'Choose the spatial structure first; only the site workflow is available for now.'],
  ['次网格怎么分？', 'How should the subgrid be represented?'],
  ['次网格方案决定 BGC 是否可用，也决定站点数据要求。', 'The subgrid scheme controls BGC availability and site-data requirements.'],
  ['土壤水力用哪套？', 'Which soil-hydraulics scheme?'],
  ['土壤方案决定 TRACER 是否可用，也改变站点数据要求。', 'The soil scheme controls TRACER availability and site-data requirements.'],
  ['还要打开哪些过程？', 'Which additional processes should be enabled?'],
  ['可多选；被上游约束挡住的项会说明回哪一页修改。', 'Select any number; blocked items link back to the prerequisite page.'],
  ['要打开调试吗？', 'Enable debugging?'],
  ['可全部不选；这些开关只增加检查与日志，不改变页间约束。', 'All may stay off; these switches only add checks and logs.'],
  ['单点站点模拟', 'Single-point site simulation'],
  ['有限范围网格', 'Limited-area grid'],
  ['全球网格', 'Global grid'],
  ['区域步骤链尚未实现', 'Regional workflow is not implemented yet'],
  ['全球步骤链尚未实现', 'Global workflow is not implemented yet'],
  ['24 类地表覆盖（旧方案）', '24-class land cover (legacy)'],
  ['17 类地表覆盖', '17-class land cover'],
  ['一个 patch 拆成多个功能型', 'Split each patch into plant functional types'],
  ['一个 patch 一个地类', 'One land-cover class per patch'],
  ['植物功能型', 'Plant functional types'],
  ['植物群落', 'Plant communities'],
  ['同 PFT，次网格组织方式不同', 'Like PFT, with a different subgrid organization'],
  ['默认土壤水力方案', 'Default soil-hydraulics scheme'],
  ['Campbell 土壤水力', 'Campbell soil hydraulics'],
  ['支持 TRACER', 'Supports TRACER'],
  ['不需要 alpha_vgm / n_vgm / theta_r', 'Does not require alpha_vgm / n_vgm / theta_r'],
  ['城市冠层与人为热', 'Urban canopy and anthropogenic heat'],
  ['土地利用变化', 'Land-use change'],
  ['碳氮循环', 'Carbon and nitrogen cycling'],
  ['作物模型', 'Crop model'],
  ['同位素 / 溶质示踪', 'Isotope / solute tracing'],
  ['逐变量范围检查', 'Per-variable range checks'],
  ['详细诊断输出', 'Detailed diagnostic output'],
  ['地表数据诊断', 'Surface-data diagnostics'],
  ['CROP 仍决定数组尺寸，需要 CROP-enabled 内核；同时需要 BGC', 'CROP still determines array sizes, so it needs a CROP-enabled kernel as well as BGC'],
  ['ⓘ 站点文件最好提供 pfttyp 与 pctpfts；缺少时会回落到 rawdata/plant_15s', 'ⓘ Site files should provide pfttyp and pctpfts; missing values fall back to rawdata/plant_15s'],
  ['ⓘ 站点数据使用 IGBP_classification', 'ⓘ Site data uses IGBP_classification'],
  ['ⓘ 必须选择一种次网格方案', 'ⓘ Choose a subgrid scheme'],
  ['ⓘ Campbell 不需要 alpha_vgm / n_vgm / theta_r，但不能使用 TRACER', 'ⓘ Campbell does not need alpha_vgm / n_vgm / theta_r, but cannot use TRACER'],
  ['ⓘ van Genuchten 需要 alpha_vgm / n_vgm / theta_r，并支持 TRACER', 'ⓘ van Genuchten needs alpha_vgm / n_vgm / theta_r and supports TRACER'],
  ['ⓘ 灰项仍然列出；带“← 第 N 页”的卡片可直接返回修改', 'ⓘ Disabled cards remain visible; cards marked “← Page N” return directly to the prerequisite'],
  ['ⓘ 打开调试会让日志明显增多，常规运行可全部关闭', 'ⓘ Debugging substantially increases log output; normal runs can leave every debug option off'],
  ['正在检查可用内核', 'Checking available kernels'],
  ['单点站点暂不支持 LULCC', 'Single-point sites do not yet support LULCC'],
  ['第 1 页选择了站点', 'Page 1 selected a site simulation'],
  ['需要 PFT 或 PC 次网格', 'Requires a PFT or PC subgrid'],
  ['第 2 页选了', 'Page 2 selected'],
  ['需要 van Genuchten 土壤水力', 'Requires van Genuchten soil hydraulics'],
  ['第 3 页选了 Campbell', 'Page 3 selected Campbell'],
  ['当前安装缺少', 'The current installation lacks'],
  ['内核', 'kernel'],
  ['工作流', 'Workflow'],
  ['界面语言', 'Interface language'],
  ['参数模式', 'Parameter mode'],
  ['已选站点', 'Selected sites'],
  ['还没有算例', 'No case yet'],
  ['返回首页', 'Home'],
  ['重新选择空间、次网格与物理配置', 'Choose spatial, subgrid, and physics settings again'],
  ['强迫场与站点属性', 'Forcing and site properties'],
  ['原始数据转成模型格式', 'Convert raw data to model inputs'],
  ['准备模型输入', 'Prepare model inputs'],
  ['基本设定', 'Basic setup'],
  ['建例与基础输入', 'Create cases and configure inputs'],
  ['文件与目录', 'Files & directories'],
  ['选站点并建算例', 'Select sites and create cases'],
  ['站点信息', 'Site information'],
  ['位置、地类与站点文件', 'Location, land cover, and site file'],
  ['时间与预热', 'Time & spin-up'],
  ['模拟范围与 spin-up', 'Simulation window and spin-up'],
  ['时间与预热（spin-up）', 'Time & spin-up'],
  ['时间范围默认就是强迫场覆盖的', 'The default time window is the'],
  ['全部', 'full'],
  ['范围，由文件说了算，不用填。', 'forcing coverage read from the file; no manual entry is needed.'],
  ['模拟', 'Simulation'],
  ['spin-up：重复开头', 'Spin-up: repeat the beginning'],
  ['任一格填 0 就是不预热', 'Set either field to 0 to disable spin-up'],
  ['已关闭预热', 'Spin-up disabled'],
  ['各算例从自己的预热结束处开始', 'Each case starts output after its own spin-up period'],
  ['个算例各跑各自强迫场的全部范围（第一个是', 'cases each use their full forcing window (first:'],
  ['个算例的预热设置不一致', 'cases have different spin-up settings'],
  ['，上面显示的是第一个的。', '; the first case is shown above.'],
  ['改一下就会把全部统一成同一套。', 'Editing either value applies the same setting to every case.'],
  ['没有预热：从初始场直接开跑。土壤温湿与碳库是慢变量，', 'No spin-up: the run starts directly from the initial state. Soil temperature, moisture, and carbon pools change slowly,'],
  ['头一段结果不代表这个站点的气候态。', 'so the initial results do not represent the site’s climatological state.'],
  ['预热期', 'The spin-up period'],
  ['不写输出', 'writes no output'],
  ['（MOD_Hist.F90:235 在 itstamp &lt;= ptstamp 时直接 RETURN），', '(MOD_Hist.F90:235 returns directly when itstamp &lt;= ptstamp),'],
  ['所以', 'Therefore'],
  ['这段不在结果里 —— 预热是从窗口头上扣的，', 'is absent from the results—the spin-up interval is taken from the beginning of the window,'],
  ['不是加在前面的。总模拟量约为', 'not added before it. Total simulated time is approximately'],
  ['年预热 + 正式那段。', 'years of spin-up plus the production period.'],
  ['年', 'years'],
  ['遍', 'cycles'],
  ['网格与并行', 'Grid & parallelism'],
  ['网格和进程划分', 'Grid and process layout'],
  ['地表数据', 'Surface data'],
  ['地表输入设置', 'Surface input settings'],
  ['初始场', 'Initial state'],
  ['初始状态设置', 'Initial-state settings'],
  ['强迫场读取设置', 'Forcing input settings'],
  ['气温', 'Air temperature'],
  ['比湿', 'Specific humidity'],
  ['气压', 'Surface pressure'],
  ['降水', 'Precipitation'],
  ['东风', 'Eastward wind'],
  ['北风 / 标量风', 'Northward / scalar wind'],
  ['短波辐射', 'Downward shortwave'],
  ['长波辐射', 'Downward longwave'],
  ['先选一份强迫场文件', 'Choose a forcing file first'],
  ['槽位映射', 'Slot mapping'],
  ['CoLM 认死了这八个槽位（', 'CoLM uses these fixed eight slots ('],
  ['1 气温 2 比湿 3 气压 4 降水 5 东风 6 北风/标量风 7 短波辐射 8 长波辐射）。', '1 air temperature, 2 specific humidity, 3 pressure, 4 precipitation, 5 eastward wind, 6 northward/scalar wind, 7 shortwave, 8 longwave).'],
  ['自动猜的对不对、单位要不要换，都要你确认一遍 ——', 'Confirm both the inferred variable names and any unit conversions—'],
  ['变量名猜错的后果是「跑得完、结果全错」', 'a wrong variable name can produce a completed but entirely wrong run'],
  ['，模型照样跑完，曲线照样是曲线，', '; the model can finish and still draw plausible curves,'],
  ['界面上什么都看不出来。', 'with no obvious visual warning.'],
  ['槽位', 'Slot'],
  ['含义', 'Meaning'],
  ['源单位', 'Source unit'],
  ['目标单位', 'Target unit'],
  ['这些映射我看过了', 'I have reviewed these mappings'],
  ['映射已确认，下面「转换」可以按了', 'Mapping confirmed; Convert is now available'],
  ['先在上面「槽位映射」卡片点一次「这些映射我看过了」', 'Click “I have reviewed these mappings” in the Slot mapping card above first'],
  ['已确认 —— 再改任何一行都会打回未确认', 'Confirmed—editing any row will require confirmation again'],
  ['还没确认，下面「转换」按钮不会亮', 'Not confirmed; Convert remains disabled'],
  ['必需槽位还没选变量：', 'Required slots without a variable: '],
  ['第', 'Slot '],
  ['槽（', ' ('],
  ['（不用）', '(unused)'],
  ['这一槽可以空着 —— 标量风的数据集没有它，模型照样能跑。', 'This slot may be empty; scalar-wind datasets do not provide it and the model can still run.'],
  ['再加一个同单位的变量合并进这一槽（降水常拆成雨 + 雪两个变量）：', 'Add another variable with the same units to this slot (precipitation is often split into rain + snow):'],
  ['（不加）', '(none)'],
  ['必填', 'Required'],
  ['时间轴与观测高度', 'Time axis and observation heights'],
  ['步长与观测高度会写进产物；模拟用哪一段时间范围仍以强迫场覆盖范围为准，在“基本设定 / 时间与预热”里看。', 'The time step and observation heights are written to the output. The simulation window still follows forcing coverage and is shown under “Basic setup / Time & spin-up”.'],
  ['步长与观测高度会写进产物；模拟用哪一段时间范围仍以强迫场 覆盖范围为准，在“基本设定 / 时间与预热”里看。', 'The time step and observation heights are written to the output. The simulation window still follows forcing coverage and is shown under “Basic setup / Time & spin-up”.'],
  ['步长', 'Time step'],
  ['秒', 'seconds'],
  ['步数', 'Steps'],
  ['是否等间隔', 'Uniform interval'],
  ['不是 —— 重采样不在这一阶段，请先自己处理', 'No—resampling is not performed here; resample the source first'],
  ['是', 'Yes'],
  ['观测高度 V（风速，米）', 'Observation height V (wind, m)'],
  ['观测高度 T（气温，米）', 'Observation height T (air temperature, m)'],
  ['观测高度 Q（湿度，米）', 'Observation height Q (humidity, m)'],
  ['CoLM 要观测高度填', 'CoLM requires observation heights in'],
  ['这份文件里没有，', 'They are missing from this file;'],
  ['不填的话模型会拿到', 'without them the model receives'],
  ['然后直接崩，而报出来的错看不出是这里的问题。', 'and crashes with an error that does not identify this field.'],
  ['选了变量但没填源单位：', 'Variables selected without source units: '],
  ['转换', 'Convert'],
  ['按上面确认过的映射写出一份 CoLM 认的标准文件。', 'Write a standard CoLM forcing file using the confirmed mapping above.'],
  ['产物目录不能与源文件所在目录相同', 'The output directory must differ from the source directory'],
  ['原始数据要原样留着，', 'the raw data must remain unchanged,'],
  ['选了同一个目录后端会直接拒绝。', 'and the backend rejects the same directory.'],
  ['产物文件名沿用源文件名', 'The output keeps the source file name'],
  ['，只是换了目录。', ', changing only the directory.'],
  ['就绪，可以转换。', 'Ready to convert.'],
  ['已转换：', 'Converted: '],
  ['下一步：回到「站点」那一步，把 Sitedata 目录指到产物所在的位置（或它的上级目录）', 'Next: return to “Site” and point the Sitedata directory to the output location (or its parent)'],
  ['重新扫描 —— 这份产物已经是标准约定的强迫场，扫描认得出来。', 'and scan again. This output follows the standard forcing convention and will be detected.'],
  ['先填产物放哪个目录', 'Choose an output directory first'],
  ['转换完成：', 'Conversion complete: '],
  ['过程参数', 'Process parameters'],
  ['按过程逐项配置', 'Configure each process'],
  ['水热过程', 'Hydrothermal processes'],
  ['土壤、积雪与水分', 'Soil, snow, and water'],
  ['生态与生地化', 'Ecology & biogeochemistry'],
  ['植被、碳氮过程', 'Vegetation, carbon, and nitrogen'],
  ['河道与水库', 'Rivers & reservoirs'],
  ['汇流与水库过程', 'Routing and reservoir processes'],
  ['数据同化', 'Data assimilation'],
  ['同化过程设置', 'Assimilation settings'],
  ['示踪过程设置', 'Tracer settings'],
  ['城市过程', 'Urban processes'],
  ['输出与运行', 'Output and execution'],
  ['运行算例', 'Run cases'],
  ['输出、阶段与日志', 'Output, stages, and logs'],
  ['曲线与指标', 'Curves and metrics'],
  ['查看结果', 'View results'],
  ['前处理', 'Preprocessing'],
  ['站点', 'Site'],
  ['区域', 'Regional'],
  ['全球', 'Global'],
  ['运行', 'Run'],
  ['结果', 'Results'],
  ['强迫场文件', 'Forcing file'],
  ['探一探', 'Inspect'],
  ['站点属性', 'Site properties'],
  ['经度（度）', 'Longitude (degrees)'],
  ['纬度（度）', 'Latitude (degrees)'],
  ['文件名', 'File name'],
  ['rawdata 目录', 'rawdata directory'],
  ['runtime 目录', 'runtime directory'],
  ['用自带的示例站点', 'Use a bundled example site'],
  ['站点目录', 'Site directory'],
  ['强迫场目录', 'Forcing directory'],
  ['选择站点', 'Select sites'],
  ['算例放哪', 'Case location'],
  ['算例根目录', 'Case root directory'],
  ['CPU 并行', 'CPU parallelism'],
  ['并行算例数（CPU 核）', 'Concurrent cases (CPU cores)'],
  ['网格与并行参数', 'Grid and parallel parameters'],
  ['参数预设', 'Parameter presets'],
  ['（还没有预设）', '(no presets yet)'],
  ['预设名', 'Preset name'],
  ['已存', 'Saved'],
  ['个身份字段未收进去', ' identity fields excluded'],
  ['已把', 'Applied'],
  ['套到', 'to'],
  ['个算例上', ' cases'],
  ['已套用', 'Applied'],
  ['已删除', 'Deleted'],
  ['跑哪些', 'Cases to run'],
  ['开始运行', 'Start run'],
  ['输出变量（按需展开）', 'Output variables (expand as needed)'],
  ['输出变量', 'Output variables'],
  ['输出', 'Output'],
  ['曲线', 'Curves'],
  ['与观测比对', 'Compare with observations'],
  ['日志', 'Logs'],
  ['进度', 'Progress'],
  ['选择日志站点', 'Select log site'],
  ['连接后端…', 'Connecting to backend…'],
  ['没有 IPC 后端 —— 这个页面不在 Tauri 里运行', 'No IPC backend—this page is not running inside Tauri'],
  ['后端出错：', 'Backend error: '],
  ['先在文件与目录建一个算例', 'Create a case under Files & directories first'],
  ['绘图与评估', 'Plotting & evaluation'],
  ['当前配置没有这一步：', 'This configuration does not include this step: '],
  ['已选 ', 'Selected '],
  ['（没有强迫场，跑不了）', ' (no forcing; cannot run)'],
  ['建算例：', 'Create case: '],
  ['专家选项还在规划中。你后续提供的专家内容会放在这里；当前不会额外改写任何模型参数。',
    'Expert options are being planned. The expert content you provide later will appear here; enabling this mode currently changes no model parameters.'],
  ['常规', 'Normal'],
  ['专家', 'Expert'],
  ['选择…', 'Choose…'],
  ['扫描', 'Scan'],
  ['全不选', 'Select none'],
  ['全选', 'Select all'],
  ['刷新本次算例', 'Refresh current cases'],
  ['应用', 'Apply'],
  ['套用', 'Apply preset'],
  ['存为预设…', 'Save as preset…'],
  ['删除', 'Delete'],
  ['生成', 'Generate'],
  ['地类要是一个整数，留空就不写', 'Land-cover class must be an integer; leave it empty to omit it'],
  ['经度必填', 'Longitude is required'],
  ['纬度必填', 'Latitude is required'],
  ['产物文件名不能为空', 'Output file name cannot be empty'],
  ['就绪，可以生成。', 'Ready to generate.'],
  ['个字段走标称假设', ' fields use nominal assumptions'],
  ['（无）', '(none)'],
  ['CoLM 无条件要读的', 'CoLM always reads'],
  ['个必需字段，每个都归到了下面某一类。', ' required fields, each assigned to one category below.'],
  ['一共', 'In total,'],
  ['个字段有来源说明（预期', ' fields have source notes (expected '],
  ['个——如果不是，说明这份 GUI 与当前的 colm-cli 版本没对齐，先别拿它建算例）。', '; if not, this GUI and colm-cli are out of sync, so do not create cases with it yet).'],
  ['未填 —— 交给 CoLM 按自己的规则回落', 'Not set—use CoLM’s own fallback'],
  ['质地', 'texture'],
  ['（第', ' (class '],
  ['类），BVIC', '), BVIC'],
  ['来自站点自身', 'From the site file'],
  ['来自 rawdata 栅格', 'From rawdata rasters'],
  ['标称假设', 'Nominal assumptions'],
  ['这些是标称假设，不是这个站点实测的 —— 拿这份文件跑出来的结果，', 'These are nominal assumptions, not measurements at this site. Results produced from this file'],
  ['这些字段部分的可信度取决于这一点，不能当成量出来的数。', 'depend on that limitation and must not be treated as measured values for these fields.'],
  ['取消', 'Cancel'],
  ['复制', 'Copy'],
  ['清空', 'Clear'],
  ['画图', 'Plot'],
  ['评估', 'Evaluate'],
  ['强制全部重跑', 'Force a full rerun'],
  ['空闲', 'Idle'],
  ['待运行', 'Queued'],
  ['运行中', 'Running'],
  ['已完成', 'Completed'],
  ['完成', 'Completed'],
  ['全部完成', 'All completed'],
  ['没有找到内核', 'No kernel found'],
  ['启动失败', 'Launch failed'],
  ['批次启动失败', 'Batch launch failed'],
  ['产物齐全且输入未变', 'Outputs complete and inputs unchanged'],
  ['等待', 'waiting'],
  ['日志是空的', 'The log is empty'],
  ['剪贴板不可用', 'Clipboard unavailable'],
  ['已全选，请按 ⌘C', 'all text is selected; press ⌘C'],
  ['完整日志在', 'full logs are in'],
  ['用自带的示例站点（城市站 AU-Preston）', 'Use bundled example site (urban AU-Preston)'],
  ['用自带的示例站点（CN-Cng）', 'Use bundled example site (CN-Cng)'],
  ['成功', 'Completed'],
  ['失败', 'Failed'],
  ['跳过', 'Skipped'],
  ['等待 CPU', 'Waiting for CPU'],
  ['当前配置没有这一类可设置项。', 'This configuration has no settings in this section.'],
  ['当前配置没有可配置的输出参数。', 'This configuration has no configurable output parameters.'],
  ['当前安装缺少与向导配置匹配的运行产物', 'The installation has no runtime matching the wizard configuration'],
  ['只看已勾选', 'Selected only'],
  ['没有匹配的变量', 'No matching variables'],
  ['个在当前配置下写不出来', ' unavailable in the current configuration'],
  ['个未知', ' unknown'],
  ['写不出来', 'Unavailable'],
  ['未知', 'Unknown'],
  ['已写入', 'Written to'],
  ['已保存', 'Saved'],
  ['强迫场 namelist 的路径。CoLM 会直接打开并读取这个文件（MOD_Namelist.F90:1392），不能删除。', 'Path to the forcing namelist. CoLM opens and reads this file directly (MOD_Namelist.F90:1392), so it cannot be removed.'],
  ['预热轮数：起始日**之前**那段反复跑几遍，让土壤温湿等状态趋于平衡。', 'Spin-up cycles: repeat the period before the start date so soil temperature, moisture, and other states approach equilibrium.'],
  ['预热期不写 history（MOD_Hist.F90:235 在 itstamp <= ptstamp 时直接 RETURN），', 'Spin-up writes no history (MOD_Hist.F90:235 returns when itstamp <= ptstamp),'],
  ['所以它不会污染输出，也不会被算进指标。', 'so it neither contaminates output nor enters the metrics.'],
  ['与结果页的「丢弃前 N 条记录」不是一回事：那个丢的是输出记录，单位是条。', 'This differs from “Discard first N records” on Results, which removes output records counted as records.'],
  ['预热截止时刻。起始时刻早于它，中间那段就是预热期。四项（年月日秒）一起决定。', 'Spin-up cutoff. The interval between the earlier start and this time is spin-up; year, month, day, and second define it together.'],
  ['预热截止时刻的月，见 spinup_repeat 的说明。', 'Month of the spin-up cutoff; see spinup_repeat.'],
  ['预热截止时刻的日，见 spinup_repeat 的说明。', 'Day of the spin-up cutoff; see spinup_repeat.'],
  ['预热截止时刻的当天秒数，见 spinup_repeat 的说明。', 'Seconds into the spin-up cutoff day; see spinup_repeat.'],
  ['下面的改动会写进', 'The changes below will be written to'],
  ['只改', 'Edit only'],
  ['输出与重启', 'Output & restart'],
  ['CoLM 不认识这个字段', 'CoLM does not recognize this field'],
  ['本内核未编入（需要', 'Not compiled into this kernel (requires '],
  ['），设了也没用', '); setting it has no effect'],
  ['这一批算例在这个字段上取值不同，显示的是第一个的值。改它会把全部改成同一个值。', 'Cases in this batch differ for this field. The first value is shown; editing it applies one value to all cases.'],
  ['这份配置没设它，显示的是默认值', 'This configuration does not set it; the default is shown'],
  ['先在“文件与目录”里选择站点并建算例', 'Select sites and create cases under “Files & directories” first'],
  ['先建算例', 'Create a case first'],
  ['先选一个算例', 'Select a case first'],
  ['本次还没有要运行的算例；先在前面选站点并建算例。', 'There are no cases to run in this session; select sites and create cases first.'],
  ['本次还没有创建算例；root 里的旧算例不会显示。', 'No cases have been created in this session; older cases under the root are hidden.'],
  ['已跑过', 'Previously run'],
  ['未跑', 'Not run'],
  ['已建算例', 'Case created'],
  ['先点一个站点，或勾选几个', 'Click one site or check several'],
  ['先在“基本设定 / 文件与目录”指定算例放哪', 'Choose the case location under “Basic setup / Files & directories” first'],
  ['没有强迫场文件，建不了算例', 'has no forcing file, so its case cannot be created'],
  ['正在为', 'Creating a case for'],
  ['建算例…', '…'],
  ['已为', 'Created a case for'],
  ['建好算例', ''],
  ['要先填 Sitedata 目录', 'Enter a Sitedata directory first'],
  ['改用它并重新扫描', 'Use it and scan again'],
  ['无强迫场', 'No forcing'],
  ['无观测', 'No observations'],
  ['读不了', 'Unreadable'],
  ['个无观测', ' without observations'],
  ['个读不了', ' unreadable'],
  ['个就绪；建不了：', ' ready; could not create: '],
  ['示例数据已经在了', 'Example data is already available'],
  ['示例数据已放好', 'Example data is ready'],
  ['这份配置有', 'This configuration has'],
  ['个 CoLM 已经不认识的字段，会让运行在读取时就停：', ' fields no longer recognized by CoLM, which will stop the run while reading:'],
  ['城市', 'Urban'],
  ['自然', 'natural'],
  ['或', 'or'],
  ['没有观测文件', 'No observation file'],
  ['净辐射 Rnet', 'Net radiation Rnet'],
  ['感热 Qh', 'Sensible heat Qh'],
  ['潜热 Qle', 'Latent heat Qle'],
  ['地表热通量 Qg', 'Ground heat flux Qg'],
  ['反射短波 SWup', 'Reflected shortwave SWup'],
  ['参考高度气温', 'Reference-height air temperature'],
  ['总产流', 'Total runoff'],
  ['地下水位', 'Water-table depth'],
  ['时间', 'Time'],
  ['要先给观测文件', 'Choose an observation file first'],
  ['没有可配对的变量', 'No variables can be paired'],
  ['变量', 'Variable'],
  ['已自动绘制第一项；点其他行可切换变量', 'The first variable was plotted automatically; click another row to switch variables'],
  ['模型 vs 观测', 'Model vs observation'],
  ['模型', 'Model'],
  ['观测（横）对模型（纵）', 'Observation (x) vs model (y)'],
  ['观测', 'Observation'],
  ['个算例有结果；', ' cases produced results; '],
  ['个没有：', ' missing: '],
  ['个算例有结果', ' cases produced results'],
  ['点', 'points'],
  ['按需展开', 'expand as needed'],
  ['默认收起', 'collapsed by default'],
  ['个字符', ' characters'],
  ['个算例', ' cases'],
  ['个站点', ' sites'],
  ['个已跑算例', ' completed cases'],
  ['个成功', ' succeeded'],
  ['个失败', ' failed'],
  ['模型步', 'model steps'],
  ['站点结束', 'sites finished'],
  ['预热', 'Spin-up'],
  ['轮', 'cycle'],
  ['页', 'page'],
  ['步', 'step'],
  ['下一步：', 'Next: '],
  ['下一步', 'Next'],
  ['← 上一步', '← Back'],
  ['当前算例', 'Current case'],
  ['深色 / 浅色', 'Dark / light'],
  ['（可选）', '(optional)'],
  ['是（.true.）', 'Yes (.true.)'],
  ['否（.false.）', 'No (.false.)'],
  ['不在已知取值里', 'not in known values'],
  ['派生值，改不了', 'derived, read-only'],
  ['默认值', 'default value'],
  ['默认', 'Default'],
  ['默认 ', 'Default '],
].sort((a, b) => b[0].length - a[0].length);

export function translateZh(text, target = 'en') {
  if (target !== 'en' || !text) return text;
  const value = String(text);
  const leading = value.match(/^\s*/)?.[0] ?? '';
  const trailing = value.match(/\s*$/)?.[0] ?? '';
  let out = value.trim().replace(/\s+/g, ' ');
  if (!out) return value;
  const source = out;
  out = out
    .replace(/^选中的\s*(\d+)\s*个$/, '$1 selected')
    .replace(/^本次\s*(\d+)\s*个$/, '$1 current')
    .replace(/^运行选中的\s*(\d+)\s*个$/, 'Run $1 selected cases')
    .replace(/^运行本次\s*(\d+)\s*个$/, 'Run $1 current cases')
    .replace(/^评估选中的\s*(\d+)\s*个已跑算例$/, 'Evaluate $1 selected completed cases')
    .replace(/^评估本次\s*(\d+)\s*个已跑算例$/, 'Evaluate $1 current completed cases')
    .replace(/^检测到\s*(\d+)\s*个逻辑 CPU；单个站点仍使用 1 核。$/, '$1 logical CPUs detected; each site still uses one core.')
    .replace(/^开始运行\s*(\d+)\s*个算例$/, 'Starting $1 cases')
    .replace(/^运行中（(\d+)\/(\d+)）$/, 'Running ($1/$2)')
    .replace(/^批量总体：(\d+)\/(\d+)\s*个站点结束(?:\s*·\s*模型步\s*(\d+)\/(\d+))?$/, (_, a, b, c, d) => `Batch total: ${a}/${b} sites finished${c ? ` · model steps ${c}/${d}` : ''}`)
    .replace(/^批次结束：(\d+)\/(\d+)\s*个成功，(\d+)\s*个失败$/, 'Batch finished: $1/$2 succeeded, $3 failed')
    .replace(/^批次结束：(\d+)\/(\d+)\s*个算例全部成功$/, 'Batch finished: all $1/$2 cases succeeded')
    .replace(/^(\d+)\s*个站点失败$/, '$1 sites failed')
    .replace(/^已复制\s*(\d+)\s*个字符$/, 'Copied $1 characters')
    .replace(/^第\s*(\d+)\/(\d+)\s*页\s*·\s*/, 'Page $1/$2 · ')
    .replace(/^第\s*(\d+)\s*页选了\s*/, 'Page $1 selected ')
    .replace(/^当前安装缺少\s*(.+)\s*内核$/, 'The current installation lacks the $1 kernel')
    .replace(/^已探测\s*(.+)：(\d+)\s*个变量，(\d+)\s*步$/, 'Inspected $1: $2 variables, $3 steps')
    .replace(/^第\s*(\d+)\s*槽$/, 'Slot $1')
    .replace(/^第\s*(\d+)\s*槽（(.+)）$/, 'Slot $1 ($2)')
    .replace(/^第\s*(\d+)\s*步$/, 'Step $1')
    .replace(/^已勾选\s*(\d+)\s*个$/, '$1 selected')
    .replace(/^已勾\s*(\d+)\s*个$/, '$1 checked')
    .replace(/^搜索\s*(\d+)\s*个输出变量$/, 'Search $1 output variables')
    .replace(/^准备算例\s*(\d+)\/(\d+)：/, 'Preparing case $1/$2: ')
    .replace(/^已选\s*(\d+)\s*个?$/, '$1 selected')
    .replace(/^建算例：选中的\s*(\d+)\s*个站点$/, 'Create cases for $1 selected sites')
    .replace(/^(\d+)\s*个(城市|自然)站点$/, (_, n, kind) => `${n} ${kind === '城市' ? 'urban' : 'natural'} sites`)
    .replace(/^目录里没有(城市|自然)站点。$/, (_, kind) => `No ${kind === '城市' ? 'urban' : 'natural'} sites in this directory.`)
    .replace(/^等\s*(\d+)\s*个$/, 'and $1 more')
    .replace(/^(.+)\s+等\s*(\d+)\s*个$/, '$1 and $2 more')
    .replace(/^已生成\s*(.+)：(\d+)\s*个字段走标称假设$/, 'Generated $1: $2 fields use nominal assumptions')
    .replace(/^一共\s*(\d+)\s*个字段有来源说明（预期\s*(\d+)\s*个——如果不是，\s*说明这份 GUI 与当前的 colm-cli 版本没对齐，先别拿它建算例）。$/, 'A total of $1 fields have source notes (expected $2; otherwise this GUI and colm-cli are out of sync, so do not create cases yet).')
    .replace(/^产物\s*(.+)\s*质地\s*(.+)（第\s*(\d+)\s*类），BVIC\s*(.+)\s*地类\s*(.+)$/, 'Output $1 · texture $2 (class $3) · BVIC $4 · land-cover class $5')
    .replace(/^预热：重复开头\s*(\d+)\s*年\s*×(\d+)\s*遍$/, 'Spin-up: repeat the first $1 years ×$2')
    .replace(/^这\s*(\d+)\s*个算例的预热设置不一致\s*，上面显示的是第一个的。$/, '$1 cases have different spin-up settings; the first is shown above.')
    .replace(/^这\s*(\d+)\s*个算例的预热设置不一致$/, '$1 cases have different spin-up settings')
    .replace(/^所以\s*(.+)\s*到\s*(.+)\s*这段不在结果里 —— 预热是从窗口头上扣的，$/, 'Therefore $1 to $2 is absent from the results—the spin-up interval is taken from the beginning of the window,')
    .replace(/^第\s*(\d+)\/(\d+)\s*步\s*·\s*/, 'Step $1/$2 · ')
    .replace(/^预热\s*(\d+)\/(\d+)\s*轮\s*·\s*/, 'Spin-up $1/$2 · ')
    .replace(/^评估\s*(\d+)\/(\d+)：/, 'Evaluating $1/$2: ')
    .replace(/^批量评估完成：(\d+)\/(\d+)\s*个算例有结果$/, 'Batch evaluation complete: $1/$2 cases produced results')
    .replace(/^批量评估完成：(\d+)\s*个算例$/, 'Batch evaluation complete: $1 cases');
  for (const [zh, en] of ZH_EN) out = out.split(zh).join(en);
  out = out
    .replace(/第\s*(\d+)\/(\d+)\s*page/g, 'Page $1/$2')
    .replace(/第\s*(\d+)\s*step/g, 'Step $1')
    .replace(/选中的\s*(\d+)\s*cases/g, '$1 selected cases')
    .replace(/本次\s*(\d+)\s*cases/g, '$1 current cases')
    .replace(/批量总体：/g, 'Batch total: ')
    .replace(/等\s*(\d+)\s*sites/g, 'and $1 sites')
    .replace(/第\s*(\d+)\/(\d+)\s*step/g, 'Step $1/$2');
  // Never expose a half-translated sentence and never rewrite a user-supplied
  // Chinese site/case name by accident. Unknown text stays intact until it has
  // an explicit translation.
  if (/[㐀-鿿]/.test(out)) {
    out = source;
  } else {
    out = out
      .replace(/。/g, '.')
      .replace(/；/g, '; ')
      .replace(/，/g, ', ')
      .replace(/：/g, ': ')
      .replace(/（/g, '(')
      .replace(/）/g, ')')
      .replace(/——/g, '—')
      .replace(/\s{2,}/g, ' ');
  }
  return leading + out + trailing;
}

let current = 'zh';
const sourceText = new WeakMap();
const renderedText = new WeakMap();
const sourceAttrs = new WeakMap();
const renderedAttrs = new WeakMap();
let observer;

export const language = () => current;

function excluded(node) {
  const el = node.nodeType === 1 ? node : node.parentElement;
  return !el || !!el.closest('script,style,code,pre,#log,.no-i18n,[data-lang]');
}

function applyText(node) {
  if (excluded(node)) return;
  const now = node.nodeValue;
  if (!sourceText.has(node) || (renderedText.has(node) && now !== renderedText.get(node))) {
    sourceText.set(node, now);
  }
  const next = current === 'en' ? translateZh(sourceText.get(node)) : sourceText.get(node);
  renderedText.set(node, next);
  if (now !== next) node.nodeValue = next;
}

function applyAttrs(el) {
  if (excluded(el)) return;
  let sources = sourceAttrs.get(el);
  let rendered = renderedAttrs.get(el);
  if (!sources) { sources = {}; sourceAttrs.set(el, sources); }
  if (!rendered) { rendered = {}; renderedAttrs.set(el, rendered); }
  for (const name of ['title', 'placeholder', 'aria-label']) {
    if (!el.hasAttribute?.(name)) continue;
    const now = el.getAttribute(name);
    if (!(name in sources) || (name in rendered && now !== rendered[name])) sources[name] = now;
    const next = current === 'en' ? translateZh(sources[name]) : sources[name];
    rendered[name] = next;
    if (now !== next) el.setAttribute(name, next);
  }
}

function applyTree(root = document.body) {
  if (!root) return;
  if (root.nodeType === 3) { applyText(root); return; }
  applyAttrs(root);
  for (const el of root.querySelectorAll?.('*') ?? []) applyAttrs(el);
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) applyText(node);
}

export function setLanguage(next, persist = true) {
  current = next === 'en' ? 'en' : 'zh';
  document.documentElement.lang = current === 'en' ? 'en' : 'zh-CN';
  document.querySelectorAll('[data-lang]').forEach(button => {
    button.classList.toggle('on', button.dataset.lang === current);
    button.setAttribute('aria-pressed', String(button.dataset.lang === current));
  });
  applyTree();
  if (persist) localStorage.setItem('language', current);
  globalThis.dispatchEvent?.(new CustomEvent('colm:language', { detail: current }));
}

export function initI18n() {
  document.addEventListener('click', event => {
    const button = event.target.closest?.('[data-lang]');
    if (button) setLanguage(button.dataset.lang);
  });
  observer = new MutationObserver(records => {
    for (const record of records) {
      if (record.type === 'characterData') applyText(record.target);
      else for (const node of record.addedNodes) applyTree(node);
    }
  });
  observer.observe(document.body, { childList: true, subtree: true, characterData: true });
  setLanguage(localStorage.getItem('language') ?? 'zh', false);
}

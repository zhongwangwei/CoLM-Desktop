# 前处理工作台重构计划

## 目标

把前处理从两个互不相干的工具，重构成一条可验证、可交接给“基本设定”的数据流水线：

1. 选择站点数据来源；
2. 生成或整理符合 CoLM Desktop 命名约定的站点文件；
3. 探测并转换强迫场；
4. 按当前自然 / PFT / PC / 城市配置检查运行契约；
5. 一键把产物带入“基本设定 / 文件与目录”。

## 不变量

- **不编造科学输入。** 经纬度不能推导出真实 LAI、SAI、PFT 比例或 24 项土壤水热参数；缺失时必须显示外部 rawdata 依赖，不能把“文件写出来了”描述成“已经可以运行”。
- **站点自身优先。** 用户提供的同站 NetCDF 变量优先于全球栅格，全球栅格优先于有文献或模型依据的标称值。
- **生成与运行分级。** 状态只有三种：
  - `self_contained`：站点文件本身满足当前模式；
  - `ready_with_rawdata`：缺项可由已选择且存在的 rawdata 目录提供；
  - `blocked`：缺少当前模式要求的输入，也没有可用 rawdata。
- **站点身份显式化。** CoLM Desktop 生成的文件写入自然 / 城市标记；扫描和建例共用同一判据，不能再用“没有 landtype 就是城市”。
- **产物可被下一步识别。** 文件名统一为 `<站点名>_site.nc`；强迫场统一为 `<站点名>_Met.nc`。生成成功后自动写入站点目录、强迫场目录并重新扫描，不要求用户手工重复选择。
- **输入变化使结果失效。** 修改经纬度、模式、来源、输出目录或 rawdata 后，旧的“就绪”状态不能继续显示。

## 数据契约

### 所有自然站点

- 位置：`longitude`、`latitude`
- 地类：`IGBP_classification`，或由 rawdata 提供
- 基础字段：现有 `REQUIRED_FIELDS` 12 项
- 植被：`canopy_height`、`LAI_year`、`LAI_monthly`、`SAI_monthly`
- 土壤：CoLM `MOD_SingleSrfdata.F90` 无条件读取的 24 项水热参数

### PFT / PC

在自然站点要求之上，植被部分改为：

- `pfttyp`、`pctpfts`
- `canopy_height_pfts`
- `LAI_year`、`LAI_pfts_monthly`、`SAI_pfts_monthly`

这些数组不能从地类编号人工拼造；缺失时必须由同站文件或 rawdata 提供。

### 城市

城市站点走独立契约。Urban-PLUMBER 已覆盖站点可由内置预抽表补齐；其他经纬度必须选择 CoLM rawdata。城市站点不再借用“缺少 landtype”作为身份判据。

## 实施顺序

1. 先补回归测试：模式感知审计、自然站点误判、Tauri JSON 字段、前端命名与交接、强迫场高度阻断。
2. 在 `colm-srfdata` 增加站点种类与就绪审计；扫描和建例只调用这一处。
3. 扩展 `site-new` JSON，返回模式、就绪等级、文件内缺项和外部依赖。
4. 扩展 Tauri 薄壳，完整透传审计结果。
5. 重构前端为三个连续子步骤，共享 `prepArtifacts`，生成/转换后自动交接到基本设定。
6. 用真实自然站点、真实城市站点以及 USGS/PFT/PC 契约做验证；随后运行 workspace、GUI 静态检查和 CI 等价命令。

## 验收

- 只给经纬度、未给 rawdata：能生成结构有效文件，但明确显示 `blocked`，列出真实缺项。
- 给合法 rawdata：显示 `ready_with_rawdata`；建例保留 rawdata 路径。
- 输入完整同站文件：显示 `self_contained`，无需 rawdata。
- 没有 landtype 的自然文件不再被当成城市；已有城市文件仍被识别为城市。
- 生成的 `<站点名>_site.nc` 与 `<站点名>_Met.nc` 可被扫描自动配对并选中。
- 三个强迫场观测高度缺任一项时，转换按钮不可用。

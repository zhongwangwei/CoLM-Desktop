# 参数暴露基线审计

基线提交：`d6850d32e0a897fcfe7df3afa2487c9ab955eb91`（`main`，2026-08-30）。
机器可读哈希与检查结果见
[`artifacts/parameter-audit/baseline.json`](../artifacts/parameter-audit/baseline.json)。

## 现有可写来源

| 来源 | 数量 | 当前读取/写入路径 | 基线状态 |
|---|---:|---|---|
| `MOD_Namelist.F90` schema | 832 | `colm-schema` → `config.rs` → `colm-namelist` | 可写字段已渲染，但常规/专家过滤影响可发现性 |
| `MOD_Const_LC.F90` | 44 个目录项，其中 39 个已审查地类常量 | `colm-case::land_cover`，按 IGBP/USGS 与当前地类解析默认值 | 稀疏 `case.nml` 覆盖；只在专家入口可见 |
| `MOD_Const_PFT.F90` | 87 | `colm-case::pft`，PFT/PC 槽位 1–79 | 稀疏槽位覆盖与恢复已存在；只在专家入口可见 |
| 过程参数类型 | 170 | Tauri 从 Fortran 类型声明/初始化解析，写 case-local 过程文件 | 批量原子写入已存在；只在专家入口可见 |
| 通用调参字段 | 44 | `colm-case::tuning`，并由统一目录交给 Study | 不再维护前端 Study 白名单；范围仍由用户显式提供 |

数量以当前生成产物和代码测试为权威基线。`docs/process-parameter-audit.md`
仍写 831 个 schema 字段，而当前 `generated.rs`/vendor 源为 832；该文档口径漂移
已在本审计中显式记录。后续
`xtask parameter-audit` 必须从源码重新生成并验证这些数量，不能把本表当作生成源。

## 可发现路径与缺口

| 参数族 | 当前路径 | 主要缺口 | 审计状态 |
|---|---|---|---|
| 普通 schema 字段 | 现有顶层分类中的常规表格 | 无跨分类搜索；显示元数据分散在前端 | `editable-common` / `editable-expert` |
| `DEF_LC_*` | 生态与生地化 → 专家参数 | 缺 IGBP/USGS 作用域卡片、别名搜索和覆盖来源 | `editable-expert` |
| `DEF_PFT_*` | 生态与生地化 → 专家模式 PFT 下拉 | 缺普通/PC 语义区分、矩阵视图和搜索 | `editable-expert` |
| 过程参数 | 专家模式 → 对应过程文件 | 缺稳定 ID、统一目录和普通模式可发现性 | `editable-expert` |
| `fveg0_p`、`sai0_p`、`z0mr_p`、`displar_p`、`respcp_p`、`roota`、`rootb`、`dsladlai` | 无发布 GUI 入口 | 尚无审查完成的运行时覆盖、验证和真实回归路径 | `blocked-pending-hook` |
| 维数、通用物理常数、缺测哨兵、求解器保护量、状态/诊断量 | 无 | 不应开放 | `excluded-internal` |

统一机器目录已由 `colm_case::parameters::all()` 与 `xtask parameter-audit` 建立；
当前统计为 `eligible=1220 / editable=1213 / read-only=7 / blocked=8 / excluded=31 / unclassified=0`。`source-inventory.json` 逐项记录 832 个 schema 字段、47 个 LC 源数组（44 个可编辑基础参数）、32 个 PFT 固定参数声明、87 个覆盖宏和 170 个过程字段。新增或删除 LC/PFT/覆盖宏/过程字段而未同步分类时审计直接失败。

## Vcmax 基线

- LCT：`DEF_LC_VMAX25`，默认值来自当前 IGBP/USGS 地类表。
- PFT/PC：`DEF_PFT_VMAX25(<Fortran 槽位>)`，默认值来自当前 PFT 或 PC 分支。
- 两条写入路径均已稀疏化并支持删除覆盖；GUI 已提供 `vcmax` / `vmax25` / 中文别名搜索。
- PC 入口会说明 PC 组分，并在可用时并列显示普通 PFT 默认与当前 PC 默认。

## 基线验证

2026-08-30 执行并通过：workspace 测试、格式检查、workspace/Tauri Clippy、
`xtask check-gui`、全部 Node GUI 测试、142 个 Tauri 单元测试和全部非忽略的
oracle 测试。仓库标记为 ignored 的 3 个真实外部数据运行测试没有被伪装成已执行。

P0 要求的独立 IGBP、PFT、PC 三类真实算例哈希在基线仓库中没有完整、同等级的
可复跑证据；在补齐相应数据/内核验收前保持为明确缺口，不以较窄的单元测试替代。

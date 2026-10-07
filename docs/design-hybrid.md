# CoLM 混合模型框架（AI + 物理）设计

分支 `colm-hybrid`。本文是方案，P0 动手前以此为准；实测结论照例记到 `docs/implementation-verification.md`。

## 0. 原则

1. **AI 只能通过插槽介入。** 物理模型里预留固定的插槽；没接模型的插槽走原物理代码。
2. **不接模型时逐位不变。** 默认路径继续与 Fortran 逐位一致，现有配对、黄金回归、CI 一条都不受影响。
3. **守恒由物理保证。** 过程插槽只输出系数、阻抗、比例，不输出通量；订正插槽的增量必须作为显式的订正通量记进收支。
4. **结果确定、可复现。** 推理在 CPU 上、线程数固定、不用随机或非确定性的算子；模型文件的哈希进阶段指纹和续跑文件。
5. **只有 Rust 引擎支持。** Fortran 引擎看到混合配置时报错，不静默忽略。

## 1. 插槽

| 类型 | 何时调用 | 输入 → 输出 | 例子 |
|---|---|---|---|
| 参数插槽 `param` | 装配时，或按月/年 | 静态属性、气候态 → 物理参数 | Vcmax、土壤水力参数、基流系数 |
| 过程插槽 `process` | 每步，在某个参数化的位置 | 状态、强迫 → 系数/阻抗/比例 | 气孔导度、土壤蒸发阻抗、雪反照率、产流比例 |
| 订正插槽 `correction` | 每步末 | 状态 → 状态增量 → 守恒投影 | 土壤湿度、雪水当量 |

每个插槽在代码里声明三样东西：
- **可用特征**：名字 → 取值函数。例如 `t_leaf`、`vpd`、`par_sun`、`soil_water_stress`、`lai`、`clay[k]`。
- **输出**：名字、物理范围、变换（`softplus`/`sigmoid`/`clamp`/`identity`）。
- **物理默认值**：不接模型时用的原代码。

配置文件只按名字挑选特征和输出，换特征不用改代码。

## 2. 配置：`hybrid.toml`

放在算例目录、和 `case.nml` 并列，**不进 namelist**：Fortran 读 namelist 遇到不认识的变量会报错。

```toml
[[slot]]
name = "soil_hydraulics"          # 插槽名，必须是引擎注册过的
kind = "param"
model = "models/soil_hyd.onnx"    # 相对算例目录
sha256 = "…"                      # 加载时校验
features = ["clay", "sand", "om_density", "bd", "climate_p", "climate_t"]
normalize = { file = "models/soil_hyd.norm.json" }   # 每个特征的 mean/std
outputs = [
  { name = "vg_alpha", range = [1e-4, 1.0], transform = "sigmoid" },
  { name = "vg_n",     range = [1.05, 3.0], transform = "sigmoid" },
]
```

接入方式：
- colm-cli 发现算例目录下有 `hybrid.toml`：
  - 如果是 `--engine fortran`，报错退出；
  - 否则把它传给 colm-rs（`colm-rs --hybrid <file>`）。
- `hybrid.toml` 和其中所有模型文件的 sha256 计入 colm 阶段的指纹（`crates/colm-cli/src/fingerprint.rs`）。改了模型就重跑 colm 阶段；mksrfdata、mkinidata 不受影响。
- 续跑文件的全局属性里写上各插槽的模型哈希。续跑时哈希不一致就报错，除非显式允许。

## 3. crate 结构

新建 `crates/colm-hybrid`：

```text
colm-hybrid/
  src/lib.rs        Slot、SlotKind、Feature、Output、Transform、HybridConfig（解析 hybrid.toml）
  src/backend.rs    trait Surrogate { fn infer(&self, x: &Matrix) -> Result<Matrix>; }
                    TractBackend（tract-onnx，CPU，单线程推理、批内由调用方并行）
  src/normalize.rs  特征归一化、输出变换与范围裁剪
  src/tap.rs        特征抓取：物理照常跑，把插槽输入与物理输出写成训练数据
```

- **推理后端**用纯 Rust 的 `tract`，可以静态编进 sidecar，用户机器不用另装东西，Windows 一样。
  - 不用 onnxruntime：它要带动态库，和 sidecar 的分发方式冲突（同 HDF5 那条教训）。
  - 第一次引入要联网下载 crate，入库的 `Cargo.lock` 冻结版本。
- **`Matrix`** 是行主序的 `Vec<f32>` 加行列数：行 = patch，列 = 特征。f32 足够，也是 ONNX 的常用类型。
- **确定性**：tract 不引入随机；批内各行的计算互不相干；不开 tract 的多线程，并行由引擎按批外层控制。

## 4. 接入引擎

### 4.1 参数插槽（P0/P1）

装配完 `StandardLctRestartTemplate`（`crates/colm-runtime/src/assembly.rs`）之后，统一调用一次：

1. 对全部 patch 收集特征，组成一个批，调用 `infer`。
2. 把输出写回模板里对应的参数。

参数名到"模板里的哪个字段、按层还是按 PFT"的映射，在 colm-runtime 里集中登记：`HybridParam { name, apply: fn(&mut Template, &[f32]) }`。

- 派生量必须重算，不能只改原始字段。例如改了 van Genuchten 参数，要按现有的装配路径重算 `porosity`、`suction_mm`、`conductivity_mm_s` 等。
- 写回点在派生量计算之前，所以要找准装配顺序；这是 P0 要摸清的第一件事。
- 之后的物理代码完全不动。

### 4.2 过程插槽（P2）

现在每步是 rayon 按 patch 并行调用 `advance_patch`。过程插槽在一步中间，逐 patch 调网络开销太大，要把这一步拆成三段：

1. **准备**（并行）：每个 patch 推进到插槽处，交出特征行，保存续算需要的中间量。
2. **推理**：整批一次 `infer`。
3. **继续**（并行）：每个 patch 取回输出行，接着跑物理。

规则与验收：
- 第一版只支持一步一个汇合点。
- 拆分必须满足"模仿物理"测试（见第 6 节第 2 条）：接一个返回物理值的模型时，结果逐位不变。

### 4.3 订正插槽（P3 之后）

每步末对状态加增量；水量增量按层记成订正通量，进水量平衡检查和 history（`f_hybrid_*`）。

## 5. 训练侧（`python/colm_hybrid/`，不进 Rust workspace）

1. **抓取**：`colm-rs --hybrid-tap <dir>`，物理照常跑，各插槽的特征和物理输出写成 netCDF。
   - 用于预训练（先让网络模仿物理）和计算归一化统计。
2. **无梯度训练**：
   - 网络参数 → 生成模型 → 批量调用 colm-cli 跑 PLUMBER2 站点 → 用 oracle 的指标（KGE/RMSE）算损失 → CMA-ES 或贝叶斯优化。
   - 适合参数插槽和小网络。
3. **可微孪生**（P2）：
   - 用 JAX 或 PyTorch 重写插槽周围的那段物理，先和 Rust 版做配对比较（前向一致），再端到端训练，导出 ONNX。
4. **数据切分**：按 IGBP 类型和气候区留出站点做外推检验；区域上再加 GLEAM、SMAP、GRACE 和 OBRef 径流。
5. **导出**：模型的 ONNX、归一化文件和 sha256，一起写进 `hybrid.toml`。

## 6. 验证

1. **空插槽逐位不变**：没有 `hybrid.toml` 时，所有现有测试与配对照旧。进 CI。
2. **"模仿物理"逐位不变**（最关键）：
   - 参数插槽：接一个原样输出物理参数的模型。
   - 过程插槽：接一个复现物理系数的模型，实现成测试专用后端，不走 ONNX。
   - 两种情况下结果都必须与无插槽逐位相同，用来证明接入与拆分本身没改变任何东西。进 CI。
3. **守恒**：混合运行通过现有的水量、能量平衡检查和示踪物收支追踪。
4. **确定性与续跑**：同配置跑两次逐位相同；一次跑完与中途续跑逐位相同；模型哈希不一致时续跑报错。
5. **混合黄金基线**：每个发布的模型配一个固定的单点算例和期望输出，独立于 Fortran 黄金基线。
6. **长期稳定**：多年 spin-up；监控特征是否超出训练范围（按归一化统计记录超界比例）、平衡误差是否累积。

## 7. GUI

界面上统一叫"AI 参数化"，不叫"混合/hybrid"，免得与甲烷淹没方案的 `hybrid` 撞名。已在第 610 轮实现：

- **运行页的"AI 参数化"卡片**（`gui/dist/app/hybrid.js`）：
  - 列出本次运行的算例：地表模式、是否装了模型（区分外部导入与参数调优训练）、作用参数、模型文件。
  - 每个装了模型的算例有"检查"（`hybrid-check`，表格列出特征与输出的范围、均值、标准差和作用行数）与"移除"（`hybrid-remove`）。
  - 选 Fortran 内核时，卡片里提前警告；运行 colm 阶段前再拦一次。
- **导入表单**：模型文件（`.onnx`、`.mlp.json`）、作用方式（按算例的地表模式自动选）、特征、输出（名字、范围、变换）、可选的标准化文件。装到本次全部算例，经 `hybrid-install`。
- **参数调优第 3 步的"同时训练 AI 参数化"**：作用方式、特征、输出、网络规模（线性、1 层 4 个节点、1 层 8 个节点）；实时显示权重数，超过种群时给出提醒。可以只训练网络、不调其他参数。
- **最优成员卡片**：网络单独一行，显示"训练出的网络，权重数 N"。应用时提示 `hybrid.toml` 与 `models/` 会一起写进新算例。
- 前端不解析 TOML：配置由 `colm-cli hybrid-info` 以 JSON 给出（含地表模式与模型校验结果）。

## 8. 分阶段计划

| 阶段 | 内容 | 验收标准 |
|---|---|---|
| **P0 基础设施** | `colm-hybrid` crate（配置、tract 后端、归一化、变换）；colm-cli 识别 `hybrid.toml`、Fortran 拒绝、指纹；一个参数插槽的接入通路；特征抓取 | 第 6 节第 1、2 条（参数插槽）进 CI；能加载一个 ONNX 并改变参数 |
| P1 参数学习 | 1–2 个参数插槽（土壤水力或 Vcmax）；无梯度训练脚本；PLUMBER2 留站检验 | 留出站点的 KGE 优于纯物理；混合黄金基线 |
| P2 过程插槽 | 单步拆分（一个汇合点，建议气孔导度）；可微孪生训练 | 第 6 节第 2 条（过程插槽）；守恒检查通过；多年稳定 |
| P3 区域与 GUI | 空间算例批量推理与性能；GUI；订正插槽 | 区域运行开销可接受；结果可复现 |

## 9. P0 的改动范围

| 位置 | 改动 |
|---|---|
| `crates/colm-hybrid/`（新） | 配置解析、`Surrogate` trait、`TractBackend`、归一化与变换、单元测试（含一个小 ONNX 测试模型） |
| `crates/colm-runtime/src/assembly.rs` | 参数插槽登记表与写回点（派生量重算之前） |
| `crates/colm-runtime/src/bin/colm-rs.rs` | `--hybrid <file>`、`--hybrid-tap <dir>` 参数 |
| `crates/colm-cli/src/main.rs`、`fingerprint.rs` | 发现 `hybrid.toml`、Fortran 引擎拒绝、指纹 |
| `Cargo.toml`/`Cargo.lock` | 新成员与 `tract-onnx` 依赖 |
| 测试 | 空插槽与"模仿物理"两条逐位测试 |

P0 不改任何物理代码，GUI 也不动。

## 10. 已定（2026-10-07）

1. 推理库：`tract-onnx` **0.22**（0.23 要 Rust 1.91，工作区 MSRV 是 1.85.1）。放在 `colm-hybrid` 的 `inference` feature 里（默认开）；colm-cli 只解析配置、算指纹，关掉它不链 tract。
2. 第一个参数插槽是 **`land_class`**，不是土壤水力参数。
   - 土壤水力参数牵动热参数和 mkinidata 按原参数算出的初始状态，只改一部分会让状态与参数对不上。
   - 地类表各列走现成的 `LandClassOverrides`：每个模板自带一份 `physics`，按 patch 覆盖、装配照原路径派生，物理代码不改。
   - 输出名就是 29 个 `DEF_LC_*` 列名，Vcmax（`DEF_LC_VMAX25`）是其中一列，所以这个插槽是通用的。
3. 训练侧代码放本仓库 `python/`。
4. 特征按名字从常数重启读（`name` 或 `name[k]`），不在 Rust 里逐个登记；换特征只改配置。
5. `model`/`sha256` 可以都不给：只能用于特征抓取（`HybridConfig::load_spec`），正式运行要求两者都有。

## 11. P0 落地（第 605 轮）

| 位置 | 内容 |
|---|---|
| `crates/colm-hybrid/` | `HybridConfig`（`load`/`load_spec`、sha256 校验、合成指纹）、`Matrix`、`Surrogate`、`TractBackend`、`FnBackend`、归一化与输出变换、`Slot::evaluate` |
| `crates/colm-core/src/land_cover.rs` | `ClassConstants::table_value(name)`：`DEF_LC_*` 列的原始表值 |
| `crates/colm-runtime/src/hybrid.rs` | `Hybrid::patch_physics`（`land_class` 插槽，只作用于 `patchtype == 0`，PFT/PC 报错）、`write_land_class_tap` |
| `crates/colm-runtime/src/bin/colm-rs.rs` | `--hybrid`、`--hybrid-tap`；单点与空间两处装配前按 patch 取物理参数 |
| `crates/colm-cli/` | 发现 `hybrid.toml` 传 `--hybrid`（预检与正式运行）；`--engine fortran` 拒绝；colm 段指纹记 `hybrid (hybrid.toml)` |
| `python/colm_hybrid/make_test_model.py` | 生成测试用 `linear.onnx` |

**还没做**（P1 再做）：
- 续跑文件里记模型哈希；
- 训练脚本；
- 过程插槽。


## 12. 在 Study 里训练（2026-10-07）

调优 Study 的 spec 可以带一个 `hybrid` 段，用 DE 直接训练一个小网络；训练好的网络与 `hybrid-install` 装上的外部模型走同一条运行路径。

```json
"hybrid": {
  "slot": "pft",
  "features": ["pftclass", "pftfrac"],
  "outputs": [{"name": "DEF_PFT_VMAX25", "range": [20.0, 80.0], "transform": "sigmoid"}],
  "hidden": [4],
  "activation": "tanh",
  "weight_range": 2.0
}
```

- **决策变量**：网络的每个权重是决策向量的一维，名字是 `hybrid:w00000`…，排在采样参数之后。
  - 次序是逐层先 `weights[输入][输出]`（行优先），再 `bias`。
  - 搜索区间是 `[-weight_range, weight_range]`。
  - 采样参数可以为空（只训练网络）；网络输出不能同时是采样参数。
- **方法限制**：只用于 DE 调优。OAT/LHS 每个权重要 2～10 个成员，搜不动。权重上限是 `MAX_HYBRID_WEIGHTS = 400`，实际能搜好的网络要小得多。
- **输出变换**：只许 `sigmoid` 和 `clamp`，保证任意权重向量都落在 `range` 内、成员不会因越界失败。
- **特征标准化**：不给 `normalize` 时，`study-create` 在每个基础算例上用无模型配置空跑 `colm-rs --hybrid-dry-run`，按行数合并成总体均值与标准差，冻结进 spec（进 `spec_sha256`）。
  - 常数特征的标准差取 1。
  - 这一步要求基础算例已经跑过 mkinidata。
- **基线**：基线成员 `m000000` 没有权重。CSV 里它的权重格子为空，也不写 `hybrid.toml`，是纯物理的参照。
- **成员**：有权重的成员写 `models/study.mlp.json`、`models/study.norm.json` 和 `hybrid.toml`（带 sha256）。
  - 续跑时核对 `hybrid.toml` 能加载、模型的 sha256 对得上。
  - 导出最优成员时，这三个文件随算例一起导出；预览里合成一行 `hybrid slot <名>`，不逐个列权重。
- **约束**：
  - 只能用 Rust 引擎。
  - 基础算例不能已有 `hybrid.toml`，否则成员与基线不在同一物理参照上。
- **旧 Study**：`hybrid` 段为空时不序列化，旧 manifest 的哈希不变。

## 13. 区域与全球（2026-10-07 起）

模型逐 patch 推理，单点与空间共用一套代码：空间运行在每个分块装配前取特征、推理。Study 训练仍然只用站点。按下面的次序扩展：

1. **空间空跑检查**（第 609 轮，已做）：`--hybrid-dry-run`、`hybrid-check` 支持空间算例，逐块汇总后合并。在区域算例上实测：不带模型时与 main 逐位一致，带模型速度不变。
2. **气候派生特征**（第 611 轮，单点已实测，空间待实测）：
   - `colm-cli hybrid-climate <case> --kernel K`（即 `colm-rs --hybrid-climate`）在运行时段（不含预热）内按模型步长取强迫，逐步累积。单点走与运行相同的 `runtime_at_calendar_time`；空间走网格强迫，经与运行相同的面积权重映射到 patch（双线性插值暂不支持）。
   - 每个 patch 得到 `clim_tair`（K）、`clim_tair_amplitude`（最暖月与最冷月的月均气温之差，K）、`clim_prec`（mm/day）、`clim_swdown`（W/m²）、`clim_vpd`（kPa，Tetens 水面饱和水汽压）。
   - 结果写到算例目录 `hybrid_climate/climate<块后缀>.nc`，按常数重启的分块切文件。**不写进重启**，原因有二：
     - 重启文件与 Fortran 对照不受影响；
     - 不带模型的运行天然逐位不变。
   - 特征名以 `clim_` 开头时从这里读；文件缺失时报错并提示先算。
   - 这些文件进入 colm 段指纹（`hybrid climate (...)`）和续跑标记的指纹（`Hybrid::fingerprint`）。
   - Study 物化成员（包括导出最优成员）时随算例一起拷过去。
   - 足迹全在缺测强迫格上的 patch（运行时整步跳过）取其余 patch 的平均。
3. **两步训练**（第 612 轮，拟合与交叉验证已做；用真实 Study 的端到端待数据）：
   - 第一步：各站独立的参数调优 Study，率定 `DEF_PFT_X(k)` 或 `DEF_LC_X`。
   - 第二步：`colm-cli hybrid-fit --studies d1,d2,… --network net.json --kernel K --out x.mlp.json`。
     - 读每个 Study 的最优成员。
     - 在它的基础算例上用 `colm-rs --hybrid-dry-run --hybrid-tap` 抓逐行特征，只取样不模拟。tap 新增 `class` 列（PFT 类或地类）。
     - 按分类对上率定值，没有率定值的行丢掉。
     - 拟合（`colm_hybrid::fit`）：目标先按输出变换反算（sigmoid 取 logit）。线性网络用加权岭回归闭式解；有隐藏层时，目标先标准化，再用全批量 Adam，尺度最后折回输出层。种子固定，结果确定。
     - 写出 `x.mlp.json`、`x.norm.json`、`x.fit.json`（样本来源、训练误差、留一站误差），并打印对应的 `hybrid-install` 命令。
   - 两个以上 Study 时做留一站交叉验证（参数空间）。正向检验是把模型装到留出站点，跑出通量再评估。
   - `--weight pftfrac` 按 PFT 面积份额加权。
   - 端到端的 DE 训练（第 12 节）只用于小网络或最后的微调。
4. **外推检查**（第 613 轮，已做）：
   - 标准化文件可以带 `min`/`max`，即各特征的训练范围。`hybrid-fit` 取训练样本的范围；Study 冻结标准化时取各基础算例范围的并。旧文件没有这两项，照常可用。
   - `hybrid-check` 与空跑报告 `outside_training`，即超出训练范围的行数（边界留 1e-9 的相对容差）。没有记录范围时不报告。
   - 插槽可设 `outside = "physics"`（`hybrid-install --outside physics`，GUI 导入表单里的勾选项）：超出范围的行不覆盖，用纯物理参数。缺省是 `apply`，与没有这个选项时完全相同。

1. P1 先做哪个参数插槽：土壤水力参数（结构简单、派生量多）还是 Vcmax（直接影响 ET/GPP）。
2. `tract` 与 `candle` 二选一：前者直接吃 ONNX，后者便于将来在 Rust 里训练。倾向 `tract`。
3. 训练侧的 Python 代码放在本仓库 `python/` 下，还是单独一个仓库。

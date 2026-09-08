# CoLM Desktop 逐模块审查与修复报告（2026-09-08）

状态：本轮逐模块审查、确认问题修复与可运行验证已完成；剩余平台/基线/条件风险已列明。原始基点 `6f91ade`；本报告记录审查完成时的验证状态。修复随后按用户要求分组提交至 `colm-destop-spatial`，通过 PR 提请合入 `main`；未更新发布内核或黄金基线。

## 1. 结论与边界

本轮覆盖 Rust workspace、GUI、执行/数据/结果接口、构建与 CI，以及 vendored CoLM 的主要模块边界。确认问题按“复现—共享入口根因修复—回归”处理；没有增加依赖、替换科学参数化或以宽松容差掩盖失败。

**最重要的发现**：算例名称和 symlink 可使清理路径越界；原生 Fortran 的 shell 文件操作无法安全处理合法路径；部分科学输入会被静默转换成错误整数；异步 GUI 请求会将旧算例结果写到新算例；DA/BGC 存在月索引、闰年、湿度类型和未初始化边界错误；旧 golden 回归的内核/基线来源不一致。

这是一轮代码与运行合同审查，**不是对全部物理算法、所有配置组合或多年科学结果的证明**。仓库有 368 个受控 Fortran 文件、175 个受控 Rust 文件；下面区分深入修复、调用链抽查、动态验证和未覆盖条件，不以编译通过冒充逐行科学验证。

## 2. 模块覆盖及详细报告

| 模块 | 本轮检查与结果 | 证据/详细记录 |
|---|---|---|
| colm-case | 目录/名称、字段生成、spinup 日期和整数边界；共享验证替代调用方重复计算 | `layout.rs`、`build.rs`、各单元测试；[GUI/共享 spinup](audit-2026-09-08-gui.md) |
| colm-namelist | 引号、重复赋值、注释、组边界、原始行尾；修复解析与编辑合同 | `parse.rs`、`document.rs`、`value.rs`；37 单元 + 5 真实 roundtrip |
| colm-schema | 字段提取、默认/枚举/运行条件、生成表 drift；未修改生成表语义 | 全单元、curated、drift；源声明行号保持一致 |
| colm-kernel | manifest、阶段产物、stdout 成败、进程终止与路径合同 | 45 单元；真实三阶段；目录不再被误判为 NetCDF 产物 |
| colm-forcing | CSV/NetCDF 输入、日期/cadence、单位/QC/gapfill、namelist 输出 | [输入报告](audit-2026-09-08-inputs.md)；134 单元 + 8 真实数据测试 |
| colm-srfdata | raster 网格/时间索引、地类整数、站点/PFT/USGS、网格/shapefile/urban 表边界 | [输入报告](audit-2026-09-08-inputs.md)；103 单元、5 栅格、6 真实站点测试（90 站扫描） |
| colm-cli | new/run/history/metrics、管理输出目录、清理安全 | [CLI/HIST 报告](audit-2026-09-08-cli-hist.md)；160 主测试 |
| Study | 命名、物化、结果边界、checkpoint、pause/cancel/export、科学指标调用 | 同上；6 tuning 集成；既有拒绝越界和恢复测试 |
| colm-hist | 时间轴、观测 QC/fill/非有限值、配对形状、KGE/相关系数等边界 | 同上；最终核心测试 39 项，真实 metrics 4 项 |
| GUI 后端 | typed writes、批量写入、history 名称、process groups、spinup、sidecar 生命周期 | [GUI 报告](audit-2026-09-08-gui.md)；Rust 单元、Clippy、IPC |
| GUI 前端 | case/scan/params/timing/histvars 异步竞态、结果输入、固定 HTML 与文本写入 | 同上；11 个 Node 测试文件（含新的乱序 IPC 动态探针） |
| oracle / xtask / CI | 大整数逐位判官、golden-run 名称、生成器、平台脚本、真实 golden job 状态 | judge 11、histmap 4、golden-run 1、xtask 集成；CI success/failure/skipped/cancelled 四分支 |
| share / mksrfdata / restart | native mkdir/copy/list/rename、RegionClip 父目录、经度非有限与超大输入 | [native IO](audit-2026-09-08-native-io.md)；生产 Fortran/C 可执行探针与完整内核 |
| postprocess | 两个 concatenate 程序的 shell 删除、字面路径、失败保留目标 | 同上；完整 postprocess 构建及 helper 动态测试 |
| BGC / DA / LULCC / URBAN | 日期/索引/类型、数组生命周期和运行开关；确认 bug 窄修复，条件风险单列 | [物理运行报告](audit-2026-09-08-physics.md) |
| HYDRO / TRACER / CaMa | 调用、运行开关、数组和并行边界；不修改无证据的物理公式 | [补充审查](audit-2026-09-08-hydro-tracer.md) |
| 主物理驱动 / 辐射 / 雪 / 土壤 / 湖泊 | 控制流和实现边界抽查、已有测试、默认 debug 真算例 | 物理运行报告；不等于多气候/多年守恒验证 |
| preprocess | 独立旧 Makefile 引用不存在的 precision/io 模块，HDF5 程序还缺 provider | native IO 报告；标明不受支持，不猜测恢复外部数据流水线 |

## 3. 主要确认问题与修改

等级：S2 为安全、错误输出或重要流程中断；S3 为局部正确性、生命周期或诊断错误。更细的复现、函数与文件锚点见分报告。

| ID | 等级 | 问题 | 最小根因修复 |
|---|---|---|---|
| A01 | S2 | `DEF_CASE_NAME='../other'` 等进入输出/历史/Study/golden 工作路径 | 共享 `validate_case_name`：单组件、非空、≤256 字节、无分隔符/冒号/控制符/首尾空白；读写入口复用 |
| A02 | S2 | `clear_history` 跟随 history 或 out 祖先 symlink，可能删外部文件 | 在 CLI 管理边界逐层拒绝 symlink，保留已有内容 |
| A03 | S2 | 手改 `DEF_dir_output` 后 Fortran 输出与 CLI 指纹/清理路径分叉 | 运行前 fail-fast；本轮不冒充支持任意自定义输出目录 |
| A04 | S2 | Fortran `mkdir -p`、`cp`、`ls`、`mv` 的路径被 shell 拆分/解释 | 共享 native filesystem 模块和 C shim；删除旧 Windows sed 补丁及所有相关 Fortran shell 调用 |
| A05 | S2 | RegionClip 在创建 mesh 父目录前复制 `mesh.nc`，正常路径也失败且忽略状态 | 在复制前建父目录，stream copy 失败显式停止 |
| A06 | S2 | 新 copy 实现复核发现同文件/硬链接或目录源可导致目标截断；Windows rename 先删目标会丢失旧文件 | 截断前检查文件类型和 native identity；Windows 原生 replace-rename，不提前删除目标；失败保留目标回归 |
| A07 | S2 | NaN/Inf/越界经纬度经浮点转整数落到边界像元；0 时间下标下溢 | 公开 raster 输入边界统一验证，时间索引必须 1-based |
| A08 | S2 | NetCDF 地类 NaN/小数/越界值通过 `as i32` 静默改变分类 | 转换前检查有限、整数及 IGBP/USGS 范围；创建入口同样验证 |
| A09 | S3 | CSV 大整数先饱和转换，再做地类判断 | 公用整数解析处先检查 i32 范围 |
| A10 | S3 | namelist 和 forcing 输出未正确处理 `O'Brien` / `O''Brien` | Fortran 定界符加倍编码/解码，保留字面路径 |
| A11 | S2 | 同字段重复赋值：编辑器取第一条、Fortran 用最后一条 | get/set/insert 统一选最后有效赋值；保留历史原文 |
| A12 | S3 | CRLF/无末尾换行被改写；注释内 `&`/组尾误判；接受组外赋值 | 保留原始行尾，仅按引号外有效代码识别结构，拒绝损坏组边界 |
| A13 | S2 | 旧 GUI 异步请求在快速 A→B 切换后仍修改当前界面/状态 | selection/render generation 在跨 await 写入点检查；不引入请求取消框架 |
| A14 | S3 | scan 按钮禁用期间改变路径，延迟 click 丢失且旧结果显示到新路径 | 捕获扫描输入、拒绝过时响应、完成后补扫最新路径 |
| A15 | S2 | 2月29日加一年产生不存在日期，年份转换溢出，JS `|0` 截断 | spinup 截止计算移到共享 Result 边界，拒绝 repeat 超过 Fortran INTEGER 和秒数超界；CLI/GUI 传播错误；JS 安全整数输入 |
| A16 | S3 | process namelist 组名后有注释时，GUI 隐藏可插入默认字段 | 组名识别去掉行尾注释，复用解析结果 |
| A17 | S3 | Inf 观测或非有限 history time 污染指标；形状不一致被部分配对 | 配对前有限性与 value/QC/time 长度合同检查 |
| A18 | S3 | `exists()` 将目录当成必须生成的 NetCDF 文件 | 产物必须 `is_file()` |
| A19 | S2 | 判官把相邻 i64/u64 的 2^53 与 2^53+1 转成相同 f64 | 整数按原始字节比对，不改浮点 NaN/容差规则 |
| A20 | S3 | CI 仅凭环境变量宣称 golden 已运行 | 状态 job 等待真实 golden job，报告 `needs.golden.result` |
| A21 | S2 | 经度每次 ±360：大值极慢，±1e36 不再变化而永久循环，NaN 被放过 | 共享 normalize 拒绝非有限/无法解析一圈的值，其余越界值 modulo；正常值含负零保持原样 |
| A22 | S2 | GRACE nextmonth 双加一，1月→3月、12月→2月 | `mod(month,12)+1`；12个月动态回归 |
| A23 | S2 | SYNOP `qref` 被声明为整数，分数湿度截断 | 保留 `real(r8)` 输入及下游传递 |
| A24 | S2 | SASU 硬编码第365天，闰年提前触发年末处理 | 复用现有 `isendofyear`，不新增日期算法 |
| A25 | S3 | 奇数 ensemble 只填成对列，最后一列未初始化 | 在采样前拒绝不支持的奇数个数，不擅自改变采样策略 |
| A26 | S3 | DA end 漏释放自身分配数组 | 在现有 end routines 添加 guarded deallocate |
| A27 | S2 | 首次 GRACE 无上月观测时除以零，debug FPE 可停止 | 上月平均只在已有上月观测时计算；见动态探针与物理报告 |

| A28 | S2 | LULCC 找不到源 class 时仅打印，继续使用未初始化的 frnp_ | 保留诊断并在使用前 CoLM_stop；不猜测回退类别 |
| A29 | S2 | 动态湖泊总深度为零或极浅时新层数组未初始化 | 干湖保留原状态，非法厚度拒绝，正厚度保证执行有界重映射；生产子程序动态测试 |
| A30 | S3 | CaMa 以 VAR×0 判 NaN/Inf，在 FPE 构建中检测本身崩溃 | IEEE finite 分类，保持原 NaN/±Inf 均为异常的语义和调用签名 |

| A31 | S2 | RiverHistConcatenate 在 serial 模式调用不存在的 MPI 初始化符号；MPI非master又在namelist广播前退出 | MPI生命周期按USEMPI守卫，所有rank先参加read_namelist，再退出非master；serial/MPI启动探针与完整链接验证 |

## 4. 验证与未掩盖的失败

### 4.1 桌面与数据层

- root `cargo test --workspace --lib --bins --exclude colm-cli`：最终455项通过（含 case 64、forcing 134、hist 39、kernel 45、namelist 37、schema 15、srfdata 103 和工具测试）。
- 共享 spinup 最终补查后，case 64、CLI 主二进制160、Study tuning 6、GUI Rust159通过；11 个 Node 文件通过。所有平台无关测试与实际 GUI 窗口人工交互分开，不宣称已做视觉验收。
- root / GUI `cargo fmt --check` 和 `cargo clippy --workspace --all-targets -- -D warnings` 在源冻结后均通过（`final-clippy-frozen.log` / `final-gui-clippy-frozen.log`）。CLI/oracle 可执行文件已重新构建。
- Namelist roundtrip 5、oracle judge 11、histmap 4、golden-run 名称 1、schema/hist drift通过；xtask 15单元+50集成通过；parameter-audit重新生成1220项并确认无diff。IPC 为 74 注册、73 调用、6 事件，均解析到提供方。
- 真实只读数据：`PLUMBER2_ROOT=/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s`；`COLM_RAWDATA=/Volumes/data02-1/zhwei/CoLMrawdata`；runtime 为 `/Volumes/data02-1/zhwei/CoLMruntime`。forcing 8、raw raster 5、real_sites 6（90站点）和 metrics 4 已跑；未更改外部数据。
- 未设置数据环境变量而返回的可选测试只算“入口编译/调用适配”，**不算真实数据通过**。

### 4.2 原生与物理边界

- `test_kernel_filesystem.py` 编译实际 Fortran/C helper 及实际 namelist 目录调用，测试空格、单引号、&、百分号、分号、命令替换字面量、绝对/相对/父路径、文件冲突、多块二进制复制、同文件/硬链接、排序列举、POSIX 反斜杠、根目录前缀、失败重命名保留目标。
- `test_longitude_normalization.py` 提取实际生产过程：预热同一二进制后，旧1e12输入计算超时；修复后正常/大值/NaN/Inf/哨兵全部通过。
- `test_physics_audit.py` 使用实际生产表达式/过程的动态 Fortran 探针与必要源码合同检查；覆盖月索引、年末、湿度类型、ensemble 和 GRACE 边界。
- 受控 vendor Python 14 文件：最终 **26 通过，6.85秒**（`vendor-tracked-frozen.log`）。早期 3 个 MPI 探针30秒超时，沿用同一已编译探针延长观察均退出0；最终完整26项重跑也通过。未关闭系统安全机制或删断言。
- 上游 `.gitignore` 排除的本地临时 vendor 套件另跑：608通过、35 subtests通过、1失败。失败为 BIF harness 使用旧 `.bld` ABI（期望 `get_lonlat_radian`，实际 `forc_free_mem`）；不改/删除用户的旧 `.bld`，也不把原失败改写成通过。随后在本轮保留的新鲜构建副本上，完全相同BIF evaluator的1/2/4 rank均通过，确认旧失败来自陈旧ABI而非该探针的物理断言。
- C `-Wall -Wextra -Werror -fsyntax-only`、shell syntax、Python 编译和 `git diff --check`；Linux/Windows CI 新增 native 路径门禁。**远程 CI 尚未执行，不宣称 Windows/UNC/非 ASCII 路径已原生实测。**

### 4.3 完整内核与 golden 的区别

1. 入库 golden 清单记录 `4894833`；原 `kernels/default` 清单记录 `f427762`；本次源码起点 `6f91ade`。三者不同。
2. 实际 `generated_case` 三阶段可运行，但旧 golden 严格比对失败：不仅末位舍入，还有 `f_lake_icefrac` / `f_wetwat` 的0与缺测填充值差异。日志 `generated-case.log` 保留，**未覆盖 golden、未降低判定标准**。
3. 在独立 tmp 构建当前源码 debug default；与 native mkdir 修改后、实际路径 `literal O'Brien & %DATA% case` 的相同算例对照，三阶段成功且 **127变量/10维逐位一致**，仅忽略既有 `create_time`。这是同源前后回归，不是旧 golden 通过。
4. 最新所有核心改动完成后，default/unstructured debug 内核均从干净副本完整编译链接。新增完整postprocess构建发现A31后，单独重编更新的River程序并用真实make链接全部postprocess目标：default/unstructured均通过。已编译且逐源文件核对的其余对象复用，不把dry-run当成功。
5. 最终9个阶段（3个算例×3阶段）全部成功：CLI普通路径、直接原生执行的 `literal O'Brien & %DATA% case`、使用实际外置rawdata/runtime目录。每个都与修改前的同源debug参考逐位 **127变量/10维一致**（仅忽略create_time）。`final-e2e.log` / `final-e2e-evidence.json`保存路径、标记、产物及比较；仍不宣称旧golden通过。
6. 最终colm二进制SHA-256：default `6a291766d42912d8f8b5cf564363d25701a699ebf831c008b3cecdd5f43d2701`；unstructured `642ad29d2245b0a6aec7d75f469b50c4dcb0e32cb05c3cbf263046e1e90564e6`。仅写入本次tmp审计产物，未覆盖发布目录。

### 4.4 集成中已纠正的测试问题

- MinGW 清理回归原来从进程启动计时，会被 macOS 首次可执行文件扫描拖慢而误判；计时改为收到成功标记后，仍要求清理少于15秒、仍能抓到原30秒坏清理。45项 kernel 回归通过，生产超时逻辑未改。
- 增加 namelist `USE` 行使 schema 的832个源码行号漂移；将 import 放入已有空行后重新生成，生成表无语义/内容变更，再做 drift 检查。不通过更新错误默认值来绕过失败。
- 一次本地命令把 `study_cli_tuning` 写成不存在的 `study_tuning`；该命令失败及后续被短路如实保留，修正目标名称后重新运行。不是产品 bug。

### 4.5 最终源身份与独立复核

- 最终保留的 default/unstructured 构建副本与工作区370份 Fortran/C 文件逐文件相同。内容摘要（按相对路径、NUL、内容、NUL顺序 SHA-256）：`a35f46564b8b41f01a272ad536ef9763e57d3dbdbd44a8675a515f1306284b60`。此值补足 dirty tree 不能仅用 HEAD 识别的问题；不含预设生成的 define.h/Makeoptions。详见本地 `final-source-evidence.json`。
- 独立 code-reviewer 对 native helper、3个调用程序、Makefile依赖和动态回归做了只读复核；后续A31启动修复又单独复核，无剩余确认阻断项。实际C/Fortran warning编译与动态helper测试通过。已知Windows、事务和跨卷限制仍保留。
- 最终构建使用原构建脚本的本地审计副本：只保留临时BUILD目录，并将postprocess.x加入同一make目标；未更改产品构建行为。第一次普通脚本退出时按设计删除BUILD，导致后续cd失败；改用保留副本后重新完整构建，不将dry-run当链接通过。

- 最新原生后处理2-rank实测通过namelist广播并到达预期“no shards found”错误，未卡在启动collective（`postprocess-mpi-startup-final.log`）；这是失败路径/启动验证，不是凭空制造shard合并成功。实际serial与spatial postprocess所有目标链接成功。
- 收尾检查：两workspace fmt/Clippy、455核心+160 CLI+159 GUI测试、31个报告问题类别对应的定向回归、真实输入、drift/IPC/parameter-audit、26受控vendor检查和最终同源算例均已完成；没有待执行的代码修复或测试任务。审查结束时尚未提交；后续 Git 提交与 PR 不代表已合并或发布。

## 5. 剩余风险与明确不做的事

1. **旧 golden 来源不可比（S2，未修数值）**：需要模型维护者确定期望来源及科学变化，再批准新的基线；本次保留原基线，并提供同源回归，不能猜测某份数值才正确。
2. **旧发布二进制未更新**：源码修复不改变现有 `kernels/*`；因此 CLI 对旧内核的空格限制没有贸然撤除。发布应重新构建并记录 dirty-tree/源码内容身份后再开放路径承诺。
3. **LULCC 条件风险**：空 patch 范围的 min/max 哨兵在不一致外部文件中可复现，但生成的有效 landpatch 不产生该空范围，下游循环也不进入；不作为已证实有效算例崩溃修改。缺失源 class 则已证实可达并加 fail-fast。
4. **Legacy preprocess**：旧构建路径引用不存在模块，HDF5 provider 缺失；已标明不受支持。恢复需要明确数据工具维护范围，不在主产品修复中假造 provider。
5. **支持边界**：自定义输出目录只 fail-fast；Windows Unicode/UNC/长路径、跨卷 move、多节点 MPI、完整多年 BGC/DA/LULCC/URBAN、CaMa 数据链仍需对应平台/场景验收。
6. **文件系统并发/故障**：copy 是有错误反馈的 stream replace，不是事务；copy中磁盘满/崩溃或恶意并发换链仍可能留下部分目标。本次 RegionClip 目标为新目录；未引入自制事务框架。
7. 生产 serial `CoLM_stop` 仍为 bare STOP，可能退出0。helper 探针用 `error stop 1` 仅证明错误分支；实际阶段成功仍要求正向标记、无错误和真实产物，不能只看退出码。

## 6. 文件范围、简化与复现

生产修改集中在各分报告所列 Rust/JS 文件，以及共享 `MOD_Filesystem.F90` / `CoLM_Mkdir.c`、约29个 Fortran 目录调用文件、RegionClip、两个 postprocess、Utils、DA/BGC、LULCC、Lake 与 CaMa 窄边界。新增小型回归脚本/集成测试，无新 package；未改 lockfile、外部输入、golden 或发布内核。

简化：删除重复名称/日期计算入口，删除 native Fortran shell 文件命令，删除 Windows sed 重写和只能观察不能断言的 cmd 探针；复用现有年月末判断、错误终止与阶段判官。不建立多余兼容包装、请求取消层或文件事务框架。

原始日志在本机 `tmp/audit-2026-09-08/`（未入库）。最小复现命令：

```sh
cargo test --workspace --lib --bins --exclude colm-cli
cargo test -p colm-cli --bin colm-cli -- --test-threads=1
cargo test -p colm-cli --test study_cli_tuning
cargo test -p oracle --test judge --test histmap --test golden_run
cargo test -p colm-namelist --test roundtrip
cargo test -p colm-schema --test drift
cargo test -p colm-hist --test drift
cargo test -p xtask --tests
cargo test --manifest-path gui/src-tauri/Cargo.toml --lib
for f in gui/tests/*.mjs; do node "$f" || exit; done
python3 oracle/scripts/test_kernel_filesystem.py
python3 oracle/scripts/test_longitude_normalization.py
python3 oracle/scripts/test_physics_audit.py
python3 oracle/scripts/test_cama_finite.py
python3 oracle/scripts/test_postprocess_startup.py
pytest -q $(git ls-files 'vendor/CoLM202X/tests/test_*.py')
COLM_KERNEL_PROFILE=debug ./oracle/scripts/build_kernel.sh default <new-audit-output>
```

API依据：[GNU Fortran C互操作](https://gcc.gnu.org/onlinedocs/gfortran/Interoperable-Subroutines-and-Functions.html)、[POSIX mkdir](https://pubs.opengroup.org/onlinepubs/9799919799/functions/mkdir.html)、[Microsoft _mkdir](https://learn.microsoft.com/en-us/cpp/c-runtime-library/reference/mkdir-wmkdir?view=msvc-170)、[_stat](https://learn.microsoft.com/en-us/cpp/c-runtime-library/reference/stat-functions?view=msvc-170)、[MoveFileExA](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexa)。C shim保留平台参数类型并传回错误码，Fortran通过BIND(C)和NUL终止字符串调用。

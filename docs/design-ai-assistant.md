# AI 助手设计

> 状态：设计稿（2026-10-07），分支 `colm-agent`。外部接口的事实于 2026-10-07 按官方文档核对，标 **待核实** 的要在装好的版本上实测后再定。

## 0. 目标与原则

在 CoLM-Desktop 里放一个 AI 助手，覆盖四件事：
- 分析结果；
- 操作算例与 Study；
- 阅读、修改 CoLM 源码（Fortran 上游与 Rust 引擎两边都开放），在本机或 T7920 上重新编译、测试；
- 诊断问题。

原则：
1. **不开助手时，应用行为完全不变。** 助手只通过工具调用现有能力，不改引擎的代码路径。
2. **结论要有证据。** 回答里引用的数字必须出自某次工具调用；界面上可以点开原始输出。
3. **写操作一律经人批准。** 改参数、跑算例、打补丁、编译、采纳内核，都要先弹审批卡片。助手永远不能自己采纳或发布。
4. **正式内核与应用本体不可写。** 代码修改只发生在独立的开发工作区，产物登记为"实验内核"，与正式结果分开。
5. **模型与后端可替换。** 默认用 DeepSeek 官方 API；也可以换用 Codex、Claude Code、OpenCode 作后端，同样在应用里驱动。

## 1. 已定的决策（2026-10-07，用户）

| 问题 | 决定 |
|---|---|
| 代码修改在哪里做 | **全部在应用内**。外部编码工具也在应用里驱动、在应用里审批 |
| 开放哪部分源码 | **Fortran（`vendor/CoLM202X`）与 Rust 引擎两边都开放**，改动后强制做对照验证 |
| 在哪里编译 | **本地与远程（T7920）都支持** |
| 默认模型 | **DeepSeek 官方 API** |
| 外部后端 | Codex、Claude Code、OpenCode **能接的都接** |

## 2. 总体结构

```
GUI 助手面板 ──(Tauri 事件)── sidecar.rs ──stdio JSONL── colm-agent（新 sidecar）
                                                          ├─ 后端（四选一）
                                                          │   ├─ 内置：OpenAI 兼容 Chat Completions（默认 DeepSeek）
                                                          │   ├─ Codex：codex app-server（JSON-RPC over stdio）
                                                          │   ├─ Claude Code：claude -p（stream-json）
                                                          │   └─ OpenCode：opencode serve（HTTP + SSE）
                                                          ├─ 工具注册表（A–D 四级，见第 4 节）
                                                          ├─ 审批中心：所有后端的审批请求统一转成 GUI 的审批卡片
                                                          ├─ 会话与审计日志（JSONL）
                                                          └─ colm-mcp：同一套工具的 MCP stdio 服务（供外部后端挂载）
```

- **`colm-agent` 是新 crate**（库加两个可执行文件 `colm-agent` 与 `colm-mcp`），随应用作为 sidecar 分发，与 `colm-cli`、`colm-rs` 放在一起。不依赖 Python。
- **工具只实现一次。** 内置后端直接调用注册表；外部后端经 `colm-mcp` 调同一份注册表。审批、审计、限额都在注册表这一层做，所以不管走哪个后端，规则都一样。
- **工具的实现方式**：绝大多数调用现有 `colm-cli` 子命令，取它的 JSON 输出；代码类工具在工作区里调 git、ripgrep、cargo 和 `build_kernel.sh`，或者调 `run-7920`。
- **GUI 与 `colm-agent` 之间**用 stdio JSONL：
  - GUI 发给 agent：`user_message`、`approval_decision`、`cancel`、`config`；
  - agent 发给 GUI：`assistant_delta`、`reasoning_delta`、`tool_call`、`tool_result`、`approval_request`、`turn_done`、`error`、`usage`。
  - Tauri 侧只做转发和存放 API Key。

## 3. 模型与后端

### 3.1 内置后端（默认 DeepSeek）

按 2026-10-07 的官方文档（api-docs.deepseek.com）：
- 服务地址 `https://api.deepseek.com`，OpenAI 兼容的 `POST /chat/completions`。
- 模型：
  - `deepseek-flash`（V4.1-Flash）与 `deepseek-v4-pro`，上下文均为 1M，都支持工具调用、JSON 输出和流式。
  - 旧名 `deepseek-chat`、`deepseek-reasoner` 已于 2026-07-24 停用，不再使用。
- 思考模式：
  - 默认开启（effort `high`），由 `thinking` 参数控制。
  - **有工具调用时，每一轮都要把之前的 `reasoning_content` 原样回传，否则接口返回 400。** 会话存储必须保留它。
  - 思考模式下不能用 `tool_choice: required` 或指定具体工具。
- 流式工具调用：第一块带 `id`、`type`、`function`，后续块只带参数片段，按 index 拼接。
- 严格模式（beta）：服务地址换成 `/beta`，每个函数都设 `strict: true`，所有属性列入 `required`，并设 `additionalProperties: false`。我们的工具 schema 按严格模式的要求来写，开不开严格模式由配置决定。
- 默认分工：分析与操作用 `deepseek-flash`；读写代码用 `deepseek-v4-pro`。可在设置里改。
- 实现上不绑死 DeepSeek：只要是 OpenAI 兼容的服务都能接，填地址、模型和 Key 即可，包括 Qwen、本地的 Ollama 或 vLLM。

### 3.2 Codex

- 用 `codex app-server`（stdio 上的 JSON-RPC，消息里省略 `"jsonrpc"` 字段）：
  - 流程：`initialize`、`initialized`、`thread/start`（带 `cwd`、`approvalPolicy`、`sandbox`），然后 `turn/start`。
  - 输出：通知 `item/*` 与 `turn/completed` 转成 GUI 事件。
  - 审批：`item/commandExecution/requestApproval`、`item/fileChange/requestApproval`、`item/permissions/requestApproval` 转到审批中心，按你的决定回复。
- 挂载 MCP：`-c 'mcp_servers.colm.command="<colm-mcp 路径>"'`，`default_tools_approval_mode = "prompt"`。
- 沙箱：`workspace-write`，`cwd` 设为开发工作区。
- 接 DeepSeek：`[model_providers.deepseek]`，`base_url = "https://api.deepseek.com/"`，`wire_api = "responses"`（Codex 现在只支持 Responses API），Key 用 `env_key` 传，不用 `experimental_bearer_token`。
- 不用 `codex mcp-server`，它已经取消。
- **待核实**：审批回复的格式。用装好的版本运行 `codex app-server generate-json-schema` 生成后固定下来，并锁定 Codex 版本。

### 3.3 Claude Code

- 启动命令：`claude --bare -p --output-format stream-json --verbose --include-partial-messages --input-format stream-json --mcp-config <json> --strict-mcp-config --permission-prompt-tool mcp__colm__approve`。
  - 进程的工作目录设为开发工作区（Claude Code 没有 `--cwd` 参数）。
- 审批：权限提示转到 `colm-mcp` 的 `approve` 工具。这个工具阻塞等待，直到你在应用里点了批准或拒绝，再返回 allow 或 deny。
- 健康检查：看流开头 `system/init` 里的 `mcp_servers` 状态；会话号从末尾的 `result` 消息取。
- 接 DeepSeek：设 `ANTHROPIC_BASE_URL=https://api.deepseek.com/anthropic` 和 `ANTHROPIC_AUTH_TOKEN`，DeepSeek 文档有配置说明。注意 Anthropic 官方**不支持**让 Claude Code 跑非 Claude 模型，界面上要说明。
- 不用 `bypassPermissions` 或 `dontAsk`。
- **待核实**：`--permission-prompt-tool` 交给工具的参数（推测是 `tool_name` 与 `input`），以及返回值的格式。

### 3.4 OpenCode

- 在本机回环地址启动 `opencode serve`，用 `OPENCODE_SERVER_PASSWORD` 鉴权。
- 配置经 `OPENCODE_CONFIG_CONTENT` 注入：
  - `mcp.colm` 写成 `{type: "local", command: [<colm-mcp>], ...}`；
  - `permission` 里的 `edit`、`bash` 都设为 `ask`；
  - 加上 DeepSeek 服务商。
- 事件流：`GET /event`（SSE）。发消息：`POST /session/:id/prompt_async`。审批：`POST /session/:id/permissions/:pid`，回复 once、always 或 reject。
- **待核实**：权限事件的名称。从 `/doc` 的 OpenAPI 读出来后固定。

### 3.5 后端探测

设置页列出四种后端各自的状态：外部 CLI 是否安装、版本号、是否已登录或已配 Key、能否连上 `colm-mcp`。外部后端各锁定一个已验证的版本范围；超出范围时给出警告，但仍允许使用。

## 4. 工具清单

每个工具都有 JSON schema（按严格模式写），并标明级别。返回值都是 JSON，太长时截断，并附上完整内容的路径。

### A 级：只读，不审批

| 工具 | 作用 | 实现 |
|---|---|---|
| `list_cases` | 项目里的算例：模式、是否空间算例、各阶段状态 | 扫描项目目录，与 GUI 后端 `project.rs` 的 `list_cases` 同一逻辑（抽成共用函数） |
| `read_case_config` | 读 `case.nml`、`forcing.nml`、`history.nml` 中的字段，可按名字或类别过滤 | namelist 解析 |
| `explain_parameter` | 参数的含义、单位、默认值，当前方案下是否生效 | 参数目录 |
| `run_status` | 各阶段的指纹、是否需要重跑、最近一次运行的日志末尾与报错 | `stages.json`、日志 |
| `metrics` | 对观测的指标（RMSE、KGE、偏差……） | `colm-cli metrics` |
| `series_stats` | 时间序列统计：均值、极值、NaN 数、按月或按日聚合 | `colm-cli series` |
| `compare_cases` | 两个算例的配置差异与指标差异 | 组合调用 |
| `study_summary` | Study 的状态、最优成员、参数是否压到边界、校准与验证期对比 | `colm-cli study-status` |
| `hybrid_info` / `hybrid_check` | AI 参数化模型的配置与空跑结果 | 现有命令 |
| `search_docs` | 搜项目文档（设计文档、实现记录、`upstream-bugs.md`） | ripgrep |
| `environment_doctor` | 工具链（gfortran、mpif90、netCDF、cargo）、内核、数据盘、远程主机的状态 | 新增 |

### B 级：运行操作，逐次审批

`set_case_fields`、`run_case`（可只跑某一阶段）、`create_study`、`run_study`、`study_control`（暂停、恢复、取消）、`hybrid_install` / `hybrid_remove` / `hybrid_climate`。

审批卡片写明：改哪个算例的哪些字段、旧值与新值；或者要跑的阶段、预计耗时、写到哪里。

### C 级：代码，在开发工作区内进行

| 工具 | 作用 | 审批 |
|---|---|---|
| `workspace_create` / `workspace_list` | 按应用版本对应的 git 标签（或指定分支）建工作区 | 建立时审批 |
| `search_code` | ripgrep，限定在工作区内 | 否 |
| `read_file` | 按行号范围读，单次有行数上限 | 否 |
| `list_symbols` | 列出 Fortran 子程序或模块、Rust 函数 | 否 |
| `apply_patch` | 应用 unified diff；每个补丁自动成为一次 git 提交 | **逐个审批**，卡片里显示彩色 diff |
| `revert` | 撤回到指定提交 | 审批 |
| `build_engine` | 编译 Rust 引擎：`cargo build --release -p colm-runtime -p colm-cli` | 按会话批一次额度 |
| `build_kernel` | 编译 Fortran 内核：`oracle/scripts/build_kernel.sh <preset> <工作区>/kernels` | 同上 |
| `run_tests` | 白名单：指定 crate 的 `cargo test`、oracle 分层检查、`check-gui` | 同上 |
| `run_case_with` | 用工作区编出的引擎或内核，跑一个算例的副本 | 同上 |
| `compare_outputs` | 与基线逐变量对比：逐位相同、在容差内、有差异；标出 NaN 与量级异常 | 否 |
| `parity_check` | 同一个算例分别用 Rust 引擎和 Fortran 内核跑，找出第一个出现差异的变量与时间步 | 按会话额度 |

**内置后端不提供任意 shell。** 所有命令都预先定义好，参数受校验。外部后端自带 shell，由它们自己的沙箱加我们的审批来约束（见第 7 节）。

### D 级：采纳，只能人工操作

- 把实验内核设为默认；
- 把工作区补丁导出为 `.patch` 或推到分支；
- 删除工作区。

这些只在 GUI 的工作区面板里提供按钮，**不存在对应的工具**，模型想调也调不到。

## 5. 开发工作区

### 5.1 布局

```
<工作区根>/<名字>/          默认 ~/CoLM-Workspaces/
  src/                      git 仓库（来自 GitHub，或从本地仓库克隆），分支 ws/<名字>
  kernels/<preset>/         本工作区编出的 Fortran 内核（含 manifest.json）
  bin/                      本工作区编出的 colm-rs / colm-cli
  runs/<编号>/              run_case_with 与对照运行的算例副本和输出
  reports/                  compare_outputs、parity_check、测试的报告
  workspace.json            来源标签、创建时间、编译与测试状态、采纳记录
```

### 5.2 本地编译

- Rust：在 `src/` 下运行 `cargo build --release`，target 放在工作区里。首次全量编译要几分钟，之后增量编译。
- Fortran：用 `build_kernel.sh` 编译，需要 gfortran、MPI、netCDF-Fortran。`environment_doctor` 会先检查这些。
- 编译与测试在受限环境里跑。macOS 用 `sandbox-exec` 配置：断网，只能写工作区和临时目录。Linux 用 bubblewrap。Windows 先不支持沙箱，界面上明确提示。

### 5.3 远程编译与运行（T7920）

- 复用现有的 `~/.local/bin/run-7920`：快照用 `--source-local <工作区>/src`；命令写在 `--` 之后；结果写到 `$OBREF_JOB_RESULTS`，用 `fetch` 取回；按作业号查询 `status` 与 `logs`；取消用 `cancel`。服务器上的沙箱、网络隔离和配额都由它负责。
- **服务器编出的是 Linux 二进制，不能在 Mac 上用。** 所以远程模式把编译、测试、运行、对比整套都放在服务器上做，Mac 端只取回报告、指标与 diff。远程编出的内核不能登记为本机可用的实验内核。
- 远程跑算例需要服务器上有数据。用 `run-7920 check-data` 检查，并用 `--input-remote` 声明服务器上的路径。只在本地才有的数据，按 runner 的规则先问用户，不悄悄上传。
- 远程主机做成可配置项：主机名或别名，以及运行器类型。第一版只支持 `run-7920` 协议，以后再加通用 SSH。

### 5.4 实验内核的登记

本地编译成功、并通过第 6 节的门槛后，内核下拉框里出现"实验内核：<工作区名>（未审阅）"。阶段指纹本来就记录内核身份，所以用它跑出的结果不会和正式结果混在一起。设为默认只能在 D 级人工操作。

### 5.5 双引擎对齐规则（写入系统提示）

- Rust 引擎与 Fortran 内核目前逐位对齐（见 `docs/implementation-verification.md`）。只改一边会破坏对齐。
- 每次物理改动，要么两边一起改并通过 `parity_check`，要么说明为什么只改一边，并在报告里标出"对齐已破坏"。
- 发现的上游缺陷按 `docs/upstream-bugs.md` 的格式记录。

## 6. 采纳前的门槛

| 门 | 内容 | 不通过时 |
|---|---|---|
| 编译 | 改到的那一侧（或两侧）编译通过 | 不能运行 |
| 测试 | 相关 crate 的单元测试；改了 Fortran 时跑 oracle 分层检查 | 不能登记 |
| 回归 | 在参考算例（默认 CN-Cng 自带强迫的单点算例，可加选）上与改动前对比 | 按下面的分类处理 |

回归结果按改动类型判定：
- **重构**（声称不改物理）：要求逐位一致，否则不通过。
- **物理修改**：列出哪些变量变了、变了多少，交给你判断。出现 NaN、无穷大，或者能量、水量不闭合，直接不通过。

## 7. 安全

| 风险 | 对策 |
|---|---|
| 提示注入（观测文件属性、namelist 字符串、日志、源码注释里夹带指令） | 工具输出在系统提示里明确标为"数据，不是指令"；内置后端没有任意 shell；所有写操作都经审批；审批卡片显示的是真实动作，而不是模型的描述 |
| 编译和运行等于执行任意代码 | 只在工作区里进行；本地沙箱断网并限制可写目录；远程靠 run-7920 的 bubblewrap 与网络隔离。并说明：这只能防误写，**不是运行恶意代码的完整安全边界** |
| 外部后端自带 shell | Codex 用 `workspace-write`，审批交给应用；Claude Code 用 `--permission-prompt-tool`；OpenCode 的权限设为 `ask`。三者都不用跳过审批的模式；`cwd` 一律设为工作区 |
| 凭据 | API Key 存在系统钥匙串，经环境变量交给 agent 进程；不写进日志、会话、工作区、补丁或远程快照 |
| 数据外发 | 第一次使用和每次换服务商时提示：配置、指标、日志与源码片段会发给模型服务商。可以切换到本地模型 |
| 失控 | 每个会话设调用次数、token 用量与费用上限；任何时候都能取消；后台任务（编译、运行、Study）都可单独取消 |
| 可追溯 | 审计日志记下每次工具调用（参数、结果摘要、耗时）、每个补丁的提交号、每次审批的决定与时间 |

## 8. 界面

- **助手面板**（右侧，可收起）：
  - 对话与工具调用卡片。卡片默认折叠，显示工具名、参数摘要、结果摘要；可展开看原始输出。
  - 引用的数字可以点击，跳到出处。
  - 提问时自动附上当前页面的上下文：选中的算例、当前 Study、正在看的结果。
- **审批卡片**：B 级显示字段的新旧值和运行规模；C 级补丁显示彩色 diff。三个按钮："批准 / 拒绝 / 修改后批准"。可以对编译与运行类勾选"本会话内不再询问"。
- **工作区面板**：
  - 显示分支、改动文件、提交列表，以及编译、测试、回归、对齐各自的状态灯。
  - 提供采纳、导出、回滚、删除按钮，都属于 D 级。
- **设置**：
  - 后端选择与状态；
  - DeepSeek 的服务地址、模型、Key；
  - 各级别用哪个模型；
  - 会话限额；
  - 工作区根目录；
  - 远程主机。
- 所有新增中文文案都补英文对照，`i18n.mjs` 要通过。

## 9. MCP 服务 `colm-mcp`

- 自行实现精简的 stdio 服务，不引入 `rmcp`。`rmcp` 3.x 要求 Rust 1.88，而本仓库最低支持版本是 1.85.1；何况协议本身很小。
- 同时支持两代协议：
  - 新规范（2026-07-28）：实现 `server/discover`，从每个请求的 `_meta` 读协议版本；
  - 旧规范（2025-11-25 及之前）：回应 `initialize`。
  - 两代都实现 `tools/list` 与 `tools/call`。
  - 外部 CLI 用的是哪一代**待核实**，所以两代都要支持。
- stdout 只输出 MCP 消息，日志写 stderr，stdin 关闭即退出。
- 提供与注册表相同的工具。另有 `approve`，专供 Claude Code 的 `--permission-prompt-tool` 使用。
- 审批的去向：从应用里启动时，经 agent 转到 GUI；被外部单独启动时（例如你在终端用 Claude Code 连它），B 到 C 级工具在终端里询问，或者直接拒绝，可配置。

## 10. 分阶段计划与验收

| 阶段 | 内容 | 验收 |
|---|---|---|
| **P0** | `colm-agent` crate：OpenAI 兼容客户端（流式，按 DeepSeek 规则回传 `reasoning_content`）、对话循环、工具注册表、审批中心、会话与审计日志；A 级工具；GUI 面板与设置（钥匙串） | 用 DeepSeek 完成"这个算例的 GPP 为什么偏低"一类分析，每个结论都有工具出处；不开助手时，全部现有测试与对照运行逐位不变 |
| **P1** | B 级工具与审批卡片 | 由助手改参数、跑算例、建 Study，全程逐次审批，审计日志完整 |
| **P2** | 开发工作区：本地编译 Rust 与 Fortran、C 级工具、沙箱、回归与 `parity_check`、实验内核登记 | 演示一次完整的小改动：助手改一处 Fortran 加对应的 Rust，两边编译，`parity_check` 逐位对齐，回归报告正确，人工采纳 |
| **P3** | 远程：run-7920 运行器（编译、测试、运行、对比都在服务器上，取回报告） | 同一个改动在 T7920 上完成全套门槛 |
| **P4** | `colm-mcp` 加三个外部后端（Codex app-server、Claude Code、OpenCode serve），审批统一路由到 GUI | 三个后端分别完成一次只读分析和一次补丁审批 |
| **P5** | 诊断套路（闭合检查、参数压边界、NaN 溯源、对齐偏差定位）与报告生成 | 对一组已知问题给出正确定位，例如 LCT 加 VG 未率定（upstream #84） |

## 11. 待核实（P4 前在装好的版本上实测）

- Codex：`--json` 与 app-server 各条事件的字段，审批回复的格式（用 `generate-json-schema` 生成），三种客户端各用哪一代 MCP 协议。
- Claude Code：`--permission-prompt-tool` 的参数与返回格式。
- OpenCode：权限事件名，以及 `run --format json` 的输出格式（以 `/doc` 为准）。
- DeepSeek：旧模型名现在调用会不会直接报错；严格模式对我们这套 schema 是否全部接受。

## 12. 否决的方案

- **只做分析、不碰代码**：不符合用户要的"随时修改、重编译、诊断"。
- **大改动交给外部终端工具**：用户要求全部在应用内完成；外部工具改为在应用里驱动的后端。
- **给内置后端开放任意 shell**：遇到提示注入时风险不可控；改为提供定义好的窄工具。
- **用 `rmcp`**：要提高最低支持的 Rust 版本，而协议本身很小，自行实现即可。
- **Mac 端直接用远程编出的二进制**：平台不同，用不了。
- **用 Python 写 agent**：要用户另配环境，与 sidecar 的分发方式冲突。

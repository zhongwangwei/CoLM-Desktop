# AI 助手设计

> 状态：P0、P1、联网、引导模式、外部后端（Codex、Claude Code，第 634 轮）、远程 R1–R5（第 637、642–646 轮）与 P2 开发工作区（第 647 轮）已完成（2026-10-09）；P3 未做，P5 首批固定诊断快照、版本检索与任务断点已实现，真实科研任务评测延期。设计稿（2026-10-07）。外部接口的事实于 2026-10-07 按官方文档核对，标 **待核实** 的要在装好的版本上实测后再定。

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

2026-10-08 补充（用户）：

| 问题 | 决定 |
|---|---|
| 远程 | **能连任意超算与服务器**，不再绑定 `run-7920`（第 5.3 节）；**先接 T7920** |
| 远程运行的定位 | **GUI 的正式功能**（首页“服务器运行”卡片、运行页选机器），助手调用同一套能力 |
| 远程第一版的引擎 | **Rust 引擎**：在服务器上从源码快照编译（T7920 有 cargo 与离线依赖缓存）；没有 cargo 的机器以后用 CI 预编的 Linux 静态二进制。Fortran 内核的远程编译留给多节点 MPI |
| 助手建算例的方式 | **引导模式**：助手在 GUI 上逐页操作（跳页、填写、按按钮），每页只问需要用户拿主意的项（第 8.1 节） |
| 顺序 | 引导模式 → 远程 R1（T7920）→ P2 开发工作区 |

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

#### 新对话与输入历史（2026-10-10）

应用重启后默认显示新对话欢迎页，旧会话保留在“历史”中，由用户主动选择续聊。欢迎页介绍四个随包示例站点（CN-Cng 草地、AT-Neu 甲烷、AU-Preston 城市、US-Ne3 农田），并提供自有数据建例、运行条件检查、参数与源码解释、已有结果分析入口。点击推荐只填入输入框，不自动运行；示例安装和建例仍走现有引导及操作审批。

输入框 ↑ / ↓ 浏览最近 20 个已保存会话中的最多 100 条用户输入，↓ 越过最新条目恢复未发送草稿。首行以外的普通多行编辑、输入法选词和组合键不被历史操作占用；手动编辑退出历史浏览。历史来自已有会话记录，不复制到 localStorage，删除会话时同步更新。现有会话无逐消息时间戳，跨会话顺序以会话更新时间为准；超长输入跳过而不截断。

#### 多服务设置与原生协议（2026-10-10）

设置中已加入 DeepSeek、OpenAI / ChatGPT API、Anthropic / Claude API、Grok、GLM、Gemini、Kimi、Qwen 和自定义服务。每个服务分别保留地址、模型、思考选项、超时、输出上限及高级参数。常规模式只显示接入方式、服务商、模型、Key、审批和联网选项；地址及高级参数、技术说明放在顶部现有“专家”模式。自定义服务的必填地址在常规模式仍显示；预设地址被修改时显示实际目标地址。切换模式不改变配置，隐藏字段照常保留和保存。预设模型只是起点，刷新模型清单和手动输入均不会自动替换当前模型。模型权限、区域和思考档位以当前账户及服务商文档为准。

| 服务 | 接口格式与默认模型 | 思考配置 |
| --- | --- | --- |
| DeepSeek | Chat Completions；`deepseek-flash`，另提供 `deepseek-v4-pro` | 开关及 low / high / max |
| OpenAI | Responses；`gpt-6.1-sol` | low / medium / high / xhigh / max；默认交给模型 |
| Claude | Anthropic Messages；`claude-sonnet-5-5` | adaptive thinking 与 `output_config.effort`；不发送通用 `reasoning_effort` |
| Grok | Chat Completions；`grok-4.7` | low / medium / high / xhigh；较旧模型档位不同 |
| GLM | Chat Completions；`glm-5.3` | 5.3 使用 low / high / max，不能关闭；较旧型号使用开关 |
| Gemini | OpenAI 兼容接口；`gemini-3.8-flash` | 默认及模型支持的 reasoning effort；原生 thinking 参数可在高级 JSON 中填写 |
| Kimi | Chat Completions；`kimi-k3` | K3 仅 low / high / max；K2.7 Code 始终思考；K2.6 使用开关 |
| Qwen | Chat Completions；`qwen3.8-flash` | `enable_thinking` 开关；`thinking_budget` 可通过高级 JSON 设置 |

OpenAI 原生 Responses 保存并回传完整输出及加密 reasoning；Claude 原生 Messages 保留 thinking / signature / redacted thinking 和 tool use；Gemini 保留工具调用的 thought signature。状态绑定服务商、接口、地址及模型，切换服务时不会回传其他模型的私有状态。流式错误、截断或不完整工具参数阻止执行工具；拒答显示实际文本。

模型刷新通过 sidecar 使用已保存 Key，窗口仅接收模型 ID。通用兼容接口读取 `/models`；Qwen 官方服务读取服务根路径的 `/api/v1/models`，分页最多 500 个模型；GLM 未确认通用列表接口，失败后继续使用预设或手动输入。HTTP 重定向不自动跟随，防止 `x-api-key` 被转发给另一个服务。高级 JSON 是原始 HTTP 请求体附加字段，禁止覆盖模型、历史、工具、流式控制、Responses 状态及凭据。

订阅接入继续使用本机已登录的官方 Codex / Claude Code；API 使用平台账户额度。其他聊天会员或受工具限制的 Coding Plan 不应直接宣传为 CoLM 自定义助手可用的 API 订阅。国内/国际/区域端点可编辑，必须与 API Key 的所属平台和区域匹配。

官方合同核对于 2026-10-10：[OpenAI reasoning](https://developers.openai.com/api/docs/guides/reasoning)、[Claude Messages](https://platform.claude.com/docs/en/api/messages/create)、[Claude effort](https://platform.claude.com/docs/en/build-with-claude/effort)、[Grok reasoning](https://docs.x.ai/developers/model-capabilities/text/reasoning)、[Gemini OpenAI 兼容](https://ai.google.dev/gemini-api/docs/openai)、[DeepSeek thinking](https://api-docs.deepseek.com/guides/thinking_mode/)、[GLM 5.3](https://docs.z.ai/guides/llm/glm-5.3)、[Kimi thinking](https://platform.kimi.ai/docs/guide/use-thinking-models)、[Qwen 模型列表](https://help.aliyun.com/en/model-studio/list-models)。本次验证使用本地模拟 HTTP 和持久化回合测试，没有消费真实账户额度；发版前仍需真实服务与安装包实测。

### 3.2 Codex（第 634 轮已实现并实测，codex-cli 0.160.1，ChatGPT 登录）

- 一个 CoLM 会话常驻一个 `codex app-server`（stdio 上的 JSON-RPC，消息不带 `jsonrpc`）：`initialize`、`initialized`、`thread/start`（或续接时 `thread/resume`），每轮 `turn/start`，取消用 `turn/interrupt`。
- 线程设置：`cwd` 为项目目录，`sandbox = "read-only"`，`approvalPolicy = "on-request"`，CoLM 的领域规则放在 `developerInstructions`。Codex 想写文件或跑越权命令都要申请。
- 事件：`item/agentMessage/delta` 是回答；`item/reasoning/*Delta` 是思考；`commandExecution`、`fileChange`、`webSearch` 等条目出工具卡片；`turn/completed` 结束一轮。`thread/tokenUsage/updated` 是线程累计值，每轮用差值。
- 审批：`item/commandExecution/requestApproval`、`item/fileChange/requestApproval`、`item/permissions/requestApproval` 转成面板审批卡片，回答 `accept` / `acceptForSession` / `decline`。调 MCP 工具前 Codex 会发 `mcpServer/elicitation/request`（`_meta.codex_approval_kind = "mcp_tool_call"`）：来自 colm 的自动同意（转发层已按我们的规则把关），其他的拒绝。
- 挂载 `colm-mcp`：`-c mcp_servers.colm.command=…`，转发地址与令牌经 app-server 的环境变量和 `mcp_servers.colm.env_vars` 交给它，不出现在命令行上。
- 联网（第 635 轮）：用 Codex 自带的搜索，计入 ChatGPT 订阅；设置里开着就 `-c web_search="live"`，关着就 `"disabled"`。不再挂 DeepSeek 的 `web_search` / `fetch_url`。
- 没用实验性的客户端工具（`dynamicTools`）：要开实验开关，版本一变就可能失效。
- 条款：OpenAI 的说法是本地或开源应用沿用 app-server 认证可以继续，但商业或托管服务从来不允许；推荐的正规路线是 Sign in with ChatGPT（要先申请客户端 ID），留待以后。

### 3.3 Claude Code（第 634 轮已实现并实测，2.1.293，Max 订阅）

- 每轮启动一次 `claude -p --output-format stream-json --verbose --include-partial-messages`，消息经 stdin 交过去；第一轮 `--session-id <uuid>`，之后 `--resume <uuid>`。进程的工作目录是项目目录。
- 用订阅登录：不加 `--bare`（它不读订阅登录），并从子进程环境里去掉 `ANTHROPIC_API_KEY`、`ANTHROPIC_AUTH_TOKEN`、`ANTHROPIC_BASE_URL` 等（它们优先于订阅登录）。实测 `apiKeySource: none`。
- 审批：用户的全局设置可能开了 `auto` 模式或放行规则（实测就是），所以显式 `--permission-mode manual --permission-prompts host --setting-sources project`，并 `--permission-prompt-tool mcp__colm__approve`。审批工具收到 `{tool_name, input, tool_use_id}`，回答 `{"behavior":"allow","updatedInput":…}` 或 `{"behavior":"deny","message":…}`。`mcp__colm__*` 直接放行（转发层已把关）。
- 联网（第 635 轮）：用 Claude Code 自带的 WebSearch / WebFetch，计入 Claude 订阅；设置里开着就 `--allowedTools WebSearch WebFetch`（不逐次审批），关着就 `--disallowedTools`。不再挂 DeepSeek 的 `web_search` / `fetch_url`。
- MCP 配置（含转发令牌）写进只有当前用户可读的临时文件，这一轮结束就删。
- 事件：`stream_event` 里的 `text_delta`/`thinking_delta` 是回答与思考；`assistant` 里的 `tool_use` 出工具卡片（`mcp__colm__*` 与 `ToolSearch` 不出）；`user` 里的 `tool_result` 是结果；`result` 结束一轮并带用量。
- 条款：Anthropic 不允许第三方应用提供 Claude.ai 登录、代用户借订阅凭据发请求，或收集、转手凭据；但不妨碍用户用自己的订阅登录未经修改的官方 Claude Code。我们只启动用户自己装的官方程序，不碰凭据，不做登录按钮。把带这个功能的版本发给其他用户算不算“第三方提供”，文档没说清，正式发版前要向 Anthropic 确认。

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
| `search_code` / `read_file` / `list_symbols`（不给工作区名） | 读**应用自己正在运行的那份源码**：Fortran 上游、Rust 引擎、GUI、文档（第 651 轮） | `colm-cli ws-* --source app`：开发环境里是仓库，安装包里把随附的 `colm-src.tar.gz` 解到缓存目录 |
| `environment_doctor` | 工具链（gfortran、mpif90、netCDF、cargo）、内核、数据盘、远程主机的状态 | 新增 |

### B 级：运行操作，逐次审批

`set_case_fields`、`run_case`（可只跑某一阶段）、`create_study`、`run_study`、`study_control`（暂停、恢复、取消）、`hybrid_install` / `hybrid_remove` / `hybrid_climate`。

审批卡片写明：改哪个算例的哪些字段、旧值与新值；或者要跑的阶段、预计耗时、写到哪里。

### 基础文件能力（内置 API 后端与外部后端共用）

| 工具 | 作用 | 级别 |
|---|---|---|
| `path_info` / `list_directory` | 检查路径、列目录（不递归，最多 200 条，扫描最多 10,000 条） | A |
| `read_text_file` | 读取不超过 64 KiB 的 UTF-8 文本，超出返回长度时明确标记截断 | A |
| `create_directory` | 创建目录及父目录；已有目录返回成功 | B |
| `copy_file` | 复制不超过 64 MiB 的文件到新的数据或报告文件 | B |
| `write_text_file` | 创建不超过 64 KiB 的文本报告或数据文件 | B |

范围是窗口选定的项目目录（未开项目时为选中算例的上一级）；未选目录时拒绝访问，不回退到用户主目录。切换项目或内核后，发送前刷新授权。相对路径从该目录开始，绝对路径也必须在目录内；拒绝 `..`、越界或失效的符号链接，以及凭据路径。复制和写入使用独占创建，不覆盖；写入目标仅允许 `.txt/.md/.csv/.tsv/.json`，复制另允许 `.nc/.dat`，源码目录、工作区元数据与 oracle/golden 结果受保护。写操作沿用现有审批策略，工具结果明确报告实际路径与完成情况。

`trash_path` 将符合这些保护规则的文件/目录移动到项目的 `.colm-trash/<id>/payload`，记录原始相对路径与时间；`list_trash` 查看，`restore_trash` 按 id 恢复且不覆盖，原父目录必须存在。移动使用系统原子拒绝覆盖操作，目标被其他运行任务重新创建时也拒绝覆盖；不支持的系统或文件系统返回错误。删除与恢复每次确认，自动执行与会话放行不能绕过。不永久删除、不自动清理、不跨盘复制删除，不递归处理符号链接或包含受保护内容的目录。回收区仍占空间，只作用于助手进程所在机器的授权目录，不自动获得服务器 SSH 删除权限。

这些检查约束共享文件工具，不替代外部编码后端自己的沙箱。并发本地进程替换路径仍受标准库路径检查与打开之间的竞态限制。算例和开发工作区继续用已有创建工具，它们本身会建目录；源码修改继续走工作区补丁工具。

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
| `run_tests` | 白名单：指定 crate 的 `cargo test`（`--lib --bins`）、漂移检查 `drift`（`crates/*/tests/drift*.rs` 全部：从 Fortran 生成的 Rust 表逐字节比对，第 652 轮）、oracle 分层检查、`check-gui` | 同上 |
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

### 5.3 远程计算（任意服务器与超算；2026-10-08 重新设计）

远程是 GUI 的正式功能，不只给助手用。

- **连接**：调系统的 `ssh`，不自己实现协议。复用用户 `~/.ssh/config` 里的别名、ProxyJump、密钥与 ssh-agent；应用不保存密码或私钥。要 VPN 或动态口令的超算，由用户在终端登录一次，应用复用 ControlMaster 主连接；断开时提示重新登录，从不代输口令。主机密钥严格校验，不自动接受。传文件用 `tar` 经 ssh 管道，不依赖 rsync。Windows 自带 OpenSSH 没有 ControlMaster，要动态口令的机器在 Windows 上第一版受限。
- **计算资源**：每台机器一份配置，含 SSH 别名、远程工作根目录、可选的环境准备脚本（`module load`、conda）。“测试连接”探测系统与架构、调度系统（Slurm / PBS / LSF / 无）、编译器、MPI、netCDF、cargo、磁盘配额，以及数据路径。
- **作业**：调度系统做成适配器。无调度时用 `setsid` 后台运行；Slurm 用 `sbatch`/`squeue`/`sacct`/`scancel`；PBS 用 `qsub`/`qstat`/`qdel`；LSF 用 `bsub`/`bjobs`/`bkill`。作业脚本由模板生成，提交前给用户看全文。作业记录存在本机，关掉应用也能接着跟踪。
- **程序**：第一版用 Rust 引擎。源码快照按内容哈希放到远程，在服务器上 `cargo build --release`，不覆盖任何已有目录；netCDF 与 HDF5 已经静态编进 `colm-cli`，服务器上不需要这些库。没有 cargo 的机器，以后改用 CI 预编的 Linux 静态二进制（x86_64 与 aarch64）。Fortran 内核的远程编译留到多节点 MPI 阶段；“仅 Fortran 支持”的配置在远程第一版里明确提示不可用。
- **数据**：每台机器配置自己的数据路径，提交前检查齐全。只在本机才有的数据先问用户，说明大小与去处，不悄悄上传。
- **结果**：前处理、运行、指标都在远程完成，只取回报告、指标与小文件；大的历史输出留在远程，按变量或时段按需取回。
- **服务器编出的是 Linux 二进制，不能在本机用。** 远程编出的程序不能登记为本机可用的实验内核。
- `run-7920` 保留为 T7920 上可选的执行方式（它自带沙箱与配额）。
- **R1 的实现（第 637 轮）**：`colm-remote` crate（ssh、探测、后台作业、引擎快照）加 `colm-cli remote-probe / remote-run / remote-status / remote-cancel / remote-fetch`；GUI 的“运行位置”与服务器对话框调它们。
  - 源码快照包含整个 workspace 和 `vendor/`（编译期会 `include_str!` 读入 `vendor/CoLM202X` 的文件），按内容算标识，在服务器上编一次就一直用，首次约 4 分钟。
  - 引擎与前处理都用 Rust 时 `colm-cli run` 只读内核清单（`Kernel::open_manifest`），服务器上不需要 Fortran 内核。
  - 远程结果与本机不逐位一致（平台数学库末位不同），差异远小于模型误差，见第 637 轮。

### 5.4 实验内核的登记

本地编译成功、并通过第 6 节的门槛后，内核下拉框里出现"实验内核：<工作区名>（未审阅）"。阶段指纹本来就记录内核身份，所以用它跑出的结果不会和正式结果混在一起。设为默认只能在 D 级人工操作。

### 5.5 双引擎对齐规则（写入系统提示）

- Rust 引擎与 Fortran 内核目前逐位对齐（见 `docs/implementation-verification.md`）。只改一边会破坏对齐。
- 每次物理改动，要么两边一起改并通过 `parity_check`，要么说明为什么只改一边，并在报告里标出"对齐已破坏"。
- 发现的上游缺陷按 `docs/upstream-bugs.md` 的格式记录。

### 5.6 P2 的实现（第 647 轮）

- **`colm-workspace` crate**：布局与生命周期（`layout`）、git（`git`）、补丁与撤回（`patch`）、搜索与读文件与列符号（`code`）、沙箱（`sandbox`）、编译（`build`）、测试白名单（`testrun`）、逐变量对比（`compare`）、对齐检查与回归（`parity`）、四道门的状态（`gates`）、实验内核登记（`kernels`）。`colm-cli ws-*` 是它的命令行；`colm-agent` 的 C 级工具、GUI 的工作区面板都只调 `colm-cli`。
- **来源**：本地 git 仓库（`git clone --local`）、仓库地址，或应用随附的 `colm-src.tar.gz`（没有历史，建一个基线提交）。分支 `ws/<名字>`；每个补丁一次提交；`target/` 放在 `src/` 外，仓库保持干净。
- **补丁的限制**：只能改 `src/` 里的普通文件；不许碰 `.git` 与 `oracle/golden/`（改了黄金等于改答案）；不许二进制补丁；512 KB 上限；应用前先 `git apply --check`，不能应用就一个字节都不改；工作区里有未提交的改动时拒绝。
- **工具与审批**：读（`workspace_list`、`workspace_status`、`search_code`、`read_file`、`list_symbols`、`compare_outputs`）不审批；`workspace_create`、`apply_patch`、`revert` **每次都问**，审批者选了“本会话都允许”也不记（`Tool::session_allowance`）；`build_engine`、`build_kernel`、`run_tests`、`run_case_with`、`parity_check`、`regression_check` 按会话批一次。补丁的审批卡片里就是补丁本身，不是模型对它的描述。
- **沙箱**：macOS 用 `sandbox-exec`（断网，只能写工作区、临时目录与 `CARGO_HOME`——cargo 即使 `--offline` 也要写锁文件）；Linux 用 `bwrap`（装了才用）；其余平台不套沙箱，报告里写 `none` 并说明。命令一律 `cargo … --offline --locked`，依赖要事先在本机缓存里。
- **“两版一致”按改动决定要不要做**（第 654 轮）：只有改了会影响计算结果的路径（`vendor/CoLM202X/`、`build_kernel.sh`、Rust 引擎的计算 crate、`Cargo.lock`/`Cargo.toml`，见 `gates::RESULT_PATHS`）才需要；只改 `colm-hybrid`、界面、助手、工作区、远程、命令行外壳、文档或测试时，灯显示“不需要”（在当前提交上真做过就照实显示）。它本来就不是登记实验内核的前提。
- **四道门都记“在哪个提交上测的”**：之后又有新提交就变成“过期”。编译与测试在当前提交上通过、回归没有判不通过，才登记实验内核；内核下拉框用的是界面里的匹配表，实验内核**只有人采纳之后**才进入匹配（排在同名正式内核前面），否则同名预设会悄悄抢走正式内核。
- **回归的判定**：重构要求逐位一致；物理修改只要求没有新的 NaN/无穷大、水量与能量闭合诊断（`f_xerr`、`f_zerr`）不超过基线的十倍（不低于 1e-6）。两个闭合诊断本身是 1e-16 到 1e-10 量级的舍入残差，不放进“变化最大的变量”。
- **D 级只在界面上**：采纳（设为默认）、导出补丁、回滚、删除是工作区面板里的按钮，对应 Tauri 命令 `workspace_adopt/export/revert/delete`；助手的工具注册表里没有这些名字，测试里断言它们不存在。

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
| 凭据 | API Key 以明文存在应用配置目录的 `assistant-keys.json`（三个平台一样；Unix 上权限 0600，只有当前用户可读写）。用户 2026-10-08 决定不用系统钥匙串：要跨平台、要简单，而 macOS 钥匙串对每个新编译的程序都要弹授权框。Key 只在 agent 进程里读出（每个进程读一次），不写进日志、会话、工作区、补丁或远程快照 |
| 数据外发 | 第一次使用和每次换服务商时提示：配置、指标、日志与源码片段会发给模型服务商。可以切换到本地模型 |
| 失控 | 每个会话设调用次数、token 用量与费用上限；任何时候都能取消；后台任务（编译、运行、Study）都可单独取消 |
| 可追溯 | 审计日志记下每次工具调用（参数、结果摘要、耗时）、每个补丁的提交号、每次审批的决定与时间 |

## 8. 界面

- **停靠方式**（第 619–620 轮按用户反馈改）：助手是主网格最右侧的一栏，与页面并排，用分隔条调宽。首页（启动页、配置向导）上同样可以打开，首页让出右侧。

- **助手面板**（右侧，可收起）：
  - 对话与工具调用卡片。卡片默认折叠，显示工具名、参数摘要、结果摘要；可展开看原始输出。
  - 引用的数字可以点击，跳到出处。
  - 提问时自动附上当前页面的上下文：选中的算例、当前 Study、正在看的结果。
- **审批卡片**：B 级显示字段的新旧值和运行规模；C 级补丁显示彩色 diff。三个按钮："批准 / 拒绝 / 修改后批准"。可以对编译与运行类勾选"本会话内不再询问"。
- **工作区面板**：
  - 研究页和助手面板均有“开发工作区”入口。面板顶部可直接“新建工作区”，默认使用当前应用源码，填写名称即可创建；其他本地源码及版本为可选项。仓库只复制已提交代码，安装版使用随包源码包。创建中防止重复提交，失败保留输入，成功显示位置并刷新列表。
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

### 8.1 引导模式（2026-10-08）

用户说“用 /Volumes/Data/Data/PLUMBER2s 里的 CA-Qfo 建一个 PC 算例”时，助手不在后台直接调命令，而是在 GUI 上带用户一页一页走：选运行方式与模拟范围、填目录、扫描、选站点、问算例放哪里、确认建例，再逐页过预热、地表、初始场、强迫场，每页只问需要用户拿主意的项，最后问要不要运行。

- 工具（A 级为只读，按钮按其后果分级）：
  - `ui_state`：当前页面与步骤、这一页的字段和当前值、GUI 自己的校验给出的缺项；
  - `ui_go`：跳到某个步骤；
  - `ui_fill`：在当前页填字段，填过的地方高亮；
  - `ui_click`：按页面上的按钮。只读按钮（扫描）直接按；会落盘或开跑的按钮（建算例、运行）按审批设置先问用户。
- 这些工具由 GUI 执行：agent 发出界面请求，GUI 照用户操作的路径改值、派发事件、等结果，再把结果回给 agent。填写与校验用的都是 GUI 自己的逻辑，所以助手建的算例与手动建的完全一样，用户也能随时接手。
- 后台的 `create_case` 等工具保留，供批量或没有界面时使用。

## 9. MCP 服务 `colm-mcp`（第 634 轮已实现）

- 自行实现精简的 stdio 服务，不引入 `rmcp`（它要求更高的 Rust 版本，而协议本身很小）。
- 两代协议都支持（规范里的 dual-era）：
  - 新一代（2026-07-28）：无握手，每个请求在 `_meta` 里带版本；`server/discover` 回 `supportedVersions`、`capabilities`；所有结果带 `resultType: "complete"`，列表结果带 `ttlMs` 与 `cacheScope`，`_meta` 里报出服务端身份；不支持的版本回 `-32022`。Claude Code 2.1.293 用的是这一代，**缺 `resultType`/`ttlMs`/`cacheScope` 时它拿到工具列表也不注册**（实测）。
  - 旧一代：回应 `initialize`。
- 从应用里启动时（外部后端挂载），`colm-mcp` 只做转发：经本机回环 TCP、带一次性令牌交回 `colm-agent`，用和内置后端同一处 `execute_tool` 执行，所以审批、审计、联网、操作窗口都一样。
- 单独启动（`colm-mcp --cli <colm-cli>`，例如在终端的 Claude Code 里挂上）时只提供只读工具，就地执行。
- stdout 只输出 MCP 消息，日志写 stderr，stdin 关闭即退出。

### 8.2 范围与上下文（第 651 轮，2026-10-09 用户）

- **范围**（写进系统提示）：CoLM 与本应用（建例、运行、排错、配置、日志、结果、Study、物理与参数、代码、开发工作区）；与 CoLM 建模相关的陆面科学；**通用编程与数据分析也答**（Fortran、Rust、Python、NetCDF、shell、数值方法、统计）。**闲聊与日常、无关问题不答**：一句话说明能帮什么，给一个贴合当前页面的下一步；问候与道谢只回一行。不另加话题分类器（多一次调用，误拦难调）。
- **讲代码要看代码**：解释 CoLM 或本仓库怎么实现时，用读代码工具与 `search_docs`，引用文件与行号；不凭记忆。读代码工具不建工作区也能用。
- **随消息附上的窗口上下文**（`[Current view in the application]`）：页面、具体步骤（`result-tuning`、`hybrid-process`……）、选中的算例、内核、项目目录；评估与研究页另附已算出的指标（每个变量的 n、NSE、KGE、RMSE、偏差，最多 8 个）、Study 状态与 AI 混合建模的模式。只是摘要，数字仍要用工具确认。
- **每一步的提问建议**：输入框上方几个按钮（`pagePrompts`），点了填进输入框、可改了再发；空状态里原有的三个通用建议不变。

## 10. 分阶段计划与验收

| 阶段 | 内容 | 验收 |
|---|---|---|
| **P0** | `colm-agent` crate：OpenAI 兼容客户端（流式，按 DeepSeek 规则回传 `reasoning_content`）、对话循环、工具注册表、审批中心、会话与审计日志；A 级工具；GUI 面板与设置（本地 Key 文件） | 用 DeepSeek 完成"这个算例的 GPP 为什么偏低"一类分析，每个结论都有工具出处；不开助手时，全部现有测试与对照运行逐位不变 |
| **P1** | B 级工具与审批卡片；首页也能用助手 | 由助手改参数、跑算例、建 Study，全程逐次审批，审计日志完整（第 620 轮已做） |
| **引导模式** | `ui_state`、`ui_go`、`ui_fill`、`ui_click`，GUI 侧执行与回报（第 8.1 节） | 一句话“用某目录里的某站建一个 PC 算例”，助手在 GUI 上走完建例与逐页设置，只在需要拿主意的地方提问 |
| **R1** | 远程连接、探测、无调度运行；在 T7920 上编 Rust 引擎、跑算例、取回报告（第 5.3 节） | 在 GUI 里把一个站点算例放到 T7920 上跑完，结果与本机逐位一致 |
| **R2**（第 643 轮已实现，用模拟的调度命令验证；真机待验） | Slurm/PBS/LSF 适配器 | 各调度系统在一台对应的机器上跑通 |
| **R3**（第 644 轮已实现并实测） | 预编 Linux 静态二进制（x86_64 与 aarch64），没有 cargo 或不能联网的机器直接用 | 在没有 cargo 的机器上跑通一个算例 |
| **R4**（第 645 轮已实现；单节点 4 进程实测，多节点待验） | 远程编 Fortran 内核与多节点 MPI | 一台带 MPI 的机器上完成编译与多进程运行 |
| **R5**（第 646 轮已实现并实测） | 大文件按需取回：按变量或时段取，传输前压缩 | 取回量与耗时随所取变量数下降 |
| **P2**（第 647 轮已实现并实测） | 开发工作区：本地编译 Rust 与 Fortran、C 级工具、沙箱、回归与 `parity_check`、实验内核登记 | 演示一次完整的小改动：助手改一处 Fortran 加对应的 Rust，两边编译，`parity_check` 逐位对齐，回归报告正确，人工采纳 |
| **P3** | 开发工作区的远程版：同一个改动在任一已配置的计算资源上编译、测试、运行、对比 | 同一个改动在 T7920 上完成全套门槛 |
| **P4** | `colm-mcp` 加三个外部后端（Codex app-server、Claude Code、OpenCode serve），审批统一路由到 GUI | 三个后端分别完成一次只读分析和一次补丁审批 |
| **P5 首批已实现** | `diagnostic_plan`：启动失败、闭合、通量偏差、率定、Rust/Fortran 对齐五种只读快照与必查清单；文档上下文与源码版本、参数生效判断；任务检查点与页面入口 | 报告区分已确认事实、待验证原因与最小实验；缺失证据保持未完成，不把快照采集当作因果验证。真实科研任务评测待用户回去后再做 |

任务检查点复用会话的 `audit.jsonl`，用户目标、动作请求及返回结果逐条持久化。历史对话能从审计补回进程退出前尚未写入模型历史的工具卡片；模型与界面显示未确认动作，续接期间写操作重新逐次审批。保存失败时停止后续工具执行并报告核对要求。进度显示检查数据、定位问题、验证假设、生成报告，“已回答”不等于验收通过。详见 [本轮范围与延期记录](assistant-reliability-plan.md) 与 [CoLM 过程知识卡片](colm-process-knowledge.md)。

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
- **远程只支持 `run-7920`**（2026-10-08 否决）：用户要能连任意超算与服务器；`run-7920` 降为 T7920 上可选的执行方式。
- **自己实现 SSH（libssh2 之类）**：无法复用用户的 ssh 配置、跳板机与 ControlMaster，还要自己管凭据。
- **助手只在后台建算例**（2026-10-08 改为引导模式）：用户看不到每一步，GUI 的校验与默认值也可能和后台路径不一致。
- **用 Python 写 agent**：要用户另配环境，与 sidecar 的分发方式冲突。

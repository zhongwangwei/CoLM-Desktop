# Design

## Source of truth
- Status: Active
- Last refreshed: 2026-10-10
- Primary product surfaces: CoLM Desktop 单点算例向导、运行工作台、结果分析、不确定性分析、参数调优
- Evidence reviewed: `docs/design.md`, `docs/design-gui3.md`, `docs/design-gate.md`, `docs/design-prep.md`, `gui/dist/index.html`, `gui/dist/app/results.js`, `gui/dist/app/style.css`

## Brand
- Personality: 可信、克制、科研导向，像有解释能力的实验工作台而不是开发者控制台。
- Trust signals: 明确输入来源、冻结内容、计算规模、当前状态、失败原因和不会被修改的原算例。
- Avoid: 暴露后端术语、同屏堆放所有控制按钮、没有原因的灰色按钮、把有限样本描述成统计置信区间。

## Product goals
- Goals: 让不了解 CoLM 内部目录结构的科研用户也能设计、运行、检查和复现实验。
- Non-goals: 不隐藏科学假设，不替用户猜参数范围，不把参数调优结果自动覆盖原算例。
- Success signals: 用户能回答“当前在哪一步、下一步做什么、点击后会发生什么、是否真的在运行、何时可以看结果”。

## Personas and jobs
- Primary personas: 陆面过程研究者、站点数据使用者、教学演示用户。
- User jobs: 准备站点算例；设计不确定性或调优方案；估算成本；启动模型；发现失败；解释结果；复现实验。
- Key contexts of use: 本地桌面、长时间计算、多站点、计算资源有限、需要保留审计记录。

## Information architecture
- Primary navigation: 基本设定完成后，不确定性分析和参数调优作为独立可选流程。
- 普通批量运行的“批量并行算例数”放在第 4 步“开始运行”旁；它只控制同时启动的独立算例数，不属于网格参数，也不改变单点 CoLM 内核的串行性质。
- Core routes/screens: 方法/目标 → 输出/目标变量 → 参数范围 → 预算确认 → 生成任务 → 运行与监控 → 结果。
- Content hierarchy: 每页先说明“本页做什么/为什么需要”，再显示单一主操作，最后显示次要信息与高级细节。

## Design principles
- Principle 1: 用用户任务命名。中文界面使用“分析任务/调优任务”，`Study` 仅保留在后端和开发者接口。
- Principle 2: 生成与运行分离。生成任务冻结内核、输入和成员清单；运行才启动 `mksrfdata`、`mkinidata`、`colm`。
- Principle 3: 控件跟随状态。只显示当前可执行的暂停、继续、重试、停止等动作；不可用时给出原因，不制造一排无解释灰按钮。
- Principle 4: 状态与日志自动更新。并行数在真正启动任务前设置；“手动刷新”只是恢复/核对手段。
- Principle 5: 科学选项必须说明适用条件和后果。线性表示等绝对差，对数表示等倍数且只接受正边界。
- Principle 6: 普通运行和研究任务的终止操作始终可见；退出应用必须终止本次应用启动并登记的进程树，遗留状态不得继续伪装为正在运行。
- Tradeoffs: 保留现有 7 页与原生 HTML/JS，避免引入新框架；将“开始计算”和监控放在同页以便长任务控制。

## Visual language
- Color: 复用现有主题变量；通过 accent/pass/warn 表达主操作、完成和风险。
- Typography: 复用现有系统字体；主操作标题清晰，说明文字保持短句。
- Spacing/layout rhythm: 复用卡片和 8/10/12/14px 间距；运行页使用一块主启动卡和一块监控卡。
- Shape/radius/elevation: 复用 `--r-sm`, `--r-md`, `--border`, `--elevated`。
- Motion: 不增加装饰动画；进度由文本、数字与实时状态变化表达。
- Imagery/iconography: 不新增图标库；使用现有圆点、勾号和文字状态。

## Components
- Existing components to reuse: `.card`, `.study-guide`, `.study-readiness`, `.study-status-box`, `.result-tools`, `.btn-next`, `.btn-ghost`, `.report-preview`。
- New/changed components: 任务准备说明卡、设计页与运行页同步的并行数、线性/对数选择指南、按状态显示的控制区、默认展开的实时日志、折叠的任务清单/原始状态。
- Variants and states: 未生成、待开始、运行中、已暂停、完成（含部分失败）、已停止、需要检查。
- Token/component ownership: 样式继续由 `gui/dist/app/style.css` 管理，状态规则由 `study-model.js` 的纯函数管理。

## Accessibility
- Target standard: 保持键盘可操作、清晰焦点、语义按钮和可读状态文本。
- Keyboard/focus behavior: 生成成功自动进入运行页；开始计算自动聚焦运行监控流程，不抢占系统焦点。
- Contrast/readability: 禁用态不能是唯一说明；同步提供状态文案或 `title` 原因。
- Screen-reader semantics: 进度、准备状态和运行说明使用 `aria-live="polite"`。
- Reduced motion and sensory considerations: 不依赖动画或颜色单独传递状态。

## Responsive behavior
- Supported breakpoints/devices: 现有桌面宽度及 980px/620px 响应断点。
- Layout adaptations: 窄屏下准备说明卡和控制区改为单列。
- Touch/hover differences: 重要解释写在页面中，不依赖 hover；`title` 只是补充。

## Interaction states
- Loading: 主按钮显示“正在生成…”或“正在启动…”，防止重复提交。
- Empty: 明确提示先生成任务，并指向上一步。
- Error: 保留后端原始错误，同时用中文说明发生在哪个阶段。
- Success: 生成后自动进入运行与监控；完成后开放结果页。
- Disabled: 根据权威任务状态计算；隐藏当前无意义的高级控制，保留原因明确的主按钮。
- Offline/slow network, if applicable: 本地运行不依赖网络；手动刷新从磁盘读取权威 checkpoint。

## Content voice
- Tone: 直接、解释性、避免开发术语。
- Terminology: “分析任务”“调优任务”“基准成员”；首次出现可说明基准成员对应内部 baseline。中文界面不使用 `Study` 作为用户概念。
- Microcopy rules: 按钮使用动词+对象；区分“生成任务”和“开始计算”；暂停说明“不再派发新成员，不强杀正在运行成员”。

## Implementation constraints
- Framework/styling system: Tauri + 原生 HTML/CSS/ES modules；不增加依赖。
- Design-token constraints: 只扩展现有 CSS 变量和组件。
- Performance constraints: WebView 只保留最近 300 条流式事件，持久状态以结构化事件和磁盘 checkpoint 为准。
- Compatibility constraints: 保持 `study_*` Tauri/CLI 接口和磁盘格式不变。
- Test/screenshot expectations: 纯函数覆盖按钮状态矩阵；Node 合约测试覆盖中文术语和步骤位置；构建后实机验证生成→运行→监控→结果。

## Open questions
- [ ] 以后若支持远程/HPC，任务监控是否需要跨设备恢复；不影响当前本地流程。

## Assistant model settings
- Decision (2026-10-10): 接入方式区分 API Key、官方 Codex、官方 Claude Code 和 OpenCode v2；API 服务商提供 DeepSeek、OpenAI、Anthropic、Grok、GLM、Gemini、Kimi、Qwen 与自定义预设。服务地址与模型始终可编辑，刷新模型失败保留当前输入。
- State contract: 服务商独立保存地址、模型、接口格式、思考选择及高级参数；切换时保留草稿，保存后下一条消息生效。未保存的 Key 阻止切换服务商，避免把凭据交给另一个地址。Key 不回显、不进入配置档案或模型列表响应。
- Progressive disclosure: 常规模式显示接入方式、服务商、模型、Key、审批及联网开关；服务地址、接口格式、输出上限、超时、严格工具参数、附加 JSON 和技术说明使用顶部现有“专家”模式。自定义服务在常规模式保留地址输入；预设使用修改过的地址时常显简短目标地址说明。切换模式只改变可见性，保留配置与草稿。设置区内部滚动，展开高级项仍能访问输入区的思考选择。
- Compatibility: 旧设置保留原服务地址和模型；预设是可修改的起点，账号实时模型列表与服务商文档决定可用性。订阅使用现有官方 CLI 登录，不把网页订阅凭据包装成通用 API Key。
- Verification: 纯函数测试覆盖服务商草稿隔离和思考选项；GUI Rust 测试覆盖旧配置、协议参数与输入校验；协议测试覆盖原生工具循环、思考状态持久化、拒答、失败和 Key 重定向保护。付费 API 和完整安装包另行实测。
- OpenCode: 使用 CoLM 持久化独立配置，首次连接模型的官方 CLI 命令在设置页折叠显示；不复制已有凭据。模型清单来自该配置已连接的服务商，必须明确选择 provider/model；思考选项来自该模型的 variants。首次外发确认绑定所选模型，换模型重新确认。发现期间禁止保存，迟到响应不能覆盖新后端草稿；失败可见且可重试。普通模式保留模型刷新，高级说明沿用专家模式；首版原生 Shell 与网页工具关闭，操作走 CoLM MCP 审批。

## Assistant flux diagnostics
- Decision (2026-10-10): 通量偏差流程增加有界、只读的真实 NetCDF 证据计算；显式确认单位、符号、固定时区、QC 与时段，再计算配对指标和 UTC 月份、观测辐射昼夜、累计降水干湿分组。
- Scope: 首版支持单点 CoLM/PLUMBER2，整分钟固定时区、模型 1800/3600 秒、观测 1800 秒，最多 366 天；packed NetCDF 或不支持的时间约定明确报错，缺少条件变量的分组保留缺失。
- Evidence: 已有对照算例可重算指标并核对配置差异、阶段身份和配对支持；方向变化不能自动证明因果。输出与实际可执行、强迫及初始态的完整绑定不足时保持未验证。回答区分事实、假设和最小实验。

## Assistant conversation entry
- Decision (2026-10-10): 应用重启后默认空白新对话，不自动恢复最近会话；历史仍可主动打开续聊。欢迎页先介绍四个真实随包示例站点 CN-Cng、AT-Neu、AU-Preston、US-Ne3，再提供自有数据、运行检查、参数解释和结果分析入口。
- Interaction: 推荐按钮只填入可编辑的问题，不自动发送、安装数据或启动计算。示例引导沿用窗口操作与审批流程，先检查方案、数据和内核条件。
- Input history: 上下键浏览已保存会话中的用户输入，向下越过最新输入恢复未发送草稿。首次从多行编辑进入历史只在首行生效；输入法选词、组合键与选区保留原有行为，手动编辑或新建对话退出历史浏览。
- Storage: 复用最近 20 个会话的用户记录，最多 100 条；不新增持久化副本，删除会话后同步刷新历史输入。无逐消息时间戳，顺序按会话更新时间及内部消息顺序排列。
- Accessibility: 输入区常显快捷键说明并绑定 `aria-describedby`；欢迎按钮复用 `.assistant-chip`，中英文文案覆盖四个示例及任务提示。

## Parameter tuning guided-flow parity
- 参数调优与不确定性分析保持同一 7 页阅读节奏：先说明目标与边界，再设计、准备任务、开始搜索、查看结果。
- 第 1 页必须用字段说明卡解释指标、最少配对、站点方式、spin-up、校准/验证窗口、种群、代数、随机种子、并行数和刷新参数，且每项都有“是什么 / 怎么选 / 选择后果”。
- 参数调优不暴露额外内核运行目录；内核继承基本设定，并在生成调优任务时记录指纹。
- 目标变量页必须解释目标权重和可评估条件；预算页必须显示运行次数公式；结果页必须提醒校准/验证对比与另存保护，最佳方案不能覆盖原算例。

## Development workspace creation
- Decision (2026-10-10): 工作区面板顶部常显“新建工作区”表单，默认当前应用源码，只需填写名称；其他源码与版本折叠为可选项。研究页和助手面板均提供明确的“开发工作区”入口。
- Reuse: 使用已有 `.card`、`.field`、`.browse`、`.run-btn` 和应用内对话框；不增加框架或样式层。
- State contract: 创建中锁定表单和重复提交；失败保留输入并显示错误；成功展示工作区位置并刷新列表。名称有标签与格式提示，结果使用 `aria-live`。
- Source contract: 本地仓库只复制已提交版本，界面明确说明未提交修改不会带入；安装版默认使用随包源码包。新建只创建源码副本，不编译或运行模型。
- Verification: 17 个 GUI 脚本通过，包含表单调用参数、重复提交、失败恢复和成功刷新；5 项工作区生命周期 Rust 测试通过，包含同名并发创建保护。CLI 用小型 Git 仓库和模拟安装包验证实际创建；前后端接口检查和 Tauri 库类型检查通过。完整 Tauri 构建、打包与桌面实测尚未执行。

## Server authentication
- Decision (2026-10-10): 服务器表单常显主机、登录用户名、SSH 端口和认证方式；沿用 SSH 配置保留旧配置兼容性，密钥模式显示本机私钥选择，密码模式显示登录密码。
- Credentials: 密码仅存应用内存，绑定主机、登录用户名和端口，关闭后重新输入；不进入配置、作业记录、命令行和日志。任务查询、取消及取回按作业记录的服务器加载认证。
- Host trust: 不自动接受未知或变更的服务器指纹；第一次连接在终端核对。有口令的私钥由系统密钥代理解锁。

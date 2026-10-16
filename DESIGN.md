# Design

## Source of truth
- Status: Active
- Last refreshed: 2026-10-13
- Primary product surfaces: CoLM Desktop 模型版本库、算例与运行、科学结果、研究、计算资源、历史记录和助手
- Evidence reviewed: `docs/ui-rebuild-plan.md`（第 10 节优先）、`gui/src/App.tsx`、`gui/src/styles.css`、`gui/src/lib/guide.ts`、`docs/design-references/`。当前重构待集中验收，旧轮次不代表新实现通过。

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
- 官方模型启动时载入作为可选来源；项目首页必须由用户明确选择版本，才解锁下方操作。选择在本次软件会话保留，重启重新选择。没有额外版本选择页、顶部流程条或启动分流页。窗口顶框仅保留品牌、AI 助手和语言按钮。
- 模型决定怎么算：冻结源码、参数、过程实现和网络。算例决定现成方案与过程开关、地点、时段、数据及输出。每次运行使用独立副本，绑定完整修订和实际程序、输入、机器身份。
- 项目首页按用户最新示意图：标题“开始你的研究”，三张并排入口卡“新建模拟 / 打开项目 / 直接查看结果”，CN-Cng 内置示例横卡，最近项目列表；目录、模型管理和数据准备不在首页展开成表单。主线：新建/试示例或打开算例 → 检查数据与时段 → 本机或服务器运行 → 内置结果分析。缺少同条件来源结果时提示缺基准，不自动追加计算。
- 左栏按用户 2026-10-12 提供的示意图统一：项目、模拟、结果、研究、历史记录；计算资源和设置固定在底部。统一使用线性图标、青绿色整行选中样式。当前模型、新建算例、算例列表与准备数据收进项目页；主题、专家模式和帮助收进设置。
- 算例：设置、运行记录、结果。版本：源码、参数、物理、网络与修订。研究：率定、不确定性、学习参数、替代过程；验证后的产物存为模型版本。
- 运行按钮默认本机；引擎、线程、阶段等放高级项。专家模式展开高级项，不另建平行页面。
- 历史统一记录有意义的操作和状态；未确认提交显示待核对，先读真实作业状态，不重复提交。
- 助手右侧常驻可开关；操作遵循实际可见字段、动作、审批和等待契约，配置与源码解释绑定所选模型修订，结果解读绑定实际运行出处。

## 算例方案设置补充（2026-10-16）
- 普通算例页通过上方卡片选择 PFT / PC 结构；逐植物类型的光合、光学、碳氮等参数留在模型版本编辑器，作用域跟随明确的结构。PC 索引指群落内 PFT 组分，使用共用类型表；已知非作物内核不列作物类型。
- 碳氮初始场按运行数据目录推导 `cnsteadystate.nc`，同时提供手填和文件选择，保留自定义路径。
- 植被积雪归入水热过程，紧随冠层截留；冠层截留只提供 1 与 8。字段保留具体约束原因，移除逐项重复的“适用性待核对”，后台适用性校验继续生效。

## Design principles
- Principle 1: 用用户任务命名。中文界面使用“分析任务/调优任务”，`Study` 仅保留在后端和开发者接口。
- Principle 2: 生成与运行分离。生成任务冻结内核、输入和成员清单；运行才启动 `mksrfdata`、`mkinidata`、`colm`。
- Principle 3: 控件跟随状态。只显示当前可执行的暂停、继续、重试、停止等动作；不可用时给出原因，不制造一排无解释灰按钮。
- Principle 4: 状态与日志自动更新。并行数在真正启动任务前设置；“手动刷新”只是恢复/核对手段。
- Principle 5: 科学选项必须说明适用条件和后果。线性表示等绝对差，对数表示等倍数且只接受正边界。
- Principle 6: 普通运行和研究任务的终止操作始终可见；退出应用必须终止本次应用启动并登记的进程树，遗留状态不得继续伪装为正在运行。
- Tradeoffs: 按用户授权采用 React/TypeScript/Vite/Tailwind/Radix，沿用 uPlot 与既有科学算法；增加前端构建，但保持原生 IPC、远程权限与科学校验。

## Visual language
- Color: 复用现有主题变量；主色 #0B7F86、浅青绿 #E6F5F3、深蓝正文 #152A43、浅灰背景 #F6F8FA；深色使用深蓝底和浅青绿。通过 accent/pass/warn 表达主操作、完成和风险。
- Typography: 复用现有系统字体；主操作标题清晰，说明文字保持短句。
- Spacing/layout rhythm: 复用卡片和 8/10/12/14px 间距；运行页使用一块主启动卡和一块监控卡。
- Shape/radius/elevation: 复用 `--r-sm`, `--r-md`, `--border`, `--elevated`。
- Motion: 不增加装饰动画；进度由文本、数字与实时状态变化表达。
- Imagery/iconography: 不新增图标库；导航使用本地 SVG 线性图标，状态复用圆点、勾号和文字。

## Components
- 组件复用 `gui/src/components/ui` 的 Button、Card、Field、Input、Select、Notice、Advanced；Radix 处理需要的交互语义，不新增图标库。
- `gui/src/styles.css` 管理青绿/深蓝主题、字号、间距、圆角、边框和焦点。左栏、内容和助手形成稳定布局，页面主操作常见，技术信息按需折叠。
- 每页通过 PageContract 注册实际可见字段、动作和状态；不依赖旧 DOM 控制器。隐藏、禁用和未加载控件不能操作，懒加载页面就绪后才回复导航完成。
- 状态包括加载、空、就绪、运行、暂停、完成、部分失败、取消、待核对；界面操作完成不等同科学计算通过。

## Accessibility
- Target standard: 保持键盘可操作、清晰焦点、语义按钮和可读状态文本。
- Keyboard/focus behavior: 生成成功自动进入运行页；开始计算自动聚焦运行监控流程，不抢占系统焦点。
- Contrast/readability: 禁用态不能是唯一说明；同步提供状态文案或 `title` 原因。
- Screen-reader semantics: 进度、准备状态和运行说明使用 `aria-live="polite"`。
- Reduced motion and sensory considerations: 不依赖动画或颜色单独传递状态。

## Responsive behavior
- Supported breakpoints/devices: 960/1240/1440px 桌面宽度，分别验证中文/英文与浅色/深色；窗口下限 960×640。
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
- 新前端源码在 `gui/src`，Vite 产物在 `gui/build`，Tauri 入口已经改为 build。旧 `gui/dist` 是过渡参考，不是新包入口。
- IPC 合约从已注册 Rust commands 生成；构建检查命令名称、必需参数、额外参数与事件 emitter。语言、主题、路由等状态不能覆盖后端事实。
- 科学结果、率定与 AI 使用内置分析路径。任何排名需要输入、时段、观测与评估条件相同且证据可验证；缺失证据明确不给排名。
- 本机程序、服务器快照和文档与实际冻结源码身份相绑定。运行时覆盖只有兼容已有程序/内核才免编译；编译期开关按匹配内核，网络仅支持 Rust。
- 自动迁移保留原配置、网络和历史结果，不原地删除字段。草稿可改，冻结修订不可覆盖；运行只物化独立副本。
- 大清单与完整出处通过带哈希的文件引用保存；历史不保存密钥、密码、聊天内容或任意 API JSON。对不确定远程操作先核对原主机/根/作业身份。
- 完整验收集中在末尾：前端测试/类型/lint/build、Rust 测试及 Clippy、原生包与真实任务、响应布局与独立代码/架构复核。合并 main 前仍需用户授权。

## 专题记录的适用范围
以下内容记录先前决定与验证。界面结构、入口和构建约束以本文当前架构及 ui-rebuild-plan.md 第 10 节为准；历史验证不能作为新前端已验收的证据。

## Assistant model settings
- Decision (2026-10-10): 接入方式区分 API Key、官方 Codex、官方 Claude Code 和 OpenCode v2；API 服务商提供 DeepSeek、OpenAI、Anthropic、Grok、GLM、Gemini、Kimi、Qwen 与自定义预设。服务地址与模型始终可编辑，刷新模型失败保留当前输入。
- State contract: 服务商独立保存地址、模型、接口格式、思考选择及高级参数；切换时保留草稿，保存后下一条消息生效。未保存的 Key 阻止切换服务商，避免把凭据交给另一个地址。Key 不回显、不进入配置档案或模型列表响应。
- Progressive disclosure: 常规模式显示接入方式、服务商、模型、Key、审批及联网开关；服务地址、接口格式、输出上限、超时、严格工具参数、附加 JSON 和技术说明使用左栏“专家设置”。自定义服务在常规模式保留地址输入；预设使用修改过的地址时常显简短目标地址说明。切换模式只改变可见性，保留配置与草稿。设置区内部滚动，展开高级项仍能访问输入区的思考选择。
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
- Accessibility: 输入区常显快捷键说明并绑定 `aria-describedby`；欢迎按钮复用助手推荐问题按钮，中英文文案覆盖四个示例及任务提示。

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

## Remote development workspaces (P3)
- Each local workspace has a collapsed “Build and verify on a server” panel. It reuses configured server authentication and uploads an immutable source snapshot into an isolated server directory.
- Main action: full verification (build candidate/baseline, tests, regression and parity); build-only and package tests remain explicit alternatives. The case field is an absolute server path; no automatic data upload.
- Jobs survive application restart. Show host, source commit/hash, state, stale evidence, phase/logs, and report location. Refresh live tasks while the panel is open; keep failed jobs and connection errors visible. Cancel and fetch act on the saved job connection.
- Remote reports stay separate from local acceptance lights and kernel adoption. Fetching a failed report means download completed, never verification passed.

## Results overview visual refresh
- Decision (2026-10-12): 先落地一个真实结果总览，建立视觉标准；研究、计算资源和历史记录的导航重组仍为待确认方案，不在此阶段伪装成已实现。
- References: 用户认可的示意图保存于 `docs/design-references/results-workbench.png` 和 `research-resources-history.png`；图内数字、日期与状态是示意数据。
- Hierarchy: 算例/实际时段与运行出处 → 已确认评估现象 → 模拟/观测曲线与真实指标 → 下一步研究操作 → 可展开的运行/研究记录与本次算例索引。原有分析与研究入口保留。
- State contract: 参数编辑 selected 与结果 resultCaseDir 分离；总览出处、事实和图表绑定当前已校验结果算例及图变量，异步切算例/页面/观测设置不能回填旧响应。
- Science: 复用内置 history/metrics/series 及有界缓存，图形最多 2400 点；缺观测仅画真实模拟或显示空态，未计算/无法计算的指标不造数值。低指标不确认为参数或结构原因。
- Components: 复用现有 CSS tokens、case selector、details、uPlot 和图形生命周期；无新框架/图标库。模型线青绿、观测线灰色并带不同线形与明确图例。
- Responsive: 结果主区自适应，rail 220px，助手默认 320px并保留用户保存宽度/开关；960/1240/1440px检查浅深主题、中英文、控件换行和无横向裁切。
- Verification: 有意义的 JS 回归覆盖真实响应展示、空态、metric/plot上下文和迟到响应；实际数据及原生桌面截图核验视觉效果。仅原生 Mac 打包在已授权本机执行，重 Cargo 检查仍走 T7920。
- [ ] User: 确认首个真实结果页后再推广其布局到其他页面。
- [ ] Future history: 确定统一历史事件和会话索引的范围，本阶段不新增历史后端。

## Simplified top bar
- User decision (2026-10-12): 当前顶栏不直观且复杂，去掉重复的版本、位置与引擎下拉；显示设置渐进展开，不把它们堆在顶栏。
- Interaction: 复用运行方式页的实际卡片与检查规则；助手 `ui_state` 列出可见卡片、已选状态和点击动作，仅在该页允许选择。删除旧顶栏字段后拒绝旧 `ui_fill`，不能改隐藏的底层输入。
- Visible context: 顶栏代表当前算例。助手只有在运行方式卡片或运行页确认行实际可见时，才把全局运行配置加入可见快照；研究私有配置与结果实际出处仍独立保留。
- Verification: 检查默认折叠与键盘展开、语言/专家/主题/日志入口、明暗主题和 960/1440px；回归覆盖手动卡片状态、助手卡片点击等待/禁用/隐藏、旧字段拒绝和运行审批。

## Task homepage
- User decision (2026-10-12): 保留浅显的首页，提供“新建算例 / 打开已有算例 / 试用内置示例”；已有工作不必重复走新建向导。
- Reuse: 复用已有算例 profile 恢复、示例安装/扫描/选站、内核匹配和会话初始化；不增加框架、后端命令或全局历史库。下方仅展示上次项目已读取的前六个真实算例，不伪造访问日期。
- New case: 范围、网格、次网格、土壤和物理/示踪方案保留科学约束；可选检查与日志折叠在最后一页。返回首页保留当前任务，打开取消不清空任务，正在处理时锁定重复操作。
- Examples: CN-Cng 草地、AT-Neu 草地甲烷、AU-Preston 城市、US-Ne3 作物使用各自现有配置。先检查内核，再准备示例；不建算例、不开始计算。AT-Neu 的外部过程数据缺口直接说明，不能暗示四例均可无条件运行。
- Assistant: 只暴露当前首页或向导的可见控件；示例写入仍使用 ui_commit 审批，首页不允许操作被覆盖的运行页面。内置提示绑定实际 new-case / show-examples / example:<site name> 动作，异步安装或扫描不得回填到新会话。
- Visual: 沿用浅灰底、青绿主卡片、深蓝文字和双主题，宽屏三入口并排，窄屏竖排并允许纵向滚动，助手宽度/开关保持用户设置。
- Verification: 33 个前端脚本、137 条 IPC 命令与 8 个事件检查；1440 中文浅色、960 英文深色，检查四例、必要配置、可选检查键盘展开与状态保持、工作台交接。独立只读浏览器预览不能代替原生文件选择/写入；Windows 和付费模型未验收。

## Project homepage reference (2026-10-12)
- 最新用户截图取代此前项目页表单布局。首页的三卡、示例横卡与最近项目遵循截图结构、青绿主操作及深蓝文字；数据准备是轻量次要入口，模型版本按用户追加要求置于三张任务卡之前。
- 用户追加：三张任务卡前先展示模型选择（官方版、已有冻结修订、新建修改版）。启动默认载入不代表已选择；选择通过 model_get 读取准确身份，明确选定后才启用新建、打开与结果入口。新建修改版入口在第一阶段可用，进入管理或创建草稿不解锁任务，选择冻结修订后才解锁。
- 打开项目使用原生目录选择和现有 list_cases，随后进入独立的算例选择步骤，选中真实算例后进入模拟；直接查看结果使用既有内置 Results 页面，不开始计算。助手只获得当前步骤的可见动作，算例选择时不暴露首页入口。
- CN-Cng 示例安装和完整输入扫描属于受审批的写动作；确认实际站点、强迫场覆盖与当前版本兼容后，只预填新建配置，不建算例、不启动模拟。
- 最近项目复用 recent.json 的现有键值存储，保留最近八个实际打开目录；不捏造日期、项目名称或运行状态。草地缩略图为本地 SVG 装饰图，不作为站点照片或科学数据。

## Simulation, results, research and history (2026-10-12)
- 模拟：正文内五步“范围与网格 / 模型结构与方案 / 数据与时段 / 预热与输出 / 检查与运行”；已有算例直接进入运行确认，可返回编辑。日期输入保留已有日内秒数和预热，汇总使用实际起止时刻。单点新建日期通常包含结束当天，到强迫场末尾时沿用真实截止。高级字段、空间网格和跨日期变更线范围保留。
- 运行：默认本机、当前修订及兼容内核；实际输入在启动路径检查，读取配置不等于输入通过。记录、日志、停止和服务器核对/取回沿用既有后端，未知提交不得重跑。
- 结果：默认总览，主标题不与算例页重复，运行选择用真实 ID 简写；高级时段、跳过记录及订正默认折叠并从隐藏契约撤下。真实运行出处、单变量曲线、观测入口、偏差/RMSE/完整配对数，再到研究与诊断。模拟青绿，观测灰色虚线；目录、评估、多站、诊断、报告保留次级入口。单算例指标绑定运行与观测条件，不能混用多站比较或抽样点数；无观测不显示已通过验证。
- 研究：首页五张方法卡（参数率定、不确定性、AI 学习参数、AI 替换过程、修改源码），进入方法后展示原工作台；当前研究只显示真实打开的任务，不编造其他任务。返回首页保留任务，忙碌时不得换方法或离页。
- 历史：可见筛选、左列表/右详情；窄正文单栏，表格可横向滚动。原始出处、命令、比较与重跑收进详情高级区。嵌入算例视图的查询/分页/导出始终固定该算例；导出和重跑保留 commit 审批。
- 助手：隐藏步骤与折叠字段撤下契约；页面切换等待真实加载。研究入口和 ui_go 共用核心转场，不递归调用驱动器。


## Reference correction: version cards and result hierarchy (2026-10-12)
- User override: version selection uses the same three-card treatment as New simulation. Existing versions opens a modal; successful exact-revision loading closes it, cancel preserves selection. Background and hidden dialog actions are not exposed to the assistant.
- Result source of truth: the top main window in docs/design-references/results-workbench.png. Header/case period/badges and Compare other runs → evidence banner → model/observation chart with three metrics → one-row next steps. No default run dropdown, tab bar or full observation form above the chart.
- Observation/file/window controls move to Evaluation settings; run selection moves to its own modal; all deeper analyses and exports remain at a quiet footer. Defaults read the real model series automatically, without starting a model run. Unpaired results use honest pending text and metric placeholders; mockup numbers are never used.
- Chart: compact overview suppresses hover value table and dense markers, reuses uPlot and theme tokens; Research and detailed charts keep their existing size/behavior. Native screenshot comparison at the top of the page gates delivery; original source/revision/paired support remains independent of selected model.
- Assistant modal boundary: shared contract aggregation exposes only the active modal, including title, fields, actions, status and execution context. Page navigation requires closing the dialog first. Time-window changes clear sampled series and its binding; observation-only edits preserve the real model preview.
- Verification: 111 frontend regressions, lint, TypeScript, 161 IPC contracts and production build passed. Native Mac bundle rebuilt; exact revision selection/cancel, automatic series, real observation evaluation and detailed-analysis navigation verified. Independent screenshot verdict: Results 93/100, version cards 96/100, PASS. Evidence saved under .omx/verification/ui-rebuild/.

## Compute resources reference correction (2026-10-12)
- User request: follow the compute-resource workspace in docs/design-references/research-resources-history.png. Header and Add server → compact selectable local/server cards → selected connection settings and three check statuses → selected resource tasks. Retain actual configured host names and counts; do not repeat every server's full configuration.
- Basic settings: address, authentication and account stay visible; key/password and existing SSH config remain supported. Data mappings and working root expand on demand; queue, threads, build and maintenance stay in secondary sections. Existing save/discard, cancellation and fetch semantics remain. Choosing a resource here does not choose the next simulation's execution location.
- Evidence: no automatic SSH check, no fabricated readiness or check time. A successful probe proves this connection; available engine snapshots prove file/executable presence; matching paths only prove existence. Source/prebuilt supply mode and local backend reachability do not establish model validation. Check evidence is bound to the actual configuration and timestamp of the current session.
- Assistant: only selected resource and expanded fields/actions are exposed. Passwords remain manual and absent from UI snapshots and saved config. Busy and unsaved drafts protect selection/navigation; status reconciliation never resubmits a task, and only actual finished status plus exit code establishes its outcome.
- Project version choices already use the same three-card grid and component styles as the task buttons; retain that implementation and show the rebuilt application explicitly for review.
- Verified: 123 frontend tests, lint, TypeScript, 161 IPC contracts and production build passed; native bundle rebuilt. Normal desktop width shows the full resource/configuration/check/task hierarchy, including key drafts and actual checks. Independent final visual verdict 93/100 PASS; AI open uses vertical scrolling for task content without horizontal overflow. Evidence saved in .omx/state/ui-rebuild/resources-reference-verdict.json.

## Explicit model choice and separate homepage stages (2026-10-12)
- Latest user override: version cards and task cards must be clearly separate. Group the three model choices inside a tinted bordered panel; follow it with a separate “Then choose an action” heading. Keep the three independent cards and existing-version modal.
- Initial state: no model choice highlight or “Selected” claim. New simulation, Open project, View results, example, recent-project and data-preparation controls stay gray and actually disabled, including their assistant actions.
- Selection: successful exact-revision loading records the choice; modal cancel/failure does not. Keep the choice for this app session when returning home, and require a new choice when the loaded revision differs or the app restarts. Selecting/freezing a revision in the model manager also records the choice; entering the manager or creating a draft does not.
- First-stage escape: New modified model remains available using the loaded model as its parent, so the selection requirement does not prevent creating a new version. Resource/settings navigation and existing scientific/backend contracts remain unchanged.
- Verified: 126 frontend tests, lint, TypeScript, 161 IPC contracts and production build passed. Native Mac package rebuilt in 19.50s; initial disabled state, modal cancel, manager entry without selection, successful official choice and return-home persistence checked in the actual app. Independent screenshot comparison: 95/100 PASS, including assistant open; no pixel-diff measurement. This homepage phase is delivered; broader scientific/server acceptance remains incomplete.

## Five-step simulation setup (2026-10-12)
- Latest user override: expose single site, watershed, region and global as independent option cards, using the approved project-choice visual language. Spatial grid types get a separate three-card group; keep actual bounds, resolution and mesh requirements. Existing case structure is displayed without pretending that switching a card reconstructs its data.
- Flow (2026-10-13 override): Range and grid → Model structure and schemes → Data and period → Spinup and output → Review and run. Land-cover classification and frozen model parameters/physics with a clear adjustment/return entry are in step 2, before data selection. Data sources and initial conditions are in step 3; time step/spinup/output/restart/diagnostics remain in step 4. Keep uncommon fields available in their corresponding expanded group, including non-model fields omitted by earlier filters.
- Dates: start and end form a dedicated two-column row independent of the simulation name. Use explicit YYYY-MM-DD text entry so WebKit's segmented date control does not rewrite partially entered years. Preserve partial edits, reject invalid dates before saving/advancing, and retain existing seconds. Empty spinup cutoff means disabled; a configured cutoff must precede the simulation end.
- Site default (2026-10-13): cards, file picking and assistant filling of a known scanned station share the full matched forcing coverage. Explicit rescanning of the selected station restores the full period for the matched forcing file; a custom forcing path cannot borrow another file's dates. Subsequent manual date edits remain editable. Observation coverage does not shorten simulation defaults, and existing saved cases retain their saved periods.
- Integrity: use existing creation fields and backend validation; apply only user-changed case options, never schema defaults over site-specific inputs. Frozen physics is changed through the existing derive/save/freeze version workflow, with the pending simulation settings restored on return. Hist-variable availability remains tied to a real created case and matching kernel.
- Acceptance: targeted regression for stage gates, partial/2001/leap dates, option visibility, case/model separation and manager round-trip; frontend lint/type/IPC/build plus native five-step screenshots and independent review. No new scientific run is required for this UI change.
- Verified: 148 frontend tests (including 16 actual component/guide regressions), lint, TypeScript, 161 IPC contracts and production build passed; latest Mac GUI bundle rebuilt in 19.24s. Actual 2001 typing, coverage rejection, five-step navigation, version-manager return and DAILY confirmation checked without creating/running a case. Independent native screenshot review 92/100 PASS; long pages use vertical scrolling. Scientific/server and Windows acceptance remain separate.

## Scientific configuration dependency order (2026-10-13)
- User requirement: expose IGBP/USGS and complete model schemes before data-dependent options; keep the five-step flow but move science configuration to step 2. Detailed field ownership and dependencies are recorded in `docs/simulation-configuration-order.md`.
- Classification: IGBP (17) and USGS (24) are compile-time capabilities selected through the existing case creation mode, independently of runtime LCT/PFT/PC. New cases select classification explicitly; existing cases show their actual kernel/creation identity. Installed kernel availability does not prove version or input compatibility.
- Foundation: case setup owns atomic LCT/PFT/PC and land classification; case setup exposes Campbell/vG, BGC, urban and tracer controls before other process options. Read explicit physics and case-scalar overrides; unspecified values belong to the frozen source and remain unconfirmed. Frozen revisions stay immutable. DEF_SUBGRID_SCHEME is a case-owned scheme label; runtime vegetation selection uses the explicit case flags.
- Coverage: reuse all model_fields and parameter_catalog records, including scoped and read-only references. Display scope, units, activation, source and reference-default labels; do not convert blocked datasets or source-derived fields into editable controls.
- Integrity: previews preserve source-case classification, crop input and domain while using the new materialized runtime structure. Local and final remote kernel gates check compile classification/crop separately from actual runtime structure and parameter scopes. Scientific applicability remains pending real checks; reference metadata never proves effective values.
- Code validation: 163 frontend tests, lint/type/IPC/build; 13 model tests and 375 serial CLI tests on T7920; 14 profile and 1 real materialization-preview tests on cached Mac backend; independent code/architecture reviews passed. Parallel CLI has 8 HDF/read-related failures retained in round 685. Linux GUI blocked by missing Wayland development library. Native package exception approval and new UI screenshot validation remain pending; round 683/684 evidence applies only to those packages.

## Conditional scientific configuration parity (2026-10-13)
- User requirement: restore the old runtime parent/child configuration paths, including routing, BGC, tracer families, real process files and PFT/PC scopes. Static field/catalog lists alone do not meet this requirement.
- Model drafts get typed grouped process controls. Explicit parent choices expand child groups immediately; inactive and unconfirmed fields remain discoverable with reasons, existing overrides stay intact when their parent is switched off. Keep the established five-step sequence and ordinary/expert separation.
- Reuse config.rs runtime/PFT/process logic through a read-only model configuration query. Bind queries to the exact frozen revision or draft id/generation/overlay; ignore stale replies. Current catalog defaults and fallback conditions are references, never evidence of a frozen source's effective values. Missing case/kernel/default authority keeps options pending rather than silently hiding them.
- Process files are read from declared, hash-verified frozen/inherited/staged model assets. Editing writes process-file overlay records; it does not write the source case or immutable assets. Only the explicitly active CH4 asset links inundation changes and resets atomically to Dynamic_Wetland; inactive or unknown assets edit their own scope. Preserve array shape and supported legacy methane aliases. Unknown/unsupported fields remain read-only.
- PFT/PC entries keep their typed index and runtime applicability, known crop capabilities, index-specific restrictions and explicit case-wide overrides; case data inputs remain in step 3 and are grouped by process with reasons. Trace definition fields must be an aligned set, not a lone enable switch.
- Validation: parent on/off/unknown, routing+BGC+CH4, isotope/solute/sediment paths, indexed scopes, asset isolation and methane linkage, source-default uncertainty, stale reads, manual/assistant parity. This slice does not imply full scientific or native-package acceptance.

- Latest evidence for this slice: 189 frontend regressions, 162 IPC contracts, lint/typecheck/build; 12 cached native query regressions and independent architectural CLEAR. Browser React fixture layout reviewed 68 → 91 with table-only scrolling at 540px content; these screenshots do not replace native package or scientific validation. Final model/CLI kernel checks run on T7920; terminal results recorded in implementation-verification round 686.


### Four land-cover and vegetation entries (2026-10-13)
- Restore the old IGBP, USGS, PFT and PC entry cards in model step 2. IGBP/USGS represent LCT with a classification; PFT/PC represent vegetation structure and retain a separate IGBP/USGS data-source choice. Reuse the existing choice cards and responsive grid.
- Vegetation scheme selection writes the complete mutually exclusive LCT/PFT/PC controls directly to the case and keeps the selected model revision. Do not silently disable BGC/CROP; check their prerequisites against case-owned schemes.
- Same-structure selection stays on the setup page. Preserve classification, dates, paths and other case edits across the editor round trip. Existing cases save scheme choices directly; data and kernel compatibility remain separate checks.
- Regression evidence and final native screenshots are recorded in implementation-verification round 688; prior round 687 delivered the source-bound Mac package for the conditional options.


### Ordered model foundation and process choices (2026-10-13)
- Fully reviewed the legacy domain/params/kernel/runner/history-variable flow. Four land/vegetation cards are followed by numbered soil, variably saturated, river routing, regional unit routing, LULCC, BGC, urban, crop and tracer cards. Use the same framed choices, selected state, and adjacent prerequisite reasons.
- New and existing simulations enter the same model-draft workflow with a typed requested choice and preserve case inputs. The shared card component owns its visible guide contract; remove parent selectors and duplicate basic fields. Frozen defaults remain unselected/pending when not verified.
- CROP is compiled capability plus explicit planting configuration, not a fabricated boolean. Match domain/classification with a candidate CROP kernel, then require a valid day; do not insert legacy test day120. Preserve original revision and downstream values.
- Trace family cards stage name/type into the existing aligned editor. Preserve saved/custom tracers and optional columns; no invented reference ratios, mixing water or concentrations. Existing asset import and exact process-file scopes provide methane/isotope/solute/sediment parameters; known conflicts block activation, unknowns remain pending.
- Detailed options keep their typed numeric/list editors and real input/output checks. Core runoff, fractionation, methane modes, and stomatal choices use cards; no parallel save protocol or new dependencies. Current source/catalog data are references until version, actual kernel and inputs are checked.
- Acceptance evidence belongs to implementation-verification round689 and .omx/reports/scientific-setup-cards-2026-10-13; layout screenshots do not replace a scientific simulation.

### Case-owned vegetation schemes (2026-10-13 user override)
- IGBP, USGS, PFT and PC all belong to the case. Selection writes case fields directly without deriving, saving or freezing a model revision. Process implementations, parameter overrides, code and AI networks remain model-owned.
- DEF_USE_LCT, DEF_USE_PFT, DEF_USE_PC and DEF_SUBGRID_SCHEME are excluded from model controls and migration overlays. Historical revisions retain their original bytes and fingerprints; effective overlays omit former scheme controls, and derived drafts do not inherit them.
- Prerequisites and PFT/PC parameter scopes use explicit case controls. Missing, conflicting or incompatible controls fail validation rather than silently dropping parameter overrides. Surface data and compiled classification still require independent checks.
- The model editor return action stays at the top and is blocked only by unsaved edits. Saved drafts may remain unfrozen while the simulation continues with its original selected revision.

## 算例与研究的版本边界（2026-10-13）

- 用户决定：没有修改参数或过程实现的配置都属于算例。植被、土壤水力、BGC、河湖、城市、作物、示踪定义及其他现成方案直接保存算例，不派生、保存或冻结模型版本。
- 实际参数值、过程文件内容、源码和 AI 网络在研究中修改，研究成果保存为新版本；模拟页面通过“研究与参数调整”进入这一流程。
- 兼容性从实际算例值、所选版本参数及内核读取。旧版本中的算例控制项保留原始字节及哈希，但不再覆盖算例，派生草稿不继承这些控制项。

## 高光谱与汇流设置补充（2026-10-16）
- 城市高光谱反照率文件属于高光谱初始化输入，不依赖城市过程；未提供默认文件时显示选择提示，支持手填和文件选择，不显示 null。
- SinglePoint 的 LULCC 禁用遵循上游停机约束。启用空间 LULCC 原子设置月尺度/逐年 LAI，关联年份进入 LULCC 设置。
- 汇水区侧向流的邻接表、估算河深、水库方法在通用水热组展示，避免依赖不适用的网格河湖过程入口。

## 土壤表面阻抗（2026-10-16）
- 土壤表面阻抗移到通用“土壤方案”，位于土壤反照率来源前；不再属于土壤水力弹窗。
- LCT（IGBP/USGS）+ van Genuchten 显示模型强制的有效值0并禁用。其他组合显示自定义值或声明默认SL14（1）。自动强制不覆盖持久化自定义值，切回时恢复原值，避免0遗留到PFT/PC。

## 参数归属与唯一入口（2026-10-16）
- 保持五步流程：范围只配置范围/网格与位置；数据与时段保留输入路径、单点专属设置、格式映射、时段/预热及普通初始场；模型结构与方案集中高光谱来源、LAI/植被结构、强迫场处理；输出、重启、诊断在检查与运行。
- 单点专属设置放在数据与时段的“更多配置”，仅单点范围显示，沿用该页保存方式；模型页只保留强迫处理弹窗。
- LAI只在模型的次网格与植被设置编辑，LULCC弹窗说明要求并指向此位置，不提供第二套编辑控件。站点文件只在数据页；已有算例坐标只在范围页。
- 完整参数目录逐项标注设置位置；研究系数、地类/PFT/PC索引、过程文件参数留在模型版本编辑器。自动管理与当前向导不提供独立编辑的字段必须明确说明。

## 土壤与过程弹窗（2026-10-16）
- 土壤水力卡片只选择Campbell/vG并简要说明各自参数来自逐层土壤数据；移除独立设置弹窗。土壤表面阻抗与参数升尺度拟合共用下方土壤方案，升尺度拟合在单点隐藏，不为两种水力方案分别保存副本。
- 过程首次启用成功后打开关联设置；已启用项再次点选直接打开已有设置，不重复保存或重置默认。关闭、失败、锁定、切换后的旧回调不得弹窗。
- 判断有无关联设置时，父过程未启用所致的字段隐藏不能误判成该过程没有设置；弹窗内仍保留真实适用性过滤。

### 自有数据与 VIC 参数文件
- 从内置示例切换到自有数据时，解绑仍由示例填入的站点、强迫、运行目录和自动时段；保留手动覆盖值，不删除数据文件。研究页往返保留数据来源标记。
- VIC 参数入口仅在产流方案 1 下出现，标量文本与空间网格文件按 VIC 参数模式互斥。显示运行目录预设路径并提供恢复预设和文件选择；空值哨兵不作为路径显示。Rust 初始化优先读取显式路径，未指定时沿用既有上游路径规则；Fortran 会覆盖显式路径，因此关联 Fortran 算例时隐藏自选文件入口。


### 算例目录与运行归属
- 项目目录和模拟名称共同确定最终算例目录，创建前显示同一校验函数得到的完整路径；非法名称不显示有效目标。
- 配置、站点副本和生成数据保存在算例目录内。绑定版本的各次运行使用 `runs/<operation-id>/` 独立快照，输出在其 `out/` 下；不递归复制旧运行。历史读取兼容原注册库运行位置，注册库可保留小型索引和共享模型源码缓存。
- 原始气象、地表和显式 Runtime 数据继续只读引用。显式城市 Runtime 缺必要参数表时报出完整路径，不再把内置表写进用户输入目录；未指定 Runtime 时在算例内创建默认表。

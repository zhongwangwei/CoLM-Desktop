# Design

## Source of truth
- Status: Active
- Last refreshed: 2026-08-28
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
- Core routes/screens: 方法/目标 → 输出/目标变量 → 参数范围 → 预算确认 → 生成任务 → 运行与监控 → 结果。
- Content hierarchy: 每页先说明“本页做什么/为什么需要”，再显示单一主操作，最后显示次要信息与高级细节。

## Design principles
- Principle 1: 用用户任务命名。中文界面使用“分析任务/调优任务”，`Study` 仅保留在后端和开发者接口。
- Principle 2: 生成与运行分离。生成任务冻结内核、输入和成员清单；运行才启动 `mksrfdata`、`mkinidata`、`colm`。
- Principle 3: 控件跟随状态。只显示当前可执行的暂停、继续、重试、停止等动作；不可用时给出原因，不制造一排无解释灰按钮。
- Principle 4: 状态与日志自动更新。并行数在真正启动任务前设置；“手动刷新”只是恢复/核对手段。
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
- New/changed components: 任务准备说明卡、带并行数的运行启动区、按状态显示的控制区、默认展开的实时日志、折叠的任务清单/原始状态。
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

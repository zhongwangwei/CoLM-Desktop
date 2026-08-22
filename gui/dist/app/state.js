//! 各模块共享的那点状态。
//!
//! 单独一个模块而不是挂在 `main.js` 上：`main.js` 要 import 其余模块，
//! 其余模块又要读状态 —— 放在 main 里就成了循环依赖。
//! ES module 的循环依赖不报错，只是让某个 import 在运行时变成 `undefined`，
//! 那种故障比编译错误难查得多。

export const state = {
  /** 当前在第几步，见 shell.js 的 STEPS。 */
  step: 'prep',
  /** 当前配置真正有内容的基础 / 过程子步骤。 */
  availableFlows: new Set(['basic-files']),
  /** 原生折叠组默认收起；用户打开后跨重绘保持。 */
  expandedFlows: new Set(),
  /** 这次要跑什么。'site' | 'region' | 'global'，进门向导第 1 页设的。
   *  区域与全球还没有步骤链，现在只可能是 'site'。 */
  domain: null,
  /** 次网格怎么分。'IGBP' | 'USGS' | 'PFT' | 'PC'，进门向导第 2 页设的。
   *
   *  USGS 仍需要单独的编译产物，界面会自动匹配；PFT/PC 是运行时选择。
   *  新建算例时由 `domain.wizardFields()` 落到 case.nml。 */
  subgrid: null,
  /** 五页向导的实际选择；不进 recent，每次启动重新问。 */
  wizard: null,
  cases: [],
  /** 本次向导中由界面新建的算例目录。root 里的历史算例仍用于避开重名，
   *  但不进入列表、批量运行或评估。返回首页开始新任务时清空。 */
  createdCases: new Set(),
  /** 站点文件 -> 本次为它创建的算例目录，防止重复点击又建一份。 */
  createdBySite: new Map(),
  /** 站点库扫描结果，见 colm-cli scan */
  sites: [],
  /** 勾中的站点，批量的作用对象。键是**站点文件的绝对路径**。
   *
   *  不能用站点名：`AU-Preston` 在 PLUMBER2 与 Urban-PLUMBER 里各有一个，
   *  按名字存的话勾一个会连带勾中另一个 —— 而那两个要跑的东西完全不同。 */
  picked: new Set(),
  /** 高亮（而不是勾选）的那一个站点。**只高亮不动文件** ——
   *  建算例是第 2 步“基本设定 / 文件与目录”按下去的事。 */
  pickedSite: null,
  /** 勾选的算例目录。批量运行与批量评估的作用对象。 */
  pickedCases: new Set(),
  /** 算例目录 -> '待运行' | '运行中' | '已完成' | '失败'。
   *  批量跑时事件是全局广播的，靠 payload 里的 `case` 分发到这里。 */
  runState: {},
  /** 算例目录 -> { mksrfdata/mkinidata/colm: begin|ok|failed|skipped }。 */
  runStages: {},
  /** 算例目录 -> 最近一次精确步进。每个站点各画一条进度，不互相覆盖。 */
  runProgress: {},
  /** 算例目录 -> 这个站点自己的 GUI 日志文本。 */
  runLogs: {},
  /** 当前这一轮运行的全部算例与尚未结束的算例。 */
  runTargets: [],
  runningCases: new Set(),
  runFailures: new Set(),
  kernels: [],
  selected: null,
  /** 参数页正在配置哪些算例（目录路径）。**参数改动作用于整批** ——
   *  用户勾了 20 个站点是要配"这一次运行"，不是配其中第一个。
   *  只配第一个的话，另外 19 个会带着未改的配置跑完，而界面上看不出异常。 */
  batch: [],
  /** 这一批里取值不一致的字段名。界面据此在那些行上标出来。 */
  varies: new Set(),
  /** 当前算例 case.nml 的全文。改字段走后端往返，前端不自己拼。 */
  text: '',
  /** schema 全表，启动时取一次 */
  fields: [],
  group: 'nl_colm',
  /** 当前内核下用不上的字段名（Set）。见 config::irrelevant_fields。 */
  irrelevant: new Set(),
  /** 当前内核 + case.nml 的统一字段交互状态。 */
  fieldStates: new Map(),
  /** 专家入口保留给后续内容；当前只显示明确的占位说明。 */
  expert: false,
  /** 结果分析默认收起右侧运行监视器；用户在结果区手动切换后保持到离开结果区。 */
  liveCollapsed: false,
  /** 运行页输出变量的搜索词与「只看已勾选」。 */
  histFilter: '',
  histOnlyOn: false,
  /** 结果工作台的当前站点。与参数页 `selected` 分开，切图不能改变批量编辑目标。 */
  resultCaseDir: null,
  /** 结果总览中主动选入分析范围的站点；空集合表示本次全部已完成算例。 */
  resultSelection: new Set(),
  resultSelectionTouched: false,
  /** 每个算例单独覆盖的观测路径。自动匹配仍由源站点映射提供。 */
  resultObsOverrides: new Map(),
  /** 批量评估的最近结果与失败项，供比较、诊断和报告分栏共享。 */
  resultMetrics: [],
  resultFailures: [],
  /** 用户明确选择的评估内容。首次打开时默认勾选当前可用的全部变量。 */
  evaluationVariables: new Set(),
  evaluationSelectionTouched: false,
  /** 逐站点、逐变量不可评估的结构化原因，不与整个站点失败混在一起。 */
  resultMetricMissing: [],
  /** 结果页筛选与排名设置。 */
  resultCaseSearch: '',
  resultStatusFilter: 'all',
  /** 批量汇总表：看哪个变量、按哪一列排。 */
  summaryVar: null,
  summarySort: 'r2',
};

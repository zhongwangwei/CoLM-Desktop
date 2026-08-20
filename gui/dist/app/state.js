//! 各模块共享的那点状态。
//!
//! 单独一个模块而不是挂在 `main.js` 上：`main.js` 要 import 其余模块，
//! 其余模块又要读状态 —— 放在 main 里就成了循环依赖。
//! ES module 的循环依赖不报错，只是让某个 import 在运行时变成 `undefined`，
//! 那种故障比编译错误难查得多。

export const state = {
  /** 当前在第几步，见 shell.js 的 STEPS。 */
  step: 'prep',
  /** 这次要跑什么。'site' | 'region' | 'global'，进门向导第 1 页设的。
   *  区域与全球还没有步骤链，现在只可能是 'site'。 */
  domain: null,
  // 次网格方案：IGBP / USGS / PFT / PC，进门第 2 页选。
  // 与 `domain` 一样现在零读取点 —— 等宏改造完成后落到 case.nml。
  subgrid: null,
  /** 这次研究什么过程。'default' | 'carbon_nitrogen' | 'urban' | 'custom'，
   *  进门向导第 2 页设的。**只管物理过程（要不要 BGC/CROP/URBAN），
   *  不绑定次网格方案或土壤水力**——那两项是第 3、4 页各自独立选的
   *  （docs/design-gate.md「默认只管物理过程，不绑定 LULC」一节）。
   *  「default」这个名字没问题——预设的语义是「填好第 5 页的初值，
   *  逐页可见、随时能改」，不是「跳过后面几页」，所以它不是「正确
   *  答案」，只是「不知道从哪开始时的起点」（docs/design-gate.md
   *  「预设是填好后面几页，不是跳过后面几页」一节）。除 'default'
   *  外现在选不到 —— CoLM 宏改造（docs/plan-macro-runtime.md）没
   *  完成之前，内核只有这一套，别的档位在 domain.js 里置灰。跟
   *  `domain` 一样零读取点：落到 case.nml 是宏改造完成后的事
   *  （docs/design-gate.md §3），这一步只把状态机立起来。 */
   *  第 5 页（其余物理开关）落地后从这里读起始值。形如
   *  `{ bgc, crop, urban, tracer }`——**不含次网格方案或土壤水力**，
   *  'custom' 时为 null（没有初值，等那几页落地后用户自己填）。跟
  cases: [],
  /** 站点库扫描结果，见 colm-cli scan */
  sites: [],
  /** 勾中的站点，批量的作用对象。键是**站点文件的绝对路径**。
   *
   *  不能用站点名：`AU-Preston` 在 PLUMBER2 与 Urban-PLUMBER 里各有一个，
   *  按名字存的话勾一个会连带勾中另一个 —— 而那两个要跑的东西完全不同。 */
  picked: new Set(),
  /** 高亮（而不是勾选）的那一个站点。**只高亮不动文件** ——
   *  建算例是第 3 步「确定」按下去的事。 */
  pickedSite: null,
  /** 勾选的算例目录。批量运行与批量评估的作用对象。 */
  pickedCases: new Set(),
  /** 算例目录 -> '待运行' | '运行中' | '已完成' | '失败'。
   *  批量跑时事件是全局广播的，靠 payload 里的 `case` 分发到这里。 */
  runState: {},
  /** 本次运行里三段各自的状态，键是 mksrfdata/mkinidata/colm。 */
  stages: {},
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
  /** 专家模式额外显示当前内核下的源码派生只读项。 */
  expert: false,
  /** 运行页输出变量的搜索词与「只看已勾选」。 */
  histFilter: '',
  histOnlyOn: false,
  /** 批量汇总表：看哪个变量、按哪一列排。 */
  summaryVar: null,
  summarySort: 'site',
};

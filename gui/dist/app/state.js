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
  /** 次网格怎么分。'IGBP' | 'USGS' | 'PFT' | 'PC'，进门向导第 2 页设的。
   *
   *  IGBP 与 PFT 已跑通；`LULC_USGS` 的数组尺寸仍由编译期参数
   *  `N_land_classification` 定死，PC 则还没有端到端跑通的算例。
   *  新建算例时由 `domain.wizardFields()` 落到 case.nml。 */
  subgrid: null,
  /** 五页向导的实际选择；不进 recent，每次启动重新问。 */
  wizard: null,
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

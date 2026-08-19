//! 各模块共享的那点状态。
//!
//! 单独一个模块而不是挂在 `main.js` 上：`main.js` 要 import 其余模块，
//! 其余模块又要读状态 —— 放在 main 里就成了循环依赖。
//! ES module 的循环依赖不报错，只是让某个 import 在运行时变成 `undefined`，
//! 那种故障比编译错误难查得多。

export const state = {
  /** 当前在第几步，见 shell.js 的 STEPS。 */
  step: 'data',
  cases: [],
  /** 站点库扫描结果，见 colm-cli scan */
  sites: [],
  /** 勾中的站点，批量的作用对象。键是**站点文件的绝对路径**。
   *
   *  不能用站点名：`AU-Preston` 在 PLUMBER2 与 Urban-PLUMBER 里各有一个，
   *  按名字存的话勾一个会连带勾中另一个 —— 而那两个要跑的东西完全不同。 */
  picked: new Set(),
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
  /** 专家模式：连**这份配置没设过**的字段一起显示（约 202 个顶层字段），
   *  未设的显示 schema 默认值并标灰。普通模式只显示实际设了的。 */
  expert: false,
  /** 是否连**当前内核编不进去**的字段也显示。
   *
   *  与 `expert` 是**两个轴**，不能合成一个开关：前者问「设没设过」，
   *  后者问「编没编进去」。压成一个的话，想看全部已设字段的人会被迫
   *  连 68 个用不上的一起看。 */
  showIrrelevant: false,
  /** 运行页输出变量的搜索词与「只看已勾选」。 */
  histFilter: '',
  histOnlyOn: false,
  /** 批量汇总表：看哪个变量、按哪一列排。 */
  summaryVar: null,
  summarySort: 'site',
};

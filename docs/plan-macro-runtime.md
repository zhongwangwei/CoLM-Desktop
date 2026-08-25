# 把 CoLM 的编译期宏改成运行时开关

**目标：一个二进制覆盖所有单点配置。**

现在每个宏组合要单独编一个内核（17 MB），有效组合几十上百种 ——
随包发不可能全覆盖，让用户自己编又要一整套 Fortran 工具链。

**前提**：`vendor/CoLM202X` 已经从 submodule 改成入库副本
（`acfb596`，见 `vendor/PROVENANCE.md`），改动就是我们自己的文件。

---

## 这件事到此为止（2026-08-21 收口）

**范围以这一节为准**，下面那张「完整清单」是 8-20 立目标时的快照，
不再更新。

**一个二进制现在覆盖这八个维度的任意组合：**

| 维度 | 开关 | 活的 `#ifdef` |
|---|---|---|
| 调试三件套 | `DEF_USE_CoLMDEBUG` / `RangeCheck` / `SrfdataDiag` | 0 |
| 土壤水力二选一 | `DEF_USE_Campbell_SOIL_MODEL`（`.not.` 即 vanGenuchten） | 0 |
| TRACER | `DEF_USE_TRACER` | 0 |
| PFT | `DEF_USE_PFT` | 0 |
| PC | `DEF_USE_PC` | 0 |
| BGC | `DEF_USE_BGC` | 0 |
| 城市 | `DEF_URBAN_RUN` | 1 |
| 土地利用变化 | `DEF_USE_LULCC` | 0 |

城市那 1 处是 `mksrfdata/MOD_LandPatch.F90:265`，`#if (!defined(URBAN_MODEL)
&& !defined(CROP))`，只控制一行 `Total: N patches.` 的打印，且同时依赖
CROP —— 要转它得先转 CROP。不影响任何物理量。

（另有 2 处 `!#if (defined CoLMDEBUG)` 在 `preprocess/rd_land_types.F90`，
是上游本来就注释掉的死代码，不算。）

### 三项不做，各自的理由不一样

**`LULC_IGBP` / `LULC_USGS`（28 / 51 处）与 `CROP`（260 处）** —— 止损，
`c3a8c6e` 记过一次。8-21 复查时试图找便宜路子，**没找到**，把证据留在这里
免得下次再试一遍：

一开始想的是「取并集」：把 `N_PFT+N_CFT` 一律开到最大（15+64=79），
运行时只用前 N 个。这条路走不通，因为那些不是普通数组，是**带逐元素
写死初始化数据的 `parameter` 数组**：

```fortran
integer , parameter :: canlay_p(0:N_PFT+N_CFT-1) &
   = (/ …一长串逐元素的数据… /)
```

不开 CROP 是 16 个元素（16 个 PFT），开了是 79 个（15 PFT + 64 CFT），
而且**两套数据的内容不同**，不是一套截断成另一套。开大尺寸只会让元素
个数对不上；要合并就得把两张表逐元素并成一张，再把 `parameter` 改成
运行时填充 —— 那是 `main/MOD_Const_PFT.F90` 整个文件加 133 处
`N_PFT+N_CFT` 尺寸声明。

`N_land_classification` 同理：定义在
`preprocess/aggregation_landtypes.F90:29,32`，24 类（USGS）与 17 类（IGBP）
各带一套配套数据，194 处依赖，29 个 `.F90` 受影响。

**判据也不一样**，这才是它该独立成一轮的真正原因：前四组的判据①是
「黄金回归 bit-identical」，那只证明**默认配置没变**；而这一轮改的正是
数组尺寸本身，默认配置不变是必然的，证明不了另一套尺寸下算得对。
要验证得另设计一套判据。

留了只读镜像 `DEF_USE_USGS` / `DEF_USE_CROP`（各 9 处），让 GUI 能显示
当前内核是哪一套 —— 不能改，只能读。

**`extend_interception`（4 处）** —— 用户明令不做（见下面那一节）。

### 所以「一个 exe 覆盖所有配置」达成到什么程度

八个维度任意组合：一个二进制。
IGBP/USGS 之间、开不开 CROP：**仍需分别编译**。

---

## 完整清单（实测处数，2026-08-20）

| 组 | 宏 | 处数 | 文件 | 状态 |
|---|---|---|---|---|
| **① 试点** | `CoLMDEBUG` | 92 | 30 | ✅ `acc596a` |
| | `RangeCheck` | 121 | 39 | ✅ 同上 |
| | `SrfdataDiag` | 98 | 20 | ✅ 同上 |
| **② 两套物理方案共存** | `Campbell_SOIL_MODEL` | 63 | 19 | ✅ `80de820` |
| | `vanGenuchten_Mualem_SOIL_MODEL` | 115 | 27 | ✅ 同上 |
| | ~~`extend_interception`~~ | 4 | 1 | ❌ **不做** |
| **③ 最大但独立** | `TRACER` | 342 | 69 | ✅ `1a60e9d` |
| **④ 核心难点（必须一起）** | `LULC_IGBP_PC` | 159 | 40 | ✅ `06543f8` |
| | `LULC_IGBP_PFT` | 150 | 38 | ✅ 同上 |
| | `BGC` | 131 | 73 | ✅ 同上 |
| | `URBAN_MODEL` | 92 | 29 | ✅ 同上 |
| | `LULCC` | 16 | 8 | ✅ 同上 |
| | `LULC_IGBP` | 205 | 46 | ❌ 止损，见下 |
| | `LULC_USGS` | 66 | 21 | ❌ 止损，见下 |
| | `CROP` | 256 | 41 | ❌ 止损，见下 |

**约 1900 处。** `URBAN_LCZ` 是死宏（模板里有，代码里 0 处引用）。

## 为什么是这个顺序

**不能并行** —— 它们改的是同一批 `.F90` 文件，`CoLMDEBUG` 与
`Campbell` 很可能出现在同一个文件里。串行推进。

顺序按**难度递增**，每一组验证一件前一组没验过的事：

| 组 | 验证什么 |
|---|---|
| ① 调试三件套 | 「`#ifdef` → `DEF_USE_*`」这条路本身走得通 |
| ② 土壤水力 | **两套物理方案能共存**（变量集不同：VG 有 `alpha_vgm`/`n_vgm`/`theta_r`，Campbell 没有） |
| ③ TRACER | 大规模改造的机械性（342 处，但独立） |
| ④ LULC 系 | **次网格数据结构可以运行时选**（维度都不同，最难） |

## 每组共同的三条判据

**① 黄金回归逐位不变**

```bash
oracle/scripts/build_kernel.sh default kernels
export PLUMBER2_ROOT=/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s
cargo test -p oracle --test forcing_convert -- --ignored
```

必须是 `identical: 129 variables, 10 dimensions`。

**改开关不该改变默认配置下的任何物理结果。** 红了就是改坏了。

**② 关掉之后确实不走那条路**

不同组的观测点不同：调试组看日志行数，土壤水力组看是否用了另一套
公式，LULC 组看次网格结构。

**③ 打开之后功能回来**

**这条最容易被忽略。** 把 `#ifdef` 删掉、代码再也不执行，也能让
① ② 通过 —— 但功能没了。必须验证开关**双向**都work。

## `extends/` 整个目录不在范围内

```
vendor/CoLM202X/extends/
  ├─ CaMa/            洪水模型 —— 单点下 create_defineh.bash 强制 #undef
  └─ interception/    冠层截留扩展 —— 就是 extend_interception 那个宏的家
```

**它们是扩展模块，不是核心物理。** 改造只动 `main/`、`share/`、
`mksrfdata/`、`mkinidata/` 那几处。

### ⚠️ 上面那句「不在范围内」有一处重要例外

`extends/interception/MOD_Thermal_CanopyPhase_Extended.F90` **必须改**。

理由：`extend_interception` 在 `create_defineh.bash` 的模板里是无条件
`#define`，所以**那个文件就是每个内核实际编译进去的 `MOD_Thermal`**。
不改它，土壤水力的运行时开关在生产配置下是**空操作** ——
第二组判据③（运行时 Campbell 与改造前编译期 Campbell 逐位相同）
直接依赖于这处改动。

**我最初统计出「土壤水力在 extends/ 里 0 处」是错的**，而且错的方式
值得记：**统计时那个 agent 已经改完了那 8 处**，`#ifdef` 已经变成
`IF (DEF_USE_Campbell_SOIL_MODEL)`，所以 grep 自然是 0。

**在一个正在被修改的代码库上统计「还剩多少要改」，得到的数是错的。**
要么先停下所有改动再统计，要么统计一个固定的 commit。

### 修正后的原则

`extends/` 里**只有 CaMa 不在范围内**（单点下强制 `#undef`）。
`extends/interception/` 参与每一次编译，与 `main/` 同等对待。

至于 `extend_interception` **这个宏本身**不做 —— 那是另一回事
（见下一节），与「它目录下的文件要不要跟着别的宏改」无关。

## `extend_interception` 为什么不做

改造第二组时顺手捎带了它，做到一半发现它跟土壤水力那两个**不是一类
东西** —— `MOD_Namelist.F90` 那边的注释写着它「不能变成 body-level
`IF`」，也就是说它影响的不只是「走哪条分支」。

4 处的收益本来就小，而它需要的是另一种改法。**从范围里去掉**，
保持编译期宏原样。

将来真要动它，得先弄清楚那 4 处到底在什么位置上 ——
那是一次独立的调查，不该混在别的组里顺手做。

## ① 试点的结果（`acc596a`）

**三条判据全过**：

```
① 黄金回归    identical: 129 variables, 10 dimensions
② 关掉        colm.log 2138 行，0 条 Check
③ 打开        colm.log 37629 行，33357 条 Check vector/block data
```

**`macros` 字段直接印证了成果**：

```
改造前  [CoLMDEBUG, LULC_IGBP, RangeCheck, SinglePoint, extend_interception, vanGenu...]
改造后  [           LULC_IGBP,             SinglePoint, extend_interception, vanGenu...]
```

两个调试宏**从编译期概念里消失了**。

### 311 处的分类（后续几组照这个走）

| 类 | 处数 | 怎么改 |
|---|---|---|
| 条件 `USE` | 44 | 去掉 `#ifdef`，模块总是编译 |
| 条件 `CALL`/`IF` 块 | 235 | 包进 `IF (DEF_USE_XXX) THEN ... ENDIF` |
| `#ifdef` 下的局部变量声明 | 16 | 总是声明 |
| 整个 `SUBROUTINE`/模块被包 | 11 | 去掉包裹 |
| 空块/死块 | 3 | 删指令 |
| 手工 | 2 | `#ifdef/#else` 的真实/桩函数对 |

### 试点踩过的坑（后续几组照着避开）

1. **`PUBLIC :: xxx` 不能包进 `IF` 块** —— 模块的 specification part
   里 `IF` 不合法。6 处踩过
2. **纯声明块也不能包 `IF`** —— 3 处，特征是里面嵌着别的 `#ifdef`
   把分类搞乱了
3. **46 处调用点要补 `USE MOD_Namelist`** —— 多是
   `USE MOD_Namelist, only: ...`，新符号不在 `only` 列表里
4. **`gui/src-tauri` 不是 cargo workspace 成员** ——
   `cargo test --workspace` 碰不到它，必须单独跑。试点靠单独跑
   才抓到一个真失败
5. **加 `DEF_USE_*` 就要动 schema** ——
   `xtask gen-schema` 重新生成、`config.rs` 给 UI 分区、
   `params.js` 的 `PARAM_SECTIONS` 对上（有测试强制两边一致）

## ③ TRACER 的四个发现（第四组要当心）

前两组没遇到的，都是**隐藏的第二道门**：

**① `vendor/CoLM202X/Makefile` 有自己的编译门**

```makefile
TRACER_ENABLED := $(shell grep '#define TRACER' include/define.h)
```

**目录级的门** —— 决定要不要编 `main/TRACER/*.o`。改了 `.F90` 里的
`#ifdef` 还不够，Makefile 在外面还拦一道。
**第四组要先查 Makefile 里有没有 LULC/BGC/CROP 的同类门。**

**② `xtask/src/usage.rs` 的 `SUBSYSTEMS` 映射**

`main/TRACER/ → "TRACER"` 用来算 schema 的 `requires` 字段。宏消失之后，
GUI 会把所有 `DEF_TRACER_*` 显示成「本内核未编入」—— 因为 `TRACER`
再也不在 `macros` 列表里。

**这条连锁不容易想到**：宏消失 → schema 的 `requires` 失效 → 界面显示错误。
第四组同样要检查 `usage.rs` 里 LULC/BGC 相关的映射。

**③ `/*` 出现在注释里会破坏 cpp**

它在 `create_defineh.bash` 里写了一句含 `main/TRACER/*.F90` 的注释，
`/*` 被当成 C 注释起始，`gfortran -cpp` 报 "unterminated comment"。

**④ near-miss：84 处的大文件差点漏掉**

`MOD_SnowLayersCombineDivide.F90` 分析过但忘了改，靠**最后的全仓库
grep 扫描**才发现。那 84 处漏掉的话，判据 ① ② 可能照样通过
（默认配置下那条路不执行），**但打开开关就是错的**。

**每组收尾前必须全仓库扫一遍**，不能靠「我记得都改了」。

## ④ 的止损：`LULC_USGS` 与 `CROP` 保持编译期宏

**这两个做不成运行时开关，原因是数据结构，不是工作量。**

```fortran
N_land_classification   ← USGS 24 类 vs IGBP 17 类
N_PFT + N_CFT           ← CROP 开/关决定
```

它们是 Fortran `parameter` **常量**，数组尺寸在编译期定死。改成运行时
要么把所有相关数组改成 `allocatable`，要么取并集（按大的分配）——
**那是真正的数据结构重构，不是 body-level `IF` 能解决的**。

**交付**：`LULC_USGS` / `CROP` 保持编译期宏，另外提供只读镜像
`DEF_USE_USGS` / `DEF_USE_IGBP` / `DEF_USE_CROP`，让运行时代码能查询
当前编译的是哪套，但改不了。

**这一组真正做成运行时的是 5 个**：
`LULC_IGBP_PFT` / `LULC_IGBP_PC` / `BGC` / `URBAN_MODEL` / `LULCC`。

### 对「一个 exe 覆盖所有配置」的影响

| 配置 | 一个二进制能覆盖吗 |
|---|---|
| IGBP ↔ PFT ↔ PC | ✅ |
| 开/关 BGC、URBAN、LULCC | ✅ |
| 开/关调试、示踪物、Campbell/VG | ✅（①②③ 组） |
| **IGBP ↔ USGS** | ❌ 仍需两个二进制 |
| **开/关 CROP** | ❌ 仍需两个二进制 |

**从「每个组合一个内核」到「两个二进制覆盖绝大多数组合」** ——
USGS 用得少，CROP 是碳循环的子集，把它们留在编译期的代价可以接受。

真要做，那是独立的一轮：把 `N_land_classification` 与 `N_PFT+N_CFT`
相关的数组改成运行时分配。**那一轮的判据与这四组不同** ——
它动的是内存布局，黄金回归只能证明「默认配置没变」，
证明不了「另一套尺寸下算得对」。

## ④ 完成的结果（`06543f8`）

真正做成运行时的五个（`LULC_IGBP_PC`/`LULC_IGBP_PFT`/`BGC`/`URBAN_MODEL`/
`LULCC`）改完之后，`default`/`bgc`/`urban` 三个内核预设产出**完全相同**
的 `define.h`——过去要三个二进制的组合，现在一个就够，运行时用哪套只看
`case.nml`。

**四类返工，从便宜到贵**（后续如果还有类似改造，照这个顺序排查）：

1. 整个 `MODULE` 被 `#ifdef` 包住——不能包成运行时 `IF`（那是非法
   Fortran），去掉包裹，模块总是编译进去，运行时逻辑挪到调用点。
2. 参数列表/声明列表中间被 `#ifdef` 断开——同样不能包成 `IF`，去掉
   包裹，参数永远在签名里。
3. **混合结构 `#if...THEN <code> #else <code> ENDIF`**：批量转换脚本
   没有特判块内的裸 `#else`，把它整段吞进 `IF...ENDIF`，留下孤立的
   `#else`。这个不只是编译错误——曾经在 `gfortran -cpp -E` 下**静默
   截断**预处理输出（退出码 0，没有 stderr），逐个手工修了约 15 处。
   **教训**：只看 stderr 的批量自检不可信，得核对预处理输出本身的
   完整性（行数、是不是以 `END MODULE`/`END PROGRAM` 收尾）。
4. **「实参无条件传，但源数组只在条件分支里分配」**：`totlitc` 等 BGC
   patch 级状态数组过去只在 `IF (DEF_USE_BGC) THEN CALL
   allocate_BGCTimeVariables ENDIF` 里分配；把 `iniTimeVar` 里它们的
   非 `optional` `intent(out)` 实参改成无条件传之后，`default` 内核
   （`DEF_USE_BGC=.false.`）跑 `mkinidata` 直接数组越界崩溃。这类 bug
   **只有真的跑起来才抓得到**，结构性检查（cpp 预处理、Fortran 嵌套）
   都看不出来。修法是让 `allocate_BGCTimeVariables` 的调用也无条件——
   这些数组只有 `numpatch` 大小，关着 BGC 时白占一点内存，没有物理
   影响；`PFTimeVariables` 保持原样有条件分配，因为它只从已经被
   `DEF_USE_PFT`/`DEF_USE_PC` 挡住的代码或真正 `optional` 的实参里
   被碰到——不是所有「无条件传参」都能这样安全地无条件分配，得挨个
   查实参是不是真的必需。

**四条判据的结果**：

1. 黄金回归逐位不变：`identical: 129 variables, 10 dimensions`
2. `urban` 内核仍能跑通真实站点（Urban-PLUMBER）：`forcing_prep` 通过
3. 运行时切到 PFT 方案（`case.nml` 里 `DEF_USE_PFT=.true.`,
   `DEF_USE_LCT=.false.`，不改 `site.nc`）：**结果好于预期**——
   `CN-Cng` 站点文件本来就自带 `pctpfts`/`LAI`/`SAI`，`mksrfdata`/
   `mkinidata`/`colm` 三段全部跑完 11 天 528 步，PFT 与 LCT 两套
   restart 都写出来了，不是「优雅失败在 `plant_15s` 缺失」而是真的
   走通了一整条 PFT 次网格路径
4. 用 `git worktree` 建 `ef1177d`（本组开工前的状态）的 `default`
   内核，同一个算例，产物与运行时版本的 `default` 内核逐位相同：
   `identical: 129 variables, 10 dimensions`

**两道协调员额外要求的自检**：

- **cpp 预处理后的非空行行数**逐文件比较（`git show HEAD` 版本 vs
  改完后的工作区），112 个改动过的 `.F90` 文件全部只增不减——排除了
  `#else` 静默截断类的回归重新出现。裸行数比较（不剔除空行）里有
  52 个文件显示减 1~4 行，查实是移除 cpp 指令行在 `-P` 模式下的
  空行边界伪影，不是内容丢失——**行数比较要剔除空行再看，否则会有
  假阳性**。
- **`nm -g` 导出符号数**：`ef1177d` 的 `default` 内核编出 270 个目标
  文件，运行时版本编出 289 个（`main/BGC/`、`main/URBAN/` 不再被
  `URBANOFF`/`BGCOFF` 挡在编译之外），总导出符号 1178 → 1423；20 个
  重点改动文件（`MOD_Namelist.o`/`MOD_Vars_TimeVariables.o`/
  `MOD_BGC_Vars_TimeVariables.o`/`MOD_IniTimeVariable.o` 等）逐个比对，
  全部持平或增加，没有一个减少。

**两道隐藏的第二道门**：`Makefile` 里 `METHANE_ENABLED` 原来靠 cpp 探测
`define.h` 里的 `#ifdef BGC`，`BGC` 从 `define.h` 里消失后这个探测永远
读到 NO，改成硬编码 `YES`；其余 `ifeq`/`ifneq` 不引用 `BGC`/`CROP`/
`URBAN_MODEL`/`LULCC`，它们的目标文件本来就无条件列着。
`xtask/src/usage.rs` 的 `SUBSYSTEMS` 去掉 `("main/BGC/", "BGC")`，
`BY_NAME` 去掉 `("Urban", "URBAN_MODEL")`，`CURATED` 清空（唯一一条
`DEF_URBAN_type_scheme` 过时了，真实守护点已经从 `#ifdef URBAN_MODEL`
挪到 `IF (DEF_URBAN_RUN)`）。

## 已完成

| | 做了什么 | commit |
|---|---|---|
| 前置 | 内核构建自检实际生效的宏 | `32129e3` |
| 前置 | vendoring（709 个文件入库） | `acfb596` |
| 上游 | PR #15：TRACER 在单点下能编了 | `2f91b435` |

### 内核构建自检为什么必须先做

`create_defineh.bash` 生成的 `define.h` 里有**静默的条件 `#undef`**：

```c
#ifndef LULC_IGBP_PFT
#ifndef LULC_IGBP_PC
#undef BGC          // 配 LULC_IGBP 的话，BGC 被悄悄关掉
#endif
#endif
```

而清单里以前只记「我们传了什么参数」，不记「实际生效了什么宏」。
一个配错的预设会产出「名字对、内容错」的内核 —— **跑得完，结果全错，
而且判据本身是假的**。

现在 `kernel-manifest.json` 的 `macros` 字段记实际生效集，
`build_kernel.sh` 编译前核对，对不上就失败（退出码 3）。

## 一条容易踩的

**编译时真正读的 `define.h` 不是 `vendor/CoLM202X/include/define.h`。**

`.github/workflows/create_defineh.bash` 第 148–236 行整个重写它，
而且两份内容曾经**不一样** —— 静态那份一度有「`URBAN_MODEL && SinglePoint`
强制 `LULC_IGBP`」，生成的那份没有；2026-08-23 已删除这条过时静态约束。

**改宏配置要改那个脚本。** 我自己差点改错文件。

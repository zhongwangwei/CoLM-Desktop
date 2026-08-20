# 把 CoLM 的编译期宏改成运行时开关

**目标：一个二进制覆盖所有单点配置。**

现在每个宏组合要单独编一个内核（17 MB），有效组合几十上百种 ——
随包发不可能全覆盖，让用户自己编又要一整套 Fortran 工具链。

**前提**：`vendor/CoLM202X` 已经从 submodule 改成入库副本
（`acfb596`，见 `vendor/PROVENANCE.md`），改动就是我们自己的文件。

---

## 完整清单（实测处数，2026-08-20）

| 组 | 宏 | 处数 | 文件 | 状态 |
|---|---|---|---|---|
| **① 试点** | `CoLMDEBUG` | 92 | 30 | ✅ `acc596a` |
| | `RangeCheck` | 121 | 39 | ✅ 同上 |
| | `SrfdataDiag` | 98 | 20 | ✅ 同上 |
| **② 两套物理方案共存** | `Campbell_SOIL_MODEL` | 63 | 19 | 🔄 |
| | `vanGenuchten_Mualem_SOIL_MODEL` | 115 | 27 | 🔄 |
| | `extend_interception` | 4 | 1 | 🔄 |
| **③ 最大但独立** | `TRACER` | 342 | 69 | 待做 |
| **④ 核心难点（必须一起）** | `LULC_IGBP` | 205 | 46 | 待做 |
| | `LULC_IGBP_PC` | 159 | 40 | 待做 |
| | `LULC_IGBP_PFT` | 150 | 38 | 待做 |
| | `LULC_USGS` | 66 | 21 | 待做 |
| | `CROP` | 256 | 41 | 待做 |
| | `BGC` | 131 | 73 | 待做 |
| | `URBAN_MODEL` | 92 | 29 | 待做 |
| | `LULCC` | 16 | 8 | 待做 |

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
而且两份内容**不一样** —— 静态那份有「`URBAN_MODEL && SinglePoint`
强制 `LULC_IGBP`」，生成的那份没有。

**改宏配置要改那个脚本。** 我自己差点改错文件。

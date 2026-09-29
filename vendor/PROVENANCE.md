# `vendor/CoLM202X` 的来源

**这不再是 git submodule，是入库的源码副本。**

| | |
|---|---|
| 上游 | `https://github.com/zhongwangwei/CoLM202X.git` |
| 分出来的 commit | `2f91b435` |
| 分支 | `fix/tracer-singlepoint` |
| 入库日期 | 2026-08-20 |
| 规模 | 磁盘 709 个文件 / 约 18 MB，入库 **591 个** |

## 为什么入库的比磁盘上少 118 个

`vendor/CoLM202X/.gitignore:21` 有一条 `/tests` —— **那是上游自己的规则**，
不是我们加的。CoLM 把 `tests/`（118 个 Python 静态检查与 Fortran 测试
工具）排除在版本控制外，它们在 submodule 时期同样是未跟踪的。

保持原样：入库副本与上游的跟踪范围一致。要用那些测试就照上游的做法
另外获取。

## 为什么从 submodule 改成入库副本

要把 CoLM 的**编译期宏改成运行时开关** —— `LULC` 四种、`BGC`、`CROP`、
`URBAN_MODEL`、土壤水力二选一、调试三件套，约 1100 处条件编译，
分布在 118 个 `.F90` 文件里。

目的是让**一个二进制覆盖所有配置**：现在每个宏组合都要单独编一个内核
（一个 17 MB），而有效组合有几十上百种，随包发不可能全覆盖，
让用户自己编又要求一整套 Fortran 工具链。

在 submodule 里做这种改造，每次同步上游都要 rebase 上千处改动 ——
那不是可持续的。入库之后改动就是我们自己的文件。

## 已经带进来的上游修复

分出来的那个 commit 里已经含着：

1. **`fix/urban-site-fallbacks`**（PR #14）—— 城市单点算例能从站点文件
   读湖深与 LAI。两处：`readflag` 的判据写错、`LAI_year` 漏读。
2. **`fix/tracer-singlepoint`**（PR #15，`2f91b435`）—— 去掉
   `create_defineh.bash` 里那条过严的 `#error`。TRACER 的 42 个模块里
   只有 2 个需要 `GridRiverLakeFlow`，而那 2 个已经各自守着自己，
   那条 `#error` 却让**每一个单点构建**都用不了 TRACER。

两个 PR 都提在上游 fork 上，接受与否不影响这份副本。

2026-08-21 又从原始版 `a7e2b2f9` 同步两处明确的数值修复：地形阴影拟合
从第二个采样点起步，空间映射除法跳过零权重。LAI 缺失数据策略没有照搬：
桌面版仍保留逐 block 完整性检查，避免把未初始化缓冲区散播进模拟。

## 要同步上游时怎么做

```bash
git clone https://github.com/CoLM-SYSU/CoLM.git /tmp/colm-upstream
diff -ru /tmp/colm-upstream vendor/CoLM202X | less
```

**逐处判断**，因为我们这边会有大量有意的改动。上面那个 commit 号是
分叉点 —— 上游从那之后的改动才需要看。

## 编译时真正被读的是哪份 `define.h`

**不是 `include/define.h`。** `oracle/scripts/build_kernel.sh` 调用
`.github/workflows/create_defineh.bash`，那个脚本第 148–236 行
**整个重写** `include/define.h`（`cat>include/define.h<<EOF`）。

入库的那份静态 `include/define.h` 从来不被编译，它和生成的那份**内容不同** ——
比如静态那份有「`URBAN_MODEL && SinglePoint` 强制 `LULC_IGBP`」，
生成的那份没有。

**改宏配置要改 `create_defineh.bash`，不是 `include/define.h`。**

（那个脚本住在 `.github/workflows/` 下但不是工作流。副作用之一是
GitHub 不许没有 `workflow` scope 的 OAuth token 推它 —— 提 PR #15 时
只能走 SSH。）

## 2026-08-30 参数目录工作

本次统一参数目录、GUI 搜索和稀疏覆盖导入导出没有修改 `vendor/CoLM202X`
中的 Fortran 数值代码。默认值继续从本目录源码解析；尚无验证与真实回归路径的
PFT 常量保留为 `blocked-pending-hook`，没有为了数量指标运行时化。

## 2026 年 9 月：`f48fbf9` 生产同步与它的静态守卫

`main/HYDRO/MOD_Grid_Reservoir.F90`、`MOD_Grid_RiverLakeBifurcation.F90`、
`MOD_Grid_RiverLakeFlow.F90`、`MOD_Grid_RiverLakeHist.F90`、`main/MOD_LeafTemperature.F90`、
`MOD_LeafTemperaturePC.F90`、`MOD_Ozone.F90`、`share/MOD_Namelist.F90` 这 8 个文件
**按语义 hunk** 同步了上游 `f48fbf9` 的生产状态 —— 不是整树覆盖：
`MOD_Grid_Reservoir.F90` 里 `#ifdef FLAT_SPMD` 的传输分支（上游没有这条路径）
必须留住，而水库状态轴的编号要改成上游那套「按活跃水库稠密编号」
（`catalogue_to_active` / `icache`，catalogue 行号只当参数查表用）。

同步进来的几处顺带改了判据：河湖分汊的限幅器要**在净通量定稿后再套一次路径上限**，
levee 保护库容在 history 里必须单列（`below-bank river channel storage` /
`visible overbank storage excluding levee-protected storage`），levee 开关与重启文件
不一致要报错而不是静默继续。

`oracle/scripts/test_upstream_f48_sync.py` 是这次同步的**静态守卫**：8 个文件里 16 条
特征字符串，上游带来的与我们自己的各钉几条，断言失败就说明某处 hunk 漏了或被整树覆盖了。
它此前只存在于工作区，却被三个提交（`5d7f373`、`6d98eb6`、`b25b904`）的 `Tested:` 行
引用 —— 那三处证据对别人不可复现。现已入库并进 CI。**再同步上游时先跑它。**

## 2026 年 9 月：`extends/interception/` 补上漏掉的那次同步

`extend_interception` 打开时（`kernels/default` 就是），`Makefile:635-647` 用
`extends/interception/MOD_{Thermal_CanopyPhase,LeafTemperature,LeafTemperaturePC,LeafInterception}_Extended.F90`
**顶替** `main/` 下的同名模块 —— `main/` 那四份不参与编译。上面那次 `f48fbf9`
语义同步只改了 `main/`，扩展版因此落后了两处：

1. `MOD_LeafTemperature_Extended.F90` 里 `rstfacsun`/`rstfacsha` 仍在
   `intent(out)` 块中（`main/MOD_LeafTemperature.F90:269-272` 是单列的
   `intent(inout)`）。`intent(out)` 使入口值未定义，`stomata` 读到的实际是
   `rstfacsun_out` 的初值 `spval = -1e36`；`calc_photo_params` 用它无界地缩放
   `vm`/`jmax`/`respc`/`omss`，于是冠层光合恒为 0、气孔阻力 ≈ 0（无限制潜在蒸腾）。
2. 两份扩展叶温模块都缺 f48fbf9 的 `gssun`/`gssha` 诊断块。

已按 `main/` 的写法补齐（含 `MOD_LeafTemperaturePC_Extended.F90`），并把
`oracle/scripts/test_upstream_f48_sync.py` 的三条断言各加一份对 `extends/` 的镜像
—— 原来只钉 `main/`，所以漏了。**再同步上游时，两份都要看。**

证据、量化改善与"怎么判断哪份文件在编译"记在
`docs/implementation-verification.md`「找到蒸腾链的真凶：扩展截获模块漏了
`intent(inout)`」一节。同步后重生成过 `oracle/golden/*.nc` 与
`oracle/golden/kernel-manifest.json`（原基线距 HEAD 已 560 个提交）。

**没有跟着改的：** 扩展版与 `main/` 之间还有一批差异是那次扩展本身带来的
（`MOD_LeafTemperature_Extended.F90` 的能量平衡项改用随 `tl` 变的 `htvpl`、
引入 `canopy_phase_heat`、`qintr_*` 取 `max(0,·)`、`MARK#dtl` 的迭代标记等），
那是两套有意不同的实现，不在这次同步范围内；本仓库的 Rust 移植按 `main/` 写，
所以用到截获方案 4~7 时仍要逐处核对。

## 2026 年 9 月：`o3coef*` 提前初始化（本地修复，黄金已重生成）

`main/MOD_LeafTemperature.F90` 与 `extends/interception/MOD_LeafTemperature_Extended.F90`
（`extend_interception` 打开时后者顶替前者，`kernels/*` 五个预设全都是）把
`o3coefv_sun/o3coefv_sha/o3coefg_sun/o3coefg_sha` 的赋值放在**稳定性迭代之后**的
非臭氧分支里：

```fortran
      ENDDO                       ! 迭代结束
      IF(DEF_USE_OZONESTRESS)THEN
         ...
      ELSE
         o3coefv_sun = 1.0_r8     ! ← 太晚了
```

而循环体里已经把 `o3coefg_*` 交给了 `stomata`（并用于 `gs0sun/gs0sha` 诊断）。
这四个是 `intent(inout)` 的哑元，调用方（`MOD_Thermal_CanopyPhase_Extended.F90`）
用模块 SAVE 变量传进来，所以**一次运行里的第一次调用**读到的是未定义值；
之后每次调用结束都会被置回 1.0，于是**只有第 1 步**被污染。

实测（CN-Cng 干窗第 1 步，成对探针）：迭代第 1 轮 `rssun` 大 ~10 倍、冠层蒸腾
`etr` 小 4800 倍（6.60985755327047220e-05 对正确的 1.37119817067997586e-08），
黄金里的 `gs0sun = -4.53e38`、`vegwp`、`rst = -5e-31` 也都是它。把初始化挪到迭代
之前后，第 1 轮全部探测量降到 1 ULP，第 1 步重启从「26 个变量差 1e-6…1e0」降到
「18 个变量差 1e-16…1e-12」。

改动：把四个赋值从迭代后的 `ELSE` 分支移到子程序初始化块（`it = 1` 之前），
两份文件同步改；`oracle/scripts/test_upstream_f48_sync.py` 加两条断言
（初始化必须出现在 `DO WHILE (it .le. itmax)` 之前，且全文件只出现一次），
再同步上游时它会打回。

**两处没有跟着改**，因为本仓库没有能验证它们的算例：

1. `main/MOD_LeafTemperaturePC.F90` 与 `extends/interception/MOD_LeafTemperaturePC_Extended.F90`
   有同一个缺陷（`o3coef*(i)` 在 `:1780`/`:2047` 才置 1，`:1205`/`:1296` 已在用）。
   PFT/PC 聚合那条路本仓库的 Rust 还没移植，也没有 PC 黄金算例。
2. 臭氧应激打开时（`DEF_USE_OZONESTRESS`）`CalcOzoneStress` 仍在迭代之后调用，
   所以迭代内的气孔看不到氧胁迫 —— 这是与上面同源的设计问题，不是本次的未定义值问题。

黄金文件与 `oracle/golden/kernel-manifest.json` 已用修好的内核重新生成
（三份，`golden-run <case> --write-golden`）。同一次提交里还给 Rust 侧
`crates/colm-core/src/variably_saturated_flow.rs` 的下界断言放行了机器量级负值
（`-8.05e-18`，相对 27.58 mm 层厚是 3e-19）—— 上游没有这类断言，夹到 0 会让下游
看到与上游不同的数。

## 2026 年 9 月：`eroot`/`SoilSurfaceResistance` 改传 `(1:)` 段（本地修复，黄金**待**重生成）

`main/MOD_Thermal.F90`（3 处）、`extends/interception/MOD_Thermal_CanopyPhase_Extended.F90`（3 处）、
`main/URBAN/MOD_Urban_Thermal.F90`（1 处）把 `(lb:nl_soil)` 的 `dz/t/wliq/wice_*sno` 整列交给
哑元为 `(1:nl_soil)` 的 `eroot` 与 `SoilSurfaceResistance`，有雪时整列下移 `|snl|` 层。
全部改为传 `(1:)` 段 —— `MOD_BGC_Veg_CNFireLi2016.F90:109` 早就是这种写法。
最新上游 `CoLM-SYSU-integration@3c799bae` **仍有**这 4 处（`extends/` 那 3 处上游已不编译），
**同步时要保住本地版本**，最好也提给上游。

证据与数值影响见 `docs/implementation-verification.md` 第 399 轮。三份黄金是带错位的内核生成的，
PLUMBER2 盘挂载后需重生成。

同一轮对照最新上游的复核结论（详表同见第 399 轮）：`o3coef*` 两处（含本仓库未修的
`MOD_LeafTemperaturePC`）上游**已修**，同步时取上游；`create_defineh.bash` 的 `LATERAL_FLOW`
与 `SiteSYSUAtmos_IGBP_VG.nml` 的未声明键上游**仍在**，本地修复要保住。

## 2026-09-28：整体同步到 `CoLM-SYSU-integration@3c799bae`

**上游换了。** 此后以 `https://github.com/zhongwangwei/CoLM-SYSU-integration`（`master`）为准，
不再跟 `CoLM202X`。两者历史相连：`CoLM202X@2f91b435`（本副本的分叉点）与
`integration@3c799bae` 的合并基是 `8c72cae2`。

### 方法：把本地改动重放到新上游上，而不是反过来

在一个临时仓库里：以 `2f91b435` 为父提交，提交本副本的全部内容（= 本地改动），
再把这一个提交 cherry-pick 到 `integration@3c799bae` 上。这样三方合并的基就是
`2f91b435`，冲突只落在"上游与本地都改过"的地方：40 个文件、约 200 处，逐处判断。
`CoLMMAIN.F90`、`MOD_Grid_RiverLakeFlow.F90`、`Makefile` 三个文件上游重构过大，
直接取上游全文，再手工重放本地改动。

### 关键取舍

1. **扩展截获不再编译。** 上游 `d6de53e9` 去掉了 `extend_interception` 的接线，CoLM2024
   截获并入 `main/`。本副本跟随：`Makefile`、`create_defineh.bash`、`include/define.h`
   都不再有它；`extends/interception/` 取上游原样（休眠）。Rust 只对齐 `main/`。
   **代价**：黄金是用扩展版生成的，且 Rust 的叶温移植的是扩展版（`htvpl` 等），
   两者都要跟着重做（见 docs/implementation-verification.md 第 400 轮）。
2. **CaMa 不管。** `extends/CaMa` 取上游原样；桌面预设都不编 CaMa（`f90b793` 的
   `CAMA_*_OBJ` 条件化照旧重放进新 Makefile）。河道只对齐格网河湖流（GridRiverLakeFlow）。
3. **宏→运行时开关（8 个维度）照旧。** 上游新代码里约 370 处这类条件编译按
   `docs/plan-macro-runtime.md` 的规则转换；`oracle/scripts/test_upstream_f48_sync.py`
   现在会扫描 `main/ share/ mksrfdata/ mkinidata/ include/`，任何一处漏网都报错。
4. **本地修复全部保住**：eroot/`SoilSurfaceResistance` 的 `(1:)` 切片、`CatchLateralFlow`、
   示例 namelist 的未声明键、`pn = ps - 1` 无条件、`MOD_Filesystem`/`CoLM_Mkdir.c`、
   `o3coef*` 迭代前无条件置 1（上游这次也修了，但只修臭氧关闭的情形，已删掉其重复段）。

### 上游"TRACER 开关改变物理"——同步时发现，按上游两种构建各自的行为保留

上游这版在很多地方给 TRACER 构建和非 TRACER 构建写了**不同的物理**，不只是记账不同。
转换成 `DEF_USE_TRACER` 运行时开关时，一律保留两边各自的行为（开关关 = 上游非 TRACER
构建），**没有擅自统一**。它们应当报给上游确认哪一边是对的：

| 位置 | TRACER 构建 | 非 TRACER 构建 |
|---|---|---|
| `MOD_LeafTemperature(PC)`：`dewfraction` / `LEAF_temperature` 的 `ipft_index` | 截获方案 8 用逐 PFT 的 `ncd_p/ncw_p/bcw_p` | 退回 patch 级参数 |
| `MOD_LeafTemperaturePC` 露水更新 | 所有方案都用 colm2014 写法，另加 VEG_SNOW 关闭时的分相 | 方案 2–7 各自的写法 |
| `MOD_Hydro_SoilWater` 含水层交换 | PHS 根系净回水与基流分**两次**交换 | 合成**一次**交换 |
| `MOD_Hydro_SoilWater` Richards 收敛 | 收敛后做液态水质量投影 | 不投影 |
| `MOD_SoilSnowHydrology` 不透水顶层 `qgtop<0` | 按冰/液分配扣减（分叉点 `2f91b435` 对所有构建都是这样） | **改回**只扣液态水 |
| `MOD_SoilSnowHydrology` 露水/霜 | 受第 1 层孔隙容量限制，超出与被霜挤出的液态水进地表积水 | 直接加进第 1 层 |
| `CoLMMAIN` | 土壤表层霜冰重分配 `relocate_soil_frost_ice` | 不调用 |
| `CoLM.F90` 氮沉降初始年份 | `s_year` | `sdate(1)`（1 月 1 日 0 时起算时被 `adj2end` 推到前一年） |
| `MOD_NewSnow` 湿地暖地面降雪 | 另要求无雪层 | 无此条件 |
| `MOD_LeafInterception` CoLM2014 入口 | 修复 `ldew` 与雨/雪分量的不一致 | 不修复 |

另有一处上游**只在 TRACER 构建里**修掉的缺陷：`MOD_Thermal` 的 `pn = ps - 1`
（非 TRACER 构建里 `pn` 未定义）。本副本对所有构建都执行它。

### 验证（本次）

`oracle/scripts/build_kernel.sh default` 通过；CN-Cng/AT-Neu 示例三段跑通；schema/histmap
两张生成表已重生成（字段 +9 −1，见第 400 轮）；`test_upstream_f48_sync.py` PASS。
**黄金未重生成**（PLUMBER2 盘未挂载）。

上游缺陷与"TRACER 改变物理"的完整清单（含行号与处理方式）见 `docs/upstream-bugs.md`，以后新发现的也记在那里。

## 2026-09-29：增量同步到 `CoLM-SYSU-integration@85cf2328`

上游 master 只前进了一个提交（PR #17，`fix/tracer-impermeable-exchange` 合入），8 个文件 +71/−52，
把 `3c799bae..85cf2328` 的差异作为补丁打到本副本上：

- 7 个文件原样打上：`CoLMMAIN.F90`（TRACER/GridRiverLakeFlow 宏内的洪泛示踪物记账）、
  `HYDRO/MOD_Grid_RiverLakeFlow.F90`、`TRACER/*` 四个、`share/MOD_SPMD_Task.F90`（`CoLM_stop` 改为 `STOP 1`）。
- `HYDRO/MOD_Hydro_SoilWater.F90` 的 5 处（含水层交换只计透水层）上下文对不上——本副本已把
  `#ifdef TRACER` 转成 `IF (DEF_USE_TRACER)`——逐行手工套上，都在 `DEF_USE_TRACER` 分支里。

**本地修复**：上游单点 `mksrfdata/MKSRFDATA.F90` 用 `CALL CoLM_stop()` 作为**正常结束**，`STOP 1`
之后每次成功的单点 mksrfdata 都返回 1（`colm-cli` 因此判为失败）。这一处改回普通 `STOP`，与同步前
`CoLM_stop` 在非 MPI 构建下的行为相同（`docs/upstream-bugs.md` 第 30 条）。

非 TRACER 构建的物理没有变化：两套内核重编后重跑纯 Fortran 的 `ci6`（CROP）与 `bl`（默认），
历史 12/12、重启 51/51 与同步前逐位相同（第 437 轮）。

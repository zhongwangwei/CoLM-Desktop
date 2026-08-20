# CoLM 同位素模块与 IsoGSM 物理过程对照审查

日期：2026-07-29  
范围：

- CoLM：`main/TRACER/`
- IsoGSM：`/Users/zhongwangwei/Desktop/CoLM-Isotope/codes/IsoGSM`

## 结论

CoLM 的液–汽、冰–汽平衡分馏，MJ79 开放水面动力学分馏，
JM84 冰相沉积过饱和修正，以及 IsoGSM SMOW-normalized 强迫解码
均与参考实现一致或在常数精度内一致。

需要修改的主要问题不是“CoLM 没有逐行复刻 IsoGSM”，而是 CoLM
内部不同下垫面分支对相同过程采用了不同算法。

## 需要修改

### 1. 冰川和水体蒸发绕过共享有限水池限制器

`main/TRACER/MOD_Tracer_SpecialPatches.F90` 的冰川和水体路径：

- 对 Craig–Gordon 结果施加 `min(R_evap, R_out)` 单边上限；
- 直接按 `water_loss * R_evap` 计算同位素通量；
- 未使用普通土壤、冠层和湿地采用的有限水池子步进；
- 冰相升华未采用 `DEF_TRACER_SUBL_SKIN_MM` 表皮限制。

这会截断合法的净同位素交换，并使大时间步和冰相升华的行为与普通
下垫面不一致。

修改原则：复用 `MOD_Tracer_EvapLimit` 的共享实现，删除调用方单边
截断；水体液相仍采用 MJ79，冰川液相维持原 Craig–Gordon 动力学方案。

### 2. 湿地自由水面未采用开放水面风速动力学

`tracer_wetland` 把湿地液相损失描述为 open-water evaporation，
但使用固定的孔隙扩散指数；湖泊路径使用 MJ79 风速函数。

修改原则：向湿地同位素过程传入风速分量，液相使用
`tracer_alpha_kinetic_open_water`，冰相继续使用冰相 Craig–Gordon
动力学因子。

### 3. 湿地雪层缺少融水–冰同位素交换

普通雪层渗流在启用 `DEF_TRACER_SNOWMELT_EQUILIBRATION` 时，会使融水
向 `R_ice / alpha_ice_liq` 松弛；湿地的镜像雪层渗流路径缺少同一处理。

默认值为 0，因此当前默认配置不受影响；但启用开关后不同下垫面会产生
不同物理结果。

修改原则：湿地路径采用与普通路径相同的交换公式、限幅和守恒处理。

### 4. 水汽强迫缺测时静默回退到初始水体同位素比

fractionation tracer 虽然在配置阶段必须声明水汽强迫，但单格点、单时次
解码失败仍会回退到 `init_delta` 对应的水体比值。对于非极干空气，这会
把不合理的水汽组成送入 Craig–Gordon 和凝露/凝霜过程。

修改原则：

- 极干空气中 normalized ratio 无定义时允许缺值，因为 `h * R_vapor`
  的贡献趋近于零；
- 对有实际水汽质量但同位素值无效的情况明确告警或终止，不再静默伪造；
- 不为单一场景增加新的缺省参数体系。

### 5. 冻结分馏策略不统一

土壤和热力阶段冠层冻结采用完整平衡 Rayleigh 分馏；截留阶段冠层冻结
因缺少事件温度采用无分馏守恒转移。

完整平衡是上限端元，实际有效分馏依赖冻结速度和混合状态。本次不引入
未经标定的新经验系数；保留守恒分支并在运行时/文档中明确该限制。若后续
提供逐 PFT 冻结事件温度，再统一调用 Rayleigh 实现。

## 有意保留的差异

### IsoGSM 陆面无分馏假设

IsoGSM `gsml/ISOTOPE/isorsv.F` 明确假设陆地蒸发和径流无分馏，
`gsml/moninp.F` 的 land/ice evaporation 直接使用 reservoir ratio。

CoLM 的土壤、冠层、雪和湿地 Craig–Gordon 过程是更细化的陆面扩展，
不应仅为逐行对齐 IsoGSM 而删除。需要严格基准时，应单独运行关闭陆面
分馏的敏感性实验。

### 不在 CoLM 重复 IsoGSM 云微物理

当前 CoLM 使用的 `prate1sfc/prate2sfc` 已是 IsoGSM 云凝结、冰晶沉积和
雨滴再蒸发之后的地面降水同位素强迫。在 CoLM 入地前再次实现
`lrgscl/eqm_deg` 会造成双重分馏。

### 特殊地表仍采用整体混合水库

`sync_tracer_patch_ratio` 会把冰川和水体各层重置为同一个混合比值。
这与 IsoGSM 简单水库一致，可用于通量和总量研究；若研究冰芯、深雪层或
湖泊同位素剖面，则需要独立的分层状态设计，不属于本次最小修复范围。

## 对照证据

- 平衡分馏：
  - CoLM：`MOD_Tracer_Isotope_O18.F90:33-47`
  - CoLM：`MOD_Tracer_Isotope_HDO.F90:33-47`
  - IsoGSM：`gsml/ISOTOPE/freq.F:10-27`
- MJ79：
  - CoLM：`MOD_Tracer_Frac.F90:263-301`
  - IsoGSM：`gsml/ISOTOPE/frkin.F:12-23`
- JM84：
  - CoLM：`MOD_Tracer_Frac.F90:176-237`
  - IsoGSM：`gsml/CLD1/lrgscl.F:195-228`
- 强迫解码：
  - CoLM：`MOD_Tracer_Forcing.F90:539-556`
  - 配置：`run/standard_O18_parameter.nml`
  - 配置：`run/standard_HDO_parameter.nml`

## 验证基线

修改前目标测试：

```text
pytest -q \
  tests/test_tracer_isotope_isogsm_alignment.py \
  tests/test_tracer_isotope_frac_runtime.py \
  tests/test_tracer_isotope_nss_static.py

113 passed
```

修改完成后的测试结果记录在本文件末尾。

## 修改后验证

已完成：

- `MOD_Tracer_SpecialPatches.F90`
  - 冰川和水体液相/冰相损失改用共享有限水池限制器；
  - 删除 `min(R_evap, R_out)` 单边截断；
  - 冰相接入 `DEF_TRACER_SUBL_SKIN_MM`；
  - 水体液相保留 MJ79，冰川液相保留原动力学选择。
- `MOD_Tracer_SoilWater.F90`、`CoLMMAIN.F90`
  - 湿地接收风速分量并使用开放水面动力学；
  - 湿地雪层补齐融水–冰同位素交换。
- `MOD_Tracer_Forcing.F90`
  - 前三次强迫范围诊断同时报告 fractionating tracer 的水汽回退
    patch 数量，不再静默使用缺省比值。
- `README.md`、`share/MOD_Namelist.F90`
  - 记录冻结阶段诊断限制、强迫回退含义，以及 Merlivat/Cappa
    选择属于实验配置而非强迫数据的物理约束。
- `tests/test_tracer_isotope_isogsm_alignment.py`
  - 增加特殊地表、湿地、雪融交换和强迫回退回归检查。
- `tests/test_tracer_misc_static.py`
  - 将旧的按比例缩放断言更新为共享有限水池限制器断言。

验证结果：

```text
pytest -q tests/test_tracer_*.py
293 passed

make -j2 colm.x
completed successfully

git diff --check
passed
```

完整构建仍报告仓库已有的 MPI legacy-interface 类型/秩警告；本次修改
涉及的同位素模块均编译并链接成功，没有新增编译错误。

## 第二轮深审与修复

本轮继续沿强迫解码、有限水池、扩散、湿地和特殊地表的完整质量路径检查，
修复以下确定性问题：

- `MOD_Tracer_EvapLimit.F90`
  - 单步有限水池限制改为精确满足残余比值上限；
  - `source_ratio == r_max` 时不再错误拒绝非分馏通量；
  - 零库存下允许有物理来源的负 Craig–Gordon 净通量（凝结/吸收）。
- `MOD_Tracer_SoilWater.F90`
  - 土壤液相和水汽扩散使用同一时刻快照计算全部界面通量，再统一更新；
  - 对每个源层汇总双向流出并一次限幅，消除逐界面更新的顺序依赖；
  - 液相和水汽扩散开关彼此独立；
  - 湿地积雪补齐 firn 水汽扩散；
  - 湿地补齐 NSS 叶片源同位素、输出同位素、叶片暂存及落叶释放；
  - 大气交换与蒸腾分别记账，避免符号相反的通量净抵消后漏记守恒项。
- `MOD_Tracer_Forcing.F90`
  - 水汽和降水强迫只在初始化时设置默认值，后续无效值保留最近一次有效值；
  - 区分有效、近干分母和无效编码，并对首次及后续异常给出明确警告；
  - 分子/总量强迫统一 `dtime`、`offset`、插值和 `timelog` 语义。
- `MOD_Tracer_Hist.F90`
  - 同位素水汽交换历史量移出溶质专属分支；
  - 增加有符号的蒸发同位素质量历史量；
  - 输出范围限制为采用 delta 诊断的同位素 tracer。
- `MOD_Tracer_SpecialPatches.F90`
  - 冰川和水体地下残余不再隐式清零；
  - 残余显式转入地表固相或地表残余库，并清零原库，保持质量归宿可追踪。
- `MOD_Tracer_Isotope_HDO.F90`
  - 增加 HDO 平衡分馏运行时回归，覆盖低温下限和常用温度范围。

### 第二轮验证

```text
pytest -q tests/test_tracer_*.py
315 passed

git diff --check
passed
```

当前重新执行 `make -j2 colm.x` 在进入本轮修改的 tracer 模块之前失败：

```text
main/MOD_Vars_Global.F90:157
DEF_URBAN_type_scheme has no IMPLICIT type
```

该符号解析错误位于非同位素模块，且可在未应用本轮同位素差异的对应文件上
复现；因此本轮以完整 tracer 测试和静态差异检查作为验证证据，不将当前
全工程构建状态表述为成功。

### 仍保留的科学方案选择

以下事项需要先确定目标实验和状态变量设计，本轮不擅自改变：

1. 冰川和水体仍按整体混合水库同步各层；垂向同位素剖面需要新增分层状态。
2. JM84 地表结霜的事件温度和动力学方案尚无统一输入，暂不引入经验参数。
3. `DEF_TRACER_USE_FRACTIONATION` 与扩散开关的全局语义保持现状；若要求一个
   总开关同时关闭全部分馏和扩散，应单独调整配置契约并补敏感性基准。

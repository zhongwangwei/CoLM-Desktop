//! 雪层里的示踪物：新雪（`MOD_Tracer_Snow:tracer_newsnow`）、雪层合并/分裂
//! （`MOD_SnowLayersCombineDivide` 中 `trc_wliq/trc_wice/trc_solid/trc_scv` 可选实参那一支）
//! 与表层土新霜挪进雪（`MOD_NewSnow:relocate_soil_frost_ice` 的 `present(trc_wice)` 分支）。
//!
//! 上游把示踪物作为可选实参塞进水侧例程，与水量同一套拓扑逐层搬运。这里拆成独立函数：
//! 宿主在调水侧例程**之前**给出它读到的水量快照（`snl`、雪层 `wice`/`dz`），本模块按
//! Fortran 的判定（`wice <= 0.1`、`snowdp < 0.01`、`dz < dzmin`、分裂阈值）重放同一串
//! 合并/平移/拆分。判定只用到加法（`wice(j+1)+wice(j)`、`combo` 的 `dz+dz2`）与
//! `drr = dz - 阈值`，与水侧逐位相同，所以重放的拓扑与水侧一致；返回的 `snl` 供宿主核对。
//!
//! 合并/分裂里的示踪物循环是 `DO itrc = 1, size(trc_wliq,1)`，即**全部**示踪物（不只走通用
//! 输运的）；`tracer_newsnow` 跳过不走通用输运的；`relocate_soil_frost_ice` 用整列切片
//! `trc_wice(:,1)`，也是全部。
//!
//! `-fdefault-real-8` 下 `snowlayerscombine` 的 `0.01` 与 `_snicar` 的 `0.01_r8` 是同一个
//! f64，两个变体（以及两个分裂变体）的示踪物路径完全相同，各用一个函数。
//!
//! 收缩形状（GIMPLE `273t.optimized`）：
//! * `tracer_newsnow`（`MOD_Tracer_Snow.F90`）：没有 FMA，全部独立舍入、按源码顺序；
//! * `snowlayerscombine[_snicar]`/`snowlayersdivide[_snicar]`（`MOD_SnowLayersCombineDivide.F90`）
//!   的示踪物部分没有 FMA（文件里的 FMA 只在 `combo` 焓与 `z = FNMA(dz, 0.5, zi)`，属水侧）；
//!   `x/2.` 被改写成 `x*0.5`（逐位等价）；`propor*strc`、`strc(next)+z_strc` 各自舍入；
//! * `relocate_soil_frost_ice`（`MOD_NewSnow.F90`）：`excess = max(FNMA(denice*porsl1, dz1, wice1), 0)`；
//!   `trc_scv + fraction*trc_wice(1)` 与 `trc_wice(top) + fraction*trc_wice(1)` 都是
//!   `FMA(trc_wice(1), fraction, ·)`；`(1-fraction)*trc_wice(1)` 独立舍入。
//!
//! 这些路径都不调用同位素分馏（`MOD_Tracer_Frac`），因此不需要 `TracerPhysics`。

use std::fmt;

use super::{soisno_slot, PatchTracerState, TracerPools, TracerSet, MAX_SNOW_LAYERS, TRC_TINY};

/// 雪层数组下界 `lb = maxsnl+1`（CoLMMAIN 传示踪物时恒为 `-4`）。
const LB: i32 = 1 - MAX_SNOW_LAYERS as i32;
/// `snowlayerscombine` 的 `dzmin`（自上而下）。
const DZMIN: [f64; MAX_SNOW_LAYERS] = [0.010, 0.015, 0.025, 0.055, 0.115];
/// `denice` [kg m-3]。
const DENICE: f64 = 917.0;

// ---------------------------------------------------------------------------
// 逐层的三池操作（`trc_wliq`/`trc_wice`/`trc_solid` 总是一起出现）。
// ---------------------------------------------------------------------------

/// `trc(:, to) = trc(:, to) + trc(:, from)`。
fn add_layer(pools: &mut TracerPools, to: i32, from: i32) {
    let (to, from) = (soisno_slot(to), soisno_slot(from));
    pools.wliq_soisno[to] += pools.wliq_soisno[from];
    pools.wice_soisno[to] += pools.wice_soisno[from];
    pools.solid_soisno[to] += pools.solid_soisno[from];
}

/// `trc(:, to) = trc(:, from)`。
fn copy_layer(pools: &mut TracerPools, to: i32, from: i32) {
    let (to, from) = (soisno_slot(to), soisno_slot(from));
    pools.wliq_soisno[to] = pools.wliq_soisno[from];
    pools.wice_soisno[to] = pools.wice_soisno[from];
    pools.solid_soisno[to] = pools.solid_soisno[from];
}

/// `trc(:, first:last) = 0`。
fn zero_layers(pools: &mut TracerPools, first: i32, last: i32) {
    for j in first..=last {
        let slot = soisno_slot(j);
        pools.wliq_soisno[slot] = 0.0;
        pools.wice_soisno[slot] = 0.0;
        pools.solid_soisno[slot] = 0.0;
    }
}

// ---------------------------------------------------------------------------
// tracer_newsnow
// ---------------------------------------------------------------------------

/// 新雪之后的最上层雪（`wliq_soisno(snl+1)` 等；上游只在 `snl < 0` 时传这三个可选实参）。
#[derive(Debug, Clone, Copy, Default)]
pub struct NewSnowTop {
    /// `wliq_soisno(1)`：`newsnow` 之后最上层雪的液水。
    pub wliq: f64,
    /// `wice_soisno(1)`：`newsnow` 之后最上层雪的冰。
    pub wice: f64,
    /// `wice_soisno_bef(1)`：`newsnow` 之前同一层的冰（CoLMMAIN 先清零再拷旧的
    /// `wice(snl_new+1:0)`；只用于 `DEF_USE_CoLMDEBUG` 诊断）。
    pub wice_before: f64,
}

/// `tracer_newsnow` 的实参。
///
/// 上游的 `ipatch`（只用于诊断输出）、`scv_bef`、`wetwat_val`（两者在例程里未被使用）不在此列。
#[derive(Debug, Clone, Copy, Default)]
pub struct NewSnowInput {
    pub patchtype: i32,
    /// `newsnow` 之后的 `snl`。
    pub snl: i32,
    /// `newsnow` 之前的 `snl`（`snl_bef`）。
    pub snl_old: i32,
    /// 到地面的降雪 `pg_snow` [mm/s]。
    pub pg_snow: f64,
    pub deltim: f64,
    /// `newsnow` 之后的 `scv`。
    pub scv: f64,
    /// `snl < 0` 时的最上层雪；`None` 对应可选实参缺席。
    pub top: Option<NewSnowTop>,
    /// `DEF_USE_CoLMDEBUG`：打开时收集诊断。
    pub debug: bool,
}

/// `tracer_newsnow` 在 `DEF_USE_CoLMDEBUG` 下的诊断（上游 `write(*,...)`）。`itrc` 从 0 起。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NewSnowWarning {
    /// 新雪之前已有雪层却还有 `trc_scv` 残留。
    LayeredScvResidual { itrc: usize, trc_scv: f64 },
    /// Case B：新层冰量与本步降雪量不符。
    NewLayerMismatch {
        itrc: usize,
        wice: f64,
        snow_mass_step: f64,
    },
    /// Case C：顶层冰增量与本步降雪量不符。
    TopLayerMismatch {
        itrc: usize,
        d_wice: f64,
        snow_mass_step: f64,
        trc_pg_snow_ground: f64,
    },
}

impl fmt::Display for NewSnowWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::LayeredScvResidual { itrc, trc_scv } => write!(
                f,
                " WARNING tracer_newsnow: trc_scv residual under layered snow itrc={:3} \
                 trc_scv_pre={trc_scv:12.5e}",
                itrc + 1
            ),
            Self::NewLayerMismatch {
                itrc,
                wice,
                snow_mass_step,
            } => write!(
                f,
                " WARNING tracer_newsnow Case B: wice_soisno(1) /= pg_snow*dt itrc={:3} \
                 wice={wice:12.5e} pg_snow*dt={snow_mass_step:12.5e}",
                itrc + 1
            ),
            Self::TopLayerMismatch {
                itrc,
                d_wice,
                snow_mass_step,
                trc_pg_snow_ground,
            } => write!(
                f,
                " WARNING tracer_newsnow Case C: d_wice /= pg_snow*dt itrc={:3} \
                 d_wice={d_wice:12.5e} pg_snow*dt={snow_mass_step:12.5e} \
                 trc_pg_snow_ground={trc_pg_snow_ground:12.5e}",
                itrc + 1
            ),
        }
    }
}

/// `tracer_newsnow`：把本步到地面的雪示踪物（`TracerStep::pg_snow_ground`）放进雪。
///
/// * Case A（`snl_old == 0`，`snl < 0`）：首层刚由 `scv` 建起，`trc_scv + 本步雪示踪物`
///   按新层冰/液水量分配，`trc_scv` 清零；
/// * Case B（`snl < snl_old < 0`）：新层按本步降雪比值，残留 `trc_scv` 并入其冰；
/// * Case C（`snl == snl_old < 0`）：本步雪示踪物与残留 `trc_scv` 直接加到顶层冰；
/// * 其余（无雪层）：累加到 `trc_scv`；湿地暖地面（`patchtype == 2` 且 `scv < trc_tiny`）
///   整体转进 `trc_wetwat`。
///
/// 最后有雪层时顶层液相做溶解度平衡。返回 `DEF_USE_CoLMDEBUG` 诊断（`debug` 关时为空）。
pub fn tracer_newsnow(
    set: &TracerSet,
    state: &mut PatchTracerState,
    input: &NewSnowInput,
) -> Vec<NewSnowWarning> {
    let mut warnings = Vec::new();
    let snl = input.snl;
    let snl_old = input.snl_old;
    for (itrc, tracer) in set.tracers.iter().enumerate() {
        if !tracer.uses_land_water_transport() {
            continue;
        }
        let pg_snow_ground = state.step[itrc].pg_snow_ground;
        let pools = &mut state.pools[itrc];
        let snow_mass_step = input.pg_snow.max(0.0) * input.deltim;
        let r_snow_step = if snow_mass_step > TRC_TINY {
            pg_snow_ground / snow_mass_step
        } else {
            0.0
        };

        if input.debug && snl_old < 0 && pools.scv.abs() > TRC_TINY {
            warnings.push(NewSnowWarning::LayeredScvResidual {
                itrc,
                trc_scv: pools.scv,
            });
        }

        if snl_old == 0 && snl < 0 {
            // Case A：首层由累积的 scv 建起，按新层冰/液分配，不重复计入液相。
            let total_tracer = pools.scv + pg_snow_ground;
            let j = soisno_slot(snl + 1);
            let (ice_water, liq_water) = match input.top {
                Some(top) => (top.wice.max(0.0), top.wliq.max(0.0)),
                None => (0.0, 0.0),
            };
            let mut layer_water = ice_water + liq_water;
            if layer_water <= TRC_TINY {
                layer_water = input.scv.max(TRC_TINY);
            }
            let r_layer = total_tracer / layer_water;
            let mut ice_tracer = (ice_water * r_layer).max(0.0).min(total_tracer.max(0.0));
            let liq_tracer = (liq_water * r_layer)
                .max(0.0)
                .min((total_tracer - ice_tracer).max(0.0));
            if ice_water <= TRC_TINY && liq_water <= TRC_TINY {
                ice_tracer = total_tracer.max(0.0);
            }
            pools.wice_soisno[j] = ice_tracer;
            pools.wliq_soisno[j] = liq_tracer;
            pools.scv = 0.0;
        } else if snl < 0 && snl < snl_old {
            // Case B：已有雪层上又长出新层（`newsnow` 本身不会走到这里）。
            let j = soisno_slot(snl + 1);
            if let Some(top) = input.top {
                if input.debug && (top.wice - input.pg_snow.max(0.0) * input.deltim).abs() > 1.0e-6
                {
                    warnings.push(NewSnowWarning::NewLayerMismatch {
                        itrc,
                        wice: top.wice,
                        snow_mass_step: input.pg_snow.max(0.0) * input.deltim,
                    });
                }
                pools.wice_soisno[j] = top.wice * r_snow_step;
                pools.wliq_soisno[j] = top.wliq * r_snow_step;
            }
            if pools.scv.abs() > TRC_TINY {
                pools.wice_soisno[j] += pools.scv;
            }
            pools.scv = 0.0;
        } else if snl < 0 && snl == snl_old {
            // Case C：新雪加到现有顶层冰，不经 trc_scv。
            let j = soisno_slot(snl + 1);
            if let Some(top) = input.top {
                let d_wice = top.wice - top.wice_before;
                if pg_snow_ground > TRC_TINY {
                    pools.wice_soisno[j] += pg_snow_ground;
                }
                if input.debug
                    && (d_wice - snow_mass_step).abs() > 1.0e-6
                    && pg_snow_ground > TRC_TINY
                {
                    warnings.push(NewSnowWarning::TopLayerMismatch {
                        itrc,
                        d_wice,
                        snow_mass_step,
                        trc_pg_snow_ground: pg_snow_ground,
                    });
                }
                if pools.scv.abs() > TRC_TINY {
                    pools.wice_soisno[j] += pools.scv;
                }
            }
            pools.scv = 0.0;
        } else {
            // 无雪层：累积到 trc_scv，直到首层建起。
            pools.scv += pg_snow_ground;
            if input.patchtype == 2 && input.scv < TRC_TINY {
                // 湿地暖地面：newsnow 把 scv 转进 wetwat 并清零。
                pools.wetwat += pools.scv;
                pools.scv = 0.0;
            }
        }
        if snl < 0 {
            if let Some(top) = input.top {
                let j = soisno_slot(snl + 1);
                tracer.equilibrate_dissolved(
                    top.wliq.max(0.0),
                    &mut pools.wliq_soisno[j],
                    &mut pools.solid_soisno[j],
                );
            }
        }
    }
    warnings
}

// ---------------------------------------------------------------------------
// snowlayerscombine / SnowLayersCombine_snicar
// ---------------------------------------------------------------------------

/// 雪层合并之前宿主的水量快照（`snowlayerscombine[_snicar]` 入口处的值）。
///
/// 数组按 Fortran 雪层 `-4..=0` 存放（下标 = [`soisno_slot`]，与宿主
/// `RuntimeSnowColumn` 的雪层槽位相同）。只需判定用到的量：液水、温度不影响拓扑。
#[derive(Debug, Clone, Copy, Default)]
pub struct SnowCombineInput {
    /// 合并前的 `snl`。
    pub snl: i32,
    /// 合并前的 `wice_soisno(-4:0)`。
    pub wice: [f64; MAX_SNOW_LAYERS],
    /// 合并前的 `dz_soisno(-4:0)`。
    pub dz: [f64; MAX_SNOW_LAYERS],
}

/// 合并的结果：重放得到的 `snl`（宿主可与水侧核对）与是否走了"雪深不足 1 cm 全部化为
/// `scv`"分支。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnowCombineOutcome {
    pub snl: i32,
    pub all_snow_gone: bool,
}

/// `snowlayerscombine` / `SnowLayersCombine_snicar` 的示踪物部分。
///
/// 1. 冰量 `<= 0.1` 的层整体并入下一层（`j = 0` 时并入土壤第 1 层），上方各层下移一格；
/// 2. 仍有雪层而 `snowdp < 0.01`：各层冰示踪物之和成为 `trc_scv`（覆盖原值），液相与固相
///    之和加到土壤第 1 层，雪层清零；
/// 3. 两层以上时薄于 `dzmin` 的层与较薄的邻层合并（`combo`），上方下移。
///
/// 每次 `snl` 增加后把 `lb..snl` 的示踪物清零（上游如此，含已下移空出的槽）。
pub fn tracer_snow_layers_combine(
    set: &TracerSet,
    state: &mut PatchTracerState,
    input: &SnowCombineInput,
) -> SnowCombineOutcome {
    debug_assert_eq!(set.len(), state.pools.len());
    let pools = &mut state.pools;
    let s = soisno_slot;
    let mut wice = input.wice;
    let mut dz = input.dz;
    let mut snl = input.snl;

    // 冰量过小的层并入下面的邻层。
    let msn_old = snl;
    for j in msn_old + 1..=0 {
        if wice[s(j)] > 0.1 {
            continue;
        }
        // `j = 0` 时并入的是土壤第 1 层的冰，判定不再用到它。
        if j < 0 {
            wice[s(j + 1)] += wice[s(j)];
        }
        for p in pools.iter_mut() {
            add_layer(p, j + 1, j);
        }
        if j > snl + 1 && snl < -1 {
            for i in (snl + 2..=j).rev() {
                wice[s(i)] = wice[s(i - 1)];
                dz[s(i)] = dz[s(i - 1)];
                for p in pools.iter_mut() {
                    copy_layer(p, i, i - 1);
                }
            }
            for p in pools.iter_mut() {
                zero_layers(p, snl + 1, snl + 1);
            }
        }
        snl += 1;
        if snl >= LB {
            for p in pools.iter_mut() {
                zero_layers(p, LB, snl);
            }
        }
    }

    if snl == 0 {
        return SnowCombineOutcome {
            snl,
            all_snow_gone: false,
        };
    }

    let mut snowdp = 0.0;
    for j in snl + 1..=0 {
        snowdp += dz[s(j)];
    }

    if snowdp < 0.01 {
        // 雪全没了：冰进 trc_scv，液水（与固相）留在土壤表层。
        for p in pools.iter_mut() {
            let mut zwtrc_ice = 0.0;
            let mut zwtrc_liq = 0.0;
            let mut zwtrc_solid = 0.0;
            for j in snl + 1..=0 {
                zwtrc_ice += p.wice_soisno[s(j)];
                zwtrc_liq += p.wliq_soisno[s(j)];
                zwtrc_solid += p.solid_soisno[s(j)];
            }
            p.scv = zwtrc_ice;
            p.wliq_soisno[s(1)] += zwtrc_liq;
            p.solid_soisno[s(1)] += zwtrc_solid;
            zero_layers(p, LB, 0);
        }
        return SnowCombineOutcome {
            snl: 0,
            all_snow_gone: true,
        };
    }

    if snl < -1 {
        let msn_old = snl;
        let mut mssi = 0;
        for i in msn_old + 1..=0 {
            if dz[s(i)] >= DZMIN[mssi] {
                mssi += 1;
                continue;
            }
            let neibor = if i == snl + 1 {
                i + 1
            } else if i == 0 || (dz[s(i - 1)] + dz[s(i)]) < (dz[s(i + 1)] + dz[s(i)]) {
                // 底层（下面是土壤）只能并上邻层；否则并入较薄的邻层。
                i - 1
            } else {
                i + 1
            };
            // 层 l 与 j 合并存进 j。
            let (j, l) = if neibor > i { (neibor, i) } else { (i, neibor) };
            // `combo`：`dzc = dz_soisno + dz2`。
            dz[s(j)] += dz[s(l)];
            for p in pools.iter_mut() {
                add_layer(p, j, l);
            }
            if j - 1 > snl + 1 {
                for k in (snl + 2..=j - 1).rev() {
                    dz[s(k)] = dz[s(k - 1)];
                    for p in pools.iter_mut() {
                        copy_layer(p, k, k - 1);
                    }
                }
            }
            snl += 1;
            if snl >= LB {
                for p in pools.iter_mut() {
                    zero_layers(p, LB, snl);
                }
            }
            if snl >= -1 {
                break;
            }
        }
    }

    SnowCombineOutcome {
        snl,
        all_snow_gone: false,
    }
}

// ---------------------------------------------------------------------------
// snowlayersdivide / SnowLayersDivide_snicar
// ---------------------------------------------------------------------------

/// 雪层分裂之前宿主的水量快照（`snowlayersdivide[_snicar]` 入口处的值）。
#[derive(Debug, Clone, Copy, Default)]
pub struct SnowDivideInput {
    /// 分裂前的 `snl`（`< 0`）。
    pub snl: i32,
    /// 分裂前的 `dz_soisno(-4:0)`（下标 = [`soisno_slot`]）。
    pub dz: [f64; MAX_SNOW_LAYERS],
}

/// 分裂时对工作列（自上而下 `1..msno`，0 起）的一步操作。
#[derive(Debug, Clone, Copy)]
enum DivideOp {
    /// `s(pos) = s(pos)/2; s(pos+1) = s(pos)`。
    Halve(usize),
    /// `z = out*s(pos); s(pos) = keep*s(pos); s(pos+1) = s(pos+1) + z`。
    MoveExcess { pos: usize, out: f64, keep: f64 },
}

/// 由厚度重放分裂的判定，得到操作序列与最终层数。
fn divide_plan(input: &SnowDivideInput) -> (Vec<DivideOp>, usize) {
    let snl = input.snl;
    let mut msno = snl.unsigned_abs() as usize;
    let mut dzsno = [0.0; MAX_SNOW_LAYERS];
    for (k, dzk) in dzsno.iter_mut().enumerate().take(msno) {
        *dzk = input.dz[soisno_slot(k as i32 + 1 + snl)];
    }
    let mut ops = Vec::new();
    let halve = |dzsno: &mut [f64; MAX_SNOW_LAYERS], ops: &mut Vec<DivideOp>, pos: usize| {
        dzsno[pos] /= 2.0;
        dzsno[pos + 1] = dzsno[pos];
        ops.push(DivideOp::Halve(pos));
    };
    let move_excess =
        |dzsno: &mut [f64; MAX_SNOW_LAYERS], ops: &mut Vec<DivideOp>, pos: usize, keep_dz: f64| {
            let drr = dzsno[pos] - keep_dz;
            let out = drr / dzsno[pos];
            let keep = keep_dz / dzsno[pos];
            dzsno[pos] = keep_dz;
            // `combo`：`dzc = dz_soisno + dz2`。
            dzsno[pos + 1] += drr;
            ops.push(DivideOp::MoveExcess { pos, out, keep });
        };

    if msno == 1 && dzsno[0] > 0.03 {
        msno = 2;
        halve(&mut dzsno, &mut ops, 0);
    }
    // (位置, 保留厚度, 下一层再分的阈值)。
    for (pos, keep_dz, split) in [(0, 0.02, 0.07), (1, 0.05, 0.18), (2, 0.11, 0.41)] {
        if msno > pos + 1 && dzsno[pos] > keep_dz {
            move_excess(&mut dzsno, &mut ops, pos, keep_dz);
            if msno <= pos + 2 && dzsno[pos + 1] > split {
                msno = pos + 3;
                halve(&mut dzsno, &mut ops, pos + 1);
            }
        }
    }
    if msno > 4 && dzsno[3] > 0.23 {
        move_excess(&mut dzsno, &mut ops, 3, 0.23);
    }
    (ops, msno)
}

/// 一个池的工作列操作。
fn apply_divide_ops(column: &mut [f64; MAX_SNOW_LAYERS], ops: &[DivideOp]) {
    for op in ops {
        match *op {
            DivideOp::Halve(pos) => {
                column[pos] /= 2.0;
                column[pos + 1] = column[pos];
            }
            DivideOp::MoveExcess { pos, out, keep } => {
                let moved = out * column[pos];
                column[pos] *= keep;
                column[pos + 1] += moved;
            }
        }
    }
}

/// `snowlayersdivide` / `SnowLayersDivide_snicar` 的示踪物部分：与水量同比例拆分
/// （对半、按 `propor` 把超出的厚度连同示踪物并入下一层）。返回重放得到的新 `snl`。
pub fn tracer_snow_layers_divide(
    set: &TracerSet,
    state: &mut PatchTracerState,
    input: &SnowDivideInput,
) -> i32 {
    debug_assert_eq!(set.len(), state.pools.len());
    let snl = input.snl;
    if snl >= 0 {
        // 上游只在 `snl < 0` 时调用；`msno = 0` 时整段不动。
        return snl;
    }
    let (ops, msno) = divide_plan(input);
    let new_snl = -(msno as i32);
    let old_count = snl.unsigned_abs() as usize;
    for p in state.pools.iter_mut() {
        for field in [&mut p.wice_soisno, &mut p.wliq_soisno, &mut p.solid_soisno] {
            let mut column = [0.0; MAX_SNOW_LAYERS];
            for (k, value) in column.iter_mut().enumerate().take(old_count) {
                *value = field[soisno_slot(k as i32 + 1 + snl)];
            }
            apply_divide_ops(&mut column, &ops);
            for (k, value) in column.iter().enumerate().take(msno) {
                field[soisno_slot(k as i32 + 1 + new_snl)] = *value;
            }
        }
    }
    new_snl
}

// ---------------------------------------------------------------------------
// relocate_soil_frost_ice
// ---------------------------------------------------------------------------

/// `relocate_soil_frost_ice` 入口处的水量（宿主调用水侧例程之前的值）。
#[derive(Debug, Clone, Copy, Default)]
pub struct FrostRelocationInput {
    /// 调用前的 `snl`。
    pub snl: i32,
    /// `porsl1`：表层土孔隙度。
    pub porsl1: f64,
    /// `dz(1)`：表层土厚度 [m]。
    pub dz1: f64,
    /// `wice(1)`：表层土冰 [kg m-2]（调用前）。
    pub wice1: f64,
    /// `snowdp`：调用前的雪深 [m]（只在 `snl == 0` 时用到）。
    pub snowdp: f64,
}

/// 挪动的量，供宿主核对。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrostRelocation {
    pub excess: f64,
    pub fraction: f64,
    pub added_depth: f64,
    /// `snl == 0` 且累积雪深达到 1 cm、新建了第 0 层。
    pub created_layer: bool,
}

/// `relocate_soil_frost_ice` 的 `present(trc_wice)` 分支：表层土冰超出孔隙容量的比例
/// `fraction = excess/wice(1)` 连同冰示踪物挪进雪。无雪层时进 `trc_scv`（雪深够 1 cm 时
/// `trc_scv` 连同挪来的一起成为新第 0 层的冰示踪物，该层液相与固相清零）；有雪层时并入
/// 顶层冰。土壤第 1 层冰示踪物乘 `1-fraction`。没有超出时返回 `None`、不动任何量。
pub fn tracer_relocate_soil_frost_ice(
    set: &TracerSet,
    state: &mut PatchTracerState,
    input: &FrostRelocationInput,
) -> Option<FrostRelocation> {
    debug_assert_eq!(set.len(), state.pools.len());
    let excess = (-(DENICE * input.porsl1))
        .mul_add(input.dz1, input.wice1)
        .max(0.0);
    if excess <= 0.0 {
        return None;
    }
    let fraction = excess / input.wice1;
    let added_depth = excess / DENICE;
    let soil = soisno_slot(1);
    let mut created_layer = false;
    if input.snl == 0 {
        created_layer = input.snowdp + added_depth >= 0.01;
        let top = soisno_slot(0);
        for p in state.pools.iter_mut() {
            if created_layer {
                p.wice_soisno[top] = p.wice_soisno[soil].mul_add(fraction, p.scv);
                p.scv = 0.0;
                p.wliq_soisno[top] = 0.0;
                p.solid_soisno[top] = 0.0;
            } else {
                p.scv = p.wice_soisno[soil].mul_add(fraction, p.scv);
            }
            p.wice_soisno[soil] *= 1.0 - fraction;
        }
    } else {
        let top = soisno_slot(input.snl + 1);
        for p in state.pools.iter_mut() {
            p.wice_soisno[top] = p.wice_soisno[soil].mul_add(fraction, p.wice_soisno[top]);
            p.wice_soisno[soil] *= 1.0 - fraction;
        }
    }
    Some(FrostRelocation {
        excess,
        fraction,
        added_depth,
        created_layer,
    })
}

#[cfg(test)]
#[path = "snow_tests.rs"]
mod tests;

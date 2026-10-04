//! 冠层截留后的示踪物（`MOD_Tracer_Precip:tracer_precip`）。
//!
//! 降水到达冠层：截留部分与原有冠层水混合，穿透部分保持降水比值，冠层饱和后滴落的
//! 带混合比值。冠层内的雨/雪相变（`ldew_smelt`/`ldew_frzc`）先搬运示踪物。到地面的
//! 雨、雪示踪物分别交给土壤水与新雪过程。
//!
//! 收缩形状取自 `MOD_Tracer_Precip.F90` 的 GIMPLE：
//! * `a_trc_precip = FMA(deltim, (forc_snow+forc_rain)*R, a)`；
//! * `R_rain_input = FMA(max(forc_rain,0), R, sprinkler*ratio) / rain_total`；
//! * 相变 `FNMA(R_pre, m, from)`、`FMA(R_pre, m, to)`；
//! * `trc_mixed = FMA(R, intercepted, trc - trc_xsc)`、`trc_ldew = max(FNMA(R_mixed, drip, trc_mixed), 0)`；
//! * 到地面 `FMA(R_mixed, drip, FMA(R, throughfall, trc_xsc))`；
//! * `canopy_input = FMA(deltim*rain_total, R_rain, (deltim*max(forc_snow,0))*R)`。

use super::{PatchTracerState, TracerSet, TRC_TINY, TRC_WATER_MIN_FOR_RATIO};

/// 一个 patch 本步的冠层截留水量（`tracer_precip` 的实参）。
#[derive(Debug, Clone, Copy, Default)]
pub struct PrecipInput {
    pub deltim: f64,
    pub forc_rain: f64,
    pub forc_snow: f64,
    pub pg_rain: f64,
    pub pg_snow: f64,
    /// 截留之后的 `ldew_rain`/`ldew_snow`。
    pub ldew_rain: f64,
    pub ldew_snow: f64,
    /// 截留之前的 `ldew_rain`/`ldew_snow`。
    pub ldew_rain_old: f64,
    pub ldew_snow_old: f64,
    pub sprinkler: f64,
    pub gross_intr_rain: f64,
    pub gross_intr_snow: f64,
    pub xsc_rain: f64,
    pub xsc_snow: f64,
    pub ldew_smelt_mass: f64,
    pub ldew_frzc_mass: f64,
    /// CROP 灌溉打开时，喷灌取水之前的 `waterstorage`。
    pub waterstorage_before: Option<f64>,
}

/// `tracer_precip`。`precip_ratio[itrc]` 是本步本 patch 的降水比值
/// （`tracer_forcing_precip_value`）。
pub fn tracer_precip(
    set: &TracerSet,
    state: &mut PatchTracerState,
    precip_ratio: &[f64],
    input: &PrecipInput,
) {
    let dt = input.deltim;
    let sprinkler_rate = input.sprinkler.max(0.0);
    let sprinkler_water = sprinkler_rate * dt;
    let rain_total = input.forc_rain.max(0.0) + sprinkler_rate;
    for (itrc, tracer) in set.tracers.iter().enumerate() {
        if !tracer.uses_land_water_transport() {
            continue;
        }
        let r_input = precip_ratio[itrc];
        let mut storage_ratio = r_input;
        let pools = &mut state.pools[itrc];
        tracer.equilibrate_dissolved(
            input.ldew_rain_old.max(0.0),
            &mut pools.ldew_rain,
            &mut pools.canopy_solid,
        );
        let canopy_trc_beg = (pools.ldew_rain + pools.ldew_snow) + pools.canopy_solid;

        let acc = &mut state.acc[itrc];
        acc.precip = dt.mul_add((input.forc_snow + input.forc_rain) * r_input, acc.precip);

        // 喷灌取自 `waterstorage`：内部转移，不进大气输入。
        if input.sprinkler > TRC_TINY {
            if let Some(before) = input.waterstorage_before {
                tracer.equilibrate_dissolved(
                    before.max(0.0),
                    &mut pools.waterstorage,
                    &mut pools.waterstorage_solid,
                );
                if before > TRC_WATER_MIN_FOR_RATIO {
                    storage_ratio = pools.waterstorage / before;
                }
            }
            let sprinkler_trc = (sprinkler_water * storage_ratio)
                .max(0.0)
                .min(pools.waterstorage.max(0.0));
            pools.waterstorage -= sprinkler_trc;
            if sprinkler_water > TRC_TINY {
                storage_ratio = sprinkler_trc / sprinkler_water;
            }
            if let Some(before) = input.waterstorage_before {
                tracer.equilibrate_dissolved(
                    (before - sprinkler_water).max(0.0),
                    &mut pools.waterstorage,
                    &mut pools.waterstorage_solid,
                );
            }
        }

        let r_rain_input = if rain_total > TRC_TINY {
            input
                .forc_rain
                .max(0.0)
                .mul_add(r_input, storage_ratio * sprinkler_rate)
                / rain_total
        } else {
            r_input
        };

        // 0. 冠层相变：先融后冻。
        let mut rain_old = input.ldew_rain_old.max(0.0);
        let mut snow_old = input.ldew_snow_old.max(0.0);
        let smelt = input.ldew_smelt_mass.max(0.0).min(snow_old);
        if smelt > 0.0 {
            let r_snow = if snow_old > TRC_TINY {
                pools.ldew_snow / snow_old
            } else {
                r_input
            };
            pools.ldew_snow = (-r_snow).mul_add(smelt, pools.ldew_snow).max(0.0);
            pools.ldew_rain = r_snow.mul_add(smelt, pools.ldew_rain);
            snow_old -= smelt;
            rain_old += smelt;
        }
        let frzc = input.ldew_frzc_mass.max(0.0).min(rain_old);
        if frzc > 0.0 {
            let r_rain = if rain_old > TRC_TINY {
                pools.ldew_rain / rain_old
            } else {
                r_input
            };
            pools.ldew_rain = (-r_rain).mul_add(frzc, pools.ldew_rain).max(0.0);
            pools.ldew_snow = r_rain.mul_add(frzc, pools.ldew_snow);
            rain_old -= frzc;
            snow_old += frzc;
        }

        // 无叶冠层把旧水整体按叶温释放成雨或雪：先把全部旧示踪物归到那一相。
        let canopy_old_water = rain_old + snow_old;
        let xsc_rain_water = input.xsc_rain.max(0.0) * dt;
        let xsc_snow_water = input.xsc_snow.max(0.0) * dt;
        if input.ldew_rain.max(0.0) + input.ldew_snow.max(0.0) <= TRC_WATER_MIN_FOR_RATIO
            && canopy_old_water > TRC_WATER_MIN_FOR_RATIO
            && ((xsc_rain_water > TRC_WATER_MIN_FOR_RATIO
                && xsc_snow_water <= TRC_WATER_MIN_FOR_RATIO)
                || (xsc_snow_water > TRC_WATER_MIN_FOR_RATIO
                    && xsc_rain_water <= TRC_WATER_MIN_FOR_RATIO))
            && (input.xsc_rain.max(0.0) + input.xsc_snow.max(0.0)) * dt
                >= canopy_old_water - TRC_WATER_MIN_FOR_RATIO
        {
            let release = (pools.ldew_rain + pools.ldew_snow) + pools.canopy_solid;
            pools.canopy_solid = 0.0;
            if xsc_rain_water > TRC_WATER_MIN_FOR_RATIO {
                pools.ldew_rain = release;
                pools.ldew_snow = 0.0;
                rain_old = canopy_old_water;
                snow_old = 0.0;
            } else {
                pools.ldew_rain = 0.0;
                pools.ldew_snow = release;
                rain_old = 0.0;
                snow_old = canopy_old_water;
            }
        }

        // 雨：先释放旧冠层水（xsc，带混合前比值），再与截留的雨混合，滴落带混合比值。
        let (rain_ground, ldew_rain) = canopy_phase(
            pools.ldew_rain,
            rain_old,
            xsc_rain_water,
            input.gross_intr_rain,
            rain_total,
            input.pg_rain,
            r_rain_input,
            dt,
            DripCap::Intercepted,
        );
        pools.ldew_rain = ldew_rain;
        // 雪：同一套两段式，穿透量按 `forc_snow`（不含喷灌）。
        let (snow_ground, ldew_snow) = canopy_phase(
            pools.ldew_snow,
            snow_old,
            xsc_snow_water,
            input.gross_intr_snow,
            input.forc_snow,
            input.pg_snow,
            r_input,
            dt,
            DripCap::Mixed,
        );
        pools.ldew_snow = ldew_snow;
        let mut trc_rain_ground = rain_ground;
        let mut trc_snow_ground = snow_ground;

        // 冠层收支精确闭合：残差记到到地面的通量上（大气输入不变）。
        tracer.equilibrate_dissolved(
            input.ldew_rain.max(0.0),
            &mut pools.ldew_rain,
            &mut pools.canopy_solid,
        );
        let canopy_trc_end = (pools.ldew_rain + pools.ldew_snow) + pools.canopy_solid;
        let canopy_input =
            (dt * rain_total).mul_add(r_rain_input, (dt * input.forc_snow.max(0.0)) * r_input);
        let canopy_resid = (((canopy_trc_end - canopy_trc_beg) - canopy_input) + trc_rain_ground)
            + trc_snow_ground;
        if canopy_resid.abs() > TRC_TINY {
            let ground_total = trc_rain_ground + trc_snow_ground;
            let desired = (ground_total - canopy_resid).max(0.0);
            if ground_total > TRC_TINY {
                trc_rain_ground = trc_rain_ground * desired / ground_total;
                trc_snow_ground = trc_snow_ground * desired / ground_total;
            } else if rain_total > TRC_TINY {
                trc_rain_ground = desired;
            } else {
                trc_snow_ground = desired;
            }
        }

        // 地面的雨是液相：超出溶解度的直接进地表固相。
        tracer.equilibrate_dissolved(
            input.pg_rain.max(0.0) * dt,
            &mut trc_rain_ground,
            &mut pools.surface_solid,
        );
        state.step[itrc].pg_rain_ground = trc_rain_ground;
        state.step[itrc].pg_snow_ground = trc_snow_ground;
    }
}

/// 两相的差别：滴落上限（雨按截留量，雪按混合后的冠层水）与穿透/滴落的收缩形状。
#[derive(Clone, Copy)]
enum DripCap {
    Intercepted,
    Mixed,
}

/// 一相的两段式归属。返回 `(到地面的示踪物, 冠层剩余示踪物)`。
#[allow(clippy::too_many_arguments)]
fn canopy_phase(
    trc_pool: f64,
    old_water: f64,
    xsc_water: f64,
    gross_intr: f64,
    arriving_rate: f64,
    pg: f64,
    r_input: f64,
    dt: f64,
    cap: DripCap,
) -> (f64, f64) {
    let xsc_mass = xsc_water.min(old_water);
    let r_pre = if old_water > TRC_TINY {
        trc_pool / old_water
    } else {
        r_input
    };
    let trc_xsc = r_pre * xsc_mass;
    let pre_mix = old_water - xsc_mass;
    let intercepted = dt * gross_intr.max(0.0);
    let water_mixed = pre_mix + intercepted;
    let trc_mixed = r_input.mul_add(intercepted, trc_pool - trc_xsc);
    let r_mixed = if water_mixed > TRC_TINY {
        trc_mixed / water_mixed
    } else {
        r_input
    };
    // 雨支 `dt*rain_total`、`dt*max(pg_rain,0)` 另有他用，独立舍入；雪支两处都收成
    // `FMS(deltim, x, y)`（GIMPLE `_456`、`_233`）。
    let (throughfall, drip) = match cap {
        DripCap::Intercepted => {
            let throughfall = (dt * arriving_rate - intercepted).max(0.0);
            (
                throughfall,
                ((dt * pg.max(0.0) - xsc_mass) - throughfall).max(0.0),
            )
        }
        DripCap::Mixed => {
            let throughfall = dt.mul_add(arriving_rate, -intercepted).max(0.0);
            (
                throughfall,
                (dt.mul_add(pg.max(0.0), -xsc_mass) - throughfall).max(0.0),
            )
        }
    };
    let drip = match cap {
        DripCap::Intercepted => drip.min(intercepted.max(0.0)),
        DripCap::Mixed => drip.min(water_mixed.max(0.0)),
    };
    let remaining = (-r_mixed).mul_add(drip, trc_mixed).max(0.0);
    let ground = r_mixed.mul_add(drip, r_input.mul_add(throughfall, trc_xsc));
    (ground, remaining)
}

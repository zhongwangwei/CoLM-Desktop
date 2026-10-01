//! 示踪物 history（`MOD_Tracer_Hist`）：步末累加与写出前的取值。
//!
//! [`tracer_hist_accumulate`] 是上游同名例程（CoLMMAIN 与冰川/水体 patch 调用）；
//! [`TRACER_HISTORY_VARIABLES`] 按 `tracer_hist_out` 的写出顺序列出每个变量的名字、
//! 维度、`DEF_hist_vars` 开关、long_name/units 与所用累加器；写出前的数值由
//! [`single_point_value`]/[`single_point_soisno`]（`HistForm == 'Single'`，`patch` 维）
//! 与 [`GridCell`]（`HistForm == 'Gridded'`，面积加权）给出。NetCDF 由运行时写。
//!
//! 比值与 δ 一律"示踪物质量与水量分别聚合、最后一次相除"（上游 `MOD_Tracer_Defs` 的
//! DELTA DIAGNOSTIC INVARIANT）：网格上质量与水量各自 `Σ area*v`，再相除。
//!
//! GIMPLE（`MOD_Tracer_Hist.F90`、`MOD_SpatialMapping.F90`、`MOD_Tracer_Defs.F90`）：
//! * 累加全部是源码顺序的独立加法，`a_trc_wa_debt_mass - (wa*m)/w`；
//! * 叶水 `R_b = FMA(delta_b, 1e-3, 1)*ref_ratio`，`mass = R_b*moles`；
//! * `mass_to_delta = ((m/w)/ref - 1)*1000` 独立舍入；
//! * 单点三维比值 `((m/w)*nac)/nac`（`single_write_3d` 内部再除 `nac`）；
//! * 网格 `pset2grid`（带 `spv`）`cell = cell + (v/1)*area` 不融合，三维同样不融合。

use super::{
    soisno_slot, PatchTracerState, TracerDescriptor, TracerSet, MAX_SNOW_LAYERS, SOIL_LAYERS,
    SOISNO_LAYERS, TRC_TINY, TRC_WATER_MIN_FOR_RATIO,
};

/// `spval`（`MOD_Vars_Global`）。
pub const SPVAL: f64 = -1.0e36;
/// `trc_water_min_for_delta`：储量 δ 的最小累计水量 [kg m-2]。
pub const TRC_WATER_MIN_FOR_DELTA: f64 = 1.0e-3;
/// `trc_flux_water_min_for_delta`：通量 δ 的最小累计水量 [kg m-2]。
pub const TRC_FLUX_WATER_MIN_FOR_DELTA: f64 = 1.0e-1;
/// `trc_delta_sanity_max`：|δ| 超过它即缺测 [‰]。
pub const TRC_DELTA_SANITY_MAX: f64 = 2.0e3;
/// `flux_map_and_write_2d` 的 `sumarea > 0.00001`：缺省 kind 的实数常量，内核以
/// `-fdefault-real-8` 编译，GIMPLE 里是双精度 `1.00000000000000008e-5`。
pub const GRID_MIN_SUMAREA: f64 = 1.0e-5;

/// `tracer_hist_accumulate` 的实参（步末水量）。
#[derive(Debug, Clone, Copy)]
pub struct HistAccumulateInput<'a> {
    pub snl: i32,
    pub ldew_rain: f64,
    pub ldew_snow: f64,
    /// `wliq_soisno(maxsnl+1:nl_soil)`（只读 `snl+1..`）。
    pub wliq_soisno: &'a [f64; SOISNO_LAYERS],
    pub wice_soisno: &'a [f64; SOISNO_LAYERS],
    pub wa: f64,
    pub wdsrf: f64,
    pub wetwat: f64,
    pub scv: f64,
}

/// `tracer_hist_accumulate`：把步末水量与示踪物存量加进 history 累加器。
///
/// 读 `trc_aquifer_ref_water(ipatch)`（`state.aquifer_ref_water`）与
/// `trc_aquifer_ref_mass`（`pools.aquifer_ref_mass`）。
pub fn tracer_hist_accumulate(
    set: &TracerSet,
    state: &mut PatchTracerState,
    input: &HistAccumulateInput<'_>,
) {
    if set.is_empty() {
        return;
    }
    let snl = input.snl;
    let wliq = input.wliq_soisno;
    let wice = input.wice_soisno;

    // 冠层水。
    state.water_acc.ldew += input.ldew_rain + input.ldew_snow;
    for (itrc, tracer) in set.tracers.iter().enumerate() {
        if !tracer.uses_land_water_transport() {
            continue;
        }
        let pools = &state.pools[itrc];
        let acc = &mut state.acc[itrc];
        acc.ldew_mass = (acc.ldew_mass + pools.ldew_rain) + pools.ldew_snow;
    }

    // 土壤层 1..nl_soil。
    for j in 1..=SOIL_LAYERS as i32 {
        let slot = soisno_slot(j);
        let layer_water = wliq[slot] + wice[slot];
        state.water_acc.soil[(j - 1) as usize] += layer_water;
        for (itrc, tracer) in set.tracers.iter().enumerate() {
            if !tracer.uses_land_water_transport() {
                continue;
            }
            let pools = &state.pools[itrc];
            let layer_tracer = pools.wliq_soisno[slot] + pools.wice_soisno[slot];
            let acc = &mut state.acc[itrc];
            if tracer.is_nonvolatile_solute() && layer_water <= TRC_WATER_MIN_FOR_RATIO {
                acc.layer_dry_mass += layer_tracer;
            } else {
                acc.soil_mass[(j - 1) as usize] += layer_tracer;
            }
        }
    }

    // 雪层 snl+1..0：`jsnow = j - maxsnl`（1..5），这里存在 `jsnow - 1 = soisno_slot(j)`。
    if snl < 0 {
        for j in snl + 1..=0 {
            let slot = soisno_slot(j);
            state.water_acc.snow[slot] = (state.water_acc.snow[slot] + wliq[slot]) + wice[slot];
            for (itrc, tracer) in set.tracers.iter().enumerate() {
                if !tracer.uses_land_water_transport() {
                    continue;
                }
                let layer_water = wliq[slot] + wice[slot];
                let pools = &state.pools[itrc];
                let layer_tracer = pools.wliq_soisno[slot] + pools.wice_soisno[slot];
                let acc = &mut state.acc[itrc];
                if tracer.is_nonvolatile_solute() && layer_water <= TRC_WATER_MIN_FOR_RATIO {
                    acc.layer_dry_mass += layer_tracer;
                } else {
                    acc.snow_mass[slot] += layer_tracer;
                }
            }
        }
    }

    // 含水层：正储量与负亏欠分开；|wa| <= 1 mm 两边都不进。
    let wa = input.wa;
    let ref_water = state.aquifer_ref_water;
    let water = &mut state.water_acc;
    if wa > 1.0 {
        water.wa += wa;
    }
    if wa < -1.0 {
        water.wa_debt -= wa;
    }
    let actual_aquifer_water = wa + ref_water;
    if ref_water > 0.0 && actual_aquifer_water > TRC_WATER_MIN_FOR_RATIO {
        water.aquifer_actual += actual_aquifer_water;
    }
    if ref_water <= 0.0 && wa > 1.0 {
        water.aquifer_actual += wa;
    }
    water.wdsrf += input.wdsrf;
    water.wetwat += input.wetwat;
    // `trc_scv` 只是建层前的薄雪；有雪层后 scv 是总雪水当量，不与之配对。
    let thin_snow = snl == 0 && input.scv > TRC_TINY;
    if thin_snow {
        water.scv += input.scv;
    }
    let solid_lb = (snl + 1).max(-(MAX_SNOW_LAYERS as i32) + 1);
    for (itrc, tracer) in set.tracers.iter().enumerate() {
        if !tracer.uses_land_water_transport() {
            continue;
        }
        let pools = &state.pools[itrc];
        let acc = &mut state.acc[itrc];
        if tracer.is_isotope() && ref_water > 0.0 && actual_aquifer_water > TRC_WATER_MIN_FOR_RATIO
        {
            let actual_aquifer_mass = pools.wa + pools.aquifer_ref_mass;
            acc.aquifer_actual_mass += actual_aquifer_mass;
            if wa < -1.0 {
                acc.wa_debt_mass -= wa * actual_aquifer_mass / actual_aquifer_water;
            }
        } else {
            if tracer.is_isotope() && wa > 1.0 {
                acc.aquifer_actual_mass += pools.wa;
            }
            if wa > 1.0 {
                acc.wa_mass += pools.wa;
            }
            if wa < -1.0 {
                acc.wa_debt_mass += (-pools.wa).max(0.0);
            }
        }
        acc.wdsrf_mass += pools.wdsrf;
        acc.wetwat_mass += pools.wetwat;
        acc.surface_residue_mass += pools.surface_residue;
        acc.subsurface_residue_mass += pools.subsurface_residue;
        acc.solid_mass = (((acc.solid_mass + pools.canopy_solid) + pools.surface_solid)
            + pools.subsurface_solid)
            + pools.waterstorage_solid;
        for j in solid_lb..=SOIL_LAYERS as i32 {
            acc.solid_mass += pools.solid_soisno[soisno_slot(j)];
        }
        if thin_snow {
            acc.scv_mass += pools.scv;
        }
    }
}

/// `mass_to_delta`：`|water| > tiny` 且 `ref > tiny` 时 `((m/w)/ref - 1)*1000`；比值为负
/// （`< -tiny`）返回 `spval`；水量太小返回 0。
pub fn mass_to_delta(trc_mass: f64, water_mass: f64, ref_ratio: f64) -> f64 {
    if water_mass.abs() > TRC_TINY && ref_ratio > TRC_TINY {
        let r_sample = trc_mass / water_mass;
        if r_sample < -TRC_TINY {
            SPVAL
        } else {
            (r_sample / ref_ratio - 1.0) * 1000.0
        }
    } else {
        0.0
    }
}

/// `tracer_concentration_units`。
pub fn concentration_units(tracer: &TracerDescriptor) -> &'static str {
    match tracer.unit_kind.trim() {
        "ratio" => "R",
        "mass_fraction" => "kg/kg water",
        "volume_fraction" => "m3/m3 water",
        "species_owned" => "species-owned",
        _ => "tracer/water",
    }
}

/// 变量标识（与 [`TRACER_HISTORY_VARIABLES`] 一一对应）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TracerHistId {
    DeltaPrecip,
    DeltaRunoff,
    DeltaEvap,
    EvapMass,
    DeltaSoilevap,
    DeltaCanopyevap,
    DeltaSubl,
    DeltaWetlandEvap,
    DeltaTransp,
    DeltaTranspSrc,
    LeafDeltaE,
    LeafDeltaB,
    ConcPrecip,
    ConcRunoff,
    VaporExchange,
    SurfaceResidue,
    SubsurfaceResidue,
    LayerDryInventory,
    SolidInventory,
    ConcLdew,
    ConcSoisno,
    ConcSnowpack,
    DeltaSnowpack,
    ConcWa,
    ConcWaDebt,
    ConcWdsrf,
    ConcWetwat,
    ConcScv,
}

/// 变量的维度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TracerHistDims {
    /// 单点 `(patch, time)`；网格 `(lon, lat, time)`。
    Surface,
    /// 单点 `(soilsnow=15, patch, time)`；网格 `(soilsnow, lon, lat, time)`，
    /// 下标 `maxsnl+1..nl_soil`（-4..10）。
    SoilSnow,
}

/// 写出方式。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TracerHistKind {
    /// `write_history_tracer_delta_2d`：(质量, 水量) 对 → δ；patch 级门槛 `water > water_min`。
    Delta { water_min: f64 },
    /// `write_history_tracer_ratio_2d`：(质量, 水量) 对 → 比值；门槛 `|water| > tiny`。
    Ratio,
    /// `write_history_tracer_ratio_3d`：逐层 (质量, 水量) → 比值。
    LayerRatio,
    /// `write_history_variable_2d`（无 `acc_num`）：累加值 / `nac`，网格面积平均。
    Mean,
    /// `write_history_variable_2d(acc_num = 1)`：状态量本身，网格面积平均。
    AreaState,
}

/// 变量适用的示踪物。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TracerScope {
    /// `tracer_uses_delta_diagnostics`（同位素）。
    Isotope,
    /// 非同位素。
    NonIsotope,
    /// `tracer_is_nonvolatile_solute`。
    NonvolatileSolute,
    /// 所有走通用输运的示踪物。
    Transport,
}

/// `filter` 的 patch 条件（另与 `forcmask_pch`、`patchmask` 相与）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchFilter {
    /// `patchtype < 99`。
    Land,
    /// `patchtype == 2`（只给 `f_trc_conc_wetwat_*`）。
    Wetland,
}

impl PatchFilter {
    /// `filter(ip)`：`forcmask_ok` 是 `.not. has_missing_value .or. forcmask_pch(ip)`。
    pub fn admits(self, patchtype: i32, forcmask_ok: bool, patchmask: bool) -> bool {
        let base = match self {
            Self::Land => patchtype < 99,
            Self::Wetland => patchtype == 2,
        };
        base && forcmask_ok && patchmask
    }
}

/// long_name 的写法。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LongName {
    /// `'<text> (<name>)'`。
    Plain(&'static str),
    /// `'<text><ratio_word> (<name>)'`，`ratio_word` 同位素为 `heavy/total ratio`，否则 `concentration`。
    RatioWord(&'static str),
}

/// units 的写法。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Units {
    Fixed(&'static str),
    /// `trc_ratio_units`：同位素 `R`，否则 [`concentration_units`]。
    RatioUnits,
}

/// 一个示踪物 history 变量（每个适用的示踪物各一个，名字 `<prefix><name>`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TracerHistoryVariable {
    pub id: TracerHistId,
    pub prefix: &'static str,
    pub dims: TracerHistDims,
    /// `DEF_hist_vars` 的开关，任一为真即写（`xy_prc .or. xy_prl`）。
    pub hist_keys: &'static [&'static str],
    pub scope: TracerScope,
    pub kind: TracerHistKind,
    pub long_name: LongName,
    pub units: Units,
    pub patch_filter: PatchFilter,
    /// 所用累加器（上游名，说明用）。
    pub accumulators: &'static str,
}

impl TracerHistoryVariable {
    pub fn variable_name(&self, tracer: &TracerDescriptor) -> String {
        format!("{}{}", self.prefix, tracer.name.trim())
    }

    pub fn long_name(&self, tracer: &TracerDescriptor) -> String {
        match self.long_name {
            LongName::Plain(text) => format!("{text} ({})", tracer.name.trim()),
            LongName::RatioWord(text) => {
                let word = if tracer.is_isotope() {
                    "heavy/total ratio"
                } else {
                    "concentration"
                };
                format!("{text}{word} ({})", tracer.name.trim())
            }
        }
    }

    pub fn units(&self, tracer: &TracerDescriptor) -> &'static str {
        match self.units {
            Units::Fixed(units) => units,
            Units::RatioUnits => {
                if tracer.is_isotope() {
                    "R"
                } else {
                    concentration_units(tracer)
                }
            }
        }
    }

    /// 是否为这个示踪物写（只看示踪物类别；开关见 [`Self::enabled`]）。
    pub fn applies_to(&self, tracer: &TracerDescriptor) -> bool {
        if !tracer.uses_land_water_transport() {
            return false;
        }
        match self.scope {
            TracerScope::Isotope => tracer.is_isotope(),
            TracerScope::NonIsotope => !tracer.is_isotope(),
            TracerScope::NonvolatileSolute => tracer.is_nonvolatile_solute(),
            TracerScope::Transport => true,
        }
    }

    /// `is_hist`：`hist_var(key)` 查 `DEF_hist_vars%<key>`。
    pub fn enabled(&self, hist_var: impl Fn(&str) -> bool) -> bool {
        self.hist_keys.iter().any(|key| hist_var(key))
    }
}

#[allow(clippy::too_many_arguments)]
const fn var(
    id: TracerHistId,
    prefix: &'static str,
    dims: TracerHistDims,
    hist_keys: &'static [&'static str],
    scope: TracerScope,
    kind: TracerHistKind,
    long_name: LongName,
    units: Units,
    accumulators: &'static str,
) -> TracerHistoryVariable {
    TracerHistoryVariable {
        id,
        prefix,
        dims,
        hist_keys,
        scope,
        kind,
        long_name,
        units,
        patch_filter: PatchFilter::Land,
        accumulators,
    }
}

const FLUX_DELTA: TracerHistKind = TracerHistKind::Delta {
    water_min: TRC_FLUX_WATER_MIN_FOR_DELTA,
};
const STORE_DELTA: TracerHistKind = TracerHistKind::Delta {
    water_min: TRC_WATER_MIN_FOR_DELTA,
};
const PRECIP_KEYS: &[&str] = &["xy_prc", "xy_prl"];
const PERMIL: Units = Units::Fixed("permil");
const AMOUNT: Units = Units::Fixed("tracer amount/m2");

use LongName::{Plain, RatioWord};
use TracerHistDims::{SoilSnow, Surface};
use TracerHistId as Id;
use TracerHistKind::{AreaState, LayerRatio, Mean, Ratio};
use TracerScope::{Isotope, NonIsotope, NonvolatileSolute, Transport};

/// `tracer_hist_out` 逐示踪物写出的全部变量，按写出顺序。
#[rustfmt::skip]
pub const TRACER_HISTORY_VARIABLES: [TracerHistoryVariable; 28] = [
    var(Id::DeltaPrecip, "f_trc_delta_precip_", Surface, PRECIP_KEYS, Isotope, FLUX_DELTA,
        Plain("precipitation/deposition tracer delta"), PERMIL, "a_trc_precip / a_water_precip"),
    var(Id::DeltaRunoff, "f_trc_delta_runoff_", Surface, &["rnof"], Isotope, FLUX_DELTA,
        Plain("total runoff tracer delta"), PERMIL, "a_trc_rnof / a_water_rnof"),
    var(Id::DeltaEvap, "f_trc_delta_evap_", Surface, &["fevpa"], Isotope, FLUX_DELTA,
        Plain("evapotranspiration tracer delta"), PERMIL, "a_trc_evap / a_water_evap_gross"),
    var(Id::EvapMass, "f_trc_evap_mass_", Surface, &["fevpa"], Isotope, Mean,
        Plain("signed evapotranspiration tracer mass"), AMOUNT, "a_trc_evap / nac"),
    var(Id::DeltaSoilevap, "f_trc_delta_soilevap_", Surface, &["fevpa"], Isotope, FLUX_DELTA,
        Plain("soil/surface evaporation tracer delta"), PERMIL, "a_trc_soilevap / a_water_soilevap"),
    var(Id::DeltaCanopyevap, "f_trc_delta_canopyevap_", Surface, &["fevpa"], Isotope, FLUX_DELTA,
        Plain("canopy evaporation tracer delta"), PERMIL, "a_trc_canopyevap / a_water_canopyevap"),
    var(Id::DeltaSubl, "f_trc_delta_subl_", Surface, &["fevpa"], Isotope, FLUX_DELTA,
        Plain("sublimation tracer delta"), PERMIL, "a_trc_subl / a_water_subl"),
    var(Id::DeltaWetlandEvap, "f_trc_delta_wetland_evap_", Surface, &["fevpa"], Isotope, FLUX_DELTA,
        Plain("wetland evaporation/sublimation tracer delta"), PERMIL,
        "a_trc_wetland_evap / a_water_wetland_evap"),
    var(Id::DeltaTransp, "f_trc_delta_transp_", Surface, &["etr"], Isotope, FLUX_DELTA,
        Plain("transpiration tracer delta"), PERMIL, "a_trc_transp / a_water_transp"),
    var(Id::DeltaTranspSrc, "f_trc_delta_transp_src_", Surface, &["etr"], Isotope, FLUX_DELTA,
        Plain("transpiration source/xylem tracer delta"), PERMIL, "a_trc_transp_src / a_water_transp"),
    var(Id::LeafDeltaE, "f_trc_leaf_delta_e_", Surface, &["etr"], Isotope, AreaState,
        Plain("leaf evaporation-site NSS delta, area-weighted state"), PERMIL,
        "trc_leaf_delta_e (state, not accumulated)"),
    var(Id::LeafDeltaB, "f_trc_leaf_delta_b_", Surface, &["etr"], Isotope, STORE_DELTA,
        Plain("bulk leaf-water NSS delta, leaf-water-mole weighted"), PERMIL,
        "R(trc_leaf_delta_b)*trc_leaf_water_moles / trc_leaf_water_moles (state)"),
    var(Id::ConcPrecip, "f_trc_conc_precip_", Surface, PRECIP_KEYS, NonIsotope, Ratio,
        Plain("precipitation/deposition tracer concentration"), Units::RatioUnits,
        "a_trc_precip / a_water_precip"),
    var(Id::ConcRunoff, "f_trc_conc_runoff_", Surface, &["rnof"], NonIsotope, Ratio,
        Plain("total runoff tracer concentration"), Units::RatioUnits, "a_trc_rnof / a_water_rnof"),
    var(Id::VaporExchange, "f_trc_vapor_exchange_", Surface, &["fevpa"], Isotope, Mean,
        Plain("zero-water-flux equilibrium vapour exchange, signed"), AMOUNT,
        "a_trc_vapor_exchange / nac"),
    var(Id::SurfaceResidue, "f_trc_surface_residue_", Surface, &["wdsrf"], NonvolatileSolute, Mean,
        Plain("immobile surface tracer residue"), AMOUNT, "a_trc_surface_residue_mass / nac"),
    var(Id::SubsurfaceResidue, "f_trc_subsurface_residue_", Surface, &["wa"], NonvolatileSolute,
        Mean, Plain("immobile subsurface tracer residue"), AMOUNT,
        "a_trc_subsurface_residue_mass / nac"),
    var(Id::LayerDryInventory, "f_trc_layer_dry_inventory_", Surface, &["wliq_soisno"],
        NonvolatileSolute, Mean, Plain("dry snow/soil layer tracer inventory"), AMOUNT,
        "a_trc_layer_dry_mass / nac"),
    var(Id::SolidInventory, "f_trc_solid_inventory_", Surface, &["wliq_soisno"], NonvolatileSolute,
        Mean, Plain("precipitated solid tracer inventory"), AMOUNT, "a_trc_solid_mass / nac"),
    var(Id::ConcLdew, "f_trc_conc_ldew_", Surface, &["ldew"], Transport, Ratio,
        RatioWord("canopy tracer "), Units::RatioUnits, "a_trc_ldew_mass / a_water_ldew"),
    var(Id::ConcSoisno, "f_trc_conc_soisno_", SoilSnow, &["wliq_soisno"], Transport, LayerRatio,
        RatioWord("soil/snow layer tracer "), Units::RatioUnits,
        "a_trc_snow_mass/a_water_snow (j<=0), a_trc_soil_mass/a_water_soil (j>=1)"),
    var(Id::ConcSnowpack, "f_trc_conc_snowpack_", Surface, &["scv"], Transport, Ratio,
        RatioWord("total snowpack tracer "), Units::RatioUnits,
        "(a_trc_scv_mass + sum a_trc_snow_mass) / (a_water_scv + sum a_water_snow)"),
    var(Id::DeltaSnowpack, "f_trc_delta_snowpack_", Surface, &["scv"], Isotope, STORE_DELTA,
        Plain("total snowpack tracer delta"), PERMIL,
        "(a_trc_scv_mass + sum a_trc_snow_mass) / (a_water_scv + sum a_water_snow)"),
    var(Id::ConcWa, "f_trc_conc_wa_", Surface, &["wa"], Transport, Ratio,
        RatioWord("aquifer tracer "), Units::RatioUnits,
        "isotope: a_trc_aquifer_actual_mass / a_water_aquifer_actual; else a_trc_wa_mass / a_water_wa"),
    var(Id::ConcWaDebt, "f_trc_conc_wa_debt_", Surface, &["wa"], Transport, Ratio,
        RatioWord("aquifer debt tracer "), Units::RatioUnits,
        "a_trc_wa_debt_mass / a_water_wa_debt"),
    var(Id::ConcWdsrf, "f_trc_conc_wdsrf_", Surface, &["wdsrf"], Transport, Ratio,
        RatioWord("surface water tracer "), Units::RatioUnits, "a_trc_wdsrf_mass / a_water_wdsrf"),
    TracerHistoryVariable {
        patch_filter: PatchFilter::Wetland,
        ..var(Id::ConcWetwat, "f_trc_conc_wetwat_", Surface, &["wetwat"], Transport, Ratio,
            RatioWord("wetland pool tracer "), Units::RatioUnits,
            "a_trc_wetwat_mass / a_water_wetwat")
    },
    var(Id::ConcScv, "f_trc_conc_scv_", Surface, &["scv"], Transport, Ratio,
        RatioWord("thin-snow (scv) tracer "), Units::RatioUnits, "a_trc_scv_mass / a_water_scv"),
];

/// 查表。
pub fn history_variable(id: TracerHistId) -> &'static TracerHistoryVariable {
    TRACER_HISTORY_VARIABLES
        .iter()
        .find(|variable| variable.id == id)
        .expect("every id has a table entry")
}

/// 一个 patch 对一个二维变量的原始取值（写出前、未做门槛）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PatchTerm {
    /// (示踪物质量, 水量) 累加对。
    Pair { mass: f64, water: f64 },
    /// 已除以 `nac`（或 `acc_num`）的标量，`spval` 表示缺测。
    Scalar(f64),
}

/// 二维变量在一个 patch 上的原始取值。`nac` 是 `MOD_Vars_1DAccFluxes` 的累加次数。
/// 三维的 `f_trc_conc_soisno_*` 用 [`soisno_layer_pairs`]。
pub fn patch_term(
    variable: &TracerHistoryVariable,
    tracer: &TracerDescriptor,
    itrc: usize,
    state: &PatchTracerState,
    nac: f64,
) -> PatchTerm {
    let acc = &state.acc[itrc];
    let water = &state.water_acc;
    let pair = |mass: f64, water: f64| PatchTerm::Pair { mass, water };
    let mean = |value: f64| PatchTerm::Scalar(if value != SPVAL { value / nac } else { value });
    match variable.id {
        Id::DeltaPrecip | Id::ConcPrecip => pair(acc.precip, acc.water_precip),
        Id::DeltaRunoff | Id::ConcRunoff => pair(acc.rnof, acc.water_rnof),
        Id::DeltaEvap => pair(acc.evap, acc.water_evap_gross),
        Id::DeltaSoilevap => pair(acc.soilevap, acc.water_soilevap),
        Id::DeltaCanopyevap => pair(acc.canopyevap, acc.water_canopyevap),
        Id::DeltaSubl => pair(acc.subl, acc.water_subl),
        Id::DeltaWetlandEvap => pair(acc.wetland_evap, acc.water_wetland_evap),
        Id::DeltaTransp => pair(acc.transp, acc.water_transp),
        Id::DeltaTranspSrc => pair(acc.transp_src, acc.water_transp),
        Id::EvapMass => mean(acc.evap),
        Id::VaporExchange => mean(acc.vapor_exchange),
        Id::SurfaceResidue => mean(acc.surface_residue_mass),
        Id::SubsurfaceResidue => mean(acc.subsurface_residue_mass),
        Id::LayerDryInventory => mean(acc.layer_dry_mass),
        Id::SolidInventory => mean(acc.solid_mass),
        Id::LeafDeltaE => {
            let delta = state.pools[itrc].leaf_delta_e;
            let value = if delta != SPVAL && delta.abs() <= TRC_DELTA_SANITY_MAX {
                delta
            } else {
                SPVAL
            };
            // `acc_num = 1`：`acc/1`。
            PatchTerm::Scalar(if value != SPVAL { value / 1.0 } else { value })
        }
        Id::LeafDeltaB => {
            let pools = &state.pools[itrc];
            let delta = pools.leaf_delta_b;
            if delta != SPVAL && delta.abs() <= TRC_DELTA_SANITY_MAX && pools.leaf_water_moles > 0.0
            {
                let leaf_r = delta.mul_add(1.0e-3, 1.0) * tracer.ref_ratio;
                pair(leaf_r * pools.leaf_water_moles, pools.leaf_water_moles)
            } else {
                pair(SPVAL, 0.0)
            }
        }
        Id::ConcLdew => pair(acc.ldew_mass, water.ldew),
        Id::ConcSnowpack | Id::DeltaSnowpack => {
            let mut mass = acc.scv_mass;
            let mut snow_water = water.scv;
            for jsnow in 0..MAX_SNOW_LAYERS {
                mass += acc.snow_mass[jsnow];
                snow_water += water.snow[jsnow];
            }
            pair(mass, snow_water)
        }
        Id::ConcWa => {
            if tracer.is_isotope() {
                pair(acc.aquifer_actual_mass, water.aquifer_actual)
            } else {
                pair(acc.wa_mass, water.wa)
            }
        }
        Id::ConcWaDebt => pair(acc.wa_debt_mass, water.wa_debt),
        Id::ConcWdsrf => pair(acc.wdsrf_mass, water.wdsrf),
        Id::ConcWetwat => pair(acc.wetwat_mass, water.wetwat),
        Id::ConcScv => pair(acc.scv_mass, water.scv),
        Id::ConcSoisno => panic!("f_trc_conc_soisno_* is 3-D; use soisno_layer_pairs"),
    }
}

/// `tracer_hist_out` 为 `f_trc_conc_soisno_*` 拼的逐层 (质量, 水量)，层 -4..10；
/// 累计水量 `<= tiny` 的层两者都是 `spval`。
pub fn soisno_layer_pairs(itrc: usize, state: &PatchTracerState) -> [(f64, f64); SOISNO_LAYERS] {
    let acc = &state.acc[itrc];
    let water = &state.water_acc;
    let mut out = [(SPVAL, SPVAL); SOISNO_LAYERS];
    for j in -(MAX_SNOW_LAYERS as i32) + 1..=0 {
        let slot = soisno_slot(j);
        if water.snow[slot] > TRC_TINY {
            out[slot] = (acc.snow_mass[slot], water.snow[slot]);
        }
    }
    for j in 1..=SOIL_LAYERS as i32 {
        let k = (j - 1) as usize;
        if water.soil[k] > TRC_TINY {
            out[soisno_slot(j)] = (acc.soil_mass[k], water.soil[k]);
        }
    }
    out
}

/// 单点比值（`write_history_tracer_ratio_2d`，`HistForm == 'Single'`）。
pub fn single_ratio(mass: f64, water: f64) -> f64 {
    if water.abs() > TRC_TINY && mass != SPVAL {
        mass / water
    } else {
        SPVAL
    }
}

/// 单点 δ（`write_history_tracer_delta_2d`）。
pub fn single_delta(mass: f64, water: f64, ref_ratio: f64, water_min: f64) -> f64 {
    if water > water_min && mass != SPVAL {
        let delta = mass_to_delta(mass, water, ref_ratio);
        if delta != SPVAL && delta.abs() <= TRC_DELTA_SANITY_MAX {
            return delta;
        }
    }
    SPVAL
}

/// 单点逐层比值（`write_history_tracer_ratio_3d` 先乘 `nac`，`single_write_3d` 再除）。
pub fn single_layer_ratio(mass: f64, water: f64, nac: f64) -> f64 {
    if water.abs() > TRC_TINY && mass != SPVAL {
        let scaled = mass / water * nac;
        if scaled != SPVAL {
            scaled / nac
        } else {
            scaled
        }
    } else {
        SPVAL
    }
}

/// 单点二维变量写进 `patch` 维的值。`patch_ok` 是该变量的 `filter(ip)`
/// （[`PatchFilter::admits`]）；不过滤的 patch 写 `spval`。
pub fn single_point_value(
    variable: &TracerHistoryVariable,
    tracer: &TracerDescriptor,
    itrc: usize,
    state: &PatchTracerState,
    nac: f64,
    patch_ok: bool,
) -> f64 {
    let value = match (
        variable.kind,
        patch_term(variable, tracer, itrc, state, nac),
    ) {
        (TracerHistKind::Delta { water_min }, PatchTerm::Pair { mass, water }) => {
            single_delta(mass, water, tracer.ref_ratio, water_min)
        }
        (TracerHistKind::Ratio, PatchTerm::Pair { mass, water }) => single_ratio(mass, water),
        (_, PatchTerm::Scalar(value)) => value,
        (kind, term) => unreachable!("kind {kind:?} with term {term:?}"),
    };
    if patch_ok {
        value
    } else {
        SPVAL
    }
}

/// 单点 `f_trc_conc_soisno_*`（`soilsnow` 维 -4..10）。
pub fn single_point_soisno(
    itrc: usize,
    state: &PatchTracerState,
    nac: f64,
    patch_ok: bool,
) -> [f64; SOISNO_LAYERS] {
    let pairs = soisno_layer_pairs(itrc, state);
    let mut out = [SPVAL; SOISNO_LAYERS];
    if patch_ok {
        for (slot, (mass, water)) in pairs.iter().enumerate() {
            out[slot] = single_layer_ratio(*mass, *water, nac);
        }
    }
    out
}

/// 网格单元的累加器（`mp2g_hist%pset2grid(..., spv = spval, msk = filter)` 与
/// `get_sumarea`）。按 patch 序、再按 patch 的各部分序调用 `add_*`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridCell {
    /// 示踪物质量（或 `Mean` 变量的值）的 `Σ area*v`；无贡献时 `spval`。
    pub mass: f64,
    /// 水量的 `Σ area*v`；无贡献时 `spval`。
    pub water: f64,
    /// `sumarea`：通过陆面 `filter` 的 patch 面积之和（只给 `Mean`/`AreaState` 用）。
    pub sumarea: f64,
}

impl Default for GridCell {
    fn default() -> Self {
        Self {
            mass: SPVAL,
            water: SPVAL,
            sumarea: 0.0,
        }
    }
}

/// `pset2grid` 的一次贡献：值为 `spval` 跳过；格点仍为 `spval` 时直接取 `v*area`。
fn pset_add(cell: &mut f64, value: f64, area: f64) {
    if value == SPVAL {
        return;
    }
    let contribution = value / 1.0 * area;
    *cell = if *cell != SPVAL {
        *cell + contribution
    } else {
        contribution
    };
}

impl GridCell {
    /// `get_sumarea`：陆面 `filter` 通过的 patch 的面积。
    pub fn add_area(&mut self, area: f64) {
        self.sumarea += area;
    }

    /// 二维变量的一次 patch 贡献（`patch_ok` 为该变量的 `filter`）。
    pub fn add(
        &mut self,
        variable: &TracerHistoryVariable,
        term: PatchTerm,
        area: f64,
        patch_ok: bool,
    ) {
        if !patch_ok {
            return;
        }
        match (variable.kind, term) {
            // 比值/δ：不合格的 patch 以 0 入图（映射向量初值是 0，不是 spval）。
            (TracerHistKind::Ratio, PatchTerm::Pair { mass, water }) => {
                let (m, w) = if water.abs() > TRC_TINY && mass != SPVAL {
                    (mass, water)
                } else {
                    (0.0, 0.0)
                };
                pset_add(&mut self.mass, m, area);
                pset_add(&mut self.water, w, area);
            }
            (TracerHistKind::Delta { water_min }, PatchTerm::Pair { mass, water }) => {
                let (m, w) = if water > water_min && mass != SPVAL {
                    (mass, water)
                } else {
                    (0.0, 0.0)
                };
                pset_add(&mut self.mass, m, area);
                pset_add(&mut self.water, w, area);
            }
            (_, PatchTerm::Scalar(value)) => pset_add(&mut self.mass, value, area),
            (kind, term) => unreachable!("kind {kind:?} with term {term:?}"),
        }
    }

    /// 三维 `f_trc_conc_soisno_*` 的一层贡献（映射初值是 spval：不合格的层不入图）。
    pub fn add_layer(&mut self, mass: f64, water: f64, area: f64, patch_ok: bool) {
        if !patch_ok || !(water > TRC_TINY && mass != SPVAL) {
            return;
        }
        pset_add(&mut self.mass, mass, area);
        pset_add(&mut self.water, water, area);
    }

    /// 写进网格的值。
    pub fn finish(&self, variable: &TracerHistoryVariable, ref_ratio: f64) -> f64 {
        match variable.kind {
            TracerHistKind::Ratio => {
                if self.water != SPVAL && self.mass != SPVAL && self.water.abs() > TRC_TINY {
                    self.mass / self.water
                } else {
                    SPVAL
                }
            }
            TracerHistKind::LayerRatio => {
                if self.water != SPVAL && self.mass != SPVAL && self.water > TRC_TINY {
                    self.mass / self.water
                } else {
                    SPVAL
                }
            }
            TracerHistKind::Delta { .. } => {
                if self.water > TRC_TINY {
                    let delta = mass_to_delta(self.mass, self.water, ref_ratio);
                    if delta != SPVAL && delta.abs() <= TRC_DELTA_SANITY_MAX {
                        return delta;
                    }
                }
                SPVAL
            }
            TracerHistKind::Mean | TracerHistKind::AreaState => {
                if self.sumarea > GRID_MIN_SUMAREA {
                    if self.mass != SPVAL {
                        self.mass / self.sumarea
                    } else {
                        self.mass
                    }
                } else {
                    SPVAL
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "hist_tests.rs"]
mod tests;

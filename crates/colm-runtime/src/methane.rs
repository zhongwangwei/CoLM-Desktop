//! CH4 provider 的运行时接线：从 CH4 示踪物的参数文件读 `&nl_colm_methane_parameter`，
//! 组装 patch 的静态量，每步在 BGC 之后调用 [`colm_core::methane::driver::soil_step`]。

use anyhow::{bail, Context, Result};
use colm_core::methane::config::{FieldValue, MethaneParameters};
use colm_core::methane::driver::{HostInputs, MethaneSite};
use colm_namelist::{Document, Segment, Value};

/// 一次运行共享的甲烷配置。
#[derive(Debug, Clone)]
pub struct MethaneSetup {
    pub params: MethaneParameters,
    /// `DEF_wetland_finundation_scheme`（`configure_methane_inundation_mode` 的结果）。
    pub scheme: i32,
    pub dynamic_wetland: bool,
}

/// 注册的 CH4 provider 示踪物（`gas` 类、名字 `CH4`/`METHANE`）。
pub fn is_methane_tracer(tracer: &colm_core::tracer::TracerDescriptor) -> bool {
    tracer.category == "gas"
        && matches!(
            tracer.name.trim().to_ascii_uppercase().as_str(),
            "CH4" | "METHANE"
        )
}

/// `ch4_reactive_init` 的配置部分：找到 CH4 示踪物、读它的参数文件、解析淹没方案。
/// 没开示踪物或没有 CH4 示踪物时返回 `None`。`grid_river` 是内核有没有网格河湖汇流。
pub fn setup_from_document(document: &Document, grid_river: bool) -> Result<Option<MethaneSetup>> {
    let Some(set) = crate::tracer::tracer_set_from_document(document)? else {
        return Ok(None);
    };
    let Some(index) = set.tracers.iter().position(is_methane_tracer) else {
        return Ok(None);
    };
    // CH4 与输运示踪物同开时，甲烷量要插在示踪物事务中间（`trc_forcing_cache_*` 之前），尚未接。
    anyhow::ensure!(
        set.transport_indices().next().is_none(),
        "methane together with water-transported tracers is not ported to the Rust runtime yet"
    );
    let files = crate::physics::text(document, "DEF_TRACER_PARAM_FILES")?;
    let path = colm_core::tracer::descriptor::param_file_for_index(&files, &set.tracers, index)?
        .context(
            "the CH4 tracer has no parameter file; methane needs &nl_colm_methane_parameter",
        )?;
    let mut params = read_parameters(&path)?;
    let dynamic_wetland = crate::physics::logical(document, "DEF_USE_Dynamic_Wetland")?;
    let mode = params.configure_inundation(dynamic_wetland, grid_river)?;
    Ok(Some(MethaneSetup {
        params,
        scheme: mode.scheme,
        dynamic_wetland,
    }))
}

/// `read_methane_namelist`：文件里没有这一组时上游 `read` 报错停机。
pub fn read_parameters(path: &str) -> Result<MethaneParameters> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("methane parameter file does not exist: {path}"))?;
    let mut lines = source.lines();
    lines
        .by_ref()
        .find(|line| {
            line.trim_start()
                .to_ascii_lowercase()
                .starts_with("&nl_colm_methane_parameter")
        })
        .with_context(|| format!("no &nl_colm_methane_parameter in {path}"))?;
    let mut group = String::from("&nl_colm_methane_parameter\n");
    for line in lines {
        let code = line.split('!').next().unwrap_or("").trim();
        if code == "/" || code.eq_ignore_ascii_case("&end") {
            break;
        }
        group.push_str(line);
        group.push('\n');
    }
    group.push_str("/\n");
    let document = colm_namelist::parse(&group)
        .with_context(|| format!("invalid &nl_colm_methane_parameter in {path}"))?;
    let mut entries = Vec::new();
    for item in &document.items {
        let colm_namelist::document::Item::Entry(entry) = item else {
            continue;
        };
        let (owner, field) = match entry.path.segments.as_slice() {
            [Segment::Field(owner), Segment::Member(field)] => (owner.clone(), field.clone()),
            _ => bail!(
                "invalid &nl_colm_methane_parameter entry {} in {path}",
                entry.path
            ),
        };
        let value = match &entry.value {
            Value::Bool(b) => FieldValue::Logical(*b),
            Value::Int(i) => FieldValue::Int(*i),
            Value::Real { .. } => FieldValue::Real(
                entry
                    .value
                    .as_f64()
                    .with_context(|| format!("{} in {path} is not a real", entry.path))?,
            ),
            Value::Str(s) => FieldValue::Text(s.clone()),
            Value::List(_) => bail!("{} in {path} must be a scalar", entry.path),
        };
        entries.push((owner, field, value));
    }
    MethaneParameters::from_entries(
        entries
            .iter()
            .map(|(o, f, v)| (o.as_str(), f.as_str(), v.clone())),
    )
    .with_context(|| format!("invalid &nl_colm_methane_parameter in {path}"))
}

/// 一步的宿主量（`ch4_impl_soil_step` 传给 `methane_driver` 的那些时间变量）。
pub struct HostArrays {
    pub t_soisno: [f64; colm_core::methane::physics::SOISNO],
    pub wliq_soisno: [f64; colm_core::methane::physics::SOISNO],
    pub wice_soisno: [f64; colm_core::methane::physics::SOISNO],
    pub smp: [f64; colm_core::methane::physics::NL_SOIL],
    pub rootr: [f64; colm_core::methane::physics::NL_SOIL],
}

impl HostArrays {
    pub fn from_state(
        state: &colm_core::StandardLctSnowSoilState,
        output: &colm_core::StandardLctSnowSoilOutput,
    ) -> Result<Self> {
        use colm_core::tracer::step::pack_soisno;
        let nl = colm_core::methane::physics::NL_SOIL;
        let fixed = |values: &[f64], name: &str| -> Result<[f64; 10]> {
            values
                .get(..nl)
                .and_then(|slice| <[f64; 10]>::try_from(slice).ok())
                .with_context(|| format!("{name} has fewer than {nl} soil layers"))
        };
        Ok(Self {
            t_soisno: pack_soisno(&state.snow.temperature_k, &state.soil_temperature_k),
            wliq_soisno: pack_soisno(
                &state.snow.liquid_water_kg_m2,
                &state.soil_water.liquid_water_kg_m2,
            ),
            wice_soisno: pack_soisno(
                &state.snow.ice_water_kg_m2,
                &state.soil_water.ice_water_kg_m2,
            ),
            smp: fixed(&state.soil_water.matric_potential_mm, "smp")?,
            rootr: fixed(&output.energy.root_uptake.layer_fraction, "rootr")?,
        })
    }
}

/// [`HostInputs`] 的其余标量。
#[allow(clippy::too_many_arguments)]
pub fn host_inputs<'a>(
    arrays: &'a HostArrays,
    idate: [i32; 3],
    deltim: f64,
    state: &colm_core::StandardLctSnowSoilState,
    output: &colm_core::StandardLctSnowSoilOutput,
    forcing: &colm_core::RuntimeForcing,
    partial_pressures_pa: (f64, f64),
    pft: colm_core::methane::bgc_link::PftInputs<'a>,
    dynamic_wetland: bool,
) -> HostInputs<'a> {
    let leaf = &output.energy.leaf;
    HostInputs {
        idate,
        deltim,
        t_soisno: &arrays.t_soisno,
        wliq_soisno: &arrays.wliq_soisno,
        wice_soisno: &arrays.wice_soisno,
        // CoLMMAIN 末尾雪层合并后的 `t_grnd = t_soisno(lb)`，不是热力学步内的地表温度。
        t_grnd: state.surface_temperature_k(),
        forc_t: forcing.air_temperature_k,
        forc_pbot: forcing.bottom_pressure_pa,
        forc_po2m: partial_pressures_pa.1,
        forc_pco2m: partial_pressures_pa.0,
        ustar: leaf.friction_velocity_m_s,
        fq: leaf.moisture_similarity,
        zwt: state.soil_water.water_table_depth_m,
        snowdp: state.snow.depth_m,
        etr: leaf.transpiration_kg_m2_s,
        wdsrf: state.soil_water.surface_water_mm,
        wetwat: state.soil_water.wetland_water_mm,
        smp: &arrays.smp,
        lai: state.energy.canopy.leaf_area_index,
        sai: state.energy.canopy.stem_area_index,
        rootr: &arrays.rootr,
        frcsat: output.water.soil.saturated_fraction,
        pft,
        dynamic_wetland,
    }
}

/// patch 的静态量（`MethaneSite`）。`root_fraction` 是地类根分布 `rootfr_lc(:, patchclass)`。
#[allow(clippy::too_many_arguments)]
pub fn site(
    patchtype: i32,
    patchclass: i32,
    patchlatr: f64,
    slpratio: f64,
    root_fraction: &[f64],
    z_soi: &[f64],
    dz_soi: &[f64],
    zi_soi: &[f64],
    bsw: &[f64],
    porsl: &[f64],
    organic_max: f64,
    wetwatmax: f64,
) -> Result<MethaneSite> {
    let take = |values: &[f64], name: &str| -> Result<[f64; 10]> {
        values
            .get(..10)
            .and_then(|slice| <[f64; 10]>::try_from(slice).ok())
            .with_context(|| format!("{name} has fewer than 10 soil layers"))
    };
    Ok(MethaneSite {
        patchtype,
        patchclass,
        dlat: patchlatr * 180.0 / std::f64::consts::PI,
        slpratio,
        rootfr: take(root_fraction, "rootfr")?,
        z_soi: take(z_soi, "z_soi")?,
        dz_soi: take(dz_soi, "dz_soi")?,
        zi_soi: take(zi_soi, "zi_soi")?,
        bsw: take(bsw, "bsw")?,
        porsl: take(porsl, "porsl")?,
        organic_max,
        wetwatmax,
    })
}

#[path = "methane_restart_generated.rs"]
mod restart_fields;

/// `METHANE_RESTART_SCHEMA_VERSION`。
const RESTART_SCHEMA: f64 = 5.0;

/// `methane_history_selector_fingerprint`：`ch4_history_vars` 去空白、转小写的 djb 式滚动哈希
/// （`ishftc(h, 7, 52)` 再异或字符码，掩到低 52 位），再并入两个开关。
pub fn history_selector_fingerprint(params: &MethaneParameters) -> i64 {
    const MASK: i64 = 0x000F_FFFF_FFFF_FFFF;
    let roll = |h: i64, code: i64| {
        // `iand(ieor(ishftc(h, 7, 52), code), mask)`：低 52 位循环左移 7 位，异或后再掩码。
        let low = h & MASK;
        let rotated = ((low << 7) | (low >> 45)) & MASK;
        (rotated ^ code) & MASK
    };
    let mut h: i64 = 5381;
    for byte in params.methane.ch4_history_vars.trim_end().bytes() {
        let mut code = i64::from(byte);
        if code <= 32 {
            continue;
        }
        if (i64::from(b'A')..=i64::from(b'Z')).contains(&code) {
            code += 32;
        }
        h = roll(h, code);
    }
    h = roll(
        h,
        if params.methane.write_ch4_history {
            257
        } else {
            256
        },
    );
    roll(
        h,
        if params.methane.use_microbial_pools {
            513
        } else {
            512
        },
    )
}

/// `ch4_reactive_write_restart`：把甲烷状态与 history 累加量追加进已写好空示踪物事务的
/// 陆面时间重启（`patch`、`soil` 维已在）。
pub fn write_restart(
    path: &std::path::Path,
    setup: &MethaneSetup,
    patches: &[(
        &colm_core::methane::driver::MethanePatch,
        &colm_core::methane::driver::CoreAccumulator,
    )],
) -> Result<()> {
    use crate::tracer::{ensure_dimension, put_array_f64};
    use colm_core::methane::config::{COMP_RICE, COMP_SOIL};
    use colm_core::methane::physics::{NL_SOIL, SPVAL};
    let mut file =
        netcdf::append(path).with_context(|| format!("cannot reopen {}", path.display()))?;
    let n = patches.len();
    ensure_dimension(&mut file, "soil", NL_SOIL)?;
    let scalar = |file: &mut netcdf::FileMut, name: &str, f: &dyn Fn(usize) -> f64| -> Result<()> {
        let values: Vec<f64> = (0..n).map(f).collect();
        put_array_f64(file, name, &["patch"], &values)
    };
    let layered = |file: &mut netcdf::FileMut,
                   name: &str,
                   f: &dyn Fn(usize) -> [f64; NL_SOIL]|
     -> Result<()> {
        let values: Vec<f64> = (0..n).flat_map(f).collect();
        put_array_f64(file, name, &["patch", "soil"], &values)
    };
    scalar(&mut file, "ch4_restart_complete", &|_| 0.0)?;
    scalar(&mut file, "ch4_restart_schema", &|_| RESTART_SCHEMA)?;
    scalar(&mut file, "ch4_history_accumulation_mode", &|_| {
        f64::from(setup.params.history_accumulation_mode())
    })?;
    scalar(&mut file, "ch4_feature_microbe_pools", &|_| {
        if setup.params.methane.use_microbial_pools {
            1.0
        } else {
            0.0
        }
    })?;
    let p = |i: usize| patches[i].0;
    let soil = |i: usize| &p(i).components[COMP_SOIL];
    let rice = |i: usize| &p(i).components[COMP_RICE];
    // patch 级聚合状态：土壤 patch 上与土壤分量相同，湿地上是唯一在推进的那份。
    let agg = |i: usize| &p(i).aggregate;
    let last = |i: usize| p(i).last.unwrap_or_default();
    // 聚合量（`aggregate_methane_columns`，土壤 patch 取土壤分量）。冷启动尚未走步时取分配时的值。
    let agg_layers =
        |i: usize,
         f: fn(&colm_core::methane::column::ColumnResult) -> [f64; NL_SOIL],
         cold: f64| { p(i).last.map_or([cold; NL_SOIL], |r| f(&r)) };
    layered(&mut file, "ch4_conc_o2", &|i| {
        agg_layers(i, |r| r.conc_o2, 1.0)
    })?;
    layered(&mut file, "ch4_conc_methane", &|i| {
        agg_layers(i, |r| r.conc_methane, 1.0e-6)
    })?;
    scalar(&mut file, "ch4_totcol_methane", &|i| p(i).totcol_methane)?;
    scalar(&mut file, "ch4_grnd_methane_cond", &|i| {
        p(i).grnd_methane_cond
    })?;
    layered(&mut file, "ch4_conc_o2_unsat", &|i| agg(i).conc_o2_unsat)?;
    layered(&mut file, "ch4_conc_o2_sat", &|i| agg(i).conc_o2_sat)?;
    layered(&mut file, "ch4_conc_ch4_unsat", &|i| {
        agg(i).conc_methane_unsat
    })?;
    layered(&mut file, "ch4_conc_ch4_sat", &|i| agg(i).conc_methane_sat)?;
    layered(&mut file, "ch4_conc_o2_unsat_soil", &|i| {
        soil(i).conc_o2_unsat
    })?;
    layered(&mut file, "ch4_conc_o2_unsat_rice", &|i| {
        rice(i).conc_o2_unsat
    })?;
    layered(&mut file, "ch4_conc_o2_sat_soil", &|i| soil(i).conc_o2_sat)?;
    layered(&mut file, "ch4_conc_o2_sat_rice", &|i| rice(i).conc_o2_sat)?;
    layered(&mut file, "ch4_conc_ch4_unsat_soil", &|i| {
        soil(i).conc_methane_unsat
    })?;
    layered(&mut file, "ch4_conc_ch4_unsat_rice", &|i| {
        rice(i).conc_methane_unsat
    })?;
    layered(&mut file, "ch4_conc_ch4_sat_soil", &|i| {
        soil(i).conc_methane_sat
    })?;
    layered(&mut file, "ch4_conc_ch4_sat_rice", &|i| {
        rice(i).conc_methane_sat
    })?;
    scalar(&mut file, "ch4_totcol_methane_unsat", &|i| {
        last(i).unsat.totcol
    })?;
    scalar(&mut file, "ch4_totcol_methane_sat", &|i| last(i).sat.totcol)?;
    let default_cond = setup.params.methane.grnd_methane_cond_default;
    scalar(&mut file, "ch4_grnd_methane_cond_unsat", &|i| {
        p(i).last.map_or(default_cond, |r| r.unsat.grnd_cond)
    })?;
    scalar(&mut file, "ch4_grnd_methane_cond_sat", &|i| {
        p(i).last.map_or(default_cond, |r| r.sat.grnd_cond)
    })?;
    layered(&mut file, "ch4_conc_o2_lake", &|_| [1.0; NL_SOIL])?;
    layered(&mut file, "ch4_conc_ch4_lake", &|_| [0.0; NL_SOIL])?;
    scalar(&mut file, "ch4_totcol_lake", &|_| 0.0)?;
    scalar(&mut file, "ch4_grnd_methane_cond_lake", &|_| default_cond)?;
    for name in [
        "ch4_lake_water_ch4_stock",
        "ch4_lake_water_o2_stock",
        "ch4_lake_frozen_ch4_stock",
        "ch4_lake_frozen_o2_stock",
    ] {
        scalar(&mut file, name, &|_| 0.0)?;
    }
    scalar(&mut file, "ch4_lake_liquid_fraction_prev", &|_| SPVAL)?;
    layered(&mut file, "ch4_layer_sat_lag", &|i| agg(i).layer_sat_lag)?;
    layered(&mut file, "ch4_layer_sat_lag_soil", &|i| {
        soil(i).layer_sat_lag
    })?;
    layered(&mut file, "ch4_layer_sat_lag_rice", &|i| {
        rice(i).layer_sat_lag
    })?;
    layered(&mut file, "ch4_lake_soilc", &|i| p(i).lake_soilc)?;
    type Annual = colm_core::methane::physics::AnnualAccumulators;
    type Pick = fn(&Annual) -> f64;
    let annual: [(&str, Pick); 9] = [
        ("annavg_agnpp", |a| a.annavg_agnpp),
        ("annavg_bgnpp", |a| a.annavg_bgnpp),
        ("annavg_somhr", |a| a.annavg_somhr),
        ("annavg_finrw", |a| a.annavg_finrw),
        ("tempavg_agnpp", |a| a.tempavg_agnpp),
        ("tempavg_bgnpp", |a| a.tempavg_bgnpp),
        ("annsum_counter", |a| a.annsum_counter),
        ("tempavg_somhr", |a| a.tempavg_somhr),
        ("tempavg_finrw", |a| a.tempavg_finrw),
    ];
    for (name, f) in annual {
        scalar(&mut file, &format!("ch4_{name}"), &|i| f(&agg(i).annual))?;
    }
    for (name, f) in annual {
        scalar(&mut file, &format!("ch4_{name}_soil"), &|i| {
            f(&soil(i).annual)
        })?;
        scalar(&mut file, &format!("ch4_{name}_rice"), &|i| {
            f(&rice(i).annual)
        })?;
    }
    scalar(&mut file, "ch4_fsat_bef", &|i| agg(i).fsat_bef)?;
    scalar(&mut file, "ch4_finundated_lag", &|i| agg(i).finundated_lag)?;
    scalar(&mut file, "ch4_fsat_bef_soil", &|i| soil(i).fsat_bef)?;
    scalar(&mut file, "ch4_fsat_bef_rice", &|i| rice(i).fsat_bef)?;
    scalar(&mut file, "ch4_finundated_lag_soil", &|i| {
        soil(i).finundated_lag
    })?;
    scalar(&mut file, "ch4_finundated_lag_rice", &|i| {
        rice(i).finundated_lag
    })?;
    scalar(&mut file, "ch4_rice_fraction_prev", &|i| {
        p(i).rice_fraction_prev
    })?;
    scalar(&mut file, "ch4_methane_dfsat_tot", &|i| last(i).dfsat_tot)?;
    scalar(&mut file, "ch4_f_h2osfc", &|i| p(i).f_h2osfc)?;
    for name in [
        "ch4_f_inund_levee_patch",
        "ch4_f_inund_flood_patch",
        "ch4_f_inund_flood_depth_patch",
    ] {
        scalar(&mut file, name, &|_| 0.0)?;
    }
    let accumulators: Vec<_> = patches.iter().map(|(_, a)| *a).collect();
    write_accflux(&mut file, setup, &accumulators)?;
    scalar(&mut file, "ch4_restart_complete", &|_| 1.0)?;
    Ok(())
}

/// `core` history 模式累加的那 16 个 `ch4_a_*` 量在 [`CoreAccumulator`] 里的位置；其余返回 `None`。
fn core_slot<'a>(
    a: &'a mut colm_core::methane::driver::CoreAccumulator,
    name: &str,
) -> Option<&'a mut f64> {
    Some(match name {
        "ch4_a_methane_surf_flux_tot" => &mut a.surf_flux_tot,
        "ch4_a_methane_surf_flux_tot_phys" => &mut a.surf_flux_tot_phys,
        "ch4_a_methane_balance_residual" => &mut a.balance_residual,
        "ch4_a_methane_ch4_clip_credit" => &mut a.ch4_clip_credit,
        "ch4_a_o2_cap_loss" => &mut a.o2_cap_loss,
        "ch4_a_o2_cap_gain" => &mut a.o2_cap_gain,
        "ch4_a_methane_prod_tot" => &mut a.prod_tot,
        "ch4_a_methane_oxid_tot" => &mut a.oxid_tot,
        "ch4_a_totcol_methane" => &mut a.totcol,
        "ch4_a_methane_surf_flux_wetland" => &mut a.surf_flux_wetland,
        "ch4_a_methane_surf_flux_soil" => &mut a.surf_flux_soil,
        "ch4_a_methane_surf_flux_lake" => &mut a.surf_flux_lake,
        "ch4_a_methane_surf_flux_rice" => &mut a.surf_flux_rice,
        "ch4_a_methane_surf_flux_tot_lake" => &mut a.surf_flux_tot_lake,
        "ch4_a_methane_acc_num" => &mut a.acc_num,
        "ch4_a_methane_acc_num_lake" => &mut a.acc_num_lake,
        _ => return None,
    })
}

/// 续跑读回的一个 patch 的甲烷状态。
pub struct RestartedPatch {
    pub patch: colm_core::methane::driver::MethanePatch,
    pub accumulator: colm_core::methane::driver::CoreAccumulator,
}

/// `ch4_reactive_read_restart`（`MOD_Tracer_Reactive_Methane.F90`）：时间重启里没有任何甲烷量时
/// 冷启动（返回 `None`）。只接受本版本写的事务（schema 5、`ch4_restart_complete = 1`）：旧 schema
/// 的迁移与非严格修补没有移植，遇到就停下。严格读时上游把任何非法或负的预报量当损坏拒绝，
/// 合法文件上其余 `WHERE` 修补都是恒等，唯一例外是导度 `<= 0` 换成默认值。
pub fn read_restart(
    time: &colm_init::RestartFile,
    patch: usize,
    setup: &MethaneSetup,
) -> Result<Option<RestartedPatch>> {
    use anyhow::ensure;
    use colm_core::methane::column::ColumnResult;
    use colm_core::methane::config::{COMP_RICE, COMP_SOIL};
    use colm_core::methane::driver::{CoreAccumulator, MethanePatch};
    use colm_core::methane::physics::{NL_SOIL, SPVAL};

    let probes = [
        "ch4_conc_methane",
        "ch4_restart_schema",
        "ch4_restart_complete",
        "ch4_history_accumulation_mode",
        "ch4_feature_microbe_pools",
    ];
    if !probes.iter().any(|name| time.contains(name)) {
        return Ok(None);
    }
    ensure!(
        time.contains("ch4_conc_methane"),
        "methane restart metadata exists without mandatory ch4_conc_methane state"
    );
    // `validate_methane_restart_transaction`。
    ensure!(
        time.contains("ch4_restart_schema") && time.contains("ch4_restart_complete"),
        "the methane restart has no committed transaction markers; legacy methane restarts \
         are not ported to the Rust runtime"
    );
    let scalar = |name: &str| -> Result<f64> {
        time.patch_scalars(name)?
            .get(patch)
            .copied()
            .with_context(|| format!("{name} has no value for patch {patch}"))
    };
    let layers = |name: &str| -> Result<[f64; NL_SOIL]> {
        let column = time.layer_column(name, patch, NL_SOIL)?;
        Ok(std::array::from_fn(|j| column[j]))
    };
    let schema = scalar("ch4_restart_schema")?;
    ensure!(
        scalar("ch4_restart_complete")? == 1.0 && schema == RESTART_SCHEMA,
        "methane restart transaction is uncommitted or has schema {schema}; only schema \
         {RESTART_SCHEMA} is ported to the Rust runtime"
    );
    ensure!(
        time.contains("ch4_history_accumulation_mode"),
        "schema-3+ methane restart is missing the history accumulation mode"
    );
    let history_mode_changed = scalar("ch4_history_accumulation_mode")?
        != f64::from(setup.params.history_accumulation_mode());
    ensure!(
        time.contains("ch4_feature_microbe_pools"),
        "schema-5 methane restart is missing the microbial-pool feature marker"
    );
    let microbes = scalar("ch4_feature_microbe_pools")?;
    ensure!(
        microbes == 0.0 || microbes == 1.0,
        "schema-5 methane restart has an invalid microbial-pool feature marker"
    );
    ensure!(
        microbes == 0.0,
        "methane restarts with microbial pools are not ported to the Rust runtime yet"
    );

    // `read_methane_restart`（严格、schema 5：分量组必须齐全）。
    let invalid = |x: f64| x.is_nan() || x.abs() >= 0.5 * SPVAL.abs();
    let bad_fraction = |x: f64| x != SPVAL && (invalid(x) || !(0.0..=1.0).contains(&x));
    let mut corrupt = 0usize;
    let mut check = |values: &[f64], bad: &dyn Fn(f64) -> bool| {
        corrupt += values.iter().filter(|&&x| bad(x)).count();
    };
    let nonneg = |x: f64| invalid(x) || x < 0.0;
    let conc_o2 = layers("ch4_conc_o2")?;
    let conc_methane = layers("ch4_conc_methane")?;
    let totcol = scalar("ch4_totcol_methane")?;
    let grnd_cond = scalar("ch4_grnd_methane_cond")?;
    for name in [
        "ch4_conc_o2_unsat",
        "ch4_conc_o2_sat",
        "ch4_conc_ch4_unsat",
        "ch4_conc_ch4_sat",
        "ch4_conc_o2_lake",
        "ch4_conc_ch4_lake",
        "ch4_lake_soilc",
    ] {
        check(&layers(name)?, &nonneg);
    }
    check(&conc_o2, &nonneg);
    check(&conc_methane, &nonneg);
    let totcol_unsat = scalar("ch4_totcol_methane_unsat")?;
    let totcol_sat = scalar("ch4_totcol_methane_sat")?;
    let cond_unsat = scalar("ch4_grnd_methane_cond_unsat")?;
    let cond_sat = scalar("ch4_grnd_methane_cond_sat")?;
    let mut nonneg_scalars = vec![
        totcol,
        totcol_unsat,
        totcol_sat,
        grnd_cond,
        cond_unsat,
        cond_sat,
    ];
    for name in ["ch4_totcol_lake", "ch4_grnd_methane_cond_lake"] {
        nonneg_scalars.push(scalar(name)?);
    }
    let annual_names = [
        "annavg_agnpp",
        "annavg_bgnpp",
        "annavg_somhr",
        "annavg_finrw",
        "tempavg_agnpp",
        "tempavg_bgnpp",
        "annsum_counter",
        "tempavg_somhr",
        "tempavg_finrw",
    ];
    // `annavg_finrw` 可以是哨兵 spval，其余年累加量必须非负。
    for name in annual_names {
        let value = scalar(&format!("ch4_{name}"))?;
        if name == "annavg_finrw" {
            check(&[value], &bad_fraction);
        } else {
            nonneg_scalars.push(value);
        }
    }
    check(&nonneg_scalars, &nonneg);
    for name in ["ch4_fsat_bef", "ch4_finundated_lag"] {
        check(&[scalar(name)?], &bad_fraction);
    }
    check(&layers("ch4_layer_sat_lag")?, &bad_fraction);
    let dfsat_tot = scalar("ch4_methane_dfsat_tot")?;
    check(&[dfsat_tot], &invalid);
    let f_h2osfc = scalar("ch4_f_h2osfc")?;
    let unit = |x: f64| invalid(x) || !(0.0..=1.0).contains(&x);
    check(&[f_h2osfc], &unit);
    for name in ["ch4_f_inund_levee_patch", "ch4_f_inund_flood_patch"] {
        check(&[scalar(name)?], &unit);
    }
    check(&[scalar("ch4_f_inund_flood_depth_patch")?], &nonneg);
    // 湖库存（schema 2+/4+ 一定在）：本移植只跑土壤 patch，它们停在冷启动值，只校验。
    let floor = |x: f64| invalid(x) || x < -1.0e-18;
    for name in [
        "ch4_lake_water_ch4_stock",
        "ch4_lake_water_o2_stock",
        "ch4_lake_frozen_ch4_stock",
        "ch4_lake_frozen_o2_stock",
    ] {
        check(&[scalar(name)?], &floor);
    }
    check(&[scalar("ch4_lake_liquid_fraction_prev")?], &bad_fraction);

    let mut restarted = MethanePatch::cold(&setup.params);
    for (component, suffix) in [(COMP_SOIL, "soil"), (COMP_RICE, "rice")] {
        let c = &mut restarted.components[component];
        c.conc_o2_unsat = layers(&format!("ch4_conc_o2_unsat_{suffix}"))?;
        c.conc_o2_sat = layers(&format!("ch4_conc_o2_sat_{suffix}"))?;
        c.conc_methane_unsat = layers(&format!("ch4_conc_ch4_unsat_{suffix}"))?;
        c.conc_methane_sat = layers(&format!("ch4_conc_ch4_sat_{suffix}"))?;
        c.layer_sat_lag = layers(&format!("ch4_layer_sat_lag_{suffix}"))?;
        let a = &mut c.annual;
        a.annavg_agnpp = scalar(&format!("ch4_annavg_agnpp_{suffix}"))?;
        a.annavg_bgnpp = scalar(&format!("ch4_annavg_bgnpp_{suffix}"))?;
        a.annavg_somhr = scalar(&format!("ch4_annavg_somhr_{suffix}"))?;
        a.annavg_finrw = scalar(&format!("ch4_annavg_finrw_{suffix}"))?;
        a.tempavg_agnpp = scalar(&format!("ch4_tempavg_agnpp_{suffix}"))?;
        a.tempavg_bgnpp = scalar(&format!("ch4_tempavg_bgnpp_{suffix}"))?;
        a.annsum_counter = scalar(&format!("ch4_annsum_counter_{suffix}"))?;
        a.tempavg_somhr = scalar(&format!("ch4_tempavg_somhr_{suffix}"))?;
        a.tempavg_finrw = scalar(&format!("ch4_tempavg_finrw_{suffix}"))?;
        c.fsat_bef = scalar(&format!("ch4_fsat_bef_{suffix}"))?;
        c.finundated_lag = scalar(&format!("ch4_finundated_lag_{suffix}"))?;
        for layer in [
            c.conc_o2_unsat,
            c.conc_o2_sat,
            c.conc_methane_unsat,
            c.conc_methane_sat,
        ] {
            check(&layer, &nonneg);
        }
        check(&c.layer_sat_lag, &bad_fraction);
        check(
            &[
                a.annavg_agnpp,
                a.annavg_bgnpp,
                a.annavg_somhr,
                a.tempavg_agnpp,
                a.tempavg_bgnpp,
                a.annsum_counter,
                a.tempavg_somhr,
                a.tempavg_finrw,
            ],
            &nonneg,
        );
        check(
            &[a.annavg_finrw, c.fsat_bef, c.finundated_lag],
            &bad_fraction,
        );
    }
    // patch 级聚合状态（湿地直接推进它；土壤 patch 上它与土壤分量相同）。
    {
        let a = &mut restarted.aggregate;
        a.conc_o2_unsat = layers("ch4_conc_o2_unsat")?;
        a.conc_o2_sat = layers("ch4_conc_o2_sat")?;
        a.conc_methane_unsat = layers("ch4_conc_ch4_unsat")?;
        a.conc_methane_sat = layers("ch4_conc_ch4_sat")?;
        a.layer_sat_lag = layers("ch4_layer_sat_lag")?;
        let n = &mut a.annual;
        n.annavg_agnpp = scalar("ch4_annavg_agnpp")?;
        n.annavg_bgnpp = scalar("ch4_annavg_bgnpp")?;
        n.annavg_somhr = scalar("ch4_annavg_somhr")?;
        n.annavg_finrw = scalar("ch4_annavg_finrw")?;
        n.tempavg_agnpp = scalar("ch4_tempavg_agnpp")?;
        n.tempavg_bgnpp = scalar("ch4_tempavg_bgnpp")?;
        n.annsum_counter = scalar("ch4_annsum_counter")?;
        n.tempavg_somhr = scalar("ch4_tempavg_somhr")?;
        n.tempavg_finrw = scalar("ch4_tempavg_finrw")?;
        a.fsat_bef = scalar("ch4_fsat_bef")?;
        a.finundated_lag = scalar("ch4_finundated_lag")?;
    }
    restarted.rice_fraction_prev = scalar("ch4_rice_fraction_prev")?;
    check(&[restarted.rice_fraction_prev], &unit);
    ensure!(
        corrupt == 0,
        "committed methane restart contains {corrupt} invalid or negative prognostic state \
         values; refusing checkpoint"
    );
    let default_cond = setup.params.methane.grnd_methane_cond_default;
    let positive_or_default = |x: f64| if x <= 0.0 { default_cond } else { x };
    restarted.lake_soilc = layers("ch4_lake_soilc")?;
    restarted.f_h2osfc = f_h2osfc;
    restarted.totcol_methane = totcol;
    restarted.grnd_methane_cond = positive_or_default(grnd_cond);
    // 只供续跑写出的聚合量（`aggregate_methane_columns` 的结果）；下一步的物理不读它们。
    let mut last = ColumnResult {
        conc_o2,
        conc_methane,
        dfsat_tot,
        ..ColumnResult::default()
    };
    last.unsat.totcol = totcol_unsat;
    last.sat.totcol = totcol_sat;
    last.unsat.grnd_cond = positive_or_default(cond_unsat);
    last.sat.grnd_cond = positive_or_default(cond_sat);
    restarted.last = Some(last);

    // `read_methane_accflux_restart`。
    ensure!(
        time.contains("ch4_acc_history_selector_hash"),
        "schema-3 methane restart is missing the history-selector fingerprint"
    );
    let selector = scalar("ch4_acc_history_selector_hash")?;
    ensure!(
        selector.is_finite() && selector.abs() < 0.5 * SPVAL.abs(),
        "schema-3 methane restart has an invalid history-selector fingerprint"
    );
    let mut accumulator = CoreAccumulator::default();
    // 历史模式或选择变了：上游清掉进行中的累加窗口（`flush_methane_acc_fluxes`）。
    if history_mode_changed || selector != history_selector_fingerprint(&setup.params) as f64 {
        eprintln!("WARNING: methane history selection changed across restart; resetting the partial window.");
        return Ok(Some(RestartedPatch {
            patch: restarted,
            accumulator,
        }));
    }
    for &(name, layered) in restart_fields::ACCFLUX_FIELDS {
        let values = if layered {
            layers(name)?.to_vec()
        } else {
            vec![scalar(name)?]
        };
        match core_slot(&mut accumulator, name) {
            Some(slot) => *slot = values[0],
            // `core` 模式只累加上面那 16 个量，同一模式写的文件里其余必然为 0。
            None => ensure!(
                values.iter().all(|&x| x == 0.0),
                "{name} is nonzero although the methane history runs in core mode"
            ),
        }
    }
    let counters = [
        "ch4_a_methane_acc_num_unsat",
        "ch4_a_methane_acc_num_sat",
        "ch4_a_methane_acc_num_extra",
        "ch4_a_annavg_finrw_acc_num",
        "ch4_a_methane_rice_fraction",
    ];
    let counter_values = counters
        .iter()
        .map(|name| scalar(name))
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        accumulator.acc_num >= 0.0
            && accumulator.acc_num_lake >= 0.0
            && counter_values.iter().all(|&x| x >= 0.0)
            && counter_values[4] <= accumulator.acc_num + 1.0e-10,
        "committed methane restart contains invalid accumulator sums or counters"
    );
    Ok(Some(RestartedPatch {
        patch: restarted,
        accumulator,
    }))
}

/// `write_methane_accflux_restart`：选择指纹 + 150 个 `ch4_a_*`（续跑文件与 history 旁车共用）。
/// 调用方先定义好 `patch` 维；`soil` 维这里补上。
pub fn write_accflux(
    file: &mut netcdf::FileMut,
    setup: &MethaneSetup,
    accumulators: &[&colm_core::methane::driver::CoreAccumulator],
) -> Result<()> {
    use crate::tracer::{ensure_dimension, put_array_f64};
    use colm_core::methane::physics::NL_SOIL;
    let n = accumulators.len();
    ensure_dimension(file, "soil", NL_SOIL)?;
    let hash = history_selector_fingerprint(&setup.params) as f64;
    put_array_f64(
        file,
        "ch4_acc_history_selector_hash",
        &["patch"],
        &vec![hash; n],
    )?;
    for &(name, layers) in restart_fields::ACCFLUX_FIELDS {
        if layers {
            // `core` 模式不累加的量停在清零值。
            put_array_f64(file, name, &["patch", "soil"], &vec![0.0; n * NL_SOIL])?;
        } else {
            let values: Vec<f64> = accumulators
                .iter()
                .map(|a| {
                    let mut a = **a;
                    core_slot(&mut a, name).map_or(0.0, |slot| *slot)
                })
                .collect();
            put_array_f64(file, name, &["patch"], &values)?;
        }
    }
    Ok(())
}

/// `ch4_reactive_read_history_sidecar`：旁车里没有 `ch4_a_methane_acc_num` 时返回 `None`（保留
/// 续跑文件读回的累加量）。上游这里走非严格读：选择指纹变了或计数损坏都只清空窗口，缺的量取 0。
pub fn read_accflux_sidecar(
    file: &netcdf::File,
    setup: &MethaneSetup,
    patches: usize,
) -> Result<Option<Vec<colm_core::methane::driver::CoreAccumulator>>> {
    use anyhow::ensure;
    use colm_core::methane::driver::CoreAccumulator;
    use colm_core::methane::physics::SPVAL;
    let read = |name: &str| -> Result<Option<Vec<f64>>> {
        file.variable(name)
            .map(|variable| -> Result<Vec<f64>> {
                let values = variable.get_values::<f64, _>(..)?;
                ensure!(
                    values.len() % patches == 0,
                    "{name} does not hold {patches} patches"
                );
                Ok(values)
            })
            .transpose()
    };
    if file.variable("ch4_a_methane_acc_num").is_none() {
        return Ok(None);
    }
    let flushed = || Ok(Some(vec![CoreAccumulator::default(); patches]));
    if let Some(hash) = read("ch4_acc_history_selector_hash")? {
        let fingerprint = history_selector_fingerprint(&setup.params) as f64;
        if hash
            .iter()
            .any(|&x| !x.is_finite() || x.abs() >= 0.5 * SPVAL.abs() || x != fingerprint)
        {
            eprintln!(
                "WARNING: methane history selection changed across restart; resetting the partial window."
            );
            return flushed();
        }
    }
    let mut accumulators = vec![CoreAccumulator::default(); patches];
    for &(name, _) in restart_fields::ACCFLUX_FIELDS {
        let Some(values) = read(name)? else { continue };
        let width = values.len() / patches;
        for (patch, accumulator) in accumulators.iter_mut().enumerate() {
            let column = &values[patch * width..(patch + 1) * width];
            match core_slot(accumulator, name) {
                Some(slot) => *slot = column[0],
                None => ensure!(
                    column.iter().all(|&x| x == 0.0),
                    "{name} is nonzero although the methane history runs in core mode"
                ),
            }
        }
    }
    let corrupt = accumulators
        .iter()
        .any(|a| a.acc_num < 0.0 || a.acc_num_lake < 0.0);
    if corrupt {
        eprintln!("WARNING: non-strict methane restart resets corrupt CH4 history accumulators.");
        return flushed();
    }
    Ok(Some(accumulators))
}

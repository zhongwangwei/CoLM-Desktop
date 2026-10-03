//! 土地覆盖变化的质量与能量守恒方案（MEC，`DEF_LULCC_SCHEME = 2`）。
//!
//! 上游 `LulccMassEnergyConserve`（`MOD_Lulcc_MassEnergyConserve.F90`）在 SAT 之后执行：
//! 对每个土地覆盖份额有变化的新 patch（`sum(lccpct) - lccpct(自身类型) > 0`），按转移份额
//! `lccpct_patches` 把同一单元里各来源旧 patch 的水、热与雪层状态加权混合，再重算雪盖、
//! 地表温度与地下水位。份额没变的 patch 保留 SAT 的结果。
//!
//! LCT 与 PFT 路径；PC 与城市分支（按城市类型取来源）调用方先拒绝。PFT（非 SOLO）时
//! 上游先把 `patchtypes == 0` 的各类份额并进类 1（PFT 土壤 patch 的 `patchclass`），
//! 再对配对单元里**每个**土壤 patch（不只是份额有变化的）按 PFT 重分 `ldew_p`、重算
//! `sigf_p/sigf/fsno` 与 `sai(_p)`/`lai(_p)`（`MOD_Lulcc_MassEnergyConserve.F90:876-905`）。
//! 数值形状按 gfortran `-O2` 的 GIMPLE：加权累加 `x + x_*lcc/sum` 与 `t + t_*cv*lcc/wgt` 都不
//! 收缩（末尾是除法），`wgt`、`hcap`、雪层热容与焓、`rhosnow` 的累加（两个商之积）是 FMA，
//! `z_sno = zi - 0.5*dz` 是 FNMA。

use std::collections::BTreeMap;

use anyhow::{bail, ensure, Context, Result};

use crate::restart_read::{RestartFile, RestartOverride};

const CPLIQ: f64 = 4188.0;
const CPICE: f64 = 2117.27;
const HFUS: f64 = 0.3336e6;
const TFRZ: f64 = 273.16;
const DENH2O: f64 = 1000.0;
const DENICE: f64 = 917.0;
/// `maxsnl = -5`：雪层是 `-4..=0`，盘上 `snow` 轴 5 层、`soilsnow` 轴从 `-4` 起。
const SNOW_LAYERS: usize = 5;
/// 冰川（`patchtype == 3`）不参与 MEC。
const GLACIER: i64 = 3;
/// 裸土粗糙度（上游在这里写死 `zlnd = 0.01`）。
const ZLND: f64 = 0.01;
/// IGBP `patchtypes`（`MOD_Const_LC.F90:398`，下标是类号 1..=17）。
pub const IGBP_PATCHTYPES: [i64; 17] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 1, 0, 3, 0, 4];
/// IGBP CROPLAND。
const CROPLAND: usize = 12;
/// PFT 归并时单独保留的类：WETLAND、URBAN、GLACIERS、WATERBODY（赋值次序不影响结果）。
const KEPT_CLASSES: [usize; 4] = [13, 11, 17, 15];

/// 控制 MEC 行为的 namelist 开关。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MecOptions {
    pub plant_hydraulics: bool,
    pub ozone_stress: bool,
    /// `DEF_USE_VariablySaturatedFlow`：真时由土壤含水重定 `zwt`，否则按份额混合 `zwt`。
    pub variably_saturated_flow: bool,
    /// `DEF_VEG_SNOW`：真时 `lai = tlai*sigf`。
    pub vegetation_snow: bool,
    /// `DEF_TUNING_SNOW_COVER_EXPONENT`（`snowfraction` 的 `m`）。
    pub snow_cover_exponent: f64,
}

/// MEC 的输入：新旧两侧的重启、单元归属与新 patch 的转移份额。
pub struct MecInputs<'a> {
    /// 新年份冷启动的时间重启（`tlai/tsai` 取这里）。
    pub new_time: &'a RestartFile,
    /// 新年份常数重启（`patchclass/patchtype/porsl`）。
    pub new_const: &'a RestartFile,
    pub new_element: &'a [i64],
    /// 旧年份终态的时间重启。
    pub old_time: &'a RestartFile,
    /// 旧年份常数重启（`patchclass/csol`）。
    pub old_const: &'a RestartFile,
    pub old_element: &'a [i64],
    /// `lccpct[patch][ilc]`，`ilc = 0..=nlc`（`landdata/lulcc/<year>/lccpct_patches_lcXX`）。
    pub lccpct: &'a [Vec<f64>],
    /// PFT 路径的输入；LCT 时为 `None`。
    pub pft: Option<MecPft<'a>>,
}

/// MEC 的 PFT 输入（新侧）：冷启动 PFT 时间重启叠上 SAT 的逐 PFT 替换。
pub struct MecPft<'a> {
    pub time: &'a RestartFile,
    /// `pft_same_type_assignment` 的逐 PFT 替换值。
    pub sat: Vec<RestartOverride>,
    pub pft_class: &'a [i64],
    pub pftfrac: &'a [f64],
    pub htop: &'a [f64],
    pub hbot: &'a [f64],
    /// 每个新 patch 的 PFT 区间（`patch_pft_s:patch_pft_e`）。
    pub ranges: &'a [std::ops::Range<usize>],
    /// `(DEF_USE_PFT .and. .not. DEF_SOLO_PFT) .or. DEF_FAST_PC`：把土壤类的份额并进类 1。
    pub merge_soil_classes: bool,
    /// `DEF_FAST_PC`：作物（CROPLAND 与类 14）另成 CROPLAND，再从类 1 里减掉。
    pub fast_pc: bool,
}

/// MEC 的结果：patch 级与 PFT 级的替换值。
#[derive(Debug, Clone, PartialEq)]
pub struct MecResult {
    pub patch: Vec<RestartOverride>,
    pub pft: Vec<RestartOverride>,
}

/// 一个变量：每 patch 的值数与全部值（盘上次序，patch 在前）。
struct Field {
    row: usize,
    values: Vec<f64>,
}

impl Field {
    fn at(&self, patch: usize, index: usize) -> f64 {
        self.values[patch * self.row + index]
    }
}

fn load(file: &RestartFile, name: &str) -> Result<Field> {
    let dims = file.variable_dimensions(name)?;
    ensure!(
        dims.first().map(String::as_str) == Some("patch"),
        "{name} is not patch-major ({dims:?})"
    );
    let row = dims[1..]
        .iter()
        .map(|dimension| file.dimension(dimension))
        .product::<Result<usize>>()?;
    let values = match file.floats(name) {
        Ok(values) => values.to_vec(),
        Err(_) => file.integers(name)?.iter().map(|&v| v as f64).collect(),
    };
    Ok(Field { row, values })
}

/// 混合时整行按份额累加的变量（上游 `DO k = 1, num` 那一段里除水、温之外的量）。
const MIXED_ROWS: [&str; 25] = [
    "tleaf",
    "ldew",
    "ldew_rain",
    "ldew_snow",
    "sag",
    "wa",
    "wdsrf",
    "snw_rds",
    "mss_bcpho",
    "mss_bcphi",
    "mss_ocpho",
    "mss_ocphi",
    "mss_dst1",
    "mss_dst2",
    "mss_dst3",
    "mss_dst4",
    "ssno_lyr",
    "smp",
    "hk",
    "trad",
    "tref",
    "qref",
    "rst",
    "emis",
    "z0m",
];
const MIXED_ROWS_TAIL: [&str; 8] = ["zol", "rib", "ustar", "qstar", "tstar", "fm", "fh", "fq"];
/// 先清零、再按需重算的量（上游把它们置 0 后不在累加段里出现）。
const ZEROED_ONLY: [&str; 6] = ["t_grnd", "scv", "snowdp", "fsno", "sigf", "zwt"];

/// 在 SAT 的替换值之上做 MEC，返回写回新重启时的全部替换值。
#[allow(clippy::needless_range_loop)] // 逐层下标与上游的层号循环一一对应
pub fn mass_energy_conserve(
    inputs: &MecInputs<'_>,
    sat: Vec<RestartOverride>,
    options: MecOptions,
) -> Result<MecResult> {
    let new_class = inputs.new_const.integers("patchclass")?;
    let new_type = inputs.new_const.integers("patchtype")?;
    let old_class = inputs.old_const.integers("patchclass")?;
    let patches = new_class.len();
    ensure!(
        inputs.new_element.len() == patches && inputs.lccpct.len() == patches,
        "the new patch topology, patchclass and transfer trace disagree on the patch count"
    );
    ensure!(
        inputs.old_element.len() == old_class.len(),
        "the old patch topology and patchclass disagree on the patch count"
    );

    // 新状态：冷启动值叠上 SAT 的替换。
    let mut names: Vec<&str> = vec!["wliq_soisno", "wice_soisno", "t_soisno", "z_sno", "dz_sno"];
    names.extend(MIXED_ROWS);
    names.extend(MIXED_ROWS_TAIL);
    names.extend(ZEROED_ONLY);
    names.extend(["lai", "sai"]);
    if options.plant_hydraulics {
        names.extend(["vegwp", "gs0sun", "gs0sha"]);
    }
    if options.ozone_stress {
        names.push("lai_old");
    }
    let mut state: BTreeMap<&str, Field> = BTreeMap::new();
    for &name in &names {
        let mut field = load(inputs.new_time, name)?;
        if let Some(entry) = sat.iter().find(|entry| entry.name == name) {
            ensure!(
                entry.values.len() == field.values.len(),
                "the SAT value of {name} does not fit the new restart"
            );
            field.values.clone_from(&entry.values);
        }
        state.insert(name, field);
    }
    let old: BTreeMap<&str, Field> = names
        .iter()
        .filter(|name| !matches!(**name, "lai" | "sai"))
        .map(|&name| Ok((name, load(inputs.old_time, name)?)))
        .collect::<Result<_>>()?;
    let tlai = inputs.new_time.floats("tlai")?;
    let tsai = inputs.new_time.floats("tsai")?;
    let csol_old = load(inputs.old_const, "csol")?;
    let porsl = load(inputs.new_const, "porsl")?;
    let soil_layers = csol_old.row;
    ensure!(
        porsl.row == soil_layers && state["t_soisno"].row == SNOW_LAYERS + soil_layers,
        "soil layer counts disagree between the restarts"
    );
    let grid = colm_core::static_state::colm_soil_grid(soil_layers)?;
    let dz_soi = &grid.thickness_m;
    let zi_soi = &grid.interface_depth_m;
    let c_water = CPLIQ * DENH2O;
    let c_ice = CPICE * DENICE;
    let nlc = inputs.lccpct[0].len() - 1;
    let layers = SNOW_LAYERS + soil_layers;
    // 雪层 `l ∈ -4..=0` 在 soilsnow 行里的下标是 `l + 4`，在 snow 行里同样。
    let sl = |l: i64| (l + SNOW_LAYERS as i64 - 1) as usize;

    // 每个单元在两侧的 patch 区间（`grid_patch_s/e`）。
    let spans = |element: &[i64]| {
        let mut spans = BTreeMap::<i64, (usize, usize)>::new();
        for (patch, &e) in element.iter().enumerate() {
            spans
                .entry(e)
                .and_modify(|(first, last)| {
                    *first = (*first).min(patch);
                    *last = (*last).max(patch);
                })
                .or_insert((patch, patch));
        }
        spans
    };
    let old_spans = spans(inputs.old_element);
    let mut touched = false;
    // 配对单元里的新 patch：PFT 尾段对其中每个土壤 patch 都执行。
    let mut matched = Vec::new();

    for (element, (first, last)) in spans(inputs.new_element) {
        let Some(&(first_, last_)) = old_spans.get(&element) else {
            continue;
        };
        for np in first..=last {
            matched.push(np);
            if new_type[np] == GLACIER {
                continue;
            }
            let lcc = match &inputs.pft {
                // PFT（非 SOLO）：`lccpct_np(1) = sum(lccpct_patches(np,1:), mask=patchtypes==0)`
                // （从 0 起的顺序加法），四个非土壤类照抄，其余为 0。
                Some(pft) if pft.merge_soil_classes => {
                    ensure!(nlc == IGBP_PATCHTYPES.len(), "PFT MEC expects the 17 IGBP classes");
                    let row = &inputs.lccpct[np];
                    let mut merged = vec![0.0; nlc];
                    merged[0] = (1..=nlc)
                        .filter(|&ilc| IGBP_PATCHTYPES[ilc - 1] == 0)
                        .fold(0.0, |acc, ilc| row[ilc] + acc);
                    for ilc in KEPT_CLASSES {
                        merged[ilc - 1] = row[ilc];
                    }
                    // `:181-185`：FAST_PC 先合出 CROPLAND，再从类 1 的总和里**减**（不是另起一条和）。
                    if pft.fast_pc {
                        merged[CROPLAND - 1] = row[CROPLAND] + row[14];
                        merged[0] -= merged[CROPLAND - 1];
                    }
                    merged
                }
                // LCT：`lccpct_np(:) = lccpct_patches(np, 1:nlc)`。
                _ => inputs.lccpct[np][1..=nlc].to_vec(),
            };
            let lcc_of = |class: i64| lcc[(class - 1) as usize];
            let sum_lcc: f64 = lcc.iter().sum();
            if sum_lcc - lcc_of(new_class[np]) <= 0.0 {
                continue;
            }
            touched = true;
            // 来源 patch：按类型从小到大，在旧单元里找同类型的第一个 patch。
            let mut sources = Vec::new();
            for ilc in 1..=nlc {
                if lcc[ilc - 1] > 0.0 {
                    let found = (first_..=last_).find(|&p| old_class[p] == ilc as i64);
                    match found {
                        Some(p) => sources.push(p),
                        None => bail!(
                            "LULCC source patch of class {ilc} not found for new patch {np} (element {element})"
                        ),
                    }
                }
            }
            let weight = |k: usize| lcc_of(old_class[sources[k]]);

            // 清零（上游逐个置 0 的那一组）。
            for (name, field) in state.iter_mut() {
                if matches!(*name, "lai" | "sai") {
                    continue;
                }
                let row = field.row;
                field.values[np * row..(np + 1) * row].fill(0.0);
            }

            // 各来源的热容 `cvsoil_` 与雪层焓 `h_`，以及按份额的热容权重 `wgt`、焓 `hc`。
            let num = sources.len();
            let mut cvsoil = vec![vec![0.0; layers]; num];
            let mut h = vec![vec![0.0; SNOW_LAYERS]; num];
            let mut wgt = vec![0.0; layers];
            let mut hc = [0.0; SNOW_LAYERS];
            for k in 0..num {
                let p = sources[k];
                for l in 0..soil_layers {
                    let s = SNOW_LAYERS + l;
                    let vf_water = old["wliq_soisno"].at(p, s) / (dz_soi[l] * DENH2O);
                    let vf_ice = old["wice_soisno"].at(p, s) / (dz_soi[l] * DENICE);
                    let hcap = vf_ice.mul_add(c_ice, vf_water.mul_add(c_water, csol_old.at(p, l)));
                    cvsoil[k][s] = hcap * dz_soi[l];
                }
                let scv_ = old["scv"].at(p, 0);
                if old["dz_sno"].at(p, sl(0)) < 1.0e-6 && scv_ > 0.0 {
                    cvsoil[k][SNOW_LAYERS] = scv_.mul_add(CPICE, cvsoil[k][SNOW_LAYERS]);
                }
                if old["z_sno"].at(p, sl(0)) < 0.0 {
                    for s in 0..SNOW_LAYERS {
                        let wliq = old["wliq_soisno"].at(p, s);
                        let wice = old["wice_soisno"].at(p, s);
                        cvsoil[k][s] = wliq.mul_add(CPLIQ, CPICE * wice);
                        let heat = wliq.mul_add(CPLIQ, CPICE * wice);
                        h[k][s] = heat.mul_add(old["t_soisno"].at(p, s) - TFRZ, HFUS * wliq);
                    }
                }
                let w = weight(k);
                for s in 0..layers {
                    wgt[s] = cvsoil[k][s].mul_add(w, wgt[s]);
                }
                for s in 0..SNOW_LAYERS {
                    hc[s] += h[k][s] * w / sum_lcc;
                }
            }

            // 份额最大的来源（并列取先出现的）决定雪层数。
            let mut dominant = 0;
            for k in 1..num {
                if weight(k) > weight(dominant) {
                    dominant = k;
                }
            }
            let nsl = (0..SNOW_LAYERS)
                .filter(|&s| old["z_sno"].at(sources[dominant], s) < 0.0)
                .count() as i64;
            let nsl_max = (0..SNOW_LAYERS).filter(|&s| wgt[s] > 0.0).count() as i64;
            let mut rhosnow = [0.0; SNOW_LAYERS];
            let maxsnl = -(SNOW_LAYERS as i64);

            // 可写的新 patch 行。
            macro_rules! get {
                ($name:expr, $index:expr) => {
                    state[$name].values[np * state[$name].row + $index]
                };
            }
            macro_rules! set {
                ($name:expr, $index:expr, $value:expr) => {{
                    let value = $value;
                    let field = state.get_mut($name).expect("mixed field is loaded");
                    let row = field.row;
                    field.values[np * row + $index] = value;
                }};
            }
            let temperature = |hc: f64, wice: f64, wliq: f64| {
                let capacity = wice.mul_add(CPICE, CPLIQ * wliq);
                if hc < 0.0 {
                    TFRZ + hc / capacity
                } else if hc <= HFUS * wliq {
                    TFRZ
                } else {
                    TFRZ + (hc - HFUS * wliq) / capacity
                }
            };

            if nsl > 0 {
                let top = sl(-nsl + 1);
                if nsl_max > nsl {
                    for l in nsl + 1..=nsl_max {
                        wgt[top] += wgt[sl(-l + 1)];
                        hc[top] += hc[sl(-l + 1)];
                    }
                }
                for k in 0..num {
                    let p = sources[k];
                    let w = weight(k);
                    for s in top..SNOW_LAYERS {
                        set!(
                            "t_soisno",
                            s,
                            get!("t_soisno", s)
                                + old["t_soisno"].at(p, s) * cvsoil[k][s] * w / wgt[s]
                        );
                        set!(
                            "wliq_soisno",
                            s,
                            get!("wliq_soisno", s) + old["wliq_soisno"].at(p, s) * w / sum_lcc
                        );
                        set!(
                            "wice_soisno",
                            s,
                            get!("wice_soisno", s) + old["wice_soisno"].at(p, s) * w / sum_lcc
                        );
                    }
                    let mut l = 1;
                    while l <= nsl && old["dz_sno"].at(p, sl(-l + 1)) > 0.0 {
                        let s = sl(-l + 1);
                        let mass = old["wliq_soisno"].at(p, s) + old["wice_soisno"].at(p, s);
                        rhosnow[s] =
                            (mass / old["dz_sno"].at(p, s)).mul_add(w / sum_lcc, rhosnow[s]);
                        l += 1;
                        if l > -maxsnl {
                            break;
                        }
                    }
                    if nsl < -maxsnl {
                        let mut l = nsl + 1;
                        while l <= -maxsnl && old["dz_sno"].at(p, sl(-l + 1)) > 0.0 {
                            let s = sl(-l + 1);
                            set!(
                                "wliq_soisno",
                                top,
                                get!("wliq_soisno", top)
                                    + old["wliq_soisno"].at(p, s) * w / sum_lcc
                            );
                            set!(
                                "wice_soisno",
                                top,
                                get!("wice_soisno", top)
                                    + old["wice_soisno"].at(p, s) * w / sum_lcc
                            );
                            set!(
                                "t_soisno",
                                top,
                                get!("t_soisno", top)
                                    + old["t_soisno"].at(p, s) * cvsoil[k][s] * w / wgt[top]
                            );
                            let mass = old["wliq_soisno"].at(p, s) + old["wice_soisno"].at(p, s);
                            rhosnow[top] =
                                (mass / old["dz_sno"].at(p, s)).mul_add(w / sum_lcc, rhosnow[top]);
                            l += 1;
                            if l > -maxsnl {
                                break;
                            }
                        }
                    }
                }
                let mut zi_sno = [0.0; SNOW_LAYERS];
                let mut l = 0;
                while l > -nsl {
                    let s = sl(l);
                    let wice = get!("wice_soisno", s);
                    let wliq = get!("wliq_soisno", s);
                    set!("t_soisno", s, temperature(hc[s], wice, wliq));
                    let dz = (wice + wliq) / rhosnow[s];
                    set!("dz_sno", s, dz);
                    set!("z_sno", s, (-0.5f64).mul_add(dz, zi_sno[s]));
                    if l - 1 < maxsnl + 1 {
                        break;
                    }
                    zi_sno[s - 1] = zi_sno[s] - dz;
                    l -= 1;
                }
            } else {
                let top = sl(0);
                if nsl_max > nsl {
                    for l in nsl + 1..=nsl_max {
                        wgt[top] += wgt[sl(-l + 1)];
                    }
                }
                let mut l = 0;
                while wgt[sl(l)] > 0.0 {
                    let s = sl(l);
                    for k in 0..num {
                        let p = sources[k];
                        let w = weight(k);
                        set!(
                            "wliq_soisno",
                            top,
                            get!("wliq_soisno", top) + old["wliq_soisno"].at(p, s) * w / sum_lcc
                        );
                        set!(
                            "wice_soisno",
                            top,
                            get!("wice_soisno", top) + old["wice_soisno"].at(p, s) * w / sum_lcc
                        );
                        set!(
                            "t_soisno",
                            top,
                            get!("t_soisno", top)
                                + old["t_soisno"].at(p, s) * cvsoil[k][s] * w / wgt[top]
                        );
                        if old["dz_sno"].at(p, s) > 0.0 {
                            let mass = old["wliq_soisno"].at(p, s) + old["wice_soisno"].at(p, s);
                            rhosnow[top] =
                                (mass / old["dz_sno"].at(p, s)).mul_add(w / sum_lcc, rhosnow[top]);
                        }
                    }
                    l -= 1;
                    if l < maxsnl + 1 {
                        break;
                    }
                }
                if wgt[top] > 0.0 {
                    let wice = get!("wice_soisno", top);
                    let wliq = get!("wliq_soisno", top);
                    set!("t_soisno", top, temperature(hc[top], wice, wliq));
                    let dz = (wice + wliq) / rhosnow[top];
                    set!("dz_sno", top, dz);
                    set!("z_sno", top, (-0.5f64).mul_add(dz, 0.0));
                }
            }

            // 土壤层与其余量按份额混合。
            let mut rows: Vec<&str> = MIXED_ROWS.to_vec();
            rows.extend(MIXED_ROWS_TAIL);
            if options.plant_hydraulics {
                rows.extend(["vegwp", "gs0sun", "gs0sha"]);
            }
            if options.ozone_stress {
                rows.push("lai_old");
            }
            if !options.variably_saturated_flow {
                rows.push("zwt");
            }
            for k in 0..num {
                let p = sources[k];
                let w = weight(k);
                for s in SNOW_LAYERS..layers {
                    set!(
                        "wliq_soisno",
                        s,
                        get!("wliq_soisno", s) + old["wliq_soisno"].at(p, s) * w / sum_lcc
                    );
                    set!(
                        "wice_soisno",
                        s,
                        get!("wice_soisno", s) + old["wice_soisno"].at(p, s) * w / sum_lcc
                    );
                    set!(
                        "t_soisno",
                        s,
                        get!("t_soisno", s) + old["t_soisno"].at(p, s) * cvsoil[k][s] * w / wgt[s]
                    );
                }
                for &name in &rows {
                    for i in 0..state[name].row {
                        set!(name, i, get!(name, i) + old[name].at(p, i) * w / sum_lcc);
                    }
                }
            }

            // 雪水当量与雪深来自混合后的雪层。
            for s in 0..SNOW_LAYERS {
                if get!("z_sno", s) < 0.0 {
                    set!(
                        "scv",
                        0,
                        get!("scv", 0) + get!("wice_soisno", s) + get!("wliq_soisno", s)
                    );
                    set!("snowdp", 0, get!("snowdp", 0) + get!("dz_sno", s));
                }
            }
            let fraction = colm_core::snow::snow_fraction(
                tlai[np],
                tsai[np],
                get!("z0m", 0),
                ZLND,
                get!("scv", 0),
                get!("snowdp", 0),
                options.snow_cover_exponent,
            )
            .with_context(|| format!("snowfraction for new patch {np}"))?;
            set!("sigf", 0, fraction.vegetation_free_fraction);
            set!("fsno", 0, fraction.ground_snow_fraction);
            set!("sai", 0, tsai[np] * get!("sigf", 0));
            if options.vegetation_snow {
                set!("lai", 0, tlai[np] * get!("sigf", 0));
            }
            let z_sum: f64 = (0..SNOW_LAYERS).map(|s| get!("z_sno", s)).sum();
            let t_grnd = if z_sum == 0.0 {
                get!("t_soisno", SNOW_LAYERS)
            } else {
                let s = (0..SNOW_LAYERS)
                    .find(|&s| get!("z_sno", s) < 0.0)
                    .expect("a nonzero snow column has a layer with negative depth");
                get!("t_soisno", s)
            };
            set!("t_grnd", 0, t_grnd);

            if options.variably_saturated_flow {
                ensure!(
                    get!("wa", 0) >= 0.0,
                    "new patch {np}: the mixed water table is below the soil column \
                     (get_zwt_from_wa is not ported)"
                );
                let mut zwt = get!("zwt", 0);
                for l in (0..soil_layers).rev() {
                    let s = SNOW_LAYERS + l;
                    let vf_water = (get!("wliq_soisno", s) / DENH2O) / dz_soi[l];
                    let vf_ice = (get!("wice_soisno", s) / DENICE) / dz_soi[l];
                    if vf_water + vf_ice < porsl.at(np, l) {
                        zwt = zi_soi[l + 1];
                        break;
                    } else if l == 0 {
                        zwt = 0.0;
                    }
                }
                set!("zwt", 0, zwt);
            }
        }
    }

    let pft = match &inputs.pft {
        Some(pft) => pft_tail(pft, &mut state, &matched, new_type, options)?,
        None => Vec::new(),
    };
    // 没有需要混合的 patch、也没有 PFT 尾段时 SAT 原样返回。
    if !touched && inputs.pft.is_none() {
        return Ok(MecResult { patch: sat, pft });
    }
    let mut overrides: Vec<RestartOverride> = sat
        .into_iter()
        .filter(|entry| !state.contains_key(entry.name.as_str()))
        .collect();
    overrides.extend(
        state
            .into_iter()
            .map(|(name, field)| RestartOverride::new(name, field.values)),
    );
    Ok(MecResult {
        patch: overrides,
        pft,
    })
}

/// PFT 尾段（`MOD_Lulcc_MassEnergyConserve.F90:876-905`），对 `matched` 里的每个土壤 patch：
/// 按混合后的 `ldew` 重分 `ldew_p`，`snowfraction_pftwrap` 重算 `sigf_p/sigf/fsno`，
/// 再由 `tsai_p*sigf_p`（`DEF_VEG_SNOW` 时还有 `tlai_p*sigf_p`）重算 `sai(_p)`/`lai(_p)`。
/// 求和都是从 0 起的顺序 FMA；`ldew_p` 的缩放先算商 `ldew/ldew_tmp` 再乘。
fn pft_tail(
    pft: &MecPft<'_>,
    state: &mut BTreeMap<&str, Field>,
    matched: &[usize],
    new_type: &[i64],
    options: MecOptions,
) -> Result<Vec<RestartOverride>> {
    let pfts = pft.pft_class.len();
    ensure!(
        pft.pftfrac.len() == pfts && pft.htop.len() == pfts && pft.hbot.len() == pfts,
        "the PFT constants disagree on the PFT count"
    );
    let mut fields: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for name in ["ldew_p", "sigf_p", "sai_p", "lai_p", "z0m_p"] {
        let mut values = pft.time.floats(name)?.to_vec();
        if let Some(entry) = pft.sat.iter().find(|entry| entry.name == name) {
            ensure!(
                entry.values.len() == values.len(),
                "the SAT value of {name} does not fit the new PFT restart"
            );
            values.clone_from(&entry.values);
        }
        ensure!(values.len() == pfts, "{name} does not have one value per PFT");
        fields.insert(name, values);
    }
    let tlai = pft.time.floats("tlai_p")?;
    let tsai = pft.time.floats("tsai_p")?;
    let fsum = |values: &[f64], range: &std::ops::Range<usize>| {
        range
            .clone()
            .fold(0.0, |acc, ip| values[ip].mul_add(pft.pftfrac[ip], acc))
    };
    for &np in matched {
        if new_type[np] != 0 {
            continue;
        }
        let range = pft
            .ranges
            .get(np)
            .with_context(|| format!("no PFT range for new patch {np}"))?;
        macro_rules! patch {
            ($name:expr) => {{
                let field = state.get_mut($name).expect("the patch field is loaded");
                &mut field.values[np * field.row]
            }};
        }
        let ldew_tmp = fsum(&fields["ldew_p"], range);
        if ldew_tmp > 0.0 {
            let ratio = *patch!("ldew") / ldew_tmp;
            let ldew_p = fields.get_mut("ldew_p").expect("loaded");
            for ip in range.clone() {
                ldew_p[ip] *= ratio;
            }
        } else {
            *patch!("ldew") = 0.0;
        }
        let (scv, snowdp) = (*patch!("scv"), *patch!("snowdp"));
        for ip in range.clone() {
            // `MOD_SnowFraction.F90:126-146`；无冠层的 PFT 不读 `z0m_p`（裸土 PFT 可能是 0）。
            let vegetated = tlai[ip] + tsai[ip] > 1.0e-6;
            let mut sigf = 1.0;
            if vegetated {
                let wt = 0.1 * snowdp / fields["z0m_p"][ip];
                sigf = 1.0 - wt / (1.0 + wt);
            }
            if options.vegetation_snow && vegetated && (1..=8).contains(&pft.pft_class[ip]) {
                let buried = ((snowdp - pft.hbot[ip]).max(0.0) / (pft.htop[ip] - pft.hbot[ip]))
                    .min(1.0);
                sigf = 1.0 - buried;
            }
            fields.get_mut("sigf_p").expect("loaded")[ip] = sigf;
        }
        // `fsno` 与 PFT 无关：同一个 `snowfraction` 地面雪盖式子。
        let fsno = colm_core::snow::snow_fraction(
            0.0,
            0.0,
            1.0,
            ZLND,
            scv,
            snowdp,
            options.snow_cover_exponent,
        )
        .with_context(|| format!("snowfraction_pftwrap for new patch {np}"))?
        .ground_snow_fraction;
        *patch!("sigf") = fsum(&fields["sigf_p"], range);
        *patch!("fsno") = fsno;
        for ip in range.clone() {
            let sigf = fields["sigf_p"][ip];
            fields.get_mut("sai_p").expect("loaded")[ip] = tsai[ip] * sigf;
            if options.vegetation_snow {
                fields.get_mut("lai_p").expect("loaded")[ip] = tlai[ip] * sigf;
            }
        }
        *patch!("sai") = fsum(&fields["sai_p"], range);
        if options.vegetation_snow {
            *patch!("lai") = fsum(&fields["lai_p"], range);
        }
    }
    let mut overrides: Vec<RestartOverride> = pft
        .sat
        .iter()
        .filter(|entry| !fields.contains_key(entry.name.as_str()))
        .cloned()
        .collect();
    overrides.extend(
        fields
            .into_iter()
            .map(|(name, values)| RestartOverride::new(name, values)),
    );
    Ok(overrides)
}

#[cfg(test)]
#[path = "lulcc_mec_tests.rs"]
mod lulcc_mec_tests;

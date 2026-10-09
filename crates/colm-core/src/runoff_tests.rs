use super::*;

fn close(actual: f64, expected: f64) {
    let tolerance = 2.0e-12 * expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tolerance,
        "got {actual:.17e}, expected {expected:.17e}, tolerance {tolerance:.17e}"
    );
}

fn storage_input() -> StorageRunoffInput<'static> {
    StorageRunoffInput {
        layer_thickness_m: &[0.1, 0.2, 0.3, 0.4, 0.5, 0.6],
        effective_porosity: &[0.45; 6],
        liquid_volume_fraction: &[0.30; 6],
        water_input_mm_s: 5.0e-4,
        time_step_seconds: 1800.0,
    }
}

#[test]
fn topmodel_partitions_saturation_and_infiltration_excess() {
    let state = topmodel_surface_runoff(TopmodelSurfaceInput {
        impermeable_porosity: 0.05,
        saturated_hydraulic_conductivity_mm_s: &[0.004, 0.006, 0.009],
        effective_porosity: &[0.45, 0.45, 0.45],
        ice_fraction: &[0.1, 0.2, 0.0],
        saturated_fraction_max: 0.6,
        saturated_fraction_decay_m_inv: 0.4,
        decay_tuning: 0.75,
        water_table_depth_m: 1.2,
        water_input_mm_s: 0.01,
        method: TopmodelMethod::Exponential,
    })
    .unwrap();
    close(state.saturated_fraction, 0.418_605_795_642_618_6);
    close(
        state.saturation_excess_runoff_mm_s,
        0.004_186_057_956_426_186_5,
    );
    close(
        state.infiltration_excess_runoff_mm_s,
        0.005_593_841_077_607_31,
    );
    close(state.surface_runoff_mm_s, 0.009_779_899_034_033_496);
}

#[test]
fn topmodel_baseflow_uses_the_source_water_table_layer_and_ice_impedance() {
    let base = TopmodelSubsurfaceInput {
        method: TopmodelMethod::Exponential,
        layer_thickness_m: &[0.1, 0.3, 0.6],
        interface_depth_m: &[0.0, 0.1, 0.4, 1.0],
        ice_fraction: &[0.1, 0.2, 0.3],
        saturated_hydraulic_conductivity_mm_s: &[0.004, 0.006, 0.009],
        decay_tuning: 0.75,
        water_table_depth_m: 0.2,
        critical_topographic_index: None,
    };
    close(
        topmodel_subsurface_runoff(base).unwrap(),
        0.003_140_680_675_284_616_3,
    );
    let hydraulic = TopmodelSubsurfaceInput {
        method: TopmodelMethod::Hydraulic {
            mean_topographic_index: 5.0,
        },
        ..base
    };
    close(
        topmodel_subsurface_runoff(hydraulic).unwrap(),
        1.383_197_163_036_168_1,
    );
}

#[test]
fn storage_runoff_preserves_the_simple_vic_and_xinanjiang_branches() {
    let vic = simple_vic_runoff(storage_input(), 0.5).unwrap();
    close(vic.saturated_fraction, 0.306_638_725_649_365_23);
    close(vic.surface_runoff_mm_s, 0.000_153_433_852_284_026_41);
    assert_eq!(vic.subsurface_runoff_mm_s, 0.0);

    let xaj = xinanjiang_runoff(storage_input(), 300.0).unwrap();
    close(xaj.saturated_fraction, 0.136_258_408_210_171_32);
    close(xaj.surface_runoff_mm_s, 0.000_068_200_299_346_565_8);
}

#[test]
fn simple_vic_baseflow_reuses_the_shared_soil_hydraulic_curve() {
    let model = [SoilHydraulicModel::Campbell { bsw: 4.0 }; 10];
    let state = simple_vic_subsurface_runoff(SimpleVicSubsurfaceInput {
        layer_center_depth_m: &[0.05, 0.15, 0.25, 0.35, 0.45, 0.55, 0.65, 0.75, 0.85, 0.95],
        layer_thickness_m: &[0.1; 10],
        ice_water_kg_m2: &[0.0; 10],
        porosity: &[0.45; 10],
        saturated_potential_mm: &[-100.0; 10],
        saturated_hydraulic_conductivity_mm_s: &[
            0.001, 0.002, 0.003, 0.004, 0.005, 0.006, 0.007, 0.008, 0.009, 0.010,
        ],
        residual_water: &[0.05; 10],
        hydraulic_model: &model,
        maximum_soil_potential_mm: -1.0e5,
        water_table_depth_m: 1.5,
        soil_ice_impedance: 6.0,
        baseflow_fraction: 0.1,
        baseflow_threshold: 0.5,
    })
    .unwrap();
    close(state, 0.000_914_813_992_977_469_3);
}

#[test]
fn storage_runoff_rejects_the_source_out_of_bounds_layer_shape() {
    let invalid = StorageRunoffInput {
        layer_thickness_m: &[0.1; 5],
        effective_porosity: &[0.45; 5],
        liquid_volume_fraction: &[0.3; 5],
        ..storage_input()
    };
    assert!(simple_vic_runoff(invalid, 0.5).is_err());
}

// ---------------------------------------------------------------------------
// 与 vendor `MOD_Runoff.F90` 逐位比对（方法 0/1/2）。
//
// 金标准 `tests/data/topmodel_runoff_gfortran.txt` 由一个小 Fortran 驱动生成：vendor 的
// `MOD_Runoff.F90`、`MOD_IncompleteGamma.F90`、`MOD_Precision.F90` 原样编译，
// `MOD_Namelist` 等只留桩（只有 `DEF_TOPMOD_method`/`DEF_TUNING_TOPMOD_DECAY` 这几个模块量），
// 编译选项与 CoLM 空间构建相同；`surfacerunoff_topmod`、`subsurfacerunoff_topmod`、`gratio`
// 三个函数的 GIMPLE 与构建树里的转储归一化后逐条相同。驱动对每组输入调一次带齐可选参数的
// `SurfaceRunoff_TOPMOD`（同 `WATER_VSF`），再各调一次带/不带可选参数的
// `SubsurfaceRunoff_TOPMOD`（带齐的是 `WATER_VSF`/`groundwater` 现在的调用；不带的是
// 可选参数缺省时的方法 0 回退）。

const TOPMODEL_FIXTURE: &str = include_str!("../tests/data/topmodel_runoff_gfortran.txt");
const ETA_SENTINEL: u64 = 0x7FF8_DEAD_BEEF_0001;
const FIXTURE_LAYERS: usize = 10;

struct FortranTopmodelCase {
    method: i32,
    decay: f64,
    zwt: f64,
    gwat: f64,
    wimp: f64,
    fsatmax: f64,
    fsatdcf: f64,
    topoweti: f64,
    alp: f64,
    chi: f64,
    mu: f64,
    dz: Vec<f64>,
    hksati: Vec<f64>,
    icefrac: Vec<f64>,
    eff_porosity: Vec<f64>,
    zi: Vec<f64>,
    /// rsur rsur_se rsur_ie frcsat eta_out rsubst(带可选参数) rsubst(不带)
    expected: [u64; 7],
}

fn fixture_cases() -> Vec<FortranTopmodelCase> {
    let hex = |s: &str| u64::from_str_radix(s, 16).expect("十六进制 f64 位");
    TOPMODEL_FIXTURE
        .lines()
        .filter(|line| line.starts_with("T "))
        .map(|line| {
            let (inputs, outputs) = line[2..].split_once(" | ").expect("输入与输出以 | 分隔");
            let mut fields = inputs.split_whitespace();
            let method = fields.next().unwrap().parse().unwrap();
            let values: Vec<f64> = fields.map(|s| f64::from_bits(hex(s))).collect();
            assert_eq!(values.len(), 10 + 4 * FIXTURE_LAYERS + FIXTURE_LAYERS + 1);
            let layer = |k: usize| -> Vec<f64> {
                (0..FIXTURE_LAYERS)
                    .map(|j| values[10 + 4 * j + k])
                    .collect()
            };
            let expected: Vec<u64> = outputs.split_whitespace().map(hex).collect();
            FortranTopmodelCase {
                method,
                decay: values[0],
                zwt: values[1],
                gwat: values[2],
                wimp: values[3],
                fsatmax: values[4],
                fsatdcf: values[5],
                topoweti: values[6],
                alp: values[7],
                chi: values[8],
                mu: values[9],
                dz: layer(0),
                hksati: layer(1),
                icefrac: layer(2),
                eff_porosity: layer(3),
                zi: values[10 + 4 * FIXTURE_LAYERS..].to_vec(),
                expected: expected.try_into().expect("七个输出"),
            }
        })
        .collect()
}

impl FortranTopmodelCase {
    fn method(&self) -> TopmodelMethod {
        match self.method {
            0 => TopmodelMethod::Exponential,
            1 => TopmodelMethod::Hydraulic {
                mean_topographic_index: self.topoweti,
            },
            2 => TopmodelMethod::Gamma {
                mean_topographic_index: self.topoweti,
                alpha: self.alp,
                chi: self.chi,
                mu: self.mu,
            },
            other => panic!("金标准里没有方法 {other}"),
        }
    }

    /// `(rsur, rsur_se, rsur_ie, frcsat, eta_out, rsubst 带可选参数, rsubst 不带)` 的位模式。
    fn run(&self) -> [u64; 7] {
        let surface = topmodel_surface_runoff(TopmodelSurfaceInput {
            impermeable_porosity: self.wimp,
            saturated_hydraulic_conductivity_mm_s: &self.hksati,
            effective_porosity: &self.eff_porosity,
            ice_fraction: &self.icefrac,
            saturated_fraction_max: self.fsatmax,
            saturated_fraction_decay_m_inv: self.fsatdcf,
            decay_tuning: self.decay,
            water_table_depth_m: self.zwt,
            water_input_mm_s: self.gwat,
            method: self.method(),
        })
        .unwrap();
        let subsurface = TopmodelSubsurfaceInput {
            method: self.method(),
            layer_thickness_m: &self.dz,
            interface_depth_m: &self.zi,
            ice_fraction: &self.icefrac,
            saturated_hydraulic_conductivity_mm_s: &self.hksati,
            decay_tuning: self.decay,
            water_table_depth_m: self.zwt,
            critical_topographic_index: surface.critical_topographic_index,
        };
        let full = topmodel_subsurface_runoff(subsurface).unwrap();
        // 不带可选参数的那种调用（上游修第 57 条之前的 `groundwater`）：恒为方法 0 的式子。
        let bare = topmodel_subsurface_runoff(TopmodelSubsurfaceInput {
            method: TopmodelMethod::Exponential,
            critical_topographic_index: None,
            ..subsurface
        })
        .unwrap();
        [
            surface.surface_runoff_mm_s.to_bits(),
            surface.saturation_excess_runoff_mm_s.to_bits(),
            surface.infiltration_excess_runoff_mm_s.to_bits(),
            surface.saturated_fraction.to_bits(),
            surface
                .critical_topographic_index
                .map_or(ETA_SENTINEL, f64::to_bits),
            full.to_bits(),
            bare.to_bits(),
        ]
    }
}

#[test]
fn topmodel_matches_vendor_fortran_bitwise() {
    colm_numeric::skip_unless_fused!();
    let cases = fixture_cases();
    let mut per_method = [0usize; 3];
    let mut bad = Vec::new();
    for (row, case) in cases.iter().enumerate() {
        // 方法 2 下 Fortran 不收敛时 `eta`、`fsat` 照用：也要逐位相同。
        per_method[case.method as usize] += 1;
        let got = case.run();
        let matches = got.iter().zip(&case.expected).all(|(&got, &want)| {
            crate::reference_bits_match(f64::from_bits(got), f64::from_bits(want), 0.0)
        });
        if !matches {
            bad.push(format!(
                "第 {row} 组（方法 {}）：{:016X?} vs {:016X?}",
                case.method, got, case.expected
            ));
        }
    }
    eprintln!(
        "TOPMODEL：{} 组（方法 0/1/2 = {per_method:?}），按位不一致 {} 组",
        cases.len(),
        bad.len()
    );
    for line in bad.iter().take(10) {
        eprintln!("  {line}");
    }
    assert!(per_method.iter().all(|&count| count > 100));
    assert!(bad.is_empty(), "{} 组与 gfortran 不一致", bad.len());
}

#[test]
fn topmodel_gamma_saturates_fully_above_the_water_table() {
    // 手挑第 0 组：zwt = -0.2 ⇒ fsat = 1、eta = mu_twi，`rsubst` 用 exp(-mu_twi)。
    let case = &fixture_cases()[0];
    assert_eq!((case.method, case.zwt, case.mu), (2, -0.2, 6.95));
    let got = case.run();
    assert_eq!(f64::from_bits(got[3]), 1.0);
    assert_eq!(f64::from_bits(got[4]), 6.95);
    assert_eq!(got, case.expected);
}

/// 方法 2 的结果确实收敛：`eta > mu`、`0 < fsat < 1`、`|gfun|` 在判据以内，且与金标准逐位相同。
fn assert_gamma_converged(case: &FortranTopmodelCase) -> f64 {
    let got = case.run();
    let (fsat, eta) = (f64::from_bits(got[3]), f64::from_bits(got[4]));
    assert!(
        fsat > 0.0 && fsat < 1.0 && eta > case.mu,
        "fsat={fsat} eta={eta}"
    );
    let (p1, _) = crate::incomplete_gamma::gratio(case.alp + 1.0, (eta - case.mu) / case.chi);
    let (p0, _) = crate::incomplete_gamma::gratio(case.alp, (eta - case.mu) / case.chi);
    let gfun = ((eta - case.mu) * p0 - case.chi * case.alp * p1) / case.decay - case.zwt;
    assert!(gfun.abs() < 1.0e-5, "gfun={gfun}");
    assert_eq!(got, case.expected);
    eta
}

#[test]
fn topmodel_gamma_iterates_to_the_critical_index() {
    // 手挑第 1 组：默认参数（topoweti 9.27、alp 1.34、chi 1.61、mu 6.95）、zwt 1.7 m，正常收敛。
    let case = &fixture_cases()[1];
    assert_eq!((case.method, case.zwt, case.topoweti), (2, 1.7, 9.27));
    assert!(assert_gamma_converged(case) > case.topoweti);
}

#[test]
fn topmodel_gamma_starts_from_the_distribution_mean_below_its_floor() {
    // upstream-bugs 第 58 条（vendor 已修）：`topoweti <= mu_twi` 时初值改为 `mu + alp*chi`。
    // 修之前第一轮 x 截到 0、`pgr0 = 0` 立即退出，fsat = 1（全饱和、与 zwt 无关）。
    // 手挑第 2 组 topoweti 5 < mu 6.95（zwt 0.8 m），第 5 组 topoweti = mu = 6.95（zwt 1.7 m）：
    // 现在都正常收敛。第 5 组与第 1 组（同参数、同 zwt，只是从 9.27 起步）收敛到同一个根附近。
    let cases = fixture_cases();
    let (below, equal) = (&cases[2], &cases[5]);
    assert_eq!((below.method, below.topoweti, below.mu), (2, 5.0, 6.95));
    assert_eq!((equal.method, equal.topoweti, equal.mu), (2, 6.95, 6.95));
    assert_gamma_converged(below);
    let eta = assert_gamma_converged(equal);
    assert!((eta - f64::from_bits(cases[1].run()[4])).abs() < 1.0e-3);
}

#[test]
fn topmodel_gamma_rejects_a_restart_without_twi_parameters() {
    // 方法 0/1 建的常数重启里 `chi_twi = 0`；上游会在 `(eta-mu)/chi` 处除零中止。
    let case = &fixture_cases()[1];
    let error = topmodel_surface_runoff(TopmodelSurfaceInput {
        impermeable_porosity: case.wimp,
        saturated_hydraulic_conductivity_mm_s: &case.hksati,
        effective_porosity: &case.eff_porosity,
        ice_fraction: &case.icefrac,
        saturated_fraction_max: case.fsatmax,
        saturated_fraction_decay_m_inv: case.fsatdcf,
        decay_tuning: case.decay,
        water_table_depth_m: case.zwt,
        water_input_mm_s: case.gwat,
        method: TopmodelMethod::Gamma {
            mean_topographic_index: 0.0,
            alpha: 0.0,
            chi: 0.0,
            mu: 0.0,
        },
    })
    .unwrap_err();
    assert!(error.to_string().contains("chi_twi"), "{error}");
}

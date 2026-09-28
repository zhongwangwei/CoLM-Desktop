//! 城市湍流通量：`MOD_Urban_Flux.F90` 的 `UrbanOnlyFlux`（无树）与 `UrbanVegFlux`（有树）。
//!
//! 两个子程序都是两层"冠层"（`run_three_layer = .false.`，所以 `numlay = 2`、`botlay = 2`，
//! 三层那些分支是死代码）：第 3 层是屋顶高度以上，第 2 层是街谷。`alpha_opt = 3`，
//! `DEF_URBAN_Irrigation` 是模块常量 `.true.`（第二次 `stomata`、`etr_deficit` 都是活的）。
//!
//! 收缩形状逐句对照 `MOD_Urban_Flux.F90` 的 `-fdump-tree-optimized-lineno`
//! （`-O2 -ffp-contract=fast`）；每处 `mul_add` 旁注了行号。

use anyhow::{ensure, Result};

use crate::{
    canopy_diffusivity_resistance_analytic, canopy_monin_obukhov_with_scheme, canopy_roughness,
    canopy_wetness, effective_canopy_wind_between, initialize_monin_obukhov,
    saturation_specific_humidity, stomata, CanopyDiffusivityProfileInput, CanopyMoninObukhovInput,
    CanopyWater, CanopyWindProfileInput, LeafBiochemistry, LeafPhotosynthesisInput, LibmPow,
    MoninObukhovInitialInput, MoninObukhovInput, StomataInput, StomataOptions, SurfaceLayerScheme,
};

const CPAIR: f64 = 1004.64;
const CPLIQ: f64 = 4188.0;
const CPICE: f64 = 2117.27;
const HVAP: f64 = 2.5104e6;
const HFUS: f64 = 0.3336e6;
const TFRZ: f64 = 273.16;
const VONKAR: f64 = 0.4;
const GRAV: f64 = 9.80616;
/// 人为热里感热、潜热的份额（`fsh`/`flh`）。
const FSH: f64 = 0.92;
const FLH: f64 = 0.08;
/// `rsoil = 0.22*1e-6`：土壤呼吸（mol m⁻² s⁻¹）。
const RSOIL: f64 = 0.22 * 1.0e-6;
/// `0.5*1.2/vonkar/vonkar`：gfortran 左结合折成一个常量。
const DRAG_FACTOR: f64 = 0.5 * 1.2 / VONKAR / VONKAR;

/// 两个子程序共用的输入（上游同名变量）。
#[derive(Debug, Clone, Copy)]
pub struct UrbanFluxInput {
    pub time_step_seconds: f64,
    /// `lbr < 1`、`lbi < 1`：屋顶/不透水面有雪层。
    pub roof_has_snow_layers: bool,
    pub impervious_has_snow_layers: bool,
    /// `hu`/`ht`/`hq` 与 `HEIGHT_mode == 'absolute'`。
    pub wind_height_m: f64,
    pub temperature_height_m: f64,
    pub humidity_height_m: f64,
    pub absolute_heights: bool,
    pub eastward_wind_m_s: f64,
    pub northward_wind_m_s: f64,
    pub thm: f64,
    pub th: f64,
    pub thv: f64,
    pub qm: f64,
    pub psrf: f64,
    pub rhoair: f64,
    pub fhac: f64,
    pub fwst: f64,
    pub fach: f64,
    pub vehc: f64,
    pub meta: f64,
    pub hroof: f64,
    pub hlr: f64,
    /// `fcover(0:5)`：屋顶、阳墙、阴墙、不透水地、透水地、树。
    pub fcover: [f64; 6],
    pub z0h_g: f64,
    pub obug: f64,
    pub ustarg: f64,
    pub zlnd: f64,
    pub zsno: f64,
    pub fsno_roof: f64,
    pub fsno_gimp: f64,
    pub fsno_gper: f64,
    /// 屋顶、不透水面第 1 层的液/固水（`wliq_roofsno(1)` 等）。
    pub wliq_roof: f64,
    pub wliq_gimp: f64,
    pub wice_roof: f64,
    pub wice_gimp: f64,
    pub htvp_roof: f64,
    pub htvp_gimp: f64,
    pub htvp_gper: f64,
    pub troof: f64,
    pub twsun: f64,
    pub twsha: f64,
    pub tgimp: f64,
    pub tgper: f64,
    pub qroof: f64,
    pub qgimp: f64,
    pub qgper: f64,
    pub dqroofdt: f64,
    pub dqgimpdt: f64,
    pub dqgperdt: f64,
    pub rss: f64,
    pub surface_layer_scheme: SurfaceLayerScheme,
}

/// `UrbanVegFlux` 额外的树冠输入。
#[derive(Debug, Clone, Copy)]
pub struct UrbanTreeInput {
    pub frl: f64,
    pub po2m: f64,
    pub pco2m: f64,
    pub par: f64,
    pub sabv: f64,
    pub rstfac: f64,
    pub ewall: f64,
    pub egimp: f64,
    pub egper: f64,
    pub ev: f64,
    pub htop: f64,
    pub hbot: f64,
    pub lai: f64,
    pub sai: f64,
    pub sqrtdi: f64,
    pub extkd: f64,
    pub dewmx: f64,
    pub etrc: f64,
    pub trsmx0: f64,
    pub biochemistry: LeafBiochemistry,
    pub stomata_options: StomataOptions,
    pub wue_lambda: f64,
    pub vegetation_snow: bool,
    /// `UrbanVegLongwave` 的矩阵：`Ainv(5,5)`、`B`、`B1`、`dBdT`、`SkyVF`、`VegVF`
    /// （第 5 个元素是树，`B(5)`/`B1(5)`/`dBdT(5)` 是尚未乘温度幂的系数）。
    pub ainv: [[f64; 5]; 5],
    pub b: [f64; 5],
    pub b1: [f64; 5],
    pub dbdt: [f64; 5],
    pub sky_vf: [f64; 5],
    pub veg_vf: [f64; 5],
    /// 上一步留下的长波修正（`lwsun`…`lveg` 入参）。
    pub lwsun: f64,
    pub lwsha: f64,
    pub lgimp: f64,
    pub lgper: f64,
    pub lveg: f64,
}

/// 树冠状态（`intent(inout)`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UrbanTreeState {
    pub tl: f64,
    pub ldew: f64,
    pub ldew_rain: f64,
    pub ldew_snow: f64,
    pub fwet_snow: f64,
}

/// 两个子程序共同的输出。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UrbanFluxOutput {
    pub taux: f64,
    pub tauy: f64,
    pub fsenroof: f64,
    pub fsenwsun: f64,
    pub fsenwsha: f64,
    pub fsengimp: f64,
    pub fsengper: f64,
    pub fevproof: f64,
    pub fevpgimp: f64,
    pub fevpgper: f64,
    pub croofs: f64,
    pub cwsuns: f64,
    pub cwshas: f64,
    pub cgrnds: f64,
    pub croofl: f64,
    pub cgimpl: f64,
    pub cgperl: f64,
    pub croof: f64,
    pub cgimp: f64,
    pub cgper: f64,
    pub tref: f64,
    pub qref: f64,
    pub z0m: f64,
    pub zol: f64,
    pub rib: f64,
    pub ustar: f64,
    pub qstar: f64,
    pub tstar: f64,
    pub fm: f64,
    pub fh: f64,
    pub fq: f64,
    pub tafu: f64,
}

/// `UrbanVegFlux` 额外的输出。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UrbanTreeOutput {
    pub fsenl: f64,
    pub fevpl: f64,
    pub etr: f64,
    pub rst: f64,
    pub assim: f64,
    pub respc: f64,
    pub lwsun: f64,
    pub lwsha: f64,
    pub lgimp: f64,
    pub lgper: f64,
    pub lveg: f64,
    pub lout: f64,
    pub etr_deficit: f64,
    pub dheatl: f64,
}

/// 两个子程序共有的起步量（`:1367-1406` / `:397-436`）。
struct Surfaces {
    fg: f64,
    fgimp: f64,
    fgper: f64,
    tg: f64,
    fwet_roof_: f64,
    fwet_gimp_: f64,
    fwet_roof: f64,
    fwet_gimp: f64,
    qg: f64,
    z0mg: f64,
}

fn surfaces(input: &UrbanFluxInput) -> Surfaces {
    let fg = 1.0 - input.fcover[0];
    let fgimp = input.fcover[3] / fg;
    let fgper = input.fcover[4] / fg;
    // `:1367` `.FMA (tgimp, fgimp, tgper*fgper)`
    let tg = input.tgimp.mul_add(fgimp, input.tgper * fgper);
    // `(max(0, wliq+wice))**(2/3.)`：`2/3.` 是整数除以实数，得 0.6666…（不是字面量 .666666666666）。
    let wetness = |has_snow: bool, fsno: f64, liquid: f64, ice: f64| {
        if has_snow {
            fsno
        } else {
            (liquid + ice).max(0.0).lpow(2.0 / 3.0).min(1.0)
        }
    };
    let fwet_roof_ = wetness(
        input.roof_has_snow_layers,
        input.fsno_roof,
        input.wliq_roof,
        input.wice_roof,
    );
    let fwet_gimp_ = wetness(
        input.impervious_has_snow_layers,
        input.fsno_gimp,
        input.wliq_gimp,
        input.wice_gimp,
    );
    let fwet_roof = if input.qm > input.qroof {
        1.0
    } else {
        fwet_roof_
    };
    let fwet_gimp = if input.qm > input.qgimp {
        1.0
    } else {
        fwet_gimp_
    };
    let qg = mixed_ground_humidity(input, fgimp, fgper, fwet_gimp);
    let z0mg = if input.fsno_gper > 0.0 {
        input.zsno
    } else {
        input.zlnd
    };
    Surfaces {
        fg,
        fgimp,
        fgper,
        tg,
        fwet_roof_,
        fwet_gimp_,
        fwet_roof,
        fwet_gimp,
        qg,
        z0mg,
    }
}

/// `qg = (qgimp*fgimp*fwet_gimp + qgper*fgper) / (fgimp*fwet_gimp + fgper)`：
/// 分子 `.FMA (qgimp*fgimp, fwet_gimp, qgper*fgper)`（`:1406`）。
fn mixed_ground_humidity(input: &UrbanFluxInput, fgimp: f64, fgper: f64, fwet_gimp: f64) -> f64 {
    let fwetfac = fgper + fgimp * fwet_gimp;
    (input.qgimp * fgimp).mul_add(fwet_gimp, input.qgper * fgper) / fwetfac
}

/// `hroof*(1 + 4.43**(-λ)*(λ-1))`：`hroof * .FMA (pow(4.43,-λ), λ-1, 1)`（`:1435`）。
fn displacement(hroof: f64, lambda: f64) -> f64 {
    hroof * 4.43_f64.lpow(-lambda).mul_add(lambda - 1.0, 1.0)
}

/// `(hroof-d)*exp(-(3.75*(1-d/hroof)*fai)**(-0.5))`（`:1438`）。
fn roughness(hroof: f64, displacement_m: f64, frontal: f64) -> f64 {
    let argument = ((1.0 - displacement_m / hroof) * DRAG_FACTOR) * frontal;
    (hroof - displacement_m) * (-argument.lpow(-0.5)).exp()
}

/// 观测高度：`absolute` 时抬到 `hroof+1` 以上，否则相对屋顶（`:1526-1552`）。
fn observation_heights(input: &UrbanFluxInput) -> (f64, f64, f64) {
    let h = input.hroof;
    if input.absolute_heights {
        let lift = |height: f64| if height <= h + 1.0 { h + 1.0 } else { height };
        (
            lift(input.wind_height_m),
            lift(input.temperature_height_m),
            lift(input.humidity_height_m),
        )
    } else {
        (
            h + input.wind_height_m,
            h + input.temperature_height_m,
            h + input.humidity_height_m,
        )
    }
}

/// 每次迭代的湍流几何（`:1579-1701` / `:567-655`）。
struct Transport {
    ustar: f64,
    fm: f64,
    fh: f64,
    fq: f64,
    ram: f64,
    rah: f64,
    raw: f64,
    rd3: f64,
    rd2: f64,
    rb0: f64,
    rb12: f64,
    ueff_veg: Option<f64>,
}

#[allow(clippy::too_many_arguments)]
fn transport(
    input: &UrbanFluxInput,
    heights: (f64, f64, f64),
    displa: f64,
    z0m: f64,
    z0h: &mut f64,
    z0q: &mut f64,
    obu: f64,
    um: f64,
    z0mg: f64,
    displau: f64,
    z0mu: f64,
    alpha: f64,
    tree: Option<(f64, f64)>,
) -> Result<Transport> {
    let profile = canopy_monin_obukhov_with_scheme(
        CanopyMoninObukhovInput {
            surface: MoninObukhovInput {
                wind_height_m: heights.0,
                temperature_height_m: heights.1,
                humidity_height_m: heights.2,
                displacement_height_m: displa,
                momentum_roughness_m: z0m,
                heat_roughness_m: *z0h,
                moisture_roughness_m: *z0q,
                obukhov_length_m: obu,
                stability_adjusted_wind_m_s: um,
                boundary_layer_height_m: None,
            },
            top_layer_displacement_m: input.hroof,
            top_layer_roughness_m: 0.0,
            canopy_top_height_m: input.hroof,
        },
        input.surface_layer_scheme,
    )?;
    let surface = profile.surface;
    let ustar = surface.friction_velocity_m_s;
    let ram = 1.0 / ((ustar * ustar) / um);
    let rah = 1.0 / ((VONKAR / (surface.heat - profile.heat_at_top_layer)) * ustar);
    let raw = 1.0 / ((VONKAR / (surface.moisture - profile.moisture_at_top_layer)) * ustar);
    // `:1597` `z0mg/exp(pow((ustar*z0mg)/1.5e-5, 0.45)*0.13)`
    let z0hg = z0mg / (((ustar * z0mg) / 1.5e-5).lpow(0.45) * 0.13).exp();
    let z0qg = z0hg;
    *z0h = z0hg.max(*z0h);
    *z0q = z0qg.max(*z0q);
    let utop = (ustar / VONKAR) * profile.momentum_at_canopy_top;
    // `:1617` `(ustar*((hroof-displa)*0.4))/phih`
    let ktop = (ustar * ((input.hroof - displa) * VONKAR)) / profile.canopy_top_heat_similarity;
    let diffusivity = CanopyDiffusivityProfileInput {
        diffusivity_at_canopy_top_m2_s: ktop,
        canopy_cover_fraction: 1.0,
        canopy_blend_weight: 1.0,
        attenuation_coefficient: alpha,
        displacement_height_m: displa / input.hroof,
        canopy_top_height_m: input.hroof,
        canopy_bottom_height_m: 0.0,
        obukhov_length_m: input.obug,
        friction_velocity_m_s: input.ustarg,
    };
    let canopy_base = z0mu + displau;
    let rd3 =
        canopy_diffusivity_resistance_analytic(diffusivity, input.hroof, canopy_base, input.z0h_g)?;
    let wind = CanopyWindProfileInput {
        wind_at_canopy_top_m_s: utop,
        canopy_cover_fraction: 1.0,
        canopy_blend_weight: 1.0,
        attenuation_coefficient: alpha,
        ground_momentum_roughness_m: z0mg,
        canopy_top_height_m: input.hroof,
        canopy_bottom_height_m: 0.0,
    };
    let ueff2 = effective_canopy_wind_between(wind, input.hroof, z0mg)?;
    let rd2 = canopy_diffusivity_resistance_analytic(diffusivity, canopy_base, z0qg, input.z0h_g)?;
    let ueff_veg = match tree {
        Some((htop, hbot)) => Some(effective_canopy_wind_between(wind, htop, hbot)?),
        None => None,
    };
    let rhocp = input.rhoair * CPAIR;
    // `:1701` `rho*cp/.FMA (ueff, 4.2, 11.8)`
    let rb0 = rhocp / utop.mul_add(4.2, 11.8);
    let rb12 = rhocp / ueff2.mul_add(4.2, 11.8);
    Ok(Transport {
        ustar,
        fm: surface.momentum,
        fh: surface.heat,
        fq: surface.moisture,
        ram,
        rah,
        raw,
        rd3,
        rd2,
        rb0,
        rb12,
        ueff_veg,
    })
}

/// 人为热进两层的份额（`:1806-1807`）。
struct AnthropogenicHeat {
    hahe2: f64,
    hahe3: f64,
    lahe: f64,
}

fn anthropogenic_heat(input: &UrbanFluxInput) -> AnthropogenicHeat {
    let four_hlr = input.hlr * 4.0;
    let denominator = four_hlr + 1.0;
    let hac_wst = input.fhac + input.fwst;
    // `:1806` `.FMA (vehc, fsh, .FMA ((4hlr/(4hlr+1))*(Fhac+Fwst), fsh, Fach)) + meta`
    let hahe2 = input.vehc.mul_add(
        FSH,
        ((four_hlr / denominator) * hac_wst).mul_add(FSH, input.fach),
    ) + input.meta;
    let hahe3 = (hac_wst * (1.0 / denominator)) * FSH;
    let lahe = (hac_wst + input.vehc) * FLH;
    AnthropogenicHeat { hahe2, hahe3, lahe }
}

/// 稳定度更新（`:2219-2244` / `:750-776`），返回 `(tstar, qstar, zeta, obu, um)`。
fn stability(
    input: &UrbanFluxInput,
    zldis: f64,
    ur: f64,
    ustar: f64,
    fh: f64,
    fq: f64,
    taf2: f64,
    qaf2: f64,
) -> (f64, f64, f64, f64, f64) {
    let dth = input.thm - taf2;
    let dqh = input.qm - qaf2;
    let tstar = dth * (VONKAR / fh);
    let qstar = dqh * (VONKAR / fq);
    // `:2225` `.FMA (1+0.61qm, tstar, (th*0.61)*qstar)`
    let thvstar = input
        .qm
        .mul_add(0.61, 1.0)
        .mul_add(tstar, (input.th * 0.61) * qstar);
    let mut zeta = (((zldis * VONKAR) * GRAV) * thvstar) / (input.thv * (ustar * ustar));
    zeta = if zeta >= 0.0 {
        zeta.clamp(1.0e-6, 2.0)
    } else {
        zeta.clamp(-100.0, -1.0e-6)
    };
    let obu = zldis / zeta;
    let um = if zeta >= 0.0 {
        ur.max(0.1)
    } else {
        // `zii = 1000`、`beta = 1`
        let wc = (-((((ustar * GRAV) * thvstar) * 1000.0) / input.thv)).lpow(1.0 / 3.0);
        ur.mul_add(ur, wc * wc).sqrt()
    };
    (tstar, qstar, zeta, obu, um)
}

/// Port of `MOD_Urban_Flux:UrbanOnlyFlux`.
pub fn urban_bare_flux(input: UrbanFluxInput) -> Result<UrbanFluxOutput> {
    validate(&input)?;
    let s = surfaces(&input);
    let fc = input.fcover;
    let fg = s.fg;
    // `:459-473`
    let displau0 = displacement(input.hroof, fc[0]);
    // `:460` `froof*(hlr*(4/PI))`
    let fai = fc[0] * (input.hlr * (4.0 / std::f64::consts::PI));
    let mut z0mu = roughness(input.hroof, displau0, fai);
    if z0mu < s.z0mg {
        z0mu = s.z0mg;
    }
    let z0m = z0mu;
    let displa = displau0;
    let displau = displau0.max(input.hroof * 0.5);
    let alpha = fai * 9.6;
    // `:503-504` `.FMA (tg, 2, thm)/3`
    let mut taf2 = s.tg.mul_add(2.0, input.thm) / 3.0;
    let mut qaf2 = s.qg.mul_add(2.0, input.qm) / 3.0;
    let mut taf3;
    let mut qaf3;
    let mut z0h = z0m;
    let mut z0q = z0m;
    let ur = input
        .eastward_wind_m_s
        .mul_add(
            input.eastward_wind_m_s,
            input.northward_wind_m_s * input.northward_wind_m_s,
        )
        .sqrt()
        .max(0.1);
    let dth = input.thm - taf2;
    let dqh = input.qm - qaf2;
    let dthv = dth.mul_add(input.qm.mul_add(0.61, 1.0), dqh * (input.th * 0.61));
    let heights = observation_heights(&input);
    let zldis = heights.0 - displa;
    ensure!(
        zldis > 0.0,
        "the urban wind height is below the displacement height"
    );
    let initial = initialize_monin_obukhov(MoninObukhovInitialInput {
        reference_wind_m_s: ur,
        potential_temperature_k: input.th,
        reference_temperature_k: input.thm,
        virtual_potential_temperature_k: input.thv,
        temperature_difference_k: dth,
        humidity_difference_kg_kg: dqh,
        virtual_temperature_difference_k: dthv,
        reference_height_m: zldis,
        momentum_roughness_m: z0m,
    })?;
    let mut um = initial.stability_adjusted_wind_m_s;
    let mut obu = initial.obukhov_length_m;
    let mut obuold = 0.0;
    let mut nmozsgn = 0;
    let anthro = anthropogenic_heat(&input);
    let rhocp = input.rhoair * CPAIR;
    let mut fwet_roof = s.fwet_roof;
    let mut fwet_gimp = s.fwet_gimp;
    let mut qg;
    let mut rss_;
    let mut t;
    let mut zeta;
    let mut tstar;
    let mut qstar;
    let mut bt;
    let mut ct;
    let mut iteration = 0;
    loop {
        iteration += 1;
        t = transport(
            &input, heights, displa, z0m, &mut z0h, &mut z0q, obu, um, s.z0mg, displau, z0mu,
            alpha, None,
        )?;
        let (rd3, rd2, rb0, rb1, rb2) = (t.rd3, t.rd2, t.rb0, t.rb12, t.rb12);
        // 温度（`:691-700`）
        bt = 1.0 / (rd3 * ((1.0 / t.rah + 1.0 / rd3) + fc[0] / rb0));
        ct = ((1.0 / rd3 + fg / rd2) + fc[1] / rb1) + fc[2] / rb2;
        let top_heat = ((input.troof * fc[0]) / rb0 + anthro.hahe3 / rhocp) + input.thm / t.rah;
        let denominator_t = (1.0 - bt / (rd3 * ct)) * ct;
        // `:697` 分子里的 `aT` 收成 `.FMA (…, bT, …)`（与 VegFlux 不同）
        taf2 = top_heat.mul_add(
            bt,
            (((fg * s.tg) / rd2 + anthro.hahe2 / rhocp) + (input.twsun * fc[1]) / rb1)
                + (input.twsha * fc[2]) / rb2,
        ) / denominator_t;
        taf3 = ((((taf2 / rd3) + (input.troof * fc[0]) / rb0) + anthro.hahe3 / rhocp)
            + input.thm / t.rah)
            / ((1.0 / t.rah + 1.0 / rd3) + fc[0] / rb0);
        // 湿度（`:702-718`）
        rss_ = if input.qgper < qaf2 { 0.0 } else { input.rss };
        let cq = (1.0 / rd3 + (fg * s.fgper) / (rd2 + rss_)) + ((fg * fwet_gimp) * s.fgimp) / rd2;
        let bq_inner = (1.0 / rd3 + 1.0 / t.raw) + (fc[0] * fwet_roof) / rb0;
        let bq = 1.0 / (rd3 * bq_inner);
        let roof_moisture = (fc[0] * (input.qroof * fwet_roof)) / rb0;
        let top_moisture = roof_moisture + input.qm / t.raw;
        qaf2 = (top_moisture.mul_add(
            bq,
            ((input.qgper * s.fgper) * fg) / (rd2 + rss_)
                + (((input.qgimp * fwet_gimp) * s.fgimp) * fg) / rd2,
        ) + (anthro.lahe / input.rhoair) / HVAP)
            / ((1.0 - bq / (rd3 * cq)) * cq);
        qaf3 = ((roof_moisture + qaf2 / rd3) + input.qm / t.raw) / bq_inner;
        fwet_roof = if qaf3 > input.qroof {
            1.0
        } else {
            s.fwet_roof_
        };
        fwet_gimp = if qaf2 > input.qgimp {
            1.0
        } else {
            s.fwet_gimp_
        };
        qg = mixed_ground_humidity(&input, s.fgimp, s.fgper, fwet_gimp);
        let updated = stability(&input, zldis, ur, t.ustar, t.fh, t.fq, taf2, qaf2);
        tstar = updated.0;
        qstar = updated.1;
        zeta = updated.2;
        obu = updated.3;
        um = updated.4;
        if obuold * obu < 0.0 {
            nmozsgn += 1;
        }
        if nmozsgn >= 4 || iteration == 6 {
            break;
        }
        obuold = obu;
    }
    let (rd3, rd2, rb0, rb1, rb2) = (t.rd3, t.rd2, t.rb0, t.rb12, t.rb12);
    let rib = ((zeta * (t.ustar * t.ustar)) / (((VONKAR * VONKAR) / t.fh) * (um * um))).min(5.0);
    let fsenroof = (rhocp / rb0) * (input.troof - taf3);
    let fsenwsun = (rhocp / rb1) * (input.twsun - taf2);
    let fsenwsha = (rhocp / rb2) * (input.twsha - taf2);
    let fevproof = ((input.rhoair / rb0) * (input.qroof - qaf3)) * fwet_roof;
    // `:799-800`：用终值 `fwet_*` 重算 `cQ`/`bQ`（`bT`/`cT` 与循环里的相同）。
    let cq = (1.0 / rd3 + (fg * s.fgper) / (rd2 + rss_)) + ((fwet_gimp * fg) * s.fgimp) / rd2;
    let bq_inner = (1.0 / rd3 + 1.0 / t.raw) + (fc[0] * fwet_roof) / rb0;
    let bq = 1.0 / (rd3 * bq_inner);
    let one_minus_bt = 1.0 - bt / (rd3 * ct);
    let one_minus_bq = 1.0 - bq / (rd3 * cq);
    let cwsuns = (rhocp / rb1) * (1.0 - fc[1] / (one_minus_bt * (rb1 * ct)));
    let cwshas = (rhocp / rb2) * (1.0 - fc[2] / (one_minus_bt * (rb2 * ct)));
    let bt_inner = (1.0 / t.rah + 1.0 / rd3) + fc[0] / rb0;
    let croofs = (rhocp / rb0)
        * ((1.0 - ((fc[0] * bt) * bt) / (one_minus_bt * (rb0 * ct))) - fc[0] / (rb0 * bt_inner));
    let wet_roof_cover = fc[0] * fwet_roof;
    // `:812`
    let croofl = (((fwet_roof * input.rhoair) / rb0) * input.dqroofdt)
        * ((1.0 - ((wet_roof_cover * bq) * bq) / ((rb0 * cq) * one_minus_bq))
            - wet_roof_cover / (rb0 * bq_inner));
    let croof = croofl.mul_add(input.htvp_roof, croofs);
    let taux = -((input.eastward_wind_m_s * input.rhoair) / t.ram);
    let tauy = -((input.northward_wind_m_s * input.rhoair) / t.ram);
    let fsengper = (rhocp / rd2) * (input.tgper - taf2);
    let fsengimp = (rhocp / rd2) * (input.tgimp - taf2);
    let fevpgper = (input.rhoair / (rd2 + rss_)) * (input.qgper - qaf2);
    let fevpgimp = fwet_gimp * ((input.rhoair / rd2) * (input.qgimp - qaf2));
    let cgrnds = (rhocp / rd2) * (1.0 - fg / (one_minus_bt * (rd2 * ct)));
    let cgperl = ((input.rhoair / (rd2 + rss_)) * input.dqgperdt)
        * (1.0 - (fg * s.fgper) / (one_minus_bq * ((rd2 + rss_) * cq)));
    let cgimpl = (((fwet_gimp * input.rhoair) / rd2) * input.dqgimpdt)
        * (1.0 - ((fwet_gimp * fg) * s.fgimp) / (one_minus_bq * (rd2 * cq)));
    let cgimp = cgimpl.mul_add(input.htvp_gimp, cgrnds);
    let cgper = cgperl.mul_add(input.htvp_gper, cgrnds);
    // `:860` `.FMA (d-2, tg, taf2*2)/d`，`d = displau + z0mu`
    let reference = displau + z0mu;
    let tref = (reference - 2.0).mul_add(s.tg, taf2 * 2.0) / reference;
    let qref = (reference - 2.0).mul_add(qg, qaf2 * 2.0) / reference;
    Ok(UrbanFluxOutput {
        taux,
        tauy,
        fsenroof,
        fsenwsun,
        fsenwsha,
        fsengimp,
        fsengper,
        fevproof,
        fevpgimp,
        fevpgper,
        croofs,
        cwsuns,
        cwshas,
        cgrnds,
        croofl,
        cgimpl,
        cgperl,
        croof,
        cgimp,
        cgper,
        tref,
        qref,
        z0m,
        zol: zeta,
        rib,
        ustar: t.ustar,
        qstar,
        tstar,
        fm: t.fm,
        fh: t.fh,
        fq: t.fq,
        tafu: taf2,
    })
}

/// Port of `MOD_Urban_Flux:UrbanVegFlux`.
pub fn urban_vegetated_flux(
    input: UrbanFluxInput,
    tree: UrbanTreeInput,
    state: &mut UrbanTreeState,
) -> Result<(UrbanFluxOutput, UrbanTreeOutput)> {
    validate(&input)?;
    let s = surfaces(&input);
    let fc = input.fcover;
    let fg = s.fg;
    let fc3 = fc[5];
    let dt = input.time_step_seconds;
    let lai = tree.lai;
    let lsai = lai + tree.sai;
    // `:1315-1317` 冠层积分因子
    let cint = [
        (1.0 - (-(lai * 0.110)).exp()) / 0.110,
        (1.0 - (-(lai * tree.extkd)).exp()) / tree.extkd,
        lai,
    ];
    // `:1328` `.FMA (ldew_snow, cpice, .FMA (0.2*lsai, cpliq, ldew_rain*cpliq))`
    let clai = if tree.vegetation_snow {
        state
            .ldew_snow
            .mul_add(CPICE, (lsai * 0.2).mul_add(CPLIQ, state.ldew_rain * CPLIQ))
    } else {
        0.0
    };
    let wetness = canopy_wetness(
        lai,
        tree.sai,
        tree.dewmx,
        CanopyWater {
            total_mm: state.ldew,
            rain_mm: state.ldew_rain,
            snow_mm: state.ldew_snow,
        },
        tree.vegetation_snow,
    )?;
    let fwet = wetness.wet_fraction;
    let qsatl0 = input.qroof;
    let qsatldt0 = input.dqroofdt;
    let leaf = saturation_specific_humidity(state.tl, input.psrf)?;
    let mut qsatl3 = leaf.specific_humidity;
    let mut qsatldt3 = leaf.specific_humidity_temperature_slope_k;
    let mut ei3 = leaf.vapor_pressure_pa;

    let displav_lay = canopy_roughness(lsai, tree.htop, fc3)?.displacement_height_m;
    let displau0 = displacement(input.hroof, fc[0]);
    let fai = fc[0] * (input.hlr * (4.0 / std::f64::consts::PI));
    let z0mu = roughness(input.hroof, displau0, fai);
    let faiv = fc3 * (1.0 - (-(lsai * 0.5)).exp());
    let lambda_ratio = (tree.htop * faiv) / input.hroof;
    let lambda = fc[0] + lambda_ratio;
    let mut displa = displacement(input.hroof, lambda).min(input.hroof * 0.95);
    let mut z0m = (input.hroof - displa)
        * (-((((1.0 - displa / input.hroof) * DRAG_FACTOR) * (lambda_ratio + fai)).lpow(-0.5)))
            .exp();
    if z0m < s.z0mg {
        z0m = s.z0mg;
    }
    if displa >= input.hroof - s.z0mg {
        displa = input.hroof - s.z0mg;
    }
    let displau = displau0.max(input.hroof * 0.5);
    // `:1468-1481`
    let sqrtdragc = faiv.mul_add(0.3, 0.003).lpow(0.5).min(0.3);
    let alphav = (tree.htop / (tree.htop - displav_lay)) / (VONKAR / sqrtdragc);
    let alphav = (tree.htop * alphav) / input.hroof;
    let alpha = fai.mul_add(9.6, alphav);

    let mut taf2 = s.tg.mul_add(2.0, input.thm) / 3.0;
    let mut qaf2 = s.qg.mul_add(2.0, input.qm) / 3.0;
    let mut taf3;
    let mut qaf3;
    let mut pco2a = tree.pco2m;
    // `:1511` `((44.6*273.16)*psrf)/1.013e5`
    let tprcor = (input.psrf * (44.6 * 273.16)) / 1.013e5;
    let mut z0h = z0m;
    let mut z0q = z0m;
    let ur = input
        .eastward_wind_m_s
        .mul_add(
            input.eastward_wind_m_s,
            input.northward_wind_m_s * input.northward_wind_m_s,
        )
        .sqrt()
        .max(0.1);
    let dth = input.thm - taf2;
    let dqh = input.qm - qaf2;
    // `:1524` `.FMA (dth, 1+0.61qm, dqh*(th*0.61))`
    let dthv = dth.mul_add(input.qm.mul_add(0.61, 1.0), dqh * (input.th * 0.61));
    let heights = observation_heights(&input);
    let zldis = heights.0 - displa;
    ensure!(
        zldis > 0.0,
        "the urban wind height is below the displacement height"
    );
    let initial = initialize_monin_obukhov(MoninObukhovInitialInput {
        reference_wind_m_s: ur,
        potential_temperature_k: input.th,
        reference_temperature_k: input.thm,
        virtual_potential_temperature_k: input.thv,
        temperature_difference_k: dth,
        humidity_difference_kg_kg: dqh,
        virtual_temperature_difference_k: dthv,
        reference_height_m: zldis,
        momentum_roughness_m: z0m,
    })?;
    let mut um = initial.stability_adjusted_wind_m_s;
    let mut obu = initial.obukhov_length_m;
    let mut obuold = 0.0;
    let mut nmozsgn = 0;
    let anthro = anthropogenic_heat(&input);
    let rhocp = input.rhoair * CPAIR;
    let mut fwet_roof = s.fwet_roof;
    let mut fwet_gimp = s.fwet_gimp;
    let mut qg;

    let mut del = 0.0;
    let mut dele = 0.0;
    let mut fevpl_bef = 0.0;
    let mut dtl_prev: Option<f64> = None;
    let mut rss_;
    let mut iteration = 1;
    // 循环里一直在变的量，出循环后要用终值
    let mut t;
    let mut zeta;
    let mut tstar;
    let mut qstar;
    let mut bt;
    let mut ct;
    let mut rv;
    let mut rs;
    let mut rs_;
    let mut assim;
    let mut respc;
    let mut fsenl;
    let mut fsenl_dtl;
    let mut etr;
    let mut etr_dtl;
    let mut etr_;
    let mut evplwet;
    let mut evplwet_dtl;
    let mut fevpl_noadj;
    let mut fevpl_dtl;
    let mut erre;
    let mut denominator;
    let mut dtl_noadj;
    let mut dtl;
    let mut dirab_dtl;
    let mut x;
    let mut dx;
    // `B1(5)`/`dBdT(5)` 按**迭代开始时**的叶温算（`:1997-1999` 在 `tl` 更新之前），
    // 出循环后的 `lveg` 用的是最后一次迭代留下的这两个值。
    let mut b1_leaf;
    let mut dbdt_leaf;
    loop {
        let tlbef = state.tl;
        let del2 = del;
        let dele2 = dele;
        t = transport(
            &input,
            heights,
            displa,
            z0m,
            &mut z0h,
            &mut z0q,
            obu,
            um,
            s.z0mg,
            displau,
            z0mu,
            alpha,
            Some((tree.htop, tree.hbot)),
        )?;
        let (rd3, rd2, rb0, rb1, rb2) = (t.rd3, t.rd2, t.rb0, t.rb12, t.rb12);
        // `:1695` `(sqrtdi*0.01)*sqrt(ueff_veg)`
        let rb3 = 1.0 / ((tree.sqrtdi * 0.01) * t.ueff_veg.unwrap_or(0.0).sqrt());

        // 气孔（`:1719-1758`）
        if lai > 0.0 {
            // `:1722` `(psrf*qaf)/.FMA (qaf, 0.378, 0.622)`
            let eah = (input.psrf * qaf2) / qaf2.mul_add(0.378, 0.622);
            let call = |stress: f64| {
                stomata(
                    StomataInput {
                        photosynthesis: LeafPhotosynthesisInput {
                            biochemistry: tree.biochemistry,
                            canopy_integration: cint,
                            leaf_temperature_k: state.tl,
                            oxygen_partial_pressure_pa: tree.po2m,
                            absorbed_par_w_m2: tree.par,
                            air_pressure_pa: input.psrf,
                            soil_water_stress: stress,
                            leaf_boundary_resistance_s_m: rb3 / lai,
                        },
                        atmospheric_co2_pa: tree.pco2m,
                        canopy_air_co2_pa: pco2a,
                        canopy_air_vapor_pressure_pa: eah,
                        leaf_saturation_vapor_pressure_pa: ei3,
                        wue_lambda: tree.wue_lambda,
                    },
                    tree.stomata_options,
                )
            };
            let first = call(tree.rstfac)?;
            rs_ = first.stomatal_resistance_s_m;
            let result = if tree.rstfac < 1.0 { call(1.0)? } else { first };
            assim = result.assimilation_mol_m2_s;
            respc = result.respiration_mol_m2_s;
            rs = result.stomatal_resistance_s_m;
        } else {
            rs = 2.0e4;
            rs_ = 2.0e4;
            assim = 0.0;
            respc = 0.0;
        }
        rs *= lai;
        rs_ *= lai;
        let delta = if qsatl3 - qaf2 > 0.0 { 1.0 } else { 0.0 };
        let wet_delta = (1.0 - fwet) * delta;
        // `:1766` `1/.FMA ((1-fwet)*delta, lai/(rs+rb3), (lsai*(1-(1-fwet)*delta))/rb3)`
        rv = 1.0 / wet_delta.mul_add(lai / (rs + rb3), (lsai * (1.0 - wet_delta)) / rb3);

        // 两层温度（`:1806-1817`）
        bt = 1.0 / (rd3 * ((1.0 / t.rah + 1.0 / rd3) + fc[0] / rb0));
        ct = (((1.0 / rd3 + fg / rd2) + fc[1] / rb1) + fc[2] / rb2) + (lsai * fc3) / rb3;
        let at = (((input.troof * fc[0]) / rb0 + anthro.hahe3 / rhocp) + input.thm / t.rah) * bt;
        let wall_sum = (((fg * s.tg) / rd2 + anthro.hahe2 / rhocp) + (input.twsun * fc[1]) / rb1)
            + (input.twsha * fc[2]) / rb2;
        let denominator_t = (1.0 - bt / (rd3 * ct)) * ct;
        let one_minus_bt = 1.0 - bt / (rd3 * ct);
        taf2 = ((wall_sum + (lsai * (fc3 * state.tl)) / rb3) + at) / denominator_t;
        // 两层湿度（`:1819-1836`）
        rss_ = if input.qgper < qaf2 { 0.0 } else { input.rss };
        let cq = ((1.0 / rd3 + (fg * s.fgper) / (rd2 + rss_)) + ((fg * fwet_gimp) * s.fgimp) / rd2)
            + fc3 / rv;
        let bq_inner = (1.0 / rd3 + 1.0 / t.raw) + (fc[0] * fwet_roof) / rb0;
        let bq = 1.0 / (rd3 * bq_inner);
        let roof_moisture = (fc[0] * (qsatl0 * fwet_roof)) / rb0;
        let top_moisture = roof_moisture + input.qm / t.raw;
        let ground_moisture = ((input.qgper * s.fgper) * fg) / (rd2 + rss_)
            + (((input.qgimp * fwet_gimp) * s.fgimp) * fg) / rd2;
        let anthropogenic_moisture = (anthro.lahe / input.rhoair) / HVAP;
        let one_minus_bq = 1.0 - bq / (rd3 * cq);
        qaf2 = (top_moisture.mul_add(bq, ground_moisture + (fc3 * qsatl3) / rv)
            + anthropogenic_moisture)
            / (one_minus_bq * cq);

        // 叶片通量（`:1914-1989`）
        let sensible_conductance = (lsai * rhocp) / rb3;
        fsenl = sensible_conductance * (state.tl - taf2);
        fsenl_dtl = sensible_conductance * (1.0 - (lsai * fc3) / (one_minus_bt * (ct * rb3)));
        let transpiration_conductance =
            ((((1.0 - fwet) * input.rhoair) * delta) * lai) / (rs + rb3);
        let deficit = qsatl3 - qaf2;
        etr = transpiration_conductance * deficit;
        let moisture_feedback = 1.0 - fc3 / (one_minus_bq * (rv * cq));
        etr_dtl = (transpiration_conductance * moisture_feedback) * qsatldt3;
        if etr >= tree.trsmx0 * 1.0 {
            etr = tree.trsmx0 * 1.0;
            etr_dtl = 0.0;
        }
        let wet_conductance = (lsai * ((1.0 - wet_delta) * input.rhoair)) / rb3;
        evplwet = deficit * wet_conductance;
        evplwet_dtl = qsatldt3 * (moisture_feedback * wet_conductance);
        if evplwet >= state.ldew / dt {
            evplwet = state.ldew / dt;
            evplwet_dtl = 0.0;
        }
        let mut fevpl = etr + evplwet;
        fevpl_dtl = etr_dtl + evplwet_dtl;
        erre = 0.0;
        fevpl_noadj = fevpl;
        if fevpl * fevpl_bef < 0.0 {
            erre = -(fevpl * 0.9);
            fevpl *= 0.1;
        }
        etr_ = deficit * (((((1.0 - fwet) * input.rhoair) * delta) * lai) / (rs_ + rb3));
        if etr_ >= tree.etrc {
            etr_ = tree.etrc;
        }

        // 叶片长波（`:1997-2007`）
        let tl2 = state.tl * state.tl;
        let mut b = tree.b;
        let mut b1 = tree.b1;
        let mut dbdt = tree.dbdt;
        b[4] = tree.b[4] * (tl2 * tl2);
        b1[4] = tree.b1[4] * (tl2 * tl2);
        dbdt[4] = (tl2 * state.tl) * tree.dbdt[4];
        b1_leaf = b1[4];
        dbdt_leaf = dbdt[4];
        x = matmul(&tree.ainv, &b);
        let unit = [
            dbdt[0] * 0.0,
            dbdt[1] * 0.0,
            dbdt[2] * 0.0,
            dbdt[3] * 0.0,
            dbdt[4],
        ];
        dx = matmul(&tree.ainv, &unit);
        let mut received = 0.0;
        for i in 0..4 {
            received = x[i].mul_add(tree.veg_vf[i], received);
        }
        received = tree.frl.mul_add(tree.veg_vf[4], received);
        let irab = (received.mul_add(tree.ev, -b1[4]) / fc3).mul_add(fg, tree.lveg);
        let mut received_dtl = 0.0;
        for i in 0..4 {
            received_dtl = dx[i].mul_add(tree.veg_vf[i], received_dtl);
        }
        dirab_dtl = (tree.ev.mul_add(received_dtl, -dbdt[4]) / fc3) * fg;

        // 叶温增量（`:2010-2039`）
        denominator = ((clai / dt - dirab_dtl) + fsenl_dtl) + fevpl_dtl * HVAP;
        dtl = (-fevpl).mul_add(HVAP, (irab + tree.sabv) - fsenl) / denominator;
        dtl_noadj = dtl;
        if dtl.abs() > 3.0 {
            dtl = (dtl * 3.0) / dtl.abs();
        }
        if let Some(previous) = dtl_prev {
            if previous * dtl <= 0.0 {
                dtl = (previous + dtl) * 0.5;
            }
        }
        state.tl = tlbef + dtl;
        del = (dtl * dtl).sqrt();
        let hvap_dtl = HVAP * fevpl_dtl;
        dele = ((dtl * dtl)
            * hvap_dtl.mul_add(
                hvap_dtl,
                dirab_dtl.mul_add(dirab_dtl, fsenl_dtl * fsenl_dtl),
            ))
        .sqrt();
        let leaf = saturation_specific_humidity(state.tl, input.psrf)?;
        qsatl3 = leaf.specific_humidity;
        qsatldt3 = leaf.specific_humidity_temperature_slope_k;
        ei3 = leaf.vapor_pressure_pa;

        // 用新叶温重算两层（`:2051-2116`）
        taf2 = ((wall_sum + (lsai * (fc3 * state.tl)) / rb3) + at) / denominator_t;
        taf3 = ((((taf2 / rd3) + (input.troof * fc[0]) / rb0) + anthro.hahe3 / rhocp)
            + input.thm / t.rah)
            / ((1.0 / t.rah + 1.0 / rd3) + fc[0] / rb0);
        rss_ = if input.qgper < qaf2 { 0.0 } else { input.rss };
        let cq = ((1.0 / rd3 + (fg * s.fgper) / (rd2 + rss_)) + ((fg * fwet_gimp) * s.fgimp) / rd2)
            + fc3 / rv;
        let ground_moisture = ((input.qgper * s.fgper) * fg) / (rd2 + rss_)
            + (((input.qgimp * fwet_gimp) * s.fgimp) * fg) / rd2;
        qaf2 = (anthropogenic_moisture
            + top_moisture.mul_add(bq, ground_moisture + (fc3 * qsatl3) / rv))
            / ((1.0 - bq / (rd3 * cq)) * cq);
        qaf3 = ((roof_moisture + qaf2 / rd3) + input.qm / t.raw) / bq_inner;
        fwet_roof = if qaf3 > input.qroof {
            1.0
        } else {
            s.fwet_roof_
        };
        fwet_gimp = if qaf2 > input.qgimp {
            1.0
        } else {
            s.fwet_gimp_
        };
        qg = mixed_ground_humidity(&input, s.fgimp, s.fgper, fwet_gimp);

        // 冠层 CO2（`:2208-2212`）
        let gah2o = ((1.0 / t.raw) * tprcor) / input.thm;
        pco2a = (-((input.psrf * 1.37) / gah2o.max(0.446)))
            .mul_add((assim - respc) - RSOIL, tree.pco2m);

        let updated = stability(&input, zldis, ur, t.ustar, t.fh, t.fq, taf2, qaf2);
        tstar = updated.0;
        qstar = updated.1;
        zeta = updated.2;
        obu = updated.3;
        um = updated.4;
        if obuold * obu < 0.0 {
            nmozsgn += 1;
        }
        if nmozsgn >= 4 {
            obu = zldis / (-0.01);
        }
        obuold = obu;
        dtl_prev = Some(dtl);
        iteration += 1;
        if iteration > 6 {
            fevpl_bef = fevpl;
            let det = del.max(del2);
            let dee = dele.max(dele2);
            if det < 0.01 && dee < 0.1 {
                break;
            }
        }
        if iteration > 40 {
            break;
        }
    }

    let (rd3, rd2, rb0, rb1, rb2) = (t.rd3, t.rd2, t.rb0, t.rb12, t.rb12);
    let rib = ((zeta * (t.ustar * t.ustar)) / (((VONKAR * VONKAR) / t.fh) * (um * um))).min(5.0);
    let rst;
    if lai > 0.001 {
        rst = rs / lai;
    } else {
        assim = 0.0;
        respc = 0.0;
        rst = 2.0e4;
    }
    respc += RSOIL;
    let etr_deficit = (etr - etr_).max(0.0);

    // 用最后一次增量把叶片通量外推到新叶温（`:2286-2304`）
    fsenl = erre.mul_add(
        HVAP,
        denominator.mul_add(dtl_noadj - dtl, fsenl_dtl.mul_add(dtl, fsenl)),
    );
    etr = etr_dtl.mul_add(dtl, etr);
    evplwet = evplwet_dtl.mul_add(dtl, evplwet);
    let mut fevpl = fevpl_dtl.mul_add(dtl, fevpl_noadj);
    let elwmax = state.ldew / dt;
    let elwdif = (evplwet - elwmax).max(0.0);
    evplwet = evplwet.min(elwmax);
    fevpl -= elwdif;
    fsenl = elwdif.mul_add(HVAP, fsenl);

    // 冠层水（`:2311-2374`）
    state.ldew = (-dt).mul_add(evplwet, state.ldew).max(0.0);
    if tree.vegetation_snow {
        let (qevpl, qdewl, qsubl, qfrol) = if state.tl > TFRZ {
            let mut qevpl = evplwet.max(0.0);
            let qdewl = evplwet.min(0.0).abs();
            let mut qsubl = 0.0;
            if qevpl > state.ldew_rain / dt {
                qsubl = qevpl - state.ldew_rain / dt;
                qevpl = state.ldew_rain / dt;
            }
            (qevpl, qdewl, qsubl, 0.0)
        } else {
            let mut qsubl = evplwet.max(0.0);
            let qfrol = evplwet.min(0.0).abs();
            let mut qevpl = 0.0;
            if qsubl > state.ldew_snow / dt {
                qevpl = qsubl - state.ldew_snow / dt;
                qsubl = state.ldew_snow / dt;
            }
            (qevpl, 0.0, qsubl, qfrol)
        };
        state.ldew_rain = dt.mul_add(qdewl - qevpl, state.ldew_rain);
        state.ldew_snow = dt.mul_add(qfrol - qsubl, state.ldew_snow);
        state.ldew = state.ldew_rain + state.ldew_snow;
        state.fwet_snow = 0.0;
        if state.ldew_snow > 0.0 {
            // `:2347` 指数是字面量 `.666666666666`
            state.fwet_snow = ((10.0 / (lsai * 48.0)) * state.ldew_snow)
                .lpow(0.666666666666)
                .min(1.0);
        }
        if state.ldew_snow > 1.0e-6 && state.tl > TFRZ {
            let qmelt = (state.ldew_snow / dt)
                .min((state.ldew_snow * ((state.tl - TFRZ) * CPICE)) / (dt * HFUS));
            state.ldew_snow = (-dt).mul_add(qmelt, state.ldew_snow).max(0.0);
            state.ldew_rain = dt.mul_add(qmelt, state.ldew_rain).max(0.0);
            state.tl = state
                .fwet_snow
                .mul_add(TFRZ, state.tl * (1.0 - state.fwet_snow));
        }
        if state.ldew_rain > 1.0e-6 && state.tl < TFRZ {
            let qfrz = (state.ldew_rain / dt)
                .min((((TFRZ - state.tl) * CPLIQ) * state.ldew_rain) / (dt * HFUS));
            state.ldew_rain = (-dt).mul_add(qfrz, state.ldew_rain).max(0.0);
            state.ldew_snow = dt.mul_add(qfrz, state.ldew_snow).max(0.0);
            state.tl = state
                .fwet_snow
                .mul_add(TFRZ, state.tl * (1.0 - state.fwet_snow));
        }
    }
    let dheatl = (clai / dt) * dtl;

    // 长波（`:2396-2430`）
    let wall = |emissivity: f64, radiance: f64, emitted: f64, dradiance: f64| {
        let base = emissivity.mul_add(radiance, -emitted) / (1.0 - emissivity);
        ((emissivity * dradiance) / (1.0 - emissivity)).mul_add(dtl, base)
    };
    let mut lwsun = wall(tree.ewall, x[0], tree.b1[0], dx[0]);
    let mut lwsha = wall(tree.ewall, x[1], tree.b1[1], dx[1]);
    let mut lgimp = wall(tree.egimp, x[2], tree.b1[2], dx[2]);
    let mut lgper = wall(tree.egper, x[3], tree.b1[3], dx[3]);
    let mut received = 0.0;
    for i in 0..4 {
        received = x[i].mul_add(tree.veg_vf[i], received);
    }
    received = tree.frl.mul_add(tree.veg_vf[4], received);
    let lveg_base = tree.ev.mul_add(received, -b1_leaf);
    let mut received_dtl = 0.0;
    for i in 0..4 {
        received_dtl = dx[i].mul_add(tree.veg_vf[i], received_dtl);
    }
    let mut lveg = tree
        .ev
        .mul_add(received_dtl, -dbdt_leaf)
        .mul_add(dtl, lveg_base);
    let mut lout = 0.0;
    for i in 0..5 {
        lout = x[i].mul_add(tree.sky_vf[i], lout);
    }
    let mut lout_dtl = 0.0;
    for i in 0..5 {
        lout_dtl = (dx[i] * tree.sky_vf[i]).mul_add(dtl, lout_dtl);
    }
    lout += lout_dtl;
    for (value, cover) in [
        (&mut lwsun, fc[1]),
        (&mut lwsha, fc[2]),
        (&mut lgimp, fc[3]),
        (&mut lgper, fc[4]),
        (&mut lveg, fc[5]),
    ] {
        if cover > 0.0 {
            *value = (*value / cover) * fg;
        }
    }
    lwsun += tree.lwsun;
    lwsha += tree.lwsha;
    lgimp += tree.lgimp;
    lgper += tree.lgper;
    lveg += tree.lveg;

    // 屋顶、墙、地面通量与导数（`:2438-2543`）
    let taux = -((input.eastward_wind_m_s * input.rhoair) / t.ram);
    let tauy = -((input.northward_wind_m_s * input.rhoair) / t.ram);
    let fsenroof = (rhocp / rb0) * (input.troof - taf3);
    let fsenwsun = (rhocp / rb1) * (input.twsun - taf2);
    let fsenwsha = (rhocp / rb2) * (input.twsha - taf2);
    let fevproof = ((input.rhoair / rb0) * (qsatl0 - qaf3)) * fwet_roof;
    let one_minus_bt = 1.0 - bt / (rd3 * ct);
    let cq = ((1.0 / rd3 + (fg * s.fgper) / (rd2 + rss_)) + ((fwet_gimp * fg) * s.fgimp) / rd2)
        + fc3 / rv;
    let bq_inner = (1.0 / rd3 + 1.0 / t.raw) + (fc[0] * fwet_roof) / rb0;
    let bq = 1.0 / (rd3 * bq_inner);
    let one_minus_bq = 1.0 - bq / (rd3 * cq);
    let cwsuns = (rhocp / rb1) * (1.0 - fc[1] / (one_minus_bt * (rb1 * ct)));
    let cwshas = (rhocp / rb2) * (1.0 - fc[2] / (one_minus_bt * (rb2 * ct)));
    let bt_inner = (1.0 / t.rah + 1.0 / rd3) + fc[0] / rb0;
    let croofs = (rhocp / rb0)
        * ((1.0 - ((fc[0] * bt) * bt) / (one_minus_bt * (rb0 * ct))) - fc[0] / (rb0 * bt_inner));
    let wet_roof_cover = fc[0] * fwet_roof;
    let croofl = (((input.rhoair * fwet_roof) / rb0) * qsatldt0)
        * ((1.0 - ((wet_roof_cover * bq) * bq) / ((rb0 * cq) * one_minus_bq))
            - wet_roof_cover / (rb0 * bq_inner));
    let croof = croofl.mul_add(input.htvp_roof, croofs);
    let fsengimp = (rhocp / rd2) * (input.tgimp - taf2);
    let fsengper = (rhocp / rd2) * (input.tgper - taf2);
    let fevpgper = (input.rhoair / (rd2 + rss_)) * (input.qgper - qaf2);
    let fevpgimp = ((input.rhoair / rd2) * (input.qgimp - qaf2)) * fwet_gimp;
    let cgrnds = (rhocp / rd2) * (1.0 - fg / (one_minus_bt * (rd2 * ct)));
    let cgperl = ((input.rhoair / (rd2 + rss_)) * input.dqgperdt)
        * (1.0 - (fg * s.fgper) / (one_minus_bq * ((rd2 + rss_) * cq)));
    let cgimpl = (((input.rhoair / rd2) * input.dqgimpdt)
        * (1.0 - ((fwet_gimp * fg) * s.fgimp) / (one_minus_bq * (rd2 * cq))))
        * fwet_gimp;
    let cgimp = cgimpl.mul_add(input.htvp_gimp, cgrnds);
    let cgper = cgperl.mul_add(input.htvp_gper, cgrnds);
    let reference = z0mu + displau;
    let tref = (reference - 2.0).mul_add(s.tg, taf2 * 2.0) / reference;
    let qref = (reference - 2.0).mul_add(qg, qaf2 * 2.0) / reference;

    Ok((
        UrbanFluxOutput {
            taux,
            tauy,
            fsenroof,
            fsenwsun,
            fsenwsha,
            fsengimp,
            fsengper,
            fevproof,
            fevpgimp,
            fevpgper,
            croofs,
            cwsuns,
            cwshas,
            cgrnds,
            croofl,
            cgimpl,
            cgperl,
            croof,
            cgimp,
            cgper,
            tref,
            qref,
            z0m,
            zol: zeta,
            rib,
            ustar: t.ustar,
            qstar,
            tstar,
            fm: t.fm,
            fh: t.fh,
            fq: t.fq,
            tafu: taf2,
        },
        UrbanTreeOutput {
            fsenl,
            fevpl,
            etr,
            rst,
            assim,
            respc,
            lwsun,
            lwsha,
            lgimp,
            lgper,
            lveg,
            lout,
            etr_deficit,
            dheatl,
        },
    ))
}

/// `matmul(Ainv, v)`：逐元素 `.FMA (A(i,j), v(j), acc)`，`j` 从 1 到 5。
fn matmul(a: &[[f64; 5]; 5], v: &[f64; 5]) -> [f64; 5] {
    let mut out = [0.0; 5];
    for (i, row) in a.iter().enumerate() {
        let mut acc = 0.0;
        for j in 0..5 {
            acc = row[j].mul_add(v[j], acc);
        }
        out[i] = acc;
    }
    out
}

fn validate(input: &UrbanFluxInput) -> Result<()> {
    ensure!(
        input.hroof > 0.0 && input.fcover[0] < 1.0 && input.time_step_seconds > 0.0,
        "urban flux geometry is invalid"
    );
    Ok(())
}

#[cfg(test)]
#[path = "urban_flux_tests.rs"]
mod urban_flux_tests;

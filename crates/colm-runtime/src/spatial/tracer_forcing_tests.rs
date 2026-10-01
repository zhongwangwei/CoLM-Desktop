use super::*;
use colm_core::tracer::descriptor::TracerNamelist;

fn isotope_set(param_file: &str) -> TracerSet {
    let namelist = TracerNamelist {
        num: 2,
        names: "H2_18O,HDO".to_owned(),
        types: "isotope,isotope".to_owned(),
        ref_ratio: "2.0052e-3,1.5576e-4".to_owned(),
        param_files: format!("H2_18O:{param_file}"),
        ..TracerNamelist::default()
    };
    TracerSet::build(&namelist, |_| Ok(None)).unwrap()
}

/// 每个测试自己的参数文件（测试并行跑，名字带标签区分）。
fn write_param(label: &str, body: &str) -> String {
    let dir = std::env::temp_dir().join(format!("colm-trcforc-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("o18.nml");
    std::fs::write(&path, body).unwrap();
    path.to_string_lossy().into_owned()
}

const STANDARD_O18: &str = "&nl_colm_tracer_parameter\n   DEF_TRACER%mol_weight = 20.0\n/\n\
&nl_colm_tracer_forcing\n    forcing_num        = 2\n    forcing_role       = 'precip', 'vapor'\n\
    forcing_fprefix    = 'IsoGSM_prate', 'IsoGSM_Q'\n    forcing_vname      = 'prate1sfc', 'spfh12m'\n\
    forcing_tintalgo   = 'nearest', 'linear'\n    forcing_dtime      = 21600, 21600\n\
    forcing_offset     = 10800, 10800\n\
    forcing_input_mode = 'normalized_over_total', 'normalized_over_total'\n/\n";

fn totals() -> MainTotals {
    let entry = |prefix: &str, name: &str, tint: &str, log: &str| {
        (
            prefix.to_owned(),
            name.to_owned(),
            tint.to_owned(),
            log.to_owned(),
            21_600,
            10_800,
        )
    };
    MainTotals {
        vapor: entry("IsoGSM_Q", "spfh2m", "linear", "instant"),
        precip: entry("IsoGSM_prate", "pratesfc", "nearest", "forward"),
    }
}

#[test]
fn the_standard_o18_group_loads_like_upstream() {
    let path = write_param("standard", STANDARD_O18);
    let set = isotope_set(&path);
    let specs = load_specs(&set, &format!("H2_18O:{path}")).unwrap();
    assert_eq!(specs.len(), 2);
    assert!(specs[1].is_empty(), "HDO has no parameter file");
    assert_eq!(specs[0].len(), 2);
    assert_eq!(specs[0][0].role, "precip");
    assert_eq!(specs[0][1].vname, "spfh12m");
    assert_eq!(specs[0][0].tintalgo, "nearest");
    assert_eq!(specs[0][1].offset, 10_800);
}

#[test]
fn indexed_assignments_override_single_elements() {
    let path = write_param("indexed", "&nl_colm_tracer_forcing\n forcing_num = 1\n forcing_role(1) = 'VAPOR'\n \
         forcing_fprefix(1) = 'q'\n forcing_vname(1) = 'v'\n forcing_input_mode(1) = 'Delta'\n/\n",
    );
    let set = isotope_set(&path);
    let specs = load_specs(&set, &format!("H2_18O:{path}")).unwrap();
    assert_eq!(specs[0][0].role, "vapor");
    assert_eq!(specs[0][0].input_mode, "delta");
    assert_eq!(
        specs[0][0].dtime, 21_600,
        "unset entries keep the namelist default"
    );
}

#[test]
fn duplicate_roles_are_rejected() {
    let path = write_param("duplicate", "&nl_colm_tracer_forcing\n forcing_num = 2\n forcing_role = 'vapor', 'vapor'\n/\n",
    );
    let set = isotope_set(&path);
    assert!(load_specs(&set, &format!("H2_18O:{path}")).is_err());
}

#[test]
fn over_total_inputs_register_their_total_first() {
    let path = write_param("total", STANDARD_O18);
    let set = isotope_set(&path);
    let specs = load_specs(&set, &format!("H2_18O:{path}")).unwrap();
    let physics = TracerPhysics::default();
    let config = configure(&set, &physics, &specs, Some(&totals())).unwrap();
    let streams: Vec<_> = config.vars.iter().map(|v| v.stream).collect();
    assert_eq!(
        streams,
        [
            Stream::TotalPrecip,
            Stream::Precip,
            Stream::TotalVapor,
            Stream::Vapor
        ]
    );
    assert_eq!(config.vars[1].total, Some(0));
    assert_eq!(
        config.vars[1].timelog, "forward",
        "inherits the total's timelog"
    );
    assert_eq!(config.runtime_forced, [true, false]);
    assert_eq!(config.vapor_configured, [true, false]);
}

#[test]
fn mismatched_total_timing_is_rejected() {
    let body = STANDARD_O18.replace("'nearest', 'linear'", "'linear', 'linear'");
    let path = write_param("mismatch", &body);
    let set = isotope_set(&path);
    let specs = load_specs(&set, &format!("H2_18O:{path}")).unwrap();
    let physics = TracerPhysics::default();
    assert!(configure(&set, &physics, &specs, Some(&totals())).is_err());
}

#[test]
fn decoding_keeps_the_last_valid_ratio() {
    let path = write_param("decode", STANDARD_O18);
    let set = isotope_set(&path);
    let specs = load_specs(&set, &format!("H2_18O:{path}")).unwrap();
    let physics = TracerPhysics::default();
    let config = configure(&set, &physics, &specs, Some(&totals())).unwrap();
    let main = isogsm_main();
    let mut forcing =
        GriddedTracerForcing::new(config, &set, 3, "arealweight".into(), &main).unwrap();
    let default = forcing.precip[0];
    // patch 0 有效、patch 1 近干、patch 2 缺测（填充值）。
    let values = vec![
        vec![1.0e-4, 1.0e-8, 9.999e20],
        vec![0.99e-4, 0.99e-8, 1.0e-4],
        vec![1.0e-2, 1.0e-2, 1.0e-2],
        vec![0.98e-2, 0.98e-2, 0.98e-2],
    ];
    forcing.update_values(&values);
    let r = 2.0052e-3;
    assert_eq!(forcing.precip[0], 0.99e-4 / 1.0e-4 * r);
    assert_eq!(
        forcing.precip[2], default,
        "near-dry keeps the previous ratio"
    );
    assert_eq!(
        forcing.precip[4], default,
        "fill value keeps the previous ratio"
    );
    assert_eq!(forcing.vapor[0], 0.98e-2 / 1.0e-2 * r);
    assert_eq!(
        forcing.vapor[1],
        set.tracers[1].vapor_default_ratio(),
        "HDO is unforced"
    );
}

fn isogsm_main() -> GriddedForcingConfig {
    let text = "&nl_colm_forcing\n DEF_dir_forcing = '/data/IsoGSM/'\n DEF_forcing%dataset = 'IsoGSM'\n\
 DEF_forcing%HEIGHT_V = 50.0\n DEF_forcing%HEIGHT_T = 40.\n DEF_forcing%HEIGHT_Q = 40.\n\
 DEF_forcing%NVAR = 8\n DEF_forcing%startyr = 1980\n DEF_forcing%startmo = 1\n\
 DEF_forcing%dtime = 21600 21600 21600 21600 21600 21600 21600 21600\n\
 DEF_forcing%offset = 10800 10800 10800 10800 10800 10800 0 10800\n\
 DEF_forcing%leapyear = .true.\n DEF_forcing%latname = 'lat'\n DEF_forcing%lonname = 'lon'\n\
 DEF_forcing%groupby = 'year'\n DEF_forcing%fprefix(1) = 'IsoGSM_temperature'\n\
 DEF_forcing%fprefix(2) = 'IsoGSM_Q'\n DEF_forcing%fprefix(3) = 'IsoGSM_Pressure'\n\
 DEF_forcing%fprefix(4) = 'IsoGSM_prate'\n DEF_forcing%fprefix(5) = 'IsoGSM_Wind'\n\
 DEF_forcing%fprefix(6) = 'IsoGSM_Wind'\n DEF_forcing%fprefix(7) = 'IsoGSM_Radiation'\n\
 DEF_forcing%fprefix(8) = 'IsoGSM_Radiation'\n\
 DEF_forcing%vname = 'tmp2m' 'spfh2m' 'pressfc' 'pratesfc' 'ugrd10m' 'vgrd10m' 'dswrfsfc' 'dlwrfsfc'\n\
 DEF_forcing%timelog = 'instant' 'instant' 'instant' 'forward' 'instant' 'instant' 'forward' 'forward'\n\
 DEF_forcing%tintalgo = 'linear' 'linear' 'linear' 'nearest' 'linear' 'linear' 'coszen' 'linear'\n/\n";
    GriddedForcingConfig::from_document(&colm_namelist::parse(text).unwrap()).unwrap()
}

#[test]
fn main_totals_and_identity_follow_the_main_forcing() {
    let path = write_param("identity", STANDARD_O18);
    let set = isotope_set(&path);
    let specs = load_specs(&set, &format!("H2_18O:{path}")).unwrap();
    let main = isogsm_main();
    let totals = MainTotals::from_config(&main);
    assert_eq!(totals.precip.1, "pratesfc");
    assert_eq!(totals.precip.3, "forward");
    let config = configure(&set, &TracerPhysics::default(), &specs, Some(&totals)).unwrap();
    let forcing = GriddedTracerForcing::new(config, &set, 2, "arealweight".into(), &main).unwrap();
    let id = forcing.identity();
    assert_eq!(id.len(), ID_WIDTH * 5);
    assert_eq!(&id[..8], &[4, 2, 1980, 1, 1, 1, 3, 0]);
    // 第 1 槽是数据集名，逐字符 `iachar`。
    assert_eq!(&id[8..14], &[73, 115, 111, 71, 83, 77]);
    // 第 2 列（1 号变量，总降水）：stream 3、itrc 0、mode 1、total 0、dtime、offset。
    assert_eq!(&id[ID_WIDTH..ID_WIDTH + 8], &[3, 0, 1, 0, 21_600, 10_800, 0, 0]);
    assert_eq!(&id[2 * ID_WIDTH..2 * ID_WIDTH + 8], &[1, 1, 4, 1, 21_600, 10_800, 0, 0]);
    let cache = forcing.cache();
    assert_eq!(cache.block(1..2).precip.len(), 2);
}

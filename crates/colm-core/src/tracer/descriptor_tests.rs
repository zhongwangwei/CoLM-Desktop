//! 描述符构建、参数文件解析与指纹格式；期望值取自 Fortran 写出的重启。

use super::*;

fn solute_namelist() -> TracerNamelist {
    TracerNamelist {
        num: 1,
        names: "sol1".to_owned(),
        types: "solute".to_owned(),
        param_files: "/tmp/sol1.nml".to_owned(),
        variably_saturated_flow: true,
        aquifer_mixing_water_mm: -1.0,
        ..TracerNamelist::default()
    }
}

#[test]
fn solute_identity_matches_the_fortran_restart() {
    let set = TracerSet::build(&solute_namelist(), |path| {
        assert_eq!(path, "/tmp/sol1.nml");
        Ok(Some(TracerParameterOverrides {
            init_conc: Some(1.0),
            precip_default_conc: Some(2.0),
            vapor_default_conc: Some(0.5),
            ..TracerParameterOverrides::default()
        }))
    })
    .unwrap();
    // tsa 算例（单点 AT，1 个 solute）`trc_land_descriptor_identity` 第一行。
    let expected = "sol1|solute|tracer_per_water|2|1|0|0| 1.8000000000000000E+001| \
                    1.0000000000000000E+000| 0.0000000000000000E+000| 1.0000000000000000E+000| \
                    2.0000000000000000E+000| 5.0000000000000000E-001| 1.7976931348623157E+308| \
                    0.0000000000000000E+000|";
    let identity = set.descriptor_identity();
    assert_eq!(identity.len(), 1);
    let text: String = identity[0].iter().map(|&c| char::from(c as u8)).collect();
    assert_eq!(text.trim_end(), expected);
    assert_eq!(text.len(), DESCRIPTOR_IDENTITY_WIDTH);
}

#[test]
fn zero_tracers_build_an_empty_set() {
    let set = TracerSet::build(
        &TracerNamelist {
            num: 0,
            ..TracerNamelist::default()
        },
        |_| unreachable!("no parameter file is read without tracers"),
    )
    .unwrap();
    assert!(set.is_empty());
}

#[test]
fn types_must_cover_every_tracer() {
    let namelist = TracerNamelist {
        num: 2,
        types: "solute".to_owned(),
        ..TracerNamelist::default()
    };
    assert!(TracerSet::build(&namelist, |_| Ok(None)).is_err());
}

#[test]
fn names_are_sanitised_defaulted_and_made_unique() {
    let namelist = TracerNamelist {
        num: 4,
        names: "H2O-16, h2o16,A".to_owned(),
        types: "solute,solute,conservative,solute".to_owned(),
        ..TracerNamelist::default()
    };
    let set = TracerSet::build(&namelist, |_| Ok(None)).unwrap();
    let names: Vec<_> = set.tracers.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["H2O16", "h2o16_2", "A", "tracer_4"]);
    assert_eq!(set.tracers[2].category, "solute");
}

#[test]
fn keyed_and_positional_parameter_files_resolve_like_upstream() {
    let tracers = TracerSet::build(
        &TracerNamelist {
            num: 2,
            names: "a,b".to_owned(),
            types: "solute,solute".to_owned(),
            ..TracerNamelist::default()
        },
        |_| Ok(None),
    )
    .unwrap()
    .tracers;
    assert_eq!(
        param_file_for_index("B:/x.nml; /y.nml", &tracers, 1).unwrap(),
        Some("/x.nml".to_owned())
    );
    assert_eq!(
        param_file_for_index("B:/x.nml; /y.nml", &tracers, 0).unwrap(),
        Some("/y.nml".to_owned())
    );
    assert_eq!(param_file_for_index("null", &tracers, 0).unwrap(), None);
    // Windows 原生路径的盘符不是映射键（`C` 不是示踪物名也照样按位置读）。
    assert_eq!(
        param_file_for_index(r"C:\cases\a.nml, C:\cases\b.nml", &tracers, 1).unwrap(),
        Some(r"C:\cases\b.nml".to_owned())
    );
}

#[test]
fn vsf_isotopes_need_a_positive_mixing_volume() {
    let namelist = TracerNamelist {
        num: 1,
        types: "isotope".to_owned(),
        variably_saturated_flow: true,
        aquifer_mixing_water_mm: -1.0,
        ..TracerNamelist::default()
    };
    assert!(TracerSet::build(&namelist, |_| Ok(None)).is_err());
}

#[test]
fn es24_16e3_pads_and_rounds_like_gfortran() {
    assert_eq!(format_es24_16e3(18.0), " 1.8000000000000000E+001");
    assert_eq!(format_es24_16e3(-0.5), "-5.0000000000000000E-001");
    assert_eq!(format_es24_16e3(f64::MAX), " 1.7976931348623157E+308");
    assert_eq!(format_es24_16e3(0.0), " 0.0000000000000000E+000");
}

#[test]
fn dissolved_limit_moves_excess_into_the_solid_pool() {
    let mut set = TracerSet::build(
        &TracerNamelist {
            num: 1,
            types: "solute".to_owned(),
            ..TracerNamelist::default()
        },
        |_| Ok(None),
    )
    .unwrap();
    set.tracers[0].max_dissolved_conc = 2.0;
    let (mut dissolved, mut solid) = (5.0, 1.0);
    set.tracers[0].equilibrate_dissolved(1.5, &mut dissolved, &mut solid);
    assert_eq!((dissolved, solid), (3.0, 3.0));
}

//! 描述符构建、参数文件解析与指纹格式；期望值取自 Fortran 写出的重启。

use super::*;

fn solute_namelist() -> MaterialNamelist {
    MaterialNamelist {
        solutes: "sol1".to_owned(),
        param_files: MaterialParamFiles {
            solutes: "/tmp/sol1.nml".to_owned(),
            ..MaterialParamFiles::default()
        },
        variably_saturated_flow: true,
        aquifer_mixing_water_mm: -1.0,
        ..MaterialNamelist::default()
    }
}

#[test]
fn solute_identity_matches_the_fortran_restart() {
    let set = MaterialSet::build(&solute_namelist(), |path| {
        assert_eq!(path, "/tmp/sol1.nml");
        Ok(Some(MaterialParameterOverrides {
            init_conc: Some(1.0),
            precip_default_conc: Some(2.0),
            vapor_default_conc: Some(0.5),
            ..MaterialParameterOverrides::default()
        }))
    })
    .unwrap();
    // 溶质的代码默认值：mol_weight=0、ref_ratio=1、init_delta=0（上游 `material_defs_init`）。
    // 最后一个是 `n2o_ef5`（上游 6a5e8985 把它加进指纹，默认 0）。
    let expected = "sol1|solute|material_per_water|2|1|0|0| 0.0000000000000000E+000| \
                    1.0000000000000000E+000| 0.0000000000000000E+000| 1.0000000000000000E+000| \
                    2.0000000000000000E+000| 5.0000000000000000E-001| 1.7976931348623157E+308| \
                    0.0000000000000000E+000| 0.0000000000000000E+000|";
    let identity = set.descriptor_identity();
    assert_eq!(identity.len(), 1);
    let text: String = identity[0].iter().map(|&c| char::from(c as u8)).collect();
    assert_eq!(text.trim_end(), expected);
    assert_eq!(text.len(), DESCRIPTOR_IDENTITY_WIDTH);
}

#[test]
fn no_family_builds_an_empty_set() {
    let set = MaterialSet::build(&MaterialNamelist::default(), |_| {
        unreachable!("no parameter file is read without materials")
    })
    .unwrap();
    assert!(set.is_empty());
}

#[test]
fn unknown_isotope_species_is_refused() {
    let namelist = MaterialNamelist {
        use_isotope: true,
        isotope_species: "H2_18O,O18".to_owned(),
        ..MaterialNamelist::default()
    };
    let error = MaterialSet::build(&namelist, |_| Ok(None)).unwrap_err();
    assert!(error.to_string().contains("H2_18O, HDO"), "{error}");
}

#[test]
fn an_enabled_isotope_family_needs_a_species() {
    let namelist = MaterialNamelist {
        use_isotope: true,
        isotope_species: " , ".to_owned(),
        ..MaterialNamelist::default()
    };
    assert!(MaterialSet::build(&namelist, |_| Ok(None)).is_err());
}

#[test]
fn families_are_built_in_upstream_order() {
    let namelist = MaterialNamelist {
        use_isotope: true,
        isotope_species: "HDO,H2_18O".to_owned(),
        solutes: "CL, NO3".to_owned(),
        use_sediment: true,
        use_methane: true,
        use_bgc: true,
        ..MaterialNamelist::default()
    };
    let set = MaterialSet::build(&namelist, |_| Ok(None)).unwrap();
    let names: Vec<_> = set.materials.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["HDO", "H2_18O", "CL", "NO3", "SEDIMENT", "CH4"]);
    assert_eq!(set.materials[0].family, MaterialFamily::Isotope);
    assert_eq!(set.materials[2].family, MaterialFamily::Solute);
    assert_eq!(set.materials[4].family, MaterialFamily::Particle);
    assert_eq!(set.materials[5].family, MaterialFamily::Gas);
    assert_eq!(set.materials[0].ref_ratio, 1.5576e-4);
    assert_eq!(set.materials[1].mol_weight, 20.0);
}

#[test]
fn solute_names_are_sanitised_and_duplicates_are_refused() {
    let set = MaterialSet::build(
        &MaterialNamelist {
            solutes: "H2O-16, A".to_owned(),
            ..MaterialNamelist::default()
        },
        |_| Ok(None),
    )
    .unwrap();
    let names: Vec<_> = set.materials.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["H2O16", "A"]);
    // 名字会成为 NetCDF 变量名，重名上游直接停机（不再自动加后缀）。
    let duplicate = MaterialNamelist {
        solutes: "cl, CL".to_owned(),
        ..MaterialNamelist::default()
    };
    assert!(MaterialSet::build(&duplicate, |_| Ok(None)).is_err());
}

#[test]
fn solute_parameter_files_index_only_the_solutes() {
    let materials = MaterialSet::build(
        &MaterialNamelist {
            use_isotope: true,
            isotope_species: "H2_18O".to_owned(),
            solutes: "a,b".to_owned(),
            ..MaterialNamelist::default()
        },
        |_| Ok(None),
    )
    .unwrap()
    .materials;
    let files = MaterialParamFiles {
        solutes: "B:/x.nml; /y.nml".to_owned(),
        ..MaterialParamFiles::default()
    };
    // 全局序号 1 是第一个溶质，所以它拿第 2 个位置项；`B:` 是 Windows 盘符不是映射键。
    assert_eq!(
        param_file_for_index(&files, &materials, 1).unwrap(),
        Some("/y.nml".to_owned())
    );
    assert_eq!(
        param_file_for_index(&files, &materials, 2).unwrap(),
        Some("/x.nml".to_owned())
    );
    let none = MaterialParamFiles {
        solutes: "null".to_owned(),
        ..MaterialParamFiles::default()
    };
    assert_eq!(param_file_for_index(&none, &materials, 1).unwrap(), None);
}

#[test]
fn each_family_reads_its_own_parameter_file_field() {
    let materials = MaterialSet::build(
        &MaterialNamelist {
            use_isotope: true,
            isotope_species: "H2_18O".to_owned(),
            solutes: "CL".to_owned(),
            use_sediment: true,
            use_methane: true,
            use_bgc: true,
            ..MaterialNamelist::default()
        },
        |_| Ok(None),
    )
    .unwrap()
    .materials;
    let files = MaterialParamFiles {
        isotope: "iso.nml".to_owned(),
        solutes: "CL:cl.nml".to_owned(),
        sediment: "sed.nml".to_owned(),
        methane: "ch4.nml".to_owned(),
    };
    let paths: Vec<_> = (0..materials.len())
        .map(|i| param_file_for_index(&files, &materials, i).unwrap())
        .collect();
    assert_eq!(
        paths,
        [
            Some("iso.nml".to_owned()),
            Some("cl.nml".to_owned()),
            Some("sed.nml".to_owned()),
            Some("ch4.nml".to_owned())
        ]
    );
}

#[test]
fn vsf_isotopes_need_a_positive_mixing_volume() {
    let namelist = MaterialNamelist {
        use_isotope: true,
        isotope_species: "H2_18O".to_owned(),
        variably_saturated_flow: true,
        aquifer_mixing_water_mm: -1.0,
        ..MaterialNamelist::default()
    };
    assert!(MaterialSet::build(&namelist, |_| Ok(None)).is_err());
}

#[test]
fn negative_decay_is_refused_instead_of_clamped() {
    let namelist = MaterialNamelist {
        solutes: "CL".to_owned(),
        param_files: MaterialParamFiles {
            solutes: "cl.nml".to_owned(),
            ..MaterialParamFiles::default()
        },
        ..MaterialNamelist::default()
    };
    let error = MaterialSet::build(&namelist, |_| {
        Ok(Some(MaterialParameterOverrides {
            decay_rate: Some(-1.0),
            ..MaterialParameterOverrides::default()
        }))
    })
    .unwrap_err();
    assert!(error.to_string().contains("negative decay_rate"), "{error}");
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
    let mut set = MaterialSet::build(
        &MaterialNamelist {
            solutes: "CL".to_owned(),
            ..MaterialNamelist::default()
        },
        |_| Ok(None),
    )
    .unwrap();
    set.materials[0].max_dissolved_conc = 2.0;
    let (mut dissolved, mut solid) = (5.0, 1.0);
    set.materials[0].equilibrate_dissolved(1.5, &mut dissolved, &mut solid);
    assert_eq!((dissolved, solid), (3.0, 3.0));
}

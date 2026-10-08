use super::*;

fn maps() -> Vec<Mapping> {
    vec![
        parse_mapping(
            "/Volumes/Data/Data/PLUMBER2s=/media/zhwei/data02/zhwei/training2026/PLUMBER2s",
        )
        .unwrap(),
        parse_mapping("/Volumes/Data/Data=/media/zhwei/data02/zhwei").unwrap(),
    ]
}

#[test]
fn paths_go_into_the_case_copy_or_through_the_longest_mapping() {
    let case = Path::new("/Users/me/Desktop/CA-Qfo-pc");
    let remote = "/srv/colm/cases/CA-Qfo-pc-1234abcd";
    // 算例目录内：换成副本目录，目录值保留结尾的 `/`。
    assert_eq!(
        place("/Users/me/Desktop/CA-Qfo-pc/site.nc", case, remote, &maps()),
        Placed::Remote(format!("{remote}/site.nc"))
    );
    assert_eq!(
        place(
            "/Users/me/Desktop/CA-Qfo-pc/rawdata_unused/",
            case,
            remote,
            &maps()
        ),
        Placed::Remote(format!("{remote}/rawdata_unused/"))
    );
    assert_eq!(
        place("site.nc", case, remote, &maps()),
        Placed::Remote(format!("{remote}/site.nc"))
    );
    // 更具体的对应优先。
    assert_eq!(
        place(
            "/Volumes/Data/Data/PLUMBER2s/Forcing/",
            case,
            remote,
            &maps()
        ),
        Placed::Remote("/media/zhwei/data02/zhwei/training2026/PLUMBER2s/Forcing/".into())
    );
    assert_eq!(
        place("/Volumes/Data/Data/CoLMrawdata/", case, remote, &maps()),
        Placed::Remote("/media/zhwei/data02/zhwei/CoLMrawdata/".into())
    );
    // 按路径分量比：/Volumes/Data/DataX 不在 /Volumes/Data/Data 之下。
    assert_eq!(
        place("/Volumes/Data/DataX/f.nc", case, remote, &maps()),
        Placed::Keep
    );
    // 对应不上、本机存在：报出来。
    let here = std::env::temp_dir();
    assert!(matches!(
        place(&here.to_string_lossy(), case, remote, &maps()),
        Placed::Unmapped(_)
    ));
    for bad in ["relative=/x", "/a", "/a=relative"] {
        assert!(parse_mapping(bad).is_err(), "{bad}");
    }
}

#[test]
fn staging_copies_inputs_only_and_rewrites_every_path() {
    let root = std::env::temp_dir().join(format!("colm-remote-stage-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let case = root.join("CA-Qfo-pc");
    std::fs::create_dir_all(case.join("out/CA-Qfo-pc/history")).unwrap();
    std::fs::write(case.join("out/CA-Qfo-pc/history/h.nc"), "big").unwrap();
    std::fs::write(case.join("colm.log"), "log").unwrap();
    std::fs::write(case.join("stages.json"), "{}").unwrap();
    std::fs::write(case.join("site.nc"), "site").unwrap();
    let c = case.display();
    std::fs::write(
        case.join("case.nml"),
        format!(
            "&nl_colm\n DEF_CASE_NAME = 'CA-Qfo-pc'\n SITE_fsitedata = '{c}/site.nc'\n DEF_dir_rawdata = '{c}/rawdata_unused/'\n DEF_dir_output = '{c}/out/'\n DEF_forcing_namelist = '{c}/forcing.nml'\n DEF_simulation_time%start_year = 2004\n/\n"
        ),
    )
    .unwrap();
    std::fs::write(
        case.join("forcing.nml"),
        "&nl_colm_forcing\n DEF_dir_forcing = '/Volumes/Data/Data/PLUMBER2s/Forcing/'\n DEF_forcing%fprefix(1) = 'CA-Qfo_2004-2010_FLUXNET2015_Met.nc'\n/\n",
    )
    .unwrap();
    let staging = root.join("staging");
    let remote = "/srv/colm/cases/CA-Qfo-pc-1234abcd";
    let staged = stage_case(&case, &staging, remote, &maps(), false).unwrap();
    // 输出、日志、阶段指纹不随算例上传。
    assert!(
        !staging.join("out").exists()
            && !staging.join("colm.log").exists()
            && !staging.join("stages.json").exists()
    );
    assert!(staging.join("site.nc").is_file());
    let case_nml = std::fs::read_to_string(staging.join("case.nml")).unwrap();
    assert!(
        case_nml.contains(&format!("SITE_fsitedata = '{remote}/site.nc'")),
        "{case_nml}"
    );
    assert!(
        case_nml.contains(&format!("DEF_dir_output = '{remote}/out/'")),
        "{case_nml}"
    );
    assert!(case_nml.contains(&format!("DEF_forcing_namelist = '{remote}/forcing.nml'")));
    assert!(case_nml.contains(&format!("DEF_dir_rawdata = '{remote}/rawdata_unused/'")));
    assert!(case_nml.contains("start_year = 2004"));
    let forcing = std::fs::read_to_string(staging.join("forcing.nml")).unwrap();
    assert!(
        forcing.contains(
            "DEF_dir_forcing = '/media/zhwei/data02/zhwei/training2026/PLUMBER2s/Forcing/'"
        ),
        "{forcing}"
    );
    assert!(
        forcing.contains("fprefix(1) = 'CA-Qfo_2004-2010_FLUXNET2015_Met.nc'"),
        "file-name prefixes stay as they are"
    );
    // 指向服务器数据的路径交给提交前核对。
    assert_eq!(
        staged.remote_inputs,
        ["/media/zhwei/data02/zhwei/training2026/PLUMBER2s/Forcing/"]
    );
    assert!(staged.unmapped.is_empty());

    // 对应不上的本机文件：默认报出来；允许上传时随算例一起传。
    let outside = root.join("extra.nc");
    std::fs::write(&outside, "x").unwrap();
    let nml = std::fs::read_to_string(case.join("case.nml"))
        .unwrap()
        .replace(
            " DEF_simulation_time",
            &format!(
                " DEF_file_SoilInit = '{}'\n DEF_simulation_time",
                outside.display()
            ),
        );
    std::fs::write(case.join("case.nml"), nml).unwrap();
    let _ = std::fs::remove_dir_all(&staging);
    assert_eq!(
        stage_case(&case, &staging, remote, &maps(), false)
            .unwrap()
            .unmapped,
        std::slice::from_ref(&outside)
    );
    let _ = std::fs::remove_dir_all(&staging);
    let uploaded = stage_case(&case, &staging, remote, &maps(), true).unwrap();
    assert_eq!(uploaded.uploaded_extra, [outside]);
    assert!(staging.join("inputs/extra.nc").is_file());
    assert!(std::fs::read_to_string(staging.join("case.nml"))
        .unwrap()
        .contains(&format!("'{remote}/inputs/extra.nc'")));
    let _ = std::fs::remove_dir_all(&root);
}

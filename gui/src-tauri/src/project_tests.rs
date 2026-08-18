use super::*;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("colm-gui-project-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("mkdir");
    d
}

fn make_case(root: &Path, dir: &str, name: &str) -> PathBuf {
    let d = root.join(dir);
    std::fs::create_dir_all(&d).expect("mkdir");
    std::fs::write(
        d.join("case.nml"),
        format!("&nl_colm\n   DEF_CASE_NAME = '{name}'\n/\n"),
    )
    .expect("write");
    d
}

#[test]
fn a_directory_with_a_case_nml_is_a_case() {
    let root = tmp("basic");
    make_case(&root, "one", "CN-Cng");
    make_case(&root, "two", "AT-Neu");
    // 没有 case.nml 的目录不算
    std::fs::create_dir_all(root.join("not-a-case")).expect("mkdir");

    let cases = list_cases(root.to_string_lossy().into_owned()).expect("lists");
    assert_eq!(cases.len(), 2);
    // 按算例名排序，不是按目录名 —— 界面上用户看到的是算例名
    assert_eq!(cases[0].name, "AT-Neu");
    assert_eq!(cases[1].name, "CN-Cng");
}

#[test]
fn the_name_comes_from_the_namelist_not_the_directory() {
    // 目录叫 whatever，算例叫 CN-Cng。产物路径由后者决定，
    // 所以界面必须显示后者，否则用户在磁盘上找不到自己的东西。
    let root = tmp("naming");
    make_case(&root, "whatever", "CN-Cng");
    let cases = list_cases(root.to_string_lossy().into_owned()).expect("lists");
    assert_eq!(cases[0].name, "CN-Cng");
    assert!(cases[0].dir.ends_with("whatever"));
}

#[test]
fn a_case_without_a_history_file_is_marked_as_not_run() {
    let root = tmp("unrun");
    let d = make_case(&root, "one", "CN-Cng");
    let cases = list_cases(root.to_string_lossy().into_owned()).expect("lists");
    assert!(!cases[0].has_history);

    // 放一个 history 进去就算跑过
    let h = d.join("out/CN-Cng/history");
    std::fs::create_dir_all(&h).expect("mkdir");
    std::fs::write(h.join("CN-Cng_hist_2008-01.nc"), b"not really netcdf").expect("write");
    let cases = list_cases(root.to_string_lossy().into_owned()).expect("lists");
    assert!(cases[0].has_history);
}

#[test]
fn a_missing_directory_says_so_rather_than_returning_nothing() {
    // 返回空列表会被界面渲染成「这里没有算例」，而真相是路径写错了。
    let e = list_cases("/no/such/place/at/all".into()).unwrap_err();
    assert!(e.contains("/no/such/place"), "{e}");
}

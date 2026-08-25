//! 判官的行为测试。
//!
//! 判官已经两次静默放行过真实回归：第一版只比全局属性、不比变量级属性；
//! 第二版不比逐变量维度顺序，于是把 119 个变量的 (time,patch,…) 换成
//! (patch,time,…) 后仍报 identical。两次都是靠手工命令才发现的。
//!
//! 所以这里对**它声称比较的每一类东西**各写一条负向测试。一个只会说
//! 「相同」的判官比没有判官更糟：它让回归带着绿色的 CI 通过。
//!
//! 测试自造小文件而不用黄金文件：跑得快，且不会因为黄金文件被重新生成
//! 而失效 —— 我们要测的是判官的逻辑，不是某份数据。

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use oracle::judge::compare;

/// 每个测试用独立目录，避免并行时互相踩。
fn workdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "oracle-judge-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("create workdir");
    d
}

/// 造一个结构上像 CoLM history 的小文件。
///
/// `patch` 长度刻意为 1，与真实输出一致 —— 正因为它是 1，维度置换才不会
/// 改变扁平化后的元素顺序，这也正是维度比较必须存在的原因。
fn write_file(path: &Path, opts: Opts) {
    let _guard = netcdf_write_lock().lock().unwrap();
    let mut f = netcdf::create(path).expect("create");
    f.add_dimension("time", 3).unwrap();
    f.add_dimension("patch", 1).unwrap();
    f.add_attribute("create_time", opts.create_time).unwrap();
    f.add_attribute("title", opts.title).unwrap();

    let dims: &[&str] = if opts.swap_dims {
        &["patch", "time"]
    } else {
        &["time", "patch"]
    };
    let vals = [1.0f64, 2.0, opts.third_value];

    if opts.float_coord {
        let mut v = f.add_variable::<f64>("band", &["time"]).unwrap();
        v.put_values(&[1.0f64, 2.0, 3.0], netcdf::Extents::All)
            .unwrap();
    } else {
        let mut v = f.add_variable::<i32>("band", &["time"]).unwrap();
        v.put_values(&[1i32, 2, 3], netcdf::Extents::All).unwrap();
    }

    let mut v = f.add_variable::<f64>("f_fsena", dims).unwrap();
    v.put_values(&vals, netcdf::Extents::All).unwrap();
    v.put_attribute("units", opts.units).unwrap();
    if opts.extra_var_attr {
        v.put_attribute("comment", "extra").unwrap();
    }

    if opts.extra_variable {
        let mut e = f.add_variable::<f64>("f_new", &["time"]).unwrap();
        e.put_values(&[0.0f64, 0.0, 0.0], netcdf::Extents::All)
            .unwrap();
    }
}

fn netcdf_write_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(Clone, Copy)]
struct Opts {
    create_time: &'static str,
    title: &'static str,
    units: &'static str,
    third_value: f64,
    swap_dims: bool,
    float_coord: bool,
    extra_variable: bool,
    extra_var_attr: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            create_time: "20260101-00:00:00 UTC+08:00",
            title: "golden",
            units: "W/m2",
            third_value: 3.0,
            swap_dims: false,
            float_coord: false,
            extra_variable: false,
            extra_var_attr: false,
        }
    }
}

/// 造一对文件：基准用默认选项，对照用 `f` 改过的选项。
fn pair(name: &str, f: impl FnOnce(&mut Opts)) -> (PathBuf, PathBuf) {
    let d = workdir(name);
    let a = d.join("golden.nc");
    let b = d.join("produced.nc");
    write_file(&a, Opts::default());
    let mut o = Opts::default();
    f(&mut o);
    write_file(&b, o);
    (a, b)
}

fn problems(name: &str, f: impl FnOnce(&mut Opts)) -> Vec<String> {
    let (a, b) = pair(name, f);
    compare(&a, &b)
        .expect("compare should succeed on readable files")
        .problems
}

#[test]
fn identical_files_have_no_problems() {
    let p = problems("identical", |_| {});
    assert!(p.is_empty(), "expected no problems, got {p:?}");
}

#[test]
fn changed_value_is_reported_with_variable_and_index() {
    let p = problems("value", |o| o.third_value = 3.5);
    assert_eq!(p.len(), 1, "{p:?}");
    assert!(p[0].starts_with("f_fsena: 1/3 values differ"), "{p:?}");
    assert!(p[0].contains("index 2"), "{p:?}");
}

#[test]
fn changed_variable_attribute_is_reported() {
    let p = problems("varattr", |o| o.units = "BOGUS");
    assert_eq!(p.len(), 1, "{p:?}");
    assert!(p[0].contains("f_fsena attribute units"), "{p:?}");
}

#[test]
fn added_variable_attribute_is_reported() {
    let p = problems("varattr_added", |o| o.extra_var_attr = true);
    assert_eq!(p.len(), 1, "{p:?}");
    assert!(p[0].contains("comment"), "{p:?}");
}

#[test]
fn changed_global_attribute_is_reported() {
    let p = problems("globalattr", |o| o.title = "different");
    assert_eq!(p.len(), 1, "{p:?}");
    assert!(p[0].contains("global attribute title"), "{p:?}");
}

#[test]
fn create_time_alone_is_whitelisted() {
    // 唯一允许不同的属性。CoLM 每次写文件都盖墙上时钟，重跑必然不同，
    // 而黄金基线的全部变量数据逐位相同 —— 这是回归基准得以成立的前提。
    let p = problems("createtime", |o| {
        o.create_time = "19700101-00:00:00 UTC+00:00"
    });
    assert!(p.is_empty(), "create_time must be ignored, got {p:?}");
}

#[test]
fn added_variable_is_reported() {
    let p = problems("addvar", |o| o.extra_variable = true);
    assert_eq!(p.len(), 1, "{p:?}");
    assert!(p[0].contains("variable only in produced: f_new"), "{p:?}");
}

#[test]
fn permuted_dimensions_are_reported_even_though_values_match() {
    // 这条是本文件存在的直接原因。patch 长度为 1，所以 (time,patch) 与
    // (patch,time) 扁平化后元素顺序完全相同，逐值比较看不出任何差异。
    // 而 colm-hist 按轴位置索引，读到这样的文件会静默取错轴。
    let p = problems("permute", |o| o.swap_dims = true);
    assert_eq!(p.len(), 1, "{p:?}");
    assert!(p[0].contains("f_fsena: dimensions"), "{p:?}");
    assert!(p[0].contains("\"time\""), "{p:?}");
}

#[test]
fn changed_storage_type_is_reported_even_though_values_match() {
    // 把整数坐标变量改写成 double：读成 f64 后值完全一样，
    // 但下游按整数索引的代码会变。
    let p = problems("retype", |o| o.float_coord = true);
    assert_eq!(p.len(), 1, "{p:?}");
    assert!(p[0].starts_with("band: type"), "{p:?}");
}

#[test]
fn unreadable_file_is_an_error_not_an_absence_of_differences() {
    // 把「打不开」当成「没有差异」是回归门禁最典型的失效方式。
    let d = workdir("missing");
    let a = d.join("golden.nc");
    write_file(&a, Opts::default());
    let err = compare(&a, &d.join("does-not-exist.nc"));
    assert!(err.is_err(), "missing file must be an error");
}

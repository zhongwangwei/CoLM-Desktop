//! `mt_any_enabled` 与参数文件字段的路径拆法。

use super::*;
use crate::parse;

fn doc(text: &str) -> Document {
    parse(text).unwrap()
}

#[test]
fn families_are_switches_and_solutes_are_a_list() {
    assert!(!any_enabled(&doc("&nl_colm\n/\n")).unwrap());
    assert!(any_enabled(&doc("&nl_colm\nDEF_USE_ISOTOPE=.true.\n/\n")).unwrap());
    assert!(any_enabled(&doc("&nl_colm\nDEF_USE_METHANE=.true.\n/\n")).unwrap());
    assert!(any_enabled(&doc("&nl_colm\nDEF_USE_SEDIMENT=.true.\n/\n")).unwrap());
    assert!(any_enabled(&doc("&nl_colm\nDEF_SOLUTE_NAMES='CL,NO3'\n/\n")).unwrap());
    // 只有空白不算启用（上游 `len_trim(DEF_SOLUTE_NAMES) > 0`）。
    assert!(!any_enabled(&doc("&nl_colm\nDEF_SOLUTE_NAMES='   '\n/\n")).unwrap());
}

#[test]
fn wrong_types_are_refused() {
    assert!(any_enabled(&doc("&nl_colm\nDEF_USE_ISOTOPE='yes'\n/\n")).is_err());
    assert!(any_enabled(&doc("&nl_colm\nDEF_SOLUTE_NAMES=1\n/\n")).is_err());
}

#[test]
fn single_and_list_parameter_fields() {
    let document = doc("&nl_colm\n\
         DEF_ISOTOPE_PARAM_FILE='run/isotope.nml'\n\
         DEF_SOLUTE_PARAM_FILES='CL:run/cl.nml,null;run/no3.nml'\n\
         DEF_METHANE_PARAM_FILE='null'\n/\n");
    assert_eq!(
        param_file_paths(&document, "DEF_ISOTOPE_PARAM_FILE").unwrap(),
        ["run/isotope.nml"]
    );
    assert_eq!(
        param_file_paths(&document, "DEF_SOLUTE_PARAM_FILES").unwrap(),
        ["run/cl.nml", "run/no3.nml"]
    );
    assert!(param_file_paths(&document, "DEF_METHANE_PARAM_FILE")
        .unwrap()
        .is_empty());
    assert!(param_file_paths(&document, "DEF_SEDIMENT_PARAM_FILE")
        .unwrap()
        .is_empty());
}

//! 算例目录布局与序列化。
//!
//! ```text
//! <case>/
//! ├── case.nml       生成，只含偏离默认的字段
//! ├── forcing.nml    生成（colm-forcing）
//! ├── site.nc        补齐后的站点文件（colm-srfdata）
//! └── out/           模型产物
//! ```

use std::path::{Path, PathBuf};

pub struct Layout {
    pub root: PathBuf,
}

impl Layout {
    pub fn new(root: &Path) -> Layout {
        Layout {
            root: root.to_path_buf(),
        }
    }
    pub fn case_nml(&self) -> PathBuf {
        self.root.join("case.nml")
    }
    pub fn forcing_nml(&self) -> PathBuf {
        self.root.join("forcing.nml")
    }
    pub fn site_nc(&self) -> PathBuf {
        self.root.join("site.nc")
    }
    pub fn out(&self) -> PathBuf {
        self.root.join("out")
    }
}

/// 把字段集合渲染成 `nl_colm` 组。
pub fn render(fields: &[&(String, colm_namelist::Value)]) -> String {
    let mut s = String::from("&nl_colm\n\n");
    for (p, v) in fields {
        s.push_str(&format!("   {p} = {v}\n"));
    }
    s.push_str("/\n");
    s
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod layout_tests;

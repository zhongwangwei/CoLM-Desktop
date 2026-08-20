//! 算例目录布局与序列化。
//!
//! ```text
//! <case>/
//! ├── case.nml       生成，只含偏离默认的字段
//! ├── forcing.nml    生成（colm-forcing）
//! ├── site.nc        补齐后的站点文件（colm-srfdata）
//! ├── runtime/       只有城市算例才有：随仓库发的 urban/LUCY_rawdata.nc
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

    /// `DEF_dir_runtime` 指向的目录。
    ///
    /// 只有城市算例用得上：`mkinidata/MOD_UrbanReadin.F90:193` 无条件读
    /// `<runtime>/urban/LUCY_rawdata.nc`，而那张表 37 KB、随仓库发，
    /// 于是每个城市算例自带一份。水热与 BGC 算例的 `DEF_dir_runtime`
    /// 故意指向一个不存在的目录 —— 跑通了就证明没读。
    pub fn runtime(&self) -> PathBuf {
        self.root.join("runtime")
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

/// 从一份 case.nml 里读出 `DEF_CASE_NAME`。
///
/// 算例名决定所有产物路径，所以取错了会一路错到「找不到 history」。
/// 用真解析器而不是字符串查找：后者会被一行注释掉的 `DEF_CASE_NAME` 骗过去。
pub fn case_name(nml: &Path) -> anyhow::Result<String> {
    use anyhow::{bail, Context};
    let text =
        std::fs::read_to_string(nml).with_context(|| format!("cannot read {}", nml.display()))?;
    let doc =
        colm_namelist::parse(&text).with_context(|| format!("cannot parse {}", nml.display()))?;
    match doc.get("DEF_CASE_NAME") {
        Some(colm_namelist::Value::Str(s)) => Ok(s.clone()),
        Some(other) => bail!("DEF_CASE_NAME is {other:?}, not a string"),
        None => bail!("no DEF_CASE_NAME in {}", nml.display()),
    }
}

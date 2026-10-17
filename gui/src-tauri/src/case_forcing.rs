//! Case-owned forcing edits, committed alongside case.nml by the configuration transaction.
use super::*;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
pub struct ForcingInput {
    pub directory: String,
    pub dataset: Option<String>,
}

pub(super) struct Prepared {
    pub path: PathBuf,
    pub text: String,
    source: Option<(PathBuf, String)>,
    previous: Option<String>,
}

fn read(dir: &Path) -> Result<(PathBuf, String, colm_namelist::Document), String> {
    let case = std::fs::read_to_string(dir.join("case.nml")).map_err(|e| e.to_string())?;
    let doc = colm_namelist::parse(&case).map_err(|e| e.to_string())?;
    let path = forcing_path(dir, &doc)?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let doc = colm_namelist::parse(&text).map_err(|e| e.to_string())?;
    Ok((path, text, doc))
}

fn forcing_path(dir: &Path, doc: &colm_namelist::Document) -> Result<PathBuf, String> {
    let name = character(doc, "DEF_forcing_namelist");
    if name.trim().is_empty() || name.eq_ignore_ascii_case("null") {
        return Err("算例没有强迫场配置文件".into());
    }
    let mut path = Path::new(name.trim()).to_path_buf();
    if let Some((_, remote)) = crate::remote::server_case_owner(&dir.to_string_lossy())? {
        if let Ok(relative) = path.strip_prefix(&remote) {
            path = dir.join(relative);
        }
    }
    if !path.is_absolute() {
        path = dir.join(path);
    }
    Ok(path)
}

#[derive(Debug, Serialize)]
pub struct NamelistPreviewFile {
    pub name: String,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct NamelistPreview {
    pub files: Vec<NamelistPreviewFile>,
    pub warnings: Vec<String>,
}

fn preview_text(path: &Path) -> Result<String, String> {
    const LIMIT: usize = 4 * 1024 * 1024;
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut text = String::new();
    file.take(LIMIT as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if text.len() > LIMIT {
        return Err(format!("{}: 配置文件超过 4 MiB，无法预览", path.display()));
    }
    Ok(text)
}

#[tauri::command]
pub fn case_namelist_preview(dir: String) -> Result<NamelistPreview, String> {
    let dir = Path::new(&dir);
    let text = preview_text(&dir.join("case.nml"))?;
    let forcing = colm_namelist::parse(&text)
        .map_err(|e| format!("case.nml: {e}"))
        .and_then(|doc| forcing_path(dir, &doc))
        .and_then(|path| preview_text(&path).map(|text| (path, text)));
    let mut result = NamelistPreview {
        files: vec![NamelistPreviewFile {
            name: "case.nml".into(),
            text,
        }],
        warnings: vec![],
    };
    match forcing {
        Ok((path, text)) => result.files.push(NamelistPreviewFile {
            name: format!(
                "forcing: {}",
                path.file_name().unwrap_or_default().to_string_lossy()
            ),
            text,
        }),
        Err(error) => result.warnings.push(error),
    }
    Ok(result)
}

#[tauri::command]
pub fn case_forcing_input(dir: String) -> Result<ForcingInput, String> {
    let (_, _, doc) = read(Path::new(&dir))?;
    let directory = character(&doc, "DEF_dir_forcing");
    let dataset = character(&doc, "DEF_forcing%dataset");
    if directory.trim().is_empty() || directory.eq_ignore_ascii_case("null") || dataset.is_empty() {
        return Err("强迫场配置缺少数据目录或数据集".into());
    }
    Ok(ForcingInput {
        directory,
        dataset: Some(dataset),
    })
}

pub(super) fn prepare(dirs: &[String], input: &ForcingInput) -> Result<Prepared, String> {
    if dirs.len() != 1
        || !colm_case::is_spatial_case(&Path::new(&dirs[0]).join("case.nml"))
            .map_err(|e| e.to_string())?
    {
        return Err("强迫场数据目录只能在单个区域或全球算例中修改".into());
    }
    let dir = Path::new(&dirs[0]);
    let directory = input.directory.trim();
    if directory.is_empty()
        || directory.eq_ignore_ascii_case("null")
        || directory.chars().any(char::is_control)
    {
        return Err("请选择有效的强迫场数据目录".into());
    }
    let remote = crate::remote::server_case_owner(&dirs[0])?.is_some();
    if remote {
        if !directory.starts_with('/') {
            return Err("请选择服务器上的绝对数据路径".into());
        }
    } else if !Path::new(directory).is_absolute() || !Path::new(directory).is_dir() {
        return Err("请选择存在的强迫场数据目录（绝对路径）".into());
    }
    let (text, source) =
        if let Some(dataset) = input.dataset.as_deref().filter(|v| !v.trim().is_empty()) {
            (
                colm_case::forcing_templates::render_forcing_namelist(dataset, directory)
                    .map_err(|e| e.to_string())?,
                None,
            )
        } else {
            let (path, text, mut doc) = read(dir)?;
            doc.set(
                "DEF_dir_forcing",
                colm_namelist::Value::Str(format!("{}/", directory.trim_end_matches('/'))),
            )
            .map_err(|e| e.to_string())?;
            (doc.to_string(), Some((path, text)))
        };
    let path = dir.join("forcing.nml");
    let previous = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.to_string()),
    };
    Ok(Prepared {
        path,
        text,
        source,
        previous,
    })
}

impl Prepared {
    pub(super) fn unchanged(&self) -> Result<(), String> {
        let current = match std::fs::read_to_string(&self.path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.to_string()),
        };
        if current != self.previous
            || self.source.as_ref().is_some_and(|(path, text)| {
                std::fs::read_to_string(path).as_ref().ok() != Some(text)
            })
        {
            return Err("强迫场配置在校验期间已改变，请重新加载后保存".into());
        }
        Ok(())
    }
    pub(super) fn stage_preview(&self, dir: &str) -> Result<(), String> {
        let path = Path::new(dir).join("forcing.nml");
        std::fs::write(&path, &self.text).map_err(|e| e.to_string())?;
        let case = Path::new(dir).join("case.nml");
        let text = std::fs::read_to_string(&case).map_err(|e| e.to_string())?;
        let mut doc = colm_namelist::parse(&text).map_err(|e| e.to_string())?;
        put(
            &mut doc,
            "DEF_forcing_namelist",
            colm_namelist::Value::Str(path.to_string_lossy().into_owned()),
        )?;
        std::fs::write(case, doc.to_string()).map_err(|e| e.to_string())
    }
}

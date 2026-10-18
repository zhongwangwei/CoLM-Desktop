//! 算例目录的发现与读写。
//!
//! 一个算例就是一个含 `case.nml` 的目录 —— 不引入独立的索引文件，
//! 因为那会立刻带来「索引与磁盘不一致」这个新问题，而目录本身就是真相。

use std::path::{Path, PathBuf};

use serde::Serialize;

pub(crate) fn validate_case_name(name: &str) -> Result<(), String> {
    colm_case::validate_case_name(name).map_err(|error| format!("{error:#}"))
}

#[derive(Debug, Serialize)]
pub struct CaseEntry {
    pub run_host: Option<String>,
    pub remote_directory: Option<String>,
    pub name: String,
    pub dir: String,
    /// 跑过没有 —— 有 history 文件就算跑过
    pub has_history: bool,
    /// Read from the namelist, not inferred from the selected wizard or comments.
    pub spatial: bool,
}

/// 扫一个目录下的算例（只看一层，不递归）。
#[tauri::command]
pub fn list_cases(root: String) -> Result<Vec<CaseEntry>, String> {
    let root = PathBuf::from(root);
    let mut out = Vec::new();
    let rd = std::fs::read_dir(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    for e in rd.flatten() {
        let d = e.path();
        if !d.join("case.nml").is_file() {
            continue;
        }
        let name = colm_case::case_name(&d.join("case.nml"))
            .ok()
            .filter(|name| validate_case_name(name).is_ok())
            .unwrap_or_else(|| {
                d.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            });
        let record = recorded_case(&d);
        out.push(CaseEntry {
            run_host: record.run_host,
            remote_directory: record.remote_directory,
            // Keep malformed cases discoverable for repair; Study validation
            // reports their parse errors before any model can be launched.
            spatial: colm_case::is_spatial_case(&d.join("case.nml")).unwrap_or(false),
            has_history: history_of(&d, &name).is_some() && !colm_case::results_are_stale(&d),
            dir: d.to_string_lossy().into_owned(),
            name,
        });
    }
    crate::history::import_project_metadata(&root)?;
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// 把一个已有项目的设置还原成「新建模拟」第一页的表单：项目里每个算例都带着建例时的选择，
/// 取第一个算例即可。读不到的字段留空，由向导自己的默认值与按服务器记住的路径补上。
fn project_wizard_form(root: String) -> Result<std::collections::BTreeMap<String, String>, String> {
    use crate::config::character;
    let cases = list_cases(root.clone())?;
    let first = cases.first().ok_or_else(|| format!("{root} 里没有算例"))?;
    let dir = PathBuf::from(&first.dir);
    let opened = open_case(first.dir.clone())?;
    let profile = opened.profile;
    let record = recorded_case(&dir);
    let doc = std::fs::read_to_string(dir.join("case.nml"))
        .ok()
        .and_then(|text| colm_namelist::parse(&text).ok());
    let forcing = std::fs::read_to_string(dir.join("forcing.nml"))
        .ok()
        .and_then(|text| colm_namelist::parse(&text).ok());
    let mut form = std::collections::BTreeMap::new();
    let mut put = |key: &str, value: Option<String>| {
        if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
            form.insert(key.to_owned(), value);
        }
    };
    let host = profile.run_host.clone().filter(|host| !host.is_empty());
    let project = match record.remote_directory.as_deref() {
        Some(remote) if host.is_some() => Path::new(remote.trim_end_matches('/'))
            .parent()
            .map(Path::to_path_buf),
        _ => Path::new(&opened.root).to_path_buf().into(),
    };
    put("engine", profile.engine.clone());
    put("host", host.clone());
    put(
        "kind",
        Some(match profile.domain_kind.as_deref() {
            Some(kind) => kind.to_owned(),
            None if profile.spatial => "region".to_owned(),
            None => "site".to_owned(),
        }),
    );
    put("subgrid", Some(profile.subgrid.to_owned()));
    put("landcover", Some(profile.classification.to_owned()));
    put("grid", profile.grid.map(str::to_owned));
    put("rawdata", Some(profile.inputs.rawdata.clone()));
    put("runtime", Some(profile.inputs.runtime.clone()));
    put("start", Some(profile.inputs.start.clone()));
    put("end", Some(profile.inputs.end.clone()));
    put("timestep", Some(format!("{}", profile.inputs.timestep)));
    put(
        "name",
        project
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|n| n.to_string_lossy().into_owned()),
    );
    if host.is_none() {
        put(
            "root",
            project
                .as_ref()
                .and_then(|path| path.parent())
                .map(|path| path.to_string_lossy().into_owned()),
        );
    }
    put("shapefile", profile.shapefile.clone());
    put(
        "forcingdir",
        forcing
            .as_ref()
            .map(|doc| character(doc, "DEF_dir_forcing")),
    );
    let site = record
        .site
        .clone()
        .or_else(|| doc.as_ref().map(|doc| character(doc, "SITE_fsitedata")));
    put(
        "sitedir",
        site.as_deref().and_then(|file| {
            Path::new(file)
                .parent()
                .map(|parent| parent.to_string_lossy().into_owned())
        }),
    );
    Ok(form)
}

/// 向导第 3 步里的一个站点：站点文件、算例名、起止与预热结束日。
#[derive(Debug, Serialize, PartialEq)]
pub struct WizardSiteDraft {
    pub site: String,
    pub name: String,
    pub start: String,
    pub end: String,
    pub spinup_end: String,
    pub spinup_repeat: String,
}

/// 第 3 步站点卡片所需的站点信息（与扫描结果同形），取自算例自己的文件，所以不必重新扫描。
#[derive(Debug, Serialize, PartialEq)]
pub struct WizardSite {
    pub name: String,
    pub site_file: String,
    pub met_file: Option<String>,
    pub obs_file: Option<String>,
    pub urban: bool,
    pub crop: bool,
    pub lon: f64,
    pub lat: f64,
    pub start: String,
    pub end: String,
    pub problem: Option<String>,
}

/// 一个已有项目还原成「新建模拟」五步所需的全部输入。
#[derive(Debug, Serialize, PartialEq)]
pub struct WizardSeed {
    /// 第 1–3 步的表单。
    pub form: std::collections::BTreeMap<String, String>,
    /// 第 4–5 步的模型方案与输出选项（取自第一个算例的 `case.nml`）。
    pub case_values: std::collections::BTreeMap<String, String>,
    /// 多站点项目的站点（含强迫场文件与时段）；空间项目为空。
    pub sites: Vec<WizardSite>,
    /// 多站点项目的逐站设置；空间项目为空。
    pub drafts: Vec<WizardSiteDraft>,
    /// `first-year`：每站都是起始后一年结束预热、重复次数一致；否则 `per-site`。
    pub multi_site_mode: &'static str,
    pub multi_site_repeat: String,
}

fn first_year_cutoff(start: &str) -> Option<String> {
    let year: i64 = start.get(..4)?.parse().ok()?;
    Some(format!("{:04}{}", year + 1, start.get(4..)?))
}

/// 向导不当作「可编辑选项」的字段：路径、身份、时间与由次网格模式决定的开关。
fn wizard_skips(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("def_simulation_time%start_")
        || lower.starts_with("def_simulation_time%end_")
        || lower.starts_with("def_simulation_time%spinup_")
        || lower.starts_with("site_")
        || matches!(
            lower.as_str(),
            "def_case_name"
                | "def_forcing_namelist"
                | "def_subgrid_scheme"
                | "def_use_lct"
                | "def_use_pft"
                | "def_use_pc"
        )
        || ["dir_", "file_", "fsitedata", "filename", "path"]
            .iter()
            .any(|part| lower.contains(part))
}

#[tauri::command]
pub fn project_wizard_seed(root: String) -> Result<WizardSeed, String> {
    use crate::config::integer;
    let form = project_wizard_form(root.clone())?;
    let cases = list_cases(root)?;
    let mut case_values = std::collections::BTreeMap::new();
    if let Some(doc) = cases
        .first()
        .and_then(|case| std::fs::read_to_string(Path::new(&case.dir).join("case.nml")).ok())
        .and_then(|text| colm_namelist::parse(&text).ok())
    {
        for name in doc.paths() {
            let known = colm_schema::find(&name).is_some() || name.starts_with("DEF_hist_vars%");
            if !known || wizard_skips(&name) {
                continue;
            }
            if let Some(value) = doc.get(&name) {
                // The page keeps strings bare ("DAILY") and logicals as literals (".true.").
                let text = match value {
                    colm_namelist::Value::Str(text) => text.clone(),
                    other => other.to_string(),
                };
                case_values.insert(name, text);
            }
        }
    }
    let mut drafts = Vec::new();
    let mut sites = Vec::new();
    for case in cases.iter().filter(|case| !case.spatial) {
        let dir = Path::new(&case.dir);
        let Ok(text) = std::fs::read_to_string(dir.join("case.nml")) else {
            continue;
        };
        let record = recorded_case(dir);
        let Some(site) = record.site.clone() else {
            continue;
        };
        let Ok(profile) = case_profile(&text, false, None, record.mode.as_deref()) else {
            continue;
        };
        let Ok(doc) = colm_namelist::parse(&text) else {
            continue;
        };
        let spinup = |unit: &str| integer(&doc, &format!("DEF_simulation_time%spinup_{unit}"));
        let spinup_end = if spinup("year") > 0 {
            format!(
                "{:04}-{:02}-{:02}",
                spinup("year"),
                spinup("month"),
                spinup("day")
            )
        } else {
            String::new()
        };
        let forcing = std::fs::read_to_string(dir.join("forcing.nml"))
            .ok()
            .and_then(|text| colm_namelist::parse(&text).ok());
        let met_file = forcing.as_ref().and_then(|doc| {
            let prefix = match doc.get("DEF_forcing%fprefix(1)") {
                Some(colm_namelist::Value::Str(prefix)) if !prefix.is_empty() => prefix.clone(),
                _ => return None,
            };
            let directory = crate::config::character(doc, "DEF_dir_forcing");
            Some(format!("{}/{prefix}", directory.trim_end_matches('/')))
        });
        sites.push(WizardSite {
            name: case.name.clone(),
            site_file: site.clone(),
            met_file,
            obs_file: record.obs_file.clone(),
            urban: profile.urban,
            crop: profile.crop,
            lon: crate::config::real(&doc, "SITE_lon_location"),
            lat: crate::config::real(&doc, "SITE_lat_location"),
            start: profile.inputs.start.clone(),
            end: profile.inputs.end.clone(),
            problem: None,
        });
        drafts.push(WizardSiteDraft {
            site,
            name: case.name.clone(),
            start: profile.inputs.start,
            end: profile.inputs.end,
            spinup_end,
            spinup_repeat: integer(&doc, "DEF_simulation_time%spinup_repeat")
                .max(1)
                .to_string(),
        });
    }
    let repeat = drafts
        .first()
        .map(|draft| draft.spinup_repeat.clone())
        .unwrap_or_else(|| "1".to_owned());
    let first_year = !drafts.is_empty()
        && drafts.iter().all(|draft| {
            draft.spinup_repeat == repeat
                && first_year_cutoff(&draft.start).as_deref() == Some(draft.spinup_end.as_str())
        });
    Ok(WizardSeed {
        form,
        case_values,
        sites,
        drafts,
        multi_site_mode: if first_year { "first-year" } else { "per-site" },
        multi_site_repeat: repeat,
    })
}

/// 打开已有算例时从 `case.nml`（和跑过时的 `stages.json`）反推出的向导选择，前端据此恢复会话。
#[derive(Debug, Serialize, PartialEq)]
pub struct CaseProfile {
    pub spatial: bool,
    /// `latlon` / `unstructured` / `catchment`；站点为 `None`。
    pub grid: Option<&'static str>,
    /// 地表分类独立于 PFT/PC 结构，避免结构覆盖 USGS 的内核身份。
    pub classification: &'static str,
    pub classification_source: &'static str,
    /// `IGBP` / `USGS` / `PFT` / `PC`。
    pub subgrid: &'static str,
    /// `vg` / `campbell`。
    pub soil: &'static str,
    pub urban: bool,
    pub lulcc: bool,
    pub bgc: bool,
    pub crop: bool,
    pub methane: bool,
    /// 水同位素（H₂¹⁸O/HDO）与是否分馏、含水层混合水量。
    pub isotope: bool,
    pub fractionation: bool,
    pub aquifer_mixing_mm: Option<f64>,
    /// 溶质与它参数文件里的 `[初始浓度, 降水浓度]`（kg/kg）。
    pub solute: bool,
    pub solute_conc: Option<[f64; 2]>,
    /// 河道泥沙（`SEDIMENT` provider）。
    pub sediment: bool,
    pub river: bool,
    pub rangecheck: bool,
    pub colmdebug: bool,
    pub srfdatadiag: bool,
    /// 空间算例的 `DEF_domain`（西、东、南、北）。
    pub domain: Option<[f64; 4]>,
    /// 经纬度网格的分辨率（`DEF_GRIDBASED_lon_res/lat_res`）。
    pub resolution: Option<[f64; 2]>,
    /// 上次运行用的内核预设（`stages.json` 里记录的 `preset=`）；没跑过为 `None`。
    pub kernel_preset: Option<String>,
    /// 「文件与目录」页建算例表单里的那些输入，打开后原样填回。
    pub inputs: CaseInputs,
    /// 非结构网格的 mesh（`DEF_file_mesh`）与流域网格文件（`DEF_CatchmentMesh_data`）。
    pub mesh_file: Option<String>,
    pub catchment_file: Option<String>,
    /// 建例记录里的范围类型与 Shapefile（`case.nml` 里没有）。
    pub domain_kind: Option<String>,
    pub shapefile: Option<String>,
    /// 甲烷算例 CH4 参数文件里的淹没方案（`DEF_METHANE%inundation_mode`）。
    pub methane_mode: Option<String>,
    /// 建例时选择的计算核心（rust / fortran）。
    pub engine: Option<String>,
    /// 算例运行的 SSH 主机别名；`None` 使用本机。
    pub run_host: Option<String>,
    #[serde(default)]
    pub remote_directory: Option<String>,
    /// 自动匹配的观测文件（内置示例或数据集里的 Flux/clean_observations 观测文件）。
    pub obs_file: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct CaseInputs {
    pub name: String,
    pub rawdata: String,
    pub runtime: String,
    /// `DEF_forcing_namelist`（建例时生成的 `forcing.nml` 或用户自己的）。
    pub forcing_namelist: String,
    /// `YYYY-MM-DD`：起始日与结束日（含）。
    pub start: String,
    pub end: String,
    pub timestep: f64,
}

#[derive(Debug, Serialize)]
pub struct OpenedCase {
    pub entry: CaseEntry,
    /// 算例所在的根目录（`root`），新建算例与查重名都在它下面。
    pub root: String,
    pub profile: CaseProfile,
}

/// GUI 建例时记下的、`case.nml` 里看不出来的向导选择：次网格模式（USGS 不留痕迹，没跑过的算例只能
/// 靠它找回 USGS 内核）、空间范围的类型与流域 Shapefile。
const PROFILE_FILE: &str = ".colm-desktop.json";

#[derive(Debug, Default, Clone, PartialEq, Serialize, serde::Deserialize)]
pub struct CaseRecord {
    /// `igbp`/`usgs`/`pft`/`pc`，城市前缀 `urban-`。
    #[serde(default)]
    pub mode: Option<String>,
    /// `watershed`/`region`/`global`。
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub shapefile: Option<String>,
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(default)]
    pub site: Option<String>,
    #[serde(default)]
    pub obs_file: Option<String>,
    #[serde(default)]
    pub run_host: Option<String>,
    #[serde(default)]
    pub remote_directory: Option<String>,
}

#[tauri::command]
pub fn set_case_run_host(dir: String, host: Option<String>) -> Result<(), String> {
    let dir = Path::new(&dir);
    if !dir.join("case.nml").is_file() {
        return Err(format!("{} 不是算例目录（没有 case.nml）", dir.display()));
    }
    let host = validate_run_host(host)?;
    let path = dir.join(PROFILE_FILE);
    let mut record: CaseRecord = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => CaseRecord::default(),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    if record.remote_directory.is_some() && record.run_host != host {
        return Err("服务器算例不能切换运行位置；请在目标服务器重新创建算例".into());
    }
    record.run_host = host;
    record_case(&dir.to_string_lossy(), &record)
}

pub(crate) fn validate_run_host(host: Option<String>) -> Result<Option<String>, String> {
    let host = host.filter(|host| !host.is_empty());
    if host.as_deref().is_some_and(|host| {
        host.starts_with('-') || host.chars().any(|c| c.is_whitespace() || c.is_control())
    }) {
        return Err("运行主机别名不能包含空白或控制字符，也不能以 - 开头".into());
    }
    Ok(host)
}

/// 保存建例记录；失败交由创建事务回滚。
pub(crate) fn record_case(dir: &str, record: &CaseRecord) -> Result<(), String> {
    crate::config::write_export_text_atomic(
        &Path::new(dir).join(PROFILE_FILE),
        serde_json::to_string(record).map_err(|e| e.to_string())?,
    )
}

pub(crate) fn stage_case_classification(
    dir: &Path,
    scheme: &str,
) -> Result<(PathBuf, String), String> {
    let path = dir.join(PROFILE_FILE);
    let mut record: CaseRecord = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => CaseRecord::default(),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    let prefix = if record
        .mode
        .as_deref()
        .is_some_and(|mode| mode.starts_with("urban-"))
    {
        "urban-"
    } else {
        ""
    };
    record.mode = Some(format!("{prefix}{}", scheme.to_ascii_lowercase()));
    Ok((
        path,
        serde_json::to_string(&record).map_err(|e| e.to_string())?,
    ))
}

pub(crate) fn companion_flux_from_met(met_file: &str) -> Option<String> {
    for (_site_suffix, met_suffix, flux_suffix) in [
        ("_site.nc", "_Met.nc", "_Flux.nc"),
        (
            "_site_v1.nc",
            "_metforcing_v1.nc",
            "_clean_observations_v1.nc",
        ),
    ] {
        if let Some(stem) = met_file.strip_suffix(met_suffix) {
            return Some(format!("{stem}{flux_suffix}"));
        }
    }
    None
}

pub(crate) fn sibling(site: &Path, dir: &str, which: usize) -> Option<PathBuf> {
    companion_name(site, which).and_then(|name| {
        let p = site.parent()?.parent()?.join(dir).join(name);
        p.is_file().then_some(p)
    })
}

pub(crate) fn companion_name(site: &Path, which: usize) -> Option<String> {
    let name = site.file_name()?.to_str()?;
    for (site_suffix, met, flux) in [
        ("_site.nc", "_Met.nc", "_Flux.nc"),
        (
            "_site_v1.nc",
            "_metforcing_v1.nc",
            "_clean_observations_v1.nc",
        ),
    ] {
        if let Some(stem) = name.strip_suffix(site_suffix) {
            return Some(format!("{stem}{}", if which == 0 { met } else { flux }));
        }
    }
    None
}

pub fn case_default_observation(case_dir: &Path, record: &CaseRecord) -> Option<String> {
    if let Some(obs) = &record.obs_file {
        if Path::new(obs).is_file() {
            return Some(obs.clone());
        }
    }
    if let Some(site) = &record.site {
        let site_p = Path::new(site);
        if let Some(obs) = sibling(site_p, "Observation", 1) {
            if obs.is_file() {
                return Some(obs.display().to_string());
            }
        }
    }

    let forcing_nml = case_dir.join("forcing.nml");
    if let Ok(content) = std::fs::read_to_string(&forcing_nml) {
        if let Ok(doc) = colm_namelist::parse(&content) {
            let dir_val = doc.get("DEF_dir_forcing").and_then(|v| match v {
                colm_namelist::Value::Str(s) => Some(s.clone()),
                _ => None,
            });
            let file_val = doc.get("DEF_forcing%fprefix(1)").and_then(|v| match v {
                colm_namelist::Value::Str(s) => Some(s.clone()),
                _ => None,
            });
            if let (Some(f_dir), Some(f_file)) = (dir_val, file_val) {
                if let Some(flux_name) = companion_flux_from_met(&f_file) {
                    let obs_path = Path::new(&f_dir)
                        .parent()
                        .map(|p| p.join("Observation").join(&flux_name));
                    if let Some(p) = obs_path {
                        if p.is_file() {
                            return Some(p.display().to_string());
                        }
                    }
                }
            }
        }
    }

    let mut candidate_dirs: Vec<PathBuf> = Vec::new();
    if let Ok(p) = std::env::var("COLM_EXAMPLES") {
        candidate_dirs.push(PathBuf::from(p).join("Observation"));
    }
    candidate_dirs
        .push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/Observation"));
    if let Ok(home) = std::env::var("HOME") {
        candidate_dirs.push(
            PathBuf::from(&home)
                .join("Library/Application Support/rw.81052.CoLM Desktop/examples/Observation"),
        );
        candidate_dirs.push(
            PathBuf::from(&home)
                .join("Library/Application Support/CoLM Desktop/examples/Observation"),
        );
        candidate_dirs.push(
            PathBuf::from(&home).join(".local/share/rw.81052.CoLM Desktop/examples/Observation"),
        );
    }

    let case_name = case_dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
    for obs_dir in candidate_dirs {
        if !obs_dir.is_dir() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&obs_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if fname.starts_with('.') || !fname.ends_with(".nc") {
                    continue;
                }
                let stem = fname.split('_').next().unwrap_or("");
                if !stem.is_empty()
                    && (case_name == stem
                        || case_name.starts_with(&format!("{stem}-"))
                        || case_name.starts_with(&format!("{stem}_")))
                {
                    return Some(path.display().to_string());
                }
            }
        }
    }

    let site_nc = case_dir.join("site.nc");
    if site_nc.is_file() {
        if let Some(obs) = sibling(&site_nc, "Observation", 1) {
            if obs.is_file() {
                return Some(obs.display().to_string());
            }
        }
    }

    None
}

fn recorded_case(dir: &Path) -> CaseRecord {
    std::fs::read_to_string(dir.join(PROFILE_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// 「打开已有算例」：`dir` 必须是含 `case.nml` 的算例目录。
#[tauri::command]
pub fn open_case(dir: String) -> Result<OpenedCase, String> {
    let dir = PathBuf::from(dir);
    let nml = dir.join("case.nml");
    if !nml.is_file() {
        return Err(format!("{} 不是算例目录（没有 case.nml）", dir.display()));
    }
    let root = dir
        .parent()
        .ok_or_else(|| format!("{} 没有上级目录", dir.display()))?
        .to_string_lossy()
        .into_owned();
    let entry = list_cases(root.clone())?
        .into_iter()
        .find(|c| Path::new(&c.dir) == dir)
        .ok_or_else(|| format!("{} 读不出算例", dir.display()))?;
    let profile = case_profile_of(&dir, entry.spatial)?;
    Ok(OpenedCase {
        entry,
        root,
        profile,
    })
}

/// One case's profile straight from its own files. `open_case` also lists every sibling and
/// records the project in history; reading many cases at once needs only this.
pub(crate) fn case_profile_of(dir: &Path, spatial: bool) -> Result<CaseProfile, String> {
    let nml = dir.join("case.nml");
    if !nml.is_file() {
        return Err(format!("{} 不是算例目录（没有 case.nml）", dir.display()));
    }
    let text = std::fs::read_to_string(&nml).map_err(|e| format!("{}: {e}", nml.display()))?;
    let stages = std::fs::read_to_string(dir.join("stages.json")).ok();
    let record = recorded_case(dir);
    let mut profile = case_profile(&text, spatial, stages.as_deref(), record.mode.as_deref())?;
    profile.obs_file = case_default_observation(dir, &record);
    profile.domain_kind = record.domain;
    profile.shapefile = record.shapefile;
    profile.engine = record.engine;
    profile.run_host = record.run_host;
    profile.remote_directory = record.remote_directory;
    let doc = colm_namelist::parse(&text).ok();
    profile.methane_mode = doc
        .as_ref()
        .and_then(|doc| crate::config::methane_mode(doc, dir));
    profile.solute_conc = doc
        .as_ref()
        .and_then(|doc| crate::config::solute_concentrations(doc, dir));
    Ok(profile)
}

/// 自含的站点算例不需要 RawData/Runtime，`colm-cli new` 填的是算例里的占位目录
/// （`rawdata_unused/`、`runtime_unused/`）。它们不能回填进表单：会被记住，之后别的算例
/// 都指到这个算例的空目录里。
fn real_input_dir(dir: String) -> String {
    let trimmed = dir.trim_end_matches(['/', '\\']);
    if trimmed.ends_with("rawdata_unused") || trimmed.ends_with("runtime_unused") {
        String::new()
    } else {
        dir
    }
}

/// 反推规则：网格看哪种网格字段写了；次网格看 `DEF_USE_PFT/PC`，分类优先算例显式选择、
/// 建例记录，最后回退上次内核；作物看内核 CROP 宏或建例播种日字段（`DEF_USE_CROP`
/// 只是宏的只读反映，建例时不写）；甲烷看示踪物名单；河湖汇流只对经纬度与非结构网格有意义（缺省开）。
pub(crate) fn case_profile(
    text: &str,
    spatial: bool,
    stages: Option<&str>,
    mode: Option<&str>,
) -> Result<CaseProfile, String> {
    use crate::config::{character, integer, logical, real};
    let doc = colm_namelist::parse(text).map_err(|e| format!("case.nml: {e}"))?;
    let has = |name: &str| doc.get(name).is_some();
    // 只看算例里显式写了的值：schema 缺省是占位路径，不代表用了这种网格。
    let set = |name: &str| match doc.get(name) {
        Some(colm_namelist::Value::Str(value)) => {
            !value.trim().is_empty() && !value.trim().eq_ignore_ascii_case("null")
        }
        _ => false,
    };
    let grid = if !spatial {
        None
    } else if set("DEF_CatchmentMesh_data") {
        Some("catchment")
    } else if has("DEF_GRIDBASED_lon_res") || has("DEF_GRIDBASED_lat_res") {
        Some("latlon")
    } else {
        Some("unstructured")
    };
    // `stages.json` 每段都记着同一个内核：`preset=latlon;...;args=GRID LULC_USGS ...;macros=...`。
    let kernel = stages
        .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
        .and_then(|value| {
            value.as_object().and_then(|stages| {
                stages
                    .values()
                    .find_map(|stage| stage.get("kernel")?.as_str().map(str::to_owned))
            })
        });
    let kernel_field = |key: &str| {
        kernel.as_deref().and_then(|k| {
            k.split(';')
                .find_map(|part| part.strip_prefix(key).map(str::to_owned))
        })
    };
    let macros = kernel_field("macros=");
    let has_macro = |name: &str| {
        macros
            .as_deref()
            .is_some_and(|m| m.split(',').any(|x| x == name))
    };
    let selected_scheme = character(&doc, "DEF_SUBGRID_SCHEME").to_ascii_uppercase();
    let selected_classification = match selected_scheme.as_str() {
        "USGS" => Some(true),
        "IGBP" => Some(false),
        _ => None,
    };
    let usgs = if let Some(usgs) = selected_classification {
        usgs
    } else if let Some(mode) = mode {
        mode.ends_with("usgs")
    } else {
        has_macro("LULC_USGS")
    };
    let crop = has_macro("CROP") || has("DEF_TUNING_CROP_PLANTING_DAY");
    let subgrid = if logical(&doc, "DEF_USE_PFT") {
        "PFT"
    } else if logical(&doc, "DEF_USE_PC") {
        "PC"
    } else if usgs {
        "USGS"
    } else {
        "IGBP"
    };
    let methane = logical(&doc, "DEF_USE_TRACER")
        && character(&doc, "DEF_TRACER_NAMES").split(',').any(|name| {
            name.trim()
                .trim_matches(['\'', '"'])
                .eq_ignore_ascii_case("CH4")
        });
    let gridded_routing = matches!(grid, Some("latlon" | "unstructured"));
    // `DEF_simulation_time%<which>_year/month/day`（缺省取 schema 里的值）。
    let date = |which: &str| {
        let part = |unit: &str| integer(&doc, &format!("DEF_simulation_time%{which}_{unit}"));
        format!(
            "{:04}-{:02}-{:02}",
            part("year"),
            part("month"),
            part("day")
        )
    };
    let finite = |v: f64| v.is_finite().then_some(v);
    // 只认显式写了的范围：schema 缺省是 ±180/±90，非结构与流域网格常常不写，不能当成全球。
    let explicit_domain = ["edgew", "edgee", "edges", "edgen"]
        .iter()
        .all(|edge| has(&format!("DEF_domain%{edge}")));
    let domain = (spatial && explicit_domain)
        .then(|| {
            Some([
                finite(real(&doc, "DEF_domain%edgew"))?,
                finite(real(&doc, "DEF_domain%edgee"))?,
                finite(real(&doc, "DEF_domain%edges"))?,
                finite(real(&doc, "DEF_domain%edgen"))?,
            ])
        })
        .flatten();
    let resolution = (grid == Some("latlon"))
        .then(|| {
            Some([
                finite(real(&doc, "DEF_GRIDBASED_lon_res"))?,
                finite(real(&doc, "DEF_GRIDBASED_lat_res"))?,
            ])
        })
        .flatten();
    Ok(CaseProfile {
        spatial,
        grid,
        classification: if usgs { "USGS" } else { "IGBP" },
        classification_source: if selected_classification.is_some() {
            "case"
        } else if mode.is_some() {
            "creation"
        } else if macros.is_some() {
            "kernel"
        } else {
            "legacy-default"
        },
        subgrid,
        soil: if logical(&doc, "DEF_USE_Campbell_SOIL_MODEL") {
            "campbell"
        } else {
            "vg"
        },
        urban: logical(&doc, "DEF_URBAN_RUN"),
        lulcc: logical(&doc, "DEF_USE_LULCC"),
        bgc: logical(&doc, "DEF_USE_BGC"),
        crop,
        methane,
        isotope: logical(&doc, "DEF_USE_TRACER")
            && character(&doc, "DEF_TRACER_TYPES")
                .split(',')
                .any(|kind| kind.trim().eq_ignore_ascii_case("isotope")),
        fractionation: logical(&doc, "DEF_TRACER_USE_FRACTIONATION"),
        aquifer_mixing_mm: Some(real(&doc, "DEF_TRACER_AQUIFER_MIXING_WATER_MM"))
            .filter(|value| *value > 0.0),
        solute: logical(&doc, "DEF_USE_TRACER")
            && character(&doc, "DEF_TRACER_TYPES")
                .split(',')
                .any(|kind| kind.trim().eq_ignore_ascii_case("solute")),
        solute_conc: None,
        sediment: logical(&doc, "DEF_USE_TRACER")
            && character(&doc, "DEF_TRACER_NAMES").split(',').any(|name| {
                matches!(
                    name.trim().to_ascii_uppercase().as_str(),
                    "SEDIMENT" | "SED"
                )
            }),
        river: gridded_routing && logical(&doc, "DEF_USE_GridRiverLakeFlow"),
        rangecheck: logical(&doc, "DEF_USE_RangeCheck"),
        colmdebug: logical(&doc, "DEF_USE_CoLMDEBUG"),
        srfdatadiag: logical(&doc, "DEF_USE_SrfdataDiag"),
        domain,
        resolution,
        kernel_preset: kernel_field("preset="),
        inputs: CaseInputs {
            name: character(&doc, "DEF_CASE_NAME"),
            rawdata: real_input_dir(character(&doc, "DEF_dir_rawdata")),
            runtime: real_input_dir(character(&doc, "DEF_dir_runtime")),
            forcing_namelist: character(&doc, "DEF_forcing_namelist"),
            start: date("start"),
            end: date("end"),
            timestep: real(&doc, "DEF_simulation_time%timestep"),
        },
        mesh_file: (grid == Some("unstructured") && set("DEF_file_mesh"))
            .then(|| character(&doc, "DEF_file_mesh")),
        catchment_file: (grid == Some("catchment") && set("DEF_CatchmentMesh_data"))
            .then(|| character(&doc, "DEF_CatchmentMesh_data")),
        domain_kind: None,
        shapefile: None,
        methane_mode: None,
        engine: None,
        run_host: None,
        remote_directory: None,
        obs_file: None,
    })
}

/// 参数已写入、新的 colm 还没有成功时，旧 history 不能跨重启重新变成“可用”。
#[tauri::command]
pub fn mark_results_stale(dirs: Vec<String>) -> Result<(), String> {
    let cases: Vec<PathBuf> = dirs.into_iter().map(PathBuf::from).collect();
    for case in &cases {
        if !case.join("case.nml").is_file() {
            return Err(format!("{} 不是算例目录", case.display()));
        }
    }
    for case in &cases {
        colm_case::mark_results_stale(case)
            .map_err(|error| format!("{}: {error}", case.display()))?;
    }
    Ok(())
}

/// 算例里那个唯一的 `*_hist_*.nc`，没有就是没跑过。
fn history_of(case: &Path, name: &str) -> Option<PathBuf> {
    validate_case_name(name).ok()?;
    let text = std::fs::read_to_string(case.join("case.nml")).ok()?;
    let document = colm_namelist::parse(&text).ok()?;
    let output = match document.get("DEF_dir_output") {
        Some(colm_namelist::Value::Str(path)) if !path.trim().is_empty() => {
            case.join(path.trim_end())
        }
        None => case.join("out"),
        _ => return None,
    };
    let dir = output.join(name).join("history");
    let mut h: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains("_hist_") && n.ends_with(".nc"))
        })
        .collect();
    h.sort();
    h.pop()
}

/// 读一份文本文件（前端拿 case.nml 来编辑）。
#[tauri::command]
pub fn read_text(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))
}

// 这里原来有一个 `write_text` —— 前端拿它把改过的 case.nml 写回去。
// 删掉了：参数改动一律由 `config::set_field_batch`
// 在后端读改写，**前端不再持有落盘的能力**。留着一个通用的"写任意路径"
// 命令，等于给"只改了第一个算例"那类 bug 留一条随时可走的路。

#[cfg(test)]
#[path = "project_tests.rs"]
mod project_tests;

/// Open an existing local artifact with its platform-associated application.
#[tauri::command]
// Each platform builds `result` differently; the binding keeps the cfg branches statements.
#[allow(clippy::let_and_return)]
pub fn open_path(path: String) -> Result<(), String> {
    let path = std::path::Path::new(&path)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");
    // explorer.exe rejects some verbatim (\\?\) paths and often exits non-zero even after
    // opening the file, so pass a plain path and treat a successful launch as success.
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer.exe")
        .arg(plain_windows_path(&path.to_string_lossy()))
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open artifact: {e}"));
    #[cfg(not(target_os = "windows"))]
    let result = {
        let status = command.arg(path).status().map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("could not open artifact: {status}"))
        }
    };
    result
}

/// `canonicalize` on Windows yields verbatim paths (`\\?\C:\…`, `\\?\UNC\server\…`);
/// return the ordinary spelling that shell programs accept.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn plain_windows_path(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        path.to_string()
    }
}

//! 空间算例的标准网格强迫场：上游 `run/forcing/*.nml` 的模板，用户只需选数据集与本机数据目录。
//!
//! 模板原样内嵌（编译期 `include_str!`），只把 `DEF_dir_forcing` 换成用户给的目录；其余字段
//! （变量名、时间步、偏移、分组方式……）都是数据集本身的约定，不该由用户逐项填写。站点用的
//! `POINT.nml` 不在其中。

use anyhow::{bail, Result};

/// `(数据集名, 模板内容)`，按名字排序。
pub const FORCING_TEMPLATES: &[(&str, &str)] = &[
    (
        "CLDAS",
        include_str!("../../../vendor/CoLM202X/run/forcing/CLDAS.nml"),
    ),
    (
        "CMFD",
        include_str!("../../../vendor/CoLM202X/run/forcing/CMFD.nml"),
    ),
    (
        "CMFDv2",
        include_str!("../../../vendor/CoLM202X/run/forcing/CMFDv2.nml"),
    ),
    (
        "CRA40",
        include_str!("../../../vendor/CoLM202X/run/forcing/CRA40.nml"),
    ),
    (
        "CRUJRA",
        include_str!("../../../vendor/CoLM202X/run/forcing/CRUJRA.nml"),
    ),
    (
        "CRUNCEPV4",
        include_str!("../../../vendor/CoLM202X/run/forcing/CRUNCEPV4.nml"),
    ),
    (
        "CRUNCEPV7",
        include_str!("../../../vendor/CoLM202X/run/forcing/CRUNCEPV7.nml"),
    ),
    (
        "ERA5",
        include_str!("../../../vendor/CoLM202X/run/forcing/ERA5.nml"),
    ),
    (
        "ERA5LAND",
        include_str!("../../../vendor/CoLM202X/run/forcing/ERA5LAND.nml"),
    ),
    (
        "ERA5_LEddy",
        include_str!("../../../vendor/CoLM202X/run/forcing/ERA5_LEddy.nml"),
    ),
    (
        "GDAS",
        include_str!("../../../vendor/CoLM202X/run/forcing/GDAS.nml"),
    ),
    (
        "GSWP3",
        include_str!("../../../vendor/CoLM202X/run/forcing/GSWP3.nml"),
    ),
    (
        "IsoGSM",
        include_str!("../../../vendor/CoLM202X/run/forcing/IsoGSM.nml"),
    ),
    (
        "JRA3Q",
        include_str!("../../../vendor/CoLM202X/run/forcing/JRA3Q.nml"),
    ),
    (
        "JRA55",
        include_str!("../../../vendor/CoLM202X/run/forcing/JRA55.nml"),
    ),
    (
        "MPI-ESM1-2-HR_ssp585",
        include_str!("../../../vendor/CoLM202X/run/forcing/MPI-ESM1-2-HR_ssp585.nml"),
    ),
    (
        "MSWX",
        include_str!("../../../vendor/CoLM202X/run/forcing/MSWX.nml"),
    ),
    (
        "PRINCETON",
        include_str!("../../../vendor/CoLM202X/run/forcing/PRINCETON.nml"),
    ),
    (
        "QIAN",
        include_str!("../../../vendor/CoLM202X/run/forcing/QIAN.nml"),
    ),
    (
        "TPMFD",
        include_str!("../../../vendor/CoLM202X/run/forcing/TPMFD.nml"),
    ),
    (
        "WFDE5",
        include_str!("../../../vendor/CoLM202X/run/forcing/WFDE5.nml"),
    ),
    (
        "WFDEI",
        include_str!("../../../vendor/CoLM202X/run/forcing/WFDEI.nml"),
    ),
];

/// 可选的数据集名。
pub fn forcing_datasets() -> impl Iterator<Item = &'static str> {
    FORCING_TEMPLATES.iter().map(|(name, _)| *name)
}

/// 按数据集模板生成 forcing namelist：只替换第一处（非注释的）`DEF_dir_forcing` 赋值，目录补上结尾的 `/`。
pub fn render_forcing_namelist(dataset: &str, forcing_dir: &str) -> Result<String> {
    let Some((_, template)) = FORCING_TEMPLATES.iter().find(|(name, _)| *name == dataset) else {
        bail!(
            "unknown forcing dataset {dataset:?}; known: {}",
            forcing_datasets().collect::<Vec<_>>().join(", ")
        );
    };
    let dir = forcing_dir.trim();
    if dir.is_empty() {
        bail!("the forcing data directory is empty");
    }
    if dir.contains('\'') {
        bail!("the forcing data directory must not contain a single quote: {dir}");
    }
    let dir = if dir.ends_with('/') || dir.ends_with('\\') {
        dir.to_owned()
    } else {
        format!("{dir}/")
    };
    let mut replaced = false;
    let mut out = String::with_capacity(template.len() + dir.len());
    for line in template.split_inclusive('\n') {
        let code = line.trim_start();
        if !replaced
            && !code.starts_with('!')
            && code.starts_with("DEF_dir_forcing")
            && code.contains('=')
        {
            let indent = &line[..line.len() - code.len()];
            out.push_str(&format!("{indent}DEF_dir_forcing  = '{dir}'\n"));
            replaced = true;
        } else {
            out.push_str(line);
        }
    }
    if !replaced {
        bail!("the {dataset} template has no DEF_dir_forcing line");
    }
    Ok(out)
}

#[cfg(test)]
#[path = "forcing_templates_tests.rs"]
mod forcing_templates_tests;

/// 与运行期文件命名一致，年份串由调用方给：`DEF_USE_ClimForcing_for_Spinup` 预热期是 `'clim'`
/// （`MOD_UserSpecifiedForcing.F90:172-176`）。
pub fn metfilename_with_year(
    dataset: &str,
    prefix: &str,
    y: &str,
    month: i32,
    variable: usize,
) -> String {
    let m = format!("{month:02}");
    match dataset {
        "PRINCETON" => format!("/{prefix}{y}-{y}.nc"),
        "GSWP3" | "QIAN" | "CRUNCEPV4" | "CRUNCEPV7" | "WFDEI" => format!("/{prefix}{y}-{m}.nc"),
        "ERA5LAND" => {
            const SUFFIX: [&str; 8] = [
                "_2m_temperature.nc",
                "_specific_humidity.nc",
                "_surface_pressure.nc",
                "_total_precipitation_m_hr.nc",
                "_10m_u_component_of_wind.nc",
                "_10m_v_component_of_wind.nc",
                "_surface_solar_radiation_downwards_w_m2.nc",
                "_surface_thermal_radiation_downwards_w_m2.nc",
            ];
            format!("/{prefix}_{y}_{m}{}", SUFFIX[variable])
        }
        "ERA5" => {
            const SUFFIX: [&str; 8] = [
                "_2m_temperature.nc4",
                "_q.nc4",
                "_surface_pressure.nc4",
                "_mean_total_precipitation_rate.nc4",
                "_10m_u_component_of_wind.nc4",
                "_10m_v_component_of_wind.nc4",
                "_mean_surface_downward_short_wave_radiation_flux.nc4",
                "_mean_surface_downward_long_wave_radiation_flux.nc4",
            ];
            format!("/{prefix}_{y}_{m}{}", SUFFIX[variable])
        }
        "MSWX" | "JRA3Q" => format!("/{prefix}_{y}_{m}.nc"),
        "WFDE5" => format!("/{prefix}{y}{m}_v2.1.nc"),
        "CRUJRA" => format!("/{prefix}{y}.365d.noc.nc"),
        "JRA55" | "CMFDv2" | "TPMFD" => format!("/{prefix}{y}{m}.nc"),
        "GDAS" | "CMFD" => format!("/{prefix}{y}{m}.nc4"),
        "CLDAS" => format!("/{prefix}-{y}{m}.nc"),
        "CMIP6" | "CRA40" | "IsoGSM" => format!("/{prefix}_{y}.nc"),
        other => unreachable!("dataset {other} has no metfilename branch"),
    }
}

#[path = "forcing_directory.rs"]
mod directory;
pub use directory::{forcing_directory_candidates, forcing_directory_parents};

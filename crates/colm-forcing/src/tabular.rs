//! CSV/TXT forcing import for one or many point sites.
//!
//! A table is only an ingestion format.  Each site is split into a standard
//! point NetCDF file, then the existing gap-repair pipeline owns interpolation,
//! ERA5-Land correction, and provenance.  Keeping that boundary prevents CSV
//! and NetCDF inputs from acquiring different scientific behaviour.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::check::cadence_problem;
use crate::convert::{canonical_units, Heights};
use crate::gapfill::decide_timezone_with_solar;
use crate::slots::SLOTS;

const MAX_SITE_STEPS: usize = 10_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LandCoverScheme {
    Igbp,
    Usgs,
    Pft,
    Pc,
    Urban,
}

impl LandCoverScheme {
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "igbp" => Ok(Self::Igbp),
            "usgs" => Ok(Self::Usgs),
            "pft" => Ok(Self::Pft),
            "pc" => Ok(Self::Pc),
            "urban" => Ok(Self::Urban),
            other => bail!("unsupported land-cover scheme {other:?}"),
        }
    }

    fn validate(self, site: &str, value: i32) -> Result<()> {
        let (name, range) = match self {
            Self::Usgs => ("USGS", 1..=24),
            Self::Urban => ("LCZ", 1..=10),
            Self::Igbp | Self::Pft | Self::Pc => ("IGBP", 1..=17),
        };
        if !range.contains(&value) {
            bail!("site {site:?} {name} class {value} is outside {range:?}");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabularColumn {
    pub name: String,
    pub units: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabularSiteProbe {
    pub id: String,
    pub rows: usize,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub landtype: Option<i32>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub step_seconds: Option<i64>,
    pub inserted_steps: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabularSlotGuess {
    pub index: usize,
    pub meaning: &'static str,
    pub optional: bool,
    pub column: Option<String>,
    pub units: Option<String>,
    pub wants: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabularProbe {
    pub delimiter: &'static str,
    pub columns: Vec<TabularColumn>,
    pub rows: usize,
    pub site_column: Option<String>,
    pub time_column: Option<String>,
    pub latitude_column: Option<String>,
    pub longitude_column: Option<String>,
    pub landtype_column: Option<String>,
    pub utc_offset_column: Option<String>,
    pub sites: Vec<TabularSiteProbe>,
    pub slots: Vec<TabularSlotGuess>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabularSlot {
    pub index: usize,
    pub column: String,
    pub source_units: String,
    pub also_add: Vec<String>,
}

impl TabularSlot {
    pub fn new(index: usize, column: impl Into<String>, units: impl Into<String>) -> Self {
        Self {
            index,
            column: column.into(),
            source_units: units.into(),
            also_add: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TabularPlan {
    pub time_column: String,
    pub site_column: Option<String>,
    pub latitude_column: Option<String>,
    pub longitude_column: Option<String>,
    pub landtype_column: Option<String>,
    pub utc_offset_column: Option<String>,
    pub manual_utc_offset: Option<f64>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    /// Explicit step wins over inference. This is necessary when the table has
    /// only two records separated by a missing row: the intended cadence cannot
    /// be recovered from those two timestamps alone.
    pub step_seconds: Option<i64>,
    /// The selected kernel/subgrid scheme decides how a numeric classification
    /// column is interpreted. Without it a value such as 20 is ambiguous.
    pub land_cover_scheme: Option<LandCoverScheme>,
    pub heights: Option<Heights>,
    pub slots: Vec<TabularSlot>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportedTableSite {
    pub site: String,
    pub safe_site: String,
    pub staged_path: PathBuf,
    pub final_path: PathBuf,
    pub rows: usize,
    pub inserted_steps: usize,
    pub latitude: f64,
    pub longitude: f64,
    pub landtype: Option<i32>,
    /// `None` means explicit timestamp offsets varied over the series.
    pub timezone_offset_hours: Option<f64>,
    pub timezone_source: String,
    pub start_utc: i64,
    pub end_utc: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Delimiter {
    Comma,
    Tab,
    Semicolon,
    Whitespace,
}

impl Delimiter {
    fn name(self) -> &'static str {
        match self {
            Self::Comma => "comma",
            Self::Tab => "tab",
            Self::Semicolon => "semicolon",
            Self::Whitespace => "whitespace",
        }
    }

    fn character(self) -> Option<char> {
        match self {
            Self::Comma => Some(','),
            Self::Tab => Some('\t'),
            Self::Semicolon => Some(';'),
            Self::Whitespace => None,
        }
    }
}

#[derive(Debug, Clone)]
struct Row {
    line: usize,
    cells: Vec<String>,
}

#[derive(Debug, Clone)]
struct Table {
    delimiter: Delimiter,
    columns: Vec<TabularColumn>,
    rows: Vec<Row>,
}

pub fn probe_table(path: &Path) -> Result<TabularProbe> {
    let table = read_table(path)?;
    let names = table
        .columns
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let site_column = guess_column(&names, &["site", "site_id", "station", "station_id"]);
    let time_column = guess_column(
        &names,
        &[
            "time",
            "timestamp",
            "TIMESTAMP_START",
            "datetime",
            "date_time",
            "date",
        ],
    );
    let latitude_column = guess_column(&names, &["latitude", "lat", "site_latitude"]);
    let longitude_column = guess_column(&names, &["longitude", "lon", "lng", "site_longitude"]);
    let landtype_column = guess_column(
        &names,
        &[
            "landtype",
            "land_type",
            "land_cover",
            "landcover",
            "igbp",
            "usgs",
            "classification",
        ],
    );
    let utc_offset_column = guess_column(
        &names,
        &[
            "utc_offset",
            "utc_offset_hours",
            "timezone_offset",
            "tz_offset",
        ],
    );
    let sites = probe_sites(
        path,
        &table,
        ProbeSiteColumns {
            site: site_column.as_deref(),
            time: time_column.as_deref(),
            utc_offset: utc_offset_column.as_deref(),
            latitude: latitude_column.as_deref(),
            longitude: longitude_column.as_deref(),
            landtype: landtype_column.as_deref(),
        },
    )?;
    let slots = SLOTS
        .iter()
        .map(|slot| {
            let column = slot_aliases(slot.index)
                .iter()
                .find_map(|candidate| guess_column(&names, &[*candidate]));
            let units = column.as_deref().and_then(|name| {
                table
                    .columns
                    .iter()
                    .find(|candidate| candidate.name == name)
                    .and_then(|candidate| candidate.units.clone())
                    .or_else(|| inferred_units(name))
            });
            TabularSlotGuess {
                index: slot.index,
                meaning: slot.meaning,
                optional: slot.optional,
                column,
                units,
                wants: canonical_units(slot.index),
            }
        })
        .collect();

    Ok(TabularProbe {
        delimiter: table.delimiter.name(),
        columns: table.columns,
        rows: table.rows.len(),
        site_column,
        time_column,
        latitude_column,
        longitude_column,
        landtype_column,
        utc_offset_column,
        sites,
        slots,
    })
}

pub fn import_table(
    path: &Path,
    destination: &Path,
    plan: &TabularPlan,
) -> Result<Vec<ImportedTableSite>> {
    let table = read_table(path)?;
    validate_plan(&table, plan)?;
    let time_index = column_index(&table, &plan.time_column)?;
    let site_index = plan
        .site_column
        .as_deref()
        .map(|name| column_index(&table, name))
        .transpose()?;
    let latitude_index = plan
        .latitude_column
        .as_deref()
        .map(|name| column_index(&table, name))
        .transpose()?;
    let longitude_index = plan
        .longitude_column
        .as_deref()
        .map(|name| column_index(&table, name))
        .transpose()?;
    let landtype_index = plan
        .landtype_column
        .as_deref()
        .map(|name| column_index(&table, name))
        .transpose()?;
    let offset_index = plan
        .utc_offset_column
        .as_deref()
        .map(|name| column_index(&table, name))
        .transpose()?;
    let swdown_index = plan
        .slots
        .iter()
        .find(|slot| slot.index == 7)
        .map(|slot| column_index(&table, &slot.column))
        .transpose()?;

    let fallback_site = fallback_site_name(path)?;
    let mut groups = BTreeMap::<String, Vec<&Row>>::new();
    for row in &table.rows {
        let site = match site_index {
            Some(index) => {
                let value = row.cells[index].trim();
                if value.is_empty() {
                    bail!("line {} has an empty site identifier", row.line);
                }
                value
            }
            None => &fallback_site,
        };
        groups.entry(site.to_string()).or_default().push(row);
    }
    if groups.is_empty() {
        bail!("{} contains no data rows", path.display());
    }
    if groups.len() > 1 && (latitude_index.is_none() || longitude_index.is_none()) {
        bail!(
            "a table with multiple sites must provide latitude and longitude columns; one fallback coordinate cannot safely describe every site"
        );
    }

    let mut safe_names = BTreeMap::<String, String>::new();
    for site in groups.keys() {
        let safe = safe_site_name(site)?;
        if let Some(other) = safe_names.insert(safe.to_ascii_lowercase(), site.clone()) {
            bail!("site names {other:?} and {site:?} both normalize to {safe:?}");
        }
    }

    let staging = destination.join(".colm-tabular");
    std::fs::create_dir_all(&staging)
        .with_context(|| format!("cannot create {}", staging.display()))?;
    std::fs::create_dir_all(destination)
        .with_context(|| format!("cannot create {}", destination.display()))?;

    let mut imported = Vec::with_capacity(groups.len());
    for (site, rows) in groups {
        let safe_site = safe_site_name(&site)?;
        let latitude = consistent_number(&rows, latitude_index, "latitude")?
            .or(plan.latitude)
            .with_context(|| {
                format!("site {site:?} has no latitude column or fallback latitude")
            })?;
        let longitude = consistent_number(&rows, longitude_index, "longitude")?
            .or(plan.longitude)
            .with_context(|| {
                format!("site {site:?} has no longitude column or fallback longitude")
            })?;
        if !(-90.0..=90.0).contains(&latitude) {
            bail!("site {site:?} latitude {latitude} is outside -90..=90");
        }
        if !(-180.0..=180.0).contains(&longitude) {
            bail!("site {site:?} longitude {longitude} is outside -180..=180");
        }
        let landtype = consistent_integer(&rows, landtype_index, "landtype")?;
        if let Some(value) = landtype {
            let scheme = plan.land_cover_scheme.context(
                "a land-cover column needs the selected scheme (IGBP/USGS/PFT/PC/Urban)",
            )?;
            scheme.validate(&site, value)?;
        }
        let mut records = Vec::with_capacity(rows.len());
        let mut resolved_row_offsets = Vec::new();
        let mut saw_timestamp_offset = false;
        let mut saw_column_offset = false;
        for row in &rows {
            let parsed = parse_timestamp(&row.cells[time_index]).with_context(|| {
                format!(
                    "line {} has unsupported timestamp {:?}",
                    row.line, row.cells[time_index]
                )
            })?;
            let column_offset = offset_index
                .map(|index| parse_offset_cell(row, index))
                .transpose()?
                .flatten();
            if let (Some(timestamp), Some(column)) = (parsed.offset_seconds, column_offset) {
                if timestamp != column {
                    bail!(
                        "line {} timestamp UTC offset disagrees with the UTC offset column",
                        row.line
                    );
                }
            }
            saw_timestamp_offset |= parsed.offset_seconds.is_some();
            saw_column_offset |= column_offset.is_some();
            let resolved = parsed.offset_seconds.or(column_offset);
            resolved_row_offsets.push(resolved);
            records.push((row, parsed, resolved));
        }
        let has_explicit = resolved_row_offsets.iter().any(Option::is_some);
        let all_explicit = resolved_row_offsets.iter().all(Option::is_some);
        if has_explicit && !all_explicit {
            bail!("site {site:?} mixes rows with and without UTC offsets; make every row explicit");
        }
        let (fallback_offset, timezone_source, timezone_confidence, timezone_conflict) =
            if all_explicit {
                let source = if saw_timestamp_offset {
                    "timestamp_offset"
                } else if saw_column_offset {
                    "table_offset_column"
                } else {
                    unreachable!()
                };
                (0_i64, source.to_string(), "high", false)
            } else {
                let local_times = records
                    .iter()
                    .map(|(_, parsed, _)| parsed.local_seconds)
                    .collect::<Vec<_>>();
                let swdown = match swdown_index {
                    Some(index) => records
                        .iter()
                        .map(|(row, _, _)| parse_value(row, index, "shortwave radiation"))
                        .collect::<Result<Vec<_>>>()?,
                    None => Vec::new(),
                };
                let decision = decide_timezone_with_solar(
                    plan.manual_utc_offset,
                    None,
                    Some(longitude),
                    &local_times,
                    &swdown,
                )?;
                let source = if plan.manual_utc_offset.is_some() {
                    "manual_override"
                } else {
                    decision.source.as_str()
                };
                (
                    (decision.offset_hours * 3600.0).round() as i64,
                    source.to_string(),
                    decision.confidence.as_str(),
                    decision.conflict,
                )
            };
        let mut by_time = BTreeMap::<i64, &Row>::new();
        for (row, parsed, row_offset) in records {
            let offset = row_offset.unwrap_or(fallback_offset);
            let utc = parsed.local_seconds - offset;
            if let Some(previous) = by_time.insert(utc, row) {
                bail!(
                    "site {site:?} has duplicate timestamp at lines {} and {} after UTC normalization",
                    previous.line,
                    row.line
                );
            }
        }
        if by_time.len() < 2 {
            bail!("site {site:?} needs at least two timestamps");
        }
        let observed_times = by_time.keys().copied().collect::<Vec<_>>();
        let step = match plan.step_seconds {
            Some(value) => validate_step(value)?,
            None => infer_step(&observed_times)
                .with_context(|| format!("cannot infer a uniform time step for site {site:?}"))?,
        };
        for pair in observed_times.windows(2) {
            let difference = pair[1] - pair[0];
            if difference <= 0 || difference % step != 0 {
                bail!(
                    "site {site:?} timestamp difference {difference}s is off the inferred {step}s cadence; correct the timestamp or choose an explicit valid cadence"
                );
            }
        }
        let start = observed_times[0];
        let end = *observed_times.last().unwrap();
        let steps = checked_step_count(start, end, step, by_time.len())?;
        let inserted_steps = steps - by_time.len();
        let staged_path = staging.join(format!("{safe_site}_Met.nc"));
        let final_path = destination.join(format!("{safe_site}_Met.nc"));
        let resolved_offset = explicit_common_offset(&resolved_row_offsets)
            .or(plan.manual_utc_offset)
            .or_else(|| (!all_explicit).then_some(fallback_offset as f64 / 3600.0));
        write_site_file(
            path,
            &table,
            &staged_path,
            &site,
            latitude,
            longitude,
            landtype,
            start,
            step,
            steps,
            &by_time,
            plan,
            &timezone_source,
            resolved_offset,
            timezone_confidence,
            timezone_conflict,
        )?;
        imported.push(ImportedTableSite {
            site,
            safe_site,
            staged_path,
            final_path,
            rows: by_time.len(),
            inserted_steps,
            latitude,
            longitude,
            landtype,
            timezone_offset_hours: resolved_offset,
            timezone_source,
            start_utc: start,
            end_utc: end,
        });
    }
    Ok(imported)
}

fn read_table(path: &Path) -> Result<Table> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read {} as UTF-8 text", path.display()))?;
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
    let lines = raw
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim();
            (!trimmed.is_empty() && !trimmed.starts_with('#')).then_some((index + 1, line))
        })
        .collect::<Vec<_>>();
    let Some((header_line, header)) = lines.first().copied() else {
        bail!("{} has no header row", path.display());
    };
    let delimiter = detect_delimiter(header);
    let header_cells = split_line(header, delimiter)
        .with_context(|| format!("cannot parse header at line {header_line}"))?;
    if header_cells.len() < 2 {
        bail!("{} header has fewer than two columns", path.display());
    }
    let columns = header_cells
        .iter()
        .map(|cell| split_header_units(cell))
        .collect::<Vec<_>>();
    let mut unique = BTreeSet::new();
    for column in &columns {
        if column.name.is_empty() {
            bail!("{} has an empty column name", path.display());
        }
        let key = normalize(&column.name);
        if !unique.insert(key) {
            bail!("{} has duplicate column {:?}", path.display(), column.name);
        }
    }
    let mut rows = Vec::new();
    for (line_number, line) in lines.into_iter().skip(1) {
        let cells = split_line(line, delimiter)
            .with_context(|| format!("cannot parse line {line_number}"))?;
        if cells.len() != columns.len() {
            bail!(
                "line {line_number} has {} fields but the header has {}",
                cells.len(),
                columns.len()
            );
        }
        rows.push(Row {
            line: line_number,
            cells,
        });
    }
    Ok(Table {
        delimiter,
        columns,
        rows,
    })
}

fn detect_delimiter(line: &str) -> Delimiter {
    let counts = [
        (Delimiter::Comma, count_unquoted(line, ',')),
        (Delimiter::Tab, count_unquoted(line, '\t')),
        (Delimiter::Semicolon, count_unquoted(line, ';')),
    ];
    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .filter(|(_, count)| *count > 0)
        .map(|(delimiter, _)| delimiter)
        .unwrap_or(Delimiter::Whitespace)
}

fn count_unquoted(line: &str, wanted: char) -> usize {
    let mut quoted = false;
    let mut count = 0;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            if quoted && chars.peek() == Some(&'"') {
                chars.next();
            } else {
                quoted = !quoted;
            }
        } else if !quoted && ch == wanted {
            count += 1;
        }
    }
    count
}

fn split_line(line: &str, delimiter: Delimiter) -> Result<Vec<String>> {
    let separator = delimiter.character();
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            if quoted && chars.peek() == Some(&'"') {
                chars.next();
                cell.push('"');
            } else {
                quoted = !quoted;
            }
            continue;
        }
        let boundary = if let Some(separator) = separator {
            !quoted && ch == separator
        } else {
            !quoted && ch.is_whitespace()
        };
        if boundary {
            if separator.is_some() || !cell.is_empty() {
                cells.push(cell.trim().to_string());
                cell.clear();
            }
            continue;
        }
        cell.push(ch);
    }
    if quoted {
        bail!("unterminated quoted field");
    }
    if separator.is_some() || !cell.is_empty() {
        cells.push(cell.trim().to_string());
    }
    Ok(cells)
}

fn split_header_units(cell: &str) -> TabularColumn {
    let text = cell.trim();
    for (open, close) in [('[', ']'), ('(', ')')] {
        if text.ends_with(close) {
            if let Some(index) = text.rfind(open) {
                let name = text[..index].trim();
                let units = text[index + 1..text.len() - 1].trim();
                if !name.is_empty() && !units.is_empty() {
                    return TabularColumn {
                        name: name.to_string(),
                        units: Some(units.to_string()),
                    };
                }
            }
        }
    }
    TabularColumn {
        name: text.to_string(),
        units: None,
    }
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn guess_column(columns: &[String], aliases: &[&str]) -> Option<String> {
    aliases.iter().find_map(|alias| {
        let wanted = normalize(alias);
        columns
            .iter()
            .find(|column| normalize(column) == wanted)
            .cloned()
    })
}

fn inferred_units(name: &str) -> Option<String> {
    let n = normalize(name);
    let unit = match n.as_str() {
        "taf" | "ta" => "degC",
        "rhf" | "rh" => "%",
        "vpdf" | "vpd" => "hPa",
        "paf" | "pa" => "kPa",
        "pf" | "p" => "mm",
        "wsf" | "ws" => "m s-1",
        "swinf" | "swin" | "lwinf" | "lwin" => "W m-2",
        _ => return None,
    };
    Some(unit.to_string())
}

fn slot_aliases(index: usize) -> &'static [&'static str] {
    match index {
        1 => &["Tair", "TA_F", "TA", "air_temperature", "temperature"],
        2 => &[
            "Qair",
            "QA_F",
            "QA",
            "VPD_F",
            "VPD",
            "RH_F",
            "RH",
            "specific_humidity",
            "humidity",
        ],
        3 => &[
            "Psurf",
            "PSurf",
            "PA_F",
            "PA",
            "surface_pressure",
            "pressure",
        ],
        4 => &["Precip", "Rainf", "P_F", "P", "precipitation", "rainfall"],
        5 => &["Wind_E", "U", "U10", "eastward_wind", "wind_east"],
        6 => &[
            "Wind_N",
            "Wind",
            "WS_F",
            "WS",
            "wind_speed",
            "northward_wind",
        ],
        7 => &["SWdown", "SW_IN_F", "SW_IN", "shortwave", "solar_radiation"],
        8 => &[
            "LWdown",
            "LW_IN_F",
            "LW_IN",
            "longwave",
            "thermal_radiation",
        ],
        _ => &[],
    }
}

struct ProbeSiteColumns<'a> {
    site: Option<&'a str>,
    time: Option<&'a str>,
    utc_offset: Option<&'a str>,
    latitude: Option<&'a str>,
    longitude: Option<&'a str>,
    landtype: Option<&'a str>,
}

fn fallback_site_name(path: &Path) -> Result<String> {
    let stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .context("tabular forcing filename has no usable site name")?;
    Ok(stem
        .strip_prefix("FLX_")
        .and_then(|name| name.split_once("_FLUXNET-CH4").map(|(site, _)| site))
        .filter(|site| !site.is_empty())
        .unwrap_or(stem)
        .to_string())
}

fn probe_sites(
    path: &Path,
    table: &Table,
    columns: ProbeSiteColumns<'_>,
) -> Result<Vec<TabularSiteProbe>> {
    let fallback = fallback_site_name(path)?;
    let site_index = columns
        .site
        .map(|name| column_index(table, name))
        .transpose()?;
    let time_index = columns
        .time
        .map(|name| column_index(table, name))
        .transpose()?;
    let offset_index = columns
        .utc_offset
        .map(|name| column_index(table, name))
        .transpose()?;
    let lat_index = columns
        .latitude
        .map(|name| column_index(table, name))
        .transpose()?;
    let lon_index = columns
        .longitude
        .map(|name| column_index(table, name))
        .transpose()?;
    let landtype_index = columns
        .landtype
        .map(|name| column_index(table, name))
        .transpose()?;
    let mut groups = BTreeMap::<String, Vec<&Row>>::new();
    for row in &table.rows {
        let id = match site_index {
            Some(index) => {
                let value = row.cells[index].trim();
                if value.is_empty() {
                    bail!("line {} has an empty site identifier", row.line);
                }
                value
            }
            None => &fallback,
        };
        groups.entry(id.to_string()).or_default().push(row);
    }
    groups
        .into_iter()
        .map(|(id, rows)| {
            let times = if let Some(index) = time_index {
                rows.iter()
                    .map(|row| {
                        let stamp = parse_timestamp(&row.cells[index])?;
                        let column_offset = offset_index
                            .map(|offset| parse_offset_cell(row, offset))
                            .transpose()?
                            .flatten();
                        if let (Some(timestamp), Some(column)) =
                            (stamp.offset_seconds, column_offset)
                        {
                            if timestamp != column {
                                bail!(
                                    "line {} timestamp UTC offset disagrees with the UTC offset column",
                                    row.line
                                );
                            }
                        }
                        let offset = stamp.offset_seconds.or(column_offset);
                        Ok((stamp.local_seconds, offset, row.cells[index].clone()))
                    })
                    .collect::<Result<Vec<_>>>()?
            } else {
                Vec::new()
            };
            let has_explicit = times.iter().any(|item| item.1.is_some());
            if has_explicit && !times.iter().all(|item| item.1.is_some()) {
                bail!("site {id:?} mixes rows with and without UTC offsets; make every row explicit");
            }
            let mut normalized = times
                .iter()
                .map(|(local, offset, raw)| (local - offset.unwrap_or(0), raw.clone()))
                .collect::<Vec<_>>();
            normalized.sort_unstable_by_key(|item| item.0);
            let seconds = normalized.iter().map(|item| item.0).collect::<Vec<_>>();
            let step_seconds = infer_step(&seconds).ok();
            let inserted_steps = step_seconds
                .filter(|_| normalized.len() >= 2)
                .and_then(|step| {
                    checked_step_count(seconds[0], seconds[seconds.len() - 1], step, seconds.len())
                        .ok()
                })
                .map(|steps| steps.saturating_sub(normalized.len()))
                .unwrap_or(0);
            Ok(TabularSiteProbe {
                id,
                rows: rows.len(),
                latitude: consistent_number(&rows, lat_index, "latitude")?,
                longitude: consistent_number(&rows, lon_index, "longitude")?,
                landtype: consistent_integer(&rows, landtype_index, "landtype")?,
                start: normalized.first().map(|item| item.1.clone()),
                end: normalized.last().map(|item| item.1.clone()),
                step_seconds,
                inserted_steps,
            })
        })
        .collect()
}

fn validate_plan(table: &Table, plan: &TabularPlan) -> Result<()> {
    column_index(table, &plan.time_column)?;
    let mut indexes = BTreeSet::new();
    for slot in &plan.slots {
        if !indexes.insert(slot.index) {
            bail!("forcing slot {} is configured more than once", slot.index);
        }
        if SLOTS.iter().all(|candidate| candidate.index != slot.index) {
            bail!("unknown CoLM forcing slot {}", slot.index);
        }
        column_index(table, &slot.column)?;
        let mut additions = BTreeSet::new();
        for extra in &slot.also_add {
            let extra = extra.trim();
            if extra.is_empty() {
                bail!("slot {} has an empty also_add column", slot.index);
            }
            column_index(table, extra)?;
            if extra == slot.column.trim() {
                bail!("slot {} uses column {:?} twice", slot.index, slot.column);
            }
            if !additions.insert(extra) {
                bail!(
                    "slot {} names also_add column {:?} more than once",
                    slot.index,
                    extra
                );
            }
        }
        if slot.source_units.trim().is_empty() {
            bail!("slot {} has no source units", slot.index);
        }
    }
    for required in [1, 2, 3, 4, 6, 7, 8] {
        if !indexes.contains(&required) {
            bail!("required CoLM forcing slot {required} is not mapped");
        }
    }
    Ok(())
}

fn column_index(table: &Table, name: &str) -> Result<usize> {
    table
        .columns
        .iter()
        .position(|column| column.name == name)
        .with_context(|| format!("table has no column {name:?}"))
}

fn consistent_number(rows: &[&Row], index: Option<usize>, label: &str) -> Result<Option<f64>> {
    let Some(index) = index else { return Ok(None) };
    let mut answer: Option<f64> = None;
    for row in rows {
        let raw = row.cells[index].trim();
        if is_missing(raw) {
            continue;
        }
        let value: f64 = raw
            .parse()
            .with_context(|| format!("line {} {label} {raw:?} is not a number", row.line))?;
        if !value.is_finite() {
            bail!("line {} {label} is not finite", row.line);
        }
        if answer.is_some_and(|previous| (previous - value).abs() > 1e-8) {
            bail!("{label} changes within one site ({answer:?} versus {value})");
        }
        answer = Some(value);
    }
    Ok(answer)
}

fn consistent_integer(rows: &[&Row], index: Option<usize>, label: &str) -> Result<Option<i32>> {
    let value = consistent_number(rows, index, label)?;
    match value {
        Some(value) if value.fract() != 0.0 => bail!("{label} {value} is not an integer"),
        Some(value) => Ok(Some(value as i32)),
        None => Ok(None),
    }
}

fn parse_value(row: &Row, index: usize, label: &str) -> Result<f64> {
    let raw = row.cells[index].trim();
    if is_missing(raw) {
        return Ok(f64::NAN);
    }
    raw.parse::<f64>()
        .with_context(|| format!("line {} {label} value {raw:?} is not numeric", row.line))
}

fn parse_offset_cell(row: &Row, index: usize) -> Result<Option<i64>> {
    let raw = row.cells[index].trim();
    if is_missing(raw) {
        return Ok(None);
    }
    let hours = raw
        .parse::<f64>()
        .with_context(|| format!("line {} UTC offset {raw:?} is not numeric", row.line))?;
    if !hours.is_finite() || !(-12.0..=14.0).contains(&hours) {
        bail!(
            "line {} UTC offset {hours} is outside -12..=14 hours",
            row.line
        );
    }
    let minutes = (hours * 60.0).round() as i64;
    if ((minutes as f64 / 60.0) - hours).abs() > 1e-9 {
        bail!(
            "line {} UTC offset {hours} is not representable in whole minutes",
            row.line
        );
    }
    Ok(Some(minutes * 60))
}

fn is_missing(raw: &str) -> bool {
    let normalized = raw.trim().to_ascii_lowercase();
    normalized.is_empty()
        || matches!(
            normalized.as_str(),
            "na" | "n/a" | "nan" | "null" | "missing"
        )
        || normalized == "-9999"
        || normalized == "-9999.0"
}

#[derive(Debug, Clone, Copy)]
struct ParsedTimestamp {
    local_seconds: i64,
    offset_seconds: Option<i64>,
}

fn parse_timestamp(raw: &str) -> Result<ParsedTimestamp> {
    let mut text = raw.trim().to_string();
    if text.is_empty() {
        bail!("timestamp is empty");
    }
    let mut offset_seconds = None;
    if text.ends_with(['Z', 'z']) {
        text.pop();
        offset_seconds = Some(0);
    } else if text.len() >= 6 {
        let bytes = text.as_bytes();
        let start = bytes.len() - 6;
        if matches!(bytes[start], b'+' | b'-') && bytes[start + 3] == b':' {
            let suffix = &text[start..];
            offset_seconds = Some(parse_offset(suffix)?);
            text.truncate(start);
        }
    }
    if offset_seconds.is_none() && text.len() >= 5 {
        let bytes = text.as_bytes();
        let start = bytes.len() - 5;
        if matches!(bytes[start], b'+' | b'-')
            && bytes[start + 1..].iter().all(|byte| byte.is_ascii_digit())
        {
            let suffix = &text[start..];
            offset_seconds = Some(parse_offset(suffix)?);
            text.truncate(start);
        }
    }
    let digits = text.trim();
    let (year, month, day, hour, minute, second): (i32, u32, u32, u32, u32, u32) =
        if (digits.len() == 12 || digits.len() == 14)
            && digits.bytes().all(|byte| byte.is_ascii_digit())
        {
            (
                digits[0..4].parse()?,
                digits[4..6].parse()?,
                digits[6..8].parse()?,
                digits[8..10].parse()?,
                digits[10..12].parse()?,
                if digits.len() == 14 {
                    digits[12..14].parse()?
                } else {
                    0
                },
            )
        } else {
            let normalized = text.trim().replace('T', " ").replace('/', "-");
            let mut parts = normalized.split_whitespace();
            let date = parts.next().context("timestamp has no date")?;
            let clock = parts.next().unwrap_or("00:00:00");
            if parts.next().is_some() {
                bail!("timestamp has unexpected trailing text");
            }
            let mut date_parts = date.split('-');
            let year = date_parts
                .next()
                .context("timestamp has no year")?
                .parse()?;
            let month = date_parts
                .next()
                .context("timestamp has no month")?
                .parse()?;
            let day = date_parts.next().context("timestamp has no day")?.parse()?;
            if date_parts.next().is_some() {
                bail!("timestamp date has too many components");
            }
            let mut clock_parts = clock.split(':');
            let hour = clock_parts.next().unwrap_or("0").parse()?;
            let minute = clock_parts.next().unwrap_or("0").parse()?;
            let second = parse_second(clock_parts.next().unwrap_or("0"))?;
            if clock_parts.next().is_some() {
                bail!("timestamp clock has too many components");
            }
            (year, month, day, hour, minute, second)
        };
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        bail!("timestamp components are outside their valid ranges");
    }
    let days = crate::civil::days_from_civil(year, month, day);
    if crate::civil::civil_from_days(days) != (year, month, day) {
        bail!("timestamp date does not exist");
    }
    Ok(ParsedTimestamp {
        local_seconds: days * 86400 + hour as i64 * 3600 + minute as i64 * 60 + second as i64,
        offset_seconds,
    })
}

fn parse_second(raw: &str) -> Result<u32> {
    let (whole, fraction) = raw.split_once('.').unwrap_or((raw, ""));
    if raw.ends_with('.')
        || (!fraction.is_empty()
            && (!fraction.bytes().all(|byte| byte.is_ascii_digit())
                || fraction.bytes().any(|byte| byte != b'0')))
    {
        bail!("timestamp fractional seconds must be zero; sub-second forcing is not supported");
    }
    whole.parse().context("timestamp seconds are not numeric")
}

fn parse_offset(raw: &str) -> Result<i64> {
    let sign = if raw.starts_with('-') { -1 } else { 1 };
    let digits = raw[1..].replace(':', "");
    if digits.len() != 4 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("invalid UTC offset {raw:?}");
    }
    let hours: i64 = digits[..2].parse()?;
    let minutes: i64 = digits[2..].parse()?;
    if hours > 14 || minutes > 59 || (hours == 14 && minutes != 0) {
        bail!("UTC offset {raw:?} is outside -12..=14 hours");
    }
    let seconds = sign * (hours * 3600 + minutes * 60);
    if !(-12 * 3600..=14 * 3600).contains(&seconds) {
        bail!("UTC offset {raw:?} is outside -12..=14 hours");
    }
    Ok(seconds)
}

fn infer_step(times: &[i64]) -> Result<i64> {
    if times.len() < 2 {
        bail!("at least two timestamps are required");
    }
    let differences = times
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect::<Vec<_>>();
    for difference in &differences {
        if *difference <= 0 {
            bail!("timestamps are not strictly increasing");
        }
    }

    // Common station cadences, longest first. A one-second typo in an otherwise
    // half-hourly file must not turn a decade into hundreds of millions of rows.
    const CADENCES: &[i64] = &[
        86_400, 43_200, 21_600, 10_800, 7_200, 3_600, 1_800, 1_200, 900, 600, 300, 60,
    ];
    if let Some(step) = CADENCES.iter().copied().find(|candidate| {
        differences.iter().all(|difference| {
            let remainder = difference.rem_euclid(*candidate);
            remainder <= 1 || candidate - remainder <= 1
        })
    }) {
        return validate_step(step);
    }

    let step = differences.iter().copied().fold(0_i64, gcd);
    if step < 60 {
        bail!(
            "timestamp differences imply an implausible {step}s cadence; correct timestamp jitter or specify the intended time step"
        );
    }
    validate_step(step)
}

fn validate_step(step: i64) -> Result<i64> {
    if let Some(problem) = cadence_problem(2, step as f64) {
        bail!(problem);
    }
    Ok(step)
}

fn gcd(mut a: i64, mut b: i64) -> i64 {
    while b != 0 {
        let remainder = a % b;
        a = b;
        b = remainder;
    }
    a.abs()
}

pub(crate) fn checked_step_count(start: i64, end: i64, step: i64, rows: usize) -> Result<usize> {
    if step <= 0 || end < start {
        bail!("invalid time range or non-positive step");
    }
    let span = end
        .checked_sub(start)
        .context("time range overflows while counting table rows")?;
    let raw = span
        .checked_div(step)
        .and_then(|value| value.checked_add(1))
        .context("time range overflows while counting table rows")?;
    let steps = usize::try_from(raw).context("table time axis does not fit in memory")?;
    if steps > MAX_SITE_STEPS {
        bail!(
            "table expansion would create {steps} time steps from {rows} rows; the safety limit is {MAX_SITE_STEPS}. Correct the cadence or split the input"
        );
    }
    Ok(steps)
}

fn safe_site_name(site: &str) -> Result<String> {
    let mut output = String::new();
    let mut separator = false;
    for ch in site.trim().chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            output.push(ch);
            separator = false;
        } else if !separator && !output.is_empty() {
            output.push('-');
            separator = true;
        }
    }
    while output.ends_with('-') {
        output.pop();
    }
    if output.is_empty() {
        bail!("site name {site:?} has no filename-safe characters");
    }
    Ok(output)
}

#[allow(clippy::too_many_arguments)]
fn write_site_file(
    source: &Path,
    table: &Table,
    destination: &Path,
    site: &str,
    latitude: f64,
    longitude: f64,
    landtype: Option<i32>,
    start: i64,
    step: i64,
    steps: usize,
    rows: &BTreeMap<i64, &Row>,
    plan: &TabularPlan,
    timezone_source: &str,
    timezone_offset_hours: Option<f64>,
    timezone_confidence: &str,
    timezone_conflict: bool,
) -> Result<()> {
    let parent = destination
        .parent()
        .context("tabular staging path has no parent")?;
    std::fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        ".{}.{}.tmp",
        destination
            .file_name()
            .unwrap_or_default()
            .to_string_lossy(),
        std::process::id()
    ));
    let _ = std::fs::remove_file(&tmp);
    let result = (|| -> Result<()> {
        let mut file =
            netcdf::create(&tmp).with_context(|| format!("cannot create {}", tmp.display()))?;
        file.add_attribute("source_format", "tabular CSV/TXT")?;
        file.add_attribute("source_path", source.display().to_string().as_str())?;
        file.add_attribute("site_id", site)?;
        file.add_attribute("time_shown_in", "UTC")?;
        file.add_attribute("tabular_timezone_source", timezone_source)?;
        file.add_attribute("tabular_timezone_confidence", timezone_confidence)?;
        file.add_attribute("tabular_timezone_conflict", i32::from(timezone_conflict))?;
        if let Some(offset) = timezone_offset_hours {
            file.add_attribute("tabular_source_utc_offset_hours", offset)?;
        }
        file.add_dimension("time", steps)?;
        let mut time = file.add_variable::<f64>("time", &["time"])?;
        time.put_attribute("units", "seconds since 1970-01-01 00:00:00")?;
        time.put_attribute("calendar", "gregorian")?;
        let values = (0..steps)
            .map(|index| (start + index as i64 * step) as f64)
            .collect::<Vec<_>>();
        time.put_values(&values, ..)?;
        drop(time);

        scalar(
            &mut file,
            "latitude",
            latitude,
            "degrees_north",
            "from tabular site metadata",
        )?;
        scalar(
            &mut file,
            "longitude",
            longitude,
            "degrees_east",
            "from tabular site metadata",
        )?;
        if let Some(landtype) = landtype {
            let mut variable = file.add_variable::<i32>("landtype", &[])?;
            variable.put_attribute("source", "from tabular site metadata")?;
            variable.put_value(landtype, ())?;
        }
        if let Some(heights) = plan.heights {
            scalar(
                &mut file,
                "reference_height_v",
                heights.v,
                "m",
                "given in tabular import",
            )?;
            scalar(
                &mut file,
                "reference_height_t",
                heights.t,
                "m",
                "given in tabular import",
            )?;
            scalar(
                &mut file,
                "reference_height_q",
                heights.q,
                "m",
                "given in tabular import",
            )?;
        }

        for slot in &plan.slots {
            let definition = SLOTS
                .iter()
                .find(|candidate| candidate.index == slot.index)
                .context("validated forcing slot disappeared")?;
            let canonical = definition.candidates[0];
            let primary_index = column_index(table, &slot.column)?;
            let extra_indexes = slot
                .also_add
                .iter()
                .map(|name| column_index(table, name))
                .collect::<Result<Vec<_>>>()?;
            let mut raw = Vec::with_capacity(steps);
            for index in 0..steps {
                let timestamp = start + index as i64 * step;
                let Some(row) = rows.get(&timestamp) else {
                    raw.push(f64::NAN);
                    continue;
                };
                raw.push(parse_value(row, primary_index, &slot.column)?);
            }
            let units = canonical_units(slot.index);
            let mut converted = if slot.index == 2 {
                let temperature = tabular_support_values(table, rows, start, step, steps, plan, 1)?;
                let pressure = tabular_support_values(table, rows, start, step, steps, plan, 3)?;
                match crate::units::humidity_to_specific(
                    &slot.column,
                    &slot.source_units,
                    &raw,
                    &temperature,
                    &pressure,
                )? {
                    Some(values) => values,
                    None => crate::units::convert_units_with_step(
                        &slot.source_units,
                        units,
                        &raw,
                        Some(step as f64),
                    )?,
                }
            } else {
                crate::units::convert_units_with_step(
                    &slot.source_units,
                    units,
                    &raw,
                    Some(step as f64),
                )?
            };
            for (extra_name, extra_index) in slot.also_add.iter().zip(&extra_indexes) {
                let source_units = table.columns[*extra_index]
                    .units
                    .as_deref()
                    .with_context(|| {
                        format!(
                            "extra column {extra_name:?} has no unit in its header; cannot safely add it to {:?}",
                            slot.column
                        )
                    })?;
                let raw_extra = (0..steps)
                    .map(|index| {
                        let timestamp = start + index as i64 * step;
                        rows.get(&timestamp).map_or(Ok(f64::NAN), |row| {
                            parse_value(row, *extra_index, extra_name)
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                let extra = crate::units::convert_units_with_step(
                    source_units,
                    units,
                    &raw_extra,
                    Some(step as f64),
                )?;
                for (value, extra) in converted.iter_mut().zip(extra) {
                    if value.is_finite() && extra.is_finite() {
                        *value += extra;
                    } else {
                        *value = f64::NAN;
                    }
                }
            }
            let fill = -9999.0_f64;
            let values = converted
                .into_iter()
                .map(|value| if value.is_finite() { value } else { fill })
                .collect::<Vec<_>>();
            let mut variable = file.add_variable::<f64>(canonical, &["time"])?;
            variable.set_fill_value(fill)?;
            variable.put_attribute("units", units)?;
            let source_note = if slot.also_add.is_empty() {
                format!("tabular column {:?} ({})", slot.column, slot.source_units)
            } else {
                format!(
                    "sum of tabular columns {:?} and {:?} ({})",
                    slot.column, slot.also_add, slot.source_units
                )
            };
            variable.put_attribute("source", source_note.as_str())?;
            variable.put_values(&values, ..)?;
        }
        drop(file);
        Ok(())
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    crate::gapfill::install_repaired_file(&tmp, destination)?;
    Ok(())
}

fn tabular_support_values(
    table: &Table,
    rows: &BTreeMap<i64, &Row>,
    start: i64,
    step: i64,
    steps: usize,
    plan: &TabularPlan,
    index: usize,
) -> Result<Vec<f64>> {
    let slot = plan
        .slots
        .iter()
        .find(|slot| slot.index == index)
        .with_context(|| format!("humidity derivation also needs forcing slot {index}"))?;
    let column = column_index(table, &slot.column)?;
    let raw = (0..steps)
        .map(|position| {
            let timestamp = start + position as i64 * step;
            rows.get(&timestamp)
                .map_or(Ok(f64::NAN), |row| parse_value(row, column, &slot.column))
        })
        .collect::<Result<Vec<_>>>()?;
    crate::units::convert_units_with_step(
        &slot.source_units,
        canonical_units(index),
        &raw,
        Some(step as f64),
    )
}

fn scalar(
    file: &mut netcdf::FileMut,
    name: &str,
    value: f64,
    units: &str,
    source: &str,
) -> Result<()> {
    let mut variable = file.add_variable::<f64>(name, &[])?;
    variable.put_attribute("units", units)?;
    variable.put_attribute("source", source)?;
    variable.put_value(value, ())?;
    Ok(())
}

fn explicit_common_offset(offsets: &[Option<i64>]) -> Option<f64> {
    let first = offsets.first().copied().flatten()?;
    offsets
        .iter()
        .all(|offset| *offset == Some(first))
        .then_some(first as f64 / 3600.0)
}

#[cfg(test)]
#[path = "tabular_tests.rs"]
mod tabular_tests;

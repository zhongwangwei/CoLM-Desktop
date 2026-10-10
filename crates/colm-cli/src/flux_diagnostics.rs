//! Bounded, read-only CoLM/PLUMBER2 diagnostics. Conditions are explicit, never inferred.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub obs: String,
    variable: String,
    observation_variable: String,
    observation_qc: Option<String>,
    from: i64,
    to: i64,
    model_units: String,
    obs_units: String,
    output_units: String,
    model_scale: f64,
    obs_scale: f64,
    sign_convention: String,
    model_utc_offset_seconds: i64,
    observation_utc_offset_seconds: i64,
    experiment: Option<Experiment>,
    alignment: String,
    model_interval_seconds: i64,
    observation_interval_seconds: i64,
    daylight: Option<Daylight>,
    wetness: Option<Wetness>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Experiment {
    case: String,
    field: String,
    expected_direction: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Daylight {
    variable: String,
    qc_variable: String,
    units: String,
    threshold: f64,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Wetness {
    variable: String,
    qc_variable: String,
    units: String,
    window_hours: u32,
    threshold_mm: f64,
}

fn check_variable(file: &netcdf::File, name: &str, units: Option<&str>, n: usize) -> Result<()> {
    let var = file
        .variable(name)
        .with_context(|| format!("missing variable {name}"))?;
    ensure!(
        var.len() == n && var.dimensions().first().is_some_and(|d| d.name() == "time"),
        "{name}: single-point time series required"
    );
    for (attribute, identity) in [("scale_factor", 1.0), ("add_offset", 0.0)] {
        if let Some(value) = var.attribute_value(attribute) {
            let values = super::numeric_attribute_values(value?);
            ensure!(values.len()==1 && values[0]==identity, "{name}: nonidentity or malformed {attribute} packing is unsupported; unpack the NetCDF before diagnosis");
        }
    }
    if name == "time" {
        if let Some(calendar) = var.attribute_value("calendar") {
            ensure!(
                matches!(calendar?,netcdf::AttributeValue::Str(ref s) if ["standard","gregorian","proleptic_gregorian"].contains(&s.as_str())),
                "unsupported calendar"
            );
        }
    }
    if let Some(expected) = units {
        ensure!(
            super::variable_units(file, name).as_deref() == Some(expected),
            "{name}: file units do not match explicit units {expected:?}"
        );
    }
    Ok(())
}
fn regular(times: &[f64], interval: i64) -> Result<()> {
    ensure!(
        times.len() >= 2 && times.iter().all(|t| t.is_finite()),
        "at least two finite timestamps required"
    );
    ensure!(
        times
            .windows(2)
            .all(|w| (w[1] - w[0] - interval as f64).abs() < 0.01),
        "timestamps are duplicated, missing, unordered or inconsistent with the declared interval"
    );
    Ok(())
}
fn summary(pairs: &[(f64, f64)]) -> Value {
    if pairs.len() < 3 {
        return json!({"status":"missing","n":pairs.len(),"reason":"at least three finite QC-approved pairs required"});
    }
    let Some(m) = colm_hist::metric::compute(pairs) else {
        return json!({"status":"missing","n":pairs.len()});
    };
    if ![m.bias, m.rmse, m.mae, m.model_mean, m.obs_mean]
        .iter()
        .all(|v| v.is_finite())
    {
        return json!({"status":"missing","n":pairs.len(),"reason":"metric overflow; verify the declared conversion scales"});
    }
    json!({"status":"computed","n":m.n,"bias":m.bias,"rmse":m.rmse,"mae":m.mae,"correlation":m.correlation,"model_mean":m.model_mean,"obs_mean":m.obs_mean})
}
fn month(seconds: i64) -> usize {
    let mut days = seconds.div_euclid(86400);
    let mut year = 1970;
    loop {
        let n = if colm_hist::time::is_leap(year) {
            366
        } else {
            365
        };
        if days < n {
            break;
        }
        days -= n;
        year += 1;
    }
    let lengths = [
        31,
        if colm_hist::time::is_leap(year) {
            29
        } else {
            28
        },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    for (index, length) in lengths.iter().enumerate() {
        if days < *length {
            return index + 1;
        }
        days -= length;
    }
    unreachable!()
}

pub(super) fn run(case: &Path, r: &Request) -> Result<Value> {
    ensure!(
        r.from >= 0 && r.to <= 4_102_444_800 && r.to > r.from && r.to - r.from <= 366 * 86400,
        "window must be 1..366 days within 1970..2100 (Unix seconds)"
    );
    ensure!(
        r.alignment == "colm_plumber2_fixed_offset"
            && (-50400..=50400).contains(&r.model_utc_offset_seconds)
            && (-50400..=50400).contains(&r.observation_utc_offset_seconds)
            && r.model_utc_offset_seconds % 60 == 0
            && r.observation_utc_offset_seconds % 60 == 0,
        "explicit whole-minute CoLM/PLUMBER2 fixed offsets required; DST is unsupported"
    );
    ensure!(
        [1800, 3600].contains(&r.model_interval_seconds) && r.observation_interval_seconds == 1800,
        "supported intervals: CoLM 1800/3600 seconds, PLUMBER2 1800 seconds"
    );
    ensure!(
        !r.sign_convention.trim().is_empty()
            && !r.output_units.trim().is_empty()
            && r.model_scale.is_finite()
            && r.obs_scale.is_finite()
            && r.model_scale != 0.0
            && r.obs_scale != 0.0,
        "explicit sign convention, common units and finite nonzero conversion scales required"
    );
    let layout = colm_case::Layout::new(case);
    let config = colm_namelist::parse(&std::fs::read_to_string(layout.case_nml())?)?;
    let greenwich = config
        .get("DEF_simulation_time%greenwich")
        .map(ToString::to_string)
        .context("case greenwich time convention missing")?;
    ensure!(
        !greenwich.eq_ignore_ascii_case(".true.") || r.model_utc_offset_seconds == 0,
        "Greenwich model history requires zero UTC offset"
    );
    let name = colm_case::case_name(&layout.case_nml())?;
    let files = super::primary_history_files(&super::history_files(&layout.out().join(name))?);
    ensure!(
        files.len() <= 120,
        "diagnosis limited to 120 history files; subset the inputs first"
    );
    let mut total = 0;
    for path in &files {
        let f = netcdf::open(path)?;
        let n = f.variable("time").context("history time missing")?.len();
        total += n;
        ensure!(
            total <= 200_000,
            "diagnosis limited to 200000 model records; subset first"
        );
        check_variable(&f, "time", Some("minutes since 1900-1-1 0:0:0"), n)?;
        check_variable(&f, &r.variable, Some(&r.model_units), n)?;
    }
    let obs_path = Path::new(&r.obs);
    ensure!(
        std::fs::metadata(obs_path)?.len() <= 128 * 1024 * 1024,
        "observation file exceeds 128 MiB; subset first"
    );
    let obs = netcdf::open(obs_path)?;
    let n = obs
        .variable("time")
        .context("observation time missing")?
        .len();
    ensure!(
        n <= 500_000,
        "diagnosis limited to 500000 observation records; subset first"
    );
    check_variable(&obs, "time", None, n)?;
    check_variable(&obs, &r.observation_variable, Some(&r.obs_units), n)?;
    let load = |name: &str| -> Result<Vec<f64>> {
        check_variable(&obs, name, None, n)?;
        super::read_file_1d(&obs, obs_path, name)
    };
    let mut ot = load("time")?;
    for t in &mut ot {
        *t -= r.observation_utc_offset_seconds as f64;
    }
    regular(&ot, 1800)?;
    let ov = load(&r.observation_variable)?;
    let qc = match &r.observation_qc {
        Some(name) => load(name)?,
        None => vec![0.; n],
    };
    let units = super::variable_units(&obs, "time").context("observation time units missing")?;
    let mut model = super::read_history_many(&files, &["time", &r.variable])?;
    let mut mt = model.remove("time").context("history time missing")?;
    for t in &mut mt {
        *t -= r.model_utc_offset_seconds as f64 / 60.;
    }
    let origin = units
        .split_once("since")
        .context("observation time units missing since")?
        .1
        .trim()
        .replace('T', " ");
    ensure!(
        origin.split_whitespace().count() <= 2,
        "embedded timezone suffix unsupported; provide explicit fixed offset"
    );
    ensure!(
        !origin.ends_with('Z') || r.observation_utc_offset_seconds == 0,
        "Z-labelled observations require zero offset"
    );
    let ms = colm_hist::time::model_seconds_from_units(&mt, &units)
        .context("unsupported observation time origin")?;
    regular(&ms, r.model_interval_seconds)?;
    let unix = colm_hist::time::unix_seconds(&mt);
    let mv = model
        .remove(&r.variable)
        .context("history variable missing")?;
    let window = super::normalized_metric_window(&mt, &ms, Some(r.from), Some(r.to))?;
    let pair = |values: &[f64], quality: &[f64]| {
        colm_hist::pair::pair_with_time_in_window(
            &ms,
            &mv,
            &colm_hist::pair::Series {
                seconds: &ot,
                values,
                qc: quality,
            },
            0,
            window,
        )
    };
    // Pair/QC in the raw domain; converted negative values are not raw missing sentinels.
    let aligned: Vec<_> = pair(&ov, &qc)
        .into_iter()
        .map(|(t, m, o)| (t, m * r.model_scale, o * r.obs_scale))
        .filter(|(_, m, o)| m.is_finite() && o.is_finite())
        .collect();
    let unix_by_time: BTreeMap<_, _> = ms.iter().zip(&unix).map(|(t, u)| (*t as i64, *u)).collect();
    let mut support = Sha256::new();
    for (t, _, o) in &aligned {
        support.update(unix_by_time[&(*t as i64)].to_le_bytes());
        support.update(o.to_bits().to_le_bytes());
    }
    let paired_support_id = format!("{:x}", support.finalize());
    let all: Vec<_> = aligned.iter().map(|(_, m, o)| (*m, *o)).collect();
    let mut groups: BTreeMap<String, Vec<(f64, f64)>> = BTreeMap::new();
    // Include requested months with no usable observations instead of silently losing them.
    for day in r.from.div_euclid(86400)..=(r.to - 1).div_euclid(86400) {
        groups
            .entry(format!("month_{:02}", month(day * 86400)))
            .or_default();
    }
    for (t, m, o) in &aligned {
        let mon = month(unix_by_time[&(*t as i64)]);
        groups
            .entry(format!("month_{mon:02}"))
            .or_default()
            .push((*m, *o));
    }
    let months: BTreeMap<_, _> = groups
        .into_iter()
        .map(|(name, p)| (name, summary(&p)))
        .collect();
    let missing_months: Vec<_> = months
        .iter()
        .filter(|(_, value)| value["status"] != "computed")
        .map(|(name, _)| name.clone())
        .collect();
    let month_status = if missing_months.is_empty() {
        "computed"
    } else if missing_months.len() < months.len() {
        "partial"
    } else {
        "missing"
    };
    let mut regimes = json!({"calendar_month":{"status":month_status,"missing":missing_months,"timezone":"UTC","groups":months},"daylight":{"status":"missing","reason":"explicit observed radiation variable, QC and threshold required"},"wetness":{"status":"missing","reason":"explicit observed precipitation rate, QC, accumulation window and threshold required"}});
    if let Some(d) = &r.daylight {
        let result = (|| -> Result<Value> {
            ensure!(
                d.units == "W m-2" && d.threshold.is_finite() && d.threshold >= 0.,
                "daylight requires W m-2 and a nonnegative threshold"
            );
            check_variable(&obs, &d.variable, Some(&d.units), n)?;
            let values = load(&d.variable)?;
            let quality = load(&d.qc_variable)?;
            let radiation: BTreeMap<_, _> = pair(&values, &quality)
                .into_iter()
                .map(|(t, _, v)| (t as i64, v))
                .collect();
            let mut day = Vec::new();
            let mut night = Vec::new();
            for (t, m, o) in &aligned {
                if let Some(v) = radiation.get(&(*t as i64)) {
                    if *v > d.threshold {
                        day.push((*m, *o));
                    } else {
                        night.push((*m, *o));
                    }
                }
            }
            Ok(
                json!({"status":if day.len()>=3 && night.len()>=3 {"computed"}else{"partial"},"day":summary(&day),"night":summary(&night),"classified":day.len()+night.len(),"unclassified":all.len()-day.len()-night.len()}),
            )
        })();
        regimes["daylight"] =
            result.unwrap_or_else(|e| json!({"status":"missing","reason":e.to_string()}));
    }
    if let Some(w) = &r.wetness {
        let result = (|| -> Result<Value> {
            ensure!(
                w.units == "kg m-2 s-1"
                    && (1..=168).contains(&w.window_hours)
                    && w.threshold_mm.is_finite()
                    && w.threshold_mm >= 0.,
                "wetness requires kg m-2 s-1, 1..168 hours and nonnegative mm threshold"
            );
            check_variable(&obs, &w.variable, Some(&w.units), n)?;
            let values = load(&w.variable)?;
            let quality = load(&w.qc_variable)?;
            let count = w.window_hours as usize * 2;
            let mut wet = Vec::new();
            let mut dry = Vec::new();
            for (t, m, o) in &aligned {
                // Rain labels are interval starts; use only complete intervals before this label.
                let end = ot.partition_point(|v| *v < *t);
                if end < count || (ot[end - 1] + 1800. - *t).abs() > 0.01 {
                    continue;
                }
                let range = end - count..end;
                if range
                    .clone()
                    .any(|i| quality[i] != 0. || !values[i].is_finite() || values[i] < 0.)
                {
                    continue;
                }
                let mm = range.map(|i| values[i] * 1800.).sum::<f64>();
                if mm > w.threshold_mm {
                    wet.push((*m, *o));
                } else {
                    dry.push((*m, *o));
                }
            }
            Ok(
                json!({"status":if wet.len()>=3 && dry.len()>=3 {"computed"}else{"partial"},"wet":summary(&wet),"dry":summary(&dry),"classified":wet.len()+dry.len(),"unclassified":all.len()-wet.len()-dry.len(),"window_hours":w.window_hours,"threshold_mm":w.threshold_mm}),
            )
        })();
        regimes["wetness"] =
            result.unwrap_or_else(|e| json!({"status":"missing","reason":e.to_string()}));
    }
    let mut hash = Sha256::new();
    let mut contract = r.clone();
    contract.experiment = None;
    hash.update(serde_json::to_vec(&contract)?);
    hash.update(super::fingerprint::sha256_file(obs_path)?.as_bytes());
    hash.update(std::fs::read(layout.case_nml())?);
    for values in [&ms, &mv, &ot, &ov, &qc] {
        for value in values {
            hash.update(value.to_bits().to_le_bytes());
        }
    }
    let evidence_id = format!("{:x}", hash.finalize());
    let mut result = json!({"version":1,"status":if all.len()>=3 {"evidence_collected"}else{"insufficient_data"},"evidence_id":evidence_id,"paired_support_id":paired_support_id,"case":case,"observation":obs_path,"variable":r.variable,"observation_variable":r.observation_variable,"window":{"from":r.from,"to":r.to,"end_exclusive":true},"alignment":{"status":if all.len()>=3 {"computed"}else{"missing"},"paired":all.len(),"model_records_excluded":unix.iter().filter(|t|**t>=r.from && **t<r.to).count().saturating_sub(all.len()),"model_records_in_window":unix.iter().filter(|t|**t>=r.from && **t<r.to).count(),"quality_control":if r.observation_qc.is_some(){"qc_zero_only"}else{"finite_only_no_qc_supplied"},"units":r.output_units,"model_scale":r.model_scale,"obs_scale":r.obs_scale,"sign_convention":r.sign_convention,"contract":"explicit fixed-offset UTC conversion; half-hour same label, hourly midpoint covers two observation start labels"},"metrics":summary(&all),"regimes":regimes,"hypotheses":[{"status":"unverified","candidate":"Forcing, water availability or process parameter mismatch","basis":{"evidence_id":evidence_id,"metric":"bias","value":summary(&all)["bias"]},"limitation":"Residual grouping suggests tests; it does not identify a causal mechanism."}],"minimal_experiment":{"status":"proposed_not_executed","baseline_evidence_id":evidence_id,"window":{"from":r.from,"to":r.to},"proposal":"Choose one active process parameter using source/configuration evidence; preserve baseline and change only that parameter over this window.","required_before_execution":["physical hypothesis and active parameter/source reference","explicit perturbation value and bounds","same forcing, observation, initial state, build identity and window","approved isolated case/run"],"required_before_causal_claim":["verified baseline and experiment artifact identity","configuration diff restricted to the proposed parameter","same aligned paired timestamps and QC","held-out/closure checks and competing explanations"],"cause_verified":false}});
    if let Some(experiment) = &r.experiment {
        result["experiment"] = compare_experiment(case, r, experiment, &result).unwrap_or_else(
            |e| json!({"status":"missing","cause_verified":false,"reason":e.to_string()}),
        );
    }
    Ok(result)
}

fn compare_experiment(case: &Path, r: &Request, e: &Experiment, baseline: &Value) -> Result<Value> {
    ensure!(
        ["bias_increase", "bias_decrease", "rmse_decrease"]
            .contains(&e.expected_direction.as_str()),
        "unsupported expected direction"
    );
    let other = Path::new(&e.case);
    ensure!(
        std::fs::canonicalize(case)? != std::fs::canonicalize(other)?,
        "experiment must be a distinct case"
    );
    let mut request = r.clone();
    request.experiment = None;
    let comparison = run(other, &request)?;
    let mut differences = Vec::new();
    for file in ["case.nml", "forcing.nml"] {
        let a = colm_namelist::parse(&std::fs::read_to_string(case.join(file))?)?;
        let b = colm_namelist::parse(&std::fs::read_to_string(other.join(file))?)?;
        let mut paths = a.paths();
        paths.extend(b.paths());
        paths.sort();
        paths.dedup();
        for path in paths {
            let av = a.get(&path).map(ToString::to_string);
            let bv = b.get(&path).map(ToString::to_string);
            if av != bv {
                differences.push(json!({"file":file,"field":path,"baseline":av,"experiment":bv}));
            }
        }
    }
    let only_target = differences.len() == 1
        && differences[0]["file"] == "case.nml"
        && differences[0]["field"]
            .as_str()
            .is_some_and(|p| p.eq_ignore_ascii_case(&e.field));
    let difference_count = differences.len();
    differences.truncate(20);
    for difference in &mut differences {
        for field in ["field", "baseline", "experiment"] {
            if let Some(text) = difference[field].as_str() {
                difference[field] = json!(text.chars().take(256).collect::<String>());
            }
        }
    }
    let a = super::fingerprint::load(case);
    let b = super::fingerprint::load(other);
    let stage_identity=a.get("colm").zip(b.get("colm")).map(|(a,b)|json!({"same_kernel":a.kernel==b.kernel,"same_site":a.site_sha256==b.site_sha256,"same_files":a.files==b.files}));
    let key = if e.expected_direction == "rmse_decrease" {
        "rmse"
    } else {
        "bias"
    };
    let same_paired_support = baseline["paired_support_id"] == comparison["paired_support_id"];
    let direction = baseline["metrics"][key]
        .as_f64()
        .zip(comparison["metrics"][key].as_f64())
        .map(|(a, b)| {
            if e.expected_direction == "bias_increase" {
                b > a
            } else {
                b < a
            }
        });
    // Saved stages do not bind current output bytes to an executable; never promote them to causal proof.
    Ok(
        json!({"status":"unverified","cause_verified":false,"baseline_evidence_id":baseline["evidence_id"],"experiment_evidence_id":comparison["evidence_id"],"metrics":comparison["metrics"],"baseline_metrics":baseline["metrics"],"regimes":comparison["regimes"],"expected_direction":e.expected_direction,"same_paired_support":same_paired_support,"direction_observed":if same_paired_support {direction}else{None},"only_target_config_changed":only_target,"configuration_difference_count":difference_count,"configuration_differences_abbreviated":difference_count>20 || differences.iter().any(|d|["field","baseline","experiment"].iter().any(|key|d[*key].as_str().is_some_and(|s|s.chars().count()>=256))),"configuration_differences":differences,"recorded_stage_identity":stage_identity,"missing":["Fresh output-to-executable/forcing/initial-state identity and exact paired support must be independently verified before experimental validation.","A descriptive metric improvement is not proof of causality."]}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(obs: &Path) -> Request {
        serde_json::from_value(json!({"obs":obs,"variable":"f_lfevpa","observation_variable":"Qle","observation_qc":"Qle_qc","from":0,"to":21600,"model_units":"W m-2","obs_units":"W m-2","output_units":"W m-2","model_scale":1.,"obs_scale":1.,"sign_convention":"positive upward","model_utc_offset_seconds":0,"observation_utc_offset_seconds":0,"alignment":"colm_plumber2_fixed_offset","model_interval_seconds":1800,"observation_interval_seconds":1800,"daylight":{"variable":"SWdown","qc_variable":"weather_qc","units":"W m-2","threshold":10.},"wetness":{"variable":"Rainf","qc_variable":"weather_qc","units":"kg m-2 s-1","window_hours":1,"threshold_mm":0.5},"experiment":null})).unwrap()
    }
    fn fixture(tag: &str) -> (std::path::PathBuf, Request) {
        let root = std::env::temp_dir().join(format!("flux-diagnose-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("out/Case/history")).unwrap();
        std::fs::write(
            root.join("case.nml"),
            "&nl_colm\n DEF_CASE_NAME = 'Case'\n DEF_simulation_time%greenwich = .true.\n/\n",
        )
        .unwrap();
        std::fs::write(root.join("forcing.nml"), "&forcing\n/\n").unwrap();
        let t: Vec<f64> = (0..12).map(|i| i as f64 * 1800.).collect();
        let mt: Vec<f64> = t
            .iter()
            .map(|t| colm_hist::time::minutes_from_1900(1970) as f64 + t / 60.)
            .collect();
        let model = vec![12.; 12];
        write(
            &root.join("out/Case/history/Case_hist_1970-01.nc"),
            &[
                ("time", &mt, "minutes since 1900-1-1 0:0:0"),
                ("f_lfevpa", &model, "W m-2"),
            ],
        );
        let radiation: Vec<f64> = (0..12).map(|i| if i < 6 { 100. } else { 0. }).collect();
        let rain: Vec<f64> = (0..12).map(|i| if i < 6 { 0.001 } else { 0. }).collect();
        let obs = root.join("obs.nc");
        write(
            &obs,
            &[
                ("time", &t, "seconds since 1970-01-01 00:00:00"),
                ("Qle", &[10.; 12], "W m-2"),
                ("Qle_qc", &[0.; 12], "1"),
                ("SWdown", &radiation, "W m-2"),
                ("Rainf", &rain, "kg m-2 s-1"),
                ("weather_qc", &[0.; 12], "1"),
            ],
        );
        let r = request(&obs);
        (root, r)
    }
    fn write(path: &Path, arrays: &[(&str, &[f64], &str)]) {
        let mut f = netcdf::create(path).unwrap();
        f.add_dimension("time", arrays[0].1.len()).unwrap();
        for (name, values, units) in arrays {
            let mut v = f.add_variable::<f64>(name, &["time"]).unwrap();
            v.put_attribute("units", *units).unwrap();
            v.put_values(values, ..).unwrap();
        }
    }
    #[test]
    fn real_files_align_group_and_never_claim_causality() {
        let _guard = crate::netcdf_test_guard();
        let (root, r) = fixture("groups");
        let result = run(&root, &r).unwrap();
        assert_eq!(result["metrics"]["n"], 12);
        assert_eq!(result["metrics"]["bias"], 2.);
        assert_eq!(result["regimes"]["daylight"]["day"]["n"], 6);
        assert_eq!(result["regimes"]["wetness"]["unclassified"], 2);
        assert_eq!(result["minimal_experiment"]["cause_verified"], false);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn missing_qc_units_and_wrong_intervals_are_not_passes() {
        let _guard = crate::netcdf_test_guard();
        let (root, mut r) = fixture("guards");
        r.obs_units = "J m-2".into();
        assert!(run(&root, &r).unwrap_err().to_string().contains("units"));
        r.obs_units = "W m-2".into();
        r.model_interval_seconds = 3600;
        assert!(run(&root, &r).is_err());
        r.model_interval_seconds = 1800;
        r.daylight.as_mut().unwrap().qc_variable = "missing".into();
        assert_eq!(
            run(&root, &r).unwrap()["regimes"]["daylight"]["status"],
            "missing"
        );
        r.to = 3600;
        assert_eq!(run(&root, &r).unwrap()["metrics"]["status"], "missing");
        r.model_utc_offset_seconds = 3600;
        assert!(run(&root, &r).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn unrelated_experiment_remains_unverified() {
        let _guard = crate::netcdf_test_guard();
        let (root, mut r) = fixture("baseline");
        let (other, _) = fixture("other");
        r.experiment = Some(Experiment {
            case: other.to_string_lossy().into(),
            field: "nonexistent".into(),
            expected_direction: "rmse_decrease".into(),
        });
        let result = run(&root, &r).unwrap();
        assert_eq!(result["experiment"]["status"], "unverified");
        assert_eq!(result["experiment"]["only_target_config_changed"], false);
        assert_eq!(result["experiment"]["cause_verified"], false);
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(other).unwrap();
    }
    #[test]
    fn qc_fill_values_and_timezone_offsets_change_real_pairing() {
        let _guard = crate::netcdf_test_guard();
        let (root, mut r) = fixture("qc-time");
        let mut file = netcdf::append(&r.obs).unwrap();
        file.variable_mut("Qle")
            .unwrap()
            .put_values(&[-9999.], 0)
            .unwrap();
        file.variable_mut("Qle_qc")
            .unwrap()
            .put_values(&[1.], 1)
            .unwrap();
        drop(file);
        let result = run(&root, &r).unwrap();
        assert_eq!(result["alignment"]["paired"], 10);
        assert_eq!(result["alignment"]["model_records_excluded"], 2);
        r.observation_utc_offset_seconds = 1800;
        let shifted = run(&root, &r).unwrap();
        assert_ne!(result["paired_support_id"], shifted["paired_support_id"]);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn insufficient_and_entirely_masked_months_remain_visible_and_partial() {
        let _guard = crate::netcdf_test_guard();
        let (root, mut r) = fixture("month-coverage");
        let n = 31 * 48 + 2;
        let t: Vec<_> = (0..n).map(|i| i as f64 * 1800.).collect();
        let mt: Vec<_> = t
            .iter()
            .map(|t| colm_hist::time::minutes_from_1900(1970) as f64 + t / 60.)
            .collect();
        write(
            &root.join("out/Case/history/Case_hist_1970-01.nc"),
            &[
                ("time", &mt, "minutes since 1900-1-1 0:0:0"),
                ("f_lfevpa", &vec![12.; n], "W m-2"),
            ],
        );
        let mut qc = vec![0.; n];
        r.to = 31 * 86400 + 3600;
        r.daylight = None;
        r.wetness = None;
        for expected_count in [2, 0] {
            if expected_count == 0 {
                qc[n - 2..].fill(1.);
            }
            write(
                Path::new(&r.obs),
                &[
                    ("time", &t, "seconds since 1970-01-01 00:00:00"),
                    ("Qle", &vec![10.; n], "W m-2"),
                    ("Qle_qc", &qc, "1"),
                ],
            );
            let result = run(&root, &r).unwrap();
            let months = &result["regimes"]["calendar_month"];
            assert_eq!(months["status"], "partial");
            assert_eq!(months["missing"], json!(["month_02"]));
            assert_eq!(months["groups"]["month_01"]["status"], "computed");
            assert_eq!(months["groups"]["month_02"]["status"], "missing");
            assert_eq!(months["groups"]["month_02"]["n"], expected_count);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn negative_unit_conversion_preserves_valid_pairs_and_raw_missing_values() {
        let _guard = crate::netcdf_test_guard();
        let (root, mut r) = fixture("signed-units");
        let mut f = netcdf::append(&r.obs).unwrap();
        f.variable_mut("Qle")
            .unwrap()
            .put_values(&[-9999.], 0)
            .unwrap();
        drop(f);
        r.model_scale = -86400.;
        r.obs_scale = -86400.;
        r.output_units = "J m-2 day-1".into();
        r.sign_convention = "positive downward".into();
        let result = run(&root, &r).unwrap();
        assert_eq!(result["metrics"]["n"], 11);
        assert_eq!(result["metrics"]["obs_mean"], -864000.);
        assert_eq!(result["metrics"]["bias"], -172800.);
        assert_eq!(result["regimes"]["daylight"]["day"]["n"], 5);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn subminute_offsets_fail_explicitly_instead_of_shifting_window_support() {
        let _guard = crate::netcdf_test_guard();
        let (root, mut r) = fixture("offset-precision");
        std::fs::write(
            root.join("case.nml"),
            "&nl_colm\n DEF_CASE_NAME = 'Case'\n DEF_simulation_time%greenwich = .false.\n/\n",
        )
        .unwrap();
        r.from = 1750;
        r.to = 5410;
        for (model, observation) in [(1, 1), (0, 1), (1, 0)] {
            r.model_utc_offset_seconds = model;
            r.observation_utc_offset_seconds = observation;
            assert!(run(&root, &r)
                .unwrap_err()
                .to_string()
                .contains("whole-minute"));
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn packed_target_time_qc_and_regime_inputs_are_rejected() {
        let _guard = crate::netcdf_test_guard();
        let (root, r) = fixture("packing");
        for name in ["Qle", "time", "Qle_qc", "SWdown", "Rainf", "weather_qc"] {
            for (attribute, nonidentity, identity) in
                [("scale_factor", 0.1, 1.0), ("add_offset", 1.0, 0.0)]
            {
                let mut f = netcdf::append(&r.obs).unwrap();
                f.variable_mut(name)
                    .unwrap()
                    .put_attribute(attribute, nonidentity)
                    .unwrap();
                drop(f);
                let f = netcdf::open(&r.obs).unwrap();
                let error = check_variable(&f, name, None, 12).unwrap_err().to_string();
                assert!(error.contains("packing is unsupported"), "{name}: {error}");
                drop(f);
                if name == "Qle" {
                    assert!(run(&root, &r).unwrap_err().to_string().contains("packing"));
                }
                let mut f = netcdf::append(&r.obs).unwrap();
                f.variable_mut(name)
                    .unwrap()
                    .put_attribute(attribute, identity)
                    .unwrap();
                drop(f);
            }
        }
        assert_eq!(run(&root, &r).unwrap()["metrics"]["obs_mean"], 10.);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn duplicate_timestamps_and_insufficient_groups_are_explicit() {
        assert!(regular(&[0., 1800., 1800.], 1800).is_err());
        assert_eq!(summary(&[(1., 2.), (2., 3.)])["status"], "missing");
        assert_eq!(
            summary(&[(1e200, 0.), (1e200, 0.), (1e200, 0.)])["status"],
            "missing"
        );
        assert_eq!(month(0), 1);
        assert_eq!(month(31 * 86400), 2);
    }
}

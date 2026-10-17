//! Names-only layout detection; runtime input validation remains authoritative.
use super::{metfilename_with_year, FORCING_TEMPLATES};
use anyhow::{Context, Result};
use colm_namelist::Value;
use std::collections::BTreeSet;

struct Template {
    name: &'static str,
    dataset: String,
    variables: Vec<(usize, String)>,
}

fn templates() -> Result<Vec<Template>> {
    FORCING_TEMPLATES
        .iter()
        .map(|(name, text)| {
            let doc = colm_namelist::parse(text)?;
            let string = |key: &str| -> Result<String> {
                match doc.get(key) {
                    Some(Value::Str(value)) => Ok(value.clone()),
                    _ => anyhow::bail!("{name}: missing string {key}"),
                }
            };
            let Some(Value::List(names)) = doc.get("DEF_forcing%vname") else {
                anyhow::bail!("{name}: missing variable names");
            };
            let mut variables = Vec::new();
            for (index, variable) in names.iter().enumerate() {
                let Value::Str(variable) = variable else {
                    anyhow::bail!("{name}: invalid variable name")
                };
                let prefix = string(&format!("DEF_forcing%fprefix({})", index + 1))?;
                if !variable.eq_ignore_ascii_case("NULL") && !prefix.eq_ignore_ascii_case("NULL") {
                    variables.push((index, prefix));
                }
            }
            anyhow::ensure!(!variables.is_empty(), "{name}: no required variables");
            Ok(Template {
                name,
                dataset: string("DEF_forcing%dataset")?,
                variables,
            })
        })
        .collect()
}

/// Only these template-defined subdirectories need listing; never recurse through user data.
pub fn forcing_directory_parents() -> Result<Vec<String>> {
    let mut parents = BTreeSet::new();
    for template in templates()? {
        for (_, prefix) in template.variables {
            parents.insert(
                prefix
                    .rsplit_once('/')
                    .map_or("", |(parent, _)| parent)
                    .to_owned(),
            );
        }
    }
    Ok(parents.into_iter().collect())
}

/// A candidate needs every required variable for one common year/month.
/// Identical layouts deliberately produce multiple candidates (e.g. ERA5 and ERA5_LEddy).
pub fn forcing_directory_candidates(files: &BTreeSet<String>) -> Result<Vec<String>> {
    let mut candidates = Vec::new();
    for template in templates()? {
        let (index, prefix) = template
            .variables
            .first()
            .context("empty forcing template")?;
        let mut matched = false;
        for month in 1..=12 {
            let pattern = metfilename_with_year(&template.dataset, prefix, "YEAR", month, *index);
            let start = pattern
                .trim_start_matches('/')
                .split("YEAR")
                .next()
                .unwrap();
            for file in files.range(start.to_owned()..) {
                let Some(rest) = file.strip_prefix(start) else {
                    break;
                };
                let Some(year) = rest
                    .get(..4)
                    .filter(|y| y.bytes().all(|c| c.is_ascii_digit()))
                else {
                    continue;
                };
                if template.variables.iter().all(|(i, p)| {
                    files.contains(
                        metfilename_with_year(&template.dataset, p, year, month, *i)
                            .trim_start_matches('/'),
                    )
                }) {
                    matched = true;
                    break;
                }
            }
            if matched {
                break;
            }
        }
        if matched {
            candidates.push(template.name.to_owned());
        }
    }
    Ok(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn files(name: &str, year: &str, month: i32) -> BTreeSet<String> {
        let t = templates()
            .unwrap()
            .into_iter()
            .find(|t| t.name == name)
            .unwrap();
        t.variables
            .iter()
            .map(|(i, p)| {
                metfilename_with_year(&t.dataset, p, year, month, *i)
                    .trim_start_matches('/')
                    .to_owned()
            })
            .collect()
    }
    #[test]
    fn all_templates_match_runtime_names_including_aliases_and_null_variables() {
        for (name, _) in FORCING_TEMPLATES {
            assert!(
                forcing_directory_candidates(&files(name, "2003", 7))
                    .unwrap()
                    .contains(&name.to_string()),
                "{name}"
            );
        }
    }
    #[test]
    fn missing_or_disjoint_periods_do_not_match() {
        let mut input = files("JRA3Q", "2003", 7);
        let removed = input.pop_first().unwrap();
        input.insert(removed.replace("2003", "2004"));
        assert!(forcing_directory_candidates(&input).unwrap().is_empty());
        assert!(forcing_directory_candidates(&BTreeSet::new())
            .unwrap()
            .is_empty());
    }
    #[test]
    fn shared_layout_is_ambiguous() {
        assert_eq!(
            forcing_directory_candidates(&files("ERA5", "2003", 1)).unwrap(),
            ["ERA5", "ERA5_LEddy"]
        );
    }
}

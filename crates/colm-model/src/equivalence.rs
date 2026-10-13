//! When two model configurations compute the same thing.
//!
//! A version is defined by what CoLM actually runs, not by how its namelist is spelled. Two
//! spellings are equivalent when every switch ends up with the same effective value:
//! - a field written with its Fortran default is the same as leaving it out;
//! - a child switch CoLM forces off whenever its parent is off (`MOD_Namelist.F90`) carries no
//!   information while that parent is off. The case generator writes several of these
//!   (`DEF_USE_FERT = .false.` with crops off, `DEF_Aerosol_Readin = .false.` with SNICAR off), so
//!   without this every migrated case would look like a modified model.
//!
//! Only `physics` (namelist switches and scalars) is normalised. Parameter records, process files
//! and networks compare exactly.

use std::collections::{BTreeMap, BTreeSet};

use colm_namelist::Value;

use crate::{Applicability, CodeRef, Overlay};

/// `(parent, child)`: CoLM sets `child = .false.` whenever `parent` is false.
/// Extracted from the `IF (.not. parent) THEN ... child = .false.` blocks of
/// `vendor/CoLM202X/share/MOD_Namelist.F90`; the test `forced_off_rules_match_colm` re-reads them.
pub(crate) const FORCED_OFF: &[(&str, &str)] = &[
    ("DEF_USE_BGC", "DEF_USE_LAIFEEDBACK"),
    ("DEF_USE_BGC", "DEF_USE_SASU"),
    ("DEF_USE_BGC", "DEF_USE_DIAGMATRIX"),
    ("DEF_USE_BGC", "DEF_USE_PN"),
    ("DEF_USE_BGC", "DEF_USE_NITRIF"),
    ("DEF_USE_BGC", "DEF_USE_FIRE"),
    ("DEF_USE_CROP", "DEF_USE_FERT"),
    ("DEF_USE_CROP", "DEF_USE_CNSOYFIXN"),
    ("DEF_USE_CROP", "DEF_USE_IRRIGATION"),
    ("DEF_USE_GRIDRIVERLAKEFLOW", "DEF_UNITCATCHMENT_REGIONAL"),
    ("DEF_USE_OZONESTRESS", "DEF_USE_OZONEDATA"),
    ("DEF_USE_SNICAR", "DEF_AEROSOL_READIN"),
];

fn default_value(key: &str) -> Option<Value> {
    let field = colm_schema::find(key)?;
    match field.default {
        colm_schema::Default::Logical(b) => Some(Value::Bool(b)),
        colm_schema::Default::Integer(i) => Some(Value::Int(i)),
        colm_schema::Default::Real(text) => {
            crate::validation::literal(text.split('_').next()?).ok()
        }
        colm_schema::Default::Str(s) => Some(Value::Str(s.to_owned())),
        colm_schema::Default::Array(_) => None,
    }
}

fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x.trim_end() == y.trim_end(),
        (Value::List(x), Value::List(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same_value(p, q))
        }
        _ => match (a.as_f64(), b.as_f64()) {
            (Some(x), Some(y)) => x == y || (x - y).abs() <= 1e-12 * x.abs().max(y.abs()),
            _ => false,
        },
    }
}

/// Effective value of `key` (upper case) given the written switches, after CoLM's coercions.
fn effective(written: &BTreeMap<String, Value>, key: &str) -> Option<Value> {
    for (parent, child) in FORCED_OFF {
        if *child == key && matches!(effective(written, parent), Some(Value::Bool(false))) {
            return Some(Value::Bool(false));
        }
    }
    written.get(key).cloned().or_else(|| default_value(key))
}

fn parse(physics: &BTreeMap<String, String>) -> BTreeMap<String, Value> {
    physics
        .iter()
        .map(|(key, text)| {
            let value =
                crate::validation::literal(text).unwrap_or_else(|_| Value::Str(text.clone()));
            (key.to_ascii_uppercase(), value)
        })
        .collect()
}

fn keys_involved(a: &BTreeMap<String, Value>, b: &BTreeMap<String, Value>) -> BTreeSet<String> {
    let mut keys: BTreeSet<String> = a.keys().chain(b.keys()).cloned().collect();
    for (parent, child) in FORCED_OFF {
        if keys.contains(*parent) || keys.contains(*child) {
            keys.insert((*parent).to_owned());
            keys.insert((*child).to_owned());
        }
    }
    keys
}

fn same_effect_physics(a: &BTreeMap<String, Value>, b: &BTreeMap<String, Value>) -> bool {
    keys_involved(a, b)
        .iter()
        .all(|key| match (effective(a, key), effective(b, key)) {
            (Some(x), Some(y)) => same_value(&x, &y),
            (None, None) => true,
            // A field without a known default only matches an identical spelling.
            _ => a
                .get(key)
                .zip(b.get(key))
                .is_some_and(|(x, y)| same_value(x, y)),
        })
}

/// Drop physics entries that do not change what CoLM runs.
pub fn minimize_physics(physics: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut kept = physics.clone();
    let reference = parse(physics);
    for key in physics.keys() {
        let mut without = kept.clone();
        without.remove(key);
        if same_effect_physics(&parse(&without), &reference) {
            kept = without;
        }
    }
    kept
}

/// Whether two overlays make CoLM compute the same thing.
pub fn same_effect(a: &Overlay, b: &Overlay) -> bool {
    let mut a = crate::model_overlay(a);
    let mut b = crate::model_overlay(b);
    crate::registry::normalize_overlay(&mut a);
    crate::registry::normalize_overlay(&mut b);
    a.parameters == b.parameters
        && a.process_files == b.process_files
        && a.hybrid == b.hybrid
        && same_effect_physics(&parse(&a.physics), &parse(&b.physics))
}

/// Whether a revision with this code, overlay and applicability already exists in effect.
pub(crate) fn same_model(
    code: &CodeRef,
    overlay: &Overlay,
    applicability: &Applicability,
    other_code: &CodeRef,
    other_overlay: &Overlay,
    other_applicability: &Applicability,
) -> bool {
    code == other_code
        && applicability == other_applicability
        && same_effect(overlay, other_overlay)
}

/// Whether nothing in this overlay changes the official model.
pub fn is_noop(overlay: &Overlay) -> bool {
    same_effect(overlay, &Overlay::default())
}

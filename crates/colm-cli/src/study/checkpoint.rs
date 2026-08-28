//! 跨平台安全的 Study 状态 checkpoint。
//!
//! 不覆盖现有文件：每次写入一个新的递增 generation。这样即使 Windows
//! 不支持原子覆盖 rename，旧状态也始终保留；崩溃留下的 `.tmp` 会被忽略。

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct Envelope<T> {
    schema_version: u32,
    sequence: u64,
    previous_sha256: Option<String>,
    payload_sha256: String,
    payload: T,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Loaded<T> {
    pub sequence: u64,
    pub file_sha256: String,
    pub payload: T,
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn final_path(dir: &Path, sequence: u64) -> PathBuf {
    dir.join(format!("{sequence:012}.json"))
}

fn sequences(dir: &Path) -> Result<Vec<u64>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(stem) = name.strip_suffix(".json") else {
            continue;
        };
        if stem.len() == 12 {
            if let Ok(sequence) = stem.parse() {
                out.push(sequence);
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

fn read_one<T: DeserializeOwned + Serialize>(dir: &Path, sequence: u64) -> Result<Loaded<T>> {
    let path = final_path(dir, sequence);
    let bytes = fs::read(&path).with_context(|| format!("cannot read {}", path.display()))?;
    let envelope: Envelope<serde_json::Value> = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid checkpoint {}", path.display()))?;
    if envelope.schema_version != SCHEMA_VERSION {
        bail!(
            "unsupported checkpoint schema {} in {}",
            envelope.schema_version,
            path.display()
        );
    }
    if envelope.sequence != sequence {
        bail!(
            "checkpoint filename sequence {sequence} disagrees with payload sequence {}",
            envelope.sequence
        );
    }
    let payload: T = serde_json::from_value(envelope.payload.clone())
        .with_context(|| format!("invalid checkpoint payload in {}", path.display()))?;
    let value_hash = sha256(&serde_json::to_vec(&envelope.payload)?);
    // Checkpoints created before payload normalization hashed the typed value.
    let legacy_hash = sha256(&serde_json::to_vec(&payload)?);
    if envelope.payload_sha256 != value_hash && envelope.payload_sha256 != legacy_hash {
        bail!("checkpoint payload hash mismatch in {}", path.display());
    }
    if let Some(previous) = envelope.previous_sha256.as_deref() {
        let earlier = sequences(dir)?
            .into_iter()
            .filter(|candidate| *candidate < sequence)
            .rev()
            .collect::<Vec<_>>();
        if !earlier.is_empty()
            && !earlier.into_iter().any(|candidate| {
                fs::read(final_path(dir, candidate)).is_ok_and(|bytes| sha256(&bytes) == previous)
            })
        {
            bail!("checkpoint chain hash mismatch in {}", path.display());
        }
    }
    Ok(Loaded {
        sequence,
        file_sha256: sha256(&bytes),
        payload,
    })
}

/// 读取最高的有效 generation。最高文件损坏时向前回退。
pub fn load_latest<T: DeserializeOwned + Serialize>(dir: &Path) -> Result<Option<Loaded<T>>> {
    let candidates = sequences(dir)?;
    for sequence in candidates.into_iter().rev() {
        if let Ok(loaded) = read_one(dir, sequence) {
            return Ok(Some(loaded));
        }
    }
    Ok(None)
}

/// 写入下一个不可变 generation，并在验证成功后只保留最近两个。
///
/// Study 调度器保证单写者；最终文件名使用 `create_new` 语义，检测意外的并发写。
pub fn write_next<T>(dir: &Path, payload: &T) -> Result<Loaded<T>>
where
    T: Serialize + DeserializeOwned,
{
    fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let previous = load_latest::<T>(dir)?;
    let previous_sequence = previous.as_ref().map(|loaded| loaded.sequence);
    let sequence = sequences(dir)?
        .last()
        .copied()
        .unwrap_or(0)
        .checked_add(1)
        .context("checkpoint sequence overflow")?;
    let final_path = final_path(dir, sequence);
    if final_path.exists() {
        bail!("checkpoint {} already exists", final_path.display());
    }

    // Hash the same JSON value that is embedded in the envelope. Hashing the
    // Rust value first is not stable for every f64: serde_json can normalize
    // the last decimal digit when that value is parsed back from the file.
    let payload = serde_json::to_value(payload)?;
    let payload: serde_json::Value = serde_json::from_slice(&serde_json::to_vec(&payload)?)?;
    let payload_bytes = serde_json::to_vec(&payload)?;
    let envelope = Envelope {
        schema_version: SCHEMA_VERSION,
        sequence,
        previous_sha256: previous.map(|loaded| loaded.file_sha256),
        payload_sha256: sha256(&payload_bytes),
        payload,
    };
    let bytes = serde_json::to_vec_pretty(&envelope)?;
    let tmp = dir.join(format!(".{sequence:012}.{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .with_context(|| format!("cannot create {}", tmp.display()))?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    publish_new(&tmp, &final_path)?;
    // 在支持目录 fsync 的平台尽量把新目录项也落盘；不支持时旧 generation
    // 仍然存在，恢复不会读到半份状态。
    let _ = File::open(dir).and_then(|directory| directory.sync_all());

    let loaded = read_one(dir, sequence)?;
    for old in sequences(dir)? {
        if old != sequence && Some(old) != previous_sequence {
            let _ = fs::remove_file(final_path.parent().unwrap().join(format!("{old:012}.json")));
        }
    }
    Ok(loaded)
}

fn publish_new(tmp: &Path, final_path: &Path) -> Result<()> {
    let result = fs::hard_link(tmp, final_path).with_context(|| {
        format!(
            "cannot publish checkpoint {} -> {}",
            tmp.display(),
            final_path.display()
        )
    });
    let _ = fs::remove_file(tmp);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
    struct State {
        done: usize,
    }

    #[derive(Debug, Deserialize, PartialEq, Serialize)]
    struct FloatState {
        score: f64,
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "colm-study-checkpoint-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn writes_monotonic_generations_and_keeps_two() {
        let dir = temp("monotonic");
        for done in 1..=3 {
            let saved = write_next(&dir, &State { done }).unwrap();
            assert_eq!(saved.sequence, done as u64);
        }
        assert_eq!(sequences(&dir).unwrap(), vec![2, 3]);
        assert_eq!(load_latest::<State>(&dir).unwrap().unwrap().payload.done, 3);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn float_payload_checksum_survives_json_round_trip() {
        let dir = temp("float-round-trip");
        let state = FloatState {
            score: 15.272157181227467,
        };
        write_next(&dir, &state).unwrap();
        let loaded = load_latest::<FloatState>(&dir).unwrap().unwrap().payload;
        assert!((loaded.score - state.score).abs() <= f64::EPSILON * state.score.abs());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reads_legacy_typed_payload_hashes() {
        let dir = temp("legacy-float-hash");
        let state = FloatState {
            score: 26.40812929083503,
        };
        let payload_bytes = serde_json::to_vec(&state).unwrap();
        let envelope = Envelope {
            schema_version: SCHEMA_VERSION,
            sequence: 1,
            previous_sha256: None,
            payload_sha256: sha256(&payload_bytes),
            payload: &state,
        };
        fs::write(
            final_path(&dir, 1),
            serde_json::to_vec_pretty(&envelope).unwrap(),
        )
        .unwrap();
        assert!(read_one::<FloatState>(&dir, 1).is_ok());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_latest_falls_back_and_tmp_is_ignored() {
        let dir = temp("fallback");
        write_next(&dir, &State { done: 1 }).unwrap();
        write_next(&dir, &State { done: 2 }).unwrap();
        fs::write(final_path(&dir, 2), b"not json").unwrap();
        fs::write(dir.join(".000000000003.1.tmp"), b"partial").unwrap();
        let recovered = load_latest::<State>(&dir).unwrap().unwrap();
        assert_eq!(recovered.sequence, 1);
        assert_eq!(recovered.payload, State { done: 1 });
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn publishing_never_overwrites_an_existing_checkpoint() {
        let dir = temp("publish-new");
        let tmp = dir.join(".000000000001.tmp");
        let final_path = dir.join("000000000001.json");
        fs::write(&tmp, b"new").unwrap();
        fs::write(&final_path, b"old").unwrap();

        assert!(publish_new(&tmp, &final_path).is_err());
        assert_eq!(fs::read(&final_path).unwrap(), b"old");
        assert!(!tmp.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_latest_can_fall_back_and_continue_writing() {
        let dir = temp("continue-after-fallback");
        write_next(&dir, &State { done: 1 }).unwrap();
        write_next(&dir, &State { done: 2 }).unwrap();
        fs::write(final_path(&dir, 2), b"not json").unwrap();

        let saved = write_next(&dir, &State { done: 3 }).unwrap();

        assert_eq!(saved.sequence, 3);
        assert_eq!(sequences(&dir).unwrap(), vec![1, 3]);
        assert_eq!(load_latest::<State>(&dir).unwrap().unwrap().payload.done, 3);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn payload_hash_mismatch_is_rejected() {
        let dir = temp("hash");
        write_next(&dir, &State { done: 1 }).unwrap();
        let path = final_path(&dir, 1);
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace("\"done\": 1", "\"done\": 9");
        fs::write(path, text).unwrap();
        assert!(load_latest::<State>(&dir).unwrap().is_none());
        fs::remove_dir_all(dir).unwrap();
    }
}

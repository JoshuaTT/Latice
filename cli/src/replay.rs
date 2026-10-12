//! Golden replay harness behind `lattice replay --verify <dir>`.
//!
//! Fixture layout is described in `tests/golden/README.md`. Engine replay does not
//! exist yet, so any discovered match fails with "not implemented": the harness must
//! never report success for a fixture it did not actually verify (AGENTS.md §2.8).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Which targets a replay run compares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// Native replay only.
    Native,
    /// Native replay plus the WASM build, comparing hashes.
    Wasm,
}

/// Successful result of a verify run.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// `<dir>` holds no match directories; nothing to verify (the M0 state).
    NoMatches,
}

/// Why a verify run failed. Every variant maps to a non-zero exit code.
#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    #[error("'{0}' does not exist or is not a directory")]
    NotADirectory(PathBuf),
    #[error("cannot read '{path}': {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid JSON in '{path}': {source}")]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("match_id '{match_id}' in match.json does not match directory name '{dir_name}'")]
    MatchIdMismatch { dir_name: String, match_id: String },
    #[error("'{0}' has no snapshots")]
    NoSnapshots(PathBuf),
    #[error("verification not implemented yet for {0}")]
    NotImplemented(String),
    #[error("WASM parity not implemented yet")]
    WasmParityNotImplemented,
}

/// `match.json`: match identity and configuration.
#[derive(Debug, Deserialize)]
pub struct MatchInfo {
    pub match_id: String,
    pub seed: String,
    pub ruleset_version: String,
    pub board_size: u32,
    pub mutators: Vec<String>,
    pub teams: BTreeMap<String, Team>,
    pub clash_length: u64,
    pub snapshot_cadence: u64,
    pub engine_version: String,
}

/// One team entry in `match.json`.
#[derive(Debug, Deserialize)]
pub struct Team {
    pub model: String,
    pub seats: Vec<String>,
}

/// `expected/hashes.json`: expected state hashes at snapshot cadence.
#[derive(Debug, Deserialize)]
pub struct ExpectedHashes {
    pub match_id: String,
    pub hash_function: String,
    pub serialization_contract: String,
    pub snapshots: Vec<Snapshot>,
}

/// One tick → state hash entry.
#[derive(Debug, Deserialize)]
pub struct Snapshot {
    pub tick: u64,
    pub hash: String,
}

/// A validated match fixture envelope.
#[derive(Debug)]
pub struct Fixture {
    pub info: MatchInfo,
    pub hashes: ExpectedHashes,
}

/// Verifies every golden match under `dir` against `target`.
pub fn verify(dir: &Path, target: Target) -> Result<Outcome, ReplayError> {
    // Validate every envelope first so a broken fixture is reported even before
    // engine replay exists.
    let mut first_id = None;
    for match_dir in discover(dir)? {
        let fixture = load_fixture(&match_dir)?;
        first_id.get_or_insert(fixture.info.match_id);
    }
    match (first_id, target) {
        (None, _) => Ok(Outcome::NoMatches),
        (Some(_), Target::Wasm) => Err(ReplayError::WasmParityNotImplemented),
        (Some(id), Target::Native) => Err(ReplayError::NotImplemented(id)),
    }
}

/// Returns match directories under `dir` (immediate subdirectories holding
/// `match.json`), sorted so results never depend on filesystem order.
pub fn discover(dir: &Path) -> Result<Vec<PathBuf>, ReplayError> {
    if !dir.is_dir() {
        return Err(ReplayError::NotADirectory(dir.to_path_buf()));
    }
    let io_err = |source| ReplayError::Io {
        path: dir.to_path_buf(),
        source,
    };
    let mut matches = Vec::new();
    for entry in fs::read_dir(dir).map_err(io_err)? {
        let path = entry.map_err(io_err)?.path();
        if path.is_dir() && path.join("match.json").is_file() {
            matches.push(path);
        }
    }
    matches.sort();
    Ok(matches)
}

/// Loads and validates the `match.json` / `expected/hashes.json` envelope of one match.
pub fn load_fixture(match_dir: &Path) -> Result<Fixture, ReplayError> {
    let info: MatchInfo = read_json(&match_dir.join("match.json"))?;
    let dir_name = match_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if info.match_id != dir_name {
        return Err(ReplayError::MatchIdMismatch {
            dir_name,
            match_id: info.match_id,
        });
    }
    let hashes_path = match_dir.join("expected").join("hashes.json");
    let hashes: ExpectedHashes = read_json(&hashes_path)?;
    if hashes.snapshots.is_empty() {
        return Err(ReplayError::NoSnapshots(hashes_path));
    }
    Ok(Fixture { info, hashes })
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, ReplayError> {
    let text = fs::read_to_string(path).map_err(|source| ReplayError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&text).map_err(|source| ReplayError::Json {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const HASHES: &str = r#"{
        "match_id": "m1",
        "hash_function": "SHA-256",
        "serialization_contract": "TBD-M1",
        "snapshots": [{"tick": 0, "hash": "e3b0c442"}]
    }"#;

    fn match_json(match_id: &str) -> String {
        format!(
            r#"{{
                "match_id": "{match_id}",
                "seed": "a1b2",
                "ruleset_version": "1.0.0",
                "board_size": 64,
                "mutators": [],
                "teams": {{
                    "alpha": {{"model": "x", "seats": ["A1","A2","A3","A4","A5"]}},
                    "bravo": {{"model": "y", "seats": ["B1","B2","B3","B4","B5"]}}
                }},
                "clash_length": 2000,
                "snapshot_cadence": 500,
                "engine_version": "0.1.0"
            }}"#
        )
    }

    fn add_match(root: &Path, dir_name: &str, match_id: &str, hashes: Option<&str>) {
        let dir = root.join(dir_name);
        fs::create_dir_all(dir.join("expected")).unwrap();
        fs::write(dir.join("match.json"), match_json(match_id)).unwrap();
        if let Some(h) = hashes {
            fs::write(dir.join("expected").join("hashes.json"), h).unwrap();
        }
    }

    #[test]
    fn empty_dir_has_no_matches() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(
            verify(tmp.path(), Target::Native).unwrap(),
            Outcome::NoMatches
        );
    }

    #[test]
    fn readme_only_has_no_matches() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("README.md"), "# Golden").unwrap();
        assert_eq!(
            verify(tmp.path(), Target::Native).unwrap(),
            Outcome::NoMatches
        );
    }

    #[test]
    fn missing_dir_is_an_error() {
        let tmp = TempDir::new().unwrap();
        let err = verify(&tmp.path().join("nope"), Target::Native).unwrap_err();
        assert!(matches!(err, ReplayError::NotADirectory(_)));
    }

    #[test]
    fn file_path_is_an_error() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("README.md");
        fs::write(&file, "").unwrap();
        let err = verify(&file, Target::Native).unwrap_err();
        assert!(matches!(err, ReplayError::NotADirectory(_)));
    }

    #[test]
    fn subdir_without_match_json_is_ignored() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join("scratch")).unwrap();
        fs::write(tmp.path().join("scratch").join("notes.txt"), "").unwrap();
        assert_eq!(
            verify(tmp.path(), Target::Native).unwrap(),
            Outcome::NoMatches
        );
    }

    #[test]
    fn discovery_is_sorted() {
        let tmp = TempDir::new().unwrap();
        for name in ["c", "a", "b"] {
            add_match(tmp.path(), name, name, Some(HASHES));
        }
        let names: Vec<_> = discover(tmp.path())
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a", "b", "c"]);
    }

    #[test]
    fn valid_match_is_not_implemented() {
        let tmp = TempDir::new().unwrap();
        add_match(tmp.path(), "m1", "m1", Some(HASHES));
        let err = verify(tmp.path(), Target::Native).unwrap_err();
        assert!(matches!(&err, ReplayError::NotImplemented(id) if id == "m1"));
        assert_eq!(err.to_string(), "verification not implemented yet for m1");
    }

    #[test]
    fn match_id_mismatch_is_an_error() {
        let tmp = TempDir::new().unwrap();
        add_match(tmp.path(), "m1", "other", Some(HASHES));
        let err = verify(tmp.path(), Target::Native).unwrap_err();
        assert!(matches!(err, ReplayError::MatchIdMismatch { .. }));
    }

    #[test]
    fn malformed_match_json_is_an_error() {
        let tmp = TempDir::new().unwrap();
        add_match(tmp.path(), "m1", "m1", Some(HASHES));
        fs::write(tmp.path().join("m1").join("match.json"), "{").unwrap();
        let err = verify(tmp.path(), Target::Native).unwrap_err();
        assert!(matches!(err, ReplayError::Json { .. }));
    }

    #[test]
    fn missing_hashes_is_an_error() {
        let tmp = TempDir::new().unwrap();
        add_match(tmp.path(), "m1", "m1", None);
        let err = verify(tmp.path(), Target::Native).unwrap_err();
        assert!(matches!(err, ReplayError::Io { .. }));
    }

    #[test]
    fn malformed_hashes_is_an_error() {
        let tmp = TempDir::new().unwrap();
        add_match(tmp.path(), "m1", "m1", Some("not json"));
        let err = verify(tmp.path(), Target::Native).unwrap_err();
        assert!(matches!(err, ReplayError::Json { .. }));
    }

    #[test]
    fn hashes_without_snapshots_is_an_error() {
        let tmp = TempDir::new().unwrap();
        let empty = HASHES.replace(r#"[{"tick": 0, "hash": "e3b0c442"}]"#, "[]");
        add_match(tmp.path(), "m1", "m1", Some(&empty));
        let err = verify(tmp.path(), Target::Native).unwrap_err();
        assert!(matches!(err, ReplayError::NoSnapshots(_)));
    }

    #[test]
    fn wasm_parity_with_match_is_not_implemented() {
        let tmp = TempDir::new().unwrap();
        add_match(tmp.path(), "m1", "m1", Some(HASHES));
        let err = verify(tmp.path(), Target::Wasm).unwrap_err();
        assert!(matches!(err, ReplayError::WasmParityNotImplemented));
    }

    #[test]
    fn wasm_parity_without_matches_is_ok() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(
            verify(tmp.path(), Target::Wasm).unwrap(),
            Outcome::NoMatches
        );
    }
}

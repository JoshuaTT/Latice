//! `lattice check`: run the engine checker over LatticePy files.
//!
//! The CLI may read files (the engine may not), so this loads the ruleset and
//! each module and hands strings to `lattice_engine::checker`. The team
//! complexity quota is applied here, as a sum over a library
//! (`spec/contracts/engine-api.md` §1).

use std::path::{Path, PathBuf};

use lattice_engine::checker::{CheckReport, Severity, check_bytes};
use lattice_engine::ruleset::{Ruleset, RulesetError};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum CheckError {
    #[error(transparent)]
    Ruleset(#[from] RulesetError),
    #[error("cannot read '{}': {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Options for one `lattice check` run.
pub struct Options<'a> {
    pub ruleset: &'a Path,
    /// Treat all paths as one team library and apply the node quota.
    pub library: bool,
    /// Overrides the ruleset's `team_complexity_nodes` (library mode only).
    pub quota: Option<u32>,
}

pub struct FileResult {
    pub path: PathBuf,
    pub report: CheckReport,
}

pub struct Quota {
    pub nodes: u64,
    pub limit: u32,
}

pub struct Outcome {
    pub files: Vec<FileResult>,
    /// Present only in library mode.
    pub quota: Option<Quota>,
}

impl Outcome {
    /// True only if every file passes and the quota (if checked) holds.
    pub fn ok(&self) -> bool {
        self.files.iter().all(|f| f.report.ok) && self.quota.as_ref().is_none_or(Quota::holds)
    }
}

impl Quota {
    pub fn holds(&self) -> bool {
        self.nodes <= u64::from(self.limit)
    }
}

/// Checks `paths` in argument order. Any unreadable file aborts the run.
pub fn run(paths: &[PathBuf], opts: &Options) -> Result<Outcome, CheckError> {
    let ruleset = Ruleset::from_path(opts.ruleset)?;
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        let bytes = std::fs::read(path).map_err(|source| CheckError::Io {
            path: path.clone(),
            source,
        })?;
        files.push(FileResult {
            path: path.clone(),
            report: check_bytes(&bytes, &ruleset),
        });
    }
    let quota = opts.library.then(|| Quota {
        nodes: files.iter().map(|f| u64::from(f.report.node_count)).sum(),
        limit: opts.quota.unwrap_or(ruleset.quota.team_complexity_nodes),
    });
    Ok(Outcome { files, quota })
}

fn severity(s: Severity) -> &'static str {
    match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
    }
}

/// `path:line:col: severity code: message`, then a per-file summary, then the
/// library line when a quota was checked.
pub fn render_human(outcome: &Outcome) -> String {
    let mut out = String::new();
    for f in &outcome.files {
        let p = f.path.display();
        for d in &f.report.diagnostics {
            out.push_str(&format!(
                "{p}:{}:{}: {} {}: {}\n",
                d.line,
                d.col,
                severity(d.severity),
                d.code,
                d.message
            ));
        }
        if f.report.ok {
            out.push_str(&format!("{p}: ok ({} nodes)\n", f.report.node_count));
        } else {
            let errors = f
                .report
                .diagnostics
                .iter()
                .filter(|d| d.severity == Severity::Error)
                .count();
            out.push_str(&format!("{p}: {errors} errors\n"));
        }
    }
    if let Some(q) = &outcome.quota {
        out.push_str(&format!("library: {} / {} nodes\n", q.nodes, q.limit));
    }
    out
}

#[derive(Serialize)]
struct JsonFile<'a> {
    path: String,
    ok: bool,
    node_count: u32,
    interface: JsonInterface,
    diagnostics: Vec<JsonDiagnostic<'a>>,
}

#[derive(Serialize)]
struct JsonInterface {
    init: bool,
    tick: bool,
    on_hit: bool,
    on_hacked: bool,
    on_radio: bool,
}

#[derive(Serialize)]
struct JsonDiagnostic<'a> {
    severity: &'static str,
    code: &'a str,
    line: u32,
    col: u32,
    message: &'a str,
}

/// One JSON array, one object per file, in argument order.
pub fn render_json(outcome: &Outcome) -> Result<String, serde_json::Error> {
    let files: Vec<JsonFile> = outcome
        .files
        .iter()
        .map(|f| JsonFile {
            path: f.path.display().to_string(),
            ok: f.report.ok,
            node_count: f.report.node_count,
            interface: JsonInterface {
                init: f.report.interface.init,
                tick: f.report.interface.tick,
                on_hit: f.report.interface.on_hit,
                on_hacked: f.report.interface.on_hacked,
                on_radio: f.report.interface.on_radio,
            },
            diagnostics: f
                .report
                .diagnostics
                .iter()
                .map(|d| JsonDiagnostic {
                    severity: severity(d.severity),
                    code: d.code,
                    line: d.line,
                    col: d.col,
                    message: &d.message,
                })
                .collect(),
        })
        .collect();
    serde_json::to_string_pretty(&files)
}

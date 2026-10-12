//! Top-level engine error (`spec/contracts/engine-api.md` §5). Variants are
//! stable for M1; sub-errors may grow.

use crate::checker::CheckReport;
use crate::ruleset::RulesetError;
use crate::tick::TickError;
use crate::world::grid::Team;
use crate::world::hash::Phase;

/// Runtime manifest checks from `manifest.schema.json` (#45 and the
/// `Match::new` issue fill this in).
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("manifest validation is not implemented yet")]
    NotImplemented,
}

/// Map loading and fairness errors (#50 and M2 mapgen fill this in).
#[derive(Debug, thiserror::Error)]
pub enum MapError {
    #[error("map loading is not implemented yet")]
    NotImplemented,
}

/// Corrupt or incompatible snapshot (the snapshot/restore issue and #40's
/// byte format fill this in).
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("snapshots are not implemented yet")]
    NotImplemented,
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Ruleset(#[from] RulesetError),
    /// A library module failed `check`.
    #[error("module '{name}' failed the checker")]
    Module { name: String, report: CheckReport },
    #[error("team {team:?} library uses {used} nodes, over the limit of {limit}")]
    Quota { team: Team, used: u32, limit: u32 },
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error(transparent)]
    Map(#[from] MapError),
    #[error("wrong phase: expected {expected}, found {found:?}")]
    WrongPhase {
        expected: &'static str,
        found: Phase,
    },
    #[error("ruleset version mismatch: expected '{expected}', found '{found}'")]
    RulesetMismatch { expected: String, found: String },
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
    /// A broken invariant: a bug, never caused by player code.
    #[error("internal error: {0}")]
    Internal(&'static str),
}

impl From<TickError> for EngineError {
    fn from(e: TickError) -> Self {
        match e {
            TickError::Ruleset(r) => EngineError::Ruleset(r),
            TickError::Pool(_) => EngineError::Internal("could not build the think thread pool"),
        }
    }
}

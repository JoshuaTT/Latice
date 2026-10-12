//! Ruleset loader: the typed, validated parameters every engine module reads.
//!
//! A ruleset is a TOML file under `spec/rulesets/` whose shape is fixed by
//! `spec/contracts/ruleset.schema.json`. Each TOML table maps to one
//! sub-struct, so later spec changes add fields without restructuring.
//! Unknown keys are rejected rather than ignored: a typo in a ruleset must
//! fail loudly instead of silently falling back to nothing (AGENTS.md §2.4).
//!
//! Game logic never touches the file system (AGENTS.md §8), so the engine core
//! takes a `&str` or an already-built [`Ruleset`]; [`Ruleset::from_path`] is
//! for the CLI.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Side length that corresponds to a scale factor of 1. Game-design §4.0
/// defines the scale factor as `s = side_length / 64`; this is that 64, part
/// of the formula's definition rather than a tunable.
pub const SCALE_UNIT_SIDE: u32 = 64;

/// Errors from loading a ruleset or deriving board-scaled values.
#[derive(Debug, thiserror::Error)]
pub enum RulesetError {
    /// The TOML failed to parse or didn't match the ruleset shape. The message
    /// names the offending key (unknown, missing, or wrong type).
    #[error("invalid ruleset: {0}")]
    Parse(#[from] toml::de::Error),
    /// The ruleset file couldn't be read.
    #[error("cannot read ruleset '{}': {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Scaling is undefined below `s = 1`.
    #[error("side length {side} is below the minimum of {SCALE_UNIT_SIDE}")]
    SideTooSmall { side: u32 },
}

/// A complete ruleset (`spec/rulesets/*.toml`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ruleset {
    pub version: String,
    #[serde(rename = "match")]
    pub timing: MatchTiming,
    pub muster: Muster,
    pub bandwidth: Bandwidth,
    pub quota: Quota,
    pub board: Board,
    pub scaling: Scaling,
    pub score_weights: ScoreWeights,
    pub structural: Structural,
    pub mutators: Mutators,
}

/// `[match]`: match timing (game-design §3, §14.2).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchTiming {
    pub epochs_per_match: u32,
    pub beats_per_council: u32,
    pub beat_length_seconds: u32,
    pub charge_bank_cap: u32,
}

/// `[muster]`: Muster token budgets (game-design §8).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Muster {
    pub opening_tokens: u32,
    pub reply_tokens: u32,
}

/// `[bandwidth]`: personal and shared bandwidth (game-design §9.3).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bandwidth {
    pub personal_tokens: u32,
    pub shared_pool_tokens: u32,
}

/// `[quota]`: complexity quota and economy (game-design §9.4).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quota {
    pub team_complexity_nodes: u32,
    pub transfer_loss_percent: u32,
    pub sim_credits: u32,
}

/// `[board]`: board size and unit stats (game-design §4, §5).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Board {
    pub side_length: u32,
    pub core_hp: u32,
    pub core_firewall: u32,
    pub drone_hp: u32,
    pub drone_firewall: u32,
}

/// `[scaling]`: board-size scaling (game-design §4.0) plus per-tick fuel.
///
/// The `*_base` keys are the §14.2 exploration-range entries the tuning system
/// adjusts; derived values are computed from each parameter's `{ base, formula }`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scaling {
    pub clash_length_ticks: Scaled,
    pub clash_length_base: Option<u32>,
    pub drone_cap: Scaled,
    pub drone_cap_base: Option<u32>,
    pub wells: Scaled,
    pub neutral_forges: Scaled,
    pub relics: Scaled,
    pub starting_energy: Scaled,
    pub starting_energy_base: Option<u32>,
    pub intel_shards: u32,
    pub fuel_per_tick: u32,
}

/// A parameter that scales with board size: `{ base, formula }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scaled {
    pub base: u32,
    pub formula: Formula,
}

/// The scaling formulas the ruleset schema allows, over `s = side / 64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Formula {
    #[serde(rename = "base * sqrt(s)")]
    BaseTimesSqrtS,
    #[serde(rename = "base * s")]
    BaseTimesS,
    #[serde(rename = "base + log2(s)")]
    BasePlusLog2S,
}

impl Scaled {
    /// Evaluates the formula at `side` (at least [`SCALE_UNIT_SIDE`]) in
    /// integer math only.
    // spec: game-design §4.0 doesn't say how to round when `s` isn't a perfect
    // square or a power of two (or when `side` isn't a multiple of 64). This
    // takes the floor of the exact value; full scaling rules are M2.
    fn at(self, side: u32) -> u64 {
        let base = u64::from(self.base);
        let side64 = u64::from(side);
        let unit = u64::from(SCALE_UNIT_SIDE);
        match self.formula {
            // floor(base * sqrt(side / 64)) == floor(isqrt(base² * side) / 8),
            // and u128 can't overflow for u32 inputs.
            Formula::BaseTimesSqrtS => {
                let radicand = u128::from(base) * u128::from(base) * u128::from(side);
                let root = radicand.isqrt() / u128::from(SCALE_UNIT_SIDE.isqrt());
                // isqrt(2^96) / 8 < 2^45, so this always fits.
                u64::try_from(root).unwrap_or(u64::MAX)
            }
            Formula::BaseTimesS => base * side64 / unit,
            // side >= 64, so ilog2(side) >= ilog2(64).
            Formula::BasePlusLog2S => base + u64::from(side.ilog2() - SCALE_UNIT_SIDE.ilog2()),
        }
    }
}

/// `[score_weights]`: weights in the §13 score formula.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScoreWeights {
    pub owned_cells: i32,
    pub wells_held: i32,
    pub relics_held: i32,
    pub remaining_core_hp: i32,
    pub live_drones: i32,
}

/// `[structural]`: parameters the tuning system never auto-promotes
/// (system-design §6.3).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Structural {
    pub team_size: u32,
    pub quorum: u32,
    pub win_conditions: Vec<WinCondition>,
}

/// Ways a match can end (game-design §13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WinCondition {
    CoreDestroyed,
    CoreCaptured,
    ScoreAfterFinalEpoch,
}

/// `[mutators]`: the §6 mutator pool and how many a match draws.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mutators {
    pub pool: Vec<String>,
    pub draw_count: u32,
}

/// Values derived from the ruleset for a particular board side length
/// (game-design §4.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Derived {
    pub clash_length_ticks: u64,
    pub drone_cap: u64,
    pub wells: u64,
    pub neutral_forges: u64,
    pub relics: u64,
    pub starting_energy: u64,
}

impl Ruleset {
    /// Parses and validates a ruleset from TOML text.
    pub fn from_toml_str(text: &str) -> Result<Self, RulesetError> {
        Ok(toml::from_str(text)?)
    }

    /// Reads and parses a ruleset file. For CLI use only; game logic must not
    /// do file I/O.
    pub fn from_path(path: &Path) -> Result<Self, RulesetError> {
        let text = std::fs::read_to_string(path).map_err(|source| RulesetError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_toml_str(&text)
    }

    /// The ruleset's semantic version, recorded with every match
    /// (system-design §2.4).
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Board-scaled values at `side`, which must be at least
    /// [`SCALE_UNIT_SIDE`].
    pub fn derived(&self, side: u32) -> Result<Derived, RulesetError> {
        if side < SCALE_UNIT_SIDE {
            return Err(RulesetError::SideTooSmall { side });
        }
        let s = &self.scaling;
        Ok(Derived {
            clash_length_ticks: s.clash_length_ticks.at(side),
            drone_cap: s.drone_cap.at(side),
            wells: s.wells.at(side),
            neutral_forges: s.neutral_forges.at(side),
            relics: s.relics.at(side),
            starting_energy: s.starting_energy.at(side),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT: &str = include_str!("../../spec/rulesets/default.toml");

    fn default() -> Ruleset {
        Ruleset::from_toml_str(DEFAULT).expect("default ruleset parses")
    }

    /// Replaces the first occurrence of `from` in the default ruleset.
    fn edited(from: &str, to: &str) -> String {
        assert!(DEFAULT.contains(from), "fixture text not found: {from}");
        DEFAULT.replacen(from, to, 1)
    }

    fn parse_err(text: &str) -> String {
        Ruleset::from_toml_str(text)
            .expect_err("ruleset should be rejected")
            .to_string()
    }

    #[test]
    fn default_ruleset_parses_with_expected_values() {
        let r = default();
        assert_eq!(r.version(), "1.0.0");
        assert_eq!(r.timing.beats_per_council, 16);
        assert_eq!(r.muster.opening_tokens, 500);
        assert_eq!(r.bandwidth.shared_pool_tokens, 3000);
        assert_eq!(r.quota.transfer_loss_percent, 20);
        assert_eq!(r.board.side_length, 64);
        assert_eq!(r.board.core_firewall, 40);
        assert_eq!(
            r.scaling.relics,
            Scaled {
                base: 2,
                formula: Formula::BasePlusLog2S
            }
        );
        assert_eq!(r.scaling.clash_length_base, Some(2000));
        assert_eq!(r.scaling.fuel_per_tick, 1000);
        assert_eq!(r.score_weights.relics_held, 60);
        assert_eq!(r.structural.quorum, 3);
        assert_eq!(
            r.structural.win_conditions,
            [
                WinCondition::CoreDestroyed,
                WinCondition::CoreCaptured,
                WinCondition::ScoreAfterFinalEpoch
            ]
        );
        assert_eq!(r.mutators.pool.len(), 10);
        assert_eq!(r.mutators.draw_count, 2);
    }

    #[test]
    fn unknown_key_is_rejected_by_name() {
        let text = edited("charge_bank_cap = 3", "charge_bank_cap = 3\nbogus_knob = 1");
        assert!(parse_err(&text).contains("bogus_knob"));
    }

    #[test]
    fn missing_key_is_rejected_by_name() {
        let text = edited("reply_tokens = 200", "");
        assert!(parse_err(&text).contains("reply_tokens"));
    }

    #[test]
    fn wrong_type_is_rejected_by_name() {
        let text = edited("drone_hp = 10", "drone_hp = \"ten\"");
        let err = parse_err(&text);
        assert!(err.contains("drone_hp"), "{err}");
        assert!(err.contains("invalid type"), "{err}");
    }

    #[test]
    fn unknown_formula_is_rejected() {
        let text = edited("\"base * s\"", "\"base * s * s\"");
        assert!(parse_err(&text).contains("base * s * s"));
    }

    #[test]
    fn missing_file_is_an_io_error() {
        let err = Ruleset::from_path(Path::new("no/such/ruleset.toml")).unwrap_err();
        assert!(matches!(err, RulesetError::Io { .. }));
    }

    #[test]
    fn derived_at_side_64_equals_bases() {
        let r = default();
        let d = r.derived(64).unwrap();
        let s = &r.scaling;
        assert_eq!(d.clash_length_ticks, u64::from(s.clash_length_ticks.base));
        assert_eq!(d.drone_cap, u64::from(s.drone_cap.base));
        assert_eq!(d.wells, u64::from(s.wells.base));
        assert_eq!(d.neutral_forges, u64::from(s.neutral_forges.base));
        assert_eq!(d.relics, u64::from(s.relics.base));
        assert_eq!(d.starting_energy, u64::from(s.starting_energy.base));
    }

    #[test]
    fn derived_matches_game_design_table() {
        // game-design §4.0 table: 256×256 and 1024×1024 columns.
        let r = default();
        let expect = |side, values: [u64; 6]| {
            let d = r.derived(side).unwrap();
            let got = [
                d.clash_length_ticks,
                d.drone_cap,
                d.wells,
                d.neutral_forges,
                d.relics,
                d.starting_energy,
            ];
            assert_eq!(got, values, "side {side}");
        };
        expect(256, [4000, 120, 24, 8, 4, 800]);
        expect(1024, [8000, 240, 96, 32, 6, 1600]);
    }

    #[test]
    fn derived_floors_non_square_scales() {
        // s = 2: 2000 * sqrt(2) = 2828.4..., log2(2) = 1.
        let d = default().derived(128).unwrap();
        assert_eq!(d.clash_length_ticks, 2828);
        assert_eq!(d.wells, 12);
        assert_eq!(d.relics, 3);
    }

    #[test]
    fn derived_rejects_side_below_unit() {
        assert!(matches!(
            default().derived(63),
            Err(RulesetError::SideTooSmall { side: 63 })
        ));
    }

    #[test]
    fn derived_handles_extreme_inputs_without_overflow() {
        let scaled = |formula| Scaled {
            base: u32::MAX,
            formula,
        };
        assert!(scaled(Formula::BaseTimesSqrtS).at(u32::MAX) > 0);
        assert!(scaled(Formula::BaseTimesS).at(u32::MAX) > 0);
        assert!(scaled(Formula::BasePlusLog2S).at(u32::MAX) > 0);
    }

    #[test]
    fn module_uses_no_host_floats() {
        // AGENTS.md §8. Built with concat! so this test doesn't match itself.
        let source = include_str!("ruleset.rs");
        for banned in [concat!("f", "32"), concat!("f", "64")] {
            assert!(!source.contains(banned), "found {banned} in ruleset.rs");
        }
    }
}

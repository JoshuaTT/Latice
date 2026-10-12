//! Mutator draw and parameter mutators (`spec/mapgen.md` §6, game-design §6).
//!
//! [`draw`] picks a match's mutators from the `Mapgen` stream exactly as §6.1
//! says; [`apply`] turns the scaled values into the match's effective values
//! (to a copy: the ruleset is never changed). Glass Core, Thin Air and Long
//! Night are applied here; the rest are drawn but pass through until their
//! issues land.

use crate::rng::MatchRng;
use crate::ruleset::{Derived, MutatorParams, Ruleset};

/// The ten mutators of game-design §6, named as in `mutators.pool`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mutator {
    Fog,
    CostlySpawn,
    QuietRadio,
    SmallBoard,
    GlassCore,
    Plague,
    ThinAir,
    BannedCall,
    Drift,
    LongNight,
}

impl Mutator {
    /// The name used in the ruleset pool and the Briefing.
    pub fn name(self) -> &'static str {
        match self {
            Mutator::Fog => "Fog",
            Mutator::CostlySpawn => "Costly Spawn",
            Mutator::QuietRadio => "Quiet Radio",
            Mutator::SmallBoard => "Small Board",
            Mutator::GlassCore => "Glass Core",
            Mutator::Plague => "Plague",
            Mutator::ThinAir => "Thin Air",
            Mutator::BannedCall => "Banned Call",
            Mutator::Drift => "Drift",
            Mutator::LongNight => "Long Night",
        }
    }

    fn from_name(name: &str) -> Option<Mutator> {
        [
            Mutator::Fog,
            Mutator::CostlySpawn,
            Mutator::QuietRadio,
            Mutator::SmallBoard,
            Mutator::GlassCore,
            Mutator::Plague,
            Mutator::ThinAir,
            Mutator::BannedCall,
            Mutator::Drift,
            Mutator::LongNight,
        ]
        .into_iter()
        .find(|m| m.name() == name)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MutatorError {
    #[error("the ruleset has no [mutators.params] table")]
    MissingParams,
    #[error("unknown mutator '{0}' in the ruleset pool")]
    UnknownMutator(String),
}

/// The result of a draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draw {
    /// In drawn order.
    pub mutators: Vec<Mutator>,
    /// The disabled library function, if Banned Call was drawn.
    pub banned_call: Option<String>,
}

/// Draws a match's mutators from `rng` (the `Mapgen` stream, after the
/// symmetry draw) per `spec/mapgen.md` §6.1.
///
/// The pool is `mutators.pool` in listed order, minus Long Night when the
/// scaled clash length is already at least `long_night_ticks`, and minus
/// Banned Call when it has no candidates. `draw_count` entries are taken by a
/// partial Fisher-Yates (`j = i + below(n - i)`); a step with only one
/// candidate left consumes nothing. Banned Call costs one more `below(len)`.
pub fn draw(
    rng: &mut MatchRng,
    ruleset: &Ruleset,
    derived: &Derived,
) -> Result<Draw, MutatorError> {
    let params = ruleset
        .mutators
        .params
        .as_ref()
        .ok_or(MutatorError::MissingParams)?;
    let mut pool = Vec::new();
    for name in &ruleset.mutators.pool {
        let m =
            Mutator::from_name(name).ok_or_else(|| MutatorError::UnknownMutator(name.clone()))?;
        let excluded = match m {
            Mutator::LongNight => derived.clash_length_ticks >= u64::from(params.long_night_ticks),
            Mutator::BannedCall => params.banned_call_candidates.is_empty(),
            _ => false,
        };
        if !excluded {
            pool.push(m);
        }
    }
    let n = pool.len();
    let count = usize::try_from(ruleset.mutators.draw_count).map_or(n, |c| c.min(n));
    for i in 0..count {
        let span = u64::try_from(n - i).unwrap_or(u64::MAX);
        let offset = usize::try_from(rng.below(span)).unwrap_or(0);
        pool.swap(i, i + offset);
    }
    pool.truncate(count);

    let banned_call = if pool.contains(&Mutator::BannedCall) {
        let len = u64::try_from(params.banned_call_candidates.len()).unwrap_or(u64::MAX);
        let idx = usize::try_from(rng.below(len)).unwrap_or(0);
        params.banned_call_candidates.get(idx).cloned()
    } else {
        None
    };
    Ok(Draw {
        mutators: pool,
        banned_call,
    })
}

/// The match's effective values: `derived` with `mutators` applied (§6.2).
/// Glass Core, Thin Air and Long Night act here; every parameter comes from
/// `params`. The mutators touch disjoint fields, so order does not matter.
pub fn apply(derived: Derived, mutators: &[Mutator], params: &MutatorParams) -> Derived {
    derived.with_overrides(|mut d| {
        for m in mutators {
            match m {
                Mutator::GlassCore => d.core_firewall = params.glass_core_firewall,
                Mutator::ThinAir => {
                    let halved =
                        u64::from(d.fuel_per_tick) * u64::from(params.thin_air_fuel_percent) / 100;
                    d.fuel_per_tick = u32::try_from(halved).unwrap_or(u32::MAX);
                }
                Mutator::LongNight => {
                    d.clash_length_ticks =
                        d.clash_length_ticks.max(u64::from(params.long_night_ticks));
                }
                // TODO(#132): Fog, Costly Spawn, Plague, Quiet Radio, Small Board.
                // TODO(#72): Drift, Banned Call.
                Mutator::Fog
                | Mutator::CostlySpawn
                | Mutator::QuietRadio
                | Mutator::SmallBoard
                | Mutator::Plague
                | Mutator::Drift
                | Mutator::BannedCall => {}
            }
        }
        d
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::StreamId;

    fn params() -> MutatorParams {
        // Test values: the spec/mapgen.md defaults.
        MutatorParams {
            fog_sense_cap: 3,
            costly_spawn_multiplier: 3,
            quiet_radio_range: 3,
            small_board_keys: 16,
            glass_core_firewall: 15,
            plague_hack_cost_percent: 50,
            thin_air_fuel_percent: 50,
            drift_percent: 1,
            long_night_ticks: 4000,
            banned_call_candidates: Vec::new(),
        }
    }

    fn ruleset(draw_count: u32, candidates: &[&str]) -> Ruleset {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/rulesets/default.toml");
        let mut r = Ruleset::from_path(&path).expect("default ruleset loads");
        let mut p = params();
        p.banned_call_candidates = candidates.iter().map(|c| (*c).to_owned()).collect();
        r.mutators.params = Some(p);
        r.mutators.draw_count = draw_count;
        r
    }

    fn mapgen(seed: u64) -> MatchRng {
        MatchRng::new(seed, StreamId::Mapgen)
    }

    fn drawn(seed: u64, r: &Ruleset, side: u32) -> Draw {
        draw(&mut mapgen(seed), r, &r.derived(side).unwrap()).unwrap()
    }

    #[test]
    fn same_stream_state_gives_the_same_draw() {
        let r = ruleset(3, &["me.step_toward"]);
        for seed in 0..50 {
            assert_eq!(drawn(seed, &r, 64), drawn(seed, &r, 64));
        }
    }

    #[test]
    fn draw_has_count_distinct_mutators_or_the_whole_pool() {
        for (count, side, candidates) in
            [(2, 64, &["f"][..]), (3, 64, &["f"][..]), (3, 1024, &[][..])]
        {
            let r = ruleset(count, candidates);
            for seed in 0..40 {
                let d = drawn(seed, &r, side);
                assert_eq!(d.mutators.len(), count as usize);
                let mut sorted: Vec<_> = d.mutators.iter().map(|m| m.name()).collect();
                sorted.sort_unstable();
                sorted.dedup();
                assert_eq!(sorted.len(), count as usize, "duplicates in {d:?}");
            }
        }
        // A pool smaller than the count yields all of it.
        let mut r = ruleset(3, &[]);
        r.mutators.pool = vec!["Fog".into(), "Drift".into()];
        let d = drawn(1, &r, 64);
        let mut names: Vec<_> = d.mutators.iter().map(|m| m.name()).collect();
        names.sort_unstable();
        assert_eq!(names, ["Drift", "Fog"]);
    }

    #[test]
    fn long_night_is_never_drawn_at_scale_four_or_more() {
        let r = ruleset(3, &["f"]);
        let mut saw_at_s1 = false;
        for seed in 0..300 {
            saw_at_s1 |= drawn(seed, &r, 64).mutators.contains(&Mutator::LongNight);
            for side in [256, 512, 1024] {
                assert!(!drawn(seed, &r, side).mutators.contains(&Mutator::LongNight));
            }
        }
        assert!(saw_at_s1, "Long Night should be drawable at s = 1");
    }

    #[test]
    fn banned_call_needs_candidates_and_picks_one() {
        let none = ruleset(3, &[]);
        let some = ruleset(3, &["me.step_toward", "me.sense", "team.board.get"]);
        let mut picked = Vec::new();
        for seed in 0..300 {
            let d = drawn(seed, &none, 64);
            assert!(!d.mutators.contains(&Mutator::BannedCall) && d.banned_call.is_none());
            let d = drawn(seed, &some, 64);
            assert_eq!(
                d.mutators.contains(&Mutator::BannedCall),
                d.banned_call.is_some()
            );
            if let Some(f) = d.banned_call {
                picked.push(f);
            }
        }
        assert!(!picked.is_empty());
        picked.sort();
        picked.dedup();
        assert!(picked.len() > 1, "all three candidates should come up");
    }

    #[test]
    fn the_draw_consumes_one_value_per_ranged_step_plus_one_for_banned_call() {
        // Replays the procedure by hand on a clone of the stream and checks
        // the real draw left the stream at the same position.
        let r = ruleset(3, &["f", "g"]);
        for seed in 0..100 {
            let derived = r.derived(64).unwrap();
            let mut real = mapgen(seed);
            let d = draw(&mut real, &r, &derived).unwrap();

            let n = r.mutators.pool.len(); // nothing is excluded at s = 1 with candidates
            let mut expected = mapgen(seed);
            for i in 0..3usize {
                expected.below((n - i) as u64);
            }
            if d.mutators.contains(&Mutator::BannedCall) {
                expected.below(2);
            }
            assert_eq!(real.state(), expected.state(), "seed {seed}");
        }
    }

    #[test]
    fn a_step_with_one_candidate_left_consumes_nothing() {
        let mut r = ruleset(2, &[]);
        r.mutators.pool = vec!["Fog".into(), "Drift".into()];
        let mut real = mapgen(5);
        let before = real.state();
        draw(&mut real, &r, &r.derived(64).unwrap()).unwrap();
        let mut expected = mapgen(5);
        expected.below(2); // the i = 1 step has range 1
        assert_ne!(real.state(), before);
        assert_eq!(real.state(), expected.state());
    }

    #[test]
    fn pinned_draw_vectors() {
        // Pins the procedure (pool order, Fisher-Yates, stream) so it cannot
        // drift. Default pool, draw_count 2 and 3, no candidates, s = 1.
        let two = drawn(1, &ruleset(2, &[]), 64);
        let three = drawn(1, &ruleset(3, &[]), 64);
        let names = |d: &Draw| d.mutators.iter().map(|m| m.name()).collect::<Vec<_>>();
        assert_eq!(names(&two), PINNED_TWO);
        assert_eq!(names(&three), PINNED_THREE);
    }
    // Worked by hand from the seed-1 Mapgen vector (14698744503296514894,
    // 4853555873038029046, 16012919734872187166, ...) over the 9-entry pool
    // Fog, Costly Spawn, Quiet Radio, Small Board, Glass Core, Plague,
    // Thin Air, Drift, Long Night (Banned Call is out: no candidates):
    // below(9) = 0, below(8) = 6, below(7) = 4.
    const PINNED_TWO: [&str; 2] = ["Fog", "Drift"];
    const PINNED_THREE: [&str; 3] = ["Fog", "Drift", "Thin Air"];

    #[test]
    fn missing_params_and_unknown_pool_names_are_errors() {
        let mut r = ruleset(2, &[]);
        let d = r.derived(64).unwrap();
        r.mutators.pool.push("Gravity".into());
        assert_eq!(
            draw(&mut mapgen(1), &r, &d),
            Err(MutatorError::UnknownMutator("Gravity".into()))
        );
        r.mutators.params = None;
        assert_eq!(
            draw(&mut mapgen(1), &r, &d),
            Err(MutatorError::MissingParams)
        );
    }

    #[test]
    fn glass_core_sets_the_core_firewall_from_params() {
        let r = ruleset(2, &[]);
        let base = r.derived(64).unwrap();
        assert_eq!(base.core_firewall, 40);
        let d = apply(base, &[Mutator::GlassCore], &params());
        assert_eq!(d.core_firewall, 15);
        assert_eq!(
            Derived {
                core_firewall: 40,
                ..d
            },
            base
        );
    }

    #[test]
    fn thin_air_floors_an_odd_fuel_value() {
        let mut r = ruleset(2, &[]);
        r.scaling.fuel_per_tick = 1001;
        let d = apply(r.derived(64).unwrap(), &[Mutator::ThinAir], &params());
        assert_eq!(d.fuel_per_tick, 500);
        let even = apply(
            ruleset(2, &[]).derived(64).unwrap(),
            &[Mutator::ThinAir],
            &params(),
        );
        assert_eq!(even.fuel_per_tick, 500); // 1000 * 50 / 100
    }

    #[test]
    fn long_night_extends_s1_and_leaves_s4_alone() {
        let r = ruleset(2, &[]);
        let at = |side| apply(r.derived(side).unwrap(), &[Mutator::LongNight], &params());
        assert_eq!(at(64).clash_length_ticks, 4000); // 2000 -> 4000
        assert_eq!(at(256).clash_length_ticks, 4000); // already 4000
        assert_eq!(at(1024).clash_length_ticks, 8000); // never shortened
    }

    #[test]
    fn other_mutators_pass_through_and_the_ruleset_is_unchanged() {
        let r = ruleset(2, &["f"]);
        let before = r.clone();
        let base = r.derived(64).unwrap();
        let all = [
            Mutator::Fog,
            Mutator::CostlySpawn,
            Mutator::QuietRadio,
            Mutator::SmallBoard,
            Mutator::Plague,
            Mutator::Drift,
            Mutator::BannedCall,
        ];
        assert_eq!(apply(base, &all, &params()), base);
        let _ = apply(
            base,
            &[Mutator::GlassCore, Mutator::ThinAir, Mutator::LongNight],
            &params(),
        );
        assert_eq!(r, before);
    }

    #[test]
    fn mutator_names_match_the_ruleset_pool() {
        let r = ruleset(2, &[]);
        for name in &r.mutators.pool {
            assert_eq!(
                Mutator::from_name(name).map(Mutator::name),
                Some(name.as_str())
            );
        }
    }
}

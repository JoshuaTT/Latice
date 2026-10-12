//! Fuzz target for the LatticePy checker: arbitrary bytes must never panic or
//! hang, and the report must satisfy the engine API contract (§1).
#![no_main]

use std::sync::OnceLock;

use lattice_engine::checker::{Severity, check_bytes};
use lattice_engine::ruleset::Ruleset;
use libfuzzer_sys::fuzz_target;

fn ruleset() -> &'static Ruleset {
    static RULESET: OnceLock<Ruleset> = OnceLock::new();
    RULESET.get_or_init(|| {
        Ruleset::from_toml_str(include_str!("../../../spec/rulesets/default.toml"))
            .expect("default ruleset loads")
    })
}

fuzz_target!(|data: &[u8]| {
    let rs = ruleset();
    let report = check_bytes(data, rs);
    // `ok` iff no Error diagnostics.
    let errors = report
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error);
    assert_eq!(report.ok, !errors);
    // Sorted by (line, col, code).
    assert!(
        report
            .diagnostics
            .windows(2)
            .all(|w| (w[0].line, w[0].col, w[0].code) <= (w[1].line, w[1].col, w[1].code))
    );
    // Pure function: the same input gives an identical report.
    assert_eq!(report, check_bytes(data, rs));
});

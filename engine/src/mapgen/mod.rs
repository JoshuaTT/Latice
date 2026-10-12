//! Map generator: seeded, symmetric maps with fairness validation.

pub mod symmetry;

pub use symmetry::{Symmetry, partner, is_self_partner, symmetry_check, team_half};
//! World model: chunked grid, cells, terrain, ownership, integrity, marks
//! (structures, drones, Cores and Relics follow in a second PR for #43).

pub mod grid;

pub use grid::{Cell, Grid, GridError, Mark, Pos, Team, Terrain};

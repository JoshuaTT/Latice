//! Chunked cell grid with copy-on-write chunks (system-design §2.1).
//!
//! The grid is split into 64×64 [`Chunk`]s held behind `Arc`, so
//! [`Grid::snapshot`] copies only the chunk table (O(chunks)) and a mutation
//! clones just the one chunk it touches (`Arc::make_mut`). Chunks that were
//! never written are absent and read as the default cell, which keeps an empty
//! 1024² board tiny.
//!
//! Marks are rare, so they live in a sparse per-chunk `BTreeMap` rather than
//! in every [`Cell`]; this keeps `Cell` at 3 bytes (a full 1024² board is
//! ~3 MiB of cells) and the ordered map keeps iteration deterministic.

use std::collections::BTreeMap;
use std::sync::Arc;

/// Side length of a chunk in cells (system-design §2.1).
pub const CHUNK_SIDE: usize = 64;
const CHUNK_CELLS: usize = CHUNK_SIDE * CHUNK_SIDE;

/// Most marks a single cell may hold (game-design §4.1).
pub const MAX_MARKS: usize = 4;

/// A cell coordinate. `u16` covers the 1024² maximum board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pos {
    pub x: u16,
    pub y: u16,
}

/// Owner of a cell or entity; names match `events.schema.json` `Team`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Team {
    A,
    B,
    #[default]
    Neutral,
}

/// Cell terrain (game-design §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Terrain {
    #[default]
    Open,
    Wall,
    Well,
    Relic,
    Forge,
    Core,
}

/// One grid cell. Integrity is 1–3 (game-design §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub terrain: Terrain,
    pub owner: Team,
    pub integrity: u8,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            terrain: Terrain::Open,
            owner: Team::Neutral,
            integrity: 1,
        }
    }
}

/// A mark left on a cell (game-design §5.5). Key and value are opaque bytes
/// until the VM defines LatticePy values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub expires_tick: u64,
    pub writer: Team,
}

/// Errors from grid access.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GridError {
    #[error("position ({}, {}) is outside the {side}×{side} grid", pos.x, pos.y)]
    OutOfBounds { pos: Pos, side: u32 },
    #[error("a cell holds at most {MAX_MARKS} marks, got {0}")]
    TooManyMarks(usize),
}

#[derive(Debug, Clone)]
struct Chunk {
    cells: Vec<Cell>,
    marks: BTreeMap<u16, Vec<Mark>>,
}

impl Chunk {
    fn new() -> Self {
        Chunk {
            cells: vec![Cell::default(); CHUNK_CELLS],
            marks: BTreeMap::new(),
        }
    }
}

/// The chunked grid. `Clone` is the same O(chunks) copy as [`Grid::snapshot`].
#[derive(Debug, Clone)]
pub struct Grid {
    side: u32,
    chunks_per_row: usize,
    chunks: Vec<Option<Arc<Chunk>>>,
}

impl Grid {
    /// An all-default grid of `side`×`side` cells; no chunks are allocated.
    pub fn new(side: u32) -> Self {
        let chunks_per_row = (side as usize).div_ceil(CHUNK_SIDE);
        Grid {
            side,
            chunks_per_row,
            chunks: vec![None; chunks_per_row * chunks_per_row],
        }
    }

    pub fn side(&self) -> u32 {
        self.side
    }

    /// O(chunks) copy-on-write snapshot: later writes to either grid don't
    /// affect the other, and untouched chunks stay shared.
    pub fn snapshot(&self) -> Grid {
        self.clone()
    }

    /// Whether `self` and `other` share the allocation for the chunk holding
    /// `pos` (both absent counts as shared). Grids must have the same side.
    pub fn shares_chunk_with(&self, other: &Grid, pos: Pos) -> bool {
        let i = self.locate(pos).0;
        match (self.chunks.get(i), other.chunks.get(i)) {
            (Some(Some(a)), Some(Some(b))) => Arc::ptr_eq(a, b),
            (Some(None), Some(None)) => true,
            _ => false,
        }
    }

    fn check(&self, pos: Pos) -> Result<(), GridError> {
        if u32::from(pos.x) < self.side && u32::from(pos.y) < self.side {
            Ok(())
        } else {
            Err(GridError::OutOfBounds {
                pos,
                side: self.side,
            })
        }
    }

    /// (chunk index, cell index within chunk). Caller must bounds-check.
    fn locate(&self, pos: Pos) -> (usize, usize) {
        let (x, y) = (usize::from(pos.x), usize::from(pos.y));
        let chunk = (y / CHUNK_SIDE) * self.chunks_per_row + x / CHUNK_SIDE;
        (chunk, (y % CHUNK_SIDE) * CHUNK_SIDE + x % CHUNK_SIDE)
    }

    fn chunk_mut(&mut self, index: usize) -> &mut Chunk {
        Arc::make_mut(self.chunks[index].get_or_insert_with(|| Arc::new(Chunk::new())))
    }

    pub fn get(&self, pos: Pos) -> Result<Cell, GridError> {
        self.check(pos)?;
        let (c, i) = self.locate(pos);
        Ok(self.chunks[c]
            .as_ref()
            .map_or_else(Cell::default, |ch| ch.cells[i]))
    }

    pub fn set(&mut self, pos: Pos, cell: Cell) -> Result<(), GridError> {
        self.check(pos)?;
        let (c, i) = self.locate(pos);
        // Writing the default into an absent chunk is a no-op; don't allocate.
        if self.chunks[c].is_none() && cell == Cell::default() {
            return Ok(());
        }
        self.chunk_mut(c).cells[i] = cell;
        Ok(())
    }

    /// Marks on `pos` in write order (at most [`MAX_MARKS`]).
    pub fn marks(&self, pos: Pos) -> Result<&[Mark], GridError> {
        self.check(pos)?;
        let (c, i) = self.locate(pos);
        Ok(self.chunks[c]
            .as_ref()
            .and_then(|ch| ch.marks.get(&(i as u16)))
            .map_or(&[][..], Vec::as_slice))
    }

    /// Replaces all marks on `pos`.
    pub fn set_marks(&mut self, pos: Pos, marks: Vec<Mark>) -> Result<(), GridError> {
        self.check(pos)?;
        if marks.len() > MAX_MARKS {
            return Err(GridError::TooManyMarks(marks.len()));
        }
        let (c, i) = self.locate(pos);
        if marks.is_empty() && self.chunks[c].is_none() {
            return Ok(());
        }
        let chunk = self.chunk_mut(c);
        if marks.is_empty() {
            chunk.marks.remove(&(i as u16));
        } else {
            chunk.marks.insert(i as u16, marks);
        }
        Ok(())
    }

    /// Every cell in row-major order (y outer, x inner), including defaults.
    pub fn iter(&self) -> impl Iterator<Item = (Pos, Cell)> + '_ {
        let side = self.side as u16;
        (0..side).flat_map(move |y| {
            (0..side).map(move |x| {
                let pos = Pos { x, y };
                let (c, i) = self.locate(pos);
                let cell = self.chunks[c]
                    .as_ref()
                    .map_or_else(Cell::default, |ch| ch.cells[i]);
                (pos, cell)
            })
        })
    }

    /// Bytes held by allocated chunks plus the chunk table; marks excluded.
    pub fn estimated_bytes(&self) -> usize {
        let table = self.chunks.len() * std::mem::size_of::<Option<Arc<Chunk>>>();
        let cells =
            self.chunks.iter().flatten().count() * CHUNK_CELLS * std::mem::size_of::<Cell>();
        table + cells
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: u16, y: u16) -> Pos {
        Pos { x, y }
    }

    fn wall() -> Cell {
        Cell {
            terrain: Terrain::Wall,
            owner: Team::A,
            integrity: 3,
        }
    }

    #[test]
    fn get_set_across_chunk_boundaries() {
        for side in [64u32, 1024] {
            let mut g = Grid::new(side);
            let last = (side - 1) as u16;
            let edge = 64.min(last);
            let mut targets = vec![p(0, 0), p(63, 63), p(edge, 0), p(last, last), p(63, edge)];
            targets.sort();
            targets.dedup();
            for pos in targets {
                assert_eq!(g.get(pos).unwrap(), Cell::default());
                g.set(pos, wall()).unwrap();
                assert_eq!(g.get(pos).unwrap(), wall());
            }
            assert_eq!(g.get(p(1, 1)).unwrap(), Cell::default());
        }
    }

    #[test]
    fn out_of_bounds_is_an_error() {
        let mut g = Grid::new(64);
        assert!(matches!(
            g.get(p(64, 0)),
            Err(GridError::OutOfBounds { .. })
        ));
        assert!(g.set(p(0, 64), wall()).is_err());
        assert!(g.marks(p(64, 64)).is_err());
    }

    #[test]
    fn snapshot_is_isolated_and_shares_untouched_chunks() {
        let mut g = Grid::new(128);
        g.set(p(0, 0), wall()).unwrap();
        g.set(p(100, 100), wall()).unwrap();
        let snap = g.snapshot();
        g.set(p(1, 0), wall()).unwrap();
        assert_eq!(snap.get(p(1, 0)).unwrap(), Cell::default());
        assert_eq!(g.get(p(1, 0)).unwrap(), wall());
        assert!(!g.shares_chunk_with(&snap, p(0, 0)));
        assert!(g.shares_chunk_with(&snap, p(100, 100)));
        assert!(g.shares_chunk_with(&snap, p(100, 0)));
    }

    #[test]
    fn marks_are_capped_and_snapshot_isolated() {
        let mark = |k: u8| Mark {
            key: vec![k],
            value: vec![],
            expires_tick: 10,
            writer: Team::B,
        };
        let mut g = Grid::new(64);
        g.set_marks(p(5, 5), (0..4).map(mark).collect()).unwrap();
        assert_eq!(
            g.set_marks(p(5, 5), (0..5).map(mark).collect()),
            Err(GridError::TooManyMarks(5))
        );
        let snap = g.snapshot();
        g.set_marks(p(5, 5), vec![]).unwrap();
        assert!(g.marks(p(5, 5)).unwrap().is_empty());
        assert_eq!(snap.marks(p(5, 5)).unwrap().len(), 4);
    }

    #[test]
    fn iteration_is_row_major() {
        let mut g = Grid::new(64);
        g.set(p(2, 1), wall()).unwrap();
        let cells: Vec<(Pos, Cell)> = g.iter().collect();
        assert_eq!(cells.len(), 64 * 64);
        assert_eq!(cells[0].0, p(0, 0));
        assert_eq!(cells[1].0, p(1, 0));
        assert_eq!(cells[64 + 2], (p(2, 1), wall()));
        assert!(
            cells
                .windows(2)
                .all(|w| (w[0].0.y, w[0].0.x) < (w[1].0.y, w[1].0.x))
        );
    }

    #[test]
    fn memory_estimate_for_1024_squared() {
        let empty = Grid::new(1024);
        assert!(empty.estimated_bytes() < 1 << 20);
        let mut full = Grid::new(1024);
        for y in (0..1024u16).step_by(CHUNK_SIDE) {
            for x in (0..1024u16).step_by(CHUNK_SIDE) {
                full.set(p(x, y), wall()).unwrap();
            }
        }
        // Budget is 200 MB for the whole world (system-design §2.1).
        assert!(full.estimated_bytes() < 200 * 1024 * 1024);
        println!(
            "1024² empty: {} B, fully allocated: {} B",
            empty.estimated_bytes(),
            full.estimated_bytes()
        );
    }
}

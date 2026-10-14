//! Pure editing operations on the Corinzia model: note input and ranges
//! (`note_input`), score structure (`structure`), tuplets (`tuplets`).
//!
//! Note input follows the "overwrite" model of the major notation editors:
//! a new event occupies `[pos, pos + duration)`, anything it covers is removed
//! and anything it partially covers is truncated; voice 1 stays contiguous
//! (remainders and gaps become rests) while voices 2–4 may have holes.
//!
//! Every voice function takes the current event slice and returns a new list,
//! so the caller decides how to store it (the app snapshots the project for
//! undo). Element ids come from an [`Ids`] counter seeded from
//! `Project::next_id` and written back afterwards.

pub mod note_input;
pub mod structure;
pub mod tuplets;

pub use note_input::*;
pub use structure::*;

/// A running element-id allocator (`Project::next_id`).
#[derive(Clone, Copy, Debug)]
pub struct Ids(pub u64);
impl Ids {
    pub fn next(&mut self) -> u64 {
        let id = self.0;
        self.0 += 1;
        id
    }
}

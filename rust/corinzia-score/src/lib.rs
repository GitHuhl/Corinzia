//! Score data shared by Corinzia's crates: exact fractions, note values and
//! the notation of a movement as the engraver and exporters read it.

pub mod beams;
pub mod frac;
pub mod score;
pub mod values;

pub use frac::Frac;

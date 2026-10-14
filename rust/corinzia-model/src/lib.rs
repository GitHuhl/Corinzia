//! Corinzia's project model in Rust.
//!
//! The same structure as the C++ model (include/corinzia/model/Model.hpp):
//! a Project owns players (with their instruments), DAW tracks and
//! movements; a movement holds the maps (tempo, meter, key), structure marks
//! and, per player, lanes of up to four voices of events. Every object has a
//! persistent element id shared with the file, the engraving and CLAP.
//!
//! `store` reads and writes `.project` files (SQLite, schema/project.sql),
//! compatible with the C++ application; `notation` converts a movement into
//! the engraver's `corinzia_score::score::Score`.

pub mod model;
pub mod notation;
pub mod store;

pub use model::*;

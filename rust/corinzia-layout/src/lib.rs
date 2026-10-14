//! Corinzia's native engraver.
//!
//! `layout_line` turns a `corinzia_score::score::Score` into a
//! [`display::DisplayList`]: SMuFL glyphs (Bravura), strokes, beams and tie
//! crescents in staff spaces, each tagged with the model's element id. It is
//! independent of any GUI toolkit: the app's iced canvas, the PDF writer and
//! the tiny-skia rasterizer used by the reference tests all draw the same list.

pub mod display;
pub mod font;
pub mod layout;
pub mod notation;
pub mod skyline;
#[cfg(feature = "pdf")]
pub mod pdf;
#[cfg(feature = "raster")]
pub mod raster;

pub use display::DisplayList;
pub use layout::{layout_line, layout_pages, Options, PageSpec};

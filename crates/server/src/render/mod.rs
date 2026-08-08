//! Turning what you typed into what gets stored.
//!
//! Rendering happens once, at write time, and both derived forms are cached in
//! the row. The public site's read path is then a bare `SELECT` into a template —
//! it never runs a markdown parser or a sanitizer.

pub mod markdown;

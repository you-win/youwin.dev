//! youwin.dev — a single-user microblog.
//!
//! One binary serves two surfaces from one SQLite file:
//!
//! - [`public`] — `youwin.dev`, server-rendered HTML, no JS, no cookies.
//! - [`write`] — `write.youwin.dev`, the authenticated JSON API.
//!
//! The split exists so the boundary between them is structural rather than
//! conventional; see `DESIGN.md`.
//!
//! This crate is a library so integration tests can reach the modules directly.
//! `main.rs` is a thin binary over it.

pub mod config;
pub mod db;
pub mod error;
pub mod public;
pub mod render;
pub mod seed;
pub mod write;

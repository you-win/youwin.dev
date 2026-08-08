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

pub mod auth;
pub mod backup;
pub mod cache;
pub mod clock;
pub mod config;
pub mod db;
pub mod error;
pub mod export;
pub mod familiar;
pub mod mood;
pub mod public;
pub mod render;
pub mod seed;
pub mod tag;
pub mod url;
pub mod write;

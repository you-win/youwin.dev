//! maud templates for the public archive.
//!
//! These are plain functions returning `Markup`, which is the point: the
//! authoring app's `/preview/:id` route (M3) calls the same functions, so a
//! draft preview is the real published rendering rather than an approximation
//! that drifts.
//!
//! maud escapes every interpolation by construction. The single exception is
//! `body_html`, which goes through `PreEscaped` — and is exactly why
//! sanitization happens at write time and is tested.

pub mod atom;
pub mod layout;
pub mod pages;
pub mod post;
pub mod time_fmt;

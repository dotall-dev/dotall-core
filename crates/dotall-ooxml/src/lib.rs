//! Shared OOXML ZIP helpers: part snapshots and package probes.
//!
//! This crate is not a format handler. Format crates pass their `format_id`.

mod probe;
mod snapshot;

pub use probe::{has_zip_magic, zip_contains_entry};
pub use snapshot::{decode_package, encode_package};

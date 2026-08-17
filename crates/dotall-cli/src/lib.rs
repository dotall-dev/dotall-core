//! Library surface for demo generators shared with examples and integration tests.
//!
//! The `dotall` binary lives in `src/main.rs` and does not depend on this module tree.

#[cfg(all(feature = "pptx", feature = "docx", feature = "pdf"))]
pub mod q3_pack;

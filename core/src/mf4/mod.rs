//! ASAM MDF4 (`.mf4`) reading support, so an instrument's direct binary
//! export can feed the exact same `ParsedTable` pipeline as a TSV/CSV
//! export -- everything from column mapping onward is unaware which path
//! produced its input.
//!
//! `blocks`/`parsing`/`api`/`signal`/`error` are a vendored, trimmed subset
//! of [`mf4-rs`](https://github.com/dmagyar-0/mf4-rs) v3.6.0 (MIT-licensed;
//! see `ATTRIBUTION.md` in this directory), kept because implementing an
//! MDF4 block reader (identification/header blocks, channel/channel-group
//! metadata, VLSD records, and -- particularly -- the six CCBLOCK
//! conversion-rule types) from scratch would be a much larger undertaking
//! than adapting a working one. Writing, cutting, merging, the Python/JS
//! bindings, and the HTTP-range lazy-index reader are all dropped -- this
//! app only ever reads a complete in-memory byte buffer once.
//!
//! Two things needed patching beyond that trim:
//! - Every reader in the original crate is `#[cfg]`-gated between a native
//!   `memmap2`-backed path and a `wasm32` `Vec<u8>`-backed path. This app
//!   never has a filesystem path to open (browser bytes only), so that
//!   split served no purpose here -- collapsed to a single always-`Vec<u8>`
//!   path, which also means `cargo test` (native) exercises the exact same
//!   code as the real wasm32 build.
//! - The crate didn't handle `##HL` (history-list) or `##DZ` (zipped data)
//!   blocks at all -- see `blocks/history_list_block.rs` and
//!   `blocks/compressed_data_block.rs`. Compressed storage turned out to be
//!   the normal case for our own real instrument exports, not an edge case.

pub mod api;
pub mod blocks;
pub mod error;
pub mod parsing;
pub mod signal;

mod convert;
pub use convert::{convert_to_table, Mf4ConvertError};

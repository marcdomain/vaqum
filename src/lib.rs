//! vaqum's internals as a library. The `vaqum` binary (src/main.rs) is a
//! thin wrapper around these modules; the library also lets the fuzz
//! targets in `fuzz/` drive the archive readers directly.

pub mod archive;
pub mod bomb;
pub mod cli;
pub mod codec;
pub mod commands;
pub mod config;
pub mod crypto;
pub mod dedup;
pub mod exclude;
pub mod extract;
pub mod foreign;
pub mod format;
pub mod keys;
pub mod origin;
pub mod progress;
pub mod risk;
pub mod secret_store;
pub mod secrets;
pub mod sign;
pub mod util;

//! Platform-independent Octos domain logic.
//!
//! M1 only exposes version/build metadata. Manifest parsing, catalog, options
//! and session routing land here in M3; keeping them out of the Windows crates
//! is what makes them testable on Linux.

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn build_info() -> String {
    format!("octos {VERSION} ({})", std::env::consts::ARCH)
}

pub mod manifest;

pub mod catalog;

pub mod cli;

pub mod hooks;

pub mod options;

pub mod session;

pub mod storage_scope;

pub mod prefs;

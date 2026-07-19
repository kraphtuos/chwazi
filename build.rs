//! Stamps the build time into the binary as `BUILD_UNIX` (seconds since the
//! Unix epoch), read at compile time via `env!("BUILD_UNIX")`.
//!
//! With no `rerun-if-*` directives, Cargo re-runs this whenever a package file
//! changes — i.e. exactly when the crate is actually rebuilt — so the stamp
//! stays in step with the code without forcing needless recompiles.

use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=BUILD_UNIX={secs}");
}

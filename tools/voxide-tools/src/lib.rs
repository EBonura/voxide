//! VoXide dev tools. Each module backs one binary under `src/bin`.

use std::path::{Path, PathBuf};

pub mod inflate;
pub mod pack;
pub mod png;
pub mod profile;
pub mod sfx;
pub mod tapes;

/// The repository root (this crate sits in `tools/voxide-tools`).
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

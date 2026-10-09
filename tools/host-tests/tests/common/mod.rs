//! Shared helpers: find the repository root, make a scratch directory, and
//! compile and run a generated Rust program with the pinned toolchain.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The repository root (this crate sits in `tools/host-tests`).
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

/// A scratch directory removed when dropped.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(prefix: &str) -> Scratch {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!("{prefix}{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&path).expect("scratch directory");
        Scratch(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Compile `source` with `rustc` (run from `cwd`) and run the result with
/// `run_args`. Panics, with the compiler or program output, if either fails.
pub fn compile_and_run(
    scratch: &Scratch,
    source: &str,
    compile_args: &[&str],
    run_args: &[&str],
    cwd: &Path,
) -> Output {
    let program = scratch.0.join("program.rs");
    let binary = scratch.0.join("program");
    std::fs::write(&program, source).expect("write generated program");
    let compiled = Command::new("rustc")
        .args(compile_args)
        .arg(&program)
        .arg("-o")
        .arg(&binary)
        .current_dir(cwd)
        .output()
        .expect("run rustc");
    assert!(
        compiled.status.success(),
        "rustc failed:\n{}{}",
        String::from_utf8_lossy(&compiled.stdout),
        String::from_utf8_lossy(&compiled.stderr)
    );
    let ran = Command::new(&binary)
        .args(run_args)
        .output()
        .expect("run generated program");
    assert!(
        ran.status.success(),
        "generated program failed:\n{}{}",
        String::from_utf8_lossy(&ran.stdout),
        String::from_utf8_lossy(&ran.stderr)
    );
    ran
}

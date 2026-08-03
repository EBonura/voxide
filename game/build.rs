//! Inject PSoXide's PSX linker script into the final link, by absolute path
//! derived from this crate's location.

use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let repo_root = manifest.parent().expect("crate must live at <repo>/game");
    // .psoxide is hydrated by psoxide-link from the pin in psoxide-pin/, so
    // the linker script sits beside the SDK crates this links rather than in
    // a sibling checkout the layout had to guarantee.
    let psoxide = std::env::var("PSOXIDE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| repo_root.join(".psoxide"));
    let ld = psoxide.join("sdk/psoxide.ld");
    let ld = ld.canonicalize().unwrap_or(ld);

    println!("cargo:rustc-link-arg=-T{}", ld.display());
    println!("cargo:rustc-link-arg=--oformat=binary");
    println!("cargo:rerun-if-changed={}", ld.display());
}

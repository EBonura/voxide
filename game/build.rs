//! Inject PSoXide's PSX linker script into the final link, by absolute path
//! derived from this crate's location.

use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let repo_root = manifest.parent().expect("crate must live at <repo>/game");
    // .psoxide is imported from components.lock.json by `make psoxide`, so
    // the linker script sits beside the SDK crates this links rather than in
    // a sibling checkout the layout had to guarantee.
    let psoxide = std::env::var("PSOXIDE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| repo_root.join(".psoxide"));
    let ld = psoxide.join("sdk/psoxide.ld");
    let ld = ld.canonicalize().unwrap_or(ld);

    println!("cargo:rustc-link-arg=-T{}", ld.display());
    // `VOXIDE_LINK_ELF=1` keeps the ELF instead of the flat PSX-EXE, for
    // psoxide-pgo, which needs the DWARF to symbolize emulator PC samples.
    // Same code at the same addresses; it does not boot.
    println!("cargo:rerun-if-env-changed=VOXIDE_LINK_ELF");
    if std::env::var_os("VOXIDE_LINK_ELF").is_none() {
        println!("cargo:rustc-link-arg=--oformat=binary");
    }
    // Optional linker map (RAM headroom, PC attribution). Link-only: the
    // emitted bytes do not change.
    println!("cargo:rerun-if-env-changed=VOXIDE_LINK_MAP");
    if let Some(map) = std::env::var_os("VOXIDE_LINK_MAP").filter(|m| !m.is_empty()) {
        println!("cargo:rustc-link-arg=-Map={}", map.to_string_lossy());
    }
    println!("cargo:rerun-if-changed={}", ld.display());
}

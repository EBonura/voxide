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
    // `PSOXIDE_LINK_ELF=1` (set by the SDK's psoxide-pgo driver) keeps the ELF
    // instead of the flat PSX-EXE, so the driver can read its DWARF. Same code
    // at the same addresses; it does not boot. Cargo passes build-script link
    // arguments after rustflags, so the driver's own --oformat=elf cannot
    // override this one.
    println!("cargo:rerun-if-env-changed=PSOXIDE_LINK_ELF");
    if std::env::var_os("PSOXIDE_LINK_ELF").is_none() {
        println!("cargo:rustc-link-arg=--oformat=binary");
    }
    // Optional linker map (RAM headroom, PC attribution). Link-only: the
    // emitted bytes do not change.
    println!("cargo:rerun-if-env-changed=VOXIDE_LINK_MAP");
    if let Some(map) = std::env::var_os("VOXIDE_LINK_MAP").filter(|m| !m.is_empty()) {
        println!("cargo:rustc-link-arg=-Map={}", map.to_string_lossy());
    }
    // The per-frame face path (see world::chunk_faces) is linked first and
    // contiguous, so it shares the R3000's direct-mapped 4 KB I-cache with
    // nothing else it calls. The functions carry stable export names because
    // mangled names change with the checkout path.
    let order = manifest.join("hot-text.order");
    println!(
        "cargo:rustc-link-arg=--symbol-ordering-file={}",
        order.display()
    );
    println!("cargo:rerun-if-changed={}", order.display());
    println!("cargo:rerun-if-changed={}", ld.display());
}

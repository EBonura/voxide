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

    // RAM the program links for, as a `-T` file ahead of psoxide.ld (the way psoxide.ld:16-21
    // asks, so the link map records the value). 0x1FE000 puts STACK_INIT at 0x801FDF00 and leaves
    // 0x801FDF00..0x80200000 to the scene system's resident core window (8,192 B) and its 256 B
    // handoff block. PROVISIONAL: the window size comes from the scene-system plan (campaign
    // scene-system-1123); the final number comes after the core's first proof run.
    const RAM_SIZE_OVERRIDE: u32 = 0x1F_E000;
    let ram_size = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("ram-size.ld");
    std::fs::write(&ram_size, format!("RAM_SIZE_OVERRIDE = {RAM_SIZE_OVERRIDE:#X};\n"))
        .expect("write ram-size.ld");
    println!("cargo:rustc-link-arg=-T{}", ram_size.display());
    // `scene-core`: the boot splash runs from the SDK's boot-only memory (sdk/psoxide-boot.ld,
    // ahead of psoxide.ld): psx_splash's text and rodata are stored after .data and run above
    // .bss. Its sections run apart from where they are stored, so the link is an ELF and the
    // driver (psoxide-pgo) writes the EXE by load address.
    let boot = std::env::var_os("CARGO_FEATURE_SCENE_CORE").is_some();
    if boot {
        let boot_ld = psoxide.join("sdk/psoxide-boot.ld");
        let boot_ld = boot_ld.canonicalize().unwrap_or(boot_ld);
        println!("cargo:rustc-link-arg=-T{}", boot_ld.display());
        println!("cargo:rerun-if-changed={}", boot_ld.display());
    }
    println!("cargo:rustc-link-arg=-T{}", ld.display());
    // `PSOXIDE_LINK_ELF=1` (set by the SDK's psoxide-pgo driver) keeps the ELF
    // instead of the flat PSX-EXE, so the driver can read its DWARF. Same code
    // at the same addresses; it does not boot. Cargo passes build-script link
    // arguments after rustflags, so the driver's own --oformat=elf cannot
    // override this one.
    println!("cargo:rerun-if-env-changed=PSOXIDE_LINK_ELF");
    if std::env::var_os("PSOXIDE_LINK_ELF").is_none() && !boot {
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
    //
    // An `+order` PGO variant (psoxide-pgo) replaces it with the ordering file
    // it placed from the layout profile, named in PSOXIDE_LINK_ORDER for that
    // link only; ld.lld keeps just one ordering file, so it is one or the other.
    let order = manifest.join("hot-text.order");
    println!("cargo:rerun-if-env-changed=PSOXIDE_LINK_ORDER");
    match std::env::var_os("PSOXIDE_LINK_ORDER") {
        Some(pgo_order) => println!(
            "cargo:rustc-link-arg=--symbol-ordering-file={}",
            pgo_order.to_string_lossy()
        ),
        None => println!(
            "cargo:rustc-link-arg=--symbol-ordering-file={}",
            order.display()
        ),
    }
    println!("cargo:rerun-if-changed={}", order.display());
    println!("cargo:rerun-if-changed={}", ld.display());
}

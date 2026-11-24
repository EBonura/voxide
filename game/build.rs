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

    // VoXide runs on the scene core (SCENES.md): the scripts go in the order the core's ABI gives,
    // the RAM size, psoxide-core.ld, psoxide-boot.ld (the boot splash's boot-only memory), then
    // psoxide.ld. 0x1FE000 puts STACK_INIT at 0x801FDF00 and leaves 0x801FDF00..0x80200000 to the
    // core's window (8,192 B) and its 256 B handoff block. Sections run apart from where they are
    // stored, so the link is an ELF and the driver (psoxide-pgo) writes the EXE by load address.
    const RAM_SIZE_OVERRIDE: u32 = 0x1F_E000;
    let ram_size = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("ram-size.ld");
    // BOOT_STACK_OVERRIDE only lets the driver's plain link (psoxide-pgo apply and collect link the
    // unprofiled build first, to bind the profile) pass the .boot fit check: unprofiled, the image
    // leaves under 10 KB between .bss and the stack reserve and .boot (10.5 KB) does not fit there.
    // That plain image is never run for the splash. The shipping (profiled) image puts .boot at
    // __bss_end, far below the splash's stack: the map's __boot_end against the stack's deepest
    // point (STACK_INIT less about 39 KB, main's frame plus the splash's 0x5000) is the proof, and
    // `make compile`'s link map is where to read it.
    let mut overrides = format!("RAM_SIZE_OVERRIDE = {RAM_SIZE_OVERRIDE:#X};\nBOOT_STACK_OVERRIDE = 0x7000;\n");
    // A feature build (the gate disc's `receipts`, the labs) adds to .bss, and its unprofiled twin
    // (the driver sets PSOXIDE_LINK_ELF for it) no longer fits the image: that twin is only read to
    // bind the profile and is never run, so it gets a smaller stack reserve and boot stack. The
    // profiled gate image keeps the shipping layout.
    let featured = std::env::vars().any(|(name, _)| name.starts_with("CARGO_FEATURE_"));
    if std::env::var_os("PSOXIDE_LINK_ELF").is_some() && featured {
        // The lab and gate discs play no splash (no boot-only code, so no splash stack floor): their twin
        // can give up nearly the whole reserve.
        let lab = ["BLOCK_LAB", "MUSIC_LAB", "SAVE_LAB", "TORCH_LAB", "RECEIPTS"]
            .iter()
            .any(|name| std::env::var_os(format!("CARGO_FEATURE_{name}")).is_some());
        let (boot, reserve) = if lab { (0x1000, 0x1000) } else { (0x5000, 0x4000) };
        overrides = format!(
            "RAM_SIZE_OVERRIDE = {RAM_SIZE_OVERRIDE:#X};\nBOOT_STACK_OVERRIDE = {boot:#X};\nSTACK_RESERVE_OVERRIDE = {reserve:#X};\n"
        );
    }
    std::fs::write(&ram_size, overrides).expect("write ram-size.ld");
    println!("cargo:rustc-link-arg=-T{}", ram_size.display());
    for name in ["sdk/psoxide-core.ld", "sdk/psoxide-boot.ld"] {
        let script = psoxide.join(name);
        let script = script.canonicalize().unwrap_or(script);
        println!("cargo:rustc-link-arg=-T{}", script.display());
        println!("cargo:rerun-if-changed={}", script.display());
    }
    println!("cargo:rustc-link-arg=-T{}", ld.display());
    // Always an ELF (see above): `PSOXIDE_LINK_ELF`, which the driver sets, changes nothing here.
    // Optional linker map (RAM headroom, PC attribution). Link-only: the
    // emitted bytes do not change.
    println!("cargo:rerun-if-env-changed=PSOXIDE_LINK_ELF");
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

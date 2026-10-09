//! Host test for game/src/units.rs: compile its #[cfg(test)] module with the
//! repository's pinned toolchain (rustc --test, host target) and run it. Pins
//! the real-unit conversions every tuning constant goes through, e.g. 1 s == 60
//! sim ticks and 20 Java ticks == 60 sim ticks.

mod common;

#[test]
fn units_rs() {
    let root = common::root();
    let scratch = common::Scratch::new("vox-units-");
    let units = root.join("game/src/units.rs");
    let source = std::fs::read_to_string(&units).expect("units.rs");
    // `rustc --test` on the file itself, as the module is self-contained.
    let output = common::compile_and_run(&scratch, &source, &["--edition", "2021", "--test"], &[], &root);
    assert!(String::from_utf8_lossy(&output.stdout).contains("test result: ok"));
}

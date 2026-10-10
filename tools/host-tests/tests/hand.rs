//! Host test for game/src/hand.rs: compile its #[cfg(test)] module with the
//! repository's pinned toolchain (rustc --test, host target) and run it. Pins
//! the pad grammar every container screen shares: X takes and places, SQUARE
//! halves and places one, TRIANGLE quick moves, CIRCLE puts back or closes,
//! and no state gives X two meanings.

mod common;

#[test]
fn hand_rs() {
    let root = common::root();
    let scratch = common::Scratch::new("vox-hand-");
    let hand = root.join("game/src/hand.rs");
    let source = std::fs::read_to_string(&hand).expect("hand.rs");
    // `rustc --test` on the file itself, as the module is self-contained.
    let output = common::compile_and_run(
        &scratch,
        &source,
        &["--edition", "2021", "--test"],
        &[],
        &root,
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("test result: ok"));
}

//! Compile the game's TNT admission path with a full entity pool.

mod common;

fn source_fn(source: &str, signature: &str) -> String {
    let start = source.find(signature).expect("game function");
    let open = source[start..].find('{').expect("function body") + start;
    let mut depth = 0;
    for (offset, byte) in source.as_bytes()[open..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return source[start..=open + offset].to_owned();
                }
            }
            _ => {}
        }
    }
    panic!("unterminated game function {signature}");
}

#[test]
fn full_tnt_pool_keeps_the_block_and_reset_clears_kick() {
    let root = common::root();
    let source = std::fs::read_to_string(root.join("game/src/tnt.rs")).expect("TNT source");
    let mut program = String::from(
        r#"
const TNT: u8 = 24;
const AIR: u8 = 0;
const BLOCK: i32 = 64;
const MAX_TNT: usize = 32;
const KICK_SIDE_Q8: i32 = 1;
const KICK_UP_Q8: i32 = 1;
static mut USED: [bool; MAX_TNT] = [false; MAX_TNT];
static mut LIVE: u8 = 0;
static mut X: [i32; MAX_TNT] = [0; MAX_TNT];
static mut Y: [i32; MAX_TNT] = [0; MAX_TNT];
static mut Z: [i32; MAX_TNT] = [0; MAX_TNT];
static mut FX: [i32; MAX_TNT] = [0; MAX_TNT];
static mut FY: [i32; MAX_TNT] = [0; MAX_TNT];
static mut FZ: [i32; MAX_TNT] = [0; MAX_TNT];
static mut VX: [i32; MAX_TNT] = [0; MAX_TNT];
static mut VY: [i32; MAX_TNT] = [0; MAX_TNT];
static mut VZ: [i32; MAX_TNT] = [0; MAX_TNT];
static mut FUSE: [u16; MAX_TNT] = [0; MAX_TNT];
static mut PENDING: [bool; MAX_TNT] = [false; MAX_TNT];
static mut GROUND: [bool; MAX_TNT] = [false; MAX_TNT];
static mut LOST: [u8; MAX_TNT] = [0; MAX_TNT];
static mut CELLS: [u8; 64] = [AIR; 64];
static mut EDITS: usize = 0;
static mut KICK: (i32, i32, i32) = (0, 0, 0);
fn rnd() -> u32 { 0 }
mod sincos {
    pub fn sin_q12(_: u16) -> i32 { 0 }
    pub fn cos_q12(_: u16) -> i32 { 4096 }
}
mod sfx { pub fn sapper_hiss() {} }
fn get_block_i32(x: i32, _: i32, _: i32) -> u8 { unsafe { CELLS[x as usize] } }
fn set_block_i32(x: i32, _: i32, _: i32, b: u8) { unsafe { CELLS[x as usize] = b } }
fn record_edit(_: i32, _: i32, _: i32, _: u8) { unsafe { EDITS += 1 } }
#[test]
fn no_slot_keeps_tnt_and_reset_drops_pending_kick() {
    unsafe { CELLS[40] = TNT; }
    for i in 0..MAX_TNT { assert!(spawn(100 + i as i32, 2, 0, 240)); }
    assert_eq!(unsafe { LIVE }, 32);
    assert!(!prime(40, 2, 0, 240));
    assert_eq!(get_block_i32(40, 2, 0), TNT);
    assert_eq!(unsafe { (LIVE, EDITS) }, (32, 0));

    release(0);
    assert!(prime(40, 2, 0, 240));
    assert_eq!(get_block_i32(40, 2, 0), AIR);
    assert_eq!(unsafe { (LIVE, EDITS) }, (32, 1));
    assert_eq!(unsafe { FUSE[0] }, 240);

    unsafe { KICK = (10, -5, 2); }
    reset();
    assert_eq!(unsafe { KICK }, (0, 0, 0));
    unsafe { CELLS[41] = TNT; EDITS = 0; }
    for i in 0..MAX_TNT { assert!(spawn(100 + i as i32, 2, 0, 240)); }
    assert!(!prime(41, 2, 0, 240));
    assert_eq!(get_block_i32(41, 2, 0), TNT);
    assert_eq!(unsafe { (LIVE, EDITS) }, (32, 0));
}
"#,
    );
    for signature in [
        "fn release(",
        "pub fn reset(",
        "pub fn prime(",
        "pub fn spawn(",
    ] {
        program.push_str(&source_fn(&source, signature));
        program.push('\n');
    }
    let scratch = common::Scratch::new("vox-tnt-capacity-");
    let output = common::compile_and_run(
        &scratch,
        &program,
        &["--edition", "2021", "--test"],
        &["--test-threads=1"],
        &root,
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

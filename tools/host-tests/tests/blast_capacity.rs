//! Compile the blast column's real capacity branch against one loaded cell.

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
fn saturated_chain_keeps_tnt_without_blocking_queue_drain() {
    let root = common::root();
    let source = std::fs::read_to_string(root.join("game/src/world.rs")).expect("world source");
    let mut program = String::from(
        r#"
const AIR: u8 = 0;
const TNT: u8 = 24;
const CHEST: u8 = 25;
const FURNACE: u8 = 26;
const CH: i32 = 64;
const CW: i32 = 1;
const CWU: usize = 1;
const BLOCK: i32 = 64;
struct BlastJob { cx: i32, cy: i32, cz: i32, power: i32, seed: u32 }
struct Cube;
impl Cube { fn column_mask(&self, _: usize) -> u32 { 1 } }
static mut BCUBE: Cube = Cube;
static mut BJ_COL: usize = 0;
static mut BJ_N: u32 = 0;
static mut BJ_LOG: bool = true;
static mut BJ_LO: (i32, i32) = (i32::MAX, i32::MAX);
static mut BJ_HI: (i32, i32) = (i32::MIN, i32::MIN);
static mut CELL: u8 = TNT;
static mut EDITS: usize = 0;
static mut CAN_SPAWN: bool = false;
static mut ACCEPTED: usize = 0;
static mut QUEUED_BLASTS: usize = 8;
static mut MESH_SCRATCH_OWNER: usize = usize::MAX;
static mut MESH_SCRATCH: [u8; 64] = [AIR; 64];
#[derive(Copy, Clone)]
struct Chunk { loaded: bool, cx: i32, cz: i32 }
static mut CHUNKS: [Chunk; 1] = [Chunk { loaded: true, cx: 0, cz: 0 }];
mod blast {
    pub const SIDE: usize = 1;
    pub const REACH_CELLS: i32 = 0;
    pub fn chain_fuse_ticks(_: u32, _: u32) -> i32 { 10 }
    pub fn drops(_: u32, _: u32, _: i32) -> bool { false }
}
mod units { pub fn java_ticks(n: i32) -> i32 { n * 3 } }
mod slack { pub fn room() -> bool { true } }
mod tnt {
    pub fn spawn(_: i32, _: i32, _: i32, _: i32) -> bool {
        unsafe {
            if !super::CAN_SPAWN { return false; }
            super::ACCEPTED += 1;
            true
        }
    }
}
mod rle {
    pub fn rset_mask(_: usize, _: usize, _: u64, _: u8) -> bool {
        unsafe { super::CELL = super::AIR; }
        true
    }
}
fn get_span(_: i32, _: i32, _: i32, col: &mut [u8; blast::SIDE]) { col[0] = unsafe { CELL }; }
fn get(_: i32, _: i32, _: i32) -> u8 { AIR }
fn floor_div(n: i32, d: i32) -> i32 { n / d }
fn slot(_: i32, _: i32) -> usize { 0 }
fn lidx(_: usize, _: usize, _: usize) -> usize { 0 }
fn col_masks_set(_: usize, _: u8) {}
fn is_fluid(_: u8) -> bool { false }
fn wake_fluid(_: i32, _: i32, _: i32) {}
fn chest_remove(_: i32, _: i32, _: i32) {}
fn furn_remove(_: i32, _: i32, _: i32) {}
fn spawn_drop(_: i32, _: i32, _: i32, _: u8, _: u32) {}
fn record_edit(_: i32, _: i32, _: i32, _: u8) { unsafe { EDITS += 1 } }

#[test]
fn full_pool_and_queue_keep_the_cell_and_finish_active_job() {
    let job = BlastJob { cx: 0, cy: 1, cz: 0, power: 4, seed: 9 };
    assert!(!blast_break(&job));
    assert_eq!(unsafe { (CELL, EDITS, ACCEPTED, QUEUED_BLASTS, BJ_COL, BJ_N) },
               (TNT, 0, 0, 8, 1, 0));
    // A later blast can revisit the still-solid cell after the queue drains.
    unsafe { QUEUED_BLASTS = 7; CAN_SPAWN = true; BJ_COL = 0; }
    assert!(!blast_break(&job));
    assert_eq!(unsafe { (CELL, EDITS, ACCEPTED, BJ_COL, BJ_N) }, (AIR, 1, 1, 1, 1));
    unsafe { BJ_COL = 0; }
    assert!(!blast_break(&job));
    assert_eq!(unsafe { (CELL, EDITS, ACCEPTED, BJ_N) }, (AIR, 1, 1, 1));
}
"#,
    );
    for signature in ["fn blast_break(", "fn blast_column("] {
        program.push_str(&source_fn(&source, signature));
        program.push('\n');
    }
    let scratch = common::Scratch::new("vox-blast-capacity-");
    let output = common::compile_and_run(
        &scratch,
        &program,
        &["--edition", "2021", "--test"],
        &["--test-threads=1"],
        &root,
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

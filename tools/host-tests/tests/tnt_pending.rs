//! Compile the game's TNT expiry path to test queue backpressure and cleanup.

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
fn expired_tnt_survives_full_queue_and_void_risk_until_one_admission() {
    let root = common::root();
    let source = std::fs::read_to_string(root.join("game/src/tnt.rs")).expect("TNT source");
    let mut program = String::from(
        r#"
const MAX_TNT: usize = 32;
const BLOCK: i32 = 64;
const HALF_W: i32 = 31;
const HEIGHT: i32 = 62;
const GRAVITY_Q8: i32 = 1;
const DRAG_65536: i32 = 0;
const GROUND_FRICTION_256: i32 = 227;
const DETONATE_COST: u32 = 200_000;
const LOST_TICKS: u8 = 120;
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
static mut PEND_X: [i32; MAX_TNT] = [0; MAX_TNT];
static mut PEND_Y: [i32; MAX_TNT] = [0; MAX_TNT];
static mut PEND_Z: [i32; MAX_TNT] = [0; MAX_TNT];
static mut GROUND: [bool; MAX_TNT] = [false; MAX_TNT];
static mut LOST: [u8; MAX_TNT] = [0; MAX_TNT];
static mut QUEUE_FULL: bool = true;
static mut LOADED: bool = true;
static mut ADMITTED: usize = 0;
static mut BLAST_AT: (i32, i32, i32) = (0, 0, 0);
mod world {
    pub fn column_loaded(_: i32, _: i32) -> bool { unsafe { super::LOADED } }
}
mod slack {
    pub fn room_for(_: u32) -> bool { true }
}
fn world_to_block_x(n: i32) -> i32 { n / BLOCK }
fn world_to_block_z(n: i32) -> i32 { n / BLOCK }
fn aabb_collides_dims(_: i32, _: i32, _: i32, _: i32, _: i32) -> bool { false }
fn spawn_particles(_: i32, _: i32, _: i32, _: (u8,u8,u8), _: i32, _: u32, _: i32) {}
fn detonate_at(x: i32, y: i32, z: i32, _: usize) -> bool {
    unsafe {
        if QUEUE_FULL { return false; }
        BLAST_AT = (x, y, z);
        ADMITTED += 1;
        true
    }
}
#[test]
fn expiry_position_is_held_and_admitted_once() {
    unsafe {
        USED[0] = true; LIVE = 1;
        X[0] = 640; Y[0] = 3200; Z[0] = 640;
        FUSE[0] = 2;
    }
    step(0); // penultimate movement and decrement take the fuse to one
    assert_eq!(unsafe { FUSE[0] }, 1);
    assert!(!unsafe { PENDING[0] });
    step(0); // ordinary final movement precedes the first admission attempt
    assert!(unsafe { PENDING[0] });
    let expiry = unsafe { (PEND_X[0], PEND_Y[0] + 4, PEND_Z[0]) };
    unsafe {
        LOADED = false;
        Y[0] = -1000; // a later displacement must not move the blast
    }
    for _ in 0..125 { step(0); }
    assert_eq!(unsafe { (USED[0], LIVE, LOST[0], ADMITTED) }, (true, 1, 0, 0));
    assert_eq!(unsafe { (PEND_X[0], PEND_Y[0] + 4, PEND_Z[0]) }, expiry);
    unsafe { QUEUE_FULL = false; }
    step(0);
    assert_eq!(unsafe { (USED[0], LIVE, ADMITTED, BLAST_AT) }, (false, 0, 1, expiry));
    assert!(!unsafe { PENDING[0] });
    // With a free queue, fuse-one TNT still moves once before detonating.
    unsafe {
        LOADED = true;
        USED[2] = true; LIVE = 1; FUSE[2] = 1;
        X[2] = 640; Y[2] = 3200; Z[2] = 640; VY[2] = 512;
    }
    step(2);
    assert_eq!(unsafe { (USED[2], LIVE, ADMITTED, BLAST_AT) },
               (false, 0, 2, (640, 3205, 640)));
    // Ordinary pre-expiry unloaded entities still leave after two seconds.
    unsafe { LOADED = false; USED[1] = true; LIVE = 1; FUSE[1] = 40; Y[1] = 3200; }
    for _ in 0..121 { step(1); }
    assert_eq!(unsafe { (USED[1], LIVE, ADMITTED) }, (false, 0, 2));
}
"#,
    );
    for signature in [
        "fn release(",
        "fn mark_pending(",
        "fn try_pending(",
        "fn step(",
    ] {
        program.push_str(&source_fn(&source, signature));
        program.push('\n');
    }
    let scratch = common::Scratch::new("vox-tnt-pending-");
    let output = common::compile_and_run(
        &scratch,
        &program,
        &["--edition", "2021", "--test"],
        &["--test-threads=1"],
        &root,
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

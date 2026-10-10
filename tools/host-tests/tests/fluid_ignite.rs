//! Compile the game's fluid wake and ignition functions against a small world.
//! The full queue case reproduces the fire lab's lava beside TNT failure.

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
fn lava_contact_ignites_even_when_fluid_queue_is_full() {
    let root = common::root();
    let source = std::fs::read_to_string(root.join("game/src/world.rs")).expect("world source");
    let queue_start = source
        .find("const FLUID_Q: usize")
        .expect("queue constants");
    let queue_end = source.find("fn fq_push(").expect("queue push");
    let mut program = String::from(
        r#"
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const AIR: u8 = 0;
const TNT: u8 = 1;
const LAVA: u8 = 2;
const PLANK: u8 = 3;
const FIRE: u8 = 4;

static CELLS: OnceLock<Mutex<HashMap<(i32, i32, i32), u8>>> = OnceLock::new();
static IGNITIONS: OnceLock<Mutex<Vec<(i32, i32, i32, i32)>>> = OnceLock::new();

fn cells() -> &'static Mutex<HashMap<(i32, i32, i32), u8>> {
    CELLS.get_or_init(|| Mutex::new(HashMap::new()))
}
fn ignitions() -> &'static Mutex<Vec<(i32, i32, i32, i32)>> {
    IGNITIONS.get_or_init(|| Mutex::new(Vec::new()))
}
fn get(x: i32, y: i32, z: i32) -> u8 {
    *cells().lock().unwrap().get(&(x, y, z)).unwrap_or(&AIR)
}
fn set(x: i32, y: i32, z: i32, b: u8) {
    cells().lock().unwrap().insert((x, y, z), b);
}
fn is_lava(b: u8) -> bool { b == LAVA }
fn is_flammable(b: u8) -> bool { b == TNT || b == PLANK }
fn light_fire_at(x: i32, y: i32, z: i32) -> bool {
    if get(x, y, z) != AIR { return false; }
    set(x, y, z, FIRE);
    true
}
mod tnt {
    pub const FUSE_FULL: i32 = 240;
    pub fn prime(x: i32, y: i32, z: i32, fuse: i32) {
        assert_eq!(super::get(x, y, z), super::TNT);
        super::set(x, y, z, super::AIR);
        super::ignitions().lock().unwrap().push((x, y, z, fuse));
    }
}
fn reset() {
    cells().lock().unwrap().clear();
    ignitions().lock().unwrap().clear();
    unsafe { FQ_HEAD = 0; FQ_LEN = 0; }
}
fn fill_queue() {
    for i in 0..FLUID_Q { fq_push(i as i32 + 1000, 2, 3); }
    assert_eq!(unsafe { FQ_LEN }, FLUID_Q);
}

#[test]
fn lava_placement_primes_neighbour_despite_full_queue() {
    reset();
    fill_queue();
    set(-1, 4, 0, TNT);
    set(0, 4, 0, LAVA);
    wake_fluid(0, 4, 0);
    assert_eq!(ignitions().lock().unwrap().as_slice(), &[(-1, 4, 0, tnt::FUSE_FULL)]);
    assert_eq!(get(-1, 4, 0), AIR);
    assert_eq!(unsafe { FQ_LEN }, FLUID_Q);
}

#[test]
fn tnt_placement_next_to_lava_primes_despite_full_queue() {
    reset();
    fill_queue();
    set(0, 4, 0, LAVA);
    set(-1, 4, 0, TNT);
    wake_fluid(-1, 4, 0);
    assert_eq!(ignitions().lock().unwrap().as_slice(), &[(-1, 4, 0, tnt::FUSE_FULL)]);
}

#[test]
fn dry_tnt_does_not_prime_and_lava_lights_fuel() {
    reset();
    fill_queue();
    set(-1, 4, 0, TNT);
    wake_fluid(-1, 4, 0);
    assert!(ignitions().lock().unwrap().is_empty());
    set(1, 4, 0, PLANK);
    set(0, 4, 0, LAVA);
    wake_fluid(0, 4, 0);
    assert_eq!(get(1, 5, 0), FIRE);
    assert_eq!(ignitions().lock().unwrap().as_slice(), &[(-1, 4, 0, tnt::FUSE_FULL)]);
}
"#,
    );
    program.push_str(&source[queue_start..queue_end]);
    for signature in ["fn fq_push(", "pub fn wake_fluid(", "fn try_ignite("] {
        program.push_str(&source_fn(&source, signature));
        program.push('\n');
    }
    let scratch = common::Scratch::new("vox-fluid-ignite-");
    let output = common::compile_and_run(
        &scratch,
        &program,
        &["--edition", "2021", "--test"],
        &["--test-threads=1"],
        &root,
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("3 passed"));
}

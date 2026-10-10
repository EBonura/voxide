//! Compile the sapper's actual blast admission and effect ordering.

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
fn full_blast_queue_keeps_sapper_and_defers_all_effects() {
    let root = common::root();
    let source = std::fs::read_to_string(root.join("game/src/mob.rs")).expect("mob source");
    let mut program = String::from(
        r#"
const SAPPER_POWER: i32 = 3;
const BLOCK: i32 = 64;
const FUSE_MAX: u16 = 90;
#[derive(Copy, Clone, PartialEq, Debug)]
struct Mob { alive: bool, x: i32, y: i32, z: i32, charged: bool, fuse: u16 }
const DEAD: Mob = Mob { alive: false, x: 0, y: 0, z: 0, charged: false, fuse: 0 };
static mut MOBS: [Mob; 1] = [Mob { alive: true, x: 64, y: 64, z: 64, charged: false, fuse: FUSE_MAX }];
static mut QUEUE_FULL: bool = true;
static mut QUEUED: usize = 0;
static mut PARTICLES: usize = 0;
static mut EFFECTS: usize = 0;
mod world {
    pub fn explode(_: i32, _: i32, _: i32, _: i32, _: u32) -> bool {
        unsafe {
            if super::QUEUE_FULL { return false; }
            super::QUEUED += 1;
        }
        true
    }
}
mod tnt {
    pub fn explosion_effects(_: i32, _: i32, _: i32, _: i32, _: usize) {
        unsafe { super::EFFECTS += 1; }
    }
}
fn spawn_particles(_: i32, _: i32, _: i32, _: (u8, u8, u8), _: i32, _: u32, _: i32) {
    unsafe { PARTICLES += 1; }
}

#[test]
fn rejection_then_acceptance_applies_effects_once() {
    finish_sapper(0, unsafe { MOBS[0] });
    assert_eq!(unsafe { (MOBS[0].alive, MOBS[0].fuse, QUEUED, PARTICLES, EFFECTS) },
               (true, FUSE_MAX, 0, 0, 0));
    unsafe { QUEUE_FULL = false; }
    finish_sapper(0, unsafe { MOBS[0] });
    assert_eq!(unsafe { (MOBS[0].alive, QUEUED, PARTICLES, EFFECTS) }, (false, 1, 1, 1));
}
"#,
    );
    program.push_str(&source_fn(&source, "fn explode(i: usize,"));
    program.push_str(&source_fn(&source, "fn finish_sapper("));
    let scratch = common::Scratch::new("vox-sapper-capacity-");
    let output = common::compile_and_run(
        &scratch,
        &program,
        &["--edition", "2021", "--test"],
        &["--test-threads=1"],
        &root,
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

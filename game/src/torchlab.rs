//! Headless proving ground for torch light stopped by walls (`--features torch-lab`, never
//! shipped). Each SELECT does the next thing of three scenes, built on a flat stone floor at
//! midnight near the spawn:
//!
//! 1. A pit under a floor of stone: the first SELECT builds it and stands the player on the
//!    ground beside it; the next puts a torch in it (the cover is sealed: the ground round it must
//!    stay dark); the next takes one cover block away (the light comes out of the hole, and only
//!    there).
//! 2. Two tunnels side by side in a block of stone with two blocks of rock between them, open at
//!    one end: the player stands at the mouth of the second, then a torch goes into the first (the
//!    second must stay as it was), then the player stands at the mouth of the first (lit).
//! 3. A room with a doorway: a torch inside lights the ground in front of the door and not the
//!    ground beside the walls; the last view is from in front of the door.
//!
//! The scenes are built with raw sets (a few thousand cells), but the torches and the hole are
//! put in with `world::set`, the path a player's edit takes, so the light of a torch is worked out
//! the way it is in play. The journeys (tests/torch-pit.toml, torch-tunnels.toml, torch-door.toml)
//! press SELECT and read the stage and whether the world has settled (the meshes and the torch
//! lights are up to date) from VOXIDE_GATE_STATE word 4, `stage << 1 | settled`.

use crate::bstate;
use crate::*;

/// Mobs stand still while the lab runs (mob.rs reads it): a zombie at midnight in front of the
/// camera would be a difference between two frames that is not the light.
pub static mut QUIET: bool = false;

/// Cycle probes, in counter ticks of root counter 2 (eight cycles each), reset by the stages that
/// measure. Runs of lighting setup of a mesh (`build_sky_top`) [count, longest], torch floods
/// [count, longest], and the most the torch lights or-ed in during one lighting setup [longest].
/// Shown in VOXIDE_GATE_STATE words 5, 6, 8, 15 and 18. The longest, not the sum: the world goes
/// on remeshing (water running, a tree growing) while a stage is measured.
pub static mut PROBE: [u32; 5] = [0; 5];
/// The or-ing in of torch lights since the lighting setup in flight began.
pub static mut APPLY_ACC: u32 = 0;

/// The counter, for a probe.
pub fn clock() -> u16 {
    psx_io::timers::counter(psx_io::timers::Timer::Timer2)
}

/// Add one more run of `slot` (a count and a longest) that began at `since`.
pub fn note(slot: usize, since: u16) {
    unsafe {
        PROBE[slot] += 1;
        let dt = clock().wrapping_sub(since) as u32;
        PROBE[slot + 1] = PROBE[slot + 1].max(dt);
    }
}

/// A lighting setup ended: the torch lights it or-ed in took `APPLY_ACC`.
pub fn end_setup() {
    unsafe {
        PROBE[4] = PROBE[4].max(APPLY_ACC);
        APPLY_ACC = 0;
    }
}

/// Fold the probes into the gate state words (the snapshot calls it).
pub fn publish(words: &mut [u32; 22]) {
    unsafe {
        words[5] = PROBE[0];
        words[6] = PROBE[1];
        words[8] = PROBE[2];
        words[15] = PROBE[3];
        words[18] = PROBE[4];
    }
}

static mut STAGE: u32 = 0;
/// The torches put in, so the next scene takes them out of the edit store (the arena itself is
/// built with raw sets, which the store does not see).
static mut PLACED: [(i32, i32, i32); 40] = [(0, 0, 0); 40];
static mut NPLACED: usize = 0;
/// The middle of the arena (x, z) and the floor's top y.
static mut MID: (i32, i32, i32) = (0, 0, 0);

/// Half the width of the arena: the floor is 29 x 29 cells.
const HALF: i32 = 14;

/// What VOXIDE_GATE_STATE word 4 shows: the stage done and whether the world has caught up.
pub fn gate_word() -> u32 {
    let settled = world::idle() as u32;
    unsafe { STAGE << 1 | settled }
}

fn put(x: i32, y: i32, z: i32, b: u8) {
    world::set_raw_pub(x, y, z, b);
}

/// A player's edit: `set` and the edit store, as placing or breaking a block does.
fn edit(x: i32, y: i32, z: i32, b: u8) {
    world::set(x, y, z, b);
    record_edit(x, y, z, b);
}

/// Cells (`x0..=x1`, `y0..=y1`, `z0..=z1`) relative to the middle of the arena and its floor.
fn fill(x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32, b: u8) {
    let (mx, my, mz) = unsafe { MID };
    let mut x = x0;
    while x <= x1 {
        let mut z = z0;
        while z <= z1 {
            let mut y = y0;
            while y <= y1 {
                put(mx + x, my + y, mz + z, b);
                y += 1;
            }
            z += 1;
        }
        x += 1;
    }
}

/// The empty arena: a stone floor four cells deep, air above. The torches of the last scene are
/// taken out first, as a player would.
fn arena() {
    unsafe {
        while NPLACED > 0 {
            NPLACED -= 1;
            let (x, y, z) = PLACED[NPLACED];
            edit(x, y, z, AIR);
        }
    }
    fill(-HALF, -4, -HALF, HALF, 0, HALF, COBBLE);
    fill(-HALF, 1, -HALF, HALF, 10, HALF, AIR);
}

fn torch() -> u8 {
    bstate::make(TORCH, 0)
}

/// Look from the player's eye at the middle of a cell: set yaw and pitch so the crosshair is on it
/// (blocklab.rs does the same).
fn aim(p: &mut Player, tx: i32, ty: i32, tz: i32) {
    let ex = p.x;
    let ez = p.z;
    let ey = p.y + EYE_HEIGHT;
    let (px, py, pz) = (
        block_to_world_x(tx) + BLOCK / 2,
        ty * BLOCK + BLOCK / 2,
        block_to_world_z(tz) + BLOCK / 2,
    );
    let (dx, dy, dz) = (px - ex, py - ey, pz - ez);
    let mut best = (i32::MIN, 0u16);
    let mut a = 0u16;
    while a < 4096 {
        let d = sincos::sin_q12(a) * dx + sincos::cos_q12(a) * dz;
        if d > best.0 {
            best = (d, a);
        }
        a += 4;
    }
    p.yaw = best.1;
    let h = psx_math::int32::isqrt_i32(dx * dx + dz * dz).max(1);
    let mut best = (i32::MAX, 0i32);
    let mut q = -1024i32;
    while q <= 1024 {
        let a = (q & 0x0FFF) as u16;
        let e = (sincos::sin_q12(a) * h - sincos::cos_q12(a) * dy).abs();
        if e < best.0 {
            best = (e, q);
        }
        q += 2;
    }
    p.pitch = best.1 as i16;
}

/// Stand at (`sx`, `sz`) on the floor and look at (`tx`, `ty`, `tz`), all relative to the middle
/// of the arena and its floor.
fn stand(p: &mut Player, sx: i32, sz: i32, tx: i32, ty: i32, tz: i32) {
    let (mx, my, mz) = unsafe { MID };
    p.x = block_to_world_x(mx + sx) + BLOCK / 2;
    p.z = block_to_world_z(mz + sz) + BLOCK / 2;
    p.y = (my + 1) * BLOCK;
    p.vy = 0;
    aim(p, mx + tx, my + ty, mz + tz);
}

/// The pit: a chamber of 3 x 3 x 2 cells inside the floor, under its top layer of stone.
fn build_pit() {
    arena();
    fill(-1, -2, -1, 1, -1, 1, AIR);
}

/// Two tunnels along x in a block of stone, two cells high, at z = -2 and z = 1, with two cells
/// of rock between them; open on the -x face, closed at the other end.
fn build_tunnels() {
    arena();
    fill(-10, 1, -8, 12, 5, 8, STONE);
    fill(-10, 1, -2, 11, 2, -2, AIR);
    fill(-10, 1, 1, 11, 2, 1, AIR);
}

/// A room of 7 x 3 x 7 cells with a roof, walls one cell thick, and a doorway two cells high in
/// the +x wall.
fn build_room() {
    arena();
    fill(-4, 1, -4, 4, 4, 4, STONE);
    fill(-3, 1, -3, 3, 3, 3, AIR);
    fill(4, 1, 0, 4, 2, 0, AIR);
}

/// Do the next thing. Takes the world clock to put it at midnight at the first.
pub fn select(p: &mut Player, day: &mut u32) {
    unsafe {
        let stage = STAGE;
        STAGE += 1;
        // Midnight, as the cheat does, at every step: the frames of one scene are told apart by
        // the torch alone, not by the sky having moved on between them.
        let target = 18_000 * DAY_LEN / 24_000;
        *day += (target + DAY_LEN - *day % DAY_LEN) % DAY_LEN;
        if stage == 0 {
            QUIET = true;
            let (bx, bz) = (world_to_block_x(p.x), world_to_block_z(p.z));
            // The middle of the chunk the player is in: the arena is inside the meshed chunks.
            let (mx, mz) = (floor_div(bx, 16) * 16 + 8, floor_div(bz, 16) * 16 + 8);
            let mut y = world::surface_y(mx, mz);
            let mut k = 0;
            while k < 4 {
                let (cx, cz) = (mx + HALF * (k & 1) * 2 - HALF, mz + HALF * (k >> 1) * 2 - HALF);
                y = y.max(world::surface_y(cx, cz));
                k += 1;
            }
            MID = (mx, y, mz);
        }
        match stage {
            0 => {
                build_pit();
                world::remesh_loaded();
                stand(p, 6, 0, 0, 0, 0);
            }
            1 => torch_at(0, -2, 0),
            2 => {
                let (mx, my, mz) = MID;
                edit(mx, my, mz, AIR);
            }
            3 => {
                build_tunnels();
                world::remesh_loaded();
                stand(p, -13, -2, 0, 1, -2);
            }
            4 => stand(p, -13, 1, 0, 1, 1),
            5 => torch_at(0, 1, -2),
            6 => stand(p, -13, -2, 0, 1, -2),
            7 => {
                build_room();
                world::remesh_loaded();
                stand(p, 10, 7, 3, 1, 0);
            }
            8 => torch_at(0, 1, 0),
            9 => stand(p, 12, 0, 5, 1, 0),
            // The cost stages. One torch on the open floor, a block of the floor broken beside it,
            // thirty-one more torches, and a remesh of the world with all thirty-two.
            10 => {
                arena();
                world::remesh_loaded();
                stand(p, 0, 12, 0, 1, 0);
            }
            11 => {
                PROBE = [0; 5];
                torch_at(0, 1, 0);
            }
            12 => {
                PROBE = [0; 5];
                edit_at(1, 0, 1, AIR);
            }
            13 => {
                PROBE = [0; 5];
                let mut k = 1;
                while k < 32 {
                    torch_at(-7 + (k % 8) * 2, 1, -3 + (k / 8) * 2);
                    k += 1;
                }
            }
            14 => {
                PROBE = [0; 5];
                world::remesh_loaded();
            }
            _ => {}
        }
    }
}

fn edit_at(x: i32, y: i32, z: i32, b: u8) {
    let (mx, my, mz) = unsafe { MID };
    edit(mx + x, my + y, mz + z, b);
}

/// A torch put in, like a player's.
fn torch_at(x: i32, y: i32, z: i32) {
    let (mx, my, mz) = unsafe { MID };
    edit(mx + x, my + y, mz + z, torch());
    unsafe {
        if NPLACED < PLACED.len() {
            PLACED[NPLACED] = (mx + x, my + y, mz + z);
            NPLACED += 1;
        }
    }
}

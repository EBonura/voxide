//! Fire and TNT lab (feature `fire-lab`, never shipped): a scripted timeline
//! that builds a flat arena over the spawn and runs the fire and explosion
//! scenarios one after another, so the frames before, during and after each
//! can be captured headless (`--route-screenshot-dir`) and the frame cadence
//! read from a route log.
//!
//! The script runs on the sim-step clock. The player stands at the arena
//! origin looking along +Z with full health; the scenes are built 8 to 10
//! blocks ahead. Phase starts, in sim steps (60 a second):
//!  single TNT, a 12 block chain, a 27 block cube, fire beside TNT, a burning
//!  hut, and lava beside TNT.

use crate::*;

/// What the gates read (`frontend launch --route-watch-u32`, the address from
/// the link map): step, lit TNT blocks, smallest fuse, flames, blasts pending,
/// TNT blocks standing, solid cells in the show area, planks/logs/leaves/wool
/// standing, and the player's health. Words 6 and 7 are counted every 20
/// steps; the rest every step.
#[no_mangle]
pub static mut VOXIDE_LAB_FIRE: [i32; 10] = [0; 10];
static mut COUNTS: (i32, i32, i32) = (0, 0, 0);
static mut T: i32 = 0; // sim steps since gameplay began
static mut ORG: (i32, i32) = (0, 0); // arena origin, block coords

const BY: i32 = 50; // platform block y; you stand at (BY + 1) * BLOCK

const P_SINGLE: i32 = 30;
const P_CHAIN: i32 = 700;
const P_CUBE: i32 = 1700;
const P_FIRE_TNT: i32 = 2800;
const P_HUT: i32 = 4000;
const P_LAVA_TNT: i32 = 7000;
const P_END: i32 = 8400;

fn put(x: i32, y: i32, z: i32, b: u8) {
    world::set_raw_pub(x, y, z, b);
}

/// Clear and refloor the show area: 25 wide, from 3 behind the player to 22
/// ahead, three deep in dirt on a stone bed, 14 of air above.
fn arena(ox: i32, oz: i32) {
    let mut z = oz - 3;
    while z <= oz + 22 {
        let mut x = ox - 12;
        while x <= ox + 12 {
            put(x, BY - 2, z, STONE);
            put(x, BY - 1, z, DIRT);
            put(x, BY, z, DIRT);
            let mut y = BY + 1;
            while y <= BY + 14 {
                put(x, y, z, AIR);
                y += 1;
            }
            x += 1;
        }
        z += 1;
    }
}

fn boxed(x0: i32, y0: i32, z0: i32, w: i32, h: i32, d: i32, b: u8) {
    let mut y = 0;
    while y < h {
        let mut z = 0;
        while z < d {
            let mut x = 0;
            while x < w {
                put(x0 + x, y0 + y, z0 + z, b);
                x += 1;
            }
            z += 1;
        }
        y += 1;
    }
}

fn hut(x0: i32, z0: i32) {
    // 5 x 5 footprint, walls 3 high: planks, logs at the corners, a leaf
    // roof, a doorway on the player's side, a wool carpet inside.
    let mut y = 0;
    while y < 3 {
        let mut dz = 0;
        while dz < 5 {
            let mut dx = 0;
            while dx < 5 {
                let edge = dx == 0 || dx == 4 || dz == 0 || dz == 4;
                let corner = (dx == 0 || dx == 4) && (dz == 0 || dz == 4);
                if edge {
                    let door = dz == 0 && dx == 2 && y < 2;
                    let b = if door {
                        AIR
                    } else if corner {
                        WOOD
                    } else {
                        PLANK
                    };
                    put(x0 + dx, BY + 1 + y, z0 + dz, b);
                }
                dx += 1;
            }
            dz += 1;
        }
        y += 1;
    }
    let mut dz = 0;
    while dz < 5 {
        let mut dx = 0;
        while dx < 5 {
            put(x0 + dx, BY + 4, z0 + dz, LEAVES);
            if dx > 0 && dx < 4 && dz > 0 && dz < 4 {
                put(x0 + dx, BY + 1, z0 + dz, WOOL);
            }
            dx += 1;
        }
        dz += 1;
    }
}

fn place(p: &mut Player, bx: i32, feet: i32, bz: i32) {
    p.x = bx * BLOCK + BLOCK / 2;
    p.y = feet;
    p.z = bz * BLOCK + BLOCK / 2;
    p.vy = 0;
    p.fall_peak = feet;
    p.yaw = 0;
    p.pitch = -140;
}

/// One script step, at the end of each sim step's survival update.
pub fn step(p: &mut Player) {
    let t = unsafe { T };
    let stand = (BY + 1) * BLOCK;
    if t == 0 {
        let (bx, bz) = (world_to_block_x(p.x), world_to_block_z(p.z));
        unsafe { ORG = (bx, bz) };
    }
    let (ox, oz) = unsafe { ORG };
    match t {
        0 => {
            arena(ox, oz);
            world::remesh_loaded();
            mob::reset();
            place(p, ox, stand, oz);
            p.food = MAX_FOOD;
            p.pick = 0;
        }
        P_SINGLE => {
            put(ox, BY + 1, oz + 8, TNT);
            world::remesh_loaded();
        }
        x if x == P_SINGLE + 60 => ignite_tnt(ox, BY + 1, oz + 8),
        P_CHAIN => {
            arena(ox, oz);
            boxed(ox - 1, BY + 1, oz + 8, 3, 2, 2, TNT);
            world::remesh_loaded();
        }
        x if x == P_CHAIN + 60 => ignite_tnt(ox - 1, BY + 1, oz + 8),
        P_CUBE => {
            arena(ox, oz);
            boxed(ox - 1, BY + 1, oz + 9, 3, 3, 3, TNT);
            world::remesh_loaded();
        }
        x if x == P_CUBE + 60 => ignite_tnt(ox - 1, BY + 1, oz + 9),
        P_FIRE_TNT => {
            arena(ox, oz);
            put(ox, BY + 1, oz + 8, TNT);
            put(ox + 1, BY + 1, oz + 8, TNT);
            world::remesh_loaded();
        }
        x if x == P_FIRE_TNT + 60 => {
            // Flint and steel on the dirt beside the pair: fire in the air
            // cell between them and the player.
            world::light_fire_at(ox, BY + 1, oz + 7);
        }
        P_HUT => {
            arena(ox, oz);
            hut(ox - 2, oz + 8);
            world::remesh_loaded();
        }
        x if x == P_HUT + 60 => {
            // The doorway's floor cell, as the player would strike it.
            world::light_fire_at(ox, BY + 1, oz + 7);
            world::light_fire_at(ox + 1, BY + 1, oz + 7);
        }
        P_LAVA_TNT => {
            arena(ox, oz);
            put(ox, BY + 1, oz + 8, TNT);
            put(ox + 1, BY + 1, oz + 8, LAVA);
            world::remesh_loaded();
            world::wake_fluid(ox + 1, BY + 1, oz + 8);
        }
        _ => {}
    }
    if t % 60 == 0 {
        mob::reset();
    }
    p.health = MAX_HEALTH;
    p.hurt_cd = 0;
    if t % 20 == 0 {
        // The show area: x -6..=6, y just under the floor to 4 up, z 5..=14.
        let (mut tntb, mut solid, mut fuel) = (0, 0, 0);
        let mut x = ox - 6;
        while x <= ox + 6 {
            let mut z = oz + 5;
            while z <= oz + 14 {
                let mut y = BY;
                while y <= BY + 4 {
                    let b = world::get(x, y, z);
                    if b == TNT {
                        tntb += 1;
                    }
                    if b != AIR && b != FIRE {
                        solid += 1;
                    }
                    if b == PLANK || b == WOOD || b == LEAVES || b == WOOL {
                        fuel += 1;
                    }
                    y += 1;
                }
                z += 1;
            }
            x += 1;
        }
        unsafe { COUNTS = (tntb, solid, fuel) };
    }
    unsafe {
        T = t + 1;
        let c = COUNTS;
        core::ptr::write_volatile(
            &raw mut VOXIDE_LAB_FIRE,
            [
                t,
                tnt::live(),
                tnt::min_fuse(),
                world::fire_count(),
                world::blasts_pending(),
                c.0,
                c.1,
                c.2,
                p.health,
                0,
            ],
        );
    }
    let _ = P_END;
}

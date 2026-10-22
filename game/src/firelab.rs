//! Fire and TNT lab (feature `fire-lab`, never shipped): a scripted timeline
//! that builds a flat arena over the spawn and runs the fire and explosion
//! scenarios one after another, so the frames before, during and after each
//! can be captured headless (`--route-screenshot-dir`) and the frame cadence
//! read from a route log.
//!
//! The script runs on the sim-step clock. The player stands at the arena
//! origin looking along +Z with full health; the scenes are built 8 to 10
//! blocks ahead. Phase starts, in sim steps (60 a second):
//!  single TNT, a 12 block chain, a 27 block cube, lava beside TNT, fire beside
//!  TNT, and a burning hut.

use crate::*;

/// What the gates read (`frontend launch --route-watch-u32`, the address from
/// the link map): step, lit TNT blocks, smallest fuse, flames, blasts pending,
/// TNT blocks standing, solid cells in the show area, planks/logs/leaves/wool
/// standing, the player's health, how far a lit block has flown and the fluid
/// queue's length. Words 6 and 7 are counted every 20
/// steps; the rest every step.
#[no_mangle]
pub static mut VOXIDE_LAB_FIRE: [i32; 11] = [0; 11];
static mut COUNTS: (i32, i32, i32) = (0, 0, 0);
static mut T: i32 = 0; // sim steps since gameplay began
static mut ORG: (i32, i32) = (0, 0); // arena origin, block coords

const BY: i32 = 50; // platform block y; you stand at (BY + 1) * BLOCK

/// The `tnt-lab` build runs only the three timing phases below; the others
/// move out of reach.
const OLD: i32 = if cfg!(feature = "tnt-lab") {
    1_000_000
} else {
    0
};
const P_SINGLE: i32 = 30 + OLD;
const P_CHAIN: i32 = 700 + OLD;
const P_CUBE: i32 = 1700 + OLD;
const P_LAVA_TNT: i32 = 3300 + OLD;
const P_FIRE_TNT: i32 = 4000 + OLD;
const P_HUT: i32 = 5600 + OLD;
const P_END: i32 = 9000 + OLD;
/// The timing phases (tnt-lab): one TNT, a 2 x 2 of four, a 4 x 2 x 2 of
/// sixteen. Lit 300 steps after the arena is built, so the mesh is settled.
#[cfg(feature = "tnt-lab")]
const P_ONE: i32 = 600;
#[cfg(feature = "tnt-lab")]
const P_FOUR: i32 = 2100;
#[cfg(feature = "tnt-lab")]
const P_SIXTEEN: i32 = 3600;
#[cfg(feature = "tnt-lab")]
const P_TNT_END: i32 = 5400;
/// The TNT lab's words (tnt-lab): step, lit TNT, TNT waiting for the blast
/// queue, queued blasts, the blast job's stage, blasts completed, cells broken,
/// stale chunk meshes, live drops, live particles and the sim tick.
#[cfg(feature = "tnt-lab")]
#[used]
#[no_mangle]
pub static mut VOXIDE_LAB_TNT: [i32; 15] = [0x544e_544c, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

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

/// The block row the current cluster stands on (tnt-lab).
#[cfg(feature = "tnt-lab")]
static mut Y0: i32 = BY + 1;

/// Build a cluster of TNT `w x h x d` with its corner at (x0, z0): on the flat
/// arena, or (tnt-meadow) on the natural ground, whose footprint is levelled
/// to the highest surface under it and cleared above.
#[cfg(feature = "tnt-lab")]
fn stage(ox: i32, oz: i32, x0: i32, z0: i32, w: i32, h: i32, d: i32) {
    if cfg!(feature = "tnt-meadow") {
        let mut gy = 0;
        let mut z = z0;
        while z < z0 + d {
            let mut x = x0;
            while x < x0 + w {
                gy = gy.max(world::surface_y(x, z));
                x += 1;
            }
            z += 1;
        }
        let mut z = z0;
        while z < z0 + d {
            let mut x = x0;
            while x < x0 + w {
                let mut y = world::surface_y(x, z) - 1;
                while y < gy {
                    put(x, y, z, DIRT);
                    y += 1;
                }
                while y < gy + h + 2 {
                    put(x, y, z, AIR);
                    y += 1;
                }
                x += 1;
            }
            z += 1;
        }
        unsafe { Y0 = gy };
    } else {
        arena(ox, oz);
        unsafe { Y0 = BY + 1 };
    }
    boxed(x0, unsafe { Y0 }, z0, w, h, d, TNT);
    world::remesh_loaded();
}

/// Chunks the player (facing +Z, as the lab holds him) could see that wait for
/// a mesh: the renderer's own chunk test against a camera at the player.
#[cfg(feature = "tnt-lab")]
fn view_pending(p: &Player) -> i32 {
    let chunk_r = 12 * BLOCK;
    let (pcx, pcz) = (
        world_to_block_x(p.x).div_euclid(world::CW),
        world_to_block_z(p.z).div_euclid(world::CW),
    );
    let mut n = 0;
    let mut cz = pcz - 2;
    while cz <= pcz + 2 {
        let mut cx = pcx - 2;
        while cx <= pcx + 2 {
            let dx = (cx * world::CW + world::CW / 2) * BLOCK - p.x;
            let dz = (cz * world::CW + world::CW / 2) * BLOCK - p.z;
            let (zc, xc) = (dz, dx);
            let seen = zc >= -chunk_r
                && zc - chunk_r <= FAR_Z
                && (zc <= 0 || (xc.abs() - chunk_r) <= zc * 160 / PROJ_H);
            if seen && world::lab_chunk_pending(cx, cz) {
                n += 1;
            }
            cx += 1;
        }
        cz += 1;
    }
    n
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
            #[cfg(feature = "tnt-meadow")]
            let stand = world::surface_y(ox, oz) * BLOCK;
            #[cfg(not(feature = "tnt-meadow"))]
            arena(ox, oz);
            world::remesh_loaded();
            mob::reset();
            place(p, ox, stand, oz);
            p.food = MAX_FOOD;
            p.pick = 0;
        }
        #[cfg(feature = "tnt-lab")]
        P_ONE => stage(ox, oz, ox, oz + 8, 1, 1, 1),
        #[cfg(feature = "tnt-lab")]
        x if x == P_ONE + 300 => ignite_tnt(ox, unsafe { Y0 }, oz + 8),
        #[cfg(feature = "tnt-lab")]
        P_FOUR => stage(ox, oz, ox, oz + 8, 2, 1, 2),
        #[cfg(feature = "tnt-lab")]
        x if x == P_FOUR + 300 => ignite_tnt(ox, unsafe { Y0 }, oz + 8),
        #[cfg(feature = "tnt-lab")]
        P_SIXTEEN => stage(ox, oz, ox - 2, oz + 15, 4, 2, 2),
        #[cfg(feature = "tnt-lab")]
        x if x == P_SIXTEEN + 300 => ignite_tnt(ox - 2, unsafe { Y0 }, oz + 15),
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
                tnt::farthest(ox, oz),
                world::fluid_pending(),
            ],
        );
    }
    let _ = P_END;
    #[cfg(feature = "tnt-lab")]
    unsafe {
        let _ = P_TNT_END;
        core::ptr::write_volatile(
            &raw mut VOXIDE_LAB_TNT,
            [
                0x544e_544c,
                t,
                tnt::live(),
                tnt::lab_pending(),
                world::blasts_pending(),
                world::lab_stage(),
                world::blasts_completed() as i32,
                world::lab_broken(),
                world::lab_dirty(),
                lab_live_drops(),
                lab_live_particles(),
                SIM_TICK as i32,
                slack::allowance() as i32,
                slack::lab_spare() as i32,
                view_pending(p),
            ],
        );
    }
}

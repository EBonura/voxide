//! Primed TNT and what an explosion does to the things in it.
//!
//! Java Edition (minecraft.wiki/w/TNT): TNT is lit by flint and steel, by
//! fire, lava, a redstone signal or another explosion. The block turns into an
//! entity with an 80 game tick (4 s) fuse that blinks white, takes a little
//! upward and sideways kick when it is lit, falls, and explodes with power 4.
//! TNT a blast lights gets a 10 to 29 tick fuse. The block side of an
//! explosion is world.rs (a job of rays, spread over ticks); this file is the
//! entity pool, the blast's damage and knockback on mobs, the player, other
//! TNT and dropped items, and the draw.

use crate::*;

/// A fresh fuse, 80 game ticks (4 s, minecraft.wiki/w/TNT).
pub const FUSE_FULL: i32 = units::java_ticks(blast::FUSE_JAVA_TICKS);
/// TNT's explosion power.
pub const POWER: i32 = 4;
/// Primed TNT at once. A 27 block cube is 27; a block that cannot be lit for
/// want of a slot goes off at once instead.
const MAX_TNT: usize = 32;

/// Java's gravity on TNT, 0.04 blocks a tick squared, 16 blocks/s^2, in Q8
/// units a sim tick a sim tick.
const GRAVITY_Q8: i32 = units::cbps2_q8(1600);
/// Java's drag, 0.98 a game tick; 1 - 0.98^(1/3) of the speed a sim tick, /65536.
const DRAG_65536: i32 = 441;
/// The kick when lit: 0.2 blocks a tick up (4 blocks/s), 0.02 across.
const KICK_UP_Q8: i32 = units::cbps_q8(400);
const KICK_SIDE_Q8: i32 = units::cbps_q8(40);
/// What the hit of one blast on the creatures around it costs, system clocks
/// (measured: the tnt stage at a detonation, 2026-10-10).
const DETONATE_COST: u32 = 200_000;
/// Half the entity's 0.98 block box.
const HALF_W: i32 = 31;
const HEIGHT: i32 = 62;
/// Ground friction on the horizontal speed a sim tick: Java's 0.7 a game tick.
const GROUND_FRICTION_256: i32 = 227;

static mut USED: [bool; MAX_TNT] = [false; MAX_TNT];
/// Feet-centre position in world units, and the Q8 remainder of each axis.
static mut X: [i32; MAX_TNT] = [0; MAX_TNT];
static mut Y: [i32; MAX_TNT] = [0; MAX_TNT];
static mut Z: [i32; MAX_TNT] = [0; MAX_TNT];
static mut FX: [i32; MAX_TNT] = [0; MAX_TNT];
static mut FY: [i32; MAX_TNT] = [0; MAX_TNT];
static mut FZ: [i32; MAX_TNT] = [0; MAX_TNT];
/// Velocity, Q8 world units a sim tick.
static mut VX: [i32; MAX_TNT] = [0; MAX_TNT];
static mut VY: [i32; MAX_TNT] = [0; MAX_TNT];
static mut VZ: [i32; MAX_TNT] = [0; MAX_TNT];
static mut FUSE: [u16; MAX_TNT] = [0; MAX_TNT];
static mut GROUND: [bool; MAX_TNT] = [false; MAX_TNT];
static mut SALT: u32 = 0x1234_5679;

fn rnd() -> u32 {
    unsafe {
        let mut x = SALT;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        SALT = x;
        x >> 4
    }
}

/// Forget every primed block (a new world, a load).
pub fn reset() {
    unsafe { USED = [false; MAX_TNT] };
}

/// Light the TNT block at a cell: it becomes a primed entity with `fuse` sim
/// ticks. Not TNT (already lit, or burnt away by the caller): then only the
/// entity is made.
pub fn prime(x: i32, y: i32, z: i32, fuse: i32) {
    if get_block_i32(x, y, z) == TNT {
        set_block_i32(x, y, z, AIR);
        record_edit(x, y, z, AIR);
    }
    spawn(x, y, z, fuse);
}

/// The entity for a block already removed by the caller, at the cell's
/// bottom centre, with Java's random upward kick. With no free slot the TNT
/// goes off at once.
pub fn spawn(x: i32, y: i32, z: i32, fuse: i32) {
    let mut i = 0;
    while i < MAX_TNT {
        if !unsafe { USED[i] } {
            let a = (rnd() & 0xFFF) as u16; // 4096 to the turn
            let (s, c) = (sincos::sin_q12(a), sincos::cos_q12(a));
            unsafe {
                USED[i] = true;
                X[i] = x * BLOCK + BLOCK / 2;
                Y[i] = y * BLOCK;
                Z[i] = z * BLOCK + BLOCK / 2;
                FX[i] = 0;
                FY[i] = 0;
                FZ[i] = 0;
                VX[i] = -(s * KICK_SIDE_Q8) >> 12;
                VY[i] = KICK_UP_Q8;
                VZ[i] = -(c * KICK_SIDE_Q8) >> 12;
                FUSE[i] = fuse.clamp(1, u16::MAX as i32) as u16;
                GROUND[i] = false;
            }
            sfx::sapper_hiss(); // the fuse's hiss, as Java plays when TNT is lit
            return;
        }
        i += 1;
    }
    // No slot: explode now.
    detonate_at(
        x * BLOCK + BLOCK / 2,
        y * BLOCK + 4,
        z * BLOCK + BLOCK / 2,
        usize::MAX,
    );
}

/// One sim tick of every primed block.
#[inline(never)]
pub fn tick() {
    let mut i = 0;
    while i < MAX_TNT {
        if unsafe { USED[i] } {
            step(i);
        }
        i += 1;
    }
}

fn step(i: usize) {
    unsafe {
        // Out of the loaded ring a block does not tick, as Java's unloaded
        // entities do not.
        if !world::column_loaded(world_to_block_x(X[i]), world_to_block_z(Z[i])) {
            return;
        }
        // Gravity, then the move, then drag (Java's order).
        VY[i] -= GRAVITY_Q8;
        // Axis at a time, the axis that hits stops.
        let qx = VX[i] + FX[i];
        FX[i] = qx & 255;
        let nx = X[i] + (qx >> 8);
        if aabb_collides_dims(nx, Y[i], Z[i], HALF_W, HEIGHT) {
            VX[i] = 0;
            FX[i] = 0;
        } else {
            X[i] = nx;
        }
        let qz = VZ[i] + FZ[i];
        FZ[i] = qz & 255;
        let nz = Z[i] + (qz >> 8);
        if aabb_collides_dims(X[i], Y[i], nz, HALF_W, HEIGHT) {
            VZ[i] = 0;
            FZ[i] = 0;
        } else {
            Z[i] = nz;
        }
        let qy = VY[i] + FY[i];
        FY[i] = qy & 255;
        let ny = Y[i] + (qy >> 8);
        GROUND[i] = false;
        if aabb_collides_dims(X[i], ny, Z[i], HALF_W, HEIGHT) {
            if VY[i] < 0 {
                GROUND[i] = true;
            }
            VY[i] = 0;
            FY[i] = 0;
        } else {
            Y[i] = ny;
        }
        VX[i] -= (VX[i] * DRAG_65536) >> 16;
        VY[i] -= (VY[i] * DRAG_65536) >> 16;
        VZ[i] -= (VZ[i] * DRAG_65536) >> 16;
        if GROUND[i] {
            VX[i] = VX[i] * GROUND_FRICTION_256 >> 8;
            VZ[i] = VZ[i] * GROUND_FRICTION_256 >> 8;
        }
        // A wisp of smoke over the fuse, a few times a second.
        if FUSE[i] % 20 == 0 {
            spawn_particles(X[i], Y[i] + HEIGHT + 8, Z[i], (70, 70, 70), 1, i as u32 + FUSE[i] as u32, 6);
        }
        if FUSE[i] > 1 {
            FUSE[i] -= 1;
        } else if slack::room_for(DETONATE_COST) {
            // The blast takes a slot in the block job's queue; with it full,
            // or the frame out of time for the blast's hit on the creatures
            // around it, the block waits a tick, blinking, rather than lose it.
            let (x, y, z) = (X[i], Y[i], Z[i]);
            if detonate_at(x, y + 4, z, i) {
                USED[i] = false;
            }
        }
    }
}

/// The blast at a point in world units: queue the block job, then hurt and
/// shove what stands in it. False when the job queue is full. `skip` is the
/// primed block that is going off, or usize::MAX.
fn detonate_at(x: i32, y: i32, z: i32, skip: usize) -> bool {
    let seed = unsafe { SIM_TICK }
        .wrapping_mul(2_654_435_761)
        .wrapping_add((x ^ (z << 7)) as u32);
    if !world::explode(x, y, z, POWER, seed) {
        return false;
    }
    explosion_effects(x, y, z, POWER, skip);
    true
}

/// A blast's sound, smoke and its hit on the mobs, the player, other primed
/// TNT and dropped items. Every entity is judged against the world as it is
/// now, before any block of this blast breaks.
pub fn explosion_effects(x: i32, y: i32, z: i32, power: i32, skip: usize) {
    sfx::explode();
    spawn_particles(x, y + BLOCK / 2, z, (96, 84, 72), 30, (x ^ z) as u32, 46);
    mob::blast_mobs(x, y, z, power);
    // The player, through the hazard path the sappers use (armour, i-frames).
    let p = unsafe { PLAYER_POS };
    if let Some(h) = hit_on(power, (x, y, z), (p.0, p.1, p.2), PLAYER_HALF_W, PLAYER_HEIGHT, EYE_HEIGHT, true) {
        mob::blast_hazard(h.dmg);
        unsafe {
            KICK.0 += h.kx;
            KICK.1 += h.ky;
            KICK.2 += h.kz;
        }
    }
    // Other primed TNT is pushed, not hurt.
    let mut i = 0;
    while i < MAX_TNT {
        if i != skip && unsafe { USED[i] } {
            let pos = unsafe { (X[i], Y[i], Z[i]) };
            if let Some(h) = hit_on(power, (x, y, z), pos, HALF_W, HEIGHT, 0, false) {
                unsafe {
                    VX[i] += h.kx * 64 / 3;
                    VY[i] += h.ky * 64 / 3;
                    VZ[i] += h.kz * 64 / 3;
                }
            }
        }
        i += 1;
    }
    // A dropped item has 5 health: a blast that deals that much destroys it.
    destroy_drops(x, y, z, power);
}

/// The player's feet position, mirrored here each sim tick so a blast queued
/// by a mob, a fuse or the world can find him without a handle on him.
pub static mut PLAYER_POS: (i32, i32, i32) = (0, 0, 0);
/// Knockback the player has taken since it was last applied, Q8 blocks per
/// Java tick (x, y, z).
static mut KICK: (i32, i32, i32) = (0, 0, 0);

/// Take the pending knockback on the player: (x, y, z), Q8 blocks per Java tick.
pub fn take_kick() -> (i32, i32, i32) {
    unsafe {
        let k = KICK;
        KICK = (0, 0, 0);
        k
    }
}

/// What a blast does to an entity.
pub struct Hit {
    pub dmg: i32,
    pub kx: i32,
    pub ky: i32,
    pub kz: i32,
}

/// Does this cell stop a blast's sight? A solid block does; air, fluids,
/// plants and fire do not.
fn stops_sight(bx: i32, by: i32, bz: i32) -> bool {
    let b = get_block_i32(bx, by, bz);
    b != AIR && !is_water(b) && !is_lava(b) && !world::is_cross_plant(b) && !is_small_block(b)
}

/// A blast of `power` centred at `centre` on an entity whose feet centre is
/// `pos`: its damage and knockback, or None out of reach or fully shielded.
/// `eye` is how far above the feet the push is aimed (Java pushes a living
/// thing from its eyes, a block from its feet).
pub fn hit_on(
    power: i32,
    centre: (i32, i32, i32),
    pos: (i32, i32, i32),
    half_w: i32,
    height: i32,
    eye: i32,
    cover: bool,
) -> Option<Hit> {
    let (dx, dy, dz) = (pos.0 - centre.0, pos.1 - centre.1, pos.2 - centre.2);
    // Cheap box reject before any square root.
    let reach = 2 * power * BLOCK;
    if dx.abs() >= reach || dy.abs() >= reach || dz.abs() >= reach {
        return None;
    }
    let dist = blast::isqrt((dx * dx + dy * dy + dz * dz) as u32) as i32;
    let prox = blast::proximity_q8(power, dist)?;
    // Cover costs a block read a step along six lines, so only what it matters
    // for (the player and the mobs) pays; TNT and dropped items take it full.
    let seen = if cover {
        blast::exposure_q8(centre, pos, half_w, height, &mut stops_sight)
    } else {
        256
    };
    let impact = prox * seen >> 8;
    if impact <= 0 {
        return None;
    }
    let (kx, ky, kz) = blast::knockback_q8(dx, dy + eye, dz, impact);
    Some(Hit {
        dmg: blast::damage_from_impact(power, impact),
        kx,
        ky,
        kz,
    })
}

/// Draw every primed block: a red body with a white band, or solid white on
/// the flash (Java flashes white for 5 game ticks in every 10).
pub fn render(cam: &Camera, count: &mut usize) {
    let mut i = 0;
    while i < MAX_TNT {
        if unsafe { USED[i] } {
            let (x, y, z) = unsafe { (X[i], Y[i], Z[i]) };
            if (x - cam.x).abs() + (z - cam.z).abs() <= FAR_Z + 4 * BLOCK {
                let flash = blast::flashing(unsafe { FUSE[i] } as u32);
                if flash {
                    emit_box(
                        cam,
                        x - HALF_W,
                        y,
                        z - HALF_W,
                        x + HALF_W,
                        y + HEIGHT,
                        z + HALF_W,
                        (255, 255, 255),
                        count,
                    );
                } else {
                    emit_box(
                        cam,
                        x - HALF_W,
                        y,
                        z - HALF_W,
                        x + HALF_W,
                        y + HEIGHT,
                        z + HALF_W,
                        (196, 44, 32),
                        count,
                    );
                    emit_box(
                        cam,
                        x - HALF_W - 1,
                        y + 22,
                        z - HALF_W - 1,
                        x + HALF_W + 1,
                        y + 40,
                        z + HALF_W + 1,
                        (226, 224, 214),
                        count,
                    );
                }
            }
        }
        i += 1;
    }
}

/// Primed blocks lit right now, for the lab and the gates.
#[allow(dead_code)]
pub fn live() -> i32 {
    let mut n = 0;
    let mut i = 0;
    while i < MAX_TNT {
        if unsafe { USED[i] } {
            n += 1;
        }
        i += 1;
    }
    n
}

/// The smallest fuse among the lit blocks (sim ticks), 0 with none lit.
#[allow(dead_code)]
pub fn min_fuse() -> i32 {
    let mut m = 0;
    let mut i = 0;
    while i < MAX_TNT {
        if unsafe { USED[i] } && (m == 0 || (unsafe { FUSE[i] } as i32) < m) {
            m = unsafe { FUSE[i] } as i32;
        }
        i += 1;
    }
    m
}

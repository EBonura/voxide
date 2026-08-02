//! Cuboid mobs: passive (pig/cow/sheep/chicken) and hostile (zombie/skeleton/
//! sapper/spider). Fixed-point FSM AI (idle/wander/chase/flee), greedy axis-step
//! toward the player with a jump when blocked (no A*), gravity + AABB collision
//! reused from the player. Capped and despawned by distance to protect fps.
//! Rendering lives in `main` (flat cuboid boxes inserted into the OT).

use crate::{
    aabb_collides_dims, world, world_to_block_x, world_to_block_y, world_to_block_z, BLOCK, GRASS,
    GRAVITY, JUMP_VY, PLAYER_HALF_W, PLAYER_HEIGHT, TERMINAL_VY,
};
use psx_fx::rng::LcgRng;

pub const CAP: usize = 8;

pub const PIG: u8 = 0;
pub const COW: u8 = 1;
pub const SHEEP: u8 = 2;
pub const CHICKEN: u8 = 3;
pub const ZOMBIE: u8 = 4;
pub const SKELETON: u8 = 5;
pub const SAPPER: u8 = 6;
pub const SPIDER: u8 = 7;
/// The void dragon. Flies (no gravity, no block collision -- it phases through
/// the pillars the way Java's does), circles the island, and dives at you.
/// Spawned by the world when you arrive in the Void, never by the spawn timer.
pub const DRAGON: u8 = 8;
/// Neutral until you hit it, then it comes for you; blinks away when hurt.
pub const WRAITH: u8 = 9;
/// Passive; feed it a bone to tame it and it follows you and fights for you.
pub const WOLF: u8 = 10;
/// Passive; trade with it (R3) rather than kill it.
pub const VILLAGER: u8 = 11;
// Inferno natives. They exist in Java to gate the brewing ingredients, which is
// exactly what they do here: ember rods, wailer tears and magma paste.
pub const EMBER: u8 = 12; // hovers, hostile
pub const WAILER: u8 = 13; // large, hovers high
pub const CHARRED_SK: u8 = 14; // walks, hostile

/// Hostile = hunts you on sight. Wraiths are NEUTRAL (they only retaliate),
/// and wolves and villagers never do, so the plain `kind >= ZOMBIE` test that
/// worked while spider was the last kind no longer holds.
#[inline]
pub fn is_hostile(kind: u8) -> bool {
    (kind >= ZOMBIE && kind <= DRAGON) || kind == WRAITH || kind >= EMBER
}

const ST_WANDER: u8 = 0;
const ST_CHASE: u8 = 1;
const ST_FLEE: u8 = 2;

const SPEED: i32 = 5; // mob walk speed, world units/frame (player walks 9)
const CHASE_R: i32 = 12 * BLOCK; // hostile aggro radius
const LOSE_R: i32 = 18 * BLOCK; // give-up radius
const LURE_R: i32 = 8 * BLOCK; // passive-animal follow radius when luring with wheat
const DESPAWN_R: i32 = 30 * BLOCK;
const SPAWN_MIN: i32 = 8 * BLOCK;
const SPAWN_MAX: i32 = 14 * BLOCK;

#[derive(Copy, Clone)]
struct Mob {
    kind: u8,
    alive: bool,
    on_ground: bool,
    x: i32,
    y: i32, // feet
    z: i32,
    vy: i32,
    health: i16,
    state: u8,
    timer: u16, // wander/flee countdown, or skeleton shoot cooldown
    fuse: u16,  // sapper detonation charge
    hurt_cd: u8,
    love: u16,  // breeding "in love" countdown (passive mobs, set by feeding wheat)
    /// Gait phase, advanced only while actually moving, so legs stop when the
    /// mob stops. Java drives limb swing off distance travelled for the same
    /// reason -- a timer-driven swing marches on the spot.
    walk: u8,
    /// Which way the body points: 0 +Z, 1 +X, 2 -Z, 3 -X. The AI already
    /// produces one of eight compass steps; this keeps the nearest quarter turn
    /// so the renderer can put the face on the correct side.
    facing: u8,
}

const DEAD: Mob = Mob {
    kind: 0,
    alive: false,
    on_ground: false,
    x: 0,
    y: 0,
    z: 0,
    vy: 0,
    health: 0,
    state: ST_WANDER,
    timer: 0,
    fuse: 0,
    hurt_cd: 0,
    love: 0,
    walk: 0,
    facing: 0,
};

// Arrows fired by skeletons.
const ARROW_CAP: usize = 8;
const ARROW_SPEED: i32 = 26;
const FUSE_MAX: u16 = 45; // 30-tick (1.5s) Java sapper fuse, at 30fps
const BLAST_R: i32 = 3; // block-destruction radius = explosion power 3 (blocks)
const BLAST_DMG_R: i32 = 6; // damage reaches 2*power = 6 blocks (Java falloff)
/// Height the dragon cruises at, above the Void island's deck.
const DRAGON_CRUISE: i32 = 42 * BLOCK;
/// The dragon never stops hunting, so it needs its own reach rather than the
/// shared aggro radius.
const DRAGON_CHASE_R: i32 = 80 * BLOCK;
/// Standoff radius: it circles at this range rather than closing to nothing.
const DRAGON_ORBIT: i32 = 9 * BLOCK;

#[derive(Copy, Clone)]
struct Arrow {
    alive: bool,
    x: i32,
    y: i32,
    z: i32,
    vx: i32,
    vy: i32,
    vz: i32,
    life: u16,
    from_player: bool, // player arrows hit mobs; skeleton arrows hit the player
}

const NO_ARROW: Arrow = Arrow {
    alive: false,
    x: 0,
    y: 0,
    z: 0,
    vx: 0,
    vy: 0,
    vz: 0,
    life: 0,
    from_player: false,
};

static mut MOBS: [Mob; CAP] = [DEAD; CAP];
static mut ARROWS: [Arrow; ARROW_CAP] = [NO_ARROW; ARROW_CAP];
static mut HAZARD_DMG: i32 = 0; // arrow hits + blast damage to the player this frame
static mut SPAWN_TIMER: u16 = 60;
static mut RNG: LcgRng = LcgRng::new(0x1234_5678);
static mut XP_DROPS: u16 = 0; // experience from kills, drained by main

// Kills this frame, for main to turn into item entities. This used to be four
// running counters (meat/wool/bones/powder) that main added straight to the
// inventory, so loot teleported off the corpse from anywhere on the map.
// Logging the position instead lets it fall where the mob died. Which items a
// kind yields stays main's business: mob.rs has no idea what RAW_MEAT is.
const DEATH_CAP: usize = 8;
static mut DEATH_X: [i32; DEATH_CAP] = [0; DEATH_CAP];
static mut DEATH_Y: [i32; DEATH_CAP] = [0; DEATH_CAP];
static mut DEATH_Z: [i32; DEATH_CAP] = [0; DEATH_CAP];
static mut DEATH_KIND: [u8; DEATH_CAP] = [0; DEATH_CAP];
static mut DEATH_N: usize = 0;

fn record_death(m: &Mob) {
    unsafe {
        if DEATH_N >= DEATH_CAP {
            return; // eight kills in one frame is already a sapper blast
        }
        DEATH_X[DEATH_N] = m.x;
        DEATH_Y[DEATH_N] = m.y;
        DEATH_Z[DEATH_N] = m.z;
        DEATH_KIND[DEATH_N] = m.kind;
        DEATH_N += 1;
    }
}

/// Kills logged since the last drain.
pub fn death_count() -> usize {
    unsafe { DEATH_N }
}

/// `(x, y, z, kind)` of logged kill `i`.
pub fn death_at(i: usize) -> (i32, i32, i32, u8) {
    unsafe { (DEATH_X[i], DEATH_Y[i], DEATH_Z[i], DEATH_KIND[i]) }
}

pub fn clear_deaths() {
    unsafe {
        DEATH_N = 0;
    }
}
static mut LURE: bool = false; // true while the player holds wheat (animals follow)

/// Take and clear accumulated XP from kills this frame.
pub fn take_xp() -> u16 {
    unsafe {
        let d = XP_DROPS;
        XP_DROPS = 0;
        d
    }
}



/// Set each frame: passive animals follow the player while this is true.
pub fn set_lure(on: bool) {
    unsafe {
        LURE = on;
    }
}



#[inline]
fn rng() -> u32 {
    // Callers lean on the low bits (& 1, % 4, % 120), and an LCG's low bits
    // alone cycle with tiny periods, so fold the strong high half back in.
    let x = unsafe { RNG.next() };
    x ^ (x >> 16)
}

/// (half-width, height) in world units per kind.
pub fn dims(kind: u8) -> (i32, i32) {
    match kind {
        // Boss scale: half-width 2.5 blocks and 1.75 tall, and render_dragon
        // reaches out to 2*hw for the wings, so it spans ~10 blocks tip to tip.
        DRAGON => (160, 112),
        WRAITH => (14, 116), // Java's tall, thin silhouette
        EMBER => (16, 56),
        WAILER => (56, 56), // Java's is enormous; this is as big as the pool allows
        CHARRED_SK => (14, 96),
        WOLF => (14, 46),
        VILLAGER => (16, 80),
        CHICKEN => (12, 40),
        SPIDER => (28, 36),
        PIG | SHEEP => (16, 56),
        COW => (18, 70),
        _ => (16, 80), // zombie/skeleton/sapper, ~human
    }
}

#[inline]
pub fn is_flyer(kind: u8) -> bool {
    kind == DRAGON || kind == EMBER || kind == WAILER
}

fn max_health(kind: u8) -> i16 {
    // Java Edition HP (2 HP = 1 heart). minecraft.wiki per-mob pages.
    match kind {
        CHICKEN => 4,
        SHEEP => 8,
        PIG | COW => 10,
        SPIDER => 16,
        ZOMBIE | SKELETON | SAPPER => 20,
        DRAGON => 200, // Java's boss pool, and it takes a while
        WRAITH => 40,
        EMBER => 20,
        WAILER => 10, // Java: papery, dies in one good hit
        CHARRED_SK => 20,
        WOLF => 16,
        VILLAGER => 20,
        _ => 16,
    }
}

#[derive(Copy, Clone)]
pub struct MobView {
    pub kind: u8,
    pub alive: bool,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub priming: bool, // sapper with a lit fuse (renderer flashes it white)
    /// Gait phase 0..255, for the leg swing.
    pub walk: u8,
    /// Body facing: 0 +Z, 1 +X, 2 -Z, 3 -X.
    pub facing: u8,
    /// Struck within the last few frames -- the renderer flashes it red.
    pub hurt: bool,
}

pub fn get(i: usize) -> MobView {
    let m = unsafe { MOBS[i] };
    MobView {
        kind: m.kind,
        alive: m.alive,
        x: m.x,
        y: m.y,
        z: m.z,
        // Flash on/off every 3 frames while the fuse burns.
        priming: m.kind == SAPPER && m.fuse > 0 && (unsafe { BURN_TICK } / 3) % 2 == 0,
        walk: m.walk,
        facing: m.facing,
        // hurt_cd was set on every hit and decremented every frame and then read
        // by nothing at all -- a dead field. It is the damage flash now.
        hurt: m.hurt_cd > 4,
    }
}

fn count_alive() -> usize {
    let mut c = 0;
    let mut i = 0;
    while i < CAP {
        if unsafe { MOBS[i].alive } {
            c += 1;
        }
        i += 1;
    }
    c
}

fn free_slot() -> Option<usize> {
    let mut i = 0;
    while i < CAP {
        if !unsafe { MOBS[i].alive } {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Spawn a mob on the ground near the player. Passive by day, hostile by night.
fn try_spawn(px: i32, pz: i32, night: bool) {
    let s = match free_slot() {
        Some(s) => s,
        None => return,
    };
    // Random offset ring around the player.
    let ang = rng();
    let dx = SPAWN_MIN + (ang % (SPAWN_MAX - SPAWN_MIN) as u32) as i32;
    let dz = SPAWN_MIN + ((ang >> 8) % (SPAWN_MAX - SPAWN_MIN) as u32) as i32;
    let sx = if ang & 1 == 0 { px + dx } else { px - dx };
    let sz = if ang & 2 == 0 { pz + dz } else { pz - dz };
    let bx = world_to_block_x(sx);
    let bz = world_to_block_z(sz);
    let sy = world::surface_y(bx, bz);
    if sy < 2 || sy > 100 {
        return;
    }
    // Light/biome gate: hostiles spawn in the dark (night); passive animals
    // only on grass.
    let kind = if world::dimension() == world::DIM_VOID {
        WRAITH // the End's own mob, and the only one that spawns there
    } else if world::dimension() == world::DIM_INFERNO {
        match rng() % 3 {
            0 => EMBER,
            1 => WAILER,
            _ => CHARRED_SK,
        }
    } else if night {
        // One in eight night spawns is an wraith, as in Java's overworld.
        if rng() % 8 == 0 {
            WRAITH
        } else {
            ZOMBIE + (rng() % 4) as u8
        }
    } else {
        if world::get(bx, sy - 1, bz) != GRASS {
            return;
        }
        // Wolves and villagers share the daytime roll with the farm animals.
        match rng() % 6 {
            4 => WOLF,
            5 => VILLAGER,
            r => r as u8,
        }
    };
    let (hw, h) = dims(kind);
    let feet = sy * BLOCK;
    if aabb_collides_dims(sx, feet, sz, hw, h) {
        return; // spawn point blocked
    }
    unsafe {
        MOBS[s] = Mob {
            kind,
            alive: true,
            on_ground: false,
            x: sx,
            y: feet,
            z: sz,
            vy: 0,
            health: max_health(kind),
            state: ST_WANDER,
            timer: (rng() % 120) as u16,
            fuse: 0,
            hurt_cd: 0,
            love: 0,
            walk: 0,
            facing: 0,
        };
    }
}

/// Put the void dragon in the air over the Void island. Called by the world
/// when the player arrives, not by the spawn timer: it is a boss, and there is
/// exactly one.
pub fn spawn_dragon(px: i32, pz: i32) {
    let s = match free_slot() {
        Some(s) => s,
        None => return,
    };
    unsafe {
        MOBS[s] = Mob {
            kind: DRAGON,
            alive: true,
            on_ground: false,
            x: px + 18 * BLOCK,
            y: DRAGON_CRUISE,
            z: pz + 18 * BLOCK,
            vy: 0,
            health: max_health(DRAGON),
            state: ST_CHASE,
            timer: 0,
            fuse: 0,
            hurt_cd: 0,
            love: 0,
            walk: 0,
            facing: 0,
        };
    }
}

/// Interact with the nearest mob within reach: feed a wolf a bone to tame it,
/// or trade with a villager. Returns what the player should be charged and
/// given, or None if nothing was in reach.
///
/// `love == u16::MAX` marks a tamed wolf. It reuses the breeding field because a
/// tamed wolf never breeds, and a whole extra byte per mob for one flag is not
/// worth it on this machine.
pub fn interact(px: i32, py: i32, pz: i32, has_bone: bool, wheat: u16) -> Interact {
    let reach = 3 * BLOCK;
    let mut i = 0;
    while i < CAP {
        let mut m = unsafe { MOBS[i] };
        if m.alive
            && (m.x - px).abs() < reach
            && (m.z - pz).abs() < reach
            && (m.y - py).abs() < 2 * BLOCK
        {
            if m.kind == WOLF && m.love != u16::MAX && has_bone {
                m.love = u16::MAX;
                unsafe { MOBS[i] = m };
                return Interact::Tamed;
            }
            if m.kind == VILLAGER && wheat >= 8 {
                return Interact::Traded;
            }
        }
        i += 1;
    }
    Interact::None
}

pub enum Interact {
    None,
    Tamed,
    Traded,
}

/// True while a dragon is alive, and its health, for the boss bar.
pub fn dragon_status() -> Option<(i16, i16)> {
    let mut i = 0;
    while i < CAP {
        let m = unsafe { MOBS[i] };
        if m.alive && m.kind == DRAGON {
            return Some((m.health, max_health(DRAGON)));
        }
        i += 1;
    }
    None
}

/// TEMP capture helper: fill the roster with one of each kind in a row on the
/// -Z side of the player (their +Z faces point at a camera looking -Z).
/// Re-assert every frame so AI/damage can't disturb the lineup.
pub const LINEUP_FIRST: u8 = 0;
pub fn debug_lineup(px: i32, py: i32, pz: i32) {
    let mut k: u8 = 0;
    while k < 8 {
        unsafe {
            MOBS[k as usize] = Mob {
                kind: k + LINEUP_FIRST,
                alive: true,
                on_ground: true,
                x: px + (k as i32 * 2 - 7) * 40,
                y: py,
                z: pz - 210,
                vy: 0,
                health: max_health(k + LINEUP_FIRST),
                state: ST_WANDER,
                timer: 30000,
                fuse: 0,
                hurt_cd: 0,
                love: 0,
                walk: 0,
                facing: 0,
            };
        }
        k += 1;
    }
}

/// Advance all mobs: spawn budget, AI, physics, despawn.
pub fn update(px: i32, py: i32, pz: i32, night: bool) {
    unsafe {
        if SPAWN_TIMER > 0 {
            SPAWN_TIMER -= 1;
        } else {
            if count_alive() < CAP {
                try_spawn(px, pz, night);
            }
            SPAWN_TIMER = 90;
        }
    }

    unsafe {
        BURN_TICK = BURN_TICK.wrapping_add(1);
    }
    let mut i = 0;
    while i < CAP {
        if unsafe { MOBS[i].alive } {
            step_mob(i, px, py, pz, night);
            // Idle voices: each slot mutters on its own ~7s cycle (offset per
            // slot so the field never speaks in chorus), volume falling with
            // distance, silent out of earshot.
            if (unsafe { BURN_TICK } as usize).wrapping_add(i * 67) % 214 == 0 {
                let (mx, mz, kind) = unsafe { (MOBS[i].x, MOBS[i].z, MOBS[i].kind) };
                let d = ((mx - px).abs() + (mz - pz).abs()) / 64;
                if d < 14 {
                    match kind {
                        PIG => crate::sfx::pig(d),
                        COW => crate::sfx::cow(d),
                        SHEEP => crate::sfx::sheep(d),
                        CHICKEN => crate::sfx::chicken(d),
                        ZOMBIE => crate::sfx::zombie(d),
                        SKELETON => crate::sfx::skeleton(d),
                        WOLF => crate::sfx::wolf(d),
                        SPIDER => crate::sfx::spider(d),
                        _ => {}
                    }
                }
            }
            if !night {
                sun_burn(i);
            }
            unsafe {
                if MOBS[i].love > 0 {
                    MOBS[i].love -= 1;
                }
            }
        }
        i += 1;
    }
    breed_tick();
    update_arrows(px, py, pz);
}

/// Pair up two nearby in-love same-kind animals into a new one, then clear both.
fn breed_tick() {
    let mut i = 0;
    while i < CAP {
        let mi = unsafe { MOBS[i] };
        if mi.alive && mi.love > 0 && !is_hostile(mi.kind) {
            let mut j = i + 1;
            while j < CAP {
                let mj = unsafe { MOBS[j] };
                if mj.alive && mj.love > 0 && mj.kind == mi.kind {
                    let d = (mi.x - mj.x).abs() + (mi.z - mj.z).abs();
                    if d < 2 * BLOCK {
                        let (cx, cz) = ((mi.x + mj.x) / 2, (mi.z + mj.z) / 2);
                        spawn_offspring(mi.kind, cx, cz);
                        unsafe {
                            MOBS[i].love = 0;
                            MOBS[j].love = 0;
                        }
                        crate::spawn_particles(cx, mi.y + BLOCK, cz, (240, 120, 170), 8, (i * 31 + j) as u32, 14);
                        break;
                    }
                }
                j += 1;
            }
        }
        i += 1;
    }
}

fn spawn_offspring(kind: u8, x: i32, z: i32) {
    let s = match free_slot() {
        Some(s) => s,
        None => return,
    };
    let bx = world_to_block_x(x);
    let bz = world_to_block_z(z);
    let sy = world::surface_y(bx, bz);
    unsafe {
        MOBS[s] = Mob {
            kind,
            alive: true,
            on_ground: false,
            x,
            y: sy * BLOCK,
            z,
            vy: 0,
            health: max_health(kind),
            state: ST_WANDER,
            timer: 30,
            fuse: 0,
            hurt_cd: 0,
            love: 0,
            walk: 0,
            facing: 0,
        };
    }
}

/// Feed wheat to the nearest passive animal in front (within reach): it enters
/// "love mode". Returns true if an animal was fed (so the caller spends a wheat).
pub fn feed(px: i32, py: i32, pz: i32, fx: i32, fz: i32, reach: i32) -> bool {
    let mut best = CAP;
    let mut best_d = reach * reach;
    let mut i = 0;
    while i < CAP {
        let m = unsafe { MOBS[i] };
        if m.alive && !is_hostile(m.kind) && m.love == 0 {
            let dx = m.x - px;
            let dz = m.z - pz;
            let dy = (m.y - py).abs();
            if dx * fx + dz * fz > 0 && dy < 2 * BLOCK {
                let d2 = dx * dx + dz * dz;
                if d2 < best_d {
                    best_d = d2;
                    best = i;
                }
            }
        }
        i += 1;
    }
    if best == CAP {
        return false;
    }
    unsafe {
        MOBS[best].love = 600; // ~20s window to find a mate
        crate::spawn_particles(MOBS[best].x, MOBS[best].y + BLOCK, MOBS[best].z, (240, 120, 170), 5, best as u32, 12);
    }
    true
}

static mut BURN_TICK: u16 = 0;

/// Zombies and skeletons caught in open daylight catch fire and burn down,
/// like Java. ponytail: "exposed" = at/above the surface column (ignores
/// overhangs/caves-with-skylight); damage ticks on a shared cadence.
/// Wraiths teleport when struck: a short hop to a random spot around the
/// player, which is what makes them awkward to fight rather than just tanky.
fn blink(m: &mut Mob, px: i32, pz: i32) {
    let r = rng();
    let dx = ((r % 9) as i32 - 4) * BLOCK;
    let dz = (((r >> 4) % 9) as i32 - 4) * BLOCK;
    let (nx, nz) = (px + dx, pz + dz);
    let bx = world_to_block_x(nx);
    let bz = world_to_block_z(nz);
    let sy = world::surface_y(bx, bz);
    if sy < 1 || sy > 120 {
        return;
    }
    let feet = sy * BLOCK;
    let (hw, h) = dims(m.kind);
    if aabb_collides_dims(nx, feet, nz, hw, h) {
        return; // no room there; stay put
    }
    m.x = nx;
    m.y = feet;
    m.z = nz;
    m.vy = 0;
}

fn sun_burn(i: usize) {
    let tick = unsafe { BURN_TICK };
    if tick % 20 != 0 {
        return;
    }
    let mut m = unsafe { MOBS[i] };
    if m.kind != ZOMBIE && m.kind != SKELETON {
        return;
    }
    let bx = world_to_block_x(m.x);
    let bz = world_to_block_z(m.z);
    if world_to_block_y(m.y) < world::surface_y(bx, bz) {
        return; // sheltered below the surface
    }
    crate::spawn_particles(m.x, m.y + BLOCK, m.z, (240, 140, 40), 4, (tick as u32) ^ (i as u32), 16);
    m.health -= 1;
    if m.health <= 0 {
        unsafe {
            MOBS[i] = DEAD;
        }
    } else {
        unsafe {
            MOBS[i] = m;
        }
    }
}

#[inline]
fn solid(wx: i32, wy: i32, wz: i32) -> bool {
    let b = world::get(world_to_block_x(wx), world_to_block_y(wy), world_to_block_z(wz));
    b != crate::AIR && b != crate::WATER && b != crate::LAVA
}

fn free_arrow() -> Option<usize> {
    let mut i = 0;
    while i < ARROW_CAP {
        if !unsafe { ARROWS[i].alive } {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn shoot_arrow(sx: i32, sy: i32, sz: i32, px: i32, py: i32, pz: i32) {
    let a = match free_arrow() {
        Some(a) => a,
        None => return,
    };
    let (ex, ey, ez) = (sx, sy + 56, sz);
    let dx = px - ex;
    let dy = (py + 40) - ey;
    let dz = pz - ez;
    let dist = (dx.abs() + dy.abs() + dz.abs()).max(1);
    unsafe {
        ARROWS[a] = Arrow {
            alive: true,
            x: ex,
            y: ey,
            z: ez,
            vx: dx * ARROW_SPEED / dist,
            vy: dy * ARROW_SPEED / dist + 6, // slight arc
            vz: dz * ARROW_SPEED / dist,
            life: 90,
            from_player: false,
        };
    }
}

/// Fire an arrow from the player's eye along a Q12 look direction (the camera
/// forward vector). Player arrows damage mobs, not the player.
pub fn player_shoot(ex: i32, ey: i32, ez: i32, dx: i32, dy: i32, dz: i32) {
    let a = match free_arrow() {
        Some(a) => a,
        None => return,
    };
    let mag = (dx.abs() + dy.abs() + dz.abs()).max(1);
    unsafe {
        ARROWS[a] = Arrow {
            alive: true,
            x: ex,
            y: ey,
            z: ez,
            vx: dx * ARROW_SPEED / mag,
            vy: dy * ARROW_SPEED / mag + 4, // gentle arc
            vz: dz * ARROW_SPEED / mag,
            life: 90,
            from_player: true,
        };
    }
}

/// An in-flight player arrow at (ax,ay,az): damage the first mob it overlaps.
fn arrow_hit_mob(ax: i32, ay: i32, az: i32) -> bool {
    let mut i = 0;
    while i < CAP {
        let mut m = unsafe { MOBS[i] };
        if m.alive {
            let (w, h) = dims(m.kind);
            if (ax - m.x).abs() < w + 6 && (az - m.z).abs() < w + 6 && ay > m.y - 8 && ay < m.y + h {
                m.health -= 4; // arrow damage (Java: 6 at full draw; 4 here)
                m.hurt_cd = 8;
                if m.health <= 0 {
                    record_death(&m);
                    unsafe {
                        XP_DROPS += if is_hostile(m.kind) { 5 } else { 2 };
                        MOBS[i] = DEAD;
                    }
                } else {
                    if !is_hostile(m.kind) {
                        m.state = ST_FLEE;
                        m.timer = 90;
                    }
                    unsafe {
                        MOBS[i] = m;
                    }
                }
                return true;
            }
        }
        i += 1;
    }
    false
}

fn explode(cx: i32, cy: i32, cz: i32, px: i32, py: i32, pz: i32) {
    crate::sfx::explode();
    crate::spawn_particles(cx, cy + BLOCK, cz, (96, 84, 72), 30, (cx ^ cz) as u32, 44);
    world::blast(
        world_to_block_x(cx),
        world_to_block_y(cy + BLOCK),
        world_to_block_z(cz),
        BLAST_R,
    );
    // Java falloff: damage peaks point-blank (lethal from full HP) and reaches
    // 0 at 2*power = 6 blocks. Linear approximation over Manhattan distance.
    let d = (px - cx).abs() + (py - cy).abs() + (pz - cz).abs();
    let dmg = 22 - 22 * d / (BLAST_DMG_R * BLOCK);
    if dmg > 0 {
        unsafe {
            HAZARD_DMG += dmg;
        }
    }
}

fn update_arrows(px: i32, py: i32, pz: i32) {
    let mut i = 0;
    while i < ARROW_CAP {
        let mut a = unsafe { ARROWS[i] };
        if a.alive {
            a.x += a.vx;
            a.y += a.vy;
            a.z += a.vz;
            a.vy -= GRAVITY / 2;
            let mut dead = a.life == 0 || solid(a.x, a.y, a.z);
            if a.life > 0 {
                a.life -= 1;
            }
            if !dead && a.from_player {
                // Player arrow: damage the first mob it overlaps.
                if arrow_hit_mob(a.x, a.y, a.z) {
                    dead = true;
                }
            } else if !dead
                && (a.x - px).abs() < PLAYER_HALF_W + 8
                && (a.z - pz).abs() < PLAYER_HALF_W + 8
                && a.y > py
                && a.y < py + PLAYER_HEIGHT
            {
                // Skeleton arrow: damage the player.
                unsafe {
                    HAZARD_DMG += 4;
                }
                dead = true;
            }
            if dead {
                a.alive = false;
            }
            unsafe {
                ARROWS[i] = a;
            }
        }
        i += 1;
    }
}

/// Arrow + blast damage to the player this frame (taken and cleared).
pub fn hazard_damage() -> i32 {
    unsafe {
        let d = HAZARD_DMG;
        HAZARD_DMG = 0;
        d
    }
}

pub const fn arrow_cap() -> usize {
    ARROW_CAP
}

pub fn arrow_view(i: usize) -> (bool, i32, i32, i32) {
    let a = unsafe { ARROWS[i] };
    (a.alive, a.x, a.y, a.z)
}

fn step_mob(i: usize, px: i32, _py: i32, pz: i32, night: bool) {
    let mut m = unsafe { MOBS[i] };
    if m.hurt_cd > 0 {
        m.hurt_cd -= 1;
    }

    let dx = px - m.x;
    let dz = pz - m.z;
    let dist2 = dx * dx + dz * dz;

    // Despawn far mobs (protects fps + RAM).
    if m.kind != DRAGON && (dx.abs() > DESPAWN_R || dz.abs() > DESPAWN_R) {
        unsafe {
            MOBS[i] = DEAD;
        }
        return;
    }

    // State transitions.
    if m.state == ST_FLEE {
        if m.timer > 0 {
            m.timer -= 1;
        } else {
            m.state = ST_WANDER;
        }
    } else if m.kind == DRAGON {
        // Never loses interest, and daylight means nothing to it.
        m.state = if dist2 < DRAGON_CHASE_R * DRAGON_CHASE_R { ST_CHASE } else { ST_WANDER };
        m.timer = m.timer.wrapping_add(1); // flight phase, drives the height weave
    } else if m.kind == EMBER || m.kind == WAILER {
        // Hostile on sight and unaffected by daylight; they only live in the
        // Inferno, where there is none.
        m.state = if dist2 < CHASE_R * CHASE_R { ST_CHASE } else { ST_WANDER };
        m.timer = m.timer.wrapping_add(1);
    } else if m.kind == WRAITH {
        // Neutral: it only hunts once you have hit it, and hitting it sets
        // ST_CHASE directly. Daylight is irrelevant to it.
        if m.state == ST_CHASE && dist2 > LOSE_R * LOSE_R {
            m.state = ST_WANDER;
        }
    } else if is_hostile(m.kind) && night && dist2 < CHASE_R * CHASE_R {
        m.state = ST_CHASE;
    } else if m.state == ST_CHASE && dist2 > LOSE_R * LOSE_R {
        m.state = ST_WANDER;
    }

    // A tamed wolf follows its owner without needing wheat.
    if m.kind == WOLF && m.love == u16::MAX {
        m.state = if dist2 > (3 * BLOCK) * (3 * BLOCK) { ST_CHASE } else { ST_WANDER };
    }
    // Passive animals follow a player holding wheat (reuses the chase walk).
    if !is_hostile(m.kind) && m.kind != WOLF && m.state != ST_FLEE {
        let lure = unsafe { LURE };
        if lure && dist2 < LURE_R * LURE_R {
            m.state = ST_CHASE;
        } else if m.state == ST_CHASE && (!lure || dist2 > LURE_R * LURE_R) {
            m.state = ST_WANDER;
        }
    }

    // Ranged + explosive hostiles while chasing.
    if m.state == ST_CHASE {
        if m.kind == SKELETON {
            if m.timer > 0 {
                m.timer -= 1;
            } else {
                shoot_arrow(m.x, m.y, m.z, px, _py, pz);
                m.timer = 50;
            }
        } else if m.kind == SAPPER {
            if dist2 < (2 * BLOCK) * (2 * BLOCK) {
                m.fuse += 1;
                if m.fuse == 1 {
                    crate::sfx::sapper_hiss(); // the dreaded tsss
                }
                if m.fuse >= FUSE_MAX {
                    explode(m.x, m.y, m.z, px, _py, pz);
                    unsafe {
                        MOBS[i] = DEAD;
                    }
                    return;
                }
            } else if m.fuse > 0 {
                m.fuse -= 1;
            }
        }
    }

    // Movement intent (greedy axis-step).
    let (mut sx, mut sz) = (0i32, 0i32);
    if m.kind == DRAGON && m.state == ST_CHASE {
        // Java's dragon keeps its distance and circles. Ours used to fly
        // straight at the player and STOP there -- a ground mob is held off by
        // its own AABB, but a phasing flyer just parks inside you, where all its
        // boxes are backface-culled and it is invisible.
        let far = dist2 > DRAGON_ORBIT * DRAGON_ORBIT;
        let v = SPEED * 2;
        if far {
            sx = if dx > 0 { v } else { -v };
            sz = if dz > 0 { v } else { -v };
        } else {
            // Tangent to the player: (dz, -dx), reduced to signs.
            sx = if dz > 0 { v } else { -v };
            sz = if dx > 0 { -v } else { v };
        }
    } else {
    match m.state {
        ST_CHASE => {
            if dx > 4 {
                sx = SPEED;
            } else if dx < -4 {
                sx = -SPEED;
            }
            if dz > 4 {
                sz = SPEED;
            } else if dz < -4 {
                sz = -SPEED;
            }
        }
        ST_FLEE => {
            sx = if dx > 0 { -SPEED } else { SPEED };
            sz = if dz > 0 { -SPEED } else { SPEED };
        }
        _ => {
            // Wander: occasionally pick a new heading; idle otherwise.
            if m.timer > 0 {
                m.timer -= 1;
            } else {
                m.timer = 60 + (rng() % 120) as u16;
            }
            let r = rng();
            if r & 3 != 0 {
                sx = ((r >> 2) % 3) as i32 - 1;
                sz = ((r >> 4) % 3) as i32 - 1;
                sx *= SPEED - 2;
                sz *= SPEED - 2;
            }
        }
    }
    }

    // Gait + facing, from the movement intent the AI just produced. Advancing
    // the phase only while moving is what makes the legs stop when the mob does;
    // a free-running timer marches on the spot.
    if sx != 0 || sz != 0 {
        let speed = (sx.abs() + sz.abs()) as u8;
        m.walk = m.walk.wrapping_add(6 + speed);
        // Nearest quarter turn to the step direction. The renderer needs only
        // which side the face is on, so two bits is the whole story.
        m.facing = if sz.abs() >= sx.abs() {
            if sz > 0 {
                0
            } else {
                2
            }
        } else if sx > 0 {
            1
        } else {
            3
        };
    }

    // Physics: move-and-slide, gravity, jump when blocked.
    let (hw, h) = dims(m.kind);
    let mut blocked = false;
    // A flyer phases through terrain HORIZONTALLY too, not just vertically.
    // The End's obsidian pillars reach above the dragon's cruise height, so
    // with collision on it pins itself against the first one it meets and never
    // reaches the player.
    let phases = is_flyer(m.kind);
    if sx != 0 {
        let nx = m.x + sx;
        if !phases && aabb_collides_dims(nx, m.y, m.z, hw, h) {
            blocked = true;
        } else {
            m.x = nx;
        }
    }
    if sz != 0 {
        let nz = m.z + sz;
        if !phases && aabb_collides_dims(m.x, m.y, nz, hw, h) {
            blocked = true;
        } else {
            m.z = nz;
        }
    }
    if is_flyer(m.kind) {
        // No gravity and no block collision: it phases through the pillars,
        // like Java's dragon. `timer` doubles as the flight phase.
        // Embers and wailers hover just off the ground rather than at the
        // dragon's cruise, which is above the Inferno roof.
        let cruise = if m.kind == DRAGON { DRAGON_CRUISE } else { m.y / BLOCK * BLOCK + 2 * BLOCK };
        let want = cruise + ((m.timer as i32 % 64) - 32) * 2;
        if m.y < want {
            m.y += 4;
        } else if m.y > want {
            m.y -= 4;
        }
        m.vy = 0;
        m.on_ground = false;
    } else {
        m.vy = (m.vy - GRAVITY).max(TERMINAL_VY);
        let ny = m.y + m.vy;
        if aabb_collides_dims(m.x, ny, m.z, hw, h) {
            if m.vy < 0 {
                m.y = (world_to_block_y(ny) + 1) * BLOCK;
                m.on_ground = true;
            }
            m.vy = 0;
        } else {
            m.y = ny;
            m.on_ground = false;
        }
    }
    if blocked {
        if m.kind == SPIDER && m.state == ST_CHASE {
            m.vy = GRAVITY + 5; // spiders climb walls toward the player
        } else if m.on_ground {
            m.vy = JUMP_VY;
        }
    }

    unsafe {
        MOBS[i] = m;
    }
}

/// Hostile contact damage to the player this frame (max over touching mobs).
pub fn contact_damage(px: i32, py: i32, pz: i32) -> i32 {
    let mut dmg = 0;
    let mut i = 0;
    while i < CAP {
        let m = unsafe { MOBS[i] };
        if m.alive && is_hostile(m.kind) {
            let dx = (px - m.x).abs();
            let dz = (pz - m.z).abs();
            let dy = (py - m.y).abs();
            if dx < BLOCK && dz < BLOCK && dy < 2 * BLOCK {
                let d = if m.kind == SAPPER { 6 } else { 3 };
                if d > dmg {
                    dmg = d;
                }
            }
        }
        i += 1;
    }
    dmg
}

/// Player melee: damage the closest mob roughly along the look ray within reach.
/// Returns true on a hit. Survivors of a passive hit flee; killed mobs vanish.
pub fn melee(px: i32, py: i32, pz: i32, fx: i32, fz: i32, reach: i32, damage: i16) -> bool {
    let mut best = CAP;
    let mut best_d = reach * reach;
    let mut i = 0;
    while i < CAP {
        let m = unsafe { MOBS[i] };
        if m.alive {
            let dx = m.x - px;
            let dz = m.z - pz;
            let dy = (m.y - py).abs();
            // In front (dot with facing > 0) and within reach + vertical band.
            if dx * fx + dz * fz > 0 && dy < 2 * BLOCK {
                let d2 = dx * dx + dz * dz;
                if d2 < best_d {
                    best_d = d2;
                    best = i;
                }
            }
        }
        i += 1;
    }
    if best == CAP {
        return false;
    }
    let mut m = unsafe { MOBS[best] };
    m.health -= damage;
    m.hurt_cd = 8;
    if m.kind == WRAITH {
        m.state = ST_CHASE; // neutral no longer: you started it
        blink(&mut m, px, pz);
    }
    // Knockback away from the player.
    m.x += if m.x >= px { SPEED * 3 } else { -SPEED * 3 };
    m.z += if m.z >= pz { SPEED * 3 } else { -SPEED * 3 };
    if m.health <= 0 {
        record_death(&m);
        unsafe {
            XP_DROPS += if is_hostile(m.kind) { 5 } else { 2 };
        }
        unsafe {
            MOBS[best] = DEAD;
        }
    } else {
        if !is_hostile(m.kind) {
            m.state = ST_FLEE;
            m.timer = 90;
        }
        unsafe {
            MOBS[best] = m;
        }
    }
    true
}

pub fn reset() {
    let mut i = 0;
    while i < CAP {
        unsafe {
            MOBS[i] = DEAD;
        }
        i += 1;
    }
    let mut a = 0;
    while a < ARROW_CAP {
        unsafe {
            ARROWS[a] = NO_ARROW;
        }
        a += 1;
    }
}

/// Place one mob of `kind` on the ground at world coords (sx, sz).
fn spawn_at(kind: u8, sx: i32, sz: i32) {
    let s = match free_slot() {
        Some(s) => s,
        None => return,
    };
    let bx = world_to_block_x(sx);
    let bz = world_to_block_z(sz);
    let sy = world::surface_y(bx, bz);
    if sy < 2 || sy > 100 {
        return;
    }
    let (hw, h) = dims(kind);
    let feet = sy * BLOCK;
    if aabb_collides_dims(sx, feet, sz, hw, h) {
        return;
    }
    unsafe {
        MOBS[s] = Mob {
            kind,
            alive: true,
            on_ground: false,
            x: sx,
            y: feet,
            z: sz,
            vy: 0,
            health: max_health(kind),
            state: ST_WANDER,
            timer: (rng() % 120) as u16,
            fuse: 0,
            hurt_cd: 0,
            love: 0,
            walk: 0,
            facing: 0,
        };
    }
}

/// Seed a few passive mobs in front of the player (+Z at spawn) at world start.
/// Despawn everything (mobs + arrows) -- the "NEW WORLD" reset.
pub fn clear() {
    unsafe {
        MOBS = [DEAD; CAP];
        ARROWS = [NO_ARROW; ARROW_CAP];
    }
}

pub fn populate(px: i32, pz: i32) {
    let mut k = 0i32;
    while k < 4 {
        spawn_at((k as u8) % 4, px + (k - 2) * 2 * BLOCK, pz + (4 + k) * BLOCK);
        k += 1;
    }
}

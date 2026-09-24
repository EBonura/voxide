//! Tuning lab (feature `tune-lab`, never shipped): a scripted timeline that
//! builds a test arena in the sky over the spawn and walks the player through
//! movement and survival scenarios, so the effective rates of the tuning
//! constants can be read off a headless route log instead of inferred.
//!
//! The script runs on the sim-step clock (one step per elapsed vblank), the
//! same clock the constants under test run on. VOXIDE_LAB_P mirrors the player every
//! step; `frontend launch --route-watch-u32` reads it. Layout (i32 words):
//!  0 step  1 frame  2 phase  3 x  4 y  5 z  6 vy  7 health  8 air  9 food
//! 10 burn 11 hurt_cd 12 exhaustion 13 target block (mining/TNT) 14 top-ups
//! 15 on_ground | sprinting<<1 | sneaking<<2

use crate::*;

#[no_mangle]
pub static mut VOXIDE_LAB_P: [i32; 16] = [0; 16];
static mut T: i32 = 0; // sim steps since gameplay began
static mut F: i32 = 0; // rendered frames since gameplay began
static mut ORG: (i32, i32) = (0, 0); // arena origin, block coords
static mut TOPUPS: i32 = 0;

const BY: i32 = 50; // platform block y; you stand at (BY + 1) * BLOCK

// Phase starts, in sim steps (60 a second).
const P_WALK: i32 = 30;
const P_SPRINT: i32 = 180;
const P_SNEAK: i32 = 350;
const P_JUMP: i32 = 520;
const P_FALL: i32 = 800;
const P_LADDER: i32 = 1000;
const P_SLIDE: i32 = 1250;
const P_SWIM: i32 = 1400;
const P_DROWN: i32 = 1600;
const P_LAVA: i32 = 3000;
const P_LAVA_OUT: i32 = 3180;
const P_REGEN: i32 = 3900;
const P_STARVE: i32 = 4500;
const P_HUNGER: i32 = 5100;
const P_CONTACT: i32 = 6400;
const P_POTION: i32 = 6600;
const P_MINE: i32 = 6900;
const P_TNT: i32 = 7300;
const P_END: i32 = 7600;

fn phase(t: i32) -> i32 {
    let starts = [
        P_WALK, P_SPRINT, P_SNEAK, P_JUMP, P_FALL, P_LADDER, P_SLIDE, P_SWIM, P_DROWN, P_LAVA,
        P_LAVA_OUT, P_REGEN, P_STARVE, P_HUNGER, P_CONTACT, P_POTION, P_MINE, P_TNT, P_END,
    ];
    let mut p = 0;
    while p < starts.len() && t >= starts[p] {
        p += 1;
    }
    p as i32
}

/// The game's pad poll with the script's buttons in place of the pad's once
/// gameplay has begun (menus and the hand-off into the world see the pad).
pub fn poll_port1() -> psx_pad::PadState {
    let mut s = psx_pad::poll_port1();
    s.buttons = pad(s.buttons);
    s
}

/// Buttons the script holds this frame (gameplay polls once a frame).
fn pad(real: ButtonState) -> ButtonState {
    let t = unsafe { T };
    if t == 0 {
        return real; // still in the menu hand-off
    }
    unsafe { F += 1 };
    let mut b = 0u16;
    let within = |a: i32, n: i32| t >= a && t < a + n;
    if within(P_WALK + 10, 120) {
        b |= button::UP;
    }
    if within(P_SPRINT + 20, 120) {
        b |= button::UP;
    }
    if within(P_SPRINT + 24, 4) {
        b |= button::L3;
    }
    if within(P_SNEAK + 20, 120) {
        b |= button::UP | button::CIRCLE;
    }
    if within(P_JUMP + 40, 4) || within(P_JUMP + 160, 4) {
        b |= button::CROSS;
    }
    if within(P_LADDER + 10, 200) {
        b |= button::UP;
    }
    if within(P_SWIM + 20, 100) {
        b |= button::CROSS;
    }
    if within(P_MINE + 20, 150) || within(P_MINE + 220, 150) {
        b |= button::R2;
    }
    ButtonState::from_bits(b)
}

fn put(x: i32, y: i32, z: i32, b: u8) {
    world::set_raw_pub(x, y, z, b);
}

/// Platform 5 wide and 44 long along +Z; a water well and a lava well (1x1,
/// five deep, stone-lined) east of it; a ladder against a wall to the west.
fn build(ox: i32, oz: i32) {
    let mut z = oz - 2;
    while z < oz + 44 {
        let mut x = ox - 2;
        while x <= ox + 2 {
            put(x, BY, z, STONE);
            let mut y = BY + 1;
            while y < BY + 6 {
                put(x, y, z, AIR);
                y += 1;
            }
            x += 1;
        }
        z += 1;
    }
    let wells = [(ox + 6, WATER), (ox + 10, LAVA)];
    let mut k = 0;
    while k < 2 {
        let (wx, fl) = wells[k];
        let wz = oz + 5;
        let mut y = BY - 5;
        while y <= BY + 1 {
            let mut dz = -1;
            while dz <= 1 {
                let mut dx = -1;
                while dx <= 1 {
                    let inner = dx == 0 && dz == 0;
                    let b = if inner && y > BY - 5 {
                        if y <= BY {
                            fl
                        } else {
                            AIR
                        }
                    } else if y <= BY {
                        STONE
                    } else {
                        AIR
                    };
                    put(wx + dx, y, wz + dz, b);
                    dx += 1;
                }
                dz += 1;
            }
            y += 1;
        }
        k += 1;
    }
    // Ladder column at (ox-6, BY+1..BY+8, oz+5), wall behind it on +Z.
    let (lx, lz) = (ox - 6, oz + 5);
    put(lx, BY, lz, STONE);
    let mut y = BY + 1;
    while y <= BY + 10 {
        put(lx, y, lz, if y <= BY + 8 { LADDER } else { AIR });
        put(lx, y, lz + 1, STONE);
        y += 1;
    }
    world::remesh_loaded();
}

fn place(p: &mut Player, bx: i32, feet: i32, bz: i32) {
    p.x = bx * BLOCK + BLOCK / 2;
    p.y = feet;
    p.z = bz * BLOCK + BLOCK / 2;
    p.vy = 0;
    p.fall_peak = feet;
    p.yaw = 0;
    p.pitch = 0;
}

/// One script step, at the end of each sim step's survival update.
pub fn step(p: &mut Player) {
    let t = unsafe { T };
    let (ox, oz) = unsafe { ORG };
    let stand = (BY + 1) * BLOCK;
    if t == 0 {
        let (bx, bz) = (world_to_block_x(p.x), world_to_block_z(p.z));
        unsafe { ORG = (bx, bz) };
        build(bx, bz);
        mob::reset();
        place(p, bx, stand, bz);
        p.health = MAX_HEALTH;
        p.food = MAX_FOOD;
        p.pick = 0;
    }
    let (ox, oz) = if t == 0 { unsafe { ORG } } else { (ox, oz) };
    match t {
        P_WALK | P_SPRINT | P_SNEAK | P_JUMP => place(p, ox, stand, oz),
        P_FALL => {
            p.health = MAX_HEALTH;
            place(p, ox, stand + 10 * BLOCK, oz + 2);
        }
        P_LADDER => place(p, ox - 6, stand, oz + 5),
        P_SLIDE => place(p, ox - 6, stand + 5 * BLOCK, oz + 5),
        P_SWIM | P_DROWN => {
            p.health = MAX_HEALTH;
            place(p, ox + 6, (BY - 4) * BLOCK, oz + 5);
            p.air = MAX_AIR;
        }
        P_LAVA => {
            p.health = MAX_HEALTH;
            p.hurt_cd = 0;
            place(p, ox + 10, (BY - 4) * BLOCK, oz + 5);
        }
        P_LAVA_OUT => place(p, ox, stand, oz),
        P_REGEN => {
            p.health = 10;
            p.food = MAX_FOOD;
            p.regen_delay = 0;
            p.regen_tick = 0;
            p.exhaustion = 0;
            p.burn = 0;
        }
        P_STARVE => {
            p.health = MAX_HEALTH;
            p.food = 0;
            p.regen_delay = 0;
            p.regen_tick = 0;
        }
        P_HUNGER => {
            p.health = MAX_HEALTH;
            p.food = 10;
            p.exhaustion = 0;
            p.regen_delay = 0;
            p.regen_tick = 0;
        }
        P_CONTACT => {
            p.health = MAX_HEALTH;
            p.food = 10;
            p.hurt_cd = 0;
        }
        P_POTION => {
            mob::reset();
            p.health = 6;
            p.food = 10;
            drink_potion(p, POTION_REGEN);
        }
        P_MINE => {
            p.eff_regen = 0;
            p.health = MAX_HEALTH;
            place(p, ox, stand, oz);
            put(ox, BY + 2, oz + 1, STONE);
            world::remesh_loaded();
        }
        x if x == P_MINE + 200 => {
            put(ox, BY + 2, oz + 1, DIRT);
            world::remesh_loaded();
        }
        P_TNT => {
            put(ox + 20, BY + 1, oz + 20, TNT);
            ignite_tnt(ox + 20, BY + 1, oz + 20);
        }
        _ => {}
    }
    if t >= P_CONTACT && t < P_POTION {
        mob::lab_pin(mob::ZOMBIE, p.x, p.y, p.z);
    }
    // Keep the player alive through the hazard phases; count the top-ups.
    if p.health < 8 && (t < P_REGEN || (t >= P_CONTACT && t < P_POTION)) {
        p.health = MAX_HEALTH;
        unsafe { TOPUPS += 1 };
    }
    let target = if t >= P_TNT {
        world::get(ox + 20, BY + 1, oz + 20)
    } else {
        world::get(ox, BY + 2, oz + 1)
    };
    unsafe {
        T = t + 1;
        core::ptr::write_volatile(
            &raw mut VOXIDE_LAB_P,
            [
            t,
            F,
            phase(t),
            p.x,
            p.y,
            p.z,
            p.vy,
            p.health,
            p.air,
            p.food,
            p.burn,
            p.hurt_cd,
            p.exhaustion,
            target as i32,
            TOPUPS,
            p.on_ground as i32 | (p.sprinting as i32) << 1 | (p.sneaking as i32) << 2,
            ],
        );
    }
}

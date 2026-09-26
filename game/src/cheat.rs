//! Hidden cheat menu, for testing on hardware. Opened from the in-game
//! OPTIONS menu (START) by holding L1 + R1 and pressing SELECT; nothing on
//! screen advertises it. Menu code, so it is built for size and kept out of
//! the gameplay loop (MIPS branches reach +/-128 KB and the loop is at the
//! edge).
//!
//! Using any cheat but the XYZ/FPS readout marks the world: the save's world
//! flags carry bit 1 from then on (save.rs), and a world that never used one
//! saves exactly as before.

use crate::*;

/// A cheat changed this world (saved in the world flags).
static mut USED: bool = false;
/// Health, hunger and breath held full; nothing can kill you.
static mut GOD: bool = false;
/// Block coordinates and frame rate in the top-left corner.
static mut HUD: bool = false;
/// Row selections that live across openings.
static mut TIME_SEL: usize = 1;
static mut MOB_SEL: usize = 0;
/// Outcome of the last action, shown under the rows.
static mut MSG: &str = "";

const R_FLY: usize = 0;
const R_GOD: usize = 1;
const R_HUD: usize = 2;
const R_GIVE: usize = 3;
const R_TIME: usize = 4;
const R_SPAWN_MOB: usize = 5;
const R_HOME: usize = 6;
const R_INFERNO: usize = 7;
const R_VOID: usize = 8;
const R_OVERWORLD: usize = 9;
pub const ROWS: [&str; 10] = [
    "FLY (NO CLIP)",
    "GOD MODE",
    "SHOW XYZ + FPS",
    "GIVE EVERYTHING",
    "TIME",
    "SPAWN",
    "TO SPAWN POINT",
    "TO INFERNO",
    "TO VOID",
    "TO OVERWORLD",
];

/// Java's clock: sunrise 0, noon 6,000, sunset 12,000, midnight 18,000
/// (minecraft.wiki/w/Daylight_cycle).
const TIMES: [(&str, i32); 4] = [
    ("SUNRISE", 0),
    ("NOON", 6_000),
    ("SUNSET", 12_000),
    ("MIDNIGHT", 18_000),
];

const MOBS: [(&str, u8); 15] = [
    ("PIG", mob::PIG),
    ("COW", mob::COW),
    ("SHEEP", mob::SHEEP),
    ("CHICKEN", mob::CHICKEN),
    ("ZOMBIE", mob::ZOMBIE),
    ("SKELETON", mob::SKELETON),
    ("SAPPER", mob::SAPPER),
    ("SPIDER", mob::SPIDER),
    ("WRAITH", mob::WRAITH),
    ("WOLF", mob::WOLF),
    ("VILLAGER", mob::VILLAGER),
    ("EMBER", mob::EMBER),
    ("WAILER", mob::WAILER),
    ("CHARRED SK", mob::CHARRED_SK),
    ("DRAGON", mob::DRAGON),
];

/// Items the hotbar cannot select, stocked alongside PLACEABLE.
const MATERIALS: [u8; 21] = [
    COAL_ORE,
    IRON_ORE,
    GOLD_ORE,
    DIAMOND_ORE,
    IRON_INGOT,
    STICK,
    STRING,
    ARROW,
    BONE,
    GUNPOWDER,
    CLAY,
    BREAD,
    RAW_MEAT,
    COOKED_MEAT,
    SUGAR_CANE,
    EMBER_CAP,
    EMBER_ROD,
    VOID_PEARL,
    WAILER_TEAR,
    MAGMA_PASTE,
    POTION_AWKWARD,
];

/// What the gameplay loop has to do for the chosen row, the actions that
/// need its locals (the world clock, the framebuffer for a loading screen).
pub enum Act {
    None,
    /// Set the clock to this point of Java's 24,000-tick day.
    Time(i32),
    /// Cross to this dimension, as a portal would.
    Travel(u8),
    /// Back to the overworld spawn point (the bed, if one was slept in).
    Home,
}

/// The hidden combo: SELECT pressed while L1 and R1 are held.
pub fn combo(pad: ButtonState, prev: ButtonState) -> bool {
    pad.is_held(button::L1) && pad.is_held(button::R1) && pad.pressed_since(prev, button::SELECT)
}

pub fn used() -> bool {
    unsafe { USED }
}

/// A load sets the flag from the save. God mode still on from before the
/// load marks the loaded world as soon as it is saved.
pub fn set_used(on: bool) {
    unsafe { USED = on || GOD };
}

pub fn god() -> bool {
    unsafe { GOD }
}

pub fn hud() -> bool {
    unsafe { HUD }
}

/// Everything back to a clean game (a fresh world).
pub fn reset() {
    unsafe {
        USED = false;
        GOD = false;
        MSG = "";
    }
}

/// Keep health, hunger and breath full. Called every sim tick.
pub fn hold_god(p: &mut Player) {
    p.health = MAX_HEALTH;
    p.food = MAX_FOOD;
    p.air = MAX_AIR;
    p.burn = 0;
}

fn mark(msg: &'static str) {
    unsafe {
        USED = true;
        MSG = msg;
    }
}

/// D-pad LEFT/RIGHT step a choice row, CROSS acts on the selected row.
#[inline(never)]
#[optimize(size)]
pub fn input(pad: ButtonState, prev: ButtonState, sel: usize, player: &mut Player) -> Act {
    let dir = if pad.pressed_since(prev, button::LEFT) {
        -1
    } else if pad.pressed_since(prev, button::RIGHT) {
        1
    } else {
        0
    };
    if dir != 0 {
        unsafe {
            if sel == R_TIME {
                TIME_SEL = (TIME_SEL as i32 + dir).rem_euclid(TIMES.len() as i32) as usize;
            } else if sel == R_SPAWN_MOB {
                MOB_SEL = (MOB_SEL as i32 + dir).rem_euclid(MOBS.len() as i32) as usize;
            }
        }
        sfx::blip();
    }
    if !pad.pressed_since(prev, button::CROSS) {
        return Act::None;
    }
    sfx::confirm();
    match sel {
        R_FLY => {
            player.fly = !player.fly;
            player.vy = 0;
            mark(if player.fly { "FLYING" } else { "WALKING" });
        }
        R_GOD => {
            unsafe { GOD = !GOD };
            mark(if god() { "GOD MODE ON" } else { "GOD MODE OFF" });
        }
        R_HUD => unsafe {
            HUD = !HUD;
            MSG = "";
        },
        R_GIVE => {
            give_all(player);
            mark("64 OF EVERYTHING, DIAMOND KIT");
        }
        R_TIME => {
            let (name, t) = TIMES[unsafe { TIME_SEL }];
            mark(name);
            return Act::Time(t);
        }
        R_SPAWN_MOB => {
            let (name, kind) = MOBS[unsafe { MOB_SEL }];
            spawn(player, kind);
            mark(name);
        }
        R_HOME => {
            mark("");
            return Act::Home;
        }
        R_INFERNO | R_VOID | R_OVERWORLD => {
            let to = match sel {
                R_INFERNO => world::DIM_INFERNO,
                R_VOID => world::DIM_VOID,
                _ => world::DIM_OVERWORLD,
            };
            if to == world::dimension() {
                unsafe { MSG = "ALREADY HERE" };
            } else {
                mark("");
                return Act::Travel(to);
            }
        }
        _ => {}
    }
    Act::None
}

/// A creative-style stock: 64 of every block and item, the diamond tier of
/// every tool and the diamond armour.
fn give_all(p: &mut Player) {
    let mut i = 0;
    while i < PLACEABLE.len() + MATERIALS.len() {
        let k = if i < PLACEABLE.len() {
            PLACEABLE[i]
        } else {
            MATERIALS[i - PLACEABLE.len()]
        };
        let have = unsafe { INV[k as usize] };
        if have < 64 {
            inv_give(k, 64 - have);
        }
        i += 1;
    }
    p.pick = 4;
    p.axe = 4;
    p.shovel = 4;
    p.sword = 4;
    p.armor = 2;
    hotbar_sync(p);
}

/// Stand a mob three blocks in front of the player, on the first free
/// height at or above the player's feet.
fn spawn(p: &Player, kind: u8) {
    if kind == mob::DRAGON {
        mob::spawn_dragon(p.x, p.z);
        return;
    }
    let sy = sincos::sin_q12(p.yaw);
    let cy = sincos::cos_q12(p.yaw);
    let x = p.x + ((sy * 3 * BLOCK) >> 12);
    let z = p.z + ((cy * 3 * BLOCK) >> 12);
    mob::cheat_spawn(kind, x, p.y, z);
}

/// The readout: block coordinates and frames per second.
#[inline(never)]
#[optimize(size)]
pub fn draw_hud(font: &FontAtlas, p: &Player, fps: u32) {
    let mut buf = [0u8; 32];
    let mut n = 0;
    let (bx, by, bz) = (
        world_to_block_x(p.x),
        world_to_block_y(p.y),
        world_to_block_z(p.z),
    );
    for (i, v) in [bx, by, bz].into_iter().enumerate() {
        if i > 0 {
            buf[n] = b' ';
            n += 1;
        }
        n = put_int(&mut buf, n, v);
    }
    let xyz = unsafe { core::str::from_utf8_unchecked(&buf[..n]) };
    ui_text(font, 6, 30, "XYZ", (0xE0, 0xE0, 0x60));
    ui_text(font, 38, 30, xyz, (0xE0, 0xE0, 0x60));
    let mut fb = [0u8; 4];
    let k = put_int(&mut fb, 0, fps as i32);
    ui_text(font, 6, 40, "FPS", (0xE0, 0xE0, 0x60));
    ui_text(font, 38, 40, unsafe { core::str::from_utf8_unchecked(&fb[..k]) }, (0xE0, 0xE0, 0x60));
}

/// Append `v` in decimal at `n`; returns the new length.
fn put_int(buf: &mut [u8], mut n: usize, v: i32) -> usize {
    if v < 0 {
        buf[n] = b'-';
        n += 1;
    }
    let mut u = v.unsigned_abs();
    let mut digits = [0u8; 10];
    let mut d = 0;
    loop {
        digits[d] = b'0' + (u % 10) as u8;
        d += 1;
        u /= 10;
        if u == 0 {
            break;
        }
    }
    while d > 0 {
        d -= 1;
        buf[n] = digits[d];
        n += 1;
    }
    n
}

#[inline(never)]
#[optimize(size)]
pub fn draw(font: &FontAtlas, sel: usize, player: &Player) {
    menu_frame(font, if used() { "CHEATS - WORLD MARKED" } else { "CHEATS" });
    let mut hx = hint_item(font, MENU_TEXT_X, MENU_HINT_Y, "X", PS_CROSS, "DO");
    hx = hint_item(font, hx, MENU_HINT_Y, "< >", PS_KEY, "CHOOSE");
    hint_item(font, hx, MENU_HINT_Y, "O", PS_CIRCLE, "CLOSE");
    let n = ROWS.len();
    let vis = 8;
    let start = list_window(n, vis, sel);
    menu_scroll_hint(font, n, vis, start, MENU_HINT_X, MENU_ROWS_Y);
    let mut j = 0;
    while j < vis && start + j < n {
        let i = start + j;
        let y = menu_row(j, i == sel);
        let color = if i == sel { MC_LABEL_SEL } else { MC_LABEL };
        ui_text(font, MENU_TEXT_X, y, ROWS[i], color);
        let on = match i {
            R_FLY => Some(player.fly),
            R_GOD => Some(god()),
            R_HUD => Some(hud()),
            _ => None,
        };
        if let Some(on) = on {
            let (label, tint) = if on {
                ("ON", (0x70, 0xE0, 0x70))
            } else {
                ("OFF", MC_LABEL_OFF)
            };
            ui_text(font, 230, y, label, tint);
        } else if i == R_TIME || i == R_SPAWN_MOB {
            let v = if i == R_TIME {
                TIMES[unsafe { TIME_SEL }].0
            } else {
                MOBS[unsafe { MOB_SEL }].0
            };
            ui_text(font, 278 - v.len() as i16 * 8, y, v, (0x70, 0xE0, 0x70));
        }
        j += 1;
    }
    let msg = unsafe { MSG };
    if !msg.is_empty() {
        draw_centered(font, 170, msg, (0xF0, 0xE0, 0x80));
    }
}

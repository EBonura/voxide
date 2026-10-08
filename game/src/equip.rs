//! Equipment: the armour worn in four slots, the off-hand item and the weapon
//! swung at mobs, with Java's numbers (minecraft.wiki/w/Armor, /w/Damage,
//! /w/Tool). The inventory screen's player page (inv.rs) is the way in; this
//! file holds the rules so the screen stays a view.
//!
//! Armour pieces are ordinary item kinds (ARMOR0..) counted in INV like
//! everything else. Wearing one takes it out of INV and puts the kind in
//! `Player::worn`; taking it off puts it back. INV carries no per-item state,
//! so a piece taken off with wear left leaves its uses in SPARE, one worn copy
//! per kind, and the next one worn takes them back. A second worn copy of the
//! same kind is refused rather than quietly repaired.

use crate::*;

/// The weapon choices, in the order the screen lists them: bare fist, then the
/// four tool classes a mob can be hit with.
pub const WEAPONS: [u8; 5] = [TOOL_NONE, TOOL_SWORD, TOOL_AXE, TOOL_PICK, TOOL_SHOVEL];

/// Attack damage in hundredths of a point by tier (0 = no tool, 1 wood .. 4
/// diamond), per weapon in WEAPONS order. Java: fist 1; swords 4 5 6 7; axes
/// 7 9 9 9; pickaxes 2 3 4 5; shovels 2.5 3.5 4.5 5.5.
const DAMAGE100: [[i32; 5]; 5] = [
    [100, 100, 100, 100, 100],
    [100, 400, 500, 600, 700],
    [100, 700, 900, 900, 900],
    [100, 200, 300, 400, 500],
    [100, 250, 350, 450, 550],
];

/// Attack speed in tenths by tier. Java: fist 4; swords 1.6; axes 0.8 0.8 0.9
/// 1.0; pickaxes 1.2; shovels 1.
const SPEED10: [[i32; 5]; 5] = [
    [40, 40, 40, 40, 40],
    [40, 16, 16, 16, 16],
    [40, 8, 8, 9, 10],
    [40, 12, 12, 12, 12],
    [40, 10, 10, 10, 10],
];

/// The index of a tool class in WEAPONS.
fn weapon_index(class: u8) -> usize {
    match class {
        TOOL_SWORD => 1,
        TOOL_AXE => 2,
        TOOL_PICK => 3,
        TOOL_SHOVEL => 4,
        _ => 0,
    }
}

/// The class and tier swung at a mob: the chosen weapon while you own it,
/// else the bare fist.
pub fn weapon_of(p: &Player) -> (u8, u8) {
    let tier = tool_tier(p, p.weapon);
    if p.weapon != TOOL_NONE && tier > 0 {
        (p.weapon, tier)
    } else {
        (TOOL_NONE, 0)
    }
}

/// Base attack damage of the weapon in hand, hundredths.
pub fn damage100(p: &Player) -> i32 {
    let (c, t) = weapon_of(p);
    DAMAGE100[weapon_index(c)][t as usize]
}

/// Six times the cooldown in game ticks: 6 x 20 / speed, with speed in tenths.
pub fn six_t(p: &Player) -> i32 {
    let (c, t) = weapon_of(p);
    1200 / SPEED10[weapon_index(c)][t as usize]
}

/// The weapon's damage and speed for the screen, hundredths and tenths.
pub fn stats(class: u8, tier: u8) -> (i32, i32) {
    let i = weapon_index(class);
    (DAMAGE100[i][tier as usize], SPEED10[i][tier as usize])
}

/// Durability a hit costs: 1 for a sword, 2 for the tools that are not
/// weapons (minecraft.wiki/w/Durability).
pub fn wear_per_hit(class: u8) -> u16 {
    if class == TOOL_SWORD {
        1
    } else {
        2
    }
}

/// Worn uses of a taken-off armour piece, by kind - ARMOR0. 0 = every copy
/// held is new.
static mut SPARE: [u16; ARMOR_KINDS as usize] = [0; ARMOR_KINDS as usize];

pub fn spare_get() -> [u16; ARMOR_KINDS as usize] {
    unsafe { SPARE }
}

pub fn spare_set(v: [u16; ARMOR_KINDS as usize]) {
    unsafe { SPARE = v };
}

/// Wear the armour piece `kind` from the inventory. A piece already in that
/// slot goes back to the inventory first. Err is the line for the screen.
pub fn wear(p: &mut Player, kind: u8) -> Result<(), &'static str> {
    let Some((t, slot)) = armor_piece(kind) else {
        return Err("NOT ARMOR");
    };
    if unsafe { INV[kind as usize] } == 0 {
        return Err("YOU HAVE NONE");
    }
    if p.worn[slot] != AIR {
        take_off(p, slot)?;
    }
    let k = (kind - ARMOR0) as usize;
    unsafe {
        INV[kind as usize] -= 1;
        p.armor_dur[slot] = if SPARE[k] > 0 { SPARE[k] } else { ARMOR_DUR[t][slot] };
        SPARE[k] = 0;
    }
    p.worn[slot] = kind;
    Ok(())
}

/// Take the piece in `slot` off, back into the inventory.
pub fn take_off(p: &mut Player, slot: usize) -> Result<(), &'static str> {
    let kind = p.worn[slot];
    let Some((t, _)) = armor_piece(kind) else {
        return Ok(());
    };
    let k = (kind - ARMOR0) as usize;
    let left = p.armor_dur[slot];
    unsafe {
        if left < ARMOR_DUR[t][slot] {
            if SPARE[k] > 0 {
                return Err("YOU HOLD A WORN ONE ALREADY");
            }
            SPARE[k] = left;
        }
        INV[kind as usize] = INV[kind as usize].saturating_add(1);
    }
    p.worn[slot] = AIR;
    p.armor_dur[slot] = 0;
    Ok(())
}

/// Everything worn goes back to the inventory (a death keeps the inventory).
pub fn take_all_off(p: &mut Player) {
    let mut s = 0;
    while s < 4 {
        if take_off(p, s).is_err() {
            // A second worn copy: it is still the same kind, so keep the
            // count and let the older worn one stand for both.
            unsafe { INV[p.worn[s] as usize] = INV[p.worn[s] as usize].saturating_add(1) };
            p.worn[s] = AIR;
            p.armor_dur[s] = 0;
        }
        s += 1;
    }
}

/// Swap the held hotbar item with the off-hand one (Java's swap-hands key).
/// A kind lives in one place, so the two just trade.
pub fn swap_hands(p: &mut Player) {
    unsafe {
        let sel = HOTBAR_SEL;
        let held = HOTBAR[sel];
        if held == AIR && p.offhand == AIR {
            return;
        }
        HOTBAR[sel] = p.offhand;
        p.offhand = held;
        p.selected = HOTBAR[sel];
    }
    p.attack_t = 0; // switching items restarts the attack cooldown
    sfx::blip();
}

/// Drop an off-hand reference that lost its stack, as hotbar_sync does.
pub fn sync(p: &mut Player) {
    if p.offhand != AIR && unsafe { INV[p.offhand as usize] } == 0 {
        p.offhand = AIR;
    }
}

// -- the figure --------------------------------------------------------------

const SKIN: (u8, u8, u8) = (196, 142, 104);
const HAIR: (u8, u8, u8) = (74, 50, 30);
const SHIRT: (u8, u8, u8) = (0, 150, 160);
const PANTS: (u8, u8, u8) = (58, 52, 140);
const SHOE: (u8, u8, u8) = (84, 84, 90);

/// A shaded armour colour: (light, mid, dark) for a tier.
fn metal(tier: usize) -> [(u8, u8, u8); 3] {
    if tier == 0 {
        [(226, 226, 234), (170, 170, 180), (112, 112, 124)]
    } else {
        [(170, 250, 240), (84, 214, 204), (36, 140, 140)]
    }
}

/// The player seen from the front at 2 px a model unit, 32 wide by 64 tall,
/// top-left at (x, y), with whatever armour is worn drawn over it. Flat
/// rectangles only: the sim runs behind the screen but this is a few quads.
#[inline(never)]
#[optimize(size)]
pub fn draw_figure(x: i16, y: i16, p: &Player) {
    // Bare player: head, body, two arms, two legs.
    rect(x + 8, y, 16, 16, SKIN.0, SKIN.1, SKIN.2);
    rect(x + 8, y, 16, 5, HAIR.0, HAIR.1, HAIR.2);
    rect(x + 8, y + 5, 3, 3, HAIR.0, HAIR.1, HAIR.2);
    rect(x + 21, y + 5, 3, 3, HAIR.0, HAIR.1, HAIR.2);
    rect(x + 10, y + 8, 4, 2, 240, 240, 240);
    rect(x + 18, y + 8, 4, 2, 240, 240, 240);
    rect(x + 12, y + 8, 2, 2, 70, 60, 130);
    rect(x + 18, y + 8, 2, 2, 70, 60, 130);
    rect(x + 14, y + 12, 4, 1, 150, 96, 70);
    rect(x + 8, y + 16, 16, 24, SHIRT.0, SHIRT.1, SHIRT.2);
    rect(x, y + 16, 8, 24, SHIRT.0, SHIRT.1, SHIRT.2);
    rect(x + 24, y + 16, 8, 24, SHIRT.0, SHIRT.1, SHIRT.2);
    rect(x, y + 34, 8, 6, SKIN.0, SKIN.1, SKIN.2);
    rect(x + 24, y + 34, 8, 6, SKIN.0, SKIN.1, SKIN.2);
    rect(x + 8, y + 40, 8, 24, PANTS.0, PANTS.1, PANTS.2);
    rect(x + 16, y + 40, 8, 24, PANTS.0, PANTS.1, PANTS.2);
    rect(x + 8, y + 58, 8, 6, SHOE.0, SHOE.1, SHOE.2);
    rect(x + 16, y + 58, 8, 6, SHOE.0, SHOE.1, SHOE.2);

    // Boots, leggings, chestplate, helmet: each in its tier's colours.
    if let Some((t, _)) = armor_piece(p.worn[3]) {
        let m = metal(t);
        rect(x + 8, y + 56, 8, 8, m[1].0, m[1].1, m[1].2);
        rect(x + 16, y + 56, 8, 8, m[1].0, m[1].1, m[1].2);
        rect(x + 8, y + 56, 8, 2, m[0].0, m[0].1, m[0].2);
        rect(x + 16, y + 56, 8, 2, m[0].0, m[0].1, m[0].2);
        rect(x + 8, y + 62, 8, 2, m[2].0, m[2].1, m[2].2);
        rect(x + 16, y + 62, 8, 2, m[2].0, m[2].1, m[2].2);
    }
    if let Some((t, _)) = armor_piece(p.worn[2]) {
        let m = metal(t);
        rect(x + 8, y + 38, 16, 18, m[1].0, m[1].1, m[1].2);
        rect(x + 8, y + 38, 16, 2, m[0].0, m[0].1, m[0].2);
        rect(x + 15, y + 40, 2, 16, m[2].0, m[2].1, m[2].2);
        rect(x + 8, y + 54, 16, 2, m[2].0, m[2].1, m[2].2);
    }
    if let Some((t, _)) = armor_piece(p.worn[1]) {
        let m = metal(t);
        rect(x + 8, y + 16, 16, 22, m[1].0, m[1].1, m[1].2);
        rect(x, y + 16, 8, 10, m[1].0, m[1].1, m[1].2);
        rect(x + 24, y + 16, 8, 10, m[1].0, m[1].1, m[1].2);
        rect(x + 8, y + 16, 16, 2, m[0].0, m[0].1, m[0].2);
        rect(x, y + 16, 8, 2, m[0].0, m[0].1, m[0].2);
        rect(x + 24, y + 16, 8, 2, m[0].0, m[0].1, m[0].2);
        rect(x + 15, y + 18, 2, 18, m[2].0, m[2].1, m[2].2);
        rect(x + 8, y + 36, 16, 2, m[2].0, m[2].1, m[2].2);
    }
    if let Some((t, _)) = armor_piece(p.worn[0]) {
        let m = metal(t);
        // The dome and the cheek guards; the eyes stay in the opening.
        rect(x + 7, y - 1, 18, 8, m[1].0, m[1].1, m[1].2);
        rect(x + 7, y - 1, 18, 2, m[0].0, m[0].1, m[0].2);
        rect(x + 7, y + 7, 4, 9, m[1].0, m[1].1, m[1].2);
        rect(x + 21, y + 7, 4, 9, m[1].0, m[1].1, m[1].2);
        rect(x + 7, y + 14, 18, 2, m[2].0, m[2].1, m[2].2);
        rect(x + 7, y + 6, 18, 1, m[2].0, m[2].1, m[2].2);
    }
}

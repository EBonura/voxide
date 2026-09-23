//! Crafting, Legacy Console style: the current tab's recipes as a strip of
//! icons on the inventory grid's columns. LEFT/RIGHT choose the recipe; tools
//! and armour come in tiers, grouped into one column by their CRAFT_* output,
//! and UP/DOWN walk the tiers (the column rolls, the chosen tier on the
//! strip). A needs box shows the ingredients as icons, red when short.
//! CROSS crafts (hold to keep crafting), TRIANGLE hides what you cannot make.

use crate::inv::{frame, number, slot, tabs};
use crate::*;

const X0: i16 = HOTBAR_X0;
const STRIP_Y: i16 = 72;
const SLOT: i16 = 18;
const COLS: usize = 9;
const CURSOR: (u8, u8, u8) = (0xFF, 0xE0, 0x40);
const LABEL: (u8, u8, u8) = (0xE0, 0xE0, 0xE0);
const GREY: (u8, u8, u8) = (0xA8, 0xA8, 0xA8);
const GREEN: (u8, u8, u8) = (0x50, 0xB0, 0x50);
const RED: (u8, u8, u8) = (0xE0, 0x60, 0x60);

const TAB_NAME: [&str; CRAFT_TABS] = ["BLOCKS", "GEAR", "ITEMS", "FOOD"];
const TAB_TILES: [u8; CRAFT_TABS] = [tex::T_PLANK, tex::T_PICK, tex::T_I_STICK, tex::T_I_BREAD];

/// Column and tier under the cursor.
static mut COL: usize = 0;
static mut TIER: usize = 0;
/// Frames CROSS has been held (hold to keep crafting).
static mut HOLD_T: u16 = 0;
static mut NAV_T: [u16; 4] = [0; 4];

#[optimize(size)]
fn tiered(out: u8) -> bool {
    is_tool_recipe(out) || out == CRAFT_ARMOR
}

#[optimize(size)]
fn reachable(i: usize) -> bool {
    (unsafe { AT_BENCH }) || !needs_bench(i)
}

/// The player's tier for a tiered output.
#[optimize(size)]
fn have_tier(p: &Player, out: u8) -> u8 {
    match out {
        CRAFT_PICK => p.pick,
        CRAFT_AXE => p.axe,
        CRAFT_SHOVEL => p.shovel,
        CRAFT_SWORD => p.sword,
        _ => p.armor,
    }
}

/// Makeable now: affordable, reachable here, and not a tier already owned
/// (the old list happily spent the ingredients on a tool you already had).
#[optimize(size)]
fn makeable(i: usize, p: &Player) -> bool {
    let r = &RECIPES[i];
    craftable_here(i) && !(tiered(r.out) && r.out_qty as u8 <= have_tier(p, r.out))
}

/// The recipes of one column: every tier of a tiered output (in RECIPES
/// order, which is tier order), or the single recipe.
#[optimize(size)]
fn tiers_of(head: usize, out: &mut [u8; 4]) -> usize {
    let o = RECIPES[head].out;
    if !tiered(o) {
        out[0] = head as u8;
        return 1;
    }
    let mut n = 0;
    let mut i = head;
    while i < RECIPES.len() && n < 4 {
        if RECIPES[i].out == o && reachable(i) {
            out[n] = i as u8;
            n += 1;
        }
        i += 1;
    }
    n
}

/// The current tab's columns, as the first recipe of each. The can-make
/// filter keeps a column when any of its tiers can be made.
#[optimize(size)]
fn columns(p: &Player, out: &mut [u8; RECIPES.len()]) -> usize {
    let (tab, hide) = unsafe { (CRAFT_TAB, CRAFT_HIDE) };
    let mut n = 0;
    let mut i = 0;
    while i < RECIPES.len() {
        let o = RECIPES[i].out;
        // A tiered output's column starts at its first reachable recipe.
        let mut head = true;
        if tiered(o) {
            let mut k = 0;
            while k < i {
                if RECIPES[k].out == o && reachable(k) {
                    head = false;
                }
                k += 1;
            }
        }
        if head && recipe_tab(i) == tab && reachable(i) {
            let mut t = [0u8; 4];
            let nt = tiers_of(i, &mut t);
            let mut any = false;
            let mut k = 0;
            while k < nt {
                any |= makeable(t[k] as usize, p);
                k += 1;
            }
            if !hide || any {
                out[n] = i as u8;
                n += 1;
            }
        }
        i += 1;
    }
    n
}

/// The tier a column opens on: the first one above what you have.
#[optimize(size)]
fn default_tier(t: &[u8; 4], nt: usize, p: &Player) -> usize {
    let o = RECIPES[t[0] as usize].out;
    if !tiered(o) {
        return 0;
    }
    let have = have_tier(p, o);
    let mut k = 0;
    while k < nt {
        if RECIPES[t[k] as usize].out_qty as u8 > have {
            return k;
        }
        k += 1;
    }
    nt - 1
}

#[optimize(size)]
fn set_col(c: usize, cols: &[u8; RECIPES.len()], p: &Player) {
    let mut t = [0u8; 4];
    let nt = tiers_of(cols[c] as usize, &mut t);
    unsafe {
        COL = c;
        TIER = default_tier(&t, nt, p);
    }
}

/// Step the tab, skipping tabs with nothing in them (the pocket grid reaches
/// only a few recipes, and paging through blank tabs looks broken).
#[optimize(size)]
fn step_tab(dir: usize, p: &Player) {
    let mut cols = [0u8; RECIPES.len()];
    let mut k = 0;
    while k < CRAFT_TABS {
        unsafe { CRAFT_TAB = (CRAFT_TAB + dir) % CRAFT_TABS };
        if columns(p, &mut cols) > 0 {
            break;
        }
        k += 1;
    }
    if columns(p, &mut cols) > 0 {
        set_col(0, &cols, p);
    }
}

#[optimize(size)]
pub fn craft_open(p: &Player) {
    unsafe {
        HOLD_T = 0;
        NAV_T = [0; 4];
        COL = 0;
        TIER = 0;
    }
    let mut cols = [0u8; RECIPES.len()];
    if columns(p, &mut cols) == 0 {
        step_tab(1, p);
    } else {
        set_col(0, &cols, p);
    }
}

/// The recipe under the cursor, or None on an empty tab.
#[optimize(size)]
fn current(p: &Player) -> Option<(usize, [u8; 4], usize)> {
    let mut cols = [0u8; RECIPES.len()];
    let n = columns(p, &mut cols);
    if n == 0 {
        return None;
    }
    unsafe {
        if COL >= n {
            COL = n - 1;
        }
    }
    let mut t = [0u8; 4];
    let nt = tiers_of(cols[unsafe { COL }] as usize, &mut t);
    unsafe {
        if TIER >= nt {
            TIER = nt - 1;
        }
    }
    Some((t[unsafe { TIER }] as usize, t, nt))
}

#[inline(never)]
#[optimize(size)]
pub fn craft_input(pad: ButtonState, previous: ButtonState, p: &mut Player) {
    let pressed = |b: u16| pad.pressed_since(previous, b);
    if pressed(button::L1) {
        step_tab(CRAFT_TABS - 1, p);
        sfx::blip();
    }
    if pressed(button::R1) {
        step_tab(1, p);
        sfx::blip();
    }
    if pressed(button::TRIANGLE) {
        unsafe { CRAFT_HIDE = !CRAFT_HIDE };
        let mut cols = [0u8; RECIPES.len()];
        if columns(p, &mut cols) == 0 {
            step_tab(1, p);
        } else {
            set_col(0, &cols, p);
        }
        sfx::blip();
    }
    let mut cols = [0u8; RECIPES.len()];
    let n = columns(p, &mut cols);
    if n == 0 {
        return;
    }
    let t = unsafe { &mut NAV_T };
    if nav_repeat(pad.is_held(button::LEFT), &mut t[0]) {
        set_col((unsafe { COL } + n - 1) % n, &cols, p);
        sfx::blip();
    }
    if nav_repeat(pad.is_held(button::RIGHT), &mut t[1]) {
        set_col((unsafe { COL } + 1) % n, &cols, p);
        sfx::blip();
    }
    let Some((ri, _, nt)) = current(p) else {
        return;
    };
    if nav_repeat(pad.is_held(button::UP), &mut t[2]) && unsafe { TIER } > 0 {
        unsafe { TIER -= 1 };
        sfx::blip();
    }
    if nav_repeat(pad.is_held(button::DOWN), &mut t[3]) && unsafe { TIER } + 1 < nt {
        unsafe { TIER += 1 };
        sfx::blip();
    }
    // CROSS crafts once; held, it keeps crafting every few frames after a
    // short delay. Tiers craft on the press only: one is all you need.
    let h = unsafe { &mut HOLD_T };
    if !pad.is_held(button::CROSS) {
        *h = 0;
        return;
    }
    *h = h.saturating_add(1);
    let fire = *h == 1 || (!tiered(RECIPES[ri].out) && *h > 18 && (*h - 18) % 6 == 0);
    if !fire {
        return;
    }
    if makeable(ri, p) {
        craft(ri, p);
        sfx::confirm();
    } else if *h == 1 {
        sfx::blip();
    }
}

/// The picture of a recipe's output: its item icon, or the tool tile in the
/// tier's colour.
#[optimize(size)]
fn draw_output(x: i16, y: i16, i: usize, lum: u8) {
    let r = &RECIPES[i];
    if tiered(r.out) {
        let tile = match r.out {
            CRAFT_AXE => tex::T_AXE,
            CRAFT_SHOVEL => tex::T_SHOVEL,
            CRAFT_SWORD => tex::T_SWORD,
            CRAFT_PICK => tex::T_PICK,
            _ => tex::T_I_ARMOR,
        };
        let tint = if r.out == CRAFT_ARMOR {
            if r.out_qty >= 2 {
                (90, 220, 220)
            } else {
                (128, 128, 128)
            }
        } else {
            tool_tint(r.out_qty as u8)
        };
        let s = |c: u8| (c as u16 * lum as u16 / 128) as u8;
        draw_tile(x, y, tile, (s(tint.0), s(tint.1), s(tint.2)));
    } else {
        draw_icon(x, y, r.out, lum);
    }
}

const TIER_WORD: [&str; 5] = ["NONE", "WOOD", "STONE", "IRON", "DIAMOND"];
const ARMOR_WORD: [&str; 3] = ["NONE", "IRON", "DIAMOND"];

#[inline(never)]
#[optimize(size)]
pub fn draw_crafting(font: &FontAtlas, p: &Player) {
    let tab = unsafe { CRAFT_TAB };
    dim_screen();
    inv::panel(8, 3, 304, 207);
    draw_centered(
        font,
        8,
        if unsafe { AT_BENCH } {
            "CRAFTING TABLE"
        } else {
            "CRAFTING"
        },
        MC_INK,
    );
    tabs(font, 20, &TAB_TILES, tab);
    ui_text(font, X0, 43, TAB_NAME[tab], MC_INK);

    let mut cols = [0u8; RECIPES.len()];
    let n = columns(p, &mut cols);
    let cur = current(p);
    let col = unsafe { COL };
    // Nine columns on screen, scrolled to keep the cursor in view.
    let start = list_window(n, COLS, col);
    if start > 0 {
        ui_text(font, X0 - 12, STRIP_Y + 5, "<", MC_INK);
    }
    if start + COLS < n {
        ui_text(font, X0 + COLS as i16 * SLOT + 4, STRIP_Y + 5, ">", MC_INK);
    }
    let mut q = 0;
    while q < COLS {
        let x = X0 + q as i16 * SLOT;
        slot(x, STRIP_Y);
        let c = start + q;
        if c < n {
            let mut t = [0u8; 4];
            let nt = tiers_of(cols[c] as usize, &mut t);
            let k = if c == col {
                unsafe { TIER }
            } else {
                default_tier(&t, nt, p)
            };
            let i = t[k.min(nt - 1)] as usize;
            draw_output(x + 1, STRIP_Y + 1, i, if makeable(i, p) { 128 } else { 56 });
        }
        q += 1;
    }
    let Some((ri, t, nt)) = cur else {
        ui_text(font, X0, STRIP_Y + 24, "NOTHING YOU CAN MAKE", MC_INK);
        hints(font, false);
        draw_hotbar(hud_tool(p, AIR));
        return;
    };
    // The tier carousel on the chosen column: lower tier above, higher below.
    let sx = X0 + (col - start) as i16 * SLOT;
    let tier = unsafe { TIER };
    let mut k = 0;
    while k < nt {
        let dy = k as i16 - tier as i16;
        if k != tier && (-1..=2).contains(&dy) {
            let y = STRIP_Y + dy * SLOT;
            slot(sx, y);
            let i = t[k] as usize;
            draw_output(sx + 1, y + 1, i, if makeable(i, p) { 128 } else { 56 });
        }
        k += 1;
    }
    if tier >= 2 {
        ui_text(font, sx + 20, STRIP_Y - SLOT + 5, "^", MC_INK);
    }
    if tier + 3 < nt {
        ui_text(font, sx + 20, STRIP_Y + 2 * SLOT + 5, "v", MC_INK);
    }
    frame(sx, STRIP_Y, CURSOR);

    // Needs: each ingredient as its icon, have/need, red when short.
    let r = &RECIPES[ri];
    mc_slot(16, 128, 140, 40);
    ui_text(font, 20, 131, "NEEDS", GREY);
    let mut k = 0;
    while k < r.n_in as usize {
        let x = 20 + k as i16 * 66;
        let item = r.in_item[k];
        let have = unsafe { INV[item as usize] };
        draw_icon(x, 142, item, 128);
        let mut hb = [0u8; 5];
        let mut nb = [0u8; 5];
        let hs = number(have.min(999), &mut hb);
        let ns = number(r.in_qty[k], &mut nb);
        let c = if have >= r.in_qty[k] { GREEN } else { RED };
        let tx = x + 19;
        ui_text(font, tx, 146, hs, c);
        ui_text(font, tx + hs.len() as i16 * 8, 146, "/", c);
        ui_text(font, tx + (hs.len() as i16 + 1) * 8, 146, ns, c);
        k += 1;
    }

    // What it makes, and what you have now.
    mc_slot(162, 128, 142, 40);
    draw_output(166, 132, ri, 128);
    ui_text(font, 186, 132, r.label, LABEL);
    if tiered(r.out) {
        let armor = r.out == CRAFT_ARMOR;
        let mut tb = [0u8; 5];
        ui_text(font, 186, 144, "TIER", GREY);
        ui_text(font, 226, 144, number(r.out_qty, &mut tb), GREY);
        let have = have_tier(p, r.out) as usize;
        let word = if armor {
            ARMOR_WORD[have.min(2)]
        } else {
            TIER_WORD[have.min(4)]
        };
        let what = match r.out {
            CRAFT_AXE => "AXE",
            CRAFT_SHOVEL => "SHOVEL",
            CRAFT_SWORD => "SWORD",
            CRAFT_PICK => "PICK",
            _ => "ARMOR",
        };
        ui_text(font, 166, 156, "HAVE:", GREY);
        ui_text(font, 214, 156, word, GREY);
        if have > 0 {
            ui_text(font, 222 + word.len() as i16 * 8, 156, what, GREY);
        }
    } else {
        let mut mb = [0u8; 5];
        let mut hb = [0u8; 5];
        ui_text(font, 186, 144, "MAKES", GREY);
        ui_text(font, 234, 144, number(r.out_qty, &mut mb), GREY);
        ui_text(font, 166, 156, "YOU HAVE", GREY);
        ui_text(font, 238, 156, number(unsafe { INV[r.out as usize] }, &mut hb), GREY);
    }
    hints(font, nt > 1);
    draw_hotbar(hud_tool(p, AIR));
}

#[optimize(size)]
fn hints(font: &FontAtlas, tiers: bool) {
    let (y1, y2) = (176, 190);
    let x = hint_item(font, 16, y1, "X", PS_CROSS, "CRAFT");
    let x = hint_item(
        font,
        x,
        y1,
        "T",
        PS_TRIANGLE,
        if unsafe { CRAFT_HIDE } { "SHOW ALL" } else { "CAN MAKE" },
    );
    hint_item(font, x, y1, "O", PS_CIRCLE, "CLOSE");
    let x = hint_item(font, 16, y2, "L1R1", PS_KEY, "TAB");
    let x = hint_item(font, x, y2, "<>", PS_KEY, "ITEM");
    if tiers {
        hint_item(font, x, y2, "^v", PS_KEY, "TIER");
    }
}

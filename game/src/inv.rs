//! Container menus: what the chest and furnace panels list, and how many items
//! a press moves. Kept out of main.rs so the menus can grow without reshaping
//! the gameplay loop (whose PGO profile is keyed to its line layout).
//!
//! Button grammar, the Legacy Console one: CROSS moves one (hold to repeat,
//! accelerating), SQUARE moves half (rounded up), TRIANGLE moves all.

use crate::*;

/// Chest transfer direction, flipped with LEFT/RIGHT: true = into the chest.
static mut CHEST_PUT: bool = true;
/// Frames CROSS has been held in a container menu.
static mut HOLD_T: u16 = 0;

/// How many items a held CROSS moves this frame: one on the press, then after
/// a short delay a repeat that speeds up the longer it is held, ending at a
/// handful per frame so a stack of hundreds still empties in a few seconds.
fn hold_step(held: bool) -> u16 {
    let t = unsafe { &mut HOLD_T };
    if !held {
        *t = 0;
        return 0;
    }
    *t = t.saturating_add(1);
    match *t {
        1 => 1,
        2..=14 => 0,
        15..=44 => (*t % 3 == 0) as u16,
        45..=89 => 1,
        _ => 4,
    }
}

/// Half a stack, rounded up, so one item still moves.
fn half(n: u16) -> u16 {
    n - n / 2
}

/// How many to move for this frame's buttons, given `have` at the source.
fn press_count(pad: ButtonState, previous: ButtonState, have: u16) -> u16 {
    if pad.pressed_since(previous, button::TRIANGLE) {
        have
    } else if pad.pressed_since(previous, button::SQUARE) {
        half(have)
    } else {
        hold_step(pad.is_held(button::CROSS)).min(have)
    }
}

/// Every kind the player or this chest holds, in id order: blocks and
/// materials alike (the chest used to list placeables only, so coal, ingots,
/// sticks and the rest could not be stored).
#[inline(never)]
#[optimize(size)]
pub fn chest_list(idx: usize, out: &mut [u8; BLOCK_KINDS]) -> usize {
    let mut n = 0;
    let mut k = 1;
    while k < BLOCK_KINDS {
        if unsafe { INV[k] > 0 || CHEST_INV[idx][k] > 0 } {
            out[n] = k as u8;
            n += 1;
        }
        k += 1;
    }
    n
}

pub fn chest_open() {
    unsafe {
        CHEST_PUT = true;
        HOLD_T = 0;
    }
}

#[inline(never)]
#[optimize(size)]
pub fn chest_input(idx: usize, sel: usize, pad: ButtonState, previous: ButtonState) {
    if pad.pressed_since(previous, button::LEFT) || pad.pressed_since(previous, button::RIGHT) {
        unsafe { CHEST_PUT = pad.is_held(button::RIGHT) };
        sfx::blip();
    }
    let mut list = [0u8; BLOCK_KINDS];
    let n = chest_list(idx, &mut list);
    if sel >= n {
        hold_step(false);
        return;
    }
    let item = list[sel] as usize;
    let put = unsafe { CHEST_PUT };
    let have = unsafe {
        if put {
            INV[item]
        } else {
            CHEST_INV[idx][item]
        }
    };
    let m = press_count(pad, previous, have);
    if m == 0 {
        return;
    }
    unsafe {
        if put {
            INV[item] -= m;
            CHEST_INV[idx][item] = CHEST_INV[idx][item].saturating_add(m);
        } else {
            CHEST_INV[idx][item] -= m;
            inv_give(item as u8, m);
        }
    }
    sfx::blip();
}

/// Furnace rows: row 0 takes from the output slot, the rest load the input or
/// the fuel from FURN_ITEMS.
pub const FURN_ROWS: usize = FURN_ITEMS.len() + 1;

#[inline(never)]
#[optimize(size)]
pub fn furnace_input(idx: usize, sel: usize, pad: ButtonState, previous: ButtonState) {
    if sel == 0 {
        let have = unsafe { FURN_OUT_N[idx] };
        let m = press_count(pad, previous, have);
        if m > 0 {
            unsafe {
                inv_give(FURN_OUT[idx], m);
                FURN_OUT_N[idx] -= m;
                if FURN_OUT_N[idx] == 0 {
                    FURN_OUT[idx] = AIR;
                }
            }
            sfx::blip();
        }
        return;
    }
    let item = FURN_ITEMS[sel - 1];
    let m = press_count(pad, previous, unsafe { INV[item as usize] });
    let mut moved = false;
    let mut i = 0;
    while i < m {
        let before = unsafe { INV[item as usize] };
        furn_deposit(idx, item);
        if unsafe { INV[item as usize] } == before {
            break; // the input slot holds a different ore
        }
        moved = true;
        i += 1;
    }
    if moved {
        sfx::blip();
    }
}

/// Chest overlay: every kind either side holds, with both counts and the
/// direction a press moves them.
#[inline(never)]
#[optimize(size)]
pub fn draw_chest(font: &FontAtlas, idx: usize, sel: usize) {
    menu_frame(font, "CHEST");
    let mut hx = hint_item(font, MENU_TEXT_X, MENU_HINT_Y, "X", PS_CROSS, "MOVE");
    hx = hint_item(font, hx, MENU_HINT_Y, "[]", PS_SQUARE, "HALF");
    hx = hint_item(font, hx, MENU_HINT_Y, "T", PS_TRIANGLE, "ALL");
    hint_item(font, hx, MENU_HINT_Y, "O", PS_CIRCLE, "CLOSE");
    let put = unsafe { CHEST_PUT };
    let hx = hint_item(
        font,
        MENU_TEXT_X,
        170,
        "< >",
        PS_KEY,
        if put { "PUT IN" } else { "TAKE OUT" },
    );
    hint_item(font, hx, 170, "HOLD X", PS_CROSS, "FASTER");
    let mut list = [0u8; BLOCK_KINDS];
    let n = chest_list(idx, &mut list);
    if n == 0 {
        ui_text(font, MENU_TEXT_X, MENU_ROWS_Y, "NOTHING TO STORE", MC_INK);
        return;
    }
    let vis = 7;
    let start = list_window(n, vis, sel);
    menu_scroll_hint(font, n, vis, start, MENU_HINT_X, MENU_ROWS_Y);
    let mut j = 0;
    while j < vis && start + j < n {
        let i = start + j;
        let b = list[i];
        let y = menu_row(j, i == sel);
        let color = if i == sel { MC_LABEL_SEL } else { MC_LABEL };
        ui_text(font, MENU_TEXT_X, y, block_name(b), color);
        ui_text(font, 158, y, "U", (0x90, 0xC0, 0x90));
        ui_text(font, 170, y, &decimal3(unsafe { INV[b as usize] }), color);
        ui_text(font, 202, y, if put { ">" } else { "<" }, color);
        ui_text(font, 222, y, "C", (0x90, 0xC0, 0x90));
        ui_text(font, 234, y, &decimal3(unsafe { CHEST_INV[idx][b as usize] }), color);
        j += 1;
    }
}

/// Furnace overlay: input / fuel / output read-outs, the smelt progress bar,
/// then the rows: TAKE OUTPUT first, then what can be loaded.
#[inline(never)]
#[optimize(size)]
pub fn draw_furnace(font: &FontAtlas, idx: usize, sel: usize) {
    menu_frame(font, "FURNACE");
    let mut hx = hint_item(font, MENU_TEXT_X, MENU_HINT_Y, "X", PS_CROSS, "MOVE");
    hx = hint_item(font, hx, MENU_HINT_Y, "[]", PS_SQUARE, "HALF");
    hx = hint_item(font, hx, MENU_HINT_Y, "T", PS_TRIANGLE, "ALL");
    hint_item(font, hx, MENU_HINT_Y, "O", PS_CIRCLE, "CLOSE");
    let (inn, in_n, fuel, outt, out_n, prog) = unsafe {
        (
            FURN_IN[idx],
            FURN_IN_N[idx],
            FURN_FUEL[idx],
            FURN_OUT[idx],
            FURN_OUT_N[idx],
            FURN_PROG[idx],
        )
    };
    let in_name = if inn == AIR { "--" } else { block_name(inn) };
    let out_name = if outt == AIR { "--" } else { block_name(outt) };
    // The slots are read-outs, not choices: vanilla's inset slot bevel.
    mc_slot(MENU_BTN_X, 41, MENU_BTN_W, 36);
    ui_text(font, MENU_TEXT_X, 44, "IN", MC_HINT);
    ui_text(font, MENU_TEXT_X + 40, 44, in_name, MC_LABEL);
    ui_text(font, 150, 44, &decimal3(in_n), MC_LABEL);
    ui_text(font, 190, 44, "FUEL", MC_HINT);
    ui_text(font, 238, 44, &decimal3(fuel), (0xF0, 0xC0, 0x50));
    ui_text(font, MENU_TEXT_X, 60, "OUT", MC_HINT);
    ui_text(font, MENU_TEXT_X + 40, 60, out_name, MC_LABEL);
    ui_text(font, 150, 60, &decimal3(out_n), MC_LABEL);
    // Smelt progress, the arrow in vanilla's furnace.
    mc_slot(MENU_BTN_X, 80, MENU_BTN_W, 9);
    let w = prog as i16 * (MENU_BTN_W - 4) / SMELT_TIME as i16;
    if w > 0 {
        rect(MENU_BTN_X + 2, 82, w, 5, 230, 140, 40);
    }

    let vis = 4;
    let top = 96;
    let start = list_window(FURN_ROWS, vis, sel);
    menu_scroll_hint(font, FURN_ROWS, vis, start, MENU_HINT_X, top);
    let mut j = 0;
    while j < vis && start + j < FURN_ROWS {
        let i = start + j;
        let y = top + (j * MENU_ROW_H) as i16;
        mc_button(y - 3, i == sel);
        let color = if i == sel { MC_LABEL_SEL } else { MC_LABEL };
        if i == 0 {
            ui_text(font, MENU_TEXT_X, y, "TAKE OUTPUT", color);
            ui_text(font, 212, y, &decimal3(out_n), color);
        } else {
            let b = FURN_ITEMS[i - 1];
            ui_text(font, MENU_TEXT_X, y, block_name(b), color);
            ui_text(font, 200, y, "U", (0x90, 0xC0, 0x90));
            ui_text(font, 212, y, &decimal3(unsafe { INV[b as usize] }), color);
        }
        j += 1;
    }
    hint_item(font, MENU_TEXT_X, 170, "HOLD X", PS_CROSS, "FASTER");
}

// ---------------------------------------------------------------------------
// The inventory: a Legacy Console style icon grid over the count-per-kind
// inventory. Nothing here is a Java slot: a kind shows once with its count,
// and the only layout the player owns is the hotbar, which sits live under the
// grid as the grid's fifth row. L1/R1 pick a tab, the D-pad snaps a cursor
// (hold to repeat), CROSS picks a kind up and puts it on a hotbar slot,
// TRIANGLE sends it to the first free slot, SQUARE clears a slot, SELECT
// shows every kind of the tab instead of only what you own.

const GRID_X: i16 = HOTBAR_X0; // the grid sits on the hotbar's columns
const GRID_Y: i16 = 53;
const SLOT: i16 = 18;
const COLS: usize = 9;
const ROWS: usize = 4;
const PER_PAGE: usize = COLS * ROWS;
/// The cursor's fifth row is the hotbar itself.
const HOT_ROW: usize = ROWS;
const CURSOR: (u8, u8, u8) = (0xFF, 0xE0, 0x40);

const TABS: usize = 4;
const TAB_NAME: [&str; TABS] = ["BLOCKS", "ITEMS", "FOOD", "MATERIALS"];
const TAB_BLOCKS: [u8; 33] = [
    GRASS, DIRT, STONE, COBBLE, SLAB, STAIRS_N, BRICK, WOOD, PLANK, FENCE, LEAVES, SAND, SNOW,
    GLASS, WOOL, OBSIDIAN, CINDERSTONE, SINK_SAND, LUMISTONE, VOID_STONE, CACTUS, SAPLING,
    LADDER, DOOR_C, CRAFT_TABLE, CHEST, FURNACE, ENCHANT, BED, TORCH, WIRE, PISTON, TNT,
];
const TAB_ITEMS: [u8; 13] = [
    BOW, FISHING_ROD, BUCKET, WATER_BUCKET, LAVA_BUCKET, FLINT_STEEL, VOID_EYE, BONEMEAL, BOTTLE,
    POTION_SPEED, POTION_STRENGTH, POTION_REGEN, POTION_FIRE,
];
const TAB_FOOD: [u8; 5] = [BREAD, COOKED_MEAT, RAW_MEAT, WHEAT_ITEM, SEEDS];
const TAB_MATERIALS: [u8; 18] = [
    COAL_ORE, IRON_ORE, IRON_INGOT, GOLD_ORE, DIAMOND_ORE, STICK, STRING, BONE, GUNPOWDER, ARROW,
    CLAY, SUGAR_CANE, EMBER_CAP, EMBER_ROD, MAGMA_PASTE, WAILER_TEAR, VOID_PEARL, POTION_AWKWARD,
];

fn tab_table(tab: usize) -> &'static [u8] {
    match tab {
        0 => &TAB_BLOCKS,
        1 => &TAB_ITEMS,
        2 => &TAB_FOOD,
        _ => &TAB_MATERIALS,
    }
}

fn listed(item: u8) -> bool {
    let mut t = 0;
    while t < TABS {
        if tab_table(t).contains(&item) {
            return true;
        }
        t += 1;
    }
    false
}

/// A tab's kinds: owned ones, or the whole tab with SELECT's ALL view. The
/// materials tab also collects anything owned that no tab lists, so no owned
/// kind is ever invisible.
#[inline(never)]
#[optimize(size)]
fn tab_list(tab: usize, all: bool, out: &mut [u8; BLOCK_KINDS]) -> usize {
    let table = tab_table(tab);
    let mut n = 0;
    let mut i = 0;
    while i < table.len() {
        if all || unsafe { INV[table[i] as usize] } > 0 {
            out[n] = table[i];
            n += 1;
        }
        i += 1;
    }
    if tab == TABS - 1 {
        let mut k = 1;
        while k < BLOCK_KINDS {
            if unsafe { INV[k] } > 0 && !listed(k as u8) {
                out[n] = k as u8;
                n += 1;
            }
            k += 1;
        }
    }
    n
}

static mut TAB: usize = 0;
static mut PAGE: usize = 0;
static mut CUR_X: usize = 0;
static mut CUR_Y: usize = 0;
static mut SHOW_ALL: bool = false;
/// The kind on the cursor (AIR = none) and the hotbar slot it was lifted
/// from (-1 = from the grid).
static mut CARRY: u8 = AIR;
static mut CARRY_FROM: i8 = -1;
/// A one-line note in the info strip after an action, for NOTE_T frames.
static mut NOTE: &str = "";
static mut NOTE_T: u8 = 0;
/// Hold-to-repeat timers: up, down, left, right.
static mut NAV_T: [u16; 4] = [0; 4];
/// Set on open: the TRIANGLE that opened the menu must not also Quick Move.
static mut FRESH: bool = false;

fn note(s: &'static str) {
    unsafe {
        NOTE = s;
        NOTE_T = 75;
    }
}

fn pages(n: usize) -> usize {
    if n == 0 {
        1
    } else {
        (n + PER_PAGE - 1) / PER_PAGE
    }
}

fn hotbar_slot_of(item: u8) -> Option<usize> {
    let mut i = 0;
    while i < HOTBAR_VIS {
        if unsafe { HOTBAR[i] } == item {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Open on the tab and slot of the item in hand, so the first thing the
/// cursor shows is what you are holding.
#[inline(never)]
#[optimize(size)]
pub fn inventory_open(selected: u8) {
    unsafe {
        SHOW_ALL = false;
        CARRY = AIR;
        NOTE_T = 0;
        TAB = 0;
        PAGE = 0;
        CUR_X = 0;
        CUR_Y = 0;
        NAV_T = [0; 4];
        FRESH = true;
    }
    if selected == AIR {
        return;
    }
    let mut list = [0u8; BLOCK_KINDS];
    let mut t = 0;
    while t < TABS {
        let n = tab_list(t, false, &mut list);
        let mut i = 0;
        while i < n {
            if list[i] == selected {
                unsafe {
                    TAB = t;
                    PAGE = i / PER_PAGE;
                    CUR_X = i % COLS;
                    CUR_Y = (i % PER_PAGE) / COLS;
                }
                return;
            }
            i += 1;
        }
        t += 1;
    }
}

/// The grid kind under the cursor, if any.
fn cursor_item(list: &[u8; BLOCK_KINDS], n: usize) -> u8 {
    let (y, x, page) = unsafe { (CUR_Y, CUR_X, PAGE) };
    if y >= ROWS {
        return AIR;
    }
    let i = page * PER_PAGE + y * COLS + x;
    if i < n {
        list[i]
    } else {
        AIR
    }
}

/// Put the carried kind on hotbar slot `j`. A kind lives in one slot, so it
/// leaves its old one. An occupant swaps into that old slot when the carry
/// came off the hotbar, and otherwise is picked up in turn.
#[inline(never)]
#[optimize(size)]
fn drop_on_hotbar(j: usize) {
    unsafe {
        let item = CARRY;
        let occupant = HOTBAR[j];
        if let Some(k) = hotbar_slot_of(item) {
            HOTBAR[k] = AIR;
        }
        HOTBAR[j] = item;
        CARRY = AIR;
        if occupant != AIR && occupant != item {
            if CARRY_FROM >= 0 {
                HOTBAR[CARRY_FROM as usize] = occupant;
            } else {
                CARRY = occupant;
                CARRY_FROM = -1;
            }
        }
    }
}

/// One frame of inventory input. Returns true when the menu should close.
#[inline(never)]
#[optimize(size)]
pub fn inventory_input(pad: ButtonState, previous: ButtonState) -> bool {
    let pressed = |b: u16| pad.pressed_since(previous, b);
    unsafe {
        if NOTE_T > 0 {
            NOTE_T -= 1;
        }
        if FRESH {
            FRESH = false;
            return false;
        }
    }
    if pressed(button::CIRCLE) {
        if unsafe { CARRY } != AIR {
            unsafe { CARRY = AIR };
            sfx::blip();
            return false;
        }
        return true;
    }
    let mut list = [0u8; BLOCK_KINDS];
    let all = unsafe { SHOW_ALL };
    let mut n = tab_list(unsafe { TAB }, all, &mut list);
    if pressed(button::L1) || pressed(button::R1) {
        unsafe {
            TAB = if pressed(button::R1) {
                (TAB + 1) % TABS
            } else {
                (TAB + TABS - 1) % TABS
            };
            PAGE = 0;
            CUR_X = 0;
            CUR_Y = if CUR_Y == HOT_ROW { HOT_ROW } else { 0 };
            NOTE_T = 0;
        }
        n = tab_list(unsafe { TAB }, all, &mut list);
        sfx::blip();
    }
    if pressed(button::SELECT) {
        unsafe {
            SHOW_ALL = !SHOW_ALL;
            PAGE = 0;
        }
        n = tab_list(unsafe { TAB }, !all, &mut list);
        sfx::blip();
    }
    let np = pages(n);
    if np > 1 && (pressed(button::L2) || pressed(button::R2)) {
        unsafe {
            PAGE = if pressed(button::R2) {
                (PAGE + 1) % np
            } else {
                (PAGE + np - 1) % np
            };
        }
        sfx::blip();
    }
    unsafe {
        if PAGE >= np {
            PAGE = np - 1;
        }
    }
    // Snap cursor, 9 columns by 4 grid rows plus the hotbar, wrapping.
    let t = unsafe { &mut NAV_T };
    let dirs = [button::UP, button::DOWN, button::LEFT, button::RIGHT];
    let mut d = 0;
    while d < 4 {
        if nav_repeat(pad.is_held(dirs[d]), &mut t[d]) {
            unsafe {
                match d {
                    0 => CUR_Y = (CUR_Y + HOT_ROW) % (HOT_ROW + 1),
                    1 => CUR_Y = (CUR_Y + 1) % (HOT_ROW + 1),
                    2 => CUR_X = (CUR_X + COLS - 1) % COLS,
                    _ => CUR_X = (CUR_X + 1) % COLS,
                }
            }
            unsafe { NOTE_T = 0 };
            sfx::blip();
        }
        d += 1;
    }

    let (cx, cy) = unsafe { (CUR_X, CUR_Y) };
    let item = cursor_item(&list, n);
    if pressed(button::CROSS) {
        unsafe {
            NOTE_T = 0; // a new action replaces the last note
            if cy == HOT_ROW {
                if CARRY != AIR {
                    drop_on_hotbar(cx);
                } else if HOTBAR[cx] != AIR {
                    CARRY = HOTBAR[cx];
                    CARRY_FROM = cx as i8;
                }
            } else if CARRY != AIR {
                // Back into the inventory: off the hotbar if it came from it.
                if CARRY_FROM >= 0 && HOTBAR[CARRY_FROM as usize] == CARRY {
                    HOTBAR[CARRY_FROM as usize] = AIR;
                    note("TAKEN OFF THE HOTBAR");
                }
                CARRY = AIR;
            } else if item != AIR {
                if INV[item as usize] == 0 {
                    note("YOU HAVE NONE YET");
                } else if !in_placeable(item) {
                    note("CAN'T BE HELD: USE IT IN RECIPES");
                } else {
                    CARRY = item;
                    CARRY_FROM = -1;
                }
            }
        }
        sfx::blip();
    }
    if pressed(button::SQUARE) {
        unsafe {
            let slot = if cy == HOT_ROW {
                Some(cx)
            } else {
                hotbar_slot_of(item).filter(|_| item != AIR)
            };
            if let Some(j) = slot {
                if HOTBAR[j] != AIR {
                    HOTBAR[j] = AIR;
                    note("HOTBAR SLOT CLEARED");
                    sfx::blip();
                }
            }
        }
    }
    if pressed(button::TRIANGLE) && cy < HOT_ROW && item != AIR {
        if unsafe { INV[item as usize] } == 0 {
            note("YOU HAVE NONE YET");
        } else if !in_placeable(item) {
            note("CAN'T BE HELD: USE IT IN RECIPES");
        } else if hotbar_slot_of(item).is_some() {
            note("ALREADY ON THE HOTBAR");
        } else if let Some(j) = hotbar_slot_of(AIR) {
            unsafe { HOTBAR[j] = item };
            note("MOVED TO THE HOTBAR");
        } else {
            note("HOTBAR FULL: X PICKS A SLOT");
        }
        sfx::blip();
    }
    false
}

// -- drawing ------------------------------------------------------------------

/// Inventory-sized dialog: black outline, light face, vanilla bevel.
#[inline(never)]
#[optimize(size)]
fn panel(x: i16, y: i16, w: i16, h: i16) {
    rect(x, y, w, h, 0, 0, 0);
    rect(x + 1, y + 1, w - 2, h - 2, 0xC6, 0xC6, 0xC6);
    rect(x + 1, y + 1, w - 3, 1, 0xFF, 0xFF, 0xFF);
    rect(x + 1, y + 1, 1, h - 3, 0xFF, 0xFF, 0xFF);
    rect(x + 2, y + h - 2, w - 3, 1, 0x55, 0x55, 0x55);
    rect(x + w - 2, y + 2, 1, h - 3, 0x55, 0x55, 0x55);
}

/// One 17px item slot, the hotbar's bevel: dark top/left, light bottom/right.
#[inline(never)]
#[optimize(size)]
pub fn slot(x: i16, y: i16) {
    rect(x, y, 17, 17, 30, 30, 36);
    rect(x + 1, y + 1, 16, 16, 96, 96, 106);
    rect(x + 1, y + 1, 15, 15, 54, 54, 62);
}

/// A slot's 2px frame, on the same lines as the hotbar's white selection.
#[inline(never)]
#[optimize(size)]
pub fn frame(x: i16, y: i16, c: (u8, u8, u8)) {
    rect(x - 2, y - 2, 22, 2, c.0, c.1, c.2);
    rect(x - 2, y + 18, 22, 2, c.0, c.1, c.2);
    rect(x - 2, y, 2, 18, c.0, c.1, c.2);
    rect(x + 18, y, 2, 18, c.0, c.1, c.2);
}

/// L1 [tab][tab].. R1, the current tab raised and lit.
#[inline(never)]
#[optimize(size)]
pub fn tabs(font: &FontAtlas, y: i16, tiles: &[u8], sel: usize) {
    let n = tiles.len() as i16;
    let total = n * 24 - 2;
    let x0 = (SCREEN_W as i16 - total) / 2;
    ui_badge(font, x0 - 26, y + 7, "L1", PS_KEY);
    let mut i = 0;
    while i < tiles.len() {
        let x = x0 + i as i16 * 24;
        if i == sel {
            rect(x, y - 2, 22, 22, 0, 0, 0);
            rect(x + 1, y - 1, 20, 20, 0xE6, 0xE6, 0xE6);
            draw_tile(x + 3, y + 1, tiles[i], (128, 128, 128));
        } else {
            rect(x, y, 22, 20, 0, 0, 0);
            rect(x + 1, y + 1, 20, 18, 0x8B, 0x8B, 0x8B);
            draw_tile(x + 3, y + 2, tiles[i], (70, 70, 70));
        }
        i += 1;
    }
    ui_badge(font, x0 + total + 5, y + 7, "R1", PS_KEY);
}

/// Unpadded decimal into `buf`, returned as text.
pub fn number(v: u16, buf: &mut [u8; 5]) -> &str {
    let mut i = 5;
    let mut v = v;
    loop {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    unsafe { core::str::from_utf8_unchecked(&buf[i..]) }
}

/// What an item is for, one line for the info strip.
fn purpose(item: u8) -> &'static str {
    match item {
        BREAD | COOKED_MEAT | RAW_MEAT => "EATEN WHEN YOU GET HUNGRY",
        ARROW => "AMMO FOR THE BOW",
        SEEDS => "L2 PLANTS THEM ON SOIL",
        WHEAT_ITEM => "ANIMALS FOLLOW IT. MAKES BREAD",
        _ if fuel_smelts(item) > 0 && !in_placeable(item) => "BURNS AS FURNACE FUEL",
        _ if smelt_result(item) != AIR && !in_placeable(item) => "SMELTS IN A FURNACE",
        _ if !in_placeable(item) => "A CRAFTING MATERIAL",
        _ if unsafe { TAB } == 1 => "L2 USES IT.",
        _ => "L2 PLACES IT.",
    }
}

const TAB_TILES: [u8; TABS] = [tex::T_GRASS_SIDE, tex::T_I_BOW, tex::T_I_BREAD, tex::T_I_COAL];
const LABEL: (u8, u8, u8) = (0xE0, 0xE0, 0xE0);
const GREY: (u8, u8, u8) = (0xA8, 0xA8, 0xA8);

#[inline(never)]
#[optimize(size)]
pub fn draw_inventory(font: &FontAtlas, player: &Player) {
    let (tab, page, all, cx, cy, carry) =
        unsafe { (TAB, PAGE, SHOW_ALL, CUR_X, CUR_Y, CARRY) };
    let mut list = [0u8; BLOCK_KINDS];
    let n = tab_list(tab, all, &mut list);
    let np = pages(n);

    dim_screen();
    panel(8, 3, 304, 207);
    draw_centered(font, 8, "INVENTORY", MC_INK);
    tabs(font, 20, &TAB_TILES, tab);
    ui_text(font, GRID_X, 43, TAB_NAME[tab], MC_INK);
    let mut pb = [0u8; 5];
    let mut pc = [0u8; 5];
    let p1 = number(page as u16 + 1, &mut pb);
    let p2 = number(np as u16, &mut pc);
    let px = GRID_X + 162 - (p1.len() + p2.len() + 6) as i16 * 8;
    ui_text(font, px, 43, "PAGE", MC_INK);
    ui_text(font, px + 40, 43, p1, MC_INK);
    ui_text(font, px + 40 + p1.len() as i16 * 8, 43, "/", MC_INK);
    ui_text(font, px + 48 + p1.len() as i16 * 8, 43, p2, MC_INK);

    // Equipped gear, read-only: tools auto-equip, and this is where you see it.
    ui_text(font, 16, 43, "GEAR", MC_INK);
    let gear = [
        (tex::T_PICK, player.pick),
        (tex::T_AXE, player.axe),
        (tex::T_SHOVEL, player.shovel),
        (tex::T_SWORD, player.sword),
        (tex::T_I_ARMOR, player.armor),
    ];
    let mut g = 0;
    while g < gear.len() {
        let (x, y) = (16 + (g % 2) as i16 * 19, GRID_Y + (g / 2) as i16 * SLOT);
        slot(x, y);
        let (tile, tier) = gear[g];
        if tier > 0 {
            let tint = if tile == tex::T_I_ARMOR {
                if tier >= 2 {
                    (90, 220, 220)
                } else {
                    (128, 128, 128)
                }
            } else {
                tool_tint(tier)
            };
            draw_tile(x + 1, y + 1, tile, tint);
        }
        g += 1;
    }

    // The grid.
    let mut k = 0;
    while k < PER_PAGE {
        let x = GRID_X + (k % COLS) as i16 * SLOT;
        let y = GRID_Y + (k / COLS) as i16 * SLOT;
        slot(x, y);
        let i = page * PER_PAGE + k;
        if i < n {
            let b = list[i];
            let c = unsafe { INV[b as usize] };
            draw_icon(x + 1, y + 1, b, if c == 0 { 48 } else { 128 });
            if c > 1 {
                draw_count(x, y, c);
            }
        }
        k += 1;
    }

    // Info strip: what is under (or on) the cursor, and what X will do.
    mc_slot(16, 130, 288, 34);
    let hot = cy == HOT_ROW;
    let shown = if carry != AIR {
        carry
    } else if hot {
        unsafe { HOTBAR[cx] }
    } else {
        cursor_item(&list, n)
    };
    if shown != AIR {
        draw_icon(22, 135, shown, 128);
        let mut x = 42;
        if carry != AIR {
            ui_text(font, x, 134, "MOVING ", LABEL);
            x += 56;
        }
        let name = block_name(shown);
        ui_text(font, x, 134, name, LABEL);
        if carry == AIR && unsafe { INV[shown as usize] } > 0 {
            let mut nb = [0u8; 5];
            let cnt = number(unsafe { INV[shown as usize] }, &mut nb);
            let nx = x + (name.len() as i16 + 1) * 8;
            ui_text(font, nx, 134, "X", GREY);
            ui_text(font, nx + 8, 134, cnt, GREY);
        }
    }
    let (note_t, note_s) = unsafe { (NOTE_T, NOTE) };
    if note_t > 0 {
        ui_text(font, 42, 148, note_s, (0xF0, 0xE0, 0x80));
    } else if carry != AIR {
        ui_text(
            font,
            42,
            148,
            if hot {
                "X PUTS IT IN THIS SLOT"
            } else {
                "X PUTS IT BACK"
            },
            GREY,
        );
    } else if shown != AIR {
        let p = purpose(shown);
        ui_text(font, 42, 148, p, GREY);
        if in_placeable(shown) && unsafe { INV[shown as usize] } > 0 {
            let x = 42 + (p.len() as i16 + 1) * 8;
            match hotbar_slot_of(shown) {
                Some(j) => {
                    ui_text(font, x, 148, "SLOT ", GREY);
                    let d = [b'1' + j as u8];
                    ui_text(font, x + 40, 148, unsafe { core::str::from_utf8_unchecked(&d) }, GREY);
                }
                None => ui_text(font, x, 148, "NOT ON HOTBAR", GREY),
            }
        }
    } else if hot {
        ui_text(font, 42, 148, "AN EMPTY HOTBAR SLOT", GREY);
    }

    // Controls.
    let (y1, y2) = (174, 190);
    let x = hint_item(font, 16, y1, "X", PS_CROSS, if carry != AIR { "PUT" } else { "MOVE" });
    let x = hint_item(font, x, y1, "T", PS_TRIANGLE, "TO HOTBAR");
    hint_item(font, x, y1, "[]", PS_SQUARE, "CLEAR SLOT");
    let x = hint_item(font, 16, y2, "L1R1", PS_KEY, "TAB");
    let x = hint_item(font, x, y2, "SEL", PS_KEY, if all { "OWNED" } else { "ALL" });
    let x = hint_item(font, x, y2, "O", PS_CIRCLE, if carry != AIR { "CANCEL" } else { "CLOSE" });
    if np > 1 {
        hint_item(font, x, y2, "L2R2", PS_KEY, "PAGE");
    }

    // The live hotbar is the grid's fifth row, drawn over the dimming.
    draw_hotbar(hud_tool(player, AIR));
    let (sx, sy) = if hot {
        (HOTBAR_X0 + cx as i16 * SLOT, HUD_HOTBAR_Y)
    } else {
        (GRID_X + cx as i16 * SLOT, GRID_Y + cy as i16 * SLOT)
    };
    frame(sx, sy, CURSOR);
    if carry != AIR {
        // The lifted stack rides the cursor, up and to the right.
        draw_icon(sx + 6, sy - 10, carry, 128);
        let c = unsafe { INV[carry as usize] };
        if c > 1 {
            draw_count(sx + 6, sy - 11, c);
        }
    }
}

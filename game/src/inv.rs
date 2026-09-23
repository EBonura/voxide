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

//! The inventory, chest and furnace menus. Kept out of main.rs so the menus
//! can grow without reshaping the gameplay loop (whose PGO profile is keyed
//! to its line layout). All of it is built for size: the sim is paused while
//! a menu is up, and RAM is the budget that binds.
//!
//! Every screen here speaks one pad grammar (hand.rs): X takes a stack and
//! places it (swapping with what is there), SQUARE takes half and places one,
//! TRIANGLE quick-moves the stack under the cursor, CIRCLE puts the stack in
//! hand back, or closes. The prompt bar at the bottom prints what each button
//! does for the slot under the cursor, from the same table the input uses.

use crate::hand::{self, Act, Btn, Spot, Zone};
use crate::*;

// -- chest and furnace: two panes on the grid ---------------------------------
//
// YOU on the left, the container on the right, on the inventory's 18px slots.
// The cursor crosses between the panes; what it sits on moves the other way.

const PANE_COLS: usize = 7;
const PANE_ROWS: usize = 4;
const PANE_PAGE: usize = PANE_COLS * PANE_ROWS;
const YOU_X: i16 = 22;
const BOX_X: i16 = 176;
const PANE_Y: i16 = 53;

/// Frames a repeating button (SQUARE placing ones, CROSS on the result) has been held.
static mut HOLD_T: u16 = 0;
/// Cursor: column 0..6 is your pane, 7.. the container's; row 0..3.
static mut BOX_X_CUR: usize = 0;
static mut BOX_Y_CUR: usize = 0;
/// Page of each pane (L2/R2 page the pane under the cursor).
static mut BOX_PAGE: [usize; 2] = [0; 2];

/// How many items a held button moves this frame: one on the press, then after
/// a short delay a repeat that speeds up the longer it is held, ending at a
/// handful per frame so a stack of hundreds still empties in a few seconds.
#[optimize(size)]
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

/// Where the stack in hand was lifted from (CARRY_FROM): a hotbar slot is
/// 0..9, and these are everything else.
const FROM_GRID: i8 = -1;
const FROM_OFF: i8 = -2;
const FROM_CELL: i8 = -3;
const FROM_PACK: i8 = -4;
/// Your pane of a chest or furnace.
const FROM_MINE: i8 = -5;
/// The chest's pane, or the furnace's input (FIN) or output (FOUT) slot.
const FROM_BOX: i8 = -6;
const FROM_FIN: i8 = -7;
const FROM_FOUT: i8 = -8;

/// Pick up `n` of `item`, lifted from `from`. The stack stays where it is
/// until it is placed; the hand is a pointer to it.
#[optimize(size)]
fn lift(item: u8, n: u16, from: i8) {
    unsafe {
        CARRY = item;
        CARRY_N = n;
        CARRY_FROM = from;
    }
}

/// The hand's slot for the prompt table: `kind` and `count` are what the
/// cursor is on, `full` whether that counts as a stack.
#[optimize(size)]
fn spot(zone: Zone, kind: u8, count: u16, full: bool, other_side: bool) -> Spot {
    let held = unsafe { CARRY };
    let holding = held != AIR;
    Spot {
        zone,
        full,
        count,
        holding,
        same: holding && held == kind,
        from_other: holding && other_side,
    }
}

/// CIRCLE in a chest (menu 2) or furnace (3) with a stack in hand: put it
/// back where it was. False when the hand was empty, or on any other menu, so
/// CIRCLE closes the screen instead.
#[optimize(size)]
pub fn put_back(menu: u8) -> bool {
    if !(menu == 2 || menu == 3) || unsafe { CARRY } == AIR {
        return false;
    }
    unsafe { CARRY = AIR };
    sfx::blip();
    true
}

/// A held SQUARE places one at a time, speeding up like the old repeat.
/// Only a press made while a stack was in hand starts it.
static mut ONE_ARMED: bool = false;
#[optimize(size)]
fn place_ones(pad: ButtonState, previous: ButtonState, can: bool) -> u16 {
    let held = pad.is_held(button::SQUARE);
    unsafe {
        if can && pad.pressed_since(previous, button::SQUARE) {
            ONE_ARMED = true;
        }
        if !held || !can {
            ONE_ARMED = false;
        }
        hold_step(ONE_ARMED && held)
    }
}

/// Reset the container cursor onto your pane.
#[optimize(size)]
pub fn container_open() {
    unsafe {
        HOLD_T = 0;
        CARRY = AIR;
        ONE_ARMED = false;
        BOX_X_CUR = 0;
        BOX_Y_CUR = 0;
        BOX_PAGE = [0; 2];
        NAV_T = [0; 4];
        TAB = 0;
    }
}

/// Snap the cursor over `cols` columns (both panes) and `rows` rows.
#[optimize(size)]
fn box_nav(pad: ButtonState, cols: usize, rows: usize) {
    let t = unsafe { &mut NAV_T };
    let dirs = [button::UP, button::DOWN, button::LEFT, button::RIGHT];
    let mut d = 0;
    while d < 4 {
        if nav_repeat(pad.is_held(dirs[d]), &mut t[d]) {
            unsafe {
                match d {
                    0 => BOX_Y_CUR = (BOX_Y_CUR + rows - 1) % rows,
                    1 => BOX_Y_CUR = (BOX_Y_CUR + 1) % rows,
                    2 => BOX_X_CUR = (BOX_X_CUR + cols - 1) % cols,
                    _ => BOX_X_CUR = (BOX_X_CUR + 1) % cols,
                }
            }
            sfx::blip();
        }
        d += 1;
    }
}

/// The kind under the cursor in a pane listing `list[..n]`.
#[optimize(size)]
fn pane_item(list: &[u8; BLOCK_KINDS], n: usize, pane: usize, col: usize) -> u8 {
    let i = unsafe { BOX_PAGE[pane] } * PANE_PAGE + unsafe { BOX_Y_CUR } * PANE_COLS + col;
    if i < n {
        list[i]
    } else {
        AIR
    }
}

/// L1/R1 tabs and L2/R2 pages, shared by the chest's panes.
#[optimize(size)]
fn box_pages(pad: ButtonState, previous: ButtonState, pane: usize, n: usize) {
    let np = pages_of(n, PANE_PAGE);
    unsafe {
        if np > 1 && pad.pressed_since(previous, button::R2) {
            BOX_PAGE[pane] = (BOX_PAGE[pane] + 1) % np;
            sfx::blip();
        }
        if np > 1 && pad.pressed_since(previous, button::L2) {
            BOX_PAGE[pane] = (BOX_PAGE[pane] + np - 1) % np;
            sfx::blip();
        }
        if BOX_PAGE[pane] >= np {
            BOX_PAGE[pane] = np - 1;
        }
    }
}

fn pages_of(n: usize, per: usize) -> usize {
    if n == 0 {
        1
    } else {
        (n + per - 1) / per
    }
}

/// The chest cursor's slot: zone, kind and how many.
#[optimize(size)]
fn chest_slot(idx: usize) -> (Zone, u8, u16) {
    let (x, tab) = unsafe { (BOX_X_CUR, TAB) };
    let pane = (x >= PANE_COLS) as usize;
    let mut list = [0u8; BLOCK_KINDS];
    let n = unsafe {
        if pane == 0 {
            tab_kinds(tab, false, &INV, &mut list)
        } else {
            tab_kinds(tab, false, &CHEST_INV[idx], &mut list)
        }
    };
    let item = pane_item(&list, n, pane, x % PANE_COLS);
    let count = if item == AIR {
        0
    } else {
        unsafe {
            if pane == 0 {
                INV[item as usize]
            } else {
                CHEST_INV[idx][item as usize]
            }
        }
    };
    (if pane == 0 { Zone::Mine } else { Zone::Theirs }, item, count)
}

#[optimize(size)]
fn chest_spot(idx: usize) -> Spot {
    let (zone, item, count) = chest_slot(idx);
    let from = unsafe { CARRY_FROM };
    let other = if zone == Zone::Mine { from == FROM_BOX } else { from == FROM_MINE };
    spot(zone, item, count, item != AIR && count > 0, other)
}

/// Move up to `m` of `item` across: into the chest, or back to your pack.
#[optimize(size)]
fn chest_move(idx: usize, item: u8, m: u16, to_chest: bool) -> u16 {
    unsafe {
        let m = m.min(if to_chest {
            INV[item as usize]
        } else {
            CHEST_INV[idx][item as usize]
        });
        if m == 0 {
            return 0;
        }
        if to_chest {
            INV[item as usize] -= m;
            CHEST_INV[idx][item as usize] = CHEST_INV[idx][item as usize].saturating_add(m);
        } else {
            CHEST_INV[idx][item as usize] -= m;
            inv_give(item, m);
        }
        sfx::blip();
        m
    }
}

/// Put `want` of the stack in hand on the other pane. The hand empties when
/// the stack is all placed or the source has none left.
#[optimize(size)]
fn chest_place(idx: usize, to_chest: bool, want: u16) {
    unsafe {
        let item = CARRY;
        let m = chest_move(idx, item, want.min(CARRY_N), to_chest);
        CARRY_N -= m;
        let left = if to_chest {
            INV[item as usize]
        } else {
            CHEST_INV[idx][item as usize]
        };
        if CARRY_N == 0 || left == 0 {
            CARRY = AIR;
        }
    }
}

#[inline(never)]
#[optimize(size)]
pub fn chest_input(idx: usize, pad: ButtonState, previous: ButtonState) {
    let pressed = |b: u16| pad.pressed_since(previous, b);
    if pressed(button::L1) || pressed(button::R1) {
        unsafe {
            TAB = if pressed(button::R1) {
                (TAB + 1) % TABS
            } else {
                (TAB + TABS - 1) % TABS
            };
            BOX_PAGE = [0; 2];
        }
        sfx::blip();
    }
    box_nav(pad, 2 * PANE_COLS, PANE_ROWS);
    let (x, tab) = unsafe { (BOX_X_CUR, TAB) };
    let pane = (x >= PANE_COLS) as usize;
    let mut list = [0u8; BLOCK_KINDS];
    let n = unsafe {
        if pane == 0 {
            tab_kinds(tab, false, &INV, &mut list)
        } else {
            tab_kinds(tab, false, &CHEST_INV[idx], &mut list)
        }
    };
    box_pages(pad, previous, pane, n);
    let (zone, item, count) = chest_slot(idx);
    let s = chest_spot(idx);
    let mine = zone == Zone::Mine;
    let side = if mine { FROM_MINE } else { FROM_BOX };
    let ones = place_ones(pad, previous, hand::decide(s, Btn::Square) == Act::One);
    match hand::decide(s, Btn::Cross) {
        Act::Take if pressed(button::CROSS) => {
            lift(item, count, side);
            sfx::blip();
        }
        Act::Place if pressed(button::CROSS) => chest_place(idx, !mine, unsafe { CARRY_N }),
        _ => {}
    }
    if pressed(button::SQUARE) && hand::decide(s, Btn::Square) == Act::Half {
        lift(item, hand::half(count), side);
        sfx::blip();
    }
    if ones > 0 {
        chest_place(idx, !mine, ones);
    }
    if pressed(button::TRIANGLE) && hand::decide(s, Btn::Triangle) != Act::None {
        chest_move(idx, item, count, mine);
    }
}

/// Draw a count of half smelts as smelts ("7" or "7.5"; coal is 8, a log
/// 1.5, as in Java). Fuel has been stored in halves since save version 7 and
/// the strip printed the halves, twice the real figure. Returns the end x.
#[optimize(size)]
fn draw_halves(font: &FontAtlas, x: i16, y: i16, halves: u16) -> i16 {
    let mut b = [0u8; 5];
    let whole = number(halves / FUEL_PER_SMELT, &mut b);
    let mut end = x + whole.len() as i16 * 8;
    ui_text(font, x, y, whole, GREY);
    if halves % FUEL_PER_SMELT != 0 {
        ui_text(font, end, y, ".5", GREY);
        end += 16;
    }
    end
}

/// Your smeltables and fuels, in FURN_ITEMS order.
#[optimize(size)]
fn furnace_list(out: &mut [u8; BLOCK_KINDS]) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < FURN_ITEMS.len() {
        if unsafe { INV[FURN_ITEMS[i] as usize] } > 0 {
            out[n] = FURN_ITEMS[i];
            n += 1;
        }
        i += 1;
    }
    n
}

/// The furnace pane's three slots, as cursor rows: input, output, fuel.
const F_IN: usize = 0;
const F_OUT: usize = 1;
const F_FUEL: usize = 2;

/// Put up to `want` of `item` from your pack into the furnace: fuel burns,
/// ore smelts. Stops when the input slot holds a different ore.
#[optimize(size)]
fn furn_fill(idx: usize, item: u8, want: u16) -> u16 {
    let mut moved = 0;
    while moved < want {
        let before = unsafe { INV[item as usize] };
        furn_deposit(idx, item);
        if unsafe { INV[item as usize] } == before {
            break;
        }
        moved += 1;
    }
    moved
}

/// Take up to `want` out of the furnace's input (F_IN) or output (F_OUT)
/// slot into your pack. Returns how many came out.
#[optimize(size)]
fn furn_pull(idx: usize, y: usize, want: u16) -> u16 {
    unsafe {
        let (kind, count) = if y == F_IN {
            (&mut FURN_IN[idx], &mut FURN_IN_N[idx])
        } else {
            (&mut FURN_OUT[idx], &mut FURN_OUT_N[idx])
        };
        let m = want.min(*count);
        if m > 0 {
            if y == F_OUT {
                crate::furnace_took(idx, *kind, m);
            }
            inv_give(*kind, m);
            *count -= m;
            if *count == 0 {
                *kind = AIR;
                if y == F_IN {
                    FURN_PROG[idx] = 0;
                }
            }
            sfx::blip();
        }
        m
    }
}

/// Whether the furnace slot under the cursor takes this kind: the fuel slot
/// what burns, the input slot what smelts and does not burn.
#[optimize(size)]
fn furn_fits(zone: Zone, item: u8) -> bool {
    if zone == Zone::FurnFuel {
        fuel_smelts(item) > 0
    } else {
        fuel_smelts(item) == 0 && smelt_result(item) != AIR
    }
}

/// The furnace cursor's slot: zone, kind and how many.
#[optimize(size)]
fn furnace_slot(idx: usize) -> (Zone, u8, u16) {
    let (x, y) = unsafe { (BOX_X_CUR, BOX_Y_CUR) };
    unsafe {
        if x < PANE_COLS {
            let mut list = [0u8; BLOCK_KINDS];
            let n = furnace_list(&mut list);
            let item = pane_item(&list, n, 0, x);
            let count = if item == AIR { 0 } else { INV[item as usize] };
            return (Zone::MineFurnace, item, count);
        }
        match y {
            F_IN => (Zone::FurnIn, FURN_IN[idx], FURN_IN_N[idx]),
            F_OUT => (Zone::FurnOut, FURN_OUT[idx], FURN_OUT_N[idx]),
            _ => (Zone::FurnFuel, AIR, 0), // burnt as it goes: nothing to lift
        }
    }
}

#[optimize(size)]
fn furnace_spot(idx: usize) -> Spot {
    let (zone, item, count) = furnace_slot(idx);
    let from = unsafe { CARRY_FROM };
    // A stack from your pane only goes in the slot it suits; the output
    // takes nothing (the table has no Place for it either way).
    let other = if zone == Zone::MineFurnace {
        from == FROM_FIN || from == FROM_FOUT
    } else {
        from == FROM_MINE && furn_fits(zone, unsafe { CARRY })
    };
    spot(zone, item, count, item != AIR && count > 0, other)
}

/// Place `want` of the stack in hand on the furnace slot under the cursor:
/// from your pack into the input or fuel slot, or from the furnace back to
/// your pack.
#[optimize(size)]
fn furn_place(idx: usize, zone: Zone, want: u16) {
    unsafe {
        let item = CARRY;
        let want = want.min(CARRY_N);
        let m = if zone == Zone::MineFurnace {
            let y = if CARRY_FROM == FROM_FIN { F_IN } else { F_OUT };
            let m = furn_pull(idx, y, want);
            let left = if y == F_IN { FURN_IN_N[idx] } else { FURN_OUT_N[idx] };
            if left == 0 {
                CARRY_N = m; // the slot is empty: this was all of it
            }
            m
        } else if furn_fits(zone, item) {
            let m = furn_fill(idx, item, want);
            if INV[item as usize] == 0 {
                CARRY_N = m;
            }
            m
        } else {
            0
        };
        sfx::blip();
        CARRY_N -= m;
        if CARRY_N == 0 {
            CARRY = AIR;
        }
    }
}

#[inline(never)]
#[optimize(size)]
pub fn furnace_input(idx: usize, pad: ButtonState, previous: ButtonState) {
    // Your pane is 7x4; the furnace pane is one column of three slots.
    let right = unsafe { BOX_X_CUR } >= PANE_COLS;
    box_nav(pad, PANE_COLS + 1, if right { 3 } else { PANE_ROWS });
    unsafe {
        if BOX_X_CUR >= PANE_COLS {
            BOX_X_CUR = PANE_COLS;
            if BOX_Y_CUR > F_FUEL {
                BOX_Y_CUR = F_FUEL;
            }
        }
    }
    let (zone, item, count) = furnace_slot(idx);
    let s = furnace_spot(idx);
    let side = match zone {
        Zone::MineFurnace => FROM_MINE,
        Zone::FurnIn => FROM_FIN,
        _ => FROM_FOUT,
    };
    let pressed = |b: u16| pad.pressed_since(previous, b);
    let ones = place_ones(pad, previous, hand::decide(s, Btn::Square) == Act::One);
    match hand::decide(s, Btn::Cross) {
        Act::Take if pressed(button::CROSS) => {
            lift(item, count, side);
            sfx::blip();
        }
        Act::Place if pressed(button::CROSS) => furn_place(idx, zone, unsafe { CARRY_N }),
        _ => {}
    }
    if pressed(button::SQUARE) && hand::decide(s, Btn::Square) == Act::Half {
        lift(item, hand::half(count), side);
        sfx::blip();
    }
    if ones > 0 {
        furn_place(idx, zone, ones);
    }
    if pressed(button::TRIANGLE) {
        match hand::decide(s, Btn::Triangle) {
            Act::ToFurnace => {
                furn_fill(idx, item, count);
                sfx::blip();
            }
            Act::ToPack => {
                furn_pull(idx, if zone == Zone::FurnIn { F_IN } else { F_OUT }, count);
            }
            _ => {}
        }
    }
}


/// A pane of kinds with counts from `counts`, and its page marker.
#[optimize(size)]
fn draw_pane(
    font: &FontAtlas,
    x0: i16,
    list: &[u8; BLOCK_KINDS],
    n: usize,
    page: usize,
    counts: &[u16; BLOCK_KINDS],
) {
    let mut k = 0;
    while k < PANE_PAGE {
        let x = x0 + (k % PANE_COLS) as i16 * SLOT;
        let y = PANE_Y + (k / PANE_COLS) as i16 * SLOT;
        slot(x, y);
        let i = page * PANE_PAGE + k;
        if i < n {
            let b = list[i];
            draw_icon(x + 1, y + 1, b, 128);
            let c = counts[b as usize];
            if c > 1 {
                draw_count(x, y, c);
            }
        }
        k += 1;
    }
    let np = pages_of(n, PANE_PAGE);
    if np > 1 {
        let mut a = [0u8; 5];
        let mut b = [0u8; 5];
        let s1 = number(page as u16 + 1, &mut a);
        let s2 = number(np as u16, &mut b);
        let x = x0 + 126 - (s1.len() + s2.len() + 1) as i16 * 8;
        ui_text(font, x, 43, s1, MC_INK);
        ui_text(font, x + s1.len() as i16 * 8, 43, "/", MC_INK);
        ui_text(font, x + (s1.len() as i16 + 1) * 8, 43, s2, MC_INK);
    }
}

/// One prompt: the button's pill and what it does for the slot under the
/// cursor (nothing is drawn for a button that does nothing there). Returns
/// the next free x.
#[optimize(size)]
fn prompt(font: &FontAtlas, x: i16, y: i16, s: Spot, b: Btn) -> i16 {
    let a = hand::decide(s, b);
    if a == Act::None {
        return x;
    }
    let (key, tint) = match b {
        Btn::Cross => ("X", PS_CROSS),
        Btn::Square => ("[]", PS_SQUARE),
        Btn::Triangle => ("T", PS_TRIANGLE),
        Btn::Circle => ("O", PS_CIRCLE),
    };
    hint_item(font, x, y, key, tint, hand::label(a))
}

/// The prompt bar every screen shares: what X, SQUARE and TRIANGLE do for the
/// slot under the cursor on the first line, CIRCLE on the second (the caller
/// adds its tab and page buttons after it). Returns the next free x there.
#[optimize(size)]
fn prompt_bar(font: &FontAtlas, y1: i16, y2: i16, s: Spot) -> i16 {
    let mut x = 16;
    x = prompt(font, x, y1, s, Btn::Cross);
    x = prompt(font, x, y1, s, Btn::Square);
    prompt(font, x, y1, s, Btn::Triangle);
    prompt(font, 16, y2, s, Btn::Circle)
}

/// The stack in hand rides the cursor, up and to the right.
#[optimize(size)]
fn draw_hand(sx: i16, sy: i16) {
    let (c, n) = unsafe { (CARRY, CARRY_N) };
    if c == AIR {
        return;
    }
    draw_icon(sx + 6, sy - 10, c, 128);
    if n > 1 {
        draw_count(sx + 6, sy - 11, n);
    }
}

/// The info strip's name line while a stack is in hand: "HOLDING 12 NAME".
#[optimize(size)]
fn holding_line(font: &FontAtlas, x: i16, y: i16) {
    let (c, n) = unsafe { (CARRY, CARRY_N) };
    let mut nb = [0u8; 5];
    ui_text(font, x, y, "HOLDING", LABEL);
    let num = number(n, &mut nb);
    ui_text(font, x + 64, y, num, LABEL);
    ui_text(font, x + 72 + num.len() as i16 * 8, y, block_name(c), LABEL);
}

/// "YOU 37   CHEST 12" style counts line.
#[optimize(size)]
fn two_counts(font: &FontAtlas, y: i16, a: &str, an: u16, b: &str, bn: u16) {
    let mut ab = [0u8; 5];
    let mut bb = [0u8; 5];
    let s1 = number(an, &mut ab);
    ui_text(font, 42, y, a, GREY);
    let x = 42 + (a.len() as i16 + 1) * 8;
    ui_text(font, x, y, s1, GREY);
    let x = x + (s1.len() as i16 + 3) * 8;
    ui_text(font, x, y, b, GREY);
    ui_text(
        font,
        x + (b.len() as i16 + 1) * 8,
        y,
        number(bn, &mut bb),
        GREY,
    );
}

#[inline(never)]
#[optimize(size)]
pub fn draw_chest(font: &FontAtlas, idx: usize, player: &Player) {
    let (tab, x, y, pg) = unsafe { (TAB, BOX_X_CUR, BOX_Y_CUR, BOX_PAGE) };
    dim_screen();
    panel(8, 3, 304, 207);
    draw_centered(font, 8, "CHEST", MC_INK);
    tabs(font, 20, &TAB_TILES, tab);
    ui_text(font, YOU_X, 43, "YOU", MC_INK);
    ui_text(font, BOX_X, 43, "CHEST", MC_INK);
    let (inv, boxed) = unsafe { (&INV, &CHEST_INV[idx]) };
    let mut mine = [0u8; BLOCK_KINDS];
    let mut theirs = [0u8; BLOCK_KINDS];
    let nm = tab_kinds(tab, false, inv, &mut mine);
    let nt = tab_kinds(tab, false, boxed, &mut theirs);
    draw_pane(font, YOU_X, &mine, nm, pg[0], inv);
    draw_pane(font, BOX_X, &theirs, nt, pg[1], boxed);
    ui_text(font, 154, 70, ">", MC_INK);
    ui_text(font, 154, 86, "<", MC_INK);

    let pane = (x >= PANE_COLS) as usize;
    let item = if pane == 0 {
        pane_item(&mine, nm, 0, x)
    } else {
        pane_item(&theirs, nt, 1, x - PANE_COLS)
    };
    mc_slot(16, 130, 288, 34);
    let hand = unsafe { CARRY };
    let shown = if hand != AIR { hand } else { item };
    if shown != AIR {
        draw_icon(22, 135, shown, 128);
        if hand != AIR {
            holding_line(font, 42, 134);
        } else {
            ui_text(font, 42, 134, block_name(shown), LABEL);
        }
        two_counts(
            font,
            148,
            "YOU",
            inv[shown as usize],
            "CHEST",
            boxed[shown as usize],
        );
    } else {
        ui_text(
            font,
            42,
            141,
            if pane == 0 {
                "NOTHING OF YOURS HERE"
            } else {
                "NOTHING STORED HERE"
            },
            GREY,
        );
    }
    let nx = prompt_bar(font, 174, 190, chest_spot(idx));
    let nx = hint_item(font, nx, 190, "L1R1", PS_KEY, "TAB");
    if pages_of(nm, PANE_PAGE) > 1 || pages_of(nt, PANE_PAGE) > 1 {
        hint_item(font, nx, 190, "L2R2", PS_KEY, "PAGE");
    }
    draw_hotbar(hud_tool(player, AIR));
    let cx = if pane == 0 {
        YOU_X + x as i16 * SLOT
    } else {
        BOX_X + (x - PANE_COLS) as i16 * SLOT
    };
    let cy = PANE_Y + y as i16 * SLOT;
    frame(cx, cy, CURSOR);
    draw_hand(cx, cy);
}

// Furnace pane geometry: input over fuel on the left, the arrow, the output.
const F_SLOT_X: i16 = 206;
const F_IN_Y: i16 = 53;
const F_FUEL_Y: i16 = 107;
const F_OUT_X: i16 = 270;
const F_OUT_Y: i16 = 80;

#[inline(never)]
#[optimize(size)]
pub fn draw_furnace(font: &FontAtlas, idx: usize, player: &Player) {
    let (x, y, pg) = unsafe { (BOX_X_CUR, BOX_Y_CUR, BOX_PAGE[0]) };
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
    dim_screen();
    panel(8, 3, 304, 207);
    draw_centered(font, 8, "FURNACE", MC_INK);
    ui_text(font, YOU_X, 43, "YOU", MC_INK);
    ui_text(font, F_SLOT_X - 22, F_IN_Y + 5, "IN", MC_INK);
    let mut mine = [0u8; BLOCK_KINDS];
    let nm = furnace_list(&mut mine);
    draw_pane(font, YOU_X, &mine, nm, pg, unsafe { &INV });
    ui_text(font, 154, 78, ">", MC_INK);

    // Input, fuel (coal-ish icon with the smelts left), progress, output.
    slot(F_SLOT_X, F_IN_Y);
    if inn != AIR {
        draw_icon(F_SLOT_X + 1, F_IN_Y + 1, inn, 128);
        draw_count(F_SLOT_X, F_IN_Y, in_n);
    }
    ui_text(font, F_SLOT_X - 38, F_FUEL_Y + 5, "FUEL", MC_INK);
    slot(F_SLOT_X, F_FUEL_Y);
    if fuel > 0 {
        draw_icon(F_SLOT_X + 1, F_FUEL_Y + 1, COAL_ORE, 128);
        draw_count(F_SLOT_X, F_FUEL_Y, fuel / 2); // whole smelts left (fuel is in halves)
    }
    // Flame between input and fuel: lit while there is fuel.
    let lit = if fuel > 0 {
        (240, 150, 40)
    } else {
        (90, 90, 96)
    };
    rect(F_SLOT_X + 5, F_IN_Y + 22, 8, 6, lit.0, lit.1, lit.2);
    rect(F_SLOT_X + 7, F_IN_Y + 20, 4, 2, lit.0, lit.1, lit.2);
    // Arrow: grey track filling orange with the smelt progress.
    let (ax, ay, aw) = (F_SLOT_X + 24, F_OUT_Y + 6, 38i16);
    rect(ax, ay, aw, 5, 0x8B, 0x8B, 0x8B);
    let w = prog as i16 * aw / SMELT_TIME as i16;
    if w > 0 {
        rect(ax, ay, w, 5, 230, 140, 40);
    }
    ui_text(font, F_OUT_X - 4, F_OUT_Y - 10, "OUT", MC_INK);
    slot(F_OUT_X, F_OUT_Y);
    if outt != AIR {
        draw_icon(F_OUT_X + 1, F_OUT_Y + 1, outt, 128);
        draw_count(F_OUT_X, F_OUT_Y, out_n);
    }

    // Info strip.
    mc_slot(16, 130, 288, 34);
    let right = x >= PANE_COLS;
    let item = if right {
        match y {
            F_IN => inn,
            F_OUT => outt,
            _ => {
                if fuel > 0 {
                    COAL_ORE
                } else {
                    AIR
                }
            }
        }
    } else {
        pane_item(&mine, nm, 0, x)
    };
    let hand = unsafe { CARRY };
    if hand != AIR {
        draw_icon(22, 135, hand, 128);
        holding_line(font, 42, 134);
        let from_mine = unsafe { CARRY_FROM } == FROM_MINE;
        let note = if !right || !from_mine {
            "O PUTS IT BACK"
        } else if y == F_OUT {
            "THE OUTPUT ONLY GIVES"
        } else if !furn_fits(if y == F_FUEL { Zone::FurnFuel } else { Zone::FurnIn }, hand) {
            if y == F_FUEL {
                "THAT WILL NOT BURN"
            } else {
                "THAT WILL NOT SMELT"
            }
        } else {
            "O PUTS IT BACK"
        };
        ui_text(font, 42, 148, note, GREY);
    } else if right && y == F_FUEL {
        ui_text(font, 42, 134, "FUEL", LABEL);
        let end = draw_halves(font, 42, 148, fuel);
        ui_text(font, end + 8, 148, "SMELTS LEFT", GREY);
    } else if item != AIR {
        draw_icon(22, 135, item, 128);
        ui_text(font, 42, 134, block_name(item), LABEL);
        if right {
            let (what, n) = if y == F_IN {
                ("SMELTING", in_n)
            } else {
                ("READY", out_n)
            };
            two_counts(font, 148, what, n, "YOU", unsafe { INV[item as usize] });
        } else {
            let f = fuel_smelts(item);
            if f > 0 {
                ui_text(font, 42, 148, "FUEL: SMELTS", GREY);
                let end = draw_halves(font, 42 + 13 * 8, 148, f);
                ui_text(font, end + 8, 148, "EACH", GREY);
            } else {
                ui_text(font, 42, 148, "SMELTS INTO", GREY);
                ui_text(font, 42 + 12 * 8, 148, block_name(smelt_result(item)), GREY);
            }
        }
    } else {
        ui_text(
            font,
            42,
            141,
            if right {
                "EMPTY"
            } else {
                "NOTHING TO SMELT OR BURN"
            },
            GREY,
        );
    }
    let nx = prompt_bar(font, 174, 190, furnace_spot(idx));
    if pages_of(nm, PANE_PAGE) > 1 {
        hint_item(font, nx, 190, "L2R2", PS_KEY, "PAGE");
    }
    draw_hotbar(hud_tool(player, AIR));
    let (cx, cy) = if right {
        match y {
            F_IN => (F_SLOT_X, F_IN_Y),
            F_OUT => (F_OUT_X, F_OUT_Y),
            _ => (F_SLOT_X, F_FUEL_Y),
        }
    } else {
        (YOU_X + x as i16 * SLOT, PANE_Y + y as i16 * SLOT)
    };
    frame(cx, cy, CURSOR);
    draw_hand(cx, cy);
}

// ---------------------------------------------------------------------------
// The inventory: a Legacy Console style icon grid over the count-per-kind
// inventory. Nothing here is a Java slot: a kind shows once with its count,
// and the only layout the player owns is the hotbar, which sits live under the
// grid as the grid's fifth row. L1/R1 pick a tab, the D-pad snaps a cursor
// (hold to repeat), X takes a kind and places it on a hotbar slot (swapping
// with what is there), TRIANGLE quick-moves it to the first free slot or, on
// the hotbar, back into the pack, CIRCLE puts a held kind back, SELECT shows
// every kind of the tab instead of only what you own.

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
    GRASS,
    DIRT,
    STONE,
    COBBLE,
    SLAB,
    STAIRS_N,
    BRICK,
    WOOD,
    PLANK,
    FENCE,
    LEAVES,
    SAND,
    SNOW,
    GLASS,
    WOOL,
    OBSIDIAN,
    CINDERSTONE,
    SINK_SAND,
    LUMISTONE,
    VOID_STONE,
    CACTUS,
    SAPLING,
    LADDER,
    DOOR_C,
    CRAFT_TABLE,
    CHEST,
    FURNACE,
    ENCHANT,
    BED,
    TORCH,
    WIRE,
    PISTON,
    TNT,
];
const TAB_ITEMS: [u8; 21] = [
    BOW,
    FISHING_ROD,
    BUCKET,
    WATER_BUCKET,
    LAVA_BUCKET,
    FLINT_STEEL,
    VOID_EYE,
    BONEMEAL,
    BOTTLE,
    POTION_SPEED,
    POTION_STRENGTH,
    POTION_REGEN,
    POTION_FIRE,
    ARMOR0,
    ARMOR0 + 1,
    ARMOR0 + 2,
    ARMOR0 + 3,
    ARMOR0 + 4,
    ARMOR0 + 5,
    ARMOR0 + 6,
    ARMOR0 + 7,
];
const TAB_FOOD: [u8; 5] = [BREAD, COOKED_MEAT, RAW_MEAT, WHEAT_ITEM, SEEDS];
/// True for the crafting materials the inventory lists under MATERIALS.
pub fn is_material(item: u8) -> bool {
    TAB_MATERIALS.contains(&item)
}
const TAB_MATERIALS: [u8; 18] = [
    COAL_ORE,
    IRON_ORE,
    IRON_INGOT,
    GOLD_ORE,
    DIAMOND_ORE,
    STICK,
    STRING,
    BONE,
    GUNPOWDER,
    ARROW,
    CLAY,
    SUGAR_CANE,
    EMBER_CAP,
    EMBER_ROD,
    MAGMA_PASTE,
    WAILER_TEAR,
    VOID_PEARL,
    POTION_AWKWARD,
];

#[optimize(size)]
fn tab_table(tab: usize) -> &'static [u8] {
    match tab {
        0 => &TAB_BLOCKS,
        1 => &TAB_ITEMS,
        2 => &TAB_FOOD,
        _ => &TAB_MATERIALS,
    }
}

#[optimize(size)]
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
#[optimize(size)]
fn tab_list(tab: usize, all: bool, out: &mut [u8; BLOCK_KINDS]) -> usize {
    tab_kinds(tab, all, unsafe { &INV }, out)
}

/// `tab_list` over any count table: the player's, or a chest's.
#[inline(never)]
#[optimize(size)]
fn tab_kinds(
    tab: usize,
    all: bool,
    counts: &[u16; BLOCK_KINDS],
    out: &mut [u8; BLOCK_KINDS],
) -> usize {
    let table = tab_table(tab);
    let mut n = 0;
    let mut i = 0;
    while i < table.len() {
        if all || counts[table[i] as usize] > 0 {
            out[n] = table[i];
            n += 1;
        }
        i += 1;
    }
    if tab == TABS - 1 {
        let mut k = 1;
        while k < BLOCK_KINDS {
            if counts[k] > 0 && !listed(k as u8) {
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
/// How many the carried kind stands for: the whole stack it was lifted from.
/// A grid cell takes up to this many from the inventory when it is put down.
static mut CARRY_N: u16 = 0;
/// A one-line note in the info strip after an action, for NOTE_T frames.
static mut NOTE: &str = "";
static mut NOTE_T: u8 = 0;
/// Hold-to-repeat timers: up, down, left, right.
static mut NAV_T: [u16; 4] = [0; 4];
/// Set on open: the TRIANGLE that opened the menu must not also Quick Move.
static mut FRESH: bool = false;

#[optimize(size)]
fn note(s: &'static str) {
    unsafe {
        NOTE = s;
        NOTE_T = 75;
    }
}

#[optimize(size)]
fn pages(n: usize) -> usize {
    if n == 0 {
        1
    } else {
        (n + PER_PAGE - 1) / PER_PAGE
    }
}

#[optimize(size)]
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
        ON_PLAYER = true;
        PZ = Z_ARMOR;
        PI = 0;
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
#[optimize(size)]
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
/// leaves its old one, or the off hand. An occupant swaps into that old place
/// when the carry came off the hotbar or the off hand, and otherwise is picked
/// up in turn.
#[inline(never)]
#[optimize(size)]
fn drop_on_hotbar(j: usize, p: &mut Player) {
    unsafe {
        let item = CARRY;
        let occupant = HOTBAR[j];
        let from_off = p.offhand == item;
        if let Some(k) = hotbar_slot_of(item) {
            HOTBAR[k] = AIR;
        }
        if from_off {
            p.offhand = AIR;
        }
        HOTBAR[j] = item;
        CARRY = AIR;
        if occupant != AIR && occupant != item {
            if from_off {
                p.offhand = occupant;
            } else if CARRY_FROM >= 0 {
                HOTBAR[CARRY_FROM as usize] = occupant;
            } else {
                CARRY = occupant;
                CARRY_FROM = FROM_GRID;
            }
        }
    }
}

/// One frame of inventory input. Returns true when the menu should close.
#[inline(never)]
#[optimize(size)]
pub fn inventory_input(pad: ButtonState, previous: ButtonState, player: &mut Player) -> bool {
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
        // The pages in display order: the player, then the four item tabs.
        unsafe {
            let d = if ON_PLAYER { 0 } else { TAB + 1 };
            let nd = if pressed(button::R1) {
                (d + 1) % PAGES
            } else {
                (d + PAGES - 1) % PAGES
            };
            ON_PLAYER = nd == 0;
            if nd > 0 {
                TAB = nd - 1;
            }
            PAGE = 0;
            CUR_X = 0;
            CUR_Y = if CUR_Y == HOT_ROW { HOT_ROW } else { 0 };
            NOTE_T = 0;
        }
        n = tab_list(unsafe { TAB }, all, &mut list);
        sfx::blip();
    }
    if unsafe { ON_PLAYER } {
        player_input(pad, previous, player);
        return false;
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
    let hot = cy == HOT_ROW;
    let kind = if hot { unsafe { HOTBAR[cx] } } else { item };
    let s = item_spot(hot, kind);
    if pressed(button::CROSS) {
        unsafe { NOTE_T = 0 }; // a new action replaces the last note
        match hand::decide(s, Btn::Cross) {
            Act::Take => lift(kind, unsafe { INV[kind as usize] }, if hot { cx as i8 } else { FROM_GRID }),
            Act::Place | Act::Swap => drop_on_hotbar(cx, player),
            _ => why_not(hot, item),
        }
        sfx::blip();
    }
    if pressed(button::SQUARE) && hand::decide(s, Btn::Square) == Act::Half {
        lift(kind, hand::half(s.count), FROM_GRID);
        sfx::blip();
    }
    if pressed(button::TRIANGLE) {
        unsafe { NOTE_T = 0 };
        match hand::decide(s, Btn::Triangle) {
            Act::ToHotbar => {
                if hotbar_slot_of(item).is_some() {
                    note("ALREADY ON THE HOTBAR");
                } else if let Some(j) = hotbar_slot_of(AIR) {
                    unsafe { HOTBAR[j] = item };
                    note("MOVED TO THE HOTBAR");
                } else {
                    note("HOTBAR FULL: TAKE ONE AND PLACE IT");
                }
            }
            Act::ToPack => unsafe {
                if CARRY == HOTBAR[cx] {
                    CARRY = AIR;
                }
                HOTBAR[cx] = AIR;
                note("BACK IN THE PACK");
            },
            _ => why_not(hot, item),
        }
        sfx::blip();
    }
    false
}

/// The slot under the item pages' cursor for the prompt table: a hotbar slot
/// or a kind in the grid. A kind you own none of, or cannot hold, is not a
/// stack to act on.
#[optimize(size)]
fn item_spot(hot: bool, kind: u8) -> Spot {
    let count = if kind == AIR { 0 } else { unsafe { INV[kind as usize] } };
    let full = kind != AIR && (hot || (count > 0 && holdable(kind)));
    spot(if hot { Zone::Hotbar } else { Zone::Grid }, kind, count, full, false)
}

/// Why a button did nothing on a grid kind that looks like a stack.
#[optimize(size)]
fn why_not(hot: bool, item: u8) {
    if hot || item == AIR || unsafe { CARRY } != AIR {
        return;
    }
    if unsafe { INV[item as usize] } == 0 {
        note("YOU HAVE NONE YET");
    } else if !holdable(item) {
        note("CAN'T BE HELD: USE IT IN RECIPES");
    }
}

// -- drawing ------------------------------------------------------------------

/// Inventory-sized dialog: black outline, light face, vanilla bevel.
#[inline(never)]
#[optimize(size)]
pub fn panel(x: i16, y: i16, w: i16, h: i16) {
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
#[optimize(size)]
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
#[optimize(size)]
fn purpose(item: u8) -> &'static str {
    match item {
        BREAD | COOKED_MEAT | RAW_MEAT => "EATEN WHEN YOU GET HUNGRY",
        ARROW => "AMMO FOR THE BOW",
        SEEDS => "L2 PLANTS THEM ON SOIL",
        WHEAT_ITEM => "ANIMALS FOLLOW IT. MAKES BREAD",
        _ if fuel_smelts(item) > 0 && !in_placeable(item) => "BURNS AS FURNACE FUEL",
        _ if smelt_result(item) != AIR && !in_placeable(item) => "SMELTS IN A FURNACE",
        _ if armor_piece(item).is_some() => "WEAR IT ON THE PLAYER PAGE",
        _ if !in_placeable(item) => "A CRAFTING MATERIAL",
        _ if unsafe { TAB } == 1 => "L2 USES IT.",
        _ => "L2 PLACES IT.",
    }
}

const TAB_TILES: [u8; TABS] = [
    tex::T_GRASS_SIDE,
    tex::T_I_BOW,
    tex::T_I_BREAD,
    tex::T_I_COAL,
];
const LABEL: (u8, u8, u8) = (0xE0, 0xE0, 0xE0);
const GREY: (u8, u8, u8) = (0xA8, 0xA8, 0xA8);

#[inline(never)]
#[optimize(size)]
pub fn draw_inventory(font: &FontAtlas, player: &Player) {
    let (tab, page, all, cx, cy, carry) = unsafe { (TAB, PAGE, SHOW_ALL, CUR_X, CUR_Y, CARRY) };
    let mut list = [0u8; BLOCK_KINDS];
    let n = tab_list(tab, all, &mut list);
    let np = pages(n);

    dim_screen();
    panel(8, 3, 304, 207);
    draw_centered(font, 8, "INVENTORY", MC_INK);
    if unsafe { ON_PLAYER } {
        tabs(font, 20, &PAGE_TILES, 0);
        draw_player_page(font, player);
        return;
    }
    tabs(font, 20, &PAGE_TILES, tab + 1);
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
    ];
    let mut g = 0;
    while g < gear.len() {
        let (x, y) = (16 + (g % 2) as i16 * 19, GRID_Y + (g / 2) as i16 * SLOT);
        slot(x, y);
        let (tile, tier) = gear[g];
        if tier > 0 {
            draw_tile(x + 1, y + 1, tile, tool_tint(tier));
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
    let shown_kind = if hot { unsafe { HOTBAR[cx] } } else { cursor_item(&list, n) };
    let shown = if carry != AIR { carry } else { shown_kind };
    if shown != AIR {
        draw_icon(22, 135, shown, 128);
        if carry != AIR {
            holding_line(font, 42, 134);
        } else {
            let name = block_name(shown);
            ui_text(font, 42, 134, name, LABEL);
            if unsafe { INV[shown as usize] } > 0 {
                let mut nb = [0u8; 5];
                let cnt = number(unsafe { INV[shown as usize] }, &mut nb);
                let nx = 42 + (name.len() as i16 + 1) * 8;
                ui_text(font, nx, 134, "X", GREY);
                ui_text(font, nx + 8, 134, cnt, GREY);
            }
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
            "CHOOSE A HOTBAR SLOT. O: BACK",
            GREY,
        );
    } else if shown != AIR {
        let p = purpose(shown);
        ui_text(font, 42, 148, p, GREY);
        if holdable(shown) && unsafe { INV[shown as usize] } > 0 {
            let x = 42 + (p.len() as i16 + 1) * 8;
            match hotbar_slot_of(shown) {
                Some(j) => {
                    ui_text(font, x, 148, "SLOT ", GREY);
                    let d = [b'1' + j as u8];
                    ui_text(
                        font,
                        x + 40,
                        148,
                        unsafe { core::str::from_utf8_unchecked(&d) },
                        GREY,
                    );
                }
                None => ui_text(font, x, 148, "NOT ON HOTBAR", GREY),
            }
        }
    } else if hot {
        ui_text(font, 42, 148, "AN EMPTY HOTBAR SLOT", GREY);
    }

    // The prompt bar: what each button does for the slot under the cursor.
    let x = prompt_bar(font, 168, 180, item_spot(hot, shown_kind));
    let x = hint_item(font, x, 180, "L1R1", PS_KEY, "TAB");
    if np > 1 {
        hint_item(font, x, 180, "L2R2", PS_KEY, "PAGE");
    }
    hint_item(font, 16, 192, "SEL", PS_KEY, if all { "SHOW OWNED" } else { "SHOW ALL" });

    // The live hotbar is the grid's fifth row, drawn over the dimming.
    draw_hotbar(hud_tool(player, AIR));
    let (sx, sy) = if hot {
        (HOTBAR_X0 + cx as i16 * SLOT, HUD_HOTBAR_Y)
    } else {
        (GRID_X + cx as i16 * SLOT, GRID_Y + cy as i16 * SLOT)
    };
    frame(sx, sy, CURSOR);
    draw_hand(sx, sy);
}

// -- the player page -----------------------------------------------------------
//
// Minecraft's survival inventory, as the first page of this screen: the four
// armour slots down the left, the player between them and the stats, the off
// hand, the weapon you swing, and the armour you carry. The hotbar is the
// bottom row, as on the item pages. Crafting by hand is SQUARE's pocket menu,
// which already offers just what a 2x2 grid can shape.

/// True while the player page, not an item tab, is showing.
static mut ON_PLAYER: bool = true;
/// The player page's cursor: zone and index within it.
static mut PZ: usize = 0;
static mut PI: usize = 0;
const Z_ARMOR: usize = 0;
const Z_OFF: usize = 1;
const Z_WEAP: usize = 2;
const Z_STOCK: usize = 3;
const Z_HOT: usize = 4;
const Z_GRID: usize = 5;
const Z_OUT: usize = 6;
const AX: i16 = 16;
const AY: i16 = 42;
const OFF_SX: i16 = 98;
const OFF_SY: i16 = 96;
const WX: i16 = 124;
const WY: i16 = 92;
const SX0: i16 = 16;
const SY0: i16 = 136;
const STOCK_N: usize = 8;
/// The 2x2 grid's top-left cell and its output slot.
const GX: i16 = 208;
const GY: i16 = 46;
const OUT_X: i16 = 264;
const OUT_Y: i16 = 55;
const SLOT_NAME: [&str; 4] = ["HELMET", "CHESTPLATE", "LEGGINGS", "BOOTS"];

/// Display order of the pages: the player first, then the item tabs.
const PAGES: usize = TABS + 1;
const PAGE_TILES: [u8; PAGES] = [
    tex::T_I_ARMOR,
    tex::T_GRASS_SIDE,
    tex::T_I_BOW,
    tex::T_I_BREAD,
    tex::T_I_COAL,
];

/// The armour kinds you carry, in kind order, up to STOCK_N.
#[optimize(size)]
fn stock(out: &mut [u8; STOCK_N]) -> usize {
    let mut n = 0;
    let mut k = ARMOR0;
    while k < ARMOR0 + ARMOR_KINDS && n < STOCK_N {
        if unsafe { INV[k as usize] } > 0 {
            out[n] = k;
            n += 1;
        }
        k += 1;
    }
    n
}

/// Where the cursor goes for a D-pad direction (0 up, 1 down, 2 left, 3
/// right), as (zone, index).
#[optimize(size)]
fn pmove(z: usize, i: usize, d: usize, ns: usize) -> (usize, usize) {
    let last = ns.max(1) - 1;
    match (z, d) {
        (Z_ARMOR, 0) => if i > 0 { (Z_ARMOR, i - 1) } else { (Z_HOT, 0) },
        (Z_ARMOR, 1) => if i < 3 { (Z_ARMOR, i + 1) } else { (Z_STOCK, 0) },
        (Z_ARMOR, 3) => if i == 3 { (Z_OFF, 0) } else { (Z_WEAP, 0) },
        (Z_OFF, 0) => (Z_ARMOR, 1),
        (Z_OFF, 1) => (Z_STOCK, 0),
        (Z_OFF, 2) => (Z_ARMOR, 3),
        (Z_OFF, 3) => (Z_WEAP, 0),
        (Z_WEAP, 0) => if i >= 3 { (Z_GRID, 2) } else { (Z_ARMOR, 0) },
        (Z_WEAP, 1) => (Z_STOCK, i.min(last)),
        (Z_WEAP, 2) => if i > 0 { (Z_WEAP, i - 1) } else { (Z_OFF, 0) },
        (Z_WEAP, 3) => if i < 4 { (Z_WEAP, i + 1) } else { (Z_GRID, 2) },
        (Z_GRID, 0) => if i >= 2 { (Z_GRID, i - 2) } else { (Z_HOT, 4) },
        (Z_GRID, 1) => if i < 2 { (Z_HOT, 5) } else { (Z_STOCK, 5 + i - 2) },
        (Z_GRID, 2) => if i % 2 == 1 { (Z_GRID, i - 1) } else { (Z_WEAP, 4) },
        (Z_GRID, 3) => if i % 2 == 0 { (Z_GRID, i + 1) } else { (Z_OUT, 0) },
        (Z_OUT, 0) => (Z_HOT, 7),
        (Z_OUT, 1) => (Z_STOCK, 7),
        (Z_OUT, 2) => (Z_GRID, 1),
        (Z_STOCK, 0) => if i < 2 { (Z_ARMOR, 3) } else if i >= 5 { (Z_GRID, 2 + (i >= 7) as usize) } else { (Z_WEAP, (i - 2).min(4)) },
        (Z_STOCK, 1) => (Z_HOT, i),
        (Z_STOCK, 2) => (Z_STOCK, (i + STOCK_N - 1) % STOCK_N),
        (Z_STOCK, 3) => (Z_STOCK, (i + 1) % STOCK_N),
        (Z_HOT, 0) => (Z_STOCK, i.min(STOCK_N - 1)),
        (Z_HOT, 1) => (Z_ARMOR, 0),
        (Z_HOT, 2) => (Z_HOT, (i + HOTBAR_VIS - 1) % HOTBAR_VIS),
        (Z_HOT, 3) => (Z_HOT, (i + 1) % HOTBAR_VIS),
        _ => (z, i),
    }
}

/// Put the carried kind in the off hand. A kind lives in one place, so the
/// slot it came from takes the old off-hand item in exchange.
#[inline(never)]
#[optimize(size)]
fn put_in_offhand(p: &mut Player) {
    unsafe {
        let item = CARRY;
        let old = p.offhand;
        if let Some(k) = hotbar_slot_of(item) {
            HOTBAR[k] = if old != item { old } else { AIR };
        }
        p.offhand = item;
        CARRY = AIR;
    }
}

/// What the player page's cursor is on, for the prompt table.
#[optimize(size)]
fn player_spot(z: usize, i: usize, p: &Player, ns: usize, st: &[u8; STOCK_N]) -> Spot {
    let owned = |k: u8| if k == AIR { 0 } else { unsafe { INV[k as usize] } };
    let (zone, kind, count, full) = match z {
        Z_ARMOR => (Zone::Armor, p.worn[i], 1, p.worn[i] != AIR),
        Z_OFF => (Zone::Off, p.offhand, owned(p.offhand), p.offhand != AIR),
        Z_WEAP => {
            let class = equip::WEAPONS[i];
            (Zone::Weapon, AIR, 0, class == TOOL_NONE || tool_tier(p, class) > 0)
        }
        Z_STOCK => {
            let k = if i < ns { st[i] } else { AIR };
            (Zone::Pack, k, owned(k), i < ns)
        }
        Z_GRID => (Zone::Cell, grid::kind(i), grid::count(i), grid::kind(i) != AIR),
        Z_OUT => (Zone::Result, AIR, 0, grid::output().is_some()),
        _ => {
            let k = unsafe { HOTBAR[i] };
            (Zone::Hotbar, k, owned(k), k != AIR)
        }
    };
    spot(zone, kind, count, full, false)
}

/// Put the armour piece in hand on armour slot `slot`.
#[optimize(size)]
fn wear_held(p: &mut Player, slot: usize) {
    let c = unsafe { CARRY };
    match armor_piece(c) {
        Some((_, s)) if s == slot => match equip::wear(p, c) {
            Ok(()) => {
                unsafe { CARRY = AIR };
                note("WORN");
                sfx::confirm();
            }
            Err(m) => note(m),
        },
        Some(_) => note("THAT PIECE GOES IN ANOTHER SLOT"),
        None => note("THAT IS NOT ARMOR"),
    }
}

/// Put the stack in hand into crafting cell `i`: onto an empty cell or the
/// same kind, or swapped with a different one, which comes into the hand.
#[inline(never)]
#[optimize(size)]
fn cell_place(i: usize) {
    unsafe {
        let c = CARRY;
        if INV[c as usize] == 0 {
            note("NONE LEFT");
            return;
        }
        let old = if grid::kind(i) != AIR && grid::kind(i) != c {
            Some(grid::take_all(i))
        } else {
            None
        };
        let m = grid::put(i, c, CARRY_N);
        if m == 0 {
            if let Some((k, n)) = old {
                grid::put(i, k, n);
            }
            note("THE CELL IS FULL");
            return;
        }
        CARRY_N -= m;
        match old {
            Some((k, n)) => lift(k, n, FROM_CELL),
            None if CARRY_N == 0 => CARRY = AIR,
            None => {}
        }
    }
}

/// One frame of the player page.
#[inline(never)]
#[optimize(size)]
fn player_input(pad: ButtonState, previous: ButtonState, p: &mut Player) {
    let pressed = |b: u16| pad.pressed_since(previous, b);
    let mut st = [0u8; STOCK_N];
    let ns = stock(&mut st);
    let t = unsafe { &mut NAV_T };
    let dirs = [button::UP, button::DOWN, button::LEFT, button::RIGHT];
    let mut d = 0;
    while d < 4 {
        if nav_repeat(pad.is_held(dirs[d]), &mut t[d]) {
            unsafe {
                let (z, i) = pmove(PZ, PI, d, ns);
                PZ = z;
                PI = i;
                NOTE_T = 0;
            }
            sfx::blip();
        }
        d += 1;
    }
    let (z, i) = unsafe { (PZ, PI) };
    let s = player_spot(z, i, p, ns, &st);
    if pressed(button::CROSS) {
        unsafe { NOTE_T = 0 };
        let own = |k: u8| unsafe { INV[k as usize] };
        match hand::decide(s, Btn::Cross) {
            Act::Take => match z {
                Z_OFF => lift(p.offhand, own(p.offhand), FROM_OFF),
                Z_STOCK => lift(st[i], own(st[i]), FROM_PACK),
                Z_GRID => {
                    let (k, n) = grid::take_all(i);
                    lift(k, n, FROM_CELL);
                }
                _ => lift(unsafe { HOTBAR[i] }, own(unsafe { HOTBAR[i] }), i as i8),
            },
            Act::Place | Act::Swap => match z {
                Z_OFF => put_in_offhand(p),
                Z_GRID => cell_place(i),
                _ => drop_on_hotbar(i, p),
            },
            Act::Wear => wear_held(p, i),
            Act::Equip => {
                p.weapon = equip::WEAPONS[i];
                note("WEAPON EQUIPPED");
                sfx::confirm();
            }
            Act::Craft => {
                if grid::craft_once() {
                    sfx::confirm();
                }
            }
            _ => match z {
                Z_WEAP => note("YOU HAVE NONE OF THOSE"),
                Z_OUT => note("PUT THE INGREDIENTS IN THE GRID"),
                _ => {}
            },
        }
        sfx::blip();
    }
    if pressed(button::SQUARE) {
        match hand::decide(s, Btn::Square) {
            Act::Half => {
                let (k, n) = grid::take_half(i);
                lift(k, n, FROM_CELL);
            }
            Act::One => unsafe {
                if grid::put(i, CARRY, 1) > 0 {
                    CARRY_N -= 1;
                    if CARRY_N == 0 {
                        CARRY = AIR;
                    }
                } else {
                    note("NONE LEFT OR THE CELL IS FULL");
                }
            },
            _ => {}
        }
    }
    if pressed(button::TRIANGLE) {
        unsafe { NOTE_T = 0 };
        match hand::decide(s, Btn::Triangle) {
            Act::ToPack => match z {
                Z_ARMOR => match equip::take_off(p, i) {
                    Ok(()) => note("TAKEN OFF"),
                    Err(m) => note(m),
                },
                Z_OFF => {
                    p.offhand = AIR;
                    note("BACK IN THE PACK");
                }
                Z_GRID => {
                    grid::clear_cell(i);
                    note("BACK IN THE PACK");
                }
                _ => unsafe {
                    if CARRY == HOTBAR[i] {
                        CARRY = AIR;
                    }
                    HOTBAR[i] = AIR;
                    note("BACK IN THE PACK");
                },
            },
            Act::Wear => match equip::wear(p, st[i]) {
                Ok(()) => {
                    if unsafe { CARRY } == st[i] {
                        unsafe { CARRY = AIR };
                    }
                    note("WORN");
                    sfx::confirm();
                }
                Err(m) => note(m),
            },
            Act::CraftAll => {
                if grid::craft_all() > 0 {
                    sfx::confirm();
                }
            }
            _ => {}
        }
    }
    // A held CROSS on the output keeps crafting, after a short delay. Only a
    // press made on the output arms it: the press that laid a recipe in the
    // grid is still down when this screen opens, and must not craft it.
    unsafe {
        if z == Z_OUT && pressed(button::CROSS) {
            OUT_ARMED = true;
        }
        if !pad.is_held(button::CROSS) || z != Z_OUT {
            OUT_ARMED = false;
        }
        if OUT_ARMED {
            let m = hold_step(true);
            if m > 1 || (m == 1 && !pressed(button::CROSS)) {
                let mut k = 0;
                while k < m && grid::craft_once() {
                    k += 1;
                }
            }
        } else {
            hold_step(false);
        }
    }
}

/// True while a CROSS pressed on the crafting output is still held.
static mut OUT_ARMED: bool = false;

/// Open on the player page with the cursor on the crafting output, as the
/// recipe book does after it lays a shape in the grid.
#[inline(never)]
#[optimize(size)]
pub fn inventory_open_grid() {
    unsafe {
        ON_PLAYER = true;
        PZ = Z_OUT;
        PI = 0;
        CARRY = AIR;
        NOTE_T = 0;
        NAV_T = [0; 4];
        FRESH = true;
        OUT_ARMED = false;
    }
}

/// Unpadded `v` as whole.tenths.
#[optimize(size)]
fn tenths(v: i32, buf: &mut [u8; 8]) -> &str {
    let mut n = [0u8; 5];
    let whole = number((v / 10) as u16, &mut n);
    let w = whole.len();
    buf[..w].copy_from_slice(whole.as_bytes());
    buf[w] = b'.';
    buf[w + 1] = b'0' + (v % 10) as u8;
    unsafe { core::str::from_utf8_unchecked(&buf[..w + 2]) }
}

/// A durability bar under an item icon (green to red), when worn.
#[optimize(size)]
fn wear_bar(x: i16, y: i16, left: i32, max: i32) {
    if left > 0 && left < max {
        let w = (14 * left / max).max(1) as i16;
        let g = (255 * left / max) as u8;
        rect(x + 1, y + 13, 14, 2, 0, 0, 0);
        rect(x + 1, y + 13, w, 1, 255 - g, g, 0);
    }
}

/// The player page: everything on it, then the cursor.
#[inline(never)]
#[optimize(size)]
fn draw_player_page(font: &FontAtlas, p: &Player) {
    let (z, i, carry) = unsafe { (PZ, PI, CARRY) };
    let mut st = [0u8; STOCK_N];
    let ns = stock(&mut st);

    // Armour slots, a ghost of the missing piece when bare.
    let mut s = 0;
    while s < 4 {
        let y = AY + s as i16 * SLOT;
        slot(AX, y);
        match armor_piece(p.worn[s]) {
            Some((t, _)) => {
                draw_icon(AX + 1, y + 1, p.worn[s], 128);
                wear_bar(AX, y, p.armor_dur[s] as i32, ARMOR_DUR[t][s] as i32);
            }
            None => draw_tile(AX + 1, y + 1, armor_tile(s), (30, 30, 36)),
        }
        s += 1;
    }

    // The figure, in a dark box, with the off hand beside it.
    mc_slot(AX + 22, AY, 52, 4 * SLOT - 2);
    equip::draw_figure(AX + 32, AY + 4, p);
    slot(OFF_SX, OFF_SY);
    if p.offhand != AIR {
        draw_icon(OFF_SX + 1, OFF_SY + 1, p.offhand, 128);
        let c = unsafe { INV[p.offhand as usize] };
        if c > 1 {
            draw_count(OFF_SX, OFF_SY, c);
        }
    }

    // Armour: Java's pips (two defense points each), defense and toughness,
    // and what a ten point hit costs through it.
    let (pts, tough) = armor_points(p);
    let mut diamond = false;
    let mut k = 0;
    while k < 4 {
        diamond |= matches!(armor_piece(p.worn[k]), Some((1, _)));
        k += 1;
    }
    let fill = if diamond { (28, 150, 160) } else { (84, 88, 104) };
    let mut j = 0i16;
    while j < 10 {
        let x = WX + j * 8;
        rect(x, AY, 6, 7, 150, 150, 158);
        let full = (j as i32 + 1) * 2 <= pts;
        let half = !full && (j as i32) * 2 < pts;
        if full || half {
            rect(x, AY, if full { 6 } else { 3 }, 7, fill.0, fill.1, fill.2);
        }
        j += 1;
    }
    let mut nb = [0u8; 5];
    let mut nc = [0u8; 5];
    let mut nd = [0u8; 5];
    ui_text(font, WX, AY + 10, "DEFENSE", MC_INK);
    ui_text(font, WX + 64, AY + 10, number(pts as u16, &mut nb), MC_INK);
    ui_text(font, WX, AY + 20, "TOUGH", MC_INK);
    ui_text(font, WX + 64, AY + 20, number(tough as u16, &mut nc), MC_INK);
    ui_text(font, WX, AY + 30, "10 HIT", MC_INK);
    ui_text(font, WX + 64, AY + 30, number(armored(10, p) as u16, &mut nd), MC_INK);

    // The 2x2 crafting grid, its arrow and the output slot.
    let mut c = 0;
    while c < 4 {
        let (x, y) = (GX + (c % 2) as i16 * SLOT, GY + (c / 2) as i16 * SLOT);
        slot(x, y);
        if grid::kind(c) != AIR {
            draw_icon(x + 1, y + 1, grid::kind(c), 128);
            if grid::count(c) > 1 {
                draw_count(x, y, grid::count(c));
            }
        }
        c += 1;
    }
    ui_text(font, OUT_X - 18, OUT_Y + 4, ">", MC_INK);
    slot(OUT_X, OUT_Y);
    if let Some((out, qty)) = grid::output() {
        draw_icon(OUT_X + 1, OUT_Y + 1, out, 128);
        if qty > 1 {
            draw_count(OUT_X, OUT_Y, qty);
        }
    }

    // Weapon: the five you can swing, the one in hand framed, its numbers.
    let (wc, wt) = equip::weapon_of(p);
    ui_text(font, WX, WY - 10, "WEAPON", MC_INK);
    let mut w = 0;
    while w < 5 {
        let x = WX + w as i16 * SLOT;
        slot(x, WY);
        let class = equip::WEAPONS[w];
        let tier = tool_tier(p, class);
        if class == TOOL_NONE {
            rect(x + 6, WY + 6, 5, 5, 200, 150, 110); // a fist
        } else {
            let lum = if tier > 0 { 1 } else { 0 };
            let tt = tool_tint(tier.max(1));
            let c = if lum == 1 { tt } else { (40, 40, 44) };
            draw_tile(x + 1, WY + 1, tool_tile(class), c);
        }
        if class == wc {
            rect(x - 1, WY + 17, 19, 2, 0xFF, 0xE0, 0x40);
        }
        w += 1;
    }
    let (dmg, spd) = equip::stats(wc, wt);
    let mut b1 = [0u8; 8];
    let mut b2 = [0u8; 8];
    ui_text(font, WX, WY + 22, "DMG", MC_INK);
    ui_text(font, WX + 32, WY + 22, tenths((dmg + 5) / 10, &mut b1), MC_INK);
    ui_text(font, WX + 72, WY + 22, "SPEED", MC_INK);
    ui_text(font, WX + 120, WY + 22, tenths(spd, &mut b2), MC_INK);

    // The armour you carry.
    let mut n = 0;
    while n < STOCK_N {
        let x = SX0 + n as i16 * SLOT;
        slot(x, SY0);
        if n < ns {
            draw_icon(x + 1, SY0 + 1, st[n], 128);
            let c = unsafe { INV[st[n] as usize] };
            if c > 1 {
                draw_count(x, SY0, c);
            }
        }
        n += 1;
    }
    ui_text(font, SX0 + 8 * SLOT + 6, SY0 + 5, "IN YOUR PACK", MC_INK);

    // Info strip: the thing under the cursor and what X does there.
    mc_slot(16, 158, 288, 26);
    let mut l1: &str = "";
    let l2: &str;
    let mut nm = [0u8; 5];
    let mut nm2 = [0u8; 5];
    let mut shown = AIR;
    match z {
        Z_ARMOR => {
            if armor_piece(p.worn[i]).is_some() {
                shown = p.worn[i];
                l2 = "T TAKES IT OFF";
            } else {
                l1 = SLOT_NAME[i];
                l2 = "EMPTY. PLACE AN ARMOR PIECE HERE";
            }
        }
        Z_OFF => {
            shown = p.offhand;
            if shown == AIR {
                l1 = "OFF HAND";
            }
            l2 = "OFF HAND. L1+R1 SWAPS HANDS";
        }
        Z_WEAP => {
            let class = equip::WEAPONS[i];
            l1 = match class {
                TOOL_SWORD => "SWORD",
                TOOL_AXE => "AXE",
                TOOL_PICK => "PICKAXE",
                TOOL_SHOVEL => "SHOVEL",
                _ => "FIST",
            };
            l2 = if class == TOOL_NONE || tool_tier(p, class) > 0 {
                "X SWINGS THIS AT MOBS"
            } else {
                "YOU HAVE NONE. CRAFT ONE"
            };
        }
        Z_GRID => {
            shown = grid::kind(i);
            if shown == AIR {
                l1 = "CRAFTING GRID";
                l2 = "PLACE A STACK HERE";
            } else {
                l2 = "A STACK IN THE GRID";
            }
        }
        Z_OUT => {
            if let Some((out, _)) = grid::output() {
                shown = out;
                l2 = "WHAT THE GRID MAKES";
            } else {
                l1 = "CRAFTING RESULT";
                l2 = "FILL THE GRID, OR USE THE [] LIST";
            }
        }
        Z_STOCK => {
            if i < ns {
                shown = st[i];
                l2 = "ARMOR YOU CARRY. T WEARS IT";
            } else {
                l1 = "NO ARMOR HERE";
                l2 = "CRAFT IT AT A TABLE";
            }
        }
        _ => {
            shown = unsafe { HOTBAR[i] };
            l2 = "YOUR HOTBAR";
        }
    }
    if carry != AIR {
        draw_icon(20, 160, carry, 128);
        holding_line(font, 40, 161);
    } else if shown != AIR {
        draw_icon(20, 160, shown, 128);
        ui_text(font, 40, 161, block_name(shown), LABEL);
        if let Some((t, sl)) = armor_piece(shown) {
            let worn = z == Z_ARMOR;
            let mut x = 40 + (block_name(shown).len() as i16 + 1) * 8;
            let pts = ARMOR_POINTS[t][sl];
            ui_text(font, x, 161, "DEF", GREY);
            x += 32;
            ui_text(font, x, 161, number(pts as u16, &mut nm), GREY);
            if worn {
                x += 16;
                ui_text(font, x, 161, "USES", GREY);
                ui_text(font, x + 40, 161, number(p.armor_dur[i], &mut nm2), GREY);
            }
        }
    } else {
        ui_text(font, 20, 161, l1, LABEL);
    }
    let (note_t, note_s) = unsafe { (NOTE_T, NOTE) };
    let l2 = if carry != AIR { "O PUTS IT BACK" } else { l2 };
    ui_text(font, 40, 172, if note_t > 0 { note_s } else { l2 }, if note_t > 0 { (0xF0, 0xE0, 0x80) } else { GREY });

    // The prompt bar: what each button does for the slot under the cursor.
    let nx = prompt_bar(font, 188, 199, player_spot(z, i, p, ns, &st));
    hint_item(font, nx, 199, "L1R1", PS_KEY, "PAGE");

    // The hotbar row and the cursor.
    unsafe { OFFHAND_SHOWN = p.offhand };
    draw_hotbar(hud_tool(p, AIR));
    let (sx, sy) = match z {
        Z_ARMOR => (AX, AY + i as i16 * SLOT),
        Z_OFF => (OFF_SX, OFF_SY),
        Z_WEAP => (WX + i as i16 * SLOT, WY),
        Z_GRID => (GX + (i % 2) as i16 * SLOT, GY + (i / 2) as i16 * SLOT),
        Z_OUT => (OUT_X, OUT_Y),
        Z_STOCK => (SX0 + i as i16 * SLOT, SY0),
        _ => (HOTBAR_X0 + i as i16 * SLOT, HUD_HOTBAR_Y),
    };
    frame(sx, sy, CURSOR);
    draw_hand(sx, sy);
}

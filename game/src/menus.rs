//! The nested options tree: Java's pause menu and Options screens.
//!
//! Which pages and rows exist is menu_tree.rs (plain data, host-tested). This
//! file steers and draws it.
//!
//! One pad grammar everywhere, the inventory's and the chests': X
//! enters a sub-page or does the row, LEFT and RIGHT change a value, O goes up
//! one level (and closes the pause menu from its first page), START leaves the
//! whole tree. Each page remembers its cursor. A changed value lives in RAM
//! only until SAVE SETTINGS writes the card; nothing here saves on its own.
//!
//! Drawn through `controls::Ink`, so the pause menu (display list) and the
//! title screen (immediate draws) share one layout. Menu code: built for size
//! and kept out of the gameplay loop.

use crate::controls::Ink;
use crate::menu_tree::{self as tree, Kind, Page, PAGES};
use crate::*;

/// The page number the cheats screen (cheat.rs) publishes: it hangs off
/// GAMEPLAY and draws itself.
pub const CHEATS_PAGE: u32 = 6;

static mut PAGE: Page = Page::Pause;
/// The cursor of every page, kept between visits within one screen's tree
/// (the first page always opens on its first row).
static mut CUR: [u8; PAGES] = [0; PAGES];
/// The tree was opened from the title screen: no pause page, no cheats.
static mut AT_TITLE: bool = false;
/// The cheats screen's cursor (it draws and steers itself, see cheat.rs).
static mut CHEAT_CUR: u8 = 0;
/// SAVE AND QUIT started the save that is in flight.
static mut QUIT_PENDING: bool = false;
/// That save finished (or was waived): leave for the title screen.
static mut QUIT_READY: bool = false;

/// What the gameplay loop has to do after a frame of the pause menu.
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Out {
    None,
    /// Leave the menu: BACK TO GAME, O on the first page. On the title
    /// screen, back to the title.
    Close,
    /// Open the controls card.
    Controls,
    /// Open the cheats screen.
    Cheats,
    /// A world was loaded from the card: the player may stand in a portal.
    Loaded,
}

fn at_title() -> bool {
    unsafe { AT_TITLE }
}

fn rows(page: Page) -> &'static [tree::Row] {
    tree::rows(page, at_title())
}

fn clear_msg() {
    unsafe { OPT_MSG = "" };
}

/// Every cursor and flag back to a cold boot's (quitting to the title).
#[inline(never)]
#[optimize(size)]
pub fn reset_session() {
    unsafe {
        PAGE = Page::Pause;
        CUR = [0; PAGES];
        AT_TITLE = false;
        CHEAT_CUR = 0;
        QUIT_PENDING = false;
        QUIT_READY = false;
    }
    clear_msg();
}

/// The pause menu opens on its first page, first row (Java reopens it fresh).
#[inline(never)]
#[optimize(size)]
pub fn open_pause() {
    open(false);
}

/// The title screen's SETTINGS card: the OPTIONS page, first row.
#[inline(never)]
#[optimize(size)]
pub fn open_title() {
    open(true);
}

fn open(title: bool) {
    let first = tree::first(title);
    unsafe {
        if AT_TITLE != title {
            CUR = [0; PAGES]; // the other screen's tree: no cursor carries over
        }
        AT_TITLE = title;
        PAGE = first;
        CUR[first as usize] = 0;
        QUIT_PENDING = false;
        QUIT_READY = false;
    }
    clear_msg();
}

/// The cursor row of the open page.
pub fn cursor() -> usize {
    unsafe { CUR[PAGE as usize] as usize }
}

/// The open page's number, for the settings receipt.
pub fn page_id() -> u32 {
    unsafe { PAGE as u32 }
}

pub fn cheat_cursor() -> usize {
    unsafe { CHEAT_CUR as usize }
}

pub fn set_cheat_cursor(sel: usize) {
    unsafe { CHEAT_CUR = sel as u8 };
}

/// O: up one level. True when that leaves the tree (the first page).
#[inline(never)]
#[optimize(size)]
pub fn back() -> bool {
    match tree::parent(unsafe { PAGE }, at_title()) {
        Some(page) => {
            unsafe { PAGE = page };
            sfx::blip();
            clear_msg();
            false
        }
        None => true,
    }
}

fn toggle_tutorial() {
    unsafe {
        TUT_ENABLED = !TUT_ENABLED;
        if TUT_ENABLED && TUT_STEP >= TUT_DONE {
            tut_reset(); // re-enabling after a finish restarts it
        }
    }
}

/// One frame of cursor, values and confirm on the open page. Returns the row
/// that X chose when it needs the caller's context; value rows and the
/// tutorial switch are handled here.
fn step(pad: ButtonState, prev: ButtonState, up: bool, down: bool) -> Option<Kind> {
    let page = unsafe { PAGE };
    let rows = rows(page);
    let n = rows.len();
    let mut cur = (unsafe { CUR[page as usize] } as usize).min(n - 1);
    if up || down {
        cur = if up { (cur + n - 1) % n } else { (cur + 1) % n };
        clear_msg();
        sfx::blip();
    }
    unsafe { CUR[page as usize] = cur as u8 };
    let kind = rows[cur].kind;
    let dir = if pad.pressed_since(prev, button::LEFT) {
        -1
    } else if pad.pressed_since(prev, button::RIGHT) {
        1
    } else {
        0
    };
    if dir != 0 {
        match kind {
            Kind::Set(i) => {
                setting_adjust(i, dir);
                clear_msg();
                sfx::blip();
            }
            Kind::Tutorial => {
                toggle_tutorial();
                sfx::blip();
            }
            _ => {}
        }
    }
    if !pad.pressed_since(prev, button::CROSS) {
        return None;
    }
    // X on a two-state row flips it, as it always did for the tutorial.
    match kind {
        Kind::Set(tree::S_INVERT_Y) => {
            setting_adjust(tree::S_INVERT_Y, 1);
            clear_msg();
            sfx::blip();
            None
        }
        Kind::Set(_) => None,
        Kind::Tutorial => {
            toggle_tutorial();
            sfx::blip();
            None
        }
        k => Some(k),
    }
}

/// One frame of the open page. `world` is the player and the world clock in a
/// game, None on the title screen. The caller has already taken O and START.
#[inline(never)]
#[optimize(size)]
pub fn input(
    pad: ButtonState,
    prev: ButtonState,
    up: bool,
    down: bool,
    world: Option<(&mut Player, &mut u32)>,
    fb: &mut FrameBuffer,
    font: &FontAtlas,
) -> Out {
    let Some(kind) = step(pad, prev, up, down) else {
        return Out::None;
    };
    sfx::confirm();
    match kind {
        Kind::Go(page) => {
            unsafe { PAGE = page };
            clear_msg();
            Out::None
        }
        Kind::Done => {
            if back() {
                Out::Close
            } else {
                Out::None
            }
        }
        Kind::Resume => Out::Close,
        Kind::Controls => {
            controls::open();
            clear_msg();
            Out::Controls
        }
        Kind::Cheats => {
            clear_msg();
            Out::Cheats
        }
        Kind::SaveSettings => {
            save_shared_settings(fb, font);
            unsafe {
                OPT_MSG = if SETTINGS_FORMAT_OK {
                    "BLANK CARD: X AGAIN TO FORMAT"
                } else {
                    ""
                };
            }
            Out::None
        }
        Kind::SaveWorld => {
            if let Some((player, day)) = world {
                save::begin(player, *day);
                unsafe { OPT_MSG = "SAVING - DO NOT REMOVE THE CARD" };
            }
            Out::None
        }
        Kind::SaveQuit => {
            // A failed save keeps the player in the menu with the reason
            // (save_step shows it); X again tries the save again.
            if let Some((player, day)) = world {
                save::begin(player, *day);
                unsafe {
                    QUIT_PENDING = true;
                    OPT_MSG = "SAVING - DO NOT REMOVE THE CARD";
                }
            }
            Out::None
        }
        Kind::LoadWorld => {
            let Some((player, day)) = world else {
                return Out::None;
            };
            if load_game(player, day, fb, font) {
                unsafe { OPT_MSG = "LOADED" };
                Out::Loaded
            } else {
                unsafe {
                    OPT_MSG = if save::selftest() {
                        "NO SAVE ON THIS CARD"
                    } else {
                        "NO MEMORY CARD IN SLOT 1"
                    };
                }
                Out::None
            }
        }
        // Set rows and the tutorial never come back from `step`.
        Kind::Set(_) | Kind::Tutorial => Out::None,
    }
}

/// SAVE AND QUIT has finished its save and the game should go back to the
/// title screen.
pub fn quit_ready() -> bool {
    unsafe { QUIT_READY }
}

/// The title screen is up again: the quit is over.
pub fn quit_done() {
    unsafe {
        QUIT_READY = false;
        QUIT_PENDING = false;
    }
}

/// A save finished. If SAVE AND QUIT started it, a good save leaves for the
/// title. Any other outcome stays in the menu: a blank card asks (X again
/// formats, saves, and leaves), a failure shows its reason (save_step) and X
/// again tries again. Java does not quit on a failed save either.
#[inline(never)]
#[optimize(size)]
pub fn quit_after(outcome: &save::Outcome) {
    unsafe {
        if QUIT_PENDING && matches!(outcome, save::Outcome::Saved) {
            QUIT_READY = true;
        }
        QUIT_PENDING = false;
    }
}

/// A button hint on the panel: pill, then its action in dark ink.
fn hint(
    ink: &dyn Ink,
    font: &FontAtlas,
    x: i16,
    key: &str,
    tint: (u8, u8, u8),
    action: &str,
) -> i16 {
    let nx = ink.pill(font, x, MENU_HINT_Y, key, tint) + 3;
    ink.text(font, nx, MENU_HINT_Y, action, MC_INK);
    nx + action.len() as i16 * 8 + 8
}

/// Draw the open page.
#[inline(never)]
#[optimize(size)]
pub fn draw(ink: &dyn Ink, font: &FontAtlas) {
    let page = unsafe { PAGE };
    let rows = rows(page);
    let sel = cursor();
    ink.frame(font, tree::title_of(page, at_title()));
    let mut hx = hint(ink, font, MENU_TEXT_X, "X", PS_CROSS, "SELECT");
    if tree::adjusts(page, at_title()) {
        hx = hint(ink, font, hx, "< >", PS_KEY, "ADJUST");
    }
    let closes = page == Page::Pause;
    hint(
        ink,
        font,
        hx,
        "O",
        PS_CIRCLE,
        if closes { "CLOSE" } else { "BACK" },
    );
    let mut air = 0i16;
    let mut j = 0;
    while j < rows.len() {
        let r = &rows[j];
        if r.gap {
            air = 8;
        }
        let y = MENU_ROWS_Y + (j * MENU_ROW_H) as i16 + air;
        ink.button(y - 3, j == sel);
        let color = if j == sel { MC_LABEL_SEL } else { MC_LABEL };
        ink.text(font, MENU_TEXT_X, y, r.label, color);
        let mut buf = [0u8; 12];
        let value: Option<(usize, (u8, u8, u8))> = match r.kind {
            Kind::Set(i) => Some((setting_value(i, &mut buf), (0x70, 0xe0, 0x70))),
            Kind::SaveSettings => Some((setting_value(tree::S_SAVE, &mut buf), (0x70, 0xe0, 0x70))),
            Kind::Tutorial => {
                let (s, tint) = if unsafe { TUT_ENABLED } {
                    (&b"ON"[..], (0x70, 0xe0, 0x70))
                } else {
                    (&b"OFF"[..], MC_LABEL_OFF)
                };
                buf[..s.len()].copy_from_slice(s);
                Some((s.len(), tint))
            }
            _ => None,
        };
        if let Some((k, tint)) = value {
            let txt = unsafe { core::str::from_utf8_unchecked(&buf[..k]) };
            ink.text(font, 278 - k as i16 * 8, y, txt, tint);
        }
        j += 1;
    }
    // One line under the list: the outcome of the last card operation, else
    // the rule that a changed value is only kept by SAVE SETTINGS.
    let msg = unsafe { OPT_MSG };
    let line = if !msg.is_empty() {
        msg
    } else if tree::holds_settings(page) {
        "CHANGES KEEP ON SAVE SETTINGS"
    } else {
        ""
    };
    ink.text(
        font,
        (SCREEN_W as i16 - line.len() as i16 * 8) / 2,
        168,
        line,
        MC_INK,
    );
}

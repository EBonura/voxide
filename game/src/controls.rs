//! The CONTROLS card: every button, in two pages (in the world, in menus), in
//! the menus' own panel, pill and font. One layout, drawn two ways: into the
//! frame's display list from the pause menu (`Listed`), and straight to the
//! GPU from the main menu, which draws after the display list has gone out
//! (`Now`). Shown once on the first PLAY, from the main menu, and from the
//! pause menu at any time.

use crate::*;

/// How a card is painted: the pause menu's display list, or the main menu's
/// immediate draws. Same shapes and colours either way.
pub trait Ink {
    fn panel(&self, x: i16, y: i16, w: i16, h: i16);
    fn text(&self, font: &FontAtlas, x: i16, y: i16, s: &str, c: (u8, u8, u8));
    /// A button pill; returns the x just past it.
    fn pill(&self, font: &FontAtlas, x: i16, y: i16, key: &str, tint: (u8, u8, u8)) -> i16;
}

/// Into the frame's display list (the pause menu).
pub struct Listed;
/// Straight to the GPU (the main menu).
pub struct Now;

impl Ink for Listed {
    fn panel(&self, x: i16, y: i16, w: i16, h: i16) {
        dim_screen();
        inv::panel(x, y, w, h);
    }
    fn text(&self, font: &FontAtlas, x: i16, y: i16, s: &str, c: (u8, u8, u8)) {
        ui_text(font, x, y, s, c);
    }
    fn pill(&self, font: &FontAtlas, x: i16, y: i16, key: &str, tint: (u8, u8, u8)) -> i16 {
        ui_badge(font, x, y, key, tint)
    }
}

impl Ink for Now {
    fn panel(&self, x: i16, y: i16, w: i16, h: i16) {
        // panel_now outlines outside its box; inv::panel inside it.
        panel_now(x + 2, y + 2, w - 4, h - 4);
    }
    fn text(&self, font: &FontAtlas, x: i16, y: i16, s: &str, c: (u8, u8, u8)) {
        font.draw_text(x, y, s, c);
    }
    fn pill(&self, font: &FontAtlas, x: i16, y: i16, key: &str, tint: (u8, u8, u8)) -> i16 {
        badge_now(font, x, y, key, tint)
    }
}

const PAGES: usize = 2;
static mut PAGE: usize = 0;

/// Open on the first page.
#[inline(never)]
#[optimize(size)]
pub fn open() {
    unsafe { PAGE = 0 };
}

/// One frame of the card. Returns true when it should close: CIRCLE or START
/// any time, X on the last page. X otherwise, RIGHT and R1 turn the page on,
/// LEFT and L1 back.
#[inline(never)]
#[optimize(size)]
pub fn input(pad: ButtonState, previous: ButtonState) -> bool {
    let pressed = |b: u16| pad.pressed_since(previous, b);
    if pressed(button::CIRCLE) || pressed(button::START) {
        return true;
    }
    let page = unsafe { PAGE };
    if pressed(button::CROSS) && page + 1 == PAGES {
        return true;
    }
    if pressed(button::CROSS) || pressed(button::RIGHT) || pressed(button::R1) {
        unsafe { PAGE = (page + 1) % PAGES };
        sfx::blip();
    } else if pressed(button::LEFT) || pressed(button::L1) {
        unsafe { PAGE = (page + PAGES - 1) % PAGES };
        sfx::blip();
    }
    false
}

/// A row: the pill(s) and what they do. An empty key makes a continuation
/// line under the row above, "=" a heading.
type Row = (&'static str, (u8, u8, u8), &'static str);

const NONE: (u8, u8, u8) = PS_KEY;
const WORLD: [Row; 12] = [
    ("LS", NONE, "MOVE (THE D-PAD TOO)"),
    ("RS", NONE, "LOOK AROUND"),
    ("X", PS_CROSS, "JUMP. SWIM UP"),
    ("O", PS_CIRCLE, "HOLD TO SNEAK"),
    ("L3", NONE, "SPRINT. OR PUSH FORWARD TWICE"),
    ("R2", NONE, "HOLD TO MINE. TAP TO HIT"),
    ("L2", NONE, "USE, PLACE, OPEN, EAT"),
    ("L1R1", NONE, "PICK THE HOTBAR SLOT"),
    ("L1+R1", NONE, "SWAP HANDS"),
    ("[]", PS_SQUARE, "CRAFTING"),
    ("T", PS_TRIANGLE, "INVENTORY"),
    ("START", NONE, "PAUSE: SAVE, OPTIONS"),
];

const MENUS: [Row; 12] = [
    ("X", PS_CROSS, "TAKE A STACK, THEN PLACE IT"),
    ("", NONE, "IT SWAPS WITH WHAT IS THERE"),
    ("[]", PS_SQUARE, "TAKE HALF, THEN PLACE ONE"),
    ("T", PS_TRIANGLE, "QUICK MOVE: SEND IT ACROSS"),
    ("O", PS_CIRCLE, "PUT IT BACK. THEN CLOSE"),
    ("L1R1", NONE, "NEXT AND PREVIOUS TAB"),
    ("L2R2", NONE, "SCROLL LONG LISTS"),
    ("SEL", NONE, "SHOW ALL, OR WHAT YOU OWN"),
    ("=", NONE, "IN THE CRAFTING LIST"),
    ("X", PS_CROSS, "CRAFT ONE. HOLD FOR MORE"),
    ("T", PS_TRIANGLE, "CRAFT AS MANY AS YOU CAN"),
    ("R2", NONE, "LAY IT IN THE 2X2 GRID"),
];

const TEXT_X: i16 = 74;
/// A continuation line: dimmer than the rows, still readable on the panel.
const DIM: (u8, u8, u8) = (0x3C, 0x3C, 0x48);

/// Draw the card: panel, title, the page's rows and the buttons that turn
/// it. `y` of a row is its text line.
#[inline(never)]
#[optimize(size)]
pub fn draw(ink: &dyn Ink, font: &FontAtlas) {
    let page = unsafe { PAGE };
    ink.panel(8, 3, 304, 207);
    let (title, rows): (&str, &[Row]) = if page == 0 {
        ("CONTROLS: IN THE WORLD", &WORLD)
    } else {
        ("CONTROLS: IN MENUS", &MENUS)
    };
    ink.text(
        font,
        (SCREEN_W as i16 - title.len() as i16 * 8) / 2,
        10,
        title,
        MC_INK,
    );
    let mut y = 28i16;
    let mut r = 0;
    while r < rows.len() {
        let (key, tint, what) = rows[r];
        if key == "=" {
            y += 3;
            ink.text(font, 16, y, what, MC_INK);
        } else if key.is_empty() {
            ink.text(font, TEXT_X, y, what, DIM);
        } else {
            ink.pill(font, 16, y, key, tint);
            ink.text(font, TEXT_X, y, what, MC_INK);
        }
        y += 12;
        r += 1;
    }
    // The buttons that move through the card.
    let by = 196;
    let x = ink.pill(font, 16, by, "X", PS_CROSS) + 3;
    let next = if page + 1 == PAGES { "DONE" } else { "MORE" };
    ink.text(font, x, by, next, MC_INK);
    let x = x + next.len() as i16 * 8 + 8;
    let x = ink.pill(font, x, by, "<>", PS_KEY) + 3;
    ink.text(font, x, by, "PAGE", MC_INK);
    let x = x + 40;
    let x = ink.pill(font, x, by, "O", PS_CIRCLE) + 3;
    ink.text(font, x, by, "CLOSE", MC_INK);
    let mut nb = [0u8; 5];
    let mut tb = [0u8; 5];
    let pg = inv::number(page as u16 + 1, &mut nb);
    let tot = inv::number(PAGES as u16, &mut tb);
    let px = 312 - 8 - (pg.len() + tot.len() + 1) as i16 * 8;
    ink.text(font, px, by, pg, MC_INK);
    ink.text(font, px + pg.len() as i16 * 8, by, "/", MC_INK);
    ink.text(font, px + (pg.len() as i16 + 1) * 8, by, tot, MC_INK);
}

//! The 2x2 crafting grid of the player page, Java's survival grid: four cells
//! that hold stacks, an output slot that shows what the shape makes, and
//! shaped recipes matched wherever they sit in the grid. What is in a cell has
//! left INV, so the counts you see everywhere else stay true; closing the
//! screen puts every cell back (`return_all`).
//!
//! The recipes are the ones Java lets a 2x2 grid make and VoXide has: planks,
//! sticks, the crafting table, torches and bone meal. Everything bigger needs
//! the table, as in Java. The SQUARE pocket list plays the recipe book: it
//! fills the grid with a recipe's shape from your inventory (`fill_recipe`).

use crate::*;

/// Java's stack limit for a cell.
pub const CELL_MAX: u16 = 64;

/// What each cell holds, row-major (0 top left, 1 top right, 2 bottom left,
/// 3 bottom right), and how many.
static mut KIND: [u8; 4] = [AIR; 4];
static mut COUNT: [u16; 4] = [0; 4];

/// A shaped recipe: a w x h block of cells (row-major, AIR = must be empty
/// inside the box), and what it makes.
struct Shape {
    w: u8,
    h: u8,
    cells: [u8; 4],
    out: u8,
    qty: u16,
}

const SHAPES: [Shape; 5] = [
    // One log, anywhere: four planks.
    Shape { w: 1, h: 1, cells: [WOOD, AIR, AIR, AIR], out: PLANK, qty: 4 },
    // Two planks one above the other: four sticks.
    Shape { w: 1, h: 2, cells: [PLANK, PLANK, AIR, AIR], out: STICK, qty: 4 },
    // Four planks: the crafting table.
    Shape { w: 2, h: 2, cells: [PLANK, PLANK, PLANK, PLANK], out: CRAFT_TABLE, qty: 1 },
    // Coal over a stick: four torches.
    Shape { w: 1, h: 2, cells: [COAL_ORE, STICK, AIR, AIR], out: TORCH, qty: 4 },
    // A bone: three bone meal.
    Shape { w: 1, h: 1, cells: [BONE, AIR, AIR, AIR], out: BONEMEAL, qty: 3 },
];

pub fn kind(c: usize) -> u8 {
    unsafe { KIND[c] }
}

pub fn count(c: usize) -> u16 {
    unsafe { COUNT[c] }
}

/// The recipe the cells form right now, as (shape index).
fn matched() -> Option<usize> {
    let (k, n) = unsafe { (KIND, COUNT) };
    // The box around the filled cells.
    let (mut x0, mut y0, mut x1, mut y1) = (2usize, 2usize, 0usize, 0usize);
    let mut any = false;
    let mut c = 0;
    while c < 4 {
        if k[c] != AIR && n[c] > 0 {
            let (x, y) = (c % 2, c / 2);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            any = true;
        }
        c += 1;
    }
    if !any {
        return None;
    }
    let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
    let mut s = 0;
    while s < SHAPES.len() {
        let sh = &SHAPES[s];
        if sh.w as usize == w && sh.h as usize == h {
            let mut ok = true;
            let mut yy = 0;
            while yy < h {
                let mut xx = 0;
                while xx < w {
                    let cell = (y0 + yy) * 2 + x0 + xx;
                    let want = sh.cells[yy * w + xx];
                    let have = if n[cell] > 0 { k[cell] } else { AIR };
                    ok &= want == have;
                    xx += 1;
                }
                yy += 1;
            }
            if ok {
                return Some(s);
            }
        }
        s += 1;
    }
    None
}

/// What the output slot shows: the kind and how many one craft makes.
pub fn output() -> Option<(u8, u16)> {
    matched().map(|s| (SHAPES[s].out, SHAPES[s].qty))
}

/// Whether the pocket grid can make this recipe output at all.
pub fn pocket_craftable(out: u8) -> bool {
    let mut s = 0;
    while s < SHAPES.len() {
        if SHAPES[s].out == out {
            return true;
        }
        s += 1;
    }
    false
}

/// Craft once: one item leaves each filled cell and the result goes to the
/// inventory. False when the cells make nothing.
pub fn craft_once() -> bool {
    let Some(s) = matched() else {
        return false;
    };
    unsafe {
        let mut c = 0;
        while c < 4 {
            if COUNT[c] > 0 {
                COUNT[c] -= 1;
                if COUNT[c] == 0 {
                    KIND[c] = AIR;
                }
            }
            c += 1;
        }
    }
    inv_give(SHAPES[s].out, SHAPES[s].qty);
    true
}

/// Craft until the cells run out (Java's shift-click on the output).
pub fn craft_all() -> u16 {
    let mut n = 0;
    while n < 64 && craft_once() {
        n += 1;
    }
    n
}

/// Put cell `c` back into the inventory.
pub fn clear_cell(c: usize) {
    unsafe {
        if KIND[c] != AIR && COUNT[c] > 0 {
            inv_give(KIND[c], COUNT[c]);
        }
        KIND[c] = AIR;
        COUNT[c] = 0;
    }
}

/// Every cell back into the inventory: the screen is closing.
pub fn return_all() {
    let mut c = 0;
    while c < 4 {
        if unsafe { COUNT[c] } > 0 || unsafe { KIND[c] } != AIR {
            clear_cell(c);
        }
        c += 1;
    }
}

/// Move up to `n` of `kind` from the inventory into cell `c`: into an empty
/// cell, or onto the same kind up to the stack limit. A different kind in the
/// cell goes back to the inventory first. Returns how many moved.
pub fn put(c: usize, item: u8, n: u16) -> u16 {
    unsafe {
        if KIND[c] != AIR && KIND[c] != item {
            clear_cell(c);
        }
        let room = CELL_MAX - if KIND[c] == item { COUNT[c] } else { 0 };
        let m = n.min(room).min(INV[item as usize]);
        if m == 0 {
            return 0;
        }
        INV[item as usize] -= m;
        KIND[c] = item;
        COUNT[c] += m;
        m
    }
}

/// Take half the stack (rounded up) out of cell `c`, back to the inventory;
/// returns (kind, how many) for the cursor to hold.
pub fn take_half(c: usize) -> (u8, u16) {
    unsafe {
        let (k, n) = (KIND[c], COUNT[c]);
        if k == AIR || n == 0 {
            return (AIR, 0);
        }
        let h = (n + 1) / 2;
        COUNT[c] -= h;
        if COUNT[c] == 0 {
            KIND[c] = AIR;
        }
        inv_give(k, h);
        (k, h)
    }
}

/// Take the whole stack out of cell `c`, back to the inventory.
pub fn take_all(c: usize) -> (u8, u16) {
    let (k, n) = unsafe { (KIND[c], COUNT[c]) };
    clear_cell(c);
    (k, n)
}

/// The recipe book: lay the shape that makes `out` into the grid, one item a
/// cell, from the inventory. The grid is emptied first. False when the
/// inventory cannot pay for it or the grid has no such recipe.
pub fn fill_recipe(out: u8) -> bool {
    let mut s = 0;
    while s < SHAPES.len() && SHAPES[s].out != out {
        s += 1;
    }
    if s == SHAPES.len() {
        return false;
    }
    return_all();
    let sh = &SHAPES[s];
    // Can the inventory pay?
    let mut need = [0u16; BLOCK_KINDS];
    let mut i = 0;
    while i < (sh.w * sh.h) as usize {
        if sh.cells[i] != AIR {
            need[sh.cells[i] as usize] += 1;
        }
        i += 1;
    }
    let mut k = 0;
    while k < BLOCK_KINDS {
        if need[k] > unsafe { INV[k] } {
            return false;
        }
        k += 1;
    }
    let (w, h) = (sh.w as usize, sh.h as usize);
    let mut yy = 0;
    while yy < h {
        let mut xx = 0;
        while xx < w {
            let item = sh.cells[yy * w + xx];
            if item != AIR {
                put(yy * 2 + xx, item, 1);
            }
            xx += 1;
        }
        yy += 1;
    }
    true
}

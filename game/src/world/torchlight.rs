//! The light of placed torches, worked out once and remembered (light.rs has the maths).
//!
//! A torch's light is a flood through the cells that let light pass (air, water, plants, shaped
//! blocks, leaves), three, six and nine steps out. Walking the blocks for every torch on every
//! remesh would cost about 70,000 cycles a torch each time, so the flood is kept: one entry per
//! torch, a [`light::FloodBits`] of about 200 bytes, read back by the chunks it reaches into
//! without a look at their blocks. An entry is worked out again only when it must be:
//!
//! * a torch has no entry until a chunk mesh asks for it (`ensure`): its first flood;
//! * a block changes within its reach so that light passes it (or no longer does), `block_changed`
//!   and `column_changed`: the entry is marked, and `tick` works one marked entry out a frame,
//!   nearest the player first, so no frame pays for more than one flood (about 55,000 cycles);
//! * a chunk comes into the loaded ring near it, `chunk_loaded`: cells of a chunk that is not
//!   loaded count as stone, so the light stopped at its border and now goes on.
//!
//! When a flood comes out different, the chunks it reaches into are marked for a remesh.
//! A chunk meshed by the stream path works out the entries it needs before it reads them, so it
//! never shows a stale pool; one meshed by an edit reads what there is and is meshed again when
//! the new flood lands.

use super::{
    floor_div, rle, slot, BCLASS, CHUNKS, CLS_LIGHT, CW, DIM, EDIT_MESH_S, EDIT_PENDING_FULL,
    MAX_FACES, MESH_FACES, MESH_S, PLAYER_BX, PLAYER_BZ,
};
use crate::light;
use crate::TORCH;

/// Torches whose light is kept. A torch is kept while a chunk it can light is meshed; when the
/// table is full the torch farthest from the player gives its place to a nearer one, and a torch
/// without a place does not light.
pub const MAX_FLOODS: usize = 32;

const FREE: u8 = 0;
/// Has a place and a block it must be worked out from again.
const DIRTY: u8 = 1;
/// Worked out, and nothing within reach has changed since.
const READY: u8 = 2;

#[derive(Copy, Clone)]
struct Entry {
    x: i32,
    z: i32,
    y: u8,
    dim: u8,
    state: u8,
    bits: light::FloodBits,
}

const NO_ENTRY: Entry = Entry {
    x: 0,
    z: 0,
    y: 0,
    dim: 0,
    state: FREE,
    bits: [0; light::FLOOD_WORDS],
};

static mut ENTRIES: [Entry; MAX_FLOODS] = [NO_ENTRY; MAX_FLOODS];
/// Words of working space a flood takes: the cells that let light pass and the cells lit so far.
/// They are not on the stack (the mesher's stream tick runs on a scratchpad stack of 1 KB) and
/// not in a static of their own (3.5 KB of the little RAM there is): they are the first words of
/// MESH_FACES, which holds nothing while the lighting of a mesh is being set up, and which
/// `tick` leaves alone while a mesh is in flight.
const SCRATCH_WORDS: usize = 2 * 21 * 21;
const _: () = assert!(MAX_FACES >= SCRATCH_WORDS);
/// Entries in use, so a world without a torch pays one compare for any block change.
static mut USED: usize = 0;
/// Some entry is DIRTY.
static mut PENDING: bool = false;

/// Forget every torch (a new world, a load, a change of dimension).
pub fn reset() {
    unsafe {
        let mut i = 0;
        while i < MAX_FLOODS {
            ENTRIES[i].state = FREE;
            i += 1;
        }
        USED = 0;
        PENDING = false;
    }
}

/// Is there any torch light to keep up to date?
#[inline(always)]
pub fn active() -> bool {
    unsafe { USED != 0 }
}

/// Is an entry waiting to be worked out again?
#[inline(always)]
pub fn pending() -> bool {
    unsafe { PENDING }
}

/// Horizontal taxicab distance from the player.
fn from_player(x: i32, z: i32) -> i32 {
    unsafe { (x - PLAYER_BX).abs() + (z - PLAYER_BZ).abs() }
}

/// The entry of the torch at the world cell, if it has one.
fn find(dim: u8, x: i32, y: i32, z: i32) -> Option<usize> {
    unsafe {
        let mut i = 0;
        while i < MAX_FLOODS {
            let e = &ENTRIES[i];
            if e.state != FREE && e.x == x && e.z == z && e.y as i32 == y && e.dim == dim {
                return Some(i);
            }
            i += 1;
        }
        None
    }
}

/// The entry of the torch at the world cell, making one (DIRTY, with no light) when it has none.
/// `None` when the table is full of torches nearer the player.
pub fn ensure(x: i32, y: i32, z: i32) -> Option<usize> {
    let dim = unsafe { DIM };
    if let Some(i) = find(dim, x, y, z) {
        return Some(i);
    }
    unsafe {
        let mut at = MAX_FLOODS;
        let mut worst = (-1, 0);
        let mut i = 0;
        while i < MAX_FLOODS {
            if ENTRIES[i].state == FREE {
                at = i;
                break;
            }
            let far = from_player(ENTRIES[i].x, ENTRIES[i].z);
            if far > worst.0 {
                worst = (far, i);
            }
            i += 1;
        }
        if at == MAX_FLOODS {
            // Full: the farthest torch makes way for a nearer one.
            if worst.0 <= from_player(x, z) {
                return None;
            }
            at = worst.1;
        } else {
            USED += 1;
        }
        let e = &mut ENTRIES[at];
        e.x = x;
        e.z = z;
        e.y = y as u8;
        e.dim = dim;
        e.state = DIRTY;
        e.bits = [0; light::FLOOD_WORDS];
        PENDING = true;
        Some(at)
    }
}

/// Is the entry's light up to date?
#[inline(always)]
pub fn ready(i: usize) -> bool {
    unsafe { ENTRIES[i].state == READY }
}

/// Or the entry's light into the tier maps of the chunk at (`cx`, `cz`). Returns whether any cell
/// was lit.
pub fn apply(cols: &mut light::Cols, i: usize, cx: i32, cz: i32) -> bool {
    unsafe {
        let e = &ENTRIES[i];
        light::apply(cols, &e.bits, e.x - cx * CW, e.y as i32, e.z - cz * CW)
    }
}

/// The tier (0 to 3) the torches' light gives the world cell, for the faces the greedy mesh does
/// not carry. Only worked-out entries count.
pub fn tier_at(x: i32, y: i32, z: i32) -> u16 {
    let mut best = 0;
    unsafe {
        let dim = DIM;
        let mut i = 0;
        while i < MAX_FLOODS {
            let e = &ENTRIES[i];
            if e.state == READY && e.dim == dim {
                let (dx, dy, dz) = (x - e.x, y - e.y as i32, z - e.z);
                if dx.abs() + dy.abs() + dz.abs() <= light::FLOOD_R {
                    best = best.max(light::flood_tier(&e.bits, dx, dy, dz));
                }
            }
            i += 1;
        }
    }
    best
}

/// A cell changed from `old` to `new`. Entries within reach whose light that changes are marked
/// to be worked out again; the entry of a torch that is gone is dropped.
pub fn block_changed(x: i32, y: i32, z: i32, old: u8, new: u8) {
    let torch_gone = crate::kind_of(old) == TORCH && crate::kind_of(new) != TORCH;
    let passes = |b: u8| unsafe { BCLASS[b as usize] } & CLS_LIGHT != 0;
    if !torch_gone && passes(old) == passes(new) {
        return;
    }
    unsafe {
        let dim = DIM;
        let mut i = 0;
        while i < MAX_FLOODS {
            let e = &mut ENTRIES[i];
            if e.state != FREE && e.dim == dim {
                let (dx, dy, dz) = (x - e.x, y - e.y as i32, z - e.z);
                if torch_gone && dx == 0 && dy == 0 && dz == 0 {
                    e.state = FREE;
                    USED -= 1;
                } else if passes(old) != passes(new)
                    && dx.abs() + dy.abs() + dz.abs() <= light::FLOOD_R
                    && e.state == READY
                {
                    e.state = DIRTY;
                    PENDING = true;
                }
            }
            i += 1;
        }
    }
}

/// Cells of the column (`x`, `z`) from `y0` to `y1` were cleared (a blast): the entries within
/// reach of any of them are marked.
pub fn column_changed(x: i32, z: i32, y0: i32, y1: i32) {
    unsafe {
        let dim = DIM;
        let mut i = 0;
        while i < MAX_FLOODS {
            let e = &mut ENTRIES[i];
            if e.state == READY && e.dim == dim {
                let flat = (x - e.x).abs() + (z - e.z).abs();
                let room = light::FLOOD_R - flat;
                let y = e.y as i32;
                if room >= 0 && y0 - room <= y && y <= y1 + room {
                    e.state = DIRTY;
                    PENDING = true;
                }
            }
            i += 1;
        }
    }
}

/// The chunk (`cx`, `cz`) has joined the loaded ring: entries within reach of it are marked, for
/// the cells they took for stone and are now blocks.
pub fn chunk_loaded(cx: i32, cz: i32) {
    unsafe {
        let dim = DIM;
        let (x0, z0) = (cx * CW, cz * CW);
        let mut i = 0;
        while i < MAX_FLOODS {
            let e = &mut ENTRIES[i];
            if e.state == READY && e.dim == dim && reaches_box(e.x, e.z, x0, z0) {
                e.state = DIRTY;
                PENDING = true;
            }
            i += 1;
        }
    }
}

/// Does light at (`x`, `z`) reach the chunk whose corner is (`x0`, `z0`)?
fn reaches_box(x: i32, z: i32, x0: i32, z0: i32) -> bool {
    light::reaches_chunk(x - x0, z - z0, &light::TORCH_REACH)
}

/// The cells of the column at the world position (`wx`, `wz`) from `y0`, 19 of them, that let
/// light pass, bit `y - y0`. A column that is not loaded is stone.
fn column_open(wx: i32, wz: i32, y0: i32) -> u32 {
    let (cx, cz) = (floor_div(wx, CW), floor_div(wz, CW));
    let s = slot(cx, cz);
    unsafe {
        if CHUNKS[s].loaded && CHUNKS[s].cx == cx && CHUNKS[s].cz == cz {
            let col = ((wz - cz * CW) * CW + (wx - cx * CW)) as usize;
            rle::col_open(s, col, y0, y0 + 2 * light::FLOOD_R + 1)
        } else {
            0
        }
    }
}

/// Work entry `i` out from the blocks, and mark the chunks it reaches into for a remesh when the
/// light came out different (all but the one at `except`, which is being meshed and reads it next).
/// Returns false when there is no torch to flood from any more: the entry is dropped.
pub fn reflow(i: usize, except: (i32, i32)) -> bool {
    #[cfg(feature = "torch-lab")]
    let probe = crate::torchlab::clock();
    let done = reflow_inner(i, except);
    #[cfg(feature = "torch-lab")]
    crate::torchlab::note(2, probe);
    done
}

fn reflow_inner(i: usize, except: (i32, i32)) -> bool {
    unsafe {
        let (x, y, z) = (ENTRIES[i].x, ENTRIES[i].y as i32, ENTRIES[i].z);
        // A torch that is gone (a blast took it) has nothing to flood from.
        if crate::kind_of(super::get(x, y, z)) != TORCH {
            ENTRIES[i].state = FREE;
            USED -= 1;
            return false;
        }
        // SAFETY: MESH_FACES is u32 and long enough (asserted); no mesh is writing faces now (see
        // SCRATCH_WORDS), and the two halves do not overlap.
        let base = core::ptr::addr_of_mut!(MESH_FACES) as *mut u32;
        let open = &mut *(base as *mut light::OpenMap);
        let work = &mut *(base.add(21 * 21) as *mut light::Work);
        *open = [0; 21 * 21];
        let r = light::FLOOD_R;
        let mut dx = -r;
        while dx <= r {
            let m = r - dx.abs();
            let mut dz = -m;
            while dz <= m {
                open[light::open_index(dx, dz)] = column_open(x + dx, z + dz, y - r);
                dz += 1;
            }
            dx += 1;
        }
        // The torch's own cell lets its light out whatever it holds.
        open[light::open_index(0, 0)] |= 1 << r;
        let mut bits: light::FloodBits = [0; light::FLOOD_WORDS];
        light::reflow(open, work, &mut bits);
        let e = &mut ENTRIES[i];
        let changed = e.bits != bits;
        e.bits = bits;
        e.state = READY;
        let mut any = false;
        let mut k = 0;
        while k < MAX_FLOODS {
            any |= ENTRIES[k].state == DIRTY;
            k += 1;
        }
        PENDING = any;
        if changed {
            remesh_reach(x, z, except);
        }
        true
    }
}

/// Mark the chunks within reach of the torch column (`x`, `z`) for a remesh, but `except`.
fn remesh_reach(x: i32, z: i32, except: (i32, i32)) {
    let (cx, cz) = (floor_div(x, CW), floor_div(z, CW));
    let mut dz = -1;
    while dz <= 1 {
        let mut dx = -1;
        while dx <= 1 {
            let (nx, nz) = (cx + dx, cz + dz);
            if (nx, nz) != except
                && light::reaches_chunk(x - nx * CW, z - nz * CW, &light::TORCH_REACH)
            {
                mark(nx, nz);
            }
            dx += 1;
        }
        dz += 1;
    }
}

/// Mark a chunk for a remesh. A chunk whose edit mesh is in flight is owed one more when it
/// lands (its commit would otherwise put the flag back as it was when the edit began).
fn mark(cx: i32, cz: i32) {
    let s = slot(cx, cz);
    unsafe {
        if CHUNKS[s].loaded && CHUNKS[s].cx == cx && CHUNKS[s].cz == cz {
            CHUNKS[s].dirty = true;
            if EDIT_MESH_S == s {
                EDIT_PENDING_FULL = true;
            }
        }
    }
}

/// Work one marked entry out, the nearest the player. Returns whether it did.
pub fn tick() -> bool {
    unsafe {
        // A mesh in flight has faces in MESH_FACES: the flood waits for it to commit.
        if !PENDING || MESH_S != usize::MAX || EDIT_MESH_S != usize::MAX {
            return false;
        }
        let mut best = (i32::MAX, MAX_FLOODS);
        let mut i = 0;
        while i < MAX_FLOODS {
            if ENTRIES[i].state == DIRTY {
                let d = from_player(ENTRIES[i].x, ENTRIES[i].z);
                if d < best.0 {
                    best = (d, i);
                }
            }
            i += 1;
        }
        if best.1 == MAX_FLOODS {
            PENDING = false;
            return false;
        }
        reflow(best.1, (i32::MIN, i32::MIN));
        true
    }
}

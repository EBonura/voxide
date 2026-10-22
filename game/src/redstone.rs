//! Redstone: levers and torches that give power, dust that carries it, and the
//! doors, pistons and TNT that take it.
//!
//! The model is Java's, cut down to what the blocks here have:
//!
//! - Dust carries 0..15. A lever that is on, or a lit redstone torch, next to
//!   it or under its block, gives 15; every dust step away loses one. Dust
//!   joins dust on its own level and one step up or down (a step up is cut off
//!   by a solid block over the lower dust, a step down by a solid block beside
//!   it), and a dust with no neighbours powers all four sides.
//! - A redstone torch is lit unless the block it is fixed to is powered, and it
//!   re-reads that once a redstone tick (two game ticks), so a ring of torches
//!   is a clock and a torch and a wire is an inverter with its delay.
//! - A door, piston or TNT reacts when the power next to it changes: it opens
//!   (extends, lights) when power arrives and shuts (retracts) when it leaves.
//!   Nothing is remembered for them, so a door you open by hand stays open
//!   until the power beside it changes.
//!
//! Only the emitters (dust, torches, levers) are registered, at most `CAP` of
//! them; the things they power are found from the emitter that changed. The
//! power itself is derived, never saved: a world loads with every wire dark and
//! every torch lit, and the first tick settles it. What is saved is the
//! lever's switch and the door's swing, which are cells (bstate.rs).
//!
//! The rules read the world through `Cells`, so the host tests run them on a
//! grid in memory.

use crate::bstate::{self, kind_of, FDX, FDZ};
use crate::{LEVER, RSTORCH, WIRE};

/// Emitters registered at once.
pub const CAP: usize = 256;
const HASH: usize = 1024;

/// What the rules need to know of the world.
pub trait Cells {
    fn block(&self, x: i32, y: i32, z: i32) -> u8;
    /// A whole solid cube redstone cannot pass through and that torches and
    /// dust stand on.
    fn conductor(&self, b: u8) -> bool;
    /// Is the column in memory? An emitter in an unloaded column is dropped
    /// (it registers again when the chunk does).
    fn loaded(&self, x: i32, z: i32) -> bool;
}

/// A change in what a door, piston or TNT block should do.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Effect {
    /// The door with a half at the cell should be open (or shut).
    Door(i32, i32, i32, bool),
    /// The piston at the cell should be extended (or retracted).
    Piston(i32, i32, i32, bool),
    /// The TNT at the cell is lit.
    Tnt(i32, i32, i32),
}

pub const EFFECTS: usize = 32;

/// One registered emitter's dynamic value: dust power, or for a torch 1 when
/// lit and 0 when put out.
pub struct Net {
    x: [i16; CAP],
    y: [i16; CAP],
    z: [i16; CAP],
    kind: [u8; CAP],
    val: [u8; CAP],
    old: [u8; CAP],
    /// What each torch will be once every torch has read the world.
    nxt: [u8; CAP],
    n: usize,
    idx: [u16; HASH],
    /// Re-evaluate on the next tick.
    dirty: bool,
    /// A torch changed last tick, so the next may change more.
    active: bool,
    /// Emitters that fell off a full registry (shown, never powered).
    pub dropped: u32,
}

impl Net {
    pub const fn new() -> Self {
        Self {
            x: [0; CAP],
            y: [0; CAP],
            z: [0; CAP],
            kind: [0; CAP],
            val: [0; CAP],
            old: [0; CAP],
            nxt: [0; CAP],
            n: 0,
            idx: [0; HASH],
            dirty: false,
            active: false,
            dropped: 0,
        }
    }

    pub fn clear(&mut self) {
        self.n = 0;
        self.idx = [0; HASH];
        self.dirty = false;
        self.active = false;
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn slot(x: i32, y: i32, z: i32) -> usize {
        let h = (x as u32)
            .wrapping_mul(0x9E37_79B1)
            .wrapping_add((z as u32).wrapping_mul(0x85EB_CA6B))
            .wrapping_add((y as u32).wrapping_mul(0xC2B2_AE35));
        (h >> 15) as usize & (HASH - 1)
    }

    fn find(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        let mut s = Self::slot(x, y, z);
        loop {
            let v = self.idx[s] as usize;
            if v == 0 {
                return None;
            }
            let i = v - 1;
            if self.x[i] as i32 == x && self.y[i] as i32 == y && self.z[i] as i32 == z {
                return Some(i);
            }
            s = (s + 1) & (HASH - 1);
        }
    }

    fn insert_index(&mut self, i: usize) {
        let mut s = Self::slot(self.x[i] as i32, self.y[i] as i32, self.z[i] as i32);
        while self.idx[s] != 0 {
            s = (s + 1) & (HASH - 1);
        }
        self.idx[s] = (i + 1) as u16;
    }

    fn reindex(&mut self) {
        self.idx = [0; HASH];
        let mut i = 0;
        while i < self.n {
            self.insert_index(i);
            i += 1;
        }
    }

    /// Note that block `b` now stands at a cell: register it if it emits.
    pub fn changed(&mut self, x: i32, y: i32, z: i32, b: u8) {
        self.dirty = true;
        let k = kind_of(b);
        if k != WIRE && k != RSTORCH && k != LEVER {
            return;
        }
        if self.find(x, y, z).is_some() {
            return;
        }
        if self.n >= CAP {
            self.dropped += 1;
            return;
        }
        let i = self.n;
        self.x[i] = x as i16;
        self.y[i] = y as i16;
        self.z[i] = z as i16;
        self.kind[i] = k;
        self.val[i] = if k == RSTORCH { 1 } else { 0 };
        self.n += 1;
        self.insert_index(i);
    }

    /// Dust power at a cell (0 where there is none).
    pub fn wire_power(&self, x: i32, y: i32, z: i32) -> u8 {
        match self.find(x, y, z) {
            Some(i) if self.kind[i] == WIRE => self.val[i],
            _ => 0,
        }
    }

    /// Is the redstone torch at a cell lit? One not yet registered is.
    pub fn torch_lit(&self, x: i32, y: i32, z: i32) -> bool {
        match self.find(x, y, z) {
            Some(i) if self.kind[i] == RSTORCH => self.val[i] != 0,
            _ => true,
        }
    }

    // ---- The rules.

    fn is_wire<W: Cells>(w: &W, x: i32, y: i32, z: i32) -> bool {
        kind_of(w.block(x, y, z)) == WIRE
    }

    /// How the dust at (x, y, z) joins each of its four sides: 0 not at all, 1
    /// to something on its own level, 2 up a step, 3 down a step. A lever or a
    /// torch beside it counts as a join (the dust leans toward it), but only
    /// dust passes power along.
    pub fn links<W: Cells>(w: &W, x: i32, y: i32, z: i32) -> [u8; 4] {
        let mut out = [0u8; 4];
        let over = w.conductor(w.block(x, y + 1, z));
        let mut d = 0;
        while d < 4 {
            let (nx, nz) = (x + FDX[d], z + FDZ[d]);
            let side = w.block(nx, y, nz);
            let k = kind_of(side);
            out[d] = if k == WIRE || k == RSTORCH || k == LEVER {
                1
            } else if !over && Self::is_wire(w, nx, y + 1, nz) && w.conductor(side) {
                2
            } else if !w.conductor(side) && Self::is_wire(w, nx, y - 1, nz) {
                3
            } else {
                0
            };
            d += 1;
        }
        // A dust with one join runs straight through the cell, out the far
        // side as well: that is what lets a line end power the door it ends at.
        let joined = (out[0] != 0) as u8 + (out[1] != 0) as u8 + (out[2] != 0) as u8 + (out[3] != 0) as u8;
        if joined == 1 {
            let mut d = 0;
            while d < 4 {
                if out[d] != 0 {
                    out[(d + 2) & 3] = 1;
                    break;
                }
                d += 1;
            }
        }
        out
    }

    /// Does the dust at (x, y, z) with these joins power the cell on side `d`?
    fn feeds_side(l: &[u8; 4], d: usize) -> bool {
        l[d] != 0 || (l[0] == 0 && l[1] == 0 && l[2] == 0 && l[3] == 0)
    }

    /// The cell a torch or lever is fixed to.
    fn attach(b: u8, x: i32, y: i32, z: i32) -> (i32, i32, i32) {
        let m = bstate::mount_of(b);
        if m == 0 {
            (x, y - 1, z)
        } else {
            let f = (m - 1) as usize;
            (x - FDX[f], y, z - FDZ[f])
        }
    }

    /// A conductor that something strong powers: a lever that is on fixed to
    /// it, or a lit redstone torch standing right under it.
    fn strongly_powered<W: Cells>(&self, w: &W, x: i32, y: i32, z: i32) -> bool {
        // Levers on any side, fixed to this cell.
        let mut d = 0;
        while d < 6 {
            let (dx, dy, dz) = crate::DIRS[d];
            let (nx, ny, nz) = (x + dx, y + dy, z + dz);
            let b = w.block(nx, ny, nz);
            if bstate::lever_on(b) && Self::attach(b, nx, ny, nz) == (x, y, z) {
                return true;
            }
            d += 1;
        }
        // A torch standing on it.
        let t = w.block(x, y - 1, z);
        kind_of(t) == RSTORCH && bstate::mount_of(t) == 0 && self.torch_lit(x, y - 1, z)
    }

    /// Is the conductor at (x, y, z) powered, by anything, weakly or strongly?
    /// This is what puts a torch fixed to it out.
    fn block_powered<W: Cells>(&self, w: &W, x: i32, y: i32, z: i32) -> bool {
        if self.strongly_powered(w, x, y, z) {
            return true;
        }
        // Dust on top of it.
        if self.wire_power(x, y + 1, z) > 0 {
            return true;
        }
        // Dust beside it that points at it.
        let mut d = 0;
        while d < 4 {
            let (wx, wz) = (x - FDX[d], z - FDZ[d]);
            if self.wire_power(wx, y, wz) > 0 {
                let l = Self::links(w, wx, y, wz);
                if Self::feeds_side(&l, d) {
                    return true;
                }
            }
            d += 1;
        }
        false
    }

    /// Does power reach the door, piston or TNT at (x, y, z)? Dust on top of it
    /// or pointing at it, a lever that is on or a lit torch beside it (not one
    /// fixed to it), or a strongly powered block beside it.
    pub fn component_powered<W: Cells>(&self, w: &W, x: i32, y: i32, z: i32) -> bool {
        if self.wire_power(x, y + 1, z) > 0 {
            return true;
        }
        let mut d = 0;
        while d < 4 {
            let (wx, wz) = (x - FDX[d], z - FDZ[d]);
            if self.wire_power(wx, y, wz) > 0 {
                let l = Self::links(w, wx, y, wz);
                if Self::feeds_side(&l, d) {
                    return true;
                }
            }
            d += 1;
        }
        let mut d = 0;
        while d < 6 {
            let (dx, dy, dz) = crate::DIRS[d];
            let (nx, ny, nz) = (x + dx, y + dy, z + dz);
            let b = w.block(nx, ny, nz);
            match kind_of(b) {
                // On, beside it or fixed to it: either way it powers the block.
                LEVER if bstate::lever_on(b) => return true,
                RSTORCH => {
                    if self.torch_lit(nx, ny, nz) && Self::attach(b, nx, ny, nz) != (x, y, z) {
                        return true;
                    }
                }
                _ => {
                    if w.conductor(b) && self.strongly_powered(w, nx, ny, nz) {
                        return true;
                    }
                }
            }
            d += 1;
        }
        false
    }

    /// 15 for dust at (x, y, z) fed straight from a source: a lever that is on
    /// or a lit torch beside it, or a strongly powered block touching it.
    fn fed<W: Cells>(&self, w: &W, x: i32, y: i32, z: i32) -> bool {
        let mut d = 0;
        while d < 6 {
            let (dx, dy, dz) = crate::DIRS[d];
            let (nx, ny, nz) = (x + dx, y + dy, z + dz);
            let b = w.block(nx, ny, nz);
            match kind_of(b) {
                LEVER if bstate::lever_on(b) => return true,
                RSTORCH if self.torch_lit(nx, ny, nz) => {
                    // Not through the block it is fixed to: dust on that block
                    // is fed by the block, which the torch under it powers.
                    if Self::attach(b, nx, ny, nz) != (x, y, z) {
                        return true;
                    }
                }
                _ => {
                    if w.conductor(b) && self.strongly_powered(w, nx, ny, nz) {
                        return true;
                    }
                }
            }
            d += 1;
        }
        false
    }

    /// One redstone tick. Writes what doors, pistons and TNT should now do to
    /// `fx` and returns how many. Quiet (and free) when nothing was disturbed.
    pub fn step<W: Cells>(&mut self, w: &W, fx: &mut [Effect; EFFECTS]) -> usize {
        // Drop emitters that are gone or off in unloaded country.
        let mut i = 0;
        let mut kept = 0;
        let mut removed = false;
        while i < self.n {
            let (x, y, z) = (self.x[i] as i32, self.y[i] as i32, self.z[i] as i32);
            let ok = w.loaded(x, z) && kind_of(w.block(x, y, z)) == self.kind[i];
            if ok {
                if kept != i {
                    self.x[kept] = self.x[i];
                    self.y[kept] = self.y[i];
                    self.z[kept] = self.z[i];
                    self.kind[kept] = self.kind[i];
                    self.val[kept] = self.val[i];
                }
                kept += 1;
            } else {
                removed = true;
            }
            i += 1;
        }
        if removed {
            self.n = kept;
            self.reindex();
            self.dirty = true;
        }
        if !self.dirty && !self.active {
            return 0;
        }
        self.dirty = false;
        let n = self.n;
        let mut i = 0;
        while i < n {
            self.old[i] = self.val[i];
            i += 1;
        }
        // Torches read the world as it was, all of them at once.
        let mut i = 0;
        while i < n {
            if self.kind[i] == RSTORCH {
                let (x, y, z) = (self.x[i] as i32, self.y[i] as i32, self.z[i] as i32);
                let (ax, ay, az) = Self::attach(w.block(x, y, z), x, y, z);
                self.nxt[i] = !self.block_powered(w, ax, ay, az) as u8;
            }
            i += 1;
        }
        let mut changed_torch = false;
        let mut i = 0;
        while i < n {
            if self.kind[i] == RSTORCH {
                changed_torch |= self.val[i] != self.nxt[i];
                self.val[i] = self.nxt[i];
            }
            i += 1;
        }
        // Dust: start from the sources, then spread down the line.
        let mut i = 0;
        while i < n {
            if self.kind[i] == WIRE {
                let (x, y, z) = (self.x[i] as i32, self.y[i] as i32, self.z[i] as i32);
                self.val[i] = if self.fed(w, x, y, z) { 15 } else { 0 };
            }
            i += 1;
        }
        let mut level = 15u8;
        while level > 1 {
            let mut i = 0;
            while i < n {
                if self.kind[i] == WIRE && self.val[i] == level {
                    let (x, y, z) = (self.x[i] as i32, self.y[i] as i32, self.z[i] as i32);
                    let l = Self::links(w, x, y, z);
                    let mut d = 0;
                    while d < 4 {
                        let ny = match l[d] {
                            1 => y,
                            2 => y + 1,
                            3 => y - 1,
                            _ => {
                                d += 1;
                                continue;
                            }
                        };
                        if let Some(j) = self.find(x + FDX[d], ny, z + FDZ[d]) {
                            if self.kind[j] == WIRE && self.val[j] < level - 1 {
                                self.val[j] = level - 1;
                            }
                        }
                        d += 1;
                    }
                }
                i += 1;
            }
            level -= 1;
        }
        // What changed, and what it powers.
        let mut count = 0;
        let mut moved = false;
        let mut i = 0;
        while i < n {
            let (x, y, z) = (self.x[i] as i32, self.y[i] as i32, self.z[i] as i32);
            let was = self.old[i];
            let now = self.val[i];
            moved |= was != now;
            let edge = match self.kind[i] {
                RSTORCH => was != now,
                WIRE => (was > 0) != (now > 0),
                _ => false,
            };
            if edge {
                count = self.react(w, x, y, z, fx, count);
            }
            i += 1;
        }
        // Anything that moved may move what reads it next tick (a torch reads
        // dust, dust reads a torch): stay awake until the net is still. A
        // ring of torches never is, and keeps ticking.
        self.dirty = moved;
        self.active = changed_torch;
        count
    }

    /// What the door, piston or TNT at (x, y, z) should do right now, judged
    /// from the power beside it: for a block just set down.
    pub fn cell_effect<W: Cells>(&self, w: &W, x: i32, y: i32, z: i32) -> Option<Effect> {
        let cb = w.block(x, y, z);
        if bstate::is_door(cb) {
            let low = if bstate::door_upper(cb) { y - 1 } else { y };
            let on = self.component_powered(w, x, low, z) || self.component_powered(w, x, low + 1, z);
            // Only an unpowered door has nothing to do; a powered one opens.
            return if on { Some(Effect::Door(x, y, z, true)) } else { None };
        }
        if kind_of(cb) == crate::PISTON && self.component_powered(w, x, y, z) {
            return Some(Effect::Piston(x, y, z, true));
        }
        if kind_of(cb) == crate::TNT && self.component_powered(w, x, y, z) {
            return Some(Effect::Tnt(x, y, z));
        }
        None
    }

    /// An emitter at (x, y, z) switched: ask the things it can reach.
    pub fn react<W: Cells>(
        &self,
        w: &W,
        x: i32,
        y: i32,
        z: i32,
        fx: &mut [Effect; EFFECTS],
        mut count: usize,
    ) -> usize {
        let b = w.block(x, y, z);
        // The block an emitter powers hard: a lever's, a torch's roof.
        let (qx, qy, qz) = match kind_of(b) {
            LEVER => Self::attach(b, x, y, z),
            RSTORCH => (x, y + 1, z),
            _ => (x, y, z),
        };
        let mut c = 0;
        while c < 12 {
            let (dx, dy, dz) = crate::DIRS[c % 6];
            // The emitter's own neighbours, then those of the block it powers.
            let (cx, cy, cz) =
                if c < 6 { (x + dx, y + dy, z + dz) } else { (qx + dx, qy + dy, qz + dz) };
            c += 1;
            let cb = w.block(cx, cy, cz);
            let eff = if bstate::is_door(cb) {
                let low = if bstate::door_upper(cb) { cy - 1 } else { cy };
                let on = self.component_powered(w, cx, low, cz)
                    || self.component_powered(w, cx, low + 1, cz);
                Effect::Door(cx, cy, cz, on)
            } else if kind_of(cb) == crate::PISTON {
                Effect::Piston(cx, cy, cz, self.component_powered(w, cx, cy, cz))
            } else if kind_of(cb) == crate::TNT && self.component_powered(w, cx, cy, cz) {
                Effect::Tnt(cx, cy, cz)
            } else {
                continue;
            };
            let mut k = 0;
            let mut seen = false;
            while k < count {
                seen |= fx[k] == eff;
                k += 1;
            }
            if !seen && count < EFFECTS {
                fx[count] = eff;
                count += 1;
            }
        }
        count
    }
}

// ---- The game's one net, over the live world.

#[cfg(not(test))]
pub static mut NET: Net = Net::new();

#[cfg(not(test))]
struct Live;

#[cfg(not(test))]
impl Cells for Live {
    fn block(&self, x: i32, y: i32, z: i32) -> u8 {
        crate::get_block_i32(x, y, z)
    }
    fn conductor(&self, b: u8) -> bool {
        crate::blocks::is_sturdy(b)
    }
    fn loaded(&self, x: i32, z: i32) -> bool {
        crate::world::column_loaded(x, z)
    }
}

#[cfg(not(test))]
/// Doors, pistons and TNT redstone has worked, for the gate.
static mut WORKED: u32 = 0;

#[cfg(not(test))]
pub fn worked() -> u32 {
    unsafe { WORKED }
}

/// Emitters registered, for the gate.
#[cfg(not(test))]
pub fn emitters() -> usize {
    unsafe { NET.len() }
}

#[cfg(not(test))]
pub fn mark_dirty() {
    unsafe { NET.mark_dirty() };
}

/// A cell was set by the player or by a replay: register it if it emits.
#[cfg(not(test))]
pub fn note(x: i32, y: i32, z: i32, b: u8) {
    unsafe { NET.changed(x, y, z, b) };
}

#[cfg(not(test))]
pub fn reset() {
    unsafe { NET.clear() };
}

#[cfg(not(test))]
pub fn wire_power(x: i32, y: i32, z: i32) -> u8 {
    unsafe { NET.wire_power(x, y, z) }
}

#[cfg(not(test))]
pub fn torch_lit(x: i32, y: i32, z: i32) -> bool {
    unsafe { NET.torch_lit(x, y, z) }
}

#[cfg(not(test))]
pub fn wire_links(x: i32, y: i32, z: i32) -> [u8; 4] {
    Net::links(&Live, x, y, z)
}

/// One redstone tick on the live world; the effects it leaves are applied here.
#[cfg(not(test))]
#[inline(never)]
pub fn tick() {
    let mut fx = [Effect::Tnt(0, 0, 0); EFFECTS];
    let n = unsafe { NET.step(&Live, &mut fx) };
    unsafe { WORKED += n as u32 };
    let mut i = 0;
    while i < n {
        match fx[i] {
            Effect::Door(x, y, z, open) => crate::blocks::set_door(x, y, z, open),
            Effect::Piston(x, y, z, on) => crate::blocks::drive_piston(x, y, z, on),
            Effect::Tnt(x, y, z) => crate::ignite_tnt(x, y, z),
        }
        i += 1;
    }
}

/// A door, piston or TNT was just set down at a cell: let it react to power
/// already beside it.
#[cfg(not(test))]
pub fn poke(x: i32, y: i32, z: i32) {
    let e = unsafe { NET.cell_effect(&Live, x, y, z) };
    match e {
        Some(Effect::Door(x, y, z, open)) => crate::blocks::set_door(x, y, z, open),
        Some(Effect::Piston(x, y, z, on)) => crate::blocks::drive_piston(x, y, z, on),
        Some(Effect::Tnt(x, y, z)) => crate::ignite_tnt(x, y, z),
        None => {}
    }
}

/// A lever was thrown: its neighbours may need to react in this tick, and the
/// net has to run.
#[cfg(not(test))]
pub fn lever_thrown(x: i32, y: i32, z: i32) {
    let mut fx = [Effect::Tnt(0, 0, 0); EFFECTS];
    let n = unsafe { NET.react(&Live, x, y, z, &mut fx, 0) };
    unsafe { NET.mark_dirty() };
    unsafe { WORKED += n as u32 };
    let mut i = 0;
    while i < n {
        match fx[i] {
            Effect::Door(x, y, z, open) => crate::blocks::set_door(x, y, z, open),
            Effect::Piston(x, y, z, on) => crate::blocks::drive_piston(x, y, z, on),
            Effect::Tnt(x, y, z) => crate::ignite_tnt(x, y, z),
        }
        i += 1;
    }
}

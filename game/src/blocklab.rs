//! Headless proving ground for the shaped blocks (`--features block-lab`, never
//! shipped). The first SELECT builds, in front of the player, a cobble floor
//! with doors, beds, torches, levers, a ladder, a circuit of dust from a lever
//! to a door and a piston, and a row of things to shoot at; every later SELECT
//! stands the player at the next place and aims the crosshair at the next
//! target. Everything after that is real input (tests/blocklab.toml throws
//! the lever with L2, shoots with L2 held on the bow, and so on); only the
//! standing and aiming, which would otherwise depend on where the spawn fell
//! and how many stick ticks a look took, is set here.

use crate::{bstate, *};

/// Stock for the proving ground.
pub fn seed() {
    let mut i = 0;
    while i < PLACEABLE.len() {
        inv_give(PLACEABLE[i], 8);
        i += 1;
    }
    inv_give(BOW, 1);
    inv_give(ARROW, 40);
}

static mut BUILT: bool = false;
static mut VIEW: usize = 0;
/// x, floor y and z of the ground's corner.
static mut ORIGIN: (i32, i32, i32) = (0, 0, 0);

/// A raw set, logged: the ground is three thousand edits, which a per-edit
/// remesh queue would take minutes to catch up with. `select` remeshes the
/// loaded chunks once when it is built, as a load does.
fn put(x: i32, y: i32, z: i32, b: u8) {
    world::set_raw_pub(x, y, z, b);
    record_edit(x, y, z, b);
}

fn door(x: i32, y: i32, z: i32, f: u8, open: bool) {
    put(x, y, z, bstate::door(false, f, open));
    put(x, y + 1, z, bstate::door(true, f, open));
}

fn bed(x: i32, y: i32, z: i32, f: u8) {
    put(x, y, z, bstate::bed(false, f));
    put(
        x + bstate::FDX[f as usize],
        y,
        z + bstate::FDZ[f as usize],
        bstate::bed(true, f),
    );
}

/// Cells of the ground relative to its corner, floor level 0.
const W: i32 = 18;
const D: i32 = 20;

fn build(x0: i32, y: i32, z0: i32) {
    let mut dx = 0;
    while dx < W {
        let mut dz = 0;
        while dz < D {
            put(x0 + dx, y, z0 + dz, COBBLE);
            let mut dy = 1;
            while dy < 8 {
                put(x0 + dx, y + dy, z0 + dz, AIR);
                dy += 1;
            }
            dz += 1;
        }
        dx += 1;
    }
    let s = y + 1;
    // Doors along z0+2: closed, open, then the facings.
    door(x0 + 1, s, z0 + 2, 0, false);
    door(x0 + 3, s, z0 + 2, 0, true);
    door(x0 + 5, s, z0 + 2, 1, false);
    door(x0 + 7, s, z0 + 2, 1, true);
    door(x0 + 9, s, z0 + 2, 2, false);
    door(x0 + 11, s, z0 + 2, 3, false);
    // Beds along z0+5 (foot first, head one cell on the way it faces).
    bed(x0 + 1, s, z0 + 5, 0);
    bed(x0 + 4, s, z0 + 5, 1);
    bed(x0 + 8, s, z0 + 6, 2);
    bed(x0 + 11, s, z0 + 5, 3);
    // A wall of stone at z0+9 with torches, levers and a ladder against it.
    let mut wx = 0;
    while wx < 9 {
        let mut wy = 0;
        while wy < 4 {
            put(x0 + wx, s + wy, z0 + 9, STONE);
            wy += 1;
        }
        wx += 1;
    }
    put(x0 + 1, s, z0 + 8, bstate::make(TORCH, 0));
    put(x0 + 2, s + 1, z0 + 8, bstate::make(TORCH, 1 + 2));
    put(x0 + 3, s, z0 + 8, bstate::make(RSTORCH, 0));
    put(x0 + 4, s + 1, z0 + 8, bstate::make(RSTORCH, 1 + 2));
    put(x0 + 5, s + 1, z0 + 8, bstate::lever(1 + 2, false));
    put(x0 + 6, s + 1, z0 + 8, bstate::lever(1 + 2, true));
    put(x0 + 7, s, z0 + 8, bstate::lever(0, false));
    put(x0 + 8, s, z0 + 8, bstate::make(LADDER, 2));
    put(x0 + 8, s + 1, z0 + 8, bstate::make(LADDER, 2));
    // The circuit along z0+12: a lever on a stone, six dust, a door at the end.
    put(x0 + 1, s, z0 + 12, STONE);
    put(x0 + 1, s + 1, z0 + 12, bstate::lever(0, false));
    let mut k = 2;
    while k < 8 {
        put(x0 + k, s, z0 + 12, WIRE);
        k += 1;
    }
    door(x0 + 8, s, z0 + 12, 0, false);
    // A branch to a piston with a block in front of it: dust at (4, 13) and
    // (4, 14) runs on toward +Z into the piston at (4, 15), which faces +Z.
    put(x0 + 4, s, z0 + 13, WIRE);
    put(x0 + 4, s, z0 + 14, WIRE);
    put(x0 + 4, s, z0 + 15, bstate::piston(4, false));
    put(x0 + 4, s, z0 + 16, COBBLE);
    // A step up for the dust to climb, fed by a lever that is already on.
    put(x0 + 12, s, z0 + 12, bstate::lever(0, true));
    put(x0 + 13, s, z0 + 12, WIRE);
    put(x0 + 14, s, z0 + 12, STONE);
    put(x0 + 14, s + 1, z0 + 12, WIRE);
}

/// Look from the player's eye at the middle of a cell: set yaw and pitch so the
/// crosshair is on it.
fn aim(p: &mut Player, tx: i32, ty: i32, tz: i32, fy: i32, fx: i32) {
    let ex = p.x;
    let ez = p.z;
    let ey = p.y + EYE_HEIGHT;
    // The point: the middle of the cell, `fy` sixty-fourths up it.
    let (px, py, pz) = (
        block_to_world_x(tx) + BLOCK / 2 + fx,
        ty * BLOCK + fy,
        block_to_world_z(tz) + BLOCK / 2,
    );
    let (dx, dy, dz) = (px - ex, py - ey, pz - ez);
    // Yaw: the Q12 angle whose forward (sin, cos) lines up with (dx, dz).
    let mut best = (i32::MIN, 0u16);
    let mut a = 0u16;
    while a < 4096 {
        let d = sincos::sin_q12(a) * dx + sincos::cos_q12(a) * dz;
        if d > best.0 {
            best = (d, a);
        }
        a += 4;
    }
    p.yaw = best.1;
    let h = psx_math::int32::isqrt_i32(dx * dx + dz * dz).max(1);
    // Pitch: sin(p) * h = cos(p) * dy.
    let mut best = (i32::MAX, 0i32);
    let mut q = -1024i32;
    while q <= 1024 {
        let a = (q & 0x0fff) as u16;
        let e = (sincos::sin_q12(a) * h - sincos::cos_q12(a) * dy).abs();
        if e < best.0 {
            best = (e, q);
        }
        q += 2;
    }
    p.pitch = best.1 as i16;
}

/// Stand at cell (sx, sz) of the ground and aim at (tx, ty, tz), all relative
/// to its corner and floor.
fn stand(p: &mut Player, sx: i32, sz: i32, tx: i32, ty: i32, tz: i32, fy: i32) {
    stand_off(p, sx, sz, tx, ty, tz, fy, 0);
}

/// `stand` aiming `fx` world units to the right of the target's middle.
#[allow(clippy::too_many_arguments)]
fn stand_off(p: &mut Player, sx: i32, sz: i32, tx: i32, ty: i32, tz: i32, fy: i32, fx: i32) {
    let (ox, oy, oz) = unsafe { ORIGIN };
    p.x = block_to_world_x(ox + sx) + BLOCK / 2;
    p.z = block_to_world_z(oz + sz) + BLOCK / 2;
    p.y = (oy + 1) * BLOCK;
    p.vy = 0;
    aim(p, ox + tx, oy + ty, oz + tz, fy, fx);
}

/// SELECT after the last view: take the bow in hand, looking down the range.
fn take_bow(p: &mut Player) {
    unsafe {
        let mut i = 0;
        while i < HOTBAR_VIS {
            if HOTBAR[i] == BOW {
                HOTBAR_SEL = i;
                p.selected = BOW;
            }
            i += 1;
        }
    }
}

/// Number of views. 0 overview, 1 the first door, 2 the same door open (aimed
/// at its panel, now on the hinge edge), 3 the beds from above, 4 the torches
/// and levers, 5 the ladder, 6 the piston (unpowered), 7 the lever of the
/// circuit, 8 dust on the line, 9 the door the circuit ends at, 10 the piston
/// again, 11 the dust step, 12 the head of the first bed, 13 the bow range.
pub const VIEWS: usize = 14;

pub fn select(p: &mut Player) {
    unsafe {
        if !BUILT {
            BUILT = true;
            let (bx, bz) = (world_to_block_x(p.x), world_to_block_z(p.z));
            let y = world::surface_y(bx, bz);
            ORIGIN = (bx - 6, y, bz + 3);
            build(bx - 6, y, bz + 3);
            world::remesh_loaded();
            redstone::mark_dirty();
            return;
        }
        let v = VIEW;
        VIEW += 1;
        match v {
            0 => stand(p, 9, 0, 9, 1, 10, 16),
            1 => stand(p, 1, 0, 1, 1, 2, 32),
            // The open door's panel is on the hinge edge, the last quarter of x.
            2 => stand_off(p, 1, 0, 1, 1, 2, 32, 26),
            3 => {
                stand(p, 6, 3, 6, 0, 5, 0);
                p.pitch = -420;
            }
            4 => stand(p, 3, 6, 4, 2, 8, 24),
            5 => stand(p, 6, 6, 8, 1, 8, 32),
            // The piston and the block in front of it, before any power.
            6 | 10 => stand(p, 7, 15, 4, 1, 15, 32),
            // The lever of the circuit, from two blocks off.
            7 => stand(p, 1, 10, 1, 2, 12, 8),
            // The dust at (4, 12), from beside the wall.
            8 => stand(p, 4, 10, 4, 1, 12, 2),
            // The door at the end of the line.
            9 => stand(p, 8, 10, 8, 1, 12, 32),
            // Dust climbing the step.
            11 => stand(p, 13, 10, 14, 2, 12, 2),
            // The head of the first bed, from two cells off.
            12 => stand(p, 3, 6, 1, 1, 6, 20),
            _ => {
                p.x = block_to_world_x(ORIGIN.0 + 14) + BLOCK / 2;
                p.z = block_to_world_z(ORIGIN.2 + 17) + BLOCK / 2;
                p.y = (ORIGIN.1 + 1) * BLOCK;
                p.vy = 0;
                p.yaw = 0x800; // looking back along -Z
                p.pitch = 0;
                take_bow(p);
            }
        }
    }
}

//! Block shapes: the one table that says what a block that is not a whole cube
//! looks like, how it is picked and what it collides with.
//!
//! A shape is a few boxes in sixteenths of a block. Rendering, the crosshair
//! ray and collision all read the same boxes, so a door cannot look open and
//! block like a closed one. Facing is a quarter turn of a box laid out for
//! facing 0 (the block faces +Z), done here once; none of the renderers or the
//! physics know about facing.
//!
//! Face order everywhere is +X -X +Y -Y +Z -Z.
//!
//! Plain data and integer maths, no allocation, so the host tests include this
//! file as it is.

use crate::bstate::{self, kind_of, state_of};
use crate::{
    tex, BED, DOOR_C, FENCE, LADDER, LEVER, PISTON, PISTON_HEAD, RSTORCH, SLAB, STAIRS_E, STAIRS_N,
    STAIRS_S, STAIRS_W, TORCH, WIRE,
};

pub const MAX_PARTS: usize = 3;
/// Marks a face that is not drawn.
pub const NO_TILE: u8 = 0xff;

/// How the texture sits on a part's side faces.
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Uv {
    /// Cut from the face's own extent, as a full block would wear the tile: a
    /// half-height slab shows half the tile.
    Auto,
    /// A narrow strip: sides read columns `u0..u1` over the part's own height,
    /// aligned to the bottom of the tile (a torch's stick and flame); the top
    /// and bottom read `u0..u1` by `top_v..top_v + (u1 - u0)`.
    Strip { u0: u8, u1: u8, top_v: u8 },
}

#[derive(Copy, Clone)]
pub struct Part {
    /// Box in sixteenths: x0 y0 z0 x1 y1 z1.
    pub b: [u8; 6],
    pub tile: [u8; 6],
    pub uv: Uv,
    /// Quarter turns of the texture on the top and bottom faces (a rotated
    /// block's top is rotated with it).
    pub turn: u8,
}

pub const EMPTY: Part = Part {
    b: [0; 6],
    tile: [NO_TILE; 6],
    uv: Uv::Auto,
    turn: 0,
};

/// What the player's body does with the block.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Solid {
    /// Walk through it (air, plants, dust, torches, ladders).
    No,
    /// A whole cube.
    Full,
    /// Collides with its `parts` only.
    Parts,
}

const fn part(b: [u8; 6], tile: [u8; 6]) -> Part {
    Part {
        b,
        tile,
        uv: Uv::Auto,
        turn: 0,
    }
}

/// All six faces wear `t`.
const fn all(t: u8) -> [u8; 6] {
    [t; 6]
}

/// Quarter turns of a horizontal direction index (0 +Z, 1 +X, 2 -Z, 3 -X)
/// onto a face index (+X -X +Y -Y +Z -Z).
const RING: [usize; 4] = [4, 0, 5, 1];

/// Turn a part laid out for facing 0 by `f` quarter turns about the cell's
/// vertical axis (+Z toward +X for f = 1).
fn turned(p: Part, f: u8) -> Part {
    let f = f & 3;
    if f == 0 {
        return p;
    }
    let [x0, y0, z0, x1, y1, z1] = p.b;
    // (x, z) of the two corners through the turn, then min and max.
    let map = |x: u8, z: u8| -> (u8, u8) {
        match f {
            1 => (z, 16 - x),
            2 => (16 - x, 16 - z),
            _ => (16 - z, x),
        }
    };
    let (ax, az) = map(x0, z0);
    let (bx, bz) = map(x1, z1);
    let mut tile = p.tile;
    // Horizontal faces move round the ring; +Y and -Y stay.
    let mut d = 0;
    while d < 4 {
        tile[RING[(d + f as usize) & 3]] = p.tile[RING[d]];
        d += 1;
    }
    Part {
        b: [ax.min(bx), y0, az.min(bz), ax.max(bx), y1, az.max(bz)],
        tile,
        uv: p.uv,
        turn: (p.turn + f) & 3,
    }
}

/// A part laid out facing up, turned to face `dir` (+X -X +Y -Y +Z -Z). Pistons
/// are symmetric about their axis, so a swap of axes does for a turn.
fn aimed(p: Part, dir: u8) -> Part {
    let [x0, y0, z0, x1, y1, z1] = p.b;
    let t = p.tile;
    // Faces: 0 +X, 1 -X, 2 +Y, 3 -Y, 4 +Z, 5 -Z.
    let (b, tile) = match dir {
        2 => return p,
        3 => (
            [x0, 16 - y1, z0, x1, 16 - y0, z1],
            [t[0], t[1], t[3], t[2], t[4], t[5]],
        ),
        4 => (
            [x0, z0, y0, x1, z1, y1],
            [t[0], t[1], t[4], t[5], t[2], t[3]],
        ),
        5 => (
            [x0, z0, 16 - y1, x1, z1, 16 - y0],
            [t[0], t[1], t[5], t[4], t[3], t[2]],
        ),
        0 => (
            [y0, x0, z0, y1, x1, z1],
            [t[2], t[3], t[0], t[1], t[4], t[5]],
        ),
        _ => (
            [16 - y1, x0, z0, 16 - y0, x1, z1],
            [t[3], t[2], t[0], t[1], t[4], t[5]],
        ),
    };
    Part {
        b,
        tile,
        uv: p.uv,
        turn: 0,
    }
}

fn push(out: &mut [Part; MAX_PARTS], n: &mut usize, p: Part) {
    if *n < MAX_PARTS {
        out[*n] = p;
        *n += 1;
    }
}

/// The boxes of block `b` (any cell, state included), in drawing order. Empty
/// for a block that is not a shape.
pub fn parts(b: u8, out: &mut [Part; MAX_PARTS]) -> usize {
    let mut n = 0;
    let k = kind_of(b);
    let s = state_of(b);
    match k {
        SLAB => push(out, &mut n, part([0, 0, 0, 16, 8, 16], all(tex::T_COBBLE))),
        STAIRS_N | STAIRS_E | STAIRS_S | STAIRS_W => {
            push(out, &mut n, part([0, 0, 0, 16, 8, 16], all(tex::T_COBBLE)));
            let step = match k {
                STAIRS_N => [0, 8, 8, 16, 16, 16],
                STAIRS_S => [0, 8, 0, 16, 16, 8],
                STAIRS_E => [8, 8, 0, 16, 16, 16],
                _ => [0, 8, 0, 8, 16, 16],
            };
            push(out, &mut n, part(step, all(tex::T_COBBLE)));
        }
        FENCE => push(
            out,
            &mut n,
            part([6, 0, 6, 10, 16, 10], all(tex::T_WOOD_SIDE)),
        ),
        DOOR_C => {
            let f = bstate::door_facing(b);
            let tile = if bstate::door_upper(b) {
                tex::T_DOOR_UP
            } else {
                tex::T_DOOR_LO
            };
            let edge = tex::T_WOOD_SIDE;
            // Laid out for facing 0, the placer looking along +Z: the closed
            // panel on the near (-Z) edge, the open one on the hinge side
            // (the placer's left, +X).
            let p = if bstate::door_open(b) {
                part([13, 0, 0, 16, 16, 16], [tile, tile, edge, edge, edge, edge])
            } else {
                part([0, 0, 0, 16, 16, 3], [edge, edge, edge, edge, tile, tile])
            };
            push(out, &mut n, turned(p, f));
        }
        BED => {
            let f = bstate::bed_facing(b);
            let top = if bstate::bed_head(b) {
                tex::T_BED_HEAD
            } else {
                tex::T_BED_FOOT
            };
            let side = tex::T_BED_SIDE;
            let p = part(
                [0, 0, 0, 16, 9, 16],
                [side, side, top, tex::T_PLANK, side, side],
            );
            push(out, &mut n, turned(p, f));
        }
        TORCH | RSTORCH => {
            let t = if k == TORCH {
                tex::T_TORCH
            } else {
                tex::T_RSTORCH
            };
            let strip = Uv::Strip {
                u0: 7,
                u1: 9,
                top_v: 6,
            };
            if s == 0 {
                push(
                    out,
                    &mut n,
                    Part {
                        uv: strip,
                        ..part([7, 0, 7, 9, 10, 9], all(t))
                    },
                );
            } else {
                // On a wall facing s - 1: laid out for facing 0, against the
                // -Z wall, then turned.
                let p = Part {
                    uv: strip,
                    ..part([7, 3, 0, 9, 13, 2], all(t))
                };
                push(out, &mut n, turned(p, s - 1));
            }
        }
        LEVER => {
            let on = bstate::lever_on(b);
            let m = bstate::mount_of(b);
            let base = all(tex::T_COBBLE);
            let stick = Uv::Strip {
                u0: 7,
                u1: 9,
                top_v: 0,
            };
            if m == 0 {
                push(out, &mut n, part([4, 0, 5, 12, 3, 11], base));
                let h = if on {
                    [7, 3, 8, 9, 11, 10]
                } else {
                    [7, 3, 6, 9, 11, 8]
                };
                push(
                    out,
                    &mut n,
                    Part {
                        uv: stick,
                        ..part(h, all(tex::T_WOOD_SIDE))
                    },
                );
            } else {
                let f = m - 1;
                let plate = part([5, 4, 0, 11, 12, 3], base);
                let h = if on {
                    [7, 8, 3, 9, 15, 5]
                } else {
                    [7, 1, 3, 9, 8, 5]
                };
                let hp = Part {
                    uv: stick,
                    ..part(h, all(tex::T_WOOD_SIDE))
                };
                push(out, &mut n, turned(plate, f));
                push(out, &mut n, turned(hp, f));
            }
        }
        LADDER => {
            // Laid out for facing 0, against the -Z wall: a thin panel.
            let t = tex::T_LADDER_W;
            let p = part(
                [0, 0, 0, 16, 16, 2],
                [NO_TILE, NO_TILE, NO_TILE, NO_TILE, t, t],
            );
            push(out, &mut n, turned(p, bstate::ladder_facing(b)));
        }
        PISTON | PISTON_HEAD => {
            // Laid out facing up: a stone body and a wooden face plate, or for the
            // arm, the plate and a shaft back to the piston.
            let d = bstate::piston_dir(b);
            let body = all(tex::T_COBBLE);
            let plate = [
                tex::T_WOOD_SIDE,
                tex::T_WOOD_SIDE,
                tex::T_PLANK,
                tex::T_COBBLE,
                tex::T_WOOD_SIDE,
                tex::T_WOOD_SIDE,
            ];
            if k == PISTON {
                push(out, &mut n, aimed(part([0, 0, 0, 16, 12, 16], body), d));
                if !bstate::piston_extended(b) {
                    push(out, &mut n, aimed(part([0, 12, 0, 16, 16, 16], plate), d));
                }
            } else {
                push(out, &mut n, aimed(part([0, 12, 0, 16, 16, 16], plate), d));
                push(
                    out,
                    &mut n,
                    aimed(part([6, 0, 6, 10, 12, 10], all(tex::T_WOOD_SIDE)), d),
                );
            }
        }
        WIRE => {
            // Only the box the crosshair aims at; dust is drawn by its own pass.
            push(out, &mut n, part([0, 0, 0, 16, 1, 16], all(NO_TILE)));
        }
        _ => {}
    }
    n
}

// One byte per cell id, built at compile time: bit 0 a shape (drawn from its
// boxes, not meshed), bits 1..2 what the body does with it. These are asked of
// every plant entry and every cell a collision test touches, so they are a
// load, not a chain of compares.
const SHAPE: u8 = 1;
const BODY_NO: u8 = 2;
const BODY_PARTS: u8 = 4;

const fn class_of(b: u8) -> u8 {
    match bstate::kind_of_c(b) {
        SLAB | STAIRS_N | STAIRS_E | STAIRS_S | STAIRS_W | DOOR_C | BED | PISTON | PISTON_HEAD => {
            SHAPE | BODY_PARTS
        }
        TORCH | RSTORCH | LEVER | LADDER | WIRE => SHAPE | BODY_NO,
        // The fence stays a whole solid cell for the body (a fence is 1.5 high
        // in Java, which the player cannot jump), though it draws as a post.
        FENCE => SHAPE,
        _ => 0,
    }
}

const fn class_table() -> [u8; 256] {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = class_of(i as u8);
        i += 1;
    }
    t
}

static CLASS: [u8; 256] = class_table();

/// Is `b` drawn from its boxes (and not meshed as a cube or a cross sprite)?
#[inline(always)]
pub fn is_shape(b: u8) -> bool {
    CLASS[b as usize] & SHAPE != 0
}

/// The whole-kind answer; the ext ids of a family share it.
#[inline(always)]
pub fn solid(b: u8) -> Solid {
    let c = CLASS[b as usize];
    if c & BODY_PARTS != 0 {
        Solid::Parts
    } else if c & BODY_NO != 0 {
        Solid::No
    } else {
        Solid::Full
    }
}

/// True if the box `part` of the block at cell (bx, by, bz) overlaps the body
/// box (centre x/z, feet y, half width, height), all in world units.
pub fn part_hits(
    p: &Part,
    bx: i32,
    by: i32,
    bz: i32,
    cx: i32,
    feet: i32,
    cz: i32,
    hw: i32,
    h: i32,
) -> bool {
    let (x0, y0, z0, x1, y1, z1) = world_box(p, bx, by, bz);
    cx + hw > x0 && cx - hw < x1 && feet + h > y0 && feet < y1 && cz + hw > z0 && cz - hw < z1
}

/// A part's box in world units (a block is 64 units, so a sixteenth is 4).
#[inline]
pub fn world_box(p: &Part, bx: i32, by: i32, bz: i32) -> (i32, i32, i32, i32, i32, i32) {
    let [x0, y0, z0, x1, y1, z1] = p.b;
    let (ox, oy, oz) = (bx * 64, by * 64, bz * 64);
    (
        ox + x0 as i32 * 4,
        oy + y0 as i32 * 4,
        oz + z0 as i32 * 4,
        ox + x1 as i32 * 4,
        oy + y1 as i32 * 4,
        oz + z1 as i32 * 4,
    )
}

/// Does the body overlap block `b` standing at cell (bx, by, bz)? For
/// `Solid::Parts` blocks only the boxes count.
pub fn body_hits(
    b: u8,
    bx: i32,
    by: i32,
    bz: i32,
    cx: i32,
    feet: i32,
    cz: i32,
    hw: i32,
    h: i32,
) -> bool {
    let mut ps = [EMPTY; MAX_PARTS];
    let n = parts(b, &mut ps);
    let mut i = 0;
    while i < n {
        if part_hits(&ps[i], bx, by, bz, cx, feet, cz, hw, h) {
            return true;
        }
        i += 1;
    }
    false
}

/// The height (world units) of the highest part of block `b` at (bx, by, bz)
/// that the body's footprint (centre cx/cz, half width hw) stands over.
pub fn top_under(b: u8, bx: i32, by: i32, bz: i32, cx: i32, cz: i32, hw: i32) -> Option<i32> {
    let mut ps = [EMPTY; MAX_PARTS];
    let n = parts(b, &mut ps);
    let mut best: Option<i32> = None;
    let mut i = 0;
    while i < n {
        let (x0, _, z0, x1, y1, z1) = world_box(&ps[i], bx, by, bz);
        if cx + hw > x0
            && cx - hw < x1
            && cz + hw > z0
            && cz - hw < z1
            && best.map_or(true, |v| y1 > v)
        {
            best = Some(y1);
        }
        i += 1;
    }
    best
}

/// Does the point (world units) lie inside a part of block `b` at (bx, by, bz)?
pub fn point_in(b: u8, bx: i32, by: i32, bz: i32, x: i32, y: i32, z: i32) -> bool {
    let mut ps = [EMPTY; MAX_PARTS];
    let n = parts(b, &mut ps);
    let mut i = 0;
    while i < n {
        let (x0, y0, z0, x1, y1, z1) = world_box(&ps[i], bx, by, bz);
        if x >= x0 && x < x1 && y >= y0 && y < y1 && z >= z0 && z < z1 {
            return true;
        }
        i += 1;
    }
    false
}

/// The nearest distance along the ray `o + d * t` (d a Q12 direction, t in
/// world units) at which it enters a part of block `b` at (bx, by, bz), no
/// further than `reach`. A ray that starts inside a part enters it at 0.
pub fn ray_hit(
    b: u8,
    bx: i32,
    by: i32,
    bz: i32,
    o: [i32; 3],
    d: [i32; 3],
    reach: i32,
) -> Option<i32> {
    let mut ps = [EMPTY; MAX_PARTS];
    let n = parts(b, &mut ps);
    let mut best: Option<i32> = None;
    let mut i = 0;
    while i < n {
        let (x0, y0, z0, x1, y1, z1) = world_box(&ps[i], bx, by, bz);
        let lo = [x0, y0, z0];
        let hi = [x1, y1, z1];
        // Slab test, t in world units: (wall - origin) * 4096 / d.
        let (mut tmin, mut tmax) = (0i32, reach);
        let mut a = 0;
        let mut hit = true;
        while a < 3 {
            if d[a] == 0 {
                if o[a] < lo[a] || o[a] >= hi[a] {
                    hit = false;
                    break;
                }
            } else {
                let t0 = ((lo[a] - o[a]) << 12) / d[a];
                let t1 = ((hi[a] - o[a]) << 12) / d[a];
                let (near, far) = if t0 < t1 { (t0, t1) } else { (t1, t0) };
                tmin = tmin.max(near);
                tmax = tmax.min(far);
                if tmin > tmax {
                    hit = false;
                    break;
                }
            }
            a += 1;
        }
        if hit && best.map_or(true, |v| tmin < v) {
            best = Some(tmin);
        }
        i += 1;
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first(b: u8) -> Part {
        let mut ps = [EMPTY; MAX_PARTS];
        assert!(parts(b, &mut ps) > 0);
        ps[0]
    }

    #[test]
    fn a_turn_keeps_the_box_in_the_cell() {
        for f in 0..4 {
            let p = turned(part([0, 0, 0, 16, 16, 3], all(1)), f);
            assert!(p.b[0] < p.b[3] && p.b[2] < p.b[5]);
            assert!(p.b.iter().all(|&v| v <= 16));
        }
        // A panel on the -Z edge turns to the -X edge for facing 1... the
        // placer looking +X stands on the -X side.
        let p = turned(part([0, 0, 0, 16, 16, 3], all(1)), 1);
        assert_eq!(p.b, [0, 0, 0, 3, 16, 16]);
        let p = turned(part([0, 0, 0, 16, 16, 3], all(1)), 2);
        assert_eq!(p.b, [0, 0, 13, 16, 16, 16]);
        let p = turned(part([0, 0, 0, 16, 16, 3], all(1)), 3);
        assert_eq!(p.b, [13, 0, 0, 16, 16, 16]);
    }

    #[test]
    fn turned_faces_follow_the_box() {
        // The +Z face of a facing-0 part is its front; at facing 1 that face is
        // +X, at 2 it is -Z, at 3 it is -X.
        let p = part([0, 0, 0, 16, 16, 3], [10, 11, 12, 13, 14, 15]);
        assert_eq!(turned(p, 1).tile[0], 14);
        assert_eq!(turned(p, 2).tile[5], 14);
        assert_eq!(turned(p, 3).tile[1], 14);
        assert_eq!(turned(p, 1).tile[2], 12);
    }

    #[test]
    fn closed_door_blocks_and_open_door_leaves_the_doorway() {
        let closed = bstate::door(false, 0, false);
        let open = bstate::door(false, 0, true);
        // A body in the middle of the doorway cell, standing on the floor.
        let (cx, cz) = (32, 32);
        assert!(!body_hits(closed, 0, 0, 0, cx, 0, cz, 12, 56)); // panel z 0..12, body z 20..44
                                                                 // Against the near edge the closed panel stops it...
        assert!(body_hits(closed, 0, 0, 0, 32, 0, 6, 12, 56));
        // ...and the open door does not (the panel is at x 52..64).
        assert!(!body_hits(open, 0, 0, 0, 32, 0, 6, 12, 56));
        assert!(body_hits(open, 0, 0, 0, 56, 0, 32, 12, 56));
    }

    #[test]
    fn slab_and_stairs_match_the_old_boxes() {
        assert_eq!(first(SLAB).b, [0, 0, 0, 16, 8, 16]);
        let mut ps = [EMPTY; MAX_PARTS];
        assert_eq!(parts(STAIRS_N, &mut ps), 2);
        assert_eq!(ps[1].b, [0, 8, 8, 16, 16, 16]);
        assert_eq!(parts(STAIRS_W, &mut ps), 2);
        assert_eq!(ps[1].b, [0, 8, 0, 8, 16, 16]);
    }

    #[test]
    fn a_ray_passes_over_the_top_half_of_a_slab_cell() {
        // Looking along +X at y = 48 (3/4 up): above the slab's 8/16.
        let o = [-100, 48, 32];
        let d = [4096, 0, 0];
        assert_eq!(ray_hit(SLAB, 0, 0, 0, o, d, 400), None);
        let o = [-100, 20, 32];
        assert_eq!(ray_hit(SLAB, 0, 0, 0, o, d, 400), Some(100));
    }

    #[test]
    fn a_ray_inside_a_part_enters_at_zero() {
        let o = [10, 10, 10];
        assert_eq!(ray_hit(SLAB, 0, 0, 0, o, [4096, 0, 0], 400), Some(0));
    }

    #[test]
    fn solid_classes() {
        assert_eq!(solid(bstate::door(true, 2, true)), Solid::Parts);
        assert_eq!(solid(WIRE), Solid::No);
        assert_eq!(solid(bstate::lever(2, true)), Solid::No);
        assert_eq!(solid(FENCE), Solid::Full);
        assert!(is_shape(FENCE) && is_shape(bstate::bed(true, 1)));
    }
}

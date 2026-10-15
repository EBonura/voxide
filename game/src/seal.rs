//! Growing opaque polygons a little on screen so neighbours overlap.
//!
//! A greedy mesh is full of T-junctions: the corner of a small face lands on the
//! edge of a large one, and each corner truncates to whole pixels on its own, so
//! it can sit a pixel or two off the edge the GPU interpolates. Where it lands
//! outside, the clear colour shows through as a hairline. Growing every opaque
//! polygon outward turns the gap into an overlap, which is invisible between
//! faces of the same texture.
//!
//! The grow has to push every EDGE outward. The first version moved each corner
//! two pixels away from the polygon's centre on both axes. That only works when
//! the edges are radial: for a thin sliver running at a slant (a distant floor
//! plate, or any cell near the screen edge) the corners at both ends of a long
//! edge move the same way along it, the edge slides instead of growing, and the
//! hairline stays. Measured on the title-screen fly-over and on a recorded walk,
//! those slanted slivers were every remaining crack.
//!
//! The rule here looks at the polygon's edges instead. Each edge has an outward
//! normal, and the square a corner may move within has a corner of its own in the
//! direction of that normal (the support point: `GROW` pixels on each axis, signed
//! like the normal). A corner takes the support point of its two edges:
//!
//! * where they agree on an axis (or one of them has no say, a normal of zero on
//!   that axis) it moves `GROW` pixels that way;
//! * where they pull opposite ways, which is a sharp corner of a sliver, it moves
//!   `GROW_TIP` pixels toward the LONGER edge, because the long edges are the
//!   ones whose cracks are visible.
//!
//! Only the signs of the edge vectors and the length order of two edges matter,
//! so there is no multiply, no division and no square root. The corner's move is
//! read from an 18-entry table built at compile time. A full offset of the edges
//! (the mitred polygon) needs a division per corner and was measured at +10% of
//! the frame on a recorded walk; on 600 random slanted slivers it leaves exactly
//! as few holes as this does (none), see tools/host-tests/tests/seal.rs.
//!
//! Integer only; no 64-bit values.

/// Pixels a corner moves on an axis where its two edges agree.
pub const GROW: i32 = 3;
/// Pixels a corner moves on an axis where its two edges disagree, toward the
/// longer edge.
pub const GROW_TIP: i32 = 1;
/// Screen rail: the GPU's vertex words are 11-bit signed, so a corner pushed past
/// 1023 would read as -1024 and flip to the far side of the screen. Corners come
/// from the GTE already inside -1024..=1023.
pub const RAIL_X: i32 = 1023;

/// One axis of one corner, from the two edges' signs there: `p` for the first
/// edge, `q` for the second, each -1, 0 or 1; `p_longer` is whether the first is
/// the longer edge.
const fn corner_axis(p: i32, q: i32, p_longer: bool) -> i32 {
    if p == q {
        p * GROW
    } else if p == 0 {
        q * GROW
    } else if q == 0 {
        p * GROW
    } else if p_longer {
        p * GROW_TIP
    } else {
        q * GROW_TIP
    }
}

/// The four corners of a quad on one axis, packed one byte each (v0 in the low
/// byte, then v1, v2, v3) for the quad's two outward normals `p` (of the edge
/// along the first axis, on its far side) and `q` (likewise for the other axis).
/// v0 sits on the near side of both edges, v1 on the far side of `q`, v2 on the
/// far side of `p`, v3 on the far side of both.
const fn pack(p: i32, q: i32, p_longer: bool) -> u32 {
    let d0 = corner_axis(-p, -q, p_longer);
    let d1 = corner_axis(-p, q, p_longer);
    let d2 = corner_axis(p, -q, p_longer);
    let d3 = corner_axis(p, q, p_longer);
    (d0 as u8 as u32) | (d1 as u8 as u32) << 8 | (d2 as u8 as u32) << 16 | (d3 as u8 as u32) << 24
}

/// Indexed by `((p + 1) * 3 + (q + 1)) * 2 + p_longer`.
pub static AXIS: [u32; 18] = {
    let mut t = [0u32; 18];
    let mut p = -1;
    while p <= 1 {
        let mut q = -1;
        while q <= 1 {
            let mut l = 0;
            while l < 2 {
                t[(((p + 1) * 3 + (q + 1)) * 2 + l) as usize] = pack(p, q, l == 1);
                l += 1;
            }
            q += 1;
        }
        p += 1;
    }
    t
};

/// Byte `i` of a packed axis entry, sign-extended.
#[inline(always)]
fn byte(e: u32, i: u32) -> i32 {
    ((e << (24 - 8 * i)) as i32) >> 24
}

#[inline(always)]
fn sgn(v: i32) -> i32 {
    (v > 0) as i32 - (v < 0) as i32
}

/// The signs of a vector's two components, with a component under a quarter of
/// the other counting as zero: an edge within 14 degrees of an axis is axis
/// aligned, and its tiny perpendicular component (a pixel of truncation noise on
/// a long edge) must not pull the corner two ways.
#[inline(always)]
fn signs(x: i32, y: i32) -> (i32, i32) {
    let (ax, ay) = (x.abs(), y.abs());
    let keep_x = -(((ax << 2) >= ay) as i32);
    let keep_y = -(((ay << 2) >= ax) as i32);
    (sgn(x) & keep_x, sgn(y) & keep_y)
}

/// `v` negated when `m` is -1, unchanged when it is 0.
#[inline(always)]
fn neg_if(v: i32, m: i32) -> i32 {
    (v ^ m) - m
}

/// Table index for outward normal signs `p`, `q` and which edge is longer.
#[inline(always)]
fn index(p: i32, q: i32, p_longer: bool) -> usize {
    (((p + 1) * 3 + (q + 1)) * 2 + p_longer as i32) as usize
}

/// Grow the quad with corners `x`, `y` in the GPU's z-order (v0 v1 / v2 v3: v1 - v0
/// and v3 - v2 are one pair of edges, v2 - v0 and v3 - v1 the other).
///
/// The result may lie past the GPU's x rail (see `RAIL_X`); the caller clamps
/// after its own bounding-box test, which it needs anyway.
#[inline(always)]
pub fn grow_quad(x: [i32; 4], y: [i32; 4]) -> ([i32; 4], [i32; 4]) {
    // The two edge vectors, summed over opposite sides so one collapsed edge does
    // not decide the signs.
    let ax = (x[1] - x[0]) + (x[3] - x[2]);
    let ay = (y[1] - y[0]) + (y[3] - y[2]);
    let bx = (x[2] - x[0]) + (x[3] - x[1]);
    let by = (y[2] - y[0]) + (y[3] - y[1]);
    let a_longer = ax.abs() + ay.abs() >= bx.abs() + by.abs();
    let (sax, say) = signs(ax, ay);
    let (sbx, sby) = signs(bx, by);
    // -1 for a negative winding, else 0.
    let m = (ax * by - ay * bx) >> 31;
    // Outward normals of the far-side edges (see `pack`): for a positive winding
    // the one along `a` is (-ay, ax) and the one along `b` is (by, -bx).
    let (px, py) = (neg_if(-say, m), neg_if(sax, m));
    let (qx, qy) = (neg_if(sby, m), neg_if(-sbx, m));
    let ex = AXIS[index(px, qx, a_longer)];
    let ey = AXIS[index(py, qy, a_longer)];
    (
        [
            x[0] + byte(ex, 0),
            x[1] + byte(ex, 1),
            x[2] + byte(ex, 2),
            x[3] + byte(ex, 3),
        ],
        [
            y[0] + byte(ey, 0),
            y[1] + byte(ey, 1),
            y[2] + byte(ey, 2),
            y[3] + byte(ey, 3),
        ],
    )
}

/// Grow a convex polygon of `n >= 3` corners in place. `get(i)` reads corner i
/// and `set(i, x, y)` writes it; every corner is read in its original position
/// even though earlier corners have already been moved. A corner moves by the
/// same rule as in `grow_quad`, from the edge into it and the edge out of it.
#[inline(always)]
pub fn grow_polygon(
    n: usize,
    get: impl Fn(usize) -> (i32, i32),
    mut set: impl FnMut(usize, i32, i32),
) {
    if n < 3 {
        return;
    }
    // Winding from the first three corners.
    let (p0x, p0y) = get(0);
    let (p1x, p1y) = get(1);
    let (p2x, p2y) = get(2);
    let cross = (p1x - p0x) * (p2y - p0y) - (p1y - p0y) * (p2x - p0x);
    if cross == 0 {
        return;
    }
    // For a polygon traversed with positive winding the outward normal of the
    // edge (ex, ey) is (ey, -ex).
    let m = cross >> 31;
    let (mut prev_x, mut prev_y) = get(n - 1);
    let mut i = 0;
    while i < n {
        let (cx, cy) = get(i);
        let (nx, ny) = if i + 1 < n { get(i + 1) } else { (p0x, p0y) };
        let (e1x, e1y) = (cx - prev_x, cy - prev_y);
        let (e2x, e2y) = (nx - cx, ny - cy);
        // Outward normals of the edge into and the edge out of this corner.
        let (s1x, s1y) = signs(e1x, e1y);
        let (s2x, s2y) = signs(e2x, e2y);
        let (n1x, n1y) = (neg_if(s1y, m), neg_if(-s1x, m));
        let (n2x, n2y) = (neg_if(s2y, m), neg_if(-s2x, m));
        let first_longer = e1x.abs() + e1y.abs() >= e2x.abs() + e2y.abs();
        let dx = byte(AXIS[index(n1x, n2x, first_longer)], 3);
        let dy = byte(AXIS[index(n1y, n2y, first_longer)], 3);
        set(i, (cx + dx).clamp(-RAIL_X - 1, RAIL_X), cy + dy);
        prev_x = cx;
        prev_y = cy;
        i += 1;
    }
}

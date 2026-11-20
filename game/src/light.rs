//! Block light: the pools of light that torches, lava, fire and lumistone throw.
//!
//! Java gives a torch light 14 (lava, fire and glowstone 15) and the level falls by one a block,
//! so a torch lights the ground nine blocks out. VoXide keeps three tiers of it, the Java levels
//! 11 and up, 8 and up and 5 and up, and a face carries the tier next to its sky light. The
//! renderer shows the brighter of the two (main.rs `refresh_mat_ccmd`), so a torch lights a cave
//! or a night field and does nothing against a bright noon, as in Java.
//!
//! A tier is a bit per cell, a column of a chunk (16 x 16 of them, and a ring of columns round
//! them, which belong to the next chunks) holding its 64 heights in two `u32`. The three tiers are nested (every cell of tier 3 is in tier 2 and tier 1), so a cell's
//! tier is the number of the three bitmaps that hold it.
//!
//! Two kinds of source:
//!
//! * A torch is stopped by solid blocks, as Java's light is. Its light is worked out once, by a
//!   flood through the cells that let light pass (`reflow`), and kept as a [`FloodBits`], about
//!   200 bytes: for each of the three tiers the cells the light reaches in that many steps, as
//!   one run of bits per column of the octahedron round the torch. A chunk's mesh reads it back
//!   (`apply`) without looking at a single block. The flood is worked out again only when the
//!   torch is placed or a block that lets light through (or stops it) changes within its reach.
//! * Lava, fire and lumistone light by taxicab distance and ignore walls (`flood`, a range of
//!   heights per column). There are hundreds of lava cells in a lake and the glow is the same
//!   from six of them, so these are not flooded cell by cell.
//!
//! Both are clipped to the chunk being meshed and the ring round it, and a source may stand
//! outside it (a torch in the next chunk) at a coordinate below 0 or above 15.
//!
//! No float and no 64-bit integer.

/// Columns a side of the tier maps: the 16 of a chunk and one more all round, so the light a face
/// takes from the cell in front of it is there when that cell is in the next chunk.
pub const SIDE: usize = 18;

/// Columns of the tier maps: index `(z + 1) * 18 + x + 1`, `x` and `z` in -1..=16.
pub const COLS: usize = SIDE * SIDE;

/// Index of the column at chunk-local (`x`, `z`), each in -1..=16.
#[inline(always)]
pub fn col_index(x: i32, z: i32) -> usize {
    ((z + 1) as usize) * SIDE + (x + 1) as usize
}

/// Tiers above none.
pub const TIERS: usize = 3;

/// The reach of a torch (Java level 14) in blocks for tiers 3, 2 and 1.
pub const TORCH_REACH: [i32; TIERS] = [3, 6, 9];

/// The reach of lava, fire and lumistone (Java level 15).
pub const BRIGHT_REACH: [i32; TIERS] = [4, 7, 10];

/// The three bitmaps, tier 1 first: per column, heights 0..32 then 32..64.
pub type Cols = [[[u32; 2]; COLS]; TIERS];

/// Bits `lo..=hi` of a 32-bit word, both inside 0..=31.
#[inline(always)]
fn span(lo: i32, hi: i32) -> u32 {
    (0xFFFF_FFFFu32 >> (31 - hi)) & (0xFFFF_FFFFu32 << lo)
}

/// Heights `lo..=hi` (clipped to 0..=63) of one column.
#[inline(always)]
fn set_heights(col: &mut [u32; 2], lo: i32, hi: i32) -> bool {
    let (lo, hi) = (lo.max(0), hi.min(63));
    if lo > hi {
        return false;
    }
    if lo < 32 {
        col[0] |= span(lo, hi.min(31));
    }
    if hi >= 32 {
        col[1] |= span((lo - 32).max(0), hi - 32);
    }
    true
}

/// Set every cell within reach of the source at chunk-local (`sx`, `sy`, `sz`), walls ignored.
/// `reach` runs tier 3 first, innermost to outermost. Returns whether any cell was set.
pub fn flood(cols: &mut Cols, sx: i32, sy: i32, sz: i32, reach: &[i32; TIERS]) -> bool {
    let outer = reach[TIERS - 1];
    let mut any = false;
    let mut dz = -outer;
    while dz <= outer {
        let z = sz + dz;
        if (-1..=16).contains(&z) {
            let rem = outer - dz.abs();
            let mut dx = -rem;
            while dx <= rem {
                let x = sx + dx;
                if (-1..=16).contains(&x) {
                    let col = col_index(x, z);
                    let flat = dx.abs() + dz.abs();
                    let mut t = 0;
                    while t < TIERS {
                        // Tier t + 1 is the (TIERS - 1 - t)th entry of `reach`.
                        let r = reach[TIERS - 1 - t] - flat;
                        if r >= 0 {
                            any |= set_heights(&mut cols[t][col], sy - r, sy + r);
                        }
                        t += 1;
                    }
                }
                dx += 1;
            }
        }
        dz += 1;
    }
    any
}

/// The tier of a chunk-local cell, 0 (none) to 3 (brightest). `lx` and `lz` may be -1 or 16 (the
/// ring), `ly` anything: a height off the world has no light.
#[inline(always)]
pub fn tier(cols: &Cols, lx: i32, ly: i32, lz: i32) -> u16 {
    if ly as u32 >= 64 {
        return 0;
    }
    let col = col_index(lx, lz);
    let (word, bit) = ((ly >> 5) as usize, (ly & 31) as u32);
    // Tier 1 holds every lit cell: most cells are not, and cost one read.
    if (cols[0][col][word] >> bit) & 1 == 0 {
        return 0;
    }
    1 + ((cols[1][col][word] >> bit) & 1) as u16 + ((cols[2][col][word] >> bit) & 1) as u16
}

/// Does a source at this chunk-local position reach any cell of the chunk or the ring of columns
/// round it? `x` and `z` may be outside 0..=15 (a source in a neighbouring chunk).
pub fn reaches_chunk(x: i32, z: i32, reach: &[i32; TIERS]) -> bool {
    let ex = if x < -1 {
        -1 - x
    } else if x > 16 {
        x - 16
    } else {
        0
    };
    let ez = if z < -1 {
        -1 - z
    } else if z > 16 {
        z - 16
    } else {
        0
    };
    ex + ez <= reach[TIERS - 1]
}

// ---- A torch stopped by walls -------------------------------------------------------------------

/// The reach of a torch's flood in steps through cells that let light pass.
pub const FLOOD_R: i32 = 9;

/// Side of the grid the flood works on: the reach either side of the torch, and one cell of
/// border so a neighbour read never leaves it.
const GRID: usize = 2 * FLOOD_R as usize + 3;

/// Cells the light may pass, for the torch at the middle: a word per column `(dx, dz)` of the
/// 21 x 21 grid (index `(dx + 10) * 21 + dz + 10`), bit `dy + 9` set when the cell at that
/// offset lets light through. The border and the columns outside the octahedron stay 0.
pub type OpenMap = [u32; GRID * GRID];

/// Index of column (`dx`, `dz`) in an [`OpenMap`].
#[inline(always)]
pub fn open_index(dx: i32, dz: i32) -> usize {
    ((dx + FLOOD_R + 1) as usize) * GRID + (dz + FLOOD_R + 1) as usize
}

/// Bits a column of the flood can hold: heights -9..=9 round the torch.
const COLUMN_BITS: u32 = 0x7_FFFF;

/// Bits of a [`FloodBits`]: the cells of the octahedron of radius 3, 6 and 9,
/// (2r + 1)(2r² + 2r + 3) / 3 of each: 63 + 377 + 1,159.
const FLOOD_BITS: usize = 63 + 377 + 1159;

/// Words of a [`FloodBits`].
pub const FLOOD_WORDS: usize = FLOOD_BITS.div_ceil(32);

/// The light of one torch, as the cells it reaches through open cells. Three runs of bits, for
/// the reach 3, 6 and 9 (tier 3 first), each the octahedron of that radius column by column
/// (`dx` from -r, `dz` from -(r - |dx|)); a column is `2 (r - |dx| - |dz|) + 1` bits wide, bit
/// `j` for the height `dy = j - (r - |dx| - |dz|)` from the torch. Every cell of a smaller tier
/// is in the larger ones.
pub type FloodBits = [u32; FLOOD_WORDS];

/// The reach of each run of a [`FloodBits`], in the order they are stored.
const RUNS: [i32; TIERS] = [3, 6, 9];

/// Bit offset of the run for radius `r` of `RUNS`: the sum of the cells of the smaller ones.
const fn run_start(k: usize) -> usize {
    let mut at = 0;
    let mut i = 0;
    while i < k {
        let r = RUNS[i] as usize;
        at += (2 * r + 1) * (2 * r * r + 2 * r + 3) / 3;
        i += 1;
    }
    at
}

/// Cells of a slice of the octahedron of radius `r` at `|dx|`: `2m² + 2m + 1` with `m = r - |dx|`.
#[inline(always)]
fn slice_len(m: i32) -> usize {
    (2 * m * m + 2 * m + 1) as usize
}

/// Bits of the columns of the slice of margin `m` that come before the column `dz`.
#[inline(always)]
fn column_start(m: i32, dz: i32) -> usize {
    if dz <= 0 {
        ((dz + m) * (dz + m)) as usize
    } else {
        (m * m + dz * (2 * m + 2 - dz)) as usize
    }
}

#[inline(always)]
fn put_bits(out: &mut FloodBits, at: usize, v: u32, w: u32) {
    let (word, bit) = (at >> 5, (at & 31) as u32);
    out[word] |= v << bit;
    if bit + w > 32 {
        out[word + 1] |= v >> (32 - bit);
    }
}

#[inline(always)]
fn get_bits(bits: &FloodBits, at: usize, w: u32) -> u32 {
    let (word, bit) = (at >> 5, (at & 31) as u32);
    let mut v = bits[word] >> bit;
    if bit + w > 32 {
        v |= bits[word + 1] << (32 - bit);
    }
    v & ((1u32 << w) - 1)
}

/// Write the cells lit after the steps so far (`lit`, in the grid of an [`OpenMap`]) as the run
/// of radius `r` starting at bit `at`.
fn pack_run(lit: &[u32; GRID * GRID], r: i32, out: &mut FloodBits, mut at: usize) {
    let mut dx = -r;
    while dx <= r {
        let m = r - dx.abs();
        let mut dz = -m;
        while dz <= m {
            let half = r - dx.abs() - dz.abs();
            let w = (2 * half + 1) as u32;
            let v = (lit[open_index(dx, dz)] >> (FLOOD_R - half)) & ((1u32 << w) - 1);
            put_bits(out, at, v, w);
            at += w as usize;
            dz += 1;
        }
        dx += 1;
    }
}

/// The working space of [`reflow`]: the cells lit so far, laid out as an [`OpenMap`].
pub type Work = [u32; GRID * GRID];

/// Work out the light of a torch: flood from it through the cells `open` lets light pass, one
/// step at a time (a step is a cell to a neighbour on a face, as Java's light falls a level a
/// block), and keep what the first 3, 6 and 9 steps reach. A column of heights is a word, so a
/// step is a few operations a column and not a visit a cell. `work` is overwritten; it lives
/// outside the function because the stream mesher runs on a scratchpad stack of one kilobyte.
pub fn reflow(open: &OpenMap, work: &mut Work, out: &mut FloodBits) {
    *out = [0; FLOOD_WORDS];
    *work = [0; GRID * GRID];
    work[open_index(0, 0)] = 1 << FLOOD_R;
    let mut step = 1;
    while step <= FLOOD_R {
        // The slabs are done in order of `dx` and the lit cells only grow, so a slab reads the
        // old cells of the one before it (kept in `prev`), its own old cells (`cur`) and the
        // untouched next slab.
        let mut prev = [0u32; GRID];
        let first = open_index(-step - 1, -FLOOD_R - 1);
        prev.copy_from_slice(&work[first..first + GRID]);
        let mut dx = -step;
        while dx <= step {
            let base = open_index(dx, -FLOOD_R - 1);
            let mut cur = [0u32; GRID];
            cur.copy_from_slice(&work[base..base + GRID]);
            let m = step - dx.abs();
            let mut dz = -m;
            while dz <= m {
                let c = open_index(dx, dz);
                let i = (dz + FLOOD_R + 1) as usize;
                let v = cur[i];
                let n = v | v << 1 | v >> 1 | prev[i] | work[c + GRID] | cur[i - 1] | cur[i + 1];
                work[c] = n & open[c] & COLUMN_BITS;
                dz += 1;
            }
            prev = cur;
            dx += 1;
        }
        let mut k = 0;
        while k < TIERS {
            if step == RUNS[k] {
                pack_run(work, step, out, run_start(k));
            }
            k += 1;
        }
        step += 1;
    }
}

/// Or the heights `v` (bit `j` is height `base + j`) into a column, clipping to 0..=63.
#[inline(always)]
fn or_heights(col: &mut [u32; 2], mut v: u32, mut base: i32) {
    if base < 0 {
        v = if base <= -32 { 0 } else { v >> (-base) as u32 };
        base = 0;
    }
    if v == 0 || base > 63 {
        return;
    }
    if base < 32 {
        let b = base as u32;
        col[0] |= v << b;
        if b > 0 {
            col[1] |= v >> (32 - b);
        }
    } else {
        col[1] |= v << (base - 32) as u32;
    }
}

/// Or the light of a torch worked out by [`reflow`] into the tier maps of a chunk, the torch at
/// chunk-local (`sx`, `sy`, `sz`), coordinates that may lie outside the chunk. Returns whether
/// any cell was set.
pub fn apply(cols: &mut Cols, bits: &FloodBits, sx: i32, sy: i32, sz: i32) -> bool {
    let mut any = false;
    let mut k = 0;
    while k < TIERS {
        let r = RUNS[k];
        // RUNS holds tier 3 first, and so does the last of `cols`.
        let target = TIERS - 1 - k;
        let mut at = run_start(k);
        let mut dx = -r;
        while dx <= r {
            let m = r - dx.abs();
            let x = sx + dx;
            if !(-1..=16).contains(&x) {
                at += slice_len(m);
                dx += 1;
                continue;
            }
            let mut dz = -m;
            while dz <= m {
                let half = m - dz.abs();
                let w = (2 * half + 1) as u32;
                let z = sz + dz;
                if (-1..=16).contains(&z) {
                    let v = get_bits(bits, at, w);
                    if v != 0 {
                        or_heights(&mut cols[target][col_index(x, z)], v, sy - half);
                        any = true;
                    }
                }
                at += w as usize;
                dz += 1;
            }
            dx += 1;
        }
        k += 1;
    }
    any
}

/// The tier (0 to 3) a torch's light gives the cell at offset (`dx`, `dy`, `dz`) from it.
pub fn flood_tier(bits: &FloodBits, dx: i32, dy: i32, dz: i32) -> u16 {
    let flat = dx.abs() + dz.abs();
    let mut k = 0;
    while k < TIERS {
        let r = RUNS[k];
        let half = r - flat;
        if half >= 0 && dy.abs() <= half {
            // The offset of the column inside the run: the slices before `dx`, then the columns
            // of this slice before `dz`.
            let mut at = run_start(k);
            let mut x = -r;
            while x < dx {
                at += slice_len(r - x.abs());
                x += 1;
            }
            at += column_start(r - dx.abs(), dz);
            if get_bits(bits, at, (2 * half + 1) as u32) >> (dy + half) & 1 != 0 {
                // Tier 3 is the first run and the highest tier; each later run is one lower.
                return (TIERS - k) as u16;
            }
        }
        k += 1;
    }
    0
}

/// `a + (b - a) * t / 255` for `a` and `b` in 0..=255 and `t` in 0..=255, the way the renderer
/// ramps a face's colour into the fog: the same value as the division, quotient rounded toward
/// zero, with the division by 255 done as shifts.
#[inline(always)]
pub fn mix_fog(a: u8, b: i32, t: i32) -> u32 {
    let d = b - a as i32;
    let p = (d.abs() * t) as u32;
    let q = ((p + 1 + (p >> 8)) >> 8) as i32;
    (a as i32 + if d < 0 { -q } else { q }) as u32
}

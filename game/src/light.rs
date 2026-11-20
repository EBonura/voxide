//! Block light: the pools of light that torches, lava, fire and lumistone throw.
//!
//! Java gives a torch light 14 (lava, fire and glowstone 15) and the level falls by one a block,
//! so a torch lights the ground nine blocks out. VoXide keeps three tiers of it, the Java levels
//! 11 and up, 8 and up and 5 and up, and a face carries the tier next to its sky light. The
//! renderer shows the brighter of the two (main.rs `refresh_mat_ccmd`), so a torch lights a cave
//! or a night field and does nothing against a bright noon, as in Java.
//!
//! A tier is a bit per cell, a row of a chunk's 16 columns in one `u16`. The three tiers are
//! nested (every cell of tier 3 is in tier 2 and tier 1), so a cell's tier is the number of the
//! three bitmaps that hold it. Flooding a source is then three range masks per row of the volume
//! it reaches, about 180 rows, not a write per cell. The reach is the taxicab distance and
//! ignores walls, as the mob-spawn light rule in main.rs does; it is clipped to the chunk being
//! meshed, and a source may stand outside it (a torch in the next chunk) at a coordinate below 0
//! or above 15.
//!
//! No float and no 64-bit integer.

/// Rows in a chunk's tier map: 64 layers of 16 columns.
pub const ROWS: usize = 64 * 16;

/// Tiers above none.
pub const TIERS: usize = 3;

/// The reach of a torch (Java level 14) in blocks of taxicab distance for tiers 3, 2 and 1.
pub const TORCH_REACH: [i32; TIERS] = [3, 6, 9];

/// The reach of lava, fire and lumistone (Java level 15).
pub const BRIGHT_REACH: [i32; TIERS] = [4, 7, 10];

/// The three bitmaps, tier 1 first.
pub type Rows = [[u16; ROWS]; TIERS];

/// Bits `lo..=hi` of a 16-bit row, both inside 0..=15.
#[inline(always)]
fn span(lo: i32, hi: i32) -> u16 {
    ((0xFFFFu32 >> (15 - hi)) & (0xFFFFu32 << lo)) as u16
}

/// Set every cell within reach of the source at chunk-local (`sx`, `sy`, `sz`). `reach` runs tier
/// 3 first, innermost to outermost. Returns whether any cell was set.
pub fn flood(rows: &mut Rows, sx: i32, sy: i32, sz: i32, reach: &[i32; TIERS]) -> bool {
    let outer = reach[TIERS - 1];
    let mut any = false;
    let mut dy = -outer;
    while dy <= outer {
        let y = sy + dy;
        if (0..64).contains(&y) {
            let rem_y = outer - dy.abs();
            let mut dz = -rem_y;
            while dz <= rem_y {
                let z = sz + dz;
                if (0..16).contains(&z) {
                    let row = (y * 16 + z) as usize;
                    let flat = dy.abs() + dz.abs();
                    let mut t = 0;
                    while t < TIERS {
                        // Tier t + 1 is the (TIERS - 1 - t)th entry of `reach`.
                        let r = reach[TIERS - 1 - t] - flat;
                        if r >= 0 {
                            let lo = (sx - r).max(0);
                            let hi = (sx + r).min(15);
                            if lo <= hi {
                                rows[t][row] |= span(lo, hi);
                                any = true;
                            }
                        }
                        t += 1;
                    }
                }
                dz += 1;
            }
        }
        dy += 1;
    }
    any
}

/// The tier of a chunk-local cell, 0 (none) to 3 (brightest).
#[inline(always)]
pub fn tier(rows: &Rows, lx: usize, ly: usize, lz: usize) -> u16 {
    let row = ly * 16 + lz;
    // Tier 1 holds every lit cell: most cells are not, and cost one read.
    if (rows[0][row] >> lx) & 1 == 0 {
        return 0;
    }
    1 + ((rows[1][row] >> lx) & 1) + ((rows[2][row] >> lx) & 1)
}

/// Does a source at this chunk-local position reach any cell of the chunk? `x` and `z` may be
/// outside 0..=15 (a source in a neighbouring chunk).
pub fn reaches_chunk(x: i32, z: i32, reach: &[i32; TIERS]) -> bool {
    let ex = if x < 0 {
        -x
    } else if x > 15 {
        x - 15
    } else {
        0
    };
    let ez = if z < 0 {
        -z
    } else if z > 15 {
        z - 15
    } else {
        0
    };
    ex + ez <= reach[TIERS - 1]
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

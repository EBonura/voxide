//! Java's explosion rules (minecraft.wiki/w/Explosion), with no access to the
//! world, in integers.
//!
//! Block damage: 1,352 rays leave the centre toward the surface points of a
//! 16 x 16 x 16 grid. A ray starts at power x (0.7 to 1.3), walks 0.3 of a
//! block at a time, loses (blast resistance + 0.3) x 0.3 inside a block and
//! 0.225 on every step, and breaks every block it still has strength left in.
//! Entity damage and knockback come from the distance to the centre and from
//! how much of the entity the centre can see.
//!
//! Intensities are in thousandths and resistances in hundredths, positions in
//! 1/65536 of a block, so a step is a handful of adds. Plain integer code with
//! no dependencies, like units.rs; the host tests (tools/host-tests/tests/
//! fire_blast.rs) run its #[cfg(test)] module.

/// Rays a full explosion casts: the surface of the 16^3 grid.
#[allow(dead_code)]
pub const RAYS: u32 = 1352;
/// Grid points, indexed 0..4096; only the 1,352 on the surface cast a ray.
pub const GRID_POINTS: u32 = 16 * 16 * 16;
/// A block the ray found nothing in, as `trace`'s resistance probe answers it.
pub const NO_BLOCK: i32 = -1;
/// The cells a blast can reach either side of its centre cell: a ray of
/// power 6 (a charged sapper) at the strongest roll, 7.8, lasts 34 steps of
/// 0.3 of a block, 10.4 blocks.
pub const REACH_CELLS: i32 = 11;
pub const SIDE: usize = (2 * REACH_CELLS + 1) as usize;
pub const CELLS: usize = SIDE * SIDE * SIDE;
pub const WORDS: usize = (CELLS + 31) / 32;
/// The strongest explosion the cube holds.
pub const MAX_POWER: i32 = 6;

/// Thousandths of intensity lost per step of air.
const STEP_LOSS: i32 = 225;

/// A hash to a roll, for the rays' 0.7 to 1.3.
pub fn mix(seed: u32, k: u32) -> u32 {
    let mut h = seed ^ k.wrapping_mul(0x9E37_79B1);
    h ^= h >> 15;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    h = h.wrapping_mul(0xC2B2_AE35);
    h ^ (h >> 16)
}

/// Integer square root, rounded down.
pub fn isqrt(n: u32) -> u32 {
    if n < 2 {
        return n;
    }
    let mut x = n;
    let mut y = (x + 1) >> 1;
    while y < x {
        x = y;
        y = (x + n / x) >> 1;
    }
    x
}

/// The direction of grid point `k` as a step of 0.3 of a block, Q16 a step
/// per axis, or `None` for the interior points that cast no ray.
pub fn ray_step(k: u32) -> Option<(i32, i32, i32)> {
    let (gx, gy, gz) = ((k & 15) as i32, ((k >> 4) & 15) as i32, (k >> 8) as i32);
    if gx != 0 && gx != 15 && gy != 0 && gy != 15 && gz != 0 && gz != 15 {
        return None;
    }
    // The point's offset from the cube's centre, in halves: odd, -15 to 15.
    let (vx, vy, vz) = (2 * gx - 15, 2 * gy - 15, 2 * gz - 15);
    // The length times 256, for the rounding.
    let len = isqrt(((vx * vx + vy * vy + vz * vz) as u32) << 16) as i32;
    // 0.3 of a block is 19,661 in Q16.
    const STEP: i32 = 19_661;
    Some((
        vx * 256 * STEP / len,
        vy * 256 * STEP / len,
        vz * 256 * STEP / len,
    ))
}

/// A ray's starting strength in thousandths, power x (0.7 + 0.6 r).
pub fn ray_power(power: i32, seed: u32, k: u32) -> i32 {
    power * (700 + (mix(seed, k) % 600) as i32)
}

/// The cells blast damage reaches: a bit set over the cube of cells around the
/// centre cell, so a ray sees the world as it was when the blast went off.
/// A column (fixed x and z) is 23 consecutive bits, y fastest.
pub struct Cube {
    bits: [u32; WORDS + 1],
}

impl Cube {
    pub const fn new() -> Cube {
        Cube { bits: [0; WORDS + 1] }
    }

    pub fn clear(&mut self) {
        self.bits = [0; WORDS + 1];
    }

    /// The cell's index: columns in z then x, y fastest within one.
    #[inline(always)]
    pub fn index(dx: i32, dy: i32, dz: i32) -> usize {
        let r = REACH_CELLS;
        (((dz + r) as usize * SIDE) + (dx + r) as usize) * SIDE + (dy + r) as usize
    }

    #[inline(always)]
    pub fn contains_cell(dx: i32, dy: i32, dz: i32) -> bool {
        let r = REACH_CELLS;
        dx >= -r && dx <= r && dy >= -r && dy <= r && dz >= -r && dz <= r
    }

    /// Mark a cell; false for one outside the cube.
    pub fn insert(&mut self, dx: i32, dy: i32, dz: i32) -> bool {
        if !Self::contains_cell(dx, dy, dz) {
            return false;
        }
        let i = Self::index(dx, dy, dz);
        self.bits[i >> 5] |= 1 << (i & 31);
        true
    }

    #[allow(dead_code)]
    pub fn get(&self, dx: i32, dy: i32, dz: i32) -> bool {
        if !Self::contains_cell(dx, dy, dz) {
            return false;
        }
        let i = Self::index(dx, dy, dz);
        self.bits[i >> 5] >> (i & 31) & 1 != 0
    }

    pub fn count(&self) -> u32 {
        let mut n = 0;
        let mut w = 0;
        while w < WORDS {
            n += self.bits[w].count_ones();
            w += 1;
        }
        n
    }

    /// The 23 bits of column number `col` (z-major, then x: `(dz + 11) * 23 +
    /// dx + 11`), bit k being height offset k - 11.
    pub fn column_mask(&self, col: usize) -> u32 {
        let i = col * SIDE;
        let (w, sh) = (i >> 5, i & 31);
        let lo = self.bits[w] >> sh;
        let hi = if sh == 0 { 0 } else { self.bits[w + 1] << (32 - sh) };
        (lo | hi) & ((1 << SIDE) - 1)
    }
}

/// What a ray reads and writes: the blocks around the centre by cell index
/// (a byte each), the blast resistance in hundredths by block id (`NO_BLOCK`
/// for air), and the set of cells broken.
pub struct Scene<'a> {
    pub snap: &'a [u8; CELLS],
    pub res: &'a [i32; 256],
    pub cube: &'a mut Cube,
}

/// One ray from the centre. `frac` is where the centre sits inside its cell
/// (Q16, 0 to 65535 an axis), `step` the ray's step, `power_k` its starting
/// strength in thousandths. Every cell the ray still had strength to break is
/// marked in the scene's cube. Consecutive steps in one cell look it up once.
pub fn trace(scene: &mut Scene, frac: (i32, i32, i32), step: (i32, i32, i32), power_k: i32) {
    let (mut px, mut py, mut pz) = frac;
    let mut left = power_k;
    let mut last = i32::MAX;
    let mut res = NO_BLOCK;
    let mut idx = usize::MAX;
    while left > 0 {
        let (cx, cy, cz) = (px >> 16, py >> 16, pz >> 16);
        let key = (cx + 16) | ((cy + 16) << 6) | ((cz + 16) << 12);
        if key != last {
            last = key;
            if Cube::contains_cell(cx, cy, cz) {
                idx = Cube::index(cx, cy, cz);
                res = scene.res[scene.snap[idx] as usize];
            } else {
                idx = usize::MAX;
                res = NO_BLOCK;
            }
        }
        if res != NO_BLOCK {
            left -= (res + 30) * 3;
            if left > 0 && idx != usize::MAX {
                scene.cube.bits[idx >> 5] |= 1 << (idx & 31);
            }
        }
        left -= STEP_LOSS;
        px += step.0;
        py += step.1;
        pz += step.2;
    }
}

/// An entity's damage from a blast, Java's on Normal: with `impact` the
/// Q8 product of (1 - distance / (2 x power)) and the exposure,
/// 7 x power x (impact^2 + impact) + 1.
pub fn damage_from_impact(power: i32, impact_q8: i32) -> i32 {
    ((7 * power * (impact_q8 * impact_q8 + impact_q8 * 256)) >> 16) + 1
}

/// The Q8 (1 - distance / reach) of an entity `dist` world units from the
/// centre of a blast of `power`, or `None` out of reach. 64 units a block.
pub fn proximity_q8(power: i32, dist: i32) -> Option<i32> {
    let reach = 2 * power * 64;
    if dist >= reach {
        None
    } else {
        Some(256 - dist * 256 / reach)
    }
}

/// How much of a box centred on a column the blast's centre can see, Q8,
/// 0 to 256: the share of six sample points (three heights by two spots
/// across) with a clear line to it. Java samples a grid across the whole box;
/// this keeps the shape (a wall between you and the blast shields you, cover
/// at one height protects that share) at a fraction of the rays. Units are
/// world units, 64 to a block; `solid` answers whether a cell stops sight.
pub fn exposure_q8<S: FnMut(i32, i32, i32) -> bool>(
    centre: (i32, i32, i32),
    entity: (i32, i32, i32),
    half_w: i32,
    height: i32,
    solid: &mut S,
) -> i32 {
    let mut seen = 0;
    let heights = [height * 15 / 100, height / 2, height * 85 / 100];
    let spots = [(half_w / 2, half_w / 2), (-half_w / 2, -half_w / 2)];
    let mut h = 0;
    while h < 3 {
        let mut s = 0;
        while s < 2 {
            let p = (entity.0 + spots[s].0, entity.1 + heights[h], entity.2 + spots[s].1);
            if clear_line(centre, p, solid) {
                seen += 1;
            }
            s += 1;
        }
        h += 1;
    }
    seen * 256 / 6
}

/// Is the straight line between two points (world units) free of solid
/// cells? Walked in 24 unit steps, three eighths of a block.
pub fn clear_line<S: FnMut(i32, i32, i32) -> bool>(
    a: (i32, i32, i32),
    b: (i32, i32, i32),
    solid: &mut S,
) -> bool {
    let (dx, dy, dz) = (b.0 - a.0, b.1 - a.1, b.2 - a.2);
    let far = dx.abs().max(dy.abs()).max(dz.abs());
    let n = far / 24 + 1;
    let mut i = 1;
    // The last sample is the entity's own cell, which the blast may stand in.
    while i < n {
        let (x, y, z) = (a.0 + dx * i / n, a.1 + dy * i / n, a.2 + dz * i / n);
        if solid(x.div_euclid(64), y.div_euclid(64), z.div_euclid(64)) {
            return false;
        }
        i += 1;
    }
    true
}

/// A blast's knockback on an entity: the unit direction from the centre to
/// it times the impact, as Q8 blocks per Java tick, from the offset in world
/// units. Zero at the centre.
pub fn knockback_q8(dx: i32, dy: i32, dz: i32, impact_q8: i32) -> (i32, i32, i32) {
    let len = isqrt((dx * dx + dy * dy + dz * dz) as u32) as i32;
    if len == 0 {
        return (0, 0, 0);
    }
    (
        dx * impact_q8 / len,
        dy * impact_q8 / len,
        dz * impact_q8 / len,
    )
}

/// A block's chance to drop what it is: Java keeps each item of a TNT or
/// creeper blast with chance 1 / power (the explosion decay condition).
pub fn drops(seed: u32, k: u32, power: i32) -> bool {
    mix(seed, k) % (power as u32) == 0
}

/// A fresh fuse, 80 game ticks (4 s, minecraft.wiki/w/TNT).
pub const FUSE_JAVA_TICKS: i32 = 80;

/// Primed TNT flashes white for 5 game ticks in every 10, from the moment it
/// is lit. `fuse` is the sim ticks left (3 to a game tick).
pub fn flashing(fuse: u32) -> bool {
    (fuse / 15) % 2 == 0
}

/// A fuse in game ticks for TNT a blast primed: 10 to 29.
pub fn chain_fuse_ticks(seed: u32, k: u32) -> i32 {
    10 + (mix(seed, k) % 20) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_rays() {
        let mut n = 0;
        let mut k = 0;
        while k < GRID_POINTS {
            if ray_step(k).is_some() {
                n += 1;
            }
            k += 1;
        }
        assert_eq!(n, RAYS);
    }

    #[test]
    fn steps_are_a_third_of_a_block() {
        let mut k = 0;
        while k < GRID_POINTS {
            if let Some((x, y, z)) = ray_step(k) {
                let len = isqrt((x as i64 * x as i64 + y as i64 * y as i64 + z as i64 * z as i64) as u32);
                // 0.3 of a block, 19,661 in Q16, within a part in a hundred.
                assert!((len as i32 - 19_661).abs() < 200, "k={k} len={len}");
            }
            k += 1;
        }
    }

    #[test]
    fn cube_roundtrip() {
        let mut c = Cube::new();
        assert!(c.insert(0, 0, 0));
        assert!(c.insert(-REACH_CELLS, REACH_CELLS, 3));
        assert!(!c.insert(REACH_CELLS + 1, 0, 0));
        assert!(c.get(0, 0, 0) && c.get(-REACH_CELLS, REACH_CELLS, 3) && !c.get(1, 0, 0));
        assert_eq!(c.count(), 2);
        // Every column's mask holds exactly its own cells, whatever the word
        // boundaries do.
        let mut seen = 0;
        for dz in -REACH_CELLS..=REACH_CELLS {
            for dx in -REACH_CELLS..=REACH_CELLS {
                let col = ((dz + REACH_CELLS) as usize) * SIDE + (dx + REACH_CELLS) as usize;
                let m = c.column_mask(col);
                for dy in -REACH_CELLS..=REACH_CELLS {
                    let bit = m >> (dy + REACH_CELLS) & 1 != 0;
                    assert_eq!(bit, c.get(dx, dy, dz));
                    seen += bit as i32;
                }
            }
        }
        assert_eq!(seen, 2);
    }

    #[test]
    fn damage_matches_the_wiki_table() {
        // Point blank, impact 1.0: TNT (power 4) 57, a creeper (3) 43.
        assert_eq!(damage_from_impact(4, 256), 57);
        assert_eq!(damage_from_impact(3, 256), 43);
        // Out of reach beyond 2 x power blocks.
        assert_eq!(proximity_q8(4, 8 * 64), None);
        assert_eq!(proximity_q8(4, 0), Some(256));
        assert_eq!(proximity_q8(4, 4 * 64), Some(128));
    }
}

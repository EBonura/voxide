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
pub const fn isqrt(n: u32) -> u32 {
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
pub const fn ray_step(k: u32) -> Option<(i32, i32, i32)> {
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

/// One ray of the table: the grid point it leaves through (the hash key of its
/// strength roll) and its step per axis, Q16 of a block. A step is under
/// 19,661, so 16 bits hold it.
#[derive(Copy, Clone)]
pub struct RayDir {
    pub k: u16,
    pub sx: i16,
    pub sy: i16,
    pub sz: i16,
}

/// The 1,352 rays in grid order, worked out when the game is compiled. Every
/// blast used to find each direction again: a square root by Newton's method
/// (some fifteen divides, 36 cycles each on the R3000) and three more
/// divides, a third of the whole ray stage.
pub static RAY_DIRS: [RayDir; RAYS as usize] = ray_dirs();

const fn ray_dirs() -> [RayDir; RAYS as usize] {
    let mut out = [RayDir { k: 0, sx: 0, sy: 0, sz: 0 }; RAYS as usize];
    let mut k = 0;
    let mut n = 0;
    while k < GRID_POINTS {
        if let Some((x, y, z)) = ray_step(k) {
            out[n] = RayDir { k: k as u16, sx: x as i16, sy: y as i16, sz: z as i16 };
            n += 1;
        }
        k += 1;
    }
    out
}

/// The most steps any ray of a blast of `power` can take: its strongest roll
/// (1.299) over the 0.225 every step costs, and one more for the first.
pub const fn max_steps(power: i32) -> i32 {
    power * 1299 / STEP_LOSS + 1
}

/// How many cells either side of the centre cell a blast of `power` can reach
/// at all: a step is 0.3 of a block, and the centre sits anywhere in its cell.
pub const fn reach_of(power: i32) -> i32 {
    let r = (max_steps(power) * 3 + 9) / 10 + 1;
    if r > REACH_CELLS {
        REACH_CELLS
    } else {
        r
    }
}

/// Bit `dy + REACH_CELLS` is set when the snapshot column of cells `dy` = -11
/// to 11 about the centre holds anything but air. `column` is the 23 cells of
/// one column, lowest first.
pub fn column_layers(column: &[u8]) -> u32 {
    let mut m = 0u32;
    let mut k = 0;
    // Four cells a look: most are all air, and a word of blocks is sorted into
    // its four flags by one multiply.
    while k + 4 <= column.len() {
        let w = u32::from_le_bytes([column[k], column[k + 1], column[k + 2], column[k + 3]]);
        if w != 0 {
            // The top bit of each byte that is not zero, down to bit 0 of each.
            let hi = ((((w & 0x7F7F_7F7F) + 0x7F7F_7F7F) | w) & 0x8080_8080) >> 7;
            m |= (hi.wrapping_mul(0x1020_4080) >> 28) << k;
        }
        k += 4;
    }
    while k < column.len() {
        if column[k] != 0 {
            m |= 1 << k;
        }
        k += 1;
    }
    m
}

/// Can a ray of this height step (Q16 of a block an axis step, up positive) from a
/// centre `fy` up Q16 into its cell, taking at most `steps` steps, stay within
/// layers that `layers` (the OR of every reachable column's `column_layers`)
/// says hold nothing but air? Such a ray marks no cell, so it need not be
/// walked. The walk is monotonic in height, so only its two ends count.
pub fn band_is_air(layers: u32, fy: i32, sy: i32, steps: i32) -> bool {
    let c = REACH_CELLS;
    let end = (fy + steps * sy) >> 16;
    let (lo, hi) = if sy >= 0 { (0, end) } else { (end, 0) };
    let lo = (lo.max(-c) + c) as u32;
    let hi = (hi.min(c) + c) as u32;
    let band = (u32::MAX >> (31 - hi)) & (u32::MAX << lo);
    layers & band == 0
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
///
/// A step is under a third of a cell, so a ray changes cell by at most one
/// on each axis at a time and the cell's index follows by adding the three
/// strides, where it used to be rebuilt with two multiplies (the largest
/// single cost of the ray stage: the R3000 stalls on each). A ray is a
/// straight line, so one that leaves the cube never returns and can stop.
#[inline(never)]
pub fn trace(scene: &mut Scene, frac: (i32, i32, i32), step: (i32, i32, i32), power_k: i32) {
    if power_k <= 0 {
        return;
    }
    // The three tables as locals: through `scene` each look was a load first.
    let snap = scene.snap;
    let table = scene.res;
    let bits = &mut scene.cube.bits;
    let (mut px, mut py, mut pz) = frac;
    let (mut cx, mut cy, mut cz) = (px >> 16, py >> 16, pz >> 16);
    if !Cube::contains_cell(cx, cy, cz) {
        return;
    }
    let mut left = power_k;
    let mut idx = Cube::index(cx, cy, cz);
    let mut res = table[snap[idx] as usize];
    loop {
        if res != NO_BLOCK {
            left -= (res + 30) * 3;
            if left > 0 {
                bits[idx >> 5] |= 1 << (idx & 31);
            }
        }
        left -= STEP_LOSS;
        if left <= 0 {
            return;
        }
        px += step.0;
        py += step.1;
        pz += step.2;
        let (nx, ny, nz) = (px >> 16, py >> 16, pz >> 16);
        if nx != cx || ny != cy || nz != cz {
            const R: i32 = REACH_CELLS;
            const SIDE_I: i32 = SIDE as i32;
            if (nx + R) as u32 > 2 * R as u32 || (ny + R) as u32 > 2 * R as u32 || (nz + R) as u32 > 2 * R as u32 {
                return;
            }
            idx = (idx as i32 + (nx - cx) * SIDE_I + (ny - cy) + (nz - cz) * SIDE_I * SIDE_I) as usize;
            cx = nx;
            cy = ny;
            cz = nz;
            res = table[snap[idx] as usize];
        }
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
    fn ray_table_is_the_rays_in_grid_order() {
        let mut n = 0;
        let mut k = 0;
        while k < GRID_POINTS {
            if let Some((x, y, z)) = ray_step(k) {
                let d = RAY_DIRS[n];
                assert_eq!((d.k as u32, d.sx as i32, d.sy as i32, d.sz as i32), (k, x, y, z));
                n += 1;
            }
            k += 1;
        }
        assert_eq!(n, RAYS as usize);
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

    /// The ray as it was before the index followed the steps: rebuilt from the
    /// position at every cell change, and run to the end even outside the cube.
    fn trace_reference(scene: &mut Scene, frac: (i32, i32, i32), step: (i32, i32, i32), power_k: i32) {
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

    /// The faster ray marks exactly the cells the old one did, for every ray
    /// direction, strengths from nothing to a charged sapper's, any start
    /// inside the centre cell, and a world of mixed resistance.
    #[test]
    fn trace_marks_what_it_always_marked() {
        let mut snap = [0u8; CELLS];
        let mut res = [NO_BLOCK; 256];
        // Block 1 dirt-soft, 2 stone, 3 obsidian-hard, 4 leaves-soft; 0 air.
        res[1] = 50;
        res[2] = 600;
        res[3] = 120_000;
        res[4] = 20;
        let mut rng = 0x1234_5678u32;
        let mut next = |m: u32| {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            (rng >> 4) % m
        };
        for round in 0..6 {
            // Round 0 is open air; the rest scatter blocks of rising density.
            for c in snap.iter_mut() {
                *c = if round == 0 || next(10) >= round as u32 + 1 { 0 } else { (1 + next(4)) as u8 };
            }
            // The centre cell is where the blast stands: open.
            snap[Cube::index(0, 0, 0)] = 0;
            let mut fast = Cube::new();
            let mut slow = Cube::new();
            let mut checked = 0;
            for k in 0..GRID_POINTS {
                let Some(step) = ray_step(k) else { continue };
                let frac = (next(65536) as i32, next(65536) as i32, next(65536) as i32);
                let power_k = match k % 5 {
                    0 => 0,
                    1 => 700,
                    _ => 6 * (700 + next(600) as i32),
                };
                {
                    let mut a = Scene { snap: &snap, res: &res, cube: &mut fast };
                    trace(&mut a, frac, step, power_k);
                }
                {
                    let mut b = Scene { snap: &snap, res: &res, cube: &mut slow };
                    trace_reference(&mut b, frac, step, power_k);
                }
                checked += 1;
            }
            assert_eq!(checked, RAYS);
            assert_eq!(fast.bits, slow.bits, "round {round}");
            assert!(round == 0 || fast.count() > 0);
        }
    }

    /// Skipping the rays whose height band is all air marks exactly what
    /// walking every ray marks, whatever the power, the start inside the
    /// cell and the scatter of blocks.
    #[test]
    fn air_band_rays_mark_nothing() {
        let mut res = [NO_BLOCK; 256];
        res[1] = 50;
        res[2] = 600;
        res[3] = 120_000;
        let mut rng = 0x9E37_79B9u32;
        let mut next = |m: u32| {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            (rng >> 4) % m
        };
        let mut skipped_total = 0u32;
        for round in 0..24 {
            let power = 1 + (round % MAX_POWER as u32) as i32;
            let rc = reach_of(power);
            let mut snap = [0u8; CELLS];
            // Round-dependent scatter: sparse, a floor below, a ceiling above,
            // or one lone block.
            match round % 4 {
                0 => {
                    for c in snap.iter_mut() {
                        if next(60) == 0 {
                            *c = 1 + next(3) as u8;
                        }
                    }
                }
                1 => {
                    for dz in -REACH_CELLS..=REACH_CELLS {
                        for dx in -REACH_CELLS..=REACH_CELLS {
                            for dy in -REACH_CELLS..0 {
                                snap[Cube::index(dx, dy, dz)] = 1 + next(3) as u8;
                            }
                        }
                    }
                }
                2 => {
                    for dz in -REACH_CELLS..=REACH_CELLS {
                        for dx in -REACH_CELLS..=REACH_CELLS {
                            snap[Cube::index(dx, 4 + (round as i32 % 3), dz)] = 2;
                        }
                    }
                }
                _ => snap[Cube::index(next(9) as i32 - 4, next(9) as i32 - 4, next(9) as i32 - 4)] = 2,
            }
            snap[Cube::index(0, 0, 0)] = 0;
            // The layers of the columns within the reach, as the game works them out.
            let mut layers = 0u32;
            for dz in -rc..=rc {
                for dx in -rc..=rc {
                    let base = Cube::index(dx, -REACH_CELLS, dz);
                    layers |= column_layers(&snap[base..base + SIDE]);
                }
            }
            let frac = (next(65536) as i32, next(65536) as i32, next(65536) as i32);
            let steps = max_steps(power);
            let mut full = Cube::new();
            let mut banded = Cube::new();
            let seed = 0xABCD_0000 + round;
            for k in 0..GRID_POINTS {
                let Some(step) = ray_step(k) else { continue };
                let p = ray_power(power, seed, k);
                let mut a = Scene { snap: &snap, res: &res, cube: &mut full };
                trace(&mut a, frac, step, p);
                if band_is_air(layers, frac.1, step.1, steps) {
                    skipped_total += 1;
                } else {
                    let mut b = Scene { snap: &snap, res: &res, cube: &mut banded };
                    trace(&mut b, frac, step, p);
                }
            }
            assert_eq!(full.bits, banded.bits, "round {round} power {power}");
        }
        assert!(skipped_total > 1000, "the test must skip some rays: {skipped_total}");
    }

    #[test]
    fn column_layers_flags_every_block() {
        let mut rng = 0x1357_9BDFu32;
        for _ in 0..2000 {
            let mut col = [0u8; SIDE];
            for c in col.iter_mut() {
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
                *c = if rng & 3 == 0 { (rng >> 8) as u8 } else { 0 };
            }
            let mut want = 0u32;
            for (k, &c) in col.iter().enumerate() {
                want |= ((c != 0) as u32) << k;
            }
            assert_eq!(column_layers(&col), want);
        }
    }

    #[test]
    fn reach_covers_the_longest_ray() {
        for power in 1..=MAX_POWER {
            // The longest ray, in cells from the centre, at the strongest roll.
            let steps = power * 1299 / STEP_LOSS + 1;
            let reach = (steps as i64 * 19_661 + 65_535) / 65_536 + 1;
            assert!(reach_of(power) as i64 >= reach.min(REACH_CELLS as i64), "power {power}");
        }
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

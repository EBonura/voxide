//! Java's fire rules (minecraft.wiki/w/Fire), with no access to the world.
//!
//! A flame has an age, 0 to 15. Every 30 to 39 game ticks it gets one scheduled
//! tick, which is `tick` here: it may go out (rain, no fuel, old age on a bare
//! block), ages by 0 or 1, burns the blocks around it away or turns them into
//! flame, and lights flames in nearby empty cells that touch something that
//! burns. The odds are the wiki's numbers; the world sits behind the `Env`
//! trait so the host tests (tools/host-tests/tests/fire_blast.rs) can pin them
//! against a grid of their own. Plain integer code with no dependencies, like
//! units.rs.

/// A flame's oldest age.
pub const MAX_AGE: u8 = 15;
/// Java's difficulty term in the spread odds: 7 a level, Normal is level 2.
pub const DIFFICULTY_TERM: u32 = 14;

/// What a cell holds, as far as fire cares.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Cell {
    /// The cell is air: the only kind a flame can spread into.
    pub empty: bool,
    /// Encouragement: how readily a flame beside this block lights its
    /// neighbours (the wiki's "chance to ignite").
    pub ignite: u8,
    /// Flammability: how readily a flame beside it burns the block itself
    /// away (the wiki's "chance to burn").
    pub burn: u8,
    /// A solid block with a top face a flame can stand on.
    pub sturdy: bool,
    /// Netherrack-like: a flame on it never goes out of old age.
    pub infinite: bool,
    /// TNT: burnt away means primed.
    pub tnt: bool,
}

/// The world as a flame sees it.
pub trait Env {
    fn cell(&self, x: i32, y: i32, z: i32) -> Cell;
    /// A uniform number in `0..n`.
    fn rand(&mut self, n: u32) -> u32;
    /// It is raining somewhere in the world (Java's isRaining).
    fn raining(&self) -> bool;
    /// Rain reaches the cell or one of its four sides (FireBlock.isNearRain).
    fn near_rain(&self, x: i32, y: i32, z: i32) -> bool;
    /// Rain reaches exactly this cell (isRainingAt).
    fn rain_at(&self, x: i32, y: i32, z: i32) -> bool;
    /// Light a new flame of this age in an empty cell.
    fn spawn_fire(&mut self, x: i32, y: i32, z: i32, age: u8);
    /// A burnt block turns into a flame of this age in place.
    fn burn_to_fire(&mut self, x: i32, y: i32, z: i32, age: u8);
    /// A burnt block is gone.
    fn burn_away(&mut self, x: i32, y: i32, z: i32);
    /// A burnt TNT block becomes primed TNT.
    fn prime(&mut self, x: i32, y: i32, z: i32);
}

/// Game ticks to a flame's next tick: 30 plus 0 to 9.
pub fn tick_delay(r: u32) -> u32 {
    30 + r % 10
}

/// Is there anything beside the flame that burns? (BaseFireBlock's valid
/// fire location: any of the six neighbours with flammability.)
pub fn valid_location<E: Env>(e: &E, x: i32, y: i32, z: i32) -> bool {
    e.cell(x + 1, y, z).burn > 0
        || e.cell(x - 1, y, z).burn > 0
        || e.cell(x, y + 1, z).burn > 0
        || e.cell(x, y - 1, z).burn > 0
        || e.cell(x, y, z + 1).burn > 0
        || e.cell(x, y, z - 1).burn > 0
}

/// A flame can stand on a sturdy top or beside fuel (FireBlock.canSurvive).
pub fn can_survive<E: Env>(e: &E, x: i32, y: i32, z: i32) -> bool {
    e.cell(x, y - 1, z).sturdy || valid_location(e, x, y, z)
}

/// The best ignite odds among an empty cell's six neighbours; 0 for a cell
/// that is not empty.
fn ignite_odds_at<E: Env>(e: &E, x: i32, y: i32, z: i32) -> u32 {
    if !e.cell(x, y, z).empty {
        return 0;
    }
    let mut best = 0u8;
    let n = [
        e.cell(x + 1, y, z),
        e.cell(x - 1, y, z),
        e.cell(x, y + 1, z),
        e.cell(x, y - 1, z),
        e.cell(x, y, z + 1),
        e.cell(x, y, z - 1),
    ];
    let mut i = 0;
    while i < 6 {
        if n[i].ignite > best {
            best = n[i].ignite;
        }
        i += 1;
    }
    best as u32
}

/// Spread odds for an empty cell touching fuel of encouragement `ignite`,
/// from a flame of age `age`: the threshold the roll must not exceed.
pub fn spread_threshold(ignite: u32, age: u8) -> u32 {
    (ignite + 40 + DIFFICULTY_TERM) / (age as u32 + 30)
}

/// The roll's range for a cell `dy` above the flame: 100, plus 100 for each
/// level above the second.
pub fn spread_range(dy: i32) -> u32 {
    if dy > 1 {
        100 + (dy as u32 - 1) * 100
    } else {
        100
    }
}

/// One face's burn check: the six neighbours roll against their own
/// flammability with a face-specific chance range, 300 beside and 250 above
/// and below.
fn check_burn_out<E: Env>(e: &mut E, x: i32, y: i32, z: i32, chance: u32, age: u8) {
    let c = e.cell(x, y, z);
    if c.burn == 0 {
        return;
    }
    if e.rand(chance) < c.burn as u32 {
        if e.rand(age as u32 + 10) < 5 && !e.rain_at(x, y, z) {
            let j = (age as u32 + e.rand(5) / 4).min(MAX_AGE as u32) as u8;
            e.burn_to_fire(x, y, z, j);
        } else {
            e.burn_away(x, y, z);
        }
        if c.tnt {
            e.prime(x, y, z);
        }
    }
}

/// One scheduled tick of the flame at (x, y, z) with age `age`. `None` is a
/// flame that went out; `Some(a)` is its new age.
pub fn tick<E: Env>(e: &mut E, x: i32, y: i32, z: i32, age: u8) -> Option<u8> {
    if !can_survive(e, x, y, z) {
        return None;
    }
    let below = e.cell(x, y - 1, z);
    let infinite = below.infinite;
    // Rain: 0.2 + 0.03 x age.
    if !infinite && e.raining() && e.near_rain(x, y, z) && e.rand(1000) < 200 + 30 * age as u32 {
        return None;
    }
    let i = age;
    let aged = (i as u32 + e.rand(3) / 2).min(MAX_AGE as u32) as u8;
    if !infinite {
        if !valid_location(e, x, y, z) {
            // Beside nothing that burns: it lives only on a sturdy floor, and
            // only while young.
            if !below.sturdy || i > 3 {
                return None;
            }
            return Some(aged);
        }
        // A fully grown flame on a block that cannot burn dies a quarter of
        // the time each tick.
        if i == MAX_AGE && e.rand(4) == 0 && below.burn == 0 {
            return None;
        }
    }
    check_burn_out(e, x + 1, y, z, 300, i);
    check_burn_out(e, x - 1, y, z, 300, i);
    check_burn_out(e, x, y - 1, z, 250, i);
    check_burn_out(e, x, y + 1, z, 250, i);
    check_burn_out(e, x, y, z - 1, 300, i);
    check_burn_out(e, x, y, z + 1, 300, i);
    // Spread into empty cells in the 3 x 6 x 3 around it (one below to four
    // above), where fuel is adjacent.
    let mut dx = -1;
    while dx <= 1 {
        let mut dz = -1;
        while dz <= 1 {
            let mut dy = -1;
            while dy <= 4 {
                if dx != 0 || dy != 0 || dz != 0 {
                    let (nx, ny, nz) = (x + dx, y + dy, z + dz);
                    let p = ignite_odds_at(e, nx, ny, nz);
                    if p > 0 {
                        let q = spread_threshold(p, i);
                        if q > 0
                            && e.rand(spread_range(dy)) <= q
                            && !(e.raining() && e.near_rain(nx, ny, nz))
                        {
                            let r = (i as u32 + e.rand(5) / 4).min(MAX_AGE as u32) as u8;
                            if can_survive(e, nx, ny, nz) {
                                e.spawn_fire(nx, ny, nz, r);
                            }
                        }
                    }
                }
                dy += 1;
            }
            dz += 1;
        }
        dx += 1;
    }
    Some(aged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delays_and_thresholds() {
        assert_eq!(tick_delay(0), 30);
        assert_eq!(tick_delay(9), 39);
        // A young flame beside planks (ignite 5): (5 + 40 + 14) / 30 = 1.
        assert_eq!(spread_threshold(5, 0), 1);
        // Leaves or wool (30): (30 + 54) / 30 = 2; TNT (15): (15 + 54) / 30 = 2.
        assert_eq!(spread_threshold(30, 0), 2);
        assert_eq!(spread_threshold(15, 0), 2);
        // The odds fall with age: (30 + 54) / 45 = 1 at age 15.
        assert_eq!(spread_threshold(30, 15), 1);
        assert_eq!(spread_range(0), 100);
        assert_eq!(spread_range(1), 100);
        assert_eq!(spread_range(2), 200);
        assert_eq!(spread_range(4), 400);
    }
}

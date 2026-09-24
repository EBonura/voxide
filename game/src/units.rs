//! Game time and speed, in one place.
//!
//! Everything that is game time (the player, mobs, survival, the day, world
//! timers, fluids, particles and item drops) advances on ONE clock: the sim
//! tick, SIM_HZ a second, one per elapsed vblank (main's `sim_n`). Nothing
//! counts rendered frames and there is no second, slower clock.
//!
//! Tuning constants are written in real units (seconds, milliseconds, blocks
//! per second, or Java game ticks where Java defines the value at 20 a second)
//! and converted to sim ticks here, at compile time, in i32. The number in a
//! constant's source is then the number in its comment: the old per-frame
//! values drifted from their comments twice (once when the sim moved to 60 Hz
//! and once more for the timers that counted frames).
//!
//! Plain `const fn`s with no dependencies, so the host test
//! (tools/test_units.py, `rustc --test` on this file) can pin them.

/// Sim ticks per second: one per NTSC vblank.
pub const SIM_HZ: i32 = 60;
/// Java Edition game ticks per second (minecraft.wiki/w/Tick#Game_tick).
pub const JAVA_TPS: i32 = 20;
/// World units per block.
pub const BLOCK: i32 = 64;

/// Whole seconds to sim ticks.
pub const fn secs(s: i32) -> i32 {
    s * SIM_HZ
}

/// Milliseconds to sim ticks, rounded to the nearest tick.
pub const fn ms(m: i32) -> i32 {
    (m * SIM_HZ + 500) / 1000
}

/// Java game ticks (20 a second) to sim ticks: exactly 3 each.
pub const fn java_ticks(t: i32) -> i32 {
    t * (SIM_HZ / JAVA_TPS)
}

/// A speed in hundredths of a block per second, as Q8 world units per sim
/// tick (256 = one world unit a tick), rounded.
pub const fn cbps_q8(cbps: i32) -> i32 {
    (cbps * BLOCK * 256 + 50 * SIM_HZ) / (100 * SIM_HZ)
}

/// A speed in hundredths of a block per second, as quarter world units per
/// sim tick (the player's vertical velocity), rounded.
pub const fn cbps_q2(cbps: i32) -> i32 {
    (cbps * BLOCK * 4 + 50 * SIM_HZ) / (100 * SIM_HZ)
}

/// An acceleration in hundredths of a block per second squared, as quarter
/// world units per sim tick per sim tick, rounded.
pub const fn cbps2_q2(cbps2: i32) -> i32 {
    (cbps2 * BLOCK * 4 + 50 * SIM_HZ * SIM_HZ) / (100 * SIM_HZ * SIM_HZ)
}

/// An acceleration in hundredths of a block per second squared, as Q8 world
/// units per sim tick per sim tick, rounded.
pub const fn cbps2_q8(cbps2: i32) -> i32 {
    (cbps2 * BLOCK * 256 + 50 * SIM_HZ * SIM_HZ) / (100 * SIM_HZ * SIM_HZ)
}

/// A turn rate in degrees per second, as Q8 angle units per sim tick
/// (4096 angle units to the turn), rounded.
pub const fn dps_q8(dps: i32) -> i32 {
    (dps * 4096 * 256 + 180 * SIM_HZ) / (360 * SIM_HZ)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time() {
        assert_eq!(secs(1), 60);
        assert_eq!(java_ticks(20), 60); // one Java second is one real second
        assert_eq!(java_ticks(10), secs(1) / 2); // Java i-frames: 0.5 s
        assert_eq!(ms(500), 30);
        assert_eq!(ms(250), java_ticks(5)); // Java water spreads every 5 game ticks
        assert_eq!(secs(600), 36_000); // the 10-minute day
    }

    #[test]
    fn speed() {
        // 4.32 blocks/s (Java walks 4.317) is 4.61 units a tick: 1180/256.
        assert_eq!(cbps_q8(432), 1180);
        assert_eq!(cbps_q8(100), 273); // 1 block/s
        // 60 blocks/s is a block a tick.
        assert_eq!(cbps_q8(6000), BLOCK * 256);
        // The player's quarter units: 26.25 blocks/s is 28 units a tick.
        assert_eq!(cbps_q2(2625), 112);
        // 56.25 blocks/s^2 is one unit a tick per tick.
        assert_eq!(cbps2_q2(5625), 4);
        assert_eq!(cbps2_q8(5625), 256);
        // 90 degrees a second is 17.07 angle units a tick.
        assert_eq!(dps_q8(90), 4369);
    }
}

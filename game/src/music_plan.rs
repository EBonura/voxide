//! What plays when: Java's music rules as plain functions, so a host test can
//! pin them (tools/host-tests/tests/music_plan.rs). The drive, the scheduler
//! state and the state block are in music.rs; the pieces are musicdata.rs.
//!
//! Java Edition's music manager (minecraft.wiki/w/Music, "Background music"):
//! the title screen starts a random menu track "after 1 to 30 seconds", and
//! the menu music stops when the loading screen for a world appears. In a
//! world, "a random general track" plays "10 to 20 minutes" after the last one
//! finished. Music is not looped: a song plays through once and a delay follows (a
//! song that is a loop has its repeats written into the file). Game music keeps playing while the game is
//! paused. The wiki gives no tick values and no delay for the first track on
//! joining a world; those come from the client's music manager (a 20 to 600
//! tick menu range, a 12,000 to 24,000 tick game range, which is the wiki's 1
//! to 30 seconds and 10 to 20 minutes, and a 100 tick starting delay), see
//! AUDIT.md for which of these the wiki confirms.

use crate::musicdata::TRACKS;

/// Java game ticks (20 a second) in an (inclusive) range.
pub type JavaRange = (u32, u32);

/// "a random menu track starts playing after 1 to 30 seconds".
pub const MENU_DELAY: JavaRange = (20, 600);
/// "10 to 20 minutes" after the previous track finishes.
pub const GAME_DELAY: JavaRange = (12_000, 24_000);
/// The first game track after joining a world.
pub const START_DELAY: u32 = 100;

/// Sim ticks (vblanks) in a Java game tick.
const VBLANKS_PER_JAVA_TICK: u32 = 3;

/// A delay in vblanks, uniform over a Java tick range from the random value `r`.
pub const fn delay_vblanks(range: JavaRange, r: u32) -> u32 {
    (range.0 + r % (range.1 - range.0 + 1)) * VBLANKS_PER_JAVA_TICK
}

/// A piece (an index into `TRACKS`) from the random value `r`. Java's overworld
/// music is one pool for the whole world, and so is this: the pieces are not
/// tagged by time of day or depth (AUDIT.md).
pub const fn pick(r: u32) -> usize {
    r as usize % TRACKS.len()
}

/// The drive mixer level at the top of the slider. 0x80 is the pieces at the
/// level they were cooked; 0x40 is 6 dB under that. Measured on the SPU output
/// (headless capture, 2026-10-15): at 0x80 the music alone ran -25 dBFS RMS with
/// seconds peaking at 0.27 of full scale, as loud as the sound effects' own
/// peaks (0.1 to 0.25), so footsteps and digging would sit in it. At 0x40 the
/// effects stand 10 dB or more above it, closer to Java's quiet music.
pub const MIXER_FULL: u32 = 0x40;

/// The drive mixer level for a MUSIC slider percentage.
pub const fn mixer_level(percent: u8) -> u8 {
    let p = if percent > 100 { 100 } else { percent } as u32;
    (p * MIXER_FULL / 100) as u8
}

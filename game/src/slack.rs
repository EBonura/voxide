//! A frame's spare time, for work that can wait: explosions and fire.
//!
//! A blast, a flame tick or a primed block's detonation is the kind of work
//! that costs the same whenever it runs, so it should run when the frame has
//! time to spare and wait when it does not, instead of pushing a frame past
//! the vblank it was making. The game's frames are quantised to vblanks (a
//! 30 fps frame takes two, a 20 fps frame three), so the time a frame has to
//! spare is the time it spent idle at the end, waiting for the next vblank
//! (and for the GPU). That is measured on root counter 2 (the system clock
//! over 8, a free-running 16-bit count, 4.2 million a second) around the
//! idle waits, and the next frame spends a share of it.
//!
//! Every frame gets at least one unit of work however busy it was, so a
//! saturated scene still sees an explosion finish, slowly.

use psx_io::timers::{self, Timer};

/// Counter ticks (8 system clocks) a frame may always spend on one unit of
/// work at least, the cost of the largest single unit below.
const FLOOR: u32 = 6_000;
/// The share of the last frames' idle time the next frame spends, in 16ths.
const SHARE_16: u32 = 10;

static mut IDLE_T: u16 = 0;
/// The idle time of the last two frames, counter ticks.
static mut IDLE: [u32; 2] = [0; 2];
static mut FRAME_T: u16 = 0;
static mut ALLOW: u32 = FLOOR;
static mut UNITS: u32 = 0;
static mut STARTED: bool = false;

/// Start root counter 2 free-running on the system clock over 8 (once; the
/// first frame's world update does it).
fn init() {
    // Mode bits 8..=9 pick the clock: 2 is the system clock over 8 for counter
    // 2; no sync, no target, no interrupt.
    timers::set_mode(Timer::Timer2, 2 << 8);
}

/// The idle waits of a frame begin.
#[inline(always)]
pub fn idle_begin() {
    unsafe { IDLE_T = timers::counter(Timer::Timer2) };
}

/// The idle waits of a frame are over: note how long they were.
#[inline(always)]
pub fn idle_end() {
    unsafe {
        let d = timers::counter(Timer::Timer2).wrapping_sub(IDLE_T) as u32;
        IDLE[1] = IDLE[0];
        IDLE[0] = d;
    }
}

/// A frame's world update begins: its allowance is a share of the smaller of
/// the last two frames' idle time (a one-off short or long frame does not
/// swing it).
pub fn frame_begin() {
    unsafe {
        if !STARTED {
            STARTED = true;
            init();
        }
        let idle = IDLE[0].min(IDLE[1]);
        ALLOW = (idle * SHARE_16 / 16).max(FLOOR);
        UNITS = 0;
        FRAME_T = timers::counter(Timer::Timer2);
    }
}

#[inline(always)]
fn spent() -> u32 {
    unsafe { timers::counter(Timer::Timer2).wrapping_sub(FRAME_T) as u32 }
}

/// May another unit of work start? Always the first of a frame, then while
/// the frame's allowance lasts.
pub fn room() -> bool {
    unsafe {
        if UNITS == 0 || spent() < ALLOW {
            UNITS += 1;
            true
        } else {
            false
        }
    }
}

/// `room` for a unit that costs about `cycles` system clocks: it needs that
/// much of the allowance left, unless it is the frame's first unit.
pub fn room_for(cycles: u32) -> bool {
    unsafe {
        if UNITS == 0 || spent() + cycles / 8 <= ALLOW {
            UNITS += 1;
            true
        } else {
            false
        }
    }
}

/// The allowance this frame has, system clocks: the lab and the gates read it.
#[allow(dead_code)]
pub fn allowance() -> u32 {
    unsafe { ALLOW * 8 }
}

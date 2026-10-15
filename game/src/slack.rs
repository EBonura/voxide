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
//!
//! An explosion the player is watching is not work that can wait: while a
//! blast is queued, running or still being meshed the frame spends most of its
//! spare time on it (`set_urgent`). The spare time is what the frame would
//! have idled had it not done the blast's work, up to the frame period the
//! scene kept before the blast (a whole number of vblanks, the smoothed period
//! of the last quiet frames): the frame that holds its vblank rate keeps it.
//! Only a scene with no spare time at all gets `URGENT_MIN`, which can cost it
//! a vblank for as long as the blast lasts.
//!
//! The first frames of an explosion after a quiet spell may borrow two vblanks
//! more (`BOOST_FRAMES` of them, `BOOST_VBLANKS`), a dip the scene can afford
//! once: the crater of a single TNT is up in that time, and a long chain goes
//! on at the rate the frame can hold.

use psx_io::timers::{self, Timer};
use psx_rt::interrupts;

/// Counter ticks (8 system clocks) a frame may always spend on one unit of
/// work at least, the cost of the largest single unit below.
const FLOOR: u32 = 6_000;
/// The share of the last frames' idle time the next frame spends, in 16ths.
const SHARE_16: u32 = 10;
/// The same while a blast is in progress, of the spare time before the blast
/// took any of it.
const URGENT_SHARE_16: u32 = 13;
/// Urgent frames at the start of an explosion that may run `BOOST_VBLANKS`
/// long, and the quiet frames (about three seconds) after which it is owed
/// again.
const BOOST_FRAMES: u32 = 6;
const BOOST_VBLANKS: u32 = 2;
const BOOST_REARM: u32 = 90;
/// What an urgent frame spends at least, counter ticks (8 system clocks):
/// 20,000 is 160,000 clocks, over a quarter of a vblank.
const URGENT_MIN: u32 = 20_000;

static mut IDLE_T: u16 = 0;
/// Idle counter ticks banked by `idle_mark` in this frame's waits.
static mut IDLE_ACC: u32 = 0;
/// The idle time of the last two frames, counter ticks.
static mut IDLE: [u32; 2] = [0; 2];
/// Counter ticks the frame has spent since `frame_begin`, summed at every
/// look at the clock so it can run past one lap of the 16-bit counter.
static mut LAST_T: u16 = 0;
static mut SPENT: u32 = 0;
static mut ALLOW: u32 = FLOOR;
static mut URGENT: bool = false;
/// What the last two frames could have spared: the frame period the scene
/// keeps when quiet, less the work the frame did besides the blast's.
static mut SPARE: [u32; 2] = [0; 2];
/// Counter ticks in a vblank: 263 lines of 2,172 system clocks (571,236), over 8.
const VBLANK_TICKS: u32 = 71_400;
static mut LAST_VB: u32 = 0;
/// The last frame's period in vblanks, and the smoothed period of the quiet
/// frames before a blast, in sixteenths of a vblank.
static mut PERIOD: u32 = 2;
static mut QUIET_16: u32 = 32;
static mut WAS_URGENT: bool = false;
static mut BOOST_LEFT: u32 = 0;
static mut QUIET_FRAMES: u32 = BOOST_REARM;
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
    unsafe {
        IDLE_T = timers::counter(Timer::Timer2);
        IDLE_ACC = 0;
    }
}

/// Bank the idle time so far (between the GPU wait and the vblank wait), so
/// neither stretch alone has to fit one lap of the 16-bit counter.
#[inline(always)]
pub fn idle_mark() {
    unsafe {
        let now = timers::counter(Timer::Timer2);
        IDLE_ACC += now.wrapping_sub(IDLE_T) as u32;
        IDLE_T = now;
    }
}

/// The idle waits of a frame are over: note how long they were.
#[inline(always)]
pub fn idle_end() {
    unsafe {
        let d = IDLE_ACC + timers::counter(Timer::Timer2).wrapping_sub(IDLE_T) as u32;
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
        // The frame that just ended: how long it took, and how much of that
        // was neither idle nor the blast's.
        let vb = interrupts::vblank_count();
        let period = vb.wrapping_sub(LAST_VB);
        LAST_VB = vb;
        if period >= 1 && period <= 6 {
            PERIOD = period;
            if !WAS_URGENT {
                QUIET_16 = (QUIET_16 * 7 + period * 16) / 8;
            }
        }
        let base = (PERIOD * VBLANK_TICKS).saturating_sub(IDLE[0] + JOB_SPENT + MESH_SPENT);
        let target = QUIET_16.div_ceil(16) * VBLANK_TICKS;
        SPARE[1] = SPARE[0];
        SPARE[0] = target.saturating_sub(base);
        JOB_SPENT = 0;
        MESH_SPENT = 0;
        WAS_URGENT = URGENT;
        let idle = IDLE[0].min(IDLE[1]);
        ALLOW = if URGENT {
            let mut spare = SPARE[0].min(SPARE[1]);
            if QUIET_FRAMES >= BOOST_REARM {
                BOOST_LEFT = BOOST_FRAMES;
            }
            QUIET_FRAMES = 0;
            if BOOST_LEFT > 0 {
                BOOST_LEFT -= 1;
                spare += BOOST_VBLANKS * VBLANK_TICKS;
            }
            (spare * URGENT_SHARE_16 / 16).max(URGENT_MIN)
        } else {
            QUIET_FRAMES += 1;
            (idle * SHARE_16 / 16).max(FLOOR)
        };
        UNITS = 0;
        LAST_T = timers::counter(Timer::Timer2);
        SPENT = 0;
    }
}

/// A blast is in progress (queued, running, or its chunks still waiting to be
/// meshed): the next frames spend the urgent share. Set once a frame by the
/// world update, before `frame_begin`.
pub fn set_urgent(on: bool) {
    unsafe { URGENT = on };
}

/// The frame's whole allowance for explosion work, counter ticks. The mesh
/// burst after the render takes what the blast job left of it.
pub fn budget() -> u32 {
    unsafe { ALLOW }
}

/// What the blast job has spent this frame, noted when it stops: the mesh
/// burst gets the rest of the allowance. And what the burst itself spent.
static mut JOB_SPENT: u32 = 0;
static mut MESH_SPENT: u32 = 0;

pub fn mesh_done(ticks: u32) {
    unsafe { MESH_SPENT = ticks };
}

pub fn job_done() {
    unsafe { JOB_SPENT = spent() };
}

/// What the mesh burst after the render may spend, counter ticks: the
/// frame's explosion allowance less the blast job's share, and at least a
/// quarter of it so a long job cannot starve the meshing.
pub fn mesh_budget() -> u32 {
    unsafe { ALLOW.saturating_sub(JOB_SPENT).max(ALLOW / 4) }
}

/// Counter ticks spent since `frame_begin`.
#[inline(always)]
pub fn spent() -> u32 {
    unsafe {
        let now = timers::counter(Timer::Timer2);
        SPENT += now.wrapping_sub(LAST_T) as u32;
        LAST_T = now;
        SPENT
    }
}

/// A stopwatch on the same counter, for a burst of work outside the world
/// update. `elapsed` must be looked at more often than one lap of the counter
/// (a little over half a vblank), which the units it measures are.
pub struct Stopwatch {
    last: u16,
    ticks: u32,
}

impl Stopwatch {
    pub fn start() -> Stopwatch {
        Stopwatch {
            last: timers::counter(Timer::Timer2),
            ticks: 0,
        }
    }

    /// Counter ticks since `start`.
    pub fn elapsed(&mut self) -> u32 {
        let now = timers::counter(Timer::Timer2);
        self.ticks += now.wrapping_sub(self.last) as u32;
        self.last = now;
        self.ticks
    }
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

/// The spare time the urgent allowance came from, system clocks (the lab).
#[allow(dead_code)]
pub fn lab_spare() -> u32 {
    unsafe { SPARE[0].min(SPARE[1]) * 8 }
}

/// The allowance this frame has, system clocks: the lab and the gates read it.
#[allow(dead_code)]
pub fn allowance() -> u32 {
    unsafe { ALLOW * 8 }
}

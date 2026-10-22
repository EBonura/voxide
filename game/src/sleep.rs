//! Sleeping in a bed. Using a bed sets the respawn point beside it (Java does
//! this at any hour); at night, with no monster close by, it also puts the
//! player to sleep: the screen fades to black, the clock runs on to morning (and
//! a storm blows over) while it is dark, and it fades back in. The player cannot
//! move while asleep, and a hit wakes them.
//!
//! Java's rules (minecraft.wiki/w/Bed): sleep is allowed when it is night (the
//! sky light is 11 or less) or in a thunderstorm, and refused while a hostile
//! mob is within 8 blocks sideways and 5 up or down of the bed.

use crate::units::ms;

/// Sim ticks of fade to black, of darkness before the morning, and of fade in.
const FADE_OUT: u32 = ms(1000) as u32;
const DARK: u32 = ms(700) as u32;
const FADE_IN: u32 = ms(1000) as u32;
const TOTAL: u32 = FADE_OUT + DARK + FADE_IN;

/// Sim ticks asleep (0 awake).
static mut T: u32 = 0;
/// The clock has not been moved on yet this sleep.
static mut SKIP_DUE: bool = false;

pub fn active() -> bool {
    unsafe { T != 0 }
}

pub fn begin() {
    unsafe {
        T = 1;
        SKIP_DUE = true;
    }
}

/// A hit, a menu, a new world: wake at once.
pub fn cancel() {
    unsafe {
        T = 0;
        SKIP_DUE = false;
    }
}

/// Advance `ticks` sim ticks.
pub fn tick(ticks: u32) {
    unsafe {
        if T != 0 {
            T += ticks;
            if T >= TOTAL {
                T = 0;
                SKIP_DUE = false;
            }
        }
    }
}

/// True once, in the dark: the caller moves the clock on to morning.
pub fn take_skip() -> bool {
    unsafe {
        if SKIP_DUE && T >= FADE_OUT {
            SKIP_DUE = false;
            return true;
        }
    }
    false
}

/// How black the screen is, 0..=4 (the fade is drawn in quarter steps).
pub fn darkness() -> u8 {
    let t = unsafe { T };
    if t == 0 {
        return 0;
    }
    let d = if t < FADE_OUT {
        t * 4 / FADE_OUT
    } else if t < FADE_OUT + DARK {
        4
    } else {
        4 - ((t - FADE_OUT - DARK) * 4 / FADE_IN).min(4)
    };
    d.min(4) as u8
}

/// Is a hostile mob close enough to the bed to refuse sleep?
pub fn monsters_near(bx: i32, by: i32, bz: i32) -> bool {
    let mut i = 0;
    while i < crate::mob::CAP {
        let m = crate::mob::get(i);
        if m.alive && crate::mob::is_hostile(m.kind) {
            let (mx, my, mz) = (
                crate::world_to_block_x(m.x),
                crate::world_to_block_y(m.y),
                crate::world_to_block_z(m.z),
            );
            if (mx - bx).abs() <= 8 && (mz - bz).abs() <= 8 && (my - by).abs() <= 5 {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// The column to wake in: the first open cell beside either half of the bed
/// (air at the bed's height and over it, something solid under), else the
/// bed's own column. Java puts you beside the bed, not on it.
pub fn respawn_beside(bx: i32, by: i32, bz: i32) -> (i32, i32) {
    let b = crate::get_block_i32(bx, by, bz);
    let f = crate::bstate::bed_facing(b) as usize;
    let sign = if crate::bstate::bed_head(b) { -1 } else { 1 };
    // The other half is one along the bed's axis.
    let (ox, oz) = (
        bx + sign * crate::bstate::FDX[f],
        bz + sign * crate::bstate::FDZ[f],
    );
    let mut half = 0;
    while half < 2 {
        let (hx, hz) = if half == 0 { (bx, bz) } else { (ox, oz) };
        let mut side = 0;
        while side < 4 {
            let (cx, cz) = (hx + crate::bstate::FDX[side], hz + crate::bstate::FDZ[side]);
            if (cx, cz) != (bx, bz)
                && (cx, cz) != (ox, oz)
                && crate::get_block_i32(cx, by, cz) == crate::AIR
                && crate::get_block_i32(cx, by + 1, cz) == crate::AIR
                && crate::blocks::is_sturdy(crate::get_block_i32(cx, by - 1, cz))
            {
                return (cx, cz);
            }
            side += 1;
        }
        half += 1;
    }
    (bx, bz)
}

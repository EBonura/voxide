//! The bow: draw it, hold it, let go.
//!
//! Java's rules (minecraft.wiki/w/Bow): use starts the draw, only if there is an
//! arrow in the pack; the pull is how many game ticks the button was held, over
//! 20, run through f = (t * t + 2 t) / 3 and capped at 1; below 0.1 nothing is
//! shot and no arrow is used; the arrow leaves at 3 blocks a game tick times f
//! and does ceil(2 x speed) damage, with a critical (random extra damage, up to
//! half again) on a full draw. While the bow is drawn the player walks at a
//! fifth of the speed.
//!
//! The held model is drawn here too: two limbs that bend as the string comes
//! back, the string itself, the arrow on it, and on release the string snaps
//! forward and rings out while the bow kicks. It is a small 3D model turned and
//! projected by hand in screen space, drawn into the UI list over the arm
//! (main.rs draw_held_item calls `draw_held`), not a flat sprite.
//!
//! The pure parts (the draw curve, the model's geometry) are plain integer
//! maths with no game dependencies beyond `psx_math::sincos`.

use psx_math::sincos::{cos_q12, sin_q12};

/// Sim ticks a full draw takes: 20 game ticks, 3 sim ticks each.
pub const FULL_DRAW: u32 = 60;
/// The least pull that shoots, Java's 0.1, in Q12.
pub const MIN_POWER: i32 = 410;

/// Java's pull: `held` sim ticks of draw to f in Q12 (4096 = a full draw).
pub fn power_q12(held: u32) -> i32 {
    let u = (held.min(FULL_DRAW) as i32) * 4096 / FULL_DRAW as i32;
    ((u * u / 4096 + 2 * u) / 3).min(4096)
}

/// How far the string is back, 0 to `PULL` units, from the draw time: it follows
/// the pull linearly in time (the animation Java plays), not the power curve.
pub const PULL: i32 = 40;
pub fn pull_units(held: u32) -> i32 {
    held.min(FULL_DRAW) as i32 * PULL / FULL_DRAW as i32
}

/// The string's offset while it rings out after a release: `since` sim ticks
/// after the shot, from `from` units back. A damped swing about rest.
pub const RING_TICKS: u32 = 14;
pub fn ring_units(from: i32, since: u32) -> i32 {
    if since >= RING_TICKS {
        return 0;
    }
    // About 2 swings in the time, dying away linearly.
    let ang = (since as i32 * 4096 * 5 / (2 * RING_TICKS as i32)) as u16;
    let swing = cos_q12(ang); // Q12, starts at 4096
    from * swing / 4096 * (RING_TICKS as i32 - since as i32) / RING_TICKS as i32
}

/// Damage of an arrow flying at `speed_q8` Q8 world units per sim tick: Java's
/// ceil(2 x blocks per game tick). A block is 64 units and a game tick 3 sim
/// ticks, so 1 block per sim tick (16,384 Q8) is 3 blocks per game tick.
pub fn damage_for(speed_q8: i32) -> i32 {
    (speed_q8 * 6 + 16383) / 16384
}

// ---- State.

/// Sim ticks the draw has run (0 when not drawing).
static mut HELD: u32 = 0;
static mut DRAWING: bool = false;
/// The string is ringing out: ticks since the release and where it started.
static mut SINCE: u32 = RING_TICKS;
static mut FROM: i32 = 0;

pub fn drawing() -> bool {
    unsafe { DRAWING }
}

pub fn reset() {
    unsafe {
        HELD = 0;
        DRAWING = false;
        SINCE = RING_TICKS;
    }
}

/// What `update` asks of the game.
pub enum Event {
    None,
    /// The draw began.
    Drew,
    /// Let go with this pull (Q12): shoot.
    Loose(i32),
    /// The draw was dropped (item changed, no arrows left, too weak a pull).
    Cancel,
}

/// One frame of the bow. `selected` is whether the bow is in hand, `usable`
/// whether an arrow is in the pack and nothing else took the button, `pressed`
/// the use button this frame, `held` that it is down, `ticks` the sim ticks
/// the frame covers.
pub fn update(selected: bool, usable: bool, pressed: bool, held: bool, ticks: u32) -> Event {
    unsafe {
        if SINCE < RING_TICKS {
            SINCE += ticks;
        }
        if !DRAWING {
            if selected && usable && pressed {
                DRAWING = true;
                HELD = 0;
                return Event::Drew;
            }
            return Event::None;
        }
        if !selected {
            DRAWING = false;
            HELD = 0;
            return Event::Cancel;
        }
        if held {
            HELD += ticks;
            return Event::None;
        }
        // Let go.
        let power = power_q12(HELD);
        let pull = pull_units(HELD);
        DRAWING = false;
        HELD = 0;
        if power < MIN_POWER {
            return Event::Cancel;
        }
        SINCE = 0;
        FROM = pull;
        Event::Loose(power)
    }
}

/// Sim ticks the draw has run, for the gate.
pub fn held_ticks() -> u32 {
    unsafe { HELD }
}

/// The draw so far, 0..4096 (for the walk slowdown and the sound).
pub fn drawn_q12() -> i32 {
    unsafe { HELD.min(FULL_DRAW) as i32 * 4096 / FULL_DRAW as i32 }
}

// ---- The model.

/// Half the length of the bow, and how far its tips sit toward the archer when
/// the string is straight (all in model units, about screen pixels).
const HALF: i32 = 64;
const TIP: i32 = 24;
const SEG: usize = 6;
/// The arrow's length.
const ARROW: i32 = 96;

/// A model point to the screen: the model is turned about its vertical axis
/// by `yaw`, rolled by `roll` (Q12 angles), seen from `D` units away at focal
/// `F`, and put at (gx, gy). Returns the screen point and the depth.
fn project(x: i32, y: i32, z: i32, yaw: u16, roll: u16, gx: i32, gy: i32) -> (i16, i16, i32) {
    const D: i32 = 230;
    const F: i32 = 230;
    let (sy, cy) = (sin_q12(yaw), cos_q12(yaw));
    let xr = (x * cy + z * sy) >> 12;
    let zr = (z * cy - x * sy) >> 12;
    let (sr, cr) = (sin_q12(roll), cos_q12(roll));
    let xx = (xr * cr - y * sr) >> 12;
    let yy = (xr * sr + y * cr) >> 12;
    let depth = (D + zr).max(40);
    (
        (gx + xx * F / depth) as i16,
        (gy - yy * F / depth) as i16,
        depth,
    )
}

/// Where the limbs are, top to bottom: (y, z) per sample for a string drawn
/// `pull` units.
fn limb(i: usize, pull: i32) -> (i32, i32) {
    // i = 0 is the top tip, SEG the grip, 2 SEG the bottom tip.
    let t = i as i32 - SEG as i32; // -SEG..SEG
    let y = -t * HALF / SEG as i32; // top is +y
    let u = t.abs() * 4096 / SEG as i32; // 0..1, Q12
    let u2 = u * u / 4096;
    let tip = TIP + pull * 3 / 10;
    (y, -(tip * u2 / 4096))
}

/// Draw the held bow. `light` is the world light (0..255) to dim by, `tick`
/// the sim clock for the idle bob.
#[cfg(not(test))]
#[inline(never)]
pub fn draw_held(light: u32, tick: u32, kick: i32) {
    let (held, drawing, since, from) = unsafe { (HELD, DRAWING, SINCE, FROM) };
    // 0 at rest, 4096 at full draw, for the pose.
    let p = if drawing {
        held.min(FULL_DRAW) as i32 * 4096 / FULL_DRAW as i32
    } else {
        0
    };
    let pull = if drawing {
        pull_units(held)
    } else {
        ring_units(from, since)
    };
    // Bow pose: lower right at rest, lifted toward the middle as it is drawn,
    // with the idle bob and a kick back after the shot.
    let bob = (sin_q12(((tick * 18) & 0x0fff) as u16) >> 10) as i32;
    let after = if !drawing && since < RING_TICKS {
        (RING_TICKS - since) as i32
    } else {
        0
    };
    let gx = 276 - 38 * p / 4096 + after / 2;
    let gy = 178 + bob + kick - 28 * p / 4096 + after / 3;
    let yaw = (4096 - (62 * 4096 / 360)) as u16 & 0x0fff;
    let roll = (4096 + (10 * 4096 / 360)) as u16 & 0x0fff;
    let shade = (60 + light * 90 / 255).min(150) as i32;
    let wood = |c: i32| -> u8 { (c * shade / 128).clamp(0, 255) as u8 };

    // The limbs, as ribbons of wood between neighbouring samples. The ribbon is
    // 5 units wide (the model's x), so it shows its face as the bow turns.
    let mut pts = [(0i16, 0i16, 0i32); 2 * SEG + 1];
    let mut lo = [(0i16, 0i16); 2 * SEG + 1];
    let mut hi = [(0i16, 0i16); 2 * SEG + 1];
    let mut i = 0;
    while i <= 2 * SEG {
        let (y, z) = limb(i, pull);
        pts[i] = project(0, y, z, yaw, roll, gx, gy);
        let a = project(-3, y, z, yaw, roll, gx, gy);
        let b = project(3, y, z, yaw, roll, gx, gy);
        lo[i] = (a.0, a.1);
        hi[i] = (b.0, b.1);
        i += 1;
    }
    let mut i = 0;
    while i < 2 * SEG {
        // Thicker at the grip, thinner at the tips.
        let shade_seg = 150 - (i as i32 - SEG as i32).abs() * 6;
        crate::ui_quad_flat(
            [lo[i], hi[i], lo[i + 1], hi[i + 1]],
            wood(shade_seg * 118 / 128),
            wood(shade_seg * 84 / 128),
            wood(shade_seg * 50 / 128),
        );
        i += 1;
    }
    // The grip: a wrap of dark leather over the middle three samples.
    crate::ui_quad_flat(
        [lo[SEG - 1], hi[SEG - 1], lo[SEG + 1], hi[SEG + 1]],
        wood(70),
        wood(48),
        wood(34),
    );

    // The string, from tip to nock to tip (a straight line when not drawn).
    let (top_y, top_z) = limb(0, pull);
    let (bot_y, bot_z) = limb(2 * SEG, pull);
    let nock_z = -(TIP + pull);
    let a = project(0, top_y, top_z, yaw, roll, gx, gy);
    let n = project(0, 0, nock_z, yaw, roll, gx, gy);
    let b = project(0, bot_y, bot_z, yaw, roll, gx, gy);
    let sc = (150 * shade / 128).min(255) as u8;
    crate::ui_line(a.0, a.1, n.0, n.1, sc, sc, sc.saturating_sub(12));
    crate::ui_line(n.0, n.1, b.0, b.1, sc, sc, sc.saturating_sub(12));
    crate::ui_line(a.0 + 1, a.1, n.0 + 1, n.1, sc, sc, sc.saturating_sub(12));
    crate::ui_line(n.0 + 1, n.1, b.0 + 1, b.1, sc, sc, sc.saturating_sub(12));

    if drawing {
        // The arrow, on the string: a shaft, a grey head, white fletching.
        let tail = nock_z;
        let tipz = nock_z + ARROW;
        let t0 = project(0, 0, tail, yaw, roll, gx, gy);
        let t1 = project(0, 0, tipz, yaw, roll, gx, gy);
        let s0a = project(0, 2, tail, yaw, roll, gx, gy);
        let s0b = project(0, -2, tail, yaw, roll, gx, gy);
        let s1a = project(0, 2, tipz, yaw, roll, gx, gy);
        let s1b = project(0, -2, tipz, yaw, roll, gx, gy);
        crate::ui_quad_flat(
            [
                (s0a.0, s0a.1),
                (s1a.0, s1a.1),
                (s0b.0, s0b.1),
                (s1b.0, s1b.1),
            ],
            wood(160),
            wood(120),
            wood(70),
        );
        // Head: a diamond at the tip.
        let h0 = project(0, 0, tipz - 12, yaw, roll, gx, gy);
        let hu = project(0, 5, tipz - 8, yaw, roll, gx, gy);
        let hd = project(0, -5, tipz - 8, yaw, roll, gx, gy);
        let ht = project(0, 0, tipz + 6, yaw, roll, gx, gy);
        crate::ui_quad_flat(
            [(h0.0, h0.1), (hu.0, hu.1), (hd.0, hd.1), (ht.0, ht.1)],
            wood(206),
            wood(206),
            wood(214),
        );
        // Fletching: two small feathers at the tail.
        let f0 = project(0, 0, tail + 4, yaw, roll, gx, gy);
        let f1 = project(0, 6, tail + 12, yaw, roll, gx, gy);
        let f2 = project(0, -6, tail + 12, yaw, roll, gx, gy);
        let f3 = project(0, 0, tail + 22, yaw, roll, gx, gy);
        crate::ui_quad_flat(
            [(f0.0, f0.1), (f1.0, f1.1), (f2.0, f2.1), (f3.0, f3.1)],
            wood(236),
            wood(236),
            wood(240),
        );
        let _ = (t0, t1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_power_curve_is_javas() {
        // t ticks of 20: f = (t^2 + 2t)/3 with t as a fraction.
        assert_eq!(power_q12(0), 0);
        assert_eq!(power_q12(FULL_DRAW), 4096);
        assert_eq!(power_q12(FULL_DRAW * 3), 4096);
        // Half a second: (0.25 + 1) / 3 = 0.4167.
        let half = power_q12(FULL_DRAW / 2);
        assert!((half - 1707).abs() <= 2, "{half}");
        // 3 game ticks (9 sim): t = 0.15, f = (0.0225 + 0.3) / 3 = 0.1075 > 0.1.
        assert!(power_q12(9) >= MIN_POWER);
        assert!(power_q12(8) < MIN_POWER);
    }

    #[test]
    fn damage_follows_speed_like_java() {
        // 3 blocks a game tick, full draw, is 6; half power 3 x 0.4167 = 1.25
        // blocks a tick is ceil(2.5) = 3.
        let full = 16384; // a block a sim tick
        assert_eq!(damage_for(full), 6);
        assert_eq!(damage_for(full * 1707 / 4096), 3);
        assert_eq!(damage_for(full * 410 / 4096), 1);
    }

    #[test]
    fn the_string_rings_out() {
        let from = 40;
        assert_eq!(ring_units(from, 0), from);
        assert_eq!(ring_units(from, RING_TICKS), 0);
        // It crosses zero (overshoots forward) at least once before stopping.
        let mut crossed = false;
        for t in 1..RING_TICKS {
            if ring_units(from, t) < 0 {
                crossed = true;
            }
        }
        assert!(crossed);
    }
}

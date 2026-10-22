//! Headless crosshair measurement (`--features pick-lab`, never shipped): the
//! view is held on a pig pinned in front of the player, looking 30 degrees
//! down so the ray that passes through the pig lands on the ground behind it.
//! A `frontend launch --press` script then taps and holds R2 and the route log
//! watches `PICK_LAB` (with --route-watch-u32) to see what the crosshair and
//! the attack reached. The pig stays pinned until it is first hurt, so the
//! knockback that follows a hit is free to show.
//!
//! `--features look-lab` only publishes the view (LOOK_LAB: yaw, pitch, both
//! 4096 to the turn) so a stick tape can measure the turn rate.

use crate::*;

#[cfg(feature = "look-lab")]
#[no_mangle]
pub static mut LOOK_LAB: [i32; 2] = [0; 2];

#[cfg(feature = "look-lab")]
pub fn look(player: &Player) {
    unsafe { LOOK_LAB = [player.yaw as i32, player.pitch as i32] };
}

#[cfg(feature = "pick-lab")]
/// Watched per route tick: pig health, pig x, pig z, block pick hit (0/1),
/// picked block x, y, z, the mining progress, the targeted mob (-1 for none)
/// and the frame the lab last wrote.
#[no_mangle]
pub static mut PICK_LAB: [i32; 10] = [0; 10];

#[cfg(feature = "pick-lab")]
/// Gameplay frame the pig is placed on; the world is streamed in by then.
const PIN_FRAME: u32 = 30;
#[cfg(feature = "pick-lab")]
/// 30 degrees below the horizon (4096 to the turn).
const PITCH: i16 = -341;

#[cfg(feature = "pick-lab")]
/// Hold the view and place the pig. Stick input is dropped so only the
/// scripted buttons act.
pub fn frame(frame: u32, player: &mut Player, ls: &mut (i16, i16), rs: &mut (i16, i16)) {
    *ls = (0, 0);
    *rs = (0, 0);
    player.yaw = 0; // facing +Z
    player.pitch = PITCH;
    if frame == PIN_FRAME {
        // Put the pig's middle (28 units over its feet) on the view ray,
        // assuming the ground there is level with the player's feet.
        let pa = (PITCH as i32 & 0x0FFF) as u16;
        let sp = -sincos::sin_q12(pa);
        let cp = sincos::cos_q12(pa);
        let d = (EYE_HEIGHT - 28) * cp / sp;
        mob::lab_pin_pick(mob::PIG, player.x, player.y, player.z + d);
    }
}

#[cfg(feature = "pick-lab")]
/// Publish what the crosshair and the attack reached this frame.
pub fn record(frame: u32, pick: &Pick, target: i32, mine_progress: u32) {
    let (h, x, z) = mob::lab_pig();
    unsafe {
        PICK_LAB = [
            h,
            x,
            z,
            pick.hit as i32,
            pick.bx,
            pick.by,
            pick.bz,
            mine_progress as i32,
            target,
            frame as i32,
        ];
    }
}

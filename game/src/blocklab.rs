//! Headless captures of the shaped blocks (`--features block-lab`, never
//! shipped): the first SELECT builds a proving ground in front of the player,
//! doors, beds, torches, levers, a ladder, dust with a lever and an inverter,
//! and every later SELECT moves the camera to the next view of it, so a journey
//! (tests/blocklab.toml) can take one checkpoint per feature.

use crate::bstate;
use crate::*;

/// Stock for the proving ground and the bow.
pub fn seed() {
    let mut i = 0;
    while i < PLACEABLE.len() {
        inv_give(PLACEABLE[i], 8);
        i += 1;
    }
    inv_give(BOW, 1);
    inv_give(ARROW, 40);
}

static mut BUILT: bool = false;
static mut VIEW: usize = 0;
static mut ORIGIN: (i32, i32, i32) = (0, 0, 0);

fn put(x: i32, y: i32, z: i32, b: u8) {
    set_block_i32(x, y, z, b);
    record_edit(x, y, z, b);
}

fn door(x: i32, y: i32, z: i32, f: u8, open: bool) {
    put(x, y, z, bstate::door(false, f, open));
    put(x, y + 1, z, bstate::door(true, f, open));
}

fn bed(x: i32, y: i32, z: i32, f: u8) {
    put(x, y, z, bstate::bed(false, f));
    put(
        x + bstate::FDX[f as usize],
        y,
        z + bstate::FDZ[f as usize],
        bstate::bed(true, f),
    );
}

/// The proving ground, 17 wide (x0 .. x0+16) and 14 deep, on a cobble floor at
/// floor level `y`.
fn build(x0: i32, y: i32, z0: i32) {
    // Floor and a clear room.
    let mut dx = 0;
    while dx < 17 {
        let mut dz = 0;
        while dz < 15 {
            put(x0 + dx, y, z0 + dz, COBBLE);
            let mut dy = 1;
            while dy < 7 {
                put(x0 + dx, y + dy, z0 + dz, AIR);
                dy += 1;
            }
            dz += 1;
        }
        dx += 1;
    }
    let s = y + 1;
    // Doors: closed, open, and the four facings, along z0+2.
    door(x0 + 1, s, z0 + 2, 0, false);
    door(x0 + 3, s, z0 + 2, 0, true);
    door(x0 + 5, s, z0 + 2, 1, false);
    door(x0 + 7, s, z0 + 2, 1, true);
    door(x0 + 9, s, z0 + 2, 2, false);
    door(x0 + 11, s, z0 + 2, 3, false);
    // Beds along z0+5 (foot first, head one cell on).
    bed(x0 + 1, s, z0 + 5, 0);
    bed(x0 + 4, s, z0 + 5, 1);
    bed(x0 + 8, s, z0 + 6, 2);
    bed(x0 + 11, s, z0 + 5, 3);
    // Torches, levers and a ladder on a wall of stone at z0+9.
    let mut wx = 0;
    while wx < 9 {
        let mut wy = 0;
        while wy < 4 {
            put(x0 + wx, s + wy, z0 + 9, STONE);
            wy += 1;
        }
        wx += 1;
    }
    put(x0 + 1, s, z0 + 8, bstate::make(TORCH, 0));
    put(x0 + 2, s + 1, z0 + 8, bstate::make(TORCH, 1 + 2)); // wall torch facing -Z
    put(x0 + 3, s, z0 + 8, bstate::make(RSTORCH, 0));
    put(x0 + 4, s + 1, z0 + 8, bstate::make(RSTORCH, 1 + 2));
    put(x0 + 5, s + 1, z0 + 8, bstate::lever(1 + 2, false));
    put(x0 + 6, s + 1, z0 + 8, bstate::lever(1 + 2, true));
    put(x0 + 7, s, z0 + 8, bstate::lever(0, false));
    put(x0 + 8, s, z0 + 8, bstate::make(LADDER, 2));
    put(x0 + 8, s + 1, z0 + 8, bstate::make(LADDER, 2));
    // The circuit along z0+12: lever, six dust, a door that opens with it.
    put(x0 + 1, s, z0 + 12, STONE);
    put(x0 + 1, s + 1, z0 + 12, bstate::lever(0, false));
    let mut k = 2;
    while k < 8 {
        put(x0 + k, s, z0 + 12, WIRE);
        k += 1;
    }
    door(x0 + 8, s, z0 + 12, 1, false);
    // A branch: a corner and a block step up.
    put(x0 + 4, s, z0 + 13, WIRE);
    put(x0 + 4, s, z0 + 14, WIRE);
    put(x0 + 5, s, z0 + 14, STONE);
    put(x0 + 5, s + 1, z0 + 14, WIRE);
    // TNT at the far end of the line, a piston beside the door.
    put(x0 + 10, s, z0 + 12, WIRE);
    put(x0 + 11, s, z0 + 12, WIRE);
}

/// (x, z, yaw, pitch) of each view, relative to the ground's origin.
const VIEWS: [(i32, i32, i32, i32); 8] = [
    (8, -3, 0, -120),    // the whole ground
    (4, -1, 0, -90),     // doors
    (6, 1, 0, -380),     // beds from above
    (4, 5, 0, -120),     // torches and levers
    (4, 6, 0x400, -300), // ladder and wall things, from the side
    (6, 10, 0, -300),    // dust, still dark
    (6, 10, 0, -300),    // lever thrown: dust lit
    (2, 14, 0x800, -200), // looking back down the line
];

pub fn select(player: &mut Player) {
    let (bx, bz) = (world_to_block_x(player.x), world_to_block_z(player.z));
    unsafe {
        if !BUILT {
            BUILT = true;
            let y = world::surface_y(bx, bz);
            ORIGIN = (bx - 6, y, bz + 3);
            build(bx - 6, y, bz + 3);
            redstone::mark_dirty();
            return;
        }
        let (ox, oy, oz) = ORIGIN;
        let v = VIEWS[VIEW.min(VIEWS.len() - 1)];
        // View 6 throws the lever the circuit starts at.
        if VIEW == 6 {
            blocks::use_lever(ox + 1, oy + 2, oz + 12);
        }
        player.x = block_to_world_x(ox + v.0) + BLOCK / 2;
        player.z = block_to_world_z(oz + v.1) + BLOCK / 2;
        player.y = (oy + 1) * BLOCK;
        player.vy = 0;
        player.yaw = v.2 as u16;
        player.pitch = v.3 as i16;
        VIEW += 1;
    }
}

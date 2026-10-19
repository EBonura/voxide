//! What the player does to the blocks that carry state: placing a door, a bed,
//! a torch, a lever or a ladder (which cell, which way it faces, what holds it
//! up), working a door or a lever, and what breaking one takes with it. The
//! state itself is bstate.rs, the boxes are shape.rs and the power is
//! redstone.rs; this is the rules between them.

use crate::bstate::{self, kind_of};
use crate::{
    container_room, get_block_i32, inv_take, is_lava, is_pushable, is_water, place_intersects_player,
    record_edit, replaceable, set_block_i32, sfx, shape, world, Pick, Player, AIR, BED, BLOCK,
    CACTUS, DOOR_C, GLASS, LADDER, LAVA, LEAVES, LEVER, PISTON, PISTON_HEAD, PLAYER_HALF_W,
    PLAYER_HEIGHT, RSTORCH, TORCH, WIRE,
};

/// What the gate reads back: the cell last placed, its second cell, and the
/// cell last used (after the use).
static mut LAST_PLACED: u8 = 0;
static mut LAST_SECOND: u8 = 0;
static mut LAST_USED: u8 = 0;

pub fn last() -> (u8, u8, u8) {
    unsafe { (LAST_PLACED, LAST_SECOND, LAST_USED) }
}

/// A face something can be fixed to: a whole solid cube. Not leaves, glass or
/// a cactus, which Java's torches and ladders will not stand on, and not
/// anything that is only a shape.
pub fn is_sturdy(b: u8) -> bool {
    let k = kind_of(b);
    b != AIR
        && shape::solid(b) == shape::Solid::Full
        && !shape::is_shape(b)
        && !is_water(b)
        && !is_lava(b)
        && !world::is_cross_plant(b)
        && !matches!(k, LEAVES | GLASS | CACTUS | LAVA)
}

/// Set a cell and log it.
fn put(x: i32, y: i32, z: i32, b: u8) {
    set_block_i32(x, y, z, b);
    record_edit(x, y, z, b);
    world::wake_fluid(x, y, z);
}

/// Mount of a torch or lever hung on the face `(nx, ny, nz)` of the block at
/// the other end of the ray (the cell it goes in minus the cell it hangs on).
/// 0 is the floor, 1..4 a wall facing 0..3; None where it cannot hang.
fn mount_for(nx: i32, ny: i32, nz: i32) -> Option<u8> {
    if ny == 1 {
        return Some(0);
    }
    if ny != 0 {
        return None;
    }
    bstate::facing_of_step(nx, nz).map(|f| 1 + f)
}

/// Place the selected item at the pick's place cell. True when something was
/// placed (and one taken from the pack).
#[inline(never)]
pub fn place_item(player: &Player, pick: &Pick) -> bool {
    let item = player.selected;
    let (px, py, pz) = (pick.px, pick.py, pick.pz);
    if !replaceable(get_block_i32(px, py, pz)) {
        return false;
    }
    let (nx, ny, nz) = (px - pick.bx, py - pick.by, pz - pick.bz);
    let hit = get_block_i32(pick.bx, pick.by, pick.bz);
    let facing = bstate::facing_of_yaw(player.yaw);
    // The cell that carries it: lower door half, the bed's foot, the lot.
    let mut second: Option<(i32, i32, i32, u8)> = None;
    let first = match item {
        DOOR_C => {
            // A door stands on something solid and needs the cell above too.
            if !is_sturdy(get_block_i32(px, py - 1, pz)) || !replaceable(get_block_i32(px, py + 1, pz))
            {
                return false;
            }
            second = Some((px, py + 1, pz, bstate::door(true, facing, false)));
            bstate::door(false, facing, false)
        }
        BED => {
            // Foot where you aim, head one cell on the way you face.
            let (hx, hz) = (px + bstate::FDX[facing as usize], pz + bstate::FDZ[facing as usize]);
            if !replaceable(get_block_i32(hx, py, hz)) {
                return false;
            }
            second = Some((hx, py, hz, bstate::bed(true, facing)));
            bstate::bed(false, facing)
        }
        TORCH | RSTORCH | LEVER => {
            if !is_sturdy(hit) {
                return false;
            }
            let Some(m) = mount_for(nx, ny, nz) else {
                return false;
            };
            match item {
                LEVER => bstate::lever(m, false),
                _ => bstate::make(item, m),
            }
        }
        LADDER => {
            // Hung on the side of a block, facing out.
            if !is_sturdy(hit) {
                return false;
            }
            let Some(f) = (if ny == 0 { bstate::facing_of_step(nx, nz) } else { None }) else {
                return false;
            };
            bstate::make(LADDER, f)
        }
        WIRE => {
            if !is_sturdy(get_block_i32(px, py - 1, pz)) {
                return false;
            }
            WIRE
        }
        crate::STAIRS_N => crate::stairs_for_yaw(player.yaw),
        PISTON => {
            // The face of a piston points at the player who set it (Java): the
            // opposite of the nearest direction they are looking in, straight
            // up or down past 45 degrees of pitch.
            let sp = crate::sincos::sin_q12((player.pitch as i32 & 0x0FFF) as u16);
            let look = if sp > 2896 {
                2
            } else if sp < -2896 {
                3
            } else {
                [4u8, 0, 5, 1][facing as usize]
            };
            bstate::piston(look ^ 1, false)
        }
        _ => item,
    };
    // The body must be clear of what goes in the cell(s).
    let clear = |x: i32, y: i32, z: i32, b: u8| -> bool {
        if shape::solid(b) == shape::Solid::Parts {
            !shape::body_hits(b, x, y, z, player.x, player.y, player.z, PLAYER_HALF_W, PLAYER_HEIGHT)
        } else {
            !place_intersects_player(player, x, y, z, b)
        }
    };
    if !clear(px, py, pz, first) {
        return false;
    }
    if let Some((x, y, z, b)) = second {
        if !clear(x, y, z, b) {
            return false;
        }
    }
    // A full store takes no new edits (an edit it could not keep would be lost
    // the first time the chunk streams out).
    if crate::edit_blocked(px, py, pz)
        || second.map_or(false, |(x, y, z, _)| crate::edit_blocked(x, y, z))
    {
        crate::edit_full_toast();
        return false;
    }
    if !container_room(item) || !inv_take(item) {
        return false;
    }
    put(px, py, pz, first);
    unsafe {
        LAST_PLACED = first;
        LAST_SECOND = 0;
    }
    if let Some((x, y, z, b)) = second {
        put(x, y, z, b);
        unsafe { LAST_SECOND = b };
    }
    // A door, piston or block of TNT set down beside power reacts at once.
    crate::redstone::poke(px, py, pz);
    true
}

// ---- Doors.

/// Open or close the door that has a half at (x, y, z): both halves.
pub fn set_door(x: i32, y: i32, z: i32, open: bool) {
    let b = get_block_i32(x, y, z);
    if !bstate::is_door(b) || bstate::door_open(b) == open {
        return;
    }
    let low = if bstate::door_upper(b) { y - 1 } else { y };
    let mut half = 0;
    while half < 2 {
        let cy = low + half;
        let c = get_block_i32(x, cy, z);
        if bstate::is_door(c) {
            put(x, cy, z, bstate::door(half == 1, bstate::door_facing(c), open));
        }
        half += 1;
    }
    sfx::door(open);
    unsafe { LAST_USED = get_block_i32(x, y, z) };
}

/// L2 on a door.
pub fn use_door(x: i32, y: i32, z: i32) {
    let open = bstate::door_open(get_block_i32(x, y, z));
    set_door(x, y, z, !open);
}

// ---- Levers.

/// Throw the lever at (x, y, z).
pub fn use_lever(x: i32, y: i32, z: i32) {
    let b = get_block_i32(x, y, z);
    if kind_of(b) != LEVER {
        return;
    }
    let on = !bstate::lever_on(b);
    put(x, y, z, bstate::lever(bstate::mount_of(b), on));
    sfx::lever(on);
    unsafe { LAST_USED = get_block_i32(x, y, z) };
    crate::redstone::lever_thrown(x, y, z);
}

// ---- Breaking.

/// The block a wall- or floor-mounted thing hangs on, as a cell offset.
fn hangs_on(b: u8) -> Option<(i32, i32, i32)> {
    let k = kind_of(b);
    match k {
        TORCH | RSTORCH | LEVER => {
            let m = bstate::mount_of(b);
            if m == 0 {
                Some((0, -1, 0))
            } else {
                let f = (m - 1) as usize;
                Some((-bstate::FDX[f], 0, -bstate::FDZ[f]))
            }
        }
        LADDER => {
            let f = bstate::ladder_facing(b) as usize;
            Some((-bstate::FDX[f], 0, -bstate::FDZ[f]))
        }
        WIRE => Some((0, -1, 0)),
        _ => None,
    }
}

/// Pop a block that has lost what held it, dropping its item.
fn pop(x: i32, y: i32, z: i32, drop_item: bool) {
    let b = get_block_i32(x, y, z);
    if b == AIR {
        return;
    }
    if drop_item {
        crate::spawn_drop(
            crate::block_to_world_x(x) + crate::BLOCK / 2,
            y * crate::BLOCK + crate::BLOCK / 4,
            crate::block_to_world_z(z) + crate::BLOCK / 2,
            b,
            (x ^ z ^ y) as u32,
        );
    }
    put(x, y, z, AIR);
}

/// Called after the block at (x, y, z) was broken (it held `was`): take the
/// other half of a door or a bed with it, and pop whatever hung on it.
#[inline(never)]
pub fn after_break(x: i32, y: i32, z: i32, was: u8) {
    if bstate::is_door(was) {
        let oy = if bstate::door_upper(was) { y - 1 } else { y + 1 };
        if bstate::is_door(get_block_i32(x, oy, z)) {
            pop(x, oy, z, false);
        }
    } else if bstate::is_bed(was) {
        // The other half is one cell along the bed's axis.
        let f = bstate::bed_facing(was) as usize;
        let s = if bstate::bed_head(was) { -1 } else { 1 };
        let (ox, oz) = (x + s * bstate::FDX[f], z + s * bstate::FDZ[f]);
        if bstate::is_bed(get_block_i32(ox, y, oz)) {
            pop(ox, y, oz, false);
        }
    } else if matches!(crate::kind_of(was), PISTON | PISTON_HEAD) {
        piston_partner(x, y, z, was);
    }
    settle(x, y, z);
    crate::redstone::mark_dirty();
}

/// Everything next to (x, y, z) that needed it to stand: torches, levers,
/// ladders and dust on its sides and top, a door's upper half over a lower
/// one that is gone, and a door standing on a floor that is gone.
pub fn settle(x: i32, y: i32, z: i32) {
    let mut d = 0;
    while d < 6 {
        let (dx, dy, dz) = crate::DIRS[d];
        let (nx, ny, nz) = (x + dx, y + dy, z + dz);
        let n = get_block_i32(nx, ny, nz);
        if let Some((hx, hy, hz)) = hangs_on(n) {
            // It hangs on the cell it points at: this one, if the offset is back.
            if (nx + hx, ny + hy, nz + hz) == (x, y, z) && !is_sturdy(get_block_i32(x, y, z)) {
                pop(nx, ny, nz, true);
            }
        } else if dy == 1
            && bstate::is_door(n)
            && !bstate::door_upper(n)
            && !is_sturdy(get_block_i32(x, y, z))
        {
            // The floor under a door is gone: the door goes with it.
            pop(nx, ny, nz, true);
            if bstate::is_door(get_block_i32(nx, ny + 1, nz)) {
                pop(nx, ny + 1, nz, false);
            }
        }
        d += 1;
    }
}

// ---- Pistons.

/// The most blocks a piston pushes (Java's).
const PUSH_LIMIT: i32 = 12;

/// Is the player's body in this cell? (A piston will not shove a block into it.)
fn body_in(x: i32, y: i32, z: i32) -> bool {
    let (px, py, pz) = unsafe { crate::tnt::PLAYER_POS };
    let (x0, y0, z0) = (x * BLOCK, y * BLOCK, z * BLOCK);
    px + PLAYER_HALF_W > x0
        && px - PLAYER_HALF_W < x0 + BLOCK
        && py + PLAYER_HEIGHT > y0
        && py < y0 + BLOCK
        && pz + PLAYER_HALF_W > z0
        && pz - PLAYER_HALF_W < z0 + BLOCK
}

/// A cell a pushed line can end in: nothing there, or something a piston
/// destroys (fluid, a plant, dust, a torch, a lever, a ladder).
fn gives_way(b: u8) -> bool {
    b == AIR
        || is_water(b)
        || is_lava(b)
        || world::is_cross_plant(b)
        || shape::solid(b) == shape::Solid::No
}

/// A powered piston pushes (or lets go).
pub fn drive_piston(x: i32, y: i32, z: i32, on: bool) {
    let b = get_block_i32(x, y, z);
    if crate::kind_of(b) != PISTON {
        return;
    }
    let d = bstate::piston_dir(b);
    if on && !bstate::piston_extended(b) {
        extend(x, y, z, d);
    } else if !on && bstate::piston_extended(b) {
        retract(x, y, z, d);
    }
}

/// Push the line of blocks in front of the piston one cell on and put the
/// arm in the cell it leaves. Nothing moves if the line is longer than 12, ends
/// in something a piston cannot move, or would shove a block into the player.
fn extend(x: i32, y: i32, z: i32, d: u8) {
    let (dx, dy, dz) = crate::DIRS[d as usize];
    let (sx, sy, sz) = (x + dx, y + dy, z + dz);
    // How many blocks are in the line, and what ends it.
    let mut n = 0;
    loop {
        let c = get_block_i32(sx + dx * n, sy + dy * n, sz + dz * n);
        if gives_way(c) {
            break;
        }
        if !is_pushable(c) || n >= PUSH_LIMIT {
            return;
        }
        n += 1;
    }
    // The cell the far end moves into, and the cells it fills on the way.
    let mut i = 0;
    while i <= n {
        if body_in(sx + dx * i, sy + dy * i, sz + dz * i)
            && shape::solid(get_block_i32(sx + dx * (i.max(1) - 1), sy + dy * (i.max(1) - 1), sz + dz * (i.max(1) - 1)))
                != shape::Solid::No
        {
            return;
        }
        i += 1;
    }
    if crate::edit_blocked(sx + dx * n, sy + dy * n, sz + dz * n) {
        crate::edit_full_toast();
        return;
    }
    // What it ends in is destroyed, with its item.
    let end = get_block_i32(sx + dx * n, sy + dy * n, sz + dz * n);
    if end != AIR && !is_water(end) && !is_lava(end) {
        pop(sx + dx * n, sy + dy * n, sz + dz * n, true);
    }
    // Far block first, so each lands on the cell the last one left.
    let mut i = n;
    while i > 0 {
        let from = get_block_i32(sx + dx * (i - 1), sy + dy * (i - 1), sz + dz * (i - 1));
        put(sx + dx * i, sy + dy * i, sz + dz * i, from);
        i -= 1;
    }
    put(sx, sy, sz, bstate::piston_head(d));
    put(x, y, z, bstate::piston(d, true));
    sfx::piston(true);
}

/// Take the arm back in. The blocks it pushed stay where they are.
fn retract(x: i32, y: i32, z: i32, d: u8) {
    let (dx, dy, dz) = crate::DIRS[d as usize];
    let h = get_block_i32(x + dx, y + dy, z + dz);
    if crate::kind_of(h) == PISTON_HEAD && bstate::piston_dir(h) == d {
        put(x + dx, y + dy, z + dz, AIR);
    }
    put(x, y, z, bstate::piston(d, false));
    sfx::piston(false);
}

/// The other half of a piston that was broken: the arm of a body, the body of
/// an arm (left retracted).
fn piston_partner(x: i32, y: i32, z: i32, was: u8) {
    let d = bstate::piston_dir(was);
    let (dx, dy, dz) = crate::DIRS[d as usize];
    if crate::kind_of(was) == PISTON {
        if bstate::piston_extended(was) {
            let h = get_block_i32(x + dx, y + dy, z + dz);
            if crate::kind_of(h) == PISTON_HEAD {
                pop(x + dx, y + dy, z + dz, false);
            }
        }
    } else {
        let p = get_block_i32(x - dx, y - dy, z - dz);
        if bstate::piston_extended(p) {
            put(x - dx, y - dy, z - dz, bstate::piston(d, false));
        }
    }
}

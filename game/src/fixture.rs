//! Headless UI fixture (`--features ui-fixture`, never shipped): a stocked
//! starting inventory, and SELECT in the world to stand up a crafting table,
//! chest or furnace at the crosshair, so a `frontend launch --press` script
//! can reach every menu without mining and crafting its way there first.
//! SELECT while sneaking sets the enchant levels and the food pouch, or, at a
//! chest, crosses dimensions in place and builds a chest at the same spot;
//! at a crafting table, opens a void portal underfoot; at a furnace, builds
//! and lights an obsidian portal there; at portal sheet, opens it underfoot;
//! at obsidian, regenerates every loaded chunk; in the Inferno at portal
//! sheet, steps you out to face it if you stand in it, else regenerates; in
//! the Void, slays the dragon.
//!
//! SELECT on a non-station block places a crafting table in front of it;
//! SELECT on a station turns it into the next one (table, chest, furnace).

use crate::*;

/// Starting stock: a spread of blocks and materials, one count past what a
/// byte holds (the old save clamped at 255), and the ingredients for the
/// stone pickaxe short a stick.
const STOCK: [(u8, u16); 50] = [
    (GRASS, 4),
    (STONE, 21),
    (COBBLE, 300),
    (WOOD, 5),
    (PLANK, 9),
    (LEAVES, 12),
    (SAND, 30),
    (SNOW, 2),
    (BRICK, 8),
    (CHEST, 1),
    (TORCH, 12),
    (COAL_ORE, 7),
    (IRON_ORE, 5),
    (STICK, 1),
    (STRING, 3),
    (BONE, 2),
    (GUNPOWDER, 1),
    (RAW_MEAT, 4),
    (BREAD, 3),
    (ARROW, 0),
    (CLAY, 6),
    (FURNACE, 1),
    // One of each item that got its own icon in the second set.
    (WATER_BUCKET, 1),
    (LAVA_BUCKET, 1),
    (FLINT_STEEL, 1),
    (BOTTLE, 3),
    (POTION_AWKWARD, 1),
    (POTION_SPEED, 1),
    (POTION_STRENGTH, 1),
    (POTION_REGEN, 1),
    (POTION_FIRE, 1),
    (BONEMEAL, 6),
    (WIRE, 16),
    (SUGAR_CANE, 5),
    (WHEAT_ITEM, 9),
    (FISHING_ROD, 1),
    (EMBER_CAP, 2),
    (EMBER_ROD, 2),
    (WAILER_TEAR, 1),
    (MAGMA_PASTE, 1),
    (VOID_PEARL, 2),
    (VOID_EYE, 1),
    (BED, 1),
    (PISTON, 2),
    (ENCHANT, 1),
    (FENCE, 8),
    (GLASS, 6),
    (SLAB, 12),
    (STAIRS_N, 4),
    (SAPLING, 3),
];

pub fn seed() {
    let mut i = 0;
    while i < STOCK.len() {
        inv_give(STOCK[i].0, STOCK[i].1);
        i += 1;
    }
}

/// Look a little down at spawn so the crosshair lands on the ground within
/// reach, where SELECT can build; and start with some gear.
pub fn spawn_pitch(p: &mut Player) {
    p.pitch = -300;
    p.pick = 1; // a wood pickaxe and iron armour, so the gear column shows
    p.armor = 1;
}

pub fn select(pick: &Pick, player: &mut Player) {
    // SELECT while sneaking (CIRCLE held) grants the state only the enchanting
    // table and fishing give, so a save test can see it round-trip. Aimed at a
    // chest, it instead crosses to the other dimension in place and stands a
    // chest at the very same x,y,z there, the case the dimension key is for.
    if player.sneaking {
        if pick.hit && get_block_i32(pick.bx, pick.by, pick.bz) == CHEST {
            same_spot_elsewhere(pick, player);
            return;
        }
        // At a crafting table: void portal sheet at your feet, so the next
        // second takes you to the Void through the real portal path.
        if pick.hit && get_block_i32(pick.bx, pick.by, pick.bz) == CRAFT_TABLE {
            let (bx, by, bz) = (
                world_to_block_x(player.x),
                world_to_block_y(player.y + 8),
                world_to_block_z(player.z),
            );
            set_block_i32(bx, by, bz, VOID_PORTAL);
            set_block_i32(bx, by + 1, bz, VOID_PORTAL);
            return;
        }
        match if pick.hit {
            get_block_i32(pick.bx, pick.by, pick.bz)
        } else {
            AIR
        } {
            FURNACE => {
                build_lit_portal(pick, player);
                return;
            }
            // Standing in a portal (the crosshair starts inside it): step
            // out and face a portal, another one if there is one near.
            PORTAL
                if get_block_i32(
                    world_to_block_x(player.x),
                    world_to_block_y(player.y + 8),
                    world_to_block_z(player.z),
                ) == PORTAL =>
            {
                face_portal(player);
                return;
            }
            // Facing a portal in the Inferno: regenerate.
            PORTAL if world::dimension() == world::DIM_INFERNO => {
                regenerate_ring(player);
                return;
            }
            PORTAL => {
                // Inferno sheet at your feet: the real portal path takes you.
                let (bx, by, bz) = (
                    world_to_block_x(player.x),
                    world_to_block_y(player.y + 8),
                    world_to_block_z(player.z),
                );
                set_block_i32(bx, by, bz, PORTAL);
                set_block_i32(bx, by + 1, bz, PORTAL);
                return;
            }
            OBSIDIAN if world::dimension() != world::DIM_INFERNO => {
                regenerate_ring(player);
                return;
            }
            _ => {}
        }
        // In the Void: the dragon dies as if to a last hit.
        if world::dimension() == world::DIM_VOID {
            mob::slay_dragon();
            return;
        }
        player.sharpness = 2;
        player.protection = 3;
        player.food_items += 5;
        return;
    }
    if !pick.hit {
        return;
    }
    let (bx, by, bz) = (pick.bx, pick.by, pick.bz);
    let next = match get_block_i32(bx, by, bz) {
        CRAFT_TABLE => CHEST,
        CHEST => FURNACE,
        FURNACE => CRAFT_TABLE,
        _ => {
            let (px, py, pz) = (pick.px, pick.py, pick.pz);
            if get_block_i32(px, py, pz) == AIR {
                set_block_i32(px, py, pz, CRAFT_TABLE);
                record_edit(px, py, pz, CRAFT_TABLE);
            }
            return;
        }
    };
    let old = get_block_i32(bx, by, bz);
    if old == CHEST {
        if let Some(i) = chest_find(bx, by, bz) {
            unsafe { CHEST_USED[i] = false };
        }
    } else if old == FURNACE {
        if let Some(i) = furn_find(bx, by, bz) {
            unsafe { FURN_USED[i] = false };
        }
    }
    set_block_i32(bx, by, bz, next);
    record_edit(bx, by, bz, next);
    if next == CHEST {
        chest_register(bx, by, bz);
        if let Some(i) = chest_find(bx, by, bz) {
            unsafe {
                CHEST_INV[i][COBBLE as usize] = 12;
                CHEST_INV[i][BRICK as usize] = 64;
                CHEST_INV[i][GRASS as usize] = 4;
                CHEST_INV[i][IRON_INGOT as usize] = 3;
            }
        }
    } else if next == FURNACE {
        furn_register(bx, by, bz);
    }
}

/// Swap overworld and Inferno without moving, then put a chest at the aimed
/// block with a clear line to it and floor underfoot.
fn same_spot_elsewhere(pick: &Pick, player: &Player) {
    let to = if world::dimension() == world::DIM_OVERWORLD {
        world::DIM_INFERNO
    } else {
        world::DIM_OVERWORLD
    };
    let px = world_to_block_x(player.x);
    let py = world_to_block_y(player.y);
    let pz = world_to_block_z(player.z);
    world::set_dimension(to, px, pz, |_, _| {});
    let (x0, x1) = (px.min(pick.bx) - 1, px.max(pick.bx) + 1);
    let (z0, z1) = (pz.min(pick.bz) - 1, pz.max(pick.bz) + 1);
    let mut x = x0;
    while x <= x1 {
        let mut z = z0;
        while z <= z1 {
            set_block_i32(x, py - 1, z, COBBLE);
            record_edit(x, py - 1, z, COBBLE);
            let mut y = py;
            while y <= py + 3 {
                set_block_i32(x, y, z, AIR);
                record_edit(x, y, z, AIR);
                y += 1;
            }
            z += 1;
        }
        x += 1;
    }
    set_block_i32(pick.bx, pick.by, pick.bz, CHEST);
    record_edit(pick.bx, pick.by, pick.bz, CHEST); // so a save keeps the spot
    chest_register(pick.bx, pick.by, pick.bz);
    world::remesh_loaded();
}

/// Replace the aimed furnace with an obsidian frame a player could have built,
/// across the line of sight, and light it with the game's own flint path.
/// Every block goes in the edit log, as the player's placements do.
fn build_lit_portal(pick: &Pick, player: &Player) {
    let (ix, iy, iz) = (pick.bx, pick.by, pick.bz);
    if let Some(i) = furn_find(ix, iy, iz) {
        unsafe { FURN_USED[i] = false };
    }
    let dx = ix - world_to_block_x(player.x);
    let dz = iz - world_to_block_z(player.z);
    // Looking along X, the frame runs along Z, and the other way round.
    let (ax, az) = if dx.abs() > dz.abs() { (0, 1) } else { (1, 0) };
    let put = |x: i32, y: i32, z: i32, b: u8| {
        set_block_i32(x, y, z, b);
        record_edit(x, y, z, b);
    };
    let mut w = -1;
    while w <= 2 {
        let mut h = -1;
        while h <= 3 {
            let (x, z) = (ix + ax * w, iz + az * w);
            let edge = w == -1 || w == 2 || h == -1 || h == 3;
            put(x, iy + h, z, if edge { OBSIDIAN } else { AIR });
            h += 1;
        }
        w += 1;
    }
    light_portal(ix, iy, iz);
}

/// Throw away and regenerate every loaded chunk, the way walking out of range
/// and back does: out to the other dimension and home again, in place.
fn regenerate_ring(player: &Player) {
    let (bx, bz) = (world_to_block_x(player.x), world_to_block_z(player.z));
    let here = world::dimension();
    let other = if here == world::DIM_OVERWORLD {
        world::DIM_INFERNO
    } else {
        world::DIM_OVERWORLD
    };
    world::set_dimension(other, bx, bz, |_, _| {});
    world::set_dimension(here, bx, bz, |_, _| {});
    // set_dimension also digs an arrival pocket around the player, raw, after
    // the edit replay; streaming never does. Replay the log over it, as a load
    // does, so what is left is what regeneration alone keeps.
    crate::save::apply_edits();
}

/// Stand two cells in front of a near portal's lower-left sheet cell, looking
/// level at it along +Z (return portals run along X, and so do the fixture's
/// own when built looking along Z, as at spawn).
fn face_portal(player: &mut Player) {
    let (bx, by, bz) = (
        world_to_block_x(player.x),
        world_to_block_y(player.y),
        world_to_block_z(player.z),
    );
    // Two passes: first skip the sheet you stand in, then take any.
    let mut pass = 0;
    while pass < 2 {
        if face_portal_pass(player, bx, by, bz, pass == 0) {
            return;
        }
        pass += 1;
    }
}

fn face_portal_pass(player: &mut Player, bx: i32, by: i32, bz: i32, other: bool) -> bool {
    let mut dx = -12;
    while dx <= 12 {
        let mut dz = -12;
        while dz <= 12 {
            let mut dy = -8;
            while dy <= 8 {
                let (x, y, z) = (bx + dx, by + dy, bz + dz);
                let mine = z == bz && (x == bx || x + 1 == bx);
                if get_block_i32(x, y, z) == PORTAL
                    && get_block_i32(x - 1, y, z) != PORTAL
                    && get_block_i32(x, y - 1, z) != PORTAL
                    && !(other && mine)
                {
                    player.x = block_to_world_x(x + 1);
                    player.z = block_to_world_z(z - 2) + BLOCK / 2;
                    player.y = y * BLOCK;
                    player.vy = 0;
                    player.fall_peak = player.y;
                    player.yaw = 0;
                    player.pitch = 0;
                    return true;
                }
                dy += 1;
            }
            dz += 1;
        }
        dx += 1;
    }
    false
}

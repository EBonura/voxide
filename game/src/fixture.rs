//! Headless UI fixture (`--features ui-fixture`, never shipped): a stocked
//! starting inventory, and SELECT in the world to stand up a crafting table,
//! chest or furnace at the crosshair, so a `frontend launch --press` script
//! can reach every menu without mining and crafting its way there first.
//! SELECT while sneaking sets the enchant levels and the food pouch, or, at a
//! chest, crosses dimensions in place and builds a chest at the same spot;
//! at a crafting table, opens a void portal underfoot; in the Void, slays
//! the dragon.
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

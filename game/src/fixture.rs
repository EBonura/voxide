//! Headless UI fixture (`--features ui-fixture`, never shipped): a stocked
//! starting inventory, and SELECT in the world to stand up a crafting table,
//! chest or furnace at the crosshair, so a `frontend launch --press` script
//! can reach every menu without mining and crafting its way there first.
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

pub fn select(pick: &Pick) {
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

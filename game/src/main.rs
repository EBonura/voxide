//! voxide -- a Minecraft-like survival sandbox for PlayStation 1.
//!
//! No heap, fixed-point math, GTE (COP2) hardware projection, ordering-table
//! painter's depth.
//! The world is chunked and streamed (see `world`); this file owns rendering,
//! the player, input, HUD, and the survival loop.

#![no_std]
#![no_main]
#![allow(static_mut_refs)]
#![feature(asm_experimental_arch)] // psx_gte::mtc2!/mfc2! expand to MIPS asm

extern crate psx_rt;

mod bonnie;
mod mob;
mod save;
mod sfx;
mod sfxdata;
mod telemetry;
mod tex;
mod texdata;
mod world;

// Profiler stage IDs (PSoXide --profile-log). Arbitrary distinct numbers; the
// CSV reports cycles per id per frame. Mapping: sim / gen / mesh / render.
const ST_SIM: u16 = telemetry::stage::UPDATE; // 1
const ST_GEN: u16 = telemetry::stage::CD_WORLD_PACK_STREAM; // 25
const ST_MESH: u16 = telemetry::stage::ROOM_SURFACE_CACHE; // 21
const ST_RENDER: u16 = telemetry::stage::RENDER; // 3
// TEMP render sub-stages for the GTE-vs-software profile (arbitrary distinct ids).
const ST_R_SKY: u16 = 40;
const ST_R_WORLD: u16 = 42;
const ST_R_MOBS: u16 = 43;
const ST_R_TAIL: u16 = 44; // particles + pick + held item + HUD + rain
const ST_OTCLEAR: u16 = 46; // TEMP: OT.clear span

use core::cmp::{max, min};

use psx_font::{fonts::BASIC, FontAtlas};
use psx_gpu::{
    self as gpu,
    framebuf::FrameBuffer,
    material::{BlendMode, TextureMaterial, TextureWindow},
    ot::OrderingTable,
    prim::{QuadFlat, QuadTexturedGouraud, QuadTexturedMaterial, TriTexturedGouraud},
    Resolution, VideoMode,
};
use psx_gte::math::{Mat3I16, Vec3I16, Vec3I32};
use psx_gte::scene;
use psx_math::sincos;
use psx_pad::{
    button, enable_analog_port1, poll_port1, ActionBinding, ActionMap, ButtonState, Deadzone,
    PadState,
};
use psx_rt::{interrupts, tty};
use psx_settings::Profile;
use psx_vram::{Clut, TexDepth, Tpage};

use tex::BlockTex;

const SCREEN_W: u16 = 320;
const SCREEN_H: u16 = 240;
const CX: i16 = 160;
const CY: i16 = 120;
const PROJ_H: i32 = 178;
const NEAR_Z: i32 = 18;
// TWELVE blocks. The 30fps mandate set this, measured on the demo route
// (worst case: walking into terrain, then digging nose-first):
//     FAR_Z 1792  15.2 fps mean         (the 28-block ring era)
//     FAR_Z 1216  19.1                  (19 blocks, the old "GTE pass 3" mark)
//     FAR_Z 1024  21.1
//     FAR_Z  896  25.4 / walk 99% at 30 (with RENDER_R 1)
//     FAR_Z  768  29.0 mean, 93% of ALL frames at 30; walking, placing and
//                 idling LOCKED at 30; residual dips only on block-break
//                 frames (particles + remesh) and heavy vista turns.
// Past ~17 blocks the route stops being face-bound and streaming bursts
// dominate -- RENDER_R (world.rs) falling to 1 was worth more than any
// distance step. The 4-level lighting measured FREE at this range (the
// merge-break inflation scales with far-face area). Distance fog reaches
// full ground-haze by the last band, so the short horizon reads as weather.
const FAR_Z: i32 = 1024;
// Side/bottom faces stop two bands earlier: beyond that every face is fully
// hazed, so only the SILHOUETTE carries information, and a heightfield's
// silhouette is its top faces (the 25..28-block-era ring measured far sides
// at over 2x the tops' cost for no visible difference).
const FAR_SIDE_Z: i32 = 896;

const BLOCK: i32 = 64;

/// Shown on the main menu; bump the patch digit with every released change.
/// Drawn bottom-left on the title screen, and read by the demo disc for its
/// carousel. Taken from Cargo rather than written out again: the manifest said
/// 0.1.0 while this said V0.1.9 and the tag said v0.1.9, so the disc believed
/// the one source nobody had been maintaining.
const VERSION: &str = concat!("V", env!("CARGO_PKG_VERSION"));

const AIR: u8 = 0;
const GRASS: u8 = 1;
const DIRT: u8 = 2;
const STONE: u8 = 3;
const WOOD: u8 = 4;
const LEAVES: u8 = 5;
const SAND: u8 = 6;
const WATER: u8 = 7;
const COAL_ORE: u8 = 8;
const IRON_ORE: u8 = 9;
const GOLD_ORE: u8 = 10;
const DIAMOND_ORE: u8 = 11;
const LAVA: u8 = 12;
const SNOW: u8 = 13;
const GLASS: u8 = 14; // transparent, smelted from sand
const CHEST: u8 = 15; // placed interactable storage block
const CRAFT_TABLE: u8 = 93; // placed crafting station: use (L2) opens the full recipe menu
// Crafted items live above the block ids in the same INV array.
const PLANK: u8 = 16;
const STICK: u8 = 17;
const FURNACE: u8 = 18; // placed interactable smelter block
const IRON_INGOT: u8 = 19; // smelted from iron ore; used for iron tools
const BED: u8 = 20; // sleep through the night
const WIRE: u8 = 21; // redstone wire (carries power)
const TORCH: u8 = 22; // redstone torch (power source)
const PISTON: u8 = 23; // pushes the block above it when powered
const TNT: u8 = 24; // ignites when redstone-powered, then explodes
const WHEAT: u8 = 25; // growing crop (planted from seeds, matures over time)
const WHEAT_RIPE: u8 = 26; // mature crop, harvest for wheat + seeds
const SEEDS: u8 = 27; // item: plant on dirt/grass to grow wheat
const WHEAT_ITEM: u8 = 28; // item: harvested wheat (3 -> bread)
const BREAD: u8 = 29; // item: food, eaten when hungry
const BOW: u8 = 30; // item: select + L2 to fire an arrow
const ARROW: u8 = 31; // item: ammo for the bow
const WOOL: u8 = 32; // placeable block, dropped by sheep (needs 6-bit packing)
const LADDER: u8 = 33; // placeable, climbable
const BUCKET: u8 = 34; // item: scoop water/lava, or empty into the world
const WATER_BUCKET: u8 = 35; // item: filled bucket (places WATER)
const LAVA_BUCKET: u8 = 36; // item: filled bucket (places LAVA)
const BONE: u8 = 37; // item: dropped by skeletons; crafts into bonemeal
const BONEMEAL: u8 = 38; // item: L2 on a crop to instantly grow it
const FISHING_ROD: u8 = 39; // item: L2 on water to cast and catch fish (food)
const COBBLE: u8 = 40; // placeable block: what mining stone yields; smelts back to stone
const ENCHANT: u8 = 41; // placeable: R3 spends XP levels for a mining-efficiency boost
const SAPLING: u8 = 42; // placeable on soil; grows into a tree (renewable wood)
const RAW_MEAT: u8 = 43; // item: dropped by passive mobs; furnace-cooks
const COOKED_MEAT: u8 = 44; // item: best food; smelted from raw meat
const GUNPOWDER: u8 = 45; // item: dropped by slain sappers; crafts TNT
const DOOR_C: u8 = 46; // placeable door, closed (solid); R3 opens
const DOOR_O: u8 = 47; // open door: invisible + walk-through; R3 closes
const CACTUS: u8 = 48; // desert plant; hurts on contact
const CLAY: u8 = 49; // found under shallow water; smelts into brick
const BRICK: u8 = 50; // placeable brick block (smelted clay)
// Decorative cross-sprite plants scattered on grass at gen time. Walk-through,
// never in the inventory -- purely to make the world feel alive.
const FLOWER_R: u8 = 51; // red flower
const FLOWER_Y: u8 = 52; // yellow flower
const TALL_GRASS: u8 = 53; // tuft of grass blades
// Non-full-block shapes. The greedy mesher only speaks in whole cubes, so these
// are excluded from it and drawn by the same per-block pass the cross-sprite
// plants use. ponytail: no stairs -- those need four ids for the four facings
// and the 6-bit block field has no room left; widening BLOCK_BITS to 7 is the
// documented way out if it ever matters.
const SLAB: u8 = 61; // half-height cobble; you stand on it at half a block
const FENCE: u8 = 62; // thin wood post; blocks movement, does not block sight
// Stairs, one id per facing -- the direction the HIGH step faces away from,
// i.e. the side you walk up from. The inventory carries STAIRS_N; placement
// picks the facing from where you are standing and `drop_of` maps them all back.
const STAIRS_N: u8 = 63; // high half on +Z
const STAIRS_E: u8 = 64; // high half on +X
const STAIRS_S: u8 = 65; // high half on -Z
const STAIRS_W: u8 = 66; // high half on -X

// Flowing lava. Java's overworld lava reaches 3 blocks, not water's 7, and
// oozes several times slower.
const LAVA_F1: u8 = 67;
const LAVA_F3: u8 = LAVA_F1 + 2;
pub const LAVA_MAX_RUN: u8 = 3;

/// Fire. Drawn as a cross-sprite like the plants, walk-through, and burns out.
/// Lava lights flammable neighbours; the fire then eats what it stands on.
/// Java's Inferno fog, and the ambient light level down there: dim, but never
/// pitch black, because there is no day/night cycle to lift it.
const INFERNO_FOG: (u8, u8, u8) = (58, 18, 16);
const INFERNO_LIGHT: u8 = 88;
/// The End's void: near-black with a violet cast, and a flat dim light with no
/// day to it.
const VOID_FOG: (u8, u8, u8) = (16, 10, 26);
const VOID_LIGHT: u8 = 76;
/// Frames of standing in portal sheet before it takes you.
const PORTAL_DWELL: u16 = 45;
/// Sentinel: standing in the portal you just arrived in, so it will not fire.
const PORTAL_IMMUNE: u16 = u16::MAX;
const FIRE: u8 = 70;
/// Obsidian: what water makes of a lava SOURCE (Java), and the only thing that
/// survives a blast. Diamond-tier to mine, and slow.
const OBSIDIAN: u8 = 71;
// The Inferno. It reuses the SAME chunk ring as the overworld -- the dimension
// is a generator switch, not a second world in RAM, which is what makes it fit
// on the machine at all.
const CINDERSTONE: u8 = 72;
const SINK_SAND: u8 = 73; // walks slow, as in Java
const LUMISTONE: u8 = 74;
const PORTAL: u8 = 75; // the sheet inside an obsidian frame; walk in to travel
const FLINT_STEEL: u8 = 76; // item: lights a portal frame, or sets a fire
// Brewing. Java's chain is bottle -> awkward (ember cap) -> effect
// (ingredient); ours is the same shape, brewed through the recipe book the way
// this port already adapts the crafting grid.
const EMBER_CAP: u8 = 77; // grows on sink sand down there; the base ingredient
const BOTTLE: u8 = 78;
const POTION_AWKWARD: u8 = 79; // no effect on its own, as in Java
const POTION_SPEED: u8 = 80;
const POTION_STRENGTH: u8 = 81;
const POTION_REGEN: u8 = 82;
const POTION_FIRE: u8 = 83;
// The End. Same trick as the Inferno -- a generator over the same chunk ring.
const VOID_STONE: u8 = 84;
const VOID_PORTAL: u8 = 85; // sheet inside an obsidian frame, lit with an eye
const VOID_EYE: u8 = 86; // item: lights an obsidian frame for the End instead
const STRING: u8 = 87; // item: spiders drop it; the bow's actual cord
const SUGAR_CANE: u8 = 88; // plant: grows on sand beside water, brews into speed
const VOID_PEARL: u8 = 89; // item: wraiths drop it; 4 + coal = a void eye
// Brewing ingredients, now with the Java sources rather than stand-ins.
const EMBER_ROD: u8 = 90;
const WAILER_TEAR: u8 = 91;
const MAGMA_PASTE: u8 = 92;

#[inline]
fn is_portal(b: u8) -> bool {
    b == PORTAL || b == VOID_PORTAL
}

#[inline]
fn is_potion(b: u8) -> bool {
    b >= POTION_AWKWARD && b <= POTION_FIRE
}

/// Frames an effect lasts. Java's are 3 to 8 minutes; at 30fps that is a very
/// long time to carry a buff on a machine with no status HUD, so these are
/// closer to a minute.
const POTION_TIME: u16 = 1800;

/// Fill a 2x3 obsidian frame with portal sheet. `(bx, by, bz)` is the block the
/// player struck; we look for the frame around and above it. Returns true if a
/// portal lit.
///
/// ponytail: only the minimum 2-wide, 3-tall frame Java allows, and only on the
/// two axes -- no larger frames. That is the shape everybody builds anyway, and
/// scanning for arbitrary rectangles is a lot of code for a portal.
#[inline(never)]
fn light_portal(bx: i32, by: i32, bz: i32) -> bool {
    light_frame(bx, by, bz, PORTAL)
}

#[inline(never)]
fn light_frame(bx: i32, by: i32, bz: i32, sheet: u8) -> bool {
    // Try the frame running along X, then along Z.
    let axes: [(i32, i32); 2] = [(1, 0), (0, 1)];
    let mut a = 0;
    while a < 2 {
        let (ax, az) = axes[a];
        // The struck block is the bottom-left interior cell, or one to its right.
        let mut off = 0;
        while off < 2 {
            let (ix, iz) = (bx - ax * off, bz - az * off);
            if portal_frame_ok(ix, by, iz, ax, az) {
                let mut w = 0;
                while w < 2 {
                    let mut h = 0;
                    while h < 3 {
                        let (px, pz) = (ix + ax * w, iz + az * w);
                        set_block_i32(px, by + h, pz, sheet);
                        record_edit(px, by + h, pz, sheet);
                        h += 1;
                    }
                    w += 1;
                }
                return true;
            }
            off += 1;
        }
        a += 1;
    }
    false
}

/// A 2x3 interior of air, ringed by obsidian on the frame's own plane.
#[inline(never)]
fn portal_frame_ok(ix: i32, iy: i32, iz: i32, ax: i32, az: i32) -> bool {
    // Interior must be clear.
    let mut w = 0;
    while w < 2 {
        let mut h = 0;
        while h < 3 {
            if get_block_i32(ix + ax * w, iy + h, iz + az * w) != AIR {
                return false;
            }
            h += 1;
        }
        w += 1;
    }
    // Sill and lintel.
    let mut w = 0;
    while w < 2 {
        let (px, pz) = (ix + ax * w, iz + az * w);
        if get_block_i32(px, iy - 1, pz) != OBSIDIAN || get_block_i32(px, iy + 3, pz) != OBSIDIAN {
            return false;
        }
        w += 1;
    }
    // Jambs.
    let mut h = 0;
    while h < 3 {
        if get_block_i32(ix - ax, iy + h, iz - az) != OBSIDIAN
            || get_block_i32(ix + ax * 2, iy + h, iz + az * 2) != OBSIDIAN
        {
            return false;
        }
        h += 1;
    }
    true
}

#[inline]
fn is_flammable(b: u8) -> bool {
    b == WOOD || b == PLANK || b == LEAVES || b == WOOL || b == FENCE || b == CRAFT_TABLE
}

#[inline]
fn is_lava(b: u8) -> bool {
    b == LAVA || (b >= LAVA_F1 && b <= LAVA_F3)
}

#[inline]
fn lava_level(b: u8) -> u8 {
    if b == LAVA {
        0
    } else if b >= LAVA_F1 && b <= LAVA_F3 {
        b - LAVA_F1 + 1
    } else {
        LAVA_MAX_RUN + 1
    }
}

#[inline]
fn lava_of_level(level: u8) -> u8 {
    LAVA_F1 + level - 1
}

#[inline]
fn is_stairs(b: u8) -> bool {
    b >= STAIRS_N && b <= STAIRS_W
}

/// Drawn per-block instead of meshed, and see-through to the mesher so the
/// blocks behind them still emit their faces.
#[inline]
fn is_small_block(b: u8) -> bool {
    b == SLAB || b == FENCE || is_stairs(b)
}

/// World-unit box of a stair's UPPER step, within the block at `(bx, by, bz)`.
/// The lower half is a full-footprint slab; this is the half that sits on top.
fn stair_step_box(blk: u8, bx: i32, by: i32, bz: i32) -> (i32, i32, i32, i32, i32, i32) {
    let (x, y, z) = (bx * BLOCK, by * BLOCK, bz * BLOCK);
    let h = BLOCK / 2;
    match blk {
        STAIRS_N => (x, y + h, z + h, x + BLOCK, y + BLOCK, z + BLOCK),
        STAIRS_S => (x, y + h, z, x + BLOCK, y + BLOCK, z + h),
        STAIRS_E => (x + h, y + h, z, x + BLOCK, y + BLOCK, z + BLOCK),
        _ => (x, y + h, z, x + h, y + BLOCK, z + BLOCK),
    }
}

/// Which stair id to place, given the yaw the player is facing. The high step
/// goes on the far side, so you always approach the low step.
fn stairs_for_yaw(yaw: u16) -> u8 {
    // Yaw is a Q12 angle; quarter turns at the diagonals so each facing owns 90
    // degrees. Yaw 0 looks along +Z.
    let q = (((yaw as i32 & 0x0FFF) + 512) >> 10) & 3;
    match q {
        0 => STAIRS_N,
        1 => STAIRS_E,
        2 => STAIRS_S,
        _ => STAIRS_W,
    }
}

// Flowing water. `WATER` (id 7) is a source, level 0; these seven are levels
// 1..7, one per block of distance travelled, exactly as Java stores them. They
// live above BLOCK_KINDS because they are never inventory items -- the 6-bit
// block field holds 0..63, so ids 54..60 were free.
const WATER_F1: u8 = 54;
const WATER_F7: u8 = WATER_F1 + 6;
/// Furthest a flow travels from its source before it runs out (Java: 7).
const WATER_MAX_RUN: u8 = 7;

/// Source or flow. Most of the engine wants "is this wet", not "which level".
#[inline]
fn is_water(b: u8) -> bool {
    b == WATER || (b >= WATER_F1 && b <= WATER_F7)
}

/// 0 for a source, 1..7 for flowing water, `WATER_MAX_RUN + 1` for anything
/// else so a non-water neighbour never reads as "closer to the source".
#[inline]
fn water_level(b: u8) -> u8 {
    if b == WATER {
        0
    } else if b >= WATER_F1 && b <= WATER_F7 {
        b - WATER_F1 + 1
    } else {
        WATER_MAX_RUN + 1
    }
}

#[inline]
fn water_of_level(level: u8) -> u8 {
    WATER_F1 + level - 1
}

// Recipe output sentinels: raise one tool's tier rather than granting an item.
const CRAFT_PICK: u8 = 255;
const CRAFT_AXE: u8 = 253;
const CRAFT_SHOVEL: u8 = 252;
const CRAFT_SWORD: u8 = 251;
const CRAFT_ARMOR: u8 = 254; // recipe output sentinel: upgrade the armour tier
/// Inventory slots. INV and CHEST_INV are indexed BY BLOCK ID, so this must
/// cover the whole id space, not just the ids that happen to be items today --
/// it was 54 while ids ran to 53, and every id added since (slab 61, obsidian
/// 71, the potions at 80+) indexed past the end. Rust bounds-checks, so that is
/// a runtime panic the moment you select one, not silent corruption; the
/// compiler only caught it when a CONSTANT index finally went out of range.
///
/// 128 = the full 7-bit block field. Costs 148 bytes on INV and 2.4KB across
/// the 16 chest inventories.
const BLOCK_KINDS: usize = 128;

// Worn-armour tier (0 none, 1 iron, 2 diamond) scales incoming combat damage.
const ARMOR_PCT: [i32; 3] = [100, 60, 35]; // % of damage taken at each tier

// Blocks the hotbar lets you select and place (building set; ores/fluids excluded).
// SEEDS is selectable but plants a crop instead of placing a block (see place path).
const PLACEABLE: [u8; 48] = [
    GRASS, DIRT, STONE, COBBLE, SLAB, STAIRS_N, BRICK, OBSIDIAN, CINDERSTONE, SINK_SAND, LUMISTONE,
    VOID_STONE, WOOD, PLANK, FENCE, LEAVES, SAND, SNOW, GLASS, WOOL, FLINT_STEEL, VOID_EYE,
    BOTTLE, POTION_SPEED,
    POTION_STRENGTH, POTION_REGEN, POTION_FIRE,
    LADDER, DOOR_C, CRAFT_TABLE, CHEST, FURNACE, ENCHANT, BED, WIRE, TORCH, PISTON, TNT, CACTUS, SEEDS, SAPLING,
    WHEAT_ITEM, BONEMEAL, BOW, FISHING_ROD, BUCKET, WATER_BUCKET, LAVA_BUCKET,
];

// Spawn column, in world block coords.
/// Spawn on GRASS instead of in the desert.
///
/// The hardcoded spawn sits at temperature 211 and desert starts at 191, so
/// every screenshot this project has ever produced was beige, and a visual
/// audit called the world "a beige staircase quarry". Measured over 184,041
/// samples of the real noise field the world is 65.2% plains, 19.0% snow and
/// 15.8% desert -- the fixed point simply landed in one of the desert patches.
/// world::pick_spawn hunts outward from SPAWN_BX/BZ for an open, flat plains
/// REGION, entirely from noise, before a single chunk is generated.
///
/// It is OFF, and that is a measured budget decision rather than a taste one:
///     desert                         world pass 592,246   29.7 fps
///     green, no vegetation          world pass 495,577   29.6 fps
///     green, trees only             world pass 770,215   23.5 fps
///     green, decorations only       world pass 676,862   24.6 fps
///     green, trees + decorations    world pass 959,953   18.8 fps
///
/// The green terrain itself is NOT expensive; it is slightly cheaper than the
/// desert on this route. An earlier four-build decomposition claimed a 205K
/// GRASS-over-DIRT terrace cost. That test was confounded: changing the surface
/// to DIRT made maybe_decoration return early, silently removing every plant.
/// Remapping only grass SIDE faces to dirt then changed essentially nothing
/// (959,953 -> 961,601), proving there was no terrace cost to merge away.
///
/// Trees add ~275K cycles in isolation and decorative plants ~181K. The costs
/// are not additive because trees replace some decorations and change what is
/// visible. Density can trade appearance for speed: tree gates 90/340 plus 5%
/// decorations reached 26.7 fps, but looked nearly empty. Keep the shipped
/// desert until that visual/performance choice is made deliberately.
///
/// Made deliberately 2026-08-01: green. Standing at the green spawn facing
/// the near tree measures 20 fps; open views keep the 30 lock. The cost is
/// being near trees, not the spawn -- any play session heads for the trees
/// anyway, so the desert only deferred the same bill and added a walk.
const SPAWN_GREEN: bool = true;
/// Where the spawn HUNT starts, not the spawn itself.
const SPAWN_BX: i32 = 8;
const SPAWN_BZ: i32 = 8;
// Respawn point in world block coords; defaults to spawn, moved by sleeping in a bed.
/// The resolved world spawn, filled in by world::pick_spawn at boot.
static mut WORLD_BX: i32 = SPAWN_BX;
static mut WORLD_BZ: i32 = SPAWN_BZ;
static mut RESPAWN_BX: i32 = SPAWN_BX;
static mut RESPAWN_BZ: i32 = SPAWN_BZ;

const OT_LEN: usize = 1024;
const MAX_QUADS: usize = 1536;
const RECIP_LEN: usize = (FAR_Z + 1) as usize; // 1/z table for the perspective divide
// Profiling toggles (ship = all false; const-folded, zero runtime cost). Flip one
// to true and read `cmd_log fills=N` from `make smoke` as a CPU dynamometer:
// the fill count is a continuous metric, where the vsync-quantised fps readout
// only moves in 60/N steps and hides real gains.
const PROFILE_SKIP_WORLD: bool = false; // skip world face render (isolate world cost)
const PERF_FREERUN: bool = false; // skip vsync so frames-per-cycle reads true CPU cost
const PROFILE_SKIP_SUBMIT: bool = false; // build OT but skip GPU draw (isolate fill cost)
const PROFILE_SKIP_TITLE: bool = false; // TEMP: bypass the title screen for headless in-game captures
const FORCE_TIME: i32 = -1;
const SHOW_FPS: bool = false;
const VISTA_VIEW: bool = false;
const VISTA_YAW: u16 = 0x0500;
// TEMP capture knob: when >= 0 the spawn search also requires this biome id
// (world::B_*) so headless captures can inspect grass/leaves/water/snow. -1 = off.
const CAPTURE_BIOME: i32 = -1;
// TEMP capture knob: force one of each mob kind in a row facing the camera.
const MOB_LINEUP: bool = false;
// TEMP capture knob: on an early frame, place a row of the once-corrupted
// placeable blocks (wool/ladder/cobble/enchant + bed/tnt) in front of spawn to
// verify the 6-bit face-word fix renders them as themselves.
const PLACE_TEST: bool = false;
// TEMP capture knob: grow a small field of young/ripe wheat + saplings ahead of
// spawn (and aim the camera at it) to verify the cross-sprite plant billboards.
const PLANT_TEST: bool = false;
// TEMP capture knob: (on, tab, filter). Stocks a few planks and sticks, then
// forces the crafting menu open on the given tab with the affordability filter
// as given, so headless captures can verify the tabbed menu. Use with
// PROFILE_SKIP_TITLE.
const CRAFT_TEST: (bool, usize, bool) = (false, 0, false);
// Fixed-pose render harness (ship = false). Bypasses the menu, pins the
// player at the spawn surface with an exact yaw/pitch, places a two-block
// tower with two ordinary edits, then freezes all input. The scene is STATIC
// after ~frame 50, so a capture at ANY later step count shows the same image
// -- which makes A/B comparisons between builds trustworthy (demo-script
// captures are not: input is per frame, sim per vblank, so build speed shifts
// what scene a given step count lands on). Built for the steep-pitch
// ground-coverage bug: POSE_PITCH -1024 reproduces it, -512 is the control.
const POSE_TEST: bool = false;
const POSE_PITCH: i16 = -1024;
const POSE_YAW: u16 = 0;
// Scripted-input playtest: DEMO_PLAY substitutes a frame-indexed input script
// for the pad -- walks, looks, mines, places, cycles the hotbar, opens the
// crafting menu, swings, and turns around like a player would. Checkpoint
// state goes to the emulator console and the frame number is stamped on the
// HUD so headless captures self-document. Ship = false.
const DEMO_PLAY: bool = false;

/// The demo input for a frame: (buttons, left stick, right stick), sticks as
/// centred i16 (+-~110 usable). Phases are generous so captures at imprecise
/// step counts still land inside the intended action.
fn demo_input(frame: u32) -> (ButtonState, (i16, i16), (i16, i16)) {
    let mut b: u16 = 0;
    let mut left = (0i16, 0i16);
    let mut right = (0i16, 0i16);
    if DEMO_MARCH {
        // Streaming stress: enter fly mode, climb clear of obstacles, then fly
        // a large square. A ground-only straight march used to stop at the first
        // cactus and falsely "pass" without crossing even one chunk boundary.
        match frame {
            0 => b = button::SELECT,
            1..=59 => b = button::CROSS,
            60..=799 => left = (0, -110),
            800..=1539 => left = (110, 0),
            1540..=2279 => left = (0, 110),
            2280..=2999 => left = (-110, 0),
            _ => {}
        }
        return (ButtonState::from_bits(b), left, right);
    }
    match frame {
        // A: walk forward across the terrain.
        0..=119 => left = (0, -100),
        // B: keep walking, ease the camera down toward the ground ahead.
        120..=179 => {
            left = (0, -100);
            if frame < 170 {
                right = (0, 60);
            }
        }
        // C: while the ground is still in reach, place dirt, cycle to the
        // test stone stack, place stone, then cycle back. Doing this before
        // mining prevents a long excavated ray from turning the placement
        // portion of the regression test into a no-target no-op.
        180 => b = button::L2,
        190 => b = button::R1,
        200 => b = button::L2,
        210 => b = button::L1,
        220 => b = button::L2,
        221..=239 => {},
        // D: stand and hold R2 -- hold-to-mine the picked block.
        240..=419 => b = button::R2,
        // E: repeat the placing sequence after mining.
        420 => b = button::L2,
        450 => b = button::R1,
        480 => b = button::L2,
        // F: cycle back and place dirt once more.
        510 => b = button::L1,
        540 => b = button::L2,
        // G: open the crafting menu and sit in it (captures the panel).
        570 => b = button::SQUARE,
        // H: close it again, then a few melee swings.
        630 => b = button::SQUARE,
        660 | 680 | 700 => b = button::R2,
        // I: turn ~180 degrees and look back over the walked path.
        730..=879 => right = (85, 0),
        // J: idle -- stable end state for late captures.
        _ => {}
    }
    (ButtonState::from_bits(b), left, right)
}

#[inline(never)]
fn demo_checkpoint(frame: u32, player: &Player) {
    telemetry::counter(70, unsafe { INV[DIRT as usize] } as u32);
    telemetry::counter(71, unsafe { INV[STONE as usize] } as u32);
    telemetry::counter(72, player.selected as u32);
    telemetry::console("DEMO checkpoint:");
    telemetry::console(&decimal3((frame / 10) as u16));
    telemetry::console(&decimal3((player.x >> 6) as u16));
    telemetry::console(&decimal3(player.y as u16 >> 6));
    telemetry::console(&decimal3((player.z >> 6) as u16));
    telemetry::console(&decimal3(unsafe { INV[DIRT as usize] }));
    telemetry::console(&decimal3(unsafe { INV[STONE as usize] }));
    telemetry::console(&decimal3(player.selected as u16));
    telemetry::console(&decimal3(player.health as u16));
}

#[inline(never)]
fn draw_demo_hud(font: &FontAtlas, frame: u32, player: &Player) {
    ui_text(font, 4, 228, "DF", (0x80, 0xE0, 0xF0));
    ui_text(font, 24, 228, &decimal3((frame / 10).min(999) as u16), (0x80, 0xE0, 0xF0));
    let bx = world_to_block_x(player.x) + 500;
    let by = world_to_block_y(player.y) + 500;
    let bz = world_to_block_z(player.z) + 500;
    ui_text(font, 64, 228, &decimal3(bx.clamp(0, 999) as u16), (0xE0, 0xC0, 0x70));
    ui_text(font, 96, 228, &decimal3(by.clamp(0, 999) as u16), (0xE0, 0xC0, 0x70));
    ui_text(font, 128, 228, &decimal3(bz.clamp(0, 999) as u16), (0xE0, 0xC0, 0x70));
    let (wl, wm, wd) = world::world_stats(world_to_block_x(player.x), world_to_block_z(player.z));
    ui_text(font, 168, 228, &decimal3(wl), (0x90, 0xE0, 0x90));
    ui_text(font, 200, 228, &decimal3(wm), (0x90, 0xE0, 0x90));
    ui_text(font, 232, 228, &decimal3(wd), (0x90, 0xE0, 0x90));
}

/// Demo variant: an obstacle-free four-direction flight (streaming stress).
/// Use with DEMO_PLAY; reproduces movement across chunk borders for a long time.
const DEMO_MARCH: bool = false;

// Block-face direction vectors, shared by mesh build and render.
const DIRS: [(i32, i32, i32); 6] = [
    (1, 0, 0),
    (-1, 0, 0),
    (0, 1, 0),
    (0, -1, 0),
    (0, 0, 1),
    (0, 0, -1),
];
const PICK_RANGE: i32 = BLOCK * 9 / 2; // 4.5-block block reach (Java survival)
const MOVE_DEADZONE: i16 = 18;
const LOOK_DEADZONE: i16 = 12;
// Live settings, tweakable from either SETTINGS card and persisted in their
// own normal memory-card file (separate from the world save).
static mut SET_MOVE_DZ: i16 = MOVE_DEADZONE;
static mut SET_LOOK_DZ: i16 = LOOK_DEADZONE;
static mut SET_LOOK_PCT: i32 = 100; // look-speed scale, percent
static mut SET_INVERT_Y: bool = false;
const SETTINGS_FILE: &str = "BESLES-00000VOXSET1";
const SETTINGS_TITLE: &str = "VoXide Settings";
const ACT_FORWARD: usize = 0;
const ACT_BACK: usize = 1;
const ACT_LEFT: usize = 2;
const ACT_RIGHT: usize = 3;
const ACT_SNEAK: usize = 4;
const VOXIDE_ACTIONS: ActionMap<5> = ActionMap::new([
    ActionBinding::new(button::UP, 0),
    ActionBinding::new(button::DOWN, 0),
    ActionBinding::new(button::LEFT, 0),
    ActionBinding::new(button::RIGHT, 0),
    ActionBinding::new(button::CIRCLE, 0),
]);
static mut SETTINGS_PROFILE: Profile<5, 0> = Profile::new(VOXIDE_ACTIONS);
static mut SETTINGS_DIRTY: bool = false;
// Frames a first jump-tap stays "armed"; a second CROSS within it toggles fly.
const DOUBLE_TAP_FRAMES: u8 = 12;

// Player physics, in world units (BLOCK = 64). Dimensions match Java Edition
// (1 block = 64 units): eye 1.62, height 1.8, width 0.6. Gravity/jump/speed are
// felt, but tuned to Java's real numbers where those are measurable.
const EYE_HEIGHT: i32 = 104; // 1.62 blocks (Java standing eye)
const PLAYER_HALF_W: i32 = 19; // 0.6-block-wide collision box (Java 0.6)
const PLAYER_HEIGHT: i32 = 115; // 1.8 blocks tall (Java)
const GRAVITY: i32 = 4; // vy lost per frame
const TERMINAL_VY: i32 = -56; // clamp fall speed below ~1 block/frame (no tunnel)
/// Fastest upward stroke while swimming; a jump out of water starts
/// above this and stays ballistic until gravity brings it back down.
const SWIM_UP: i32 = 14;
const JUMP_VY: i32 = 28; // impulse tuned so the peak is ~1.3 blocks (Java jumps 1.25 -> clears one block, not two)
const WALK_SPEED: i32 = 9; // ~4.2 blocks/s (Java walk 4.317)
const FLY_SPEED: i32 = 8; // vertical units/frame in creative fly

const FONT_TPAGE: Tpage = Tpage::new(320, 0, TexDepth::Bit4);
const FONT_CLUT: Clut = Clut::new(320, 256);

// Day/night: a full cycle is DAY_LEN frames (~600s / 10 min at 30fps -- about 2x
// the pace of Java's 20-min day, no longer the old 20x-compressed 60s). LIGHT
// (0..128) scales the global sky light; NIGHT_LIGHT keeps night dim but not pitch
// black (no block-light yet). Sky colour lerps between night and day by brightness.
const DAY_LEN: u32 = 18000;
const NIGHT_LIGHT: i32 = 38;
// Java's plains biome sky_color is 0x78A7FF. The old (82,130,190) was a dull,
// slightly green-leaning blue that read as late afternoon at altitude.
const SKY_DAY: (i32, i32, i32) = (120, 167, 255);
/// The ZENITH is tracked separately from the horizon rather than derived from
/// it. Deriving it meant a warm sunset horizon dragged the whole dome brown.
const SKY_DAY_ZENITH: (i32, i32, i32) = (58, 110, 214);
const SKY_NIGHT_ZENITH: (i32, i32, i32) = (4, 6, 28);
const SKY_NIGHT: (i32, i32, i32) = (12, 18, 58);
const OVERCAST: (i32, i32, i32) = (118, 122, 126); // flat neutral grey a rainy sky washes toward

const EMPTY_QUAD: QuadTexturedMaterial = QuadTexturedMaterial {
    tag: 0,
    tex_window: 0,
    color_cmd: 0,
    v0: 0,
    uv0_clut: 0,
    v1: 0,
    uv1_tpage: 0,
    v2: 0,
    uv2: 0,
    v3: 0,
    uv3: 0,
};

const EMPTY_FLAT: QuadFlat = QuadFlat {
    tag: 0,
    color_cmd: 0,
    v0: 0,
    v1: 0,
    v2: 0,
    v3: 0,
};
const MAX_MOB_QUADS: usize = mob::CAP * 30 + mob::arrow_cap() * 6; // multi-box mob models

const RENDER_ARENAS: usize = 2;
/// Arena the CPU is currently building. The GPU may still be rasterising the
/// other arena, so every packet address reachable from an OT is double-buffered.
static mut RENDER_ARENA: usize = 0;
static mut OT: [OrderingTable<OT_LEN>; RENDER_ARENAS] =
    [OrderingTable::new(), OrderingTable::new()];
/// The sky keeps a chain of its own so it is queued ahead of the world chain.
/// Both are submitted only after the previous frame has finished: while the GPU
/// rasterises them, the CPU builds the next frame in the other arena.
static mut SKY_OT: [OrderingTable<2>; RENDER_ARENAS] =
    [OrderingTable::new(), OrderingTable::new()];
static mut QUADS: [[QuadTexturedMaterial; MAX_QUADS]; RENDER_ARENAS] =
    [[EMPTY_QUAD; MAX_QUADS], [EMPTY_QUAD; MAX_QUADS]];
static mut MOB_QUADS: [[QuadFlat; MAX_MOB_QUADS]; RENDER_ARENAS] =
    [[EMPTY_FLAT; MAX_MOB_QUADS], [EMPTY_FLAT; MAX_MOB_QUADS]];

const EMPTY_GOURAUD_TRI: TriTexturedGouraud = TriTexturedGouraud {
    tag: 0,
    tex_window: 0,
    color0_cmd: 0,
    v0: 0,
    uv0_clut: 0,
    color1: 0,
    v1: 0,
    uv1_tpage: 0,
    color2: 0,
    v2: 0,
    uv2: 0,
};
/// Close greedy faces are expanded into block-sized, clipped triangle cells.
/// Four maximum-size visible faces can all take the slow path without dropping
/// geometry; in normal play the high-water mark is far below this.
const MAX_NEAR_TRIS: usize = 1536;
static mut NEAR_TRIS: [[TriTexturedGouraud; MAX_NEAR_TRIS]; RENDER_ARENAS] =
    [[EMPTY_GOURAUD_TRI; MAX_NEAR_TRIS], [EMPTY_GOURAUD_TRI; MAX_NEAR_TRIS]];
static mut NEAR_TRI_N: usize = 0;


/// Master switch for ambient occlusion. ON.
///
/// It shipped OFF, because measured vsync-locked it looked like this:
///   AO_ON=false                 1,209,154 cyc  2.14 vbl  28.0 fps
///   AO_ON=true                  1,712,599 cyc  3.03 vbl  19.8 fps
/// That was never AO's own weight. The loop body grew 1,134,651 -> 1,168,227,
/// about +34K or +3%; the rest was the vsync quantum, because the frame was
/// already at ~100% of its 2-vblank budget, so +3% tipped it to three vblanks
/// and the sim_n catch-up feedback locked it there.
///
/// Nor was +34K tunable. With AO_BAND=0 the compiler deletes the branch and the
/// profile is byte-identical to AO_ON=false, which pins the cost on the second
/// packet builder EXISTING inside emit_face -- inline(always) into a loop at the
/// edge of the R3000's 4KB instruction cache -- rather than on how often it runs.
///
/// What unblocked it was finding the headroom elsewhere: the far cull was
/// testing the bounding sphere's NEAREST point against FAR_Z while emit_face
/// discards on the average corner depth, so faces in that band paid a full GTE
/// projection to be thrown away. Culling on the centre reclaimed ~92K, nearly
/// three times what AO costs. Now:
///   AO_ON=false, after the cull fix   loop body 1,042,905   2.02 vbl  29.7 fps
///   AO_ON=true,  after the cull fix   loop body 1,060,124   2.02 vbl  29.7 fps
///
/// Honest limitation: greedy_plane computes AO at the four corners of the
/// MERGED rectangle and the merge is not AO-aware, so a 12x8 plate with one
/// occluded corner smears its gradient over twelve blocks where Minecraft
/// confines the contact shadow to one. Folding AO into the merge key would fix
/// it and multiply face count in a loop that is already 40% of the frame.
const AO_ON: bool = true;
/// Fog band past which faces stop paying for AO (bands are depth >> FOG_SHIFT,
/// so 4 is ~512 units, about a third of FAR_Z).
const AO_BAND: usize = 4;
/// Separate packet pool for the AO faces: a Gouraud textured quad is 13 data
/// words against the flat one's 9, so the two cannot share an array. Faces past
/// this cap draw flat rather than not at all.
const MAX_AO_QUADS: usize = 1024;
static mut AO_QUADS: [[QuadTexturedGouraud; MAX_AO_QUADS]; RENDER_ARENAS] =
    [
        [QuadTexturedGouraud::EMPTY; MAX_AO_QUADS],
        [QuadTexturedGouraud::EMPTY; MAX_AO_QUADS],
    ];
static mut AO_N: usize = 0;
/// Scale a packed 24-bit GP0 colour by one corner's 2-bit AO level.
///
/// Each channel sits on its own byte, so a masked shift of the WHOLE word is a
/// per-channel shift -- a couple of ALU ops instead of three R3000 multiplies,
/// on a CPU with a slow `mult` and no divider. The factors are 100 / 81.25 /
/// 62.5 / 50 percent, the nearest cheap-shift fit to Minecraft's ramp.
///
/// Level 2 (exactly one occluder) is much the most common, so its factor is
/// what sets the scene's overall brightness. At 75% the terrain averaged 14%
/// darker than with AO off, which reads as gloom rather than as shading; 81.25%
/// keeps the contact shadows and costs two extra shifts on that one level.
#[inline(always)]
fn ao_shade(rgb: u32, level: u32) -> u32 {
    match level & 3 {
        3 => rgb,
        2 => rgb - ((rgb >> 3) & 0x001F_1F1F) - ((rgb >> 4) & 0x000F_0F0F),
        1 => ((rgb >> 1) & 0x007F_7F7F) + ((rgb >> 3) & 0x001F_1F1F),
        _ => (rgb >> 1) & 0x007F_7F7F,
    }
}

// Prepacked per-tile material words (borrowed idea: psx-gpu's
// TexturedPacketMaterial, built for hl-psx's hot paths). Window/CLUT/tpage
// words are LIGHT-independent -> built once at boot ([0]=opaque, [1]=blended);
// the tint-bearing colour command varies only by dir+LIGHT -> 12 words
// refreshed per frame. emit_face then just stores raw words.
static mut MAT_WIN: [[u32; tex::TILE_COUNT]; 2] = [[0; tex::TILE_COUNT]; 2];
static mut MAT_CLUT_HI: [[u32; tex::TILE_COUNT]; 2] = [[0; tex::TILE_COUNT]; 2];
static mut MAT_TPAGE_HI: [[u32; tex::TILE_COUNT]; 2] = [[0; tex::TILE_COUNT]; 2];
// Face colour command per (see-through, direction, DEPTH BAND). The depth axis
// is distance fog: without it the world just stops at FAR_Z against open sky,
// which the imposter cards used to paper over badly and nothing has since.
// Prebuilt per frame, so a face costs one extra index and no arithmetic.
// 16 bands of 128 depth units: FAR_Z is 1792, so bands 0..14 are in use and the
// index is a SHIFT. `depth * N / FAR_Z` would be a real division, and the R3000
// has no divider -- that was ~35 cycles on every face.
const FOG_BANDS: usize = 16;
const FOG_SHIFT: u32 = 7;
#[inline(never)]
fn advance_streaming() {
    // Slices scale with the streaming BACKLOG and nothing else. Render-headroom
    // thresholds have been wrong here twice: the original tiers (240 quads /
    // 1000 faces / dt==1) are exceeded on every normal terrain frame, which
    // starved streaming to literal zero, and a softer 160/600 gate re-created
    // the same failure as a feedback loop -- meshing more terrain raises the
    // quad count, which trips the gate, which throttles meshing at exactly the
    // moment the frontier needs it. Backlog urgency is the honest signal.
    //
    // Demand arithmetic: a chunk is ~38 gen ticks (~40K cycles each) + ~60 mesh
    // ticks (~144K at MESH_CELL_BUDGET 2048), a boundary cross needs five fresh
    // chunks, and a sprint crosses every ~57 frames -- low-4s of sustained
    // slices. The split: NEAR work (within one chunk, i.e. inside the ~21-block
    // fog radius, or a player edit) would be visible pop-in, so it gets 6
    // slices, which outruns any movement speed. FAR-only work (the ring's outer
    // band, beyond the fog) fills at 3 -- slightly under sprint demand on
    // purpose, because a flat-out sprint promotes the shortfall to near work
    // within half a crossing and the 6-slice burst absorbs it; anything gentler
    // never sees the edge at all. Idle: one cheap slice. This keeps the
    // catch-up fps cost proportional to how visible the missing terrain is.
    let (near, far) = world::stream_backlog();
    let stream_slices = if near > 0 || world::edit_backlog() {
        6
    } else if far > 0 {
        3
    } else {
        1
    };
    let mut sk = 0;
    while sk < stream_slices {
        telemetry::stage_begin(ST_GEN);
        world::gen_tick();
        telemetry::stage_end(ST_GEN);
        world::stream_tick_claim(true);
        sk += 1;
    }
}

/// Precomputed GP0 colour words, now indexed by SKYLIGHT bucket as well as
/// blend mode, face direction and fog band. Four buckets multiplies the table
/// by four (768 words, 3KB) and keeps the per-face cost at one array read --
/// which is the only reason block light is affordable here at all.
static mut MAT_CCMD: [[[[u32; FOG_BANDS]; 6]; 2]; SKY_LEVELS] =
    [[[[0; FOG_BANDS]; 6]; 2]; SKY_LEVELS];
const SKY_LEVELS: usize = 4;
/// Fraction of full brightness at each bucket: fully shadowed cells keep a
/// floor, because pitch black on a PS1 palette just reads as a hole.
/// Skylight multiplier per bucket. Four steps now rather than two: the old pair
/// meant a cell was either fully lit or 44% lit with a hard boundary between
/// them, which a visual audit repeatedly flagged as the reason caves and house
/// interiors read as two flat tones instead of falling off.
const SKY_SCALE: [u32; SKY_LEVELS] = [56, 80, 104, 128];
// Horizon colour the far bands fade into; set from the sky each frame.
static mut FOG_RGB: (u8, u8, u8) = (82, 130, 190);
/// How enclosed the camera is: 0 under open sky, 255 deep underground. The fog
/// ramp and the backdrop both fade to cave black by this.
///
/// The fog was calibrated for daylight and applied at every depth, so a cave
/// dissolved into the same warm sand-grey the horizon uses -- terrain a few
/// blocks away sat against what looked like a lit void, with the far wall of
/// the cave simply absent. That is the screenshot behind the itch report of
/// caves "generating very strangely", and behind the ask for fog: there was
/// fog, it was just the wrong colour to read as depth underground.
static mut CAVE: u8 = 0;
/// What fully-fogged terrain fades to underground. Not pure black: at 15-bit
/// colour a true zero reads as a hole punched in the frame.
const CAVE_FOG: (u8, u8, u8) = (10, 11, 16);
/// Past this, the sky dome is replaced by a flat cave wall. The dome's zenith
/// is computed from the time of day rather than from the horizon colour, so
/// darkening the horizon alone would leave blue sky showing through a cave
/// roof.
const CAVE_SOLID: u8 = 128;

/// Blocks of cover before the fog starts going dark, and how many more until
/// it is fully cave. A house roof is one or two blocks of cover and has to
/// stay daylight; a mine ten blocks down is a cave.
fn cave_amount(py: i32, bx: i32, bz: i32) -> u8 {
    if world::dimension() != world::DIM_OVERWORLD {
        return 0; // both off-world dims already draw their own flat fog wall
    }
    let depth = world::surface_y(bx, bz) - world_to_block_y(py);
    (((depth - 3).max(0) * 255) / 8).min(255) as u8
}

/// Build the boot-time material word tables (after tex::upload).
fn init_mat_tables() {
    let bt = unsafe { &BLOCK_TEX };
    let mut t = 0;
    while t < tex::TILE_COUNT {
        let (tux, tuy) = tex::tile_uv(t as u8);
        let win = TextureWindow::power_of_two_tile(tux, tuy, 16, 16);
        let o = TextureMaterial::opaque(bt.clut[t], bt.tpage, (128, 128, 128))
            .with_texture_window(win)
            .textured_packet_material();
        let a = TextureMaterial::blended(bt.clut_alpha[t], bt.tpage, (128, 128, 128), BlendMode::Average)
            .with_texture_window(win)
            .textured_packet_material();
        unsafe {
            MAT_WIN[0][t] = o.tex_window_word;
            MAT_CLUT_HI[0][t] = o.clut_high_word;
            MAT_TPAGE_HI[0][t] = o.tpage_high_word;
            MAT_WIN[1][t] = a.tex_window_word;
            MAT_CLUT_HI[1][t] = a.clut_high_word;
            MAT_TPAGE_HI[1][t] = a.tpage_high_word;
        }
        t += 1;
    }
}

/// Refresh the 12 tint-bearing colour-command words for the current LIGHT.
/// NB: TexturedPacketMaterial::from_texture bakes the TRIANGLE header
/// (quad=false); these packets are QUADS -- build the header directly.
/// The tone fully fogged GROUND renders at: warm sand-grey, skylight-scaled.
/// Shared by the fog ramp's far bands and the sky's below-horizon fill, which
/// is what makes the end of the far ring meet the sky with no visible seam.
fn ground_haze(light: u8) -> (u8, u8, u8) {
    (
        (light as u32 * 205 / 128).min(255) as u8,
        (light as u32 * 192 / 128).min(255) as u8,
        (light as u32 * 152 / 128).min(255) as u8,
    )
}

fn refresh_mat_ccmd() {
    let fog = unsafe { FOG_RGB }; // already faded toward CAVE_FOG by the caller
    let cave = unsafe { CAVE } as i32;
    let h = ground_haze(unsafe { LIGHT });
    // The far bands converge on ground haze, which is a daylight colour. Fade
    // it with the near bands or the end of a cave sightline stays sand-grey.
    let haze = (
        lerp_u8(h.0 as i32, CAVE_FOG.0 as i32, cave, 255),
        lerp_u8(h.1 as i32, CAVE_FOG.1 as i32, cave, 255),
        lerp_u8(h.2 as i32, CAVE_FOG.2 as i32, cave, 255),
    );
    let mut sky = 0usize;
    while sky < SKY_LEVELS {
    let mut dir = 0;
    while dir < 6 {
        let lit = face_tint(dir);
        let sc = SKY_SCALE[sky];
        let tint = (
            (lit.0 as u32 * sc / 128) as u8,
            (lit.1 as u32 * sc / 128) as u8,
            (lit.2 as u32 * sc / 128) as u8,
        );
        let mut b = 0;
        while b < FOG_BANDS {
            // Clear until ~12 blocks out, then ramp to the far plane, so close
            // terrain keeps full contrast and only the band where the hard edge
            // against the sky used to be gets blended.
            let t = if b <= 2 {
                0
            } else if b <= 5 {
                // Bands 0..7 at FAR_Z 1024: clear to ~5 blocks, ramp through
                // mid distance, full ground-haze in the last band so the
                // horizon dissolves into the below-horizon sky fill.
                ((b - 2) as i32) * 255 / 6
            } else if b == 6 {
                200
            } else {
                255
            };
            // ...and the far bands converge on GROUND HAZE rather than sky
            // blue: those bands exist to give crest sightlines something to
            // hit, and they only work if the ring's silhouettes are the same
            // colour as the below-horizon sky behind them (ground_haze on
            // both sides). Sky-blue full fog would repaint the old void
            // sliver ON the terrain.
            let target = if b <= 4 {
                fog
            } else if b <= 6 {
                (
                    ((fog.0 as u16 + haze.0 as u16) / 2) as u8,
                    ((fog.1 as u16 + haze.1 as u16) / 2) as u8,
                    ((fog.2 as u16 + haze.2 as u16) / 2) as u8,
                )
            } else {
                haze
            };
            // The tint is a MODULATION word: the GPU computes texel * cmd / 128.
            // Lerping it toward the raw fog colour therefore does not converge
            // distant terrain ON the fog colour, it multiplies the fog by the
            // texel -- so a bright sand texel (~230) against a fog tint of
            // (82,130,190) landed near (147,234,255), lighter and cooler than
            // both the near ground AND the sky. The first sky audit measured
            // exactly that and called the far dunes "a wall of ice". Pre-divide
            // by the mean terrain texel so the product lands on the fog colour.
            const MEAN_TEXEL: u32 = 176;
            let fm = |f: u8| ((f as u32 * 128 / MEAN_TEXEL).min(255)) as u8;
            let (fr, fg, fb) = (fm(target.0), fm(target.1), fm(target.2));
            let mix = |c: u8, f: u8| lerp_u8(c as i32, f as i32, t, 255);
            let ft = (mix(tint.0, fr), mix(tint.1, fg), mix(tint.2, fb));
            let o = TextureMaterial::opaque(0, 0, ft).flat_textured_polygon_header(true);
            let a = TextureMaterial::blended(0, 0, ft, BlendMode::Average)
                .flat_textured_polygon_header(true);
            unsafe {
                MAT_CCMD[sky][0][dir][b] = o;
                MAT_CCMD[sky][1][dir][b] = a;
            }
            b += 1;
        }
        dir += 1;
    }
    sky += 1;
    }
}

/// Depth -> fog band, by shift (see FOG_SHIFT).
#[inline]
fn fog_band(depth: i32) -> usize {
    let b = (depth >> FOG_SHIFT).max(0) as usize;
    if b >= FOG_BANDS {
        FOG_BANDS - 1
    } else {
        b
    }
}
static mut BLOCK_TEX: BlockTex = BlockTex::EMPTY;
static mut LIGHT: u8 = 128;
/// 0 at noon .. 90 at midnight, weather-independent. Drives face_tint's cool
/// night cast, which LIGHT cannot because it carries the rain dimming.
static mut NIGHT_BIAS: u8 = 0;
/// 0..255 sunset strength, so the TERRAIN warms with the sky rather than
/// drifting cold while the horizon burns orange.
static mut SUN_WARMTH: u8 = 0; // global sky light, set each frame from time-of-day
static mut INV: [u16; BLOCK_KINDS] = [0; BLOCK_KINDS]; // block counts by block id
// Vanilla hotbar: 9 REAL slots holding item ids (AIR = empty), not a scrolling
// window over the catalogue. A kind you did not own claims the first free slot
// when acquired (Bedrock's rule); an emptied stack vacates its slot and the
// gap stays, exactly like the original.
static mut HOTBAR: [u8; HOTBAR_VIS] = [AIR; HOTBAR_VIS];
static mut HOTBAR_SEL: usize = 0;

// Edited-block deltas vs the seeded terrain. This is not just the memory-card
// payload any more: world::publish_chunk replays it into every chunk it
// generates, which is what makes a build survive leaving the loaded ring.
//
// 1024, up from 240. Chunks outside the 5x5 ring are thrown away rather than
// stored, so this log is the ONLY record a build has, and 240 blocks is a hut.
// The cost is 7 bytes each in RAM and 8 on the card, so 1024 is ~7 KiB and a
// two-block save file.
// ponytail: a flat log with a hard cap -- past 1024 edits new ones are simply
// dropped and will not come back. Per-chunk delta stores are the real answer
// and a much bigger change; do that when players hit this rather than before.
pub const MAX_EDITS: usize = 1024;
pub static mut EDIT_X: [i16; MAX_EDITS] = [0; MAX_EDITS];
pub static mut EDIT_Y: [i16; MAX_EDITS] = [0; MAX_EDITS];
pub static mut EDIT_Z: [i16; MAX_EDITS] = [0; MAX_EDITS];
pub static mut EDIT_B: [u8; MAX_EDITS] = [0; MAX_EDITS];
/// Which dimension the edit belongs to. All three share one coordinate space
/// and one chunk ring, so without this the replay would stamp your overworld
/// house into the Inferno.
pub static mut EDIT_D: [u8; MAX_EDITS] = [0; MAX_EDITS];
pub static mut EDIT_N: usize = 0;

/// Record a player block edit (last-write-wins per position; capped region).
fn record_edit(wx: i32, wy: i32, wz: i32, b: u8) {
    let (x, y, z) = (wx as i16, wy as i16, wz as i16);
    let d = world::dimension();
    unsafe {
        let mut i = 0;
        while i < EDIT_N {
            if EDIT_X[i] == x && EDIT_Y[i] == y && EDIT_Z[i] == z && EDIT_D[i] == d {
                EDIT_B[i] = b;
                return;
            }
            i += 1;
        }
        if EDIT_N < MAX_EDITS {
            EDIT_X[EDIT_N] = x;
            EDIT_Y[EDIT_N] = y;
            EDIT_Z[EDIT_N] = z;
            EDIT_B[EDIT_N] = b;
            EDIT_D[EDIT_N] = d;
            EDIT_N += 1;
        }
    }
}
// RECIP[z] = (PROJ_H << 16) / z, so projection multiplies by 1/z instead of
// dividing (R3000 div stalls ~36 cycles; projection is the render hot loop).
static mut RECIP: [i32; RECIP_LEN] = [0; RECIP_LEN];

// Per-chest storage, keyed by the chest block's world position.
const MAX_CHESTS: usize = 16;
static mut CHEST_X: [i32; MAX_CHESTS] = [0; MAX_CHESTS];
static mut CHEST_Y: [i32; MAX_CHESTS] = [0; MAX_CHESTS];
static mut CHEST_Z: [i32; MAX_CHESTS] = [0; MAX_CHESTS];
static mut CHEST_INV: [[u16; BLOCK_KINDS]; MAX_CHESTS] = [[0; BLOCK_KINDS]; MAX_CHESTS];
static mut CHEST_USED: [bool; MAX_CHESTS] = [false; MAX_CHESTS];

// Per-furnace state, keyed by the furnace block's world position.
const MAX_FURNACES: usize = 8;
const SMELT_TIME: u16 = 150; // ~5s to smelt one item (ponytail: half Java's 10s, a nod
                             // to our compressed clock; FURN_FUEL counts item-smelts)
const COAL_SMELTS: u16 = 8;  // one coal smelts 8 items (Java)
static mut FURN_X: [i32; MAX_FURNACES] = [0; MAX_FURNACES];
static mut FURN_Y: [i32; MAX_FURNACES] = [0; MAX_FURNACES];
static mut FURN_Z: [i32; MAX_FURNACES] = [0; MAX_FURNACES];
static mut FURN_USED: [bool; MAX_FURNACES] = [false; MAX_FURNACES];
static mut FURN_IN: [u8; MAX_FURNACES] = [AIR; MAX_FURNACES]; // input ore (AIR = empty)
static mut FURN_IN_N: [u16; MAX_FURNACES] = [0; MAX_FURNACES];
static mut FURN_FUEL: [u16; MAX_FURNACES] = [0; MAX_FURNACES]; // coal units
static mut FURN_OUT: [u8; MAX_FURNACES] = [AIR; MAX_FURNACES];
static mut FURN_OUT_N: [u16; MAX_FURNACES] = [0; MAX_FURNACES];
static mut FURN_PROG: [u16; MAX_FURNACES] = [0; MAX_FURNACES];

// Items the furnace can deposit into (UI list); coal/wood/planks are fuel,
// the rest are inputs.
const FURN_ITEMS: [u8; 8] = [IRON_ORE, SAND, COBBLE, RAW_MEAT, CLAY, COAL_ORE, WOOD, PLANK];

/// Fuel value in item-smelts (0 = not a fuel). Coal 8 (Java), wood/planks 2.
fn fuel_smelts(item: u8) -> u16 {
    match item {
        COAL_ORE => COAL_SMELTS,
        WOOD | PLANK => 2,
        _ => 0,
    }
}

/// What a furnace input smelts into (AIR = not smeltable).
fn smelt_result(item: u8) -> u8 {
    match item {
        IRON_ORE => IRON_INGOT,
        SAND => GLASS,
        COBBLE => STONE, // smelt cobblestone back into smooth stone (Java)
        RAW_MEAT => COOKED_MEAT,
        CLAY => BRICK,
        _ => AIR,
    }
}

#[derive(Copy, Clone)]
struct Player {
    x: i32, // feet centre (world units)
    y: i32, // feet bottom (world units)
    z: i32,
    vy: i32, // vertical velocity
    yaw: u16,
    pitch: i16,
    on_ground: bool,
    fly: bool,
    health: i32,       // 0..MAX_HEALTH (2 hp per heart)
    fall_peak: i32,    // highest y since last on ground, for fall damage
    air: i32,          // breath remaining underwater
    burn: i32,         // burning timer (set by lava/fire), damages over time
    hurt_cd: i32,      // i-frames between environmental damage ticks
    regen_delay: i32,  // frames to wait after damage before regen resumes
    regen_tick: i32,   // frames accumulated toward the next regen/starve point
    food: i32,         // hunger, 0..MAX_FOOD
    food_items: i32,   // raw food carried (mob drops); auto-eaten when hungry
    exhaustion: i32,   // counts toward the next hunger point lost
    // A tier per tool TYPE (0 none, 1 wood .. 4 diamond). They are player
    // stats rather than inventory items: there is no durability, the best of
    // each auto-equips, and the right one is chosen for whatever you swing at.
    pick: u8,
    axe: u8,
    shovel: u8,
    sword: u8,
    armor: u8,         // 0 none, 1 iron, 2 diamond (scales combat damage taken)
    selected: u8,
    xp: i32,           // experience from kills + ore; XP_PER_LEVEL each level
    efficiency: u8,    // mining-speed enchant level (0..3), bought with XP at a table
    sharpness: u8,     // melee-damage enchant level (0..3)
    protection: u8,    // damage-reduction enchant level (0..3)
    sprinting: bool,   // latched by L3 or double-tap-forward: 1.3x speed, 3x exhaustion
    sneaking: bool,    // CIRCLE held: slow, and will not walk off a ledge
    eff_speed: u16,    // potion effect timers, in frames
    eff_strength: u16,
    eff_regen: u16,
    eff_fire: u16,
    /// Accumulated horizontal distance, in world units. Drives the view bob, so
    /// the bob stops dead when the player does -- a timer-driven bob sways while
    /// you stand still, which reads as an idle screensaver rather than walking.
    bob: u32,
    /// Frames of hurt-tilt left, and which way to lean. Java rotates the view
    /// toward the damage source and decays it over ~10 ticks.
    hurt_tilt: u8,
    sprint_tap: u8,    // frames left in the double-tap-forward window (sprint start)
    sprint_latch: bool, // sprint engaged (L3 or double-tap); drops when forward stops
    was_fwd: bool,     // forward-input edge detector for the sprint double-tap
}

const XP_PER_LEVEL: i32 = 20;
const ENCHANT_COST: i32 = 3 * XP_PER_LEVEL; // 3 levels per efficiency upgrade
const MAX_EFFICIENCY: u8 = 3;

/// Reduce combat damage by the worn armour tier and the protection enchant
/// (never below 1 if any was dealt). Java caps total reduction at 80%; the
/// tier table already stops at 35%, and protection takes 12% a level off what
/// is left, so three levels land near that cap rather than at immunity.
fn armored(raw: i32, armor: u8, protection: u8) -> i32 {
    if raw <= 0 {
        return 0;
    }
    let after_armor = raw * ARMOR_PCT[(armor as usize).min(ARMOR_PCT.len() - 1)] / 100;
    let p = (protection.min(MAX_EFFICIENCY)) as i32;
    (after_armor * (100 - p * 12) / 100).max(1)
}

const MAX_HEALTH: i32 = 20;
const SAFE_FALL_BLOCKS: i32 = 3; // first 3 blocks of a fall do no damage (Java)
const MAX_AIR: i32 = 450;        // 15s underwater before drowning (Java: 300 ticks)
const REGEN_DELAY: i32 = 90;     // ponytail: ~3s post-hit regen pause; Java has none,
                                 // but our slow regen makes it near-moot. Kept for feel.
const REGEN_PERIOD: i32 = 120;   // slow regen: +1 hp / 4s (Java food>=18 tier)
const REGEN_FAST: i32 = 15;      // fast regen: +1 hp / 0.5s when well fed (food==20)
const FIRE_DURATION: i32 = 240;  // 8s of burning after lava contact (Cuberite BURN_TICKS)
const MAX_FOOD: i32 = 20;
const FOOD_DRAIN: i32 = 600;     // ~20s per hunger point lost (time-based; ponytail:
                                 // Java uses per-action exhaustion, but heal-burns-food
                                 // below carries the core "activity drains food" loop)
const STARVE_PERIOD: i32 = 120;  // lose 1 hp per 4s while starving (Java rate)
const REGEN_FOOD_MIN: i32 = 18;  // need food >= 18 to regen (Java threshold)

#[derive(Copy, Clone)]
struct Camera {
    x: i32,
    y: i32,
    z: i32,
    sy: i32,
    cy: i32,
    sp: i32,
    cp: i32,
    /// Camera roll in Q12 angle units: view bob sway plus the hurt tilt. Java
    /// rolls +/-3 degrees on the bob and up to 14 degrees when you take a hit.
    roll: i32,
}

#[derive(Copy, Clone)]
struct Proj {
    x: i16,
    y: i16,
    z: i32,
}

#[derive(Copy, Clone)]
struct Pick {
    hit: bool,
    bx: i32,
    by: i32,
    bz: i32,
    px: i32,
    py: i32,
    pz: i32,
}

const NO_PICK: Pick = Pick {
    hit: false,
    bx: 0,
    by: 0,
    bz: 0,
    px: 0,
    py: 0,
    pz: 0,
};

/// A crafting recipe (recipe-book style; a gamepad-friendly stand-in for the
/// 2x2/3x3 grid -- same ingredient logic, far better UX on a D-pad).
struct Recipe {
    in_item: [u8; 2],
    in_qty: [u16; 2],
    n_in: u8,
    out: u8,      // item id, or a CRAFT_* tool/armour sentinel
    out_qty: u16, // count, or the tool tier when out is a CRAFT_* sentinel
    label: &'static str,
}

const RECIPES: [Recipe; 50] = [
    Recipe { in_item: [PLANK, 0], in_qty: [4, 0], n_in: 1, out: CRAFT_TABLE, out_qty: 1, label: "CRAFT TABLE" },
    Recipe { in_item: [PLANK, 0], in_qty: [6, 0], n_in: 1, out: DOOR_C, out_qty: 3, label: "DOOR" },
    Recipe { in_item: [WOOD, 0], in_qty: [1, 0], n_in: 1, out: PLANK, out_qty: 4, label: "PLANKS" },
    Recipe { in_item: [PLANK, 0], in_qty: [2, 0], n_in: 1, out: STICK, out_qty: 4, label: "STICKS" },
    Recipe { in_item: [PLANK, 0], in_qty: [8, 0], n_in: 1, out: CHEST, out_qty: 1, label: "CHEST" },
    // Java: 3 cobble -> 6 slabs, 4 planks + 2 sticks -> 3 fences.
    Recipe { in_item: [COBBLE, 0], in_qty: [3, 0], n_in: 1, out: SLAB, out_qty: 6, label: "SLABS" },
    Recipe { in_item: [COBBLE, 0], in_qty: [6, 0], n_in: 1, out: STAIRS_N, out_qty: 4, label: "STAIRS" },
    Recipe { in_item: [IRON_ORE, COAL_ORE], in_qty: [1, 1], n_in: 2, out: FLINT_STEEL, out_qty: 1, label: "FLINT+STEEL" },
    // Brewing: bottle -> awkward -> effect, one ingredient per step (Java).
    Recipe { in_item: [GLASS, 0], in_qty: [3, 0], n_in: 1, out: BOTTLE, out_qty: 3, label: "BOTTLES" },
    Recipe { in_item: [BOTTLE, EMBER_CAP], in_qty: [1, 1], n_in: 2, out: POTION_AWKWARD, out_qty: 1, label: "AWKWARD POT" },
    Recipe { in_item: [POTION_AWKWARD, SUGAR_CANE], in_qty: [1, 1], n_in: 2, out: POTION_SPEED, out_qty: 1, label: "POT SPEED" },
    Recipe { in_item: [VOID_PEARL, EMBER_ROD], in_qty: [1, 1], n_in: 2, out: VOID_EYE, out_qty: 1, label: "VOID EYE" },
    Recipe { in_item: [POTION_AWKWARD, EMBER_ROD], in_qty: [1, 1], n_in: 2, out: POTION_STRENGTH, out_qty: 1, label: "POT STRENGTH" },
    Recipe { in_item: [POTION_AWKWARD, WAILER_TEAR], in_qty: [1, 1], n_in: 2, out: POTION_REGEN, out_qty: 1, label: "POT REGEN" },
    Recipe { in_item: [POTION_AWKWARD, MAGMA_PASTE], in_qty: [1, 1], n_in: 2, out: POTION_FIRE, out_qty: 1, label: "POT FIRERES" },
    // Java: ember powder + slime ball. No slimes here, so a rod and gunpowder.
    Recipe { in_item: [EMBER_ROD, GUNPOWDER], in_qty: [1, 1], n_in: 2, out: MAGMA_PASTE, out_qty: 1, label: "MAGMA PASTE" },
    Recipe { in_item: [PLANK, STICK], in_qty: [4, 2], n_in: 2, out: FENCE, out_qty: 3, label: "FENCES" },
    Recipe { in_item: [COBBLE, 0], in_qty: [8, 0], n_in: 1, out: FURNACE, out_qty: 1, label: "FURNACE" },
    Recipe { in_item: [PLANK, STICK], in_qty: [3, 2], n_in: 2, out: CRAFT_PICK, out_qty: 1, label: "WOOD PICKAXE" },
    Recipe { in_item: [PLANK, STICK], in_qty: [3, 2], n_in: 2, out: CRAFT_AXE, out_qty: 1, label: "WOOD AXE" },
    Recipe { in_item: [PLANK, STICK], in_qty: [1, 2], n_in: 2, out: CRAFT_SHOVEL, out_qty: 1, label: "WOOD SHOVEL" },
    Recipe { in_item: [PLANK, STICK], in_qty: [2, 1], n_in: 2, out: CRAFT_SWORD, out_qty: 1, label: "WOOD SWORD" },
    Recipe { in_item: [COBBLE, STICK], in_qty: [3, 2], n_in: 2, out: CRAFT_PICK, out_qty: 2, label: "STONE PICKAXE" },
    Recipe { in_item: [COBBLE, STICK], in_qty: [3, 2], n_in: 2, out: CRAFT_AXE, out_qty: 2, label: "STONE AXE" },
    Recipe { in_item: [COBBLE, STICK], in_qty: [1, 2], n_in: 2, out: CRAFT_SHOVEL, out_qty: 2, label: "STONE SHOVEL" },
    Recipe { in_item: [COBBLE, STICK], in_qty: [2, 1], n_in: 2, out: CRAFT_SWORD, out_qty: 2, label: "STONE SWORD" },
    Recipe { in_item: [IRON_INGOT, STICK], in_qty: [3, 2], n_in: 2, out: CRAFT_PICK, out_qty: 3, label: "IRON PICKAXE" },
    Recipe { in_item: [IRON_INGOT, STICK], in_qty: [3, 2], n_in: 2, out: CRAFT_AXE, out_qty: 3, label: "IRON AXE" },
    Recipe { in_item: [IRON_INGOT, STICK], in_qty: [1, 2], n_in: 2, out: CRAFT_SHOVEL, out_qty: 3, label: "IRON SHOVEL" },
    Recipe { in_item: [IRON_INGOT, STICK], in_qty: [2, 1], n_in: 2, out: CRAFT_SWORD, out_qty: 3, label: "IRON SWORD" },
    Recipe { in_item: [DIAMOND_ORE, STICK], in_qty: [3, 2], n_in: 2, out: CRAFT_PICK, out_qty: 4, label: "DIAMOND PICKAXE" },
    Recipe { in_item: [DIAMOND_ORE, STICK], in_qty: [3, 2], n_in: 2, out: CRAFT_AXE, out_qty: 4, label: "DIAMOND AXE" },
    Recipe { in_item: [DIAMOND_ORE, STICK], in_qty: [1, 2], n_in: 2, out: CRAFT_SHOVEL, out_qty: 4, label: "DIAMOND SHOVEL" },
    Recipe { in_item: [DIAMOND_ORE, STICK], in_qty: [2, 1], n_in: 2, out: CRAFT_SWORD, out_qty: 4, label: "DIAMOND SWORD" },
    Recipe { in_item: [SAND, COAL_ORE], in_qty: [1, 1], n_in: 2, out: GLASS, out_qty: 1, label: "GLASS" },
    Recipe { in_item: [WOOL, PLANK], in_qty: [3, 3], n_in: 2, out: BED, out_qty: 1, label: "BED" },
    Recipe { in_item: [COAL_ORE, 0], in_qty: [1, 0], n_in: 1, out: WIRE, out_qty: 4, label: "WIRE" },
    Recipe { in_item: [COAL_ORE, STICK], in_qty: [1, 1], n_in: 2, out: TORCH, out_qty: 4, label: "TORCH" },
    Recipe { in_item: [STICK, 0], in_qty: [7, 0], n_in: 1, out: LADDER, out_qty: 3, label: "LADDER" },
    Recipe { in_item: [PLANK, IRON_INGOT], in_qty: [3, 1], n_in: 2, out: PISTON, out_qty: 1, label: "PISTON" },
    Recipe { in_item: [GUNPOWDER, SAND], in_qty: [5, 4], n_in: 2, out: TNT, out_qty: 1, label: "TNT" },
    Recipe { in_item: [WHEAT_ITEM, 0], in_qty: [3, 0], n_in: 1, out: BREAD, out_qty: 1, label: "BREAD" },
    Recipe { in_item: [IRON_INGOT, 0], in_qty: [5, 0], n_in: 1, out: CRAFT_ARMOR, out_qty: 1, label: "IRON ARMOR" },
    Recipe { in_item: [DIAMOND_ORE, 0], in_qty: [5, 0], n_in: 1, out: CRAFT_ARMOR, out_qty: 2, label: "DIAMOND ARMOR" },
    Recipe { in_item: [STICK, STRING], in_qty: [3, 3], n_in: 2, out: BOW, out_qty: 1, label: "BOW" },
    Recipe { in_item: [STICK, COAL_ORE], in_qty: [1, 1], n_in: 2, out: ARROW, out_qty: 4, label: "ARROWS" },
    Recipe { in_item: [IRON_INGOT, 0], in_qty: [3, 0], n_in: 1, out: BUCKET, out_qty: 1, label: "BUCKET" },
    Recipe { in_item: [BONE, 0], in_qty: [1, 0], n_in: 1, out: BONEMEAL, out_qty: 3, label: "BONEMEAL" },
    Recipe { in_item: [STICK, STRING], in_qty: [3, 2], n_in: 2, out: FISHING_ROD, out_qty: 1, label: "FISH ROD" },
    Recipe { in_item: [OBSIDIAN, DIAMOND_ORE], in_qty: [4, 2], n_in: 2, out: ENCHANT, out_qty: 1, label: "ENCHANT TBL" },
];

// Category tabs, the console-edition answer to a long flat recipe list: the
// legacy console UI grouped recipes into L1/R1-switched tabs and Bedrock's
// PS4 UI keeps the same category tabs, so L1/R1 pages these. TRIANGLE hides
// recipes the inventory cannot pay for.
const CRAFT_TABS: usize = 4;
const CRAFT_TAB_TITLE: [&str; CRAFT_TABS] =
    ["CRAFTING < BLOCKS >", "CRAFTING < GEAR >", "CRAFTING < ITEMS >", "CRAFTING < FOOD >"];
static mut CRAFT_TAB: usize = 0;
static mut CRAFT_HIDE: bool = false;
/// True while the crafting menu was opened at a placed crafting table; the
/// handheld (SQUARE) menu is the 2x2 pocket grid and only offers what that
/// grid can shape.
static mut AT_BENCH: bool = false;

// Menu hold-to-scroll: frames each nav direction has been held.
static mut NAV_UP_T: u16 = 0;
static mut NAV_DOWN_T: u16 = 0;

/// Step-once-then-autorepeat: fires on the first held frame, then every 4
/// frames once past an 18-frame delay.
fn nav_repeat(held: bool, t: &mut u16) -> bool {
    if !held {
        *t = 0;
        return false;
    }
    *t = t.saturating_add(1);
    *t == 1 || (*t > 18 && (*t - 18) % 4 == 0)
}

// One-shot golden toast over the hotbar: what a craft just equipped.
static mut EQUIP_MSG: &str = "";
static mut EQUIP_T: u16 = 0;

/// Inventory panel filter: owned items only. Defaults ON -- the full catalogue
/// is the browsing view, not the working one.
static mut INV_HIDE: bool = true;

/// The inventory panel's working set: item ids, filtered to owned stacks
/// while the filter is on.
fn inv_list(out: &mut [u8; PLACEABLE.len()]) -> usize {
    let hide = unsafe { INV_HIDE };
    let mut n = 0;
    let mut i = 0;
    while i < PLACEABLE.len() {
        if !hide || unsafe { INV[PLACEABLE[i] as usize] } > 0 {
            out[n] = PLACEABLE[i];
            n += 1;
        }
        i += 1;
    }
    n
}

/// Recipes the 2x2 pocket grid cannot make -- everything beyond planks,
/// sticks, torches and the table itself wants the 3x3 bench, as in Java.
fn needs_bench(i: usize) -> bool {
    !matches!(RECIPES[i].out, PLANK | STICK | TORCH | CRAFT_TABLE)
}

/// A recipe the menu should show as makeable right now.
fn craftable_here(i: usize) -> bool {
    can_craft(i) && (unsafe { AT_BENCH } || !needs_bench(i))
}

// --- contextual tutorial ---------------------------------------------------
// Java-style hint chain: one small toast at the top-right watches the live
// game state and walks a fixed order -- look, move, jump, chop, craft, bench,
// tool. No scripting: each step is a badge, a line, and a done-predicate over
// state the game already tracks. The box hides while a menu is open and the
// chain retires for the session once it finishes (OPTIONS can turn it off).
static mut TUT_STEP: usize = 0;
static mut TUT_ENABLED: bool = true;
static mut TUT_TIMER: u32 = 0; // frames on the current step (paces the box)
static mut TUT_LOOK: i32 = 0;
static mut TUT_WALK: i32 = 0;
static mut TUT_PREV: (u16, i16, i32, i32) = (0, 0, 0, 0); // yaw, pitch, x, z
static mut TUT_JUMPED: bool = false;
static mut TUT_TABLE_PLACED: bool = false;
static mut TUT_BENCHED: bool = false;
const TUT_DONE: usize = 11;
const TUT_STEPS: [(&str, &str); TUT_DONE] = [
    ("", "RIGHT STICK: LOOK AROUND"),
    ("", "LEFT STICK: WALK"),
    ("X", "JUMP"),
    ("R2", "HOLD: CHOP A TREE"),
    ("[]", "CRAFT PLANKS"),
    ("[]", "CRAFT A CRAFTING TABLE"),
    ("L2", "PLACE THE CRAFTING TABLE"),
    ("L2", "AIM AT THE TABLE TO OPEN"),
    ("X", "CRAFT STICKS + A PICKAXE"),
    ("", "TOOLS SUIT WHAT YOU AIM AT"),
    ("", "YOU KNOW THE BASICS!"),
];

fn tut_reset() {
    unsafe {
        TUT_STEP = 0;
        TUT_TIMER = 0;
        TUT_LOOK = 0;
        TUT_WALK = 0;
        TUT_JUMPED = false;
        TUT_TABLE_PLACED = false;
        TUT_BENCHED = false;
    }
}

/// Advance the chain from the live game state. Runs every frame, menus
/// included -- crafting steps complete while their menu is open.
fn tut_tick(player: &Player) {
    unsafe {
        if !TUT_ENABLED || TUT_STEP >= TUT_DONE {
            return;
        }
        TUT_TIMER += 1;
        let (py, pp, px, pz) = TUT_PREV;
        TUT_PREV = (player.yaw, player.pitch, player.x, player.z);
        if TUT_TIMER > 1 {
            // Progress meters for the first two steps (wrap-aware yaw).
            if TUT_STEP == 0 {
                let dy = (((player.yaw as i32 - py as i32) + 2048) & 4095) - 2048;
                TUT_LOOK += dy.abs() + (player.pitch - pp).abs() as i32;
            } else if TUT_STEP == 1 {
                TUT_WALK += (player.x - px).abs() + (player.z - pz).abs();
            }
        }
        let done = match TUT_STEP {
            0 => TUT_LOOK > 1024, // ~90 degrees of accumulated look
            1 => TUT_WALK > 8 * BLOCK,
            2 => TUT_JUMPED,
            3 => INV[WOOD as usize] > 0,
            4 => INV[PLANK as usize] > 0,
            5 => INV[CRAFT_TABLE as usize] > 0 || TUT_TABLE_PLACED,
            6 => TUT_TABLE_PLACED,
            7 => TUT_BENCHED,
            8 => player.pick >= 1,
            9 => TUT_TIMER > 120, // point at the new tool slot for ~4s
            _ => TUT_TIMER > 150, // the sign-off lingers ~5s, then retires
        };
        if done {
            TUT_STEP += 1;
            TUT_TIMER = 0;
            if TUT_STEP < TUT_DONE {
                sfx::blip();
            }
        }
    }
}

fn badge_tint(key: &str) -> (u8, u8, u8) {
    match key.as_bytes() {
        b"X" => PS_CROSS,
        b"O" => PS_CIRCLE,
        b"T" => PS_TRIANGLE,
        b"[]" => PS_SQUARE,
        _ => PS_KEY,
    }
}

/// The hint toast, console-edition style: panel chrome, badge, dark ink.
fn draw_tutorial(font: &FontAtlas) {
    let (step, t, on) = unsafe { (TUT_STEP, TUT_TIMER, TUT_ENABLED) };
    if !on || step >= TUT_DONE {
        return;
    }
    // A short breath before each new hint, so an advance reads as such.
    if t < 40 && step != 0 {
        return;
    }
    let (key, text) = TUT_STEPS[step];
    let bw = if key.is_empty() { 0 } else { key.len() as i16 * 8 + 10 };
    let w = bw + text.len() as i16 * 8 + 12;
    let x = SCREEN_W as i16 - 6 - w;
    let y = 6i16;
    rect(x - 2, y - 2, w + 4, 18, 0, 0, 0);
    rect(x, y, w, 14, 0xC6, 0xC6, 0xC6);
    let mut tx = x + 6;
    if !key.is_empty() {
        tx = ui_badge(font, tx, y + 3, key, badge_tint(key)) + 3;
    }
    ui_text(font, tx, y + 3, text, MC_INK);
}

// Sleep prompt: set each frame from the pick, drawn with the HUD.
static mut PROMPT_SLEEP: bool = false;

fn draw_sleep_prompt(font: &FontAtlas) {
    if !unsafe { PROMPT_SLEEP } {
        return;
    }
    let w = 23 + 3 + 5 * 8;
    let x = (SCREEN_W as i16 - w) / 2;
    let y = HUD_ROW_Y - 18;
    let nx = ui_badge(font, x, y, "L2", PS_KEY) + 3;
    ui_text(font, nx + 1, y + 1, "SLEEP", (0, 0, 0));
    ui_text(font, nx, y, "SLEEP", (0xF0, 0xF0, 0xF0));
}

/// Which tab a recipe lives on, by its output: gear (tools/armor/weapons and
/// the held kit), food and brewing, small items, and everything placeable
/// under blocks.
fn recipe_tab(i: usize) -> usize {
    match RECIPES[i].out {
        CRAFT_PICK | CRAFT_AXE | CRAFT_SHOVEL | CRAFT_SWORD | CRAFT_ARMOR | BOW | ARROW
        | FLINT_STEEL | BUCKET | FISHING_ROD => 1,
        STICK | WIRE | TORCH | BONEMEAL | VOID_EYE | MAGMA_PASTE => 2,
        BREAD | BOTTLE | POTION_AWKWARD | POTION_SPEED | POTION_STRENGTH | POTION_REGEN
        | POTION_FIRE => 3,
        _ => 0,
    }
}

/// Page the category tab, skipping any tab that would come up empty -- the
/// pocket grid only reaches a few recipes, so most tabs are blank without a
/// table and paging through them looks broken.
fn step_craft_tab(dir: i32) {
    let mut scratch = [0u8; RECIPES.len()];
    let mut i = 0;
    while i < CRAFT_TABS {
        unsafe {
            CRAFT_TAB = (CRAFT_TAB as i32 + dir).rem_euclid(CRAFT_TABS as i32) as usize;
        }
        if craft_list(&mut scratch) > 0 {
            return; // landed on a tab with something in it
        }
        i += 1;
    }
    // Every tab is empty (nothing craftable at all): leave the tab where it is.
}

/// The working set the crafting menu shows and selects over: recipe indices on
/// the current tab, minus the unaffordable ones while the filter is on.
fn craft_list(out: &mut [u8; RECIPES.len()]) -> usize {
    let (tab, hide) = unsafe { (CRAFT_TAB, CRAFT_HIDE) };
    let mut n = 0;
    let mut i = 0;
    while i < RECIPES.len() {
        // Away from a table, bench-only recipes are not listed at all. They
        // used to sit greyed with a "NEEDS CRAFTING TABLE" note, which made
        // the pocket menu look mostly broken; the table's own menu still
        // shows the full book.
        let reachable = unsafe { AT_BENCH } || !needs_bench(i);
        if recipe_tab(i) == tab && reachable && (!hide || craftable_here(i)) {
            out[n] = i as u8;
            n += 1;
        }
        i += 1;
    }
    n
}

#[inline(never)]
fn can_craft(i: usize) -> bool {
    let r = &RECIPES[i];
    let mut k = 0;
    while k < r.n_in as usize {
        if unsafe { INV[r.in_item[k] as usize] } < r.in_qty[k] {
            return false;
        }
        k += 1;
    }
    true
}

/// Consume a recipe's inputs and grant its output (item to INV, or tool upgrade).
#[inline(never)]
fn craft(i: usize, player: &mut Player) {
    if !can_craft(i) {
        return;
    }
    let r = &RECIPES[i];
    let mut k = 0;
    while k < r.n_in as usize {
        unsafe {
            INV[r.in_item[k] as usize] -= r.in_qty[k];
        }
        k += 1;
    }
    if is_tool_recipe(r.out) {
        let tier = r.out_qty as u8;
        let slot = match r.out {
            CRAFT_AXE => &mut player.axe,
            CRAFT_SHOVEL => &mut player.shovel,
            CRAFT_SWORD => &mut player.sword,
            _ => &mut player.pick,
        };
        if tier > *slot {
            *slot = tier;
            unsafe {
                EQUIP_MSG = match r.out {
                    CRAFT_AXE => "AXE EQUIPPED",
                    CRAFT_SHOVEL => "SHOVEL EQUIPPED",
                    CRAFT_SWORD => "SWORD EQUIPPED",
                    _ => "PICKAXE EQUIPPED",
                };
                EQUIP_T = 100;
            }
        }
    } else if r.out == CRAFT_ARMOR {
        if r.out_qty as u8 > player.armor {
            player.armor = r.out_qty as u8;
            unsafe {
                EQUIP_MSG = if player.armor == 1 {
                    "IRON ARMOR EQUIPPED"
                } else {
                    "DIAMOND ARMOR EQUIPPED"
                };
                EQUIP_T = 100;
            }
        }
    } else {
        inv_give(r.out, r.out_qty);
    }
}

#[no_mangle]
fn main() {
    tty::println("voxide: boot");

    gpu::init(VideoMode::Ntsc, Resolution::R320X240);
    let mut fb = FrameBuffer::new(SCREEN_W, SCREEN_H);
    gpu::set_draw_area(0, 0, SCREEN_W - 1, SCREEN_H - 1);
    gpu::set_draw_offset(0, 0);

    let font = FontAtlas::upload(&BASIC, FONT_TPAGE, FONT_CLUT);
    unsafe {
        BLOCK_TEX = tex::upload();
    }
    init_mat_tables();
    refresh_mat_ccmd();
    // ORDER MATTERS: enable analog BEFORE turning on interrupts. The DualShock
    // analog-enable is a tight one-shot SIO config dance (0x43/0x44) with no
    // retry; a VBlank ISR firing mid-dance desyncs it on real hardware, leaving
    // the pad in digital mode (stick dead) for the whole session while the
    // retrying poll keeps the buttons alive. install_vblank_counter() flips on
    // CPU interrupts, so it must run AFTER the handshake. (Latent until psx-rt
    // 2aceef5c fixed an MFC0 hazard that had kept interrupt-enable from actually
    // taking effect -- which is why analog "used to work".)
    let _ = enable_analog_port1();
    interrupts::install_vblank_counter();
    load_shared_settings();
    sfx::init();
    show_intro(&mut fb, &font);

    unsafe {
        let mut z = 1usize;
        while z < RECIP_LEN {
            RECIP[z] = (PROJ_H << 16) / z as i32;
            z += 1;
        }
    }

    let (spawn_bx, spawn_bz) = if SPAWN_GREEN {
        world::pick_spawn(SPAWN_BX, SPAWN_BZ)
    } else {
        (SPAWN_BX, SPAWN_BZ)
    };
    unsafe {
        WORLD_BX = spawn_bx;
        WORLD_BZ = spawn_bz;
        RESPAWN_BX = spawn_bx;
        RESPAWN_BZ = spawn_bz;
    }
    // No synchronous boot gen: the main menu appears immediately and pumps the
    // streaming machinery while it is up, so the world assembles behind the
    // buttons. PLAY finishes any remainder behind the plain progress bar
    // (usually nothing is left by the time a human has picked an option).
    world::boot_prepare(spawn_bx, spawn_bz);
    main_menu(&mut fb, &font);
    tty::println("voxide: world ready");
    // The starter kit for both a fresh boot and however many NEW WORLD
    // reseeds happened in the menu.
    reset_game_state();
    let mut player = spawn_player();
    mob::populate(player.x, player.z);
    // Prime the edge detector with whatever is ACTUALLY held right now, not
    // NONE. The title screen exits on START, world generation then runs for
    // several seconds without polling, and the player is usually still holding
    // the button when the gameplay loop starts. Against a NONE baseline that
    // reads as a fresh press and drops you into the world with the options menu
    // already open.
    let mut previous = poll_port1().buttons;
    // Frames since the world opened. Monotonic: it counts loop iterations and
    // nothing else moves it.
    let mut frame: u32 = 0;
    // The world clock, in frames: `frame` plus however much sleeping has
    // skipped.
    //
    // Separate because sleeping jumps it to dawn, and this used to be `frame`
    // itself. Everything reading a frame count for another reason was dragged
    // forward with it -- the telemetry frame boundary, the demo's scripted
    // timings, the `% 8` and `% 7` periodic work, and the particle and mob RNG
    // seeds all skipped a few hundred frames on every night slept through. One
    // counter cannot be both monotonic and skippable.
    let mut day: u32 = 0;
    let mut prev_vbl = interrupts::vblank_count();
    let mut cast_t: u16 = 0; // fishing: frames until a bite while the rod is cast
    let mut swing: i32 = 0; // held-item swing animation countdown (mine/place/attack)
    let mut mine_active = false;
    // Frames spent standing in portal sheet. Java makes you wait ~4s in
    // survival; a shorter dwell here, but a dwell all the same, so brushing
    // past a portal does not fling you into the Inferno.
    let mut portal_dwell: u16 = 0;
    let (mut mine_x, mut mine_y, mut mine_z) = (0i32, 0i32, 0i32);
    let mut mine_progress: u32 = 0;
    let mut menu: u8 = 0; // 0 none, 1 crafting, 2 chest, 3 furnace
    let mut menu_sel = 0usize;
    let mut chest_idx = 0usize;
    // The first gameplay frame primes the GPU. Thereafter frame N+1 is built
    // while frame N rasterises, with this flag tracking that pending frame.
    let mut render_in_flight = false;

    loop {
        // Stop any SFX voice whose sample has ended. Ahead of everything that
        // might start one, so a sound keyed this frame is not cut by the
        // deadline of the one it replaced.
        sfx::tick();
        telemetry::frame_begin(frame); // profiler frame boundary (no-op without the feature)
        telemetry::stage_begin(50); // TEMP: whole loop body minus vsync
        // Honest frame cost: VBlank IRQs elapsed since the last loop top.
        // 1 vbl = 60 fps, 2 = 30, 3 = 20 (NTSC). Read before doing work.
        let now_vbl = interrupts::vblank_count();
        let dt = now_vbl.wrapping_sub(prev_vbl).max(1);
        prev_vbl = now_vbl;
        let fps = (60 / dt).min(99);
        // Fixed-timestep count: run the sim this many times per rendered frame so the
        // game runs at ~60Hz regardless of fps (capped so a hitch can't spiral).
        let sim_n = dt.min(4);

        telemetry::stage_begin(49); // TEMP: pad poll (SIO exchange)
        let state = poll_port1();
        telemetry::stage_end(49);
        let mut pad = state.buttons;
        let mut lstick = state.sticks.left_centered();
        let mut rstick = state.sticks.right_centered();
        // Safety net for a controller that wasn't ready at the boot handshake
        // (or gets re-plugged): re-assert analog while the pad still reports
        // non-analog. In the normal case the pre-interrupt boot enable already
        // made it analog, so this never fires. Skips once analog; harmless on a
        // digital-only pad.
        if !state.is_analog() && frame % 15 == 0 {
            let _ = enable_analog_port1();
        }
        if DEMO_PLAY {
            let (db, dl, dr) = demo_input(frame);
            pad = db;
            lstick = dl;
            rstick = dr;
            // Checkpoint state to the emulator console at phase boundaries so
            // the smoke output documents the playthrough textually.
            if frame % 120 == 0 {
                demo_checkpoint(frame, &player);
            }
        }
        if POSE_TEST {
            // Freeze all input and pin the exact pose; two ordinary edits
            // build the trigger tower ahead of the player, then nothing else
            // ever changes -- the scene is static from here on.
            pad = ButtonState::NONE;
            lstick = (0, 0);
            rstick = (0, 0);
            player.yaw = POSE_YAW;
            player.pitch = POSE_PITCH;
            if frame == 30 || frame == 40 {
                // Two ordinary edits, ten frames apart, exercising the edit
                // remesh chain; surface_y is re-read so the second lands on
                // the first (the first raises the surface by one).
                let bx = world_to_block_x(player.x);
                let bz = world_to_block_z(player.z);
                let sy = world::surface_y(bx, bz + 2);
                let b = if frame == 30 { DIRT } else { STONE };
                set_block_i32(bx, sy, bz + 2, b);
            }
        }

        // Bedrock PS layout: SQUARE toggles crafting, TRIANGLE the inventory.
        // Each opener only acts from the world or its own menu -- SQUARE means
        // "withdraw" inside the chest/furnace panels. The world sim pauses
        // while any menu is open.
        if (menu == 0 || menu == 1) && pressed(pad, previous, button::SQUARE) {
            menu = if menu == 1 { 0 } else { 1 };
            menu_sel = 0;
            unsafe { AT_BENCH = false }; // the handheld menu is the pocket grid
            sfx::blip();
        }
        if (menu == 0 || menu == MENU_INV) && pressed(pad, previous, button::TRIANGLE) {
            if menu == MENU_INV {
                menu = 0;
            } else {
                menu = MENU_INV;
                // Open on the item currently in hand, not the top of the list
                // (its position in the FILTERED list, which is what shows).
                let mut l = [0u8; PLACEABLE.len()];
                let ln = inv_list(&mut l);
                menu_sel = 0;
                let mut k = 0;
                while k < ln {
                    if l[k] == player.selected {
                        menu_sel = k;
                        break;
                    }
                    k += 1;
                }
            }
            sfx::blip();
        }
        // START opens the options menu, and closes whatever menu is open
        // (the death screen excepted -- only respawn leaves it).
        if menu != MENU_DEAD && pressed(pad, previous, button::START) {
            menu = if menu == 0 { MENU_OPTIONS } else { 0 };
            menu_sel = 0;
        }

        if menu != 0 {
            let mut craft_vis = [0u8; RECIPES.len()];
            let mut inv_vis = [0u8; PLACEABLE.len()];
            let n = match menu {
                1 => craft_list(&mut craft_vis),
                2 => PLACEABLE.len(),
                MENU_INV => inv_list(&mut inv_vis),
                MENU_OPTIONS => OPTIONS.len(),
                MENU_DEAD => 1,
                _ => FURN_ITEMS.len(),
            };
            // Crafting can shrink under the cursor (tab switch, filter, the
            // last affordable recipe crafted); keep the selection in the list.
            if menu_sel >= n {
                menu_sel = n.saturating_sub(1);
            }
            if menu == 1 {
                // L1/R1 page the category tabs, TRIANGLE toggles the
                // can-afford filter -- both console-edition conventions.
                if pressed(pad, previous, button::L1) {
                    step_craft_tab(-1);
                    menu_sel = 0;
                    sfx::blip();
                }
                if pressed(pad, previous, button::R1) {
                    step_craft_tab(1);
                    menu_sel = 0;
                    sfx::blip();
                }
                if pressed(pad, previous, button::TRIANGLE) {
                    unsafe { CRAFT_HIDE = !CRAFT_HIDE };
                    menu_sel = 0;
                    sfx::blip();
                }
            }
            // Hold-to-scroll: the first press steps once; keep holding and
            // after a short delay it repeats fast, the console list feel.
            let nav_up = nav_repeat(pad.is_held(button::UP), unsafe { &mut NAV_UP_T });
            let nav_down = nav_repeat(pad.is_held(button::DOWN), unsafe { &mut NAV_DOWN_T });
            if n > 0 && nav_up {
                menu_sel = (menu_sel + n - 1) % n;
                sfx::blip();
            }
            if n > 0 && nav_down {
                menu_sel = (menu_sel + 1) % n;
                sfx::blip();
            }
            // CIRCLE backs out of any menu, the PS convention -- except the
            // death screen, which only CROSS (respawn) leaves.
            if menu == MENU_DEAD {
                if pressed(pad, previous, button::CROSS) {
                    player = spawn_player();
                    mob::reset();
                    menu = 0;
                }
            } else if pressed(pad, previous, button::CIRCLE) {
                if menu == MENU_OPTIONS {
                    persist_shared_settings();
                }
                menu = 0;
            } else if menu == MENU_INV {
                if pressed(pad, previous, button::SQUARE) {
                    unsafe { INV_HIDE = !INV_HIDE };
                    menu_sel = 0;
                    sfx::blip();
                }
                if n > 0 && pressed(pad, previous, button::CROSS) {
                    let item = inv_vis[menu_sel];
                    if unsafe { INV[item as usize] } > 0 {
                        hotbar_pick(item);
                        player.selected = item;
                        menu = 0;
                        sfx::blip();
                    }
                }
            } else if menu == MENU_OPTIONS {
                // Settings rows adjust with left/right, like the main menu card.
                if menu_sel >= OPT_SETTINGS {
                    let dir = if pressed(pad, previous, button::LEFT) {
                        -1
                    } else if pressed(pad, previous, button::RIGHT) {
                        1
                    } else {
                        0
                    };
                    if dir != 0 {
                        setting_adjust(menu_sel - OPT_SETTINGS, dir);
                        sfx::blip();
                    }
                }
                if pressed(pad, previous, button::CROSS) {
                    match menu_sel {
                        OPT_FLIGHT => {
                            player.fly = !player.fly;
                            if player.fly {
                                player.vy = 0; // stop any fall the moment it engages
                            }
                            unsafe { OPT_MSG = "" };
                        }
                        OPT_SAVE => {
                            let ok = save::save(&player);
                            unsafe {
                                OPT_MSG = if ok {
                                    "SAVED"
                                } else if save::selftest() {
                                    "CARD FULL OR WRITE-PROTECTED"
                                } else {
                                    "NO MEMORY CARD IN SLOT 1"
                                };
                            }
                        }
                        OPT_TUTORIAL => {
                            unsafe { TUT_ENABLED = !TUT_ENABLED };
                            if unsafe { TUT_ENABLED } && unsafe { TUT_STEP } >= TUT_DONE {
                                tut_reset(); // re-enabling after a finish restarts it
                            }
                            unsafe { OPT_MSG = "" };
                        }
                        OPT_LOAD => {
                            if save::load(&mut player) {
                                // Edits are raw-set into the world, then the
                                // touched chunks remesh; the player may have
                                // landed in a different chunk, so recentre the
                                // streaming ring before the next frame draws.
                                save::apply_edits();
                                world::recenter(
                                    world_to_block_x(player.x),
                                    world_to_block_z(player.z),
                                );
                                unsafe { OPT_MSG = "LOADED" };
                            } else {
                                unsafe {
                                    OPT_MSG = if save::selftest() {
                                        "NO SAVE ON THIS CARD"
                                    } else {
                                        "NO MEMORY CARD IN SLOT 1"
                                    };
                                }
                            }
                        }
                        _ => {}
                    }
                    sfx::confirm();
                }
            } else if menu == 1 {
                if n > 0 && pressed(pad, previous, button::CROSS) {
                    let ri = craft_vis[menu_sel] as usize;
                    if craftable_here(ri) {
                        craft(ri, &mut player);
                        sfx::confirm();
                    } else {
                        sfx::blip();
                    }
                }
            } else if menu == 2 {
                let item = PLACEABLE[menu_sel];
                if pressed(pad, previous, button::CROSS) {
                    chest_deposit(chest_idx, item);
                    sfx::blip();
                }
                if pressed(pad, previous, button::SQUARE) {
                    chest_withdraw(chest_idx, item);
                    sfx::blip();
                }
            } else {
                if pressed(pad, previous, button::CROSS) {
                    furn_deposit(chest_idx, FURN_ITEMS[menu_sel]);
                    sfx::blip();
                }
                if pressed(pad, previous, button::SQUARE) {
                    furn_withdraw(chest_idx);
                    sfx::blip();
                }
            }
        } else {
            // Hotbar select: L1/R1 step the 9 real slots (Bedrock's shoulder
            // scroll), wrapping, empty slots included -- an empty slot is an
            // empty hand, as in the original. TRIANGLE's inventory panel jumps
            // straight to an item. Tools are not cycled: crafting a tier
            // equips it and better never hurts (no durability here).
            hotbar_sync(&mut player);
            if pressed(pad, previous, button::R1) {
                unsafe { HOTBAR_SEL = (HOTBAR_SEL + 1) % HOTBAR_VIS };
                player.selected = unsafe { HOTBAR[HOTBAR_SEL] };
                sfx::blip();
            }
            if pressed(pad, previous, button::L1) {
                unsafe { HOTBAR_SEL = (HOTBAR_SEL + HOTBAR_VIS - 1) % HOTBAR_VIS };
                player.selected = unsafe { HOTBAR[HOTBAR_SEL] };
                sfx::blip();
            }
            // Fixed timestep: advance movement / physics / mobs / survival sim_n times
            // (== capped fps delta) so pace is fps-independent. Edge actions inside
            // update_player (jump, fly toggle) fire only on the first sub-tick (prev_i).
            telemetry::stage_begin(ST_SIM);
            let mut st = 0;
            while st < sim_n {
                let prev_i = if st == 0 { previous } else { pad };
                update_player(&mut player, pad, prev_i, lstick, rstick);
                let hp_before = player.health;
                update_survival(&mut player);
                let night = (day_brightness(day % DAY_LEN) as i32) <= NIGHT_LIGHT;
                mob::set_lure(player.selected == WHEAT_ITEM); // animals follow held wheat
                mob::update(player.x, player.y, player.z, night);
                let raw_hit =
                    mob::contact_damage(player.x, player.y, player.z) + mob::hazard_damage();
                let mob_hit = armored(raw_hit, player.armor, player.protection);
                if mob_hit > 0 && player.hurt_cd == 0 {
                    player.health -= mob_hit;
                    player.hurt_cd = 16;
                    player.hurt_tilt = HURT_TILT_FRAMES;
                    player.regen_delay = REGEN_DELAY;
                }
                if player.health < hp_before {
                    sfx::hurt();
                }
                if player.health <= 0 {
                    // Death: hold on the YOU DIED screen; CROSS respawns.
                    // Inventory and hotbar are statics, so they persist.
                    menu = MENU_DEAD;
                    menu_sel = 0;
                    break;
                }
                // Keep the chunk grid centred on the player (cheap unless a boundary
                // was crossed).
                world::recenter(world_to_block_x(player.x), world_to_block_z(player.z));
                st += 1;
            }
            telemetry::stage_end(ST_SIM);
        }
        furn_tick(); // furnaces smelt whether or not a menu is open
        if frame % 8 == 0 {
            redstone_tick(); // budgeted: amortized, only over player-edited blocks
        }
        if frame % world::FLUID_INTERVAL == 0 {
            world::fluid_tick(); // budgeted: only cells woken by a player edit
        }
        if portal_tick(&player, &mut portal_dwell) {
            portal_travel(&mut player, &mut fb, &font);
        }
        tnt_tick(&mut player); // burn lit fuses; explode on zero
        crop_tick(); // age planted crops; ripen the mature ones
        sap_tick(); // grow planted saplings into trees

        if VISTA_VIEW {
            player.yaw = VISTA_YAW; // FIXED yaw: A/B captures must not drift with fps
            player.pitch = (-140i32 & 0x0FFF) as i16; // eye-level, slight down: the real gameplay view
        }
        let cam = camera_from_player(player);
        let pick = if menu != 0 { NO_PICK } else { trace_pick(&cam) };
        // The block under the crosshair decides which tool the HUD and the hand
        // show, so the set reads as one tool that changes to suit the job.
        let aimed_block = if pick.hit {
            get_block_i32(pick.bx, pick.by, pick.bz)
        } else {
            AIR
        };
        unsafe {
            // "Press L2 to sleep": aiming at a bed at (or near) nightfall --
            // the same window the bed interaction itself accepts.
            PROMPT_SLEEP = pick.hit
                && get_block_i32(pick.bx, pick.by, pick.bz) == BED
                && (day_brightness(day % DAY_LEN) as i32) <= NIGHT_LIGHT + 20;
        }

        let mut mine_den: u32 = 0;
        if menu == 0 {
            // "Use" is ONE button, L2, as on Bedrock: a mob in front takes it
            // first (bone tames a wolf, wheat trades with a villager), then the
            // block under the crosshair (chest, furnace, bed, enchant table,
            // door), then the held item's own action (place, bow, seeds...).
            // Sneak+L2 skips the block interaction and force-places against it,
            // the Java/Bedrock rule.
            let use_pressed = pressed(pad, previous, button::L2);
            let mut used = false;
            if use_pressed {
                used = mob_interact(&player);
            }
            if !used && use_pressed && pick.hit && !player.sneaking {
                let tb = get_block_i32(pick.bx, pick.by, pick.bz);
                if tb == CRAFT_TABLE {
                    used = true;
                    menu = 1;
                    menu_sel = 0;
                    unsafe {
                        AT_BENCH = true;
                        TUT_BENCHED = true;
                    }
                    sfx::confirm();
                } else if tb == CHEST {
                    used = true;
                    if let Some(ci) = chest_find(pick.bx, pick.by, pick.bz) {
                        menu = 2;
                        chest_idx = ci;
                        menu_sel = 0;
                        sfx::chest_open();
                    }
                } else if tb == FURNACE {
                    used = true;
                    if let Some(fi) = furn_find(pick.bx, pick.by, pick.bz) {
                        menu = 3;
                        chest_idx = fi;
                        menu_sel = 0;
                        sfx::chest_open();
                    }
                } else if tb == BED {
                    used = true;
                    // Sleep: set the respawn point here; at night, skip to morning.
                    unsafe {
                        RESPAWN_BX = pick.bx;
                        RESPAWN_BZ = pick.bz;
                    }
                    let dt = day % DAY_LEN;
                    if (day_brightness(dt) as i32) <= NIGHT_LIGHT + 20 {
                        // Only the world clock skips. `frame` keeps counting
                        // the frames that actually happened.
                        day += DAY_LEN - dt;
                    }
                    sfx::confirm();
                } else if tb == ENCHANT {
                    used = true;
                    // Spend XP levels at the table. Java rolls a random
                    // enchantment from the ones the item can take; here the
                    // table cycles efficiency -> sharpness -> protection, so
                    // three visits gets you one of each rather than a lottery.
                    if player.xp >= ENCHANT_COST && enchant_next(&player) != 0 {
                        player.xp -= ENCHANT_COST;
                        match enchant_next(&player) {
                            1 => player.efficiency += 1,
                            2 => player.sharpness += 1,
                            _ => player.protection += 1,
                        }
                        sfx::confirm();
                        spawn_particles(
                            block_to_world_x(pick.bx) + BLOCK / 2,
                            pick.by * BLOCK + BLOCK,
                            block_to_world_z(pick.bz) + BLOCK / 2,
                            (150, 110, 230),
                            12,
                            frame,
                            18,
                        );
                    } else {
                        sfx::blip();
                    }
                } else if tb == DOOR_C || tb == DOOR_O {
                    used = true;
                    // Toggle the door: closed <-> open (open is invisible + passable).
                    let nb = if tb == DOOR_C { DOOR_O } else { DOOR_C };
                    set_block_i32(pick.bx, pick.by, pick.bz, nb);
                    record_edit(pick.bx, pick.by, pick.bz, nb);
                    sfx::door();
                }
            }

            // Any R2/L2 action swings the arm.
            if pressed(pad, previous, button::R2) || use_pressed {
                swing = 8;
            }

            // Melee: tap R2 to strike a mob in front (before block mining).
            if pressed(pad, previous, button::R2) {
                let fx = (cam.sy * cam.cp) >> 12;
                let fz = (cam.cy * cam.cp) >> 12;
                // Java sword damage: fist 1, then wood 4 / stone 5 / iron 6 / diamond 7.
                let mut dmg = if player.sword == 0 { 1 } else { 3 + player.sword as i16 };
                if player.eff_strength > 0 {
                    dmg += dmg / 2; // Java strength I: +3 hearts-ish, here +50%
                }
                dmg += player.sharpness as i16; // Java: +1.25 per level, rounded here
                if mob::melee(cam.x, cam.y, cam.z, fx, fz, 2 * BLOCK + BLOCK / 2, dmg) {
                    sfx::hit_mob();
                }
            }

            // Mine: hold R2 to break the targeted block over time (by hardness).
            if pick.hit && pad.is_held(button::R2) {
                let tb = get_block_i32(pick.bx, pick.by, pick.bz);
                let hard = block_hardness(tb);
                if hard > 0 && pick.by != 0 {
                    let speed = mine_speed(&player, tb) + player.efficiency as u32 * 3;
                    if mine_active && mine_x == pick.bx && mine_y == pick.by && mine_z == pick.bz {
                        mine_progress += speed;
                    } else {
                        mine_active = true;
                        mine_x = pick.bx;
                        mine_y = pick.by;
                        mine_z = pick.bz;
                        mine_progress = speed;
                    }
                    mine_den = hard;
                    if frame % 7 == 0 {
                        sfx::dig(step_mat(tb), frame); // hit voiced by the block
                        swing = 8; // arm-swing each hit
                    }
                    if mine_progress >= hard {
                        sfx::break_block();
                        player.xp += match tb {
                            COAL_ORE => 1,
                            IRON_ORE | GOLD_ORE => 3,
                            DIAMOND_ORE => 7,
                            _ => 0,
                        };
                        spawn_particles(
                            block_to_world_x(pick.bx) + BLOCK / 2,
                            pick.by * BLOCK + BLOCK / 2,
                            block_to_world_z(pick.bz) + BLOCK / 2,
                            block_particle_color(tb),
                            10,
                            frame,
                            18,
                        );
                        // Everything a broken block yields now falls as an item
                        // entity from the centre of the block, instead of
                        // teleporting into the inventory.
                        let (dx, dy, dz) = (
                            block_to_world_x(pick.bx) + BLOCK / 2,
                            pick.by * BLOCK + BLOCK / 4,
                            block_to_world_z(pick.bz) + BLOCK / 2,
                        );
                        if tb == WHEAT_RIPE {
                            give_drop(dx, dy, dz, WHEAT_ITEM, frame); // mature: wheat + a seed back
                            give_drop(dx, dy, dz, SEEDS, frame ^ 0x9E37);
                        } else if tb == WHEAT {
                            give_drop(dx, dy, dz, SEEDS, frame); // immature: just the seed
                        } else if tb == LEAVES {
                            // ~30% sapling drop keeps wood renewable (Java-ish rate).
                            if (pick.bx.wrapping_mul(73) ^ pick.by.wrapping_mul(151) ^ pick.bz.wrapping_mul(37)) as u32 % 10 < 3 {
                                give_drop(dx, dy, dz, SAPLING, frame);
                            }
                        } else if player.pick >= mine_min_tier(tb) {
                            spawn_drop(dx, dy, dz, tb, frame); // under-tier: block breaks but drops nothing
                        }
                        if tb == CHEST {
                            chest_remove(pick.bx, pick.by, pick.bz);
                        } else if tb == FURNACE {
                            furn_remove(pick.bx, pick.by, pick.bz);
                        }
                        set_block_i32(pick.bx, pick.by, pick.bz, AIR);
                        record_edit(pick.bx, pick.by, pick.bz, AIR);
                        world::wake_fluid(pick.bx, pick.by, pick.bz);
                        mine_active = false;
                        mine_progress = 0;
                        mine_den = 0;
                    }
                }
            } else {
                mine_active = false;
                mine_progress = 0;
            }

            // Bow: tap L2 to loose an arrow along the look direction (needs no
            // block target). Player arrows damage mobs, not the player.
            if !used && player.selected == BOW && use_pressed {
                if inv_take(ARROW) {
                    let fx = (cam.sy * cam.cp) >> 12;
                    let fy = cam.sp;
                    let fz = (cam.cy * cam.cp) >> 12;
                    mob::player_shoot(cam.x, cam.y, cam.z, fx, fy, fz);
                    sfx::place(); // bow twang stand-in
                }
            }

            // Feed: tap L2 with wheat to put a nearby animal in love mode; two
            // fed animals breed (needs no block target).
            if !used && player.selected == WHEAT_ITEM && use_pressed {
                let fx = (cam.sy * cam.cp) >> 12;
                let fz = (cam.cy * cam.cp) >> 12;
                if unsafe { INV[WHEAT_ITEM as usize] } > 0
                    && mob::feed(cam.x, cam.y, cam.z, fx, fz, 2 * BLOCK + BLOCK / 2)
                {
                    inv_take(WHEAT_ITEM);
                    sfx::eat();
                }
            }

            // Place: tap L2. Seeds plant a crop on soil; everything else places
            // the selected block, consuming one from the inventory.
            if !used
                && player.selected != BOW
                && player.selected != WHEAT_ITEM
                && pick.hit
                && use_pressed
            {
                if player.selected == SEEDS {
                    let below = get_block_i32(pick.px, pick.py - 1, pick.pz);
                    if (below == GRASS || below == DIRT)
                        && get_block_i32(pick.px, pick.py, pick.pz) == AIR
                        && inv_take(SEEDS)
                    {
                        set_block_i32(pick.px, pick.py, pick.pz, WHEAT);
                        record_edit(pick.px, pick.py, pick.pz, WHEAT);
                        plant_crop(pick.px, pick.py, pick.pz);
                        sfx::place();
                    }
                } else if player.selected == SAPLING {
                    // Plant on soil like seeds; grows into a tree (see sap_tick).
                    let below = get_block_i32(pick.px, pick.py - 1, pick.pz);
                    if (below == GRASS || below == DIRT)
                        && get_block_i32(pick.px, pick.py, pick.pz) == AIR
                        && inv_take(SAPLING)
                    {
                        set_block_i32(pick.px, pick.py, pick.pz, SAPLING);
                        record_edit(pick.px, pick.py, pick.pz, SAPLING);
                        plant_sapling(pick.px, pick.py, pick.pz);
                        sfx::place();
                    }
                } else if player.selected == FISHING_ROD {
                    // Cast onto water; a bite arrives after cast_t frames (below).
                    if cast_t == 0 && is_water(get_block_i32(pick.bx, pick.by, pick.bz)) {
                        cast_t = 50 + (frame as u16 & 0x7F);
                        sfx::splash();
                    }
                } else if is_potion(player.selected) {
                    let kind = player.selected;
                    if inv_take(kind) {
                        drink_potion(&mut player, kind);
                        sfx::eat();
                    }
                } else if player.selected == VOID_EYE {
                    if light_frame(pick.px, pick.py, pick.pz, VOID_PORTAL) && inv_take(VOID_EYE) {
                        sfx::place();
                    }
                } else if player.selected == FLINT_STEEL {
                    // Light a portal frame if we struck one, otherwise start a
                    // fire on top of whatever we hit, as in Java.
                    if light_portal(pick.px, pick.py, pick.pz) {
                        sfx::place();
                    } else {
                        world::light_fire_at(pick.px, pick.py, pick.pz);
                        sfx::place();
                    }
                } else if player.selected == BONEMEAL {
                    // Bonemeal a young crop straight to ripe.
                    if get_block_i32(pick.bx, pick.by, pick.bz) == WHEAT && inv_take(BONEMEAL) {
                        set_block_i32(pick.bx, pick.by, pick.bz, WHEAT_RIPE);
                        record_edit(pick.bx, pick.by, pick.bz, WHEAT_RIPE);
                        spawn_particles(
                            block_to_world_x(pick.bx) + BLOCK / 2,
                            pick.by * BLOCK + BLOCK,
                            block_to_world_z(pick.bz) + BLOCK / 2,
                            (220, 230, 180),
                            8,
                            frame,
                            16,
                        );
                        sfx::place();
                    }
                } else if player.selected == BUCKET {
                    // Scoop the looked-at fluid into a filled bucket.
                    let tb = get_block_i32(pick.bx, pick.by, pick.bz);
                    let filled = if tb == WATER {
                        WATER_BUCKET
                    } else if tb == LAVA {
                        LAVA_BUCKET
                    } else {
                        AIR
                    };
                    if filled != AIR && inv_take(BUCKET) {
                        set_block_i32(pick.bx, pick.by, pick.bz, AIR);
                        record_edit(pick.bx, pick.by, pick.bz, AIR);
                        world::wake_fluid(pick.bx, pick.by, pick.bz);
                        inv_add(filled);
                        sfx::splash();
                    }
                } else if player.selected == WATER_BUCKET || player.selected == LAVA_BUCKET {
                    // Empty the bucket into the adjacent cell.
                    let fluid = if player.selected == WATER_BUCKET { WATER } else { LAVA };
                    if get_block_i32(pick.px, pick.py, pick.pz) == AIR && inv_take(player.selected) {
                        set_block_i32(pick.px, pick.py, pick.pz, fluid);
                        record_edit(pick.px, pick.py, pick.pz, fluid);
                        world::wake_fluid(pick.px, pick.py, pick.pz);
                        inv_add(BUCKET);
                        sfx::splash();
                    }
                } else if get_block_i32(pick.px, pick.py, pick.pz) == AIR
                    && !place_intersects_player(&player, pick.px, pick.py, pick.pz, player.selected)
                    && inv_take(player.selected)
                {
                    let put = if player.selected == STAIRS_N {
                        stairs_for_yaw(player.yaw)
                    } else {
                        player.selected
                    };
                    set_block_i32(pick.px, pick.py, pick.pz, put);
                    record_edit(pick.px, pick.py, pick.pz, put);
                    world::wake_fluid(pick.px, pick.py, pick.pz);
                    sfx::place();
                    if player.selected == CHEST {
                        chest_register(pick.px, pick.py, pick.pz);
                    } else if player.selected == FURNACE {
                        furn_register(pick.px, pick.py, pick.pz);
                    } else if player.selected == CRAFT_TABLE {
                        unsafe { TUT_TABLE_PLACED = true };
                    }
                } else {
                    sfx::blip(); // nothing placed (empty stack or blocked cell)
                }
            }

            // Fishing: a cast line counts down to a bite, then lands a fish.
            // Switching off the rod reels in (cancels the cast).
            if player.selected == FISHING_ROD {
                if cast_t > 0 {
                    cast_t -= 1;
                    if cast_t == 0 {
                        player.food_items += 1;
                        sfx::splash();
                    }
                }
            } else {
                cast_t = 0;
            }

            // Collect loot dropped by mobs killed this frame. Passive kills
            // yield RAW MEAT (cookable in a furnace); fishing still fills the
            // generic food_items pouch.
            // Loot from mobs killed this frame, dropped where they died rather
            // than teleported into the pack. Passives yield RAW MEAT (cookable
            // in a furnace); fishing still fills the generic food pouch.
            let deaths = mob::death_count();
            let mut d = 0usize;
            while d < deaths {
                let (mx, my, mz, kind) = mob::death_at(d);
                let seed = frame.wrapping_add(d as u32 * 977);
                if !mob::is_hostile(kind) {
                    give_drop(mx, my + BLOCK / 2, mz, RAW_MEAT, seed);
                    give_drop(mx, my + BLOCK / 2, mz, RAW_MEAT, seed ^ 0x51ED);
                }
                let extra = match kind {
                    mob::SHEEP => WOOL,
                    mob::SKELETON => BONE,
                    mob::SAPPER => GUNPOWDER, // slain (not exploded) sappers
                    mob::SPIDER => STRING,     // the bow's cord, as in Java
                    mob::WRAITH => VOID_PEARL,
                    mob::EMBER => EMBER_ROD,
                    mob::WAILER => WAILER_TEAR,
                    mob::CHARRED_SK => BONE,
                    _ => AIR,
                };
                if extra != AIR {
                    give_drop(mx, my + BLOCK / 2, mz, extra, seed ^ 0x2C1B);
                }
                d += 1;
            }
            mob::clear_deaths();
            player.xp += mob::take_xp() as i32;
        }

        // Weather: rain in one of every three ~40s windows. Phase 2, not 0, so a
        // fresh world always opens dry -- `% 3 == 0` meant frame 0 was rain and
        // every session started in a downpour.
        let (tod, rain, light, sky) = world_lighting(day);
        let raining = rain > 0;
        // Underground the horizon colour is nonsense: fade it (and, in
        // refresh_mat_ccmd, the far-band haze it meets) to cave black by how
        // deep the camera has gone.
        let cave = cave_amount(
            player.y,
            world_to_block_x(player.x),
            world_to_block_z(player.z),
        );
        let sky = (
            lerp_u8(sky.0 as i32, CAVE_FOG.0 as i32, cave as i32, 255),
            lerp_u8(sky.1 as i32, CAVE_FOG.1 as i32, cave as i32, 255),
            lerp_u8(sky.2 as i32, CAVE_FOG.2 as i32, cave as i32, 255),
        );
        unsafe {
            LIGHT = light;
            FOG_RGB = sky;
            CAVE = cave;
        }
        refresh_mat_ccmd(); // tint words follow LIGHT and the horizon colour
        if MOB_LINEUP {
            mob::debug_lineup(player.x, player.y, player.z);
            player.yaw = 0x800; // face -Z so the lineup's +Z faces point at us
            player.pitch = -180; // slight down at the row
        }
        if CRAFT_TEST.0 && frame == 20 {
            // Enough for some recipes on each tab, not all: the capture should
            // show bright, greyed, and (with the filter) hidden rows at once.
            inv_give(PLANK, 6);
            inv_give(STICK, 2);
            unsafe {
                CRAFT_TAB = CRAFT_TEST.1;
                CRAFT_HIDE = CRAFT_TEST.2;
            }
            menu = 1;
            menu_sel = 0;
        }
        if PLACE_TEST && frame == 5 {
            // A row of the once-corrupted placeables 5 blocks ahead (+Z).
            let bx = world_to_block_x(player.x);
            let bz = world_to_block_z(player.z) + 5;
            let row = [SAPLING, DOOR_C, CACTUS, CLAY, BRICK, TNT];
            let mut i = 0;
            while i < row.len() {
                let x = bx - 2 + i as i32;
                let y = world::surface_y(x, bz);
                world::set(x, y, bz, row[i]);
                i += 1;
            }
        }
        if PLANT_TEST {
            // Plant a small field of crops + flowers + tall grass just ahead and
            // aim at it, to verify the cross-sprite billboards.
            let bx = world_to_block_x(player.x);
            let bz0 = world_to_block_z(player.z) + 2;
            let mut dz = 0;
            while dz < 3 {
                let mut i = 0;
                while i < 5 {
                    let x = bx - 2 + i;
                    let z = bz0 + dz;
                    let y = world::surface_y(x, z) + 1;
                    let b = [WHEAT_RIPE, FLOWER_R, TALL_GRASS, FLOWER_Y, SAPLING][i as usize];
                    world::set(x, y, z, b);
                    i += 1;
                }
                dz += 1;
            }
            player.yaw = 0; // face +Z toward the field
            player.pitch = -240; // slight look-down: see the sprites standing up
        }
        tick_particles();
        tick_drops(&player);
        // No fb.clear: draw_sky's two gouraud triangles cover every pixel of
        // the frame, so the clear was 76.8K pixels of pure GPU overdraw --
        // roughly 0.15 vblank of fill, the difference between 30 and 60fps at
        // short draw distances.
        telemetry::stage_begin(ST_R_SKY);
        // Build only: the GPU is still rasterising the previous arena. Even GPU
        // register state such as dithering belongs in this frame's list.
        ui_frame_begin();
        queue_dither();
        draw_frame_sky(&cam, day, tod, light, sky, rain > 128);
        ui_finish_sky(false);
        telemetry::stage_end(ST_R_SKY);
        telemetry::stage_begin(ST_RENDER);
        gte_load_camera(&cam);
        telemetry::stage_begin(ST_R_WORLD);
        let (_world_quads, _face_work) = render_world(&cam);
        render_plants(&cam); // cross-sprite plants, depth-sorted into the same OT
        telemetry::stage_end(ST_R_WORLD);
        telemetry::stage_begin(ST_R_MOBS);
        render_mobs(&cam, frame);
        telemetry::stage_end(ST_R_MOBS);
        // Spend streaming work only from the headroom left by the current world
        // pass. Heavy terrain gets no extra work this frame; moderate terrain
        // gets one slice; cheap views get two. This breaks the old feedback loop
        // where a slow frame performed more streaming and became slower still.
        telemetry::stage_begin(ST_MESH);
        advance_streaming();
        telemetry::stage_end(ST_MESH);
        telemetry::stage_end(ST_RENDER);
        telemetry::stage_begin(ST_R_TAIL);
        render_particles(&cam);
        draw_pick_outline(&cam, pick);
        // Minecraft puts the destroy stage ON the block you are hitting. Ours
        // only had an 18px bar under the crosshair, which is nearly subliminal at
        // 320x240 and keeps your eye off the block.
        draw_break_overlay(&cam, pick, mine_progress, mine_den);
        if swing > 0 {
            swing -= 1;
        }
        if menu == 0 {
            draw_held_item(player.selected, hud_tool(&player, aimed_block), frame, swing);
        }
        if raining && under_open_sky(&player) {
            // Over the world, UNDER the HUD (it used to fall in front of the
            // hotbar, hearts and every open menu), and only where sky reaches.
            draw_rain(frame, &cam, rain);
        }
        // Full-screen tints. The GPU is measured at 17% busy with ZERO fill
        // cost, so a 320x240 blend is about 77K GPU cycles against ~940K of
        // headroom -- the one place left to buy feel without spending CPU.
        screen_tint(&player);
        tut_tick(&player);
        draw_all_hud(&font, player, menu, hud_tool(&player, aimed_block));
        if SHOW_FPS {
            ui_text(&font, 276, 6, &decimal3(fps as u16), (0xE0, 0xE0, 0x60));
        }
        if DEMO_PLAY {
            // Frame, player block position (+500 for signed coordinates), and
            // loaded/meshed/dirty chunk counts document headless captures.
            draw_demo_hud(&font, frame, &player);
        }
        draw_menu(&font, menu, menu_sel, chest_idx, player);
        // Particles, the pick outline, the held item, rain, tints, the HUD and
        // any open menu all live in OT slot 0 -- the slot the DMA walker
        // reaches last, so they paint over the world.
        ui_submit_tail();
        telemetry::stage_end(ST_R_TAIL);
        frame_present(&mut fb, &mut render_in_flight);
        previous = pad;
        frame = frame.wrapping_add(1);
        day = day.wrapping_add(1);
    }
}

/// Title screen: a slow camera orbit over the spawn world with the logo and a
/// blinking prompt. Returns once the player presses Start.
/// Returns Some(entropy) when the player asked for a NEW WORLD (SQUARE+START:
/// the title frame counter is honest player-timing entropy on hardware with no
/// RTC), or None for plain START into the default world.
/// One step of bar-time world building: recentre the ring on the spawn,
/// then a burst of gen/mesh slices. Also carves the spawn pocket the moment
/// the spawn chunk publishes (before its first mesh, so the carve is free).
/// Returns the remaining backlog.
fn menu_world_pump(pocket_done: &mut bool) -> u32 {
    let (sbx, sbz) = unsafe { (WORLD_BX, WORLD_BZ) };
    world::recenter(sbx, sbz);
    // Two bursts of the game's own streaming entry point, NOT a hand-rolled
    // gen_tick/stream_tick loop: a constant-bound loop over those calls was
    // MISCOMPILED under fat LTO (the optimizer cached the static-mut gen state
    // across the inlined iterations and the writes never landed -- the state
    // machine froze while ticking). advance_streaming is #[inline(never)], so
    // it is immune, and its backlog tiers give the menu 12 slices while the
    // ring is missing and next to nothing once it is complete.
    advance_streaming();
    advance_streaming();
    if !*pocket_done && world::column_loaded(sbx, sbz) {
        world::carve_spawn_pocket(sbx, sbz);
        *pocket_done = true;
    }
    let (n, f) = world::stream_backlog();
    n + f
}

/// The Minecraft-style main menu: the VOXIDE mark and widgets.png-bevel
/// buttons over the live spawn vista, which is GENERATING while you look at
/// it -- boot reaches this screen with no world at all, the pump assembles
/// terrain behind the buttons over the first few seconds, and PLAY finishes
/// whatever is left behind the plain progress bar. NEW WORLD reseeds and the
/// vista dissolves into the fresh terrain without leaving the menu.
fn main_menu(fb: &mut FrameBuffer, font: &FontAtlas) {
    let mut pocket_done = false;
    // The world loads FIRST, behind the plain bar, and the menu appears over a
    // COMPLETE vista. The previous design assembled the world behind the live
    // menu, which read nicely in still captures -- and made every menu frame
    // pay burst-tier streaming on real hardware, where it crawled. First
    // pad-in-hand test caught it in seconds; the bar takes a moment and then
    // the menu is one cheap static scene.
    finish_world_gen(fb, font, &mut pocket_done);
    if PROFILE_SKIP_TITLE || POSE_TEST {
        return;
    }
    let mut prev = poll_port1().buttons;
    let mut sel = 0usize; // 0 = play, 1 = new world, 2 = settings, 3 = credits
    let mut credits = false; // the provenance card is up
    let mut settings = false; // the settings card is up
    let mut set_sel = 0usize;
    let mut t: u32 = 0;
    loop {
        sfx::tick();
        let mut pad = poll_port1().buttons;
        if DEMO_PLAY && t == 90 {
            // Headless playtest: confirm PLAY at menu-frame 90.
            pad = ButtonState::from_bits(button::CROSS);
        }
        // The ring is complete; one idle-tier tick keeps it centred and
        // services stragglers without costing the menu anything.
        world::recenter(unsafe { WORLD_BX }, unsafe { WORLD_BZ });
        advance_streaming();
        if credits {
            // Any confirm/back dismisses the card.
            if pressed(pad, prev, button::CROSS)
                || pressed(pad, prev, button::CIRCLE)
                || pressed(pad, prev, button::START)
            {
                credits = false;
                sfx::blip();
            }
        } else if settings {
            if pressed(pad, prev, button::CIRCLE) || pressed(pad, prev, button::START) {
                persist_shared_settings();
                settings = false;
                sfx::blip();
            }
            if pressed(pad, prev, button::UP) {
                set_sel = (set_sel + SETTING_ROWS - 1) % SETTING_ROWS;
                sfx::blip();
            }
            if pressed(pad, prev, button::DOWN) {
                set_sel = (set_sel + 1) % SETTING_ROWS;
                sfx::blip();
            }
            let dir = if pressed(pad, prev, button::LEFT) {
                -1
            } else if pressed(pad, prev, button::RIGHT) {
                1
            } else {
                0
            };
            if dir != 0 {
                setting_adjust(set_sel, dir);
                sfx::blip();
            }
        } else {
            if pressed(pad, prev, button::UP) {
                sel = (sel + 3) % 4;
                sfx::blip();
            }
            if pressed(pad, prev, button::DOWN) {
                sel = (sel + 1) % 4;
                sfx::blip();
            }
        }
        let go = !credits
            && !settings
            && (pressed(pad, prev, button::CROSS) || pressed(pad, prev, button::START));
        prev = pad;
        if go && sel == 1 {
            // NEW WORLD: reseed from the menu timing (honest entropy, the PS1
            // has no clock) and let the pump regenerate behind the menu.
            world::prepare_new_world(t as i32 | 1);
            let (bx, bz) = if SPAWN_GREEN {
                world::pick_spawn(SPAWN_BX, SPAWN_BZ)
            } else {
                (SPAWN_BX, SPAWN_BZ)
            };
            unsafe {
                WORLD_BX = bx;
                WORLD_BZ = bz;
                RESPAWN_BX = bx;
                RESPAWN_BZ = bz;
            }
            pocket_done = false;
            sfx::confirm();
            // Regenerate behind the bar, then return to the menu over the
            // fresh vista.
            finish_world_gen(fb, font, &mut pocket_done);
            prev = poll_port1().buttons;
        } else if go && sel == 2 {
            settings = true;
            set_sel = 0;
            sfx::confirm();
        } else if go && sel == 3 {
            credits = true;
            sfx::confirm();
        } else if go {
            persist_shared_settings();
            sfx::confirm();
            return;
        }

        // The vista: a slow orbit over the spawn surface, exactly the game's
        // sky + world pass. surface_y answers from the noise field before the
        // chunk exists, so the camera height is right even at frame zero.
        let (sbx, sbz) = unsafe { (WORLD_BX, WORLD_BZ) };
        let yaw = (t.wrapping_mul(5) & 0x0FFF) as u16; // slow orbit
        let pitch = ((-340i32) & 0x0FFF) as u16; // tilt down ~30deg over the terrain
        let cam = Camera {
            x: block_to_world_x(sbx) + BLOCK / 2,
            y: world::surface_y(sbx, sbz) * BLOCK + 4 * BLOCK,
            z: block_to_world_z(sbz) + BLOCK / 2,
            sy: sincos::sin_q12(yaw),
            cy: sincos::cos_q12(yaw),
            sp: sincos::sin_q12(pitch),
            cp: sincos::cos_q12(pitch),
            roll: 0, // the menu orbit does not bob
        };
        let light = day_brightness(t % DAY_LEN);
        unsafe {
            LIGHT = light;
        }
        let lf = light as i32 - NIGHT_LIGHT;
        let ld = 128 - NIGHT_LIGHT;
        let sky = (
            lerp_u8(SKY_NIGHT.0, SKY_DAY.0, lf, ld),
            lerp_u8(SKY_NIGHT.1, SKY_DAY.1, lf, ld),
            lerp_u8(SKY_NIGHT.2, SKY_DAY.2, lf, ld),
        );
        let sky = apply_sunset(sky, t % DAY_LEN, false);
        unsafe { FOG_RGB = sky };
        refresh_mat_ccmd(); // tint words follow LIGHT and the horizon colour
        // No fb.clear: draw_sky covers every pixel (same as the game loop).
        ui_frame_begin();
        draw_sky(&cam, t, t % DAY_LEN, light, sky, false); // menu is always fair weather
        ui_finish_sky(true);
        gte_load_camera(&cam);
        render_world(&cam);
        unsafe {
            OT[RENDER_ARENA].submit();
        }
        // Drop shadow first: the mark must read over a bright sky and pale
        // terrain alike, the job the Minecraft logo's dark outline does.
        draw_mark_text(91, 43, "VoXide", 3, (0x20, 0x20, 0x28));
        draw_mark_text(88, 40, "VoXide", 3, (0xF4, 0xF4, 0xF4));
        if credits {
            draw_credits_now(font);
        } else if settings {
            draw_settings_now(font, set_sel);
        } else {
            menu_button_now(font, 140, "PLAY GAME", sel == 0);
            menu_button_now(font, 164, "NEW WORLD", sel == 1);
            menu_button_now(font, 188, "SETTINGS", sel == 2);
            menu_button_now(font, 212, "CREDITS", sel == 3);
            draw_splash_now(font, t);
            font.draw_text(6, SCREEN_H as i16 - 12, VERSION, (0xC8, 0xC8, 0xD0));
        }
        gpu::draw_sync();
        wait_vblank();
        fb.swap();
        t = t.wrapping_add(1);
    }
}

/// Finish whatever generation and meshing the menu pump has not covered yet,
/// behind the plain progress bar. Usually over in a blink; at worst (PLAY
/// mashed at frame zero) it is the old boot bar.
fn finish_world_gen(fb: &mut FrameBuffer, font: &FontAtlas, pocket_done: &mut bool) {
    let total = menu_world_pump(pocket_done) as usize;
    // A defensive ceiling: the full boot is ~150 pump steps, so thousands of
    // steps with no progress means the streaming machine is wedged -- show
    // the world as-is rather than freezing on the bar forever.
    let mut stalled = 0u32;
    let mut last_left = usize::MAX;
    loop {
        let left = menu_world_pump(pocket_done) as usize;
        if left == 0 && *pocket_done {
            return;
        }
        if left == last_left {
            stalled += 1;
            if stalled > 4000 {
                return;
            }
        } else {
            stalled = 0;
            last_left = left;
        }
        draw_loading(fb, font, total.saturating_sub(left), total.max(1));
    }
}

/// A widgets.png button in immediate mode -- the menu runs outside the game
/// frame's UI display list, so it cannot use `mc_button`/`rect`. Same colours.
// The yellow splash line -- in-house PS1 jokes where Mojang keeps theirs.
// Cycles every ~10s so an idle menu shows a few.
const SPLASHES: [&str; 13] = [
    "NOW WITH 2MB OF RAM!",
    "ALSO TRY WIPEOUT!",
    "RUNS ON REAL HARDWARE!",
    "AFFINE TEXTURES FOREVER!",
    "GTE GO BRRR!",
    "FITS ON ONE CD!",
    "SAVES TO MEMORY CARD!",
    "WOBBLY VERTICES INSIDE!",
    "MADE WITH RUST!",
    "NO DISC 2 REQUIRED!",
    "PRESS X TO CRAFT!",
    "NOT AN N64 GAME!",
    "LOADS IN ONE BAR!",
];

/// The splash, tilted and pulsing like the original. The font cannot rotate,
/// so the line climbs a pixel every couple of characters -- a stair that
/// reads as the classic slant at 320x240 -- and pulses between two yellows.
fn draw_splash_now(font: &FontAtlas, t: u32) {
    let sp = SPLASHES[(t / 300) as usize % SPLASHES.len()];
    let n = sp.len() as i16;
    let bob = ((t / 20) % 2) as i16;
    let col = if (t / 10) % 2 == 0 { (0xFF, 0xFF, 0x40) } else { (0xD8, 0xD8, 0x20) };
    let x0 = SCREEN_W as i16 - 14 - n * 8;
    let y0 = 92 + bob;
    let mut i = 0i16;
    while i < n {
        let ch = &sp[i as usize..i as usize + 1];
        let x = x0 + i * 8;
        let y = y0 - (i * 2) / 3;
        font.draw_text(x + 1, y + 1, ch, (0x30, 0x30, 0x08));
        font.draw_text(x, y, ch, col);
        i += 1;
    }
}

const SETTING_ROWS: usize = 5;
const SETTING_NAMES: [&str; SETTING_ROWS] =
    ["MOVE DEADZONE", "LOOK DEADZONE", "LOOK SPEED", "INVERT LOOK Y", "SFX VOLUME"];

fn load_shared_settings() {
    let Ok(profile) = psx_settings::load_slot_one(SETTINGS_FILE) else {
        return;
    };
    unsafe {
        SETTINGS_PROFILE = profile;
        SET_MOVE_DZ = SETTINGS_PROFILE.move_deadzone as i16;
        SET_LOOK_DZ = SETTINGS_PROFILE.look_deadzone as i16;
        SET_LOOK_PCT = SETTINGS_PROFILE.look_speed_percent as i32;
        SET_INVERT_Y = SETTINGS_PROFILE.invert_y();
        sfx::set_volume_pct(SETTINGS_PROFILE.sfx_volume as i32);
        SETTINGS_DIRTY = false;
    }
}

fn persist_shared_settings() {
    unsafe {
        if !SETTINGS_DIRTY {
            return;
        }
        SETTINGS_PROFILE.move_deadzone = SET_MOVE_DZ as u8;
        SETTINGS_PROFILE.look_deadzone = SET_LOOK_DZ as u8;
        SETTINGS_PROFILE.look_speed_percent = SET_LOOK_PCT as u8;
        SETTINGS_PROFILE.set_invert_y(SET_INVERT_Y);
        SETTINGS_PROFILE.sfx_volume = sfx::volume_pct() as u8;
        if psx_settings::save_slot_one(SETTINGS_FILE, SETTINGS_TITLE, &SETTINGS_PROFILE).is_ok() {
            SETTINGS_DIRTY = false;
        }
    }
}

/// Step one setting up or down, clamped to its range.
fn setting_adjust(row: usize, dir: i32) {
    unsafe {
        match row {
            0 => SET_MOVE_DZ = (SET_MOVE_DZ + dir as i16 * 2).clamp(4, 40),
            1 => SET_LOOK_DZ = (SET_LOOK_DZ + dir as i16 * 2).clamp(4, 40),
            2 => SET_LOOK_PCT = (SET_LOOK_PCT + dir * 20).clamp(60, 160),
            3 => SET_INVERT_Y = !SET_INVERT_Y,
            _ => sfx::set_volume_pct(sfx::volume_pct() + dir * 25),
        }
        SETTINGS_DIRTY = true;
    }
}

/// A setting's display value into a tiny buffer ("18", "120%", "ON").
fn setting_value(row: usize, buf: &mut [u8; 4]) -> usize {
    let (v, pct) = unsafe {
        match row {
            0 => (SET_MOVE_DZ as i32, false),
            1 => (SET_LOOK_DZ as i32, false),
            2 => (SET_LOOK_PCT, true),
            3 => return if SET_INVERT_Y { buf[..2].copy_from_slice(b"ON"); 2 } else { buf[..3].copy_from_slice(b"OFF"); 3 },
            _ => (sfx::volume_pct(), true),
        }
    };
    let mut n = 0;
    if v >= 100 {
        buf[n] = b'0' + (v / 100) as u8;
        n += 1;
    }
    if v >= 10 {
        buf[n] = b'0' + ((v / 10) % 10) as u8;
        n += 1;
    }
    buf[n] = b'0' + (v % 10) as u8;
    n += 1;
    if pct {
        buf[n] = b'%';
        n += 1;
    }
    n
}

/// Immediate-mode twin of the in-game dialog chrome (menu_panel), for the
/// main-menu cards, which draw after the OT has already gone out.
fn panel_now(x: i16, y: i16, w: i16, h: i16) {
    gpu::draw_rect_flat(x - 2, y - 2, (w + 4) as u16, (h + 4) as u16, 0, 0, 0);
    gpu::draw_rect_flat(x, y, w as u16, h as u16, 0xC6, 0xC6, 0xC6);
    gpu::draw_rect_flat(x, y, w as u16, 1, 0xFF, 0xFF, 0xFF);
    gpu::draw_rect_flat(x, y, 1, h as u16, 0xFF, 0xFF, 0xFF);
    gpu::draw_rect_flat(x, y + h - 1, w as u16, 1, 0x55, 0x55, 0x55);
    gpu::draw_rect_flat(x + w - 1, y, 1, h as u16, 0x55, 0x55, 0x55);
}

/// Immediate twin of ui_badge: the rounded dark pill with a PlayStation
/// glyph tint. Returns the x just past the pill.
fn badge_now(font: &FontAtlas, x: i16, y: i16, key: &str, tint: (u8, u8, u8)) -> i16 {
    let w = key.len() as i16 * 8 + 7;
    gpu::draw_rect_flat(x + 1, y - 2, (w - 2) as u16, 12, 0x22, 0x22, 0x22);
    gpu::draw_rect_flat(x, y - 1, w as u16, 10, 0x22, 0x22, 0x22);
    font.draw_text(x + 4, y, key, tint);
    x + w
}

/// Immediate twin of hint_item: badge + dark action label.
fn hint_now(font: &FontAtlas, x: i16, y: i16, key: &str, tint: (u8, u8, u8), action: &str) -> i16 {
    let nx = badge_now(font, x, y, key, tint) + 3;
    font.draw_text(nx, y, action, MC_INK);
    nx + action.len() as i16 * 8 + 8
}

/// Immediate twin of mc_button at the in-game row geometry.
fn row_button_now(y: i16, sel: bool) {
    let (x, w, h) = (MENU_BTN_X, MENU_BTN_W, MENU_BTN_H);
    gpu::draw_rect_flat(x, y, w as u16, h as u16, 0, 0, 0);
    let (top, bot) = if sel { (0xA6, 0x8B) } else { (0x8B, 0x6E) };
    let half = h / 2;
    gpu::draw_rect_flat(x + 1, y + 1, (w - 2) as u16, (half - 1) as u16, top, top, top);
    gpu::draw_rect_flat(x + 1, y + half, (w - 2) as u16, (h - half - 1) as u16, bot, bot, bot);
    let hi = if sel { 0xFF } else { 0xC6 };
    gpu::draw_rect_flat(x + 1, y + 1, (w - 2) as u16, 1, hi, hi, hi);
    gpu::draw_rect_flat(x + 1, y + 1, 1, (h - 2) as u16, hi, hi, hi);
    gpu::draw_rect_flat(x + 1, y + h - 2, (w - 2) as u16, 1, 0x37, 0x37, 0x37);
    gpu::draw_rect_flat(x + w - 2, y + 1, 1, (h - 2) as u16, 0x37, 0x37, 0x37);
}

/// The settings card: live tune of stick feel and volume, in the same
/// panel-and-bevel-rows dress as the in-game menus. Session-only.
fn draw_settings_now(font: &FontAtlas, sel: usize) {
    panel_now(MENU_PANEL_X, MENU_PANEL_Y, MENU_PANEL_W, MENU_PANEL_H);
    font.draw_text((SCREEN_W as i16 - 8 * 8) / 2, 12, "SETTINGS", MC_INK);
    let hx = hint_now(font, MENU_TEXT_X, MENU_HINT_Y, "< >", PS_KEY, "ADJUST");
    hint_now(font, hx, MENU_HINT_Y, "O", PS_CIRCLE, "BACK");
    let mut i = 0usize;
    while i < SETTING_ROWS {
        let y = MENU_ROWS_Y + i as i16 * MENU_ROW_H as i16;
        row_button_now(y - 3, i == sel);
        let color = if i == sel { MC_LABEL_SEL } else { MC_LABEL };
        font.draw_text(MENU_TEXT_X, y, SETTING_NAMES[i], color);
        let mut buf = [0u8; 4];
        let n = setting_value(i, &mut buf);
        let txt = unsafe { core::str::from_utf8_unchecked(&buf[..n]) };
        font.draw_text(278 - n as i16 * 8, y, txt, (0x70, 0xE0, 0x70));
        i += 1;
    }
    font.draw_text(MENU_TEXT_X, MENU_ROWS_Y + 5 * MENU_ROW_H as i16 + 8,
        "TAKES EFFECT IMMEDIATELY", (0x6A, 0x6A, 0x74));
}

/// The credits card: asset provenance in the same panel dress as every
/// other menu. The full list with URLs lives in assets/pack/CREDITS.md.
fn draw_credits_now(font: &FontAtlas) {
    panel_now(14, 4, SCREEN_W as i16 - 28, SCREEN_H as i16 - 8);
    // (indent px, text, style: 0 title, 1 heading, 2 body)
    const LINES: [(i16, &str, u8); 20] = [
        (0, "CREDITS", 0),
        (0, "", 2),
        (0, "BLOCK TEXTURES  (CC0)", 1),
        (10, "16X16 BLOCK TEXTURE SET", 2),
        (10, "FROM OPENGAMEART.ORG", 2),
        (0, "", 2),
        (0, "SOUNDS  (ALL CC0)", 1),
        (10, "KENNEY.NL: IMPACT SOUNDS,", 2),
        (10, "RPG AUDIO, INTERFACE SOUNDS", 2),
        (10, "FREESOUND.ORG:", 2),
        (18, "QUBODUP: PIG BARK EAT BOOM", 2),
        (18, "ZOZZY: COW  SQEEEEK: SHEEP", 2),
        (18, "MBPL: CHICKEN", 2),
        (18, "OWNATHAN: ZOMBIE", 2),
        (18, "KNEELING: BONES  REITANNA: HISS", 2),
        (18, "SWORDOFKINGS128: SPLASH", 2),
        (0, "", 2),
        (0, "FULL LINKS: CREDITS.MD IN THE REPO", 2),
        (0, "NOT AFFILIATED WITH MOJANG STUDIOS", 2),
        (0, "BUILT WITH PSOXIDE", 1),
    ];
    let mut y = 10i16;
    for (indent, text, style) in LINES {
        if !text.is_empty() {
            let (x, c) = match style {
                0 => ((SCREEN_W as i16 - text.len() as i16 * 8) / 2, (0x14, 0x14, 0x1A)),
                1 => (24 + indent, (0x10, 0x10, 0x16)),
                _ => (24 + indent, (0x2C, 0x2C, 0x34)),
            };
            font.draw_text(x, y, text, c);
        }
        y += if style == 0 { 14 } else { 11 };
    }
}

fn menu_button_now(font: &FontAtlas, y: i16, label: &str, sel: bool) {
    let (x, w, h) = (100i16, 120i16, 20i16);
    gpu::draw_rect_flat(x, y, w as u16, h as u16, 0, 0, 0);
    let (top, bot) = if sel { (0xA6, 0x8B) } else { (0x8B, 0x6E) };
    let half = h / 2;
    gpu::draw_rect_flat(x + 1, y + 1, (w - 2) as u16, (half - 1) as u16, top, top, top);
    gpu::draw_rect_flat(x + 1, y + half, (w - 2) as u16, (h - half - 1) as u16, bot, bot, bot);
    let hi = if sel { 0xFF } else { 0xC6 };
    gpu::draw_rect_flat(x + 1, y + 1, (w - 2) as u16, 1, hi, hi, hi);
    gpu::draw_rect_flat(x + 1, y + 1, 1, (h - 2) as u16, hi, hi, hi);
    gpu::draw_rect_flat(x + 1, y + h - 2, (w - 2) as u16, 1, 0x37, 0x37, 0x37);
    gpu::draw_rect_flat(x + w - 2, y + 1, 1, (h - 2) as u16, 0x37, 0x37, 0x37);
    let tx = x + (w - (label.len() as i16) * 8) / 2;
    let color = if sel { (0xFF, 0xFF, 0xA0) } else { (0xE0, 0xE0, 0xE0) };
    font.draw_text(tx, y + (h - 8) / 2, label, color);
}

/// Reset every gameplay registry for the "NEW WORLD" path: inventory back to
/// the starter kit, edit log, chests/furnaces/crops/saplings, respawn point,
/// mobs + arrows. World-side state is world::prepare_new_world's job.
fn reset_game_state() {
    tut_reset();
    unsafe {
        let mut i = 0;
        while i < BLOCK_KINDS {
            INV[i] = 0;
            i += 1;
        }
        let mut h = 0;
        while h < HOTBAR_VIS {
            HOTBAR[h] = AIR;
            h += 1;
        }
        HOTBAR_SEL = 0;
        INV[DIRT as usize] = 16;
        INV[SEEDS as usize] = 6;
        INV[BOW as usize] = 1;
        INV[ARROW as usize] = 8;
        INV[BUCKET as usize] = 1;
        if DEMO_PLAY && !DEMO_MARCH {
            INV[STONE as usize] = 4;
        }
        // The starter kit claims hotbar slots in grant order (the same
        // first-free-slot rule every later pickup follows).
        hotbar_add(DIRT);
        hotbar_add(SEEDS);
        hotbar_add(BOW);
        hotbar_add(ARROW);
        hotbar_add(BUCKET);
        if DEMO_PLAY && !DEMO_MARCH {
            hotbar_add(STONE);
        }
        EDIT_N = 0;
        let mut c = 0;
        while c < MAX_CHESTS {
            CHEST_USED[c] = false;
            c += 1;
        }
        let mut f = 0;
        while f < MAX_FURNACES {
            FURN_USED[f] = false;
            f += 1;
        }
        let mut k = 0;
        while k < MAX_CROPS {
            CROP_T[k] = 0;
            k += 1;
        }
        let mut s = 0;
        while s < MAX_SAPS {
            SAP_T[s] = 0;
            s += 1;
        }
        RESPAWN_BX = WORLD_BX;
        RESPAWN_BZ = WORLD_BZ;
    }
    mob::clear();
}

/// Wait for the next VBlank IRQ (the SDK's fixed version of the old
/// gpu::vsync() timer-reset bug; see psx_rt::interrupts::wait_vblank docs).
#[inline]
fn wait_vblank() {
    interrupts::wait_vblank();
}

// Bonnie Studios intro logo: 128x128 4bpp at a free VRAM column, its CLUT in
// the spare row under the block CLUT band (matches the celeste collection).
const BONNIE_TPAGE: Tpage = Tpage::new(896, 0, TexDepth::Bit4);
const BONNIE_CLUT_POS: Clut = Clut::new(768, 256);

/// Boot intro, ported from the celeste collection: fade the Bonnie Studios
/// logo in, hold, fade out, with the "Built with PSoXide" sheen line. Any
/// face button skips it.
fn show_intro(fb: &mut FrameBuffer, font: &FontAtlas) {
    psx_vram::upload_16bpp(
        psx_vram::VramRect::new(BONNIE_TPAGE.x(), BONNIE_TPAGE.y(), 32, 128),
        &bonnie::COVER_BONNIE,
    );
    let mut clut = bonnie::BONNIE_CLUT;
    clut[0] = 0x0421; // entry 0 opaque near-black (intro backdrop is black)
    psx_vram::upload_16bpp(
        psx_vram::VramRect::new(BONNIE_CLUT_POS.x(), BONNIE_CLUT_POS.y(), 16, 1),
        &clut,
    );

    const FADE_IN: i32 = 32;
    const HOLD: i32 = 74;
    const TOTAL: i32 = 150;
    const FADE_OUT: i32 = TOTAL - FADE_IN - HOLD;
    let any = |b: ButtonState| {
        b.is_held(button::CROSS) || b.is_held(button::CIRCLE) || b.is_held(button::START)
    };
    let mut prev = poll_port1().buttons;
    let mut t = 0i32;
    while t < TOTAL {
        let b = poll_port1().buttons;
        if t > 8 && any(b) && !any(prev) {
            break; // fresh press skips
        }
        prev = b;
        let lvl = if t < FADE_IN {
            t * 0x80 / FADE_IN
        } else if t < FADE_IN + HOLD {
            0x80
        } else {
            (TOTAL - t) * 0x80 / FADE_OUT
        }
        .clamp(0, 0x80) as u8;

        fb.clear(0, 0, 0);
        // Logo: 96x96 on screen from the 128x128 source, centred.
        gpu::draw_quad_textured(
            [(112, 34), (208, 34), (112, 130), (208, 130)],
            [(0, 0), (128, 0), (0, 128), (128, 128)],
            BONNIE_CLUT_POS.uv_clut_word(),
            BONNIE_TPAGE.uv_tpage_word(0),
            (lvl, lvl, lvl),
        );
        // "Built with PSoXide" -- gradient text with a sweeping sheen.
        let tag = "Built with PSoXide";
        let x0 = 160 - (font.text_width(tag) / 2) as i16;
        let span = tag.chars().count() as i32 + 18;
        let head = (t / 2).rem_euclid(span);
        let mix = |c: (u8, u8, u8), k: i32| -> (u8, u8, u8) {
            let f = |v: u8| {
                let base = v as i32 * lvl as i32 / 0x80;
                (base + (lvl as i32 - base) * k / 18) as u8
            };
            (f(c.0), f(c.1), f(c.2))
        };
        let mut x = x0;
        for (i, ch) in tag.char_indices() {
            let glyph = &tag[i..i + ch.len_utf8()];
            for (dx, dy) in [(-1, -1), (1, -1), (-1, 1), (1, 1)] {
                font.draw_text(x + dx, 150 + dy, glyph, (0, 0, 0));
            }
            let k = (18 - ((i as i32) - head).abs() * 6).max(0);
            font.draw_text_gradient(x, 150, glyph, mix((0x68, 0x80, 0x80), k), mix((0x38, 0x58, 0x80), k));
            x += font.text_width(glyph) as i16;
        }
        gpu::draw_sync();
        wait_vblank();
        fb.swap();
        t += 1;
    }
}

/// Boot/regen loading screen: dark backdrop, logo, and a progress bar. Drawn
/// (and swapped) once per generated/meshed chunk, ~50 frames over the ~9s
/// boot -- replaces what used to be a long black screen.
/// The world-generation bar: just the text and the progress, Minecraft's own
/// spartan style -- the logo lives on the main menu now, nowhere else.
fn draw_loading(fb: &mut FrameBuffer, font: &FontAtlas, done: usize, total: usize) {
    // Immediate-mode: this runs during world generation, with no ordering table
    // in flight, so it cannot use the UI display list (`rect`).
    gpu::draw_rect_flat(0, 0, 320, 240, 14, 14, 22);
    font.draw_text(96, 106, "GENERATING WORLD", (0xD0, 0xD0, 0xD0));
    gpu::draw_rect_flat(80, 124, 160, 10, 40, 40, 52);
    let w = (done * 156 / total.max(1)) as i16;
    gpu::draw_rect_flat(82, 126, w.max(2) as u16, 6, 120, 220, 120);
    gpu::draw_sync();
    fb.swap();
}

/// Draw the front-end mark from flat pixel runs instead of textured quads.
///
/// A scaled 8x8 atlas glyph is one large PS1 polygon. On silicon-accurate
/// rasterisation its two internal triangles do not necessarily choose the
/// same texel on their shared edge, which made narrow columns in the enlarged
/// `i` and `d` appear broken. Flat runs have no UV interpolation at all: every
/// source pixel becomes an exact screen-aligned square on emulator and console.
/// Eight-pixel cells, matching the original chunky BASIC mark.
///
/// These hand-audited rows deliberately replace the malformed lowercase `d`
/// in the legacy BASIC atlas as well as bypassing scaled-quad sampling. Bits
/// are left-to-right in 0x80..0x01.
fn mark_glyph(ch: u8) -> [u8; 8] {
    match ch {
        b'V' => [0xC3, 0xC3, 0xC3, 0xC3, 0xC3, 0x66, 0x3C, 0x00],
        b'o' => [0x00, 0x00, 0x3C, 0xC3, 0xC3, 0xC3, 0x3C, 0x00],
        b'X' => [0xC3, 0xC3, 0x66, 0x3C, 0x3C, 0x66, 0xC3, 0x00],
        b'i' => [0x18, 0x00, 0x38, 0x18, 0x18, 0x18, 0x3C, 0x00],
        b'd' => [0x03, 0x03, 0x03, 0x3F, 0xC3, 0xC3, 0xC3, 0x3F],
        b'e' => [0x00, 0x00, 0x3C, 0xC3, 0xFF, 0xC0, 0x3F, 0x00],
        b'P' => [0xFC, 0xC3, 0xC3, 0xFC, 0xC0, 0xC0, 0xC0, 0x00],
        b'S' => [0x3F, 0xC0, 0xC0, 0x3C, 0x03, 0x03, 0x7E, 0x00],
        _ => [0; 8],
    }
}

fn draw_mark_text(x: i16, y: i16, text: &str, scale: i16, color: (u8, u8, u8)) {
    for (glyph_index, ch) in text.bytes().enumerate() {
        let rows = mark_glyph(ch);
        let mut row = 0usize;
        while row < rows.len() {
            let bits = rows[row];
            let mut column = 0i16;
            while column < 8 {
                if bits & (0x80 >> column) == 0 {
                    column += 1;
                    continue;
                }
                let start = column;
                while column < 8 && bits & (0x80 >> column) != 0 {
                    column += 1;
                }
                gpu::draw_rect_flat(
                    x + (glyph_index as i16 * 8 + start) * scale,
                    y + row as i16 * scale,
                    ((column - start) * scale) as u16,
                    scale as u16,
                    color.0,
                    color.1,
                    color.2,
                );
            }
            row += 1;
        }
    }
}

fn spawn_player() -> Player {
    let (mut sx, mut sz) = unsafe { (RESPAWN_BX, RESPAWN_BZ) }; // bed if slept, else world spawn
    // At the default spawn, search outward for open lowland (not a peak/ocean) so
    // the first thing you see is a proper landscape, not a mountainside.
    let (wbx, wbz) = unsafe { (WORLD_BX, WORLD_BZ) };
    if sx == wbx && sz == wbz {
        // Biome-hunting captures may search wider, but stay inside the
        // boot-generated GRID (surface_y reads AIR beyond it).
        let rmax = if CAPTURE_BIOME >= 0 { 35 } else { 22 };
        let mut r: i32 = 0;
        'search: while r <= rmax {
            let mut dz: i32 = -r;
            while dz <= r {
                let mut dx: i32 = -r;
                while dx <= r {
                    if dx.abs().max(dz.abs()) == r {
                        let h = world::surface_y(wbx + dx, wbz + dz);
                        let biome_ok = CAPTURE_BIOME < 0
                            || world::biome_at(wbx + dx, wbz + dz, h) == CAPTURE_BIOME as u8;
                        // The surface must be actual GROUND: surface_y counts
                        // a tree canopy as the surface, and spawning there
                        // embeds the camera in leaves (see-through "void") and
                        // wedges the player -- the freeze-on-moving bug on
                        // regenerated worlds.
                        let top = world::get(wbx + dx, h - 1, wbz + dz);
                        let ground_ok =
                            top == GRASS || top == DIRT || top == SAND || top == SNOW || top == STONE;
                        // ...and the two blocks of body room above must be
                        // clear (an overhanging canopy would embed the head).
                        let room_ok = world::get(wbx + dx, h, wbz + dz) == AIR
                            && world::get(wbx + dx, h + 1, wbz + dz) == AIR;
                        if (30..=40).contains(&h) && biome_ok && ground_ok && room_ok {
                            sx = wbx + dx;
                            sz = wbz + dz;
                            break 'search;
                        }
                    }
                    dx += 1;
                }
                dz += 1;
            }
            r += 1;
        }
        unsafe {
            RESPAWN_BX = sx;
            RESPAWN_BZ = sz;
        }
    }
    // The bed may be a long way from wherever you died, so the ring is still
    // centred there and this column has no terrain yet. Generate it before
    // asking it how tall it is.
    world::ensure_loaded(sx, sz);
    let h = world::surface_y(sx, sz);
    Player {
        x: block_to_world_x(sx) + BLOCK / 2,
        y: h * BLOCK, // feet on the ground surface
        z: block_to_world_z(sz) + BLOCK / 2,
        vy: 0,
        yaw: 0,
        pitch: 0,
        on_ground: true,
        fly: false,
        health: MAX_HEALTH,
        fall_peak: h * BLOCK,
        air: MAX_AIR,
        burn: 0,
        hurt_cd: 0,
        regen_delay: 0,
        regen_tick: 0,
        food: MAX_FOOD,
        food_items: 0,
        exhaustion: 0,
        sprinting: false,
        sneaking: false,
        eff_speed: 0,
        eff_strength: 0,
        eff_regen: 0,
        eff_fire: 0,
        bob: 0,
        hurt_tilt: 0,
        pick: 0,
        axe: 0,
        shovel: 0,
        sword: 0,
        armor: 0,
        selected: DIRT,
        xp: 0,
        efficiency: 0,
        sharpness: 0,
        protection: 0,
        sprint_tap: 0,
        sprint_latch: false,
        was_fwd: false,
    }
}

/// Time of day, weather, terrain light and horizon colour for this frame.
///
/// Pulled out of the gameplay loop rather than left inline: the loop is one
/// enormous function and MIPS branches only reach +/-128KB, so every block that
/// Rain strength, 0 (clear) to 255 (full shower), as a trapezoid over the
/// weather cycle: one window in three rains, and each one fades in and back
/// out over RAIN_RAMP frames. A hard boolean made showers snap on and off
/// mid-stride, which read as a glitch rather than as weather.
fn rain_amount(frame: u32) -> i32 {
    if FORCE_TIME >= 0 {
        return 0; // deterministic captures stay dry
    }
    const WINDOW: u32 = 1200;
    const RAIN_RAMP: u32 = 240; // ~8s in and out at 30fps
    let phase = frame % (WINDOW * 3);
    if phase < WINDOW * 2 {
        return 0;
    }
    let p = phase - WINDOW * 2;
    if p < RAIN_RAMP {
        (p * 255 / RAIN_RAMP) as i32
    } else if p >= WINDOW - RAIN_RAMP {
        ((WINDOW - p) * 255 / RAIN_RAMP) as i32
    } else {
        255
    }
}

/// can live behind a call has to.
#[inline(never)]
/// Time of day, rain, terrain light and sky colour from the world clock.
///
/// `day`, not a frame count: sleeping skips this forward, and the weather and
/// the sun are meant to move with it.
fn world_lighting(day: u32) -> (u32, i32, u8, (u8, u8, u8)) {
    let tod = if FORCE_TIME >= 0 { FORCE_TIME as u32 } else { day % DAY_LEN };
    let rain = rain_amount(day);
    let mut light = day_brightness(tod);
    if rain > 0 {
        // Scaled by the ramp, so the terrain darkens as the shower arrives
        // rather than the instant it starts. Full rain is the old x0.7.
        light = (light as i32 * (2550 - 3 * rain) / 2550) as u8;
    }
    let lf = light as i32 - NIGHT_LIGHT;
    let ld = 128 - NIGHT_LIGHT;
    let mut sky = (
        lerp_u8(SKY_NIGHT.0, SKY_DAY.0, lf, ld),
        lerp_u8(SKY_NIGHT.1, SKY_DAY.1, lf, ld),
        lerp_u8(SKY_NIGHT.2, SKY_DAY.2, lf, ld),
    );
    if rain > 0 {
        // Rain reads as OVERCAST, not as night. The old code dimmed twice
        // (light x0.7 AND sky x0.6), so a midday shower came out darker than
        // real dusk. Scale the cloud deck by the TIME OF DAY, not by `light`:
        // `light` already carries the rain dimming (which belongs on the
        // terrain), while an overcast deck stays bright at noon. The deck
        // then blends in over the ramp, so cloud cover rolls in.
        let d = day_brightness(tod) as i32;
        sky = (
            lerp_u8(sky.0 as i32, (OVERCAST.0 * d / 128) as i32, rain, 255),
            lerp_u8(sky.1 as i32, (OVERCAST.1 * d / 128) as i32, rain, 255),
            lerp_u8(sky.2 as i32, (OVERCAST.2 * d / 128) as i32, rain, 255),
        );
    }
    sky = apply_sunset(sky, tod, rain > 128);
    // The Inferno has no sky: it is a closed cavern lit by its own lava and
    // lumistone. Java fogs it dark red, and without this the roof reads as an
    // overcast afternoon.
    if world::dimension() == world::DIM_INFERNO {
        sky = INFERNO_FOG;
        light = INFERNO_LIGHT;
    } else if world::dimension() == world::DIM_VOID {
        sky = VOID_FOG;
        light = VOID_LIGHT;
    }
    unsafe {
        NIGHT_BIAS = (128 - day_brightness(tod) as i32).clamp(0, 90) as u8;
        SUN_WARMTH = sunset_warmth(tod, rain > 128).clamp(0, 255) as u8;
    }
    (tod, rain, light, sky)
}

/// Start a potion's effect. Java refreshes the timer rather than stacking, so
/// drinking a second speed potion just tops it back up.
fn drink_potion(player: &mut Player, kind: u8) {
    match kind {
        POTION_SPEED => player.eff_speed = POTION_TIME,
        POTION_STRENGTH => player.eff_strength = POTION_TIME,
        POTION_REGEN => player.eff_regen = POTION_TIME,
        POTION_FIRE => player.eff_fire = POTION_TIME,
        _ => {} // awkward: no effect, exactly as in Java
    }
}

/// Which enchant the table would grant next: 1 efficiency, 2 sharpness,
/// 3 protection, 0 when all three are maxed. Round-robin so the levels spread
/// instead of pouring into whichever one the player asks for first.
fn enchant_next(p: &Player) -> u8 {
    let (e, s, pr) = (p.efficiency, p.sharpness, p.protection);
    if e <= s && e <= pr && e < MAX_EFFICIENCY {
        1
    } else if s <= pr && s < MAX_EFFICIENCY {
        2
    } else if pr < MAX_EFFICIENCY {
        3
    } else {
        0
    }
}

/// Standing in portal sheet for long enough swaps dimensions. The world side is
/// a generator switch over the same chunk ring (world::set_dimension); this side
/// owns moving the player and giving them somewhere to stand.
/// True when the player has stood in portal sheet long enough to travel.
#[inline(never)]
fn portal_tick(player: &Player, dwell: &mut u16) -> bool {
    let bx = world_to_block_x(player.x);
    let bz = world_to_block_z(player.z);
    let inside = is_portal(get_block_i32(bx, world_to_block_y(player.y + 8), bz))
        || is_portal(get_block_i32(bx, world_to_block_y(player.y + PLAYER_HEIGHT / 2), bz));
    if !inside {
        *dwell = 0;
        return false;
    }
    // You ARRIVE standing in the return portal, so without this you bounce
    // straight back and ping-pong between dimensions forever. Java does the
    // same: immune until you step out of the sheet.
    if *dwell == PORTAL_IMMUNE {
        return false;
    }
    *dwell += 1;
    if *dwell < PORTAL_DWELL {
        return false;
    }
    *dwell = PORTAL_IMMUNE;
    true
}

/// Do the crossing. Separate from the dwell check because regenerating the ring
/// needs the framebuffer and font for a loading screen.
#[inline(never)]
fn portal_travel(player: &mut Player, fb: &mut FrameBuffer, font: &FontAtlas) {
    let bx = world_to_block_x(player.x);
    let bz = world_to_block_z(player.z);
    // Which sheet you are standing in decides where you go; from anywhere but
    // the overworld, any portal is the way home.
    let sheet = {
        let a = get_block_i32(bx, world_to_block_y(player.y + 8), bz);
        if is_portal(a) {
            a
        } else {
            get_block_i32(bx, world_to_block_y(player.y + PLAYER_HEIGHT / 2), bz)
        }
    };
    let here = world::dimension();
    let to = if here != world::DIM_OVERWORLD {
        world::DIM_OVERWORLD
    } else if sheet == VOID_PORTAL {
        world::DIM_VOID
    } else {
        world::DIM_INFERNO
    };
    // Java's 8:1 scaling: the Inferno is eight times smaller, so a long
    // overworld haul is a short walk down there and back. The End always drops
    // you on its island, near the origin, whatever you came from.
    let (nx, nz) = if to == world::DIM_VOID {
        (0, 0)
    } else if to == world::DIM_INFERNO {
        (bx / 8, bz / 8)
    } else if here == world::DIM_VOID {
        (bx, bz) // the End has no scaling to undo
    } else {
        (bx * 8, bz * 8)
    };
    world::set_dimension(to, nx, nz, |done, total| draw_loading(fb, font, done, total));
    // Land on solid ground, and carve out a return portal so you are never
    // stranded: Java builds one for you too.
    let sy = world::surface_y(nx, nz);
    build_return_portal(nx, sy, nz);
    player.x = block_to_world_x(nx) + BLOCK / 2;
    player.z = block_to_world_z(nz) + BLOCK / 2;
    player.y = sy * BLOCK;
    player.vy = 0;
    player.fall_peak = player.y;
    if to == world::DIM_VOID {
        mob::spawn_dragon(player.x, player.z);
    }
    sfx::splash();
}

/// Obsidian frame + sheet at the arrival point, with the ground under it made
/// solid. Without this you can arrive inside a lava sea or in mid-air.
#[inline(never)]
fn build_return_portal(bx: i32, by: i32, bz: i32) {
    let mut dx = -2i32;
    while dx <= 3 {
        let mut dz = -2i32;
        while dz <= 2 {
            world::set_raw_pub(bx + dx, by - 1, bz + dz, OBSIDIAN); // standing pad
            let mut h = 0;
            while h < 4 {
                world::set_raw_pub(bx + dx, by + h, bz + dz, AIR);
                h += 1;
            }
            dz += 1;
        }
        dx += 1;
    }
    // 2x3 frame along X, sheet inside.
    let mut w = -1i32;
    while w <= 2 {
        world::set_raw_pub(bx + w, by + 3, bz, OBSIDIAN);
        w += 1;
    }
    let mut h = 0i32;
    while h < 3 {
        world::set_raw_pub(bx - 1, by + h, bz, OBSIDIAN);
        world::set_raw_pub(bx + 2, by + h, bz, OBSIDIAN);
        world::set_raw_pub(bx, by + h, bz, PORTAL);
        world::set_raw_pub(bx + 1, by + h, bz, PORTAL);
        h += 1;
    }
    world::remesh_loaded();
}

/// Environmental survival: lava + drowning damage (with i-frames via
/// regen_delay), and passive health regen when unhurt for a while.
#[inline(never)]
fn update_survival(player: &mut Player) {
    let bx = world_to_block_x(player.x);
    let bz = world_to_block_z(player.z);
    let feet = world::get(bx, world_to_block_y(player.y + 8), bz);
    let mid = world::get(bx, world_to_block_y(player.y + PLAYER_HEIGHT / 2), bz);
    let head = world::get(bx, world_to_block_y(player.y + EYE_HEIGHT), bz);

    if player.hurt_cd > 0 {
        player.hurt_cd -= 1;
    }

    // Per-hazard damage + cadence at 30fps, matching Java rates.
    let mut hurt = 0;
    let mut cd = 15;
    if is_lava(feet) || is_lava(mid) || feet == FIRE || mid == FIRE {
        hurt = 4; // Java: 4 hp per 0.5s in lava
        cd = 15;
        player.burn = FIRE_DURATION; // and catch fire
    }
    // Cactus pricks: standing against one (any cardinal neighbour at feet or
    // mid height) costs 1 hp per half second, like Java contact damage.
    if hurt == 0 {
        let fy = world_to_block_y(player.y + 8);
        let my = world_to_block_y(player.y + PLAYER_HEIGHT / 2);
        if world::get(bx + 1, fy, bz) == CACTUS
            || world::get(bx - 1, fy, bz) == CACTUS
            || world::get(bx, fy, bz + 1) == CACTUS
            || world::get(bx, fy, bz - 1) == CACTUS
            || world::get(bx + 1, my, bz) == CACTUS
            || world::get(bx - 1, my, bz) == CACTUS
            || world::get(bx, my, bz + 1) == CACTUS
            || world::get(bx, my, bz - 1) == CACTUS
        {
            hurt = 1;
            cd = 15;
        }
    }
    if is_water(head) {
        if player.air > 0 {
            player.air -= 1;
        } else if hurt < 2 {
            hurt = 2; // out of air -> drown 2 hp per 1s
            cd = 30;
        }
        player.burn = 0; // water douses fire
    } else {
        player.air = MAX_AIR;
    }
    // Potion timers run down here, once a frame like every other survival clock.
    if player.eff_speed > 0 {
        player.eff_speed -= 1;
    }
    if player.eff_strength > 0 {
        player.eff_strength -= 1;
    }
    if player.eff_regen > 0 {
        player.eff_regen -= 1;
    }
    if player.eff_fire > 0 {
        player.eff_fire -= 1;
        // Fire resistance: no lava/fire damage, and nothing keeps burning.
        player.burn = 0;
        if hurt == 4 {
            hurt = 0;
        }
    }
    if player.burn > 0 {
        player.burn -= 1;
        if hurt == 0 {
            hurt = 1; // burning: 1 hp per 1s even after leaving lava
            cd = 30;
        }
    }

    if hurt > 0 && player.hurt_cd == 0 {
        player.health -= hurt;
        player.hurt_cd = cd;
        player.regen_delay = REGEN_DELAY;
    }

    // Hunger drains over time; auto-eat carried food (mob drops) when hungry.
    player.exhaustion += if player.sprinting { 3 } else { 1 };
    if player.exhaustion >= FOOD_DRAIN {
        player.exhaustion = 0;
        if player.food > 0 {
            player.food -= 1;
        }
    }
    // Auto-eat, best food first: steak +6, bread +5, fish +4, raw meat +3.
    if player.food <= MAX_FOOD - 6 && unsafe { INV[COOKED_MEAT as usize] } > 0 {
        unsafe {
            INV[COOKED_MEAT as usize] -= 1;
        }
        player.food = (player.food + 6).min(MAX_FOOD);
        sfx::eat();
    } else if player.food <= MAX_FOOD - 5 && unsafe { INV[BREAD as usize] } > 0 {
        unsafe {
            INV[BREAD as usize] -= 1;
        }
        player.food = (player.food + 5).min(MAX_FOOD);
        sfx::eat();
    } else if player.food <= MAX_FOOD - 4 && player.food_items > 0 {
        player.food = (player.food + 4).min(MAX_FOOD);
        player.food_items -= 1;
        sfx::eat();
    } else if player.food <= MAX_FOOD - 3 && unsafe { INV[RAW_MEAT as usize] } > 0 {
        // Raw meat is the last resort; cook it for double the value.
        unsafe {
            INV[RAW_MEAT as usize] -= 1;
        }
        player.food = (player.food + 3).min(MAX_FOOD);
        sfx::eat();
    }

    // Regeneration potion heals regardless of food or recent damage, which is
    // the whole point of carrying one into a fight.
    if player.eff_regen > 0 && player.health < MAX_HEALTH && player.eff_regen % 15 == 0 {
        player.health += 1;
    }
    // Regen only when fed and unhurt; starve (to 1 hp) when out of food.
    if player.regen_delay > 0 {
        player.regen_delay -= 1;
    } else if player.food == 0 {
        player.regen_tick += 1;
        if player.regen_tick >= STARVE_PERIOD {
            if player.health > 1 {
                player.health -= 1;
            }
            player.regen_tick = 0;
        }
    } else if player.health < MAX_HEALTH && player.food >= REGEN_FOOD_MIN {
        player.regen_tick += 1;
        // Fast regen when well fed (Java: food==20 & saturation>0), else slow.
        let period = if player.food == MAX_FOOD { REGEN_FAST } else { REGEN_PERIOD };
        if player.regen_tick >= period {
            player.health += 1;
            player.food -= 1; // healing burns food (Java: ~1.5 food/hp via exhaustion)
            player.regen_tick = 0;
        }
    }
}

#[inline(never)]
fn camera_from_player(p: Player) -> Camera {
    let pitch_angle = (p.pitch as i32 & 0x0FFF) as u16;
    // Two peaks per stride: the vertical bob runs at twice the sway's rate, as
    // in Java. 26 world units of travel is roughly one step.
    let ph = ((p.bob / 3) & 0x0FFF) as u16;
    let sway = sincos::sin_q12(ph); // Q12, -4096..4096
    let lift = sincos::sin_q12(ph.wrapping_mul(2) & 0x0FFF);
    // 3 degrees of roll is 34 Q12 units; the vertical throw is ~3 world units.
    let roll = (sway * 34) >> 12;
    let bob_y = (lift * 3) >> 12;
    // Hurt tilt decays over its countdown and leans much harder than the bob.
    let hurt = (p.hurt_tilt as i32 * 160) / HURT_TILT_FRAMES as i32;
    Camera {
        x: p.x,
        // Sneaking drops the eye, the way Java does. Without it there is no
        // on-screen sign that sneak is engaged at all.
        y: p.y + if p.sneaking { EYE_HEIGHT - 8 } else { EYE_HEIGHT } + bob_y,
        z: p.z,
        sy: sincos::sin_q12(p.yaw),
        cy: sincos::cos_q12(p.yaw),
        sp: sincos::sin_q12(pitch_angle),
        cp: sincos::cos_q12(pitch_angle),
        roll: roll + hurt,
    }
}

/// Frames a hurt tilt takes to decay. Java's is 10 ticks.
const HURT_TILT_FRAMES: u8 = 12;

/// inline(never) across the gameplay loop's big callees is load-bearing, not
/// taste: the loop is one enormous function and MIPS conditional branches only
/// reach +/-128KB. Letting these inline into it overflows that and the LINK
/// fails with "out of range PC16 fixup" -- which is what happens the moment
/// anything new is added to the loop. Each runs once a frame, so the call ABI
/// is free.
#[inline(never)]
fn update_player(
    player: &mut Player,
    pad: ButtonState,
    previous: ButtonState,
    left: (i16, i16),
    right: (i16, i16),
) {
    let action_map = unsafe { SETTINGS_PROFILE.actions };
    let current = PadState {
        buttons: pad,
        ..PadState::NONE
    };
    let prior = PadState {
        buttons: previous,
        ..PadState::NONE
    };
    let actions = action_map.input(current, prior);

    // --- look: RIGHT stick = camera (yaw + pitch), proportional analog ---
    //
    // Radial, through psx-pad. This used to gate each axis on its own, which
    // makes the dead region a square: a stick pushed gently along a diagonal
    // clears the threshold on one axis and not the other, so the look snapped
    // to a cardinal instead of going where it was pushed. A stick's centre
    // drift is radial, so the region that ignores it has to be a circle.
    let (rx, ry) = Deadzone::new(unsafe { SET_LOOK_DZ })
        .scaled(right.0, right.1)
        .map_or((0, 0), |(x, y)| (x as i32, y as i32));
    // Response CURVE, not a flat divisor: a gentle linear base keeps aim precise
    // near centre (placing blocks with a stick), plus a quadratic term so a full
    // push turns quickly. Tops out ~90°/s yaw / ~75°/s pitch (was a flat ~60/50,
    // sluggish). Feel knob -- tune the divisors on hardware.
    // Turning is EXCLUSIVELY on the right stick -- the left stick and d-pad only
    // ever move/strafe, never yaw.
    let lp = unsafe { SET_LOOK_PCT };
    let yaw_delta = (rx / 6 + rx * rx.abs() / 850) * lp / 100;
    let mut pitch_delta = -(ry / 7 + ry * ry.abs() / 1000) * lp / 100;
    if unsafe { SET_INVERT_Y } {
        pitch_delta = -pitch_delta;
    }
    player.yaw = ((player.yaw as i32 + yaw_delta) & 0x0FFF) as u16;
    // Clamp to +-90 degrees (1024 = 90 deg in the 4096 = 360 deg system) so you
    // can look straight up/down like Java -- was +-760 (~67 deg), which stopped
    // short of your own feet.
    player.pitch = (player.pitch as i32 + pitch_delta).clamp(-1024, 1024) as i16;

    // Creative fly toggle: SELECT, or double-tap CROSS (jump) like Minecraft
    // creative -- one button, since we're in creative anyway. First CROSS arms a
    // short window; a second CROSS inside it flips fly. Held CROSS (fly ascend)
    // never re-triggers because `pressed` is edge-only.
    // Fly is toggled ONLY from the OPTIONS menu. The double-tap-CROSS toggle
    // is gone for cause: closing a menu with CROSS and then jumping counted
    // as the double tap, silently launching the player into fly mode. SELECT
    // stays wired in demo builds only -- the headless DEMO_MARCH scripts
    // depend on it.
    let toggle_fly = DEMO_PLAY && pressed(pad, previous, button::SELECT);
    if toggle_fly {
        player.fly = !player.fly;
        player.vy = 0;
    }

    // --- move: LEFT stick = character (forward/back + strafe), camera-relative analog ---
    let (lx, ly) = Deadzone::new(unsafe { SET_MOVE_DZ })
        .scaled(left.0, left.1)
        .map_or((0, 0), |(x, y)| (x as i32, y as i32));
    let mut strafe = lx / 11;
    let mut forward = -ly / 11;
    if lx == 0 && ly == 0 {
        // D-pad as a movement fallback when the stick is idle: UP/DOWN walk,
        // LEFT/RIGHT strafe (turning stays on the right stick only).
        if actions.held(ACT_FORWARD) {
            forward += WALK_SPEED;
        }
        if actions.held(ACT_BACK) {
            forward -= WALK_SPEED;
        }
        if actions.held(ACT_LEFT) {
            strafe -= WALK_SPEED;
        }
        if actions.held(ACT_RIGHT) {
            strafe += WALK_SPEED;
        }
    }

    // Sneak: hold CIRCLE (the Bedrock PS4/PS5 default; it doubles as fly-down
    // while flying, exactly as on console). Java walks you at ~30% speed and
    // refuses to step off a ledge; it beats sprint, so you cannot sprint-sneak.
    let sneaking = actions.held(ACT_SNEAK) && !player.fly;
    player.sneaking = sneaking;
    // Sprint (Bedrock): press L3, or push the stick forward twice inside the
    // double-tap window. The latch drops the moment forward input stops, so a
    // sprint is one gesture and then just steering -- no held button fighting
    // the look stick. 1.3x speed (Java 5.612 vs 4.317 b/s), extra hunger
    // (exhaustion accrues 3x, see update_survival).
    let fwd_now = forward > 0;
    if fwd_now && !player.was_fwd {
        if player.sprint_tap > 0 {
            player.sprint_latch = true;
        }
        player.sprint_tap = DOUBLE_TAP_FRAMES;
    } else {
        player.sprint_tap = player.sprint_tap.saturating_sub(1);
    }
    player.was_fwd = fwd_now;
    if pressed(pad, previous, button::L3) {
        player.sprint_latch = true;
    }
    if !fwd_now {
        player.sprint_latch = false;
    }
    let sprinting = !sneaking && player.sprint_latch && fwd_now;
    if player.eff_speed > 0 {
        forward += forward * 3 / 10; // Java speed I: +20%; a touch more here
        strafe += strafe * 3 / 10;
    }
    if sprinting {
        forward += forward * 3 / 10;
    } else if sneaking {
        forward = forward * 3 / 10;
        strafe = strafe * 3 / 10;
    }
    player.sprinting = sprinting;

    let sy = sincos::sin_q12(player.yaw);
    let cy = sincos::cos_q12(player.yaw);
    let dx = ((sy * forward) + (cy * strafe)) >> 12;
    let dz = ((cy * forward) - (sy * strafe)) >> 12;

    if player.fly {
        // Fly along the FULL look vector, pitch included. dx/dz above are built
        // from yaw only, which is right for walking but made flying feel like
        // walking in the air: looking up and pushing forward just moved you
        // horizontally. This is the same forward vector trace_pick uses for the
        // pick ray, so where you aim is now where you travel.
        let pa = (player.pitch as i32 & 0x0FFF) as u16;
        let sp = sincos::sin_q12(pa);
        let cp = sincos::cos_q12(pa);
        let fx = (sy * cp) >> 12;
        let fz = (cy * cp) >> 12;
        // Forward takes the pitched vector; strafe stays horizontal, as it should.
        player.x += ((fx * forward) >> 12) + ((cy * strafe) >> 12);
        player.z += ((fz * forward) >> 12) - ((sy * strafe) >> 12);
        player.y += (sp * forward) >> 12;
        if pad.is_held(button::CROSS) {
            player.y += FLY_SPEED;
        }
        if pad.is_held(button::CIRCLE) {
            player.y -= FLY_SPEED;
        }
        // Keep it inside the world column rather than letting it drift out of
        // the chunk's y range, where every lookup reads AIR.
        player.y = player.y.clamp(BLOCK, (world::CH - 2) * BLOCK);
        player.on_ground = false;
    } else {
        let was_ground = player.on_ground;
        // Bob phase advances with GROUND distance covered, so it freezes the
        // instant you stop and speeds up when you sprint, for free.
        if was_ground {
            player.bob = player.bob.wrapping_add((dx.abs() + dz.abs()) as u32);
        }
        if player.hurt_tilt > 0 {
            player.hurt_tilt -= 1;
        }
        // Collide-and-slide, axis-separated: undo whichever axis hits a wall so
        // the other still moves (lets the player slide along faces).
        //
        // Each axis also refuses to leave the GENERATED region. world::get answers
        // AIR both for "this is empty" and for "this is not streamed in yet", so
        // walking past the edge of the loaded ring used to drop you through ground
        // that did not exist yet -- you fell to the bottom of the world. Now you
        // stop at the boundary and streaming catches up.
        //
        // Checked HERE, once per axis, and deliberately not inside
        // aabb_collides_dims: mob physics calls that dozens of times a frame, and
        // putting the same test there took the frame from 760K to 1,570K cycles
        // and two thirds of the frame rate.
        // column_loaded takes BLOCK coords; player.x/z are world units (64 per
        // block). Passing them raw computed a chunk coord 64x too large, the
        // lookup missed for every real position, and both axes rolled back every
        // step -- the player could not walk at all (fly skips this branch, which
        // is why flying still worked).
        player.x += dx;
        if !world::column_loaded(world_to_block_x(player.x), world_to_block_z(player.z))
            || aabb_collides(player.x, player.y, player.z)
            || (sneaking && was_ground && !supported(player.x, player.y, player.z))
        {
            player.x -= dx;
        }
        player.z += dz;
        if !world::column_loaded(world_to_block_x(player.x), world_to_block_z(player.z))
            || aabb_collides(player.x, player.y, player.z)
            || (sneaking && was_ground && !supported(player.x, player.y, player.z))
        {
            player.z -= dz;
        }

        // Ladders override gravity: press toward your look (forward) to climb up,
        // back to descend, otherwise slide down slowly.
        if at_ladder(player.x, player.y, player.z) {
            player.vy = if forward > 0 {
                10
            } else if forward < 0 {
                -10
            } else {
                -3
            };
        } else if in_water_body(player.x, player.y, player.z) && player.vy <= SWIM_UP {
            // Swimming: buoyancy fights gravity. Hold CROSS to stroke upward,
            // otherwise a slow sink; both clamped well under air speeds so
            // water feels thick and a dive decelerates on entry. The vy gate
            // above lets a jump OUT of the water stay ballistic instead of
            // being clamped back to stroke speed on its first frame.
            player.vy = if pad.is_held(button::CROSS) {
                (player.vy + 4).clamp(-6, SWIM_UP)
            } else {
                (player.vy - 1).clamp(-6, SWIM_UP)
            };
            player.fall_peak = player.y; // water breaks the fall, as in Java
        } else {
            player.vy = (player.vy - GRAVITY).max(TERMINAL_VY);
        }
        // The horizontal guard above stops you WALKING off the generated ring,
        // but nothing stopped you falling through it. A teleport puts you
        // there without walking -- respawning at a bed the ring has not reached
        // yet -- and world::get answers AIR for those columns, so there was no
        // floor to land on and the terrain generated on top of you a second
        // later. That is the "appeared underground when respawned" report.
        if !world::column_loaded(world_to_block_x(player.x), world_to_block_z(player.z)) {
            player.vy = 0;
            player.fall_peak = player.y;
        }
        let ny = player.y + player.vy;
        if aabb_collides(player.x, ny, player.z) {
            if player.vy < 0 {
                // Landed: snap feet to the top surface of the block below. A
                // slab's surface is half a block up, not at the block boundary
                // -- snapping to the boundary put the feet back ABOVE the slab,
                // so the fall restarted and you bounced on it forever.
                let by = world_to_block_y(ny);
                let bx = world_to_block_x(player.x);
                let bz = world_to_block_z(player.z);
                let under = get_block_i32(bx, by, bz);
                player.y = if under == SLAB {
                    by * BLOCK + BLOCK / 2
                } else if is_stairs(under) {
                    let (sx0, _, sz0, sx1, _, sz1) = stair_step_box(under, bx, by, bz);
                    if player.x > sx0 && player.x < sx1 && player.z > sz0 && player.z < sz1 {
                        (by + 1) * BLOCK // standing on the high step
                    } else {
                        by * BLOCK + BLOCK / 2
                    }
                } else {
                    (by + 1) * BLOCK
                };
                player.on_ground = true;
            } else {
                // Head bonk: snap so the head sits just under the block above.
                let by = world_to_block_y(ny + PLAYER_HEIGHT);
                player.y = by * BLOCK - PLAYER_HEIGHT;
            }
            player.vy = 0;
        } else {
            player.y = ny;
            player.on_ground = false;
        }

        // Fall damage: on landing, hurt for blocks fallen past the safe margin.
        if player.on_ground {
            if !was_ground {
                let blocks = (player.fall_peak - player.y) / BLOCK;
                if blocks > SAFE_FALL_BLOCKS {
                    player.health -= blocks - SAFE_FALL_BLOCKS;
                    player.regen_delay = REGEN_DELAY;
                }
            }
            player.fall_peak = player.y;
        } else if player.y > player.fall_peak {
            player.fall_peak = player.y;
        }

        // Jumping works from the ground OR from the water, so you can hop a
        // shore instead of bobbing against it.
        if (player.on_ground || in_water_body(player.x, player.y, player.z))
            && pressed(pad, previous, button::CROSS)
        {
            player.vy = JUMP_VY;
            player.on_ground = false;
            unsafe { TUT_JUMPED = true };
        }
    }

    // Water-entry splash and footsteps, both from the frame's FINAL position
    // (attempted deltas lie when a wall eats the move).
    unsafe {
        let wet = in_water_body(player.x, player.y, player.z);
        if wet && !WAS_WET && player.vy <= -4 {
            sfx::splash();
        }
        WAS_WET = wet;
        let moved = (player.x - STEP_PX).abs() + (player.z - STEP_PZ).abs();
        STEP_PX = player.x;
        STEP_PZ = player.z;
        if player.on_ground && !wet && moved > 0 {
            STEP_ACC += moved;
            if STEP_ACC > 52 {
                // ~0.8 blocks per step, voiced by the block underfoot.
                STEP_ACC = 0;
                STEP_N = STEP_N.wrapping_add(1);
                let under = get_block_i32(
                    world_to_block_x(player.x),
                    world_to_block_y(player.y - 2),
                    world_to_block_z(player.z),
                );
                sfx::step_on(step_mat(under), STEP_N);
            }
        }
    }

    // Infinite world: no horizontal bounds. Bedrock at y=0 stops the fall; this
    // is just a guard against runaway values.
    player.y = player.y.clamp(-2 * BLOCK, (world::CH + 6) * BLOCK);
}

// Footstep state: cadence accumulator and last position, plus the wet edge
// detector for the entry splash.
static mut STEP_ACC: i32 = 0;
static mut STEP_N: u32 = 0;
static mut STEP_PX: i32 = 0;
static mut STEP_PZ: i32 = 0;
static mut WAS_WET: bool = false;

/// Which footstep voice a block underfoot gets.
fn step_mat(b: u8) -> u32 {
    match b {
        STONE | COBBLE | BRICK | OBSIDIAN | FURNACE | CINDERSTONE | VOID_STONE | SLAB => 1,
        SAND | SINK_SAND | SNOW => 2,
        WOOD | PLANK | FENCE | CHEST | CRAFT_TABLE => 3,
        _ => 0,
    }
}

/// True if the player's AABB overlaps any solid block.
fn aabb_collides(cx: i32, feet: i32, cz: i32) -> bool {
    aabb_collides_dims(cx, feet, cz, PLAYER_HALF_W, PLAYER_HEIGHT)
}

/// Java refuses to place a block that would occupy your own body, and so do
/// we now. Without this, looking straight down (exactly what hold-to-dig
/// does) puts the pick on the block underfoot, so the place cell is YOUR OWN
/// feet -- and a second place is your head. Two taps entombed the camera
/// inside solid blocks, where backface culling opens the world into a
/// see-through of caves and sky that a long investigation chased as terrain
/// corruption. Walk-through blocks stay placeable at your feet, as in Java.
fn place_intersects_player(player: &Player, bx: i32, by: i32, bz: i32, b: u8) -> bool {
    let passable = b == LADDER
        || b == DOOR_O
        || world::is_cross_plant(b)
        || is_water(b)
        || is_lava(b);
    if passable {
        return false;
    }
    let x0 = block_to_world_x(bx);
    let z0 = block_to_world_z(bz);
    let y0 = by * BLOCK;
    player.x + PLAYER_HALF_W > x0
        && player.x - PLAYER_HALF_W < x0 + BLOCK
        && player.y + PLAYER_HEIGHT > y0
        && player.y < y0 + BLOCK
        && player.z + PLAYER_HALF_W > z0
        && player.z - PLAYER_HALF_W < z0 + BLOCK
}

/// True if there is ground immediately under the player's box. Sneak uses this
/// to refuse a step that would walk you off a ledge: the box dropped two units
/// still overlaps whatever you are standing on, including a slab or a stair's
/// lower half, but finds nothing over a drop.
fn supported(cx: i32, feet: i32, cz: i32) -> bool {
    aabb_collides(cx, feet - 2, cz)
}

/// True if the player's body (waist or chest) is immersed in water -- feet
/// alone in a shallow film do not count, so beaches stay walkable.
fn in_water_body(cx: i32, feet: i32, cz: i32) -> bool {
    let bx = world_to_block_x(cx);
    let bz = world_to_block_z(cz);
    // EITHER feet or chest, not both. Requiring both meant you settled with
    // your chest under the surface, which parks your feet a full block BELOW
    // the waterline -- under the lip of any shore you were trying to reach,
    // so you could never climb out. Floating by the feet puts them level with
    // the water top, which is also the top of the adjacent land block.
    is_water(get_block_i32(bx, world_to_block_y(feet + 8), bz))
        || is_water(get_block_i32(bx, world_to_block_y(feet + PLAYER_HEIGHT / 2), bz))
}

/// True if the player is standing in a ladder block (enables climbing).
fn at_ladder(cx: i32, feet: i32, cz: i32) -> bool {
    let bx = world_to_block_x(cx);
    let bz = world_to_block_z(cz);
    get_block_i32(bx, world_to_block_y(feet + 8), bz) == LADDER
        || get_block_i32(bx, world_to_block_y(feet + PLAYER_HEIGHT - 8), bz) == LADDER
}

/// True if an AABB at (centre x, feet y, centre z) with the given half-width and
/// height overlaps any solid block. Water and lava are non-solid (you sink).
pub fn aabb_collides_dims(cx: i32, feet: i32, cz: i32, halfw: i32, height: i32) -> bool {
    let bx0 = world_to_block_x(cx - halfw);
    let bx1 = world_to_block_x(cx + halfw);
    let by0 = world_to_block_y(feet);
    let by1 = world_to_block_y(feet + height);
    let bz0 = world_to_block_z(cz - halfw);
    let bz1 = world_to_block_z(cz + halfw);
    let mut bx = bx0;
    while bx <= bx1 {
        let mut by = by0;
        while by <= by1 {
            let mut bz = bz0;
            while bz <= bz1 {
                let b = get_block_i32(bx, by, bz);
                if b == SLAB || is_stairs(b) {
                    // Only the bottom half is solid, so you step onto a slab at
                    // half a block and can walk under one placed overhead.
                    if feet < by * BLOCK + BLOCK / 2 && feet + height > by * BLOCK {
                        return true;
                    }
                    // A stair adds an upper step over half its footprint.
                    if is_stairs(b) && feet < (by + 1) * BLOCK && feet + height > by * BLOCK + BLOCK / 2
                    {
                        let (sx0, _, sz0, sx1, _, sz1) = stair_step_box(b, bx, by, bz);
                        if cx + halfw > sx0 && cx - halfw < sx1 && cz + halfw > sz0 && cz - halfw < sz1
                        {
                            return true;
                        }
                    }
                } else if b != AIR
                    && !is_water(b)
                    && !is_lava(b)
                    && b != LADDER
                    && b != DOOR_O
                    && !world::is_cross_plant(b)
                {
                    return true; // cross plants/ladders/open doors are walk-through
                }
                bz += 1;
            }
            by += 1;
        }
        bx += 1;
    }
    false
}

/// Caller must have run `ui_frame_begin` (which clears the table) first.
#[inline(never)]
fn render_world(cam: &Camera) -> (usize, usize) {
    let mut count = 0usize;
    if PROFILE_SKIP_WORLD {
        return (0, 0);
    }
    // world::for_visible_faces does the distance + backface cull inline; the
    // closure only fires for faces that will project.
    unsafe {
        AO_N = 0;
        NEAR_TRI_N = 0;
    }
    telemetry::stage_begin(ST_OTCLEAR); // TEMP: whole face-loop span
    let face_work = world::for_visible_faces(
        cam,
        |oxw, ozw| gte_begin_chunk(cam, oxw, ozw),
        |block, lx, wy, lz, dir, w, h, light, ao| {
            emit_face(block, lx, wy, lz, dir, w, h, light, ao, &mut count);
        },
    );
    render_near_block_shell(cam);
    telemetry::stage_end(ST_OTCLEAR);
    let near_tris = unsafe { NEAR_TRI_N };
    (count + near_tris, face_work)
}

/// Mob colour by kind (modulated per-face + by day/night in emit_box).
fn mob_color(kind: u8) -> (u8, u8, u8) {
    match kind {
        mob::PIG => (236, 150, 160),
        mob::COW => (96, 64, 42),
        mob::SHEEP => (220, 220, 214),
        mob::CHICKEN => (236, 232, 200),
        mob::ZOMBIE => (66, 132, 74),
        mob::SKELETON => (200, 200, 204),
        mob::SAPPER => (54, 168, 64),
        mob::WRAITH => (18, 18, 24), // near-black, with the purple eyes below
        mob::EMBER => (240, 176, 40),
        mob::WAILER => (222, 222, 218),
        mob::CHARRED_SK => (44, 46, 42),
        mob::WOLF => (206, 202, 196),
        mob::VILLAGER => (140, 106, 78),
        _ => (44, 44, 50), // spider
    }
}

/// Emit the visible faces of an axis-aligned box as flat-shaded quads into the OT.
fn emit_box(cam: &Camera, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32, base: (u8, u8, u8), count: &mut usize) {
    emit_box_masked(cam, x0, y0, z0, x1, y1, z1, base, count, 0x3F);
}

/// emit_box with a direction bitmask (bit d = draw dir d); mob heads mask out
/// the +Z front (bit 4) because it wears a textured FACE quad instead.
#[allow(clippy::too_many_arguments)]
fn emit_box_masked(cam: &Camera, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32, base: (u8, u8, u8), count: &mut usize, mask: u8) {
    let mut dir = 0;
    while dir < 6 {
        if mask & (1 << dir) == 0 {
            dir += 1;
            continue;
        }
        let facing = match dir {
            0 => cam.x > x1 - 8,
            1 => cam.x < x0 + 8,
            2 => cam.y > y1 - 8,
            3 => cam.y < y0 + 8,
            4 => cam.z > z1 - 8,
            _ => cam.z < z0 + 8,
        };
        if facing && *count < MAX_MOB_QUADS {
            let verts = match dir {
                0 => [(x1, y1, z0), (x1, y1, z1), (x1, y0, z0), (x1, y0, z1)],
                1 => [(x0, y1, z1), (x0, y1, z0), (x0, y0, z1), (x0, y0, z0)],
                2 => [(x0, y1, z0), (x1, y1, z0), (x0, y1, z1), (x1, y1, z1)],
                3 => [(x0, y0, z1), (x1, y0, z1), (x0, y0, z0), (x1, y0, z0)],
                4 => [(x1, y1, z1), (x0, y1, z1), (x1, y0, z1), (x0, y0, z1)],
                _ => [(x0, y1, z0), (x1, y1, z0), (x0, y0, z0), (x1, y0, z0)],
            };
            let Some([p0, p1, p2, p3]) = project_quad_gte(cam, &verts) else {
                dir += 1;
                continue;
            };
            // Mobs beyond the far plane don't draw (matches the old per-vertex cull).
            if (p0.z + p1.z + p2.z + p3.z) >> 2 < FAR_Z {
                let (sr, _, _) = face_tint(dir); // shade scalar (incl. day/night)
                let s = sr as u32;
                let col = (
                    (base.0 as u32 * s / 128) as u8,
                    (base.1 as u32 * s / 128) as u8,
                    (base.2 as u32 * s / 128) as u8,
                );
                let depth = (p0.z + p1.z + p2.z + p3.z) >> 2;
                let slot = depth_slot(depth);
                let packet = QuadFlat::new(
                    [(p0.x, p0.y), (p1.x, p1.y), (p2.x, p2.y), (p3.x, p3.y)],
                    col.0,
                    col.1,
                    col.2,
                );
                unsafe {
                    MOB_QUADS[RENDER_ARENA][*count] = packet;
                    OT[RENDER_ARENA].insert(
                        slot,
                        &mut MOB_QUADS[RENDER_ARENA][*count] as *mut QuadFlat as *mut u32,
                        QuadFlat::WORDS,
                    );
                }
                *count += 1;
            }
        }
        dir += 1;
    }
}

fn scale_rgb(c: (u8, u8, u8), pct: u32) -> (u8, u8, u8) {
    (
        (c.0 as u32 * pct / 100).min(255) as u8,
        (c.1 as u32 * pct / 100).min(255) as u8,
        (c.2 as u32 * pct / 100).min(255) as u8,
    )
}

/// First-person "held item": a small isometric block in the bottom-right,
/// coloured by the selected block, bobbing gently. ponytail: flat-shaded faux-3D
/// cube (no texture), three shaded faces -- reads clearly as "you hold this".
/// One textured face of the held-item cube, shaded flat (128 = full-bright).
fn held_face(verts: [(i16, i16); 4], tile: u8, bt: BlockTex, shade: u8) {
    let (u, v) = tex::tile_uv(tile);
    let win = TextureWindow::power_of_two_tile(u, v, 16, 16);
    let mat =
        TextureMaterial::opaque(bt.clut[tile as usize], bt.tpage, (shade, shade, shade)).with_texture_window(win);
    ui_quad_textured(verts, [(0, 0), (16, 0), (0, 16), (16, 16)], mat);
}

/// First-person held item: a TEXTURED faux-3D cube in the bottom-right using the
/// selected block's real tiles, with an idle bob and a downward dip while
/// swinging (mining/placing).
#[inline(never)]
fn draw_held_item(selected: u8, tool: (u8, u8), frame: u32, swing: i32) {
    let bob = (sincos::sin_q12(((frame * 36) & 0x0FFF) as u16) >> 9) as i16; // ~-8..8
    let dip = (swing * 3) as i16; // dips down + nudges right on a swing
    let cx = 272i16 + (swing / 2) as i16;
    let cyt = 176i16 + bob + dip; // centre of the top face
    let r = 22i16;
    let h = 22i16;
    // The ARM holding the block (classic first-person look): a blocky
    // skin-tone forearm slanting in from the bottom-right screen corner to a
    // lit fist under the block; drawn first so the block sits on the hand.
    let (wx, wy) = (cx + 6, cyt + h - 2); // wrist, tucked behind the block
    let al = unsafe { LIGHT } as u32;
    let arm = |c: u32| ((c * (48 + al * 80 / 128)) / 128).clamp(20, 255) as u8;
    // The arm used to be two flat skin quads meeting at a hard diagonal with no
    // sleeve and no outline, which a blind comparison called "a flat untextured
    // skin wedge". It is now a sleeve, a forearm with a lit top edge, and a fist
    // with a knuckle break -- still flat-shaded, but it reads as an arm.
    // Sleeve first, so the forearm overlaps it at the cuff.
    ui_quad_flat(
        [(wx + 16, wy + 30), (wx + 50, wy + 20), (wx + 54, wy + 78), (wx + 88, wy + 58)],
        arm(96),
        arm(126),
        arm(178),
    ); // sleeve (a blue-grey shirt, so the limb has a silhouette break)
    ui_quad_flat(
        [(wx + 14, wy + 31), (wx + 34, wy + 25), (wx + 36, wy + 55), (wx + 56, wy + 49)],
        arm(70),
        arm(94),
        arm(136),
    ); // cuff shadow, sells the sleeve edge
    ui_quad_flat(
        [(wx - 8, wy + 8), (wx + 26, wy - 2), (wx + 26, wy + 66), (wx + 60, wy + 44)],
        arm(186),
        arm(138),
        arm(106),
    ); // forearm (shaded skin)
    ui_quad_flat(
        [(wx - 8, wy + 8), (wx + 26, wy - 2), (wx + 22, wy + 22), (wx + 56, wy + 12)],
        arm(214),
        arm(164),
        arm(128),
    ); // lit upper edge of the forearm, so it is not one flat plane
    ui_quad_flat(
        [(wx - 12, wy - 10), (wx + 22, wy - 18), (wx - 4, wy + 12), (wx + 30, wy + 2)],
        arm(212),
        arm(160),
        arm(124),
    ); // fist (lit skin)
    ui_quad_flat(
        [(wx - 10, wy - 2), (wx + 20, wy - 9), (wx - 8, wy + 3), (wx + 22, wy - 4)],
        arm(150),
        arm(108),
        arm(84),
    ); // knuckle break across the fist
    if selected == AIR {
        // Empty hand: the equipped tool rides in the fist. Mirrored so the
        // handle meets the hand and the head points up-left, tier-tinted like
        // the HUD slot, lit like the arm.
        if tool.1 > 0 {
            let base = tool_tint(tool.1);
            let (tr, tg, tb2) = (base.0 as u32, base.1 as u32, base.2 as u32);
            let l = unsafe { LIGHT } as u32;
            let f = 48 + l * 80 / 128;
            let tint = (
                ((tr * f) / 128).clamp(16, 255) as u8,
                ((tg * f) / 128).clamp(16, 255) as u8,
                ((tb2 * f) / 128).clamp(16, 255) as u8,
            );
            let tile = tool_tile(tool.0);
            let (u, v) = tex::tile_uv(tile);
            let win = TextureWindow::power_of_two_tile(u, v, 16, 16);
            let btx = unsafe { BLOCK_TEX };
            let mat = TextureMaterial::opaque(btx.clut[tile as usize], btx.tpage, tint)
                .with_texture_window(win);
            ui_quad_textured(
                [(wx - 58, wy - 66), (wx + 8, wy - 52), (wx - 44, wy - 4), (wx + 22, wy + 10)],
                [(16, 0), (0, 0), (16, 16), (0, 16)],
                mat,
            );
        }
        return; // otherwise: just the arm, like the original
    }
    let t = (cx, cyt - r / 2);
    let rt = (cx + r, cyt);
    let lf = (cx - r, cyt);
    let b = (cx, cyt + r / 2);
    let bl = (cx, cyt + r / 2 + h);
    let ll = (cx - r, cyt + h);
    let rl = (cx + r, cyt + h);
    let bt = unsafe { BLOCK_TEX };
    // Take the world's light, like every other surface. Held items sit in the
    // player's own hand so Java keeps them a little brighter than ambient; a
    // floor under the skylight rather than a full multiply.
    let lit = |base: u32| {
        let l = unsafe { LIGHT } as u32;
        (base * (48 + l * 80 / 128) / 128).clamp(16, 255) as u8
    };
    held_face([t, rt, lf, b], face_tile(selected, 2), bt, lit(128)); // top tile, bright
    held_face([lf, b, ll, bl], face_tile(selected, 4), bt, lit(98)); // left side, mid
    held_face([b, rt, bl, rl], face_tile(selected, 0), bt, lit(72)); // right side, dark
}

/// Render a mob as a small set of cuboids (body + head + legs) -- recognizable
/// but cheap. Quadrupeds (pig/cow/sheep/chicken/spider) carry the body on four
/// legs with the head out front (+Z); the rest are bipeds.
/// The 16x16 face tile a mob kind wears on its head's front (+Z) face.
fn mob_face_tile(kind: u8) -> u8 {
    match kind {
        mob::PIG => tex::T_FACE_PIG,
        mob::COW => tex::T_FACE_COW,
        mob::SHEEP => tex::T_FACE_SHEEP,
        mob::CHICKEN => tex::T_FACE_CHICKEN,
        mob::ZOMBIE => tex::T_FACE_ZOMBIE,
        mob::SKELETON => tex::T_FACE_SKELETON,
        mob::SAPPER => tex::T_FACE_SAPPER,
        mob::WRAITH => tex::T_FACE_WRAITH,
        mob::EMBER => tex::T_FACE_EMBER,
        mob::WAILER => tex::T_FACE_WAILER,
        mob::CHARRED_SK => tex::T_FACE_CHARRED,
        mob::VILLAGER => tex::T_FACE_VILLAGER,
        mob::WOLF => tex::T_FACE_WOLF,
        _ => tex::T_FACE_SPIDER,
    }
}

/// True when a kind's body may be drawn with the hide texture.
///
/// emit_body_box draws T_HIDE through the CLUT of the kind's FACE tile, so a
/// kind borrowing another kind's face inherited that kind's COLOURS too -- the
/// wraith had a zombie-green torso, the villager was pink, the wolf was a
/// small sheep. All three have their own face tiles now (the atlas holds 64 and
/// had stopped at 50), so all three are textured again and this is every kind.
fn mob_body_textured(_kind: u8) -> bool {
    true
}

// Textured mob-head front faces (one per live mob; matches mob cap).
static mut HEAD_QUADS: [[QuadTexturedMaterial; 8]; RENDER_ARENAS] =
    [[EMPTY_QUAD; 8], [EMPTY_QUAD; 8]];
static mut HEAD_N: usize = 0;

// Textured mob-body faces (up to 4 visible faces x 8 mobs).
static mut BODY_QUADS: [[QuadTexturedMaterial; 32]; RENDER_ARENAS] =
    [[EMPTY_QUAD; 32], [EMPTY_QUAD; 32]];
static mut BODY_N: usize = 0;

/// Textured box for a mob BODY: every visible face wears the shared T_HIDE
/// mottle tile through the mob's own face-tile CLUT (indices 0..3 are that
/// palette's skin ramp), so bodies read as kind-coloured hide -- one tile,
/// zero extra CLUTs.
#[allow(clippy::too_many_arguments)]
fn emit_body_box(
    cam: &Camera,
    x0: i32,
    y0: i32,
    z0: i32,
    x1: i32,
    y1: i32,
    z1: i32,
    face_tile_of_kind: u8,
    count: &mut usize,
) {
    let hide = tex::T_HIDE as usize;
    let ft = face_tile_of_kind as usize;
    let mut dir = 0;
    while dir < 6 {
        let facing = match dir {
            0 => cam.x > x1 - 8,
            1 => cam.x < x0 + 8,
            2 => cam.y > y1 - 8,
            3 => cam.y < y0 + 8,
            4 => cam.z > z1 - 8,
            _ => cam.z < z0 + 8,
        };
        if facing && unsafe { BODY_N } < 32 {
            let verts = match dir {
                0 => [(x1, y1, z0), (x1, y1, z1), (x1, y0, z0), (x1, y0, z1)],
                1 => [(x0, y1, z1), (x0, y1, z0), (x0, y0, z1), (x0, y0, z0)],
                2 => [(x0, y1, z0), (x1, y1, z0), (x0, y1, z1), (x1, y1, z1)],
                3 => [(x0, y0, z1), (x1, y0, z1), (x0, y0, z0), (x1, y0, z0)],
                4 => [(x1, y1, z1), (x0, y1, z1), (x1, y0, z1), (x0, y0, z1)],
                _ => [(x0, y1, z0), (x1, y1, z0), (x0, y0, z0), (x1, y0, z0)],
            };
            let Some(p) = project_quad_gte(cam, &verts) else {
                dir += 1;
                continue;
            };
            let depth = (p[0].z + p[1].z + p[2].z + p[3].z) >> 2;
            if depth < FAR_Z && !quad_exploded(&p) {
                let slot = depth_slot(depth);
                unsafe {
                    let n = BODY_N;
                    let q = &mut BODY_QUADS[RENDER_ARENA][n];
                    q.tex_window = MAT_WIN[0][hide];
                    // Mobs take the full-sky row: they light by day/night, not
                    // by the block they happen to stand on.
                    q.color_cmd = MAT_CCMD[SKY_LEVELS - 1][0][dir][fog_band(depth)];
                    q.v0 = (p[0].x as u16 as u32) | ((p[0].y as u16 as u32) << 16);
                    q.uv0_clut = MAT_CLUT_HI[0][ft]; // the mob's face palette
                    q.v1 = (p[1].x as u16 as u32) | ((p[1].y as u16 as u32) << 16);
                    q.uv1_tpage = MAT_TPAGE_HI[0][hide] | 16;
                    q.v2 = (p[2].x as u16 as u32) | ((p[2].y as u16 as u32) << 16);
                    q.uv2 = 16 << 8;
                    q.v3 = (p[3].x as u16 as u32) | ((p[3].y as u16 as u32) << 16);
                    q.uv3 = (16 << 8) | 16;
                    OT[RENDER_ARENA].insert(
                        slot,
                        q as *mut QuadTexturedMaterial as *mut u32,
                        QuadTexturedMaterial::WORDS,
                    );
                    BODY_N = n + 1;
                }
                *count += 1;
            }
        }
        dir += 1;
    }
}

/// emit_box, except the +Z (front) face wears the mob's FACE tile -- the face
/// is what makes a Minecraft mob read as itself. Other five faces stay flat.
/// Head box with the textured face on an arbitrary side. `dir` is a face index
/// (0 +X, 1 -X, 4 +Z, 5 -Z); the remaining five sides draw flat.
#[allow(clippy::too_many_arguments)]
fn emit_head_box_dir(
    cam: &Camera,
    x0: i32,
    y0: i32,
    z0: i32,
    x1: i32,
    y1: i32,
    z1: i32,
    base: (u8, u8, u8),
    tile: u8,
    dir: usize,
    count: &mut usize,
) {
    // Only bother with the sticker when that side actually faces the camera.
    let facing = match dir {
        0 => cam.x > x1 - 8,
        1 => cam.x < x0 + 8,
        4 => cam.z > z1 - 8,
        _ => cam.z < z0 + 8,
    };
    if facing && unsafe { HEAD_N } < 8 {
        let verts = match dir {
            0 => [(x1, y1, z0), (x1, y1, z1), (x1, y0, z0), (x1, y0, z1)],
            1 => [(x0, y1, z1), (x0, y1, z0), (x0, y0, z1), (x0, y0, z0)],
            4 => [(x1, y1, z1), (x0, y1, z1), (x1, y0, z1), (x0, y0, z1)],
            _ => [(x0, y1, z0), (x1, y1, z0), (x0, y0, z0), (x1, y0, z0)],
        };
        if let Some(p) = project_quad_gte(cam, &verts) {
            let depth = (p[0].z + p[1].z + p[2].z + p[3].z) >> 2;
            if depth < FAR_Z && !quad_exploded(&p) {
                let slot = depth_slot(depth);
                let tint = face_tint(dir);
                let bt = unsafe { &BLOCK_TEX };
                let (tux, tuy) = tex::tile_uv(tile);
                let win = TextureWindow::power_of_two_tile(tux, tuy, 16, 16);
                let mat = TextureMaterial::opaque(bt.clut[tile as usize], bt.tpage, tint)
                    .with_texture_window(win);
                let packet = QuadTexturedMaterial::with_material(
                    [(p[0].x, p[0].y), (p[1].x, p[1].y), (p[2].x, p[2].y), (p[3].x, p[3].y)],
                    [(0, 0), (16, 0), (0, 16), (16, 16)],
                    mat,
                );
                unsafe {
                    let n = HEAD_N;
                    HEAD_QUADS[RENDER_ARENA][n] = packet;
                    OT[RENDER_ARENA].insert(
                        slot,
                        &mut HEAD_QUADS[RENDER_ARENA][n] as *mut QuadTexturedMaterial as *mut u32,
                        QuadTexturedMaterial::WORDS,
                    );
                    HEAD_N = n + 1;
                }
            }
        }
    }
    // Every side but the one wearing the face.
    let mask = 0x3F & !(1 << dir);
    emit_box_masked(cam, x0, y0, z0, x1, y1, z1, base, count, mask as u8);
}

/// Where a mob's face sits, and which way its long axis runs, from the 2-bit
/// facing. Returns (face-normal dir, unit step along the facing axis) so the
/// caller can place the head and swing the legs on the correct axis without a
/// rotation matrix -- the boxes are axis-aligned and stay that way.
#[inline]
fn facing_axis(facing: u8) -> (usize, i32, i32) {
    match facing {
        0 => (4, 0, 1),  // +Z
        1 => (0, 1, 0),  // +X
        2 => (5, 0, -1), // -Z
        _ => (1, -1, 0), // -X
    }
}

/// Fore/aft offset of a leg at gait phase `walk`, scaled by `amp`. Two legs in
/// antiphase gives a walk; four legs in diagonal pairs gives a trot, which is
/// what Java's quadrupeds do.
#[inline]
fn gait(walk: u8, amp: i32, antiphase: bool) -> i32 {
    let a = ((walk as u16) << 4) + if antiphase { 2048 } else { 0 };
    (sincos::sin_q12(a & 0x0FFF) * amp) >> 12
}

/// A small half-black ground decal under a mob. The first attempt drew these
/// immediately and terrain later painted over them; this version allocates the
/// blend packet in the UI arena, then flushes it into the world OT at the
/// ground's depth. One flat quad is enough at 320x240, and costs no texture or
/// CLUT space.
const MOB_SHADOWS: bool = true; // A/B knob; false removes the shadow packets

fn render_mob_shadow(cam: &Camera, m: mob::MobView, hw: i32) {
    if !MOB_SHADOWS || mob::is_flyer(m.kind) {
        return;
    }
    let r = (hw * 3 / 2).clamp(18, 42);
    let y = m.y + 2; // just above the floor: avoid coplanar raster disagreement
    let verts = [
        (m.x - r, y, m.z - r),
        (m.x + r, y, m.z - r),
        (m.x - r, y, m.z + r),
        (m.x + r, y, m.z + r),
    ];
    let Some(p) = project_quad_gte(cam, &verts) else {
        return;
    };
    let depth = (p[0].z + p[1].z + p[2].z + p[3].z) >> 2;
    if depth <= NEAR_Z || depth >= FAR_Z || quad_exploded(&p) {
        return;
    }
    // One slot nearer than the floor makes the painter draw the decal after
    // the ground, while genuinely nearer terrain still occludes it.
    let slot = depth_slot(depth).saturating_sub(1).max(1);
    ui_quad_blend_depth(
        [(p[0].x, p[0].y), (p[1].x, p[1].y), (p[2].x, p[2].y), (p[3].x, p[3].y)],
        0,
        0,
        0,
        slot,
    );
}

fn render_mob(cam: &Camera, m: mob::MobView, count: &mut usize) {
    let (hw, h) = mob::dims(m.kind);
    if m.kind == mob::DRAGON {
        render_dragon(cam, m, hw, h, count);
        return;
    }
    render_mob_shadow(cam, m, hw);
    // A priming sapper flashes bright white; a mob struck in the last few
    // frames flashes red. hurt_cd was already being tracked and read by nothing.
    let base = if m.priming {
        (250, 250, 250)
    } else if m.hurt {
        let c = mob_color(m.kind);
        ((c.0 / 2).saturating_add(128), c.1 / 2, c.2 / 2)
    } else {
        mob_color(m.kind)
    };
    let legc = scale_rgb(base, 72);
    let biped = m.kind == mob::ZOMBIE
        || m.kind == mob::SKELETON
        || m.kind == mob::SAPPER
        || m.kind == mob::WRAITH
        || m.kind == mob::VILLAGER
        || m.kind == mob::EMBER
        || m.kind == mob::CHARRED_SK;
    let (x, y, z) = (m.x, m.y, m.z);
    let face = mob_face_tile(m.kind);
    let (fdir, fx, fz) = facing_axis(m.facing);
    // Leg LOD: the old Manhattan test at 8 blocks dropped legs on a mob only 6
    // ahead and 6 to the side, and the body then stretched to the ground as a
    // legless slab at very ordinary range. True squared distance, twice as far.
    let (ddx, ddz) = (x - cam.x, z - cam.z);
    let legs = ddx * ddx + ddz * ddz < (16 * BLOCK) * (16 * BLOCK);
    // Flyers have no business standing on legs.
    let legs = legs && !mob::is_flyer(m.kind);
    if biped {
        let leg_h = h * 3 / 8; // Java bipeds are 0.375 legs, not 0.45
        let lw = hw / 2;
        let sw = if legs { gait(m.walk, hw, false) } else { 0 };
        if legs {
            // Swing along the facing axis, the two legs in antiphase.
            let (ax, az) = (fx * sw, fz * sw);
            let (bx, bz) = (-ax, -az);
            emit_box(cam, x - hw + ax, y, z - lw + az, x + ax, y + leg_h, z + lw + az, legc, count);
            emit_box(cam, x + bx, y, z - lw + bz, x + hw + bx, y + leg_h, z + lw + bz, legc, count);
        }
        let body_y0 = if legs { y + leg_h } else { y };
        let body_y1 = y + h * 13 / 16;
        if m.priming || m.hurt || !mob_body_textured(m.kind) {
            emit_box(cam, x - hw, body_y0, z - lw, x + hw, body_y1, z + lw, base, count);
        } else {
            emit_body_box(cam, x - hw, body_y0, z - lw, x + hw, body_y1, z + lw, face, count);
        }
        // Arms. A zombie holds them straight out forward, which IS the zombie
        // read; everyone else hangs them at the sides and swings them against
        // the legs. Six quads a mob at 215 primitives a frame is nothing.
        let aw = hw / 3;
        let arm_y1 = body_y1 - h / 16;
        let arm_y0 = arm_y1 - h * 5 / 16;
        if m.kind != mob::SAPPER {
            let zom = m.kind == mob::ZOMBIE;
            let (ox, oz) = if zom { (fx * hw, fz * hw) } else { (0, 0) };
            let asw = if zom { 0 } else { -sw };
            emit_box(cam, x - hw - aw * 2 + ox, arm_y0, z - aw + oz + fz * asw,
                     x - hw + ox, arm_y1, z + aw + oz + fz * asw, legc, count);
            emit_box(cam, x + hw + ox, arm_y0, z - aw + oz - fz * asw,
                     x + hw + aw * 2 + ox, arm_y1, z + aw + oz - fz * asw, legc, count);
        }
        let hh = hw * 3 / 4;
        emit_face_box(cam, x - hh, body_y1, z - hh, x + hh, y + h, z + hh, base, face, fdir, m, count);
    } else {
        let leg_h = h * 2 / 5;
        let lw = hw / 3;
        let sx = hw - lw;
        // Animal bodies are LONG and low in Java -- a pig is 16 deep against 10
        // wide. Ours was a cube, which is most of why every quadruped read as a
        // coloured box rather than an animal. Stretch along the facing axis.
        let (long_x, long_z) = (hw + fx.abs() * hw / 2, hw + fz.abs() * hw / 2);
        if legs {
            // Diagonal pairs, as Java trots them.
            let s1 = gait(m.walk, hw / 2, false);
            let s2 = gait(m.walk, hw / 2, true);
            let leg = |cx: i32, cz: i32, sw: i32, c: &mut usize| {
                let (ax, az) = (fx * sw, fz * sw);
                emit_box(cam, cx - lw + ax, y, cz - lw + az, cx + lw + ax, y + leg_h, cz + lw + az, legc, c);
            };
            leg(x - sx, z - long_z + lw, s1, count);
            leg(x + sx, z - long_z + lw, s2, count);
            leg(x - sx, z + long_z - lw, s2, count);
            leg(x + sx, z + long_z - lw, s1, count);
        }
        let body_y0 = if legs { y + leg_h } else { y };
        // Head no longer as tall as the whole body, and placed on the FACING
        // side rather than hard-coded to +Z.
        let body_y1 = y + h;
        if m.hurt || !mob_body_textured(m.kind) {
            emit_box(cam, x - long_x, body_y0, z - long_z, x + long_x, body_y1, z + long_z, base, count);
        } else {
            emit_body_box(cam, x - long_x, body_y0, z - long_z, x + long_x, body_y1, z + long_z, face, count);
        }
        let hh = hw * 3 / 4;
        let hy0 = body_y1 - h * 2 / 5;
        let (hx, hz) = (fx * (long_x + hh), fz * (long_z + hh));
        emit_face_box(cam, x + hx - hh, hy0, z + hz - hh, x + hx + hh, hy0 + h * 2 / 5, z + hz + hh,
                      base, face, fdir, m, count);
    }
}

/// Head box whose textured face goes on the side the mob is FACING, instead of
/// always on +Z. Walking behind a pig used to show an anonymous pink box.
#[allow(clippy::too_many_arguments)]
fn emit_face_box(
    cam: &Camera,
    x0: i32,
    y0: i32,
    z0: i32,
    x1: i32,
    y1: i32,
    z1: i32,
    base: (u8, u8, u8),
    tile: u8,
    dir: usize,
    m: mob::MobView,
    count: &mut usize,
) {
    if m.priming || m.hurt {
        emit_box(cam, x0, y0, z0, x1, y1, z1, base, count);
        return;
    }
    emit_head_box_dir(cam, x0, y0, z0, x1, y1, z1, base, tile, dir, count);
}

/// Render all live mobs as cuboid models into the OT./// Render all live mobs as cuboid models into the OT.
#[inline(never)]
fn render_mobs(cam: &Camera, frame: u32) {
    let mut count = 0usize;
    // The world pass leaves the GTE translation on its last CHUNK's origin;
    // mob/particle/pick projection is camera-relative and needs TR = 0.
    scene::load_translation(Vec3I32::new(0, 0, 0));
    unsafe {
        HEAD_N = 0;
        BODY_N = 0;
    }
    let mut i = 0;
    while i < mob::CAP {
        let m = mob::get(i);
        if m.alive {
            let dx = (m.x - cam.x).abs();
            let dz = (m.z - cam.z).abs();
            if dx + dz <= FAR_Z + 4 * BLOCK {
                render_mob(cam, m, &mut count);
            }
        }
        i += 1;
    }
    // Arrows as small dark boxes.
    let mut j = 0;
    while j < mob::arrow_cap() {
        let (alive, ax, ay, az) = mob::arrow_view(j);
        if alive {
            let dx = (ax - cam.x).abs();
            let dz = (az - cam.z).abs();
            if dx + dz <= FAR_Z + 4 * BLOCK {
                emit_box(cam, ax - 5, ay - 5, az - 5, ax + 5, ay + 5, az + 5, (50, 38, 24), &mut count);
            }
        }
        j += 1;
    }
    render_drops(cam, frame, &mut count);
}

// ---- Cross-sprite plants: wheat + saplings as X-billboards ----
const CROSS_PLANTS: bool = true; // false = old cube rendering (A/B knob)
/// Inside this squared distance a plant draws both diagonals; past it, one.
/// 8 blocks -- beyond that a cross sprite is a few pixels wide.
const PLANT_SINGLE_D2: i32 = (8 * BLOCK) * (8 * BLOCK);
const MAX_PLANT_QUADS: usize = 128; // 2 quads/plant; ponytail cap on visible plants/frame
static mut PLANT_QUADS: [[QuadTexturedMaterial; MAX_PLANT_QUADS]; RENDER_ARENAS] =
    [
        [EMPTY_QUAD; MAX_PLANT_QUADS],
        [EMPTY_QUAD; MAX_PLANT_QUADS],
    ];
static mut PLANT_N: usize = 0;

fn plant_tile(blk: u8) -> u8 {
    match blk {
        WHEAT => tex::T_CROP_YOUNG,
        WHEAT_RIPE => tex::T_CROP_RIPE,
        FLOWER_R => tex::T_FLOWER_R,
        FLOWER_Y => tex::T_FLOWER_Y,
        TALL_GRASS => tex::T_TALLGRASS,
        SUGAR_CANE => tex::T_CROP_YOUNG,
        FIRE => tex::T_FIRE,
        PORTAL | VOID_PORTAL => tex::T_PORTAL,
        EMBER_CAP => tex::T_CROP_RIPE,
        _ => tex::T_SAPLING_CROSS, // SAPLING
    }
}

/// Draw cross-sprite plants as depth-sorted X-billboards: two diagonal textured
/// quads per cell with a transparent (texel-0) background. Runs after the world
/// pass (so it resets TR = 0) and before OT.submit, so terrain occludes plants.
#[inline(never)]
fn render_plants(cam: &Camera) {
    if !CROSS_PLANTS {
        return;
    }
    scene::load_translation(Vec3I32::new(0, 0, 0)); // camera-relative, like mobs
    unsafe { PLANT_N = 0 };
    let bt = unsafe { BLOCK_TEX };
    let tint = face_tint(2); // top-face shade -> plants track day/night light
    world::for_plants(cam, |blk, wx, wy, wz| {
        let cdx = wx + BLOCK / 2 - cam.x;
        let cdz = wz + BLOCK / 2 - cam.z;
        if is_small_block(blk) {
            // Solid shapes, not billboards: no near-cull (you can stand on one)
            // and they draw as far as the terrain does.
            let (x0, y0, z0, x1, y1, z1, tile, vhi) = small_box_shape(blk, wx, wy, wz);
            emit_small_box(cam, tile, x0, y0, z0, x1, y1, z1, vhi);
            if is_stairs(blk) {
                // The upper step, on top of the slab the call above drew.
                let (sx0, sy0, sz0, sx1, sy1, sz1) = stair_step_box(
                    blk,
                    world_to_block_x(wx),
                    world_to_block_y(wy),
                    world_to_block_z(wz),
                );
                emit_small_box(cam, tile, sx0, sy0, sz0, sx1, sy1, sz1, 8);
            }
            return;
        }
        // Cull a plant you're standing on: its near billboard edge blows up to
        // fill the screen, and a meadow of them overlaps into a coloured mess.
        let plant_d2 = cdx * cdx + cdz * cdz;
        if plant_d2 < NEAR_PLANT_D2 {
            return;
        }
        // The chunk gate now reaches 20 blocks for built shapes; a cross sprite
        // that far out is a couple of pixels, so drop it here.
        if plant_d2 > PLANT_SPRITE_FAR_D2 {
            return;
        }
        // ONE quad past PLANT_SINGLE_D2, two inside it. A cross sprite is two
        // vertical quads on the cell diagonals, which is what Java does and what
        // makes a plant read from any angle -- but at 320x240 a distant plant is
        // a handful of pixels and the second diagonal adds nothing you can see.
        //
        // Kept, but do not expect much: measured on the green spawn this moved
        // the world pass 971,303 -> 959,953 and the frame 18.6 -> 18.8 fps.
        // Controlled builds later showed decorations as a whole add ~181K
        // cycles; the second diagonal is simply a small share of their cost.
        // Improving plants means reducing the per-plant projection work, not
        // shaving the already-tiny distant half of each cross.
        let near_enough = plant_d2 < PLANT_SINGLE_D2;
        let ncross = if near_enough { 2 } else { 1 };
        if unsafe { PLANT_N } + 2 > MAX_PLANT_QUADS {
            return;
        }
        let tile = plant_tile(blk);
        let (tux, tuy) = tex::tile_uv(tile);
        let win = TextureWindow::power_of_two_tile(tux, tuy, 16, 16);
        let mat =
            TextureMaterial::opaque(bt.clut[tile as usize], bt.tpage, tint).with_texture_window(win);
        let (x0, x1) = (wx, wx + BLOCK);
        let (y0, y1) = (wy, wy + BLOCK);
        let (z0, z1) = (wz, wz + BLOCK);
        // Two vertical quads on the cell diagonals. Corner order
        // (top-left, top-right, bottom-left, bottom-right) matches the UVs below.
        let crosses = [
            [(x0, y1, z0), (x1, y1, z1), (x0, y0, z0), (x1, y0, z1)], // '\' diagonal
            [(x0, y1, z1), (x1, y1, z0), (x0, y0, z1), (x1, y0, z0)], // '/' diagonal
        ];
        let mut c = 0;
        while c < ncross {
            let Some(p) = project_quad_gte(cam, &crosses[c]) else {
                c += 1;
                continue;
            };
            let depth = (p[0].z + p[1].z + p[2].z + p[3].z) >> 2;
            // Backstop the near-cull: skip any quad still spanning far more than
            // a sane close billboard (would draw as a full-screen explosion).
            if depth < FAR_Z && !quad_exploded(&p) {
                let slot = depth_slot(depth);
                let packet = QuadTexturedMaterial::with_material(
                    [(p[0].x, p[0].y), (p[1].x, p[1].y), (p[2].x, p[2].y), (p[3].x, p[3].y)],
                    [(0, 0), (16, 0), (0, 16), (16, 16)],
                    mat,
                );
                unsafe {
                    let n = PLANT_N;
                    PLANT_QUADS[RENDER_ARENA][n] = packet;
                    OT[RENDER_ARENA].insert(
                        slot,
                        &mut PLANT_QUADS[RENDER_ARENA][n] as *mut QuadTexturedMaterial as *mut u32,
                        QuadTexturedMaterial::WORDS,
                    );
                    PLANT_N = n + 1;
                }
            }
            c += 1;
        }
    });
}

/// One textured box for a non-full-block shape, emitted into the same OT and
/// quad pool as the plants. Not `emit_box`: that one is flat-shaded (it draws
/// mobs), and a grey cuboid sitting in textured terrain reads as a placeholder.
///
/// `vhi` is the bottom of the V range used on the SIDE faces. A half-height slab
/// takes the tile's top half at native scale rather than squashing the whole
/// tile into eight pixels.
#[allow(clippy::too_many_arguments)]
fn emit_small_box(
    cam: &Camera,
    tile: u8,
    x0: i32,
    y0: i32,
    z0: i32,
    x1: i32,
    y1: i32,
    z1: i32,
    vhi: u8,
) {
    let bt = unsafe { BLOCK_TEX };
    let (tux, tuy) = tex::tile_uv(tile);
    let win = TextureWindow::power_of_two_tile(tux, tuy, 16, 16);
    let mut dir = 0usize;
    while dir < 6 {
        // Same backface test emit_box uses: keep only the faces pointing at us.
        let facing = match dir {
            0 => cam.x > x1 - 8,
            1 => cam.x < x0 + 8,
            2 => cam.y > y1 - 8,
            3 => cam.y < y0 + 8,
            4 => cam.z > z1 - 8,
            _ => cam.z < z0 + 8,
        };
        if facing && unsafe { PLANT_N } < MAX_PLANT_QUADS {
            let verts = match dir {
                0 => [(x1, y1, z0), (x1, y1, z1), (x1, y0, z0), (x1, y0, z1)],
                1 => [(x0, y1, z1), (x0, y1, z0), (x0, y0, z1), (x0, y0, z0)],
                2 => [(x0, y1, z0), (x1, y1, z0), (x0, y1, z1), (x1, y1, z1)],
                3 => [(x0, y0, z1), (x1, y0, z1), (x0, y0, z0), (x1, y0, z0)],
                4 => [(x0, y1, z1), (x1, y1, z1), (x0, y0, z1), (x1, y0, z1)],
                _ => [(x1, y1, z0), (x0, y1, z0), (x1, y0, z0), (x0, y0, z0)],
            };
            if let Some(p) = project_quad_gte(cam, &verts) {
                let depth = (p[0].z + p[1].z + p[2].z + p[3].z) >> 2;
                if depth > 0 && depth < FAR_Z {
                    // Top and bottom faces read the whole tile; the sides take
                    // only as much of it as the box is tall.
                    let v1 = if dir == 2 || dir == 3 { 16 } else { vhi };
                    let mat = TextureMaterial::opaque(bt.clut[tile as usize], bt.tpage, face_tint(dir))
                        .with_texture_window(win);
                    let packet = QuadTexturedMaterial::with_material(
                        [(p[0].x, p[0].y), (p[1].x, p[1].y), (p[2].x, p[2].y), (p[3].x, p[3].y)],
                        [(0, 0), (16, 0), (0, v1), (16, v1)],
                        mat,
                    );
                    unsafe {
                        let n = PLANT_N;
                        PLANT_QUADS[RENDER_ARENA][n] = packet;
                        OT[RENDER_ARENA].insert(
                            depth_slot(depth),
                            &mut PLANT_QUADS[RENDER_ARENA][n] as *mut QuadTexturedMaterial
                                as *mut u32,
                            QuadTexturedMaterial::WORDS,
                        );
                        PLANT_N = n + 1;
                    }
                }
            }
        }
        dir += 1;
    }
}

/// Geometry of a small block: `(x0, y0, z0, x1, y1, z1, tile, side V range)`
/// in world units, relative to the block's corner.
fn small_box_shape(blk: u8, wx: i32, wy: i32, wz: i32) -> (i32, i32, i32, i32, i32, i32, u8, u8) {
    match blk {
        // Half-height, full footprint. Sits on the block grid, so a slab floor
        // is walkable at half a block up.
        SLAB | STAIRS_N | STAIRS_E | STAIRS_S | STAIRS_W => {
            (wx, wy, wz, wx + BLOCK, wy + BLOCK / 2, wz + BLOCK, tex::T_COBBLE, 8)
        }
        // A post through the middle of the cell. Vanilla adds arms to the
        // neighbours; ponytail: post only, which still reads as a fence line and
        // costs one box instead of up to five.
        _ => (
            wx + BLOCK * 3 / 8,
            wy,
            wz + BLOCK * 3 / 8,
            wx + BLOCK * 5 / 8,
            wy + BLOCK,
            wz + BLOCK * 5 / 8,
            tex::T_WOOD_SIDE,
            16,
        ),
    }
}

// ---- Particles: short-lived debris on block break and explosions ----
// ponytail: a fixed pool, no block collision (they just fall and fade), drawn
// immediate over the world (tiny + brief, so depth errors are unnoticeable).
const MAX_PARTICLES: usize = 40;
static mut PART_X: [i32; MAX_PARTICLES] = [0; MAX_PARTICLES];
static mut PART_Y: [i32; MAX_PARTICLES] = [0; MAX_PARTICLES];
static mut PART_Z: [i32; MAX_PARTICLES] = [0; MAX_PARTICLES];
static mut PART_VX: [i32; MAX_PARTICLES] = [0; MAX_PARTICLES];
static mut PART_VY: [i32; MAX_PARTICLES] = [0; MAX_PARTICLES];
static mut PART_VZ: [i32; MAX_PARTICLES] = [0; MAX_PARTICLES];
static mut PART_LIFE: [u8; MAX_PARTICLES] = [0; MAX_PARTICLES]; // 0 = dead
static mut PART_COL: [(u8, u8, u8); MAX_PARTICLES] = [(0, 0, 0); MAX_PARTICLES];

#[inline(never)]
fn spawn_particles(wx: i32, wy: i32, wz: i32, col: (u8, u8, u8), n: u32, seed: u32, spread: i32) {
    let mut spawned = 0u32;
    let mut i = 0usize;
    while i < MAX_PARTICLES && spawned < n {
        if unsafe { PART_LIFE[i] } == 0 {
            let h = seed.wrapping_add(spawned).wrapping_mul(2_654_435_761);
            unsafe {
                PART_X[i] = wx + ((h & 0x1F) as i32 - 16);
                PART_Y[i] = wy + (((h >> 5) & 0x1F) as i32 - 16);
                PART_Z[i] = wz + (((h >> 10) & 0x1F) as i32 - 16);
                PART_VX[i] = (((h >> 15) & 0x1F) as i32 - 16) * spread / 16;
                PART_VY[i] = (((h >> 20) & 0x1F) as i32) * spread / 16 + 6;
                PART_VZ[i] = (((h >> 25) & 0x1F) as i32 - 16) * spread / 16;
                PART_LIFE[i] = 16 + ((h >> 3) & 0xF) as u8;
                PART_COL[i] = col;
            }
            spawned += 1;
        }
        i += 1;
    }
}

fn tick_particles() {
    let mut i = 0usize;
    while i < MAX_PARTICLES {
        unsafe {
            if PART_LIFE[i] > 0 {
                PART_VY[i] -= 3; // gravity
                PART_X[i] += PART_VX[i];
                PART_Y[i] += PART_VY[i];
                PART_Z[i] += PART_VZ[i];
                PART_VX[i] = PART_VX[i] * 7 / 8; // air drag
                PART_VZ[i] = PART_VZ[i] * 7 / 8;
                PART_LIFE[i] -= 1;
            }
        }
        i += 1;
    }
}

// ---- Item entities: mined blocks fall as pickups ----
// Breaking a block used to teleport it straight into the inventory, which
// skipped the most recognisable beat of Minecraft's core loop. Drops now pop
// out, fall, settle, and get collected when you walk over them.
//
// ponytail: a fixed pool, no stack merging, and a `rest` flag that drops a
// settled item to an age + pickup check per tick. Horizontal collision only
// runs while an item is still moving, so the steady-state cost of a floor
// covered in drops is a few compares.
const MAX_DROPS: usize = 24;
const DROP_HALF_W: i32 = 5;
const DROP_H: i32 = 10;
/// Horizontal reach of a pickup. Vanilla is one block; matching that means you
/// sweep up what you mined without having to stand on it.
// Java's pickup rule (minecraft.wiki, Item (entity)): the player's hitbox
// inflated by 1 block horizontally and 0.5 vertically collects any item whose
// own hitbox it touches -- there is NO magnet; the fly-to-player is a purely
// cosmetic animation after the item is already collected. The generous box is
// what makes vanilla pickup feel like attraction, so ours matches it:
// half-width 19 + 64 + the item's 6 = 89 units (~1.4 blocks) from centre.
const DROP_PICKUP_R: i32 = PLAYER_HALF_W + BLOCK + DROP_HALF_W;
/// Magnet bubble: inside this range a drop stops settling and steers at the
/// player. Well past pickup range, so items visibly swim to you.
const DROP_ATTRACT_R: i32 = 5 * BLOCK / 2;
/// Frames of the cosmetic zoom-to-player after collection (Java animates
/// roughly this long; the item entity there never actually moves).
const DROP_FLY_FRAMES: u8 = 8;
/// ~40s at 30fps. Vanilla is 5 minutes, but the pool is 24 and a player who
/// strip-mines would otherwise fill it and silently lose later drops.
const DROP_TTL: u16 = 1200;
/// Vanilla's 10-tick delay before an item can be collected, so a drop is
/// visible rather than vanishing on the frame it spawns.
const DROP_PICKUP_DELAY: u8 = 10;

static mut DROP_ITEM: [u8; MAX_DROPS] = [AIR; MAX_DROPS]; // AIR = free slot
static mut DROP_FLY: [u8; MAX_DROPS] = [0; MAX_DROPS]; // >0: cosmetic zoom, frames left
static mut DROP_X: [i32; MAX_DROPS] = [0; MAX_DROPS];
static mut DROP_Y: [i32; MAX_DROPS] = [0; MAX_DROPS];
static mut DROP_Z: [i32; MAX_DROPS] = [0; MAX_DROPS];
static mut DROP_VX: [i32; MAX_DROPS] = [0; MAX_DROPS];
static mut DROP_VY: [i32; MAX_DROPS] = [0; MAX_DROPS];
static mut DROP_VZ: [i32; MAX_DROPS] = [0; MAX_DROPS];
static mut DROP_AGE: [u16; MAX_DROPS] = [0; MAX_DROPS];
static mut DROP_DELAY: [u8; MAX_DROPS] = [0; MAX_DROPS];
static mut DROP_REST: [bool; MAX_DROPS] = [false; MAX_DROPS];

/// Spawn the drop for `block` at a world point, scattered by `seed`. Silently
/// does nothing for blocks that yield nothing (leaves, fluids, flowers) or when
/// the pool is full -- the same outcome the old direct-to-inventory path had for
/// a full stack.
fn spawn_drop(wx: i32, wy: i32, wz: i32, block: u8, seed: u32) {
    let item = drop_of(block);
    if item == AIR {
        return;
    }
    give_drop(wx, wy, wz, item, seed);
}

/// Spawn an already-resolved item (mob loot, sapling rolls) rather than a block
/// to run through `drop_of`.
fn give_drop(wx: i32, wy: i32, wz: i32, item: u8, seed: u32) {
    let mut i = 0usize;
    while i < MAX_DROPS {
        if unsafe { DROP_ITEM[i] } == AIR {
            let h = seed.wrapping_mul(2_654_435_761);
            unsafe {
                DROP_ITEM[i] = item;
                DROP_X[i] = wx;
                DROP_Y[i] = wy;
                DROP_Z[i] = wz;
                // A small sideways pop, so a vein of ore does not stack every
                // drop in one column.
                DROP_VX[i] = ((h & 0x7) as i32) - 3;
                DROP_VY[i] = 6;
                DROP_VZ[i] = (((h >> 3) & 0x7) as i32) - 3;
                DROP_AGE[i] = 0;
                DROP_DELAY[i] = DROP_PICKUP_DELAY;
                DROP_REST[i] = false;
                DROP_FLY[i] = 0;
            }
            return;
        }
        i += 1;
    }
}

fn tick_drops(player: &Player) {
    let mut i = 0usize;
    while i < MAX_DROPS {
        let item = unsafe { DROP_ITEM[i] };
        if item == AIR {
            i += 1;
            continue;
        }
        unsafe {
            // Collected: the cosmetic zoom to the player's chest, as in Java
            // (where the entity never moves and the fly-in is client-side
            // animation). The item is already in the inventory; each frame
            // closes 1/n of the remaining gap, converging exactly.
            if DROP_FLY[i] > 0 {
                let n = DROP_FLY[i] as i32;
                DROP_X[i] += (player.x - DROP_X[i]) / n;
                DROP_Y[i] += (player.y + 60 - DROP_Y[i]) / n;
                DROP_Z[i] += (player.z - DROP_Z[i]) / n;
                DROP_FLY[i] -= 1;
                if DROP_FLY[i] == 0 {
                    DROP_ITEM[i] = AIR;
                }
                i += 1;
                continue;
            }
            DROP_AGE[i] += 1;
            if DROP_AGE[i] >= DROP_TTL {
                DROP_ITEM[i] = AIR;
                i += 1;
                continue;
            }
            if DROP_DELAY[i] > 0 {
                DROP_DELAY[i] -= 1;
            }
            // Magnet: inside the bubble the drop un-settles and steers at the
            // player's chest -- through the collision steps below, so it flows
            // around corners rather than through walls. Speed is proportional
            // to distance, so it decelerates into the pickup radius.
            if DROP_DELAY[i] == 0 {
                let adx = player.x - DROP_X[i];
                let ady = player.y + 40 - DROP_Y[i];
                let adz = player.z - DROP_Z[i];
                if adx.abs() < DROP_ATTRACT_R
                    && adz.abs() < DROP_ATTRACT_R
                    && ady.abs() < DROP_ATTRACT_R
                {
                    DROP_REST[i] = false;
                    DROP_VX[i] = (adx / 6).clamp(-18, 18);
                    DROP_VZ[i] = (adz / 6).clamp(-18, 18);
                    DROP_VY[i] = (ady / 6).clamp(-14, 14) + 3; // +3 pre-cancels gravity
                }
            }
            if !DROP_REST[i] {
                // Axis at a time, reverting the axis that hits, so an item
                // sliding into a wall keeps the rest of its motion.
                let (x, y, z) = (DROP_X[i], DROP_Y[i], DROP_Z[i]);
                DROP_VY[i] -= 3; // gravity, same pull the particles use
                let nx = x + DROP_VX[i];
                if aabb_collides_dims(nx, y, z, DROP_HALF_W, DROP_H) {
                    DROP_VX[i] = 0;
                } else {
                    DROP_X[i] = nx;
                }
                let nz = DROP_Z[i] + DROP_VZ[i];
                if aabb_collides_dims(DROP_X[i], y, nz, DROP_HALF_W, DROP_H) {
                    DROP_VZ[i] = 0;
                } else {
                    DROP_Z[i] = nz;
                }
                let ny = y + DROP_VY[i];
                if aabb_collides_dims(DROP_X[i], ny, DROP_Z[i], DROP_HALF_W, DROP_H) {
                    // Landed (or bumped a ceiling). Only a downward stop counts
                    // as settled.
                    if DROP_VY[i] < 0 {
                        DROP_REST[i] = true;
                    }
                    DROP_VY[i] = 0;
                } else {
                    DROP_Y[i] = ny;
                }
                DROP_VX[i] = DROP_VX[i] * 7 / 8; // ground/air drag
                DROP_VZ[i] = DROP_VZ[i] * 7 / 8;
            }
            if DROP_DELAY[i] == 0 {
                let dx = (DROP_X[i] - player.x).abs();
                let dz = (DROP_Z[i] - player.z).abs();
                let dy = DROP_Y[i] - player.y;
                // Generous vertically on purpose: mining straight down drops the
                // item into the hole you are about to stand in, and a tight
                // window there would lose drops the old teleport never lost.
                if dx <= DROP_PICKUP_R
                    && dz <= DROP_PICKUP_R
                    && dy > -2 * BLOCK
                    && dy < PLAYER_HEIGHT + BLOCK
                {
                    inv_give(item, 1);
                    DROP_FLY[i] = DROP_FLY_FRAMES;
                    sfx::blip();
                }
            }
        }
        i += 1;
    }
}

/// Drops as small boxes in the world pass, the same way arrows are drawn, so
/// they sort against terrain instead of floating over it. Vanilla bobs them;
/// a 4-step triangle wave off the frame counter is enough to read as alive.
fn render_drops(cam: &Camera, frame: u32, count: &mut usize) {
    let mut i = 0usize;
    while i < MAX_DROPS {
        let item = unsafe { DROP_ITEM[i] };
        if item != AIR {
            let (x, y, z) = unsafe { (DROP_X[i], DROP_Y[i], DROP_Z[i]) };
            if (x - cam.x).abs() + (z - cam.z).abs() <= FAR_Z + 4 * BLOCK {
                let phase = ((frame >> 2).wrapping_add(i as u32 * 3) & 7) as i32;
                let bob = if phase < 4 { phase } else { 7 - phase };
                let yb = y + bob;
                let c = block_particle_color(item);
                emit_box(cam, x - 6, yb, z - 6, x + 6, yb + 12, z + 6, c, count);
            }
        }
        i += 1;
    }
}

/// Project one camera-relative world point on the GTE (one RTPS; TR must be 0).
#[inline(always)]
fn project_point_gte(cam: &Camera, p: (i32, i32, i32)) -> Proj {
    let v = Vec3I16::new((p.0 - cam.x) as i16, (p.1 - cam.y) as i16, (p.2 - cam.z) as i16);
    let pr = scene::project_vertex_scheduled(v);
    Proj { x: pr.sx, y: pr.sy.clamp(-511, 511), z: pr.sz as i32 }
}

#[inline(never)]
fn render_particles(cam: &Camera) {
    // Runs after render_mobs, so the GTE translation is already 0
    // (camera-relative); each particle is one hardware RTPS.
    let mut i = 0usize;
    while i < MAX_PARTICLES {
        unsafe {
            if PART_LIFE[i] > 0 {
                let p = project_point_gte(cam, (PART_X[i], PART_Y[i], PART_Z[i]));
                if p.z >= GTE_NEAR as i32 && p.z <= FAR_Z {
                    let r = if p.z < 3 * BLOCK { 2 } else { 1 };
                    billboard(p.x, p.y, r, r, PART_COL[i]);
                }
            }
        }
        i += 1;
    }
}

fn block_particle_color(b: u8) -> (u8, u8, u8) {
    match b {
        GRASS => (96, 150, 72),
        DIRT => (122, 86, 56),
        SAND => (204, 192, 142),
        WOOD => (140, 112, 72),
        LEAVES => (74, 120, 60),
        WATER | WATER_F1..=WATER_F7 => (70, 110, 200),
        SNOW => (232, 234, 244),
        LAVA | LAVA_F1..=LAVA_F3 | FIRE => (220, 110, 40),
        STONE | COAL_ORE | IRON_ORE | GOLD_ORE | DIAMOND_ORE => (132, 132, 132),
        _ => (140, 122, 102),
    }
}

/// Camera-space vertex used by the close-terrain clipper. UVs are Q8 so an
/// edge intersection does not snap to an integer texel until packet emission.
#[derive(Copy, Clone)]
struct ClipVert {
    x: i32,
    y: i32,
    z: i32,
    u: i32,
    v: i32,
    r: i32,
    g: i32,
    b: i32,
}

const EMPTY_CLIP_VERT: ClipVert =
    ClipVert { x: 0, y: 0, z: 0, u: 0, v: 0, r: 0, g: 0, b: 0 };
const CLIP_VERT_CAP: usize = 12;
#[inline]
fn clip_distance(p: ClipVert, plane: usize) -> i32 {
    match plane {
        // Real camera plane. Software division is valid below the GTE's H/2
        // saturation point, so only actual eye-crossing geometry is removed.
        0 => p.z - NEAR_Z,
        1 => FAR_Z - p.z,
        // Screen x = CX + H*x/z, screen y = CY + H*y/z. Clipping in camera
        // space before the divide keeps every emitted coordinate inside the
        // GPU's signed 11-bit rails without distorting an endpoint.
        2 => PROJ_H * p.x + CX as i32 * p.z,
        3 => (SCREEN_W as i32 - CX as i32) * p.z - PROJ_H * p.x,
        4 => PROJ_H * p.y + CY as i32 * p.z,
        _ => (SCREEN_H as i32 - CY as i32) * p.z - PROJ_H * p.y,
    }
}

#[inline]
fn clip_intersection(a: ClipVert, b: ClipVert, da: i32, db: i32) -> ClipVert {
    let den = da - db;
    let t_q8 = (da << 8) / den;
    let lerp = |av: i32, bv: i32| -> i32 {
        av + (((bv - av) * t_q8) >> 8)
    };
    ClipVert {
        x: lerp(a.x, b.x),
        y: lerp(a.y, b.y),
        z: lerp(a.z, b.z),
        u: lerp(a.u, b.u),
        v: lerp(a.v, b.v),
        r: lerp(a.r, b.r),
        g: lerp(a.g, b.g),
        b: lerp(a.b, b.b),
    }
}

fn clip_polygon_plane(
    src: &[ClipVert; CLIP_VERT_CAP],
    src_n: usize,
    dst: &mut [ClipVert; CLIP_VERT_CAP],
    plane: usize,
) -> usize {
    if src_n == 0 {
        return 0;
    }
    let mut out = 0usize;
    let mut previous = src[src_n - 1];
    let mut previous_d = clip_distance(previous, plane);
    let mut i = 0usize;
    while i < src_n {
        let current = src[i];
        let current_d = clip_distance(current, plane);
        let previous_in = previous_d >= 0;
        let current_in = current_d >= 0;
        if previous_in != current_in && out < CLIP_VERT_CAP {
            dst[out] = clip_intersection(previous, current, previous_d, current_d);
            out += 1;
        }
        if current_in && out < CLIP_VERT_CAP {
            dst[out] = current;
            out += 1;
        }
        previous = current;
        previous_d = current_d;
        i += 1;
    }
    out
}

/// Clip a camera-space line segment against the same six planes as terrain.
///
/// Selection-box edges used to project their endpoints independently. If one
/// endpoint crossed the eye, the surviving endpoint could leave a long edge in
/// an unrelated screen position. Clipping the segment itself preserves the box
/// silhouette and never feeds a camera-plane endpoint to the divider.
fn clip_line_segment(mut a: ClipVert, mut b: ClipVert) -> Option<(ClipVert, ClipVert)> {
    let mut plane = 0usize;
    while plane < 6 {
        let da = clip_distance(a, plane);
        let db = clip_distance(b, plane);
        if da < 0 && db < 0 {
            return None;
        }
        if da < 0 {
            a = clip_intersection(a, b, da, db);
        } else if db < 0 {
            b = clip_intersection(a, b, da, db);
        }
        plane += 1;
    }
    Some((a, b))
}

#[inline]
fn clip_project(p: ClipVert) -> Proj {
    // Clipping guarantees NEAR_Z <= z <= FAR_Z, so use the renderer's 1/z
    // table just like project_world/clip_quad_near. The former two integer
    // divides per emitted vertex dominated the tessellated close-face path.
    let r = unsafe { RECIP[p.z as usize] };
    Proj {
        x: (CX as i32 + ((p.x * r) >> 16)).clamp(0, SCREEN_W as i32) as i16,
        y: (CY as i32 + ((p.y * r) >> 16)).clamp(0, SCREEN_H as i32) as i16,
        z: p.z,
    }
}

#[inline]
fn pack_clip_color(p: ClipVert) -> u32 {
    p.r.clamp(0, 255) as u32
        | ((p.g.clamp(0, 255) as u32) << 8)
        | ((p.b.clamp(0, 255) as u32) << 16)
}

#[inline]
fn pack_clip_uv(p: ClipVert) -> u32 {
    ((p.u + 128) >> 8).clamp(0, 255) as u32
        | ((((p.v + 128) >> 8).clamp(0, 255) as u32) << 8)
}

/// Clip one block-sized terrain cell against the near/far and display planes,
/// then fan-triangulate the surviving convex polygon. The fan pivot is the
/// vertex whose depth is closest to the rest, matching the diagonal choice
/// that measured best on real PS1 affine texture interpolation.
#[allow(clippy::too_many_arguments)]
fn emit_clipped_cell(
    corners: [ClipVert; 4],
    win: u32,
    cl_hi: u32,
    tp_hi: u32,
    blended: bool,
    depth_bias: i32,
) {
    // Perimeter order for the PS1 quad convention (TL, TR, BL, BR).
    let mut a = [EMPTY_CLIP_VERT; CLIP_VERT_CAP];
    a[0] = corners[0];
    a[1] = corners[1];
    a[2] = corners[3];
    a[3] = corners[2];
    let mut b = [EMPTY_CLIP_VERT; CLIP_VERT_CAP];
    let mut n = 4usize;
    let mut plane = 0usize;
    while plane < 6 {
        // Most close block cells are wholly inside five of the six planes.
        // Avoid copying the polygon through a second array for an identity
        // clip; on the R3000 this is substantially dearer than the tests.
        let mut any_inside = false;
        let mut any_outside = false;
        let mut scan = 0usize;
        while scan < n {
            if clip_distance(a[scan], plane) >= 0 {
                any_inside = true;
            } else {
                any_outside = true;
            }
            scan += 1;
        }
        if !any_inside {
            return;
        }
        if !any_outside {
            plane += 1;
            continue;
        }
        n = clip_polygon_plane(&a, n, &mut b, plane);
        if n < 3 {
            return;
        }
        core::mem::swap(&mut a, &mut b);
        plane += 1;
    }

    let mut pivot = 0usize;
    let mut best = i32::MAX;
    let mut candidate = 0usize;
    while candidate < n {
        let mut worst = 0i32;
        let mut j = 0usize;
        while j < n {
            worst = worst.max((a[candidate].z - a[j].z).abs());
            j += 1;
        }
        if worst < best {
            best = worst;
            pivot = candidate;
        }
        candidate += 1;
    }

    // Project the polygon ONCE, then grow it a pixel outward from its centre:
    // the same T-junction seal emit_face applies to distant plates, which this
    // path never had. The near band is FULL of independently truncated
    // boundaries -- refined 1-block cells beside 4-block coarse patches, the
    // plate/shell handoff, shell cubes against clipped plate continuations --
    // and each showed as dashed sky-coloured hairlines right in front of the
    // camera. Same-texture overlap is invisible; transparent cells keep exact
    // edges because overlapping two Average-blended quads double-blends.
    let mut pp = [Proj { x: 0, y: 0, z: 0 }; CLIP_VERT_CAP];
    let mut sx = 0i32;
    let mut sy = 0i32;
    let mut i = 0usize;
    while i < n {
        pp[i] = clip_project(a[i]);
        sx += pp[i].x as i32;
        sy += pp[i].y as i32;
        i += 1;
    }
    if !blended {
        let nn = n as i32;
        let mut i = 0usize;
        while i < n {
            let p = &mut pp[i];
            if (p.x as i32) * nn >= sx {
                if p.x < 1022 {
                    p.x += 2;
                }
            } else if p.x > -1022 {
                p.x -= 2;
            }
            if (p.y as i32) * nn >= sy {
                if p.y < 510 {
                    p.y += 2;
                }
            } else if p.y > -510 {
                p.y -= 2;
            }
            i += 1;
        }
    }

    let command = if blended { 0x3600_0000 } else { 0x3400_0000 };
    let mut fan = 1usize;
    while fan + 1 < n {
        let tri_n = unsafe { NEAR_TRI_N };
        if tri_n >= MAX_NEAR_TRIS {
            return;
        }
        let c0 = a[pivot];
        let c1 = a[(pivot + fan) % n];
        let c2 = a[(pivot + fan + 1) % n];
        let p0 = pp[pivot];
        let p1 = pp[(pivot + fan) % n];
        let p2 = pp[(pivot + fan + 1) % n];
        let area = (p1.x as i32 - p0.x as i32) * (p2.y as i32 - p0.y as i32)
            - (p1.y as i32 - p0.y as i32) * (p2.x as i32 - p0.x as i32);
        if area != 0 {
            let slot = depth_slot(((p0.z + p1.z + p2.z) / 3 - depth_bias).max(NEAR_Z));
            unsafe {
                let q = &mut NEAR_TRIS[RENDER_ARENA][tri_n];
                q.tex_window = win;
                q.color0_cmd = command | pack_clip_color(c0);
                q.v0 = p0.x as u16 as u32 | ((p0.y as u16 as u32) << 16);
                q.uv0_clut = pack_clip_uv(c0) | cl_hi;
                q.color1 = pack_clip_color(c1);
                q.v1 = p1.x as u16 as u32 | ((p1.y as u16 as u32) << 16);
                q.uv1_tpage = pack_clip_uv(c1) | tp_hi;
                q.color2 = pack_clip_color(c2);
                q.v2 = p2.x as u16 as u32 | ((p2.y as u16 as u32) << 16);
                q.uv2 = pack_clip_uv(c2);
                OT[RENDER_ARENA].insert(
                    slot,
                    q as *mut TriTexturedGouraud as *mut u32,
                    TriTexturedGouraud::WORDS,
                );
                NEAR_TRI_N = tri_n + 1;
            }
        }
        fan += 1;
    }
}

#[inline]
fn ao_factor(level: u8) -> i32 {
    match level & 3 {
        3 => 256,
        2 => 208,
        1 => 160,
        _ => 128,
    }
}

#[inline]
fn tint_factor(rgb: u32, factor: i32) -> (i32, i32, i32) {
    (
        ((rgb & 0xFF) as i32 * factor) >> 8,
        (((rgb >> 8) & 0xFF) as i32 * factor) >> 8,
        (((rgb >> 16) & 0xFF) as i32 * factor) >> 8,
    )
}

#[inline]
fn camera_space_world(cam: &Camera, p: (i32, i32, i32)) -> (i32, i32, i32) {
    let rows = unsafe { CAM_ROWS };
    let d = (p.0 - cam.x, p.1 - cam.y, p.2 - cam.z);
    let dot = |row: [i32; 3]| -> i32 {
        (row[0] * d.0 + row[1] * d.1 + row[2] * d.2) >> 12
    };
    (dot(rows[0]), dot(rows[1]), dot(rows[2]))
}

#[inline]
fn near_shell_meshable(b: u8) -> bool {
    b != AIR && b != DOOR_O && !world::is_cross_plant(b) && !is_small_block(b)
}

#[inline]
fn near_shell_see_through(b: u8) -> bool {
    b == AIR || is_water(b) || b == DOOR_O || world::is_cross_plant(b) || is_small_block(b)
}

/// Authoritative close-range block shell, independent of the greedy mesh.
///
/// The fast mesh can merge a top face across the eye and later reject the
/// rectangle as a unit, leaving sky under the player. This bounded 3x3x4 pass
/// guarantees that every exposed face nearest the view exists as two clipped
/// triangles, even while a chunk remesh is pending or a large merged plate
/// straddles the view plane. Close cells outside this footprint remain owned
/// by emit_near_face, so the bounded pass never opens a handoff gap.
fn render_near_block_shell(cam: &Camera) {
    let cbx = world_to_block_x(cam.x);
    let cby = world_to_block_y(cam.y);
    let cbz = world_to_block_z(cam.z);
    let (shell_x0, shell_z0) = (cbx - 1, cbz - 1);
    // One packed-world read per cell, including a one-cell neighbour border.
    // Looking up six neighbours independently made this bounded pass 3x more
    // expensive than its triangle work on the R3000.
    let mut cells = [[[AIR; 5]; 5]; 6];
    let mut iy = 0usize;
    while iy < 6 {
        let mut iz = 0usize;
        while iz < 5 {
            let mut ix = 0usize;
            while ix < 5 {
                cells[iy][iz][ix] =
                    get_block_i32(
                        shell_x0 + ix as i32 - 1,
                        cby + iy as i32 - 3,
                        shell_z0 + iz as i32 - 1,
                    );
                ix += 1;
            }
            iz += 1;
        }
        iy += 1;
    }

    let mut iy = 1usize;
    while iy < 5 {
        let by = cby + iy as i32 - 3;
        let mut iz = 1usize;
        while iz < 4 {
            let bz = shell_z0 + iz as i32 - 1;
            let mut ix = 1usize;
            while ix < 4 {
                let bx = shell_x0 + ix as i32 - 1;
                let block = cells[iy][iz][ix];
                if near_shell_meshable(block) && !is_transparent(block) {
                    let x0 = bx * BLOCK;
                    let x1 = x0 + BLOCK;
                    let y0 = by * BLOCK;
                    let y1 = y0 + BLOCK;
                    let z0 = bz * BLOCK;
                    let z1 = z0 + BLOCK;
                    let mut dir = 0usize;
                    while dir < 6 {
                        let neighbour = match dir {
                            0 => cells[iy][iz][ix + 1],
                            1 => cells[iy][iz][ix - 1],
                            2 => cells[iy + 1][iz][ix],
                            3 => cells[iy - 1][iz][ix],
                            4 => cells[iy][iz + 1][ix],
                            _ => cells[iy][iz - 1][ix],
                        };
                        let facing = match dir {
                            0 => cam.x > x1,
                            1 => cam.x < x0,
                            2 => cam.y > y1,
                            3 => cam.y < y0,
                            4 => cam.z > z1,
                            _ => cam.z < z0,
                        };
                        if facing && neighbour != block && near_shell_see_through(neighbour) {
                            let world = match dir {
                                0 => [(x1, y1, z0), (x1, y1, z1), (x1, y0, z0), (x1, y0, z1)],
                                1 => [(x0, y1, z1), (x0, y1, z0), (x0, y0, z1), (x0, y0, z0)],
                                2 => [(x0, y1, z0), (x1, y1, z0), (x0, y1, z1), (x1, y1, z1)],
                                3 => [(x0, y0, z1), (x1, y0, z1), (x0, y0, z0), (x1, y0, z0)],
                                4 => [(x1, y1, z1), (x0, y1, z1), (x1, y0, z1), (x0, y0, z1)],
                                _ => [(x0, y1, z0), (x1, y1, z0), (x0, y0, z0), (x1, y0, z0)],
                            };
                            let camera = [
                                camera_space_world(cam, world[0]),
                                camera_space_world(cam, world[1]),
                                camera_space_world(cam, world[2]),
                                camera_space_world(cam, world[3]),
                            ];
                            let min_z = camera[0].2.min(camera[1].2).min(camera[2].2).min(camera[3].2);
                            // Keep the replacement shell through two block
                            // depths. A large greedy plate can be rejected as a
                            // unit before its nearest constituent cell reaches
                            // the GTE saturation band; limiting this to GTE_NEAR
                            // reopened a narrow underfoot gap on the real camera
                            // path.
                            if min_z >= NEAR_BLOCK_Z {
                                dir += 1;
                                continue;
                            }
                            let tile = face_tile(block, dir) as usize;
                            let win = unsafe { MAT_WIN[0][tile] };
                            let cl_hi = unsafe { MAT_CLUT_HI[0][tile] };
                            let tp_hi = unsafe { MAT_TPAGE_HI[0][tile] };
                            let row = unsafe { &MAT_CCMD[1][0][dir] };
                            let make = |index: usize, u: i32, v: i32| -> ClipVert {
                                let (x, y, z) = camera[index];
                                let rgb = row[fog_band(z)] & 0x00FF_FFFF;
                                ClipVert {
                                    x,
                                    y,
                                    z,
                                    u: u << 8,
                                    v: v << 8,
                                    r: (rgb & 0xFF) as i32,
                                    g: ((rgb >> 8) & 0xFF) as i32,
                                    b: ((rgb >> 16) & 0xFF) as i32,
                                }
                            };
                            emit_clipped_cell(
                                [
                                    make(0, 0, 0),
                                    make(1, 16, 0),
                                    make(2, 0, 16),
                                    make(3, 16, 16),
                                ],
                                win,
                                cl_hi,
                                tp_hi,
                                false,
                                BLOCK / 2,
                            );
                        }
                        dir += 1;
                    }
                }
                ix += 1;
            }
            iz += 1;
        }
        iy += 1;
    }
}

/// Replace one close greedy rectangle with block-sized cells and two triangles
/// per cell. Geometry that crosses the eye is clipped into a convex polygon
/// before projection; it is never represented by moving arbitrary quad
/// corners to the near plane.
#[allow(clippy::too_many_arguments)]
fn emit_near_face(
    verts: &[(i32, i32, i32); 4],
    dir: usize,
    w: usize,
    h: usize,
    ao: u8,
    ccmd_row: &[u32; FOG_BANDS],
    win: u32,
    cl_hi: u32,
    tp_hi: u32,
    blended: bool,
) {
    let (uc, vc) = if dir < 2 { (h, w) } else { (w, h) };
    let ao_corner = [
        ao_factor(ao as u8),
        ao_factor((ao >> 2) as u8),
        ao_factor((ao >> 4) as u8),
        ao_factor((ao >> 6) as u8),
    ];
    let denom = (uc * vc) as i32;
    // The greedy rectangle is an exact block grid, so compute its two grid
    // steps once. The former point() divided all three coordinates for every
    // emitted corner -- hundreds of integer divisions for a close 16x8 plate.
    let du = (
        (verts[1].0 - verts[0].0) / uc as i32,
        (verts[1].1 - verts[0].1) / uc as i32,
        (verts[1].2 - verts[0].2) / uc as i32,
    );
    let dv = (
        (verts[2].0 - verts[0].0) / vc as i32,
        (verts[2].1 - verts[0].1) / vc as i32,
        (verts[2].2 - verts[0].2) / vc as i32,
    );
    // Exact q12 camera-space planes for the whole block grid. Camera position
    // is affine across this face, so transforming every emitted corner with
    // nine fresh multiplies is redundant; base + du*u + dv*v is byte-identical
    // to camera_space_local(point(u,v)).
    let (rows, chdx, chdy, chdz) =
        unsafe { (CAM_ROWS, CH_DX, CH_DY, CH_DZ) };
    let local_base = (verts[0].0 + chdx, verts[0].1 + chdy, verts[0].2 + chdz);
    let plane_base = |row: [i32; 3]| -> i32 {
        row[0] * local_base.0 + row[1] * local_base.1 + row[2] * local_base.2
    };
    let plane_du = |row: [i32; 3]| -> i32 {
        row[0] * du.0 + row[1] * du.1 + row[2] * du.2
    };
    let plane_dv = |row: [i32; 3]| -> i32 {
        row[0] * dv.0 + row[1] * dv.1 + row[2] * dv.2
    };
    let camera_base = [plane_base(rows[0]), plane_base(rows[1]), plane_base(rows[2])];
    let camera_du = [plane_du(rows[0]), plane_du(rows[1]), plane_du(rows[2])];
    let camera_dv = [plane_dv(rows[0]), plane_dv(rows[1]), plane_dv(rows[2])];
    let grid_z = |u: usize, v: usize| -> i32 {
        (camera_base[2] + camera_du[2] * u as i32 + camera_dv[2] * v as i32) >> 12
    };
    let grid_camera = |u: usize, v: usize| -> (i32, i32, i32) {
        let u = u as i32;
        let v = v as i32;
        (
            (camera_base[0] + camera_du[0] * u + camera_dv[0] * v) >> 12,
            (camera_base[1] + camera_du[1] * u + camera_dv[1] * v) >> 12,
            (camera_base[2] + camera_du[2] * u + camera_dv[2] * v) >> 12,
        )
    };
    let point = |u: usize, v: usize| -> (i32, i32, i32) {
        let u = u as i32;
        let v = v as i32;
        (
            verts[0].0 + du.0 * u + dv.0 * v,
            verts[0].1 + du.1 * u + dv.1 * v,
            verts[0].2 + du.2 * u + dv.2 * v,
        )
    };
    // Map a tessellated face cell back to its source block. The near shell is
    // deliberately bounded, so a depth test alone is not enough to hand it
    // ownership: at a diagonal screen edge a close cell can be outside the
    // shell footprint. Such a cell must remain on this clipped greedy path or
    // neither renderer emits it.
    let near_shell_owns = |u: usize, v: usize| -> bool {
        let p = point(u, v);
        let mut center = (
            p.0 + (du.0 + dv.0) / 2,
            p.1 + (du.1 + dv.1) / 2,
            p.2 + (du.2 + dv.2) / 2,
        );
        match dir {
            0 => center.0 -= BLOCK / 2,
            1 => center.0 += BLOCK / 2,
            2 => center.1 -= BLOCK / 2,
            3 => center.1 += BLOCK / 2,
            4 => center.2 -= BLOCK / 2,
            _ => center.2 += BLOCK / 2,
        }
        let (cbx, cby, cbz, chbx, chbz) =
            unsafe { (CAM_BX, CAM_BY, CAM_BZ, CH_BX, CH_BZ) };
        let bx = chbx + center.0 / BLOCK;
        let by = center.1 / BLOCK;
        let bz = chbz + center.2 / BLOCK;
        bx >= cbx - 1
            && bx <= cbx + 1
            && by >= cby - 2
            && by <= cby + 1
            && bz >= cbz - 1
            && bz <= cbz + 1
    };
    let factor = |u: usize, v: usize| -> i32 {
        let u0 = uc as i32 - u as i32;
        let v0 = vc as i32 - v as i32;
        (ao_corner[0] * u0 * v0
            + ao_corner[1] * u as i32 * v0
            + ao_corner[2] * u0 * v as i32
            + ao_corner[3] * u as i32 * v as i32)
            / denom
    };
    let vertex = |u: usize, v: usize, tu: i32, tv: i32| -> ClipVert {
        let (x, y, z) = grid_camera(u, v);
        let rgb = ccmd_row[fog_band(z)] & 0x00FF_FFFF;
        let (r, g, b) = tint_factor(rgb, factor(u, v));
        ClipVert { x, y, z, u: tu << 8, v: tv << 8, r, g, b }
    };

    // Start from the old, inexpensive 4x4-cell fallback, then refine only the
    // local patches that actually enter the GTE divide-saturation band. This
    // avoids turning the far end of one 16x8 greedy plate into 128 cells merely
    // because its nearest corner crosses the eye.
    const COARSE_PATCH: usize = 4;
    let mut v = 0usize;
    while v < vc {
        let v1 = (v + COARSE_PATCH).min(vc);
        let mut u = 0usize;
        while u < uc {
            let u1 = (u + COARSE_PATCH).min(uc);
            let coarse = [
                vertex(u, v, 0, 0),
                vertex(u1, v, ((u1 - u) * 16) as i32, 0),
                vertex(u, v1, 0, ((v1 - v) * 16) as i32),
                vertex(
                    u1,
                    v1,
                    ((u1 - u) * 16) as i32,
                    ((v1 - v) * 16) as i32,
                ),
            ];
            let coarse_min_z =
                coarse[0].z.min(coarse[1].z).min(coarse[2].z).min(coarse[3].z);
            // Reject the coarse patch before refining it. Close walls and
            // floors often have most of their merged plate behind the eye or
            // beyond one display edge; expanding those invisible regions into
            // sixteen cells was pure CPU work.
            let mut coarse_outside = false;
            let mut plane = 0usize;
            while plane < 6 {
                if clip_distance(coarse[0], plane) < 0
                    && clip_distance(coarse[1], plane) < 0
                    && clip_distance(coarse[2], plane) < 0
                    && clip_distance(coarse[3], plane) < 0
                {
                    coarse_outside = true;
                    break;
                }
                plane += 1;
            }
            if coarse_outside {
                u = u1;
                continue;
            }
            if coarse_min_z < NEAR_BLOCK_Z {
                let mut cv = v;
                while cv < v1 {
                    let mut cu = u;
                    while cu < u1 {
                        let cell_min_z = grid_z(cu, cv)
                            .min(grid_z(cu + 1, cv))
                            .min(grid_z(cu, cv + 1))
                            .min(grid_z(cu + 1, cv + 1));
                        // The bounded near shell emits these opaque source
                        // cubes authoritatively as individual faces after the
                        // greedy pass. Reject them before constructing four
                        // fully shaded vertices. Transparent terrain is not
                        // part of that shell and must still be emitted here.
                        if !blended
                            && cell_min_z < NEAR_BLOCK_Z
                            && near_shell_owns(cu, cv)
                        {
                            cu += 1;
                            continue;
                        }
                        let cell = [
                            vertex(cu, cv, 0, 0),
                            vertex(cu + 1, cv, 16, 0),
                            vertex(cu, cv + 1, 0, 16),
                            vertex(cu + 1, cv + 1, 16, 16),
                        ];
                        emit_clipped_cell(cell, win, cl_hi, tp_hi, blended, 0);
                        cu += 1;
                    }
                    cv += 1;
                }
            } else {
                emit_clipped_cell(coarse, win, cl_hi, tp_hi, blended, 0);
            }
            u = u1;
        }
        v = v1;
    }
}

/// inline(always): was outlined while for_visible_faces carried 640B of cull
/// tables; the MVMVA cull shrank that frame to 192B, so the ~60-cycle call ABI
/// per surviving face now costs more than the register pressure it saved.
/// (Re-tested 2026-07-30 after the body grew: outlining measured ~2% WORSE on
/// the route -- the note stands.)
#[inline(always)]
fn emit_face(block: u8, lx: i32, by: i32, lz: i32, dir: usize, w: usize, h: usize, light: usize, ao: u8, count: &mut usize) {
    if *count >= MAX_QUADS {
        return;
    }

    // A greedy-merged face spans w x h blocks. Which world axes (w,h) cover
    // depends on the face plane (matches world::cell_to_local):
    //   x-face (0,1): w->y, h->z ; y-face (2,3): w->x, h->z ; z-face (4,5): w->x, h->y
    let (ex, ey, ez) = match dir {
        0 | 1 => (1, w as i32, h as i32),
        2 | 3 => (w as i32, 1, h as i32),
        _ => (w as i32, h as i32, 1),
    };
    // CHUNK-LOCAL corner coordinates (gte_begin_chunk pointed the GTE's TR at
    // this chunk's origin): all values fit i16 raw, no per-corner subtraction.
    let x0 = lx * BLOCK;
    let x1 = x0 + ex * BLOCK;
    let y0 = by * BLOCK;
    let y1 = y0 + ey * BLOCK;
    let z0 = lz * BLOCK;
    let z1 = z0 + ez * BLOCK;

    let verts = match dir {
        0 => [(x1, y1, z0), (x1, y1, z1), (x1, y0, z0), (x1, y0, z1)],
        1 => [(x0, y1, z1), (x0, y1, z0), (x0, y0, z1), (x0, y0, z0)],
        2 => [(x0, y1, z0), (x1, y1, z0), (x0, y1, z1), (x1, y1, z1)],
        3 => [(x0, y0, z1), (x1, y0, z1), (x0, y0, z0), (x1, y0, z0)],
        4 => [(x1, y1, z1), (x0, y1, z1), (x1, y0, z1), (x0, y0, z1)],
        _ => [(x0, y1, z0), (x1, y1, z0), (x0, y0, z0), (x1, y0, z0)],
    };

    // Kick RTPT for corners 0..2, then do the material lookups WHILE the GTE
    // grinds through its 23-cycle flight -- overlapped work is free.
    // Corners go to COP2 CAMERA-RELATIVE (local + chunk-origin-minus-camera):
    // shared border corners get byte-identical values in both chunks, so no
    // cracks (the per-chunk-TR variant rounded differently per chunk).
    let (chdx, chdy, chdz) = unsafe { (CH_DX, CH_DY, CH_DZ) };
    let lv = |p: (i32, i32, i32)| {
        Vec3I16::new((p.0 + chdx) as i16, (p.1 + chdy) as i16, (p.2 + chdz) as i16)
    };
    let inflight = scene::rtpt_kick(lv(verts[0]), lv(verts[1]), lv(verts[2]));
    let tile = face_tile(block, dir) as usize;
    let (su, sv) = match dir {
        0 | 1 => ((h * 16) as u32, (w * 16) as u32),
        _ => ((w * 16) as u32, (h * 16) as u32),
    };
    let bl = is_transparent(block) as usize;
    let win = unsafe { MAT_WIN[bl][tile] };
    // Row now, band later: the depth is not known until the corners project,
    // and this lookup is deliberately here to overlap the GTE's flight time.
    let ccmd_row = unsafe { &MAT_CCMD[light][bl][dir] };
    let cl_hi = unsafe { MAT_CLUT_HI[bl][tile] };
    let tp_hi = unsafe { MAT_TPAGE_HI[bl][tile] };
    let t = inflight.read();
    // Kick RTPS for corner 3 and assemble corners 0..2 while its 15-cycle
    // divide grinds -- the read below lands after the op has settled.
    let v3 = lv(verts[3]);
    psx_gte::mtc2!(0, v3.xy_packed());
    psx_gte::mtc2!(1, v3.z_packed());
    // SAFETY: V0 just loaded; RT/TR/H/OFX/OFY set by gte_load_camera/begin_chunk.
    unsafe { psx_gte::ops::rtps() };
    let min_sz012 = t[0].sz.min(t[1].sz).min(t[2].sz);
    let q0 = Proj { x: t[0].sx, y: t[0].sy.clamp(-511, 511), z: t[0].sz as i32 };
    let q1 = Proj { x: t[1].sx, y: t[1].sy.clamp(-511, 511), z: t[1].sz as i32 };
    let q2 = Proj { x: t[2].sx, y: t[2].sy.clamp(-511, 511), z: t[2].sz as i32 };
    let sxy3 = psx_gte::mfc2!(14);
    let sz3 = psx_gte::mfc2!(19) as u16;
    let min_sz = min_sz012.min(sz3);
    // PS1 textures are affine inside each primitive. Hand the entire two-block
    // foreground band to the per-block shell before a large greedy plate can
    // overlay and deform those individual faces. The fallback retains only the
    // plate's farther continuation, clipped and locally tessellated.
    // Opaque long plates get the same treatment through the mid band (see
    // MID_SUBDIV_Z): blended terrain is excluded because water's near-uniform
    // texture doesn't show affine bend, and its plates are the biggest.
    // A leaves exclusion was tried on the same "noisy texture" argument and
    // measured NO win at the standing spawn profile (1,500K vs 1,486K loop
    // body, inside codegen noise) -- the close-plate cost there is not
    // canopy-dominated, so the branch was dropped.
    if min_sz < NEAR_BLOCK_Z as u16
        || (bl == 0
            && min_sz < MID_SUBDIV_Z as u16
            && w.max(h) >= MID_SUBDIV_SPAN
            && unsafe { NEAR_TRI_N } < MID_SUBDIV_TRI_CAP)
    {
        emit_near_face(
            &verts,
            dir,
            w,
            h,
            ao,
            ccmd_row,
            win,
            cl_hi,
            tp_hi,
            bl != 0,
        );
        return;
    }
    let mut p0 = q0;
    let mut p1 = q1;
    let mut p2 = q2;
    let mut p3 = Proj {
        x: sxy3 as i16,
        y: ((sxy3 >> 16) as i16).clamp(-511, 511),
        z: sz3 as i32,
    };

    // Seal T-junction hairlines. A greedy mesh is full of T-junctions: a large
    // merged quad's screen edge is subdivided by the corners of several smaller
    // neighbours, and every vertex truncates to integer screen coordinates on
    // its own, so such a corner lands up to a pixel off the large quad's
    // DDA-interpolated edge. Where it lands OUTSIDE, the sky shows through as a
    // hairline -- dashed, because the error varies along the edge.
    //
    // Growing the quad a pixel outward on ALL FOUR sides makes neighbours
    // overlap instead of gap; same-texture overlap is invisible. The previous
    // version moved only the corners sitting exactly at max-x/max-y, which is
    // ONE corner per axis once perspective and camera roll make the screen
    // edges diagonal: the expansion tapered to nothing along the rest of the
    // edge and sealed nothing. Measured on a dumped frame's real quad list,
    // rasterised through the GPU's own coverage rule: 20 interior holes with
    // that seal, 20 with no seal at all, 3 with this one.
    //
    // Opaque faces only. The transparent materials draw with
    // BlendMode::Average, and overlapping two blended quads double-blends into
    // a seam that reads worse than the hairline does.
    if bl == 0 {
        let sx = p0.x as i32 + p1.x as i32 + p2.x as i32 + p3.x as i32;
        let sy = p0.y as i32 + p1.y as i32 + p2.y as i32 + p3.y as i32;
        // Do NOT push a corner past the GPU's 11-bit screen rail: a corner the
        // SW near-fallback clamped to 1023 would become 1024, which the GPU
        // reads as -1024 (bit 10 is the sign), flipping it to the far side and
        // turning an eye-straddling face into a screen-spanning explosion.
        // 4*corner vs the corner SUM is the centre test without a divide.
        // TWO pixels, not one: adjacent plates truncate their shared edge's
        // vertices independently, so the relative error reaches ~2px -- and
        // when the neighbour is a near-band tessellated polygon it projects
        // through clip_project's RECIP table rather than the GTE divide, which
        // disagrees by another pixel. One pixel left dashed hairlines on flat
        // ground; two, on both paths, closed every truncation seam in the
        // orbit-vista audit. Same-texture overlap is invisible.
        let grow = |p: &mut Proj| {
            if (p.x as i32) * 4 >= sx {
                if p.x < 1022 {
                    p.x += 2;
                }
            } else if p.x > -1022 {
                p.x -= 2;
            }
            if (p.y as i32) * 4 >= sy {
                if p.y < 510 {
                    p.y += 2;
                }
            } else if p.y > -510 {
                p.y -= 2;
            }
        };
        grow(&mut p0);
        grow(&mut p1);
        grow(&mut p2);
        grow(&mut p3);
    }

    let min_x = min(min(p0.x, p1.x), min(p2.x, p3.x));
    let max_x = max(max(p0.x, p1.x), max(p2.x, p3.x));
    let min_y = min(min(p0.y, p1.y), min(p2.y, p3.y));
    let max_y = max(max(p0.y, p1.y), max(p2.y, p3.y));
    if max_x < -80 || min_x > 400 || max_y < -80 || min_y > 320 {
        return;
    }

    let depth = (p0.z + p1.z + p2.z + p3.z) >> 2;
    // The GTE has no far plane; match the old projector's far cull on the
    // face average (depth_slot clamps stragglers to the back OT slot).
    if depth >= FAR_Z {
        return;
    }
    let slot = depth_slot(depth);
    let band = fog_band(depth);
    let ccmd = ccmd_row[band];
    // Per-vertex fog bands. Fog was picked once from the face's AVERAGE depth
    // and applied flat, so a quad receding from the camera got ONE haze value
    // and its neighbour got another: an audit measured the ground jumping from
    // (198,198,181) hazy to (231,214,165) unfogged across two pixels, with
    // adjacent blocks in a terrace row in different bands, "a chequerboard of
    // hazy and clear cubes". Sampling each corner lets the GPU interpolate the
    // haze across the quad instead.
    let (b0, b1) = (fog_band(p0.z), fog_band(p1.z));
    let (b2, b3) = (fog_band(p2.z), fog_band(p3.z));
    let spans_fog = b0 != b1 || b0 != b2 || b0 != b3;
    // AO faces take the 13-word Gouraud packet; everything else keeps the
    // 9-data-word flat one.
    //
    // Past AO_BAND the face is small and fog is already washing it toward the
    // horizon colour, so the shading is not readable and it draws flat.
    if (AO_ON && ao != world::AO_LIT && band < AO_BAND) || spans_fog {
        let n = unsafe { AO_N };
        if n < MAX_AO_QUADS {
            unsafe {
                AO_N = n + 1;
                let q = &mut AO_QUADS[RENDER_ARENA][n];
                q.tex_window = win;
                // ccmd is a prepacked FLAT textured-quad header (0x2C/0x2E) with
                // the face tint in its low 24 bits. Bit 28 promotes it to
                // Gouraud (0x3C/0x3E); the tint becomes v0's colour and the
                // other three corners get words of their own.
                // Each corner takes its OWN fog band, then its own AO step.
                let rgb0 = ccmd_row[b0] & 0x00FF_FFFF;
                let rgb1 = ccmd_row[b1] & 0x00FF_FFFF;
                let rgb2 = ccmd_row[b2] & 0x00FF_FFFF;
                let rgb3 = ccmd_row[b3] & 0x00FF_FFFF;
                q.color0_cmd = (ccmd & 0xFF00_0000) | 0x1000_0000 | ao_shade(rgb0, ao as u32);
                q.v0 = (p0.x as u16 as u32) | ((p0.y as u16 as u32) << 16);
                q.uv0_clut = cl_hi;
                q.color1 = ao_shade(rgb1, (ao >> 2) as u32);
                q.v1 = (p1.x as u16 as u32) | ((p1.y as u16 as u32) << 16);
                q.uv1_tpage = tp_hi | su;
                q.color2 = ao_shade(rgb2, (ao >> 4) as u32);
                q.v2 = (p2.x as u16 as u32) | ((p2.y as u16 as u32) << 16);
                q.uv2 = sv << 8;
                q.color3 = ao_shade(rgb3, (ao >> 6) as u32);
                q.v3 = (p3.x as u16 as u32) | ((p3.y as u16 as u32) << 16);
                q.uv3 = (sv << 8) | su;
                OT[RENDER_ARENA].insert(
                    slot,
                    q as *mut QuadTexturedGouraud as *mut u32,
                    QuadTexturedGouraud::WORDS,
                );
            }
            *count += 1;
            return;
        }
        // Pool full: fall through and draw it flat rather than dropping it.
    }
    // Raw in-place packet stores from the prepacked tables: the builder path
    // (window + material + with_material + copy) measured ~600 cyc/face.
    unsafe {
        let q = &mut QUADS[RENDER_ARENA][*count];
        q.tex_window = win;
        q.color_cmd = ccmd;
        q.v0 = (p0.x as u16 as u32) | ((p0.y as u16 as u32) << 16);
        q.uv0_clut = cl_hi;
        q.v1 = (p1.x as u16 as u32) | ((p1.y as u16 as u32) << 16);
        q.uv1_tpage = tp_hi | su;
        q.v2 = (p2.x as u16 as u32) | ((p2.y as u16 as u32) << 16);
        q.uv2 = sv << 8;
        q.v3 = (p3.x as u16 as u32) | ((p3.y as u16 as u32) << 16);
        q.uv3 = (sv << 8) | su;
        OT[RENDER_ARENA].insert(
            slot,
            q as *mut QuadTexturedMaterial as *mut u32,
            QuadTexturedMaterial::WORDS,
        );
    }
    *count += 1;
}

/// Software projection for a quad straddling the eye, from CAMERA-RELATIVE
/// corner deltas. A corner at or behind the near plane is slid ALONG THE
/// SEGMENT to a corner that is in front until it reaches the near plane.
///
/// The old path clamped instead: it kept the corner's camera-space x/y and
/// pinned z to NEAR_Z. A corner BEHIND the eye therefore came back mirrored
/// onto the opposite side of the screen at the nearest possible depth, which
/// both sheared the quad across the display AND dragged the face's average
/// depth into OT slot 0, the frontmost layer. That is the "something behind me
/// is covering the screen" artifact: stand in a tree and the trunk's top face
/// paints over the whole canopy.
///
/// Sliding along the segment preserves direction, so a corner can never wrap to
/// the far side, and the quad stays inside the silhouette it should have. It is
/// not a true near-plane clip (that turns a quad into a pentagon, which this GPU
/// cannot take), but it is bounded and it leaves no hole -- unlike dropping the
/// whole face, which punches a gap in terrain you are standing against.
///
/// `None` when every corner is at/behind the eye: there is nothing to draw.
fn clip_quad_near(cam: &Camera, d: &[(i32, i32, i32); 4]) -> Option<[Proj; 4]> {
    let mut cx4 = [0i32; 4];
    let mut cy4 = [0i32; 4];
    let mut cz4 = [0i32; 4];
    let mut front: Option<usize> = None;
    let mut i = 0;
    while i < 4 {
        let (dx, dy, dz) = d[i];
        let x1 = ((dx * cam.cy) - (dz * cam.sy)) >> 12;
        let z1 = ((dx * cam.sy) + (dz * cam.cy)) >> 12;
        cx4[i] = x1;
        cy4[i] = ((dy * cam.cp) - (z1 * cam.sp)) >> 12;
        cz4[i] = ((dy * cam.sp) + (z1 * cam.cp)) >> 12;
        if cz4[i] >= NEAR_Z && front.is_none() {
            front = Some(i);
        }
        i += 1;
    }
    let f = front?;
    let mut out = [Proj { x: 0, y: 0, z: 0 }; 4];
    let mut i = 0;
    while i < 4 {
        let (mut x, mut y, mut z) = (cx4[i], cy4[i], cz4[i]);
        if z < NEAR_Z {
            let num = NEAR_Z - z;
            let den = cz4[f] - z; // > 0: cz4[f] >= NEAR_Z > z
            x += (cx4[f] - x) * num / den;
            y += (cy4[f] - y) * num / den;
            z = NEAR_Z;
        }
        let zc = if z > FAR_Z { FAR_Z } else { z };
        let r = unsafe { RECIP[zc as usize] };
        out[i] = Proj {
            x: (CX as i32 + ((x * r) >> 16)).clamp(-1023, 1023) as i16,
            y: (CY as i32 - ((y * r) >> 16)).clamp(-511, 511) as i16,
            z: zc,
        };
        i += 1;
    }
    Some(out)
}

/// Load the camera into the GTE (COP2) once per frame: rotation = pitch*yaw
/// with the Y row negated (GTE computes `OFY + IR2*n`; our screen Y grows
/// downward, `CY - y2*r`), translation zero (vertices are camera-relative),
/// screen offset at centre, projection plane H = PROJ_H. After this every
/// projected vertex is one hardware RTPS instead of ~10 CPU multiplies.
fn gte_load_camera(cam: &Camera) {
    let (sy, cy, sp, cp) = (cam.sy, cam.cy, cam.sp, cam.cp);
    let r = [
        [cy, 0, -sy],
        [(sp * sy) >> 12, -cp, (sp * cy) >> 12],
        [(cp * sy) >> 12, sp, (cp * cy) >> 12],
    ];
    // Roll about the view axis: the walk bob's sway and the hurt tilt. Applied
    // as a post-rotation of the first two rows, which is 8 multiplies once per
    // frame -- the camera used to be a rigid tripod with no roll term at all.
    let r = if cam.roll != 0 {
        let a = (cam.roll & 0x0FFF) as u16;
        let (sr, cr) = (sincos::sin_q12(a), sincos::cos_q12(a));
        [
            [
                (cr * r[0][0] - sr * r[1][0]) >> 12,
                (cr * r[0][1] - sr * r[1][1]) >> 12,
                (cr * r[0][2] - sr * r[1][2]) >> 12,
            ],
            [
                (sr * r[0][0] + cr * r[1][0]) >> 12,
                (sr * r[0][1] + cr * r[1][1]) >> 12,
                (sr * r[0][2] + cr * r[1][2]) >> 12,
            ],
            r[2],
        ]
    } else {
        r
    };
    let m = Mat3I16 {
        m: [
            [r[0][0] as i16, 0, r[0][2] as i16],
            [r[1][0] as i16, r[1][1] as i16, r[1][2] as i16],
            [r[2][0] as i16, r[2][1] as i16, r[2][2] as i16],
        ],
    };
    unsafe {
        CAM_ROWS = r; // reused by gte_begin_chunk for per-chunk translations
        CAM_BX = world_to_block_x(cam.x);
        CAM_BY = world_to_block_y(cam.y);
        CAM_BZ = world_to_block_z(cam.z);
    }
    scene::load_rotation(&m);
    scene::load_translation(Vec3I32::new(0, 0, 0));
    scene::set_screen_offset((CX as i32) << 16, (CY as i32) << 16);
    scene::set_projection_plane(PROJ_H as u16);
}

// Rotation rows (q12 i32) mirrored from gte_load_camera, plus the current
// chunk's camera-relative origin (set by gte_begin_chunk; the software
// near-fallback reconstructs camera-relative corners from it).
static mut CAM_ROWS: [[i32; 3]; 3] = [[0; 3]; 3];
static mut CAM_BX: i32 = 0;
static mut CAM_BY: i32 = 0;
static mut CAM_BZ: i32 = 0;
static mut CH_DX: i32 = 0;
static mut CH_DY: i32 = 0;
static mut CH_DZ: i32 = 0;
static mut CH_BX: i32 = 0;
static mut CH_BZ: i32 = 0;

/// Note a chunk's camera-relative origin for emit_face's corner math. The GTE
/// translation stays ZERO and corners are made camera-relative on the CPU:
/// a per-chunk TR = R*(origin-cam) was tried and REVERTED -- its pre-floored
/// rounding differed per chunk by up to 1 world unit, so shared edges across
/// chunk borders projected 1px apart (visible cracks all over the terrain).
/// Camera-relative corners give byte-identical values on both sides of a
/// border, so shared edges project identically.
fn gte_begin_chunk(cam: &Camera, oxw: i32, ozw: i32) {
    unsafe {
        CH_DX = oxw - cam.x;
        CH_DY = -cam.y;
        CH_DZ = ozw - cam.z;
        CH_BX = oxw / BLOCK;
        CH_BZ = ozw / BLOCK;
    }
}

/// Below this camera-space Z the GTE's H/SZ3 divide saturates (SZ3 <= H/2) and
/// the projection warps toward the screen centre; behind-the-eye corners clamp
/// SZ to 0. Quads with such a corner re-project on the software path, which
/// near-clamps like the pre-GTE renderer did. Only geometry touching the
/// camera lands here (a handful of faces per frame).
const GTE_NEAR: u16 = 96;
/// Through this full two-block band, opaque terrain ownership belongs to the
/// authoritative per-block shell: one independently clipped face per cube.
const NEAR_BLOCK_Z: i32 = 2 * BLOCK;

/// One-level mid-band subdivision for large greedy plates (the camera-space
/// subdivided affine technique from the Quake work, expressed in this
/// renderer's own tessellator). A merged plate drawn as ONE affine quad
/// interpolates UVs linearly in screen space, so a long floor or wall whose
/// depth varies a lot across the primitive bends and swims its texture. The
/// near band already fixes the worst of it; this extends the fix outward:
/// a plate at least MID_SUBDIV_SPAN blocks long whose nearest projected
/// corner is inside MID_SUBDIV_Z routes through emit_near_face, whose 4x4
/// coarse patches interpolate position/UV/tint in camera space BEFORE the
/// perspective divide (per-block refinement still only happens inside
/// NEAR_BLOCK_Z, so the mid band costs 4x4 patches, not cells).
/// Small faces stay on the one-quad path: their depth variation is bounded
/// by their size, so the affine error is already below notice.
///
/// Threshold measured at the fixed meadow vista (make profile, VISTA knobs),
/// loop-body cycles/frame vs baseline 1,573,431: 4 blocks +36,770 (+2.3%),
/// 6 blocks +102,464 (+6.5%). Frame-diff at the same vista: 4 blocks covers
/// ~75% of the pixels 6 blocks changes; the remainder is a thin far strip
/// where plates are already small on screen. 4 blocks is the keep.
const MID_SUBDIV_Z: i32 = 4 * BLOCK;
const MID_SUBDIV_SPAN: usize = 4;
/// Graceful degrade: leave headroom in the shared NEAR_TRIS pool so mid-band
/// subdivision can never starve the authoritative near band of packets. Past
/// this watermark a mid-band plate draws as the plain single quad again.
const MID_SUBDIV_TRI_CAP: usize = MAX_NEAR_TRIS - 256;

/// TEMP A/B toggle: route project_quad_gte through the software projector
/// instead of COP2, for profiler comparisons. Ship = true.
const USE_GTE: bool = true;

// "Vertex explosion" guard. A billboard the camera is inside/against projects
// with its near edge blown up to span most of the screen; a fistful of them (a
// meadow of grass + flowers) overlaps into a coloured mess -- the "vertex
// explosion". Two defences: render_plants distance-culls plants you're standing
// on (NEAR_PLANT_D2), and this SPAN test rejects any quad still spanning far
// more than one sane close billboard could (backstop, borrowed from hl-psx's
// guard-band idea as a resolution-independent span check).
const MAX_QUAD_SPAN: i16 = 460;
// Horizontal distance (squared, world units) below which a plant is culled --
// you're standing on it and its billboard would blow up. ~1.6 blocks.
const NEAR_PLANT_D2: i32 = 104 * 104;
/// Past this a cross sprite is a couple of pixels. The chunk-level gate in
/// world::for_plants reaches further now, for slabs and fences.
const PLANT_SPRITE_FAR_D2: i32 = (11 * BLOCK) * (11 * BLOCK);

#[inline]
fn quad_exploded(p: &[Proj; 4]) -> bool {
    let (mut lo_x, mut hi_x, mut lo_y, mut hi_y) = (i16::MAX, i16::MIN, i16::MAX, i16::MIN);
    let mut i = 0;
    while i < 4 {
        lo_x = lo_x.min(p[i].x);
        hi_x = hi_x.max(p[i].x);
        lo_y = lo_y.min(p[i].y);
        hi_y = hi_y.max(p[i].y);
        i += 1;
    }
    (hi_x - lo_x) > MAX_QUAD_SPAN || (hi_y - lo_y) > MAX_QUAD_SPAN
}

/// Project a quad's 4 corners on the GTE: one RTPT (3 vertices) + one RTPS.
/// Corners are made camera-relative on the CPU because world coordinates
/// exceed the GTE's i16 vertex range; relative ones stay well inside it
/// (detailed terrain <= ~3K units, imposters <= ~10K; i16 caps IMP_R at ~30).
/// inline(always): outlined, the 20-register ABI save/restore cost ~1000
/// cycles per call -- 25x the 38-cycle GTE work (2026-07-02 profile).
#[inline(always)]
fn project_quad_gte(cam: &Camera, verts: &[(i32, i32, i32); 4]) -> Option<[Proj; 4]> {
    if !USE_GTE {
        return project_quad_sw(cam, verts);
    }
    let rel = |p: (i32, i32, i32)| {
        Vec3I16::new((p.0 - cam.x) as i16, (p.1 - cam.y) as i16, (p.2 - cam.z) as i16)
    };
    let t = scene::project_triangle_scheduled(rel(verts[0]), rel(verts[1]), rel(verts[2]));
    let p3 = scene::project_vertex_scheduled(rel(verts[3]));
    let min_sz = t[0].sz.min(t[1].sz).min(t[2].sz).min(p3.sz);
    if min_sz < GTE_NEAR {
        return project_quad_sw(cam, verts);
    }
    // Clamp Y like the software path: the GPU drops primitives taller than
    // 511, and GTE saturation alone allows +-1023.
    Some([
        Proj { x: t[0].sx, y: t[0].sy.clamp(-511, 511), z: t[0].sz as i32 },
        Proj { x: t[1].sx, y: t[1].sy.clamp(-511, 511), z: t[1].sz as i32 },
        Proj { x: t[2].sx, y: t[2].sy.clamp(-511, 511), z: t[2].sz as i32 },
        Proj { x: p3.sx, y: p3.sy.clamp(-511, 511), z: p3.sz as i32 },
    ])
}

/// Cold software path: quads with a corner inside GTE_NEAR (straddling the
/// eye) and the USE_GTE=false A/B route. Deliberately NOT inlined so the hot
/// GTE path stays small enough to live inside emit_face.
#[inline(never)]
fn project_quad_sw(cam: &Camera, verts: &[(i32, i32, i32); 4]) -> Option<[Proj; 4]> {
    #[cfg(feature = "emulator-telemetry")]
    telemetry::counter(62, 1);
    let d = [
        (verts[0].0 - cam.x, verts[0].1 - cam.y, verts[0].2 - cam.z),
        (verts[1].0 - cam.x, verts[1].1 - cam.y, verts[1].2 - cam.z),
        (verts[2].0 - cam.x, verts[2].1 - cam.y, verts[2].2 - cam.z),
        (verts[3].0 - cam.x, verts[3].1 - cam.y, verts[3].2 - cam.z),
    ];
    clip_quad_near(cam, &d)
}

fn project_world(cam: &Camera, p: (i32, i32, i32)) -> Option<Proj> {
    let dx = p.0 - cam.x;
    let dy = p.1 - cam.y;
    let dz = p.2 - cam.z;

    let x1 = ((dx * cam.cy) - (dz * cam.sy)) >> 12;
    let z1 = ((dx * cam.sy) + (dz * cam.cy)) >> 12;
    let y2 = ((dy * cam.cp) - (z1 * cam.sp)) >> 12;
    let z2 = ((dy * cam.sp) + (z1 * cam.cp)) >> 12;
    // Match gte_load_camera's post-rotation around the view axis. The pick and
    // break overlays use this software projection while terrain uses the GTE;
    // omitting roll made their cube visibly drift off the selected block during
    // walk bob and hurt tilt.
    let (xr, yr) = if cam.roll != 0 {
        let a = (cam.roll & 0x0FFF) as u16;
        let (sr, cr) = (sincos::sin_q12(a), sincos::cos_q12(a));
        (((cr * x1) + (sr * y2)) >> 12, ((cr * y2) - (sr * x1)) >> 12)
    } else {
        (x1, y2)
    };

    // Cull only past the far plane. A vertex nearer than the near plane -- INCLUDING
    // one BEHIND the eye (z2 <= 0) -- is clamped to the near plane and still drawn,
    // never culled. A greedy-merged floor/wall face straddles the eye when you stand
    // on/next to it; culling on one behind corner drops the whole face and punches a
    // hole in the close terrain. Faces ENTIRELY behind are already removed upstream by
    // the frustum cull (`zf + fr < 0`), so clamping here can't draw behind-you garbage.
    if z2 > FAR_Z {
        return None;
    }
    let zc = if z2 < NEAR_Z { NEAR_Z } else { z2 };

    // 1/z from the table -> two multiplies + shifts instead of two divides.
    let r = unsafe { RECIP[zc as usize] };
    Some(Proj {
        x: (CX as i32 + ((xr * r) >> 16)).clamp(-1023, 1023) as i16,
        y: (CY as i32 - ((yr * r) >> 16)).clamp(-511, 511) as i16,
        z: zc,
    })
}

// ---- Sky: gradient backdrop + sun/moon billboards + night stars + clouds ----

/// Project a sky DIRECTION (Q12 unit-ish vector) to the screen, ignoring the
/// world FAR_Z clamp. `None` if behind the camera or far off-axis.
/// Returns the screen position and the camera-space depth `z2`. Callers that
/// draw something with real extent (clouds) need z2 to size it in perspective;
/// point-like billboards (sun, moon, stars) ignore it.
fn project_dir(cam: &Camera, dx: i32, dy: i32, dz: i32) -> Option<(i16, i16, i32)> {
    let x1 = (dx * cam.cy - dz * cam.sy) >> 12;
    let z1 = (dx * cam.sy + dz * cam.cy) >> 12;
    let y2 = (dy * cam.cp - z1 * cam.sp) >> 12;
    let z2 = (dy * cam.sp + z1 * cam.cp) >> 12;
    if z2 < 320 {
        return None; // behind the eye, or within ~85deg of the view edge
    }
    let sx = CX as i32 + x1 * PROJ_H / z2;
    let sy = CY as i32 - y2 * PROJ_H / z2;
    if !(-90..=410).contains(&sx) || !(-90..=330).contains(&sy) {
        return None;
    }
    Some((sx as i16, sy as i16, z2))
}

/// A camera-facing flat square drawn with 50% blending -- the sun's horizon
/// glow, which has to sit ON the sky rather than punch a hole in it.
#[allow(dead_code)] // no caller: see the sun-glow note in draw_sky
fn billboard_blend(cx: i16, cy: i16, hw: i16, hh: i16, col: (u8, u8, u8)) {
    // An octagon, not a rectangle. Nesting rectangles nests BOXES: an audit
    // measured the result as a three-step luminance staircase with three
    // traceable vertical edges, every step far above the 15-bit quantum. A
    // fan of eight triangles costs four more primitives on a GPU measured at
    // 17-35% busy and has no long straight edge to trace, so the same stack of
    // Average blends reads as a disc falling off instead of cards stacked up.
    // Octagon corner offset: 0.4142 (tan 22.5 deg), i.e. 53/128.
    let (ox, oy) = ((hw as i32 * 53 / 128) as i16, (hh as i32 * 53 / 128) as i16);
    let pts: [(i16, i16); 8] = [
        (cx - ox, cy - hh),
        (cx + ox, cy - hh),
        (cx + hw, cy - oy),
        (cx + hw, cy + oy),
        (cx + ox, cy + hh),
        (cx - ox, cy + hh),
        (cx - hw, cy + oy),
        (cx - hw, cy - oy),
    ];
    let mut i = 0usize;
    while i < 6 {
        ui_tri_blend([pts[0], pts[i + 1], pts[i + 2]], col.0, col.1, col.2);
        i += 1;
    }
}

/// A camera-facing flat square (sun, moon, star).
fn billboard(cx: i16, cy: i16, hw: i16, hh: i16, col: (u8, u8, u8)) {
    ui_quad_flat(
        [(cx - hw, cy - hh), (cx + hw, cy - hh), (cx - hw, cy + hh), (cx + hw, cy + hh)],
        col.0,
        col.1,
        col.2,
    );
}

/// A direction on the celestial sphere from azimuth/elevation (Q12 angles).
fn sky_dir(az: u16, el: u16) -> (i32, i32, i32) {
    let ce = sincos::cos_q12(el);
    (
        (ce * sincos::sin_q12(az)) >> 12,
        sincos::sin_q12(el),
        (ce * sincos::cos_q12(az)) >> 12,
    )
}

/// Horizontal half-angle of the view, in Q12 units (4096 = full turn). The
/// screen is 320 wide with the projection plane at PROJ_H, so the edge of the
/// screen is atan(160/PROJ_H) off-axis -- about 42 degrees.
const SKY_HALF_FOV: i32 = 478;
/// Columns and rows in the sky mesh. 4x3 quads replaces the old 2 triangles at
/// the same total fill, and buys per-azimuth colour, which is what makes a
/// sunset a BAND IN THE SUN'S DIRECTION rather than a global orange wash.
const SKY_COLS: usize = 4;

/// Draw the sky behind the world: a gouraud dome, the sun and moon arcing over
/// the day, night stars, and a drifting cloud band. Must run before the world
/// OT is submitted so terrain paints over it.
#[inline(never)]
/// `day` is the world clock, not a frame count: the moon phase below counts
/// days off it, and sleeping has to advance the moon.
fn draw_sky(cam: &Camera, day: u32, tod: u32, light: u8, horizon: (u8, u8, u8), raining: bool) {
    let h = SCREEN_H as i16;
    // Zenith is tracked in its own right, not derived from the horizon. The old
    // code scaled the horizon down by fixed ratios, so a warm sunset horizon
    // dragged the entire dome to brown and dawn came out mauve.
    let lf = light as i32 - NIGHT_LIGHT;
    let ld = 128 - NIGHT_LIGHT;
    let zenith = if raining {
        (
            (horizon.0 as u32 * 88 / 100) as u8,
            (horizon.1 as u32 * 90 / 100) as u8,
            (horizon.2 as u32 * 94 / 100) as u8,
        )
    } else {
        (
            lerp_u8(SKY_NIGHT_ZENITH.0, SKY_DAY_ZENITH.0, lf, ld),
            lerp_u8(SKY_NIGHT_ZENITH.1, SKY_DAY_ZENITH.1, lf, ld),
            lerp_u8(SKY_NIGHT_ZENITH.2, SKY_DAY_ZENITH.2, lf, ld),
        )
    };

    // Anchor the pale haze to the REAL horizon line instead of the bottom of the
    // screen. Looking level, the old gradient spent three quarters of its range
    // on rows the terrain covers, so the visible skyline was nearly pure zenith
    // and met the ground with no haze at all.
    let hy = if cam.cp.abs() > 64 {
        (CY as i32 + cam.sp * PROJ_H / cam.cp).clamp(-200, 440)
    } else {
        CY as i32
    };

    // Sun azimuth: the disc always sits in the x=0 plane, so its horizontal
    // direction is simply the sign of `c` below. Warmth at a given screen column
    // is the sunset strength scaled by how much that column faces the sun.
    let t = tod as i32;
    let phi = 1024 + 4096 * (t - 2 * DAY_LEN as i32 / 10) / DAY_LEN as i32;
    let sun_c = sincos::cos_q12(phi as u16);
    let warmth = sunset_warmth(tod, raining);

    let mut col: [(u8, u8, u8); SKY_COLS + 1] = [horizon; SKY_COLS + 1];
    let mut i = 0usize;
    while i <= SKY_COLS {
        let off = (i as i32 - SKY_COLS as i32 / 2) * 2 * SKY_HALF_FOV / SKY_COLS as i32;
        // cos(yaw + off), Q12: how much this column looks along +Z.
        let cs = sincos::cos_q12((off & 0x0FFF) as u16);
        let sn = sincos::sin_q12((off & 0x0FFF) as u16);
        let caz = (cam.cy * cs - cam.sy * sn) >> 12;
        // Dot with the sun's horizontal unit, which is +Z or -Z.
        let facing = if sun_c >= 0 { caz } else { -caz };
        let k = (warmth * facing.max(0)) >> 12;
        col[i] = (
            lerp_u8(horizon.0 as i32, SUNSET.0, k, 255),
            lerp_u8(horizon.1 as i32, SUNSET.1, k, 255),
            lerp_u8(horizon.2 as i32, SUNSET.2, k, 255),
        );
        i += 1;
    }

    // Three bands: zenith to a midway blend, midway to the haze at the horizon,
    // then haze continuing below (terrain covers most of it, but not when you
    // look up over a valley).
    let mid = |a: (u8, u8, u8), b: (u8, u8, u8)| {
        (
            ((a.0 as u16 + b.0 as u16) / 2) as u8,
            ((a.1 as u16 + b.1 as u16) / 2) as u8,
            ((a.2 as u16 + b.2 as u16) / 2) as u8,
        )
    };
    let rows: [i16; 4] = [
        0,
        (hy / 2).clamp(0, h as i32) as i16,
        hy.clamp(0, h as i32) as i16,
        h,
    ];
    // BELOW the horizon line the sky is GROUND HAZE, not more horizon blue.
    // Terrain covers most of that region, but not all of it: a sightline
    // grazing a dune crest onto ground beyond FAR_Z reveals whatever is
    // painted here as a sliver between two terrain faces. Horizon blue read
    // as a glitch line; a fade to dark shadow still read as a line against
    // pale sand. What a sightline past a crest WOULD hit is more distant
    // fogged ground, so paint that: the tone far terrain actually renders at
    // through the fog ramp (~(205,192,152) by day, skylight-scaled). The
    // blend from horizon colour completes within ~40px of the skyline --
    // slivers sit in that range or below, so they land on the haze itself,
    // which is within a few percent of the sunlit sand beside them.
    let haze = ground_haze(light);
    let rows: [i16; 5] = [
        rows[0],
        rows[1],
        rows[2],
        (hy + 14).clamp(0, h as i32) as i16,
        h,
    ];
    let mut r = 0usize;
    while r < 4 {
        let (y0, y1) = (rows[r], rows[r + 1]);
        if y1 <= y0 {
            r += 1;
            continue;
        }
        let mut c = 0usize;
        while c < SKY_COLS {
            let x0 = (c as i32 * SCREEN_W as i32 / SKY_COLS as i32) as i16;
            let x1 = ((c + 1) as i32 * SCREEN_W as i32 / SKY_COLS as i32) as i16;
            // Per-row colour: zenith at the top, the column's (possibly warmed)
            // horizon colour at the skyline, ground haze below it.
            let top = match r {
                0 => zenith,
                1 => mid(zenith, col[c]),
                2 => col[c],
                _ => haze,
            };
            let topr = match r {
                0 => zenith,
                1 => mid(zenith, col[c + 1]),
                2 => col[c + 1],
                _ => haze,
            };
            let bot = match r {
                0 => mid(zenith, col[c]),
                1 => col[c],
                _ => haze,
            };
            let botr = match r {
                0 => mid(zenith, col[c + 1]),
                1 => col[c + 1],
                _ => haze,
            };
            ui_tri_gouraud([(x0, y0), (x1, y0), (x0, y1)], [top, topr, bot]);
            ui_tri_gouraud([(x1, y0), (x0, y1), (x1, y1)], [topr, bot, botr]);
            c += 1;
        }
        r += 1;
    }

    // Sun and (opposite) moon arc through the north-south vertical plane, one
    // full turn per day; phi = 90deg (zenith) at noon.
    let t = tod as i32;
    let phi = 1024 + 4096 * (t - 2 * DAY_LEN as i32 / 10) / DAY_LEN as i32;
    let s = sincos::sin_q12(phi as u16); // elevation, Q12
    let c = sincos::cos_q12(phi as u16);
    // Overcast hides both: a sun disc burning through a grey rain sky was the
    // single most wrong-looking thing on screen. Sun/moon also track the
    // skylight now, so they dim into dusk instead of staying full-bright in an
    // almost-black sky.
    if !raining {
        if s > -500 {
            if let Some((x, y, _)) = project_dir(cam, 0, s, c) {
                // Java's sun stays a near-white pale disc right down to the
                // horizon; it does not dim with skylight. Around sunrise and
                // sunset it sits inside a warm glow, which is a real quad in
                // Java too, not an invention.
                // Warm glow around the low sun: a GOURAUD FAN, ten triangles
                // from a pale core vertex out to rim vertices carrying the sky's
                // own colour, so the hardware interpolates the falloff and the
                // rim is invisible against what it sits on. No blend mode in the
                // path at all.
                //
                // Worth recording how long this took. I "fixed" this five times
                // and each time measured no halo at any radius, so I blamed the
                // blend arithmetic, then the shape, then the packet pool, and
                // eventually deleted the feature as unfixable. All five
                // verifications were worthless: this whole block sits inside
                // `if let Some(..) = project_dir(..)`, the spawn faces yaw 0, and
                // the dusk sun is at azimuth 2048 -- directly BEHIND the camera.
                // Every capture I took was of the opposite half of the sky. A
                // magenta probe triangle at a fixed screen position read 0 pixels
                // facing away and 1,280 facing the sun, which is the test that
                // should have come first.
                if warmth > 0 {
                    let rim = col[SKY_COLS / 2];
                    let core = (
                        lerp_u8(rim.0 as i32, 255, warmth, 255),
                        lerp_u8(rim.1 as i32, 246, warmth, 255),
                        lerp_u8(rim.2 as i32, 205, warmth, 255),
                    );
                    const SEG: usize = 10;
                    let rr = 44i32 + (warmth * 28 / 255);
                    let mut k = 0usize;
                    while k < SEG {
                        let a0 = (k as i32 * 4096 / SEG as i32) as u16 & 0x0FFF;
                        let a1 = ((k + 1) as i32 * 4096 / SEG as i32) as u16 & 0x0FFF;
                        let p0 = (
                            x + ((sincos::cos_q12(a0) * rr) >> 12) as i16,
                            y + ((sincos::sin_q12(a0) * rr * 2 / 3) >> 12) as i16,
                        );
                        let p1 = (
                            x + ((sincos::cos_q12(a1) * rr) >> 12) as i16,
                            y + ((sincos::sin_q12(a1) * rr * 2 / 3) >> 12) as i16,
                        );
                        ui_tri_gouraud([(x, y), p0, p1], [core, rim, rim]);
                        k += 1;
                    }
                }
                billboard(x, y, 15, 15, (0xFF, 0xF4, 0xC8));
            }
        }
        if -s > -500 {
            if let Some((x, y, _)) = project_dir(cam, 0, -s, -c) {
                // The moon used to be scaled by `light` too, which at night is
                // 38/128 -- it rendered DARKER than the cloud slabs and was
                // genuinely hard to find in the sky. It is the brightest thing
                // up there in Java.
                billboard(x, y, 12, 12, (0xE8, 0xEA, 0xF4));
                // Phase: a sky-coloured occluder slid across the disc. Eight
                // phases over eight days, as in Java.
                let phase = ((tod / DAY_LEN.max(1)) % 8) as i32;
                let days = (day / DAY_LEN.max(1)) as i32;
                let ph = (days + phase) % 8;
                if ph != 0 {
                    let shift = ((ph - 4) * 6) as i16;
                    billboard(x + shift, y, 12, 12, zenith);
                }
            }
        }
    }

    // Stars fade in as the sky darkens. Fixed directions, pan with the camera.
    // ponytail: a hash spread, not a real star catalogue. 36 stars: 72 cost
    // ~23K cyc/frame (2 divides each in project_dir) -- the whole night-vs-day
    // delta that tipped frames past the 2-vblank 30fps line.
    // Gate on the TIME OF DAY, not on `light`: `light` carries the rain dimming,
    // so the old test lit up the whole star field during a midday shower.
    let nightness = 128i32 - day_brightness(tod) as i32;
    if !raining && nightness > 16 {
        let a = (nightness.min(96) * 255 / 96) as u8;
        let star = (a, a, a);
        let mut i = 0u32;
        while i < 36 {
            let hsh = i.wrapping_mul(2654435761);
            let (dx, dy, dz) = sky_dir((hsh & 0x0FFF) as u16, (170 + ((hsh >> 13) & 0x3FF)) as u16);
            if let Some((x, y, _)) = project_dir(cam, dx, dy, dz) {
                billboard(x, y, 1, 1, star);
            }
            i += 1;
        }
    }

    // A drifting band of white puffs high in the sky, dimmed by skylight. Drawn
    // OPAQUE (Java clouds read as solid white) -- the old Average blend washed
    // them to pale sky-blue. Widths vary a little for a fluffier band.
    let lit = light as u32;
    let cloud = (
        (0xF0 * lit / 128).min(255) as u8,
        (0xF2 * lit / 128).min(255) as u8,
        (0xF8 * lit / 128).min(255) as u8,
    );
    // Clouds sit on a flat layer overhead, so their screen size has to fall off
    // with distance: half-width scales with sin(elevation) over camera depth.
    // The old code drew a FIXED 22..40px half-width in every direction, which is
    // why they read as grey slabs pasted onto the sky rather than a receding
    // band. Clamped at the top so one drifting out past the edge of view cannot
    // blow up into a screen-filler.
    // Clouds live on a FLAT PLANE at a fixed altitude, as Java's do, and the
    // band drifts by translating that plane. The old code added the drift to
    // each puff's AZIMUTH, which orbited them around the camera like a carousel
    // -- they never crossed overhead, never sank toward the horizon and never
    // left the sky, and elevation was pinned to a narrow 13-23 degree band so
    // the whole lower sky was empty. Offsetting in world XZ instead makes them
    // converge toward the horizon for free, because project_dir is doing real
    // perspective on the offset vector.
    const CLOUD_H: i32 = 300; // altitude of the deck above the eye
    const CLOUD_SPAN: i32 = 4096; // how far the deck extends before it wraps
    let drift = (day as i32 * 3) % CLOUD_SPAN;
    let mut i = 0i32;
    while i < 34 {
        // A scattered but stable grid: two coprime strides so the cells do not
        // line up into rows.
        let ox = ((i * 617) % CLOUD_SPAN) - CLOUD_SPAN / 2;
        let oz = (((i * 971) + drift) % CLOUD_SPAN) - CLOUD_SPAN / 2;
        if let Some((x, y, z2)) = project_dir(cam, ox, CLOUD_H, oz) {
            // Screen size falls off with depth on its own now.
            let hw = ((22000 / z2.max(1)) + (i & 3) * 3).clamp(4, 110) as i16;
            let hh = (hw / 3).max(2);
            let step = if i & 1 == 0 { hw / 2 } else { -(hw / 2) };
            ui_quad_flat(
                [(x - hw, y - hh), (x + hw, y - hh), (x - hw, y + hh), (x + hw, y + hh)],
                cloud.0,
                cloud.1,
                cloud.2,
            );
            let (a, b) = (x + step - hw / 2, x + step + hw / 2);
            ui_quad_flat(
                [(a, y - hh * 2), (b, y - hh * 2), (a, y), (b, y)],
                cloud.0,
                cloud.1,
                cloud.2,
            );
        }
        i += 1;
    }
}

/// Depth-to-OT-slot for world geometry. Slot 0 is reserved for the HUD -- it
/// is the slot the DMA walker reaches last, so anything in it paints over
/// everything else -- which confines terrain to `1..=OT_LEN - 1`.
fn depth_slot(depth: i32) -> usize {
    if depth <= NEAR_Z {
        return 1;
    }
    if depth >= FAR_Z {
        return OT_LEN - 1;
    }
    1 + ((depth - NEAR_Z) as usize * (OT_LEN - 2)) / (FAR_Z - NEAR_Z) as usize
}

/// Voxel ray march for the crosshair target, Amanatides-Woo style: step to
/// whichever cell WALL the ray reaches next, so every cell the ray actually
/// passes through gets tested, in order.
///
/// This replaces a fixed 16-unit sample march. Sampling every quarter block
/// cannot skip a whole block along an axis, but it can cut a CORNER: where two
/// solid blocks meet diagonally, consecutive samples land in the two empty
/// diagonal cells and the ray slips between them into whatever is behind. That
/// needs a particular yaw and pitch to line up, which is exactly the itch
/// report of digging "through textures in specific positions of right analog
/// stick", and jagged mined-out cave walls are full of those diagonals.
///
/// It is also CHEAPER than the march it replaces -- at most 19 cell tests over
/// the 4.5-block reach against 18 fixed samples, and usually far fewer -- and
/// the reported place cell is now always face-adjacent to the hit, where the
/// sampler could hand back a diagonal neighbour.
#[inline(never)]
fn trace_pick(cam: &Camera) -> Pick {
    let dir = [(cam.sy * cam.cp) >> 12, cam.sp, (cam.cy * cam.cp) >> 12];
    let pos = [cam.x, cam.y, cam.z];
    let mut cell = [
        world_to_block_x(cam.x),
        world_to_block_y(cam.y),
        world_to_block_z(cam.z),
    ];
    // An axis the ray does not travel along never crosses a wall: park its
    // next-wall distance past the reach so the min-pick below ignores it.
    const NEVER: i32 = i32::MAX / 4;
    // Fractional bits on those distances. Whole world units are NOT enough
    // resolution: two walls a hundredth of a unit apart truncate to the same
    // integer, the tie-break then crosses them in the wrong order, and the ray
    // steps diagonally past the corner they share -- the exact tunnel this
    // rewrite exists to close, just 200x rarer. Ten bits costs nothing (the
    // divisions happen once, at setup) and keeps the worst accumulated value
    // near 2^28, well inside i32 and below NEVER.
    const T_FRAC: u32 = 10;
    let mut step = [0i32; 3];
    let mut t_wall = [NEVER; 3]; // distance to this axis' next cell wall
    let mut t_cell = [NEVER; 3]; // distance between walls on this axis
    let mut a = 0;
    while a < 3 {
        if dir[a] != 0 {
            let mag = dir[a].abs();
            let local = pos[a] - cell[a] * BLOCK; // 0..BLOCK-1 inside the cell
            let to_wall = if dir[a] > 0 { BLOCK - local } else { local };
            step[a] = if dir[a] > 0 { 1 } else { -1 };
            // dir is a Q12 unit vector, so <<12 puts these in world units.
            t_wall[a] = (to_wall << (12 + T_FRAC)) / mag;
            t_cell[a] = (BLOCK << (12 + T_FRAC)) / mag;
        }
        a += 1;
    }
    let reach = PICK_RANGE << T_FRAC;
    let mut place = cell;
    // A ray this long crosses at most ceil(reach)+1 walls per axis.
    let mut guard = 0;
    while guard <= 3 * (PICK_RANGE / BLOCK + 2) {
        if get_block_i32(cell[0], cell[1], cell[2]) != AIR {
            return Pick {
                hit: true,
                bx: cell[0],
                by: cell[1],
                bz: cell[2],
                px: place[0],
                py: place[1],
                pz: place[2],
            };
        }
        let a = if t_wall[0] <= t_wall[1] && t_wall[0] <= t_wall[2] {
            0
        } else if t_wall[1] <= t_wall[2] {
            1
        } else {
            2
        };
        if t_wall[a] > reach {
            break;
        }
        place = cell;
        cell[a] += step[a];
        t_wall[a] += t_cell[a];
        guard += 1;
    }
    NO_PICK
}

/// The destroy-stage crack, drawn over the face of the block being mined.
///
/// The pick ray reports the block AND the empty cell in front of it (px/py/pz),
/// so their difference is the face normal -- no extra ray work. Blended, and one
/// quad, on a GPU measured at 17-35% busy.
#[inline(never)]
fn draw_break_overlay(cam: &Camera, pick: Pick, progress: u32, den: u32) {
    if !pick.hit || den == 0 || progress == 0 {
        return;
    }
    let stage = ((progress * 4) / den).min(3) as u8;
    let tile = tex::T_CRACK0 + stage;
    let (bx, by, bz) = (pick.bx, pick.by, pick.bz);
    let x0 = block_to_world_x(bx);
    let x1 = x0 + BLOCK;
    let y0 = by * BLOCK;
    let y1 = y0 + BLOCK;
    let z0 = block_to_world_z(bz);
    let z1 = z0 + BLOCK;
    // Vanilla paints the destroy stage on the whole block, not one face; draw
    // every camera-facing face (up to three are visible on a cube).
    let mut dir = 0usize;
    while dir < 6 {
        let facing = match dir {
            0 => cam.x > x1,
            1 => cam.x < x0,
            2 => cam.y > y1,
            3 => cam.y < y0,
            4 => cam.z > z1,
            _ => cam.z < z0,
        };
        if facing {

            draw_break_face(cam, dir, x0, x1, y0, y1, z0, z1, tile);
        }
        dir += 1;
    }
}

/// One face of the destroy-stage overlay, lifted a unit off the surface so it
/// does not z-fight the block.
#[allow(clippy::too_many_arguments)]
fn draw_break_face(
    cam: &Camera,
    dir: usize,
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
    z0: i32,
    z1: i32,
    tile: u8,
) {
    const E: i32 = 1;
    let v = match dir {
        0 => [(x1 + E, y1, z0), (x1 + E, y1, z1), (x1 + E, y0, z0), (x1 + E, y0, z1)],
        1 => [(x0 - E, y1, z1), (x0 - E, y1, z0), (x0 - E, y0, z1), (x0 - E, y0, z0)],
        2 => [(x0, y1 + E, z0), (x1, y1 + E, z0), (x0, y1 + E, z1), (x1, y1 + E, z1)],
        3 => [(x0, y0 - E, z1), (x1, y0 - E, z1), (x0, y0 - E, z0), (x1, y0 - E, z0)],
        4 => [(x1, y1, z1 + E), (x0, y1, z1 + E), (x1, y0, z1 + E), (x0, y0, z1 + E)],
        _ => [(x0, y1, z0 - E), (x1, y1, z0 - E), (x0, y0, z0 - E), (x1, y0, z0 - E)],
    };
    let mut pp = [(0i16, 0i16); 4];
    let mut i = 0;
    while i < 4 {
        let Some(p) = project_world(cam, v[i]) else {
            return;
        };
        if p.z <= NEAR_Z {
            return;
        }
        pp[i] = (p.x, p.y);
        i += 1;
    }
    let bt = unsafe { BLOCK_TEX };
    let (u, vv) = tex::tile_uv(tile);
    let win = TextureWindow::power_of_two_tile(u, vv, 16, 16);
    // OPAQUE, plain CLUT: index 0 is texel 0x0000, which the GPU never paints
    // even for opaque prims, so the block shows through between the fracture
    // lines while the lines themselves land full-strength. (An Average blend
    // here halved the contrast and the cracks vanished at 320x240; clut_alpha
    // is worse still -- its index 0 is semi-transparent black, a uniform
    // darken.)
    let mat = TextureMaterial::opaque(bt.clut[tile as usize], bt.tpage, (128, 128, 128))
        .with_texture_window(win);
    ui_quad_textured(pp, [(0, 0), (16, 0), (0, 16), (16, 16)], mat);
}

fn draw_pick_outline(cam: &Camera, pick: Pick) {
    if !pick.hit {
        return;
    }
    let x0 = block_to_world_x(pick.bx);
    let x1 = x0 + BLOCK;
    let y0 = pick.by * BLOCK;
    let y1 = y0 + BLOCK;
    let z0 = block_to_world_z(pick.bz);
    let z1 = z0 + BLOCK;
    let pts = [
        (x0, y0, z0),
        (x1, y0, z0),
        (x1, y1, z0),
        (x0, y1, z0),
        (x0, y0, z1),
        (x1, y0, z1),
        (x1, y1, z1),
        (x0, y1, z1),
    ];
    let edges: [(usize, usize); 12] = [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 0),
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ];
    for &(a, b) in &edges {
        let av = camera_space_world(cam, pts[a]);
        let bv = camera_space_world(cam, pts[b]);
        let va = ClipVert { x: av.0, y: av.1, z: av.2, ..EMPTY_CLIP_VERT };
        let vb = ClipVert { x: bv.0, y: bv.1, z: bv.2, ..EMPTY_CLIP_VERT };
        if let Some((ca, cb)) = clip_line_segment(va, vb) {
            let pa = clip_project(ca);
            let pb = clip_project(cb);
            ui_line(pa.x, pa.y, pb.x, pb.y, 12, 12, 14);
        }
    }
}

/// Bottom-centre hotbar: one slot per placeable block, the selected one framed.
/// Icons are the block's side tile drawn as a 16x16 sprite from the atlas.
const HOTBAR_VIS: usize = 9; // the 9 real hotbar slots, vanilla-style

// Bottom-HUD rows, stacked upward from the hotbar. The hotbar tray spans
// HUD_HOTBAR_Y-2 .. +22 and the selected slot's frame overhangs it by another
// 2px, so nothing may sit below HUD_XP_Y. These used to be open-coded as
// `SCREEN_H - 18 - 4 - n` in five places, which is how the XP bar ended up
// drawing into the tray.
const HUD_HOTBAR_Y: i16 = SCREEN_H as i16 - 22; // slot top row
const HUD_XP_Y: i16 = HUD_HOTBAR_Y - 6;         // XP bar (2px tall)
const HUD_ROW_Y: i16 = HUD_XP_Y - 10;           // hearts + hunger (7px)
const HUD_ROW2_Y: i16 = HUD_ROW_Y - 9;          // armour (7px)
const HUD_ROW3_Y: i16 = HUD_ROW2_Y - 9;         // air bubbles (5px)
const HUD_NAME_Y: i16 = HUD_ROW3_Y - 12;        // item-name popup, clear of every pip row
// Hotbar tray geometry, shared so the XP bar can line up with it.
const HOTBAR_SLOT: i16 = 18;
const HOTBAR_W: i16 = HOTBAR_VIS as i16 * HOTBAR_SLOT;
const HOTBAR_X0: i16 = (SCREEN_W as i16 - HOTBAR_W) / 2;

/// Scroll window over a list of `n` items showing `vis` rows, kept so `sel`
/// stays visible. Shared by the hotbar and all three menus -- every menu used to
/// draw its whole list from a fixed origin, so crafting ran 24 rows down to
/// y=476 and the chest 32 rows to y=554 on a 240px screen.
fn list_window(n: usize, vis: usize, sel: usize) -> usize {
    if n <= vis {
        0
    } else {
        sel.saturating_sub(vis / 2).min(n - vis)
    }
}

fn draw_hotbar(font: &FontAtlas, tool: (u8, u8)) {
    // Vanilla hotbar: 9 real slots. Only stacks you actually own appear (no
    // greyed-out catalogue -- the original never shows what you don't have),
    // empty slots are just chrome, and the fat white frame overhangs the
    // selected slot by a pixel like the real one.
    let bt = unsafe { BLOCK_TEX };
    let slot = HOTBAR_SLOT;
    let x0 = (SCREEN_W as i16 - HOTBAR_VIS as i16 * slot) / 2;
    let y0 = HUD_HOTBAR_Y;
    rect(x0 - 2, y0 - 2, HOTBAR_VIS as i16 * slot + 4, slot + 4, 24, 24, 30);
    let mut j = 0usize;
    while j < HOTBAR_VIS {
        let b = unsafe { HOTBAR[j] };
        let sx = x0 + j as i16 * slot;
        rect(sx, y0, 17, 17, 54, 54, 62); // slot face
        ui_line(sx, y0, sx + 16, y0, 30, 30, 36); // bevel: dark top
        ui_line(sx, y0, sx, y0 + 16, 30, 30, 36); // dark left
        ui_line(sx, y0 + 16, sx + 16, y0 + 16, 96, 96, 106); // light bottom
        ui_line(sx + 16, y0, sx + 16, y0 + 16, 96, 96, 106); // light right
        if b != AIR {
            let tile = face_tile(b, 0);
            let uv = tex::tile_uv(tile);
            let mat = TextureMaterial::opaque(bt.clut[tile as usize], bt.tpage, (128, 128, 128));
            ui_sprite(sx + 1, y0 + 1, 16, 16, uv, mat);
            // Stack count in the slot's top-right corner, shadowed so it
            // reads over any tile. Hidden at 1, like vanilla; capped at 99.
            // 0 IS shown: a menu craft can drain a stack while hotbar_sync
            // is paused, and an unnumbered icon would read as "one left".
            let cnt = unsafe { INV[b as usize] };
            if cnt != 1 {
                let c = if cnt > 99 { 99 } else { cnt };
                let d = [b'0' + (c / 10) as u8, b'0' + (c % 10) as u8];
                let from = if c >= 10 { 0 } else { 1 };
                let txt = unsafe { core::str::from_utf8_unchecked(&d[from..]) };
                let tx = sx + 16 - (2 - from as i16) * 8;
                ui_text(font, tx + 1, y0 + 2, txt, (0, 0, 0));
                ui_text(font, tx, y0 + 1, txt, (255, 255, 255));
            }
        }
        if j == unsafe { HOTBAR_SEL } {
            let (r, g, bl) = (245, 245, 245);
            // Double-thick white frame overhanging the slot by 1px.
            ui_line(sx - 1, y0 - 1, sx + 18, y0 - 1, r, g, bl);
            ui_line(sx - 1, y0 - 2, sx + 18, y0 - 2, r, g, bl);
            ui_line(sx - 1, y0 + 18, sx + 18, y0 + 18, r, g, bl);
            ui_line(sx - 1, y0 + 19, sx + 18, y0 + 19, r, g, bl);
            ui_line(sx - 1, y0 - 1, sx - 1, y0 + 18, r, g, bl);
            ui_line(sx - 2, y0 - 1, sx - 2, y0 + 18, r, g, bl);
            ui_line(sx + 18, y0 - 1, sx + 18, y0 + 18, r, g, bl);
            ui_line(sx + 19, y0 - 1, sx + 19, y0 + 18, r, g, bl);
        }
        j += 1;
    }
    // The equipped tool in its own slot left of the bar (Bedrock's offhand
    // position). Tools are not hotbar items here -- no durability, the best
    // crafted tier auto-equips -- so this slot is how the equip is SEEN.
    let tx = x0 - 24;
    rect(tx - 2, y0 - 2, 21, 21, 24, 24, 30);
    rect(tx, y0, 17, 17, 54, 54, 62);
    ui_line(tx, y0, tx + 16, y0, 30, 30, 36);
    ui_line(tx, y0, tx, y0 + 16, 30, 30, 36);
    ui_line(tx, y0 + 16, tx + 16, y0 + 16, 96, 96, 106);
    ui_line(tx + 16, y0, tx + 16, y0 + 16, 96, 96, 106);
    if tool.1 > 0 {
        let tile = tool_tile(tool.0);
        let uv = tex::tile_uv(tile);
        let mat = TextureMaterial::opaque(bt.clut[tile as usize], bt.tpage, tool_tint(tool.1));
        ui_sprite(tx + 1, y0 + 1, 16, 16, uv, mat);
    }
}

/// Ten heart pips, left of the hotbar (2 hp each, rounded up).
/// Experience bar (green) just above the hotbar, filling toward the next level.
fn draw_xp(xp: i32) {
    let prog = xp.rem_euclid(XP_PER_LEVEL);
    let bx = HOTBAR_X0;
    let bw = HOTBAR_W;
    let by = HUD_XP_Y;
    rect(bx, by, bw, 2, 28, 54, 28);
    let fill = (prog * bw as i32 / XP_PER_LEVEL) as i16;
    if fill > 0 {
        rect(bx, by, fill, 2, 110, 235, 70);
    }
}

/// Armour pips one row above the hearts: more pips + a brighter colour at higher
/// tiers (iron grey, diamond cyan). Hidden when unarmoured.
fn draw_armor(armor: u8) {
    if armor == 0 {
        return;
    }
    let (pips, col) = if armor >= 2 {
        (5i16, (120, 200, 220))
    } else {
        (3i16, (184, 184, 196))
    };
    let x0 = 8i16;
    let y = HUD_ROW2_Y;
    let mut i = 0i16;
    while i < pips {
        rect(x0 + i * 9, y, 7, 7, col.0, col.1, col.2);
        i += 1;
    }
}

fn draw_hearts(health: i32) {
    let total = (MAX_HEALTH / 2) as i16;
    let hearts = ((health + 1) / 2).clamp(0, MAX_HEALTH / 2) as i16;
    let x0 = 8i16;
    let y = HUD_ROW_Y;
    let mut i = 0i16;
    while i < total {
        let on = i < hearts;
        let (r, g, b) = if on { (208, 36, 36) } else { (46, 24, 24) };
        let x = x0 + i * 9;
        // Pixel heart: two 3-wide lobes, a full middle row, a tapering tip.
        rect(x, y, 3, 2, r, g, b);
        rect(x + 4, y, 3, 2, r, g, b);
        rect(x, y + 2, 7, 2, r, g, b);
        rect(x + 1, y + 4, 5, 1, r, g, b);
        rect(x + 2, y + 5, 3, 1, r, g, b);
        rect(x + 3, y + 6, 1, 1, r, g, b);
        i += 1;
    }
}

/// Ten hunger pips, right of the hotbar (2 food each, rounded up).
fn draw_food(food: i32) {
    let total = (MAX_FOOD / 2) as i16;
    let pips = ((food + 1) / 2).clamp(0, MAX_FOOD / 2) as i16;
    let x0 = SCREEN_W as i16 - 8 - total * 9;
    let y = HUD_ROW_Y;
    let mut i = 0i16;
    while i < total {
        let on = i < pips;
        let c = if on { (190, 130, 50) } else { (50, 38, 22) };
        rect(x0 + i * 9, y, 7, 7, c.0, c.1, c.2);
        i += 1;
    }
}

/// Whichever overlay is open, if any. Outlined out of the gameplay loop: MIPS
/// branches only reach +/-128KB and the loop is at that edge.
#[inline(never)]
fn draw_menu(font: &FontAtlas, menu: u8, sel: usize, chest_idx: usize, player: Player) {
    if menu == 1 {
        draw_crafting(font, sel);
    } else if menu == 2 {
        draw_chest(font, chest_idx, sel);
    } else if menu == 3 {
        draw_furnace(font, chest_idx, sel);
    } else if menu == MENU_OPTIONS {
        draw_options(font, sel, player);
    } else if menu == MENU_INV {
        draw_inventory(font, sel);
    } else if menu == MENU_DEAD {
        draw_death(font);
    }
}

/// Vanilla's death pall: the dimmed world under a red wash, YOU DIED, and the
/// one control that leaves it.
fn draw_death(font: &FontAtlas) {
    dim_screen();
    let (w, h) = (SCREEN_W as i16, SCREEN_H as i16);
    ui_tri_blend([(0, 0), (w, 0), (0, h)], 96, 8, 8);
    ui_tri_blend([(w, 0), (0, h), (w, h)], 96, 8, 8);
    draw_centered(font, 96, "YOU DIED", (0xFF, 0x58, 0x48));
    ui_text(font, (SCREEN_W as i16 - 64) / 2 + 1, 96, "YOU DIED", (0xFF, 0x58, 0x48));
    let bx = (SCREEN_W as i16 - 92) / 2;
    let nx = ui_badge(font, bx, 128, "X", PS_CROSS) + 3;
    ui_text(font, nx, 128, "RESPAWN", (0xE0, 0xE0, 0xE0));
}

/// Inventory overlay: every placeable with the carried count; Cross equips it
/// to the hand. The direct-pick answer to a hotbar that R1-cycles one at a time.
fn draw_inventory(font: &FontAtlas, sel: usize) {
    menu_frame(font, "INVENTORY");
    let hide = unsafe { INV_HIDE };
    let mut hx = hint_item(font, MENU_TEXT_X, MENU_HINT_Y, "X", PS_CROSS, "SELECT");
    hx = hint_item(font, hx, MENU_HINT_Y, "[]", PS_SQUARE, if hide { "ALL" } else { "OWNED" });
    hint_item(font, hx, MENU_HINT_Y, "O", PS_CIRCLE, "CLOSE");
    let mut list = [0u8; PLACEABLE.len()];
    let n = inv_list(&mut list);
    let vis = 8;
    let start = list_window(n, vis, sel);
    menu_scroll_hint(font, n, vis, start, MENU_HINT_X, MENU_ROWS_Y);
    // This panel lists PLACEABLES, and tools are not among them: they have no
    // durability and the best tier equips itself into the slot left of the
    // hotbar. Nothing said so, so a player on itch spent the session hunting
    // for a way to "equip my pickaxe" and concluded a full hotbar had blocked
    // it. One line in the footer, where the panel already has the room.
    draw_centered(font, 170, "TOOLS EQUIP THEMSELVES: NO SLOT", MC_INK);
    if n == 0 {
        ui_text(font, MENU_TEXT_X, MENU_ROWS_Y, "NOTHING YET: GO MINE!", MC_INK);
        return;
    }
    let mut j = 0;
    while j < vis && start + j < n {
        let i = start + j;
        let b = list[i];
        let y = menu_row(j, i == sel);
        let color = if i == sel { MC_LABEL_SEL } else { MC_LABEL };
        ui_text(font, MENU_TEXT_X, y, block_name(b), color);
        ui_text(font, 222, y, &decimal3(unsafe { INV[b as usize] }), color);
        j += 1;
    }
}

/// Crafting overlay: the current tab's recipes with the selected one marked,
/// craftable ones bright, and the selected recipe's cost on the bottom line.
/// Drawn over the dimmed world while the menu is open.
fn draw_crafting(font: &FontAtlas, sel: usize) {
    let hide = unsafe { CRAFT_HIDE };
    menu_frame(font, CRAFT_TAB_TITLE[unsafe { CRAFT_TAB }]);
    let mut hx = MENU_PANEL_X + 6;
    hx = ui_badge(font, hx, MENU_HINT_Y, "L1", PS_KEY) + 2;
    hx = ui_badge(font, hx, MENU_HINT_Y, "R1", PS_KEY) + 3;
    ui_text(font, hx, MENU_HINT_Y, "TAB", MC_INK);
    hx += 3 * 8 + 8;
    hx = hint_item(font, hx, MENU_HINT_Y, "T", PS_TRIANGLE, if hide { "ALL" } else { "HIDE" });
    hx = hint_item(font, hx, MENU_HINT_Y, "X", PS_CROSS, "MAKE");
    hint_item(font, hx, MENU_HINT_Y, "O", PS_CIRCLE, "CLOSE");
    let mut list = [0u8; RECIPES.len()];
    let n = craft_list(&mut list);
    let vis = 5; // 5 recipe rows; the bottom of the panel is the cost box
    let start = list_window(n, vis, sel);
    menu_scroll_hint(font, n, vis, start, MENU_HINT_X, MENU_ROWS_Y);
    if n == 0 {
        // The filter emptied the tab: say so rather than show a bare panel.
        ui_text(font, MENU_TEXT_X, MENU_ROWS_Y, "NOTHING CRAFTABLE HERE", MC_INK);
        return;
    }
    let mut j = 0;
    while j < vis && start + j < n {
        let i = start + j;
        let ri = list[i] as usize;
        let y = menu_row(j, i == sel);
        let color = if i == sel {
            MC_LABEL_SEL
        } else if craftable_here(ri) {
            MC_LABEL
        } else {
            MC_LABEL_OFF
        };
        ui_text(font, MENU_TEXT_X, y, RECIPES[ri].label, color);
        j += 1;
    }
    // The cost box: a recessed slot listing the selected recipe's
    // ingredients, one per line, red until the inventory covers it; the list
    // row above already names the recipe. A grey MAKES line carries the yield
    // when it is more than one.
    let r = &RECIPES[list[sel] as usize];
    let by = MENU_ROWS_Y + (5 * MENU_ROW_H) as i16 + 2;
    mc_slot(MENU_BTN_X, by, MENU_BTN_W, 50);
    let mut iy = by + 6;
    let mut k = 0;
    while k < r.n_in as usize {
        let have = unsafe { INV[r.in_item[k] as usize] } >= r.in_qty[k];
        let c = if have { MC_LABEL } else { (0xE0, 0x60, 0x60) };
        let qty = [b'0' + (r.in_qty[k] % 10) as u8]; // recipe quantities are all single-digit
        ui_text(font, MENU_TEXT_X, iy, unsafe { core::str::from_utf8_unchecked(&qty) }, c);
        ui_text(font, MENU_TEXT_X + 16, iy, block_name(r.in_item[k]), c);
        iy += 13;
        k += 1;
    }
    if !is_tool_recipe(r.out) && r.out != CRAFT_ARMOR && r.out_qty > 1 {
        let makes = [b'M', b'A', b'K', b'E', b'S', b' ', b'0' + (r.out_qty % 10) as u8];
        ui_text(font, MENU_TEXT_X, iy, unsafe { core::str::from_utf8_unchecked(&makes) }, MC_HINT);
    }
}

/// Scroll marker at the panel's right edge, level with the first row, when the
/// list runs past the window -- so a menu never silently hides two thirds of
/// itself. Beside the ROWS, not in the header: the chest and furnace header
/// strings reach x=288 and would have run straight into it.
fn menu_scroll_hint(font: &FontAtlas, n: usize, vis: usize, start: usize, x: i16, y: i16) {
    if n <= vis {
        return;
    }
    let above = start > 0;
    let below = start + vis < n;
    let mark = match (above, below) {
        (true, true) => "^v",
        (true, false) => "^",
        _ => "v",
    };
    ui_text(font, x, y, mark, (0x55, 0x55, 0x55));
}

// -- UI display list --------------------------------------------------------
//
// The sky and the whole HUD used to write GP0 directly. Anything that writes
// GP0 after an ordering table's DMA has been kicked pays for the GPU backlog:
// a profiling pass measured ~258K cycles/frame of pure stall, all of it
// absorbed by whichever tail draw happened to touch GP0 first. So the UI does
// not draw immediately any more -- every primitive is built as an OT packet in
// this arena and linked into a chain that goes out by DMA.
//
// Each arena has two chains:
//   SKY_OT  draw-mode state + sky
//   OT      world + plants + mobs + the whole tail
// Frame N's chains rasterise while the CPU builds frame N+1 in the other arena.
// No gameplay renderer writes GP0 directly during that build.
//
// `OrderingTable::clear` chains [N-1] -> ... -> [0] and submission starts at
// [N-1], so slot 0 draws LAST. That is the HUD's slot; `depth_slot` keeps
// world geometry in 1..=OT_LEN-1 so terrain can never interleave with it.
//
// Insertion prepends within a slot, so a flush walks its batch BACKWARDS: the
// first packet built ends up first in the chain, which is the painter order
// the immediate-mode code had.
// 7168, was 8192: the tail of each arena was headroom, and the RAM ceiling
// wanted it back when the settings card landed. ui_alloc drops a primitive,
// not the frame, if this ever binds.
const UI_WORDS: usize = 7168;
const UI_MAX_PACKETS: usize = 1536;
static mut UI_POOL: [[u32; UI_WORDS]; RENDER_ARENAS] = [[0; UI_WORDS]; RENDER_ARENAS];
static mut UI_POOL_N: usize = 0;
static mut UI_OFF: [u16; UI_MAX_PACKETS] = [0; UI_MAX_PACKETS];
static mut UI_WC: [u8; UI_MAX_PACKETS] = [0; UI_MAX_PACKETS];
static mut UI_N: usize = 0;
static mut UI_FLUSHED: usize = 0;

/// Start a fresh frame's UI list. Must run before the first sky primitive.
fn ui_reset() {
    unsafe {
        UI_POOL_N = 0;
        UI_N = 0;
        UI_FLUSHED = 0;
    }
}

/// Reserve one packet with `words` data words after its tag, returning the tag
/// pointer. Null when the arena is full: the primitive is then dropped, which
/// is a missing HUD pip rather than a corrupted DMA chain.
#[inline]
fn ui_alloc(words: usize) -> *mut u32 {
    unsafe {
        let off = UI_POOL_N;
        if off + words + 1 > UI_WORDS || UI_N >= UI_MAX_PACKETS {
            return core::ptr::null_mut();
        }
        UI_POOL_N = off + words + 1;
        UI_OFF[UI_N] = off as u16;
        UI_WC[UI_N] = words as u8;
        UI_N += 1;
        UI_POOL[RENDER_ARENA].as_mut_ptr().add(off)
    }
}

/// Link every packet built since the last flush into `ot`'s `slot`, build
/// order first.
///
/// `inline(never)` here and on the two wrappers below is not a hint: the
/// gameplay loop already sits at the edge of the MIPS +/-128KB branch range,
/// and the telemetry build fails to link ("out of range PC16 fixup") the
/// moment any of this is inlined into it.
#[inline(never)]
fn ui_flush<const N: usize>(ot: &mut OrderingTable<N>, slot: usize) {
    unsafe {
        let mut i = UI_N;
        while i > UI_FLUSHED {
            i -= 1;
            let p = UI_POOL[RENDER_ARENA].as_mut_ptr().add(UI_OFF[i] as usize);
            ot.insert(slot, p, UI_WC[i]);
        }
        UI_FLUSHED = UI_N;
    }
}

/// Reset both ordering tables and the UI arena for a new frame.
#[inline(never)]
fn ui_frame_begin() {
    unsafe {
        OT[RENDER_ARENA].clear();
        SKY_OT[RENDER_ARENA].clear();
    }
    ui_reset();
}

/// Link the sky batch. The title screen may submit it immediately because it
/// uses a synchronous renderer; gameplay leaves it queued until frame_present.
#[inline(never)]
fn ui_finish_sky(submit_now: bool) {
    unsafe {
        let arena = RENDER_ARENA;
        ui_flush(&mut (*core::ptr::addr_of_mut!(SKY_OT))[arena], 0);
        if submit_now && !PROFILE_SKIP_SUBMIT {
            SKY_OT[arena].submit();
        }
    }
}

/// Link the tail batch (particles, held item, rain, HUD, menus) into the slot
/// the DMA walker reaches last.
#[inline(never)]
fn ui_submit_tail() {
    unsafe {
        let arena = RENDER_ARENA;
        ui_flush(&mut (*core::ptr::addr_of_mut!(OT))[arena], 0);
    }
}

/// Submit the arena just built and switch CPU packet allocation to the other.
///
/// The second kick waits only for the sky's DMA walk, not its raster, so both
/// chains remain contiguous in the GPU command queue.
#[inline(never)]
fn submit_built_frame() {
    unsafe {
        let arena = RENDER_ARENA;
        SKY_OT[arena].submit_async();
        OT[arena].submit_async();
        RENDER_ARENA ^= 1;
    }
}

/// Present the completed GPU frame, then queue the arena the CPU just built.
/// After the one-frame prime, CPU frame N+1 overlaps GPU frame N. Packet RAM is
/// reusable as soon as its DMA walk finishes; rasterisation no longer reads it.
#[inline(never)]
fn frame_present(fb: &mut FrameBuffer, in_flight: &mut bool) {
    telemetry::stage_end(50);
    if PROFILE_SKIP_SUBMIT {
        if !PERF_FREERUN {
            wait_vblank();
        }
        telemetry::stage_begin(48);
        fb.swap();
        telemetry::stage_end(48);
        return;
    }

    // Prime without a flip: the title remains visible for this single build,
    // while the next gameplay frame immediately starts filling the other arena.
    if !*in_flight {
        submit_built_frame();
        *in_flight = true;
        return;
    }

    telemetry::stage_begin(47); // residual DMA walk + raster after CPU overlap
    gpu::submit_linked_list_wait();
    gpu::draw_sync();
    telemetry::stage_end(47);

    // The completed back buffer is only exposed at a VBlank boundary. swap()
    // also queues the draw-area state for the other VRAM half, before its OTs.
    if !PERF_FREERUN {
        wait_vblank();
    }
    telemetry::stage_begin(48);
    fb.swap();
    submit_built_frame();
    telemetry::stage_end(48);
}

#[inline(always)]
fn xy(x: i16, y: i16) -> u32 {
    (x as u16 as u32) | ((y as u16 as u32) << 16)
}

#[inline(always)]
fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

/// Flat-shaded quad (GP0 0x28).
fn ui_quad_flat(v: [(i16, i16); 4], r: u8, g: u8, b: u8) {
    let p = ui_alloc(5);
    if p.is_null() {
        return;
    }
    unsafe {
        *p.add(1) = 0x2800_0000 | rgb(r, g, b);
        *p.add(2) = xy(v[0].0, v[0].1);
        *p.add(3) = xy(v[1].0, v[1].1);
        *p.add(4) = xy(v[2].0, v[2].1);
        *p.add(5) = xy(v[3].0, v[3].1);
    }
}

/// Gouraud triangle (GP0 0x30).
fn ui_tri_gouraud(v: [(i16, i16); 3], c: [(u8, u8, u8); 3]) {
    let p = ui_alloc(6);
    if p.is_null() {
        return;
    }
    unsafe {
        *p.add(1) = 0x3000_0000 | rgb(c[0].0, c[0].1, c[0].2);
        *p.add(2) = xy(v[0].0, v[0].1);
        *p.add(3) = rgb(c[1].0, c[1].1, c[1].2);
        *p.add(4) = xy(v[1].0, v[1].1);
        *p.add(5) = rgb(c[2].0, c[2].1, c[2].2);
        *p.add(6) = xy(v[2].0, v[2].1);
    }
}

/// Half-blended flat triangle (GP0 0x22). Carries its own E1 draw-mode and E2
/// texture-window words, because the blend bits live in draw-mode state and an
/// OT chain has no "current material" the caller can set up beforehand.
fn ui_tri_blend(v: [(i16, i16); 3], r: u8, g: u8, b: u8) {
    let p = ui_alloc(6);
    if p.is_null() {
        return;
    }
    let mat = TextureMaterial::blended(0, 0, (r, g, b), BlendMode::Average);
    unsafe {
        *p.add(1) = mat.draw_mode_word();
        *p.add(2) = mat.texture_window_word();
        *p.add(3) = 0x2200_0000 | rgb(r, g, b);
        *p.add(4) = xy(v[0].0, v[0].1);
        *p.add(5) = xy(v[1].0, v[1].1);
        *p.add(6) = xy(v[2].0, v[2].1);
    }
}

/// Half-blended flat quad (GP0 0x2A), linked immediately into a world-depth
/// slot instead of joining the HUD batch at slot 0.
fn ui_quad_blend_depth(v: [(i16, i16); 4], r: u8, g: u8, b: u8, slot: usize) {
    let p = ui_alloc(7);
    if p.is_null() {
        return;
    }
    let mat = TextureMaterial::blended(0, 0, (r, g, b), BlendMode::Average);
    unsafe {
        *p.add(1) = mat.draw_mode_word();
        *p.add(2) = mat.texture_window_word();
        *p.add(3) = 0x2A00_0000 | rgb(r, g, b);
        *p.add(4) = xy(v[0].0, v[0].1);
        *p.add(5) = xy(v[1].0, v[1].1);
        *p.add(6) = xy(v[2].0, v[2].1);
        *p.add(7) = xy(v[3].0, v[3].1);
        let arena = RENDER_ARENA;
        ui_flush(&mut (*core::ptr::addr_of_mut!(OT))[arena], slot);
    }
}

/// Monochrome line (GP0 0x40).
fn ui_line(x0: i16, y0: i16, x1: i16, y1: i16, r: u8, g: u8, b: u8) {
    let p = ui_alloc(3);
    if p.is_null() {
        return;
    }
    unsafe {
        *p.add(1) = 0x4000_0000 | rgb(r, g, b);
        *p.add(2) = xy(x0, y0);
        *p.add(3) = xy(x1, y1);
    }
}

/// Variable-size textured sprite (GP0 0x64). Rect primitives read their tpage
/// from draw-mode state, so the packet leads with the material's E1 + E2 --
/// exactly what `gpu::draw_sprite_material` writes immediately.
fn ui_sprite(x: i16, y: i16, w: u16, h: u16, uv: (u8, u8), mat: TextureMaterial) {
    let p = ui_alloc(6);
    if p.is_null() {
        return;
    }
    unsafe {
        *p.add(1) = mat.draw_mode_word();
        *p.add(2) = mat.texture_window_word();
        *p.add(3) = mat.textured_rect_header();
        *p.add(4) = xy(x, y);
        *p.add(5) = (uv.0 as u32) | ((uv.1 as u32) << 8) | ((mat.clut_word() as u32) << 16);
        *p.add(6) = (w as u32) | ((h as u32) << 16);
    }
}

/// Textured quad with its own texture window (GP0 E2 + 0x2C).
fn ui_quad_textured(v: [(i16, i16); 4], uvs: [(u8, u8); 4], mat: TextureMaterial) {
    let p = ui_alloc(QuadTexturedMaterial::WORDS as usize);
    if p.is_null() {
        return;
    }
    let q = QuadTexturedMaterial::with_material(v, uvs, mat);
    let src = &q as *const QuadTexturedMaterial as *const u32;
    unsafe {
        let mut i = 1usize;
        while i <= QuadTexturedMaterial::WORDS as usize {
            *p.add(i) = *src.add(i);
            i += 1;
        }
    }
}

/// Text, as one GP0 0x64 sprite per glyph out of the font atlas.
///
/// `psx-font`'s `draw_text` is immediate-mode, so it cannot be used past the
/// OT kick. The atlas layout it uses is reproduced here: `glyphs_per_row` is
/// picked at upload time as `min(glyph_count, 256 / glyph_w)`.
fn ui_text(font: &FontAtlas, x: i16, y: i16, text: &str, tint: (u8, u8, u8)) {
    let f = font.font();
    let mat = TextureMaterial::opaque(FONT_CLUT.uv_clut_word(), FONT_TPAGE.uv_tpage_word(0), tint);
    // One draw-mode packet per run, matching draw_text's single tpage apply.
    let p = ui_alloc(2);
    if p.is_null() {
        return;
    }
    unsafe {
        *p.add(1) = mat.draw_mode_word();
        *p.add(2) = mat.texture_window_word();
    }
    let per_row = (f.glyph_count).min(256 / f.glyph_w as u16).max(1);
    let header = mat.textured_rect_header();
    let clut_hi = (mat.clut_word() as u32) << 16;
    let size = (f.glyph_w as u32) | ((f.glyph_h as u32) << 16);
    let mut cx = x;
    for ch in text.chars() {
        if let Some(idx) = f.glyph_index(ch) {
            let g = ui_alloc(4);
            if g.is_null() {
                return;
            }
            let u = (idx % per_row) * f.glyph_w as u16;
            let v = (idx / per_row) * f.glyph_h as u16;
            unsafe {
                *g.add(1) = header;
                *g.add(2) = xy(cx, y);
                *g.add(3) = (u as u32 & 0xFF) | ((v as u32 & 0xFF) << 8) | clut_hi;
                *g.add(4) = size;
            }
        }
        cx = cx.wrapping_add(f.glyph_advance(ch) as i16);
    }
}

/// Filled rectangle via a flat quad (polygon path). HUD must use this, not
/// `gpu::fill_rect`: GP0(0x02) fill ignores the draw offset and lands in the
/// wrong double-buffer half (flicker). The polygon path respects the offset.
fn rect(x: i16, y: i16, w: i16, h: i16, r: u8, g: u8, b: u8) {
    ui_quad_flat([(x, y), (x + w, y), (x, y + h), (x + w, y + h)], r, g, b);
}

// -- Minecraft GUI chrome ---------------------------------------------------
// The menus used to be flat blue-grey boxes, which looked nothing like the
// game they are imitating. Vanilla dims the world behind a GUI and draws its
// widgets from `widgets.png`: a black outline, a light top/left bevel, a dark
// bottom/right bevel, and a two-tone grey face that goes lighter on hover.

/// Row geometry shared by every menu. 8px glyphs on a 16px pitch leaves 2px of
/// air between buttons.
const MENU_ROW_H: usize = 16;
const MENU_ROWS_Y: i16 = 44;
const MENU_BTN_X: i16 = 34;
const MENU_BTN_W: i16 = 252;
const MENU_BTN_H: i16 = 14;
/// Text inset: past the outline and bevel, with room to breathe.
const MENU_TEXT_X: i16 = MENU_BTN_X + 8;
/// Scroll marker, clear of the button's right edge.
const MENU_HINT_X: i16 = MENU_BTN_X + MENU_BTN_W + 4;

const MC_LABEL: (u8, u8, u8) = (0xE0, 0xE0, 0xE0);
const MC_LABEL_SEL: (u8, u8, u8) = (0xFF, 0xFF, 0xA0);
const MC_LABEL_OFF: (u8, u8, u8) = (0x9E, 0x9E, 0x9E);
const MC_HINT: (u8, u8, u8) = (0xA8, 0xA8, 0xA8);

/// Half-blend black over the whole frame. Vanilla darkens the world rather than
/// hiding it behind an opaque panel, and the frozen world is still legible
/// through it.
fn dim_screen() {
    let (w, h) = (SCREEN_W as i16, SCREEN_H as i16);
    ui_tri_blend([(0, 0), (w, 0), (0, h)], 0, 0, 0);
    ui_tri_blend([(w, 0), (0, h), (w, h)], 0, 0, 0);
}

fn draw_centered(font: &FontAtlas, y: i16, s: &str, c: (u8, u8, u8)) {
    ui_text(font, (SCREEN_W as i16 - s.len() as i16 * 8) / 2, y, s, c);
}

// The dialog panel every menu sits on: vanilla's light-grey box with a black
// outline and an inset bevel, so text never floats over the raw world.
const MENU_PANEL_X: i16 = 24;
const MENU_PANEL_Y: i16 = 6;
const MENU_PANEL_W: i16 = 272;
const MENU_PANEL_H: i16 = 176;
/// Dark ink for text that sits on the panel face rather than on a button.
const MC_INK: (u8, u8, u8) = (0x3A, 0x3A, 0x3A);
// PlayStation glyph tints for the button badges.
const PS_CROSS: (u8, u8, u8) = (0x6E, 0x96, 0xF0);
const PS_CIRCLE: (u8, u8, u8) = (0xF0, 0x64, 0x5A);
const PS_TRIANGLE: (u8, u8, u8) = (0x46, 0xC8, 0x6E);
const PS_SQUARE: (u8, u8, u8) = (0xE8, 0x7E, 0xC8);
const PS_KEY: (u8, u8, u8) = (0xF0, 0xF0, 0xF0);

fn menu_panel() {
    let (x, y, w, h) = (MENU_PANEL_X, MENU_PANEL_Y, MENU_PANEL_W, MENU_PANEL_H);
    rect(x - 2, y - 2, w + 4, h + 4, 0, 0, 0); // outline
    rect(x, y, w, h, 0xC6, 0xC6, 0xC6);
    ui_line(x, y, x + w - 1, y, 0xFF, 0xFF, 0xFF);
    ui_line(x, y, x, y + h - 1, 0xFF, 0xFF, 0xFF);
    ui_line(x, y + h - 1, x + w - 1, y + h - 1, 0x55, 0x55, 0x55);
    ui_line(x + w - 1, y, x + w - 1, y + h - 1, 0x55, 0x55, 0x55);
}

/// A rounded dark pill with a button glyph in its PlayStation tint, the
/// console-edition control-hint look. Returns the x just past the pill.
fn ui_badge(font: &FontAtlas, x: i16, y: i16, key: &str, tint: (u8, u8, u8)) -> i16 {
    let w = key.len() as i16 * 8 + 7;
    // Two stacked rects, each 1px shy of the other's corners: rounded enough
    // at 8px glyph scale.
    rect(x + 1, y - 2, w - 2, 12, 0x22, 0x22, 0x22);
    rect(x, y - 1, w, 10, 0x22, 0x22, 0x22);
    ui_text(font, x + 4, y, key, tint);
    x + w
}

/// One control hint: badge + dark action label. Returns the next free x.
fn hint_item(font: &FontAtlas, x: i16, y: i16, key: &str, tint: (u8, u8, u8), action: &str) -> i16 {
    let nx = ui_badge(font, x, y, key, tint) + 3;
    ui_text(font, nx, y, action, MC_INK);
    nx + action.len() as i16 * 8 + 8
}

const MENU_HINT_Y: i16 = 26;

/// Dim + panel + centred title. Every menu opens with this, then lays its own
/// badge hints at MENU_HINT_Y.
fn menu_frame(font: &FontAtlas, title: &str) {
    dim_screen();
    menu_panel();
    draw_centered(font, 12, title, MC_INK);
}

/// A `widgets.png` button. `sel` is vanilla's hover state: lighter face, white
/// highlight instead of grey.
fn mc_button(y: i16, sel: bool) {
    let (x, w, h) = (MENU_BTN_X, MENU_BTN_W, MENU_BTN_H);
    rect(x, y, w, h, 0, 0, 0); // outline
    let (top, bot) = if sel { (0xA6, 0x8B) } else { (0x8B, 0x6E) };
    let half = h / 2;
    rect(x + 1, y + 1, w - 2, half - 1, top, top, top);
    rect(x + 1, y + half, w - 2, h - half - 1, bot, bot, bot);
    let hi = if sel { 0xFF } else { 0xC6 };
    ui_line(x + 1, y + 1, x + w - 2, y + 1, hi, hi, hi);
    ui_line(x + 1, y + 1, x + 1, y + h - 2, hi, hi, hi);
    ui_line(x + 1, y + h - 2, x + w - 2, y + h - 2, 0x37, 0x37, 0x37);
    ui_line(x + w - 2, y + 1, x + w - 2, y + h - 2, 0x37, 0x37, 0x37);
}

/// An item slot: the button bevel inverted (dark top/left, light bottom/right)
/// over a dark face, so it reads as a recess rather than something to press.
fn mc_slot(x: i16, y: i16, w: i16, h: i16) {
    rect(x, y, w, h, 0x37, 0x37, 0x37);
    ui_line(x, y, x + w - 1, y, 0x1E, 0x1E, 0x1E);
    ui_line(x, y, x, y + h - 1, 0x1E, 0x1E, 0x1E);
    ui_line(x, y + h - 1, x + w - 1, y + h - 1, 0xC6, 0xC6, 0xC6);
    ui_line(x + w - 1, y, x + w - 1, y + h - 1, 0xC6, 0xC6, 0xC6);
}

/// Top-left corner of row `j`'s text, with the button drawn under it. Text sits
/// 3px into a 14px button so the 8px glyphs land centred.
fn menu_row(j: usize, sel: bool) -> i16 {
    let y = MENU_ROWS_Y + (j * MENU_ROW_H) as i16;
    mc_button(y - 3, sel);
    y
}

/// Options menu (START). One row per entry; CROSS acts on the selected one.
/// Table-driven so another option is one line here plus one arm in the CROSS
/// handler.
const MENU_OPTIONS: u8 = 4;
const MENU_DEAD: u8 = 6;
/// TRIANGLE's inventory panel (Bedrock PS layout): pick any placeable directly
/// instead of R1-cycling the whole hotbar one item at a time.
const MENU_INV: u8 = 5;

/// Index of `sel` in PLACEABLE, so the inventory opens on the item in hand.
const OPT_FLIGHT: usize = 0;
const OPT_SAVE: usize = 1;
const OPT_LOAD: usize = 2;
const OPT_TUTORIAL: usize = 3;
/// Rows from OPT_SETTINGS on are the SETTINGS card's own rows, folded into
/// the in-game menu so stick feel and volume can be tuned mid-session
/// instead of only from the main menu before a world loads.
const OPT_SETTINGS: usize = 4;
const OPTIONS: [&str; OPT_SETTINGS + SETTING_ROWS] = [
    "FLIGHT",
    "SAVE TO CARD",
    "LOAD FROM CARD",
    "TUTORIAL",
    SETTING_NAMES[0],
    SETTING_NAMES[1],
    SETTING_NAMES[2],
    SETTING_NAMES[3],
    SETTING_NAMES[4],
];
/// Result of the last card operation, shown under the list.
static mut OPT_MSG: &str = "";

fn draw_options(font: &FontAtlas, sel: usize, player: Player) {
    menu_frame(font, "OPTIONS");
    let mut hx = hint_item(font, MENU_TEXT_X, MENU_HINT_Y, "X", PS_CROSS, "TOGGLE");
    hx = hint_item(font, hx, MENU_HINT_Y, "< >", PS_KEY, "ADJUST");
    hint_item(font, hx, MENU_HINT_Y, "O", PS_CIRCLE, "CLOSE");
    let n = OPTIONS.len();
    let vis = 8;
    let start = list_window(n, vis, sel);
    menu_scroll_hint(font, n, vis, start, MENU_HINT_X, MENU_ROWS_Y);
    let mut j = 0;
    while j < vis && start + j < n {
        let i = start + j;
        let y = menu_row(j, i == sel);
        let color = if i == sel { MC_LABEL_SEL } else { MC_LABEL };
        ui_text(font, MENU_TEXT_X, y, OPTIONS[i], color);
        if i == OPT_FLIGHT || i == OPT_TUTORIAL {
            // Vanilla writes the value into the button label ("Clouds: Fancy"),
            // right-aligned here so the states line up with future toggles.
            let on = if i == OPT_FLIGHT { player.fly } else { unsafe { TUT_ENABLED } };
            let (label, tint) = if on {
                ("ON", (0x70, 0xE0, 0x70))
            } else {
                ("OFF", MC_LABEL_OFF)
            };
            ui_text(font, 230, y, label, tint);
        } else if i >= OPT_SETTINGS {
            let mut buf = [0u8; 4];
            let k = setting_value(i - OPT_SETTINGS, &mut buf);
            let txt = unsafe { core::str::from_utf8_unchecked(&buf[..k]) };
            ui_text(font, 278 - k as i16 * 8, y, txt, (0x70, 0xE0, 0x70));
        }
        j += 1;
    }
    // Outcome of the last card operation: saving silently is worse than not
    // saving at all, because you only find out at the next power cycle.
    // Below the rows, not over them. (A line here used to advertise SELECT and
    // double-tap-CROSS as fly toggles; both were removed when they started
    // firing on menu-exit jumps, and the hint outlived them.)
    let msg = unsafe { OPT_MSG };
    if !msg.is_empty() {
        draw_centered(font, 170, msg, (0xF0, 0xE0, 0x80));
    }
}

/// Chest overlay: each placeable item with the count in the player inventory
/// (U) and in this chest (C); Cross deposits, Square withdraws.
fn draw_chest(font: &FontAtlas, idx: usize, sel: usize) {
    menu_frame(font, "CHEST");
    let mut hx = hint_item(font, MENU_TEXT_X, MENU_HINT_Y, "X", PS_CROSS, "PUT");
    hx = hint_item(font, hx, MENU_HINT_Y, "[]", PS_SQUARE, "TAKE");
    hint_item(font, hx, MENU_HINT_Y, "O", PS_CIRCLE, "CLOSE");
    let n = PLACEABLE.len();
    let vis = 8;
    let start = list_window(n, vis, sel);
    menu_scroll_hint(font, n, vis, start, MENU_HINT_X, MENU_ROWS_Y);
    let mut j = 0;
    while j < vis && start + j < n {
        let i = start + j;
        let b = PLACEABLE[i];
        let y = menu_row(j, i == sel);
        let color = if i == sel { MC_LABEL_SEL } else { MC_LABEL };
        ui_text(font, MENU_TEXT_X, y, block_name(b), color);
        let inv_c = decimal3(unsafe { INV[b as usize] });
        let ch_c = decimal3(unsafe { CHEST_INV[idx][b as usize] });
        ui_text(font, 166, y, "U", (0x90, 0xC0, 0x90));
        ui_text(font, 178, y, &inv_c, color);
        ui_text(font, 222, y, "C", (0x90, 0xC0, 0x90));
        ui_text(font, 234, y, &ch_c, color);
        j += 1;
    }
}

/// Furnace overlay: input / fuel / output slots, a smelt progress bar, and the
/// depositable items (ore + sand smelt; coal is fuel). Cross loads, Square takes.
fn draw_furnace(font: &FontAtlas, idx: usize, sel: usize) {
    menu_frame(font, "FURNACE");
    let mut hx = hint_item(font, MENU_TEXT_X, MENU_HINT_Y, "X", PS_CROSS, "PUT");
    hx = hint_item(font, hx, MENU_HINT_Y, "[]", PS_SQUARE, "TAKE");
    hint_item(font, hx, MENU_HINT_Y, "O", PS_CIRCLE, "CLOSE");
    let inn = unsafe { FURN_IN[idx] };
    let in_n = unsafe { FURN_IN_N[idx] };
    let fuel = unsafe { FURN_FUEL[idx] };
    let outt = unsafe { FURN_OUT[idx] };
    let out_n = unsafe { FURN_OUT_N[idx] };
    let prog = unsafe { FURN_PROG[idx] };

    let in_name = if inn == AIR { "--" } else { block_name(inn) };
    let out_name = if outt == AIR { "--" } else { block_name(outt) };
    // The three slots are read-outs, not choices, so they get vanilla's inset
    // slot bevel rather than a button.
    mc_slot(MENU_BTN_X, 41, MENU_BTN_W, 52);
    ui_text(font, MENU_TEXT_X, 44, "IN", MC_HINT);
    ui_text(font, MENU_TEXT_X + 48, 44, in_name, MC_LABEL);
    ui_text(font, 230, 44, &decimal3(in_n), MC_LABEL);
    ui_text(font, MENU_TEXT_X, 60, "FUEL", MC_HINT);
    ui_text(font, 230, 60, &decimal3(fuel), (0xF0, 0xC0, 0x50));
    ui_text(font, MENU_TEXT_X, 76, "OUT", MC_HINT);
    ui_text(font, MENU_TEXT_X + 48, 76, out_name, MC_LABEL);
    ui_text(font, 230, 76, &decimal3(out_n), MC_LABEL);
    // Smelt progress, the arrow in vanilla's furnace.
    mc_slot(MENU_BTN_X, 96, MENU_BTN_W, 9);
    let w = prog as i16 * (MENU_BTN_W - 4) / SMELT_TIME as i16;
    if w > 0 {
        rect(MENU_BTN_X + 2, 98, w, 5, 230, 140, 40);
    }

    let n = FURN_ITEMS.len();
    let vis = 4; // what fits between the progress bar and the HUD
    let start = list_window(n, vis, sel);
    menu_scroll_hint(font, n, vis, start, MENU_HINT_X, 112);
    let mut j = 0;
    while j < vis && start + j < n {
        let i = start + j;
        let b = FURN_ITEMS[i];
        let y = 112 + (j * MENU_ROW_H) as i16;
        mc_button(y - 3, i == sel);
        let color = if i == sel { MC_LABEL_SEL } else { MC_LABEL };
        ui_text(font, MENU_TEXT_X, y, block_name(b), color);
        ui_text(font, 200, y, "U", (0x90, 0xC0, 0x90));
        ui_text(font, 212, y, &decimal3(unsafe { INV[b as usize] }), color);
        j += 1;
    }
}


/// Rain streaks: short vertical lines at fixed columns, animated downward.
/// Rain. Two layers so it has depth: a dense, short, dim far layer and a
/// sparse, long, bright near one that falls faster. Leans with the camera yaw,
/// because 48 dead-vertical hairlines of identical length and brightness read
/// as static rather than weather.
#[inline(never)]
fn draw_rain(frame: u32, cam: &Camera, rain: i32) {
    // Shear the streaks with the view direction so turning sells the wind.
    let lean = (cam.sy * 5) >> 12;
    // Streak count follows the ramp: a shower thickens as it arrives and
    // thins as it leaves, instead of 190 lines appearing at once.
    let n = (190 * rain / 255).clamp(0, 190) as u32;
    let mut i = 0u32;
    while i < n {
        let h = i.wrapping_mul(2654435761) ^ (i << 7);
        let near = i & 3 == 0;
        let (len, speed, c) = if near {
            (14i16, 29u32, (185, 205, 240))
        } else {
            (7i16, 17u32, (120, 140, 180))
        };
        let x = (h % 320) as i16;
        let y = ((h >> 8).wrapping_add(frame.wrapping_mul(speed)) % 248) as i16 - 8;
        ui_line(x, y, x + lean as i16, y + len, c.0, c.1, c.2);
        i += 1;
    }
}

/// True if open sky sits directly above the player. Rain used to fall inside
/// caves and under roofs because the draw was gated on the weather alone.
#[inline(never)]
fn under_open_sky(player: &Player) -> bool {
    let bx = world_to_block_x(player.x);
    let bz = world_to_block_z(player.z);
    let mut y = world_to_block_y(player.y) + 2;
    while y < world::CH {
        let b = get_block_i32(bx, y, bz);
        if b != AIR && !world::is_cross_plant(b) {
            return false;
        }
        y += 1;
    }
    true
}

/// Underwater blue, lava orange, and the red flash of taking a hit. Java tints
/// the screen underwater and fills it with orange in lava; the damage flash is
/// Bedrock's rather than Java's, but it reads on a CRT and the fill is free.
#[inline(never)]
fn screen_tint(player: &Player) {
    let bx = world_to_block_x(player.x);
    let bz = world_to_block_z(player.z);
    let head = get_block_i32(bx, world_to_block_y(player.y + EYE_HEIGHT), bz);
    let tint = if is_lava(head) {
        Some((220, 90, 20))
    } else if is_water(head) {
        Some((30, 80, 190))
    } else if player.hurt_tilt > HURT_TILT_FRAMES - 4 {
        Some((190, 30, 30))
    } else {
        None
    };
    if let Some(c) = tint {
        let (w, h) = (SCREEN_W as i16, SCREEN_H as i16);
        ui_tri_blend([(0, 0), (w, 0), (0, h)], c.0, c.1, c.2);
        ui_tri_blend([(w, 0), (0, h), (w, h)], c.0, c.1, c.2);
    }
}

/// The whole HUD stack in one call. Pulled out of the gameplay loop for the
/// same reason as everything else here: MIPS branches reach +/-128KB and the
/// loop is at the edge -- the telemetry build, which adds instrumentation to it,
/// tipped over first.
#[inline(never)]
fn draw_all_hud(font: &FontAtlas, player: Player, menu: u8, tool: (u8, u8)) {
    if menu == 0 {
        // Vanilla drops the crosshair whenever a GUI is up; ours used to sit in
        // the middle of every menu.
        draw_crosshair();
        draw_tutorial(font);
        draw_sleep_prompt(font);
    }
    draw_hotbar(font, tool);
    draw_xp(player.xp);
    draw_armor(player.armor);
    draw_hearts(player.health);
    draw_food(player.food);
    draw_hud(font, player);
}

/// R3 on a mob: feed a wolf a bone to tame it, or trade 8 wheat with a villager
/// for an iron ingot (this port's stand-in for emeralds).
#[inline(never)]
/// True if a mob in front took the interaction (so a shared "use" button
/// doesn't also place a block through it).
fn mob_interact(player: &Player) -> bool {
    let has_bone = unsafe { INV[BONE as usize] } > 0;
    let wheat = unsafe { INV[WHEAT_ITEM as usize] };
    match mob::interact(player.x, player.y, player.z, has_bone, wheat) {
        mob::Interact::Tamed => {
            inv_take(BONE);
            sfx::confirm();
            true
        }
        mob::Interact::Traded => {
            let mut k = 0;
            while k < 8 {
                inv_take(WHEAT_ITEM);
                k += 1;
            }
            inv_give(IRON_ORE, 1);
            sfx::confirm();
            true
        }
        mob::Interact::None => false,
    }
}

fn draw_crosshair() {
    // Vanilla's crosshair is a small SOLID plus (it colour-inverts on PC,
    // which the PS1 can't do cheaply -- pale grey reads on both sky and
    // terrain here).
    ui_line(CX - 5, CY, CX + 5, CY, 235, 235, 220);
    ui_line(CX, CY - 5, CX, CY + 5, 235, 235, 220);
}

/// Frames the item-name popup stays up after a switch, and how many of those
/// it spends fading. Vanilla holds it about two seconds, then fades.
const HUD_NAME_HOLD: u32 = 70;
const HUD_NAME_FADE: u32 = 25;
static mut HUD_NAME_T: u32 = 0;
static mut HUD_NAME_LAST: u8 = 0xFF;

/// The dragon: a long dark body, a wedge head out front, and two wings. Boxes,
/// like every other mob here -- but seven of them instead of four, because a
/// silhouette is the only thing that makes it read as a dragon rather than a
/// large cow.
#[inline(never)]
fn render_dragon(cam: &Camera, m: mob::MobView, hw: i32, h: i32, count: &mut usize) {
    let (x, y, z) = (m.x, m.y, m.z);
    let body = (34, 26, 44);
    let dark = (22, 16, 30);
    let wing = (30, 22, 40);
    // Body along Z, tapering to a tail.
    emit_box(cam, x - hw / 2, y, z - hw, x + hw / 2, y + h * 2 / 3, z + hw / 2, body, count);
    emit_box(cam, x - hw / 4, y + h / 6, z + hw / 2, x + hw / 4, y + h / 2, z + hw * 3 / 2, dark, count);
    // Head and snout out front.
    emit_box(cam, x - hw / 3, y + h / 3, z - hw * 3 / 2, x + hw / 3, y + h, z - hw, body, count);
    emit_box(cam, x - hw / 5, y + h / 3, z - hw * 2, x + hw / 5, y + h * 2 / 3, z - hw * 3 / 2, dark, count);
    // Wings, swept out to the sides.
    emit_box(cam, x - hw * 2, y + h / 2, z - hw / 2, x - hw / 2, y + h * 2 / 3, z + hw / 2, wing, count);
    emit_box(cam, x + hw / 2, y + h / 2, z - hw / 2, x + hw * 2, y + h * 2 / 3, z + hw / 2, wing, count);
    // Two purple eyes, the one bright thing on it.
    emit_box(cam, x - hw / 4, y + h * 3 / 4, z - hw * 3 / 2 - 2, x - hw / 8, y + h * 7 / 8, z - hw * 3 / 2, (200, 90, 240), count);
    emit_box(cam, x + hw / 8, y + h * 3 / 4, z - hw * 3 / 2 - 2, x + hw / 4, y + h * 7 / 8, z - hw * 3 / 2, (200, 90, 240), count);
}

/// Boss bar across the top, Java-style, whenever the dragon is alive. It is the
/// only feedback that a 200 hp fight is going anywhere.
#[inline(never)]
fn draw_boss_bar(font: &FontAtlas) {
    let (hp, max) = match mob::dragon_status() {
        Some(v) => v,
        None => return,
    };
    let w = 200i16;
    let x = (SCREEN_W as i16 - w) / 2;
    draw_centered(font, 4, "VOID DRAGON", (0xE0, 0xC0, 0xF0));
    rect(x, 16, w, 5, 40, 20, 50);
    let fill = (hp.max(0) as i32 * w as i32 / max as i32) as i16;
    if fill > 0 {
        rect(x, 16, fill, 5, 190, 80, 220);
    }
}

fn draw_hud(font: &FontAtlas, player: Player) {
    draw_boss_bar(font);
    // Vanilla has no permanent readout in the corner; it flashes the item's name
    // above the hotbar when you switch and lets it fade. This used to sit at
    // (6,6) for the whole session, which is the one bit of chrome that never
    // looked like the game.
    unsafe {
        if EQUIP_T > 0 {
            EQUIP_T -= 1;
            let w = EQUIP_MSG.len() as i16 * 8;
            let x = (SCREEN_W as i16 - w) / 2;
            let lit = if EQUIP_T >= 30 { 255 } else { (EQUIP_T as u32 * 255 / 30) as u8 };
            let g = |c: u8| ((c as u32 * lit as u32) >> 8) as u8;
            ui_text(font, x, HUD_NAME_Y - 12, EQUIP_MSG, (g(0xFF), g(0xE0), g(0x60)));
        }
        if player.selected != HUD_NAME_LAST {
            HUD_NAME_LAST = player.selected;
            HUD_NAME_T = HUD_NAME_HOLD;
        }
        if HUD_NAME_T > 0 {
            HUD_NAME_T -= 1;
            let name = block_name(player.selected);
            let cnt = decimal3(INV[player.selected as usize]);
            // Name and count centred as one unit, with a space between.
            let w = (name.len() + 1 + cnt.len()) as i16 * 8;
            let x = (SCREEN_W as i16 - w) / 2;
            // Fade out over the last stretch rather than blinking off.
            let lit = if HUD_NAME_T >= HUD_NAME_FADE {
                255
            } else {
                (HUD_NAME_T * 255 / HUD_NAME_FADE) as u8
            };
            let dim = |c: u8| ((c as u32 * lit as u32) >> 8) as u8;
            ui_text(font, x, HUD_NAME_Y, name, (dim(0xF0), dim(0xF0), dim(0xF0)));
            ui_text(font, 
                x + (name.len() as i16 + 1) * 8,
                HUD_NAME_Y,
                &cnt,
                (dim(0xC8), dim(0xC8), dim(0x88)),
            );
        }
    }

    // Air bubbles (blue pips) above the hearts only while submerged.
    if player.air < MAX_AIR {
        let bubbles = (player.air * 10 / MAX_AIR).clamp(0, 10) as i16;
        let x0 = (SCREEN_W as i16 - 10 * 9) / 2;
        let y = HUD_ROW3_Y;
        let mut i = 0i16;
        while i < bubbles {
            rect(x0 + i * 9, y, 7, 5, 80, 150, 230);
            i += 1;
        }
    }
}

fn decimal3(v: u16) -> Decimal3 {
    let n = if v > 999 { 999 } else { v };
    Decimal3([
        b'0' + ((n / 100) as u8),
        b'0' + (((n / 10) % 10) as u8),
        b'0' + ((n % 10) as u8),
    ])
}


struct Decimal3([u8; 3]);
impl Decimal3 {
    fn as_str(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.0) }
    }
}
impl core::ops::Deref for Decimal3 {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

fn is_pushable(b: u8) -> bool {
    b != AIR
        && !is_small_block(b)
        && !is_water(b)
        && !is_lava(b)
        && b != PISTON
        && b != CHEST
        && b != FURNACE
        && b != BED
        && b != WIRE
        && b != TORCH
}

fn has_torch_neighbor(x: i32, y: i32, z: i32) -> bool {
    let mut d = 0;
    while d < 6 {
        let dd = DIRS[d];
        if get_block_i32(x + dd.0, y + dd.1, z + dd.2) == TORCH {
            return true;
        }
        d += 1;
    }
    false
}

/// Powered if a torch is adjacent, or an adjacent wire touches a torch.
fn piston_powered(x: i32, y: i32, z: i32) -> bool {
    if has_torch_neighbor(x, y, z) {
        return true;
    }
    let mut d = 0;
    while d < 6 {
        let dd = DIRS[d];
        let (nx, ny, nz) = (x + dd.0, y + dd.1, z + dd.2);
        if get_block_i32(nx, ny, nz) == WIRE && has_torch_neighbor(nx, ny, nz) {
            return true;
        }
        d += 1;
    }
    false
}

/// Basic redstone: every powered piston pushes the block above it up one cell.
/// Iterates only player-edited positions (the EDIT log), so it stays cheap and
/// needs no whole-world scan.
#[inline(never)]
fn redstone_tick() {
    let n = unsafe { EDIT_N };
    let mut i = 0;
    while i < n {
        let b = unsafe { EDIT_B[i] };
        let (x, y, z) = unsafe { (EDIT_X[i] as i32, EDIT_Y[i] as i32, EDIT_Z[i] as i32) };
        if b == PISTON {
            if get_block_i32(x, y, z) == PISTON && piston_powered(x, y, z) {
                let above = get_block_i32(x, y + 1, z);
                if is_pushable(above) && get_block_i32(x, y + 2, z) == AIR {
                    set_block_i32(x, y + 2, z, above);
                    set_block_i32(x, y + 1, z, AIR);
                }
            }
        } else if b == TNT {
            // A redstone-powered TNT block lights its fuse.
            if get_block_i32(x, y, z) == TNT && piston_powered(x, y, z) {
                ignite_tnt(x, y, z);
            }
        }
        i += 1;
    }
}

// ---- TNT: a short fuse, then world::blast + debris + a hit on nearby entities.
// ponytail: blast just destroys neighbouring TNT (no chain reaction); fixed pool.
const MAX_TNT: usize = 8;
const TNT_FUSE_FRAMES: u8 = 45; // ~1.5s at 30fps (Java: 80 ticks)
const TNT_BLAST_R: i32 = 3;
static mut TNT_X: [i32; MAX_TNT] = [0; MAX_TNT];
static mut TNT_Y: [i32; MAX_TNT] = [0; MAX_TNT];
static mut TNT_Z: [i32; MAX_TNT] = [0; MAX_TNT];
static mut TNT_FUSE: [u8; MAX_TNT] = [0; MAX_TNT]; // 0 = inactive

fn ignite_tnt(x: i32, y: i32, z: i32) {
    unsafe {
        let mut i = 0;
        while i < MAX_TNT {
            if TNT_FUSE[i] > 0 && TNT_X[i] == x && TNT_Y[i] == y && TNT_Z[i] == z {
                return; // already fusing here
            }
            i += 1;
        }
        i = 0;
        while i < MAX_TNT {
            if TNT_FUSE[i] == 0 {
                TNT_X[i] = x;
                TNT_Y[i] = y;
                TNT_Z[i] = z;
                TNT_FUSE[i] = TNT_FUSE_FRAMES;
                return;
            }
            i += 1;
        }
    }
}

#[inline(never)]
fn tnt_tick(player: &mut Player) {
    let mut i = 0usize;
    while i < MAX_TNT {
        unsafe {
            if TNT_FUSE[i] > 0 {
                TNT_FUSE[i] -= 1;
                if TNT_FUSE[i] == 0 {
                    let (x, y, z) = (TNT_X[i], TNT_Y[i], TNT_Z[i]);
                    if get_block_i32(x, y, z) == TNT {
                        set_block_i32(x, y, z, AIR);
                        record_edit(x, y, z, AIR);
                        world::blast(x, y, z, TNT_BLAST_R);
                        let wx = block_to_world_x(x) + BLOCK / 2;
                        let wy = y * BLOCK + BLOCK / 2;
                        let wz = block_to_world_z(z) + BLOCK / 2;
                        spawn_particles(wx, wy, wz, (96, 84, 72), 30, (x ^ z) as u32, 46);
                        sfx::explode();
                        // Java-style falloff: lethal point-blank, 0 at 2*power blocks.
                        let pd = (player.x - wx).abs() + (player.y - wy).abs() + (player.z - wz).abs();
                        let dmg = armored(22 - 22 * pd / (6 * BLOCK), player.armor, player.protection);
                        if dmg > 0 && player.hurt_cd == 0 {
                            player.health -= dmg;
                            player.hurt_cd = 16;
                            player.regen_delay = REGEN_DELAY;
                        }
                    }
                }
            }
        }
        i += 1;
    }
}

// ---- Crops: plant seeds on dirt/grass, grow over time, harvest wheat + seeds.
const MAX_CROPS: usize = 24;
const CROP_GROW: u16 = 900; // ~30s to mature at 30fps (a compressed crop cycle)
static mut CROP_X: [i32; MAX_CROPS] = [0; MAX_CROPS];
static mut CROP_Y: [i32; MAX_CROPS] = [0; MAX_CROPS];
static mut CROP_Z: [i32; MAX_CROPS] = [0; MAX_CROPS];
static mut CROP_T: [u16; MAX_CROPS] = [0; MAX_CROPS]; // 0 = free slot, else age 1..CROP_GROW

fn plant_crop(x: i32, y: i32, z: i32) {
    unsafe {
        let mut i = 0;
        while i < MAX_CROPS {
            if CROP_T[i] == 0 {
                CROP_X[i] = x;
                CROP_Y[i] = y;
                CROP_Z[i] = z;
                CROP_T[i] = 1;
                return;
            }
            i += 1;
        }
        // ponytail: pool full -> this crop stays young (never auto-matures);
        // bump MAX_CROPS if players farm more than 24 plots at once.
    }
}

/// Water within ~2 blocks of the soil under a crop (the 4 near + 4 far cardinal
/// cells at soil level). Cheap: 8 lookups, not a full area scan.
fn hydrated(x: i32, y: i32, z: i32) -> bool {
    let sy = y - 1; // soil block under the crop
    is_water(get_block_i32(x + 1, sy, z))
        || is_water(get_block_i32(x - 1, sy, z))
        || is_water(get_block_i32(x, sy, z + 1))
        || is_water(get_block_i32(x, sy, z - 1))
        || is_water(get_block_i32(x + 2, sy, z))
        || is_water(get_block_i32(x - 2, sy, z))
        || is_water(get_block_i32(x, sy, z + 2))
        || is_water(get_block_i32(x, sy, z - 2))
}

#[inline(never)]
fn crop_tick() {
    let mut i = 0usize;
    while i < MAX_CROPS {
        unsafe {
            if CROP_T[i] > 0 {
                let (x, y, z) = (CROP_X[i], CROP_Y[i], CROP_Z[i]);
                if get_block_i32(x, y, z) != WHEAT {
                    CROP_T[i] = 0; // harvested or blown up: free the slot
                } else {
                    // Hydrated soil (water within ~2 blocks) grows 3x faster.
                    CROP_T[i] += if hydrated(x, y, z) { 3 } else { 1 };
                    if CROP_T[i] >= CROP_GROW {
                        set_block_i32(x, y, z, WHEAT_RIPE);
                        record_edit(x, y, z, WHEAT_RIPE);
                        CROP_T[i] = 0;
                    }
                }
            }
        }
        i += 1;
    }
}

// ---- Saplings: plant on soil, grow into a tree (renewable wood).
const MAX_SAPS: usize = 8;
const SAP_GROW: u16 = 1350; // ~45s at 30fps
static mut SAP_X: [i32; MAX_SAPS] = [0; MAX_SAPS];
static mut SAP_Y: [i32; MAX_SAPS] = [0; MAX_SAPS];
static mut SAP_Z: [i32; MAX_SAPS] = [0; MAX_SAPS];
static mut SAP_T: [u16; MAX_SAPS] = [0; MAX_SAPS]; // 0 = free slot

fn plant_sapling(x: i32, y: i32, z: i32) {
    unsafe {
        let mut i = 0;
        while i < MAX_SAPS {
            if SAP_T[i] == 0 {
                SAP_X[i] = x;
                SAP_Y[i] = y;
                SAP_Z[i] = z;
                SAP_T[i] = 1;
                return;
            }
            i += 1;
        }
        // ponytail: pool full -> extra saplings stay bushes; bump MAX_SAPS if needed.
    }
}

#[inline(never)]
fn sap_tick() {
    let mut i = 0usize;
    while i < MAX_SAPS {
        unsafe {
            if SAP_T[i] > 0 {
                let (x, y, z) = (SAP_X[i], SAP_Y[i], SAP_Z[i]);
                if get_block_i32(x, y, z) != SAPLING {
                    SAP_T[i] = 0; // broken or blown up
                } else {
                    SAP_T[i] += 1;
                    if SAP_T[i] >= SAP_GROW {
                        world::grow_tree(x, y, z);
                        record_edit(x, y, z, WOOD); // trunk base persists in the edit log
                        SAP_T[i] = 0;
                        sfx::place();
                    }
                }
            }
        }
        i += 1;
    }
}

/// Blocks drawn semi-transparent (water, glass) -- after opaques via OT depth.
fn is_transparent(block: u8) -> bool {
    // Leaves are OPAQUE (solid canopy); only water and glass are see-through. Water
    // shows the meshed sea floor through it; its deeper-than-sky blue stays legible.
    is_water(block) || block == GLASS
}

fn face_tile(block: u8, dir: usize) -> u8 {
    match block {
        GRASS => match dir {
            2 => tex::T_GRASS_TOP,
            3 => tex::T_DIRT,
            _ => tex::T_GRASS_SIDE,
        },
        DIRT => tex::T_DIRT,
        STONE => tex::T_STONE,
        WOOD => match dir {
            2 | 3 => tex::T_WOOD_TOP,
            _ => tex::T_WOOD_SIDE,
        },
        LEAVES => tex::T_LEAVES,
        SAPLING => tex::T_LEAVES, // young bush look
        DOOR_C | DOOR_O => tex::T_DOOR,
        CACTUS => tex::T_CACTUS,
        CLAY => tex::T_CLAY,
        BRICK => tex::T_BRICK,
        SAND => tex::T_SAND,
        WATER | WATER_F1..=WATER_F7 => tex::T_WATER,
        SLAB | STAIRS_N | STAIRS_E | STAIRS_S | STAIRS_W => tex::T_COBBLE, // hotbar icon
        FLINT_STEEL => tex::T_FIRE, // hotbar icon only
        EMBER_CAP => tex::T_CROP_RIPE, // hotbar icons only; never placed
        VOID_STONE => tex::T_VOID_STONE,
        VOID_EYE | VOID_PEARL => tex::T_PORTAL,
        STRING => tex::T_WOOL,
        EMBER_ROD => tex::T_FACE_EMBER,
        WAILER_TEAR => tex::T_FACE_WAILER,
        MAGMA_PASTE => tex::T_LAVA,
        SUGAR_CANE => tex::T_CROP_YOUNG,
        BOTTLE => tex::T_SNOW_TOP,
        POTION_AWKWARD | POTION_SPEED | POTION_STRENGTH | POTION_REGEN | POTION_FIRE => {
            tex::T_PORTAL
        }
        FENCE => tex::T_WOOD_SIDE,
        COAL_ORE => tex::T_COAL,
        IRON_ORE => tex::T_IRON,
        GOLD_ORE => tex::T_GOLD,
        DIAMOND_ORE => tex::T_DIAMOND,
        LAVA | LAVA_F1..=LAVA_F3 => tex::T_LAVA,
        FIRE => tex::T_FIRE,
        OBSIDIAN => tex::T_OBSIDIAN,
        CINDERSTONE => tex::T_CINDERSTONE,
        SINK_SAND => tex::T_SINK_SAND,
        LUMISTONE => tex::T_LUMISTONE,
        SNOW => match dir {
            2 => tex::T_SNOW_TOP,
            3 => tex::T_DIRT,
            _ => tex::T_SNOW_SIDE,
        },
        GLASS => tex::T_SNOW_TOP, // frosted (white) tile, drawn translucent
        CHEST => match dir {
            2 | 3 => tex::T_WOOD_TOP,
            _ => tex::T_CHEST,
        },
        CRAFT_TABLE => match dir {
            2 | 3 => tex::T_CRAFT_TOP,
            _ => tex::T_CRAFT_SIDE,
        },
        FURNACE => tex::T_STONE,
        BED => match dir {
            2 => tex::T_WOOD_TOP,
            _ => tex::T_WOOD_SIDE,
        },
        WIRE => tex::T_COAL,   // dark dust stand-in
        TORCH => tex::T_LAVA,  // glowing red/orange
        PISTON => tex::T_WOOD_TOP,
        TNT => tex::T_TNT,           // red body with a white band
        WHEAT => tex::T_LEAVES,      // young crop: green
        WHEAT_RIPE => tex::T_WHEAT_RIPE, // golden stalks
        SEEDS => tex::T_GRASS_TOP,   // hotbar icon only (planting places WHEAT)
        WHEAT_ITEM => tex::T_WHEAT_RIPE, // hotbar icon only (feeds animals)
        BONEMEAL => tex::T_WOOL,     // hotbar icon only (white powder)
        FISHING_ROD => tex::T_WOOD_SIDE, // hotbar icon only
        BOW => tex::T_WOOD_SIDE,     // hotbar icon only (never placed)
        BUCKET => tex::T_IRON,       // empty bucket icon (iron)
        WATER_BUCKET => tex::T_WATER, // filled-bucket icons (never placed as ids)
        LAVA_BUCKET => tex::T_LAVA,
        PLANK => tex::T_PLANK,       // horizontal boards
        WOOL => tex::T_WOOL,         // white fuzzy wool
        LADDER => tex::T_LADDER,     // rails + rungs
        COBBLE => tex::T_COBBLE,     // mortar-grid cobblestone
        ENCHANT => tex::T_DIAMOND,   // mystical blue (reuses the diamond tile)
        FLOWER_R => tex::T_FLOWER_R, // cross-sprite tiles (only reached if selected)
        FLOWER_Y => tex::T_FLOWER_Y,
        TALL_GRASS => tex::T_TALLGRASS,
        _ => tex::T_STONE,
    }
}

/// Per-face directional shade that modulates the texture (128 = unchanged),
/// further scaled by the global day/night sky light. Top brightest, bottom
/// darkest -- cheap fake AO with no extra geometry.
fn face_tint(dir: usize) -> (u8, u8, u8) {
    // Java Edition's face shading is 1.0 / 0.8 / 0.6 / 0.5 (top / N-S / E-W /
    // bottom). Ours was 1.0 / 0.875 / 0.75 / 0.5625 -- side faces about 25% too
    // bright, which at 15-bit colour left a barely-visible step between a top
    // face and a side and flattened every cube edge in the frame. Two separate
    // visual audits landed on this independently as the reason the dunes read
    // as one mush instead of a floor made of metre cubes.
    let s = match dir {
        2 => 128u32,
        4 | 5 => 102,
        0 | 1 => 77,
        _ => 64,
    };
    let light = unsafe { LIGHT } as u32;
    let v = (s * light / 128) as u32;
    // Night is BLUE, not a darkened copy of noon. A flat achromatic multiply
    // turned the whole world olive-brown after dusk; Java's night light source
    // is cool, so grass, sand and stone all shift toward slate. Lerp the bias
    // from neutral at full light to cool at night -- baked into MAT_CCMD once a
    // frame, so it costs nothing per face.
    // Keyed off the TIME OF DAY, not `LIGHT`: `LIGHT` carries the rain dimming,
    // so keying on it painted a midday shower with night's cool cast and turned
    // the sand grey. Exactly the trap the star field fell into.
    let night = unsafe { NIGHT_BIAS } as i32;
    // The SUN'S OWN COLOUR warms the ground at dawn and dusk. Without this the
    // two halves of the frame disagreed about where the light was coming from:
    // an audit measured the sky at (236,122,60) hot orange while the ground had
    // gone (114,107,91) cold grey-green, because the cool night bias was already
    // ramping in during the sunset window. Sunlight reddens as it goes down; the
    // ground has to redden with it.
    let warm = unsafe { SUN_WARMTH } as i32;
    let cool = |num: i32| 128 * (90 - night) + num * night;
    let (mut br, mut bg, mut bb) = (cool(100), cool(110), cool(147));
    if warm > 0 {
        // Lerp the whole bias toward a warm low-sun tint: red HIGHEST, blue
        // lowest. The first version had red at 34 -- almost certainly a slip for
        // 134 -- which suppressed red harder than either other channel, so
        // instead of warming, the terrain went green. An audit caught the desert
        // turning into a golf course twice per in-game day, with the fog band
        // (which takes its colour from the sky, not from here) still orange
        // beside it. Strictly worse than the cold grey it replaced.
        let d = 128 * 90;
        br = br + (d * 147 / 128 - br) * warm / 255;
        bg = bg + (d * 112 / 128 - bg) * warm / 255;
        bb = bb + (d * 78 / 128 - bb) * warm / 255;
    }
    let bias = |num: i32| ((v as i32 * num) / (128 * 90)) as u8;
    (bias(br), bias(bg), bias(bb))
}

/// Queue the GPU's ordered-dither draw mode at the head of the sky list.
///
/// This was previously attempted with `TextureMaterial::with_dither(true)` on
/// the per-face materials, and an audit proved it did NOTHING -- a build with
/// with_dither(false) was pixel-identical. The reason is that with_dither sets
/// bit 9 of the POLYGON's embedded texpage attribute word, and dither is not
/// there: it is bit 9 of the GP0(E1h) draw-mode REGISTER. Real hardware copies
/// only bits 0-8 and 11 of a polygon's texpage attribute into GPUSTAT, so the
/// dither bit in that word is ignored on console exactly as it was in the
/// emulator. It has to be an E1 write, and nothing in the game was making one.
///
/// This must be a packet, not an immediate GP0 write: gameplay builds the next
/// arena while the GPU rasterises the previous one. At 15-bit colour the sky
/// gradient otherwise steps in 8-unit bands across the whole dome.
#[inline(never)]
fn queue_dither() {
    let bt = unsafe { BLOCK_TEX };
    let p = ui_alloc(1);
    if p.is_null() {
        return;
    }
    unsafe {
        *p.add(1) = TextureMaterial::opaque(0, bt.tpage, (128, 128, 128))
            .with_dither(true)
            .draw_mode_word();
    }
}

/// Sky for whichever dimension we are in. Off-world there is none: the Inferno
/// is a closed cavern and the Void is open blackness, so both get a flat fog wall
/// instead of the gradient/sun/moon/stars/cloud stack.
#[inline(never)]
fn draw_frame_sky(cam: &Camera, day: u32, tod: u32, light: u8, sky: (u8, u8, u8), raining: bool) {
    // Deep underground gets the same treatment for the same reason: the dome's
    // zenith comes from the time of day, not from `sky`, so a darkened horizon
    // alone still left daylight blue showing through the cave roof.
    if world::dimension() != world::DIM_OVERWORLD || unsafe { CAVE } >= CAVE_SOLID {
        rect(0, 0, SCREEN_W as i16, SCREEN_H as i16, sky.0, sky.1, sky.2);
    } else {
        draw_sky(cam, day, tod, light, sky, raining);
    }
}

/// Sky light (0..128) for a point in the day. Trapezoid: ~40% full day, short
/// dusk down to night, ~40% night, short dawn back up.
fn day_brightness(t: u32) -> u8 {
    let q = (DAY_LEN / 10) as i32;
    let t = t as i32;
    if t < 4 * q {
        128
    } else if t < 5 * q {
        lerp_u8(128, NIGHT_LIGHT, t - 4 * q, q)
    } else if t < 9 * q {
        NIGHT_LIGHT as u8
    } else {
        lerp_u8(NIGHT_LIGHT, 128, t - 9 * q, q)
    }
}

#[inline]
fn lerp_u8(a: i32, b: i32, num: i32, den: i32) -> u8 {
    (a + (b - a) * num / den).clamp(0, 255) as u8
}

/// Warm the horizon toward sunset orange during the dawn/dusk transition windows
/// (day_brightness ramps light over 4q..5q dusk and 9q..10q dawn). Warmth peaks
/// mid-window and is zero in full day/night; rain suppresses it. Only the sky
/// horizon is tinted -- the terrain keeps the day/night light.
/// Warm orange glow colour at the horizon around sunrise/sunset.
const SUNSET: (i32, i32, i32) = (236, 122, 60);

/// How warm the horizon is right now, 0..255. Split out of `apply_sunset` so
/// `draw_sky` can apply it PER AZIMUTH: Java's sunset is a band in the sun's
/// direction only, and tinting all 360 degrees at once (which is what the old
/// single-colour horizon did) is the least Minecraft-looking thing the sky did.
fn sunset_warmth(tod: u32, raining: bool) -> i32 {
    if raining {
        return 0;
    }
    let q = (DAY_LEN / 10) as i32;
    let t = tod as i32;
    let window = |start: i32| -> i32 {
        let d = t - start;
        if d < 0 || d >= q {
            return 0;
        }
        let half = (q / 2).max(1);
        let m = if d < half { d } else { q - d };
        m * 255 / half
    };
    window(4 * q).max(window(9 * q)) // dusk, then dawn
}

fn apply_sunset(sky: (u8, u8, u8), tod: u32, raining: bool) -> (u8, u8, u8) {
    if raining {
        return sky;
    }
    let warmth = sunset_warmth(tod, raining);
    if warmth == 0 {
        return sky;
    }
        (
        lerp_u8(sky.0 as i32, SUNSET.0, warmth, 255),
        lerp_u8(sky.1 as i32, SUNSET.1, warmth, 255),
        lerp_u8(sky.2 as i32, SUNSET.2, warmth, 255),
    )
}

fn block_name(block: u8) -> &'static str {
    match block {
        GRASS => "GRASS",
        DIRT => "DIRT",
        STONE => "STONE",
        WOOD => "WOOD",
        LEAVES => "LEAVES",
        SAND => "SAND",
        WATER | WATER_F1..=WATER_F7 => "WATER",
        SLAB => "SLAB",
        FIRE => "FIRE",
        OBSIDIAN => "OBSIDIAN",
        CINDERSTONE => "CINDER",
        SINK_SAND => "SINKSAND",
        LUMISTONE => "LUMISTN",
        PORTAL => "PORTAL",
        FLINT_STEEL => "FLINT",
        EMBER_CAP => "EMBERCAP",
        BOTTLE => "BOTTLE",
        POTION_AWKWARD => "AWKWARD",
        POTION_SPEED => "SPEED",
        POTION_STRENGTH => "STRENGTH",
        POTION_REGEN => "REGEN",
        POTION_FIRE => "FIRERES",
        VOID_STONE => "VOIDROCK",
        VOID_PORTAL => "VOIDPORT",
        VOID_EYE => "EYE",
        STRING => "STRING",
        SUGAR_CANE => "CANE",
        VOID_PEARL => "PEARL",
        EMBER_ROD => "EMBERROD",
        WAILER_TEAR => "TEAR",
        MAGMA_PASTE => "MAGMA",
        STAIRS_N | STAIRS_E | STAIRS_S | STAIRS_W => "STAIRS",
        FENCE => "FENCE",
        COAL_ORE => "COAL",
        IRON_ORE => "IRON",
        GOLD_ORE => "GOLD",
        DIAMOND_ORE => "DIAMOND",
        LAVA | LAVA_F1..=LAVA_F3 => "LAVA",
        SNOW => "SNOW",
        GLASS => "GLASS",
        CHEST => "CHEST",
        CRAFT_TABLE => "CRAFT TABLE",
        FURNACE => "FURNACE",
        BED => "BED",
        WIRE => "WIRE",
        TORCH => "TORCH",
        PISTON => "PISTON",
        TNT => "TNT",
        WHEAT | WHEAT_RIPE | WHEAT_ITEM => "WHEAT",
        SEEDS => "SEEDS",
        BREAD => "BREAD",
        BOW => "BOW",
        ARROW => "ARROW",
        WOOL => "WOOL",
        LADDER => "LADDER",
        BUCKET => "BUCKET",
        WATER_BUCKET => "WATERBKT",
        LAVA_BUCKET => "LAVABKT",
        BONE => "BONE",
        BONEMEAL => "BONEMEAL",
        FISHING_ROD => "ROD",
        COBBLE => "COBBLE",
        ENCHANT => "ENCHANT",
        IRON_INGOT => "INGOT",
        PLANK => "PLANK",
        STICK => "STICK",
        SAPLING => "SAPLING",
        RAW_MEAT => "RAW MEAT",
        COOKED_MEAT => "STEAK",
        GUNPOWDER => "POWDER",
        DOOR_C | DOOR_O => "DOOR",
        CACTUS => "CACTUS",
        CLAY => "CLAY",
        BRICK => "BRICK",
        FLOWER_R | FLOWER_Y => "FLOWER",
        TALL_GRASS => "TALLGRASS",
        _ => "BLOCK",
    }
}

/// What a mined block yields. Grass gives dirt; leaves/fluids give nothing.
fn drop_of(block: u8) -> u8 {
    match block {
        GRASS => DIRT,
        STONE => COBBLE, // mining smooth stone yields cobblestone (Java)
        DOOR_O => DOOR_C, // breaking an open door yields the door
        STAIRS_E | STAIRS_S | STAIRS_W => STAIRS_N, // all four facings are one item
        TALL_GRASS => SEEDS, // harvesting grass gives seeds (Java), feeding farming
        LEAVES | WATER | WATER_F1..=WATER_F7 | LAVA | LAVA_F1..=LAVA_F3 | FIRE | FLOWER_R
        | FLOWER_Y => AIR, // flowers are decor only
        _ => block,
    }
}

fn inv_add(block: u8) {
    let d = drop_of(block);
    if d != AIR {
        inv_give(d, 1);
    }
}

/// Central inventory gain: every acquisition routes through here so a newly
/// owned kind auto-fills the first free hotbar slot, as in Bedrock.
fn inv_give(item: u8, n: u16) {
    if n == 0 {
        return;
    }
    unsafe {
        if INV[item as usize] == 0 {
            hotbar_add(item);
        }
        INV[item as usize] = INV[item as usize].saturating_add(n);
    }
}

fn in_placeable(item: u8) -> bool {
    let mut i = 0;
    while i < PLACEABLE.len() {
        if PLACEABLE[i] == item {
            return true;
        }
        i += 1;
    }
    false
}

/// First free slot, unless the item already holds one. Non-selectable items
/// (ammo, drops with no use) never occupy a slot.
fn hotbar_add(item: u8) {
    if !in_placeable(item) {
        return;
    }
    unsafe {
        let mut i = 0;
        while i < HOTBAR_VIS {
            if HOTBAR[i] == item {
                return;
            }
            i += 1;
        }
        let mut i = 0;
        while i < HOTBAR_VIS {
            if HOTBAR[i] == AIR {
                HOTBAR[i] = item;
                return;
            }
            i += 1;
        }
    }
}

/// Inventory-panel pick: jump to the item's slot if it has one (vanilla),
/// otherwise it takes over the currently selected slot.
fn hotbar_pick(item: u8) {
    unsafe {
        let mut i = 0;
        while i < HOTBAR_VIS {
            if HOTBAR[i] == item {
                HOTBAR_SEL = i;
                return;
            }
            i += 1;
        }
        HOTBAR[HOTBAR_SEL] = item;
    }
}

/// Once per frame: emptied stacks vacate their slot (the gap stays), and the
/// held item is re-derived from the selected slot -- so every INV decrement
/// site is covered without hooks.
fn hotbar_sync(player: &mut Player) {
    unsafe {
        let mut i = 0;
        while i < HOTBAR_VIS {
            if HOTBAR[i] != AIR && INV[HOTBAR[i] as usize] == 0 {
                HOTBAR[i] = AIR;
            }
            i += 1;
        }
        player.selected = HOTBAR[HOTBAR_SEL];
    }
}

fn inv_take(block: u8) -> bool {
    unsafe {
        if INV[block as usize] > 0 {
            INV[block as usize] -= 1;
            true
        } else {
            false
        }
    }
}

fn chest_find(x: i32, y: i32, z: i32) -> Option<usize> {
    let mut i = 0;
    while i < MAX_CHESTS {
        unsafe {
            if CHEST_USED[i] && CHEST_X[i] == x && CHEST_Y[i] == y && CHEST_Z[i] == z {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn chest_register(x: i32, y: i32, z: i32) {
    if chest_find(x, y, z).is_some() {
        return;
    }
    let mut i = 0;
    while i < MAX_CHESTS {
        unsafe {
            if !CHEST_USED[i] {
                CHEST_USED[i] = true;
                CHEST_X[i] = x;
                CHEST_Y[i] = y;
                CHEST_Z[i] = z;
                CHEST_INV[i] = [0; BLOCK_KINDS];
                return;
            }
        }
        i += 1;
    }
}

/// Remove a chest, spilling its contents back into the player inventory.
fn chest_remove(x: i32, y: i32, z: i32) {
    if let Some(i) = chest_find(x, y, z) {
        let mut k = 0;
        while k < BLOCK_KINDS {
            unsafe {
                inv_give(k as u8, CHEST_INV[i][k]);
                CHEST_INV[i][k] = 0;
            }
            k += 1;
        }
        unsafe {
            CHEST_USED[i] = false;
        }
    }
}

fn chest_deposit(i: usize, item: u8) {
    unsafe {
        if INV[item as usize] > 0 {
            INV[item as usize] -= 1;
            CHEST_INV[i][item as usize] += 1;
        }
    }
}

fn chest_withdraw(i: usize, item: u8) {
    unsafe {
        if CHEST_INV[i][item as usize] > 0 {
            CHEST_INV[i][item as usize] -= 1;
            inv_give(item, 1);
        }
    }
}

fn furn_find(x: i32, y: i32, z: i32) -> Option<usize> {
    let mut i = 0;
    while i < MAX_FURNACES {
        unsafe {
            if FURN_USED[i] && FURN_X[i] == x && FURN_Y[i] == y && FURN_Z[i] == z {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn furn_register(x: i32, y: i32, z: i32) {
    if furn_find(x, y, z).is_some() {
        return;
    }
    let mut i = 0;
    while i < MAX_FURNACES {
        unsafe {
            if !FURN_USED[i] {
                FURN_USED[i] = true;
                FURN_X[i] = x;
                FURN_Y[i] = y;
                FURN_Z[i] = z;
                FURN_IN[i] = AIR;
                FURN_IN_N[i] = 0;
                FURN_FUEL[i] = 0;
                FURN_OUT[i] = AIR;
                FURN_OUT_N[i] = 0;
                FURN_PROG[i] = 0;
                return;
            }
        }
        i += 1;
    }
}

/// Remove a furnace, spilling its slots back into the player inventory.
fn furn_remove(x: i32, y: i32, z: i32) {
    if let Some(i) = furn_find(x, y, z) {
        unsafe {
            if FURN_IN[i] != AIR {
                inv_give(FURN_IN[i], FURN_IN_N[i]);
            }
            // Refund only whole unburnt coal (partial fuel is lost, as in Java).
            inv_give(COAL_ORE, FURN_FUEL[i] / COAL_SMELTS);
            if FURN_OUT[i] != AIR {
                inv_give(FURN_OUT[i], FURN_OUT_N[i]);
            }
            FURN_USED[i] = false;
        }
    }
}

fn furn_deposit(i: usize, item: u8) {
    unsafe {
        let f = fuel_smelts(item);
        if f > 0 {
            if INV[item as usize] > 0 {
                INV[item as usize] -= 1;
                FURN_FUEL[i] += f; // coal 8 smelts, wood/planks 2
            }
        } else if smelt_result(item) != AIR
            && INV[item as usize] > 0
            && (FURN_IN[i] == AIR || FURN_IN[i] == item)
        {
            INV[item as usize] -= 1;
            FURN_IN[i] = item;
            FURN_IN_N[i] += 1;
        }
    }
}

fn furn_withdraw(i: usize) {
    unsafe {
        if FURN_OUT_N[i] > 0 {
            inv_give(FURN_OUT[i], FURN_OUT_N[i]);
            FURN_OUT_N[i] = 0;
            FURN_OUT[i] = AIR;
        }
    }
}

/// Advance all furnaces by one frame: smelt when input + fuel are present.
#[inline(never)]
fn furn_tick() {
    let mut i = 0;
    while i < MAX_FURNACES {
        unsafe {
            if FURN_USED[i] && FURN_IN_N[i] > 0 && FURN_FUEL[i] > 0 {
                let r = smelt_result(FURN_IN[i]);
                if r != AIR && (FURN_OUT[i] == AIR || FURN_OUT[i] == r) {
                    FURN_PROG[i] += 1;
                    if FURN_PROG[i] >= SMELT_TIME {
                        FURN_PROG[i] = 0;
                        FURN_IN_N[i] -= 1;
                        FURN_FUEL[i] -= 1;
                        FURN_OUT[i] = r;
                        FURN_OUT_N[i] = FURN_OUT_N[i].saturating_add(1);
                        if FURN_IN_N[i] == 0 {
                            FURN_IN[i] = AIR;
                        }
                    }
                } else {
                    FURN_PROG[i] = 0;
                }
            } else if FURN_USED[i] {
                FURN_PROG[i] = 0;
            }
        }
        i += 1;
    }
}

/// Bare-hand break time in frames (~30 fps). 0 means not mineable (water,
/// air) or unbreakable. Tool tiers will divide these once tools exist.
/// Relative break difficulty, scaled ~15x Java hardness to keep MC's ordering.
/// (Java: leaves/bed 0.2, glass 0.3, dirt/sand 0.5, grass 0.6, stone 1.5,
/// log 2.0, chest 2.5, all ores 3.0, furnace 3.5.)
fn block_hardness(block: u8) -> u32 {
    match block {
        WIRE | TORCH | WHEAT | WHEAT_RIPE | SAPLING | FLOWER_R | FLOWER_Y | TALL_GRASS | FIRE
        | SUGAR_CANE => 1, // ~instant (Java 0)
        LADDER | CACTUS => 6,                                // 0.4
        LEAVES | SNOW | BED => 3,                            // 0.2
        WOOL => 12,                                          // 0.8
        GLASS => 5,                                          // 0.3
        GRASS | DIRT | SAND | TNT | CLAY => 9,               // 0.5-0.6
        STONE | PISTON => 22,                                // 1.5
        WOOD | PLANK | COBBLE | DOOR_C | DOOR_O => 30,       // 2.0
        SLAB | STAIRS_N | STAIRS_E | STAIRS_S | STAIRS_W => 30, // as cobble
        FENCE => 30,                                         // as planks
        BRICK => 30,                                         // 2.0
        CHEST | CRAFT_TABLE => 38,                           // 2.5
        COAL_ORE | IRON_ORE | GOLD_ORE | DIAMOND_ORE => 45,  // all 3.0 in Java
        ENCHANT => 45,                                       // sturdy
        FURNACE => 53,                                       // 3.5
        OBSIDIAN => 200,                                     // Java 50: the long one
        CINDERSTONE => 6,                                     // 0.4, crumbles
        VOID_STONE => 45,                                     // 3.0, like Java
        SINK_SAND | LUMISTONE => 9,                          // 0.5 / 0.3
        _ => 0,
    }
}

/// Minimum tool tier (0 hand .. 4 diamond) to actually DROP a block, mirroring
/// Java's pickaxe gating: stone/coal need wood-pick (1), iron ore stone (2),
/// gold/diamond ore iron (3). Below tier, the block still breaks but drops
/// nothing. ponytail: one generic tool, so tier stands in for tool class.
fn mine_min_tier(block: u8) -> u8 {
    match block {
        STONE | COBBLE | PISTON | FURNACE | COAL_ORE | ENCHANT => 1,
        IRON_ORE => 2,
        GOLD_ORE | DIAMOND_ORE => 3,
        OBSIDIAN => 4, // diamond only, as in Java
        _ => 0,
    }
}

/// The tool the HUD and the hand should show: whatever suits the block under
/// the crosshair, falling back to the sword (what you would swing at a mob)
/// and finally to the best tool owned, so the slot is never blank for nothing.
fn hud_tool(p: &Player, target: u8) -> (u8, u8) {
    let class = tool_for(target);
    if class != TOOL_NONE && tool_tier(p, class) > 0 {
        return (class, tool_tier(p, class));
    }
    if p.sword > 0 {
        return (TOOL_SWORD, p.sword);
    }
    let best = [TOOL_PICK, TOOL_AXE, TOOL_SHOVEL];
    let mut i = 0;
    let (mut bc, mut bt) = (TOOL_PICK, 0);
    while i < best.len() {
        let tier = tool_tier(p, best[i]);
        if tier > bt {
            bc = best[i];
            bt = tier;
        }
        i += 1;
    }
    (bc, bt)
}

/// Tier colour: wood, stone, iron, diamond.
fn tool_tint(tier: u8) -> (u8, u8, u8) {
    match tier {
        1 => (140, 100, 55),
        2 => (115, 115, 120),
        3 => (205, 205, 210),
        _ => (90, 220, 220),
    }
}

/// The atlas tile for a tool class.
fn tool_tile(class: u8) -> u8 {
    match class {
        TOOL_AXE => tex::T_AXE,
        TOOL_SHOVEL => tex::T_SHOVEL,
        TOOL_SWORD => tex::T_SWORD,
        _ => tex::T_PICK,
    }
}

/// True for the recipe outputs that raise a tool tier instead of granting an item.
fn is_tool_recipe(out: u8) -> bool {
    matches!(out, CRAFT_PICK | CRAFT_AXE | CRAFT_SHOVEL | CRAFT_SWORD)
}

// Which tool a block yields to. 0 = none (bare hands are as good as anything).
const TOOL_NONE: u8 = 0;
const TOOL_PICK: u8 = 1;
const TOOL_AXE: u8 = 2;
const TOOL_SHOVEL: u8 = 3;
const TOOL_SWORD: u8 = 4;

/// The tool class a block is worked with, following Java's material families.
fn tool_for(block: u8) -> u8 {
    match block {
        STONE | COBBLE | BRICK | OBSIDIAN | FURNACE | ENCHANT | SLAB | PISTON | CINDERSTONE
        | VOID_STONE | LUMISTONE | COAL_ORE | IRON_ORE | GOLD_ORE | DIAMOND_ORE => TOOL_PICK,
        WOOD | PLANK | FENCE | CHEST | CRAFT_TABLE | DOOR_C | DOOR_O | LADDER | BED => TOOL_AXE,
        GRASS | DIRT | SAND | SINK_SAND | SNOW | CLAY => TOOL_SHOVEL,
        _ => TOOL_NONE,
    }
}

/// The player's tier in one tool class.
fn tool_tier(p: &Player, class: u8) -> u8 {
    match class {
        TOOL_PICK => p.pick,
        TOOL_AXE => p.axe,
        TOOL_SHOVEL => p.shovel,
        TOOL_SWORD => p.sword,
        _ => 0,
    }
}

/// Mining speed (progress per frame). The RIGHT tool speeds a block up; the
/// wrong one works at bare-hand pace, which is what makes carrying a set of
/// tools worth the crafting.
fn mine_speed(p: &Player, block: u8) -> u32 {
    let class = tool_for(block);
    if class == TOOL_NONE {
        return 1 + tool_tier(p, TOOL_SHOVEL).max(tool_tier(p, TOOL_PICK)) as u32 / 2;
    }
    1 + tool_tier(p, class) as u32
}

/// Cycle to the next placeable item that the player actually owns. Previously
/// R1/L1 walked through the entire 47-item catalogue, so nearly every selection
/// looked valid but could never place; starter dirt appeared to be the only
/// working block.
/// Analog axis with a SMOOTH deadzone: 0 inside the zone, otherwise ramps from 0
/// (subtracts the zone) so the stick eases in instead of jumping to full speed at
/// the edge -- the classic dual-stick feel.
fn pressed(now: ButtonState, previous: ButtonState, mask: u16) -> bool {
    now.is_held(mask) && !previous.is_held(mask)
}

fn get_block_i32(x: i32, y: i32, z: i32) -> u8 {
    world::get(x, y, z)
}

fn set_block_i32(x: i32, y: i32, z: i32, b: u8) {
    world::set(x, y, z, b);
}

fn block_to_world_x(x: i32) -> i32 {
    x * BLOCK
}

fn block_to_world_z(z: i32) -> i32 {
    z * BLOCK
}

fn world_to_block_x(x: i32) -> i32 {
    floor_div(x, BLOCK)
}

fn world_to_block_y(y: i32) -> i32 {
    floor_div(y, BLOCK)
}

fn world_to_block_z(z: i32) -> i32 {
    floor_div(z, BLOCK)
}

#[inline]
fn floor_div(v: i32, d: i32) -> i32 {
    if v >= 0 {
        v / d
    } else {
        -((-v + d - 1) / d)
    }
}

//! Procedurally-generated, classic-style 16x16 block textures packed into one
//! shared 4-bit CLUT page in VRAM.
//!
//! No Mojang assets ship here: every texel is computed from a fixed 16-colour
//! palette and a small hash-noise function. One page + one CLUT means every
//! block face draws with the same tpage/clut words -- no per-face GPU state
//! thrash. 16 tiles fill one row of the page.

use psx_math::int32::isqrt_i32;
use psx_vram::{upload_bytes, Clut, Color555, TexDepth, Tpage, VramRect};

pub const TILE: usize = 16;
const TILES_PER_ROW: usize = 16;
// Five rows of 16. The atlas sits at VRAM (384,0); a 4bpp page is 256x256
// texels, so the page could hold SIXTEEN rows (256 tiles) and we are nowhere
// near it. The binding limit is the CLUT band below at y=256: two CLUTs per
// tile stack one row apart, so 128 tiles would reach y=511, the floor of
// VRAM. Everything here stays clear of the framebuffers (x<320) and the font
// page (x=320).
pub const TILE_COUNT: usize = 80;
const ATLAS_W: usize = TILE * TILES_PER_ROW; // 256 texels wide
const ATLAS_H: usize = TILE * (TILE_COUNT / TILES_PER_ROW); // 80 texels tall (5 rows)

// Tile indices into the shared page. Row 0 = tiles 0..15, row 1 = 16..31.
pub const T_GRASS_TOP: u8 = 0;
pub const T_GRASS_SIDE: u8 = 1;
pub const T_DIRT: u8 = 2;
pub const T_STONE: u8 = 3;
pub const T_WOOD_TOP: u8 = 4;
pub const T_WOOD_SIDE: u8 = 5;
pub const T_LEAVES: u8 = 6;
pub const T_SAND: u8 = 7;
pub const T_WATER: u8 = 8;
pub const T_COAL: u8 = 9;
pub const T_IRON: u8 = 10;
pub const T_GOLD: u8 = 11;
pub const T_DIAMOND: u8 = 12;
pub const T_LAVA: u8 = 13;
pub const T_SNOW_TOP: u8 = 14;
pub const T_SNOW_SIDE: u8 = 15;
// Row 1 (added when the 16-tile row filled up).
pub const T_COBBLE: u8 = 16;
pub const T_PLANK: u8 = 17;
pub const T_WOOL: u8 = 18;
pub const T_TNT: u8 = 19;
pub const T_BRICK: u8 = 20;
pub const T_WHEAT_RIPE: u8 = 21;
pub const T_LADDER: u8 = 22;
// Mob FACE tiles (row 1, slots 23..30): hand-authored 16x16 fronts for the
// mob heads -- the face is what makes a Minecraft mob read as itself.
pub const T_FACE_PIG: u8 = 23;
pub const T_FACE_COW: u8 = 24;
pub const T_FACE_SHEEP: u8 = 25;
pub const T_FACE_CHICKEN: u8 = 26;
pub const T_FACE_ZOMBIE: u8 = 27;
pub const T_FACE_SKELETON: u8 = 28;
pub const T_FACE_SAPPER: u8 = 29;
pub const T_FACE_SPIDER: u8 = 30;
// Shared mob-body "hide" tile: a soft mottle over palette indices 0..3, which
// every mob-face palette reserves for its skin-shade ramp -- so one tile plus
// the mob's own face CLUT textures its body (no extra CLUTs).
pub const T_HIDE: u8 = 31;
// Row 2 (tiles 32..47): late-game blocks.
pub const T_DOOR: u8 = 32; // plank door with a dark window pane
pub const T_CACTUS: u8 = 33; // ribbed desert green
pub const T_CLAY: u8 = 34; // smooth blue-grey clay
// Cross-sprite plant tiles: index 0 is (0,0,0) = 0x0000 = TRANSPARENT on the
// PS1, so the background texels are not drawn and the plant reads as an
// X-billboard silhouette. Do NOT set p[0] for these in palette_for.
pub const T_CROP_YOUNG: u8 = 35; // short green sprouts (immature wheat)
pub const T_CROP_RIPE: u8 = 36; // golden-headed wheat stalks
pub const T_SAPLING_CROSS: u8 = 37; // little bush/sapling
pub const T_FLOWER_R: u8 = 38; // red flower (stem + blossom)
pub const T_FLOWER_Y: u8 = 39; // yellow flower
pub const T_TALLGRASS: u8 = 40; // fan of grass blades
pub const T_FIRE: u8 = 41; // flame tongues, drawn as a cross-sprite like the plants
pub const T_OBSIDIAN: u8 = 42; // near-black with faint purple glints
// Inferno set.
pub const T_CINDERSTONE: u8 = 43; // dark red, pitted
pub const T_SINK_SAND: u8 = 44; // brown with sunken faces
pub const T_LUMISTONE: u8 = 45; // bright yellow speckle
pub const T_PORTAL: u8 = 46; // purple swirl, drawn as a cross-sprite
pub const T_VOID_STONE: u8 = 47; // pale yellow
// Row 3: Inferno mob faces, and the items they drop.
pub const T_FACE_EMBER: u8 = 48;
pub const T_FACE_WAILER: u8 = 49;
pub const T_FACE_CHARRED: u8 = 50;
// The atlas holds TILE_COUNT = 64 tiles and stopped at 50, so these three cost
// nothing anyone was using. They exist because the wraith, villager and wolf
// used to BORROW another kind's face -- and, because emit_body_box draws the
// hide through the face tile's CLUT, that kind's body colours with it.
pub const T_FACE_WRAITH: u8 = 51;
pub const T_FACE_VILLAGER: u8 = 52;
pub const T_FACE_WOLF: u8 = 53;
/// Four destroy stages, drawn OVER the block you are breaking. Minecraft has
/// ten; four is what reads at 16x16 on a 320x240 display, and they occupy atlas
/// slots that were sitting empty.
pub const T_CRACK0: u8 = 54;
pub const T_CRACK1: u8 = 55;
pub const T_CRACK2: u8 = 56;
pub const T_CRACK3: u8 = 57;
/// Chest side: plank body in a dark frame with the grey latch, so a placed
/// chest stops reading as a plain wood block.
pub const T_CHEST: u8 = 58;
pub const T_CRAFT_TOP: u8 = 59; // crafting table top: the ruled 3x3 work grid
pub const T_CRAFT_SIDE: u8 = 60; // crafting table side: planks with tool marks
/// Pickaxe icon for the HUD tool slot. Background index 0 is transparent
/// (the plant trick); the greys take a per-tier tint at draw time.
pub const T_PICK: u8 = 61;
/// The other three tool heads, drawn in the same neutral greys as the pick so
/// the HUD and the held hand can tint them per tier.
pub const T_AXE: u8 = 62;
pub const T_SHOVEL: u8 = 63;
pub const T_SWORD: u8 = 64;

// VRAM placement: framebuffers own x<320, the font page sits at x=320, so the
// next free 4-bit page column is x=384. CLUTs live in the band at y=256.
const TPAGE_X: u16 = 384;
const TPAGE_Y: u16 = 0;
const CLUT_X: u16 = 384;
const CLUT_Y: u16 = 256;

#[derive(Copy, Clone)]
pub struct BlockTex {
    pub clut: [u16; TILE_COUNT],       // one opaque CLUT per tile
    pub clut_alpha: [u16; TILE_COUNT], // same colours with the STP (blend) bit set
    pub tpage: u16,
}

impl BlockTex {
    pub const EMPTY: Self = Self {
        clut: [0; TILE_COUNT],
        clut_alpha: [0; TILE_COUNT],
        tpage: 0,
    };
}

#[inline]
fn hash(x: i32, y: i32, s: i32) -> u32 {
    let mut v = x
        .wrapping_mul(374761393)
        .wrapping_add(y.wrapping_mul(668265263))
        .wrapping_add(s.wrapping_mul(362437)) as u32;
    v ^= v >> 13;
    v = v.wrapping_mul(1274126177);
    v ^ (v >> 16)
}

#[inline]
fn clampu(v: i32, lo: i32, hi: i32) -> u8 {
    if v < lo {
        lo as u8
    } else if v > hi {
        hi as u8
    } else {
        v as u8
    }
}

// --- per-block palettes ---
//
// Each tile gets its OWN 16-colour CLUT (texels stay 4bpp), so a block uses a full
// 16-shade ramp of its material -- 16 greens for grass, 16 greys for stone -- not
// ~3 shades from one shared palette. ~16x more colours, same texture-page RAM; the
// CLUT is just picked per face (it is already per-primitive on the GPU).

#[inline]
fn lerp8(a: u8, b: u8, i: usize, n: usize) -> u8 {
    (a as i32 + (b as i32 - a as i32) * i as i32 / (n as i32 - 1)) as u8
}
#[inline]
fn ramp(a: (u8, u8, u8), b: (u8, u8, u8), i: usize, n: usize) -> (u8, u8, u8) {
    (lerp8(a.0, b.0, i, n), lerp8(a.1, b.1, i, n), lerp8(a.2, b.2, i, n))
}
/// Fill p[base..base+n] with a gradient a->b.
fn fill(p: &mut [(u8, u8, u8); 16], base: usize, a: (u8, u8, u8), b: (u8, u8, u8), n: usize) {
    let mut i = 0;
    while i < n {
        p[base + i] = ramp(a, b, i, n);
        i += 1;
    }
}

/// The 16-colour palette (one CLUT) for a tile. Tiles the CC0 pack covers use its
/// quantised palette (crate::texdata); the rest fall back to procedural ramps.
fn palette_for(tile: u8) -> [(u8, u8, u8); 16] {
    let ti = tile as usize;
    if ti < 32 && crate::texdata::PACK_HAS[ti] {
        return crate::texdata::PACK_PAL[ti];
    }
    let mut p = [(0u8, 0u8, 0u8); 16];
    match tile {
        T_GRASS_TOP => {
            fill(&mut p, 0, (66, 104, 40), (150, 196, 92), 12);
            p[12] = (58, 92, 36);
            p[13] = (70, 110, 44);
        }
        T_GRASS_SIDE => {
            fill(&mut p, 0, (70, 112, 44), (150, 196, 92), 6);
            fill(&mut p, 6, (150, 108, 74), (96, 66, 44), 9); // dirt shades 6..14
            p[15] = (60, 42, 26);
        }
        T_DIRT => {
            fill(&mut p, 0, (150, 110, 76), (92, 64, 44), 12);
            p[12] = (120, 88, 60);
            p[13] = (134, 100, 68);
            p[14] = (108, 78, 52);
        }
        T_STONE => {
            fill(&mut p, 0, (158, 158, 164), (96, 96, 102), 13);
            p[13] = (120, 120, 126);
            p[14] = (140, 140, 146);
        }
        T_COBBLE => {
            fill(&mut p, 0, (176, 176, 182), (70, 70, 76), 14);
            p[14] = (120, 120, 126);
            p[15] = (34, 34, 40); // dark mortar
        }
        T_SAND => {
            fill(&mut p, 0, (228, 216, 168), (196, 180, 132), 12);
            p[12] = (210, 196, 150);
            p[13] = (176, 158, 116);
        }
        T_WOOD_SIDE => {
            fill(&mut p, 0, (150, 116, 70), (78, 56, 34), 13);
            p[13] = (120, 92, 56);
        }
        T_WOOD_TOP => {
            fill(&mut p, 0, (166, 132, 84), (96, 70, 42), 14);
            p[14] = (120, 92, 56);
        }
        T_PLANK => {
            fill(&mut p, 0, (178, 140, 86), (104, 78, 48), 13);
            p[13] = (150, 116, 72);
        }
        T_LEAVES => {
            fill(&mut p, 0, (40, 80, 28), (110, 168, 72), 12);
            p[12] = (28, 58, 20);
            p[13] = (20, 44, 14);
        }
        T_WATER => {
            // The old 16-step ramp was sampled at indices 8..10 only, three
            // shades 2-3 units apart that collapsed to TWO colours after the
            // PS1's 15-bit truncation -- a dead flat slab. These four are
            // spaced on 8-unit (5-bit) boundaries so they stay distinct.
            p[0] = (48, 100, 208);
            p[1] = (56, 110, 220);
            p[2] = (63, 118, 228);
            p[3] = (80, 136, 240);
        }
        T_LAVA => {
            fill(&mut p, 0, (150, 40, 12), (248, 200, 60), 14);
            p[14] = (90, 20, 8);
            p[15] = (40, 12, 6);
        }
        T_COAL => {
            fill(&mut p, 0, (158, 158, 164), (96, 96, 102), 14);
            p[14] = (46, 46, 50);
            p[15] = (28, 28, 32);
        }
        T_IRON => {
            fill(&mut p, 0, (158, 158, 164), (96, 96, 102), 14);
            p[14] = (198, 152, 112);
            p[15] = (150, 108, 74);
        }
        T_GOLD => {
            fill(&mut p, 0, (158, 158, 164), (96, 96, 102), 14);
            p[14] = (240, 200, 70);
            p[15] = (208, 150, 40);
        }
        T_DIAMOND => {
            fill(&mut p, 0, (158, 158, 164), (96, 96, 102), 14);
            p[14] = (120, 222, 226);
            p[15] = (78, 190, 200);
        }
        T_SNOW_TOP => {
            fill(&mut p, 0, (230, 234, 244), (252, 253, 255), 14);
            p[14] = (206, 212, 226);
            p[15] = (220, 226, 238);
        }
        T_SNOW_SIDE => {
            p[0] = (246, 249, 255);
            p[1] = (232, 236, 245);
            p[2] = (218, 224, 236);
            fill(&mut p, 3, (150, 108, 74), (96, 66, 44), 13);
        }
        T_WOOL => {
            fill(&mut p, 0, (224, 226, 232), (252, 253, 255), 15);
            p[15] = (204, 206, 214);
        }
        T_TNT => {
            fill(&mut p, 0, (180, 40, 28), (232, 82, 42), 12);
            p[12] = (242, 242, 246);
            p[13] = (212, 212, 220);
            p[14] = (44, 22, 16);
        }
        T_BRICK => {
            fill(&mut p, 0, (150, 60, 44), (202, 98, 70), 12);
            p[12] = (198, 182, 152); // mortar
            p[13] = (172, 156, 128);
        }
        T_WHEAT_RIPE => {
            fill(&mut p, 0, (150, 120, 40), (240, 212, 92), 14);
            p[14] = (120, 96, 32);
            p[15] = (96, 132, 48);
        }
        T_LADDER => {
            fill(&mut p, 0, (150, 116, 70), (96, 70, 42), 12);
            p[12] = (58, 42, 26);
        }
        T_DOOR => {
            fill(&mut p, 0, (172, 132, 84), (128, 94, 56), 10); // plank body
            p[10] = (84, 60, 36); // frame/joint dark
            p[11] = (60, 42, 26);
            p[12] = (40, 52, 72); // window pane
            p[13] = (66, 86, 112); // pane highlight
        }
        T_CACTUS => {
            fill(&mut p, 0, (58, 118, 44), (94, 160, 70), 12); // ribbed greens
            p[12] = (34, 78, 30); // rib shadow
            p[13] = (140, 196, 110); // rib highlight
            p[14] = (24, 56, 22);
        }
        T_CLAY => {
            fill(&mut p, 0, (144, 150, 162), (172, 178, 190), 14); // blue-grey
            p[14] = (120, 126, 140);
        }
        // Plant crosses: p[0] left (0,0,0) = transparent. Only 1.. carry colour.
        T_CROP_YOUNG => {
            p[1] = (110, 150, 58);
            p[2] = (140, 175, 72);
            p[3] = (90, 128, 50);
        }
        T_CROP_RIPE => {
            p[1] = (120, 150, 60);
            p[2] = (150, 170, 70); // green lower stalk
            p[3] = (200, 180, 80);
            p[4] = (230, 205, 95);
            p[5] = (245, 225, 120); // golden grain
        }
        T_SAPLING_CROSS => {
            p[1] = (92, 64, 40);
            p[2] = (70, 48, 30); // stem
            p[3] = (40, 96, 34);
            p[4] = (64, 130, 50);
            p[5] = (96, 168, 72); // leaves
            p[6] = (28, 66, 24); // leaf shadow
        }
        T_FLOWER_R => {
            p[1] = (58, 120, 44);
            p[2] = (84, 152, 58); // stem/leaves
            p[3] = (200, 50, 44);
            p[4] = (232, 86, 72); // red petals
            p[5] = (244, 222, 96); // yellow centre
        }
        T_FLOWER_Y => {
            p[1] = (58, 120, 44);
            p[2] = (84, 152, 58);
            p[3] = (238, 198, 56);
            p[4] = (250, 226, 104); // yellow petals
            p[5] = (220, 150, 40); // orange centre
        }
        T_TALLGRASS => {
            p[1] = (52, 110, 40);
            p[2] = (74, 140, 54);
            p[3] = (102, 168, 70); // blade greens
            p[4] = (40, 92, 34); // dark base
        }
        T_FACE_EMBER => {
            face_pal(&mut p, [(250,200,60),(232,170,40),(206,132,24),(255,232,140),(180,96,16),(255,255,255),(40,20,0),(255,120,20),(120,50,0)]);
        }
        T_FACE_WAILER => {
            face_pal(&mut p, [(226,226,222),(206,206,202),(182,182,178),(244,244,240),(160,160,156),(255,255,255),(180,30,30),(120,20,20),(60,10,10)]);
        }
        T_FACE_CHARRED => {
            face_pal(&mut p, [(58,60,56),(46,48,44),(34,36,32),(78,80,74),(24,26,22),(200,200,196),(10,10,10),(120,20,20),(60,60,56)]);
        }
        T_VOID_STONE => {
            fill(&mut p, 0, (222, 224, 168), (166, 168, 120), 13);
            p[13] = (140, 142, 100);
            p[14] = (198, 200, 146);
        }
        T_CINDERSTONE => {
            fill(&mut p, 0, (108, 38, 34), (58, 18, 18), 13);
            p[13] = (132, 52, 46);
            p[14] = (40, 12, 12);
        }
        T_SINK_SAND => {
            fill(&mut p, 0, (96, 74, 60), (58, 42, 34), 12);
            p[12] = (44, 30, 24); // sunken face hollows
            p[13] = (112, 88, 70);
        }
        T_LUMISTONE => {
            fill(&mut p, 0, (150, 112, 52), (250, 226, 140), 13);
            p[13] = (255, 248, 200); // hot speck
            p[14] = (110, 80, 36);
        }
        // Portal: index 0 transparent, so the swirl reads as a sheet you see
        // through, like the plants.
        T_PORTAL => {
            p[1] = (58, 18, 96);
            p[2] = (96, 32, 150);
            p[3] = (140, 66, 200);
            p[4] = (186, 128, 236);
        }
        // Near-black, with the faint purple glints the Java texture has.
        T_OBSIDIAN => {
            fill(&mut p, 0, (26, 20, 38), (48, 38, 68), 12);
            p[12] = (72, 56, 104);
            p[13] = (92, 72, 132); // glint
            p[14] = (18, 14, 26);
            p[15] = (10, 8, 16);
        }
        // Flame ramp, hottest at the base: index 0 stays transparent so the
        // tongues read against whatever is burning behind them.
        T_FIRE => {
            p[1] = (120, 24, 8); // dark outer edge
            p[2] = (216, 78, 16);
            p[3] = (246, 150, 30);
            p[4] = (255, 224, 96); // white-hot core
        }
        // Mob face palettes: 0..3 skin shades (0 base, 1/2 darker, 3 lighter),
        // 4 accent, 5..8 features (eyes/snout/detail). Prototyped in python
        // against the minecraft.wiki mob looks (hand-authored, not ripped).
        T_FACE_PIG => {
            face_pal(&mut p, [(238,152,160),(228,138,148),(214,120,132),(246,168,176),(196,100,116),(255,255,255),(30,30,40),(160,60,80),(120,40,60)]);
        }
        T_FACE_COW => {
            face_pal(&mut p, [(96,64,42),(84,54,36),(72,46,30),(110,76,52),(230,226,218),(255,255,255),(30,26,24),(214,160,150),(170,110,104)]);
        }
        T_FACE_SHEEP => {
            face_pal(&mut p, [(226,222,214),(214,208,198),(200,192,180),(238,234,228),(182,172,158),(255,255,255),(40,36,34),(216,186,170),(150,140,130)]);
        }
        T_FACE_CHICKEN => {
            face_pal(&mut p, [(240,236,226),(228,222,210),(214,206,192),(250,248,242),(190,182,168),(255,255,255),(30,30,36),(228,150,40),(200,60,50)]);
        }
        T_FACE_ZOMBIE => {
            face_pal(&mut p, [(70,140,80),(60,124,70),(50,108,60),(84,156,94),(40,90,50),(36,60,40),(20,34,24),(90,170,100),(28,46,32)]);
        }
        T_FACE_SKELETON => {
            face_pal(&mut p, [(198,198,202),(184,184,190),(168,168,176),(214,214,218),(146,146,156),(60,60,68),(36,36,44),(228,228,232),(90,90,100)]);
        }
        T_FACE_SAPPER => {
            face_pal(&mut p, [(58,178,68),(48,158,58),(40,140,50),(70,196,80),(32,120,42),(24,54,28),(12,30,16),(84,214,94),(20,44,24)]);
        }
        T_FACE_SPIDER => {
            face_pal(&mut p, [(46,42,50),(40,36,44),(34,30,38),(54,50,58),(26,24,30),(170,30,34),(210,50,54),(120,20,24),(14,12,16)]);
        }
        T_CHEST => {
            fill(&mut p, 0, (178, 128, 64), (128, 88, 44), 10); // chest planks
            p[10] = (72, 48, 26); // frame
            p[11] = (52, 34, 18); // lid seam
            p[12] = (158, 158, 166); // latch
            p[13] = (94, 94, 102); // latch shadow
        }
        T_CRAFT_TOP => {
            fill(&mut p, 0, (204, 158, 96), (162, 118, 64), 11); // worn tabletop
            p[11] = (96, 64, 34); // frame
            p[12] = (62, 42, 24); // grid lines
        }
        T_CRAFT_SIDE => {
            fill(&mut p, 0, (178, 132, 76), (144, 102, 56), 10); // planks
            p[10] = (96, 64, 34); // frame
            p[11] = (54, 36, 20); // tool dark
            p[12] = (196, 196, 204); // saw-blade grey
            p[13] = (118, 76, 40); // handle brown
        }
        T_PICK | T_AXE | T_SHOVEL | T_SWORD => {
            // Neutral greys; the HUD tints the sprite per tool tier.
            p[1] = (235, 235, 240); // head edge
            p[2] = (192, 192, 200); // head fill
            p[3] = (150, 150, 156); // handle
            p[4] = (120, 120, 128); // sword guard
        }
        // Crack overlay: index 0 is TRANSPARENT (like the plants), so only the
        // fracture lines paint and the block shows through.
        T_CRACK0 | T_CRACK1 | T_CRACK2 | T_CRACK3 => {
            p[1] = (30, 28, 26); // crack line
            p[2] = (16, 15, 14); // fork/joint, darkened
        }
        // Near-black with the magenta eye bar Java gives it.
        T_FACE_WRAITH => {
            face_pal(&mut p, [(20,20,26),(16,16,22),(12,12,18),(26,26,32),(8,8,12),(196,84,220),(228,140,240),(150,60,180),(6,6,8)]);
        }
        // Brown robe, big nose, the heavy brow.
        T_FACE_VILLAGER => {
            face_pal(&mut p, [(150,116,86),(134,102,74),(118,88,64),(168,132,100),(96,70,50),(72,50,34),(52,36,24),(186,150,116),(40,28,18)]);
        }
        // Pale grey coat, dark snout, amber eye.
        T_FACE_WOLF => {
            face_pal(&mut p, [(206,202,196),(188,184,178),(168,164,158),(226,222,216),(140,136,130),(60,56,52),(34,32,30),(236,180,80),(20,18,16)]);
        }
        _ => fill(&mut p, 0, (120, 120, 126), (152, 152, 158), 16),
    }
    p
}

/// Copy a 9-entry mob-face palette into slots 0..8 (9.. stay black; unused).
fn face_pal(p: &mut [(u8, u8, u8); 16], src: [(u8, u8, u8); 9]) {
    let mut i = 0;
    while i < 9 {
        p[i] = src[i];
        i += 1;
    }
}

/// Stone-ramp base index (into an ore/stone tile's grey ramp).
#[inline]
fn stone_ramp(x: i32, y: i32) -> u8 {
    match hash(x, y, 2) % 16 {
        0 => 3,
        1 => 10,
        _ => 6,
    }
}

/// Ore = stone-ramp grey with small (2x2) clustered blobs of `speck`-index ore.
#[inline]
fn ore(x: i32, y: i32, speck: u8, salt: i32) -> u8 {
    if hash(x / 2, y / 2, salt) % 6 == 0 {
        speck
    } else {
        stone_ramp(x, y)
    }
}

/// Cross-sprite flower: stem + two side leaves + a small blossom head. Shared by
/// the red and yellow flowers (they differ only in CLUT). 0 = transparent.
fn flower_texel(x: i32, y: i32) -> u8 {
    if (7..=8).contains(&x) && (6..=14).contains(&y) {
        return if (x + y) % 2 == 0 { 1 } else { 2 }; // stem
    }
    if y == 10 && (4..=6).contains(&x) {
        return 2; // left leaf
    }
    if y == 12 && (9..=11).contains(&x) {
        return 1; // right leaf
    }
    let (dx, dy) = (x - 7, y - 4);
    let d2 = dx * dx + dy * dy;
    if d2 == 0 {
        5 // centre
    } else if d2 <= 2 {
        if hash(x, y, 3) % 4 == 0 { 5 } else { 4 }
    } else if d2 <= 6 {
        if (x + y) & 1 == 1 { 3 } else { 4 } // petals
    } else {
        0
    }
}

/// Cross-sprite tall grass: a fan of thin blades rising from a common base.
/// 0 = transparent. Blade x is `8 + (top_x-8)*(15-y)/(15-top_y)` in 1/16 units.
/// Three flame tongues, wide and hot at the bottom, narrowing and cooling as
/// they rise. Same shape logic as the grass blades, run upside down.
fn fire_texel(x: i32, y: i32) -> u8 {
    if y < 3 {
        return 0; // clear at the very top
    }
    const TONGUES: [i32; 3] = [3, 8, 13];
    let mut best = 0u8;
    let mut slot = 0;
    while slot < 3 {
        let cx = TONGUES[slot];
        let top = 3 + ((slot as i32 & 1) * 3); // stagger the tips
        if y >= top {
            // Half-width grows toward the base.
            let half = 1 + (y - top) * 4 / (15 - top).max(1);
            let d = (x - cx).abs();
            if d <= half {
                // Hotter at the core and lower down.
                let heat = if d == half {
                    1
                } else if d * 2 >= half || y < 8 {
                    2
                } else if y < 12 {
                    3
                } else {
                    4
                };
                if heat > best {
                    best = heat;
                }
            }
        }
        slot += 1;
    }
    best
}

/// Portal sheet: diagonal bands of the purple ramp with holes punched through,
/// so it shimmers rather than reading as flat paint.
fn portal_texel(x: i32, y: i32) -> u8 {
    let band = ((x * 3 + y * 5) & 15) as u32;
    let n = hash(x, y, 57) % 8;
    if n == 0 {
        return 0; // see-through speckle
    }
    if band < 4 {
        4
    } else if band < 8 {
        3
    } else if band < 12 {
        2
    } else {
        1
    }
}

fn tallgrass_texel(x: i32, y: i32) -> u8 {
    if !(2..=15).contains(&y) {
        return 0;
    }
    const TOPS: [i32; 5] = [2, 5, 8, 11, 14];
    let mut best = 0u8;
    let mut slot = 0;
    while slot < 5 {
        let top_x = TOPS[slot];
        let top_y = 2 + (slot as i32 & 1);
        if y >= top_y {
            let denom = 15 - top_y; // > 0
            let ynum = 15 - y; // 0..
            let bx16 = 128 + (top_x - 8) * 16 * ynum / denom; // blade centre * 16
            let thr = if 10 * ynum > 6 * denom { 8 } else { 14 }; // thinner near the tip
            if (x * 16 - bx16).abs() <= thr {
                let shade = if y > 12 { 4 } else { 1 + (hash(x, y, 11 + slot as i32) % 3) as u8 };
                if shade > best {
                    best = shade;
                }
            }
        }
        slot += 1;
    }
    best
}

/// Palette index (into this tile's own CLUT) for texel (x,y). Hand-authored to
/// match the classic Minecraft block art -- grass drips, log rings, cobble stones,
/// plank seams -- now over a full per-block shade ramp. Pure: the capture is the test.
fn texel(tile: u8, x: usize, y: usize) -> u8 {
    let ti = tile as usize;
    if ti < 32 && crate::texdata::PACK_HAS[ti] {
        return crate::texdata::PACK_IDX[ti][y * 16 + x];
    }
    let xi = x as i32;
    let yi = y as i32;
    match tile {
        T_GRASS_TOP => clampu(4 + (hash(xi, yi, 0) % 14) as i32 % 5 - 2, 0, 11),
        T_GRASS_SIDE => {
            // Green cap (0..5) with a jagged bottom edge, then shaded dirt (6..14).
            const DEPTH: [usize; 16] = [3, 4, 3, 4, 5, 3, 4, 3, 5, 4, 3, 4, 3, 4, 5, 3];
            if y < 3 || y < DEPTH[x] {
                clampu(1 + (hash(xi, yi, 1) % 6) as i32 % 4 - 1, 0, 5)
            } else {
                clampu(6 + (yi - 3) / 2 + (hash(xi, yi, 9) % 3) as i32 - 1, 6, 14)
            }
        }
        T_DIRT => clampu(3 + (hash(xi, yi, 9) % 7) as i32 - 3, 0, 11),
        T_STONE => {
            let n = hash(xi, yi, 2) % 20;
            let mut b = 6 + (hash(xi, yi, 5) % 3) as i32 - 1;
            if n == 0 {
                b = 11;
            } else if n == 1 {
                b = 2;
            }
            if (x + y) % 16 == 5 && hash(xi, 0, 7) % 2 == 0 {
                b = 12; // faint diagonal crack
            }
            clampu(b, 0, 12)
        }
        T_WOOD_TOP => {
            // Concentric growth rings (alternating shades) around a centre knot.
            let d = isqrt_i32((xi - 8) * (xi - 8) + (yi - 8) * (yi - 8));
            if d < 2 {
                13
            } else {
                (if d % 2 == 0 { 2 } else { 9 }) + (hash(xi, yi, 3) % 2) as u8
            }
        }
        T_WOOD_SIDE => {
            // Vertical bark stripes (varying darkness) with occasional grain.
            const COL: [u8; 16] = [10, 3, 4, 10, 5, 12, 4, 10, 3, 5, 10, 2, 4, 10, 3, 12];
            let mut c = COL[x] as i32;
            if hash(xi, yi, 3) % 9 == 0 {
                c = (c + 3).min(12);
            }
            c as u8
        }
        T_LEAVES => match hash(xi, yi, 4) % 12 {
            0 => 13,
            1 => 12,
            _ => clampu(4 + (hash(xi / 2, yi / 2, 9) % 6) as i32, 0, 11),
        },
        T_SAND => {
            if hash(xi, yi, 5) % 14 == 0 {
                12
            } else {
                clampu(4 + (hash(xi / 2, yi / 2, 6) % 4) as i32 - 2, 0, 11)
            }
        }
        T_WATER => {
            // Coarse horizontal banding with a slow phase shift across x, so the
            // surface reads as a swell instead of per-texel static.
            let band = (y as i32 + ((x as i32) >> 2)) & 7;
            let n = hash(xi >> 1, yi >> 1, 5) % 4;
            (if band < 2 { 3 } else if band < 4 { 2 } else if n == 0 { 0 } else { 1 }) as u8
        }
        T_COAL => ore(xi, yi, 15, 21),
        T_IRON => ore(xi, yi, 14, 22),
        T_GOLD => ore(xi, yi, 14, 23),
        T_DIAMOND => ore(xi, yi, 14, 24),
        T_LAVA => {
            if hash(xi, yi, 32) % 12 == 0 {
                15 // dark crust
            } else {
                (hash(xi / 2, yi / 2, 31) % 14) as u8
            }
        }
        T_SNOW_TOP => {
            if hash(xi, yi, 41) % 9 == 0 {
                14
            } else {
                (hash(xi, yi, 42) % 6) as u8
            }
        }
        T_SNOW_SIDE => {
            if y < 4 {
                (hash(xi, yi, 43) % 3) as u8
            } else {
                clampu(3 + (yi - 4) / 2 + (hash(xi, yi, 9) % 3) as i32 - 1, 3, 14)
            }
        }
        // ---- Row 1 ----
        T_COBBLE => {
            // 2x2 grid of rounded stones (AO-shaded) separated by dark mortar.
            let cx = x % 8;
            let cy = y % 8;
            if cx == 0 || cy == 0 {
                15
            } else {
                let dx = cx as i32 - 4;
                let dy = cy as i32 - 4;
                if dx * dx + dy * dy >= 11 {
                    13
                } else {
                    clampu(3 + (cy as i32 - 1) + (hash(xi, yi, 3) % 3) as i32 - 1, 0, 13)
                }
            }
        }
        T_PLANK => {
            if y % 4 == 3 {
                12 // dark seam
            } else if x % 8 == 7 {
                11 // vertical joint
            } else if hash(xi, yi, 17) % 9 == 0 {
                10 // knot
            } else {
                clampu(3 + (hash(xi / 3, yi, 8) % 3) as i32, 0, 12)
            }
        }
        T_WOOL => {
            if hash(xi, yi, 18) % 8 == 0 {
                15
            } else {
                (hash(xi, yi, 19) % 6) as u8
            }
        }
        T_TNT => {
            if (6..10).contains(&y) {
                if hash(xi, yi, 19) % 5 == 0 {
                    13
                } else {
                    12 // white band
                }
            } else if hash(xi, yi, 19) % 7 == 0 {
                14 // dark fleck
            } else {
                clampu(2 + (hash(xi, yi, 20) % 6) as i32, 0, 11)
            }
        }
        T_BRICK => {
            let row = (y / 4) as i32;
            let off = (row % 2) * 4;
            if y % 4 == 3 || (xi + off) % 8 == 0 {
                12 // mortar
            } else {
                clampu(2 + (hash(xi, yi, 21) % 6) as i32, 0, 11)
            }
        }
        T_WHEAT_RIPE => match x % 3 {
            1 => clampu(9 + (hash(xi, yi, 22) % 4) as i32, 0, 13),
            2 => clampu(4 + (hash(xi, yi, 22) % 3) as i32, 0, 13),
            _ => 2,
        },
        T_LADDER => {
            if x < 3 || x >= 13 {
                (hash(xi, yi, 23) % 3) as u8 // side rails
            } else if y % 5 < 2 {
                6 // rung
            } else {
                12 // dark gap
            }
        }
        T_DOOR => {
            if x == 0 || x == 15 || y == 0 || y == 15 {
                10 // outer frame
            } else if y == 7 || y == 8 || x == 7 || x == 8 {
                11 // cross joints
            } else if x >= 3 && x <= 6 && y >= 2 && y <= 6 {
                if (x + y) % 4 == 0 { 13 } else { 12 } // window pane (upper-left)
            } else {
                clampu((hash(xi, yi, 24) % 6) as i32 + (y as i32 / 8), 0, 9) // planks
            }
        }
        T_CACTUS => {
            // Vertical ribs: alternating light/dark columns with spine dots.
            let rib = x % 4;
            if rib == 0 {
                12 // rib shadow
            } else if rib == 2 && y % 5 == 2 {
                14 // spine
            } else if rib == 3 {
                13 // rib highlight
            } else {
                clampu((hash(xi, yi, 25) % 8) as i32, 0, 11)
            }
        }
        T_CHEST => {
            if x == 0 || x == 15 || y == 0 || y == 15 {
                10 // frame
            } else if (6..=9).contains(&x) && (5..=10).contains(&y) {
                if y <= 7 { 12 } else { 13 } // latch plate crossing the lid seam
            } else if y == 7 || y == 8 {
                11 // lid seam
            } else {
                clampu((hash(xi, yi, 40) % 6) as i32 + (y as i32 / 8), 0, 9)
            }
        }
        T_CRAFT_TOP => {
            if x == 0 || x == 15 || y == 0 || y == 15 {
                11 // frame
            } else if x == 5 || x == 10 || y == 5 || y == 10 {
                12 // the 3x3 work grid ruled onto the top
            } else {
                clampu((hash(xi, yi, 41) % 8) as i32, 0, 10)
            }
        }
        T_CRAFT_SIDE => {
            if x == 0 || x == 15 || y == 0 || y == 15 {
                10 // frame
            } else if (3..=6).contains(&x) && (3..=6).contains(&y) {
                if x + y < 10 { 12 } else { 11 } // saw: blade over dark grip
            } else if (9..=12).contains(&x) && (4..=7).contains(&y) {
                if y <= 5 { 11 } else { 13 } // hammer: dark head, brown handle
            } else {
                clampu((hash(xi, yi, 42) % 6) as i32 + 2, 0, 9)
            }
        }
        T_PICK | T_AXE | T_SHOVEL | T_SWORD => {
            // Hand-authored heads: 1 edge, 2 fill, 3 handle, 4 guard. All four
            // share a down-left handle so they read as a matched set.
            // The pickaxe was the odd one out and read as broken rather than
            // stylised -- an itch player called it "cursed". Its two prongs
            // were different lengths and different widths, the right one just
            // stopped mid-air, and the handle started INSIDE the head instead
            // of below it. Redrawn: a symmetric arc, two matched prongs, and
            // the handle emerging from the hollow under the middle.
            const PICK: [&[u8; 16]; 16] = [
                b"................", b"......1111......", b"....12222221....",
                b"..122222222221..", b"..122......221..", b"..12.....33.21..",
                b"..1.....33...1..", b".......33.......", b"......33........",
                b".....33.........", b"....33..........", b"...33...........",
                b"..33............", b".33.............", b"33..............",
                b"................",
            ];
            const AXE: [&[u8; 16]; 16] = [
                b".......11111....", b"......1222221...", b".....122222221..",
                b".....122222221..", b".....12222221...", b"....1222.221....",
                b"....12..33......", b"....1..33.......", b".....33.........",
                b"....33..........", b"...33...........", b"..33............",
                b".33.............", b"33..............", b"3...............",
                b"................",
            ];
            const SHOVEL: [&[u8; 16]; 16] = [
                b"........1111....", b".......122221...", b".......122221...",
                b".......122221...", b".......122221...", b"........1221....",
                b".........33.....", b"........33......", b".......33.......",
                b"......33........", b".....33.........", b"....33..........",
                b"...33...........", b"..33............", b".33.............",
                b"................",
            ];
            const SWORD: [&[u8; 16]; 16] = [
                b"............11..", b"...........1221.", b"..........12221.",
                b".........12221..", b"........12221...", b".......12221....",
                b"......12221.....", b".....12221......", b"....12221.......",
                b"...4444.........", b"..44.33.........", b"...33...........",
                b"..33............", b".33.............", b"33..............",
                b"................",
            ];
            let map = match tile {
                T_AXE => &AXE,
                T_SHOVEL => &SHOVEL,
                T_SWORD => &SWORD,
                _ => &PICK,
            };
            let c = map[y][x];
            if c == b'.' { 0 } else { c - b'0' }
        }
        T_CLAY => clampu((hash(xi / 2, yi / 2, 26) % 10) as i32, 0, 13),
        // Cross-sprite plants. 0 = transparent background; the shape is drawn in
        // 1.. . Read as an X-billboard once mapped onto two diagonal quads.
        T_CROP_YOUNG => {
            if x % 3 != 2 || y < 8 {
                0 // short sprouts on 5 columns, lower half only
            } else {
                clampu(1 + (hash(xi, yi, 35) % 3) as i32, 1, 3)
            }
        }
        T_CROP_RIPE => {
            if x % 3 != 2 || y < 2 {
                0 // 5 tall stalks: green low, golden grain up top
            } else {
                let base = if y > 10 { 1 } else if y > 5 { 3 } else { 4 };
                clampu(base + (hash(xi, yi, 36) % 3) as i32, 1, 5)
            }
        }
        T_SAPLING_CROSS => {
            if (7..=8).contains(&x) && y >= 9 {
                if (x + y) % 2 == 0 { 1 } else { 2 } // stem
            } else {
                let dx = xi - 8;
                let dy = yi - 7;
                if dx * dx + dy * dy <= 26 && y <= 11 {
                    if hash(xi, yi, 37) % 8 == 0 {
                        6 // dark leaf fleck
                    } else {
                        clampu(3 + (hash(xi / 2, yi / 2, 3) % 3) as i32, 3, 5) // canopy
                    }
                } else {
                    0
                }
            }
        }
        T_FLOWER_R | T_FLOWER_Y => flower_texel(xi, yi),
        T_TALLGRASS => tallgrass_texel(xi, yi),
        T_FIRE => fire_texel(xi, yi),
        T_CINDERSTONE => {
            // Near-flat base with deliberate 2x2 pits, not per-texel static.
            let c = hash(xi >> 1, yi >> 1, 51) % 20;
            if c == 0 {
                14 // deep pit
            } else if c < 3 {
                13 // bright fleck
            } else {
                (4 + (hash(xi >> 1, yi >> 1, 52) % 3)) as u8
            }
        }
        T_SINK_SAND => {
            // Two sunken hollows per tile, over a grainy base.
            let (fx, fy) = (x % 8, y % 8);
            let d = (fx - 4) * (fx - 4) + (fy - 4) * (fy - 4);
            if d <= 3 {
                12
            } else if d <= 6 {
                (8 + hash(xi, yi, 53) % 3) as u8
            } else {
                (hash(xi, yi, 54) % 8) as u8
            }
        }
        T_LUMISTONE => {
            // Clustered hot cells on a warm base -- Java's lumistone is lumpy,
            // not sparkly. Delta was 59.5 per texel; this keeps it in the base
            // band except at the clusters.
            let c = hash(xi >> 1, yi >> 1, 55) % 9;
            if c == 0 {
                13 // hot core
            } else if c < 3 {
                (10 + (hash(xi >> 1, yi >> 1, 57) % 3)) as u8
            } else if c == 8 {
                14 // shadowed crevice between cells
            } else {
                (6 + (hash(xi >> 1, yi >> 1, 56) % 3)) as u8
            }
        }
        T_PORTAL => portal_texel(xi, yi),
        T_VOID_STONE => {
            // Pale and nearly flat with sparse 2x2 mottling, as Java's is.
            let c = hash(xi >> 1, yi >> 1, 61) % 14;
            if c == 0 {
                13
            } else if c < 3 {
                14
            } else {
                (2 + (hash(xi >> 1, yi >> 1, 62) % 3)) as u8
            }
        }
        T_OBSIDIAN => {
            // Near-black with rare purple glints, clustered so they read as
            // facets rather than sparkle.
            let c = hash(xi >> 1, yi >> 1, 44) % 24;
            if c == 0 {
                13 // glint
            } else if c < 3 {
                12
            } else if c < 6 {
                14
            } else {
                (hash(xi >> 1, yi >> 1, 45) % 4) as u8
            }
        }
        // The wraith/villager/wolf arms were added to `mob_face` and to the
        // palette table but NOT here, so all three fell through to `_ => 6` and
        // wore a flat index-6 sticker: near-black for the wolf and the
        // villager, magenta for the wraith. That is the "mobs have black
        // faces" report from itch.
        T_FACE_PIG | T_FACE_COW | T_FACE_SHEEP | T_FACE_CHICKEN | T_FACE_ZOMBIE
        | T_FACE_SKELETON | T_FACE_SAPPER | T_FACE_SPIDER | T_FACE_EMBER | T_FACE_WAILER
        | T_FACE_CHARRED | T_FACE_WRAITH | T_FACE_VILLAGER | T_FACE_WOLF | T_CRACK0
        | T_CRACK1 | T_CRACK2 | T_CRACK3 => mob_face(tile, x, y),
        T_HIDE => {
            // Soft hide mottle over skin indices 0..3 (denser than the face
            // base dither so bodies read as fur/hide, not flat paint).
            let c = (x / 2) + (y / 2);
            let hn = hash(xi, yi, 31) % 8;
            if hn == 0 {
                2
            } else if c % 3 == 1 || hn == 1 {
                1
            } else if c % 3 == 2 && (x + y) % 2 == 0 {
                3
            } else {
                0
            }
        }
        _ => 6,
    }
}

/// Mob face texels: feature rectangles over a subtly dithered skin base.
/// Checks run in reverse draw order (later features win), mirroring the
/// python prototype (scratch mobfaces.py) these were designed in.
fn mob_face(tile: u8, x: usize, y: usize) -> u8 {
    #[inline]
    fn r(x: usize, y: usize, x0: usize, y0: usize, x1: usize, y1: usize) -> bool {
        x >= x0 && x <= x1 && y >= y0 && y <= y1
    }
    match tile {
        T_FACE_PIG => {
            if r(x, y, 6, 10, 6, 11) || r(x, y, 9, 10, 9, 11) {
                return 8; // nostrils
            }
            if r(x, y, 5, 9, 10, 12) {
                return 3; // snout light
            }
            if r(x, y, 4, 8, 11, 13) {
                return 7; // snout plate
            }
            if r(x, y, 4, 4, 4, 6) || r(x, y, 11, 4, 11, 6) {
                return 6; // pupils
            }
            if r(x, y, 2, 4, 4, 6) || r(x, y, 11, 4, 13, 6) {
                return 5; // eye whites
            }
        }
        T_FACE_COW => {
            if r(x, y, 5, 11, 5, 12) || r(x, y, 10, 11, 10, 12) {
                return 8; // nostrils
            }
            if r(x, y, 3, 9, 12, 15) {
                return 7; // pink muzzle
            }
            if r(x, y, 4, 4, 4, 6) || r(x, y, 11, 4, 11, 6) {
                return 6;
            }
            if r(x, y, 2, 4, 4, 6) || r(x, y, 11, 4, 13, 6) {
                return 5;
            }
            if r(x, y, 0, 0, 15, 3) {
                return 4; // white blaze
            }
        }
        T_FACE_SHEEP => {
            if r(x, y, 7, 10, 8, 12) {
                return 8; // nose shadow
            }
            if r(x, y, 4, 6, 5, 8) || r(x, y, 10, 6, 11, 8) {
                return 6; // eyes
            }
            if r(x, y, 3, 3, 12, 12) {
                return 7; // pale face patch
            }
        }
        T_FACE_CHICKEN => {
            if r(x, y, 6, 10, 9, 12) {
                return 8; // wattle
            }
            if r(x, y, 6, 6, 9, 9) {
                return 7; // beak
            }
            if r(x, y, 3, 3, 5, 5) || r(x, y, 10, 3, 12, 5) {
                return 6; // eyes
            }
        }
        T_FACE_ZOMBIE => {
            if r(x, y, 6, 13, 7, 13) {
                return 8;
            }
            if r(x, y, 6, 10, 9, 12) {
                return 5; // mouth
            }
            if r(x, y, 2, 5, 5, 7) || r(x, y, 10, 5, 13, 7) {
                return 6; // sunken eyes
            }
        }
        T_FACE_SKELETON => {
            if y >= 12 && y <= 13 && (4..12).contains(&x) && x % 2 == 0 {
                return 8; // teeth ticks
            }
            if r(x, y, 3, 12, 12, 13) {
                return 5; // grim mouth
            }
            if r(x, y, 7, 8, 8, 10) {
                return 5; // nose hole
            }
            if r(x, y, 2, 4, 5, 7) || r(x, y, 10, 4, 13, 7) {
                return 6; // sockets
            }
        }
        // Destroy stages, hand-authored (python prototype, session scratch;
        // faithful to Java's destroy_stage strip compressed to 4 frames). One
        // map, digit = the stage a texel appears at, so the crack strictly
        // GROWS: fissure -> spanning crack -> web -> shatter. Drawn OPAQUE
        // (index 0 = texel 0x0000, which the GPU never paints); an Average
        // blend of these was invisible at 320x240.
        T_CRACK0 | T_CRACK1 | T_CRACK2 | T_CRACK3 => {
            const MAP: [&[u8; 16]; 16] = [
                b"......1..3....3.",
                b"......1..3...1..",
                b"2......0...11...",
                b".22.....0.1....2",
                b"...22...0....22.",
                b"....3220.2222...",
                b"....3..0....3...",
                b"..3...0.0.33....",
                b"...3.0...0.....3",
                b".3.31.0...0..33.",
                b"3..1..0....11...",
                b"..1..0.22....1..",
                b"..1..0...23..1..",
                b".12...0..2.3..1.",
                b"..2..3.1...3...2",
                b".2..3..1.......2",
            ];
            let stage = tile - T_CRACK0; // 0..3
            let (xi, yi) = (x as i32, y as i32);
            let lit = |xx: i32, yy: i32| -> bool {
                if xx < 0 || xx > 15 || yy < 0 || yy > 15 {
                    return false;
                }
                let c = MAP[yy as usize][xx as usize];
                c != b'.' && c - b'0' <= stage
            };
            if !lit(xi, yi) {
                return 0;
            }
            // A texel with 3+ lit neighbours is a fork/joint: darken it so the
            // web gains depth where cracks meet.
            let mut n = 0;
            let mut dy = -1;
            while dy <= 1 {
                let mut dx = -1;
                while dx <= 1 {
                    if (dx != 0 || dy != 0) && lit(xi + dx, yi + dy) {
                        n += 1;
                    }
                    dx += 1;
                }
                dy += 1;
            }
            return if n >= 3 { 2 } else { 1 };
        }
        // Wraith: a tall black head, and the magenta eye bar that IS the read.
        T_FACE_WRAITH => {
            if r(x, y, 1, 6, 6, 8) || r(x, y, 9, 6, 14, 8) {
                return 5; // eye bar, left and right
            }
            if r(x, y, 2, 7, 5, 7) || r(x, y, 10, 7, 13, 7) {
                return 6; // hot core of each eye
            }
            if r(x, y, 6, 6, 9, 8) {
                return 4; // the dark bridge between them
            }
            if r(x, y, 0, 0, 15, 2) {
                return 2; // shadowed crown
            }
            return (hash(x as i32, y as i32, 71) % 3) as u8;
        }
        // Villager: heavy brow, long nose, the folded arms are on the body.
        T_FACE_VILLAGER => {
            if r(x, y, 6, 6, 9, 12) {
                return 5; // the nose, front and centre
            }
            if r(x, y, 6, 11, 9, 12) {
                return 6; // its shadow
            }
            if r(x, y, 2, 4, 5, 5) || r(x, y, 10, 4, 13, 5) {
                return 8; // brow
            }
            if r(x, y, 3, 6, 4, 7) || r(x, y, 11, 6, 12, 7) {
                return 6; // eyes
            }
            if r(x, y, 0, 0, 15, 2) {
                return 4; // hairline
            }
            return (hash(x as i32, y as i32, 72) % 4) as u8;
        }
        // Wolf: dark snout down the middle, amber eyes, ears at the corners.
        T_FACE_WOLF => {
            if r(x, y, 6, 8, 9, 15) {
                return 5; // snout
            }
            if r(x, y, 7, 12, 8, 14) {
                return 6; // nose
            }
            if r(x, y, 3, 6, 4, 7) || r(x, y, 11, 6, 12, 7) {
                return 7; // amber eyes
            }
            if r(x, y, 0, 0, 2, 4) || r(x, y, 13, 0, 15, 4) {
                return 4; // ears
            }
            return (hash(x as i32, y as i32, 73) % 4) as u8;
        }
        T_FACE_SAPPER => {
            if r(x, y, 6, 12, 9, 12) {
                return 5;
            }
            if r(x, y, 4, 9, 11, 9) {
                return 8;
            }
            if r(x, y, 4, 10, 5, 14) || r(x, y, 10, 10, 11, 14) {
                return 6; // mouth drop sides
            }
            if r(x, y, 6, 8, 9, 11) {
                return 6; // mouth core
            }
            if r(x, y, 2, 3, 6, 7) || r(x, y, 9, 3, 13, 7) {
                return 6; // THE eyes
            }
        }
        // Ember: burning rods around a bright core, and slit eyes.
        T_FACE_EMBER => {
            if r(x, y, 5, 5, 6, 7) || r(x, y, 9, 5, 10, 7) {
                return 6; // dark slit eyes
            }
            if r(x, y, 6, 10, 9, 11) {
                return 7; // mouth glow
            }
            if x < 3 || x > 12 || y < 3 || y > 12 {
                return 4; // outer rods, darker
            }
            return 3; // hot core
        }
        // Wailer: the flat white face with the closed slit eyes and frown.
        T_FACE_WAILER => {
            if r(x, y, 3, 5, 5, 6) || r(x, y, 10, 5, 12, 6) {
                return 6; // closed eyes
            }
            if r(x, y, 5, 10, 10, 11) {
                return 7; // frown
            }
            return 0;
        }
        // Charred skeleton: a skull, darker and squarer than the plain skeleton.
        T_FACE_CHARRED => {
            if r(x, y, 4, 5, 5, 7) || r(x, y, 10, 5, 11, 7) {
                return 6; // sunken sockets
            }
            if r(x, y, 6, 10, 9, 10) || r(x, y, 6, 12, 9, 12) {
                return 6; // teeth rows
            }
            if r(x, y, 7, 8, 8, 9) {
                return 4; // nasal
            }
            return 0;
        }
        T_FACE_SPIDER => {
            if r(x, y, 4, 10, 11, 11) {
                return 8; // mandible line
            }
            if r(x, y, 7, 7, 8, 8) {
                return 5; // centre eyes
            }
            if r(x, y, 5, 4, 6, 5) || r(x, y, 9, 4, 10, 5) {
                return 6; // big eyes
            }
            if r(x, y, 2, 5, 3, 6) || r(x, y, 12, 5, 13, 6) {
                return 5; // outer eyes
            }
        }
        _ => {}
    }
    // Skin base with a subtle 2x2 dither so the face isn't flat.
    let c = (x / 2) + (y / 2);
    if c % 3 == 1 {
        1
    } else if c % 3 == 2 && (x + y) % 2 == 0 {
        3
    } else {
        0
    }
}

/// Texel-space UV origin of a tile within the shared page (2D: row 0 or 1).
#[inline]
pub fn tile_uv(tile: u8) -> (u8, u8) {
    let t = tile as usize;
    (((t % TILES_PER_ROW) * TILE) as u8, ((t / TILES_PER_ROW) * TILE) as u8)
}

/// Build the atlas + CLUT and upload both to VRAM. Call once at boot.
pub fn upload() -> BlockTex {
    // Pack 4-bit indices: even x -> low nibble, odd x -> high nibble. Two tile
    // rows now, so iterate the full ATLAS_H and pick the tile from (x,y).
    // Build and upload ONE tile-row at a time. A whole-atlas buffer used to sit
    // on the stack, and the linker only reserves 32 KiB for it; a 2 KiB row
    // scratch keeps that cost flat no matter how many rows the atlas grows to.
    let tpage = Tpage::new(TPAGE_X, TPAGE_Y, TexDepth::Bit4);
    let row_bytes = ATLAS_W / 2;
    let mut row = [0u8; ATLAS_W * TILE / 2];
    let mut ty = 0;
    while ty < TILE_COUNT / TILES_PER_ROW {
        let mut i = 0;
        while i < row.len() {
            row[i] = 0;
            i += 1;
        }
        let mut y = 0;
        while y < TILE {
            let mut x = 0;
            while x < ATLAS_W {
                let tile = (ty * TILES_PER_ROW + x / TILE) as u8;
                let idx = texel(tile, x % TILE, y) & 0x0F;
                let byte = y * row_bytes + x / 2;
                row[byte] |= if x & 1 == 0 { idx } else { idx << 4 };
                x += 1;
            }
            y += 1;
        }
        upload_bytes(
            VramRect::new(TPAGE_X, TPAGE_Y + (ty * TILE) as u16, (ATLAS_W / 4) as u16, TILE as u16),
            &row,
        );
        ty += 1;
    }

    // One CLUT per tile: normal CLUTs stack at rows CLUT_Y.., the STP (blend)
    // variants at CLUT_Y + TILE_COUNT.. . Each is 16 texels wide (32 bytes).
    let mut bt = BlockTex::EMPTY;
    bt.tpage = tpage.uv_tpage_word(0);
    let mut t = 0;
    while t < TILE_COUNT {
        let pal = palette_for(t as u8);
        let mut clut = [0u8; 32];
        let mut clut_a = [0u8; 32];
        let mut i = 0;
        while i < 16 {
            let base = Color555::rgb8(pal[i].0, pal[i].1, pal[i].2);
            let c = base.as_u16();
            clut[i * 2] = c as u8;
            clut[i * 2 + 1] = (c >> 8) as u8;
            let a = base.with_mask_bit().as_u16();
            clut_a[i * 2] = a as u8;
            clut_a[i * 2 + 1] = (a >> 8) as u8;
            i += 1;
        }
        let ny = CLUT_Y + t as u16;
        let ay = CLUT_Y + TILE_COUNT as u16 + t as u16;
        upload_bytes(VramRect::new(CLUT_X, ny, 16, 1), &clut);
        upload_bytes(VramRect::new(CLUT_X, ay, 16, 1), &clut_a);
        bt.clut[t] = Clut::new(CLUT_X, ny).uv_clut_word();
        bt.clut_alpha[t] = Clut::new(CLUT_X, ay).uv_clut_word();
        t += 1;
    }
    bt
}

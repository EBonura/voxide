//! Chunked, streamed voxel world.
//!
//! A GRID x GRID ring of 16x16x64 chunks is kept loaded around the player and
//! recentred (toroidally) as they move, so the world is effectively infinite
//! while only ~25 chunks live in RAM. Terrain is seeded fixed-point value noise
//! -> biome -> height + caves + ores + lava + trees. Each chunk caches its
//! visible-face mesh (rebuilt on gen/edit, never per frame); the renderer walks
//! only chunks that pass a cheap distance + horizontal-frustum cull.

use crate::{
    floor_div, Camera, AIR, BLOCK, CACTUS, CLAY, COAL_ORE, DIAMOND_ORE, DIRS, DIRT, DOOR_O, FAR_SIDE_Z,
    FAR_Z, FLOWER_R, FLOWER_Y, GOLD_ORE, GRASS, IRON_ORE, LAVA, LEAVES, PROJ_H, SAND, SAPLING, SNOW,
    STONE, TALL_GRASS, WATER, WHEAT, WHEAT_RIPE, WOOD,
};
use crate::{
    is_flammable, VOID_STONE, FIRE, LUMISTONE, CINDERSTONE, EMBER_CAP, OBSIDIAN, PORTAL, SINK_SAND,
    SUGAR_CANE, TORCH,
};
use crate::{
    is_lava, is_small_block, is_water, lava_level, lava_of_level, water_level, water_of_level,
    COBBLE, LAVA_MAX_RUN, WATER_MAX_RUN,
};
use psx_gte::math::Vec3I16;
use psx_gte::scene;
use psx_math::int32::isqrt_i32;

/// A cross-sprite plant (wheat/sapling/flower/tall grass): rendered as an
/// X-billboard, never as a meshed cube. `face_here` treats these as see-through
/// so they neither emit cube faces nor hide the block behind them; `commit_mesh`
/// records their cells into the chunk's plant list for the render pass.
#[inline]
pub fn is_cross_plant(b: u8) -> bool {
    b == WHEAT
        || b == WHEAT_RIPE
        || b == SAPLING
        || b == FLOWER_R
        || b == FLOWER_Y
        || b == TALL_GRASS
        || b == FIRE
        || b == PORTAL
        || b == EMBER_CAP
        || b == SUGAR_CANE
}

pub const CW: i32 = 16;
pub const CH: i32 = 64;
const CWU: usize = 16;
const CHU: usize = 64;
const CHUNK_VOL: usize = CWU * CWU * CHU; // 16384
pub const GRID: i32 = 5; // 5x5 loaded ring (~80 blocks)
const GRIDU: usize = 5;
const NCHUNKS: usize = GRIDU * GRIDU; // 25  GRID fits RAM but boots slow -- needs a screen.
const MAX_FACES: usize = 3000; // headroom: meshing the sea floor (faces vs water) adds faces/chunk

// Blocks pack to 5 bits each (we have 24 block kinds; 5 bits hold 32). 16384
// blocks * 5 bits = 10240 bytes/chunk vs 16384 at one byte each -- 37.5% less
// RAM, so the loaded ring can hold more chunks. +1 byte so a 5-bit field at the
// last index can be read as a 2-byte window without overrunning.
// 7-bit block ids (0..127). 5 bits (24 kinds) ran out, then 6 did too: the
// water levels took 54..60 and slabs/fences 61..62, leaving no room for the
// four stair facings or the lava levels. 7 costs ~2KB/chunk (25 loaded chunks,
// so ~51KB of 2MB) and buys 65 spare ids.
//
// The 16-bit bget/bset window still works: a 7-bit field at bit offset 0..7
// spans at most bits 0..14. `pack_blocks` is the one place that cares about the
// width beyond these constants, because it packs whole bytes.
const PACKED_SIZE: usize = (CHUNK_VOL * 7 + 7) / 8 + 1; // 14337
const BLOCK_BITS: usize = 7;
const BLOCK_MASK: u32 = 0x7F;

/// Read the 5-bit block at flat index `i` from a packed chunk store.
#[inline]
fn bget(data: &[u8; PACKED_SIZE], i: usize) -> u8 {
    let bit = i * BLOCK_BITS;
    let byte = bit >> 3;
    let off = bit & 7;
    let win = data[byte] as u32 | ((data[byte + 1] as u32) << 8);
    ((win >> off) & BLOCK_MASK) as u8
}

/// Decode the eight blocks of group `g` (blocks 8g..8g+8). Eight 7-bit fields
/// are exactly seven bytes, so a group never straddles a byte and the eight
/// values come out of two little-endian windows with constant shifts: seven
/// byte loads instead of sixteen, and no per-block multiply by 7.
#[inline(always)]
fn bget8(data: &[u8; PACKED_SIZE], g: usize) -> [u8; 8] {
    let p = g * 7;
    let d = &data[p..p + 7];
    let lo = d[0] as u32 | (d[1] as u32) << 8 | (d[2] as u32) << 16 | (d[3] as u32) << 24;
    let hi = d[3] as u32 | (d[4] as u32) << 8 | (d[5] as u32) << 16 | (d[6] as u32) << 24;
    [
        (lo & 0x7F) as u8,
        ((lo >> 7) & 0x7F) as u8,
        ((lo >> 14) & 0x7F) as u8,
        ((lo >> 21) & 0x7F) as u8,
        ((hi >> 4) & 0x7F) as u8,
        ((hi >> 11) & 0x7F) as u8,
        ((hi >> 18) & 0x7F) as u8,
        ((hi >> 25) & 0x7F) as u8,
    ]
}

/// Write the 5-bit block `v` at flat index `i` into a packed chunk store.
#[inline]
fn bset(data: &mut [u8; PACKED_SIZE], i: usize, v: u8) {
    let bit = i * BLOCK_BITS;
    let byte = bit >> 3;
    let off = bit & 7;
    let mask = BLOCK_MASK << off;
    let win = data[byte] as u32 | ((data[byte + 1] as u32) << 8);
    let win = (win & !mask) | (((v as u32) & BLOCK_MASK) << off);
    data[byte] = win as u8;
    data[byte + 1] = (win >> 8) as u8;
}

// Unpacked u8 scratch for terrain gen (gen_column/maybe_tree fill it; finish_chunk
// packs it into the chunk). Only one chunk generates at a time, so one is enough.
static mut GEN_SCRATCH: [u8; CHUNK_VOL] = [AIR; CHUNK_VOL];
// Per-column height/biome cache for the in-progress chunk: gen_column fills it,
// finish_chunk's tree/decoration pass reuses it instead of recomputing height_at
// (3x vnoise) and biome_at a second and third time per column. Only one chunk
// generates at a time, so a single 256-entry cache is enough.
static mut GEN_H: [i16; CHUNK_AREA] = [0; CHUNK_AREA];
static mut GEN_BM: [u8; CHUNK_AREA] = [0; CHUNK_AREA];

// Unpacked u8 copy of the chunk currently being MESHED. Decoded once when a mesh
// starts, so the ~96K per-chunk block reads in greedy_plane hit a flat u8 array
// instead of paying the 5-bit unpack each time. Distinct from GEN_SCRATCH so a
// gen and a mesh (different chunks) can run the same frame.
static mut MESH_SCRATCH: [u8; CHUNK_VOL] = [AIR; CHUNK_VOL];
// The decoded block cache remains valid after a mesh commits. Block edits patch
// it in place, so repeated mining/placement in one chunk does not pay another
// 16K packed-block decode on every hit.
static mut MESH_SCRATCH_OWNER: usize = usize::MAX;

/// Unpack chunk `s`'s 5-bit store into MESH_SCRATCH for fast meshing.
fn decode_to_mesh_scratch(s: usize) {
    unsafe {
        if MESH_SCRATCH_OWNER == s {
            return;
        }
        let src = &CHUNKS[s].blocks;
        let mut top = 0usize;
        col_masks_reset();
        decode_blocks::<false>(src, 0, CHUNK_VOL, &mut top);
        // Highest ly holding anything but air. Nothing above it can emit a face
        // (a face needs a MESHABLE cell, and every cell up there is air), so the
        // mesher clips its y range to this -- see dir_dims.
        MESH_YHI = top / (CWU * CWU);
        MESH_SCRATCH_OWNER = s;
    }
}

// Highest non-air ly in MESH_SCRATCH, set by decode_to_mesh_scratch.
static mut MESH_YHI: usize = CHU as usize - 1;

const SEA: i32 = 28; // sea level (block y)
const CAVE_TOP: i32 = 40; // no cave sampling above this: cost scales with the band
const LAVA_Y: i32 = 9; // cave air below this fills with lava
const SEED: i32 = 0x5D2C;
// Extra seed entropy for "NEW WORLD" (title-screen timing); 0 = the classic
// default world, so headless captures stay deterministic.
static mut SEED_XTRA: i32 = 0;

#[inline]
fn seedx() -> i32 {
    SEED ^ unsafe { SEED_XTRA }
}

/// Reset every chunk/pool/stream static so `init` can regenerate a fresh world
/// with `extra` mixed into the seed (the "NEW WORLD" path).
pub fn prepare_new_world(extra: i32) {
    fluid_reset();
    unsafe {
        SEED_XTRA = extra;
        let mut s = 0;
        while s < NCHUNKS {
            CHUNKS[s].loaded = false;
            CHUNKS[s].dirty = false;
            CHUNKS[s].face_slot = NO_SLOT;
            s += 1;
        }
        let mut p = 0;
        while p < POOL {
            POOL_OWNER[p] = usize::MAX;
            POOL_NFACE[p] = 0;
            POOL_NPLANT[p] = 0;
            p += 1;
        }
        GEN_S = usize::MAX;
        MESH_S = usize::MAX;
        EDIT_MESH_S = usize::MAX;
        MESH_SCRATCH_OWNER = usize::MAX;
    }
}

struct Chunk {
    cx: i32,
    cz: i32,
    loaded: bool,
    dirty: bool,       // mesh out of date (block edit or a neighbour appeared)
    face_slot: u16,    // index into FACE_POOL, or NO_SLOT if this chunk has no mesh
    face_lod: u8,      // LOD level the current mesh was built at (0 near, 1 far)
    blocks: [u8; PACKED_SIZE], // 5-bit-packed block ids; access via bget/bset
}

const EMPTY_CHUNK: Chunk = Chunk {
    cx: 0,
    cz: 0,
    loaded: false,
    dirty: false,
    // 0, NOT the real NO_SLOT default: an all-zero initializer keeps the
    // 350 KiB CHUNKS array in .bss instead of shipping it as baked bytes in
    // the EXE. boot_prepare stamps NO_SLOT before anything reads it.
    face_slot: 0,
    face_lod: 0,
    blocks: [0; PACKED_SIZE], // all-zero packs to all-AIR (AIR == 0)
};

static mut CHUNKS: [Chunk; NCHUNKS] = [EMPTY_CHUNK; NCHUNKS];

// --- face pool ---
//
// The cached face mesh (8.8KB/chunk) lives here, NOT inline in every Chunk, so
// face RAM is bounded by how many chunks can be ON SCREEN at once, not by GRID^2.
// Only chunks within render range carry a slot; a chunk that leaves range frees
// its slot (and re-meshes if it comes back). This is what lets GRID grow large.
const POOL: usize = 28; // RENDER_R=2 means up to 5x5 chunks meshed at once
const NO_SLOT: u16 = u16::MAX;
static mut POOL_FACES: [[u32; MAX_FACES]; POOL] = [[0; MAX_FACES]; POOL];
/// Per-face vertex ambient occlusion, ONE BYTE beside each packed face word:
/// two bits per corner in emit_face's v0..v3 order, 3 = fully lit, 0 = darkest.
/// It rides alongside rather than inside the face word because that word is
/// exactly full (4+6+4+3+7+4+3+1 = 32) and the only bits left to steal are the
/// greedy merge run caps, which measured 50% of the frame (see `pack`).
/// 28 x 3000 = 84KB, which a 2MB machine can spare.
/// Per-face side-band, one entry per face, parallel to POOL_FACES.
///   bits 0-7   ambient-occlusion, 2 bits per corner
///   bits 8-10  skylight level, 0..LIGHT_BUCKETS-1
/// Light lives here rather than in the packed face word because that word is
/// exactly full -- it had light in bit 31, so any value above 1 shifted off the
/// end and corrupted the face. This array is already read for every surviving
/// face to get AO, so carrying light in the spare byte is a 16-bit load instead
/// of an 8-bit one and costs no extra cache miss.
// Zero-initialized (.bss) rather than the AO_LIT fill it semantically wants:
// the fill shipped 164 KiB of repeated bytes in the EXE. boot_prepare stamps
// the real default.
static mut POOL_AO: [[u16; MAX_FACES]; POOL] = [[0; MAX_FACES]; POOL];
static mut POOL_DIR_START: [[u16; 7]; POOL] = [[0; 7]; POOL];
static mut POOL_NFACE: [u16; POOL] = [0; POOL];
// Optional per-plane rectangle bounds. The face start/end range remains the
// authority for emptiness; a zero bound falls back to per-face culling.
static mut POOL_PLANE_BOUNDS: [[u32; PL_TOTAL]; POOL] = [[0; PL_TOTAL]; POOL];
// Cross-sprite plant cells recorded per meshed chunk (see is_cross_plant). Each
// entry packs local coords + block: lx | ly<<4 | lz<<10 | blk<<14. Scanned once
// per mesh in commit_mesh (plants are sparse), iterated by for_plants.
// ponytail: 96/chunk ceiling. A player tiling a whole 16x16 chunk with crops
// hits it and the overflow just isn't drawn -- a farm that dense is an edge
// case; bump MAX_PLANTS if it bites.
const MAX_PLANTS: usize = 96;
static mut POOL_PLANTS: [[u32; MAX_PLANTS]; POOL] = [[0; MAX_PLANTS]; POOL];
static mut POOL_NPLANT: [u16; POOL] = [0; POOL];
static mut MESH_PLANTS: [u32; MAX_PLANTS] = [0; MAX_PLANTS];
static mut MESH_NPLANT: usize = 0;

// Per-plane face ranges within each dir's span. greedy_plane meshes planes in
// ascending order, so a dir's faces are already plane-sorted; recording where
// each plane starts lets the renderer iterate ONLY the camera-facing plane
// sub-range (the per-face backface test depends only on the plane coordinate,
// so a range cut is exact) instead of testing every face. Layout per slot:
// dir0 planes at [0..=16], dir1 [17..=33], dir2 [34..=98], dir3 [99..=163],
// dir4 [164..=180], dir5 [181..=197] (each dir has plane_count+1 boundaries).
const PL_OFF: [usize; 6] = [0, 17, 34, 99, 164, 181];
const PL_N: [usize; 6] = [16, 16, 64, 64, 16, 16];
const PL_TOTAL: usize = 198;
static mut POOL_PLANE_START: [[u16; PL_TOTAL]; POOL] = [[0; PL_TOTAL]; POOL];
static mut POOL_OWNER: [usize; POOL] = [usize::MAX; POOL]; // chunk slot owning each, or MAX

// Player chunk, refreshed by recenter() each frame; stream_tick uses it to mesh
// only chunks within render range.
static mut PLAYER_CX: i32 = 0;
static mut PLAYER_CZ: i32 = 0;
// Chunk-radius that gets meshed (covers the draw distance in any facing). POOL
// must be >= (2*RENDER_R+1)^2.
const RENDER_R: i32 = 1; // mesh ring: 3x3 around the player. At FAR_Z 768
// (12 blocks) a chunk entering this ring is 16+ blocks out -- past the far
// plane, so it meshes before it can be seen. The old 5x5 ring meshed 25
// chunks per boundary cross; that streaming load, not face count, was what
// kept walking frames off the 30fps quantum below ~17-block draws.

/// Find a free pool slot for chunk `s`, or reuse its current one. Evicts a slot
/// owned by an out-of-render-range chunk if the pool is full. Returns NO_SLOT if
/// every slot is held by an in-range chunk (POOL too small -- shouldn't happen).
fn alloc_slot(s: usize) -> u16 {
    unsafe {
        if CHUNKS[s].face_slot != NO_SLOT {
            return CHUNKS[s].face_slot;
        }
        let mut p = 0;
        while p < POOL {
            if POOL_OWNER[p] == usize::MAX {
                POOL_OWNER[p] = s;
                CHUNKS[s].face_slot = p as u16;
                return p as u16;
            }
            p += 1;
        }
        // Full: evict a slot whose owner is now out of render range.
        let mut p = 0;
        while p < POOL {
            let o = POOL_OWNER[p];
            if o != usize::MAX && !in_render_range(o) {
                CHUNKS[o].face_slot = NO_SLOT;
                CHUNKS[o].dirty = true; // re-mesh if it returns to view
                POOL_OWNER[p] = s;
                CHUNKS[s].face_slot = p as u16;
                return p as u16;
            }
            p += 1;
        }
        NO_SLOT
    }
}

/// Reserve a pool slot without publishing it through CHUNKS[s].face_slot.
/// Streaming copies into this hidden slot over several frames, then swaps it in
/// atomically so rendering never observes a partial mesh.
fn reserve_stream_slot(s: usize) -> u16 {
    unsafe {
        let old = CHUNKS[s].face_slot;
        let mut p = 0usize;
        while p < POOL {
            if POOL_OWNER[p] == usize::MAX {
                POOL_OWNER[p] = s;
                return p as u16;
            }
            p += 1;
        }
        p = 0;
        while p < POOL {
            if p as u16 != old {
                let o = POOL_OWNER[p];
                if o != usize::MAX && !in_render_range(o) {
                    CHUNKS[o].face_slot = NO_SLOT;
                    CHUNKS[o].dirty = true;
                    POOL_OWNER[p] = s;
                    return p as u16;
                }
            }
            p += 1;
        }
        NO_SLOT
    }
}

/// Is loaded chunk slot `s` within RENDER_R chunks of the player (in any facing)?
fn in_render_range(s: usize) -> bool {
    unsafe {
        if !CHUNKS[s].loaded {
            return false;
        }
        (CHUNKS[s].cx - PLAYER_CX).abs() <= RENDER_R && (CHUNKS[s].cz - PLAYER_CZ).abs() <= RENDER_R
    }
}

/// LOD level for chunk `s`: 0 (near, fine mesh) or 1 (far, coarse mesh).
fn chunk_lod(s: usize) -> u8 {
    unsafe {
        let d = (CHUNKS[s].cx - PLAYER_CX)
            .abs()
            .max((CHUNKS[s].cz - PLAYER_CZ).abs());
        if d > LOD_R {
            1
        } else {
            0
        }
    }
}

/// Set the merge cap + record the LOD for the chunk about to be meshed.
fn set_mesh_lod(s: usize) {
    let lod = chunk_lod(s);
    unsafe {
        MESH_CAP = if lod == 1 { MAX_MERGE_FAR } else { MAX_MERGE };
        CHUNKS[s].face_lod = lod;
    }
}

// Scratch face mask for 2D greedy meshing (largest plane is CHU*CWU = 1024).
// Face mask cells are u16, not u8: the low 7 bits are the block and the top
// bits are a SKYLIGHT bucket, so the greedy merge's single equality test also
// refuses to merge across a light step. That is the whole cost of block light
// in the mesher -- no second array, no second compare.
static mut FMASK: [u16; CHU as usize * CWU] = [0; CHU as usize * CWU];
/// Rows of FMASK (the plane's b axis) that hold at least one face cell after
/// `build_mask`; the greedy seed scan skips the others. All ones on the
/// dense border path.
static mut FMASK_ROWS: u64 = 0;
/// Per row of FMASK, one bit per cell that holds a face (bit a of row b).
/// The greedy seed scan jumps between set bits instead of reading every
/// cell of a non-empty row; the merge clears the bits of the cells it
/// consumes as it zeroes them.
static mut FMASK_BITS: [u64; CHU as usize] = [0; CHU as usize];
/// OR over all columns of the +Y / -Y candidate masks: bit ly set when some
/// column has a face cell in y-plane ly for that direction. Rebuilt whenever
/// the column masks change (decode or edit), tracked by the epoch below.
static mut Y_ANY: [u64; 2] = [0; 2];
static mut Y_ANY_EPOCH: u32 = u32::MAX;
static mut COL_MASKS_EPOCH: u32 = 0;
/// Per-column occupancy of the chunk in MESH_SCRATCH, one bit per ly: which
/// cells are meshable (CLS_MESH) and which can be seen through (CLS_SEE).
/// Built alongside every scratch decode and kept in step by the edit
/// writers, so `build_mask` can find a plane's face cells with a few 64-bit
/// ANDs per column instead of visiting every cell. Index is lx + lz * CWU.
static mut COL_MESH: [u64; CWU * CWU] = [0; CWU * CWU];
static mut COL_SEE: [u64; CWU * CWU] = [0; CWU * CWU];

#[inline(always)]
fn col_masks_reset() {
    unsafe {
        COL_MESH = [0; CWU * CWU];
        COL_SEE = [0; CWU * CWU];
        COL_MASKS_EPOCH = COL_MASKS_EPOCH.wrapping_add(1);
    }
}

/// Note block `b` at scratch index `i` during a decode (bits only ever set).
/// Decode blocks `i0..i1` (multiples of 256, i.e. whole layers) of a packed
/// store into MESH_SCRATCH, noting the column masks. `top` tracks the highest
/// non-air index. With FULL the stream mesher's extras are recorded as well:
/// per-column sky top, light sources and plants.
#[inline(never)]
fn decode_blocks<const FULL: bool>(src: &[u8; PACKED_SIZE], i0: usize, i1: usize, top: &mut usize) {
    let mut i = i0;
    while i < i1 {
        // One layer: the y bit of the column masks is fixed for 256 blocks.
        let ly = i >> 8;
        let bit = 1u64 << ly;
        let layer_end = i + CWU * CWU;
        while i < layer_end {
            let blk = bget8(src, i >> 3);
            let mut k = 0;
            while k < 8 {
                let b = blk[k];
                let idx = i + k;
                let cls = unsafe { BCLASS[b as usize] };
                unsafe { MESH_SCRATCH[idx] = b };
                if b != AIR {
                    *top = idx;
                    // Air is see-through only; everything else touches a mask.
                    let col = idx & (CWU * CWU - 1);
                    unsafe {
                        if cls & CLS_MESH != 0 {
                            COL_MESH[col] |= bit;
                            if FULL {
                                SKY_TOP[col] = ly as u8;
                            }
                        }
                        if cls & CLS_SEE != 0 {
                            COL_SEE[col] |= bit;
                        }
                        if FULL && cls & CLS_SPECIAL != 0 {
                            let lx = col & (CWU - 1);
                            let lz = col >> 4;
                            if MESH_NLIGHT < MAX_SOURCES && is_light_source(b) {
                                MESH_LIGHT_SOURCES[MESH_NLIGHT] = (lx, ly, lz);
                                MESH_NLIGHT += 1;
                            }
                            if MESH_NPLANT < MAX_PLANTS && (is_cross_plant(b) || is_small_block(b)) {
                                MESH_PLANTS[MESH_NPLANT] = lx as u32
                                    | ((ly as u32) << 4)
                                    | ((lz as u32) << 10)
                                    | ((b as u32) << 14);
                                MESH_NPLANT += 1;
                            }
                        }
                    }
                } else {
                    let col = idx & (CWU * CWU - 1);
                    unsafe { COL_SEE[col] |= bit };
                }
                k += 1;
            }
            i += 8;
        }
    }
}

#[inline(always)]
fn col_masks_note(i: usize, b: u8) {
    let cls = unsafe { BCLASS[b as usize] };
    if cls & (CLS_MESH | CLS_SEE) == 0 {
        return;
    }
    let col = i & (CWU * CWU - 1);
    let bit = 1u64 << (i >> 8);
    unsafe {
        if cls & CLS_MESH != 0 {
            COL_MESH[col] |= bit;
        }
        if cls & CLS_SEE != 0 {
            COL_SEE[col] |= bit;
        }
    }
}

/// Rewrite the bits of scratch index `i` after an edit stored block `b`.
#[inline(always)]
fn col_masks_set(i: usize, b: u8) {
    let cls = unsafe { BCLASS[b as usize] };
    let col = i & (CWU * CWU - 1);
    let bit = 1u64 << (i >> 8);
    unsafe {
        COL_MASKS_EPOCH = COL_MASKS_EPOCH.wrapping_add(1);
        if cls & CLS_MESH != 0 {
            COL_MESH[col] |= bit;
        } else {
            COL_MESH[col] &= !bit;
        }
        if cls & CLS_SEE != 0 {
            COL_SEE[col] |= bit;
        } else {
            COL_SEE[col] &= !bit;
        }
    }
}
/// Height of the first sky-blocking block in each column of the chunk being
/// meshed, rebuilt per mesh. Skylight is derived from depth below it.
static mut SKY_TOP: [u8; CWU * CWU] = [0; CWU * CWU];
/// Two buckets, against Java's sixteen. This is the largest remaining visual
/// gap after the overworld's colour, and it is MEASURED rather than assumed --
/// I previously claimed more levels would cost too much in merge breaks, which
/// was reasoning, not a number, and the number turned out smaller than I said.
///
/// Two experiments, on the profile scene:
///   1. Raising this constant alone changes NOTHING (1,644,351 vs 1,644,332
///      cycles). sky_bucket only ever answers 0 or LIGHT_BUCKETS-1, so more
///      buckets create no intermediate values and therefore no new merge breaks.
///   2. Grading sky_bucket by depth -- one step per two blocks of cover, which is
///      what actually produces intermediate levels -- costs:
///         2 buckets   loop body 1,628,117   face loop 1,431,300
///         4 buckets   loop body 1,716,662   face loop 1,504,800   (+88,545, +5.4%)
///         8 buckets   loop body 1,783,148   face loop 1,570,999   (+155,031, +9.5%)
///
/// So 4-level smooth lighting costs about 5%. What blocks it is STORAGE, not
/// cost: `pack` puts light in bit 31, so any value above 1 shifts off the word
/// and corrupts it. The fix has a precedent already in the tree -- POOL_AO is a
/// parallel per-face byte array that is already read for every surviving face,
/// so widening it to u16 and carrying the light level in the spare byte costs a
/// 16-bit load instead of an 8-bit one and no extra cache miss. Then SKY_LEVELS
/// and SKY_SCALE in main.rs grow to match (the table is rebuilt once a frame, so
/// per-face cost stays one index).
/// Levels of skylight. FOUR, live since 2026-07-30: the graded sky_bucket
/// below, the u16 side-band that carries the value, and SKY_SCALE in main.rs
/// together. It shipped at TWO for a while because the ~90K (+5.4%) work cost
/// tipped the then-tree over a vsync quantum; the current tree absorbs it
/// (see the commit for the re-measurement). Change this constant with
/// SKY_LEVELS/SKY_SCALE together.
const LIGHT_BUCKETS: u16 = 4;
/// Blocks of cover before a cell counts as shadowed. Two, not more: a house
/// interior and a cave ceiling should both read as indoors, and at two buckets
/// the cutoff has to sit where cover starts rather than where it gets deep.
const LIGHT_DEPTH: i32 = 2;

/// One bit per cell of the chunk being meshed: lit by a TORCH, lava, lumistone
/// or fire. Rebuilt per mesh alongside SKY_TOP.
///
/// This is what makes point lights affordable without the 200KB per-block light
/// array a real propagation needs. Sources are found in one pass (the same pass
/// that builds SKY_TOP), then each floods a bounded sphere -- a few thousand bit
/// writes per mesh -- and the mask build pays ONE bitmap lookup per cell instead
/// of a distance loop over sources.
static mut TORCH_LIT: [u32; CHUNK_VOL / 32] = [0; CHUNK_VOL / 32];
/// How far a light source reaches. Java's torch is 14; at two buckets this only
/// has to decide lit-or-not, and 6 keeps the flood cheap.
const TORCH_R: i32 = 6;
/// Sources considered per chunk. A room lit by twenty torches looks the same as
/// one lit by six, and the flood is the cost.
const MAX_SOURCES: usize = 6;

#[inline]
fn torch_lit(lx: usize, ly: usize, lz: usize) -> bool {
    let i = lidx(lx, ly, lz);
    unsafe { TORCH_LIT[i >> 5] & (1 << (i & 31)) != 0 }
}

#[inline]
fn is_light_source(b: u8) -> bool {
    b == TORCH || b == LUMISTONE || b == FIRE || is_lava(b)
}

/// Light bucket for a chunk-local cell: lit by sky OR by a nearby source.
#[inline]
fn sky_bucket(lx: usize, ly: usize, lz: usize) -> u16 {
    let top = unsafe { SKY_TOP[lz * CWU + lx] } as i32;
    let depth = top - ly as i32;
    if depth < LIGHT_DEPTH {
        return LIGHT_BUCKETS - 1;
    }
    if torch_lit(lx, ly, lz) {
        return LIGHT_BUCKETS - 1;
    }
    // GRADED falloff under cover, one step per two blocks of depth, instead of a
    // cliff straight to black. This is what actually produces intermediate
    // levels: raising LIGHT_BUCKETS alone changed nothing measurable, because
    // this function only ever answered 0 or max.
    let steps = (LIGHT_BUCKETS - 1) as i32;
    let d = (depth - LIGHT_DEPTH) / 2;
    (steps - d.min(steps)).max(0) as u16
}

/// Rebuild SKY_TOP and TORCH_LIT from MESH_SCRATCH. One pass per column finds
/// both the highest sky-blocking block and any light sources below it; the
/// sources then flood.
fn build_sky_top() {
    unsafe { TORCH_LIT = [0; CHUNK_VOL / 32] };
    let mut src: [(usize, usize, usize); MAX_SOURCES] = [(0, 0, 0); MAX_SOURCES];
    let mut nsrc = 0usize;
    let mut lz = 0usize;
    while lz < CWU {
        let mut lx = 0usize;
        while lx < CWU {
            let mut ly = CHU - 1;
            let mut top = 0u8;
            while ly > 0 {
                let b = unsafe { MESH_SCRATCH[lidx(lx, ly, lz)] };
                if top == 0 && unsafe { BCLASS[b as usize] } & CLS_MESH != 0 {
                    top = ly as u8;
                }
                if nsrc < MAX_SOURCES && is_light_source(b) {
                    src[nsrc] = (lx, ly, lz);
                    nsrc += 1;
                }
                ly -= 1;
            }
            unsafe { SKY_TOP[lz * CWU + lx] = top };
            lx += 1;
        }
        lz += 1;
    }
    let mut k = 0;
    while k < nsrc {
        flood_light(src[k]);
        k += 1;
    }
}

/// Refresh the one skylight column touched by an ordinary block edit. Emitted
/// face lighting only consults SKY_TOP for its own column, so rescanning all 256
/// columns here was pure edit-time cost.
fn build_sky_column(lx: usize, lz: usize) {
    let mut ly = CHU - 1;
    let mut top = 0u8;
    while ly > 0 {
        let b = unsafe { MESH_SCRATCH[lidx(lx, ly, lz)] };
        if unsafe { BCLASS[b as usize] } & CLS_MESH != 0 {
            top = ly as u8;
            break;
        }
        ly -= 1;
    }
    unsafe { SKY_TOP[lz * CWU + lx] = top };
}

/// Mark every cell within TORCH_R of a source as lit. A cube test, not a sphere:
/// the corners are wrong by a block and nobody can tell at two brightness levels.
fn flood_light(p: (usize, usize, usize)) {
    let (sx, sy, sz) = (p.0 as i32, p.1 as i32, p.2 as i32);
    let y0 = (sy - TORCH_R).max(0);
    let y1 = (sy + TORCH_R).min(CHU as i32 - 1);
    let z0 = (sz - TORCH_R).max(0);
    let z1 = (sz + TORCH_R).min(CWU as i32 - 1);
    let x0 = (sx - TORCH_R).max(0);
    let x1 = (sx + TORCH_R).min(CWU as i32 - 1);
    let mut y = y0;
    while y <= y1 {
        let mut z = z0;
        while z <= z1 {
            let mut x = x0;
            while x <= x1 {
                let i = lidx(x as usize, y as usize, z as usize);
                unsafe { TORCH_LIT[i >> 5] |= 1 << (i & 31) };
                x += 1;
            }
            z += 1;
        }
        y += 1;
    }
}

// Meshing scratch: a chunk is meshed into MESH_FACES/MESH_DIR_START, then copied
// into the chunk in one shot. The amortized streaming path (stream_tick) fills it
// over a cell budget so no gameplay frame does a whole chunk's mesh; the
// synchronous mesh_chunk is reserved for boot/load transitions. Only one mesh
// is ever in flight, so a single shared scratch is safe.
static mut MESH_FACES: [u32; MAX_FACES] = [0; MAX_FACES];
static mut MESH_AO: [u16; MAX_FACES] = [0; MAX_FACES]; // stamped in boot_prepare
static mut MESH_DIR_START: [u16; 7] = [0; 7];
static mut MESH_PLANE_START: [u16; PL_TOTAL] = [0; PL_TOTAL];
static mut MESH_PLANE_BOUNDS: [u32; PL_TOTAL] = [0; PL_TOTAL];
static mut MESH_S: usize = usize::MAX; // chunk slot being amortized-meshed (MAX = none)
static mut MESH_DIR: usize = 0; // face-direction currently being meshed for MESH_S
static mut MESH_PLANE: usize = 0; // next plane index within MESH_DIR
static mut MESH_N: usize = 0; // faces written into the scratch so far
static mut MESH_PREP: u8 = 0; // 0 decode, 1 lighting, 2 plane meshing, 3 commit
static mut MESH_DECODE: usize = 0;
static mut MESH_DECODE_TOP: usize = 0;
static mut MESH_COMMIT_SLOT: u16 = NO_SLOT;
static mut MESH_COMMIT_I: usize = 0;
static mut MESH_LIGHT_SOURCES: [(usize, usize, usize); MAX_SOURCES] = [(0, 0, 0); MAX_SOURCES];
static mut MESH_NLIGHT: usize = 0;
static mut MESH_LIGHT_I: usize = 0;
const STREAM_DECODE_BATCH: usize = 4096;
const STREAM_COMMIT_BATCH: usize = 512;

fn abort_stream_mesh() {
    unsafe {
        if MESH_S != usize::MAX {
            CHUNKS[MESH_S].dirty = true;
        }
        if MESH_COMMIT_SLOT != NO_SLOT {
            POOL_OWNER[MESH_COMMIT_SLOT as usize] = usize::MAX;
            MESH_COMMIT_SLOT = NO_SLOT;
        }
        MESH_S = usize::MAX;
    }
}

// A player edit is rebuilt atomically into the same mesh scratch, but one
// direction per rendered frame. The committed pool remains visible until the
// replacement is complete, avoiding a CPU hitch without exposing half a mesh.
static mut EDIT_MESH_S: usize = usize::MAX;
static mut EDIT_MESH_LX: usize = 0;
static mut EDIT_MESH_LY: usize = 0;
static mut EDIT_MESH_LZ: usize = 0;
static mut EDIT_MESH_PHASE: u8 = 0; // 0 decode, 1 lighting/setup, 2 dirs, 3 commit
static mut EDIT_MESH_DECODE: usize = 0;
static mut EDIT_MESH_TOP: usize = 0;
static mut EDIT_MESH_DIR: usize = 0;
static mut EDIT_MESH_N: usize = 0;
static mut EDIT_MESH_POOL: usize = 0;
static mut EDIT_LIGHT_CHANGED: bool = false;
static mut EDIT_PLANTS_CHANGED: bool = false;
// The chunk already owed a full rebuild when this edit queued; the partial
// commit must re-set dirty instead of clearing it (see queue_mesh_edit).
static mut EDIT_PENDING_FULL: bool = false;
// The edit moved the column's skylight top, so light changed in cells far
// outside the ±1-plane rebuild window; a full remesh follows the partial.
static mut EDIT_SKY_CHANGED: bool = false;
const EDIT_DECODE_BATCH: usize = 4096;
// Plane-cells meshed per stream slice. Sized against movement speed, not just
// hitch avoidance: a full chunk mesh scans ~90K plane cells (~75 slices at the
// old 1280), a walk crosses a chunk boundary every ~76 frames and needs five
// fresh meshes inside it, so the budget must sustain ~6K cells/frame while the
// heavy-frame tier caps slices at 4 -- 1280 could not (5.1K), and the frontier
// slowly outran the mesher. 2048 puts the heavy tier at 8.2K cells/frame,
// covering a sprint (~7.9K) with the slice count unchanged. Decode, lighting
// and atomic pool publication are separately staged and batch-bounded, so the
// worst frame still cannot swallow a whole-chunk lump.
const MESH_CELL_BUDGET: usize = 2048;

// Amortized generation: a streamed-in chunk is filled GEN_BATCH columns per
// streaming tick (kept loaded=false -> reads as air, unrendered -- until
// complete) so a boundary cross never does a whole chunk's gen in one frame.
const CHUNK_AREA: usize = CWU * CWU; // 256 columns
// Columns per streaming TICK. A chunk is 256 columns and main runs at most
// STREAM_TICKS_MAX ticks a frame, so this sets how many FRAMES a chunk takes.
// Crossing a chunk boundary needs five new chunks inside the ~57 frames it takes
// to walk 16 blocks; much slower and generation cannot keep up in a straight
// line, which is exactly what the too-fine publish split below caused.
const GEN_BATCH: usize = 8;
// The decorate + pack phases only ever totalled ~2 vblanks. Split them just
// finely enough that no single frame swallows the lump (2 and 4 ticks), NOT as
// finely as possible -- at 16 ticks each they doubled chunk latency.
const DECORATE_BATCH: usize = 128;
const PACK_BATCH: usize = 4096;
static mut GEN_S: usize = usize::MAX; // slot being incrementally generated (MAX = none)
static mut GEN_CX: i32 = 0;
static mut GEN_CZ: i32 = 0;
static mut GEN_COL: usize = 0; // progress cursor within the current GEN_PHASE
static mut GEN_PHASE: u8 = 0; // 0 = columns, 1 = decorate, 2 = pack

// Per-face camera-space view-frustum cull before the costly projection in
// emit(). Measured worth 1.78x render throughput (A/B at the shipped config:
// 15fps with it off, 30fps on).
const FRUSTUM_CULL: bool = true;

#[inline]
fn lidx(lx: usize, ly: usize, lz: usize) -> usize {
    (ly * CWU + lz) * CWU + lx
}

#[inline]
fn slot(cx: i32, cz: i32) -> usize {
    let sx = cx.rem_euclid(GRID) as usize;
    let sz = cz.rem_euclid(GRID) as usize;
    sz * GRIDU + sx
}

// --- fixed-point value noise ---

#[inline]
fn hash2(x: i32, z: i32, seed: i32) -> i32 {
    let mut h = (x
        .wrapping_mul(73856093)
        .wrapping_add(z.wrapping_mul(19349663))
        .wrapping_add(seed.wrapping_mul(83492791))) as u32;
    h ^= h >> 13;
    h = h.wrapping_mul(1274126177);
    h ^= h >> 16;
    (h & 0xFF) as i32
}

/// Smoothstep on a 0..256 (q8) input, returns 0..256.
#[inline]
fn smooth(t: i32) -> i32 {
    (t * t * (768 - 2 * t)) / 65536
}

/// Bilinear value noise sampled on a `scale`-block lattice. Returns 0..255.
// `S` (the lattice spacing) is a const generic so every division by it -- in
// floor_div and the *256/S below -- folds to a multiply+shift at each call site.
// The R3000 has no hardware divide, and gen does thousands of these per chunk.
fn vnoise<const S: i32>(x: i32, z: i32, seed: i32) -> i32 {
    let gx = floor_div(x, S);
    let gz = floor_div(z, S);
    let fx = smooth((x - gx * S) * 256 / S);
    let fz = smooth((z - gz * S) * 256 / S);
    let h00 = hash2(gx, gz, seed);
    let h10 = hash2(gx + 1, gz, seed);
    let h01 = hash2(gx, gz + 1, seed);
    let h11 = hash2(gx + 1, gz + 1, seed);
    let a = h00 + (h10 - h00) * fx / 256;
    let b = h01 + (h11 - h01) * fx / 256;
    a + (b - a) * fz / 256
}

/// Nine lattice-corner hashes covering one chunk for spacing `S`.
///
/// Every shape noise uses S >= 44 and a chunk is 16 blocks wide, so a chunk
/// spans at most two lattice cells per axis: nine corners always suffice. The
/// old code re-hashed all four corners for EVERY column, i.e. 4 hashes x 256
/// columns x 5 noises per chunk, when 9 per noise is the whole truth.
#[derive(Clone, Copy)]
struct NoiseTile {
    gx0: i32,
    gz0: i32,
    h: [i32; 9],
}

fn tile<const S: i32>(ox: i32, oz: i32, seed: i32) -> NoiseTile {
    let gx0 = floor_div(ox, S);
    let gz0 = floor_div(oz, S);
    let mut h = [0i32; 9];
    let mut j = 0;
    while j < 3 {
        let mut i = 0;
        while i < 3 {
            h[j * 3 + i] = hash2(gx0 + i as i32, gz0 + j as i32, seed);
            i += 1;
        }
        j += 1;
    }
    NoiseTile { gx0, gz0, h }
}

/// Same bilinear value noise as `vnoise`, but reading corners from a prebuilt
/// tile instead of hashing them. Identical output.
#[inline]
fn tsample<const S: i32>(t: &NoiseTile, x: i32, z: i32) -> i32 {
    let gx = floor_div(x, S);
    let gz = floor_div(z, S);
    let ix = (gx - t.gx0) as usize;
    let iz = (gz - t.gz0) as usize;
    let fx = smooth((x - gx * S) * 256 / S);
    let fz = smooth((z - gz * S) * 256 / S);
    let r0 = iz * 3 + ix;
    let r1 = r0 + 3;
    let h00 = t.h[r0];
    let h10 = t.h[r0 + 1];
    let h01 = t.h[r1];
    let h11 = t.h[r1 + 1];
    let a = h00 + (h10 - h00) * fx / 256;
    let b = h01 + (h11 - h01) * fx / 256;
    a + (b - a) * fz / 256
}

/// The five per-column shape noises for one chunk, hashed once.
#[derive(Clone, Copy)]
struct ShapeTiles {
    cont: NoiseTile,
    ero: NoiseTile,
    hill: NoiseTile,
    mtn: NoiseTile,
    temp: NoiseTile,
}

fn shape_tiles(ox: i32, oz: i32) -> ShapeTiles {
    ShapeTiles {
        cont: tile::<192>(ox, oz, seedx()),
        ero: tile::<112>(ox, oz, seedx() + 31),
        hill: tile::<44>(ox, oz, seedx() + 11),
        mtn: tile::<170>(ox, oz, seedx() + 23),
        temp: tile::<96>(ox, oz, seedx() + 50),
    }
}

/// Pseudo-3D noise for caves: a single 2D field sheared by y (cheap; one
/// vnoise per block dominates gen cost). Returns 0..255.
fn cave_density(x: i32, y: i32, z: i32) -> i32 {
    vnoise::<13>(x + y * 7, z - y * 5 + x, seedx() + 77)
}

// --- terrain shape ---

// Biomes.
pub const B_OCEAN: u8 = 0;
pub const B_PLAINS: u8 = 1;
pub const B_DESERT: u8 = 2;
pub const B_MOUNTAIN: u8 = 3;
pub const B_SNOW: u8 = 4;

/// Continentalness -> base height, as a piecewise-linear spline.
///
/// Minecraft maps continentalness through a spline rather than scaling it
/// linearly, which is what lets mid-range noise produce ocean basins and a
/// sharp coastline instead of relying on rare noise extremes. The old code
/// here was `SEA + 8 + (cont-128)/7 + (hill-128)*5/16`, so the 44-block hill
/// term (+-40) dominated the 128-block continent term (+-18): oceans came out
/// as hill-sized ponds and the world had no flat ground anywhere.
///
/// x is 0..255 (the raw noise), y is the block height. The steep run between
/// 72 and 92 is the coastline.
const CONT_SPLINE: [(i32, i32); 7] = [
    (0, 8),
    (42, 15),
    (72, 23),
    (92, 32),
    (128, 38),
    (190, 43),
    (255, 50),
];

fn spline(x: i32, pts: &[(i32, i32)]) -> i32 {
    if x <= pts[0].0 {
        return pts[0].1;
    }
    let mut i = 0;
    while i + 1 < pts.len() {
        let (x0, y0) = pts[i];
        let (x1, y1) = pts[i + 1];
        if x <= x1 {
            return y0 + (y1 - y0) * (x - x0) / (x1 - x0);
        }
        i += 1;
    }
    pts[pts.len() - 1].1
}

/// Turn the four shape noises into a block height. Shared by the general and
/// the tiled samplers so the terrain formula lives in exactly one place.
fn shape_from(cont: i32, ero: i32, hill: i32, mtn: i32) -> i32 {
    let base = spline(cont, &CONT_SPLINE);
    // Erosion, squared so most of the world lands at the flat end: this is what
    // turns "every step is a step up or down" into real plains.
    let e = ero * ero / 255;
    let mut amp = 2 + e * 18 / 255;
    if base < SEA {
        // Sea beds stay flat: relief down there is never seen through the water
        // and only costs faces.
        amp /= 3;
    }
    let mut h = base + (hill - 128) * amp / 64;
    // Peaks only on land, rarer but taller, so mountains are an event.
    if mtn > 180 && base >= SEA {
        let m = mtn - 180;
        h += m * m / 95;
    }
    h.clamp(2, CH - 4)
}

pub fn height_at(wx: i32, wz: i32) -> i32 {
    shape_from(
        vnoise::<192>(wx, wz, seedx()),
        vnoise::<112>(wx, wz, seedx() + 31),
        vnoise::<44>(wx, wz, seedx() + 11),
        vnoise::<170>(wx, wz, seedx() + 23),
    )
}

#[inline]
fn height_at_tiled(t: &ShapeTiles, wx: i32, wz: i32) -> i32 {
    shape_from(
        tsample::<192>(&t.cont, wx, wz),
        tsample::<112>(&t.ero, wx, wz),
        tsample::<44>(&t.hill, wx, wz),
        tsample::<170>(&t.mtn, wx, wz),
    )
}

fn biome_from(temp: i32, h: i32) -> u8 {
    if h < SEA - 2 {
        return B_OCEAN;
    }
    if h > SEA + 16 {
        B_MOUNTAIN
    } else if temp < 74 {
        B_SNOW
    } else if temp > 190 {
        B_DESERT
    } else {
        B_PLAINS
    }
}

pub fn biome_at(wx: i32, wz: i32, h: i32) -> u8 {
    biome_from(vnoise::<96>(wx, wz, seedx() + 50), h)
}

/// Pick a world spawn on GRASS, the way Minecraft picks a spawn biome rather
/// than a coordinate.
///
/// The old spawn was the hardcoded block (8, 8), and for the shipping seed that
/// lands on temperature 211 -- desert starts at 191 -- with the whole
/// neighbourhood out to ~32 blocks between 180 and 211. So every screenshot ever
/// taken of this game was beige, and a visual audit called the world "a beige
/// staircase quarry". Measured over 184,041 samples of the real field, the world
/// is 65.2% plains, 19.0% snow and 15.8% desert: the fixed point simply landed
/// in one of the desert patches.
///
/// This runs BEFORE any chunk is generated, which it can because height and
/// temperature are both pure noise -- no terrain needed. It walks outward in
/// rings and takes the first cell that is plains at a sensible altitude, so it
/// follows a new seed instead of being re-tuned for one.
pub fn pick_spawn(bx: i32, bz: i32) -> (i32, i32) {
    /// Open plains at an altitude worth looking at: above the waterline, below
    /// the point where the surface turns to mountain snow and stone.
    fn open_plains(wx: i32, wz: i32) -> bool {
        let h = height_at(wx, wz);
        if !(SEA + 2..=SEA + 14).contains(&h) || biome_at(wx, wz, h) != B_PLAINS {
            return false;
        }
        // ...and OPEN, not deep forest. maybe_tree gates on this same field:
        // above 168 a column plants a tree 1 in 13, below it 1 in 48. Spawning
        // in a dense patch put the camera in wall-to-wall canopy at 15.8 fps
        // standing and 10.7 walking. Meadow at spawn, forest to walk into.
        if vnoise::<24>(wx, wz, seedx() + 200) > 150 {
            return false;
        }
        true
    }

    /// Local terrain relief over a 17-block cross. Face count tracks relief, and
    /// the old desert spawn was cheap mainly because it was FLAT (measured 3).
    /// A green spawn with more relief cost 30fps -> 20fps, and it was the relief
    /// doing that rather than the greenery.
    ///
    /// Deliberately NOT part of open_plains: that runs nine times per candidate
    /// for the neighbourhood test, and folding four extra height_at calls into
    /// it put ~180 noise evaluations on every candidate. Boot then ate the whole
    /// profiling budget -- 7 frames at 96,035,372 cycles each. Checked once, on
    /// a candidate that has already passed everything cheaper.
    fn relief(wx: i32, wz: i32) -> i32 {
        let h = height_at(wx, wz);
        let (mut lo, mut hi) = (h, h);
        let mut i = 0;
        while i < 4 {
            let (dx, dz) = match i {
                0 => (8, 0),
                1 => (-8, 0),
                2 => (0, 8),
                _ => (0, -8),
            };
            let n = height_at(wx + dx, wz + dz);
            if n < lo {
                lo = n;
            }
            if n > hi {
                hi = n;
            }
            i += 1;
        }
        hi - lo
    }
    // Sampling a candidate's NEIGHBOURHOOD, not just the cell. The first version
    // took the first plains cell it found and landed 12 blocks from the old
    // spawn, in a one-cell plains pocket inside the desert -- less than a chunk
    // away, so the view did not change at all. Eight of these nine must be
    // plains, which finds a plains REGION.
    const RING: [(i32, i32); 9] = [
        (0, 0),
        (24, 0),
        (-24, 0),
        (0, 24),
        (0, -24),
        (17, 17),
        (-17, 17),
        (17, -17),
        (-17, -17),
    ];
    let mut r: i32 = 0;
    while r <= 256 {
        let step = if r > 64 { 8 } else { 4 };
        let mut dz = -r;
        while dz <= r {
            let mut dx = -r;
            while dx <= r {
                // Ring, not disc: only cells at exactly radius r are new.
                if dx.abs().max(dz.abs()) == r {
                    let (cx, cz) = (bx + dx, bz + dz);
                    if open_plains(cx, cz) {
                        let mut n = 0;
                        let mut i = 0;
                        while i < RING.len() {
                            if open_plains(cx + RING[i].0, cz + RING[i].1) {
                                n += 1;
                            }
                            i += 1;
                        }
                        if n >= 8 && relief(cx, cz) <= 3 {
                            return (cx, cz);
                        }
                    }
                }
                dx += step;
            }
            dz += step;
        }
        r += step;
    }
    (bx, bz) // nothing found: keep the caller's guess rather than hunt forever
}

#[inline]
fn biome_at_tiled(t: &ShapeTiles, wx: i32, wz: i32, h: i32) -> u8 {
    biome_from(tsample::<96>(&t.temp, wx, wz), h)
}

/// Ore blobs, scattered per chunk the way Minecraft does it: a fixed number of
/// attempts per ore type, each seeding a short random walk through stone.
///
/// This replaces a per-block roll that ran `hash2` on EVERY stone block --
/// roughly 30 hashes per column, about a third of the whole generator's cost --
/// and produced salt-and-pepper single blocks rather than veins you can follow.
/// Now it is ~26 walks per chunk total.
///
/// (kind, tries per chunk, max y, walk length)
const ORE_VEINS: [(u8, i32, i32, i32); 4] = [
    (COAL_ORE, 10, 58, 9),
    (IRON_ORE, 8, 46, 7),
    (GOLD_ORE, 5, 26, 5),
    (DIAMOND_ORE, 3, 13, 4),
];

/// Deterministic per-chunk scatter: everything derives from (cx, cz) and the
/// seed, so a chunk that streams out and back generates identically.
fn scatter_ores(blk: &mut [u8; CHUNK_VOL], cx: i32, cz: i32) {
    let mut t = 0usize;
    while t < ORE_VEINS.len() {
        let (kind, tries, max_y, walk) = ORE_VEINS[t];
        let mut i = 0;
        while i < tries {
            // Three independent hashes give the seed cell; salt by ore and try.
            let salt = seedx() + 91 + t as i32 * 17 + i * 131;
            let a = hash2(cx, cz, salt);
            let b = hash2(cx + 31, cz - 17, salt);
            let c = hash2(cx - 13, cz + 41, salt);
            let mut lx = (a as usize) & 15;
            let mut lz = (b as usize) & 15;
            // Bias low: c/255 squared keeps most veins near the bottom of the band.
            let mut ly = (c * c / 255 * (max_y - 2) / 255 + 1) as usize;
            let mut step = 0;
            while step < walk {
                if ly < CHU && blk[lidx(lx, ly, lz)] == STONE {
                    blk[lidx(lx, ly, lz)] = kind;
                }
                // Wander one cell; the hash of the step keeps it deterministic.
                let d = hash2(step + i * 7, t as i32, salt);
                match d & 7 {
                    0 => lx = (lx + 1) & 15,
                    1 => lx = (lx + 15) & 15,
                    2 => lz = (lz + 1) & 15,
                    3 => lz = (lz + 15) & 15,
                    4 => ly = if ly + 1 < CHU { ly + 1 } else { ly },
                    5 => ly = if ly > 1 { ly - 1 } else { ly },
                    6 => lx = (lx + 1) & 15,
                    _ => lz = (lz + 1) & 15,
                }
                step += 1;
            }
            i += 1;
        }
        t += 1;
    }
}

fn gen_column(blk: &mut [u8; CHUNK_VOL], t: &ShapeTiles, lx: usize, lz: usize, wx: i32, wz: i32) {
    let h = height_at_tiled(t, wx, wz);
    let bm = biome_at_tiled(t, wx, wz, h);
    // Cache height+biome for finish_chunk's decoration pass (see GEN_H/GEN_BM).
    unsafe {
        GEN_H[lz * CWU + lx] = h as i16;
        GEN_BM[lz * CWU + lx] = bm;
    }

    // Fill the column as RUNS rather than deciding per block. A column is only
    // ever five bands -- bedrock, stone, subsoil, surface, then water or air --
    // and the old loop re-evaluated that whole if/else chain 64 times per column
    // at ~43 cycles a block, nearly all of it branching. `lidx` steps by
    // CWU*CWU per y, so each run is a strided store loop with no decisions in it.
    let sub = if bm == B_DESERT || bm == B_OCEAN {
        SAND
    } else {
        DIRT
    };
    let top = match bm {
        B_DESERT | B_OCEAN => SAND,
        B_SNOW => SNOW,
        B_MOUNTAIN if h > SEA + 22 => SNOW,
        _ => GRASS,
    };
    let base = lidx(lx, 0, lz);
    const STRIDE: usize = CWU * CWU;
    let mut put_run = |y0: i32, y1: i32, b: u8| {
        // half-open [y0, y1), clamped to the column
        let lo = if y0 < 0 { 0 } else { y0 };
        let hi = if y1 > CH { CH } else { y1 };
        let mut y = lo;
        let mut i = base + lo as usize * STRIDE;
        while y < hi {
            // i < base + CH*STRIDE = CHUNK_VOL for every clamped y.
            unsafe { *blk.get_unchecked_mut(i) = b };
            i += STRIDE;
            y += 1;
        }
    };
    put_run(0, 1, STONE); // bedrock (unbreakable: main guards by == 0)
    put_run(1, h - 4, STONE); // ore blobs are scattered later by scatter_ores
    put_run(if h - 4 < 1 { 1 } else { h - 4 }, h, sub);
    put_run(h, h + 1, top);
    put_run(h + 1, SEA + 1, WATER); // oceans and lakes
    // The air above is already there: gen_columns clears the scratch to AIR
    // once per chunk (one memset beats 256 strided store loops).

    // Carve caves through the stone band. Deep cave air becomes lava. Caves are
    // 42% of the generator, so the band is sampled every 4 blocks and stops at
    // CAVE_TOP: surface-level caves cost the most samples on tall terrain and
    // are the least interesting, real systems live well below. 4-block-tall
    // caves are more walkable than the old 2 anyway.
    {
        let hi = if h - 3 < CAVE_TOP + 1 {
            h - 3
        } else {
            CAVE_TOP + 1
        };
        let mut y = 3;
        let mut cave_air = false;
        let mut i = base + 3 * STRIDE;
        while y < hi {
            if y & 3 == 0 {
                let d = cave_density(wx, y, wz);
                cave_air = d > 196 && d < 208;
            }
            if cave_air {
                unsafe { *blk.get_unchecked_mut(i) = if y < LAVA_Y { LAVA } else { AIR } };
            }
            i += STRIDE;
            y += 1;
        }
    }
}

fn maybe_tree(blk: &mut [u8; CHUNK_VOL], lx: usize, lz: usize, wx: i32, wz: i32, h: i32, bm: u8) {
    // Keep the 5x5 canopy inside the chunk to avoid cross-chunk writes.
    if lx < 2 || lx > 13 || lz < 2 || lz > 13 {
        return;
    }
    // h + bm arrive cached from gen_column (see GEN_H/GEN_BM), not recomputed.
    // Deserts sprout sparse cacti instead of trees (1-3 tall on sand).
    if bm == B_DESERT {
        if blk[lidx(lx, h as usize, lz)] == SAND
            && hash2(wx, wz, seedx() + 400) % 97 == 0
            && (h + 3) < CH as i32
        {
            let tall = 1 + (hash2(wx, wz, seedx() + 401) % 3) as i32;
            let mut t = 1;
            while t <= tall {
                blk[lidx(lx, (h + t) as usize, lz)] = CACTUS;
                t += 1;
            }
        }
        return;
    }
    // Shallow sea floor seeds clay patches (smelts into brick).
    if h < SEA - 1 && h >= SEA - 4 && hash2(wx, wz, seedx() + 402) % 11 == 0 {
        if blk[lidx(lx, h as usize, lz)] == SAND || blk[lidx(lx, h as usize, lz)] == DIRT {
            blk[lidx(lx, h as usize, lz)] = CLAY;
        }
        return;
    }
    if bm != B_PLAINS && bm != B_MOUNTAIN {
        return;
    }
    if blk[lidx(lx, h as usize, lz)] != GRASS {
        return;
    }
    // Forest density: a hash gate, denser where a low-freq field is high. Plains
    // are mostly OPEN grassland (so the meadow decorations read and the leaf
    // overdraw stays cheap); only the higher-density patches thicken into a
    // walkable forest -- never the wall-to-wall canopy the old 1/7 gate made.
    let dens = vnoise::<24>(wx, wz, seedx() + 200);
    let gate = if dens > 168 { 13 } else { 48 };
    if hash2(wx, wz, seedx() + 300) % gate != 0 {
        return;
    }
    let base = h + 1;
    if base + 6 >= CH {
        return;
    }
    let mut t = 0;
    while t < 4 {
        blk[lidx(lx, (base + t) as usize, lz)] = WOOD;
        t += 1;
    }
    let mut dz = -2i32;
    while dz <= 2 {
        let mut dx = -2i32;
        while dx <= 2 {
            let dist = dx * dx + dz * dz;
            let y0 = base + 3;
            let y1 = base + if dist <= 2 { 5 } else { 4 };
            let mut ly = y0;
            while ly <= y1 {
                if dist <= 4 && !(dx == 0 && dz == 0 && ly < base + 5) {
                    let cx = (lx as i32 + dx) as usize;
                    let cz = (lz as i32 + dz) as usize;
                    blk[lidx(cx, ly as usize, cz)] = LEAVES;
                }
                ly += 1;
            }
            dx += 1;
        }
        dz += 1;
    }
}

/// Scatter a decorative cross-sprite plant (tall grass or a flower) on a grass
/// surface. Runs for every column (no tree-canopy margin) but only where the
/// cell above the grass is still AIR, so it never buries a tree trunk. Common
/// tall grass, rarer flowers -- a meadow look with no per-frame cost (they mesh
/// into the plant list like any cross plant).
fn maybe_decoration(blk: &mut [u8; CHUNK_VOL], lx: usize, lz: usize, wx: i32, wz: i32, h: i32) {
    // h arrives cached from gen_column (see GEN_H), not recomputed.
    if h < 0 || h + 1 >= CH as i32 {
        return;
    }
    if blk[lidx(lx, h as usize, lz)] != GRASS {
        return; // grass only -> plains/mountain surfaces, never sand/snow/water
    }
    let top = (h + 1) as usize;
    if blk[lidx(lx, top, lz)] != AIR {
        return; // a tree trunk (or anything) already occupies this cell
    }
    let plant = match hash2(wx, wz, seedx() + 500) % 100 {
        0..=23 => TALL_GRASS,
        24..=27 => FLOWER_R,
        28..=31 => FLOWER_Y,
        _ => return,
    };
    blk[lidx(lx, top, lz)] = plant;
}

/// Generate columns [from, from+count) into the unpacked GEN_SCRATCH (origin
/// ox,oz). Columns are indexed col = lz*CWU + lx; finish_chunk packs the scratch
/// into the chunk's 5-bit store once all columns are done. Returns next column.
// ---- Dimensions -------------------------------------------------------------
//
// The Inferno was recorded as descoped on RAM, on the assumption it needed a
// second chunk ring. It does not: a dimension is a GENERATOR SWITCH over the
// same ring. Travelling regenerates the loaded chunks with the other generator,
// which costs the usual worldgen time and no extra memory at all.
pub const DIM_OVERWORLD: u8 = 0;
pub const DIM_INFERNO: u8 = 1;
pub const DIM_VOID: u8 = 2;
static mut DIM: u8 = DIM_OVERWORLD;

pub fn dimension() -> u8 {
    unsafe { DIM }
}

/// Switch dimensions and rebuild the ring around `(bx, bz)`. The caller moves
/// the player; this only owns the world.
///
/// It regenerates the ring SYNCHRONOUSLY, through the same path boot uses, and
/// reports progress so the caller can put a loading screen up. Streaming it in
/// instead would return with no chunks loaded, and everything the caller then
/// writes -- the return portal above all -- would land in an unloaded slot and
/// be silently dropped, leaving the player in the Inferno with no way home.
pub fn set_dimension<F: FnMut(usize, usize)>(dim: u8, bx: i32, bz: i32, progress: F) {
    unsafe {
        if DIM == dim {
            return;
        }
        DIM = dim;
    }
    fluid_reset();
    // Drop every chunk: they hold the OTHER dimension's terrain.
    unsafe {
        let mut s = 0;
        while s < NCHUNKS {
            CHUNKS[s].loaded = false;
            CHUNKS[s].dirty = false;
            CHUNKS[s].face_slot = NO_SLOT;
            s += 1;
        }
        let mut p = 0;
        while p < POOL {
            POOL_OWNER[p] = usize::MAX;
            p += 1;
        }
    }
    sync_init(bx, bz, progress);
}

/// Synchronous gen + spawn pocket + mesh of the whole ring, behind a progress
/// bar. Only DIMENSION TRAVEL uses this now (a loading screen there is what
/// Minecraft does too); boot goes through boot_prepare + the menu's amortized
/// pump instead.
fn sync_init<F: FnMut(usize, usize)>(wx: i32, wz: i32, mut progress: F) {
    init_block_class();
    let pcx = floor_div(wx, CW);
    let pcz = floor_div(wz, CW);
    unsafe {
        PLAYER_CX = pcx;
        PLAYER_CZ = pcz;
    }
    let total = (NCHUNKS * 2) as usize; // gen ticks + mesh ticks (upper bound)
    let mut done = 0usize;
    let boot_r = GRID / 2;
    let mut j = -boot_r;
    while j <= boot_r {
        let mut i = -boot_r;
        while i <= boot_r {
            let cx = pcx + i;
            let cz = pcz + j;
            gen_chunk(slot(cx, cz), cx, cz);
            done += 1;
            progress(done, total);
            i += 1;
        }
        j += 1;
    }
    // Clear the arrival pocket before the mesh pass picks it up.
    let sy = surface_y(wx, wz);
    let mut dz = -2;
    while dz <= 2 {
        let mut dx = -2;
        while dx <= 2 {
            let mut y = sy;
            while y < sy + 6 {
                raw_set(wx + dx, y, wz + dz, AIR);
                y += 1;
            }
            dx += 1;
        }
        dz += 1;
    }
    let mut s = 0;
    while s < NCHUNKS {
        if unsafe { CHUNKS[s].loaded } && in_render_range(s) {
            mesh_chunk(s);
        }
        done += 1;
        progress(done, total);
        s += 1;
    }
}

/// Inferno column: a cinderstone shell with a lava sea in the floor, an open
/// middle, and a solid roof. Same five-run shape as the overworld generator, so
/// it costs the same per column.
fn gen_nether_column(blk: &mut [u8; CHUNK_VOL], lx: usize, lz: usize, wx: i32, wz: i32) {
    const LAVA_SEA: i32 = 10;
    const ROOF: i32 = CH - 6;
    // Rolling floor and roof from the same cheap hash noise the overworld uses.
    let f = (hash2(wx >> 2, wz >> 2, 0x5EA) & 7) + (hash2(wx >> 4, wz >> 4, 0x11) & 7);
    let floor = 6 + f; // 6..20
    let c = hash2(wx >> 3, wz >> 3, 0xC0FF) & 5;
    let roof = ROOF - c;
    unsafe {
        GEN_H[lz * CWU + lx] = floor as i16;
        GEN_BM[lz * CWU + lx] = B_MOUNTAIN; // unused down here; keeps the array valid
    }
    let base = lidx(lx, 0, lz);
    const STRIDE: usize = CWU * CWU;
    let mut put_run = |y0: i32, y1: i32, b: u8| {
        let lo = if y0 < 0 { 0 } else { y0 };
        let hi = if y1 > CH { CH } else { y1 };
        let mut y = lo;
        let mut i = base + lo as usize * STRIDE;
        while y < hi {
            blk[i] = b;
            i += STRIDE;
            y += 1;
        }
    };
    put_run(0, 1, STONE); // bedrock floor
    put_run(1, floor, CINDERSTONE);
    // Where the floor sits below sea level the hollow fills with lava, which is
    // what makes the Inferno's lava oceans.
    if floor <= LAVA_SEA {
        put_run(floor, LAVA_SEA + 1, LAVA);
        put_run(LAVA_SEA + 1, roof, AIR);
    } else {
        put_run(floor, roof, AIR);
    }
    put_run(roof, CH - 1, CINDERSTONE);
    put_run(CH - 1, CH, STONE); // bedrock roof
    // Sink sand patches on dry floor, lumistone blotches under the roof.
    if floor > LAVA_SEA && (hash2(wx, wz, 0x503) & 15) < 3 {
        blk[base + (floor as usize - 1) * STRIDE] = SINK_SAND;
        // ember cap grows on sink sand. It is the base of every potion, so
        // without a source down here the whole brewing chain is unreachable.
        if (hash2(wx, wz, 0x7A27) & 7) < 2 {
            blk[base + floor as usize * STRIDE] = EMBER_CAP;
        }
    }
    if (hash2(wx, wz, 0x61A) & 31) < 2 {
        blk[base + roof as usize * STRIDE] = LUMISTONE;
    }
}

/// End column: one floating island of void stone over the void, with obsidian
/// pillars around the rim. Outside the island radius the column is pure air --
/// that emptiness IS the Void, and it is also why this generator is the cheapest
/// of the three.
fn gen_end_column(blk: &mut [u8; CHUNK_VOL], lx: usize, lz: usize, wx: i32, wz: i32) {
    const ISLAND_R: i32 = 44; // blocks from the origin
    const DECK: i32 = 32; // island surface height
    let d2 = wx * wx + wz * wz;
    let base = lidx(lx, 0, lz);
    const STRIDE: usize = CWU * CWU;
    let mut put_run = |y0: i32, y1: i32, b: u8| {
        let lo = if y0 < 0 { 0 } else { y0 };
        let hi = if y1 > CH { CH } else { y1 };
        let mut y = lo;
        let mut i = base + lo as usize * STRIDE;
        while y < hi {
            blk[i] = b;
            i += STRIDE;
            y += 1;
        }
    };
    unsafe {
        GEN_H[lz * CWU + lx] = DECK as i16;
        GEN_BM[lz * CWU + lx] = B_MOUNTAIN;
    }
    if d2 > ISLAND_R * ISLAND_R {
        put_run(0, CH, AIR); // the void
        return;
    }
    // The island thins toward the rim, as Java's does.
    let r = isqrt_i32(d2);
    let thick = 4 + (ISLAND_R - r) / 4;
    put_run(0, DECK - thick, AIR);
    put_run(DECK - thick, DECK + 1, VOID_STONE);
    put_run(DECK + 1, CH, AIR);
    // Obsidian pillars on a ring, the dragon's perches.
    if r > ISLAND_R / 2 && r < ISLAND_R - 6 {
        let a = (hash2(wx / 6, wz / 6, 0xE4D) & 15) as i32;
        if a == 0 {
            let top = DECK + 6 + (hash2(wx / 6, wz / 6, 0x91) & 7);
            put_run(DECK + 1, top, OBSIDIAN);
        }
    }
}

fn gen_columns(ox: i32, oz: i32, from: usize, count: usize) -> usize {
    let blk: &mut [u8; CHUNK_VOL] = unsafe { &mut GEN_SCRATCH };
    let end = (from + count).min(CHUNK_AREA);
    if unsafe { DIM } == DIM_VOID {
        let mut col = from;
        while col < end {
            let lx = col % CWU;
            let lz = col / CWU;
            gen_end_column(blk, lx, lz, ox + lx as i32, oz + lz as i32);
            col += 1;
        }
        return col;
    }
    if unsafe { DIM } == DIM_INFERNO {
        let mut col = from;
        while col < end {
            let lx = col % CWU;
            let lz = col / CWU;
            gen_nether_column(blk, lx, lz, ox + lx as i32, oz + lz as i32);
            col += 1;
        }
        return col;
    }
    // Hash each shape noise's lattice ONCE for the chunk (45 hashes) instead of
    // per column (5120). Streaming calls this several times per chunk with the
    // same origin, so the rebuild is amortised anyway.
    let t = shape_tiles(ox, oz);
    if from == 0 {
        blk.fill(AIR);
    }
    let mut col = from;
    while col < end {
        let lx = col % CWU;
        let lz = col / CWU;
        gen_column(blk, &t, lx, lz, ox + lx as i32, oz + lz as i32);
        col += 1;
    }
    col
}

/// Scatter trees over the finished scratch, pack it into slot `s`'s 5-bit store,
/// then publish: mark loaded + dirty (so it meshes) and dirty its neighbours
/// (their border faces now resolve against real blocks instead of "air").
/// Decorate a range of columns (trees, flowers) into the gen scratch.
/// Sugar cane: on sand with water beside it, one to three tall (Java).
fn place_cane(blk: &mut [u8; CHUNK_VOL], lx: usize, lz: usize, wx: i32, wz: i32, h: i32) {
    if h < 1 || h + 3 >= CH {
        return;
    }
    let base = lidx(lx, 0, lz);
    const STRIDE: usize = CWU * CWU;
    if blk[base + h as usize * STRIDE] != SAND {
        return;
    }
    let wet = is_water(get(wx + 1, h, wz))
        || is_water(get(wx - 1, h, wz))
        || is_water(get(wx, h, wz + 1))
        || is_water(get(wx, h, wz - 1));
    if !wet || (hash2(wx, wz, 0x5CA9) & 7) != 0 {
        return;
    }
    let tall = 1 + (hash2(wx, wz, 0x11E) & 3).min(2);
    let mut k = 0;
    while k < tall {
        blk[base + (h + 1 + k) as usize * STRIDE] = SUGAR_CANE;
        k += 1;
    }
}

fn decorate_columns(cx: i32, cz: i32, from: usize, count: usize) -> usize {
    let ox = cx * CW;
    let oz = cz * CW;
    let blk: &mut [u8; CHUNK_VOL] = unsafe { &mut GEN_SCRATCH };
    let end = (from + count).min(CHUNK_AREA);
    if unsafe { DIM } != DIM_OVERWORLD {
        return end; // no trees, flowers or grass off-world
    }
    let mut col = from;
    while col < end {
        let lx = col % CWU;
        let lz = col / CWU;
        let (h, bm) = unsafe { (GEN_H[lz * CWU + lx] as i32, GEN_BM[lz * CWU + lx]) };
        maybe_tree(blk, lx, lz, ox + lx as i32, oz + lz as i32, h, bm);
        maybe_decoration(blk, lx, lz, ox + lx as i32, oz + lz as i32, h);
        place_cane(blk, lx, lz, ox + lx as i32, oz + lz as i32, h);
        col += 1;
    }
    end
}

/// Pack a range of scratch blocks into slot `s`'s bit store.
///
/// Packing in whole-byte groups keeps this resumable with no carry state:
/// BLOCK_BITS is 7, so 8 blocks are exactly 56 bits = 7 whole bytes. That is
/// what lets the pack be spread over several ticks instead of landing as one
/// 16K-iteration lump inside a single frame. (It was 4 blocks / 3 bytes at 6
/// bits; PACK_BATCH is a multiple of 8 either way.)
fn pack_blocks(s: usize, from: usize, count: usize) -> usize {
    // Release-safe guard where a debug_assert used to be: a misaligned or
    // out-of-range cursor here previously meant miscompiled gen state, and
    // the resulting index panic FROZE the console. Realign and log rather
    // than halt; the inline(never) barriers above are the real fix.
    let from = if from % 8 != 0 || from > CHUNK_VOL {
        psx_rt::tty::println("pack_blocks: misaligned cursor, realigned");
        (from & !7).min(CHUNK_VOL)
    } else {
        from
    };
    debug_assert!(from % 8 == 0 && count % 8 == 0);
    let end = (from + count).min(CHUNK_VOL);
    unsafe {
        let dst = &mut CHUNKS[s].blocks;
        let mut i = from;
        let mut bi = from / 8 * 7;
        while i < end {
            // Two 28-bit halves rather than one 56-bit value: the R3000 has no
            // 64-bit shift, so a u64 here would cost a library call per group.
            let lo = (GEN_SCRATCH[i] as u32) & BLOCK_MASK
                | (((GEN_SCRATCH[i + 1] as u32) & BLOCK_MASK) << 7)
                | (((GEN_SCRATCH[i + 2] as u32) & BLOCK_MASK) << 14)
                | (((GEN_SCRATCH[i + 3] as u32) & BLOCK_MASK) << 21);
            let hi = (GEN_SCRATCH[i + 4] as u32) & BLOCK_MASK
                | (((GEN_SCRATCH[i + 5] as u32) & BLOCK_MASK) << 7)
                | (((GEN_SCRATCH[i + 6] as u32) & BLOCK_MASK) << 14)
                | (((GEN_SCRATCH[i + 7] as u32) & BLOCK_MASK) << 21);
            dst[bi] = lo as u8;
            dst[bi + 1] = (lo >> 8) as u8;
            dst[bi + 2] = (lo >> 16) as u8;
            // Byte 3 straddles the halves: top nibble of lo, low nibble of hi.
            dst[bi + 3] = ((lo >> 24) as u8 & 0x0F) | ((hi as u8 & 0x0F) << 4);
            dst[bi + 4] = (hi >> 4) as u8;
            dst[bi + 5] = (hi >> 12) as u8;
            dst[bi + 6] = (hi >> 20) as u8;
            bi += 7;
            i += 8;
        }
    }
    end
}

/// Publish a fully generated + packed chunk and queue its own mesh.
///
/// Existing neighbours may retain solid boundary faces that were built while
/// this chunk was absent. Once the new solid neighbour appears those faces are
/// internal and cannot be seen; rebuilding four complete neighbours merely to
/// delete them doubled streaming demand. A later edit/LOD transition naturally
/// refreshes those ranges.
fn publish_chunk(s: usize, cx: i32, cz: i32) {
    replay_edits(s, cx, cz);
    unsafe {
        CHUNKS[s].loaded = true;
        CHUNKS[s].dirty = true;
    }
}

/// Stamp the player's block edits back onto a chunk that has just been
/// generated from the seed.
///
/// A chunk that leaves the 5x5 ring is thrown away, not stored, so coming back
/// re-derives it from noise -- and everything built there was gone. Walk far
/// enough that one side of a build leaves the ring, come back, and half of it
/// has reverted to terrain, which is exactly what an itch player reported
/// happening to their house. The edit log already existed for the memory card;
/// this makes it authoritative for streaming too.
///
/// Runs on the frame a chunk completes, which already carries the last gen
/// tick. A full scan of the log is a few thousand cycles against that tick's
/// ~40K, and only chunk completions pay it.
fn replay_edits(s: usize, cx: i32, cz: i32) {
    let dim = unsafe { DIM };
    let (x0, z0) = (cx * CW, cz * CW);
    let n = unsafe { crate::EDIT_N };
    let mut i = 0;
    while i < n {
        let (x, z) = unsafe { (crate::EDIT_X[i] as i32, crate::EDIT_Z[i] as i32) };
        if unsafe { crate::EDIT_D[i] } == dim
            && x >= x0
            && x < x0 + CW
            && z >= z0
            && z < z0 + CW
        {
            let y = unsafe { crate::EDIT_Y[i] as i32 };
            if y >= 0 && y < CH {
                let idx = lidx((x - x0) as usize, y as usize, (z - z0) as usize);
                unsafe { bset(&mut CHUNKS[s].blocks, idx, crate::EDIT_B[i]) };
            }
        }
        i += 1;
    }
}

fn finish_chunk(s: usize, cx: i32, cz: i32) {
    let blk: &mut [u8; CHUNK_VOL] = unsafe { &mut GEN_SCRATCH };
    if unsafe { DIM } == DIM_OVERWORLD {
        scatter_ores(blk, cx, cz); // no overworld ores down there
    }
    decorate_columns(cx, cz, 0, CHUNK_AREA);
    pack_blocks(s, 0, CHUNK_VOL);
    publish_chunk(s, cx, cz);
}

/// Synchronous full chunk gen (boot/init only). Streaming uses the amortized
/// recenter + gen_tick path so it never gens a whole chunk in one frame.
fn gen_chunk(s: usize, cx: i32, cz: i32) {
    unsafe {
        if MESH_S == s {
            abort_stream_mesh();
        }
        if MESH_SCRATCH_OWNER == s {
            MESH_SCRATCH_OWNER = usize::MAX;
        }
        // This slot may be recycled from a different chunk -- drop its stale faces.
        if CHUNKS[s].face_slot != NO_SLOT {
            POOL_OWNER[CHUNKS[s].face_slot as usize] = usize::MAX;
            CHUNKS[s].face_slot = NO_SLOT;
        }
        CHUNKS[s].cx = cx;
        CHUNKS[s].cz = cz;
    }
    gen_columns(cx * CW, cz * CW, 0, CHUNK_AREA);
    finish_chunk(s, cx, cz);
}

fn set_dirty(cx: i32, cz: i32) {
    let s = slot(cx, cz);
    unsafe {
        if CHUNKS[s].loaded && CHUNKS[s].cx == cx && CHUNKS[s].cz == cz {
            CHUNKS[s].dirty = true;
        }
    }
}

// --- public block access ---

/// Is the chunk containing this column resident?
///
/// Collision needs to tell "there is nothing here" apart from "we have not
/// generated here yet". `get` answers AIR for both, which is why walking off the
/// edge of the loaded ring dropped you through the floor: the ground under your
/// feet had simply not been generated, read as AIR, and nothing stopped the
/// fall.
pub fn column_loaded(wx: i32, wz: i32) -> bool {
    let cx = floor_div(wx, CW);
    let cz = floor_div(wz, CW);
    let s = slot(cx, cz);
    unsafe { CHUNKS[s].loaded && CHUNKS[s].cx == cx && CHUNKS[s].cz == cz }
}

pub fn get(wx: i32, wy: i32, wz: i32) -> u8 {
    if wy < 0 || wy >= CH {
        return AIR;
    }
    let cx = floor_div(wx, CW);
    let cz = floor_div(wz, CW);
    let s = slot(cx, cz);
    unsafe {
        if CHUNKS[s].loaded && CHUNKS[s].cx == cx && CHUNKS[s].cz == cz {
            let lx = (wx - cx * CW) as usize;
            let lz = (wz - cz * CW) as usize;
            bget(&CHUNKS[s].blocks, lidx(lx, wy as usize, lz))
        } else {
            AIR
        }
    }
}

#[inline(never)]
pub fn set(wx: i32, wy: i32, wz: i32, b: u8) {
    if wy < 0 || wy >= CH {
        return;
    }
    let cx = floor_div(wx, CW);
    let cz = floor_div(wz, CW);
    let s = slot(cx, cz);
    let (old, sky_changed) = unsafe {
        if !(CHUNKS[s].loaded && CHUNKS[s].cx == cx && CHUNKS[s].cz == cz) {
            return;
        }
        let lx = (wx - cx * CW) as usize;
        let lz = (wz - cz * CW) as usize;
        let i = lidx(lx, wy as usize, lz);
        let old = bget(&CHUNKS[s].blocks, i);
        // Does this edit move the column's skylight top? Scan for the current
        // top BEFORE the write (first sky-blocking cell from above, the same
        // test build_sky_column uses). The top rises if a blocking block lands
        // above it, and falls if the top block itself stops blocking. Light in
        // the whole column below then changes, which reaches planes far outside
        // the edit's local rebuild -- the commit uses this to queue a full
        // remesh behind the fast partial one.
        let mut col_top = 0i32;
        let mut y = CHU - 1;
        while y > 0 {
            let cb = bget(&CHUNKS[s].blocks, lidx(lx, y, lz));
            if BCLASS[cb as usize] & CLS_MESH != 0 {
                col_top = y as i32;
                break;
            }
            y -= 1;
        }
        let old_blocks = BCLASS[old as usize] & CLS_MESH != 0;
        let new_blocks = BCLASS[b as usize] & CLS_MESH != 0;
        let sky_changed = (new_blocks && wy > col_top)
            || (old_blocks && !new_blocks && wy == col_top);
        bset(&mut CHUNKS[s].blocks, i, b);
        if MESH_SCRATCH_OWNER == s {
            MESH_SCRATCH[i] = b;
            col_masks_set(i, b);
            if b != AIR {
                MESH_YHI = MESH_YHI.max(wy as usize);
    }
        }
        (old, sky_changed)
    };
    let lx = wx - cx * CW;
    let lz = wz - cz * CW;
    // A normal block break used to rebuild all ~198 mesh planes and cost about
    // 8.57M cycles (7.5 complete 30fps frame periods). Greedy merging never
    // crosses a plane, so only the planes touching the edited cell can change
    // topology or AO. Rebuild those and copy every unaffected plane from the
    // committed mesh; collision updates immediately and the mesh swaps in
    // atomically after its bounded rebuild.
    queue_mesh_edit(s, lx as usize, wy as usize, lz as usize, old, b, sky_changed);
    if lx == 0 {
        set_dirty(cx - 1, cz);
    }
    if lx == CW - 1 {
        set_dirty(cx + 1, cz);
    }
    if lz == 0 {
        set_dirty(cx, cz - 1);
    }
    if lz == CW - 1 {
        set_dirty(cx, cz + 1);
    }
}

fn remesh(cx: i32, cz: i32) {
    let s = slot(cx, cz);
    unsafe {
        if CHUNKS[s].loaded && CHUNKS[s].cx == cx && CHUNKS[s].cz == cz {
            if in_render_range(s) {
                mesh_chunk(s);
            } else {
                CHUNKS[s].dirty = true;
            }
        }
    }
}

/// Destroy breakable blocks in a sphere (sapper blast). Raw-sets every block
/// then remeshes the overlapping chunks once -- per-block `set` would remesh
/// the chunk ~100 times and hitch hard.
pub fn blast(wx: i32, wy: i32, wz: i32, r: i32) {
    let r2 = r * r;
    let mut dy = -r;
    while dy <= r {
        let mut dz = -r;
        while dz <= r {
            let mut dx = -r;
            while dx <= r {
                if dx * dx + dy * dy + dz * dz <= r2 {
                    let (x, y, z) = (wx + dx, wy + dy, wz + dz);
                    if y > 0 {
                        let b = get(x, y, z);
                        if b != AIR && b != LAVA && !is_water(b) {
                            raw_set(x, y, z, AIR);
                        }
                    }
                }
                dx += 1;
            }
            dz += 1;
        }
        dy += 1;
    }
    let c0x = floor_div(wx - r, CW);
    let c1x = floor_div(wx + r, CW);
    let c0z = floor_div(wz - r, CW);
    let c1z = floor_div(wz + r, CW);
    let mut cz = c0z - 1;
    while cz <= c1z + 1 {
        let mut cx = c0x - 1;
        while cx <= c1x + 1 {
            remesh(cx, cz);
            cx += 1;
        }
        cz += 1;
    }
}

// ---- Flowing water --------------------------------------------------------
//
// Java's rules, with Java's storage: a source is level 0, and each block a flow
// travels away from it costs one level until it runs out at 7. Water falls
// first and only spreads sideways when it cannot; a flow with nothing feeding
// it (no water above, no lower-level neighbour) dries up.
//
// It lives here rather than in main.rs because it MUST batch. `set` remeshes
// the chunk synchronously, so running a fluid through it would remesh a chunk
// per block placed and hitch for a second per bucket. This raw-sets, tracks the
// world rectangle it touched, and remeshes that once per tick -- the same trick
// `blast` uses.
//
// Nothing schedules itself: updates come only from player edits (`wake_fluid`)
// and from cells a flow just changed. That is also how Java behaves, and it is
// what keeps generated oceans inert -- otherwise every shoreline in the world
// would start draining into the nearest cave on the first frame.
const FLUID_Q: usize = 256;
static mut FQ_X: [i32; FLUID_Q] = [0; FLUID_Q];
static mut FQ_Y: [i32; FLUID_Q] = [0; FLUID_Q];
static mut FQ_Z: [i32; FLUID_Q] = [0; FLUID_Q];
static mut FQ_HEAD: usize = 0;
static mut FQ_LEN: usize = 0;
static mut FLUID_PHASE: u32 = 0;
/// Cells evaluated per fluid tick. Each one can set up to 4 blocks, so this is
/// also the frame's remesh footprint.
const FLUID_BUDGET: usize = 6;
/// Java spreads water every 5 game ticks; at 30fps this is the same cadence.
pub const FLUID_INTERVAL: u32 = 5;

fn fq_push(x: i32, y: i32, z: i32) {
    unsafe {
        if FQ_LEN >= FLUID_Q {
            return; // full: drop it. The flow stalls rather than the frame.
        }
        let i = (FQ_HEAD + FQ_LEN) % FLUID_Q;
        FQ_X[i] = x;
        FQ_Y[i] = y;
        FQ_Z[i] = z;
        FQ_LEN += 1;
    }
}

/// Schedule a cell and its six neighbours. Call after any player edit: placing
/// water, scooping it, or breaking the block that was holding it back.
pub fn wake_fluid(x: i32, y: i32, z: i32) {
    fq_push(x, y, z);
    fq_push(x + 1, y, z);
    fq_push(x - 1, y, z);
    fq_push(x, y + 1, z);
    fq_push(x, y - 1, z);
    fq_push(x, y, z + 1);
    fq_push(x, y, z - 1);
}

/// Water washes these away rather than flowing around them (Java drops them as
/// items; we just delete them).
fn fluid_replaceable(b: u8) -> bool {
    b == AIR || is_cross_plant(b)
}

/// The lowest level among the four horizontal neighbours, i.e. how close the
/// nearest source is. `WATER_MAX_RUN + 1` means none of them is water.
fn lowest_neighbour_level(x: i32, y: i32, z: i32) -> u8 {
    let mut best = WATER_MAX_RUN + 1;
    let mut k = 0;
    while k < 4 {
        let (dx, dz) = [(1, 0), (-1, 0), (0, 1), (0, -1)][k];
        let l = water_level(get(x + dx, y, z + dz));
        if l < best {
            best = l;
        }
        k += 1;
    }
    best
}

/// How many of the four horizontal neighbours are full source blocks.
fn source_neighbours(x: i32, y: i32, z: i32) -> u8 {
    let mut n = 0;
    let mut k = 0;
    while k < 4 {
        let (dx, dz) = [(1, 0), (-1, 0), (0, 1), (0, -1)][k];
        if get(x + dx, y, z + dz) == WATER {
            n += 1;
        }
        k += 1;
    }
    n
}

/// The chunks changed this tick. Tracked as a deduped list rather than a
/// bounding rectangle: `remesh` re-meshes synchronously, so a rect plus the
/// customary one-chunk ring would pay NINE chunk remeshes per fluid tick where
/// placing a single block pays one, and flowing water would hitch harder than
/// anything else in the game.
const TOUCH_CAP: usize = 16;
struct Touched {
    n: usize,
    cx: [i32; TOUCH_CAP],
    cz: [i32; TOUCH_CAP],
}

impl Touched {
    fn mark(&mut self, cx: i32, cz: i32) {
        let mut i = 0;
        while i < self.n {
            if self.cx[i] == cx && self.cz[i] == cz {
                return;
            }
            i += 1;
        }
        if self.n < TOUCH_CAP {
            self.cx[self.n] = cx;
            self.cz[self.n] = cz;
            self.n += 1;
        }
        // Over capacity: that chunk keeps a stale mesh until the next edit
        // touches it. Cannot happen at the current budget (6 cells, each
        // setting at most 4 blocks, all on one flow front), and a missing
        // remesh is a cosmetic lag rather than a wrong world.
    }
}

fn fluid_put(t: &mut Touched, x: i32, y: i32, z: i32, b: u8) {
    raw_set(x, y, z, b);
    // Same border rule `set` uses: a block on a chunk edge shows faces in the
    // neighbour's mesh too.
    let cx = floor_div(x, CW);
    let cz = floor_div(z, CW);
    t.mark(cx, cz);
    let lx = x - cx * CW;
    let lz = z - cz * CW;
    if lx == 0 {
        t.mark(cx - 1, cz);
    }
    if lx == CW - 1 {
        t.mark(cx + 1, cz);
    }
    if lz == 0 {
        t.mark(cx, cz - 1);
    }
    if lz == CW - 1 {
        t.mark(cx, cz + 1);
    }
    // Only the cell that CHANGED wakes its neighbours; scheduling the whole
    // evaluated cell's surroundings every step floods the queue.
    wake_fluid(x, y, z);
}

fn fluid_step(t: &mut Touched, x: i32, y: i32, z: i32) {
    if y <= 0 || y >= CH {
        return;
    }
    let b = get(x, y, z);
    if is_water(b) {
        water_step(t, x, y, z, water_level(b));
    } else if is_lava(b) {
        lava_step(t, x, y, z, lava_level(b));
    }
}

fn water_step(t: &mut Touched, x: i32, y: i32, z: i32, level: u8) {
    if level > 0 {
        // Java: a flow with two or more SOURCE neighbours becomes a source
        // itself. That single rule is what makes a 2x2 hole an infinite water
        // supply, and it cannot run away -- a channel fed by one source never
        // has two source neighbours.
        if source_neighbours(x, y, z) >= 2 && !fluid_replaceable(get(x, y - 1, z)) {
            fluid_put(t, x, y, z, WATER);
            return;
        }
        // Flowing water needs a supply: water directly above, or a horizontal
        // neighbour nearer the source than it is.
        let fed = is_water(get(x, y + 1, z)) || lowest_neighbour_level(x, y, z) < level;
        if !fed {
            fluid_put(t, x, y, z, AIR);
            return;
        }
    }
    // Down first, and at full strength: a waterfall reaches the floor and only
    // then spreads out, which is what makes Java's fluids read as falling.
    let below = get(x, y - 1, z);
    if fluid_replaceable(below) {
        fluid_put(t, x, y - 1, z, water_of_level(1));
        return;
    }
    if is_lava(below) {
        let made = if below == LAVA { OBSIDIAN } else { COBBLE };
        fluid_put(t, x, y - 1, z, made);
        return;
    }
    if is_water(below) {
        return; // pooling, not spreading
    }
    let next = level + 1;
    if next > WATER_MAX_RUN {
        return; // out of reach of the source
    }
    let mut k = 0;
    while k < 4 {
        let (dx, dz) = [(1, 0), (-1, 0), (0, 1), (0, -1)][k];
        let (nx, nz) = (x + dx, z + dz);
        let nb = get(nx, y, nz);
        if is_lava(nb) {
            // Java: a lava SOURCE becomes obsidian, flowing lava becomes cobble.
            let made = if nb == LAVA { OBSIDIAN } else { COBBLE };
            fluid_put(t, nx, y, nz, made);
        } else if fluid_replaceable(nb) || water_level(nb) > next {
            // `water_level` returns MAX+1 for non-water, so the second test only
            // fires for a weaker flow we are now feeding better.
            fluid_put(t, nx, y, nz, water_of_level(next));
        }
        k += 1;
    }
}

/// Same shape as `water_step`, three differences: the run is 3 blocks not 7,
/// there is no infinite-source rule (two lava sources never make a third), and
/// water in the path wins -- lava flowing into water becomes stone.
fn lava_step(t: &mut Touched, x: i32, y: i32, z: i32, level: u8) {
    if level > 0 {
        let fed = is_lava(get(x, y + 1, z)) || lowest_lava_level(x, y, z) < level;
        if !fed {
            fluid_put(t, x, y, z, AIR);
            return;
        }
    }
    let below = get(x, y - 1, z);
    if fluid_replaceable(below) {
        fluid_put(t, x, y - 1, z, lava_of_level(1));
        return;
    }
    if is_water(below) {
        fluid_put(t, x, y - 1, z, STONE); // Java: lava onto water makes stone
        return;
    }
    if is_lava(below) {
        return;
    }
    let next = level + 1;
    if next > LAVA_MAX_RUN {
        return;
    }
    let mut k = 0;
    while k < 4 {
        let (dx, dz) = [(1, 0), (-1, 0), (0, 1), (0, -1)][k];
        let (nx, nz) = (x + dx, z + dz);
        let nb = get(nx, y, nz);
        if is_water(nb) {
            fluid_put(t, nx, y, nz, COBBLE);
        } else if fluid_replaceable(nb) || lava_level(nb) > next {
            fluid_put(t, nx, y, nz, lava_of_level(next));
        }
        k += 1;
    }
}

fn lowest_lava_level(x: i32, y: i32, z: i32) -> u8 {
    let mut best = LAVA_MAX_RUN + 1;
    let mut k = 0;
    while k < 4 {
        let (dx, dz) = [(1, 0), (-1, 0), (0, 1), (0, -1)][k];
        let l = lava_level(get(x + dx, y, z + dz));
        if l < best {
            best = l;
        }
        k += 1;
    }
    best
}

/// Advance the fluid queue by one budget's worth. Call on an interval, not
/// every frame.
///
/// inline(never) is load-bearing: the gameplay loop is one enormous function
/// and inlining this into it pushes a conditional branch past MIPS's +/-128KB
/// PC16 range, which fails at link time ("out of range PC16 fixup"). It runs
/// once every five frames, so the call ABI is free.
#[inline(never)]
pub fn fluid_tick() {
    // Java runs lava six times slower than water in the overworld. Rather than a
    // second queue, lava cells only step on every sixth tick and get put back
    // otherwise -- capped, so a queue full of lava cannot spin.
    let lava_turn = unsafe { FLUID_PHASE % 6 == 0 };
    unsafe { FLUID_PHASE = FLUID_PHASE.wrapping_add(1) };
    let mut t = Touched {
        n: 0,
        cx: [0; TOUCH_CAP],
        cz: [0; TOUCH_CAP],
    };
    let mut n = 0usize;
    let mut deferred = 0usize;
    while n < FLUID_BUDGET {
        let have = unsafe { FQ_LEN } > 0;
        if !have {
            break;
        }
        let (x, y, z) = unsafe {
            let i = FQ_HEAD;
            FQ_HEAD = (FQ_HEAD + 1) % FLUID_Q;
            FQ_LEN -= 1;
            (FQ_X[i], FQ_Y[i], FQ_Z[i])
        };
        if !lava_turn && is_lava(get(x, y, z)) {
            fq_push(x, y, z);
            deferred += 1;
            if deferred > FLUID_BUDGET * 2 {
                break; // all lava this round; leave the rest for the lava tick
            }
            continue;
        }
        fluid_step(&mut t, x, y, z);
        try_ignite(&mut t, x, y, z);
        n += 1;
    }
    // Flames age on the lava cadence, which is slow enough to watch.
    if lava_turn {
        fire_tick(&mut t);
    }
    // One remesh per chunk that actually changed, instead of one per block set.
    let mut i = 0;
    while i < t.n {
        remesh(t.cx[i], t.cz[i]);
        i += 1;
    }
}

// ---- Fire ------------------------------------------------------------------
//
// Rides the fluid queue rather than adding a second one: ignition and burn-out
// are the same kind of local, edit-driven cellular update, and sharing the queue
// means sharing the batched remesh too. A tracked cell is (position, fuel);
// fuel counts down each fire tick and the block it stands on goes with it.
const FIRE_CAP: usize = 16;
static mut FIRE_X: [i32; FIRE_CAP] = [0; FIRE_CAP];
static mut FIRE_Y: [i32; FIRE_CAP] = [0; FIRE_CAP];
static mut FIRE_Z: [i32; FIRE_CAP] = [0; FIRE_CAP];
static mut FIRE_FUEL: [u8; FIRE_CAP] = [0; FIRE_CAP]; // 0 = free slot
/// Fire ticks a flame survives before it burns out.
const FIRE_LIFE: u8 = 12;

fn light_fire(t: &mut Touched, x: i32, y: i32, z: i32) {
    if !fluid_replaceable(get(x, y, z)) {
        return;
    }
    let mut i = 0;
    while i < FIRE_CAP {
        if unsafe { FIRE_FUEL[i] } == 0 {
            unsafe {
                FIRE_X[i] = x;
                FIRE_Y[i] = y;
                FIRE_Z[i] = z;
                FIRE_FUEL[i] = FIRE_LIFE;
            }
            fluid_put(t, x, y, z, FIRE);
            return;
        }
        i += 1;
    }
    // Pool full: the block simply does not catch. A world where every log is
    // alight at once is not one this machine wants to remesh anyway.
}

/// True if any of the six neighbours is lava OR fire. Java spreads flame between
/// flammable blocks, and so do we now -- the FIRE_CAP pool is what bounds it, so
/// a forest fire is capped at sixteen live flames however big the forest is.
fn ignition_adjacent(x: i32, y: i32, z: i32) -> bool {
    let hot = |b: u8| is_lava(b) || b == FIRE;
    hot(get(x + 1, y, z))
        || hot(get(x - 1, y, z))
        || hot(get(x, y, z + 1))
        || hot(get(x, y, z - 1))
        || hot(get(x, y + 1, z))
        || hot(get(x, y - 1, z))
}

fn fire_tick(t: &mut Touched) {
    let mut i = 0;
    while i < FIRE_CAP {
        let fuel = unsafe { FIRE_FUEL[i] };
        if fuel == 0 {
            i += 1;
            continue;
        }
        let (x, y, z) = unsafe { (FIRE_X[i], FIRE_Y[i], FIRE_Z[i]) };
        if get(x, y, z) != FIRE {
            unsafe { FIRE_FUEL[i] = 0 }; // put out by an edit or a flood
            i += 1;
            continue;
        }
        // Water anywhere adjacent douses it, the way a bucket does in Java.
        let doused = is_water(get(x + 1, y, z))
            || is_water(get(x - 1, y, z))
            || is_water(get(x, y, z + 1))
            || is_water(get(x, y, z - 1))
            || is_water(get(x, y + 1, z));
        unsafe { FIRE_FUEL[i] = fuel - 1 };
        if doused || fuel == 1 {
            fluid_put(t, x, y, z, AIR);
            // Burnt out: the fuel underneath goes with it, so a wooden floor
            // actually disappears rather than smouldering forever.
            if !doused && is_flammable(get(x, y - 1, z)) {
                fluid_put(t, x, y - 1, z, AIR);
            }
            // Wake the ring around it: the neighbours are the next thing to
            // catch, and nothing else would ever schedule them.
            let mut k = 0;
            while k < 4 {
                let (dx, dz) = [(1, 0), (-1, 0), (0, 1), (0, -1)][k];
                wake_fluid(x + dx, y - 1, z + dz);
                k += 1;
            }
            unsafe { FIRE_FUEL[i] = 0 };
        }
        i += 1;
    }
}

/// Public ignition, for flint and steel: light the air above the struck block.
pub fn light_fire_at(x: i32, y: i32, z: i32) {
    let mut t = Touched {
        n: 0,
        cx: [0; TOUCH_CAP],
        cz: [0; TOUCH_CAP],
    };
    light_fire(&mut t, x, y, z);
    let mut i = 0;
    while i < t.n {
        remesh(t.cx[i], t.cz[i]);
        i += 1;
    }
}

/// Look for flammable blocks next to lava and set them alight. Budgeted the
/// same way the fluid step is: only cells the player woke get considered.
fn try_ignite(t: &mut Touched, x: i32, y: i32, z: i32) {
    if !is_flammable(get(x, y, z)) {
        return;
    }
    if !ignition_adjacent(x, y, z) {
        return;
    }
    // Fire sits in the air ABOVE what burns, as in Java.
    light_fire(t, x, y + 1, z);
}

/// Drop every pending fluid update (new world / teleport): the queue holds
/// absolute block coordinates that mean nothing once the world changes.
pub fn fluid_reset() {
    unsafe {
        FQ_HEAD = 0;
        FQ_LEN = 0;
        FLUID_PHASE = 0;
        FIRE_FUEL = [0; FIRE_CAP];
    }
}

/// Set a block without remeshing (for bulk apply, e.g. loading a save).
pub fn set_raw_pub(wx: i32, wy: i32, wz: i32, b: u8) {
    raw_set(wx, wy, wz, b);
}

/// Diagnostic: (loaded, meshed, faces-of-chunk-at-wx/wz) for the demo HUD.
pub fn world_stats(wx: i32, wz: i32) -> (u16, u16, u16) {
    let mut loaded = 0;
    let mut meshed = 0;
    let mut s = 0;
    while s < NCHUNKS {
        unsafe {
            if CHUNKS[s].loaded {
                loaded += 1;
                if CHUNKS[s].face_slot != NO_SLOT {
                    meshed += 1;
                }
            }
        }
        s += 1;
    }
    let _ = (wx, wz);
    let mut total: u32 = 0;
    let mut p = 0;
    while p < POOL {
        unsafe {
            if POOL_OWNER[p] != usize::MAX {
                total += POOL_NFACE[p] as u32;
            }
        }
        p += 1;
    }
    (loaded, meshed, (total / 10).min(999) as u16)
}

/// Grow a tree in place of a sapling at (wx, wy, wz): 4-block trunk + a leaf
/// blob, bulk raw-set then one remesh of the touched chunks (blast's pattern).
pub fn grow_tree(wx: i32, wy: i32, wz: i32) {
    let h = 4;
    let mut i = 0;
    while i < h {
        raw_set(wx, wy + i, wz, WOOD);
        i += 1;
    }
    // Leaf blob: 3x3 at the two levels below the crown, plus a top cross.
    let mut ly = wy + h - 2;
    while ly < wy + h {
        let mut dz = -1;
        while dz <= 1 {
            let mut dx = -1;
            while dx <= 1 {
                if !(dx == 0 && dz == 0) && get(wx + dx, ly, wz + dz) == AIR {
                    raw_set(wx + dx, ly, wz + dz, LEAVES);
                }
                dx += 1;
            }
            dz += 1;
        }
        ly += 1;
    }
    if get(wx, wy + h, wz) == AIR {
        raw_set(wx, wy + h, wz, LEAVES);
    }
    if get(wx + 1, wy + h, wz) == AIR {
        raw_set(wx + 1, wy + h, wz, LEAVES);
    }
    if get(wx - 1, wy + h, wz) == AIR {
        raw_set(wx - 1, wy + h, wz, LEAVES);
    }
    if get(wx, wy + h, wz + 1) == AIR {
        raw_set(wx, wy + h, wz + 1, LEAVES);
    }
    if get(wx, wy + h, wz - 1) == AIR {
        raw_set(wx, wy + h, wz - 1, LEAVES);
    }
    let cx = floor_div(wx, CW);
    let cz = floor_div(wz, CW);
    let mut dz = -1;
    while dz <= 1 {
        let mut dx = -1;
        while dx <= 1 {
            remesh(cx + dx, cz + dz);
            dx += 1;
        }
        dz += 1;
    }
}

/// Re-mesh loaded chunks after a bulk edit (e.g. loading a save). In-range chunks
/// mesh now; the rest are flagged dirty so they mesh when the player nears them.
#[inline(never)]
pub fn remesh_loaded() {
    let mut s = 0;
    while s < NCHUNKS {
        unsafe {
            if CHUNKS[s].loaded {
                if in_render_range(s) {
                    mesh_chunk(s);
                } else {
                    CHUNKS[s].dirty = true;
                }
            }
        }
        s += 1;
    }
}

/// Top ground block (surface + 1) at a column, for spawning. Skips fluids and
/// vegetation so a tree trunk never reads as the surface.
pub fn surface_y(wx: i32, wz: i32) -> i32 {
    if unsafe { DIM } == DIM_INFERNO {
        // Scanning down from the roof would land you inside it. Walk UP from
        // the floor to the first open pair of blocks instead.
        let mut y = 1;
        while y < CH - 2 {
            if get(wx, y, wz) == AIR && get(wx, y + 1, wz) == AIR && get(wx, y - 1, wz) != AIR {
                return y;
            }
            y += 1;
        }
        return 1;
    }
    // An UNGENERATED column has no blocks, so the scan below would read AIR all
    // the way down and answer 1 -- the bottom of the world. That is the same
    // "AIR means two different things" bug that dropped the player through the
    // floor at the streaming edge, and here it is worse: surface_y feeds RESPAWN,
    // mob spawning and portal placement, so dying near an unloaded column put you
    // at bedrock or in the void. The noise field knows the terrain height without
    // needing the chunk, which is exactly what pick_spawn relies on.
    if !column_loaded(wx, wz) {
        return height_at(wx, wz) + 1;
    }
    let mut y = CH - 1;
    while y > 0 {
        let b = get(wx, y, wz);
        if b != AIR && !is_water(b) && b != LEAVES && b != WOOD {
            return y + 1;
        }
        y -= 1;
    }
    1
}

/// Write a block without remeshing (used during gen, before the mesh pass).
fn raw_set(wx: i32, wy: i32, wz: i32, b: u8) {
    if wy < 0 || wy >= CH {
        return;
    }
    let cx = floor_div(wx, CW);
    let cz = floor_div(wz, CW);
    let s = slot(cx, cz);
    unsafe {
        if CHUNKS[s].loaded && CHUNKS[s].cx == cx && CHUNKS[s].cz == cz {
            let lx = (wx - cx * CW) as usize;
            let lz = (wz - cz * CW) as usize;
            let i = lidx(lx, wy as usize, lz);
            bset(&mut CHUNKS[s].blocks, i, b);
            if MESH_SCRATCH_OWNER == s {
                MESH_SCRATCH[i] = b;
                col_masks_set(i, b);
                if b != AIR {
                    MESH_YHI = MESH_YHI.max(wy as usize);
                }
            }
        }
    }
}

// --- meshing ---

const MAX_MERGE: usize = 12; // w-axis cap (4-bit field)
/// h-axis cap: 3-bit field, so the skylight bit fits in the same word.
const MAX_MERGE_H: usize = 8;
const MAX_MERGE_FAR: usize = 15; // LOD: distant chunks merge harder -> far fewer faces to
                                 // iterate/project. Coarser/blockier, but small on screen.
                                 // 15*16=240 keeps UV in a u8 and (w-1) in 4 bits.
// Current merge cap for the chunk being meshed; mesh_chunk/stream_tick set it per LOD.
static mut MESH_CAP: usize = MAX_MERGE;
// Chunks at a chunk-distance > LOD_R from the player mesh at the coarse (far) LOD.
/// LOD off. Note what the "LOD" here actually IS before reviving it: it only
/// swaps the greedy merge cap from MAX_MERGE (12) to MAX_MERGE_FAR (15) on
/// distant chunks. That is a merge-cap tweak, not a resolution LOD -- it does
/// not resample the chunk coarsely, so it cannot collapse a hill into fewer
/// faces the way a real LOD would.
///
/// Measured on the green spawn, which is the scene that needs it:
///     LOD off (as shipped)      world faces   959,953   18.8 fps
///     LOD_R = 1, occlusion off  world faces 1,001,583   18.4 fps
/// Slightly WORSE -- 12 to 15 is barely more merging and the per-chunk LOD
/// bookkeeping is not free. Also beware that enabling it silently switches on
/// chunk_occluded, whose gate is chunk_lod(s) == 1: with both on the same scene
/// measured 1,575,128 faces and 13.9 fps. My first run of this experiment was
/// confounded exactly that way.
///
/// Getting a green overworld to 30fps needs a REAL LOD -- coarse resampled
/// meshes for distant chunks -- which does not exist here.
const LOD_R: i32 = 99;

#[inline]
fn pack(
    lx: usize,
    ly: usize,
    lz: usize,
    dir: usize,
    block: u8,
    w: usize,
    h: usize,
    light: u16,
) -> u32 {
    (lx as u32)
        | ((ly as u32) << 4)
        | ((lz as u32) << 10)
        | ((dir as u32) << 14)
        | ((block as u32) << 17) // 7 bits, matching BLOCK_BITS (lava levels mesh)
        // w keeps 4 bits; h drops to 3 (cap 8) to buy the ONE bit skylight needs
        // to reach the renderer. Taking two bits -- capping BOTH axes at 8 --
        // measured 1,712,429 cycles against 1,141,820: more faces, and the face
        // loop is half the frame. One axis is the affordable half of that.
        | (((w - 1) as u32) << 24)
        | (((h - 1) as u32) << 28)
        | ((light as u32) << 31)
}

/// Per-block mesh classification, built once by `init`. Replaces the compare
/// chains that ran on every one of the ~98K plane-cells per chunk mesh
/// (`is_cross_plant` alone was six comparisons a cell).
///   bit 0 MESHABLE  -- this block emits faces at all
///   bit 1 SEE_THRU  -- a neighbour of this kind leaves the face behind it visible
static mut BCLASS: [u8; 128] = [0; 128];
const CLS_MESH: u8 = 1;
const CLS_SEE: u8 = 2;
/// Light source, cross plant or small block: the decode records these per
/// cell, so the compare chains only run on blocks that carry this bit.
const CLS_SPECIAL: u8 = 4;

fn init_block_class() {
    unsafe {
        let mut i = 0usize;
        while i < 128 {
            let b = i as u8;
            // An OPEN door is invisible (never meshed) but stays a real block so
            // the pick ray can target it. Cross-sprite plants are the same: they
            // render as X-billboards in the plant pass, never as cube faces.
            let meshable = b != AIR && b != DOOR_O && !is_cross_plant(b) && !is_small_block(b);
            // A face shows against a SEE-THROUGH neighbour, not just air, so the
            // sea floor under water gets meshed and the ground under a plant
            // still draws.
            let see =
                b == AIR || is_water(b) || b == DOOR_O || is_cross_plant(b) || is_small_block(b);
            let special = is_light_source(b) || is_cross_plant(b) || is_small_block(b);
            BCLASS[i] = (meshable as u8) | ((see as u8) << 1) | ((special as u8) << 2);
            i += 1;
        }
    }
}

// MESH_SCRATCH index deltas for the six face directions: lidx is
// ly*256 + lz*16 + lx, so each axis step is a constant stride.
const NOFF: [i32; 6] = [1, -1, 256, -256, 16, -16];

/// Fill FMASK for one `dir`-plane: FMASK[cell] = the block whose face shows
/// there, or AIR.
///
/// The neighbour offset runs along the plane's own normal, so a plane is either
/// ENTIRELY interior or ENTIRELY on a chunk border -- never mixed. The interior
/// case (186 of the 192 planes) is then a flat walk of two constant strides with
/// no per-cell coordinate rebuild, no CHUNKS[s] indexing (that struct is ~12KB,
/// so `CHUNKS[s].cx` was a multiply by a big constant on every cell) and no
/// cross-chunk `get()` (which costs a rem_euclid division).
fn build_mask(s: usize, dir: usize, plane: usize, a_dim: usize, b_dim: usize) -> bool {
    let (base, sa, sb) = match dir {
        0 | 1 => (plane, CWU * CWU, CWU),
        2 | 3 => (plane * CWU * CWU, 1, CWU),
        _ => (plane * CWU, 1, CWU * CWU),
    };
    let border = match dir {
        0 | 4 => plane == CWU - 1,
        2 => plane == CHU - 1,
        _ => plane == 0,
    };
    let mut any = false;
    if border {
        // Cold: the neighbour lives in the next chunk, so go through `get`.
        // The dense fill below writes every cell; the row bitmaps are
        // rebuilt from it afterwards.
        let (cx, cz) = unsafe { (CHUNKS[s].cx, CHUNKS[s].cz) };
        let d = DIRS[dir];
        let mut b = 0;
        while b < b_dim {
            let mut a = 0;
            while a < a_dim {
                let (lx, ly, lz) = cell_to_local(dir, plane, a, b);
                let blk = unsafe { MESH_SCRATCH[lidx(lx, ly, lz)] };
                let mut f = AIR;
                if unsafe { BCLASS[blk as usize] } & CLS_MESH != 0 {
                    let nb = get(
                        cx * CW + lx as i32 + d.0,
                        ly as i32 + d.1,
                        cz * CW + lz as i32 + d.2,
                    );
                    if unsafe { BCLASS[nb as usize] } & CLS_SEE != 0 && nb != blk {
                        f = blk;
                    }
                }
                let cell = if f == AIR {
                    0
                } else {
                    f as u16 | (sky_bucket(lx, ly, lz) << 7)
                };
                unsafe { FMASK[b * a_dim + a] = cell };
                any |= f != AIR;
                a += 1;
            }
            b += 1;
        }
        let mut b = 0;
        while b < b_dim {
            let mut bits = 0u64;
            let mut a = 0;
            while a < a_dim {
                if unsafe { FMASK[b * a_dim + a] } != 0 {
                    bits |= 1u64 << a;
                }
                a += 1;
            }
            unsafe { FMASK_BITS[b] = bits };
            b += 1;
        }
        unsafe { FMASK_ROWS = !0 };
        return any;
    }
    let _ = (base, sa, sb);
    // Interior planes: a face cell is a meshable block whose neighbour along
    // the face normal can be seen through and is a different block. The
    // first two are one 64-bit AND of the column masks (COL_MESH of this
    // column against COL_SEE of the neighbour column), so only the surviving
    // bits are visited; the old code read two blocks and up to two class
    // bytes for every one of the plane's cells (about 90K cells a chunk).
    // The cell values written are identical to that scan's. Only face cells
    // are written: the greedy merge zeroes every cell it consumes and it
    // consumes every non-zero cell, so FMASK is all zero between planes (the
    // border path still writes every cell of its plane).
    let mut rows = 0u64;
    // Row cell bitmaps start empty for an interior plane (the previous plane's
    // merge consumed every set bit).
    match dir {
        2 | 3 => {
            // a = lx, b = lz, both 16: FMASK index is the column index.
            let nplane = if dir == 2 { plane + 1 } else { plane - 1 };
            // Which y-planes hold any candidate at all, for this direction:
            // OR of every column's (mesh & shifted see). Rebuilt when the
            // column masks changed since the last time.
            unsafe {
                if Y_ANY_EPOCH != COL_MASKS_EPOCH {
                    let mut up = 0u64;
                    let mut down = 0u64;
                    let mut c = 0usize;
                    while c < CWU * CWU {
                        let m = COL_MESH[c];
                        let v = COL_SEE[c];
                        up |= m & (v >> 1);
                        down |= m & (v << 1);
                        c += 1;
                    }
                    Y_ANY = [up, down];
                    Y_ANY_EPOCH = COL_MASKS_EPOCH;
                }
                if (Y_ANY[dir - 2] >> plane) & 1 == 0 {
                    FMASK_ROWS = 0;
                    return false;
                }
            }
            // Test the plane's bit through 32-bit halves: a 64-bit shift by a
            // runtime count is a branchy multi-word sequence on the R3000, and
            // this ran once per column per plane.
            let (hp, sp) = (plane >> 5, (plane & 31) as u32);
            let (hn, sn) = (nplane >> 5, (nplane & 31) as u32);
            let mesh_words = unsafe { &*(core::ptr::addr_of!(COL_MESH) as *const [[u32; 2]; CWU * CWU]) };
            let see_words = unsafe { &*(core::ptr::addr_of!(COL_SEE) as *const [[u32; 2]; CWU * CWU]) };
            let mut col = 0usize;
            while col < CWU * CWU {
                let cand = (mesh_words[col][hp] >> sp) & (see_words[col][hn] >> sn) & 1;
                if cand != 0 {
                    let blk = unsafe { MESH_SCRATCH[col + plane * CWU * CWU] };
                    let nb = unsafe { MESH_SCRATCH[col + nplane * CWU * CWU] };
                    if blk != nb {
                        let lx = col & (CWU - 1);
                        let lz = col / CWU;
                        unsafe {
                            FMASK[col] = blk as u16 | (sky_bucket(lx, plane, lz) << 7);
                            FMASK_BITS[lz] |= 1u64 << lx;
                        }
                        rows |= 1u64 << lz;
                        any = true;
                    }
                }
                col += 1;
            }
        }
        0 | 1 => {
            // plane = lx; a = ly (a_dim = hy), b = lz.
            let nlx = if dir == 0 { plane + 1 } else { plane - 1 };
            let mut lz = 0usize;
            while lz < CWU {
                let mut cand = unsafe { COL_MESH[lz * CWU + plane] & COL_SEE[lz * CWU + nlx] };
                while cand != 0 {
                    let ly = cand.trailing_zeros() as usize;
                    cand &= cand - 1;
                    if ly >= a_dim {
                        break;
                    }
                    let blk = unsafe { MESH_SCRATCH[lidx(plane, ly, lz)] };
                    let nb = unsafe { MESH_SCRATCH[lidx(nlx, ly, lz)] };
                    if blk != nb {
                        unsafe {
                            FMASK[lz * a_dim + ly] = blk as u16 | (sky_bucket(plane, ly, lz) << 7);
                            FMASK_BITS[lz] |= 1u64 << ly;
                        }
                        rows |= 1u64 << lz;
                        any = true;
                    }
                }
                lz += 1;
            }
        }
        _ => {
            // plane = lz; a = lx (16), b = ly (b_dim = hy).
            let nlz = if dir == 4 { plane + 1 } else { plane - 1 };
            let mut lx = 0usize;
            while lx < CWU {
                let mut cand = unsafe { COL_MESH[plane * CWU + lx] & COL_SEE[nlz * CWU + lx] };
                while cand != 0 {
                    let ly = cand.trailing_zeros() as usize;
                    cand &= cand - 1;
                    if ly >= b_dim {
                        break;
                    }
                    let blk = unsafe { MESH_SCRATCH[lidx(lx, ly, plane)] };
                    let nb = unsafe { MESH_SCRATCH[lidx(lx, ly, nlz)] };
                    if blk != nb {
                        unsafe {
                            FMASK[ly * a_dim + lx] = blk as u16 | (sky_bucket(lx, ly, plane) << 7);
                            FMASK_BITS[ly] |= 1u64 << lx;
                        }
                        rows |= 1u64 << ly;
                        any = true;
                    }
                }
                lx += 1;
            }
        }
    }
    unsafe { FMASK_ROWS = rows };
    any
}

#[inline]
fn push_face(
    n: usize,
    lx: usize,
    ly: usize,
    lz: usize,
    dir: usize,
    blk: u8,
    w: usize,
    h: usize,
    light: u16,
    ao: u8,
) -> usize {
    if n < MAX_FACES {
        unsafe {
            MESH_FACES[n] = pack(lx, ly, lz, dir, blk, w, h, 0);
            MESH_AO[n] = (ao as u16) | ((light & 7) << 8);
        }
        n + 1
    } else {
        n
    }
}

// ---------------------------------------------------------------- ambient occlusion
//
// Minecraft's classic vertex AO, computed HERE (mesh time) rather than per
// frame: chunk meshes are cached in the pool and only rebuilt on edit, so the
// cost amortizes to nothing, whereas a per-frame version would land in the face
// loop that is already half the frame.

/// All four corners fully lit -- the common case, and the renderer's fast path.
pub const AO_LIT: u8 = 0xFF;

/// Which (a, b) corner of the merged rectangle feeds each of emit_face's
/// v0..v3, per face direction. Corner index is `a_max as usize | (b_max as
/// usize) << 1`. Baking the permutation at mesh time keeps the renderer's
/// unpack to four shifts.
///
/// Derived from the `verts` table in main::emit_face together with the w/h axis
/// mapping there: for dir 0|1 (w -> y, h -> z) a is y and b is z; for dir 2|3
/// (w -> x, h -> z) a is x and b is z; for dir 4|5 (w -> x, h -> y) a is x and
/// b is y -- which is exactly cell_to_local's mapping.
const AO_VORDER: [[u8; 4]; 6] = [
    [1, 3, 0, 2], // +X: (a+,b-) (a+,b+) (a-,b-) (a-,b+)
    [3, 1, 2, 0], // -X
    [0, 1, 2, 3], // +Y
    [2, 3, 0, 1], // -Y
    [3, 2, 1, 0], // +Z
    [2, 3, 0, 1], // -Z
];

/// Vertex AO for one greedy-merged face rectangle.
///
/// `ob` is the MESH_SCRATCH index of cell (a=0, b=0) in the block plane JUST
/// OUTSIDE the face; `sa`/`sb` are that plane's a/b strides. For each of the
/// four corners of the MERGED rectangle we read the two edge-adjacent
/// neighbours and the diagonal one in that outside plane, then apply the
/// classic rule: two solid sides fully occlude the corner regardless of the
/// diagonal.
///
/// Cells outside the sampled plane's a/b range count as NON-occluding. Along
/// the y axis that is exact (the mesher clips a_dim/b_dim to MESH_YHI and
/// everything above it is air); along x/z it is a deliberate chunk-border
/// approximation, since reaching into the neighbour needs the rem_euclid
/// `get()` path on every corner. The seam it leaves is one vertex wide on a
/// chunk edge and reads as a very slight brightening.
#[inline]
fn face_ao(
    ob: usize,
    sa: usize,
    sb: usize,
    a_dim: usize,
    b_dim: usize,
    dir: usize,
    a0: usize,
    b0: usize,
    w: usize,
    h: usize,
) -> u8 {
    let occ = |a: i32, b: i32| -> u32 {
        if a < 0 || b < 0 || a as usize >= a_dim || b as usize >= b_dim {
            return 0;
        }
        let blk = unsafe { MESH_SCRATCH[ob + a as usize * sa + b as usize * sb] };
        // Same occlusion test the mesher uses: anything a face can be seen
        // THROUGH (air, water, plants, open doors) does not occlude.
        (unsafe { BCLASS[blk as usize] } & CLS_SEE == 0) as u32
    };
    let mut corner = [3u32; 4];
    let mut c = 0;
    while c < 4 {
        // Outward step and the rectangle-edge cell for this corner.
        let (ca, da) = if c & 1 != 0 {
            ((a0 + w - 1) as i32, 1)
        } else {
            (a0 as i32, -1)
        };
        let (cb, db) = if c & 2 != 0 {
            ((b0 + h - 1) as i32, 1)
        } else {
            (b0 as i32, -1)
        };
        let s1 = occ(ca + da, cb);
        let s2 = occ(ca, cb + db);
        corner[c] = if s1 != 0 && s2 != 0 {
            0
        } else {
            3 - (s1 + s2 + occ(ca + da, cb + db))
        };
        c += 1;
    }
    let o = &AO_VORDER[dir];
    (corner[o[0] as usize]
        | (corner[o[1] as usize] << 2)
        | (corner[o[2] as usize] << 4)
        | (corner[o[3] as usize] << 6)) as u8
}

/// Map a plane cell (a, b) to local block coords for the plane's direction.
#[inline]
fn cell_to_local(dir: usize, plane: usize, a: usize, b: usize) -> (usize, usize, usize) {
    match dir {
        0 | 1 => (plane, a, b), // x-plane: lx=plane, ly=a, lz=b
        2 | 3 => (a, plane, b), // y-plane: ly=plane, lx=a, lz=b
        _ => (a, b, plane),     // z-plane: lz=plane, lx=a, ly=b
    }
}

/// 2D greedy merge of one `dir`-plane into maximal rectangles (capped MAX_MERGE
/// per axis), appended to the MESH_FACES scratch. The packed (w,h) is
/// (a-extent, b-extent); cell_to_local + emit_face agree on the axis mapping.
fn greedy_plane(
    s: usize,
    dir: usize,
    plane: usize,
    a_dim: usize,
    b_dim: usize,
    mut n: usize,
) -> usize {
    // Record where this plane's faces start (both mesh paths call planes in
    // ascending order, so this single hook keeps the ranges sorted).
    let bound_i = PL_OFF[dir] + plane;
    unsafe {
        MESH_PLANE_START[bound_i] = n as u16;
        MESH_PLANE_BOUNDS[bound_i] = 0;
    }
    // dir 3 is -Y and plane 0 is the world floor: it can never show a face, and
    // skipping it outright saves a whole 16x16 mask build per chunk.
    if dir == 3 && plane == 0 {
        return n;
    }
    // Most planes are entirely interior or entirely open and carry no faces at
    // all; the merge would otherwise scan every cell of them looking for a seed.
    if !build_mask(s, dir, plane, a_dim, b_dim) {
        return n;
    }
    // AO samples the block plane one step along the face normal. Resolve that
    // plane's MESH_SCRATCH base + a/b strides once here; if it falls outside the
    // chunk (a face on the outer shell) every corner is fully lit.
    //   dir 0|1 x-planes: a=ly (stride 256) b=lz (16), plane=lx (1)
    //   dir 2|3 y-planes: a=lx (1)          b=lz (16), plane=ly (256)
    //   dir 4|5 z-planes: a=lx (1)          b=ly (256), plane=lz (16)
    let (sa, sb, pstride, pdim) = match dir {
        0 | 1 => (CWU * CWU, CWU, 1, CWU),
        2 | 3 => (1, CWU, CWU * CWU, unsafe { MESH_YHI } + 1),
        _ => (1, CWU * CWU, CWU, CWU),
    };
    let po = plane as i32 + if dir & 1 == 0 { 1 } else { -1 };
    let ob = if po >= 0 && (po as usize) < pdim {
        (po as usize) * pstride
    } else {
        usize::MAX
    };
    let mut amin = 127usize;
    let mut amax = 0usize;
    let mut bmin = 127usize;
    let mut bmax = 0usize;
    let _ = unsafe { FMASK_ROWS };
    let mut b0 = 0;
    while b0 < b_dim {
        // Seeds come from the row's cell bitmap: the next set bit is the next
        // unconsumed face cell, so empty cells are never read.
        let mut row_bits = unsafe { FMASK_BITS[b0] };
        while row_bits != 0 {
            let a0 = row_bits.trailing_zeros() as usize;
            let cell = unsafe { FMASK[b0 * a_dim + a0] };
            if cell == 0 {
                // Cannot happen (bits and cells are cleared together); stay
                // safe rather than spin.
                row_bits &= row_bits - 1;
                continue;
            }
            let cap = unsafe { MESH_CAP };
            let mut w = 1;
            while w < cap && a0 + w < a_dim && unsafe { FMASK[b0 * a_dim + a0 + w] } == cell {
                w += 1;
            }
            let mut h = 1;
            let cap_h = if cap < MAX_MERGE_H { cap } else { MAX_MERGE_H };
            'grow: while h < cap_h && b0 + h < b_dim {
                let mut aa = 0;
                while aa < w {
                    if unsafe { FMASK[(b0 + h) * a_dim + a0 + aa] } != cell {
                        break 'grow;
                    }
                    aa += 1;
                }
                h += 1;
            }
            let span = (((1u64 << (w as u32)) - 1) << (a0 as u32)) as u64;
            let mut bb = 0;
            while bb < h {
                let mut aa = 0;
                while aa < w {
                    unsafe { FMASK[(b0 + bb) * a_dim + a0 + aa] = 0 };
                    aa += 1;
                }
                unsafe { FMASK_BITS[b0 + bb] &= !span };
                bb += 1;
            }
            row_bits &= !span;
            let ao = if ob == usize::MAX {
                AO_LIT
            } else {
                face_ao(ob, sa, sb, a_dim, b_dim, dir, a0, b0, w, h)
            };
            let (lx, ly, lz) = cell_to_local(dir, plane, a0, b0);
            n = push_face(n, lx, ly, lz, dir, (cell & 0x7F) as u8, w, h, cell >> 7, ao);
            amin = amin.min(a0);
            amax = amax.max(a0 + w);
            bmin = bmin.min(b0);
            bmax = bmax.max(b0 + h);
        }
        b0 += 1;
    }
    unsafe {
        MESH_PLANE_BOUNDS[bound_i] =
            amin as u32 | ((amax as u32) << 7) | ((bmin as u32) << 14) | ((bmax as u32) << 21);
    }
    n
}

/// (plane a-dim, plane b-dim, number of planes) for a face direction. A plane
/// covers a_dim*b_dim cells; greedy_plane meshes one plane.
#[inline]
fn dir_dims(dir: usize) -> (usize, usize, usize) {
    // The y extent is clipped to the chunk's highest non-air layer: terrain tops
    // out well below CHU=64, and scanning the empty air above it was pure cost in
    // BOTH the mask build and the greedy merge. Whichever of a/b/plane maps to ly
    // for this direction is the one that shrinks.
    let hy = unsafe { MESH_YHI } + 1;
    match dir {
        0 | 1 => (hy, CWU, CWU),  // x-planes: a=ly b=lz(16), 16 of them (lx)
        2 | 3 => (CWU, CWU, hy),  // y-planes: a=lx(16) b=lz(16), hy of them (ly)
        _ => (CWU, hy, CWU),      // z-planes: a=lx(16) b=ly, 16 of them (lz)
    }
}

/// Fill the plane-start table for the planes a clipped `dir` never visits, so
/// the renderer's `POOL_PLANE_START[off + lo ..= off + hi + 1]` slicing stays
/// valid (they are all empty ranges at `n`).
fn pad_plane_starts(dir: usize, from: usize, n: usize) {
    // A completed-direction sentinel has no planes to pad. Keep this guard as a
    // last line of defence against corrupting or indexing past the range table.
    if dir >= PL_N.len() {
        #[cfg(feature = "emulator-telemetry")]
        crate::telemetry::debug_log("voxide: invalid mesh direction");
        return;
    }
    let mut p = from;
    let end = PL_N[dir];
    while p <= end {
        unsafe {
            MESH_PLANE_START[PL_OFF[dir] + p] = n as u16;
            if p < end {
                MESH_PLANE_BOUNDS[PL_OFF[dir] + p] = 0;
            }
        }
        p += 1;
    }
}

/// Derive bounds from the completed face ranges themselves. This is the
/// authoritative pass: incremental mesh cursors can leave a zero cache entry
/// for a non-empty copied/padded range, and treating that as empty caused whole
/// strips of title/game terrain to disappear.
fn rebuild_plane_bounds() {
    let mut dir = 0usize;
    while dir < 6 {
        let off = PL_OFF[dir];
        let mut plane = 0usize;
        while plane < PL_N[dir] {
            let start = unsafe { MESH_PLANE_START[off + plane] } as usize;
            let end = unsafe { MESH_PLANE_START[off + plane + 1] } as usize;
            if start == end {
                unsafe { MESH_PLANE_BOUNDS[off + plane] = 0 };
                plane += 1;
                continue;
            }
            let mut amin = 127usize;
            let mut amax = 0usize;
            let mut bmin = 127usize;
            let mut bmax = 0usize;
            let mut k = start;
            while k < end {
                let f = unsafe { MESH_FACES[k] };
                let lx = (f & 15) as usize;
                let ly = ((f >> 4) & 63) as usize;
                let lz = ((f >> 10) & 15) as usize;
                let w = ((f >> 24) & 15) as usize + 1;
                let h = ((f >> 28) & 7) as usize + 1;
                let (a, b) = match dir {
                    0 | 1 => (ly, lz),
                    2 | 3 => (lx, lz),
                    _ => (lx, ly),
                };
                amin = amin.min(a);
                amax = amax.max(a + w);
                bmin = bmin.min(b);
                bmax = bmax.max(b + h);
                k += 1;
            }
            unsafe {
                MESH_PLANE_BOUNDS[off + plane] = amin as u32
                    | ((amax as u32) << 7)
                    | ((bmin as u32) << 14)
                    | ((bmax as u32) << 21);
            }
            plane += 1;
        }
        dir += 1;
    }
}

/// 2D greedy-mesh ONE face direction of chunk `s` into the scratch, from index n.
fn mesh_dir(s: usize, dir: usize, mut n: usize) -> usize {
    let (a_dim, b_dim, num_planes) = dir_dims(dir);
    if dir < 2 {
        // x-planes: a=ly, b=lz (CWU)
        let mut lx = 0;
        while lx < num_planes {
            n = greedy_plane(s, dir, lx, a_dim, b_dim, n);
            lx += 1;
        }
    } else if dir < 4 {
        // y-planes: a=lx (CWU), b=lz (CWU)
        let mut ly = 0;
        while ly < num_planes {
            n = greedy_plane(s, dir, ly, a_dim, b_dim, n);
            ly += 1;
        }
    } else {
        // z-planes: a=lx (CWU), b=ly
        let mut lz = 0;
        while lz < num_planes {
            n = greedy_plane(s, dir, lz, a_dim, b_dim, n);
            lz += 1;
        }
    }
    n
}

/// Copy the finished scratch mesh into chunk `s`'s pool slot (allocating one).
#[inline(never)]
fn commit_mesh_inner(s: usize, n: usize, scan_plants: bool) {
    rebuild_plane_bounds();
    unsafe {
        let slot = alloc_slot(s);
        if slot == NO_SLOT {
            // No pool room (POOL >= the render window, so this shouldn't happen).
            // Re-dirty before bailing: stream_tick clears `dirty` when it STARTS a
            // mesh, so dropping out here silently would leave the chunk loaded,
            // in range, unmeshed and unqueued -- invisible forever, with collision
            // still working off its blocks.
            CHUNKS[s].dirty = true;
            return;
        }
        let p = slot as usize;
        let mut i = 0;
        while i < n {
            POOL_FACES[p][i] = MESH_FACES[i];
            POOL_AO[p][i] = MESH_AO[i];
            i += 1;
        }
        let mut d = 0;
        while d < 7 {
            POOL_DIR_START[p][d] = MESH_DIR_START[d];
            d += 1;
        }
        let mut q = 0;
        while q < PL_TOTAL {
            POOL_PLANE_START[p][q] = MESH_PLANE_START[q];
            POOL_PLANE_BOUNDS[p][q] = MESH_PLANE_BOUNDS[q];
            q += 1;
        }
        POOL_NFACE[p] = n as u16;
        if scan_plants {
            // Record cross-sprite plant cells from the decoded scratch. Ordinary
            // cube edits leave this list untouched; only plant/small-block edits
            // need the full scan.
            let mut np = 0usize;
            let mut ly = 0usize;
            while ly < CHU && np < MAX_PLANTS {
                let mut lz = 0usize;
                while lz < CWU && np < MAX_PLANTS {
                    let mut lx = 0usize;
                    while lx < CWU && np < MAX_PLANTS {
                        let b = MESH_SCRATCH[lidx(lx, ly, lz)];
                        if is_cross_plant(b) || is_small_block(b) {
                            POOL_PLANTS[p][np] = (lx as u32)
                                | ((ly as u32) << 4)
                                | ((lz as u32) << 10)
                                | ((b as u32) << 14);
                            np += 1;
                        }
                        lx += 1;
                    }
                    lz += 1;
                }
                ly += 1;
            }
            POOL_NPLANT[p] = np as u16;
        }
    }
}

fn commit_mesh(s: usize, n: usize) {
    commit_mesh_inner(s, n, true);
}

fn begin_stream_commit(s: usize) -> bool {
    rebuild_plane_bounds();
    let slot = reserve_stream_slot(s);
    if slot == NO_SLOT {
        unsafe { CHUNKS[s].dirty = true };
        return false;
    }
    unsafe {
        MESH_COMMIT_SLOT = slot;
        MESH_COMMIT_I = 0;
        MESH_PREP = 3;
    }
    true
}

fn stream_commit_tick(s: usize, n: usize) {
    unsafe {
        if MESH_COMMIT_SLOT == NO_SLOT
            || POOL_OWNER[MESH_COMMIT_SLOT as usize] != s
            || !in_render_range(s)
        {
            CHUNKS[s].dirty = in_render_range(s);
            abort_stream_mesh();
            return;
        }
        let p = MESH_COMMIT_SLOT as usize;
        let end = (MESH_COMMIT_I + STREAM_COMMIT_BATCH).min(n);
        let mut i = MESH_COMMIT_I;
        while i < end {
            POOL_FACES[p][i] = MESH_FACES[i];
            POOL_AO[p][i] = MESH_AO[i];
            i += 1;
        }
        MESH_COMMIT_I = end;
        if end < n {
            return;
        }

        let mut d = 0usize;
        while d < 7 {
            POOL_DIR_START[p][d] = MESH_DIR_START[d];
            d += 1;
        }
        let mut q = 0usize;
        while q < PL_TOTAL {
            POOL_PLANE_START[p][q] = MESH_PLANE_START[q];
            POOL_PLANE_BOUNDS[p][q] = MESH_PLANE_BOUNDS[q];
            q += 1;
        }
        POOL_NFACE[p] = n as u16;
        let mut np = 0usize;
        while np < MESH_NPLANT {
            POOL_PLANTS[p][np] = MESH_PLANTS[np];
            np += 1;
        }
        POOL_NPLANT[p] = MESH_NPLANT as u16;

        let old = CHUNKS[s].face_slot;
        CHUNKS[s].face_slot = MESH_COMMIT_SLOT;
        POOL_OWNER[p] = s;
        if old != NO_SLOT && old != MESH_COMMIT_SLOT {
            POOL_OWNER[old as usize] = usize::MAX;
        }
        MESH_COMMIT_SLOT = NO_SLOT;
        MESH_S = usize::MAX;
    }
}

/// Synchronous full remesh of chunk `s` (edits + boot). Streaming uses the
/// amortized per-direction path in `stream_tick`. Aborts any in-flight amortized
/// mesh first (it shares the scratch) and re-queues that chunk.
fn mesh_chunk(s: usize) {
    if unsafe { MESH_S != usize::MAX } {
        abort_stream_mesh();
    }
    set_mesh_lod(s);
    decode_to_mesh_scratch(s);
    build_sky_top(); // skylight depends on the whole column, so it precedes meshing
    let mut n = 0usize;
    let mut dir = 0;
    while dir < 6 {
        unsafe { MESH_DIR_START[dir] = n as u16 };
        n = mesh_dir(s, dir, n);
        pad_plane_starts(dir, dir_dims(dir).2, n);
        dir += 1;
    }
    unsafe {
        MESH_DIR_START[6] = n as u16;
    }
    commit_mesh(s, n);
    unsafe { CHUNKS[s].dirty = false };
}

/// Queue a fast edit remesh. The block/collision state changes immediately,
/// while `mesh_edit_tick` builds the replacement mesh over several frames.
// Player edits waiting behind the one being meshed: a short FIFO so rapid
// mining gets one fast partial rebuild per block. Each queued edit starts
// only after the previous commit, so its plane-local copy always reads a
// committed mesh that already contains the earlier edits -- the correctness
// problem that used to force a slow full-rebuild fallback. Overflow (five
// edits inside ~2 frames) still falls back.
const EDIT_QN: usize = 4;
static mut EDIT_Q: [(usize, usize, usize, usize, u8, u8, bool); EDIT_QN] =
    [(0, 0, 0, 0, 0, 0, false); EDIT_QN];
static mut EDIT_Q_LEN: usize = 0;

/// True while any player-edit mesh work is outstanding; the main loop runs
/// burst streaming slices until it drains, so an edit never idles at the
/// one-slice tier with a stale mesh on screen.
pub fn edit_backlog() -> bool {
    unsafe { EDIT_MESH_S != usize::MAX || EDIT_Q_LEN > 0 }
}

#[inline(never)]
fn queue_mesh_edit(s: usize, lx: usize, ly: usize, lz: usize, old: u8, new: u8, sky_changed: bool) {
    unsafe {
        // The edit owns the shared scratch until its atomic commit. Resume any
        // interrupted streaming chunk from its dirty flag afterwards.
        if MESH_S != usize::MAX {
            abort_stream_mesh();
        }
        if EDIT_MESH_S != usize::MAX {
            // One partial is in flight: queue this edit behind it.
            if EDIT_Q_LEN < EDIT_QN {
                EDIT_Q[EDIT_Q_LEN] = (s, lx, ly, lz, old, new, sky_changed);
                EDIT_Q_LEN += 1;
            } else {
                // Queue full: the old fallback, one amortized full rebuild.
                CHUNKS[s].dirty = true;
            }
            return;
        }
        begin_mesh_edit(s, lx, ly, lz, old, new, sky_changed);
    }
}

#[inline(never)]
fn begin_mesh_edit(s: usize, lx: usize, ly: usize, lz: usize, old: u8, new: u8, sky_changed: bool) {
    unsafe {
        if CHUNKS[s].face_slot == NO_SLOT {
            CHUNKS[s].dirty = true;
            EDIT_MESH_S = usize::MAX;
            return;
        }
        // Snapshot a full rebuild already owed to this chunk (an earlier
        // rapid-edit fallback, an aborted stream mesh). The partial rebuild
        // below copies unaffected plane ranges from the committed pool, which
        // predates that owed work -- so its commit must put the dirty flag
        // BACK rather than clear it, or those edits stay invisible until some
        // unrelated event re-dirties the chunk.
        EDIT_PENDING_FULL = CHUNKS[s].dirty;
        EDIT_SKY_CHANGED = sky_changed;
        EDIT_MESH_S = s;
        EDIT_MESH_LX = lx;
        EDIT_MESH_LY = ly;
        EDIT_MESH_LZ = lz;
        EDIT_MESH_PHASE = 0;
        EDIT_MESH_DECODE = 0;
        EDIT_MESH_TOP = 0;
        EDIT_LIGHT_CHANGED |= is_light_source(old) || is_light_source(new);
        EDIT_PLANTS_CHANGED |= is_cross_plant(old)
            || is_small_block(old)
            || is_cross_plant(new)
            || is_small_block(new);
    }
}

/// Advance a queued player edit by one bounded step. Greedy merging never
/// crosses a plane, so only the planes touching the edited cell can change
/// topology or AO. The other plane ranges are copied from the committed mesh.
#[inline(never)]
fn mesh_edit_tick() {
    unsafe {
        let s = EDIT_MESH_S;
        if s == usize::MAX {
            return;
        }

        if EDIT_MESH_PHASE == 0 {
            if MESH_SCRATCH_OWNER != s {
                let src = &CHUNKS[s].blocks;
                let end = (EDIT_MESH_DECODE + EDIT_DECODE_BATCH).min(CHUNK_VOL);
                if EDIT_MESH_DECODE == 0 {
                    col_masks_reset();
                }
                let mut top = EDIT_MESH_TOP;
                decode_blocks::<false>(src, EDIT_MESH_DECODE, end, &mut top);
                EDIT_MESH_TOP = top;
                EDIT_MESH_DECODE = end;
                if end < CHUNK_VOL {
                    return;
                }
                MESH_YHI = EDIT_MESH_TOP / (CWU * CWU);
                MESH_SCRATCH_OWNER = s;
            }
            EDIT_MESH_PHASE = 1;
        }

        if EDIT_MESH_PHASE == 1 {
            set_mesh_lod(s);
            // If phase 0 re-decoded the scratch (another chunk owned it),
            // SKY_TOP and TORCH_LIT still describe THAT chunk -- shading the
            // rebuilt planes from a stranger's skylight heights is what made
            // whole sections go dark on an ordinary block edit. Rebuild them
            // for this chunk. Only when the scratch was already ours (and no
            // light source moved) is the single-column refresh enough.
            if EDIT_MESH_DECODE != 0 || EDIT_LIGHT_CHANGED {
                build_sky_top();
            } else {
                build_sky_column(EDIT_MESH_LX, EDIT_MESH_LZ);
            }
            EDIT_MESH_POOL = CHUNKS[s].face_slot as usize;
            EDIT_MESH_DIR = 0;
            EDIT_MESH_N = 0;
            EDIT_MESH_PHASE = 2;
            return;
        }

        if EDIT_MESH_PHASE == 2 {
            let dir = EDIT_MESH_DIR;
            let coords = [
                EDIT_MESH_LX,
                EDIT_MESH_LX,
                EDIT_MESH_LY,
                EDIT_MESH_LY,
                EDIT_MESH_LZ,
                EDIT_MESH_LZ,
            ];
            let mut n = EDIT_MESH_N;
            MESH_DIR_START[dir] = n as u16;
            let (a_dim, b_dim, num_planes) = dir_dims(dir);
            let off = PL_OFF[dir];
            let mut plane = 0usize;
            while plane < PL_N[dir] {
                MESH_PLANE_START[off + plane] = n as u16;
                MESH_PLANE_BOUNDS[off + plane] = 0;
                if plane < num_planes {
                    if plane.abs_diff(coords[dir]) <= 1 {
                        n = greedy_plane(s, dir, plane, a_dim, b_dim, n);
                    } else {
                        MESH_PLANE_BOUNDS[off + plane] =
                            POOL_PLANE_BOUNDS[EDIT_MESH_POOL][off + plane];
                        let old_start = POOL_PLANE_START[EDIT_MESH_POOL][off + plane] as usize;
                        let old_end = POOL_PLANE_START[EDIT_MESH_POOL][off + plane + 1] as usize;
                        let mut i = old_start;
                        while i < old_end && n < MAX_FACES {
                            MESH_FACES[n] = POOL_FACES[EDIT_MESH_POOL][i];
                            MESH_AO[n] = POOL_AO[EDIT_MESH_POOL][i];
                            n += 1;
                            i += 1;
                        }
                    }
                }
                plane += 1;
            }
            MESH_PLANE_START[off + PL_N[dir]] = n as u16;
            EDIT_MESH_N = n;
            EDIT_MESH_DIR += 1;
            if EDIT_MESH_DIR >= 6 {
                MESH_DIR_START[6] = n as u16;
                EDIT_MESH_PHASE = 3;
            }
            return;
        }

        commit_mesh_inner(s, EDIT_MESH_N, EDIT_PLANTS_CHANGED);
        // Leave the chunk dirty -- queueing an amortized full remesh behind
        // this fast partial one -- when (a) a full rebuild was already owed
        // (the pool ranges we copied predate it), (b) the skylight column
        // moved (light changed below the edit, outside the rebuilt planes), or
        // (c) a light source changed (its radius spans many planes). The
        // partial gives instant feedback; the follow-up makes light exact.
        CHUNKS[s].dirty = EDIT_PENDING_FULL || EDIT_SKY_CHANGED || EDIT_LIGHT_CHANGED;
        EDIT_MESH_S = usize::MAX;
        EDIT_LIGHT_CHANGED = false;
        EDIT_PLANTS_CHANGED = false;
        EDIT_PENDING_FULL = false;
        EDIT_SKY_CHANGED = false;
        // Chain the next queued edit; its plane copy reads the mesh this
        // commit just published.
        if EDIT_Q_LEN > 0 {
            let (qs, qlx, qly, qlz, qold, qnew, qsky) = EDIT_Q[0];
            let mut i = 1;
            while i < EDIT_Q_LEN {
                EDIT_Q[i - 1] = EDIT_Q[i];
                i += 1;
            }
            EDIT_Q_LEN -= 1;
            begin_mesh_edit(qs, qlx, qly, qlz, qold, qnew, qsky);
        }
    }
}

// --- streaming ---

/// Generate the initial grid centred on the chunk containing (wx, wz), then
/// mesh everything (neighbours are loaded so border faces are correct).
/// Generate + mesh the boot ring around (wx, wz). `progress(done, total)` is
/// called after every generated/meshed chunk so the caller can draw a loading
/// bar (boot was ~10 seconds of black screen without one).
/// Boot prep for the menu-driven world load: block classes + centre the
/// streaming ring on the spawn. There is NO synchronous boot gen pass any
/// more -- the main menu pumps the ordinary recenter/gen_tick/stream_tick
/// machinery every frame, so the world assembles behind the menu instead of
/// behind a blocking loading bar.
#[inline(never)]
pub fn boot_prepare(wx: i32, wz: i32) {
    init_block_class();
    unsafe {
        // Stamp the non-zero defaults the statics above gave up so their
        // initializers could be all-zero (.bss) -- see EMPTY_CHUNK/POOL_AO.
        let mut i = 0;
        while i < CHUNKS.len() {
            CHUNKS[i].face_slot = NO_SLOT;
            i += 1;
        }
        let mut p = 0;
        while p < POOL {
            let mut f = 0;
            while f < MAX_FACES {
                POOL_AO[p][f] = AO_LIT as u16;
                f += 1;
            }
            p += 1;
        }
        let mut f = 0;
        while f < MAX_FACES {
            MESH_AO[f] = AO_LIT as u16;
            f += 1;
        }
        PLAYER_CX = floor_div(wx, CW);
        PLAYER_CZ = floor_div(wz, CW);
    }
}


/// Clear the 5x5x6 spawn pocket so the player never starts inside a tree.
/// Called by the menu the moment the spawn chunk publishes; raw writes, then
/// re-dirty the touched chunks (aborting any in-flight mesh of them) so the
/// pocket is in their first visible mesh.
#[inline(never)]
pub fn carve_spawn_pocket(wx: i32, wz: i32) {
    let sy = surface_y(wx, wz);
    let mut dz = -2;
    while dz <= 2 {
        let mut dx = -2;
        while dx <= 2 {
            let mut y = sy;
            while y < sy + 6 {
                raw_set(wx + dx, y, wz + dz, AIR);
                y += 1;
            }
            dx += 1;
        }
        dz += 1;
    }
    unsafe {
        if MESH_S != usize::MAX {
            abort_stream_mesh(); // re-dirties whatever was building; cheap to redo
        }
    }
    let mut dz = -2;
    while dz <= 2 {
        let mut dx = -2;
        while dx <= 2 {
            set_dirty(floor_div(wx + dx, CW), floor_div(wz + dz, CW));
            dx += 4;
        }
        dz += 4;
    }
}

/// Ensure the grid is centred on the player's chunk; (re)generate + mesh any
/// slot now holding a different chunk. Cheap when the player hasn't crossed a
/// boundary (every slot already matches its target).
/// Release pool slots whose owner chunk has left render range (re-meshes if it
/// returns). This is what keeps face RAM at POOL slots, not GRID^2.
fn free_far_slots() {
    unsafe {
        let mut p = 0;
        while p < POOL {
            let o = POOL_OWNER[p];
            if o != usize::MAX {
                if !in_render_range(o) {
                    CHUNKS[o].face_slot = NO_SLOT;
                    CHUNKS[o].dirty = true; // re-mesh on return to range
                    POOL_OWNER[p] = usize::MAX;
                } else if chunk_lod(o) != CHUNKS[o].face_lod {
                    CHUNKS[o].dirty = true; // crossed the near/far band -> re-mesh at new LOD
                }
            }
            p += 1;
        }
    }
}

/// COMPILER BARRIER: every public entry point of the streaming state
/// machines is #[inline(never)]. Twice now, fat LTO has inlined one of these
/// into a caller's hot loop and cached the static-mut state in registers
/// across iterations -- the menu pump froze the gen state machine on
/// 2026-07-30, and the shipped build packed chunks from a garbage GEN_COL
/// (an out-of-bounds panic that froze the console seconds after boot). A
/// real call boundary forces the state to memory around every entry.
#[inline(never)]
pub fn recenter(wx: i32, wz: i32) {
    let pcx = floor_div(wx, CW);
    let pcz = floor_div(wz, CW);
    unsafe {
        PLAYER_CX = pcx;
        PLAYER_CZ = pcz;
    }
    free_far_slots(); // give back slots of chunks now out of view
    // Start the amortized gen of at most ONE missing chunk (gen_tick fills it over
    // ~8 frames, then it meshes; all beyond the far plane, so it loads before it's
    // seen). Only one gen runs at a time. Cheap when nothing is missing.
    unsafe {
        if GEN_S != usize::MAX {
            return; // a gen is already in flight; finish it first
        }
    }
    // Pick the NEAREST missing chunk so the chunks the player is about to see are
    // generated before the ones at the far edge of the ring.
    let half = GRID / 2;
    let mut best_d = i32::MAX;
    let (mut best_cx, mut best_cz) = (0, 0);
    let mut j = -half;
    while j <= half {
        let mut i = -half;
        while i <= half {
            let cx = pcx + i;
            let cz = pcz + j;
            let s = slot(cx, cz);
            let ok = unsafe { CHUNKS[s].loaded && CHUNKS[s].cx == cx && CHUNKS[s].cz == cz };
            if !ok {
                let d = i.abs().max(j.abs());
                if d < best_d {
                    best_d = d;
                    best_cx = cx;
                    best_cz = cz;
                }
            }
            i += 1;
        }
        j += 1;
    }
    if best_d != i32::MAX {
        // Start amortized gen of the nearest missing chunk (gen_tick fills it a
        // column-batch per frame). With RENDER_R one ring wider than the draw
        // distance, this chunk is beyond the far plane and finishes (gen + mesh)
        // before the player can see it -- smooth AND complete.
        let s = slot(best_cx, best_cz);
        unsafe {
            if MESH_S == s {
                abort_stream_mesh();
            }
            if MESH_SCRATCH_OWNER == s {
                MESH_SCRATCH_OWNER = usize::MAX;
            }
            if CHUNKS[s].face_slot != NO_SLOT {
                POOL_OWNER[CHUNKS[s].face_slot as usize] = usize::MAX;
                CHUNKS[s].face_slot = NO_SLOT;
            }
            CHUNKS[s].loaded = false; // air / unrendered until gen completes
            CHUNKS[s].cx = best_cx;
            CHUNKS[s].cz = best_cz;
            GEN_S = s;
            GEN_CX = best_cx;
            GEN_CZ = best_cz;
            GEN_COL = 0;
            GEN_PHASE = 0;
        }
    }
}

/// Recentre on (bx, bz) and finish the chunk under it before returning.
///
/// `recenter` only STARTS one amortized gen, so anything that TELEPORTS the
/// player -- respawning at a bed on the far side of the ring -- returns with
/// the destination column still ungenerated. Everything that then reads the
/// terrain there gets AIR: `surface_y` falls back to the noise height, gravity
/// finds no floor, and the chunk finally generates around a player who has
/// been falling for a second. Blocking here costs the frame a normal chunk gen
/// (~38 ticks) and only ever happens on a teleport.
#[inline(never)]
pub fn ensure_loaded(bx: i32, bz: i32) {
    // Generous: a chunk is ~38 gen ticks, and the centre chunk is the nearest
    // missing one so recenter always picks it first.
    let mut guard = 0;
    while !column_loaded(bx, bz) && guard < 4096 {
        recenter(bx, bz);
        gen_tick();
        guard += 1;
    }
}

/// Advance the in-flight chunk gen by GEN_BATCH columns; finish + publish it when
/// all columns are done. Paired with recenter, this streams terrain in without
/// ever doing a whole chunk's gen in a single frame.
#[inline(never)]
pub fn gen_tick() {
    unsafe {
        if GEN_S == usize::MAX {
            return;
        }
        let s = GEN_S;
        // Three phases, each resumable, so a chunk never lands as one lump in a
        // single frame. Publishing used to do the decoration pass AND pack all
        // 16K blocks in one go, measured at 2 vblanks -- a visible share of the
        // hitch when a chunk completed while you were walking.
        match GEN_PHASE {
            0 => {
                GEN_COL = gen_columns(GEN_CX * CW, GEN_CZ * CW, GEN_COL, GEN_BATCH);
                if GEN_COL >= CHUNK_AREA {
                    scatter_ores(&mut GEN_SCRATCH, GEN_CX, GEN_CZ);
                    GEN_PHASE = 1;
                    GEN_COL = 0;
                }
            }
            1 => {
                GEN_COL = decorate_columns(GEN_CX, GEN_CZ, GEN_COL, DECORATE_BATCH);
                if GEN_COL >= CHUNK_AREA {
                    GEN_PHASE = 2;
                    GEN_COL = 0;
                }
            }
            _ => {
                // Multiple of 4 blocks: see pack_blocks.
                GEN_COL = pack_blocks(s, GEN_COL, PACK_BATCH);
                if GEN_COL >= CHUNK_VOL {
                    publish_chunk(s, GEN_CX, GEN_CZ);
                    GEN_S = usize::MAX;
                    GEN_PHASE = 0;
                    GEN_COL = 0;
                }
            }
        }
    }
}

/// Amortized streaming mesh: advance the in-flight chunk's mesh by a cell budget
/// per call, or claim the next dirty in-RANGE chunk. RENDER_R covers a ring 1
/// chunk WIDER than the draw distance, so a chunk meshes while it's still beyond
/// the far plane (lead time) -- the player never sees it half-built, and the cost
/// spreads over frames instead of freezing on a boundary cross.
/// `allow_claim` gates the costlier decode/light/commit stages to the first
/// slice of a frame. Plane meshing may use a second slice when the renderer's
/// measured quad load leaves room.
#[inline(never)]
pub fn stream_tick_claim(allow_claim: bool) {
    unsafe { ALLOW_CLAIM = allow_claim };
    stream_tick();
}

/// Pending streaming work, split by urgency: `near` is work within one chunk of
/// the player (inside the ~21-block fog radius, so its absence is visible pop-in)
/// plus any in-flight player-edit remesh; `far` is the rest of the ring. Main
/// scales its per-frame slice count by these, so the machine spends peak budget
/// only when missing terrain could actually be seen, fills the outer ring at a
/// cheaper tier, and idles when caught up.
#[inline(never)]
pub fn stream_backlog() -> (u32, u32) {
    let mut near = 0u32;
    let mut far = 0u32;
    unsafe {
        // An in-flight edit rebuild rides the CHEAP tier: its chunk still
        // displays the pre-edit mesh, so nothing is visibly missing while the
        // partial completes -- at three slices that is ~4 frames from break to
        // visible hole, hidden behind the break animation. Charging the
        // 6-slice tier here was what pushed mining frames over the 30fps
        // quantum.
        if EDIT_MESH_S != usize::MAX {
            far += 1;
        }
        let mesh_s = MESH_S;
        let half = GRID / 2;
        let mut j = -half;
        while j <= half {
            let mut i = -half;
            while i <= half {
                let cx = PLAYER_CX + i;
                let cz = PLAYER_CZ + j;
                let s = slot(cx, cz);
                let resident = CHUNKS[s].loaded && CHUNKS[s].cx == cx && CHUNKS[s].cz == cz;
                // A re-mesh in flight (MESH_S) keeps its old face_slot visible
                // and clears dirty at claim; count it so it cannot stall at the
                // idle tier mid-build.
                let pending = !resident
                    || mesh_s == s
                    || (in_render_range(s)
                        && (CHUNKS[s].dirty || CHUNKS[s].face_slot == NO_SLOT));
                if pending {
                    // URGENT means a hole the player could see: no mesh at all
                    // within one chunk. A re-dirty that still displays its old
                    // mesh (an edit's follow-up rebuild, an LOD flip) shows
                    // nothing wrong on screen while it waits, so it rides the
                    // cheap tier -- this is what keeps the 6-slice burst out of
                    // mining frames that are already paying for the edit.
                    let holed = !resident || CHUNKS[s].face_slot == NO_SLOT;
                    if holed && i.abs().max(j.abs()) <= 1 {
                        near += 1;
                    } else {
                        far += 1;
                    }
                }
                i += 1;
            }
            j += 1;
        }
    }
    (near, far)
}

static mut ALLOW_CLAIM: bool = true;

#[inline(never)]
pub fn stream_tick() {
    unsafe {
        if EDIT_MESH_S != usize::MAX {
            // Main can call stream_tick once per simulation catch-up tick.
            // Advance player-visible edit work only on the first call so a slow
            // frame cannot multiply its own mesh cost.
            if ALLOW_CLAIM {
                mesh_edit_tick();
            }
            return;
        }
        if MESH_S == usize::MAX {
            if !ALLOW_CLAIM {
                return;
            }
            // Claim the NEAREST dirty chunk, not the first in slot order: `slot`
            // is a toroidal hash of the chunk coords, so scan order has nothing
            // to do with distance and the chunk behind you could be meshed ahead
            // of the one you are walking into.
            let mut best = usize::MAX;
            let mut best_d = i32::MAX;
            let mut s = 0;
            while s < NCHUNKS {
                if CHUNKS[s].loaded && CHUNKS[s].dirty && in_render_range(s) {
                    let d = (CHUNKS[s].cx - PLAYER_CX)
                        .abs()
                        .max((CHUNKS[s].cz - PLAYER_CZ).abs());
                    if d < best_d {
                        best_d = d;
                        best = s;
                    }
                }
                s += 1;
            }
            if best == usize::MAX {
                return;
            }
            MESH_S = best;
            MESH_DIR = 0;
            MESH_PLANE = 0;
            MESH_N = 0;
            MESH_PREP = 0;
            MESH_DECODE = 0;
            MESH_DECODE_TOP = 0;
            MESH_COMMIT_SLOT = NO_SLOT;
            MESH_COMMIT_I = 0;
            MESH_NPLANT = 0;
            MESH_NLIGHT = 0;
            MESH_LIGHT_I = 0;
            SKY_TOP = [0; CWU * CWU];
            TORCH_LIT = [0; CHUNK_VOL / 32];
            MESH_DIR_START[0] = 0;
            CHUNKS[best].dirty = false;
            set_mesh_lod(best);
        }
        let s = MESH_S;
        if MESH_PREP == 0 {
            if !ALLOW_CLAIM {
                return;
            }
            let src = &CHUNKS[s].blocks;
            let end = (MESH_DECODE + STREAM_DECODE_BATCH).min(CHUNK_VOL);
            if MESH_DECODE == 0 {
                col_masks_reset();
            }
            let mut top = MESH_DECODE_TOP;
            decode_blocks::<true>(src, MESH_DECODE, end, &mut top);
            MESH_DECODE_TOP = top;
            MESH_DECODE = end;
            if end < CHUNK_VOL {
                return;
            }
            MESH_YHI = MESH_DECODE_TOP / (CWU * CWU);
            MESH_SCRATCH_OWNER = s;
            MESH_PREP = 1;
            return;
        }
        if MESH_PREP == 1 {
            if !ALLOW_CLAIM {
                return;
            }
            if MESH_LIGHT_I < MESH_NLIGHT {
                flood_light(MESH_LIGHT_SOURCES[MESH_LIGHT_I]);
                MESH_LIGHT_I += 1;
                return;
            }
            MESH_PREP = 2;
            return;
        }
        if MESH_PREP == 3 {
            if ALLOW_CLAIM {
                stream_commit_tick(s, MESH_N);
            }
            return;
        }
        // Work on local cursors and publish them once. Mutating the static
        // cursors throughout the loop let the completed-direction sentinel (6)
        // leak into the next padding call on MIPS after direction 5 finished,
        // indexing PL_N[6] and panicking the guest during traversal.
        let mut mesh_dir = MESH_DIR;
        let mut mesh_plane = MESH_PLANE;
        let mut mesh_n = MESH_N;
        let mut budget = MESH_CELL_BUDGET as i32;
        while mesh_dir < 6 && budget > 0 {
            let (a_dim, b_dim, num_planes) = dir_dims(mesh_dir);
            mesh_n = greedy_plane(s, mesh_dir, mesh_plane, a_dim, b_dim, mesh_n);
            budget -= (a_dim * b_dim) as i32;
            mesh_plane += 1;
            if mesh_plane >= num_planes {
                pad_plane_starts(mesh_dir, num_planes, mesh_n);
                mesh_dir += 1;
                mesh_plane = 0;
                if mesh_dir < 6 {
                    MESH_DIR_START[mesh_dir] = mesh_n as u16;
                }
            }
        }
        MESH_DIR = mesh_dir;
        MESH_PLANE = mesh_plane;
        MESH_N = mesh_n;
        if mesh_dir >= 6 {
            MESH_DIR_START[6] = mesh_n as u16;
            if !begin_stream_commit(s) {
                MESH_S = usize::MAX;
            }
        }
    }
}

// --- occlusion culling ---

/// Skip far chunks hidden behind nearer terrain.
///
/// NOTE this is currently DEAD: the call site gates on `chunk_lod(s) == 1`, and
/// LOD_R is 99, so chunk_lod never returns 1 and chunk_occluded never runs.
///
/// Do not "fix" that by widening the gate. Measured on the green spawn, which is
/// the heaviest scene we have: running chunk_occluded on every ring took world
/// faces from 971,303 to 1,807,009 and the frame from 18.6 fps to 12.7. It casts
/// five rays per chunk through surface_y, and at this draw distance that costs
/// far more than the chunks it manages to reject. It would need a much cheaper
/// occluder (a horizon/height-span test rather than ray casts) to pay for
/// itself.
const OCCLUSION_CULL: bool = true;

/// Solid enough to hide what's behind it. Air/water/lava/leaves see through.
#[inline]
fn occluder(b: u8) -> bool {
    b != AIR && !is_water(b) && b != LAVA && b != LEAVES
}

/// March the voxel grid from world-unit point (sx,sy,sz) toward (tx,ty,tz); true
/// if an opaque block sits between them (so the target is hidden). Skips the ends
/// (the eye's own block and the target chunk itself).
fn ray_hits_solid(sx: i32, sy: i32, sz: i32, tx: i32, ty: i32, tz: i32) -> bool {
    let dx = tx - sx;
    let dy = ty - sy;
    let dz = tz - sz;
    let span = dx.abs().max(dy.abs()).max(dz.abs());
    let steps = span / 48; // ~0.75-block sample spacing
    if steps < 3 {
        return false;
    }
    let mut i = 2;
    let lim = steps - 2;
    while i < lim {
        let x = sx + dx * i / steps;
        let y = sy + dy * i / steps;
        let z = sz + dz * i / steps;
        if occluder(get(
            floor_div(x, BLOCK),
            floor_div(y, BLOCK),
            floor_div(z, BLOCK),
        )) {
            return true;
        }
        i += 1;
    }
    false
}

/// Is chunk (cx,cz) fully hidden behind nearer terrain? Casts a ray from the eye
/// to the chunk surface at its centre + 4 corners; occluded only if EVERY ray is
/// blocked (conservative -- a single poking-out corner keeps the chunk drawn).
fn chunk_occluded(cam: &Camera, cx: i32, cz: i32) -> bool {
    let ox = cx * CW;
    let oz = cz * CW;
    let cols = [
        (ox + CW / 2, oz + CW / 2),
        (ox + 1, oz + 1),
        (ox + CW - 2, oz + 1),
        (ox + 1, oz + CW - 2),
        (ox + CW - 2, oz + CW - 2),
    ];
    let mut k = 0;
    while k < cols.len() {
        let (bx, bz) = cols[k];
        let sy = surface_y(bx, bz);
        let tx = bx * BLOCK + BLOCK / 2;
        let ty = sy * BLOCK + BLOCK / 2;
        let tz = bz * BLOCK + BLOCK / 2;
        if !ray_hits_solid(cam.x, cam.y, cam.z, tx, ty, tz) {
            return false; // this point is visible
        }
        k += 1;
    }
    true
}

// --- culled face iteration for the renderer ---

#[inline(never)]
fn plane_in_frustum(
    bx0: i32,
    by0: i32,
    bz0: i32,
    dir: usize,
    plane: usize,
    bounds: u32,
    far_lim: i32,
) -> u8 {
    let amin = (bounds & 127) as i32;
    let amax = ((bounds >> 7) & 127) as i32;
    let bmin = ((bounds >> 14) & 127) as i32;
    let bmax = ((bounds >> 21) & 127) as i32;
    let ac = (amin + amax) * (BLOCK / 2);
    let bc = (bmin + bmax) * (BLOCK / 2);
    let normal = (plane as i32 + ((dir & 1 == 0) as i32)) * BLOCK;
    let (x, y, z) = match dir {
        0 | 1 => (normal, ac, bc),
        2 | 3 => (ac, normal, bc),
        _ => (ac, bc, normal),
    };
    let ha = (amax - amin) * (BLOCK / 2);
    let hb = (bmax - bmin) * (BLOCK / 2);
    let long = ha.max(hb);
    let short = ha.min(hb);
    let r = long + (short * 27 + 63) / 64;
    let c = scene::transform_vertex_scheduled(Vec3I16::new(
        (bx0 + x) as i16,
        (by0 + y) as i16,
        (bz0 + z) as i16,
    ));
    if c.z + r < 0 || c.z - r > far_lim {
        return 0;
    }
    // Wholly OUTSIDE the screen cone: the same conservative sphere-vs-cone
    // test the per-face cull applies, lifted to the plane's bounding sphere
    // (nearest possible lateral offset |c|-r against the farthest possible
    // depth c.z+r). An earlier aggregate test was disabled after it rejected
    // planes with visible faces; it used a different construction. This is
    // BYTE-FOR-BYTE the per-face form, so a plane it rejects could not have
    // yielded a surviving face -- and it turns the dominant cost of the face
    // loop (iterating faces the per-face cull then discards, ~85% of all
    // iterated faces at a 28-block draw) into one MVMVA per plane.
    let zf = c.z + r;
    if (c.y.abs() - r) * PROJ_H > zf * 200 {
        return 0;
    }
    if (c.x.abs() - r) * PROJ_H > zf * 160 {
        return 0;
    }
    let zn = c.z - r;
    if zn > 0
        && c.z + r <= far_lim
        && (c.y.abs() + r) * PROJ_H <= zn * 200
        && (c.x.abs() + r) * PROJ_H <= zn * 160
    {
        2
    } else {
        1
    }
}

/// Call `emit(block, lx, wy, lz, dir, w, h)` for every cached face of every
/// loaded chunk that passes a cheap distance + frustum cull. Face coordinates
/// are CHUNK-LOCAL: `begin_chunk(oxw, ozw)` fires before a visible chunk's
/// faces so the caller can point the GTE translation at that chunk's origin
/// and feed corners to COP2 as tiny i16s (no per-corner CPU subtraction).
/// inline(never): inlined into main's huge frame the loop's hot state (cull
/// tables, pool pointers, camera trig) all spilled to stack slots -- ~180
/// cycles per iterated face. A dedicated frame keeps them in registers.
#[inline(never)]
pub fn for_visible_faces<
    G: FnMut(i32, i32),
    F: FnMut(u8, i32, i32, i32, usize, usize, usize, usize, u8),
>(
    cam: &Camera,
    mut begin_chunk: G,
    mut emit: F,
) -> usize {
    let mut face_work = 0usize;
    let chunk_r = 12 * BLOCK; // horizontal half-extent of a chunk, world units
    // The per-face frustum cull runs ON THE GTE: one MVMVA (rt*v0+tr, ~8 GTE
    // cycles) of the face's anchor-cell centre -- chunk-local i16, using the
    // per-chunk TR begin_chunk just loaded -- returns camera-space (x, -y, z)
    // in MAC1..3. That replaces the old 640 bytes of per-frame cull tables
    // (whose base pointers spilled) plus 2 multiplies per face, and the
    // tilted-frame values are exact instead of table-split approximations.
    let mut s = 0;
    while s < NCHUNKS {
        let fslot = unsafe { CHUNKS[s].face_slot };
        // Only chunks with a meshed pool slot can draw (loaded-but-unmeshed and
        // out-of-range chunks have none).
        if unsafe { CHUNKS[s].loaded } && fslot != NO_SLOT {
            let p = fslot as usize;
            let cx = unsafe { CHUNKS[s].cx };
            let cz = unsafe { CHUNKS[s].cz };
            let ccx = (cx * CW + CW / 2) * BLOCK;
            let ccz = (cz * CW + CW / 2) * BLOCK;
            let dxb = ccx - cam.x;
            let dzb = ccz - cam.z;
            let zc = ((dxb * cam.sy) + (dzb * cam.cy)) >> 12; // forward
            let xc = ((dxb * cam.cy) - (dzb * cam.sy)) >> 12; // right
            let visible = zc >= -chunk_r
                && zc - chunk_r <= FAR_Z
                && (zc <= 0 || (xc.abs() - chunk_r) <= zc * 160 / PROJ_H);
            // A chunk wholly past the side horizon contributes only its
            // silhouette, which is its TOP faces; skip the other dirs before
            // iterating a single face of them.
            let tops_only = zc - chunk_r > FAR_SIDE_Z;
            // Far-ring chunks: skip entirely if a ray to their surface is blocked
            // by nearer terrain (don't iterate/project a hill that's hidden behind
            // another hill). Near chunks always draw -- the ray would be too short.
            let occluded = OCCLUSION_CULL && chunk_lod(s) == 1 && chunk_occluded(cam, cx, cz);
            if visible && !occluded {
                let ox = cx * CW;
                let oz = cz * CW;
                let oxw = ox * BLOCK;
                let ozw = oz * BLOCK;
                begin_chunk(oxw, ozw); // caller notes this chunk's camera-relative origin
                // Camera-relative anchor-centre base for the MVMVA cull (the
                // GTE translation is zero; see gte_begin_chunk's crack note).
                let ccx0 = oxw + BLOCK / 2 - cam.x;
                let ccy0 = BLOCK / 2 - cam.y;
                let ccz0 = ozw + BLOCK / 2 - cam.z;
                let mut dir = 0;
                while dir < 6 {
                    if tops_only && dir != 2 {
                        dir += 1;
                        continue;
                    }
                    // Sides and bottoms stop at FAR_SIDE_Z; tops carry the
                    // silhouette out to FAR_Z (see FAR_SIDE_Z in main).
                    let far_lim = if dir == 2 { FAR_Z } else { FAR_SIDE_Z };
                    // Visible plane sub-range for this dir: the old per-face
                    // backface test compared the camera to the face's PLANE
                    // coordinate only, so cutting the (plane-sorted) range is
                    // exact -- and skips iterating back-facing faces at all.
                    let rel = match dir {
                        0 | 1 => cam.x - oxw,
                        2 | 3 => cam.y,
                        _ => cam.z - ozw,
                    };
                    let npl = PL_N[dir] as i32;
                    // dir even (+axis): planes 0..=hi with hi from `cam > plane_centre`;
                    // dir odd (-axis): planes lo..=npl-1 with lo from `cam < plane_centre`.
                    let (lo, hi) = if dir & 1 == 0 {
                        (0, floor_div(rel - BLOCK / 2 - 1, BLOCK).min(npl - 1))
                    } else {
                        ((floor_div(rel - BLOCK / 2, BLOCK) + 1).max(0), npl - 1)
                    };
                    if lo > hi {
                        dir += 1;
                        continue;
                    }
                    let off = PL_OFF[dir];
                    let mut plane = lo as usize;
                    while plane <= hi as usize {
                        let start = unsafe { POOL_PLANE_START[p][off + plane] } as usize;
                        let end = unsafe { POOL_PLANE_START[p][off + plane + 1] } as usize;
                        if start == end {
                            plane += 1;
                            continue;
                        }
                        let bounds = unsafe { POOL_PLANE_BOUNDS[p][off + plane] };
                        let plane_vis = if bounds == 0 {
                            // The face range, not this optional cache, decides
                            // whether a plane is empty.
                            1
                        } else {
                            plane_in_frustum(
                                oxw - cam.x,
                                -cam.y,
                                ozw - cam.z,
                                dir,
                                plane,
                                bounds,
                                far_lim,
                            )
                        };
                        if plane_vis == 0 {
                            plane += 1;
                            continue;
                        }
                        face_work += end - start;
                    let mut k = start;
                    while k < end {
                        let f = unsafe { POOL_FACES[p][k] };
                        k += 1;
                        let lx = (f & 15) as usize;
                        let wy = ((f >> 4) & 63) as i32;
                        let lz = ((f >> 10) & 15) as usize;
                        let w = ((f >> 24) & 15) as usize + 1;
                        let h = ((f >> 28) & 7) as usize + 1;
                        // Light comes from the side-band word beside the face,
                        // read below with AO. Bit 31 of the packed word used to
                        // hold it, which capped it at ONE bit -- values above 1
                        // shifted off the end.
                        // View-frustum cull in camera space, BEFORE the costly
                        // 4-corner perspective projection in emit(): drop faces
                        // behind the camera, beyond far, above/below, or outside
                        // the FOV cone.
                            if FRUSTUM_CULL && plane_vis != 2 {
                            // Bounding sphere of the whole merged plate, NOT its
                            // min corner: a greedy face spans up to 12x12 blocks,
                            // so sampling the anchor and comparing it against an
                            // anchor-depth cone dropped plates whose corner sat
                            // behind/beside the eye while most of the plate was
                            // dead ahead -- the ground vanished from under the
                            // player in terraced terrain.
                            //   centre = anchor + half the face extent
                                //   r      = conservative half-diagonal. For
                                //            0<=short<=long, long+27/64*short bounds
                                //            sqrt(long²+short²), closely following the
                                //            chord to sqrt(2) without a runtime sqrt.
                            let hw = (w as i32 - 1) * (BLOCK / 2);
                            let hh = (h as i32 - 1) * (BLOCK / 2);
                            let (cox, coy, coz) = match dir {
                                    0 => (BLOCK / 2, hw, hh),
                                    1 => (-BLOCK / 2, hw, hh),
                                    2 => (hw, BLOCK / 2, hh),
                                    3 => (hw, -BLOCK / 2, hh),
                                    4 => (hw, hh, BLOCK / 2),
                                    _ => (hw, hh, -BLOCK / 2),
                            };
                            let rw = w as i32 * (BLOCK / 2);
                            let rh = h as i32 * (BLOCK / 2);
                            let long = rw.max(rh);
                            let short = rw.min(rh);
                                let r = long + (short * 27 + 63) / 64;
                            // ONE GTE MVMVA gives the sphere centre in camera
                            // space exactly: c.x = screen-right, c.y = negated
                            // height (the loaded Y row is negated), c.z = true
                            // pitch-tilted depth.
                            let c = scene::transform_vertex_scheduled(Vec3I16::new(
                                (ccx0 + lx as i32 * BLOCK + cox) as i16,
                                (ccy0 + wy * BLOCK + coy) as i16,
                                (ccz0 + lz as i32 * BLOCK + coz) as i16,
                            ));
                            let z2 = c.z;
                            // Far cull on the sphere CENTRE, not its near point.
                            // emit_face throws a face away after full GTE
                            // projection when the average of its four projected
                            // corner depths reaches FAR_Z -- and that average is
                            // this same centre depth. Culling on `z2 - r` was
                            // looser than the test the face would face anyway, so
                            // every face in that band paid a full projection to be
                            // discarded. A profiling pass counted 77 of 393 faces
                            // a frame dying exactly there.
                            if z2 + r < 0 || z2 > far_lim {
                                continue;
                            }
                            // Cone tests against the sphere: compare the nearest
                            // possible screen offset (|c| - r) with the FARTHEST
                            // possible depth (z2 + r). Half-height 120 + the emit
                            // bbox's +-80 margin = 200; half-width 160.
                            // No near-exemption needed: the sphere bound is exact
                            // enough that close off-axis plates survive on their
                            // own (the old +-3-block escape hatch existed only to
                            // paper over the min-corner sampling above).
                            let zs = z2 + r;
                            if zs > 0 {
                                if (c.y.abs() - r) * PROJ_H > zs * 200 {
                                    continue;
                                }
                                if (c.x.abs() - r) * PROJ_H > zs * 160 {
                                    continue;
                                }
                            }
                        }
                        let block = ((f >> 17) & 127) as u8;
                        // AO byte read only for faces that SURVIVED the cull:
                        // it is a second uncached array, so paying it per
                        // iterated face would cost on the ones we throw away.
                        let side = unsafe { POOL_AO[p][k - 1] };
                        let ao = (side & 0xFF) as u8;
                        let light = ((side >> 8) & 7) as usize;
                        emit(block, lx as i32, wy, lz as i32, dir, w, h, light, ao);
                    }
                        plane += 1;
                    }
                    dir += 1;
                }
            }
        }
        s += 1;
    }
    face_work
}

/// Iterate visible cross-sprite plants near the camera, calling
/// `emit(block, world_x, world_y, world_z)` at each cell's min corner. Plants
/// are tiny on screen, so only chunks whose centre is within PLANT_FAR are
/// considered; the caller (render_plants) builds the X-billboard and depth-sorts
/// it into the OT. Reuses the per-chunk plant list recorded at mesh time.
pub fn for_plants<F: FnMut(u8, i32, i32, i32)>(cam: &Camera, mut emit: F) {
    // Built shapes (slabs, fences) have to survive as far as the terrain does;
    // cross sprites do not, and main drops those per block past PLANT_SPRITE_FAR.
    const PLANT_FAR: i32 = 20 * BLOCK;
    let chunk_r = 12 * BLOCK;
    let mut s = 0;
    while s < NCHUNKS {
        let fslot = unsafe { CHUNKS[s].face_slot };
        if unsafe { CHUNKS[s].loaded } && fslot != NO_SLOT {
            let p = fslot as usize;
            let npl = unsafe { POOL_NPLANT[p] } as usize;
            if npl != 0 {
                let cx = unsafe { CHUNKS[s].cx };
                let cz = unsafe { CHUNKS[s].cz };
                let ccx = (cx * CW + CW / 2) * BLOCK;
                let ccz = (cz * CW + CW / 2) * BLOCK;
                let dxb = ccx - cam.x;
                let dzb = ccz - cam.z;
                let zc = ((dxb * cam.sy) + (dzb * cam.cy)) >> 12; // forward depth
                let xc = ((dxb * cam.cy) - (dzb * cam.sy)) >> 12; // right
                // Near, ahead, and within the horizontal FOV cone (+ chunk half-extent).
                if zc >= -chunk_r
                    && zc - chunk_r <= PLANT_FAR
                    && (zc <= 0 || (xc.abs() - chunk_r) <= zc * 160 / PROJ_H)
                {
                    let ox = cx * CW * BLOCK;
                    let oz = cz * CW * BLOCK;
                    let mut k = 0;
                    while k < npl {
                        let e = unsafe { POOL_PLANTS[p][k] };
                        let lx = (e & 15) as i32;
                        let ly = ((e >> 4) & 63) as i32;
                        let lz = ((e >> 10) & 15) as i32;
                        let b = ((e >> 14) & 127) as u8;
                        emit(b, ox + lx * BLOCK, ly * BLOCK, oz + lz * BLOCK);
                        k += 1;
                    }
                }
            }
        }
        s += 1;
    }
}

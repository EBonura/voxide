//! Column run-length block store.
//!
//! Every resident chunk keeps its blocks as runs down each of its 256 columns:
//! (block, length) byte pairs from y = 63 downwards, the lengths of a column
//! summing to 64. Terrain is a few long runs per column (air, grass, dirt,
//! stone, the odd cave or ore), so a chunk averages ~3.4 KB here against
//! 14,337 bytes 7-bit packed, and a read near the surface -- collision, the
//! pick ray, mobs -- finds its run within the first few pairs.
//!
//! The runs of all chunks share one byte arena. A chunk holds one region of it
//! (RBASE/RCAP, in bytes), its columns packed back to back, with COL_OFF giving
//! each column's start inside the region and COL_OFF[256] the bytes in use.
//! An edit re-encodes one column and shifts the rest of the region by the size
//! change; a region that runs out of room is moved (blk_alloc), sliding the
//! other regions down to close gaps when it has to.

use super::*;

/// Average bytes per resident chunk. The largest chunk measured was 4,018
/// bytes plus the 514-byte offset table kept alongside (48 generated chunks
/// and 250 ring chunks from the train/unseen routes), so this leaves ~50%
/// headroom for edits and for terrain rougher than the routes saw.
pub(super) const BLK_BUDGET: usize = if cfg!(feature = "ram-stress-blk") { 3200 } else { 6144 };
const BLK_BYTES: usize = NCHUNKS * BLK_BUDGET;
/// Slack given to a region beyond what it needs, so most edits grow in place.
const SLACK: usize = if cfg!(feature = "ram-stress-blk") { 0 } else { 64 };

static mut BLK: [u8; BLK_BYTES] = [0; BLK_BYTES];
static mut COL_OFF: [[u16; CWU * CWU + 1]; NCHUNKS] = [[0; CWU * CWU + 1]; NCHUNKS];
static mut RBASE: [u32; NCHUNKS] = [0; NCHUNKS];
/// Region capacity in bytes; 0 = the slot holds no region.
static mut RCAP: [u32; NCHUNKS] = [0; NCHUNKS];
/// Allocations that failed even after compacting (the chunk or edit was lost).
pub(super) static mut BLK_SHORT: u32 = 0;
pub(super) static mut BLK_COMPACTS: u32 = 0;
/// Compactions that had to carry a growing region's contents along.
pub(super) static mut BLK_ROTATES: u32 = 0;
/// Layer-major bitmap of the CLS_SPECIAL cells found by a FULL decode (bit col
/// of word col / 32 in row ly), so light sources and plants are recorded in
/// the same ascending index order the per-cell decode used.
static mut SPEC: [[u32; 8]; CHU] = [[0; 8]; CHU];

/// Block at chunk-local index `i` of resident slot `s`.
#[inline(always)]
pub(super) fn rget(s: usize, i: usize) -> u8 {
    unsafe {
        let y = (i >> 8) as u32;
        let mut p = BLK
            .as_ptr()
            .add(RBASE[s] as usize + COL_OFF[s][i & (CWU * CWU - 1)] as usize);
        let mut top = CHU as u32;
        loop {
            top -= *p.add(1) as u32;
            if y >= top {
                return *p;
            }
            p = p.add(2);
        }
    }
}

fn col_unpack(s: usize, col: usize, out: &mut [u8; CHU]) {
    unsafe {
        let mut o = RBASE[s] as usize + COL_OFF[s][col] as usize;
        let mut top = CHU;
        while top > 0 {
            let b = BLK[o];
            let lo = top - BLK[o + 1] as usize;
            let mut y = lo;
            while y < top {
                out[y] = b;
                y += 1;
            }
            top = lo;
            o += 2;
        }
    }
}

/// Encode a column (index = y) top-down into `out`; returns bytes written.
fn col_pack(inp: &[u8; CHU], out: &mut [u8; 2 * CHU]) -> usize {
    let mut n = 0;
    let mut y = CHU;
    while y > 0 {
        let b = inp[y - 1];
        let mut k = 1;
        while k < y && inp[y - 1 - k] == b {
            k += 1;
        }
        out[n] = b;
        out[n + 1] = k as u8;
        n += 2;
        y -= k;
    }
    n
}

/// Write block `b` at chunk-local index `i` of resident slot `s`.
pub(super) fn rset(s: usize, i: usize, b: u8) {
    let col = i & (CWU * CWU - 1);
    let y = i >> 8;
    let mut c = [0u8; CHU];
    col_unpack(s, col, &mut c);
    if c[y] == b {
        return;
    }
    c[y] = b;
    let mut enc = [0u8; 2 * CHU];
    let n = col_pack(&c, &mut enc);
    unsafe {
        let o0 = COL_OFF[s][col] as usize;
        let o1 = COL_OFF[s][col + 1] as usize;
        let old = o1 - o0;
        let len = COL_OFF[s][CWU * CWU] as usize;
        if n != old {
            let newlen = len + n - old;
            if newlen > RCAP[s] as usize && !blk_alloc(s, newlen + SLACK, true) {
                BLK_SHORT += 1;
                return;
            }
            let base = RBASE[s] as usize;
            if n > old {
                let d = n - old;
                let mut k = len;
                while k > o1 {
                    k -= 1;
                    BLK[base + k + d] = BLK[base + k];
                }
            } else {
                let d = old - n;
                let mut k = o1;
                while k < len {
                    BLK[base + k - d] = BLK[base + k];
                    k += 1;
                }
            }
            let mut k = col + 1;
            while k <= CWU * CWU {
                COL_OFF[s][k] = (COL_OFF[s][k] as usize + n - old) as u16;
                k += 1;
            }
        }
        let base = RBASE[s] as usize + o0;
        let mut k = 0;
        while k < n {
            BLK[base + k] = enc[k];
            k += 1;
        }
    }
}

/// Give slot `s` a region of at least `need` bytes. With `keep` its current
/// contents (COL_OFF[s][256] bytes) move along; without, the old region is
/// dropped. Returns false when the arena is full even after compacting (the
/// slot then keeps its old region if `keep`, or none).
fn blk_alloc(s: usize, need: usize, keep: bool) -> bool {
    unsafe {
        let need = (need + 3) & !3;
        if !keep {
            RCAP[s] = 0;
        }
        let mut order = [0usize; NCHUNKS];
        let mut nl = 0;
        let mut q = 0;
        while q < NCHUNKS {
            if RCAP[q] != 0 {
                let mut k = nl;
                while k > 0 && RBASE[order[k - 1]] > RBASE[q] {
                    order[k] = order[k - 1];
                    k -= 1;
                }
                order[k] = q;
                nl += 1;
            }
            q += 1;
        }
        let len = if keep { COL_OFF[s][CWU * CWU] as usize } else { 0 };
        // First fit.
        let mut cur = 0usize;
        let mut k = 0;
        while k <= nl {
            let next = if k < nl { RBASE[order[k]] as usize } else { BLK_BYTES };
            if next >= cur + need {
                if keep {
                    let src = RBASE[s] as usize;
                    let mut i = 0;
                    while i < len {
                        BLK[cur + i] = BLK[src + i];
                        i += 1;
                    }
                }
                RBASE[s] = cur as u32;
                RCAP[s] = need as u32;
                return true;
            }
            if k < nl {
                cur = next + RCAP[order[k]] as usize;
            }
            k += 1;
        }
        // Compact: slide every live region down in address order.
        BLK_COMPACTS += 1;
        let mut cur = 0usize;
        let mut k = 0;
        while k < nl {
            let q = order[k];
            let b = RBASE[q] as usize;
            let c = RCAP[q] as usize;
            if b != cur {
                let mut i = 0;
                while i < c {
                    BLK[cur + i] = BLK[b + i];
                    i += 1;
                }
                RBASE[q] = cur as u32;
            }
            cur += c;
            k += 1;
        }
        if keep {
            // Rotate s to the top of the used space so it can grow into the
            // free space above: three reversals, no staging buffer.
            BLK_ROTATES += 1;
            let bs = RBASE[s] as usize;
            let cs = RCAP[s] as usize;
            reverse(bs, bs + cs);
            reverse(bs + cs, cur);
            reverse(bs, cur);
            let mut q = 0;
            while q < NCHUNKS {
                if q != s && RCAP[q] != 0 && RBASE[q] as usize > bs {
                    RBASE[q] -= cs as u32;
                }
                q += 1;
            }
            RBASE[s] = (cur - cs) as u32;
            if cur - cs + need <= BLK_BYTES {
                RCAP[s] = need as u32;
                return true;
            }
            return false;
        }
        RBASE[s] = cur as u32;
        if cur + need <= BLK_BYTES {
            RCAP[s] = need as u32;
            return true;
        }
        false
    }
}

fn reverse(mut a: usize, mut b: usize) {
    unsafe {
        while a + 1 < b {
            b -= 1;
            let t = BLK[a];
            BLK[a] = BLK[b];
            BLK[b] = t;
            a += 1;
        }
    }
}

/// Run starts of each generator column, bit y (word y / 32) set when cell y
/// differs from the cell above it; y = 63 always starts a run. Built once per
/// chunk by gen_bounds, then read by every pack batch of that chunk.
static mut GEN_BOUND: [[u32; 2]; CWU * CWU] = [[0; 2]; CWU * CWU];

/// Fill GEN_BOUND from GEN_SCRATCH and return the chunk's total run count.
/// Compares whole layers a word (four columns) at a time: run boundaries are
/// a few per column, so almost every word compares equal and the per-column
/// work happens only where a boundary is.
fn gen_bounds() -> usize {
    unsafe {
        let mut col = 0;
        while col < CWU * CWU {
            GEN_BOUND[col] = [0, 1 << 31];
            col += 1;
        }
        let mut runs = CWU * CWU;
        let base = core::ptr::addr_of!(GEN_SCRATCH) as *const u8;
        let mut y = 0;
        while y < CHU - 1 {
            let lo = base.add(y * CWU * CWU);
            let hi = lo.add(CWU * CWU);
            let bit = 1u32 << (y & 31);
            let half = y >> 5;
            let mut w = 0;
            while w < CWU * CWU {
                let x = core::ptr::read_unaligned(lo.add(w) as *const u32)
                    ^ core::ptr::read_unaligned(hi.add(w) as *const u32);
                if x != 0 {
                    let mut k = 0;
                    while k < 4 {
                        if x & (0xFF << (8 * k)) != 0 {
                            GEN_BOUND[w + k][half] |= bit;
                            runs += 1;
                        }
                        k += 1;
                    }
                }
                w += 4;
            }
            y += 1;
        }
        runs
    }
}

/// Pack generator columns `c0..c1` of GEN_SCRATCH into slot `s`. The first
/// call (c0 == 0) finds the run boundaries, sizes the whole chunk and
/// allocates its region; returns false if that failed.
pub(super) fn pack_cols(s: usize, c0: usize, c1: usize) -> bool {
    unsafe {
        if c0 == 0 {
            let runs = gen_bounds();
            if !blk_alloc(s, 2 * runs + SLACK, false) {
                BLK_SHORT += 1;
                return false;
            }
            COL_OFF[s][0] = 0;
        }
        let base = RBASE[s] as usize;
        let mut col = c0;
        while col < c1 {
            // Run starts in ascending y, then written top-down.
            let mut starts = [0u8; CHU];
            let mut n = 0;
            let mut half = 0;
            while half < 2 {
                let mut m = GEN_BOUND[col][half];
                while m != 0 {
                    starts[n] = (32 * half + m.trailing_zeros() as usize) as u8;
                    n += 1;
                    m &= m - 1;
                }
                half += 1;
            }
            let mut o = base + COL_OFF[s][col] as usize;
            while n > 0 {
                n -= 1;
                let y = starts[n] as usize;
                let below = if n > 0 { starts[n - 1] as usize + 1 } else { 0 };
                BLK[o] = GEN_SCRATCH[y * CWU * CWU + col];
                BLK[o + 1] = (y + 1 - below) as u8;
                o += 2;
            }
            COL_OFF[s][col + 1] = (o - base) as u16;
            col += 1;
        }
        true
    }
}

/// Blocks `y0..y0 + out.len()` of column `col` of resident slot `s`, one run
/// walk for the whole span; cells outside 0..64 read AIR, as `get` answers.
pub(super) fn col_span(s: usize, col: usize, y0: i32, out: &mut [u8]) {
    unsafe {
        out.fill(AIR);
        let y1 = y0 + out.len() as i32;
        let mut o = RBASE[s] as usize + COL_OFF[s][col] as usize;
        let mut top = CHU as i32;
        while top > y0 && top > 0 {
            let b = BLK[o];
            let lo = top - BLK[o + 1] as i32;
            let mut y = lo.max(y0);
            while y < top.min(y1) {
                out[(y - y0) as usize] = b;
                y += 1;
            }
            top = lo;
            o += 2;
        }
    }
}

/// Decode columns `c0..c1` of slot `s` into MESH_SCRATCH and the column
/// masks, a run at a time. `top` tracks the highest non-air index. With FULL
/// the stream mesher's extras are recorded as well: per-column sky top, and,
/// once the last column is in, light sources and plants in ascending index
/// order (the SPEC bitmap is cleared by the first batch).
#[inline(never)]
pub(super) fn decode_cols<const FULL: bool>(s: usize, c0: usize, c1: usize, top: &mut usize) {
    unsafe {
        if FULL && c0 == 0 {
            SPEC = [[0; 8]; CHU];
        }
        let region = RBASE[s] as usize;
        let mut col = c0;
        while col < c1 {
            let mut o = region + COL_OFF[s][col] as usize;
            let mut y1 = CHU;
            let mut mesh = 0u64;
            let mut see = 0u64;
            let mut sky = false;
            while y1 > 0 {
                let b = BLK[o];
                let n = BLK[o + 1] as usize;
                o += 2;
                let y0 = y1 - n;
                let bits = if n == CHU { !0u64 } else { ((1u64 << n) - 1) << y0 };
                let mut i = y0 * CWU * CWU + col;
                let mut k = 0;
                while k < n {
                    MESH_SCRATCH[i] = b;
                    i += CWU * CWU;
                    k += 1;
                }
                if b != AIR {
                    let t = (y1 - 1) * CWU * CWU + col;
                    if t > *top {
                        *top = t;
                    }
                    let cls = BCLASS[b as usize];
                    if cls & CLS_MESH != 0 {
                        mesh |= bits;
                        if FULL && !sky {
                            SKY_TOP[col] = (y1 - 1) as u8;
                            sky = true;
                        }
                    }
                    if cls & CLS_SEE != 0 {
                        see |= bits;
                    }
                    if FULL && cls & CLS_SPECIAL != 0 {
                        let mut y = y0;
                        while y < y1 {
                            SPEC[y][col >> 5] |= 1 << (col & 31);
                            y += 1;
                        }
                    }
                } else {
                    see |= bits;
                }
                y1 = y0;
            }
            COL_MESH[col] |= mesh;
            COL_SEE[col] |= see;
            col += 1;
        }
        if FULL && c1 == CWU * CWU {
            let mut ly = 0;
            while ly < CHU {
                let mut w = 0;
                while w < 8 {
                    let mut m = SPEC[ly][w];
                    while m != 0 {
                        let col = w * 32 + m.trailing_zeros() as usize;
                        m &= m - 1;
                        let b = MESH_SCRATCH[ly * CWU * CWU + col];
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
                    w += 1;
                }
                ly += 1;
            }
        }
    }
}

/// ram-stress-blk builds (from recenter, once the ring is loaded): force region growth past the arena's free space
/// (compaction with a carried region), check every chunk cell by cell against
/// hashes taken first, restore, and print the verdict on the TTY.
#[cfg(feature = "ram-stress-blk")]
static mut SELF_TESTED: bool = false;

/// Run self_test once, the first time the whole ring is resident.
#[cfg(feature = "ram-stress-blk")]
pub(super) fn self_test_once() {
    unsafe {
        if SELF_TESTED {
            return;
        }
        let mut s = 0;
        while s < NCHUNKS {
            if !CHUNKS[s].loaded {
                return;
            }
            s += 1;
        }
        SELF_TESTED = true;
    }
    self_test();
}

#[cfg(feature = "ram-stress-blk")]
fn self_test() {
    fn chunk_hash(s: usize) -> u32 {
        let mut h = 0x811C_9DC5u32;
        let mut i = 0;
        while i < CHUNK_VOL {
            h = (h ^ rget(s, i) as u32).wrapping_mul(0x0100_0193);
            i += 1;
        }
        h
    }
    unsafe {
        let mut before = [0u32; NCHUNKS];
        let mut s = 0;
        while s < NCHUNKS {
            if CHUNKS[s].loaded {
                before[s] = chunk_hash(s);
            }
            s += 1;
        }
        let r0 = BLK_ROTATES;
        let c0 = BLK_COMPACTS;
        let short0 = BLK_SHORT;
        let mut bad = 0u32;
        // Stripe 12 columns in the first loaded slot: a column of
        // alternating blocks is 64 runs, 128 bytes, so each chunk grows ~1.4 KB.
        let mut saved = [[0u8; CHU]; 12];
        let mut done = 0;
        let mut s = 0;
        while s < NCHUNKS && done < 1 {
            if CHUNKS[s].loaded {
                let mut c = 0;
                while c < 12 {
                    col_unpack(s, c * 16 + 3, &mut saved[c]);
                    let mut y = 0;
                    while y < CHU {
                        rset(s, y * CWU * CWU + c * 16 + 3, if y & 1 == 0 { STONE } else { DIRT });
                        y += 1;
                    }
                    c += 1;
                }
                // Striped cells read back?
                let mut c = 0;
                while c < 12 {
                    let mut y = 0;
                    while y < CHU {
                        if rget(s, y * CWU * CWU + c * 16 + 3) != if y & 1 == 0 { STONE } else { DIRT } {
                            bad += 1;
                        }
                        y += 1;
                    }
                    c += 1;
                }
                // Restore.
                let mut c = 0;
                while c < 12 {
                    let mut y = 0;
                    while y < CHU {
                        rset(s, y * CWU * CWU + c * 16 + 3, saved[c][y]);
                        y += 1;
                    }
                    c += 1;
                }
                done += 1;
            }
            s += 1;
        }
        let mut s = 0;
        while s < NCHUNKS {
            if CHUNKS[s].loaded && chunk_hash(s) != before[s] {
                bad += 1;
            }
            s += 1;
        }
        psx_rt::tty::print("RLE selftest ");
        psx_rt::tty::print(if bad == 0 && BLK_SHORT == short0 { "ok" } else { "FAIL" });
        psx_rt::tty::print(" bad=");
        psx_rt::tty::print_hex_u32(bad);
        psx_rt::tty::print(" rotates=");
        psx_rt::tty::print_hex_u32(BLK_ROTATES - r0);
        psx_rt::tty::print(" compacts=");
        psx_rt::tty::print_hex_u32(BLK_COMPACTS - c0);
        psx_rt::tty::print(" shorts=");
        psx_rt::tty::print_hex_u32(BLK_SHORT - short0);
        psx_rt::tty::println("");
    }
}

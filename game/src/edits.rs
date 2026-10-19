//! The player's edits to the world: what differs from the seeded terrain.
//!
//! A chunk that leaves the loaded ring is thrown away and later generated again
//! from the seed, so this store is the only record a build has: it is replayed
//! onto every chunk as it is generated, and it is what the memory card saves.
//!
//! It used to be one flat log of 1,024 positions that also dropped every edit
//! past the cap without a word. Now each chunk has its own chain of edits, found
//! through a small table of chunks, so
//!
//! - a chunk's replay reads that chunk's edits and nothing else (it scanned the
//!   whole log per generated chunk);
//! - the store holds `CAP` edits (8,192) in about 100 KB, and when it is full
//!   `record` says so, which the callers turn into a refused edit and a message
//!   instead of a silent loss;
//! - the card gets the edits grouped by chunk, 3 bytes an edit plus 7 for the
//!   chunk, instead of 8 an edit.
//!
//! An edit is the cell's whole byte (bstate.rs), so a door's swing or a lever's
//! switch is saved the same way a placed block is. Last write wins per position.
//!
//! Plain data with no dependencies, so the host tests include this file as it is.

/// Edits held at once.
pub const CAP: usize = 8192;
/// Chunks that can hold edits at once.
pub const CHUNKS: usize = 1024;
const POS_SLOTS: usize = 2 * CAP;
const CK_SLOTS: usize = 2 * CHUNKS;

/// The card record sizes: a chunk header, then a record per edit.
pub const CHUNK_REC: usize = 7;
pub const EDIT_REC: usize = 3;

pub struct Edits {
    // Per edit: cell index in its chunk (ly << 8 | lz << 4 | lx), block, the
    // next edit of the same chunk (index + 1, 0 ends the chain), and the chunk.
    idx: [u16; CAP],
    blk: [u8; CAP],
    next: [u16; CAP],
    ck: [u16; CAP],
    n: usize,
    // Per chunk: where it is, the head of its chain and how many it holds.
    cx: [i16; CHUNKS],
    cz: [i16; CHUNKS],
    dim: [u8; CHUNKS],
    head: [u16; CHUNKS],
    cnt: [u16; CHUNKS],
    nck: usize,
    // Open-addressed indexes (value = index + 1, 0 empty): chunks by position,
    // and edits by (chunk, cell) so a repeat edit finds its entry without
    // walking the chain.
    ck_hash: [u16; CK_SLOTS],
    pos_hash: [u16; POS_SLOTS],
}

#[inline]
fn ck_slot(cx: i32, cz: i32, dim: u8) -> usize {
    let h = (cx as u32)
        .wrapping_mul(0x9E37_79B1)
        .wrapping_add((cz as u32).wrapping_mul(0x85EB_CA6B))
        .wrapping_add((dim as u32).wrapping_mul(0xC2B2_AE35));
    (h >> 17) as usize & (CK_SLOTS - 1)
}

#[inline]
fn pos_slot(ck: usize, idx: u16) -> usize {
    let h = (ck as u32)
        .wrapping_mul(0x9E37_79B1)
        .wrapping_add((idx as u32).wrapping_mul(0x85EB_CA6B));
    (h >> 15) as usize & (POS_SLOTS - 1)
}

impl Edits {
    pub const fn new() -> Self {
        Self {
            idx: [0; CAP],
            blk: [0; CAP],
            next: [0; CAP],
            ck: [0; CAP],
            n: 0,
            cx: [0; CHUNKS],
            cz: [0; CHUNKS],
            dim: [0; CHUNKS],
            head: [0; CHUNKS],
            cnt: [0; CHUNKS],
            nck: 0,
            ck_hash: [0; CK_SLOTS],
            pos_hash: [0; POS_SLOTS],
        }
    }

    pub fn clear(&mut self) {
        self.n = 0;
        self.nck = 0;
        self.ck_hash = [0; CK_SLOTS];
        self.pos_hash = [0; POS_SLOTS];
    }

    pub fn len(&self) -> usize {
        self.n
    }

    /// Edits that can still be added.
    pub fn room(&self) -> usize {
        CAP - self.n
    }

    pub fn chunks(&self) -> usize {
        self.nck
    }

    fn find_chunk(&self, cx: i32, cz: i32, dim: u8) -> Option<usize> {
        let mut s = ck_slot(cx, cz, dim);
        loop {
            let v = self.ck_hash[s] as usize;
            if v == 0 {
                return None;
            }
            let c = v - 1;
            if self.cx[c] as i32 == cx && self.cz[c] as i32 == cz && self.dim[c] == dim {
                return Some(c);
            }
            s = (s + 1) & (CK_SLOTS - 1);
        }
    }

    fn add_chunk(&mut self, cx: i32, cz: i32, dim: u8) -> Option<usize> {
        if self.nck >= CHUNKS {
            return None;
        }
        let c = self.nck;
        self.nck += 1;
        self.cx[c] = cx as i16;
        self.cz[c] = cz as i16;
        self.dim[c] = dim;
        self.head[c] = 0;
        self.cnt[c] = 0;
        let mut s = ck_slot(cx, cz, dim);
        while self.ck_hash[s] != 0 {
            s = (s + 1) & (CK_SLOTS - 1);
        }
        self.ck_hash[s] = (c + 1) as u16;
        Some(c)
    }

    fn find_edit(&self, ck: usize, idx: u16) -> (usize, Option<usize>) {
        let mut s = pos_slot(ck, idx);
        loop {
            let v = self.pos_hash[s] as usize;
            if v == 0 {
                return (s, None);
            }
            let i = v - 1;
            if self.ck[i] as usize == ck && self.idx[i] == idx {
                return (s, Some(i));
            }
            s = (s + 1) & (POS_SLOTS - 1);
        }
    }

    /// Chunk coordinates and chunk-local cell of a world position.
    #[inline]
    pub fn split(x: i32, y: i32, z: i32) -> (i32, i32, u16) {
        let (cx, cz) = (x >> 4, z >> 4);
        let (lx, lz) = ((x & 15) as u16, (z & 15) as u16);
        (cx, cz, ((y as u16) << 8) | (lz << 4) | lx)
    }

    /// Record that the cell at (x, y, z) in `dim` now holds `b`. False when the
    /// store has no room for a new entry (an existing one is always updated).
    pub fn record(&mut self, dim: u8, x: i32, y: i32, z: i32, b: u8) -> bool {
        if !(0..64).contains(&y) {
            return true;
        }
        let (cx, cz, idx) = Self::split(x, y, z);
        let ck = match self.find_chunk(cx, cz, dim) {
            Some(c) => c,
            None => {
                if self.n >= CAP {
                    return false;
                }
                match self.add_chunk(cx, cz, dim) {
                    Some(c) => c,
                    None => return false,
                }
            }
        };
        let (slot, found) = self.find_edit(ck, idx);
        if let Some(i) = found {
            self.blk[i] = b;
            return true;
        }
        if self.n >= CAP {
            return false;
        }
        let i = self.n;
        self.n += 1;
        self.idx[i] = idx;
        self.blk[i] = b;
        self.ck[i] = ck as u16;
        self.next[i] = self.head[ck];
        self.head[ck] = (i + 1) as u16;
        self.cnt[ck] += 1;
        self.pos_hash[slot] = (i + 1) as u16;
        true
    }

    /// The edit at a position, if there is one.
    pub fn get(&self, dim: u8, x: i32, y: i32, z: i32) -> Option<u8> {
        let (cx, cz, idx) = Self::split(x, y, z);
        let ck = self.find_chunk(cx, cz, dim)?;
        self.find_edit(ck, idx).1.map(|i| self.blk[i])
    }

    /// Call `f(local x, y, z, block)` for each edit of one chunk, newest first.
    pub fn for_chunk<F: FnMut(usize, usize, usize, u8)>(&self, dim: u8, cx: i32, cz: i32, mut f: F) {
        let Some(c) = self.find_chunk(cx, cz, dim) else {
            return;
        };
        let mut v = self.head[c] as usize;
        while v != 0 {
            let i = v - 1;
            let idx = self.idx[i] as usize;
            f(idx & 15, idx >> 8, (idx >> 4) & 15, self.blk[i]);
            v = self.next[i] as usize;
        }
    }

    /// Call `f(dim, chunk x, chunk z, edits in the chunk)` for each chunk, and
    /// `g(cell index, block)` for each of its edits: the card's order.
    pub fn for_each_chunk<F: FnMut(u8, i32, i32, usize), G: FnMut(u16, u8)>(&self, mut f: F, mut g: G) {
        let mut c = 0;
        while c < self.nck {
            f(self.dim[c], self.cx[c] as i32, self.cz[c] as i32, self.cnt[c] as usize);
            let mut v = self.head[c] as usize;
            while v != 0 {
                let i = v - 1;
                g(self.idx[i], self.blk[i]);
                v = self.next[i] as usize;
            }
            c += 1;
        }
    }

    /// Every edit as world coordinates: `f(dim, x, y, z, block)`.
    pub fn for_each<F: FnMut(u8, i32, i32, i32, u8)>(&self, mut f: F) {
        let mut c = 0;
        while c < self.nck {
            let mut v = self.head[c] as usize;
            while v != 0 {
                let i = v - 1;
                let idx = self.idx[i] as i32;
                f(
                    self.dim[c],
                    self.cx[c] as i32 * 16 + (idx & 15),
                    idx >> 8,
                    self.cz[c] as i32 * 16 + ((idx >> 4) & 15),
                    self.blk[i],
                );
                v = self.next[i] as usize;
            }
            c += 1;
        }
    }

    /// Bytes the card needs for the current edits.
    pub fn card_bytes(&self) -> usize {
        2 + self.nck * CHUNK_REC + self.n * EDIT_REC
    }

    /// Lay the edits out for the card in `out` (at least `card_bytes` long):
    /// a u16 chunk count, then per chunk its dimension u8, x i16, z i16 and
    /// edit count u16 followed by each edit's cell u16 and block u8, all
    /// little-endian. Returns the bytes written.
    pub fn write_card(&self, out: &mut [u8]) -> usize {
        let put16 = |o: &mut [u8], at: usize, v: u16| {
            o[at] = v as u8;
            o[at + 1] = (v >> 8) as u8;
        };
        put16(out, 0, self.nck as u16);
        let mut at = 2;
        let mut c = 0;
        while c < self.nck {
            out[at] = self.dim[c];
            put16(out, at + 1, self.cx[c] as u16);
            put16(out, at + 3, self.cz[c] as u16);
            put16(out, at + 5, self.cnt[c]);
            at += CHUNK_REC;
            let mut v = self.head[c] as usize;
            while v != 0 {
                let i = v - 1;
                put16(out, at, self.idx[i]);
                out[at + 2] = self.blk[i];
                at += EDIT_REC;
                v = self.next[i] as usize;
            }
            c += 1;
        }
        at
    }

    /// Read what `write_card` wrote, adding to this store (blocks go through
    /// `fix`, which maps a saved id to the cell it is now). None, with the store
    /// possibly part filled, if the bytes are short, name a cell outside a
    /// chunk, or do not fit.
    pub fn read_card<F: Fn(u8) -> u8>(&mut self, src: &[u8], fix: F) -> Option<usize> {
        let get16 = |at: usize| -> Option<u16> {
            Some(*src.get(at)? as u16 | (*src.get(at + 1)? as u16) << 8)
        };
        let chunks = get16(0)? as usize;
        if chunks > CHUNKS {
            return None;
        }
        let mut at = 2;
        let mut c = 0;
        while c < chunks {
            let dim = *src.get(at)?;
            let cx = get16(at + 1)? as i16 as i32;
            let cz = get16(at + 3)? as i16 as i32;
            let n = get16(at + 5)? as usize;
            at += CHUNK_REC;
            if dim > 2 || at + n * EDIT_REC > src.len() {
                return None;
            }
            let mut k = 0;
            while k < n {
                let idx = get16(at)?;
                let b = fix(src[at + 2]);
                at += EDIT_REC;
                if idx >= 64 * 256 {
                    return None;
                }
                let x = cx * 16 + (idx & 15) as i32;
                let z = cz * 16 + ((idx >> 4) & 15) as i32;
                if !self.record(dim, x, (idx >> 8) as i32, z, b) {
                    return None;
                }
                k += 1;
            }
            c += 1;
        }
        Some(at)
    }
}

/// The most bytes the edits can take on the card.
pub const CARD_MAX: usize = 2 + CHUNKS * CHUNK_REC + CAP * EDIT_REC;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_write_wins_and_counts_once() {
        let mut e = Box::new(Edits::new());
        assert!(e.record(0, 5, 10, 7, 3));
        assert!(e.record(0, 5, 10, 7, 9));
        assert_eq!(e.len(), 1);
        assert_eq!(e.get(0, 5, 10, 7), Some(9));
        assert_eq!(e.get(1, 5, 10, 7), None);
        assert_eq!(e.get(0, 5, 11, 7), None);
    }

    #[test]
    fn negative_coordinates_split_into_the_right_chunk() {
        let mut e = Box::new(Edits::new());
        e.record(0, -1, 0, -1, 4);
        e.record(0, -17, 3, 16, 5);
        let mut seen = Vec::new();
        e.for_chunk(0, -1, -1, |x, y, z, b| seen.push((x, y, z, b)));
        assert_eq!(seen, vec![(15, 0, 15, 4)]);
        let mut seen = Vec::new();
        e.for_chunk(0, -2, 1, |x, y, z, b| seen.push((x, y, z, b)));
        assert_eq!(seen, vec![(15, 3, 0, 5)]);
    }

    #[test]
    fn replay_reads_only_its_chunk() {
        let mut e = Box::new(Edits::new());
        for i in 0..100 {
            e.record(0, i, 5, 0, 7);
        }
        let mut n = 0;
        e.for_chunk(0, 0, 0, |_, _, _, _| n += 1);
        assert_eq!(n, 16);
        let mut n = 0;
        e.for_chunk(0, 6, 0, |_, _, _, _| n += 1);
        assert_eq!(n, 4);
    }

    #[test]
    fn it_holds_far_more_than_the_old_log_and_says_when_full() {
        let mut e = Box::new(Edits::new());
        // 4,000 edits spread over a few hundred chunks, then to the cap.
        for i in 0..CAP as i32 {
            let (x, z) = ((i % 160) * 3 - 200, (i / 160) * 2 - 90);
            assert!(e.record(0, x, 20 + i % 30, z, 1 + (i % 100) as u8), "edit {i}");
        }
        assert_eq!(e.len(), CAP);
        assert!(!e.record(0, 1000, 5, 1000, 1), "a new position needs room");
        // An existing position still updates.
        assert!(e.record(0, -200, 20, -90, 77));
        assert_eq!(e.get(0, -200, 20, -90), Some(77));
    }

    #[test]
    fn totals_match_between_the_chains_and_the_walk() {
        let mut e = Box::new(Edits::new());
        for i in 0..3000i32 {
            e.record((i % 3) as u8, i * 7 % 500 - 250, i % 64, i * 13 % 500 - 250, (i % 120) as u8);
        }
        let mut by_chunk = 0usize;
        e.for_each_chunk(|_, _, _, n| by_chunk += n, |_, _| {});
        let mut by_edit = 0usize;
        e.for_each(|_, _, _, _, _| by_edit += 1);
        assert_eq!(by_chunk, e.len());
        assert_eq!(by_edit, e.len());
        assert!(e.card_bytes() < e.len() * 8, "{} bytes for {} edits", e.card_bytes(), e.len());
    }

    #[test]
    fn the_card_layout_round_trips_and_rejects_damage() {
        let mut e = Box::new(Edits::new());
        for i in 0..2500i32 {
            e.record((i % 3) as u8, i * 5 % 400 - 200, i % 64, i * 11 % 400 - 200, (i % 200) as u8 + 1);
        }
        let mut buf = vec![0u8; CARD_MAX];
        let n = e.write_card(&mut buf);
        assert_eq!(n, e.card_bytes());
        let mut back = Box::new(Edits::new());
        assert_eq!(back.read_card(&buf[..n], |b| b), Some(n));
        assert_eq!(back.len(), e.len());
        e.for_each(|d, x, y, z, b| assert_eq!(back.get(d, x, y, z), Some(b)));
        // Short, and a cell outside the chunk.
        let mut again = Box::new(Edits::new());
        assert_eq!(again.read_card(&buf[..n - 1], |b| b), None);
        let mut bad = buf[..n].to_vec();
        bad[2 + 5] = 255;
        bad[2 + 6] = 255; // first chunk claims 65,535 edits
        assert_eq!(Box::new(Edits::new()).read_card(&bad, |b| b), None);
    }

    #[test]
    fn world_coordinates_round_trip_through_for_each() {
        let mut e = Box::new(Edits::new());
        e.record(2, -33, 63, 47, 200);
        let mut got = Vec::new();
        e.for_each(|d, x, y, z, b| got.push((d, x, y, z, b)));
        assert_eq!(got, vec![(2, -33, 63, 47, 200)]);
    }
}

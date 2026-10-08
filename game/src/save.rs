//! Memory-card save: persist the player + edited-block deltas to a PS1 memory
//! card through `psx-mc` (SIO0 transport + the standard PS1 filesystem). The
//! save is a named file ("VOXIDE" in the console's card manager), so it lives
//! alongside retail saves instead of clobbering raw frames like the old
//! hand-rolled driver did. Old raw-frame dev saves are orphaned by the switch.
//!
//! Payload: a versioned header (player with its enchant levels and food
//! pouch, hotbar, 16-bit inventory counts),
//! then every chest and furnace, then 8-byte edit-delta records. Version 1
//! saves (one-byte counts, no containers or hotbar) still load.

use crate::{
    Player, AIR, BLOCK_KINDS, CHEST, CHEST_D, CHEST_INV, CHEST_USED, CHEST_X, CHEST_Y, CHEST_Z,
    EDIT_B, EDIT_D, EDIT_N, EDIT_X, EDIT_Y, EDIT_Z, FURNACE, FURN_D, FURN_FUEL, FURN_IN, FURN_IN_N,
    FURN_OUT, FURN_OUT_N, FURN_PROG, FURN_USED, FURN_X, FURN_Y, FURN_Z, HOTBAR, HOTBAR_SEL,
    HOTBAR_VIS, INV, MAX_CHESTS, MAX_EDITS, MAX_FURNACES, PLACEABLE,
};
use psx_mc::{Block, Card, Error as McError, HardwareCard, Slot, FRAME_SIZE};

/// Version 1: the original layout. No version field; counts were one byte
/// each, and chests, furnaces and the hotbar layout were not saved at all.
/// Still read, never written.
const MAGIC_V1: [u8; 4] = *b"MCPX";
/// Version 2 on: this magic, then a u16 layout version. A new layout bumps
/// VERSION and teaches `layout` where its sections moved; every older version
/// still loads.
const MAGIC: [u8; 4] = *b"VOXS";
const VERSION: u16 = 11;
/// BIOS file names: region+product code + label, 20 ASCII chars max. A save
/// is written under the name the card does not hold yet and the old one is
/// deleted after it, so pulling the card mid-save leaves the last save
/// whole (Java keeps level.dat_old for the same reason). Older builds
/// only ever wrote the first name.
const FILE_NAME: &str = "BESLES-00000VOXIDE01";
const FILE_NAME_B: &str = "BESLES-00000VOXIDE02";
/// Human-readable label shown by the console's memory-card manager.
const FILE_TITLE: &str = "VOXIDE";

// Version 1 offsets. The header ends with the progression fields; edit
// records follow at V1_HDR, 8 bytes apiece (x i16, y i16, z i16, block u8, dim).
const V1_EDIT_COUNT: usize = 28 + BLOCK_KINDS;
const V1_PROGRESS: usize = 30 + BLOCK_KINDS;
const V1_HDR: usize = V1_PROGRESS + 9; // armor u8, efficiency u8, xp i32, 3 tool tiers

// Version 2 layout, all little-endian:
//   0 magic, 4 version u16, 6 reserved u16
//   8 player: x y z i32, yaw u16, pick u8, selected u8, health i32, food i32,
//     armor u8, efficiency u8, xp i32, axe u8, shovel u8, sword u8
//  41 hotbar selection u8, 42 hotbar slots [u8; 9], 51 pad
//  52 inventory counts [u16; BLOCK_KINDS], full 16-bit
// 308 chest count u8, furnace count u8, edit count u16
// 312 chests (CHEST_REC each), then furnaces (FURN_REC), then edits (EDIT_STRIDE)
//
// Version 3 inserts the player extras at 308 (sharpness u8, protection u8,
// pad u16, food_items i32), moving the counts to 316 and the records to 320.
// Version 4 adds the dimension to each chest and furnace record, as a byte
// (and a pad byte) after x y z; earlier saves' containers are overworld ones.
// Version 5 puts the player's dimension in byte 6 (was reserved, written 0),
// so a game saved in the Inferno loads there; earlier saves load overworld.
// Version 6 uses byte 7 (pad, written 0) for world flags: bit 0 = the void
// dragon is slain. No earlier version recorded the kill, so their dragon lives.
// Bit 1 (0.2.1 to 0.3.0, same layout version) marked a world the hidden cheat
// menu had been used in; it is written 0 and ignored now.
// Version 7 stores furnace fuel in half smelts (a log or plank burns 1.5
// items); older saves' fuel doubles on load.
// Version 8 appends the hunger state after the extras: saturation i32
// (hundredths) and exhaustion i32 (1/64,000ths). Older saves load with
// saturation 5.0 and no exhaustion, as a respawn does.
// Version 9 appends durability after that: tool uses left [u16; 4] (pick,
// axe, shovel, sword), armour pieces [u16; 4], bow / rod / flint and steel
// [u16; 3], pad u16. Older saves load with every owned tool and armour piece
// at full durability.
// Version 10 appends the world after that: the NEW WORLD seed i32, the
// respawn column (bx i32, bz i32), the day clock u32, then the tamed wolves:
// count u8, pad [u8; 3], and TAMED_CAP records of dimension u8, pad u8,
// health i16, x y z i32. Older saves load as the default world (seed 0) at
// its spawn point at sunrise, with no tamed wolves.
// Version 11 appends the weather and a save counter after the wolves:
// flags u8 (bit 0 raining, bit 1 thundering), pad [u8; 3], then the rain,
// thunder and clear-weather timers u32 in sim ticks (Java's rainTime,
// thunderTime and clearWeatherTime, three sim ticks to a game tick), then the
// save counter u32, which picks the newer file when both names are on the
// card. Older saves load in clear weather, as a level.dat without those
// fields does.
const OFF_HOTBAR_SEL: usize = 41;
const OFF_HOTBAR: usize = 42;
const OFF_INV: usize = 52;
const OFF_EXTRA: usize = OFF_INV + BLOCK_KINDS * 2;

/// Where a layout version keeps its sections.
#[derive(Copy, Clone)]
struct Layout {
    extras: bool,
    /// Byte 6 holds the player's dimension.
    dim: bool,
    /// Byte 7 holds the world flags.
    flags: bool,
    counts: usize,
    hdr: usize,
    /// Where x,y,z end in a container record: 8 with the dimension, else 6.
    pos: usize,
    /// Furnace fuel is stored in half smelts (logs and planks burn 1.5).
    half_fuel: bool,
    /// Saturation and exhaustion follow the extras.
    hunger: bool,
    /// Durability follows the hunger state.
    durability: bool,
    /// Seed, respawn, day and tamed wolves follow the durability.
    world: bool,
    /// The weather and the save counter follow the wolves.
    weather: bool,
}

const fn layout(version: u16) -> Layout {
    let extras = version >= 3;
    let hunger = version >= 8;
    let durability = version >= 9;
    let world = version >= 10;
    let weather = version >= 11;
    let counts = OFF_EXTRA
        + if extras { 8 } else { 0 }
        + if hunger { 8 } else { 0 }
        + if durability { 24 } else { 0 }
        + if world { WORLD_BYTES } else { 0 }
        + if weather { WEATHER_BYTES } else { 0 };
    Layout {
        extras,
        dim: version >= 5,
        flags: version >= 6,
        counts,
        hdr: counts + 4,
        pos: if version >= 4 { 8 } else { 6 },
        half_fuel: version >= 7,
        hunger,
        durability,
        world,
        weather,
    }
}

const OFF_WORLD: usize = OFF_EXTRA + 40;
const OFF_WOLVES: usize = OFF_WORLD + 20;
const WOLF_REC: usize = 16;
const WORLD_BYTES: usize = 20 + crate::mob::TAMED_CAP * WOLF_REC;
const OFF_WEATHER: usize = OFF_WORLD + WORLD_BYTES;
const WEATHER_BYTES: usize = 20;

/// What a save says about the world itself, for the caller to rebuild it.
pub struct WorldMeta {
    /// The NEW WORLD seed (0 = the default world).
    pub seed: i32,
    /// Respawn column; None (the world spawn) for saves before version 10.
    pub respawn: Option<(i32, i32)>,
    /// Day clock; 0 (sunrise, as a new world starts) before version 10.
    pub day: u32,
    /// Weather; None (clear) before version 11.
    pub weather: Option<crate::weather::Saved>,
}

impl WorldMeta {
    /// What a save that predates a field gets for it: the default world at
    /// its spawn point at sunrise in clear weather, which is what Java gives
    /// a level.dat missing SpawnX/Z, DayTime and the weather tags.
    const DEFAULT: WorldMeta = WorldMeta {
        seed: 0,
        respawn: None,
        day: 0,
        weather: None,
    };
}

/// An id read off the card, kept inside the item tables: a corrupt byte used
/// to index INV out of range and hang the game.
#[inline]
fn kind(b: u8) -> u8 {
    if (b as usize) < BLOCK_KINDS {
        b
    } else {
        AIR
    }
}

const CUR: Layout = layout(VERSION);
/// x y z i16 (and, from version 4, dimension u8 + pad), then every kind's
/// count as u16. Stored whole rather than sparse: it keeps the buffer bound
/// simple, and 16 full chests still fit two blocks.
const fn chest_rec(l: Layout) -> usize {
    l.pos + BLOCK_KINDS * 2
}
/// Position as above, then input u8, output u8, input count, fuel, output
/// count, progress u16.
const fn furn_rec(l: Layout) -> usize {
    l.pos + 10
}
const EDIT_STRIDE: usize = 8;
const MAX_PAYLOAD: usize =
    CUR.hdr + MAX_CHESTS * chest_rec(CUR) + MAX_FURNACES * furn_rec(CUR) + MAX_EDITS * EDIT_STRIDE;
const _: () = assert!(HOTBAR_VIS <= OFF_INV - OFF_HOTBAR);
const _: () = assert!(MAX_PAYLOAD >= V1_HDR + MAX_EDITS * EDIT_STRIDE);

// Serialization scratch (BSS, not stack: the payload is ~12.5 KiB at most).
static mut BUF: [u8; MAX_PAYLOAD] = [0; MAX_PAYLOAD];

fn card() -> Card<HardwareCard> {
    Card::new(HardwareCard::new(Slot::One))
}

#[inline]
fn put_i32(b: &mut [u8], o: usize, v: i32) {
    let u = v as u32;
    b[o] = u as u8;
    b[o + 1] = (u >> 8) as u8;
    b[o + 2] = (u >> 16) as u8;
    b[o + 3] = (u >> 24) as u8;
}
#[inline]
fn get_i32(b: &[u8], o: usize) -> i32 {
    (b[o] as u32 | (b[o + 1] as u32) << 8 | (b[o + 2] as u32) << 16 | (b[o + 3] as u32) << 24)
        as i32
}
#[inline]
fn put_u16(b: &mut [u8], o: usize, v: u16) {
    b[o] = v as u8;
    b[o + 1] = (v >> 8) as u8;
}
#[inline]
fn get_u16(b: &[u8], o: usize) -> u16 {
    b[o] as u16 | (b[o + 1] as u16) << 8
}
#[inline]
fn put_i16(b: &mut [u8], o: usize, v: i16) {
    put_u16(b, o, v as u16);
}
#[inline]
fn get_i16(b: &[u8], o: usize) -> i16 {
    get_u16(b, o) as i16
}

/// Non-destructive card probe: true if a card answers on port 1 and its
/// directory frame reads back clean (formatted or not). Replaces the old
/// scratch-frame write test, which scribbled on block 0.
pub fn selftest() -> bool {
    card().is_formatted().is_ok()
}

/// Lay player, inventory, hotbar, world, weather, chests, furnaces and edit
/// deltas out in BUF as one save payload; returns its length.
#[inline(never)]
#[optimize(size)] // card I/O dominates; keep the bytes
fn serialize(p: &Player, day: u32) -> usize {
    let buf = unsafe { &mut BUF[..] };
    buf[..4].copy_from_slice(&MAGIC);
    put_u16(buf, 4, VERSION);
    buf[6] = crate::world::dimension();
    // Bit 1 marked a world the cheats had touched (0.2.1 to 0.3.0); Java keeps
    // no such mark, so it is written 0 and ignored.
    buf[7] = crate::mob::dragon_slain() as u8;
    put_i32(buf, 8, p.x);
    put_i32(buf, 12, p.y);
    put_i32(buf, 16, p.z);
    put_u16(buf, 20, p.yaw);
    buf[22] = p.pick;
    buf[23] = p.selected;
    put_i32(buf, 24, p.health);
    put_i32(buf, 28, p.food);
    buf[32] = p.armor;
    buf[33] = p.efficiency;
    put_i32(buf, 34, p.xp);
    buf[38] = p.axe;
    buf[39] = p.shovel;
    buf[40] = p.sword;
    unsafe {
        buf[OFF_HOTBAR_SEL] = HOTBAR_SEL as u8;
        let mut h = 0;
        while h < HOTBAR_VIS {
            buf[OFF_HOTBAR + h] = HOTBAR[h];
            h += 1;
        }
        buf[OFF_HOTBAR + HOTBAR_VIS..OFF_INV].fill(0);
        let mut k = 0;
        while k < BLOCK_KINDS {
            put_u16(buf, OFF_INV + k * 2, INV[k]);
            k += 1;
        }
    }
    buf[OFF_EXTRA] = p.sharpness;
    buf[OFF_EXTRA + 1] = p.protection;
    put_u16(buf, OFF_EXTRA + 2, 0);
    put_i32(buf, OFF_EXTRA + 4, p.food_items);
    put_i32(buf, OFF_EXTRA + 8, p.saturation);
    put_i32(buf, OFF_EXTRA + 12, p.exhaustion);
    let mut k = 0;
    while k < 4 {
        put_u16(buf, OFF_EXTRA + 16 + k * 2, p.tool_dur[k]);
        put_u16(buf, OFF_EXTRA + 24 + k * 2, p.armor_dur[k]);
        k += 1;
    }
    k = 0;
    while k < 3 {
        put_u16(buf, OFF_EXTRA + 32 + k * 2, p.item_dur[k]);
        k += 1;
    }
    put_u16(buf, OFF_EXTRA + 38, 0);
    put_i32(buf, OFF_WORLD, crate::world::seed_extra());
    unsafe {
        put_i32(buf, OFF_WORLD + 4, crate::RESPAWN_BX);
        put_i32(buf, OFF_WORLD + 8, crate::RESPAWN_BZ);
    }
    put_i32(buf, OFF_WORLD + 12, day as i32);
    let mut wolves = [crate::mob::NO_PARKED; crate::mob::TAMED_CAP];
    let nw = crate::mob::tamed_list(&mut wolves);
    buf[OFF_WORLD + 16] = nw as u8;
    buf[OFF_WORLD + 17..OFF_WORLD + 20].fill(0);
    let mut w = 0;
    while w < crate::mob::TAMED_CAP {
        let o = OFF_WOLVES + w * WOLF_REC;
        let r = wolves[w];
        buf[o] = r.dim;
        buf[o + 1] = 0;
        put_i16(buf, o + 2, r.hp);
        put_i32(buf, o + 4, r.x);
        put_i32(buf, o + 8, r.y);
        put_i32(buf, o + 12, r.z);
        w += 1;
    }
    let ws = crate::weather::saved();
    buf[OFF_WEATHER] = ws.raining as u8 | (ws.thundering as u8) << 1;
    buf[OFF_WEATHER + 1..OFF_WEATHER + 4].fill(0);
    put_i32(buf, OFF_WEATHER + 4, ws.rain_t as i32);
    put_i32(buf, OFF_WEATHER + 8, ws.thunder_t as i32);
    put_i32(buf, OFF_WEATHER + 12, ws.clear_t as i32);
    put_i32(
        buf,
        OFF_WEATHER + 16,
        unsafe { SAVE_SEQ }.wrapping_add(1) as i32,
    );

    let mut off = CUR.hdr;
    let mut chests = 0u8;
    let mut i = 0;
    while i < MAX_CHESTS {
        unsafe {
            if CHEST_USED[i] {
                put_i16(buf, off, CHEST_X[i] as i16);
                put_i16(buf, off + 2, CHEST_Y[i] as i16);
                put_i16(buf, off + 4, CHEST_Z[i] as i16);
                buf[off + 6] = CHEST_D[i];
                buf[off + 7] = 0;
                let mut k = 0;
                while k < BLOCK_KINDS {
                    put_u16(buf, off + CUR.pos + k * 2, CHEST_INV[i][k]);
                    k += 1;
                }
                off += chest_rec(CUR);
                chests += 1;
            }
        }
        i += 1;
    }
    let mut furnaces = 0u8;
    let mut i = 0;
    while i < MAX_FURNACES {
        unsafe {
            if FURN_USED[i] {
                put_i16(buf, off, FURN_X[i] as i16);
                put_i16(buf, off + 2, FURN_Y[i] as i16);
                put_i16(buf, off + 4, FURN_Z[i] as i16);
                buf[off + 6] = FURN_D[i];
                buf[off + 7] = 0;
                let o = off + CUR.pos;
                buf[o] = FURN_IN[i];
                buf[o + 1] = FURN_OUT[i];
                put_u16(buf, o + 2, FURN_IN_N[i]);
                put_u16(buf, o + 4, FURN_FUEL[i]);
                put_u16(buf, o + 6, FURN_OUT_N[i]);
                put_u16(buf, o + 8, FURN_PROG[i]);
                off += furn_rec(CUR);
                furnaces += 1;
            }
        }
        i += 1;
    }
    let n = unsafe { EDIT_N }.min(MAX_EDITS);
    buf[CUR.counts] = chests;
    buf[CUR.counts + 1] = furnaces;
    put_u16(buf, CUR.counts + 2, n as u16);
    let mut idx = 0;
    while idx < n {
        unsafe {
            put_i16(buf, off, EDIT_X[idx]);
            put_i16(buf, off + 2, EDIT_Y[idx]);
            put_i16(buf, off + 4, EDIT_Z[idx]);
            buf[off + 6] = EDIT_B[idx];
            buf[off + 7] = EDIT_D[idx];
        }
        off += EDIT_STRIDE;
        idx += 1;
    }

    off
}

// ---- Saving without stopping the frame ------------------------------------
//
// A card frame write is about 4 ms of transfer and then two video periods
// the card needs to commit (psx-mc's POST_WRITE_SETTLE_SPINS), and a save is
// tens of frames: written in one call, 0.3.0 held the screen still for 44
// vblanks on a fresh world and a few seconds on a big one. So the save is
// planned first, entirely in RAM: psx-mc runs the format, write and delete
// against JOURNAL, a card that serves the directory from a cache read one
// frame per game frame, and records every frame it would write. Then one
// recorded frame goes to the card per game frame, and the menu keeps
// drawing, polling and animating its progress bar between them.

/// Frames a plan may write: a format (64), the largest save (2 title + 102
/// data frames for 13,032 bytes with its container header, + 2 directory),
/// freeing an overwritten copy (2) and deleting the old one (2): 174.
const JOURNAL_CAP: usize = 176;
/// Directory frames 0..=15, read ahead one a frame.
const DIR_FRAMES: usize = 16;

static mut J_FRAME: [u16; JOURNAL_CAP] = [0; JOURNAL_CAP];
static mut J_DATA: [[u8; FRAME_SIZE]; JOURNAL_CAP] = [[0; FRAME_SIZE]; JOURNAL_CAP];
static mut J_N: usize = 0;
static mut DIR: [[u8; FRAME_SIZE]; DIR_FRAMES] = [[0; FRAME_SIZE]; DIR_FRAMES];

/// The card a save is planned on: reads come from the frames it has already
/// recorded, then from the directory cache; writes are recorded, not sent.
struct Journal;

impl Block for Journal {
    fn read_frame(&mut self, frame: u16, out: &mut [u8; FRAME_SIZE]) -> psx_mc::Result<()> {
        unsafe {
            let mut i = J_N;
            while i > 0 {
                i -= 1;
                if J_FRAME[i] == frame {
                    *out = J_DATA[i];
                    return Ok(());
                }
            }
            if (frame as usize) < DIR_FRAMES {
                *out = DIR[frame as usize];
                return Ok(());
            }
        }
        // The plan never reads past the directory; if it did, ask the card.
        HardwareCard::new(Slot::One).read_frame(frame, out)
    }

    fn write_frame(&mut self, frame: u16, data: &[u8; FRAME_SIZE]) -> psx_mc::Result<()> {
        unsafe {
            if J_N == JOURNAL_CAP {
                return Err(McError::NoSpace);
            }
            J_FRAME[J_N] = frame;
            J_DATA[J_N] = *data;
            J_N += 1;
        }
        Ok(())
    }
}

/// Where a save in progress has got to.
#[derive(Copy, Clone, PartialEq, Eq)]
enum Phase {
    Idle,
    /// Reading directory frame n into DIR.
    Probe(usize),
    Plan,
    /// Sending recorded frame n.
    Write(usize),
}

static mut PHASE: Phase = Phase::Idle;
static mut PAYLOAD: usize = 0;
/// The save counter of the newest save this session wrote or loaded.
static mut SAVE_SEQ: u32 = 0;
/// Which name holds this session's save (0 = FILE_NAME, 1 = FILE_NAME_B),
/// once known; the next save goes under the other.
static mut CUR_SLOT: Option<u8> = None;
static mut TARGET: u8 = 0;
/// The player has been told the card is blank and saved again anyway.
static mut FORMAT_OK: bool = false;

/// What a finished save reports.
pub enum Outcome {
    Saved,
    /// The card has no directory: the next save, asked again, formats it.
    NeedFormat,
    NoCard,
    Full,
    Removed,
    Failed,
}

/// Start a save: lay the payload out now (it is a snapshot; the world may
/// change while it is written) and begin reading the card's directory.
pub fn begin(p: &Player, day: u32) {
    unsafe {
        PAYLOAD = serialize(p, day);
        PHASE = Phase::Probe(0);
    }
}

pub fn busy() -> bool {
    unsafe { PHASE != Phase::Idle }
}

/// Progress of the save in progress, 0..=256.
pub fn progress() -> i32 {
    unsafe {
        match PHASE {
            Phase::Idle => 0,
            Phase::Probe(n) => (n * 16 / DIR_FRAMES) as i32,
            Phase::Plan => 16,
            Phase::Write(n) => 16 + (n * 240 / J_N.max(1)) as i32,
        }
    }
}

fn fail(o: Outcome) -> Option<Outcome> {
    unsafe { PHASE = Phase::Idle };
    Some(o)
}

/// One step of the save, once a game frame: one card frame read or written.
/// Some(outcome) when it has finished.
#[inline(never)]
#[optimize(size)] // card I/O dominates; keep the bytes
pub fn pump() -> Option<Outcome> {
    let phase = unsafe { PHASE };
    match phase {
        Phase::Idle => None,
        Phase::Probe(n) => {
            let r = HardwareCard::new(Slot::One).read_frame(n as u16, unsafe { &mut DIR[n] });
            match r {
                Ok(()) => {}
                Err(McError::NoCard) => return fail(Outcome::NoCard),
                Err(_) => return fail(Outcome::Failed),
            }
            unsafe {
                PHASE = if n + 1 < DIR_FRAMES {
                    Phase::Probe(n + 1)
                } else {
                    Phase::Plan
                };
            }
            None
        }
        Phase::Plan => match plan() {
            Ok(()) => {
                unsafe { PHASE = Phase::Write(0) };
                None
            }
            Err(McError::NoSpace) => fail(Outcome::Full),
            Err(McError::NotFormatted) => fail(Outcome::NeedFormat),
            Err(_) => fail(Outcome::Failed),
        },
        Phase::Write(n) => {
            #[cfg(feature = "save-pull")]
            {
                static mut PULLED: bool = false;
                if n == 3 && !unsafe { PULLED } {
                    unsafe { PULLED = true };
                    return fail(Outcome::Removed);
                }
            }
            let (frame, data) = unsafe { (J_FRAME[n], &J_DATA[n]) };
            match HardwareCard::new(Slot::One).write_frame(frame, data) {
                Ok(()) => {}
                Err(McError::NoCard) => return fail(Outcome::Removed),
                Err(_) => return fail(Outcome::Failed),
            }
            unsafe {
                if n + 1 < J_N {
                    PHASE = Phase::Write(n + 1);
                    None
                } else {
                    PHASE = Phase::Idle;
                    FORMAT_OK = false;
                    SAVE_SEQ = SAVE_SEQ.wrapping_add(1);
                    CUR_SLOT = Some(TARGET);
                    Some(Outcome::Saved)
                }
            }
        }
    }
}

const NAMES: [&str; 2] = [FILE_NAME, FILE_NAME_B];

/// Plan the card writes in RAM: format a blank card, write the payload under
/// the name not in use, then delete the old file. When the card has no room
/// for a second copy, overwrite the old file in place instead: not safe
/// against pulling the card, but it still saves on a nearly full card.
#[inline(never)]
#[optimize(size)]
fn plan() -> psx_mc::Result<()> {
    unsafe { J_N = 0 };
    let mut card = Card::new(Journal);
    if !card.is_formatted()? {
        // A card with no directory may be blank or may hold something this
        // game cannot read; formatting erases it, so ask first, as the BIOS does.
        if !unsafe { FORMAT_OK } {
            unsafe { FORMAT_OK = true };
            return Err(McError::NotFormatted);
        }
        card.format()?;
    }
    let mut list = [psx_mc::Entry {
        name: [0; psx_mc::MAX_NAME + 1],
        name_len: 0,
        blocks: 0,
    }; psx_mc::DATA_BLOCKS];
    let n = card.list(&mut list)?;
    let mut have = [false; 2];
    let mut i = 0;
    while i < n {
        let mut s = 0;
        while s < 2 {
            if list[i].name() == NAMES[s] {
                have[s] = true;
            }
            s += 1;
        }
        i += 1;
    }
    // The file this session's save lives in, if any: the one it loaded or
    // last wrote, else the one on the card (the first if both are).
    let cur = match unsafe { CUR_SLOT } {
        Some(s) if have[s as usize] => Some(s),
        _ if have[0] => Some(0),
        _ if have[1] => Some(1),
        _ => None,
    };
    let mut target = cur.map_or(0, |c| c ^ 1);
    let payload = unsafe { &BUF[..PAYLOAD] };
    let mark = unsafe { J_N };
    match card.write(NAMES[target as usize], FILE_TITLE, payload) {
        Ok(()) => {
            // Both old copies go once the new one is down, the stray one a
            // pulled card may have left behind included.
            let mut s = 0;
            while s < 2 {
                if s != target as usize && have[s] {
                    card.delete(NAMES[s])?;
                }
                s += 1;
            }
        }
        Err(McError::NoSpace) if cur.is_some() => {
            unsafe { J_N = mark };
            target = cur.unwrap_or(0);
            card.write(NAMES[target as usize], FILE_TITLE, payload)?;
        }
        Err(e) => return Err(e),
    }
    unsafe { TARGET = target };
    Ok(())
}

/// The save counter of a payload in BUF, if it is a save this build reads
/// (0 for versions before 11, which had none).
#[inline(never)]
#[optimize(size)] // card I/O dominates; keep the bytes
fn readable_seq(len: usize) -> Option<u32> {
    let buf = unsafe { &BUF[..len] };
    if len >= V1_HDR && buf[..4] == MAGIC_V1 {
        return Some(0);
    }
    if len < 6 || buf[..4] != MAGIC || !(2..=VERSION).contains(&get_u16(buf, 4)) {
        return None;
    }
    let l = layout(get_u16(buf, 4));
    if len < l.hdr {
        return None;
    }
    Some(if l.weather {
        get_i32(buf, OFF_WEATHER + 16) as u32
    } else {
        0
    })
}

/// Read one of the two names into BUF: its length and counter, if readable.
#[inline(never)]
#[optimize(size)] // card I/O dominates; keep the bytes
fn read_slot(s: usize) -> Option<(usize, u32)> {
    let buf = unsafe { &mut BUF[..] };
    let len = card().read(NAMES[s], buf).ok()?.min(buf.len());
    readable_seq(len).map(|q| (len, q))
}

/// Load a save from the card: the dimension the player saved in, or None,
/// touching nothing, if there is no save it understands. Edits land in the
/// EDIT log; the caller enters that dimension, then calls [`apply_edits`] to
/// replay them into the world (raw sets, then one remesh).
///
/// With both names on the card (a save cut short by pulling the card, before
/// the old file was deleted) the higher save counter wins; a half-written
/// file does not read back, so the old one is used.
///
/// Saves before version 5 load in the overworld. A version 1 save's edit log
/// does know each edit's dimension, but not where the player stood, so it
/// cannot say which dimension the player was in without guessing.
#[inline(never)]
#[optimize(size)] // card I/O dominates; keep the bytes
pub fn load(p: &mut Player) -> Option<(u8, WorldMeta)> {
    let b = read_slot(1);
    let a = read_slot(0);
    let (slot, len, seq) = match (a, b) {
        (Some(a), Some(b)) if b.1 > a.1 => (1, read_slot(1)?.0, b.1),
        (Some(a), _) => (0, a.0, a.1),
        (None, Some(b)) => (1, read_slot(1)?.0, b.1),
        (None, None) => return None,
    };
    let buf = unsafe { &mut BUF[..] };
    let out = if len >= V1_HDR && buf[..4] == MAGIC_V1 {
        load_v1(p, &buf[..len]);
        crate::mob::set_dragon_slain(false);
        crate::mob::set_tamed(&[]);
        Some((crate::world::DIM_OVERWORLD, WorldMeta::DEFAULT))
    } else {
        let l = layout(get_u16(buf, 4));
        if !load_versioned(p, &buf[..len], l) {
            return None;
        }
        crate::mob::set_dragon_slain(l.flags && buf[7] & 1 != 0);
        let d = buf[6];
        let dim = if l.dim && d <= crate::world::DIM_VOID {
            d
        } else {
            crate::world::DIM_OVERWORLD
        };
        let mut meta = WorldMeta::DEFAULT;
        if l.world {
            let mut wolves = [crate::mob::NO_PARKED; crate::mob::TAMED_CAP];
            let nw = (buf[OFF_WORLD + 16] as usize).min(crate::mob::TAMED_CAP);
            let mut w = 0;
            while w < nw {
                let o = OFF_WOLVES + w * WOLF_REC;
                wolves[w] = crate::mob::Parked {
                    used: true,
                    dim: buf[o].min(crate::world::DIM_VOID),
                    hp: get_i16(buf, o + 2).max(1),
                    x: get_i32(buf, o + 4),
                    y: get_i32(buf, o + 8),
                    z: get_i32(buf, o + 12),
                };
                w += 1;
            }
            crate::mob::set_tamed(&wolves[..nw]);
            meta.seed = get_i32(buf, OFF_WORLD);
            meta.respawn = Some((get_i32(buf, OFF_WORLD + 4), get_i32(buf, OFF_WORLD + 8)));
            meta.day = get_i32(buf, OFF_WORLD + 12) as u32;
        } else {
            crate::mob::set_tamed(&[]);
        }
        if l.weather {
            let f = buf[OFF_WEATHER];
            meta.weather = Some(crate::weather::Saved {
                raining: f & 1 != 0,
                thundering: f & 2 != 0,
                rain_t: get_i32(buf, OFF_WEATHER + 4) as u32,
                thunder_t: get_i32(buf, OFF_WEATHER + 8) as u32,
                clear_t: get_i32(buf, OFF_WEATHER + 12) as u32,
            });
        }
        Some((dim, meta))
    };
    unsafe {
        SAVE_SEQ = seq;
        CUR_SLOT = Some(slot);
    }
    out
}

#[inline(never)]
#[optimize(size)] // card I/O dominates; keep the bytes
fn load_v1(p: &mut Player, buf: &[u8]) {
    p.x = get_i32(buf, 4);
    p.y = get_i32(buf, 8);
    p.z = get_i32(buf, 12);
    p.yaw = get_u16(buf, 16);
    p.pick = buf[18];
    p.selected = kind(buf[19]);
    p.health = get_i32(buf, 20);
    p.food = get_i32(buf, 24);
    let mut k = 0;
    while k < BLOCK_KINDS {
        unsafe { INV[k] = buf[28 + k] as u16 };
        k += 1;
    }
    p.armor = buf[V1_PROGRESS];
    p.efficiency = buf[V1_PROGRESS + 1];
    p.xp = get_i32(buf, V1_PROGRESS + 2);
    p.axe = buf[V1_PROGRESS + 6];
    p.shovel = buf[V1_PROGRESS + 7];
    p.sword = buf[V1_PROGRESS + 8];
    let n = (get_u16(buf, V1_EDIT_COUNT) as usize)
        .min(MAX_EDITS)
        .min((buf.len() - V1_HDR) / EDIT_STRIDE);
    read_edits(buf, V1_HDR, n);
    // A version 1 save has no container state: its chests and furnaces come
    // back EMPTY but usable (before, they were unregistered after a load and
    // would not even open), and the hotbar is laid out again from what is
    // owned, the item in hand first.
    clear_containers();
    let mut i = 0;
    while i < n {
        // Each edit knows its dimension, so a rebuilt container does too.
        let (x, y, z, b, d) = unsafe {
            (
                EDIT_X[i] as i32,
                EDIT_Y[i] as i32,
                EDIT_Z[i] as i32,
                EDIT_B[i],
                EDIT_D[i],
            )
        };
        if b == CHEST {
            crate::chest_register_in(x, y, z, d);
        } else if b == FURNACE {
            crate::furn_register_in(x, y, z, d);
        }
        i += 1;
    }
    unsafe {
        HOTBAR = [AIR; HOTBAR_VIS];
        HOTBAR_SEL = 0;
        if INV[p.selected as usize] > 0 {
            crate::hotbar_add(p.selected);
        }
    }
    let mut i = 0;
    while i < PLACEABLE.len() {
        if unsafe { INV[PLACEABLE[i] as usize] } > 0 {
            crate::hotbar_add(PLACEABLE[i]);
        }
        i += 1;
    }
}

#[inline(never)]
#[optimize(size)] // card I/O dominates; keep the bytes
fn load_versioned(p: &mut Player, buf: &[u8], l: Layout) -> bool {
    let chests = buf[l.counts] as usize;
    let furnaces = buf[l.counts + 1] as usize;
    let edits = get_u16(buf, l.counts + 2) as usize;
    // Refuse a truncated or out-of-range file before changing any state.
    if chests > MAX_CHESTS
        || furnaces > MAX_FURNACES
        || edits > MAX_EDITS
        || buf.len() < l.hdr + chests * chest_rec(l) + furnaces * furn_rec(l) + edits * EDIT_STRIDE
    {
        return false;
    }
    p.x = get_i32(buf, 8);
    p.y = get_i32(buf, 12);
    p.z = get_i32(buf, 16);
    p.yaw = get_u16(buf, 20);
    p.pick = buf[22];
    p.selected = kind(buf[23]);
    p.health = get_i32(buf, 24);
    p.food = get_i32(buf, 28).clamp(0, crate::MAX_FOOD);
    p.armor = buf[32];
    p.efficiency = buf[33];
    p.xp = get_i32(buf, 34);
    p.axe = buf[38];
    p.shovel = buf[39];
    p.sword = buf[40];
    // Saves before version 3 did not carry these; the session keeps its own.
    if l.extras {
        p.sharpness = buf[OFF_EXTRA];
        p.protection = buf[OFF_EXTRA + 1];
        p.food_items = get_i32(buf, OFF_EXTRA + 4);
    }
    if l.hunger {
        p.saturation = get_i32(buf, OFF_EXTRA + 8).clamp(0, p.food * 100);
        p.exhaustion = get_i32(buf, OFF_EXTRA + 12).max(0);
    } else {
        p.saturation = 500.min(p.food * 100);
        p.exhaustion = 0;
    }
    if l.durability {
        let mut k = 0;
        while k < 4 {
            p.tool_dur[k] = get_u16(buf, OFF_EXTRA + 16 + k * 2);
            p.armor_dur[k] = get_u16(buf, OFF_EXTRA + 24 + k * 2);
            k += 1;
        }
        k = 0;
        while k < 3 {
            p.item_dur[k] = get_u16(buf, OFF_EXTRA + 32 + k * 2);
            k += 1;
        }
    } else {
        crate::full_durability(p);
    }
    unsafe {
        HOTBAR_SEL = (buf[OFF_HOTBAR_SEL] as usize).min(HOTBAR_VIS - 1);
        let mut h = 0;
        while h < HOTBAR_VIS {
            HOTBAR[h] = kind(buf[OFF_HOTBAR + h]);
            h += 1;
        }
        let mut k = 0;
        while k < BLOCK_KINDS {
            INV[k] = get_u16(buf, OFF_INV + k * 2);
            k += 1;
        }
    }
    clear_containers();
    let mut off = l.hdr;
    let mut i = 0;
    while i < chests {
        unsafe {
            CHEST_USED[i] = true;
            CHEST_X[i] = get_i16(buf, off) as i32;
            CHEST_Y[i] = get_i16(buf, off + 2) as i32;
            CHEST_Z[i] = get_i16(buf, off + 4) as i32;
            // Before version 4 every container was an overworld one.
            CHEST_D[i] = if l.pos > 6 { buf[off + 6] } else { 0 };
            let mut k = 0;
            while k < BLOCK_KINDS {
                CHEST_INV[i][k] = get_u16(buf, off + l.pos + k * 2);
                k += 1;
            }
        }
        off += chest_rec(l);
        i += 1;
    }
    let mut i = 0;
    while i < furnaces {
        unsafe {
            FURN_USED[i] = true;
            FURN_X[i] = get_i16(buf, off) as i32;
            FURN_Y[i] = get_i16(buf, off + 2) as i32;
            FURN_Z[i] = get_i16(buf, off + 4) as i32;
            FURN_D[i] = if l.pos > 6 { buf[off + 6] } else { 0 };
            let o = off + l.pos;
            FURN_IN[i] = kind(buf[o]);
            FURN_OUT[i] = kind(buf[o + 1]);
            FURN_IN_N[i] = get_u16(buf, o + 2);
            FURN_FUEL[i] = get_u16(buf, o + 4);
            if !l.half_fuel {
                FURN_FUEL[i] = FURN_FUEL[i].saturating_mul(2);
            }
            FURN_OUT_N[i] = get_u16(buf, o + 6);
            FURN_PROG[i] = get_u16(buf, o + 8);
        }
        off += furn_rec(l);
        i += 1;
    }
    read_edits(buf, off, edits);
    true
}

#[inline(never)]
#[optimize(size)] // card I/O dominates; keep the bytes
fn read_edits(buf: &[u8], base: usize, n: usize) {
    let mut idx = 0;
    while idx < n {
        let off = base + idx * EDIT_STRIDE;
        unsafe {
            EDIT_X[idx] = get_i16(buf, off);
            EDIT_Y[idx] = get_i16(buf, off + 2);
            EDIT_Z[idx] = get_i16(buf, off + 4);
            EDIT_B[idx] = kind(buf[off + 6]);
            // The record's last byte was pad before dimensions existed; it
            // reads back 0 (= overworld) on saves written before them.
            EDIT_D[idx] = buf[off + 7];
        }
        idx += 1;
    }
    unsafe { EDIT_N = n };
}

/// Forget every chest and furnace. Loading replaces the session's containers
/// rather than merging with them: the old code kept them, so a chest placed
/// before a load stayed registered at a spot the loaded world may not have.
#[inline(never)]
#[optimize(size)] // card I/O dominates; keep the bytes
fn clear_containers() {
    unsafe {
        CHEST_USED = [false; MAX_CHESTS];
        FURN_USED = [false; MAX_FURNACES];
    }
}

/// Apply the loaded edit log to the world (raw sets + one remesh per chunk).
pub fn apply_edits() {
    let n = unsafe { EDIT_N };
    let dim = crate::world::dimension();
    let mut i = 0;
    while i < n {
        let (x, y, z, b, d) = unsafe {
            (
                EDIT_X[i] as i32,
                EDIT_Y[i] as i32,
                EDIT_Z[i] as i32,
                EDIT_B[i],
                EDIT_D[i],
            )
        };
        let _ = AIR;
        if d == dim {
            crate::world::set_raw_pub(x, y, z, b);
        }
        i += 1;
    }
    crate::world::remesh_loaded();
}

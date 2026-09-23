//! Memory-card save: persist the player + edited-block deltas to a PS1 memory
//! card through `psx-mc` (SIO0 transport + the standard PS1 filesystem). The
//! save is a named file ("VOXIDE" in the console's card manager), so it lives
//! alongside retail saves instead of clobbering raw frames like the old
//! hand-rolled driver did. Old raw-frame dev saves are orphaned by the switch.
//!
//! Payload: a versioned header (player, hotbar, 16-bit inventory counts),
//! then every chest and furnace, then 8-byte edit-delta records. Version 1
//! saves (one-byte counts, no containers or hotbar) still load.

use crate::{
    Player, AIR, BLOCK_KINDS, CHEST, CHEST_INV, CHEST_USED, CHEST_X, CHEST_Y, CHEST_Z, EDIT_B,
    EDIT_D, EDIT_N, EDIT_X, EDIT_Y, EDIT_Z, FURNACE, FURN_FUEL, FURN_IN, FURN_IN_N, FURN_OUT,
    FURN_OUT_N, FURN_PROG, FURN_USED, FURN_X, FURN_Y, FURN_Z, HOTBAR, HOTBAR_SEL, HOTBAR_VIS, INV,
    MAX_CHESTS, MAX_EDITS, MAX_FURNACES, PLACEABLE,
};
use psx_mc::{Card, HardwareCard, Slot};

/// Version 1: the original layout. No version field; counts were one byte
/// each, and chests, furnaces and the hotbar layout were not saved at all.
/// Still read, never written.
const MAGIC_V1: [u8; 4] = *b"MCPX";
/// Version 2 on: this magic, then a u16 layout version. A future layout bumps
/// VERSION and adds an arm to `load` instead of a new magic.
const MAGIC: [u8; 4] = *b"VOXS";
const VERSION: u16 = 2;
/// BIOS file name: region+product code + label, 20 ASCII chars max.
const FILE_NAME: &str = "BESLES-00000VOXIDE01";
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
const OFF_HOTBAR_SEL: usize = 41;
const OFF_HOTBAR: usize = 42;
const OFF_INV: usize = 52;
const OFF_COUNTS: usize = OFF_INV + BLOCK_KINDS * 2;
const V2_HDR: usize = OFF_COUNTS + 4;
/// x y z i16, then every kind's count as u16. Stored whole rather than sparse:
/// it keeps the buffer bound simple, and 16 full chests still fit two blocks.
const CHEST_REC: usize = 6 + BLOCK_KINDS * 2;
/// x y z i16, input u8, output u8, input count, fuel, output count, progress u16.
const FURN_REC: usize = 16;
const EDIT_STRIDE: usize = 8;
const MAX_PAYLOAD: usize =
    V2_HDR + MAX_CHESTS * CHEST_REC + MAX_FURNACES * FURN_REC + MAX_EDITS * EDIT_STRIDE;
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

/// Persist player, inventory, hotbar, chests, furnaces and edit deltas as one
/// card file. Formats a blank card first; overwrites any previous VOXIDE save.
#[inline(never)]
pub fn save(p: &Player) -> bool {
    let buf = unsafe { &mut BUF[..] };
    buf[..4].copy_from_slice(&MAGIC);
    put_u16(buf, 4, VERSION);
    put_u16(buf, 6, 0);
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

    let mut off = V2_HDR;
    let mut chests = 0u8;
    let mut i = 0;
    while i < MAX_CHESTS {
        unsafe {
            if CHEST_USED[i] {
                put_i16(buf, off, CHEST_X[i] as i16);
                put_i16(buf, off + 2, CHEST_Y[i] as i16);
                put_i16(buf, off + 4, CHEST_Z[i] as i16);
                let mut k = 0;
                while k < BLOCK_KINDS {
                    put_u16(buf, off + 6 + k * 2, CHEST_INV[i][k]);
                    k += 1;
                }
                off += CHEST_REC;
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
                buf[off + 6] = FURN_IN[i];
                buf[off + 7] = FURN_OUT[i];
                put_u16(buf, off + 8, FURN_IN_N[i]);
                put_u16(buf, off + 10, FURN_FUEL[i]);
                put_u16(buf, off + 12, FURN_OUT_N[i]);
                put_u16(buf, off + 14, FURN_PROG[i]);
                off += FURN_REC;
                furnaces += 1;
            }
        }
        i += 1;
    }
    let n = unsafe { EDIT_N }.min(MAX_EDITS);
    buf[OFF_COUNTS] = chests;
    buf[OFF_COUNTS + 1] = furnaces;
    put_u16(buf, OFF_COUNTS + 2, n as u16);
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

    let mut card = card();
    match card.is_formatted() {
        Ok(true) => {}
        Ok(false) => {
            if card.format().is_err() {
                return false;
            }
        }
        Err(_) => return false,
    }
    card.write(FILE_NAME, FILE_TITLE, &buf[..off]).is_ok()
}

/// Load a save from the card. Returns false, touching nothing, if there is no
/// save it understands. Edits land in the EDIT log; call [`apply_edits`] to
/// replay them into the world (raw sets, then one remesh).
#[inline(never)]
pub fn load(p: &mut Player) -> bool {
    let buf = unsafe { &mut BUF[..] };
    let len = match card().read(FILE_NAME, buf) {
        Ok(len) => len.min(buf.len()),
        Err(_) => return false,
    };
    if len >= V1_HDR && buf[..4] == MAGIC_V1 {
        load_v1(p, &buf[..len]);
        true
    } else if len >= V2_HDR && buf[..4] == MAGIC && get_u16(buf, 4) == VERSION {
        load_v2(p, &buf[..len])
    } else {
        false
    }
}

#[inline(never)]
fn load_v1(p: &mut Player, buf: &[u8]) {
    p.x = get_i32(buf, 4);
    p.y = get_i32(buf, 8);
    p.z = get_i32(buf, 12);
    p.yaw = get_u16(buf, 16);
    p.pick = buf[18];
    p.selected = buf[19];
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
        let (x, y, z, b) = unsafe {
            (EDIT_X[i] as i32, EDIT_Y[i] as i32, EDIT_Z[i] as i32, EDIT_B[i])
        };
        if b == CHEST {
            crate::chest_register(x, y, z);
        } else if b == FURNACE {
            crate::furn_register(x, y, z);
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
fn load_v2(p: &mut Player, buf: &[u8]) -> bool {
    let chests = buf[OFF_COUNTS] as usize;
    let furnaces = buf[OFF_COUNTS + 1] as usize;
    let edits = get_u16(buf, OFF_COUNTS + 2) as usize;
    // Refuse a truncated or out-of-range file before changing any state.
    if chests > MAX_CHESTS
        || furnaces > MAX_FURNACES
        || edits > MAX_EDITS
        || buf.len() < V2_HDR + chests * CHEST_REC + furnaces * FURN_REC + edits * EDIT_STRIDE
    {
        return false;
    }
    p.x = get_i32(buf, 8);
    p.y = get_i32(buf, 12);
    p.z = get_i32(buf, 16);
    p.yaw = get_u16(buf, 20);
    p.pick = buf[22];
    p.selected = buf[23];
    p.health = get_i32(buf, 24);
    p.food = get_i32(buf, 28);
    p.armor = buf[32];
    p.efficiency = buf[33];
    p.xp = get_i32(buf, 34);
    p.axe = buf[38];
    p.shovel = buf[39];
    p.sword = buf[40];
    unsafe {
        HOTBAR_SEL = (buf[OFF_HOTBAR_SEL] as usize).min(HOTBAR_VIS - 1);
        let mut h = 0;
        while h < HOTBAR_VIS {
            HOTBAR[h] = buf[OFF_HOTBAR + h];
            h += 1;
        }
        let mut k = 0;
        while k < BLOCK_KINDS {
            INV[k] = get_u16(buf, OFF_INV + k * 2);
            k += 1;
        }
    }
    clear_containers();
    let mut off = V2_HDR;
    let mut i = 0;
    while i < chests {
        unsafe {
            CHEST_USED[i] = true;
            CHEST_X[i] = get_i16(buf, off) as i32;
            CHEST_Y[i] = get_i16(buf, off + 2) as i32;
            CHEST_Z[i] = get_i16(buf, off + 4) as i32;
            let mut k = 0;
            while k < BLOCK_KINDS {
                CHEST_INV[i][k] = get_u16(buf, off + 6 + k * 2);
                k += 1;
            }
        }
        off += CHEST_REC;
        i += 1;
    }
    let mut i = 0;
    while i < furnaces {
        unsafe {
            FURN_USED[i] = true;
            FURN_X[i] = get_i16(buf, off) as i32;
            FURN_Y[i] = get_i16(buf, off + 2) as i32;
            FURN_Z[i] = get_i16(buf, off + 4) as i32;
            FURN_IN[i] = buf[off + 6];
            FURN_OUT[i] = buf[off + 7];
            FURN_IN_N[i] = get_u16(buf, off + 8);
            FURN_FUEL[i] = get_u16(buf, off + 10);
            FURN_OUT_N[i] = get_u16(buf, off + 12);
            FURN_PROG[i] = get_u16(buf, off + 14);
        }
        off += FURN_REC;
        i += 1;
    }
    read_edits(buf, off, edits);
    true
}

#[inline(never)]
fn read_edits(buf: &[u8], base: usize, n: usize) {
    let mut idx = 0;
    while idx < n {
        let off = base + idx * EDIT_STRIDE;
        unsafe {
            EDIT_X[idx] = get_i16(buf, off);
            EDIT_Y[idx] = get_i16(buf, off + 2);
            EDIT_Z[idx] = get_i16(buf, off + 4);
            EDIT_B[idx] = buf[off + 6];
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

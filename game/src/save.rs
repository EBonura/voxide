//! Memory-card save: persist the player + edited-block deltas to a PS1 memory
//! card through `psx-mc` (SIO0 transport + the standard PS1 filesystem). The
//! save is a named file ("VOXIDE" in the console's card manager), so it lives
//! alongside retail saves instead of clobbering raw frames like the old
//! hand-rolled driver did. Old raw-frame dev saves are orphaned by the switch.
//!
//! Payload layout (unchanged from the raw-frame era, minus the frame split):
//! a fixed header (magic, player pose/stats, inventory, edit count,
//! progression) followed by 8-byte edit-delta records.

use crate::{
    Player, AIR, BLOCK_KINDS, EDIT_B, EDIT_D, EDIT_N, EDIT_X, EDIT_Y, EDIT_Z, INV, MAX_EDITS,
};
use psx_mc::{Card, HardwareCard, Slot};

const MAGIC: [u8; 4] = *b"MCPX";
/// BIOS file name: region+product code + label, 20 ASCII chars max.
const FILE_NAME: &str = "BESLES-00000VOXIDE01";
/// Human-readable label shown by the console's memory-card manager.
const FILE_TITLE: &str = "VOXIDE";

// Payload layout offsets. The header ends with the progression fields; edit
// records follow at HDR, 8 bytes apiece (x i16, y i16, z i16, block u8, pad).
const OFF_EDIT_COUNT: usize = 28 + BLOCK_KINDS;
const OFF_PROGRESS: usize = 30 + BLOCK_KINDS;
const HDR: usize = OFF_PROGRESS + 9; // armor u8, efficiency u8, xp i32, 3 tool tiers
const EDIT_STRIDE: usize = 8;
const MAX_PAYLOAD: usize = HDR + MAX_EDITS * EDIT_STRIDE;

// Serialization scratch (BSS, not stack: the payload is ~2 KiB).
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
    (b[o] as u32 | (b[o + 1] as u32) << 8 | (b[o + 2] as u32) << 16 | (b[o + 3] as u32) << 24) as i32
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

/// Persist player + inventory + edit deltas as one card file. Formats a
/// blank card first; overwrites any previous VOXIDE save.
pub fn save(p: &Player) -> bool {
    let n = unsafe { EDIT_N }.min(MAX_EDITS);
    let len = HDR + n * EDIT_STRIDE;
    let buf = unsafe { &mut BUF[..len] };

    buf[..4].copy_from_slice(&MAGIC);
    put_i32(buf, 4, p.x);
    put_i32(buf, 8, p.y);
    put_i32(buf, 12, p.z);
    put_u16(buf, 16, p.yaw);
    buf[18] = p.pick;
    buf[19] = p.selected;
    put_i32(buf, 20, p.health);
    put_i32(buf, 24, p.food);
    // Inventory counts saved as u8 (clamped 255).
    let mut k = 0;
    while k < BLOCK_KINDS {
        buf[28 + k] = unsafe { INV[k] }.min(255) as u8;
        k += 1;
    }
    put_u16(buf, OFF_EDIT_COUNT, n as u16);
    buf[OFF_PROGRESS] = p.armor;
    buf[OFF_PROGRESS + 1] = p.efficiency;
    put_i32(buf, OFF_PROGRESS + 2, p.xp);
    // The other three tool tiers; the pickaxe keeps its original byte 18.
    buf[OFF_PROGRESS + 6] = p.axe;
    buf[OFF_PROGRESS + 7] = p.shovel;
    buf[OFF_PROGRESS + 8] = p.sword;
    let mut idx = 0;
    while idx < n {
        let off = HDR + idx * EDIT_STRIDE;
        put_i16(buf, off, unsafe { EDIT_X[idx] });
        put_i16(buf, off + 2, unsafe { EDIT_Y[idx] });
        put_i16(buf, off + 4, unsafe { EDIT_Z[idx] });
        buf[off + 6] = unsafe { EDIT_B[idx] };
        // Byte 7 was the record's pad. It now carries the dimension, and it
        // reads back 0 (= overworld) on saves written before this, because the
        // serialization buffer is zeroed .bss and nothing ever wrote there.
        buf[off + 7] = unsafe { EDIT_D[idx] };
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
    card.write(FILE_NAME, FILE_TITLE, buf).is_ok()
}

/// Load player + inventory + edits from the card. Returns false (leaving the
/// caller's state untouched beyond what it reads) if there's no valid save.
/// Edits land in the EDIT log; call [`apply_edits`] to replay them into the
/// world (raw sets, then one remesh).
pub fn load(p: &mut Player) -> bool {
    let buf = unsafe { &mut BUF[..] };
    let len = match card().read(FILE_NAME, buf) {
        Ok(len) => len,
        Err(_) => return false,
    };
    if len < HDR || buf[..4] != MAGIC {
        return false;
    }
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
    p.armor = buf[OFF_PROGRESS];
    p.efficiency = buf[OFF_PROGRESS + 1];
    p.xp = get_i32(buf, OFF_PROGRESS + 2);
    p.axe = buf[OFF_PROGRESS + 6];
    p.shovel = buf[OFF_PROGRESS + 7];
    p.sword = buf[OFF_PROGRESS + 8];
    let n = (get_u16(buf, OFF_EDIT_COUNT) as usize)
        .min(MAX_EDITS)
        .min((len - HDR) / EDIT_STRIDE);
    let mut idx = 0;
    while idx < n {
        let off = HDR + idx * EDIT_STRIDE;
        unsafe {
            EDIT_X[idx] = get_i16(buf, off);
            EDIT_Y[idx] = get_i16(buf, off + 2);
            EDIT_Z[idx] = get_i16(buf, off + 4);
            EDIT_B[idx] = buf[off + 6];
            EDIT_D[idx] = buf[off + 7];
        }
        idx += 1;
    }
    unsafe { EDIT_N = n };
    true
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

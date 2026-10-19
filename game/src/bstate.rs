//! The block-state palette.
//!
//! A world cell is one byte. Ids below 128 are the *kinds* (what the inventory,
//! recipes, hardness and drops talk about). A kind that has to carry state in
//! the world (a door's facing and swing, a bed's head or foot, a torch's wall,
//! a lever's switch) is a *family*: its state 0 is the kind's own id and its
//! other states take ids from 128 up, handed out in the order of `FAMILIES`.
//! That is the shape Java settled on when it flattened block metadata into one
//! global palette of states, and it gives the same properties here:
//!
//! - the state is part of the cell, so it travels with the block through the
//!   run-length store, the edit log, the save, a piston push and a blast with
//!   no side table to keep in step;
//! - everything that only cares *which* block this is calls `kind_of`;
//! - the id space is the only price: 74 of the 128 spare ids at the moment.
//!
//! Only state that has to survive a save lives here. Derived state (the power
//! a redstone wire carries, whether a redstone torch is lit) is recomputed by
//! `redstone.rs` and kept out of the world.
//!
//! Plain data with no dependencies beyond the kind constants, so the host
//! tests include this file as it is.

use crate::{BED, DOOR_C, LADDER, LEVER, PISTON, PISTON_HEAD, RSTORCH, TORCH};

/// First id past the kinds.
pub const EXT0: u8 = 128;

/// A kind whose cells carry `states` states (state 0 is the kind itself).
#[derive(Copy, Clone)]
pub struct Family {
    pub kind: u8,
    pub states: u8,
}

pub const FAMILIES: [Family; 8] = [
    // bit 0 open, bits 1..2 facing, bit 3 upper half
    Family { kind: DOOR_C, states: 16 },
    // bits 0..1 facing (the way from the foot to the head), bit 2 head
    Family { kind: BED, states: 8 },
    // 0 on the floor, 1..4 on a wall facing 0..3
    Family { kind: TORCH, states: 5 },
    Family { kind: RSTORCH, states: 5 },
    // mount (0 floor, 1..4 wall) + 5 when switched on
    Family { kind: LEVER, states: 10 },
    // 0..3 facing (the way the panel looks, away from its wall)
    Family { kind: LADDER, states: 4 },
    // direction (see `piston`) + 6 when extended
    Family { kind: PISTON, states: 12 },
    // the head of an extended piston, by direction
    Family { kind: PISTON_HEAD, states: 6 },
];

const NFAM: usize = FAMILIES.len();

/// Number of ext ids in use.
pub const EXT_USED: usize = {
    let mut n = 0;
    let mut i = 0;
    while i < NFAM {
        n += FAMILIES[i].states as usize - 1;
        i += 1;
    }
    n
};
const _: () = assert!(EXT0 as usize + EXT_USED <= 256, "the palette outgrew a byte");

const NONE: u8 = 0xFF;

/// Per ext id: kind, state; per kind: its family; per family: first ext id.
struct Tables {
    kind: [u8; 128],
    state: [u8; 128],
    fam_of: [u8; 128],
    first: [u8; NFAM],
}

const fn build() -> Tables {
    let mut t = Tables { kind: [0; 128], state: [0; 128], fam_of: [NONE; 128], first: [0; NFAM] };
    let mut next = EXT0 as usize;
    let mut i = 0;
    while i < NFAM {
        let f = FAMILIES[i];
        t.fam_of[f.kind as usize] = i as u8;
        t.first[i] = next as u8;
        let mut s = 1;
        while s < f.states as usize {
            t.kind[next - EXT0 as usize] = f.kind;
            t.state[next - EXT0 as usize] = s as u8;
            next += 1;
            s += 1;
        }
        i += 1;
    }
    t
}

static T: Tables = build();

/// The kind of a cell: itself below 128, else the family it belongs to. An id
/// no family hands out (a corrupt save) answers AIR.
#[inline(always)]
pub fn kind_of(b: u8) -> u8 {
    if b < EXT0 {
        b
    } else {
        T.kind[(b - EXT0) as usize]
    }
}

/// The state of a cell within its kind's family (0 for a plain kind).
#[inline(always)]
pub fn state_of(b: u8) -> u8 {
    if b < EXT0 {
        0
    } else {
        T.state[(b - EXT0) as usize]
    }
}

/// The cell for `kind` in `state`. A kind without a family only has state 0.
#[inline]
pub fn make(kind: u8, state: u8) -> u8 {
    if state == 0 || kind >= EXT0 {
        return kind;
    }
    let f = T.fam_of[kind as usize];
    if f == NONE || state >= FAMILIES[f as usize].states {
        return kind;
    }
    T.first[f as usize] + state - 1
}

/// True when `b` is a cell this palette can hold (not an unassigned ext id).
#[inline]
pub fn valid(b: u8) -> bool {
    b < EXT0 || ((b - EXT0) as usize) < EXT_USED
}

/// Same kind, other state.
#[inline]
pub fn with_state(b: u8, state: u8) -> u8 {
    make(kind_of(b), state)
}

/// What a saved or legacy cell becomes now: ids this palette does not hold turn
/// to AIR, and the one-block open door of the older saves (id 47) becomes the
/// lower half of an open door. Saves from before the palette have no ext ids.
pub fn migrate(b: u8) -> u8 {
    if b == 47 {
        return door(false, 0, true);
    }
    if valid(b) {
        b
    } else {
        0
    }
}

// ---- Facing: 0 +Z, 1 +X, 2 -Z, 3 -X (the quarter of the yaw, 0 looks along +Z).

pub const FDX: [i32; 4] = [0, 1, 0, -1];
pub const FDZ: [i32; 4] = [1, 0, -1, 0];

/// The facing a Q12 yaw looks along.
#[inline]
pub fn facing_of_yaw(yaw: u16) -> u8 {
    ((((yaw as i32 & 0x0FFF) + 512) >> 10) & 3) as u8
}

/// The facing that looks the other way.
#[inline]
pub fn opposite(f: u8) -> u8 {
    (f + 2) & 3
}

/// The facing a unit step in x/z is, if it is one.
#[inline]
pub fn facing_of_step(dx: i32, dz: i32) -> Option<u8> {
    match (dx, dz) {
        (0, 1) => Some(0),
        (1, 0) => Some(1),
        (0, -1) => Some(2),
        (-1, 0) => Some(3),
        _ => None,
    }
}

// ---- Door: the closed panel stands on the near edge (the side the placer stands
// on, opposite the facing) and swings to the hinge's edge, the placer's left,
// when open.

#[inline]
pub fn door(upper: bool, facing: u8, open: bool) -> u8 {
    make(DOOR_C, open as u8 | (facing & 3) << 1 | (upper as u8) << 3)
}
#[inline]
pub fn is_door(b: u8) -> bool {
    kind_of(b) == DOOR_C
}
#[inline]
pub fn door_open(b: u8) -> bool {
    state_of(b) & 1 != 0
}
#[inline]
pub fn door_facing(b: u8) -> u8 {
    (state_of(b) >> 1) & 3
}
#[inline]
pub fn door_upper(b: u8) -> bool {
    state_of(b) & 8 != 0
}

// ---- Bed.

#[inline]
pub fn bed(head: bool, facing: u8) -> u8 {
    make(BED, (facing & 3) | (head as u8) << 2)
}
#[inline]
pub fn is_bed(b: u8) -> bool {
    kind_of(b) == BED
}
#[inline]
pub fn bed_head(b: u8) -> bool {
    state_of(b) & 4 != 0
}
/// The way from the foot half to the head half.
#[inline]
pub fn bed_facing(b: u8) -> u8 {
    state_of(b) & 3
}

// ---- Pistons: a direction in the order of `crate::DIRS` (+X -X +Y -Y +Z -Z),
// stored from up so that state 0, the kind itself, is a retracted piston facing up.

#[inline]
pub fn piston(dir: u8, extended: bool) -> u8 {
    make(PISTON, (dir + 4) % 6 + if extended { 6 } else { 0 })
}
#[inline]
pub fn piston_head(dir: u8) -> u8 {
    make(PISTON_HEAD, (dir + 4) % 6)
}
/// The way a piston or its head faces.
#[inline]
pub fn piston_dir(b: u8) -> u8 {
    (state_of(b) % 6 + 2) % 6
}
#[inline]
pub fn piston_extended(b: u8) -> bool {
    kind_of(b) == PISTON && state_of(b) >= 6
}

// ---- Wall-mounted things: torches, levers, ladders.

/// Mount 0 floor, 1..4 wall with the object facing 0..3 (away from the wall).
#[inline]
pub fn mount_of(b: u8) -> u8 {
    match kind_of(b) {
        TORCH | RSTORCH => state_of(b),
        LEVER => state_of(b) % 5,
        _ => 0,
    }
}
#[inline]
pub fn lever_on(b: u8) -> bool {
    kind_of(b) == LEVER && state_of(b) >= 5
}
#[inline]
pub fn lever(mount: u8, on: bool) -> u8 {
    make(LEVER, mount % 5 + if on { 5 } else { 0 })
}
#[inline]
pub fn ladder_facing(b: u8) -> u8 {
    state_of(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_round_trips() {
        let mut i = 0;
        while i < FAMILIES.len() {
            let f = FAMILIES[i];
            for s in 0..f.states {
                let b = make(f.kind, s);
                assert_eq!(kind_of(b), f.kind);
                assert_eq!(state_of(b), s);
                assert!(valid(b));
            }
            i += 1;
        }
    }

    #[test]
    fn ext_ids_are_dense_and_distinct() {
        let mut seen = [false; 256];
        for i in 0..FAMILIES.len() {
            let f = FAMILIES[i];
            for s in 1..f.states {
                let b = make(f.kind, s) as usize;
                assert!(b >= EXT0 as usize);
                assert!(!seen[b], "id {b} handed out twice");
                seen[b] = true;
            }
        }
        for b in EXT0 as usize..EXT0 as usize + EXT_USED {
            assert!(seen[b], "hole at {b}");
        }
        assert!(!valid(EXT0 + EXT_USED as u8));
        assert_eq!(kind_of(255), 0);
    }

    #[test]
    fn door_fields_pack() {
        for upper in [false, true] {
            for f in 0..4 {
                for open in [false, true] {
                    let b = door(upper, f, open);
                    assert!(is_door(b));
                    assert_eq!((door_upper(b), door_facing(b), door_open(b)), (upper, f, open));
                }
            }
        }
        assert_eq!(door(false, 0, false), DOOR_C);
    }

    #[test]
    fn bed_and_lever_fields_pack() {
        for head in [false, true] {
            for f in 0..4 {
                let b = bed(head, f);
                assert!(is_bed(b));
                assert_eq!((bed_head(b), bed_facing(b)), (head, f));
            }
        }
        for m in 0..5 {
            for on in [false, true] {
                let b = lever(m, on);
                assert_eq!((mount_of(b), lever_on(b)), (m, on));
            }
        }
    }

    #[test]
    fn legacy_open_door_becomes_a_lower_open_door() {
        let b = migrate(47);
        assert!(is_door(b) && door_open(b) && !door_upper(b));
        assert_eq!(migrate(EXT0 + EXT_USED as u8), 0);
        assert_eq!(migrate(20), 20);
    }

    #[test]
    fn yaw_quarters() {
        assert_eq!(facing_of_yaw(0), 0);
        assert_eq!(facing_of_yaw(0x400), 1);
        assert_eq!(facing_of_yaw(0x800), 2);
        assert_eq!(facing_of_yaw(0xC00), 3);
        assert_eq!(facing_of_yaw(0xFFF), 0);
    }
}

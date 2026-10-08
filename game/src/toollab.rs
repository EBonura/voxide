//! Headless tool measurements (`--features tool-lab`, never shipped): the
//! guest computes how long every tier takes to break a set of blocks and what
//! every weapon deals, then mines ores for real with each pickaxe tier under a
//! tape that holds R2, and publishes what each swing yielded. The table is
//! read back from a RAM dump at the TOOL_LAB symbol.
//!
//! Layout of TOOL_LAB (i32): [0] magic, [1] trials done, then
//!   MINE_AT: per block (MINE_BLOCKS order) and tier 0..4 (hand, wood, stone,
//!     iron, diamond): sim ticks to break x 2 + 1 when the tier harvests it
//!   MELEE_AT: per weapon (equip::WEAPONS order) and tier 0..4: the hit at
//!     full charge in whole hp, the base damage in hundredths, six_t
//!   TRIAL_AT: per trial: tier, ore, broke (0/1), sim ticks to break, change
//!     in INV of the ore's drop, harvest expected (0/1)
//!   ARMOR_AT: hp taken by a hit of raw 1..=20 through take_hit, for no armour,
//!     a full iron set, a full diamond set, and iron with a diamond chestplate
//!   EQUIP_AT: the wear / take-off / wear-again checks, see `equip_check`

use crate::*;

#[no_mangle]
pub static mut TOOL_LAB: [i32; 400] = [0; 400];

const MINE_BLOCKS: [u8; 14] = [
    DIRT, SAND, GRASS, WOOD, PLANK, STONE, COBBLE, COAL_ORE, IRON_ORE, GOLD_ORE, DIAMOND_ORE,
    OBSIDIAN, LUMISTONE, SNOW,
];
const MINE_AT: usize = 8;
const MELEE_AT: usize = MINE_AT + 14 * 5;
const TRIAL_AT: usize = MELEE_AT + 25 * 3;
const ARMOR_AT: usize = 270;
const EQUIP_AT: usize = 360;

/// (tier, ore) in the order they run. By hand only the two the report asks
/// about, since a hand takes 15 s to break an ore.
#[cfg(feature = "lab-demo")]
const TRIALS: [(u8, u8); 4] = [(0, COAL_ORE), (2, GOLD_ORE), (1, COAL_ORE), (3, GOLD_ORE)];
#[cfg(not(feature = "lab-demo"))]
const TRIALS: [(u8, u8); 18] = [
    (0, COAL_ORE),
    (0, GOLD_ORE),
    (1, COAL_ORE),
    (1, IRON_ORE),
    (1, GOLD_ORE),
    (1, DIAMOND_ORE),
    (2, COAL_ORE),
    (2, IRON_ORE),
    (2, GOLD_ORE),
    (2, DIAMOND_ORE),
    (3, COAL_ORE),
    (3, IRON_ORE),
    (3, GOLD_ORE),
    (3, DIAMOND_ORE),
    (4, COAL_ORE),
    (4, IRON_ORE),
    (4, GOLD_ORE),
    (4, DIAMOND_ORE),
];

const START_FRAME: u32 = 90;
/// Frames to wait after a break for the drop to fall and be picked up.
const PICKUP_FRAMES: u32 = 50;
/// Frames a trial may take before it is written down as unbroken.
const GIVE_UP: u32 = 900;

static mut TRIAL: usize = 0;
/// 0 place, 1 mining, 2 waiting for the pickup.
static mut PHASE: u8 = 0;
static mut T0: u32 = 0;
static mut F0: u32 = 0;
static mut INV0: i32 = 0;
static mut SPOT: (i32, i32, i32) = (0, 0, 0);
static mut HOME_Y: i32 = 0;

fn table(p: &mut Player) {
    let (pick, axe, shovel, sword, weapon) = (p.pick, p.axe, p.shovel, p.sword, p.weapon);
    let mut b = 0;
    while b < MINE_BLOCKS.len() {
        let mut t = 0;
        while t < 5 {
            p.pick = t as u8;
            p.axe = t as u8;
            p.shovel = t as u8;
            let blk = MINE_BLOCKS[b];
            let ticks = break_ticks(p, blk, block_hardness100(blk), false, true) as i32;
            let h = can_harvest(p, blk) as i32;
            unsafe { TOOL_LAB[MINE_AT + b * 5 + t] = ticks * 2 + h };
            t += 1;
        }
        b += 1;
    }
    p.pick = 0;
    p.axe = 0;
    p.shovel = 0;
    p.sword = 0;
    p.on_ground = true;
    p.attack_t = 2000;
    let mut w = 0;
    while w < 5 {
        let mut t = 0;
        while t < 5 {
            let class = equip::WEAPONS[w];
            p.pick = 0;
            p.axe = 0;
            p.shovel = 0;
            p.sword = 0;
            match class {
                TOOL_SWORD => p.sword = t as u8,
                TOOL_AXE => p.axe = t as u8,
                TOOL_PICK => p.pick = t as u8,
                TOOL_SHOVEL => p.shovel = t as u8,
                _ => {}
            }
            p.weapon = class;
            let at = MELEE_AT + (w * 5 + t) * 3;
            unsafe {
                TOOL_LAB[at] = melee_damage(p) as i32;
                TOOL_LAB[at + 1] = equip::damage100(p);
                TOOL_LAB[at + 2] = equip::six_t(p);
            }
            t += 1;
        }
        w += 1;
    }
    p.pick = pick;
    p.axe = axe;
    p.shovel = shovel;
    p.sword = sword;
    p.weapon = weapon;
}

fn armor_table(player: &Player) {
    let sets: [[u8; 4]; 4] = [
        [AIR; 4],
        [ARMOR0, ARMOR0 + 1, ARMOR0 + 2, ARMOR0 + 3],
        [ARMOR0 + 4, ARMOR0 + 5, ARMOR0 + 6, ARMOR0 + 7],
        [ARMOR0, ARMOR0 + 5, ARMOR0 + 2, ARMOR0 + 3],
    ];
    let mut si = 0;
    while si < 4 {
        let mut raw = 1;
        while raw <= 20 {
            let mut q = *player;
            q.worn = sets[si];
            q.protection = 0;
            q.hurt_cd = 0;
            q.health = 200;
            full_durability(&mut q);
            let d = take_hit(&mut q, raw, true);
            unsafe { TOOL_LAB[ARMOR_AT + si * 20 + raw as usize - 1] = d };
            raw += 1;
        }
        si += 1;
    }
}

/// Wear an iron helmet, wear it down to 100 uses, take it off and put it on
/// again: the stack count, the slot, the uses and the spare each step.
/// [0..4] after wearing: INV count, worn kind, uses; [3..6] after taking off:
/// INV count, spare, worn kind; [6..8] after wearing again: uses, spare;
/// [8] a second worn copy refused when one is held already.
fn equip_check(player: &Player) {
    let mut q = *player;
    let kind = ARMOR0;
    let saved = unsafe { INV[kind as usize] };
    q.worn = [AIR; 4];
    unsafe { INV[kind as usize] = 2 };
    let a = equip::wear(&mut q, kind).is_ok() as i32;
    let o = EQUIP_AT;
    unsafe {
        TOOL_LAB[o] = INV[kind as usize] as i32;
        TOOL_LAB[o + 1] = q.worn[0] as i32;
        TOOL_LAB[o + 2] = q.armor_dur[0] as i32;
    }
    q.armor_dur[0] = 100;
    let b = equip::take_off(&mut q, 0).is_ok() as i32;
    unsafe {
        TOOL_LAB[o + 3] = INV[kind as usize] as i32;
        TOOL_LAB[o + 4] = equip::spare_get()[0] as i32;
        TOOL_LAB[o + 5] = q.worn[0] as i32;
    }
    let c = equip::wear(&mut q, kind).is_ok() as i32;
    unsafe {
        TOOL_LAB[o + 6] = q.armor_dur[0] as i32;
        TOOL_LAB[o + 7] = equip::spare_get()[0] as i32;
    }
    // Wear it down again and take off with a spare already held: refused.
    q.armor_dur[0] = 50;
    equip::spare_set([77, 0, 0, 0, 0, 0, 0, 0]);
    let d = equip::take_off(&mut q, 0).is_err() as i32;
    unsafe {
        TOOL_LAB[o + 8] = a | b << 1 | c << 2 | d << 3;
        INV[kind as usize] = saved;
    }
    equip::spare_set([0; 8]);
}

/// Run each frame from the gameplay loop, before input is read. Holds the
/// view straight down at the block under the player's feet.
pub fn frame(frame: u32, player: &mut Player, ls: &mut (i16, i16), rs: &mut (i16, i16)) {
    *ls = (0, 0);
    *rs = (0, 0);
    player.yaw = 0;
    player.pitch = -1000;
    player.health = MAX_HEALTH;
    player.food = MAX_FOOD;
    unsafe { TOOL_LAB[0] = 0x544F4F4C };
    if frame == 3 {
        let mut copy = *player;
        table(&mut copy);
        armor_table(player);
        equip_check(player);
    }
    if frame >= START_FRAME && unsafe { TRIAL } >= TRIALS.len() {
        player.pitch = 1000; // done: stop mining
    }
    if frame < START_FRAME || unsafe { TRIAL } >= TRIALS.len() {
        return;
    }
    let now = unsafe { SIM_TICK };
    let (tier, ore) = TRIALS[unsafe { TRIAL }];
    // Look at the sky while the drop settles, so the held button breaks
    // nothing more and the hotbar shows what the swing yielded.
    player.pitch = if unsafe { PHASE } == 2 { 1000 } else { -1000 };
    unsafe {
        match PHASE {
            0 => {
                if TRIAL == 0 {
                    SPOT = (
                        world_to_block_x(player.x),
                        world_to_block_y(player.y) - 1,
                        world_to_block_z(player.z),
                    );
                    HOME_Y = player.y;
                }
                let (bx, by, bz) = SPOT;
                // Back on top of the spot, a fresh ore block under the feet.
                player.y = HOME_Y;
                player.vy = 0;
                set_block_i32(bx, by, bz, ore);
                player.pick = tier;
                player.axe = 0;
                player.shovel = 0;
                player.sword = 0;
                INV0 = INV[ore as usize] as i32;
                T0 = now;
                F0 = frame;
                PHASE = 1;
            }
            1 => {
                let (bx, by, bz) = SPOT;
                let broke = get_block_i32(bx, by, bz) == AIR;
                if broke || frame - F0 > GIVE_UP {
                    let at = TRIAL_AT + TRIAL * 6;
                    TOOL_LAB[at] = tier as i32;
                    TOOL_LAB[at + 1] = ore as i32;
                    TOOL_LAB[at + 2] = broke as i32;
                    TOOL_LAB[at + 3] = now.wrapping_sub(T0) as i32;
                    TOOL_LAB[at + 5] = {
                        let mut q = *player;
                        q.pick = tier;
                        can_harvest(&q, ore) as i32
                    };
                    F0 = frame;
                    PHASE = 2;
                }
            }
            _ => {
                if frame - F0 >= PICKUP_FRAMES {
                    let at = TRIAL_AT + TRIAL * 6;
                    TOOL_LAB[at + 4] = INV[ore as usize] as i32 - INV0;
                    TRIAL += 1;
                    TOOL_LAB[1] = TRIAL as i32;
                    PHASE = 0;
                }
            }
        }
    }
}

//! Headless edit-persistence check (`--features edit-lab`, never shipped).
//!
//! Builds N (1,500) floating stone blocks by the normal edit path (set + log), walks
//! the player 150 blocks away so the ring regenerates from the seed, walks back,
//! and counts how many of the 1,500 are still stone, on the TTY:
//!
//!   EDITLAB recorded, back a=<edits the store holds> b=<stone cells found>
//!
//! The 1,024-edit log of the shipped game keeps the first 1,024 and drops the
//! rest without a word, so `back` comes out 1024 there.

use crate::*;

/// Cells to build (VOXIDE_EDITLAB_N at build time; 1,500 by default).
const N: i32 = match option_env!("VOXIDE_EDITLAB_N") {
    Some(s) => parse_const_i32(s),
    None => 1500,
};
static mut PHASE: u8 = 0;
static mut T0: u32 = 0;
static mut ORIGIN: (i32, i32, i32) = (0, 0, 0);
static mut RECORDED: usize = 0;

fn cell(i: i32) -> (i32, i32, i32) {
    let (bx, y0, bz) = unsafe { ORIGIN };
    (bx - 20 + i % 40, y0 + i / 960, bz + 4 + (i / 40) % 24)
}

fn say(tag: &str, a: usize, b: usize) {
    tty::print(tag);
    tty::print(" a=");
    tty::print_hex_u32(a as u32);
    tty::print(" b=");
    tty::print_hex_u32(b as u32);
    tty::println("");
}

pub fn tick(player: &mut Player, frame: u32) {
    player.health = 20;
    unsafe {
        match PHASE {
            0 if frame >= 40 => {
                let (bx, bz) = (world_to_block_x(player.x), world_to_block_z(player.z));
                ORIGIN = (bx, world::surface_y(bx, bz) + 6, bz);
                let mut i = 0;
                while i < N {
                    let (x, y, z) = cell(i);
                    set_block_i32(x, y, z, STONE);
                    record_edit(x, y, z, STONE);
                    i += 1;
                }
                RECORDED = edit_count();
                say("EDITLAB built (edits held, cells)", RECORDED, N as usize);
                PHASE = 1;
                T0 = frame;
            }
            1 if frame - T0 >= 60 => {
                let (bx, bz) = (world_to_block_x(player.x) + 150, world_to_block_z(player.z));
                player.x += 150 * BLOCK;
                player.y = (world::surface_y(bx, bz) + 3) * BLOCK;
                player.vy = 0;
                PHASE = 2;
                T0 = frame;
            }
            2 if frame - T0 >= 900 => {
                let (bx, bz) = (world_to_block_x(player.x) - 150, world_to_block_z(player.z));
                player.x -= 150 * BLOCK;
                player.y = (world::surface_y(bx, bz) + 9) * BLOCK;
                player.vy = 0;
                PHASE = 3;
                T0 = frame;
            }
            3 if frame - T0 >= 900 => {
                let mut back = 0usize;
                let mut i = 0;
                while i < N {
                    let (x, y, z) = cell(i);
                    if get_block_i32(x, y, z) == STONE {
                        back += 1;
                    }
                    i += 1;
                }
                say("EDITLAB recorded, back", RECORDED, back);
                PHASE = 4;
            }
            _ => {}
        }
    }
}

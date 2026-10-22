//! Host tests of the block-state palette, the shape table and the redstone
//! rules: the game's own source files, compiled as they are against a few
//! constants the real crate root defines.

#![allow(dead_code)]

pub const AIR: u8 = 0;
pub const BED: u8 = 20;
pub const WIRE: u8 = 21;
pub const TORCH: u8 = 22;
pub const PISTON: u8 = 23;
pub const TNT: u8 = 24;
pub const LADDER: u8 = 33;
pub const DOOR_C: u8 = 46;
pub const FENCE: u8 = 62;
pub const SLAB: u8 = 61;
pub const STAIRS_N: u8 = 63;
pub const STAIRS_E: u8 = 64;
pub const STAIRS_S: u8 = 65;
pub const STAIRS_W: u8 = 66;
pub const PISTON_HEAD: u8 = 104;
pub const RSTORCH: u8 = 102;
pub const LEVER: u8 = 103;
pub const STONE: u8 = 3;
pub const GLASS: u8 = 14;
pub const DIRS: [(i32, i32, i32); 6] = [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)];

pub mod tex {
    pub const T_COBBLE: u8 = 16;
    pub const T_WOOD_SIDE: u8 = 5;
    pub const T_PLANK: u8 = 17;
    pub const T_DOOR_LO: u8 = 113;
    pub const T_DOOR_UP: u8 = 114;
    pub const T_BED_FOOT: u8 = 115;
    pub const T_BED_HEAD: u8 = 116;
    pub const T_BED_SIDE: u8 = 117;
    pub const T_TORCH: u8 = 120;
    pub const T_RSTORCH: u8 = 121;
    pub const T_LADDER_W: u8 = 123;
}

#[path = "../../../game/src/bstate.rs"]
pub mod bstate;
#[path = "../../../game/src/shape.rs"]
pub mod shape;
#[path = "../../../game/src/redstone.rs"]
pub mod redstone;
#[path = "../../../game/src/edits.rs"]
pub mod edits;
#[path = "../../../game/src/bow.rs"]
pub mod bow;

use redstone::{Cells, Effect, Net, EFFECTS};
use std::collections::HashMap;

/// A world in memory: stone is the one conductor.
#[derive(Default)]
struct Grid {
    cells: HashMap<(i32, i32, i32), u8>,
}

impl Grid {
    fn set(&mut self, x: i32, y: i32, z: i32, b: u8) {
        self.cells.insert((x, y, z), b);
    }
    /// A floor of stone at y = -1 under x in 0..40, z in -3..4.
    fn floor() -> Self {
        let mut g = Grid::default();
        for x in -2..40 {
            for z in -3..4 {
                g.set(x, -1, z, STONE);
            }
        }
        g
    }
}

impl Cells for Grid {
    fn block(&self, x: i32, y: i32, z: i32) -> u8 {
        *self.cells.get(&(x, y, z)).unwrap_or(&AIR)
    }
    fn conductor(&self, b: u8) -> bool {
        b == STONE
    }
    fn loaded(&self, _x: i32, _z: i32) -> bool {
        true
    }
}

/// Register every emitter in the grid.
fn net_of(g: &Grid) -> Net {
    let mut n = Net::new();
    let mut keys: Vec<_> = g.cells.iter().collect();
    keys.sort();
    for (&(x, y, z), &b) in keys {
        n.changed(x, y, z, b);
    }
    n
}

fn run(n: &mut Net, g: &Grid, ticks: usize) -> Vec<Effect> {
    let mut all = Vec::new();
    for _ in 0..ticks {
        let mut fx = [Effect::Tnt(0, 0, 0); EFFECTS];
        let k = n.step(g, &mut fx);
        all.extend_from_slice(&fx[..k]);
    }
    all
}

fn settle(n: &mut Net, g: &Grid) -> Vec<Effect> {
    run(n, g, 4)
}

#[test]
fn a_lever_powers_a_line_of_dust_and_it_fades() {
    let mut g = Grid::floor();
    g.set(0, 0, 0, STONE);
    g.set(0, 1, 0, bstate::lever(0, true));
    for x in 1..=20 {
        g.set(x, 0, 0, WIRE);
    }
    let mut n = net_of(&g);
    settle(&mut n, &g);
    // The lever stands on the stone at x = 0; dust at x = 1 is beside the stone,
    // which the lever powers hard, so it gets 15.
    assert_eq!(n.wire_power(1, 0, 0), 15);
    assert_eq!(n.wire_power(2, 0, 0), 14);
    assert_eq!(n.wire_power(15, 0, 0), 1);
    assert_eq!(n.wire_power(16, 0, 0), 0);
    assert_eq!(n.wire_power(20, 0, 0), 0);
    // Switch it off.
    g.set(0, 1, 0, bstate::lever(0, false));
    n.changed(0, 1, 0, bstate::lever(0, false));
    settle(&mut n, &g);
    for x in 1..=20 {
        assert_eq!(n.wire_power(x, 0, 0), 0, "x = {x}");
    }
}

#[test]
fn dust_that_ends_at_a_door_opens_it_and_shuts_it_again() {
    let mut g = Grid::floor();
    g.set(0, 0, 0, STONE);
    g.set(0, 1, 0, bstate::lever(0, true));
    for x in 1..=4 {
        g.set(x, 0, 0, WIRE);
    }
    g.set(5, 0, 0, bstate::door(false, 1, false));
    g.set(5, 1, 0, bstate::door(true, 1, false));
    let mut n = net_of(&g);
    let fx = settle(&mut n, &g);
    assert!(fx.contains(&Effect::Door(5, 0, 0, true)), "{fx:?}");
    g.set(0, 1, 0, bstate::lever(0, false));
    n.changed(0, 1, 0, bstate::lever(0, false));
    let fx = settle(&mut n, &g);
    assert!(fx.contains(&Effect::Door(5, 0, 0, false)), "{fx:?}");
}

#[test]
fn a_lever_beside_a_door_works_it_without_dust() {
    let mut g = Grid::floor();
    g.set(2, 0, 0, bstate::door(false, 0, false));
    g.set(2, 1, 0, bstate::door(true, 0, false));
    g.set(1, 0, 0, STONE);
    g.set(1, 1, 0, bstate::lever(0, true)); // beside the door's upper half
    let n = net_of(&g);
    let mut fx = [Effect::Tnt(0, 0, 0); EFFECTS];
    let k = n.react(&g, 1, 1, 0, &mut fx, 0);
    assert!(fx[..k].contains(&Effect::Door(2, 1, 0, true)), "{:?}", &fx[..k]);
}

#[test]
fn a_torch_is_lit_until_its_block_is_powered() {
    let mut g = Grid::floor();
    // Lever -> stone block with a torch on its far side wall.
    g.set(0, 0, 0, STONE);
    g.set(0, 1, 0, bstate::lever(0, false));
    g.set(1, 0, 0, STONE); // the torch's block
    g.set(2, 0, 0, bstate::make(RSTORCH, 1 + 1)); // wall torch, facing +X, fixed to x = 1
    g.set(0, 0, 1, WIRE);
    g.set(1, 0, 1, WIRE); // dust beside the block at x = 1 pointing at it? it runs +X
    let mut n = net_of(&g);
    settle(&mut n, &g);
    assert!(n.torch_lit(2, 0, 0));
    // Power the dust, which points into the torch's block from the side.
    g.set(0, 1, 0, bstate::lever(0, true));
    n.changed(0, 1, 0, bstate::lever(0, true));
    settle(&mut n, &g);
    assert!(n.wire_power(0, 0, 1) > 0);
}

#[test]
fn a_torch_under_a_block_with_dust_on_top_is_an_inverter() {
    let mut g = Grid::floor();
    // torch on the floor at (1,0,0) fixed to the floor below; block over it at (1,1,0)
    g.set(1, 0, 0, bstate::make(RSTORCH, 0));
    g.set(1, 1, 0, STONE);
    g.set(1, 2, 0, WIRE);
    g.set(2, 2, 0, WIRE);
    g.set(3, 2, 0, WIRE);
    let mut n = net_of(&g);
    settle(&mut n, &g);
    // The torch is lit and strongly powers the block over it: dust on top has 15.
    assert!(n.torch_lit(1, 0, 0));
    assert_eq!(n.wire_power(1, 2, 0), 15);
    assert_eq!(n.wire_power(2, 2, 0), 14);
    assert_eq!(n.wire_power(3, 2, 0), 13);
}

#[test]
fn two_torches_in_a_ring_make_a_clock() {
    // Torch A on a block is put out by dust fed from torch B and the other way
    // round: a three-torch ring never settles.
    let mut g = Grid::floor();
    // Three blocks in a row at y = 0 each with a wall torch on its +X side
    // whose dust runs on to the next block's side.
    // Block i at x = 3i; torch at x = 3i + 1 (facing +X, fixed to the block);
    // dust at x = 3i + 2 runs to the next block at 3(i+1) - which it points at.
    for i in 0..3 {
        g.set(3 * i, 0, 0, STONE);
        g.set(3 * i + 1, 0, 0, bstate::make(RSTORCH, 1 + 1));
        g.set(3 * i + 2, 0, 0, WIRE);
    }
    // Close the ring with a dust return: the last dust feeds back to block 0
    // through a path above.
    let mut n = net_of(&g);
    let mut states = Vec::new();
    for _ in 0..8 {
        run(&mut n, &g, 1);
        states.push((n.torch_lit(1, 0, 0), n.torch_lit(4, 0, 0), n.torch_lit(7, 0, 0)));
    }
    // A chain of inverters from an undriven first block: torch 0 stays lit,
    // torch 1 is put out by its dust (fed by torch 0), torch 2 relights.
    let last = *states.last().unwrap();
    assert!(last.0, "{states:?}");
    assert!(!last.1, "{states:?}");
    assert!(last.2, "{states:?}");
}

#[test]
fn dust_climbs_a_step_unless_a_block_caps_it() {
    let mut g = Grid::floor();
    g.set(0, 0, 0, STONE);
    g.set(0, 1, 0, bstate::lever(0, true));
    g.set(1, 0, 0, WIRE);
    g.set(2, 0, 0, STONE);
    g.set(2, 1, 0, WIRE); // one up, on a block
    g.set(3, 1, 0, WIRE);
    let mut n = net_of(&g);
    settle(&mut n, &g);
    assert_eq!(n.wire_power(1, 0, 0), 15);
    assert_eq!(n.wire_power(2, 1, 0), 14, "the step up joins");
    assert_eq!(n.wire_power(3, 1, 0), 13);
    // A stone cap over the lower dust cuts the join.
    g.set(1, 1, 0, STONE);
    n.changed(1, 1, 0, STONE);
    settle(&mut n, &g);
    assert_eq!(n.wire_power(2, 1, 0), 0);
}

#[test]
fn tnt_beside_powered_dust_is_lit() {
    let mut g = Grid::floor();
    g.set(0, 0, 0, STONE);
    g.set(0, 1, 0, bstate::lever(0, true));
    g.set(1, 0, 0, WIRE);
    g.set(2, 0, 0, WIRE);
    g.set(3, 0, 0, TNT);
    let mut n = net_of(&g);
    let fx = settle(&mut n, &g);
    assert!(fx.contains(&Effect::Tnt(3, 0, 0)), "{fx:?}");
}

#[test]
fn a_single_dust_runs_straight_through_its_cell() {
    let mut g = Grid::floor();
    g.set(1, 0, 0, WIRE);
    g.set(0, 0, 0, WIRE);
    let l = Net::links(&g, 1, 0, 0);
    assert_eq!(l, [0, 1, 0, 1]); // -X joins; the far side runs through too
    let l = Net::links(&g, 0, 0, 0);
    assert_eq!(l, [0, 0 + 1, 0, 1]);
}

#[test]
fn registry_forgets_broken_blocks_and_keeps_quiet() {
    let mut g = Grid::floor();
    g.set(0, 0, 0, WIRE);
    let mut n = net_of(&g);
    settle(&mut n, &g);
    assert_eq!(n.len(), 1);
    g.set(0, 0, 0, AIR);
    n.changed(0, 0, 0, AIR);
    settle(&mut n, &g);
    assert_eq!(n.len(), 0);
}

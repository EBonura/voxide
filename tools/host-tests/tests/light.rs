//! Block light (game/src/light.rs): the tiers a torch or a lava cell throws, clipped to the chunk
//! being meshed, with sources that may stand in a neighbouring chunk.

#[path = "../../../game/src/light.rs"]
mod light;

use light::{BRIGHT_REACH, ROWS, TIERS, TORCH_REACH};

fn empty() -> light::Rows {
    [[0; ROWS]; TIERS]
}

fn tier(rows: &light::Rows, x: i32, y: i32, z: i32) -> u16 {
    light::tier(rows, x as usize, y as usize, z as usize)
}

/// What the tier should be from the distance alone.
fn expected(reach: &[i32; TIERS], d: i32) -> u16 {
    if d <= reach[0] {
        3
    } else if d <= reach[1] {
        2
    } else if d <= reach[2] {
        1
    } else {
        0
    }
}

#[test]
fn a_torch_in_the_open_lights_by_taxicab_distance() {
    let mut rows = empty();
    assert!(light::flood(&mut rows, 8, 30, 8, &TORCH_REACH));
    for dy in -12i32..=12 {
        for dz in -7i32..=7 {
            for dx in -7i32..=7 {
                let d = dx.abs() + dy.abs() + dz.abs();
                assert_eq!(
                    tier(&rows, 8 + dx, 30 + dy, 8 + dz),
                    expected(&TORCH_REACH, d),
                    "offset ({dx}, {dy}, {dz})"
                );
            }
        }
    }
}

#[test]
fn lava_reaches_a_block_further_than_a_torch() {
    let mut torch = empty();
    let mut lava = empty();
    light::flood(&mut torch, 8, 30, 8, &TORCH_REACH);
    light::flood(&mut lava, 8, 30, 8, &BRIGHT_REACH);
    // Nine blocks up is the torch's last cell, ten the lava's.
    assert_eq!(tier(&torch, 8, 30, 8), 3);
    assert_eq!(tier(&lava, 8, 30, 8), 3);
    assert_eq!(tier(&torch, 8, 39, 8), 1, "nine up is the torch's tier 1");
    assert_eq!(tier(&torch, 8, 40, 8), 0, "ten up is outside the torch");
    assert_eq!(tier(&lava, 8, 40, 8), 1, "ten up is the lava's tier 1");
    assert_eq!(tier(&lava, 8, 41, 8), 0);
}

#[test]
fn the_tiers_are_nested_so_the_count_is_the_level() {
    let mut rows = empty();
    light::flood(&mut rows, 3, 20, 12, &TORCH_REACH);
    light::flood(&mut rows, 13, 25, 2, &BRIGHT_REACH);
    for row in 0..ROWS {
        assert_eq!(rows[2][row] & !rows[1][row], 0, "tier 3 inside tier 2, row {row}");
        assert_eq!(rows[1][row] & !rows[0][row], 0, "tier 2 inside tier 1, row {row}");
    }
}

#[test]
fn two_sources_take_the_brighter_tier() {
    let mut rows = empty();
    light::flood(&mut rows, 4, 30, 8, &TORCH_REACH);
    light::flood(&mut rows, 12, 30, 8, &TORCH_REACH);
    // Between them, four blocks from each: the second tier of both.
    assert_eq!(tier(&rows, 8, 30, 8), 2);
    // Right beside one of them.
    assert_eq!(tier(&rows, 5, 30, 8), 3);
}

#[test]
fn the_flood_is_clipped_at_the_chunk_and_never_wraps() {
    let mut rows = empty();
    light::flood(&mut rows, 0, 0, 0, &TORCH_REACH);
    assert_eq!(tier(&rows, 0, 0, 0), 3);
    assert_eq!(tier(&rows, 9, 0, 0), 1);
    assert_eq!(tier(&rows, 10, 0, 0), 0);
    // The far side of a 16-wide row must not light up from the near side.
    for x in 10..16 {
        assert_eq!(tier(&rows, x, 0, 0), 0, "x {x}");
    }
    for z in 10..16 {
        assert_eq!(tier(&rows, 0, 0, z), 0, "z {z}");
    }
    let mut rows = empty();
    light::flood(&mut rows, 15, 63, 15, &TORCH_REACH);
    assert_eq!(tier(&rows, 15, 63, 15), 3);
    assert_eq!(tier(&rows, 6, 63, 15), 1);
    assert_eq!(tier(&rows, 5, 63, 15), 0);
    assert_eq!(tier(&rows, 15, 54, 15), 1);
}

#[test]
fn a_torch_in_the_next_chunk_lights_this_chunks_edge() {
    // The torch stands at x = -3 (three blocks west of this chunk): its tier-2 reach is six.
    let mut rows = empty();
    assert!(light::flood(&mut rows, -3, 30, 8, &TORCH_REACH));
    assert_eq!(tier(&rows, 0, 30, 8), 3, "three blocks away");
    assert_eq!(tier(&rows, 3, 30, 8), 2, "six blocks away");
    assert_eq!(tier(&rows, 6, 30, 8), 1, "nine blocks away");
    assert_eq!(tier(&rows, 7, 30, 8), 0);
    // And the east neighbour lights the east edge.
    let mut rows = empty();
    light::flood(&mut rows, 18, 30, 8, &TORCH_REACH);
    assert_eq!(tier(&rows, 15, 30, 8), 3);
    assert_eq!(tier(&rows, 9, 30, 8), 1);
    assert_eq!(tier(&rows, 8, 30, 8), 0);
}

#[test]
fn a_source_too_far_away_sets_nothing() {
    let mut rows = empty();
    assert!(!light::flood(&mut rows, -10, 30, 8, &TORCH_REACH));
    assert!(!light::flood(&mut rows, 8, 30, 26, &TORCH_REACH));
    assert!(rows.iter().all(|t| t.iter().all(|&r| r == 0)));
    assert!(!light::reaches_chunk(-10, 8, &TORCH_REACH));
    assert!(light::reaches_chunk(-9, 8, &TORCH_REACH));
    assert!(light::reaches_chunk(-5, -4, &TORCH_REACH), "a corner: five and four is nine");
    assert!(!light::reaches_chunk(-5, -5, &TORCH_REACH));
    assert!(light::reaches_chunk(0, 15, &TORCH_REACH));
}

#[test]
fn a_flood_never_reads_or_writes_outside_the_rows() {
    // Sources at every corner, edge and just outside of the volume.
    for &x in &[-12, -1, 0, 7, 15, 16, 27] {
        for &y in &[-12, -1, 0, 31, 63, 64, 75] {
            for &z in &[-12, -1, 0, 7, 15, 16, 27] {
                let mut rows = empty();
                light::flood(&mut rows, x, y, z, &BRIGHT_REACH);
            }
        }
    }
}

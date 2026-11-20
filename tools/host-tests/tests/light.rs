//! Block light (game/src/light.rs): the tiers a torch or a lava cell throws, clipped to the chunk
//! being meshed, with sources that may stand in a neighbouring chunk.

#![feature(optimize_attribute)]

#[path = "../../../game/src/light.rs"]
mod light;

use light::{BRIGHT_REACH, COLS, TIERS, TORCH_REACH};

fn empty() -> light::Cols {
    [[[0; 2]; COLS]; TIERS]
}

fn tier(rows: &light::Cols, x: i32, y: i32, z: i32) -> u16 {
    light::tier(rows, x, y, z)
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
    for col in 0..COLS {
        for w in 0..2 {
            assert_eq!(rows[2][col][w] & !rows[1][col][w], 0, "tier 3 inside tier 2, column {col}");
            assert_eq!(rows[1][col][w] & !rows[0][col][w], 0, "tier 2 inside tier 1, column {col}");
        }
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
    assert!(!light::flood(&mut rows, -11, 30, 8, &TORCH_REACH));
    assert!(!light::flood(&mut rows, 8, 30, 27, &TORCH_REACH));
    assert!(rows.iter().all(|t| t.iter().all(|c| c[0] == 0 && c[1] == 0)));
    // The ring of columns one outside the chunk counts: nine from -1 is -10.
    assert!(!light::reaches_chunk(-11, 8, &TORCH_REACH));
    assert!(light::reaches_chunk(-10, 8, &TORCH_REACH));
    assert!(light::reaches_chunk(-6, -5, &TORCH_REACH), "a corner: five and four past the ring");
    assert!(!light::reaches_chunk(-6, -6, &TORCH_REACH));
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

#[test]
fn the_fog_mix_is_the_division_it_replaces() {
    for a in 0..=255i32 {
        for b in 0..=255i32 {
            for t in 0..=255i32 {
                let want = (a + (b - a) * t / 255).clamp(0, 255) as u32;
                assert_eq!(light::mix_fog(a as u8, b, t), want, "a {a} b {b} t {t}");
            }
        }
    }
}

#[test]
fn ramping_into_the_fog_keeps_the_order_of_two_colours() {
    // renderer: the rows with block light are the brighter of the sky row and the torch row,
    // each ramped on its own, instead of the ramp of the brighter tint.
    for f in [0i32, 1, 37, 128, 200, 254, 255] {
        for t in 0..=255i32 {
            let mix: Vec<u32> = (0..=255u32).map(|c| light::mix_fog(c as u8, f, t)).collect();
            for a in 0..=255usize {
                for b in a..=255usize {
                    assert!(mix[a] <= mix[b], "f {f} t {t}: {a} -> {}, {b} -> {}", mix[a], mix[b]);
                }
            }
        }
    }
}

// ---- A torch stopped by walls ---------------------------------------------------------------

use light::{FloodBits, OpenMap, FLOOD_R, FLOOD_WORDS};

/// The reference: a plain breadth-first search over cells, for the path distance of each cell.
fn bfs<F: Fn(i32, i32, i32) -> bool>(solid: &F) -> std::collections::HashMap<(i32, i32, i32), i32> {
    let mut dist = std::collections::HashMap::new();
    let mut queue = std::collections::VecDeque::new();
    dist.insert((0, 0, 0), 0);
    queue.push_back((0, 0, 0));
    while let Some((x, y, z)) = queue.pop_front() {
        let d = dist[&(x, y, z)];
        if d == FLOOD_R {
            continue;
        }
        for (ax, ay, az) in [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
            let n = (x + ax, y + ay, z + az);
            if !dist.contains_key(&n) && !solid(n.0, n.1, n.2) {
                dist.insert(n, d + 1);
                queue.push_back(n);
            }
        }
    }
    dist
}

/// The open map of a world for a torch at its origin.
fn open_map<F: Fn(i32, i32, i32) -> bool>(solid: &F) -> OpenMap {
    let mut open: OpenMap = [0; 21 * 21];
    for dx in -FLOOD_R..=FLOOD_R {
        for dz in -FLOOD_R..=FLOOD_R {
            if dx.abs() + dz.abs() > FLOOD_R {
                continue;
            }
            let mut word = 0u32;
            for dy in -FLOOD_R..=FLOOD_R {
                if !solid(dx, dy, dz) {
                    word |= 1 << (dy + FLOOD_R);
                }
            }
            open[light::open_index(dx, dz)] = word;
        }
    }
    // The torch's own cell lets light out whatever it is.
    open[light::open_index(0, 0)] |= 1 << FLOOD_R;
    open
}

/// What the tier of a cell should be from its path distance.
fn tier_of_path(d: i32) -> u16 {
    if d <= 3 {
        3
    } else if d <= 6 {
        2
    } else if d <= 9 {
        1
    } else {
        0
    }
}

fn check_against_bfs<F: Fn(i32, i32, i32) -> bool>(solid: F, what: &str) -> FloodBits {
    let dist = bfs(&solid);
    let mut bits: FloodBits = [0; FLOOD_WORDS];
    light::reflow(&open_map(&solid), &mut [0; 21 * 21], &mut bits);
    for dx in -11i32..=11 {
        for dy in -11i32..=11 {
            for dz in -11i32..=11 {
                let want = dist.get(&(dx, dy, dz)).map_or(0, |&d| tier_of_path(d));
                assert_eq!(
                    light::flood_tier(&bits, dx, dy, dz),
                    want,
                    "{what}: offset ({dx}, {dy}, {dz})"
                );
            }
        }
    }
    bits
}

#[test]
fn a_torch_in_the_open_floods_to_the_taxicab_ball() {
    let bits = check_against_bfs(|_, _, _| false, "open");
    // A flood of open air is the same as the blind flood.
    let mut walled = empty();
    let mut blind = empty();
    light::apply(&mut walled, &bits, 8, 30, 8);
    light::flood(&mut blind, 8, 30, 8, &TORCH_REACH);
    assert_eq!(walled, blind);
}

#[test]
fn a_sealed_pit_keeps_its_light() {
    // A box of air round the torch, three blocks a side, in stone: nothing outside is lit.
    let solid = |x: i32, y: i32, z: i32| x.abs() > 1 || y.abs() > 1 || z.abs() > 1;
    let bits = check_against_bfs(solid, "sealed");
    assert_eq!(light::flood_tier(&bits, 1, 0, 0), 3);
    assert_eq!(light::flood_tier(&bits, 2, 0, 0), 0, "the wall itself is not lit");
    let mut cols = empty();
    light::apply(&mut cols, &bits, 8, 30, 8);
    assert_eq!(tier(&cols, 8, 30, 8), 3);
    assert_eq!(tier(&cols, 8, 36, 8), 0, "the ground above the cover is dark");
}

#[test]
fn light_goes_through_an_opening_only() {
    // A wall at x = 2 with a doorway at y = 0..=1, z = 0: the room behind it is lit through the
    // door and nowhere else.
    let solid = |x: i32, y: i32, z: i32| x == 2 && !(z == 0 && (0..=1).contains(&y));
    let bits = check_against_bfs(solid, "doorway");
    assert_eq!(light::flood_tier(&bits, 3, 0, 0), 3, "straight through the door");
    assert_eq!(light::flood_tier(&bits, 3, 0, 5), 1, "round the corner: path 3 + 5 = 8 steps");
    assert_eq!(light::flood_tier(&bits, 3, 5, 0), 1, "up behind the door: 3 + 5 steps");
    assert_eq!(light::flood_tier(&bits, 3, 7, 0), 0, "path longer than 9");
    // The cell right behind the wall to the side is reached by the long way round, not through it.
    assert_eq!(light::flood_tier(&bits, 3, 0, 3), 2);
}

#[test]
fn a_parallel_tunnel_two_blocks_behind_stone_stays_dark() {
    // A tunnel along x (y 0..=1, z 0) and another at z = 3, the stone between them two blocks
    // thick (z = 1 and 2), the two joined only far away at x = 30.
    let solid = |x: i32, y: i32, z: i32| {
        let in_a = (0..=1).contains(&y) && z == 0;
        let in_b = (0..=1).contains(&y) && z == 3;
        let join = (0..=1).contains(&y) && (0..=3).contains(&z) && x >= 30;
        !(in_a || in_b || join)
    };
    let bits = check_against_bfs(solid, "tunnels");
    assert_eq!(light::flood_tier(&bits, 2, 0, 0), 3, "down the tunnel");
    assert_eq!(light::flood_tier(&bits, 8, 0, 0), 1);
    for x in -9..=9 {
        assert_eq!(light::flood_tier(&bits, x, 0, 3), 0, "the other tunnel at x {x}");
    }
}

#[test]
fn the_flood_matches_a_search_in_noise() {
    // A few hundred worlds of scattered stone at varying density.
    let mut seed = 0x1234_5678u32;
    let mut next = move || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        seed >> 8
    };
    for density in [8u32, 24, 40, 56] {
        for n in 0..6 {
            let mut cells = std::collections::HashSet::new();
            for _ in 0..6_000 {
                let r = next();
                let x = (r % 23) as i32 - 11;
                let y = ((r / 23) % 23) as i32 - 11;
                let z = ((r / 529) % 23) as i32 - 11;
                if next() % 100 < density {
                    cells.insert((x, y, z));
                }
            }
            check_against_bfs(|x, y, z| cells.contains(&(x, y, z)), &format!("noise {density}/{n}"));
        }
    }
}

#[test]
fn applying_a_flood_clips_like_a_walled_search_in_the_chunk() {
    // The same torch at chunk edges and corners, outside it and at the ceiling, against the
    // search: every lit cell of the chunk is the torch's, and nothing else.
    let solid = |x: i32, y: i32, z: i32| (x + 2 * y + 3 * z).rem_euclid(7) == 0 && (x, y, z) != (0, 0, 0);
    let dist = bfs(&solid);
    let mut bits: FloodBits = [0; FLOOD_WORDS];
    light::reflow(&open_map(&solid), &mut [0; 21 * 21], &mut bits);
    for &(sx, sy, sz) in &[
        (8, 30, 8),
        (0, 30, 0),
        (15, 30, 15),
        (-3, 30, 8),
        (18, 30, -4),
        (8, 0, 8),
        (8, 63, 8),
        (8, 59, 8),
        (-9, 63, 24),
    ] {
        let mut cols = empty();
        light::apply(&mut cols, &bits, sx, sy, sz);
        for z in -1..=16i32 {
            for x in -1..=16i32 {
                for y in 0..64i32 {
                    let want = dist.get(&(x - sx, y - sy, z - sz)).map_or(0, |&d| tier_of_path(d));
                    assert_eq!(
                        tier(&cols, x, y, z),
                        want,
                        "torch ({sx}, {sy}, {sz}), cell ({x}, {y}, {z})"
                    );
                }
            }
        }
    }
}

#[test]
fn two_torches_take_the_brighter_tier_and_the_runs_are_nested() {
    let solid = |x: i32, y: i32, _: i32| x == 3 && y != 0;
    let mut bits: FloodBits = [0; FLOOD_WORDS];
    light::reflow(&open_map(&solid), &mut [0; 21 * 21], &mut bits);
    let mut cols = empty();
    light::apply(&mut cols, &bits, 4, 30, 8);
    light::apply(&mut cols, &bits, 12, 30, 8);
    for col in 0..COLS {
        for w in 0..2 {
            assert_eq!(cols[2][col][w] & !cols[1][col][w], 0);
            assert_eq!(cols[1][col][w] & !cols[0][col][w], 0);
        }
    }
}

#[test]
fn a_flood_fits_its_words() {
    assert_eq!(FLOOD_WORDS, 50);
    // The torch buried in stone lights only its own cell.
    let solid = |x: i32, y: i32, z: i32| (x, y, z) != (0, 0, 0);
    let bits = check_against_bfs(solid, "buried");
    assert_eq!(light::flood_tier(&bits, 0, 0, 0), 3);
    assert_eq!(light::flood_tier(&bits, 1, 0, 0), 0);
}

#[test]
fn the_ring_of_columns_round_the_chunk_holds_the_light_its_faces_take() {
    let mut rows = empty();
    light::flood(&mut rows, 0, 30, 0, &TORCH_REACH);
    assert_eq!(tier(&rows, -1, 30, 0), 3, "one block out of the chunk, beside the torch");
    assert_eq!(tier(&rows, 0, 30, -1), 3);
    assert_eq!(tier(&rows, -1, 30, -1), 3, "diagonally: two steps");
    assert_eq!(tier(&rows, 16, 30, 0), 0, "the far ring is out of reach");
    // Heights off the world have no light and are not read.
    assert_eq!(tier(&rows, 0, -1, 0), 0);
    assert_eq!(tier(&rows, 0, 64, 0), 0);
    let mut rows = empty();
    light::flood(&mut rows, 15, 63, 15, &TORCH_REACH);
    assert_eq!(tier(&rows, 16, 63, 16), 3);
    assert_eq!(tier(&rows, 16, 64, 16), 0);
}

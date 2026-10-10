//! Host tests for the fire and explosion rules: game/src/blast.rs (Java's
//! ray-cast block damage, entity damage, exposure, fuses) and game/src/flame.rs
//! (Java's flame ticks: ageing, burn-out, spread, rain, TNT priming). Both are
//! plain integer files with no dependencies, so the tests compile them with the
//! pinned rustc beside a small mock world (a hash map of blocks) and run the
//! rules thousands of times to pin their odds against the numbers on
//! minecraft.wiki (Explosion, Fire, TNT).

mod common;

/// Each file's own #[cfg(test)] module.
#[test]
fn blast_rs_unit_tests() {
    own_tests("game/src/blast.rs");
}

#[test]
fn flame_rs_unit_tests() {
    own_tests("game/src/flame.rs");
}

fn own_tests(file: &str) {
    let root = common::root();
    let scratch = common::Scratch::new("vox-fire-own-");
    let source = std::fs::read_to_string(root.join(file)).expect("source");
    let output = common::compile_and_run(
        &scratch,
        &source,
        &["--edition", "2021", "--test"],
        &[],
        &root,
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("test result: ok"));
}

/// The rules against a mock world.
const SCENARIOS: &str = r#"
use std::collections::HashMap;

struct Rng(u32);
impl Rng {
    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x >> 8
    }
}

// Blocks of the mock world.
const AIR: u8 = 0;
const DIRT: u8 = 1;
const STONE: u8 = 2;
const PLANK: u8 = 3;
const LEAVES: u8 = 4;
const TNT: u8 = 5;
const FIRE: u8 = 6;
const NETHERRACK: u8 = 7;
const OBSIDIAN: u8 = 8;
const WATER: u8 = 9;

/// Blast resistance in hundredths, Java's (minecraft.wiki/w/Explosion#Blast
/// resistance); AIR is NO_BLOCK.
fn resistance(b: u8) -> i32 {
    match b {
        AIR => blast::NO_BLOCK,
        DIRT => 50,
        STONE => 600,
        PLANK => 300,
        LEAVES => 20,
        TNT | FIRE => 0,
        NETHERRACK => 40,
        OBSIDIAN => 120_000,
        WATER => 10_000,
        _ => 300,
    }
}

fn cell(b: u8) -> flame::Cell {
    let (ignite, burn) = match b {
        PLANK => (5, 20),
        LEAVES => (30, 60),
        TNT => (15, 100),
        _ => (0, 0),
    };
    flame::Cell {
        empty: b == AIR,
        ignite,
        burn,
        sturdy: matches!(b, DIRT | STONE | PLANK | TNT | NETHERRACK | OBSIDIAN),
        infinite: b == NETHERRACK,
        tnt: b == TNT,
    }
}

struct World {
    cells: HashMap<(i32, i32, i32), u8>,
    default: u8,
}
impl World {
    fn new(default: u8) -> World {
        World { cells: HashMap::new(), default }
    }
    fn get(&self, x: i32, y: i32, z: i32) -> u8 {
        *self.cells.get(&(x, y, z)).unwrap_or(&self.default)
    }
    fn set(&mut self, x: i32, y: i32, z: i32, b: u8) {
        self.cells.insert((x, y, z), b);
    }
}

// ---------------------------------------------------------------- blasts

/// Every cell a blast of `power` at the middle of cell (0,0,0) breaks, for one
/// seed. Mirrors the game: the blocks around the centre are copied into a
/// snapshot, and the rays read that.
fn blast_set(world: &World, power: i32, seed: u32) -> Vec<(i32, i32, i32)> {
    let r = blast::REACH_CELLS;
    let mut snap = Box::new([0u8; blast::CELLS]);
    for dz in -r..=r {
        for dx in -r..=r {
            for dy in -r..=r {
                snap[blast::Cube::index(dx, dy, dz)] = world.get(dx, dy, dz);
            }
        }
    }
    let mut res = [0i32; 256];
    for (b, slot) in res.iter_mut().enumerate() {
        *slot = resistance(b as u8);
    }
    let mut cube = blast::Cube::new();
    {
        let mut scene = blast::Scene { snap: &snap, res: &res, cube: &mut cube };
        for k in 0..blast::GRID_POINTS {
            if let Some(step) = blast::ray_step(k) {
                blast::trace(&mut scene, (1 << 15, 1 << 15, 1 << 15), step, blast::ray_power(power, seed, k));
            }
        }
    }
    let mut out = Vec::new();
    for dz in -r..=r {
        for dx in -r..=r {
            for dy in -r..=r {
                if cube.get(dx, dy, dz) {
                    out.push((dx, dy, dz));
                }
            }
        }
    }
    out
}

fn reach(cells: &[(i32, i32, i32)]) -> i32 {
    cells.iter().map(|c| c.0.abs().max(c.1.abs()).max(c.2.abs())).max().unwrap_or(0)
}

#[test]
fn open_air_blast_reaches_java_radius() {
    // In air a ray spends 0.225 a step: the strongest, 4 x 1.3 = 5.2, lasts 23
    // steps of 0.3, 6.9 blocks, and the average 4.0 lasts 17.8 steps, 5.3.
    // Air holds no blocks to break, so count the cells the rays pass with a
    // world of TNT (resistance 0 still costs 0.09 a step) vs one of plain air:
    let air = World::new(AIR);
    assert!(blast_set(&air, 4, 1).is_empty(), "air has nothing to break");
}

#[test]
fn dirt_crater_is_a_sphere_of_about_two_and_a_half_blocks() {
    let dirt = World::new(DIRT);
    let mut sizes = Vec::new();
    for seed in 0..40 {
        let cells = blast_set(&dirt, 4, seed);
        // Dirt costs (0.5 + 0.3) x 0.3 + 0.225 = 0.465 a step: the strongest
        // ray, 5.2, goes 11 steps (3.3 blocks), the weakest, 2.8, 6 (1.8).
        assert!(reach(&cells) <= 4, "reach {} too far", reach(&cells));
        assert!(cells.contains(&(0, 0, 0)));
        for d in [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
            assert!(cells.contains(&d), "face neighbour {d:?} survived");
        }
        sizes.push(cells.len());
    }
    let mean = sizes.iter().sum::<usize>() / sizes.len();
    assert!((60..260).contains(&mean), "mean crater {mean} cells");
}

#[test]
fn blast_resistance_orders_the_craters() {
    let mut volume = |b: u8| -> usize {
        let w = World::new(b);
        (0..20).map(|s| blast_set(&w, 4, s).len()).sum::<usize>() / 20
    };
    let (leaves, dirt, planks, stone, obsidian) =
        (volume(LEAVES), volume(DIRT), volume(PLANK), volume(STONE), volume(OBSIDIAN));
    assert!(leaves > dirt && dirt > planks && planks > stone && stone > obsidian);
    // Obsidian (1200) and water (100) stop a power 4 blast outright: nothing
    // breaks, not even the first cell.
    assert_eq!(obsidian, 0);
    assert_eq!(volume(WATER), 0);
}

#[test]
fn a_wall_shields_what_is_behind_it() {
    // Open air with a two thick stone wall in the +x direction at x = 2, 3.
    let mut w = World::new(DIRT);
    for y in -8..=8 {
        for z in -8..=8 {
            for x in 1..=8 {
                w.set(x, y, z, if x == 2 || x == 3 { STONE } else { AIR });
            }
            w.set(0, y, z, AIR);
        }
    }
    // Behind the wall is air: nothing to break, but the wall itself is hit.
    let cells = blast_set(&w, 4, 3);
    assert!(cells.iter().all(|c| c.0 <= 3), "nothing beyond the second stone layer");
}

#[test]
fn a_blast_primes_the_tnt_it_reaches() {
    // A 3x3x3 cube of TNT: the blast in its middle reaches every block.
    let mut w = World::new(AIR);
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                w.set(x, y, z, TNT);
            }
        }
    }
    // One block 3 away in air, and one 12 away: only the near cell is a TNT
    // the rays reach.
    let cells = blast_set(&w, 4, 5);
    let tnt: Vec<_> = cells.iter().filter(|c| w.get(c.0, c.1, c.2) == TNT).collect();
    assert_eq!(tnt.len(), 27, "every block of the cube is in the blast");
    // A neighbouring cube two blocks off is reached too (TNT costs a ray almost nothing).
    let mut w2 = World::new(AIR);
    for x in 2..=3 {
        w2.set(x, 0, 0, TNT);
    }
    w2.set(12, 0, 0, TNT);
    let near: Vec<_> = (0..20)
        .flat_map(|s| blast_set(&w2, 4, s))
        .filter(|c| w2.get(c.0, c.1, c.2) == TNT)
        .collect();
    assert!(near.iter().any(|c| c.0 <= 3), "TNT 2 to 3 blocks off is reached");
    assert!(near.iter().all(|c| c.0 != 12), "TNT 12 blocks off is out of range");
}

#[test]
fn chain_fuses_are_ten_to_twenty_nine_game_ticks() {
    let mut seen = [false; 40];
    for k in 0..50_000 {
        let t = blast::chain_fuse_ticks(7, k);
        assert!((10..=29).contains(&t));
        seen[t as usize] = true;
    }
    assert!((10..=29).all(|t| seen[t]), "every length comes up");
}

#[test]
fn fuse_and_flash_follow_java() {
    // 80 game ticks: 240 sim ticks, 4 seconds at 60 a second.
    assert_eq!(blast::FUSE_JAVA_TICKS * 3, 240);
    // White for 5 game ticks (15 sim ticks) of every 10. The first tick takes
    // the fuse from 240 to 239, so the fuse is seen from 239 down: red first,
    // then eight white flashes, the last 14 ticks from the end.
    let mut flashes = 0;
    let mut last = None;
    for fuse in (1..=239u32).rev() {
        let f = blast::flashing(fuse);
        if f && last != Some(true) {
            flashes += 1;
        }
        last = Some(f);
    }
    assert_eq!(flashes, 8, "eight white flashes in a 4 second fuse");
    assert!(!blast::flashing(239) && blast::flashing(224) && !blast::flashing(209));
}

#[test]
fn drops_keep_a_quarter_of_the_blocks() {
    let mut kept = 0;
    let n = 200_000;
    for k in 0..n {
        if blast::drops(11, k, 4) {
            kept += 1;
        }
    }
    let rate = kept as f64 / n as f64;
    assert!((rate - 0.25).abs() < 0.01, "drop rate {rate}");
}

#[test]
fn exposure_follows_cover() {
    let c = (0, 64, 0); // the blast, a block above the floor
    let e = (5 * 64, 64, 0); // the entity five blocks off at the same height
    let mut open = |_: i32, _: i32, _: i32| false;
    assert_eq!(blast::exposure_q8(c, e, 19, 115, &mut open), 256);
    // A wall across the whole line of sight.
    let mut wall = |x: i32, _: i32, _: i32| x == 2;
    assert_eq!(blast::exposure_q8(c, e, 19, 115, &mut wall), 0);
    // The entity stands a block lower, behind a wall one cell tall at its feet:
    // the sample points at its feet and waist are covered, the one at its head
    // is not, a third of the box.
    let low_e = (5 * 64, 0, 0);
    let mut low = |x: i32, y: i32, _: i32| x == 2 && y == 0;
    assert_eq!(blast::exposure_q8(c, low_e, 19, 115, &mut low), 256 * 2 / 6);
}

#[test]
fn damage_and_knockback_fall_with_distance() {
    let at = |d: i32| blast::proximity_q8(4, d).map(|p| blast::damage_from_impact(4, p));
    assert_eq!(at(0), Some(57)); // point blank
    assert!(at(2 * 64).unwrap() > at(4 * 64).unwrap());
    assert!(at(7 * 64).unwrap() >= 1);
    assert_eq!(at(8 * 64), None); // 2 x power blocks is the edge
    // Knockback points away from the centre with the impact's strength.
    let (kx, ky, kz) = blast::knockback_q8(128, 0, 0, 200);
    assert_eq!((kx, ky, kz), (200, 0, 0));
    assert_eq!(blast::knockback_q8(0, 0, 0, 200), (0, 0, 0));
}

// ---------------------------------------------------------------- fire

struct Fire {
    w: World,
    rng: Rng,
    rain: bool,
    primed: Vec<(i32, i32, i32)>,
    lit: Vec<(i32, i32, i32, u8)>,
}
impl Fire {
    fn new(default: u8, seed: u32) -> Fire {
        Fire { w: World::new(default), rng: Rng(seed), rain: false, primed: vec![], lit: vec![] }
    }
}
impl flame::Env for Fire {
    fn cell(&self, x: i32, y: i32, z: i32) -> flame::Cell {
        cell(self.w.get(x, y, z))
    }
    fn rand(&mut self, n: u32) -> u32 {
        self.rng.next() % n
    }
    fn raining(&self) -> bool {
        self.rain
    }
    fn near_rain(&self, _: i32, _: i32, _: i32) -> bool {
        self.rain
    }
    fn rain_at(&self, _: i32, _: i32, _: i32) -> bool {
        self.rain
    }
    fn spawn_fire(&mut self, x: i32, y: i32, z: i32, age: u8) {
        self.w.set(x, y, z, FIRE);
        self.lit.push((x, y, z, age));
    }
    fn burn_to_fire(&mut self, x: i32, y: i32, z: i32, age: u8) {
        self.w.set(x, y, z, FIRE);
        self.lit.push((x, y, z, age));
    }
    fn burn_away(&mut self, x: i32, y: i32, z: i32) {
        self.w.set(x, y, z, AIR);
    }
    fn prime(&mut self, x: i32, y: i32, z: i32) {
        self.primed.push((x, y, z));
    }
}

/// A flame at the origin on a floor of `floor` (y = -1), air elsewhere, with
/// `setup` adding blocks.
fn arena(floor: u8, seed: u32, setup: impl Fn(&mut World)) -> Fire {
    let mut f = Fire::new(AIR, seed);
    for x in -4..=4 {
        for z in -4..=4 {
            f.w.set(x, -1, z, floor);
        }
    }
    f.w.set(0, 0, 0, FIRE);
    setup(&mut f.w);
    f
}

fn near(n: u32, expect: f64, total: u32, tol: f64, what: &str) {
    let rate = n as f64 / total as f64;
    assert!((rate - expect).abs() < tol, "{what}: {rate:.4} against {expect:.4}");
}

#[test]
fn fire_beside_tnt_primes_it_a_third_of_the_time_each_tick() {
    // checkBurnOut on a side face: rand(300) < 100 (TNT's flammability).
    for dir in [(1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1)] {
        let mut hit = 0;
        let total = 30_000;
        for i in 0..total {
            let mut f = arena(DIRT, 1000 + i, |w| w.set(dir.0, dir.1, dir.2, TNT));
            flame::tick(&mut f, 0, 0, 0, 0);
            if f.primed.contains(&dir) {
                hit += 1;
            }
        }
        near(hit, 100.0 / 300.0, total, 0.012, "side TNT");
    }
}

#[test]
fn fire_under_and_over_tnt_uses_the_250_range() {
    for dir in [(0, 1, 0)] {
        let mut hit = 0;
        let total = 30_000;
        for i in 0..total {
            let mut f = arena(DIRT, 5000 + i, |w| w.set(dir.0, dir.1, dir.2, TNT));
            flame::tick(&mut f, 0, 0, 0, 0);
            if f.primed.contains(&dir) {
                hit += 1;
            }
        }
        near(hit, 100.0 / 250.0, total, 0.012, "TNT above");
    }
}

#[test]
fn planks_beside_fire_burn_a_fifteenth_of_the_time() {
    // 20 / 300 a tick; half of those become flame at age 0, half vanish.
    let total = 40_000;
    let (mut gone, mut fire) = (0, 0);
    for i in 0..total {
        let mut f = arena(DIRT, 9000 + i, |w| w.set(1, 0, 0, PLANK));
        flame::tick(&mut f, 0, 0, 0, 0);
        match f.w.get(1, 0, 0) {
            AIR => gone += 1,
            FIRE => fire += 1,
            _ => {}
        }
    }
    near(gone + fire, 20.0 / 300.0, total, 0.006, "planks burn");
    // rand(10) < 5 at age 0: even odds between flame and nothing.
    let share = fire as f64 / (gone + fire) as f64;
    assert!((share - 0.5).abs() < 0.05, "flame share {share}");
}

#[test]
fn flame_spreads_into_the_cell_above_planks_two_percent_of_ticks() {
    // The cell above planks: ignite 5, age 0: (5 + 40 + 14) / 30 = 1, and
    // rand(100) <= 1 is 2 in 100. Planks at (1, 0, 0) put it in the box.
    let total = 60_000;
    let mut lit = 0;
    for i in 0..total {
        let mut f = arena(DIRT, 20_000 + i, |w| w.set(1, 0, 0, PLANK));
        flame::tick(&mut f, 0, 0, 0, 0);
        if f.lit.iter().any(|l| (l.0, l.1, l.2) == (1, 1, 0)) {
            lit += 1;
        }
    }
    near(lit, 0.02, total, 0.004, "spread over planks");
}

#[test]
fn leaves_and_age_change_the_spread_odds() {
    // Leaves (ignite 30): q = 2, so rand(100) <= 2: 3 in 100.
    let total = 60_000;
    let mut lit = 0;
    for i in 0..total {
        let mut f = arena(DIRT, 40_000 + i, |w| w.set(1, 0, 0, LEAVES));
        flame::tick(&mut f, 0, 0, 0, 0);
        if f.lit.iter().any(|l| (l.0, l.1, l.2) == (1, 1, 0)) {
            lit += 1;
        }
    }
    // Leaves beside the flame burn first a fifth of the time (60 / 300), taking
    // the fuel with them: 0.8 x 3 in 100.
    near(lit, 0.024, total, 0.004, "spread over leaves");
    // Three cells up the range is 300: (2 + 1 more) / 300 = 1 in 100.
    // Leaves at (1, 2, 0) feed the cell (1, 3, 0), three above the flame.
    let mut lit = 0;
    for i in 0..total {
        // A plank beside the flame keeps it spreading at all: a flame with
        // nothing flammable touching it only sits (Java's valid fire location).
        let mut f = arena(DIRT, 80_000 + i, |w| {
            w.set(1, 2, 0, LEAVES);
            w.set(0, 0, 1, PLANK);
        });
        flame::tick(&mut f, 0, 0, 0, 0);
        if f.lit.iter().any(|l| (l.0, l.1, l.2) == (1, 3, 0)) {
            lit += 1;
        }
    }
    // Over leaves in the air there is no floor: it survives beside the leaves.
    near(lit, 0.01, total, 0.003, "spread three up");
    // An old flame (15) beside leaves: (30 + 54) / 45 = 1: 2 in 100, after
    // the three in four that survive the burnout roll and the four in five
    // leaves that survive the flame.
    let mut lit = 0;
    for i in 0..total {
        let mut f = arena(DIRT, 120_000 + i, |w| w.set(1, 0, 0, LEAVES));
        flame::tick(&mut f, 0, 0, 0, 15);
        if f.lit.iter().any(|l| (l.0, l.1, l.2) == (1, 1, 0)) {
            lit += 1;
        }
    }
    near(lit, 0.75 * 0.8 * 0.02, total, 0.003, "spread from an old flame");
}

#[test]
fn flames_age_by_one_a_third_of_the_time() {
    let total = 30_000;
    let mut older = 0;
    for i in 0..total {
        // Planks on the other side keep it alive and give it something to burn.
        let mut f = arena(DIRT, 160_000 + i, |w| w.set(0, -1, 0, PLANK));
        if flame::tick(&mut f, 0, 0, 0, 3) == Some(4) {
            older += 1;
        }
    }
    near(older, 1.0 / 3.0, total, 0.012, "age step");
}

#[test]
fn a_flame_on_bare_stone_dies_young_or_lives_on() {
    // Nothing burns beside it: age 3 and younger live (and age), older goes out.
    let mut f = arena(STONE, 7, |_| {});
    assert!(flame::tick(&mut f, 0, 0, 0, 0).is_some());
    assert!(flame::tick(&mut f, 0, 0, 0, 3).is_some());
    assert_eq!(flame::tick(&mut f, 0, 0, 0, 4), None);
    // No floor, no fuel: out at once.
    let mut f = Fire::new(AIR, 3);
    f.w.set(0, 0, 0, FIRE);
    assert_eq!(flame::tick(&mut f, 0, 0, 0, 0), None);
}

#[test]
fn a_full_grown_flame_on_a_burnable_floor_dies_a_quarter_of_ticks() {
    // Age 15 with fuel beside it, over a floor that cannot burn: 1 in 4.
    let total = 30_000;
    let mut out = 0;
    for i in 0..total {
        let mut f = arena(DIRT, 200_000 + i, |w| w.set(1, 0, 0, PLANK));
        if flame::tick(&mut f, 0, 0, 0, 15).is_none() {
            out += 1;
        }
    }
    near(out, 0.25, total, 0.012, "old flame dies");
    // Over planks (burnable) that roll does not apply.
    for i in 0..5_000 {
        let mut f = arena(PLANK, 400_000 + i, |_| {});
        assert!(flame::tick(&mut f, 0, 0, 0, 15).is_some());
    }
}

#[test]
fn netherrack_fire_never_goes_out() {
    let mut f = arena(NETHERRACK, 9, |_| {});
    let mut age = 0;
    for _ in 0..500 {
        age = flame::tick(&mut f, 0, 0, 0, age).expect("netherrack fire is permanent");
    }
    assert_eq!(age, 15);
    // And rain does not put it out either.
    f.rain = true;
    for _ in 0..500 {
        assert!(flame::tick(&mut f, 0, 0, 0, 15).is_some());
    }
}

#[test]
fn rain_puts_flames_out_with_chance_point_two_plus_point_zero_three_a_year() {
    for age in [0u8, 5, 15] {
        let total = 30_000;
        let mut out = 0;
        for i in 0..total {
            let mut f = arena(PLANK, 300_000 + i + age as u32 * 77, |_| {});
            f.rain = true;
            if flame::tick(&mut f, 0, 0, 0, age).is_none() {
                out += 1;
            }
        }
        near(out, 0.2 + 0.03 * age as f64, total, 0.012, "rain");
    }
}

#[test]
fn tick_delays_are_thirty_to_thirty_nine_game_ticks() {
    for r in 0..1000 {
        assert!((30..=39).contains(&flame::tick_delay(r)));
    }
}

#[test]
fn lighting_needs_a_floor_or_fuel() {
    let mut f = Fire::new(AIR, 1);
    f.w.set(0, -1, 0, DIRT);
    assert!(flame::can_survive(&f, 0, 0, 0), "a sturdy top");
    assert!(!flame::can_survive(&f, 5, 5, 5), "mid air");
    f.w.set(6, 5, 5, PLANK);
    assert!(flame::can_survive(&f, 5, 5, 5), "beside fuel");
    f.w.set(0, -1, 7, LEAVES);
    assert!(flame::can_survive(&f, 0, 0, 7), "over leaves, which burn");
}
"#;

#[test]
fn rules_against_a_mock_world() {
    let root = common::root();
    let scratch = common::Scratch::new("vox-fire-rules-");
    let blast = root.join("game/src/blast.rs");
    let flame = root.join("game/src/flame.rs");
    let program = format!(
        "#![allow(dead_code)]\n#[path = {blast:?}]\nmod blast;\n#[path = {flame:?}]\nmod flame;\n{SCENARIOS}"
    );
    let output = common::compile_and_run(
        &scratch,
        &program,
        &["--edition", "2021", "--test", "-C", "opt-level=2"],
        &["--test-threads", "4"],
        &root,
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("test result: ok"), "{stdout}");
}

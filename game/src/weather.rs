//! Weather and lightning, on Java Edition's rules.
//!
//! Java keeps two flags, rain and thunder, each on its own timer
//! (minecraft.wiki/w/Weather, /w/Thunderstorm): rain stays on for 12,000 to
//! 24,000 game ticks and off for 12,000 to 180,000; thunder stays on for 3,600
//! to 15,600 and off for 12,000 to 180,000, and only matters while it rains.
//! A clear-weather timer, set only by `/weather clear` (the cheat menu here),
//! holds both off while it runs. A new world starts clear.
//!
//! The flags drive two levels that move 0.01 a game tick (five seconds from
//! nothing to full): the rain level, and the thunder level, which Java reports
//! multiplied by the rain level. The world reads "raining" above 0.2 and
//! "thundering" above 0.9, and the sky light is computed from both levels, so
//! a shower that is still arriving darkens the world by as much as it has
//! arrived instead of switching tables halfway in.
//!
//! In a thunderstorm every loaded chunk has a 1 in 100,000 chance a game tick
//! of a lightning strike. Java ticks the chunks whose centre is within 128
//! blocks of the player, about 201 of them, so the player sees about 2.4
//! strikes a minute (minecraft.wiki/w/Thunderstorm#Lightning mechanics).
//! VoXide only holds 5x5 chunks, but the strike rate and area are Java's:
//! the generator knows every column's height, so a strike out in the hills
//! still lands on the ground and still lights the sky.

use crate::units::java_ticks;
use crate::*;

/// Sim ticks a level takes from 0 to full: 0.01 a game tick is 100 game ticks
/// (Level.tick's rain and thunder levels).
const RAMP: u32 = java_ticks(100) as u32;

static mut RAINING: bool = false;
static mut THUNDERING: bool = false;
/// Sim ticks left on each timer (0 = roll a new length on the next tick).
static mut RAIN_T: u32 = 0;
static mut THUNDER_T: u32 = 0;
static mut CLEAR_T: u32 = 0;
/// The two levels, 0..=RAMP.
static mut RAIN_Q: u32 = 0;
static mut THUNDER_Q: u32 = 0;

fn roll(lo: i32, hi: i32) -> u32 {
    java_ticks(lo + (world_rand() % (hi - lo + 1) as u32) as i32) as u32
}

/// One sim tick of Java's weather cycle (ServerLevel.advanceWeatherCycle,
/// with every timer in sim ticks: three to a game tick).
#[inline(never)]
pub fn tick() {
    #[cfg(feature = "storm-lab")]
    unsafe {
        static mut LAB: bool = false;
        if !LAB {
            LAB = true;
            RAINING = true;
            THUNDERING = true;
            RAIN_T = java_ticks(15_000) as u32;
            THUNDER_T = RAIN_T;
            RAIN_Q = RAMP;
            THUNDER_Q = RAMP;
        }
    }
    unsafe {
        if CLEAR_T > 0 {
            CLEAR_T -= 1;
            RAIN_T = if RAINING { 0 } else { 1 };
            THUNDER_T = if THUNDERING { 0 } else { 1 };
            RAINING = false;
            THUNDERING = false;
        } else {
            if RAIN_T > 0 {
                RAIN_T -= 1;
                if RAIN_T == 0 {
                    RAINING = !RAINING;
                }
            } else {
                RAIN_T = if RAINING {
                    roll(12_000, 24_000)
                } else {
                    roll(12_000, 180_000)
                };
            }
            if THUNDER_T > 0 {
                THUNDER_T -= 1;
                if THUNDER_T == 0 {
                    THUNDERING = !THUNDERING;
                }
            } else {
                THUNDER_T = if THUNDERING {
                    roll(3_600, 15_600)
                } else {
                    roll(12_000, 180_000)
                };
            }
        }
        RAIN_Q = if RAINING {
            (RAIN_Q + 1).min(RAMP)
        } else {
            RAIN_Q.saturating_sub(1)
        };
        THUNDER_Q = if THUNDERING {
            (THUNDER_Q + 1).min(RAMP)
        } else {
            THUNDER_Q.saturating_sub(1)
        };
        crate::world::set_raining(raining());
        crate::mob::set_raining(raining());
    }
}

/// Waking from a night's sleep resets the weather cycle (Java's
/// resetWeatherCycle): both flags off and both timers rolled anew. The levels
/// are left alone, so a shower fades out over five seconds after waking, as
/// it does in Java.
#[inline(never)]
#[optimize(size)] // weather changes and strikes are rare: bytes over cycles
pub fn sleep_clear() {
    unsafe {
        RAINING = false;
        RAIN_T = 0;
        THUNDERING = false;
        THUNDER_T = 0;
    }
}

/// A new world: clear sky, every timer unrolled, no bolts.
#[inline(never)]
#[optimize(size)] // weather changes and strikes are rare: bytes over cycles
pub fn reset() {
    unsafe {
        RAINING = false;
        THUNDERING = false;
        RAIN_T = 0;
        THUNDER_T = 0;
        CLEAR_T = 0;
        RAIN_Q = 0;
        THUNDER_Q = 0;
        BOLTS = [NO_BOLT; BOLT_CAP];
        FLASH = 0;
    }
}

/// `/weather clear|rain|thunder` with no duration: Java picks one at random,
/// 12,000-180,000, 12,000-24,000 and 3,600-15,600 game ticks
/// (minecraft.wiki/w/Commands/weather), and sets the clear timer, both
/// weather timers and both flags at once (setWeatherParameters).
#[inline(never)]
#[optimize(size)] // weather changes and strikes are rare: bytes over cycles
pub fn command(kind: u8) {
    let (clear, t, rain, thunder) = match kind {
        0 => (roll(12_000, 180_000), 0, false, false),
        1 => (0, roll(12_000, 24_000), true, false),
        _ => (0, roll(3_600, 15_600), true, true),
    };
    unsafe {
        CLEAR_T = clear;
        RAIN_T = t;
        THUNDER_T = t;
        RAINING = rain;
        THUNDERING = thunder;
    }
}

/// Rain level, 0 (clear) to 255 (full).
pub fn rain() -> i32 {
    (unsafe { RAIN_Q } * 255 / RAMP) as i32
}

/// Thunder level as Java reports it: its own level times the rain level,
/// 0 to 255.
pub fn thunder() -> i32 {
    (unsafe { THUNDER_Q * RAIN_Q } * 255 / (RAMP * RAMP)) as i32
}

/// Level.isRaining: the rain level above 0.2.
pub fn raining() -> bool {
    unsafe { RAIN_Q * 5 > RAMP }
}

/// Level.isThundering: the thunder level (times rain) above 0.9.
pub fn thundering() -> bool {
    unsafe { THUNDER_Q * RAIN_Q * 10 > 9 * RAMP * RAMP }
}

/// The weather a save keeps: Java's level.dat stores the flags and the three
/// timers (raining, thundering, rainTime, thunderTime, clearWeatherTime).
/// Timers here are in sim ticks.
#[derive(Copy, Clone)]
pub struct Saved {
    pub raining: bool,
    pub thundering: bool,
    pub rain_t: u32,
    pub thunder_t: u32,
    pub clear_t: u32,
}

pub fn saved() -> Saved {
    unsafe {
        Saved {
            raining: RAINING,
            thundering: THUNDERING,
            rain_t: RAIN_T,
            thunder_t: THUNDER_T,
            clear_t: CLEAR_T,
        }
    }
}

/// Restore a save's weather. Java starts a loaded world's levels at full
/// when it was raining, and the thunder level at full when it was also
/// thundering (Level.prepareWeather), so a storm resumes as a storm. A save
/// from before weather was kept loads clear, as a level.dat without the
/// fields does.
#[inline(never)]
#[optimize(size)] // weather changes and strikes are rare: bytes over cycles
pub fn restore(s: Option<Saved>) {
    reset();
    if let Some(s) = s {
        // Java's longest spell is 180,000 game ticks.
        let cap = java_ticks(180_000) as u32;
        unsafe {
            RAINING = s.raining;
            THUNDERING = s.thundering;
            RAIN_T = s.rain_t.min(cap);
            THUNDER_T = s.thunder_t.min(cap);
            CLEAR_T = s.clear_t.min(cap);
            if RAINING {
                RAIN_Q = RAMP;
                if THUNDERING {
                    THUNDER_Q = RAMP;
                }
            }
        }
    }
}

// ---- Lightning -------------------------------------------------------------

/// Bolts alive at once. A strike lasts a few game ticks, so two only overlap
/// when strikes land within a fraction of a second.
const BOLT_CAP: usize = 2;

#[derive(Copy, Clone)]
struct Bolt {
    live: bool,
    /// Bottom centre, world units.
    x: i32,
    y: i32,
    z: i32,
    /// Java's LightningBolt.life: 2 when it strikes, flashing while >= 0.
    life: i8,
    /// Re-flashes left (Java: 1 to 3 at random).
    flashes: u8,
    /// Shape seed; a re-flash takes a new one, so the bolt forks anew.
    seed: u32,
}

const NO_BOLT: Bolt = Bolt {
    live: false,
    x: 0,
    y: 0,
    z: 0,
    life: 0,
    flashes: 0,
    seed: 0,
};

static mut BOLTS: [Bolt; BOLT_CAP] = [NO_BOLT; BOLT_CAP];
/// Java's skyFlashTime, in game ticks: 2 while a bolt flashes.
static mut FLASH: u8 = 0;
/// Sim ticks into the current game tick.
static mut SUB: u8 = 0;
/// Strike chunks a game tick: Java's 1 in 100,000 per ticked chunk, with the
/// candidates drawn from the 17 x 17 chunks around the player and kept only
/// when the chunk's centre is within 128 blocks, which leaves about the 201
/// chunks the wiki counts.
const STRIKE_DIE: u32 = 100_000;
const STRIKE_SPAN: i32 = 17;
const STRIKE_R: i32 = 128;
/// Lightning damage, 5 (minecraft.wiki/w/Thunderstorm#Lightning), and the
/// 8 s it sets an entity alight for (Entity.thunderHit).
const BOLT_DAMAGE: i32 = 5;
const BOLT_BURN: i32 = java_ticks(160);
/// Java renders a bolt within 64 blocks at the default entity distance
/// (LightningBolt.shouldRenderAtSqrDistance); farther strikes are a flash
/// and a roll of thunder.
const BOLT_VIEW: i32 = 64 * BLOCK;

/// Sky flash strength 0..=255 for this frame: Java lerps the sky 45% toward
/// (0.8, 0.8, 1.0) while skyFlashTime is up, fading through its last tick.
pub fn flash() -> i32 {
    let f = unsafe { FLASH } as i32;
    if f >= 2 {
        115
    } else if f == 1 {
        115 * (3 - unsafe { SUB } as i32) / 3
    } else {
        0
    }
}

/// True while Java would light the world as full daylight (LightTexture uses
/// a sky brightness of 1 whenever skyFlashTime > 0).
pub fn flashing() -> bool {
    unsafe { FLASH > 0 }
}

/// A random number below `n`.
fn below(n: u32) -> u32 {
    world_rand() % n
}

/// The ground a bolt lands on in a column: one above the highest block that
/// blocks movement or holds a fluid (Java's MOTION_BLOCKING heightmap).
/// Leaves count; plants, fire and portals do not. A column outside the
/// loaded ring answers from the generator's height.
fn strike_top(bx: i32, bz: i32) -> i32 {
    if !world::column_loaded(bx, bz) {
        return world::surface_y(bx, bz);
    }
    let mut y = world::CH - 1;
    while y > 0 {
        let b = world::get(bx, y, bz);
        if b != AIR && !world::is_cross_plant(b) {
            return y + 1;
        }
        y -= 1;
    }
    1
}

/// Open sky over a cell: nothing but air and plants above it.
pub fn sky_above(bx: i32, by: i32, bz: i32) -> bool {
    if !world::column_loaded(bx, bz) {
        return true;
    }
    let mut y = by.max(0);
    while y < world::CH {
        let b = world::get(bx, y, bz);
        if b != AIR && !world::is_cross_plant(b) {
            return false;
        }
        y += 1;
    }
    true
}

/// Rain, not snow and not dry air: Java's lightning needs it falling on the
/// target, and deserts get none while snowy ground gets snow.
fn rains_on(bx: i32, by: i32, bz: i32) -> bool {
    let b = world::biome_at(bx, bz, by);
    b != world::B_DESERT && b != world::B_SNOW
}

/// One sim tick of lightning: rolls strikes and runs the bolts on the game
/// tick (every third sim tick), as ServerLevel.tickChunk and
/// LightningBolt.tick do.
#[inline(never)]
pub fn lightning_tick(p: &mut Player) {
    unsafe {
        SUB += 1;
        if SUB < 3 {
            return;
        }
        SUB = 0;
        if FLASH > 0 {
            FLASH -= 1;
        }
    }
    #[cfg(all(feature = "storm-lab", not(feature = "storm-calm")))]
    lab_strike(p);
    let mut i = 0;
    while i < BOLT_CAP {
        if unsafe { BOLTS[i].live } {
            bolt_tick(i, p);
        }
        i += 1;
    }
    if world::dimension() != world::DIM_OVERWORLD || !(raining() && thundering()) {
        return;
    }
    let r = below(STRIKE_DIE) as i32;
    if r >= STRIKE_SPAN * STRIKE_SPAN {
        return;
    }
    let (pbx, pbz) = (world_to_block_x(p.x), world_to_block_z(p.z));
    let cx = floor_div(pbx, 16) + r % STRIKE_SPAN - STRIKE_SPAN / 2;
    let cz = floor_div(pbz, 16) + r / STRIKE_SPAN - STRIKE_SPAN / 2;
    let (dx, dz) = (cx * 16 + 8 - pbx, cz * 16 + 8 - pbz);
    if dx * dx + dz * dz >= STRIKE_R * STRIKE_R {
        return;
    }
    let rc = below(256) as i32;
    strike_column(cx * 16 + (rc & 15), cz * 16 + (rc >> 4), p);
}

/// Aim a strike at a column, Java's findLightningTargetAround: the ground
/// there, unless a living thing that can see the sky stands within 3 blocks
/// of it, from 3 below the ground up, in which case one of them, at random.
/// Then the strike needs open sky and rain at the spot.
#[inline(never)]
#[optimize(size)] // weather changes and strikes are rare: bytes over cycles
fn strike_column(bx: i32, bz: i32, p: &Player) {
    let by = strike_top(bx, bz);
    let (mut tx, mut ty, mut tz) = (bx * BLOCK + BLOCK / 2, by * BLOCK, bz * BLOCK + BLOCK / 2);
    let (pbx, pby, pbz) = (
        world_to_block_x(p.x),
        world_to_block_y(p.y),
        world_to_block_z(p.z),
    );
    let player_in = (pbx - bx).abs() <= 3
        && (pbz - bz).abs() <= 3
        && pby >= by - 3
        && sky_above(pbx, pby, pbz)
        && p.health > 0;
    let n = mob::thunder_candidates(bx, by, bz) + player_in as usize;
    if n > 0 {
        let k = below(n as u32) as usize;
        if player_in && k == n - 1 {
            tx = p.x;
            ty = p.y;
            tz = p.z;
        } else if let Some((x, y, z)) = mob::thunder_candidate(bx, by, bz, k) {
            tx = x;
            ty = y;
            tz = z;
        }
    }
    let (sx, sy, sz) = (
        world_to_block_x(tx),
        world_to_block_y(ty),
        world_to_block_z(tz),
    );
    if !sky_above(sx, sy, sz) || !rains_on(sx, sy, sz) {
        return;
    }
    spawn_bolt(tx, ty, tz);
}

/// A bolt at a point (world units), as `/summon lightning_bolt` would.
#[inline(never)]
#[optimize(size)] // weather changes and strikes are rare: bytes over cycles
pub fn spawn_bolt(x: i32, y: i32, z: i32) {
    unsafe {
        // Reuse the slot of the oldest bolt if both are busy.
        let mut s = 0;
        while s < BOLT_CAP && BOLTS[s].live {
            s += 1;
        }
        if s == BOLT_CAP {
            s = 0;
        }
        BOLTS[s] = Bolt {
            live: true,
            x,
            y,
            z,
            life: 2,
            flashes: below(3) as u8 + 1,
            seed: world_rand(),
        };
    }
}

/// Java's LightningBolt.tick, once a game tick.
#[inline(never)]
#[optimize(size)] // weather changes and strikes are rare: bytes over cycles
fn bolt_tick(i: usize, p: &mut Player) {
    let mut b = unsafe { BOLTS[i] };
    if b.life == 2 {
        let d = ((b.x - p.x).abs() + (b.z - p.z).abs()) / BLOCK;
        // Thunder is heard everywhere in the dimension; the impact (Java's
        // explosion sound at pitch 0.5-0.7, volume 2.0) carries 32 blocks.
        sfx::thunder(80 + below(21));
        if d < 32 {
            sfx::bolt_impact(d, 50 + below(21));
        }
        // Fire where it lands and at up to four spots in the 3x3x3 around
        // it, on Normal difficulty (LightningBolt.spawnFire(4)).
        spawn_fire(&b, 4);
    }
    b.life -= 1;
    if b.life < 0 {
        if b.flashes == 0 {
            unsafe { BOLTS[i] = NO_BOLT };
            return;
        } else if (b.life as i32) < -(below(10) as i32) {
            b.flashes -= 1;
            b.life = 1;
            b.seed = world_rand();
            spawn_fire(&b, 0);
        }
    }
    if b.life >= 0 {
        unsafe { FLASH = 2 };
        hit_entities(&b, p);
    }
    unsafe { BOLTS[i] = b };
}

/// Java's spawnFire: fire in the bolt's own cell if it is air and fire can
/// stand there, then `extra` more tries at random cells of the 3x3x3 around
/// it.
#[inline(never)]
#[optimize(size)] // weather changes and strikes are rare: bytes over cycles
fn spawn_fire(b: &Bolt, extra: u32) {
    let (bx, by, bz) = (
        world_to_block_x(b.x),
        world_to_block_y(b.y),
        world_to_block_z(b.z),
    );
    if !world::column_loaded(bx, bz) {
        return;
    }
    world::lightning_fire(bx, by, bz);
    let mut k = 0;
    while k < extra {
        let r = world_rand();
        world::lightning_fire(
            bx + (r % 3) as i32 - 1,
            by + ((r >> 4) % 3) as i32 - 1,
            bz + ((r >> 8) % 3) as i32 - 1,
        );
        k += 1;
    }
}

/// Everything in Java's strike box, 3 blocks around the bolt and from 3
/// below it to 9 above: 5 damage and 8 s of fire to the living (rain puts
/// the fire straight out where it falls), sappers charged, dropped items
/// burnt up. Damage immunity keeps a creature that is struck on several
/// flashes to the first 5 hp.
#[inline(never)]
#[optimize(size)] // weather changes and strikes are rare: bytes over cycles
fn hit_entities(b: &Bolt, p: &mut Player) {
    let r = 3 * BLOCK;
    if (p.x - b.x).abs() <= r
        && (p.z - b.z).abs() <= r
        && p.y >= b.y - r
        && p.y <= b.y + 9 * BLOCK
        && p.health > 0
    {
        mob::add_hazard(BOLT_DAMAGE);
        p.burn = p.burn.max(BOLT_BURN);
    }
    mob::thunder_hit(b.x, b.y, b.z, BOLT_BURN as u16);
    burn_drops(b.x, b.y, b.z);
}

// ---- Drawing ---------------------------------------------------------------

/// Java's LightningBoltRenderer: three strands of 16-block segments, the main
/// one eight segments long from 128 blocks up down to the ground, wandering
/// up to 5 blocks a segment, and two shorter forks wandering 15. Each segment
/// is drawn as four nested boxes 0.1 + 0.2 j blocks wide, colour (0.45, 0.45,
/// 0.5) at alpha 0.3 blended additively. A box shows two of its sides, so a
/// camera-facing ribbon here adds twice that colour per layer.
const BOLT_RGB: (u8, u8, u8) = (69, 69, 77);
const SEG_H: i32 = 16 * BLOCK;

/// A little LCG for the bolt's shape, reseeded on every pass as Java reseeds
/// its RandomSource from the bolt's seed.
struct Shape(u32);
impl Shape {
    fn step(&mut self, n: i32) -> i32 {
        self.0 = self.0.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        ((self.0 >> 16) % n as u32) as i32
    }
}

/// Draw every live bolt within view range into the world OT. Runs after the
/// mobs (no UI packets pending), with the GTE translation already zero.
#[inline(never)]
pub fn render_bolts(cam: &Camera) {
    let mut i = 0;
    while i < BOLT_CAP {
        let b = unsafe { BOLTS[i] };
        i += 1;
        if !b.live {
            continue;
        }
        let (dx, dz) = (b.x - cam.x, b.z - cam.z);
        if dx.abs() > BOLT_VIEW || dz.abs() > BOLT_VIEW {
            continue;
        }
        let d = isqrt_dist(dx, dz);
        if d > BOLT_VIEW {
            continue;
        }
        // The main strand's offsets, top to bottom (Java's afloat arrays).
        let mut r = Shape(b.seed);
        let mut ox = [0i32; 8];
        let mut oz = [0i32; 8];
        let (mut fx, mut fz) = (0, 0);
        let mut k = 8;
        while k > 0 {
            k -= 1;
            ox[k] = fx;
            oz[k] = fz;
            fx += r.step(11) - 5;
            fz += r.step(11) - 5;
        }
        let mut layer = 0;
        while layer < 4 {
            let mut r1 = Shape(b.seed);
            let mut strand = 0;
            while strand < 3 {
                let (top, bottom) = if strand == 0 {
                    (7, 0)
                } else {
                    (7 - strand, 5 - strand)
                };
                let mut sx = ox[top as usize] - fx;
                let mut sz = oz[top as usize] - fz;
                let mut j1 = top;
                while j1 >= bottom {
                    let (ux, uz) = (sx, sz);
                    if strand == 0 {
                        sx += r1.step(11) - 5;
                        sz += r1.step(11) - 5;
                    } else {
                        sx += r1.step(31) - 15;
                        sz += r1.step(31) - 15;
                    }
                    // Half-widths in 1/1000 block: 0.1 + 0.2 layer, the main
                    // strand thickening toward the top.
                    let w = 100 + 200 * layer;
                    let (wb, wt) = if strand == 0 {
                        (w * (j1 + 10) / 10, w * (j1 + 9) / 10)
                    } else {
                        (w, w)
                    };
                    segment(
                        cam,
                        (b.x + sx * BLOCK, b.y + j1 * SEG_H, b.z + sz * BLOCK),
                        (b.x + ux * BLOCK, b.y + (j1 + 1) * SEG_H, b.z + uz * BLOCK),
                        wb * BLOCK / 1000,
                        wt * BLOCK / 1000,
                    );
                    j1 -= 1;
                }
                strand += 1;
            }
            layer += 1;
        }
    }
}

fn isqrt_dist(dx: i32, dz: i32) -> i32 {
    // Both under 64 blocks here, so the squares fit an i32.
    psx_math::int32::isqrt_i32(dx * dx + dz * dz)
}

/// Camera space for a world point: (x, y, depth), no far-plane cull.
fn view(cam: &Camera, p: (i32, i32, i32)) -> (i32, i32, i32) {
    let dx = p.0 - cam.x;
    let dy = p.1 - cam.y;
    let dz = p.2 - cam.z;
    let x1 = ((dx * cam.cy) - (dz * cam.sy)) >> 12;
    let z1 = ((dx * cam.sy) + (dz * cam.cy)) >> 12;
    let y2 = ((dy * cam.cp) - (z1 * cam.sp)) >> 12;
    let z2 = ((dy * cam.sp) + (z1 * cam.cp)) >> 12;
    if cam.roll != 0 {
        let a = (cam.roll & 0x0FFF) as u16;
        let (sr, cr) = (sincos::sin_q12(a), sincos::cos_q12(a));
        (
            ((cr * x1) + (sr * y2)) >> 12,
            ((cr * y2) - (sr * x1)) >> 12,
            z2,
        )
    } else {
        (x1, y2, z2)
    }
}

/// One segment as a screen ribbon from `a` (bottom) to `b` (top), half-widths
/// in world units, cut at the near plane, sorted into the OT by depth (past
/// the far plane it sits behind all terrain, on the sky).
#[inline(never)]
fn segment(cam: &Camera, a: (i32, i32, i32), b: (i32, i32, i32), wa: i32, wb: i32) {
    let (mut a, mut b) = (view(cam, a), view(cam, b));
    let near = NEAR_Z;
    if a.2 < near && b.2 < near {
        return;
    }
    if a.2 < near || b.2 < near {
        // Slide the near end along the segment to the near plane.
        let (n, f) = if a.2 < near { (&mut a, b) } else { (&mut b, a) };
        let t = ((near - n.2) << 8) / (f.2 - n.2).max(1);
        n.0 += ((f.0 - n.0) * t) >> 8;
        n.1 += ((f.1 - n.1) * t) >> 8;
        n.2 = near;
    }
    let px = |v: (i32, i32, i32)| (CX as i32 + v.0 * PROJ_H / v.2).clamp(-1023, 1023);
    let py = |v: (i32, i32, i32)| (CY as i32 - v.1 * PROJ_H / v.2).clamp(-511, 511);
    let (xa, ya, xb, yb) = (px(a), py(a), px(b), py(b));
    if (xa < -40 && xb < -40)
        || (xa > 360 && xb > 360)
        || (ya < -40 && yb < -40)
        || (ya > 280 && yb > 280)
    {
        return;
    }
    let ha = (wa * PROJ_H / a.2).max(1);
    let hb = (wb * PROJ_H / b.2).max(1);
    let slot = depth_slot((a.2 + b.2) >> 1);
    ui_quad_add_depth(
        [
            ((xb - hb) as i16, yb as i16),
            ((xb + hb) as i16, yb as i16),
            ((xa - ha) as i16, ya as i16),
            ((xa + ha) as i16, ya as i16),
        ],
        BOLT_RGB,
        slot,
    );
}

/// storm-lab: a bolt 12 blocks ahead of the player every 4 s, 4 blocks to
/// the right and then to the left, so captures and gates see strikes in view.
#[cfg(all(feature = "storm-lab", not(feature = "storm-calm")))]
fn lab_strike(p: &Player) {
    static mut T: u32 = 0;
    unsafe {
        T += 1;
        if T % 80 != 40 {
            return;
        }
        let side = if (T / 80) % 2 == 0 { 4 } else { -4 };
        let (s, c) = (sincos::sin_q12(p.yaw), sincos::cos_q12(p.yaw));
        let x = p.x + ((s * 12 * BLOCK + c * side * BLOCK) >> 12);
        let z = p.z + ((c * 12 * BLOCK - s * side * BLOCK) >> 12);
        let (bx, bz) = (world_to_block_x(x), world_to_block_z(z));
        let by = strike_top(bx, bz);
        spawn_bolt(bx * BLOCK + BLOCK / 2, by * BLOCK, bz * BLOCK + BLOCK / 2);
    }
}

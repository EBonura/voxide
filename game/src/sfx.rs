//! Sampled sound effects: CC0 recordings (Kenney.nl packs and freesound.org,
//! credited in assets/pack/CREDITS.md) cooked to SPU-ADPCM by
//! tools/voxide-tools (convert-sfx) and uploaded to SPU RAM once at boot -- ~110 KiB of
//! SPU RAM, nothing streamed. Each cooked sample ends in a self-looping
//! silent block, so a finished one-shot parks its voice on silence until the
//! round-robin re-keys it.
//!
//! There is deliberately no music: the world's own sounds carry the
//! atmosphere.

#![allow(dead_code)]

use crate::sfxdata::{
    Sample, SAMPLES, SFX_CHUNK_ID, S_BARK, S_BONES, S_BREAK, S_CHEST, S_CHICKEN, S_CLICK,
    S_CONFIRM, S_COW, S_DIG_SOFT, S_DIG_STONE, S_DIG_WOOD, S_DOOR, S_EAT, S_EXPLODE, S_HISS,
    S_HURT, S_PIG, S_PLACE, S_SHEEP, S_SPLASH, S_STEP_GRASS, S_STEP_SAND, S_STEP_STONE,
    S_STEP_WOOD, S_ZOMBIE,
};
use psx_pack::cd::{self, SectorReader, SECTOR_WORDS};
use psx_pack::SECTOR_BYTES;
use psx_sfx::{LoopingSample, OneShot, Player, Sample as SfxSample};
use psx_spu::{Pitch, SpuAddr, Voice, Volume};

/// SPU RAM byte offset of the sample bank: just above the 0x0000..0x1000
/// SPU/BIOS reserved page, 8-byte aligned.
const SPU_BASE: u32 = 0x1010;

// Voices reserved for SFX, cycled round-robin so a new sound rarely cuts off
// a still-ringing one.
const SFX_VOICES: usize = 4;
/// Round-robin across those voices, with the correct key-on and the cutoff
/// behind it.
static mut PLAYER: Player<SFX_VOICES> =
    Player::new([Voice::V0, Voice::V1, Voice::V2, Voice::V3], 60);

/// The SFX clock, in frames.
///
/// Its own counter because sounds play from more than one loop -- the world
/// and the main menu each run their own -- and a cutoff has to be measured on
/// a clock that spans both. (It was originally separate for a worse reason:
/// the world loop's `frame` doubled as the day clock and jumped forward when
/// sleeping, so a deadline against it expired early. That is fixed; this
/// stands on its own.)
static mut TICK: u32 = 0;

/// Advance the SFX clock and silence any voice whose sample has ended. Call
/// once a frame, before anything that might start a sound.
///
/// Every sample's blob ends in a self-looping silent block, so a finished
/// voice parks rather than wandering. This is the other half: parked is not
/// stopped, and a voice sitting on that block still holds a live envelope.
pub fn tick() {
    unsafe {
        TICK = TICK.wrapping_add(1);
        PLAYER.tick(TICK);
        THUNDER_VOICE.tick(TICK);
    }
    thunder_step();
}
static mut READY: bool = false;

// Master SFX volume in percent, set from the SETTINGS card.
static mut VOL_PCT: i32 = 100;

pub fn volume_pct() -> i32 {
    unsafe { VOL_PCT }
}
pub fn set_volume_pct(p: i32) {
    unsafe { VOL_PCT = p.clamp(0, 100) };
}

/// Reset the SPU and stream the cooked sample bank from the disc's WORLD.PAK
/// straight into SPU RAM, one sector at a time -- the ~88 KiB blob never
/// touches main RAM (it used to sit in .data forever). Call once at boot.
#[optimize(size)] // boot-time, once: its bytes are worth more than its cycles
pub fn init() {
    psx_spu::init();
    let ok = unsafe { stream_bank() };
    if !ok {
        // No disc chunk (or a read fault): the game plays on, silently.
        psx_rt::tty::println("sfx: WORLD.PAK bank load failed; muted");
    }
    unsafe {
        READY = ok;
        if ok {
            thunder_init(BANK_END);
        }
    }
}

unsafe fn stream_bank() -> bool {
    let mut rd = SectorReader::new();
    let mut scratch = [0u32; SECTOR_WORDS];
    let Some(entry) = cd::find_entry(
        &mut rd,
        cd::WORLD_PACK_DEFAULT_LBA,
        SFX_CHUNK_ID,
        &mut scratch,
    ) else {
        return false;
    };
    if !rd.prepare() || !rd.start_read(cd::WORLD_PACK_DEFAULT_LBA + entry.sector_offset) {
        rd.stop();
        return false;
    }
    let mut left = entry.byte_size as usize;
    let mut addr = SPU_BASE;
    while left > 0 {
        if !rd.read_sector(&mut scratch) {
            rd.stop();
            return false;
        }
        // Round the tail up to a whole ADPCM block; the cooked bank is
        // 16-byte-block aligned so the pad never reaches a played sample.
        let n = (left.min(SECTOR_BYTES) + 15) & !15;
        let bytes = core::slice::from_raw_parts(scratch.as_ptr() as *const u8, n);
        psx_spu::upload_adpcm(SpuAddr::new(addr), bytes);
        addr += n as u32;
        left = left.saturating_sub(SECTOR_BYTES);
    }
    rd.stop();
    BANK_END = addr;
    true
}

/// Key a cooked sample on the next round-robin voice. `pct` scales pitch in
/// percent of the sample's native rate (100 = as recorded).
fn play(id: usize, vol: i16, pct: u32) {
    unsafe {
        if !READY {
            return;
        }
        let s: &Sample = &SAMPLES[id];
        let vol = ((vol as i32) * VOL_PCT / 100) as i16;
        // psx-sfx owns the key-on and the cutoff. The cooked bank ends every
        // sample with a self-looping silent block (flags 0x07: loop-start,
        // repeat, end), so silicon latches the repeat address as it decodes
        // that block and the voice parks there on its own. The cutoff then
        // silences it outright, which parking does not.
        let sample = SfxSample::resident(
            SpuAddr::new(SPU_BASE + s.off),
            s.rate as u32,
            s.blocks as u32,
        );
        let shot = OneShot::new(sample, Volume(vol))
            .with_pitch(Pitch::for_frequency(s.rate as u32 * pct / 100, 44100));
        PLAYER.play(&shot, TICK);
    }
}

// ---- Game sound effects ----

/// Repeated hit while mining, voiced by the block's material class
/// (step_mat: 0 soft, 1 stone, 2 sand, 3 wood); `n` jitters the pitch.
pub fn dig(mat: u32, n: u32) {
    let id = match mat {
        1 => S_DIG_STONE,
        3 => S_DIG_WOOD,
        _ => S_DIG_SOFT,
    };
    play(id, 0x2400, 92 + (n % 3) * 8);
}
/// A block finished breaking.
pub fn break_block() {
    play(S_BREAK, 0x2C00, 100);
}
/// A block was placed.
pub fn place() {
    play(S_PLACE, 0x2600, 100);
}
/// Footstep by surface (0 soft turf, 1 stone, 2 sand/snow, 3 wood), kept
/// quiet with a two-step pitch alternation.
pub fn step_on(mat: u32, n: u32) {
    let id = match mat {
        1 => S_STEP_STONE,
        2 => S_STEP_SAND,
        3 => S_STEP_WOOD,
        _ => S_STEP_GRASS,
    };
    play(id, 0x1400, 95 + (n % 2) * 10);
}
/// Player took damage.
pub fn hurt() {
    play(S_HURT, 0x3000, 100);
}
/// Menu cursor move / generic UI tick.
pub fn blip() {
    play(S_CLICK, 0x2000, 100);
}
/// Confirm: craft, deposit, sleep.
pub fn confirm() {
    play(S_CONFIRM, 0x2400, 100);
}
/// Eating food.
pub fn eat() {
    play(S_EAT, 0x2400, 100);
}
/// Sapper / TNT explosion.
pub fn explode() {
    play(S_EXPLODE, 0x3800, 100);
}
/// Entering (or bucketing) water.
pub fn splash() {
    play(S_SPLASH, 0x2800, 100);
}
/// Hit a mob: the punch, pitched up so it reads apart from taking damage.
pub fn hit_mob() {
    play(S_HURT, 0x2800, 112);
}

// ---- Mob voices: distance-attenuated calls. ----

fn vol_at(base: i16, dist_blocks: i32) -> i16 {
    (base as i32 - dist_blocks * 0x0180).max(0x0600) as i16
}

pub fn pig(d: i32) {
    play(S_PIG, vol_at(0x2C00, d), 100);
}
pub fn cow(d: i32) {
    play(S_COW, vol_at(0x2C00, d), 100);
}
pub fn sheep(d: i32) {
    play(S_SHEEP, vol_at(0x2A00, d), 100);
}
pub fn chicken(d: i32) {
    play(S_CHICKEN, vol_at(0x2600, d), 100);
}
pub fn zombie(d: i32) {
    play(S_ZOMBIE, vol_at(0x3000, d), 100);
}
/// Skeleton: the bone rattle.
pub fn skeleton(d: i32) {
    play(S_BONES, vol_at(0x2400, d), 100);
}
pub fn wolf(d: i32) {
    play(S_BARK, vol_at(0x2A00, d), 100);
}
/// Spider: the rattle again, faster and quieter -- a chitter.
pub fn spider(d: i32) {
    play(S_BONES, vol_at(0x1C00, d), 135);
}
/// Sapper fuse lit: the dreaded hiss.
pub fn sapper_hiss() {
    play(S_HISS, 0x2C00, 100);
}

// ---- Interaction sounds. ----

/// Door swings.
pub fn door() {
    play(S_DOOR, 0x2800, 100);
}
/// Chest or furnace opened.
pub fn chest_open() {
    play(S_CHEST, 0x2800, 100);
}

// ---- Weather: thunder ----

/// SPU RAM just past the cooked bank, where the thunder is built at boot.
static mut BANK_END: u32 = SPU_BASE;
/// The thunder roll, synthesised into SPU RAM at boot (no recording, no disc
/// data): 4 s of filtered noise at 11,025 Hz.
static mut THUNDER: Option<SfxSample> = None;
/// Thunder rolls on for seconds, so it gets a voice of its own: on the shared
/// round-robin the next footstep would cut it off.
static mut THUNDER_VOICE: Player<1> = Player::new([Voice::V4], 60);

const THUNDER_HZ: u32 = 11_025;
const THUNDER_LEN: u32 = 4 * THUNDER_HZ;
const THUNDER_BLOCKS: u32 = (THUNDER_LEN + 27) / 28;

/// The rolls: start (ms), peak (Q15), decay time constant (ms). One sharp
/// crack-led first peal, then the rumble rolling on in uneven waves.
const ROLLS: [(u32, i32, u32); 5] = [
    (0, 32_767, 420),
    (330, 24_000, 520),
    (900, 27_000, 600),
    (1_650, 17_000, 700),
    (2_450, 11_000, 800),
];

/// Deterministic synthesis state: noise, two low-passes, the envelope.
struct Synth {
    seed: u32,
    crack: i32,
    lp1: i32,
    lp2: i32,
    wob: i32,
    n: u32,
    /// The envelopes, refreshed every 28 samples (2.5 ms): per-sample
    /// envelopes cost a dozen divides a sample, a second of boot.
    env: i32,
    ce: i32,
}

impl Synth {
    const fn new() -> Self {
        Self {
            seed: 0x7C3A_91E5,
            crack: 0,
            lp1: 0,
            lp2: 0,
            wob: 0,
            n: 0,
            env: 0,
            ce: 0,
        }
    }

    /// Envelope of one roll at sample `n`, Q15: a 30 ms linear attack, then
    /// exponential decay, done as exp(-t/tau) ~ (1 - t/(4 tau))^4 on Q15.
    fn roll(n: u32, r: (u32, i32, u32)) -> i32 {
        let start = r.0 * THUNDER_HZ / 1000;
        if n < start {
            return 0;
        }
        let t = n - start;
        let att = 30 * THUNDER_HZ / 1000;
        if t < att {
            return (r.1 as u32 * t / att) as i32;
        }
        let span = 4 * r.2 * THUNDER_HZ / 1000;
        let u = t - att;
        if u >= span {
            return 0;
        }
        let k = ((span - u) << 15) / span; // 1 - t/(4 tau), Q15
        let k2 = (k * k) >> 15;
        let k4 = (k2 * k2) >> 15;
        ((r.1 as u32 * k4) >> 15) as i32
    }

    /// Next sample, about +/-32k before scaling.
    fn next(&mut self) -> i32 {
        self.seed = self
            .seed
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        let w = (self.seed >> 16) as i32 - 32_768;
        // Crack: bright noise (one-pole at about 2 kHz), gone in 0.2 s.
        self.crack += ((w - self.crack) * 13_000) >> 15;
        // Rumble: noise through two one-poles at about 90 Hz, made up in gain.
        self.lp1 += ((w - self.lp1) * 1_700) >> 15;
        self.lp2 += ((self.lp1 - self.lp2) * 1_700) >> 15;
        // Slow wobble (about 3 Hz) so the roll swells and sags.
        self.wob += ((w - self.wob) * 60) >> 15;
        let n = self.n;
        self.n += 1;
        if n % 28 == 0 {
            let mut env = 0;
            let mut i = 0;
            while i < ROLLS.len() {
                env = env.max(Self::roll(n, ROLLS[i]));
                i += 1;
            }
            self.env = env;
            self.ce = Self::roll(n, (0, 32_767, 50));
        }
        let (env, ce) = (self.env, self.ce);
        let wob = 24_576 + (self.wob * 12).clamp(-8_192, 8_192); // 0.5..1.0, Q15
        let rumble = ((self.lp2 * 6) * ((env * wob) >> 15)) >> 15;
        let crack = (self.crack * ce) >> 16;
        rumble + crack
    }
}

/// SPU-ADPCM, filter 0 to 4 per 28-sample block (the same coding as
/// tools/voxide-tools, with the shift chosen from the block's largest
/// residual instead of a full search, to keep boot short).
const ADPCM_K: [(i32, i32); 5] = [(0, 0), (60, 0), (115, -52), (98, -55), (122, -60)];

fn adpcm_block(s: &[i32; 28], p1: &mut i32, p2: &mut i32, out: &mut [u8]) {
    // Pick the filter whose residuals (on the source) stay smallest.
    let mut best = (i32::MAX, 0usize);
    let mut f = 0;
    while f < 5 {
        let (k0, k1) = ADPCM_K[f];
        let (mut a, mut b) = (*p1, *p2);
        let mut m = 0;
        let mut i = 0;
        while i < 28 {
            let r = s[i] - ((a * k0 + b * k1) >> 6);
            m = m.max(r.abs());
            b = a;
            a = s[i];
            i += 1;
        }
        if m < best.0 {
            best = (m, f);
        }
        f += 1;
    }
    let f = best.1;
    // Largest shift (finest step) whose range covers the residual.
    let mut range = 12;
    while range > 0 && (7 << (12 - range)) < best.0 {
        range -= 1;
    }
    let shift = range;
    let (k0, k1) = ADPCM_K[f];
    out[0] = (shift as u8) | ((f as u8) << 4);
    out[1] = 0;
    let step = 12 - shift;
    let mut i = 0;
    while i < 28 {
        let pred = (*p1 * k0 + *p2 * k1) >> 6;
        let r = s[i] - pred;
        let q = if step > 0 {
            ((r + (1 << (step - 1))) >> step).clamp(-8, 7)
        } else {
            r.clamp(-8, 7)
        };
        let dec = ((q << step) + pred).clamp(-32_768, 32_767);
        *p2 = *p1;
        *p1 = dec;
        let nib = (q & 0xF) as u8;
        if i % 2 == 0 {
            out[2 + i / 2] = nib;
        } else {
            out[2 + i / 2] |= nib << 4;
        }
        i += 1;
    }
}

/// Gain to 0.9 of full scale, Q12. The synthesis is deterministic and peaks
/// at 28,623 (measured by running the same code on the host), so there is no
/// peak-finding pass to pay for on the console.
const THUNDER_GAIN: i32 = 4_220;
/// Blocks synthesised and uploaded per call of `thunder_step`: about a
/// quarter of a vblank, once a frame, for the first 25 frames of the menu.
const THUNDER_STEP: usize = 64;

/// The thunder being built: synthesis state and the next block to write.
struct Gen {
    s: Synth,
    b: u32,
    at: u32,
    p1: i32,
    p2: i32,
}
static mut GEN: Option<Gen> = None;
static mut THUNDER_AT: u32 = 0;

/// Start building the thunder at `addr`. All 4 s in one go cost two thirds of
/// a second of boot, so it is spread over the first menu frames instead
/// (`thunder_step`, from `tick`); a strike before it is ready is silent.
fn thunder_init(addr: u32) {
    unsafe {
        THUNDER_AT = addr;
        GEN = Some(Gen {
            s: Synth::new(),
            b: 0,
            at: addr,
            p1: 0,
            p2: 0,
        });
    }
}

/// Synthesise, encode and upload the next THUNDER_STEP blocks, ending with
/// the self-looping silent block every sample here ends with.
#[inline(never)]
fn thunder_step() {
    let g = unsafe {
        match (*core::ptr::addr_of_mut!(GEN)).as_mut() {
            Some(g) => g,
            None => return,
        }
    };
    let mut buf = [0u32; THUNDER_STEP * 4]; // word aligned for DMA
    let bytes =
        unsafe { core::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut u8, THUNDER_STEP * 16) };
    let mut blk = [0i32; 28];
    let mut fill = 0;
    while fill < THUNDER_STEP && g.b <= THUNDER_BLOCKS {
        let out = &mut bytes[fill * 16..fill * 16 + 16];
        if g.b < THUNDER_BLOCKS {
            let mut i = 0;
            while i < 28 {
                let v = if g.b * 28 + (i as u32) < THUNDER_LEN {
                    g.s.next()
                } else {
                    0
                };
                blk[i] = ((v * THUNDER_GAIN) >> 12).clamp(-32_768, 32_767);
                i += 1;
            }
            adpcm_block(&blk, &mut g.p1, &mut g.p2, out);
        } else {
            out.fill(0);
            out[1] = 0x07; // loop start + loop end + repeat: park on silence
        }
        fill += 1;
        g.b += 1;
    }
    psx_spu::upload_adpcm(SpuAddr::new(g.at), &bytes[..fill * 16]);
    g.at += fill as u32 * 16;
    if g.b > THUNDER_BLOCKS {
        unsafe {
            THUNDER = Some(SfxSample::resident(
                SpuAddr::new(THUNDER_AT),
                THUNDER_HZ,
                THUNDER_BLOCKS,
            ));
            GEN = None;
        }
        rain_build(unsafe { THUNDER_AT } + (THUNDER_BLOCKS + 1) * 16);
    }
}

/// Left and right levels for a sound `pan` (Q12 sine of its bearing, -4096
/// hard left to 4096 hard right): the far ear drops to a quarter, as a
/// positional source does in Java's OpenAL mix.
fn panned(vol: i32, pan: i32) -> (Volume, Volume) {
    let l = vol * (4096 - pan.max(0) * 3 / 4) / 4096;
    let r = vol * (4096 + pan.min(0) * 3 / 4) / 4096;
    (Volume(l as i16), Volume(r as i16))
}

/// A peal of thunder at `pct` percent pitch (Java's random 0.8 to 1.0) from
/// bearing `pan`. Java plays it at volume 10,000, which carries 160,000
/// blocks: full loudness wherever the strike is, with no delay, placed by
/// direction.
pub fn thunder(pct: u32, pan: i32) {
    unsafe {
        if !READY {
            return;
        }
        let Some(sample) = THUNDER else {
            return;
        };
        let vol = 0x3800 * VOL_PCT / 100;
        let shot = OneShot::new(sample, Volume(vol as i16))
            .with_pitch(Pitch::for_frequency(THUNDER_HZ * pct / 100, 44100));
        THUNDER_VOICE.play(&shot, TICK);
        let (l, r) = panned(vol, pan);
        THUNDER_VOICE.voice(0).set_volume(l, r);
    }
}

/// The crack where a bolt lands: Java's explosion sound at pitch 0.5 to 0.7
/// and volume 2.0, so it fades out linearly over 32 blocks, from bearing
/// `pan`.
pub fn bolt_impact(dist_blocks: i32, pct: u32, pan: i32) {
    let v = 0x3800 * (32 - dist_blocks.clamp(0, 32)) / 32;
    if v <= 0x0200 {
        return;
    }
    unsafe {
        if !READY {
            return;
        }
        let s: &Sample = &SAMPLES[S_EXPLODE];
        let vol = v * VOL_PCT / 100;
        let sample = SfxSample::resident(
            SpuAddr::new(SPU_BASE + s.off),
            s.rate as u32,
            s.blocks as u32,
        );
        let shot = OneShot::new(sample, Volume(vol as i16))
            .with_pitch(Pitch::for_frequency(s.rate as u32 * pct / 100, 44100));
        let slot = PLAYER.play(&shot, TICK);
        let (l, r) = panned(vol, pan);
        PLAYER.voice(slot).set_volume(l, r);
    }
}

// ---- Weather: rain ----

/// Rain: half a second of synthesised hiss with droplet ticks, looped in
/// hardware on a voice of its own, built after the thunder.
const RAIN_HZ: u32 = 11_025;
const RAIN_BLOCKS: u32 = 197;
/// Gain to 0.9 of full scale, Q12: the synthesis peaks at 28,072 (host run
/// of the same code).
const RAIN_GAIN: i32 = 4_302;
static mut RAIN: Option<SfxSample> = None;
static mut RAIN_ON: bool = false;
static mut RAIN_LEVEL: i32 = 0;
const RAIN_VOICE: Voice = Voice::V5;

/// The rain's synthesis: noise with its lows taken out (the hiss of drops on
/// leaves and ground) and a few hundred short decaying ticks a second.
struct RainSynth {
    seed: u32,
    lp: i32,
    drop: i32,
}

impl RainSynth {
    const fn new() -> Self {
        Self {
            seed: 0x51A7_2C03,
            lp: 0,
            drop: 0,
        }
    }

    fn next(&mut self) -> i32 {
        self.seed = self
            .seed
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        let w = (self.seed >> 16) as i32 - 32_768;
        self.lp += ((w - self.lp) * 8_000) >> 15;
        let hiss = (w - self.lp) >> 2;
        if (self.seed >> 7) % 37 == 0 {
            self.drop = 6_000 + ((self.seed >> 3) & 0x3FFF) as i32;
        }
        self.drop = self.drop * 29 / 32;
        hiss + ((self.drop * w) >> 15)
    }
}

/// Synthesise and upload the rain loop at `addr`: block 0 starts the loop,
/// the last block ends it and repeats (flags 0x04 and 0x03).
#[inline(never)]
#[optimize(size)]
fn rain_build(addr: u32) {
    let mut s = RainSynth::new();
    let mut buf = [0u32; RAIN_BLOCKS as usize * 4];
    let bytes = unsafe {
        core::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut u8, RAIN_BLOCKS as usize * 16)
    };
    let (mut p1, mut p2) = (0, 0);
    let mut blk = [0i32; 28];
    let mut b = 0;
    while b < RAIN_BLOCKS as usize {
        let mut i = 0;
        while i < 28 {
            blk[i] = ((s.next() * RAIN_GAIN) >> 12).clamp(-32_768, 32_767);
            i += 1;
        }
        let out = &mut bytes[b * 16..b * 16 + 16];
        adpcm_block(&blk, &mut p1, &mut p2, out);
        out[1] = if b == 0 {
            0x04
        } else if b + 1 == RAIN_BLOCKS as usize {
            0x03
        } else {
            0
        };
        b += 1;
    }
    psx_spu::upload_adpcm(SpuAddr::new(addr), bytes);
    unsafe {
        RAIN = Some(SfxSample::resident(
            SpuAddr::new(addr),
            RAIN_HZ,
            RAIN_BLOCKS,
        ));
    }
}

/// Set the rain's level, 0 to 4096 of Java's weather.rain volume 0.2, and
/// whether it is the muffled weather.rain.above (volume 0.1, pitch 0.5) a
/// roof over the player gives. Once a game tick; the loop starts the first
/// time it is wanted and then only its level moves.
pub fn rain(level: i32, above: bool) {
    unsafe {
        let Some(sample) = RAIN else {
            return;
        };
        if !READY {
            return;
        }
        if !RAIN_ON {
            if level == 0 {
                return;
            }
            LoopingSample::new(sample, Volume::SILENCE).play(RAIN_VOICE);
            RAIN_ON = true;
        }
        // Ease toward the target over about a quarter second.
        RAIN_LEVEL += (level - RAIN_LEVEL) / 4;
        let full = if above { 0x3800 / 10 } else { 0x3800 / 5 };
        let v = (full * RAIN_LEVEL / 4096 * VOL_PCT / 100) as i16;
        RAIN_VOICE.set_volume(Volume(v), Volume(v));
        RAIN_VOICE.set_pitch(Pitch::for_frequency(
            if above { RAIN_HZ / 2 } else { RAIN_HZ },
            44100,
        ));
    }
}

//! Sampled sound effects: CC0 recordings (Kenney.nl packs and freesound.org,
//! credited in assets/pack/CREDITS.md) cooked to SPU-ADPCM by
//! tools/convert_sfx.py and uploaded to SPU RAM once at boot -- ~110 KiB of
//! SPU RAM, nothing streamed. Each cooked sample ends in a self-looping
//! silent block, so a finished one-shot parks its voice on silence until the
//! round-robin re-keys it.
//!
//! There is deliberately no music: the world's own sounds carry the
//! atmosphere.

#![allow(dead_code)]

use crate::sfxdata::{
    Sample, S_BARK, S_BONES, S_BREAK, S_CHEST, S_CHICKEN, S_CLICK, S_CONFIRM, S_COW, S_DIG_SOFT,
    S_DIG_STONE, S_DIG_WOOD, S_DOOR, S_EAT, S_EXPLODE, S_HISS, S_HURT, S_PIG, S_PLACE, S_SHEEP,
    S_SPLASH, S_STEP_GRASS, S_STEP_SAND, S_STEP_STONE, S_STEP_WOOD, S_ZOMBIE, SAMPLES,
    SFX_CHUNK_ID,
};
use psx_pack::cd::{self, SectorReader, SECTOR_WORDS};
use psx_pack::SECTOR_BYTES;
use psx_sfx::{OneShot, Player, Sample as SfxSample};
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
    }
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
pub fn init() {
    psx_spu::init();
    let ok = unsafe { stream_bank() };
    if !ok {
        // No disc chunk (or a read fault): the game plays on, silently.
        psx_rt::tty::println("sfx: WORLD.PAK bank load failed; muted");
    }
    unsafe {
        READY = ok;
    }
}

unsafe fn stream_bank() -> bool {
    let mut rd = SectorReader::new();
    let mut scratch = [0u32; SECTOR_WORDS];
    let Some(entry) = cd::find_entry(&mut rd, cd::WORLD_PACK_DEFAULT_LBA, SFX_CHUNK_ID, &mut scratch)
    else {
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

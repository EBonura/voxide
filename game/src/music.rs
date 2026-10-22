//! Background music: the pieces in assets/music/tracks.txt (musicdata.rs),
//! streamed from the disc as XA-ADPCM. The drive decodes them in hardware and
//! feeds the SPU's CD input, so the CPU does no audio work; what costs anything
//! is the handful of drive commands that start a piece, stop it and ask where
//! its head is.
//!
//! When a piece plays follows Java Edition (music_plan.rs, AUDIT.md): music is
//! not looped, a random delay separates the pieces, the title screen has its
//! own, the menu music stops when a world loads, and the pause menu does not
//! stop it. The MUSIC slider is Java's: at 0 nothing plays and the delay runs
//! on, and it is saved only by SAVE SETTINGS.
//!
//! The drive cannot read data while a piece streams, so every disc read the
//! game makes (the sound bank, the song files' directory entries) happens in
//! `init`, before the first piece can start. Nothing reads the disc afterwards.
//! Anything that ever does must `stop` first and let the scheduler start the
//! next piece at its own delay.

use crate::music_plan as plan;
use crate::musicdata::{FILES, TRACKS};
use psx_fmv::iso;
use psx_fx::rng::LcgRng;
use psx_io::cd::xa::{DriveSpeed, Event, File, Player};
use psx_io::periph::Cd;
use psx_pack::cd::{SectorReader, SECTOR_WORDS};
use psx_rt::{interrupts, tty};
use psx_spu::CdVolume;

/// The song files are `MUSIC1.XA`, `MUSIC2.XA` and so on in the disc's root
/// directory, encoded for single speed with file number n in `MUSIC<n>.XA`
/// (assets/music/MUSIC<n>.json), one channel per song.

/// Vblanks between asking the drive where its head is: once a second while a
/// song plays, ten times a second for the last ten seconds so the song ends
/// near its own length.
const POLL_VBLANKS: u32 = 60;
const POLL_END_VBLANKS: u32 = 6;
const END_WINDOW_MS: u32 = 10_000;
/// The receipt is rewritten at least this often.
const RECEIPT_VBLANKS: u32 = 30;
/// Wait before asking the drive again after it refused to start a song.
const RETRY_VBLANKS: u32 = 600;

/// Headless scheduling checks only (the `music-lab` feature): the 10 to 20
/// minute delay is 5 to 10 seconds and every song is cut at 12 seconds, so a
/// journey can walk several songs. Never ship.
const LAB: bool = cfg!(feature = "music-lab");
const LAB_LENGTH_MS: u32 = 12_000;

#[derive(Copy, Clone, PartialEq, Eq)]
enum Phase {
    /// No song file on the disc.
    Off,
    /// Ready, but no screen has said when the first song is due.
    Idle,
    /// Counting down to the next song.
    Waiting,
    /// A song is streaming.
    Playing,
}

struct Music {
    phase: Phase,
    files: [Option<File>; FILES],
    player: Player,
    rng: LcgRng,
    /// MUSIC slider, percent.
    volume: u8,
    /// Vblank at which the next song may be picked (Waiting).
    until: u32,
    /// Vblank at which the current song started, and when to ask the drive next.
    started: u32,
    next_poll: u32,
    track: u8,
    /// What the last pick was for: nothing yet 0, the title screen 1, a world 2.
    picked: u8,
    /// The last delay set, in vblanks.
    delay: u32,
    /// Vblank of the last receipt write.
    published: u32,
    plays: u32,
    failures: u32,
}

// SAFETY: the only holder of the CD token after boot; the sound bank loader's
// SectorReader has finished with the drive by the time `init` returns, and
// nothing reads the disc once a song can play.
static mut MUSIC: Music = Music {
    phase: Phase::Off,
    files: [None; FILES],
    player: Player::new(unsafe { Cd::steal() }),
    rng: LcgRng::new(0x4d55_5349),
    volume: 100,
    until: 0,
    started: 0,
    next_poll: 0,
    track: NO_TRACK,
    picked: 0,
    delay: 0,
    published: 0,
    plays: 0,
    failures: 0,
};
const NO_TRACK: u8 = 255;

/// Read-only music receipt for the journeys: magic, version, byte length,
/// phase (off 0, waiting 1, playing 2, not yet scheduled 3), piece (255 none),
/// what the last pick was for (title 1, world 2), the slider percent, the
/// vblanks left until the next pick, pieces started, the drive saw its head
/// inside the file, milliseconds into the piece at the last poll, refused
/// starts, the last delay set in vblanks, and the drive mixer level.
/// Gameplay never reads this block.
#[used]
#[no_mangle]
pub static mut VOXIDE_MUSIC_STATE: [u32; 16] = [
    u32::from_le_bytes(*b"VMUS"),
    1,
    64,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
];

fn m() -> &'static mut Music {
    // SAFETY: single-threaded; no interrupt handler touches the music state.
    unsafe { &mut MUSIC }
}

/// The slider percent from the settings record. Safe before `init`.
pub fn set_volume_pct(percent: i32) {
    let now = interrupts::vblank_count();
    let x = m();
    x.volume = percent.clamp(0, 100) as u8;
    if x.phase == Phase::Off {
        return;
    }
    x.player
        .set_volume(plan::mixer_level(x.volume), plan::mixer_level(x.volume));
    if x.volume == 0 && x.phase == Phase::Playing {
        // Java stops a song the slider takes to zero, and the delay to the
        // next one starts.
        stop_song(now);
    }
    publish();
}

pub fn volume_pct() -> i32 {
    m().volume as i32
}

/// Find the song file on the disc and switch the SPU's CD input on. Call
/// once at boot after `sfx::init` (which resets the SPU) and before the first
/// `tick`.
#[inline(never)]
pub fn init() {
    let x = m();
    x.files = find_song_files();
    // The SPU's CD input at full level; the drive mixer carries the slider.
    psx_spu::set_cd_volume(CdVolume::MAX, CdVolume::MAX);
    psx_spu::enable_cd_audio(true);
    if x.files.iter().all(Option::is_none) {
        tty::println("music: no MUSIC<n>.XA on the disc; no music");
        publish();
        return;
    }
    x.phase = Phase::Idle;
    x.player
        .set_volume(plan::mixer_level(x.volume), plan::mixer_level(x.volume));
    publish();
}

/// The song files' places in the root directory, looked up by name so no LBA
/// is baked in.
fn find_song_files() -> [Option<File>; FILES] {
    let mut files = [None; FILES];
    // The reader owns the drive until `stop`; the music player does not run
    // yet, and the sound bank's own reader has finished.
    // SAFETY: a token is a logic guard (psx_io::periph); see above.
    let mut reader = SectorReader::with_cd(unsafe { Cd::steal() });
    let mut sector = [0u32; SECTOR_WORDS];
    if !reader.prepare() {
        return files;
    }
    let Some(root) =
        read_sector(&mut reader, &mut sector, iso::PVD_LBA).and_then(iso::root_directory)
    else {
        return files;
    };
    let Some(directory) = read_sector(&mut reader, &mut sector, root.0) else {
        return files;
    };
    let mut n = 0;
    while n < FILES {
        let name = [
            b'M',
            b'U',
            b'S',
            b'I',
            b'C',
            b'1' + n as u8,
            b'.',
            b'X',
            b'A',
        ];
        if let Ok(name) = core::str::from_utf8(&name) {
            files[n] = iso::find_in_directory(directory, name).map(|(lba, size)| {
                File::from_directory_entry(lba, size, n as u8 + 1, DriveSpeed::Single)
            });
        }
        n += 1;
    }
    files
}

/// One data sector as bytes.
fn read_sector<'s>(
    reader: &mut SectorReader,
    sector: &'s mut [u32; SECTOR_WORDS],
    lba: u32,
) -> Option<&'s [u8]> {
    if !reader.start_read(lba) {
        reader.stop();
        return None;
    }
    let ok = reader.read_sector(sector);
    reader.stop();
    // SAFETY: a `[u32; N]` viewed as its own bytes; alignment only shrinks.
    ok.then(|| unsafe {
        core::slice::from_raw_parts(sector.as_ptr().cast::<u8>(), SECTOR_WORDS * 4)
    })
}

/// The title screen is up: the menu song comes after Java's 1 to 30 seconds.
#[inline(never)]
pub fn enter_title() {
    let now = interrupts::vblank_count();
    let x = m();
    if x.phase == Phase::Off {
        return;
    }
    if x.phase == Phase::Playing {
        x.player.stop();
    }
    x.rng = LcgRng::new(0x4d55_5349 ^ now);
    let d = plan::delay_vblanks(plan::MENU_DELAY, x.rng.next_mixed());
    wait(now, d);
    publish();
}

/// A world is loading: the menu song stops, and the first game song comes
/// after Java's starting delay. The seed is the time since boot, which the
/// player's own pace through the menus makes different every session.
#[inline(never)]
pub fn enter_world() {
    let now = interrupts::vblank_count();
    let x = m();
    if x.phase == Phase::Off {
        return;
    }
    if x.phase == Phase::Playing {
        x.player.stop();
    }
    x.rng = LcgRng::new(0x4d55_5349 ^ now.rotate_left(7));
    wait(now, plan::START_DELAY * 3);
    publish();
}

fn wait(now: u32, vblanks: u32) {
    let x = m();
    x.phase = Phase::Waiting;
    x.until = now.wrapping_add(vblanks);
    x.delay = vblanks;
}

/// A song is over (or was stopped): the next one comes after the game delay.
fn stop_song(now: u32) {
    let x = m();
    x.player.stop();
    let d = game_delay(&mut x.rng);
    wait(now, d);
}

fn game_delay(rng: &mut LcgRng) -> u32 {
    if LAB {
        5 * 60 + rng.below(5 * 60)
    } else {
        plan::delay_vblanks(plan::GAME_DELAY, rng.next_mixed())
    }
}

/// How long a piece plays: its length in the file.
fn length_ms(track: u8) -> u32 {
    if LAB {
        LAB_LENGTH_MS
    } else {
        TRACKS[track as usize].length_ms
    }
}

/// True when the next song is due and the running screen should `start` it.
#[inline]
pub fn pick_due() -> bool {
    let x = m();
    x.phase == Phase::Waiting && (interrupts::vblank_count().wrapping_sub(x.until) as i32) >= 0
}

/// Pick a piece (for the title screen or a world) and start it.
#[inline(never)]
pub fn start(title: bool) {
    let now = interrupts::vblank_count();
    let x = m();
    let track = plan::pick(x.rng.next_mixed());
    x.picked = if title { 1 } else { 2 };
    if x.volume == 0 {
        // Java asks the sound engine to play it, which plays nothing at zero
        // volume, and the song counts as over: the delay starts again.
        x.track = NO_TRACK;
        let d = game_delay(&mut x.rng);
        wait(now, d);
        publish();
        return;
    }
    let song = &TRACKS[track];
    let played = match x.files[song.file as usize - 1] {
        Some(file) => x.player.play(file.song(song.channel), false).is_ok(),
        None => false,
    };
    match played {
        true => {
            x.phase = Phase::Playing;
            x.track = track as u8;
            x.started = now;
            x.next_poll = now.wrapping_add(POLL_VBLANKS);
            x.plays += 1;
        }
        false => {
            x.failures += 1;
            x.track = NO_TRACK;
            wait(now, RETRY_VBLANKS);
        }
    }
    publish();
}

/// Once a frame, from the loop that is running (title or world).
#[inline]
pub fn tick() {
    let x = m();
    if x.phase == Phase::Off || x.phase == Phase::Idle {
        return;
    }
    let now = interrupts::vblank_count();
    if x.phase == Phase::Playing && (now.wrapping_sub(x.next_poll) as i32) >= 0 {
        poll(now);
    } else if now.wrapping_sub(x.published) >= RECEIPT_VBLANKS {
        publish();
    }
}

#[inline(never)]
fn poll(now: u32) {
    let x = m();
    let event = x.player.poll();
    let length = length_ms(x.track);
    let elapsed = x.player.elapsed_millis();
    if event == Event::Finished || elapsed >= length {
        stop_song(now);
    } else {
        let step = if length - elapsed <= END_WINDOW_MS {
            POLL_END_VBLANKS
        } else {
            POLL_VBLANKS
        };
        x.next_poll = now.wrapping_add(step);
    }
    publish();
}

fn publish() {
    let x = m();
    let now = interrupts::vblank_count();
    x.published = now;
    let left = if x.phase == Phase::Waiting {
        (x.until.wrapping_sub(now) as i32).max(0) as u32
    } else {
        0
    };
    let state = [
        u32::from_le_bytes(*b"VMUS"),
        1,
        64,
        match x.phase {
            Phase::Off => 0,
            Phase::Waiting => 1,
            Phase::Playing => 2,
            Phase::Idle => 3,
        },
        x.track as u32,
        x.picked as u32,
        x.volume as u32,
        left,
        x.plays,
        x.player.is_streaming() as u32,
        x.player.elapsed_millis(),
        x.failures,
        x.delay,
        plan::mixer_level(x.volume) as u32,
        0,
        0,
    ];
    // SAFETY: a plain store of the receipt; nothing else writes it.
    unsafe { core::ptr::write_volatile(core::ptr::addr_of_mut!(VOXIDE_MUSIC_STATE), state) };
}

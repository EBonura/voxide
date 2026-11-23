# VoXide on the scene core

How VoXide maps onto the SDK's scene system: a resident core in a fixed window at the top of RAM, a
handoff block above it, and `SCENES.TBL` at member LBA 22. The row format, kinds and link rules are the
core lane's CORE-ABI.md (campaign `scene-system-1123`, day 0, 11-23); everything here is provisional
with it. CORE-ABI is written for the 10,240 B cap (window 0x801FD700); this branch links for the
provisional 8,192 B window, **0x801FDF00..0x801FFF00**, so every SP and bound below uses 0x801FDF00.
Line numbers are at commit b7b7bd8 of this repo (the scene-core commits keep every line where it was);
SDK lines are at the pin 6bfe478ff0e0 (`components.lock.json`). Sizes are from the b7b7bd8 ship link
map unless they say otherwise.

Rule this follows (Manny, 11-23): scene boundaries only where the player already waits today, and every
screen reachable in play stays resident in the playing scene. VoXide is one scene. The title renders the
live generated world with the gameplay renderer and PLAY GAME enters that same ring (`main.rs:4402-4418`,
4570-4607), and after boot the game never reads the disc again: the XA player holds the drive for good
(`music.rs:13-17`).

## What the player sees

| What the player sees | In the program | Resident in |
|---|---|---|
| Boot splash "Built with PSoXide", 150 frames, skip after 8 | `show_intro` (`main.rs:5179-5192`), called at `main.rs:3089` after the settings read and the SFX bank | scene 0 today; the core's boot scene under `scene-core` (step 3) |
| Loading bar ("Generating"), dirt backdrop | `finish_world_gen` in `main_menu` (`main.rs:4413-4421`, 4644-4675): world generation, no disc read | scene 0 |
| Title: live world vista, PLAY GAME / LOAD WORLD / NEW WORLD / CONTROLS / SETTINGS / CREDITS | `main_menu` (`main.rs:4410-4643`) | scene 0 |
| Title cards: controls, settings tree, credits | flags in the same loop | scene 0 |
| NEW WORLD, LOAD WORLD | `prepare_new_world`, `load_game` (`main.rs:4520-4545`, 5631-5690): generation and a card read, no disc read | scene 0 |
| Gameplay | loop at `main.rs:3195` | scene 0 |
| Crafting, inventory, chest, furnace, death screen | `menu` 1..3, `MENU_INV`, `MENU_DEAD` (`main.rs:3249-3470`) | scene 0 |
| Pause (options tree), cheats, controls card | `MENU_OPTIONS`, `MENU_CHEAT`, `MENU_CONTROLS` (`main.rs:3480`) | scene 0 |
| Save from pause (card, over several frames) | `save::begin`, `save_step` (`main.rs:3240-3242`, 5598-5612) | scene 0 |
| Dimension travel (Inferno, Void, Overworld) | `world::set_dimension` behind the loading bar (`world.rs:1642-1665`): generation, no disc read | scene 0 |
| SAVE AND QUIT TO TITLE | `quit_to_title` (`main.rs:4297-4316`) | scene 0 |
| Exit | none: `main` never returns | |

## Scene table rows

| id | kind | What it is | Fields that matter | return scene |
|---|---|---|---|---|
| 0 | 0 Program (boot scene: header 0x10 = 0) | `PSX.EXE`, the whole game | `t_addr` 0x80010000, `t_size` 790,528 (0xC1000), file 387 sectors with the header, entry PC `__core_boot` once linked with psoxide-core.ld (`_start` today), SP 0x801FDF00, name `VOXIDE` | 0xFFFF (none: Manny 11-23) |
| 1 | 2 Data **(provisional kind)** | `WORLD.PAK`, member LBA 1024, 48 sectors, one chunk (id 3000, the SFX bank, 47 sectors) | LBA and sectors | n/a |
| 2 | 2 Data **(provisional)** | `MUSIC1.XA`, LBA 1072, 12,392 sectors, four songs on channels 0-3 | LBA and sectors, so `find_song_files` could skip the PVD and root read if the core ever hands rows to a program (CORE-ABI has no call for that today) | n/a |

Row 2 is optional; without it the root lookup stays. On the demo disc VoXide would be one Program row in
the host's table with its own `lba_offset`; it already chain-loads there through `disc_base` today.

## Shared code versus per scene

One row, so everything is shared: one static image, no heap, every table a static (`LIMITS.md:3`).
World chunk store, face arena and scratch (528,528 B of `.bss`) and the render arenas (482,080 B) serve
the title, play, dimension travel and load alike.

## The relink (step 1, done)

`game/build.rs` writes `RAM_SIZE_OVERRIDE = 0x1FE000;` into `$OUT_DIR/ram-size.ld` and passes it as a
`-T` file before psoxide.ld (psoxide.ld:16-21); the map records it (map line 2). One constant in
`game/build.rs`, marked provisional. `make compile` (ship), `make gate-disc` and every lab build link
through build.rs. VoXide keeps the default 32 KiB stack reserve.

Measured on the ship build (`make compile` then `make pack`, the two commands of `[[build]] ship`)
against the 6d06c0d ship build kept by the 1122 lane (`voxide-repin-1122/keep/6d06c0dac6f3/`, map sha256
97fd0c8b... = the `[maps] ship` baseline):

| | before (6d06c0d) | after (b7b7bd8) |
|---|---|---|
| header SP (`STACK_INIT`) | 0x801FFF00 | 0x801FDF00 |
| `__heap_end` (`STACK_INIT - STACK_RESERVE`) | 0x801F7F00 | 0x801F5F00 |
| `__bss_end`, highest byte the image owns | 0x801E71A4 | 0x801E71A4 |
| .text / .data / .bss | 692,708 / 96,872 / 1,140,036 | the same |
| free RAM (`__heap_end - __bss_end`, the size stage's `free`) | 68,956 (baseline and keep PASS json `ship/free`) | 60,764 (-8,192) |
| stack reserve | 0x801F7F00..0x801FFF00 | 0x801F5F00..0x801FDF00 |
| exe bytes that differ | | 32 of 792,576: header SP (1) and 31 of the 32-byte gate receipt digest at 0x408. No code byte: VoXide has no heap, so nothing in the code refers to `STACK_INIT` or `__heap_end` |
| PGO apply | | profile binds 100.0% of its samples, 0 of 721 functions missing, 2 changed (the same report on every build of this commit) |

Nothing is linked at or above 0x801FDF00 (no map line with a VMA at or above 0x801F0000). The window
fits with 60,764 B still free.

**The stack still starts above the window when the BIOS boots the disc.** psx-rt's `_start` does not
set `$sp`; it trusts the loader (`psx-rt/src/lib.rs:164-168`), and the BIOS boot path takes `STACK =
801FFFF0` from the `SYSTEM.CNF` the SDK writes for every disc (`psx-iso` `iso9660.rs:692`). Measured on
Quake's ship-boot run on the same frontend (`entry_sp = 0x801ffff0`, quake-psx SCENES.md); not measured
for VoXide. So until `__core_boot` sets `$sp = 0x801FDF00` (CORE-ABI 4.3 step 1), VoXide's stack runs
from 0x801FFFF0 through the window, and the relink only moves the link-time budget.

## Conflicts with CORE-ABI.md

- **Window size.** CORE-ABI says 0x801FD700 (10,240 B); this branch uses the brief's 8,192 B. 10,240 B
  would leave 58,716 B free.
- **Disc layout.** mkisopsx writes `PSX.EXE` at LBA 22; the table wants 22 and the exe at 23. SDK
  branch lane/disc-scenes-1123 adds `mkisopsx --scenes`; the 6bfe pin does not have it.
- **The SFX bank streams sector by sector into SPU RAM inside one read session** (`sfx.rs:91-129`: pack
  header via `find_entry`, then 47 sectors, an SPU upload after each). `psx_core_read` ends every call
  with a proven Pause and writes to one linear buffer, and VoXide has no free 94,272 B buffer outside
  the idle render arenas. So the bank must use the stream-client calls (`psx_core_client_attach`,
  `psx_core_stream_open`, 47 x `psx_core_stream_sector` into the 2 KB scratch, `psx_core_pause`,
  `psx_core_client_detach`), or read into a borrowed render arena with one `psx_core_read`. Per-sector
  `psx_core_read` calls would add a Pause and a seek per sector.
- **Boot order and the settings record.** Today the card's settings are read before the splash so the
  splash shows the saved brightness (`main.rs:3086`, 5187). With the splash in the core's boot scene the
  brightness has to come from the handoff block's settings record (0x30, provisional layout) or the
  splash shows the default.
- **The splash is checked at a fixed tick.** `music::init` runs after the splash on purpose
  (`main.rs:3090-3091`); a boot scene in front changes every boot-relative tick (see the tests below).
- **No CD IRQ handler, polled only.** True today (`psx-io` reader and the polled XA `Player`).
- **Card.** The card stays VoXide's (settings and world saves, `save.rs`); nothing crosses scenes.

## What the core replaces (step 3)

Measured on the b7b7bd8 ship map (sizes equal the 6d06c0d map: no code moved):

| What | Symbols (file:line) | Bytes |
|---|---|---|
| Boot splash | `psx_splash::show` 2,880, `logo::expand` 664, `draw_line` closure 260, `voxide::show_intro` 100 + closure 44 (text); `logo::CODED` 2,746, `PALETTE` 32 (data). `show_intro` at `main.rs:5179-5192`, called at `main.rs:3089`. `FontAtlas::upload` stays: VoXide uploads its own font with it (`main.rs:3070`) | 3,948 text + 2,778 data = **6,726** on the map; the `scene-core` link measures **7,132** less (text 692,708 to 688,504, data 96,872 to 93,944) |
| SDK polled sector reader | `SectorReader::prepare` 3,084, `read_sector` 1,944, `send_command_waiting` 1,180, `start_read` 268, `stop` 232; `ReaderPause::wait_seek_end` 1,052, `status` 1,040 | **8,800** text |
| psx-io command layer | `try_command_until_complete_inner` 552, `Cd::command_within` 484, `read_response_fifo` 300, `begin_polled_command` 160, `finish_failed_polled_command` 112, `drain_response_limited` 92, `ack_all_and_reset_parameters` 48, `lba_to_bcd_msf` 324 | **2,072** text (shared with the XA player's `Drive` impl, which reaches the drive through `psx_core_cd` under the core) |
| | **Total that leaves once the core owns the drive** | **about 10,870 B plus the splash** |

Stays in VoXide: the XA player (`psx_io::cd::xa::Engine` and its `Drive` impl, 2,552 B text, driven from
VoXide's loop, PLAN 3.1), the music scheduler (`music.rs`, `music_plan.rs`), `sfx::init`'s upload loop,
`music::read_sector` (300) and the root lookup in `music::init` (784, `find_in_root` inlined), whose
reads become `psx_core_read` calls.

`Cd::steal()` sites, all three replaced: `sfx.rs:93` (bank reader), `music.rs:182` (root lookup reader),
`music.rs:94` (`Player::new`, which keeps its token but reaches the drive through `psx_core_cd`).

**Done now, behind the off-by-default `scene-core` feature (commits c884719, 0641e4e):** `show_intro` and
its call are compiled out. A `make compile FEATURES=scene-core` link has no `psx_splash` symbol
(text 688,504, data 93,944, bss unchanged). The default build of the feature commit equals b7b7bd8's exe
byte for byte (`cmp`; the first try moved lines in `main.rs` and with them 40 bytes of panic-location
line numbers, so the attributes sit on the existing lines). The feature is not usable on a disc yet:
nothing else shows the splash, and `Screen::new` takes the display as `gpu::init` left it.

**TODO when `psx-core` exists:**

1. `game/build.rs`: `-T psoxide-core.ld` after the ram-size file; `game/Cargo.toml`: `psx-core`
   (feature `link`) under `scene-core`.
2. `sfx.rs:91-129`: the bank through the stream-client calls (conflicts above).
3. `music.rs:177-200`: the PVD and root reads through `psx_core_read`; `Player` through `psx_core_cd`.
4. Splash: `scene-core` on; brightness from the handoff block's settings record.
5. No exit row (CORE-ABI return scene 0xFFFF).

## Load speed (Manny 11-23: every load the same speed or faster)

No code or data that is resident today becomes a disc read. After boot VoXide reads nothing; every
"load" the player sees (world generation, dimension travel, LOAD WORLD) is computation or a card read.

| Transition | Disc read today | Under the core | Measured today | Slower? What keeps it equal |
|---|---|---|---|---|
| Boot to first frame (splash) | BIOS loads `PSX.EXE`, 386 payload sectors; before the splash, the pack header (1) and the SFX bank (47) in one session | the same plus the core image stored after `.data`, about 4 sectors at 8,192 B **(estimate)**, and `__core_boot`'s copy and word FNV over 8 KB | title reached at frame 421 on PCSX-ReARMed, 1219 on PCSX-Redux (keep PASS json, `cross`); smoke journey 469 ticks. Boot to splash alone: not measured | **yes, slightly (estimate):** about 4 sectors, about 27 ms at double speed. With `scene-core` on and a separate boot-scene program showing the splash, add that program's whole load. Keeping the splash in VoXide while it is the disc's first program keeps this to the 4 sectors |
| Music lookup after the splash | PVD (LBA 16) and the root directory | the same sectors via `psx_core_read` | not measured | no |
| SFX bank | one session, 47 sectors, an SPU upload per sector | stream-client session | not measured | only if it goes through per-sector `psx_core_read` (a Pause and seek per sector); the stream client keeps it one session |
| Title, NEW WORLD, LOAD WORLD, dimension travel, quit to title | none (generation; LOAD WORLD reads the card) | none | not measured per transition | no |
| Every in-play screen | none | none | | no |

## Tests, bars and goldens that move

| Check | Moves? |
|---|---|
| `[[size]] ship` (baseline free 68,956, bars 0) | yes, fails: free 60,764 (-8,192); text, data, bss unchanged. Re-bless with the window as the reason |
| `[maps] ship` (baseline 97fd0c8b...) | yes: 9aa3602f... |
| `PGO_RAM_FLOOR ?= 92876` (`Makefile:263`) | already above today's free RAM (68,956), now 60,764: `make pgo-choose` refuses every variant either way. Stale setting, separate fix |
| speed `train`, `unseen` (tapes from boot, window polls 252..1200) | not expected from the relink: no code address moves. Not run. A core or boot scene in front shifts the boot-relative polls; both tapes would need re-recording |
| PGO profile `pgo/voxide.prof` and `game/hot-text.order` | the relink binds the same (100.0%, 2 changed, as before). Any core call ABI moves code and needs the speed gate |
| journeys and goldens (splash golden, strict regions on the sheen pixel at (223,150), `tests/journey.toml:28-55`, `save-repin.toml`) | not expected from the relink. Not run. They move when the splash moves to a boot scene |
| `[cross]` smoke (title within 3,000 frames) | not expected; a boot scene adds its time to every timeout (421 and 1,219 frames today) |
| card journeys (`reboot` action) | not from the relink; "reboot" boots the core first once there is one |
| `[clean]` (bin, cue, exe) | the exe and bin change; the feature commit's default exe equals b7b7bd8's |
| install facts | yes: exe sha256 1be56be2..., bin e1ed78a7...; `VoXide.provenance.json`, `install-stamp.json` and the library map name move |

# VoXide tuning audit

VoXide matches Minecraft Java Edition values. Each fix below cites minecraft.wiki
(`W:` is `https://minecraft.wiki/w/`). Clocks: everything in `update_survival` and
`mob::update` runs on the 60 Hz sim tick, and `units::java_ticks(n)` converts Java
game ticks (20 a second) to it, so a 20 TPS constant cannot run at 60 Hz by accident.

## 2026-10-19: the bow, bed, door, redstone and piston values

Rules and numbers of the shaped blocks, from the pages named (W: as above).
A value marked "to confirm" was written from memory of the game and has not
been checked against the page; one that is not on the wiki is named as ours.

| Item | Code | Java value | Result |
|---|---|---|---|
| Bow draw | `bow::power_q12`: f = (t^2 + 2t) / 3, t = held / 20 game ticks, capped at 1 | W:Bow, same curve | matches |
| Shortest shot | no arrow and no use below f = 0.1 (9 sim ticks, 3 game ticks) | 0.1 (W:Bow) | matches |
| Arrow speed | 3 blocks a game tick x f; gravity 0.05, drag 0.99 (existing) | 3.0 at full draw (W:Arrow) | matches |
| Arrow damage | ceil(2 x speed in blocks a game tick), full draw 6; a full draw is critical, up to half again | W:Arrow | matches |
| Drawing slows the player | movement x 1/5, sprint stops | W:Walking, from memory of the game: using an item 0.2 (to confirm) | matches |
| Drawn string | 40 units back over the 1 s draw; rings out in 14 sim ticks | our own: the animation, not a value | n/a |
| Bow shoot sound | pitch 0.85 to 1.10 by pull (Java: 1 / (random x 0.4 + 1.2) + f x 0.5) | W:Sounds, from memory (to confirm) | deterministic, no random |
| Sleep | allowed when sky light is 11 or less or in a thunderstorm; refused with a hostile within 8 blocks sideways, 5 up or down | W:Bed | matches |
| Respawn point | set beside the bed on any use, daytime too | W:Bed, from memory of the game: any use sets it (to confirm) | matches |
| Sleep length | fade out 1 s, dark 0.7 s, fade in 1 s | ours; Java's sleep is a longer wait (to confirm) | ours |
| Door | 2 blocks, 3/16 thick panel on the near edge, opens to the hinge edge | W:Door | matches (no hinge side: always the placer's left) |
| Door needs | a solid block under it; breaking one half takes both | W:Door | matches |
| Torch, lever, ladder | hang on a solid face or stand on one, drop when it goes | W:Torch, W:Lever, W:Ladder | matches |
| Dust power | 15 at a source, 1 less a block; steps up and down | W:Redstone_Dust | matches |
| Redstone tick | torches re-read every 2 game ticks (0.1 s) | W:Redstone_Torch | matches |
| Piston push limit | 12 blocks | W:Piston | matches |
| Piston face | toward the player who placed it | W:Piston | matches |
| Not in: | repeaters, comparators, buttons, pressure plates, sticky pistons, torch burnout, quasi-connectivity, door hinge sides, iron doors, trapdoors | | open |

## 2026-10-15: hunger and mob attack speed (GitHub issue 1)

The issue was filed on 2026-08-30 against 0.1.11, before the Java parity pass
(hunger by exhaustion in `c3414e1`, attack cooldown in `a3bedcc`, both 2026-09-24).
The rows below check the current code.

| Item | Code | Java value | Result |
|---|---|---|---|
| Exhaustion threshold | 4.0 costs 1 saturation, else 1 hunger (`update_survival`) | 4.0, same order (W:Hunger) | matches |
| Sprinting | 0.1 per block (`EXH_SPRINT_PER_UNIT`) | 0.1 per metre (W:Hunger) | matches |
| Swimming | 0.01 per block | 0.01 per metre (W:Hunger) | matches |
| Jump, sprint jump | 0.05, 0.2 | 0.05, 0.2 (W:Hunger) | matches |
| Attack landed, damage taken, block broken | 0.1, 0.1, 0.005 | 0.1, 0.1, 0.005 (W:Hunger) | matches |
| Walking | 0 | not listed (W:Hunger) | matches |
| Regeneration, hunger 18+ | 1 hp per 80 game ticks (240 sim ticks), 6.0 exhaustion | 1 hp per 80 ticks, 6.0 (W:Hunger) | matches |
| Saturation boost, hunger 20 | every 10 game ticks, min(saturation, 6) / 6 hp | every 10 ticks, 1.5 saturation per heal (W:Hunger) | matches |
| Starvation | 1 hp per 80 game ticks, stops at 1 hp | 1 hp per 80 ticks, stops at 1 hp on Normal (W:Hunger) | matches |
| Skeleton fire interval | 3 s | once every 3 s on Easy and Normal (W:Skeleton) | matches |
| Creeper (sapper) fuse | 30 game ticks | 30 ticks (1.5 s) | matches |
| Hostile melee damage | zombie 3, spider 2, enderman 7 | Normal values (W:Zombie, W:Spider, W:Enderman) | matches |
| Player invulnerability after a hit | 10 game ticks | 10 game ticks, 0.5 s (W:Damage) | matches |
| Hostile melee swing interval | a touching mob hit every time the player's invulnerability ended, so every 0.5 s | one swing per 20 game ticks (1 s) | **mismatch, fixed** |

### Fixed: hostile melee interval

`mob::contact_damage` returned the damage of every mob touching the player on every
sim tick, and the player's 10-tick invulnerability was the only brake. A zombie in
reach therefore hit every 0.5 s (6 hp a second). In Java the invulnerability is a
second, separate limit: the melee goal strikes once per 20 game ticks. Each mob now
carries an `atk` countdown, set to 20 game ticks (60 sim ticks) when it strikes, so
one mob in reach deals at most 3 hp a second. Several mobs still strike
independently, and the strongest one inside the player's invulnerability window
still decides the hit, as before.

The 20-tick interval is Java's `MeleeAttackGoal`, the figure `WOLF_BITE_TICKS`
already uses. The wiki does not state it on W:Zombie or W:Damage, so this one value
is cited from the game source rather than the wiki. If a playtest shows it feels
wrong, `MELEE_TICKS` in `game/src/mob.rs` is the only constant to change.

### Not changed

Hunger drain already matches Java. Before 0.2.0 the clock was `FOOD_DRAIN`, one
hunger point every 10 s (a tick-rate bug), which is the likely source of "you die very
quickly from hunger". Starvation cannot kill on Normal (it stops at 1 hp). Music is
checked below.

## 2026-10-15: how fast an explosion shows (TNT, creepers)

Manny: "the TNT does explode, but the world is taking a lot to update". The
rules did not change, only when the work happens. `W:` is
`https://minecraft.wiki/w/`.

| Rule | VoXide | Source | Status |
|---|---|---|---|
| Block damage | 1,352 rays from the centre, each power x (0.7 to 1.3), 0.3 block steps, 0.225 lost a step and (resistance + 0.3) x 0.3 in a block | W:Explosion | matches, unchanged |
| TNT power, fuse | power 4, 80 game tick fuse | W:TNT | matches, unchanged |
| Chained TNT | fuse of 10 to 29 game ticks (0.5 to 1.5 s) | W:TNT | matches, unchanged |
| Drops | each broken block keeps its drop with chance 1 / power | W:Explosion | matches, unchanged |
| When the crater exists | Java removes the blocks in the tick the TNT goes off and the client remeshes the next frame | W:Explosion | deviation, below |

The faster ray is the same ray: a host test (`blast.rs`, `trace_marks_what_it_always_marked`)
marks the same cells as the old code for every ray direction, strengths from
nothing to a charged sapper's, any start inside the centre cell and a world of
mixed resistance. The 1,352 directions are a table built when the game is
compiled (`RAY_DIRS`), so a blast casts the same rays with the same
(seed, grid point) rolls.

Where the time went (cycle profile and the tnt-lab words, PGO build, one blast
of 79 cells on the flat arena at 30 fps): the idle time a frame could spare
was shared out at about a third of a vblank a frame. A blast is 3.1M cycles of
rays (a third of it the per-ray square root and divides for the direction),
0.3M of snapshot and 0.9M of breaking, then about a million cycles of remesh
for each chunk the crater touches (one to four), and the mesh got three slices
a frame. Each phase of the job also ended its frame, and every blast of a
chain restarted the mesh of whatever chunk was in flight, so a chain remeshed
a crater it had not yet finished.

Ticks after the first TNT's fuse ends (60 a second; PGO build, headless;
`tnt-lab`, `tnt-meadow`; 4 TNT is a 2 x 2 flat, 16 a 4 x 2 x 2, only the first lit):

| | TNT | blocks gone | crater meshed in view | chain done |
|---|---|---|---|---|
| flat arena, before | 1 | 32 to 37 | 129 | 37 |
| flat arena, after | 1 | 9 | 17 | 9 |
| flat arena, before | 4 | 34 | 397 | 309 |
| flat arena, after | 4 | 8 | 185 | 143 |
| flat arena, before | 16 | 36 | not in 960 | 943 |
| flat arena, after | 16 | 8 | 482 | 475 |
| spawn meadow, before | 1 | 53 to 77 | 168 | 77 |
| spawn meadow, after | 1 | 5 to 9 | 34 | 9 |
| spawn meadow, before | 4 | 58 | 605 | 509 |
| spawn meadow, after | 4 | 5 | 262 | 214 |
| spawn meadow, before | 16 | 97 | not in 960 | 905 |
| spawn meadow, after | 16 | 6 | 790 | 751 |

("Crater meshed in view" is the last tick a chunk the player faces waited for
its new mesh. A chain's first column is the first blast; its last column is the
last blast's blocks. "Not in 960": the 1,500 tick phase ended before the last
chunk was meshed.) The frame strips (a single TNT and a 4-chain on the meadow, before above
after, at matched ticks) were viewed for this audit and are not committed.

Deviations, deliberate:

- The crater is not there the tick the TNT goes off. The R3000 spends about 3M
  cycles on one blast's rays and block work and 1M a crater chunk on the mesh
  (a frame is 1.1M), so the work is spread: the blocks are gone 5 to 9 ticks
  later, the drops with them, and the crater is on
  screen 17 to 34 ticks (0.3 to 0.6 s) after the fuse ends for one TNT, where it
  was 129 to 168. Java is instant.
- Craters in view are meshed first, in a burst; chunks of the crater behind the
  player or off to the side mesh at the old pace (three slices a frame), so
  turning round within a second of a blast can show a stale mesh.
- A frame spends the idle time it would have had without the blast (13/16 of
  it, against the period the scene kept before the blast), so a chain does not
  change the vblank rate in a scene with room, and in a scene with none it
  takes `URGENT_MIN`, 160,000 clocks a frame, which makes a 2 vblank scene 3
  vblanks for the length of the chain. A chain of four on the meadow ran at 3
  vblanks for about 3 s.
- The first six frames after a quiet spell (90 frames) may take two vblanks
  more: one TNT cost 4 to 5 vblank frames four times on the arena and six
  4 vblank frames then seven 3 vblank frames on the meadow, about 0.15 to 0.3
  s of lost time, against a crater 2 to 3 s late before. This dip is the price
  of the crater arriving 5 to 34 ticks after the fuse instead of 53 to 168.
- Flames wait while a blast near the player is urgent (they share the frame's
  allowance); they catch up after.
- Chained TNT still explode in the order the queue admits them, one blast job
  at a time, so a chain of 16 took 8 to 12 s to finish where Java takes 1.5 s.
  Work per blast was 3M cycles; the 0.4.1 section below makes the blast cheaper
  and lets a long chain borrow more of the frame.
- Blasts farther than the render range (`near` false) keep the old idle share.

Measured outside explosions (unseen walking tape, the RC disc against this
branch, same tool chain as the release notes): work per frame 930,955 and
931,081 cycles (+0.014 percent), train tape 1,133,205 and 1,133,331
(+0.011 percent), the same frames on screen (display hashes identical) and the
same vblank counts.

New gates: `tests/journey.toml` and `tests/save-repin.toml` wait for the fuse,
then require the crater's blocks gone within 30 ticks (the old build takes 37
to 77 and fails it; this one takes 5 to 9 in the lab).

## 2026-10-15: long TNT chains (0.4.1)

Manny: a 16 TNT chain takes 8 to 12 s to settle where Java takes about 1.5 s.
The rules did not change (power 4, a fuse of 0.5 to 1.5 s for TNT a blast
lights, W:TNT and W:Explosion as in the table above); only when and at what
cost the blasts run. Measured with the `tnt-lab` and `tnt-meadow` discs (a
4 x 2 x 2 block of 16, only the first lit), the PGO build, headless, 60 ticks a
second, ticks counted from the first TNT going off.

Where the ticks went in the 16 chain on the flat arena (cycle counts over the
window from the first blast to the last crater meshed, 330 frames of 1.2M):

| | before | after |
|---|---|---|
| blast job, ray stage | 27.9M (1.74M a blast) | 16 to 20M |
| blast job, snapshot of the blocks around the centre | 4.3M | 5.3M (it now also finds which heights hold a block) |
| blast job, breaking (column rewrites, drops, edit log) | 15.2M, 7.0M of it `record_edit` scanning the edit log | 8.5 to 10M |
| the TNT tick (fall, fuse, the blast's hit on mobs, player, drops, smoke) | 4.8M | 5.0M |
| remesh of the craters | 25.7M | 22M (two bursts of four chunks: after the first blast, which costs nothing because the fuses are still burning, and after the last) |
| lighting (`flood_light`) | 1.5M of 249M profiled cycles | 1.4M of 178M, not changed |
| admission (the queue of eight blasts) | no wait: a primed block is admitted on the first free unit of a frame | same |

The admission queue was never the limit: the blast job is, one at a time, and a
frame spared it about 0.25M of its 1.14M. 83M of work at 0.25M a frame is 330
frames, the 7 s. Three changes, none of which alters a cell a blast breaks:

- A frame may run longer while blasts wait (`slack.rs`, `STRETCH_AT`,
  `STRETCH_VBLANKS`). A backlog of 2 (queued and running blasts, primed blocks
  whose fuse is out, and, for a run of four or more blasts, the craters waiting
  for their mesh) lets the frame take one vblank more than the scene's own
  period, a backlog of 5 two more. The backlog is the throttle: a chain the
  quiet spare time keeps up with never stretches, and a single TNT still gets
  its existing boost.
- A ray whose whole height band holds only air marks nothing and is not walked
  (`blast::band_is_air`; the snapshot records which heights hold a block). On
  open ground that is every upward ray, half of them and the longest. The
  snapshot also reads only the columns the blast's power can reach (19 x 19 for
  power 4, not 23 x 23). A host test (`air_band_rays_mark_nothing`) casts all
  1,352 rays both ways over scattered, floored and ceilinged snapshots at every
  power and asserts the same set of broken cells.
- `trace` is a function of its own (its state stays in registers: a quarter
  off the ray stage) and `record_edit` finds a position through a hash index
  (`EDIT_HASH`, 4 KB) instead of scanning up to 1,024 entries a hole.

Ticks after the lab lights the first TNT (60 a second; the first goes off at
tick 240), before and after (flat arena; spawn meadow, whose scene runs at 3
vblanks to begin with). "In view" is the last tick a chunk the player faces
waited for its new mesh.

| | chain | all blasts done | craters in view meshed | settled |
|---|---|---|---|---|
| flat | 1 | 249 to 247 | 268 to 255 | 279 to 255 |
| flat | 4 | 380 to 328 | 425 to 352 | 425 to 360 |
| flat | 16 | 679 to 421 | 694 to 450 | 705 to 450 |
| meadow | 1 | 254 to 253 | 279 to 278 | 294 to 293 |
| meadow | 4 | 479 to 370 | 536 to 403 | 551 to 403 |
| meadow | 16 | 1,015 to 439 | 1,105 to 473 | 1,105 to 473 |

(Each cell is before to after.) A chain of 16 is done 3.0 s after its first
blast on the flat arena and 3.3 s on the meadow, where it took 7.3 s and 12.9
s. Java's is done at about 1.5 s, the longest fuse of the blocks the first
blast lights (87 ticks here); what is left is the blast job, about 2M clocks a
blast against a frame that has 1.1M to give, so the tail is work, not waiting.

Frames, 16 chain on the flat arena (from the first blast on, the lab's route log): before, 35 of 505 frames longer than two vblanks,
41 extra vblanks, worst streak 10 frames, longest frame 4 vblanks. After, 54 of
478 frames, 93 extra vblanks, worst streak 42 frames (about 2.7 s at 15 to 20
fps), longest frame 5 vblanks (3 frames). The 4 chain: 20 of 337 before,
25 of 334 after; the single TNT: 9 of 293 before, 5 of 295 after. Outside the
explosions the lab's frames stay at two vblanks (6,519 of 6,552 before, 6,301
of 6,701 after; the 366 one-vblank frames after are the title and loading frames of the
lab script, which now draws a menu that is cheaper to run).

Not changed, as before: the first six frames after a quiet spell may run two
vblanks long; blasts beyond the render range keep the idle share; the crater
is not there the tick the TNT goes off.

Outside explosions (same tool chain and tapes as the release notes, the
pre-0.4.1 integration BIN `b831110` against the final 0.4.1 BIN): train tape
1,122,775 and 1,122,356 work cycles a frame (display hashes identical), Manny's
tape of 2026-10-15 1,122,832 and 1,123,470 (+0.06 percent; frames at 2, 3 and 5
vblanks 1,721, 12, 1 and 1,718, 15, 1), unseen tape 906,079 and 924,319 (+2.0
percent). The unseen tape's display hash differs: the title now looks at the
card, which moves the route by a few ticks, and that tape walks the sticks and
meets different ground. The train tape, whose route does not diverge, shows no
cost. Instruction-cache stall cycles are 4 to 6 percent higher on all three
tapes (code layout; the work totals are level).

## 2026-10-15: 20 block view distance (0.4.1)

Manny: ship 20 blocks. `DRAW_BLOCKS` in main.rs is 20 (it is the one knob: far
plane, side cut, fog ramp and the meshed ring, `world::RENDER_R`, follow it).
Above 16 the ring is 5x5 chunks and its outer chunks mesh as the existing far
LOD, `mesh_lod_tops` (two-block cells, tops and skirts, no plants); nothing new
was written. Frames of the far band on the recorded walk at poll 991 and in the
title fly-over show the extra ring as terrain and whole trees with no holes at
the join; `vista-manny-poll991-20blocks.png` (in the work directory) is one.

Real cadence on the three tapes, the frames longer than two vblanks (below 30
fps) over the frames drawn, PGO builds with the profile re-collected for each,
`psoxide-pgo measure`, polls 252..1990 (Manny) and 252..1200:

| tape | 16 blocks | 20 blocks |
|---|---|---|
| Manny's walk, 2026-10-15 | 20 of 1,734 (1.2%), work 1,128,828 a frame | 108 of 1,734 (6.2%), 1,152,911 |
| train | 2 of 947 (0.2%), 1,126,808 | 10 of 947 (1.1%), 1,127,910 |
| unseen | 0 of 947, 907,102 | 3 of 947 (0.3%), 902,787 |

(The view-distance work measured 9.9 percent below 30 fps on Manny's walk at 20
with its own builds; the share swings by several points between builds of the
same code, so read these as a range.) The two pixel-identical rejects it found
(faces wholly off screen before the seal, far plants by axis distance) are in
this branch; its PGO profile and crater literal are not, both were taken again
here.

Journeys: the 20 block title takes longer to settle (its first frames run 4 to 8
vblanks while the ring streams, then 3), so a press in the first two seconds
can fall between two polls; the journeys that press the pad first wait 120
ticks after the title shows, and the two loads in `settings-menu-quit` and
`save-repin` wait 500 ticks for the 5x5 ring to come back. The crater of
`save-repin` is still 138 edits.

## 2026-10-15: music (GitHub issue 1, item 3)

Music is scheduled like Java's (`game/src/music_plan.rs`, `music.rs`). `W:` is
`https://minecraft.wiki/w/`; the page is W:Music, section "Background music".

| Item | Code | Java value | Source | Result |
|---|---|---|---|---|
| Title screen | one piece starts a random 20 to 600 game ticks (1 to 30 s) after the title appears, `MENU_DELAY` | "a random menu track starts playing after 1 to 30 seconds" | wiki | matches |
| Joining a world | the menu piece stops when the world loads; `enter_world` | "Menu music stops playing" at the loading world screen | wiki | matches |
| Gap between pieces | a random 12,000 to 24,000 game ticks (10 to 20 min) after a piece finishes, `GAME_DELAY` | "randomly plays 10 to 20 minutes" after the current track | wiki (minutes); the tick figures are the same span at 20 ticks a second | matches |
| First piece in a world | 100 game ticks (5 s) after joining, `START_DELAY` | not on the wiki | recalled from the client's music manager source (its starting delay), not verified against the wiki | unverified |
| Looping | a piece plays once, then the gap | background music is not looped | wiki (jukebox music loops, background music does not) | matches |
| Pause menu | the music and the gap carry on | "When the game is paused, game music continues to play in Java Edition" | wiki | matches |
| MUSIC slider | 0 plays nothing and the gap runs on; changing it never restarts a piece; saved only by SAVE SETTINGS | Java has a music volume slider; the wiki does not describe its behaviour at 0 | slider position at 0 follows the client's sound engine as recalled, not the wiki | unverified |
| Quit | no world to quit to; the pieces stop at the title only when a world loads | "Game music stops playing when the player quits the world" | wiki | not applicable |

Deviations, deliberate:

- One shared pool. Java picks "a random general track" for the overworld and
  keeps separate lists only for some biomes (caves with dripstone or lush
  growth, the Deep Dark, oceans, the Nether, the End) and Creative mode. VoXide has
  four pieces, so the title screen, the overworld by day and night, caves, the
  Inferno and the Void all draw from the same four, with no mood tags.
- The pieces are four, not Java's fifty. With 10 to 20 minutes between them a
  session hears a piece or two an hour.
- No Music Frequency option and no music toast (Java 25w20a and later).
- The pieces are made offline from loops (`tools/loop-mixer`) because the PS1
  drive decodes one XA stream at a time, so two loops cannot overlap at run time.

Level: the pieces are cooked at -16 LUFS integrated (EBU R128, turned down
only, true peak under -1.7 dBTP; `tools/loop-mixer`), and the drive mixer sits 6
dB under that at 100 percent (`MIXER_FULL` in `music_plan.rs`). A headless
capture of the SPU output on the title screen (2026-10-15) measured the music
alone at -32.5 dBFS RMS at that setting, seconds peaking at 0.11 of full scale
(0.19 at most); the sound effects in a walking capture peak at 0.1 to 0.25, and at
the cooked level the music's peaks (0.23 typical) would have been level with
them. Nobody listened to the result; the level is a measurement, not an ear.

## 2026-10-15: attack bar on a distant block

Pressing R2 at a block beyond the 4.5-block reach restarted the attack cooldown
(the swing "found nothing", which Java also does), so the 16 x 2 cooldown bar
flashed under the crosshair on every try at a far block. Manny read it as a
loading bar; `e51418e` had only stopped the flash for blocks inside reach.

Deviation, on purpose: only a swing at a mob (or a hotbar switch) restarts the
cooldown now. Java also restarts it on a swing at air, so a Java player who
swings and then hits a mob within the cooldown does less damage; here that
swing costs nothing. Holding R2 never repeated a swing in either game.
`tests/distant-mining.toml` asserts no bar after taps and a hold-and-press at
the far terrain, and that a hotbar switch does show it.

## 2026-10-17: frame time after the SDK repin (0.4.1)

Pinning the SDK (5219afbe, then b5365b18) moved Manny's walk from 15 to 131
frames longer than two vblanks at the 20 block view distance, the train walk from
1 to 10: the game sat at about 98 percent of its two-vblank budget at the 20
blocks and a fraction of a percent more work, or a different code layout, tipped
frames over. Measured in cycles the cause was never one crate. This pass cut the
work instead of re-rolling the layout.

Method: `psoxide-pgo measure` on the three tapes for the three-vblank counts
(frontend bc7c066f, tapes `voxide-2026-10-15.pxtape` c28bb7d8, `pgo/train.pxtape`
9106aa80, `pgo/unseen.pxtape` 4417f58d); for each code change a lockstep build
(`--features "emulator-telemetry lockstep"`) of both sides replayed on the three
tapes with a display hash logged for every frame, and the in-guest loop body
(stage 50) compared frame for frame. Every commit below but the last has 0
differing hashes over 1,708 / 1,398 / 1,398 frames.

| change | loop body, Manny / train / unseen (PGO off) |
|---|---|
| near-face patches and cells classified on positions before shading | -2.1% / -1.7% / -2.3% |
| face cull survivors emitted once per chunk, not per plane (the cull loop plus plane test plus packet builder are 5.4 KB hot against a 4 KB cache) | -7.7% / -6.5% / -6.7% |
| chunk statics read once per call, a dead box test dropped, whole scratchpad for the face stack | -3.1% / -3.0% / -2.9% |
| direction masks kept arithmetic in the cull loop | -0.5% / -0.6% / -0.6% |

The streaming change is a policy and was measured on the real cadence only: when
the clock refused a slice six frames running one was forced, and in a heavy scene
(face loop 850K and up) that slice was the one that pushed the frame past two
vblanks, a late frame in about six. It now waits for a frame with room for it
before the true frame limit and goes regardless after 24 starved frames. The
slices a visible hole or an edit guarantees are untouched. (48 to 38 on Manny's
tape.)

Result (three-vblank frames of the frames drawn, PGO re-collected on the train and
unseen tapes for the shipped commit; the 9bb73fc line is the installed build):

| tape | 9bb73fc | this branch |
|---|---|---|
| Manny's walk | 131 of 1,734 | 38 |
| train | 10 of 947 | 1 |
| unseen | 1 of 947 | 0 |

The 0.4.1 target was 15 and 1; Manny's walk is not there. What is left on it is
mostly the slices a hole near the player guarantees (3 slices, 100K to 400K cycles
in one frame) on top of a body that already sits near 0.95M of 1.14M. Before the streaming change the same
code gave 48 to 68 with different profiles (rebuilds of identical source move the
count by +-10), so read 38 as a range. Training the profile on Manny's own tape
did not help (68).

Tried, identical pixels, and slower or no better: plane flags that let the cull
skip tests per plane (+1.6%), the clip kernel shared out of line (+0.3%), a lean
emitter for cells inside all six clip planes with a proof that drops cells deeply
outside a later plane (clipper calls 30 to 16 a frame, but +1.4% overall), column
spans for the player's collision probe (+0.3 to 0.6%), the `+order` layout (121
against 131 before these changes), PGO variants other than `hot=500+profi` (all
within 0.3%).

Constraint worth knowing: the face pass runs on the scratchpad stack and the
deepest call chain was 1000 of 1004 bytes; any growth of the cull, the packet
builder, the near-face tessellation or the clipper stopped the link. The cull's
frame is now gone before the builder runs (800 bytes).

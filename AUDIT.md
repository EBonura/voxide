# VoXide tuning audit

VoXide matches Minecraft Java Edition values. Each fix below cites minecraft.wiki
(`W:` is `https://minecraft.wiki/w/`). Clocks: everything in `update_survival` and
`mob::update` runs on the 60 Hz sim tick, and `units::java_ticks(n)` converts Java
game ticks (20 a second) to it, so a 20 TPS constant cannot run at 60 Hz by accident.

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

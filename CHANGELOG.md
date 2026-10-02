# Changelog

## Unreleased

Thunderstorms and saves, test disc only.

- Java's thunderstorms: lightning bolts drawn Java's way, a flash on the
  sky and the world, thunder heard everywhere, the crack within 32 blocks,
  darker storm clouds. Strikes come at Java's rate (about 2.4 a minute
  within 128 blocks), land on the highest block or on a creature that sees
  the sky, set fire around them, deal 5 damage, burn up dropped items and
  charge sappers (twice the blast).
- Rain and thunder levels and the sky light follow Java's formulas, so a
  shower darkens the world as it rolls in; rain puts fires out.
- Saves keep the weather (format 11). A save writes one card frame per
  game frame with a progress bar instead of freezing the screen, and is
  written beside the previous save, so pulling the card mid-save keeps
  the old one. Older saves still load.
- Cheat menu: WEATHER (clear, rain, thunder, or a lightning bolt).

## 0.3.0 | 2026-10-01

Standalone disc published on itch.io.

Java Edition parity, round two, and a review round.

- The world runs on Java's clocks: a 20-minute day, Java's weather (now
  with thunderstorms), crops and saplings on random ticks, 10 s smelting,
  a 4 s TNT fuse and portal.
- Java's day and night: sky light steps down at dusk and up at dawn by
  Java's table (darker in rain, darker still in a storm) while the screen
  fades smoothly; the sun and moon follow Java's eased arc, crossing the
  horizon at 12,786 and 23,216, with the sunset glow centred there.
  Monsters spawn outdoors from 13,188 to 22,812 in clear weather and by
  day in a thunderstorm; beds work and the undead burn on Java's ticks.
- Java's hunger (exhaustion and saturation), 1.9 attack cooldown, crits,
  and durability for tools, armour, the bow, the rod and flint and steel.
- Only the pause menu pauses the world; inventory and containers do not.
- Mobs see before they hunt and stop at touching distance; skeletons keep
  their range and shoot to 15 blocks; creepers (sappers) fuse within 3
  blocks; tamed wolves follow, fight for you, persist and are saved.
- Blocks are held as their 3D models, items as flat icons in the fist.
- The sun and moon are Java's size; the sun has its halo and sets behind
  the horizon.
- Sprinting needs food above 6; sink sand slows you; ladders climb when
  you push against them; you can attack and build underwater.
- Saves keep the world's seed, respawn point, day and tamed wolves, and a
  load rebuilds the world (it kept blocks placed after the save).
- Fixes: ladders no longer cause fall damage; burning or drowning no
  longer makes you immune to mobs; embers and wailers hover instead of
  rising forever; farms keep growing; held buttons in the furnace and
  crafting no longer roll onto the next item; L2 acts on the mob under
  the crosshair; an exploding sapper drops nothing; a sapper blast stalls
  the game half as long.

## 0.2.1 | 2026-09-26

Standalone disc published on itch.io.


- The right stick turns 30% faster: a full push is 117 degrees a second of
  yaw and 98 of pitch at LOOK SPEED 100% (was 90 and 75).
- Mobs take the crosshair. A mob between you and a block is targeted, as in
  Java: the block outline and break cracks no longer land on the block
  behind or under it, R2 strikes that mob (damage and knockback), and
  holding R2 no longer mines through it. Hitboxes use Java's widths (pig,
  cow and sheep 0.9 blocks, chicken 0.4, spider 1.4) and reach up to each
  model's drawn height.
- A hidden cheat menu for testing: from OPTIONS, hold L1 + R1 and press
  Select. Flight, god mode, a coordinates and FPS readout, a full
  inventory, time of day, spawning any mob, back to spawn, and travel to the
  Inferno, the Void or the overworld. Using one marks the world's save;
  saves from worlds that never do are unchanged.

## 0.2.0 | 2026-09-26

Standalone disc published on itch.io.


- Gameplay follows Minecraft Java Edition's numbers: walking, sprinting, sneaking,
  jumping and swimming, block break times, combat and explosions, burning, healing
  and hunger, mob spawning by light level, day length, fluids, fishing, experience,
  breeding and taming. All game time runs on one 60 Hz clock.
- The inventory is an icon grid with the hotbar as its last row; chests and
  furnaces open as two panes and move whole stacks.
- Saves keep chests, furnaces, the hotbar, enchant levels, the food pouch, the
  current dimension, return portals and the Void's dragon. Older saves load.

## Source 2026.09.05

This source snapshot is tagged `source-2026.09.05`. Download versions are
listed separately below; source cleanup does not replace an already published disc.

- Switched standalone builds to the extracted SDK and separate emulator.
- Removed unused terrain helpers and formatted Rust sources.

## 0.1.11-split.20260905 | 2026-09-05

Standalone disc published on itch.io.

- Published the SDK split build of VoXide 0.1.11.

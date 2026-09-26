# Changelog

## 0.2.1 (not yet published)

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

# Changelog

## Unreleased

Equipment, and fixes from a playtest.

- The inventory opens on a player page, as Java's survival screen does: the
  four armour slots down the left, the player in the middle wearing what you
  put on, the off hand, your armour and defense, the weapon you swing, the
  armour you carry and the hotbar. L1 and R1 page between it and the item tabs.
- Armour is made a piece at a time at a crafting table (helmet, chestplate,
  leggings, boots, in iron or diamond) for Java's 5, 8, 7 and 4 ingots, and
  worn from the player page. Each piece keeps its own wear and takes it off
  with you, a worn piece that runs out is gone, and defense, toughness and the
  armour bar are Java's. Before, one recipe equipped a whole set at once.
- The player page has Java's 2x2 crafting grid and an output slot. Lift a
  stack with X, put it down with X, put one down or lift half with SQUARE,
  send a cell back with TRIANGLE; X on the output crafts once, TRIANGLE crafts
  as many as the cells allow. Shapes are matched wherever they sit: planks from
  a log, sticks from two planks one above the other, a crafting table from four
  planks, torches from coal over a stick, bone meal from a bone. What is in the
  grid goes back to your inventory when the screen closes.
- The SQUARE crafting list away from a table still crafts with X. It is also
  the recipe book: R2 on planks, sticks, a table, torches or bone meal lays the
  shape in the grid for you and takes you to the output. At a crafting table
  nothing changes.
- Pick the weapon you swing at mobs: fist, sword, axe, pickaxe or shovel, each
  with Java's damage and attack speed, and each wearing as Java's does (a
  sword loses one use a hit, the other tools two).
- L1 and R1 together swap the item in your hand with the off hand, as Java's
  swap-hands key does. Put an item in the off hand from the player page.
- Coal, iron, gold, diamond, sticks and the rest of what you collect now go
  on the hotbar like any other item, so a mined ore shows up where you look.
  Nothing is placed from them. Ore still drops only to the pickaxe Java asks
  for: nothing from coal by hand, nothing from iron below stone, nothing from
  gold or diamond below iron.
- The short bar under the crosshair when you started mining a block was the
  attack cooldown. Java only restarts it when a swing hits a mob or finds
  nothing, so mining no longer shows it.
- Glowstone breaks at the same speed with any tool, as in Java.
- Saves are version 12 and keep what you wear, the off hand and the weapon.
  Saves from before load wearing the armour tier they had.
- BRIGHTNESS and SCREEN X / SCREEN Y join the settings, on the main menu's
  SETTINGS page and in OPTIONS. BRIGHTNESS runs from DARKER 5 through DEFAULT to
  BRIGHTER 5 and covers the HUD and menus; DEFAULT draws exactly what it did
  before. SCREEN X / Y move the picture up to 16 pixels for a television that
  crops the edge. All three are saved with the stick and volume settings;
  settings saved by an earlier version load with them at their defaults.

## 0.3.2 | 2026-10-08

Portals, controls and world speed fixes, on top of 0.3.1.

- Dying in the Inferno or the Void now sends you back to the overworld, as in
  Java. Before, you came back at the overworld spawn's coordinates inside the
  dimension you died in.
- Holding L2 keeps placing blocks, one every four game ticks, so a row or a
  pillar goes up with the button held, as with Java's use button.
- Holding Cross jumps again as soon as you land, ten game ticks apart, as in
  Java. Before, each jump needed a fresh press.
- Saving to a memory card with no directory asks first: the first save says
  the card will be formatted and the second does it. Before, it erased
  whatever the card held at once.
- The tutorial hint box in the Void is drawn below the dragon's boss bar
  instead of over its title.
- Portals: each sheet is drawn as the two blended faces of Java's portal model,
  cut to the frame and animated, instead of two opaque quads that poked
  through the obsidian and vanished up close and far away. The Void portal is
  a sheet you can walk through, like the Nether portal. Standing in a portal
  tints the screen purple, and the crosshair reaches the block behind a sheet.
- Faster world: meshing a chunk no longer scans every cell for skylight,
  lights and plants, and a spreading fluid rebuilds only the planes it
  touches, so a flowing lake or lava stream no longer drags the frame rate
  down.
- Newer PSoXide SDK, engine and emulator components, with the build profile
  collected again for them.

## 0.3.1 | 2026-10-02

Standalone disc published on itch.io.

Thunderstorms, saves and the cheats page.

- Java's thunderstorms: lightning bolts drawn Java's way, a flash on the
  sky and the world, thunder heard everywhere, the crack within 32 blocks,
  darker storm clouds. Strikes come at Java's rate (about 2.4 a minute
  within 128 blocks), land on the highest block or on a creature that sees
  the sky, set fire around them, deal 5 damage, burn up dropped items and
  charge sappers (twice the blast).
- Rain and thunder levels and the sky light follow Java's formulas, so a
  shower darkens the world as it rolls in; rain puts fires out.
- Rain, snow or nothing by biome as in Java: deserts stay dry under a
  storm sky, cold and high ground gets snow. Rain has a sound (muffled
  under a roof); thunder comes from the strike's side. The distant haze
  darkens with the storm instead of staying sand-coloured.
- Saves keep the weather (format 11). A save writes one card frame per
  game frame with a progress bar instead of freezing the screen, and is
  written beside the previous save, so pulling the card mid-save keeps
  the old one. Older saves still load.
- The cheats are a CHEATS page in OPTIONS (the hidden combo is gone),
  with WEATHER: clear, rain, thunder or a lightning bolt. Using them no
  longer marks the world, as in Java.

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

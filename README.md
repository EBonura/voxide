# VoXide

A Minecraft-style survival sandbox for the PlayStation 1, written in Rust on
top of the PSoXide SDK.

The world is chunked and streamed. Chunks are 16x16x64 and recentre on the
player as you walk, so it never runs out. Terrain comes from seeded fixed-point
noise, which gives biomes, caves, ore banded by depth, lava pockets and a sea
level, with trees scattered on top of that.

From there it's most of the survival loop: mining and placing with tool tiers,
crafting, smelting, hunger, farming, mobs that fight back, and the Inferno and
the Void if you build the portals. `FEATURES.md` has the full matrix, including
the places where this port deliberately does something other than what Java
does, and why.

Performance is the part worth talking about on this hardware. Detailed terrain
draws 16 blocks ahead and 14 to the sides, projected on the GTE, using 2D
greedy meshing over cached per-chunk face pools that only rebuild the planes an
edit actually touches. That holds 30fps most of the time. The worst route I
measured averages 28.8 and misses the deadline on a few percent of frames,
usually while mining or panning fast across dense terrain. What limits draw
distance is the cost of projecting faces, not memory: I tried imposters for the
far ring and pulled them back out after measuring that they drew no visible
pixels.

Every asset is CC0. Block art is the "16x16 Block Texture Set" from
OpenGameArt, quantised to per-block CLUTs by `tools/convert_pack.py`; tiles the
pack doesn't cover are drawn in code. Sources are credited in
`assets/pack/CREDITS.md` and on the in-game credits screen. No Mojang assets,
code or data are used.

## Build

VoXide uses the PSoXide SDK as Cargo **path** dependencies, so the two
checkouts have to sit side by side:

```bash
git clone https://github.com/EBonura/PSoXide.git
git clone https://github.com/EBonura/voxide.git
cd voxide && make
```

```bash
make compile      # PSX-EXE only
make disc         # dist/voxide.cue + dist/voxide.bin
make install      # copy the disc into the local game library
make smoke        # boot through PSoXide headless and write captures/
```

The build needs a nightly Rust toolchain (pinned by `rust-toolchain.toml`,
which also pulls `rust-src` for the `mipsel-sony-psx` target) and Python 3 for
the asset cooks in `tools/`.

## Controls

The layout follows Minecraft Bedrock's PS4/PS5 defaults.

- Left stick: move (forward/back + strafe); D-pad works as a fallback
  (up/down walk, left/right turn)
- Right stick: turn and look
- Cross: jump; hold it in water to swim up; in creative fly (toggled in the
  options menu) Cross rises and Circle sinks
- Circle: hold to sneak (slow, and you will not walk off a ledge); backs
  out of any menu
- Square: pocket crafting (planks, sticks, torches, the crafting table);
  everything else needs a placed crafting table, opened with L2, as in the
  original. L1/R1 page the category tabs (blocks, gear, items, food), D-pad
  selects, Cross crafts, Triangle hides what you lack materials for;
  Square also withdraws inside the chest/furnace panels
- Triangle: inventory panel, opened on the item in hand (Cross equips);
  shows what you own by default, Square switches to the full catalogue
- L2: use -- interact with the targeted mob or block (chest, furnace, bed,
  enchant table, door), otherwise place/use the held block or item (bow,
  bucket, wheat, ...); sneak + L2 force-places
- R2: tap to attack, hold to mine the targeted block (time scales with
  hardness and tool)
- L1 / R1: hotbar previous/next (9 slots, wraps; an empty slot is an empty
  hand)
- L3 or double-tap forward: sprint (drops when forward stops)
- Start: options menu (toggle flight, save/load to memory card)
- Main menu: up/down moves between PLAY GAME, NEW WORLD, SETTINGS and
  CREDITS; Cross confirms

A first run walks you through the basics with a short chain of hint toasts
(look, move, chop, craft, bench); OPTIONS > TUTORIAL turns it off.

There are four tools, a pickaxe, axe, shovel and sword, each crafted per tier
from wood up to diamond. They aren't cycled or selected: there's no durability,
a better one auto-equips, and the game picks whichever suits the block under
your crosshair, so the slot beside the hotbar changes as you look around. Using
the right tool is what makes a block break quickly; the wrong one works at
bare-hand pace. Only a pickaxe of the right tier will yield ore, and melee
damage comes from the sword.

Holding up or down in any menu autorepeats after a beat.

The HUD carries hearts, hunger, air bubbles, armour pips, the XP bar and the
hotbar. Mining draws crack stages on the block itself rather than a progress
bar.

## A note on names

VoXide is an original implementation inspired by Minecraft. It ships no Mojang
assets, code or data, and it is not affiliated with, endorsed by or associated
with Mojang Studios or Microsoft. Minecraft is a trademark of Mojang Studios.

Blocks, mobs and dimensions that Minecraft calls by invented names are called
something else here, so the vocabulary is this project's own: the two other
dimensions are the Inferno and the Void, and you will meet sappers, wraiths,
wailers and embers rather than their counterparts.

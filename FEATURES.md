# VoXide feature matrix

What this port does and doesn't do, checked against classic Minecraft survival
(Java-era core). Last audited 2026-08-02. Legend: [x] done, [~] adapted for the
PS1 with the divergence recorded, [ ] missing, [-] out of scope for the
hardware.

## World
- [x] Chunked, streamed world (toroidal recenter, effectively infinite)
- [x] Seeded terrain: biomes (ocean/plains/desert/mountain/snow), caves, ores
      (coal/iron/gold/diamond), lava pockets, sea level, trees
- [x] Day/night cycle; rain; snow biome
- [x] 16-block detailed draw ahead (14 to the sides), GTE-projected; the
      shipped config holds the 30fps cadence for most of a session, with the
      worst measured route at a 28.8fps mean and single-digit percentages of
      frames missing the deadline while mining or panning across dense terrain
- [x] Random world seed: NEW WORLD on the main menu (menu-timing
      entropy reseeds gen; full world+registry reset, verified via pad injection)
- [x] Flowing water and lava (see Fluids)
- [x] Inferno: a real second dimension on the SAME chunk ring (a generator
      switch, not a second world in RAM). Cinderstone shells, lava seas,
      lumistone, sink sand, ember cap; obsidian portal lit with flint and
      steel, 8:1 coordinate scaling, a return portal built on arrival
- [x] Void: a floating void-stone island over the emptiness with obsidian
      pillars, reached through an obsidian frame lit with a void eye
- [x] Void dragon: flies (phases through terrain), 200 hp, boss bar, keeps a
      nine-block standoff and circles
- [x] Skylight shading: faces under cover render dark, faces under open sky
      bright. Derived per column from the highest sky-blocking block, so no
      light array and no BFS; the bucket rides in the face mask so the greedy
      merge refuses to merge across a light step. Measured free (1,141,905
      cycles vs 1,141,820 baseline)
- [x] Torch / lava / lumistone / fire light: sources found in the same pass as
      the skylight, then flooded into a 1-bit-per-cell bitmap rebuilt per mesh.
      No per-block light array, no BFS. Also measured free (1,141,922)

## Blocks & building
- [x] ~55 block kinds incl. glass, chest, crafting table, furnace, bed, TNT,
      doors, cactus, clay, brick, saplings, redstone wire/torch/piston, wheat
      crops, wool, ladder, cobblestone, enchant table (48 of them placeable)
- [x] Mine (hold, per-block hardness, tool tiers + efficiency) / place
- [x] Correct drops (stone->cobble, ores, leaves) + block XP
- [x] Transparent blocks (water/glass) with PS1 average-blend
- [x] Saplings: leaves drop them ~30%, plant on soil, grow into trees
- [x] Doors: 6 planks craft 3 (Java), use (L2) toggles open (invisible+passable)
      / closed
- [x] Cactus: desert gen 1-3 tall, contact damage, placeable
- [x] Clay -> brick: shallow-sea clay patches smelt into brick blocks
- [x] Slabs, stairs (4 facings, placed by your yaw) and fences, drawn by the
      small-block pass rather than the greedy mesher; collision follows the
      shape (stand on a slab at half a block, walk under one)
- [x] Crafting: a recipe list rather than a free-form 2x2/3x3 grid. Not a
      shortfall -- Minecraft's own console editions shipped menu-driven
      crafting for exactly this reason, because placing items into a grid with
      a d-pad is worse than picking the thing you want to make. Like those
      editions it is tabbed by category (L1/R1: blocks, gear, items, food),
      with a toggle to hide unaffordable recipes and the selected recipe's
      cost on the bottom line, red until the inventory covers it. The
      handheld menu is the 2x2 pocket grid (planks, sticks, torches, the
      table itself); the rest wants a placed crafting table, as in Java

## Survival loop
- [x] Health, hunger, regen-gated-on-food, starvation, i-frames
- [x] Fall damage, drowning + air bubbles, lava + burning, fire douse
- [x] Death -> respawn (bed sets spawn point)
- [x] Furnace smelting (iron ore->ingot, sand->glass, cobble->stone) with fuel
- [x] Cooked food chain: passive kills drop raw meat; furnace cooks it (+6 vs +3)
- [x] Furnace fuel variety: coal 8 smelts, wood/planks 2
- [x] Swimming: buoyancy in water -- hold jump to stroke up, otherwise a slow
      sink; entry breaks a fall
- [x] Death screen: the red YOU DIED pall, X to respawn (bed spawn honoured)
- [x] Contextual tutorial: a Java-style hint chain in a top-right toast --
      look, move, jump, chop, planks, table, place, open, tool -- each step
      completing off live game state; sleep prompt when aiming at a bed at
      night; OPTIONS toggle
- [x] Sprint: L3 or double-tap forward (latched), 1.3x speed, 3x exhaustion
- [x] Sneak (hold CIRCLE): ~30% speed, will not step off a ledge, eye drops
- [x] Brewing: bottle -> awkward (ember cap) -> effect, as recipes. Speed,
      strength, regeneration, fire resistance
- [x] Sugar cane generates on sand beside water and is the speed ingredient
- [x] Ember rods, wailer tears and magma paste come from the Inferno mobs that
      drop them in Java, so every potion brews from its own ingredient
- [x] Enchanting: efficiency, sharpness and protection, granted round-robin

## Items & progression
- [x] Inventory, scrolling hotbar, held-item view
- [x] Four tool types (pickaxe, axe, shovel, sword) x four tiers (wood..diamond,
      iron gated behind smelting). The right tool mines its material family
      fast and the wrong one works at hand speed; ore drops gate on the pickaxe
      tier and melee damage on the sword, as in Java
- [~] Tools are player stats rather than inventory items: no durability, the
      best of each auto-equips, and the one matching what you aim at is used.
      A d-pad has no good way to swap tools mid-swing, so the game picks
- [x] Armor (iron/diamond)
- [x] Bow + arrows (player ranged), buckets, fishing rod, bonemeal, bread
- [x] XP + levels + enchant (mining efficiency)
- [x] Gunpowder: slain sappers drop it; TNT = 5 gunpowder + 4 sand (Java)
- [x] Item entities: mined blocks pop out, fall, settle, get picked up
- [x] Mob loot drops as item entities where the mob died
- [x] String: spiders drop it, and it is the bow's cord

## Fluids
- [x] Flowing water: Java levels 0..7, falls before spreading, dries up when
      the source goes
- [x] Flowing lava: Java's 3-block overworld range, six times slower than water
- [x] Water/lava contact turns fluid to stone or cobble
- [x] Infinite source (two adjacent sources making a third)
- [x] Water on a lava SOURCE gives obsidian, on flowing lava gives cobble
- [x] Fire: lava ignites flammable blocks, flames burn out and consume their
      fuel, water douses, standing in it burns you
- [x] Fire spreads between flammable blocks; the 16-flame pool bounds it

## Mobs
- [x] 8 kinds: pig/cow/sheep/chicken + zombie/skeleton/sapper/spider
- [x] FSM AI (wander/chase/flee), jump-when-blocked, spider wall-climb
- [x] Day/night spawn gating, sun-burning undead, despawn by distance
- [x] Combat: melee (tool-scaled), knockback, skeleton arrows, sapper
      fuse+explosion, contact damage
- [x] Drops: food, wool, bones, XP; breeding + wheat feeding/luring
- [x] Textured faces + mottled hide bodies, leg LOD
- [x] Distinct meat drops: passives yield raw meat for the cooking chain
- [x] Wraiths (neutral, blink when struck, drop void pearls -> void eye),
      wolves (tame with a bone, then follow), villagers (trade 8 wheat for iron)
- [x] Inferno mobs: ember and wailer hover, charred skeletons walk; all three
      spawn only in the Inferno and drop the brewing ingredients

## Presentation & audio
- [x] Title screen (orbit camera), sky (gradient sun/moon/stars/clouds);
      main-menu CREDITS card lists the CC0 asset provenance
- [x] Particles (break debris, explosion smoke, hearts), rain streaks
- [x] Sampled CC0 sound effects via SPU-ADPCM (dig and steps voiced by
      material, place/hurt/eat/explode/door/chest/UI...)
- [x] MC-faithful block art (CC0 pack + per-block CLUTs; sand matched to
      minecraft.wiki reference)
- [-] Music: deliberately none -- the world's own sounds carry the atmosphere
- [x] Mob voices: distance-attenuated recorded calls (pig oink, cow moo,
      sheep baa, zombie groan, sapper fuse hiss...), footsteps by material,
      water-entry splash, door and chest sounds
- [x] HUD: hearts, hunger, air, armor pips, XP bar, hotbar; mining draws
      crack stages on the block

## Persistence
- [x] Memory-card save/load from the in-game OPTIONS menu (START). Writes a
      console-visible "VOXIDE" file; restores player, inventory and block edits

## Rendering performance, measured

Numbers from the final `make profile` routes. The measured two-VBlank period is
1,142,476 profiler bus cycles; this observed cadence avoids the old false
29.7fps reading derived from the nominal CPU clock.

| route | frames | loop body | mean frame | effective fps | below 30fps |
|---|---:|---:|---:|---:|---:|
| mining / dirt+stone placement / turning | 1,562 | 913,934 | 1,141,595 | 30.0 | 0 |
| four-direction full-speed flight + recovery | 3,704 | 549,692 | 792,622 | 43.2 | 0 |

The traversal crosses chunk boundaries in +Z, +X, -Z and -X, including the
dense reverse view that previously froze. Streaming now stages decode,
lighting, plane meshing and atomic pool publication, and spends zero, one or
two slices according to current quad and candidate-face load. Player edits
update block collision immediately and rebuild affected mesh planes across
bounded frames.
Both routes completed without a panic, corrupt DMA chain, or 30fps cadence
miss.

Before the renderer pipeline, walking was bimodal: 78% of frames landed at two
VBlanks and 22% at three. GPU raster was serialised after the CPU. The
double-buffered pipeline reduced residual GPU drain from roughly 110K cycles to
about 60 cycles by overlapping it with the next frame's CPU work.

## Double-buffered ordering table: built

Frame N's sky/world chains rasterise while the CPU builds frame N+1. Every
DMA-reachable packet allocation is double-buffered; shared counters and UI
link metadata are build-only and can safely reset once their packets are
linked.

Additional BSS, computed from the current pool sizes:

| pool | bytes to duplicate |
|---|---|
| QUADS (1536) | 61,440 |
| AO_QUADS (1024) | 57,344 |
| UI_POOL | 32,768 |
| MOB_QUADS (288) | 6,912 |
| PLANT_QUADS (128) | 5,120 |
| OT (1024 slots) | 4,096 |
| BODY_QUADS + HEAD_QUADS | 1,600 |
| SKY_OT | 8 |
| **total** | **169,288** |

The per-frame dither GP0 register write is also a packet in SKY_OT. That is
important: no gameplay renderer touches GP0 while the previous arena may still
be rasterising. `frame_present` waits for DMA and raster completion, flips at
VBlank, writes the next framebuffer's draw area, and only then submits the two
new chains. The cost is one rendered frame of input latency (about 33ms at
30fps).

Validation also records dirt and stone inventory counters at scripted
checkpoints. Stone falls from 4 to 3 through the real R1/L2 selection and
placement path, preventing the earlier test from silently treating a no-target
input as successful placement.

## Why the overworld is a desert, corrected

SPAWN_GREEN finds a plains region from the noise field and spawns there. It is
off. Five controlled builds on the same route isolate terrain, trees and
decorative plants:

| build | world pass | fps |
|---|---|---|
| desert (shipped) | 592,246 | 29.7 |
| green, no trees or decorations | 495,577 | 29.6 |
| green, trees only | 770,215 | 23.5 |
| green, decorations only | 676,862 | 24.6 |
| green, trees + decorations | 959,953 | 18.8 |

The green terrain itself is slightly cheaper than the desert. Trees add about
275K cycles in isolation and decorative plants about 181K; those costs are not
strictly additive because trees replace some decorations and change visibility.

This corrects a committed bad decomposition. The earlier DIRT-surface control
silently disabled decorations because `maybe_decoration` only accepts GRASS.
It therefore mislabelled the missing plants as a 205K grass/dirt terrace split.
A direct side-face remap tested that claim without touching generation and
changed the world pass from 959,953 to 961,601 -- measurement noise, not a
saving. There is no terrace penalty to merge away.

Density can buy speed, but the tested settings describe a visual trade rather
than a free fix:

| lever | result |
|---|---|
| trees 13/48 -> 90/340, decorations 32% -> 5% | 26.7 fps |
| trees 13/48 -> 26/96, decorations 32% -> 8% | 22.4 fps |
| trees 13/48 -> 70/260, decorations unchanged | 21.7 fps |
| trees 13/48 -> 30/110 | 20.1 fps |
| single-billboard distant plants | 18.8 fps |
| adaptive far cull, down to FAR_Z/4 | 24.3 fps |
| existing LOD (merge cap 12 -> 15) | 18.4 fps |
| ray-cast occlusion culling on every ring | 12.7 fps |

Green at 30 is therefore possible only when nearly bare; green with the current
Minecraft-like vegetation is not. The remaining choices are to accept a lower
frame rate, thin the vegetation visibly, or reduce the cost of tree faces and
per-plant projection. A real coarse terrain LOD may help filled views, but it
was never the only possible answer to this particular spawn.

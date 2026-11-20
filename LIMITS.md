# Fixed limits

VoXide has no allocator. Every table below is a static array, sized at build time, and every one that
can fill up has a stated behaviour when it does. Numbers are as of the block-state work (2026-10-19).

## RAM

The image region is 2 MB less 64 KB reserved and 32 KB of stack. `.bss` is not in the PS-X EXE size, so
free RAM is read from the linker map (`__bss_end` against 0x801F7F00), not from the file.

| Build | Free |
| --- | --- |
| 0.4.1 (ac39270) | 166 KB |
| Block states, shapes, redstone, bow, edit store | 37,800 B |

The edit store is the biggest single item (51 KB for 4,096 edits, against 11 KB for the old log). The
lab discs (block-lab and friends) overflow `.bss` with the profile-ordered layout and are built with
`PGO_VARIANT=off`; they are never shipped.

## World edits

Edits are kept per chunk (edits.rs): 4,096 edits across at most 256 chunks, 7 B each plus a position
hash. A save writes them grouped by chunk at 3 B per edit (format 13; formats up to 12 still load).
When the store is full the edit is refused and the toast WORLD EDIT LIMIT shows; a blast logs its
craters only if all of them fit, so a crater never comes back half filled. The old global log held
1,024 and dropped the rest silently (measured: 476 of 1,500 lost).

## Redstone

256 emitters (dust, redstone torches, levers). Power is derived and not saved. A redstone tick is
0.1 s (6 sim ticks); a torch re-reads its block once per tick, so a ring of torches is a clock.

## Shapes in view

Per frame 192 quads across plants, shaped blocks and dust, 64 of them reserved for shapes. Per chunk
128 plant and shape entries, shapes counted first, so a meadow cannot starve a door. 64 player-placed
torches are registered for the mob-spawn light rule and the close-range shell (a ring: the 65th replaces
the oldest).

## Block light

Torches, lava, fire and lumistone light the ground in three tiers, not Java's fifteen levels: a torch
(Java level 14) lights to 3, 6 and 9 blocks (Java levels 11, 8 and 5 and up, so 116, 88 and 56 of 128
of full brightness) where Java fades one level a block out to 13; lava, fire and lumistone (level 15)
reach 4, 7 and 10. A face carries its tier beside its sky bucket (four bits: sky bucket in 8..=9, tier in
10..=11 of its side word) and is shown at the brighter of the two, a channel at a time (never the sum),
in the neutral shade of the texture. This is a deliberate difference from the original: per-block
falloff would need 14 block levels against 4 sky buckets = 56 light levels, a colour table of
56 x 768 B = 43 KB (this one is 12 KB) and 6 face bits, and a table build several times dearer.

The reach is clipped to the chunk being meshed and goes through walls (Java's does not): a torch under a
one-block cover still lights the ground around it. Blocking by solid blocks would need a flood through
air cells per torch (up to 1,160 cells each, a queue and a per-cell neighbour read from uncached RAM,
roughly 70,000 cycles a torch against 5,000 now), run for every torch of a chunk on every remesh, and
the torches of the neighbouring chunks would need those chunks' blocks.

Per chunk: 32 torches (those in it, then those of the eight around it whose light reaches in, read from
the edit store) and, apart from them, 6 other emitters (lava, fire, lumistone). When more torches than
that reach a chunk, the ones farthest from the player (taxicab, at the time of the mesh) are left unlit.
Fixed cost, not growing with torches: the tier maps are 6 KB of scratch (one chunk at a time), the colour
table 16 light levels (12 KB, was 3 KB), the torch list 384 B. Far-LOD chunks are skylit only.

## Block states

A cell is one byte. Ids below 128 are kinds; families (door 16 states, bed 8, torch 5, redstone torch 5,
lever 10, ladder 4, piston 12, piston head 6) use 58 of the 128 ids from 128 up. 70 are left.

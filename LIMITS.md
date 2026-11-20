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
56 x 512 B = 28 KB (this one is 16 x 512 B = 8 KB: a face shade has four rows, not six) and 6 face bits, and a table build several times dearer.

Torch light is stopped by solid blocks, as Java's is, and goes round corners: it is a flood through
the cells that let light pass (air, water, glass, plants, shaped blocks such as doors and slabs,
leaves),
counted in steps, so the tiers are 3, 6 and 9 steps from the torch and a thin wall does not shorten
the way. A torch in a pit under a cover leaves the ground round it dark, a torch in a tunnel does
not light the tunnel behind two blocks of rock, a torch in a doorway lights what is in front of the
opening. The flood is worked out once for each torch and kept (212 B: the three tiers as columns of
bits). It is worked out again when the torch is placed, when a block within 9 of it changes between
letting light through and not (placing or breaking stone, a door is not either), and when a chunk
next to it comes into the loaded ring. Cells of a chunk that is not loaded count as stone: at the
edge of the ring the light stops at the border, and goes on when the neighbour streams in. One flood
a frame, about 100,000 cycles each (a block break beside a torch shows the old light for the edit's
remesh and one more frame, then the chunks the light reaches are remeshed). A chunk's mesh reads the
kept light of the torches near it for about 19,000 cycles a torch: a chunk with 32 torches in reach
sets its lighting up in about 620,000 cycles, spread over the streaming steps (one torch a step).
A face takes the light of the cell in front of it, so the tier maps carry a ring of columns round the
chunk.

Per chunk: 32 torches (those in it, then those of the eight around it whose light reaches in, read from
the edit store) and, apart from them, 6 other emitters (lava, fire, lumistone). When more torches than
that reach a chunk, the ones farthest from the player (taxicab, at the time of the mesh) are left unlit.
The kept lights are 32 in all (the 32 nearest the player when more are placed: a torch without a place
does not light until one frees). Fixed cost, not growing with torches: the tier maps are 7.8 KB of
scratch (one chunk at a time, ring included), the colour table 16 light levels (8 KB, was 3 KB), the
torch list 384 B, the kept lights 6.8 KB (32 x 212 B); the flood works in the first 3.5 KB of the mesh
face scratch, which is empty while a mesh sets its lighting up. Far-LOD chunks are skylit only.

Not stopped by walls: lava, fire and lumistone (their light still goes through blocks, within their
own chunk), and the light the mob-spawn rule sees (main.rs `block_light`, taxicab). Leaves, glass and
water let light through without dimming it (Java dims leaves and water one and two levels more).

## Block states

A cell is one byte. Ids below 128 are kinds; families (door 16 states, bed 8, torch 5, redstone torch 5,
lever 10, ladder 4, piston 12, piston head 6) use 58 of the 128 ids from 128 up. 70 are left.

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
128 plant and shape entries, shapes counted first, so a meadow cannot starve a door. 64 torches are
registered for block light (a ring: the 65th replaces the oldest).

## Block states

A cell is one byte. Ids below 128 are kinds; families (door 16 states, bed 8, torch 5, redstone torch 5,
lever 10, ladder 4, piston 12, piston head 6) use 58 of the 128 ids from 128 up. 70 are left.

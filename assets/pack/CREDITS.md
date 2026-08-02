# Asset sources

Everything shipped with VoXide is CC0 1.0 (public domain). No Mojang assets,
code or data are used anywhere in this project.

## Block textures

The "16x16 Block Texture Set" from OpenGameArt, CC0 1.0.

* Source: https://opengameart.org/content/16x16-block-texture-set
* License: https://creativecommons.org/publicdomain/zero/1.0/

`tools/convert_pack.py` quantises the pack into a 4bpp atlas with one CLUT per
block and writes `game/src/texdata.rs`. Tiles the pack doesn't cover are drawn
procedurally in `game/src/tex.rs`: the crafting table, chest, the four tool
icons, the crack overlays and every mob face.

## Sound effects

Recordings are trimmed and resampled with ffmpeg, then cooked to SPU-ADPCM by
`tools/convert_sfx.py`, which writes the sample bank to
`assets/sfx/pak/chunk_3000.bin` and the lookup table to `game/src/sfxdata.rs`.
The bank is packed into WORLD.PAK on the disc and streamed into SPU RAM at
boot, so it never occupies main memory.

From Kenney.nl (https://kenney.nl), CC0:

* Impact Sounds: footsteps on grass, concrete, snow and wood; mining and
  generic impacts; a plank crack; a punch. These become `step_*`, `dig_soft`,
  `dig_stone`, `break`, `place` and `hurt`.
* RPG Audio: `dig_wood` (chop), `door` (doorOpen_1), `chest` (metalLatch).
* Interface Sounds: `click`, `confirm`.

From freesound.org, all CC0:

| sound | source |
| --- | --- |
| `pig` | "Pig Oink" by qubodup, freesound.org/s/442906/ |
| `cow` | "z-moo01" by Zozzy, freesound.org/s/59245/ |
| `sheep` | "sqeeeek_sheep" by sqeeeek, freesound.org/s/237103/ |
| `chicken` | "Chicken clucking 3" by MBPL, freesound.org/s/668803/ |
| `zombie` | "Zombie Groan 0" by OwNathan, freesound.org/s/754438/ |
| `bones` | "bones" by Kneeling, freesound.org/s/473526/ (skeleton and spider) |
| `bark` | "Short Dog Bark" by qubodup, freesound.org/s/827323/ |
| `hiss` | "hiss" by Reitanna, freesound.org/s/241562/ (sapper fuse) |
| `splash` | "Splash" by swordofkings128, freesound.org/s/398032/ |
| `eat` | "Crunch" by qubodup, freesound.org/s/816237/ |
| `explode` | "Explosion, short" by qubodup, freesound.org/s/171971/ |

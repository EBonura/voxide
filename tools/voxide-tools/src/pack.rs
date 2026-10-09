//! Convert the CC0 "16x16 Block Texture Set" (OpenGameArt, CC0, no attribution
//! required) into per-tile 16-colour CLUTs and index maps for VoXide's 4bpp
//! atlas. Emits `game/src/texdata.rs`. Tiles the pack lacks stay on the
//! procedural path.

use crate::png;
use std::fmt::Write as _;
use std::path::Path;

/// Number of atlas tiles.
pub const NTILES: usize = 32;

/// How a tile's palette is remapped after quantising.
#[derive(Clone, Copy)]
pub enum Recolor {
    /// Darker, warmer oak-leaf green.
    Leaves,
    /// Pale cream sand.
    Sand,
    /// Ore nuggets turned to coal.
    Coal,
    /// Ore nuggets turned to iron.
    Iron,
}

/// One tile's source: pack file name, background to composite onto, recolor.
struct Source {
    tile: usize,
    file: &'static str,
    background: Option<[u8; 3]>,
    recolor: Option<Recolor>,
}

const fn src(
    tile: usize,
    file: &'static str,
    background: Option<[u8; 3]>,
    recolor: Option<Recolor>,
) -> Source {
    Source {
        tile,
        file,
        background,
        recolor,
    }
}

/// Tile id to pack file.
const MAP: [Source; 17] = [
    src(0, "grass_top", None, None),
    src(1, "grass_side", None, None),
    src(2, "dirt", None, None),
    src(3, "stone_generic", None, None),
    src(4, "oak_log_top", None, None),
    src(5, "oak_log_side", None, None),
    src(6, "oak_leaves", Some([32, 56, 26]), Some(Recolor::Leaves)),
    src(7, "sand_ugly_2", None, Some(Recolor::Sand)),
    src(9, "stone_generic_ore_nuggets", None, Some(Recolor::Coal)),
    src(10, "stone_generic_ore_nuggets", None, Some(Recolor::Iron)),
    src(11, "stone_generic_ore_nuggets", None, None),
    src(12, "stone_generic_ore_crystalline", None, None),
    src(14, "snow", None, None),
    src(15, "grass_snowy_side", None, None),
    src(16, "cobblestone", None, None),
    src(17, "oak_planks", None, None),
    src(20, "mud_bricks", None, None),
];

/// A 16-entry palette and the 256 palette indices of a 16x16 tile.
pub type Tile = ([[u8; 3]; 16], [u8; 256]);

/// Weights the median cut uses to pick the axis to split: the box's range on
/// each channel is scaled by these (the 8-bit luma weights, summing to 256).
const AXIS_WEIGHT: [u32; 3] = [77, 150, 29];

/// One entry of a box: a distinct colour and how many pixels have it.
type Counted = ([u8; 3], u32);

/// Median cut as the pack's palettes were originally made. A box holds the
/// distinct colours it covers with their pixel counts. It splits along the
/// channel whose range, scaled by `AXIS_WEIGHT`, is greatest (the first on a
/// tie). Colours sort by that channel; runs of equal value stay together; the
/// run holding the median pixel starts the upper half (unless it is first,
/// when it ends the lower half). The upper half is emitted before the lower.
fn cut(mut colors: Vec<Counted>, out: &mut Vec<[u8; 3]>) {
    if colors.len() < 2 {
        let (color, _) = colors[0];
        out.push(color);
        return;
    }
    let mut axis = 0;
    let mut best = 0;
    for (a, weight) in AXIS_WEIGHT.iter().enumerate() {
        let lo = colors.iter().map(|c| c.0[a]).min().unwrap();
        let hi = colors.iter().map(|c| c.0[a]).max().unwrap();
        let weighted = u32::from(hi - lo) * weight;
        if weighted > best {
            best = weighted;
            axis = a;
        }
    }
    colors.sort_by_key(|c| c.0[axis]);
    let total: u32 = colors.iter().map(|c| c.1).sum();
    let mut seen = 0;
    let mut split = 0;
    let mut i = 0;
    while i < colors.len() {
        let value = colors[i].0[axis];
        let mut j = i;
        while j < colors.len() && colors[j].0[axis] == value {
            seen += colors[j].1;
            j += 1;
        }
        if seen * 2 >= total {
            split = if i == 0 { j } else { i };
            break;
        }
        i = j;
    }
    let upper = colors.split_off(split);
    cut(upper, out);
    cut(colors, out);
}

/// Quantise RGB pixels to at most 16 colours, returning the palette in the
/// order the original tool produced it and each pixel's palette index. Fails
/// when the image has more than 16 distinct colours (the pack has none).
pub fn quantize(pixels: &[[u8; 3]]) -> Result<(Vec<[u8; 3]>, Vec<u8>), String> {
    let mut colors: Vec<Counted> = Vec::new();
    for &p in pixels {
        match colors.iter_mut().find(|c| c.0 == p) {
            Some(c) => c.1 += 1,
            None => colors.push((p, 1)),
        }
    }
    if colors.len() > 16 {
        return Err(format!(
            "{} distinct colours: only images with at most 16 are supported",
            colors.len()
        ));
    }
    let mut palette = Vec::new();
    cut(colors, &mut palette);
    let indices = pixels
        .iter()
        .map(|p| palette.iter().position(|c| c == p).unwrap() as u8)
        .collect();
    Ok((palette, indices))
}

/// Python-style floor division by a positive divisor.
fn floor_div(a: i32, b: i32) -> i32 {
    a.div_euclid(b)
}

fn recolor(palette: &mut [[u8; 3]; 16], mode: Recolor) {
    for entry in palette.iter_mut() {
        let [r, g, b] = entry.map(i32::from);
        let lum = (r + g + b) / 3;
        let out = match mode {
            // Remap to Minecraft's dark warm oak-leaf green (~57,84,34): the
            // pack's leaves quantised to a washed-out cyan-green, reading
            // harsh and pale next to the plains reference.
            Recolor::Leaves => {
                let lum = 58 + floor_div(lum - 58, 2);
                [
                    (57 * lum / 58).min(255),
                    (84 * lum / 58).min(255),
                    (34 * lum / 58).min(255),
                ]
            }
            // Remap the whole palette to a pale-cream sand (base ~219,207,163),
            // scaled by relative luminance with the contrast eased.
            Recolor::Sand => {
                let lum = 171 + floor_div((lum - 171) * 2, 3);
                [
                    (219 * lum / 171).min(255),
                    (207 * lum / 171).min(255),
                    (163 * lum / 171).min(255),
                ]
            }
            // Swap the saturated (ore-nugget) entries to a new colour, keep greys.
            Recolor::Coal | Recolor::Iron => {
                let saturation = r.max(g).max(b) - r.min(g).min(b);
                if saturation > 34 && r >= b {
                    match mode {
                        Recolor::Coal => [(lum - 70).max(0), (lum - 66).max(0), (lum - 60).max(0)],
                        _ => [
                            (lum + 40).min(255),
                            (f64::from(lum) * 0.82) as i32,
                            (f64::from(lum) * 0.6) as i32,
                        ],
                    }
                } else {
                    [r, g, b]
                }
            }
        };
        *entry = out.map(|v| v as u8);
    }
}

/// Quantise one tile image (RGBA, 16x16) to its palette and index map,
/// compositing transparent pixels onto `background` first.
pub fn convert_tile(
    image: &png::Image,
    background: Option<[u8; 3]>,
    mode: Option<Recolor>,
) -> Result<Tile, String> {
    if image.width != 16 || image.height != 16 {
        return Err(format!(
            "{}x{} tile, expected 16x16",
            image.width, image.height
        ));
    }
    let mut pixels = Vec::with_capacity(256);
    for &[r, g, b, a] in &image.rgba {
        pixels.push(match (a, background) {
            (255, _) => [r, g, b],
            (0, Some(bg)) => bg,
            _ => return Err("partly transparent pixels are not supported".into()),
        });
    }
    let (palette, indices) = quantize(&pixels)?;
    let mut padded = [[0u8; 3]; 16];
    padded[..palette.len()].copy_from_slice(&palette);
    if let Some(mode) = mode {
        recolor(&mut padded, mode);
    }
    let mut map = [0u8; 256];
    map.copy_from_slice(&indices);
    Ok((padded, map))
}

/// Render `texdata.rs` for the given per-tile results.
pub fn render(tiles: &[Option<Tile>; NTILES]) -> String {
    let mut out = String::new();
    out.push_str("// GENERATED by tools/voxide-tools (convert-pack) -- do not edit.\n");
    out.push_str("// Source: OpenGameArt \"16x16 Block Texture Set\", CC0 (public domain, no\n");
    out.push_str(
        "// attribution required). Quantised to per-tile 16-colour CLUTs for the 4bpp atlas.\n\n",
    );
    out.push_str("pub const PACK_HAS: [bool; 32] = [");
    let has: Vec<&str> = tiles
        .iter()
        .map(|t| if t.is_some() { "true" } else { "false" })
        .collect();
    out.push_str(&has.join(","));
    out.push_str("];\n\n");
    out.push_str("pub const PACK_PAL: [[(u8,u8,u8); 16]; 32] = [\n");
    for tile in tiles {
        let entries: Vec<String> = match tile {
            Some((pal, _)) => pal
                .iter()
                .map(|[r, g, b]| format!("({r},{g},{b})"))
                .collect(),
            None => vec!["(0,0,0)".to_string(); 16],
        };
        let _ = writeln!(out, "  [{}],", entries.join(","));
    }
    out.push_str("];\n\n");
    out.push_str("pub const PACK_IDX: [[u8; 256]; 32] = [\n");
    for tile in tiles {
        let entries: Vec<String> = match tile {
            Some((_, idx)) => idx.iter().map(u8::to_string).collect(),
            None => vec!["0".to_string(); 256],
        };
        let _ = writeln!(out, "  [{}],", entries.join(","));
    }
    out.push_str("];\n");
    out
}

/// Convert every mapped tile under `root` and write `game/src/texdata.rs`.
pub fn run(root: &Path) -> Result<(), String> {
    let src_dir = root.join("assets/pack/blocks/blocks");
    let mut tiles: [Option<Tile>; NTILES] = std::array::from_fn(|_| None);
    for source in &MAP {
        let path = src_dir.join(format!("{}.png", source.file));
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let image = png::decode(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        tiles[source.tile] = Some(
            convert_tile(&image, source.background, source.recolor)
                .map_err(|e| format!("{}: {e}", path.display()))?,
        );
        println!("tile {:2} <- {}", source.tile, source.file);
    }
    let out = root.join("game/src/texdata.rs");
    std::fs::write(&out, render(&tiles)).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("wrote game/src/texdata.rs");
    Ok(())
}

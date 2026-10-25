//! The loading screen: a darkened dirt backdrop, a slowly turning grass block,
//! two status lines that name the work in progress, and a thin progress bar.
//!
//! Everything comes from VoXide's own block atlas (`tex.rs`), so no outside art
//! is involved. It runs between world-generation steps with no ordering table
//! in flight, so every primitive goes to the GPU immediately.
//!
//! The bar is driven by the world's real backlog and the stage line by the
//! ring's real state (`world::load_stage`). Only the block's turn reads the
//! clock, and it is drawn at the flips the progress callbacks already make, so
//! it never costs a flip of its own.

use psx_font::FontAtlas;
use psx_gpu::{
    self as gpu,
    material::{BlendMode, TextureMaterial, TextureWindow},
    prim::GpuPacket,
};
use psx_math::sincos;

use crate::tex::{self, BlockTex};
use crate::world::LoadStage;

/// What the loading screen is for, which sets its first line. The caller sets
/// it before the work starts (`set_kind`), as the callbacks that report
/// progress carry only counts.
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Kind {
    /// Boot and NEW WORLD: the ring is built from nothing.
    Generate,
    /// A saved world is read back.
    Load,
    /// A portal into the Inferno.
    Inferno,
    /// A portal into the Void.
    Void,
    /// The way home from either dimension, or back to the title.
    Overworld,
}

static mut KIND: Kind = Kind::Generate;
/// Frames still to paint in full. The two display buffers each need the whole
/// backdrop once; after that only the rectangles that change are repainted.
static mut FULL_PAINTS: u8 = 2;

/// Start a loading screen: choose its first line, and have the next two frames
/// paint the whole backdrop (one per display buffer).
pub fn set_kind(kind: Kind) {
    // SAFETY: single-threaded guest; read only by `draw` between generation steps.
    unsafe {
        KIND = kind;
        FULL_PAINTS = 2;
        FADE_ON = false;
        FADE_ARMED = false;
    }
}

const SCREEN_W: i16 = 320;

/// Dirt tile brightness: the 0x80 neutral tint scaled down, as Minecraft's
/// backdrop is its dirt texture at a fraction of full brightness.
const BACKDROP_TINT: u8 = 0x30;

/// Block half-edge in pixels at the centre of the screen.
const R: i32 = 26;
/// View pitch: the block is seen from 30 degrees above, as its inventory icon is.
const SIN_PITCH: i32 = 2048;
const COS_PITCH: i32 = 3547;
/// Where the block sits.
const BLOCK_CX: i32 = 160;
const BLOCK_CY: i32 = 82;

/// Spin speed in Q12 turns per vblank (about five seconds a turn).
const SPIN_PER_VBLANK: u32 = 14;

const BAR_W: i16 = 160;
const BAR_H: i16 = 8;

fn tile_material(bt: &BlockTex, tile: u8, tint: u8) -> TextureMaterial {
    let (u, v) = tex::tile_uv(tile);
    TextureMaterial::opaque(bt.clut[tile as usize], bt.tpage, (tint, tint, tint))
        .with_texture_window(TextureWindow::power_of_two_tile(u, v, 16, 16))
}

/// A rectangle of the screen that changes from frame to frame: its corner (a
/// multiple of the tile size, so the dirt lines up with the full-screen paint)
/// and its size.
struct Dirty {
    x: i16,
    y: i16,
    w: i16,
    h: i16,
}

/// The block, its shadow and its bob.
const BLOCK_RECT: Dirty = Dirty {
    x: 112,
    y: 32,
    w: 96,
    h: 96,
};
/// The two text lines and the bar.
const TEXT_RECT: Dirty = Dirty {
    x: 48,
    y: 128,
    w: 224,
    h: 48,
};

/// Dirt over `r`: one textured quad whose texture window wraps the 16x16 tile,
/// so the repeat costs no more than a single tile would. One texel is one
/// pixel, so the two triangles of a quad cannot disagree on a texel.
#[optimize(size)]
fn dirt(mat: TextureMaterial, r: &Dirty) {
    gpu::draw_quad_textured_material(
        [
            (r.x, r.y),
            (r.x + r.w, r.y),
            (r.x, r.y + r.h),
            (r.x + r.w, r.y + r.h),
        ],
        [
            (0, 0),
            (r.w as u8, 0),
            (0, r.h as u8),
            (r.w as u8, r.h as u8),
        ],
        mat,
    );
}

/// Repaint the dirt: the whole screen for the first frame in each display
/// buffer, after that only the rectangles the block and the text change. The
/// GPU fills about a quarter of the pixels, which keeps this screen from
/// slowing the load it sits over.
#[optimize(size)]
fn draw_backdrop(bt: &BlockTex) {
    let mat = tile_material(bt, tex::T_DIRT, BACKDROP_TINT);
    // SAFETY: single-threaded guest; see `set_kind`.
    let full = unsafe { FULL_PAINTS > 0 };
    if full {
        unsafe { FULL_PAINTS -= 1 };
        // Two halves: a quad's texture coordinates are bytes, and 160 is a
        // whole number of tiles.
        dirt(
            mat,
            &Dirty {
                x: 0,
                y: 0,
                w: 160,
                h: 240,
            },
        );
        dirt(
            mat,
            &Dirty {
                x: 160,
                y: 0,
                w: 160,
                h: 240,
            },
        );
    } else {
        dirt(mat, &BLOCK_RECT);
        dirt(mat, &TEXT_RECT);
    }
}

/// One face of the block, in object space: its four corners in the order the
/// GPU takes a quad (top-left, top-right, bottom-left, bottom-right as seen
/// from outside) and its outward normal.
struct Face {
    corners: [(i8, i8, i8); 4],
    normal: (i8, i8, i8),
    tile: u8,
}

const FACES: [Face; 5] = [
    Face {
        corners: [(-1, 1, -1), (1, 1, -1), (-1, 1, 1), (1, 1, 1)],
        normal: (0, 1, 0),
        tile: tex::T_GRASS_TOP,
    },
    Face {
        corners: [(-1, 1, 1), (1, 1, 1), (-1, -1, 1), (1, -1, 1)],
        normal: (0, 0, 1),
        tile: tex::T_GRASS_SIDE,
    },
    Face {
        corners: [(1, 1, -1), (-1, 1, -1), (1, -1, -1), (-1, -1, -1)],
        normal: (0, 0, -1),
        tile: tex::T_GRASS_SIDE,
    },
    Face {
        corners: [(1, 1, 1), (1, 1, -1), (1, -1, 1), (1, -1, -1)],
        normal: (1, 0, 0),
        tile: tex::T_GRASS_SIDE,
    },
    Face {
        corners: [(-1, 1, -1), (-1, 1, 1), (-1, -1, -1), (-1, -1, 1)],
        normal: (-1, 0, 0),
        tile: tex::T_GRASS_SIDE,
    },
];

/// Turn `p` about the vertical axis (`s` and `c` are the sine and cosine of
/// the turn, Q12) and project it flat: no perspective, so the GPU's affine
/// texture mapping is exact. Returns the screen position.
#[inline]
fn project(p: (i8, i8, i8), s: i32, c: i32, cx: i32, cy: i32) -> (i16, i16) {
    let (x, y, z) = (p.0 as i32, p.1 as i32, p.2 as i32);
    let xr = (x * c + z * s) * R >> 12;
    let zr = (z * c - x * s) * R >> 12;
    let sx = cx + xr;
    let sy = cy - ((y * R * COS_PITCH) >> 12) + ((zr * SIN_PITCH) >> 12);
    (sx as i16, sy as i16)
}

/// The brightness of a side by how far it turns toward the light: 0x58 facing
/// right to 0x80 facing left. The top is always full.
#[inline]
fn side_shade(nx_turned: i32) -> u8 {
    (0x6C - ((nx_turned * 0x14) >> 12)) as u8
}

/// The grass block at `angle` (Q12 turns), `bob` pixels off its rest height.
#[optimize(size)]
fn draw_block(bt: &BlockTex, angle: u16, bob: i32) {
    let s = sincos::sin_q12(angle);
    let c = sincos::cos_q12(angle);
    let cy = BLOCK_CY + bob;
    // A flat shadow on the dirt under the block, so it reads as resting there.
    draw_shadow(s, c, BLOCK_CY + 4 + ((R * COS_PITCH) >> 12));
    let mut i = 0;
    while i < FACES.len() {
        let f = &FACES[i];
        let (nx, ny, nz) = (f.normal.0 as i32, f.normal.1 as i32, f.normal.2 as i32);
        let nx_t = nx * c + nz * s; // the turned normal, Q12
        let nz_t = nz * c - nx * s;
        // The top is always seen from above; a side shows when it turns toward the viewer.
        if ny == 0 && nz_t <= 0 {
            i += 1;
            continue;
        }
        let shade = if ny == 1 { 0x80 } else { side_shade(nx_t) };
        let v = [
            project(f.corners[0], s, c, BLOCK_CX, cy),
            project(f.corners[1], s, c, BLOCK_CX, cy),
            project(f.corners[2], s, c, BLOCK_CX, cy),
            project(f.corners[3], s, c, BLOCK_CX, cy),
        ];
        // 0..15, not 0..16: the window wraps, and the far edge would sample the
        // opposite edge of the tile as a one-pixel rim.
        gpu::draw_quad_textured_material(
            v,
            [(0, 0), (15, 0), (0, 15), (15, 15)],
            tile_material(bt, f.tile, shade),
        );
        i += 1;
    }
}

/// A dark square on the ground under the block, turning with it.
#[optimize(size)]
fn draw_shadow(s: i32, c: i32, y: i32) {
    let r = R;
    let p = |x: i32, z: i32| {
        let xr = (x * c + z * s) >> 12;
        let zr = (z * c - x * s) >> 12;
        (
            (BLOCK_CX + xr) as i16,
            (y + ((zr * SIN_PITCH) >> 12)) as i16,
        )
    };
    gpu::draw_quad_flat([p(-r, -r), p(r, -r), p(-r, r), p(r, r)], 0x12, 0x0E, 0x0A);
}

/// Black frame, dark track, green fill with a lit top edge and a shaded bottom
/// edge: Minecraft's bar, thin and bright against the dark backdrop.
#[optimize(size)]
fn draw_bar(x: i16, y: i16, filled: i16) {
    gpu::draw_rect_flat(
        x - 1,
        y - 1,
        (BAR_W + 2) as u16,
        (BAR_H + 2) as u16,
        0,
        0,
        0,
    );
    gpu::draw_rect_flat(x, y, BAR_W as u16, BAR_H as u16, 0x2C, 0x2C, 0x2C);
    if filled > 0 {
        gpu::draw_rect_flat(x, y, filled as u16, BAR_H as u16, 0x4A, 0xB8, 0x2E);
        gpu::draw_rect_flat(x, y, filled as u16, 2, 0xB4, 0xFF, 0x8C);
        gpu::draw_rect_flat(x, y + BAR_H - 2, filled as u16, 2, 0x2C, 0x78, 0x1C);
    }
}

/// Text with the one-pixel dark drop shadow the game's menus use, centred on
/// the screen.
#[optimize(size)]
fn centred(font: &FontAtlas, y: i16, text: &str, color: (u8, u8, u8)) {
    let x = SCREEN_W / 2 - (font.text_width(text) / 2) as i16;
    font.draw_text(x + 1, y + 1, text, (0x10, 0x10, 0x10));
    font.draw_text(x, y, text, color);
}

fn title(kind: Kind) -> &'static str {
    match kind {
        Kind::Generate => "GENERATING WORLD",
        Kind::Load => "LOADING WORLD",
        Kind::Inferno => "ENTERING THE INFERNO",
        Kind::Void => "ENTERING THE VOID",
        Kind::Overworld => "RETURNING TO THE OVERWORLD",
    }
}

fn stage_line(stage: LoadStage) -> &'static str {
    match stage {
        LoadStage::Terrain => "SHAPING TERRAIN",
        LoadStage::Meshes => "BUILDING TERRAIN",
    }
}

/// Back to sampling the whole texture page (GP0 E2), as every other screen
/// expects: the block and backdrop quads leave their tile window set.
fn release_window() {
    // SAFETY: the GPU is driven from this one thread, and no ordering table is in flight.
    let mut dma = unsafe { psx_io::periph::GpuDma::steal() };
    psx_gpu::Gpu::from_dma_mut(&mut dma).set_texture_window(TextureWindow::NONE);
}

/// Draw one frame of the loading screen at `done` of `total` steps, the world
/// being in `stage`, at vblank `tick`. The caller flips.
#[optimize(size)] // a load, not a gameplay frame: its bytes are worth more than its cycles
#[inline(never)]
pub fn draw(
    font: &FontAtlas,
    bt: &BlockTex,
    done: usize,
    total: usize,
    stage: LoadStage,
    tick: u32,
) {
    draw_backdrop(bt);
    let angle = (tick.wrapping_mul(SPIN_PER_VBLANK) & 0x0FFF) as u16;
    let bob = (sincos::sin_q12((tick.wrapping_mul(9) & 0x0FFF) as u16) * 2) >> 12;
    draw_block(bt, angle, bob);

    let total = total.max(1);
    let done = done.min(total);
    centred(font, 132, title(unsafe { KIND }), (0x80, 0x80, 0x80));
    centred(font, 146, stage_line(stage), (0x5A, 0x5A, 0x5A));
    let x = SCREEN_W / 2 - BAR_W / 2;
    draw_bar(x, 164, (done * BAR_W as usize / total) as i16);
    release_window();
}

// ---- the fade in after a load ------------------------------------------------

/// Vblanks the picture takes to come up out of black once a load is over: 0.4 s,
/// whatever frame rate the screen after the load runs at.
const FADE_VBLANKS: u32 = 24;
/// A finished load arms the fade; it starts with the first frame drawn after
/// the load. The loading screen can stay up for a while after its last flip
/// (the save is still being applied), and the picture should come up when it
/// replaces the screen, not before.
static mut FADE_ARMED: bool = false;
/// Vblank count when the running fade began, and whether one is running.
static mut FADE_START: u32 = 0;
static mut FADE_ON: bool = false;

/// A load just finished: the next frames of whatever follows (the title or the
/// world) come up from black instead of cutting in from the dirt screen.
pub fn begin_fade() {
    // SAFETY: single-threaded guest.
    unsafe { FADE_ARMED = true };
}

/// The fade-in's packet for the frame being built at vblank `now`, or `None`
/// once it is over.
#[optimize(size)]
pub fn fade_packet(now: u32) -> Option<FadePacket> {
    // SAFETY: single-threaded guest.
    unsafe {
        if FADE_ARMED {
            FADE_ARMED = false;
            FADE_START = now;
            FADE_ON = true;
        }
        if !FADE_ON {
            return None;
        }
        let elapsed = now.wrapping_sub(FADE_START);
        if elapsed >= FADE_VBLANKS {
            FADE_ON = false;
            return None;
        }
        // The GPU subtracts 5 bits a channel: 31 is black, 0 is the picture as drawn.
        let step = ((FADE_VBLANKS - elapsed) * 31 / FADE_VBLANKS) << 3;
        Some(FadePacket {
            tag: 0,
            draw_mode: TextureMaterial::blended(0, 0, (0, 0, 0), BlendMode::Subtract)
                .draw_mode_word(),
            // GP0 62h: semi-transparent flat rectangle, grey `step`.
            colour: 0x6200_0000 | step * 0x0001_0101,
            origin: 0,
            size: (240 << 16) | 320,
        })
    }
}

/// A full-screen darkening rectangle, laid out like the SDK's brightness
/// overlay: a draw-mode word (B - F) and a semi-transparent grey rectangle.
#[derive(Copy, Clone)]
#[repr(C, align(4))]
pub struct FadePacket {
    tag: u32,
    draw_mode: u32,
    colour: u32,
    origin: u32,
    size: u32,
}

impl FadePacket {
    /// The payload words, for copying into the frame's UI list.
    pub fn words(&self) -> [u32; 4] {
        [self.draw_mode, self.colour, self.origin, self.size]
    }
}

// SAFETY: `repr(C, align(4))`, a `u32` tag first, then four words of plain
// `u32` data, no interior mutability (the layout of the SDK's own overlay).
unsafe impl GpuPacket for FadePacket {
    const WORDS: u8 = 4;
}

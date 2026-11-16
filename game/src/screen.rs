//! The two screen buffers, presented through the SDK's one present path (`psx_rt::present`).
//!
//! A frame is either drawn through the GP0 port or a blocking DMA walk (menus, loading screens:
//! `swap`) or built as an ordering-table chain and handed over (gameplay: `submit_chain`). Either
//! way the presenter flips it at the first VBlank after the GPU has finished it. The handle is
//! zero-sized: the buffers and the presenter are statics, so a screen without the handle in reach
//! (`settle_gpu`) can still take the display back with `Screen::resume`.

use psx_gpu::display::{DoubleBuffer, Resolution};
use psx_rt::present::{self, Part, PresentCap, Presenter};

static mut BUFFERS: DoubleBuffer = DoubleBuffer::new(Resolution::R320X240);
static mut PRESENTER: Option<Presenter> = None;

/// Handle to the presented screen.
pub struct Screen;

#[inline(always)]
fn presenter() -> &'static mut Presenter {
    // SAFETY: single-threaded guest; `Screen::new` ran before any use.
    unsafe {
        (*core::ptr::addr_of_mut!(PRESENTER))
            .as_mut()
            .unwrap_unchecked()
    }
}

#[inline(always)]
fn buffers() -> &'static mut DoubleBuffer {
    // SAFETY: single-threaded guest.
    unsafe { &mut *core::ptr::addr_of_mut!(BUFFERS) }
}

impl Screen {
    /// Take over presentation. Call after `install_vblank_counter`, with the GPU idle. The game
    /// presents at the display rate when it can (the 30 fps cap would halve it at short draw
    /// distances), so the cap is lifted.
    pub fn new() -> Self {
        present::set_cap(PresentCap::UNCAPPED);
        unsafe { *core::ptr::addr_of_mut!(PRESENTER) = Some(Presenter::new(None)) };
        presenter().begin_frame(buffers());
        Screen
    }

    /// Follow a splash screen's buffers: it flipped through its own driver, so the draw side is
    /// whichever it ended on.
    pub fn follow(&mut self, splash: DoubleBuffer) {
        *buffers() = splash;
        presenter().begin_frame(buffers());
    }

    /// Present the frame drawn into the draw side through the GP0 port or a blocking walk, wait
    /// until it is on screen and point the draw area at the other side.
    #[inline(never)]
    pub fn swap(&mut self) {
        presenter().present_drawn(buffers());
        presenter().begin_frame(buffers());
    }

    /// Hand over a frame built as an ordering-table chain that ends on the draw-done node.
    ///
    /// # Safety
    ///
    /// As `Presenter::submit` with `Part::Final`.
    #[inline(never)]
    pub unsafe fn submit_chain(&mut self, head: *const u32) {
        // SAFETY: forwarded contract.
        unsafe { presenter().submit(buffers(), head, Part::Final) };
    }

    /// Wait until no chain is in flight and point the draw area at the draw side: before a
    /// screen that draws through the GP0 port follows gameplay frames.
    #[inline(never)]
    pub fn resume() {
        present::wait_idle();
        presenter().begin_frame(buffers());
    }
}

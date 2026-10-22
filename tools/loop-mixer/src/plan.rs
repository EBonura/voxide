//! Which loops, how many times, in what order: the sequence of every piece.

use crate::rng::Rng;

/// One run of the same loop: loop `loop_index`, `repeats` whole laps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    /// Index into the source list.
    pub loop_index: usize,
    /// Laps, 1 to 3.
    pub repeats: u32,
}

/// A piece: segments in order.
pub type Piece = Vec<Segment>;

/// Laps in a piece. Three laps of loops that run 50 to 60 seconds is the two
/// and a half to three minutes a piece is meant to last.
pub const LAPS: u32 = 3;

/// A shape for the laps: `[1,1,1]` three loops of one lap, `[2,1]` a loop twice
/// then another once, and so on. Every shape has at least one crossfade, and
/// most pieces use the first so that crossfades are what the set consists of.
fn shape(rng: &mut Rng) -> &'static [u32] {
    match rng.below(10) {
        0..=5 => &[1, 1, 1],
        6..=7 => &[2, 1],
        _ => &[1, 2],
    }
}

/// A random piece: no loop appears twice in it, and its laps add up to at most
/// `max_seconds` (loop lengths in `lengths`, seconds).
pub fn piece(rng: &mut Rng, lengths: &[f64], max_seconds: f64) -> Piece {
    loop {
        let mut out: Piece = Vec::new();
        for &repeats in shape(rng) {
            let mut l = rng.below(lengths.len());
            while out.iter().any(|s| s.loop_index == l) {
                l = rng.below(lengths.len());
            }
            out.push(Segment { loop_index: l, repeats });
        }
        if seconds(&out, lengths) <= max_seconds {
            return out;
        }
    }
}

/// The laps' total length.
pub fn seconds(p: &Piece, lengths: &[f64]) -> f64 {
    p.iter().map(|s| s.repeats as f64 * lengths[s.loop_index]).sum()
}

/// `count` pieces from `seed`: every loop is used somewhere, no two pieces are
/// the same sequence. The set is drawn again (from the same generator, so the
/// seed still names it) until both hold.
pub fn set(seed: u64, lengths: &[f64], count: usize, max_seconds: f64) -> Vec<Piece> {
    let mut rng = Rng::new(seed);
    loop {
        let pieces: Vec<Piece> = (0..count).map(|_| piece(&mut rng, lengths, max_seconds)).collect();
        let all_used = (0..lengths.len()).all(|l| pieces.iter().flatten().any(|s| s.loop_index == l));
        let distinct = pieces.iter().enumerate().all(|(i, p)| !pieces[..i].contains(p));
        if all_used && distinct {
            return pieces;
        }
    }
}

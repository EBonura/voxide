//! Render one piece: the laps, the crossfades, the fades at the ends.

use crate::loudness;
use crate::plan::Piece;
use crate::resample::resample;
use crate::rng::Rng;
use crate::wav::Audio;
use std::f64::consts::FRAC_PI_2;

/// Seconds of equal-power crossfade, centred on the boundary between loops.
pub const CROSSFADE_SECONDS: f64 = 4.0;
/// Seconds of fade-in at the start of a piece.
pub const FADE_IN_SECONDS: f64 = 1.5;
/// Seconds of fade-out at the end of a piece.
pub const FADE_OUT_SECONDS: f64 = 5.0;
/// Sample rate of the rendered pieces: 37.8 kHz, the XA-ADPCM rate.
pub const OUT_RATE: u32 = 37_800;
/// Integrated loudness the pieces are turned down to, in LUFS (EBU R128 and
/// ITU-R BS.1770). A piece already quieter is left alone.
pub const TARGET_LUFS: f64 = -16.0;
/// True peak the pieces are kept under, in dBTP.
pub const TRUE_PEAK_LIMIT_DB: f64 = -1.7;

/// A rendered piece.
pub struct Rendered {
    /// 16-bit left and right at [`OUT_RATE`].
    pub left: Vec<i16>,
    /// 16-bit right channel.
    pub right: Vec<i16>,
    /// Integrated loudness before any gain, LUFS.
    pub lufs: f64,
    /// True peak before any gain, 1.0 is full scale.
    pub peak: f64,
    /// The gain applied (1.0 or less: loudness first, then the true peak).
    pub gain: f64,
    /// True peak after the gain, dBTP.
    pub peak_db: f64,
}

/// Mix `piece` from `loops` (all at one rate) at that rate, as two channels.
pub fn mix(piece: &Piece, loops: &[Audio]) -> [Vec<f64>; 2] {
    let rate = loops[0].rate as f64;
    let x = (CROSSFADE_SECONDS * rate) as i64;
    let total: i64 = piece
        .iter()
        .map(|s| s.repeats as i64 * loops[s.loop_index].ch[0].len() as i64)
        .sum();
    let mut out = [vec![0.0; total as usize], vec![0.0; total as usize]];
    let mut start = 0i64;
    for (i, seg) in piece.iter().enumerate() {
        let src = &loops[seg.loop_index];
        let len = src.ch[0].len() as i64;
        let end = start + seg.repeats as i64 * len;
        let first = i == 0;
        let last = i == piece.len() - 1;
        let from = if first { start } else { start - x / 2 };
        let to = if last { end } else { end + x / 2 };
        for t in from.max(0)..to.min(total + x) {
            if t >= total {
                break;
            }
            let mut g = 1.0;
            if !first && t < start + x / 2 {
                g *= (((t - (start - x / 2)) as f64 / x as f64) * FRAC_PI_2).sin();
            }
            if !last && t >= end - x / 2 {
                g *= (((t - (end - x / 2)) as f64 / x as f64) * FRAC_PI_2).cos();
            }
            let pos = (t - start).rem_euclid(len) as usize;
            out[0][t as usize] += g * src.ch[0][pos];
            out[1][t as usize] += g * src.ch[1][pos];
        }
        start = end;
    }
    let fade_in = (FADE_IN_SECONDS * rate) as usize;
    let fade_out = (FADE_OUT_SECONDS * rate) as usize;
    let n = total as usize;
    for c in &mut out {
        for t in 0..fade_in.min(n) {
            c[t] *= (t as f64 / fade_in as f64 * FRAC_PI_2).sin();
        }
        for t in 0..fade_out.min(n) {
            c[n - 1 - t] *= (t as f64 / fade_out as f64 * FRAC_PI_2).sin();
        }
    }
    out
}

/// Mix, resample and dither one piece.
pub fn render(piece: &Piece, loops: &[Audio], dither: &mut Rng) -> Rendered {
    let rate = loops[0].rate;
    let mixed = mix(piece, loops);
    let mut ch: [Vec<f64>; 2] = [resample(&mixed[0], rate, OUT_RATE), resample(&mixed[1], rate, OUT_RATE)];
    let refs = [&ch[0][..], &ch[1][..]];
    let lufs = loudness::integrated_lufs(refs, OUT_RATE);
    let peak = loudness::true_peak(refs, OUT_RATE);
    let mut gain = if lufs > TARGET_LUFS { 10f64.powf((TARGET_LUFS - lufs) / 20.0) } else { 1.0 };
    let limit = 10f64.powf(TRUE_PEAK_LIMIT_DB / 20.0);
    if peak * gain > limit {
        gain = limit / peak;
    }
    let peak_db = 20.0 * (peak * gain).log10();
    let mut quantise = |c: &mut Vec<f64>| -> Vec<i16> {
        c.iter()
            .map(|v| {
                // Triangular dither, one least-significant bit each side.
                let d = dither.unit() - dither.unit();
                (v * gain * 32768.0 + d).round().clamp(-32768.0, 32767.0) as i16
            })
            .collect()
    };
    let left = quantise(&mut ch[0]);
    let right = quantise(&mut ch[1]);
    Rendered { left, right, lufs, peak, gain, peak_db }
}

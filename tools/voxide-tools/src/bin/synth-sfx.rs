//! Synthesise the sounds that have no recording to draw on: the bow's twang and
//! the thud of an arrow landing. Deterministic (a fixed noise generator), mono
//! 22,050 Hz 16-bit, written to `assets/sfx/` for `convert-sfx` to cook. The
//! sounds are the project's own, so they carry no credit.
//!
//!   cargo run --release --bin synth-sfx

use std::f64::consts::PI;
use std::path::Path;

const RATE: u32 = 22_050;

/// A fixed linear-congruential noise source in -1..1.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f64 / (1u32 << 23) as f64 - 1.0
    }
}

fn wav(samples: &[f64]) -> Vec<u8> {
    let data: Vec<u8> = samples
        .iter()
        .flat_map(|&s| ((s.clamp(-1.0, 1.0) * 32_000.0) as i16).to_le_bytes())
        .collect();
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

/// A bowstring let go: a plucked string (Karplus-Strong at 170 Hz, a little
/// damped) with the sharp slap of the string on the bow in front of it, and a
/// short low "thwip" of air under both.
fn bow() -> Vec<f64> {
    let n = (RATE as f64 * 0.42) as usize;
    let mut noise = Noise(0x0b0b_0b0b);
    let period = (RATE as f64 / 170.0).round() as usize;
    let mut line: Vec<f64> = (0..period).map(|_| noise.next()).collect();
    // Take the top off the initial burst so the string rings, not hisses.
    for i in 1..period {
        line[i] = 0.55 * line[i] + 0.45 * line[i - 1];
    }
    let mut out = vec![0.0; n];
    let mut pos = 0;
    for (i, o) in out.iter_mut().enumerate() {
        let a = line[pos];
        let b = line[(pos + 1) % period];
        let v = 0.5 * (a + b) * 0.9965;
        line[pos] = v;
        pos = (pos + 1) % period;
        let t = i as f64 / RATE as f64;
        // Pluck envelope: loud at once, gone in a third of a second.
        *o = v * (1.0 - t / 0.42).max(0.0).powi(2);
    }
    // The slap: a few milliseconds of noise.
    for (i, o) in out
        .iter_mut()
        .take((RATE as f64 * 0.012) as usize)
        .enumerate()
    {
        let t = i as f64 / (RATE as f64 * 0.012);
        *o += noise.next() * 0.7 * (1.0 - t);
    }
    // The rush of air: low noise, a swell and fall over 90 ms.
    let mut lp = 0.0;
    for (i, o) in out
        .iter_mut()
        .take((RATE as f64 * 0.09) as usize)
        .enumerate()
    {
        let t = i as f64 / (RATE as f64 * 0.09);
        lp += (noise.next() - lp) * 0.12;
        *o += lp * 1.4 * (PI * t).sin();
    }
    out
}

/// An arrow landing: a wooden knock (a sine falling from 190 to 70 Hz, fast
/// decay) over a short burst of band-limited noise for the impact.
fn arrow_hit() -> Vec<f64> {
    let n = (RATE as f64 * 0.2) as usize;
    let mut noise = Noise(0x1234_abcd);
    let mut out = vec![0.0; n];
    let mut phase = 0.0;
    let mut lp = 0.0;
    for (i, o) in out.iter_mut().enumerate() {
        let t = i as f64 / RATE as f64;
        let f = 70.0 + 120.0 * (-t * 38.0).exp();
        phase += 2.0 * PI * f / RATE as f64;
        let knock = phase.sin() * (-t * 22.0).exp();
        lp += (noise.next() - lp) * 0.35;
        let hit = lp * (-t * 70.0).exp();
        *o = 0.9 * knock + 0.8 * hit;
    }
    out
}

fn main() {
    let root = voxide_tools::repo_root();
    let dir: &Path = &root.join("assets/sfx");
    for (name, samples) in [("bow", bow()), ("arrow_hit", arrow_hit())] {
        let path = dir.join(format!("{name}.wav"));
        std::fs::write(&path, wav(&samples)).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        println!("wrote {} ({} samples)", path.display(), samples.len());
    }
}

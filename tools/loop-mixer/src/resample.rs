//! Rational resampling with a Kaiser-windowed sinc.

use std::f64::consts::PI;

/// Zeroth-order modified Bessel function of the first kind.
fn i0(x: f64) -> f64 {
    let (mut sum, mut term, mut k) = (1.0, 1.0, 1.0);
    while term > 1e-14 * sum {
        term *= (x / (2.0 * k)).powi(2);
        sum += term;
        k += 1.0;
    }
    sum
}

/// Half the kernel width, in input samples. 64 each side puts the stop band
/// about 1.9 kHz above the pass band edge at 44.1 kHz with Kaiser beta 9
/// (roughly 90 dB down).
const HALF: usize = 64;
const BETA: f64 = 9.0;

/// Resample `input` from `from` to `to` frames a second. The two must differ by
/// a ratio whose reduced numerator and denominator are small (44100 to 37800
/// is 7 to 6). The low-pass cutoff is 95 percent of the lower rate's Nyquist
/// frequency, so nothing above the new Nyquist frequency aliases down.
pub fn resample(input: &[f64], from: u32, to: u32) -> Vec<f64> {
    let g = gcd(from, to);
    let (num, den) = ((from / g) as usize, (to / g) as usize); // input step per `den` outputs
    let fc = 0.95 * (to.min(from) as f64 / 2.0) / from as f64; // cycles per input sample
    let n_out = (input.len() as u64 * den as u64 / num as u64) as usize;
    // One kernel per fractional position.
    let kernels: Vec<Vec<f64>> = (0..den)
        .map(|phase| {
            let frac = phase as f64 * num as f64 / den as f64 % 1.0;
            let mut k: Vec<f64> = (0..2 * HALF)
                .map(|j| {
                    let x = (j as f64 - (HALF as f64 - 1.0)) - frac;
                    let sinc = if x == 0.0 { 2.0 * fc } else { (2.0 * PI * fc * x).sin() / (PI * x) };
                    let r = x / HALF as f64;
                    let w = if r.abs() >= 1.0 { 0.0 } else { i0(BETA * (1.0 - r * r).sqrt()) / i0(BETA) };
                    sinc * w
                })
                .collect();
            let sum: f64 = k.iter().sum();
            k.iter_mut().for_each(|v| *v /= sum);
            k
        })
        .collect();
    let mut out = Vec::with_capacity(n_out);
    for n in 0..n_out {
        let pos = n * num; // in units of 1/den input samples
        let base = pos / den;
        let kernel = &kernels[pos % den];
        let mut acc = 0.0;
        for (j, h) in kernel.iter().enumerate() {
            let i = base as isize + j as isize - (HALF as isize - 1);
            if i >= 0 && (i as usize) < input.len() {
                acc += input[i as usize] * h;
            }
        }
        out.push(acc);
    }
    out
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

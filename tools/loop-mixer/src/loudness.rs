//! EBU R128 / ITU-R BS.1770 integrated loudness and true peak, to level the
//! pieces to a common loudness.

use crate::resample::resample;

/// A biquad in direct form I, double precision.
#[derive(Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
}

impl Biquad {
    fn run(&self, x: &[f64]) -> Vec<f64> {
        let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
        x.iter()
            .map(|&x0| {
                let y0 = self.b[0] * x0 + self.b[1] * x1 + self.b[2] * x2 - self.a[0] * y1 - self.a[1] * y2;
                x2 = x1;
                x1 = x0;
                y2 = y1;
                y1 = y0;
                y0
            })
            .collect()
    }
}

/// The two K-weighting stages for sample rate `fs`: a high shelf (+4 dB above
/// about 1.7 kHz, the head's acoustic effect) and the 38 Hz high-pass.
fn k_weighting(fs: f64) -> [Biquad; 2] {
    let (f0, gain_db, q) = (1_681.974_450_955_533, 3.999_843_853_973_347, 0.707_175_236_955_419_6);
    let k = (std::f64::consts::PI * f0 / fs).tan();
    let vh = 10f64.powf(gain_db / 20.0);
    let vb = vh.powf(0.499_666_774_154_541_6);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad {
        b: [(vh + vb * k / q + k * k) / a0, 2.0 * (k * k - vh) / a0, (vh - vb * k / q + k * k) / a0],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
    };
    let (f0, q) = (38.135_470_876_024_44, 0.500_327_037_323_877_3);
    let k = (std::f64::consts::PI * f0 / fs).tan();
    let a0 = 1.0 + k / q + k * k;
    let high_pass = Biquad {
        b: [1.0, -2.0, 1.0],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
    };
    [shelf, high_pass]
}

/// Integrated loudness in LUFS of a stereo signal: 400 ms blocks every 100 ms,
/// an absolute gate at -70 LUFS and a relative gate 10 LU under what passes it.
pub fn integrated_lufs(ch: [&[f64]; 2], fs: u32) -> f64 {
    let stages = k_weighting(fs as f64);
    let weighted: Vec<Vec<f64>> = ch.iter().map(|c| stages[1].run(&stages[0].run(c))).collect();
    let block = (0.4 * fs as f64) as usize;
    let step = (0.1 * fs as f64) as usize;
    let n = weighted[0].len();
    let mut energies = Vec::new();
    let mut at = 0;
    while at + block <= n {
        let z: f64 = weighted.iter().map(|w| w[at..at + block].iter().map(|v| v * v).sum::<f64>() / block as f64).sum();
        energies.push(z);
        at += step;
    }
    let lufs = |z: f64| -0.691 + 10.0 * z.log10();
    let above: Vec<f64> = energies.iter().copied().filter(|&z| lufs(z) > -70.0).collect();
    if above.is_empty() {
        return f64::NEG_INFINITY;
    }
    let relative = lufs(above.iter().sum::<f64>() / above.len() as f64) - 10.0;
    let kept: Vec<f64> = above.into_iter().filter(|&z| lufs(z) > relative).collect();
    if kept.is_empty() {
        return f64::NEG_INFINITY;
    }
    lufs(kept.iter().sum::<f64>() / kept.len() as f64)
}

/// True peak: the largest absolute value after 4x oversampling, 1.0 is full scale.
pub fn true_peak(ch: [&[f64]; 2], fs: u32) -> f64 {
    ch.iter()
        .flat_map(|c| resample(c, fs, fs * 4))
        .fold(0.0f64, |m, v| m.max(v.abs()))
}

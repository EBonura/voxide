//! Measure WAVs: integrated loudness and true peak, and the gain between two files.
use loop_mixer::{loudness, wav};
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let load = |p: &str| wav::read(&std::fs::read(p).unwrap()).unwrap();
    for p in &args {
        let a = load(p);
        let r = [&a.ch[0][..], &a.ch[1][..]];
        println!(
            "{p}: {:.2} LUFS, true peak {:.2} dBTP",
            loudness::integrated_lufs(r, a.rate),
            20.0 * loudness::true_peak(r, a.rate).log10()
        );
    }
    if args.len() == 2 {
        let (a, b) = (load(&args[0]), load(&args[1]));
        let n = a.ch[0].len().min(b.ch[0].len());
        let (mut ab, mut aa) = (0.0, 0.0);
        for i in 0..n {
            ab += a.ch[0][i] * b.ch[0][i];
            aa += a.ch[0][i] * a.ch[0][i];
        }
        println!(
            "gain {} over {}: {:.3} dB",
            args[1],
            args[0],
            20.0 * (ab / aa).log10()
        );
    }
}

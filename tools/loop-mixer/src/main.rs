//! `loop-mixer --src DIR --out DIR [--seed N] [--pieces N] [--max-seconds S] LOOP.wav...`
//!
//! Renders the pieces as `piece-1.wav`, `piece-2.wav`, ... (shortest first, so
//! pieces that share an XA file are of a length) and prints one line each.

use loop_mixer::{mix, plan, wav};
use std::path::PathBuf;

fn main() {
    if let Err(e) = run() {
        eprintln!("loop-mixer: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let (mut src, mut out) = (None, None);
    let (mut seed, mut count, mut max) = (20_261_015u64, 4usize, 168.0f64);
    let mut names: Vec<String> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = || args.next().ok_or(format!("{a} takes a value"));
        match a.as_str() {
            "--src" => src = Some(PathBuf::from(value()?)),
            "--out" => out = Some(PathBuf::from(value()?)),
            "--seed" => seed = value()?.parse().map_err(|_| "--seed is a whole number")?,
            "--pieces" => count = value()?.parse().map_err(|_| "--pieces is a whole number")?,
            "--max-seconds" => max = value()?.parse().map_err(|_| "--max-seconds is a number")?,
            _ if a.starts_with("--") => return Err(format!("unknown flag {a}")),
            _ => names.push(a),
        }
    }
    let (src, out) = (src.ok_or("--src DIR is required")?, out.ok_or("--out DIR is required")?);
    if names.is_empty() {
        return Err("name the loop WAVs".into());
    }
    let mut loops = Vec::new();
    for n in &names {
        let bytes = std::fs::read(src.join(n)).map_err(|e| format!("{n}: {e}"))?;
        let a = wav::read(&bytes).map_err(|e| format!("{n}: {e}"))?;
        if loops.first().is_some_and(|f: &wav::Audio| f.rate != a.rate) {
            return Err(format!("{n}: the loops must share one sample rate"));
        }
        loops.push(a);
    }
    let lengths: Vec<f64> = loops.iter().map(|a| a.ch[0].len() as f64 / a.rate as f64).collect();
    let mut pieces = plan::set(seed, &lengths, count, max);
    pieces.sort_by(|a, b| plan::seconds(a, &lengths).total_cmp(&plan::seconds(b, &lengths)));
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let mut dither = loop_mixer::rng::Rng::new(seed ^ 0xD17E_D17E);
    let mut total = 0.0;
    for (i, p) in pieces.iter().enumerate() {
        let r = mix::render(p, &loops, &mut dither);
        let secs = r.left.len() as f64 / mix::OUT_RATE as f64;
        total += secs;
        let path = out.join(format!("piece-{}.wav", i + 1));
        std::fs::write(&path, wav::write16(mix::OUT_RATE, &r.left, &r.right)).map_err(|e| e.to_string())?;
        let recipe: Vec<String> = p
            .iter()
            .map(|s| format!("{} x{}", names[s.loop_index].trim_end_matches(".wav"), s.repeats))
            .collect();
        println!(
            "piece-{}.wav {:7.2} s  {:6.2} LUFS  true peak {:5.2} dBTP  gain {:6.2} dB  {}",
            i + 1,
            secs,
            r.lufs,
            r.peak_db,
            20.0 * r.gain.log10(),
            recipe.join(" > ")
        );
    }
    println!("{} pieces, {:.1} minutes", pieces.len(), total / 60.0);
    Ok(())
}

//! Host test for the thunder and rain SPU-ADPCM encoding in game/src/sfx.rs.
//!
//! The game encodes its two synthesised sounds at run time through `psx_hw::adpcm`
//! (`best_filter`, `shift_for_peak`, `encode_block`). Before that it carried a private encoder
//! whose prediction shifted the sum of the two products once, where the SPU shifts each product
//! (so the encoder's history drifted from what the SPU reconstructs). This test builds the two
//! real inputs (the game's own `Synth` and `RainSynth` source text, cut out of sfx.rs and
//! compiled unchanged), encodes them with the old private encoder and with the SDK kernel,
//! decodes both the way the SPU does, and requires that the new encoder's error against the
//! source is not worse than the old one's, in mean and in peak, for each sound.

mod common;

/// The private encoder as it shipped before the SDK kernel replaced it (`adpcm_block` of
/// game/src/sfx.rs at ab521b3), verbatim, as the oracle of what the game did.
const OLD_ENCODER: &str = r#"
const ADPCM_K: [(i32, i32); 5] = psx_hw::adpcm::FILTERS;

fn old_adpcm_block(s: &[i32; 28], p1: &mut i32, p2: &mut i32, out: &mut [u8]) {
    let mut best = (i32::MAX, 0usize);
    let mut f = 0;
    while f < 5 {
        let (k0, k1) = ADPCM_K[f];
        let (mut a, mut b) = (*p1, *p2);
        let mut m = 0;
        let mut i = 0;
        while i < 28 {
            let r = s[i] - ((a * k0 + b * k1) >> 6);
            m = m.max(r.abs());
            b = a;
            a = s[i];
            i += 1;
        }
        if m < best.0 {
            best = (m, f);
        }
        f += 1;
    }
    let f = best.1;
    let mut range = 12;
    while range > 0 && (7 << (12 - range)) < best.0 {
        range -= 1;
    }
    let shift = range;
    let (k0, k1) = ADPCM_K[f];
    out[0] = (shift as u8) | ((f as u8) << 4);
    out[1] = 0;
    let step = 12 - shift;
    let mut i = 0;
    while i < 28 {
        let pred = (*p1 * k0 + *p2 * k1) >> 6;
        let r = s[i] - pred;
        let q = if step > 0 {
            ((r + (1 << (step - 1))) >> step).clamp(-8, 7)
        } else {
            r.clamp(-8, 7)
        };
        let dec = ((q << step) + pred).clamp(-32_768, 32_767);
        *p2 = *p1;
        *p1 = dec;
        let nib = (q & 0xF) as u8;
        if i % 2 == 0 {
            out[2 + i / 2] = nib;
        } else {
            out[2 + i / 2] |= nib << 4;
        }
        i += 1;
    }
}
"#;

/// The game's encoder call, as in sfx.rs `adpcm_block`.
const NEW_ENCODER: &str = r#"
fn new_adpcm_block(s: &[i32; 28], p1: &mut i32, p2: &mut i32, out: &mut [u8]) {
    use psx_hw::adpcm::{best_filter, encode_block, filter_peaks, shift_for_peak};
    let Some(out) = out.first_chunk_mut::<16>() else {
        return;
    };
    let state = (*p1, *p2);
    let (filter, peak) = best_filter(&filter_peaks(s, state));
    (*p1, *p2) = encode_block(s, state, filter, shift_for_peak(peak), 0, out);
}
"#;

const HARNESS: &str = r#"
/// What the SPU plays: two floored products per sample, clamped to 16 bits, the clamped value
/// becomes the history. `blocks` is a run of 16-byte blocks from a zero history.
fn spu_decode(blocks: &[u8]) -> Vec<i32> {
    let mut out = Vec::new();
    let (mut s1, mut s2) = (0i32, 0i32);
    for b in blocks.chunks(16) {
        let shift = (b[0] & 0xf) as i32;
        let shift = if shift > 12 { 9 } else { shift };
        let f = psx_hw::adpcm::FILTERS[((b[0] >> 4) as usize).min(4)];
        for i in 0..28 {
            let byte = b[2 + i / 2];
            let nib = if i % 2 == 0 { byte & 0xf } else { byte >> 4 } as i32;
            let nib = (nib << 28) >> 28;
            let v = ((nib << 12) >> shift) + psx_hw::adpcm::predict(f, s1, s2);
            let v = v.clamp(-32_768, 32_767);
            s2 = s1;
            s1 = v;
            out.push(v);
        }
    }
    out
}

struct Err {
    n: usize,
    sum_abs: u64,
    peak: i32,
    sum_sq: u64,
}

fn error(src: &[i32], dec: &[i32]) -> Err {
    let mut e = Err { n: src.len(), sum_abs: 0, peak: 0, sum_sq: 0 };
    for (a, b) in src.iter().zip(dec) {
        let d = (a - b).abs();
        e.sum_abs += d as u64;
        e.sum_sq += (d as u64) * (d as u64);
        e.peak = e.peak.max(d);
    }
    e
}

fn run(name: &str, src: &[i32]) -> bool {
    let mut results = Vec::new();
    for new in [false, true] {
        let (mut p1, mut p2) = (0, 0);
        let mut bytes = Vec::new();
        for blk in src.chunks(28) {
            let mut s = [0i32; 28];
            s[..blk.len()].copy_from_slice(blk);
            let mut out = [0u8; 16];
            if new {
                new_adpcm_block(&s, &mut p1, &mut p2, &mut out);
            } else {
                old_adpcm_block(&s, &mut p1, &mut p2, &mut out);
            }
            bytes.extend_from_slice(&out);
        }
        let dec = spu_decode(&bytes);
        results.push((error(src, &dec), bytes));
    }
    let differing = results[0].1.chunks(16).zip(results[1].1.chunks(16)).filter(|(a, b)| a != b).count();
    let blocks = results[0].1.len() / 16;
    let mut ok = true;
    for (label, (e, _)) in ["old", "new"].iter().zip(&results) {
        println!(
            "{name} {label}: samples {} mean_abs {}.{:04} rms_sq {} peak {}",
            e.n,
            e.sum_abs / e.n as u64,
            (e.sum_abs % e.n as u64) * 10_000 / e.n as u64,
            e.sum_sq / e.n as u64,
            e.peak
        );
    }
    let (old, new) = (&results[0].0, &results[1].0);
    // Means compared exactly: sum_abs is over the same sample count.
    if new.sum_abs > old.sum_abs || new.peak > old.peak {
        ok = false;
    }
    println!("{name}: {differing} of {blocks} blocks encode differently; not worse: {ok}");
    ok
}

fn thunder_source() -> Vec<i32> {
    let mut s = Synth::new();
    let mut src = Vec::new();
    for b in 0..THUNDER_BLOCKS {
        for i in 0..28u32 {
            let v = if b * 28 + i < THUNDER_LEN { s.next() } else { 0 };
            src.push(((v * THUNDER_GAIN) >> 12).clamp(-32_768, 32_767));
        }
    }
    src
}

fn rain_source() -> Vec<i32> {
    let mut s = RainSynth::new();
    let mut src = Vec::new();
    for _ in 0..RAIN_BLOCKS {
        for _ in 0..28 {
            src.push(((s.next() * RAIN_GAIN) >> 12).clamp(-32_768, 32_767));
        }
    }
    src
}

fn main() {
    let a = run("thunder", &thunder_source());
    let b = run("rain", &rain_source());
    if !(a && b) {
        std::process::exit(1);
    }
}
"#;

/// The text of `source` from `from` up to (not including) `to`.
fn cut<'a>(source: &'a str, from: &str, to: &str) -> &'a str {
    let start = source.find(from).unwrap_or_else(|| panic!("sfx.rs lost `{from}`"));
    let end = source[start..].find(to).unwrap_or_else(|| panic!("sfx.rs lost `{to}`")) + start;
    &source[start..end]
}

/// The single line of `source` that starts with `prefix`.
fn line<'a>(source: &'a str, prefix: &str) -> &'a str {
    source
        .lines()
        .find(|l| l.starts_with(prefix))
        .unwrap_or_else(|| panic!("sfx.rs lost `{prefix}`"))
}

#[test]
fn the_sdk_encoder_is_not_worse_than_the_old_one_on_thunder_and_rain() {
    let root = common::root();
    let scratch = common::Scratch::new("vox-adpcm-");
    let sfx = std::fs::read_to_string(root.join("game/src/sfx.rs")).expect("sfx.rs");
    let sdk = root.join(".psoxide/crates/psx-hw/src/adpcm.rs");
    assert!(sdk.exists(), "hydrate .psoxide first (make psoxide)");

    let mut program = String::new();
    program.push_str("#![allow(dead_code, unused)]\n");
    program.push_str(&format!(
        "mod psx_hw {{ #[path = {:?}] pub mod adpcm; }}\n",
        sdk.display().to_string()
    ));
    program.push_str(
        "mod psx_fx { pub struct NrLcg; impl NrLcg { \
         pub fn step(s: u32) -> u32 { s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223) } } }\n",
    );
    // The game's synthesisers and constants, compiled as written.
    program.push_str(line(&sfx, "const THUNDER_GAIN"));
    program.push('\n');
    program.push_str(line(&sfx, "const RAIN_GAIN"));
    program.push('\n');
    program.push_str(line(&sfx, "const RAIN_BLOCKS"));
    program.push('\n');
    program.push_str(cut(&sfx, "const THUNDER_HZ", "/// SPU-ADPCM, filter 0 to 4"));
    program.push_str(cut(&sfx, "struct RainSynth {", "/// Synthesise and upload the rain loop"));
    // The new encoder must still be this call in the game.
    assert!(
        sfx.contains("best_filter(&filter_peaks(s, state))")
            && sfx.contains("encode_block(s, state, filter, shift_for_peak(peak), 0, out)"),
        "sfx.rs adpcm_block no longer calls the SDK kernel as NEW_ENCODER does"
    );
    program.push_str(OLD_ENCODER);
    program.push_str(NEW_ENCODER);
    program.push_str(HARNESS);

    let output = common::compile_and_run(
        &scratch,
        &program,
        &["--edition", "2021", "-O"],
        &[],
        &root,
    );
    let text = String::from_utf8_lossy(&output.stdout);
    println!("{text}");
    assert!(text.contains("thunder: ") && text.contains("rain: "));
}

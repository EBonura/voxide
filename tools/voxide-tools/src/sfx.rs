//! Cook `assets/sfx/*.wav` into SPU-ADPCM: `assets/sfx/pak/chunk_3000.bin` and
//! `game/src/sfxdata.rs`.
//!
//! Sources are CC0 recordings (Kenney.nl packs, freesound.org, see
//! `assets/pack/CREDITS.md`), pre-trimmed to 22050/11025 Hz mono s16 by ffmpeg.
//! This peak-normalizes, encodes PS1 SPU-ADPCM (filters 0-4, best filter and
//! shift per 28-sample block), appends a self-looping silent block so a
//! finished one-shot parks the voice on silence, and emits the offset/rate
//! table the game indexes by name.

use std::path::Path;

/// One SPU ADPCM block: shift/filter, flags, 14 bytes of data.
const BLOCK_BYTES: usize = 16;

/// Fixed order: the generated `S_*` consts index `SAMPLES`.
pub const SOUNDS: [&str; 25] = [
    "step_grass",
    "step_stone",
    "step_sand",
    "step_wood",
    "dig_soft",
    "dig_stone",
    "dig_wood",
    "break",
    "place",
    "hurt",
    "door",
    "chest",
    "click",
    "confirm",
    "pig",
    "cow",
    "sheep",
    "chicken",
    "zombie",
    "bones",
    "bark",
    "hiss",
    "splash",
    "eat",
    "explode",
];

/// The five SPU predictor filters as (k0, k1) in 1/64ths.
const FILTERS: [(i32, i32); 5] = [(0, 0), (60, 0), (115, -52), (98, -55), (122, -60)];

/// Mono 16-bit PCM from a RIFF/WAVE file: sample rate and samples.
pub fn read_wav(file: &[u8]) -> Result<(u32, Vec<i16>), String> {
    if file.len() < 12 || &file[0..4] != b"RIFF" || &file[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }
    let mut pos = 12;
    let mut format = None;
    let mut data = None;
    while pos + 8 <= file.len() {
        let len = u32::from_le_bytes(file[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let end = (pos + 8 + len).min(file.len());
        let body = &file[pos + 8..end];
        match &file[pos..pos + 4] {
            b"fmt " => format = Some(body),
            b"data" => {
                data = Some(body);
                break;
            }
            _ => {}
        }
        pos += 8 + len + (len & 1);
    }
    let format = format.ok_or("no fmt chunk")?;
    let data = data.ok_or("no data chunk")?;
    if format.len() < 16 {
        return Err("short fmt chunk".into());
    }
    let channels = u16::from_le_bytes([format[2], format[3]]);
    let rate = u32::from_le_bytes(format[4..8].try_into().unwrap());
    let bits = u16::from_le_bytes([format[14], format[15]]);
    if channels != 1 || bits != 16 {
        return Err(format!(
            "expected mono 16-bit, got {channels} channel(s) at {bits} bits"
        ));
    }
    let samples = data
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    Ok((rate, samples))
}

/// Peak-normalize to 90% of full scale.
fn normalize(samples: &[i16]) -> Vec<i32> {
    let peak = samples
        .iter()
        .map(|&s| i32::from(s).abs())
        .max()
        .unwrap_or(0)
        .max(1);
    let scale = 0.9 * 32767.0 / f64::from(peak);
    samples
        .iter()
        .map(|&s| (f64::from(s) * scale) as i32)
        .collect()
}

/// Best (filter, shift) for one 28-sample block: its 16 bytes and the two
/// samples of decoder history it leaves behind.
fn encode_block(block: &[i32], prev1: i32, prev2: i32) -> ([u8; 16], i32, i32) {
    struct Best {
        err: i64,
        filter: usize,
        shift: i32,
        nibbles: Vec<u8>,
        p1: i32,
        p2: i32,
    }
    let mut best: Option<Best> = None;
    for (filter, &(k0, k1)) in FILTERS.iter().enumerate() {
        for shift in 0..13 {
            let (mut p1, mut p2) = (prev1, prev2);
            let mut nibbles = Vec::with_capacity(28);
            let mut err = 0i64;
            for &s in block {
                let predicted = (p1 * k0 + p2 * k1) >> 6;
                let residual = s - predicted;
                // Round-to-nearest quantize into a signed nibble.
                let q = if shift < 12 {
                    ((residual + (1 << (11 - shift))) >> (12 - shift)).clamp(-8, 7)
                } else {
                    residual.clamp(-8, 7)
                };
                let decoded = ((q << (12 - shift)) + predicted).clamp(-32768, 32767);
                nibbles.push((q & 0xf) as u8);
                err += i64::from(decoded - s) * i64::from(decoded - s);
                p2 = p1;
                p1 = decoded;
            }
            if best.as_ref().is_none_or(|b| err < b.err) {
                best = Some(Best {
                    err,
                    filter,
                    shift,
                    nibbles,
                    p1,
                    p2,
                });
            }
        }
    }
    let best = best.expect("filters and shifts are non-empty");
    let mut out = [0u8; 16];
    out[0] = (best.shift as u8 & 0xf) | ((best.filter as u8) << 4);
    for (i, &n) in best.nibbles.iter().enumerate() {
        out[2 + i / 2] |= if i % 2 == 0 { n } else { n << 4 };
    }
    (out, best.p1, best.p2)
}

/// Encode normalized samples as SPU-ADPCM blocks plus the silent loop tail.
pub fn encode(samples: &[i32]) -> Vec<u8> {
    let mut samples = samples.to_vec();
    samples.resize(samples.len() + (28 - samples.len() % 28) % 28, 0);
    let mut blob = Vec::new();
    let (mut p1, mut p2) = (0, 0);
    for block in samples.chunks(28) {
        let (bytes, n1, n2) = encode_block(block, p1, p2);
        blob.extend_from_slice(&bytes);
        p1 = n1;
        p2 = n2;
    }
    // Self-looping silent tail: a finished voice parks here until re-keyed.
    let mut silent = [0u8; 16];
    silent[1] = 0x07; // loop start + loop end + repeat
    blob.extend_from_slice(&silent);
    blob
}

/// One cooked sample's table entry.
struct Entry {
    name: &'static str,
    offset: usize,
    len: usize,
    rate: u32,
}

/// Render `sfxdata.rs` for the cooked table.
fn render(table: &[Entry]) -> String {
    let mut lines: Vec<String> = [
        "//! GENERATED by tools/voxide-tools (convert-sfx) -- do not edit by hand.",
        "//! CC0 sampled SFX (Kenney.nl + freesound.org, see assets/pack/CREDITS.md)",
        "//! as SPU-ADPCM blobs with per-sample offsets and native rates.",
        "",
        "/// One cooked sample: byte offset into the uploaded SPU bank, the",
        "/// rate it was recorded at, and how many ADPCM blocks of real audio",
        "/// it holds.",
        "///",
        "/// `blocks` excludes the self-looping silent tail every sample ends",
        "/// with, so it is the length of the sound itself. psx-sfx times its",
        "/// cutoff from it: without a length there is nothing to schedule and",
        "/// a finished voice sits on the tail with a live envelope instead of",
        "/// being silenced.",
        "pub struct Sample {",
        "    pub off: u32,",
        "    pub rate: u16,",
        "    pub blocks: u16,",
        "}",
        "",
        "/// WORLD.PAK chunk id of the cooked bank (streamed to SPU at boot;",
        "/// the blob never lives in main RAM).",
        "pub const SFX_CHUNK_ID: u32 = 3000;",
        "",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();
    for (i, e) in table.iter().enumerate() {
        lines.push(format!(
            "pub const S_{}: usize = {i};",
            e.name.to_uppercase()
        ));
    }
    lines.push(String::new());
    lines.push(format!("pub static SAMPLES: [Sample; {}] = [", table.len()));
    for e in table {
        // The trailing silent block is termination, not sound: a cutoff timed
        // to include it would hold the voice open past the end of the sample.
        let blocks = e.len / BLOCK_BYTES - 1;
        lines.push(format!(
            "    Sample {{ off: {}, rate: {}, blocks: {blocks} }}, // {}",
            e.offset, e.rate, e.name
        ));
    }
    lines.push("];".to_string());
    lines.push(String::new());
    lines.join("\n")
}

/// Cook every sound under `root`; returns the bank and the `sfxdata.rs` text.
pub fn cook(root: &Path, log: bool) -> Result<(Vec<u8>, String), String> {
    let dir = root.join("assets/sfx");
    let mut bank = Vec::new();
    let mut table = Vec::new();
    for name in SOUNDS {
        let path = dir.join(format!("{name}.wav"));
        let file = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let (rate, samples) = read_wav(&file).map_err(|e| format!("{}: {e}", path.display()))?;
        let blob = encode(&normalize(&samples));
        if log {
            println!(
                "{name:12} {rate:5}Hz {:6} smp -> {:6}B",
                samples.len(),
                blob.len()
            );
        }
        table.push(Entry {
            name,
            offset: bank.len(),
            len: blob.len(),
            rate,
        });
        bank.extend_from_slice(&blob);
    }
    Ok((bank, render(&table)))
}

/// Cook and write the bank and `game/src/sfxdata.rs` under `root`.
pub fn run(root: &Path) -> Result<(), String> {
    let (bank, rs) = cook(root, true)?;
    let pak = root.join("assets/sfx/pak");
    std::fs::create_dir_all(&pak).map_err(|e| format!("{}: {e}", pak.display()))?;
    let bin = pak.join("chunk_3000.bin");
    std::fs::write(&bin, &bank).map_err(|e| format!("{}: {e}", bin.display()))?;
    println!(
        "sfx.bin: {} bytes ({:.0} KiB SPU RAM)",
        bank.len(),
        bank.len() as f64 / 1024.0
    );
    let out = root.join("game/src/sfxdata.rs");
    std::fs::write(&out, rs).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("wrote game/src/sfxdata.rs");
    Ok(())
}

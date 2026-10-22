//! Just enough RIFF/WAVE: integer PCM in, 16-bit PCM out.

/// Decoded stereo audio, one `f64` in -1..1 per channel per frame.
pub struct Audio {
    /// Frames per second.
    pub rate: u32,
    /// Left channel, then right.
    pub ch: [Vec<f64>; 2],
}

fn le16(b: &[u8], at: usize) -> Result<u16, String> {
    Ok(u16::from_le_bytes(
        b.get(at..at + 2).ok_or("short WAV")?.try_into().unwrap(),
    ))
}
fn le32(b: &[u8], at: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(
        b.get(at..at + 4).ok_or("short WAV")?.try_into().unwrap(),
    ))
}

/// Read a 16, 24 or 32-bit integer PCM WAV with one or two channels (a mono
/// file is used for both).
pub fn read(bytes: &[u8]) -> Result<Audio, String> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }
    let (mut fmt, mut data) = (None, None);
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let len = le32(bytes, at + 4)? as usize;
        let end = (at + 8 + len).min(bytes.len());
        match &bytes[at..at + 4] {
            b"fmt " => fmt = Some(&bytes[at + 8..end]),
            b"data" => data = Some(&bytes[at + 8..end]),
            _ => {}
        }
        at = at + 8 + len + (len & 1);
    }
    let fmt = fmt.ok_or("no fmt chunk")?;
    let data = data.ok_or("no data chunk")?;
    let mut format = le16(fmt, 0)?;
    if format == 0xfffe && fmt.len() >= 26 {
        format = le16(fmt, 24)?;
    }
    let channels = le16(fmt, 2)? as usize;
    let rate = le32(fmt, 4)?;
    let bits = le16(fmt, 14)? as usize;
    if format != 1 || !(channels == 1 || channels == 2) || ![16, 24, 32].contains(&bits) {
        return Err(format!(
            "unsupported WAV: format {format}, {channels} channels, {bits} bits"
        ));
    }
    let width = bits / 8;
    let frames = data.len() / (width * channels);
    let mut ch = [Vec::with_capacity(frames), Vec::with_capacity(frames)];
    let scale = 1.0 / (1u64 << (bits - 1)) as f64;
    for f in 0..frames {
        let mut v = [0.0; 2];
        for c in 0..channels {
            let o = (f * channels + c) * width;
            let s = match bits {
                16 => i16::from_le_bytes([data[o], data[o + 1]]) as i32,
                24 => {
                    ((data[o] as i32) << 8
                        | (data[o + 1] as i32) << 16
                        | (data[o + 2] as i32) << 24)
                        >> 8
                }
                _ => i32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]),
            };
            v[c] = s as f64 * scale;
        }
        if channels == 1 {
            v[1] = v[0];
        }
        ch[0].push(v[0]);
        ch[1].push(v[1]);
    }
    Ok(Audio { rate, ch })
}

/// A stereo 16-bit WAV.
pub fn write16(rate: u32, left: &[i16], right: &[i16]) -> Vec<u8> {
    let frames = left.len();
    let data_len = frames * 4;
    let mut w = Vec::with_capacity(44 + data_len);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 4).to_le_bytes());
    w.extend_from_slice(&4u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data_len as u32).to_le_bytes());
    for i in 0..frames {
        w.extend_from_slice(&left[i].to_le_bytes());
        w.extend_from_slice(&right[i].to_le_bytes());
    }
    w
}

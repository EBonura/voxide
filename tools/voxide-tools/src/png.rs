//! A minimal PNG reader: 8-bit RGB or RGBA, no interlacing. That is every
//! file in the 16x16 block pack.

use crate::inflate;

/// A decoded image as RGBA bytes, row by row.
pub struct Image {
    /// Width in pixels.
    pub width: usize,
    /// Height in pixels.
    pub height: usize,
    /// `width * height` RGBA pixels.
    pub rgba: Vec<[u8; 4]>,
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (ia, ib, ic) = (i32::from(a), i32::from(b), i32::from(c));
    let p = ia + ib - ic;
    let (pa, pb, pc) = ((p - ia).abs(), (p - ib).abs(), (p - ic).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Decode a PNG file's bytes.
pub fn decode(file: &[u8]) -> Result<Image, String> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !file.starts_with(SIGNATURE) {
        return Err("not a PNG file".into());
    }
    let mut pos = SIGNATURE.len();
    let mut header = None;
    let mut compressed = Vec::new();
    while pos + 8 <= file.len() {
        let len = u32::from_be_bytes(file[pos..pos + 4].try_into().unwrap()) as usize;
        let kind = &file[pos + 4..pos + 8];
        let body = file
            .get(pos + 8..pos + 8 + len)
            .ok_or("PNG chunk is cut off")?;
        match kind {
            b"IHDR" => header = Some(body),
            b"IDAT" => compressed.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        pos += 12 + len;
    }
    let header = header.ok_or("PNG has no IHDR")?;
    if header.len() < 13 {
        return Err("short IHDR".into());
    }
    let width = u32::from_be_bytes(header[0..4].try_into().unwrap()) as usize;
    let height = u32::from_be_bytes(header[4..8].try_into().unwrap()) as usize;
    let (depth, color, interlace) = (header[8], header[9], header[12]);
    let bpp = match (depth, color, interlace) {
        (8, 2, 0) => 3,
        (8, 6, 0) => 4,
        _ => {
            return Err(format!(
                "unsupported PNG (depth {depth}, color {color}, interlace {interlace})"
            ))
        }
    };
    let raw = inflate::zlib(&compressed)?;
    let stride = width * bpp;
    if raw.len() != height * (stride + 1) {
        return Err("PNG pixel data has the wrong size".into());
    }
    let mut rows = vec![0u8; height * stride];
    for y in 0..height {
        let filter = raw[y * (stride + 1)];
        let line = &raw[y * (stride + 1) + 1..(y + 1) * (stride + 1)];
        for x in 0..stride {
            let left = if x >= bpp {
                rows[y * stride + x - bpp]
            } else {
                0
            };
            let up = if y > 0 { rows[(y - 1) * stride + x] } else { 0 };
            let up_left = if y > 0 && x >= bpp {
                rows[(y - 1) * stride + x - bpp]
            } else {
                0
            };
            let add = match filter {
                0 => 0,
                1 => left,
                2 => up,
                3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
                4 => paeth(left, up, up_left),
                _ => return Err(format!("bad PNG filter {filter}")),
            };
            rows[y * stride + x] = line[x].wrapping_add(add);
        }
    }
    let rgba = rows
        .chunks_exact(bpp)
        .map(|p| [p[0], p[1], p[2], if bpp == 4 { p[3] } else { 255 }])
        .collect();
    Ok(Image {
        width,
        height,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_other_files() {
        assert!(decode(b"GIF89a").is_err());
    }
}

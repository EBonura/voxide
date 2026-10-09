//! A small DEFLATE (RFC 1951) and zlib (RFC 1950) decompressor, enough to read
//! the pack's PNG files without a dependency.

/// Why a stream could not be inflated.
pub type Error = String;

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    bit: u32,
}

impl Bits<'_> {
    fn take(&mut self, count: u32) -> Result<u32, Error> {
        let mut value = 0;
        for i in 0..count {
            let byte = *self
                .data
                .get(self.pos)
                .ok_or("deflate stream ended early")?;
            value |= (u32::from(byte >> self.bit) & 1) << i;
            self.bit += 1;
            if self.bit == 8 {
                self.bit = 0;
                self.pos += 1;
            }
        }
        Ok(value)
    }

    fn align(&mut self) {
        if self.bit != 0 {
            self.bit = 0;
            self.pos += 1;
        }
    }
}

/// A canonical Huffman code: how many codes have each length, and the symbols
/// ordered by (length, value).
struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Huffman {
        let mut counts = [0u16; 16];
        for &len in lengths {
            counts[usize::from(len)] += 1;
        }
        let mut symbols = Vec::new();
        for len in 1..16u8 {
            for (symbol, &l) in lengths.iter().enumerate() {
                if l == len {
                    symbols.push(symbol as u16);
                }
            }
        }
        Huffman { counts, symbols }
    }

    fn decode(&self, bits: &mut Bits) -> Result<u16, Error> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= bits.take(1)? as i32;
            let count = i32::from(self.counts[len]);
            if code - count < first {
                return Ok(self.symbols[(index + (code - first)) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err("invalid huffman code".into())
    }
}

const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

fn codes(
    bits: &mut Bits,
    out: &mut Vec<u8>,
    literals: &Huffman,
    distances: &Huffman,
) -> Result<(), Error> {
    loop {
        let symbol = literals.decode(bits)?;
        match symbol {
            0..=255 => out.push(symbol as u8),
            256 => return Ok(()),
            257..=285 => {
                let i = usize::from(symbol - 257);
                let length =
                    usize::from(LENGTH_BASE[i]) + bits.take(u32::from(LENGTH_EXTRA[i]))? as usize;
                let d = usize::from(distances.decode(bits)?);
                if d >= DIST_BASE.len() {
                    return Err("invalid distance code".into());
                }
                let distance =
                    usize::from(DIST_BASE[d]) + bits.take(u32::from(DIST_EXTRA[d]))? as usize;
                if distance > out.len() {
                    return Err("distance reaches before the output".into());
                }
                for _ in 0..length {
                    out.push(out[out.len() - distance]);
                }
            }
            _ => return Err("invalid literal/length code".into()),
        }
    }
}

fn dynamic_tables(bits: &mut Bits) -> Result<(Huffman, Huffman), Error> {
    const ORDER: [usize; 19] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];
    let hlit = bits.take(5)? as usize + 257;
    let hdist = bits.take(5)? as usize + 1;
    let hclen = bits.take(4)? as usize + 4;
    let mut code_lengths = [0u8; 19];
    for &slot in ORDER.iter().take(hclen) {
        code_lengths[slot] = bits.take(3)? as u8;
    }
    let code_table = Huffman::new(&code_lengths);
    let mut lengths = vec![0u8; hlit + hdist];
    let mut i = 0;
    while i < lengths.len() {
        let symbol = code_table.decode(bits)?;
        let (value, repeat) = match symbol {
            0..=15 => (symbol as u8, 1),
            16 => {
                let previous = *lengths[..i]
                    .last()
                    .ok_or("repeat with no previous length")?;
                (previous, 3 + bits.take(2)? as usize)
            }
            17 => (0, 3 + bits.take(3)? as usize),
            _ => (0, 11 + bits.take(7)? as usize),
        };
        if i + repeat > lengths.len() {
            return Err("code lengths overflow".into());
        }
        lengths[i..i + repeat].fill(value);
        i += repeat;
    }
    Ok((
        Huffman::new(&lengths[..hlit]),
        Huffman::new(&lengths[hlit..]),
    ))
}

/// Inflate a raw DEFLATE stream.
pub fn inflate(data: &[u8]) -> Result<Vec<u8>, Error> {
    let mut bits = Bits {
        data,
        pos: 0,
        bit: 0,
    };
    let mut out = Vec::new();
    loop {
        let last = bits.take(1)? == 1;
        match bits.take(2)? {
            0 => {
                bits.align();
                let header = data
                    .get(bits.pos..bits.pos + 4)
                    .ok_or("stored block header is cut off")?;
                let len = usize::from(u16::from_le_bytes([header[0], header[1]]));
                let start = bits.pos + 4;
                let body = data
                    .get(start..start + len)
                    .ok_or("stored block is cut off")?;
                out.extend_from_slice(body);
                bits.pos = start + len;
            }
            1 => {
                let mut lengths = [8u8; 288];
                lengths[144..256].fill(9);
                lengths[256..280].fill(7);
                codes(
                    &mut bits,
                    &mut out,
                    &Huffman::new(&lengths),
                    &Huffman::new(&[5u8; 30]),
                )?;
            }
            2 => {
                let (literals, distances) = dynamic_tables(&mut bits)?;
                codes(&mut bits, &mut out, &literals, &distances)?;
            }
            _ => return Err("reserved deflate block type".into()),
        }
        if last {
            return Ok(out);
        }
    }
}

/// Inflate a zlib stream (two header bytes, DEFLATE data, Adler-32 trailer).
/// The trailer is not checked.
pub fn zlib(data: &[u8]) -> Result<Vec<u8>, Error> {
    if data.len() < 2 || data[0] & 0x0f != 8 {
        return Err("not a zlib stream".into());
    }
    inflate(&data[2..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_block() {
        let stream = [0x01, 0x03, 0x00, 0xfc, 0xff, b'a', b'b', b'c'];
        assert_eq!(inflate(&stream).unwrap(), b"abc");
    }

    #[test]
    fn fixed_block_with_a_back_reference() {
        // zlib.compress(b"abcabcabcabc") from CPython: a fixed-Huffman block.
        let stream = [120, 156, 75, 76, 74, 78, 132, 33, 0, 29, 224, 4, 153];
        assert_eq!(zlib(&stream).unwrap(), b"abcabcabcabc");
    }

    #[test]
    fn rejects_truncated_input() {
        assert!(inflate(&[0x01, 0x03]).is_err());
    }
}

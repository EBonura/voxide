// SPDX-License-Identifier: GPL-2.0-or-later
//! Locate the versioned journey state in the exact PS-X EXE to be packed.

use std::{env, fs, io::Write, path::Path};

const HEADER_LEN: usize = 0x800;
const MAGIC: &[u8; 4] = b"VOXG";
const VERSION: u32 = 1;
const BLOCK_LEN: usize = 36;

fn word(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or("short PS-X EXE header")?;
    Ok(u32::from_le_bytes(
        slice.try_into().map_err(|_| "short word")?,
    ))
}

fn symbol_address(path: &Path) -> Result<u32, String> {
    let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.get(..8) != Some(b"PS-X EXE") {
        return Err("input is not a PS-X EXE".into());
    }
    let load = word(&bytes, 0x18)?;
    let image_len = word(&bytes, 0x1c)? as usize;
    let image = bytes
        .get(HEADER_LEN..HEADER_LEN + image_len)
        .ok_or("PS-X EXE payload is shorter than its header says")?;
    let mut matches = Vec::new();
    for (offset, block) in image.windows(BLOCK_LEN).enumerate() {
        if &block[..4] == MAGIC
            && word(block, 4)? == VERSION
            && word(block, 8)? == BLOCK_LEN as u32
            && block[12..].iter().all(|&byte| byte == 0)
        {
            matches.push(offset);
        }
    }
    if matches.len() != 1 {
        return Err(format!(
            "expected one initialized VOXIDE_GATE_STATE block, found {}",
            matches.len()
        ));
    }
    load.checked_add(matches[0] as u32)
        .ok_or_else(|| "gate block address overflowed".into())
}

fn main() {
    let mut args = env::args_os().skip(1);
    let Some(exe) = args.next() else {
        eprintln!("usage: gate-symbols PSX_EXE");
        std::process::exit(2);
    };
    if args.next().is_some() {
        eprintln!("usage: gate-symbols PSX_EXE");
        std::process::exit(2);
    }
    match symbol_address(Path::new(&exe)) {
        Ok(addr) => {
            writeln!(std::io::stdout(), "{addr:#010x} VOXIDE_GATE_STATE").expect("write address")
        }
        Err(error) => {
            eprintln!("gate-symbols: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(block_offsets: &[usize], declared_len: usize) -> Vec<u8> {
        let mut bytes = vec![0; HEADER_LEN + 0x300];
        bytes[..8].copy_from_slice(b"PS-X EXE");
        bytes[0x18..0x1c].copy_from_slice(&0x8001_0000u32.to_le_bytes());
        bytes[0x1c..0x20].copy_from_slice(&(declared_len as u32).to_le_bytes());
        for &offset in block_offsets {
            let block = &mut bytes[HEADER_LEN + offset..HEADER_LEN + offset + BLOCK_LEN];
            block[..4].copy_from_slice(MAGIC);
            block[4..8].copy_from_slice(&VERSION.to_le_bytes());
            block[8..12].copy_from_slice(&(BLOCK_LEN as u32).to_le_bytes());
        }
        bytes
    }

    fn check(name: &str, bytes: &[u8]) -> Result<u32, String> {
        let path = env::temp_dir().join(format!("vox-gate-symbols-{}-{name}", std::process::id()));
        fs::write(&path, bytes).unwrap();
        let result = symbol_address(&path);
        fs::remove_file(path).unwrap();
        result
    }

    #[test]
    fn locates_exact_versioned_block() {
        assert_eq!(
            check("valid", &sample(&[0x40], 0x300)).unwrap(),
            0x8001_0040
        );
    }

    #[test]
    fn rejects_ambiguous_initializer() {
        assert_eq!(
            check("duplicate", &sample(&[0x40, 0x180], 0x300)).unwrap_err(),
            "expected one initialized VOXIDE_GATE_STATE block, found 2"
        );
    }

    #[test]
    fn rejects_wrong_version_and_length() {
        let mut wrong_version = sample(&[0x40], 0x300);
        wrong_version[HEADER_LEN + 0x44] = 2;
        assert_eq!(
            check("version", &wrong_version).unwrap_err(),
            "expected one initialized VOXIDE_GATE_STATE block, found 0"
        );
        let mut wrong_length = sample(&[0x40], 0x300);
        wrong_length[HEADER_LEN + 0x48] = 16;
        assert_eq!(
            check("length", &wrong_length).unwrap_err(),
            "expected one initialized VOXIDE_GATE_STATE block, found 0"
        );
    }

    #[test]
    fn rejects_truncated_payload_and_invalid_image() {
        let mut truncated = sample(&[0x40], 0x300);
        truncated.truncate(HEADER_LEN + 0x200);
        assert_eq!(
            check("truncated", &truncated).unwrap_err(),
            "PS-X EXE payload is shorter than its header says"
        );
        assert_eq!(
            check("invalid", b"not a PS-X EXE").unwrap_err(),
            "input is not a PS-X EXE"
        );
    }
}

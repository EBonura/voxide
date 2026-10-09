//! Generate the synthetic standing and walking/mining renderer benchmarks.
//!
//! PXITAPE1 samples are tied to emulated video time. These normal controller
//! routes measure shipping cadence; they are not an exact simulation-state
//! oracle when two builds render at different rates. Start from a cold disc
//! boot.

use std::path::{Path, PathBuf};

const FRAMES: u32 = 3500;

/// (start tick, button mask) pulses, each held for ten ticks.
const PULSES: [(u32, u16); 12] = [
    (1360, 0x100),
    (1400, 0x800),
    (1440, 0x100),
    (1480, 0x400),
    (1520, 0x100),
    (1900, 0x100),
    (1940, 0x800),
    (1980, 0x100),
    (2020, 0x400),
    (2060, 0x100),
    (2120, 0x8000),
    (2240, 0x8000),
];

/// The bytes of one tape. `actions` adds the walk, look, jump and mine route.
pub fn tape(actions: bool) -> Vec<u8> {
    let mut data = Vec::with_capacity(12 + FRAMES as usize * 6);
    data.extend_from_slice(b"PXITAPE1");
    data.extend_from_slice(&FRAMES.to_le_bytes());
    for tick in 0..FRAMES {
        let mut buttons: u16 = if (700..760).contains(&tick) { 8 } else { 0 };
        let (mut rx, mut ry, lx, mut ly) = (128u8, 128u8, 128u8, 128u8);
        if actions {
            if (900..1320).contains(&tick) {
                ly = 28;
            }
            if (1260..1320).contains(&tick) {
                ry = 188;
            }
            for (start, button) in PULSES {
                if (start..start + 10).contains(&tick) {
                    buttons |= button;
                }
            }
            if (1560..1860).contains(&tick) {
                buttons |= 0x200;
            }
            if (2380..2560).contains(&tick) {
                rx = 213;
            }
        }
        data.extend_from_slice(&buttons.to_le_bytes());
        data.extend_from_slice(&[rx, ry, lx, ly]);
    }
    data
}

/// Write `standing.pxtape` and `actions.pxtape` into `out`, refusing to
/// replace a tape that differs.
pub fn run(out: &Path) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut written = Vec::new();
    for (name, actions) in [("standing", false), ("actions", true)] {
        let path = out.join(format!("{name}.pxtape"));
        let data = tape(actions);
        if let Ok(existing) = std::fs::read(&path) {
            if existing != data {
                return Err(format!(
                    "refusing to replace different tape: {}",
                    path.display()
                ));
            }
        }
        std::fs::write(&path, data).map_err(|e| format!("{}: {e}", path.display()))?;
        written.push(path);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_and_size() {
        for actions in [false, true] {
            let t = tape(actions);
            assert_eq!(&t[..8], b"PXITAPE1");
            assert_eq!(u32::from_le_bytes(t[8..12].try_into().unwrap()), 3500);
            assert_eq!(t.len(), 12 + 3500 * 6);
        }
    }

    #[test]
    fn standing_only_presses_the_one_button() {
        let t = tape(false);
        for tick in 0..3500usize {
            let f = &t[12 + tick * 6..12 + tick * 6 + 6];
            let buttons = u16::from_le_bytes([f[0], f[1]]);
            assert_eq!(buttons, if (700..760).contains(&tick) { 8 } else { 0 });
            assert_eq!(&f[2..], &[128, 128, 128, 128]);
        }
    }

    #[test]
    fn actions_route_has_the_walk_look_and_pulses() {
        let t = tape(true);
        let frame = |tick: usize| &t[12 + tick * 6..12 + tick * 6 + 6];
        assert_eq!(frame(900)[5], 28);
        assert_eq!(frame(1260)[3], 188);
        assert_eq!(u16::from_le_bytes([frame(1360)[0], frame(1360)[1]]), 0x100);
        assert_eq!(u16::from_le_bytes([frame(1370)[0], frame(1370)[1]]), 0);
        assert_eq!(u16::from_le_bytes([frame(1600)[0], frame(1600)[1]]), 0x200);
        assert_eq!(frame(2400)[2], 213);
    }
}

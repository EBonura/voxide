//! Summarise a PSoXide `--profile-log` CSV in VoXide's own stage names.
//!
//! The emulator labels telemetry stages with the id table it ships, which is
//! hl-psx's vocabulary, so a raw CSV column reads `box_prop_debris` where VoXide
//! means "world face render". This maps them back and prints mean cycles per
//! frame.
//!
//! VoXide's measured two-VBlank period is 1,142,472 profiler bus cycles. Use
//! that observed cadence rather than deriving it from the nominal CPU clock:
//! the latter made a perfectly locked run print as 2.02 VBlanks / 29.7fps.

use std::collections::HashMap;

/// Profiler bus cycles in VoXide's two-VBlank (30 fps) period.
pub const FRAME_30_CYCLES: f64 = 1_142_472.0;

/// Emulator CSV column and VoXide's label, from the `ST_*` stage constants in
/// `game/src/main.rs` and the emulator's id table.
const STAGES: [(&str, &str); 13] = [
    ("frame_cycles", "frame total"),
    ("cell_collect", "loop body (minus vsync)"),
    ("render", "  render"),
    ("box_prop_debris", "    world faces"),
    ("update_actor", "      face loop"),
    ("box_prop_shards", "    mobs"),
    ("room_surface_cache", "    streaming total"),
    ("cd_world_pack_stream", "      generation"),
    ("sim_solve", "  sky"),
    ("image_cards", "  tail (HUD, particles)"),
    ("cell_depth", "  pad poll"),
    ("update_window", "  gpu drain"),
    ("cell_lookup", "  buffer swap"),
];

/// The usage text printed when no file is given.
pub const USAGE: &str = "Usage: profile-report <profile.csv>\n";

/// Format like Python's `{:,.0f}`.
fn grouped(value: f64) -> String {
    let digits = format!("{value:.0}");
    let (sign, digits) = match digits.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", digits.as_str()),
    };
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    format!("{sign}{out}")
}

/// A CSV cell as Python's `float(cell or 0)`.
fn cell(row: &HashMap<&str, &str>, column: &str) -> Result<f64, String> {
    match row.get(column).copied().unwrap_or("") {
        "" => Ok(0.0),
        text => text
            .trim()
            .parse::<f64>()
            .map_err(|_| format!("could not convert string to float: {text:?}")),
    }
}

/// Summarise `csv`. Returns the exit status and the text to print.
pub fn report(csv: &str) -> Result<(i32, String), String> {
    let mut lines = csv.lines().filter(|l| !l.is_empty());
    let header: Vec<&str> = lines.next().unwrap_or("").split(',').collect();
    // Stopping at frame_begin leaves a terminal zero-cycle row. It has
    // rendered nothing and must not count as a free successful frame.
    let mut rows = Vec::new();
    for line in lines {
        let row: HashMap<&str, &str> = header.iter().copied().zip(line.split(',')).collect();
        if cell(&row, "frame_cycles")? > 0.0 {
            rows.push(row);
        }
    }
    if rows.is_empty() {
        return Ok((
            1,
            "no frames recorded -- did the run press START? \
             frame_begin only runs in the gameplay loop.\n"
                .to_string(),
        ));
    }
    let n = rows.len() as f64;
    let mean = |column: &str| -> Result<Option<f64>, String> {
        if !header.contains(&column) {
            return Ok(None);
        }
        let mut sum = 0.0;
        for row in &rows {
            sum += cell(row, column)?;
        }
        Ok(Some(sum / n))
    };
    let total = match mean("frame_cycles")? {
        Some(v) if v != 0.0 => v,
        _ => 1.0,
    };
    // A frame above 1.25 periods cannot still belong to the tight two-VBlank
    // cluster (~1.14247M); it has slipped to at least the three-VBlank cadence.
    // The margin keeps one-off profiler-event jitter from becoming a false miss.
    let mut misses = 0;
    for row in &rows {
        if cell(row, "frame_cycles")? > FRAME_30_CYCLES * 1.25 {
            misses += 1;
        }
    }
    let mut out = String::new();
    out += &format!(
        "completed frames: {}   mean frame: {} cycles ({:.2} 30fps periods, {:.1} fps)\n",
        rows.len(),
        grouped(total),
        total / FRAME_30_CYCLES,
        30.0 * FRAME_30_CYCLES / total
    );
    out += &format!(
        "  30fps deadline misses: {misses} ({:.2}%)\n\n",
        100.0 * f64::from(misses) / n
    );
    for (column, label) in STAGES {
        let Some(v) = mean(column)? else { continue };
        if v < 1.0 {
            continue;
        }
        out += &format!(
            "  {label:<28} {:>10}  {:5.1}%\n",
            grouped(v),
            100.0 * v / total
        );
    }
    Ok((0, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_marker_is_not_a_free_frame_and_generation_uses_stage_25() {
        let (status, output) = report(
            "frame_cycles,cell_collect,cd_room_chunk_load,cd_world_pack_stream\n\
             1142472,700000,900000,100000\n0,0,0,0\n",
        )
        .unwrap();
        assert_eq!(status, 0);
        assert!(output.contains("completed frames: 1"));
        assert!(output.contains("30.0 fps"));
        let generation = output.lines().find(|l| l.contains("generation")).unwrap();
        assert!(generation.contains("100,000"));
        assert!(!generation.contains("900,000"));
    }

    #[test]
    fn unexported_generation_and_no_completed_frames() {
        let (status, output) = report("frame_cycles,cd_room_chunk_load\n1142472,900000\n").unwrap();
        assert_eq!(status, 0);
        assert!(!output.contains("generation"));
        let (status, _) = report("frame_cycles\n0\n").unwrap();
        assert_eq!(status, 1);
    }

    #[test]
    fn groups_thousands() {
        assert_eq!(grouped(0.4), "0");
        assert_eq!(grouped(999.0), "999");
        assert_eq!(grouped(1_142_472.0), "1,142,472");
    }
}

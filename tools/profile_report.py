#!/usr/bin/env python3
"""Summarise a PSoXide --profile-log CSV in VoXide's own stage names.

The emulator labels telemetry stages with the id table it ships, which is
hl-psx's vocabulary, so a raw CSV column reads "box_prop_debris" where VoXide
means "world face render". This maps them back and prints mean cycles per frame.

Usage: profile_report.py <profile.csv>
VoXide's measured two-VBlank period is 1,142,472 profiler bus cycles. Use that
observed cadence rather than deriving it from the nominal CPU clock: the latter
made a perfectly locked run print as 2.02 VBlanks / 29.7fps.
"""
import csv
import sys

FRAME_30_CYCLES = 1_142_472

# VoXide stage id -> (emulator CSV column, our label). Ids come from the ST_*
# constants in game/src/main.rs; column names from the emulator's id table.
STAGES = [
    ("frame_cycles", "frame total", 0),
    ("cell_collect", "loop body (minus vsync)", 1),
    ("render", "  render", 2),
    ("box_prop_debris", "    world faces", 3),
    ("update_actor", "      face loop", 4),
    ("box_prop_shards", "    mobs", 3),
    ("room_surface_cache", "    streaming total", 3),
    ("cd_world_pack_stream", "      generation", 4),
    ("sim_solve", "  sky", 2),
    ("image_cards", "  tail (HUD, particles)", 2),
    ("cell_depth", "  pad poll", 2),
    ("update_window", "  gpu drain", 2),
    ("cell_lookup", "  buffer swap", 2),
]


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    with open(sys.argv[1]) as stream:
        # Stopping at frame_begin leaves a terminal zero-cycle row. It has
        # rendered nothing and must not count as a free successful frame.
        rows = [row for row in csv.DictReader(stream)
                if float(row.get("frame_cycles") or 0) > 0]
    if not rows:
        print("no frames recorded -- did the run press START? "
              "frame_begin only runs in the gameplay loop.")
        return 1
    n = len(rows)

    def mean(col):
        if col not in rows[0]:
            return None
        return sum(float(r[col] or 0) for r in rows) / n

    total = mean("frame_cycles") or 1.0
    # A frame above 1.25 periods cannot still belong to the tight two-VBlank
    # cluster (~1.14247M); it has slipped to at least the three-VBlank cadence.
    # The margin keeps one-off profiler-event jitter from becoming a false miss.
    misses = sum(float(r["frame_cycles"] or 0) > FRAME_30_CYCLES * 1.25 for r in rows)
    print(f"completed frames: {n}   mean frame: {total:,.0f} cycles "
          f"({total / FRAME_30_CYCLES:.2f} 30fps periods, "
          f"{30 * FRAME_30_CYCLES / total:.1f} fps)")
    print(f"  30fps deadline misses: {misses} ({100 * misses / n:.2f}%)\n")
    for col, label, depth in STAGES:
        v = mean(col)
        if v is None or v < 1:
            continue
        print(f"  {label:<28} {v:10,.0f}  {100 * v / total:5.1f}%")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

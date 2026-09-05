# Derive near-face grid steps from their world axes

Every near-face tessellation step is exactly one block along a signed world
axis. The old setup divided all six corner-span coordinates by the merged
width/height and evaluated full three-component dot products for those steps.
Selecting the axis from the face direction gives the exact same step and
camera-plane increment directly. Vertices, UVs, colors, clipping, ordering,
subdivision and all render/streaming capacities are unchanged.

This follows the terrain renderer work merged as 5637e2c. That ordinary build
was verified byte-identical to its accepted artifacts and installed before
this experiment. All comparisons here use 5637e2c and the same SDK 8df242b.

## MIPS and arithmetic proof

The near helper shrinks 7,760 -> 7,524 bytes. Its disassembly contains six fewer
divisions (14 -> 8) and 23 fewer multiply instructions (116 -> 93). These are
static instructions in the function, not an instruction-execution count over
the entire game. The common face-loop body remains 8,184 bytes with a 352-byte
stack frame; BSS still ends at 0x801f1608 in diagnostic and normal builds.
The normal executable has zero load-delay hazards after the standard patch.

`tools/test_near_grid.py` extracts the current face construction, axis selection
and camera-plane expressions directly from the game source and compiles them
on the host. An independent span/division oracle checks all six directions,
all 16x8 packed face dimensions and signed camera rows. All 321,024 grid/row
comparisons pass. The frozen proof source is retained beside this report.

## Fixed-state renderer checks

The three existing fixtures use fixed two-VBlank simulation steps so faster
rendering cannot change the world state. All 233 display checkpoints match:
33 action/menu, 100 high-flight and 100 terrain-following checkpoints. Final
display and full VRAM hashes match in every fixture. The terrain rig looks
down and follows a 6,000-unit square above the surface; the high flight often
looks into sky. Neither is an ordinary locomotion or whole-game 30 FPS proof.

| Fixed-state fixture | Baseline body cycles | Candidate body cycles | Baseline deadline misses | Candidate misses |
| --- | ---: | ---: | ---: | ---: |
| Terrain, 2,998 steady frames | 1,066,725 | 1,046,267 | 844 | 796 |
| Actions/menu, 998 steady frames | 979,757 | 958,596 | 58 | 48 |
| High flight, 2,998 steady frames | 873,637 | 856,964 | 720 | 691 |

Terrain body work falls 1.92%, and face-loop work falls 2.92%. Terminal
zero-cycle markers and the first two completed frames are excluded. A miss
means exceeding 1.25 times the observed two-VBlank period of 1,142,472 cycles.

## Ordinary controller replay

Both 3,500-sample video-bound tapes run from cold boot using default game
features and normal VBlank-derived simulation time. Compare ticks 901-3500,
2,600 ticks or approximately 43.852 bus seconds:

| Route | Baseline flips | Candidate flips | Baseline nominal FPS | Candidate nominal FPS |
| --- | ---: | ---: | ---: | ---: |
| Standing | 1,300 | 1,300 | 30.000 | 30.000 |
| Actions | 867 | 871 | 20.008 | 20.100 |

All eight complete standing windows remain at two VBlanks. The mining window
changes 19.0 -> 19.2 nominal FPS. Actual measured bus-time rates are about
29.645 standing and 19.771 -> 19.862 on actions. Both denominators and cold-boot
totals are retained in shipping-results.json. The frozen frontend's video
period is 571,236 bus cycles; nominal cadence is not a hardware FPS claim.

All eight standing screenshots are exact. Two action screenshots are exact;
the six changed pairs were inspected, with differences in animation/input
phase and no observed new geometry failure. Video-bound tapes can sample
different inputs after a cadence change, so exact renderer parity is supplied
by the fixed-state fixtures, not inferred from this normal route. Root stack
sampling remains at min SP 0x801fa8b0 / 22,336-byte depth. Packet occupancy and
unexported generation timings are unavailable, not zero.

View distance remains 16 blocks for tops and 14 for sides with the same chunk
configuration. The moving 30 FPS / larger-terrain objective remains unfinished.

## Removed experiments

Passing the cull's centre depth to skip redundant near RTPT/RTPS projections
grew the hot-loop frame from 352 to 368 bytes and body from 8,184 to 8,680 bytes.
Standing useful body cycles increased 1,080,402 -> 1,121,640 (+3.82%). Removed.

An early near-helper exit for opaque plates entirely owned by the bounded
near-block shell increased body cycles to 1,116,319 (+3.33%). Removed before
the axis-step trial. A looser near-depth-only cull was rejected during source
review, without a replay: the shell owns only a 3x3x4 footprint and excludes
transparent terrain. Depth alone does not prove ownership.

## Reproduction

Run `python3 -m unittest discover -s tools -p test_near_grid.py` from the root.
Build ordinary discs with `make disc GAMES_DIR=/tmp/vox-near-grid-library`;
the destination override is required for experiments because make installs.
The existing tape generator and terrain-renderer report describe the fixtures.
Exact commands, source snapshots, logs, map/disassembly files and images are
retained under /tmp/astra-vox-near-projection-20260905. Normal and diagnostic
artifact identities and compact results are committed here.

# Terrain renderer: reduce spills and repeated depth scans

Two changes improve terrain rendering without changing geometry, culling,
lighting, texture resolution, subdivision, packet limits or view distance:

* Keep `emit_near_face` out of the common face loop. Its tessellation
  temporaries otherwise force a 1,016-byte stack frame on that loop. Outlining
  reduces the common frame to 352 bytes and its MIPS body from 15,080 to 8,184
  bytes. The helper is 7,760 bytes; this is a register-pressure improvement,
  not a claim that outlining alone makes all code smaller.
* Select a clipped polygon's fan pivot from its minimum/maximum depths. The
  farthest depth from a candidate must be one of those extrema. Two linear
  scans replace the quadratic all-pairs search, preserving the first minimum
  on ties and therefore the exact same fan and draw order. The clipper shrinks
  from 12,604 to 11,292 MIPS bytes.

VoXide now uses the same immutable SDK revision `8df242b` as the measured HL
and Quake builds. The pin manifest and lockfile previously named different
revisions; both are now consistent. All performance comparisons below use that
same SDK on both sides, so they do not separately quantify the SDK update.

This advances the terrain/30 FPS goal but does not complete it. Moving gameplay
still falls below 30, and the shipping distance remains 16 blocks for tops,
14 for sides, with the existing chunk configuration.

## Normal shipping builds

The baseline is upstream `1d4816c` with SDK `8df242b`. Each image uses ordinary
default features, real controller polling and the game's normal VBlank-derived
simulation delta. The two generated `PXITAPE1` routes run from cold boot for
3,500 video samples. `tools/make_renderer_tapes.py` reproduces both frozen tapes
byte for byte. The action route walks, looks down, places blocks, selects items,
mines, opens/closes inventory and turns before an idle tail. It is a synthetic
test, not a user recording or exhaustive gameplay validation.

Compare route ticks 901 through 3500: 2,600 ticks, about 43.852 emulated seconds.

| Build | Standing flips | Standing nominal cadence | Action flips | Action nominal cadence |
| --- | ---: | ---: | ---: | ---: |
| Baseline | 867 | 20.008 FPS | 860 | 19.846 FPS |
| Outline only | 1,300 | 30.000 FPS | 865 | 19.962 FPS |
| Outline + linear pivot | 1,300 | 30.000 FPS | 867 | 20.008 FPS |

All eight complete 300-tick standing windows hold the two-VBlank cadence in the
accepted build. The action route's mining window improves from 18.4 to 19.0
nominal FPS, but most action windows still hold only 20. The accepted measured
bus-time rates are 29.645 FPS standing and 19.771 FPS on actions, against
19.771 and 19.611 respectively at baseline. Nominal cadence uses 60 video ticks
per second; the frozen frontend's observed tick period is 571,236 bus cycles.
Do not present nominal cadence as a measured original-console result.

`shipping-results.json` retains every denominator, the cold-boot flips/bus time,
all 300-tick windows, complete-tape endpoints and stack samples. Different
render rates can sample these video-bound inputs differently. Exact-state
renderer comparisons are separate fixtures below.

## Attribution, memory and the tradeoff

On the outlined shipping action route, PC sampling attributes 32.9% of work to
the common face loop, 9.8% to near-face tessellation and 7.7% to clipped-cell
emission. Presentation/waiting is 20.5%. The generated clipper contains the
repeated depth-difference scans; this was not selected from source appearance
alone. The next work should address the common face loop and repeated near
projection, rather than assume that world generation is the primary cost.

For 598 completed steady standing diagnostic frames, useful frame-body cycles
fall from 1,202,410 to 1,105,670 with outlining and to 1,080,402 with the linear
pivot. Face-loop cycles fall from 864,623 to 804,477 to 780,219. Crossing from
three to two VBlanks also reduces simulation catch-up work; not all frame-body
savings are attributable directly to the changed instructions.

The ground-following fixed-state tour has a real tradeoff: useful body cycles
are 1,213,038 at baseline, 1,050,593 outlined, and 1,066,725 with the linear
pivot. The pivot trial is 1.54% slower than outlining alone on this route, while
the combined result remains 12.06% faster than the original. Two-VBlank deadline
misses are 1,539, 810 and 844 of 2,998 steady frames respectively. The accepted
combination retains the action/standing gains and recovers the outlining RAM
cost; it is not a uniform speedup over the outline-only intermediate.

Outlining moves BSS from `0x801f1608` to `0x801f1e08`; the smaller clipper returns
it to `0x801f1608`. The common loop remains a 352-byte frame. The normal replay
stack profile is retained separately; reducing one function frame does not
prove a reduction in the program's overall maximum stack use. Post-link hazard
patch/scan passes on all tested executables. Render/streaming capacities remain
unchanged. Packet occupancy and unexported generation timings are unknown,
represented as null rather than zero.

## Exact-state visual checks

Diagnostic fixtures set simulation delta to two VBlanks so renderer speed
cannot change the simulated state being compared. They are visual oracles,
not shipping performance or gameplay-speed evidence:

| Fixture | Guest checkpoints | Result for both successive A/B comparisons |
| --- | ---: | --- |
| Walking, placement, mining and menus | 33 | All display hashes equal |
| Existing high flight / streaming route | 100 | All display hashes equal |
| Ground-following camera square | 100 | All display hashes equal |

Final display and full VRAM hashes also match in every fixture. The camera tour
traverses a 6,000-unit square at eight units per frame, maintains nine blocks
above the sampled surface, looks down 45 degrees and restores health. Its
checkpoints show terrain, water, canopy and rain. It is a camera/renderer rig,
not a collision or survival test. The earlier high-flight route often looks
into empty sky; a discarded low flight died in canopy; the initial shallow
camera tour gave poor ground coverage. Those were not used as proof of a
complete visible-terrain tour. Diagnostic patches are provided alongside this
report and must never be installed as normal builds. Apply one in an isolated
checkout with `git apply --unidiff-zero <fixture.patch>`; the zero-context
patches target the exact source identity in this report.

The actual new Rust pivot block was also tested against an independent
quadratic reference: 488,280 exhaustive ordered inputs, 100,000 randomized
inputs through twelve corners, and three explicit extreme/tie cases all match.
The scratch proof source and result hashes are recorded in `identities.json`.

## Distance trials and reporting fixes

An 18-block short trial misses 422 of 598 steady deadlines. A 17-block trial
with outlining passes the short run but drops to 23.4 FPS in the last full
standing window of the longer normal replay. This is why short spawn captures
are insufficient to establish sustained cadence.

With the linear pivot, 17 blocks does hold all standing windows at 30 nominal
FPS. Its action route still averages 19.985 FPS, so it is a promising distance
candidate, not proof of terrain at 30 FPS during movement. Shipping remains at
16 while the larger moving bottleneck is addressed.

`profile_report.py` now excludes terminal zero-cycle frame markers and uses
the actual generation stage (25) instead of the unrelated stage 26. Missing
generation telemetry is omitted. Two regression tests pass. This removes the
false extra completed frame / 30.1 FPS result from stopped diagnostic captures.

Reproduce the normal input routes with:

```sh
python3 tools/make_renderer_tapes.py --out /tmp/vox-renderer-tapes
make disc GAMES_DIR=/tmp/vox-renderer-library
```

`make disc` installs as part of its operation, so always override `GAMES_DIR`
for experimental builds. Build/replay commands, maps, captures and full logs
remain in `/tmp/astra-vox-distance-20260905`; compact identities and results are
committed here. No whole-game 30 FPS, larger shipping distance, or completion
of the broader cross-game performance/feature goal is claimed.

The ordinary `make disc` build hydrates SDK8df242b through the committed pin
and produces an EXE, BIN and CUE byte-identical to the frozen accepted shipping
artifacts. The installed scratch-library copy is an ordinary build, with all
diagnostic camera and simulation changes absent. See `ordinary-build.json`.

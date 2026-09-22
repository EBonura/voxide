# VoXide -- a tiny Minecraft-like voxel sandbox for PlayStation 1, built on the
# PSoXide Rust SDK (imported into .psoxide/ from components.lock.json).

ROOT     := $(CURDIR)
GAME     := $(ROOT)/game
# The locked PSoXide components are imported into .psoxide by
# tools/bootstrap-components.py. The crate paths in game/Cargo.toml point
# there, so this builds from a clean clone with no sibling checkout: Cargo
# resolves path dependencies relative to the manifest and no variable could
# move them.
PSOXIDE  ?= $(ROOT)/.psoxide
MKISOPSX := $(PSOXIDE)/tools/mkisopsx
TARGET   := mipsel-sony-psx
DIST     := $(ROOT)/dist
CAPTURE_DIR ?= $(ROOT)/captures

GAMES_DIR ?= $(HOME)/Downloads/ps1 games
GAME_NAME ?= VoXide
EXE      := $(GAME)/target/$(TARGET)/release/voxide.exe
FRONTEND ?= frontend
PSOXIDE_LAUNCH = "$(FRONTEND)" launch
PSOXIDE_SMOKE_STEPS ?= 70000000
PSOXIDE_PROFILE_STEPS ?= 900000000
PSOXIDE_START_PULSE ?= 0x0008@700+60

.DEFAULT_GOAL := build
.PHONY: help psoxide build compile disc install release run smoke profile pgo clean

help:
	@echo "VoXide targets:"
	@echo "  make psoxide    - import the components.lock.json revisions into .psoxide"
	@echo "  make            - build + install into the PSoXide game library"
	@echo "  make compile    - build PSX-EXE only -> $(EXE)"
	@echo "  make disc       - compile + pack dist/voxide.cue/.bin"
	@echo "  make install    - install into $(GAMES_DIR)"
	@echo "  make smoke      - boot the disc headlessly through PSoXide and capture PPM"
	@echo "  make profile    - telemetry build + per-frame stage-cycle CSV report"
	@echo "  make pgo TAPE=x - profile-guided build from an emulator replay of tape x"
	@echo "  make clean      - remove build output"

# Which PSoXide this is built against. components.lock.json pins the SDK,
# editor/engine and emulator-library revisions separately, the same lock the
# rest of the game fleet uses, and tools/bootstrap-components.py imports them
# into .psoxide so the crate paths and the linker script resolve. An unchanged
# lock is verified against its receipt and not fetched again.
#
# PSOXIDE_FROM=/path/to/tree overrides the lock with a working tree, which is
# how the demo disc puts every program it presses on one SDK.
PSOXIDE_FROM ?=
psoxide:
	@if [ -n "$(PSOXIDE_FROM)" ]; then \
		cargo run -q --manifest-path $(PSOXIDE_FROM)/tools/psoxide-link/Cargo.toml -- \
			--from "$(PSOXIDE_FROM)" --into $(PSOXIDE); \
	else \
		python3 $(ROOT)/tools/bootstrap-components.py --root $(PSOXIDE) --lock $(ROOT)/components.lock.json; \
	fi

compile: psoxide
	cd $(GAME) && PSOXIDE="$(PSOXIDE)" cargo build --release
	python3 $(PSOXIDE)/tools/hazard_patch.py $(EXE)
	@echo "EXE -> $(EXE)"

# disc always installs into the game library too, so EVERY build (disc, smoke,
# install, default) lands in $(GAMES_DIR) and the latest is always testable there.
disc: compile
	@mkdir -p $(DIST)
	cd $(MKISOPSX) && cargo run --release -- \
		--exe $(EXE) \
		--out $(DIST)/voxide.bin \
		--volume VOXIDE \
		--world-pack-extra-dir $(ROOT)/assets/sfx/pak
	@echo "DISC -> $(DIST)/voxide.cue"
	@mkdir -p "$(GAMES_DIR)/$(GAME_NAME)"
	@cp "$(DIST)/voxide.bin" "$(GAMES_DIR)/$(GAME_NAME)/$(GAME_NAME).bin"
	@printf 'FILE "%s.bin" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n' \
		"$(GAME_NAME)" > "$(GAMES_DIR)/$(GAME_NAME)/$(GAME_NAME).cue"
	@echo "INSTALLED -> $(GAMES_DIR)/$(GAME_NAME)/"

install: disc

run: install

# Build with PSoXide guest telemetry and write per-frame profile CSVs.
#
# NOTE the --pad-pulses: telemetry::frame_begin only runs in the GAMEPLAY loop,
# so without pressing START the game sits on the title screen and the profiler
# records ZERO frames (silently -- you get a CSV with only a header). 0x0008 is
# START; the tick is after world gen finishes.
profile: psoxide
	cd $(GAME) && PSOXIDE="$(PSOXIDE)" cargo build --release --features emulator-telemetry
	python3 $(PSOXIDE)/tools/hazard_patch.py $(EXE)
	cd $(MKISOPSX) && cargo run --release -- --exe $(EXE) --out $(DIST)/voxide.bin --volume VOXIDE --world-pack-extra-dir $(ROOT)/assets/sfx/pak
	@mkdir -p $(CAPTURE_DIR)
	$(PSOXIDE_LAUNCH) \
		--path $(DIST)/voxide.cue \
		--embedded-playtest \
		--steps $(PSOXIDE_PROFILE_STEPS) \
		--pad-pulses '$(PSOXIDE_START_PULSE)' \
		--profile-log $(CAPTURE_DIR)/voxide-profile.csv \
		--counter-log $(CAPTURE_DIR)/voxide-counter.csv \
		--dump-guest-profile \
		--dump-hw $(CAPTURE_DIR)/voxide-profile.ppm
	@echo "PROFILE -> $(CAPTURE_DIR)/voxide-profile.csv (per-frame stage cycles)"
	@python3 tools/profile_report.py $(CAPTURE_DIR)/voxide-profile.csv

# Profile-guided build from the emulator's own PC samples (psoxide-pgo; same
# recipe as hl-psx `hl-build pgo`). A build with profiling line tables replays
# TAPE, psoxide-pgo maps the PC histogram through the matching ELF's DWARF
# into an LLVM sample profile, and the game is rebuilt with it. The result is
# $(EXE) and $(DIST)/voxide.cue; nothing is installed into $(GAMES_DIR).
# Profile names carry crate hashes that depend on the checkout path, so the
# profile is regenerated here, never committed.
#   make pgo TAPE=route.pxtape FRONTEND=/path/to/frontend
#
# A/B builds with `--features lockstep` (identical state at every poll) need the
# profile rebound onto their own names (psoxide-pgo `portable` + `rebind`):
# cargo features enter -C metadata, so every voxide symbol of a lockstep build
# is named differently from the shipping build this target profiles, and LLVM
# silently matches none of them.
#
# The two LLVM options below are what made the profile a win. LLVM's default
# 3000-point inline budget for profile-hot calls grew the image by ~30 KB
# (inlining box emitters into render_mobs, collision into main) and, with
# `-sample-profile-use-profi` added, overflowed RAM by 4 KB in the telemetry
# build; 500 keeps the hot paths inside the 4 KB I-cache. Profile inference
# (profi) fills in the block counts of code the samples cannot place (line-0
# code, 15% of samples in the face loop). Measured 2026-09-22 at 9502a58,
# loop body per frame, plain -> profiled: 973,679 -> 954,312 on the training
# route, 777,566 -> 760,235 on an unseen one; with LLVM's defaults it was
# 965,222 and 776,213.
TAPE ?=
PGO_DIR := $(ROOT)/.pgo
# `--config` appends to game/.cargo/config.toml's flags; RUSTFLAGS would replace them.
PGO_COLLECT := "-Cdebuginfo=1","-Zdebug-info-for-profiling","-Cstrip=none"
PGO_USE := $(PGO_COLLECT),"-Zprofile-sample-use=$(PGO_DIR)/voxide.prof", \
	"-Cllvm-args=-hot-callsite-threshold=500","-Cllvm-args=-sample-profile-use-profi"
CARGO_PSX = cd $(GAME) && PSOXIDE="$(PSOXIDE)" cargo build --release --config
PACK_EXE = cd $(MKISOPSX) && cargo run --release -- --exe $(EXE) --volume VOXIDE \
	--world-pack-extra-dir $(ROOT)/assets/sfx/pak --out
pgo: psoxide
	@test -n "$(TAPE)" || { echo "usage: make pgo TAPE=route.pxtape [FRONTEND=frontend]"; exit 1; }
	rm -rf $(PGO_DIR) && mkdir -p $(PGO_DIR) $(DIST)
	$(CARGO_PSX) 'target.$(TARGET).rustflags=[$(PGO_COLLECT)]'
	python3 $(PSOXIDE)/tools/hazard_patch.py $(EXE)
	$(PACK_EXE) $(PGO_DIR)/collect.bin
	cd $(GAME) && VOXIDE_LINK_ELF=1 PSOXIDE="$(PSOXIDE)" cargo build --release \
		--config 'target.$(TARGET).rustflags=[$(PGO_COLLECT)]'
	cp $(EXE) $(PGO_DIR)/voxide.elf
	$(PSOXIDE_LAUNCH) --path $(PGO_DIR)/collect.cue --embedded-playtest \
		--steps 40000000000 --input-tape "$(TAPE)" \
		--pc-sample-log $(PGO_DIR)/pc.csv --pc-sample-instructions 61
	cargo run -q --release --manifest-path $(PSOXIDE)/tools/psoxide-pgo/Cargo.toml -- \
		$(PGO_DIR)/voxide.elf $(PGO_DIR)/pc.csv $(PGO_DIR)/voxide.prof
	rm -f $(PGO_DIR)/pc.csv $(PGO_DIR)/collect.bin $(PGO_DIR)/collect.cue
	$(CARGO_PSX) 'target.$(TARGET).rustflags=[$(PGO_USE)]'
	python3 $(PSOXIDE)/tools/hazard_patch.py $(EXE)
	$(PACK_EXE) $(DIST)/voxide.bin
	@echo "PGO EXE -> $(EXE)  DISC -> $(DIST)/voxide.cue  PROFILE -> $(PGO_DIR)/voxide.prof"

smoke: disc
	@mkdir -p $(CAPTURE_DIR)
	$(PSOXIDE_LAUNCH) \
		--path $(DIST)/voxide.cue \
		--embedded-playtest \
		--steps $(PSOXIDE_SMOKE_STEPS) \
		--dump-hw $(CAPTURE_DIR)/voxide-hw.ppm \
		--dump-display $(CAPTURE_DIR)/voxide-display.ppm \
		--dump-hash
	@echo "SMOKE -> $(CAPTURE_DIR)/voxide-display.ppm"

# clean leaves $(CAPTURE_DIR) alone: captures/ is local run history, not build output.
clean:
	rm -rf $(DIST) $(GAME)/target

# Stage the itch.io payload. CI (deploy.yml) pushes release/ via butler
# whenever it changes on main, versioned from the VERSION file.
release: disc
	@mkdir -p $(ROOT)/release
	cp $(DIST)/voxide.bin $(DIST)/voxide.cue $(ROOT)/release/
	@echo "RELEASE -> $(ROOT)/release (commit + push to deploy)"

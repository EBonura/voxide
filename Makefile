# VoXide -- a tiny Minecraft-like voxel sandbox for PlayStation 1, built on the
# sibling PSoXide Rust SDK checkout.

ROOT     := $(CURDIR)
GAME     := $(ROOT)/game
# The SDK is hydrated into .psoxide by psoxide-link, from the pin in
# psoxide-pin/. The crate paths in game/Cargo.toml point there, so this builds
# from a clean clone with no sibling checkout -- which the old layout required
# outright, since Cargo resolves path dependencies relative to the manifest and
# no variable could move them.
PSOXIDE  ?= $(ROOT)/.psoxide
MKISOPSX := $(PSOXIDE)/tools/mkisopsx
TARGET   := mipsel-sony-psx
DIST     := $(ROOT)/dist
CAPTURE_DIR ?= $(ROOT)/captures

GAMES_DIR ?= $(HOME)/Downloads/ps1 games
GAME_NAME ?= VoXide
EXE      := $(GAME)/target/$(TARGET)/release/voxide.exe
PSOXIDE_LAUNCH = cd $(PSOXIDE)/emu && cargo run -p frontend --release -- launch
PSOXIDE_SMOKE_STEPS ?= 70000000
PSOXIDE_PROFILE_STEPS ?= 900000000
PSOXIDE_START_PULSE ?= 0x0008@700+60

.DEFAULT_GOAL := build
.PHONY: help psoxide build compile disc install run smoke profile clean

help:
	@echo "VoXide targets:"
	@echo "  make psoxide-check - verify the sibling PSoXide checkout"
	@echo "  make            - build + install into the PSoXide game library"
	@echo "  make compile    - build PSX-EXE only -> $(EXE)"
	@echo "  make disc       - compile + pack dist/voxide.cue/.bin"
	@echo "  make install    - install into $(GAMES_DIR)"
	@echo "  make smoke      - boot the disc headlessly through PSoXide and capture PPM"
	@echo "  make profile    - telemetry build + per-frame stage-cycle CSV report"
	@echo "  make clean      - remove build output"

# Which PSoXide this is built against. Cargo owns the pin (psoxide-pin/), and
# psoxide-link copies the resolved checkout into .psoxide so the crate paths
# and the linker script resolve. This replaces a check that could only tell you
# to go and clone a sibling checkout by hand.
#
# PSOXIDE_FROM=/path/to/tree overrides the pin with a working tree, which is
# how the demo disc puts every program it presses on one SDK.
PSOXIDE_FROM ?=
psoxide:
	@if [ -n "$(PSOXIDE_FROM)" ]; then \
		cargo run -q --manifest-path $(PSOXIDE_FROM)/tools/psoxide-link/Cargo.toml -- \
			--from "$(PSOXIDE_FROM)" --into $(PSOXIDE); \
	else \
		cargo run -q --manifest-path $(ROOT)/psoxide-pin/Cargo.toml -- $(PSOXIDE); \
	fi

compile: psoxide
	cd $(GAME) && PSOXIDE="$(PSOXIDE)" cargo build --release
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
profile: psoxide-check
	cd $(GAME) && PSOXIDE="$(PSOXIDE)" cargo build --release --features emulator-telemetry
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

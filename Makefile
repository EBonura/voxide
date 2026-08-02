# VoXide -- a tiny Minecraft-like voxel sandbox for PlayStation 1, built on the
# sibling PSoXide Rust SDK checkout.

ROOT     := $(CURDIR)
GAME     := $(ROOT)/game
# The SDK crates are Cargo PATH dependencies (game/Cargo.toml), and Cargo
# resolves those relative to the manifest -- so the sibling layout is a hard
# requirement, not a default. PSOXIDE only redirects the linker script and the
# host tools; it cannot move the crate paths.
SIBLING  := $(abspath $(ROOT)/../PSoXide)
PSOXIDE  ?= $(SIBLING)
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
.PHONY: help psoxide-check build compile disc install run smoke profile clean

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

psoxide-check:
	@test -f "$(SIBLING)/sdk/psoxide.ld" || { \
		echo ""; \
		echo "PSoXide SDK not found at $(SIBLING)"; \
		echo ""; \
		echo "VoXide builds against the PSoXide SDK as Cargo path dependencies,"; \
		echo "so the two checkouts must sit side by side:"; \
		echo ""; \
		echo "    git clone https://github.com/EBonura/PSoXide.git"; \
		echo "    git clone https://github.com/EBonura/voxide.git"; \
		echo "    cd voxide && make"; \
		echo ""; \
		exit 1; \
	}
	@echo "PSoXide -> $(PSOXIDE)"

build: install
	@echo "build -> live in PSoXide ($(GAMES_DIR)/$(GAME_NAME))"

compile: psoxide-check
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

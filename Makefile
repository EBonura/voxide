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
.PHONY: help psoxide build compile pack disc install release run smoke profile \
	pgo-collect pgo-order pgo-choose clean

help:
	@echo "VoXide targets:"
	@echo "  make psoxide    - import the components.lock.json revisions into .psoxide"
	@echo "  make            - build + install into the PSoXide game library"
	@echo "  make compile    - build PSX-EXE only -> $(EXE) (PGO_VARIANT=off for no PGO)"
	@echo "  make disc       - compile + pack dist/voxide.cue/.bin"
	@echo "  make install    - install into $(GAMES_DIR)"
	@echo "  make smoke      - boot the disc headlessly through PSoXide and capture PPM"
	@echo "  make profile    - telemetry build + per-frame stage-cycle CSV report"
	@echo "  make pgo-collect FRONTEND=x - regenerate the committed PGO profile"
	@echo "  make pgo-choose FRONTEND=x  - build and gate every PGO variant"
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
		cargo run -q --manifest-path "$(PSOXIDE_FROM)/tools/psoxide-link/Cargo.toml" -- \
			--from "$(PSOXIDE_FROM)" --into "$(PSOXIDE)"; \
	else \
		python3 "$(ROOT)/tools/bootstrap-components.py" --root "$(PSOXIDE)" --lock "$(ROOT)/components.lock.json"; \
	fi

# Profile-guided optimisation through the SDK's shared driver
# ($(PSOXIDE)/tools/psoxide-pgo/README.md). pgo/voxide.prof is committed and
# portable (its names carry no checkout-path or feature hashes), so every build
# applies it with no emulator: `make compile`, `make disc`, the demo disc.
# PGO_VARIANT is the winner of `make pgo-choose`; PGO_VARIANT=off builds the
# plain image. Either way the driver runs the hazard patcher and scanner and
# stops on a failure, and the exe lands at $(EXE) as before.
#
# The host tools (psoxide-pgo, mkisopsx) build in .psoxide's Cargo workspace
# against the Cargo.lock imported from the editor pin. --locked keeps a host
# build from rewriting that imported file, which the next `make psoxide` would
# refuse as an edit.
FEATURES    ?=
GAME_CARGO   = build --release$(if $(strip $(FEATURES)), --features "$(FEATURES)")
PGO          = cargo run -q --release --locked --manifest-path "$(PSOXIDE)/tools/psoxide-pgo/Cargo.toml" --
PGO_PROFILE  = $(ROOT)/pgo/voxide.prof
PGO_VARIANT ?= hot=500
# Layout profile for `+order` variants (make pgo-order); only they read it.
PGO_LAYOUT   = $(ROOT)/pgo/voxide.layout

compile: psoxide
	PSOXIDE="$(PSOXIDE)" $(PGO) apply --crate "$(GAME)" --profile "$(PGO_PROFILE)" \
		--variant "$(PGO_VARIANT)" $(if $(findstring +order,$(PGO_VARIANT)),--layout "$(PGO_LAYOUT)") -- $(GAME_CARGO)
	@echo "EXE -> $(EXE)"

# `make pack PACK_EXE=x PACK_OUT=y.bin` wraps any exe in the game's disc image.
PACK_EXE ?= $(EXE)
PACK_OUT ?= $(DIST)/voxide.bin
pack:
	@mkdir -p "$$(dirname "$(PACK_OUT)")"
	cd "$(MKISOPSX)" && cargo run -q --release --locked -- \
		--exe "$(PACK_EXE)" \
		--out "$(PACK_OUT)" \
		--volume VOXIDE \
		--world-pack-extra-dir "$(ROOT)/assets/sfx/pak"

# disc always installs into the game library too, so EVERY build (disc, smoke,
# install, default) lands in $(GAMES_DIR) and the latest is always testable there.
disc: compile
	@$(MAKE) --no-print-directory pack PACK_EXE="$(EXE)" PACK_OUT="$(DIST)/voxide.bin"
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
profile:
	@$(MAKE) --no-print-directory compile FEATURES=emulator-telemetry
	@$(MAKE) --no-print-directory pack PACK_EXE="$(EXE)" PACK_OUT="$(DIST)/voxide.bin"
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

# Regenerating the profile and picking the variant need the emulator:
#   make pgo-collect FRONTEND=/path/to/frontend   (after gameplay code changes or an SDK repin)
#   make pgo-choose  FRONTEND=/path/to/frontend   (then commit the winner as PGO_VARIANT)
# PGO_LAUNCH_ARGS adds frontend arguments (--launch-arg X per word). Both tapes
# press PLAY at the same point; polls 252..1200 are gameplay on both (the
# world load ends near poll 154 and the first ~100 polls stream in chunks).
# The profile trains on the shipping build and the recorded tape alone, so the
# unseen tape (a different walk, on the sticks) stays a holdout for the gate.
TRAIN_TAPE   = $(ROOT)/pgo/train.pxtape
TRAIN_POLLS  = 252..1200
UNSEEN_TAPE  = $(ROOT)/pgo/unseen.pxtape
UNSEEN_POLLS = 252..1200
PGO_LAUNCH_ARGS ?=
PGO_PACK = '$(MAKE) --no-print-directory -C "$(ROOT)" pack PACK_EXE="$$PSOXIDE_PGO_EXE" PACK_OUT="$$PSOXIDE_PGO_DISC"'
pgo-collect: psoxide
	PSOXIDE="$(PSOXIDE)" $(PGO) collect --crate "$(GAME)" --frontend "$(FRONTEND)" \
		--tape "$(TRAIN_TAPE)" --polls $(TRAIN_POLLS) \
		--pack $(PGO_PACK) --launch-arg --embedded-playtest $(PGO_LAUNCH_ARGS) \
		--out "$(PGO_PROFILE)" -- $(GAME_CARGO)
	@$(MAKE) --no-print-directory compile

# The layout profile an `+order` variant places functions from, collected on
# PGO_BASE_VARIANT with the training tape. Rerun after code, SDK, profile or
# variant changes: apply refuses a stale layout rather than guess. Not in
# PGO_VARIANTS: choose builds with --features lockstep, which a layout taken
# on the shipping build does not bind to, and on the shipping build
# hot=500+order measured the same as hot=500 (2026-09-24, 16/8 trajectory
# samples of unseen/train), while any code change would fail the build until
# the layout was retaken. pgo/voxide.layout is not committed for that reason.
PGO_BASE_VARIANT ?= hot=500
pgo-order: psoxide
	PSOXIDE="$(PSOXIDE)" $(PGO) order --crate "$(GAME)" --frontend "$(FRONTEND)" \
		--tape "$(TRAIN_TAPE)" --polls $(TRAIN_POLLS) \
		--pack $(PGO_PACK) --launch-arg --embedded-playtest $(PGO_LAUNCH_ARGS) \
		--profile "$(PGO_PROFILE)" --variant "$(PGO_BASE_VARIANT)" --out "$(PGO_LAYOUT)" -- $(GAME_CARGO)

# Every variant is built with --features lockstep (one sim step per pad poll),
# so all of them reach the same state at every poll: a display hash that
# differs from the `off` row is a miscompile, and ticks over the gameplay
# window compare state for state. The vram hash can differ by which of the two
# framebuffers holds the frame (loading screens flip on wall time). The last
# build is a lockstep exe, so this rebuilds the shipping one at the end.
PGO_VARIANTS = off default hot=500 hot=500+profi accurate+hot=500
PGO_MEASURE  = "$$PSOXIDE_PGO" measure --frontend "$(FRONTEND)" --image "$$PSOXIDE_PGO_IMAGE" \
	--launch-arg --embedded-playtest $(PGO_LAUNCH_ARGS)
pgo-choose: psoxide
	PSOXIDE="$(PSOXIDE)" $(PGO) choose --crate "$(GAME)" --profile "$(PGO_PROFILE)" \
		$(foreach v,$(PGO_VARIANTS),--variant $(v)) --pack $(PGO_PACK) \
		--gate '$(PGO_MEASURE) --tape "$(TRAIN_TAPE)" --polls $(TRAIN_POLLS) --name train \
			&& $(PGO_MEASURE) --tape "$(UNSEEN_TAPE)" --polls $(UNSEEN_POLLS) --name unseen' \
		-- build --release --features lockstep
	@$(MAKE) --no-print-directory compile

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

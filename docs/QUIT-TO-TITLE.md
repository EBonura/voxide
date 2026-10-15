# SAVE AND QUIT TO TITLE: what is reset

main() runs the title screen and the world after it as one pass of a loop.
A quit saves (`save::begin`, a card frame per game frame), waits for the GPU's
last frame, then `quit_to_title` runs and the next pass starts at the title:
`main_menu`, `music::enter_world` on PLAY, `reset_game_state`, `spawn_player`,
`mob::populate`, exactly as after a boot.

Statics cannot be re-initialised from the executable (no loader runs), so the
reset is explicit: `reset_session` in main.rs calls one reset per owner, and
`tests/settings-menu-quit-state.toml` reads a world summary from the settings
receipt (VOXIDE_SETTINGS_STATE words 11 to 19) to prove a dirtied world leaves
nothing behind. Every static in the game was read; this is the list.

## Reset (to a cold boot's value)

| State | Owner | How |
|---|---|---|
| Chunks, chunk pool owners, half-done generation and mesh jobs | world.rs | `prepare_new_world` (the NEW WORLD path), new seed from the frame count |
| Other dimension | world.rs | quit leaves through the overworld first (`mob::leave_dimension`, `enter_dimension`) |
| Fluid, fire, blast queues; fire list | world.rs | `world::reset_session` (`fluid_reset`, `blast_reset`) |
| Queued and half-done edit remeshes, their change flags | world.rs | `world::reset_session` |
| Fire generator, rain flag | world.rs | `world::reset_session` |
| Mobs, arrows, knockback, dragon slain | mob.rs | `mob::reset_session` (`clear`, `reset`) |
| Wolves parked in other dimensions | mob.rs | `mob::reset_session` (was not cleared by `clear`) |
| Pack target, line-of-sight cache, hazard damage, spawn timer, generator, XP drops, death list, lure, spawn alternation, burn tick, rain flag | mob.rs | `mob::reset_session` |
| Primed TNT, pending TNT, kick | tnt.rs | `tnt::reset_session` (`reset`) |
| Fuse scatter generator, lost ticks, ground flags, player position | tnt.rs | `tnt::reset_session` |
| Weather flags, timers, queued values, bolts, flash | weather.rs | `weather::reset` (from `reset_game_state`) |
| Rain loop volume | sfx.rs | `sfx::reset_weather` (the voice kept playing at its last level) |
| Inventory, hotbar, selection, edit log, chests, furnaces, crops, saplings | main.rs | `reset_game_state` |
| Furnace XP fractions, pending XP | main.rs | `reset_session` |
| Tutorial progress | main.rs | `reset_game_state` (`tut_reset`) and `TUT_PREV` |
| Cheat flags: god mode, coordinates readout, the time, weather and spawn selections, message | cheat.rs | `cheat::reset` (it left the readout and the selections) |
| Spare armour | equip.rs | `equip::reset` (was not cleared anywhere) |
| 2x2 grid cells | grid.rs | `grid::reset` |
| Carried stack, inventory and container cursors | inv.rs | the screens' own open functions, run once (`inventory_open`, `inventory_open_grid`, `container_open`) |
| Drops, blast kick | main.rs | `clear_world_entities`, and the drop arrays' fly, age, delay and rest fields |
| Particles | main.rs | `PART_LIFE` cleared |
| Crafting tab and filter, at-bench flag, menu repeat timers, held-use and jump timers | main.rs | `reset_session` |
| Equip toast, sleep prompt, footstep state, wet edge, HUD name popup, shown off hand and tool wear | main.rs | `reset_session` |
| Sim tick (blast seeds, animation clock) | main.rs | `reset_session` sets 0, as at a cold boot |
| The world clock, frame counter, game clock, player, hotbar timers, mining state, portal dwell | main() locals | declared inside the loop pass, so new each pass |
| Pause-menu page, every cursor, the cheats cursor, quit flags, menu message | menus.rs | `menus::reset_session` |
| Music: playing piece, delay, drive | music.rs | `music::enter_title` (called by `main_menu`: stops a playing piece, new generator, the title's delay), then `enter_world` on PLAY |
| Respawn point, world spawn | main.rs | `quit_to_title` sets them from the new seed |

## Kept on purpose

- The settings and their save state (`SETTINGS_*`, the sliders, the profile): a
  player's, not a world's. SAVE SETTINGS still decides what reaches the card.
- The tutorial switch: a preference like the settings.
- `save.rs` card state (which of the two file names holds the newest save, the
  blank-card question): it describes the card, not the world. The job is idle
  when a quit starts (`save::busy`).
- What boot builds once: textures and block tables, the sound bank, the music
  player and its pieces, the font, the reciprocal table, the order of the
  boot-time statics in world/rle.rs (`SELF_TESTED`), the sky and material
  tables. `main()` does not run them again.
- Per-frame render state: the ordering tables, quad pools, UI packet pool, the
  clip buffer, camera rows. Rebuilt every frame; the render arena index and the
  one-frame GPU prime start over with the pass (`render_in_flight`).
- Gate receipts (`VOXIDE_GATE_STATE`, the settings receipt) mirror the new
  world next frame.
- Function-local statics (smoothing and noise generators in weather.rs and
  main.rs, lab flags): cosmetic or feature-gated seeds with no world state.

## Failure paths

A save that does not finish never quits (Java does not either): the reason stays
on the line under the menu and X tries again. NO MEMORY CARD, MEMORY CARD FULL and
CARD REMOVED each keep the player in the game menu with the world running. A blank
card asks first; the second X formats, saves and quits. Journeys:
settings-menu-quit-nocard, -full (tests/fixtures/full.mcd), -removed (save-pull
lab disc), -blank, and the state journey.

Not provable here: the real console. A power cut (the gate's `power_cut`
action) reboots the console, so it cannot stand in for pulling a card from a
running game; the removed-card journey runs on the `save-pull` lab disc, which
stops the first save after three card frames. A player with no card cannot
quit to the title, which Java's menu has no equivalent of either (the title
screen is the boot screen here).

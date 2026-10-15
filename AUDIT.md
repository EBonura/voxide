# VoXide tuning audit

VoXide matches Minecraft Java Edition values. Each fix below cites minecraft.wiki
(`W:` is `https://minecraft.wiki/w/`). Clocks: everything in `update_survival` and
`mob::update` runs on the 60 Hz sim tick, and `units::java_ticks(n)` converts Java
game ticks (20 a second) to it, so a 20 TPS constant cannot run at 60 Hz by accident.

## 2026-10-15: hunger and mob attack speed (GitHub issue 1)

The issue was filed on 2026-08-30 against 0.1.11, before the Java parity pass
(hunger by exhaustion in `c3414e1`, attack cooldown in `a3bedcc`, both 2026-09-24).
The rows below check the current code.

| Item | Code | Java value | Result |
|---|---|---|---|
| Exhaustion threshold | 4.0 costs 1 saturation, else 1 hunger (`update_survival`) | 4.0, same order (W:Hunger) | matches |
| Sprinting | 0.1 per block (`EXH_SPRINT_PER_UNIT`) | 0.1 per metre (W:Hunger) | matches |
| Swimming | 0.01 per block | 0.01 per metre (W:Hunger) | matches |
| Jump, sprint jump | 0.05, 0.2 | 0.05, 0.2 (W:Hunger) | matches |
| Attack landed, damage taken, block broken | 0.1, 0.1, 0.005 | 0.1, 0.1, 0.005 (W:Hunger) | matches |
| Walking | 0 | not listed (W:Hunger) | matches |
| Regeneration, hunger 18+ | 1 hp per 80 game ticks (240 sim ticks), 6.0 exhaustion | 1 hp per 80 ticks, 6.0 (W:Hunger) | matches |
| Saturation boost, hunger 20 | every 10 game ticks, min(saturation, 6) / 6 hp | every 10 ticks, 1.5 saturation per heal (W:Hunger) | matches |
| Starvation | 1 hp per 80 game ticks, stops at 1 hp | 1 hp per 80 ticks, stops at 1 hp on Normal (W:Hunger) | matches |
| Skeleton fire interval | 3 s | once every 3 s on Easy and Normal (W:Skeleton) | matches |
| Creeper (sapper) fuse | 30 game ticks | 30 ticks (1.5 s) | matches |
| Hostile melee damage | zombie 3, spider 2, enderman 7 | Normal values (W:Zombie, W:Spider, W:Enderman) | matches |
| Player invulnerability after a hit | 10 game ticks | 10 game ticks, 0.5 s (W:Damage) | matches |
| Hostile melee swing interval | a touching mob hit every time the player's invulnerability ended, so every 0.5 s | one swing per 20 game ticks (1 s) | **mismatch, fixed** |

### Fixed: hostile melee interval

`mob::contact_damage` returned the damage of every mob touching the player on every
sim tick, and the player's 10-tick invulnerability was the only brake. A zombie in
reach therefore hit every 0.5 s (6 hp a second). In Java the invulnerability is a
second, separate limit: the melee goal strikes once per 20 game ticks. Each mob now
carries an `atk` countdown, set to 20 game ticks (60 sim ticks) when it strikes, so
one mob in reach deals at most 3 hp a second. Several mobs still strike
independently, and the strongest one inside the player's invulnerability window
still decides the hit, as before.

The 20-tick interval is Java's `MeleeAttackGoal`, the figure `WOLF_BITE_TICKS`
already uses. The wiki does not state it on W:Zombie or W:Damage, so this one value
is cited from the game source rather than the wiki. If a playtest shows it feels
wrong, `MELEE_TICKS` in `game/src/mob.rs` is the only constant to change.

### Not changed

Hunger drain already matches Java. Before 0.2.0 the clock was `FOOD_DRAIN`, one
hunger point every 10 s (a tick-rate bug), which is the likely source of "you die very
quickly from hunger". Starvation cannot kill on Normal (it stops at 1 hp). Music is
not a tuning value and is not planned.

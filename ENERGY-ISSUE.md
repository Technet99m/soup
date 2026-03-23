# Energy Issue: Memory Fills to 100%

## Observed Behavior

Memory fills completely with programs in what appears to be a sudden explosion.
Seen in multiple runs. The population grows slowly at first, then seems to blow up.

From the log (`soup.log`, ~80k ticks, 5 templates):

- **911 births, only 37 deaths** (all from energy starvation)
- Live count grew monotonically: 5 → 879 programs, never dropping
- Growth was exponential — slow early, then accelerating

## Root Cause

`COMMIT` gifts each child `cfg.initial_energy` (default 1000) for free. That energy
is not deducted from the parent. The parent only pays `commit_cost = 20` plus base
instruction costs. Every successful replication **injects 1000 new energy into the
system from nothing**.

```
// vm.rs — Commit opcode
let child = Program::new(..., cfg.initial_energy, ...);  // ← free energy
p.energy -= cfg.commit_cost;                             // ← parent pays only 20
```

With unlimited free energy on each birth, programs almost never run out. The
population grows until memory is physically exhausted. Because growth is exponential,
it looks sudden — each generation roughly doubles the population.

## Secondary Observation: 224-byte Mutants

A lineage of 224-byte programs appeared repeatedly (parent ID 20 → 31 → 83/84 → ...).
The `measurer` template uses `MEASURE_SELF` for ALLOC, loop count, and COMMIT sizes.
A mutation inflated a size-related byte to 224, and since `MEASURE_SELF` reads the
program's tracked `length` field (not a hardcoded immediate), those children
faithfully replicate at 224 bytes. They consume more memory per program but are
otherwise functional.

## Levers to Fix

In order of invasiveness:

| Option | Where | Effect |
|--------|-------|--------|
| Reduce `initial_energy` | `config.rs` default | Children start with less runway; more die before replicating |
| Raise `alloc_cost` / `commit_cost` | `config.rs` default | Replication drains parent faster; population growth slows |
| Add per-tick existence cost | `vm.rs` step | Programs pay energy just to stay alive; idle programs starve |
| Draw child energy from parent | `vm.rs` Commit | Remove the free-energy injection entirely; zero-sum system |
| Global energy cap | `world.rs` tick | Hard ceiling on total system energy; forces competition |

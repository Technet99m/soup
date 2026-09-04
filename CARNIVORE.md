# Carnivore Programs

## Mechanic: Code Injection Parasitism

Carnivore programs write crafted bytecodes directly into another live program's memory. When the victim's IP reaches the injected instructions, it executes them — potentially draining its own energy into a drop zone the carnivore then harvests. The world adds no special transfer logic; everything flows through normal VM execution.

### Three New Opcodes

| Byte | Name | Behavior |
|------|------|----------|
| 34 | `SET_READ_HEAD` | `RH = reg_b` — mirror of `SET_WRITE_HEAD` (11). Lets programs snap RH to any scanned or computed address. |
| 35 | `SEEK_FOREIGN_START` | Scans circularly from RH for the nearest address owned by a *different* live program. Sets `reg_b` to that address. No-op if alone. |
| 36 | `GIVE_ENERGY_IMM` | `amount = min(reg_b, energy); energy -= amount; energy_map[imm16] += amount; IP += 3`. The immediate 16-bit address is encoded little-endian in the two bytes following the opcode. **This is the injectable drain payload.** |

### World Infrastructure

`World.addr_to_owner` — a 65536-cell `Box<[Option<ProgramId>]>` mapping each byte address to its owning program. Updated on spawn (mark) and death (clear). Passed as a read-only slice to `vm::step`, used by `SEEK_FOREIGN_START` and the `ForeignExec` event.

### Observability Events

- `ForeignExec { tick, id, ip, owner_id }` — emitted each tick a program is about to execute an instruction at an address owned by a different program. Controlled by `foreign_exec_tracking` config (default: true).
- `ForeignWrite { tick, attacker_id, victim_id, address }` — emitted when `WRITE` or `COPY` targets a foreign-owned cell. Controlled by `foreign_write_tracking` config (default: false, can be noisy).

### Config Knobs

| Key | Env Var | Default |
|-----|---------|---------|
| `foreign_exec_tracking` | `FOREIGN_EXEC_TRACKING` | true |
| `foreign_write_tracking` | `FOREIGN_WRITE_TRACKING` | false |

---

## Starter Templates

### `06_carnivore_killer` (8 bytes)

Repeatedly overwrites the first reachable byte of the nearest foreign program with `HALT` (0xFF). The victim halts when its IP reaches that byte, returning all remaining energy to the ambient pool.

```
SEEK_FOREIGN_START → SET_WRITE_HEAD → LOAD_IMM 255 → WRITE → [loop]
```

Bytes: `[35, 11, 12, 255, 9, 12, 8, 20]`

### `07_carnivore_drain` (27 bytes)

Injects a 3-byte `GIVE_ENERGY_IMM` payload at the victim's address, directing victim energy to drop zone `0x0000`. Then harvests the drop zone with `TAKE_ENERGY`. Loops indefinitely.

Injected payload: `[36, 0x00, 0x00]` — when executed by victim, deposits `victim.reg_b` energy to `energy_map[0]`.

```
; Phase 1: write [36, 0, 0] at victim address (3 writes, advancing WH each time)
SEEK_FOREIGN_START → SET_WRITE_HEAD → write 36 → advance WH → write 0 → advance WH → write 0

; Phase 2: harvest
LOAD_IMM 0 → SWAP → SET_READ_HEAD → TAKE_ENERGY

; Loop
JMP_BWD to top
```

Bytes: `[35, 11, 12, 36, 9, 17, 15, 17, 11, 12, 0, 9, 17, 15, 17, 11, 12, 0, 9, 12, 0, 17, 34, 31, 12, 27, 20]`

---

## Arms Race Dynamics

- **Victims can defend** by overwriting injected bytes back (self-repair via WRITE to own body).
- **Drain amount is stochastic** — depends on victim's `reg_b` at the moment the injected `GIVE_ENERGY_IMM` executes.
- **Killers create space** — victim death frees memory to the free list, which other programs can ALLOC.
- **Multiple parasites compete** for the same drop zone (address 0), creating selection pressure for faster harvesting.
- **No world-level special casing** — all behavior emerges from standard VM execution.

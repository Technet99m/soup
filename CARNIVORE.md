# Carnivore Programs

## Mechanic: Code Injection Parasitism

Carnivore programs write crafted bytecodes directly into another live program's memory. When the victim's IP reaches the injected instructions, it executes them — potentially excreting a stored metabolite into a drop zone the carnivore then harvests. The world adds no special transfer logic; everything flows through normal VM execution.

### Three New Opcodes

| Byte | Name | Behavior |
|------|------|----------|
| 34 | `SET_READ_HEAD` | `RH = reg_b` — mirror of `SET_WRITE_HEAD` (11). Lets programs snap RH to any scanned or computed address. |
| 35 | `SEEK_FOREIGN_START` | Searches both directions from RH for the nearest address owned by a *different* live program within the inclusive `interaction_radius`. Sets `reg_b` to that address; leaves it unchanged if no foreign address is local. |
| 36 | `EXCRETE_A_IMM` | `amount = min(reg_b, metabolite_a); metabolite_a -= amount; resource_a[imm16] += amount; IP += 3`. The immediate 16-bit address is encoded little-endian in the two bytes following the opcode. It cannot turn energy into A. |

### World Infrastructure

`World.addr_to_owner` — a 65536-cell `Box<[Option<ProgramId>]>` mapping each byte address to its owning program. Updated on spawn (mark) and death (clear). Passed as a read-only slice to `vm::step`, used by `SEEK_FOREIGN_START` and the `ForeignExec` event.

### Observability Events

- `ForeignExec { tick, id, ip, owner_id }` — emitted each tick a program is about to execute an instruction at an address owned by a different program. Controlled by `foreign_exec_tracking` config (default: true).
- `ForeignWrite { tick, attacker_id, victim_id, address }` — emitted when `WRITE` or `COPY` targets a foreign-owned cell. Controlled by `foreign_write_tracking` config (default: true so the observer can show attacks).

### Config Knobs

| Key | Env Var | Default |
|-----|---------|---------|
| `foreign_exec_tracking` | `FOREIGN_EXEC_TRACKING` | true |
| `foreign_write_tracking` | `FOREIGN_WRITE_TRACKING` | true |
| `interaction_radius` | `INTERACTION_RADIUS` | 256 |

Organism searches use circular distance, including across address zero. Increasing distance is considered symmetrically, with the forward address winning exact-distance ties. `SEEK_FOREIGN_START` and `SEEK_TAG` each cost the ordinary one energy unit whether they succeed or fail; search distance has no extra transfer or energy yield.

---

## Starter Templates

Both templates ship with `seed = false`. Set it to `true` in the template file to inoculate a world with that predator.

### `06_carnivore_killer` (8 bytes)

Repeatedly overwrites the first reachable byte of the nearest *local* foreign program with `HALT` (0xFF). The victim halts when its IP reaches that byte, returning all remaining energy to the ambient pool. A victim beyond `interaction_radius` is not selected by that seek; reaching it requires an evolved sequence of local movement or relays.

```
SEEK_FOREIGN_START → SET_WRITE_HEAD → LOAD_IMM 255 → WRITE → [loop]
```

Bytes: `[35, 11, 12, 255, 9, 12, 8, 20]`

### `07_carnivore_drain` (28 bytes)

Injects a 3-byte `EXCRETE_A_IMM` payload at the nearest local victim's address, directing the victim's stored A to drop zone `0x0000`. Then takes and converts the A. Loops indefinitely. Remote organisms outside `interaction_radius` are isolated from a single seek.

Injected payload: `[36, 0x00, 0x00]` — when executed by victim, deposits up to `victim.reg_b` units from `victim.metabolite_a` at resource-A cell 0.

```
; Phase 1: write [36, 0, 0] at victim address (3 writes, advancing WH each time)
SEEK_FOREIGN_START → SET_WRITE_HEAD → write 36 → advance WH → write 0 → advance WH → write 0

; Phase 2: harvest
LOAD_IMM 0 → SWAP → SET_READ_HEAD → TAKE_RESOURCE_A → CONVERT_A

; Loop
JMP_BWD to top
```

Bytes: `[35, 11, 12, 36, 9, 17, 15, 17, 11, 12, 0, 9, 17, 15, 17, 11, 12, 0, 9, 12, 0, 17, 34, 31, 44, 12, 28, 20]`

---

## Arms Race Dynamics

- **Victims can defend** by overwriting injected bytes back (self-repair via WRITE to own body).
- **Drain amount is stochastic** — depends on both victim `reg_b` and stored A when `EXCRETE_A_IMM` executes.
- **Killers create space** — victim death frees memory to the free list, which other programs can ALLOC.
- **Multiple parasites compete** for the same drop zone (address 0), creating selection pressure for faster harvesting.
- **No world-level special casing** — all behavior emerges from standard VM execution.

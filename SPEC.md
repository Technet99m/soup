# Primordial Soup — Digital Life Simulation Spec

## Overview

A digital life simulation where self-replicating bytecode programs evolve inside a shared finite memory. No behavior is hardcoded beyond the physics of the environment. Complexity, cooperation, and ecosystem structure should emerge purely from selection pressure.

---

## 1. Memory Model

- A flat array of **65,536 cells** (u8 values, 0–255)
- Every cell is always a valid instruction (no "invalid" states)
- Memory is **circular** — address arithmetic wraps around
- Programs occupy contiguous slices: `[start, start + length)`
- A separate **program registry** maps program IDs to `{ start, length, age, energy }`
- A parallel **energy map** (`[u32; 65536]`) holds deposited energy per cell, independent of instruction bytes. Programs can deposit energy (GIVE_ENERGY) for children or for themselves across cycles; other programs can sense (SENSE_ENERGY) and drain (TAKE_ENERGY) these deposits — enabling parasitic behavior to emerge.

---

## 2. Instruction Set

Every possible byte value must map to a valid instruction. Group them:

| Opcode (dec) | Mnemonic | Description |
|---|---|---|
| 0 | `NOP` | Do nothing |
| 1 | `MOV_FWD` | Move read head forward 1 |
| 2 | `MOV_BWD` | Move read head backward 1 |
| 3 | `MOV_FWD_N` | Move read head forward by register A |
| 4 | `MOV_BWD_N` | Move read head backward by register A |
| 5 | `SEEK_SELF_START` | Seek read head to own program start |
| 6 | `SEEK_SELF_END` | Seek read head to own program end |
| 7 | `SEEK_FREE_START` | Find nearest free memory block of size >= own length, put address in register B |
| 8 | `READ` | Read cell at read head into register A |
| 9 | `WRITE` | Write register A to write head location |
| 10 | `COPY` | Copy cell at read head to write head, advance both |
| 11 | `SET_WRITE_HEAD` | Set write head to register B |
| 12 | `LOAD_IMM` | Load next byte in own code as immediate into register A |
| 13 | `ADD` | A = A + B |
| 14 | `SUB` | A = A - B |
| 15 | `INC` | A = A + 1 |
| 16 | `DEC` | A = A - 1 |
| 17 | `SWAP` | Swap A and B |
| 18 | `JMP` | Jump instruction pointer to address in register A |
| 19 | `JMP_FWD` | Jump instruction pointer forward by A |
| 20 | `JMP_BWD` | Jump instruction pointer backward by A |
| 21 | `JMP_IF_ZERO` | Jump forward by A if B == 0 |
| 22 | `JMP_IF_NONZERO` | Jump forward by A if B != 0 |
| 23 | `LOOP_OPEN` | Push current address to loop stack, continue |
| 24 | `LOOP_CLOSE` | If A != 0: decrement A, jump back to matching `LOOP_OPEN` |
| 25 | `ALLOC` | Allocate a new empty block of size A, put start address in register B. Costs energy. |
| 26 | `COMMIT` | Register the block at register B with size A as a new child program. Costs energy. |
| 27 | `SPLIT` | Like COMMIT but immediately gives child half of remaining energy |
| 28 | `SCAN_FWD` | Scan forward until cell == A, put found address in B |
| 29 | `SCAN_BWD` | Scan backward until cell == A, put found address in B |
| 30 | `GIVE_ENERGY` | Deposit register B energy from own pool into energy map at write head. Costs 1 base + energy given. |
| 31 | `TAKE_ENERGY` | Drain all energy from energy map at read head into own pool. Costs 1 base. |
| 32 | `SENSE_ENERGY` | Load energy map value at read head into register B (saturating at 65535). Costs 1 base. |
| 33 | `MEASURE_SELF` | Load this program's registered length into register A. |
| 34–254 | `NOP_*` | All treated as NOP (mutation-safe padding) |
| 255 | `HALT` | Stop execution immediately |

---

## 3. Registers & Execution State

Each running program has:

```text
IP   — instruction pointer (absolute memory address)
A    — general purpose / size register (u16)
B    — address register (u16, for memory addressing)
RH   — read head (u16)
WH   — write head (u16)
LOOP — loop stack (max depth 8)
```

---

## 4. Energy System

- Each program starts with **200 energy units**
- Each instruction executed costs **1 energy**
- `ALLOC` costs **10 energy**
- `COMMIT` / `SPLIT` costs **20 energy**
- When energy reaches 0: program is **killed and memory freed**
- A program that successfully calls `COMMIT` passes a lineage record to the child

This replaces age-based death. Survival requires efficient replication.

---

## 5. Scheduler

- Maintains a **run queue** (ordered list of live program IDs)
- Each tick: execute **one instruction** for the current program, advance queue
- This gives all programs roughly equal CPU time (no starvation)
- Programs added via `COMMIT` are appended to the end of the queue
- Dead programs are removed from the queue lazily

---

## 6. Mutation

Applied at **write time** (when `WRITE` or `COPY` executes):

- **0.5% chance per byte written** of flipping the written value to a random u8
- Mutation rate is a **configurable global parameter**
- No other source of mutation — errors only enter through replication

---

## 7. Memory Allocation

- A **free list** tracks unoccupied memory ranges
- `ALLOC` searches the free list for the best fit block
- If no block is large enough: instruction is a no-op (costs energy anyway)
- Freed memory is not zeroed — it retains old values (fossil data, potential parasite fuel)

---

## 8. The Seed Program

The simulation starts with **one hand-written program** placed at address 0. It should be the simplest possible self-replicator:

```text
SEEK_SELF_START     ; point read head at own start
LOAD_IMM [own_len]  ; A = own length
ALLOC               ; find free block of that size, B = destination
SET_WRITE_HEAD      ; WH = B
SEEK_SELF_START     ; reset RH
LOAD_IMM [own_len]  ; A = loop counter
LOOP_OPEN           ;
  COPY              ; copy one byte RH→WH, advance both
LOOP_CLOSE          ; repeat A times
LOAD_IMM [own_len]  ; A = child size
COMMIT              ; register new program at B
HALT
```

This is the primordial ancestor. Evolution takes it from here.

With `MEASURE_SELF`, descendants can instead derive size dynamically:

```text
MEASURE_SELF         ; A = own registered length
... arithmetic ...   ; optionally grow/shrink A
ALLOC
... copy ...
COMMIT               ; child size comes from A
```

---

## 9. Lineage Tracking

- Each program has a `lineage_id` (UUID) and `parent_id`
- On `COMMIT`/`SPLIT`: child inherits parent's lineage chain
- Mutations are recorded as diffs at commit time
- This allows a **full evolutionary tree** to be reconstructed from logs

---

## 10. Observability

### Required logs (append-only event stream):
```text
TICK       { tick_number }
BORN       { id, parent_id, start, length, energy }
DIED       { id, cause: "energy" | "killed" | "evicted" }
MUTATED    { tick, address, old_value, new_value }
COMMITTED  { parent_id, child_id }
```

### Live stats (emit every N ticks):
```text
- Total live programs
- Memory utilization %
- Unique lineages alive
- Most common instruction distribution
- Oldest living program age (in ticks)
```

---

## 11. Configuration

All as environment variables or a config file:

```text
MEMORY_SIZE         default: 65536
INITIAL_ENERGY      default: 200
MUTATION_RATE       default: 0.005
ALLOC_COST          default: 10
COMMIT_COST         default: 20
LOOP_MAX_DEPTH      default: 8
TICKS_PER_STAT_LOG  default: 10000
SEED_PROGRAM        default: built-in minimal replicator
```

---

## 12. Implementation Notes

- Implement in **Rust** for performance (this will run millions of ticks)
- Memory array should be a plain `[u8; 65536]`
- Run queue should be a `VecDeque<ProgramId>`
- All u8 arithmetic is **wrapping** (no panics on overflow)
- Logs should go to a file, not stdout (stdout reserved for live stats)
- No threads initially — single-threaded deterministic simulation first

---

## Out of Scope (for now)

- Visualization (add later)
- Networking / distributed memory
- Sexual recombination
- Any fitness function — **there is no goal, only survival**

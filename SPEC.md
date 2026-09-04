# Primordial Soup — Digital Life Simulation Spec

## Overview

A digital life simulation where self-replicating bytecode programs evolve inside a shared finite memory. No behavior is hardcoded beyond the physics of the environment. Complexity, cooperation, and ecosystem structure should emerge purely from selection pressure.

---

## 1. Memory Model

- A flat array of **65,536 cells** (u8 values, 0–255)
- Every cell is always a valid instruction (no "invalid" states)
- Memory is **circular** — address arithmetic wraps around
- Programs occupy contiguous slices: `[start, start + length)`
- A separate **program registry** maps program IDs to `{ start, length, age, energy, metabolite_a, metabolite_b }`
- Parallel **resource A** and **resource B** maps (`[u32; 65536]` each) hold deposits independently of instruction bytes. Each chemistry has distinct seek, sense, take, and give instructions.
- External A and B deposits come from fixed or moving periodic sources. Source positions are relative to a seed-derived environmental origin and never depend on the live population or simulation RNG state.
- A moves forward and B moves backward during decay sweeps. Source movement and counter-currents produce changing local resource conditions while conserving the combined energy budget.

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
| 30 | `EXCRETE_A` | Move up to register B units from the internal A store to the A map at the write head. |
| 31 | `TAKE_RESOURCE_A` | Move the A deposit at the read head into the internal A store; it does not become energy. |
| 32 | `SENSE_RESOURCE_A` | Load the A deposit at the read head into register B (saturating at 65535). |
| 33 | `MEASURE_SELF` | Load this program's registered length into register A. |
| 34 | `SET_READ_HEAD` | Set read head to register B |
| 35 | `SEEK_FOREIGN_START` | Find memory owned by another live program |
| 36 | `EXCRETE_A_IMM` | Move stored A to a two-byte immediate address |
| 37 | `TAKE_RESOURCE_B` | Move the B deposit at the read head into the internal B store; it does not become energy. |
| 38 | `SENSE_RESOURCE_B` | Load B resource at the read head into register B |
| 39 | `EXCRETE_B` | Move up to register B units from the internal B store to the B map at the write head |
| 40 | `SEEK_RESOURCE_A` | Move the read head to the nearest A deposit |
| 41 | `SEEK_RESOURCE_B` | Move the read head to the nearest B deposit |
| 42 | `SET_TAG` | Set the organism's recognition tag from register A |
| 43 | `SEEK_TAG` | Find another organism whose tag matches register A |
| 44 | `CONVERT_A` | Convert stored A to energy, up to register B units; B=0 converts all. |
| 45 | `CONVERT_B` | Convert stored B to energy, up to register B units; B=0 converts all. |
| 46 | `COMBINE_AB` | Consume equal A and B amounts and yield two energy per pair; B=0 combines all possible pairs. |
| 47–254 | `NOP_*` | All treated as NOP |
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
MA   — internal resource-A metabolite store (u32)
MB   — internal resource-B metabolite store (u32)
LOOP — loop stack (max depth 8)
```

---

## 4. Energy System

- Each program starts with **200 energy units**
- Each instruction executed costs **1 energy**
- `ALLOC` costs **10 energy**
- `COMMIT` / `SPLIT` costs **20 energy**
- When energy reaches 0: program is **killed and memory freed**
- An organism also wears out after a configurable instruction budget. This prevents sterile energy-hoarding loops from stopping generational turnover.
- A program that successfully calls `COMMIT` passes a lineage record to the child
- Resource uptake fills `MA` or `MB`; only the matching conversion opcode can turn that store into energy. Energy can never be relabeled as A or B.
- `EXCRETE_A`, `EXCRETE_A_IMM`, and `EXCRETE_B` move existing metabolites back to the world. They never spend energy as product.
- `COMMIT` and `SPLIT` transfer half of each metabolite store to the child. Death returns energy and both stores to ambient.
- Instruction costs, uptake, conversion, combination, excretion, inheritance, decay, and rain all conserve `TOTAL_ENERGY` exactly.

Energy scarcity and finite body lifetime make survival through descendants require efficient replication.

---

## 5. Scheduler

- Maintains a **run queue** (ordered list of live program IDs)
- Each tick: execute **one instruction** for the current program, advance queue
- This gives all programs roughly equal CPU time (no starvation)
- Programs added via `COMMIT` are appended to the end of the queue
- Dead programs are removed from the queue lazily

---

## 6. Mutation

Substitution is applied at **write time** (when `WRITE` or `COPY` executes):

- **0.5% chance per byte written** of flipping the written value to a random u8
- Mutation rate is a **configurable global parameter**
- At birth, independent configurable rates insert one byte, delete a 1–8 byte span, or duplicate a 1–8 byte span.
- A child's inherited recognition tag can also mutate independently.

---

## 7. Memory Allocation

- A **free list** tracks unoccupied memory ranges
- `ALLOC` usually chooses the closest fitting location to the parent; a configurable fraction uses global best fit
- If no block is large enough: instruction is a no-op (costs energy anyway)
- Freed memory is not zeroed — it retains old values (fossil data, potential parasite fuel)

---

## 8. The Seed Program

The simulation starts with **one hand-written 18-byte program**. It takes and converts both resource types, measures its genome, copies that measured span, splits energy and metabolites with the child, and loops:

```text
SEEK_RESOURCE_A
TAKE_RESOURCE_A
CONVERT_A
SEEK_RESOURCE_B
TAKE_RESOURCE_B
CONVERT_B
MEASURE_SELF        ; A = current registered length
ALLOC               ; find free block of that size, B = destination
SET_WRITE_HEAD      ; WH = B
SEEK_SELF_START     ; reset RH
MEASURE_SELF        ; A = loop counter
LOOP_OPEN           ;
  COPY              ; copy one byte RH→WH, advance both
LOOP_CLOSE          ; repeat A times
MEASURE_SELF        ; A = child size
SPLIT               ; register child and split remaining energy
MEASURE_SELF
JMP_BWD             ; repeat
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
- Substitutions and structural genome edits are emitted as events
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
- Distinct live genomes, generation depth, and byte distance from each startup ancestor
- Most common instruction distribution
- Oldest living program age (in ticks)
- Per-organism execution counts, A/B harvests, A/B gifts, and tag searches
- Per-organism A/B stores plus A, B, and combined conversion totals
- Donor provenance for resources consumed by a different genome
- `METABOLIZED` events containing the pathway, A/B inputs, and energy yield
- Counterfactual reproductive-rate change after removing either candidate partner
```

---

## 11. Configuration

All as environment variables or a config file:

```text
MEMORY_SIZE         default: 65536
INITIAL_ENERGY      default: 200
MUTATION_RATE       default: 0.005
INSERTION_RATE      default: 0.004 per birth
DELETION_RATE       default: 0.004 per birth
DUPLICATION_RATE    default: 0.004 per birth
MAX_GENOME_LENGTH   default: 512
CHILD_LOCALITY_BIAS default: 0.92
TAG_MUTATION_RATE   default: 0.01
INTERACTION_RADIUS  default: 256
ALLOC_COST          default: 10
COMMIT_COST         default: 20
MAX_PROGRAM_AGE     default: 20000
LOOP_MAX_DEPTH      default: 8
TICKS_PER_STAT_LOG  default: 10000
ENERGY_CURRENT      default: 17
RESOURCE_SOURCES    default: four A/B emitters configured by offset, interval,
                    amount, width, and velocity in soup.toml
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

## Out of Scope

- Networking / distributed memory
- Sexual recombination
- Any fitness function — **there is no goal, only survival**

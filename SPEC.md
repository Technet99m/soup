# Primordial Soup — Digital Life Simulation Spec

## Overview

A digital life simulation where self-replicating bytecode programs evolve inside a shared finite memory. No behavior is hardcoded beyond the physics of the environment. Complexity, cooperation, and ecosystem structure should emerge purely from selection pressure.

---

## 1. Memory Model

- A flat array of **65,536 cells** (u8 values, 0–255)
- Every cell is always a valid instruction (no "invalid" states)
- Memory is **circular** — address arithmetic wraps around
- Programs occupy contiguous slices: `[start, start + length)`
- A separate **program registry** maps program IDs to `{ start, length, age, energy, metabolite_a, metabolite_b, tag, lineage_id, parent_id, generation }`
- Parallel **resource A** and **resource B** maps (`[u32; 65536]` each) hold deposits independently of instruction bytes. Each chemistry has distinct seek, sense, take, and give instructions. Parallel provenance maps retain exact quantities keyed by the donor ID and the donor's deposit-time `HeritableIdentity`; organism-independent source quantities are explicitly unattributed.
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
| 35 | `SEEK_FOREIGN_START` | Put the nearest foreign-owned address within `INTERACTION_RADIUS` in B; leave B unchanged if none is local. |
| 36 | `EXCRETE_A_IMM` | Move stored A to a two-byte immediate address |
| 37 | `TAKE_RESOURCE_B` | Move the B deposit at the read head into the internal B store; it does not become energy. |
| 38 | `SENSE_RESOURCE_B` | Load B resource at the read head into register B |
| 39 | `EXCRETE_B` | Move up to register B units from the internal B store to the B map at the write head |
| 40 | `SEEK_RESOURCE_A` | Move the read head to the nearest A deposit |
| 41 | `SEEK_RESOURCE_B` | Move the read head to the nearest B deposit |
| 42 | `SET_TAG` | Set the organism's recognition tag from register A |
| 43 | `SEEK_TAG` | Put the nearest foreign-owned address whose owner's tag matches A within `INTERACTION_RADIUS` in B; leave B unchanged if none is local. |
| 44 | `CONVERT_A` | Convert stored A to energy, up to register B units; B=0 converts all. |
| 45 | `CONVERT_B` | Convert stored B to energy, up to register B units; B=0 converts all. |
| 46 | `COMBINE_AB` | Consume equal A and B amounts and yield two energy per pair; B=0 combines all possible pairs. |
| 47–254 | aliases | Decode with instruction index `(byte - 47) mod 48`, where indices 0–46 are the canonical rows above and index 47 is `HALT`. |
| 255 | `HALT` | Stop execution immediately |

The named bytes 0–46 and 255 are canonical and retain their historical semantics. Converting an instruction back to a byte always returns that canonical encoding, so the shipped ancestor and templates require no migration. Bytes 47–254 provide four complete extra passes over the 48-instruction alphabet plus a fifth pass for indices 0–15. Consequently, every phenotype has five or six encodings (at most 2.34% of the alphabet), including `NOP`; every non-NOP instruction has multiple synonymous raw genotypes. All 256 input bytes decode without rejection.

---

## 3. Registers & Execution State

### Local circular searches

`SEEK_FOREIGN_START` and `SEEK_TAG` use the same deterministic neighborhood rule around RH. Circular distance is the shorter wrapped distance on the 65,536-cell ring, and `INTERACTION_RADIUS` is inclusive: a target exactly at the radius is reachable while one cell beyond is not. Distance zero is checked first, then increasing distances in both directions. At an exact-distance tie the forward address wins; at the 32,768-cell antipode forward and backward are the same address. Radii above 32,768 therefore cover the whole ring without visiting an address twice.

Both instructions ignore the executing organism's own cells. `SEEK_TAG` additionally requires the foreign owner's current tag to equal the low byte of A. Success writes the matched address to B. Failure—including a lone organism, no foreign target, or no matching tag—leaves A, B, and RH unchanged. Each attempt costs the normal one energy unit regardless of radius or result; that unit returns to the ambient pool, so searching neither transfers nor creates energy.

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

- Each program starts with **5,000 energy units** by default
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

- The configurable mutation-rate draw decides whether mutation occurs. If it does, one uniformly sampled mutation-choice byte drives a pure local kernel.
- For an even choice, the result is another encoding of the same decoded instruction. Encodings are ordered by raw byte after excluding the source; `(choice >> 1) mod synonym_count` selects one, so the stored raw byte always changes while behavior is synonymous.
- For an odd choice, bit 1 selects the preceding or following **non-NOP** instruction in canonical opcode order (wrapping between `MOV_FWD` and `HALT`), and `choice >> 2` selects one of its raw aliases. A NOP source enters the same functional ring at one of those endpoints.
- Thus the complete one-step neighborhood of every functional encoding contains both changed-byte synonyms and non-NOP functional neighbors. It cannot fall into NOP from a functional source.
- At birth, an insertion position is chosen as before and one uniformly sampled choice byte is inserted unchanged. Since the complete raw alphabet is balanced, this reaches each genotype once without recreating a dominant NOP outcome. Deletion and duplication continue to operate on raw spans.
- The kernel receives only the source byte and mutation-choice byte. It never receives or inspects viability, lineage, fitness, intended behavior, or world state; there is no post-mutation filtering.
- Both mappings are pure, so a fixed mutation-choice stream replays byte-for-byte under the same event schedule and RNG seed.
- Mutation rate is a **configurable global parameter**.
- At birth, independent configurable rates insert one byte, delete a 1–8 byte span, or duplicate a 1–8 byte span.
- `COMMIT` and `SPLIT` copy the parent's current recognition tag to the child before birth mutation.
- A child's inherited recognition tag can mutate independently at birth according to `TAG_MUTATION_RATE`; a triggered mutation always chooses a different tag.
- `SET_TAG` changes only the executing organism's current tag. Existing deposits and emitted lineage events retain their earlier snapshots.

Behavior traces increment the canonical decoded instruction index, so synonymous opcode bytes aggregate into the same phenotype counters. Genome hashes and mutation events continue to use raw bytes, preserving genotype identity.

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
- On `COMMIT`/`SPLIT`: child inherits the parent's lineage chain and current recognition tag before independent birth mutations are applied.
- Substitutions and structural genome edits are emitted as events
- A `HeritableIdentity` is the raw `(genome hash, recognition tag)` pair used for lineage, activity, transfer, and counterfactual accounting. Raw genome counts remain tag-independent. “Ecotype” is reserved for the future persistent behavioral-viability concept.
- This allows a **full evolutionary tree** to be reconstructed from logs

---

## 10. Observability

### Required logs (append-only event stream):
```text
TICK       { tick }
BORN       { tick, id, parent_id, lineage_id, parent_lineage_id, start, length, energy, generation, heritable_identity: { genome, tag } }
DIED       { tick, id, cause: "energy" | "senescence" | "killed" | "evicted" }
MUTATED    { tick, address, old_value, new_value }
STRUCTURAL_MUTATION { tick, id, parent_id, kind, index, old_length, new_length }
TAG_CHANGED { tick, id, old_tag, new_tag }
RESOURCE_TRANSFER { tick, donor_id, donor_heritable_identity, receiver_id, receiver_heritable_identity, resource, amount }
METABOLIZED { tick, id, pathway, input_a, input_b, energy_yield }
COMMITTED  { tick, parent_id, child_id }
```

`RESOURCE_TRANSFER` is a cross-organism exchange: the donor and receiver must have different `ProgramId` values. The donor's deposit-time `HeritableIdentity` is retained for attribution even if that organism later changes its tag or genome; an identity change alone does not turn an organism's recovery of its own deposit into a transfer.

### Live stats (emit every N ticks):
```text
- Total live programs
- Memory utilization %
- Distinct raw live genomes, distinct live heritable identities, generation depth, and byte distance from each startup ancestor
- Most common instruction distribution
- Oldest living program age (in ticks)
- Per-organism execution counts, A/B harvests, A/B gifts, and tag searches
- Per-organism A/B stores plus A, B, and combined conversion totals
- Exact deposit-time donor provenance for resources consumed by a different organism (`ProgramId`), including after donor mutation or death
- `METABOLIZED` events containing the pathway, A/B inputs, and energy yield
- Counterfactual reproductive-rate change after removing either candidate partner
```

---

## 11. Configuration

All as environment variables or a config file:

```text
MEMORY_SIZE         default: 65536
INITIAL_ENERGY      default: 5000
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

## 13. Deterministic Identity, Ordering, and Replay

### Run and lineage identity

`World::run_namespace()` returns a 32-byte BLAKE3-256 namespace. Its input is the
**resolved** simulation configuration supplied to `World::new` (defaults, TOML,
and environment overrides have already been applied), followed by the loaded
startup templates in loader order. The namespace includes all mutation rates,
energy and lifespan parameters, allocation/commit costs, interaction and loop
limits, seed, current/decay parameters, and every source's kind, offset, interval,
amount, width, and velocity. Source order is significant because sources draw
from the same finite ambient pool.

Templates are loaded in filename order. Each loaded template contributes its
name and exact instruction bytes, with explicit boundaries. Paths, file contents
that only change formatting/comments, and description text do not contribute.
Disabled, empty, or invalid templates do not contribute; if none are loaded, the
actual built-in ancestor name and bytes contribute. Changing template names,
genomes, or the order of distinct templates changes the namespace, even if seed
and starting placements happen to match.

Log paths, template directory paths, statistics frequency, ecotype viability
thresholds, and foreign execution/write logging switches do not affect run or
lineage identity. The legacy
`Config::memory_size` field is not effective: this VM always has 65,536 cells,
and that fixed size is encoded instead. Configuration changes after construction
do not rewrite the original namespace; current effective configuration is also
included in state and birth hashing.

Numeric program IDs remain deterministic monotonic `u32` IDs in startup/birth
order. Each startup program gets a lineage UUID from the namespace, preceding
startup history, full initial program state, and template bytes. Each successful
birth extends a full 256-bit history chain using the canonical birth-time world
state, full child state, and exact post-mutation child genome. This happens after
birth mutations and before installation and emission of `Born`. The birth-time
state includes the preceding history, tick, allocator, parent/program state,
queue, resources, accounting, and RNG position. Birth hashing excludes foreign
logging switches and their two event counters, plus ecotype observation archives,
active segments, viability caches, announcement history, and thresholds. These
observer values affect reports and emitted events but cannot affect dynamics.

Lineage UUIDs use the first 16 bytes of the cryptographic result with RFC variant
bits and custom version 8 bits set; the untruncated history remains in `World`.
No identity generation consumes OS entropy or simulation RNG draws. The existing
`Program::new` API is retained: standalone VM callers receive a deterministic
provisional UUID hashed from all constructor arguments. `World` replaces it with
the run/history identity before exposing a startup organism or birth. Parent
UUIDs continue to reference the actual parent's lineage UUID. The existing
64-bit genome fingerprint plus recognition tag (`HeritableIdentity`) is unchanged;
it groups ecological observations and is not the cryptographic lineage identity.

Clones copy namespace, history, and RNG position. Corresponding births in equal
clones have identical UUIDs. A counterfactual intervention that changes the state
at a birth splits subsequent history even if the child genome and numeric ID
match. Existing organisms retain their IDs. Restoring an intervened value after
a divergent birth does not erase that divergence. No fitness, selection,
metabolism, scheduling, or candidate scoring rule is added or changed.

### Total observer orderings

All heritable identity comparisons use `(genome, tag)` in ascending order, with
full values rather than shortened display hashes.

- Transfer-based candidates canonicalize each pair by ascending identity and
  rank by descending combined transfer amount, then ascending identity pair.
- Fallback candidates retain the existing activity filter, rank identities by
  descending population then ascending identity, and take the first 12. Pairs
  retain this ranked traversal orientation and compare the existing score with
  `f64::total_cmp`, descending, then the identity pair ascending. These scores
  generate observer hypotheses; they never affect reproduction or selection.
- TUI genome summaries rank by descending population, descending maximum
  generation, ascending genome fingerprint, then ascending tag. Phenotype
  discovery uses that order. Program selection uses ascending numeric IDs.
- Tag summaries rank by descending population then ascending tag, including
  extinct tags with recorded births. Headless truncation takes the first six
  entries only after sorting. Integer sums/maxima and instruction histograms are
  independent of program/map insertion order.
- Events remain in deterministic tick/VM order; observers do not resort them.
  The scheduler preserves queue order and lazily skips dead IDs.

### Canonical public state digest

`World::state_digest()` returns a lowercase, 64-digit BLAKE3-256 hexadecimal
fingerprint without changing the world or consuming randomness. It covers:

- All 65,536 instruction bytes, including free and pending child memory; every
  cell of both resource maps; and each map's complete per-cell provenance with
  exact quantities, unattributed entries, donor IDs, and deposit-time genome/tag.
- The entire queue in order, including stale IDs, and all live programs sorted
  by registry key. Both the key and stored program ID are encoded. Every program
  field is included: addresses, registers, heads, pending allocation, ordered
  loop stack, energy and both metabolites, age, generation, lineage and parent
  links, template ID, tag, and every trace counter including all opcode counts.
- Each allocator block's start and full `u32` length in allocator order; tick,
  next ID, ambient pool, all birth/death/mutation/foreign-event counters, maximum
  generation, ownership map, tag history, identity history, all per-identity
  accounting maps, interactions, template names/genomes, namespace, and history.
- Current effective configuration, including the complete ordered source
  schedule. Origin and emission phase are determined by seed and tick; there is
  no hidden schedule RNG or mutable source cursor. Unlike lineage hashing, public
  state includes foreign tracking switches and their counters because these
  determine observable events. Ecotype viability thresholds, completed behavior
  archives, active segment bookkeeping, viable result cache, and the set of
  already-announced equivalence classes are also included because they determine
  current reports or future `NEW_PROGRAM` events. Ecotype hash collections are
  encoded in total key order. File paths and statistics cadence are excluded.
- The `rand_chacha` 0.3.1 ChaCha12 algorithm tag, full RNG seed, stream number, and
  next 32-bit word position (`u128`). Buffer layout is irrelevant to replay.
  ChaCha12 preserves the current rand 0.8 `StdRng` stream, including mixed-width
  draws. Startup placement uses the same algorithm and existing derived seed;
  its temporary RNG is exhausted before construction returns.

Encoding is explicit, not Rust `Hash`, `Debug`, serde object layout, or
`DefaultHasher`. Every encoding starts with length-prefixed `soup/canonical/v1`
and a distinct domain: `run-namespace/v1`, `standalone-program/v1`,
`startup-lineage/v1`, `birth-state/v1`, `birth-lineage/v1`, or `public-state/v1`.
Configuration also carries `effective-config/v1`. Fields have fixed schema order.
Integers use their declared width in little endian; `usize` values and collection/
string lengths use `u64`. Floats encode their IEEE-754 `u64` bits. Strings encode
UTF-8 bytes; sequences (including byte arrays) encode length then elements.
Booleans and option discriminants use one byte (`0`/`1`); `Some` adds its value.
UUIDs encode 16 raw bytes. Resource kinds encode A=0 and B=1. Unordered maps sort
by full keys before encoding their length and entries; provenance sorts by
optional origin (`None` first, then donor ID, genome, tag). No randomized
iteration contributes to a fingerprint. Encoding/schema or algorithm changes
must update the relevant version tags.

Replay comparisons require the same simulation implementation, resolved
inputs, dependency versions, target, and tick count. `soup --ticks N
--state-digest` prints `State digest: <hex>` after the main run and flushing its
event log, including runs ending through interruption or extinction. An optional
counterfactual analysis operates on clones and does not alter this final state.
The CLI does not change its normal output unless the flag is present. Event logs
continue to **append**; independent replay fixtures explicitly truncate their
log files first and separately verify deliberate append behavior.

The TUI reset rebuilds from its retained configuration and the same loaded
inputs, preserving display speed. It reproduces startup identities, subsequent
birth/event streams, and digest sequences. Display timing, pause state,
selection, and activity buffers are observer state outside the world digest.

---

## Out of Scope

- Networking / distributed memory
- Sexual recombination
- Any fitness function — **there is no goal, only survival**

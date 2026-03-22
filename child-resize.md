## Problem

Programs can currently only reproduce with the child size they manually place in `reg_a` before `COMMIT`. That makes replication size entirely explicit and prevents a child from adapting its own size based on how much code it actually needs or how much memory it can afford.

## Proposed approach

Add a `MEASURE_SELF` instruction that writes the current program's effective size into a size register or size path, so programs can compute child size from their own actual footprint at runtime.

The plan assumes the size path is widened beyond `u8`, so child sizes can exceed 255 bytes. That keeps the feature useful for larger genomes and avoids coupling replication semantics to a single-byte immediate.

The instruction should measure the program's registered length, not a guessed length from the current instruction stream. That makes the result stable and aligned with allocation / commit bookkeeping.

Child creation should then work in one of two patterns:

1. `MEASURE_SELF` -> optionally adjust size with arithmetic -> `ALLOC` / `COMMIT`
2. `MEASURE_SELF` -> adjust size -> `ALLOC` / `COPY` / `COMMIT`

This lets evolution select for smaller, faster children or larger, more capable children without changing the overall VM model.

## Todos

- Define the new size representation in the runtime so child size is not limited to `u8`.
- Add `MEASURE_SELF` to the opcode table and decoding.
- Implement `MEASURE_SELF` in the VM so it reports the current program's registered length.
- Update allocation and commit paths so they accept the widened size value consistently.
- Add tests covering `MEASURE_SELF`, size adjustment, and child creation with sizes above 255 bytes.
- Update the spec to describe the new instruction and the revised size semantics.

## Notes

- The feature should stay mutation-safe: existing byte values should continue decoding to valid instructions.
- `MEASURE_SELF` should use the program's tracked region size, not inspect memory heuristically.
- If the runtime introduces a dedicated size register, the spec should describe how it interacts with existing `A` / `B` registers and how size values flow into `ALLOC` and `COMMIT`.

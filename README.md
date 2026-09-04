# Primordial Soup

A single 18-byte ancestor enters a circular 64 KiB memory and leaves descendants. Nothing is scored and no target program is supplied. Programs survive by executing bytecode, finding finite resources, converting distinct A and B metabolites, copying themselves, and paying for every instruction.

The redesign gives evolution room to change more than byte values:

- insertions, deletions, and gene-like span duplications change genome length;
- birth-time growth claims adjacent capacity atomically when possible, while impossible selected edits emit an explicit failure event and leave inheritance and accounting unchanged;
- the ancestor measures its current length, so resized descendants can still reproduce;
- offspring usually occupy memory beside their parent, allowing persistent local ecologies;
- A and B resources come from organism-independent fixed or moving sources, flow in opposite currents, occupy separate internal stores, require different uptake/conversion/excretion instructions, and can only be found locally;
- a heritable, mutable tag can be set and searched for by programs;
- execution traces classify what a genome actually does, aggregating redundant raw opcode aliases under one decoded instruction while exact byte genomes remain distinct;
- all 256 bytes use a balanced redundant opcode encoding, and substitutions move through a documented local synonymous/adjacent-instruction kernel rather than an overwhelmingly NOP basin;
- replication fidelity and insertion/deletion/duplication spectra are inherited extra-genomic traits that can themselves drift upward or downward;
- suspected partnerships can be tested in cloned worlds with either partner removed.

Run the live evolution observer:

```sh
cargo run --release --bin viz
```

Start at 100 ticks per frame and use `+` to accelerate. The genome view colors live heritable identities (byte sequence plus recognition tag). White flashes identify recent mutations and attacks. The resource view uses cyan for A, magenta for B, and yellow where both occur. The right-hand table describes observed metabolism and interaction behavior.

## Recognition identity

`HeritableIdentity` is the raw tuple `(genome hash, recognition tag, mutation strategy)`. The explicit extra-genomic components keep identical byte sequences with different recognition behavior or mutation spectra in separate clades. Children inherit their parent's tag and five-locus mutation strategy on both `COMMIT` and `SPLIT`. Each strategy locus is a fixed-point probability controlling replication-copy substitution, insertion, deletion, duplication, or mutation of the strategy itself. A strategy mutation chooses one locus and an unbiased higher/lower step without inspecting survival, behavior, fitness, or the mutation's outcome; no offspring is filtered after mutation. `SET_TAG` changes the executing organism's tag during its lifetime, and `tag_mutation_rate` independently replaces an inherited tag at birth. Birth lineage events record the child's complete heritable identity.

The strategy's fidelity locus applies only when `COPY` writes into the executing parent's reserved child allocation. `WRITE`, attack-like copies, and working-memory copies remain exact, so evolvable replication fidelity cannot silently alter other memory operations. The legacy global mutation rates now initialize startup ancestors; they do not overwrite descendants. Defaults quantize to 1/65,536 probability units and retain the ancestor's viable reproduction cycle.

## Behavioral ecotypes

An `EcotypeIdentity` records the exact `HeritableIdentity` that expressed an execution `BehaviorSignature`. The signature is count-independent: it records which opcodes and resource/recognition effects occurred, so running the same behavior longer does not create a new phenotype. For viable-ecotype counting, two raw genomes are equivalent when they have the same recognition tag and behavior signature; the report retains a deterministic representative genome and the number of equivalent raw genomes as evidence. Different tags remain distinct because recognition changes ecological behavior even when execution is otherwise equal.

The world segments a program's trace whenever its genome bytes or recognition tag change. Completed segments and final segments from dead organisms remain in `behavior_archive`, including their reproductive output and exact child IDs. Only completed segments are viability evidence, so a live descendant's incomplete execution prefix cannot be mistaken for stable behavior. This prevents behavior observed before an identity change from being attributed to the new identity and lets dead ancestors provide lineage evidence.

A behavioral equivalence class becomes viable only after it meets all three observer-only thresholds: `ecotype_min_persistence_ticks`, `ecotype_min_reproductive_output`, and `ecotype_min_descendant_generations`. Descendant generations count explicit stable parent-to-child links; the default of two therefore requires a behaviorally equivalent grandchild. Only then is a `NEW_PROGRAM` event emitted. These measurements are passive observability: they never enter scheduling, mutation, resource allocation, fitness, or selection.

Heritable-identity-keyed reproduction, resource-transfer, activity, and counterfactual-removal accounting therefore preserve tag-defined clades. Each organism deposit snapshots the donor ID and heritable identity; exact per-origin quantities travel with both resource maps through merges, partial drains, decay, and opposing currents. Attribution therefore survives later donor tag/genome changes and death. The headless observer appends `tags(pop/births)` to each statistics line, and the TUI identity table shows each tag and its reproductive output; its title aggregates live frequency and births by tag.

Controls:

- `space`: pause
- `.`: advance one VM tick
- `+` / `-`: change speed by 10×
- `v`: cycle genome, ancestry, and resource views
- `up` / `down`: inspect another organism
- `y`: run replicated 100,000-tick partner-removal experiments on cloned worlds
- `x`: cancel the active partner-removal experiment
- `r`: start again from the single ancestor
- `q`: exit

The simulation records the donor behind each consumed deposit. Its behavior trace separately counts local foreign-organism and recognition-tag searches; the observer labels these as local foreign, local tag, or combined organism seekers. Both searches use the configured inclusive circular `interaction_radius`, check both directions, and cannot target remote organisms in one instruction.

Counterfactual tests first choose an active pair of heritable identities with direct cross-organism resource-transfer evidence, falling back to abundant complementary metabolisms when no such exchange exists. Each snapshot-time identity defines the roots of a focal clade; all parent-to-child descendants stay in that clade through later genome, recognition-tag, and behavior changes. This ancestry definition is independent of behavioral ecotypes.

Each deterministic replicate derives a domain-separated endogenous RNG seed from the exact birth-state digest, focal identities, and replicate index, then gives its intact, sham-intact, and two removal branches that same seed. All branches replay the same organism-independent resource schedule. Reproduction is normalized by instructions executed, and the report identifies the source state digest and gives paired mean losses with 95% Student-t intervals, effective sample and birth counts, replay-control failures, and direct cross-clade transfers separately from indirect ecological effects. A verdict requires at least two valid replicates, two intact births per clade per replicate on aggregate, passing controls, and intervals wholly beyond the predeclared 20% effect margin (or wholly within it for `NoEffect`); unavailable, low-evidence, and noisy results remain `Inconclusive`. These observer-only trackers never affect scheduling, mutation, resources, fitness, or selection.

For a reproducible headless run:

```sh
LOG_PATH=/tmp/soup.log cargo run --release --bin soup -- \
  --ticks 1000000 --test-symbiosis --symbiosis-horizon 100000 \
  --symbiosis-replicates 8
```

Configuration lives in `soup.toml`; `counterfactual_replicates` (or `COUNTERFACTUAL_REPLICATES`) sets the default replicate count. Resource sources have independently configurable chemistry, position, cadence, amount, width, and velocity. Their shared spatial origin is derived from `rng_seed`, and the ancestor is placed there so it can reach both chemistries without any source targeting live organisms. The other templates are retained as optional inoculations, but only `templates/01_ancestor.toml` has `seed = true` by default. Every default descendant therefore comes from the same minimal ancestor.

Resource movement and metabolism are bounded per scheduler turn. `max_resource_flux_per_instruction` caps every A/B `TAKE` and `EXCRETE`; `max_metabolism_per_instruction` caps every A/B `CONVERT` and the number of A+B pairs processed by `COMBINE_AB`. Register B remains the requested quantity, except B=0 requests the configured metabolic maximum rather than an unlimited conversion. The 256-unit defaults exceed each default source emission (at most 200 units), preserve the ancestor's deterministic long-run viability, and still prevent a single instruction from moving an accumulated `u32` store. These caps are physical rules only: they do not inspect identity, behavior, lineage, reproduction, or fitness.


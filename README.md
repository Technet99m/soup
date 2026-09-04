# Primordial Soup

A single 18-byte ancestor enters a circular 64 KiB memory and leaves descendants. Nothing is scored and no target program is supplied. Programs survive by executing bytecode, finding finite resources, converting distinct A and B metabolites, copying themselves, and paying for every instruction.

The redesign gives evolution room to change more than byte values:

- insertions, deletions, and gene-like span duplications change genome length;
- the ancestor measures its current length, so resized descendants can still reproduce;
- offspring usually occupy memory beside their parent, allowing persistent local ecologies;
- A and B resources come from organism-independent fixed or moving sources, flow in opposite currents, occupy separate internal stores, require different uptake/conversion/excretion instructions, and can only be found locally;
- a heritable, mutable tag can be set and searched for by programs;
- execution traces classify what a genome actually does, aggregating redundant raw opcode aliases under one decoded instruction while exact byte genomes remain distinct;
- all 256 bytes use a balanced redundant opcode encoding, and substitutions move through a documented local synonymous/adjacent-instruction kernel rather than an overwhelmingly NOP basin;
- suspected partnerships can be tested in cloned worlds with either partner removed.

Run the live evolution observer:

```sh
cargo run --release --bin viz
```

Start at 100 ticks per frame and use `+` to accelerate. The genome view colors live heritable identities (byte sequence plus recognition tag). White flashes identify recent mutations and attacks. The resource view uses cyan for A, magenta for B, and yellow where both occur. The right-hand table describes observed metabolism and interaction behavior.

## Recognition identity

`HeritableIdentity` is the raw pair `(genome hash, recognition tag)`. The explicit tag component keeps identical byte sequences with different partner-recognition behavior separate, while the genome component keeps unrelated programs that happen to use the same tag separate. “Ecotype” is reserved for a future persistent behavioral-viability concept rather than this raw key. Children inherit their parent's tag on both `COMMIT` and `SPLIT`. `SET_TAG` changes the executing organism's tag during its lifetime, and `tag_mutation_rate` independently replaces an inherited tag at birth (a mutation always chooses a value different from the inherited one). Birth lineage events record the child's complete heritable identity.

Heritable-identity-keyed reproduction, resource-transfer, activity, and counterfactual-removal accounting therefore preserve tag-defined clades. Each organism deposit snapshots the donor ID and heritable identity; exact per-origin quantities travel with both resource maps through merges, partial drains, decay, and opposing currents. Attribution therefore survives later donor tag/genome changes and death. The headless observer appends `tags(pop/births)` to each statistics line, and the TUI identity table shows each tag and its reproductive output; its title aggregates live frequency and births by tag.

Controls:

- `space`: pause
- `.`: advance one VM tick
- `+` / `-`: change speed by 10×
- `v`: cycle genome, ancestry, and resource views
- `up` / `down`: inspect another organism
- `y`: run a 100,000-tick partner-removal experiment on cloned worlds
- `r`: start again from the single ancestor
- `q`: exit

The simulation records the donor behind each consumed deposit. Counterfactual tests first choose an active pair of heritable identities with direct cross-organism resource-transfer evidence, falling back to abundant complementary metabolisms when no such exchange exists. The result reports how much each heritable identity's reproduction rate falls without the other. `Mutualism` requires both losses to reach 20% with at least two intact-world births each. That is evidence of ecological dependence under the current conditions, not proof that either program is semantically novel.

For a reproducible headless run:

```sh
LOG_PATH=/tmp/soup.log cargo run --release --bin soup -- \
  --ticks 1000000 --test-symbiosis --symbiosis-horizon 100000
```

Configuration lives in `soup.toml`. Resource sources have independently configurable chemistry, position, cadence, amount, width, and velocity. Their shared spatial origin is derived from `rng_seed`, and the ancestor is placed there so it can reach both chemistries without any source targeting live organisms. The other templates are retained as optional inoculations, but only `templates/01_ancestor.toml` has `seed = true` by default. Every default descendant therefore comes from the same minimal ancestor.

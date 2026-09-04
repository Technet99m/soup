# Primordial Soup

A single 16-byte ancestor enters a circular 64 KiB memory and leaves descendants. Nothing is scored and no target program is supplied. Programs survive by executing bytecode, finding finite resources, copying themselves, and paying for every instruction.

The redesign gives evolution room to change more than byte values:

- insertions, deletions, and gene-like span duplications change genome length;
- the ancestor measures its current length, so resized descendants can still reproduce;
- offspring usually occupy memory beside their parent, allowing persistent local ecologies;
- A and B resources move in opposite currents, require different instructions, and can only be found locally;
- a heritable, mutable tag can be set and searched for by programs;
- execution traces classify what a genome actually does, not merely how its bytes differ;
- suspected partnerships can be tested in cloned worlds with either partner removed.

Run the live evolution observer:

```sh
cargo run --release --bin viz
```

Start at 100 ticks per frame and use `+` to accelerate. The genome view colors exact live byte sequences. White flashes identify recent mutations and attacks. The resource view uses cyan for A, magenta for B, and yellow where both occur. The right-hand table describes observed metabolism and interaction behavior.

Controls:

- `space`: pause
- `.`: advance one VM tick
- `+` / `-`: change speed by 10×
- `v`: cycle genome, ancestry, and resource views
- `up` / `down`: inspect another organism
- `y`: run a 100,000-tick partner-removal experiment on cloned worlds
- `r`: start again from the single ancestor
- `q`: exit

The simulation records the donor behind each consumed deposit. Counterfactual tests first choose an active pair with direct cross-genome transfer evidence, falling back to abundant complementary metabolisms when no such exchange exists. The result reports how much each genome's reproduction rate falls without the other. `Mutualism` requires both losses to reach 20% with at least two intact-world births each. That is evidence of ecological dependence under the current conditions, not proof that either program is semantically novel.

For a reproducible headless run:

```sh
LOG_PATH=/tmp/soup.log cargo run --release --bin soup -- \
  --ticks 1000000 --test-symbiosis --symbiosis-horizon 100000
```

Configuration lives in `soup.toml`. The other templates are retained as optional inoculations, but only `templates/01_ancestor.toml` has `seed = true` by default. Every default descendant therefore comes from the same minimal ancestor.

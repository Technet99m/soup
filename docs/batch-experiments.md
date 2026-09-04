# Batch emergence experiments

`soup-batch` runs independent deterministic replicates and writes a resumable JSON document. It is an observer: measurements are collected from emitted events and final `World` state and are never supplied to scheduling, mutation, allocation, resources, counterfactual candidate selection, or survival.

## Run an experiment

Use exactly one seed source. Ranges support exclusive and inclusive Rust notation:

```sh
cargo run --release --bin soup-batch -- \
  --seeds 1000..=1099 \
  --ticks 1000000 \
  --counterfactual \
  --counterfactual-horizon 100000 \
  --config soup.toml \
  --output experiments/emergence-1000-1099.json
```

Or supply one unsigned decimal seed per line; blank lines, `#` comments, duplicates, and input order are normalized deterministically:

```sh
cargo run --release --bin soup-batch -- \
  --seed-file examples/batch-seeds.txt \
  --ticks 1000000 \
  --config soup.toml \
  --output experiments/selected-seeds.json
```

`--templates-dir PATH` overrides the effective config's template directory. Counterfactual trials are disabled unless `--counterfactual` is present. `--counterfactual-horizon` defaults to 100,000 ticks when enabled.

Each replicate seed replaces **only** `simulation_config.rng_seed`; every other effective simulation setting and the compiled Git commit stay fixed. Seeds are sorted and deduplicated, and replicate rows are always seed-ordered.

## Output and interpretation

The versioned JSON contains:

- `commit`, a BLAKE3 `source_fingerprint` covering dirty tracked/untracked source, `experiment`, the complete effective `simulation_config`, and the exact ordered startup-template names, descriptions, and bytes;
- one `replicates` row per attempted seed, including status, deterministic run namespace, final canonical state digest, and an error for isolated failures;
- horizon survival and final population;
- births and final persistent behavioral ecotypes;
- stable-new-behavior announcements (`NEW_PROGRAM` events);
- direct cross-organism transfer event counts and transferred amount;
- the complete counterfactual relationship verdict and evidence when a candidate pair exists;
- an energy-conservation check for every completed replicate;
- aggregate emergence rates with two-sided 95% Wilson score intervals;
- count means, sample standard deviations, minima, and maxima;
- requested/completed/failed/pending counts and relationship-verdict frequencies.

Rate denominators contain completed replicates only. Failed and pending seeds are reported separately rather than silently treated as negative outcomes; this also makes failure-selection bias visible. Wilson intervals assume the configured seeds are meaningful independent replicates. A relationship is `confirmed` only when the existing observer returns mutualism, one-way dependence, or competition; `inconclusive` and absent-candidate results are not counted as confirmed. When counterfactual inference is disabled, `confirmed_relationship_emergence` is `null` rather than a misleading zero rate.

`survived` means at least one program remains alive when the replicate stops. A lineage that dies before the requested horizon has `survived: false` and an earlier `ticks_completed`. `persistent_ecotypes` is the final observer count; `stable_new_behaviors` counts first-time stable ecotype announcements over the trajectory. These are related but intentionally distinct measurements.

## Resume and failures

The destination is exclusively locked and atomically replaced after each seed. Re-running the exact command skips completed seeds and retries failed or unattempted seeds. Resume fails closed if the schema, Git commit, source fingerprint, seeds, tick horizon, counterfactual settings, effective simulation configuration, or effective startup-template snapshot differs. The source fingerprint prevents a dirty build from being confused with another build at the same commit. Captured templates, rather than mutable files, initialize every replicate in one batch. Use a new output path for a changed experiment.

Ctrl-C stops the active replicate at a tick boundary, leaves that partial seed unrecorded, and preserves all earlier completed rows. The same command resumes from that seed. A panic in one replicate is recorded as a failed row and later seeds continue; malformed CLI input, seed files, configs, and incompatible resume files fail before simulation.

The JSON file is the machine-readable interface. Normal successful execution writes nothing to stdout; diagnostics go to stderr.

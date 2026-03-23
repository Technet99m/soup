# Seed Templates

Support for multiple named seed programs loaded from files, placed at startup, and tracked in viz.

---

## Template File Format

Directory: `templates/` at project root. Each file is `<nickname>.toml`:

```toml
name = "looper"           # nickname, used as display label
description = "..."       # human-readable strategy summary
bytes = [5, 31, 12, ...]  # u8 bytecode array
```

`bytes` is validated on load: all values 0–255, non-empty. Files are loaded in filename-sorted order for deterministic placement.

---

## Template Ideas

| Nickname | Distinct Strategy |
|----------|-------------------|
| **looper** | Baseline (current SEED). Uses `LOOP_OPEN`/`LOOP_CLOSE` for copy. Hardcoded size. Lots of NOP padding — easy evolution target. |
| **measurer** | Uses `MEASURE_SELF` instead of hardcoded length. More resilient to length mutations; offspring of different sizes still replicate correctly. |
| **scanner** | Uses `SCAN_FWD` to hunt energy deposits in memory before starting replication. May find leftover energy from dead programs to fund copies. |
| **jumper** | Uses `JMP_IF_ZERO` + `DEC` for the copy loop instead of structured `LOOP_OPEN`/`LOOP_CLOSE`. Different control flow, exposes different mutation targets. |
| **splitter** | Uses `SPLIT` instead of `COMMIT` — parent dies on each reproduction. Shorter program, faster generational turnover, but no parent survival. |

---

## Implementation

### `src/template.rs` (new)

```rust
pub struct Template {
    pub name: String,
    pub description: String,
    pub bytes: Vec<u8>,
}

pub fn load_templates(dir: &Path) -> Vec<Template>
```

- Reads all `*.toml` from `dir`, sorted by filename
- Skips/warns on bad files, does not panic
- Falls back to hardcoded SEED if dir is missing or empty

### `src/program.rs`

Add field:
```rust
pub template_id: Option<u8>,  // index into startup template list
```

`Program::new()` gains `template_id: Option<u8>` parameter. Children inherit `parent.template_id` (propagate in `vm.rs`).

### `src/config.rs`

Add:
```rust
pub templates_dir: PathBuf,  // default: "templates", env: SOUP_TEMPLATES_DIR
```

### `src/world.rs`

Replace single SEED placement with:
1. `load_templates(&config.templates_dir)`
2. Place each template at evenly spaced addresses: `offset = i * (65536 / num_templates)`
3. Create a `Program` per template with `template_id = Some(i as u8)`
4. Free list covers everything not occupied by any template

### `src/bin/viz.rs`

**Program list** — add "Tmpl" column (shows nickname, or `·` for None). Color by template index using a fixed palette separate from the ID-cycling one.

**Memory map** — add an "origins" display mode (cycle with `t` key: programs → energy → origins). In origins mode, cells are colored by `template_id` instead of program ID. This shows which lineage dominates spatially.

**Legend** — in origins mode, render `[color] nickname` for each template at the bottom of the memory panel.

**`App` struct** — add `template_names: Vec<String>` populated from the world at init.

---

## Files to Touch

| File | Change |
|------|--------|
| `src/template.rs` | new |
| `src/lib.rs` | add `pub mod template;` |
| `src/program.rs` | add `template_id` field |
| `src/vm.rs` | propagate `template_id` to children |
| `src/world.rs` | load + place all templates |
| `src/config.rs` | add `templates_dir` |
| `src/bin/viz.rs` | Tmpl column, origins mode, legend |
| `templates/*.toml` | template files (write the bytes yourself) |

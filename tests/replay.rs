use soup::{config::Config, world::World};
use std::{fs, path::PathBuf, process::Command};

fn config() -> Config {
    Config {
        templates_dir: PathBuf::from("/nonexistent_soup_replay_templates"),
        ..Config::default()
    }
}

#[test]
fn independent_worlds_and_clones_emit_identical_births() {
    let mut first = World::new(config());
    let mut second = World::new(config());
    let mut cloned = first.clone();
    let events = first.run(5_000);
    assert!(events
        .iter()
        .any(|e| matches!(e, soup::events::Event::Born { .. })));
    let bytes = serde_json::to_vec(&events).unwrap();
    assert!(
        bytes == serde_json::to_vec(&second.run(5_000)).unwrap(),
        "independent birth streams differ"
    );
    assert!(
        bytes == serde_json::to_vec(&cloned.run(5_000)).unwrap(),
        "cloned birth streams differ"
    );
}

#[test]
fn cloned_worlds_replay_strategy_mutation_in_births_and_digests() {
    let mut cfg = config();
    cfg.initial_energy = 10_000;
    cfg.mutation_rate = 0.0;
    cfg.insertion_rate = 0.0;
    cfg.deletion_rate = 0.0;
    cfg.duplication_rate = 0.0;
    cfg.tag_mutation_rate = 0.0;
    cfg.strategy_mutation_rate = 1.0;
    let mut first = World::new(cfg);
    let ancestor_strategy = first.programs[&0].mutation_strategy;
    let mut cloned = first.clone();

    let first_events = first.run(5_000);
    let cloned_events = cloned.run(5_000);
    assert_eq!(
        serde_json::to_vec(&first_events).unwrap(),
        serde_json::to_vec(&cloned_events).unwrap()
    );
    let child_strategy = first_events.iter().find_map(|event| match event {
        soup::events::Event::Born {
            heritable_identity, ..
        } => Some(heritable_identity.mutation_strategy),
        _ => None,
    });
    assert_ne!(
        child_strategy.expect("strategy-mutated child"),
        ancestor_strategy
    );
    assert_eq!(first.state_digest(), cloned.state_digest());
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("soup-replay-{}", std::process::id()));
        fs::create_dir_all(path.join("templates")).unwrap();
        fs::write(
            path.join("templates/ancestor.toml"),
            format!(
                "name='ancestor'\ndescription='replay fixture'\nbytes={:?}\n",
                soup::seed::SEED,
            ),
        )
        .unwrap();
        Self(path)
    }
    fn run(&self, name: &str, seed: u64, truncate: bool) -> (Vec<u8>, String) {
        let log = self.0.join(name);
        // The application appends. Each independent replay explicitly truncates first.
        if truncate {
            fs::write(&log, []).unwrap();
        }
        let output = Command::new(env!("CARGO_BIN_EXE_soup"))
            .env_clear()
            .env("SOUP_CONFIG", self.0.join("missing.toml"))
            .env("SOUP_TEMPLATES_DIR", self.0.join("templates"))
            .env("LOG_PATH", &log)
            .env("RNG_SEED", seed.to_string())
            .args(["--ticks", "5000", "--state-digest"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        let digest = stdout
            .lines()
            .find_map(|s| s.strip_prefix("State digest: "))
            .expect("--state-digest must print a final canonical digest")
            .to_owned();
        assert_eq!(digest.len(), 64);
        assert!(digest.bytes().all(|b| b.is_ascii_hexdigit()));
        (fs::read(log).unwrap(), digest)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn fresh_processes_replay_events_digest_and_deliberate_append() {
    let fixture = Fixture::new();
    let (first, digest) = fixture.run("first.jsonl", 42, true);
    assert!(String::from_utf8_lossy(&first).contains("\"type\":\"BORN\""));
    let (second, second_digest) = fixture.run("second.jsonl", 42, true);
    assert_eq!(first, second);
    assert_eq!(digest, second_digest);
    let (appended, appended_digest) = fixture.run("second.jsonl", 42, false);
    assert_eq!(appended, [first.as_slice(), first.as_slice()].concat());
    assert_eq!(digest, appended_digest);
    let (different, different_digest) = fixture.run("third.jsonl", 43, true);
    assert_ne!(first, different);
    assert_ne!(digest, different_digest);
    // File seed is overridden by RNG_SEED: resolved values, not TOML text, are hashed.
    fs::write(fixture.0.join("missing.toml"), "rng_seed=999\n").unwrap();
    let (overridden, overridden_digest) = fixture.run("overridden.jsonl", 42, true);
    assert!(first == overridden);
    assert_eq!(digest, overridden_digest);
    fs::write(fixture.0.join("missing.toml"), "initial_energy=5001\n").unwrap();
    let (_, changed_config_digest) = fixture.run("config.jsonl", 42, true);
    assert_ne!(digest, changed_config_digest);
    fs::write(fixture.0.join("missing.toml"), "").unwrap();
    fs::write(
        fixture.0.join("templates/ancestor.toml"),
        "name='ancestor'\ndescription='different genome'\nbytes=[255]\n",
    )
    .unwrap();
    let (_, changed_template_digest) = fixture.run("template.jsonl", 42, true);
    assert_ne!(digest, changed_template_digest);
}

#[test]
fn template_namespace_uses_loaded_semantics_and_order_not_paths_or_formatting() {
    let root = std::env::temp_dir().join(format!("soup-templates-replay-{}", std::process::id()));
    let fixture = Fixture(root);
    let left = fixture.0.join("left");
    let right = fixture.0.join("right");
    fs::create_dir_all(&left).unwrap();
    fs::create_dir_all(&right).unwrap();
    let a = "name='a'\ndescription='description'\nbytes=[0, 1, 2]\n";
    let b = "name='b'\ndescription='description'\nbytes=[3, 4]\n";
    fs::write(left.join("01.toml"), a).unwrap();
    fs::write(left.join("02.toml"), b).unwrap();
    fs::write(
        right.join("a.toml"),
        format!("# ignored formatting\n{a}seed=true\n"),
    )
    .unwrap();
    fs::write(right.join("z.toml"), b).unwrap();
    fs::write(
        right.join("ignored.toml"),
        "name='ignored'\ndescription=''\nbytes=[255]\nseed=false",
    )
    .unwrap();
    let make = |dir| {
        World::new(Config {
            templates_dir: dir,
            ..config()
        })
    };
    let original = make(left.clone());
    assert_eq!(
        original.run_namespace(),
        make(right.clone()).run_namespace()
    );
    assert_eq!(original.state_digest(), make(right.clone()).state_digest());
    // Description is authoring metadata; name and bytes are observer/simulation semantics.
    fs::write(
        right.join("a.toml"),
        a.replace(
            "description='description'",
            "description='edited documentation'",
        ),
    )
    .unwrap();
    assert_eq!(
        original.run_namespace(),
        make(right.clone()).run_namespace()
    );
    for different in [
        a.replace("name='a'", "name='renamed'"),
        a.replace("0, 1, 2", "0, 1, 3"),
    ] {
        fs::write(right.join("a.toml"), different).unwrap();
        assert_ne!(
            original.run_namespace(),
            make(right.clone()).run_namespace()
        );
    }
    fs::write(right.join("a.toml"), b).unwrap();
    fs::write(right.join("z.toml"), a).unwrap();
    assert_ne!(original.run_namespace(), make(right).run_namespace());
}

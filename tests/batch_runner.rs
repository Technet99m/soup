use soup::batch::{
    aggregate, parse_seed_range, run_pending, run_replicate, BatchInterrupted, BatchReport,
    CounterfactualSettings, ExperimentConfig, ReplicateResult, ReplicateStatus,
};
use soup::config::Config;
use std::{collections::BTreeSet, fs, path::PathBuf};

fn completed(seed: u64, survived: bool, births: u64, ecotypes: u64) -> ReplicateResult {
    ReplicateResult {
        seed,
        status: ReplicateStatus::Completed,
        error: None,
        run_namespace: Some("0".repeat(64)),
        final_state_digest: Some("1".repeat(64)),
        ticks_completed: 100,
        survived,
        final_population: usize::from(survived),
        births,
        persistent_ecotypes: ecotypes,
        stable_new_behaviors: ecotypes,
        direct_transfers: if seed == 2 { 3 } else { 0 },
        direct_transfer_amount: if seed == 2 { 9 } else { 0 },
        counterfactual_tested: false,
        relationship: None,
        conserved_energy: true,
    }
}

fn report(seeds: Vec<u64>) -> BatchReport {
    BatchReport::new(
        "test-commit".into(),
        ExperimentConfig {
            seeds,
            ticks: 100,
            counterfactual: CounterfactualSettings {
                enabled: false,
                horizon: 0,
            },
        },
        Config {
            templates_dir: PathBuf::from("/nonexistent_soup_batch_fixture"),
            ..Config::default()
        },
    )
}

#[test]
fn seed_ranges_are_inclusive_and_reject_ambiguous_input() {
    assert_eq!(parse_seed_range("7..=9").unwrap(), vec![7, 8, 9]);
    assert_eq!(parse_seed_range("7..10").unwrap(), vec![7, 8, 9]);
    assert!(parse_seed_range("9..=7").is_err());
    assert!(parse_seed_range("7-9").is_err());
    assert!(parse_seed_range("1..=18446744073709551615").is_err());
    assert!(parse_seed_range("1..=1000001").is_err());
}

#[test]
fn deterministic_fixture_reports_rates_uncertainty_and_count_summaries() {
    let results = vec![
        completed(1, true, 2, 0),
        completed(2, false, 4, 1),
        ReplicateResult::failed(3, "fixture failure"),
    ];

    let summary = aggregate(&results);

    assert_eq!(summary.requested, 3);
    assert_eq!(summary.completed, 2);
    assert_eq!(summary.failed, 1);
    assert_eq!(summary.survival.numerator, 1);
    assert_eq!(summary.survival.denominator, 2);
    assert_eq!(summary.survival.estimate, 0.5);
    assert!(summary.survival.ci95.lower < 0.5);
    assert!(summary.survival.ci95.upper > 0.5);
    assert_eq!(summary.persistent_ecotype_emergence.numerator, 1);
    assert_eq!(summary.direct_transfer_emergence.numerator, 1);
    assert!(summary.confirmed_relationship_emergence.is_none());
    assert_eq!(summary.births.mean, 3.0);
    assert_eq!(summary.births.min, 2);
    assert_eq!(summary.births.max, 4);
}

#[test]
fn resume_skips_completed_seeds_retries_failures_and_is_byte_deterministic() {
    let dir = std::env::temp_dir().join(format!("soup-batch-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let output = dir.join("results.json");
    let mut first = report(vec![11, 12, 13]);
    first.replicates.push(completed(11, true, 1, 0));
    first
        .replicates
        .push(ReplicateResult::failed(12, "old failure"));
    first.refresh_aggregate();
    first.write_atomic(&output).unwrap();

    let mut called = BTreeSet::new();
    run_pending(&mut first, &output, |seed| {
        called.insert(seed);
        Ok(completed(seed, true, seed, 0))
    })
    .unwrap();

    assert_eq!(called, BTreeSet::from([12, 13]));
    let bytes_once = fs::read(&output).unwrap();
    let loaded = BatchReport::read(&output).unwrap();
    assert_eq!(
        loaded.replicates.iter().map(|r| r.seed).collect::<Vec<_>>(),
        vec![11, 12, 13]
    );

    let mut called_again = BTreeSet::new();
    let mut loaded_again = loaded;
    run_pending(&mut loaded_again, &output, |seed| {
        called_again.insert(seed);
        Ok(completed(seed, true, seed, 0))
    })
    .unwrap();
    assert!(called_again.is_empty());
    assert_eq!(bytes_once, fs::read(&output).unwrap());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn resume_rejects_a_changed_experiment_configuration() {
    let dir = std::env::temp_dir().join(format!("soup-batch-mismatch-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let output = dir.join("results.json");
    report(vec![1, 2]).write_atomic(&output).unwrap();

    let requested = report(vec![1, 3]);
    let error = BatchReport::resume_or_new(&output, requested).unwrap_err();

    assert!(error.to_string().contains("does not match"));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn resume_rejects_changed_effective_templates() {
    let dir = std::env::temp_dir().join(format!("soup-batch-template-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let template = dir.join("ancestor.toml");
    let output = dir.join("results.json");
    fs::write(
        &template,
        "name = \"fixture\"\ndescription = \"first\"\nbytes = [1, 2, 3]\nseed = true\n",
    )
    .unwrap();
    let config = Config {
        templates_dir: dir.clone(),
        ..Config::default()
    };
    let mut initial = BatchReport::new(
        "test-commit".into(),
        ExperimentConfig {
            seeds: vec![1],
            ticks: 10,
            counterfactual: CounterfactualSettings {
                enabled: false,
                horizon: 0,
            },
        },
        config.clone(),
    );
    initial.refresh_aggregate();
    initial.write_atomic(&output).unwrap();
    fs::write(
        &template,
        "name = \"fixture\"\ndescription = \"changed\"\nbytes = [1, 2, 4]\nseed = true\n",
    )
    .unwrap();

    let requested = BatchReport::new(
        "test-commit".into(),
        ExperimentConfig {
            seeds: vec![1],
            ticks: 10,
            counterfactual: CounterfactualSettings {
                enabled: false,
                horizon: 0,
            },
        },
        config,
    );
    assert!(BatchReport::resume_or_new(&output, requested).is_err());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn resume_rejects_corrupt_completed_checkpoint_rows() {
    let dir = std::env::temp_dir().join(format!("soup-batch-corrupt-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let output = dir.join("results.json");
    let mut initial = report(vec![1]);
    initial.replicates.push(completed(1, true, 1, 0));
    initial.refresh_aggregate();
    initial.write_atomic(&output).unwrap();

    let mut json: serde_json::Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    json["replicates"][0]["conserved_energy"] = serde_json::Value::Bool(false);
    fs::write(&output, serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(BatchReport::read(&output).is_err());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn repeated_replicates_are_identical_and_conserve_energy() {
    let config = Config {
        templates_dir: PathBuf::from("/nonexistent_soup_batch_determinism"),
        mutation_rate: 0.0,
        insertion_rate: 0.0,
        deletion_rate: 0.0,
        duplication_rate: 0.0,
        tag_mutation_rate: 0.0,
        ..Config::default()
    };
    let counterfactual = CounterfactualSettings {
        enabled: false,
        horizon: 0,
    };

    let first = run_replicate(&config, 77, 2_000, &counterfactual, || true).unwrap();
    let second = run_replicate(&config, 77, 2_000, &counterfactual, || true).unwrap();

    assert_eq!(first, second);
    assert_eq!(first.status, ReplicateStatus::Completed);
    assert!(first.conserved_energy);
    assert_eq!(first.seed, 77);
    assert_eq!(first.run_namespace.as_ref().unwrap().len(), 64);
    assert_eq!(first.final_state_digest.as_ref().unwrap().len(), 64);
}

#[test]
fn interruption_preserves_completed_seeds_without_recording_the_partial_seed() {
    let dir = std::env::temp_dir().join(format!("soup-batch-interrupt-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let output = dir.join("results.json");
    let mut batch = report(vec![1, 2, 3]);

    let error = run_pending(&mut batch, &output, |seed| {
        if seed == 1 {
            Ok(completed(seed, true, 1, 0))
        } else {
            Err(Box::new(BatchInterrupted))
        }
    })
    .unwrap_err();

    assert!(error.downcast_ref::<BatchInterrupted>().is_some());
    let saved = BatchReport::read(&output).unwrap();
    assert_eq!(saved.replicates.len(), 1);
    assert_eq!(saved.replicates[0].seed, 1);
    assert_eq!(saved.aggregate.requested, 3);
    assert_eq!(saved.aggregate.completed, 1);
    assert_eq!(saved.aggregate.failed, 0);
    assert_eq!(saved.aggregate.pending, 2);
    fs::remove_dir_all(&dir).unwrap();
}

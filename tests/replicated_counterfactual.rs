use soup::{
    identity::HeritableIdentity,
    world::{summarize_counterfactual_samples, CounterfactualSample, RelationshipVerdict},
};

fn pair() -> (HeritableIdentity, HeritableIdentity) {
    (
        HeritableIdentity::new(0xaaa, 1),
        HeritableIdentity::new(0xbbb, 2),
    )
}

fn sample(
    intact_a: u64,
    without_b_a: u64,
    intact_b: u64,
    without_a_b: u64,
) -> CounterfactualSample {
    CounterfactualSample {
        intact_births_a: intact_a,
        without_b_births_a: without_b_a,
        intact_births_b: intact_b,
        without_a_births_b: without_a_b,
        intact_steps_a: 100,
        without_b_steps_a: 100,
        intact_steps_b: 100,
        without_a_steps_b: 100,
        a_received_from_b: 0,
        b_received_from_a: 0,
        sham_matches_intact: true,
        resource_schedule_matches: true,
    }
}

fn repeated(value: CounterfactualSample) -> Vec<CounterfactualSample> {
    vec![value; 8]
}

#[test]
fn deterministic_fixture_classifies_mutualism() {
    let report = summarize_counterfactual_samples(pair(), 1_000, &repeated(sample(10, 5, 10, 5)));

    assert_eq!(report.verdict, RelationshipVerdict::Mutualism);
    assert_eq!(report.replicates, 8);
    assert_eq!(report.dependence_a_samples, 8);
    assert_eq!(report.dependence_b_samples, 8);
    assert_eq!(report.dependence_a, 0.5);
    assert_eq!(report.dependence_a_interval.unwrap().lower, 0.5);
    assert_eq!(report.dependence_a_interval.unwrap().upper, 0.5);
}

#[test]
fn deterministic_fixture_classifies_competition() {
    let report = summarize_counterfactual_samples(pair(), 1_000, &repeated(sample(10, 15, 10, 15)));
    assert_eq!(report.verdict, RelationshipVerdict::Competition);
}

#[test]
fn deterministic_fixture_classifies_one_way_dependence() {
    let report = summarize_counterfactual_samples(pair(), 1_000, &repeated(sample(10, 5, 10, 10)));
    assert_eq!(report.verdict, RelationshipVerdict::ADependsOnB);
}

#[test]
fn deterministic_fixture_classifies_no_effect() {
    let report = summarize_counterfactual_samples(pair(), 1_000, &repeated(sample(10, 10, 10, 10)));
    assert_eq!(report.verdict, RelationshipVerdict::NoEffect);
}

#[test]
fn low_evidence_and_single_replicate_are_inconclusive() {
    let low_births = summarize_counterfactual_samples(pair(), 1_000, &repeated(sample(1, 0, 1, 0)));
    assert_eq!(low_births.verdict, RelationshipVerdict::Inconclusive);

    let unreplicated = summarize_counterfactual_samples(pair(), 1_000, &[sample(10, 0, 10, 0)]);
    assert_eq!(unreplicated.verdict, RelationshipVerdict::Inconclusive);
}

#[test]
fn zero_intact_birth_replicates_are_excluded_not_treated_as_no_effect() {
    let mut sparse = vec![sample(0, 10, 0, 10); 7];
    sparse.push(sample(16, 16, 16, 16));

    let report = summarize_counterfactual_samples(pair(), 1_000, &sparse);

    assert_eq!(report.verdict, RelationshipVerdict::Inconclusive);
    assert_eq!(report.dependence_a_samples, 1);
    assert_eq!(report.dependence_b_samples, 1);
}

#[test]
fn student_t_interval_remains_conservative_above_thirty_replicates() {
    let mut samples = vec![sample(10, 10, 10, 10); 16];
    samples.extend(vec![sample(10, 6, 10, 6); 16]);
    let report = summarize_counterfactual_samples(pair(), 1_000, &samples);
    let effects: Vec<f64> = (0..32)
        .map(|index| if index < 16 { 0.0 } else { 0.4 })
        .collect();
    let mean = effects.iter().sum::<f64>() / effects.len() as f64;
    let variance = effects
        .iter()
        .map(|effect| (effect - mean).powi(2))
        .sum::<f64>()
        / (effects.len() - 1) as f64;
    let normal_margin = 1.96 * (variance / effects.len() as f64).sqrt();

    let interval = report.dependence_a_interval.unwrap();
    let standard_error = (variance / effects.len() as f64).sqrt();
    let critical = (interval.upper - mean) / standard_error;
    assert!(interval.upper - mean > normal_margin);
    assert!(mean - interval.lower > normal_margin);
    assert!(critical >= 2.039_513_446);
}

#[test]
fn noisy_effects_and_failed_controls_are_inconclusive() {
    let mut noisy = Vec::new();
    for index in 0..8 {
        noisy.push(if index % 2 == 0 {
            sample(10, 5, 10, 5)
        } else {
            sample(10, 15, 10, 15)
        });
    }
    assert_eq!(
        summarize_counterfactual_samples(pair(), 1_000, &noisy).verdict,
        RelationshipVerdict::Inconclusive
    );

    let mut failed_control = repeated(sample(10, 5, 10, 5));
    failed_control[3].resource_schedule_matches = false;
    let report = summarize_counterfactual_samples(pair(), 1_000, &failed_control);
    assert_eq!(report.verdict, RelationshipVerdict::Inconclusive);
    assert_eq!(report.control_failures, 1);
}

#[test]
fn direct_transfer_evidence_is_not_conflated_with_ecological_dependence() {
    let mut samples = repeated(sample(10, 10, 10, 10));
    for sample in &mut samples {
        sample.a_received_from_b = 7;
        sample.b_received_from_a = 3;
    }
    let report = summarize_counterfactual_samples(pair(), 1_000, &samples);

    assert_eq!(report.verdict, RelationshipVerdict::NoEffect);
    assert_eq!(report.direct_transfer.a_received_from_b, 56);
    assert_eq!(report.direct_transfer.b_received_from_a, 24);
    assert_eq!(report.direct_transfer.sample_count, 8);
}

#[test]
fn aggregate_counts_saturate_instead_of_wrapping() {
    let samples = vec![
        CounterfactualSample {
            a_received_from_b: u64::MAX,
            b_received_from_a: u64::MAX,
            ..sample(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        };
        2
    ];

    let report = summarize_counterfactual_samples(pair(), 1_000, &samples);

    assert_eq!(report.baseline_births_a, u64::MAX);
    assert_eq!(report.baseline_births_b, u64::MAX);
    assert_eq!(report.direct_transfer.a_received_from_b, u64::MAX);
    assert_eq!(report.direct_transfer.b_received_from_a, u64::MAX);
}

#[test]
fn extreme_countereffects_are_not_clipped_out_of_inference() {
    let mut samples = vec![sample(10, 0, 10, 0); 63];
    samples.push(CounterfactualSample {
        intact_births_a: 10,
        without_b_births_a: 10_010,
        intact_births_b: 10,
        without_a_births_b: 10_010,
        ..sample(10, 0, 10, 0)
    });

    let report = summarize_counterfactual_samples(pair(), 1_000, &samples);

    assert_eq!(report.verdict, RelationshipVerdict::Inconclusive);
    assert!(report.dependence_a < -1.0);
}

#[test]
fn empty_samples_are_safe_and_inconclusive() {
    let report = summarize_counterfactual_samples(pair(), 0, &[]);
    assert_eq!(report.verdict, RelationshipVerdict::Inconclusive);
    assert_eq!(report.replicates, 0);
    assert_eq!(report.dependence_a, 0.0);
    assert!(report.dependence_a_interval.is_none());
    assert!(report.dependence_b_interval.is_none());
}

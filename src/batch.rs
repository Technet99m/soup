//! Deterministic, observer-only batch experiments.
//!
//! Measurements in this module consume `World` state and emitted events. They
//! are never passed back into scheduling, mutation, allocation, or selection.

use crate::{
    config::Config,
    events::Event,
    template,
    world::{RelationshipVerdict, World},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt, fs,
    fs::OpenOptions,
    io::Write,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_REPLICATES: u64 = 1_000_000;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

type BoxError = Box<dyn Error + Send + Sync>;

#[derive(Debug)]
pub struct BatchInterrupted;

impl fmt::Display for BatchInterrupted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("batch interrupted")
    }
}

impl Error for BatchInterrupted {}

#[derive(Debug)]
pub struct BatchInputChanged(pub String);

impl fmt::Display for BatchInputChanged {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for BatchInputChanged {}

fn is_hex_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CounterfactualSettings {
    pub enabled: bool,
    pub horizon: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExperimentConfig {
    /// Sorted, unique replicate seeds. Each replaces only `Config::rng_seed`.
    pub seeds: Vec<u64>,
    pub ticks: u64,
    pub counterfactual: CounterfactualSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EffectiveTemplate {
    pub name: String,
    pub description: String,
    pub bytes: Vec<u8>,
}

pub fn effective_templates(config: &Config) -> Vec<EffectiveTemplate> {
    template::load_templates(&config.templates_dir)
        .into_iter()
        .map(|template| EffectiveTemplate {
            name: template.name,
            description: template.description,
            bytes: template.bytes,
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BatchReport {
    pub schema_version: u32,
    pub commit: String,
    /// BLAKE3 of commit plus tracked diff and untracked source content at build time.
    pub source_fingerprint: String,
    pub experiment: ExperimentConfig,
    pub simulation_config: Config,
    /// Exact ordered startup programs used by `World::new`.
    pub effective_templates: Vec<EffectiveTemplate>,
    pub replicates: Vec<ReplicateResult>,
    pub aggregate: AggregateSummary,
}

impl BatchReport {
    pub fn new(
        commit: String,
        mut experiment: ExperimentConfig,
        simulation_config: Config,
    ) -> Self {
        experiment.seeds.sort_unstable();
        experiment.seeds.dedup();
        let effective_templates = effective_templates(&simulation_config);
        let source_fingerprint = commit.clone();
        let mut report = Self {
            schema_version: SCHEMA_VERSION,
            commit,
            source_fingerprint,
            experiment,
            simulation_config,
            effective_templates,
            replicates: Vec::new(),
            aggregate: aggregate(&[]),
        };
        report.refresh_aggregate();
        report
    }

    pub fn refresh_aggregate(&mut self) {
        self.replicates.sort_by_key(|replicate| replicate.seed);
        self.aggregate = aggregate(&self.replicates);
        self.aggregate.requested = self.experiment.seeds.len() as u64;
        self.aggregate.pending = self
            .aggregate
            .requested
            .saturating_sub(self.aggregate.completed + self.aggregate.failed);
    }

    pub fn read(path: &Path) -> Result<Self, BoxError> {
        let bytes = fs::read(path)?;
        let mut report: Self = serde_json::from_slice(&bytes)?;
        if report.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "unsupported batch schema version {} (expected {SCHEMA_VERSION})",
                report.schema_version
            )
            .into());
        }
        if report.experiment.seeds.is_empty() || report.experiment.ticks == 0 {
            return Err("batch report has an empty seed set or zero tick horizon".into());
        }
        if report
            .experiment
            .seeds
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        {
            return Err("batch report seeds must be sorted and unique".into());
        }
        let requested: BTreeSet<_> = report.experiment.seeds.iter().copied().collect();
        let mut observed = BTreeSet::new();
        for replicate in &report.replicates {
            if !requested.contains(&replicate.seed) || !observed.insert(replicate.seed) {
                return Err("batch report contains a duplicate or unrequested replicate".into());
            }
            match replicate.status {
                ReplicateStatus::Completed
                    if replicate.error.is_some()
                        || !replicate
                            .run_namespace
                            .as_deref()
                            .is_some_and(is_hex_digest)
                        || !replicate
                            .final_state_digest
                            .as_deref()
                            .is_some_and(is_hex_digest)
                        || replicate.ticks_completed > report.experiment.ticks
                        || (replicate.survived
                            && replicate.ticks_completed != report.experiment.ticks)
                        || replicate.counterfactual_tested
                            != report.experiment.counterfactual.enabled
                        || replicate.survived != (replicate.final_population > 0)
                        || !replicate.conserved_energy
                        || (!replicate.counterfactual_tested
                            && replicate.relationship.is_some()) =>
                {
                    return Err("batch report contains an inconsistent completed replicate".into());
                }
                ReplicateStatus::Failed
                    if replicate.error.is_none()
                        || replicate.run_namespace.is_some()
                        || replicate.final_state_digest.is_some() =>
                {
                    return Err("batch report contains an inconsistent failed replicate".into());
                }
                _ => {}
            }
        }
        let mut normalized = report.clone();
        normalized.refresh_aggregate();
        if normalized.replicates != report.replicates {
            return Err("batch report replicate ordering is inconsistent".into());
        }
        // Aggregates are derived data. Regenerate them from validated replicate
        // rows rather than trusting serialized floating-point summaries.
        report.aggregate = normalized.aggregate;
        Ok(report)
    }

    pub fn resume_or_new(path: &Path, requested: Self) -> Result<Self, BoxError> {
        if !path.exists() {
            return Ok(requested);
        }
        let existing = Self::read(path)?;
        if existing.commit != requested.commit
            || existing.source_fingerprint != requested.source_fingerprint
            || existing.experiment != requested.experiment
            || existing.simulation_config != requested.simulation_config
            || existing.effective_templates != requested.effective_templates
        {
            return Err("existing output configuration does not match this experiment".into());
        }
        Ok(existing)
    }

    /// Replace the destination atomically so interruption cannot leave partial JSON.
    pub fn write_atomic(&self, path: &Path) -> Result<(), BoxError> {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("output path must have a UTF-8 file name")?;
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(
            ".{file_name}.{}.{}.tmp",
            std::process::id(),
            sequence
        ));
        let bytes = serde_json::to_vec_pretty(self)?;
        let result = (|| -> Result<(), BoxError> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

pub fn run_pending<F>(report: &mut BatchReport, output: &Path, mut run: F) -> Result<(), BoxError>
where
    F: FnMut(u64) -> Result<ReplicateResult, BoxError>,
{
    let completed: BTreeSet<_> = report
        .replicates
        .iter()
        .filter(|replicate| replicate.status == ReplicateStatus::Completed)
        .map(|replicate| replicate.seed)
        .collect();
    for seed in report.experiment.seeds.clone() {
        if completed.contains(&seed) {
            continue;
        }
        report.replicates.retain(|replicate| replicate.seed != seed);
        let result = match run(seed) {
            Ok(result) => result,
            Err(error)
                if error.downcast_ref::<BatchInterrupted>().is_some()
                    || error.downcast_ref::<BatchInputChanged>().is_some() =>
            {
                report.refresh_aggregate();
                report.write_atomic(output)?;
                return Err(error);
            }
            Err(error) => ReplicateResult::failed(seed, error.to_string()),
        };
        if result.seed != seed {
            return Err(format!(
                "runner returned seed {} for requested seed {seed}",
                result.seed
            )
            .into());
        }
        report.replicates.push(result);
        report.refresh_aggregate();
        report.write_atomic(output)?;
    }
    // Also materialize an empty or already-complete experiment deterministically.
    report.refresh_aggregate();
    report.write_atomic(output)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReplicateStatus {
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RelationshipResult {
    pub verdict: String,
    pub confirmed: bool,
    pub horizon: u64,
    pub identity_a_genome: u64,
    pub identity_a_tag: u8,
    pub identity_b_genome: u64,
    pub identity_b_tag: u8,
    pub baseline_births_a: u64,
    pub baseline_births_b: u64,
    pub dependence_a: f64,
    pub dependence_b: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReplicateResult {
    pub seed: u64,
    pub status: ReplicateStatus,
    pub error: Option<String>,
    pub run_namespace: Option<String>,
    pub final_state_digest: Option<String>,
    pub ticks_completed: u64,
    pub survived: bool,
    pub final_population: usize,
    pub births: u64,
    pub persistent_ecotypes: u64,
    pub stable_new_behaviors: u64,
    pub direct_transfers: u64,
    pub direct_transfer_amount: u64,
    /// Whether relationship inference was requested for this completed replicate.
    pub counterfactual_tested: bool,
    pub relationship: Option<RelationshipResult>,
    pub conserved_energy: bool,
}

impl ReplicateResult {
    pub fn failed(seed: u64, error: impl Into<String>) -> Self {
        Self {
            seed,
            status: ReplicateStatus::Failed,
            error: Some(error.into()),
            run_namespace: None,
            final_state_digest: None,
            ticks_completed: 0,
            survived: false,
            final_population: 0,
            births: 0,
            persistent_ecotypes: 0,
            stable_new_behaviors: 0,
            direct_transfers: 0,
            direct_transfer_amount: 0,
            counterfactual_tested: false,
            relationship: None,
            conserved_energy: false,
        }
    }
}

/// Run one seed. The seed replaces only `rng_seed`; all observables are read
/// after world updates and cannot influence the simulation trajectory.
pub fn run_replicate<F>(
    simulation_config: &Config,
    seed: u64,
    ticks: u64,
    counterfactual: &CounterfactualSettings,
    should_continue: F,
) -> Result<ReplicateResult, BoxError>
where
    F: FnMut() -> bool,
{
    let templates = effective_templates(simulation_config);
    run_replicate_with_templates(
        simulation_config,
        &templates,
        seed,
        ticks,
        counterfactual,
        should_continue,
    )
}

pub fn run_replicate_with_templates<F>(
    simulation_config: &Config,
    templates: &[EffectiveTemplate],
    seed: u64,
    ticks: u64,
    counterfactual: &CounterfactualSettings,
    mut should_continue: F,
) -> Result<ReplicateResult, BoxError>
where
    F: FnMut() -> bool,
{
    let mut config = simulation_config.clone();
    config.rng_seed = seed;
    let expected_energy = config.total_energy;
    let templates = templates
        .iter()
        .map(|template| template::Template {
            name: template.name.clone(),
            description: template.description.clone(),
            bytes: template.bytes.clone(),
            seed: true,
        })
        .collect();
    let mut world = World::new_with_templates(config, templates);
    let mut stable_new_behaviors = 0_u64;
    let mut direct_transfers = 0_u64;
    let mut direct_transfer_amount = 0_u64;

    while world.tick < ticks && world.live_count() > 0 {
        if !should_continue() {
            return Err(Box::new(BatchInterrupted));
        }
        for event in world.tick() {
            match event {
                Event::NewProgram { .. } => {
                    stable_new_behaviors = stable_new_behaviors
                        .checked_add(1)
                        .ok_or("stable behavior count overflow")?;
                }
                Event::ResourceTransfer { amount, .. } => {
                    direct_transfers = direct_transfers
                        .checked_add(1)
                        .ok_or("direct transfer count overflow")?;
                    direct_transfer_amount = direct_transfer_amount
                        .checked_add(u64::from(amount))
                        .ok_or("direct transfer amount overflow")?;
                }
                _ => {}
            }
        }
    }

    let relationship = if counterfactual.enabled {
        if !should_continue() {
            return Err(Box::new(BatchInterrupted));
        }
        match world.candidate_partner_pair() {
            Some(pair) => {
                let report = world.counterfactual_symbiosis_for_pair_with_control(
                    pair,
                    counterfactual.horizon,
                    |_| should_continue(),
                );
                let Some(report) = report else {
                    return Err(Box::new(BatchInterrupted));
                };
                let confirmed = !matches!(
                    report.verdict,
                    RelationshipVerdict::NoEffect | RelationshipVerdict::Inconclusive
                );
                Some(RelationshipResult {
                    verdict: verdict_name(report.verdict).into(),
                    confirmed,
                    horizon: report.horizon,
                    identity_a_genome: report.heritable_identity_a.genome,
                    identity_a_tag: report.heritable_identity_a.tag,
                    identity_b_genome: report.heritable_identity_b.genome,
                    identity_b_tag: report.heritable_identity_b.tag,
                    baseline_births_a: report.baseline_births_a,
                    baseline_births_b: report.baseline_births_b,
                    dependence_a: report.dependence_a,
                    dependence_b: report.dependence_b,
                })
            }
            None => None,
        }
    } else {
        None
    };

    Ok(ReplicateResult {
        seed,
        status: ReplicateStatus::Completed,
        error: None,
        run_namespace: Some(
            blake3::Hash::from_bytes(world.run_namespace())
                .to_hex()
                .to_string(),
        ),
        final_state_digest: Some(world.state_digest()),
        ticks_completed: world.tick,
        survived: world.live_count() > 0,
        final_population: world.live_count(),
        births: world.total_births,
        persistent_ecotypes: world.viable_ecotype_count() as u64,
        stable_new_behaviors,
        direct_transfers,
        direct_transfer_amount,
        counterfactual_tested: counterfactual.enabled,
        relationship,
        conserved_energy: world.accounted_budget() == expected_energy,
    })
}

fn verdict_name(verdict: RelationshipVerdict) -> &'static str {
    match verdict {
        RelationshipVerdict::Mutualism => "mutualism",
        RelationshipVerdict::ADependsOnB => "a_depends_on_b",
        RelationshipVerdict::BDependsOnA => "b_depends_on_a",
        RelationshipVerdict::Competition => "competition",
        RelationshipVerdict::NoEffect => "no_effect",
        RelationshipVerdict::Inconclusive => "inconclusive",
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct ConfidenceInterval {
    pub confidence: f64,
    pub lower: f64,
    pub upper: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct RateSummary {
    pub numerator: u64,
    pub denominator: u64,
    pub estimate: f64,
    pub ci95: ConfidenceInterval,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct CountSummary {
    pub n: u64,
    pub mean: f64,
    pub sample_stddev: f64,
    pub min: u64,
    pub max: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AggregateSummary {
    pub requested: u64,
    pub completed: u64,
    pub failed: u64,
    pub pending: u64,
    pub survival: RateSummary,
    pub persistent_ecotype_emergence: RateSummary,
    pub stable_behavior_emergence: RateSummary,
    pub direct_transfer_emergence: RateSummary,
    /// `None` when counterfactual inference was disabled for all completed seeds.
    pub confirmed_relationship_emergence: Option<RateSummary>,
    pub energy_conservation: RateSummary,
    pub births: CountSummary,
    pub persistent_ecotypes: CountSummary,
    pub stable_new_behaviors: CountSummary,
    pub direct_transfers: CountSummary,
    pub direct_transfer_amount: CountSummary,
    pub relationship_verdicts: BTreeMap<String, u64>,
}

pub fn aggregate(results: &[ReplicateResult]) -> AggregateSummary {
    let completed: Vec<_> = results
        .iter()
        .filter(|result| result.status == ReplicateStatus::Completed)
        .collect();
    let denominator = completed.len() as u64;
    let rate = |predicate: fn(&ReplicateResult) -> bool| {
        rate_summary(
            completed.iter().filter(|result| predicate(result)).count() as u64,
            denominator,
        )
    };
    let mut relationship_verdicts = BTreeMap::new();
    for relationship in completed
        .iter()
        .filter_map(|result| result.relationship.as_ref())
    {
        *relationship_verdicts
            .entry(relationship.verdict.clone())
            .or_insert(0) += 1;
    }
    AggregateSummary {
        requested: results.len() as u64,
        completed: denominator,
        failed: results.len() as u64 - denominator,
        pending: 0,
        survival: rate(|result| result.survived),
        persistent_ecotype_emergence: rate(|result| result.persistent_ecotypes > 0),
        stable_behavior_emergence: rate(|result| result.stable_new_behaviors > 0),
        direct_transfer_emergence: rate(|result| result.direct_transfers > 0),
        confirmed_relationship_emergence: {
            let tested: Vec<_> = completed
                .iter()
                .filter(|result| result.counterfactual_tested)
                .collect();
            (!tested.is_empty()).then(|| {
                rate_summary(
                    tested
                        .iter()
                        .filter(|result| {
                            result
                                .relationship
                                .as_ref()
                                .is_some_and(|relationship| relationship.confirmed)
                        })
                        .count() as u64,
                    tested.len() as u64,
                )
            })
        },
        energy_conservation: rate(|result| result.conserved_energy),
        births: count_summary(&completed, |result| result.births),
        persistent_ecotypes: count_summary(&completed, |result| result.persistent_ecotypes),
        stable_new_behaviors: count_summary(&completed, |result| result.stable_new_behaviors),
        direct_transfers: count_summary(&completed, |result| result.direct_transfers),
        direct_transfer_amount: count_summary(&completed, |result| result.direct_transfer_amount),
        relationship_verdicts,
    }
}

fn rate_summary(numerator: u64, denominator: u64) -> RateSummary {
    if denominator == 0 {
        return RateSummary {
            numerator,
            denominator,
            estimate: 0.0,
            ci95: ConfidenceInterval {
                confidence: 0.95,
                lower: 0.0,
                upper: 1.0,
            },
        };
    }
    let n = denominator as f64;
    let estimate = numerator as f64 / n;
    let z = 1.959_963_984_540_054_f64;
    if numerator == 0 {
        return RateSummary {
            numerator,
            denominator,
            estimate: 0.0,
            ci95: ConfidenceInterval {
                confidence: 0.95,
                lower: 0.0,
                upper: (z * z / n) / (1.0 + z * z / n),
            },
        };
    }
    let denominator_term = 1.0 + z * z / n;
    let center = (estimate + z * z / (2.0 * n)) / denominator_term;
    let half_width =
        z * ((estimate * (1.0 - estimate) / n + z * z / (4.0 * n * n)).sqrt()) / denominator_term;
    RateSummary {
        numerator,
        denominator,
        estimate,
        ci95: ConfidenceInterval {
            confidence: 0.95,
            lower: (center - half_width).max(0.0),
            upper: (center + half_width).min(1.0),
        },
    }
}

fn count_summary<F>(results: &[&ReplicateResult], value: F) -> CountSummary
where
    F: Fn(&ReplicateResult) -> u64,
{
    if results.is_empty() {
        return CountSummary {
            n: 0,
            mean: 0.0,
            sample_stddev: 0.0,
            min: 0,
            max: 0,
        };
    }
    let values: Vec<_> = results.iter().map(|result| value(result)).collect();
    let base = values[0];
    let deviations: Vec<i128> = values
        .iter()
        .map(|item| i128::from(*item) - i128::from(base))
        .collect();
    let mean = base as f64 + deviations.iter().sum::<i128>() as f64 / values.len() as f64;
    let sample_stddev = if values.len() < 2 {
        0.0
    } else {
        let deviation_mean = deviations.iter().sum::<i128>() as f64 / values.len() as f64;
        (deviations
            .iter()
            .map(|item| (*item as f64 - deviation_mean).powi(2))
            .sum::<f64>()
            / (values.len() - 1) as f64)
            .sqrt()
    };
    CountSummary {
        n: values.len() as u64,
        mean,
        sample_stddev,
        min: *values.iter().min().expect("nonempty values"),
        max: *values.iter().max().expect("nonempty values"),
    }
}

#[derive(Debug)]
pub struct SeedRangeError(String);

impl fmt::Display for SeedRangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for SeedRangeError {}

pub fn parse_seed_range(input: &str) -> Result<Vec<u64>, SeedRangeError> {
    let (start, end, inclusive) = if let Some((start, end)) = input.split_once("..=") {
        (start, end, true)
    } else if let Some((start, end)) = input.split_once("..") {
        (start, end, false)
    } else {
        return Err(SeedRangeError(
            "seed range must use START..END or START..=END".into(),
        ));
    };
    let start: u64 = start
        .parse()
        .map_err(|_| SeedRangeError("invalid seed range start".into()))?;
    let end: u64 = end
        .parse()
        .map_err(|_| SeedRangeError("invalid seed range end".into()))?;
    let exclusive_end = if inclusive {
        end.checked_add(1)
            .ok_or_else(|| SeedRangeError("inclusive seed range end is too large".into()))?
    } else {
        end
    };
    if start >= exclusive_end {
        return Err(SeedRangeError(
            "seed range must be nonempty and ascending".into(),
        ));
    }
    let count = exclusive_end - start;
    if count > MAX_REPLICATES {
        return Err(SeedRangeError(format!(
            "seed range contains {count} replicates; maximum is {MAX_REPLICATES}"
        )));
    }
    usize::try_from(count).map_err(|_| SeedRangeError("seed range is too large".into()))?;
    Ok((start..exclusive_end).collect())
}

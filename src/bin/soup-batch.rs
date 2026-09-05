use soup::{
    batch::{
        effective_templates, parse_seed_range, run_pending, run_replicate_with_templates,
        BatchInputChanged, BatchReport, CounterfactualSettings, ExperimentConfig, ReplicateResult,
    },
    config::Config,
};
use std::{
    collections::BTreeSet,
    error::Error,
    fs::{self, OpenOptions},
    panic::{catch_unwind, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

type BoxError = Box<dyn Error + Send + Sync>;

#[derive(Debug)]
struct Arguments {
    seeds: Vec<u64>,
    ticks: u64,
    output: PathBuf,
    config: PathBuf,
    templates_dir: Option<PathBuf>,
    counterfactual: CounterfactualSettings,
}

struct OutputLock {
    path: PathBuf,
    _file: fs::File,
}

impl OutputLock {
    fn acquire(output: &Path) -> Result<Self, BoxError> {
        let parent = output.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let name = output
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("output path must have a UTF-8 file name")?;
        let path = parent.join(format!(".{name}.lock"));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                format!(
                    "cannot lock output {} (another writer may be active): {error}",
                    output.display()
                )
            })?;
        Ok(Self { path, _file: file })
    }
}

impl Drop for OutputLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("soup-batch: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), BoxError> {
    let arguments = parse_arguments(std::env::args().skip(1).collect())?;
    let contents = fs::read_to_string(&arguments.config)
        .map_err(|error| format!("cannot read config {}: {error}", arguments.config.display()))?;
    let mut simulation_config: Config = toml::from_str(&contents)
        .map_err(|error| format!("invalid TOML in {}: {error}", arguments.config.display()))?;
    if let Some(templates_dir) = arguments.templates_dir {
        simulation_config.templates_dir = templates_dir;
    }
    validate_config(&simulation_config)?;
    let _output_lock = OutputLock::acquire(&arguments.output)?;

    let mut requested = BatchReport::new(
        env!("SOUP_GIT_COMMIT").into(),
        ExperimentConfig {
            seeds: arguments.seeds,
            ticks: arguments.ticks,
            counterfactual: arguments.counterfactual.clone(),
        },
        simulation_config.clone(),
    );
    requested.source_fingerprint = env!("SOUP_SOURCE_FINGERPRINT").into();
    let mut report = BatchReport::resume_or_new(&arguments.output, requested)?;
    let expected_templates = report.effective_templates.clone();
    let running = Arc::new(AtomicBool::new(true));
    let signal_flag = Arc::clone(&running);
    ctrlc::set_handler(move || signal_flag.store(false, Ordering::Release))?;

    let result = run_pending(&mut report, &arguments.output, |seed| {
        if effective_templates(&simulation_config) != expected_templates {
            return Err(Box::new(BatchInputChanged(
                "effective templates changed while the batch was running".into(),
            )));
        }
        let running_for_ticks = Arc::clone(&running);
        match catch_unwind(AssertUnwindSafe(|| {
            run_replicate_with_templates(
                &simulation_config,
                &expected_templates,
                seed,
                arguments.ticks,
                &arguments.counterfactual,
                || running_for_ticks.load(Ordering::Acquire),
            )
        })) {
            Ok(result) => result,
            Err(payload) => Ok(ReplicateResult::failed(seed, panic_message(payload))),
        }
    });
    if let Err(error) = result {
        report.refresh_aggregate();
        report.write_atomic(&arguments.output)?;
        return Err(error);
    }
    if report.aggregate.completed == 0 && report.aggregate.failed > 0 {
        return Err("all batch replicates failed".into());
    }
    Ok(())
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        format!("replicate panicked: {message}")
    } else if let Some(message) = payload.downcast_ref::<String>() {
        format!("replicate panicked: {message}")
    } else {
        "replicate panicked".into()
    }
}

fn parse_arguments(arguments: Vec<String>) -> Result<Arguments, BoxError> {
    let mut seed_range = None;
    let mut seed_file = None;
    let mut seed_option_seen = false;
    let mut ticks = None;
    let mut output = None;
    let mut config = PathBuf::from("soup.toml");
    let mut templates_dir = None;
    let mut counterfactual_enabled = false;
    let mut counterfactual_horizon = 100_000;
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        let value = |index: &mut usize| -> Result<&str, BoxError> {
            *index += 1;
            arguments
                .get(*index)
                .map(String::as_str)
                .ok_or_else(|| format!("{argument} requires a value").into())
        };
        match argument.as_str() {
            "--seeds" => {
                if seed_option_seen {
                    return Err(
                        "provide exactly one of --seeds or --seed-file (duplicate seed option)"
                            .into(),
                    );
                }
                seed_option_seen = true;
                seed_range = Some(value(&mut index)?.to_owned());
            }
            "--seed-file" => {
                if seed_option_seen {
                    return Err(
                        "provide exactly one of --seeds or --seed-file (duplicate seed option)"
                            .into(),
                    );
                }
                seed_option_seen = true;
                seed_file = Some(PathBuf::from(value(&mut index)?));
            }
            "--ticks" => ticks = Some(parse_positive(value(&mut index)?, "--ticks")?),
            "--output" => output = Some(PathBuf::from(value(&mut index)?)),
            "--config" => config = PathBuf::from(value(&mut index)?),
            "--templates-dir" => templates_dir = Some(PathBuf::from(value(&mut index)?)),
            "--counterfactual" => counterfactual_enabled = true,
            "--counterfactual-horizon" => {
                counterfactual_horizon =
                    parse_positive(value(&mut index)?, "--counterfactual-horizon")?;
            }
            "--help" | "-h" => {
                eprintln!(
                    "Usage: soup-batch (--seeds START..=END | --seed-file PATH) \
                     --ticks N --output PATH [--config PATH] [--templates-dir PATH] \
                     [--counterfactual] [--counterfactual-horizon N]"
                );
                std::process::exit(0);
            }
            unknown => return Err(format!("unknown argument: {unknown}").into()),
        }
        index += 1;
    }
    if seed_range.is_some() == seed_file.is_some() {
        return Err("provide exactly one of --seeds or --seed-file".into());
    }
    let seeds = if let Some(range) = seed_range {
        parse_seed_range(&range)?
    } else {
        parse_seed_file(&seed_file.expect("checked seed file"))?
    };
    Ok(Arguments {
        seeds,
        ticks: ticks.ok_or("--ticks is required")?,
        output: output.ok_or("--output is required")?,
        config,
        templates_dir,
        counterfactual: CounterfactualSettings {
            enabled: counterfactual_enabled,
            horizon: if counterfactual_enabled {
                counterfactual_horizon
            } else {
                0
            },
        },
    })
}

fn parse_positive(value: &str, name: &str) -> Result<u64, BoxError> {
    let parsed: u64 = value
        .parse()
        .map_err(|_| format!("{name} requires a positive integer"))?;
    if parsed == 0 {
        return Err(format!("{name} must be greater than zero").into());
    }
    Ok(parsed)
}

fn parse_seed_file(path: &PathBuf) -> Result<Vec<u64>, BoxError> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("cannot read seed file {}: {error}", path.display()))?;
    let mut seeds = BTreeSet::new();
    for (index, raw_line) in contents.lines().enumerate() {
        let line = raw_line.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        let seed = line.parse::<u64>().map_err(|_| {
            format!(
                "invalid seed on line {} of {}: {line}",
                index + 1,
                path.display()
            )
        })?;
        if !seeds.insert(seed) {
            return Err(format!(
                "invalid TOML/duplicate seed: seed file {} contains duplicate seed {seed}",
                path.display()
            )
            .into());
        }
        if seeds.len() as u64 > soup::batch::MAX_REPLICATES {
            return Err(format!(
                "seed file {} contains more than {} seeds",
                path.display(),
                soup::batch::MAX_REPLICATES
            )
            .into());
        }
    }
    if seeds.is_empty() {
        return Err(format!("seed file {} contains no seeds", path.display()).into());
    }
    Ok(seeds.into_iter().collect())
}

fn validate_config(config: &Config) -> Result<(), BoxError> {
    if config.memory_size != 65_536 {
        return Err("memory_size must be 65536".into());
    }
    for (name, rate) in [
        ("mutation_rate", config.mutation_rate),
        ("strategy_mutation_rate", config.strategy_mutation_rate),
        ("insertion_rate", config.insertion_rate),
        ("deletion_rate", config.deletion_rate),
        ("duplication_rate", config.duplication_rate),
        ("child_locality_bias", config.child_locality_bias),
        ("tag_mutation_rate", config.tag_mutation_rate),
    ] {
        if !(0.0..=1.0).contains(&rate) {
            return Err(format!("{name} must be finite and between 0 and 1").into());
        }
    }
    if config.max_genome_length == 0 {
        return Err("max_genome_length must be greater than zero".into());
    }
    Ok(())
}

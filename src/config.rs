use crate::events::ResourceKind;
use crate::mutation::MutationStrategy;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ResourceSource {
    /// Resource-map address relative to the seed-derived environment origin.
    pub offset: u16,
    pub kind: ResourceKind,
    /// Ticks between emissions. Zero disables this source.
    pub interval: u64,
    /// Energy requested from the conserved ambient pool per emission.
    pub amount: u32,
    /// Number of consecutive cells receiving each emission.
    pub width: usize,
    /// Cells moved after each emission; negative values move backward.
    pub velocity: i16,
}

impl Default for ResourceSource {
    fn default() -> Self {
        Self {
            offset: 0,
            kind: ResourceKind::A,
            interval: 10,
            amount: 125,
            width: 8,
            velocity: 0,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub memory_size: usize,
    pub initial_energy: u32,
    pub mutation_rate: f64,
    /// Ancestor default chance per birth that one mutation-strategy locus changes.
    pub strategy_mutation_rate: f64,
    /// Chance per birth of inserting a random instruction.
    pub insertion_rate: f64,
    /// Chance per birth of deleting a short instruction span.
    pub deletion_rate: f64,
    /// Chance per birth of duplicating a short instruction span.
    pub duplication_rate: f64,
    /// Largest genome admitted after structural mutation.
    pub max_genome_length: u16,
    /// Chance that ALLOC chooses free memory nearest the parent.
    pub child_locality_bias: f64,
    /// Chance per birth that the inherited recognition tag changes.
    pub tag_mutation_rate: f64,
    /// Inclusive maximum circular distance scanned by resource and organism seeks.
    /// Values above half the 65,536-cell ring are equivalent to 32,768.
    pub interaction_radius: u16,
    pub alloc_cost: u32,
    pub commit_cost: u32,
    /// Maximum instructions executed by one organism before senescence. Zero disables it.
    pub max_program_age: u64,
    /// Maximum A or B units moved by one TAKE/EXCRETE instruction.
    pub max_resource_flux_per_instruction: u32,
    /// Maximum A units, B units, or A+B pairs processed by one metabolic instruction.
    pub max_metabolism_per_instruction: u32,
    pub loop_max_depth: usize,
    pub ticks_per_stat_log: u64,
    /// Minimum accumulated observation time before reporting an ecotype.
    pub ecotype_min_persistence_ticks: u64,
    /// Minimum births by behaviorally equivalent organisms.
    pub ecotype_min_reproductive_output: u64,
    /// Stable parent-to-descendant links required (2 requires a grandchild).
    pub ecotype_min_descendant_generations: u32,
    /// Number of paired counterfactual replicates. Values below two are allowed
    /// for diagnostics but can only produce an Inconclusive verdict.
    pub counterfactual_replicates: usize,
    pub rng_seed: u64,
    /// Amount subtracted from each energy deposit per decay event. Default: 1.
    pub energy_decay_rate: u32,
    /// How many ticks between decay sweeps of the energy map. Default: 100.
    pub energy_decay_interval: u64,
    /// Cells that deposited energy travels forward on each decay sweep.
    /// A current keeps finite energy moving through the circular world.
    pub energy_current: usize,
    /// Total energy in the system (conserved). Default: 1_000_000.
    pub total_energy: u64,
    /// Energy transferred from parent to child on COMMIT. Default: 500.
    pub child_energy: u32,
    /// Organism-independent fixed or moving resource emitters.
    pub resource_sources: Vec<ResourceSource>,
    /// Emit ForeignExec events when a program's IP is in another program's region. Default: true.
    pub foreign_exec_tracking: bool,
    /// Emit ForeignWrite events when a program writes to another program's region. Default: true.
    pub foreign_write_tracking: bool,
    #[serde(skip)]
    pub log_path: PathBuf,
    /// Directory containing `*.toml` template files. Default: "templates".
    /// Falls back to hardcoded SEED if missing or empty.
    #[serde(skip)]
    pub templates_dir: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            memory_size: 65536,
            initial_energy: 5_000,
            mutation_rate: 0.005,
            strategy_mutation_rate: 0.01,
            insertion_rate: 0.004,
            deletion_rate: 0.004,
            duplication_rate: 0.004,
            max_genome_length: 512,
            child_locality_bias: 0.92,
            tag_mutation_rate: 0.01,
            interaction_radius: 256,
            alloc_cost: 10,
            commit_cost: 20,
            max_program_age: 20_000,
            max_resource_flux_per_instruction: 256,
            max_metabolism_per_instruction: 256,
            loop_max_depth: 8,
            ticks_per_stat_log: 10_000,
            ecotype_min_persistence_ticks: 10_000,
            ecotype_min_reproductive_output: 2,
            ecotype_min_descendant_generations: 2,
            counterfactual_replicates: 8,
            rng_seed: 42,
            energy_decay_rate: 1,
            energy_decay_interval: 100,
            energy_current: 17,
            total_energy: 1_000_000,
            child_energy: 500,
            resource_sources: vec![
                ResourceSource {
                    offset: 0,
                    kind: ResourceKind::A,
                    amount: 200,
                    ..ResourceSource::default()
                },
                ResourceSource {
                    offset: 128,
                    kind: ResourceKind::B,
                    amount: 50,
                    velocity: -1,
                    ..ResourceSource::default()
                },
                ResourceSource {
                    offset: 16_384,
                    kind: ResourceKind::B,
                    ..ResourceSource::default()
                },
                ResourceSource {
                    offset: 32_768,
                    kind: ResourceKind::A,
                    velocity: 1,
                    ..ResourceSource::default()
                },
            ],
            foreign_exec_tracking: true,
            foreign_write_tracking: true,
            log_path: PathBuf::from("soup.log"),
            templates_dir: PathBuf::from("templates"),
        }
    }
}

impl Config {
    /// Initial extra-genomic strategy assigned to startup ancestors.
    pub fn ancestor_mutation_strategy(&self) -> MutationStrategy {
        MutationStrategy::new(
            MutationStrategy::rate_from_probability(self.mutation_rate),
            MutationStrategy::rate_from_probability(self.insertion_rate),
            MutationStrategy::rate_from_probability(self.deletion_rate),
            MutationStrategy::rate_from_probability(self.duplication_rate),
            MutationStrategy::rate_from_probability(self.strategy_mutation_rate),
        )
    }

    /// Load config: start with defaults, optionally overlay a TOML file,
    /// then override individual fields with env vars.
    pub fn from_env() -> Self {
        let mut c = Self::default();

        // Try loading TOML file
        let config_path = std::env::var("SOUP_CONFIG").unwrap_or_else(|_| "soup.toml".to_string());
        if let Ok(contents) = std::fs::read_to_string(&config_path) {
            if let Ok(file_cfg) = toml::from_str::<Config>(&contents) {
                // Overlay file values onto defaults
                c.initial_energy = file_cfg.initial_energy;
                c.mutation_rate = file_cfg.mutation_rate;
                c.strategy_mutation_rate = file_cfg.strategy_mutation_rate;
                c.insertion_rate = file_cfg.insertion_rate;
                c.deletion_rate = file_cfg.deletion_rate;
                c.duplication_rate = file_cfg.duplication_rate;
                c.max_genome_length = file_cfg.max_genome_length;
                c.child_locality_bias = file_cfg.child_locality_bias;
                c.tag_mutation_rate = file_cfg.tag_mutation_rate;
                c.interaction_radius = file_cfg.interaction_radius;
                c.alloc_cost = file_cfg.alloc_cost;
                c.commit_cost = file_cfg.commit_cost;
                c.max_program_age = file_cfg.max_program_age;
                c.max_resource_flux_per_instruction = file_cfg.max_resource_flux_per_instruction;
                c.max_metabolism_per_instruction = file_cfg.max_metabolism_per_instruction;
                c.loop_max_depth = file_cfg.loop_max_depth;
                c.ticks_per_stat_log = file_cfg.ticks_per_stat_log;
                c.ecotype_min_persistence_ticks = file_cfg.ecotype_min_persistence_ticks;
                c.ecotype_min_reproductive_output = file_cfg.ecotype_min_reproductive_output;
                c.ecotype_min_descendant_generations = file_cfg.ecotype_min_descendant_generations;
                c.counterfactual_replicates = file_cfg.counterfactual_replicates;
                c.rng_seed = file_cfg.rng_seed;
                c.energy_decay_rate = file_cfg.energy_decay_rate;
                c.energy_decay_interval = file_cfg.energy_decay_interval;
                c.energy_current = file_cfg.energy_current;
                c.total_energy = file_cfg.total_energy;
                c.child_energy = file_cfg.child_energy;
                c.resource_sources = file_cfg.resource_sources;
                c.foreign_exec_tracking = file_cfg.foreign_exec_tracking;
                c.foreign_write_tracking = file_cfg.foreign_write_tracking;
            }
        }

        // Env vars take highest priority
        macro_rules! parse_env {
            ($field:ident, $key:literal) => {
                if let Ok(v) = std::env::var($key) {
                    if let Ok(n) = v.parse() {
                        c.$field = n;
                    }
                }
            };
        }
        parse_env!(initial_energy, "INITIAL_ENERGY");
        parse_env!(mutation_rate, "MUTATION_RATE");
        parse_env!(strategy_mutation_rate, "STRATEGY_MUTATION_RATE");
        parse_env!(insertion_rate, "INSERTION_RATE");
        parse_env!(deletion_rate, "DELETION_RATE");
        parse_env!(duplication_rate, "DUPLICATION_RATE");
        parse_env!(max_genome_length, "MAX_GENOME_LENGTH");
        parse_env!(child_locality_bias, "CHILD_LOCALITY_BIAS");
        parse_env!(tag_mutation_rate, "TAG_MUTATION_RATE");
        parse_env!(interaction_radius, "INTERACTION_RADIUS");
        parse_env!(alloc_cost, "ALLOC_COST");
        parse_env!(commit_cost, "COMMIT_COST");
        parse_env!(max_program_age, "MAX_PROGRAM_AGE");
        parse_env!(
            max_resource_flux_per_instruction,
            "MAX_RESOURCE_FLUX_PER_INSTRUCTION"
        );
        parse_env!(
            max_metabolism_per_instruction,
            "MAX_METABOLISM_PER_INSTRUCTION"
        );
        parse_env!(loop_max_depth, "LOOP_MAX_DEPTH");
        parse_env!(ticks_per_stat_log, "TICKS_PER_STAT_LOG");
        parse_env!(
            ecotype_min_persistence_ticks,
            "ECOTYPE_MIN_PERSISTENCE_TICKS"
        );
        parse_env!(
            ecotype_min_reproductive_output,
            "ECOTYPE_MIN_REPRODUCTIVE_OUTPUT"
        );
        parse_env!(
            ecotype_min_descendant_generations,
            "ECOTYPE_MIN_DESCENDANT_GENERATIONS"
        );
        parse_env!(counterfactual_replicates, "COUNTERFACTUAL_REPLICATES");
        parse_env!(rng_seed, "RNG_SEED");
        parse_env!(energy_decay_rate, "ENERGY_DECAY_RATE");
        parse_env!(energy_decay_interval, "ENERGY_DECAY_INTERVAL");
        parse_env!(energy_current, "ENERGY_CURRENT");
        parse_env!(total_energy, "TOTAL_ENERGY");
        parse_env!(child_energy, "CHILD_ENERGY");
        parse_env!(foreign_exec_tracking, "FOREIGN_EXEC_TRACKING");
        parse_env!(foreign_write_tracking, "FOREIGN_WRITE_TRACKING");
        if let Ok(v) = std::env::var("LOG_PATH") {
            c.log_path = PathBuf::from(v);
        }
        if let Ok(v) = std::env::var("SOUP_TEMPLATES_DIR") {
            c.templates_dir = PathBuf::from(v);
        }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throughput_limits_load_from_toml_and_have_viable_defaults() {
        let parsed: Config = toml::from_str(
            "max_resource_flux_per_instruction = 17\nmax_metabolism_per_instruction = 23\n",
        )
        .unwrap();
        assert_eq!(parsed.max_resource_flux_per_instruction, 17);
        assert_eq!(parsed.max_metabolism_per_instruction, 23);

        let defaults = Config::default();
        assert_eq!(defaults.max_resource_flux_per_instruction, 256);
        assert_eq!(defaults.max_metabolism_per_instruction, 256);
    }
}

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub memory_size: usize,
    pub initial_energy: u32,
    pub mutation_rate: f64,
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
    /// Maximum circular distance scanned by resource and tag seeks.
    pub interaction_radius: u16,
    pub alloc_cost: u32,
    pub commit_cost: u32,
    /// Maximum instructions executed by one organism before senescence. Zero disables it.
    pub max_program_age: u64,
    pub loop_max_depth: usize,
    pub ticks_per_stat_log: u64,
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
    /// How many ticks between ambient drip events. Default: 10.
    pub ambient_drip_interval: u64,
    /// Energy deposited to a random cell per drip event. Default: 500.
    pub ambient_drip_amount: u32,
    /// Width in cells of each random energy front. Default: 8.
    pub energy_rain_width: usize,
    /// Chance that a front lands near a randomly chosen live organism. Default: 0.95.
    pub energy_rain_life_bias: f64,
    /// Maximum circular offset for life-biased rain. Default: 96.
    pub energy_rain_radius: u16,
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
            loop_max_depth: 8,
            ticks_per_stat_log: 10_000,
            rng_seed: 42,
            energy_decay_rate: 1,
            energy_decay_interval: 100,
            energy_current: 17,
            total_energy: 1_000_000,
            child_energy: 500,
            ambient_drip_interval: 10,
            ambient_drip_amount: 500,
            energy_rain_width: 8,
            energy_rain_life_bias: 0.95,
            energy_rain_radius: 96,
            foreign_exec_tracking: true,
            foreign_write_tracking: true,
            log_path: PathBuf::from("soup.log"),
            templates_dir: PathBuf::from("templates"),
        }
    }
}

impl Config {
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
                c.loop_max_depth = file_cfg.loop_max_depth;
                c.ticks_per_stat_log = file_cfg.ticks_per_stat_log;
                c.rng_seed = file_cfg.rng_seed;
                c.energy_decay_rate = file_cfg.energy_decay_rate;
                c.energy_decay_interval = file_cfg.energy_decay_interval;
                c.energy_current = file_cfg.energy_current;
                c.total_energy = file_cfg.total_energy;
                c.child_energy = file_cfg.child_energy;
                c.ambient_drip_interval = file_cfg.ambient_drip_interval;
                c.ambient_drip_amount = file_cfg.ambient_drip_amount;
                c.energy_rain_width = file_cfg.energy_rain_width;
                c.energy_rain_life_bias = file_cfg.energy_rain_life_bias;
                c.energy_rain_radius = file_cfg.energy_rain_radius;
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
        parse_env!(loop_max_depth, "LOOP_MAX_DEPTH");
        parse_env!(ticks_per_stat_log, "TICKS_PER_STAT_LOG");
        parse_env!(rng_seed, "RNG_SEED");
        parse_env!(energy_decay_rate, "ENERGY_DECAY_RATE");
        parse_env!(energy_decay_interval, "ENERGY_DECAY_INTERVAL");
        parse_env!(energy_current, "ENERGY_CURRENT");
        parse_env!(total_energy, "TOTAL_ENERGY");
        parse_env!(child_energy, "CHILD_ENERGY");
        parse_env!(ambient_drip_interval, "AMBIENT_DRIP_INTERVAL");
        parse_env!(ambient_drip_amount, "AMBIENT_DRIP_AMOUNT");
        parse_env!(energy_rain_width, "ENERGY_RAIN_WIDTH");
        parse_env!(energy_rain_life_bias, "ENERGY_RAIN_LIFE_BIAS");
        parse_env!(energy_rain_radius, "ENERGY_RAIN_RADIUS");
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

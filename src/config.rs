use std::path::PathBuf;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub memory_size: usize,
    pub initial_energy: u32,
    pub mutation_rate: f64,
    pub alloc_cost: u32,
    pub commit_cost: u32,
    pub loop_max_depth: usize,
    pub ticks_per_stat_log: u64,
    pub rng_seed: u64,
    /// Amount subtracted from each energy deposit per decay event. Default: 1.
    pub energy_decay_rate: u32,
    /// How many ticks between decay sweeps of the energy map. Default: 100.
    pub energy_decay_interval: u64,
    /// Total energy in the system (conserved). Default: 1_000_000.
    pub total_energy: u64,
    /// Energy transferred from parent to child on COMMIT. Default: 500.
    pub child_energy: u32,
    /// How many ticks between ambient drip events. Default: 10.
    pub ambient_drip_interval: u64,
    /// Energy deposited to a random cell per drip event. Default: 500.
    pub ambient_drip_amount: u32,
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
            alloc_cost: 10,
            commit_cost: 20,
            loop_max_depth: 8,
            ticks_per_stat_log: 10_000,
            rng_seed: 42,
            energy_decay_rate: 1,
            energy_decay_interval: 100,
            total_energy: 1_000_000,
            child_energy: 500,
            ambient_drip_interval: 10,
            ambient_drip_amount: 500,
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
        let config_path = std::env::var("SOUP_CONFIG")
            .unwrap_or_else(|_| "soup.toml".to_string());
        if let Ok(contents) = std::fs::read_to_string(&config_path) {
            if let Ok(file_cfg) = toml::from_str::<Config>(&contents) {
                // Overlay file values onto defaults
                c.initial_energy = file_cfg.initial_energy;
                c.mutation_rate = file_cfg.mutation_rate;
                c.alloc_cost = file_cfg.alloc_cost;
                c.commit_cost = file_cfg.commit_cost;
                c.loop_max_depth = file_cfg.loop_max_depth;
                c.ticks_per_stat_log = file_cfg.ticks_per_stat_log;
                c.rng_seed = file_cfg.rng_seed;
                c.energy_decay_rate = file_cfg.energy_decay_rate;
                c.energy_decay_interval = file_cfg.energy_decay_interval;
                c.total_energy = file_cfg.total_energy;
                c.child_energy = file_cfg.child_energy;
                c.ambient_drip_interval = file_cfg.ambient_drip_interval;
                c.ambient_drip_amount = file_cfg.ambient_drip_amount;
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
        parse_env!(initial_energy,     "INITIAL_ENERGY");
        parse_env!(mutation_rate,      "MUTATION_RATE");
        parse_env!(alloc_cost,         "ALLOC_COST");
        parse_env!(commit_cost,        "COMMIT_COST");
        parse_env!(loop_max_depth,     "LOOP_MAX_DEPTH");
        parse_env!(ticks_per_stat_log, "TICKS_PER_STAT_LOG");
        parse_env!(rng_seed,              "RNG_SEED");
        parse_env!(energy_decay_rate,     "ENERGY_DECAY_RATE");
        parse_env!(energy_decay_interval, "ENERGY_DECAY_INTERVAL");
        parse_env!(total_energy,          "TOTAL_ENERGY");
        parse_env!(child_energy,          "CHILD_ENERGY");
        parse_env!(ambient_drip_interval, "AMBIENT_DRIP_INTERVAL");
        parse_env!(ambient_drip_amount,   "AMBIENT_DRIP_AMOUNT");
        if let Ok(v) = std::env::var("LOG_PATH") {
            c.log_path = PathBuf::from(v);
        }
        if let Ok(v) = std::env::var("SOUP_TEMPLATES_DIR") {
            c.templates_dir = PathBuf::from(v);
        }
        c
    }
}

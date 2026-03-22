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
    #[serde(skip)]
    pub log_path: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            memory_size: 65536,
            initial_energy: 1_000,
            mutation_rate: 0.005,
            alloc_cost: 10,
            commit_cost: 20,
            loop_max_depth: 8,
            ticks_per_stat_log: 10_000,
            rng_seed: 42,
            log_path: PathBuf::from("soup.log"),
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
        parse_env!(rng_seed,           "RNG_SEED");
        if let Ok(v) = std::env::var("LOG_PATH") {
            c.log_path = PathBuf::from(v);
        }
        c
    }
}

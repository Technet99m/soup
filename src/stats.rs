use crate::world::World;

#[derive(Debug, Clone)]
pub struct StatsSnapshot {
    pub tick: u64,
    pub live_programs: usize,
    pub memory_utilization: f64,
    /// Number of distinct live byte sequences.
    pub live_genomes: usize,
    pub max_generation: u32,
    pub max_ancestor_distance: usize,
    pub total_births: u64,
    pub total_mutations: u64,
    /// Counts of each byte value across all occupied memory regions.
    pub instruction_histogram: Box<[u64; 256]>,
    pub oldest_age: u64,
    pub total_free_blocks: usize,
    /// Energy currently held in the ambient pool.
    pub ambient_pool: u64,
    /// Total resource A currently deposited across the world.
    pub energy_map_total: u64,
    /// Total resource B deposited across the world.
    pub resource_b_total: u64,
}

impl StatsSnapshot {
    pub fn compute(world: &World) -> Self {
        let live_programs = world.programs.len();

        let memory_utilization = world.memory_utilization();

        let live_genomes = world.live_genomes();
        let max_ancestor_distance = world
            .programs
            .values()
            .map(|program| world.ancestor_distance(program))
            .max()
            .unwrap_or(0);

        // Oldest living program by age
        let oldest_age = world.programs.values().map(|p| p.age).max().unwrap_or(0);

        // Instruction histogram: count byte values across all occupied regions
        let mut histogram = Box::new([0u64; 256]);
        for p in world.programs.values() {
            for i in 0..p.length {
                let byte = world.memory.read(p.start.wrapping_add(i));
                histogram[byte as usize] += 1;
            }
        }

        let energy_map_total: u64 = world.memory.energy_map.iter().map(|&v| v as u64).sum();
        let resource_b_total: u64 = world
            .memory
            .resource_b_map
            .iter()
            .map(|&value| value as u64)
            .sum();

        StatsSnapshot {
            tick: world.tick,
            live_programs,
            memory_utilization,
            live_genomes,
            max_generation: world.max_generation,
            max_ancestor_distance,
            total_births: world.total_births,
            total_mutations: world.total_mutations,
            instruction_histogram: histogram,
            oldest_age,
            total_free_blocks: world.free_list.num_blocks(),
            ambient_pool: world.ambient_pool,
            energy_map_total,
            resource_b_total,
        }
    }

    /// Print a human-readable summary to stdout.
    pub fn print(&self) {
        println!(
            "tick={:>12}  live={:>5}  genomes={:>4}  gen={:>4}  drift={:>3}  births={:>7}  mutations={:>6}  mem={:>5.1}%  ambient={:>10}  A={:>9}  B={:>9}",
            self.tick,
            self.live_programs,
            self.live_genomes,
            self.max_generation,
            self.max_ancestor_distance,
            self.total_births,
            self.total_mutations,
            self.memory_utilization * 100.0,
            self.ambient_pool,
            self.energy_map_total,
            self.resource_b_total,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::world::World;

    #[test]
    fn snapshot_from_fresh_world() {
        use std::path::PathBuf;
        let cfg = Config {
            templates_dir: PathBuf::from("/nonexistent_soup_test_templates"),
            ..Config::default()
        };
        let world = World::new(cfg);
        let snap = StatsSnapshot::compute(&world);

        assert_eq!(snap.live_programs, 1);
        assert_eq!(snap.tick, 0);
        assert!(snap.memory_utilization > 0.0);
        assert!(snap.oldest_age == 0);
        // Histogram total should equal the single ancestor's measured length.
        let total: u64 = snap.instruction_histogram.iter().sum();
        assert_eq!(total, crate::seed::SEED_LEN as u64);
    }
}

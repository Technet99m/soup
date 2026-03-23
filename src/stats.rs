use std::collections::HashSet;
use uuid::Uuid;
use crate::world::World;

#[derive(Debug, Clone)]
pub struct StatsSnapshot {
    pub tick: u64,
    pub live_programs: usize,
    pub memory_utilization: f64,
    pub unique_lineages: usize,
    /// Counts of each byte value across all occupied memory regions.
    pub instruction_histogram: Box<[u64; 256]>,
    pub oldest_age: u64,
    pub total_free_blocks: usize,
    /// Energy currently held in the ambient pool.
    pub ambient_pool: u64,
    /// Total energy currently deposited across all energy map cells.
    pub energy_map_total: u64,
}

impl StatsSnapshot {
    pub fn compute(world: &World) -> Self {
        let live_programs = world.programs.len();

        let memory_utilization = world.memory_utilization();

        // Collect unique lineage IDs
        let unique_lineages = world
            .programs
            .values()
            .map(|p| p.lineage_id)
            .collect::<HashSet<Uuid>>()
            .len();

        // Oldest living program by age
        let oldest_age = world
            .programs
            .values()
            .map(|p| p.age)
            .max()
            .unwrap_or(0);

        // Instruction histogram: count byte values across all occupied regions
        let mut histogram = Box::new([0u64; 256]);
        for p in world.programs.values() {
            for i in 0..p.length {
                let byte = world.memory.read(p.start.wrapping_add(i));
                histogram[byte as usize] += 1;
            }
        }

        let energy_map_total: u64 = world.memory.energy_map.iter().map(|&v| v as u64).sum();

        StatsSnapshot {
            tick: world.tick,
            live_programs,
            memory_utilization,
            unique_lineages,
            instruction_histogram: histogram,
            oldest_age,
            total_free_blocks: world.free_list.num_blocks(),
            ambient_pool: world.ambient_pool,
            energy_map_total,
        }
    }

    /// Print a human-readable summary to stdout.
    pub fn print(&self) {
        println!(
            "tick={:>12}  live={:>5}  mem={:>5.1}%  lineages={:>4}  oldest={:>8}  free_blocks={:>4}  ambient={:>10}  emap={:>10}",
            self.tick,
            self.live_programs,
            self.memory_utilization * 100.0,
            self.unique_lineages,
            self.oldest_age,
            self.total_free_blocks,
            self.ambient_pool,
            self.energy_map_total,
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
        let mut cfg = Config::default();
        cfg.templates_dir = PathBuf::from("/nonexistent_soup_test_templates");
        let world = World::new(cfg);
        let snap = StatsSnapshot::compute(&world);

        assert_eq!(snap.live_programs, 1);
        assert_eq!(snap.tick, 0);
        assert!(snap.memory_utilization > 0.0);
        assert!(snap.oldest_age == 0);
        // Looper (fallback SEED) is 32 bytes; histogram total should equal its length
        let total: u64 = snap.instruction_histogram.iter().sum();
        assert_eq!(total, crate::seed::SEED_LEN as u64);
    }
}

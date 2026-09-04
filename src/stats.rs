use crate::world::World;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagStats {
    pub tag: u8,
    pub population: usize,
    pub births: u64,
}

pub fn tag_stats(world: &World) -> Vec<TagStats> {
    let mut tags = std::collections::BTreeMap::<u8, TagStats>::new();
    for program in world.programs.values() {
        let entry = tags.entry(program.tag).or_insert(TagStats {
            tag: program.tag,
            population: 0,
            births: 0,
        });
        entry.population += 1;
    }
    for (heritable_identity, births) in &world.births_by_parent_heritable_identity {
        let entry = tags.entry(heritable_identity.tag).or_insert(TagStats {
            tag: heritable_identity.tag,
            population: 0,
            births: 0,
        });
        entry.births += births;
    }
    let mut tags: Vec<_> = tags.into_values().collect();
    tags.sort_by_key(|tag| (std::cmp::Reverse(tag.population), tag.tag));
    tags
}

#[derive(Debug, Clone)]
pub struct StatsSnapshot {
    pub tick: u64,
    pub live_programs: usize,
    pub memory_utilization: f64,
    /// Number of distinct live byte sequences.
    pub live_genomes: usize,
    /// Number of distinct live byte-and-tag evolutionary identities.
    pub live_heritable_identities: usize,
    /// Persistent behavior/recognition classes with stable descendants.
    pub viable_ecotypes: usize,
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
    /// Live frequency and cumulative reproductive output grouped by recognition tag.
    pub tags: Vec<TagStats>,
}

impl StatsSnapshot {
    /// Column headings for the headless statistics rows.
    pub fn headless_header() -> String {
        format!(
            "{:>12}  {:>5}  {:>11}  {:>20}  {:>16}  {:>4}  {:>5}  {:>7}  {:>9}  {:>6}  {:>10}  {:>9}  {:>9}  {}",
            "tick",
            "live",
            "raw_genomes",
            "heritable_identities",
            "viable_ecotypes",
            "gen",
            "drift",
            "births",
            "mutations",
            "mem%",
            "ambient",
            "A",
            "B",
            "tags(pop/births)"
        )
    }

    pub fn compute(world: &World) -> Self {
        let live_programs = world.programs.len();

        let memory_utilization = world.memory_utilization();

        let live_genomes = world.live_genomes();
        let live_heritable_identities = world.live_heritable_identities();
        let viable_ecotypes = world.viable_ecotype_count();
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
        let tags = tag_stats(world);

        StatsSnapshot {
            tick: world.tick,
            live_programs,
            memory_utilization,
            live_genomes,
            live_heritable_identities,
            viable_ecotypes,
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
            tags,
        }
    }

    /// Format one row for the headless statistics table.
    pub fn format_headless(&self) -> String {
        let tags = self
            .tags
            .iter()
            .take(6)
            .map(|tag| format!("{:02x}:{}/{}", tag.tag, tag.population, tag.births))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{:>12}  {:>5}  {:>11}  {:>20}  {:>16}  {:>4}  {:>5}  {:>7}  {:>9}  {:>5.1}%  {:>10}  {:>9}  {:>9}  {}",
            self.tick,
            self.live_programs,
            self.live_genomes,
            self.live_heritable_identities,
            self.viable_ecotypes,
            self.max_generation,
            self.max_ancestor_distance,
            self.total_births,
            self.total_mutations,
            self.memory_utilization * 100.0,
            self.ambient_pool,
            self.energy_map_total,
            self.resource_b_total,
            tags,
        )
    }

    /// Print a human-readable summary to stdout.
    pub fn print(&self) {
        println!("{}", self.format_headless());
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

    #[test]
    fn snapshot_reports_tag_frequency_and_reproductive_output() {
        let cfg = Config {
            templates_dir: std::path::PathBuf::from("/nonexistent_soup_test_templates"),
            ..Config::default()
        };
        let mut world = World::new(cfg);
        world.programs.get_mut(&0).unwrap().tag = 12;
        world.program_tags[0] = 12;
        let heritable_identity = world.heritable_identity(&world.programs[&0]);
        let mut sibling = world.programs[&0].clone();
        sibling.id = 1;
        world.programs.insert(1, sibling);
        world
            .births_by_parent_heritable_identity
            .insert(heritable_identity, 4);
        world.births_by_parent_heritable_identity.insert(
            crate::identity::HeritableIdentity::new(heritable_identity.genome ^ 1, 12),
            2,
        );

        let snapshot = StatsSnapshot::compute(&world);

        assert_eq!(snapshot.tags.len(), 1);
        assert_eq!(snapshot.tags[0].tag, 12);
        assert_eq!(snapshot.tags[0].population, 2);
        assert_eq!(snapshot.tags[0].births, 6);
    }

    #[test]
    fn headless_header_matches_snapshot_output_columns() {
        fn token_end_columns(line: &str) -> Vec<usize> {
            let bytes = line.as_bytes();
            bytes
                .iter()
                .enumerate()
                .filter_map(|(index, byte)| {
                    (!byte.is_ascii_whitespace()
                        && bytes
                            .get(index + 1)
                            .is_none_or(|next| next.is_ascii_whitespace()))
                    .then_some(index + 1)
                })
                .collect()
        }

        let cfg = Config {
            templates_dir: std::path::PathBuf::from("/nonexistent_soup_test_templates"),
            ..Config::default()
        };
        let snapshot = StatsSnapshot::compute(&World::new(cfg));
        let header = StatsSnapshot::headless_header();
        let line = snapshot.format_headless();

        assert!(header.contains("raw_genomes"));
        assert!(header.contains("heritable_identities"));
        assert!(header.contains("viable_ecotypes"));
        assert_eq!(
            header.split_whitespace().count(),
            line.split_whitespace().count()
        );
        assert_eq!(
            &token_end_columns(&header)[..13],
            &token_end_columns(&line)[..13]
        );
    }
}

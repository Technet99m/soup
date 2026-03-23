use std::collections::{HashMap, VecDeque};
use rand::{SeedableRng, rngs::StdRng};
use crate::{
    allocator::FreeList,
    config::Config,
    events::{DeathCause, Event},
    memory::Memory,
    program::{Program, ProgramId},
    template,
    vm::{self, StepResult},
};

pub struct World {
    pub memory: Memory,
    pub free_list: FreeList,
    /// All live programs, keyed by ID.
    pub programs: HashMap<ProgramId, Program>,
    /// Round-robin run queue. Dead IDs are removed lazily.
    queue: VecDeque<ProgramId>,
    pub config: Config,
    pub tick: u64,
    next_id: ProgramId,
    rng: StdRng,
    /// Names of the startup templates, indexed by template_id.
    pub template_names: Vec<String>,
    /// The ambient energy pool — conserved total minus program energies and energy map.
    /// Burns from instruction execution return here; drip deposits from here to the
    /// energy map; deaths return remaining program energy here.
    pub ambient_pool: u64,
}

/// Build a FreeList covering all memory NOT occupied by the given placements.
/// `placements` is a slice of (start, len) pairs (need not be sorted).
/// `total` is the total address space size (65536).
fn make_free_list(total: u32, placements: &[(u16, u16)]) -> FreeList {
    let mut sorted = placements.to_vec();
    sorted.sort_by_key(|&(s, _)| s);

    let mut fl = FreeList::new(0, 0); // start empty

    let n = sorted.len();

    // Gap before first placement
    if n > 0 && sorted[0].0 > 0 {
        fl.free(0, sorted[0].0);
    }

    // Gaps between placements and after the last one
    for i in 0..n {
        let (start, len) = sorted[i];
        let occupied_end = start as u32 + len as u32;
        let next: u32 = if i + 1 < n {
            sorted[i + 1].0 as u32
        } else {
            total
        };
        if next > occupied_end {
            let gap_start = occupied_end as u16;
            let gap_len = (next - occupied_end) as u16; // safe: max gap < 65536
            fl.free(gap_start, gap_len);
        }
    }

    fl
}

impl World {
    /// Create a new World, loading templates and placing each at evenly spaced addresses.
    pub fn new(config: Config) -> Self {
        let templates = template::load_templates(&config.templates_dir);
        let num = templates.len();
        let stride = 65536u32 / num as u32;

        let mut memory = Memory::new();
        let mut placements: Vec<(u16, u16)> = Vec::with_capacity(num);

        for (i, tmpl) in templates.iter().enumerate() {
            let addr = (i as u32 * stride) as u16;
            memory.place(addr, &tmpl.bytes);
            placements.push((addr, tmpl.bytes.len() as u16));
        }

        let free_list = make_free_list(65536, &placements);

        let mut programs = HashMap::new();
        let mut queue = VecDeque::new();

        for (i, &(start, len)) in placements.iter().enumerate() {
            let prog = Program::new(
                i as ProgramId,
                start,
                len,
                config.initial_energy,
                None,
                None,
                Some(i as u8),
            );
            programs.insert(i as ProgramId, prog);
            queue.push_back(i as ProgramId);
        }

        let template_names = templates.into_iter().map(|t| t.name).collect();

        let seed_energy: u64 = programs.values().map(|p| p.energy as u64).sum();
        let ambient_pool = config.total_energy.saturating_sub(seed_energy);

        World {
            memory,
            free_list,
            programs,
            queue,
            rng: StdRng::seed_from_u64(config.rng_seed),
            config,
            tick: 0,
            next_id: num as ProgramId,
            template_names,
            ambient_pool,
        }
    }

    /// Execute one tick: run one instruction for the next program in the queue.
    /// Returns a Vec of events that occurred this tick.
    pub fn tick(&mut self) -> Vec<Event> {
        self.tick += 1;
        let mut events = Vec::new();

        // Periodic energy map decay: return decayed energy to ambient pool.
        let decay_interval = self.config.energy_decay_interval;
        if decay_interval > 0 && self.tick % decay_interval == 0 {
            let rate = self.config.energy_decay_rate;
            for cell in self.memory.energy_map.iter_mut() {
                let decay = (*cell).min(rate);
                *cell -= decay;
                self.ambient_pool += decay as u64;
            }
        }

        // Periodic ambient drip: deposit a chunk from ambient pool to a random cell.
        let drip_interval = self.config.ambient_drip_interval;
        if drip_interval > 0 && self.tick % drip_interval == 0 {
            let amount = (self.config.ambient_drip_amount as u64).min(self.ambient_pool) as u32;
            if amount > 0 {
                use rand::Rng;
                let addr = self.rng.gen::<u16>();
                self.memory.give_energy(addr, amount);
                self.ambient_pool -= amount as u64;
            }
        }

        // Pop the next program ID (skipping dead ones lazily).
        let id = loop {
            match self.queue.pop_front() {
                None => return events, // no live programs
                Some(id) if self.programs.contains_key(&id) => break id,
                Some(_) => {} // dead, skip
            }
        };

        let result = {
            let p = self.programs.get_mut(&id).unwrap();
            vm::step(p, &mut self.memory, &mut self.free_list, &self.config, &mut self.next_id, &mut self.rng, &mut events, self.tick, &mut self.ambient_pool)
        };

        match result {
            StepResult::Continue => {
                self.queue.push_back(id);
            }
            StepResult::Halted => {
                // Program executed HALT — return remaining energy to ambient, free memory.
                if let Some(p) = self.programs.remove(&id) {
                    self.ambient_pool += p.energy as u64;
                    self.free_list.free(p.start, p.length);
                    events.push(Event::Died {
                        tick: self.tick,
                        id,
                        cause: DeathCause::Killed,
                    });
                }
            }
            StepResult::OutOfEnergy => {
                // Energy is 0 (all burned to ambient via step()); nothing to return.
                if let Some(p) = self.programs.remove(&id) {
                    self.ambient_pool += p.energy as u64; // always 0, but explicit
                    self.free_list.free(p.start, p.length);
                    events.push(Event::Died {
                        tick: self.tick,
                        id,
                        cause: DeathCause::Energy,
                    });
                }
            }
            StepResult::Spawned(child) => {
                // Phase 4: handle child registration.
                // For now (should not occur in Phase 3), just re-queue parent.
                events.push(Event::Born {
                    tick: self.tick,
                    id: child.id,
                    parent_id: child.parent_id,
                    lineage_id: child.lineage_id,
                    parent_lineage_id: child.parent_lineage_id,
                    start: child.start,
                    length: child.length,
                    energy: child.energy,
                });
                events.push(Event::Committed {
                    tick: self.tick,
                    parent_id: id,
                    child_id: child.id,
                });
                let child_id = child.id;
                self.programs.insert(child_id, *child);
                self.queue.push_back(child_id);
                self.queue.push_back(id);
            }
        }

        events
    }

    /// Run for `n` ticks, collecting all events.
    pub fn run(&mut self, n: u64) -> Vec<Event> {
        let mut all = Vec::new();
        for _ in 0..n {
            all.extend(self.tick());
        }
        all
    }

    /// Number of currently live programs.
    pub fn live_count(&self) -> usize {
        self.programs.len()
    }

    /// Memory utilization as fraction 0.0..=1.0
    pub fn memory_utilization(&self) -> f64 {
        let free = self.free_list.free_bytes() as f64;
        1.0 - free / 65536.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Config that skips the templates directory so tests always fall back to the
    /// hardcoded SEED (single program, deterministic initial state).
    fn seed_config() -> Config {
        let mut cfg = Config::default();
        cfg.templates_dir = PathBuf::from("/nonexistent_soup_test_templates");
        cfg
    }

    #[test]
    fn world_initializes_with_seed() {
        let world = World::new(seed_config());
        assert_eq!(world.live_count(), 1);
        assert!(world.memory_utilization() > 0.0);
        // Seed (looper) is 32 bytes out of 65536
        let expected_util = crate::seed::SEED_LEN as f64 / 65536.0;
        assert!((world.memory_utilization() - expected_util).abs() < 0.0001);
    }

    #[test]
    fn tick_advances_tick_counter() {
        let mut world = World::new(seed_config());
        world.tick();
        assert_eq!(world.tick, 1);
        world.tick();
        assert_eq!(world.tick, 2);
    }

    #[test]
    fn seed_eventually_dies_without_replication() {
        // With very low energy the seed cannot afford ALLOC or COMMIT,
        // so it runs out of energy and dies without replicating.
        let mut cfg = seed_config();
        cfg.initial_energy = 5; // too little to reach COMMIT
        let mut world = World::new(cfg);

        // Run until dead or max ticks
        let mut died = false;
        for _ in 0..10_000 {
            let events = world.tick();
            for e in &events {
                if matches!(e, Event::Died { .. }) {
                    died = true;
                }
            }
            if world.live_count() == 0 {
                break;
            }
        }
        assert!(died, "Seed should die from energy depletion");
    }

    #[test]
    fn seed_replicates_at_least_once() {
        // With enough energy and free memory, seed should COMMIT at least one child.
        let mut cfg = seed_config();
        cfg.initial_energy = 10_000; // plenty of energy
        let mut world = World::new(cfg);

        let mut children_born = 0usize;
        for _ in 0..100_000 {
            let events = world.tick();
            for e in &events {
                if matches!(e, Event::Born { .. }) {
                    children_born += 1;
                }
            }
            if children_born > 0 {
                break;
            }
            if world.live_count() == 0 {
                break;
            }
        }
        assert!(children_born > 0, "Seed should replicate at least once");
    }

    #[test]
    fn mutation_events_are_emitted() {
        // Use mutation_rate = 1.0 to guarantee every write mutates
        let mut cfg = seed_config();
        cfg.mutation_rate = 1.0;
        cfg.initial_energy = 10_000;
        let mut world = World::new(cfg);

        let mut mutation_count = 0usize;
        for _ in 0..100_000 {
            let events = world.tick();
            for e in &events {
                if matches!(e, Event::Mutated { .. }) {
                    mutation_count += 1;
                }
            }
            if mutation_count > 0 {
                break;
            }
            if world.live_count() == 0 {
                break;
            }
        }
        assert!(mutation_count > 0, "Should observe mutations with rate=1.0");
    }

    #[test]
    fn second_generation_replicates() {
        // Verify that children can themselves produce grandchildren.
        // This validates the full replication cycle.
        let mut cfg = seed_config();
        cfg.initial_energy = 50_000;
        cfg.mutation_rate = 0.0; // no mutation — pure replication test
        let mut world = World::new(cfg);

        let mut generations: std::collections::HashSet<crate::program::ProgramId> = std::collections::HashSet::new();
        let mut grandchildren_born = 0usize;

        for _ in 0..500_000 {
            let events = world.tick();
            for e in &events {
                if let crate::events::Event::Born { id, parent_id, .. } = e {
                    if let Some(pid) = parent_id {
                        if generations.contains(pid) {
                            // Parent was itself a child (i.e., this is a grandchild)
                            grandchildren_born += 1;
                        }
                    }
                    generations.insert(*id);
                }
            }
            if grandchildren_born > 0 {
                break;
            }
            if world.live_count() == 0 {
                break;
            }
        }
        assert!(grandchildren_born > 0, "Should observe at least one grandchild");
    }

    #[test]
    fn dead_ids_cleaned_lazily() {
        // After a program dies, subsequent ticks should not panic
        let mut cfg = seed_config();
        cfg.initial_energy = 50;
        let mut world = World::new(cfg);
        // Run well past death
        for _ in 0..1000 {
            world.tick();
        }
        // Should still be alive or dead gracefully — no panics
        assert!(world.live_count() == 0 || world.live_count() >= 1);
    }
}

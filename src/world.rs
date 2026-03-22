use std::collections::{HashMap, VecDeque};
use rand::{SeedableRng, rngs::StdRng};
use crate::{
    allocator::FreeList,
    config::Config,
    events::{DeathCause, Event},
    memory::Memory,
    program::{Program, ProgramId},
    seed::{SEED, SEED_LEN},
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
}

impl World {
    /// Create a new World and place the seed program at address 0.
    pub fn new(config: Config) -> Self {
        let mut memory = Memory::new();
        memory.place(0, &SEED);

        // Free list covers everything after the seed.
        let free_start = SEED_LEN;
        let free_len = u16::MAX - SEED_LEN + 1; // 65536 - SEED_LEN
        let free_list = FreeList::new(free_start, free_len);

        let seed_program = Program::new(
            0,
            0,
            SEED_LEN,
            config.initial_energy,
            None,
            None,
        );

        let mut programs = HashMap::new();
        programs.insert(0, seed_program);

        let mut queue = VecDeque::new();
        queue.push_back(0u32);

        World {
            memory,
            free_list,
            programs,
            queue,
            rng: StdRng::seed_from_u64(config.rng_seed),
            config,
            tick: 0,
            next_id: 1,
        }
    }

    /// Execute one tick: run one instruction for the next program in the queue.
    /// Returns a Vec of events that occurred this tick.
    pub fn tick(&mut self) -> Vec<Event> {
        self.tick += 1;
        let mut events = Vec::new();

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
            vm::step(p, &mut self.memory, &mut self.free_list, &self.config, &mut self.next_id, &mut self.rng, &mut events, self.tick)
        };

        match result {
            StepResult::Continue => {
                self.queue.push_back(id);
            }
            StepResult::Halted => {
                // Program executed HALT — kill it and free memory.
                if let Some(p) = self.programs.remove(&id) {
                    self.free_list.free(p.start, p.length);
                    events.push(Event::Died {
                        tick: self.tick,
                        id,
                        cause: DeathCause::Killed,
                    });
                }
            }
            StepResult::OutOfEnergy => {
                if let Some(p) = self.programs.remove(&id) {
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

    #[test]
    fn world_initializes_with_seed() {
        let cfg = Config::default();
        let world = World::new(cfg);
        assert_eq!(world.live_count(), 1);
        assert!(world.memory_utilization() > 0.0);
        // Seed occupies 15 bytes out of 65536
        let expected_util = crate::seed::SEED_LEN as f64 / 65536.0;
        assert!((world.memory_utilization() - expected_util).abs() < 0.0001);
    }

    #[test]
    fn tick_advances_tick_counter() {
        let cfg = Config::default();
        let mut world = World::new(cfg);
        world.tick();
        assert_eq!(world.tick, 1);
        world.tick();
        assert_eq!(world.tick, 2);
    }

    #[test]
    fn seed_eventually_dies_without_replication() {
        // With very low energy the seed cannot afford ALLOC or COMMIT,
        // so it runs out of energy and dies without replicating.
        let mut cfg = Config::default();
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
        let mut cfg = Config::default();
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
        let mut cfg = Config::default();
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
        let mut cfg = Config::default();
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
        let mut cfg = Config::default();
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

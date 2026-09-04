use crate::{
    allocator::FreeList,
    config::Config,
    events::{DeathCause, Event, ResourceKind, StructuralMutationKind},
    memory::Memory,
    program::{Program, ProgramId},
    template,
    vm::{self, StepResult},
};
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::collections::{HashMap, VecDeque};

#[derive(Debug, Clone)]
pub struct SymbiosisReport {
    pub genome_a: u64,
    pub genome_b: u64,
    pub horizon: u64,
    pub baseline_births_a: u64,
    pub baseline_births_b: u64,
    pub dependence_a: f64,
    pub dependence_b: f64,
    pub verdict: RelationshipVerdict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationshipVerdict {
    Mutualism,
    ADependsOnB,
    BDependsOnA,
    Competition,
    Inconclusive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceDeposit {
    pub kind: ResourceKind,
    pub start: u16,
    pub width: usize,
    pub amount: u32,
}

#[derive(Clone)]
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
    /// Startup genomes, indexed by template_id, used as the evolutionary baseline.
    pub template_bytes: Vec<Vec<u8>>,
    /// The ambient energy pool — conserved total minus organism and deposited resources.
    /// Burns from instruction execution return here; drip deposits from here to the
    /// resource maps; deaths return remaining program energy here.
    pub ambient_pool: u64,
    /// Per-cell ownership map: addr_to_owner[addr] = Some(id) if a live program owns that byte.
    /// Kept in sync on spawn and death; used by SeekForeignStart and ForeignExec tracking.
    pub addr_to_owner: Box<[Option<ProgramId>]>,
    /// Current tag by program ID. Dead IDs remain as harmless historical entries.
    pub program_tags: Vec<u8>,
    /// Successful reproduction attributed to the parent's current genome.
    pub births_by_parent_genome: HashMap<u64, u64>,
    /// Tick of the latest successful reproduction by each genome.
    pub last_birth_by_genome: HashMap<u64, u64>,
    /// Most recently observed genome for every program ID, retained after death.
    pub genome_by_id: Vec<u64>,
    /// Cross-genome resources consumed, keyed by (donor genome, receiver genome).
    pub interactions: HashMap<(u64, u64), u64>,
    /// Executed instructions attributed to the genome present before each step.
    pub steps_by_genome: HashMap<u64, u64>,
    pub total_births: u64,
    pub total_deaths: u64,
    pub total_mutations: u64,
    pub total_foreign_execs: u64,
    pub total_foreign_writes: u64,
    pub max_generation: u32,
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
    /// Create a new World, loading templates and placing each at random addresses.
    pub fn new(config: Config) -> Self {
        let templates = template::load_templates(&config.templates_dir);
        let num = templates.len();
        let mut startup_rng = StdRng::seed_from_u64(config.rng_seed ^ 0x510a_f00d);

        let mut memory = Memory::new();
        let mut placements: Vec<(u16, u16)> = Vec::with_capacity(num);
        let mut free_ranges: Vec<(u32, u32)> = vec![(0, 65536)];

        let environment_origin = environment_origin(config.rng_seed);
        for (template_index, tmpl) in templates.iter().enumerate() {
            let len = tmpl.bytes.len() as u32;
            let preferred_start = (environment_origin as u32).min(65_536 - len);
            let fitting: Vec<usize> = free_ranges
                .iter()
                .enumerate()
                .filter_map(|(i, (start, end))| (end - start >= len).then_some(i))
                .collect();
            let chosen_idx = if template_index == 0 {
                fitting
                    .iter()
                    .copied()
                    .find(|&index| {
                        let (start, end) = free_ranges[index];
                        start <= preferred_start && preferred_start + len <= end
                    })
                    .expect("the environment origin fits the first startup template")
            } else {
                fitting[startup_rng.gen_range(0..fitting.len())]
            };
            let (range_start, range_end) = free_ranges.remove(chosen_idx);
            let start = if template_index == 0 {
                preferred_start
            } else {
                startup_rng.gen_range(range_start..=range_end - len)
            };
            let end = start + len;

            if range_start < start {
                free_ranges.push((range_start, start));
            }
            if end < range_end {
                free_ranges.push((end, range_end));
            }

            let addr = start as u16;
            let plen = len as u16;
            memory.place(addr, &tmpl.bytes);
            placements.push((addr, plen));
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

        let template_names = templates.iter().map(|t| t.name.clone()).collect();
        let template_bytes = templates.into_iter().map(|t| t.bytes).collect();

        let seed_energy: u64 = programs.values().map(|p| p.energy as u64).sum();
        let ambient_pool = config.total_energy.saturating_sub(seed_energy);

        let mut addr_to_owner: Box<[Option<ProgramId>]> = vec![None; 65536].into_boxed_slice();
        let mut program_tags = vec![0; num];
        for prog in programs.values() {
            for offset in 0..prog.length as usize {
                let addr = (prog.start as usize + offset) % 65536;
                addr_to_owner[addr] = Some(prog.id);
            }
            program_tags[prog.id as usize] = prog.tag;
        }
        let mut genome_by_id = vec![0; num];
        for program in programs.values() {
            genome_by_id[program.id as usize] = genome_hash_in_memory(&memory, program);
        }

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
            template_bytes,
            ambient_pool,
            addr_to_owner,
            program_tags,
            births_by_parent_genome: HashMap::new(),
            last_birth_by_genome: HashMap::new(),
            genome_by_id,
            interactions: HashMap::new(),
            steps_by_genome: HashMap::new(),
            total_births: 0,
            total_deaths: 0,
            total_mutations: 0,
            total_foreign_execs: 0,
            total_foreign_writes: 0,
            max_generation: 0,
        }
    }

    /// Execute one tick: run one instruction for the next program in the queue.
    /// Returns a Vec of events that occurred this tick.
    pub fn tick(&mut self) -> Vec<Event> {
        self.tick += 1;
        let mut events = Vec::new();

        // Periodic resource decay: return decayed energy to the ambient pool.
        let decay_interval = self.config.energy_decay_interval;
        if decay_interval > 0 && self.tick.is_multiple_of(decay_interval) {
            let rate = self.config.energy_decay_rate;
            for (index, cell) in self.memory.energy_map.iter_mut().enumerate() {
                let decay = (*cell).min(rate);
                *cell -= decay;
                if *cell == 0 {
                    self.memory.resource_a_donor[index] = None;
                }
                self.ambient_pool += decay as u64;
            }
            for (index, cell) in self.memory.resource_b_map.iter_mut().enumerate() {
                let decay = (*cell).min(rate);
                *cell -= decay;
                if *cell == 0 {
                    self.memory.resource_b_donor[index] = None;
                }
                self.ambient_pool += decay as u64;
            }
            let current = self.config.energy_current % self.memory.energy_map.len();
            self.memory.energy_map.rotate_right(current);
            self.memory.resource_a_donor.rotate_right(current);
            self.memory.resource_b_map.rotate_left(current);
            self.memory.resource_b_donor.rotate_left(current);
        }

        // External sources have their own deterministic schedule. They never inspect
        // live organisms or consume the VM/mutation RNG stream.
        for deposit in self.scheduled_resources(self.tick) {
            self.apply_resource_deposit(deposit);
        }

        // Pop the next program ID (skipping dead ones lazily).
        let id = loop {
            match self.queue.pop_front() {
                None => return events, // no live programs
                Some(id) if self.programs.contains_key(&id) => break id,
                Some(_) => {} // dead, skip
            }
        };

        // Emit ForeignExec if this program is about to execute code owned by another program.
        if self.config.foreign_exec_tracking {
            let ip = self.programs[&id].ip;
            if let Some(owner) = self.addr_to_owner[ip as usize] {
                if owner != id {
                    events.push(crate::events::Event::ForeignExec {
                        tick: self.tick,
                        id,
                        ip,
                        owner_id: owner,
                    });
                }
            }
        }

        let executing_hash = self
            .programs
            .get(&id)
            .map(|program| self.genome_hash(program))
            .unwrap_or(0);
        if let Some(genome) = self.genome_by_id.get_mut(id as usize) {
            *genome = executing_hash;
        }
        *self.steps_by_genome.entry(executing_hash).or_default() += 1;
        let result = {
            let p = self.programs.get_mut(&id).unwrap();
            vm::step(
                p,
                &mut self.memory,
                &mut self.free_list,
                &self.addr_to_owner,
                &self.program_tags,
                &self.config,
                &mut self.next_id,
                &mut self.rng,
                &mut events,
                self.tick,
                &mut self.ambient_pool,
            )
        };
        if let Some(program) = self.programs.get(&id) {
            if let Some(tag) = self.program_tags.get_mut(id as usize) {
                *tag = program.tag;
            }
        }

        match result {
            StepResult::Continue => {
                let senescent = self.config.max_program_age > 0
                    && self.programs[&id].age >= self.config.max_program_age;
                if senescent {
                    if let Some(p) = self.programs.remove(&id) {
                        if let Some((start, length)) = p.pending_allocation {
                            self.free_list.free(start, length);
                        }
                        for offset in 0..p.length as usize {
                            self.addr_to_owner[(p.start as usize + offset) % 65536] = None;
                        }
                        self.ambient_pool += p.energy as u64;
                        self.free_list.free(p.start, p.length);
                        events.push(Event::Died {
                            tick: self.tick,
                            id,
                            cause: DeathCause::Senescence,
                        });
                    }
                } else {
                    self.queue.push_back(id);
                }
            }
            StepResult::Halted => {
                // Program executed HALT — return remaining energy to ambient, free memory.
                if let Some(p) = self.programs.remove(&id) {
                    if let Some((start, length)) = p.pending_allocation {
                        self.free_list.free(start, length);
                    }
                    for offset in 0..p.length as usize {
                        self.addr_to_owner[(p.start as usize + offset) % 65536] = None;
                    }
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
                    if let Some((start, length)) = p.pending_allocation {
                        self.free_list.free(start, length);
                    }
                    for offset in 0..p.length as usize {
                        self.addr_to_owner[(p.start as usize + offset) % 65536] = None;
                    }
                    self.ambient_pool += p.energy as u64; // always 0, but explicit
                    self.free_list.free(p.start, p.length);
                    events.push(Event::Died {
                        tick: self.tick,
                        id,
                        cause: DeathCause::Energy,
                    });
                }
            }
            StepResult::Spawned(mut child) => {
                let parent_hash = self
                    .programs
                    .get(&id)
                    .map(|parent| self.genome_hash(parent))
                    .unwrap_or(0);
                *self.births_by_parent_genome.entry(parent_hash).or_default() += 1;
                self.last_birth_by_genome.insert(parent_hash, self.tick);
                let parent_start = self.programs[&id].start;
                self.apply_birth_mutations(&mut child, parent_start, &mut events);
                events.push(Event::Born {
                    tick: self.tick,
                    id: child.id,
                    parent_id: child.parent_id,
                    lineage_id: child.lineage_id,
                    parent_lineage_id: child.parent_lineage_id,
                    start: child.start,
                    length: child.length,
                    energy: child.energy,
                    generation: child.generation,
                });
                events.push(Event::Committed {
                    tick: self.tick,
                    parent_id: id,
                    child_id: child.id,
                });
                let child_id = child.id;
                if self.program_tags.len() <= child_id as usize {
                    self.program_tags.resize(child_id as usize + 1, 0);
                }
                self.program_tags[child_id as usize] = child.tag;
                if self.genome_by_id.len() <= child_id as usize {
                    self.genome_by_id.resize(child_id as usize + 1, 0);
                }
                self.genome_by_id[child_id as usize] = self.genome_hash(&child);
                for offset in 0..child.length as usize {
                    self.addr_to_owner[(child.start as usize + offset) % 65536] = Some(child_id);
                }
                self.programs.insert(child_id, *child);
                self.queue.push_back(child_id);
                self.queue.push_back(id);
            }
        }

        for event in &events {
            match event {
                Event::Born { generation, .. } => {
                    self.total_births += 1;
                    self.max_generation = self.max_generation.max(*generation);
                }
                Event::Died { .. } => self.total_deaths += 1,
                Event::Mutated { .. } => self.total_mutations += 1,
                Event::StructuralMutation { .. } => self.total_mutations += 1,
                Event::ForeignExec { .. } => self.total_foreign_execs += 1,
                Event::ForeignWrite { .. } => self.total_foreign_writes += 1,
                Event::ResourceTransfer {
                    donor_id,
                    receiver_id,
                    amount,
                    ..
                } => {
                    let donor = self.genome_by_id.get(*donor_id as usize).copied();
                    let receiver = self.genome_by_id.get(*receiver_id as usize).copied();
                    if let (Some(donor), Some(receiver)) = (donor, receiver) {
                        if donor != receiver {
                            *self.interactions.entry((donor, receiver)).or_default() +=
                                *amount as u64;
                        }
                    }
                }
                _ => {}
            }
        }

        events
    }

    fn apply_birth_mutations(
        &mut self,
        child: &mut Program,
        parent_start: u16,
        events: &mut Vec<Event>,
    ) {
        let roll = self.rng.gen::<f64>();
        let insert_edge = self.config.insertion_rate.max(0.0);
        let delete_edge = insert_edge + self.config.deletion_rate.max(0.0);
        let duplicate_edge = delete_edge + self.config.duplication_rate.max(0.0);
        let kind = if roll < insert_edge {
            Some(StructuralMutationKind::Insertion)
        } else if roll < delete_edge {
            Some(StructuralMutationKind::Deletion)
        } else if roll < duplicate_edge {
            Some(StructuralMutationKind::Duplication)
        } else {
            None
        };

        if let Some(kind) = kind {
            let old_length = child.length;
            let mut genome = self.memory.read_slice(child.start, child.length);
            let mut mutation_index = 0usize;
            match kind {
                StructuralMutationKind::Insertion
                    if child.length < self.config.max_genome_length =>
                {
                    mutation_index = self.rng.gen_range(0..=genome.len());
                    genome.insert(mutation_index, self.rng.gen());
                }
                StructuralMutationKind::Deletion if genome.len() > 4 => {
                    mutation_index = self.rng.gen_range(0..genome.len());
                    let max_span = 8usize
                        .min(genome.len() - mutation_index)
                        .min(genome.len() - 4);
                    let span = self.rng.gen_range(1..=max_span);
                    genome.drain(mutation_index..mutation_index + span);
                }
                StructuralMutationKind::Duplication
                    if child.length < self.config.max_genome_length =>
                {
                    let remaining = self.config.max_genome_length as usize - genome.len();
                    let max_span = 8usize.min(genome.len()).min(remaining);
                    if max_span > 0 {
                        let span = self.rng.gen_range(1..=max_span);
                        let source = self.rng.gen_range(0..=genome.len() - span);
                        let duplicate = genome[source..source + span].to_vec();
                        mutation_index = self.rng.gen_range(0..=genome.len());
                        genome.splice(mutation_index..mutation_index, duplicate);
                    }
                }
                _ => {}
            }

            let new_length = genome.len() as u16;
            if new_length != old_length && self.install_resized_genome(child, parent_start, &genome)
            {
                child.ip = child.start;
                child.rh = child.start;
                child.wh = child.start;
                child.loop_stack.clear();
                events.push(Event::StructuralMutation {
                    tick: self.tick,
                    id: child.id,
                    parent_id: child.parent_id.unwrap_or_default(),
                    kind,
                    index: mutation_index as u16,
                    old_length,
                    new_length,
                });
            }
        }

        if self.rng.gen::<f64>() < self.config.tag_mutation_rate.clamp(0.0, 1.0) {
            let old_tag = child.tag;
            let mut new_tag = self.rng.gen::<u8>();
            if new_tag == old_tag {
                new_tag = new_tag.wrapping_add(1);
            }
            child.tag = new_tag;
            events.push(Event::TagChanged {
                tick: self.tick,
                id: child.id,
                old_tag,
                new_tag,
            });
        }
    }

    fn install_resized_genome(
        &mut self,
        child: &mut Program,
        parent_start: u16,
        genome: &[u8],
    ) -> bool {
        let new_length = genome.len() as u16;
        if new_length > child.length {
            let Some(new_start) = self.free_list.alloc_near(parent_start, new_length) else {
                return false;
            };
            self.memory.place(new_start, genome);
            self.free_list.free(child.start, child.length);
            child.start = new_start;
        } else {
            self.memory.place(child.start, genome);
            self.free_list.free(
                child.start.wrapping_add(new_length),
                child.length - new_length,
            );
        }
        child.length = new_length;
        true
    }

    /// Return the complete external source schedule for a tick. This is a pure
    /// function of the seed, tick, and source configuration: organism state and
    /// the simulation RNG cannot influence it.
    pub fn scheduled_resources(&self, tick: u64) -> Vec<ResourceDeposit> {
        if tick == 0 {
            return Vec::new();
        }
        let origin = self.environment_origin();
        self.config
            .resource_sources
            .iter()
            .filter_map(|source| {
                if source.interval == 0 || !tick.is_multiple_of(source.interval) {
                    return None;
                }
                let emission = tick / source.interval - 1;
                let movement =
                    (source.velocity as i128 * emission as i128).rem_euclid(65_536) as u16;
                Some(ResourceDeposit {
                    kind: source.kind,
                    start: origin.wrapping_add(source.offset).wrapping_add(movement),
                    width: source.width.clamp(1, 65_536),
                    amount: source.amount,
                })
            })
            .collect()
    }

    pub fn environment_origin(&self) -> u16 {
        environment_origin(self.config.rng_seed)
    }

    fn apply_resource_deposit(&mut self, deposit: ResourceDeposit) {
        let requested = (deposit.amount as u64).min(self.ambient_pool) as u32;
        let base = requested / deposit.width as u32;
        let remainder = requested % deposit.width as u32;
        let mut deposited = 0u64;
        for offset in 0..deposit.width {
            let share = base + u32::from((offset as u32) < remainder);
            if share == 0 {
                continue;
            }
            let index = deposit.start.wrapping_add(offset as u16) as usize;
            let available = match deposit.kind {
                ResourceKind::A => u32::MAX - self.memory.energy_map[index],
                ResourceKind::B => u32::MAX - self.memory.resource_b_map[index],
            };
            let actual = share.min(available);
            if actual == 0 {
                continue;
            }
            match deposit.kind {
                ResourceKind::A => self.memory.give_energy(index as u16, actual),
                ResourceKind::B => self.memory.give_resource_b(index as u16, actual),
            }
            deposited += actual as u64;
        }
        self.ambient_pool -= deposited;
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

    /// Stable fingerprint of an organism's current bytes. Equal hashes are exact
    /// genotypes for the purposes of the live observer.
    pub fn genome_hash(&self, program: &Program) -> u64 {
        genome_hash_in_memory(&self.memory, program)
    }

    pub fn live_genomes(&self) -> usize {
        self.programs
            .values()
            .map(|program| self.genome_hash(program))
            .collect::<std::collections::HashSet<_>>()
            .len()
    }

    /// Byte distance from the startup genome that founded this organism's clade.
    pub fn ancestor_distance(&self, program: &Program) -> usize {
        let Some(template_id) = program.template_id else {
            return 0;
        };
        let Some(ancestor) = self.template_bytes.get(template_id as usize) else {
            return 0;
        };
        let genome = self.memory.read_slice(program.start, program.length);
        let substitutions = genome.iter().zip(ancestor).filter(|(a, b)| a != b).count();
        substitutions + genome.len().abs_diff(ancestor.len())
    }

    /// Pick the strongest live candidate pair, preferring abundant genomes with
    /// opposite A/B harvesting profiles. This is only hypothesis generation;
    /// `counterfactual_symbiosis` performs the actual removal experiment.
    pub fn candidate_partner_pair(&self) -> Option<(u64, u64)> {
        let live_hashes: std::collections::HashSet<_> = self
            .programs
            .values()
            .map(|program| self.genome_hash(program))
            .collect();
        let mut transferred_by_pair: HashMap<(u64, u64), u64> = HashMap::new();
        for (&(donor, receiver), &amount) in &self.interactions {
            let active = |hash: u64| {
                self.births_by_parent_genome
                    .get(&hash)
                    .is_some_and(|births| *births >= 2)
                    && self
                        .last_birth_by_genome
                        .get(&hash)
                        .is_some_and(|tick| self.tick.saturating_sub(*tick) <= 100_000)
            };
            if donor != receiver
                && live_hashes.contains(&donor)
                && live_hashes.contains(&receiver)
                && active(donor)
                && active(receiver)
            {
                let pair = if donor < receiver {
                    (donor, receiver)
                } else {
                    (receiver, donor)
                };
                *transferred_by_pair.entry(pair).or_default() += amount;
            }
        }
        if let Some((pair, _)) = transferred_by_pair
            .into_iter()
            .max_by_key(|(_, amount)| *amount)
        {
            return Some(pair);
        }

        #[derive(Default)]
        struct Phenotype {
            population: u64,
            a: u64,
            b: u64,
            births: u64,
        }
        let mut phenotypes: HashMap<u64, Phenotype> = HashMap::new();
        for program in self.programs.values() {
            let hash = self.genome_hash(program);
            let phenotype = phenotypes.entry(hash).or_default();
            phenotype.population += 1;
            phenotype.a += program.trace.opcode_counts[31];
            phenotype.b += program.trace.opcode_counts[37];
            phenotype.births = self
                .births_by_parent_genome
                .get(&hash)
                .copied()
                .unwrap_or(0);
        }
        let mut live: Vec<_> = phenotypes.into_iter().collect();
        let has_active_pair = live
            .iter()
            .filter(|(hash, phenotype)| {
                phenotype.births >= 2
                    && self
                        .last_birth_by_genome
                        .get(hash)
                        .is_some_and(|tick| self.tick.saturating_sub(*tick) <= 100_000)
            })
            .count()
            >= 2;
        if has_active_pair {
            live.retain(|(hash, phenotype)| {
                phenotype.births >= 2
                    && self
                        .last_birth_by_genome
                        .get(hash)
                        .is_some_and(|tick| self.tick.saturating_sub(*tick) <= 100_000)
            });
        }
        live.sort_by_key(|(_, phenotype)| std::cmp::Reverse(phenotype.population));
        live.truncate(12);

        let mut best = None;
        for left in 0..live.len() {
            for right in left + 1..live.len() {
                let (hash_a, a) = &live[left];
                let (hash_b, b) = &live[right];
                let preference_a = a.a as f64 / (a.a + a.b).max(1) as f64;
                let preference_b = b.a as f64 / (b.a + b.b).max(1) as f64;
                let complement = (preference_a - preference_b).abs();
                let abundance = a.population.min(b.population) as f64;
                let reproductive_evidence = a.births.min(b.births) as f64;
                let score = abundance * 1_000_000.0
                    + reproductive_evidence * 1_000.0
                    + complement * 10_000.0;
                if best.is_none_or(|(_, _, best_score)| score > best_score) {
                    best = Some((*hash_a, *hash_b, score));
                }
            }
        }
        best.map(|(a, b, _)| (a, b))
    }

    /// Clone the present ecosystem three ways: intact, without B, and without A.
    /// Reproduction is normalized by instructions executed, preventing the
    /// removed organisms' freed CPU share from masquerading as a benefit.
    pub fn counterfactual_symbiosis(&self, horizon: u64) -> Option<SymbiosisReport> {
        let (genome_a, genome_b) = self.candidate_partner_pair()?;
        let mut intact = self.clone();
        let mut without_b = self.clone();
        let mut without_a = self.clone();
        without_b.remove_genome(genome_b);
        without_a.remove_genome(genome_a);

        let intact_before = intact.measure_genomes(genome_a, genome_b);
        let without_b_before = without_b.measure_genomes(genome_a, genome_b);
        let without_a_before = without_a.measure_genomes(genome_a, genome_b);
        for _ in 0..horizon {
            intact.tick();
            without_b.tick();
            without_a.tick();
        }
        let intact_after = intact.measure_genomes(genome_a, genome_b);
        let without_b_after = without_b.measure_genomes(genome_a, genome_b);
        let without_a_after = without_a.measure_genomes(genome_a, genome_b);

        let baseline_a = intact_after.0.saturating_sub(intact_before.0);
        let baseline_b = intact_after.1.saturating_sub(intact_before.1);
        let no_b_a = without_b_after.0.saturating_sub(without_b_before.0);
        let no_a_b = without_a_after.1.saturating_sub(without_a_before.1);
        let baseline_steps_a = intact_after.2.saturating_sub(intact_before.2);
        let baseline_steps_b = intact_after.3.saturating_sub(intact_before.3);
        let no_b_steps_a = without_b_after.2.saturating_sub(without_b_before.2);
        let no_a_steps_b = without_a_after.3.saturating_sub(without_a_before.3);
        let baseline_rate_a = baseline_a as f64 / baseline_steps_a.max(1) as f64;
        let baseline_rate_b = baseline_b as f64 / baseline_steps_b.max(1) as f64;
        let no_b_rate_a = no_b_a as f64 / no_b_steps_a.max(1) as f64;
        let no_a_rate_b = no_a_b as f64 / no_a_steps_b.max(1) as f64;
        let dependence_a = relative_loss(baseline_rate_a, no_b_rate_a);
        let dependence_b = relative_loss(baseline_rate_b, no_a_rate_b);
        let enough_evidence = baseline_a >= 2 && baseline_b >= 2;
        let a_depends = enough_evidence && dependence_a >= 0.2;
        let b_depends = enough_evidence && dependence_b >= 0.2;
        let verdict = match (a_depends, b_depends) {
            (true, true) => RelationshipVerdict::Mutualism,
            (true, false) => RelationshipVerdict::ADependsOnB,
            (false, true) => RelationshipVerdict::BDependsOnA,
            (false, false) if enough_evidence && dependence_a <= -0.2 && dependence_b <= -0.2 => {
                RelationshipVerdict::Competition
            }
            _ => RelationshipVerdict::Inconclusive,
        };
        Some(SymbiosisReport {
            genome_a,
            genome_b,
            horizon,
            baseline_births_a: baseline_a,
            baseline_births_b: baseline_b,
            dependence_a,
            dependence_b,
            verdict,
        })
    }

    fn measure_genomes(&self, a: u64, b: u64) -> (u64, u64, u64, u64) {
        (
            self.births_by_parent_genome.get(&a).copied().unwrap_or(0),
            self.births_by_parent_genome.get(&b).copied().unwrap_or(0),
            self.steps_by_genome.get(&a).copied().unwrap_or(0),
            self.steps_by_genome.get(&b).copied().unwrap_or(0),
        )
    }

    fn remove_genome(&mut self, genome: u64) {
        let ids: Vec<_> = self
            .programs
            .values()
            .filter(|program| self.genome_hash(program) == genome)
            .map(|program| program.id)
            .collect();
        for id in ids {
            if let Some(program) = self.programs.remove(&id) {
                if let Some((start, length)) = program.pending_allocation {
                    self.free_list.free(start, length);
                }
                for offset in 0..program.length {
                    self.addr_to_owner[program.start.wrapping_add(offset) as usize] = None;
                }
                self.free_list.free(program.start, program.length);
                self.ambient_pool += program.energy as u64;
            }
        }
    }
}

fn environment_origin(seed: u64) -> u16 {
    // SplitMix64 finalizer gives each seed a stable, well-distributed spatial phase.
    let mut value = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    let mixed = value ^ (value >> 31);
    (mixed % 64_512) as u16
}

#[cfg(test)]
fn circular_distance(a: u16, b: u16) -> u16 {
    a.wrapping_sub(b).min(b.wrapping_sub(a))
}

fn relative_loss(baseline: f64, counterfactual: f64) -> f64 {
    if baseline <= f64::EPSILON {
        0.0
    } else {
        ((baseline - counterfactual) / baseline).clamp(-10.0, 1.0)
    }
}

fn genome_hash_in_memory(memory: &Memory, program: &Program) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in memory.read_slice(program.start, program.length) {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash ^ program.length as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Config that skips the templates directory so tests always fall back to the
    /// hardcoded SEED (single program, deterministic initial state).
    fn seed_config() -> Config {
        Config {
            templates_dir: PathBuf::from("/nonexistent_soup_test_templates"),
            ..Config::default()
        }
    }

    #[test]
    fn world_initializes_with_seed() {
        let world = World::new(seed_config());
        assert_eq!(world.live_count(), 1);
        assert!(world.memory_utilization() > 0.0);
        // The single ancestor occupies exactly its genome length.
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
    fn energy_current_moves_deposits_around_the_ring() {
        let mut cfg = seed_config();
        cfg.resource_sources.clear();
        cfg.energy_decay_interval = 1;
        cfg.energy_decay_rate = 0;
        cfg.energy_current = 17;
        let mut world = World::new(cfg);
        world.memory.give_energy(60_000, 123);

        world.tick();

        assert_eq!(world.memory.sense_energy(60_000), 0);
        assert_eq!(world.memory.sense_energy(60_017), 123);
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
    fn organisms_senesce_after_their_instruction_budget() {
        let mut cfg = seed_config();
        cfg.max_program_age = 3;
        cfg.initial_energy = 100;
        cfg.resource_sources.clear();
        let mut world = World::new(cfg);

        let events = world.run(3);

        assert!(events.iter().any(|event| matches!(
            event,
            Event::Died {
                cause: DeathCause::Senescence,
                ..
            }
        )));
        assert_eq!(world.live_count(), 0);
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
        assert!(world.total_births > 0);
        assert!(world.max_generation >= 1);
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
        assert!(world.total_mutations > 0);
    }

    #[test]
    fn insertion_mutation_changes_child_length_and_stays_local() {
        let mut cfg = seed_config();
        cfg.initial_energy = 10_000;
        cfg.mutation_rate = 0.0;
        cfg.insertion_rate = 1.0;
        cfg.deletion_rate = 0.0;
        cfg.duplication_rate = 0.0;
        cfg.tag_mutation_rate = 0.0;
        cfg.child_locality_bias = 1.0;
        let mut world = World::new(cfg);
        let parent_start = world.programs[&0].start;
        let mut child = None;

        for _ in 0..10_000 {
            for event in world.tick() {
                if let Event::StructuralMutation {
                    id,
                    kind: StructuralMutationKind::Insertion,
                    old_length,
                    new_length,
                    ..
                } = event
                {
                    child = Some(id);
                    assert_eq!(old_length, crate::seed::SEED_LEN);
                    assert_eq!(new_length, crate::seed::SEED_LEN + 1);
                }
            }
            if child.is_some() {
                break;
            }
        }

        let child = &world.programs[&child.expect("an insertion-bearing child")];
        let distance = child
            .start
            .wrapping_sub(parent_start)
            .min(parent_start.wrapping_sub(child.start));
        assert!(distance <= crate::seed::SEED_LEN + 1);
    }

    #[test]
    fn deletion_mutation_shortens_the_committed_genome() {
        let mut cfg = seed_config();
        cfg.initial_energy = 10_000;
        cfg.mutation_rate = 0.0;
        cfg.insertion_rate = 0.0;
        cfg.deletion_rate = 1.0;
        cfg.duplication_rate = 0.0;
        cfg.tag_mutation_rate = 0.0;
        let mut world = World::new(cfg);

        let event = (0..10_000).find_map(|_| {
            world.tick().into_iter().find(|event| {
                matches!(
                    event,
                    Event::StructuralMutation {
                        kind: StructuralMutationKind::Deletion,
                        ..
                    }
                )
            })
        });
        let Event::StructuralMutation {
            old_length,
            new_length,
            ..
        } = event.expect("a deletion-bearing child")
        else {
            unreachable!()
        };
        assert!(new_length < old_length);
    }

    #[test]
    fn duplication_mutation_expands_the_committed_genome() {
        let mut cfg = seed_config();
        cfg.initial_energy = 10_000;
        cfg.mutation_rate = 0.0;
        cfg.insertion_rate = 0.0;
        cfg.deletion_rate = 0.0;
        cfg.duplication_rate = 1.0;
        cfg.tag_mutation_rate = 0.0;
        let mut world = World::new(cfg);

        let event = (0..10_000).find_map(|_| {
            world.tick().into_iter().find(|event| {
                matches!(
                    event,
                    Event::StructuralMutation {
                        kind: StructuralMutationKind::Duplication,
                        ..
                    }
                )
            })
        });
        let Event::StructuralMutation {
            old_length,
            new_length,
            ..
        } = event.expect("a duplication-bearing child")
        else {
            unreachable!()
        };
        assert!(new_length > old_length);
    }

    #[test]
    fn second_generation_replicates() {
        // Verify that children can themselves produce grandchildren.
        // This validates the full replication cycle.
        let mut cfg = seed_config();
        cfg.initial_energy = 50_000;
        cfg.mutation_rate = 0.0; // no mutation — pure replication test
        let mut world = World::new(cfg);

        let mut generations: std::collections::HashSet<crate::program::ProgramId> =
            std::collections::HashSet::new();
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
        assert!(
            grandchildren_born > 0,
            "Should observe at least one grandchild"
        );
        assert!(world.max_generation >= 2);
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

    #[test]
    fn resource_sources_do_not_emit_at_tick_zero() {
        let world = World::new(seed_config());
        assert!(world.scheduled_resources(0).is_empty());
    }

    #[test]
    fn saturated_source_cell_preserves_existing_donor() {
        let mut world = World::new(seed_config());
        let start = 123;
        world.memory.energy_map[start as usize] = u32::MAX;
        world.memory.resource_a_donor[start as usize] = Some(7);
        let ambient_before = world.ambient_pool;

        world.apply_resource_deposit(ResourceDeposit {
            kind: ResourceKind::A,
            start,
            width: 1,
            amount: 10,
        });

        assert_eq!(world.memory.resource_a_donor[start as usize], Some(7));
        assert_eq!(world.ambient_pool, ambient_before);
    }

    #[test]
    fn external_resource_schedule_is_independent_of_organisms() {
        let cfg = seed_config();
        let intact = World::new(cfg.clone());
        let mut empty = intact.clone();
        let ancestor = empty.genome_hash(&empty.programs[&0]);
        empty.remove_genome(ancestor);
        for _ in 0..100 {
            empty.rng.gen::<u64>();
        }

        for tick in 1..=1_000 {
            assert_eq!(
                intact.scheduled_resources(tick),
                empty.scheduled_resources(tick),
                "external schedule diverged at tick {tick}"
            );
        }
    }

    #[test]
    fn external_resource_schedule_replays_from_seed_and_config() {
        let left = World::new(seed_config());
        let right = World::new(seed_config());

        let left_schedule: Vec<_> = (1..=1_000)
            .flat_map(|tick| left.scheduled_resources(tick))
            .collect();
        let right_schedule: Vec<_> = (1..=1_000)
            .flat_map(|tick| right.scheduled_resources(tick))
            .collect();

        assert_eq!(left_schedule, right_schedule);
        assert!(!left_schedule.is_empty());
    }

    #[test]
    fn seed_changes_the_resource_schedule_spatial_phase() {
        let left = World::new(seed_config());
        let mut other_config = seed_config();
        other_config.rng_seed = other_config.rng_seed.wrapping_add(1);
        let right = World::new(other_config);

        assert_ne!(left.scheduled_resources(10), right.scheduled_resources(10));
    }

    #[test]
    fn multiple_sources_create_different_local_conditions() {
        let mut world = World::new(seed_config());
        let ancestor = world.genome_hash(&world.programs[&0]);
        world.remove_genome(ancestor);
        world.run(10);

        let origin = world.environment_origin();
        let sum_window = |map: &[u32; 65536], start: u16| -> u64 {
            (0..256)
                .map(|offset| map[start.wrapping_add(offset) as usize] as u64)
                .sum()
        };
        let a_near_a = sum_window(&world.memory.energy_map, origin);
        let b_near_a = sum_window(&world.memory.resource_b_map, origin);
        let distant = origin.wrapping_add(16_384);
        let a_distant = sum_window(&world.memory.energy_map, distant);
        let b_distant = sum_window(&world.memory.resource_b_map, distant);

        assert_ne!((a_near_a, b_near_a), (a_distant, b_distant));
        assert!(
            a_near_a > b_near_a,
            "A source should dominate its local niche"
        );
        assert!(
            b_distant > a_distant,
            "B source should dominate its local niche"
        );
    }

    #[test]
    fn ancestor_starts_with_both_external_resources_in_reach() {
        let world = World::new(seed_config());
        let ancestor = &world.programs[&0];
        let first_tick = world
            .config
            .resource_sources
            .iter()
            .map(|source| source.interval)
            .filter(|interval| *interval > 0)
            .min()
            .expect("at least one enabled resource source");
        let first_deposits = world.scheduled_resources(first_tick);

        for kind in [
            crate::events::ResourceKind::A,
            crate::events::ResourceKind::B,
        ] {
            assert!(first_deposits.iter().any(|deposit| {
                deposit.kind == kind
                    && circular_distance(ancestor.start, deposit.start)
                        <= world.config.interaction_radius
            }));
        }
    }

    #[test]
    fn ancestor_remains_viable_without_targeted_rain() {
        let mut cfg = seed_config();
        cfg.rng_seed = 14_201;
        cfg.initial_energy = 500;
        cfg.mutation_rate = 0.0;
        cfg.insertion_rate = 0.0;
        cfg.deletion_rate = 0.0;
        cfg.duplication_rate = 0.0;
        cfg.tag_mutation_rate = 0.0;
        let mut world = World::new(cfg);

        world.run(100_000);

        assert!(
            world.total_births > 0,
            "ancestor should reproduce from fixed sources"
        );
        assert!(world.live_count() > 0, "the lineage should remain alive");
    }

    #[test]
    fn external_sources_preserve_total_energy() {
        let mut world = World::new(seed_config());

        for _ in 0..10_000 {
            world.tick();
            let program_energy: u64 = world.programs.values().map(|p| p.energy as u64).sum();
            let resource_a: u64 = world.memory.energy_map.iter().map(|&v| v as u64).sum();
            let resource_b: u64 = world.memory.resource_b_map.iter().map(|&v| v as u64).sum();
            assert_eq!(
                world.ambient_pool + program_energy + resource_a + resource_b,
                world.config.total_energy,
                "energy was not conserved at tick {}",
                world.tick
            );
        }
    }
}

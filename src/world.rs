use crate::canonical::{self, Encoder};
use crate::{
    allocator::FreeList,
    config::Config,
    ecotype::{
        viable_ecotypes, BehaviorObservation, ObservationTermination, ViabilityRule, ViableEcotype,
    },
    events::{DeathCause, Event, ResourceKind, StructuralMutationKind},
    identity::{EcotypeEquivalence, HeritableIdentity},
    memory::Memory,
    mutation::{self, Direction},
    opcode::Opcode,
    program::{Program, ProgramId},
    template,
    vm::{self, StepResult},
};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha12Rng;
use std::collections::{BTreeMap, HashMap, VecDeque};

#[derive(Debug, Clone)]
struct ActiveBehaviorSegment {
    identity: HeritableIdentity,
    start_tick: u64,
    began_at_birth: bool,
    reproductive_output: u64,
    offspring_ids: Vec<ProgramId>,
}

#[derive(Debug, Clone)]
pub struct SymbiosisReport {
    pub heritable_identity_a: HeritableIdentity,
    pub heritable_identity_b: HeritableIdentity,
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
    rng: ChaCha12Rng,
    run_namespace: [u8; 32],
    birth_history: [u8; 32],
    /// Names of the startup templates, indexed by template_id.
    pub template_names: Vec<String>,
    /// Startup genomes, indexed by template_id, used as the evolutionary baseline.
    pub template_bytes: Vec<Vec<u8>>,
    /// The ambient energy pool — conserved total minus organism and deposited resources.
    /// Burns from instruction execution return here; drip deposits from here to the
    /// resource maps; deaths return remaining program energy here.
    pub ambient_pool: u64,
    /// Per-cell ownership map: addr_to_owner[addr] = Some(id) if a live program owns that byte.
    /// Kept in sync on spawn and death; used by local organism seeks and ForeignExec tracking.
    pub addr_to_owner: Box<[Option<ProgramId>]>,
    /// Current tag by program ID. Dead IDs remain as harmless historical entries.
    pub program_tags: Vec<u8>,
    /// Successful reproduction attributed to the parent's complete heritable identity.
    pub births_by_parent_heritable_identity: HashMap<HeritableIdentity, u64>,
    /// Tick of the latest successful reproduction by each heritable identity.
    pub last_birth_by_heritable_identity: HashMap<HeritableIdentity, u64>,
    /// Most recently observed heritable identity for every program ID, retained after death.
    pub heritable_identity_by_id: Vec<HeritableIdentity>,
    /// Completed execution segments, retained permanently after identity changes and death.
    pub behavior_archive: Vec<BehaviorObservation>,
    active_behavior_segments: HashMap<ProgramId, ActiveBehaviorSegment>,
    announced_ecotypes: std::collections::HashSet<EcotypeEquivalence>,
    viable_ecotypes_cache: BTreeMap<EcotypeEquivalence, ViableEcotype>,
    /// Cross-identity resources consumed, keyed by (donor, receiver).
    pub interactions: HashMap<(HeritableIdentity, HeritableIdentity), u64>,
    /// Executed instructions attributed to the heritable identity present before each step.
    pub steps_by_heritable_identity: HashMap<HeritableIdentity, u64>,
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

mod digest;

impl World {
    /// Create a new World, loading templates and placing each at random addresses.
    pub fn new(config: Config) -> Self {
        let templates = template::load_templates(&config.templates_dir);
        let run_namespace = canonical::namespace(&config, &templates);
        let mut birth_history = run_namespace;
        let num = templates.len();
        let mut startup_rng = ChaCha12Rng::seed_from_u64(config.rng_seed ^ 0x510a_f00d);

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
        let mut remaining_energy = config.total_energy;

        for (i, &(start, len)) in placements.iter().enumerate() {
            let seed_energy = remaining_energy.min(config.initial_energy as u64) as u32;
            remaining_energy -= seed_energy as u64;
            let mut prog = Program::new(
                i as ProgramId,
                start,
                len,
                seed_energy,
                None,
                None,
                Some(i as u8),
            );
            prog.mutation_strategy = config.ancestor_mutation_strategy();
            let mut identity = Encoder::new("startup-lineage/v1");
            identity.value(&run_namespace);
            identity.value(&birth_history);
            identity.value(&prog);
            identity.value(&templates[i].bytes);
            birth_history = identity.finish();
            prog.lineage_id = canonical::uuid(birth_history);
            programs.insert(i as ProgramId, prog);
            queue.push_back(i as ProgramId);
        }

        let template_names = templates.iter().map(|t| t.name.clone()).collect();
        let template_bytes = templates.into_iter().map(|t| t.bytes).collect();

        let ambient_pool = remaining_energy;

        let mut addr_to_owner: Box<[Option<ProgramId>]> = vec![None; 65536].into_boxed_slice();
        let mut program_tags = vec![0; num];
        for prog in programs.values() {
            for offset in 0..prog.length as usize {
                let addr = (prog.start as usize + offset) % 65536;
                addr_to_owner[addr] = Some(prog.id);
            }
            program_tags[prog.id as usize] = prog.tag;
        }
        let mut heritable_identity_by_id = vec![HeritableIdentity::new(0, 0); num];
        for program in programs.values() {
            heritable_identity_by_id[program.id as usize] = HeritableIdentity::with_strategy(
                genome_hash_in_memory(&memory, program),
                program.tag,
                program.mutation_strategy,
            );
        }
        let active_behavior_segments = programs
            .values()
            .map(|program| {
                (
                    program.id,
                    ActiveBehaviorSegment {
                        identity: heritable_identity_by_id[program.id as usize],
                        start_tick: 0,
                        began_at_birth: true,
                        reproductive_output: 0,
                        offspring_ids: Vec::new(),
                    },
                )
            })
            .collect();

        World {
            memory,
            free_list,
            programs,
            queue,
            rng: ChaCha12Rng::seed_from_u64(config.rng_seed),
            run_namespace,
            birth_history,
            config,
            tick: 0,
            next_id: num as ProgramId,
            template_names,
            template_bytes,
            ambient_pool,
            addr_to_owner,
            program_tags,
            births_by_parent_heritable_identity: HashMap::new(),
            last_birth_by_heritable_identity: HashMap::new(),
            heritable_identity_by_id,
            behavior_archive: Vec::new(),
            active_behavior_segments,
            announced_ecotypes: std::collections::HashSet::new(),
            viable_ecotypes_cache: BTreeMap::new(),
            interactions: HashMap::new(),
            steps_by_heritable_identity: HashMap::new(),
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
            for index in 0..self.memory.energy_map.len() {
                let decay = self.memory.energy_map[index].min(rate);
                self.memory.take_energy_up_to(index as u16, decay);
                self.ambient_pool += decay as u64;
            }
            for index in 0..self.memory.resource_b_map.len() {
                let decay = self.memory.resource_b_map[index].min(rate);
                self.memory.take_resource_b_up_to(index as u16, decay);
                self.ambient_pool += decay as u64;
            }
            let current = self.config.energy_current % self.memory.energy_map.len();
            self.memory.energy_map.rotate_right(current);
            self.memory.resource_a_provenance.rotate_right(current);
            self.memory.resource_b_map.rotate_left(current);
            self.memory.resource_b_provenance.rotate_left(current);
        }

        // External sources have their own deterministic schedule. They never inspect
        // live organisms or consume the VM/mutation RNG stream.
        for deposit in self.scheduled_resources(self.tick) {
            self.apply_resource_deposit(deposit);
        }

        // Pop the next program ID (skipping dead ones lazily).
        let id = loop {
            match self.queue.pop_front() {
                None => {
                    self.refresh_viable_ecotypes(&mut events);
                    return events;
                } // no live programs
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

        let executing_heritable_identity = self
            .programs
            .get(&id)
            .map(|program| self.heritable_identity(program))
            .unwrap_or(HeritableIdentity::new(0, 0));
        self.segment_identity_change(id, executing_heritable_identity);
        if let Some(heritable_identity) = self.heritable_identity_by_id.get_mut(id as usize) {
            *heritable_identity = executing_heritable_identity;
        }
        *self
            .steps_by_heritable_identity
            .entry(executing_heritable_identity)
            .or_default() += 1;
        let write_victim = self.programs.get(&id).and_then(|program| {
            matches!(
                Opcode::from(self.memory.read(program.ip)),
                Opcode::Write | Opcode::Copy
            )
            .then(|| self.addr_to_owner[program.wh as usize])
            .flatten()
        });
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
        let current_heritable_identity = self.programs.get(&id).map(|program| {
            if let Some(tag) = self.program_tags.get_mut(id as usize) {
                *tag = program.tag;
            }
            HeritableIdentity::with_strategy(
                genome_hash_in_memory(&self.memory, program),
                program.tag,
                program.mutation_strategy,
            )
        });
        if let Some(current_heritable_identity) = current_heritable_identity {
            if let Some(heritable_identity) = self.heritable_identity_by_id.get_mut(id as usize) {
                *heritable_identity = current_heritable_identity;
            }
            self.segment_identity_change(id, current_heritable_identity);
        }
        if let Some(victim_id) = write_victim.filter(|victim_id| *victim_id != id) {
            let victim_identity = self.programs.get(&victim_id).map(|program| {
                HeritableIdentity::with_strategy(
                    genome_hash_in_memory(&self.memory, program),
                    program.tag,
                    program.mutation_strategy,
                )
            });
            if let Some(victim_identity) = victim_identity {
                if let Some(historical) = self.heritable_identity_by_id.get_mut(victim_id as usize)
                {
                    *historical = victim_identity;
                }
                self.segment_identity_change(victim_id, victim_identity);
            }
        }

        match result {
            StepResult::Continue => {
                let senescent = self.config.max_program_age > 0
                    && self.programs[&id].age >= self.config.max_program_age;
                if senescent {
                    if let Some(mut p) = self.programs.remove(&id) {
                        self.finish_behavior_segment(&mut p, ObservationTermination::Death);
                        if let Some((start, length)) = p.pending_allocation {
                            self.free_list.free(start, length);
                        }
                        for offset in 0..p.length as usize {
                            self.addr_to_owner[(p.start as usize + offset) % 65536] = None;
                        }
                        self.ambient_pool +=
                            p.energy as u64 + p.metabolite_a as u64 + p.metabolite_b as u64;
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
                if let Some(mut p) = self.programs.remove(&id) {
                    self.finish_behavior_segment(&mut p, ObservationTermination::Death);
                    if let Some((start, length)) = p.pending_allocation {
                        self.free_list.free(start, length);
                    }
                    for offset in 0..p.length as usize {
                        self.addr_to_owner[(p.start as usize + offset) % 65536] = None;
                    }
                    self.ambient_pool +=
                        p.energy as u64 + p.metabolite_a as u64 + p.metabolite_b as u64;
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
                if let Some(mut p) = self.programs.remove(&id) {
                    self.finish_behavior_segment(&mut p, ObservationTermination::Death);
                    if let Some((start, length)) = p.pending_allocation {
                        self.free_list.free(start, length);
                    }
                    for offset in 0..p.length as usize {
                        self.addr_to_owner[(p.start as usize + offset) % 65536] = None;
                    }
                    self.ambient_pool +=
                        p.energy as u64 + p.metabolite_a as u64 + p.metabolite_b as u64; // always 0, but explicit
                    self.free_list.free(p.start, p.length);
                    events.push(Event::Died {
                        tick: self.tick,
                        id,
                        cause: DeathCause::Energy,
                    });
                }
            }
            StepResult::Spawned(mut child) => {
                let parent_heritable_identity = self
                    .programs
                    .get(&id)
                    .map(|parent| self.heritable_identity(parent))
                    .unwrap_or(HeritableIdentity::new(0, 0));
                *self
                    .births_by_parent_heritable_identity
                    .entry(parent_heritable_identity)
                    .or_default() += 1;
                self.last_birth_by_heritable_identity
                    .insert(parent_heritable_identity, self.tick);
                if let Some(segment) = self.active_behavior_segments.get_mut(&id) {
                    segment.reproductive_output += 1;
                    segment.offspring_ids.push(child.id);
                }
                let parent_start = self.programs[&id].start;
                self.apply_birth_mutations(&mut child, parent_start, &mut events);
                // Include the entire birth-time simulation state, not only the child
                // genome or numeric ID: counterfactual interventions must split history.
                let mut identity = Encoder::new("birth-lineage/v1");
                identity.value(&self.state_hash(false));
                identity.value(child.as_ref());
                identity.value(&self.memory.read_slice(child.start, child.length));
                self.birth_history = identity.finish();
                child.lineage_id = canonical::uuid(self.birth_history);
                let child_heritable_identity = self.heritable_identity(&child);
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
                    heritable_identity: child_heritable_identity,
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
                if self.heritable_identity_by_id.len() <= child_id as usize {
                    self.heritable_identity_by_id
                        .resize(child_id as usize + 1, HeritableIdentity::new(0, 0));
                }
                self.heritable_identity_by_id[child_id as usize] = child_heritable_identity;
                self.active_behavior_segments.insert(
                    child_id,
                    ActiveBehaviorSegment {
                        identity: child_heritable_identity,
                        start_tick: self.tick,
                        began_at_birth: true,
                        reproductive_output: 0,
                        offspring_ids: Vec::new(),
                    },
                );
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
                    donor_heritable_identity,
                    receiver_heritable_identity,
                    amount,
                    ..
                } => {
                    self.record_resource_transfer(
                        *donor_heritable_identity,
                        *receiver_heritable_identity,
                        *amount,
                    );
                }
                _ => {}
            }
        }

        let viability_checkpoint = self
            .tick
            .is_multiple_of(self.config.ecotype_min_persistence_ticks.clamp(1, 10_000));
        if viability_checkpoint {
            self.refresh_viable_ecotypes(&mut events);
        }

        events
    }

    fn refresh_viable_ecotypes(&mut self, events: &mut Vec<Event>) {
        let evaluated = viable_ecotypes(
            &self.behavior_observations(),
            self.tick,
            self.viability_rule(),
        );
        for (&equivalence, report) in &evaluated {
            if self.announced_ecotypes.insert(equivalence) {
                events.push(Event::NewProgram {
                    tick: self.tick,
                    ecotype_identity: report.identity,
                    equivalent_raw_genomes: report.equivalent_raw_genomes,
                    persistence_ticks: report.persistence_ticks,
                    reproductive_output: report.reproductive_output,
                    descendant_generations: report.descendant_generations,
                });
            }
        }
        self.viable_ecotypes_cache = evaluated;
    }

    fn segment_identity_change(&mut self, id: ProgramId, identity: HeritableIdentity) {
        let Some(active) = self.active_behavior_segments.get(&id) else {
            self.active_behavior_segments.insert(
                id,
                ActiveBehaviorSegment {
                    identity,
                    start_tick: self.tick,
                    began_at_birth: false,
                    reproductive_output: 0,
                    offspring_ids: Vec::new(),
                },
            );
            return;
        };
        if active.identity == identity {
            return;
        }
        let Some(mut active) = self.active_behavior_segments.remove(&id) else {
            return;
        };
        let Some(program) = self.programs.get_mut(&id) else {
            return;
        };
        self.behavior_archive.push(BehaviorObservation {
            program_id: id,
            parent_id: program.parent_id,
            generation: program.generation,
            began_at_birth: active.began_at_birth,
            identity: active.identity,
            behavior: std::mem::take(&mut program.trace),
            start_tick: active.start_tick,
            end_tick: Some(self.tick),
            reproductive_output: active.reproductive_output,
            offspring_ids: std::mem::take(&mut active.offspring_ids),
            termination: ObservationTermination::IdentityChanged,
        });
        self.active_behavior_segments.insert(
            id,
            ActiveBehaviorSegment {
                identity,
                start_tick: self.tick,
                began_at_birth: false,
                reproductive_output: 0,
                offspring_ids: Vec::new(),
            },
        );
    }

    fn finish_behavior_segment(
        &mut self,
        program: &mut Program,
        termination: ObservationTermination,
    ) {
        let Some(mut active) = self.active_behavior_segments.remove(&program.id) else {
            return;
        };
        self.behavior_archive.push(BehaviorObservation {
            program_id: program.id,
            parent_id: program.parent_id,
            generation: program.generation,
            began_at_birth: active.began_at_birth,
            identity: active.identity,
            behavior: std::mem::take(&mut program.trace),
            start_tick: active.start_tick,
            end_tick: Some(self.tick),
            reproductive_output: active.reproductive_output,
            offspring_ids: std::mem::take(&mut active.offspring_ids),
            termination,
        });
    }

    /// Completed archive plus snapshots of every currently active segment.
    pub fn behavior_observations(&self) -> Vec<BehaviorObservation> {
        let mut observations = self.behavior_archive.clone();
        observations.extend(self.programs.values().filter_map(|program| {
            let active = self.active_behavior_segments.get(&program.id)?;
            Some(BehaviorObservation {
                program_id: program.id,
                parent_id: program.parent_id,
                generation: program.generation,
                began_at_birth: active.began_at_birth,
                identity: active.identity,
                behavior: program.trace.clone(),
                start_tick: active.start_tick,
                end_tick: None,
                reproductive_output: active.reproductive_output,
                offspring_ids: active.offspring_ids.clone(),
                termination: ObservationTermination::Live,
            })
        }));
        observations
    }

    pub fn viability_rule(&self) -> ViabilityRule {
        ViabilityRule {
            min_persistence_ticks: self.config.ecotype_min_persistence_ticks,
            min_reproductive_output: self.config.ecotype_min_reproductive_output,
            min_descendant_generations: self.config.ecotype_min_descendant_generations,
        }
    }

    pub fn viable_ecotypes(&self) -> &BTreeMap<EcotypeEquivalence, ViableEcotype> {
        &self.viable_ecotypes_cache
    }

    pub fn viable_ecotype_count(&self) -> usize {
        self.viable_ecotypes_cache.len()
    }

    fn record_resource_transfer(
        &mut self,
        donor: HeritableIdentity,
        receiver: HeritableIdentity,
        amount: u32,
    ) {
        if donor != receiver {
            *self.interactions.entry((donor, receiver)).or_default() += amount as u64;
        }
    }

    fn apply_birth_mutations(
        &mut self,
        child: &mut Program,
        parent_start: u16,
        events: &mut Vec<Event>,
    ) {
        let kind = child.mutation_strategy.structural_kind(self.rng.gen());

        if let Some(kind) = kind {
            let old_length = child.length;
            let mut genome = self.memory.read_slice(child.start, child.length);
            let mut mutation_index = 0usize;
            match kind {
                StructuralMutationKind::Insertion
                    if child.length < self.config.max_genome_length =>
                {
                    mutation_index = self.rng.gen_range(0..=genome.len());
                    genome.insert(mutation_index, mutation::insert(self.rng.gen()));
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

        if (self.rng.gen::<u16>() as u32) < child.mutation_strategy.strategy_mutation_rate {
            let locus = self
                .rng
                .gen_range(0..mutation::MutationStrategy::LOCUS_COUNT);
            let direction = if self.rng.gen::<bool>() {
                Direction::Higher
            } else {
                Direction::Lower
            };
            child.mutation_strategy = child.mutation_strategy.mutate_locus(locus, direction);
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
            };
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

    /// Every configured budget unit is in exactly one reservoir.
    pub fn accounted_budget(&self) -> u64 {
        let organisms: u64 = self
            .programs
            .values()
            .map(|p| p.energy as u64 + p.metabolite_a as u64 + p.metabolite_b as u64)
            .sum();
        let resource_a: u64 = self
            .memory
            .energy_map
            .iter()
            .map(|&amount| amount as u64)
            .sum();
        let resource_b: u64 = self
            .memory
            .resource_b_map
            .iter()
            .map(|&amount| amount as u64)
            .sum();
        self.ambient_pool + organisms + resource_a + resource_b
    }

    /// Memory utilization as fraction 0.0..=1.0
    pub fn memory_utilization(&self) -> f64 {
        let free = self.free_list.free_bytes() as f64;
        1.0 - free / 65536.0
    }

    /// Stable fingerprint of an organism's current bytes. Equal hashes represent
    /// equal byte sequences; recognition state is intentionally represented by `HeritableIdentity`.
    pub fn genome_hash(&self, program: &Program) -> u64 {
        genome_hash_in_memory(&self.memory, program)
    }

    /// Identity combines executable bytes, recognition tag, and mutation strategy.
    pub fn heritable_identity(&self, program: &Program) -> HeritableIdentity {
        HeritableIdentity::with_strategy(
            self.genome_hash(program),
            program.tag,
            program.mutation_strategy,
        )
    }

    /// Number of distinct live byte sequences, independent of recognition tag.
    pub fn live_genomes(&self) -> usize {
        self.programs
            .values()
            .map(|program| self.genome_hash(program))
            .collect::<std::collections::HashSet<_>>()
            .len()
    }

    /// Number of distinct live byte-and-tag heritable identities.
    pub fn live_heritable_identities(&self) -> usize {
        self.programs
            .values()
            .map(|program| self.heritable_identity(program))
            .collect::<std::collections::HashSet<_>>()
            .len()
    }

    /// Remove every live organism with this raw byte genome, regardless of tag.
    #[cfg(test)]
    fn remove_genome(&mut self, genome: u64) {
        let identities: Vec<_> = self
            .programs
            .values()
            .filter(|program| self.genome_hash(program) == genome)
            .map(|program| self.heritable_identity(program))
            .collect();
        for identity in identities {
            self.remove_heritable_identity(identity);
        }
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

    /// Pick the strongest live candidate pair, preserving byte-and-tag clades and
    /// preferring abundant heritable identities with opposite A/B harvesting profiles. This is
    /// only hypothesis generation; `counterfactual_symbiosis` performs removal.
    pub fn candidate_partner_pair(&self) -> Option<(HeritableIdentity, HeritableIdentity)> {
        let live_heritable_identities: std::collections::HashSet<_> = self
            .programs
            .values()
            .map(|program| self.heritable_identity(program))
            .collect();
        let mut transferred_by_pair: HashMap<(HeritableIdentity, HeritableIdentity), u64> =
            HashMap::new();
        for (&(donor, receiver), &amount) in &self.interactions {
            let active = |heritable_identity: HeritableIdentity| {
                self.births_by_parent_heritable_identity
                    .get(&heritable_identity)
                    .is_some_and(|births| *births >= 2)
                    && self
                        .last_birth_by_heritable_identity
                        .get(&heritable_identity)
                        .is_some_and(|tick| self.tick.saturating_sub(*tick) <= 100_000)
            };
            if donor != receiver
                && live_heritable_identities.contains(&donor)
                && live_heritable_identities.contains(&receiver)
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
        if let Some((pair, _)) = transferred_by_pair.into_iter().max_by(
            |(left_pair, left_amount), (right_pair, right_amount)| {
                left_amount
                    .cmp(right_amount)
                    .then_with(|| right_pair.cmp(left_pair))
            },
        ) {
            return Some(pair);
        }

        #[derive(Default)]
        struct Phenotype {
            population: u64,
            a: u64,
            b: u64,
            births: u64,
        }
        let mut phenotypes: HashMap<HeritableIdentity, Phenotype> = HashMap::new();
        for program in self.programs.values() {
            let heritable_identity = self.heritable_identity(program);
            let phenotype = phenotypes.entry(heritable_identity).or_default();
            phenotype.population += 1;
            phenotype.a += program.trace.opcode_counts[31];
            phenotype.b += program.trace.opcode_counts[37];
            phenotype.births = self
                .births_by_parent_heritable_identity
                .get(&heritable_identity)
                .copied()
                .unwrap_or(0);
        }
        let mut live: Vec<_> = phenotypes.into_iter().collect();
        let has_active_pair = live
            .iter()
            .filter(|(heritable_identity, phenotype)| {
                phenotype.births >= 2
                    && self
                        .last_birth_by_heritable_identity
                        .get(heritable_identity)
                        .is_some_and(|tick| self.tick.saturating_sub(*tick) <= 100_000)
            })
            .count()
            >= 2;
        if has_active_pair {
            live.retain(|(heritable_identity, phenotype)| {
                phenotype.births >= 2
                    && self
                        .last_birth_by_heritable_identity
                        .get(heritable_identity)
                        .is_some_and(|tick| self.tick.saturating_sub(*tick) <= 100_000)
            });
        }
        live.sort_by_key(|(identity, phenotype)| {
            (std::cmp::Reverse(phenotype.population), *identity)
        });
        live.truncate(12);

        let mut best: Option<((HeritableIdentity, HeritableIdentity), f64)> = None;
        for left in 0..live.len() {
            for right in left + 1..live.len() {
                let (heritable_identity_a, a) = &live[left];
                let (heritable_identity_b, b) = &live[right];
                let preference_a = a.a as f64 / (a.a + a.b).max(1) as f64;
                let preference_b = b.a as f64 / (b.a + b.b).max(1) as f64;
                let complement = (preference_a - preference_b).abs();
                let abundance = a.population.min(b.population) as f64;
                let reproductive_evidence = a.births.min(b.births) as f64;
                let score = abundance * 1_000_000.0
                    + reproductive_evidence * 1_000.0
                    + complement * 10_000.0;
                let pair = (*heritable_identity_a, *heritable_identity_b);
                if best.is_none_or(|(best_pair, best_score)| {
                    score
                        .total_cmp(&best_score)
                        .then_with(|| best_pair.cmp(&pair))
                        .is_gt()
                }) {
                    best = Some((pair, score));
                }
            }
        }
        best.map(|(pair, _)| pair)
    }

    /// Clone the present ecosystem three ways: intact, without B, and without A.
    /// Reproduction is normalized by instructions executed, preventing the
    /// removed organisms' freed CPU share from masquerading as a benefit.
    pub fn counterfactual_symbiosis(&self, horizon: u64) -> Option<SymbiosisReport> {
        let pair = self.candidate_partner_pair()?;
        Some(self.counterfactual_symbiosis_for_pair(pair, horizon))
    }

    /// Run a counterfactual trial for an explicitly selected heritable-identity pair.
    pub fn counterfactual_symbiosis_for_pair(
        &self,
        pair: (HeritableIdentity, HeritableIdentity),
        horizon: u64,
    ) -> SymbiosisReport {
        self.counterfactual_symbiosis_for_pair_with_control(pair, horizon, |_| true)
            .expect("an uncancelled counterfactual always completes")
    }

    /// Runs a specified candidate pair while allowing a worker to report progress
    /// and cooperatively cancel between simulated ticks.
    pub(crate) fn counterfactual_symbiosis_for_pair_with_control<F>(
        &self,
        (heritable_identity_a, heritable_identity_b): (HeritableIdentity, HeritableIdentity),
        horizon: u64,
        mut continue_after: F,
    ) -> Option<SymbiosisReport>
    where
        F: FnMut(u64) -> bool,
    {
        let mut intact = self.clone();
        let mut without_b = self.clone();
        let mut without_a = self.clone();
        without_b.remove_heritable_identity(heritable_identity_b);
        without_a.remove_heritable_identity(heritable_identity_a);

        let intact_before =
            intact.measure_heritable_identities(heritable_identity_a, heritable_identity_b);
        let without_b_before =
            without_b.measure_heritable_identities(heritable_identity_a, heritable_identity_b);
        let without_a_before =
            without_a.measure_heritable_identities(heritable_identity_a, heritable_identity_b);
        if !continue_after(0) {
            return None;
        }
        for completed in 1..=horizon {
            intact.tick();
            without_b.tick();
            without_a.tick();
            if !continue_after(completed) {
                return None;
            }
        }
        let intact_after =
            intact.measure_heritable_identities(heritable_identity_a, heritable_identity_b);
        let without_b_after =
            without_b.measure_heritable_identities(heritable_identity_a, heritable_identity_b);
        let without_a_after =
            without_a.measure_heritable_identities(heritable_identity_a, heritable_identity_b);

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
            heritable_identity_a,
            heritable_identity_b,
            horizon,
            baseline_births_a: baseline_a,
            baseline_births_b: baseline_b,
            dependence_a,
            dependence_b,
            verdict,
        })
    }

    fn measure_heritable_identities(
        &self,
        a: HeritableIdentity,
        b: HeritableIdentity,
    ) -> (u64, u64, u64, u64) {
        (
            self.births_by_parent_heritable_identity
                .get(&a)
                .copied()
                .unwrap_or(0),
            self.births_by_parent_heritable_identity
                .get(&b)
                .copied()
                .unwrap_or(0),
            self.steps_by_heritable_identity
                .get(&a)
                .copied()
                .unwrap_or(0),
            self.steps_by_heritable_identity
                .get(&b)
                .copied()
                .unwrap_or(0),
        )
    }

    fn remove_heritable_identity(&mut self, heritable_identity: HeritableIdentity) {
        let ids: Vec<_> = self
            .programs
            .values()
            .filter(|program| self.heritable_identity(program) == heritable_identity)
            .map(|program| program.id)
            .collect();
        for id in ids {
            if let Some(mut program) = self.programs.remove(&id) {
                self.finish_behavior_segment(&mut program, ObservationTermination::Removed);
                if let Some((start, length)) = program.pending_allocation {
                    self.free_list.free(start, length);
                }
                for offset in 0..program.length {
                    self.addr_to_owner[program.start.wrapping_add(offset) as usize] = None;
                }
                self.free_list.free(program.start, program.length);
                self.ambient_pool += program.energy as u64
                    + program.metabolite_a as u64
                    + program.metabolite_b as u64;
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
    use crate::identity::HeritableIdentity;
    use std::path::PathBuf;

    /// Config that skips the templates directory so tests always fall back to the
    /// hardcoded SEED (single program, deterministic initial state).
    fn seed_config() -> Config {
        Config {
            templates_dir: PathBuf::from("/nonexistent_soup_test_templates"),
            ..Config::default()
        }
    }

    fn add_seed_clone(world: &mut World, id: ProgramId, tag: u8) -> HeritableIdentity {
        let source = &world.programs[&0];
        let bytes = world.memory.read_slice(source.start, source.length);
        let start = world
            .free_list
            .alloc(source.length)
            .expect("space for test organism");
        world.memory.place(start, &bytes);
        let mut program = Program::new(id, start, source.length, 1_000, Some(0), None, None);
        program.tag = tag;
        for offset in 0..program.length {
            world.addr_to_owner[start.wrapping_add(offset) as usize] = Some(id);
        }
        if world.program_tags.len() <= id as usize {
            world.program_tags.resize(id as usize + 1, 0);
        }
        world.program_tags[id as usize] = tag;
        let heritable_identity = world.heritable_identity(&program);
        if world.heritable_identity_by_id.len() <= id as usize {
            world
                .heritable_identity_by_id
                .resize(id as usize + 1, HeritableIdentity::new(0, 0));
        }
        world.heritable_identity_by_id[id as usize] = heritable_identity;
        world.active_behavior_segments.insert(
            id,
            ActiveBehaviorSegment {
                identity: heritable_identity,
                start_tick: world.tick,
                began_at_birth: true,
                reproductive_output: 0,
                offspring_ids: Vec::new(),
            },
        );
        world.programs.insert(id, program);
        heritable_identity
    }

    #[test]
    fn heritable_identity_distinguishes_recognition_tag_genome_and_strategy() {
        let mut world = World::new(seed_config());
        let program = world.programs[&0].clone();
        let genome = world.genome_hash(&program);

        let mut other_tag = program.clone();
        other_tag.tag = 7;
        assert_eq!(world.genome_hash(&other_tag), genome);
        assert_ne!(
            world.heritable_identity(&program),
            world.heritable_identity(&other_tag)
        );

        let original_heritable_identity = world.heritable_identity(&program);
        let mut other_genome = program.clone();
        world.memory.write(other_genome.start, 255);
        other_genome.tag = program.tag;
        assert_ne!(
            original_heritable_identity,
            world.heritable_identity(&other_genome)
        );

        world.memory.place(program.start, &world.template_bytes[0]);
        let mut other_strategy = program.clone();
        other_strategy.mutation_strategy.copy_error_rate += 1;
        assert_ne!(
            world.heritable_identity(&program),
            world.heritable_identity(&other_strategy)
        );

        let mut changed_world = world.clone();
        changed_world
            .programs
            .get_mut(&0)
            .unwrap()
            .mutation_strategy = other_strategy.mutation_strategy;
        assert_ne!(world.state_digest(), changed_world.state_digest());
    }

    #[test]
    fn offspring_inherit_parent_tag_strategy_and_lineage_event_identity() {
        let mut cfg = seed_config();
        cfg.initial_energy = 10_000;
        cfg.mutation_rate = 0.0;
        cfg.strategy_mutation_rate = 0.0;
        cfg.insertion_rate = 0.0;
        cfg.deletion_rate = 0.0;
        cfg.duplication_rate = 0.0;
        cfg.tag_mutation_rate = 0.0;
        let mut world = World::new(cfg);
        world.programs.get_mut(&0).unwrap().tag = 23;
        world.program_tags[0] = 23;
        let parent_strategy = mutation::MutationStrategy::new(11, 22, 33, 44, 0);
        world.programs.get_mut(&0).unwrap().mutation_strategy = parent_strategy;

        let born = (0..10_000).find_map(|_| {
            world
                .tick()
                .into_iter()
                .find(|event| matches!(event, Event::Born { .. }))
        });

        let Event::Born {
            id,
            heritable_identity,
            ..
        } = born.expect("tagged child")
        else {
            unreachable!()
        };
        assert_eq!(heritable_identity.tag, 23);
        assert_eq!(heritable_identity.mutation_strategy, parent_strategy);
        assert_eq!(world.programs[&id].tag, 23);
        assert_eq!(world.programs[&id].mutation_strategy, parent_strategy);
        assert_eq!(
            world.heritable_identity(&world.programs[&id]),
            heritable_identity
        );
    }

    #[test]
    fn mutation_strategy_self_mutates_across_generations_without_filtering() {
        let mut cfg = seed_config();
        cfg.mutation_rate = 0.0;
        cfg.insertion_rate = 0.0;
        cfg.deletion_rate = 0.0;
        cfg.duplication_rate = 0.0;
        cfg.tag_mutation_rate = 0.0;
        cfg.strategy_mutation_rate = 1.0;
        let mut world = World::new(cfg);
        let parent_start = world.programs[&0].start;
        let inherited = world.programs[&0].mutation_strategy;
        let mut child = Program::new(1, parent_start, 4, 100, Some(0), None, None);
        child.mutation_strategy = inherited;

        world.apply_birth_mutations(&mut child, parent_start, &mut Vec::new());
        assert_ne!(child.mutation_strategy, inherited);
        let child_strategy = child.mutation_strategy;

        let mut grandchild = Program::new(2, parent_start, 4, 100, Some(1), None, None);
        grandchild.mutation_strategy = child_strategy;
        world.apply_birth_mutations(&mut grandchild, parent_start, &mut Vec::new());
        assert_ne!(grandchild.mutation_strategy, child_strategy);
    }

    #[test]
    fn tag_mutation_creates_a_new_child_heritable_identity() {
        let mut cfg = seed_config();
        cfg.initial_energy = 10_000;
        cfg.mutation_rate = 0.0;
        cfg.insertion_rate = 0.0;
        cfg.deletion_rate = 0.0;
        cfg.duplication_rate = 0.0;
        cfg.tag_mutation_rate = 1.0;
        let mut world = World::new(cfg);
        let parent_heritable_identity = world.heritable_identity(&world.programs[&0]);

        let mut tag_change = None;
        let child_id = (0..10_000).find_map(|_| {
            let events = world.tick();
            tag_change = tag_change.or_else(|| {
                events.iter().find_map(|event| match event {
                    Event::TagChanged {
                        id,
                        old_tag,
                        new_tag,
                        ..
                    } => Some((*id, *old_tag, *new_tag)),
                    _ => None,
                })
            });
            events.into_iter().find_map(|event| match event {
                Event::Born { id, .. } => Some(id),
                _ => None,
            })
        });

        let child_id = child_id.expect("mutated child");
        let child = &world.programs[&child_id];
        let (changed_id, old_tag, new_tag) = tag_change.expect("tag mutation event");
        assert_eq!(changed_id, child_id);
        assert_eq!(old_tag, parent_heritable_identity.tag);
        assert_eq!(new_tag, child.tag);
        assert_ne!(child.tag, parent_heritable_identity.tag);
        assert_ne!(world.heritable_identity(child), parent_heritable_identity);
    }

    #[test]
    fn provenance_attribution_does_not_follow_later_tag_changes_or_death() {
        let mut world = World::new(seed_config());
        let deposited = world.heritable_identity(&world.programs[&0]);
        let receiver = HeritableIdentity::new(deposited.genome ^ 1, 42);

        world.programs.get_mut(&0).unwrap().tag = 99;
        world
            .programs
            .get_mut(&0)
            .unwrap()
            .mutation_strategy
            .copy_error_rate += 1;
        let changed = world.heritable_identity(&world.programs[&0]);
        world.programs.remove(&0);
        world.record_resource_transfer(deposited, receiver, 77);

        assert_eq!(world.interactions.get(&(deposited, receiver)), Some(&77));
        assert!(!world.interactions.contains_key(&(changed, receiver)));
    }

    #[test]
    fn candidate_selection_and_removal_preserve_tag_defined_clades() {
        let mut world = World::new(seed_config());
        world.programs.get_mut(&0).unwrap().tag = 3;
        world.program_tags[0] = 3;
        let first = world.heritable_identity(&world.programs[&0]);
        let second = add_seed_clone(&mut world, 1, 9);
        assert_eq!(world.live_genomes(), 1);
        assert_eq!(world.live_heritable_identities(), 2);

        let pair = world.candidate_partner_pair().expect("two tag clades");
        assert_eq!(
            std::collections::HashSet::from([pair.0, pair.1]),
            [first, second].into()
        );

        world.remove_heritable_identity(first);
        assert_eq!(world.live_count(), 1);
        assert_eq!(
            world.heritable_identity(world.programs.values().next().unwrap()),
            second
        );
    }

    #[test]
    fn candidate_transfer_ties_are_independent_of_insertion_order() {
        fn candidate_with_order(
            reverse: bool,
        ) -> (
            (HeritableIdentity, HeritableIdentity),
            (HeritableIdentity, HeritableIdentity),
        ) {
            let mut world = World::new(seed_config());
            world.programs.get_mut(&0).unwrap().tag = 1;
            let first = world.heritable_identity(&world.programs[&0]);
            let second = add_seed_clone(&mut world, 1, 2);
            let third = add_seed_clone(&mut world, 2, 3);
            for identity in [first, second, third] {
                world
                    .births_by_parent_heritable_identity
                    .insert(identity, 2);
                world.last_birth_by_heritable_identity.insert(identity, 0);
            }
            let mut transfers = [((first, second), 50), ((first, third), 50)];
            if reverse {
                transfers.reverse();
            }
            for (pair, amount) in transfers {
                world.interactions.insert(pair, amount);
            }
            (
                world.candidate_partner_pair().expect("candidate pair"),
                std::cmp::min((first, second), (first, third)),
            )
        }

        let (forward, expected) = candidate_with_order(false);
        let (reverse, reverse_expected) = candidate_with_order(true);
        assert_eq!(forward, reverse);
        assert_eq!(expected, reverse_expected);
        assert_eq!(forward, expected);
    }

    #[test]
    fn candidate_fallback_ties_are_independent_of_program_insertion_order() {
        fn candidate_with_order(reverse: bool) -> (HeritableIdentity, HeritableIdentity) {
            let mut world = World::new(seed_config());
            world.programs.get_mut(&0).unwrap().tag = 1;
            let second_identity = add_seed_clone(&mut world, 1, 2);
            let third_identity = add_seed_clone(&mut world, 2, 3);
            let first = world.programs.remove(&0).unwrap();
            let first_identity = world.heritable_identity(&first);
            if reverse {
                world.programs.insert(0, first);
            } else {
                let second = world.programs.remove(&1).unwrap();
                let third = world.programs.remove(&2).unwrap();
                world.programs.insert(0, first);
                world.programs.insert(1, second);
                world.programs.insert(2, third);
            }
            let mut identities = [first_identity, second_identity, third_identity];
            identities.sort();
            assert_eq!(
                world.candidate_partner_pair(),
                Some((identities[0], identities[1]))
            );
            world.candidate_partner_pair().unwrap()
        }

        assert_eq!(candidate_with_order(false), candidate_with_order(true));
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
    fn opposing_currents_move_each_resource_with_its_provenance() {
        let mut cfg = seed_config();
        cfg.resource_sources.clear();
        cfg.energy_decay_interval = 1;
        cfg.energy_decay_rate = 0;
        cfg.energy_current = 17;
        let mut world = World::new(cfg);
        let identity = world.heritable_identity(&world.programs[&0]);
        let origin = crate::memory::ResourceOrigin::new(0, identity);
        world.memory.give_energy_from(60_000, 123, Some(origin));
        world.memory.give_resource_b_from(60_000, 456, Some(origin));

        world.tick();

        assert_eq!(
            world.memory.resource_a_provenance[60_017].amount_for(origin),
            123
        );
        assert_eq!(
            world.memory.resource_b_provenance[59_983].amount_for(origin),
            456
        );
        assert_eq!(world.memory.resource_a_provenance[60_000].total(), 0);
        assert_eq!(world.memory.resource_b_provenance[60_000].total(), 0);
    }

    #[test]
    fn configured_budget_caps_seed_energy() {
        let mut cfg = seed_config();
        cfg.initial_energy = 500;
        cfg.total_energy = 100;
        let world = World::new(cfg);

        assert_eq!(world.accounted_budget(), 100);
        assert_eq!(world.programs[&0].energy, 100);
    }

    #[test]
    fn configured_budget_is_strictly_conserved_each_tick() {
        let mut cfg = seed_config();
        cfg.initial_energy = 10_000;
        cfg.mutation_rate = 0.0;
        cfg.insertion_rate = 0.0;
        cfg.deletion_rate = 0.0;
        cfg.duplication_rate = 0.0;
        cfg.tag_mutation_rate = 0.0;
        let expected = cfg.total_energy;
        let mut world = World::new(cfg);

        for _ in 0..25_000 {
            world.tick();
            assert_eq!(world.accounted_budget(), expected);
        }
    }

    #[test]
    fn ecotype_observation_thresholds_do_not_affect_simulation_state() {
        let mut strict = seed_config();
        strict.ecotype_min_persistence_ticks = 10_000;
        strict.ecotype_min_reproductive_output = 10;
        strict.ecotype_min_descendant_generations = 4;
        let mut permissive = strict.clone();
        permissive.ecotype_min_persistence_ticks = 1;
        permissive.ecotype_min_reproductive_output = 0;
        permissive.ecotype_min_descendant_generations = 0;
        let mut left = World::new(strict);
        let mut right = World::new(permissive);

        left.run(5_000);
        right.run(5_000);

        let project = |world: &World| {
            let mut programs: Vec<_> = world
                .programs
                .values()
                .map(|program| {
                    (
                        program.id,
                        program.start,
                        program.length,
                        program.ip,
                        program.reg_a,
                        program.reg_b,
                        program.rh,
                        program.wh,
                        (
                            program.energy,
                            program.metabolite_a,
                            program.metabolite_b,
                            program.age,
                            program.generation,
                            program.tag,
                            program.trace.clone(),
                        ),
                    )
                })
                .collect();
            programs.sort_by_key(|program| program.0);
            programs
        };
        assert_eq!(project(&left), project(&right));
        assert_eq!(left.ambient_pool, right.ambient_pool);
        assert_eq!(left.memory.energy_map, right.memory.energy_map);
        assert_eq!(left.memory.resource_b_map, right.memory.resource_b_map);
        assert_eq!(left.total_births, right.total_births);
        assert_eq!(left.total_mutations, right.total_mutations);
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
    fn dead_organisms_archive_behavior_and_reproductive_output() {
        let mut cfg = seed_config();
        cfg.max_program_age = 3;
        cfg.initial_energy = 100;
        cfg.resource_sources.clear();
        let mut world = World::new(cfg);

        world.run(3);

        let archived = world
            .behavior_archive
            .iter()
            .find(|observation| observation.program_id == 0)
            .expect("dead ancestor behavior");
        assert_eq!(archived.termination, ObservationTermination::Death);
        assert_eq!(archived.behavior.steps, 3);
        assert_eq!(archived.reproductive_output, 0);
        assert_eq!(archived.end_tick, Some(3));
    }

    #[test]
    fn dead_reproducer_keeps_nonzero_output_in_archive() {
        let mut cfg = seed_config();
        cfg.max_program_age = 0;
        cfg.initial_energy = 10_000;
        cfg.mutation_rate = 0.0;
        cfg.insertion_rate = 0.0;
        cfg.deletion_rate = 0.0;
        cfg.duplication_rate = 0.0;
        cfg.tag_mutation_rate = 0.0;
        let mut world = World::new(cfg);

        for _ in 0..10_000 {
            if world.tick().iter().any(|event| {
                matches!(
                    event,
                    Event::Born {
                        parent_id: Some(0),
                        ..
                    }
                )
            }) {
                break;
            }
        }
        assert!(world.total_births > 0);
        world.config.max_program_age = world.programs[&0].age + 1;
        for _ in 0..world.live_count() + 1 {
            world.tick();
        }

        assert!(world.behavior_archive.iter().any(|observation| {
            observation.program_id == 0
                && observation.termination == ObservationTermination::Death
                && observation.reproductive_output >= 1
        }));
    }

    #[test]
    fn identity_changes_segment_execution_traces() {
        let mut cfg = seed_config();
        cfg.resource_sources.clear();
        let mut world = World::new(cfg);
        let original = world.heritable_identity(&world.programs[&0]);

        world.tick();
        world.programs.get_mut(&0).unwrap().tag = 44;
        world.tick();

        let segment = world
            .behavior_archive
            .iter()
            .find(|observation| {
                observation.program_id == 0
                    && observation.termination == ObservationTermination::IdentityChanged
            })
            .expect("identity-change segment");
        assert_eq!(segment.identity, original);
        assert_eq!(segment.behavior.steps, 1);
        let live = world
            .behavior_observations()
            .into_iter()
            .find(|observation| {
                observation.program_id == 0
                    && observation.termination == ObservationTermination::Live
            })
            .expect("new live segment");
        assert_eq!(live.identity.tag, 44);
        assert_eq!(live.behavior.steps, 1);
    }

    #[test]
    fn foreign_writes_segment_victim_even_when_event_tracking_is_disabled() {
        let mut cfg = seed_config();
        cfg.resource_sources.clear();
        cfg.foreign_write_tracking = false;
        let mut world = World::new(cfg);
        let victim_identity = add_seed_clone(&mut world, 1, 0);
        let victim_start = world.programs[&1].start;
        let attacker_start = world.programs[&0].start;
        world.memory.write(attacker_start, u8::from(Opcode::Write));
        {
            let attacker = world.programs.get_mut(&0).unwrap();
            attacker.ip = attacker_start;
            attacker.wh = victim_start;
            attacker.reg_a = 255;
        }

        let events = world.tick();

        assert!(!events
            .iter()
            .any(|event| matches!(event, Event::ForeignWrite { .. })));
        assert!(world.behavior_archive.iter().any(|observation| {
            observation.program_id == 1
                && observation.identity == victim_identity
                && observation.termination == ObservationTermination::IdentityChanged
        }));
    }

    #[test]
    fn new_program_event_waits_for_stable_grandchild_evidence() {
        let mut cfg = seed_config();
        cfg.ecotype_min_persistence_ticks = 1;
        cfg.ecotype_min_reproductive_output = 2;
        cfg.ecotype_min_descendant_generations = 2;
        let mut world = World::new(cfg);
        let mut trace = crate::program::BehaviorTrace::default();
        trace.record(crate::opcode::Opcode::MovFwd);
        trace.record(crate::opcode::Opcode::MovBwd);
        let fixture =
            |id, parent_id, generation, offspring_ids: Vec<ProgramId>| BehaviorObservation {
                program_id: id,
                parent_id,
                generation,
                began_at_birth: true,
                identity: HeritableIdentity::new(100 + id as u64, 7),
                behavior: trace.clone(),
                start_tick: 0,
                end_tick: Some(10),
                reproductive_output: offspring_ids.len() as u64,
                offspring_ids,
                termination: ObservationTermination::Death,
            };
        world.behavior_archive.extend([
            fixture(10, None, 0, vec![11]),
            fixture(11, Some(10), 1, vec![12]),
        ]);

        assert!(!world
            .tick()
            .iter()
            .any(|event| matches!(event, Event::NewProgram { .. })));

        world
            .behavior_archive
            .push(fixture(12, Some(11), 2, vec![]));
        let events = world.tick();

        assert!(events.iter().any(|event| matches!(
            event,
            Event::NewProgram {
                descendant_generations: 2,
                equivalent_raw_genomes: 3,
                ..
            }
        )));
        assert!(!world
            .tick()
            .iter()
            .any(|event| matches!(event, Event::NewProgram { .. })));
    }

    #[test]
    fn extinction_refreshes_final_archived_viability_evidence() {
        let mut cfg = seed_config();
        cfg.ecotype_min_persistence_ticks = 100;
        cfg.ecotype_min_reproductive_output = 2;
        cfg.ecotype_min_descendant_generations = 2;
        let mut world = World::new(cfg);
        let mut trace = crate::program::BehaviorTrace::default();
        trace.record(Opcode::Nop);
        for (id, parent_id, generation, offspring_ids) in [
            (10, None, 0, vec![11]),
            (11, Some(10), 1, vec![12]),
            (12, Some(11), 2, vec![]),
        ] {
            world.behavior_archive.push(BehaviorObservation {
                program_id: id,
                parent_id,
                generation,
                began_at_birth: true,
                identity: HeritableIdentity::new(10, 1),
                behavior: trace.clone(),
                start_tick: 0,
                end_tick: Some(100),
                reproductive_output: offspring_ids.len() as u64,
                offspring_ids,
                termination: ObservationTermination::Death,
            });
        }
        world.programs.get_mut(&0).unwrap().energy = 0;

        assert!(!world
            .tick()
            .iter()
            .any(|event| matches!(event, Event::NewProgram { .. })));
        let events = world.tick();

        assert!(events
            .iter()
            .any(|event| matches!(event, Event::NewProgram { .. })));
        assert_eq!(world.viable_ecotype_count(), 1);
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
        // Use mutation_rate = 1.0 to guarantee every replication COPY mutates.
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
    fn saturated_source_cell_preserves_existing_provenance() {
        let mut world = World::new(seed_config());
        let start = 123;
        let identity = world.heritable_identity(&world.programs[&0]);
        let origin = crate::memory::ResourceOrigin::new(7, identity);
        world.memory.give_energy_from(start, u32::MAX, Some(origin));
        let ambient_before = world.ambient_pool;

        world.apply_resource_deposit(ResourceDeposit {
            kind: ResourceKind::A,
            start,
            width: 1,
            amount: 10,
        });

        assert_eq!(
            world.memory.resource_a_provenance[start as usize].amount_for(origin),
            u32::MAX
        );
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

        assert_eq!(world.config.max_resource_flux_per_instruction, 256);
        assert_eq!(world.config.max_metabolism_per_instruction, 256);
        assert_eq!(world.accounted_budget(), world.config.total_energy);
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
            assert_eq!(
                world.accounted_budget(),
                world.config.total_energy,
                "energy was not conserved at tick {}",
                world.tick
            );
        }
    }
}

#[cfg(test)]
mod replay_tests;

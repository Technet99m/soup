use super::*;
use crate::memory::ResourceOrigin;
use crate::program::BehaviorTrace;
use crate::stats::StatsSnapshot;
use uuid::Uuid;

fn world() -> World {
    World::new(Config {
        templates_dir: "/nonexistent_soup_replay_templates".into(),
        ..Config::default()
    })
}

#[test]
fn digest_is_canonical_under_all_hashmap_insertion_orders() {
    let mut a = world();
    a.run(500);
    for i in 0..16 {
        let key = HeritableIdentity::new(i, i as u8);
        a.births_by_parent_heritable_identity.insert(key, i);
        a.last_birth_by_heritable_identity.insert(key, i);
        a.steps_by_heritable_identity.insert(key, i);
        a.interactions
            .insert((key, HeritableIdentity::new(i + 1, 0)), i);
    }
    let mut b = a.clone();
    macro_rules! reverse_map {
        ($field:ident) => {{
            let mut items: Vec<_> = b.$field.drain().collect();
            items.sort_by_key(|(k, _)| *k);
            for (k, v) in items.into_iter().rev() {
                b.$field.insert(k, v);
            }
        }};
    }
    reverse_map!(programs);
    reverse_map!(births_by_parent_heritable_identity);
    reverse_map!(last_birth_by_heritable_identity);
    reverse_map!(interactions);
    reverse_map!(steps_by_heritable_identity);
    assert_eq!(a.state_digest(), b.state_digest());
    assert_eq!(
        StatsSnapshot::compute(&a).format_headless(),
        StatsSnapshot::compute(&b).format_headless()
    );
    assert_eq!(a.candidate_partner_pair(), b.candidate_partner_pair());
    assert!(serde_json::to_vec(&a.run(500)).unwrap() == serde_json::to_vec(&b.run(500)).unwrap());
    assert_eq!(a.state_digest(), b.state_digest());
}

#[test]
fn digest_covers_every_program_field_and_trace() {
    let original = world();
    #[inline(never)]
    fn check(original: &World, mutate: impl FnOnce(&mut Program), label: &str) {
        let mut changed = original.clone();
        mutate(changed.programs.get_mut(&0).unwrap());
        assert_ne!(original.state_digest(), changed.state_digest(), "{label}");
    }
    macro_rules! change {
        ($($mutation:tt)*) => { check(&original, $($mutation)*, stringify!($($mutation)*)) };
    }
    macro_rules! increment { ($($field:ident),*) => { $(change!(|p: &mut Program| p.$field += 1);)* }; }
    increment!(
        id,
        start,
        length,
        ip,
        reg_a,
        reg_b,
        rh,
        wh,
        energy,
        metabolite_a,
        metabolite_b,
        age,
        generation,
        tag
    );
    change!(|p: &mut Program| p.pending_allocation = Some((123, 5)));
    change!(|p: &mut Program| p.loop_stack.push(13));
    change!(|p: &mut Program| p.lineage_id = Uuid::nil());
    change!(|p: &mut Program| p.parent_lineage_id = Some(Uuid::nil()));
    change!(|p: &mut Program| p.parent_id = Some(0));
    change!(|p: &mut Program| p.template_id = None);
    macro_rules! trace { ($($field:ident),*) => { $(change!(|p: &mut Program| p.trace.$field += 1);)* }; }
    trace!(
        steps,
        harvested_a,
        harvested_b,
        given_a,
        given_b,
        converted_a,
        converted_b,
        combined_ab,
        foreign_seeks,
        tag_seeks
    );
    for index in 0..crate::program::TRACE_OPCODE_COUNT {
        change!(|p: &mut Program| p.trace.opcode_counts[index] += 1);
    }
}

#[test]
fn digest_covers_world_storage_accounting_schedule_and_rng() {
    let original = world();
    #[inline(never)]
    fn check(original: &World, mutate: impl FnOnce(&mut World), label: &str) {
        let mut changed = original.clone();
        mutate(&mut changed);
        assert_ne!(original.state_digest(), changed.state_digest(), "{label}");
    }
    macro_rules! change {
        ($mutation:expr) => {
            check(&original, $mutation, stringify!($mutation))
        };
    }
    change!(|w: &mut World| {
        w.memory.write(65_535, 255);
    }); // unowned memory
    change!(|w: &mut World| w.memory.energy_map[65_535] += 1);
    change!(|w: &mut World| w.memory.resource_b_map[65_535] += 1);
    change!(|w: &mut World| w.queue.push_back(u32::MAX)); // stale IDs count
    change!(|w: &mut World| {
        w.queue.push_front(u32::MAX);
    });
    change!(|w: &mut World| {
        w.free_list.alloc(1);
    });
    change!(|w: &mut World| {
        w.rng.gen::<u64>();
    });
    change!(|w: &mut World| w.run_namespace[0] ^= 1);
    change!(|w: &mut World| w.birth_history[0] ^= 1);
    change!(|w: &mut World| w.template_names[0].push('x'));
    change!(|w: &mut World| w.template_bytes[0].push(0));
    change!(|w: &mut World| w.addr_to_owner[65_535] = Some(123));
    change!(|w: &mut World| w.program_tags.push(1));
    change!(|w: &mut World| w
        .heritable_identity_by_id
        .push(HeritableIdentity::new(9, 2)));
    macro_rules! increment { ($($field:ident),*) => { $(change!(|w: &mut World| w.$field += 1);)* }; }
    increment!(
        tick,
        next_id,
        ambient_pool,
        total_births,
        total_deaths,
        total_mutations,
        total_foreign_execs,
        total_foreign_writes,
        max_generation
    );
    macro_rules! map { ($($field:ident),*) => { $(change!(|w: &mut World| { w.$field.insert(HeritableIdentity::new(7, 3), 9); });)* }; }
    map!(
        births_by_parent_heritable_identity,
        last_birth_by_heritable_identity,
        steps_by_heritable_identity
    );
    change!(|w: &mut World| {
        w.interactions.insert(
            (HeritableIdentity::new(7, 3), HeritableIdentity::new(8, 4)),
            9,
        );
    });
    change!(|w: &mut World| w.config.resource_sources[0].velocity += 1);
    change!(|w: &mut World| w.config.rng_seed += 1);
    change!(|w: &mut World| w.config.child_energy += 1);
    change!(|w: &mut World| w.config.foreign_exec_tracking = false);
    change!(|w: &mut World| w.config.foreign_write_tracking = false);
    // Equal resource totals, different attribution, including quantities and each donor dimension.
    for resource_b in [false, true] {
        for variant in 0..4 {
            let mut a = original.clone();
            let mut b = original.clone();
            let first = ResourceOrigin::new(3, HeritableIdentity::new(7, 2));
            let second = match variant {
                0 => None,
                1 => Some(ResourceOrigin::new(4, first.heritable_identity)),
                2 => Some(ResourceOrigin::new(3, HeritableIdentity::new(8, 2))),
                _ => Some(ResourceOrigin::new(3, HeritableIdentity::new(7, 3))),
            };
            for (w, origin) in [(&mut a, Some(first)), (&mut b, second)] {
                if resource_b {
                    w.memory.give_resource_b_from(17, 30, origin);
                } else {
                    w.memory.give_energy_from(17, 30, origin);
                }
            }
            assert_ne!(a.state_digest(), b.state_digest());
        }
    }
}

#[test]
fn namespaces_cover_effective_config_and_ignore_observer_paths() {
    let original = world();
    #[inline(never)]
    fn check(original: &World, mutate: impl FnOnce(&mut Config), label: &str) {
        let mut cfg = original.config.clone();
        mutate(&mut cfg);
        let changed = World::new(cfg);
        assert_ne!(original.run_namespace(), changed.run_namespace(), "{label}");
        assert_ne!(
            original.programs[&0].lineage_id,
            changed.programs[&0].lineage_id
        );
    }
    macro_rules! change {
        ($mutation:expr) => {
            check(&original, $mutation, stringify!($mutation))
        };
    }
    macro_rules! ints { ($($field:ident),*) => { $(change!(|c: &mut Config| c.$field += 1);)* }; }
    ints!(
        rng_seed,
        initial_energy,
        max_genome_length,
        interaction_radius,
        alloc_cost,
        commit_cost,
        max_program_age,
        loop_max_depth,
        energy_decay_rate,
        energy_decay_interval,
        energy_current,
        total_energy,
        child_energy
    );
    macro_rules! rates { ($($field:ident),*) => { $(change!(|c: &mut Config| c.$field += 0.01);)* }; }
    rates!(
        mutation_rate,
        insertion_rate,
        deletion_rate,
        duplication_rate,
        child_locality_bias,
        tag_mutation_rate
    );
    change!(|c: &mut Config| c.resource_sources[0].offset += 1);
    change!(|c: &mut Config| c.resource_sources[0].kind = ResourceKind::B);
    change!(|c: &mut Config| c.resource_sources[0].interval += 1);
    change!(|c: &mut Config| c.resource_sources[0].amount += 1);
    change!(|c: &mut Config| c.resource_sources[0].width += 1);
    change!(|c: &mut Config| c.resource_sources[0].velocity += 1);
    change!(|c: &mut Config| c.resource_sources.reverse());
    let mut cfg = original.config.clone();
    cfg.log_path = "/different/log".into();
    cfg.templates_dir = "/another/nonexistent/templates".into();
    cfg.ticks_per_stat_log += 1;
    cfg.memory_size = 123; // legacy field does not change the fixed 64-KiB VM
    let same = World::new(cfg.clone());
    assert_eq!(original.run_namespace(), same.run_namespace());
    assert_eq!(original.state_digest(), same.state_digest());
    cfg.foreign_exec_tracking = false;
    cfg.foreign_write_tracking = false;
    assert_eq!(original.run_namespace(), World::new(cfg).run_namespace());
}

#[test]
fn clone_births_match_until_divergence_and_keep_history() {
    fn birth(w: &mut World) -> uuid::Uuid {
        for _ in 0..10_000 {
            for event in w.tick() {
                if let Event::Born { lineage_id, .. } = event {
                    return lineage_id;
                }
            }
        }
        panic!("fixture must reproduce");
    }
    let mut a = world();
    let mut b = a.clone();
    assert_eq!(birth(&mut a), birth(&mut b));
    assert_eq!(a.state_digest(), b.state_digest());
    // An intervention outside the reproducing genome must still split future identities.
    b.memory.write(65_535, 255);
    assert_eq!(a.run_namespace(), b.run_namespace());
    assert_ne!(birth(&mut a), birth(&mut b));
    // Restoring the changed byte does not erase birth history.
    b.memory.write(65_535, a.memory.read(65_535));
    assert_ne!(birth(&mut a), birth(&mut b));
}

#[test]
fn rng_retains_stdrng_stream_and_digest_observation_never_draws() {
    use rand::RngCore;
    let mut w = world();
    let mut old = rand::rngs::StdRng::seed_from_u64(w.config.rng_seed);
    for index in 0..200 {
        let before = w.rng.get_word_pos();
        if index % 50 == 0 {
            let _ = w.state_digest();
        }
        assert_eq!(before, w.rng.get_word_pos());
        assert_eq!(w.rng.next_u32(), old.next_u32());
        assert_eq!(w.rng.next_u64(), old.next_u64());
        let mut a = [0; 13];
        let mut b = [0; 13];
        w.rng.fill_bytes(&mut a);
        old.fill_bytes(&mut b);
        assert_eq!(a, b);
        assert_eq!(
            w.rng.gen_range(0..65_536usize),
            old.gen_range(0..65_536usize)
        );
        assert_eq!(w.rng.gen::<f64>(), old.gen::<f64>());
    }
    let same = w.state_digest();
    let position = w.rng.get_word_pos();
    w.rng.next_u32();
    assert_ne!(same, w.state_digest());
    w.rng.set_word_pos(position);
    assert_eq!(same, w.state_digest());
    w.rng.set_stream(1);
    assert_ne!(same, w.state_digest());
    w.rng = ChaCha12Rng::seed_from_u64(43);
    w.rng.set_word_pos(position);
    assert_ne!(same, w.state_digest());
}

#[test]
fn observer_controls_do_not_change_birth_identity_or_dynamics() {
    let mut tracked = world();
    let mut cfg = tracked.config.clone();
    cfg.foreign_exec_tracking = false;
    cfg.foreign_write_tracking = false;
    let mut quiet = World::new(cfg);
    for _ in 0..500 {
        let a = tracked.tick();
        let b = quiet.tick();
        let without_tracking: Vec<_> = a
            .into_iter()
            .filter(|event| {
                !matches!(
                    event,
                    Event::ForeignExec { .. } | Event::ForeignWrite { .. }
                )
            })
            .collect();
        assert_eq!(
            serde_json::to_string(&without_tracking).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
    }
    assert!(tracked.total_births > 0);
    assert_eq!(tracked.state_hash(false), quiet.state_hash(false));
    assert_ne!(tracked.state_digest(), quiet.state_digest());
}

#[test]
fn queue_order_and_equal_total_provenance_quantities_change_digest() {
    let mut a = world();
    a.queue.extend([u32::MAX, u32::MAX - 1]);
    let mut b = a.clone();
    b.queue.swap(1, 2);
    assert_ne!(a.state_digest(), b.state_digest());
    for resource_b in [false, true] {
        let mut a = world();
        let mut b = a.clone();
        let donor = Some(ResourceOrigin::new(2, HeritableIdentity::new(3, 4)));
        for (w, amount) in [(&mut a, 10), (&mut b, 20)] {
            if resource_b {
                w.memory.give_resource_b_from(15, amount, donor);
                w.memory.give_resource_b(15, 30 - amount);
            } else {
                w.memory.give_energy_from(15, amount, donor);
                w.memory.give_energy(15, 30 - amount);
            }
        }
        assert_eq!(a.memory.energy_map, b.memory.energy_map);
        assert_eq!(a.memory.resource_b_map, b.memory.resource_b_map);
        assert_ne!(a.state_digest(), b.state_digest());
    }
}

#[test]
fn allocator_geometry_and_loop_order_are_part_of_state() {
    let mut a = world();
    let mut b = a.clone();
    a.free_list = FreeList::new(100, 20);
    b.free_list = FreeList::new(101, 20);
    assert_eq!(a.free_list.free_bytes(), b.free_list.free_bytes());
    assert_eq!(a.free_list.num_blocks(), b.free_list.num_blocks());
    assert_ne!(a.state_digest(), b.state_digest());
    b.free_list = a.free_list.clone();
    a.programs.get_mut(&0).unwrap().loop_stack.extend([10, 20]);
    b.programs.get_mut(&0).unwrap().loop_stack.extend([20, 10]);
    assert_ne!(a.state_digest(), b.state_digest());
}

fn ecotype_world() -> World {
    let mut world = world();
    let mut opcode_counts = [0; crate::program::TRACE_OPCODE_COUNT];
    opcode_counts[0] = 1;
    let behavior = BehaviorTrace {
        steps: 1,
        opcode_counts,
        harvested_a: 1,
        harvested_b: 1,
        given_a: 1,
        given_b: 1,
        converted_a: 1,
        converted_b: 1,
        combined_ab: 1,
        foreign_seeks: 1,
        tag_seeks: 1,
    };
    let observation = BehaviorObservation {
        program_id: 4,
        parent_id: Some(3),
        generation: 2,
        began_at_birth: true,
        identity: HeritableIdentity::new(11, 7),
        behavior,
        start_tick: 5,
        end_tick: Some(9),
        reproductive_output: 2,
        offspring_ids: vec![5, 6],
        termination: ObservationTermination::Death,
    };
    let ecotype_identity = observation.ecotype_identity();
    let equivalence = ecotype_identity.equivalence();
    world.behavior_archive.push(observation);
    world.active_behavior_segments.insert(
        0,
        ActiveBehaviorSegment {
            identity: HeritableIdentity::new(12, 8),
            start_tick: 10,
            began_at_birth: false,
            reproductive_output: 3,
            offspring_ids: vec![7, 8],
        },
    );
    world.announced_ecotypes.insert(equivalence);
    world.viable_ecotypes_cache.insert(
        equivalence,
        ViableEcotype {
            identity: ecotype_identity,
            equivalent_raw_genomes: 2,
            persistence_ticks: 20,
            reproductive_output: 3,
            descendant_generations: 2,
        },
    );
    world
}

#[test]
fn digest_covers_every_persistent_ecotype_field() {
    let original = ecotype_world();
    #[inline(never)]
    fn check(original: &World, mutate: impl FnOnce(&mut World), label: &str) {
        let mut changed = original.clone();
        mutate(&mut changed);
        assert_ne!(original.state_digest(), changed.state_digest(), "{label}");
    }
    macro_rules! change {
        ($mutation:expr) => {
            check(&original, $mutation, stringify!($mutation))
        };
    }

    change!(|w: &mut World| w.behavior_archive[0].program_id += 1);
    change!(|w: &mut World| w.behavior_archive[0].parent_id = None);
    change!(|w: &mut World| w.behavior_archive[0].generation += 1);
    change!(|w: &mut World| w.behavior_archive[0].began_at_birth = false);
    change!(|w: &mut World| w.behavior_archive[0].identity.genome += 1);
    change!(|w: &mut World| w.behavior_archive[0].identity.tag += 1);
    change!(|w: &mut World| w.behavior_archive[0].start_tick += 1);
    change!(|w: &mut World| w.behavior_archive[0].end_tick = None);
    change!(|w: &mut World| w.behavior_archive[0].reproductive_output += 1);
    change!(|w: &mut World| w.behavior_archive[0].offspring_ids[0] += 1);
    change!(|w: &mut World| w.behavior_archive[0].offspring_ids.swap(0, 1));
    change!(|w: &mut World| w.behavior_archive[0].termination = ObservationTermination::Removed);
    macro_rules! archived_trace {
        ($($field:ident),*) => { $(change!(|w: &mut World| w.behavior_archive[0].behavior.$field += 1);)* };
    }
    archived_trace!(
        steps,
        harvested_a,
        harvested_b,
        given_a,
        given_b,
        converted_a,
        converted_b,
        combined_ab,
        foreign_seeks,
        tag_seeks
    );
    for index in 0..crate::program::TRACE_OPCODE_COUNT {
        check(
            &original,
            |w| w.behavior_archive[0].behavior.opcode_counts[index] += 1,
            "archived opcode count",
        );
    }

    change!(|w: &mut World| {
        let segment = w.active_behavior_segments.remove(&0).unwrap();
        w.active_behavior_segments.insert(1, segment);
    });
    change!(|w: &mut World| w
        .active_behavior_segments
        .get_mut(&0)
        .unwrap()
        .identity
        .genome += 1);
    change!(|w: &mut World| w.active_behavior_segments.get_mut(&0).unwrap().identity.tag += 1);
    change!(|w: &mut World| w.active_behavior_segments.get_mut(&0).unwrap().start_tick += 1);
    change!(|w: &mut World| w
        .active_behavior_segments
        .get_mut(&0)
        .unwrap()
        .began_at_birth = true);
    change!(|w: &mut World| w
        .active_behavior_segments
        .get_mut(&0)
        .unwrap()
        .reproductive_output += 1);
    change!(|w: &mut World| w
        .active_behavior_segments
        .get_mut(&0)
        .unwrap()
        .offspring_ids[0] += 1);
    change!(|w: &mut World| w
        .active_behavior_segments
        .get_mut(&0)
        .unwrap()
        .offspring_ids
        .swap(0, 1));

    change!(|w: &mut World| {
        let value = *w.announced_ecotypes.iter().next().unwrap();
        w.announced_ecotypes.remove(&value);
    });
    change!(|w: &mut World| {
        let mut value = *w.announced_ecotypes.iter().next().unwrap();
        w.announced_ecotypes.clear();
        value.tag += 1;
        w.announced_ecotypes.insert(value);
    });
    change!(|w: &mut World| {
        let mut value = *w.announced_ecotypes.iter().next().unwrap();
        w.announced_ecotypes.clear();
        value.behavior.opcode_presence += 1;
        w.announced_ecotypes.insert(value);
    });
    change!(|w: &mut World| {
        let mut value = *w.announced_ecotypes.iter().next().unwrap();
        w.announced_ecotypes.clear();
        value.behavior.effect_presence += 1;
        w.announced_ecotypes.insert(value);
    });

    change!(|w: &mut World| {
        let key = *w.viable_ecotypes_cache.keys().next().unwrap();
        let value = w.viable_ecotypes_cache.remove(&key).unwrap();
        let mut changed = key;
        changed.tag += 1;
        w.viable_ecotypes_cache.insert(changed, value);
    });
    change!(|w: &mut World| {
        let key = *w.viable_ecotypes_cache.keys().next().unwrap();
        let value = w.viable_ecotypes_cache.remove(&key).unwrap();
        let mut changed = key;
        changed.behavior.opcode_presence += 1;
        w.viable_ecotypes_cache.insert(changed, value);
    });
    change!(|w: &mut World| {
        let key = *w.viable_ecotypes_cache.keys().next().unwrap();
        let value = w.viable_ecotypes_cache.remove(&key).unwrap();
        let mut changed = key;
        changed.behavior.effect_presence += 1;
        w.viable_ecotypes_cache.insert(changed, value);
    });
    change!(|w: &mut World| w
        .viable_ecotypes_cache
        .values_mut()
        .next()
        .unwrap()
        .identity
        .heritable_identity
        .genome += 1);
    change!(|w: &mut World| w
        .viable_ecotypes_cache
        .values_mut()
        .next()
        .unwrap()
        .identity
        .heritable_identity
        .tag += 1);
    change!(|w: &mut World| w
        .viable_ecotypes_cache
        .values_mut()
        .next()
        .unwrap()
        .identity
        .behavior
        .opcode_presence += 1);
    change!(|w: &mut World| w
        .viable_ecotypes_cache
        .values_mut()
        .next()
        .unwrap()
        .identity
        .behavior
        .effect_presence += 1);
    change!(|w: &mut World| w
        .viable_ecotypes_cache
        .values_mut()
        .next()
        .unwrap()
        .equivalent_raw_genomes += 1);
    change!(|w: &mut World| w
        .viable_ecotypes_cache
        .values_mut()
        .next()
        .unwrap()
        .persistence_ticks += 1);
    change!(|w: &mut World| w
        .viable_ecotypes_cache
        .values_mut()
        .next()
        .unwrap()
        .reproductive_output += 1);
    change!(|w: &mut World| w
        .viable_ecotypes_cache
        .values_mut()
        .next()
        .unwrap()
        .descendant_generations += 1);
}

#[test]
fn ecotype_hash_collections_have_canonical_insertion_order() {
    let mut a = ecotype_world();
    let mut trace = a.behavior_archive[0].behavior.clone();
    trace.opcode_counts[1] += 1;
    let second = BehaviorObservation {
        program_id: 9,
        behavior: trace,
        ..a.behavior_archive[0].clone()
    };
    let second_key = second.ecotype_identity().equivalence();
    a.behavior_archive.push(second.clone());
    a.active_behavior_segments.insert(
        9,
        ActiveBehaviorSegment {
            identity: second.identity,
            start_tick: 12,
            began_at_birth: true,
            reproductive_output: 1,
            offspring_ids: vec![10],
        },
    );
    a.announced_ecotypes.insert(second_key);
    a.viable_ecotypes_cache.insert(
        second_key,
        ViableEcotype {
            identity: second.ecotype_identity(),
            equivalent_raw_genomes: 1,
            persistence_ticks: 2,
            reproductive_output: 1,
            descendant_generations: 1,
        },
    );
    let mut b = a.clone();
    let mut segments: Vec<_> = b.active_behavior_segments.drain().collect();
    segments.sort_by_key(|(key, _)| *key);
    b.active_behavior_segments
        .extend(segments.into_iter().rev());
    let mut announced: Vec<_> = b.announced_ecotypes.drain().collect();
    announced.sort();
    b.announced_ecotypes.extend(announced.into_iter().rev());
    assert_eq!(a.state_digest(), b.state_digest());
}

#[test]
fn ecotype_viability_config_changes_only_observer_digest() {
    let original = world();
    for mutate in [
        |config: &mut Config| config.ecotype_min_persistence_ticks += 1,
        |config: &mut Config| config.ecotype_min_reproductive_output += 1,
        |config: &mut Config| config.ecotype_min_descendant_generations += 1,
    ] {
        let mut config = original.config.clone();
        mutate(&mut config);
        let changed = World::new(config);
        assert_eq!(original.run_namespace(), changed.run_namespace());
        assert_ne!(original.state_digest(), changed.state_digest());
    }
}

fn next_birth_lineage(world: &mut World) -> Uuid {
    for _ in 0..10_000 {
        for event in world.tick() {
            if let Event::Born { lineage_id, .. } = event {
                return lineage_id;
            }
        }
    }
    panic!("fixture must reproduce");
}

#[test]
fn persistent_ecotype_state_keeps_deterministic_lineage_history_observer_independent() {
    let mut a = ecotype_world();
    let mut same = a.clone();
    let announced: Vec<_> = same.announced_ecotypes.drain().collect();
    same.announced_ecotypes.extend(announced.into_iter().rev());
    assert_eq!(next_birth_lineage(&mut a), next_birth_lineage(&mut same));

    let mut baseline = ecotype_world();
    let mut changed = baseline.clone();
    changed.announced_ecotypes.clear();
    assert_eq!(
        next_birth_lineage(&mut baseline),
        next_birth_lineage(&mut changed)
    );
}

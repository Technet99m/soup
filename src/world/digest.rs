use super::*;

impl World {
    /// Full BLAKE3-256 run namespace derived from effective config and loaded templates.
    pub fn run_namespace(&self) -> [u8; 32] {
        self.run_namespace
    }

    /// Canonical BLAKE3-256 state fingerprint, rendered as 64 lowercase hex digits.
    /// This is an observation (no RNG draws, queue cleanup, or state mutation).
    /// See SPEC.md for the versioned encoding and excluded observer controls.
    pub fn state_digest(&self) -> String {
        blake3::Hash::from_bytes(self.state_hash(true))
            .to_hex()
            .to_string()
    }

    pub(super) fn state_hash(&self, include_observers: bool) -> [u8; 32] {
        // Exhaustive destructuring makes newly added state a compile-time reminder
        // to update the replay schema rather than silently omitting it.
        let Self {
            memory,
            free_list,
            programs,
            queue,
            config,
            tick,
            next_id,
            rng,
            run_namespace,
            birth_history,
            template_names,
            template_bytes,
            ambient_pool,
            addr_to_owner,
            program_tags,
            births_by_parent_heritable_identity,
            last_birth_by_heritable_identity,
            heritable_identity_by_id,
            behavior_archive,
            active_behavior_segments,
            announced_ecotypes,
            viable_ecotypes_cache,
            interactions,
            steps_by_heritable_identity,
            steps_by_program_id,
            total_births,
            total_deaths,
            total_mutations,
            total_foreign_execs,
            total_foreign_writes,
            max_generation,
        } = self;
        let mut out = Encoder::new(if include_observers {
            "public-state/v1"
        } else {
            "birth-state/v1"
        });
        out.value(run_namespace);
        out.value(birth_history);
        canonical::config(config, &mut out);
        if include_observers {
            out.value(&config.foreign_exec_tracking);
            out.value(&config.foreign_write_tracking);
            out.value(&config.ecotype_min_persistence_ticks);
            out.value(&config.ecotype_min_reproductive_output);
            out.value(&config.ecotype_min_descendant_generations);
            out.value(&config.counterfactual_replicates);
            out.value(total_foreign_execs);
            out.value(total_foreign_writes);
        }
        out.value(template_names);
        out.value(template_bytes);
        out.value(memory.as_bytes());
        out.value(memory.energy_map.as_slice());
        out.value(memory.resource_b_map.as_slice());
        out.value(memory.resource_a_provenance.as_ref());
        out.value(memory.resource_b_provenance.as_ref());
        out.value(&queue.len());
        for id in queue {
            out.value(id);
        }
        out.map(programs);
        out.value(&free_list.blocks().len());
        for crate::allocator::FreeBlock { start, length } in free_list.blocks() {
            out.value(start);
            out.value(length);
        }
        out.value(tick);
        out.value(next_id);
        out.value(ambient_pool);
        out.value(addr_to_owner.as_ref());
        out.value(program_tags);
        out.map(births_by_parent_heritable_identity);
        out.map(last_birth_by_heritable_identity);
        out.value(heritable_identity_by_id);
        if include_observers {
            out.value(behavior_archive);
            let mut active_segments: Vec<_> = active_behavior_segments.iter().collect();
            active_segments.sort_unstable_by_key(|(id, _)| *id);
            out.value(&active_segments.len());
            for (id, segment) in active_segments {
                out.value(id);
                out.value(&segment.identity);
                out.value(&segment.start_tick);
                out.value(&segment.began_at_birth);
                out.value(&segment.reproductive_output);
                out.value(&segment.offspring_ids);
            }
            let mut announced: Vec<_> = announced_ecotypes.iter().collect();
            announced.sort_unstable();
            out.value(&announced.len());
            for ecotype in announced {
                out.value(ecotype);
            }
            out.value(&viable_ecotypes_cache.len());
            for (equivalence, ecotype) in viable_ecotypes_cache {
                out.value(equivalence);
                out.value(ecotype);
            }
        }
        out.map(interactions);
        out.map(steps_by_heritable_identity);
        if include_observers {
            out.map(steps_by_program_id);
        }
        out.value(total_births);
        out.value(total_deaths);
        out.value(total_mutations);
        out.value(max_generation);
        // ChaCha12 is rand 0.8 StdRng's algorithm. Seed, stream, and next 32-bit
        // word position fully describe future output, independent of buffer layout.
        out.value(&"rand_chacha/ChaCha12Rng/0.3.1");
        out.value(&rng.get_seed());
        out.value(&rng.get_stream());
        out.value(&rng.get_word_pos());
        // Source phase/origin is derived from config.rng_seed and tick above;
        // the schedule has no hidden state or separate RNG.
        out.finish()
    }
}

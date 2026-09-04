use crate::{
    identity::{BehaviorSignature, EcotypeEquivalence, EcotypeIdentity, HeritableIdentity},
    program::{BehaviorTrace, ProgramId},
};
use std::collections::{BTreeMap, HashSet};

/// Why an execution segment ended. Identity changes split traces so behavior
/// collected under one genome/tag can never leak into the next identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationTermination {
    IdentityChanged,
    Death,
    Removed,
    Live,
}

/// Archived or live execution evidence for one organism under one exact
/// heritable identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BehaviorObservation {
    pub program_id: ProgramId,
    pub parent_id: Option<ProgramId>,
    pub generation: u32,
    /// True only for the first segment, whose identity was present at birth.
    pub began_at_birth: bool,
    pub identity: HeritableIdentity,
    pub behavior: BehaviorTrace,
    pub start_tick: u64,
    pub end_tick: Option<u64>,
    pub reproductive_output: u64,
    pub offspring_ids: Vec<ProgramId>,
    pub termination: ObservationTermination,
}

impl BehaviorObservation {
    pub fn ecotype_identity(&self) -> EcotypeIdentity {
        EcotypeIdentity {
            heritable_identity: self.identity,
            behavior: BehaviorSignature::from_trace(&self.behavior),
        }
    }

    fn duration(&self, now: u64) -> u64 {
        self.end_tick.unwrap_or(now).saturating_sub(self.start_tick)
    }
}

/// Minimum historical evidence required before an ecotype is reportable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViabilityRule {
    pub min_persistence_ticks: u64,
    pub min_reproductive_output: u64,
    /// Number of stable parent-to-descendant links required (2 means that a
    /// matching grandchild has been observed).
    pub min_descendant_generations: u32,
}

/// Aggregated evidence for one viable behavioral equivalence class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViableEcotype {
    /// Deterministic representative retaining one exact raw genome as evidence.
    pub identity: EcotypeIdentity,
    pub equivalent_raw_genomes: usize,
    pub persistence_ticks: u64,
    pub reproductive_output: u64,
    pub descendant_generations: u32,
}

/// Evaluate observational history without feeding any result back into VM
/// scheduling, mutation, resource allocation, fitness, or selection.
pub fn viable_ecotypes(
    observations: &[BehaviorObservation],
    now: u64,
    rule: ViabilityRule,
) -> BTreeMap<EcotypeEquivalence, ViableEcotype> {
    let mut groups = BTreeMap::<EcotypeEquivalence, Vec<&BehaviorObservation>>::new();
    for observation in observations {
        if observation.behavior.steps == 0
            || observation.termination == ObservationTermination::Live
        {
            continue;
        }
        groups
            .entry(observation.ecotype_identity().equivalence())
            .or_default()
            .push(observation);
    }

    groups
        .into_iter()
        .filter_map(|(key, group)| {
            let persistence_ticks = group.iter().map(|item| item.duration(now)).sum();
            let reproductive_output = group.iter().map(|item| item.reproductive_output).sum();
            let descendant_generations = descendant_depth(&group);
            if persistence_ticks < rule.min_persistence_ticks
                || reproductive_output < rule.min_reproductive_output
                || descendant_generations < rule.min_descendant_generations
            {
                return None;
            }

            let mut identities: Vec<_> = group.iter().map(|item| item.ecotype_identity()).collect();
            identities.sort();
            let equivalent_raw_genomes = identities
                .iter()
                .map(|identity| identity.heritable_identity.genome)
                .collect::<HashSet<_>>()
                .len();
            Some((
                key,
                ViableEcotype {
                    identity: identities[0],
                    equivalent_raw_genomes,
                    persistence_ticks,
                    reproductive_output,
                    descendant_generations,
                },
            ))
        })
        .collect()
}

fn descendant_depth(group: &[&BehaviorObservation]) -> u32 {
    let mut ordered = group.to_vec();
    ordered.sort_by_key(|item| (item.generation, item.start_tick, item.program_id));
    let mut depth = vec![0u32; ordered.len()];
    for index in 0..ordered.len() {
        let item = ordered[index];
        if !item.began_at_birth {
            continue;
        }
        let Some(parent_id) = item.parent_id else {
            continue;
        };
        depth[index] = ordered[..index]
            .iter()
            .enumerate()
            .filter(|(_, parent)| {
                parent.program_id == parent_id && parent.offspring_ids.contains(&item.program_id)
            })
            .map(|(parent_index, _)| depth[parent_index].saturating_add(1))
            .max()
            .unwrap_or(0);
    }
    depth.into_iter().max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(
        id: u32,
        parent_id: Option<u32>,
        generation: u32,
        genome: u64,
        start_tick: u64,
        end_tick: u64,
        offspring_ids: Vec<u32>,
    ) -> BehaviorObservation {
        let mut trace = BehaviorTrace::default();
        trace.record(crate::opcode::Opcode::MovFwd);
        trace.record(crate::opcode::Opcode::MovBwd);
        BehaviorObservation {
            program_id: id,
            parent_id,
            generation,
            began_at_birth: true,
            identity: HeritableIdentity::new(genome, 7),
            behavior: trace,
            start_tick,
            end_tick: Some(end_tick),
            reproductive_output: offspring_ids.len() as u64,
            offspring_ids,
            termination: ObservationTermination::Death,
        }
    }

    #[test]
    fn transient_mutant_is_not_a_viable_ecotype() {
        let observations = vec![observation(1, Some(0), 1, 99, 10, 12, vec![])];
        let rule = ViabilityRule {
            min_persistence_ticks: 10,
            min_reproductive_output: 1,
            min_descendant_generations: 1,
        };

        assert!(viable_ecotypes(&observations, 12, rule).is_empty());
    }

    #[test]
    fn persistent_reproducing_descendants_become_viable() {
        let observations = vec![
            observation(1, None, 0, 10, 0, 10, vec![2]),
            observation(2, Some(1), 1, 10, 10, 20, vec![]),
        ];
        let rule = ViabilityRule {
            min_persistence_ticks: 20,
            min_reproductive_output: 1,
            min_descendant_generations: 1,
        };

        let viable = viable_ecotypes(&observations, 20, rule);

        assert_eq!(viable.len(), 1);
        let report = viable.values().next().unwrap();
        assert_eq!(report.persistence_ticks, 20);
        assert_eq!(report.reproductive_output, 1);
        assert_eq!(report.descendant_generations, 1);
    }

    #[test]
    fn viability_requires_the_configured_number_of_descendant_generations() {
        let mut observations = vec![
            observation(1, None, 0, 10, 0, 10, vec![2]),
            observation(2, Some(1), 1, 10, 10, 20, vec![3]),
        ];
        let rule = ViabilityRule {
            min_persistence_ticks: 1,
            min_reproductive_output: 2,
            min_descendant_generations: 2,
        };
        assert!(viable_ecotypes(&observations, 20, rule).is_empty());

        observations.push(observation(3, Some(2), 2, 10, 20, 30, vec![]));

        assert_eq!(viable_ecotypes(&observations, 30, rule).len(), 1);
    }

    #[test]
    fn behaviorally_equivalent_genomes_count_as_one_ecotype() {
        let observations = vec![
            observation(1, None, 0, 10, 0, 10, vec![2]),
            observation(2, Some(1), 1, 20, 10, 20, vec![]),
        ];
        let rule = ViabilityRule {
            min_persistence_ticks: 20,
            min_reproductive_output: 1,
            min_descendant_generations: 1,
        };

        let viable = viable_ecotypes(&observations, 20, rule);

        assert_eq!(viable.len(), 1);
        assert_eq!(viable.values().next().unwrap().equivalent_raw_genomes, 2);
    }

    #[test]
    fn identity_change_cannot_bridge_an_inheritance_chain() {
        let mut observations = vec![
            observation(1, None, 0, 10, 0, 10, vec![2]),
            observation(2, Some(1), 1, 20, 10, 20, vec![3]),
            observation(3, Some(2), 2, 30, 20, 30, vec![]),
        ];
        observations[1].began_at_birth = false;
        let rule = ViabilityRule {
            min_persistence_ticks: 1,
            min_reproductive_output: 2,
            min_descendant_generations: 2,
        };

        assert!(viable_ecotypes(&observations, 30, rule).is_empty());
    }

    #[test]
    fn returning_to_an_old_behavior_does_not_reconnect_segments() {
        let mut inherited = observation(2, Some(1), 1, 10, 10, 15, vec![]);
        inherited.termination = ObservationTermination::IdentityChanged;
        let mut returned = observation(2, Some(1), 1, 10, 20, 25, vec![3]);
        returned.began_at_birth = false;
        let observations = vec![
            observation(1, None, 0, 10, 0, 10, vec![2]),
            inherited,
            returned,
            observation(3, Some(2), 2, 10, 25, 30, vec![]),
        ];
        let rule = ViabilityRule {
            min_persistence_ticks: 1,
            min_reproductive_output: 2,
            min_descendant_generations: 2,
        };

        assert!(viable_ecotypes(&observations, 30, rule).is_empty());
    }

    #[test]
    fn incomplete_live_behavior_cannot_establish_viability() {
        let mut observations = vec![
            observation(1, None, 0, 10, 0, 10, vec![2]),
            observation(2, Some(1), 1, 10, 10, 20, vec![3]),
            observation(3, Some(2), 2, 10, 20, 30, vec![]),
        ];
        observations[2].end_tick = None;
        observations[2].termination = ObservationTermination::Live;
        let rule = ViabilityRule {
            min_persistence_ticks: 1,
            min_reproductive_output: 2,
            min_descendant_generations: 2,
        };

        assert!(viable_ecotypes(&observations, 30, rule).is_empty());

        observations[2].end_tick = Some(30);
        observations[2].termination = ObservationTermination::Death;
        assert_eq!(viable_ecotypes(&observations, 30, rule).len(), 1);
    }
}

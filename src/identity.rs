use crate::mutation::MutationStrategy;
use crate::program::BehaviorTrace;
use serde::{Deserialize, Serialize};

/// Heritable evolutionary identity used by ecology and lineage reporting.
///
/// Genome bytes, recognition tag, and mutation strategy are identity dimensions:
/// Offspring inherit the parent's tag and strategy. Either can mutate at birth,
/// while `SET_TAG` changes the tag during life. Explicit extra-genomic fields
/// keep equal byte sequences with different ecological/evolutionary controls in
/// separate clades; the genome hash keeps distinct code separate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct HeritableIdentity {
    pub genome: u64,
    pub tag: u8,
    pub mutation_strategy: MutationStrategy,
}

impl HeritableIdentity {
    pub fn new(genome: u64, tag: u8) -> Self {
        Self {
            genome,
            tag,
            mutation_strategy: MutationStrategy::default(),
        }
    }

    pub const fn with_strategy(genome: u64, tag: u8, mutation_strategy: MutationStrategy) -> Self {
        Self {
            genome,
            tag,
            mutation_strategy,
        }
    }
}

/// Stable, count-independent summary of expressed execution behavior.
///
/// Opcode and effect *presence* define behavior, rather than execution totals,
/// so two organisms that run the same operations for different durations are
/// behaviorally equivalent. An ecotype still keeps a representative raw genome
/// for provenance, while equivalence deliberately collapses different genomes
/// when their recognition tag, mutation strategy, and behavior signature match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BehaviorSignature {
    pub opcode_presence: u64,
    pub effect_presence: u16,
}

impl BehaviorSignature {
    pub fn from_trace(trace: &BehaviorTrace) -> Self {
        let opcode_presence = trace
            .opcode_counts
            .iter()
            .enumerate()
            .fold(0u64, |mask, (opcode, count)| {
                mask | (u64::from(*count > 0) << opcode)
            });
        let effects = [
            trace.harvested_a,
            trace.harvested_b,
            trace.given_a,
            trace.given_b,
            trace.converted_a,
            trace.converted_b,
            trace.combined_ab,
            trace.foreign_seeks,
            trace.tag_seeks,
        ];
        let effect_presence = effects
            .into_iter()
            .enumerate()
            .fold(0u16, |mask, (effect, count)| {
                mask | (u16::from(count > 0) << effect)
            });
        Self {
            opcode_presence,
            effect_presence,
        }
    }
}

/// Evidence-bearing identity for a behavioral ecotype observation.
///
/// `heritable_identity` records the exact genome and recognition state that
/// expressed `behavior`. Ecotype counting uses [`EcotypeEquivalence`], which
/// retains recognition state and behavior but treats raw genomes as equivalent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EcotypeIdentity {
    pub heritable_identity: HeritableIdentity,
    pub behavior: BehaviorSignature,
}

impl EcotypeIdentity {
    pub fn equivalence(self) -> EcotypeEquivalence {
        EcotypeEquivalence {
            tag: self.heritable_identity.tag,
            mutation_strategy: self.heritable_identity.mutation_strategy,
            behavior: self.behavior,
        }
    }
}

/// Counting key for behaviorally equivalent heritable observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EcotypeEquivalence {
    pub tag: u8,
    pub mutation_strategy: MutationStrategy,
    pub behavior: BehaviorSignature,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutation_strategy_is_part_of_clade_and_ecotype_identity() {
        let behavior = BehaviorSignature {
            opcode_presence: 1,
            effect_presence: 2,
        };
        let low = MutationStrategy::new(100, 10, 10, 10, 10);
        let high = MutationStrategy::new(1_000, 10, 10, 10, 10);
        let first = EcotypeIdentity {
            heritable_identity: HeritableIdentity::with_strategy(7, 9, low),
            behavior,
        };
        let second = EcotypeIdentity {
            heritable_identity: HeritableIdentity::with_strategy(7, 9, high),
            behavior,
        };

        assert_ne!(first.heritable_identity, second.heritable_identity);
        assert_ne!(first.equivalence(), second.equivalence());
    }

    #[test]
    fn ecotype_identity_has_a_total_lexicographic_order() {
        let identities = [
            EcotypeIdentity {
                heritable_identity: HeritableIdentity::new(2, 0),
                behavior: BehaviorSignature {
                    opcode_presence: 0,
                    effect_presence: 0,
                },
            },
            EcotypeIdentity {
                heritable_identity: HeritableIdentity::new(1, 2),
                behavior: BehaviorSignature {
                    opcode_presence: 0,
                    effect_presence: 0,
                },
            },
            EcotypeIdentity {
                heritable_identity: HeritableIdentity::new(1, 1),
                behavior: BehaviorSignature {
                    opcode_presence: 2,
                    effect_presence: 0,
                },
            },
            EcotypeIdentity {
                heritable_identity: HeritableIdentity::new(1, 1),
                behavior: BehaviorSignature {
                    opcode_presence: 1,
                    effect_presence: 2,
                },
            },
            EcotypeIdentity {
                heritable_identity: HeritableIdentity::new(1, 1),
                behavior: BehaviorSignature {
                    opcode_presence: 1,
                    effect_presence: 1,
                },
            },
        ];
        let ordered: Vec<_> = identities
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        assert_eq!(
            ordered,
            [
                identities[4],
                identities[3],
                identities[2],
                identities[1],
                identities[0]
            ]
        );
    }
}

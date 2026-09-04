use crate::program::BehaviorTrace;
use serde::{Deserialize, Serialize};

/// Heritable evolutionary identity used by ecology and lineage reporting.
///
/// Genome bytes and the recognition tag are independent identity dimensions:
/// offspring inherit the parent's tag, `SET_TAG` changes it during life, and
/// `tag_mutation_rate` may replace it at birth. Keeping the tag explicit avoids
/// collapsing equal byte sequences that recognize different partners, while
/// retaining the genome hash prevents equal tags from collapsing distinct code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct HeritableIdentity {
    pub genome: u64,
    pub tag: u8,
}

impl HeritableIdentity {
    pub const fn new(genome: u64, tag: u8) -> Self {
        Self { genome, tag }
    }
}

/// Stable, count-independent summary of expressed execution behavior.
///
/// Opcode and effect *presence* define behavior, rather than execution totals,
/// so two organisms that run the same operations for different durations are
/// behaviorally equivalent. An ecotype still keeps a representative raw genome
/// for provenance, while equivalence deliberately collapses different genomes
/// when their recognition tag and behavior signature match.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EcotypeIdentity {
    pub heritable_identity: HeritableIdentity,
    pub behavior: BehaviorSignature,
}

impl EcotypeIdentity {
    pub fn equivalence(self) -> EcotypeEquivalence {
        EcotypeEquivalence {
            tag: self.heritable_identity.tag,
            behavior: self.behavior,
        }
    }
}

/// Counting key for behaviorally equivalent heritable observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EcotypeEquivalence {
    pub tag: u8,
    pub behavior: BehaviorSignature,
}

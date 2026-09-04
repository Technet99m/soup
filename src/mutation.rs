//! Local, phenotype-aware byte mutation and heritable mutation control.
//!
//! A choice byte selects either a synonymous encoding or an adjacent functional
//! instruction. Mutation strategy is extra-genomic, inherited state. Its five
//! fixed-point loci independently control replication-copy errors, the three
//! structural operators, and mutation of the strategy itself. All choices are
//! blind to world, lineage, behavior, and fitness state, so an identical random
//! stream and strategy always produce identical outcomes.

use crate::{events::StructuralMutationKind, opcode::Opcode};
use serde::{Deserialize, Serialize};

/// A probability encoded as a count of the 65,536 possible `u16` rolls.
pub type MutationRate = u32;

/// Extra-genomic, heritable control of the mutation spectrum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MutationStrategy {
    pub copy_error_rate: MutationRate,
    pub insertion_rate: MutationRate,
    pub deletion_rate: MutationRate,
    pub duplication_rate: MutationRate,
    pub strategy_mutation_rate: MutationRate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Lower,
    Higher,
}

impl Default for MutationStrategy {
    fn default() -> Self {
        Self::new(328, 262, 262, 262, 655)
    }
}

impl MutationStrategy {
    pub const LOCUS_COUNT: usize = 5;

    pub const fn new(
        copy_error_rate: MutationRate,
        insertion_rate: MutationRate,
        deletion_rate: MutationRate,
        duplication_rate: MutationRate,
        strategy_mutation_rate: MutationRate,
    ) -> Self {
        Self {
            copy_error_rate,
            insertion_rate,
            deletion_rate,
            duplication_rate,
            strategy_mutation_rate,
        }
    }

    /// Convert an environmental ancestor default into deterministic fixed point.
    pub fn rate_from_probability(probability: f64) -> MutationRate {
        (probability.clamp(0.0, 1.0) * 65_536.0)
            .round()
            .clamp(0.0, 65_536.0) as u32
    }

    #[inline]
    pub fn copy_mutates(self, roll: u16) -> bool {
        (roll as u32) < self.copy_error_rate.min(65_536)
    }

    /// Select at most one structural operator from independent spectrum weights.
    pub fn structural_kind(self, roll: u16) -> Option<StructuralMutationKind> {
        let insertion = self.insertion_rate.min(65_536);
        let deletion = self.deletion_rate.min(65_536);
        let duplication = self.duplication_rate.min(65_536);
        let total = insertion + deletion + duplication;
        if total == 0 {
            return None;
        }
        let selected = if total <= 65_536 {
            let roll = roll as u32;
            if roll >= total {
                return None;
            }
            roll
        } else {
            ((roll as u64 * total as u64) / 65_536) as u32
        };
        if selected < insertion {
            Some(StructuralMutationKind::Insertion)
        } else if selected < insertion + deletion {
            Some(StructuralMutationKind::Deletion)
        } else {
            Some(StructuralMutationKind::Duplication)
        }
    }

    pub const fn locus(self, locus: usize) -> MutationRate {
        match locus % Self::LOCUS_COUNT {
            0 => self.copy_error_rate,
            1 => self.insertion_rate,
            2 => self.deletion_rate,
            3 => self.duplication_rate,
            _ => self.strategy_mutation_rate,
        }
    }

    /// Apply one unbiased-direction random-walk step to one selected locus.
    pub fn mutate_locus(mut self, locus: usize, direction: Direction) -> Self {
        let old = self.locus(locus);
        let step = if old / 8 > 0 { old / 8 } else { 1 };
        let new = match direction {
            Direction::Lower => old.saturating_sub(step),
            Direction::Higher => old.saturating_add(step).min(65_536),
        };
        match locus % Self::LOCUS_COUNT {
            0 => self.copy_error_rate = new,
            1 => self.insertion_rate = new,
            2 => self.deletion_rate = new,
            3 => self.duplication_rate = new,
            _ => self.strategy_mutation_rate = new,
        }
        self
    }
}

/// Map an insertion choice to a raw byte in the balanced alphabet.
///
/// This identity mapping keeps all 256 genotypes reachable exactly once.
#[inline]
pub const fn insert(choice: u8) -> u8 {
    choice
}

/// Apply one local substitution selected by `choice`.
///
/// Even choices select a different alias of the same instruction. Odd choices
/// select an alias of the preceding or following non-NOP instruction in opcode
/// order. Higher choice bits select among that phenotype's aliases.
pub fn substitute(source: u8, choice: u8) -> u8 {
    let source_opcode = Opcode::from(source);
    if choice & 1 == 0 {
        return encoding(source_opcode, choice >> 1, Some(source));
    }

    let index = source_opcode.index();
    let neighbor = if choice & 2 == 0 {
        if index == 0 || index == Opcode::COUNT - 1 {
            1
        } else {
            index + 1
        }
    } else if index <= 1 {
        Opcode::COUNT - 1
    } else {
        index - 1
    };
    encoding(Opcode::from_index(neighbor), choice >> 2, None)
}

fn encoding(opcode: Opcode, ordinal: u8, exclude: Option<u8>) -> u8 {
    let count = (0u8..=255)
        .filter(|&byte| Some(byte) != exclude && Opcode::from(byte) == opcode)
        .count();
    let wanted = ordinal as usize % count;
    (0u8..=255)
        .filter(|&byte| Some(byte) != exclude && Opcode::from(byte) == opcode)
        .nth(wanted)
        .expect("every opcode has a redundant encoding")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opcode::Opcode;

    #[test]
    fn every_functional_encoding_has_synonymous_and_functional_neighbors() {
        for source in 0u8..=255 {
            let source_opcode = Opcode::from(source);
            if source_opcode == Opcode::Nop {
                continue;
            }

            let outcomes: Vec<_> = (0u8..=255)
                .map(|choice| substitute(source, choice))
                .collect();
            assert!(
                outcomes.iter().all(|&outcome| outcome != source),
                "byte {source} can mutate to itself"
            );
            assert!(
                outcomes.iter().any(|&outcome| {
                    outcome != source && Opcode::from(outcome) == source_opcode
                }),
                "byte {source} ({source_opcode:?}) has no synonymous neighbor"
            );
            assert!(
                outcomes.iter().any(|&outcome| {
                    let opcode = Opcode::from(outcome);
                    opcode != Opcode::Nop && opcode != source_opcode
                }),
                "byte {source} ({source_opcode:?}) has no functional neighbor"
            );
        }
    }

    #[test]
    fn fixed_choice_stream_replays_byte_for_byte() {
        let mut byte = 1;
        let observed: Vec<_> = [0, 1, 2, 3]
            .into_iter()
            .map(|choice| {
                byte = substitute(byte, choice);
                byte
            })
            .collect();

        assert_eq!(observed, [48, 2, 97, 1]);
        assert_eq!([0, 47, 255].map(insert), [0, 47, 255]);
    }

    #[test]
    fn inherited_rates_change_distributions_under_identical_rolls() {
        let low = MutationStrategy::new(100, 30, 20, 10, 50);
        let high = MutationStrategy::new(1_000, 300, 200, 100, 50);
        let rolls = 0..=u16::MAX;

        assert_eq!(
            rolls.clone().filter(|&roll| low.copy_mutates(roll)).count(),
            100
        );
        assert_eq!(
            rolls
                .clone()
                .filter(|&roll| high.copy_mutates(roll))
                .count(),
            1_000
        );
        assert_eq!(
            rolls
                .clone()
                .filter(|&roll| low.structural_kind(roll).is_some())
                .count(),
            60
        );
        assert_eq!(
            rolls
                .filter(|&roll| high.structural_kind(roll).is_some())
                .count(),
            600
        );
    }

    #[test]
    fn structural_loci_define_the_operator_spectrum() {
        let insertions = MutationStrategy::new(0, 1_000, 0, 0, 0);
        let deletions = MutationStrategy::new(0, 0, 1_000, 0, 0);
        let duplications = MutationStrategy::new(0, 0, 0, 1_000, 0);

        assert_eq!(
            insertions.structural_kind(42),
            Some(StructuralMutationKind::Insertion)
        );
        assert_eq!(
            deletions.structural_kind(42),
            Some(StructuralMutationKind::Deletion)
        );
        assert_eq!(
            duplications.structural_kind(42),
            Some(StructuralMutationKind::Duplication)
        );
    }

    #[test]
    fn saturated_structural_weights_are_normalized_without_order_bias() {
        let strategy = MutationStrategy::new(0, 65_536, 65_536, 65_536, 0);
        let mut counts = [0usize; 3];
        for roll in 0..=u16::MAX {
            match strategy.structural_kind(roll) {
                Some(StructuralMutationKind::Insertion) => counts[0] += 1,
                Some(StructuralMutationKind::Deletion) => counts[1] += 1,
                Some(StructuralMutationKind::Duplication) => counts[2] += 1,
                None => panic!("saturated spectrum skipped a mutation"),
            }
        }
        assert!(counts
            .iter()
            .all(|&count| (21_845..=21_846).contains(&count)));
    }

    #[test]
    fn strategy_mutation_can_raise_and_lower_every_locus() {
        let ancestor = MutationStrategy::new(1_000, 2_000, 3_000, 4_000, 5_000);
        for locus in 0..MutationStrategy::LOCUS_COUNT {
            let lower = ancestor.mutate_locus(locus, Direction::Lower);
            let higher = ancestor.mutate_locus(locus, Direction::Higher);
            assert!(lower.locus(locus) < ancestor.locus(locus));
            assert!(higher.locus(locus) > ancestor.locus(locus));
        }
    }
}

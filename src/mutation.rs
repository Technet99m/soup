//! Local, phenotype-aware byte mutation.
//!
//! A choice byte selects either a synonymous encoding or an adjacent functional
//! instruction. The functions are pure and receive no world, lineage, or fitness
//! state, so an identical choice stream always produces identical bytes.

use crate::opcode::Opcode;

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
}

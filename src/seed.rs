/// The single primordial ancestor. It is deliberately small and uses measured
/// length, so insertions, deletions, and duplications can remain heritable.
///
/// It seeks and harvests both resource chemistries, copies its current genome,
/// splits its energy with the child, then loops. Descendants may lose or alter
/// either metabolic path and can acquire tag-based interaction instructions.
pub const SEED: [u8; 18] = [
    40, // [0]  SEEK_RESOURCE_A
    31, // [1]  TAKE_RESOURCE_A into metabolite A
    44, // [2]  CONVERT_A into usable energy
    41, // [3]  SEEK_RESOURCE_B
    37, // [4]  TAKE_RESOURCE_B into metabolite B
    45, // [5]  CONVERT_B into usable energy
    33, // [6]  MEASURE_SELF
    25, // [7]  ALLOC measured length
    11, // [8]  SET_WRITE_HEAD to child block
    5,  // [9]  SEEK_SELF_START
    33, // [10] MEASURE_SELF for copy count
    23, // [11] LOOP_OPEN
    10, // [12] COPY
    24, // [13] LOOP_CLOSE
    33, // [14] MEASURE_SELF for child length
    27, // [15] SPLIT energy and commit child
    33, // [16] MEASURE_SELF for jump distance
    20, // [17] JMP_BWD to [0]
];

pub const SEED_LEN: u16 = SEED.len() as u16;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_is_length_adaptive_and_loops() {
        assert_eq!(SEED.len(), 18);
        assert_eq!(SEED[6], 33, "allocation length is measured");
        assert_eq!(SEED[10], 33, "copy count is measured");
        assert_eq!(SEED[14], 33, "child length is measured");
        assert_eq!(SEED[16], 33, "loop distance is measured");
        assert_eq!((17u16 + 1).wrapping_sub(SEED_LEN), 0);
    }

    #[test]
    fn seed_can_process_both_resources() {
        assert_eq!(&SEED[..6], &[40, 31, 44, 41, 37, 45]);
    }
}

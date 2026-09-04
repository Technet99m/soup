/// The single primordial ancestor. It is deliberately small and uses measured
/// length, so insertions, deletions, and duplications can remain heritable.
///
/// It seeks and harvests both resource chemistries, copies its current genome,
/// splits its energy with the child, then loops. Descendants may lose or alter
/// either metabolic path and can acquire tag-based interaction instructions.
pub const SEED: [u8; 16] = [
    40, // [0]  SEEK_RESOURCE_A
    31, // [1]  TAKE_ENERGY (resource A)
    41, // [2]  SEEK_RESOURCE_B
    37, // [3]  TAKE_RESOURCE_B
    33, // [4]  MEASURE_SELF
    25, // [5]  ALLOC measured length
    11, // [6]  SET_WRITE_HEAD to child block
    5,  // [7]  SEEK_SELF_START
    33, // [8]  MEASURE_SELF for copy count
    23, // [9]  LOOP_OPEN
    10, // [10] COPY
    24, // [11] LOOP_CLOSE
    33, // [12] MEASURE_SELF for child length
    27, // [13] SPLIT energy and commit child
    33, // [14] MEASURE_SELF for jump distance
    20, // [15] JMP_BWD to [0]
];

pub const SEED_LEN: u16 = SEED.len() as u16;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_is_length_adaptive_and_loops() {
        assert_eq!(SEED.len(), 16);
        assert_eq!(SEED[4], 33, "allocation length is measured");
        assert_eq!(SEED[8], 33, "copy count is measured");
        assert_eq!(SEED[12], 33, "child length is measured");
        assert_eq!(SEED[14], 33, "loop distance is measured");
        assert_eq!((15u16 + 1).wrapping_sub(SEED_LEN), 0);
    }

    #[test]
    fn seed_can_process_both_resources() {
        assert_eq!(&SEED[..4], &[40, 31, 41, 37]);
    }
}

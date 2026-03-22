/// The primordial ancestor — a bloated, inefficient self-replicator. 32 bytes.
/// Intentionally wasteful so evolution has clear room to improve.
///
/// Improvable regions (mutations that make descendants more competitive):
///   [1]        SEEK_SELF_START — redundant; RH is already at start
///   [4]        NOP_40          — wasted execute + copy cycle
///   [8],[9]    SEEK_SELF_START — both redundant before copy loop
///   [12]       NOP_50          — wasted cycle inside approach to loop
///   [18],[19]  NOP_60, NOP_70  — wasted cycles after loop
///   [21]..[28] NOP variants    — 8 bytes of pure padding; each costs 1 energy
///                                to execute AND 1 energy to copy per cycle
///
/// The 15 unnecessary bytes (vs a minimal 17-byte equivalent) cost roughly:
///   - 3 extra SEEKs:          3 energy/cycle
///   - 10 extra NOPs:         10 energy/cycle
///   - 15 extra COPY iters:   45 energy/cycle  (32 copies vs 17)
///   ≈ 58 extra energy per replication — about 40% overhead
///
/// JMP_BWD at [31], A=32: ip_next = (31+1) - 32 = 0 → loops back to start ✓
/// LOOP_CLOSE "decrement then check": A=32 → COPY executes exactly 32 times ✓
pub const SEED: [u8; 32] = [
    5,    // [0]  SEEK_SELF_START   — RH = start (necessary)
    5,    // [1]  SEEK_SELF_START   — REDUNDANT: RH is already at start
    12,   // [2]  LOAD_IMM
    32,   // [3]    A = 32          — own length (for ALLOC)
    40,   // [4]  NOP               — wasted cycle
    25,   // [5]  ALLOC             — B = child block start
    11,   // [6]  SET_WRITE_HEAD    — WH = B
    5,    // [7]  SEEK_SELF_START   — RH = start (necessary: reset for copy)
    5,    // [8]  SEEK_SELF_START   — REDUNDANT
    5,    // [9]  SEEK_SELF_START   — REDUNDANT
    12,   // [10] LOAD_IMM
    32,   // [11]   A = 32          — loop counter
    50,   // [12] NOP               — wasted cycle
    23,   // [13] LOOP_OPEN
    10,   // [14] COPY              — copy RH→WH, advance both
    24,   // [15] LOOP_CLOSE        — decrement A; if A!=0 jump back to [13]
    12,   // [16] LOAD_IMM
    32,   // [17]   A = 32          — size for COMMIT
    60,   // [18] NOP               — wasted cycle
    70,   // [19] NOP               — wasted cycle
    26,   // [20] COMMIT            — register child at B with size A
    80,   // [21] NOP               — padding (wasted copy + execute)
    90,   // [22] NOP               — padding
    100,  // [23] NOP               — padding
    110,  // [24] NOP               — padding
    120,  // [25] NOP               — padding
    130,  // [26] NOP               — padding
    140,  // [27] NOP               — padding
    150,  // [28] NOP               — padding
    12,   // [29] LOAD_IMM
    32,   // [30]   A = 32          — JMP_BWD distance
    20,   // [31] JMP_BWD           — ip_next = (31+1) - 32 = 0 ✓
];

pub const SEED_LEN: u16 = SEED.len() as u16;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_length_matches_encoded_immediates() {
        assert_eq!(SEED.len(), 32);
        // All four LOAD_IMM immediates must equal own length
        assert_eq!(SEED[3],  32, "ALLOC size immediate");
        assert_eq!(SEED[11], 32, "loop counter immediate");
        assert_eq!(SEED[17], 32, "COMMIT size immediate");
        assert_eq!(SEED[30], 32, "JMP_BWD distance immediate");
    }

    #[test]
    fn seed_opcodes_at_expected_positions() {
        assert_eq!(SEED[0],  5,  "SEEK_SELF_START");
        assert_eq!(SEED[1],  5,  "SEEK_SELF_START (redundant)");
        assert_eq!(SEED[2],  12, "LOAD_IMM");
        assert_eq!(SEED[5],  25, "ALLOC");
        assert_eq!(SEED[6],  11, "SET_WRITE_HEAD");
        assert_eq!(SEED[7],  5,  "SEEK_SELF_START");
        assert_eq!(SEED[8],  5,  "SEEK_SELF_START (redundant)");
        assert_eq!(SEED[9],  5,  "SEEK_SELF_START (redundant)");
        assert_eq!(SEED[10], 12, "LOAD_IMM");
        assert_eq!(SEED[13], 23, "LOOP_OPEN");
        assert_eq!(SEED[14], 10, "COPY");
        assert_eq!(SEED[15], 24, "LOOP_CLOSE");
        assert_eq!(SEED[16], 12, "LOAD_IMM");
        assert_eq!(SEED[20], 26, "COMMIT");
        assert_eq!(SEED[29], 12, "LOAD_IMM");
        assert_eq!(SEED[31], 20, "JMP_BWD");
    }

    #[test]
    fn nop_variants_are_all_nops() {
        // Positions 21-28 should all decode as NOP (opcodes 30-254 are NOP variants)
        for pos in 21..=28 {
            assert!(
                SEED[pos] >= 30 && SEED[pos] <= 254,
                "position {pos} should be a NOP variant, got {}",
                SEED[pos]
            );
        }
    }

    #[test]
    fn jmp_bwd_returns_to_start() {
        // JMP_BWD at [31], A = SEED[30] = 32 → ip_next = (31+1) - 32 = 0
        let jmpbwd_pos: u16 = 31;
        let a: u16 = SEED[30] as u16;
        let ip_next = (jmpbwd_pos + 1).wrapping_sub(a);
        assert_eq!(ip_next, 0, "JMP_BWD should jump back to address 0");
    }
}
